//! The touch strip as a whole: what it shows ([`StripFace`]), the local time
//! it reads ([`WallClock`]) and the machine load it measures
//! ([`SystemProbe`]). It is drawn by [`crate::render::Renderer::render_strip`]
//! and belongs to no dial: the dials only get a reminder under it.

mod clock;
mod face;
mod system;

pub use clock::{ClockReading, FixedClock, SystemClock, WallClock, delay_until_next_change};
pub use face::{HintFace, MeasureFace, PageFace, PomodoroFace, PopupFace, StripFace};
pub use system::{
    HISTORY_LEN, ProcProbe, SimulatedProbe, SystemHistory, SystemProbe, SystemSample,
};
