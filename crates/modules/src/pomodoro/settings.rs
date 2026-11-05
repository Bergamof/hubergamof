//! Settings of a pomodoro instance and their catalog fields.

use std::ops::RangeInclusive;
use std::time::Duration;

use hubergamof_module_api::{SettingField, SettingKind};
use serde::Deserialize;

const WORK_MINUTES_KEY: &str = "workMinutes";
const SHORT_BREAK_MINUTES_KEY: &str = "shortBreakMinutes";
const LONG_BREAK_MINUTES_KEY: &str = "longBreakMinutes";
const LONG_BREAK_EVERY_KEY: &str = "longBreakEvery";
const AUTO_START_KEY: &str = "autoStart";

/// Allowed length of a work session, in minutes.
pub const WORK_MINUTES: RangeInclusive<u32> = 1..=120;
/// Allowed length of a short break, in minutes.
pub const SHORT_BREAK_MINUTES: RangeInclusive<u32> = 1..=60;
/// Allowed length of a long break, in minutes.
pub const LONG_BREAK_MINUTES: RangeInclusive<u32> = 1..=120;
/// Allowed number of pomodoros between two long breaks.
pub const LONG_BREAK_EVERY: RangeInclusive<u32> = 1..=12;

const MINUTE_UNIT: &str = "min";

/// Settings of a pomodoro instance, as stored in the configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PomodoroSettings {
    /// Length of a work session, in minutes.
    pub work_minutes: u32,
    /// Length of a short break, in minutes.
    pub short_break_minutes: u32,
    /// Length of a long break, in minutes.
    pub long_break_minutes: u32,
    /// A long break follows every `long_break_every`-th pomodoro.
    pub long_break_every: u32,
    /// Starts the next step as soon as one ends, instead of waiting for a
    /// press.
    pub auto_start: bool,
}

impl Default for PomodoroSettings {
    fn default() -> Self {
        Self {
            work_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            long_break_every: 4,
            auto_start: false,
        }
    }
}

impl PomodoroSettings {
    /// Lengths of the steps.
    #[must_use]
    pub fn durations(&self) -> Durations {
        Durations {
            work: minutes(self.work_minutes),
            short_break: minutes(self.short_break_minutes),
            long_break: minutes(self.long_break_minutes),
            long_break_every: self.long_break_every.max(1),
        }
    }
}

/// Lengths of the steps of a pomodoro cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Durations {
    /// Work session.
    pub work: Duration,
    /// Short break.
    pub short_break: Duration,
    /// Long break.
    pub long_break: Duration,
    /// A long break follows every `long_break_every`-th pomodoro (at least 1).
    pub long_break_every: u32,
}

impl Default for Durations {
    fn default() -> Self {
        PomodoroSettings::default().durations()
    }
}

fn minutes(count: u32) -> Duration {
    Duration::from_mins(u64::from(count))
}

/// Catalog fields of the settings, in display order.
#[must_use]
pub fn fields() -> Vec<SettingField> {
    let defaults = PomodoroSettings::default();
    vec![
        minutes_field(
            WORK_MINUTES_KEY,
            "Durée du travail",
            defaults.work_minutes,
            WORK_MINUTES,
        ),
        minutes_field(
            SHORT_BREAK_MINUTES_KEY,
            "Durée de la pause courte",
            defaults.short_break_minutes,
            SHORT_BREAK_MINUTES,
        ),
        minutes_field(
            LONG_BREAK_MINUTES_KEY,
            "Durée de la pause longue",
            defaults.long_break_minutes,
            LONG_BREAK_MINUTES,
        ),
        SettingField {
            key: LONG_BREAK_EVERY_KEY.into(),
            label: "Pause longue tous les".into(),
            help: Some("Nombre de pomodoros avant une pause longue.".into()),
            kind: integer(defaults.long_break_every, &LONG_BREAK_EVERY, None),
        },
        SettingField {
            key: AUTO_START_KEY.into(),
            label: "Enchaîner automatiquement".into(),
            help: Some(
                "Lance l'étape suivante dès la fin de la précédente, sans attendre d'appui.".into(),
            ),
            kind: SettingKind::Boolean {
                default: defaults.auto_start,
            },
        },
    ]
}

fn minutes_field(key: &str, label: &str, default: u32, range: RangeInclusive<u32>) -> SettingField {
    SettingField {
        key: key.into(),
        label: label.into(),
        help: None,
        kind: integer(default, &range, Some(MINUTE_UNIT)),
    }
}

fn integer(default: u32, range: &RangeInclusive<u32>, unit: Option<&str>) -> SettingKind {
    SettingKind::Integer {
        default: default.into(),
        min: Some((*range.start()).into()),
        max: Some((*range.end()).into()),
        unit: unit.map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use hubergamof_module_api::ModuleError;
    use serde_json::json;

    use super::*;

    #[test]
    fn missing_keys_take_their_defaults() {
        let settings = PomodoroSettings::parse(&json!({ "workMinutes": 50 })).unwrap();
        assert_eq!(
            settings,
            PomodoroSettings {
                work_minutes: 50,
                ..PomodoroSettings::default()
            }
        );
        assert_eq!(settings.durations().work, Duration::from_mins(50));
    }

    #[test]
    fn out_of_bounds_values_are_rejected() {
        for settings in [
            json!({ "workMinutes": 0 }),
            json!({ "shortBreakMinutes": 61 }),
            json!({ "longBreakMinutes": 121 }),
            json!({ "longBreakEvery": 0 }),
            json!({ "autoStart": "yes" }),
            json!({ "workMinutes": -5 }),
        ] {
            assert!(
                matches!(
                    PomodoroSettings::parse(&settings),
                    Err(ModuleError::InvalidSettings(_))
                ),
                "{settings} accepted"
            );
        }
    }

    #[test]
    fn fields_declare_the_parsed_defaults_and_bounds() {
        let fields = fields();
        let keys: Vec<_> = fields.iter().map(|field| field.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                WORK_MINUTES_KEY,
                SHORT_BREAK_MINUTES_KEY,
                LONG_BREAK_MINUTES_KEY,
                LONG_BREAK_EVERY_KEY,
                AUTO_START_KEY
            ]
        );
        assert_eq!(
            fields[0].kind,
            SettingKind::Integer {
                default: 25,
                min: Some(1),
                max: Some(120),
                unit: Some("min".into()),
            }
        );
    }
}
