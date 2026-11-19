//! The configuration of one mouse and its validation.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use super::action::is_shortcut;
use super::haptic::{MAX_HAPTIC_EVENTS, is_event_key};
use super::{ButtonAction, ControlKind, HapticLevel, HapticSettings, MouseControlId, MouseModel};
use crate::is_identifier;

/// Mouse configuration schema version produced and understood by this crate.
pub const MOUSE_SCHEMA_VERSION: u32 = 1;

/// Sensitivity range, in dots per inch: the same on every supported mouse.
pub const DPI_RANGE: RangeInclusive<u16> = 200..=8000;

/// Step, in dots per inch, between two sensitivities, counted from the start
/// of [`DPI_RANGE`].
pub const DPI_STEP: u16 = 50;

/// Speed from which the wheel leaves its ratchet in automatic mode (`SmartShift`).
pub const WHEEL_THRESHOLD_RANGE: RangeInclusive<u8> = 1..=50;

/// Speeds of the thumb wheel.
pub const THUMB_SPEED_RANGE: RangeInclusive<u8> = 1..=10;

/// Sensitivity of a mouse seen for the first time.
pub const DEFAULT_DPI: u16 = 1600;

/// Automatic wheel threshold of a mouse seen for the first time.
pub const DEFAULT_WHEEL_THRESHOLD: u8 = 20;

/// Thumb wheel speed of a mouse seen for the first time.
pub const DEFAULT_THUMB_SPEED: u8 = 5;

/// Pointer settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PointerSettings {
    /// Sensitivity in dots per inch: within [`DPI_RANGE`] and a multiple of
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

/// Thumb wheel settings. Always present and always checked; a model without
/// a thumb wheel simply does not use them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThumbWheelSettings {
    /// Speed, within [`THUMB_SPEED_RANGE`].
    pub speed: u8,
    /// Reverse the direction.
    pub invert: bool,
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
    /// What the controls do when it is not what the mouse does by itself. A
    /// control without an entry is left alone: there is one single way to say
    /// so. Locked controls never have an entry, and a thumb wheel only
    /// accepts [`ButtonAction::None`].
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
    #[error("{model} has no {control}")]
    UnknownControl {
        /// Model of the configuration.
        model: MouseModel,
        /// Control that does not exist on it.
        control: MouseControlId,
    },
    /// A locked control given an entry.
    #[error("the {0} cannot be given another action")]
    LockedControl(MouseControlId),
    /// An action a rotation cannot do.
    #[error("the {0} is turned, not pressed: it keeps its function or is switched off")]
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
    #[error("{0} has no haptic feedback")]
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

/// Whether the sensitivity is one the mice offer.
fn is_valid_dpi(dpi: u16) -> bool {
    DPI_RANGE.contains(&dpi) && (dpi - DPI_RANGE.start()).is_multiple_of(DPI_STEP)
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
            pointer: PointerSettings { dpi: DEFAULT_DPI },
            wheel: WheelSettings {
                mode: WheelMode::Auto,
                auto_threshold: DEFAULT_WHEEL_THRESHOLD,
                natural: false,
                hi_res: true,
            },
            thumb_wheel: ThumbWheelSettings {
                speed: DEFAULT_THUMB_SPEED,
                invert: false,
            },
            haptics: HapticSettings {
                level: HapticLevel::Medium,
                events: BTreeMap::new(),
            },
            revision: 0,
        }
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
            let Some(known) = capabilities.control(*control) else {
                return Err(MouseConfigError::UnknownControl {
                    model: self.model,
                    control: *control,
                });
            };
            if known.locked {
                return Err(MouseConfigError::LockedControl(*control));
            }
            if known.kind == ControlKind::Rotation && *action != ButtonAction::None {
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
        if !is_valid_dpi(self.pointer.dpi) {
            return Err(MouseConfigError::InvalidDpi(self.pointer.dpi));
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
        match events.keys().find(|key| !is_event_key(key)) {
            Some(key) => Err(MouseConfigError::InvalidHapticEvent(key.clone())),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HapticEventSetting, HapticWaveform, MouseButton};

    const SERIAL: &str = "mx-test";
    const MASTER: MouseModel = MouseModel::MxMaster4;
    const ANYWHERE: MouseModel = MouseModel::MxAnywhere3S;

    fn config(model: MouseModel) -> MouseConfig {
        MouseConfig::default_for(SERIAL, model)
    }

    fn bound(model: MouseModel, control: MouseControlId, action: ButtonAction) -> MouseConfig {
        let mut config = config(model);
        config.bindings.insert(control, action);
        config
    }

    fn event(waveform: HapticWaveform) -> HapticEventSetting {
        HapticEventSetting {
            enabled: true,
            waveform,
        }
    }

    #[test]
    fn defaults_are_valid_and_open_the_menu_where_decided() {
        for model in [MASTER, ANYWHERE] {
            let config = config(model);
            assert_eq!(config.validate(), Ok(()), "{model}");
            assert_eq!(
                config.bindings,
                BTreeMap::from([(model.default_menu_control(), ButtonAction::RadialMenu)]),
                "{model}"
            );
        }
    }

    #[test]
    fn default_settings_are_inside_their_ranges() {
        assert!(is_valid_dpi(DEFAULT_DPI));
        assert!(WHEEL_THRESHOLD_RANGE.contains(&DEFAULT_WHEEL_THRESHOLD));
        assert!(THUMB_SPEED_RANGE.contains(&DEFAULT_THUMB_SPEED));
    }

    #[test]
    fn a_control_the_model_lacks_is_refused() {
        let config = bound(ANYWHERE, MouseControlId::Gesture, ButtonAction::None);
        assert_eq!(
            config.validate(),
            Err(MouseConfigError::UnknownControl {
                model: ANYWHERE,
                control: MouseControlId::Gesture
            })
        );
    }

    #[test]
    fn left_and_right_click_never_have_an_entry() {
        let actions = [
            ButtonAction::None,
            ButtonAction::RadialMenu,
            ButtonAction::Click {
                button: MouseButton::Right,
            },
        ];
        for control in [MouseControlId::Left, MouseControlId::Right] {
            for action in &actions {
                let config = bound(MASTER, control, action.clone());
                assert_eq!(
                    config.validate(),
                    Err(MouseConfigError::LockedControl(control)),
                    "{control} {action:?}"
                );
            }
        }
    }

    #[test]
    fn the_thumb_wheel_can_only_be_switched_off() {
        let thumb = MouseControlId::ThumbWheel;
        for action in [
            ButtonAction::Click {
                button: MouseButton::Middle,
            },
            ButtonAction::RadialMenu,
            ButtonAction::DoubleClick,
        ] {
            let config = bound(MASTER, thumb, action.clone());
            assert_eq!(
                config.validate(),
                Err(MouseConfigError::NotAButton(thumb)),
                "{action:?}"
            );
        }
        assert_eq!(bound(MASTER, thumb, ButtonAction::None).validate(), Ok(()));
    }

    #[test]
    fn the_radial_menu_has_one_opener() {
        let mut config = config(MASTER);
        config
            .bindings
            .insert(MouseControlId::Gesture, ButtonAction::RadialMenu);
        assert_eq!(config.validate(), Err(MouseConfigError::RadialMenuTwice));
        config.bindings.remove(&MouseControlId::HapticPanel);
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn module_actions_need_plain_identifiers() {
        let action = |module: &str, action: &str| ButtonAction::Module {
            module: module.into(),
            action: action.to_owned(),
        };
        let valid = bound(
            MASTER,
            MouseControlId::Gesture,
            action("pomodoro", "skip-period"),
        );
        assert_eq!(valid.validate(), Ok(()));
        let too_long = "a".repeat(crate::MAX_IDENTIFIER_LENGTH + 1);
        let cases = [
            ("po mo", "skip", "po mo"),
            ("", "skip", ""),
            ("pomodoro", "", ""),
            ("pomodoro", "a\nb", "a\nb"),
            ("pomodoro", "é", "é"),
            ("pomodoro", too_long.as_str(), too_long.as_str()),
        ];
        for (module, name, culprit) in cases {
            let config = bound(MASTER, MouseControlId::Gesture, action(module, name));
            assert_eq!(
                config.validate(),
                Err(MouseConfigError::InvalidIdentifier(culprit.to_owned())),
                "{module:?}/{name:?}"
            );
        }
    }

    #[test]
    fn shortcuts_are_checked_through_validate() {
        let shortcut = |keys: &str| ButtonAction::Shortcut {
            keys: keys.to_owned(),
        };
        assert_eq!(
            bound(MASTER, MouseControlId::Back, shortcut("Ctrl+Shift+M")).validate(),
            Ok(())
        );
        assert_eq!(
            bound(MASTER, MouseControlId::Back, shortcut("Ctrl+")).validate(),
            Err(MouseConfigError::InvalidShortcut("Ctrl+".to_owned()))
        );
    }

    #[test]
    fn locked_controls_are_reported_before_anything_else_about_them() {
        // A locked control is refused whatever the action (here also a second
        // radial opener and a bad shortcut): the lock comes first.
        let config = bound(
            MASTER,
            MouseControlId::Left,
            ButtonAction::Shortcut {
                keys: "+".to_owned(),
            },
        );
        assert_eq!(
            config.validate(),
            Err(MouseConfigError::LockedControl(MouseControlId::Left))
        );
    }

    #[test]
    fn the_sensitivity_is_a_multiple_of_the_step_from_the_lowest() {
        let cases = [
            (0, false),
            (100, false),
            (199, false),
            (200, true),
            (225, false),
            (250, true),
            (1650, true),
            (7950, true),
            (8000, true),
            (8001, false),
            (8049, false),
            (8050, false),
            (u16::MAX, false),
        ];
        for (dpi, valid) in cases {
            let mut config = config(MASTER);
            config.pointer.dpi = dpi;
            let expected = if valid {
                Ok(())
            } else {
                Err(MouseConfigError::InvalidDpi(dpi))
            };
            assert_eq!(config.validate(), expected, "{dpi} dpi");
        }
    }

    #[test]
    fn the_wheel_threshold_and_the_thumb_speed_stay_in_range() {
        for (threshold, valid) in [
            (0, false),
            (1, true),
            (50, true),
            (51, false),
            (u8::MAX, false),
        ] {
            let mut config = config(MASTER);
            config.wheel.auto_threshold = threshold;
            let expected = if valid {
                Ok(())
            } else {
                Err(MouseConfigError::InvalidWheelThreshold(threshold))
            };
            assert_eq!(config.validate(), expected, "threshold {threshold}");
        }
        for (speed, valid) in [
            (0, false),
            (1, true),
            (10, true),
            (11, false),
            (u8::MAX, false),
        ] {
            let mut config = config(MASTER);
            config.thumb_wheel.speed = speed;
            let expected = if valid {
                Ok(())
            } else {
                Err(MouseConfigError::InvalidThumbSpeed(speed))
            };
            assert_eq!(config.validate(), expected, "speed {speed}");
        }
    }

    #[test]
    fn settings_a_model_does_not_use_are_still_checked() {
        let mut config = config(ANYWHERE);
        config.thumb_wheel.speed = 99;
        assert_eq!(
            config.validate(),
            Err(MouseConfigError::InvalidThumbSpeed(99))
        );
    }

    #[test]
    fn haptic_events_are_keyed_module_dot_event() {
        let mut config = config(MASTER);
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
            assert_eq!(
                bad.validate(),
                Err(MouseConfigError::InvalidHapticEvent(key.to_owned())),
                "{key}"
            );
        }
    }

    #[test]
    fn haptic_events_need_a_haptic_mouse_and_stay_few() {
        let mut anywhere = config(ANYWHERE);
        anywhere
            .haptics
            .events
            .insert("teams.call".to_owned(), event(HapticWaveform::Ringing));
        assert_eq!(
            anywhere.validate(),
            Err(MouseConfigError::HapticsUnsupported(ANYWHERE))
        );
        let mut master = config(MASTER);
        for index in 0..MAX_HAPTIC_EVENTS {
            master
                .haptics
                .events
                .insert(format!("m.e{index}"), event(HapticWaveform::Wave));
        }
        assert_eq!(master.validate(), Ok(()), "exactly the limit");
        master
            .haptics
            .events
            .insert("m.one-more".to_owned(), event(HapticWaveform::Wave));
        assert_eq!(
            master.validate(),
            Err(MouseConfigError::TooManyHapticEvents {
                count: MAX_HAPTIC_EVENTS + 1
            })
        );
    }

    #[test]
    fn another_schema_version_is_refused_first() {
        let mut config = bound(MASTER, MouseControlId::Left, ButtonAction::None);
        config.schema_version = 2;
        assert_eq!(
            config.validate(),
            Err(MouseConfigError::UnsupportedSchemaVersion { found: 2 })
        );
    }

    #[test]
    fn a_configuration_survives_json_and_keeps_its_shape() {
        let mut config = bound(
            MASTER,
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
        assert_eq!(json["pointer"]["dpi"], DEFAULT_DPI);
        assert_eq!(json["wheel"]["autoThreshold"], DEFAULT_WHEEL_THRESHOLD);
        assert_eq!(
            json["haptics"]["events"]["teams.call"]["waveform"],
            "ringing"
        );
        let back: MouseConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back, config);
    }

    #[test]
    fn errors_name_things_for_people() {
        assert_eq!(
            MouseConfigError::InvalidDpi(225).to_string(),
            "sensitivity of 225 dpi is not a multiple of 50 in 200..=8000"
        );
        assert_eq!(
            MouseConfigError::LockedControl(MouseControlId::Left).to_string(),
            "the left click cannot be given another action"
        );
        assert_eq!(
            MouseConfigError::UnknownControl {
                model: ANYWHERE,
                control: MouseControlId::Gesture
            }
            .to_string(),
            "MX Anywhere 3S has no gesture button"
        );
        assert_eq!(
            MouseConfigError::HapticsUnsupported(ANYWHERE).to_string(),
            "MX Anywhere 3S has no haptic feedback"
        );
    }
}
