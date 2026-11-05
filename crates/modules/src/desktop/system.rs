//! The user's desktop session: windows through KWin, processes from
//! `/proc`, notifications observed on the session bus, applications
//! started in scopes of their own.

use std::path::Path;

use async_trait::async_trait;

use super::backend::{
    DesktopBackend, DesktopError, NotificationFilter, NotificationStream, WindowChanges,
    WindowFilter,
};
use super::kwin::Kwin;
use super::launcher::Launcher;
use super::model::{ExecutableMatch, Launch, WindowId};
use super::monitor;
use super::processes::{self, PROC_ROOT};
use crate::session_bus::SessionBus;

/// The desktop session of the user. Nothing is touched before the first
/// call: the bus is connected on first use.
#[derive(Debug, Clone)]
pub struct SystemDesktop {
    bus: SessionBus,
    kwin: Kwin,
    launcher: Launcher,
}

impl SystemDesktop {
    /// The session reached through `bus` (shared with the other modules).
    #[must_use]
    pub fn new(bus: SessionBus) -> Self {
        Self {
            kwin: Kwin::new(bus.clone()),
            bus,
            launcher: Launcher::scoped(),
        }
    }
}

#[async_trait]
impl DesktopBackend for SystemDesktop {
    async fn windows(&self, filter: &WindowFilter) -> Result<Box<dyn WindowChanges>, DesktopError> {
        self.kwin.windows(filter).await
    }

    async fn activate(&self, id: &WindowId) -> Result<(), DesktopError> {
        self.kwin.activate(id).await
    }

    async fn running(&self, executables: &[ExecutableMatch]) -> Result<Vec<bool>, DesktopError> {
        let executables = executables.to_vec();
        let uid = rustix::process::getuid().as_raw();
        tokio::task::spawn_blocking(move || {
            processes::running(Path::new(PROC_ROOT), uid, &executables)
        })
        .await
        .map_err(|error| DesktopError::Processes(error.to_string()))?
        .map_err(|error| DesktopError::Processes(error.to_string()))
    }

    async fn notifications(
        &self,
        filter: &NotificationFilter,
    ) -> Result<Box<dyn NotificationStream>, DesktopError> {
        monitor::observe(&self.bus, filter).await
    }

    async fn launch(&self, launch: &Launch) -> Result<(), DesktopError> {
        self.launcher.launch(launch).await
    }
}
