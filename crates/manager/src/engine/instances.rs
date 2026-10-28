//! Live module instances of a device, kept in sync with its configuration.

use std::collections::BTreeMap;

use async_trait::async_trait;
use bergadeck_module_api::{
    KeyContent, Module, ModuleContext, ModuleError, ModuleInstanceId, ModuleKind, RedrawHandle,
    TaskScope,
};
use bergadeck_modules::ModuleRegistry;
use bergadeck_protocol::ModuleInstance;
use serde_json::Value;

use crate::config_store::DefaultSettingsProvider;

impl DefaultSettingsProvider for ModuleRegistry {
    fn default_settings(&self, kind: &ModuleKind) -> Value {
        self.get(kind).map_or_else(
            || Value::Object(serde_json::Map::new()),
            |factory| factory.default_settings(),
        )
    }
}

/// Suffix of the label of a module that cannot run.
const UNAVAILABLE_SUFFIX: &str = "indisponible";

/// Stand-in for a module whose kind is unknown or whose creation failed.
struct UnavailableModule {
    label: String,
}

#[async_trait]
impl Module for UnavailableModule {
    fn key(&self) -> KeyContent {
        KeyContent::with_label(self.label.clone())
    }

    fn update_settings(&mut self, _settings: &Value) -> Result<(), ModuleError> {
        Ok(())
    }
}

struct Instance {
    kind: ModuleKind,
    /// Whether `module` is the real module rather than a placeholder.
    available: bool,
    module: Box<dyn Module>,
    /// Background tasks of the module, aborted when the instance is dropped.
    _tasks: TaskScope,
}

/// Module instances of a device by id.
pub(crate) struct Instances {
    registry: std::sync::Arc<ModuleRegistry>,
    redraw: RedrawHandle,
    live: BTreeMap<ModuleInstanceId, Instance>,
}

impl Instances {
    pub(crate) fn new(registry: std::sync::Arc<ModuleRegistry>, redraw: RedrawHandle) -> Self {
        Self {
            registry,
            redraw,
            live: BTreeMap::new(),
        }
    }

    pub(crate) fn get(&self, id: &ModuleInstanceId) -> Option<&dyn Module> {
        self.live.get(id).map(|instance| instance.module.as_ref())
    }

    pub(crate) fn get_mut(&mut self, id: &ModuleInstanceId) -> Option<&mut (dyn Module + 'static)> {
        self.live
            .get_mut(id)
            .map(|instance| instance.module.as_mut())
    }

    /// Makes the instances match `configured`: an instance keeping its id
    /// and kind receives the new settings and keeps its state; others are
    /// created or destroyed. A placeholder is retried when its kind became
    /// available.
    pub(crate) fn sync(&mut self, configured: &[ModuleInstance]) {
        let mut previous = std::mem::take(&mut self.live);
        for config in configured {
            let kept = previous
                .remove(&config.id)
                .filter(|instance| instance.kind == config.kind)
                .filter(|instance| instance.available || self.registry.get(&config.kind).is_none());
            let instance = match kept {
                Some(mut instance) => {
                    if let Err(error) = instance.module.update_settings(&config.settings) {
                        tracing::warn!(module = %config.id, %error, "settings rejected, keeping the previous ones");
                    }
                    instance
                }
                None => self.create(config),
            };
            self.live.insert(config.id.clone(), instance);
        }
        // Instances left in `previous` are dropped here, aborting their tasks.
    }

    /// Destroys every instance.
    pub(crate) fn clear(&mut self) {
        self.live.clear();
    }

    fn create(&self, config: &ModuleInstance) -> Instance {
        let tasks = TaskScope::new();
        let created = if let Some(factory) = self.registry.get(&config.kind) {
            let ctx = ModuleContext::new(config.id.clone(), self.redraw.clone(), tasks.spawner());
            factory.create(ctx, &config.settings).map_err(|error| {
                tracing::warn!(module = %config.id, kind = %config.kind, %error, "cannot create module");
            })
        } else {
            tracing::warn!(module = %config.id, kind = %config.kind, "unknown module kind");
            Err(())
        };
        let (available, module) = match created {
            Ok(module) => (true, module),
            Err(()) => (false, unavailable(&config.kind)),
        };
        Instance {
            kind: config.kind.clone(),
            available,
            module,
            _tasks: tasks,
        }
    }
}

fn unavailable(kind: &ModuleKind) -> Box<dyn Module> {
    Box::new(UnavailableModule {
        label: format!("{kind} {UNAVAILABLE_SUFFIX}"),
    })
}
