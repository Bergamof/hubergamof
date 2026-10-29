//! Desktop notifications (`org.freedesktop.Notifications`), behind the
//! [`Notifier`] trait so that modules can be tested without a session bus.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::OnceCell;
use zbus::{Connection, Variant, proxy};

/// Application name shown by the notification server.
const APP_NAME: &str = "Bergadeck";

/// Longest wait for the notification server, so that a stalled bus does not
/// keep a background task alive forever.
pub const NOTIFY_TIMEOUT: Duration = Duration::from_secs(5);

/// Hint asking the server not to play its notification sound.
const SUPPRESS_SOUND_HINT: &str = "suppress-sound";

/// `expire_timeout` letting the server choose how long it shows the
/// notification.
const SERVER_DEFAULT_EXPIRATION: i32 = -1;

/// `replaces_id` of a new notification (replaces nothing).
const NEW_NOTIFICATION: u32 = 0;

/// A silent desktop notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// One-line summary, shown in bold.
    pub summary: String,
    /// Details.
    pub body: String,
}

/// Failure to show a notification.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NotifyError {
    /// The session bus or the notification server failed.
    #[error("D-Bus: {0}")]
    Bus(#[from] zbus::Error),
    /// The notification server did not answer within [`NOTIFY_TIMEOUT`].
    #[error("the notification server did not answer within {NOTIFY_TIMEOUT:?}")]
    Timeout,
}

/// Shows desktop notifications. Called from background tasks only: showing
/// one is I/O.
#[async_trait]
pub trait Notifier: Send + Sync {
    /// Shows `notification`.
    ///
    /// # Errors
    ///
    /// [`NotifyError`] when it could not be shown; callers log it and go on.
    async fn notify(&self, notification: &Notification) -> Result<(), NotifyError>;
}

#[proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    /// Shows a notification and returns its id.
    #[allow(
        clippy::too_many_arguments,
        reason = "signature fixed by the Desktop Notifications specification"
    )]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Variant<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;
}

/// Notifier of the desktop session, through the session bus opened on first
/// use (a failed connection is retried at the next notification).
#[derive(Debug, Default)]
pub struct DesktopNotifier {
    connection: OnceCell<Connection>,
}

impl DesktopNotifier {
    /// Notifier connecting to the session bus when first needed.
    #[must_use]
    pub fn session() -> Self {
        Self::default()
    }

    /// Notifier using `connection`, whose peer serves
    /// `org.freedesktop.Notifications`.
    #[must_use]
    pub fn with_connection(connection: Connection) -> Self {
        Self {
            connection: OnceCell::new_with(Some(connection)),
        }
    }

    async fn send(&self, notification: &Notification) -> Result<(), NotifyError> {
        let connection = self.connection.get_or_try_init(Connection::session).await?;
        // No property cache: it would issue a `GetAll` nobody needs.
        let proxy = NotificationsProxy::builder(connection)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        let hints = HashMap::from([(SUPPRESS_SOUND_HINT, Variant::from(true))]);
        proxy
            .notify(
                APP_NAME,
                NEW_NOTIFICATION,
                "",
                &notification.summary,
                &notification.body,
                &[],
                hints,
                SERVER_DEFAULT_EXPIRATION,
            )
            .await?;
        Ok(())
    }
}

#[async_trait]
impl Notifier for DesktopNotifier {
    async fn notify(&self, notification: &Notification) -> Result<(), NotifyError> {
        tokio::time::timeout(NOTIFY_TIMEOUT, self.send(notification))
            .await
            .map_err(|_| NotifyError::Timeout)?
    }
}

/// Notifier of the simulated environment (`--demo`, tests): logs and
/// remembers the notifications instead of showing them on the desktop.
#[derive(Debug, Default)]
pub struct SimulatedNotifier {
    shown: Mutex<Vec<Notification>>,
}

impl SimulatedNotifier {
    /// Notifications received so far, in order.
    #[must_use]
    pub fn shown(&self) -> Vec<Notification> {
        self.lock().clone()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Notification>> {
        // Only ever pushed to: a poisoned lock is still valid.
        self.shown.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl Notifier for SimulatedNotifier {
    async fn notify(&self, notification: &Notification) -> Result<(), NotifyError> {
        tracing::info!(
            summary = %notification.summary,
            body = %notification.body,
            "simulated desktop notification"
        );
        self.lock().push(notification.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;
    use std::sync::{Arc, Mutex};

    use zbus::connection::Builder;
    use zbus::{Guid, OwnedVariant, interface};

    use super::*;

    /// What the fake server received: summary, body and hints.
    type Received = Arc<Mutex<Vec<(String, String, HashMap<String, OwnedVariant>)>>>;

    struct FakeServer {
        received: Received,
    }

    #[interface(name = "org.freedesktop.Notifications")]
    impl FakeServer {
        #[allow(clippy::too_many_arguments, clippy::needless_pass_by_value)]
        fn notify(
            &self,
            app_name: String,
            replaces_id: u32,
            app_icon: String,
            summary: String,
            body: String,
            actions: Vec<String>,
            hints: HashMap<String, OwnedVariant>,
            expire_timeout: i32,
        ) -> u32 {
            assert_eq!(app_name, APP_NAME);
            assert_eq!(replaces_id, NEW_NOTIFICATION);
            assert!(app_icon.is_empty() && actions.is_empty());
            assert_eq!(expire_timeout, SERVER_DEFAULT_EXPIRATION);
            self.received.lock().unwrap().push((summary, body, hints));
            1
        }
    }

    /// A client connection whose peer is a fake notification server.
    async fn connected_client(received: Received) -> (Connection, Connection) {
        let (server_socket, client_socket) = UnixStream::pair().unwrap();
        let server = Builder::unix_stream(server_socket)
            .serve_at("/org/freedesktop/Notifications", FakeServer { received })
            .server(Guid::generate())
            .p2p()
            .build();
        let client = Builder::unix_stream(client_socket).p2p().build();
        let (server, client) = tokio::join!(server, client);
        (server.unwrap(), client.unwrap())
    }

    #[tokio::test]
    async fn notifications_are_sent_silently() {
        let received = Received::default();
        let (_server, client) = connected_client(Arc::clone(&received)).await;
        let notifier = DesktopNotifier::with_connection(client);
        let notification = Notification {
            summary: "Pause !".into(),
            body: "Pomodoro terminé.".into(),
        };
        notifier.notify(&notification).await.unwrap();

        let received = received.lock().unwrap();
        let [(summary, body, hints)] = received.as_slice() else {
            panic!("expected one notification, got {}", received.len());
        };
        assert_eq!(
            (summary.as_str(), body.as_str()),
            ("Pause !", "Pomodoro terminé.")
        );
        assert_eq!(hints[SUPPRESS_SOUND_HINT], OwnedVariant::from(true));
    }

    #[tokio::test]
    async fn simulated_notifications_are_only_recorded() {
        let notifier = SimulatedNotifier::default();
        let notification = Notification {
            summary: "Pause !".into(),
            body: "Pomodoro terminé.".into(),
        };
        notifier.notify(&notification).await.unwrap();
        notifier.notify(&notification).await.unwrap();
        assert_eq!(notifier.shown(), [notification.clone(), notification]);
    }
}
