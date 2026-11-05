//! Pomodoro module: work sessions and breaks on a key, a dial and its touch
//! strip zone.
//!
//! - Key or dial press: starts or pauses the time (dial: when the menu is
//!   closed).
//! - Dial rotation: opens a menu (skip to the next step, reset), browsed by
//!   rotating, confirmed by a press, closed after [`MENU_TIMEOUT`] without
//!   input.
//! - When a step ends, the next one is prepared stopped (unless `autoStart`)
//!   and the key asks for attention; a silent desktop notification says so.
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
use bergadeck_module_api::{
    Command, EventSender, Feedback, GaugeContent, GaugeLevel, ImageData, KeyContent, KeyState,
    Module, ModuleCapabilities, ModuleContext, ModuleDescriptor, ModuleError, ModuleEvent,
    ModuleFactory, NavigationView, ZoneContent,
};
use serde_json::Value;
use tokio::sync::oneshot;

pub use menu::MENU_TIMEOUT;
pub use settings::PomodoroSettings;

use crate::icons::svg;
use crate::notifications::{Notification, Notifier};
use crate::time::{Clock, ClockInstant};
use menu::{MENU_ENTRIES, Menu, MenuEntry};
use timer::{Phase, StepEnd, Timer};

/// Kind of the pomodoro module.
pub const POMODORO_KIND: &str = "pomodoro";

/// Icon of the module: a timer.
const ICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="13.5" r="7.5"/><path d="M12 9.5v4l2.5 2M9.5 2.5h5M12 2.5V6"/></svg>"##;

/// Period of the refresh ticks, while the time runs or the menu is open.
pub(crate) const TICK_PERIOD: Duration = Duration::from_secs(1);

/// Marker in front of the selected menu entry.
const SELECTION_MARKER: &str = "›";

const SECONDS_PER_MINUTE: u64 = 60;

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
        description: "Sessions de travail et pauses : appuyer lance ou met en pause, \
                      tourner la molette ouvre le menu."
            .into(),
        capabilities: ModuleCapabilities {
            key: true,
            dial: true,
            touch_zone: true,
        },
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
            MenuEntry::Skip => self.timer.skip(self.now()),
            MenuEntry::Reset => self.timer.reset(),
        }
        self.menu = None;
        self.sync_ticker();
    }

    fn on_tick(&mut self) {
        let now = self.now();
        if self.menu.is_some_and(|menu| menu.is_expired(now)) {
            self.menu = None;
        }
        if let Some(end) = self.timer.tick(now) {
            self.notify(notification_for(&end, self.timer.cycle_length()));
        }
        self.sync_ticker();
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

    fn menu_zone(&self, menu: Menu) -> ZoneContent {
        let mut lines = vec![step_title(&self.timer)];
        lines.extend(MENU_ENTRIES.iter().map(|&entry| {
            let label = entry_label(entry, self.timer.phase());
            if entry == menu.selected() {
                format!("{SELECTION_MARKER} {label}")
            } else {
                label.to_owned()
            }
        }));
        ZoneContent::Lines(lines)
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

    async fn on_dial_rotate(&mut self, ticks: i32) -> Vec<Command> {
        let now = self.now();
        match &mut self.menu {
            Some(menu) => menu.rotate(ticks, now),
            None => self.menu = Some(Menu::open(now)),
        }
        self.sync_ticker();
        Vec::new()
    }

    async fn on_dial_press(&mut self) -> Vec<Command> {
        if let Some(menu) = self.menu {
            self.run_menu_entry(menu.selected());
            vec![Command::Feedback(Feedback::Ok)]
        } else {
            self.toggle();
            Vec::new()
        }
    }

    async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
        if event.is::<Tick>() {
            self.on_tick();
        }
        Vec::new()
    }

    fn zone(&self, _nav: &NavigationView) -> Option<ZoneContent> {
        if let Some(menu) = self.menu {
            return Some(self.menu_zone(menu));
        }
        let remaining = self.timer.remaining(self.now());
        let gauge = GaugeContent::new(
            step_title(&self.timer),
            format_remaining(remaining),
            displayed_level(self.timer.step_duration(), remaining),
        )
        .with_dimmed(!self.timer.is_running());
        Some(ZoneContent::Gauge(gauge))
    }

    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError> {
        let settings = PomodoroSettings::parse(settings)?;
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

/// Gauge level matching the time shown: the bar moves when the displayed
/// second changes and is identical in between, so that the zone image is
/// rendered once per second.
fn displayed_level(step: Duration, remaining: Duration) -> GaugeLevel {
    let shown_remaining = Duration::from_secs(displayed_seconds(remaining));
    GaugeLevel::from_fraction(millis(step.saturating_sub(shown_remaining)), millis(step))
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

/// Title of the zone: the step, with the pomodoro number while working.
fn step_title(timer: &Timer) -> String {
    match timer.phase() {
        Phase::Work => format!(
            "{} · {}/{}",
            phase_name(Phase::Work),
            timer.pomodoro(),
            timer.cycle_length()
        ),
        phase => phase_name(phase).to_owned(),
    }
}

fn entry_label(entry: MenuEntry, phase: Phase) -> &'static str {
    match entry {
        MenuEntry::Skip if phase.is_break() => "Passer au travail",
        MenuEntry::Skip => "Passer à la pause",
        MenuEntry::Reset => "Remise à zéro",
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
