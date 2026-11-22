//! The modules that live in the application, apart from any device.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::config::{ConfigError, MAX_MODULES, ModuleInstance, validate_module_instance};

/// Version of [`AppModulesConfig`] this version of HuBergamof reads and writes.
pub const APP_MODULES_SCHEMA_VERSION: u32 = 1;

/// The instances of the modules that run in the application: they work
/// whether or not a device is connected, and devices, mice and the interface
/// only show or drive them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppModulesConfig {
    /// Schema version, see [`APP_MODULES_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The instances. A kind that is not meant to live in the application, or
    /// that is unknown, is ignored by the manager.
    pub modules: Vec<ModuleInstance>,
    /// Revision counter, incremented on every accepted update; an update
    /// carrying an outdated revision is rejected.
    pub revision: u64,
}

/// Why a configuration of the application modules is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AppModulesConfigError {
    /// The schema version is not supported by this version of HuBergamof.
    #[error(
        "unsupported application modules schema version {found} \
         (supported: {APP_MODULES_SCHEMA_VERSION})"
    )]
    UnsupportedSchemaVersion {
        /// Version found in the configuration.
        found: u32,
    },
    /// An instance is out of bounds, or two share an id.
    #[error(transparent)]
    Instance(#[from] ConfigError),
}

impl AppModulesConfig {
    /// A configuration without any instance.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema_version: APP_MODULES_SCHEMA_VERSION,
            modules: Vec::new(),
            revision: 0,
        }
    }

    /// Checks the internal consistency of the configuration, with the same
    /// bounds as the instances of a device: it is read from a file the user
    /// can edit.
    ///
    /// # Errors
    ///
    /// The first [`AppModulesConfigError`] found.
    pub fn validate(&self) -> Result<(), AppModulesConfigError> {
        if self.schema_version != APP_MODULES_SCHEMA_VERSION {
            return Err(AppModulesConfigError::UnsupportedSchemaVersion {
                found: self.schema_version,
            });
        }
        if self.modules.len() > MAX_MODULES {
            return Err(ConfigError::TooMany {
                what: "module instances",
                count: self.modules.len(),
                max: MAX_MODULES,
            }
            .into());
        }
        let mut known = HashSet::with_capacity(self.modules.len());
        for module in &self.modules {
            validate_module_instance(module)?;
            // There is no image folder for the application.
            if let Some(image) = &module.image {
                return Err(ConfigError::InvalidImageName {
                    module: module.id.clone(),
                    image: image.clone(),
                }
                .into());
            }
            if !known.insert(&module.id) {
                return Err(ConfigError::DuplicateModuleId(module.id.clone()).into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn instance(id: &str, kind: &str) -> ModuleInstance {
        ModuleInstance {
            id: id.into(),
            kind: kind.into(),
            label: None,
            image: None,
            settings: json!({}),
        }
    }

    fn config(modules: Vec<ModuleInstance>) -> AppModulesConfig {
        AppModulesConfig {
            modules,
            ..AppModulesConfig::empty()
        }
    }

    #[test]
    fn an_empty_configuration_and_plain_instances_are_valid() {
        assert_eq!(AppModulesConfig::empty().validate(), Ok(()));
        let modules = vec![instance("app-pomodoro", "pomodoro"), instance("b", "audio")];
        assert_eq!(config(modules).validate(), Ok(()));
    }

    #[test]
    fn another_schema_version_is_refused() {
        let mut other = AppModulesConfig::empty();
        other.schema_version = 99;
        assert_eq!(
            other.validate(),
            Err(AppModulesConfigError::UnsupportedSchemaVersion { found: 99 })
        );
    }

    #[test]
    fn instances_are_bounded_like_those_of_a_device() {
        let bad_id = config(vec![instance("../x", "pomodoro")]);
        assert!(matches!(
            bad_id.validate(),
            Err(AppModulesConfigError::Instance(
                ConfigError::InvalidIdentifier(_)
            ))
        ));

        let mut huge = instance("a", "pomodoro");
        huge.settings = json!({ "text": "x".repeat(crate::MAX_SETTINGS_BYTES) });
        assert!(matches!(
            config(vec![huge]).validate(),
            Err(AppModulesConfigError::Instance(
                ConfigError::SettingsTooLarge(_)
            ))
        ));

        let many = (0..=MAX_MODULES)
            .map(|index| instance(&format!("m{index}"), "pomodoro"))
            .collect();
        assert!(matches!(
            config(many).validate(),
            Err(AppModulesConfigError::Instance(ConfigError::TooMany { .. }))
        ));
    }

    #[test]
    fn duplicate_ids_and_images_are_refused() {
        let twice = config(vec![instance("a", "pomodoro"), instance("a", "audio")]);
        assert!(matches!(
            twice.validate(),
            Err(AppModulesConfigError::Instance(
                ConfigError::DuplicateModuleId(_)
            ))
        ));

        let mut with_image = instance("a", "pomodoro");
        with_image.image = Some("icon.png".into());
        assert!(matches!(
            config(vec![with_image]).validate(),
            Err(AppModulesConfigError::Instance(
                ConfigError::InvalidImageName { .. }
            ))
        ));
    }
}
