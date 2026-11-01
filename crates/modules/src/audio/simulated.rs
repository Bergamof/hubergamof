//! In-memory [`AudioBackend`]: the hermetic audio server of `--demo` and of
//! the tests. It never touches the audio of the machine.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use tokio::sync::broadcast;

use super::backend::{AudioBackend, AudioChange, AudioChanges, AudioError, ReadScope};
use super::state::{
    AudioDevice, AudioSnapshot, Defaults, Direction, PartialSnapshot, Volume, level_of_percent,
};

/// Room for change notifications not yet read; a subscriber that lags
/// behind reads everything again.
const CHANGES_CAPACITY: usize = 16;

/// Changes and reads a [`SimulatedAudio`] remembers for inspection; older
/// ones are forgotten, so that a long demo does not grow without bound.
pub(crate) const SIMULATED_HISTORY_CAPACITY: usize = 64;

/// Channels of the demo devices.
const STEREO: usize = 2;

/// What subscribers are told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Notice {
    Changed(ReadScope),
    /// The server "restarted": current subscriptions end.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only the tests take the server away")
    )]
    Disconnected,
}

/// The last [`SIMULATED_HISTORY_CAPACITY`] items of a kind.
#[derive(Debug)]
struct History<T>(Mutex<VecDeque<T>>);

impl<T: Clone> History<T> {
    fn new() -> Self {
        Self(Mutex::new(VecDeque::with_capacity(
            SIMULATED_HISTORY_CAPACITY,
        )))
    }

    fn push(&self, item: T) {
        let mut items = lock(&self.0);
        if items.len() == SIMULATED_HISTORY_CAPACITY {
            items.pop_front();
        }
        items.push_back(item);
    }

    #[cfg(test)]
    fn items(&self) -> Vec<T> {
        lock(&self.0).iter().cloned().collect()
    }
}

/// Simulated audio server.
#[derive(Debug)]
pub struct SimulatedAudio {
    state: Mutex<AudioSnapshot>,
    notices: broadcast::Sender<Notice>,
    available: AtomicBool,
    applied: History<AudioChange>,
    reads: History<ReadScope>,
    subscriptions: AtomicUsize,
    snapshot_reads: AtomicUsize,
}

impl SimulatedAudio {
    /// Server holding `snapshot`.
    #[must_use]
    pub fn new(snapshot: AudioSnapshot) -> Self {
        Self {
            state: Mutex::new(snapshot),
            notices: broadcast::channel(CHANGES_CAPACITY).0,
            available: AtomicBool::new(true),
            applied: History::new(),
            reads: History::new(),
            subscriptions: AtomicUsize::new(0),
            snapshot_reads: AtomicUsize::new(0),
        }
    }

    /// Server of the demo: three outputs and two inputs.
    #[must_use]
    pub fn demo() -> Self {
        let device = |name: &str, description: &str, percent: u32| AudioDevice {
            name: name.into(),
            description: description.into(),
            volume: Volume::uniform(level_of_percent(percent), STEREO),
            muted: false,
        };
        Self::new(AudioSnapshot {
            sinks: vec![
                device("demo.speakers", "Haut-parleurs", 40),
                device("demo.headset", "Casque Bluetooth", 25),
                device("demo.hdmi", "Écran HDMI", 100),
            ],
            sources: vec![
                device("demo.internal-mic", "Micro interne", 70),
                device("demo.usb-mic", "Micro USB", 85),
            ],
            default_sink: Some("demo.speakers".into()),
            default_source: Some("demo.internal-mic".into()),
        })
    }

    /// Current state.
    #[must_use]
    pub fn state(&self) -> AudioSnapshot {
        lock(&self.state).clone()
    }

    fn notify(&self, notice: Notice) {
        // Without subscriber, nobody needs telling.
        let _ = self.notices.send(notice);
    }

    fn check_available(&self) -> Result<(), AudioError> {
        if self.available.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(AudioError::Unavailable)
        }
    }

    fn change_state(state: &mut AudioSnapshot, change: &AudioChange) -> Result<(), AudioError> {
        let failure = |reason: &str| AudioError::Failed {
            command: format!("{change:?}"),
            status: "simulated".into(),
            stderr: reason.into(),
        };
        let missing = || failure("No such entity");
        match change {
            AudioChange::SinkVolume { sink, volume } => {
                let device = state
                    .device_mut(Direction::Output, sink)
                    .ok_or_else(missing)?;
                // Like the real server: one volume per channel.
                if device.volume.channels().len() != volume.channels().len() {
                    return Err(failure("Invalid argument"));
                }
                device.volume = volume.clone();
            }
            AudioChange::Mute {
                direction,
                device,
                muted,
            } => {
                state
                    .device_mut(*direction, device)
                    .ok_or_else(missing)?
                    .muted = *muted;
            }
            AudioChange::Default { direction, device } => {
                state.device_mut(*direction, device).ok_or_else(missing)?;
                state.set_default(*direction, device);
            }
        }
        Ok(())
    }
}

/// Inspection and control, for the tests of the modules.
#[cfg(test)]
impl SimulatedAudio {
    /// The last changes applied, oldest first.
    #[must_use]
    pub(crate) fn applied(&self) -> Vec<AudioChange> {
        self.applied.items()
    }

    /// What the last reads asked for, oldest first.
    #[must_use]
    pub(crate) fn reads(&self) -> Vec<ReadScope> {
        self.reads.items()
    }

    /// Number of subscriptions started so far.
    #[must_use]
    pub(crate) fn subscriptions(&self) -> usize {
        self.subscriptions.load(Ordering::SeqCst)
    }

    /// Number of reads so far.
    #[must_use]
    pub(crate) fn snapshot_reads(&self) -> usize {
        self.snapshot_reads.load(Ordering::SeqCst)
    }

    /// Number of subscriptions currently listening.
    #[must_use]
    pub(crate) fn listeners(&self) -> usize {
        self.notices.receiver_count()
    }

    /// Changes the state as another program would, and notifies.
    pub(crate) fn change_externally(&self, change: impl FnOnce(&mut AudioSnapshot)) {
        change(&mut lock(&self.state));
        self.notify(Notice::Changed(ReadScope::ALL));
    }

    /// Makes every call fail (`false`) or work again (`true`); going away
    /// ends the subscriptions.
    pub(crate) fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::SeqCst);
        if !available {
            self.notify(Notice::Disconnected);
        }
    }
}

#[async_trait]
impl AudioBackend for SimulatedAudio {
    async fn read(&self, scope: ReadScope) -> Result<PartialSnapshot, AudioError> {
        self.snapshot_reads.fetch_add(1, Ordering::SeqCst);
        self.reads.push(scope);
        self.check_available()?;
        let state = self.state();
        Ok(PartialSnapshot {
            sinks: scope.outputs.then_some(state.sinks),
            sources: scope.inputs.then_some(state.sources),
            defaults: scope.defaults.then_some(Defaults {
                sink: state.default_sink,
                source: state.default_source,
            }),
        })
    }

    async fn apply(&self, change: &AudioChange) -> Result<(), AudioError> {
        self.check_available()?;
        Self::change_state(&mut lock(&self.state), change)?;
        self.applied.push(change.clone());
        self.notify(Notice::Changed(change.scope()));
        Ok(())
    }

    async fn subscribe(&self) -> Result<Box<dyn AudioChanges>, AudioError> {
        self.subscriptions.fetch_add(1, Ordering::SeqCst);
        self.check_available()?;
        Ok(Box::new(SimulatedChanges(self.notices.subscribe())))
    }
}

struct SimulatedChanges(broadcast::Receiver<Notice>);

#[async_trait]
impl AudioChanges for SimulatedChanges {
    async fn next_change(&mut self) -> Option<ReadScope> {
        match self.0.recv().await {
            Ok(Notice::Changed(scope)) => Some(scope),
            Err(broadcast::error::RecvError::Lagged(_)) => Some(ReadScope::ALL),
            Ok(Notice::Disconnected) | Err(broadcast::error::RecvError::Closed) => None,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every update is a single assignment: a poisoned lock is still valid.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::super::state::VOLUME_NORM;
    use super::*;

    #[tokio::test]
    async fn changes_apply_and_notify_what_they_alter() {
        let audio = SimulatedAudio::demo();
        let mut changes = audio.subscribe().await.unwrap();
        let quieter = Volume::uniform(level_of_percent(30), STEREO);
        audio
            .apply(&AudioChange::SinkVolume {
                sink: "demo.speakers".into(),
                volume: quieter.clone(),
            })
            .await
            .unwrap();
        assert_eq!(changes.next_change().await, Some(ReadScope::OUTPUTS));
        let read = audio.read(ReadScope::OUTPUTS).await.unwrap();
        assert_eq!(read.sinks.unwrap()[0].volume, quieter);
        assert_eq!((read.sources, read.defaults), (None, None));
        audio
            .apply(&AudioChange::Default {
                direction: Direction::Input,
                device: "demo.usb-mic".into(),
            })
            .await
            .unwrap();
        assert_eq!(changes.next_change().await, Some(ReadScope::DEFAULTS));
        assert_eq!(
            audio.state().default_source.as_deref(),
            Some("demo.usb-mic")
        );
        assert_eq!(audio.applied().len(), 2);
        assert_eq!(audio.reads(), [ReadScope::OUTPUTS]);
        assert_eq!(audio.state().sinks[2].volume.channels()[0], VOLUME_NORM);
    }

    #[tokio::test]
    async fn unknown_devices_wrong_channels_and_an_absent_server_fail() {
        let audio = SimulatedAudio::demo();
        let unknown = AudioChange::Default {
            direction: Direction::Output,
            device: "nope".into(),
        };
        assert!(audio.apply(&unknown).await.is_err());
        let mono = AudioChange::SinkVolume {
            sink: "demo.speakers".into(),
            volume: Volume::uniform(1, 1),
        };
        assert!(audio.apply(&mono).await.is_err());
        let mut changes = audio.subscribe().await.unwrap();
        assert_eq!(audio.listeners(), 1);
        audio.set_available(false);
        assert_eq!(changes.next_change().await, None);
        assert!(audio.read(ReadScope::ALL).await.is_err());
        assert!(audio.subscribe().await.is_err());
        assert_eq!(audio.subscriptions(), 2);
        audio.set_available(true);
        assert!(audio.read(ReadScope::ALL).await.is_ok());
    }

    #[tokio::test]
    async fn histories_keep_only_the_last_entries() {
        let audio = SimulatedAudio::demo();
        for _ in 0..SIMULATED_HISTORY_CAPACITY + 3 {
            audio.read(ReadScope::INPUTS).await.unwrap();
        }
        assert_eq!(audio.reads().len(), SIMULATED_HISTORY_CAPACITY);
        assert_eq!(audio.snapshot_reads(), SIMULATED_HISTORY_CAPACITY + 3);
    }
}
