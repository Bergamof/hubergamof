//! The modules that live in the application: instances that work whether or
//! not a device is connected.
//!
//! The host runs, in one task, the instances of the kinds that declare
//! [`ModuleLifetime::Application`] (see `config/modules.json`), with the same
//! isolation as the engines (a panic replaces the instance, a handler past its
//! budget is abandoned).
//!
//! Whoever shows or drives such a module goes through the host:
//! - what an instance shows (its key, strip widget, popup and dial hints) is
//!   published as [`ModuleViews`] on a `watch`, so that the engine of a device
//!   reads it as cheaply as it reads an instance of its own;
//! - inputs reach the instance through [`ModuleHost::call`], one at a time like
//!   those of a device;
//! - what an instance asks for outside of an input (a notification when a step
//!   ends) is broadcast to the devices as [`AppCommands`]; its haptic events go
//!   to the mice, with or without a device;
//! - the radial menu, the settings the devices edit and the stop of the
//!   manager come here too.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use hubergamof_module_api::{
    Command, DialFunctionId, DialHint, KeyContent, ModuleInstanceId, ModuleKind, ModuleLifetime,
    PageContent, PageId, ScreenLockState, StripPopup, StripWidget,
};
use hubergamof_modules::ModuleRegistry;
use hubergamof_protocol::{AppModulesConfig, ModuleInstance};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::config_store::{ConfigStore, ConfigStoreError};
use crate::engine::{Call, Instances, MODULE_EVENT_CAPACITY, Outcome, RoutedEvent};
use crate::haptics::HapticOutput;
use crate::screen_lock::ScreenLock;

/// Requests waiting for the host, at most.
const REQUEST_CAPACITY: usize = 16;

/// Command batches waiting for a device that is slow to read them, at most.
const COMMAND_CAPACITY: usize = 64;

/// Longest a device waits for the application to take an input, and longest
/// the application keeps one it has not started: an input the device gave up
/// on must not run later.
pub(crate) const APPLICATION_CALL_TIMEOUT: Duration = Duration::from_secs(2);

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

/// What an instance of the application shows: what a device reads of an
/// instance of its own, as of the last time the instance changed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModuleView {
    /// Key of the home page.
    pub key: KeyContent,
    /// Widget of the touch strip.
    pub strip_widget: Option<StripWidget>,
    /// Menu open over the touch strip.
    pub strip_popup: Option<StripPopup>,
    /// Reminder of what each dial function does, by function.
    pub dial_hints: HashMap<DialFunctionId, DialHint>,
    /// What its petal says in the radial menu.
    pub radial_status: Option<String>,
    /// The badge on the icon of its petal in the radial menu.
    pub radial_badge: Option<u32>,
    /// The pages it offers (see [`hubergamof_module_api::Module::page_ids`]),
    /// as they are now.
    pub pages: BTreeMap<PageId, PageContent>,
    /// Its settings as the instance runs with them.
    pub settings: Value,
}

/// The views of the instances of the application, by module kind.
pub type ModuleViews = BTreeMap<ModuleKind, Arc<ModuleView>>;

/// What an instance of the application asked for outside of an input.
#[derive(Debug, Clone, PartialEq)]
pub struct AppCommands {
    /// The module it belongs to.
    pub kind: ModuleKind,
    /// The commands, in order. Haptic events are not among them: the host
    /// played them.
    pub commands: Vec<Command>,
}

enum Request {
    RadialAction {
        module: ModuleKind,
        action: String,
        reply: oneshot::Sender<RadialReply>,
    },
    Call {
        kind: ModuleKind,
        call: Call,
        /// The page it comes from declares it usable while locked.
        declared: bool,
        /// After this the device has given up: the input is dropped.
        deadline: tokio::time::Instant,
        reply: oneshot::Sender<Option<Outcome>>,
    },
    /// A device found settings for a kind the file does not mention.
    Adopt {
        kind: ModuleKind,
        settings: Value,
    },
    /// A device saved other settings for a kind.
    UpdateSettings {
        kind: ModuleKind,
        settings: Value,
    },
    /// A client of the interface saves other settings for a kind, and wants
    /// to know whether they were taken.
    SetSettings {
        kind: ModuleKind,
        settings: Value,
        reply: oneshot::Sender<Result<(), SettingsRefusal>>,
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

/// Why settings were not taken.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsRefusal {
    /// The application holds no such module.
    #[error("the application holds no such module")]
    NotHeld,
    /// The module, or the file, refuses them.
    #[error("{0}")]
    Refused(String),
    /// The computer is locked: nothing is changed from the interface.
    #[error("the computer is locked")]
    Locked,
    /// They could not be written: nothing was changed.
    #[error("the settings could not be saved")]
    NotSaved,
}

/// Entry point of the host: cheap to clone.
#[derive(Clone)]
pub struct ModuleHost {
    requests: mpsc::Sender<Request>,
    views: watch::Receiver<ModuleViews>,
    commands: broadcast::Sender<AppCommands>,
}

impl ModuleHost {
    /// Starts the host on the current Tokio runtime. The configuration is
    /// read by the task, off the async threads: requests sent meanwhile wait.
    #[must_use]
    pub fn start(services: HostServices) -> (Self, JoinHandle<()>) {
        let (requests, received) = mpsc::channel(REQUEST_CAPACITY);
        let (views_tx, views) = watch::channel(ModuleViews::new());
        let (commands, _) = broadcast::channel(COMMAND_CAPACITY);
        let host = Self {
            requests,
            views,
            commands: commands.clone(),
        };
        let task = tokio::spawn(async move {
            let for_reading = services.clone();
            let config = tokio::task::spawn_blocking(move || read_config(&for_reading))
                .await
                .unwrap_or_else(|error| {
                    tracing::error!(%error, "application modules not read, defaults used");
                    AppModulesConfig::empty()
                });
            let hosted = Hosted::new(services, config, views_tx, commands);
            run(hosted, received).await;
        });
        (host, task)
    }

    /// What the instances of the application show, followed as it changes.
    #[must_use]
    pub fn views(&self) -> watch::Receiver<ModuleViews> {
        self.views.clone()
    }

    /// What the instances ask for outside of an input, as it happens: a
    /// device that reads too slowly misses some (and is told).
    #[must_use]
    pub fn subscribe_commands(&self) -> broadcast::Receiver<AppCommands> {
        self.commands.subscribe()
    }

    /// Hands `call`, an input of a device, to the instance of `kind`. `None`
    /// when the host holds none or is gone. The host plays the haptic events
    /// itself: they are not in the commands of the outcome. `declared`: the
    /// page the input comes from declares it usable while the computer is
    /// locked, which the device checked (see [`Module::page`]'s
    /// `lock_safe_pages`).
    pub(crate) async fn call(
        &self,
        kind: &ModuleKind,
        call: Call,
        declared: bool,
    ) -> Option<Outcome> {
        let (reply, answer) = oneshot::channel();
        let request = Request::Call {
            kind: kind.clone(),
            call,
            declared,
            deadline: tokio::time::Instant::now() + APPLICATION_CALL_TIMEOUT,
            reply,
        };
        self.requests.send(request).await.ok()?;
        answer.await.ok().flatten()
    }

    /// A device has settings for `kind`: kept if the file has none yet. Never
    /// waits: a full queue drops it (said).
    pub fn adopt(&self, kind: &ModuleKind, settings: &Value) {
        self.post(Request::Adopt {
            kind: kind.clone(),
            settings: settings.clone(),
        });
    }

    /// A device saved other settings for `kind`: they replace the stored
    /// ones. Never waits, like [`Self::adopt`].
    pub fn update_settings(&self, kind: &ModuleKind, settings: &Value) {
        self.post(Request::UpdateSettings {
            kind: kind.clone(),
            settings: settings.clone(),
        });
    }

    /// Sends `request` without waiting: when the queue is full a task waits
    /// for room instead, so that a settings change is never lost.
    /// Saves `settings` for `kind` and says whether the module took them. The
    /// last to save wins, a device or a client of the interface.
    ///
    /// # Errors
    ///
    /// [`SettingsRefusal`]: no such module, or settings it refuses.
    pub async fn set_settings(
        &self,
        kind: &ModuleKind,
        settings: &Value,
    ) -> Result<(), SettingsRefusal> {
        let (reply, answer) = oneshot::channel();
        let request = Request::SetSettings {
            kind: kind.clone(),
            settings: settings.clone(),
            reply,
        };
        if self.requests.send(request).await.is_err() {
            return Err(SettingsRefusal::NotHeld);
        }
        answer.await.unwrap_or(Err(SettingsRefusal::NotHeld))
    }

    fn post(&self, request: Request) {
        match self.requests.try_send(request) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(request)) => {
                let requests = self.requests.clone();
                tokio::spawn(async move {
                    let _ = requests.send(request).await;
                });
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::debug!("the module host is gone: a settings change is dropped");
            }
        }
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
    registry: Arc<ModuleRegistry>,
    store: ConfigStore,
    /// The file as stored: only the kinds it mentions.
    config: AppModulesConfig,
    /// What runs: the file's instances and a default for each other kind.
    configured: Vec<ModuleInstance>,
    /// Instance of each kind that lives in the application.
    by_kind: BTreeMap<ModuleKind, ModuleInstanceId>,
    /// The dial functions of each of those kinds, whose hints are published.
    dial_functions: BTreeMap<ModuleKind, Vec<DialFunctionId>>,
    events: mpsc::Receiver<RoutedEvent>,
    haptics: Arc<dyn HapticOutput>,
    locked: watch::Receiver<bool>,
    views: watch::Sender<ModuleViews>,
    commands: broadcast::Sender<AppCommands>,
}

impl Hosted {
    fn new(
        services: HostServices,
        config: AppModulesConfig,
        views: watch::Sender<ModuleViews>,
        commands: broadcast::Sender<AppCommands>,
    ) -> Self {
        let (events_tx, events) = mpsc::channel(MODULE_EVENT_CAPACITY);
        let locked = services.screen_lock.watch();
        let screen = locked.clone();
        let mut instances = Instances::detached(
            Arc::clone(&services.registry),
            events_tx,
            ScreenLockState::new(move || *screen.borrow()),
        );
        let configured = instances_for(&config, &services.registry);
        instances.sync(&configured);
        let mut hosted = Self {
            instances,
            registry: services.registry,
            store: services.store,
            config,
            by_kind: BTreeMap::new(),
            dial_functions: BTreeMap::new(),
            configured,
            events,
            haptics: services.haptics,
            locked,
            views,
            commands,
        };
        hosted.index_kinds();
        hosted.publish();
        hosted
    }

    fn index_kinds(&mut self) {
        self.by_kind = self
            .configured
            .iter()
            .map(|instance| (instance.kind.clone(), instance.id.clone()))
            .collect();
        self.dial_functions = self
            .by_kind
            .keys()
            .map(|kind| {
                let functions = self.registry.get(kind).map_or_else(Vec::new, |factory| {
                    factory
                        .descriptor()
                        .dial_functions
                        .into_iter()
                        .map(|function| function.id)
                        .collect()
                });
                (kind.clone(), functions)
            })
            .collect();
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
        if let Some(Outcome::Done(commands)) = self
            .instances
            .call(&id, Call::RadialAction(action), false)
            .await
        {
            self.run(&id, commands);
        }
        RadialReply::Done
    }

    /// An input of a device: its commands go back to that device, but the
    /// haptic events are played here, once.
    async fn call(
        &mut self,
        kind: &ModuleKind,
        call: Call,
        declared: bool,
        deadline: tokio::time::Instant,
    ) -> Option<Outcome> {
        let id = self.by_kind.get(kind).cloned()?;
        // The device decided before the computer locked, or gave up: an input
        // that waited in the queue does not run now.
        if tokio::time::Instant::now() >= deadline {
            tracing::debug!(%id, "input dropped: the device gave up on it");
            return None;
        }
        let may_run_locked =
            self.usable_when_locked(kind) || (declared && self.lock_safe_pages(kind));
        if *self.locked.borrow() && !may_run_locked {
            tracing::debug!(%id, "input dropped: the computer is locked");
            return None;
        }
        let outcome = self.instances.call(&id, call, declared).await?;
        Some(match outcome {
            Outcome::Done(commands) => Outcome::Done(self.play_haptics(&id, commands)),
            Outcome::Failed => Outcome::Failed,
        })
    }

    async fn on_event(&mut self, routed: RoutedEvent) {
        let id = routed.module.clone();
        if let Some(Outcome::Done(commands)) = self.instances.dispatch(routed).await {
            self.run(&id, commands);
        }
    }

    /// What the commands of an instance do outside of an input: the haptic
    /// events go to the mice, the rest to the devices.
    fn run(&mut self, source: &ModuleInstanceId, commands: Vec<Command>) {
        let rest = self.play_haptics(source, commands);
        let Some(kind) = self.kind_of(source) else {
            return;
        };
        // A module of the application has no pages and no device of its own to
        // navigate: only what can be shown on any device is passed on.
        let (rest, navigation): (Vec<_>, Vec<_>) = rest.into_iter().partition(|command| {
            matches!(
                command,
                Command::Notify(_)
                    | Command::ShowNotification(_)
                    | Command::Feedback(_)
                    | Command::FeedbackOn(..)
            )
        });
        if !navigation.is_empty() {
            tracing::warn!(module = %source, dropped = navigation.len(), "navigation commands of a module of the application ignored");
        }
        if !rest.is_empty() {
            // Nobody listening is fine: no device, nobody to show it to.
            let _ = self.commands.send(AppCommands {
                kind,
                commands: rest,
            });
        }
    }

    /// Plays the haptic events among `commands`, the declared ones only, and
    /// returns the others.
    fn play_haptics(&self, source: &ModuleInstanceId, commands: Vec<Command>) -> Vec<Command> {
        let mut rest = Vec::with_capacity(commands.len());
        for command in commands {
            match command {
                Command::Haptic(event) => match self.instances.haptic_play(source, &event) {
                    Some(play) => self.haptics.play(&play),
                    None => {
                        tracing::warn!(module = %source, %event, "undeclared haptic event dropped");
                    }
                },
                other => rest.push(other),
            }
        }
        rest
    }

    fn kind_of(&self, id: &ModuleInstanceId) -> Option<ModuleKind> {
        self.by_kind
            .iter()
            .find(|(_, held)| *held == id)
            .map(|(kind, _)| kind.clone())
    }

    /// Whether the module of `kind` declares itself usable while the computer
    /// is locked.
    fn usable_when_locked(&self, kind: &ModuleKind) -> bool {
        self.registry
            .get(kind)
            .is_some_and(|factory| factory.descriptor().capabilities.usable_when_locked)
    }

    /// Whether the pages of the module of `kind` may declare what stays
    /// usable while the computer is locked.
    fn lock_safe_pages(&self, kind: &ModuleKind) -> bool {
        self.registry
            .get(kind)
            .is_some_and(|factory| factory.descriptor().capabilities.lock_safe_pages)
    }

    /// A device found `settings` for `kind`: kept when the file has nothing
    /// for it, so that what was configured on a deck before the module moved
    /// to the application is not lost. The first device to say wins.
    async fn adopt(&mut self, kind: ModuleKind, settings: Value) {
        if self.config.modules.iter().any(|stored| stored.kind == kind) {
            return;
        }
        // Refused settings are logged by `set_settings`; a device has no use of
        // the reason.
        let _ = self.set_settings(kind, settings).await;
    }

    /// Replaces the settings of `kind`, in the running instance and in the
    /// file, when the module and the file accept them, and only once they are
    /// written: settings that cannot be kept are not run with. The last device
    /// to save wins.
    async fn set_settings(
        &mut self,
        kind: ModuleKind,
        settings: Value,
    ) -> Result<(), SettingsRefusal> {
        let Some(id) = self.by_kind.get(&kind).cloned() else {
            return Err(SettingsRefusal::NotHeld);
        };
        // The file is global: settings the module refuses are not written
        // (a deck's stored configuration may hold some).
        let factory = self.registry.get(&kind).ok_or(SettingsRefusal::NotHeld)?;
        if let Err(error) = factory.validate_settings(&settings) {
            tracing::warn!(%kind, %error, "settings of the application module refused: not stored");
            return Err(SettingsRefusal::Refused(error.to_string()));
        }
        let unchanged = self
            .configured
            .iter()
            .any(|instance| instance.kind == kind && instance.settings == settings);
        if unchanged && self.config.modules.iter().any(|stored| stored.kind == kind) {
            return Ok(());
        }
        let mut candidate = self.config.clone();
        match candidate
            .modules
            .iter_mut()
            .find(|existing| existing.kind == kind)
        {
            Some(existing) => existing.settings = settings,
            None => candidate.modules.push(ModuleInstance {
                id,
                kind,
                label: None,
                image: None,
                settings,
            }),
        }
        self.config = save_config(self.store.clone(), candidate).await?;
        self.configured = instances_for(&self.config, &self.registry);
        self.instances.sync(&self.configured);
        Ok(())
    }

    /// The settings `kind` runs with.
    fn settings_of(&self, kind: &ModuleKind) -> Value {
        self.configured
            .iter()
            .find(|instance| instance.kind == *kind)
            .map_or(Value::Null, |instance| instance.settings.clone())
    }

    /// Publishes what the instances show now, if it changed. The instances
    /// reach the host through events (a tick), never through a redraw
    /// request: the host publishes after each of them.
    fn publish(&mut self) {
        let _ = self.instances.take_failures();
        let mut views = ModuleViews::new();
        for (kind, id) in &self.by_kind {
            let functions = self.dial_functions.get(kind).map_or(&[][..], Vec::as_slice);
            let dial_hints = functions
                .iter()
                .filter_map(|function| {
                    let hint = self.instances.dial_hint(id, function)?;
                    Some((function.clone(), hint))
                })
                .collect();
            views.insert(
                kind.clone(),
                Arc::new(ModuleView {
                    key: self.instances.key(id).unwrap_or_default(),
                    strip_widget: self.instances.strip_widget(id),
                    strip_popup: self.instances.strip_popup(id),
                    dial_hints,
                    radial_status: self.instances.radial_status(id),
                    radial_badge: self.instances.radial_badge(id),
                    pages: self.instances.pages(id),
                    settings: self.settings_of(kind),
                }),
            );
        }
        self.views.send_if_modified(|current| {
            let changed = *current != views;
            if changed {
                *current = views;
            }
            changed
        });
    }
}

/// Writes `config`, off the async threads, and gives back the file as
/// stored; a file that is too big or invalid is refused, a failure to
/// write is logged.
async fn save_config(
    store: ConfigStore,
    config: AppModulesConfig,
) -> Result<AppModulesConfig, SettingsRefusal> {
    match tokio::task::spawn_blocking(move || store.save_app_modules(config)).await {
        Ok(Ok(saved)) => Ok(saved),
        Ok(Err(ConfigStoreError::InvalidAppModules(error))) => {
            Err(SettingsRefusal::Refused(error.to_string()))
        }
        Ok(Err(error)) => {
            tracing::warn!(%error, "application modules not saved");
            Err(SettingsRefusal::NotSaved)
        }
        Err(error) => {
            tracing::error!(%error, "saving the application modules failed");
            Err(SettingsRefusal::NotSaved)
        }
    }
}

/// The configuration of the application modules, set aside and replaced by an
/// empty one when unreadable (logged). Blocking: call it off the async threads.
fn read_config(services: &HostServices) -> AppModulesConfig {
    match services.store.load_app_modules() {
        Ok(loaded) => loaded.config,
        Err(error) => {
            tracing::warn!(%error, "application modules not loaded, defaults used");
            AppModulesConfig::empty()
        }
    }
}

/// The instances the host runs: those of `config` whose kind lives in the
/// application, and a default one for each such kind it does not mention.
fn instances_for(config: &AppModulesConfig, registry: &ModuleRegistry) -> Vec<ModuleInstance> {
    let lives_in_application = |kind: &ModuleKind| {
        registry
            .get(kind)
            .is_some_and(|factory| factory.lifetime() == ModuleLifetime::Application)
    };
    let mut instances: Vec<ModuleInstance> = Vec::new();
    for instance in &config.modules {
        if !lives_in_application(&instance.kind) {
            tracing::warn!(module = %instance.id, kind = %instance.kind, "not a module of the application: ignored");
        } else if instances.iter().any(|kept| kept.kind == instance.kind) {
            tracing::warn!(module = %instance.id, kind = %instance.kind, "a second instance of a module of the application: ignored");
        } else {
            instances.push(instance.clone());
        }
    }
    let present: BTreeSet<ModuleKind> = instances.iter().map(|kept| kept.kind.clone()).collect();
    for factory in registry.factories() {
        let kind = factory.descriptor().kind;
        if factory.lifetime() == ModuleLifetime::Application && !present.contains(&kind) {
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
                Some(Request::Call { kind, call, declared, deadline, reply }) => {
                    let outcome = hosted.call(&kind, call, declared, deadline).await;
                    let _ = reply.send(outcome);
                }
                Some(Request::Adopt { kind, settings }) => hosted.adopt(kind, settings).await,
                Some(Request::UpdateSettings { kind, settings }) => {
                    let _ = hosted.set_settings(kind, settings).await;
                }
                Some(Request::SetSettings { kind, settings, reply }) => {
                    // Checked here too, not only by the server: the request may
                    // have waited while the computer locked.
                    let answer = if *hosted.locked.borrow() {
                        Err(SettingsRefusal::Locked)
                    } else {
                        hosted.set_settings(kind, settings).await
                    };
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
        hosted.publish();
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

    use hubergamof_module_api::Feedback;
    use hubergamof_modules::pomodoro::{MemoryStateStore, PomodoroStateStore};
    use hubergamof_modules::{BuiltinServices, builtin_registry_with};
    use serde_json::json;
    use tokio::sync::watch;

    use super::*;
    use crate::haptics::{HapticPlay, NoHaptics};
    use crate::screen_lock::NoScreenLock;

    fn configured_instances(services: &HostServices) -> Vec<ModuleInstance> {
        instances_for(&read_config(services), &services.registry)
    }

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
        for module in ["docker", "nothing"] {
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
        let config = read_config(&services);
        let (views, _) = watch::channel(ModuleViews::new());
        let (commands, mut heard) = broadcast::channel(COMMAND_CAPACITY);
        let mut hosted = Hosted::new(services, config, views, commands);
        let id = ModuleInstanceId(POMODORO_ID.to_owned());
        hosted.run(
            &id,
            vec![
                Command::haptic("step-end"),
                Command::haptic("undeclared"),
                Command::Feedback(Feedback::Ok),
            ],
        );
        assert_eq!(*recorder.0.lock().unwrap(), ["pomodoro.step-end"]);
        // What is not a haptic event goes to the devices.
        assert_eq!(
            heard.try_recv().unwrap(),
            AppCommands {
                kind: "pomodoro".into(),
                commands: vec![Command::Feedback(Feedback::Ok)],
            }
        );
        assert!(heard.try_recv().is_err());

        // Navigation means nothing without pages: it is not passed on.
        hosted.run(&id, vec![Command::GoHome, Command::Back]);
        assert!(heard.try_recv().is_err());
    }

    #[tokio::test]
    async fn the_first_settings_a_device_offers_are_kept_and_later_offers_are_not() {
        let fixture = Fixture::new();
        let (host, _task) = fixture.start();
        host.adopt(&"pomodoro".into(), &json!({ "workMinutes": 30 }));
        host.adopt(&"pomodoro".into(), &json!({ "workMinutes": 45 }));
        // A round trip: the requests are handled in the order they were sent.
        host.radial_action(&"nothing".into(), "x").await;
        let stored = ConfigStore::new(fixture.dir.path(), Arc::clone(&fixture.registry))
            .load_app_modules()
            .unwrap()
            .config
            .modules;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].settings["workMinutes"], 30);
    }

    #[tokio::test]
    async fn the_settings_a_device_saves_replace_the_stored_ones() {
        let fixture = Fixture::new();
        let (host, _task) = fixture.start();
        host.adopt(&"pomodoro".into(), &json!({ "workMinutes": 30 }));
        host.update_settings(&"pomodoro".into(), &json!({ "workMinutes": 45 }));
        host.update_settings(&"docker".into(), &json!({}));
        host.radial_action(&"nothing".into(), "x").await;
        let store = ConfigStore::new(fixture.dir.path(), Arc::clone(&fixture.registry));
        let stored = store.load_app_modules().unwrap().config;
        assert_eq!(stored.modules.len(), 1, "a kind of a device is not stored");
        assert_eq!(stored.modules[0].settings["workMinutes"], 45);
        assert_eq!(stored.revision, 2);
    }

    #[tokio::test]
    async fn settings_the_module_refuses_are_not_stored() {
        let fixture = Fixture::new();
        let (host, _task) = fixture.start();
        host.adopt(&"pomodoro".into(), &json!({ "workMinutes": 0 }));
        host.update_settings(&"pomodoro".into(), &json!({ "workMinutes": "long" }));
        host.radial_action(&"nothing".into(), "x").await;
        let store = ConfigStore::new(fixture.dir.path(), Arc::clone(&fixture.registry));
        let stored = store.load_app_modules().unwrap().config.modules;
        assert_eq!(stored.len(), 0);
    }

    #[tokio::test]
    async fn settings_too_big_for_the_file_are_refused_and_change_nothing() {
        let fixture = Fixture::new();
        let (host, _task) = fixture.start();
        let mut views = host.views();
        views
            .wait_for(|views| views.contains_key(&"pomodoro".into()))
            .await
            .unwrap();
        let before = views
            .borrow()
            .get(&"pomodoro".into())
            .unwrap()
            .settings
            .clone();

        // A key the module does not know passes its own check, not the file's.
        let huge = json!({ "note": "x".repeat(32 * 1024) });
        let refusal = host.set_settings(&"pomodoro".into(), &huge).await;
        assert!(
            matches!(refusal, Err(SettingsRefusal::Refused(_))),
            "{refusal:?}"
        );

        let after = views
            .borrow()
            .get(&"pomodoro".into())
            .unwrap()
            .settings
            .clone();
        assert_eq!(
            after, before,
            "the instance keeps running with its settings"
        );
        let store = ConfigStore::new(fixture.dir.path(), Arc::clone(&fixture.registry));
        assert_eq!(store.load_app_modules().unwrap().config.modules.len(), 0);
    }

    #[tokio::test]
    async fn settings_asked_for_by_a_client_are_refused_while_the_computer_is_locked() {
        let fixture = Fixture::new();
        let locked = watch::Sender::new(true);
        let (host, _task) = ModuleHost::start(
            fixture.services(Arc::new(NoHaptics), Arc::new(FakeLock(locked.clone()))),
        );
        let settings = json!({ "workMinutes": 30 });
        assert_eq!(
            host.set_settings(&"pomodoro".into(), &settings).await,
            Err(SettingsRefusal::Locked)
        );
        locked.send_replace(false);
        assert_eq!(
            host.set_settings(&"pomodoro".into(), &settings).await,
            Ok(())
        );
    }

    #[tokio::test]
    async fn an_input_that_waited_too_long_or_met_a_locked_computer_does_not_run() {
        let fixture = Fixture::new();
        let locked = watch::Sender::new(false);
        let lock = Arc::new(FakeLock(locked.clone()));
        let services = fixture.services(Arc::new(NoHaptics), lock);
        let config = read_config(&services);
        let (views, _) = watch::channel(ModuleViews::new());
        let (commands, _) = broadcast::channel(COMMAND_CAPACITY);
        let mut hosted = Hosted::new(services, config, views, commands);
        let kind: ModuleKind = "pomodoro".into();
        let now = tokio::time::Instant::now();

        // The device gave up on it.
        let late = hosted
            .call(&kind, Call::RadialAction("toggle".into()), false, now)
            .await;
        assert!(late.is_none());
        assert_eq!(fixture.saved(), None);

        // The pomodoro is usable while locked: the computer being locked
        // is no reason, and the input of a device runs.
        locked.send_replace(true);
        let later = now + Duration::from_secs(60);
        let done = hosted
            .call(&kind, Call::RadialAction("toggle".into()), false, later)
            .await;
        assert!(matches!(done, Some(Outcome::Done(_))));
        assert!(fixture.saved().is_some_and(|saved| saved.is_running()));
    }

    #[tokio::test]
    async fn the_views_follow_the_instances_and_an_input_of_a_device_is_taken() {
        let fixture = Fixture::new();
        let (host, _task) = fixture.start();
        let mut views = host.views();
        views
            .wait_for(|views| views.contains_key(&"pomodoro".into()))
            .await
            .unwrap();
        let before = views.borrow().get(&"pomodoro".into()).unwrap().key.clone();
        assert_eq!(
            views
                .borrow()
                .get(&"pomodoro".into())
                .unwrap()
                .radial_status
                .as_deref(),
            Some("Focus 1/4\n25:00 · prêt")
        );

        let outcome = host
            .call(
                &"pomodoro".into(),
                Call::RadialAction("toggle".into()),
                false,
            )
            .await;
        assert!(matches!(outcome, Some(Outcome::Done(_))));
        views
            .wait_for(|views| views.get(&"pomodoro".into()).unwrap().key != before)
            .await
            .unwrap();
        assert!(
            host.call(&"docker".into(), Call::KeyPress, false)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn the_pages_a_module_lists_are_shared_in_its_view() {
        let fixture = Fixture::new();
        let (host, _task) = fixture.start();
        let mut views = host.views();
        views
            .wait_for(|views| views.contains_key(&"audio".into()))
            .await
            .unwrap();
        let view = views.borrow().get(&"audio".into()).cloned().unwrap();
        assert!(
            view.pages.contains_key(&PageId("audio".into())),
            "{:?}",
            view.pages.keys()
        );
        // The module that lists none shares none.
        let pomodoro = views.borrow().get(&"pomodoro".into()).cloned().unwrap();
        assert!(pomodoro.pages.is_empty());
        assert_eq!(pomodoro.radial_badge, None);
    }

    #[test]
    fn the_configured_pomodoro_is_used_and_a_default_one_fills_the_gap() {
        let fixture = Fixture::new();
        let none =
            configured_instances(&fixture.services(Arc::new(NoHaptics), Arc::new(NoScreenLock)));
        let default = none
            .iter()
            .find(|instance| instance.kind.0 == "pomodoro")
            .expect("a default pomodoro");
        assert_eq!(default.id.0, POMODORO_ID);

        stored(
            &fixture,
            vec![instance("mine", "pomodoro", json!({ "workMinutes": 30 }))],
        );
        let mine =
            configured_instances(&fixture.services(Arc::new(NoHaptics), Arc::new(NoScreenLock)));
        let pomodoros: Vec<_> = mine
            .iter()
            .filter(|instance| instance.kind.0 == "pomodoro")
            .collect();
        assert_eq!(pomodoros.len(), 1);
        assert_eq!(pomodoros[0].id.0, "mine");
        assert_eq!(pomodoros[0].settings["workMinutes"], 30);
    }

    #[test]
    fn kinds_that_do_not_live_in_the_application_and_second_instances_are_ignored() {
        let fixture = Fixture::new();
        stored(
            &fixture,
            vec![
                instance("a", "docker", json!({})),
                instance("first", "pomodoro", json!({})),
                instance("second", "pomodoro", json!({})),
                instance("ghost", "nothing", json!({})),
            ],
        );
        let kept =
            configured_instances(&fixture.services(Arc::new(NoHaptics), Arc::new(NoScreenLock)));
        let ids: Vec<_> = kept.iter().map(|instance| instance.id.0.as_str()).collect();
        assert!(ids.contains(&"first"), "{ids:?}");
        for ignored in ["a", "second", "ghost"] {
            assert!(!ids.contains(&ignored), "{ignored} is ignored: {ids:?}");
        }
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
        let application_kinds = fixture
            .registry
            .factories()
            .filter(|factory| factory.lifetime() == ModuleLifetime::Application)
            .count();
        assert_eq!(kept.len(), application_kinds);
        assert!(
            kept.iter()
                .all(|instance| instance.id.0.starts_with("app-"))
        );
    }
}
