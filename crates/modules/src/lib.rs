//! Built-in HuBergamof modules and the registry the manager instantiates them
//! from.
//!
//! Each module implements [`hubergamof_module_api::Module`] exactly like an
//! external module would: [`audio`], [`claude`], [`docker`], [`intellij`]
//! and [`pomodoro`] (the manager shows a placeholder key for kinds missing
//! from the registry). Time, date, system load and navigation belong to the
//! touch strip of the host, not to a module.
//!
//! Modules acting on the desktop (audio server, notifications, Docker…)
//! run against the [`BuiltinServices`] given to
//! [`builtin_registry_with`]: those of the system, or simulated ones. There
//! is no default: the caller always chooses (the manager from `--demo`,
//! tests the simulated services).

pub mod audio;
mod background;
pub mod claude;
pub mod desktop;
pub mod docker;
mod icons;
pub mod inbox;
pub mod intellij;
pub mod notifications;
mod paths;
pub mod pomodoro;
mod process;
pub mod session_bus;
pub mod time;
mod trust;

use std::collections::BTreeMap;
use std::sync::Arc;

use hubergamof_module_api::{DialFunctionId, ModuleFactory, ModuleKind};
use serde_json::{Map, Value};

/// Built-in modules shown on the home page of a new device, in key order.
pub const DEFAULT_HOME_MODULES: [&str; 5] = [
    audio::AUDIO_KIND,
    docker::DOCKER_KIND,
    intellij::INTELLIJ_KIND,
    claude::CLAUDE_KIND,
    inbox::INBOX_KIND,
];

/// What a dial of a new device is assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultDial {
    /// A function of a module, whose instance is created with the device.
    Function {
        /// Kind of the module.
        kind: ModuleKind,
        /// Function of that module.
        function: DialFunctionId,
    },
    /// Navigation between the pages (host function).
    Navigation,
    /// Contextual function (host function).
    Contextual,
    /// Nothing.
    Nothing,
}

impl DefaultDial {
    /// The function `function` of the module `kind`.
    #[must_use]
    pub fn function(kind: &str, function: &str) -> Self {
        Self::Function {
            kind: kind.into(),
            function: function.into(),
        }
    }
}

/// Modules placed on a device seen for the first time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefaultLayout {
    /// Kinds shown on the home page, in key order.
    pub home: Vec<ModuleKind>,
    /// What the dials are assigned, from left to right; a device with fewer
    /// dials takes the first ones.
    pub dials: Vec<DefaultDial>,
}

impl DefaultLayout {
    /// Layout of the built-in modules: volume, pomodoro, contextual
    /// function, navigation.
    #[must_use]
    pub fn builtin() -> Self {
        Self {
            home: DEFAULT_HOME_MODULES.map(ModuleKind::from).to_vec(),
            dials: vec![
                DefaultDial::function(audio::AUDIO_KIND, audio::VOLUME_FUNCTION),
                DefaultDial::function(pomodoro::POMODORO_KIND, pomodoro::CONTROL_FUNCTION),
                DefaultDial::Contextual,
                DefaultDial::Navigation,
            ],
        }
    }
}

/// Module factories by kind, and the layout of a new device.
#[derive(Clone, Default)]
pub struct ModuleRegistry {
    factories: BTreeMap<ModuleKind, Arc<dyn ModuleFactory>>,
    layout: DefaultLayout,
}

impl ModuleRegistry {
    /// Registry without any module, with an empty default layout.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Modules placed on a device seen for the first time.
    #[must_use]
    pub fn default_layout(&self) -> &DefaultLayout {
        &self.layout
    }

    /// Replaces the default layout.
    pub fn set_default_layout(&mut self, layout: DefaultLayout) {
        self.layout = layout;
    }

    /// Settings of a new instance of `kind`: those of its factory, or an
    /// empty object for a kind missing from the registry.
    #[must_use]
    pub fn default_settings(&self, kind: &ModuleKind) -> Value {
        self.get(kind).map_or_else(
            || Value::Object(Map::new()),
            |factory| factory.default_settings(),
        )
    }

    /// Registers `factory` under the kind of its descriptor, replacing and
    /// returning a factory already registered for that kind.
    pub fn register(&mut self, factory: Arc<dyn ModuleFactory>) -> Option<Arc<dyn ModuleFactory>> {
        self.factories.insert(factory.descriptor().kind, factory)
    }

    /// Factory of `kind`, if registered.
    #[must_use]
    pub fn get(&self, kind: &ModuleKind) -> Option<&Arc<dyn ModuleFactory>> {
        self.factories.get(kind)
    }

    /// Every registered factory, ordered by kind.
    pub fn factories(&self) -> impl Iterator<Item = &Arc<dyn ModuleFactory>> {
        self.factories.values()
    }
}

impl std::fmt::Debug for ModuleRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.factories.keys()).finish()
    }
}

/// The services the built-in modules act on.
#[derive(Debug, Clone)]
pub struct BuiltinServices {
    /// Audio server of the audio module.
    pub audio: Arc<dyn audio::AudioBackend>,
    /// Docker host of the Docker module.
    pub docker: Arc<dyn docker::DockerBackend>,
    /// Desktop notifications of the pomodoro.
    pub notifier: Arc<dyn notifications::Notifier>,
    /// Monotonic time of the pomodoro, counting suspends.
    pub pomodoro_clock: Arc<dyn time::Clock>,
    /// Desktop session of the modules following applications.
    pub desktop: Arc<dyn desktop::DesktopBackend>,
    /// Installation and recent projects of IntelliJ.
    pub intellij: Arc<dyn intellij::ProjectCatalog>,
}

impl BuiltinServices {
    /// The user's session: `pactl`, the Docker Engine, the session bus and
    /// the desktop (KWin, `/proc`);
    /// Docker and the bus are connected on first use, the bus shared by the
    /// modules that need it.
    #[must_use]
    pub fn system() -> Self {
        let session_bus = session_bus::SessionBus::new();
        Self {
            audio: Arc::new(audio::PactlBackend::new()),
            docker: Arc::new(docker::DockerEngine::new()),
            notifier: Arc::new(notifications::DesktopNotifier::new(session_bus.clone())),
            pomodoro_clock: Arc::new(time::BootClock),
            desktop: Arc::new(desktop::SystemDesktop::new(session_bus)),
            intellij: Arc::new(intellij::SystemCatalog::new()),
        }
    }

    /// Simulations leaving the machine untouched (`--demo`, tests), hermetic
    /// and reproducible: the demo audio server and Docker host, and
    /// notifications that are only logged. The pomodoro's monotonic time is
    /// real.
    #[must_use]
    pub fn simulated() -> Self {
        Self {
            audio: Arc::new(audio::SimulatedAudio::demo()),
            docker: Arc::new(docker::SimulatedDocker::demo()),
            notifier: Arc::new(notifications::SimulatedNotifier::default()),
            pomodoro_clock: Arc::new(time::BootClock),
            desktop: Arc::new(desktop::SimulatedDesktop::demo()),
            intellij: Arc::new(intellij::SimulatedCatalog::demo()),
        }
    }
}

/// Registry of the built-in modules acting on `services`, with their
/// default layout.
#[must_use]
pub fn builtin_registry_with(services: BuiltinServices) -> ModuleRegistry {
    let mut registry = ModuleRegistry::new();
    registry.set_default_layout(DefaultLayout::builtin());
    registry.register(Arc::new(audio::AudioFactory::new(services.audio)));
    registry.register(Arc::new(docker::DockerFactory::new(services.docker)));
    // One desktop hub for every module following an application: the
    // backend follows the windows for one hub only.
    let desktop = desktop::DesktopHub::new(
        services.desktop,
        vec![claude::desktop_app(), intellij::desktop_app()],
    );
    registry.register(Arc::new(claude::ClaudeFactory::new(Arc::clone(&desktop))));
    registry.register(Arc::new(intellij::IntellijFactory::new(
        Arc::clone(&desktop),
        services.intellij,
    )));
    registry.register(Arc::new(inbox::InboxFactory));
    registry.register(Arc::new(pomodoro::PomodoroFactory::new(
        services.notifier,
        services.pomodoro_clock,
    )));
    registry
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hubergamof_module_api::testing::TestContext;
    use hubergamof_module_api::{
        Module, ModuleCapabilities, ModuleEvent, StripWidget, TaskScope, VolumeWidget,
    };
    use serde_json::json;
    use tokio::sync::mpsc::UnboundedReceiver;

    use hubergamof_module_api::ModuleDescriptor;

    use super::*;
    use crate::time::testing::SimulatedClock;

    fn simulated_registry() -> ModuleRegistry {
        builtin_registry_with(BuiltinServices::simulated())
    }

    fn descriptors(registry: &ModuleRegistry) -> Vec<ModuleDescriptor> {
        registry
            .factories()
            .map(|factory| factory.descriptor())
            .collect()
    }

    #[test]
    fn builtin_registry_offers_the_builtin_modules() {
        let registry = simulated_registry();
        let kinds: Vec<_> = descriptors(&registry)
            .into_iter()
            .map(|descriptor| descriptor.kind.0)
            .collect();
        assert_eq!(
            kinds,
            [
                audio::AUDIO_KIND,
                claude::CLAUDE_KIND,
                docker::DOCKER_KIND,
                intellij::INTELLIJ_KIND,
                inbox::INBOX_KIND,
                pomodoro::POMODORO_KIND
            ]
        );
        assert!(registry.get(&"audio".into()).is_some());
        assert!(registry.get(&"unknown".into()).is_none());
        assert_eq!(
            descriptors(&builtin_registry_with(BuiltinServices::system())),
            descriptors(&registry)
        );
    }

    #[test]
    fn every_builtin_module_has_an_svg_icon() {
        for factory in simulated_registry().factories() {
            let icon = factory.icon().expect("built-in modules have an icon");
            let source = std::str::from_utf8(&icon.bytes).unwrap();
            assert!(
                icons::is_complete_svg(source),
                "{}: {source}",
                factory.descriptor().kind.0
            );
        }
    }

    #[test]
    fn each_environment_has_its_own_services() {
        // Building the system services touches nothing: the bus and Docker
        // connect on first use and `pactl` runs when an instance needs it.
        let names = |services: BuiltinServices| {
            [
                format!("{:?}", services.audio),
                format!("{:?}", services.notifier),
                format!("{:?}", services.pomodoro_clock),
                format!("{:?}", services.docker),
                format!("{:?}", services.desktop),
            ]
        };
        let simulated = names(BuiltinServices::simulated());
        assert!(simulated[0].starts_with("SimulatedAudio "), "{simulated:?}");
        assert!(
            simulated[1].starts_with("SimulatedNotifier "),
            "{simulated:?}"
        );
        let system = names(BuiltinServices::system());
        assert!(system[0].starts_with("PactlBackend "), "{system:?}");
        assert!(
            system[1].starts_with("DesktopNotifier { bus: SessionBus { connected: false } }"),
            "{system:?}"
        );
        assert_eq!(simulated[2], "BootClock");
        assert_eq!(system[2], "BootClock");
        assert!(
            simulated[3].starts_with("SimulatedDocker "),
            "{simulated:?}"
        );
        assert!(system[3].starts_with("DockerEngine "), "{system:?}");
        assert!(
            simulated[4].starts_with("SimulatedDesktop "),
            "{simulated:?}"
        );
        assert!(
            system[4].starts_with("SystemDesktop { bus: SessionBus { connected: false }"),
            "{system:?}"
        );
    }

    #[test]
    fn default_settings_come_from_the_factory() {
        let registry = simulated_registry();
        assert_eq!(
            registry.default_settings(&audio::AUDIO_KIND.into()),
            json!({ "volumeStep": 5, "maxVolume": 100 })
        );
        assert_eq!(registry.default_settings(&"unknown".into()), json!({}));
        assert_eq!(
            registry.default_settings(&docker::DOCKER_KIND.into()),
            json!({ "composeFiles": [] })
        );
    }

    #[test]
    fn modules_declare_where_they_go_and_how_often() {
        let registry = simulated_registry();
        let capabilities = |kind: &str| {
            registry
                .get(&kind.into())
                .unwrap_or_else(|| panic!("no module {kind}"))
                .descriptor()
                .capabilities
        };
        let on_a_key = ModuleCapabilities {
            key: true,
            multiple_instances: false,
        };
        assert_eq!(capabilities("pomodoro"), ModuleCapabilities::default());
        assert_eq!(
            capabilities("docker"),
            ModuleCapabilities {
                key: true,
                multiple_instances: true
            }
        );
        for kind in ["audio", "intellij", "claude", "notifications"] {
            assert_eq!(capabilities(kind), on_a_key, "{kind}");
        }
    }

    #[test]
    fn the_default_home_page_only_holds_modules_with_a_key() {
        let registry = simulated_registry();
        for kind in &registry.default_layout().home {
            assert!(
                registry.get(kind).unwrap().descriptor().capabilities.key,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn builtin_layout_fills_home_and_dials() {
        let registry = simulated_registry();
        let layout = registry.default_layout();
        assert_eq!(
            layout.home,
            ["audio", "docker", "intellij", "claude", "notifications"].map(ModuleKind::from)
        );
        assert_eq!(
            layout.dials,
            [
                DefaultDial::function(audio::AUDIO_KIND, audio::VOLUME_FUNCTION),
                DefaultDial::function(pomodoro::POMODORO_KIND, pomodoro::CONTROL_FUNCTION),
                DefaultDial::Contextual,
                DefaultDial::Navigation,
            ]
        );
        assert_eq!(
            ModuleRegistry::new().default_layout(),
            &DefaultLayout::default()
        );
    }

    #[test]
    fn registering_a_kind_twice_replaces_the_factory() {
        let mut registry = simulated_registry();
        let previous = registry.register(Arc::new(inbox::InboxFactory));
        assert!(previous.is_some());
        assert_eq!(
            registry.factories().count(),
            simulated_registry().factories().count()
        );
    }

    /// An instance of `kind` from `registry`, its events and its tasks.
    fn instance(
        registry: &ModuleRegistry,
        kind: &str,
        settings: &Value,
    ) -> (Box<dyn Module>, UnboundedReceiver<ModuleEvent>, TaskScope) {
        let TestContext { ctx, events, scope } = TestContext::new(kind);
        let module = registry
            .get(&kind.into())
            .unwrap()
            .create(ctx, settings)
            .unwrap();
        (module, events, scope)
    }

    /// Lets `duration` pass second by second, handing the events to `module`.
    async fn run_for(
        module: &mut dyn Module,
        events: &mut UnboundedReceiver<ModuleEvent>,
        duration: Duration,
    ) {
        let second = Duration::from_secs(1);
        for _ in 0..duration.as_secs() {
            tokio::time::sleep(second).await;
            while let Ok(event) = events.try_recv() {
                module.on_event(event).await;
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_simulated_audio_is_the_demo_server() {
        let registry = builtin_registry_with(BuiltinServices::simulated());
        let (mut volume, mut events, _tasks) = instance(&registry, audio::AUDIO_KIND, &json!({}));
        run_for(volume.as_mut(), &mut events, Duration::from_secs(2)).await;
        // The speakers of the demo server, at their volume.
        assert_eq!(
            volume.strip_widget(),
            Some(StripWidget::Volume(VolumeWidget::new(40, false)))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn builtin_modules_act_on_the_services_given() {
        let server = Arc::new(audio::SimulatedAudio::demo());
        let notifier = Arc::new(notifications::SimulatedNotifier::default());
        let registry = builtin_registry_with(BuiltinServices {
            audio: Arc::clone(&server) as _,
            docker: Arc::new(docker::SimulatedDocker::empty()),
            notifier: Arc::clone(&notifier) as _,
            pomodoro_clock: Arc::new(SimulatedClock::new()),
            desktop: Arc::new(desktop::SimulatedDesktop::empty()),
            intellij: Arc::new(intellij::SimulatedCatalog::default()),
        });
        let minute = Duration::from_secs(60);

        let (mut volume, mut volume_events, _volume_tasks) =
            instance(&registry, audio::AUDIO_KIND, &json!({}));
        run_for(volume.as_mut(), &mut volume_events, minute).await;
        volume.on_dial_press(&audio::VOLUME_FUNCTION.into()).await;
        run_for(volume.as_mut(), &mut volume_events, minute).await;
        assert!(server.state().sinks[0].muted);

        let (mut timer, mut timer_events, _timer_tasks) = instance(
            &registry,
            pomodoro::POMODORO_KIND,
            &json!({ "workMinutes": 1 }),
        );
        timer.on_key_press().await;
        run_for(timer.as_mut(), &mut timer_events, minute + minute).await;
        assert_eq!(notifier.shown().len(), 1);
    }
}
