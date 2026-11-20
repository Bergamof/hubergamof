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
    use hubergamof_modules::{BuiltinServices, builtin_registry_with};
    use hubergamof_protocol::{RadialMenuConfig, RadialPrimary};

    #[test]
    fn every_entry_of_the_default_menu_is_an_action_a_builtin_module_declares() {
        let registry = builtin_registry_with(BuiltinServices::simulated());
        for entry in RadialMenuConfig::default_menu().entries {
            let factory = registry
                .get(&entry.module)
                .unwrap_or_else(|| panic!("no module `{}`", entry.module));
            let RadialPrimary::Action { action } = &entry.primary else {
                continue;
            };
            let declared: Vec<String> = factory
                .radial_actions()
                .into_iter()
                .map(|declared| declared.id)
                .collect();
            assert!(
                declared.contains(action),
                "`{}` declares {declared:?}, not `{action}`",
                entry.module
            );
        }
    }

    #[test]
    fn the_first_action_of_a_module_is_what_its_default_entry_does() {
        let registry = builtin_registry_with(BuiltinServices::simulated());
        for entry in RadialMenuConfig::default_menu().entries {
            let factory = registry.get(&entry.module).unwrap();
            let first = factory.radial_actions().remove(0).id;
            assert_eq!(
                entry.primary,
                RadialPrimary::Action { action: first },
                "the first action of `{}`",
                entry.module
            );
        }
    }

    #[test]
    fn the_radial_actions_of_each_module_have_plain_unique_ids() {
        let registry = builtin_registry_with(BuiltinServices::simulated());
        for factory in registry.factories() {
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
