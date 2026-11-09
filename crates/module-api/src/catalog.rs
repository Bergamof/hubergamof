//! Module catalog: what each module kind offers and which settings it takes.
//!
//! The UI builds settings forms generically from these descriptors. These types
//! are part of the wire contract: `hubergamof-protocol` re-exports them and
//! exports them to TypeScript, so their JSON format must stay stable.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::file_entries::check_file_entries;
use crate::{
    EntryNaming, FileChooser, FileEntry, ModuleError, ModuleKind, is_setting_path, parse_settings,
};

/// Description of a module kind available to the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
// Not exported on its own: the contract sends it flattened into
// `hubergamof_protocol::CatalogModule`.
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    /// Filesystem path: absolute, or starting with `~/` for the home
    /// directory (see [`crate::expand_home`]); empty when not set.
    Path {
        /// Default value.
        default: String,
        /// Whether the path must be a directory rather than a file.
        directory: bool,
    },
    /// Ordered list of filesystem paths, each absolute or starting with
    /// `~/` (see [`SettingKind::Path`]).
    PathList {
        /// Default value.
        default: Vec<String>,
    },
    /// Ordered list of files, each with an optional name and icon: see
    /// [`FileEntry`]. The UI lets the user pick a file with the system file
    /// chooser, then name it and give it an icon.
    FileEntryList {
        /// Default value.
        default: Vec<FileEntry>,
        /// Most files in the list.
        max_entries: usize,
        /// Longest name of a file, in characters.
        max_name_chars: usize,
        /// The dialog choosing a file; without one the path is typed.
        chooser: Option<FileChooser>,
        /// Example path shown in the empty path field.
        path_placeholder: String,
        /// What the empty name field means (« Vide : nom du projet »).
        name_hint: String,
        /// What a file without name is called.
        default_name: EntryNaming,
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

    /// Checks `settings` against the declared fields
    /// ([`ModuleDescriptor::validate_settings`]), then deserializes them into
    /// the typed settings of the module ([`parse_settings`]): the one call a
    /// module needs in [`crate::ModuleFactory::create`] and
    /// [`crate::Module::update_settings`], so that bounds are enforced even
    /// when the host did not validate first.
    ///
    /// # Errors
    ///
    /// [`ModuleError::InvalidSettings`] naming the first offending setting,
    /// or explaining why `settings` does not match `T`.
    pub fn parse_settings<T: DeserializeOwned>(&self, settings: &Value) -> Result<T, ModuleError> {
        self.validate_settings(settings)?;
        parse_settings(settings)
    }
}

impl ModuleDescriptor {
    /// Names of the stored images `settings` refer to (the icons of its file
    /// entries), in order, duplicates included.
    #[must_use]
    pub fn settings_images(&self, settings: &Value) -> Vec<String> {
        let mut images = Vec::new();
        for icon in self.file_entry_icons(settings) {
            if let Some(name) = icon.as_str() {
                images.push(name.to_owned());
            }
        }
        images
    }

    /// Replaces every image `settings` refer to by `replace(name)`; `None`
    /// removes the reference. Used when images are renamed (import).
    pub fn replace_settings_images(
        &self,
        settings: &mut Value,
        mut replace: impl FnMut(&str) -> Option<String>,
    ) {
        for field in &self.settings {
            if !matches!(field.kind, SettingKind::FileEntryList { .. }) {
                continue;
            }
            let Some(entries) = settings.get_mut(&field.key).and_then(Value::as_array_mut) else {
                continue;
            };
            for entry in entries.iter_mut().filter_map(Value::as_object_mut) {
                let Some(name) = entry.get(ICON_KEY).and_then(Value::as_str) else {
                    continue;
                };
                match replace(name) {
                    Some(new_name) => entry.insert(ICON_KEY.to_owned(), Value::String(new_name)),
                    None => entry.remove(ICON_KEY),
                };
            }
        }
    }

    fn file_entry_icons<'a>(&'a self, settings: &'a Value) -> impl Iterator<Item = &'a Value> {
        self.settings
            .iter()
            .filter(|field| matches!(field.kind, SettingKind::FileEntryList { .. }))
            .filter_map(|field| settings.get(&field.key)?.as_array())
            .flatten()
            .filter_map(|entry| entry.get(ICON_KEY))
    }
}

/// Key of the icon in a stored [`FileEntry`].
const ICON_KEY: &str = "icon";

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
            Self::FileEntryList { default, .. } => {
                serde_json::to_value(default).unwrap_or_else(|_| Value::Array(Vec::new()))
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
            Self::Text { .. } => expect(value.is_string(), "expected a string"),
            Self::Path { .. } => {
                let path = value
                    .as_str()
                    .ok_or_else(|| "expected a string".to_owned())?;
                if path.is_empty() {
                    Ok(())
                } else {
                    check_path(path)
                }
            }
            Self::PathList { .. } => value
                .as_array()
                .ok_or_else(|| "expected a list of strings".to_owned())?
                .iter()
                .try_for_each(|path| {
                    check_path(
                        path.as_str()
                            .ok_or_else(|| "expected a list of strings".to_owned())?,
                    )
                }),
            Self::FileEntryList {
                max_entries,
                max_name_chars,
                ..
            } => check_file_entries(value, *max_entries, *max_name_chars),
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

/// A path the host can resolve without a working directory.
fn check_path(path: &str) -> Result<(), String> {
    if is_setting_path(path) {
        Ok(())
    } else {
        Err(format!(
            "`{path}` is neither absolute nor in the home directory (`~/…`)"
        ))
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
                SettingKind::Path {
                    default: String::new(),
                    directory: false,
                },
                json!(""),
            ),
            (
                SettingKind::PathList {
                    default: Vec::new(),
                },
                json!(["/a", "~/b"]),
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
            (
                SettingKind::PathList {
                    default: Vec::new(),
                },
                json!(["relative/compose.yaml"]),
            ),
            (
                SettingKind::PathList {
                    default: Vec::new(),
                },
                json!(["~user/file"]),
            ),
            (
                SettingKind::PathList {
                    default: Vec::new(),
                },
                json!(["~"]),
            ),
            (
                SettingKind::Path {
                    default: String::new(),
                    directory: true,
                },
                json!("projects"),
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
    fn parsing_checks_the_bounds_before_deserializing() {
        #[derive(Debug, PartialEq, Deserialize)]
        struct Typed {
            #[serde(default)]
            count: i64,
        }
        let descriptor = descriptor(vec![field("count", bounded(Some(0), Some(9)))]);
        assert_eq!(
            descriptor
                .parse_settings::<Typed>(&json!({ "count": 3 }))
                .unwrap(),
            Typed { count: 3 }
        );
        assert_eq!(
            descriptor.parse_settings::<Typed>(&json!({})).unwrap(),
            Typed { count: 0 }
        );
        // Within the type, outside the declared bounds.
        assert!(matches!(
            descriptor.parse_settings::<Typed>(&json!({ "count": 10 })),
            Err(ModuleError::InvalidSettings(reason)) if reason.starts_with("`count`")
        ));
        assert!(descriptor.parse_settings::<Typed>(&json!([])).is_err());
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

    const MAX_ENTRIES: usize = 50;
    const MAX_NAME_CHARS: usize = 40;

    fn file_entries() -> SettingKind {
        SettingKind::FileEntryList {
            default: Vec::new(),
            max_entries: MAX_ENTRIES,
            max_name_chars: MAX_NAME_CHARS,
            chooser: None,
            path_placeholder: "/path/to/file".into(),
            name_hint: "Empty: the file name.".into(),
            default_name: EntryNaming::FileName,
        }
    }

    #[test]
    fn file_entries_hold_a_path_a_name_and_an_icon() {
        let accepted = [
            json!([]),
            json!(["~/app/compose.yaml"]),
            json!([{ "path": "/srv/compose.yaml" }]),
            json!([{ "path": "~/a.yaml", "name": null, "icon": null }]),
            json!([{ "path": "~/a.yaml", "name": "Web", "icon": "0f3a.png" }]),
            json!([{ "path": "~/a.yaml", "name": "x".repeat(MAX_NAME_CHARS) }]),
            json!([{ "path": "~/a.yaml", "name": "é".repeat(MAX_NAME_CHARS) }]),
        ];
        for value in accepted {
            assert!(field("files", file_entries()).validate(&value).is_ok());
        }
    }

    #[test]
    fn invalid_file_entries_are_rejected() {
        let rejected = [
            json!("a.yaml"),
            json!([1]),
            json!([{}]),
            json!([{ "path": "relative.yaml" }]),
            json!([{ "path": "~/a.yaml", "name": 3 }]),
            json!([{ "path": "~/a.yaml", "name": "a\nb" }]),
            json!([{ "path": "~/a.yaml", "name": "a\u{202E}b" }]),
            json!([{ "path": "~/a.yaml", "name": "x".repeat(MAX_NAME_CHARS + 1) }]),
            json!([{ "path": "~/a.yaml", "icon": "../etc/passwd" }]),
            json!([{ "path": "~/a.yaml", "icon": 4 }]),
            Value::Array(vec![json!("/a.yaml"); MAX_ENTRIES + 1]),
        ];
        for value in rejected {
            assert!(
                field("files", file_entries()).validate(&value).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn the_icons_of_file_entries_are_the_images_of_the_settings() {
        let module = descriptor(vec![field("files", file_entries())]);
        let mut settings = json!({ "files": [
            "~/legacy.yaml",
            { "path": "~/a.yaml", "icon": "a.png" },
            { "path": "~/b.yaml" },
            { "path": "~/c.yaml", "icon": "c.svg" },
        ] });
        assert_eq!(module.settings_images(&settings), ["a.png", "c.svg"]);
        module.replace_settings_images(&mut settings, |name| {
            (name == "a.png").then(|| "renamed.png".to_owned())
        });
        assert_eq!(
            settings,
            json!({ "files": [
                "~/legacy.yaml",
                { "path": "~/a.yaml", "icon": "renamed.png" },
                { "path": "~/b.yaml" },
                { "path": "~/c.yaml" },
            ] })
        );
        assert_eq!(module.settings_images(&json!({})), Vec::<String>::new());
    }
}
