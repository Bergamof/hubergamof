//! Transient outcome marks ([`Feedback`]) shown over keys and in the reminders of the dials.

use std::collections::BTreeMap;
use std::time::Duration;

use hubergamof_module_api::Feedback;
use tokio::time::Instant;

use super::frame::KeyTarget;

/// How long a feedback mark stays over its surface.
pub(crate) const FEEDBACK_DURATION: Duration = Duration::from_secs(1);

/// Surface a feedback mark is drawn over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Surface {
    /// The key showing this target: the mark follows the item if the page
    /// scrolls and disappears with it when another page is shown.
    Key(KeyTarget),
    /// The reminder of this dial, under the touch strip.
    Dial(u8),
}

#[derive(Debug, Clone, Copy)]
struct Mark {
    feedback: Feedback,
    until: Instant,
}

/// Marks currently shown, each until its deadline.
#[derive(Debug, Default)]
pub(crate) struct Marks {
    keys: BTreeMap<KeyTarget, Mark>,
    dials: BTreeMap<u8, Mark>,
}

impl Marks {
    /// Shows `feedback` over `surface` for [`FEEDBACK_DURATION`] from `now`,
    /// replacing a mark already there.
    pub(crate) fn show(&mut self, surface: Surface, feedback: Feedback, now: Instant) {
        let mark = Mark {
            feedback,
            until: now + FEEDBACK_DURATION,
        };
        match surface {
            Surface::Key(target) => self.keys.insert(target, mark),
            Surface::Dial(dial) => self.dials.insert(dial, mark),
        };
    }

    /// Mark over the key showing `target`.
    pub(crate) fn key(&self, target: &KeyTarget) -> Option<Feedback> {
        self.keys.get(target).map(|mark| mark.feedback)
    }

    /// Mark over the reminder of `dial`.
    pub(crate) fn dial(&self, dial: u8) -> Option<Feedback> {
        self.dials.get(&dial).map(|mark| mark.feedback)
    }

    /// Earliest deadline, `None` without marks.
    pub(crate) fn next_expiry(&self) -> Option<Instant> {
        self.keys
            .values()
            .chain(self.dials.values())
            .map(|mark| mark.until)
            .min()
    }

    /// Removes the marks whose deadline passed at `now`.
    pub(crate) fn expire(&mut self, now: Instant) {
        self.keys.retain(|_, mark| mark.until > now);
        self.dials.retain(|_, mark| mark.until > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_last_their_duration_and_are_replaced() {
        let start = Instant::now();
        let target = KeyTarget::Home("audio".into());
        let mut marks = Marks::default();
        assert_eq!(marks.next_expiry(), None);

        marks.show(Surface::Key(target.clone()), Feedback::Ok, start);
        marks.show(
            Surface::Dial(2),
            Feedback::Alert,
            start + Duration::from_millis(300),
        );
        assert_eq!(marks.key(&target), Some(Feedback::Ok));
        assert_eq!(marks.dial(2), Some(Feedback::Alert));
        assert_eq!(marks.dial(1), None);
        assert_eq!(marks.next_expiry(), Some(start + FEEDBACK_DURATION));

        marks.show(Surface::Key(target.clone()), Feedback::Alert, start);
        assert_eq!(marks.key(&target), Some(Feedback::Alert));

        marks.expire(start + FEEDBACK_DURATION);
        assert_eq!(marks.key(&target), None);
        assert_eq!(marks.dial(2), Some(Feedback::Alert));
        marks.expire(start + Duration::from_secs(2));
        assert_eq!(marks.next_expiry(), None);
    }
}
