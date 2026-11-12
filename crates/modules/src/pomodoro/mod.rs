//! Pomodoro module: work sessions and breaks on a key, a dial function and
//! the pomodoro widget of the touch strip.
//!
//! - Key press: starts or pauses the time.
//! - Press of a dial assigned [`CONTROL_FUNCTION`]: opens a menu (pause or
//!   resume, skip to the next step, reset), shown as a popup over the strip,
//!   browsed by rotating, confirmed by a press, closed after
//!   [`MENU_TIMEOUT`] without input.
//! - When a step ends, the next one is prepared stopped (unless `autoStart`)
//!   and the key asks for attention; a silent desktop notification and a
//!   notification over the touch strip (settings `notificationEnabled`,
//!   `notificationSeconds`, 0 = until dismissed) say so.
//!
//! The time follows real time, suspends included (see [`crate::time`]): a
//! step whose end passed while the machine slept is over on waking up. The
//! state lives in memory only: a restart begins a new cycle.

mod menu;
mod settings;
mod timer;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hubergamof_module_api::{
    Command, ContextDial, DialFunctionDescriptor, DialFunctionId, DialHint, EventSender, ImageData,
    KeyContent, KeyState, Module, ModuleCapabilities, ModuleContext, ModuleDescriptor, ModuleError,
    ModuleEvent, ModuleFactory, Notification as StripNotification, NotificationId, PomodoroState,
    PomodoroWidget, PopupEntry, PopupIcon, Rgb, StripPopup, StripWidget,
};
use serde_json::Value;
use tokio::sync::oneshot;

pub use menu::MENU_TIMEOUT;
pub use settings::{PomodoroSettings, StripNotice};

use crate::icons::{asset, svg};
use crate::notifications::{Notification, Notifier};
use crate::time::{Clock, ClockInstant};
use menu::{MENU_ENTRIES, Menu, MenuEntry};
use timer::{Phase, StepEnd, Timer};

/// Kind of the pomodoro module.
pub const POMODORO_KIND: &str = "pomodoro";

/// Dial function: pressing opens the menu of the pomodoro, rotating browses
/// it and pressing again runs the selected entry.
pub const CONTROL_FUNCTION: &str = "control";

/// Icon of the module: a timer.
const ICON_SVG: &str = asset!("pomodoro");

/// Period of the refresh ticks, while the time runs or the menu is open.
pub(crate) const TICK_PERIOD: Duration = Duration::from_secs(1);

const SECONDS_PER_MINUTE: u64 = 60;

/// Name of a work step on the touch strip.
const WORK_NAME: &str = "Focus";
const READY_SUFFIX: &str = "prêt";
const PAUSED_SUFFIX: &str = "pause";

const MENU_HINT: &str = "Menu pomodoro";
const CHOOSE_HINT: &str = "Choisir";
const VALIDATE_HINT: &str = "Valider";

/// Creates [`Pomodoro`] instances.
#[derive(Clone)]
pub struct PomodoroFactory {
    notifier: Arc<dyn Notifier>,
    clock: Arc<dyn Clock>,
}

impl PomodoroFactory {
    /// Factory of pomodoros notifying through `notifier` and following
    /// `clock` (which must count the time the machine sleeps, see
    /// [`crate::time`]).
    #[must_use]
    pub fn new(notifier: Arc<dyn Notifier>, clock: Arc<dyn Clock>) -> Self {
        Self { notifier, clock }
    }
}

impl std::fmt::Debug for PomodoroFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PomodoroFactory")
            .field("notifier", &self.notifier)
            .field("clock", &self.clock)
            .finish()
    }
}

/// Kind, capabilities and settings of the pomodoro.
fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        kind: POMODORO_KIND.into(),
        name: "Pomodoro".into(),
        description: "Sessions de travail et pauses, avec un minuteur affiché sur la bande \
                      tactile. Se pilote depuis une molette."
            .into(),
        capabilities: ModuleCapabilities::default(),
        dial_functions: vec![DialFunctionDescriptor {
            id: CONTROL_FUNCTION.into(),
            name: "Contrôle du pomodoro".into(),
            description: "Appuyer ouvre le menu (pause, passer, remise à zéro), tourner \
                          choisit, appuyer valide."
                .into(),
        }],
        settings: settings::fields(),
    }
}

impl PomodoroSettings {
    /// Checks settings against the catalog fields and parses them (missing keys
    /// take their defaults).
    ///
    /// # Errors
    ///
    /// [`ModuleError::InvalidSettings`] when they do not parse or are out of
    /// bounds.
    pub fn parse(settings: &Value) -> Result<Self, ModuleError> {
        descriptor().parse_settings(settings)
    }
}

impl ModuleFactory for PomodoroFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor()
    }

    fn icon(&self) -> Option<ImageData> {
        Some(svg(ICON_SVG))
    }

    fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
        let settings = PomodoroSettings::parse(settings)?;
        Ok(Box::new(Pomodoro {
            timer: Timer::new(settings.durations(), settings.auto_start),
            menu: None,
            ticker: None,
            notifier: Arc::clone(&self.notifier),
            clock: Arc::clone(&self.clock),
            strip_notice: settings.strip_notice(),
            ctx,
        }))
    }
}

/// Sent by the ticker task.
#[derive(Debug, Clone, Copy)]
struct Tick;

/// The running ticker task, stopped when dropped.
#[derive(Debug)]
struct Ticker {
    /// Deadline the ticks are aligned on (`None`: every [`TICK_PERIOD`]).
    deadline: Option<ClockInstant>,
    _stop: oneshot::Sender<()>,
}

/// A pomodoro instance.
pub struct Pomodoro {
    timer: Timer,
    menu: Option<Menu>,
    ticker: Option<Ticker>,
    notifier: Arc<dyn Notifier>,
    clock: Arc<dyn Clock>,
    strip_notice: StripNotice,
    ctx: ModuleContext,
}

impl Pomodoro {
    fn now(&self) -> ClockInstant {
        self.clock.now()
    }

    fn toggle(&mut self) {
        self.timer.toggle(self.now());
        self.sync_ticker();
    }

    fn run_menu_entry(&mut self, entry: MenuEntry) {
        match entry {
            MenuEntry::Toggle => self.timer.toggle(self.now()),
            MenuEntry::Skip => self.timer.skip(self.now()),
            MenuEntry::Reset => self.timer.reset(),
        }
        self.menu = None;
        self.sync_ticker();
    }

    /// Handles a tick; returns the notification for the touch strip when a
    /// step just ended.
    fn on_tick(&mut self) -> Vec<Command> {
        let now = self.now();
        if self.menu.is_some_and(|menu| menu.is_expired(now)) {
            self.menu = None;
        }
        let mut commands = Vec::new();
        if let Some(end) = self.timer.tick(now) {
            let notification = notification_for(&end, self.timer.cycle_length());
            if self.strip_notice.enabled {
                commands.push(Command::Notify(strip_notification(
                    &notification,
                    &end,
                    self.strip_notice.timeout,
                )));
            }
            self.notify(notification);
        }
        self.sync_ticker();
        commands
    }

    /// Runs the ticker only while the time runs or the menu is open, aligned
    /// on the current deadline: a new deadline (start, skip, new lengths…)
    /// replaces the ticker.
    fn sync_ticker(&mut self) {
        let deadline = self.timer.deadline();
        if deadline.is_none() && self.menu.is_none() {
            self.ticker = None;
            return;
        }
        if self
            .ticker
            .as_ref()
            .is_some_and(|ticker| ticker.deadline == deadline)
        {
            return;
        }
        let (stop, stopped) = oneshot::channel();
        let ticks = tick_until_stopped(
            self.ctx.events().clone(),
            Arc::clone(&self.clock),
            deadline,
            stopped,
        );
        self.ticker = self.ctx.tasks().spawn(ticks).then_some(Ticker {
            deadline,
            _stop: stop,
        });
    }

    /// Shows `notification` from a background task; a failure is logged.
    fn notify(&self, notification: Notification) {
        let notifier = Arc::clone(&self.notifier);
        let instance = self.ctx.instance_id().0.clone();
        let spawned = self.ctx.tasks().spawn(async move {
            if let Err(error) = notifier.notify(&notification).await {
                tracing::warn!(%error, %instance, "pomodoro notification failed");
            }
        });
        if !spawned {
            tracing::debug!(instance = %self.ctx.instance_id().0, "pomodoro notification dropped");
        }
    }

    /// Whether none of the current step has elapsed.
    fn is_untouched(&self) -> bool {
        self.timer.elapsed(self.now()).is_zero()
    }

    /// The widget of the touch strip: step, time left and progress.
    fn widget(&self) -> PomodoroWidget {
        let timer = &self.timer;
        let remaining = timer.remaining(self.now());
        let name = step_name(timer);
        let (state, label) = if timer.is_running() {
            let state = if timer.phase().is_break() {
                PomodoroState::Resting
            } else {
                PomodoroState::Working
            };
            (state, name)
        } else if self.is_untouched() {
            (PomodoroState::Ready, format!("{name} · {READY_SUFFIX}"))
        } else {
            (PomodoroState::Paused, format!("{name} · {PAUSED_SUFFIX}"))
        };
        PomodoroWidget::new(
            state,
            label,
            format_remaining(remaining),
            displayed_progress(timer.step_duration(), remaining),
        )
    }

    /// The popup of the open menu: one entry per [`MENU_ENTRIES`].
    fn popup(&self, menu: Menu) -> StripPopup {
        let entries = MENU_ENTRIES
            .iter()
            .map(|&entry| {
                PopupEntry::new(
                    entry_icon(entry, &self.timer),
                    entry_label(entry, &self.timer, self.is_untouched()),
                )
            })
            .collect();
        StripPopup::new(CONTROL_FUNCTION.into(), entries, menu.selected_index())
    }
}

#[async_trait]
impl Module for Pomodoro {
    fn key(&self) -> KeyContent {
        let label = format!(
            "{} {}",
            format_remaining(self.timer.remaining(self.now())),
            phase_name(self.timer.phase())
        );
        let state = if self.timer.is_awaiting_start() {
            KeyState::Attention
        } else if self.timer.is_running() {
            KeyState::Active
        } else {
            KeyState::Normal
        };
        KeyContent::with_label(label).in_state(state)
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        self.toggle();
        Vec::new()
    }

    /// Starts the step the notification announced, unless it was started in
    /// the meantime (key, menu): the notification is then only read.
    async fn on_notification_press(&mut self, _id: NotificationId) -> Vec<Command> {
        if self.timer.is_awaiting_start() {
            self.toggle();
        }
        Vec::new()
    }

    async fn on_dial_rotate(&mut self, function: &DialFunctionId, ticks: i32) -> Vec<Command> {
        let now = self.now();
        if let (CONTROL_FUNCTION, Some(menu)) = (function.0.as_str(), &mut self.menu) {
            menu.rotate(ticks, now);
        }
        Vec::new()
    }

    async fn on_dial_press(&mut self, function: &DialFunctionId) -> Vec<Command> {
        if function.0 != CONTROL_FUNCTION {
            return Vec::new();
        }
        if let Some(menu) = self.menu {
            self.run_menu_entry(menu.selected());
        } else {
            self.menu = Some(Menu::open(self.now()));
            self.sync_ticker();
        }
        Vec::new()
    }

    async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
        if event.is::<Tick>() {
            return self.on_tick();
        }
        Vec::new()
    }

    fn dial_hint(&self, function: &DialFunctionId) -> DialHint {
        if function.0 != CONTROL_FUNCTION {
            return DialHint::none();
        }
        if self.menu.is_some() {
            DialHint::none()
                .rotating(CHOOSE_HINT)
                .pressing(VALIDATE_HINT)
        } else {
            DialHint::none().pressing(MENU_HINT)
        }
    }

    fn strip_widget(&self) -> Option<StripWidget> {
        Some(StripWidget::Pomodoro(self.widget()))
    }

    fn strip_popup(&self) -> Option<StripPopup> {
        self.menu.map(|menu| self.popup(menu))
    }

    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError> {
        let settings = PomodoroSettings::parse(settings)?;
        self.strip_notice = settings.strip_notice();
        self.timer
            .update(settings.durations(), settings.auto_start, self.now());
        self.sync_ticker();
        Ok(())
    }
}

/// Sends a [`Tick`] whenever the time left reaches a whole second (the
/// display changes) and at `deadline`, or every [`TICK_PERIOD`] without a
/// deadline, until `stop` fires or is dropped, or the instance is gone.
///
/// Every delay is computed from `clock` again: after a suspend, the first
/// tick comes at most a second after waking up (Tokio's timers do not count
/// the time asleep), finds the step over if its deadline passed, and the
/// next ticks fall on whole seconds again. Missed ticks are not replayed.
async fn tick_until_stopped(
    events: EventSender,
    clock: Arc<dyn Clock>,
    deadline: Option<ClockInstant>,
    mut stop: oneshot::Receiver<()>,
) {
    // Whether the last tick was sent with the deadline passed: that tick
    // ends the step and replaces this ticker; until it is handled, do not
    // spin.
    let mut ticked_at_deadline = false;
    loop {
        let delay = match deadline {
            Some(_) if ticked_at_deadline => TICK_PERIOD,
            Some(deadline) => {
                delay_to_whole_second(deadline.saturating_duration_since(clock.now()))
            }
            None => TICK_PERIOD,
        };
        tokio::select! {
            _ = &mut stop => return,
            () = tokio::time::sleep(delay) => {
                if !events.send(Tick) {
                    return;
                }
            }
        }
        ticked_at_deadline = deadline
            .is_some_and(|deadline| deadline.saturating_duration_since(clock.now()).is_zero());
    }
}

/// Delay until the time left `remaining` reaches a whole second (the next
/// display change); zero when the time is up.
fn delay_to_whole_second(remaining: Duration) -> Duration {
    let fraction = Duration::from_nanos(u64::from(remaining.subsec_nanos()));
    if fraction.is_zero() && !remaining.is_zero() {
        TICK_PERIOD
    } else {
        fraction
    }
}

/// `mm:ss`, rounded up to the second so that a step shows its full length
/// when it starts and `00:00` only when it is over.
fn format_remaining(remaining: Duration) -> String {
    let seconds = displayed_seconds(remaining);
    format!(
        "{:02}:{:02}",
        seconds / SECONDS_PER_MINUTE,
        seconds % SECONDS_PER_MINUTE
    )
}

/// Seconds shown for `remaining`: rounded up (see [`format_remaining`]).
fn displayed_seconds(remaining: Duration) -> u64 {
    remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
}

/// Progress matching the time shown, in thousandths of the step: it moves
/// when the displayed second changes and is identical in between, so that the
/// strip is drawn once per second.
fn displayed_progress(step: Duration, remaining: Duration) -> u16 {
    let shown_remaining = Duration::from_secs(displayed_seconds(remaining));
    let done = millis(step.saturating_sub(shown_remaining));
    let total = millis(step).max(1);
    let permille = done.saturating_mul(u64::from(PomodoroWidget::FULL_PROGRESS)) / total;
    u16::try_from(permille).unwrap_or(PomodoroWidget::FULL_PROGRESS)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Short name of a step, on the key.
fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Work => "Travail",
        Phase::ShortBreak => "Pause",
        Phase::LongBreak => "Pause longue",
    }
}

/// Name of the step on the touch strip: `Focus 2/4` for work, the name of
/// the break otherwise.
fn step_name(timer: &Timer) -> String {
    match timer.phase() {
        Phase::Work => format!("{WORK_NAME} {}/{}", timer.pomodoro(), timer.cycle_length()),
        Phase::ShortBreak | Phase::LongBreak => phase_name(timer.phase()).to_owned(),
    }
}

/// Label of `entry`; `untouched` tells that none of the step has elapsed.
fn entry_label(entry: MenuEntry, timer: &Timer, untouched: bool) -> &'static str {
    match entry {
        MenuEntry::Toggle if timer.is_running() => "Pause",
        MenuEntry::Toggle if untouched => "Démarrer",
        MenuEntry::Toggle => "Reprendre",
        MenuEntry::Skip if timer.phase().is_break() => "Passer au travail",
        MenuEntry::Skip => "Passer à la pause",
        MenuEntry::Reset => "Remise à zéro",
    }
}

/// Icon of `entry`: the action it runs given the timer.
fn entry_icon(entry: MenuEntry, timer: &Timer) -> PopupIcon {
    match entry {
        MenuEntry::Toggle if timer.is_running() => PopupIcon::Pause,
        MenuEntry::Toggle => PopupIcon::Play,
        MenuEntry::Skip => PopupIcon::Skip,
        MenuEntry::Reset => PopupIcon::Reset,
    }
}

/// Background of the strip notification when work starts (red, like the icon).
const WORK_BACKGROUND: Rgb = Rgb(0xF0, 0x3E, 0x2F);
/// Background of the strip notification when a break starts.
const BREAK_BACKGROUND: Rgb = Rgb(0x16, 0xA0, 0x6E);

/// Reminder of the contextual dial under a notification that ended a step
/// when a break is next.
const START_BREAK: &str = "Démarrer la pause";
/// Same, when work is next.
const START_WORK: &str = "Démarrer le travail";

/// The touch strip notification for the desktop `notification`: its title
/// and text on one line, red when work comes next and green for a break.
/// Unless the next step started by itself (`autoStart`), pressing the
/// contextual dial starts it.
fn strip_notification(
    notification: &Notification,
    end: &StepEnd,
    timeout: Option<Duration>,
) -> StripNotification {
    let next = end.next;
    let background = if next.is_break() {
        BREAK_BACKGROUND
    } else {
        WORK_BACKGROUND
    };
    let message = format!("{} {}", notification.summary, notification.body);
    let mut notification = StripNotification::new(message, background);
    if !end.started {
        let text = if next.is_break() {
            START_BREAK
        } else {
            START_WORK
        };
        notification = notification.with_context_dial(ContextDial::none().pressing(text));
    }
    match timeout {
        Some(timeout) => notification.with_timeout(timeout),
        None => notification,
    }
}

/// Notification of the end of a step.
fn notification_for(end: &StepEnd, cycle_length: u32) -> Notification {
    let summary = match end.next {
        Phase::Work => "Au travail !",
        Phase::ShortBreak => "Pause !",
        Phase::LongBreak => "Pause longue !",
    };
    let finished = if end.finished.is_break() {
        "Pause terminée.".to_owned()
    } else {
        format!("Pomodoro {}/{cycle_length} terminé.", end.pomodoro)
    };
    let minutes = end.next_duration.as_secs() / SECONDS_PER_MINUTE;
    let next = match end.next {
        Phase::Work => format!("pomodoro {}/{cycle_length}", end.pomodoro),
        Phase::ShortBreak => format!("pause de {minutes} min"),
        Phase::LongBreak => format!("pause longue de {minutes} min"),
    };
    let (article, capitalized) = if end.next.is_break() {
        ("la", "La")
    } else {
        ("le", "Le")
    };
    let body = if end.started {
        format!("{finished} {capitalized} {next} commence.")
    } else {
        format!("{finished} Appuyez pour lancer {article} {next}.")
    };
    Notification {
        summary: summary.into(),
        body,
    }
}

#[cfg(test)]
mod tests;
