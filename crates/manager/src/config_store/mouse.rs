//! Configuration of the mice and of the radial menu: one JSON document each,
//! `devices/<serial>/mouse.json` and `radial-menu.json` at the root, written
//! and guarded by `revision` like the device configurations. An invalid
//! document is set aside (`mouse.json.invalid-<time>`) and replaced by defaults.

use std::fs;
use std::io;
use std::path::Path;

use hubergamof_protocol::{MouseConfig, MouseModel, RadialMenuConfig};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::files::{io_error, set_invalid_aside, write_bytes_atomically};
use super::{ConfigStore, ConfigStoreError, LoadOutcome, MOUSE_FILE, RADIAL_MENU_FILE};

/// A document of the store that carries a revision and checks itself.
trait Document: Serialize + DeserializeOwned {
    fn revision(&self) -> u64;
    fn set_revision(&mut self, revision: u64);
    fn check(&self) -> Result<(), ConfigStoreError>;
}

impl Document for MouseConfig {
    fn revision(&self) -> u64 {
        self.revision
    }
    fn set_revision(&mut self, revision: u64) {
        self.revision = revision;
    }
    fn check(&self) -> Result<(), ConfigStoreError> {
        Ok(self.validate()?)
    }
}

impl Document for RadialMenuConfig {
    fn revision(&self) -> u64 {
        self.revision
    }
    fn set_revision(&mut self, revision: u64) {
        self.revision = revision;
    }
    fn check(&self) -> Result<(), ConfigStoreError> {
        Ok(self.validate()?)
    }
}

/// A configuration with how it was obtained.
#[derive(Debug, Clone)]
pub struct Loaded<T> {
    /// The configuration to use.
    pub config: T,
    /// Where it came from.
    pub outcome: LoadOutcome,
}

impl ConfigStore {
    /// Loads the configuration of a mouse, creating it from defaults when
    /// missing and setting an invalid or foreign file aside.
    ///
    /// # Errors
    /// [`ConfigStoreError::InvalidSerial`] or [`ConfigStoreError::Io`].
    pub fn load_mouse(
        &self,
        serial: &str,
        model: MouseModel,
    ) -> Result<Loaded<MouseConfig>, ConfigStoreError> {
        let _guard = self.lock();
        let path = self.device_dir(serial)?.join(MOUSE_FILE);
        load_or_default(
            &path,
            |config: &MouseConfig| {
                // Another mouse's file under this name is not ours to use.
                if config.serial == serial {
                    config.check()
                } else {
                    Err(ConfigStoreError::SerialMismatch {
                        expected: serial.to_owned(),
                        found: config.serial.clone(),
                    })
                }
            },
            || MouseConfig::default_for(serial, model),
        )
    }

    /// Saves the configuration of a mouse if it is valid and based on the
    /// current revision; returns it with its new revision.
    ///
    /// # Errors
    /// [`ConfigStoreError::StaleRevision`], a validation error, or an I/O or
    /// parse error on the current file.
    pub fn save_mouse(&self, config: MouseConfig) -> Result<MouseConfig, ConfigStoreError> {
        let _guard = self.lock();
        let path = self.device_dir(&config.serial)?.join(MOUSE_FILE);
        save_document(&path, config)
    }

    /// Loads the radial menu configuration, creating it from defaults when
    /// missing and setting an invalid file aside.
    ///
    /// # Errors
    /// [`ConfigStoreError::Io`].
    pub fn load_radial_menu(&self) -> Result<Loaded<RadialMenuConfig>, ConfigStoreError> {
        let _guard = self.lock();
        let path = self.root.join(RADIAL_MENU_FILE);
        load_or_default(&path, Document::check, RadialMenuConfig::default_menu)
    }

    /// Saves the radial menu configuration, like [`ConfigStore::save_mouse`].
    ///
    /// # Errors
    /// See [`ConfigStore::save_mouse`].
    pub fn save_radial_menu(
        &self,
        config: RadialMenuConfig,
    ) -> Result<RadialMenuConfig, ConfigStoreError> {
        let _guard = self.lock();
        save_document(&self.root.join(RADIAL_MENU_FILE), config)
    }
}

/// The stored document, or the default written in its place when there is
/// none or the stored one was invalid (set aside, not overwritten).
fn load_or_default<T: Document>(
    path: &Path,
    check: impl Fn(&T) -> Result<(), ConfigStoreError>,
    default: impl FnOnce() -> T,
) -> Result<Loaded<T>, ConfigStoreError> {
    let outcome = match fs::read(path) {
        Ok(bytes) => {
            let parsed =
                serde_json::from_slice::<T>(&bytes).map_err(|source| ConfigStoreError::Json {
                    path: path.to_owned(),
                    source,
                });
            match parsed.and_then(|config| check(&config).map(|()| config)) {
                Ok(config) => {
                    return Ok(Loaded {
                        config,
                        outcome: LoadOutcome::Loaded,
                    });
                }
                Err(reason) => set_invalid_aside(path, &reason)?,
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => LoadOutcome::CreatedDefault,
        Err(error) => return Err(io_error(path, error)),
    };
    let config = default();
    write_document(path, &config)?;
    Ok(Loaded { config, outcome })
}

fn save_document<T: Document>(path: &Path, mut config: T) -> Result<T, ConfigStoreError> {
    let current = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<T>(&bytes)
            .map_err(|source| ConfigStoreError::Json {
                path: path.to_owned(),
                source,
            })?
            .revision(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(io_error(path, error)),
    };
    if config.revision() != current {
        return Err(ConfigStoreError::StaleRevision {
            current,
            provided: config.revision(),
        });
    }
    config.check()?;
    config.set_revision(current + 1);
    write_document(path, &config)?;
    Ok(config)
}

fn write_document<T: Serialize>(path: &Path, config: &T) -> Result<(), ConfigStoreError> {
    let mut json = serde_json::to_vec_pretty(config).map_err(|source| ConfigStoreError::Json {
        path: path.to_owned(),
        source,
    })?;
    json.push(b'\n');
    write_bytes_atomically(path, &json)
}
