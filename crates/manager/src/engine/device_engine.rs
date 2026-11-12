//! The task driving one device.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hubergamof_module_api::{
    Command, DialFunctionId, Feedback, ModuleInstanceId, ModuleSurface, PageContent, PageId,
    RedrawHandle, UnreadNotification,
};
use hubergamof_protocol::{DeviceConfig, DialAssignment, ModuleInstance};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{Notify, broadcast, mpsc, watch};
use tokio::task::spawn_blocking;
use tokio::time::Instant;

use super::feedback::{Marks, Surface};
use super::frame::{
    ContextFace, ContextGesture, Frame, FrameKey, KeyFace, KeyTarget, NotificationAction,
    NotificationFace, StripOverlay,
};
use super::handle::{REQUEST_CAPACITY, Request};
use super::instances::{Call, Instances, Outcome, RoutedEvent};
use super::layout::KeyGrid;
use super::live::{LiveSnapshot, PageSummary};
use super::navigation::Navigation;
use super::notifications::NotificationCenter;
use super::painter::Painter;
use super::placement_notice;
use super::validation::check_modules;
use super::{EngineError, EngineHandle, EngineServices, HOME_PAGE_ID, HOME_PAGE_TITLE};
use crate::config_store::{LoadOutcome, check_config};
use crate::device::{self, DeckBackend, InputEvent, StripLayout, TouchPoint};
use crate::render::{ImageSource, is_system_panel};
use crate::strip::SystemHistory;

mod strip;

/// Reminder of the navigation dial under a notification: its press hides it.
const HIDE_NOTIFICATION: &str = "Masquer";

/// Longest wait for the screens to be blanked when the engine stops.
const BLANK_TIMEOUT: Duration = Duration::from_secs(2);

/// Capacity of the queue of events sent by the module instances; past it,
/// events are dropped (and the sender told so).
pub(crate) const MODULE_EVENT_CAPACITY: usize = 256;

/// Most inputs and module events already queued that are handled together,
/// before a single redraw.
const MAX_BATCHED_WAKES: usize = 32;

/// How long a key must be held for its label to show on the strip instead of
/// its action running when it is released.
pub(crate) const HOLD_THRESHOLD: Duration = Duration::from_millis(400);

/// How long after the first drawing the whole touch strip is written once
/// more: a device just connected may lose the images written while it
/// starts.
pub(super) const STRIP_SETTLE_DELAY: Duration = Duration::from_millis(400);

/// A key being held down, and what releasing it does.
struct HeldKey {
    key: u8,
    /// What the key showed when it was pressed.
    target: KeyTarget,
    label: Option<String>,
    since: Instant,
    /// Whether it was held long enough for its label to show: releasing it
    /// then runs no action. Never set for a key without a label, which has
    /// nothing to show and keeps acting on release however long it is held.
    long: bool,
}

impl HeldKey {
    fn long_at(&self) -> Instant {
        self.since + HOLD_THRESHOLD
    }

    /// When the label is due, `None` once it is or for a key without one.
    fn label_due(&self) -> Option<Instant> {
        (!self.long && self.label.is_some()).then(|| self.long_at())
    }

    /// Whether releasing the key at `now` is the end of a hold, not a press.
    fn is_hold(&self, now: Instant) -> bool {
        self.label.is_some() && (self.long || now >= self.long_at())
    }

    fn shown_label(&self) -> Option<&str> {
        self.label.as_deref().filter(|_| self.long)
    }
}

/// The contextual dial held pressed on a page that declares a long press:
/// its action waits for the hold to last, or for the release.
struct HeldDial {
    dial: u8,
    /// What the dial acts on, as displayed when it was pressed.
    context: ContextFace,
    since: Instant,
}

impl HeldDial {
    /// When the long press is due.
    fn due(&self) -> Instant {
        self.since + HOLD_THRESHOLD
    }
}

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
    context: Option<ContextFace>,
}

/// What woke the engine up.
enum Wake {
    Request(Request),
    Input(InputEvent),
    ModuleEvent(RoutedEvent),
    Redraw,
    FeedbackExpired,
    /// The notification on the strip moves a step or may have expired.
    NotificationStep,
    /// A key has been held long enough to show its label.
    HoldElapsed,
    /// The contextual dial has been held long enough for a long press.
    DialHoldElapsed,
    /// The device had time to settle: the strip is written again.
    StripSettled,
    /// The time shown on the strip changes.
    ClockTick,
    /// The system measures of the strip are due.
    SystemSample,
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
    /// Whether the device has a touch strip.
    has_strip: bool,
    painter: Painter,
    marks: Marks,
    notifications: NotificationCenter,
    held: Option<HeldKey>,
    /// The contextual dial being held, waiting for a long press or its release.
    held_dial: Option<HeldDial>,
    /// The key that held the cursor of the contextual dial when the page was
    /// last composed: the page scrolls to it when it moves.
    focus: Option<KeyTarget>,
    /// Brightness the device last accepted, `None` after a failure: applied
    /// again with the next configuration.
    applied_brightness: Option<u8>,
    last_frame: Option<Frame>,
    /// When the strip is written again after the first drawing, `None`
    /// once done (or without a strip).
    strip_settle: Option<Instant>,
    /// Recent system measures, for the curves of the strip.
    system: SystemHistory,
    /// When the system is measured next, `None` without a strip.
    next_system_sample: Option<Instant>,
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
        let has_strip = StripLayout::for_device(deck.info()).is_some();
        let notifications = NotificationCenter::new(has_strip);
        let mut instances = Instances::new(
            Arc::clone(&services.registry),
            RedrawHandle::new(move || notify.notify_one()),
            events_tx,
            notifications.handle(),
            services.store.image_library(&deck.info().serial),
        );
        instances.sync(&loaded.config.modules);

        let (requests_tx, requests) = mpsc::channel(REQUEST_CAPACITY);
        let (live, live_rx) = watch::channel(LiveSnapshot::empty(&deck.info().serial));
        let (config_tx, config_rx) = watch::channel(loaded.config.clone());
        let handle = EngineHandle::new(deck.info().clone(), requests_tx, live_rx, config_rx);
        let mut engine = Self {
            grid: KeyGrid::new(&deck.info().capabilities),
            has_strip,
            painter: Painter::new(deck.as_ref()),
            deck,
            services,
            config: loaded.config,
            instances,
            nav: Navigation::default(),
            marks: Marks::default(),
            notifications,
            held: None,
            held_dial: None,
            focus: None,
            applied_brightness,
            last_frame: None,
            strip_settle: None,
            system: SystemHistory::default(),
            next_system_sample: None,
            inputs,
            requests: Some(requests),
            events,
            redraw,
            live,
            config_tx,
        };
        if let LoadOutcome::Normalized { changes } = &loaded.outcome {
            engine.notifications.notify(
                placement_notice::source(),
                None,
                &placement_notice::notice(changes),
                Instant::now(),
            );
        }
        if has_strip {
            engine.strip_settle = Some(Instant::now() + STRIP_SETTLE_DELAY);
            engine.sample_system().await;
        }
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
                    self.handle_simulated_input(event).await;
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
                // The strip changed (clock) or a module asked for a redraw.
                Wake::Redraw | Wake::ClockTick => self.refresh_or_log().await,
                Wake::NotificationStep => {
                    self.notifications.step(Instant::now());
                    self.refresh_or_log().await;
                }
                Wake::HoldElapsed => {
                    if let Some(held) = &mut self.held {
                        held.long = true;
                    }
                    self.refresh_or_log().await;
                }
                Wake::DialHoldElapsed => {
                    self.long_press_context().await;
                    self.refresh_or_log().await;
                }
                Wake::FeedbackExpired => {
                    self.marks.expire(Instant::now());
                    self.refresh_or_log().await;
                }
                Wake::StripSettled => {
                    self.strip_settle = None;
                    self.painter.forget_strip();
                    self.refresh_or_log().await;
                }
                Wake::SystemSample => {
                    self.sample_system().await;
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
            let clock_tick = self.next_clock_tick();
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
                () = sleep_until(self.notifications.next_step()) => Some(Wake::NotificationStep),
                () = sleep_until(self.held.as_ref().and_then(HeldKey::label_due)) => {
                    Some(Wake::HoldElapsed)
                }
                () = sleep_until(self.held_dial.as_ref().map(HeldDial::due)) => {
                    Some(Wake::DialHoldElapsed)
                }
                () = sleep_until(self.strip_settle) => Some(Wake::StripSettled),
                () = sleep_until(clock_tick) => Some(Wake::ClockTick),
                () = sleep_until(self.next_system_sample) => Some(Wake::SystemSample),
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
            InputEvent::KeyDown(index) => self.hold_key(index),
            InputEvent::KeyUp(index) => self.release_key(index).await,
            InputEvent::DialRotate { index, ticks } => {
                // Turning a dial held pressed is neither a press nor a long one.
                self.held_dial.take_if(|held| held.dial == index);
                self.dial_input(index, DialInput::Rotate(ticks)).await;
            }
            InputEvent::DialDown(index) => self.dial_down(index).await,
            InputEvent::DialUp(index) => self.release_dial(index).await,
            InputEvent::TouchTap { point, long } => self.tap_strip(point, long).await,
            // Swipes are not used yet.
            InputEvent::TouchSwipe { .. } | InputEvent::Disconnected => {}
        }
    }

    /// A simulated input of the web UI: a key press is complete (pressed
    /// and released at once), as nothing holds a key or a dial there.
    async fn handle_simulated_input(&mut self, event: InputEvent) {
        match event {
            InputEvent::KeyDown(index) => self.press_key(index).await,
            InputEvent::DialDown(index) => self.dial_input(index, DialInput::Press).await,
            _ => self.handle_input(event).await,
        }
    }

    /// Starts holding `key`: its action waits for the release, and is
    /// dropped if the key is held until its label shows.
    fn hold_key(&mut self, key: u8) {
        let Some(frame) = &self.last_frame else {
            return;
        };
        let Some(target) = frame.target(key).cloned() else {
            return;
        };
        let label = frame
            .keys
            .get(usize::from(key))
            .and_then(|key| key.face.label.as_deref())
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .map(str::to_owned);
        self.held = Some(HeldKey {
            key,
            target,
            label,
            since: Instant::now(),
            long: false,
        });
    }

    /// Releases `key`: runs its action unless it was held long enough for
    /// the label to show.
    async fn release_key(&mut self, key: u8) {
        let Some(held) = self.held.take_if(|held| held.key == key) else {
            return;
        };
        if held.is_hold(Instant::now()) {
            return;
        }
        self.press_target(held.target).await;
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
        self.press_target(target).await;
    }

    async fn press_target(&mut self, target: KeyTarget) {
        let call = match &target {
            KeyTarget::Home(_) => Call::KeyPress,
            KeyTarget::Item { page, item, .. } => Call::ItemPress(page.clone(), item.clone()),
        };
        let module = target.module().clone();
        self.run_handler(&module, call, vec![Surface::Key(target)])
            .await;
    }

    async fn dial_input(&mut self, dial: u8, input: DialInput) {
        let assignment = self.dial_assignment(dial);
        if self.notifications.shown().is_some() {
            self.dial_input_under_notification(&assignment, &input)
                .await;
            return;
        }
        match assignment {
            DialAssignment::Navigation => self.navigate(input),
            DialAssignment::Module { instance, function } => {
                let surfaces = self.pressed_dial_surfaces(dial);
                self.run_handler(&instance, input.into_call(function), surfaces)
                    .await;
            }
            DialAssignment::Contextual => self.context_input(dial, input).await,
            DialAssignment::None => {}
        }
    }

    /// Under a notification only two dials act: the navigation dial, whose
    /// press removes the notification leaving it unread, and the contextual
    /// dial, whose press does what the notification declares and reads it.
    async fn dial_input_under_notification(
        &mut self,
        assignment: &DialAssignment,
        input: &DialInput,
    ) {
        if !matches!(input, DialInput::Press) {
            return;
        }
        match assignment {
            DialAssignment::Navigation => self.notifications.dismiss(Instant::now()),
            DialAssignment::Contextual => self.press_notification_action().await,
            _ => {}
        }
    }

    /// Hands the press to the module that sent the notification shown, if it
    /// declares what the press does, then reads the notification; a failed
    /// handler leaves it unread (the module shows the alert).
    async fn press_notification_action(&mut self) {
        let Some(shown) = self.notifications.shown() else {
            return;
        };
        if !shown.notification.context_dial.reacts_to_press() {
            return;
        }
        let (id, source) = (shown.notification.id, shown.notification.source.clone());
        let surfaces = self.failure_surfaces(&source);
        let Some(outcome) = self
            .instances
            .call(&source, Call::NotificationPress(id))
            .await
        else {
            return;
        };
        let done = matches!(outcome, Outcome::Done(_));
        self.conclude(&source, &surfaces, outcome);
        if done
            && self
                .notifications
                .shown()
                .is_some_and(|s| s.notification.id == id)
        {
            self.notifications.read(Instant::now());
        }
    }

    fn dial_assignment(&self, dial: u8) -> DialAssignment {
        self.config
            .dials
            .get(usize::from(dial))
            .cloned()
            .unwrap_or(DialAssignment::None)
    }

    /// The reminder of `dial` shows the outcome of what it does.
    fn pressed_dial_surfaces(&self, dial: u8) -> Vec<Surface> {
        if self.has_strip {
            vec![Surface::Dial(dial)]
        } else {
            Vec::new()
        }
    }

    /// A press of a dial of the device. The contextual dial starts holding
    /// when the page displayed declares a long press, its press then waiting
    /// for the release; otherwise the press acts at once.
    async fn dial_down(&mut self, dial: u8) {
        if let Some(held) = self.start_holding_dial(dial) {
            self.held_dial = Some(held);
            return;
        }
        self.dial_input(dial, DialInput::Press).await;
    }

    fn start_holding_dial(&self, dial: u8) -> Option<HeldDial> {
        if self.dial_assignment(dial) != DialAssignment::Contextual
            || self.notifications.shown().is_some()
        {
            return None;
        }
        let context = self.last_frame.as_ref()?.context.clone()?;
        context.dial.reacts_to_long_press().then(|| HeldDial {
            dial,
            context,
            since: Instant::now(),
        })
    }

    /// The release of `dial`: a press, if it was held for less than a long
    /// press.
    async fn release_dial(&mut self, dial: u8) {
        let Some(held) = self.held_dial.take_if(|held| held.dial == dial) else {
            return;
        };
        if self.notifications.shown().is_none() {
            self.run_context(dial, &held.context, ContextGesture::Press)
                .await;
        }
    }

    /// The long press of the contextual dial held until it is due.
    async fn long_press_context(&mut self) {
        let Some(held) = self.held_dial.take() else {
            return;
        };
        if self.notifications.shown().is_some() {
            return;
        }
        self.run_context(held.dial, &held.context, ContextGesture::LongPress)
            .await;
    }

    /// A rotation or a press of the contextual dial: handled by the module
    /// of the page displayed, if the page declares that it reacts.
    async fn context_input(&mut self, dial: u8, input: DialInput) {
        let Some(context) = self.last_frame.as_ref().and_then(|f| f.context.clone()) else {
            return;
        };
        let gesture = match input {
            DialInput::Rotate(ticks) => ContextGesture::Rotate(ticks),
            DialInput::Press => ContextGesture::Press,
        };
        self.run_context(dial, &context, gesture).await;
    }

    /// Hands `gesture` to the module of `context`, if its page declares it.
    async fn run_context(&mut self, dial: u8, context: &ContextFace, gesture: ContextGesture) {
        let Some(call) = context.call(gesture) else {
            return;
        };
        let surfaces = self.pressed_dial_surfaces(dial);
        self.run_handler(&context.module, call, surfaces).await;
    }

    /// The navigation dial: turning scrolls the rows of the displayed page,
    /// pressing goes back one page.
    fn navigate(&mut self, input: DialInput) {
        match input {
            DialInput::Rotate(ticks) => self.scroll_rows(ticks),
            DialInput::Press => self.nav.back(),
        }
    }

    fn scroll_rows(&mut self, delta: i32) {
        let items = self.page_item_count();
        let row = self.grid.scroll(self.nav.row(), delta, items);
        self.nav.set_row(row);
    }

    /// A tap on the strip reads the notification on it, unless the label of a
    /// held key covers it: it is not what the user sees. With nothing on the
    /// strip, a tap on the system measures swaps their history for the
    /// current values, as the setting of the web UI does.
    async fn tap_strip(&mut self, point: TouchPoint, long: bool) {
        let covered = self
            .held
            .as_ref()
            .is_some_and(|held| held.shown_label().is_some());
        if covered {
            return;
        }
        if self.notifications.shown().is_some() {
            self.notifications.read(Instant::now());
        } else if !long && is_system_panel(point.x) {
            self.toggle_history().await;
        }
    }

    async fn toggle_history(&mut self) {
        let mut config = self.config.clone();
        config.strip.show_history = !config.strip.show_history;
        if let Err(error) = self.apply_config(config).await {
            tracing::warn!(serial = %self.serial(), %error, "history toggle refused");
        }
    }

    async fn handle_event(&mut self, routed: RoutedEvent) {
        let module = routed.module.clone();
        let surfaces = self.event_surfaces(&module);
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

    /// Surfaces showing the outcome of an event of `module`: its home key
    /// when displayed, or else the reminders of its dials.
    fn event_surfaces(&self, module: &ModuleInstanceId) -> Vec<Surface> {
        let home = self.displayed(vec![Self::home_surface(module)]);
        if home.is_empty() {
            self.displayed_dials(module)
        } else {
            home
        }
    }

    /// Surfaces alerting that `module` was replaced by a placeholder: its
    /// home key if it is on the home page, or else the reminders of its dials.
    /// Not limited to the last frame: the replacement happens while the next
    /// one is composed.
    fn failure_surfaces(&self, module: &ModuleInstanceId) -> Vec<Surface> {
        if self.config.home.contains(module) {
            vec![Self::home_surface(module)]
        } else {
            self.dial_surfaces(module)
        }
    }

    fn home_surface(module: &ModuleInstanceId) -> Surface {
        Surface::Key(KeyTarget::Home(module.clone()))
    }

    /// The reminders of the dials assigned a function of `module`.
    fn dial_surfaces(&self, module: &ModuleInstanceId) -> Vec<Surface> {
        if !self.has_strip {
            return Vec::new();
        }
        self.config
            .dials
            .iter()
            .zip(0_u8..)
            .filter(|(assigned, _)| assigned.instance() == Some(module))
            .map(|(_, dial)| Surface::Dial(dial))
            .collect()
    }

    /// The displayed reminders of the dials assigned a function of `module`.
    fn displayed_dials(&self, module: &ModuleInstanceId) -> Vec<Surface> {
        self.displayed(self.dial_surfaces(module))
    }

    /// Surfaces of `module` named by a [`Command::FeedbackOn`], among those
    /// displayed.
    fn named_surfaces(&self, module: &ModuleInstanceId, surface: ModuleSurface) -> Vec<Surface> {
        match surface {
            ModuleSurface::HomeKey => self.displayed(vec![Self::home_surface(module)]),
            ModuleSurface::Dial => self.displayed_dials(module),
            ModuleSurface::Item { page, item } => {
                self.displayed(vec![Surface::Key(KeyTarget::Item {
                    module: module.clone(),
                    page,
                    item,
                })])
            }
            // Surfaces added to the API after this version of the host.
            unknown => {
                tracing::warn!(%module, surface = ?unknown, "unsupported feedback surface");
                Vec::new()
            }
        }
    }

    /// The `surfaces` shown on the last drawn frame: a mark is only ever
    /// drawn over what the user sees when the outcome comes, never later
    /// over a surface shown afterwards.
    fn displayed(&self, mut surfaces: Vec<Surface>) -> Vec<Surface> {
        let Some(frame) = &self.last_frame else {
            return Vec::new();
        };
        surfaces.retain(|surface| match surface {
            Surface::Key(target) => frame
                .keys
                .iter()
                .any(|key| key.target.as_ref() == Some(target)),
            Surface::Dial(dial) => frame
                .strip
                .as_ref()
                .is_some_and(|strip| usize::from(*dial) < strip.hints.len()),
        });
        surfaces
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
                Command::BackTo(page) => self.nav.back_to(source, &page),
                Command::GoHome => self.nav.home(),
                Command::ScrollRows(delta) => self.scroll_rows(delta),
                Command::Feedback(feedback) => self.show_feedback(surfaces, feedback),
                Command::FeedbackOn(surface, feedback) => {
                    let named = self.named_surfaces(source, surface);
                    self.show_feedback(&named, feedback);
                }
                Command::Notify(notification) => {
                    let icon = self.instances.icon(source);
                    self.notifications
                        .notify(source.clone(), icon, &notification, Instant::now());
                }
                Command::ShowNotification(id) => {
                    self.notifications.show_again(id, Instant::now());
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
        // Bounds first: the modules are checked on lists of a known size.
        check_config(&config, self.deck.info())?;
        check_modules(&self.services.registry, &config)?;
        let store = self.services.store.clone();
        let info = self.deck.info().clone();
        let saved = spawn_blocking(move || store.save(&info, config)).await??;
        self.instances.sync(&saved.modules);
        // A hold started under the previous configuration has no meaning.
        self.held_dial = None;
        let refresh_changed =
            saved.strip.system_refresh_seconds != self.config.strip.system_refresh_seconds;
        self.config = saved.clone();
        if refresh_changed && self.has_strip {
            self.schedule_system_sample();
        }
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
                    self.nav.set_current_title(&content.title);
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
        if let Some(scrolls) = self.painter.take_scrolls() {
            self.notifications.set_scrolls(scrolls);
        }
        if self.last_frame.as_ref() == Some(&frame) {
            return Ok(());
        }
        if self
            .last_frame
            .as_ref()
            .is_none_or(|last| last.page != frame.page)
        {
            // Debug-escaped: page ids and titles come from modules (Docker
            // project names chosen by anyone with access to Docker).
            tracing::info!(
                serial = %self.serial(),
                page = ?frame.page.id,
                title = ?frame.page.title,
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
            let surfaces = self.failure_surfaces(&module);
            self.show_feedback(&surfaces, Feedback::Alert);
        }
        loop {
            let page = self.displayed_page();
            self.follow_focus(&page.keys);
            self.clamp_row(page.keys.len());
            let frame = self.compose(page);
            let failed = self.instances.take_failures();
            if failed.is_empty() {
                return frame;
            }
            for module in &failed {
                let surfaces = self.failure_surfaces(module);
                self.show_feedback(&surfaces, Feedback::Alert);
            }
        }
    }

    /// What covers the touch strip: the label of a held key, else the
    /// notification.
    fn strip_overlay(&self) -> Option<StripOverlay> {
        if let Some(label) = self.held.as_ref().and_then(HeldKey::shown_label) {
            return Some(StripOverlay::HeldLabel(label.to_owned()));
        }
        let shown = self.notifications.shown()?;
        Some(StripOverlay::Notification(NotificationFace::new(
            &shown.notification,
            shown.timeout,
            shown.elapsed(Instant::now()),
            self.notification_actions(&shown.notification),
        )))
    }

    /// The reminders of the dials that act under `notification`: the
    /// contextual dial if the notification declares a press, and the
    /// navigation dials, whose press hides it.
    fn notification_actions(&self, notification: &UnreadNotification) -> Vec<NotificationAction> {
        let dials = self.config.dials.len();
        let contextual = notification.context_dial.hint.press.clone();
        self.config
            .dials
            .iter()
            .enumerate()
            .filter_map(|(dial, assignment)| {
                let text = match assignment {
                    DialAssignment::Contextual => contextual.clone()?,
                    DialAssignment::Navigation => HIDE_NOTIFICATION.to_owned(),
                    _ => return None,
                };
                Some(NotificationAction { text, dial, dials })
            })
            .collect()
    }

    /// Scrolls to the key holding the cursor of the contextual dial when it
    /// moved, so that the user always sees it; a page the user scrolls
    /// elsewhere is left alone until the cursor moves again.
    fn follow_focus(&mut self, keys: &[FrameKey]) {
        let focused = keys.iter().position(|key| key.face.focused);
        let target = focused.and_then(|index| keys[index].target.clone());
        if target == self.focus {
            return;
        }
        if let Some(index) = focused {
            let row = self.grid.reveal(self.nav.row(), index, keys.len());
            self.nav.set_row(row);
        }
        self.focus = target;
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
                context: None,
            },
            PageModel::Module {
                module,
                page,
                content,
            } => DisplayedPage {
                id: format!("{module}/{page}"),
                title: content.title,
                context: Some(ContextFace {
                    module: module.clone(),
                    page: page.clone(),
                    dial: content.context_dial,
                }),
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
        let positions = self.grid.max_row(page.keys.len()) + 1;
        let strip = self
            .has_strip
            .then(|| self.strip_face(&page.title, row, positions, page.context.as_ref()));
        Frame {
            strip,
            page: PageSummary {
                id: page.id,
                title: page.title,
                row,
                row_count,
            },
            keys,
            overlay: self.strip_overlay(),
            context: page.context,
            theme: self.config.theme,
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

    /// Title of the page the navigation leads back to: the home page, or the
    /// module page that opened the displayed one, as it was when shown.
    fn parent_title(&self) -> String {
        self.nav
            .parent()
            .map(|parent| parent.title.as_str())
            .filter(|title| !title.is_empty())
            .unwrap_or(HOME_PAGE_TITLE)
            .to_owned()
    }

    /// Destroys the modules and blanks the screens.
    async fn blank(&mut self) {
        self.instances.clear();
        device::blank(self.deck.as_ref(), BLANK_TIMEOUT).await;
    }
}

/// What a dial did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialInput {
    Rotate(i32),
    Press,
}

impl DialInput {
    /// The call of a module handler for a dial assigned `function`.
    fn into_call(self, function: DialFunctionId) -> Call {
        match self {
            Self::Rotate(ticks) => Call::DialRotate(function, ticks),
            Self::Press => Call::DialPress(function),
        }
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
