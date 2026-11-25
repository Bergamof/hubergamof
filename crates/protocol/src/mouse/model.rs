//! Mouse models and the controls they have.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::MouseBattery;

/// A supported mouse model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MouseModel {
    /// Logitech MX Master 4.
    MxMaster4,
    /// Logitech MX Anywhere 3S.
    MxAnywhere3S,
}

impl fmt::Display for MouseModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// A button or wheel of a mouse that has its own place in the configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MouseControlId {
    /// Left click.
    Left,
    /// Right click.
    Right,
    /// Press on the scroll wheel.
    WheelClick,
    /// Small button that switches the wheel between ratchet and free spin.
    WheelMode,
    /// Back button of the thumb side.
    Back,
    /// Forward button of the thumb side.
    Forward,
    /// Gesture button of the thumb side.
    Gesture,
    /// Rotation of the thumb wheel.
    ThumbWheel,
    /// Touch panel of the thumb rest.
    HapticPanel,
}

impl fmt::Display for MouseControlId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Left => "left click",
            Self::Right => "right click",
            Self::WheelClick => "wheel click",
            Self::WheelMode => "wheel mode button",
            Self::Back => "back button",
            Self::Forward => "forward button",
            Self::Gesture => "gesture button",
            Self::ThumbWheel => "thumb wheel",
            Self::HapticPanel => "haptic panel",
        })
    }
}

/// How a control is operated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ControlKind {
    /// Pressed.
    Button,
    /// Turned.
    Rotation,
}

/// A control of a mouse model.
///
/// How the mouse itself numbers its buttons (HID++ control ids) is the
/// business of the `hubergamof-mouse` crate, not of this contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MouseControl {
    /// Which control.
    pub id: MouseControlId,
    /// How it is operated.
    pub kind: ControlKind,
    /// Cannot be given another action: left and right click always keep
    /// theirs, so that the mouse stays usable whatever the configuration says.
    pub locked: bool,
}

/// What a mouse model offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MouseCapabilities {
    /// Controls, in the order the interface lists them.
    pub controls: Vec<MouseControl>,
    /// Has a thumb wheel, with its own settings (it is also among the
    /// controls).
    pub thumb_wheel: bool,
    /// Can play haptic feedback.
    pub haptics: bool,
}

impl MouseCapabilities {
    /// The control, when the model has it.
    #[must_use]
    pub fn control(&self, id: MouseControlId) -> Option<&MouseControl> {
        self.controls.iter().find(|control| control.id == id)
    }
}

/// A mouse currently known to the manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MouseInfo {
    /// Serial number, used as the mouse identifier.
    pub serial: String,
    /// Model.
    pub model: MouseModel,
    /// Human-readable name shown in the UI.
    pub display_name: String,
    /// What the model offers.
    pub capabilities: MouseCapabilities,
    /// Its battery, once the mouse has said it; absent while unknown. The list of
    /// mice is sent again when it changes: every mouse shows its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<MouseBattery>,
}

const fn button(id: MouseControlId) -> MouseControl {
    MouseControl {
        id,
        kind: ControlKind::Button,
        locked: false,
    }
}

const fn locked(id: MouseControlId) -> MouseControl {
    MouseControl {
        locked: true,
        ..button(id)
    }
}

impl MouseModel {
    /// Name shown to people.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::MxMaster4 => "MX Master 4",
            Self::MxAnywhere3S => "MX Anywhere 3S",
        }
    }

    /// What the model offers, as read on the real mice (see
    /// `docs/mouse-hidpp.md`).
    #[must_use]
    pub fn capabilities(self) -> MouseCapabilities {
        use MouseControlId as Id;
        let common = [
            locked(Id::Left),
            locked(Id::Right),
            button(Id::WheelClick),
            button(Id::WheelMode),
            button(Id::Back),
            button(Id::Forward),
        ];
        match self {
            Self::MxMaster4 => MouseCapabilities {
                controls: common
                    .into_iter()
                    .chain([
                        button(Id::Gesture),
                        MouseControl {
                            id: Id::ThumbWheel,
                            kind: ControlKind::Rotation,
                            locked: false,
                        },
                        button(Id::HapticPanel),
                    ])
                    .collect(),
                thumb_wheel: true,
                haptics: true,
            },
            Self::MxAnywhere3S => MouseCapabilities {
                controls: common.to_vec(),
                thumb_wheel: false,
                haptics: false,
            },
        }
    }

    /// The control that opens the radial menu unless the user chooses another:
    /// the haptic panel when there is one, the wheel mode button otherwise.
    #[must_use]
    pub fn default_menu_control(self) -> MouseControlId {
        match self {
            Self::MxMaster4 => MouseControlId::HapticPanel,
            Self::MxAnywhere3S => MouseControlId::WheelMode,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODELS: [MouseModel; 2] = [MouseModel::MxMaster4, MouseModel::MxAnywhere3S];

    #[test]
    fn models_offer_what_was_read_on_the_mice() {
        let master = MouseModel::MxMaster4.capabilities();
        let anywhere = MouseModel::MxAnywhere3S.capabilities();
        assert_eq!(master.controls.len(), 9);
        assert_eq!(anywhere.controls.len(), 6);
        assert!(master.haptics && master.thumb_wheel);
        assert!(!anywhere.haptics && !anywhere.thumb_wheel);
        assert_eq!(
            master.control(MouseControlId::ThumbWheel).map(|c| c.kind),
            Some(ControlKind::Rotation)
        );
        assert_eq!(anywhere.control(MouseControlId::Gesture), None);
        assert_eq!(MouseModel::MxAnywhere3S.display_name(), "MX Anywhere 3S");
    }

    #[test]
    fn controls_are_distinct_and_only_clicks_are_locked() {
        for model in MODELS {
            let capabilities = model.capabilities();
            let locked: Vec<_> = capabilities
                .controls
                .iter()
                .filter(|control| control.locked)
                .map(|control| control.id)
                .collect();
            assert_eq!(
                locked,
                [MouseControlId::Left, MouseControlId::Right],
                "{model}"
            );
            let mut ids: Vec<_> = capabilities.controls.iter().map(|c| c.id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), capabilities.controls.len(), "{model}");
        }
    }

    #[test]
    fn the_capability_flags_agree_with_the_controls() {
        for model in MODELS {
            let capabilities = model.capabilities();
            let has = |id| capabilities.control(id).is_some();
            assert_eq!(
                capabilities.thumb_wheel,
                has(MouseControlId::ThumbWheel),
                "{model}"
            );
            assert_eq!(
                capabilities.haptics,
                has(MouseControlId::HapticPanel),
                "{model}"
            );
        }
    }

    #[test]
    fn the_menu_opens_from_a_free_button_the_model_has() {
        assert_eq!(
            MouseModel::MxMaster4.default_menu_control(),
            MouseControlId::HapticPanel
        );
        assert_eq!(
            MouseModel::MxAnywhere3S.default_menu_control(),
            MouseControlId::WheelMode
        );
        for model in MODELS {
            let control = model
                .capabilities()
                .control(model.default_menu_control())
                .copied();
            let control = control.unwrap_or_else(|| panic!("{model} lacks its menu control"));
            assert!(
                !control.locked && control.kind == ControlKind::Button,
                "{model}"
            );
        }
    }

    #[test]
    fn names_are_for_people() {
        assert_eq!(MouseModel::MxMaster4.to_string(), "MX Master 4");
        assert_eq!(MouseControlId::HapticPanel.to_string(), "haptic panel");
        assert_eq!(MouseControlId::Left.to_string(), "left click");
    }

    #[test]
    fn identifiers_travel_as_camel_case() {
        assert_eq!(
            serde_json::to_string(&MouseModel::MxAnywhere3S).unwrap(),
            "\"mxAnywhere3S\""
        );
        assert_eq!(
            serde_json::to_string(&MouseControlId::WheelMode).unwrap(),
            "\"wheelMode\""
        );
    }
}
