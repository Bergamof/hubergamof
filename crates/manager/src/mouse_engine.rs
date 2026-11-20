//! Logitech mice: a thread per connected mouse that applies its configuration
//! and follows its diverted buttons, and the supervisor that finds the mice
//! and starts those threads.
//!
//! Mice come and go (sleep, battery, receiver unplugged), so the supervisor
//! looks again every [`SCAN_INTERVAL`] for the ones it does not serve yet; a
//! thread that ends (the mouse left, the channel failed) frees its place for
//! the next scan, which is the reconnection. Every thread gives the buttons
//! back before it ends.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use hubergamof_mouse::channel::Channel;
use hubergamof_mouse::hidraw::{FoundMouse, HidrawChannel, MouseKey, find_mice};
use hubergamof_mouse::session::{MouseSession, Notice};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::config_store::ConfigStore;

/// Delay between two searches for mice not served yet.
pub const SCAN_INTERVAL: Duration = Duration::from_secs(5);

/// Where mice come from: the hardware, or scripted devices in the tests.
pub trait MouseSource: Send + 'static {
    /// The channel of the mice it finds.
    type Channel: Channel + Send + 'static;

    /// The mice not in `owned` (places already served) that answer now.
    fn find(&mut self, owned: &[MouseKey]) -> Vec<FoundMouse<Self::Channel>>;
}

/// The mice of the Logitech receivers and devices of this computer.
pub struct HidrawSource;

impl MouseSource for HidrawSource {
    type Channel = HidrawChannel;

    fn find(&mut self, owned: &[MouseKey]) -> Vec<FoundMouse<HidrawChannel>> {
        match find_mice(owned) {
            Ok((found, setup_error)) => {
                if let Some(error) = setup_error {
                    // At every scan while the mouse stays unusable: not a warning.
                    tracing::debug!(%error, "a mouse answered but cannot be set up");
                }
                found
            }
            Err(error) => {
                tracing::debug!(%error, "cannot look for mice");
                Vec::new()
            }
        }
    }
}

/// Places a thread serves now.
#[derive(Clone, Default)]
struct Running(Arc<Mutex<HashSet<MouseKey>>>);

impl Running {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<MouseKey>> {
        // Plain data: nothing to recover from a poisoned lock.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn snapshot(&self) -> Vec<MouseKey> {
        self.lock().iter().cloned().collect()
    }

    /// Marks a place as served until the returned guard is dropped (the
    /// thread ends, panics included).
    fn claim(&self, key: MouseKey) -> RunningGuard {
        self.lock().insert(key.clone());
        RunningGuard {
            running: self.clone(),
            key,
        }
    }
}

struct RunningGuard {
    running: Running,
    key: MouseKey,
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.running.lock().remove(&self.key);
    }
}

/// Finds the mice and serves each one, until [`MouseSupervisor::shutdown`].
pub struct MouseSupervisor {
    stop_flag: Arc<AtomicBool>,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl MouseSupervisor {
    /// Starts looking for mice every `scan_interval`.
    #[must_use]
    pub fn start<S: MouseSource>(source: S, store: ConfigStore, scan_interval: Duration) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(supervise(
            source,
            store,
            scan_interval,
            stopped,
            Arc::clone(&stop_flag),
        ));
        Self {
            stop_flag,
            stop,
            task,
        }
    }

    /// Stops every mouse thread, which gives its buttons back, and waits for
    /// them.
    pub async fn shutdown(self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        let _ = self.stop.send(());
        if let Err(error) = self.task.await {
            tracing::warn!(%error, "mouse supervisor failed");
        }
    }
}

async fn supervise<S: MouseSource>(
    source: S,
    store: ConfigStore,
    scan_interval: Duration,
    mut stopped: oneshot::Receiver<()>,
    stop_flag: Arc<AtomicBool>,
) {
    let source = Arc::new(Mutex::new(source));
    let running = Running::default();
    let mut threads: Vec<thread::JoinHandle<()>> = Vec::new();
    loop {
        let owned = running.snapshot();
        let scanning = Arc::clone(&source);
        let scan = tokio::task::spawn_blocking(move || {
            scanning
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .find(&owned)
        });
        let found = tokio::select! {
            _ = &mut stopped => break,
            found = scan => found,
        };
        match found {
            Ok(found) => {
                threads.retain(|thread| !thread.is_finished());
                for mouse in found {
                    threads.extend(spawn_mouse(mouse, &running, &store, &stop_flag));
                }
            }
            Err(error) => tracing::warn!(%error, "searching for mice failed"),
        }
        tokio::select! {
            _ = &mut stopped => break,
            () = tokio::time::sleep(scan_interval) => {}
        }
    }
    stop_flag.store(true, Ordering::Relaxed);
    let joined = tokio::task::spawn_blocking(move || {
        for thread in threads {
            let _ = thread.join();
        }
    });
    if let Err(error) = joined.await {
        tracing::warn!(%error, "waiting for the mouse threads failed");
    }
}

fn spawn_mouse<C: Channel + Send + 'static>(
    mouse: FoundMouse<C>,
    running: &Running,
    store: &ConfigStore,
    stop_flag: &Arc<AtomicBool>,
) -> Option<thread::JoinHandle<()>> {
    let guard = running.claim(mouse.key.clone());
    let name = format!("mouse-{}-{}", mouse.key.path, mouse.key.index);
    let store = store.clone();
    let stop_flag = Arc::clone(stop_flag);
    let spawned = thread::Builder::new().name(name).spawn(move || {
        let _guard = guard;
        serve_mouse(mouse, &store, &stop_flag, log_notice);
    });
    match spawned {
        Ok(handle) => Some(handle),
        Err(error) => {
            // The closure, and the guard it owned, are gone: the place is free.
            tracing::error!(%error, "cannot start a mouse thread");
            None
        }
    }
}

/// Serves one mouse until `stop` or until it is gone: loads its configuration
/// (created from the defaults the first time), applies it, reports its
/// buttons through `notify` with the mouse's serial.
fn serve_mouse<C: Channel>(
    found: FoundMouse<C>,
    store: &ConfigStore,
    stop: &AtomicBool,
    mut notify: impl FnMut(&str, Notice),
) {
    let model = found.link.model();
    let Some(unit) = found.unit else {
        tracing::warn!(%model, "a mouse without a unit id is left alone");
        return;
    };
    let serial = unit.to_string();
    let config = match store.load_mouse(&serial, model) {
        Ok(loaded) => loaded.config,
        Err(error) => {
            tracing::warn!(%serial, %model, %error, "cannot load the mouse configuration");
            return;
        }
    };
    tracing::info!(%serial, %model, "mouse attached");
    let mut session = MouseSession::new(found.link);
    match session.serve(&config, stop, |notice| notify(&serial, notice)) {
        Ok(()) => tracing::info!(%serial, "mouse released"),
        Err(error) => tracing::warn!(%serial, %error, "mouse lost"),
    }
}

fn log_notice(serial: &str, notice: Notice) {
    match notice {
        Notice::Problem(problem) => tracing::warn!(
            %serial,
            setting = problem.setting,
            error = %problem.error,
            "the mouse refused a setting"
        ),
        Notice::Button(event) => tracing::debug!(
            %serial,
            control = %event.control,
            pressed = event.pressed,
            "mouse button"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use hubergamof_mouse::channel::Hidpp;
    use hubergamof_mouse::features::REPROG_CONTROLS_V4;
    use hubergamof_mouse::link::{MouseLink, UnitId};
    use hubergamof_mouse::testing::{FakeDevice, FakeLink, frame};

    use super::*;
    use crate::testing::builtin_registry;

    const DEVICE: u8 = 0xFF;
    const UNIT: UnitId = UnitId([0xDE, 0xAD, 0xBE, 0xEF]);

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path(), Arc::new(builtin_registry()));
        (dir, store)
    }

    fn mouse_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("devices/deadbeef/mouse.json")
    }

    fn found(fake: FakeLink, unit: Option<UnitId>, index: u8) -> FoundMouse<FakeLink> {
        FoundMouse {
            key: MouseKey {
                path: "/dev/hidraw-test".to_owned(),
                index,
            },
            unit,
            link: MouseLink::connect(Hidpp::new(fake), DEVICE).unwrap(),
        }
    }

    fn pressing_back() -> FakeLink {
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        FakeLink::direct(device).with_incoming(vec![frame(DEVICE, reprog, 0x00, &[0, 0x53])])
    }

    #[test]
    fn a_mouse_is_served_with_its_stored_configuration_and_its_buttons_reported() {
        let (dir, store) = store();
        let stop = AtomicBool::new(false);
        let mut seen = Vec::new();
        serve_mouse(
            found(pressing_back(), Some(UNIT), 2),
            &store,
            &stop,
            |serial, notice| {
                assert_eq!(serial, "deadbeef");
                if let Notice::Button(event) = notice {
                    seen.push((event.control.to_string(), event.pressed));
                    stop.store(true, Ordering::Relaxed);
                }
            },
        );
        assert_eq!(
            seen,
            [
                ("back button".to_owned(), true),
                ("back button".to_owned(), false)
            ]
        );
        assert!(
            mouse_file(&dir).is_file(),
            "the default configuration was stored"
        );
    }

    #[test]
    fn a_mouse_without_a_unit_id_is_left_alone() {
        let (dir, store) = store();
        let stop = AtomicBool::new(true);
        serve_mouse(found(pressing_back(), None, 2), &store, &stop, |_, _| {
            panic!("nothing to report")
        });
        assert!(!dir.path().join("devices").exists());
    }

    #[test]
    fn a_configuration_that_cannot_be_loaded_leaves_the_mouse_alone() {
        let (dir, store) = store();
        // A file where the mouse's folder should be.
        std::fs::write(dir.path().join("devices"), b"x").unwrap();
        let stop = AtomicBool::new(true);
        serve_mouse(
            found(pressing_back(), Some(UNIT), 2),
            &store,
            &stop,
            |_, _| panic!("nothing to report"),
        );
    }

    /// Hands out its mice once and records the places it was told to skip.
    struct Scripted {
        mice: Vec<FoundMouse<FakeLink>>,
        asked: Arc<Mutex<Vec<Vec<MouseKey>>>>,
    }

    impl MouseSource for Scripted {
        type Channel = FakeLink;

        fn find(&mut self, owned: &[MouseKey]) -> Vec<FoundMouse<FakeLink>> {
            self.asked.lock().unwrap().push(owned.to_vec());
            std::mem::take(&mut self.mice)
        }
    }

    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn the_supervisor_serves_a_mouse_and_does_not_ask_for_it_again() {
        let (dir, store) = store();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let source = Scripted {
            mice: vec![found(
                FakeLink::direct(FakeDevice::mx_master_4()),
                Some(UNIT),
                2,
            )],
            asked: Arc::clone(&asked),
        };
        let supervisor = MouseSupervisor::start(source, store, Duration::from_millis(10));
        let file = mouse_file(&dir);
        until("the mouse to be served", || file.is_file()).await;
        until("a second scan", || asked.lock().unwrap().len() >= 2).await;
        supervisor.shutdown().await;
        let asked = asked.lock().unwrap();
        assert_eq!(asked[0], []);
        assert_eq!(asked[1].len(), 1, "the served place is skipped: {asked:?}");
    }

    #[tokio::test]
    async fn the_supervisor_stops_with_no_mouse_at_all() {
        let (_dir, store) = store();
        let source = Scripted {
            mice: Vec::new(),
            asked: Arc::default(),
        };
        let supervisor = MouseSupervisor::start(source, store, Duration::from_secs(3600));
        tokio::time::timeout(Duration::from_secs(10), supervisor.shutdown())
            .await
            .expect("the shutdown does not wait for the next scan");
    }

    #[test]
    fn a_place_is_free_again_when_its_thread_ends() {
        let running = Running::default();
        let key = MouseKey {
            path: "/dev/hidraw0".to_owned(),
            index: 1,
        };
        let guard = running.claim(key.clone());
        assert_eq!(running.snapshot(), [key]);
        drop(guard);
        assert_eq!(running.snapshot(), []);
    }
}
