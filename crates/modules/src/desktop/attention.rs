//! The attention a module shows for its application: while one of its
//! windows asks for it, and after a notification of it.
//!
//! Applications notify while in the background (tests run, Claude asking
//! for a permission…): the module's key goes in attention until the user
//! turns to the application — by pressing the key, or by bringing one of
//! its windows to the front. The texts are neither kept nor read: they are
//! translated and change with the versions.

use std::any::Any;

use bergadeck_module_api::EventSender;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use super::model::{AppId, AppNotification, AppState};

/// Whether an application asks for the user's attention: it notified
/// since the user last turned to it, or (when its windows are reliable)
/// one of its windows asks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppAttention {
    windows: WindowAttention,
    notified: bool,
    window_asks: bool,
    in_front: bool,
}

/// Whether the windows asking for attention count: some applications ask
/// whenever they lose the focus (Claude Desktop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowAttention {
    Counts,
    Ignored,
}

impl AppAttention {
    /// Attention from the notifications and the windows asking for it.
    #[must_use]
    pub fn with_windows() -> Self {
        Self::new(WindowAttention::Counts)
    }

    /// Attention from the notifications only.
    #[must_use]
    pub fn notifications_only() -> Self {
        Self::new(WindowAttention::Ignored)
    }

    fn new(windows: WindowAttention) -> Self {
        Self {
            windows,
            notified: false,
            window_asks: false,
            in_front: false,
        }
    }

    /// The application sent a notification: of no matter while it is in
    /// front, the user is already looking at it.
    pub fn notified(&mut self) {
        self.notified |= !self.in_front;
    }

    /// The application's state changed: one of its windows in front ends
    /// the attention of its notifications.
    pub fn app_changed(&mut self, app: &AppState) {
        self.in_front = app.is_in_front();
        self.notified &= !self.in_front;
        self.window_asks = self.windows == WindowAttention::Counts && app.demands_attention();
    }

    /// The user pressed the key: ends the attention of the notifications
    /// (a window asking keeps asking).
    pub fn acknowledge(&mut self) {
        self.notified = false;
    }

    /// Whether the key should be in attention.
    #[must_use]
    pub fn is_on(self) -> bool {
        self.notified || self.window_asks
    }
}

/// Hands an instance the event `notified` makes each time `app` sends a
/// notification (missed ones are of no matter: one is enough); ends with
/// the instance, or when nothing publishes any more.
pub async fn forward_notifications<E: Any + Send>(
    mut notifications: broadcast::Receiver<AppNotification>,
    app: AppId,
    events: EventSender,
    notified: impl Fn() -> E,
) {
    loop {
        match notifications.recv().await {
            Ok(notification) if notification.app == app => {
                if !events.send(notified()) {
                    return;
                }
            }
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bergadeck_module_api::testing::TestContext;

    use super::*;
    use crate::desktop::Window;

    const APP: AppId = AppId("app");

    fn with_window(active: bool, demands_attention: bool) -> AppState {
        let mut window = Window::new("1", "app", "App");
        window.active = active;
        window.demands_attention = demands_attention;
        AppState {
            running: Some(true),
            windows: Some(Arc::from([window])),
        }
    }

    #[test]
    fn notifications_ask_until_pressed_or_in_front() {
        let mut attention = AppAttention::with_windows();
        assert!(!attention.is_on());
        attention.notified();
        attention.app_changed(&with_window(false, false));
        assert!(attention.is_on(), "in the background");
        attention.acknowledge();
        assert!(!attention.is_on(), "pressed");

        attention.notified();
        attention.app_changed(&with_window(true, false));
        assert!(!attention.is_on(), "in front");
        attention.notified();
        assert!(!attention.is_on(), "notified while in front");
        attention.app_changed(&with_window(false, false));
        assert!(!attention.is_on(), "seen meanwhile");
    }

    #[test]
    fn a_window_asks_for_as_long_as_it_does() {
        let mut attention = AppAttention::with_windows();
        attention.app_changed(&with_window(false, true));
        attention.acknowledge();
        assert!(attention.is_on(), "still asking");
        attention.app_changed(&with_window(false, false));
        assert!(!attention.is_on());
    }

    #[test]
    fn windows_asking_may_be_ignored() {
        let mut attention = AppAttention::notifications_only();
        attention.app_changed(&with_window(false, true));
        assert!(!attention.is_on(), "window asking");
        attention.notified();
        assert!(attention.is_on(), "notified");
    }

    #[tokio::test(start_paused = true)]
    async fn only_the_notifications_of_the_app_are_forwarded() {
        let TestContext {
            ctx, mut events, ..
        } = TestContext::new("app");
        let (sender, notifications) = broadcast::channel(4);
        let forwarding = tokio::spawn(forward_notifications(
            notifications,
            APP,
            ctx.events().clone(),
            || "notified",
        ));
        let notification = |app| AppNotification { app };
        sender.send(notification(AppId("other"))).unwrap();
        sender.send(notification(APP)).unwrap();
        drop(sender);
        forwarding.await.unwrap();

        let event = events.recv().await.unwrap();
        assert_eq!(*event.downcast::<&str>().unwrap(), "notified");
        assert!(events.try_recv().is_err(), "only the app's");
    }
}
