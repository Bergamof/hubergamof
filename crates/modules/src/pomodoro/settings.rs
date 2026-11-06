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
const NOTIFICATION_ENABLED_KEY: &str = "notificationEnabled";
const NOTIFICATION_SECONDS_KEY: &str = "notificationSeconds";

/// Allowed length of a work session, in minutes.
pub const WORK_MINUTES: RangeInclusive<u32> = 1..=120;
/// Allowed length of a short break, in minutes.
pub const SHORT_BREAK_MINUTES: RangeInclusive<u32> = 1..=60;
/// Allowed length of a long break, in minutes.
pub const LONG_BREAK_MINUTES: RangeInclusive<u32> = 1..=120;
/// Allowed number of pomodoros between two long breaks.
pub const LONG_BREAK_EVERY: RangeInclusive<u32> = 1..=12;
/// Allowed time a step-end notification stays on the strip, in seconds;
/// 0 keeps it until it is dismissed.
pub const NOTIFICATION_SECONDS: RangeInclusive<u32> = 0..=3600;

const MINUTE_UNIT: &str = "min";
const SECOND_UNIT: &str = "s";

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
    /// Shows a notification on the touch strip when a step ends.
    pub notification_enabled: bool,
    /// Seconds the notification stays on the strip; 0 keeps it until it is
    /// dismissed.
    pub notification_seconds: u32,
}

impl Default for PomodoroSettings {
    fn default() -> Self {
        Self {
            work_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            long_break_every: 4,
            auto_start: false,
            notification_enabled: true,
            notification_seconds: 10,
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

/// How the end of a step is announced on the touch strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StripNotice {
    /// Whether it is announced at all.
    pub enabled: bool,
    /// How long it stays, `None` until dismissed.
    pub timeout: Option<Duration>,
}

impl PomodoroSettings {
    /// How the end of a step is announced on the touch strip.
    #[must_use]
    pub fn strip_notice(&self) -> StripNotice {
        StripNotice {
            enabled: self.notification_enabled,
            timeout: (self.notification_seconds > 0)
                .then(|| Duration::from_secs(u64::from(self.notification_seconds))),
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
        SettingField {
            key: NOTIFICATION_ENABLED_KEY.into(),
            label: "Notification sur la bande tactile".into(),
            help: Some("Affiche un message sur la bande tactile à la fin de chaque étape.".into()),
            kind: SettingKind::Boolean {
                default: defaults.notification_enabled,
            },
        },
        SettingField {
            key: NOTIFICATION_SECONDS_KEY.into(),
            label: "Durée de la notification".into(),
            help: Some(
                "0 : la notification reste affichée jusqu'à ce qu'on la ferme (toucher la bande, \
                 ou appuyer sur la molette de navigation)."
                    .into(),
            ),
            kind: integer(
                defaults.notification_seconds,
                &NOTIFICATION_SECONDS,
                Some(SECOND_UNIT),
            ),
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
    fn the_notice_follows_the_settings_and_zero_means_permanent() {
        let notice = |settings| PomodoroSettings::parse(&settings).unwrap().strip_notice();
        assert_eq!(
            notice(json!({})),
            StripNotice {
                enabled: true,
                timeout: Some(Duration::from_secs(10))
            }
        );
        assert_eq!(
            notice(json!({ "notificationSeconds": 0 })),
            StripNotice {
                enabled: true,
                timeout: None
            }
        );
        assert!(!notice(json!({ "notificationEnabled": false })).enabled);
    }

    #[test]
    fn out_of_bounds_values_are_rejected() {
        for settings in [
            json!({ "workMinutes": 0 }),
            json!({ "shortBreakMinutes": 61 }),
            json!({ "longBreakMinutes": 121 }),
            json!({ "longBreakEvery": 0 }),
            json!({ "autoStart": "yes" }),
            json!({ "notificationSeconds": 3601 }),
            json!({ "notificationSeconds": -1 }),
            json!({ "notificationEnabled": 1 }),
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
                AUTO_START_KEY,
                NOTIFICATION_ENABLED_KEY,
                NOTIFICATION_SECONDS_KEY
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
