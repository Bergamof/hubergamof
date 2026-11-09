//! Per-device configuration, persisted as `config/devices/<serial>/config.json`.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::{DeviceModel, ModuleInstanceId, ModuleKind, is_safe_image_name};

/// Configuration schema version produced and understood by this crate.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Highest accepted brightness, in percent.
pub const MAX_BRIGHTNESS: u8 = 100;

// Bounds of a configuration: well above any real use, so that an imported
// or received configuration cannot hold the manager back at every start.

/// Most module instances of a device.
pub const MAX_MODULES: usize = 256;

/// Most keys of the home page.
pub const MAX_HOME_KEYS: usize = 256;

/// Most dials of a configuration (4 on a Stream Deck+).
pub const MAX_DIALS: usize = 16;

/// Longest key label, in characters (a few words fit on a key).
pub const MAX_LABEL_CHARS: usize = 64;

/// Longest module instance id or module kind.
pub const MAX_IDENTIFIER_LENGTH: usize = 64;

/// Largest settings of an instance, serialized as JSON.
pub const MAX_SETTINGS_BYTES: usize = 16 * 1024;

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
    /// The schema version is not supported by this version of HuBergamof.
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
    /// More of something than a configuration may hold.
    #[error("too many {what}: {count} (at most {max})")]
    TooMany {
        /// What there are too many of.
        what: &'static str,
        /// How many there are.
        count: usize,
        /// The bound.
        max: usize,
    },
    /// A module instance id or kind is empty, too long or has characters
    /// other than ASCII letters, digits, `-` and `_` (shown escaped).
    #[error("invalid identifier {0:?}")]
    InvalidIdentifier(String),
    /// A label longer than [`MAX_LABEL_CHARS`].
    #[error("module `{0}` has a label longer than {MAX_LABEL_CHARS} characters")]
    LabelTooLong(ModuleInstanceId),
    /// Settings larger than [`MAX_SETTINGS_BYTES`] once serialized.
    #[error("module `{0}` has settings larger than {MAX_SETTINGS_BYTES} bytes")]
    SettingsTooLarge(ModuleInstanceId),
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
        self.validate_sizes()?;
        self.validate_instances()?;
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

    fn validate_sizes(&self) -> Result<(), ConfigError> {
        let counts = [
            ("module instances", self.modules.len(), MAX_MODULES),
            ("home keys", self.home.len(), MAX_HOME_KEYS),
            ("dials", self.dials.len(), MAX_DIALS),
        ];
        match counts.into_iter().find(|(_, count, max)| count > max) {
            Some((what, count, max)) => Err(ConfigError::TooMany { what, count, max }),
            None => Ok(()),
        }
    }

    /// Identifiers, labels and settings of each instance: also written to
    /// the logs and drawn on keys, so bounded and plain.
    fn validate_instances(&self) -> Result<(), ConfigError> {
        for module in &self.modules {
            for identifier in [&module.id.0, &module.kind.0] {
                if !is_identifier(identifier) {
                    return Err(ConfigError::InvalidIdentifier(identifier.clone()));
                }
            }
            if module
                .label
                .as_ref()
                .is_some_and(|label| label.chars().count() > MAX_LABEL_CHARS)
            {
                return Err(ConfigError::LabelTooLong(module.id.clone()));
            }
            let settings =
                serde_json::to_vec(&module.settings).map_or(usize::MAX, |json| json.len());
            if settings > MAX_SETTINGS_BYTES {
                return Err(ConfigError::SettingsTooLarge(module.id.clone()));
            }
        }
        Ok(())
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

/// Whether `identifier` is a valid module instance id or module kind: ASCII
/// letters, digits, `-` and `_`, at most [`MAX_IDENTIFIER_LENGTH`].
#[must_use]
pub fn is_identifier(identifier: &str) -> bool {
    !identifier.is_empty()
        && identifier.len() <= MAX_IDENTIFIER_LENGTH
        && identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}
