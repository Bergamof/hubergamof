//! Haptic waveforms, the vibration patterns a mouse can play.

use serde::{Deserialize, Serialize};

/// A predefined vibration pattern of a haptic mouse.
///
/// Modules pick the one that best suits what they signal; the user may change
/// it per event, and the manager plays it on the mouse. The names describe the
/// feel; the exact pattern is the device's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[non_exhaustive]
pub enum HapticWaveform {
    /// Crisp tick, a state that flips.
    SharpStateChange,
    /// Softer tick, a state that flips.
    DampStateChange,
    /// Crisp knock, hitting a limit.
    SharpCollision,
    /// Dull knock, hitting a limit.
    DampCollision,
    /// Light knock.
    SubtleCollision,
    /// Barely there knock.
    WhisperCollision,
    /// Cheerful pattern, good news.
    HappyAlert,
    /// Abrupt pattern, a problem.
    AngryAlert,
    /// Rising pattern, something finished.
    Completed,
    /// Square pulse.
    Square,
    /// Smooth swell.
    Wave,
    /// Burst, then fading pulses.
    Firework,
    /// Insistent pattern.
    Mad,
    /// Two quick knocks, like on a door.
    Knock,
    /// Bell-like repetition.
    Jingle,
    /// Phone-like repetition, an incoming call.
    Ringing,
}

impl HapticWaveform {
    /// Every waveform, in the order the interface lists them.
    pub const ALL: [Self; 16] = [
        Self::SharpStateChange,
        Self::DampStateChange,
        Self::SharpCollision,
        Self::DampCollision,
        Self::SubtleCollision,
        Self::WhisperCollision,
        Self::HappyAlert,
        Self::AngryAlert,
        Self::Completed,
        Self::Square,
        Self::Wave,
        Self::Firework,
        Self::Mad,
        Self::Knock,
        Self::Jingle,
        Self::Ringing,
    ];
}

/// An event of a module that a haptic mouse can signal: a step that ends, a
/// call that rings. The module declares it (see
/// [`ModuleFactory::haptic_events`](crate::ModuleFactory::haptic_events)) and
/// raises it with [`Command::haptic`](crate::Command::haptic); the user chooses,
/// per mouse, whether it is played and with which waveform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[non_exhaustive]
pub struct HapticEventDescriptor {
    /// Identifier, passed back in [`Command::Haptic`](crate::Command::Haptic):
    /// letters, digits, `-` and `_`, unique within the module.
    pub id: String,
    /// Name shown in the configuration of the haptics.
    pub name: String,
    /// When it is played.
    pub description: String,
    /// The waveform played until the user picks another.
    pub waveform: HapticWaveform,
}

impl HapticEventDescriptor {
    /// An event `id` named `name`, described by `description`, played as
    /// `waveform` by default.
    #[must_use]
    pub fn new(id: &str, name: &str, description: &str, waveform: HapticWaveform) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            waveform,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// One arm per variant, no catch-all: adding a variant stops the build
    /// here until it is also added to `ALL` and to the name table below.
    fn wire_name(waveform: HapticWaveform) -> &'static str {
        match waveform {
            HapticWaveform::SharpStateChange => "sharpStateChange",
            HapticWaveform::DampStateChange => "dampStateChange",
            HapticWaveform::SharpCollision => "sharpCollision",
            HapticWaveform::DampCollision => "dampCollision",
            HapticWaveform::SubtleCollision => "subtleCollision",
            HapticWaveform::WhisperCollision => "whisperCollision",
            HapticWaveform::HappyAlert => "happyAlert",
            HapticWaveform::AngryAlert => "angryAlert",
            HapticWaveform::Completed => "completed",
            HapticWaveform::Square => "square",
            HapticWaveform::Wave => "wave",
            HapticWaveform::Firework => "firework",
            HapticWaveform::Mad => "mad",
            HapticWaveform::Knock => "knock",
            HapticWaveform::Jingle => "jingle",
            HapticWaveform::Ringing => "ringing",
        }
    }

    #[test]
    fn an_event_is_built_from_its_texts_and_waveform() {
        let event = HapticEventDescriptor::new(
            "step-end",
            "Fin d'étape",
            "Quand elle finit",
            HapticWaveform::Completed,
        );
        assert_eq!(event.id, "step-end");
        assert_eq!(event.waveform, HapticWaveform::Completed);
    }

    #[test]
    fn all_lists_each_waveform_once() {
        let distinct: HashSet<_> = HapticWaveform::ALL.into_iter().collect();
        assert_eq!(distinct.len(), 16);
        assert_eq!(HapticWaveform::ALL.len(), 16);
    }

    #[test]
    fn every_waveform_travels_under_its_name_and_comes_back() {
        for waveform in HapticWaveform::ALL {
            let json = serde_json::to_string(&waveform).unwrap();
            assert_eq!(json, format!("\"{}\"", wire_name(waveform)));
            assert_eq!(
                serde_json::from_str::<HapticWaveform>(&json).unwrap(),
                waveform
            );
        }
    }

    #[test]
    fn unknown_names_and_other_cases_are_refused() {
        for name in [
            "\"fire\"",
            "\"Knock\"",
            "\"sharp_state_change\"",
            "\"\"",
            "3",
        ] {
            assert!(
                serde_json::from_str::<HapticWaveform>(name).is_err(),
                "{name}"
            );
        }
    }
}
