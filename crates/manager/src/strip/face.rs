//! Everything the touch strip shows, as plain data: two equal faces are
//! drawn the same, so the painter only writes the strip when it changed.

use hubergamof_module_api::{Feedback, PomodoroWidget, PopupEntry, VolumeWidget};
use hubergamof_protocol::StripTheme;

use super::clock::ClockReading;

/// One measure of the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasureFace {
    /// Short name (`CPU`).
    pub label: &'static str,
    /// Current value in percent, `None` before the first sample.
    pub percent: Option<u8>,
    /// Recent values, oldest first; empty when the curves are hidden.
    pub history: Vec<u8>,
    /// Whether the load reached the alert threshold.
    pub alert: bool,
}

/// Where the user is in the pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageFace {
    /// Path of the displayed page (`Docker › api-server`).
    pub trail: String,
    /// Scroll position of the page: its first visible row.
    pub row: u32,
    /// Scroll positions of the page (at least 1): more than one when rows are hidden.
    pub positions: u32,
}

/// The pomodoro widget and how it is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PomodoroFace {
    /// What the module reports.
    pub widget: PomodoroWidget,
    /// Whether its progress is drawn along the strip.
    pub show_progress: bool,
}

/// Reminder of what one dial does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HintFace {
    /// What turning the dial does.
    pub rotate: Option<String>,
    /// What pressing the dial does.
    pub press: Option<String>,
    /// Outcome of the last action of the dial, for a moment.
    pub feedback: Option<Feedback>,
}

/// A menu drawn over the strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopupFace {
    /// Entries, in display order.
    pub entries: Vec<PopupEntry>,
    /// Selected entry.
    pub selected: usize,
    /// Dial the card points at, `None` to centre it.
    pub dial: Option<usize>,
}

/// The whole touch strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripFace {
    /// Colours.
    pub theme: StripTheme,
    /// Time, seconds and date.
    pub clock: ClockReading,
    /// CPU, memory and swap.
    pub measures: Vec<MeasureFace>,
    /// Displayed page.
    pub page: PageFace,
    /// Volume, when an audio module is configured.
    pub volume: Option<VolumeWidget>,
    /// Pomodoro, when a pomodoro module is configured.
    pub pomodoro: Option<PomodoroFace>,
    /// One reminder per dial, left to right.
    pub hints: Vec<HintFace>,
    /// Menu opened from a dial.
    pub popup: Option<PopupFace>,
}
