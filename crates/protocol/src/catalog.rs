//! Module catalog as sent to the UI.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::ModuleDescriptor;

/// A module kind the user can configure, with the image standing for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CatalogModule {
    /// Kind, name, capabilities and settings of the module.
    #[serde(flatten)]
    pub descriptor: ModuleDescriptor,
    /// Icon of the module, rendered like a key, as a PNG data URL; `null`
    /// when the module has none.
    pub icon: Option<String>,
    /// Settings of a new instance, as the module chooses them (usually the
    /// default of every field, but a module may compute its own).
    pub default_settings: Value,
}
