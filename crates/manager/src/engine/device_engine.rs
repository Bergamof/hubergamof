//! The task driving one device.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use bergadeck_module_api::{
    Command, Feedback, ModuleInstanceId, ModuleSurface, NavigationView, PageContent, PageId,
    RedrawHandle,
};
use bergadeck_protocol::{DeviceConfig, ModuleInstance};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{Notify, broadcast, mpsc, watch};
use tokio::task::spawn_blocking;
use tokio::time::Instant;

use super::feedback::{Marks, Surface};
use super::frame::{DialFace, Frame, FrameKey, KeyFace, KeyTarget, ZoneContentFace, ZoneFace};
use super::handle::{REQUEST_CAPACITY, Request};
use super::instances::{Call, Instances, Outcome, RoutedEvent};
use super::layout::KeyGrid;
use super::live::{LiveSnapshot, PageSummary};
use super::navigation::Navigation;
use super::painter::Painter;
use super::validation::check_modules;
use super::{EngineError, EngineHandle, EngineServices, HOME_PAGE_ID, HOME_PAGE_TITLE};
use crate::device::{self, DeckBackend, InputEvent, StripLayout};
use crate::render::ImageSource;

/// Longest wait for the screens to be blanked when the engine stops.
const BLANK_TIMEOUT: Duration = Duration::from_secs(2);

/// Capacity of the queue of events sent by the module instances; past it,
/// events are dropped (and the sender told so).
pub(crate) const MODULE_EVENT_CAPACITY: usize = 256;

/// Most inputs and module events already queued that are handled together,
/// before a single redraw.
const MAX_BATCHED_WAKES: usize = 32;

/// Page shown on the keys, resolved against the live instances.
enum PageModel {
    Home,
    Module {
        module: ModuleInstanceId,
        page: PageId,
        content: PageContent,
    },
}

/// The displayed page and its keys, before placement on the grid.
struct DisplayedPage {
    id: String,
    title: String,
    keys: Vec<FrameKey>,
}

/// What woke the engine up.
enum Wake {
    Request(Request),
    Input(InputEvent),
    ModuleEvent(RoutedEvent),
    Redraw,
    FeedbackExpired,
    RequestsClosed,
    Disconnected,
}

/// Drives one device: owns its configuration, module instances, navigation
/// and what is drawn. Built by [`DeviceEngine::start`], then consumed by
/// [`DeviceEngine::run`] on its own task.
pub struct DeviceEngine {
    deck: Arc<dyn DeckBackend>,
    services: EngineServices,
    config: DeviceConfig,
    instances: Instances,
    nav: Navigation,
    grid: KeyGrid,
    zone_count: usize,
    painter: Painter,
    marks: Marks,
    /// Brightness the device last accepted, `None` after a failure: applied
    /// again with the next configuration.
    applied_brightness: Option<u8>,
    last_frame: Option<Frame>,
    inputs: broadcast::Receiver<InputEvent>,
    requests: Option<mpsc::Receiver<Request>>,
    events: mpsc::Receiver<RoutedEvent>,
    redraw: Arc<Notify>,
    live: watch::Sender<LiveSnapshot>,
    config_tx: watch::Sender<DeviceConfig>,
}

impl DeviceEngine {
    /// Loads the device configuration, applies its brightness, creates its
    /// module instances and draws the home page.
    ///
    /// # Errors
    ///
    /// The configuration cannot be loaded, or the device cannot be written.
    pub async fn start(
        deck: Arc<dyn DeckBackend>,
        services: EngineServices,
    ) -> Result<(Self, EngineHandle), EngineError> {
        // Subscribe first: no input is lost while starting.
        let inputs = deck.subscribe();
        let info = deck.info().clone();
        let store = services.store.clone();
        let loaded = spawn_blocking(move || store.load(&info)).await??;
        tracing::info!(
            serial = %loaded.config.serial,
            revision = loaded.config.revision,
            outcome = ?loaded.outcome,
            "configuration ready"
        );
        if let Err(error) = check_modules(&services.registry, &loaded.config) {
            tracing::warn!(
                serial = %loaded.config.serial,
                %error,
                "stored configuration refused by a module, used as is"
            );
        }
        let applied_brightness = apply_brightness(deck.as_ref(), loaded.config.brightness).await;

        let redraw = Arc::new(Notify::new());
        let notify = Arc::clone(&redraw);
        let (events_tx, events) = mpsc::channel(MODULE_EVENT_CAPACITY);
        let mut instances = Instances::new(
            Arc::clone(&services.registry),
            RedrawHandle::new(move || notify.notify_one()),
            events_tx,
        );
        instances.sync(&loaded.config.modules);

        let (requests_tx, requests) = mpsc::channel(REQUEST_CAPACITY);
        let (live, live_rx) = watch::channel(LiveSnapshot::empty(&deck.info().serial));
        let (config_tx, config_rx) = watch::channel(loaded.config.clone());
        let handle = EngineHandle::new(deck.info().clone(), requests_tx, live_rx, config_rx);
        let mut engine = Self {
            grid: KeyGrid::new(&deck.info().capabilities),
            zone_count: StripLayout::for_device(deck.info())
                .map_or(0, |layout| usize::from(layout.zone_count())),
            painter: Painter::new(deck.as_ref()),
            deck,
            services,
            config: loaded.config,
            instances,
            nav: Navigation::default(),
            marks: Marks::default(),
            applied_brightness,
            last_frame: None,
            inputs,
            requests: Some(requests),
            events,
            redraw,
            live,
            config_tx,
        };
        engine.refresh().await?;
        Ok((engine, handle))
    }

    /// Runs until the device disconnects or [`EngineHandle::stop`] is called.
    pub async fn run(mut self) {
        loop {
            match self.next_wake().await {
                Wake::Request(Request::Stop(done)) => {
                    self.blank().await;
                    let _ = done.send(());
                    break;
                }
                Wake::Request(Request::Input(event, reply)) => {
                    self.handle_input(event).await;
                    let _ = reply.send(self.refresh().await);
                }
                Wake::Request(Request::ApplyConfig(config, reply)) => {
                    let result = self.apply_config(*config).await;
                    self.refresh_or_log().await;
                    let _ = reply.send(result);
                }
                Wake::Input(event) => {
                    self.handle_input(event).await;
                    if self.handle_queued().await == Queued::Disconnected {
                        break;
                    }
                    self.refresh_or_log().await;
                }
                Wake::ModuleEvent(routed) => {
                    self.handle_event(routed).await;
                    if self.handle_queued().await == Queued::Disconnected {
                        break;
                    }
                    self.refresh_or_log().await;
                }
                Wake::Redraw => self.refresh_or_log().await,
                Wake::FeedbackExpired => {
                    self.marks.expire(Instant::now());
                    self.refresh_or_log().await;
                }
                Wake::RequestsClosed => self.requests = None,
                Wake::Disconnected => break,
            }
        }
        self.instances.clear();
        tracing::debug!(serial = %self.serial(), "device engine stopped");
    }

    async fn next_wake(&mut self) -> Wake {
        loop {
            let wake = tokio::select! {
                request = recv_request(self.requests.as_mut()) => {
                    Some(request.map_or(Wake::RequestsClosed, Wake::Request))
                }
                input = self.inputs.recv() => match input {
                    Ok(InputEvent::Disconnected) | Err(RecvError::Closed) => Some(Wake::Disconnected),
                    Ok(event) => Some(Wake::Input(event)),
                    Err(RecvError::Lagged(missed)) => {
                        tracing::warn!(serial = %self.deck.info().serial, missed, "missed input events");
                        None
                    }
                },
                // The instances keep a sender: the queue never closes.
                Some(routed) = self.events.recv() => Some(Wake::ModuleEvent(routed)),
                () = self.redraw.notified() => Some(Wake::Redraw),
                () = sleep_until(self.marks.next_expiry()) => Some(Wake::FeedbackExpired),
            };
            if let Some(wake) = wake {
                return wake;
            }
        }
    }

    /// Handles the inputs and module events already queued, at most
    /// [`MAX_BATCHED_WAKES`], so that a burst (a dial turned fast) costs one
    /// redraw. Inputs keep their order, and so do events.
    async fn handle_queued(&mut self) -> Queued {
        for _ in 0..MAX_BATCHED_WAKES {
            match self.inputs.try_recv() {
                Ok(InputEvent::Disconnected) | Err(TryRecvError::Closed) => {
                    return Queued::Disconnected;
                }
                Ok(event) => {
                    self.handle_input(event).await;
                    continue;
                }
                Err(TryRecvError::Lagged(missed)) => {
                    tracing::warn!(serial = %self.serial(), missed, "missed input events");
                    continue;
                }
                Err(TryRecvError::Empty) => {}
            }
            match self.events.try_recv() {
                Ok(routed) => self.handle_event(routed).await,
                Err(_) => break,
            }
        }
        Queued::Handled
    }

    fn serial(&self) -> &str {
        &self.deck.info().serial
    }

    async fn handle_input(&mut self, event: InputEvent) {
        match event {
            InputEvent::KeyDown(index) => self.press_key(index).await,
            InputEvent::DialRotate { index, ticks } => {
                self.dial_input(index, Call::DialRotate(ticks)).await;
            }
            InputEvent::DialDown(index) => self.dial_input(index, Call::DialPress).await,
            // Releases act on nothing; touch strip gestures are not used yet.
            InputEvent::KeyUp(_)
            | InputEvent::DialUp(_)
            | InputEvent::TouchTap { .. }
            | InputEvent::TouchSwipe { .. }
            | InputEvent::Disconnected => {}
        }
    }

    /// Hands the press to what the key shows on the last drawn frame, even
    /// if the module content changed since.
    async fn press_key(&mut self, key: u8) {
        let Some(target) = self
            .last_frame
            .as_ref()
            .and_then(|frame| frame.target(key))
            .cloned()
        else {
            return;
        };
        let call = match &target {
            KeyTarget::Home(_) => Call::KeyPress,
            KeyTarget::Item { page, item, .. } => Call::ItemPress(page.clone(), item.clone()),
        };
        let module = target.module().clone();
        self.run_handler(&module, call, vec![Surface::Key(target)])
            .await;
    }

    async fn dial_input(&mut self, dial: u8, call: Call) {
        let Some(module) = self.config.dials.get(usize::from(dial)).cloned().flatten() else {
            return;
        };
        let surfaces = if usize::from(dial) < self.zone_count {
            vec![Surface::Zone(dial)]
        } else {
            Vec::new()
        };
        self.run_handler(&module, call, surfaces).await;
    }

    async fn handle_event(&mut self, routed: RoutedEvent) {
        let module = routed.module.clone();
        let surfaces = self.module_surfaces(&module);
        if let Some(outcome) = self.instances.dispatch(routed).await {
            self.conclude(&module, &surfaces, outcome);
        }
    }

    /// Runs a handler of `module`; `surfaces` show its outcome.
    async fn run_handler(&mut self, module: &ModuleInstanceId, call: Call, surfaces: Vec<Surface>) {
        if let Some(outcome) = self.instances.call(module, call).await {
            self.conclude(module, &surfaces, outcome);
        }
    }

    fn conclude(&mut self, module: &ModuleInstanceId, surfaces: &[Surface], outcome: Outcome) {
        match outcome {
            Outcome::Done(commands) => self.apply_commands(module, surfaces, commands),
            Outcome::Failed => self.show_feedback(surfaces, Feedback::Alert),
        }
    }

    /// Surfaces showing `module` outside of a pressed key: its home key (when
    /// displayed), or else the zones of its dials.
    fn module_surfaces(&self, module: &ModuleInstanceId) -> Vec<Surface> {
        if self.config.home.contains(module) {
            return self.home_surfaces(module);
        }
        self.zone_surfaces(module)
    }

    /// The home key of `module`, if it is on the home page.
    fn home_surfaces(&self, module: &ModuleInstanceId) -> Vec<Surface> {
        if self.config.home.contains(module) {
            vec![Surface::Key(KeyTarget::Home(module.clone()))]
        } else {
            Vec::new()
        }
    }

    /// The zones of the dials `module` is assigned to.
    fn zone_surfaces(&self, module: &ModuleInstanceId) -> Vec<Surface> {
        self.config
            .dials
            .iter()
            .zip(0_u8..)
            .filter(|(assigned, dial)| {
                assigned.as_ref() == Some(module) && usize::from(*dial) < self.zone_count
            })
            .map(|(_, dial)| Surface::Zone(dial))
            .collect()
    }

    /// Surfaces of `module` named by a [`Command::FeedbackOn`].
    fn named_surfaces(&self, module: &ModuleInstanceId, surface: ModuleSurface) -> Vec<Surface> {
        match surface {
            ModuleSurface::HomeKey => self.home_surfaces(module),
            ModuleSurface::Zone => self.zone_surfaces(module),
            ModuleSurface::Item { page, item } => vec![Surface::Key(KeyTarget::Item {
                module: module.clone(),
                page,
                item,
            })],
            // Surfaces added to the API after this version of the host.
            unknown => {
                tracing::warn!(%module, surface = ?unknown, "unsupported feedback surface");
                Vec::new()
            }
        }
    }

    fn show_feedback(&mut self, surfaces: &[Surface], feedback: Feedback) {
        let now = Instant::now();
        for surface in surfaces {
            self.marks.show(surface.clone(), feedback, now);
        }
    }

    fn apply_commands(
        &mut self,
        source: &ModuleInstanceId,
        surfaces: &[Surface],
        commands: Vec<Command>,
    ) {
        for command in commands {
            match command {
                Command::OpenPage(page) => self.nav.open(source.clone(), page),
                Command::Back => self.nav.back(),
                Command::GoHome => self.nav.home(),
                Command::ScrollRows(delta) => {
                    let items = self.page_item_count();
                    let row = self.grid.scroll(self.nav.row(), delta, items);
                    self.nav.set_row(row);
                }
                Command::Feedback(feedback) => self.show_feedback(surfaces, feedback),
                Command::FeedbackOn(surface, feedback) => {
                    let named = self.named_surfaces(source, surface);
                    self.show_feedback(&named, feedback);
                }
                // Commands added to the API after this version of the host.
                unknown => {
                    tracing::warn!(module = %source, command = ?unknown, "unsupported command");
                }
            }
        }
    }

    /// Checks `config` against the modules, saves it, then applies it. Once
    /// saved, the configuration is always adopted; a brightness the device
    /// refuses is only logged, and tried again with the next configuration.
    async fn apply_config(&mut self, config: DeviceConfig) -> Result<DeviceConfig, EngineError> {
        check_modules(&self.services.registry, &config)?;
        let store = self.services.store.clone();
        let info = self.deck.info().clone();
        let saved = spawn_blocking(move || store.save(&info, config)).await??;
        self.instances.sync(&saved.modules);
        self.config = saved.clone();
        self.config_tx.send_replace(saved.clone());
        tracing::info!(serial = %self.serial(), revision = saved.revision, "configuration applied");
        if self.applied_brightness != Some(saved.brightness) {
            self.applied_brightness = apply_brightness(self.deck.as_ref(), saved.brightness).await;
        }
        Ok(saved)
    }

    /// Displayed page; pages that no longer exist are popped.
    fn resolve_page(&mut self) -> PageModel {
        loop {
            let Some(current) = self.nav.current().cloned() else {
                return PageModel::Home;
            };
            match self.instances.page(&current.module, &current.page) {
                Some(content) => {
                    return PageModel::Module {
                        module: current.module,
                        page: current.page,
                        content,
                    };
                }
                None => self.nav.back(),
            }
        }
    }

    fn page_item_count(&mut self) -> usize {
        match self.resolve_page() {
            PageModel::Home => self.config.home.len(),
            PageModel::Module { content, .. } => content.items.len(),
        }
    }

    async fn refresh_or_log(&mut self) {
        if let Err(error) = self.refresh().await {
            tracing::warn!(serial = %self.serial(), %error, "cannot redraw");
        }
    }

    /// Draws what changed and publishes the live state if anything did.
    async fn refresh(&mut self) -> Result<(), EngineError> {
        let frame = self.next_frame();
        let written = self
            .painter
            .paint(self.deck.as_ref(), &self.services.renderer, &frame)
            .await?;
        if self.last_frame.as_ref() == Some(&frame) {
            return Ok(());
        }
        if self
            .last_frame
            .as_ref()
            .is_none_or(|last| last.page != frame.page)
        {
            tracing::info!(
                serial = %self.serial(),
                page = %frame.page.id,
                title = %frame.page.title,
                row = frame.page.row,
                rows = frame.page.row_count,
                "page shown"
            );
        }
        tracing::debug!(serial = %self.serial(), written, "frame drawn");
        self.live
            .send_replace(self.painter.snapshot(self.serial(), &frame));
        self.last_frame = Some(frame);
        Ok(())
    }

    /// Frame to draw now. A module panicking in a getter is replaced by a
    /// placeholder, marked with an alert, and the frame built again; this
    /// ends because placeholders never panic.
    fn next_frame(&mut self) -> Frame {
        for module in self.instances.take_task_panics() {
            let surfaces = self.module_surfaces(&module);
            self.show_feedback(&surfaces, Feedback::Alert);
        }
        loop {
            let page = self.displayed_page();
            self.clamp_row(page.keys.len());
            let frame = self.compose(page);
            let failed = self.instances.take_failures();
            if failed.is_empty() {
                return frame;
            }
            for module in &failed {
                let surfaces = self.module_surfaces(module);
                self.show_feedback(&surfaces, Feedback::Alert);
            }
        }
    }

    /// Keeps the visible row within the page, which may have shrunk since
    /// the last scroll.
    fn clamp_row(&mut self, item_count: usize) {
        let row = self.nav.row().min(self.grid.max_row(item_count));
        self.nav.set_row(row);
    }

    fn displayed_page(&mut self) -> DisplayedPage {
        match self.resolve_page() {
            PageModel::Home => DisplayedPage {
                id: HOME_PAGE_ID.to_owned(),
                title: HOME_PAGE_TITLE.to_owned(),
                keys: self.home_keys(),
            },
            PageModel::Module {
                module,
                page,
                content,
            } => DisplayedPage {
                id: format!("{module}/{page}"),
                title: content.title,
                keys: content
                    .items
                    .into_iter()
                    .map(|item| FrameKey {
                        face: KeyFace::from_content(item.key),
                        target: Some(KeyTarget::Item {
                            module: module.clone(),
                            page: page.clone(),
                            item: item.id,
                        }),
                    })
                    .collect(),
            },
        }
    }

    /// Places `page` on the keys from the visible row, with the feedback
    /// marks, and adds the dials.
    fn compose(&mut self, page: DisplayedPage) -> Frame {
        let row = self.nav.row();
        let row_count = self.grid.row_count(page.keys.len());
        let keys = (0..self.deck.info().capabilities.key_count())
            .map(|key| {
                let shown = u8::try_from(key)
                    .ok()
                    .and_then(|key| page.keys.get(self.grid.item_index(key, row)));
                shown
                    .cloned()
                    .map(|key| self.marked(key))
                    .unwrap_or_default()
            })
            .collect();
        let mut view = NavigationView::new(
            page.title.clone(),
            row,
            row_count,
            u32::from(self.deck.info().capabilities.key_rows),
        );
        if let Some(current) = self.nav.current() {
            view = view.on_page(
                current.module.clone(),
                current.page.clone(),
                self.nav.depth(),
            );
        }
        Frame {
            dials: self.dial_faces(&view),
            page: PageSummary {
                id: page.id,
                title: page.title,
                row,
                row_count,
            },
            keys,
        }
    }

    fn marked(&self, mut key: FrameKey) -> FrameKey {
        key.face.feedback = key
            .target
            .as_ref()
            .and_then(|target| self.marks.key(target));
        key
    }

    /// Keys of the home modules, with the label and image of the
    /// configuration taking precedence over the module's.
    fn home_keys(&mut self) -> Vec<FrameKey> {
        let configured: BTreeMap<&ModuleInstanceId, &ModuleInstance> = self
            .config
            .modules
            .iter()
            .map(|module| (&module.id, module))
            .collect();
        let mut keys = Vec::with_capacity(self.config.home.len());
        for id in &self.config.home {
            let mut face = self
                .instances
                .key(id)
                .map(KeyFace::from_content)
                .unwrap_or_default();
            if let Some(config) = configured.get(id) {
                if let Some(label) = &config.label {
                    face.label = Some(label.clone());
                }
                if let Some(name) = &config.image {
                    match self
                        .services
                        .store
                        .image_path(&self.deck.info().serial, name)
                    {
                        Ok(path) => face.image = Some(ImageSource::File(path)),
                        Err(error) => tracing::warn!(module = %id, %error, "ignoring image"),
                    }
                }
            }
            keys.push(FrameKey {
                face,
                target: Some(KeyTarget::Home(id.clone())),
            });
        }
        keys
    }

    fn dial_faces(&mut self, view: &NavigationView) -> Vec<DialFace> {
        let dial_count = self.deck.info().capabilities.dial_count;
        let mut dials = Vec::with_capacity(usize::from(dial_count));
        for dial in 0..dial_count {
            let module = self.config.dials.get(usize::from(dial)).cloned().flatten();
            let zone = (usize::from(dial) < self.zone_count).then(|| {
                let content = module.as_ref().and_then(|id| self.instances.zone(id, view));
                ZoneFace {
                    content: ZoneContentFace::from_content(content),
                    feedback: self.marks.zone(dial),
                }
            });
            dials.push(DialFace { module, zone });
        }
        dials
    }

    /// Destroys the modules and blanks the screens.
    async fn blank(&mut self) {
        self.instances.clear();
        device::blank(self.deck.as_ref(), BLANK_TIMEOUT).await;
    }
}

/// Whether the device went away while handling queued inputs.
#[derive(Debug, PartialEq, Eq)]
enum Queued {
    Handled,
    Disconnected,
}

/// Sets the brightness of `deck`; returns it when the device accepted it.
async fn apply_brightness(deck: &dyn DeckBackend, percent: u8) -> Option<u8> {
    match deck.set_brightness(percent).await {
        Ok(()) => Some(percent),
        Err(error) => {
            tracing::warn!(serial = %deck.info().serial, %error, "cannot apply the brightness");
            None
        }
    }
}

/// Next request, or pending forever once the queue is closed.
async fn recv_request(requests: Option<&mut mpsc::Receiver<Request>>) -> Option<Request> {
    match requests {
        Some(requests) => requests.recv().await,
        None => std::future::pending().await,
    }
}

/// Sleeps until `deadline`, forever without one.
async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}
