//! Reminders of the meetings the extension read from the calendar: a
//! notification on the strip a few minutes before one starts.
//!
//! Nothing is read here on a schedule: the meetings are those of the last
//! report, which the extension only makes when the user opens the calendar.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hubergamof_module_api::{EventSender, SettingField, SettingKind};
use serde::Deserialize;
use tokio::sync::watch;

use super::bridge::TeamsState;
use super::wire::Meeting;

/// Settings key of the lead time.
const REMINDER_MINUTES_KEY: &str = "reminderMinutes";

/// Allowed lead time, in minutes; 0 turns the reminders off.
pub const REMINDER_MINUTES: std::ops::RangeInclusive<u32> = 0..=60;

/// Default lead time, in minutes.
const DEFAULT_REMINDER_MINUTES: u32 = 5;

/// How often the clock is looked at: the time spent suspended must not delay
/// a reminder, which a single long sleep would.
const CHECK_EVERY: Duration = Duration::from_secs(10);

const SECONDS_PER_MINUTE: i64 = 60;

/// Settings of a Teams instance, as stored in the configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TeamsSettings {
    /// Minutes before a meeting that it is announced; 0 for never.
    pub reminder_minutes: u32,
}

impl Default for TeamsSettings {
    fn default() -> Self {
        Self {
            reminder_minutes: DEFAULT_REMINDER_MINUTES,
        }
    }
}

/// Catalog fields of the settings.
#[must_use]
pub fn fields() -> Vec<SettingField> {
    vec![SettingField {
        key: REMINDER_MINUTES_KEY.into(),
        label: "Rappel des réunions".into(),
        help: Some(
            "Prévient sur la bande tactile avant le début d'une réunion du calendrier. Le \
             calendrier n'est lu que lorsque vous l'ouvrez dans Teams. 0 : pas de rappel."
                .into(),
        ),
        kind: SettingKind::Integer {
            default: DEFAULT_REMINDER_MINUTES.into(),
            min: Some((*REMINDER_MINUTES.start()).into()),
            max: Some((*REMINDER_MINUTES.end()).into()),
            unit: Some("min".into()),
        },
    }]
}

/// The time of day, in seconds since the Unix epoch, read from a clock that
/// tests can replace.
#[derive(Clone)]
pub struct WallClock(Arc<dyn Fn() -> i64 + Send + Sync>);

impl WallClock {
    /// The system clock.
    #[must_use]
    pub fn system() -> Self {
        Self::from_fn(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| {
                    i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
                })
        })
    }

    /// A clock reading `now`.
    #[must_use]
    pub fn from_fn(now: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        Self(Arc::new(now))
    }

    fn now(&self) -> i64 {
        (self.0)()
    }
}

impl fmt::Debug for WallClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WallClock")
    }
}

/// A meeting about to start, announced to the instance.
pub(super) struct MeetingSoon {
    /// What the notification says.
    pub message: String,
}

/// Which meeting was announced: a rescheduled one is announced again.
type MeetingKey = (i64, String);

/// Messages for the meetings that start within `lead_minutes` of `now` and
/// were not announced yet, which are then remembered in `announced`.
fn due_reminders(
    meetings: &[Meeting],
    now: i64,
    lead_minutes: u32,
    announced: &mut HashSet<MeetingKey>,
) -> Vec<String> {
    // Only what is still announced is remembered: the set stays as small as
    // the calendar.
    announced.retain(|(starts_at, subject)| {
        meetings
            .iter()
            .any(|meeting| meeting.starts_at == *starts_at && meeting.subject == *subject)
    });
    if lead_minutes == 0 {
        return Vec::new();
    }
    let lead = i64::from(lead_minutes) * SECONDS_PER_MINUTE;
    meetings
        .iter()
        .filter(|meeting| meeting.starts_at > now && meeting.starts_at - now <= lead)
        .filter(|meeting| announced.insert((meeting.starts_at, meeting.subject.clone())))
        .map(|meeting| {
            let minutes = (meeting.starts_at - now + SECONDS_PER_MINUTE - 1) / SECONDS_PER_MINUTE;
            format!("Réunion dans {minutes} min : {}", meeting.subject)
        })
        .collect()
}

/// Announces the meetings of `state` to the instance, `lead` minutes ahead,
/// until the instance is gone.
pub(super) async fn run(
    mut state: watch::Receiver<TeamsState>,
    mut lead: watch::Receiver<u32>,
    clock: WallClock,
    events: EventSender,
) {
    let mut announced = HashSet::new();
    let mut tick = tokio::time::interval(CHECK_EVERY);
    loop {
        tokio::select! {
            _ = tick.tick() => {}
            changed = state.changed() => if changed.is_err() { return },
            changed = lead.changed() => if changed.is_err() { return },
        }
        let lead_minutes = *lead.borrow_and_update();
        let reminders = due_reminders(
            &state.borrow_and_update().meetings,
            clock.now(),
            lead_minutes,
            &mut announced,
        );
        for message in reminders {
            events.send(MeetingSoon { message });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting(subject: &str, starts_at: i64) -> Meeting {
        Meeting {
            subject: subject.into(),
            starts_at,
            ends_at: starts_at + 1800,
        }
    }

    #[test]
    fn a_meeting_is_announced_once_within_the_lead_time() {
        let meetings = [meeting("Point", 1000)];
        let mut announced = HashSet::new();
        assert_eq!(due_reminders(&meetings, 600, 5, &mut announced).len(), 0);
        assert_eq!(
            due_reminders(&meetings, 700, 5, &mut announced),
            ["Réunion dans 5 min : Point"]
        );
        assert_eq!(due_reminders(&meetings, 710, 5, &mut announced).len(), 0);
    }

    #[test]
    fn the_minutes_left_are_rounded_up() {
        let mut announced = HashSet::new();
        let messages = due_reminders(&[meeting("Point", 1000)], 941, 5, &mut announced);
        assert_eq!(messages, ["Réunion dans 1 min : Point"]);
    }

    #[test]
    fn a_meeting_already_started_or_a_zero_lead_announces_nothing() {
        let meetings = [meeting("Point", 1000)];
        let mut announced = HashSet::new();
        assert_eq!(due_reminders(&meetings, 1000, 5, &mut announced).len(), 0);
        assert_eq!(due_reminders(&meetings, 900, 0, &mut announced).len(), 0);
    }

    #[test]
    fn a_moved_meeting_is_announced_again_and_the_memory_follows_the_calendar() {
        let mut announced = HashSet::new();
        due_reminders(&[meeting("Point", 1000)], 800, 5, &mut announced);
        assert_eq!(
            due_reminders(&[meeting("Point", 1200)], 1000, 5, &mut announced).len(),
            1
        );
        due_reminders(&[], 1000, 5, &mut announced);
        assert!(announced.is_empty());
    }

    #[test]
    fn several_meetings_are_announced_together() {
        let meetings = [meeting("A", 900), meeting("B", 950)];
        let mut announced = HashSet::new();
        assert_eq!(due_reminders(&meetings, 800, 5, &mut announced).len(), 2);
    }
}
