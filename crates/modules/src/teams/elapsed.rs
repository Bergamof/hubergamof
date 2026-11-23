//! The duration of a call, as the Hang up key shows it.

use std::time::Duration;

use hubergamof_module_api::EventSender;
use tokio::sync::watch;

use super::bridge::TeamsState;

/// How often the duration is read to see whether its text changed: well
/// under a second, so that a new second shows without delay, while an event
/// is only sent when the text does change.
const CHECK_EVERY: Duration = Duration::from_millis(250);

const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 3600;

/// The duration of the call in `state` as text: `None` outside a call.
pub(super) fn shown(state: &TeamsState) -> Option<String> {
    state
        .call
        .elapsed
        .map(|clock| format_duration(clock.seconds()))
}

/// `m:ss` below an hour, `h:mm:ss` from then on, as Teams writes it.
fn format_duration(total_seconds: u64) -> String {
    let hours = total_seconds / SECONDS_PER_HOUR;
    let minutes = total_seconds / SECONDS_PER_MINUTE % SECONDS_PER_MINUTE;
    let seconds = total_seconds % SECONDS_PER_MINUTE;
    if hours == 0 {
        format!("{minutes}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    }
}

/// The duration shown changed: the instance draws its keys again.
pub(super) struct ElapsedChanged;

/// Tells the instance each time the text of the duration of the call changes,
/// until the instance is gone.
pub(super) async fn run(state: watch::Receiver<TeamsState>, events: EventSender) {
    let mut last = None;
    let mut tick = tokio::time::interval(CHECK_EVERY);
    loop {
        tick.tick().await;
        let text = shown(&state.borrow());
        if text == last {
            continue;
        }
        last = text;
        // A refusal is a full queue as much as a dead instance: the next
        // second tries again, and the tasks of a dead instance are stopped
        // with it.
        events.send(ElapsedChanged);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_call_shows_minutes_and_seconds() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(59), "0:59");
        assert_eq!(format_duration(61), "1:01");
        assert_eq!(format_duration(3599), "59:59");
    }

    #[test]
    fn a_long_call_shows_hours() {
        assert_eq!(format_duration(3600), "1:00:00");
        assert_eq!(format_duration(3600 + 5 * 60 + 7), "1:05:07");
        assert_eq!(format_duration(26 * 3600), "26:00:00");
    }
}
