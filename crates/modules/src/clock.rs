//! Clock module: local time and French date in the touch strip zone, redrawn
//! on every minute (or second) boundary.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bergadeck_module_api::{
    Module, ModuleCapabilities, ModuleContext, ModuleDescriptor, ModuleError, ModuleFactory,
    NavigationView, RedrawHandle, SettingField, SettingKind, ZoneContent, parse_settings,
};
use jiff::civil::DateTime;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::watch;

/// Kind of the clock module.
pub const CLOCK_KIND: &str = "clock";

const SHOW_SECONDS_KEY: &str = "showSeconds";
const SHOW_DATE_KEY: &str = "showDate";

/// French abbreviations of the days of the week, from Monday.
const WEEKDAYS: [&str; 7] = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];

/// French abbreviations of the months, from January.
const MONTHS: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];

const NANOS_PER_SECOND: u64 = 1_000_000_000;
const SECONDS_PER_MINUTE: u64 = 60;

/// Source of the current local time; replaced in tests.
pub trait WallClock: Send + Sync {
    /// Current date and time in the local time zone.
    fn now(&self) -> DateTime;
}

/// Wall clock of the system, in its configured time zone.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl WallClock for SystemClock {
    fn now(&self) -> DateTime {
        jiff::Zoned::now().datetime()
    }
}

/// Creates [`Clock`] instances.
#[derive(Clone)]
pub struct ClockFactory {
    clock: Arc<dyn WallClock>,
}

impl ClockFactory {
    /// Factory of clocks reading the system time.
    #[must_use]
    pub fn system() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    /// Factory of clocks reading `clock`.
    #[must_use]
    pub fn with_clock(clock: Arc<dyn WallClock>) -> Self {
        Self { clock }
    }
}

impl std::fmt::Debug for ClockFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClockFactory").finish_non_exhaustive()
    }
}

impl ModuleFactory for ClockFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        let defaults = ClockSettings::default();
        ModuleDescriptor {
            kind: CLOCK_KIND.into(),
            name: "Horloge".into(),
            description: "Heure et date au-dessus d'une molette.".into(),
            capabilities: ModuleCapabilities {
                key: false,
                dial: true,
                touch_zone: true,
            },
            settings: vec![
                SettingField {
                    key: SHOW_SECONDS_KEY.into(),
                    label: "Afficher les secondes".into(),
                    help: None,
                    kind: SettingKind::Boolean {
                        default: defaults.show_seconds,
                    },
                },
                SettingField {
                    key: SHOW_DATE_KEY.into(),
                    label: "Afficher la date".into(),
                    help: None,
                    kind: SettingKind::Boolean {
                        default: defaults.show_date,
                    },
                },
            ],
        }
    }

    fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
        let settings: ClockSettings = parse_settings(settings)?;
        let (settings_tx, settings_rx) = watch::channel(settings);
        ctx.tasks().spawn(refresh_on_ticks(
            Arc::clone(&self.clock),
            settings_rx,
            ctx.redraw().clone(),
        ));
        Ok(Box::new(Clock {
            clock: Arc::clone(&self.clock),
            settings: settings_tx,
        }))
    }
}

/// Settings of a clock instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ClockSettings {
    show_seconds: bool,
    show_date: bool,
}

impl Default for ClockSettings {
    fn default() -> Self {
        Self {
            show_seconds: false,
            show_date: true,
        }
    }
}

impl ClockSettings {
    fn tick(self) -> Tick {
        if self.show_seconds {
            Tick::Second
        } else {
            Tick::Minute
        }
    }
}

/// Granularity of the displayed time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tick {
    Second,
    Minute,
}

/// Shows the time; the settings live in a channel watched by the refresh task.
pub struct Clock {
    clock: Arc<dyn WallClock>,
    settings: watch::Sender<ClockSettings>,
}

#[async_trait]
impl Module for Clock {
    fn zone(&self, _nav: &NavigationView) -> Option<ZoneContent> {
        let settings = *self.settings.borrow();
        let now = self.clock.now();
        let mut lines = vec![format_time(now, settings.tick())];
        if settings.show_date {
            lines.push(format_date(now));
        }
        Some(ZoneContent::Lines(lines))
    }

    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError> {
        self.settings.send_replace(parse_settings(settings)?);
        Ok(())
    }
}

/// Requests a redraw at every tick boundary until the clock is dropped (the
/// settings sender closes) or its task scope aborts the task.
async fn refresh_on_ticks(
    clock: Arc<dyn WallClock>,
    mut settings: watch::Receiver<ClockSettings>,
    redraw: RedrawHandle,
) {
    loop {
        let tick = settings.borrow_and_update().tick();
        let delay = delay_until_next_tick(clock.now(), tick);
        tokio::select! {
            () = tokio::time::sleep(delay) => redraw.request_redraw(),
            changed = settings.changed() => {
                if changed.is_err() {
                    return;
                }
                // The tick may have changed: redraw now and realign.
                redraw.request_redraw();
            }
        }
    }
}

/// Time left until the next second or minute boundary after `now`.
fn delay_until_next_tick(now: DateTime, tick: Tick) -> Duration {
    let subsec = u64::try_from(now.subsec_nanosecond()).unwrap_or(0);
    let elapsed_nanos = match tick {
        Tick::Second => subsec,
        Tick::Minute => u64::try_from(now.second()).unwrap_or(0) * NANOS_PER_SECOND + subsec,
    };
    let period_nanos = match tick {
        Tick::Second => NANOS_PER_SECOND,
        Tick::Minute => SECONDS_PER_MINUTE * NANOS_PER_SECOND,
    };
    Duration::from_nanos(period_nanos - elapsed_nanos.min(period_nanos - 1))
}

/// `14:05`, or `14:05:09` with seconds.
fn format_time(now: DateTime, tick: Tick) -> String {
    match tick {
        Tick::Second => format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second()),
        Tick::Minute => format!("{:02}:{:02}", now.hour(), now.minute()),
    }
}

/// Abbreviated French date, e.g. `mer. 28 oct.`.
fn format_date(now: DateTime) -> String {
    let weekday = usize::try_from(now.weekday().to_monday_zero_offset())
        .ok()
        .and_then(|index| WEEKDAYS.get(index));
    let month = usize::try_from(now.month() - 1)
        .ok()
        .and_then(|index| MONTHS.get(index));
    match (weekday, month) {
        (Some(weekday), Some(month)) => format!("{weekday} {} {month}", now.day()),
        // Unreachable with a valid civil date; keep a readable fallback.
        _ => now.date().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use bergadeck_module_api::TaskScope;
    use jiff::civil::date;
    use serde_json::json;
    use tokio::time::Instant;

    use super::*;

    /// Wall clock starting at `base` and following Tokio's (paused) time.
    struct TokioClock {
        base: DateTime,
        start: Instant,
    }

    impl WallClock for TokioClock {
        fn now(&self) -> DateTime {
            self.base.checked_add(self.start.elapsed()).unwrap()
        }
    }

    struct Fixture {
        clock: Box<dyn Module>,
        redraws: Arc<AtomicUsize>,
        _scope: TaskScope,
    }

    fn start(base: DateTime, settings: &Value) -> Fixture {
        let factory = ClockFactory::with_clock(Arc::new(TokioClock {
            base,
            start: Instant::now(),
        }));
        let redraws = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&redraws);
        let scope = TaskScope::new();
        let ctx = ModuleContext::new(
            "clock".into(),
            RedrawHandle::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
            scope.spawner(),
        );
        Fixture {
            clock: factory.create(ctx, settings).unwrap(),
            redraws,
            _scope: scope,
        }
    }

    fn lines(clock: &dyn Module) -> Vec<String> {
        let nav = NavigationView {
            page_title: "Accueil".into(),
            page: None,
            row: 0,
            row_count: 1,
            visible_rows: 2,
            depth: 0,
        };
        match clock.zone(&nav) {
            Some(ZoneContent::Lines(lines)) => lines,
            other => panic!("unexpected zone {other:?}"),
        }
    }

    #[test]
    fn dates_are_abbreviated_in_french() {
        assert_eq!(
            format_date(date(2026, 10, 28).at(9, 0, 0, 0)),
            "mer. 28 oct."
        );
        assert_eq!(format_date(date(2026, 2, 1).at(0, 0, 0, 0)), "dim. 1 févr.");
        assert_eq!(format_date(date(2026, 8, 3).at(0, 0, 0, 0)), "lun. 3 août");
    }

    #[test]
    fn time_is_zero_padded() {
        let at = date(2026, 10, 28).at(7, 5, 9, 0);
        assert_eq!(format_time(at, Tick::Minute), "07:05");
        assert_eq!(format_time(at, Tick::Second), "07:05:09");
    }

    #[test]
    fn delays_reach_the_next_boundary() {
        let at = date(2026, 10, 28).at(14, 5, 30, 250_000_000);
        assert_eq!(
            delay_until_next_tick(at, Tick::Minute),
            Duration::from_millis(29_750)
        );
        assert_eq!(
            delay_until_next_tick(at, Tick::Second),
            Duration::from_millis(750)
        );
        let on_boundary = date(2026, 10, 28).at(14, 6, 0, 0);
        assert_eq!(
            delay_until_next_tick(on_boundary, Tick::Minute),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn default_settings_hide_seconds_and_show_date() {
        assert_eq!(
            ClockFactory::system().default_settings(),
            json!({ "showSeconds": false, "showDate": true })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn zone_shows_time_and_date_per_settings() {
        let base = date(2026, 10, 28).at(14, 5, 9, 0);
        let mut fixture = start(base, &json!({}));
        assert_eq!(lines(fixture.clock.as_ref()), ["14:05", "mer. 28 oct."]);
        fixture
            .clock
            .update_settings(&json!({ "showSeconds": true, "showDate": false }))
            .unwrap();
        assert_eq!(lines(fixture.clock.as_ref()), ["14:05:09"]);
        assert!(matches!(
            fixture.clock.update_settings(&json!({ "showDate": "yes" })),
            Err(ModuleError::InvalidSettings(_))
        ));
        assert_eq!(lines(fixture.clock.as_ref()), ["14:05:09"]);
    }

    #[tokio::test(start_paused = true)]
    async fn redraws_are_aligned_on_minutes() {
        let fixture = start(date(2026, 10, 28).at(14, 5, 30, 0), &json!({}));
        let redraws = || fixture.redraws.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(29_900)).await;
        assert_eq!(redraws(), 0);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(redraws(), 1);
        assert_eq!(lines(fixture.clock.as_ref())[0], "14:06");
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(redraws(), 2);
        assert_eq!(lines(fixture.clock.as_ref())[0], "14:07");
    }

    #[tokio::test(start_paused = true)]
    async fn enabling_seconds_switches_to_second_ticks() {
        let mut fixture = start(date(2026, 10, 28).at(14, 5, 30, 0), &json!({}));
        // Let the refresh task start waiting for the next minute.
        tokio::task::yield_now().await;
        fixture
            .clock
            .update_settings(&json!({ "showSeconds": true }))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(3_100)).await;
        // One immediate redraw for the change, then one per second.
        assert_eq!(fixture.redraws.load(Ordering::SeqCst), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn refresh_stops_with_the_instance() {
        let fixture = start(date(2026, 10, 28).at(14, 5, 59, 0), &json!({}));
        let redraws = Arc::clone(&fixture.redraws);
        drop(fixture);
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert_eq!(redraws.load(Ordering::SeqCst), 0);
    }
}
