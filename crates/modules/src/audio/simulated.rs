//! In-memory [`AudioBackend`]: the hermetic audio server of `--demo` and of
//! the tests. It never touches the audio of the machine.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use tokio::sync::broadcast;

use super::backend::{AudioBackend, AudioChange, AudioChanges, AudioError};
use super::state::{AudioDevice, AudioSnapshot, Direction, Volume, level_of_percent};

/// Room for change notifications not yet read; a subscriber that lags
/// behind only sees one change.
const CHANGES_CAPACITY: usize = 16;

/// Channels of the demo devices.
const STEREO: usize = 2;

/// What subscribers are told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Notice {
    Changed,
    /// The server "restarted": current subscriptions end.
    Disconnected,
}

/// Simulated audio server.
#[derive(Debug)]
pub struct SimulatedAudio {
    state: Mutex<AudioSnapshot>,
    notices: broadcast::Sender<Notice>,
    available: AtomicBool,
    applied: Mutex<Vec<AudioChange>>,
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
            applied: Mutex::new(Vec::new()),
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

    /// Changes applied so far, in order.
    #[must_use]
    pub fn applied(&self) -> Vec<AudioChange> {
        lock(&self.applied).clone()
    }

    /// Number of subscriptions started so far.
    #[must_use]
    pub fn subscriptions(&self) -> usize {
        self.subscriptions.load(Ordering::SeqCst)
    }

    /// Number of state reads so far.
    #[must_use]
    pub fn snapshot_reads(&self) -> usize {
        self.snapshot_reads.load(Ordering::SeqCst)
    }

    /// Changes the state as another program would, and notifies.
    pub fn change_externally(&self, change: impl FnOnce(&mut AudioSnapshot)) {
        change(&mut lock(&self.state));
        self.notify(Notice::Changed);
    }

    /// Makes every call fail (`false`) or work again (`true`); going away
    /// ends the subscriptions.
    pub fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::SeqCst);
        if !available {
            self.notify(Notice::Disconnected);
        }
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
        let missing = || AudioError::Failed {
            command: format!("{change:?}"),
            status: "simulated".into(),
            stderr: "No such entity".into(),
        };
        match change {
            AudioChange::SinkVolume { sink, delta } => {
                let device = state
                    .device_mut(Direction::Output, sink)
                    .ok_or_else(missing)?;
                device.volume = device.volume.shifted(*delta);
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

impl Default for SimulatedAudio {
    fn default() -> Self {
        Self::demo()
    }
}

#[async_trait]
impl AudioBackend for SimulatedAudio {
    async fn snapshot(&self) -> Result<AudioSnapshot, AudioError> {
        self.snapshot_reads.fetch_add(1, Ordering::SeqCst);
        self.check_available()?;
        Ok(self.state())
    }

    async fn apply(&self, change: &AudioChange) -> Result<(), AudioError> {
        self.check_available()?;
        Self::change_state(&mut lock(&self.state), change)?;
        lock(&self.applied).push(change.clone());
        self.notify(Notice::Changed);
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
    async fn changed(&mut self) -> bool {
        match self.0.recv().await {
            Ok(Notice::Changed) | Err(broadcast::error::RecvError::Lagged(_)) => true,
            Ok(Notice::Disconnected) | Err(broadcast::error::RecvError::Closed) => false,
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
    async fn changes_apply_and_notify_subscribers() {
        let audio = SimulatedAudio::demo();
        let mut changes = audio.subscribe().await.unwrap();
        audio
            .apply(&AudioChange::SinkVolume {
                sink: "demo.speakers".into(),
                delta: -1000,
            })
            .await
            .unwrap();
        assert!(changes.changed().await);
        let state = audio.snapshot().await.unwrap();
        assert_eq!(
            state.sinks[0].volume.channels(),
            [level_of_percent(40) - 1000; 2]
        );
        audio
            .apply(&AudioChange::Default {
                direction: Direction::Input,
                device: "demo.usb-mic".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            audio.state().default_source.as_deref(),
            Some("demo.usb-mic")
        );
        assert_eq!(audio.applied().len(), 2);
        assert_eq!(audio.state().sinks[2].volume.channels()[0], VOLUME_NORM);
    }

    #[tokio::test]
    async fn unknown_devices_and_an_absent_server_fail() {
        let audio = SimulatedAudio::demo();
        let unknown = AudioChange::Default {
            direction: Direction::Output,
            device: "nope".into(),
        };
        assert!(audio.apply(&unknown).await.is_err());
        let mut changes = audio.subscribe().await.unwrap();
        audio.set_available(false);
        assert!(!changes.changed().await);
        assert!(audio.snapshot().await.is_err());
        assert!(audio.subscribe().await.is_err());
        assert_eq!(audio.subscriptions(), 2);
        audio.set_available(true);
        assert!(audio.snapshot().await.is_ok());
    }
}
