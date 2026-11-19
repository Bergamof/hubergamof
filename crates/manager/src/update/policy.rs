//! The user's choice about installing updates, kept in `updates.json` next
//! to the device configurations.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::config_store::write_bytes_atomically;

/// File name, in the configuration directory.
pub const POLICY_FILE: &str = "updates.json";

/// Whether a newer release is installed as soon as it is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdatePolicy {
    /// Install automatically (the default: the point of the feature).
    pub auto_install: bool,
}

impl Default for UpdatePolicy {
    fn default() -> Self {
        Self { auto_install: true }
    }
}

/// Where the policy is kept.
pub trait PolicyStore: Send + Sync {
    /// The saved policy; the default when there is none or it is unreadable
    /// (a damaged file must not stop the manager).
    fn load(&self) -> UpdatePolicy;

    /// Saves `policy`; failures are logged, the choice then lasts until the
    /// next start.
    fn save(&self, policy: UpdatePolicy);
}

/// The policy in a JSON file of the configuration directory.
pub struct FilePolicyStore {
    path: PathBuf,
}

impl FilePolicyStore {
    /// Policy kept in `config_dir/updates.json`.
    #[must_use]
    pub fn in_dir(config_dir: &std::path::Path) -> Self {
        Self {
            path: config_dir.join(POLICY_FILE),
        }
    }
}

impl PolicyStore for FilePolicyStore {
    fn load(&self) -> UpdatePolicy {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                tracing::warn!(%error, path = %self.path.display(), "unreadable update policy, using the default");
                UpdatePolicy::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => UpdatePolicy::default(),
            Err(error) => {
                tracing::warn!(%error, path = %self.path.display(), "cannot read the update policy");
                UpdatePolicy::default()
            }
        }
    }

    fn save(&self, policy: UpdatePolicy) {
        let outcome = serde_json::to_vec_pretty(&policy)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                write_bytes_atomically(&self.path, &bytes).map_err(|error| error.to_string())
            });
        if let Err(error) = outcome {
            tracing::warn!(%error, "cannot save the update policy");
        }
    }
}

/// A policy kept in memory (tests, `--demo`).
#[derive(Default)]
pub struct MemoryPolicyStore(Mutex<UpdatePolicy>);

impl MemoryPolicyStore {
    /// A store holding `policy`.
    #[must_use]
    pub fn holding(policy: UpdatePolicy) -> Self {
        Self(Mutex::new(policy))
    }
}

impl PolicyStore for MemoryPolicyStore {
    fn load(&self) -> UpdatePolicy {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn save(&self, policy: UpdatePolicy) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = policy;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installing_automatically_is_the_default() {
        assert!(UpdatePolicy::default().auto_install);
        let store = FilePolicyStore::in_dir(tempfile::tempdir().unwrap().path());
        assert!(store.load().auto_install);
    }

    #[test]
    fn a_saved_choice_is_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePolicyStore::in_dir(dir.path());
        store.save(UpdatePolicy {
            auto_install: false,
        });
        assert!(!FilePolicyStore::in_dir(dir.path()).load().auto_install);
        let text = std::fs::read_to_string(dir.path().join(POLICY_FILE)).unwrap();
        assert!(text.contains("autoInstall"), "{text}");
    }

    #[test]
    fn a_damaged_file_gives_the_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(POLICY_FILE), "{ not json").unwrap();
        assert!(FilePolicyStore::in_dir(dir.path()).load().auto_install);
    }

    #[test]
    fn unknown_or_missing_fields_do_not_matter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(POLICY_FILE), r#"{ "other": 1 }"#).unwrap();
        assert!(FilePolicyStore::in_dir(dir.path()).load().auto_install);
    }

    #[test]
    fn saving_into_a_missing_directory_creates_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePolicyStore::in_dir(&dir.path().join("config"));
        store.save(UpdatePolicy {
            auto_install: false,
        });
        assert!(!store.load().auto_install);
    }

    #[test]
    fn the_memory_store_keeps_the_last_choice() {
        let store = MemoryPolicyStore::default();
        store.save(UpdatePolicy {
            auto_install: false,
        });
        assert!(!store.load().auto_install);
    }
}
