//! The radial menu: where its petals are ([`geometry`]) and how it answers the
//! button that opens it and the pointer ([`interaction`]). Neither draws
//! anything or reads a device: the overlay and the mouse threads feed them.

pub mod actions;
pub mod backend;
pub mod controller;
pub mod geometry;
pub mod interaction;
pub mod overlay;
pub mod overlay_x11;
pub mod paint;
pub mod preview;
pub mod service;
pub mod window;

#[cfg(test)]
mod tests {
    use hubergamof_modules::{BuiltinServices, ModuleRegistry, builtin_registry_with};
    use hubergamof_protocol::{RadialMenuConfig, RadialPrimary};

    fn registry() -> ModuleRegistry {
        builtin_registry_with(BuiltinServices::simulated())
    }

    #[test]
    fn every_entry_of_the_default_menu_does_the_first_action_its_module_declares() {
        let registry = registry();
        for entry in RadialMenuConfig::default_menu().entries {
            let factory = registry
                .get(&entry.module)
                .unwrap_or_else(|| panic!("no module `{}`", entry.module));
            let declared = factory.radial_actions();
            let first = declared
                .first()
                .unwrap_or_else(|| panic!("`{}` declares no radial action", entry.module));
            assert_eq!(
                entry.primary,
                RadialPrimary::Action {
                    action: first.id.clone()
                },
                "the default entry of `{}` is the first action it declares",
                entry.module
            );
        }
    }

    #[test]
    fn the_radial_actions_of_each_module_have_plain_unique_ids() {
        for factory in registry().factories() {
            let mut seen = std::collections::HashSet::new();
            for action in factory.radial_actions() {
                assert!(
                    hubergamof_protocol::is_identifier(&action.id),
                    "`{}` is not a plain identifier",
                    action.id
                );
                assert!(seen.insert(action.id.clone()), "{} twice", action.id);
                assert!(!action.name.is_empty() && !action.description.is_empty());
            }
        }
    }
}
