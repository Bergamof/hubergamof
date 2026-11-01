use std::path::Path;

use async_trait::async_trait;
use bergadeck_module_api::ModuleEvent;
use bergadeck_module_api::testing::TestContext;
use tokio::sync::mpsc::UnboundedReceiver;

use super::super::pactl::PactlBackend;
use super::super::simulated::SimulatedAudio;
use super::super::state::PartialSnapshot;
use super::super::state::{Direction, Volume, level_of_percent};
use super::*;

/// A subscriber of a hub, with what it receives.
struct Probe {
    subscription: Subscription,
    states: watch::Receiver<HubState>,
    outcomes: UnboundedReceiver<ModuleEvent>,
    _test: TestContext,
}

impl Probe {
    fn new(hub: &Arc<AudioHub>) -> Self {
        let mut test = TestContext::new("audio");
        let subscription = hub.subscribe(test.ctx.events().clone());
        let states = subscription.states();
        let outcomes = std::mem::replace(&mut test.events, mpsc::unbounded_channel().1);
        Self {
            subscription,
            states,
            outcomes,
            _test: test,
        }
    }

    fn state(&self) -> HubState {
        self.states.borrow().clone()
    }

    fn snapshot(&self) -> Arc<AudioSnapshot> {
        match self.state().server {
            ServerView::Ready(snapshot) => snapshot,
            other => panic!("state expected, got {other:?}"),
        }
    }

    fn outcomes(&mut self) -> Vec<(ModuleSurface, Feedback)> {
        let mut outcomes = Vec::new();
        while let Ok(event) = self.outcomes.try_recv() {
            if let Ok(event) = event.downcast::<AudioEvent>()
                && let AudioEvent::Outcome { surface, feedback } = *event
            {
                outcomes.push((surface, feedback));
            }
        }
        outcomes
    }

    fn set_volume(&self, percent: u32) -> Option<Sequence> {
        self.subscription.submit(
            AudioChange::SinkVolume {
                sink: "demo.speakers".into(),
                volume: Volume::uniform(level_of_percent(percent), 2),
            },
            ModuleSurface::Zone,
        )
    }
}

/// Lets the (paused) time pass for `duration`.
async fn idle(duration: Duration) {
    tokio::time::sleep(duration).await;
}

/// Long enough for every scheduled read to happen.
const SETTLE: Duration = Duration::from_secs(1);

fn hub_on(audio: &Arc<SimulatedAudio>) -> Arc<AudioHub> {
    AudioHub::new(Arc::clone(audio) as _)
}

fn job(seq: Sequence, subscriber: SubscriberId, change: AudioChange) -> Request {
    Request::Apply(Job {
        seq,
        subscriber,
        replies: EventSender::noop(),
        change,
        surface: ModuleSurface::Zone,
    })
}

fn volume(level: u32) -> AudioChange {
    AudioChange::SinkVolume {
        sink: "s".into(),
        volume: Volume::uniform(level, 2),
    }
}

// ---------------------------------------------------------------- Merging

#[test]
fn consecutive_changes_of_one_instance_become_one() {
    let mute = AudioChange::Mute {
        direction: Direction::Output,
        device: "s".into(),
        muted: true,
    };
    let merged = merge(vec![
        job(1, 7, volume(10)),
        Request::Refresh,
        job(2, 7, volume(15)),
        job(3, 7, mute),
        job(4, 7, volume(1)),
        job(5, 8, volume(2)),
    ]);
    assert!(merged.refresh);
    assert!(!merged.stop);
    let summary: Vec<_> = merged
        .jobs
        .iter()
        .map(|job| (job.seq, job.change.clone()))
        .collect();
    assert_eq!(summary.len(), 4, "{summary:?}");
    assert_eq!(summary[0], (2, volume(15)));
    assert_eq!(summary[2], (4, volume(1)));
    // Another instance's change is never merged with this one's.
    assert_eq!(summary[3], (5, volume(2)));
    assert!(merge(vec![Request::Refresh]).jobs.is_empty());
    assert!(merge(vec![job(1, 7, volume(10)), Request::Stop]).stop);
}

#[test]
fn backoff_doubles_up_to_its_bound_and_resets() {
    let mut backoff = Backoff::default();
    let delays: Vec<_> = (0..7).map(|_| backoff.next_delay().as_secs()).collect();
    assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30]);
    backoff.reset();
    assert_eq!(backoff.next_delay(), INITIAL_BACKOFF);
}

// ---------------------------------------------------------------- Sharing

#[tokio::test(start_paused = true)]
async fn instances_share_one_subscription_and_one_read_per_change() {
    let audio = Arc::new(SimulatedAudio::demo());
    let hub = hub_on(&audio);
    let first = Probe::new(&hub);
    let second = Probe::new(&hub);
    idle(SETTLE).await;
    assert_eq!(audio.subscriptions(), 1);
    assert_eq!(audio.listeners(), 1);
    assert_eq!(audio.snapshot_reads(), 1);

    audio.change_externally(|state| state.sinks[0].muted = true);
    idle(SETTLE).await;
    assert_eq!(audio.snapshot_reads(), 2);
    assert!(first.snapshot().sinks[0].muted);
    assert!(second.snapshot().sinks[0].muted);
}

#[tokio::test(start_paused = true)]
async fn changes_of_several_instances_are_applied_in_order() {
    let audio = Arc::new(SimulatedAudio::demo());
    let hub = hub_on(&audio);
    let (first, second) = (Probe::new(&hub), Probe::new(&hub));
    let a = first.set_volume(10).unwrap();
    let b = second.set_volume(20).unwrap();
    let c = first.set_volume(30).unwrap();
    assert!(a < b && b < c);
    idle(SETTLE).await;
    let applied: Vec<_> = audio
        .applied()
        .into_iter()
        .map(|change| match change {
            AudioChange::SinkVolume { volume, .. } => volume.percent(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(applied, [10, 20, 30]);
    assert_eq!(first.state().applied, c);
    assert_eq!(first.snapshot().sinks[0].volume.percent(), 30);
}

#[tokio::test(start_paused = true)]
async fn the_worker_ends_with_its_last_subscriber() {
    let audio = Arc::new(SimulatedAudio::demo());
    let hub = hub_on(&audio);
    let first = Probe::new(&hub);
    let second = Probe::new(&hub);
    idle(SETTLE).await;
    drop(first);
    idle(SETTLE).await;
    assert_eq!(audio.listeners(), 1, "still one subscriber");
    drop(second);
    idle(SETTLE).await;
    assert_eq!(audio.listeners(), 0, "subscription to the server dropped");
    let reads = audio.snapshot_reads();
    audio.change_externally(|_| {});
    idle(SETTLE).await;
    assert_eq!(audio.snapshot_reads(), reads);

    // A new subscriber starts a new worker, which reads at once.
    assert!(
        hub.lock().worker.is_none(),
        "the stopped worker is forgotten"
    );
    let _again = Probe::new(&hub);
    idle(SETTLE).await;
    assert_eq!(audio.subscriptions(), 2);
    assert_eq!(audio.snapshot_reads(), reads + 1);
}

#[tokio::test(start_paused = true)]
async fn changes_queued_by_the_last_subscriber_are_applied_before_the_end() {
    let audio = Arc::new(SimulatedAudio::demo());
    let hub = hub_on(&audio);
    let probe = Probe::new(&hub);
    idle(SETTLE).await;
    assert!(probe.set_volume(10).is_some());
    drop(probe);
    idle(SETTLE).await;
    assert_eq!(audio.state().sinks[0].volume.percent(), 10);
    assert_eq!(audio.listeners(), 0);
}

#[tokio::test(start_paused = true)]
async fn subscribing_again_before_the_end_keeps_the_same_worker() {
    let audio = Arc::new(SimulatedAudio::demo());
    let hub = hub_on(&audio);
    let first = Probe::new(&hub);
    idle(SETTLE).await;
    // Queued while the worker is busy elsewhere: it handles the stop only
    // after the new subscription, so it keeps serving.
    assert!(first.set_volume(10).is_some());
    drop(first);
    let second = Probe::new(&hub);
    assert!(second.set_volume(20).is_some());
    idle(SETTLE).await;

    assert_eq!(audio.subscriptions(), 1, "one worker throughout");
    let applied: Vec<_> = audio
        .applied()
        .into_iter()
        .map(|change| match change {
            AudioChange::SinkVolume { volume, .. } => volume.percent(),
            other => panic!("volume expected, got {other:?}"),
        })
        .collect();
    assert_eq!(applied, [10, 20], "applied once each, in order");
    assert_eq!(second.state().applied, hub.lock().last_seq);
}

// ---------------------------------------------------------------- Reads

#[tokio::test(start_paused = true)]
async fn a_change_reads_once_only_what_it_altered() {
    let audio = Arc::new(SimulatedAudio::demo());
    let hub = hub_on(&audio);
    let mut probe = Probe::new(&hub);
    idle(SETTLE).await;
    assert_eq!(audio.reads(), [ReadScope::ALL]);

    // The echo of the change is absorbed by the debounce.
    probe.set_volume(55).unwrap();
    idle(SETTLE).await;
    assert_eq!(audio.reads()[1..], [ReadScope::OUTPUTS]);

    let seq = probe
        .subscription
        .submit(
            AudioChange::Default {
                direction: Direction::Input,
                device: "demo.usb-mic".into(),
            },
            ModuleSurface::Zone,
        )
        .unwrap();
    idle(SETTLE).await;
    assert_eq!(audio.reads()[2..], [ReadScope::DEFAULTS]);
    assert_eq!(probe.state().applied, seq);
    assert_eq!(
        probe.snapshot().default_source.as_deref(),
        Some("demo.usb-mic")
    );
    assert_eq!(probe.outcomes(), [(ModuleSurface::Zone, Feedback::Ok)]);
}

#[tokio::test(start_paused = true)]
async fn a_burst_of_notifications_costs_one_read() {
    let audio = Arc::new(SimulatedAudio::demo());
    let hub = hub_on(&audio);
    let _probe = Probe::new(&hub);
    idle(SETTLE).await;
    for _ in 0..5 {
        audio.change_externally(|state| state.sinks[0].muted = true);
    }
    idle(SETTLE).await;
    assert_eq!(audio.snapshot_reads(), 2);

    // Notifications 10 ms apart, within the debounce: still one read.
    for _ in 0..4 {
        audio.change_externally(|_| {});
        idle(Duration::from_millis(10)).await;
    }
    idle(SETTLE).await;
    assert_eq!(audio.snapshot_reads(), 3);
}

#[tokio::test(start_paused = true)]
async fn continuous_changes_are_read_at_a_bounded_pace_and_to_the_end() {
    let audio = Arc::new(SimulatedAudio::demo());
    let hub = hub_on(&audio);
    let probe = Probe::new(&hub);
    idle(SETTLE).await;
    let before = audio.snapshot_reads();
    let burst = Duration::from_secs(3);
    let step = Duration::from_millis(20);
    let mut level = 0;
    let start = Instant::now();
    while start.elapsed() < burst {
        level += 1;
        audio.change_externally(|state| state.sinks[0].volume = Volume::uniform(level, 2));
        idle(step).await;
    }
    idle(SETTLE).await;
    let reads = audio.snapshot_reads() - before;
    let paced = burst.as_millis() / MIN_READ_INTERVAL.as_millis();
    assert!(
        reads >= 2 && u128::try_from(reads).unwrap() <= paced + 2,
        "{reads} reads"
    );
    // The last change is shown.
    assert_eq!(probe.snapshot().sinks[0].volume.channels(), [level; 2]);
}

// ---------------------------------------------------------------- Failures

#[tokio::test(start_paused = true)]
async fn listening_is_retried_with_a_bounded_backoff() {
    let audio = Arc::new(SimulatedAudio::demo());
    audio.set_available(false);
    let hub = hub_on(&audio);
    let probe = Probe::new(&hub);
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
    // Every failed attempt read the state again.
    assert_eq!(audio.snapshot_reads(), 8);
    assert_eq!(probe.state().server, ServerView::Unavailable);

    audio.set_available(true);
    attempts_at(121, 9).await;
    assert!(matches!(probe.state().server, ServerView::Ready(_)));
    audio.change_externally(|state| state.sinks[0].muted = true);
    idle(SETTLE).await;
    assert!(probe.snapshot().sinks[0].muted);
}

#[tokio::test(start_paused = true)]
async fn the_backoff_starts_over_after_a_stable_subscription() {
    let audio = Arc::new(SimulatedAudio::demo());
    let hub = hub_on(&audio);
    let _probe = Probe::new(&hub);
    let start = Instant::now();
    let at = |millis: u64| tokio::time::sleep_until(start + Duration::from_millis(millis));

    // A short outage: attempts at 1.5 s (fails), 3.5 s (succeeds).
    at(500).await;
    audio.set_available(false);
    at(2_000).await;
    assert_eq!(audio.subscriptions(), 2);
    audio.set_available(true);
    at(4_000).await;
    assert_eq!(audio.subscriptions(), 3);
    assert_eq!(audio.listeners(), 1);

    // Healthy for more than STABLE_SUBSCRIPTION: the next wait is 1 s, not 4.
    at(20_000).await;
    audio.set_available(false);
    audio.set_available(true);
    at(20_000 + u64::try_from(INITIAL_BACKOFF.as_millis()).unwrap() + 100).await;
    assert_eq!(audio.subscriptions(), 4);
    assert_eq!(audio.listeners(), 1);
}

/// The demo server, whose changes panic.
#[derive(Debug)]
struct PanickingChanges(SimulatedAudio);

#[async_trait]
impl AudioBackend for PanickingChanges {
    async fn read(&self, scope: ReadScope) -> Result<PartialSnapshot, AudioError> {
        self.0.read(scope).await
    }

    async fn apply(&self, _change: &AudioChange) -> Result<(), AudioError> {
        panic!("broken backend");
    }

    async fn subscribe(&self) -> Result<Box<dyn AudioChanges>, AudioError> {
        self.0.subscribe().await
    }
}

#[tokio::test(start_paused = true)]
async fn a_panicking_backend_fails_the_change_only() {
    let hub = AudioHub::new(Arc::new(PanickingChanges(SimulatedAudio::demo())));
    let mut probe = Probe::new(&hub);
    idle(SETTLE).await;
    let seq = probe.set_volume(90).unwrap();
    idle(SETTLE).await;
    assert_eq!(probe.outcomes(), [(ModuleSurface::Zone, Feedback::Alert)]);
    // The real state was read again, covering the failed change.
    assert_eq!(probe.state().applied, seq);
    assert_eq!(probe.snapshot().sinks[0].volume.percent(), 40);
    assert!(probe.set_volume(91).is_some(), "the worker still runs");
}

#[tokio::test(start_paused = true)]
async fn a_stopped_worker_is_started_again() {
    let audio = Arc::new(SimulatedAudio::demo());
    let hub = hub_on(&audio);
    let probe = Probe::new(&hub);
    idle(SETTLE).await;
    hub.crash_worker();
    idle(SETTLE).await;

    // The change found the worker stopped: it goes to the new one.
    assert!(probe.set_volume(10).is_some());
    idle(SETTLE).await;
    assert_eq!(audio.state().sinks[0].volume.percent(), 10);
    assert_eq!(probe.state().applied, hub.lock().last_seq);
    assert_eq!(probe.snapshot().sinks[0].volume.percent(), 10);
    assert_eq!(audio.listeners(), 1, "the stopped worker no longer listens");
}

#[test]
fn without_a_runtime_nothing_is_read() {
    let audio = Arc::new(SimulatedAudio::demo());
    let hub = hub_on(&audio);
    let probe = Probe::new(&hub);
    assert_eq!(probe.state().server, ServerView::Loading);
    assert_eq!(audio.snapshot_reads(), 0);
}

// ---------------------------------------------------------------- With pactl

/// A fake `pactl` logging its calls, one per line, in `calls.log`; its
/// `subscribe` follows `events.log`, to which every `set-*` call appends the
/// event the real server would send.
fn echoing_pactl(dir: &Path) -> PactlBackend {
    for (name, content) in [
        ("info.json", include_str!("../fixtures/pipewire-info.json")),
        (
            "sinks.json",
            include_str!("../fixtures/pipewire-sinks.json"),
        ),
        (
            "sources.json",
            include_str!("../fixtures/pipewire-sources.json"),
        ),
        ("events.log", ""),
    ] {
        std::fs::write(dir.join(name), content).unwrap();
    }
    let script = dir.join("pactl");
    std::fs::write(
        &script,
        format!(
            r#"cd '{}'
echo "$*" >> calls.log
case "$1" in
  subscribe) exec tail -n 0 -F events.log ;;
  --format=json) if [ "$2" = info ]; then cat info.json; else cat "$3.json"; fi ;;
  set-*) echo "Event 'change' on sink #54" >> events.log ;;
esac
"#,
            dir.display()
        ),
    )
    .unwrap();
    PactlBackend::shell_script(script)
}

fn calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Waits (real time) until `condition` holds.
async fn eventually(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition not reached");
}

#[tokio::test]
async fn a_volume_notch_costs_two_pactl_calls() {
    let dir = tempfile::tempdir().unwrap();
    let hub = AudioHub::new(Arc::new(echoing_pactl(dir.path())));
    let probe = Probe::new(&hub);
    eventually(|| matches!(probe.state().server, ServerView::Ready(_))).await;
    eventually(|| calls(dir.path()).len() == 4).await;
    let sink = probe.snapshot().sinks[0].name.clone();

    let seq = probe
        .subscription
        .submit(
            AudioChange::SinkVolume {
                sink,
                volume: Volume::uniform(level_of_percent(30), 2),
            },
            ModuleSurface::Zone,
        )
        .unwrap();
    eventually(|| probe.state().applied == seq).await;
    // Leave time to a read the echo would cause.
    tokio::time::sleep(MIN_READ_INTERVAL * 3).await;
    let calls = calls(dir.path());
    let round_trip = &calls[4..];
    // The echo of `subscribe` usually lands within the debounce (two calls);
    // on a loaded machine it may come after the read and cost one more.
    assert!(
        (2..=3).contains(&round_trip.len()) && round_trip[0].starts_with("set-sink-volume "),
        "{round_trip:?}"
    );
    assert!(
        round_trip[1..]
            .iter()
            .all(|call| call == "--format=json list sinks")
    );
}
