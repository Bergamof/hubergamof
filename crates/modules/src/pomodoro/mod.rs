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
//! The state lives in memory only: a restart begins a new cycle.

mod menu;
mod settings;
mod timer;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bergadeck_module_api::{
    Command, EventSender, Feedback, GaugeContent, GaugeLevel, KeyContent, KeyState, Module,
    ModuleCapabilities, ModuleContext, ModuleDescriptor, ModuleError, ModuleEvent, ModuleFactory,
    NavigationView, ZoneContent,
};
use serde_json::Value;
use tokio::sync::oneshot;
use tokio::time::{Instant, MissedTickBehavior};

pub use menu::MENU_TIMEOUT;
pub use settings::PomodoroSettings;

use crate::notifications::{DesktopNotifier, Notification, Notifier};
use menu::{MENU_ENTRIES, Menu, MenuEntry};
use timer::{Phase, StepEnd, Timer};

/// Kind of the pomodoro module.
pub const POMODORO_KIND: &str = "pomodoro";

/// Period of the refresh ticks, while the time runs or the menu is open.
pub const TICK_PERIOD: Duration = Duration::from_secs(1);

/// Marker in front of the selected menu entry.
const SELECTION_MARKER: &str = "›";

const SECONDS_PER_MINUTE: u64 = 60;

/// Creates [`Pomodoro`] instances.
#[derive(Clone)]
pub struct PomodoroFactory {
    notifier: Arc<dyn Notifier>,
}

impl PomodoroFactory {
    /// Factory of pomodoros notifying the desktop session.
    #[must_use]
    pub fn desktop() -> Self {
        Self::with_notifier(Arc::new(DesktopNotifier::session()))
    }

    /// Factory of pomodoros notifying through `notifier`.
    #[must_use]
    pub fn with_notifier(notifier: Arc<dyn Notifier>) -> Self {
        Self { notifier }
    }
}

impl std::fmt::Debug for PomodoroFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PomodoroFactory").finish_non_exhaustive()
    }
}

impl ModuleFactory for PomodoroFactory {
    fn descriptor(&self) -> ModuleDescriptor {
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

    fn validate_settings(&self, settings: &Value) -> Result<(), ModuleError> {
        self.descriptor().validate_settings(settings)?;
        PomodoroSettings::parse(settings).map(|_| ())
    }

    fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
        let settings = PomodoroSettings::parse(settings)?;
        Ok(Box::new(Pomodoro {
            timer: Timer::new(settings.durations(), settings.auto_start),
            menu: None,
            ticker: None,
            notifier: Arc::clone(&self.notifier),
            ctx,
        }))
    }
}

/// Sent by the ticker task every [`TICK_PERIOD`].
#[derive(Debug, Clone, Copy)]
struct Tick;

/// Stops the ticker task when dropped.
#[derive(Debug)]
struct TickerGuard {
    _stop: oneshot::Sender<()>,
}

/// A pomodoro instance.
pub struct Pomodoro {
    timer: Timer,
    menu: Option<Menu>,
    ticker: Option<TickerGuard>,
    notifier: Arc<dyn Notifier>,
    ctx: ModuleContext,
}

impl Pomodoro {
    fn toggle(&mut self, now: Instant) {
        self.timer.toggle(now);
        self.sync_ticker(now, true);
    }

    fn run_menu_entry(&mut self, entry: MenuEntry, now: Instant) {
        match entry {
            MenuEntry::Skip => self.timer.skip(now),
            MenuEntry::Reset => self.timer.reset(),
        }
        self.menu = None;
        self.sync_ticker(now, true);
    }

    fn on_tick(&mut self, now: Instant) {
        if self.menu.is_some_and(|menu| menu.is_expired(now)) {
            self.menu = None;
        }
        let ended = self.timer.tick(now);
        if let Some(end) = ended {
            self.notify(notification_for(&end, self.timer.cycle_length()));
        }
        self.sync_ticker(now, ended.is_some());
    }

    /// Runs the ticker only while the time runs or the menu is open. With
    /// `realign` (the deadline moved), a running timer gets a new ticker
    /// whose ticks fall on whole seconds of the time left, so that the
    /// display changes on time and the last tick hits the deadline.
    fn sync_ticker(&mut self, now: Instant, realign: bool) {
        let running = self.timer.is_running();
        if !running && self.menu.is_none() {
            self.ticker = None;
            return;
        }
        if self.ticker.is_some() && !(realign && running) {
            return;
        }
        let first_tick = if running {
            delay_to_whole_second(self.timer.remaining(now))
        } else {
            TICK_PERIOD
        };
        let (stop, stopped) = oneshot::channel();
        let ticker = tick_until_stopped(self.ctx.events().clone(), now + first_tick, stopped);
        self.ticker = self
            .ctx
            .tasks()
            .spawn(ticker)
            .then_some(TickerGuard { _stop: stop });
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
        let now = Instant::now();
        let label = format!(
            "{} {}",
            format_remaining(self.timer.remaining(now)),
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
        self.toggle(Instant::now());
        Vec::new()
    }

    async fn on_dial_rotate(&mut self, ticks: i32) -> Vec<Command> {
        let now = Instant::now();
        match &mut self.menu {
            Some(menu) => menu.rotate(ticks, now),
            None => self.menu = Some(Menu::open(now)),
        }
        self.sync_ticker(now, false);
        Vec::new()
    }

    async fn on_dial_press(&mut self) -> Vec<Command> {
        let now = Instant::now();
        if let Some(menu) = self.menu {
            self.run_menu_entry(menu.selected(), now);
            vec![Command::Feedback(Feedback::Ok)]
        } else {
            self.toggle(now);
            Vec::new()
        }
    }

    async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
        if event.is::<Tick>() {
            self.on_tick(Instant::now());
        }
        Vec::new()
    }

    fn zone(&self, _nav: &NavigationView) -> Option<ZoneContent> {
        if let Some(menu) = self.menu {
            return Some(self.menu_zone(menu));
        }
        let now = Instant::now();
        let level = GaugeLevel::from_fraction(
            millis(self.timer.elapsed(now)),
            millis(self.timer.step_duration()),
        );
        let gauge = GaugeContent::new(
            step_title(&self.timer),
            format_remaining(self.timer.remaining(now)),
            level,
        )
        .with_dimmed(!self.timer.is_running());
        Some(ZoneContent::Gauge(gauge))
    }

    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError> {
        let settings = PomodoroSettings::parse(settings)?;
        let now = Instant::now();
        self.timer
            .update(settings.durations(), settings.auto_start, now);
        self.sync_ticker(now, true);
        Ok(())
    }
}

/// Sends a [`Tick`] at `first` then every [`TICK_PERIOD`], until `stop`
/// fires or is dropped, or the instance is gone.
async fn tick_until_stopped(events: EventSender, first: Instant, mut stop: oneshot::Receiver<()>) {
    let mut ticks = tokio::time::interval_at(first, TICK_PERIOD);
    // After a suspend, one tick is enough (the state comes from the clock),
    // and the next ones stay on whole seconds of the time left.
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = &mut stop => return,
            _ = ticks.tick() => {
                if !events.send(Tick) {
                    return;
                }
            }
        }
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
    let seconds = remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0);
    format!(
        "{:02}:{:02}",
        seconds / SECONDS_PER_MINUTE,
        seconds % SECONDS_PER_MINUTE
    )
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
