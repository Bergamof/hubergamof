//! Per-device configuration, persisted as `config/devices/<serial>/config.json`.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::{DeviceModel, ModuleInstanceId, ModuleKind};

/// Configuration schema version produced and understood by this crate.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Highest accepted brightness, in percent.
pub const MAX_BRIGHTNESS: u8 = 100;

/// Longest accepted image file name.
pub const MAX_IMAGE_NAME_LENGTH: usize = 128;

/// Full configuration of one device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceConfig {
    /// Schema version, see [`CURRENT_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Serial of the device this configuration belongs to.
    pub serial: String,
    /// Model of the device when the configuration was written.
    pub model: DeviceModel,
    /// Brightness in percent, `0..=100`.
    pub brightness: u8,
    /// Module instances shown on the home page, in key order.
    pub home: Vec<ModuleInstanceId>,
    /// All module instances of the device.
    pub modules: Vec<ModuleInstance>,
    /// Module instance assigned to each dial, by dial index.
    pub dials: Vec<Option<ModuleInstanceId>>,
    /// Revision counter, incremented on every accepted update; an update
    /// carrying an outdated revision is rejected.
    pub revision: u64,
}

/// A configured instance of a module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModuleInstance {
    /// Identifier, unique within the device configuration.
    pub id: ModuleInstanceId,
    /// Module kind this instance is built from.
    pub kind: ModuleKind,
    /// Label overriding the module default.
    pub label: Option<String>,
    /// Image file name in the device `images/` directory, overriding the
    /// module default.
    pub image: Option<String>,
    /// Module-specific settings, described by
    /// [`crate::ModuleDescriptor::settings`]. Free-form JSON so that modules
    /// (including future external ones) evolve without changing the contract.
    #[serde(default = "empty_settings")]
    pub settings: serde_json::Value,
}

fn empty_settings() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

/// Reason a [`DeviceConfig`] is rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfigError {
    /// The schema version is not supported by this version of Bergadeck.
    #[error("unsupported schema version {found} (supported: {CURRENT_SCHEMA_VERSION})")]
    UnsupportedSchemaVersion {
        /// Version found in the configuration.
        found: u32,
    },
    /// Brightness above [`MAX_BRIGHTNESS`].
    #[error("brightness {0} is above {MAX_BRIGHTNESS}")]
    BrightnessOutOfRange(u8),
    /// Two module instances share the same id.
    #[error("duplicate module id `{0}`")]
    DuplicateModuleId(ModuleInstanceId),
    /// The home page references a module instance that does not exist.
    #[error("home references unknown module `{0}`")]
    UnknownHomeModule(ModuleInstanceId),
    /// A dial references a module instance that does not exist.
    #[error("dial {dial} references unknown module `{module}`")]
    UnknownDialModule {
        /// Dial index.
        dial: usize,
        /// Unknown module id.
        module: ModuleInstanceId,
    },
    /// An image file name is empty or could escape the `images/` directory.
    #[error("module `{module}` has invalid image name `{image}`")]
    InvalidImageName {
        /// Module instance owning the image.
        module: ModuleInstanceId,
        /// Rejected file name.
        image: String,
    },
}

impl DeviceConfig {
    /// Checks the internal consistency of the configuration.
    ///
    /// Device-dependent checks (number of keys or dials) are left to the
    /// manager, which knows the connected hardware.
    ///
    /// # Errors
    ///
    /// Returns the first [`ConfigError`] found.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_schema_version()?;
        self.validate_brightness()?;
        let known_ids = self.validate_module_ids()?;
        self.validate_home(&known_ids)?;
        self.validate_dials(&known_ids)?;
        self.validate_images()
    }

    fn validate_schema_version(&self) -> Result<(), ConfigError> {
        if self.schema_version == CURRENT_SCHEMA_VERSION {
            Ok(())
        } else {
            Err(ConfigError::UnsupportedSchemaVersion {
                found: self.schema_version,
            })
        }
    }

    fn validate_brightness(&self) -> Result<(), ConfigError> {
        if self.brightness <= MAX_BRIGHTNESS {
            Ok(())
        } else {
            Err(ConfigError::BrightnessOutOfRange(self.brightness))
        }
    }

    fn validate_module_ids(&self) -> Result<HashSet<&ModuleInstanceId>, ConfigError> {
        let mut known_ids = HashSet::with_capacity(self.modules.len());
        for module in &self.modules {
            if !known_ids.insert(&module.id) {
                return Err(ConfigError::DuplicateModuleId(module.id.clone()));
            }
        }
        Ok(known_ids)
    }

    fn validate_home(&self, known_ids: &HashSet<&ModuleInstanceId>) -> Result<(), ConfigError> {
        match self.home.iter().find(|id| !known_ids.contains(id)) {
            Some(unknown) => Err(ConfigError::UnknownHomeModule(unknown.clone())),
            None => Ok(()),
        }
    }

    fn validate_dials(&self, known_ids: &HashSet<&ModuleInstanceId>) -> Result<(), ConfigError> {
        let unknown = self.dials.iter().enumerate().find_map(|(dial, module)| {
            module
                .as_ref()
                .filter(|id| !known_ids.contains(id))
                .map(|id| (dial, id))
        });
        match unknown {
            Some((dial, module)) => Err(ConfigError::UnknownDialModule {
                dial,
                module: module.clone(),
            }),
            None => Ok(()),
        }
    }

    fn validate_images(&self) -> Result<(), ConfigError> {
        for module in &self.modules {
            if let Some(image) = module
                .image
                .as_deref()
                .filter(|name| !is_safe_image_name(name))
            {
                return Err(ConfigError::InvalidImageName {
                    module: module.id.clone(),
                    image: image.to_owned(),
                });
            }
        }
        Ok(())
    }
}

/// Whether `name` is a plain file name of the device `images/` directory:
/// ASCII letters, digits, `-`, `_` and `.`, not starting with a dot (no
/// hidden file, no `..`), at most [`MAX_IMAGE_NAME_LENGTH`] characters. Such
/// a name cannot designate another directory.
#[must_use]
pub fn is_safe_image_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_IMAGE_NAME_LENGTH
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}
