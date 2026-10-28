use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bergadeck_module_api::{
    Command, ItemId, KeyContent, Module, ModuleCapabilities, ModuleContext, ModuleDescriptor,
    ModuleError, ModuleFactory, PageContent, PageId, PageItem, RedrawHandle, parse_settings,
};
use bergadeck_modules::{ModuleRegistry, builtin_registry};
use bergadeck_protocol::{
    DeviceCapabilities, DeviceConfig, DeviceInfo, DeviceModel, ErrorCode, ImageSize, ModuleInstance,
};
use image::RgbaImage;
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use super::*;
use crate::config_store::ConfigStore;
use crate::device::virtual_deck::{DEMO_SERIAL, VirtualDeck, VirtualDevices, virtual_plus_info};
use crate::device::{DeckBackend, DeviceDiscovery, DeviceError, InputEvent, StripLayout};
use crate::render::ImageKind;

const LIST_KIND: &str = "list";
const LIST_PAGE: &str = "list";
const WAIT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------- Test module

/// Shared with the test: redraw handle of the last instance and a label
/// overriding the key of every instance.
#[derive(Clone, Default)]
struct Probe {
    redraw: Arc<Mutex<Option<RedrawHandle>>>,
    key_label: Arc<Mutex<Option<String>>>,
}

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

/// Key counting its presses (and opening its page); page items `i<n>`:
/// `i0` goes back, `i1` home, `i2` scrolls one row, `i3` opens a nested page.
struct List {
    settings: ListSettings,
    presses: u32,
    probe: Probe,
}

#[async_trait]
impl Module for List {
    fn key(&self) -> KeyContent {
        let label = self.probe.key_label.lock().unwrap().clone();
        KeyContent {
            label: Some(
                label.unwrap_or_else(|| format!("{} {}", self.settings.label, self.presses)),
            ),
            image: None,
            active: self.presses % 2 == 1,
        }
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        self.presses += 1;
        vec![Command::OpenPage(LIST_PAGE.into())]
    }

    fn page(&self, page: &PageId) -> Option<PageContent> {
        (page.0 == LIST_PAGE || page.0 == "nested").then(|| PageContent {
            title: format!("{} ({page})", self.settings.label),
            items: (0..self.settings.items)
                .map(|index| PageItem {
                    id: ItemId(format!("i{index}")),
                    key: KeyContent::with_label(format!("item {index}")),
                })
                .collect(),
        })
    }

    async fn on_item_press(&mut self, _page: &PageId, item: &ItemId) -> Vec<Command> {
        match item.0.as_str() {
            "i0" => vec![Command::Back],
            "i1" => vec![Command::GoHome],
            "i2" => vec![Command::ScrollRows(1), Command::Redraw],
            "i3" => vec![Command::OpenPage("nested".into())],
            _ => Vec::new(),
        }
    }

    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError> {
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
            capabilities: ModuleCapabilities {
                key: true,
                dial: false,
                touch_zone: false,
            },
            settings: Vec::new(),
        }
    }

    fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
        *self.0.redraw.lock().unwrap() = Some(ctx.redraw().clone());
        Ok(Box::new(List {
            settings: parse_settings(settings)?,
            presses: 0,
            probe: self.0.clone(),
        }))
    }
}

// ---------------------------------------------------------------- Counting backend

/// Virtual deck counting the images written.
struct CountingDeck {
    inner: Arc<VirtualDeck>,
    key_writes: AtomicUsize,
    strip_writes: AtomicUsize,
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

    async fn set_key_image(&self, index: u8, image: RgbaImage) -> Result<(), DeviceError> {
        self.key_writes.fetch_add(1, Ordering::SeqCst);
        self.inner.set_key_image(index, image).await
    }

    async fn set_strip_region(&self, x: u32, image: RgbaImage) -> Result<(), DeviceError> {
        self.strip_writes.fetch_add(1, Ordering::SeqCst);
        self.inner.set_strip_region(x, image).await
    }

    async fn set_brightness(&self, percent: u8) -> Result<(), DeviceError> {
        self.inner.set_brightness(percent).await
    }

    async fn clear_all(&self) -> Result<(), DeviceError> {
        self.inner.clear_all().await
    }

    async fn flush(&self) -> Result<(), DeviceError> {
        self.inner.flush().await
    }

    async fn close(&self) {
        self.inner.close().await;
    }
}

impl CountingDeck {
    fn writes(&self) -> (usize, usize) {
        (
            self.key_writes.swap(0, Ordering::SeqCst),
            self.strip_writes.swap(0, Ordering::SeqCst),
        )
    }
}

// ---------------------------------------------------------------- Fixture

struct Fixture {
    _dir: TempDir,
    store: ConfigStore,
    deck: Arc<CountingDeck>,
    engine: EngineHandle,
    task: JoinHandle<()>,
    probe: Probe,
}

fn registry(probe: &Probe) -> Arc<ModuleRegistry> {
    let mut registry = builtin_registry();
    registry.register(Arc::new(ListFactory(probe.clone())));
    Arc::new(registry)
}

fn services(dir: &TempDir, probe: &Probe) -> EngineServices {
    let registry = registry(probe);
    EngineServices {
        store: ConfigStore::new(dir.path(), Arc::clone(&registry) as _),
        renderer: Arc::new(crate::render::Renderer::new().unwrap()),
        registry,
    }
}

async fn start(info: DeviceInfo) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let probe = Probe::default();
    let services = services(&dir, &probe);
    let store = services.store.clone();
    let deck = Arc::new(CountingDeck {
        inner: Arc::new(VirtualDeck::new(info)),
        key_writes: AtomicUsize::new(0),
        strip_writes: AtomicUsize::new(0),
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

fn no_dial_info() -> DeviceInfo {
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

fn instance(id: &str, kind: &str, settings: Value) -> ModuleInstance {
    ModuleInstance {
        id: id.into(),
        kind: kind.into(),
        label: None,
        image: None,
        settings,
    }
}

fn list(items: usize) -> ModuleInstance {
    instance("list", LIST_KIND, json!({ "items": items }))
}

impl Fixture {
    fn config(&self) -> DeviceConfig {
        self.engine.config().borrow().clone()
    }

    /// Applies `modules`, `home` and `dials` on top of the current
    /// configuration.
    async fn configure(
        &self,
        modules: Vec<ModuleInstance>,
        home: &[&str],
        dials: &[Option<&str>],
    ) -> DeviceConfig {
        let mut config = self.config();
        config.modules = modules;
        config.home = home.iter().map(|id| (*id).into()).collect();
        config.dials = dials.iter().map(|id| id.map(Into::into)).collect();
        self.engine.apply_config(config).await.unwrap()
    }

    fn live(&self) -> LiveSnapshot {
        self.engine.live().borrow().clone()
    }

    fn labels(&self) -> Vec<Option<String>> {
        self.live().keys.into_iter().map(|key| key.label).collect()
    }

    fn page(&self) -> PageSummary {
        self.live().page
    }

    /// Lines of the zone above `dial`.
    fn zone_lines(&self, dial: usize) -> Vec<String> {
        self.live().dials[dial].zone.clone().unwrap().lines
    }
}

fn labels(expected: &[Option<&str>]) -> Vec<Option<String>> {
    expected
        .iter()
        .map(|label| label.map(str::to_owned))
        .collect()
}

// ---------------------------------------------------------------- Tests

#[tokio::test]
async fn default_configuration_shows_placeholders_and_dial_zones() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    let live = fixture.live();
    assert_eq!(live.page.id, HOME_PAGE_ID);
    assert_eq!(live.page.title, HOME_PAGE_TITLE);
    assert_eq!(
        fixture.labels(),
        labels(&[
            Some("audio indisponible"),
            Some("pomodoro indisponible"),
            Some("docker indisponible"),
            Some("intellij indisponible"),
            Some("claude indisponible"),
            None,
            None,
            None,
        ])
    );
    assert!(live.keys[5].image.is_none());
    let modules: Vec<_> = live
        .dials
        .iter()
        .map(|dial| dial.module_id.clone().unwrap().0)
        .collect();
    assert_eq!(modules, ["clock", "audio", "pomodoro", "navigation"]);
    let clock = fixture.zone_lines(0);
    assert_eq!(clock.len(), 2);
    assert!(clock[0].contains(':'));
    // A placeholder has no zone: black, without image.
    assert_eq!(fixture.zone_lines(1), Vec::<String>::new());
    assert!(live.dials[1].zone.as_ref().unwrap().image.is_none());
    assert_eq!(fixture.zone_lines(3), ["Accueil"]);
}

#[tokio::test]
async fn overflowing_pages_scroll_by_rows_within_bounds() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(
            vec![list(11), instance("nav", "navigation", json!({}))],
            &["list"],
            &[Some("nav"), None, None, None],
        )
        .await;

    fixture.engine.press_key(0).await.unwrap();
    let page = fixture.page();
    assert_eq!(page.id, "list/list");
    assert_eq!(page.title, "list (list)");
    assert_eq!((page.row, page.row_count), (0, 3));
    assert_eq!(fixture.labels()[7].as_deref(), Some("item 7"));
    assert_eq!(fixture.zone_lines(0), ["list (list)", "ligne 1/3"]);

    fixture.engine.rotate_dial(0, 5).await.unwrap();
    assert_eq!(fixture.page().row, 1);
    assert_eq!(
        fixture.labels(),
        labels(&[
            Some("item 4"),
            Some("item 5"),
            Some("item 6"),
            Some("item 7"),
            Some("item 8"),
            Some("item 9"),
            Some("item 10"),
            None,
        ])
    );
    assert_eq!(fixture.zone_lines(0), ["list (list)", "ligne 2/3"]);
    // Beyond the content: blank.
    assert!(fixture.live().keys[7].image.is_none());
    assert!(
        fixture
            .deck
            .inner
            .key_image(7)
            .unwrap()
            .pixels()
            .all(|p| p.0 == [0, 0, 0, 255])
    );

    fixture.engine.rotate_dial(0, -9).await.unwrap();
    assert_eq!(fixture.page().row, 0);
}

#[tokio::test]
async fn devices_without_dials_scroll_through_module_commands() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(8)], &["list"], &[]).await;
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!((fixture.page().row, fixture.page().row_count), (0, 3));

    // Item i2 scrolls one row; the page then shows items 3..8.
    fixture.engine.press_key(2).await.unwrap();
    assert_eq!(fixture.page().row, 1);
    assert_eq!(
        fixture.labels(),
        labels(&[
            Some("item 3"),
            Some("item 4"),
            Some("item 5"),
            Some("item 6"),
            Some("item 7"),
            None,
        ])
    );
    // A key beyond the content does nothing.
    fixture.engine.press_key(5).await.unwrap();
    assert_eq!(fixture.page().row, 1);

    assert!(fixture.live().dials.is_empty());
    let error = fixture.engine.rotate_dial(0, 1).await.unwrap_err();
    assert_eq!(error.error_code(), ErrorCode::InvalidInput);
    let error = fixture.engine.press_key(6).await.unwrap_err();
    assert_eq!(error.error_code(), ErrorCode::InvalidInput);
}

#[tokio::test]
async fn back_and_home_unwind_the_navigation_stack() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(
            vec![list(4), instance("nav", "navigation", json!({}))],
            &["list"],
            &[None, None, None, Some("nav")],
        )
        .await;
    // Back on the home page does nothing.
    fixture.engine.press_dial(3).await.unwrap();
    assert_eq!(fixture.page().id, HOME_PAGE_ID);

    fixture.engine.press_key(0).await.unwrap();
    fixture.engine.press_key(3).await.unwrap();
    assert_eq!(fixture.page().id, "list/nested");
    // Item i0 goes back.
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, "list/list");

    fixture.engine.press_key(3).await.unwrap();
    // Item i1 goes home.
    fixture.engine.press_key(1).await.unwrap();
    assert_eq!(fixture.page().id, HOME_PAGE_ID);

    fixture.engine.press_key(0).await.unwrap();
    fixture.engine.press_dial(3).await.unwrap();
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
    assert_eq!(fixture.zone_lines(3), ["Accueil"]);
}

#[tokio::test]
async fn hardware_inputs_are_handled_like_simulated_ones() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(vec![list(4)], &["list"], &[None; 4])
        .await;
    let mut live = fixture.engine.live();
    live.mark_unchanged();
    fixture.deck.inner.inject(InputEvent::KeyDown(0)).unwrap();
    tokio::time::timeout(WAIT, live.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fixture.page().id, "list/list");
}

#[tokio::test]
async fn unknown_kinds_get_a_placeholder_key() {
    let fixture = start(no_dial_info()).await;
    let mut weird = instance("weird", "teleporter", json!({}));
    weird.label = None;
    fixture
        .configure(vec![weird, list(0)], &["weird", "list"], &[])
        .await;
    assert_eq!(
        fixture.labels()[..2],
        labels(&[Some("teleporter indisponible"), Some("list 0")])
    );
    // Pressing it does nothing.
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
}

#[tokio::test]
async fn applying_a_config_keeps_the_state_of_unchanged_instances() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(2)], &["list"], &[]).await;
    fixture.engine.press_key(0).await.unwrap();
    fixture.engine.press_key(1).await.unwrap();
    assert_eq!(fixture.labels()[0].as_deref(), Some("list 1"));
    assert!(fixture.live().keys[0].active);

    // Same id and kind: new settings, same press count.
    let renamed = instance("list", LIST_KIND, json!({ "label": "Liste", "items": 2 }));
    let saved = fixture.configure(vec![renamed], &["list"], &[]).await;
    assert_eq!(fixture.labels()[0].as_deref(), Some("Liste 1"));
    assert_eq!(*fixture.engine.config().borrow(), saved);

    // The configuration label wins over the module's.
    let mut labelled = list(2);
    labelled.label = Some("Ma liste".into());
    fixture.configure(vec![labelled], &["list"], &[]).await;
    assert_eq!(fixture.labels()[0].as_deref(), Some("Ma liste"));

    // Another id: a new instance.
    let other = instance("other", LIST_KIND, json!({}));
    fixture.configure(vec![other], &["other"], &[]).await;
    assert_eq!(fixture.labels()[0].as_deref(), Some("list 0"));
}

#[tokio::test]
async fn removing_the_displayed_module_returns_home() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(2)], &["list"], &[]).await;
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, "list/list");
    fixture.configure(Vec::new(), &[], &[]).await;
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
    assert_eq!(fixture.labels(), vec![None; 6]);
}

#[tokio::test]
async fn stale_revisions_are_rejected_without_change() {
    let fixture = start(no_dial_info()).await;
    let current = fixture.configure(vec![list(2)], &["list"], &[]).await;
    let mut stale = current.clone();
    stale.revision -= 1;
    stale.home.clear();
    let error = fixture.engine.apply_config(stale).await.unwrap_err();
    assert_eq!(error.error_code(), ErrorCode::StaleRevision);
    assert_eq!(*fixture.engine.config().borrow(), current);
    assert_eq!(fixture.labels()[0].as_deref(), Some("list 0"));
}

#[tokio::test]
async fn only_changed_keys_are_rewritten() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(vec![list(0)], &["list"], &[None; 4])
        .await;
    let _ = fixture.deck.writes();

    let mut live = fixture.engine.live();
    live.mark_unchanged();
    *fixture.probe.key_label.lock().unwrap() = Some("changed".into());
    let redraw = fixture.probe.redraw.lock().unwrap().clone().unwrap();
    redraw.request_redraw();
    tokio::time::timeout(WAIT, live.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fixture.deck.writes(), (1, 0));
    assert_eq!(fixture.labels()[0].as_deref(), Some("changed"));

    // Nothing changed: nothing written.
    redraw.request_redraw();
    fixture.engine.press_key(7).await.unwrap();
    assert_eq!(fixture.deck.writes(), (0, 0));
}

#[tokio::test]
async fn dial_zones_follow_their_modules() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(
            vec![instance("nav", "navigation", json!({}))],
            &[],
            &[Some("nav"), None, None, None],
        )
        .await;
    let strip = fixture.deck.inner.screen().strip.unwrap();
    let layout = StripLayout::for_device(fixture.deck.info()).unwrap();
    let zone_width = layout.zone_size().width;
    let lit = |zone: u32| {
        (zone * zone_width..(zone + 1) * zone_width)
            .any(|x| (0..strip.height()).any(|y| strip.get_pixel(x, y).0[0] > 128))
    };
    assert!(lit(0));
    assert!(!lit(1));
    let live = fixture.live();
    assert_eq!(live.dials[0].module_id, Some("nav".into()));
    assert_eq!(live.dials[1].module_id, None);
    assert_eq!(fixture.zone_lines(1), Vec::<String>::new());
}

#[tokio::test]
async fn configured_images_replace_module_images() {
    let fixture = start(no_dial_info()).await;
    let mut png = std::io::Cursor::new(Vec::new());
    RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let name = fixture
        .store
        .store_image("NODIAL", png.get_ref(), ImageKind::Png)
        .unwrap();
    let mut with_image = list(0);
    with_image.image = Some(name);
    fixture.configure(vec![with_image], &["list"], &[]).await;
    let key = fixture.deck.inner.key_image(0).unwrap();
    assert_eq!(key.get_pixel(36, 10).0, [255, 0, 0, 255]);
}

#[tokio::test]
async fn live_state_is_published_after_each_visible_change() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(1)], &["list"], &[]).await;
    let mut live = fixture.engine.live();
    live.mark_unchanged();

    fixture.engine.press_key(0).await.unwrap();
    assert!(live.has_changed().unwrap());
    let state = live.borrow_and_update().to_live_state().unwrap();
    assert_eq!(state.serial, "NODIAL");
    assert_eq!(state.page.title, "list (list)");
    assert_eq!(state.page.keys.len(), 6);
    assert!(
        state.page.keys[0]
            .image
            .as_deref()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
    assert_eq!(state.page.keys[1].image, None);

    // An input changing nothing publishes nothing.
    fixture.engine.press_key(4).await.unwrap();
    assert!(!live.has_changed().unwrap());
}

#[tokio::test]
async fn disconnecting_the_device_stops_the_engine() {
    let fixture = start(no_dial_info()).await;
    fixture.deck.close().await;
    tokio::time::timeout(WAIT, fixture.task)
        .await
        .unwrap()
        .unwrap();
    let error = fixture.engine.press_key(0).await.unwrap_err();
    assert_eq!(error.error_code(), ErrorCode::UnknownDevice);
}

#[tokio::test]
async fn stopping_the_engine_blanks_the_device() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    assert!(fixture.deck.inner.key_image(0).is_some());
    fixture.engine.stop().await;
    tokio::time::timeout(WAIT, fixture.task)
        .await
        .unwrap()
        .unwrap();
    assert!(fixture.deck.inner.screen().keys.iter().all(Option::is_none));
    // Stopping again returns at once.
    fixture.engine.stop().await;
}

#[tokio::test]
async fn supervisor_restarts_engines_on_reattach() {
    let dir = tempfile::tempdir().unwrap();
    let discovery = DeviceDiscovery::new();
    let supervisor = Supervisor::start(&discovery, services(&dir, &Probe::default()));
    let devices = VirtualDevices::new(discovery.clone());
    let engines = supervisor.engines();
    let wait_for_engine = |previous: Option<u64>| {
        let engines = engines.clone();
        async move {
            loop {
                if let Some(engine) = engines
                    .get(DEMO_SERIAL)
                    .filter(|engine| Some(engine.id()) != previous)
                {
                    return engine;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    };

    let _deck = devices.attach(virtual_plus_info(DEMO_SERIAL));
    let first = tokio::time::timeout(WAIT, wait_for_engine(None))
        .await
        .unwrap();
    assert!(devices.detach(DEMO_SERIAL).await);
    tokio::time::timeout(WAIT, async {
        while engines.get(DEMO_SERIAL).is_some() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();

    let deck = devices.attach(virtual_plus_info(DEMO_SERIAL));
    let second = tokio::time::timeout(WAIT, wait_for_engine(Some(first.id())))
        .await
        .unwrap();
    assert_eq!(second.live().borrow().page.id, HOME_PAGE_ID);

    supervisor.shutdown().await;
    assert!(deck.screen().keys.iter().all(Option::is_none));
    assert!(engines.handles().is_empty());
}
