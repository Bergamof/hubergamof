//! `modules.json` at the root: the modules that live in the application.

use hubergamof_protocol::AppModulesConfig;

use super::mouse::{Document, load_or_default, save_document};
use super::{APP_MODULES_FILE, ConfigStore, ConfigStoreError, Loaded};

impl Document for AppModulesConfig {
    fn revision(&self) -> u64 {
        self.revision
    }
    fn set_revision(&mut self, revision: u64) {
        self.revision = revision;
    }
    fn check(&self) -> Result<(), ConfigStoreError> {
        Ok(self.validate()?)
    }
}

impl ConfigStore {
    /// Loads the configuration of the application modules, creating it empty
    /// when missing and setting an invalid file aside.
    ///
    /// # Errors
    /// [`ConfigStoreError::Io`].
    pub fn load_app_modules(&self) -> Result<Loaded<AppModulesConfig>, ConfigStoreError> {
        let _guard = self.lock();
        let path = self.root.join(APP_MODULES_FILE);
        load_or_default(&path, Document::check, AppModulesConfig::empty)
    }

    /// Saves the configuration of the application modules if it is valid and
    /// based on the current revision; returns it with its new revision.
    ///
    /// # Errors
    /// [`ConfigStoreError::StaleRevision`], a validation error, or an I/O or
    /// parse error on the current file.
    pub fn save_app_modules(
        &self,
        config: AppModulesConfig,
    ) -> Result<AppModulesConfig, ConfigStoreError> {
        let _guard = self.lock();
        save_document(&self.root.join(APP_MODULES_FILE), config)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use hubergamof_protocol::ModuleInstance;
    use serde_json::json;

    use super::*;
    use crate::config_store::LoadOutcome;
    use crate::testing::builtin_registry;

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path(), Arc::new(builtin_registry()));
        (dir, store)
    }

    fn pomodoro() -> ModuleInstance {
        ModuleInstance {
            id: "app-pomodoro".into(),
            kind: "pomodoro".into(),
            label: None,
            image: None,
            settings: json!({ "workMinutes": 30 }),
        }
    }

    #[test]
    fn a_new_store_has_an_empty_configuration_on_disk() {
        let (dir, store) = store();
        let loaded = store.load_app_modules().unwrap();
        assert_eq!(loaded.outcome, LoadOutcome::CreatedDefault);
        assert_eq!(loaded.config, AppModulesConfig::empty());
        assert!(dir.path().join(APP_MODULES_FILE).is_file());
        assert_eq!(
            store.load_app_modules().unwrap().outcome,
            LoadOutcome::Loaded
        );
    }

    #[test]
    fn saving_bumps_the_revision_and_is_read_back() {
        let (_dir, store) = store();
        let mut config = store.load_app_modules().unwrap().config;
        config.modules.push(pomodoro());
        let saved = store.save_app_modules(config).unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(store.load_app_modules().unwrap().config, saved);
    }

    #[test]
    fn an_outdated_revision_is_refused() {
        let (_dir, store) = store();
        let config = store.load_app_modules().unwrap().config;
        store.save_app_modules(config.clone()).unwrap();
        assert!(matches!(
            store.save_app_modules(config),
            Err(ConfigStoreError::StaleRevision { .. })
        ));
    }

    #[test]
    fn an_invalid_file_is_set_aside_and_replaced_by_the_default() {
        let (dir, store) = store();
        let path = dir.path().join(APP_MODULES_FILE);
        fs::write(&path, br#"{"schemaVersion":1,"modules":[{"id":"../x","kind":"pomodoro","label":null,"image":null,"settings":{}}],"revision":0}"#).unwrap();
        let loaded = store.load_app_modules().unwrap();
        assert_eq!(loaded.config, AppModulesConfig::empty());
        assert_ne!(loaded.outcome, LoadOutcome::Loaded);
        let aside = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains("invalid"));
        assert!(aside, "the refused file is kept");
    }

    #[test]
    fn an_invalid_configuration_is_not_saved() {
        let (_dir, store) = store();
        let mut config = store.load_app_modules().unwrap().config;
        config.modules.push(pomodoro());
        config.modules.push(pomodoro());
        assert!(matches!(
            store.save_app_modules(config),
            Err(ConfigStoreError::InvalidAppModules(_))
        ));
    }
}
