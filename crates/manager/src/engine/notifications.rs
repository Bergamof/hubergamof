//! Notifications of a device: those not read yet, and the one shown over the
//! touch strip.
//!
//! A notification sent by a module is unread until the user touches the
//! strip while it is shown. It is shown once, in order of arrival, until its
//! timeout elapses, the strip is touched or the navigation dial is pressed;
//! only touching reads it. The notifications module can show an unread one
//! again, with no timeout.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use hubergamof_module_api::{
    ImageData, ModuleInstanceId, Notification, NotificationId, UnreadNotification,
    UnreadNotifications,
};
use tokio::time::Instant;

/// Most unread notifications kept; past it, the oldest is forgotten.
pub(crate) const MAX_UNREAD: usize = 50;

/// Period of the animation (time bar, scrolling text) of a shown
/// notification: about 15 images per second, what the device link carries
/// comfortably.
pub(crate) const ANIMATION_STEP: Duration = Duration::from_millis(66);

/// A notification waiting its turn.
struct Pending {
    id: NotificationId,
    timeout: Option<Duration>,
}

/// The notification on the strip.
pub(crate) struct Shown {
    pub(crate) notification: UnreadNotification,
    pub(crate) timeout: Option<Duration>,
    started: Instant,
    /// Whether the message is too long to stand still, known once measured.
    scrolls: Option<bool>,
}

impl Shown {
    /// Whether what is drawn changes with time: a time bar, or a scrolling
    /// message (assumed until measured).
    fn animated(&self) -> bool {
        self.timeout.is_some() || self.scrolls != Some(false)
    }

    /// Time since it appeared, in whole animation steps; zero when nothing
    /// moves, so that an unchanging notification is drawn once.
    pub(crate) fn elapsed(&self, now: Instant) -> Duration {
        if !self.animated() {
            return Duration::ZERO;
        }
        let steps =
            now.saturating_duration_since(self.started).as_millis() / ANIMATION_STEP.as_millis();
        ANIMATION_STEP.saturating_mul(u32::try_from(steps).unwrap_or(u32::MAX))
    }

    /// Whether the message was measured.
    pub(crate) fn is_measured(&self) -> bool {
        self.scrolls.is_some()
    }
}

/// Unread and shown notifications of one device.
pub(crate) struct NotificationCenter {
    /// Whether the device has a strip to show them on.
    on_strip: bool,
    unread: Arc<Mutex<Vec<UnreadNotification>>>,
    queue: VecDeque<Pending>,
    shown: Option<Shown>,
    next_id: u64,
    /// Instant of the last animation step.
    last_step: Instant,
}

impl NotificationCenter {
    /// A center showing notifications on the strip if the device has one;
    /// otherwise they only count as unread.
    pub(crate) fn new(on_strip: bool) -> Self {
        Self {
            on_strip,
            unread: Arc::default(),
            queue: VecDeque::new(),
            shown: None,
            next_id: 0,
            last_step: Instant::now(),
        }
    }

    /// Read access for the modules.
    pub(crate) fn handle(&self) -> UnreadNotifications {
        let unread = Arc::clone(&self.unread);
        UnreadNotifications::new(move || lock(&unread).clone())
    }

    /// The notification on the strip.
    pub(crate) fn shown(&self) -> Option<&Shown> {
        self.shown.as_ref()
    }

    /// Records the measure of the shown message.
    pub(crate) fn set_scrolls(&mut self, scrolls: bool) {
        if let Some(shown) = &mut self.shown {
            shown.scrolls = Some(scrolls);
        }
    }

    /// Adds a notification sent by `source`; it is shown right away if the
    /// strip is free.
    pub(crate) fn notify(
        &mut self,
        source: ModuleInstanceId,
        icon: Option<ImageData>,
        notification: &Notification,
        now: Instant,
    ) {
        let id = NotificationId(self.next_id);
        self.next_id += 1;
        let overflow = {
            let mut unread = lock(&self.unread);
            unread.push(UnreadNotification::new(id, source, icon, notification));
            (unread.len() > MAX_UNREAD).then(|| unread.remove(0).id)
        };
        if let Some(forgotten) = overflow {
            self.forget(forgotten);
        }
        if self.on_strip {
            self.queue.push_back(Pending {
                id,
                timeout: notification.timeout,
            });
            self.promote(now);
        }
    }

    /// Shows unread notification `id` again, with no timeout, in place of
    /// the one shown; ignored when it was read since.
    pub(crate) fn show_again(&mut self, id: NotificationId, now: Instant) {
        if !self.on_strip {
            return;
        }
        self.queue.retain(|pending| pending.id != id);
        self.show(id, None, now);
    }

    /// Removes the shown notification, which stays unread; the next one, if
    /// any, appears.
    pub(crate) fn dismiss(&mut self, now: Instant) {
        self.shown = None;
        self.promote(now);
    }

    /// Reads the shown notification: it leaves the unread ones.
    pub(crate) fn read(&mut self, now: Instant) {
        if let Some(shown) = self.shown.take() {
            lock(&self.unread).retain(|unread| unread.id != shown.notification.id);
        }
        self.promote(now);
    }

    /// Advances the animation and removes the shown notification when its
    /// timeout elapsed.
    pub(crate) fn step(&mut self, now: Instant) {
        self.last_step = now;
        let expired = self.shown.as_ref().is_some_and(|shown| {
            shown
                .timeout
                .is_some_and(|timeout| now.saturating_duration_since(shown.started) >= timeout)
        });
        if expired {
            self.dismiss(now);
        }
    }

    /// When the next animation step is due, `None` while nothing moves.
    pub(crate) fn next_step(&self) -> Option<Instant> {
        self.shown
            .as_ref()
            .filter(|shown| shown.animated())
            .map(|_| self.last_step + ANIMATION_STEP)
    }

    fn promote(&mut self, now: Instant) {
        while self.shown.is_none() {
            let Some(pending) = self.queue.pop_front() else {
                return;
            };
            self.show(pending.id, pending.timeout, now);
        }
    }

    fn show(&mut self, id: NotificationId, timeout: Option<Duration>, now: Instant) {
        let notification = lock(&self.unread)
            .iter()
            .find(|unread| unread.id == id)
            .cloned();
        self.shown = notification.map(|notification| Shown {
            notification,
            timeout,
            started: now,
            scrolls: None,
        });
        self.last_step = now;
    }

    fn forget(&mut self, id: NotificationId) {
        self.queue.retain(|pending| pending.id != id);
        if self
            .shown
            .as_ref()
            .is_some_and(|shown| shown.notification.id == id)
        {
            self.shown = None;
        }
    }
}

fn lock(unread: &Mutex<Vec<UnreadNotification>>) -> MutexGuard<'_, Vec<UnreadNotification>> {
    // The list is never left half-updated: a poisoned lock is still valid.
    unread.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use hubergamof_module_api::Rgb;

    use super::*;

    fn message(text: &str) -> Notification {
        Notification::new(text, Rgb(1, 2, 3))
    }

    fn timed(text: &str, seconds: u64) -> Notification {
        message(text).with_timeout(Duration::from_secs(seconds))
    }

    fn center() -> NotificationCenter {
        NotificationCenter::new(true)
    }

    fn send(center: &mut NotificationCenter, notification: &Notification, now: Instant) {
        center.notify("p".into(), None, notification, now);
    }

    fn shown_text(center: &NotificationCenter) -> Option<&str> {
        center
            .shown()
            .map(|shown| shown.notification.message.as_str())
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_notification_is_shown_and_the_others_wait() {
        let mut center = center();
        let now = Instant::now();
        send(&mut center, &message("un"), now);
        send(&mut center, &message("deux"), now);
        assert_eq!(shown_text(&center), Some("un"));
        assert_eq!(center.handle().count(), 2);
        center.dismiss(now);
        assert_eq!(shown_text(&center), Some("deux"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_timeout_removes_the_notification_but_keeps_it_unread() {
        let mut center = center();
        let start = Instant::now();
        send(&mut center, &timed("un", 3), start);
        center.step(start + Duration::from_secs(2));
        assert_eq!(shown_text(&center), Some("un"));
        center.step(start + Duration::from_secs(3));
        assert_eq!(shown_text(&center), None);
        assert_eq!(center.handle().count(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn without_a_timeout_the_notification_stays() {
        let mut center = center();
        let start = Instant::now();
        send(&mut center, &message("un"), start);
        center.step(start + Duration::from_secs(3600));
        assert_eq!(shown_text(&center), Some("un"));
    }

    #[tokio::test(start_paused = true)]
    async fn reading_removes_it_from_the_unread_ones() {
        let mut center = center();
        let now = Instant::now();
        send(&mut center, &timed("un", 3), now);
        send(&mut center, &message("deux"), now);
        center.read(now);
        assert_eq!(center.handle().count(), 1);
        assert_eq!(shown_text(&center), Some("deux"));
    }

    #[tokio::test(start_paused = true)]
    async fn dismissing_keeps_it_unread() {
        let mut center = center();
        let now = Instant::now();
        send(&mut center, &message("un"), now);
        center.dismiss(now);
        assert_eq!(shown_text(&center), None);
        assert_eq!(center.handle().list()[0].message, "un");
    }

    #[tokio::test(start_paused = true)]
    async fn an_unread_notification_is_shown_again_without_timeout() {
        let mut center = center();
        let start = Instant::now();
        send(&mut center, &timed("un", 3), start);
        let id = center.handle().list()[0].id;
        center.step(start + Duration::from_secs(3));
        center.show_again(id, start + Duration::from_secs(10));
        center.step(start + Duration::from_secs(600));
        assert_eq!(shown_text(&center), Some("un"));
        assert_eq!(center.shown().unwrap().timeout, None);
        center.read(start + Duration::from_secs(601));
        assert_eq!(center.handle().count(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_read_notification_cannot_be_shown_again() {
        let mut center = center();
        let now = Instant::now();
        send(&mut center, &message("un"), now);
        let id = center.handle().list()[0].id;
        center.read(now);
        center.show_again(id, now);
        assert_eq!(shown_text(&center), None);
    }

    #[tokio::test(start_paused = true)]
    async fn showing_again_removes_it_from_the_waiting_ones() {
        let mut center = center();
        let now = Instant::now();
        send(&mut center, &message("un"), now);
        send(&mut center, &message("deux"), now);
        let second = center.handle().list()[1].id;
        center.show_again(second, now);
        assert_eq!(shown_text(&center), Some("deux"));
        center.read(now);
        assert_eq!(shown_text(&center), None);
    }

    #[tokio::test(start_paused = true)]
    async fn the_oldest_notifications_are_forgotten_past_the_limit() {
        let mut center = center();
        let now = Instant::now();
        for index in 0..=MAX_UNREAD {
            send(&mut center, &message(&index.to_string()), now);
        }
        let unread = center.handle().list();
        assert_eq!(unread.len(), MAX_UNREAD);
        assert_eq!(unread[0].message, "1");
        // The one on the strip was the oldest: it is gone, the next shows.
        assert_eq!(shown_text(&center), Some("1"));
    }

    #[tokio::test(start_paused = true)]
    async fn without_a_strip_notifications_only_count() {
        let mut center = NotificationCenter::new(false);
        let now = Instant::now();
        send(&mut center, &message("un"), now);
        assert!(center.shown().is_none());
        assert_eq!(center.handle().count(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn only_moving_notifications_ask_for_animation_steps() {
        let mut center = center();
        let now = Instant::now();
        send(&mut center, &message("un"), now);
        // Not measured yet: assumed to scroll.
        assert!(center.next_step().is_some());
        center.set_scrolls(false);
        assert!(center.next_step().is_none());
        center.dismiss(now);
        send(&mut center, &timed("deux", 5), now);
        center.set_scrolls(false);
        assert_eq!(center.next_step(), Some(now + ANIMATION_STEP));
    }

    #[tokio::test(start_paused = true)]
    async fn elapsed_time_counts_whole_steps_and_is_zero_when_still() {
        let mut center = center();
        let start = Instant::now();
        send(&mut center, &timed("un", 9), start);
        center.set_scrolls(false);
        let shown = center.shown().unwrap();
        assert_eq!(
            shown.elapsed(start + ANIMATION_STEP * 3 + Duration::from_millis(5)),
            ANIMATION_STEP * 3
        );
        center.dismiss(start);
        send(&mut center, &message("deux"), start);
        center.set_scrolls(false);
        assert_eq!(
            center
                .shown()
                .unwrap()
                .elapsed(start + Duration::from_secs(2)),
            Duration::ZERO
        );
    }
}
