//! Picks the window of the radial menu the desktop can show: a layer-shell
//! surface on Wayland compositors that have the protocol (KDE, wlroots), an X11
//! window otherwise (Xorg, or `XWayland` under GNOME).

use thiserror::Error;

use super::overlay::{Overlay, OverlayError};
use super::overlay_x11::{X11Error, X11Overlay};
use super::window::{Report, Surface};

/// Why no window can be shown, one reason for each way tried.
#[derive(Debug, Error)]
#[error("Wayland layer-shell: {wayland}; X11: {x11}")]
pub struct NoWindow {
    wayland: Box<OverlayError>,
    x11: Box<X11Error>,
}

/// Starts the window the desktop allows, preferring the layer-shell one.
///
/// # Errors
///
/// [`NoWindow`] when neither can start.
pub fn start(events: Report) -> Result<Box<dyn Surface + Send>, NoWindow> {
    let wayland = match Overlay::start(events.clone()) {
        Ok(window) => return Ok(Box::new(window)),
        Err(error) => error,
    };
    tracing::debug!(%wayland, "no layer-shell window, trying X11");
    match X11Overlay::start(events) {
        Ok(window) => {
            tracing::info!("the radial menu is shown in an X11 window");
            Ok(Box::new(window))
        }
        Err(x11) => Err(NoWindow {
            wayland: Box::new(wayland),
            x11: Box::new(x11),
        }),
    }
}
