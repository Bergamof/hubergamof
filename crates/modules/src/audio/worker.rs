//! Background tasks of an audio instance: the worker, sole user of the
//! backend, and the watcher of external changes.
//!
//! Handlers never wait for the audio server: they update the state shown
//! (optimistically) and queue a [`Request`]. The worker applies requests one
//! at a time, merging those queued meanwhile (a fast dial turn costs one
//! `set-sink-volume` per round trip, not one per notch), then reads the
//! state again and sends it to the instance, tagged with the last request it
//! covers so that the instance ignores a state older than its own changes.

use std::sync::Arc;
use std::time::Duration;

use bergadeck_module_api::{EventSender, Feedback, ModuleSurface};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::time::Instant;

use super::backend::{AudioBackend, AudioChange};
use super::state::AudioSnapshot;

/// Time given to a burst of external change notifications (one change
/// produces several) before reading the state once.
pub(super) const REFRESH_DEBOUNCE: Duration = Duration::from_millis(50);

/// First wait before listening to changes again after the watcher stopped.
pub(super) const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Longest wait between two attempts to listen to changes.
pub(super) const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// A watcher that lived this long was healthy: the next wait starts over.
pub(super) const STABLE_SUBSCRIPTION: Duration = Duration::from_secs(10);

/// Sequence number of the changes queued by the instance, from 1.
pub(super) type Sequence = u64;

/// Work queued for the worker.
#[derive(Debug)]
pub(super) enum Request {
    /// Applies a change; `surface` shows its failure (or its success when
    /// the change is a selection).
    Apply {
        seq: Sequence,
        change: AudioChange,
        surface: ModuleSurface,
    },
    /// Reads the state again (something changed elsewhere).
    Refresh,
}

/// What the worker tells the instance.
#[derive(Debug)]
pub(super) enum AudioEvent {
    /// State of the server (`None`: unreachable), covering the changes up
    /// to `applied`.
    State {
        snapshot: Option<AudioSnapshot>,
        applied: Sequence,
    },
    /// Outcome of a change, to show on `surface`.
    Outcome {
        surface: ModuleSurface,
        feedback: Feedback,
    },
}

/// A change to apply, possibly merged from several requests.
#[derive(Debug)]
struct Job {
    seq: Sequence,
    change: AudioChange,
    surface: ModuleSurface,
}

/// Serves `requests` until the instance is gone (its event sender refuses
/// events, or every request sender is dropped).
pub(super) async fn run_worker(
    backend: Arc<dyn AudioBackend>,
    mut requests: UnboundedReceiver<Request>,
    events: EventSender,
) {
    let mut worker = Worker {
        backend,
        events,
        applied: 0,
        reachable: true,
    };
    if !worker.publish_state().await {
        return;
    }
    while let Some(first) = requests.recv().await {
        let mut batch = vec![first];
        drain(&mut requests, &mut batch);
        if batch
            .iter()
            .all(|request| matches!(request, Request::Refresh))
        {
            tokio::time::sleep(REFRESH_DEBOUNCE).await;
            drain(&mut requests, &mut batch);
        }
        for job in merge(batch) {
            if !worker.apply(job).await {
                return;
            }
        }
        if !worker.publish_state().await {
            return;
        }
    }
}

struct Worker {
    backend: Arc<dyn AudioBackend>,
    events: EventSender,
    applied: Sequence,
    /// Whether the last read succeeded, to log only transitions.
    reachable: bool,
}

impl Worker {
    /// Applies `job`; `false` when the instance is gone.
    async fn apply(&mut self, job: Job) -> bool {
        self.applied = job.seq;
        if job.change.is_noop() {
            return true;
        }
        let feedback = match self.backend.apply(&job.change).await {
            Ok(()) => matches!(job.change, AudioChange::Default { .. }).then_some(Feedback::Ok),
            Err(error) => {
                tracing::warn!(%error, change = ?job.change, "audio change failed");
                Some(Feedback::Alert)
            }
        };
        feedback.is_none_or(|feedback| {
            self.events.send(AudioEvent::Outcome {
                surface: job.surface,
                feedback,
            })
        })
    }

    /// Reads the state and sends it; `false` when the instance is gone.
    async fn publish_state(&mut self) -> bool {
        let snapshot = match self.backend.snapshot().await {
            Ok(snapshot) => {
                if !self.reachable {
                    tracing::info!("audio server reachable again");
                }
                self.reachable = true;
                Some(snapshot)
            }
            Err(error) => {
                if self.reachable {
                    tracing::warn!(%error, "cannot read the audio state");
                }
                self.reachable = false;
                None
            }
        };
        self.events.send(AudioEvent::State {
            snapshot,
            applied: self.applied,
        })
    }
}

fn drain(requests: &mut UnboundedReceiver<Request>, batch: &mut Vec<Request>) {
    while let Ok(request) = requests.try_recv() {
        batch.push(request);
    }
}

/// The changes of `batch` in order, consecutive ones merged when they
/// combine ([`AudioChange::merged_with`]); refreshes are dropped (the
/// worker reads the state after every batch).
fn merge(batch: Vec<Request>) -> Vec<Job> {
    let mut jobs: Vec<Job> = Vec::new();
    for request in batch {
        let Request::Apply {
            seq,
            change,
            surface,
        } = request
        else {
            continue;
        };
        if let Some(last) = jobs.last_mut()
            && let Some(merged) = last.change.merged_with(&change)
        {
            *last = Job {
                seq,
                change: merged,
                surface,
            };
            continue;
        }
        jobs.push(Job {
            seq,
            change,
            surface,
        });
    }
    jobs
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

/// Asks the worker for a refresh on every external change, listening again
/// (after a bounded backoff) whenever the notifications stop; ends with the
/// worker.
pub(super) async fn watch_changes(
    backend: Arc<dyn AudioBackend>,
    requests: UnboundedSender<Request>,
) {
    let mut backoff = Backoff::default();
    loop {
        let started = Instant::now();
        match backend.subscribe().await {
            Ok(mut changes) => loop {
                tokio::select! {
                    () = requests.closed() => return,
                    notified = changes.changed() => {
                        if !notified {
                            break;
                        }
                        if requests.send(Request::Refresh).is_err() {
                            return;
                        }
                    }
                }
            },
            Err(error) => tracing::debug!(%error, "cannot listen to audio changes"),
        }
        // The server may have restarted or gone: read the state again.
        if requests.send(Request::Refresh).is_err() {
            return;
        }
        if started.elapsed() >= STABLE_SUBSCRIPTION {
            backoff.reset();
        }
        tokio::select! {
            () = requests.closed() => return,
            () = tokio::time::sleep(backoff.next_delay()) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tokio::sync::mpsc;

    use super::super::simulated::SimulatedAudio;
    use super::super::state::Direction;
    use super::*;

    fn volume(seq: Sequence, delta: i64) -> Request {
        Request::Apply {
            seq,
            change: AudioChange::SinkVolume {
                sink: "s".into(),
                delta,
            },
            surface: ModuleSurface::Zone,
        }
    }

    #[test]
    fn consecutive_volume_changes_become_one() {
        let mute = Request::Apply {
            seq: 3,
            change: AudioChange::Mute {
                direction: Direction::Output,
                device: "s".into(),
                muted: true,
            },
            surface: ModuleSurface::Zone,
        };
        let jobs = merge(vec![
            volume(1, 10),
            Request::Refresh,
            volume(2, 5),
            mute,
            volume(4, -1),
        ]);
        let summary: Vec<_> = jobs
            .iter()
            .map(|job| (job.seq, job.change.clone()))
            .collect();
        assert_eq!(summary.len(), 3);
        assert_eq!(
            summary[0],
            (
                2,
                AudioChange::SinkVolume {
                    sink: "s".into(),
                    delta: 15
                }
            )
        );
        assert_eq!(summary[2].0, 4);
        assert!(merge(vec![Request::Refresh]).is_empty());
    }

    #[test]
    fn backoff_doubles_up_to_its_bound_and_resets() {
        let mut backoff = Backoff::default();
        let delays: Vec<_> = (0..7).map(|_| backoff.next_delay().as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30]);
        backoff.reset();
        assert_eq!(backoff.next_delay(), INITIAL_BACKOFF);
    }

    fn recording_sender(accept: bool) -> (EventSender, Arc<Mutex<usize>>) {
        let count = Arc::new(Mutex::new(0));
        let counter = Arc::clone(&count);
        let sender = EventSender::new(move |_| {
            *counter.lock().unwrap() += 1;
            accept
        });
        (sender, count)
    }

    #[tokio::test(start_paused = true)]
    async fn the_worker_stops_when_the_instance_refuses_events() {
        let (events, sent) = recording_sender(false);
        let (_requests, receiver) = mpsc::unbounded_channel();
        let worker = tokio::spawn(run_worker(
            Arc::new(SimulatedAudio::demo()),
            receiver,
            events,
        ));
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*sent.lock().unwrap(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn the_worker_stops_with_its_requests() {
        let (events, _) = recording_sender(true);
        let (requests, receiver) = mpsc::unbounded_channel::<Request>();
        let worker = tokio::spawn(run_worker(
            Arc::new(SimulatedAudio::demo()),
            receiver,
            events,
        ));
        drop(requests);
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_notifications_costs_one_read() {
        let audio = Arc::new(SimulatedAudio::demo());
        let (events, sent) = recording_sender(true);
        let (requests, receiver) = mpsc::unbounded_channel();
        let worker = tokio::spawn(run_worker(Arc::clone(&audio) as _, receiver, events));
        let watcher = tokio::spawn(watch_changes(Arc::clone(&audio) as _, requests));
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(audio.snapshot_reads(), 1);
        for _ in 0..5 {
            audio.change_externally(|state| state.sinks[0].muted = true);
        }
        tokio::time::sleep(REFRESH_DEBOUNCE * 4).await;
        assert_eq!(audio.snapshot_reads(), 2);
        assert_eq!(*sent.lock().unwrap(), 2);
        watcher.abort();
        worker.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn listening_is_retried_with_a_bounded_backoff() {
        let audio = Arc::new(SimulatedAudio::demo());
        audio.set_available(false);
        let (requests, mut receiver) = mpsc::unbounded_channel();
        let watcher = tokio::spawn(watch_changes(Arc::clone(&audio) as _, requests));
        let start = Instant::now();
        let attempts_at = |seconds: u64, attempts: usize| {
            let audio = Arc::clone(&audio);
            async move {
                tokio::time::sleep_until(start + Duration::from_millis(seconds * 1000 + 500)).await;
                assert_eq!(audio.subscriptions(), attempts, "after {seconds} s");
            }
        };
        // Attempts at 0, 1, 3, 7, 15, 31, 61, 91 s.
        attempts_at(0, 1).await;
        attempts_at(1, 2).await;
        attempts_at(3, 3).await;
        attempts_at(30, 5).await;
        attempts_at(61, 7).await;
        attempts_at(91, 8).await;
        // Every failed attempt asked for the state to be read again.
        let mut batch = Vec::new();
        drain(&mut receiver, &mut batch);
        assert_eq!(batch.len(), 8);

        audio.set_available(true);
        attempts_at(121, 9).await;
        audio.change_externally(|_| {});
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert!(matches!(receiver.try_recv(), Ok(Request::Refresh)));

        drop(receiver);
        tokio::time::timeout(Duration::from_secs(1), watcher)
            .await
            .unwrap()
            .unwrap();
    }
}
