//! What a module offers to the radial menu of the mice.

use serde::{Deserialize, Serialize};

/// An action of a module that the radial menu can run: a click on the petal of
/// the module, or an entry that opens around it.
///
/// The host runs it on an instance of the module that a device has (the first
/// one), through [`Module::on_radial_action`](crate::Module::on_radial_action);
/// like every other call it is refused while the computer is locked unless
/// the module declares itself usable then.
///
/// The order matters: [`ModuleFactory::radial_actions`](crate::ModuleFactory::radial_actions)
/// lists them, and the **first** is what a click on the petal of the module
/// does until the user picks another. A module that reorders its list for
/// display therefore changes that default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
// Not exported to TypeScript yet: the contract does not carry it. It will be
// once the catalog sends the actions to the configuration interface.
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[non_exhaustive]
pub struct RadialActionDescriptor {
    /// Identifier, passed back to the module when the action runs: letters,
    /// digits, `-` and `_`, unique within the module.
    pub id: String,
    /// Name shown in the configuration of the menu.
    pub name: String,
    /// What the action does.
    pub description: String,
}

impl RadialActionDescriptor {
    /// An action named `name` and described by `description`, run as `id`.
    #[must_use]
    pub fn new(id: &str, name: &str, description: &str) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_action_is_built_from_its_three_texts() {
        let action = RadialActionDescriptor::new("toggle", "Démarrer", "Lance ou met en pause");
        assert_eq!(action.id, "toggle");
        assert_eq!(action.name, "Démarrer");
        assert_eq!(action.description, "Lance ou met en pause");
    }
}
