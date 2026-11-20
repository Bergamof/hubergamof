//! `hubergamof radial-preview`: shows the radial menu for a few seconds so
//! that the window can be tried without the mouse buttons or the service.
//! There is no opening button: the menu stays open as after a short press,
//! hover lights a petal, a click chooses it, Escape closes.

use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use hubergamof_protocol::RadialMenuConfig;
use thiserror::Error;

use super::controller::{Controller, Effect};
use super::geometry::{DEFAULT_RING, Size};
use super::overlay::{Overlay, OverlayError};
use super::paint::DEFAULT_PALETTE;

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
    Overlay(#[from] OverlayError),
    /// The menu has no entry to show.
    #[error("the radial menu shows no entry")]
    NoEntry,
    /// The report cannot be written.
    #[error("cannot write the report: {0}")]
    Write(#[from] std::io::Error),
}

/// Shows the default menu for at most `duration`, reporting on `out` what the
/// person chooses.
///
/// # Errors
///
/// [`PreviewError`] when the window cannot start or the report cannot be written.
pub fn run(duration: Duration, out: &mut impl Write) -> Result<(), PreviewError> {
    let (events, window_events) = mpsc::channel();
    let overlay = Overlay::start(events)?;
    let mut controller = Controller::new(
        &RadialMenuConfig::default_menu(),
        DEFAULT_RING,
        UNKNOWN_SCREEN,
    );
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
        match window_events.recv_timeout(left) {
            Ok(event) => effects = controller.window(event),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}
