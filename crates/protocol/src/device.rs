//! Physical (or virtual) device description.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Stream Deck hardware model.
///
/// Mirrors the models supported by the `elgato-streamdeck` crate, plus
/// [`DeviceModel::Virtual`] for the `--demo` backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DeviceModel {
    /// Original Stream Deck, first revision.
    Original,
    /// Original Stream Deck, second revision.
    OriginalV2,
    /// Stream Deck Mini.
    Mini,
    /// Stream Deck Mini Mk2.
    MiniMk2,
    /// Stream Deck Mini Discord Edition.
    MiniDiscord,
    /// Stream Deck Mini Mk2 module.
    MiniMk2Module,
    /// Stream Deck XL, first revision.
    Xl,
    /// Stream Deck XL, second revision.
    XlV2,
    /// Stream Deck XL module.
    XlV2Module,
    /// Stream Deck Mk2.
    Mk2,
    /// Stream Deck Mk2 with scissor keys.
    Mk2Scissor,
    /// Stream Deck Mk2 module.
    Mk2Module,
    /// Stream Deck Mk2 module with scissor keys.
    Mk2ModuleScissor,
    /// Stream Deck Neo.
    Neo,
    /// Stream Deck Pedal.
    Pedal,
    /// Stream Deck+.
    Plus,
    /// Stream Deck+ XL.
    PlusXl,
    /// Simulated device used by the `--demo` backend.
    Virtual,
}

/// Pixel dimensions of an image surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImageSize {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// What a device can display and which inputs it offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceCapabilities {
    /// Number of key columns.
    pub key_columns: u8,
    /// Number of key rows.
    pub key_rows: u8,
    /// Size of a key image.
    pub key_image_size: ImageSize,
    /// Number of rotary dials.
    pub dial_count: u8,
    /// Size of the touch strip, if the device has one.
    pub touch_strip: Option<ImageSize>,
}

impl DeviceCapabilities {
    /// Total number of keys.
    #[must_use]
    pub fn key_count(&self) -> usize {
        usize::from(self.key_columns) * usize::from(self.key_rows)
    }
}

/// A device currently known to the manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceInfo {
    /// Hardware serial number, used as the device identifier.
    pub serial: String,
    /// Hardware model.
    pub model: DeviceModel,
    /// Human-readable name shown in the UI.
    pub display_name: String,
    /// Display and input capabilities.
    pub capabilities: DeviceCapabilities,
}
