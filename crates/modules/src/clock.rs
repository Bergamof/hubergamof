//! Clock module: local time, French date and ISO week in the touch strip zone
//! as a clock face (calendar leaf, time, seconds), redrawn on every minute (or
//! second) boundary.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hubergamof_module_api::{
    CalendarLeaf, ClockFace, ImageData, Module, ModuleCapabilities, ModuleContext,
    ModuleDescriptor, ModuleError, ModuleFactory, NavigationView, RedrawHandle, SettingField,
    SettingKind, ZoneContent, parse_settings,
};
use jiff::civil::DateTime;
use serde::Deserialize;
use serde_json::Value;

use crate::icons::{asset, svg};
use tokio::sync::watch;

/// Kind of the clock module.
pub const CLOCK_KIND: &str = "clock";

/// Icon of the module: a clock face.
const ICON_SVG: &str = asset!("clock");

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

    fn icon(&self) -> Option<ImageData> {
        Some(svg(ICON_SVG))
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
        Some(ZoneContent::Clock(clock_face(self.clock.now(), settings)))
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

/// The face shown at `now`: the time, then per `settings` the seconds, the
/// calendar leaf and the ISO week.
fn clock_face(now: DateTime, settings: ClockSettings) -> ClockFace {
    let mut face = ClockFace::new(format_time(now));
    if settings.show_seconds {
        face = face.with_seconds(u8::try_from(now.second()).unwrap_or(0));
    }
    if settings.show_date {
        face = face
            .with_leaf(calendar_leaf(now))
            .with_caption(format!("semaine {}", now.date().iso_week_date().week()));
    }
    face
}

/// `14:05`.
fn format_time(now: DateTime) -> String {
    format!("{:02}:{:02}", now.hour(), now.minute())
}

/// Leaf of `now`: abbreviated weekday and month (`mer.`, `nov.`).
fn calendar_leaf(now: DateTime) -> CalendarLeaf {
    let weekday = usize::try_from(now.weekday().to_monday_zero_offset())
        .ok()
        .and_then(|index| WEEKDAYS.get(index));
    let month = usize::try_from(now.month() - 1)
        .ok()
        .and_then(|index| MONTHS.get(index));
    // Unreachable with a valid civil date: empty texts rather than a panic.
    let weekday = weekday.copied().unwrap_or_default();
    let month = month.copied().unwrap_or_default();
    CalendarLeaf::new(weekday, u8::try_from(now.day()).unwrap_or(1), month)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use hubergamof_module_api::{ClockSeconds, TaskScope};
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
        let ctx = ModuleContext::for_tests("clock")
            .with_redraw(RedrawHandle::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }))
            .with_tasks(scope.spawner());
        Fixture {
            clock: factory.create(ctx, settings).unwrap(),
            redraws,
            _scope: scope,
        }
    }

    fn face(clock: &dyn Module) -> ClockFace {
        match clock.zone(&NavigationView::new("Accueil", 0, 1, 2)) {
            Some(ZoneContent::Clock(face)) => face,
            other => panic!("unexpected zone {other:?}"),
        }
    }

    fn leaf_texts(face: &ClockFace) -> (String, u8, String) {
        let leaf = face.leaf.as_ref().unwrap();
        (leaf.weekday.clone(), leaf.day, leaf.month.clone())
    }

    #[test]
    fn leaves_use_french_abbreviations() {
        let leaf = |at: DateTime| leaf_texts(&clock_face(at, ClockSettings::default()));
        assert_eq!(
            leaf(date(2026, 10, 28).at(9, 0, 0, 0)),
            ("mer.".into(), 28, "oct.".into())
        );
        assert_eq!(
            leaf(date(2026, 2, 1).at(0, 0, 0, 0)),
            ("dim.".into(), 1, "févr.".into())
        );
        assert_eq!(
            leaf(date(2026, 8, 3).at(0, 0, 0, 0)),
            ("lun.".into(), 3, "août".into())
        );
    }

    #[test]
    fn time_is_zero_padded() {
        assert_eq!(format_time(date(2026, 10, 28).at(7, 5, 9, 0)), "07:05");
    }

    #[test]
    fn week_numbers_follow_iso_8601() {
        let week = |year, month, day| {
            clock_face(
                date(year, month, day).at(9, 0, 0, 0),
                ClockSettings::default(),
            )
            .caption
            .unwrap()
        };
        let cases = [
            // First days of January: week 1, or the last week of the
            // previous year.
            (2026, 1, 1, "semaine 1"),
            (2027, 1, 1, "semaine 53"),
            (2027, 1, 4, "semaine 1"),
            (2024, 1, 1, "semaine 1"),
            // Last days of December: the last week, or week 1 of the next
            // year.
            (2026, 12, 28, "semaine 53"),
            (2024, 12, 29, "semaine 52"),
            (2024, 12, 30, "semaine 1"),
            (2025, 12, 31, "semaine 1"),
            // Leap day.
            (2024, 2, 29, "semaine 9"),
            (2026, 11, 5, "semaine 45"),
        ];
        for (year, month, day, expected) in cases {
            assert_eq!(week(year, month, day), expected, "{year}-{month}-{day}");
        }
    }

    #[test]
    fn faces_follow_the_date_and_seconds_settings() {
        let now = date(2026, 10, 28).at(23, 59, 59, 0);
        let settings = |show_seconds, show_date| ClockSettings {
            show_seconds,
            show_date,
        };
        let bare = clock_face(now, settings(false, false));
        assert_eq!((bare.seconds, bare.leaf, bare.caption), (None, None, None));
        let with_seconds = clock_face(now, settings(true, false));
        assert_eq!(with_seconds.seconds.map(ClockSeconds::get), Some(59));
        let first = clock_face(date(2026, 10, 28).at(0, 0, 0, 0), settings(true, true));
        assert_eq!(first.seconds.map(ClockSeconds::get), Some(0));
        assert_eq!(first.time, "00:00");
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
        let shown = face(fixture.clock.as_ref());
        assert_eq!(shown.time, "14:05");
        assert_eq!(shown.seconds, None);
        assert_eq!(leaf_texts(&shown), ("mer.".into(), 28, "oct.".into()));
        assert_eq!(shown.caption.as_deref(), Some("semaine 44"));
        fixture
            .clock
            .update_settings(&json!({ "showSeconds": true, "showDate": false }))
            .unwrap();
        let shown = face(fixture.clock.as_ref());
        assert_eq!(shown.time, "14:05");
        assert_eq!(shown.seconds.map(ClockSeconds::get), Some(9));
        assert_eq!((shown.leaf, shown.caption), (None, None));
        for invalid in [json!({ "showDate": "yes" }), json!({ "showSeconds": 1 })] {
            assert!(matches!(
                fixture.clock.update_settings(&invalid),
                Err(ModuleError::InvalidSettings(_))
            ));
        }
        // A refused update changes nothing.
        let unchanged = face(fixture.clock.as_ref());
        assert_eq!(unchanged.seconds.map(ClockSeconds::get), Some(9));
        assert_eq!((unchanged.leaf, unchanged.caption), (None, None));
    }

    #[tokio::test(start_paused = true)]
    async fn redraws_are_aligned_on_minutes() {
        let fixture = start(date(2026, 10, 28).at(14, 5, 30, 0), &json!({}));
        let redraws = || fixture.redraws.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(29_900)).await;
        assert_eq!(redraws(), 0);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(redraws(), 1);
        assert_eq!(face(fixture.clock.as_ref()).time, "14:06");
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(redraws(), 2);
        assert_eq!(face(fixture.clock.as_ref()).time, "14:07");
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
    async fn the_leaf_changes_at_midnight() {
        let fixture = start(date(2026, 12, 31).at(23, 59, 30, 0), &json!({}));
        assert_eq!(leaf_texts(&face(fixture.clock.as_ref())).1, 31);
        tokio::time::sleep(Duration::from_secs(31)).await;
        let shown = face(fixture.clock.as_ref());
        assert_eq!(shown.time, "00:00");
        assert_eq!(leaf_texts(&shown), ("ven.".into(), 1, "janv.".into()));
        assert_eq!(shown.caption.as_deref(), Some("semaine 53"));
        assert!(fixture.redraws.load(Ordering::SeqCst) >= 1);
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
