//! An in-memory desktop (`--demo` and tests): windows, processes and
//! notifications set by hand, activations and launches only recorded. It
//! keeps the contract of the real one: notifications are attributed by
//! their sender, and activating a window gone does nothing.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use tokio::sync::{broadcast, watch};

use super::backend::{
    DesktopBackend, DesktopError, NotificationFilter, NotificationStream, WindowChanges,
    WindowFilter,
};
use super::model::{AppNotification, ExecutableMatch, Launch, NotificationHints, Window, WindowId};

/// Notifications a slow observer may fall behind by before missing some.
const NOTIFICATION_CAPACITY: usize = 16;

/// Class of the Claude Desktop window of the demo.
pub(crate) const DEMO_CLAUDE_CLASS: &str = "com.anthropic.Claude";

/// Executable of Claude Desktop in the demo.
pub(crate) const DEMO_CLAUDE_EXECUTABLE: &str = "/usr/lib/claude-desktop/claude-desktop";

/// Class of the IntelliJ window of the demo.
pub(crate) const DEMO_INTELLIJ_CLASS: &str = "jetbrains-idea";

/// Executable of IntelliJ in the demo.
pub(crate) const DEMO_INTELLIJ_EXECUTABLE: &str = "/opt/intellij-idea/bin/idea";

/// A notification sent on the simulated desktop.
#[derive(Debug, Clone)]
struct Sent {
    executable: PathBuf,
    hints: NotificationHints,
}

/// The simulated desktop.
#[derive(Debug)]
pub struct SimulatedDesktop {
    windows: watch::Sender<Vec<Window>>,
    running: Mutex<Vec<PathBuf>>,
    notifications: broadcast::Sender<Sent>,
    activations: Mutex<Vec<WindowId>>,
    launches: Mutex<Vec<Launch>>,
    /// Activations and launches fail (recorded all the same).
    failing: AtomicBool,
    /// Windows can be followed (KWin); otherwise following them fails.
    compositor: AtomicBool,
}

impl SimulatedDesktop {
    /// A desktop without any window nor application.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            windows: watch::Sender::new(Vec::new()),
            running: Mutex::default(),
            notifications: broadcast::Sender::new(NOTIFICATION_CAPACITY),
            activations: Mutex::default(),
            launches: Mutex::default(),
            failing: AtomicBool::new(false),
            compositor: AtomicBool::new(true),
        }
    }

    /// The demo desktop: Claude Desktop and IntelliJ running, each with a
    /// window.
    #[must_use]
    pub fn demo() -> Self {
        let desktop = Self::empty();
        desktop.set_running(vec![
            DEMO_CLAUDE_EXECUTABLE.into(),
            DEMO_INTELLIJ_EXECUTABLE.into(),
        ]);
        desktop.set_windows(vec![
            Window::new("claude", DEMO_CLAUDE_CLASS, "Claude"),
            Window::new("intellij", DEMO_INTELLIJ_CLASS, "bergadeck – main.rs"),
        ]);
        desktop
    }

    /// Replaces the windows.
    pub fn set_windows(&self, windows: Vec<Window>) {
        self.windows.send_replace(windows);
    }

    /// The windows.
    #[must_use]
    pub fn windows(&self) -> Vec<Window> {
        self.windows.borrow().clone()
    }

    /// Replaces the executables of the running processes.
    pub fn set_running(&self, executables: Vec<PathBuf>) {
        *lock(&self.running) = executables;
    }

    /// `executable` sends a notification with the `desktop-entry` hint
    /// `entry`.
    pub fn notify(&self, executable: &Path, entry: Option<&str>) {
        self.notify_with(executable, NotificationHints::entry(entry));
    }

    /// `executable` sends a notification with `hints`.
    pub fn notify_with(&self, executable: &Path, hints: NotificationHints) {
        // No observer: nobody to tell.
        let _receivers = self.notifications.send(Sent {
            executable: executable.to_owned(),
            hints,
        });
    }

    /// Whether following the windows works from now on (without, as under
    /// GNOME, it fails).
    pub fn set_compositor(&self, present: bool) {
        self.compositor.store(present, Ordering::SeqCst);
    }

    /// Makes the next activations and launches fail, or succeed again.
    pub fn fail_actions(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }

    /// The outcome of an action of `program`.
    fn outcome(&self, program: &str) -> Result<(), DesktopError> {
        if self.failing.load(Ordering::SeqCst) {
            Err(DesktopError::Launch {
                program: program.into(),
                reason: "simulated failure".into(),
            })
        } else {
            Ok(())
        }
    }

    /// The windows activated so far, oldest first.
    #[must_use]
    pub fn activations(&self) -> Vec<WindowId> {
        lock(&self.activations).clone()
    }

    /// The programs launched so far, oldest first.
    #[must_use]
    pub fn launches(&self) -> Vec<Launch> {
        lock(&self.launches).clone()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every update leaves the data consistent: a poisoned lock is valid.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[async_trait]
impl DesktopBackend for SimulatedDesktop {
    async fn windows(&self, filter: &WindowFilter) -> Result<Box<dyn WindowChanges>, DesktopError> {
        if !self.compositor.load(Ordering::SeqCst) {
            return Err(DesktopError::NoCompositor("simulated".into()));
        }
        Ok(Box::new(SimulatedWindows {
            windows: self.windows.subscribe(),
            filter: filter.clone(),
            first: true,
        }))
    }

    async fn activate(&self, id: &WindowId) -> Result<(), DesktopError> {
        self.windows.send_if_modified(|windows| {
            if !windows.iter().any(|window| window.id == *id) {
                return false;
            }
            for window in windows.iter_mut() {
                window.active = window.id == *id;
                if window.active {
                    window.minimized = false;
                    window.demands_attention = false;
                }
            }
            true
        });
        lock(&self.activations).push(id.clone());
        self.outcome("kwin")
    }

    async fn running(&self, executables: &[ExecutableMatch]) -> Result<Vec<bool>, DesktopError> {
        let running = lock(&self.running);
        Ok(executables
            .iter()
            .map(|matcher| {
                running
                    .iter()
                    .any(|executable| matcher.matches_executable(executable))
            })
            .collect())
    }

    async fn notifications(
        &self,
        filter: &NotificationFilter,
    ) -> Result<Box<dyn NotificationStream>, DesktopError> {
        Ok(Box::new(SimulatedNotifications {
            received: self.notifications.subscribe(),
            filter: filter.clone(),
        }))
    }

    async fn launch(&self, launch: &Launch) -> Result<(), DesktopError> {
        tracing::info!(program = %launch.program.display(), "simulated launch");
        lock(&self.launches).push(launch.clone());
        self.outcome(&launch.program.to_string_lossy())
    }
}

struct SimulatedWindows {
    windows: watch::Receiver<Vec<Window>>,
    filter: WindowFilter,
    first: bool,
}

#[async_trait]
impl WindowChanges for SimulatedWindows {
    async fn next(&mut self) -> Option<Vec<Window>> {
        if self.first {
            self.first = false;
        } else {
            self.windows.changed().await.ok()?;
        }
        let windows = self.windows.borrow_and_update();
        Some(
            windows
                .iter()
                .filter(|window| self.filter.accepts(&window.class))
                .cloned()
                .collect(),
        )
    }
}

struct SimulatedNotifications {
    received: broadcast::Receiver<Sent>,
    filter: NotificationFilter,
}

#[async_trait]
impl NotificationStream for SimulatedNotifications {
    async fn next(&mut self) -> Option<AppNotification> {
        loop {
            match self.received.recv().await {
                Ok(sent) => {
                    if let Some(app) = self.filter.attribute(&sent.hints, &sent.executable) {
                        return Some(AppNotification { app });
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}
