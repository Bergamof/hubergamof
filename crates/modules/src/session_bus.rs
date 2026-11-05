//! The D-Bus session bus, shared by every module that talks to the desktop
//! (notifications, KWin, the notification monitor's lookups).

use std::fmt;
use std::sync::Arc;

use tokio::sync::OnceCell;
use zbus::Connection;

/// The D-Bus session bus, connected on first use and shared by every clone
/// (one connection for all the modules of the manager). A failed connection
/// is retried at the next use.
#[derive(Clone, Default)]
pub struct SessionBus {
    connection: Arc<OnceCell<Connection>>,
}

impl SessionBus {
    /// Bus of the desktop session, not connected yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bus already connected through `connection` (tests: a peer-to-peer
    /// connection to a fake service).
    #[must_use]
    pub fn with_connection(connection: Connection) -> Self {
        Self {
            connection: Arc::new(OnceCell::new_with(Some(connection))),
        }
    }

    /// The connection, opened now if needed.
    ///
    /// # Errors
    ///
    /// [`zbus::Error`] when the session bus cannot be reached.
    pub async fn connection(&self) -> zbus::Result<&Connection> {
        self.connection.get_or_try_init(Connection::session).await
    }
}

impl fmt::Debug for SessionBus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionBus")
            .field("connected", &self.connection.initialized())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_bus_connects_only_when_used() {
        let bus = SessionBus::new();
        assert_eq!(format!("{bus:?}"), "SessionBus { connected: false }");
        let shared = bus.clone();
        assert!(Arc::ptr_eq(&bus.connection, &shared.connection));
    }
}
