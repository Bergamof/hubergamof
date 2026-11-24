//! The Git configuration of the computer, as the web interface sees it: what
//! HuBergamof saved (`config/git.json`: identity, settings, aliases and
//! identities by folder) and the managed values that Git finds elsewhere. It is
//! a feature of the machine like the others, see [`Hub`].
//!
//! - [`render`]: the configuration as the files Git reads.
//! - [`apply`]: those files on the computer, and the include that loads them.
//! - [`claim`]: the aliases taken over, commented out where they came from.
//! - [`system`]: what `git config --list` finds besides.

mod apply;
mod claim;
mod render;
mod system;

use std::sync::Arc;

use hubergamof_protocol::{GitConfig, GitSystemEntry};

pub use apply::{GitFiles, GitFilesError};
pub use render::UnwritableValue;
pub use system::{MAX_SYSTEM_GIT_ENTRIES, SystemRead};

use super::hub::{Feature, Hub, HubError};
use crate::config_store::{ConfigStore, ConfigStoreError, Loaded};

/// The Git configuration file of the demo's home folder, so that the interface
/// has something to list.
const DEMO_GIT_CONFIG: &str = "[user]\n\tname = Ada Lovelace\n\temail = ada@example.org\n\
                               [alias]\n\tst = status --short\n\tlg = log --oneline --graph\n";

/// The Git configuration and what the computer holds besides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitSnapshot {
    /// The saved configuration.
    pub config: GitConfig,
    /// The managed values that Git finds in other files.
    pub system_entries: Vec<GitSystemEntry>,
    /// `false` only when the `git` program could not be run.
    pub git_available: bool,
}

/// Why a request about Git failed.
pub type GitError = HubError<GitFilesError>;

/// The Git configuration of the machine.
pub struct GitConfiguration;

impl Feature for GitConfiguration {
    type Config = GitConfig;
    type Target = GitFiles;
    type Snapshot = GitSnapshot;
    type ApplyError = GitFilesError;

    fn load(store: &ConfigStore) -> Result<Loaded<GitConfig>, ConfigStoreError> {
        store.load_git()
    }

    fn save(store: &ConfigStore, config: GitConfig) -> Result<GitConfig, ConfigStoreError> {
        store.save_git(config)
    }

    fn apply(files: &GitFiles, config: &GitConfig) -> Result<(), GitFilesError> {
        files.apply(config)
    }

    async fn snapshot(files: Option<Arc<GitFiles>>, config: GitConfig) -> GitSnapshot {
        // Without a home folder nothing is looked for, which is not `git` missing.
        let read = match files {
            Some(files) => files.read_system().await,
            None => SystemRead {
                entries: Vec::new(),
                git_available: true,
            },
        };
        GitSnapshot {
            config,
            system_entries: read.entries,
            git_available: read.git_available,
        }
    }
}

/// The one place that reads and changes the Git configuration.
pub type GitConfigHub = Hub<GitConfiguration>;

impl GitConfigHub {
    /// A hub for `--demo`: the Git files are those of a temporary home folder,
    /// which holds a few values, so that the real ones are never read nor
    /// written.
    ///
    /// # Errors
    /// The failure to create the folder or its Git configuration.
    pub fn demo(store: ConfigStore) -> std::io::Result<Self> {
        Self::with_demo_home(store, |home| {
            std::fs::write(home.join(".gitconfig"), DEMO_GIT_CONFIG)?;
            Ok(GitFiles::new(home.to_owned(), false))
        })
    }
}

#[cfg(test)]
mod tests;
