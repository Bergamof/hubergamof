//! Mice: models, what they offer, and the per-mouse configuration, persisted
//! as `config/devices/<serial>/mouse.json`.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::{HapticWaveform, ModuleKind, is_identifier};

/// Mouse configuration schema version produced and understood by this crate.
pub const MOUSE_SCHEMA_VERSION: u32 = 1;

/// Sensitivity range, in dots per inch, and its step: the same on every
/// supported mouse.
pub const DPI_RANGE: RangeInclusive<u16> = 200..=8000;

/// Step, in dots per inch, between two sensitivities.
pub const DPI_STEP: u16 = 50;

/// Speed from which the wheel leaves its ratchet in automatic mode (`SmartShift`).
pub const WHEEL_THRESHOLD_RANGE: RangeInclusive<u8> = 1..=50;

/// Speeds of the thumb wheel.
pub const THUMB_SPEED_RANGE: RangeInclusive<u8> = 1..=10;

/// Most haptic events of a configuration (every module offers a handful).
pub const MAX_HAPTIC_EVENTS: usize = 256;

/// Longest keyboard shortcut, in characters.
pub const MAX_SHORTCUT_CHARS: usize = 64;

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
    /// Control id the mouse itself gives the button (`ReprogControlsV4`),
    /// `None` for a wheel, which is not a button.
    pub cid: Option<u16>,
}

/// What a mouse model offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MouseCapabilities {
    /// Controls, in the order the interface lists them.
    pub controls: Vec<MouseControl>,
    /// Has a thumb wheel, with its own settings.
    pub thumb_wheel: bool,
    /// Can play haptic feedback.
    pub haptics: bool,
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
}

const fn button(id: MouseControlId, cid: u16) -> MouseControl {
    MouseControl {
        id,
        kind: ControlKind::Button,
        locked: false,
        cid: Some(cid),
    }
}

const fn locked(id: MouseControlId, cid: u16) -> MouseControl {
    MouseControl {
        locked: true,
        ..button(id, cid)
    }
}

/// Control ids read from the mice with `hubergamof mouse-probe` (see
/// `docs/mouse-hidpp.md`); the haptic panel's is the one unknown control of the
/// MX Master 4, not yet confirmed.
const CID_LEFT: u16 = 0x0050;
const CID_RIGHT: u16 = 0x0051;
const CID_WHEEL_CLICK: u16 = 0x0052;
const CID_BACK: u16 = 0x0053;
const CID_FORWARD: u16 = 0x0056;
const CID_GESTURE: u16 = 0x00C3;
const CID_WHEEL_MODE: u16 = 0x00C4;
const CID_HAPTIC_PANEL: u16 = 0x01A0;

impl MouseModel {
    /// Name shown to people.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::MxMaster4 => "MX Master 4",
            Self::MxAnywhere3S => "MX Anywhere 3S",
        }
    }

    /// What the model offers.
    #[must_use]
    pub fn capabilities(self) -> MouseCapabilities {
        use MouseControlId as Id;
        let common = [
            locked(Id::Left, CID_LEFT),
            locked(Id::Right, CID_RIGHT),
            button(Id::WheelClick, CID_WHEEL_CLICK),
            button(Id::WheelMode, CID_WHEEL_MODE),
            button(Id::Back, CID_BACK),
            button(Id::Forward, CID_FORWARD),
        ];
        match self {
            Self::MxMaster4 => MouseCapabilities {
                controls: common
                    .into_iter()
                    .chain([
                        button(Id::Gesture, CID_GESTURE),
                        MouseControl {
                            id: Id::ThumbWheel,
                            kind: ControlKind::Rotation,
                            locked: false,
                            cid: None,
                        },
                        button(Id::HapticPanel, CID_HAPTIC_PANEL),
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

/// A mouse button, as a click a control can produce.
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

/// What a control does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ButtonAction {
    /// What the mouse does by itself: the control is left alone.
    Native,
    /// Nothing.
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
    /// An action of a module (see the radial entries of the module).
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

/// Pointer settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PointerSettings {
    /// Sensitivity in dots per inch, within [`DPI_RANGE`], a multiple of
    /// [`DPI_STEP`] from its start.
    pub dpi: u16,
}

/// How the scroll wheel turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum WheelMode {
    /// One notch at a time.
    Ratchet,
    /// Free spin.
    Free,
    /// Free spin once thrown fast enough (`SmartShift`).
    Auto,
}

/// Scroll wheel settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WheelSettings {
    /// How the wheel turns.
    pub mode: WheelMode,
    /// Speed from which `Auto` mode frees the wheel, within
    /// [`WHEEL_THRESHOLD_RANGE`].
    pub auto_threshold: u8,
    /// Content follows the gesture, as on a touch screen.
    pub natural: bool,
    /// Finer, smoother scrolling where applications support it.
    pub hi_res: bool,
}

/// Thumb wheel settings (models that have one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThumbWheelSettings {
    /// Speed, within [`THUMB_SPEED_RANGE`].
    pub speed: u8,
    /// Reverse the direction.
    pub invert: bool,
}

/// Overall strength of the haptic feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum HapticLevel {
    /// Nothing is played.
    Off,
    /// Barely felt.
    Subtle,
    /// Light.
    Low,
    /// Medium.
    Medium,
    /// Strong.
    High,
}

/// How one haptic event of a module is signalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HapticEventSetting {
    /// Played at all.
    pub enabled: bool,
    /// The pattern played.
    pub waveform: HapticWaveform,
}

/// Haptic feedback settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HapticSettings {
    /// Overall strength.
    pub level: HapticLevel,
    /// Settings of the events the user changed, keyed `module.event` (the
    /// event ids a module declares); the others keep what the module declares.
    pub events: BTreeMap<String, HapticEventSetting>,
}

/// Full configuration of one mouse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MouseConfig {
    /// Schema version, see [`MOUSE_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Serial of the mouse this configuration belongs to.
    pub serial: String,
    /// Model of the mouse when the configuration was written.
    pub model: MouseModel,
    /// What each control does; a control left out is [`ButtonAction::Native`].
    pub bindings: BTreeMap<MouseControlId, ButtonAction>,
    /// Pointer settings.
    pub pointer: PointerSettings,
    /// Scroll wheel settings.
    pub wheel: WheelSettings,
    /// Thumb wheel settings, used by models that have one.
    pub thumb_wheel: ThumbWheelSettings,
    /// Haptic feedback, used by models that can play it.
    pub haptics: HapticSettings,
    /// Revision counter, incremented on every accepted update; an update
    /// carrying an outdated revision is rejected.
    pub revision: u64,
}

/// Why a mouse configuration is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MouseConfigError {
    /// The schema version is not supported by this version of HuBergamof.
    #[error("unsupported mouse schema version {found} (supported: {MOUSE_SCHEMA_VERSION})")]
    UnsupportedSchemaVersion {
        /// Version found in the configuration.
        found: u32,
    },
    /// A binding for a control the model does not have.
    #[error("{model:?} has no control {control:?}")]
    UnknownControl {
        /// Model of the configuration.
        model: MouseModel,
        /// Control that does not exist on it.
        control: MouseControlId,
    },
    /// A locked control given an action.
    #[error("control {0:?} cannot be given another action")]
    LockedControl(MouseControlId),
    /// An action a rotation cannot do.
    #[error("control {0:?} is turned, not pressed: it keeps its function or does nothing")]
    NotAButton(MouseControlId),
    /// More than one control opens the radial menu.
    #[error("the radial menu can be opened by one control only")]
    RadialMenuTwice,
    /// A module or action identifier that is not a plain identifier.
    #[error("invalid identifier {0:?}")]
    InvalidIdentifier(String),
    /// A shortcut that is empty, too long or made of anything but `+`-joined
    /// key names.
    #[error("invalid shortcut {0:?}")]
    InvalidShortcut(String),
    /// A sensitivity outside [`DPI_RANGE`] or not a multiple of [`DPI_STEP`].
    #[error("sensitivity of {0} dpi is not a multiple of {DPI_STEP} in {DPI_RANGE:?}")]
    InvalidDpi(u16),
    /// A wheel threshold outside [`WHEEL_THRESHOLD_RANGE`].
    #[error("wheel threshold {0} is outside {WHEEL_THRESHOLD_RANGE:?}")]
    InvalidWheelThreshold(u8),
    /// A thumb wheel speed outside [`THUMB_SPEED_RANGE`].
    #[error("thumb wheel speed {0} is outside {THUMB_SPEED_RANGE:?}")]
    InvalidThumbSpeed(u8),
    /// Haptic settings on a model that cannot play haptics.
    #[error("{0:?} has no haptic feedback")]
    HapticsUnsupported(MouseModel),
    /// Too many haptic events.
    #[error("{count} haptic events (at most {MAX_HAPTIC_EVENTS})")]
    TooManyHapticEvents {
        /// Number of events found.
        count: usize,
    },
    /// A haptic event key that is not `module.event`.
    #[error("invalid haptic event key {0:?}")]
    InvalidHapticEvent(String),
}

/// A shortcut is key names joined by `+`: letters, digits, `_` and `-` only,
/// so that a configuration cannot carry anything else to the keyboard layer.
fn is_shortcut(keys: &str) -> bool {
    !keys.is_empty()
        && keys.chars().count() <= MAX_SHORTCUT_CHARS
        && keys.split('+').all(|key| {
            !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

/// `module.event`, both parts plain identifiers.
fn is_haptic_event_key(key: &str) -> bool {
    key.split_once('.')
        .is_some_and(|(module, event)| is_identifier(module) && is_identifier(event))
}

impl MouseConfig {
    /// The configuration of a mouse seen for the first time: every control as
    /// the mouse has it, except the one that opens the radial menu.
    #[must_use]
    pub fn default_for(serial: &str, model: MouseModel) -> Self {
        Self {
            schema_version: MOUSE_SCHEMA_VERSION,
            serial: serial.to_owned(),
            model,
            bindings: BTreeMap::from([(model.default_menu_control(), ButtonAction::RadialMenu)]),
            pointer: PointerSettings { dpi: 1600 },
            wheel: WheelSettings {
                mode: WheelMode::Auto,
                auto_threshold: 20,
                natural: false,
                hi_res: true,
            },
            thumb_wheel: ThumbWheelSettings {
                speed: 5,
                invert: false,
            },
            haptics: HapticSettings {
                level: HapticLevel::Medium,
                events: BTreeMap::new(),
            },
            revision: 0,
        }
    }

    /// What this control does: its binding, or [`ButtonAction::Native`].
    #[must_use]
    pub fn action(&self, control: MouseControlId) -> &ButtonAction {
        self.bindings.get(&control).unwrap_or(&ButtonAction::Native)
    }

    /// Checks the configuration against the capabilities of its model.
    ///
    /// # Errors
    ///
    /// Returns the first [`MouseConfigError`] found.
    pub fn validate(&self) -> Result<(), MouseConfigError> {
        if self.schema_version != MOUSE_SCHEMA_VERSION {
            return Err(MouseConfigError::UnsupportedSchemaVersion {
                found: self.schema_version,
            });
        }
        self.validate_bindings()?;
        self.validate_settings()?;
        self.validate_haptics()
    }

    fn validate_bindings(&self) -> Result<(), MouseConfigError> {
        let capabilities = self.model.capabilities();
        let mut menu_controls = 0;
        for (control, action) in &self.bindings {
            let Some(known) = capabilities.controls.iter().find(|c| c.id == *control) else {
                return Err(MouseConfigError::UnknownControl {
                    model: self.model,
                    control: *control,
                });
            };
            if known.locked && *action != ButtonAction::Native {
                return Err(MouseConfigError::LockedControl(*control));
            }
            if known.kind == ControlKind::Rotation
                && !matches!(action, ButtonAction::Native | ButtonAction::None)
            {
                return Err(MouseConfigError::NotAButton(*control));
            }
            match action {
                ButtonAction::RadialMenu => menu_controls += 1,
                ButtonAction::Module { module, action } => {
                    for identifier in [&module.0, action] {
                        if !is_identifier(identifier) {
                            return Err(MouseConfigError::InvalidIdentifier(identifier.clone()));
                        }
                    }
                }
                ButtonAction::Shortcut { keys } if !is_shortcut(keys) => {
                    return Err(MouseConfigError::InvalidShortcut(keys.clone()));
                }
                _ => {}
            }
        }
        if menu_controls > 1 {
            return Err(MouseConfigError::RadialMenuTwice);
        }
        Ok(())
    }

    fn validate_settings(&self) -> Result<(), MouseConfigError> {
        let dpi = self.pointer.dpi;
        if !DPI_RANGE.contains(&dpi) || !(dpi - DPI_RANGE.start()).is_multiple_of(DPI_STEP) {
            return Err(MouseConfigError::InvalidDpi(dpi));
        }
        let threshold = self.wheel.auto_threshold;
        if !WHEEL_THRESHOLD_RANGE.contains(&threshold) {
            return Err(MouseConfigError::InvalidWheelThreshold(threshold));
        }
        let speed = self.thumb_wheel.speed;
        if !THUMB_SPEED_RANGE.contains(&speed) {
            return Err(MouseConfigError::InvalidThumbSpeed(speed));
        }
        Ok(())
    }

    fn validate_haptics(&self) -> Result<(), MouseConfigError> {
        let events = &self.haptics.events;
        if events.is_empty() {
            return Ok(());
        }
        if !self.model.capabilities().haptics {
            return Err(MouseConfigError::HapticsUnsupported(self.model));
        }
        if events.len() > MAX_HAPTIC_EVENTS {
            return Err(MouseConfigError::TooManyHapticEvents {
                count: events.len(),
            });
        }
        match events.keys().find(|key| !is_haptic_event_key(key)) {
            Some(key) => Err(MouseConfigError::InvalidHapticEvent(key.clone())),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERIAL: &str = "mx-test";

    fn config(model: MouseModel) -> MouseConfig {
        MouseConfig::default_for(SERIAL, model)
    }

    fn with_binding(
        model: MouseModel,
        control: MouseControlId,
        action: ButtonAction,
    ) -> MouseConfig {
        let mut config = config(model);
        config.bindings.insert(control, action);
        config
    }

    #[test]
    fn defaults_are_valid_and_open_the_menu_where_decided() {
        for model in [MouseModel::MxMaster4, MouseModel::MxAnywhere3S] {
            let config = config(model);
            assert_eq!(config.validate(), Ok(()), "{model:?}");
            let menu = model.default_menu_control();
            assert_eq!(config.action(menu), &ButtonAction::RadialMenu);
            assert!(
                !model
                    .capabilities()
                    .controls
                    .iter()
                    .find(|c| c.id == menu)
                    .unwrap()
                    .locked
            );
        }
        assert_eq!(
            MouseModel::MxMaster4.default_menu_control(),
            MouseControlId::HapticPanel
        );
        assert_eq!(
            MouseModel::MxAnywhere3S.default_menu_control(),
            MouseControlId::WheelMode
        );
    }

    #[test]
    fn models_offer_what_the_probe_found() {
        let master = MouseModel::MxMaster4.capabilities();
        let anywhere = MouseModel::MxAnywhere3S.capabilities();
        assert_eq!(master.controls.len(), 9);
        assert_eq!(anywhere.controls.len(), 6);
        assert!(master.haptics && master.thumb_wheel);
        assert!(!anywhere.haptics && !anywhere.thumb_wheel);
        for capabilities in [&master, &anywhere] {
            let locked: Vec<_> = capabilities
                .controls
                .iter()
                .filter(|c| c.locked)
                .map(|c| c.id)
                .collect();
            assert_eq!(locked, [MouseControlId::Left, MouseControlId::Right]);
            let mut ids: Vec<_> = capabilities.controls.iter().map(|c| c.id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(
                ids.len(),
                capabilities.controls.len(),
                "controls are distinct"
            );
        }
        let gesture = master
            .controls
            .iter()
            .find(|c| c.id == MouseControlId::Gesture)
            .unwrap();
        assert_eq!(gesture.cid, Some(0x00C3));
        let thumb = master
            .controls
            .iter()
            .find(|c| c.id == MouseControlId::ThumbWheel)
            .unwrap();
        assert_eq!((thumb.kind, thumb.cid), (ControlKind::Rotation, None));
        assert_eq!(MouseModel::MxAnywhere3S.display_name(), "MX Anywhere 3S");
    }

    #[test]
    fn a_control_the_model_lacks_is_refused() {
        let config = with_binding(
            MouseModel::MxAnywhere3S,
            MouseControlId::Gesture,
            ButtonAction::None,
        );
        assert_eq!(
            config.validate(),
            Err(MouseConfigError::UnknownControl {
                model: MouseModel::MxAnywhere3S,
                control: MouseControlId::Gesture
            })
        );
    }

    #[test]
    fn left_and_right_click_keep_their_function() {
        for control in [MouseControlId::Left, MouseControlId::Right] {
            let config = with_binding(MouseModel::MxMaster4, control, ButtonAction::None);
            assert_eq!(
                config.validate(),
                Err(MouseConfigError::LockedControl(control))
            );
            let native = with_binding(MouseModel::MxMaster4, control, ButtonAction::Native);
            assert_eq!(native.validate(), Ok(()));
        }
    }

    #[test]
    fn the_thumb_wheel_only_keeps_its_function_or_stops() {
        let thumb = MouseControlId::ThumbWheel;
        let click = ButtonAction::Click {
            button: MouseButton::Middle,
        };
        assert_eq!(
            with_binding(MouseModel::MxMaster4, thumb, click).validate(),
            Err(MouseConfigError::NotAButton(thumb))
        );
        assert_eq!(
            with_binding(MouseModel::MxMaster4, thumb, ButtonAction::None).validate(),
            Ok(())
        );
    }

    #[test]
    fn the_radial_menu_has_one_opener() {
        let mut config = config(MouseModel::MxMaster4);
        config
            .bindings
            .insert(MouseControlId::Gesture, ButtonAction::RadialMenu);
        assert_eq!(config.validate(), Err(MouseConfigError::RadialMenuTwice));
        config.bindings.remove(&MouseControlId::HapticPanel);
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn module_actions_need_plain_identifiers() {
        let good = ButtonAction::Module {
            module: "pomodoro".into(),
            action: "skip".to_owned(),
        };
        assert_eq!(
            with_binding(MouseModel::MxMaster4, MouseControlId::Gesture, good).validate(),
            Ok(())
        );
        for (module, action) in [("po mo", "skip"), ("pomodoro", ""), ("pomodoro", "a\nb")] {
            let bad = ButtonAction::Module {
                module: module.into(),
                action: action.to_owned(),
            };
            let result =
                with_binding(MouseModel::MxMaster4, MouseControlId::Gesture, bad).validate();
            assert!(
                matches!(result, Err(MouseConfigError::InvalidIdentifier(_))),
                "{module}/{action}"
            );
        }
    }

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
        for keys in [
            "",
            "+",
            "Ctrl+",
            "+A",
            "Ctrl++A",
            "Ctrl+A B",
            "Ctrl+é",
            "a;rm",
            &"A".repeat(65),
        ] {
            assert!(!is_shortcut(keys), "{keys:?}");
        }
        let bad = ButtonAction::Shortcut {
            keys: "Ctrl+".to_owned(),
        };
        assert_eq!(
            with_binding(MouseModel::MxMaster4, MouseControlId::Back, bad).validate(),
            Err(MouseConfigError::InvalidShortcut("Ctrl+".to_owned()))
        );
    }

    #[test]
    fn settings_stay_in_range() {
        let mut config = config(MouseModel::MxMaster4);
        for (dpi, ok) in [
            (200, true),
            (1650, true),
            (8000, true),
            (199, false),
            (225, false),
            (8050, false),
            (0, false),
        ] {
            config.pointer.dpi = dpi;
            assert_eq!(config.validate().is_ok(), ok, "{dpi} dpi");
        }
        config.pointer.dpi = 1600;
        config.wheel.auto_threshold = 0;
        assert_eq!(
            config.validate(),
            Err(MouseConfigError::InvalidWheelThreshold(0))
        );
        config.wheel.auto_threshold = 51;
        assert_eq!(
            config.validate(),
            Err(MouseConfigError::InvalidWheelThreshold(51))
        );
        config.wheel.auto_threshold = 50;
        config.thumb_wheel.speed = 11;
        assert_eq!(
            config.validate(),
            Err(MouseConfigError::InvalidThumbSpeed(11))
        );
        config.thumb_wheel.speed = 0;
        assert_eq!(
            config.validate(),
            Err(MouseConfigError::InvalidThumbSpeed(0))
        );
    }

    fn event(waveform: HapticWaveform) -> HapticEventSetting {
        HapticEventSetting {
            enabled: true,
            waveform,
        }
    }

    #[test]
    fn haptic_events_are_keyed_module_dot_event() {
        let mut config = config(MouseModel::MxMaster4);
        config
            .haptics
            .events
            .insert("teams.new-message".to_owned(), event(HapticWaveform::Knock));
        assert_eq!(config.validate(), Ok(()));
        for key in ["teams", "teams.", ".event", "te ams.x", "a.b.c"] {
            let mut bad = config.clone();
            bad.haptics
                .events
                .insert(key.to_owned(), event(HapticWaveform::Knock));
            assert!(
                matches!(bad.validate(), Err(MouseConfigError::InvalidHapticEvent(_))),
                "{key}"
            );
        }
    }

    #[test]
    fn haptic_events_need_a_haptic_mouse_and_stay_few() {
        let mut anywhere = config(MouseModel::MxAnywhere3S);
        anywhere
            .haptics
            .events
            .insert("teams.call".to_owned(), event(HapticWaveform::Ringing));
        assert_eq!(
            anywhere.validate(),
            Err(MouseConfigError::HapticsUnsupported(
                MouseModel::MxAnywhere3S
            ))
        );
        let mut master = config(MouseModel::MxMaster4);
        for index in 0..=MAX_HAPTIC_EVENTS {
            master
                .haptics
                .events
                .insert(format!("m.e{index}"), event(HapticWaveform::Wave));
        }
        assert_eq!(
            master.validate(),
            Err(MouseConfigError::TooManyHapticEvents {
                count: MAX_HAPTIC_EVENTS + 1
            })
        );
    }

    #[test]
    fn another_schema_version_is_refused() {
        let mut config = config(MouseModel::MxMaster4);
        config.schema_version = 2;
        assert_eq!(
            config.validate(),
            Err(MouseConfigError::UnsupportedSchemaVersion { found: 2 })
        );
    }

    #[test]
    fn a_configuration_survives_json_and_keeps_its_shape() {
        let mut config = with_binding(
            MouseModel::MxMaster4,
            MouseControlId::Gesture,
            ButtonAction::Module {
                module: "audio".into(),
                action: "mute".to_owned(),
            },
        );
        config
            .haptics
            .events
            .insert("teams.call".to_owned(), event(HapticWaveform::Ringing));
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["model"], "mxMaster4");
        assert_eq!(json["bindings"]["hapticPanel"]["kind"], "radialMenu");
        assert_eq!(json["bindings"]["gesture"]["module"], "audio");
        assert_eq!(json["pointer"]["dpi"], 1600);
        assert_eq!(json["wheel"]["autoThreshold"], 20);
        assert_eq!(
            json["haptics"]["events"]["teams.call"]["waveform"],
            "ringing"
        );
        let back: MouseConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back, config);
    }

    #[test]
    fn errors_say_what_is_wrong() {
        assert!(
            MouseConfigError::InvalidDpi(225)
                .to_string()
                .contains("225 dpi")
        );
        assert!(
            MouseConfigError::LockedControl(MouseControlId::Left)
                .to_string()
                .contains("Left")
        );
    }
}
