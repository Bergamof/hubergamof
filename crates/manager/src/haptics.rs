//! Haptic events of the modules, on their way to the mice: the engine of a
//! device raises one (a step ends, a call rings), whoever drives haptic mice
//! plays it.

use std::sync::{Arc, OnceLock};

use hubergamof_module_api::HapticWaveform;
use hubergamof_protocol::ModuleKind;

/// An event of a module that a haptic mouse may signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HapticPlay {
    /// The module that raised it.
    pub module: ModuleKind,
    /// The event, one the module declares.
    pub event: String,
    /// What the module declares it is played as: what a mouse does unless its
    /// user chose otherwise.
    pub waveform: HapticWaveform,
}

impl HapticPlay {
    /// The key of the event in the haptic settings of a mouse:
    /// `module.event`.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}.{}", self.module, self.event)
    }
}

/// Where the haptic events of the modules go.
pub trait HapticOutput: Send + Sync {
    /// Signals `play`, never waiting.
    fn play(&self, play: &HapticPlay);
}

/// No mouse to signal on (`--demo`, the tests, the mice not served).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoHaptics;

impl HapticOutput for NoHaptics {
    fn play(&self, _: &HapticPlay) {}
}

/// An output connected later: the devices are started before the mice are
/// served, and both need to know each other.
#[derive(Default)]
pub struct HapticRelay {
    target: OnceLock<Arc<dyn HapticOutput>>,
}

impl HapticRelay {
    /// A relay that goes nowhere until [`HapticRelay::connect`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends what comes to `target` from now on. Only the first call counts.
    pub fn connect(&self, target: Arc<dyn HapticOutput>) {
        let _ = self.target.set(target);
    }
}

impl HapticOutput for HapticRelay {
    fn play(&self, play: &HapticPlay) {
        if let Some(target) = self.target.get() {
            target.play(play);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl HapticOutput for Recorder {
        fn play(&self, play: &HapticPlay) {
            self.0.lock().unwrap().push(play.key());
        }
    }

    fn play() -> HapticPlay {
        HapticPlay {
            module: "pomodoro".into(),
            event: "step-end".to_owned(),
            waveform: HapticWaveform::Completed,
        }
    }

    #[test]
    fn the_key_of_an_event_is_module_dot_event() {
        assert_eq!(play().key(), "pomodoro.step-end");
    }

    #[test]
    fn a_relay_that_is_not_connected_drops_the_event() {
        let relay = HapticRelay::new();
        relay.play(&play());
    }

    #[test]
    fn a_connected_relay_passes_the_events_on() {
        let relay = HapticRelay::new();
        let recorder = Arc::new(Recorder::default());
        relay.connect(Arc::clone(&recorder) as Arc<dyn HapticOutput>);
        relay.play(&play());
        assert_eq!(*recorder.0.lock().unwrap(), ["pomodoro.step-end"]);
    }

    #[test]
    fn only_the_first_connection_counts() {
        let relay = HapticRelay::new();
        let (first, second) = (Arc::new(Recorder::default()), Arc::new(Recorder::default()));
        relay.connect(Arc::clone(&first) as Arc<dyn HapticOutput>);
        relay.connect(Arc::clone(&second) as Arc<dyn HapticOutput>);
        relay.play(&play());
        assert_eq!(first.0.lock().unwrap().len(), 1);
        assert!(second.0.lock().unwrap().is_empty());
    }
}
