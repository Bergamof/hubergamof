//! Live module instances of a device, kept in sync with its configuration,
//! and the only place module code is called from: every call is isolated
//! (a panic replaces the instance by a placeholder) and every handler runs
//! within [`HANDLER_BUDGET`].

use std::collections::BTreeMap;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::FutureExt as _;
use hubergamof_module_api::ModuleLifetime;
use hubergamof_module_api::{
    Command, DialFunctionId, DialHint, EventSender, HANDLER_BUDGET, ImageLibrary, ItemId,
    KeyContent, Module, ModuleContext, ModuleError, ModuleEvent, ModuleInstanceId, ModuleKind,
    NotificationId, PageContent, PageId, RedrawHandle, ScreenLockState, StripPopup, StripWidget,
    TaskScope, UnreadNotifications,
};
use hubergamof_modules::ModuleRegistry;
use hubergamof_protocol::ModuleInstance;
use serde_json::Value;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, watch};

use crate::module_host::{APPLICATION_CALL_TIMEOUT, ModuleHost, ModuleView, ModuleViews};

/// A getter taking longer than this is logged: getters run on every redraw.
const SLOW_GETTER_THRESHOLD: Duration = Duration::from_millis(1);

/// Suffix of the label of a module that cannot run.
const UNAVAILABLE_SUFFIX: &str = "indisponible";

/// Stand-in for a module whose kind is unknown, whose creation failed or
/// which panicked.
struct UnavailableModule {
    label: String,
}

#[async_trait]
impl Module for UnavailableModule {
    fn key(&self) -> KeyContent {
        KeyContent::with_label(self.label.clone())
    }

    fn update_settings(&mut self, _settings: &Value) -> Result<(), ModuleError> {
        Ok(())
    }
}

/// An instance of the application that a device places: the device keeps its
/// place (key, dial, strip) and its settings as the user last saved them there.
struct Shared {
    kind: ModuleKind,
    settings: Value,
}

/// Alive exactly as long as the module it was issued for: the event senders
/// of a module hold a [`Weak`] to it.
type Liveness = Arc<()>;

/// An event sent by a module instance, routed back to it by the engine.
pub(crate) struct RoutedEvent {
    pub(crate) module: ModuleInstanceId,
    /// Liveness of the sending module: events queued before it was removed
    /// or replaced are dropped.
    sender: Weak<()>,
    event: ModuleEvent,
}

/// Input or event handed to a module handler.
pub(crate) enum Call {
    KeyPress,
    ItemPress(PageId, ItemId),
    DialRotate(DialFunctionId, i32),
    DialPress(DialFunctionId),
    ContextRotate(PageId, i32),
    ContextPress(PageId),
    ContextLongPress(PageId),
    NotificationPress(NotificationId),
    RadialAction(String),
    Event(ModuleEvent),
}

/// How a handler ended.
pub(crate) enum Outcome {
    /// It returned these commands.
    Done(Vec<Command>),
    /// It panicked (the instance was replaced) or exceeded its budget.
    Failed,
}

struct Instance {
    kind: ModuleKind,
    /// Settings last accepted by the module.
    settings: Value,
    /// Whether `module` is the real module rather than a placeholder.
    available: bool,
    /// Held, never read: its event senders work while it lives.
    _alive: Liveness,
    module: Box<dyn Module>,
    /// Background tasks of the module, aborted when the instance is dropped.
    tasks: TaskScope,
}

impl Instance {
    /// Placeholder for a module of `kind` that cannot run: no task, and a
    /// new liveness, so that nothing of a failed module survives.
    fn placeholder(kind: ModuleKind, settings: Value) -> Self {
        Self {
            module: Box::new(UnavailableModule {
                label: format!("{kind} {UNAVAILABLE_SUFFIX}"),
            }),
            kind,
            settings,
            available: false,
            _alive: Liveness::default(),
            tasks: TaskScope::new(),
        }
    }

    fn replace_with_placeholder(&mut self) {
        *self = Self::placeholder(self.kind.clone(), self.settings.clone());
    }
}

/// Module instances of a device by id.
pub(crate) struct Instances {
    registry: Arc<ModuleRegistry>,
    redraw: RedrawHandle,
    events: mpsc::Sender<RoutedEvent>,
    unread: UnreadNotifications,
    images: ImageLibrary,
    /// Whether the computer is locked, as modules ask it.
    screen_lock: ScreenLockState,
    /// Whether the event queue was found full and not sent to since: the
    /// overflow is logged once, when it starts.
    queue_full: Arc<AtomicBool>,
    live: BTreeMap<ModuleInstanceId, Instance>,
    /// The application, when this device places modules that live there:
    /// those have no instance here, only a place.
    application: Option<ModuleHost>,
    views: Option<watch::Receiver<ModuleViews>>,
    shared: BTreeMap<ModuleInstanceId, Shared>,
    /// Instances replaced after a panic outside of a handler, not yet
    /// reported.
    failures: Vec<ModuleInstanceId>,
}

impl Instances {
    pub(crate) fn new(
        registry: Arc<ModuleRegistry>,
        redraw: RedrawHandle,
        events: mpsc::Sender<RoutedEvent>,
        unread: UnreadNotifications,
        images: ImageLibrary,
        screen_lock: ScreenLockState,
    ) -> Self {
        Self {
            registry,
            redraw,
            events,
            unread,
            images,
            screen_lock,
            queue_full: Arc::default(),
            live: BTreeMap::new(),
            application: None,
            views: None,
            shared: BTreeMap::new(),
            failures: Vec::new(),
        }
    }

    /// The modules that live in the application are not instantiated here but
    /// placed: what they show is read from `application`, their inputs are
    /// handed to it, and the settings saved on this device reach it.
    pub(crate) fn with_application(mut self, application: Option<ModuleHost>) -> Self {
        self.views = application.as_ref().map(ModuleHost::views);
        self.application = application;
        self
    }

    /// Whether `kind` lives in the application, which this device has.
    fn lives_in_application(&self, kind: &ModuleKind) -> bool {
        self.application.is_some()
            && self
                .registry
                .get(kind)
                .is_some_and(|factory| factory.lifetime() == ModuleLifetime::Application)
    }

    /// Kind of the module of instance `id`, local or placed.
    fn kind_of(&self, id: &ModuleInstanceId) -> Option<&ModuleKind> {
        self.live
            .get(id)
            .map(|instance| &instance.kind)
            .or_else(|| self.shared.get(id).map(|shared| &shared.kind))
    }

    /// What the application shows for the placed instance `id`.
    fn view(&self, id: &ModuleInstanceId) -> Option<Arc<ModuleView>> {
        let kind = &self.shared.get(id)?.kind;
        self.views.as_ref()?.borrow().get(kind).cloned()
    }

    /// Places or updates the instances of the application among `configured`:
    /// a new one offers its settings to the application (kept if it had none),
    /// one whose settings changed saves them there.
    fn sync_shared(&mut self, configured: &[ModuleInstance]) {
        let mut previous = std::mem::take(&mut self.shared);
        for config in configured {
            if !self.lives_in_application(&config.kind) {
                continue;
            }
            let Some(host) = &self.application else {
                continue;
            };
            match previous
                .remove(&config.id)
                .filter(|p| p.kind == config.kind)
            {
                None => host.adopt(&config.kind, &config.settings),
                Some(before) if before.settings != config.settings => {
                    host.update_settings(&config.kind, &config.settings);
                }
                Some(_) => {}
            }
            self.shared.insert(
                config.id.clone(),
                Shared {
                    kind: config.kind.clone(),
                    settings: config.settings.clone(),
                },
            );
        }
    }

    /// Instances that belong to no device (the application's): nothing of a
    /// device reaches them, so they have no notifications to read, no images
    /// and nothing to redraw. A module that lives in the application must
    /// not rely on those (see [`hubergamof_module_api::ModuleLifetime`]).
    pub(crate) fn detached(
        registry: Arc<ModuleRegistry>,
        events: mpsc::Sender<RoutedEvent>,
        screen_lock: ScreenLockState,
    ) -> Self {
        Self::new(
            registry,
            RedrawHandle::noop(),
            events,
            UnreadNotifications::none(),
            ImageLibrary::empty(),
            screen_lock,
        )
    }

    /// Makes the instances match `configured`: an instance keeping its id
    /// and kind keeps its state and receives the settings if they changed;
    /// others are created or destroyed. A placeholder is retried when its
    /// kind became available.
    pub(crate) fn sync(&mut self, configured: &[ModuleInstance]) {
        self.sync_shared(configured);
        let mut previous = std::mem::take(&mut self.live);
        for config in configured {
            if self.shared.contains_key(&config.id) {
                continue;
            }
            let kept = previous
                .remove(&config.id)
                .filter(|instance| instance.kind == config.kind)
                .filter(|instance| instance.available || self.registry.get(&config.kind).is_none());
            let instance = match kept {
                Some(instance) => self.update(&config.id, instance, &config.settings),
                None => self.create(config),
            };
            self.live.insert(config.id.clone(), instance);
        }
        // Instances left in `previous` are dropped here, aborting their tasks.
    }

    /// Destroys every instance.
    pub(crate) fn clear(&mut self) {
        self.live.clear();
        self.shared.clear();
    }

    /// Tells every instance that the manager stops clean (see
    /// [`Module::on_shutdown`]), then destroys them. A panic of one does not
    /// keep the others from being told.
    pub(crate) fn shutdown(&mut self) {
        for (id, instance) in &mut self.live {
            let module = instance.module.as_mut();
            if catch_unwind(AssertUnwindSafe(|| module.on_shutdown())).is_err() {
                tracing::error!(module = %id, kind = %instance.kind, "module panicked on shutdown");
            }
        }
        self.clear();
    }

    /// Key of the home page of instance `id`.
    pub(crate) fn key(&mut self, id: &ModuleInstanceId) -> Option<KeyContent> {
        if self.shared.contains_key(id) {
            return self.view(id).map(|view| view.key.clone());
        }
        self.inspect(id, "key", |module| Some(module.key()))
    }

    /// Icon of the module kind of instance `id`, if it has one.
    pub(crate) fn icon(&self, id: &ModuleInstanceId) -> Option<hubergamof_module_api::ImageData> {
        self.registry.get(self.kind_of(id)?)?.icon()
    }

    /// Whether instance `id` keeps working while the computer is locked: its
    /// module declares it. An unknown instance does not.
    pub(crate) fn usable_when_locked(&self, id: &ModuleInstanceId) -> bool {
        self.kind_of(id)
            .and_then(|kind| self.registry.get(kind))
            .is_some_and(|factory| factory.descriptor().capabilities.usable_when_locked)
    }

    /// What the haptic event `event` of instance `id` is, when its module
    /// declares such an event.
    pub(crate) fn haptic_play(
        &self,
        id: &ModuleInstanceId,
        event: &str,
    ) -> Option<crate::haptics::HapticPlay> {
        let kind = self.kind_of(id)?;
        let declared = self
            .registry
            .get(kind)?
            .haptic_events()
            .into_iter()
            .find(|declared| declared.id == event)?;
        Some(crate::haptics::HapticPlay {
            module: kind.clone(),
            event: declared.id,
            waveform: declared.waveform,
        })
    }

    /// Whether the pages of instance `id` may declare what stays usable
    /// while the computer is locked: its module declares it. An unknown
    /// instance does not.
    pub(crate) fn lock_safe_pages(&self, id: &ModuleInstanceId) -> bool {
        self.kind_of(id)
            .and_then(|kind| self.registry.get(kind))
            .is_some_and(|factory| factory.descriptor().capabilities.lock_safe_pages)
    }

    /// Page `page` of instance `id`, if it exists.
    pub(crate) fn page(&mut self, id: &ModuleInstanceId, page: &PageId) -> Option<PageContent> {
        // A module of the application offers no pages (see `ModuleLifetime`).
        if self.shared.contains_key(id) {
            return None;
        }
        self.inspect(id, "page", |module| module.page(page))
    }

    /// Reminder of what the dials assigned `function` of instance `id` do.
    pub(crate) fn dial_hint(
        &mut self,
        id: &ModuleInstanceId,
        function: &DialFunctionId,
    ) -> Option<DialHint> {
        if self.shared.contains_key(id) {
            let view = self.view(id)?;
            return Some(view.dial_hints.get(function).cloned().unwrap_or_default());
        }
        self.inspect(id, "dial_hint", |module| Some(module.dial_hint(function)))
    }

    /// What instance `id` shows on the touch strip, if anything.
    pub(crate) fn strip_widget(&mut self, id: &ModuleInstanceId) -> Option<StripWidget> {
        if self.shared.contains_key(id) {
            return self.view(id)?.strip_widget.clone();
        }
        self.inspect(id, "strip_widget", |module| module.strip_widget())
    }

    /// What the petal of instance `id` says in the radial menu, if anything.
    pub(crate) fn radial_status(&mut self, id: &ModuleInstanceId) -> Option<String> {
        if self.shared.contains_key(id) {
            return self.view(id)?.radial_status.clone();
        }
        self.inspect(id, "radial_status", |module| module.radial_status())
    }

    /// The menu instance `id` has open, if any.
    pub(crate) fn strip_popup(&mut self, id: &ModuleInstanceId) -> Option<StripPopup> {
        if self.shared.contains_key(id) {
            return self.view(id)?.strip_popup.clone();
        }
        self.inspect(id, "strip_popup", |module| module.strip_popup())
    }

    /// Instances replaced since the last call because they panicked outside
    /// of a handler (getter, settings, creation).
    pub(crate) fn take_failures(&mut self) -> Vec<ModuleInstanceId> {
        std::mem::take(&mut self.failures)
    }

    /// Instances whose background tasks panicked since the last call, logged.
    pub(crate) fn take_task_panics(&self) -> Vec<ModuleInstanceId> {
        self.live
            .iter()
            .filter_map(|(id, instance)| {
                let panics = instance.tasks.reap_panics();
                (panics > 0).then(|| {
                    tracing::error!(module = %id, panics, "background task of module panicked");
                    id.clone()
                })
            })
            .collect()
    }

    /// Runs a handler of instance `id`; `None` when there is no such
    /// instance.
    pub(crate) async fn call(&mut self, id: &ModuleInstanceId, call: Call) -> Option<Outcome> {
        if let Some(shared) = self.shared.get(id) {
            let (host, kind) = (self.application.clone()?, shared.kind.clone());
            return call_application(host, kind, call).await;
        }
        let instance = self.live.get_mut(id)?;
        let result = run_handler(instance.module.as_mut(), call).await;
        Some(match result {
            HandlerResult::Returned(commands) => Outcome::Done(commands),
            HandlerResult::Panicked => {
                self.disable(id, "handler");
                Outcome::Failed
            }
            HandlerResult::TimedOut => {
                tracing::warn!(
                    module = %id,
                    budget_ms = HANDLER_BUDGET.as_millis(),
                    "module handler too slow, abandoned"
                );
                Outcome::Failed
            }
        })
    }

    /// Hands `routed` to [`Module::on_event`] of its instance, unless that
    /// instance was removed or replaced since it sent it.
    pub(crate) async fn dispatch(&mut self, routed: RoutedEvent) -> Option<Outcome> {
        if routed.sender.strong_count() == 0 {
            tracing::debug!(module = %routed.module, "dropping event of a replaced instance");
            return None;
        }
        self.call(&routed.module, Call::Event(routed.event)).await
    }

    fn inspect<T>(
        &mut self,
        id: &ModuleInstanceId,
        getter: &str,
        read: impl FnOnce(&dyn Module) -> Option<T>,
    ) -> Option<T> {
        let module = self.live.get(id)?.module.as_ref();
        let started = Instant::now();
        let result = catch_unwind(AssertUnwindSafe(|| read(module)));
        let elapsed = started.elapsed();
        if elapsed > SLOW_GETTER_THRESHOLD {
            tracing::debug!(module = %id, getter, elapsed_us = elapsed.as_micros(), "slow module getter");
        }
        if let Ok(value) = result {
            return value;
        }
        self.disable(id, getter);
        self.failures.push(id.clone());
        None
    }

    /// Replaces instance `id` by a placeholder after a panic in `call`; the
    /// configuration being applied again retries the module.
    fn disable(&mut self, id: &ModuleInstanceId, call: &str) {
        if let Some(instance) = self.live.get_mut(id) {
            tracing::error!(module = %id, kind = %instance.kind, call, "module panicked, replaced by a placeholder");
            instance.replace_with_placeholder();
        }
    }

    fn update(
        &mut self,
        id: &ModuleInstanceId,
        mut instance: Instance,
        settings: &Value,
    ) -> Instance {
        if instance.settings == *settings {
            return instance;
        }
        let module = instance.module.as_mut();
        match catch_unwind(AssertUnwindSafe(|| module.update_settings(settings))) {
            Ok(Ok(())) => instance.settings = settings.clone(),
            Ok(Err(error)) => {
                tracing::warn!(module = %id, %error, "settings rejected, keeping the previous ones");
            }
            Err(_) => {
                tracing::error!(module = %id, kind = %instance.kind, "module panicked applying settings");
                instance.replace_with_placeholder();
                self.failures.push(id.clone());
            }
        }
        instance
    }

    fn create(&mut self, config: &ModuleInstance) -> Instance {
        let alive = Liveness::default();
        let redraw = self.redraw.clone();
        // Wake the engine at once so that the failure shows.
        let tasks = TaskScope::with_panic_hook(move || redraw.request_redraw());
        match self.instantiate(config, &alive, &tasks) {
            Some(module) => Instance {
                kind: config.kind.clone(),
                settings: config.settings.clone(),
                available: true,
                _alive: alive,
                module,
                tasks,
            },
            // Dropping `tasks` aborts what the failed factory spawned.
            None => Instance::placeholder(config.kind.clone(), config.settings.clone()),
        }
    }

    /// The module of `config` from its factory; `None` when the kind is
    /// unknown or the factory fails.
    fn instantiate(
        &mut self,
        config: &ModuleInstance,
        alive: &Liveness,
        tasks: &TaskScope,
    ) -> Option<Box<dyn Module>> {
        let Some(factory) = self.registry.get(&config.kind).cloned() else {
            tracing::warn!(module = %config.id, kind = %config.kind, "unknown module kind");
            return None;
        };
        let ctx = ModuleContext::new(
            config.id.clone(),
            self.redraw.clone(),
            self.event_sender(&config.id, alive),
            tasks.spawner(),
        )
        .with_unread(self.unread.clone())
        .with_images(self.images.clone())
        .with_screen_lock(self.screen_lock.clone());
        match catch_unwind(AssertUnwindSafe(|| factory.create(ctx, &config.settings))) {
            Ok(Ok(module)) => Some(module),
            Ok(Err(error)) => {
                tracing::warn!(module = %config.id, kind = %config.kind, %error, "cannot create module");
                None
            }
            Err(_) => {
                tracing::error!(module = %config.id, kind = %config.kind, "module panicked on creation");
                self.failures.push(config.id.clone());
                None
            }
        }
    }

    fn event_sender(&self, id: &ModuleInstanceId, alive: &Liveness) -> EventSender {
        let events = self.events.clone();
        let queue_full = Arc::clone(&self.queue_full);
        let module = id.clone();
        let sender = Arc::downgrade(alive);
        EventSender::new(move |event| {
            if sender.strong_count() == 0 {
                return false;
            }
            let routed = RoutedEvent {
                module: module.clone(),
                sender: sender.clone(),
                event,
            };
            match events.try_send(routed) {
                Ok(()) => {
                    queue_full.store(false, Ordering::Relaxed);
                    true
                }
                Err(TrySendError::Full(_)) => {
                    if !queue_full.swap(true, Ordering::Relaxed) {
                        tracing::warn!(%module, "module event queue full, dropping events");
                    }
                    false
                }
                Err(TrySendError::Closed(_)) => false,
            }
        })
    }
}

/// An input for a module of the application, taken by the application within
/// [`APPLICATION_CALL_TIMEOUT`].
async fn call_application(host: ModuleHost, kind: ModuleKind, call: Call) -> Option<Outcome> {
    if let Ok(outcome) =
        tokio::time::timeout(APPLICATION_CALL_TIMEOUT, host.call(&kind, call)).await
    {
        return outcome;
    }
    tracing::warn!(%kind, "the application did not take the input in time");
    Some(Outcome::Failed)
}

enum HandlerResult {
    Returned(Vec<Command>),
    Panicked,
    TimedOut,
}

type HandlerFuture<'a> = Pin<Box<dyn Future<Output = Vec<Command>> + Send + 'a>>;

/// Runs `call` on `module` within [`HANDLER_BUDGET`], catching panics
/// (also those raised while creating the future).
async fn run_handler(module: &mut dyn Module, call: Call) -> HandlerResult {
    let Ok(future) = catch_unwind(AssertUnwindSafe(|| invoke(module, call))) else {
        return HandlerResult::Panicked;
    };
    match tokio::time::timeout(HANDLER_BUDGET, AssertUnwindSafe(future).catch_unwind()).await {
        Ok(Ok(commands)) => HandlerResult::Returned(commands),
        // The panic hook already printed the message.
        Ok(Err(_)) => HandlerResult::Panicked,
        Err(_) => HandlerResult::TimedOut,
    }
}

fn invoke(module: &mut dyn Module, call: Call) -> HandlerFuture<'_> {
    match call {
        Call::KeyPress => module.on_key_press(),
        Call::ItemPress(page, item) => {
            Box::pin(async move { module.on_item_press(&page, &item).await })
        }
        Call::DialRotate(function, ticks) => {
            Box::pin(async move { module.on_dial_rotate(&function, ticks).await })
        }
        Call::DialPress(function) => Box::pin(async move { module.on_dial_press(&function).await }),
        Call::ContextRotate(page, ticks) => {
            Box::pin(async move { module.on_context_rotate(&page, ticks).await })
        }
        Call::ContextPress(page) => Box::pin(async move { module.on_context_press(&page).await }),
        Call::ContextLongPress(page) => {
            Box::pin(async move { module.on_context_long_press(&page).await })
        }
        Call::NotificationPress(id) => {
            Box::pin(async move { module.on_notification_press(id).await })
        }
        Call::RadialAction(action) => {
            Box::pin(async move { module.on_radial_action(&action).await })
        }
        Call::Event(event) => module.on_event(event),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    use hubergamof_module_api::{ModuleCapabilities, ModuleDescriptor, ModuleFactory};
    use serde_json::json;

    use super::*;

    const KIND: &str = "counter";

    /// Counts the events its instances handle; keeps their senders.
    #[derive(Clone, Default)]
    struct Probe {
        handled: Arc<AtomicUsize>,
        shutdowns: Arc<AtomicUsize>,
        senders: Arc<Mutex<Vec<EventSender>>>,
    }

    struct Counter(Probe);

    #[async_trait]
    impl Module for Counter {
        fn on_shutdown(&mut self) {
            self.0.shutdowns.fetch_add(1, Ordering::SeqCst);
        }

        async fn on_event(&mut self, _event: ModuleEvent) -> Vec<Command> {
            self.0.handled.fetch_add(1, Ordering::SeqCst);
            Vec::new()
        }

        fn update_settings(&mut self, _settings: &Value) -> Result<(), ModuleError> {
            Ok(())
        }
    }

    struct CounterFactory(Probe);

    impl ModuleFactory for CounterFactory {
        fn descriptor(&self) -> ModuleDescriptor {
            ModuleDescriptor {
                kind: KIND.into(),
                name: String::new(),
                description: String::new(),
                capabilities: ModuleCapabilities {
                    key: true,
                    ..Default::default()
                },
                dial_functions: Vec::new(),
                settings: Vec::new(),
            }
        }

        fn create(
            &self,
            ctx: ModuleContext,
            _settings: &Value,
        ) -> Result<Box<dyn Module>, ModuleError> {
            self.0.senders.lock().unwrap().push(ctx.events().clone());
            Ok(Box::new(Counter(self.0.clone())))
        }
    }

    fn counter() -> ModuleInstance {
        ModuleInstance {
            id: "c".into(),
            kind: KIND.into(),
            label: None,
            image: None,
            settings: json!({}),
        }
    }

    /// A module whose shutdown panics.
    struct Fragile;

    #[async_trait]
    impl Module for Fragile {
        fn on_shutdown(&mut self) {
            panic!("fragile on shutdown");
        }

        fn update_settings(&mut self, _settings: &Value) -> Result<(), ModuleError> {
            Ok(())
        }
    }

    struct FragileFactory;

    impl ModuleFactory for FragileFactory {
        fn descriptor(&self) -> ModuleDescriptor {
            ModuleDescriptor {
                kind: FRAGILE_KIND.into(),
                name: String::new(),
                description: String::new(),
                capabilities: ModuleCapabilities::default(),
                dial_functions: Vec::new(),
                settings: Vec::new(),
            }
        }

        fn create(
            &self,
            _ctx: ModuleContext,
            _settings: &Value,
        ) -> Result<Box<dyn Module>, ModuleError> {
            Ok(Box::new(Fragile))
        }
    }

    const FRAGILE_KIND: &str = "fragile";

    fn instances(probe: &Probe) -> (Instances, mpsc::Receiver<RoutedEvent>) {
        let mut registry = ModuleRegistry::new();
        registry.register(Arc::new(CounterFactory(probe.clone())));
        registry.register(Arc::new(FragileFactory));
        let (events, received) = mpsc::channel(8);
        (
            Instances::detached(Arc::new(registry), events, ScreenLockState::never()),
            received,
        )
    }

    #[tokio::test]
    async fn a_shutdown_tells_every_instance_even_after_one_panicked() {
        let probe = Probe::default();
        let (mut instances, _received) = instances(&probe);
        let fragile = ModuleInstance {
            id: "a".into(),
            kind: FRAGILE_KIND.into(),
            ..counter()
        };
        // `a` is told first (ids are ordered) and panics.
        instances.sync(&[fragile, counter()]);
        instances.shutdown();
        assert_eq!(probe.shutdowns.load(Ordering::SeqCst), 1);
        assert!(instances.key(&"c".into()).is_none(), "everything is gone");
    }

    #[tokio::test]
    async fn events_queued_before_a_replacement_are_dropped() {
        let probe = Probe::default();
        let (mut instances, mut received) = instances(&probe);
        instances.sync(&[counter()]);
        let first = probe.senders.lock().unwrap()[0].clone();
        assert!(first.send(()));

        // Removed then created again while the event waits in the queue.
        instances.sync(&[]);
        instances.sync(&[counter()]);
        let stale = received.recv().await.unwrap();
        assert!(instances.dispatch(stale).await.is_none());
        assert_eq!(probe.handled.load(Ordering::SeqCst), 0);

        let current = probe.senders.lock().unwrap()[1].clone();
        assert!(current.send(()));
        let fresh = received.recv().await.unwrap();
        assert!(matches!(
            instances.dispatch(fresh).await,
            Some(Outcome::Done(_))
        ));
        assert_eq!(probe.handled.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn senders_of_removed_instances_refuse_events() {
        let probe = Probe::default();
        let (mut instances, _received) = instances(&probe);
        instances.sync(&[counter()]);
        let sender = probe.senders.lock().unwrap()[0].clone();
        assert!(sender.send(()));
        instances.sync(&[]);
        assert!(!sender.send(()));
    }
}
