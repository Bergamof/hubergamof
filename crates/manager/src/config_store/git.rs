//! `git.json` at the root: the Git configuration of the computer.

use hubergamof_protocol::GitConfig;

use super::mouse::{Document, load_or_default, save_document};
use super::{ConfigStore, ConfigStoreError, GIT_FILE, Loaded};

impl Document for GitConfig {
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
    /// Loads the Git configuration, creating it empty when missing and
    /// setting an invalid file aside.
    ///
    /// # Errors
    /// [`ConfigStoreError::Io`].
    pub fn load_git(&self) -> Result<Loaded<GitConfig>, ConfigStoreError> {
        let _guard = self.lock();
        let path = self.root.join(GIT_FILE);
        load_or_default(&path, Document::check, GitConfig::empty)
    }

    /// Saves the Git configuration if it is valid and based on the current
    /// revision; returns it with its new revision.
    ///
    /// # Errors
    /// [`ConfigStoreError::StaleRevision`], a validation error, or an I/O or
    /// parse error on the current file.
    pub fn save_git(&self, config: GitConfig) -> Result<GitConfig, ConfigStoreError> {
        let _guard = self.lock();
        save_document(&self.root.join(GIT_FILE), config)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use hubergamof_protocol::GitAlias;

    use super::*;
    use crate::config_store::LoadOutcome;
    use crate::testing::builtin_registry;

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path(), Arc::new(builtin_registry()));
        (dir, store)
    }

    fn alias(name: &str, command: &str) -> GitAlias {
        GitAlias {
            name: name.into(),
            command: command.into(),
            enabled: true,
        }
    }

    #[test]
    fn a_new_store_has_an_empty_configuration_on_disk() {
        let (dir, store) = store();
        let loaded = store.load_git().unwrap();
        assert_eq!(loaded.outcome, LoadOutcome::CreatedDefault);
        assert_eq!(loaded.config, GitConfig::empty());
        assert!(dir.path().join(GIT_FILE).is_file());
        assert_eq!(store.load_git().unwrap().outcome, LoadOutcome::Loaded);
    }

    #[test]
    fn saving_bumps_the_revision_and_is_read_back() {
        let (_dir, store) = store();
        let mut config = store.load_git().unwrap().config;
        config.aliases.push(alias("co", "checkout"));
        config.identity.email = "ada@example.org".into();
        let saved = store.save_git(config).unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(store.load_git().unwrap().config, saved);
    }

    #[test]
    fn an_outdated_revision_is_refused() {
        let (_dir, store) = store();
        let config = store.load_git().unwrap().config;
        store.save_git(config.clone()).unwrap();
        assert!(matches!(
            store.save_git(config),
            Err(ConfigStoreError::StaleRevision { .. })
        ));
    }

    #[test]
    fn a_shell_alias_is_not_saved() {
        let (_dir, store) = store();
        let mut config = store.load_git().unwrap().config;
        config.aliases.push(alias("pwn", "!rm -rf ~"));
        let error = store.save_git(config).unwrap_err();
        assert!(matches!(error, ConfigStoreError::InvalidGit(_)), "{error}");
        assert_eq!(store.load_git().unwrap().config.revision, 0);
    }

    #[test]
    fn an_invalid_file_is_set_aside_and_replaced_by_the_default() {
        let (dir, store) = store();
        let path = dir.path().join(GIT_FILE);
        let mut hostile = GitConfig::empty();
        hostile.aliases.push(alias("pwn", "!rm -rf ~"));
        fs::write(&path, serde_json::to_vec(&hostile).unwrap()).unwrap();
        let loaded = store.load_git().unwrap();
        assert_eq!(loaded.config, GitConfig::empty());
        assert_ne!(loaded.outcome, LoadOutcome::Loaded);
        let aside = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains("invalid"));
        assert!(aside, "the refused file is kept");
    }
}
