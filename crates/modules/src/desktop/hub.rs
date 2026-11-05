//! The desktop hub: one per manager, shared by every module and device
//! following desktop applications, so that a single compositor script, a
//! single notification monitor and a single look at the processes serve
//! them all.
//!
//! A worker task, started with the first [`DesktopSubscription`] and
//! stopped with the last one:
//!
//! - follows the windows of the applications, again after a bounded
//!   backoff when that stops (KWin restarted) or fails (no KWin);
//! - observes their notifications, likewise, once a module listens to
//!   them (the monitor sees the whole session: not without a reason);
//! - looks at their processes every [`PROCESS_PROBE_INTERVAL`], less often
//!   ([`IDLE_PROBE_INTERVAL`]) while nothing changes, again often as soon as
//!   their windows change (an application started or quit);
//! - publishes what it knows of each application
//!   ([`DesktopSubscription::states`]) and hands their notifications on
//!   ([`DesktopSubscription::notifications`]).
//!
//! Actions (activating a window, launching a program) run each in a task
//! of its own, outliving the instance that asked; one launch of each
//! application at a time, whatever the instance or device asking.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::{broadcast, watch};
use tokio::task::AbortHandle;
use tokio::time::Instant;

use super::backend::{
    DesktopBackend, DesktopError, NotificationFilter, NotificationStream, WindowChanges,
    WindowFilter,
};
use super::launch_gate::LaunchGate;
use super::model::{
    AppId, AppNotification, AppState, DesktopApp, ExecutableMatch, Launch, Window, WindowId,
};
use crate::background::{
    Backoff, Listener, WorkerControl, catch_panic, sleep_until, spawn_action, spawn_worker,
};

/// Time between two looks at the processes.
const PROCESS_PROBE_INTERVAL: Duration = Duration::from_secs(2);

/// Time between two looks at the processes once [`PROBES_BEFORE_IDLE`] in a
/// row found nothing new: an idle session costs a walk of `/proc` less
/// often. Without KWin (no window to tell a change), what the key shows
/// lags by this much at most.
const IDLE_PROBE_INTERVAL: Duration = Duration::from_secs(6);

/// Looks at the processes finding nothing new before slowing down.
const PROBES_BEFORE_IDLE: u32 = 5;

/// First wait before following the windows, or observing the
/// notifications, again.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Longest wait between two attempts (a session without KWin is retried at
/// this pace).
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// A following that lasted this long was healthy: the next wait starts over.
const STABLE_FOLLOWING: Duration = Duration::from_secs(10);

/// Notifications a slow subscriber may fall behind by before missing some.
const NOTIFICATION_CAPACITY: usize = 16;

/// What the hub knows of every followed application.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopState {
    apps: BTreeMap<AppId, AppState>,
}

impl DesktopState {
    /// What is known of `app` (nothing for an application not followed).
    #[must_use]
    pub fn app(&self, app: AppId) -> AppState {
        self.apps.get(&app).cloned().unwrap_or_default()
    }

    /// Whether one of the applications has the window `id`.
    #[must_use]
    pub fn has_window(&self, id: &WindowId) -> bool {
        self.apps.values().any(|state| state.has_window(id))
    }
}

/// An action a module asks of the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopAction {
    /// Bringing a window to the front.
    Activate,
    /// Starting an application.
    Launch,
}

/// How a launch request went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchStart {
    /// Started: its outcome will be reported.
    Started,
    /// Another launch of the application is under way: nothing done.
    Busy,
    /// Not started (application not followed, no Tokio runtime): its
    /// outcome will never be reported.
    Refused,
}

/// Shared access to the desktop, see the [module documentation](self).
#[derive(Debug)]
pub struct DesktopHub {
    backend: Arc<dyn DesktopBackend>,
    apps: Arc<[DesktopApp]>,
    /// One launch at a time, per application.
    launches: BTreeMap<AppId, LaunchGate>,
    states: watch::Sender<DesktopState>,
    notifications: broadcast::Sender<AppNotification>,
    /// Whether a module listens to the notifications: the monitor only
    /// runs from then on.
    observing: watch::Sender<bool>,
    control: Mutex<WorkerControl>,
}

impl DesktopHub {
    /// The hub following `apps` on `backend`. One hub per backend: a
    /// backend follows the windows for one hub at a time.
    ///
    /// # Panics
    ///
    /// In debug builds, when two applications share an identifier (their
    /// states would mix).
    #[must_use]
    pub fn new(backend: Arc<dyn DesktopBackend>, apps: Vec<DesktopApp>) -> Arc<Self> {
        debug_assert!(
            apps.iter()
                .enumerate()
                .all(|(index, app)| apps[..index].iter().all(|other| other.id != app.id)),
            "application identifiers must be unique"
        );
        Arc::new(Self {
            backend,
            launches: apps
                .iter()
                .map(|app| (app.id, LaunchGate::default()))
                .collect(),
            apps: apps.into(),
            states: watch::Sender::new(DesktopState::default()),
            notifications: broadcast::Sender::new(NOTIFICATION_CAPACITY),
            observing: watch::Sender::new(false),
            control: Mutex::new(WorkerControl::default()),
        })
    }

    /// Whether the hub follows `app`.
    #[must_use]
    pub fn follows(&self, app: AppId) -> bool {
        self.launches.contains_key(&app)
    }

    /// A subscription; the first one starts the worker (on the current
    /// Tokio runtime, if any), as does one finding it stopped.
    #[must_use]
    pub fn subscribe(self: &Arc<Self>) -> DesktopSubscription {
        self.lock().subscribe(|| self.start_worker());
        DesktopSubscription {
            hub: Arc::clone(self),
        }
    }

    fn start_worker(&self) -> Option<AbortHandle> {
        let worker = Worker::new(
            Arc::clone(&self.backend),
            Arc::clone(&self.apps),
            self.states.clone(),
            self.notifications.clone(),
            self.observing.subscribe(),
        );
        spawn_worker("desktop", worker.run())
    }

    fn lock(&self) -> MutexGuard<'_, WorkerControl> {
        // Every update keeps the control consistent: a poisoned lock is
        // still valid.
        self.control.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A module's access to the hub; dropping it unsubscribes.
#[derive(Debug)]
pub struct DesktopSubscription {
    hub: Arc<DesktopHub>,
}

impl DesktopSubscription {
    /// What is known of the applications, the current state first.
    #[must_use]
    pub fn states(&self) -> watch::Receiver<DesktopState> {
        self.hub.states.subscribe()
    }

    /// The notifications of the applications, from now on; the first call
    /// starts observing them.
    #[must_use]
    pub fn notifications(&self) -> broadcast::Receiver<AppNotification> {
        self.hub
            .observing
            .send_if_modified(|observing| !std::mem::replace(observing, true));
        self.hub.notifications.subscribe()
    }

    /// Activates the window `id` in the background, then hands the outcome
    /// to `report` ([`DesktopError::UnknownWindow`] for a window not in the
    /// published state, including one so new it was not reported yet);
    /// `false` without a Tokio runtime.
    pub fn activate(
        &self,
        id: WindowId,
        report: impl FnOnce(Result<(), DesktopError>) + Send + 'static,
    ) -> bool {
        let known = self.hub.states.borrow().has_window(&id);
        let backend = Arc::clone(&self.hub.backend);
        let activation = async move {
            if known {
                backend.activate(&id).await
            } else {
                Err(DesktopError::UnknownWindow(id))
            }
        };
        spawn_action(activation, Err(DesktopError::Panicked), report)
    }

    /// Starts `launch` of `app` in the background, unless another launch
    /// of it is under way, then hands the outcome to `report` (dropped
    /// unused unless [`LaunchStart::Started`]).
    pub fn launch(
        &self,
        app: AppId,
        launch: Launch,
        report: impl FnOnce(Result<(), DesktopError>) + Send + 'static,
    ) -> LaunchStart {
        let Some(gate) = self.hub.launches.get(&app) else {
            return LaunchStart::Refused;
        };
        let Some(pass) = gate.enter() else {
            return LaunchStart::Busy;
        };
        let backend = Arc::clone(&self.hub.backend);
        let launching = async move {
            let outcome = backend.launch(&launch).await;
            drop(pass);
            outcome
        };
        if spawn_action(launching, Err(DesktopError::Panicked), report) {
            LaunchStart::Started
        } else {
            LaunchStart::Refused
        }
    }
}

impl Drop for DesktopSubscription {
    fn drop(&mut self) {
        // The worker only reads: stopping it mid-call loses nothing.
        self.hub.lock().unsubscribe();
    }
}

/// Runs a backend call, a panic counting as a failure
/// ([`DesktopError::Panicked`]).
async fn guarded<T>(
    call: impl Future<Output = Result<T, DesktopError>> + Unpin,
) -> Result<T, DesktopError> {
    catch_panic(call)
        .await
        .unwrap_or(Err(DesktopError::Panicked))
}

struct Worker {
    backend: Arc<dyn DesktopBackend>,
    apps: Arc<[DesktopApp]>,
    states: watch::Sender<DesktopState>,
    notifications: broadcast::Sender<AppNotification>,
    window_filter: WindowFilter,
    notification_filter: NotificationFilter,
    executables: Vec<ExecutableMatch>,
    windows: Listener<Box<dyn WindowChanges>>,
    observer: Listener<Box<dyn NotificationStream>>,
    /// Whether to observe the notifications (see [`DesktopHub`]).
    wanted: watch::Receiver<bool>,
    probe_due: Instant,
    /// Looks at the processes in a row that changed nothing.
    quiet_probes: u32,
    /// Whether the last attempt of each kind succeeded, to log only
    /// transitions.
    following: bool,
    observing: bool,
    probing: bool,
}

impl Worker {
    fn new(
        backend: Arc<dyn DesktopBackend>,
        apps: Arc<[DesktopApp]>,
        states: watch::Sender<DesktopState>,
        notifications: broadcast::Sender<AppNotification>,
        wanted: watch::Receiver<bool>,
    ) -> Self {
        Self {
            window_filter: WindowFilter::of(&apps),
            notification_filter: NotificationFilter::of(&apps),
            executables: apps.iter().map(|app| app.executable.clone()).collect(),
            backend,
            apps,
            states,
            notifications,
            windows: listener(),
            observer: listener(),
            wanted,
            probe_due: Instant::now(),
            quiet_probes: 0,
            following: true,
            observing: true,
            probing: true,
        }
    }

    /// Serves until aborted.
    async fn run(mut self) {
        loop {
            let follow_due = self.windows.due();
            let observe_due = self.observer.due().filter(|_| *self.wanted.borrow());
            tokio::select! {
                // Wanted from now on: the next turn starts observing.
                Ok(()) = self.wanted.changed() => {}
                () = sleep_until(follow_due) => self.follow().await,
                () = sleep_until(observe_due) => self.observe().await,
                () = tokio::time::sleep_until(self.probe_due) => self.probe().await,
                windows = next_windows(self.windows.changes()) => {
                    if windows.is_none() {
                        self.windows.ended();
                    }
                    self.publish_windows(windows.as_deref());
                }
                notification = next_notification(self.observer.changes()) => {
                    match notification {
                        Some(notification) => self.forward(notification),
                        None => self.observer.ended(),
                    }
                }
            }
        }
    }

    async fn follow(&mut self) {
        let filter = self.window_filter.clone();
        let followed = guarded(self.backend.windows(&filter)).await;
        log_transition(&mut self.following, &followed, "follow the windows");
        self.windows.listened(followed.ok());
    }

    async fn observe(&mut self) {
        let filter = self.notification_filter.clone();
        let observed = guarded(self.backend.notifications(&filter)).await;
        log_transition(&mut self.observing, &observed, "observe the notifications");
        self.observer.listened(observed.ok());
    }

    async fn probe(&mut self) {
        let running = guarded(self.backend.running(&self.executables)).await;
        log_transition(&mut self.probing, &running, "look at the processes");
        let running = running.ok();
        let changed = self.update(|index, state| {
            state.running = running.as_ref().and_then(|found| found.get(index).copied());
        });
        self.quiet_probes = if changed {
            0
        } else {
            self.quiet_probes.saturating_add(1)
        };
        let interval = if self.quiet_probes >= PROBES_BEFORE_IDLE {
            IDLE_PROBE_INTERVAL
        } else {
            PROCESS_PROBE_INTERVAL
        };
        self.probe_due = Instant::now() + interval;
    }

    /// Hands the windows of each application on; `None`: not followed. A
    /// change brings the next look at the processes forward.
    fn publish_windows(&mut self, windows: Option<&[Window]>) {
        let changed = self.update(|index, state| {
            let class = &self.apps[index].window_class;
            state.windows = windows.map(|windows| {
                windows
                    .iter()
                    .filter(|window| class.matches(&window.class))
                    .cloned()
                    .collect()
            });
        });
        if changed {
            self.quiet_probes = 0;
            self.probe_due = self.probe_due.min(Instant::now() + PROCESS_PROBE_INTERVAL);
        }
    }

    /// Applies `change` to the state of each application (by its index),
    /// publishing only when something changed; whether it did.
    fn update(&self, mut change: impl FnMut(usize, &mut AppState)) -> bool {
        self.states.send_if_modified(|current| {
            let mut changed = false;
            for (index, app) in self.apps.iter().enumerate() {
                let state = current.apps.entry(app.id).or_default();
                let before = state.clone();
                change(index, state);
                changed |= *state != before;
            }
            changed
        })
    }

    fn forward(&self, notification: AppNotification) {
        // Nobody listening: nobody to tell.
        let _receivers = self.notifications.send(notification);
    }
}

/// A listener of the desktop, retried with the backoff of the hub.
fn listener<C>() -> Listener<C> {
    Listener::new(Backoff::new(INITIAL_BACKOFF, MAX_BACKOFF), STABLE_FOLLOWING)
}

/// Logs when `outcome` differs from the last one (`succeeded`).
fn log_transition<T>(succeeded: &mut bool, outcome: &Result<T, DesktopError>, what: &str) {
    match outcome {
        Ok(_) if !*succeeded => tracing::info!("desktop: can {what} again"),
        Err(error) if *succeeded => tracing::info!(%error, "desktop: cannot {what}"),
        _ => {}
    }
    *succeeded = outcome.is_ok();
}

/// The next windows, or pending forever while not following.
async fn next_windows(changes: Option<&mut Box<dyn WindowChanges>>) -> Option<Vec<Window>> {
    match changes {
        Some(changes) => changes.next().await,
        None => std::future::pending().await,
    }
}

/// The next notification, or pending forever while not observing.
async fn next_notification(
    stream: Option<&mut Box<dyn NotificationStream>>,
) -> Option<AppNotification> {
    match stream {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests;
