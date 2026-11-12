//! Configuration generated for a device seen for the first time.

use hubergamof_module_api::ModuleDescriptor;
use hubergamof_modules::{DefaultDial, ModuleRegistry};
use hubergamof_protocol::{
    CURRENT_SCHEMA_VERSION, DeviceConfig, DeviceInfo, DeviceTheme, DialAssignment, ModuleInstance,
    ModuleInstanceId, ModuleKind, StripConfig,
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

    /// What the dials are assigned, from left to right; a device with fewer
    /// dials takes the first ones, one with more leaves the others empty.
    fn dials(&self) -> Vec<DefaultDial>;
}

impl ConfigDefaults for ModuleRegistry {
    fn default_settings(&self, kind: &ModuleKind) -> Value {
        ModuleRegistry::default_settings(self, kind)
    }

    fn home_modules(&self) -> Vec<ModuleKind> {
        self.default_layout().home.clone()
    }

    fn dials(&self) -> Vec<DefaultDial> {
        self.default_layout().dials.clone()
    }
}

/// The settings the modules describe, for what a configuration holds besides
/// its own fields: the images its settings refer to.
pub trait ModuleDescriptors: Send + Sync {
    /// What module `kind` is and takes; `None` for a kind not provided.
    fn descriptor(&self, kind: &ModuleKind) -> Option<ModuleDescriptor>;
}

impl ModuleDescriptors for ModuleRegistry {
    fn descriptor(&self, kind: &ModuleKind) -> Option<ModuleDescriptor> {
        self.get(kind).map(|factory| factory.descriptor())
    }
}

/// Default configuration of `device`: the default modules on the home page
/// and the default function of each dial, one instance per kind whose id is
/// the kind.
#[must_use]
pub fn default_config(device: &DeviceInfo, defaults: &dyn ConfigDefaults) -> DeviceConfig {
    let home = defaults.home_modules();
    let mut wanted = defaults.dials().into_iter();
    let dials: Vec<DefaultDial> = (0..device.capabilities.dial_count)
        .map(|_| wanted.next().unwrap_or(DefaultDial::Nothing))
        .collect();
    let mut kinds: Vec<&ModuleKind> = Vec::new();
    let dial_kinds = dials.iter().filter_map(|dial| match dial {
        DefaultDial::Function { kind, .. } => Some(kind),
        DefaultDial::Navigation | DefaultDial::Contextual | DefaultDial::Nothing => None,
    });
    for kind in home.iter().chain(dial_kinds) {
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
        dials: dials.into_iter().map(assignment).collect(),
        strip: StripConfig::default(),
        theme: DeviceTheme::default(),
        revision: 0,
    }
}

fn assignment(dial: DefaultDial) -> DialAssignment {
    match dial {
        DefaultDial::Function { kind, function } => DialAssignment::Module {
            instance: instance_id(&kind),
            function,
        },
        DefaultDial::Navigation => DialAssignment::Navigation,
        DefaultDial::Contextual => DialAssignment::Contextual,
        DefaultDial::Nothing => DialAssignment::None,
    }
}

fn instance_id(kind: &ModuleKind) -> ModuleInstanceId {
    ModuleInstanceId(kind.0.clone())
}
