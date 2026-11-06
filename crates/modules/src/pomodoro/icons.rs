//! Glyphs inside the ring of the zone gauge (SVG without background, drawn by
//! the host).

use std::sync::LazyLock;

use hubergamof_module_api::ImageData;

use crate::icons::{asset, svg};

/// Tomato: work in progress.
pub(super) static WORK: LazyLock<ImageData> = LazyLock::new(|| svg(asset!("pomodoro-glyph-work")));
/// Cup: break in progress.
pub(super) static BREAK: LazyLock<ImageData> =
    LazyLock::new(|| svg(asset!("pomodoro-glyph-break")));
/// Pause bars: the timer is stopped.
pub(super) static PAUSED: LazyLock<ImageData> =
    LazyLock::new(|| svg(asset!("pomodoro-glyph-paused")));
