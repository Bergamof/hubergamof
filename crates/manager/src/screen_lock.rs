//! Whether the computer is locked: the devices go dark and ignore their
//! inputs meanwhile.
//!
//! The lock is read from `systemd-logind` (`LockedHint` of the user session),
//! which KDE, GNOME and the other desktops keep up to date.

use std::sync::Arc;

use futures_util::StreamExt as _;
use tokio::sync::watch;
use zbus::{Connection, OwnedObjectPath, proxy};

/// Name given to the logind session of the caller: the session of the user
/// even when the manager runs as a user service, outside any session.
const AUTO_SESSION: &str = "auto";

/// Source of the lock state of the computer.
pub trait ScreenLock: Send + Sync {
    /// Follows the lock state: `true` while the computer is locked.
    fn watch(&self) -> watch::Receiver<bool>;
}

/// A computer that never locks: without logind, or to switch the feature off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoScreenLock;

impl ScreenLock for NoScreenLock {
    fn watch(&self) -> watch::Receiver<bool> {
        watch::Sender::new(false).subscribe()
    }
}

/// A lock driven by hand: demo, tests. Clones share the state.
#[derive(Debug, Clone)]
pub struct SimulatedScreenLock {
    state: Arc<watch::Sender<bool>>,
}

impl Default for SimulatedScreenLock {
    fn default() -> Self {
        Self {
            state: Arc::new(watch::Sender::new(false)),
        }
    }
}

impl SimulatedScreenLock {
    /// Locks or unlocks the computer.
    pub fn set(&self, locked: bool) {
        self.state.send_replace(locked);
    }
}

impl ScreenLock for SimulatedScreenLock {
    fn watch(&self) -> watch::Receiver<bool> {
        self.state.subscribe()
    }
}

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LogindManager {
    fn get_session(&self, session_id: &str) -> zbus::Result<OwnedObjectPath>;
}

#[proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait LogindSession {
    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;
}

/// The lock state of the user session of `systemd-logind` (system bus).
#[derive(Debug, Clone)]
pub struct LogindScreenLock {
    state: Arc<watch::Sender<bool>>,
}

impl LogindScreenLock {
    /// Starts following the session in the background. Without logind the
    /// computer is considered never locked (reported once in the logs).
    #[must_use]
    pub fn start() -> Self {
        let state = Arc::new(watch::Sender::new(false));
        let followed = Arc::clone(&state);
        tokio::spawn(async move {
            match follow_logind(&followed).await {
                Ok(()) => tracing::warn!("the logind session ended, no lock detection anymore"),
                Err(error) => {
                    tracing::warn!(%error, "no lock detection (systemd-logind unreachable)");
                }
            }
            // A lock that nobody can announce the end of must not stay.
            followed.send_replace(false);
        });
        Self { state }
    }
}

impl ScreenLock for LogindScreenLock {
    fn watch(&self) -> watch::Receiver<bool> {
        self.state.subscribe()
    }
}

async fn follow_logind(state: &watch::Sender<bool>) -> zbus::Result<()> {
    let connection = Connection::system().await?;
    let session = LogindManagerProxy::new(&connection)
        .await?
        .get_session(AUTO_SESSION)
        .await?;
    follow_session(&connection, session, state).await
}

/// Publishes `LockedHint` of `session` in `state` now and at each change;
/// returns when the connection closes.
async fn follow_session(
    connection: &Connection,
    session: OwnedObjectPath,
    state: &watch::Sender<bool>,
) -> zbus::Result<()> {
    let session = LogindSessionProxy::builder(connection)
        .path(session)
        .build()
        .await?;
    // Subscribed before the first read: a change in between is not missed.
    let mut changes = session.receive_locked_hint_changed().await;
    state.send_replace(session.locked_hint().await?);
    while let Some(change) = changes.next().await {
        let locked = change.get().await?;
        tracing::info!(locked, "lock state changed");
        state.send_replace(locked);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use zbus::{Guid, connection::Builder, interface};

    use super::*;

    const SESSION_PATH: &str = "/org/freedesktop/login1/session/_31";

    struct FakeSession {
        locked: bool,
    }

    #[interface(name = "org.freedesktop.login1.Session")]
    impl FakeSession {
        #[zbus(property)]
        fn locked_hint(&self) -> bool {
            self.locked
        }
    }

    async fn connect(locked: bool) -> (Connection, Connection) {
        let (server_socket, client_socket) = UnixStream::pair().unwrap();
        let server = Builder::unix_stream(server_socket)
            .serve_at(SESSION_PATH, FakeSession { locked })
            .server(Guid::generate())
            .p2p()
            .build();
        let client = Builder::unix_stream(client_socket).p2p().build();
        let (server, client) = tokio::join!(server, client);
        (server.unwrap(), client.unwrap())
    }

    async fn locked_after_change(state: &mut watch::Receiver<bool>) -> bool {
        tokio::time::timeout(std::time::Duration::from_secs(5), state.changed())
            .await
            .expect("the lock state is published")
            .unwrap();
        *state.borrow_and_update()
    }

    #[tokio::test]
    async fn the_hint_of_the_session_is_read_then_followed() {
        let (server, client) = connect(true).await;
        let state = watch::Sender::new(false);
        let mut seen = state.subscribe();
        let follower = tokio::spawn(async move {
            let _ = follow_session(&client, SESSION_PATH.try_into().unwrap(), &state).await;
        });

        assert!(locked_after_change(&mut seen).await, "locked at the start");

        let object = server
            .object_server()
            .interface::<_, FakeSession>(SESSION_PATH)
            .await
            .unwrap();
        object.get_mut().await.locked = false;
        object
            .get()
            .await
            .locked_hint_changed(object.signal_emitter())
            .await
            .unwrap();
        assert!(!locked_after_change(&mut seen).await, "unlocked later");
        follower.abort();
    }

    #[test]
    fn a_simulated_lock_is_shared_by_its_clones() {
        let lock = SimulatedScreenLock::default();
        let watcher = lock.watch();
        assert!(!*watcher.borrow());

        lock.clone().set(true);

        assert!(*watcher.borrow());
    }

    #[test]
    fn no_lock_is_never_locked() {
        assert!(!*NoScreenLock.watch().borrow());
    }
}
