//! The modules that live in the application: instances that work whether or
//! not a device is connected.
//!
//! The host runs, in one task, the instances of the kinds that declare
//! [`ModuleLifetime::Application`] (see `config/modules.json`), with the same
//! isolation as the engines (a panic replaces the instance, a handler past its
//! budget is abandoned). Today it serves what has no device to turn to: the
//! actions of the radial menu, the haptic events, the stop of the manager.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hubergamof_module_api::{
    Command, ModuleInstanceId, ModuleKind, ModuleLifetime, ScreenLockState,
};
use hubergamof_modules::ModuleRegistry;
use hubergamof_protocol::ModuleInstance;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::config_store::ConfigStore;
use crate::engine::{Call, Instances, MODULE_EVENT_CAPACITY, Outcome, RoutedEvent};
use crate::haptics::HapticOutput;
use crate::screen_lock::ScreenLock;

/// Requests waiting for the host, at most.
const REQUEST_CAPACITY: usize = 16;

/// Longest wait for the host to stop when the manager leaves, request
/// included.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Longest the instances are given to hear that the manager stops: a module
/// that blocks there must not hold the manager back.
const INSTANCES_STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// Prefix of the identifier of the instance the host creates for a kind that
/// the configuration does not mention.
const DEFAULT_ID_PREFIX: &str = "app-";

/// What the host needs from the manager.
#[derive(Clone)]
pub struct HostServices {
    /// Available modules.
    pub registry: Arc<ModuleRegistry>,
    /// Where `modules.json` is.
    pub store: ConfigStore,
    /// Where the haptic events of the modules go.
    pub haptics: Arc<dyn HapticOutput>,
    /// Whether the computer is locked.
    pub screen_lock: Arc<dyn ScreenLock>,
}

enum Request {
    RadialAction {
        module: ModuleKind,
        action: String,
        reply: oneshot::Sender<RadialReply>,
    },
    Shutdown(oneshot::Sender<()>),
}

/// What the host answers to a radial action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadialReply {
    /// The host holds no instance of the module: somebody else may.
    NotHeld,
    /// The host holds it and refused the action: the computer is locked.
    Refused,
    /// The host holds it and ran the action.
    Done,
}

impl RadialReply {
    /// Whether the host holds the module, whatever it did with the action.
    #[must_use]
    pub fn is_held(self) -> bool {
        self != Self::NotHeld
    }
}

/// Entry point of the host: cheap to clone.
#[derive(Clone)]
pub struct ModuleHost {
    requests: mpsc::Sender<Request>,
}

impl ModuleHost {
    /// Starts the host on the current Tokio runtime. The configuration is
    /// read by the task, off the async threads: requests sent meanwhile wait.
    #[must_use]
    pub fn start(services: HostServices) -> (Self, JoinHandle<()>) {
        let (requests, received) = mpsc::channel(REQUEST_CAPACITY);
        let task = tokio::spawn(async move {
            let for_reading = services.clone();
            let configured =
                tokio::task::spawn_blocking(move || configured_instances(&for_reading))
                    .await
                    .unwrap_or_else(|error| {
                        tracing::error!(%error, "application modules not read, none run");
                        Vec::new()
                    });
            run(Hosted::new(services, configured), received).await;
        });
        (Self { requests }, task)
    }

    /// Runs the radial action `action` of the instance of `module` the host
    /// holds, as the menu asks.
    pub async fn radial_action(&self, module: &ModuleKind, action: &str) -> RadialReply {
        let (reply, answer) = oneshot::channel();
        let request = Request::RadialAction {
            module: module.clone(),
            action: action.to_owned(),
            reply,
        };
        if self.requests.send(request).await.is_err() {
            return RadialReply::NotHeld;
        }
        answer.await.unwrap_or(RadialReply::NotHeld)
    }

    /// Stops the host: its instances are told that the manager stops clean,
    /// then destroyed. Gives up after [`SHUTDOWN_TIMEOUT`], the request
    /// waiting for room in the queue included.
    pub async fn shutdown(&self) {
        let stop = async {
            let (done, stopped) = oneshot::channel();
            if self.requests.send(Request::Shutdown(done)).await.is_ok() {
                let _ = stopped.await;
            }
        };
        if tokio::time::timeout(SHUTDOWN_TIMEOUT, stop).await.is_err() {
            tracing::warn!("the module host did not stop in time");
        }
    }
}

/// What the task owns.
struct Hosted {
    instances: Instances,
    /// Instance of each kind that lives in the application.
    by_kind: BTreeMap<ModuleKind, ModuleInstanceId>,
    events: mpsc::Receiver<RoutedEvent>,
    haptics: Arc<dyn HapticOutput>,
    locked: tokio::sync::watch::Receiver<bool>,
}

impl Hosted {
    fn new(services: HostServices, configured: Vec<ModuleInstance>) -> Self {
        let (events_tx, events) = mpsc::channel(MODULE_EVENT_CAPACITY);
        let locked = services.screen_lock.watch();
        let screen = locked.clone();
        let mut instances = Instances::detached(
            Arc::clone(&services.registry),
            events_tx,
            ScreenLockState::new(move || *screen.borrow()),
        );
        instances.sync(&configured);
        let by_kind = configured
            .into_iter()
            .map(|instance| (instance.kind, instance.id))
            .collect();
        Self {
            instances,
            by_kind,
            events,
            haptics: services.haptics,
            locked,
        }
    }

    async fn radial_action(&mut self, module: &ModuleKind, action: String) -> RadialReply {
        let Some(id) = self.by_kind.get(module).cloned() else {
            return RadialReply::NotHeld;
        };
        // Stricter than a key, as on a device: the menu shows nothing on a
        // locked computer, so nothing runs from it then.
        if *self.locked.borrow() {
            tracing::debug!(%id, "radial action refused: the computer is locked");
            return RadialReply::Refused;
        }
        if let Some(Outcome::Done(commands)) =
            self.instances.call(&id, Call::RadialAction(action)).await
        {
            self.run(&id, commands);
        }
        RadialReply::Done
    }

    async fn on_event(&mut self, routed: RoutedEvent) {
        let id = routed.module.clone();
        if let Some(Outcome::Done(commands)) = self.instances.dispatch(routed).await {
            self.run(&id, commands);
        }
    }

    /// What the commands of an instance do with no device: the haptic events
    /// go to the mice, the rest has no surface to show on.
    fn run(&mut self, source: &ModuleInstanceId, commands: Vec<Command>) {
        for command in commands {
            match command {
                Command::Haptic(event) => match self.instances.haptic_play(source, &event) {
                    Some(play) => self.haptics.play(&play),
                    None => {
                        tracing::warn!(module = %source, %event, "undeclared haptic event dropped");
                    }
                },
                other => {
                    tracing::debug!(module = %source, command = ?other, "no device to show it on");
                }
            }
        }
    }
}

/// The instances the host runs: those of `modules.json` whose kind lives in
/// the application, and a default one for each such kind that file does not
/// mention. An unreadable file is logged and counts as empty.
fn configured_instances(services: &HostServices) -> Vec<ModuleInstance> {
    let stored = match services.store.load_app_modules() {
        Ok(loaded) => loaded.config.modules,
        Err(error) => {
            tracing::warn!(%error, "application modules not loaded, defaults used");
            Vec::new()
        }
    };
    let lives_in_application = |kind: &ModuleKind| {
        services
            .registry
            .get(kind)
            .is_some_and(|factory| factory.lifetime() == ModuleLifetime::Application)
    };
    let mut instances: Vec<ModuleInstance> = Vec::new();
    for instance in stored {
        if !lives_in_application(&instance.kind) {
            tracing::warn!(module = %instance.id, kind = %instance.kind, "not a module of the application: ignored");
        } else if instances.iter().any(|kept| kept.kind == instance.kind) {
            tracing::warn!(module = %instance.id, kind = %instance.kind, "a second instance of a module of the application: ignored");
        } else {
            instances.push(instance);
        }
    }
    for factory in services.registry.factories() {
        let kind = factory.descriptor().kind;
        if factory.lifetime() == ModuleLifetime::Application
            && !instances.iter().any(|kept| kept.kind == kind)
        {
            instances.push(ModuleInstance {
                id: ModuleInstanceId(format!("{DEFAULT_ID_PREFIX}{}", kind.0)),
                settings: factory.default_settings(),
                kind,
                label: None,
                image: None,
            });
        }
    }
    instances
}

async fn run(mut hosted: Hosted, mut requests: mpsc::Receiver<Request>) {
    loop {
        tokio::select! {
            request = requests.recv() => match request {
                Some(Request::RadialAction { module, action, reply }) => {
                    let answer = hosted.radial_action(&module, action).await;
                    let _ = reply.send(answer);
                }
                Some(Request::Shutdown(done)) => {
                    stop_instances(hosted.instances).await;
                    let _ = done.send(());
                    return;
                }
                None => break,
            },
            Some(routed) = hosted.events.recv() => hosted.on_event(routed).await,
        }
        hosted.instances.take_task_panics();
    }
    stop_instances(hosted.instances).await;
}

/// Tells the instances that the manager stops, off the async threads and for
/// no longer than [`INSTANCES_STOP_TIMEOUT`]: a module that blocks is
/// abandoned.
async fn stop_instances(mut instances: Instances) {
    let stopping = tokio::task::spawn_blocking(move || instances.shutdown());
    if tokio::time::timeout(INSTANCES_STOP_TIMEOUT, stopping)
        .await
        .is_err()
    {
        tracing::warn!("a module did not stop in time, abandoned");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use hubergamof_modules::pomodoro::{MemoryStateStore, PomodoroStateStore};
    use hubergamof_modules::{BuiltinServices, builtin_registry_with};
    use serde_json::json;
    use tokio::sync::watch;

    use super::*;
    use crate::haptics::{HapticPlay, NoHaptics};
    use crate::screen_lock::NoScreenLock;

    /// The id of the pomodoro the host creates when `modules.json` has none.
    const POMODORO_ID: &str = "app-pomodoro";

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl HapticOutput for Recorder {
        fn play(&self, play: &HapticPlay) {
            self.0.lock().unwrap().push(play.key());
        }
    }

    struct FakeLock(watch::Sender<bool>);

    impl ScreenLock for FakeLock {
        fn watch(&self) -> watch::Receiver<bool> {
            self.0.subscribe()
        }
    }

    struct Fixture {
        dir: tempfile::TempDir,
        state: Arc<MemoryStateStore>,
        registry: Arc<ModuleRegistry>,
    }

    impl Fixture {
        fn new() -> Self {
            let state = Arc::new(MemoryStateStore::default());
            let registry = Arc::new(builtin_registry_with(BuiltinServices {
                pomodoro_state: Arc::clone(&state) as _,
                ..BuiltinServices::simulated()
            }));
            Self {
                dir: tempfile::tempdir().unwrap(),
                state,
                registry,
            }
        }

        fn services(
            &self,
            haptics: Arc<dyn HapticOutput>,
            screen_lock: Arc<dyn ScreenLock>,
        ) -> HostServices {
            HostServices {
                registry: Arc::clone(&self.registry),
                store: ConfigStore::new(self.dir.path(), Arc::clone(&self.registry)),
                haptics,
                screen_lock,
            }
        }

        fn start(&self) -> (ModuleHost, JoinHandle<()>) {
            ModuleHost::start(self.services(Arc::new(NoHaptics), Arc::new(NoScreenLock)))
        }

        fn saved(&self) -> Option<hubergamof_modules::pomodoro::SavedTimer> {
            self.state.load(POMODORO_ID)
        }
    }

    fn stored(fixture: &Fixture, modules: Vec<ModuleInstance>) {
        let store = ConfigStore::new(fixture.dir.path(), Arc::clone(&fixture.registry));
        let mut config = store.load_app_modules().unwrap().config;
        config.modules = modules;
        store.save_app_modules(config).unwrap();
    }

    fn instance(id: &str, kind: &str, settings: serde_json::Value) -> ModuleInstance {
        ModuleInstance {
            id: id.into(),
            kind: kind.into(),
            label: None,
            image: None,
            settings,
        }
    }

    #[tokio::test]
    async fn a_radial_action_reaches_the_pomodoro_of_the_application() {
        let fixture = Fixture::new();
        let (host, _task) = fixture.start();
        assert_eq!(
            host.radial_action(&"pomodoro".into(), "toggle").await,
            RadialReply::Done
        );
        assert!(fixture.saved().is_some_and(|saved| saved.is_running()));
        host.radial_action(&"pomodoro".into(), "toggle").await;
        assert!(fixture.saved().is_some_and(|saved| !saved.is_running()));
    }

    #[tokio::test]
    async fn a_module_the_application_does_not_hold_is_not_found() {
        let fixture = Fixture::new();
        let (host, _task) = fixture.start();
        for module in ["audio", "nothing"] {
            assert_eq!(
                host.radial_action(&module.into(), "toggle").await,
                RadialReply::NotHeld
            );
        }
    }

    #[tokio::test]
    async fn nothing_runs_from_the_menu_while_the_computer_is_locked() {
        let fixture = Fixture::new();
        let locked = watch::Sender::new(true);
        let lock = Arc::new(FakeLock(locked.clone()));
        let (host, _task) = ModuleHost::start(fixture.services(Arc::new(NoHaptics), lock));
        // Held, so that the menu does not go looking elsewhere, but refused.
        let reply = host.radial_action(&"pomodoro".into(), "toggle").await;
        assert_eq!(reply, RadialReply::Refused);
        assert!(reply.is_held());
        assert_eq!(fixture.saved(), None);

        locked.send_replace(false);
        host.radial_action(&"pomodoro".into(), "toggle").await;
        assert!(fixture.saved().is_some_and(|saved| saved.is_running()));
    }

    #[tokio::test]
    async fn a_clean_stop_pauses_the_running_pomodoro_where_it_is() {
        let fixture = Fixture::new();
        let (host, task) = fixture.start();
        host.radial_action(&"pomodoro".into(), "toggle").await;
        assert!(fixture.saved().is_some_and(|saved| saved.is_running()));

        host.shutdown().await;
        task.await.unwrap();
        assert!(fixture.saved().is_some_and(|saved| !saved.is_running()));
        // The host is gone: nothing is held any more.
        assert_eq!(
            host.radial_action(&"pomodoro".into(), "toggle").await,
            RadialReply::NotHeld
        );
    }

    #[tokio::test]
    async fn dropping_every_handle_stops_the_host_clean_too() {
        let fixture = Fixture::new();
        let (host, task) = fixture.start();
        host.radial_action(&"pomodoro".into(), "toggle").await;
        drop(host);
        task.await.unwrap();
        assert!(fixture.saved().is_some_and(|saved| !saved.is_running()));
    }

    #[tokio::test]
    async fn the_haptic_events_a_module_declares_reach_the_mice_without_any_device() {
        let fixture = Fixture::new();
        let recorder = Arc::new(Recorder::default());
        let services = fixture.services(
            Arc::clone(&recorder) as Arc<dyn HapticOutput>,
            Arc::new(NoScreenLock),
        );
        let configured = configured_instances(&services);
        let mut hosted = Hosted::new(services, configured);
        let id = ModuleInstanceId(POMODORO_ID.to_owned());
        hosted.run(
            &id,
            vec![
                Command::haptic("step-end"),
                Command::haptic("undeclared"),
                Command::Back,
            ],
        );
        assert_eq!(*recorder.0.lock().unwrap(), ["pomodoro.step-end"]);
    }

    #[test]
    fn the_configured_pomodoro_is_used_and_a_default_one_fills_the_gap() {
        let fixture = Fixture::new();
        let none =
            configured_instances(&fixture.services(Arc::new(NoHaptics), Arc::new(NoScreenLock)));
        assert_eq!(none.len(), 1);
        assert_eq!(
            (none[0].id.0.as_str(), none[0].kind.0.as_str()),
            (POMODORO_ID, "pomodoro")
        );

        stored(
            &fixture,
            vec![instance("mine", "pomodoro", json!({ "workMinutes": 30 }))],
        );
        let mine =
            configured_instances(&fixture.services(Arc::new(NoHaptics), Arc::new(NoScreenLock)));
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].id.0, "mine");
        assert_eq!(mine[0].settings["workMinutes"], 30);
    }

    #[test]
    fn kinds_that_do_not_live_in_the_application_and_second_instances_are_ignored() {
        let fixture = Fixture::new();
        stored(
            &fixture,
            vec![
                instance("a", "audio", json!({})),
                instance("first", "pomodoro", json!({})),
                instance("second", "pomodoro", json!({})),
                instance("ghost", "nothing", json!({})),
            ],
        );
        let kept =
            configured_instances(&fixture.services(Arc::new(NoHaptics), Arc::new(NoScreenLock)));
        let ids: Vec<_> = kept.iter().map(|instance| instance.id.0.as_str()).collect();
        assert_eq!(ids, ["first"]);
    }

    #[test]
    fn an_unreadable_file_leaves_the_defaults() {
        let fixture = Fixture::new();
        std::fs::write(
            fixture
                .dir
                .path()
                .join(crate::config_store::APP_MODULES_FILE),
            b"[",
        )
        .unwrap();
        let kept =
            configured_instances(&fixture.services(Arc::new(NoHaptics), Arc::new(NoScreenLock)));
        assert_eq!(kept.len(), 1);
    }
}
