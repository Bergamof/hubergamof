//! What the desktop layer needs from the desktop: the [`DesktopBackend`]
//! trait, its streams and failures.

use std::fmt;
use std::path::Path;

use async_trait::async_trait;
use thiserror::Error;

use super::model::{
    AppId, AppNotification, ClassMatch, DesktopApp, ExecutableMatch, Launch, Window, WindowId,
};

/// Failure to act on the desktop.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DesktopError {
    /// The windows cannot be followed or activated: no supported compositor
    /// (KWin) on the session bus.
    #[error("no supported compositor: {0}")]
    NoCompositor(String),
    /// The session bus or a desktop service failed.
    #[error("D-Bus: {0}")]
    Bus(#[from] zbus::Error),
    /// The compositor script could not be written or loaded.
    #[error("compositor script: {0}")]
    Script(String),
    /// The processes cannot be listed.
    #[error("cannot list the processes: {0}")]
    Processes(String),
    /// A program could not be started, or failed at once.
    #[error("cannot launch `{program}`: {reason}")]
    Launch {
        /// Program started.
        program: String,
        /// Why it failed.
        reason: String,
    },
    /// The window is not, or no longer, there.
    #[error("unknown window `{}`", .0.0)]
    UnknownWindow(WindowId),
    /// The backend panicked (a bug): the call is treated as failed.
    #[error("the desktop backend panicked")]
    Panicked,
}

impl From<zbus::fdo::Error> for DesktopError {
    fn from(error: zbus::fdo::Error) -> Self {
        Self::Bus(error.into())
    }
}

/// The windows to follow: those of the followed applications.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowFilter {
    classes: Vec<ClassMatch>,
}

impl WindowFilter {
    /// The windows of `apps`.
    #[must_use]
    pub fn of(apps: &[DesktopApp]) -> Self {
        Self {
            classes: apps.iter().map(|app| app.window_class.clone()).collect(),
        }
    }

    /// The classes followed.
    #[must_use]
    pub fn classes(&self) -> &[ClassMatch] {
        &self.classes
    }

    /// Whether windows of `class` are followed.
    #[must_use]
    pub fn accepts(&self, class: &str) -> bool {
        self.classes
            .iter()
            .any(|class_match| class_match.matches(class))
    }
}

/// The notifications to observe: those of the followed applications, each
/// attributed by the executable of its sender; the others are never kept
/// nor logged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotificationFilter {
    apps: Vec<DesktopApp>,
}

impl NotificationFilter {
    /// The notifications of `apps`.
    #[must_use]
    pub fn of(apps: &[DesktopApp]) -> Self {
        Self {
            apps: apps.to_vec(),
        }
    }

    /// Whether a notification with the `desktop-entry` hint `entry` may be
    /// one of a followed application, so that its sender is worth looking
    /// up.
    #[must_use]
    pub fn may_attribute(&self, entry: Option<&str>) -> bool {
        self.apps.iter().any(|app| app.may_have_sent(entry))
    }

    /// The application that sent a notification with the hint `entry`, from
    /// `executable`.
    #[must_use]
    pub fn attribute(&self, entry: Option<&str>, executable: &Path) -> Option<AppId> {
        self.apps
            .iter()
            .find(|app| app.sent(entry, executable))
            .map(|app| app.id)
    }
}

/// The followed windows, as the compositor reports them.
#[async_trait]
pub trait WindowChanges: Send {
    /// The next list of the followed windows (the first one comes at once);
    /// `None` when the compositor stopped reporting (to follow again).
    ///
    /// Cancellation safe: dropped before it completes, it loses nothing.
    async fn next(&mut self) -> Option<Vec<Window>>;
}

/// Notifications of the followed applications.
#[async_trait]
pub trait NotificationStream: Send {
    /// The next notification; `None` when observing stopped (to observe
    /// again).
    ///
    /// Cancellation safe: dropped before it completes, it loses nothing.
    async fn next(&mut self) -> Option<AppNotification>;
}

/// The desktop session.
///
/// Calls may wait for the bus or the file system: the desktop layer only
/// makes them from its background tasks.
#[async_trait]
pub trait DesktopBackend: Send + Sync + fmt::Debug {
    /// Starts following the windows `filter` accepts. One following at a
    /// time per backend.
    async fn windows(&self, filter: &WindowFilter) -> Result<Box<dyn WindowChanges>, DesktopError>;

    /// Brings the window `id` to the foreground (restored if minimised).
    /// A window gone meanwhile is left alone, without error: callers check
    /// that it is known first.
    async fn activate(&self, id: &WindowId) -> Result<(), DesktopError>;

    /// For each of `executables`, in the same order, whether a process of
    /// the user runs it.
    async fn running(&self, executables: &[ExecutableMatch]) -> Result<Vec<bool>, DesktopError>;

    /// Starts observing the notifications `filter` attributes.
    async fn notifications(
        &self,
        filter: &NotificationFilter,
    ) -> Result<Box<dyn NotificationStream>, DesktopError>;

    /// Starts `launch`, outside the manager's service.
    async fn launch(&self, launch: &Launch) -> Result<(), DesktopError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE_EXE: &str = "/usr/lib/claude-desktop/claude-desktop";

    fn apps() -> [DesktopApp; 2] {
        [
            DesktopApp {
                id: AppId("claude"),
                window_class: ClassMatch::Exact("com.anthropic.Claude".into()),
                executable: ExecutableMatch {
                    path: CLAUDE_EXE.into(),
                    helper_argument: None,
                },
                desktop_entry: Some("com.anthropic.Claude".into()),
            },
            DesktopApp {
                id: AppId("intellij"),
                window_class: ClassMatch::Prefix("jetbrains-".into()),
                executable: ExecutableMatch {
                    path: "bin/idea".into(),
                    helper_argument: None,
                },
                desktop_entry: None,
            },
        ]
    }

    #[test]
    fn windows_are_filtered_by_class() {
        let windows = WindowFilter::of(&apps());
        assert!(windows.accepts("com.anthropic.Claude") && windows.accepts("jetbrains-idea"));
        assert!(!windows.accepts("org.mozilla.Thunderbird"));
        assert_eq!(windows.classes().len(), 2);
        assert!(!WindowFilter::default().accepts("com.anthropic.Claude"));
    }

    #[test]
    fn notifications_are_attributed_by_their_sender() {
        let filter = NotificationFilter::of(&apps());
        let claude = Path::new(CLAUDE_EXE);
        let idea = Path::new("/opt/idea/bin/idea");
        assert_eq!(
            filter.attribute(Some("com.anthropic.Claude"), claude),
            Some(AppId("claude"))
        );
        assert_eq!(filter.attribute(None, idea), Some(AppId("intellij")));
        assert_eq!(
            filter.attribute(
                Some("com.anthropic.Claude"),
                Path::new("/usr/bin/notify-send")
            ),
            None,
            "a forged hint"
        );
        // IntelliJ declares no entry: any hint may be its own.
        assert!(filter.may_attribute(Some("org.kde.konsole")));
        let claude_only = NotificationFilter::of(&apps()[..1]);
        assert!(!claude_only.may_attribute(Some("org.kde.konsole")));
        assert!(claude_only.may_attribute(None));
        assert!(!NotificationFilter::default().may_attribute(None));
    }
}
