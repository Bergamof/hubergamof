//! Checks a configuration against the modules before it is saved: settings
//! accepted by each module and modules placed only where they can work.

use std::panic::{AssertUnwindSafe, catch_unwind};

use hubergamof_module_api::{ModuleCapabilities, ModuleInstanceId};
use hubergamof_modules::ModuleRegistry;
use hubergamof_protocol::{DeviceConfig, DialAssignment};

use super::EngineError;

/// Refuses `config` when a module of a known kind rejects its settings, or
/// is placed on the home page without being able to show a key, or is given
/// to a dial for a function it does not offer. Kinds missing from the
/// registry are not checked (they show a placeholder); references to unknown
/// instances are left to [`DeviceConfig::validate`].
///
/// # Errors
///
/// [`EngineError::InvalidConfig`] naming the first offending instance.
pub(crate) fn check_modules(
    registry: &ModuleRegistry,
    config: &DeviceConfig,
) -> Result<(), EngineError> {
    for instance in &config.modules {
        let Some(factory) = registry.get(&instance.kind) else {
            continue;
        };
        let checked = guarded(&instance.id, || {
            let capabilities = factory.descriptor().capabilities;
            check_placement(config, &instance.id, capabilities)?;
            factory
                .validate_settings(&instance.settings)
                .map_err(|error| invalid(&instance.id, error.to_string()))
        });
        checked?;
    }
    check_dial_functions(registry, config)
}

/// Every function given to a dial is offered by the module of its instance.
fn check_dial_functions(
    registry: &ModuleRegistry,
    config: &DeviceConfig,
) -> Result<(), EngineError> {
    for assignment in &config.dials {
        let DialAssignment::Module { instance, function } = assignment else {
            continue;
        };
        let Some(factory) = config
            .modules
            .iter()
            .find(|module| module.id == *instance)
            .and_then(|module| registry.get(&module.kind))
        else {
            continue;
        };
        let offered = guarded(instance, || {
            let functions = factory.descriptor().dial_functions;
            functions
                .iter()
                .any(|offered| offered.id == *function)
                .then_some(())
                .ok_or_else(|| {
                    invalid(
                        instance,
                        format!("this module has no dial function `{function}`"),
                    )
                })
        });
        offered?;
    }
    Ok(())
}

fn check_placement(
    config: &DeviceConfig,
    id: &ModuleInstanceId,
    capabilities: ModuleCapabilities,
) -> Result<(), EngineError> {
    if !capabilities.key && config.home.contains(id) {
        return Err(invalid(id, "this module cannot be placed on a key".into()));
    }
    Ok(())
}

/// Runs module code, turning a panic into a refusal of the configuration.
fn guarded(
    id: &ModuleInstanceId,
    check: impl FnOnce() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    catch_unwind(AssertUnwindSafe(check)).unwrap_or_else(|_| {
        tracing::error!(module = %id, "module panicked validating its configuration");
        Err(invalid(
            id,
            "the module failed to check its settings".into(),
        ))
    })
}

fn invalid(module: &ModuleInstanceId, reason: String) -> EngineError {
    EngineError::InvalidConfig {
        module: module.clone(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::testing::builtin_registry;

    #[test]
    fn example_configurations_pass_the_builtin_modules() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/example");
        let registry = builtin_registry();
        let mut checked = 0;
        for entry in std::fs::read_dir(&examples).unwrap() {
            let path = entry.unwrap().path().join("config.json");
            let config: DeviceConfig =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            if let Err(error) = check_modules(&registry, &config) {
                panic!("{}: {error}", path.display());
            }
            checked += 1;
        }
        assert!(checked > 0, "no example configuration found");
    }

    #[test]
    fn modules_panicking_while_validating_refuse_the_configuration() {
        use std::sync::Arc;

        use hubergamof_module_api::{
            Module, ModuleContext, ModuleDescriptor, ModuleError, ModuleFactory,
        };
        use serde_json::{Value, json};

        struct Panicking;
        impl ModuleFactory for Panicking {
            fn descriptor(&self) -> ModuleDescriptor {
                ModuleDescriptor {
                    kind: "panicking".into(),
                    name: String::new(),
                    description: String::new(),
                    capabilities: ModuleCapabilities { key: true },
                    dial_functions: Vec::new(),
                    settings: Vec::new(),
                }
            }

            fn validate_settings(&self, _settings: &Value) -> Result<(), ModuleError> {
                panic!("validation failure")
            }

            fn create(
                &self,
                _ctx: ModuleContext,
                _settings: &Value,
            ) -> Result<Box<dyn Module>, ModuleError> {
                Err(ModuleError::InvalidSettings("unused".into()))
            }
        }

        let mut registry = builtin_registry();
        registry.register(Arc::new(Panicking));
        let mut config: DeviceConfig = serde_json::from_value(json!({
            "schemaVersion": 2,
            "serial": "S",
            "model": "plus",
            "brightness": 50,
            "home": ["p"],
            "modules": [{ "id": "p", "kind": "panicking" }],
            "dials": [{ "kind": "none" }, { "kind": "none" }, { "kind": "none" }, { "kind": "none" }],
            "revision": 0
        }))
        .unwrap();
        let error = check_modules(&registry, &config).unwrap_err();
        assert!(matches!(error, EngineError::InvalidConfig { module, .. } if module.0 == "p"));
        config.modules[0].kind = "teleporter".into();
        assert!(check_modules(&registry, &config).is_ok());
    }
}
