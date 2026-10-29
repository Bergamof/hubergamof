//! Built-in Bergadeck modules and the registry the manager instantiates them
//! from.
//!
//! Each module implements [`bergadeck_module_api::Module`] exactly like an
//! external module would. Available so far: [`clock`] and [`navigation`];
//! audio, pomodoro, docker, intellij and claude come later (the manager shows
//! a placeholder key for kinds missing from the registry).

pub mod clock;
pub mod navigation;

use std::collections::BTreeMap;
use std::sync::Arc;

use bergadeck_module_api::{ModuleDescriptor, ModuleFactory, ModuleKind};
use serde_json::{Map, Value};

/// Built-in modules shown on the home page of a new device, in key order.
/// Kinds not implemented yet show a placeholder key.
pub const DEFAULT_HOME_MODULES: [&str; 5] = ["audio", "pomodoro", "docker", "intellij", "claude"];

/// Built-in modules assigned to the dials of a new device, from left to
/// right; a device with fewer dials takes the first ones.
pub const DEFAULT_DIAL_MODULES: [&str; 4] = ["clock", "audio", "pomodoro", "navigation"];

/// Modules placed on a device seen for the first time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefaultLayout {
    /// Kinds shown on the home page, in key order.
    pub home: Vec<ModuleKind>,
    /// Kinds assigned to the dials, from left to right.
    pub dials: Vec<ModuleKind>,
}

impl DefaultLayout {
    /// Layout of the built-in modules.
    #[must_use]
    pub fn builtin() -> Self {
        Self {
            home: DEFAULT_HOME_MODULES.map(ModuleKind::from).to_vec(),
            dials: DEFAULT_DIAL_MODULES.map(ModuleKind::from).to_vec(),
        }
    }
}

/// Module factories by kind, and the layout of a new device.
#[derive(Clone, Default)]
pub struct ModuleRegistry {
    factories: BTreeMap<ModuleKind, Arc<dyn ModuleFactory>>,
    layout: DefaultLayout,
}

impl ModuleRegistry {
    /// Registry without any module, with an empty default layout.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Modules placed on a device seen for the first time.
    #[must_use]
    pub fn default_layout(&self) -> &DefaultLayout {
        &self.layout
    }

    /// Replaces the default layout.
    pub fn set_default_layout(&mut self, layout: DefaultLayout) {
        self.layout = layout;
    }

    /// Settings of a new instance of `kind`: those of its factory, or an
    /// empty object for a kind missing from the registry.
    #[must_use]
    pub fn default_settings(&self, kind: &ModuleKind) -> Value {
        self.get(kind).map_or_else(
            || Value::Object(Map::new()),
            |factory| factory.default_settings(),
        )
    }

    /// Registers `factory` under the kind of its descriptor, replacing and
    /// returning a factory already registered for that kind.
    pub fn register(&mut self, factory: Arc<dyn ModuleFactory>) -> Option<Arc<dyn ModuleFactory>> {
        self.factories.insert(factory.descriptor().kind, factory)
    }

    /// Factory of `kind`, if registered.
    #[must_use]
    pub fn get(&self, kind: &ModuleKind) -> Option<&Arc<dyn ModuleFactory>> {
        self.factories.get(kind)
    }

    /// Descriptors of every registered module, ordered by kind.
    #[must_use]
    pub fn descriptors(&self) -> Vec<ModuleDescriptor> {
        self.factories
            .values()
            .map(|factory| factory.descriptor())
            .collect()
    }
}

impl std::fmt::Debug for ModuleRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.factories.keys()).finish()
    }
}

/// Registry of the built-in modules, with their default layout.
#[must_use]
pub fn builtin_registry() -> ModuleRegistry {
    let mut registry = ModuleRegistry::new();
    registry.set_default_layout(DefaultLayout::builtin());
    registry.register(Arc::new(clock::ClockFactory::system()));
    registry.register(Arc::new(navigation::NavigationFactory));
    registry
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn builtin_registry_offers_clock_and_navigation() {
        let registry = builtin_registry();
        let kinds: Vec<_> = registry
            .descriptors()
            .into_iter()
            .map(|descriptor| descriptor.kind.0)
            .collect();
        assert_eq!(kinds, [clock::CLOCK_KIND, navigation::NAVIGATION_KIND]);
        assert!(registry.get(&"clock".into()).is_some());
        assert!(registry.get(&"audio".into()).is_none());
    }

    #[test]
    fn default_settings_come_from_the_factory() {
        let registry = builtin_registry();
        assert_eq!(
            registry.default_settings(&clock::CLOCK_KIND.into()),
            clock::ClockFactory::system().default_settings()
        );
        assert_eq!(registry.default_settings(&"audio".into()), json!({}));
    }

    #[test]
    fn builtin_layout_fills_home_and_dials() {
        let registry = builtin_registry();
        let layout = registry.default_layout();
        assert_eq!(layout.home.len(), DEFAULT_HOME_MODULES.len());
        assert_eq!(layout.dials[0], ModuleKind::from(clock::CLOCK_KIND));
        assert_eq!(
            ModuleRegistry::new().default_layout(),
            &DefaultLayout::default()
        );
    }

    #[test]
    fn registering_a_kind_twice_replaces_the_factory() {
        let mut registry = builtin_registry();
        let previous = registry.register(Arc::new(navigation::NavigationFactory));
        assert!(previous.is_some());
        assert_eq!(registry.descriptors().len(), 2);
    }
}
