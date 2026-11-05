//! Hardware abstraction: one [`DeckBackend`] per connected device, published
//! by a [`DeviceDiscovery`] registry.
//!
//! Two sources feed the registry: [`hardware::HardwareHotplug`] (real Stream
//! Decks through `hidapi`) and [`virtual_deck::VirtualDevices`] (`--demo` and
//! tests). Everything above this module only sees [`DeckBackend`] trait
//! objects, so behaviour is tested on the virtual backend.

mod discovery;
mod error;
pub mod hardware;
mod input;
mod strip;
pub mod virtual_deck;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hubergamof_protocol::DeviceInfo;
use image::RgbaImage;
use tokio::sync::broadcast;

pub use discovery::{DeviceDiscovery, DiscoveryEvent};
pub use error::DeviceError;
pub use input::{InputEvent, TouchPoint};
pub use strip::StripLayout;

/// Capacity of the per-device input event channel. A subscriber lagging by
/// more than this many events loses the oldest ones.
pub const INPUT_EVENT_CAPACITY: usize = 256;

/// One connected device (real or virtual).
///
/// Images are given at the device size (see
/// [`hubergamof_protocol::DeviceCapabilities`]); conversion to the device wire
/// format (JPEG/BMP, rotation) is the backend's job. Images are shared
/// (`Arc`) because the caller usually keeps them (render cache, live state).
/// Writes may be buffered until [`DeckBackend::flush`].
#[async_trait]
pub trait DeckBackend: Send + Sync {
    /// Description and capabilities of the device.
    fn info(&self) -> &DeviceInfo;

    /// Subscribes to the input events of the device. The last event of a
    /// device is always [`InputEvent::Disconnected`].
    fn subscribe(&self) -> broadcast::Receiver<InputEvent>;

    /// Whether the device is still usable.
    fn is_connected(&self) -> bool;

    /// Shows `image` on key `index` (row-major).
    async fn set_key_image(&self, index: u8, image: Arc<RgbaImage>) -> Result<(), DeviceError>;

    /// Draws `image` on the touch strip with its left edge at `x`.
    async fn set_strip_region(&self, x: u32, image: Arc<RgbaImage>) -> Result<(), DeviceError>;

    /// Draws `image` on touch strip zone `zone` (the zone above dial `zone`).
    async fn set_touch_zone_image(
        &self,
        zone: u8,
        image: Arc<RgbaImage>,
    ) -> Result<(), DeviceError> {
        let layout = StripLayout::for_device(self.info()).ok_or(DeviceError::NoTouchStrip)?;
        let x = layout.zone_x(zone)?;
        layout.check_zone_image(&image)?;
        self.set_strip_region(x, image).await
    }

    /// Sets the display brightness, in percent (`0..=100`).
    async fn set_brightness(&self, percent: u8) -> Result<(), DeviceError>;

    /// Blanks every key and the touch strip.
    async fn clear_all(&self) -> Result<(), DeviceError>;

    /// Sends buffered writes to the device.
    async fn flush(&self) -> Result<(), DeviceError>;

    /// Stops the device worker. The device emits [`InputEvent::Disconnected`]
    /// and every later call fails with [`DeviceError::Disconnected`].
    async fn close(&self);
}

/// Blanks every key and the touch strip of `deck`, waiting at most `timeout`;
/// failures are logged, as there is nothing else to do with them when a
/// device is being released.
pub async fn blank(deck: &dyn DeckBackend, timeout: Duration) {
    let serial = &deck.info().serial;
    let blank = async {
        deck.clear_all().await?;
        deck.flush().await
    };
    match tokio::time::timeout(timeout, blank).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(%serial, %error, "cannot blank device"),
        Err(_) => tracing::warn!(%serial, "timed out blanking device"),
    }
}
