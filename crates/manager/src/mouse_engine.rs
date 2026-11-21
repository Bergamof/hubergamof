//! Logitech mice: a thread per connected mouse that applies its configuration
//! and follows its diverted buttons, and the supervisor that finds the mice
//! and starts those threads.
//!
//! The supervisor looks again every [`SCAN_INTERVAL`] for the mice it does not
//! serve yet; a thread that ends (the channel failed, the mouse stopped
//! answering) frees its place for the next scan, which is the reconnection.
//! Every thread gives the buttons back before it ends.
//!
//! Not handled yet: on a receiver, a mouse that goes to sleep leaves its node
//! open and silent, so its thread goes on without ending, and the settings
//! are not applied again when it wakes. The receiver's connection
//! notifications are to be observed on the hardware first
//! (`docs/mouse-hidpp.md`).

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use hubergamof_mouse::channel::Channel;
use hubergamof_mouse::hidraw::{FoundMouse, HidrawChannel, MouseKey, find_mice};
use hubergamof_mouse::link::{Battery, UnitId};
use hubergamof_mouse::session::{MouseSession, Notice, Request};
use hubergamof_protocol::{MouseBattery, MouseConfig, MouseInfo, MouseModel, MouseProblem};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::config_store::{ConfigStore, ConfigStoreError, LoadOutcome};
use crate::mouse_actions::{ButtonActions, ButtonDispatcher};
use crate::mouse_hub::MouseHub;

/// Delay between two searches for mice not served yet.
pub const SCAN_INTERVAL: Duration = Duration::from_secs(5);

/// Time between two readings of the battery of a mouse: it drains over days,
/// and every reading is a frame the mouse has to answer.
const BATTERY_INTERVAL: Duration = Duration::from_secs(300);

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
struct Running(Arc<Mutex<Served>>);

/// The places and the physical mice a thread serves.
#[derive(Default)]
struct Served {
    keys: HashSet<MouseKey>,
    units: HashSet<UnitId>,
}

impl Running {
    fn lock(&self) -> std::sync::MutexGuard<'_, Served> {
        // Plain data: nothing to recover from a poisoned lock.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn snapshot(&self) -> Vec<MouseKey> {
        self.lock().keys.iter().cloned().collect()
    }

    /// Marks a place as served until the returned guard is dropped (the
    /// thread ends, panics included).
    ///
    /// `None` when that place, or that physical mouse reached through another
    /// place (two nodes), is already served: two sessions would fight over its
    /// buttons.
    fn claim(&self, key: MouseKey, unit: Option<UnitId>) -> Option<RunningGuard> {
        let mut served = self.lock();
        if served.keys.contains(&key) || unit.is_some_and(|unit| served.units.contains(&unit)) {
            return None;
        }
        served.keys.insert(key.clone());
        served.units.extend(unit);
        drop(served);
        Some(RunningGuard {
            running: self.clone(),
            key,
            unit,
        })
    }
}

struct RunningGuard {
    running: Running,
    key: MouseKey,
    unit: Option<UnitId>,
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        let mut served = self.running.lock();
        served.keys.remove(&self.key);
        if let Some(unit) = self.unit {
            served.units.remove(&unit);
        }
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
    pub fn start<S: MouseSource>(
        source: S,
        store: ConfigStore,
        hub: MouseHub,
        actions: ButtonActions,
        scan_interval: Duration,
    ) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(supervise(
            source,
            store,
            hub,
            actions,
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
    hub: MouseHub,
    actions: ButtonActions,
    scan_interval: Duration,
    mut stopped: oneshot::Receiver<()>,
    stop_flag: Arc<AtomicBool>,
) {
    let source = Arc::new(Mutex::new(source));
    let running = Running::default();
    let mut workers = Workers::new(Arc::clone(&stop_flag));
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
                workers.forget_finished();
                for mouse in found {
                    workers.extend(spawn_mouse(
                        mouse, &running, &store, &hub, &actions, &stop_flag,
                    ));
                }
            }
            Err(error) => tracing::warn!(%error, "searching for mice failed"),
        }
        tokio::select! {
            _ = &mut stopped => break,
            () = tokio::time::sleep(scan_interval) => {}
        }
    }
    // Dropping the workers raises the stop flag and waits for them.
    if let Err(error) = tokio::task::spawn_blocking(move || drop(workers)).await {
        tracing::warn!(%error, "waiting for the mouse threads failed");
    }
}

/// The mouse threads. Dropping them (the task ended, or was cancelled with the
/// runtime) stops and waits for them: each gives its buttons back first.
struct Workers {
    handles: Vec<thread::JoinHandle<()>>,
    stop_flag: Arc<AtomicBool>,
}

impl Workers {
    fn new(stop_flag: Arc<AtomicBool>) -> Self {
        Self {
            handles: Vec::new(),
            stop_flag,
        }
    }

    fn forget_finished(&mut self) {
        self.handles.retain(|handle| !handle.is_finished());
    }

    fn extend(&mut self, handle: Option<thread::JoinHandle<()>>) {
        self.handles.extend(handle);
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        for handle in self.handles.drain(..) {
            if handle.join().is_err() {
                tracing::warn!("a mouse thread panicked");
            }
        }
    }
}

fn spawn_mouse<C: Channel + Send + 'static>(
    mouse: FoundMouse<C>,
    running: &Running,
    store: &ConfigStore,
    hub: &MouseHub,
    actions: &ButtonActions,
    stop_flag: &Arc<AtomicBool>,
) -> Option<thread::JoinHandle<()>> {
    let Some(guard) = running.claim(mouse.key.clone(), mouse.unit) else {
        tracing::debug!(model = %mouse.link.model(), "a mouse already served through another place");
        return None;
    };
    let name = format!("mouse-{}-{}", mouse.key.path, mouse.key.index);
    let store = store.clone();
    let hub = hub.clone();
    let actions = actions.clone();
    let stop_flag = Arc::clone(stop_flag);
    let spawned = thread::Builder::new().name(name).spawn(move || {
        let _guard = guard;
        serve_mouse(mouse, &store, &hub, &actions, &stop_flag, log_notice);
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
/// (the first time, created from what the mouse is set to, so that serving it
/// changes nothing until the person does), applies it, reports its buttons
/// through `notify` with the mouse's serial, applies the configurations the
/// interface saves while it is served, and puts the settings back at the end.
fn serve_mouse<C: Channel>(
    found: FoundMouse<C>,
    store: &ConfigStore,
    hub: &MouseHub,
    actions: &ButtonActions,
    stop: &AtomicBool,
    mut notify: impl FnMut(&str, Notice),
) {
    let model = found.link.model();
    let Some(unit) = found.unit else {
        tracing::warn!(%model, "a mouse without a unit id is left alone");
        return;
    };
    let serial = unit.to_string();
    let mut session = MouseSession::new(found.link);
    let config = match load_config(store, &serial, model, &mut session) {
        Ok(config) => config,
        Err(error) => {
            tracing::warn!(%model, %error, "cannot load the mouse configuration");
            return;
        }
    };
    // The unit id is a hardware serial number: only at debug level, where
    // someone looks for it, not in logs pasted into bug reports.
    tracing::info!(%model, "mouse attached");
    tracing::debug!(%serial, "mouse unit");
    // Listed for the interface for as long as the mouse is served.
    let mut listed = hub.register(
        MouseInfo {
            serial: serial.clone(),
            model,
            display_name: model.display_name().to_owned(),
            capabilities: model.capabilities(),
        },
        config.clone(),
    );
    // Saved by the interface between the load and the listing.
    if let Ok(latest) = store.load_mouse(&serial, model) {
        listed.refresh(latest.config);
    }
    let status = listed.status_reporter();
    let (updates, haptics) = listed.streams();
    // The configuration in force, which says what each button does, and the
    // actions of this mouse: shared by the two closures the session calls.
    let current = RefCell::new(config.clone());
    let dispatcher = RefCell::new(ButtonDispatcher::new(actions.clone()));
    let battery_due = Cell::new(Instant::now());
    let next_request = || {
        if let Some(request) = dispatcher.borrow_mut().next_request() {
            return Some(request);
        }
        // A configuration saved since the last turn, the latest one.
        if updates.has_changed().unwrap_or(false) {
            let saved = updates.borrow_and_update().clone();
            current.borrow_mut().clone_from(&saved);
            return Some(Request::Apply(saved));
        }
        // What the events of the modules ask this mouse to play comes after the
        // configuration (a new strength is in force by then).
        if let Ok(waveform) = haptics.try_recv() {
            return Some(Request::PlayHaptic(waveform));
        }
        // The least urgent: the battery, read now and then.
        (Instant::now() >= battery_due.get()).then(|| {
            battery_due.set(Instant::now() + BATTERY_INTERVAL);
            Request::ReadBattery
        })
    };
    let refused = RefCell::new(Vec::new());
    let on_notice = |notice: Notice| {
        match &notice {
            // The problems of an application come one by one, then its end.
            Notice::Problem(problem) => refused.borrow_mut().push(MouseProblem {
                setting: problem.setting.to_owned(),
                message: problem.error.to_string(),
            }),
            Notice::Applied => status.report(refused.take()),
            Notice::Battery(battery) => status.report_battery(battery.and_then(known_battery)),
            Notice::Button(_) => {}
        }
        if let Notice::Button(event) = &notice {
            let configuration = current.borrow();
            dispatcher
                .borrow_mut()
                .handle(*event, configuration.bindings.get(&event.control));
        }
        notify(&serial, notice);
    };
    match session.serve_with_requests(&config, stop, next_request, on_notice) {
        Ok(()) => tracing::info!(%model, "mouse released"),
        Err(error) => tracing::warn!(%model, %error, "mouse lost"),
    }
}

/// What the interface shows of a battery reading: nothing when the mouse gave
/// no percentage.
fn known_battery(battery: Battery) -> Option<MouseBattery> {
    Some(MouseBattery {
        percent: battery.percent?,
        charging: battery.charging(),
    })
}

/// The stored configuration of a mouse. A new one (or one that had to be set
/// aside) adopts the settings the mouse has, and is stored so.
fn load_config<C: Channel>(
    store: &ConfigStore,
    serial: &str,
    model: MouseModel,
    session: &mut MouseSession<C>,
) -> Result<MouseConfig, ConfigStoreError> {
    let loaded = store.load_mouse(serial, model)?;
    if loaded.outcome == LoadOutcome::Loaded {
        return Ok(loaded.config);
    }
    let Ok(settings) = session.capture() else {
        // The mouse does not answer: the defaults, which are not stored, so
        // that the next time adopts what the mouse really has.
        return Ok(loaded.config);
    };
    let mut adopted = settings.to_config(serial, model);
    adopted.revision = loaded.config.revision;
    store.save_mouse(adopted)
}

fn log_notice(serial: &str, notice: Notice) {
    match notice {
        Notice::Problem(problem) => tracing::warn!(
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
        Notice::Applied => tracing::debug!(%serial, "configuration applied"),
        Notice::Battery(battery) => tracing::debug!(%serial, ?battery, "battery read"),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io;
    use std::rc::Rc;

    use hubergamof_protocol::{ButtonAction, MouseControlId};
    use std::sync::mpsc;
    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

    use hubergamof_mouse::channel::{ChannelError, Hidpp};
    use hubergamof_mouse::features::REPROG_CONTROLS_V4;
    use hubergamof_mouse::link::{MouseLink, UnitId};
    use hubergamof_mouse::testing::{FakeDevice, FakeLink, frame};

    use super::*;
    use crate::testing::builtin_registry;

    const DEVICE: u8 = 0xFF;
    const UNIT: UnitId = UnitId([0xDE, 0xAD, 0xBE, 0xEF]);
    const WAIT: Duration = Duration::from_secs(10);

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path(), Arc::new(builtin_registry()));
        (dir, store)
    }

    /// Actions that never find a device.
    fn no_actions() -> ButtonActions {
        let (_lock, locked) = tokio::sync::watch::channel(false);
        ButtonActions::new(locked, || Err(io::Error::from(io::ErrorKind::NotFound)))
    }

    fn hub_of(store: &ConfigStore) -> MouseHub {
        MouseHub::new(store.clone()).unwrap()
    }

    fn key(index: u8) -> MouseKey {
        MouseKey {
            path: "/dev/hidraw-test".to_owned(),
            index,
        }
    }

    fn found<C: Channel>(channel: C, unit: Option<UnitId>, index: u8) -> FoundMouse<C> {
        FoundMouse {
            key: key(index),
            unit,
            link: MouseLink::connect(Hidpp::new(channel), DEVICE).unwrap(),
        }
    }

    fn pressing_back() -> FakeLink {
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        FakeLink::direct(device).with_incoming(vec![frame(DEVICE, reprog, 0x00, &[0, 0x53])])
    }

    /// A link the test keeps a hand on while a session owns it.
    #[derive(Clone)]
    struct Shared(Rc<RefCell<FakeLink>>);

    impl Channel for Shared {
        fn write(&mut self, report: &[u8]) -> Result<(), ChannelError> {
            self.0.borrow_mut().write(report)
        }

        fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, ChannelError> {
            self.0.borrow_mut().read(timeout)
        }
    }

    fn state(shared: &Shared) -> hubergamof_mouse::testing::FakeState {
        shared.0.borrow().device_at(DEVICE).unwrap().state.clone()
    }

    #[test]
    fn a_mouse_is_served_with_its_stored_configuration_and_its_buttons_reported() {
        let (_dir, store) = store();
        let mut stored = store
            .load_mouse("deadbeef", hubergamof_protocol::MouseModel::MxMaster4)
            .unwrap()
            .config;
        stored.pointer.dpi = 2400;
        store.save_mouse(stored).unwrap();
        let shared = Shared(Rc::new(RefCell::new(pressing_back())));
        let stop = AtomicBool::new(false);
        let mut seen = Vec::new();
        let mut applied_dpi = None;
        serve_mouse(
            found(shared.clone(), Some(UNIT), 2),
            &store,
            &hub_of(&store),
            &no_actions(),
            &stop,
            |serial, notice| {
                assert_eq!(serial, "deadbeef");
                if let Notice::Button(event) = notice {
                    seen.push((event.control.to_string(), event.pressed));
                    applied_dpi.get_or_insert(state(&shared).dpi);
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
        assert_eq!(
            applied_dpi,
            Some(2400),
            "the stored configuration was applied"
        );
        let state = state(&shared);
        assert_eq!(state.dpi, 1600, "and the sensitivity the mouse had is back");
        assert!(state.diverted.is_empty(), "everything is given back");
    }

    #[test]
    fn a_new_mouse_is_served_with_what_it_has_and_nothing_changes() {
        let (_dir, store) = store();
        let mut device = FakeDevice::mx_master_4();
        device.state.dpi = 1000;
        device.state.wheel = [2, 10, 75];
        device.state.hires = 0x00;
        device.state.haptic = [1, 60];
        let shared = Shared(Rc::new(RefCell::new(FakeLink::direct(device))));
        let before = state(&shared);
        let stop = AtomicBool::new(true);
        serve_mouse(
            found(shared.clone(), Some(UNIT), 2),
            &store,
            &hub_of(&store),
            &no_actions(),
            &stop,
            |_, _| {},
        );
        let stored = store
            .load_mouse("deadbeef", MouseModel::MxMaster4)
            .unwrap()
            .config;
        assert_eq!(stored.pointer.dpi, 1000);
        assert!(!stored.wheel.hi_res, "the wheel keeps its resolution");
        assert_eq!(stored.revision, 1, "stored once, with what it adopted");
        let after = state(&shared);
        assert_eq!(
            (after.dpi, after.wheel, after.hires, after.haptic),
            (before.dpi, before.wheel, before.hires, before.haptic)
        );
    }

    /// What serving a mouse while the interface saves configurations gave: the
    /// mouse, the store, the problems reported, and what the mouse had at each
    /// button event (pressed, sensitivity, buttons diverted).
    type Saved = (
        Shared,
        ConfigStore,
        Vec<&'static str>,
        Vec<(bool, u16, usize)>,
    );

    /// Serves a mouse that is pressed then released, the interface saving
    /// `dpis` (one configuration each) when the press is seen. Returns the
    /// mouse, the store, the problems reported and what the mouse had at each
    /// button event.
    fn serve_saving(dpis: &[u16]) -> Saved {
        let (dir, store) = store();
        // The folder lives as long as the store is used by the test.
        std::mem::forget(dir);
        let hub = hub_of(&store);
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        let fake = FakeLink::direct(device).with_incoming(vec![
            frame(DEVICE, reprog, 0x00, &[0, 0x53]),
            frame(DEVICE, reprog, 0x00, &[]),
        ]);
        let shared = Shared(Rc::new(RefCell::new(fake)));
        let stop = AtomicBool::new(false);
        let mut problems = Vec::new();
        let mut events = Vec::new();
        serve_mouse(
            found(shared.clone(), Some(UNIT), 2),
            &store,
            &hub,
            &no_actions(),
            &stop,
            |_, notice| match notice {
                Notice::Problem(problem) => problems.push(problem.setting),
                Notice::Applied | Notice::Battery(_) => {}
                Notice::Button(event) => {
                    let state = state(&shared);
                    events.push((event.pressed, state.dpi, state.diverted.len()));
                    if event.pressed {
                        // The interface saves new configurations.
                        for dpi in dpis {
                            let mut config =
                                hub.subscribe_config("deadbeef").unwrap().borrow().clone();
                            config.pointer.dpi = *dpi;
                            hub.update_mouse("deadbeef", config).unwrap();
                        }
                    } else {
                        stop.store(true, Ordering::Relaxed);
                    }
                }
            },
        );
        (shared, store, problems, events)
    }

    #[test]
    fn a_configuration_saved_while_the_mouse_is_served_reaches_the_mouse_and_is_kept() {
        let (shared, store, problems, events) = serve_saving(&[2400]);
        assert_eq!(problems, [] as [&str; 0]);
        assert_eq!(
            events,
            [(true, 1600, 1), (false, 2400, 1)],
            "the new sensitivity is applied before the release is reported"
        );
        let after = state(&shared);
        assert_eq!(after.dpi, 1600, "the mouse's own sensitivity is back");
        assert!(after.diverted.is_empty(), "and its buttons");
        let stored = store
            .load_mouse("deadbeef", MouseModel::MxMaster4)
            .unwrap()
            .config;
        assert_eq!((stored.pointer.dpi, stored.revision), (2400, 2));
    }

    #[test]
    fn several_configurations_saved_between_two_turns_apply_only_the_last() {
        let (shared, store, _, events) = serve_saving(&[1200, 2400]);
        assert_eq!(events[1], (false, 2400, 1));
        let stored = store
            .load_mouse("deadbeef", MouseModel::MxMaster4)
            .unwrap()
            .config;
        assert_eq!((stored.pointer.dpi, stored.revision), (2400, 3));
        let dpi_writes: Vec<u16> = shared
            .0
            .borrow()
            .written()
            .iter()
            .filter(|report| report.get(3).is_some_and(|byte| byte >> 4 == 3))
            .filter_map(|report| Some(u16::from_be_bytes([*report.get(5)?, *report.get(6)?])))
            .collect();
        assert!(
            !dpi_writes.contains(&1200),
            "the sensitivity that was overtaken was never written: {dpi_writes:?}"
        );
    }

    /// A device that records what it is told, kept by the test.
    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<(crate::input::Key, bool)>>>);

    impl crate::input::Injector for Recorder {
        fn send(&mut self, key: crate::input::Key, pressed: bool) -> io::Result<()> {
            self.0.lock().unwrap().push((key, pressed));
            Ok(())
        }
    }

    /// Actions that send to `recorder`, held off while `locked` is true.
    fn recording_actions(recorder: &Recorder, locked: bool) -> ButtonActions {
        let (_lock, locked) = tokio::sync::watch::channel(locked);
        let recorder = recorder.clone();
        ButtonActions::new(locked, move || {
            Ok(Box::new(recorder.clone()) as Box<dyn crate::input::Injector>)
        })
    }

    /// The back button, pressed and released `times` times, bound to `action`.
    /// `on_release` runs at each release, with the mouse.
    fn serve_back_button(
        action: ButtonAction,
        times: usize,
        actions: &ButtonActions,
        on_release: impl FnMut(usize, &MouseHub, &Shared),
    ) -> Shared {
        let (_dir, store) = store();
        let mut stored = store
            .load_mouse("deadbeef", MouseModel::MxMaster4)
            .unwrap()
            .config;
        stored.bindings.insert(MouseControlId::Back, action);
        store.save_mouse(stored).unwrap();
        let hub = hub_of(&store);
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        let frames = (0..times)
            .flat_map(|_| {
                [
                    frame(DEVICE, reprog, 0x00, &[0, 0x53]),
                    frame(DEVICE, reprog, 0x00, &[]),
                ]
            })
            .collect();
        let shared = Shared(Rc::new(RefCell::new(
            FakeLink::direct(device).with_incoming(frames),
        )));
        let stop = AtomicBool::new(false);
        let mut releases = 0;
        let mut on_release = on_release;
        serve_mouse(
            found(shared.clone(), Some(UNIT), 2),
            &store,
            &hub,
            actions,
            &stop,
            |_, notice| {
                if let Notice::Button(event) = notice
                    && !event.pressed
                    && event.control == MouseControlId::Back
                {
                    releases += 1;
                    on_release(releases, &hub, &shared);
                    if releases == times {
                        stop.store(true, Ordering::Relaxed);
                    }
                }
            },
        );
        shared
    }

    #[test]
    fn a_button_bound_to_a_click_clicks_while_the_mouse_is_served() {
        use crate::input::Key;
        let recorder = Recorder::default();
        let click = ButtonAction::Click {
            button: hubergamof_protocol::MouseButton::Middle,
        };
        serve_back_button(click, 1, &recording_actions(&recorder, false), |_, _, _| {});
        assert_eq!(
            *recorder.0.lock().unwrap(),
            [(Key::ButtonMiddle, true), (Key::ButtonMiddle, false)]
        );
    }

    #[test]
    fn nothing_is_sent_for_a_button_while_the_computer_is_locked() {
        let recorder = Recorder::default();
        let click = ButtonAction::Click {
            button: hubergamof_protocol::MouseButton::Middle,
        };
        serve_back_button(click, 2, &recording_actions(&recorder, true), |_, _, _| {});
        assert_eq!(*recorder.0.lock().unwrap(), []);
    }

    #[test]
    fn a_button_held_when_the_service_ends_is_let_go_of() {
        use crate::input::Key;
        let (_dir, store) = store();
        let mut stored = store
            .load_mouse("deadbeef", MouseModel::MxMaster4)
            .unwrap()
            .config;
        stored.bindings.insert(
            MouseControlId::Back,
            ButtonAction::Click {
                button: hubergamof_protocol::MouseButton::Middle,
            },
        );
        store.save_mouse(stored).unwrap();
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        // A press and no release: the mouse is stopped while the button is down.
        let shared = Shared(Rc::new(RefCell::new(
            FakeLink::direct(device).with_incoming(vec![frame(DEVICE, reprog, 0x00, &[0, 0x53])]),
        )));
        let recorder = Recorder::default();
        let stop = AtomicBool::new(false);
        serve_mouse(
            found(shared, Some(UNIT), 2),
            &store,
            &hub_of(&store),
            &recording_actions(&recorder, false),
            &stop,
            |_, notice| {
                if matches!(notice, Notice::Button(event) if event.pressed) {
                    stop.store(true, Ordering::Relaxed);
                }
            },
        );
        assert_eq!(
            *recorder.0.lock().unwrap(),
            [(Key::ButtonMiddle, true), (Key::ButtonMiddle, false)]
        );
    }

    #[test]
    fn a_haptic_event_of_a_module_is_played_on_the_mouse_while_it_is_served() {
        use crate::haptics::{HapticOutput, HapticPlay};
        use hubergamof_module_api::HapticWaveform;
        let recorder = Recorder::default();
        let shared = serve_back_button(
            ButtonAction::None,
            2,
            &recording_actions(&recorder, false),
            |release, hub, _| {
                if release == 1 {
                    hub.play(&HapticPlay {
                        module: "pomodoro".into(),
                        event: "step-end".to_owned(),
                        waveform: HapticWaveform::Completed,
                    });
                }
            },
        );
        // The turn after the event played it, before the service ended.
        assert_eq!(
            state(&shared).played,
            [hubergamof_mouse::link::waveform_id(HapticWaveform::Completed).unwrap()]
        );
    }

    #[test]
    fn the_battery_of_the_mouse_reaches_the_interface_while_it_is_served() {
        let recorder = Recorder::default();
        let mut told = None;
        serve_back_button(
            ButtonAction::None,
            1,
            &recording_actions(&recorder, false),
            |_, hub, _| {
                told = hub
                    .subscribe_status("deadbeef")
                    .and_then(|status| status.borrow().battery);
            },
        );
        let battery = told.expect("the battery was read at the start");
        assert_eq!(battery.percent, 82);
        assert!(!battery.charging);
    }

    #[test]
    fn a_button_bound_to_the_wheel_toggle_switches_the_wheel_and_it_is_given_back() {
        let recorder = Recorder::default();
        let mut during = Vec::new();
        let shared = serve_back_button(
            ButtonAction::ToggleWheelMode,
            2,
            &recording_actions(&recorder, false),
            |release, _, shared| {
                // The toggle asked on a press is carried out at the next turn,
                // before the release of that button is read.
                during.push((release, state(shared).wheel[0]));
            },
        );
        assert_eq!(*recorder.0.lock().unwrap(), [], "no key is involved");
        assert_eq!(
            during,
            [(1, 1), (2, 2)],
            "free after one toggle, ratchet after two"
        );
        assert_eq!(state(&shared).wheel[0], 2, "the mouse's own mode is back");
    }

    #[test]
    fn the_new_binding_is_used_from_the_next_press_after_the_interface_saves_it() {
        use crate::input::Key;
        let recorder = Recorder::default();
        let click = ButtonAction::Click {
            button: hubergamof_protocol::MouseButton::Middle,
        };
        serve_back_button(
            click,
            2,
            &recording_actions(&recorder, false),
            |release, hub, _| {
                if release == 1 {
                    let mut config = hub.subscribe_config("deadbeef").unwrap().borrow().clone();
                    config
                        .bindings
                        .insert(MouseControlId::Back, ButtonAction::DoubleClick);
                    hub.update_mouse("deadbeef", config).unwrap();
                }
            },
        );
        assert_eq!(
            *recorder.0.lock().unwrap(),
            [
                (Key::ButtonMiddle, true),
                (Key::ButtonMiddle, false),
                (Key::ButtonLeft, true),
                (Key::ButtonLeft, false),
                (Key::ButtonLeft, true),
                (Key::ButtonLeft, false),
            ]
        );
    }

    #[test]
    fn a_stored_configuration_is_kept_as_it_is() {
        let (_dir, store) = store();
        let mut stored = store
            .load_mouse("deadbeef", MouseModel::MxMaster4)
            .unwrap()
            .config;
        stored.pointer.dpi = 2400;
        store.save_mouse(stored).unwrap();
        let shared = Shared(Rc::new(RefCell::new(FakeLink::direct(
            FakeDevice::mx_master_4(),
        ))));
        let stop = AtomicBool::new(true);
        serve_mouse(
            found(shared.clone(), Some(UNIT), 2),
            &store,
            &hub_of(&store),
            &no_actions(),
            &stop,
            |_, _| {},
        );
        let config = store
            .load_mouse("deadbeef", MouseModel::MxMaster4)
            .unwrap()
            .config;
        assert_eq!((config.pointer.dpi, config.revision), (2400, 1));
    }

    #[test]
    fn a_mouse_without_a_unit_id_is_left_alone() {
        let (dir, store) = store();
        let stop = AtomicBool::new(true);
        serve_mouse(
            found(pressing_back(), None, 2),
            &store,
            &hub_of(&store),
            &no_actions(),
            &stop,
            |_, _| panic!("nothing to report"),
        );
        assert!(!dir.path().join("devices").exists());
    }

    #[test]
    fn a_configuration_that_cannot_be_loaded_leaves_the_mouse_untouched() {
        let (dir, store) = store();
        // A file where the mouse's folder should be.
        std::fs::write(dir.path().join("devices"), b"x").unwrap();
        let shared = Shared(Rc::new(RefCell::new(pressing_back())));
        let mouse = found(shared.clone(), Some(UNIT), 2);
        let written = shared.0.borrow().written().len();
        let stop = AtomicBool::new(true);
        serve_mouse(
            mouse,
            &store,
            &hub_of(&store),
            &no_actions(),
            &stop,
            |_, _| panic!("nothing to report"),
        );
        assert_eq!(
            shared.0.borrow().written().len(),
            written,
            "nothing written"
        );
    }

    #[test]
    fn a_corrupt_stored_configuration_is_replaced_by_the_defaults() {
        let (dir, store) = store();
        let path = dir.path().join("devices/deadbeef/mouse.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        let mut device = FakeDevice::mx_master_4();
        device.state.dpi = 1000;
        let shared = Shared(Rc::new(RefCell::new(FakeLink::direct(device))));
        let stop = AtomicBool::new(true);
        serve_mouse(
            found(shared.clone(), Some(UNIT), 2),
            &store,
            &hub_of(&store),
            &no_actions(),
            &stop,
            |_, _| {},
        );
        assert_eq!(state(&shared).dpi, 1000, "the mouse keeps its sensitivity");
        let adopted = store
            .load_mouse("deadbeef", MouseModel::MxMaster4)
            .unwrap()
            .config;
        assert_eq!(adopted.pointer.dpi, 1000, "the new configuration adopts it");
        assert!(
            std::fs::read_dir(path.parent().unwrap())
                .unwrap()
                .any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("mouse.json.invalid-"))
        );
    }

    /// What a scripted mouse does once it is served.
    #[derive(Clone, Copy)]
    enum Behaviour {
        Serve,
        Panic,
        Vanish,
    }

    /// A scripted link that tells the test when its mouse is being served.
    struct Probe {
        inner: FakeLink,
        behaviour: Behaviour,
        armed: Arc<AtomicBool>,
        served: UnboundedSender<u8>,
        label: u8,
        reported: bool,
    }

    impl Channel for Probe {
        fn write(&mut self, report: &[u8]) -> Result<(), ChannelError> {
            self.inner.write(report)
        }

        fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, ChannelError> {
            if self.armed.load(Ordering::Relaxed) {
                if !std::mem::replace(&mut self.reported, true) {
                    let _ = self.served.send(self.label);
                }
                match self.behaviour {
                    Behaviour::Serve => {}
                    Behaviour::Panic => panic!("scripted mouse failure"),
                    Behaviour::Vanish => return Err(ChannelError("gone".to_owned())),
                }
            }
            self.inner.read(timeout)
        }
    }

    /// A mouse of the scripted source, served from its first read on.
    fn probe(
        label: u8,
        index: u8,
        behaviour: Behaviour,
        served: &UnboundedSender<u8>,
    ) -> FoundMouse<Probe> {
        let armed = Arc::new(AtomicBool::new(false));
        let channel = Probe {
            inner: FakeLink::direct(FakeDevice::mx_master_4()),
            behaviour,
            armed: Arc::clone(&armed),
            served: served.clone(),
            label,
            reported: false,
        };
        let mouse = FoundMouse {
            key: key(index),
            unit: Some(UnitId([0, 0, 0, label])),
            link: MouseLink::connect(Hidpp::new(channel), DEVICE).unwrap(),
        };
        armed.store(true, Ordering::Relaxed);
        mouse
    }

    /// Hands its mice out when their place is free, and tells the test about
    /// each scan.
    struct Scripted {
        mice: Vec<FoundMouse<Probe>>,
        scans: UnboundedSender<Vec<MouseKey>>,
        block_first_scan: Option<mpsc::Receiver<()>>,
        fail_first_scan: bool,
    }

    impl Scripted {
        fn new(mice: Vec<FoundMouse<Probe>>) -> (Self, UnboundedReceiver<Vec<MouseKey>>) {
            let (scans, scanned) = unbounded_channel();
            let source = Self {
                mice,
                scans,
                block_first_scan: None,
                fail_first_scan: false,
            };
            (source, scanned)
        }
    }

    impl MouseSource for Scripted {
        type Channel = Probe;

        fn find(&mut self, owned: &[MouseKey]) -> Vec<FoundMouse<Probe>> {
            let _ = self.scans.send(owned.to_vec());
            if let Some(release) = self.block_first_scan.take() {
                let _ = release.recv();
            }
            assert!(
                !std::mem::take(&mut self.fail_first_scan),
                "scripted scan failure"
            );
            // One mouse per place and scan, as the hardware gives.
            let mut places = owned.to_vec();
            let (free, taken): (Vec<_>, Vec<_>) = std::mem::take(&mut self.mice)
                .into_iter()
                .partition(|mouse| {
                    let free = !places.contains(&mouse.key);
                    places.push(mouse.key.clone());
                    free
                });
            self.mice = taken;
            free
        }
    }

    /// Waits for what the scripted mice or source report: a signal, not a
    /// duration (the threads are real, so the clock cannot be paused here).
    async fn next<T>(receiver: &mut UnboundedReceiver<T>) -> T {
        tokio::time::timeout(WAIT, receiver.recv())
            .await
            .expect("the scripted event never came")
            .expect("the scripted source is gone")
    }

    #[tokio::test]
    async fn the_supervisor_serves_a_mouse_and_skips_its_place_in_the_next_scans() {
        let (_dir, store) = store();
        let (served, mut served_rx) = unbounded_channel();
        let (source, mut scans) = Scripted::new(vec![probe(1, 2, Behaviour::Serve, &served)]);
        let supervisor = MouseSupervisor::start(
            source,
            store.clone(),
            hub_of(&store),
            no_actions(),
            Duration::from_millis(10),
        );
        assert_eq!(next(&mut scans).await, [], "the first scan skips nothing");
        assert_eq!(next(&mut served_rx).await, 1);
        assert_eq!(
            next(&mut scans).await,
            [key(2)],
            "the served place is skipped from then on"
        );
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn two_mice_are_served_at_once_each_with_its_own_configuration() {
        let (dir, store) = store();
        let (served, mut served_rx) = unbounded_channel();
        let (source, _scans) = Scripted::new(vec![
            probe(1, 1, Behaviour::Serve, &served),
            probe(2, 2, Behaviour::Serve, &served),
        ]);
        let supervisor = MouseSupervisor::start(
            source,
            store.clone(),
            hub_of(&store),
            no_actions(),
            Duration::from_millis(10),
        );
        let mut labels = vec![next(&mut served_rx).await, next(&mut served_rx).await];
        labels.sort_unstable();
        assert_eq!(labels, [1, 2]);
        supervisor.shutdown().await;
        for unit in ["00000001", "00000002"] {
            assert!(
                dir.path()
                    .join(format!("devices/{unit}/mouse.json"))
                    .is_file()
            );
        }
    }

    #[tokio::test]
    async fn a_mouse_that_vanishes_is_served_again_when_it_comes_back() {
        let (_dir, store) = store();
        let (served, mut served_rx) = unbounded_channel();
        // The same place, twice: the second can only be taken once the first
        // thread has ended and freed it.
        let (source, _scans) = Scripted::new(vec![
            probe(1, 2, Behaviour::Vanish, &served),
            probe(2, 2, Behaviour::Serve, &served),
        ]);
        let supervisor = MouseSupervisor::start(
            source,
            store.clone(),
            hub_of(&store),
            no_actions(),
            Duration::from_millis(10),
        );
        assert_eq!(next(&mut served_rx).await, 1);
        assert_eq!(next(&mut served_rx).await, 2);
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn a_thread_that_panics_frees_its_place_and_does_not_stop_the_supervisor() {
        let (_dir, store) = store();
        let (served, mut served_rx) = unbounded_channel();
        let (source, _scans) = Scripted::new(vec![
            probe(1, 2, Behaviour::Panic, &served),
            probe(2, 2, Behaviour::Serve, &served),
        ]);
        let supervisor = MouseSupervisor::start(
            source,
            store.clone(),
            hub_of(&store),
            no_actions(),
            Duration::from_millis(10),
        );
        assert_eq!(next(&mut served_rx).await, 1);
        assert_eq!(next(&mut served_rx).await, 2);
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn a_failing_scan_is_survived() {
        let (_dir, store) = store();
        let (served, mut served_rx) = unbounded_channel();
        let (mut source, _scans) = Scripted::new(vec![probe(1, 2, Behaviour::Serve, &served)]);
        source.fail_first_scan = true;
        let supervisor = MouseSupervisor::start(
            source,
            store.clone(),
            hub_of(&store),
            no_actions(),
            Duration::from_millis(10),
        );
        assert_eq!(next(&mut served_rx).await, 1, "the next scan found it");
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn the_supervisor_stops_during_a_scan() {
        let (_dir, store) = store();
        let (mut source, mut scans) = Scripted::new(Vec::new());
        let (release, blocked) = mpsc::channel();
        source.block_first_scan = Some(blocked);
        let supervisor = MouseSupervisor::start(
            source,
            store.clone(),
            hub_of(&store),
            no_actions(),
            Duration::from_secs(3600),
        );
        // The scan has started, and is stuck.
        next(&mut scans).await;
        tokio::time::timeout(WAIT, supervisor.shutdown())
            .await
            .expect("the shutdown does not wait for a scan in progress");
        release.send(()).unwrap();
    }

    #[tokio::test]
    async fn the_supervisor_stops_with_no_mouse_at_all() {
        let (_dir, store) = store();
        let (source, _scans) = Scripted::new(Vec::new());
        let supervisor = MouseSupervisor::start(
            source,
            store.clone(),
            hub_of(&store),
            no_actions(),
            Duration::from_secs(3600),
        );
        tokio::time::timeout(WAIT, supervisor.shutdown())
            .await
            .expect("the shutdown does not wait for the next scan");
    }

    #[test]
    fn a_place_is_free_again_when_its_thread_ends() {
        let running = Running::default();
        let guard = running.claim(key(1), Some(UNIT)).unwrap();
        assert_eq!(running.snapshot(), [key(1)]);
        drop(guard);
        assert_eq!(running.snapshot(), []);
        assert!(
            running.claim(key(2), Some(UNIT)).is_some(),
            "the unit is free too"
        );
    }

    #[test]
    fn a_mouse_reached_by_two_places_is_served_once() {
        let running = Running::default();
        let first = running.claim(key(1), Some(UNIT)).unwrap();
        assert!(running.claim(key(2), Some(UNIT)).is_none());
        assert_eq!(
            running.snapshot(),
            [key(1)],
            "the refused place is not taken"
        );
        assert!(running.claim(key(2), Some(UnitId([1, 2, 3, 4]))).is_some());
        drop(first);
    }
}
