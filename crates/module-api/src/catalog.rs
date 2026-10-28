//! Module catalog: what each module kind offers and which settings it takes.
//!
//! The UI builds settings forms generically from these descriptors. These types
//! are part of the wire contract: `bergadeck-protocol` re-exports them and
//! exports them to TypeScript, so their JSON format must stay stable.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

use crate::ModuleKind;

/// Description of a module kind available to the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModuleDescriptor {
    /// Module kind, referenced by the `kind` of a configured module instance.
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
    /// Key of the setting in the `settings` object of a module instance.
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

impl ModuleDescriptor {
    /// Settings object holding the default value of every setting, as stored
    /// in a new module instance.
    #[must_use]
    pub fn default_settings(&self) -> Value {
        let settings: Map<String, Value> = self
            .settings
            .iter()
            .map(|field| (field.key.clone(), field.kind.default_value()))
            .collect();
        Value::Object(settings)
    }
}

impl SettingKind {
    /// Default value of the setting, as JSON.
    #[must_use]
    pub fn default_value(&self) -> Value {
        match self {
            Self::Boolean { default } => Value::Bool(*default),
            Self::Integer { default, .. } => Value::from(*default),
            Self::Text { default } | Self::Path { default, .. } | Self::Choice { default, .. } => {
                Value::String(default.clone())
            }
            Self::PathList { default } => {
                Value::Array(default.iter().cloned().map(Value::String).collect())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn default_settings_collect_every_field_default() {
        let field = |key: &str, kind| SettingField {
            key: key.into(),
            label: key.into(),
            help: None,
            kind,
        };
        let descriptor = ModuleDescriptor {
            kind: "sample".into(),
            name: "Sample".into(),
            description: String::new(),
            capabilities: ModuleCapabilities {
                key: true,
                dial: false,
                touch_zone: false,
            },
            settings: vec![
                field("flag", SettingKind::Boolean { default: true }),
                field(
                    "count",
                    SettingKind::Integer {
                        default: 25,
                        min: None,
                        max: None,
                        unit: None,
                    },
                ),
                field(
                    "paths",
                    SettingKind::PathList {
                        default: vec!["/a".into()],
                    },
                ),
                field(
                    "mode",
                    SettingKind::Choice {
                        default: "x".into(),
                        options: Vec::new(),
                    },
                ),
            ],
        };
        assert_eq!(
            descriptor.default_settings(),
            json!({ "flag": true, "count": 25, "paths": ["/a"], "mode": "x" })
        );
    }
}
