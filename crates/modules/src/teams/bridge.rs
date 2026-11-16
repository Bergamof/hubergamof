//! The link with the browser extension: what Teams last showed
//! ([`TeamsState`]) and the actions asked of it ([`TeamsBridge`]).
//!
//! The real bridge ([`SocketBridge`]) listens on a Unix socket of the
//! user's runtime directory; `hubergamof native-host`, started by the
//! browser for the extension, relays the extension to it. The simulated one
//! ([`SimulatedBridge`]) serves `--demo` and the tests.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, watch};

use super::wire::{
    BrowserReport, CallAction, CallPhase, MAX_FRAME_BYTES, ManagerRequest, Meeting, read_frame,
    write_frame,
};

/// Folder of the socket in the runtime directory of the user.
const SOCKET_DIR: &str = "hubergamof";

/// File name of the socket.
const SOCKET_NAME: &str = "teams.sock";

/// Actions waiting for the extension, per connection.
const OUTGOING_QUEUE: usize = 8;

/// The call, as Teams shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallState {
    /// Where the call stands.
    pub phase: CallPhase,
    /// The microphone is muted; `None` when unknown.
    pub muted: Option<bool>,
    /// The camera is on; `None` when unknown.
    pub camera_on: Option<bool>,
    /// Who is calling, while it rings.
    pub caller: Option<String>,
}

/// What Teams last showed, as the extension reported it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TeamsState {
    /// An extension is connected: without one, nothing below is current.
    pub connected: bool,
    /// Number of unread conversations.
    pub unread: u32,
    /// Names of the unread conversations (possibly fewer than `unread`).
    pub conversations: Vec<String>,
    /// The call.
    pub call: CallState,
    /// Upcoming meetings.
    pub meetings: Vec<Meeting>,
}

impl TeamsState {
    /// The state after `report`, which is expected sanitized.
    pub(crate) fn apply(&mut self, report: BrowserReport) {
        match report {
            BrowserReport::Unread { count } => self.unread = count,
            BrowserReport::Conversations { unread } => self.conversations = unread,
            BrowserReport::Call {
                phase,
                muted,
                camera_on,
                caller,
            } => {
                self.call = CallState {
                    phase,
                    muted,
                    camera_on,
                    caller,
                };
            }
            BrowserReport::Meetings { items } => self.meetings = items,
        }
    }
}

/// What the Teams module reads and asks.
#[async_trait]
pub trait TeamsBridge: fmt::Debug + Send + Sync {
    /// What Teams last showed; changes as the extension reports.
    fn state(&self) -> watch::Receiver<TeamsState>;

    /// Asks the extension to do `action` in Teams; `false` when none is
    /// connected (nothing was sent).
    async fn request(&self, action: CallAction) -> bool;
}

/// Where the socket is: `$XDG_RUNTIME_DIR/hubergamof/teams.sock`, `None`
/// without a runtime directory.
#[must_use]
pub fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty())?;
    Some(PathBuf::from(runtime).join(SOCKET_DIR).join(SOCKET_NAME))
}

/// The extension of the connection that is current, if any.
struct Current {
    generation: u64,
    actions: mpsc::Sender<CallAction>,
}

struct Shared {
    state: watch::Sender<TeamsState>,
    current: Mutex<Option<Current>>,
    generations: AtomicU64,
}

impl Shared {
    /// A new connection becomes the current one; the previous one, if any,
    /// ends as its action channel closes.
    fn connected(&self) -> (u64, mpsc::Receiver<CallAction>) {
        let generation = self.generations.fetch_add(1, Ordering::SeqCst) + 1;
        let (actions, receiver) = mpsc::channel(OUTGOING_QUEUE);
        *self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Current {
            generation,
            actions,
        });
        self.state.send_modify(|state| {
            *state = TeamsState {
                connected: true,
                ..TeamsState::default()
            };
        });
        (generation, receiver)
    }

    /// The connection `generation` ended: if it still was the current one,
    /// nothing is current any more.
    fn disconnected(&self, generation: u64) {
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current
            .as_ref()
            .is_some_and(|current| current.generation == generation)
        {
            *current = None;
            self.state.send_replace(TeamsState::default());
        }
    }

    fn is_current(&self, generation: u64) -> bool {
        self.current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|current| current.generation == generation)
    }
}

/// The bridge to the real extension, over the socket of the user. Listens
/// on first use of [`TeamsBridge::state`], so building it touches nothing.
pub struct SocketBridge {
    path: Option<PathBuf>,
    shared: Arc<Shared>,
    started: OnceLock<()>,
}

impl SocketBridge {
    /// A bridge listening on `path` (`None`: never listens, nothing is ever
    /// connected).
    #[must_use]
    pub fn new(path: Option<PathBuf>) -> Self {
        let (state, _) = watch::channel(TeamsState::default());
        Self {
            path,
            shared: Arc::new(Shared {
                state,
                current: Mutex::new(None),
                generations: AtomicU64::new(0),
            }),
            started: OnceLock::new(),
        }
    }

    fn start(&self) {
        if self.started.set(()).is_err() {
            return;
        }
        let Some(path) = self.path.clone() else {
            tracing::warn!("no runtime directory: Teams cannot be reached");
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("no runtime to listen for the Teams extension");
            return;
        };
        let shared = Arc::clone(&self.shared);
        runtime.spawn(async move {
            match listen(&path) {
                Ok(listener) => accept_loop(listener, shared).await,
                Err(error) => tracing::warn!(%error, "cannot listen for the Teams extension"),
            }
        });
    }
}

impl fmt::Debug for SocketBridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SocketBridge")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl TeamsBridge for SocketBridge {
    fn state(&self) -> watch::Receiver<TeamsState> {
        self.start();
        self.shared.state.subscribe()
    }

    async fn request(&self, action: CallAction) -> bool {
        let actions = self
            .shared
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|current| current.actions.clone());
        actions.is_some_and(|actions| actions.try_send(action).is_ok())
    }
}

/// Binds the socket: its folder is the user's own (0700), a stale socket
/// left by a previous run is replaced, the socket itself is 0600.
fn listen(path: &Path) -> std::io::Result<UnixListener> {
    use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};

    let folder = path
        .parent()
        .ok_or_else(|| std::io::Error::other("socket path without a folder"))?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)?;
    let uid = rustix::process::getuid().as_raw();
    let metadata = std::fs::symlink_metadata(folder)?;
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err(std::io::Error::other(
            "the socket folder is not private to the user",
        ));
    }
    match std::fs::symlink_metadata(path) {
        Ok(existing) if existing.file_type().is_socket() && existing.uid() == uid => {
            std::fs::remove_file(path)?;
        }
        Ok(_) => return Err(std::io::Error::other("not a socket of the user in the way")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

async fn accept_loop(listener: UnixListener, shared: Arc<Shared>) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(serve(stream, Arc::clone(&shared)));
            }
            Err(error) => {
                tracing::warn!(%error, "cannot accept a Teams extension connection");
                return;
            }
        }
    }
}

/// Whether the peer of `stream` is the user running the manager.
fn is_same_user(stream: &UnixStream) -> bool {
    stream
        .peer_cred()
        .is_ok_and(|credentials| credentials.uid() == rustix::process::getuid().as_raw())
}

/// One connection of the native host: reports in, actions out, until either
/// side ends or a newer connection replaces this one.
async fn serve(stream: UnixStream, shared: Arc<Shared>) {
    if !is_same_user(&stream) {
        tracing::warn!("Teams extension connection of another user refused");
        return;
    }
    let (mut reader, mut writer) = stream.into_split();
    let (generation, mut actions) = shared.connected();
    tracing::info!("Teams extension connected");
    loop {
        tokio::select! {
            frame = read_frame(&mut reader, MAX_FRAME_BYTES) => match frame {
                Ok(Some(frame)) => {
                    if shared.is_current(generation) {
                        handle_frame(&shared, &frame);
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    tracing::debug!(%error, "Teams extension stream failed");
                    break;
                }
            },
            action = actions.recv() => {
                let Some(action) = action else { break };
                let Ok(bytes) = serde_json::to_vec(&ManagerRequest::Do { action }) else {
                    continue;
                };
                if write_frame(&mut writer, &bytes).await.is_err() {
                    break;
                }
            }
        }
    }
    shared.disconnected(generation);
    tracing::info!("Teams extension disconnected");
}

/// A frame of the extension: a report, or garbage that is ignored.
fn handle_frame(shared: &Shared, frame: &[u8]) {
    match serde_json::from_slice::<BrowserReport>(frame) {
        Ok(report) => {
            let report = report.sanitized();
            shared.state.send_if_modified(|state| {
                let before = state.clone();
                state.apply(report);
                *state != before
            });
        }
        Err(error) => tracing::debug!(%error, "unreadable Teams report ignored"),
    }
}

/// A bridge without a browser, for `--demo` and the tests: reports are given
/// by hand and the actions asked are kept.
#[derive(Debug)]
pub struct SimulatedBridge {
    state: watch::Sender<TeamsState>,
    requests: Mutex<Vec<CallAction>>,
}

impl SimulatedBridge {
    /// A connected extension that has reported nothing yet.
    #[must_use]
    pub fn new() -> Self {
        let (state, _) = watch::channel(TeamsState {
            connected: true,
            ..TeamsState::default()
        });
        Self {
            state,
            requests: Mutex::new(Vec::new()),
        }
    }

    /// The demo: two unread conversations, no call.
    #[must_use]
    pub fn demo() -> Self {
        let bridge = Self::new();
        bridge.report(BrowserReport::Unread { count: 2 });
        bridge.report(BrowserReport::Conversations {
            unread: vec!["Équipe produit".into(), "Camille".into()],
        });
        bridge
    }

    /// The extension reports `report`.
    pub fn report(&self, report: BrowserReport) {
        let report = report.sanitized();
        self.state.send_modify(|state| state.apply(report));
    }

    /// The extension connects or disconnects.
    pub fn set_connected(&self, connected: bool) {
        self.state.send_modify(|state| {
            *state = TeamsState {
                connected,
                ..TeamsState::default()
            };
        });
    }

    /// The actions asked so far.
    #[must_use]
    pub fn requests(&self) -> Vec<CallAction> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Default for SimulatedBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TeamsBridge for SimulatedBridge {
    fn state(&self) -> watch::Receiver<TeamsState> {
        self.state.subscribe()
    }

    async fn request(&self, action: CallAction) -> bool {
        if !self.state.borrow().connected {
            return false;
        }
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(action);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::net::UnixStream;
    use tokio::time::timeout;

    use super::*;

    const WAIT: Duration = Duration::from_secs(5);

    async fn bridge_in(dir: &tempfile::TempDir) -> (SocketBridge, watch::Receiver<TeamsState>) {
        let bridge = SocketBridge::new(Some(dir.path().join("run").join("teams.sock")));
        let state = bridge.state();
        // The listener starts in the background: wait for the socket.
        let path = dir.path().join("run").join("teams.sock");
        timeout(WAIT, async {
            while !path.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        (bridge, state)
    }

    async fn extension(dir: &tempfile::TempDir) -> UnixStream {
        UnixStream::connect(dir.path().join("run").join("teams.sock"))
            .await
            .unwrap()
    }

    async fn send_report(stream: &mut UnixStream, json: &str) {
        write_frame(stream, json.as_bytes()).await.unwrap();
    }

    async fn state_where(
        state: &mut watch::Receiver<TeamsState>,
        done: impl Fn(&TeamsState) -> bool,
    ) -> TeamsState {
        timeout(WAIT, state.wait_for(|state| done(state)))
            .await
            .unwrap()
            .unwrap()
            .clone()
    }

    #[tokio::test]
    async fn the_socket_is_private_to_the_user() {
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let (_bridge, _state) = bridge_in(&dir).await;
        let folder = dir.path().join("run");
        let socket = folder.join("teams.sock");
        assert_eq!(
            std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let metadata = std::fs::metadata(&socket).unwrap();
        assert!(metadata.file_type().is_socket());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    }

    #[tokio::test]
    async fn reports_update_the_state_and_actions_reach_the_extension() {
        let dir = tempfile::tempdir().unwrap();
        let (bridge, mut state) = bridge_in(&dir).await;
        assert!(!state.borrow().connected);
        assert!(!bridge.request(CallAction::HangUp).await);

        let mut stream = extension(&dir).await;
        state_where(&mut state, |state| state.connected).await;
        send_report(&mut stream, r#"{"type":"unread","count":3}"#).await;
        send_report(
            &mut stream,
            r#"{"type":"call","phase":"inCall","muted":true,"cameraOn":false,"caller":null}"#,
        )
        .await;
        let seen = state_where(&mut state, |state| state.call.phase == CallPhase::InCall).await;
        assert_eq!(seen.unread, 3);
        assert_eq!(seen.call.muted, Some(true));

        assert!(bridge.request(CallAction::ToggleMute).await);
        let frame = timeout(WAIT, read_frame(&mut stream, 64))
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(frame, br#"{"type":"do","action":"toggleMute"}"#);
    }

    #[tokio::test]
    async fn garbage_and_unbounded_text_are_handled() {
        let dir = tempfile::tempdir().unwrap();
        let (_bridge, mut state) = bridge_in(&dir).await;
        let mut stream = extension(&dir).await;
        state_where(&mut state, |state| state.connected).await;

        send_report(&mut stream, "not json").await;
        send_report(&mut stream, r#"{"type":"eval","code":"x"}"#).await;
        let long = "x".repeat(1000);
        send_report(
            &mut stream,
            &format!(r#"{{"type":"conversations","unread":["{long}"]}}"#),
        )
        .await;
        let seen = state_where(&mut state, |state| !state.conversations.is_empty()).await;
        assert_eq!(seen.conversations[0].chars().count(), 120);
    }

    #[tokio::test]
    async fn the_state_is_forgotten_when_the_extension_leaves() {
        let dir = tempfile::tempdir().unwrap();
        let (bridge, mut state) = bridge_in(&dir).await;
        let mut stream = extension(&dir).await;
        send_report(&mut stream, r#"{"type":"unread","count":5}"#).await;
        state_where(&mut state, |state| state.unread == 5).await;

        drop(stream);
        let seen = state_where(&mut state, |state| !state.connected).await;
        assert_eq!(seen, TeamsState::default());
        assert!(!bridge.request(CallAction::HangUp).await);
    }

    #[tokio::test]
    async fn a_newer_connection_replaces_the_older_one() {
        let dir = tempfile::tempdir().unwrap();
        let (bridge, mut state) = bridge_in(&dir).await;
        let mut first = extension(&dir).await;
        state_where(&mut state, |state| state.connected).await;
        let mut second = extension(&dir).await;
        // Let the second connection take over.
        tokio::time::sleep(Duration::from_millis(100)).await;

        // The older connection was closed by the bridge: a late report fails
        // or is ignored, never counted.
        let _ = write_frame(&mut first, br#"{"type":"unread","count":9}"#).await;
        send_report(&mut second, r#"{"type":"unread","count":1}"#).await;
        state_where(&mut state, |state| state.unread == 1).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(state.borrow().unread, 1, "the older one no longer counts");

        assert!(bridge.request(CallAction::Decline).await);
        let frame = timeout(WAIT, read_frame(&mut second, 64))
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(frame, br#"{"type":"do","action":"decline"}"#);
    }

    #[tokio::test]
    async fn without_a_runtime_directory_nothing_is_ever_connected() {
        let bridge = SocketBridge::new(None);
        let state = bridge.state();
        assert!(!state.borrow().connected);
        assert!(!bridge.request(CallAction::HangUp).await);
    }

    #[tokio::test]
    async fn the_simulated_bridge_keeps_what_was_asked() {
        let bridge = SimulatedBridge::demo();
        assert_eq!(bridge.state().borrow().unread, 2);
        assert!(bridge.request(CallAction::HangUp).await);
        assert_eq!(bridge.requests(), [CallAction::HangUp]);
        bridge.set_connected(false);
        assert!(!bridge.request(CallAction::HangUp).await);
        assert_eq!(bridge.requests().len(), 1);
    }
}
