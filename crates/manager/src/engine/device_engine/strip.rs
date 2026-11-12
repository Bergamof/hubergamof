//! The touch strip of a device: what it shows now, from the configuration,
//! the modules, the clock and the measures of the machine.

use std::sync::Arc;
use std::time::Duration;

use hubergamof_module_api::{ModuleInstanceId, StripWidget};
use hubergamof_protocol::DialAssignment;
use tokio::task::spawn_blocking;
use tokio::time::Instant;

use super::DeviceEngine;
use crate::strip::{
    ClockReading, HintFace, MeasureFace, PageFace, PomodoroFace, PopupFace, StripFace,
    SystemSample, delay_until_next_change,
};

/// Reminder of the navigation dial: what turning and pressing it do.
const NAVIGATION_ROTATE_HINT: &str = "Navigation";
const NAVIGATION_PRESS_HINT: &str = "Retour";
/// Reminder of the contextual dial, whose function is not defined yet.
const CONTEXTUAL_HINT: &str = "Contexte";
/// Between the parent page and the displayed one in the path of the page.
const TRAIL_SEPARATOR: &str = "›";

impl DeviceEngine {
    /// When the time shown changes, `None` without a strip.
    pub(super) fn next_clock_tick(&self) -> Option<Instant> {
        let delay = delay_until_next_change(
            self.services.wall_clock.now(),
            self.config.strip.show_seconds,
        );
        self.has_strip.then(|| Instant::now() + delay)
    }

    /// Plans the next measure of the machine one period from now.
    pub(super) fn schedule_system_sample(&mut self) {
        let period = Duration::from_secs(u64::from(self.config.strip.system_refresh_seconds));
        self.next_system_sample = Some(Instant::now() + period);
    }

    /// Measures the machine, from the blocking pool, and plans the next
    /// measure.
    pub(super) async fn sample_system(&mut self) {
        self.schedule_system_sample();
        let probe = Arc::clone(&self.services.system);
        match spawn_blocking(move || probe.sample()).await {
            Ok(Some(sample)) => self.system.push(sample),
            Ok(None) => tracing::debug!(serial = %self.serial(), "system measure unavailable"),
            Err(error) => tracing::warn!(serial = %self.serial(), %error, "system measure failed"),
        }
    }

    /// The strip to draw: `title` is the displayed page, at `row` of
    /// `positions` scroll positions.
    pub(super) fn strip_face(&mut self, title: &str, row: u32, positions: u32) -> StripFace {
        let strip = self.config.strip;
        let (volume, pomodoro) = self.widgets();
        StripFace {
            theme: self.config.theme,
            clock: ClockReading::at(self.services.wall_clock.now(), strip.show_seconds),
            measures: self.measures(),
            page: PageFace {
                trail: self.trail(title),
                row,
                positions,
            },
            volume,
            pomodoro: pomodoro.map(|widget| PomodoroFace {
                widget,
                show_progress: strip.show_pomodoro_progress,
            }),
            hints: self.hints(),
            popup: self.popup(),
        }
    }

    /// Path of the displayed page: its parent and itself, or the home page
    /// alone.
    fn trail(&self, title: &str) -> String {
        if self.nav.current().is_some() {
            format!("{} {TRAIL_SEPARATOR} {title}", self.parent_title())
        } else {
            title.to_owned()
        }
    }

    /// The first volume and the first pomodoro the modules show.
    fn widgets(
        &mut self,
    ) -> (
        Option<hubergamof_module_api::VolumeWidget>,
        Option<hubergamof_module_api::PomodoroWidget>,
    ) {
        let ids: Vec<ModuleInstanceId> = self
            .config
            .modules
            .iter()
            .map(|module| module.id.clone())
            .collect();
        let (mut volume, mut pomodoro) = (None, None);
        for id in &ids {
            match self.instances.strip_widget(id) {
                Some(StripWidget::Volume(widget)) if volume.is_none() => volume = Some(widget),
                Some(StripWidget::Pomodoro(widget)) if pomodoro.is_none() => {
                    pomodoro = Some(widget);
                }
                Some(_) => tracing::debug!(module = %id, "widget ignored, its place is taken"),
                None => {}
            }
        }
        (volume, pomodoro)
    }

    fn measures(&self) -> Vec<MeasureFace> {
        let strip = self.config.strip;
        let latest = self.system.latest();
        let measure = |label: &'static str, read: fn(&SystemSample) -> u8| {
            let percent = latest.as_ref().map(read);
            MeasureFace {
                label,
                percent,
                history: if strip.show_history {
                    self.system.series(read)
                } else {
                    Vec::new()
                },
                alert: percent.is_some_and(|percent| percent >= strip.load_alert_percent),
            }
        };
        vec![
            measure("CPU", |sample| sample.cpu),
            measure("RAM", |sample| sample.ram),
            measure("SWAP", |sample| sample.swap),
        ]
    }

    /// One reminder per dial: what the function assigned to it does now.
    fn hints(&mut self) -> Vec<HintFace> {
        let dial_count = self.deck.info().capabilities.dial_count;
        let mut hints = Vec::with_capacity(usize::from(dial_count));
        for dial in 0..dial_count {
            let assignment = self
                .config
                .dials
                .get(usize::from(dial))
                .cloned()
                .unwrap_or(DialAssignment::None);
            let (rotate, press) = match assignment {
                DialAssignment::None => (None, None),
                DialAssignment::Navigation => (
                    Some(NAVIGATION_ROTATE_HINT.to_owned()),
                    // The home page has nothing to go back to.
                    self.nav
                        .current()
                        .is_some()
                        .then(|| NAVIGATION_PRESS_HINT.to_owned()),
                ),
                DialAssignment::Contextual => (Some(CONTEXTUAL_HINT.to_owned()), None),
                DialAssignment::Module { instance, function } => self
                    .instances
                    .dial_hint(&instance, &function)
                    .map(|hint| (hint.rotate, hint.press))
                    .unwrap_or_default(),
            };
            hints.push(HintFace {
                rotate,
                press,
                feedback: self.marks.dial(dial),
            });
        }
        hints
    }

    /// The menu of the first dial function that has one open.
    fn popup(&mut self) -> Option<PopupFace> {
        let assignments = self.config.dials.clone();
        for (assignment, dial) in assignments.into_iter().zip(0_usize..) {
            let DialAssignment::Module { instance, function } = assignment else {
                continue;
            };
            let Some(popup) = self.instances.strip_popup(&instance) else {
                continue;
            };
            if popup.function == function {
                return Some(PopupFace {
                    entries: popup.entries().to_vec(),
                    selected: popup.selected(),
                    dial: Some(dial),
                });
            }
        }
        None
    }
}
