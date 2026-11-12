//! Test modules, a counting backend and the engine fixture.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use hubergamof_module_api::{
    ChoiceOption, Command, DialFunctionDescriptor, DialFunctionId, DialHint, EventSender, Feedback,
    ItemId, KeyContent, KeyState, Module, ModuleCapabilities, ModuleContext, ModuleDescriptor,
    ModuleError, ModuleEvent, ModuleFactory, PageContent, PageId, PageItem, RedrawHandle,
    SettingField, SettingKind, StripWidget, UnreadNotifications, VolumeWidget, parse_settings,
};
use hubergamof_modules::ModuleRegistry;
use hubergamof_protocol::{
    DeviceCapabilities, DeviceConfig, DeviceInfo, DeviceModel, DialAssignment, ImageSize,
    ModuleInstance,
};
use image::RgbaImage;
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::config_store::ConfigStore;
use crate::device::virtual_deck::VirtualDeck;
use crate::device::{DeckBackend, DeviceError, InputEvent};
use crate::engine::*;
use crate::strip::{HintFace, SimulatedProbe, StripFace, SystemProbe, SystemSample, WallClock};
use crate::testing::builtin_registry;

pub(super) const LIST_KIND: &str = "list";
pub(super) const LIST_PAGE: &str = "list";
pub(super) const FRAGILE_KIND: &str = "fragile";
pub(super) const FRAGILE_PAGE: &str = "p";
/// Dial function of the fragile module: turning counts, pressing succeeds.
pub(super) const FRAGILE_FUNCTION: &str = "turn";
/// Stands for the navigation function of the manager in the dials of
/// [`Fixture::with`].
pub(super) const NAVIGATION_DIAL: &str = "nav";
pub(super) const WAIT: Duration = Duration::from_secs(5);

/// Upper bound of the `items` setting of the list module.
const MAX_LIST_ITEMS: i64 = 64;

/// How long the slow fragile module takes to handle a press.
pub(super) const SLOW_HANDLER: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------- List module

/// Shared with the test: what the list instances were given and did, and
/// knobs changing what they show without any redraw.
#[derive(Clone, Default)]
pub(super) struct Probe {
    pub(super) redraw: Arc<Mutex<Option<RedrawHandle>>>,
    pub(super) events: Arc<Mutex<Option<EventSender>>>,
    /// Unread notifications, as given to the last list instance.
    pub(super) unread: Arc<Mutex<Option<UnreadNotifications>>>,
    /// Overrides the key label of every instance.
    pub(super) key_label: Arc<Mutex<Option<String>>>,
    /// Number of system measures taken by the engines.
    pub(super) system_samples: Arc<AtomicUsize>,
    /// Local time of the engines.
    pub(super) clock: SettableClock,
    /// Volume shown on the strip by fragile instances.
    pub(super) volume: Arc<Mutex<Option<VolumeWidget>>>,
    /// Page the list instances no longer have (removed by an event).
    pub(super) gone_page: Arc<Mutex<Option<String>>>,
    /// Added to the index in the ids of page items.
    pub(super) item_shift: Arc<AtomicUsize>,
    /// Items pressed, in order.
    pub(super) pressed: Arc<Mutex<Vec<ItemId>>>,
    /// Number of calls to `update_settings`.
    pub(super) settings_updates: Arc<AtomicUsize>,
    /// Number of events handled by list instances.
    pub(super) list_events: Arc<AtomicUsize>,
    /// Event sender of the last fragile instance.
    pub(super) fragile_events: Arc<Mutex<Option<EventSender>>>,
    /// Number of events handled by fragile instances.
    pub(super) fragile_events_handled: Arc<AtomicUsize>,
    /// Number of tasks spawned by an `abandon` fragile factory still alive.
    pub(super) abandoned_tasks: Arc<AtomicUsize>,
}

/// Wall clock a test sets.
#[derive(Debug, Clone)]
pub(super) struct SettableClock(Arc<Mutex<jiff::civil::DateTime>>);

impl Default for SettableClock {
    /// A Wednesday at 14:05:30, like [`crate::strip::FixedClock`].
    fn default() -> Self {
        Self(Arc::new(Mutex::new(
            jiff::civil::date(2026, 10, 28).at(14, 5, 30, 0),
        )))
    }
}

impl SettableClock {
    pub(super) fn set(&self, now: jiff::civil::DateTime) {
        *self.0.lock().unwrap() = now;
    }
}

impl WallClock for SettableClock {
    fn now(&self) -> jiff::civil::DateTime {
        *self.0.lock().unwrap()
    }
}

/// System probe counting its samples, cycling through lively figures.
#[derive(Debug)]
pub(super) struct CountingProbe {
    samples: Arc<AtomicUsize>,
    inner: SimulatedProbe,
}

impl SystemProbe for CountingProbe {
    fn sample(&self) -> Option<SystemSample> {
        self.samples.fetch_add(1, Ordering::SeqCst);
        self.inner.sample()
    }
}

/// Sent through the event sender: adds ten presses.
pub(super) struct Bump;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ListSettings {
    label: String,
    items: usize,
}

impl Default for ListSettings {
    fn default() -> Self {
        Self {
            label: "list".into(),
            items: 0,
        }
    }
}

/// Key counting its presses (active when odd) and opening its page; page
/// items `i<n>`: `i0` goes back, `i1` home, `i2` scrolls one row, `i3` opens
/// a nested page, `i4` reports success.
struct List {
    settings: ListSettings,
    presses: u32,
    probe: Probe,
}

#[async_trait]
impl Module for List {
    fn key(&self) -> KeyContent {
        let label = self.probe.key_label.lock().unwrap().clone();
        let state = if self.presses % 2 == 1 {
            KeyState::Active
        } else {
            KeyState::Normal
        };
        KeyContent::with_label(
            label.unwrap_or_else(|| format!("{} {}", self.settings.label, self.presses)),
        )
        .in_state(state)
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        self.presses += 1;
        vec![Command::OpenPage(LIST_PAGE.into())]
    }

    fn page(&self, page: &PageId) -> Option<PageContent> {
        let shift = self.probe.item_shift.load(Ordering::SeqCst);
        let gone = self.probe.gone_page.lock().unwrap().as_deref() == Some(page.0.as_str());
        let exists = (page.0 == LIST_PAGE || page.0 == "nested") && !gone;
        exists.then(|| PageContent {
            title: format!("{} ({page})", self.settings.label),
            items: (0..self.settings.items)
                .map(|index| PageItem {
                    id: ItemId(format!("i{}", index + shift)),
                    key: KeyContent::with_label(format!("item {index}")),
                })
                .collect(),
        })
    }

    async fn on_item_press(&mut self, _page: &PageId, item: &ItemId) -> Vec<Command> {
        self.probe.pressed.lock().unwrap().push(item.clone());
        match item.0.as_str() {
            "i0" => vec![Command::Back],
            "i1" => vec![Command::GoHome],
            "i2" => vec![Command::ScrollRows(1)],
            "i3" => vec![Command::OpenPage("nested".into())],
            "i4" => vec![Command::Feedback(Feedback::Ok)],
            _ => Vec::new(),
        }
    }

    async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
        self.probe.list_events.fetch_add(1, Ordering::SeqCst);
        if event.is::<Bump>() {
            self.presses += 10;
        }
        vec![Command::Feedback(Feedback::Ok)]
    }

    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError> {
        self.probe.settings_updates.fetch_add(1, Ordering::SeqCst);
        self.settings = parse_settings(settings)?;
        Ok(())
    }
}

struct ListFactory(Probe);

impl ModuleFactory for ListFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor {
            kind: LIST_KIND.into(),
            name: "List".into(),
            description: String::new(),
            capabilities: ModuleCapabilities { key: true },
            dial_functions: Vec::new(),
            settings: vec![
                SettingField {
                    key: "label".into(),
                    label: "Label".into(),
                    help: None,
                    kind: SettingKind::Text {
                        default: "list".into(),
                    },
                },
                SettingField {
                    key: "items".into(),
                    label: "Items".into(),
                    help: None,
                    kind: SettingKind::Integer {
                        default: 0,
                        min: Some(0),
                        max: Some(MAX_LIST_ITEMS),
                        unit: None,
                    },
                },
            ],
        }
    }

    fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
        *self.0.redraw.lock().unwrap() = Some(ctx.redraw().clone());
        *self.0.events.lock().unwrap() = Some(ctx.events().clone());
        *self.0.unread.lock().unwrap() = Some(ctx.unread_notifications().clone());
        Ok(Box::new(List {
            settings: parse_settings(settings)?,
            presses: 0,
            probe: self.0.clone(),
        }))
    }
}

// ---------------------------------------------------------------- Fragile module

/// What goes wrong with a fragile instance (setting `failure`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Failure {
    /// Nothing.
    None,
    /// `dial_hint` panics.
    Hint,
    /// `key` panics.
    Key,
    /// `page` panics.
    Page,
    /// Every handler panics.
    Press,
    /// Every handler takes [`SLOW_HANDLER`].
    Slow,
    /// `update_settings` panics when given this failure.
    Settings,
    /// The factory panics.
    Create,
    /// The factory spawns a task, then fails.
    Abandon,
    /// A background task panics shortly after creation.
    Task,
}

/// Every failure, as written in the settings.
pub(super) const FAILURES: [&str; 10] = [
    "none", "hint", "key", "page", "press", "slow", "settings", "create", "abandon", "task",
];

/// Delay before the background task of [`Failure::Task`] panics.
const TASK_PANIC_DELAY: Duration = Duration::from_millis(50);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FragileSettings {
    failure: Failure,
    #[serde(default)]
    tag: Option<String>,
    /// Makes `update_settings` refuse the settings.
    #[serde(default)]
    reject: bool,
}

/// Module misbehaving as its settings say. Its key shows its tag (default
/// `fragile`) and opens page `p` (one item `x`); the reminder of its dial
/// shows the dial turns, and it shows the volume of the probe on the strip;
/// its dial, item and event handlers report success
/// (an event carrying a `Vec<Command>` returns those commands instead).
struct Fragile {
    failure: Failure,
    tag: Option<String>,
    turns: i32,
    probe: Probe,
}

impl Fragile {
    async fn misbehave(&self) {
        match self.failure {
            Failure::Press => panic!("fragile handler"),
            Failure::Slow => tokio::time::sleep(SLOW_HANDLER).await,
            _ => {}
        }
    }
}

#[async_trait]
impl Module for Fragile {
    fn key(&self) -> KeyContent {
        assert_ne!(self.failure, Failure::Key, "fragile key");
        KeyContent::with_label(self.tag.as_deref().unwrap_or(FRAGILE_KIND))
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        self.misbehave().await;
        vec![Command::OpenPage(FRAGILE_PAGE.into())]
    }

    fn page(&self, page: &PageId) -> Option<PageContent> {
        assert_ne!(self.failure, Failure::Page, "fragile page");
        (page.0 == FRAGILE_PAGE).then(|| PageContent {
            title: FRAGILE_KIND.into(),
            items: vec![PageItem {
                id: "x".into(),
                key: KeyContent::with_label("x"),
            }],
        })
    }

    async fn on_item_press(&mut self, _page: &PageId, _item: &ItemId) -> Vec<Command> {
        self.misbehave().await;
        vec![Command::Feedback(Feedback::Ok)]
    }

    async fn on_dial_rotate(&mut self, _function: &DialFunctionId, ticks: i32) -> Vec<Command> {
        self.misbehave().await;
        self.turns += ticks;
        vec![Command::Feedback(Feedback::Ok)]
    }

    async fn on_dial_press(&mut self, _function: &DialFunctionId) -> Vec<Command> {
        self.misbehave().await;
        vec![Command::Feedback(Feedback::Ok)]
    }

    async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
        self.misbehave().await;
        self.probe
            .fragile_events_handled
            .fetch_add(1, Ordering::SeqCst);
        // An event carrying commands asks for exactly those.
        match event.downcast::<Vec<Command>>() {
            Ok(commands) => *commands,
            Err(_) => vec![Command::Feedback(Feedback::Ok)],
        }
    }

    fn dial_hint(&self, _function: &DialFunctionId) -> DialHint {
        assert_ne!(self.failure, Failure::Hint, "fragile hint");
        DialHint::none().rotating(format!("{FRAGILE_KIND} {}", self.turns))
    }

    fn strip_widget(&self) -> Option<StripWidget> {
        let volume = *self.probe.volume.lock().unwrap();
        volume.map(StripWidget::Volume)
    }

    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError> {
        let settings: FragileSettings = parse_settings(settings)?;
        assert_ne!(settings.failure, Failure::Settings, "fragile settings");
        if settings.reject {
            return Err(ModuleError::InvalidSettings("rejected".into()));
        }
        self.failure = settings.failure;
        self.tag = settings.tag;
        Ok(())
    }
}

/// Counts itself among the live tasks while it exists.
struct LiveTask(Arc<AtomicUsize>);

impl LiveTask {
    fn new(count: &Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(count))
    }
}

impl Drop for LiveTask {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct FragileFactory(Probe);

impl ModuleFactory for FragileFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor {
            kind: FRAGILE_KIND.into(),
            name: "Fragile".into(),
            description: String::new(),
            capabilities: ModuleCapabilities { key: true },
            dial_functions: vec![DialFunctionDescriptor {
                id: FRAGILE_FUNCTION.into(),
                name: "Turn".into(),
                description: String::new(),
            }],
            settings: vec![SettingField {
                key: "failure".into(),
                label: "Failure".into(),
                help: None,
                kind: SettingKind::Choice {
                    default: "none".into(),
                    options: FAILURES
                        .map(|value| ChoiceOption {
                            value: value.into(),
                            label: value.into(),
                        })
                        .to_vec(),
                },
            }],
        }
    }

    fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
        let FragileSettings { failure, tag, .. } = parse_settings(settings)?;
        *self.0.fragile_events.lock().unwrap() = Some(ctx.events().clone());
        match failure {
            Failure::Create => panic!("fragile factory"),
            Failure::Abandon => {
                let alive = LiveTask::new(&self.0.abandoned_tasks);
                ctx.tasks().spawn(async move {
                    let _alive = alive;
                    std::future::pending::<()>().await;
                });
                return Err(ModuleError::InvalidSettings("abandoned".into()));
            }
            Failure::Task => {
                ctx.tasks().spawn(async {
                    tokio::time::sleep(TASK_PANIC_DELAY).await;
                    panic!("fragile task");
                });
            }
            _ => {}
        }
        Ok(Box::new(Fragile {
            failure,
            tag,
            turns: 0,
            probe: self.0.clone(),
        }))
    }
}

// ---------------------------------------------------------------- Counting backend

/// Virtual deck counting the images written, whose brightness can be made
/// to fail.
pub(super) struct CountingDeck {
    pub(super) inner: Arc<VirtualDeck>,
    key_writes: AtomicUsize,
    strip_writes: AtomicUsize,
    flushes: AtomicUsize,
    pub(super) fail_brightness: AtomicBool,
}

#[async_trait]
impl DeckBackend for CountingDeck {
    fn info(&self) -> &DeviceInfo {
        self.inner.info()
    }

    fn subscribe(&self) -> broadcast::Receiver<InputEvent> {
        self.inner.subscribe()
    }

    fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }

    async fn set_key_image(&self, index: u8, image: Arc<RgbaImage>) -> Result<(), DeviceError> {
        self.key_writes.fetch_add(1, Ordering::SeqCst);
        self.inner.set_key_image(index, image).await
    }

    async fn set_strip_region(&self, x: u32, image: Arc<RgbaImage>) -> Result<(), DeviceError> {
        self.strip_writes.fetch_add(1, Ordering::SeqCst);
        self.inner.set_strip_region(x, image).await
    }

    async fn set_brightness(&self, percent: u8) -> Result<(), DeviceError> {
        if self.fail_brightness.load(Ordering::SeqCst) {
            return Err(DeviceError::Disconnected);
        }
        self.inner.set_brightness(percent).await
    }

    async fn clear_all(&self) -> Result<(), DeviceError> {
        self.inner.clear_all().await
    }

    async fn flush(&self) -> Result<(), DeviceError> {
        self.flushes.fetch_add(1, Ordering::SeqCst);
        self.inner.flush().await
    }

    async fn close(&self) {
        self.inner.close().await;
    }
}

impl CountingDeck {
    /// Key and strip writes since the last call.
    pub(super) fn writes(&self) -> (usize, usize) {
        (
            self.key_writes.swap(0, Ordering::SeqCst),
            self.strip_writes.swap(0, Ordering::SeqCst),
        )
    }

    /// Flushes (one per redraw writing anything) since the last call.
    pub(super) fn flushes(&self) -> usize {
        self.flushes.swap(0, Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------- Fixture

pub(super) struct Fixture {
    pub(super) _dir: TempDir,
    pub(super) store: ConfigStore,
    pub(super) deck: Arc<CountingDeck>,
    pub(super) engine: EngineHandle,
    pub(super) task: JoinHandle<()>,
    pub(super) probe: Probe,
}

pub(super) fn registry(probe: &Probe) -> Arc<ModuleRegistry> {
    let mut registry = builtin_registry();
    registry.register(Arc::new(ListFactory(probe.clone())));
    registry.register(Arc::new(FragileFactory(probe.clone())));
    Arc::new(registry)
}

pub(super) fn services(dir: &TempDir, probe: &Probe) -> EngineServices {
    let registry = registry(probe);
    EngineServices {
        store: ConfigStore::new(dir.path(), Arc::clone(&registry)),
        renderer: Arc::new(crate::render::Renderer::new().unwrap()),
        registry,
        wall_clock: Arc::new(probe.clock.clone()),
        system: Arc::new(CountingProbe {
            samples: Arc::clone(&probe.system_samples),
            inner: SimulatedProbe::demo(),
        }),
    }
}

pub(super) async fn start(info: DeviceInfo) -> Fixture {
    start_with(info, false).await
}

/// Starts an engine and waits for its strip to be written again once the device settled, so that
/// tests counting writes do not meet that late rewrite on a slow machine.
pub(super) async fn start_settled(info: DeviceInfo) -> Fixture {
    let fixture = start(info).await;
    tokio::time::timeout(WAIT, async {
        while fixture.deck.strip_writes.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the strip is written again once the device settled");
    let _ = fixture.deck.writes();
    let _ = fixture.deck.flushes();
    fixture
}

/// Starts an engine on a deck whose brightness fails if `fail_brightness`.
pub(super) async fn start_with(info: DeviceInfo, fail_brightness: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let probe = Probe::default();
    let services = services(&dir, &probe);
    let store = services.store.clone();
    let deck = Arc::new(CountingDeck {
        inner: Arc::new(VirtualDeck::new(info)),
        key_writes: AtomicUsize::new(0),
        strip_writes: AtomicUsize::new(0),
        flushes: AtomicUsize::new(0),
        fail_brightness: AtomicBool::new(fail_brightness),
    });
    let (engine, handle) = DeviceEngine::start(Arc::clone(&deck) as _, services)
        .await
        .unwrap();
    Fixture {
        _dir: dir,
        store,
        deck,
        engine: handle,
        task: tokio::spawn(engine.run()),
        probe,
    }
}

pub(super) fn no_dial_info() -> DeviceInfo {
    DeviceInfo {
        serial: "NODIAL".into(),
        model: DeviceModel::Mk2,
        display_name: "No dial".into(),
        capabilities: DeviceCapabilities {
            key_columns: 3,
            key_rows: 2,
            key_image_size: ImageSize {
                width: 72,
                height: 72,
            },
            dial_count: 0,
            touch_strip: None,
        },
    }
}

/// A device with dials but no touch strip.
pub(super) fn dials_without_strip_info() -> DeviceInfo {
    let mut info = no_dial_info();
    info.serial = "NOSTRIP".into();
    info.capabilities.dial_count = 2;
    info
}

pub(super) fn instance(id: &str, kind: &str, settings: Value) -> ModuleInstance {
    ModuleInstance {
        id: id.into(),
        kind: kind.into(),
        label: None,
        image: None,
        settings,
    }
}

pub(super) fn list(items: usize) -> ModuleInstance {
    instance("list", LIST_KIND, json!({ "items": items }))
}

pub(super) fn fragile(failure: &str) -> ModuleInstance {
    instance(FRAGILE_KIND, FRAGILE_KIND, json!({ "failure": failure }))
}

impl Fixture {
    pub(super) fn config(&self) -> DeviceConfig {
        self.engine.config().borrow().clone()
    }

    /// `modules`, `home` and `dials` on top of the current configuration. A
    /// dial names the instance whose function it is given
    /// ([`FRAGILE_FUNCTION`]), [`NAVIGATION_DIAL`] for the navigation of the
    /// manager, or nothing.
    pub(super) fn with(
        &self,
        modules: Vec<ModuleInstance>,
        home: &[&str],
        dials: &[Option<&str>],
    ) -> DeviceConfig {
        let mut config = self.config();
        config.modules = modules;
        config.home = home.iter().map(|id| (*id).into()).collect();
        config.dials = dials.iter().map(|dial| assignment(*dial)).collect();
        config
    }

    /// Applies `modules`, `home` and `dials` on top of the current
    /// configuration.
    pub(super) async fn configure(
        &self,
        modules: Vec<ModuleInstance>,
        home: &[&str],
        dials: &[Option<&str>],
    ) -> DeviceConfig {
        let config = self.with(modules, home, dials);
        self.engine.apply_config(config).await.unwrap()
    }

    pub(super) fn live(&self) -> LiveSnapshot {
        self.engine.live().borrow().clone()
    }

    pub(super) fn labels(&self) -> Vec<Option<String>> {
        self.live().keys.into_iter().map(|key| key.label).collect()
    }

    pub(super) fn page(&self) -> PageSummary {
        self.live().page
    }

    /// Feedback mark over key `index`.
    pub(super) fn key_mark(&self, index: usize) -> Option<Feedback> {
        self.live().keys[index].feedback
    }

    /// The strip as last drawn.
    pub(super) fn strip(&self) -> StripFace {
        self.live().strip.expect("the device has a strip").face
    }

    /// The reminder of `dial`.
    pub(super) fn hint(&self, dial: usize) -> HintFace {
        self.strip().hints[dial].clone()
    }

    /// Feedback mark over the reminder of `dial`.
    pub(super) fn dial_mark(&self, dial: usize) -> Option<Feedback> {
        self.hint(dial).feedback
    }

    /// What turning `dial` does, as its reminder says.
    pub(super) fn hint_rotation(&self, dial: usize) -> Option<String> {
        self.hint(dial).rotate
    }

    /// Waits until `done` holds, checking after every live state change.
    pub(super) async fn wait_for(&self, done: impl Fn(&Self) -> bool) {
        let mut live = self.engine.live();
        tokio::time::timeout(WAIT, async {
            while !done(self) {
                live.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }

    /// Waits for the next publication of the live state after `action`.
    pub(super) async fn after_change(&self, action: impl FnOnce()) {
        let mut live = self.engine.live();
        live.mark_unchanged();
        action();
        tokio::time::timeout(WAIT, live.changed())
            .await
            .unwrap()
            .unwrap();
    }
}

fn assignment(dial: Option<&str>) -> DialAssignment {
    match dial {
        None => DialAssignment::None,
        Some(NAVIGATION_DIAL) => DialAssignment::Navigation,
        Some(instance) => DialAssignment::Module {
            instance: instance.into(),
            function: FRAGILE_FUNCTION.into(),
        },
    }
}

pub(super) fn labels(expected: &[Option<&str>]) -> Vec<Option<String>> {
    expected
        .iter()
        .map(|label| label.map(str::to_owned))
        .collect()
}
