//! `machine.json` at the root: the configuration of the computer (shell aliases).

use hubergamof_protocol::MachineConfig;

use super::mouse::{Document, load_or_default, save_document};
use super::{ConfigStore, ConfigStoreError, Loaded, MACHINE_FILE};

impl Document for MachineConfig {
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
    /// Loads the configuration of the machine, creating it empty when missing
    /// and setting an invalid file aside.
    ///
    /// # Errors
    /// [`ConfigStoreError::Io`].
    pub fn load_machine(&self) -> Result<Loaded<MachineConfig>, ConfigStoreError> {
        let _guard = self.lock();
        let path = self.root.join(MACHINE_FILE);
        load_or_default(&path, Document::check, MachineConfig::empty)
    }

    /// Saves the configuration of the machine if it is valid and based on the
    /// current revision; returns it with its new revision.
    ///
    /// # Errors
    /// [`ConfigStoreError::StaleRevision`], a validation error, or an I/O or
    /// parse error on the current file.
    pub fn save_machine(&self, config: MachineConfig) -> Result<MachineConfig, ConfigStoreError> {
        let _guard = self.lock();
        save_document(&self.root.join(MACHINE_FILE), config)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use hubergamof_protocol::ShellAlias;

    use super::*;
    use crate::config_store::LoadOutcome;
    use crate::testing::builtin_registry;

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path(), Arc::new(builtin_registry()));
        (dir, store)
    }

    fn alias(name: &str) -> ShellAlias {
        ShellAlias {
            name: name.into(),
            command: "ls -la".into(),
            enabled: true,
        }
    }

    #[test]
    fn a_new_store_has_an_empty_configuration_on_disk() {
        let (dir, store) = store();
        let loaded = store.load_machine().unwrap();
        assert_eq!(loaded.outcome, LoadOutcome::CreatedDefault);
        assert_eq!(loaded.config, MachineConfig::empty());
        assert!(dir.path().join(MACHINE_FILE).is_file());
        assert_eq!(store.load_machine().unwrap().outcome, LoadOutcome::Loaded);
    }

    #[test]
    fn saving_bumps_the_revision_and_is_read_back() {
        let (_dir, store) = store();
        let mut config = store.load_machine().unwrap().config;
        config.aliases.push(alias("ll"));
        let saved = store.save_machine(config).unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(store.load_machine().unwrap().config, saved);
    }

    #[test]
    fn an_outdated_revision_is_refused() {
        let (_dir, store) = store();
        let config = store.load_machine().unwrap().config;
        store.save_machine(config.clone()).unwrap();
        assert!(matches!(
            store.save_machine(config),
            Err(ConfigStoreError::StaleRevision { .. })
        ));
    }

    #[test]
    fn an_invalid_file_is_set_aside_and_replaced_by_the_default() {
        let (dir, store) = store();
        let path = dir.path().join(MACHINE_FILE);
        fs::write(
            &path,
            br#"{"schemaVersion":1,"aliases":[{"name":"a b","command":"ls","enabled":true}],"revision":0}"#,
        )
        .unwrap();
        let loaded = store.load_machine().unwrap();
        assert_eq!(loaded.config, MachineConfig::empty());
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
        let mut config = store.load_machine().unwrap().config;
        config.aliases.push(alias("ll"));
        config.aliases.push(alias("ll"));
        assert!(matches!(
            store.save_machine(config),
            Err(ConfigStoreError::InvalidMachine(_))
        ));
    }
}
