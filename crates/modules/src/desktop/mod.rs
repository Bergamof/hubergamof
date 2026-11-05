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

mod backend;
mod hub;
mod kwin;
mod launcher;
mod model;
mod monitor;
mod processes;
mod script_file;
mod simulated;
mod system;

pub use backend::{
    DesktopBackend, DesktopError, NotificationFilter, NotificationStream, WindowChanges,
    WindowFilter,
};
pub use hub::{DesktopHub, DesktopState, DesktopSubscription};
pub use model::{
    AppId, AppNotification, AppState, ClassMatch, DesktopApp, ExecutableMatch, Launch, Window,
    WindowId,
};
pub(crate) use simulated::DEMO_INTELLIJ_EXECUTABLE;
pub use simulated::SimulatedDesktop;
pub use system::SystemDesktop;
