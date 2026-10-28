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

/// Module factories by kind.
#[derive(Clone, Default)]
pub struct ModuleRegistry {
    factories: BTreeMap<ModuleKind, Arc<dyn ModuleFactory>>,
}

impl ModuleRegistry {
    /// Registry without any module.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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

/// Registry of the built-in modules.
#[must_use]
pub fn builtin_registry() -> ModuleRegistry {
    let mut registry = ModuleRegistry::new();
    registry.register(Arc::new(clock::ClockFactory::system()));
    registry.register(Arc::new(navigation::NavigationFactory));
    registry
}

#[cfg(test)]
mod tests {
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
    fn registering_a_kind_twice_replaces_the_factory() {
        let mut registry = builtin_registry();
        let previous = registry.register(Arc::new(navigation::NavigationFactory));
        assert!(previous.is_some());
        assert_eq!(registry.descriptors().len(), 2);
    }
}
