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

mod archive;
mod defaults;
mod migration;
mod placement;

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use hubergamof_module_api::{ImageData, ImageLibrary, ModuleDescriptor};
use hubergamof_protocol::{
    ConfigError, DeviceConfig, DeviceInfo, DialAssignment, ErrorCode, ModuleInstance,
    is_safe_image_name,
};
use placement::normalize_placement;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

pub use archive::{
    ARCHIVE_CONFIG_ENTRY, ARCHIVE_IMAGES_PREFIX, Archive, ArchiveError, ArchiveImages,
    MAX_ARCHIVE_BYTES, MAX_ARCHIVE_CONFIG_BYTES, MAX_ARCHIVE_ENTRIES, MAX_ARCHIVE_EXPANDED_BYTES,
    read_archive, write_archive,
};
pub use defaults::{ConfigDefaults, DEFAULT_BRIGHTNESS, ModuleDescriptors, default_config};
pub use placement::PlacementChange;

use tokio::io::AsyncReadExt as _;

use crate::render::{ImageKind, MAX_SOURCE_BYTES};

/// Directory holding one sub-directory per device serial.
pub const DEVICES_DIR: &str = "devices";
/// Configuration file name inside a device directory.
pub const CONFIG_FILE: &str = "config.json";
/// Image directory name inside a device directory.
pub const IMAGES_DIR: &str = "images";
/// Prefix of the backup name given to an invalid configuration file.
pub const INVALID_BACKUP_PREFIX: &str = "config.json.invalid-";

/// Most images a device stores: far more than its keys use, bounding what
/// uploads may pile up before unused ones are removed at the next load.
pub const MAX_STORED_IMAGES: usize = 512;

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
    /// The device already stores [`MAX_STORED_IMAGES`] images (unused ones
    /// are removed when the configuration is next loaded).
    #[error("the device already stores {MAX_STORED_IMAGES} images")]
    TooManyImages,
    /// An image of an archive cannot be drawn.
    #[error("image `{name}` cannot be drawn: {reason}")]
    UnreadableImage {
        /// Name in the archive.
        name: String,
        /// Why.
        reason: String,
    },
    /// The archive cannot be read.
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    /// The archived configuration uses an image the archive does not hold.
    #[error("the archive has no image {0:?}")]
    MissingImage(String),
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
            | Self::InvalidImageName(_) => ErrorCode::InvalidConfig,
            Self::ImageKindMismatch { .. }
            | Self::ImageTooLarge
            | Self::TooManyImages
            | Self::UnreadableImage { .. } => ErrorCode::InvalidImage,
            Self::Archive(_) | Self::MissingImage(_) => ErrorCode::InvalidArchive,
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
    /// Read from disk, then brought back to the placement rules of the
    /// modules and written again.
    Normalized {
        /// What was changed.
        changes: Vec<PlacementChange>,
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
    descriptors: Arc<dyn ModuleDescriptors>,
    write_lock: Arc<Mutex<()>>,
}

impl ConfigStore {
    /// Store rooted at `root`, with `modules` giving the defaults of new
    /// configurations and describing the settings that refer to images.
    #[must_use]
    pub fn new<M>(root: impl Into<PathBuf>, modules: Arc<M>) -> Self
    where
        M: ConfigDefaults + ModuleDescriptors + 'static,
    {
        Self {
            root: root.into(),
            defaults: modules.clone(),
            descriptors: modules,
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
        self.make_device_private(&device.serial)?;
        let outcome = match read_config(&path) {
            Ok(Some(mut config)) => {
                let changes = normalize_placement(&mut config, self.descriptors.as_ref());
                match check_config(&config, device) {
                    Ok(()) => {
                        let outcome = Self::keep_normalized(&path, &mut config, changes)?;
                        self.prune_images(&device.serial, &config);
                        return Ok(LoadedConfig { config, outcome });
                    }
                    Err(error) => set_invalid_aside(&path, &error)?,
                }
            }
            Ok(None) => LoadOutcome::CreatedDefault,
            Err(error @ ConfigStoreError::Json { .. }) => set_invalid_aside(&path, &error)?,
            Err(error) => return Err(error),
        };
        let config = default_config(device, self.defaults.as_ref());
        write_atomically(&path, &config)?;
        tracing::info!(serial = %device.serial, path = %path.display(), "default configuration written");
        Ok(LoadedConfig { config, outcome })
    }

    /// Writes `config` again when `changes` normalized it; the revision goes
    /// up so that clients holding the previous one refresh.
    fn keep_normalized(
        path: &Path,
        config: &mut DeviceConfig,
        changes: Vec<PlacementChange>,
    ) -> Result<LoadOutcome, ConfigStoreError> {
        if changes.is_empty() {
            return Ok(LoadOutcome::Loaded);
        }
        config.revision += 1;
        write_atomically(path, config)?;
        tracing::info!(serial = %config.serial, ?changes, "configuration brought back to the placement rules");
        Ok(LoadOutcome::Normalized { changes })
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
        let name = image_name(bytes, kind);
        let path = self.image_path(serial, &name)?;
        if !path.exists() {
            self.check_room_for_images(serial, 1)?;
            write_bytes_atomically(&path, bytes)?;
        }
        Ok(name)
    }

    /// Archive of `config` (the current configuration of device `serial`)
    /// with the images it uses. An image missing from the disk is left out,
    /// and so is its reference: the archive stays importable.
    ///
    /// # Errors
    ///
    /// An I/O error on an image, or [`ConfigStoreError::Archive`].
    pub fn export(&self, serial: &str, config: &DeviceConfig) -> Result<Vec<u8>, ConfigStoreError> {
        let mut exported = config.clone();
        let mut images = ArchiveImages::new();
        for module in &mut exported.modules {
            let mut missing = HashSet::new();
            for name in self.image_names(module) {
                if images.contains_key(&name) || missing.contains(&name) {
                    continue;
                }
                let path = self.image_path(serial, &name)?;
                match fs::read(&path) {
                    Ok(bytes) => {
                        images.insert(name, bytes);
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        tracing::warn!(%serial, image = %name, "exported without its missing image");
                        missing.insert(name);
                    }
                    Err(error) => return Err(io_error(&path, error)),
                }
            }
            if module
                .image
                .as_ref()
                .is_some_and(|name| missing.contains(name))
            {
                module.image = None;
            }
            self.replace_settings_images(module, |name| {
                (!missing.contains(name)).then(|| name.to_owned())
            });
        }
        Ok(write_archive(&exported, &images)?)
    }

    /// Writes the images of `import` that device `serial` does not have yet;
    /// returns their names, for [`ConfigStore::remove_images`] should the
    /// configuration be refused.
    ///
    /// # Errors
    ///
    /// An I/O error (the images written so far are removed).
    pub fn write_import_images(
        &self,
        serial: &str,
        import: &PreparedImport,
    ) -> Result<Vec<String>, ConfigStoreError> {
        let mut new_images = 0;
        for (name, _) in &import.images {
            if !self.image_path(serial, name)?.exists() {
                new_images += 1;
            }
        }
        self.check_room_for_images(serial, new_images)?;
        let mut created = Vec::new();
        for (name, bytes) in &import.images {
            let path = self.image_path(serial, name)?;
            if path.exists() {
                continue;
            }
            if let Err(error) = write_bytes_atomically(&path, bytes) {
                self.remove_images(serial, &created);
                return Err(error);
            }
            created.push(name.clone());
        }
        Ok(created)
    }

    /// Removes images `names` of device `serial`, best effort (logged).
    pub fn remove_images(&self, serial: &str, names: &[String]) {
        for name in names {
            let removed = self
                .image_path(serial, name)
                .map_err(|error| error.to_string())
                .and_then(|path| fs::remove_file(path).map_err(|error| error.to_string()));
            if let Err(error) = removed {
                tracing::warn!(%serial, image = %name, %error, "cannot remove image");
            }
        }
    }

    /// Removes the images of device `serial` that `config` does not use
    /// (left by replaced images, abandoned uploads…), best effort (logged).
    /// Run when the configuration is loaded, before any upload can be under
    /// way.
    fn prune_images(&self, serial: &str, config: &DeviceConfig) {
        let used: HashSet<String> = config
            .modules
            .iter()
            .flat_map(|module| self.image_names(module))
            .collect();
        let Ok(dir) = self.device_dir(serial).map(|dir| dir.join(IMAGES_DIR)) else {
            return;
        };
        let Ok(entries) = fs::read_dir(&dir) else {
            return;
        };
        let unused: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| is_safe_image_name(name) && !used.contains(name))
            .collect();
        if !unused.is_empty() {
            tracing::info!(%serial, count = unused.len(), "removing unused images");
            self.remove_images(serial, &unused);
        }
    }

    /// Names of the images `module` uses: its own, and the icons in its
    /// settings.
    fn image_names(&self, module: &ModuleInstance) -> Vec<String> {
        module
            .image
            .iter()
            .cloned()
            .chain(
                self.descriptor_of(module)
                    .map_or_else(Vec::new, |descriptor| {
                        descriptor.settings_images(&module.settings)
                    }),
            )
            .collect()
    }

    /// Reads archive `bytes` and prepares it for `device` at its current
    /// `revision`, writing nothing: the configuration is fitted
    /// ([`fit_to_device`]) and validated, and every image it uses must pass
    /// `check` (the same check as uploads). Archived image names are only
    /// labels: images are named by content.
    ///
    /// # Errors
    ///
    /// [`ConfigStoreError::Archive`], [`ConfigStoreError::MissingImage`],
    /// [`ConfigStoreError::UnreadableImage`] or a validation error.
    pub fn prepare_import(
        &self,
        bytes: &[u8],
        device: &DeviceInfo,
        revision: u64,
        check: impl Fn(&[u8]) -> Result<ImageKind, String>,
    ) -> Result<PreparedImport, ConfigStoreError> {
        let Archive { config, images } = read_archive(bytes)?;
        let mut config = fit_to_device(config, device, revision);
        let changes = normalize_placement(&mut config, self.descriptors.as_ref());
        if !changes.is_empty() {
            tracing::info!(
                ?changes,
                "imported configuration brought back to the placement rules"
            );
        }
        let mut stager = ImageStager::new(&images, check);
        for module in &mut config.modules {
            if let Some(name) = module.image.take() {
                module.image = Some(stager.stage(&name)?);
            }
            if let Some(descriptor) = self.descriptor_of(module) {
                for name in descriptor.settings_images(&module.settings) {
                    stager.stage(&name)?;
                }
                descriptor
                    .replace_settings_images(&mut module.settings, |name| stager.stored_name(name));
            }
        }
        check_config(&config, device)?;
        Ok(PreparedImport {
            config,
            images: stager.prepared,
        })
    }

    /// What the module of `module` describes, `None` for a kind not provided.
    fn descriptor_of(&self, module: &ModuleInstance) -> Option<ModuleDescriptor> {
        self.descriptors.descriptor(&module.kind)
    }

    /// Replaces the images the settings of `module` refer to by
    /// `replace(name)`, `None` removing the reference.
    fn replace_settings_images(
        &self,
        module: &mut ModuleInstance,
        replace: impl FnMut(&str) -> Option<String>,
    ) {
        if let Some(descriptor) = self.descriptor_of(module) {
            descriptor.replace_settings_images(&mut module.settings, replace);
        }
    }

    /// Fails with [`ConfigStoreError::TooManyImages`] when `adding` more
    /// images would take device `serial` beyond [`MAX_STORED_IMAGES`]:
    /// uploads are bounded in size, not in number otherwise.
    fn check_room_for_images(&self, serial: &str, adding: usize) -> Result<(), ConfigStoreError> {
        let dir = self.device_dir(serial)?.join(IMAGES_DIR);
        let stored = match fs::read_dir(&dir) {
            Ok(entries) => entries.count(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
            Err(source) => return Err(io_error(&dir, source)),
        };
        if stored + adding > MAX_STORED_IMAGES {
            return Err(ConfigStoreError::TooManyImages);
        }
        Ok(())
    }

    /// Reader of the images of device `serial` for its module instances: an
    /// unsafe name, a missing or unreadable file gives `None`.
    #[must_use]
    pub fn image_library(&self, serial: &str) -> ImageLibrary {
        let store = self.clone();
        let serial = serial.to_owned();
        ImageLibrary::new(move |name| {
            let path = store.image_path(&serial, name).ok();
            Box::pin(async move {
                // Bounded like any source image, whatever the file became.
                let file = tokio::fs::File::open(path?).await.ok()?;
                let mut bytes = Vec::new();
                file.take(MAX_SOURCE_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .await
                    .ok()?;
                (bytes.len() <= MAX_SOURCE_BYTES).then(|| ImageData::new(bytes))
            })
        })
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

    /// Brings the folders of a device created before they were private
    /// (by an older version) back to [`PRIVATE_DIR_MODE`].
    fn make_device_private(&self, serial: &str) -> Result<(), ConfigStoreError> {
        let dir = self.device_dir(serial)?;
        restrict_existing_dir(&dir)?;
        restrict_existing_dir(&dir.join(IMAGES_DIR))
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

/// `config` (imported, possibly from another device or model) made for
/// `device` at its current `revision`: serial and model of the device, one
/// dial entry per dial (extra ones dropped, missing ones empty), and no
/// instance left without a key or a dial.
#[must_use]
pub fn fit_to_device(mut config: DeviceConfig, device: &DeviceInfo, revision: u64) -> DeviceConfig {
    config.serial.clone_from(&device.serial);
    config.model = device.model;
    config.revision = revision;
    config.dials.resize(
        usize::from(device.capabilities.dial_count),
        DialAssignment::None,
    );
    let used: HashSet<_> = config
        .home
        .iter()
        .chain(config.dials.iter().filter_map(DialAssignment::instance))
        .cloned()
        .collect();
    config.modules.retain(|module| used.contains(&module.id));
    config
}

/// Name of an image in the `images/` directory: from its content, so that
/// the same image is stored once and a name never changes content.
#[must_use]
pub fn image_name(bytes: &[u8], kind: ImageKind) -> String {
    let hex = hex_digest(&Sha256::digest(bytes));
    format!("{}.{}", &hex[..IMAGE_DIGEST_LENGTH], kind.extension())
}

/// An archive ready to be applied: its configuration fitted to the device,
/// naming its images as they will be stored, and those images, all checked.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedImport {
    /// Configuration to apply.
    pub config: DeviceConfig,
    /// Images used by `config`, by stored name.
    pub images: Vec<(String, Vec<u8>)>,
}

/// Names the images an archive uses as they will be stored (by content),
/// checking each once.
struct ImageStager<'a, C> {
    archived: &'a ArchiveImages,
    check: C,
    /// Stored name of each archived image already staged.
    stored: BTreeMap<String, String>,
    /// Images to write, by stored name.
    prepared: Vec<(String, Vec<u8>)>,
}

impl<'a, C: Fn(&[u8]) -> Result<ImageKind, String>> ImageStager<'a, C> {
    fn new(archived: &'a ArchiveImages, check: C) -> Self {
        Self {
            archived,
            check,
            stored: BTreeMap::new(),
            prepared: Vec::new(),
        }
    }

    /// The stored name of an archived image already staged.
    fn stored_name(&self, name: &str) -> Option<String> {
        self.stored.get(name).cloned()
    }

    /// The stored name of archived image `name`.
    fn stage(&mut self, name: &str) -> Result<String, ConfigStoreError> {
        if let Some(stored) = self.stored.get(name) {
            return Ok(stored.clone());
        }
        // A name no image of the archive can have: not echoed beyond the error's escaped form.
        let content = Some(name)
            .filter(|name| is_safe_image_name(name))
            .and_then(|name| self.archived.get(name))
            .ok_or_else(|| ConfigStoreError::MissingImage(name.to_owned()))?;
        let kind = (self.check)(content).map_err(|reason| ConfigStoreError::UnreadableImage {
            name: name.to_owned(),
            reason,
        })?;
        let stored = image_name(content, kind);
        if !self
            .prepared
            .iter()
            .any(|(existing, _)| *existing == stored)
        {
            self.prepared.push((stored.clone(), content.clone()));
        }
        self.stored.insert(name.to_owned(), stored.clone());
        Ok(stored)
    }
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
    migration::parse_config(&bytes)
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

/// Creates `dir` and its missing parents, readable by the user only (the
/// service umask would let other accounts list serials and images).
fn create_private_dir(dir: &Path) -> Result<(), ConfigStoreError> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIR_MODE)
        .create(dir)
        .map_err(|source| io_error(dir, source))
}

/// Sets an existing folder of the store to [`PRIVATE_DIR_MODE`]; a missing
/// one, or a link (never followed), is left alone.
fn restrict_existing_dir(dir: &Path) -> Result<(), ConfigStoreError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = match fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(io_error(dir, source)),
    };
    if !metadata.is_dir() || metadata.permissions().mode() & 0o777 == PRIVATE_DIR_MODE {
        return Ok(());
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(PRIVATE_DIR_MODE))
        .map_err(|source| io_error(dir, source))
}

/// Mode of the folders the store creates.
const PRIVATE_DIR_MODE: u32 = 0o700;

/// Writes `bytes` to a temporary file next to `path`, flushes it to disk and
/// renames it over `path`: readers see either the old or the new content.
fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> Result<(), ConfigStoreError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    create_private_dir(dir)?;
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
