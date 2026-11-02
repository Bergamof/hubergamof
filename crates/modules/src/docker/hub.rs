//! The Docker hub: one per Docker host, shared by every Docker instance of
//! every device, so that Docker is listened to once and its containers
//! listed once per burst of changes whatever the number of instances.
//!
//! A worker task, started with the first [`Subscription`] and stopped with
//! the last one:
//!
//! - listens to the changes of the containers, listening again after a
//!   bounded backoff when that stops;
//! - lists the containers again once a burst of changes is gathered
//!   ([`READ_COALESCE_DELAY`]: `compose up` creates and starts several),
//!   and at most every [`MIN_READ_INTERVAL`];
//! - publishes the list to every instance ([`Subscription::states`]).
//!
//! Actions ([`Subscription::run`]) run in parallel, each in a task of its
//! own that outlives the instance that asked: an action started is never
//! cut short (a `compose down` killed midway would leave the project half
//! down). Their effect comes back as changes of the containers.

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::future::join_all;
use tokio::sync::watch;
use tokio::task::AbortHandle;

use super::backend::{ComposeAction, ContainerAction, DockerBackend, DockerChanges, DockerError};
use super::model::{ComposeTarget, Container};
use crate::background::{Backoff, Listener, ReadPacer, catch_panic, sleep_until};

/// Time from the first change of a burst to the list that covers it.
const READ_COALESCE_DELAY: Duration = Duration::from_millis(100);

/// Shortest time between the starts of two lists while changes keep coming.
const MIN_READ_INTERVAL: Duration = Duration::from_millis(300);

/// First wait before listening to changes again after it stopped.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Longest wait between two attempts to listen to changes.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// A listening that lasted this long was healthy: the next wait starts over.
const STABLE_LISTENING: Duration = Duration::from_secs(10);

/// What the hub knows of the containers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DockerView {
    /// Not listed yet.
    Loading,
    /// Last list.
    Ready(Arc<[Container]>),
    /// Docker could not be reached.
    Unavailable,
}

/// An action to run on the Docker host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Request {
    /// `action` on every container of `ids`, at once.
    Containers {
        ids: Vec<String>,
        action: ContainerAction,
    },
    /// `action` on the compose project `target`.
    Compose {
        target: ComposeTarget,
        action: ComposeAction,
    },
}

#[derive(Debug, Default)]
struct Control {
    subscribers: usize,
    worker: Option<AbortHandle>,
}

/// Shared access to a Docker host, see the [module documentation](self).
#[derive(Debug)]
pub(super) struct DockerHub {
    backend: Arc<dyn DockerBackend>,
    states: watch::Sender<DockerView>,
    control: Mutex<Control>,
}

impl DockerHub {
    pub(super) fn new(backend: Arc<dyn DockerBackend>) -> Arc<Self> {
        Arc::new(Self {
            backend,
            states: watch::Sender::new(DockerView::Loading),
            control: Mutex::new(Control::default()),
        })
    }

    /// A subscription; the first one starts the worker (on the current
    /// Tokio runtime, if any), as does one finding it stopped.
    pub(super) fn subscribe(self: &Arc<Self>) -> Subscription {
        let mut control = self.lock();
        control.subscribers += 1;
        let running = control
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished());
        if !running {
            control.worker = self.start_worker();
        }
        Subscription {
            hub: Arc::clone(self),
        }
    }

    fn start_worker(&self) -> Option<AbortHandle> {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::debug!("no Tokio runtime: Docker is not listed");
            return None;
        };
        let worker = Worker::new(Arc::clone(&self.backend), self.states.clone());
        Some(runtime.spawn(worker.run()).abort_handle())
    }

    fn unsubscribe(&self) {
        let mut control = self.lock();
        control.subscribers = control.subscribers.saturating_sub(1);
        if control.subscribers == 0
            && let Some(worker) = control.worker.take()
        {
            // The worker only reads: stopping it mid-call loses nothing.
            worker.abort();
        }
    }

    /// Stops the worker as a crash would.
    #[cfg(test)]
    pub(super) fn crash_worker(&self) {
        if let Some(worker) = &self.lock().worker {
            worker.abort();
        }
    }

    fn lock(&self) -> MutexGuard<'_, Control> {
        // Every update keeps the control consistent: a poisoned lock is
        // still valid.
        self.control.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// An instance's access to the hub; dropping it unsubscribes.
#[derive(Debug)]
pub(super) struct Subscription {
    hub: Arc<DockerHub>,
}

impl Subscription {
    /// The states published by the hub, the current one first.
    pub(super) fn states(&self) -> watch::Receiver<DockerView> {
        self.hub.states.subscribe()
    }

    /// Runs `request` in a task of its own (see the [module
    /// documentation](self)), then hands its outcome to `report`; `false`
    /// without a Tokio runtime to run it on.
    pub(super) fn run(
        &self,
        request: Request,
        report: impl FnOnce(Result<(), DockerError>) + Send + 'static,
    ) -> bool {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return false;
        };
        let backend = Arc::clone(&self.hub.backend);
        runtime.spawn(async move {
            let outcome = catch_panic(Box::pin(execute(backend.as_ref(), request)))
                .await
                .unwrap_or(Err(DockerError::Panicked));
            report(outcome);
        });
        true
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.hub.unsubscribe();
    }
}

/// Runs `request` on `backend`; the actions on several containers run at
/// once, and the first failure is reported.
async fn execute(backend: &dyn DockerBackend, request: Request) -> Result<(), DockerError> {
    match request {
        Request::Containers { ids, action } => {
            let results = join_all(ids.iter().map(|id| backend.container(id, action))).await;
            results.into_iter().collect()
        }
        Request::Compose { target, action } => backend.compose(&target, action).await,
    }
}

struct Worker {
    backend: Arc<dyn DockerBackend>,
    states: watch::Sender<DockerView>,
    pacer: ReadPacer,
    /// Whether the last list succeeded, to log only transitions.
    reachable: bool,
    listener: Listener<Box<dyn DockerChanges>>,
}

impl Worker {
    fn new(backend: Arc<dyn DockerBackend>, states: watch::Sender<DockerView>) -> Self {
        Self {
            backend,
            states,
            pacer: ReadPacer::new(READ_COALESCE_DELAY, MIN_READ_INTERVAL),
            reachable: true,
            listener: Listener::new(Backoff::new(INITIAL_BACKOFF, MAX_BACKOFF), STABLE_LISTENING),
        }
    }

    /// Serves until aborted.
    async fn run(mut self) {
        loop {
            let (read_due, listen_due) = (self.pacer.due(), self.listener.due());
            // Listening comes before listing, so that no change is missed
            // between the two.
            tokio::select! {
                biased;
                () = sleep_until(listen_due) => self.listen().await,
                changed = next_change(self.listener.changes()) => {
                    if !changed {
                        // Docker may have restarted or gone.
                        self.listener.ended();
                    }
                    self.pacer.request();
                }
                () = sleep_until(read_due) => self.list().await,
            }
        }
    }

    async fn list(&mut self) {
        self.pacer.start();
        let listed = guarded(self.backend.containers()).await;
        let view = match listed {
            Ok(containers) => {
                if !self.reachable {
                    tracing::info!("Docker reachable again");
                }
                self.reachable = true;
                DockerView::Ready(containers.into())
            }
            Err(error) => {
                if self.reachable {
                    tracing::warn!(%error, "cannot list the Docker containers");
                }
                self.reachable = false;
                DockerView::Unavailable
            }
        };
        self.states.send_if_modified(|current| {
            let changed = *current != view;
            *current = view;
            changed
        });
    }

    /// Listens to the changes; lists either way: changes made while not
    /// listening were missed, or Docker may have gone.
    async fn listen(&mut self) {
        let changes = guarded(self.backend.changes())
            .await
            .inspect_err(|error| tracing::debug!(%error, "cannot listen to Docker changes"))
            .ok();
        self.listener.listened(changes);
        self.pacer.request();
    }
}

/// Runs a backend call, a panic counting as a failure
/// ([`DockerError::Panicked`]).
async fn guarded<T>(
    call: impl Future<Output = Result<T, DockerError>> + Unpin,
) -> Result<T, DockerError> {
    catch_panic(call)
        .await
        .unwrap_or(Err(DockerError::Panicked))
}

/// Whether a change was notified (`false`: listening stopped), or pending
/// forever while not listening.
async fn next_change(changes: Option<&mut Box<dyn DockerChanges>>) -> bool {
    match changes {
        Some(changes) => changes.changed().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use tokio::time::Instant;

    use super::super::simulated::SimulatedDocker;
    use super::*;

    /// Long enough for every scheduled list to happen.
    const SETTLE: Duration = Duration::from_secs(1);

    async fn idle(duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    fn hub_on(docker: &Arc<SimulatedDocker>) -> Arc<DockerHub> {
        DockerHub::new(Arc::clone(docker) as _)
    }

    fn names(view: &DockerView) -> Vec<String> {
        match view {
            DockerView::Ready(containers) => containers.iter().map(|c| c.name.clone()).collect(),
            other => panic!("list expected, got {other:?}"),
        }
    }

    fn id_of(docker: &SimulatedDocker, name: &str) -> String {
        docker.named(name).unwrap().id
    }

    #[tokio::test(start_paused = true)]
    async fn subscribers_share_one_listener_and_one_list_per_burst() {
        let docker = Arc::new(SimulatedDocker::demo());
        let hub = hub_on(&docker);
        let first = hub.subscribe();
        let _second = hub.subscribe();
        idle(SETTLE).await;
        assert_eq!(docker.listeners(), 1);
        assert_eq!(names(&first.states().borrow()).len(), 7);
        let lists = docker.lists();

        for _ in 0..5 {
            docker.change_externally(|containers| {
                containers.pop();
            });
        }
        idle(SETTLE).await;
        assert_eq!(docker.lists(), lists + 1, "one list for the burst");
        assert_eq!(names(&first.states().borrow()).len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn the_worker_stops_with_its_last_subscriber_and_starts_again() {
        let docker = Arc::new(SimulatedDocker::demo());
        let hub = hub_on(&docker);
        let subscription = hub.subscribe();
        idle(SETTLE).await;
        drop(subscription);
        idle(SETTLE).await;
        assert_eq!(docker.listeners(), 0);
        let lists = docker.lists();
        docker.change_externally(|_| {});
        idle(SETTLE).await;
        assert_eq!(docker.lists(), lists);

        let _again = hub.subscribe();
        idle(SETTLE).await;
        assert_eq!((docker.listeners(), docker.lists()), (1, lists + 1));
    }

    #[tokio::test(start_paused = true)]
    async fn a_crashed_worker_is_replaced_by_the_next_subscription() {
        let docker = Arc::new(SimulatedDocker::demo());
        let hub = hub_on(&docker);
        let _first = hub.subscribe();
        idle(SETTLE).await;
        hub.crash_worker();
        idle(SETTLE).await;
        assert_eq!(docker.listeners(), 0);
        let _second = hub.subscribe();
        idle(SETTLE).await;
        assert_eq!(docker.listeners(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn an_absent_docker_shows_unavailable_and_is_retried_with_a_backoff() {
        let docker = Arc::new(SimulatedDocker::demo());
        let hub = hub_on(&docker);
        let subscription = hub.subscribe();
        idle(SETTLE).await;
        docker.set_available(false);
        idle(SETTLE).await;
        assert_eq!(*subscription.states().borrow(), DockerView::Unavailable);

        let lists = docker.lists();
        // Attempts after 1 s, then 2 s, 4 s…: about 4 in 15 s, not 15.
        idle(Duration::from_secs(15)).await;
        let attempts = docker.lists() - lists;
        assert!((3..=5).contains(&attempts), "{attempts} attempts");

        docker.set_available(true);
        idle(MAX_BACKOFF).await;
        assert_eq!(names(&subscription.states().borrow()).len(), 7);
        assert_eq!(docker.listeners(), 1);
    }

    /// A backend whose every call panics.
    #[derive(Debug)]
    struct Fragile;

    #[async_trait]
    impl DockerBackend for Fragile {
        async fn containers(&self) -> Result<Vec<Container>, DockerError> {
            panic!("fragile list");
        }
        async fn changes(&self) -> Result<Box<dyn DockerChanges>, DockerError> {
            panic!("fragile listening");
        }
        async fn container(&self, _: &str, _: ContainerAction) -> Result<(), DockerError> {
            panic!("fragile action");
        }
        async fn compose(&self, _: &ComposeTarget, _: ComposeAction) -> Result<(), DockerError> {
            panic!("fragile compose");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_panicking_backend_shows_unavailable_and_fails_its_actions() {
        let hub = DockerHub::new(Arc::new(Fragile));
        let subscription = hub.subscribe();
        idle(SETTLE).await;
        assert_eq!(*subscription.states().borrow(), DockerView::Unavailable);
        let (sender, outcome) = tokio::sync::oneshot::channel();
        let request = Request::Containers {
            ids: vec!["a".into()],
            action: ContainerAction::Start,
        };
        assert!(subscription.run(request, move |outcome| {
            let _ = sender.send(outcome);
        }));
        assert!(matches!(outcome.await.unwrap(), Err(DockerError::Panicked)));
        // The worker still serves.
        idle(MAX_BACKOFF).await;
        assert!(!hub.lock().worker.as_ref().unwrap().is_finished());
    }

    #[tokio::test(start_paused = true)]
    async fn actions_on_several_containers_run_at_once() {
        let docker = Arc::new(SimulatedDocker::demo());
        docker.set_latency(Duration::from_secs(2));
        let subscription = hub_on(&docker).subscribe();
        let ids = ["supervision-grafana-1", "supervision-prometheus-1"]
            .map(|name| id_of(&docker, name))
            .to_vec();
        let (sender, outcome) = tokio::sync::oneshot::channel();
        let start = Instant::now();
        let request = Request::Containers {
            ids,
            action: ContainerAction::Start,
        };
        subscription.run(request, move |outcome| {
            let _ = sender.send(outcome);
        });
        outcome.await.unwrap().unwrap();
        assert_eq!(start.elapsed(), Duration::from_secs(2), "not 4: at once");
    }

    #[tokio::test(start_paused = true)]
    async fn an_action_outlives_the_subscription_that_started_it() {
        let docker = Arc::new(SimulatedDocker::demo());
        docker.set_latency(Duration::from_secs(5));
        let subscription = hub_on(&docker).subscribe();
        let request = Request::Compose {
            target: ComposeTarget::new(Some("boutique".into()), None, Vec::new()),
            action: ComposeAction::Down { volumes: false },
        };
        let (sender, outcome) = tokio::sync::oneshot::channel();
        subscription.run(request, move |outcome| {
            let _ = sender.send(outcome);
        });
        idle(Duration::from_secs(1)).await;
        drop(subscription);
        outcome.await.unwrap().unwrap();
        assert_eq!(docker.actions(), ["down boutique"]);
    }

    #[test]
    fn without_a_runtime_nothing_is_listed_nor_run() {
        let docker = Arc::new(SimulatedDocker::demo());
        let subscription = hub_on(&docker).subscribe();
        assert_eq!(*subscription.states().borrow(), DockerView::Loading);
        assert_eq!(docker.lists(), 0);
        let request = Request::Containers {
            ids: Vec::new(),
            action: ContainerAction::Stop,
        };
        assert!(!subscription.run(request, |_| {}));
    }
}
