//! What a pomodoro keeps across a restart of the manager, and where.
//!
//! The state is written to `$XDG_RUNTIME_DIR/hubergamof/` (see
//! [`crate::runtime_files`]): that folder lives as long as the user's session
//! runs, so a restart of the manager finds the state again and a reboot of the
//! machine does not. The file also carries the boot identifier of the
//! machine, and is ignored under any other: the time of the pomodoro is
//! `CLOCK_BOOTTIME` ([`crate::time`]), which means nothing after a reboot.
//!
//! A step running when the manager stopped keeps running by its deadline: the
//! time the manager was down counts, as a suspend of the machine does.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::timer::{Countdown, Phase, TimerState};
use crate::runtime_files::PrivateFolder;
use crate::time::ClockInstant;

/// Version of the file format: a file of another version is ignored.
const FORMAT_VERSION: u32 = 1;

/// Longest state file read: the real one is a few hundred bytes.
const MAX_FILE_LENGTH: u64 = 4096;

/// Longest identifier of an instance that is kept (the bound of the
/// configuration).
const MAX_INSTANCE_ID_LENGTH: usize = 64;

/// Where the kernel gives the identifier of the current boot.
const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";

/// A saved [`TimerState`], in a form that can be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedTimer {
    phase: Phase,
    pomodoro: u32,
    awaiting_start: bool,
    countdown: SavedCountdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
enum SavedCountdown {
    #[serde(rename_all = "camelCase")]
    Paused { remaining_ms: u64 },
    #[serde(rename_all = "camelCase")]
    Running { deadline_ms: u64 },
}

impl From<TimerState> for SavedTimer {
    fn from(state: TimerState) -> Self {
        let countdown = match state.countdown {
            Countdown::Paused { remaining } => SavedCountdown::Paused {
                remaining_ms: millis(remaining),
            },
            Countdown::Running { deadline } => SavedCountdown::Running {
                deadline_ms: millis(deadline.since_origin()),
            },
        };
        Self {
            phase: state.phase,
            pomodoro: state.pomodoro,
            awaiting_start: state.awaiting_start,
            countdown,
        }
    }
}

impl From<SavedTimer> for TimerState {
    fn from(saved: SavedTimer) -> Self {
        let countdown = match saved.countdown {
            SavedCountdown::Paused { remaining_ms } => Countdown::Paused {
                remaining: Duration::from_millis(remaining_ms),
            },
            SavedCountdown::Running { deadline_ms } => Countdown::Running {
                deadline: ClockInstant::from_origin(Duration::from_millis(deadline_ms)),
            },
        };
        Self {
            phase: saved.phase,
            pomodoro: saved.pomodoro,
            awaiting_start: saved.awaiting_start,
            countdown,
        }
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Keeps the state of the pomodoros, by instance identifier.
pub trait PomodoroStateStore: Send + Sync + fmt::Debug {
    /// The state saved for `instance` that is still valid, if any. Reads a
    /// file of a few hundred bytes: called once, when the instance is created.
    fn load(&self, instance: &str) -> Option<SavedTimer>;

    /// Saves `state` for `instance`. Never waits for the disk; a failure is
    /// logged.
    fn save(&self, instance: &str, state: SavedTimer);
}

/// State kept in memory only: simulations, tests, and the fallback when there
/// is no runtime folder. It still survives a device being unplugged.
#[derive(Debug, Default)]
pub struct MemoryStateStore(Mutex<HashMap<String, SavedTimer>>);

impl MemoryStateStore {
    fn states(&self) -> std::sync::MutexGuard<'_, HashMap<String, SavedTimer>> {
        // A map of plain values is never left half-updated.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl PomodoroStateStore for MemoryStateStore {
    fn load(&self, instance: &str) -> Option<SavedTimer> {
        self.states().get(instance).copied()
    }

    fn save(&self, instance: &str, state: SavedTimer) {
        self.states().insert(instance.to_owned(), state);
    }
}

/// What is written in the file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    version: u32,
    boot_id: String,
    timer: SavedTimer,
}

/// State kept in the private folder of the runtime directory, written in order
/// by a thread of its own so that no handler waits for the disk.
pub struct FileStateStore {
    folder: Arc<PrivateFolder>,
    boot_id: String,
    writes: Sender<(String, Vec<u8>)>,
}

impl fmt::Debug for FileStateStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileStateStore")
            .field("boot_id", &self.boot_id)
            .finish_non_exhaustive()
    }
}

impl FileStateStore {
    /// The store of the user's session: `$XDG_RUNTIME_DIR` and the boot
    /// identifier of the machine.
    ///
    /// # Errors
    ///
    /// What is missing or refused (no runtime directory, no boot identifier,
    /// a folder that is not private).
    pub fn of_session() -> Result<Self, String> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .ok_or_else(|| "XDG_RUNTIME_DIR is not set".to_owned())?;
        let boot_id = std::fs::read_to_string(BOOT_ID_PATH)
            .map_err(|error| format!("{BOOT_ID_PATH}: {error}"))?;
        Self::new(Path::new(&runtime), boot_id.trim())
    }

    /// A store in `runtime_dir`, for the boot `boot_id`.
    ///
    /// # Errors
    ///
    /// The folder cannot be opened or is not private, or `boot_id` is empty.
    pub fn new(runtime_dir: &Path, boot_id: &str) -> Result<Self, String> {
        if boot_id.is_empty() {
            return Err("empty boot identifier".to_owned());
        }
        let folder = Arc::new(PrivateFolder::open(runtime_dir)?);
        let (writes, queued) = mpsc::channel::<(String, Vec<u8>)>();
        let thread_folder = Arc::clone(&folder);
        std::thread::Builder::new()
            .name("pomodoro-state".to_owned())
            .spawn(move || {
                // Ends when the store, which holds the sender, is dropped.
                for (name, content) in queued {
                    if let Err(error) = thread_folder.write_atomic(&name, &content) {
                        tracing::warn!(%error, "pomodoro state not saved");
                    }
                }
            })
            .map_err(|error| format!("cannot start the writer: {error}"))?;
        Ok(Self {
            folder,
            boot_id: boot_id.to_owned(),
            writes,
        })
    }
}

/// Name of the file of `instance`, `None` for an identifier that is not
/// plain (the configuration only allows plain ones).
fn file_name(instance: &str) -> Option<String> {
    let plain = !instance.is_empty()
        && instance.len() <= MAX_INSTANCE_ID_LENGTH
        && instance
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    plain.then(|| format!("pomodoro-{instance}.json"))
}

impl PomodoroStateStore for FileStateStore {
    fn load(&self, instance: &str) -> Option<SavedTimer> {
        let name = file_name(instance)?;
        let content = match self.folder.read(&name, MAX_FILE_LENGTH) {
            Ok(Some(content)) => content,
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(%error, "pomodoro state ignored");
                return None;
            }
        };
        let envelope: Envelope = serde_json::from_slice(&content)
            .inspect_err(|error| {
                // Only where it is wrong: the message may quote the file.
                tracing::warn!(
                    line = error.line(),
                    column = error.column(),
                    "pomodoro state ignored: unreadable"
                );
            })
            .ok()?;
        (envelope.version == FORMAT_VERSION && envelope.boot_id == self.boot_id)
            .then_some(envelope.timer)
    }

    fn save(&self, instance: &str, state: SavedTimer) {
        let Some(name) = file_name(instance) else {
            tracing::debug!("pomodoro state not saved: the instance identifier is not plain");
            return;
        };
        let envelope = Envelope {
            version: FORMAT_VERSION,
            boot_id: self.boot_id.clone(),
            timer: state,
        };
        match serde_json::to_vec(&envelope) {
            Ok(content) => {
                // The writer outlives the store; a send fails only at exit.
                let _ = self.writes.send((name, content));
            }
            Err(error) => tracing::warn!(%error, "pomodoro state not saved"),
        }
    }
}

/// The store of the user's session, or the memory when the session has no
/// usable runtime folder (logged).
#[must_use]
pub fn session_store() -> Arc<dyn PomodoroStateStore> {
    match FileStateStore::of_session() {
        Ok(store) => Arc::new(store),
        Err(error) => {
            tracing::warn!(%error, "the pomodoro state will not survive a restart of the manager");
            Arc::new(MemoryStateStore::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::runtime_files::testing::runtime_dir;

    const BOOT: &str = "boot-1";

    fn running() -> SavedTimer {
        TimerState {
            phase: Phase::ShortBreak,
            pomodoro: 3,
            awaiting_start: false,
            countdown: Countdown::Running {
                deadline: ClockInstant::from_origin(Duration::from_millis(123_456)),
            },
        }
        .into()
    }

    fn paused() -> SavedTimer {
        TimerState {
            phase: Phase::Work,
            pomodoro: 1,
            awaiting_start: true,
            countdown: Countdown::Paused {
                remaining: Duration::from_millis(1500),
            },
        }
        .into()
    }

    /// Waits for the writer thread to have written `instance`.
    fn loaded(store: &FileStateStore, instance: &str) -> Option<SavedTimer> {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            if let Some(state) = store.load(instance) {
                return Some(state);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }

    #[test]
    fn a_state_becomes_a_timer_state_and_back() {
        for saved in [running(), paused()] {
            assert_eq!(SavedTimer::from(TimerState::from(saved)), saved);
        }
    }

    #[test]
    fn the_memory_keeps_the_last_state_of_each_instance() {
        let store = MemoryStateStore::default();
        assert_eq!(store.load("a"), None);
        store.save("a", running());
        store.save("a", paused());
        store.save("b", running());
        assert_eq!(store.load("a"), Some(paused()));
        assert_eq!(store.load("b"), Some(running()));
    }

    #[test]
    fn a_state_written_in_a_boot_is_found_again_in_that_boot_only() {
        let runtime = runtime_dir();
        let store = FileStateStore::new(runtime.path(), BOOT).unwrap();
        store.save("pomodoro", running());
        assert_eq!(loaded(&store, "pomodoro"), Some(running()));

        let same_boot = FileStateStore::new(runtime.path(), BOOT).unwrap();
        assert_eq!(same_boot.load("pomodoro"), Some(running()));
        let other_boot = FileStateStore::new(runtime.path(), "boot-2").unwrap();
        assert_eq!(other_boot.load("pomodoro"), None);
    }

    #[test]
    fn the_last_write_wins_and_instances_are_apart() {
        let runtime = runtime_dir();
        let store = FileStateStore::new(runtime.path(), BOOT).unwrap();
        store.save("a", running());
        store.save("a", paused());
        store.save("b", running());
        assert_eq!(loaded(&store, "b"), Some(running()));
        // Writes are in order: once `b` is there, so is the last write of `a`.
        assert_eq!(store.load("a"), Some(paused()));
    }

    #[test]
    fn damaged_files_and_other_versions_are_ignored() {
        let runtime = runtime_dir();
        let store = FileStateStore::new(runtime.path(), BOOT).unwrap();
        let folder = PrivateFolder::open(runtime.path()).unwrap();
        folder.write_new("pomodoro-bad.json", b"{not json").unwrap();
        let future = br#"{"version":99,"bootId":"boot-1","timer":{}}"#;
        folder.write_new("pomodoro-future.json", future).unwrap();
        assert_eq!(store.load("bad"), None);
        assert_eq!(store.load("future"), None);
    }

    #[test]
    fn identifiers_that_are_not_plain_are_neither_saved_nor_loaded() {
        let runtime = runtime_dir();
        let store = FileStateStore::new(runtime.path(), BOOT).unwrap();
        for instance in ["", "../x", "a/b", "a b", &"x".repeat(65)] {
            store.save(instance, running());
            assert_eq!(store.load(instance), None, "{instance:?}");
        }
        assert!(FileStateStore::new(runtime.path(), "").is_err());
    }

    #[test]
    fn the_file_is_a_small_private_json() {
        let runtime = runtime_dir();
        let store = FileStateStore::new(runtime.path(), BOOT).unwrap();
        store.save("pomodoro", paused());
        assert!(loaded(&store, "pomodoro").is_some());
        let path = runtime.path().join("hubergamof/pomodoro-pomodoro.json");
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains(r#""state":"paused""#), "{text}");
        assert!(text.len() < 300, "{text}");
    }
}
