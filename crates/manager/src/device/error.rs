//! Device errors.

use hubergamof_protocol::MAX_BRIGHTNESS;
use thiserror::Error;

/// Failure of a device operation.
#[derive(Debug, Error)]
pub enum DeviceError {
    /// The device was unplugged or closed.
    #[error("device disconnected")]
    Disconnected,
    /// The key index does not exist on the device.
    #[error("key {index} does not exist (device has {count} keys)")]
    InvalidKey {
        /// Requested key.
        index: u8,
        /// Number of keys of the device.
        count: usize,
    },
    /// The dial index does not exist on the device.
    #[error("dial {index} does not exist (device has {count} dials)")]
    InvalidDial {
        /// Requested dial.
        index: u8,
        /// Number of dials of the device.
        count: u8,
    },
    /// The touch strip zone does not exist on the device.
    #[error("touch strip zone {index} does not exist (device has {count} zones)")]
    InvalidZone {
        /// Requested zone.
        index: u8,
        /// Number of zones of the device.
        count: u8,
    },
    /// The device has no touch strip.
    #[error("device has no touch strip")]
    NoTouchStrip,
    /// The image does not have the expected size.
    #[error("image is {actual_width}x{actual_height}, expected {expected_width}x{expected_height}")]
    ImageSizeMismatch {
        /// Expected width.
        expected_width: u32,
        /// Expected height.
        expected_height: u32,
        /// Actual width.
        actual_width: u32,
        /// Actual height.
        actual_height: u32,
    },
    /// The image does not fit in the touch strip at the requested position.
    #[error("strip region at x={x} of width {width} exceeds the strip")]
    RegionOutOfBounds {
        /// Left edge of the region.
        x: u32,
        /// Width of the region.
        width: u32,
    },
    /// Brightness above [`MAX_BRIGHTNESS`].
    #[error("brightness {0} is above {MAX_BRIGHTNESS}")]
    BrightnessOutOfRange(u8),
    /// The operation touches a coordinate outside the device protocol range.
    #[error("coordinate {0} is outside the device protocol range")]
    CoordinateOutOfRange(u32),
    /// The thread driving a hardware device could not be started.
    #[error("cannot start the device worker thread: {0}")]
    WorkerSpawn(#[source] std::io::Error),
    /// The HID subsystem failed (initialisation, enumeration).
    #[error("HID error: {0}")]
    Hid(#[from] hidapi::HidError),
    /// A blocking HID task panicked or was cancelled.
    #[error("HID task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
    /// Error reported by the Stream Deck library.
    #[error("Stream Deck error: {0}")]
    Hardware(#[from] elgato_streamdeck::StreamDeckError),
}
