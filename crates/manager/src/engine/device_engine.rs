//! The task driving one device.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hubergamof_module_api::{
    Command, DialFunctionId, Feedback, ModuleInstanceId, ModuleSurface, PageContent, PageId,
    PageItem, RedrawHandle, ScreenLockState, UnreadNotification,
};
use hubergamof_protocol::{DeviceConfig, DialAssignment, ModuleInstance};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{Notify, broadcast, mpsc, watch};
use tokio::task::spawn_blocking;
use tokio::time::Instant;

use self::lock::LockState;
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

mod compose;
mod gestures;
mod lock;
mod strip;
mod surfaces;

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
    /// Whether the key declared its press harmless on a locked computer.
    usable_when_locked: bool,
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

/// What a dial held pressed acts on.
enum HeldDialKind {
    /// The contextual dial, on the page displayed when it was pressed.
    Context(ContextFace),
    /// The navigation dial: a press goes back, a long press goes home.
    Navigation,
}

/// A dial held pressed that has a long press: its action waits for the hold
/// to last, or for the release.
struct HeldDial {
    dial: u8,
    kind: HeldDialKind,
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
    /// The key grid with the rows this page pins: the first keys stay in
    /// place while the rest scrolls (blank keys included: they fill whole
    /// rows).
    grid: KeyGrid,
    context: Option<ContextFace>,
    keeps_lit: bool,
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
    /// The computer was locked or unlocked.
    LockChanged,
    /// The device has been locked long enough to go dark.
    LockSleep,
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
    /// The lock of the computer: no action, then dark.
    lock: LockState,
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
        let lock = LockState::new(services.screen_lock.watch());

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
            {
                let screen = services.screen_lock.watch();
                ScreenLockState::new(move || *screen.borrow())
            },
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
            lock,
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
        engine.schedule_initial_sleep();
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
                    self.long_press_dial().await;
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
                Wake::LockChanged => {
                    self.on_lock_changed().await;
                    self.refresh_or_log().await;
                }
                Wake::LockSleep => self.fall_asleep().await,
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
            let lock_sleep = self.lock.sleep_at();
            // Nothing is drawn on a dark device: its timers wait for it to wake.
            let lit = !self.lock.is_asleep();
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
                () = sleep_until(self.marks.next_expiry().filter(|_| lit)) => Some(Wake::FeedbackExpired),
                () = sleep_until(self.notifications.next_step().filter(|_| lit)) => Some(Wake::NotificationStep),
                () = sleep_until(self.held.as_ref().and_then(HeldKey::label_due)) => {
                    Some(Wake::HoldElapsed)
                }
                () = sleep_until(self.held_dial.as_ref().map(HeldDial::due)) => {
                    Some(Wake::DialHoldElapsed)
                }
                () = sleep_until(self.strip_settle.filter(|_| lit)) => Some(Wake::StripSettled),
                () = sleep_until(clock_tick.filter(|_| lit)) => Some(Wake::ClockTick),
                () = sleep_until(self.next_system_sample.filter(|_| lit)) => Some(Wake::SystemSample),
                () = self.lock.changed() => Some(Wake::LockChanged),
                () = sleep_until(lock_sleep) => Some(Wake::LockSleep),
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
        if event != InputEvent::Disconnected && self.absorbed_while_locked().await {
            return;
        }
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

    /// Hands the press to what the key shows on the last drawn frame, even
    /// if the module content changed since.
    async fn press_key(&mut self, key: u8) {
        let Some(frame) = &self.last_frame else {
            return;
        };
        let Some(target) = frame.target(key).cloned() else {
            return;
        };
        let usable_when_locked = frame.usable_when_locked(key);
        self.press_target(target, usable_when_locked).await;
    }

    async fn press_target(&mut self, target: KeyTarget, usable_when_locked: bool) {
        let call = match &target {
            KeyTarget::Home(_) => Call::KeyPress,
            KeyTarget::Item { page, item, .. } => Call::ItemPress(page.clone(), item.clone()),
        };
        let module = target.module().clone();
        self.dispatch(
            &module,
            call,
            vec![Surface::Key(target)],
            usable_when_locked,
        )
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
        let declared = shown.notification.context_dial.usable_when_locked;
        let surfaces = self.failure_surfaces(&source);
        let done = self
            .dispatch(&source, Call::NotificationPress(id), surfaces, declared)
            .await;
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
        self.dispatch(
            &context.module,
            call,
            surfaces,
            context.dial.usable_when_locked,
        )
        .await;
    }

    /// The navigation dial: turning scrolls the rows of the displayed page,
    /// pressing goes back one page (holding it goes home, see
    /// `long_press_dial`).
    fn navigate(&mut self, input: DialInput) {
        match input {
            DialInput::Rotate(ticks) => self.scroll_rows(ticks),
            DialInput::Press => self.nav.back(),
        }
    }

    fn scroll_rows(&mut self, delta: i32) {
        let page = self.displayed_page();
        let row = page.grid.scroll(self.nav.row(), delta, page.keys.len());
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
        self.dispatch(module, call, surfaces, false).await;
    }

    /// The one place where a call reaches a module: refused while the
    /// computer is locked unless the module is usable then, or the page
    /// (`declared`) declares it and the module may declare that. Tells
    /// whether the handler completed.
    async fn dispatch(
        &mut self,
        module: &ModuleInstanceId,
        call: Call,
        surfaces: Vec<Surface>,
        declared: bool,
    ) -> bool {
        if !self.may_press(module, declared) {
            return false;
        }
        self.trace_declared_while_locked(module, declared);
        let Some(outcome) = self.instances.call(module, call).await else {
            return false;
        };
        let done = matches!(outcome, Outcome::Done(_));
        self.conclude(module, &surfaces, outcome);
        done
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
        // A dark device gets its brightness back when it wakes up.
        if !self.lock.is_asleep() && self.applied_brightness != Some(saved.brightness) {
            self.applied_brightness = apply_brightness(self.deck.as_ref(), saved.brightness).await;
        }
        self.lock_config_changed().await;
        Ok(saved)
    }

    async fn refresh_or_log(&mut self) {
        if let Err(error) = self.refresh().await {
            tracing::warn!(serial = %self.serial(), %error, "cannot redraw");
        }
    }

    /// Draws what changed and publishes the live state if anything did.
    async fn refresh(&mut self) -> Result<(), EngineError> {
        self.close_refused_page();
        if self.lock.is_asleep() && !self.page_now_keeps_lit().await {
            return Ok(());
        }
        let frame = self.next_frame();
        self.page_keeps_lit_changed(frame.keeps_lit);
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
            self.follow_focus(&page.keys, page.grid);
            self.clamp_row(page.keys.len(), page.grid);
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
