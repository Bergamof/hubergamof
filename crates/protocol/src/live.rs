//! Live state of a device, as currently displayed.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{KeyState, ModuleInstanceId};

/// What a device currently shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LiveState {
    /// Device serial.
    pub serial: String,
    /// Page currently displayed on the keys.
    pub page: PageView,
    /// State of each dial, by dial index.
    pub dials: Vec<DialView>,
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

/// A dial and its touch strip zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DialView {
    /// Dial index on the device.
    pub index: u32,
    /// Module instance assigned to the dial.
    pub module_id: Option<ModuleInstanceId>,
    /// Touch strip zone above the dial, if the device has a strip.
    pub zone: Option<ZoneView>,
}

/// A touch strip zone as currently displayed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ZoneView {
    /// Text lines shown in the zone.
    pub lines: Vec<String>,
    /// Rendered zone image as a data URL.
    pub image: Option<String>,
}
