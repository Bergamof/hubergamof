//! The desktop layer: what modules following desktop applications (Claude
//! Desktop, IntelliJ) need from the session — their windows (followed and
//! activated through KWin), whether they run (`/proc`), their notifications
//! (observed on the session bus) and launching them (outside the manager's
//! service).
//!
//! The session is reached through a [`DesktopBackend`]: the user's
//! ([`SystemDesktop`]) or an in-memory one ([`SimulatedDesktop`], `--demo`
//! and tests), shared by every module through a [`DesktopHub`].
//!
//! Without KWin (GNOME…), windows are not followed: modules only know
//! whether the application runs, and can launch it.

mod attention;
mod backend;
mod followed_app;
mod hub;
mod kwin;
mod launch_gate;
mod launcher;
mod model;
mod monitor;
mod processes;
mod script_file;
mod simulated;
mod system;

pub(crate) use attention::AppAttention;
pub use backend::{
    DesktopBackend, DesktopError, NotificationFilter, NotificationStream, WindowChanges,
    WindowFilter,
};
pub(crate) use followed_app::{AppEvent, FollowedApp};
pub(crate) use hub::{DesktopAction, DesktopHub, LaunchStart};
pub use launcher::scoped_command;
pub use model::{
    AppId, AppNotification, AppState, ClassMatch, DesktopApp, ExecutableMatch, Launch,
    NotificationHints, NotificationRelay, Window, WindowId,
};
#[cfg(test)]
pub(crate) use simulated::DEMO_CLAUDE_EXECUTABLE;
pub(crate) use simulated::DEMO_INTELLIJ_EXECUTABLE;
pub use simulated::SimulatedDesktop;
pub use system::SystemDesktop;
