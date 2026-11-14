//! Whether the computer is locked: the devices go dark and ignore their
//! inputs meanwhile.
//!
//! The lock is read from `systemd-logind` (`LockedHint` of the user session,
//! which KDE, GNOME and the other desktops keep up to date): a session that is
//! no longer the active one (fast user switching) counts as locked too.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use tokio::sync::watch;
use zbus::{Connection, OwnedObjectPath, proxy};

/// First wait before reaching logind again, doubled up to [`MAX_RECONNECT_DELAY`].
const FIRST_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

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

/// A lock driven by hand, for the tests. Clones share the state.
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct SimulatedScreenLock {
    state: Arc<watch::Sender<bool>>,
}

#[cfg(test)]
impl Default for SimulatedScreenLock {
    fn default() -> Self {
        Self {
            state: Arc::new(watch::Sender::new(false)),
        }
    }
}

#[cfg(test)]
impl SimulatedScreenLock {
    /// Locks or unlocks the computer.
    pub fn set(&self, locked: bool) {
        self.state.send_replace(locked);
    }
}

#[cfg(test)]
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

    #[zbus(property)]
    fn active(&self) -> zbus::Result<bool>;
}

/// The lock state of the user session of `systemd-logind` (system bus).
#[derive(Debug, Clone)]
pub struct LogindScreenLock {
    state: Arc<watch::Sender<bool>>,
}

impl LogindScreenLock {
    /// Starts following the session in the background. Without logind the
    /// computer is considered never locked (reported once in the logs).
    ///
    /// # Panics
    ///
    /// Outside a Tokio runtime.
    #[must_use]
    pub fn start() -> Self {
        let state = Arc::new(watch::Sender::new(false));
        let followed = Arc::clone(&state);
        tokio::spawn(async move {
            let mut delay = FIRST_RECONNECT_DELAY;
            loop {
                match follow_logind(&followed).await {
                    Ok(()) => tracing::warn!("the logind session ended, no lock detection"),
                    Err(error) => {
                        tracing::warn!(%error, "no lock detection (systemd-logind unreachable)");
                    }
                }
                // A lock that nobody can announce the end of must not stay.
                followed.send_replace(false);
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(MAX_RECONNECT_DELAY);
            }
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
    let mut hint_changes = session.receive_locked_hint_changed().await;
    let mut active_changes = session.receive_active_changed().await;
    loop {
        let locked = session.locked_hint().await? || !session.active().await?;
        if state.send_replace(locked) != locked {
            tracing::info!(locked, "lock state changed");
        }
        tokio::select! {
            change = hint_changes.next() => change,
            change = active_changes.next() => change,
        }
        .ok_or_else(|| zbus::Error::Failure("the session stopped answering".into()))?;
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use zbus::{Guid, connection::Builder, interface};

    use super::*;

    const SESSION_PATH: &str = "/org/freedesktop/login1/session/_31";

    struct FakeSession {
        locked: bool,
        active: bool,
    }

    #[interface(name = "org.freedesktop.login1.Session")]
    impl FakeSession {
        #[zbus(property)]
        fn locked_hint(&self) -> bool {
            self.locked
        }

        #[zbus(property)]
        fn active(&self) -> bool {
            self.active
        }
    }

    async fn connect(locked: bool, active: bool) -> (Connection, Connection) {
        let (server_socket, client_socket) = UnixStream::pair().unwrap();
        let server = Builder::unix_stream(server_socket)
            .serve_at(SESSION_PATH, FakeSession { locked, active })
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
        let (server, client) = connect(true, true).await;
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

    #[tokio::test]
    async fn a_session_that_is_not_the_active_one_counts_as_locked() {
        let (_server, client) = connect(false, false).await;
        let state = watch::Sender::new(false);
        let mut seen = state.subscribe();
        let follower = tokio::spawn(async move {
            let _ = follow_session(&client, SESSION_PATH.try_into().unwrap(), &state).await;
        });

        assert!(locked_after_change(&mut seen).await);
        follower.abort();
    }

    #[tokio::test]
    async fn the_end_of_the_connection_ends_the_following() {
        let (server, client) = connect(true, true).await;
        let state = watch::Sender::new(false);
        let follower = tokio::spawn(async move {
            follow_session(&client, SESSION_PATH.try_into().unwrap(), &state).await
        });
        drop(server);

        let result = tokio::time::timeout(std::time::Duration::from_secs(5), follower)
            .await
            .expect("the following ends")
            .unwrap();
        assert!(result.is_err());
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
