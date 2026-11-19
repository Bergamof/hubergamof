//! What a control does.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ModuleKind;

/// Longest keyboard shortcut, in characters.
pub const MAX_SHORTCUT_CHARS: usize = 64;

/// A mouse button, as a click a control can produce. Not the same as a
/// [`crate::MouseControlId`], which is a physical control of a model: a control
/// can be told to click any of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MouseButton {
    /// Left button.
    Left,
    /// Right button.
    Right,
    /// Middle button.
    Middle,
    /// Back button.
    Back,
    /// Forward button.
    Forward,
}

/// What a control does when the user gives it something other than what the
/// mouse does by itself. A control without an entry in
/// [`crate::MouseConfig::bindings`] is left alone; there is no entry to say so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ButtonAction {
    /// Nothing: the control is switched off.
    None,
    /// Clicks a mouse button.
    Click {
        /// The button clicked.
        button: MouseButton,
    },
    /// Double click.
    DoubleClick,
    /// Switches the wheel between ratchet and free spin.
    ToggleWheelMode,
    /// Opens the radial menu.
    RadialMenu,
    /// An action of a module (see the radial entries of the module). The
    /// manager ignores one whose module or action does not exist, rather than
    /// failing.
    Module {
        /// Module the action belongs to.
        module: ModuleKind,
        /// The action of that module.
        action: String,
    },
    /// Presses a keyboard shortcut, e.g. `Ctrl+Shift+M`.
    Shortcut {
        /// Keys joined by `+`.
        keys: String,
    },
}

/// A shortcut is key names joined by `+`: letters, digits, `_` and `-` only,
/// so that a configuration cannot carry anything else to the keyboard layer.
pub(super) fn is_shortcut(keys: &str) -> bool {
    !keys.is_empty()
        && keys.chars().count() <= MAX_SHORTCUT_CHARS
        && keys.split('+').all(|key| {
            !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_are_plus_joined_key_names() {
        for keys in [
            "Ctrl+Shift+M",
            "F5",
            "Super+Tab",
            "Alt+Left_Arrow",
            "Ctrl+Page-Up",
        ] {
            assert!(is_shortcut(keys), "{keys}");
        }
        let longest = "A".repeat(MAX_SHORTCUT_CHARS);
        assert!(is_shortcut(&longest), "exactly the limit");
        let too_long = "A".repeat(MAX_SHORTCUT_CHARS + 1);
        for keys in [
            "", "+", "Ctrl+", "+A", "Ctrl++A", "Ctrl+A B", "Ctrl+é", "a;rm", "A\nB", &too_long,
        ] {
            assert!(!is_shortcut(keys), "{keys:?}");
        }
    }

    #[test]
    fn actions_travel_as_tagged_camel_case() {
        let click = serde_json::to_value(ButtonAction::Click {
            button: MouseButton::Middle,
        })
        .unwrap();
        assert_eq!(
            click,
            serde_json::json!({ "kind": "click", "button": "middle" })
        );
        let radial = serde_json::to_value(ButtonAction::RadialMenu).unwrap();
        assert_eq!(radial, serde_json::json!({ "kind": "radialMenu" }));
        let module = ButtonAction::Module {
            module: "audio".into(),
            action: "mute".to_owned(),
        };
        let json = serde_json::to_value(&module).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "kind": "module", "module": "audio", "action": "mute" })
        );
        assert_eq!(
            serde_json::from_value::<ButtonAction>(json).unwrap(),
            module
        );
    }
}
