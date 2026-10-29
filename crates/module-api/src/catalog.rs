//! Module catalog: what each module kind offers and which settings it takes.
//!
//! The UI builds settings forms generically from these descriptors. These types
//! are part of the wire contract: `bergadeck-protocol` re-exports them and
//! exports them to TypeScript, so their JSON format must stay stable.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{ModuleError, ModuleKind};

/// Description of a module kind available to the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
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

    /// Checks `settings` against the declared fields: it must be an object
    /// and every declared key present must hold a value of the declared type
    /// and constraints. Missing keys are accepted (modules fill them with
    /// their defaults) and undeclared keys are ignored.
    ///
    /// # Errors
    ///
    /// [`ModuleError::InvalidSettings`] naming the first offending setting.
    pub fn validate_settings(&self, settings: &Value) -> Result<(), ModuleError> {
        let Value::Object(values) = settings else {
            return Err(ModuleError::InvalidSettings(
                "settings must be a JSON object".into(),
            ));
        };
        self.settings
            .iter()
            .filter_map(|field| values.get(&field.key).map(|value| (field, value)))
            .try_for_each(|(field, value)| field.validate(value))
    }
}

impl SettingField {
    /// Checks that `value` has the type of the setting and respects its
    /// constraints (bounds of an integer, options of a choice).
    ///
    /// # Errors
    ///
    /// [`ModuleError::InvalidSettings`] naming the setting.
    pub fn validate(&self, value: &Value) -> Result<(), ModuleError> {
        self.kind
            .check(value)
            .map_err(|reason| ModuleError::InvalidSettings(format!("`{}`: {reason}", self.key)))
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

    /// Why `value` does not fit this kind, if it does not.
    fn check(&self, value: &Value) -> Result<(), String> {
        match self {
            Self::Boolean { .. } => expect(value.is_boolean(), "expected a boolean"),
            Self::Integer { min, max, .. } => {
                let number = value
                    .as_i64()
                    .ok_or_else(|| "expected an integer".to_owned())?;
                if let Some(min) = min.filter(|min| number < *min) {
                    return Err(format!("{number} is below the minimum {min}"));
                }
                if let Some(max) = max.filter(|max| number > *max) {
                    return Err(format!("{number} is above the maximum {max}"));
                }
                Ok(())
            }
            Self::Text { .. } | Self::Path { .. } => expect(value.is_string(), "expected a string"),
            Self::PathList { .. } => expect(
                value
                    .as_array()
                    .is_some_and(|paths| paths.iter().all(Value::is_string)),
                "expected a list of strings",
            ),
            Self::Choice { options, .. } => {
                let choice = value
                    .as_str()
                    .ok_or_else(|| "expected a string".to_owned())?;
                expect(
                    options.iter().any(|option| option.value == choice),
                    &format!("`{choice}` is not one of the allowed values"),
                )
            }
        }
    }
}

fn expect(condition: bool, reason: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(reason.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn field(key: &str, kind: SettingKind) -> SettingField {
        SettingField {
            key: key.into(),
            label: key.into(),
            help: None,
            kind,
        }
    }

    fn descriptor(settings: Vec<SettingField>) -> ModuleDescriptor {
        ModuleDescriptor {
            kind: "sample".into(),
            name: "Sample".into(),
            description: String::new(),
            capabilities: ModuleCapabilities {
                key: true,
                dial: false,
                touch_zone: false,
            },
            settings,
        }
    }

    fn bounded(min: Option<i64>, max: Option<i64>) -> SettingKind {
        SettingKind::Integer {
            default: 5,
            min,
            max,
            unit: None,
        }
    }

    fn choice() -> SettingKind {
        SettingKind::Choice {
            default: "a".into(),
            options: ["a", "b"]
                .map(|value| ChoiceOption {
                    value: value.into(),
                    label: value.into(),
                })
                .to_vec(),
        }
    }

    #[test]
    fn values_of_the_declared_type_and_bounds_are_accepted() {
        let cases = [
            (SettingKind::Boolean { default: false }, json!(true)),
            (bounded(Some(1), Some(10)), json!(10)),
            (bounded(None, None), json!(-3)),
            (
                SettingKind::Text {
                    default: String::new(),
                },
                json!("x"),
            ),
            (
                SettingKind::Path {
                    default: String::new(),
                    directory: true,
                },
                json!("/tmp"),
            ),
            (
                SettingKind::PathList {
                    default: Vec::new(),
                },
                json!(["/a", "/b"]),
            ),
            (choice(), json!("b")),
        ];
        for (kind, value) in cases {
            assert!(
                field("k", kind).validate(&value).is_ok(),
                "{value} rejected"
            );
        }
    }

    #[test]
    fn wrong_types_bounds_and_choices_are_rejected() {
        let cases = [
            (SettingKind::Boolean { default: false }, json!("yes")),
            (bounded(None, None), json!(1.5)),
            (bounded(Some(1), None), json!(0)),
            (bounded(None, Some(10)), json!(11)),
            (
                SettingKind::Text {
                    default: String::new(),
                },
                json!(3),
            ),
            (
                SettingKind::PathList {
                    default: Vec::new(),
                },
                json!(["/a", 1]),
            ),
            (choice(), json!("c")),
            (choice(), json!(null)),
        ];
        for (kind, value) in cases {
            let error = field("k", kind).validate(&value).unwrap_err();
            assert!(
                matches!(&error, ModuleError::InvalidSettings(reason) if reason.starts_with("`k`")),
                "{value}: {error}"
            );
        }
    }

    #[test]
    fn settings_objects_are_checked_field_by_field() {
        let descriptor = descriptor(vec![
            field("flag", SettingKind::Boolean { default: true }),
            field("count", bounded(Some(0), None)),
        ]);
        assert!(descriptor.validate_settings(&json!({})).is_ok());
        assert!(
            descriptor
                .validate_settings(&json!({ "flag": false, "extra": 1 }))
                .is_ok()
        );
        assert!(
            descriptor
                .validate_settings(&json!({ "count": -1 }))
                .is_err()
        );
        assert!(descriptor.validate_settings(&json!([])).is_err());
    }

    #[test]
    fn default_settings_collect_every_field_default() {
        let descriptor = descriptor(vec![
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
        ]);
        assert_eq!(
            descriptor.default_settings(),
            json!({ "flag": true, "count": 25, "paths": ["/a"], "mode": "x" })
        );
    }
}
