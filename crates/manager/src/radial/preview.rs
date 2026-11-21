//! `hubergamof radial-preview`: shows the radial menu for a few seconds so
//! that the window can be tried without the mouse buttons or the service.
//! There is no opening button: the menu stays open as after a short press,
//! hover lights a petal, a click chooses it, Escape closes.

use std::io::Write;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use hubergamof_protocol::RadialMenuConfig;
use thiserror::Error;

use super::backend::{self, NoWindow};
use super::controller::{Controller, Effect};
use super::geometry::{DEFAULT_RING, Size};
use super::looks::Looks;
use super::paint::DEFAULT_PALETTE;
use super::service::{NUDGE_PERIOD, NUDGE_TRIES};
use super::window::Surface;

/// What the screen is taken to be until the window reports its size.
const UNKNOWN_SCREEN: Size = Size {
    width: 1920.0,
    height: 1080.0,
};

/// Why the preview does not run.
#[derive(Debug, Error)]
pub enum PreviewError {
    /// The window cannot be started.
    #[error(transparent)]
    NoWindow(#[from] NoWindow),
    /// The menu has no entry to show.
    #[error("the radial menu shows no entry")]
    NoEntry,
    /// The window stopped on its own (its error is in the log).
    #[error("the window of the menu stopped")]
    WindowGone,
    /// The report cannot be written.
    #[error("cannot write the report: {0}")]
    Write(#[from] std::io::Error),
}

/// Shows the default menu for at most `duration`, reporting on `out` what the
/// person chooses. `nudge` moves the pointer a pixel and back, tried while the
/// menu is not placed, as the service does. `looks` gives the petals their
/// names and icons.
///
/// # Errors
///
/// [`PreviewError`] when the window cannot start or the report cannot be written.
pub fn run(
    duration: Duration,
    out: &mut impl Write,
    nudge: impl Fn(),
    looks: Looks,
) -> Result<(), PreviewError> {
    let (events, window_events) = mpsc::channel();
    let overlay = backend::start(Arc::new(move |event| events.send(event).is_ok()))?;
    let mut controller = Controller::new(
        &RadialMenuConfig::default_menu(),
        DEFAULT_RING,
        UNKNOWN_SCREEN,
    );
    controller.set_looks(looks);
    let content = controller
        .content(DEFAULT_PALETTE, true)
        .ok_or(PreviewError::NoEntry)?;
    let started = Instant::now();
    // A press then a release at once: the menu stays open, clicks choose.
    let mut effects = controller.press(started.elapsed());
    effects.extend(controller.release(started.elapsed()));
    writeln!(
        out,
        "Move the pointer over a petal and click it; Escape closes."
    )?;
    let mut content = Some(content);
    let mut nudges_left = NUDGE_TRIES;
    loop {
        for effect in effects.drain(..) {
            match effect {
                Effect::Open => {
                    if let Some(content) = content.take() {
                        overlay.open(content);
                    }
                }
                Effect::Place(view) => overlay.place(view),
                Effect::Redraw(view) => overlay.redraw(view),
                Effect::Choose(entry) => writeln!(out, "Chosen: {}", entry.module)?,
                Effect::Close => {
                    overlay.close();
                    return Ok(());
                }
            }
        }
        let Some(left) = duration.checked_sub(started.elapsed()) else {
            overlay.close();
            writeln!(out, "Time is up.")?;
            return Ok(());
        };
        let waiting_for_the_pointer = controller.view().is_none() && nudges_left > 0;
        let wait = if waiting_for_the_pointer {
            left.min(NUDGE_PERIOD)
        } else {
            left
        };
        match window_events.recv_timeout(wait) {
            Ok(event) => effects = controller.window(event),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if waiting_for_the_pointer {
                    nudges_left -= 1;
                    nudge();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(PreviewError::WindowGone),
        }
    }
}
