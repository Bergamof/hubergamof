//! What an entry chosen in the radial menu does: its action runs on the first
//! device that has the module (a Stream Deck: that is where the modules, and
//! their state, live), and otherwise on the instance the application holds
//! (see [`crate::module_host`]): the menu does not need a deck.
//!
//! The actions run one after the other, in the order they were chosen, by one
//! task: two quick choices (mute the microphone, then hang up) must not swap.

use std::sync::Arc;
use std::time::Duration;

use hubergamof_modules::ModuleRegistry;
use hubergamof_protocol::{ModuleKind, RadialEntryConfig, RadialPrimary};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{self, error::TrySendError};

use crate::engine::EngineRegistry;
use crate::module_host::ModuleHost;

/// Choices waiting to run, at most: a person does not choose faster than the
/// devices answer, and a full queue means something is stuck.
const QUEUE_CAPACITY: usize = 8;

/// Longest an action may take to be taken by a device before it is given up.
const ACTION_TIMEOUT: Duration = Duration::from_secs(2);

/// What happened to a choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Queued {
    /// The entry only closes the menu.
    Nothing,
    /// The action will run.
    Accepted,
    /// The module does not declare that action (an entry of an old
    /// configuration, or of another version): nothing runs, and it is said.
    Unknown,
    /// The queue is full: the choice is dropped.
    Dropped,
}

/// Runs the actions of the entries chosen in the menu.
#[derive(Clone)]
pub struct ModuleActions {
    registry: Arc<ModuleRegistry>,
    queue: mpsc::Sender<(ModuleKind, String)>,
}

impl ModuleActions {
    /// Actions of the modules of `registry`, run on the devices of `engines`
    /// by a task of `runtime`.
    #[must_use]
    pub fn new(
        engines: EngineRegistry,
        host: ModuleHost,
        runtime: &Handle,
        registry: Arc<ModuleRegistry>,
    ) -> Self {
        let (queue, mut received) = mpsc::channel::<(ModuleKind, String)>(QUEUE_CAPACITY);
        runtime.spawn(async move {
            while let Some((module, action)) = received.recv().await {
                run_on_first_device(&engines, &host, &module, &action).await;
            }
        });
        Self { registry, queue }
    }

    /// Does what a click on the petal of `entry` does. Never waits: the choice
    /// is queued and the menu thread goes on.
    pub fn run(&self, entry: &RadialEntryConfig) -> Queued {
        let RadialPrimary::Action { action } = &entry.primary else {
            return Queued::Nothing;
        };
        if !self.declares(&entry.module, action) {
            tracing::warn!(
                module = %entry.module,
                action,
                "the radial entry names an action its module does not declare: ignored"
            );
            return Queued::Unknown;
        }
        match self.queue.try_send((entry.module.clone(), action.clone())) {
            Ok(()) => Queued::Accepted,
            Err(TrySendError::Full(_)) => {
                tracing::warn!(module = %entry.module, "radial actions pile up: a choice is dropped");
                Queued::Dropped
            }
            // The task ends with the runtime, at shutdown.
            Err(TrySendError::Closed(_)) => Queued::Dropped,
        }
    }
}

impl ModuleActions {
    fn declares(&self, module: &ModuleKind, action: &str) -> bool {
        self.registry.get(module).is_some_and(|factory| {
            factory
                .radial_actions()
                .iter()
                .any(|declared| declared.id == action)
        })
    }
}

/// Runs `action` on the first device (in the order of the serial numbers) that
/// has `module`, giving up on a device that does not take it within
/// [`ACTION_TIMEOUT`]; when none has it, on the instance of the application.
/// Whether somebody had the module.
async fn run_on_first_device(
    engines: &EngineRegistry,
    host: &ModuleHost,
    module: &ModuleKind,
    action: &str,
) -> bool {
    for engine in engines.handles() {
        match tokio::time::timeout(ACTION_TIMEOUT, engine.radial_action(module, action)).await {
            Ok(true) => return true,
            Ok(false) => {}
            Err(_) => {
                tracing::warn!(%module, action, "a device did not take the radial action in time");
            }
        }
    }
    if host.radial_action(module, action).await {
        return true;
    }
    tracing::warn!(%module, action, "nobody has the module of the radial entry");
    false
}

#[cfg(test)]
mod tests {
    use hubergamof_modules::{BuiltinServices, builtin_registry_with};

    use crate::config_store::ConfigStore;
    use crate::haptics::NoHaptics;
    use crate::module_host::HostServices;
    use crate::screen_lock::NoScreenLock;

    use super::*;

    fn registry() -> Arc<ModuleRegistry> {
        Arc::new(builtin_registry_with(BuiltinServices::simulated()))
    }

    fn entry(module: &str, primary: RadialPrimary) -> RadialEntryConfig {
        RadialEntryConfig {
            module: module.into(),
            enabled: true,
            primary,
        }
    }

    fn action(name: &str) -> RadialPrimary {
        RadialPrimary::Action {
            action: name.to_owned(),
        }
    }

    fn host_of(registry: &Arc<ModuleRegistry>) -> ModuleHost {
        let dir = tempfile::tempdir().unwrap();
        let (host, _task) = ModuleHost::start(HostServices {
            registry: Arc::clone(registry),
            store: ConfigStore::new(dir.path(), Arc::clone(registry)),
            haptics: Arc::new(NoHaptics),
            screen_lock: Arc::new(NoScreenLock),
        });
        host
    }

    fn actions() -> ModuleActions {
        let registry = registry();
        ModuleActions::new(
            EngineRegistry::default(),
            host_of(&registry),
            &Handle::current(),
            registry,
        )
    }

    #[tokio::test]
    async fn an_entry_that_only_closes_the_menu_queues_nothing() {
        assert_eq!(
            actions().run(&entry("pomodoro", RadialPrimary::Close)),
            Queued::Nothing
        );
    }

    #[tokio::test]
    async fn an_action_the_module_declares_is_accepted() {
        assert_eq!(
            actions().run(&entry("pomodoro", action("toggle"))),
            Queued::Accepted
        );
    }

    #[tokio::test]
    async fn an_action_or_a_module_that_is_not_declared_is_ignored_and_said() {
        let actions = actions();
        // An identifier of an older default menu, and a module that is not there.
        assert_eq!(actions.run(&entry("teams", action("mic"))), Queued::Unknown);
        assert_eq!(
            actions.run(&entry("docker", action("start"))),
            Queued::Unknown
        );
        assert_eq!(
            actions.run(&entry("nothing", action("toggle"))),
            Queued::Unknown
        );
    }

    #[tokio::test]
    async fn with_no_device_the_application_runs_the_action() {
        let registry = registry();
        let host = host_of(&registry);
        let engines = EngineRegistry::default();
        assert!(run_on_first_device(&engines, &host, &"pomodoro".into(), "toggle").await);
    }

    #[tokio::test]
    async fn a_module_nobody_holds_runs_nowhere() {
        let registry = registry();
        let host = host_of(&registry);
        let engines = EngineRegistry::default();
        assert!(!run_on_first_device(&engines, &host, &"docker".into(), "start").await);
    }

    #[tokio::test]
    async fn a_full_queue_drops_the_choice_instead_of_waiting() {
        // A queue nobody reads: the first `QUEUE_CAPACITY` choices fit.
        let (queue, _unread) = mpsc::channel(QUEUE_CAPACITY);
        let actions = ModuleActions {
            registry: registry(),
            queue,
        };
        for _ in 0..QUEUE_CAPACITY {
            assert_eq!(
                actions.run(&entry("pomodoro", action("toggle"))),
                Queued::Accepted
            );
        }
        assert_eq!(
            actions.run(&entry("pomodoro", action("toggle"))),
            Queued::Dropped
        );
    }

    #[tokio::test]
    async fn a_queue_whose_task_is_gone_drops_the_choice() {
        let (queue, unread) = mpsc::channel(QUEUE_CAPACITY);
        drop(unread);
        let actions = ModuleActions {
            registry: registry(),
            queue,
        };
        assert_eq!(
            actions.run(&entry("pomodoro", action("toggle"))),
            Queued::Dropped
        );
    }
}
