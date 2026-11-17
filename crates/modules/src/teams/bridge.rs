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
use std::time::Duration;

use thiserror::Error;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, watch};

use super::wire::{
    BrowserReport, CallAction, CallPhase, MAX_FRAME_BYTES, ManagerRequest, Meeting, read_frame,
    write_frame,
};
use crate::background::spawn_worker;
use crate::sync::lock_unpoisoned;

/// Folder of the socket in the runtime directory of the user.
const SOCKET_DIR: &str = "hubergamof";

/// File name of the socket.
const SOCKET_NAME: &str = "teams.sock";

/// Actions waiting for the extension, per connection.
const OUTGOING_QUEUE: usize = 8;

/// Frames read from the extension and not yet handled, per connection.
const INCOMING_QUEUE: usize = 16;

/// Wait after a failed `accept` (descriptors exhausted, connection
/// aborted…) before listening again.
const ACCEPT_RETRY_DELAY: Duration = Duration::from_secs(1);

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
    /// Nothing known yet, with an extension connected or not.
    #[must_use]
    pub fn fresh(connected: bool) -> Self {
        Self {
            connected,
            ..Self::default()
        }
    }

    /// The state after `report`, which is sanitized here: what the page
    /// sent is never trusted. Tells whether the state changed.
    pub(crate) fn apply(&mut self, report: BrowserReport) -> bool {
        let before = self.clone();
        match report.sanitized() {
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
        *self != before
    }
}

/// Why an action was not sent to the extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RequestError {
    /// No extension is connected.
    #[error("no Teams extension is connected")]
    NotConnected,
    /// The extension has too many actions waiting.
    #[error("the Teams extension has too many actions waiting")]
    Busy,
}

/// What the Teams module reads and asks.
pub trait TeamsBridge: fmt::Debug + Send + Sync {
    /// What Teams last showed; changes as the extension reports.
    fn state(&self) -> watch::Receiver<TeamsState>;

    /// Asks the extension to do `action` in Teams, without waiting for it.
    ///
    /// # Errors
    ///
    /// Fails, sending nothing, when no extension is connected or its queue
    /// is full.
    fn request(&self, action: CallAction) -> Result<(), RequestError>;
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
        let mut current = lock_unpoisoned(&self.current);
        *current = Some(Current {
            generation,
            actions,
        });
        self.state.send_replace(TeamsState::fresh(true));
        (generation, receiver)
    }

    /// The connection `generation` ended: if it still was the current one,
    /// nothing is current any more.
    fn disconnected(&self, generation: u64) {
        let mut current = lock_unpoisoned(&self.current);
        if current
            .as_ref()
            .is_some_and(|current| current.generation == generation)
        {
            *current = None;
            self.state.send_replace(TeamsState::fresh(false));
        }
    }

    /// Applies `report` if the connection `generation` is still the
    /// current one; the check and the update happen under one lock, so a
    /// replaced connection cannot slip a report in after its successor
    /// started from a clean state.
    fn apply_if_current(&self, generation: u64, report: BrowserReport) {
        let current = lock_unpoisoned(&self.current);
        if current
            .as_ref()
            .is_some_and(|current| current.generation == generation)
        {
            self.state.send_if_modified(|state| state.apply(report));
        }
    }

    fn actions(&self) -> Option<mpsc::Sender<CallAction>> {
        lock_unpoisoned(&self.current)
            .as_ref()
            .map(|current| current.actions.clone())
    }
}

/// The bridge to the real extension, over the socket of the user. Listens
/// on first use of [`TeamsBridge::state`], so building it touches nothing;
/// the listener lives as long as the process (the socket is replaced by the
/// next start rather than removed at exit).
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
        let Some(path) = self.path.clone() else {
            tracing::warn!("no runtime directory: Teams cannot be reached");
            return;
        };
        if self.started.get().is_some() {
            return;
        }
        let shared = Arc::clone(&self.shared);
        let started = spawn_worker("teams-bridge", async move {
            match listen(&path) {
                Ok(listener) => accept_loop(listener, shared).await,
                Err(error) => tracing::warn!(%error, "cannot listen for the Teams extension"),
            }
        });
        // Started only once a runtime ran it: a first call outside one must
        // not disable the bridge for good.
        if started.is_some() {
            let _ = self.started.set(());
        }
    }
}

impl fmt::Debug for SocketBridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SocketBridge")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl TeamsBridge for SocketBridge {
    fn state(&self) -> watch::Receiver<TeamsState> {
        self.start();
        self.shared.state.subscribe()
    }

    fn request(&self, action: CallAction) -> Result<(), RequestError> {
        let actions = self.shared.actions().ok_or(RequestError::NotConnected)?;
        actions.try_send(action).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => RequestError::Busy,
            mpsc::error::TrySendError::Closed(_) => RequestError::NotConnected,
        })
    }
}

/// Checks that `folder` is a real directory of the user, closed to others.
fn check_private_folder(folder: &Path, uid: u32) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(folder)?;
    if metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o022 == 0 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "{} is not a folder of the user closed to others",
            folder.display()
        )))
    }
}

/// Binds the socket: its folder and the folder holding it are the user's
/// own and closed to others, a stale socket left by a previous run is
/// replaced (never one a manager still listens on), the socket itself is
/// 0600.
fn listen(path: &Path) -> std::io::Result<UnixListener> {
    use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};

    let folder = path
        .parent()
        .ok_or_else(|| std::io::Error::other("socket path without a folder"))?;
    let uid = rustix::process::getuid().as_raw();
    if let Some(runtime) = folder.parent() {
        check_private_folder(runtime, uid)?;
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)?;
    check_private_folder(folder, uid)?;
    if std::fs::symlink_metadata(folder)?.mode() & 0o077 != 0 {
        return Err(std::io::Error::other("the socket folder is not private"));
    }
    match std::fs::symlink_metadata(path) {
        Ok(existing) if existing.file_type().is_socket() && existing.uid() == uid => {
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                return Err(std::io::Error::other(
                    "another manager already listens on the socket",
                ));
            }
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
                // Transient (descriptors exhausted, connection aborted): one
                // failure must not leave the bridge deaf until a restart.
                tracing::warn!(%error, "cannot accept a Teams extension connection");
                tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
            }
        }
    }
}

/// Whether the peer of `stream` is the user running this process.
#[must_use]
pub fn is_same_user(stream: &UnixStream) -> bool {
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
    // Frames are read in a task of their own: reading half a frame must
    // never be abandoned because an action came in meanwhile.
    let (frames_in, mut frames) = mpsc::channel(INCOMING_QUEUE);
    let reading = tokio::spawn(async move {
        loop {
            match read_frame(&mut reader, MAX_FRAME_BYTES).await {
                Ok(Some(frame)) => {
                    if frames_in.send(frame).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    tracing::debug!(%error, "Teams extension stream failed");
                    break;
                }
            }
        }
    });
    loop {
        tokio::select! {
            frame = frames.recv() => {
                let Some(frame) = frame else { break };
                handle_frame(&shared, generation, &frame);
            }
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
    reading.abort();
    shared.disconnected(generation);
    tracing::info!("Teams extension disconnected");
}

/// A frame of the extension: a report, or garbage that is ignored.
fn handle_frame(shared: &Shared, generation: u64, frame: &[u8]) {
    match serde_json::from_slice::<BrowserReport>(frame) {
        Ok(report) => shared.apply_if_current(generation, report),
        // Only where the error is, never what it quotes: that is text of the
        // page.
        Err(error) => tracing::debug!(
            kind = ?error.classify(),
            line = error.line(),
            column = error.column(),
            "unreadable Teams report ignored"
        ),
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
        let (state, _) = watch::channel(TeamsState::fresh(true));
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
        self.state.send_if_modified(|state| state.apply(report));
    }

    /// The extension connects or disconnects.
    pub fn set_connected(&self, connected: bool) {
        self.state.send_replace(TeamsState::fresh(connected));
    }

    /// The actions asked so far.
    #[must_use]
    pub fn requests(&self) -> Vec<CallAction> {
        lock_unpoisoned(&self.requests).clone()
    }
}

impl Default for SimulatedBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl TeamsBridge for SimulatedBridge {
    fn state(&self) -> watch::Receiver<TeamsState> {
        self.state.subscribe()
    }

    fn request(&self, action: CallAction) -> Result<(), RequestError> {
        if !self.state.borrow().connected {
            return Err(RequestError::NotConnected);
        }
        lock_unpoisoned(&self.requests).push(action);
        Ok(())
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
        assert_eq!(
            bridge.request(CallAction::HangUp),
            Err(RequestError::NotConnected)
        );

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

        assert_eq!(bridge.request(CallAction::ToggleMute), Ok(()));
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
        assert_eq!(
            bridge.request(CallAction::HangUp),
            Err(RequestError::NotConnected)
        );
    }

    #[tokio::test]
    async fn a_newer_connection_replaces_the_older_one() {
        let dir = tempfile::tempdir().unwrap();
        let (bridge, mut state) = bridge_in(&dir).await;
        let mut first = extension(&dir).await;
        state_where(&mut state, |state| state.connected).await;
        let mut second = extension(&dir).await;
        // The bridge closes the first connection when the second takes over.
        assert!(
            timeout(WAIT, read_frame(&mut first, 64))
                .await
                .unwrap()
                .is_ok_and(|frame| frame.is_none())
        );

        // The older connection was closed by the bridge: a late report fails
        // or is ignored, never counted.
        let _ = write_frame(&mut first, br#"{"type":"unread","count":9}"#).await;
        send_report(&mut second, r#"{"type":"unread","count":1}"#).await;
        state_where(&mut state, |state| state.unread == 1).await;
        assert_eq!(state.borrow().unread, 1, "the older one no longer counts");

        assert_eq!(bridge.request(CallAction::Decline), Ok(()));
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
        assert_eq!(
            bridge.request(CallAction::HangUp),
            Err(RequestError::NotConnected)
        );
    }

    #[tokio::test]
    async fn the_simulated_bridge_keeps_what_was_asked() {
        let bridge = SimulatedBridge::demo();
        assert_eq!(bridge.state().borrow().unread, 2);
        assert_eq!(bridge.request(CallAction::HangUp), Ok(()));
        assert_eq!(bridge.requests(), [CallAction::HangUp]);
        bridge.set_connected(false);
        assert_eq!(
            bridge.request(CallAction::HangUp),
            Err(RequestError::NotConnected)
        );
        assert_eq!(bridge.requests().len(), 1);
    }

    #[tokio::test]
    async fn an_action_sent_while_a_frame_arrives_in_pieces_loses_nothing() {
        use tokio::io::AsyncWriteExt;

        let dir = tempfile::tempdir().unwrap();
        let (bridge, mut state) = bridge_in(&dir).await;
        let mut stream = extension(&dir).await;
        state_where(&mut state, |state| state.connected).await;

        let report = br#"{"type":"unread","count":6}"#;
        let (length, body) = (
            u32::try_from(report.len()).unwrap().to_ne_bytes(),
            report.as_slice(),
        );
        stream.write_all(&length).await.unwrap();
        stream.write_all(&body[..5]).await.unwrap();
        // An action goes out while the frame is half in.
        assert_eq!(bridge.request(CallAction::ToggleMute), Ok(()));
        let frame = timeout(WAIT, read_frame(&mut stream, 64))
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(frame, br#"{"type":"do","action":"toggleMute"}"#);
        stream.write_all(&body[5..]).await.unwrap();
        state_where(&mut state, |state| state.unread == 6).await;
    }

    #[tokio::test]
    async fn a_manager_already_listening_keeps_its_socket() {
        let dir = tempfile::tempdir().unwrap();
        let (_bridge, _state) = bridge_in(&dir).await;
        let path = dir.path().join("run").join("teams.sock");
        let error = listen(&path).unwrap_err();
        assert!(error.to_string().contains("already listens"), "{error}");
        // The first one still answers.
        assert!(UnixStream::connect(&path).await.is_ok());
    }

    #[tokio::test]
    async fn a_stale_socket_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run").join("teams.sock");
        drop(listen(&path).unwrap());
        // The listener is gone, its socket file stayed.
        assert!(path.exists());
        assert!(listen(&path).is_ok());
    }

    #[tokio::test]
    async fn a_runtime_folder_open_to_others_is_refused() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        let error = listen(&dir.path().join("run").join("teams.sock")).unwrap_err();
        assert!(error.to_string().contains("closed to others"), "{error}");
    }
}
