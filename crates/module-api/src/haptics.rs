//! Haptic waveforms, the vibration patterns a mouse can play.

use serde::{Deserialize, Serialize};

/// A predefined vibration pattern of a haptic mouse.
///
/// Modules pick the one that best suits what they signal; the user may change
/// it per event, and the manager plays it on the mouse (the exact feel is the
/// device's). Names follow the patterns Logitech documents for its haptic
/// mice.
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn all_lists_each_waveform_once() {
        let distinct: HashSet<_> = HapticWaveform::ALL.into_iter().collect();
        assert_eq!(distinct.len(), HapticWaveform::ALL.len());
    }

    #[test]
    fn waveforms_travel_as_camel_case_names() {
        let json = serde_json::to_string(&HapticWaveform::SharpStateChange).unwrap();
        assert_eq!(json, "\"sharpStateChange\"");
        let back: HapticWaveform = serde_json::from_str("\"whisperCollision\"").unwrap();
        assert_eq!(back, HapticWaveform::WhisperCollision);
    }
}
