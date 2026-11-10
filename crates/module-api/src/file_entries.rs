//! Settings holding files: the entries of the list, as the user named them,
//! and the description of the dialog that chooses them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{is_safe_image_name, is_setting_path};

/// A file of a [`crate::SettingKind::FileEntryList`], as the user named it.
///
/// A setting saved before names and icons existed holds plain paths: they
/// are still read, as entries with neither.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FileEntry {
    /// Absolute path, or starting with `~/` (see [`crate::SettingKind::Path`]).
    pub path: String,
    /// Name shown for the file, within the `max_name_chars` of its list; the
    /// module names the file itself when absent.
    pub name: Option<String>,
    /// Name of a stored image of the device (see
    /// `ModuleInstance::image`); the module draws its own icon when absent.
    pub icon: Option<String>,
}

/// How a file entry is stored: a path alone (older settings), or an entry.
#[derive(Deserialize)]
#[serde(untagged)]
enum StoredFileEntry {
    Path(String),
    Entry {
        path: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        icon: Option<String>,
    },
}

// Written by hand rather than with `#[serde(from)]`, which the TypeScript
// generation does not understand.
impl<'de> Deserialize<'de> for FileEntry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        StoredFileEntry::deserialize(deserializer).map(Self::from)
    }
}

impl From<StoredFileEntry> for FileEntry {
    fn from(stored: StoredFileEntry) -> Self {
        match stored {
            StoredFileEntry::Path(path) => Self {
                path,
                name: None,
                icon: None,
            },
            StoredFileEntry::Entry { path, name, icon } => Self { path, name, icon },
        }
    }
}

/// The file dialog of a list of files: what it is titled and shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FileChooser {
    /// Title of the dialog.
    pub title: String,
    /// Kinds of files the dialog offers, the first one selected.
    pub filters: Vec<FileFilter>,
}

/// A kind of file of a [`FileChooser`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FileFilter {
    /// Name of the kind, as the dialog shows it.
    pub name: String,
    /// Glob patterns of its files (`*.yaml`).
    pub globs: Vec<String>,
}

/// What an entry without a name is called in the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum EntryNaming {
    /// The name of the folder of the file (a compose project).
    FolderName,
    /// The name of the file.
    FileName,
}

/// Characters that scramble what a name shows on a device or in logs:
/// control characters and the bidirectional overrides (U+202A–202E,
/// U+2066–2069).
#[must_use]
pub fn is_hidden_control(c: char) -> bool {
    c.is_control() || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Checks a stored list of file entries against the limits of its setting.
pub(crate) fn check_file_entries(
    value: &Value,
    max_entries: usize,
    max_name_chars: usize,
) -> Result<(), String> {
    let entries = FileEntry::list(value)?;
    if entries.len() > max_entries {
        return Err(format!("at most {max_entries} files"));
    }
    entries
        .iter()
        .try_for_each(|entry| check_entry(entry, max_name_chars))
}

impl FileEntry {
    /// The entries a stored value holds.
    fn list(value: &Value) -> Result<Vec<Self>, String> {
        Vec::<Self>::deserialize(value)
            .map_err(|_| "expected a list of files, each with a `path`".to_owned())
    }
}

fn check_entry(entry: &FileEntry, max_name_chars: usize) -> Result<(), String> {
    if !is_setting_path(&entry.path) {
        return Err(format!(
            "`{}` is neither absolute nor in the home directory (`~/…`)",
            entry.path
        ));
    }
    if let Some(name) = &entry.name {
        if name.chars().count() > max_name_chars {
            return Err(format!(
                "a file name is longer than {max_name_chars} characters"
            ));
        }
        if name.chars().any(is_hidden_control) {
            return Err("a file name holds control or bidirectional characters".to_owned());
        }
    }
    match &entry.icon {
        Some(icon) if !is_safe_image_name(icon) => {
            Err("a file `icon` must be the name of a stored image".to_owned())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn an_entry_is_read_from_an_object_or_from_a_plain_path() {
        let entries = FileEntry::list(&json!([
            "~/old.yaml",
            { "path": "/a.yaml" },
            { "path": "/b.yaml", "name": "B", "icon": "b.png", "other": 1 },
            { "path": "/c.yaml", "name": null }
        ]))
        .unwrap();
        let named = |path: &str, name: Option<&str>, icon: Option<&str>| FileEntry {
            path: path.into(),
            name: name.map(Into::into),
            icon: icon.map(Into::into),
        };
        assert_eq!(
            entries,
            [
                named("~/old.yaml", None, None),
                named("/a.yaml", None, None),
                named("/b.yaml", Some("B"), Some("b.png")),
                named("/c.yaml", None, None),
            ]
        );
        for invalid in [json!("a"), json!([1]), json!([{}]), json!([{ "path": 3 }])] {
            assert!(FileEntry::list(&invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn hidden_controls_are_the_ones_that_scramble_a_name() {
        for c in ['\n', '\u{7}', '\u{202E}', '\u{2066}'] {
            assert!(is_hidden_control(c), "{c:?}");
        }
        for c in ['a', 'é', ' ', '😀'] {
            assert!(!is_hidden_control(c), "{c:?}");
        }
    }
}
