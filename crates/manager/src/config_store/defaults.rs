//! Configuration generated for a device seen for the first time.

use hubergamof_modules::ModuleRegistry;
use hubergamof_protocol::{
    CURRENT_SCHEMA_VERSION, DeviceConfig, DeviceInfo, ModuleInstance, ModuleInstanceId, ModuleKind,
};
use serde_json::Value;

/// Brightness of a new configuration, in percent.
pub const DEFAULT_BRIGHTNESS: u8 = 70;

/// Supplies what a new configuration contains: which modules go on the home
/// page and on the dials, and their settings. Implemented by the
/// [`ModuleRegistry`], which knows the modules.
pub trait ConfigDefaults: Send + Sync {
    /// Default settings of a new instance of `kind`.
    fn default_settings(&self, kind: &ModuleKind) -> Value;

    /// Kinds shown on the home page, in key order.
    fn home_modules(&self) -> Vec<ModuleKind>;

    /// Kinds assigned to the dials, from left to right; a device with fewer
    /// dials takes the first ones.
    fn dial_modules(&self) -> Vec<ModuleKind>;

    /// Names of the stored images the settings of `module` refer to (the
    /// icons of its items); none by default.
    fn settings_images(&self, _module: &ModuleInstance) -> Vec<String> {
        Vec::new()
    }

    /// Replaces every image the settings of `module` refer to by
    /// `replace(name)`, `None` removing the reference; nothing by default.
    fn replace_settings_images(
        &self,
        _module: &mut ModuleInstance,
        _replace: &mut dyn FnMut(&str) -> Option<String>,
    ) {
    }
}

impl ConfigDefaults for ModuleRegistry {
    fn default_settings(&self, kind: &ModuleKind) -> Value {
        ModuleRegistry::default_settings(self, kind)
    }

    fn home_modules(&self) -> Vec<ModuleKind> {
        self.default_layout().home.clone()
    }

    fn dial_modules(&self) -> Vec<ModuleKind> {
        self.default_layout().dials.clone()
    }

    fn settings_images(&self, module: &ModuleInstance) -> Vec<String> {
        self.get(&module.kind).map_or_else(Vec::new, |factory| {
            factory.descriptor().settings_images(&module.settings)
        })
    }

    fn replace_settings_images(
        &self,
        module: &mut ModuleInstance,
        replace: &mut dyn FnMut(&str) -> Option<String>,
    ) {
        if let Some(factory) = self.get(&module.kind) {
            factory
                .descriptor()
                .replace_settings_images(&mut module.settings, replace);
        }
    }
}

/// Default configuration of `device`: the default modules on the home page
/// and on the dials, one instance per kind whose id is the kind.
#[must_use]
pub fn default_config(device: &DeviceInfo, defaults: &dyn ConfigDefaults) -> DeviceConfig {
    let home = defaults.home_modules();
    let dials: Vec<Option<ModuleKind>> = {
        let mut kinds = defaults.dial_modules().into_iter();
        (0..device.capabilities.dial_count)
            .map(|_| kinds.next())
            .collect()
    };
    let mut kinds: Vec<&ModuleKind> = Vec::new();
    for kind in home.iter().chain(dials.iter().flatten()) {
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    let modules = kinds
        .into_iter()
        .map(|kind| ModuleInstance {
            id: instance_id(kind),
            settings: defaults.default_settings(kind),
            kind: kind.clone(),
            label: None,
            image: None,
        })
        .collect();
    DeviceConfig {
        schema_version: CURRENT_SCHEMA_VERSION,
        serial: device.serial.clone(),
        model: device.model,
        brightness: DEFAULT_BRIGHTNESS,
        home: home.iter().map(instance_id).collect(),
        modules,
        dials: dials
            .iter()
            .map(|kind| kind.as_ref().map(instance_id))
            .collect(),
        revision: 0,
    }
}

fn instance_id(kind: &ModuleKind) -> ModuleInstanceId {
    ModuleInstanceId(kind.0.clone())
}
