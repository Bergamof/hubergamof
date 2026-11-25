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
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
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

/// Most items the host draws around a petal: more would not fit in the arc of
/// a petal of a menu of eight. The extra ones are ignored.
pub const MAX_RADIAL_ITEMS: usize = 6;

/// Most characters of an item's badge the host draws.
pub const MAX_BADGE_CHARS: usize = 3;

/// The pictures an item can show. A closed list the host draws itself: a
/// module never sends an image to the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RadialGlyph {
    /// A microphone.
    Microphone,
    /// A camera.
    Camera,
    /// A handset put down: end a call.
    HangUp,
    /// A speech bubble: conversations.
    Chat,
    /// A bell: activity, notifications.
    Bell,
    /// Two people: teams, groups.
    People,
    /// An arrow back: return to what is running.
    ReturnToCall,
    /// A speaker.
    Speaker,
    /// Headphones: noise cancelling, or the headphones themselves.
    Headphones,
    /// An ear: ambient sound.
    Ear,
    /// A triangle: start.
    Play,
    /// Two bars: pause.
    Pause,
    /// A triangle and a bar: next step.
    Skip,
    /// A circular arrow: start over.
    Reset,
}

/// How an item looks, apart from its glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum RadialItemState {
    /// The usual look.
    #[default]
    Normal,
    /// What the item controls is off (microphone muted, camera cut): the glyph
    /// is crossed out.
    Off,
}

/// An action shown around the petal of a module while the pointer is on it.
///
/// Unlike the [`RadialActionDescriptor`]s of the catalogue, which are fixed,
/// the items describe the current state of an instance: they come and go (the
/// call controls exist during a call only), change their label ("Pause" then
/// "Resume") and carry a count. Choosing one runs
/// [`Module::on_radial_action`](crate::Module::on_radial_action) with its `id`,
/// which must be the `id` of an action of the catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RadialItem {
    /// The action it runs: an `id` of
    /// [`ModuleFactory::radial_actions`](crate::ModuleFactory::radial_actions).
    pub id: String,
    /// Name shown when the pointer is on the item.
    pub label: String,
    /// Picture.
    pub glyph: RadialGlyph,
    /// A short count or mark drawn on the item (unread conversations), cut
    /// after [`MAX_BADGE_CHARS`] characters.
    pub badge: Option<String>,
    /// Look of the item.
    pub state: RadialItemState,
}

impl RadialItem {
    /// An item that runs the action `id`, named `label`, drawn as `glyph`,
    /// without badge and in its usual state.
    #[must_use]
    pub fn new(id: &str, label: &str, glyph: RadialGlyph) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
            glyph,
            badge: None,
            state: RadialItemState::Normal,
        }
    }

    /// The same item with `badge` drawn on it.
    #[must_use]
    pub fn with_badge(mut self, badge: impl Into<String>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    /// The same item in `state`.
    #[must_use]
    pub const fn with_state(mut self, state: RadialItemState) -> Self {
        self.state = state;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_starts_plain_and_takes_a_badge_and_a_state() {
        let item = RadialItem::new("mute", "Micro", RadialGlyph::Microphone);
        assert_eq!(item.badge, None);
        assert_eq!(item.state, RadialItemState::Normal);
        let item = item.with_badge("3").with_state(RadialItemState::Off);
        assert_eq!(item.badge.as_deref(), Some("3"));
        assert_eq!(item.state, RadialItemState::Off);
    }

    #[test]
    fn an_action_is_built_from_its_three_texts() {
        let action = RadialActionDescriptor::new("toggle", "Démarrer", "Lance ou met en pause");
        assert_eq!(action.id, "toggle");
        assert_eq!(action.name, "Démarrer");
        assert_eq!(action.description, "Lance ou met en pause");
    }
}
