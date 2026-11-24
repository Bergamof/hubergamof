//! `ssh.json` at the root: the SSH hosts of the computer.

use hubergamof_protocol::SshConfig;

use super::mouse::{Document, load_or_default, save_document};
use super::{ConfigStore, ConfigStoreError, Loaded, SSH_FILE};

impl Document for SshConfig {
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
    /// Loads the SSH configuration, creating it empty when missing and
    /// setting an invalid file aside.
    ///
    /// # Errors
    /// [`ConfigStoreError::Io`].
    pub fn load_ssh(&self) -> Result<Loaded<SshConfig>, ConfigStoreError> {
        let _guard = self.lock();
        let path = self.root.join(SSH_FILE);
        load_or_default(&path, Document::check, SshConfig::empty)
    }

    /// Saves the SSH configuration if it is valid and based on the current
    /// revision; returns it with its new revision.
    ///
    /// # Errors
    /// [`ConfigStoreError::StaleRevision`], a validation error, or an I/O or
    /// parse error on the current file.
    pub fn save_ssh(&self, config: SshConfig) -> Result<SshConfig, ConfigStoreError> {
        let _guard = self.lock();
        save_document(&self.root.join(SSH_FILE), config)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use hubergamof_protocol::SshHost;

    use super::*;
    use crate::config_store::LoadOutcome;
    use crate::testing::builtin_registry;

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path(), Arc::new(builtin_registry()));
        (dir, store)
    }

    fn host(alias: &str, proxy_jump: &str) -> SshHost {
        SshHost {
            alias: alias.into(),
            host_name: String::new(),
            user: String::new(),
            port: None,
            identity_file: String::new(),
            proxy_jump: proxy_jump.into(),
            forward_agent: false,
            enabled: true,
            options: Vec::new(),
        }
    }

    #[test]
    fn a_new_store_has_an_empty_configuration_on_disk() {
        let (dir, store) = store();
        let loaded = store.load_ssh().unwrap();
        assert_eq!(loaded.outcome, LoadOutcome::CreatedDefault);
        assert_eq!(loaded.config, SshConfig::empty());
        assert!(dir.path().join(SSH_FILE).is_file());
    }

    #[test]
    fn saving_bumps_the_revision_and_an_outdated_one_is_refused() {
        let (_dir, store) = store();
        let mut config = store.load_ssh().unwrap().config;
        config.hosts.push(host("prod", ""));
        let saved = store.save_ssh(config.clone()).unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(store.load_ssh().unwrap().config, saved);
        assert!(matches!(
            store.save_ssh(config),
            Err(ConfigStoreError::StaleRevision { .. })
        ));
    }

    #[test]
    fn a_jump_that_would_be_an_option_is_not_saved() {
        let (_dir, store) = store();
        let mut config = store.load_ssh().unwrap().config;
        config.hosts.push(host("pwn", "-oProxyCommand=evil"));
        let error = store.save_ssh(config).unwrap_err();
        assert!(matches!(error, ConfigStoreError::InvalidSsh(_)), "{error}");
    }

    #[test]
    fn an_invalid_file_is_set_aside_and_replaced_by_the_default() {
        let (dir, store) = store();
        let mut hostile = SshConfig::empty();
        hostile.hosts.push(host("pwn", "-oProxyCommand=evil"));
        fs::write(
            dir.path().join(SSH_FILE),
            serde_json::to_vec(&hostile).unwrap(),
        )
        .unwrap();
        let loaded = store.load_ssh().unwrap();
        assert_eq!(loaded.config, SshConfig::empty());
        assert_ne!(loaded.outcome, LoadOutcome::Loaded);
    }
}
