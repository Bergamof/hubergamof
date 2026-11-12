//! Checks a configuration against the modules before it is saved: settings
//! accepted by each module and modules placed only where they can work.

use std::panic::{AssertUnwindSafe, catch_unwind};

use hubergamof_module_api::ModuleInstanceId;
use hubergamof_modules::ModuleRegistry;
use hubergamof_protocol::{DeviceConfig, DialAssignment};

use super::EngineError;
use crate::placement::violations;

/// Refuses `config` when a module of a known kind rejects its settings, or
/// is placed on the home page without being able to show a key, is configured
/// several times without allowing it, or is given to a dial for a function it
/// does not offer. Kinds missing from the
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
    if let Some(violation) = violations_of(registry, config) {
        return Err(violation);
    }
    for instance in &config.modules {
        let Some(factory) = registry.get(&instance.kind) else {
            continue;
        };
        let checked = guarded(&instance.id, || {
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

/// The first placement rule the configuration breaks. Module code is run
/// by [`violations`] under its own guard.
fn violations_of(registry: &ModuleRegistry, config: &DeviceConfig) -> Option<EngineError> {
    violations(config, registry)
        .into_iter()
        .next()
        .map(|violation| invalid(&violation.instance, violation.reason.into()))
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

    use hubergamof_module_api::ModuleCapabilities;

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

    fn default_config_with(edit: impl FnOnce(&mut DeviceConfig)) -> DeviceConfig {
        let registry = builtin_registry();
        let info = crate::device::virtual_deck::virtual_plus_info("S");
        let mut config = crate::config_store::default_config(&info, &registry);
        edit(&mut config);
        config
    }

    fn instance(id: &str, kind: &str) -> hubergamof_protocol::ModuleInstance {
        let registry = builtin_registry();
        hubergamof_protocol::ModuleInstance {
            id: id.into(),
            kind: kind.into(),
            label: None,
            image: None,
            settings: registry.default_settings(&kind.into()),
        }
    }

    #[test]
    fn the_default_configuration_follows_the_placement_rules() {
        let config = default_config_with(|_| {});
        assert!(check_modules(&builtin_registry(), &config).is_ok());
    }

    #[test]
    fn a_module_without_key_is_refused_on_the_home_page() {
        let config = default_config_with(|config| config.home.push("pomodoro".into()));
        let error = check_modules(&builtin_registry(), &config).unwrap_err();
        assert!(
            matches!(error, EngineError::InvalidConfig { module, .. } if module.0 == "pomodoro")
        );
    }

    #[test]
    fn a_single_instance_module_is_refused_on_two_keys() {
        let config = default_config_with(|config| config.home.push("claude".into()));
        let error = check_modules(&builtin_registry(), &config).unwrap_err();
        assert!(matches!(error, EngineError::InvalidConfig { module, .. } if module.0 == "claude"));
    }

    #[test]
    fn a_module_is_refused_on_two_dials_even_when_it_allows_several_instances() {
        let registry = builtin_registry();
        let twice = default_config_with(|config| config.dials[2] = config.dials[0].clone());
        assert!(check_modules(&registry, &twice).is_err());

        let twice_docker = default_config_with(|config| {
            config.modules.push(instance("docker-2", "docker"));
            for (dial, id) in [(0, "docker"), (1, "docker-2")] {
                config.dials[dial] = DialAssignment::Module {
                    instance: id.into(),
                    function: "none".into(),
                };
            }
        });
        let error = check_modules(&registry, &twice_docker).unwrap_err();
        assert!(
            matches!(error, EngineError::InvalidConfig { module, .. } if module.0 == "docker-2")
        );
    }

    #[test]
    fn a_module_is_configured_once_unless_it_says_otherwise() {
        let registry = builtin_registry();
        let twice_audio = default_config_with(|config| {
            config.modules.push(instance("audio-2", "audio"));
            config.home.push("audio-2".into());
        });
        let error = check_modules(&registry, &twice_audio).unwrap_err();
        assert!(
            matches!(error, EngineError::InvalidConfig { module, .. } if module.0 == "audio-2")
        );

        let twice_docker = default_config_with(|config| {
            config.modules.push(instance("docker-2", "docker"));
            config.home.push("docker-2".into());
        });
        assert!(check_modules(&registry, &twice_docker).is_ok());
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
                    capabilities: ModuleCapabilities {
                        key: true,
                        ..Default::default()
                    },
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
            "schemaVersion": 3,
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
