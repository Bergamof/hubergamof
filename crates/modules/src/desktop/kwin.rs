//! Windows under KWin (KDE Plasma 6), through its scripting API: Wayland
//! gives no other process access to the windows.
//!
//! - Following: a script loaded into KWin reports the followed windows
//!   (filtered in the script, so that the titles of other windows never
//!   leave KWin) at once and at each change, by calling `Report` on an
//!   object the manager serves on the session bus. Only calls from KWin
//!   itself are accepted. Following ends when KWin restarts (its scripts
//!   are then gone). One following at a time: they share the script's
//!   plugin name.
//! - Activating: a one-shot script activates the window; KWin runs a script
//!   before `run` returns, so it is unloaded at once.
//!
//! Every value put in a script is a JSON literal: nothing reported by an
//! application can change its code. Scripts are private files
//! ([`script_file`](super::script_file)). A following ended is cleaned up
//! in the background; if the manager stops first, its script is unloaded
//! at the next start.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Serialize;
use tokio::sync::{Mutex, watch};
use zbus::fdo::{DBusProxy, NameOwnerChangedStream};
use zbus::message::Header;
use zbus::names::{BusName, OwnedUniqueName, UniqueName};
use zbus::{Connection, interface, proxy};

use super::backend::{DesktopError, WindowChanges, WindowFilter};
use super::model::{ClassMatch, Window, WindowId};
use super::script_file;
use crate::session_bus::SessionBus;

/// Well-known name of KWin on the session bus.
const KWIN_SERVICE: &str = "org.kde.KWin";

/// Plugin name of the script following the windows.
const WINDOWS_PLUGIN: &str = "bergadeck-windows";

/// Plugin name of the one-shot activation script.
const ACTIVATE_PLUGIN: &str = "bergadeck-activate";

/// Path of the objects receiving the reports, one per following.
const REPORT_PATH: &str = "/org/bergadeck/Windows";

/// Interface of those objects.
const REPORT_INTERFACE: &str = "org.bergadeck.Windows1";

/// Largest report accepted: far above a few windows, bounded whatever KWin
/// sends.
const MAX_REPORT_BYTES: usize = 256 * 1024;

/// Most windows kept from a report.
const MAX_WINDOWS: usize = 256;

#[proxy(
    interface = "org.kde.kwin.Scripting",
    default_service = "org.kde.KWin",
    default_path = "/Scripting"
)]
trait Scripting {
    /// Loads the script `file_path` as `plugin_name`; its id, `-1` when that
    /// plugin is already loaded. (KWin's members are in lower camel case,
    /// not in the Pascal case zbus derives.)
    #[zbus(name = "loadScript")]
    fn load_script(&self, file_path: &str, plugin_name: &str) -> zbus::Result<i32>;

    /// Unloads the plugin `plugin_name`; whether it was loaded.
    #[zbus(name = "unloadScript")]
    fn unload_script(&self, plugin_name: &str) -> zbus::Result<bool>;
}

#[proxy(interface = "org.kde.kwin.Script", default_service = "org.kde.KWin")]
trait Script {
    /// Runs the script.
    #[zbus(name = "run")]
    fn run(&self) -> zbus::Result<()>;
}

/// Which following owns the windows script.
#[derive(Debug, Default)]
struct FollowingSlot {
    /// Number of the last following started.
    generation: u64,
    /// Whether it still runs.
    active: bool,
}

impl FollowingSlot {
    /// Takes the slot for a new following, numbered.
    fn start(&mut self) -> Result<u64, DesktopError> {
        if self.active {
            // Also while the previous following is still being cleaned up
            // (a new one right after the last subscriber left): the hub
            // tries again after its backoff.
            return Err(DesktopError::Script(
                "the windows are already followed, or the previous following is closing".into(),
            ));
        }
        self.generation += 1;
        self.active = true;
        Ok(self.generation)
    }

    /// The following `generation` ended: whether it held the slot (its
    /// script is then to be unloaded), rather than one started since.
    fn end(&mut self, generation: u64) -> bool {
        let current = self.active && self.generation == generation;
        if current {
            self.active = false;
        }
        current
    }
}

/// The windows of the session under KWin.
#[derive(Debug, Clone)]
pub(super) struct Kwin {
    bus: SessionBus,
    following: Arc<Mutex<FollowingSlot>>,
    /// One activation at a time (they share their plugin name).
    activation: Arc<Mutex<()>>,
}

impl Kwin {
    pub(super) fn new(bus: SessionBus) -> Self {
        Self {
            bus,
            following: Arc::default(),
            activation: Arc::default(),
        }
    }

    /// Starts following the windows `filter` accepts.
    pub(super) async fn windows(
        &self,
        filter: &WindowFilter,
    ) -> Result<Box<dyn WindowChanges>, DesktopError> {
        let connection = self.bus.connection().await?.clone();
        let dbus = DBusProxy::new(&connection).await?;
        // Listened to before KWin is looked up: a restart in between ends
        // the following instead of leaving it waiting for an old KWin.
        let restarts = dbus
            .receive_name_owner_changed_with_args(&[(0, KWIN_SERVICE)])
            .await?;
        let kwin = compositor(&dbus).await?;

        let mut slot = self.following.lock().await;
        let guard = FollowingGuard {
            connection: connection.clone(),
            path: next_report_path(),
            generation: slot.start()?,
            slot: Arc::clone(&self.following),
        };
        let received = serve_reports(&connection, &guard.path, kwin.clone()).await?;
        load_windows_script(&connection, &guard.path, filter).await?;
        if compositor(&dbus).await? != kwin {
            return Err(DesktopError::NoCompositor("KWin restarted".into()));
        }
        drop(slot);
        Ok(Box::new(KwinWindows {
            received,
            restarts,
            _guard: guard,
        }))
    }

    /// Activates the window `id`.
    pub(super) async fn activate(&self, id: &WindowId) -> Result<(), DesktopError> {
        let connection = self.bus.connection().await?;
        let _one_at_a_time = self.activation.lock().await;
        let file = write_script("activate.js", activation_script(id)).await?;
        reload(connection, &file, ACTIVATE_PLUGIN).await?;
        // The script ran before `run` returned.
        ScriptingProxy::new(connection)
            .await?
            .unload_script(ACTIVATE_PLUGIN)
            .await?;
        Ok(())
    }
}

/// The unique name of KWin.
async fn compositor(dbus: &DBusProxy<'_>) -> Result<OwnedUniqueName, DesktopError> {
    dbus.get_name_owner(BusName::try_from(KWIN_SERVICE)?)
        .await
        .map_err(|error| DesktopError::NoCompositor(error.to_string()))
}

/// Serves at `path` the object receiving the reports sent by `compositor`.
async fn serve_reports(
    connection: &Connection,
    path: &str,
    compositor: OwnedUniqueName,
) -> Result<watch::Receiver<Option<Vec<Window>>>, DesktopError> {
    let (reports, received) = watch::channel(None);
    connection
        .object_server()
        .at(
            path,
            Reporter {
                compositor,
                reports,
            },
        )
        .await?;
    Ok(received)
}

/// Loads the following script, reporting to `path` of this connection.
async fn load_windows_script(
    connection: &Connection,
    path: &str,
    filter: &WindowFilter,
) -> Result<(), DesktopError> {
    let Some(own_name) = connection.unique_name() else {
        return Err(DesktopError::NoCompositor("not on a message bus".into()));
    };
    let script = windows_script(own_name.as_str(), path, filter);
    let file = write_script("windows.js", script).await?;
    reload(connection, &file, WINDOWS_PLUGIN).await
}

/// A path for a new report object, never used before by this manager.
fn next_report_path() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!("{REPORT_PATH}/{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// Writes `script` as `name` among the private scripts.
async fn write_script(name: &'static str, script: String) -> Result<PathBuf, DesktopError> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| DesktopError::Script("XDG_RUNTIME_DIR is not set".into()))?;
    tokio::task::spawn_blocking(move || script_file::write(Path::new(&runtime_dir), name, &script))
        .await
        .map_err(|error| DesktopError::Script(error.to_string()))?
        .map_err(DesktopError::Script)
}

/// Unloads `plugin` (left by an earlier run of the manager, or an earlier
/// following), then loads and runs the script `file` as `plugin`.
async fn reload(connection: &Connection, file: &Path, plugin: &str) -> Result<(), DesktopError> {
    let scripting = ScriptingProxy::new(connection).await?;
    scripting.unload_script(plugin).await?;
    let path = file.to_str().ok_or_else(|| {
        DesktopError::Script(format!("script path not in UTF-8: {}", file.display()))
    })?;
    let id = scripting.load_script(path, plugin).await?;
    if id < 0 {
        return Err(DesktopError::Script(format!("KWin refused {plugin}")));
    }
    ScriptProxy::builder(connection)
        .path(format!("/Scripting/Script{id}"))
        .build()
        .await?
        .run()
        .await?;
    Ok(())
}

/// Receives the reports of the following script.
struct Reporter {
    /// Unique name of KWin: the only sender accepted.
    compositor: OwnedUniqueName,
    reports: watch::Sender<Option<Vec<Window>>>,
}

impl Reporter {
    /// Takes `report` if `sender` is KWin.
    fn receive(&self, sender: Option<&UniqueName<'_>>, report: &str) -> zbus::fdo::Result<()> {
        if sender != Some(&self.compositor.as_ref()) {
            return Err(zbus::fdo::Error::AccessDenied("only KWin reports".into()));
        }
        let windows = parse_report(report).map_err(zbus::fdo::Error::InvalidArgs)?;
        self.reports.send_replace(Some(windows));
        Ok(())
    }
}

#[interface(name = "org.bergadeck.Windows1")]
impl Reporter {
    /// The followed windows, as a JSON array.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "zbus passes the header by value"
    )]
    fn report(&self, #[zbus(header)] header: Header<'_>, windows: &str) -> zbus::fdo::Result<()> {
        self.receive(header.sender(), windows)
    }
}

/// The windows of `report`, at most [`MAX_WINDOWS`].
fn parse_report(report: &str) -> Result<Vec<Window>, String> {
    if report.len() > MAX_REPORT_BYTES {
        return Err(format!("report over {MAX_REPORT_BYTES} bytes"));
    }
    let mut windows: Vec<Window> =
        serde_json::from_str(report).map_err(|error| format!("unreadable report: {error}"))?;
    windows.truncate(MAX_WINDOWS);
    Ok(windows)
}

/// A following under way; dropping it removes its report object and, if
/// it still holds the slot, unloads the script (in the background: drop
/// cannot wait).
struct FollowingGuard {
    connection: Connection,
    path: String,
    generation: u64,
    slot: Arc<Mutex<FollowingSlot>>,
}

impl Drop for FollowingGuard {
    fn drop(&mut self) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let (connection, path) = (self.connection.clone(), self.path.clone());
        let (generation, slot) = (self.generation, Arc::clone(&self.slot));
        runtime.spawn(async move {
            let mut slot = slot.lock().await;
            if let Err(error) = connection
                .object_server()
                .remove::<Reporter, _>(path.as_str())
                .await
            {
                tracing::debug!(%error, "cannot remove the window report object");
            }
            if slot.end(generation) {
                let unloaded = async {
                    ScriptingProxy::new(&connection)
                        .await?
                        .unload_script(WINDOWS_PLUGIN)
                        .await
                };
                if let Err(error) = unloaded.await {
                    tracing::debug!(%error, "cannot unload the KWin script");
                }
            }
        });
    }
}

/// The windows followed through KWin.
struct KwinWindows {
    received: watch::Receiver<Option<Vec<Window>>>,
    restarts: NameOwnerChangedStream,
    _guard: FollowingGuard,
}

#[async_trait]
impl WindowChanges for KwinWindows {
    async fn next(&mut self) -> Option<Vec<Window>> {
        // Both waits are cancellation safe.
        loop {
            tokio::select! {
                changed = self.received.changed() => {
                    changed.ok()?;
                    if let Some(windows) = self.received.borrow_and_update().clone() {
                        return Some(windows);
                    }
                }
                // KWin restarted or left: its scripts are gone.
                _ = self.restarts.next() => return None,
            }
        }
    }
}

/// The classes a script follows: exact ones and prefixes.
#[derive(Serialize)]
struct ScriptFilter<'a> {
    classes: Vec<&'a str>,
    prefixes: Vec<&'a str>,
}

impl<'a> ScriptFilter<'a> {
    fn of(filter: &'a WindowFilter) -> Self {
        let mut classes = Vec::new();
        let mut prefixes = Vec::new();
        for class_match in filter.classes() {
            match class_match {
                ClassMatch::Exact(class) => classes.push(class.as_str()),
                ClassMatch::Prefix(prefix) => prefixes.push(prefix.as_str()),
            }
        }
        Self { classes, prefixes }
    }
}

/// `value` as a JavaScript literal: JSON, with the two line separators JSON
/// allows in strings escaped (JavaScript engines before ES2019 end a line
/// there).
fn literal(value: &impl Serialize) -> String {
    // Strings, sequences and plain structures always serialise.
    serde_json::to_string(value)
        .unwrap_or_else(|_| "null".into())
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// The following script: reports the windows `filter` accepts to the
/// object `path` of the connection `bus_name`, at once and when one of
/// them is added, removed, renamed, minimised or restored, asks for
/// attention or takes or loses the focus.
fn windows_script(bus_name: &str, path: &str, filter: &WindowFilter) -> String {
    let filter = ScriptFilter::of(filter);
    let constants = [
        ("BUS", literal(&bus_name)),
        ("PATH", literal(&path)),
        ("INTERFACE", literal(&REPORT_INTERFACE)),
        ("CLASSES", literal(&filter.classes)),
        ("PREFIXES", literal(&filter.prefixes)),
    ];
    with_constants(&constants, include_str!("kwin_windows.js"))
}

/// The activation script of the window `id`.
fn activation_script(id: &WindowId) -> String {
    with_constants(&[("ID", literal(&id.0))], include_str!("kwin_activate.js"))
}

/// `body` preceded by the declaration of `constants`.
fn with_constants(constants: &[(&str, String)], body: &str) -> String {
    let declarations = constants
        .iter()
        .flat_map(|(name, value)| ["const ", name, " = ", value, ";\n"]);
    declarations.chain([body]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter() -> WindowFilter {
        use crate::desktop::model::{AppId, DesktopApp, ExecutableMatch};
        let app = |id, window_class| DesktopApp {
            id: AppId(id),
            window_class,
            executable: ExecutableMatch {
                path: "x".into(),
                helper_argument: None,
            },
            desktop_entry: None,
            relays: Vec::new(),
        };
        WindowFilter::of(&[
            app("claude", ClassMatch::Exact("com.anthropic.Claude".into())),
            app("intellij", ClassMatch::Prefix("jetbrains-".into())),
        ])
    }

    fn reporter() -> (Reporter, watch::Receiver<Option<Vec<Window>>>) {
        let (reports, received) = watch::channel(None);
        let compositor = OwnedUniqueName::try_from(":1.10").unwrap();
        (
            Reporter {
                compositor,
                reports,
            },
            received,
        )
    }

    const REPORT: &str = r#"[{"id":"{1}","class":"com.anthropic.Claude","caption":"Claude",
        "pid":1806,"demandsAttention":false,"active":true,"minimized":false}]"#;

    #[test]
    fn only_kwin_may_report() {
        let (reporter, received) = reporter();
        let kwin = UniqueName::try_from(":1.10").unwrap();
        let other = UniqueName::try_from(":1.99").unwrap();
        assert!(matches!(
            reporter.receive(Some(&other), REPORT),
            Err(zbus::fdo::Error::AccessDenied(_))
        ));
        assert!(reporter.receive(None, REPORT).is_err());
        assert!(received.borrow().is_none());

        reporter.receive(Some(&kwin), REPORT).unwrap();
        let windows = received.borrow().clone().unwrap();
        assert_eq!(windows[0].id, WindowId("{1}".into()));
        assert!(windows[0].active);
    }

    #[test]
    fn reports_are_bounded() {
        assert!(parse_report("not json").is_err());
        let huge = format!("[{}]", " ".repeat(MAX_REPORT_BYTES));
        assert!(parse_report(&huge).unwrap_err().contains("bytes"));
        let one = REPORT.trim_start_matches('[').trim_end_matches(']');
        let many = format!("[{}]", vec![one; MAX_WINDOWS + 5].join(","));
        assert_eq!(parse_report(&many).unwrap().len(), MAX_WINDOWS);
    }

    #[test]
    fn scripts_get_their_values_as_literals() {
        let script = windows_script(":1.42", "/org/bergadeck/Windows/0", &filter());
        assert!(script.contains("const BUS = \":1.42\";\n"), "{script}");
        assert!(script.contains(r#"const CLASSES = ["com.anthropic.Claude"];"#));
        assert!(script.contains(r#"const PREFIXES = ["jetbrains-"];"#));
        assert!(script.contains(&format!("const INTERFACE = \"{REPORT_INTERFACE}\";")));
        assert!(script.ends_with(include_str!("kwin_windows.js")));

        let hostile = WindowId("\"); workspace.activeWindow = null; (\"".into());
        let activation = activation_script(&hostile);
        assert!(
            activation.contains(r#"const ID = "\"); workspace.activeWindow = null; (\"";"#),
            "{activation}"
        );
    }

    #[test]
    fn line_separators_are_escaped() {
        let id = WindowId("a\u{2028}b\u{2029}c".into());
        let activation = activation_script(&id);
        assert!(
            activation.contains("const ID = \"a\\u2028b\\u2029c\";"),
            "{activation}"
        );
        assert!(!activation.contains('\u{2028}') && !activation.contains('\u{2029}'));
    }

    #[test]
    fn one_following_at_a_time_and_only_the_current_one_unloads() {
        let mut slot = FollowingSlot::default();
        let first = slot.start().unwrap();
        assert!(matches!(slot.start(), Err(DesktopError::Script(_))));
        assert!(slot.end(first), "the first held the slot");
        assert!(!slot.end(first), "already ended");

        let second = slot.start().unwrap();
        assert_ne!(first, second);
        assert!(!slot.end(first), "a late cleanup leaves the second alone");
        assert!(slot.end(second));
    }

    #[test]
    fn report_paths_are_never_reused() {
        assert_ne!(next_report_path(), next_report_path());
        assert!(next_report_path().starts_with(REPORT_PATH));
    }
}
