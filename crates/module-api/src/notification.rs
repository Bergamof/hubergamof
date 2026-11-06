//! Notifications: messages a module shows over the whole touch strip.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::{ImageData, ModuleInstanceId};

/// Opaque colour, as red, green and blue components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// A message shown over the whole touch strip until its timeout elapses or
/// the user touches the strip. Returned with [`crate::Command::Notify`].
///
/// A notification the user touches is *read*; one that vanishes by timeout,
/// or is dismissed with the navigation dial, stays *unread* and can be shown
/// again from the notifications module. A message too long for the strip
/// scrolls.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Notification {
    /// Text shown, on one line.
    pub message: String,
    /// Background colour; the host picks black or white text to stay legible.
    pub background: Rgb,
    /// How long it stays; `None` keeps it until the user dismisses it.
    pub timeout: Option<Duration>,
}

impl Notification {
    /// A notification of `message` on `background`, with no timeout.
    #[must_use]
    pub fn new(message: impl Into<String>, background: Rgb) -> Self {
        Self {
            message: message.into(),
            background,
            timeout: None,
        }
    }

    /// The same notification vanishing after `timeout`; a zero duration
    /// means no timeout.
    #[must_use]
    pub fn with_timeout(self, timeout: Duration) -> Self {
        Self {
            timeout: (!timeout.is_zero()).then_some(timeout),
            ..self
        }
    }
}

/// Identifier of a notification, unique for a device while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NotificationId(pub u64);

/// A notification the user has not touched yet.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnreadNotification {
    /// Identifier, for [`crate::Command::ShowNotification`].
    pub id: NotificationId,
    /// Module instance that sent it.
    pub source: ModuleInstanceId,
    /// Icon of the sending module, if it has one.
    pub icon: Option<ImageData>,
    /// Message.
    pub message: String,
    /// Background colour.
    pub background: Rgb,
}

impl UnreadNotification {
    /// An unread notification, for hosts and tests.
    #[must_use]
    pub fn new(
        id: NotificationId,
        source: ModuleInstanceId,
        icon: Option<ImageData>,
        notification: &Notification,
    ) -> Self {
        Self {
            id,
            source,
            icon,
            message: notification.message.clone(),
            background: notification.background,
        }
    }
}

/// Read access to the unread notifications of the device, oldest first.
///
/// Reading is cheap and side-effect free, like a getter: the host redraws
/// whenever the list changes.
#[derive(Clone)]
pub struct UnreadNotifications {
    read: Arc<dyn Fn() -> Vec<UnreadNotification> + Send + Sync>,
}

impl UnreadNotifications {
    /// Handle reading the list with `read`; used by the host.
    #[must_use]
    pub fn new(read: impl Fn() -> Vec<UnreadNotification> + Send + Sync + 'static) -> Self {
        Self {
            read: Arc::new(read),
        }
    }

    /// A list that stays empty.
    #[must_use]
    pub fn none() -> Self {
        Self::new(Vec::new)
    }

    /// The unread notifications, oldest first.
    #[must_use]
    pub fn list(&self) -> Vec<UnreadNotification> {
        (self.read)()
    }

    /// Number of unread notifications.
    #[must_use]
    pub fn count(&self) -> usize {
        self.list().len()
    }
}

impl fmt::Debug for UnreadNotifications {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnreadNotifications")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_timeout_means_none() {
        let notification = Notification::new("x", Rgb(0, 0, 0));
        assert_eq!(notification.timeout, None);
        assert_eq!(
            notification
                .clone()
                .with_timeout(Duration::from_secs(3))
                .timeout,
            Some(Duration::from_secs(3))
        );
        assert_eq!(notification.with_timeout(Duration::ZERO).timeout, None);
    }

    #[test]
    fn the_handle_reads_the_host_list() {
        let notification = Notification::new("fin", Rgb(1, 2, 3));
        let unread = UnreadNotification::new(NotificationId(7), "p".into(), None, &notification);
        let list = vec![unread];
        let handle = UnreadNotifications::new({
            let list = list.clone();
            move || list.clone()
        });
        assert_eq!(handle.list(), list);
        assert_eq!(handle.count(), 1);
        assert_eq!(UnreadNotifications::none().count(), 0);
    }
}
