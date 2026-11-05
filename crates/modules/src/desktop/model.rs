//! What the desktop layer knows of the applications it follows: their
//! description ([`DesktopApp`]), their windows, the notifications they send
//! and how to start them.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

/// Longest window title or notification text kept, in characters: enough to
/// recognise a project or a request, bounded whatever an application sends.
pub(super) const MAX_TEXT_CHARS: usize = 512;

/// Identifier of a followed application (`"claude"`, `"intellij"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppId(pub &'static str);

/// Window class of an application (`resourceClass` under KWin).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassMatch {
    /// This class only.
    Exact(String),
    /// Every class starting with this prefix (`jetbrains-` covers every
    /// JetBrains IDE and edition).
    Prefix(String),
}

impl ClassMatch {
    /// Whether `class` is matched.
    #[must_use]
    pub fn matches(&self, class: &str) -> bool {
        match self {
            Self::Exact(exact) => class == exact,
            Self::Prefix(prefix) => class.starts_with(prefix.as_str()),
        }
    }
}

/// Main process of an application, recognised by its executable
/// (`/proc/<pid>/exe`), never by its command line: other programs may
/// name the application in theirs (a `java` borrowing the runtime of
/// IntelliJ).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableMatch {
    /// End of the executable's path, compared component by component
    /// (`bin/idea`), or its whole path when absolute.
    pub path: PathBuf,
    /// Processes with an argument starting with this are helpers, not the
    /// application (`--type=` for the renderers of Electron).
    pub helper_argument: Option<String>,
}

impl ExecutableMatch {
    /// Whether `executable` is the application's (its helpers aside, see
    /// [`Self::is_helper`]).
    #[must_use]
    pub fn matches_executable(&self, executable: &Path) -> bool {
        executable.ends_with(&self.path)
    }

    /// Whether a process run with `arguments` is a helper of the
    /// application.
    pub fn is_helper<'a>(&self, mut arguments: impl Iterator<Item = &'a [u8]>) -> bool {
        self.helper_argument
            .as_ref()
            .is_some_and(|helper| arguments.any(|argument| argument.starts_with(helper.as_bytes())))
    }
}

/// An application followed on the desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopApp {
    /// Its identifier.
    pub id: AppId,
    /// Class of its windows.
    pub window_class: ClassMatch,
    /// Its main process.
    pub executable: ExecutableMatch,
    /// `desktop-entry` hint its notifications carry, if known: a
    /// notification naming another entry is not its own.
    pub desktop_entry: Option<String>,
}

impl DesktopApp {
    /// Whether a notification with the `desktop-entry` hint `entry` may be
    /// this application's: the hint alone proves nothing (any program may
    /// give any), but a hint naming another application rules it out.
    #[must_use]
    pub fn may_have_sent(&self, entry: Option<&str>) -> bool {
        match (self.desktop_entry.as_deref(), entry) {
            (Some(own), Some(entry)) => own == entry,
            _ => true,
        }
    }

    /// Whether a notification with the hint `entry`, sent by `executable`,
    /// is this application's: its sender runs the application.
    #[must_use]
    pub fn sent(&self, entry: Option<&str>, executable: &Path) -> bool {
        self.may_have_sent(entry) && self.executable.matches_executable(executable)
    }
}

/// Identifier of a window, given by the compositor.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(transparent)]
pub struct WindowId(pub String);

/// A window of a followed application.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    /// Its identifier.
    pub id: WindowId,
    /// Its class (`resourceClass`).
    pub class: String,
    /// Its title, at most [`MAX_TEXT_CHARS`] characters.
    #[serde(deserialize_with = "bounded_text")]
    pub caption: String,
    /// Its process, `0` when unknown.
    pub pid: u32,
    /// It asks for the user's attention.
    pub demands_attention: bool,
    /// It has the focus.
    pub active: bool,
    /// It is minimised.
    pub minimized: bool,
}

/// `text` cut to [`MAX_TEXT_CHARS`] characters.
pub(super) fn bounded(mut text: String) -> String {
    if let Some((cut, _)) = text.char_indices().nth(MAX_TEXT_CHARS) {
        text.truncate(cut);
    }
    text
}

fn bounded_text<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    String::deserialize(deserializer).map(bounded)
}

/// A notification of a followed application. Its texts are the sender's
/// own: shown, never interpreted.
#[derive(Clone, PartialEq, Eq)]
pub struct AppNotification {
    /// Application that sent it.
    pub app: AppId,
    /// Its summary, at most [`MAX_TEXT_CHARS`] characters.
    pub summary: String,
    /// Its body, at most [`MAX_TEXT_CHARS`] characters.
    pub body: String,
}

impl std::fmt::Debug for AppNotification {
    /// Without its texts, which may be private (what Claude asks), so that
    /// logging it never leaks them.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppNotification")
            .field("app", &self.app)
            .finish_non_exhaustive()
    }
}

/// A program to start, run directly (never through a shell).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// Program: absolute path, or name looked up in `PATH`.
    pub program: PathBuf,
    /// Its arguments, each passed separately.
    pub args: Vec<OsString>,
}

/// What the desktop layer knows of one application.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppState {
    /// Its main process is running; `None` while not known (not looked at
    /// yet, `/proc` unreadable).
    pub running: Option<bool>,
    /// Its windows, in the compositor's order; `None` when the windows
    /// cannot be followed (not under KWin, KWin unreachable).
    pub windows: Option<Arc<[Window]>>,
}

impl AppState {
    /// Whether the application runs: a window of it is open, or its
    /// process was seen.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.windows
            .as_deref()
            .is_some_and(|windows| !windows.is_empty())
            || self.running == Some(true)
    }

    /// Whether one of its windows asks for attention.
    #[must_use]
    pub fn demands_attention(&self) -> bool {
        self.windows
            .iter()
            .flat_map(|windows| windows.iter())
            .any(|window| window.demands_attention)
    }

    /// Whether it has the window `id`.
    #[must_use]
    pub fn has_window(&self, id: &WindowId) -> bool {
        self.windows
            .iter()
            .flat_map(|windows| windows.iter())
            .any(|window| window.id == *id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE_EXE: &str = "/usr/lib/claude-desktop/claude-desktop";

    fn claude() -> DesktopApp {
        DesktopApp {
            id: AppId("claude"),
            window_class: ClassMatch::Exact("com.anthropic.Claude".into()),
            executable: ExecutableMatch {
                path: CLAUDE_EXE.into(),
                helper_argument: Some("--type=".into()),
            },
            desktop_entry: Some("com.anthropic.Claude".into()),
        }
    }

    fn window(id: &str, demands_attention: bool) -> Window {
        Window {
            id: WindowId(id.into()),
            class: "c".into(),
            caption: String::new(),
            pid: 0,
            demands_attention,
            active: false,
            minimized: false,
        }
    }

    #[test]
    fn classes_match_exactly_or_by_prefix() {
        let exact = ClassMatch::Exact("com.anthropic.Claude".into());
        assert!(exact.matches("com.anthropic.Claude"));
        assert!(!exact.matches("com.anthropic.Claude2"));
        let prefix = ClassMatch::Prefix("jetbrains-".into());
        assert!(prefix.matches("jetbrains-idea") && prefix.matches("jetbrains-idea-ce"));
        assert!(!prefix.matches("org.mozilla.firefox"));
    }

    #[test]
    fn executables_match_by_their_path_components() {
        let idea = ExecutableMatch {
            path: "bin/idea".into(),
            helper_argument: None,
        };
        assert!(idea.matches_executable(Path::new(
            "/home/u/.local/share/JetBrains/Toolbox/apps/intellij-idea-ultimate/bin/idea"
        )));
        assert!(!idea.matches_executable(Path::new("/usr/bin/xidea")));
        assert!(!idea.matches_executable(Path::new(
            "/home/u/.local/share/JetBrains/Toolbox/apps/intellij-idea-ultimate/jbr/bin/java"
        )));

        let claude = claude().executable;
        assert!(claude.matches_executable(Path::new(CLAUDE_EXE)));
        assert!(!claude.matches_executable(Path::new("/opt/usr/lib/claude-desktop/x")));
    }

    #[test]
    fn helpers_are_recognised_by_their_arguments() {
        let claude = claude().executable;
        let arguments = |list: &'static [&'static str]| list.iter().map(|a| a.as_bytes());
        assert!(claude.is_helper(arguments(&["claude-desktop", "--type=zygote"])));
        assert!(!claude.is_helper(arguments(&["claude-desktop", "--startup"])));
        let idea = ExecutableMatch {
            path: "bin/idea".into(),
            helper_argument: None,
        };
        assert!(!idea.is_helper(arguments(&["idea", "--type=zygote"])));
    }

    #[test]
    fn notifications_are_attributed_by_their_sender_only() {
        let app = claude();
        let claude_exe = Path::new(CLAUDE_EXE);
        let other_exe = Path::new("/usr/bin/notify-send");
        assert!(app.sent(Some("com.anthropic.Claude"), claude_exe));
        assert!(app.sent(None, claude_exe));
        assert!(
            !app.sent(Some("com.anthropic.Claude"), other_exe),
            "a hint alone proves nothing"
        );
        assert!(!app.sent(Some("org.kde.konsole"), claude_exe));

        assert!(app.may_have_sent(None) && app.may_have_sent(Some("com.anthropic.Claude")));
        assert!(!app.may_have_sent(Some("org.kde.konsole")));
        let without_entry = DesktopApp {
            desktop_entry: None,
            ..claude()
        };
        assert!(without_entry.may_have_sent(Some("org.kde.konsole")));
    }

    #[test]
    fn texts_are_bounded_on_a_character_boundary() {
        let long = "é".repeat(MAX_TEXT_CHARS + 3);
        assert_eq!(bounded(long).chars().count(), MAX_TEXT_CHARS);
        assert_eq!(bounded("court".into()), "court");
    }

    #[test]
    fn windows_are_read_from_the_compositor_report() {
        let caption = "x".repeat(MAX_TEXT_CHARS * 2);
        let report = serde_json::json!([{
            "id": "{25b6a6ad}", "class": "com.anthropic.Claude", "caption": caption,
            "pid": 1806, "demandsAttention": true, "active": false, "minimized": false
        }]);
        let windows: Vec<Window> = serde_json::from_value(report).unwrap();
        assert_eq!(windows[0].id, WindowId("{25b6a6ad}".into()));
        assert_eq!(windows[0].caption.len(), MAX_TEXT_CHARS);
        assert!(windows[0].demands_attention);
    }

    #[test]
    fn the_state_tells_running_attention_and_windows() {
        let unknown = AppState::default();
        assert!(!unknown.is_running() && !unknown.demands_attention());
        let state = AppState {
            running: Some(false),
            windows: Some(vec![window("1", false), window("2", true)].into()),
        };
        assert!(state.is_running(), "a window is open");
        assert!(state.demands_attention());
        assert!(state.has_window(&WindowId("2".into())));
        assert!(!state.has_window(&WindowId("3".into())));
        let in_tray = AppState {
            running: Some(true),
            windows: Some(Vec::new().into()),
        };
        assert!(in_tray.is_running() && !in_tray.demands_attention());
    }

    #[test]
    fn notifications_never_show_their_texts_in_logs() {
        let notification = AppNotification {
            app: AppId("claude"),
            summary: "secret".into(),
            body: "secret".into(),
        };
        let logged = format!("{notification:?}");
        assert!(!logged.contains("secret"), "{logged}");
        assert!(logged.contains("claude"));
    }
}
