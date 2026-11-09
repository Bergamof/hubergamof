//! The settings of a Docker instance: the compose files, as stored and as read.

use hubergamof_module_api::{
    EntryNaming, FileChooser, FileEntry, FileFilter, ModuleError, SettingField, SettingKind,
};
use serde::Deserialize;
use serde_json::Value;

use super::descriptor;
use super::model::DeclaredFile;
use crate::paths::expand_user_home;

pub(super) const COMPOSE_FILES_KEY: &str = "composeFiles";
/// Most compose files of an instance.
pub(super) const MAX_COMPOSE_FILES: usize = 50;
/// Longest name given to a compose file, in characters.
pub(super) const MAX_PROJECT_NAME_CHARS: usize = 40;

/// Settings of a Docker instance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct DockerSettings {
    /// Compose files offered even when their project is down.
    pub(super) compose_files: Vec<FileEntry>,
}

/// What an instance reads of its settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ParsedSettings {
    pub(super) compose_files: Vec<DeclaredFile>,
}

impl ParsedSettings {
    /// Checks the types, reads them, and makes every path absolute: `~/`
    /// stands for the home directory, any other path must be absolute
    /// (compose files are given to `docker compose`, run from the manager's
    /// directory). A blank name counts as none.
    pub(super) fn parse(settings: &Value) -> Result<Self, ModuleError> {
        let parsed: DockerSettings = descriptor().parse_settings(settings)?;
        let compose_files = parsed
            .compose_files
            .into_iter()
            .map(|entry| {
                let path = expand_user_home(entry.path.as_ref())
                    .filter(|file| file.is_absolute())
                    .ok_or_else(|| {
                        ModuleError::InvalidSettings(format!(
                            "`{COMPOSE_FILES_KEY}`: `{}` is not an absolute path",
                            entry.path
                        ))
                    })?;
                Ok(DeclaredFile {
                    path,
                    name: entry
                        .name
                        .map(|name| name.trim().to_owned())
                        .filter(|name| !name.is_empty()),
                    icon: entry.icon,
                })
            })
            .collect::<Result<_, ModuleError>>()?;
        Ok(Self { compose_files })
    }

    /// Names of the icons given to the files.
    pub(super) fn icon_names(&self) -> impl Iterator<Item = &str> {
        self.compose_files
            .iter()
            .filter_map(|file| file.icon.as_deref())
    }
}

/// The setting listing the compose files.
pub(super) fn compose_files_field() -> SettingField {
    SettingField {
        key: COMPOSE_FILES_KEY.into(),
        label: "Fichiers compose".into(),
        help: Some(
            "Fichiers compose.yaml, avec un nom et une icône au choix, proposés \
                 même quand le projet est arrêté (après un down)."
                .into(),
        ),
        kind: SettingKind::FileEntryList {
            default: Vec::new(),
            max_entries: MAX_COMPOSE_FILES,
            max_name_chars: MAX_PROJECT_NAME_CHARS,
            chooser: Some(FileChooser {
                title: "Choisir un fichier compose".into(),
                filters: vec![
                    FileFilter {
                        name: "Fichiers compose".into(),
                        globs: vec!["*.yaml".into(), "*.yml".into()],
                    },
                    FileFilter {
                        name: "Tous les fichiers".into(),
                        globs: vec!["*".into()],
                    },
                ],
            }),
            path_placeholder: "/chemin/vers/compose.yaml".into(),
            name_hint: "Vide : nom du projet compose.".into(),
            default_name: EntryNaming::FolderName,
        },
    }
}
