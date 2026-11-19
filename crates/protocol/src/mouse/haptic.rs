//! Haptic feedback settings of a mouse.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{HapticWaveform, is_identifier};

/// Most haptic events of a configuration (every module offers a handful).
pub const MAX_HAPTIC_EVENTS: usize = 256;

/// Overall strength of the haptic feedback. [`HapticLevel::Off`] silences
/// everything on that mouse, whatever else (the radial menu's own switch, the
/// events) says.
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

/// `module.event`, both parts plain identifiers.
pub(super) fn is_event_key(key: &str) -> bool {
    key.split_once('.')
        .is_some_and(|(module, event)| is_identifier(module) && is_identifier(event))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_keys_are_module_dot_event() {
        for key in ["teams.new-message", "pomodoro.work_end", "a.b"] {
            assert!(is_event_key(key), "{key}");
        }
        for key in [
            "", "teams", "teams.", ".event", "te ams.x", "a.b.c", "a..b", "é.x",
        ] {
            assert!(!is_event_key(key), "{key:?}");
        }
    }

    #[test]
    fn levels_travel_as_camel_case() {
        assert_eq!(
            serde_json::to_string(&HapticLevel::Subtle).unwrap(),
            "\"subtle\""
        );
        assert!(serde_json::from_str::<HapticLevel>("\"loud\"").is_err());
    }
}
