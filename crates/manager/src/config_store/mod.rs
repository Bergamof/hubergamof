//! Per-device configuration persistence.
//!
//! Layout under the root (`--config-dir`):
//!
//! ```text
//! devices/<serial>/config.json   pretty-printed DeviceConfig
//! devices/<serial>/images/       content-addressed images (<sha256>.<ext>)
//! ```
//!
//! Writes are atomic (temporary file + `rename`) and guarded by the
//! configuration `revision`. A corrupt or invalid file is never overwritten:
//! it is renamed `config.json.invalid-<unix time>` and replaced by defaults.
//!
//! The API is synchronous (small files); async callers run it with
//! `tokio::task::spawn_blocking`.

mod defaults;

use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use bergadeck_protocol::{ConfigError, DeviceConfig, DeviceInfo, ErrorCode, is_safe_image_name};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

pub use defaults::{ConfigDefaults, DEFAULT_BRIGHTNESS, default_config};

use crate::render::{ImageKind, MAX_SOURCE_BYTES};

/// Directory holding one sub-directory per device serial.
pub const DEVICES_DIR: &str = "devices";
/// Configuration file name inside a device directory.
pub const CONFIG_FILE: &str = "config.json";
/// Image directory name inside a device directory.
pub const IMAGES_DIR: &str = "images";
/// Prefix of the backup name given to an invalid configuration file.
pub const INVALID_BACKUP_PREFIX: &str = "config.json.invalid-";

/// Longest accepted serial number.
const MAX_SERIAL_LENGTH: usize = 64;
/// Number of hexadecimal digest characters in a stored image name.
const IMAGE_DIGEST_LENGTH: usize = 32;

/// Configuration storage failure.
#[derive(Debug, Error)]
pub enum ConfigStoreError {
    /// The serial cannot be used as a directory name.
    #[error("invalid device serial `{0}`")]
    InvalidSerial(String),
    /// A file operation failed.
    #[error("cannot access {path}: {source}")]
    Io {
        /// Path involved.
        path: PathBuf,
        /// Underlying error.
        source: io::Error,
    },
    /// The stored configuration is not valid JSON for [`DeviceConfig`].
    #[error("cannot parse {path}: {source}")]
    Json {
        /// File path.
        path: PathBuf,
        /// Underlying error.
        source: serde_json::Error,
    },
    /// The configuration is internally inconsistent.
    #[error("invalid configuration: {0}")]
    Invalid(#[from] ConfigError),
    /// The configuration does not have one dial entry per device dial.
    #[error("configuration has {found} dial entries, the device has {expected} dials")]
    DialCountMismatch {
        /// Dials of the device.
        expected: u8,
        /// Entries in the configuration.
        found: usize,
    },
    /// The configuration belongs to another device.
    #[error("configuration is for device `{found}`, expected `{expected}`")]
    SerialMismatch {
        /// Serial of the device.
        expected: String,
        /// Serial in the configuration.
        found: String,
    },
    /// The update was based on an outdated revision.
    #[error("revision {provided} is stale (current revision is {current})")]
    StaleRevision {
        /// Revision on disk.
        current: u64,
        /// Revision of the update.
        provided: u64,
    },
    /// The image name is not a plain file name of the `images/` directory.
    #[error("invalid image name `{0}`")]
    InvalidImageName(String),
    /// The image content does not match the declared format.
    #[error("image content is not {declared:?}")]
    ImageKindMismatch {
        /// Declared format.
        declared: ImageKind,
    },
    /// The image is larger than [`MAX_SOURCE_BYTES`].
    #[error("image is larger than {MAX_SOURCE_BYTES} bytes")]
    ImageTooLarge,
}

impl ConfigStoreError {
    /// Error code reported to the UI.
    #[must_use]
    pub fn error_code(&self) -> ErrorCode {
        match self {
            Self::StaleRevision { .. } => ErrorCode::StaleRevision,
            Self::InvalidSerial(_) => ErrorCode::UnknownDevice,
            Self::Invalid(_)
            | Self::DialCountMismatch { .. }
            | Self::SerialMismatch { .. }
            | Self::InvalidImageName(_)
            | Self::ImageKindMismatch { .. }
            | Self::ImageTooLarge => ErrorCode::InvalidConfig,
            Self::Io { .. } | Self::Json { .. } => ErrorCode::Internal,
        }
    }
}

/// How [`ConfigStore::load`] obtained the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    /// Read from disk.
    Loaded,
    /// No file existed; defaults were written.
    CreatedDefault,
    /// The file was invalid; it was renamed to `backup` and defaults written.
    ReplacedInvalid {
        /// New path of the invalid file.
        backup: PathBuf,
    },
}

/// Result of [`ConfigStore::load`].
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    /// Configuration to use.
    pub config: DeviceConfig,
    /// Where it came from.
    pub outcome: LoadOutcome,
}

/// Configuration store rooted at the `--config-dir` directory. Cheap to
/// clone; clones share the write lock.
#[derive(Clone)]
pub struct ConfigStore {
    root: PathBuf,
    defaults: Arc<dyn ConfigDefaults>,
    write_lock: Arc<Mutex<()>>,
}

impl ConfigStore {
    /// Store rooted at `root`, using `defaults` for new configurations.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, defaults: Arc<dyn ConfigDefaults>) -> Self {
        Self {
            root: root.into(),
            defaults,
            write_lock: Arc::default(),
        }
    }

    /// Root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory of a device.
    ///
    /// # Errors
    ///
    /// [`ConfigStoreError::InvalidSerial`] when the serial is not a safe
    /// directory name.
    pub fn device_dir(&self, serial: &str) -> Result<PathBuf, ConfigStoreError> {
        if is_safe_serial(serial) {
            Ok(self.root.join(DEVICES_DIR).join(serial))
        } else {
            Err(ConfigStoreError::InvalidSerial(serial.to_owned()))
        }
    }

    /// Loads the configuration of `device`, creating it from defaults when
    /// missing and setting an invalid file aside.
    ///
    /// # Errors
    ///
    /// [`ConfigStoreError::InvalidSerial`] or [`ConfigStoreError::Io`].
    pub fn load(&self, device: &DeviceInfo) -> Result<LoadedConfig, ConfigStoreError> {
        let _guard = self.lock();
        let path = self.config_path(&device.serial)?;
        let outcome = match read_config(&path) {
            Ok(Some(config)) => match check_config(&config, device) {
                Ok(()) => {
                    return Ok(LoadedConfig {
                        config,
                        outcome: LoadOutcome::Loaded,
                    });
                }
                Err(error) => set_invalid_aside(&path, &error)?,
            },
            Ok(None) => LoadOutcome::CreatedDefault,
            Err(error @ ConfigStoreError::Json { .. }) => set_invalid_aside(&path, &error)?,
            Err(error) => return Err(error),
        };
        let config = default_config(device, self.defaults.as_ref());
        write_atomically(&path, &config)?;
        tracing::info!(serial = %device.serial, path = %path.display(), "default configuration written");
        Ok(LoadedConfig { config, outcome })
    }

    /// Saves `config` for `device` if it is valid and based on the current
    /// revision; returns the stored configuration with its new revision.
    ///
    /// # Errors
    ///
    /// [`ConfigStoreError::StaleRevision`], a validation error, or an I/O or
    /// parse error on the current file.
    pub fn save(
        &self,
        device: &DeviceInfo,
        mut config: DeviceConfig,
    ) -> Result<DeviceConfig, ConfigStoreError> {
        let _guard = self.lock();
        let path = self.config_path(&device.serial)?;
        let current = read_config(&path)?.map_or(0, |stored| stored.revision);
        if config.revision != current {
            return Err(ConfigStoreError::StaleRevision {
                current,
                provided: config.revision,
            });
        }
        check_config(&config, device)?;
        config.revision = current + 1;
        write_atomically(&path, &config)?;
        Ok(config)
    }

    /// Stores an image of a device under a name derived from its content and
    /// returns that name.
    ///
    /// # Errors
    ///
    /// [`ConfigStoreError::ImageTooLarge`], [`ConfigStoreError::ImageKindMismatch`]
    /// or an I/O error.
    pub fn store_image(
        &self,
        serial: &str,
        bytes: &[u8],
        kind: ImageKind,
    ) -> Result<String, ConfigStoreError> {
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(ConfigStoreError::ImageTooLarge);
        }
        if ImageKind::sniff(bytes) != Some(kind) {
            return Err(ConfigStoreError::ImageKindMismatch { declared: kind });
        }
        let digest = Sha256::digest(bytes);
        let hex = hex_digest(&digest);
        let name = format!("{}.{}", &hex[..IMAGE_DIGEST_LENGTH], kind.extension());
        let path = self.image_path(serial, &name)?;
        if !path.exists() {
            write_bytes_atomically(&path, bytes)?;
        }
        Ok(name)
    }

    /// Path of image `name` of a device; the name must be a plain file name.
    ///
    /// # Errors
    ///
    /// [`ConfigStoreError::InvalidImageName`] for anything that could leave
    /// the `images/` directory, or [`ConfigStoreError::InvalidSerial`].
    pub fn image_path(&self, serial: &str, name: &str) -> Result<PathBuf, ConfigStoreError> {
        if !is_safe_image_name(name) {
            return Err(ConfigStoreError::InvalidImageName(name.to_owned()));
        }
        Ok(self.device_dir(serial)?.join(IMAGES_DIR).join(name))
    }

    fn config_path(&self, serial: &str) -> Result<PathBuf, ConfigStoreError> {
        Ok(self.device_dir(serial)?.join(CONFIG_FILE))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        // The guarded data is the file system: nothing to recover.
        self.write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Validates `config` and checks it against `device`.
///
/// # Errors
///
/// The first inconsistency found.
pub fn check_config(config: &DeviceConfig, device: &DeviceInfo) -> Result<(), ConfigStoreError> {
    config.validate()?;
    if config.serial != device.serial {
        return Err(ConfigStoreError::SerialMismatch {
            expected: device.serial.clone(),
            found: config.serial.clone(),
        });
    }
    let dial_count = device.capabilities.dial_count;
    if config.dials.len() != usize::from(dial_count) {
        return Err(ConfigStoreError::DialCountMismatch {
            expected: dial_count,
            found: config.dials.len(),
        });
    }
    Ok(())
}

fn set_invalid_aside(
    path: &Path,
    reason: &ConfigStoreError,
) -> Result<LoadOutcome, ConfigStoreError> {
    let backup = backup_path(path);
    fs::rename(path, &backup).map_err(|source| io_error(path, source))?;
    tracing::warn!(
        path = %path.display(),
        backup = %backup.display(),
        %reason,
        "invalid configuration set aside, using defaults"
    );
    Ok(LoadOutcome::ReplacedInvalid { backup })
}

fn read_config(path: &Path) -> Result<Option<DeviceConfig>, ConfigStoreError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(path, error)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|source| ConfigStoreError::Json {
            path: path.to_owned(),
            source,
        })
}

fn write_atomically(path: &Path, config: &DeviceConfig) -> Result<(), ConfigStoreError> {
    let mut json = serde_json::to_vec_pretty(config).map_err(|source| ConfigStoreError::Json {
        path: path.to_owned(),
        source,
    })?;
    json.push(b'\n');
    write_bytes_atomically(path, &json)
}

/// Writes `bytes` to a temporary file next to `path`, flushes it to disk and
/// renames it over `path`: readers see either the old or the new content.
fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> Result<(), ConfigStoreError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir).map_err(|source| io_error(dir, source))?;
    let mut file = NamedTempFile::new_in(dir).map_err(|source| io_error(dir, source))?;
    file.write_all(bytes)
        .and_then(|()| file.as_file().sync_all())
        .map_err(|source| io_error(file.path(), source))?;
    file.persist(path)
        .map_err(|error| io_error(path, error.error))?;
    // Persist the rename itself.
    fs::File::open(dir)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error(dir, source))
}

fn backup_path(path: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let base = format!("{INVALID_BACKUP_PREFIX}{timestamp}");
    let mut candidate = dir.join(&base);
    let mut attempt = 1_u32;
    while candidate.exists() {
        candidate = dir.join(format!("{base}-{attempt}"));
        attempt += 1;
    }
    candidate
}

fn hex_digest(digest: &[u8]) -> String {
    digest.iter().fold(String::new(), |mut hex, byte| {
        // Writing to a `String` cannot fail.
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

fn io_error(path: &Path, source: io::Error) -> ConfigStoreError {
    ConfigStoreError::Io {
        path: path.to_owned(),
        source,
    }
}

fn is_safe_serial(serial: &str) -> bool {
    !serial.is_empty()
        && serial.len() <= MAX_SERIAL_LENGTH
        && serial
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests;
