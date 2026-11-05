//! Building blocks of the background workers that follow an external
//! service (audio server, Docker…) for several instances: pacing of the
//! reads, waits between reconnections, and isolation of the backend calls.

use std::any::Any;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::task::Poll;
use std::time::Duration;

use bergadeck_module_api::EventSender;
use tokio::sync::watch;
use tokio::task::AbortHandle;
use tokio::time::Instant;

/// When to read the state of a service again after it notified changes.
///
/// A read covers a burst of notifications: it happens a short delay after
/// the first one (never postponed by the next ones, so that changes that
/// keep coming are still shown), and not sooner than a minimal interval
/// after the start of the previous read.
#[derive(Debug, Clone)]
pub(crate) struct ReadPacer {
    coalesce: Duration,
    min_interval: Duration,
    due: Option<Instant>,
    last: Option<Instant>,
}

impl ReadPacer {
    /// A pacer whose first read is due at once.
    pub(crate) fn new(coalesce: Duration, min_interval: Duration) -> Self {
        Self {
            coalesce,
            min_interval,
            due: Some(Instant::now()),
            last: None,
        }
    }

    /// Schedules a read, unless one already is.
    pub(crate) fn request(&mut self) {
        if self.due.is_none() {
            let coalesced = Instant::now() + self.coalesce;
            let paced = self.last.map(|last| last + self.min_interval);
            self.due = Some(paced.map_or(coalesced, |paced| paced.max(coalesced)));
        }
    }

    /// When the scheduled read is due, `None` without one.
    pub(crate) fn due(&self) -> Option<Instant> {
        self.due
    }

    /// The scheduled read starts now.
    pub(crate) fn start(&mut self) {
        self.due = None;
        self.last = Some(Instant::now());
    }
}

/// Waits between two attempts to reach a service, doubling from `initial`
/// up to `max`.
#[derive(Debug, Clone)]
pub(crate) struct Backoff {
    initial: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    pub(crate) fn new(initial: Duration, max: Duration) -> Self {
        Self {
            initial,
            max,
            next: initial,
        }
    }

    pub(crate) fn next_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = (self.next * 2).min(self.max);
        delay
    }

    /// The next wait is the initial one again.
    pub(crate) fn reset(&mut self) {
        self.next = self.initial;
    }
}

/// When to listen to the changes of a service: at once, then, each time
/// listening fails or stops, after a [`Backoff`] that starts over once a
/// listening lasted long enough to be called healthy.
pub(crate) struct Listener<C> {
    changes: Option<C>,
    /// Start of the current listening.
    since: Instant,
    due: Option<Instant>,
    backoff: Backoff,
    stable: Duration,
}

impl<C> Listener<C> {
    /// A listener due at once; a listening lasting `stable` resets
    /// `backoff`.
    pub(crate) fn new(backoff: Backoff, stable: Duration) -> Self {
        let now = Instant::now();
        Self {
            changes: None,
            since: now,
            due: Some(now),
            backoff,
            stable,
        }
    }

    /// When to try to listen, `None` while listening.
    pub(crate) fn due(&self) -> Option<Instant> {
        self.due
    }

    /// The changes listened to, `None` while not listening.
    pub(crate) fn changes(&mut self) -> Option<&mut C> {
        self.changes.as_mut()
    }

    /// Outcome of an attempt to listen: the changes, or `None` when it
    /// failed (tried again after the backoff).
    pub(crate) fn listened(&mut self, changes: Option<C>) {
        match changes {
            Some(changes) => {
                self.changes = Some(changes);
                self.since = Instant::now();
                self.due = None;
            }
            None => self.retry_later(),
        }
    }

    /// The service stopped notifying: listen again after the backoff.
    pub(crate) fn ended(&mut self) {
        self.changes = None;
        if self.since.elapsed() >= self.stable {
            self.backoff.reset();
        }
        self.retry_later();
    }

    fn retry_later(&mut self) {
        self.due = Some(Instant::now() + self.backoff.next_delay());
    }
}

/// The subscribers of a shared worker: the first subscription starts it
/// (as does one finding it stopped), the last one stops it.
#[derive(Debug, Default)]
pub(crate) struct WorkerControl {
    subscribers: usize,
    worker: Option<AbortHandle>,
}

impl WorkerControl {
    /// Counts a subscriber, starting the worker with `start` unless it runs
    /// (`start` gives `None` when it cannot, without a Tokio runtime).
    pub(crate) fn subscribe(&mut self, start: impl FnOnce() -> Option<AbortHandle>) {
        self.subscribers += 1;
        let running = self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished());
        if !running {
            self.worker = start();
        }
    }

    /// Counts a subscriber gone, stopping the worker with the last one.
    pub(crate) fn unsubscribe(&mut self) {
        self.subscribers = self.subscribers.saturating_sub(1);
        if self.subscribers == 0
            && let Some(worker) = self.worker.take()
        {
            worker.abort();
        }
    }

    /// Whether the worker runs.
    #[cfg(test)]
    pub(crate) fn is_running(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
    }

    /// Stops the worker as a crash would.
    #[cfg(test)]
    pub(crate) fn crash(&self) {
        if let Some(worker) = &self.worker {
            worker.abort();
        }
    }
}

/// Spawns the worker `name` on the current Tokio runtime, `None` without
/// one.
pub(crate) fn spawn_worker(
    name: &str,
    worker: impl Future<Output = ()> + Send + 'static,
) -> Option<AbortHandle> {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        tracing::debug!(worker = name, "no Tokio runtime: worker not started");
        return None;
    };
    Some(runtime.spawn(worker).abort_handle())
}

/// Runs `action` in a task of its own (outliving whoever asked), then hands
/// its outcome to `report`, `panicked` if it panicked; `false` without a
/// Tokio runtime to run it on.
pub(crate) fn spawn_action<R: Send + 'static>(
    action: impl Future<Output = R> + Send + 'static,
    panicked: R,
    report: impl FnOnce(R) + Send + 'static,
) -> bool {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return false;
    };
    runtime.spawn(async move {
        report(catch_panic(Box::pin(action)).await.unwrap_or(panicked));
    });
    true
}

/// A backend call panicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Panicked;

/// Runs `call`, turning a panic into [`Panicked`], so that a bug in a
/// backend does not stop the worker shared by every device.
pub(crate) async fn catch_panic<F: Future + Unpin>(mut call: F) -> Result<F::Output, Panicked> {
    std::future::poll_fn(move |cx| {
        // A panicking call is never polled again: it is complete.
        match catch_unwind(AssertUnwindSafe(|| std::pin::Pin::new(&mut call).poll(cx))) {
            Ok(Poll::Ready(output)) => Poll::Ready(Ok(output)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(_) => Poll::Ready(Err(Panicked)),
        }
    })
    .await
}

/// Sleeps until `deadline`, forever without one.
pub(crate) async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// Hands every state published on `states` to an instance, the current one
/// first, as the event `wrap` makes of it; ends when the instance is gone
/// (its event sender refuses events) or when nothing publishes any more.
pub(crate) async fn forward_states<T: Clone, E: Any + Send>(
    mut states: watch::Receiver<T>,
    events: EventSender,
    wrap: impl Fn(T) -> E,
) {
    loop {
        let state = states.borrow_and_update().clone();
        if !events.send(wrap(state)) || states.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COALESCE: Duration = Duration::from_millis(50);
    const MIN_INTERVAL: Duration = Duration::from_millis(150);

    #[tokio::test(start_paused = true)]
    async fn reads_are_due_at_once_then_coalesced_then_paced() {
        let start = Instant::now();
        let mut pacer = ReadPacer::new(COALESCE, MIN_INTERVAL);
        assert_eq!(pacer.due(), Some(start));
        pacer.start();
        assert_eq!(pacer.due(), None);

        // Paced: not sooner than the interval after the previous read.
        pacer.request();
        assert_eq!(pacer.due(), Some(start + MIN_INTERVAL));
        // Never postponed by the next requests.
        tokio::time::sleep(Duration::from_millis(100)).await;
        pacer.request();
        assert_eq!(pacer.due(), Some(start + MIN_INTERVAL));

        // Long after the previous read: only the coalescing delay.
        tokio::time::sleep(Duration::from_secs(1)).await;
        pacer.start();
        tokio::time::sleep(Duration::from_secs(1)).await;
        pacer.request();
        assert_eq!(pacer.due(), Some(Instant::now() + COALESCE));
    }

    #[test]
    fn backoff_doubles_up_to_its_bound_and_resets() {
        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(30));
        let delays: Vec<_> = (0..7).map(|_| backoff.next_delay().as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30]);
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn listening_is_retried_with_a_backoff_that_starts_over_when_healthy() {
        let backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(30));
        let stable = Duration::from_secs(10);
        let mut listener: Listener<u8> = Listener::new(backoff, stable);
        let delay = |listener: &Listener<u8>| listener.due().map(|due| due - Instant::now());
        assert_eq!(delay(&listener), Some(Duration::ZERO));

        listener.listened(None);
        assert_eq!(delay(&listener), Some(Duration::from_secs(1)));
        listener.listened(Some(7));
        assert_eq!((listener.due(), listener.changes()), (None, Some(&mut 7)));
        // Ended at once: not healthy, the backoff goes on.
        listener.ended();
        assert_eq!(listener.changes(), None);
        assert_eq!(delay(&listener), Some(Duration::from_secs(2)));

        listener.listened(Some(8));
        tokio::time::sleep(stable).await;
        listener.ended();
        assert_eq!(
            delay(&listener),
            Some(Duration::from_secs(1)),
            "healthy: starts over"
        );
    }

    #[tokio::test]
    async fn a_panicking_call_is_reported_and_a_finished_one_returned() {
        let panicking = Box::pin(async {
            panic!("fragile backend");
        });
        assert_eq!(catch_panic(panicking).await, Err::<(), _>(Panicked));
        assert_eq!(catch_panic(Box::pin(async { 7 })).await, Ok(7));
    }

    #[tokio::test]
    async fn states_are_forwarded_until_the_instance_is_gone() {
        let (publisher, states) = watch::channel(1_u8);
        let (sender, mut received) = tokio::sync::mpsc::unbounded_channel();
        let events = EventSender::new(move |event| sender.send(event).is_ok());
        let forwarding = tokio::spawn(forward_states(states, events, u16::from));
        let first = received.recv().await.unwrap();
        assert_eq!(*first.downcast::<u16>().unwrap(), 1);
        publisher.send_replace(2);
        let second = received.recv().await.unwrap();
        assert_eq!(*second.downcast::<u16>().unwrap(), 2);
        drop(received);
        publisher.send_replace(3);
        forwarding.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn sleeping_without_a_deadline_never_ends() {
        let never = tokio::time::timeout(Duration::from_secs(3600), sleep_until(None)).await;
        assert!(never.is_err());
        let deadline = Instant::now() + Duration::from_secs(1);
        sleep_until(Some(deadline)).await;
        assert_eq!(Instant::now(), deadline);
    }
}
