//! Module catalog: what each module kind offers and which settings it takes.
//!
//! The UI builds settings forms generically from these descriptors.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ModuleKind;

/// Description of a module kind available to the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModuleDescriptor {
    /// Module kind, referenced by [`crate::ModuleInstance::kind`].
    pub kind: ModuleKind,
    /// Human-readable name.
    pub name: String,
    /// Short description shown in the catalog.
    pub description: String,
    /// Where an instance of this module can be placed.
    pub capabilities: ModuleCapabilities,
    /// Settings accepted by the module, in display order.
    pub settings: Vec<SettingField>,
}

/// Device surfaces a module can drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirrored as-is in the JSON contract"
)]
pub struct ModuleCapabilities {
    /// Can be placed on a key.
    pub key: bool,
    /// Can be assigned to a dial.
    pub dial: bool,
    /// Can render a touch strip zone.
    pub touch_zone: bool,
}

/// One setting of a module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SettingField {
    /// Key of the setting in [`crate::ModuleInstance::settings`].
    pub key: String,
    /// Form label.
    pub label: String,
    /// Optional help text.
    pub help: Option<String>,
    /// Value type, default and constraints.
    pub kind: SettingKind,
}

/// Type of a setting value, with its default and constraints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum SettingKind {
    /// On/off switch.
    Boolean {
        /// Default value.
        default: bool,
    },
    /// Whole number, optionally bounded.
    Integer {
        /// Default value.
        default: i64,
        /// Inclusive lower bound.
        min: Option<i64>,
        /// Inclusive upper bound.
        max: Option<i64>,
        /// Unit displayed next to the field (`"s"`, `"%"`…).
        unit: Option<String>,
    },
    /// Free text.
    Text {
        /// Default value.
        default: String,
    },
    /// Filesystem path.
    Path {
        /// Default value.
        default: String,
        /// Whether the path must be a directory rather than a file.
        directory: bool,
    },
    /// Ordered list of filesystem paths.
    PathList {
        /// Default value.
        default: Vec<String>,
    },
    /// One value among a fixed set.
    Choice {
        /// Default value, one of `options[].value`.
        default: String,
        /// Allowed values.
        options: Vec<ChoiceOption>,
    },
}

/// An allowed value of a [`SettingKind::Choice`] setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChoiceOption {
    /// Stored value.
    pub value: String,
    /// Displayed label.
    pub label: String,
}
