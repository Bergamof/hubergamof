//! The task driving one device.

use std::sync::Arc;
use std::time::Duration;

use bergadeck_module_api::{
    Command, ModuleInstanceId, NavigationView, PageContent, PageId, RedrawHandle,
};
use bergadeck_protocol::DeviceConfig;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{Notify, broadcast, mpsc, watch};
use tokio::task::spawn_blocking;

use super::frame::{DialFace, Frame, KeyFace, ZoneFace};
use super::handle::{REQUEST_CAPACITY, Request};
use super::instances::Instances;
use super::layout::KeyGrid;
use super::live::{LiveSnapshot, PageSummary};
use super::navigation::Navigation;
use super::painter::Painter;
use super::{EngineError, EngineHandle, EngineServices, HOME_PAGE_ID, HOME_PAGE_TITLE};
use crate::device::{DeckBackend, InputEvent, StripLayout};
use crate::render::ImageSource;

/// Longest wait for the screens to be blanked when the engine stops.
const BLANK_TIMEOUT: Duration = Duration::from_secs(2);

/// Page shown on the keys, resolved against the live instances.
enum PageModel {
    Home,
    Module {
        module: ModuleInstanceId,
        page: PageId,
        content: PageContent,
    },
}

/// What woke the engine up.
enum Wake {
    Request(Request),
    Input(InputEvent),
    Redraw,
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
    last_frame: Option<Frame>,
    inputs: broadcast::Receiver<InputEvent>,
    requests: Option<mpsc::Receiver<Request>>,
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
        deck.set_brightness(loaded.config.brightness).await?;

        let redraw = Arc::new(Notify::new());
        let notify = Arc::clone(&redraw);
        let mut instances = Instances::new(
            Arc::clone(&services.registry),
            RedrawHandle::new(move || notify.notify_one()),
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
            last_frame: None,
            inputs,
            requests: Some(requests),
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
                    if let Err(error) = self.refresh().await {
                        tracing::warn!(serial = %self.serial(), %error, "cannot redraw");
                    }
                    let _ = reply.send(result);
                }
                Wake::Input(event) => {
                    self.handle_input(event).await;
                    self.refresh_or_log().await;
                }
                Wake::Redraw => self.refresh_or_log().await,
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
                () = self.redraw.notified() => Some(Wake::Redraw),
            };
            if let Some(wake) = wake {
                return wake;
            }
        }
    }

    fn serial(&self) -> &str {
        &self.deck.info().serial
    }

    async fn handle_input(&mut self, event: InputEvent) {
        match event {
            InputEvent::KeyDown(index) => self.press_key(index).await,
            InputEvent::DialRotate { index, ticks } => {
                if let Some((id, module)) = self.dial_module(index) {
                    let commands = module.on_dial_rotate(ticks).await;
                    self.apply_commands(&id, commands);
                }
            }
            InputEvent::DialDown(index) => {
                if let Some((id, module)) = self.dial_module(index) {
                    let commands = module.on_dial_press().await;
                    self.apply_commands(&id, commands);
                }
            }
            // Releases act on nothing; touch strip gestures are not used yet.
            InputEvent::KeyUp(_)
            | InputEvent::DialUp(_)
            | InputEvent::TouchTap { .. }
            | InputEvent::TouchSwipe { .. }
            | InputEvent::Disconnected => {}
        }
    }

    async fn press_key(&mut self, key: u8) {
        let item = self.grid.item_index(key, self.nav.row());
        match self.resolve_page() {
            PageModel::Home => {
                let Some(id) = self.config.home.get(item).cloned() else {
                    return;
                };
                if let Some(module) = self.instances.get_mut(&id) {
                    let commands = module.on_key_press().await;
                    self.apply_commands(&id, commands);
                }
            }
            PageModel::Module {
                module: id,
                page,
                content,
            } => {
                let Some(item) = content.items.get(item) else {
                    return;
                };
                if let Some(module) = self.instances.get_mut(&id) {
                    let commands = module.on_item_press(&page, &item.id).await;
                    self.apply_commands(&id, commands);
                }
            }
        }
    }

    fn dial_module(
        &mut self,
        dial: u8,
    ) -> Option<(
        ModuleInstanceId,
        &mut (dyn bergadeck_module_api::Module + 'static),
    )> {
        let id = self.config.dials.get(usize::from(dial))?.clone()?;
        let module = self.instances.get_mut(&id)?;
        Some((id, module))
    }

    fn apply_commands(&mut self, source: &ModuleInstanceId, commands: Vec<Command>) {
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
                // Every input is followed by a redraw of what changed.
                Command::Redraw => {}
            }
        }
    }

    async fn apply_config(&mut self, config: DeviceConfig) -> Result<DeviceConfig, EngineError> {
        let store = self.services.store.clone();
        let info = self.deck.info().clone();
        let saved = spawn_blocking(move || store.save(&info, config)).await??;
        if saved.brightness != self.config.brightness {
            self.deck.set_brightness(saved.brightness).await?;
        }
        self.instances.sync(&saved.modules);
        self.config = saved.clone();
        self.config_tx.send_replace(saved.clone());
        tracing::info!(serial = %self.serial(), revision = saved.revision, "configuration applied");
        Ok(saved)
    }

    /// Displayed page; pages that no longer exist are popped.
    fn resolve_page(&mut self) -> PageModel {
        loop {
            let Some(current) = self.nav.current() else {
                return PageModel::Home;
            };
            let content = self
                .instances
                .get(&current.module)
                .and_then(|module| module.page(&current.page));
            match content {
                Some(content) => {
                    return PageModel::Module {
                        module: current.module.clone(),
                        page: current.page.clone(),
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
        let frame = self.build_frame();
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

    fn build_frame(&mut self) -> Frame {
        let (summary, faces) = match self.resolve_page() {
            PageModel::Home => (
                PageSummary {
                    id: HOME_PAGE_ID.to_owned(),
                    title: HOME_PAGE_TITLE.to_owned(),
                    row: 0,
                    row_count: 0,
                },
                self.home_faces(),
            ),
            PageModel::Module {
                module,
                page,
                content,
            } => (
                PageSummary {
                    id: format!("{module}/{page}"),
                    title: content.title,
                    row: 0,
                    row_count: 0,
                },
                content
                    .items
                    .into_iter()
                    .map(|item| KeyFace::from_content(item.key))
                    .collect(),
            ),
        };
        // The page may have shrunk since the last scroll.
        let row = self.nav.row().min(self.grid.max_row(faces.len()));
        self.nav.set_row(row);
        let page = PageSummary {
            row,
            row_count: self.grid.row_count(faces.len()),
            ..summary
        };
        let key_count = self.deck.info().capabilities.key_count();
        let keys = (0..key_count)
            .map(|key| {
                u8::try_from(key)
                    .ok()
                    .and_then(|key| faces.get(self.grid.item_index(key, row)))
                    .cloned()
                    .unwrap_or_default()
            })
            .collect();
        let view = NavigationView {
            page_title: page.title.clone(),
            page: self.nav.current().map(|current| current.page.clone()),
            row,
            row_count: page.row_count,
            visible_rows: u32::from(self.deck.info().capabilities.key_rows),
            depth: self.nav.depth(),
        };
        Frame {
            dials: self.dial_faces(&view),
            page,
            keys,
        }
    }

    /// Keys of the home modules, with the label and image of the
    /// configuration taking precedence over the module's.
    fn home_faces(&self) -> Vec<KeyFace> {
        self.config
            .home
            .iter()
            .map(|id| {
                let mut face = self
                    .instances
                    .get(id)
                    .map(|module| KeyFace::from_content(module.key()))
                    .unwrap_or_default();
                if let Some(config) = self.config.modules.iter().find(|module| module.id == *id) {
                    if let Some(label) = &config.label {
                        face.label = Some(label.clone());
                    }
                    if let Some(image) = config.image.as_deref().and_then(|name| {
                        self.services
                            .store
                            .image_path(self.serial(), name)
                            .inspect_err(
                                |error| tracing::warn!(module = %id, %error, "ignoring image"),
                            )
                            .ok()
                    }) {
                        face.image = Some(ImageSource::File(image));
                    }
                }
                face
            })
            .collect()
    }

    fn dial_faces(&self, view: &NavigationView) -> Vec<DialFace> {
        let dial_count = usize::from(self.deck.info().capabilities.dial_count);
        (0..dial_count)
            .map(|dial| {
                let module = self.config.dials.get(dial).cloned().flatten();
                let zone = (dial < self.zone_count).then(|| {
                    module
                        .as_ref()
                        .and_then(|id| self.instances.get(id))
                        .and_then(|module| module.zone(view))
                        .map_or(ZoneFace::Blank, ZoneFace::from_content)
                });
                DialFace { module, zone }
            })
            .collect()
    }

    /// Destroys the modules and blanks the screens.
    async fn blank(&mut self) {
        self.instances.clear();
        let blank = async {
            self.deck.clear_all().await?;
            self.deck.flush().await
        };
        match tokio::time::timeout(BLANK_TIMEOUT, blank).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(serial = %self.serial(), %error, "cannot blank device");
            }
            Err(_) => tracing::warn!(serial = %self.serial(), "timed out blanking device"),
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
