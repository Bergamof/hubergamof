//! Configuration generated for a device seen for the first time.

use bergadeck_protocol::{
    CURRENT_SCHEMA_VERSION, DeviceConfig, DeviceInfo, ModuleInstance, ModuleInstanceId, ModuleKind,
};

/// Brightness of a new configuration, in percent.
pub const DEFAULT_BRIGHTNESS: u8 = 70;

/// Built-in modules shown on the home page, in key order.
pub const DEFAULT_HOME_MODULES: [&str; 5] = ["audio", "pomodoro", "docker", "intellij", "claude"];

/// Modules assigned to the dials, from left to right; a device with fewer
/// dials takes the first ones.
pub const DEFAULT_DIAL_MODULES: [&str; 4] = ["clock", "audio", "pomodoro", "navigation"];

/// Supplies the default settings of each module kind (provided by the module
/// registry; [`EmptyDefaultSettings`] until modules exist).
pub trait DefaultSettingsProvider: Send + Sync {
    /// Default settings of a new instance of `kind`.
    fn default_settings(&self, kind: &ModuleKind) -> serde_json::Value;
}

/// Gives every module empty settings (`{}`).
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyDefaultSettings;

impl DefaultSettingsProvider for EmptyDefaultSettings {
    fn default_settings(&self, _kind: &ModuleKind) -> serde_json::Value {
        serde_json::Value::Object(serde_json::Map::new())
    }
}

/// Default configuration of `device`: built-in modules on the home page and
/// on the dials, each instance id being its module kind.
#[must_use]
pub fn default_config(device: &DeviceInfo, settings: &dyn DefaultSettingsProvider) -> DeviceConfig {
    let dials: Vec<Option<ModuleInstanceId>> = (0..usize::from(device.capabilities.dial_count))
        .map(|dial| {
            DEFAULT_DIAL_MODULES
                .get(dial)
                .map(|id| ModuleInstanceId::from(*id))
        })
        .collect();
    let mut kinds: Vec<&str> = DEFAULT_HOME_MODULES.to_vec();
    for id in dials.iter().flatten() {
        if !kinds.contains(&id.0.as_str()) {
            kinds.push(&id.0);
        }
    }
    let modules = kinds
        .into_iter()
        .map(|kind| {
            let kind = ModuleKind::from(kind);
            ModuleInstance {
                id: ModuleInstanceId(kind.0.clone()),
                settings: settings.default_settings(&kind),
                kind,
                label: None,
                image: None,
            }
        })
        .collect();
    DeviceConfig {
        schema_version: CURRENT_SCHEMA_VERSION,
        serial: device.serial.clone(),
        model: device.model,
        brightness: DEFAULT_BRIGHTNESS,
        home: DEFAULT_HOME_MODULES.map(ModuleInstanceId::from).to_vec(),
        modules,
        dials,
        revision: 0,
    }
}
