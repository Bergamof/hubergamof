//! Live state of a device, as currently displayed.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::KeyState;

/// What a device currently shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LiveState {
    /// Device serial.
    pub serial: String,
    /// Page currently displayed on the keys.
    pub page: PageView,
    /// The whole touch strip as a PNG data URL; `null` on a device without
    /// a strip.
    pub strip: Option<String>,
}

/// A page as currently displayed on the keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PageView {
    /// Page identifier.
    pub id: String,
    /// Page title.
    pub title: String,
    /// Index of the visible row when the page has more rows than the device.
    pub row: u32,
    /// Total number of rows of the page.
    pub row_count: u32,
    /// Visible keys.
    pub keys: Vec<KeyView>,
}

/// A key as currently displayed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KeyView {
    /// Key index on the device, row-major.
    pub index: u32,
    /// Text label, if any.
    pub label: Option<String>,
    /// Rendered key image as a data URL.
    pub image: Option<String>,
    /// Visual state reported by the module.
    pub state: KeyState,
}
