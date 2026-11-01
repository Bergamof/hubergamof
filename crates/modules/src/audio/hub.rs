//! The audio hub: one per audio server, shared by every audio instance of
//! every device, so that the server is listened to once and read once per
//! change whatever the number of instances.
//!
//! Handlers never wait for the audio server: they update the state shown
//! (optimistically) and submit a change. A single worker task, started with
//! the first [`Subscription`] and ending by itself once the last one is
//! dropped and the changes queued are applied, owns every call to the
//! backend:
//!
//! - it applies the changes one at a time (calls are serialised), merging
//!   those queued meanwhile (a fast dial turn costs one `set-sink-volume`
//!   per round trip, not one per notch);
//! - it listens to the changes of the server (one `pactl subscribe`),
//!   listening again after a bounded backoff when that stops;
//! - it reads again only the parts of the state that changed, once a burst
//!   of changes is gathered ([`READ_COALESCE_DELAY`], which also absorbs the
//!   echo of its own changes) and at most every [`MIN_READ_INTERVAL`];
//! - it publishes the state to every instance ([`watch`]), tagged with the
//!   last change applied so that an instance ignores a state older than its
//!   own changes.

use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Poll;
use std::time::Duration;

use bergadeck_module_api::{EventSender, Feedback, ModuleSurface};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::watch;
use tokio::task::AbortHandle;
use tokio::time::Instant;

use super::backend::{AudioBackend, AudioChange, AudioChanges, AudioError, ReadScope};
use super::state::AudioSnapshot;

/// Time from the first change of a burst (one change produces several
/// notifications, and the server echoes the changes of the worker) to the
/// read that covers the whole burst. Counted from the first change, never
/// postponed by the next ones.
pub(super) const READ_COALESCE_DELAY: Duration = Duration::from_millis(50);

/// Shortest time between the starts of two reads while changes keep coming
/// (dial turned without pause, levels moving): the display follows at this
/// pace, and the last change is always read.
pub(super) const MIN_READ_INTERVAL: Duration = Duration::from_millis(150);

/// First wait before listening to changes again after it stopped.
pub(super) const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Longest wait between two attempts to listen to changes.
pub(super) const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// A subscription that lived this long was healthy: the next wait starts
/// over.
pub(super) const STABLE_SUBSCRIPTION: Duration = Duration::from_secs(10);

/// Sequence number of the changes submitted to a hub, from 1, in the order
/// the worker applies them.
pub(super) type Sequence = u64;

/// Identifies a subscription within its hub.
type SubscriberId = u64;

/// What the hub knows of the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ServerView {
    /// Not read yet.
    Loading,
    /// Last state read.
    Ready(Arc<AudioSnapshot>),
    /// The server could not be read.
    Unavailable,
}

/// State published to the instances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HubState {
    pub(super) server: ServerView,
    /// Every change up to this one was applied (or failed) before the read.
    pub(super) applied: Sequence,
}

/// What an instance receives through its event sender.
#[derive(Debug)]
pub(super) enum AudioEvent {
    /// A new state of the server.
    State(HubState),
    /// Outcome of a change, to show on `surface`.
    Outcome {
        surface: ModuleSurface,
        feedback: Feedback,
    },
}

/// A change submitted by an instance.
#[derive(Debug)]
struct Job {
    seq: Sequence,
    subscriber: SubscriberId,
    /// Where the outcome goes.
    replies: EventSender,
    change: AudioChange,
    /// Shows the failure (or the success of a selection).
    surface: ModuleSurface,
}

/// Work queued for the worker.
#[derive(Debug)]
enum Request {
    Apply(Job),
    /// Reads everything again (an instance found the state missing).
    Refresh,
    /// The last subscription was dropped: end unless subscribed again
    /// meanwhile.
    Stop,
}

/// The running worker.
#[derive(Debug)]
struct WorkerHandle {
    generation: u64,
    requests: UnboundedSender<Request>,
    task: AbortHandle,
}

#[derive(Debug, Default)]
struct Control {
    subscribers: usize,
    next_subscriber: SubscriberId,
    last_seq: Sequence,
    /// Number of workers started so far.
    generations: u64,
    worker: Option<WorkerHandle>,
}

/// Shared access to an audio server, see the [module documentation](self).
#[derive(Debug)]
pub(super) struct AudioHub {
    backend: Arc<dyn AudioBackend>,
    states: watch::Sender<HubState>,
    control: Mutex<Control>,
}

impl AudioHub {
    pub(super) fn new(backend: Arc<dyn AudioBackend>) -> Arc<Self> {
        let initial = HubState {
            server: ServerView::Loading,
            applied: 0,
        };
        Arc::new(Self {
            backend,
            states: watch::Sender::new(initial),
            control: Mutex::new(Control::default()),
        })
    }

    /// A subscription whose outcomes go to `replies`; the first one starts
    /// the worker (on the current Tokio runtime, if any).
    pub(super) fn subscribe(self: &Arc<Self>, replies: EventSender) -> Subscription {
        let mut control = self.lock();
        control.subscribers += 1;
        control.next_subscriber += 1;
        let id = control.next_subscriber;
        if control.worker.is_none() {
            self.start_worker(&mut control);
        }
        Subscription {
            hub: Arc::clone(self),
            id,
            replies,
        }
    }

    fn start_worker(self: &Arc<Self>, control: &mut Control) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::debug!("no Tokio runtime: the audio server is not read");
            return;
        };
        control.generations += 1;
        let generation = control.generations;
        let (requests, receiver) = mpsc::unbounded_channel();
        let worker = Worker::new(Arc::clone(self), generation, control.last_seq);
        let task = runtime.spawn(worker.run(receiver)).abort_handle();
        control.worker = Some(WorkerHandle {
            generation,
            requests,
            task,
        });
    }

    /// Starts a new worker in place of one that stopped unexpectedly. Its
    /// first state covers every change submitted so far: those lost with
    /// the old worker (taken by a call that panicked the task) are not
    /// waited for.
    fn restart_worker(self: &Arc<Self>, control: &mut Control) {
        if let Some(stopped) = control.worker.take() {
            stopped.task.abort();
        }
        tracing::warn!("the audio worker stopped; starting it again");
        self.start_worker(control);
    }

    /// The worker is not stopped here but asked to stop: it applies the
    /// changes queued first, and keeps serving a subscription made before
    /// it handles the request, so that two workers never call the backend
    /// at the same time.
    fn unsubscribe(&self) {
        let mut control = self.lock();
        control.subscribers = control.subscribers.saturating_sub(1);
        if control.subscribers == 0
            && let Some(worker) = &control.worker
            && worker.requests.send(Request::Stop).is_err()
        {
            control.worker = None;
        }
    }

    /// Whether the worker `generation` must end on a stop request: nobody
    /// subscribed again and nothing is left to serve (requests are sent
    /// under the same lock), or it was replaced. It is then forgotten, so
    /// that the next subscription starts a new one.
    fn retire(&self, generation: u64, requests: &UnboundedReceiver<Request>) -> bool {
        let mut control = self.lock();
        let current = control
            .worker
            .as_ref()
            .is_some_and(|worker| worker.generation == generation);
        if !current {
            return true;
        }
        if control.subscribers > 0 || !requests.is_empty() {
            return false;
        }
        control.worker = None;
        true
    }

    fn submit(self: &Arc<Self>, job: impl FnOnce(Sequence) -> Job) -> Option<Sequence> {
        let mut control = self.lock();
        control.last_seq += 1;
        let seq = control.last_seq;
        self.send(&mut control, Request::Apply(job(seq)))
            .then_some(seq)
    }

    fn refresh(self: &Arc<Self>) {
        let mut control = self.lock();
        self.send(&mut control, Request::Refresh);
    }

    /// Sends `request` to the worker, to a new one if it stopped
    /// unexpectedly (nothing submitted is lost); `false` without a runtime
    /// to start one on.
    fn send(self: &Arc<Self>, control: &mut Control, request: Request) -> bool {
        let refused = match &control.worker {
            Some(worker) => match worker.requests.send(request) {
                Ok(()) => return true,
                Err(refused) => refused.0,
            },
            None => request,
        };
        self.restart_worker(control);
        control
            .worker
            .as_ref()
            .is_some_and(|worker| worker.requests.send(refused).is_ok())
    }

    /// Publishes `state` unless the worker `generation` was replaced.
    fn publish(&self, generation: u64, state: HubState) {
        let control = self.lock();
        if control
            .worker
            .as_ref()
            .is_some_and(|worker| worker.generation == generation)
        {
            self.states.send_replace(state);
        }
    }

    /// Stops the worker as a crash would.
    #[cfg(test)]
    pub(super) fn crash_worker(&self) {
        if let Some(worker) = &self.lock().worker {
            worker.task.abort();
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
    hub: Arc<AudioHub>,
    id: SubscriberId,
    replies: EventSender,
}

impl Subscription {
    /// The states published by the hub, the current one first.
    pub(super) fn states(&self) -> watch::Receiver<HubState> {
        self.hub.states.subscribe()
    }

    /// Queues `change`, whose outcome will be shown on `surface`; its
    /// sequence number, or `None` without a Tokio runtime to apply it on.
    pub(super) fn submit(&self, change: AudioChange, surface: ModuleSurface) -> Option<Sequence> {
        self.hub.submit(|seq| Job {
            seq,
            subscriber: self.id,
            replies: self.replies.clone(),
            change,
            surface,
        })
    }

    /// Asks for the whole state to be read again.
    pub(super) fn refresh(&self) {
        self.hub.refresh();
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.hub.unsubscribe();
    }
}

/// Hands every state published by the hub to an instance, until the
/// instance is gone (its event sender refuses events).
pub(super) async fn forward_states(mut states: watch::Receiver<HubState>, events: EventSender) {
    loop {
        let state = states.borrow_and_update().clone();
        if !events.send(AudioEvent::State(state)) || states.changed().await.is_err() {
            return;
        }
    }
}

struct Worker {
    hub: Arc<AudioHub>,
    generation: u64,
    applied: Sequence,
    /// Last state read, `None` until read or while unreachable.
    snapshot: Option<Arc<AudioSnapshot>>,
    /// Whether the last read succeeded, to log only transitions.
    reachable: bool,
    /// Parts to read at `read_due`.
    pending: ReadScope,
    read_due: Option<Instant>,
    last_read: Option<Instant>,
    changes: Option<Box<dyn AudioChanges>>,
    subscribed_at: Instant,
    resubscribe_at: Option<Instant>,
    backoff: Backoff,
}

impl Worker {
    fn new(hub: Arc<AudioHub>, generation: u64, applied: Sequence) -> Self {
        let now = Instant::now();
        Self {
            hub,
            generation,
            applied,
            snapshot: None,
            reachable: true,
            pending: ReadScope::ALL,
            read_due: Some(now),
            last_read: None,
            changes: None,
            subscribed_at: now,
            resubscribe_at: Some(now),
            backoff: Backoff::default(),
        }
    }

    /// Serves until asked to stop for good (see [`AudioHub::retire`]) or
    /// until replaced.
    async fn run(mut self, mut requests: UnboundedReceiver<Request>) {
        loop {
            let (read_due, resubscribe_at) = (self.read_due, self.resubscribe_at);
            // Requests first: a change is applied before a read that would
            // show the state without it. Listening comes before reading so
            // that no change is missed between the two.
            tokio::select! {
                biased;
                request = requests.recv() => {
                    let Some(first) = request else {
                        return;
                    };
                    let mut batch = vec![first];
                    while let Ok(request) = requests.try_recv() {
                        batch.push(request);
                    }
                    let stop = self.serve(batch).await;
                    if stop && self.hub.retire(self.generation, &requests) {
                        return;
                    }
                }
                () = sleep_until(resubscribe_at) => self.subscribe().await,
                change = next_change(self.changes.as_mut()) => match change {
                    Some(scope) => self.mark_dirty(scope),
                    None => self.subscription_ended(),
                },
                () = sleep_until(read_due) => self.read().await,
            }
        }
    }

    /// Serves `batch`; whether it asked the worker to stop.
    async fn serve(&mut self, batch: Vec<Request>) -> bool {
        let merged = merge(batch);
        if merged.refresh {
            self.mark_dirty(ReadScope::ALL);
        }
        for job in merged.jobs {
            self.apply(job).await;
        }
        merged.stop
    }

    async fn apply(&mut self, job: Job) {
        self.applied = job.seq;
        let result = guarded(self.hub.backend.apply(&job.change)).await;
        // Read again even after a failure, to show the real state.
        self.mark_dirty(job.change.scope());
        let feedback = match result {
            Ok(()) => matches!(job.change, AudioChange::Default { .. }).then_some(Feedback::Ok),
            Err(error) => {
                tracing::warn!(%error, change = ?job.change, "audio change failed");
                Some(Feedback::Alert)
            }
        };
        if let Some(feedback) = feedback {
            // An instance gone meanwhile needs no outcome.
            let _ = job.replies.send(AudioEvent::Outcome {
                surface: job.surface,
                feedback,
            });
        }
    }

    /// Schedules a read of `scope` (with what is already pending), after
    /// [`READ_COALESCE_DELAY`] and [`MIN_READ_INTERVAL`]; an already
    /// scheduled read is not postponed, so that changes that keep coming are
    /// still shown.
    fn mark_dirty(&mut self, scope: ReadScope) {
        self.pending = self.pending | scope;
        if self.read_due.is_none() {
            let coalesced = Instant::now() + READ_COALESCE_DELAY;
            let paced = self.last_read.map(|last| last + MIN_READ_INTERVAL);
            self.read_due = Some(paced.map_or(coalesced, |paced| paced.max(coalesced)));
        }
    }

    async fn read(&mut self) {
        self.read_due = None;
        self.last_read = Some(Instant::now());
        let pending = std::mem::take(&mut self.pending);
        // Without a state to update, read everything.
        let scope = if self.snapshot.is_some() {
            pending
        } else {
            ReadScope::ALL
        };
        let server = match guarded(self.hub.backend.read(scope)).await {
            Ok(partial) => {
                if !self.reachable {
                    tracing::info!("audio server reachable again");
                }
                self.reachable = true;
                let snapshot = self.snapshot.get_or_insert_default();
                Arc::make_mut(snapshot).update(partial);
                ServerView::Ready(Arc::clone(snapshot))
            }
            Err(error) => {
                if self.reachable {
                    tracing::warn!(%error, "cannot read the audio state");
                }
                self.reachable = false;
                self.snapshot = None;
                ServerView::Unavailable
            }
        };
        self.hub.publish(
            self.generation,
            HubState {
                server,
                applied: self.applied,
            },
        );
    }

    async fn subscribe(&mut self) {
        self.resubscribe_at = None;
        match guarded(self.hub.backend.subscribe()).await {
            Ok(changes) => {
                self.changes = Some(changes);
                self.subscribed_at = Instant::now();
                // Changes made while not listening were missed.
                self.mark_dirty(ReadScope::ALL);
            }
            Err(error) => {
                tracing::debug!(%error, "cannot listen to audio changes");
                self.listen_later();
            }
        }
    }

    fn subscription_ended(&mut self) {
        self.changes = None;
        if self.subscribed_at.elapsed() >= STABLE_SUBSCRIPTION {
            self.backoff.reset();
        }
        self.listen_later();
    }

    /// Listens again after the backoff and reads everything meanwhile: the
    /// server may have restarted or gone.
    fn listen_later(&mut self) {
        self.resubscribe_at = Some(Instant::now() + self.backoff.next_delay());
        self.mark_dirty(ReadScope::ALL);
    }
}

/// A batch of requests, merged.
#[derive(Debug, Default)]
struct Merged {
    /// The changes in order, consecutive ones of the same instance merged
    /// when the later one replaces the earlier one
    /// ([`AudioChange::merged_with`]).
    jobs: Vec<Job>,
    refresh: bool,
    stop: bool,
}

fn merge(batch: Vec<Request>) -> Merged {
    let mut merged = Merged::default();
    let jobs = &mut merged.jobs;
    for request in batch {
        let job = match request {
            Request::Apply(job) => job,
            Request::Refresh => {
                merged.refresh = true;
                continue;
            }
            Request::Stop => {
                merged.stop = true;
                continue;
            }
        };
        if let Some(last) = jobs.last_mut()
            && last.subscriber == job.subscriber
            && let Some(merged) = last.change.merged_with(&job.change)
        {
            *last = Job {
                change: merged,
                ..job
            };
            continue;
        }
        jobs.push(job);
    }
    merged
}

/// Runs a backend call, turning a panic into [`AudioError::Panicked`] so
/// that a bug in a backend does not stop the audio of every device.
async fn guarded<T>(
    mut call: impl Future<Output = Result<T, AudioError>> + Unpin,
) -> Result<T, AudioError> {
    std::future::poll_fn(move |cx| {
        // A panicking call is never polled again: it is complete.
        catch_unwind(AssertUnwindSafe(|| std::pin::Pin::new(&mut call).poll(cx)))
            .unwrap_or(Poll::Ready(Err(AudioError::Panicked)))
    })
    .await
}

/// The next change notified, or pending forever while not listening.
async fn next_change(changes: Option<&mut Box<dyn AudioChanges>>) -> Option<ReadScope> {
    match changes {
        Some(changes) => changes.next_change().await,
        None => std::future::pending().await,
    }
}

/// Sleeps until `deadline`, forever without one.
async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// Waits between two attempts to listen to changes, doubling up to
/// [`MAX_BACKOFF`].
#[derive(Debug)]
struct Backoff {
    next: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            next: INITIAL_BACKOFF,
        }
    }
}

impl Backoff {
    fn next_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = (self.next * 2).min(MAX_BACKOFF);
        delay
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests;
