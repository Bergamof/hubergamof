//! The Git configuration of the computer, as the web interface sees it: what
//! HuBergamof saved (`config/git.json`: identity, settings, aliases and
//! identities by folder) and the managed values that Git finds elsewhere.
//!
//! The computer is not watched: it is read when a client asks
//! ([`GitConfigHub::refresh`]) and after an update, which every client hears
//! through [`GitConfigHub::subscribe`].
//!
//! - [`render`]: the configuration as the files Git reads.
//! - [`apply`]: those files on the computer, and the include that loads them.
//! - [`system`]: what `git config --list` finds besides.

mod apply;
mod render;
mod system;

use std::sync::Arc;

use hubergamof_protocol::{ErrorCode, GitConfig, GitSystemEntry};
use thiserror::Error;
use tokio::sync::{Semaphore, broadcast};

pub use apply::{GitFiles, GitFilesError};
pub use render::UnwritableValue;
pub use system::{MAX_SYSTEM_GIT_ENTRIES, SystemRead};

use crate::config_store::{ConfigStore, ConfigStoreError, LoadOutcome};

/// The Git configuration file of the demo's home folder, so that the interface
/// has something to list.
const DEMO_GIT_CONFIG: &str = "[user]\n\tname = Ada Lovelace\n\temail = ada@example.org\n\
                               [alias]\n\tst = status --short\n\tlg = log --oneline --graph\n";

/// Snapshots a slow client may miss before it is told only the latest one.
const CHANGE_BACKLOG: usize = 8;

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
#[derive(Debug, Error)]
pub enum GitError {
    /// The configuration was refused or could not be stored.
    #[error(transparent)]
    Store(#[from] ConfigStoreError),
    /// The configuration was saved but the files could not be written.
    #[error("the configuration is saved but not applied: {0}")]
    Files(#[from] GitFilesError),
    /// The home folder is unknown: nothing can be applied.
    #[error("the home folder of the user is unknown: the Git configuration cannot be applied")]
    NoHome,
    /// The blocking task failed.
    #[error("the task failed: {0}")]
    Task(String),
}

impl GitError {
    /// Error code reported to the UI.
    #[must_use]
    pub fn error_code(&self) -> ErrorCode {
        match self {
            Self::Store(error) => error.error_code(),
            Self::Files(_) | Self::NoHome | Self::Task(_) => ErrorCode::Internal,
        }
    }
}

struct Inner {
    store: ConfigStore,
    /// Kept so that a demo's temporary home folder lives as long as the hub.
    _demo_home: Option<tempfile::TempDir>,
    files: Option<GitFiles>,
    changes: broadcast::Sender<Arc<GitSnapshot>>,
    /// One request at a time: what reaches the clients follows the order of
    /// the revisions, and the blocking threads that the renderer and the store
    /// share are not flooded by a client sending requests in a burst.
    requests: Semaphore,
}

/// The one place that reads and changes the Git configuration.
#[derive(Clone)]
pub struct GitConfigHub {
    inner: Arc<Inner>,
}

impl GitConfigHub {
    /// A hub saving in `store` and applying to `files`.
    #[must_use]
    pub fn new(store: ConfigStore, files: Option<GitFiles>) -> Self {
        Self::build(store, files, None)
    }

    fn build(
        store: ConfigStore,
        files: Option<GitFiles>,
        demo_home: Option<tempfile::TempDir>,
    ) -> Self {
        let (changes, _) = broadcast::channel(CHANGE_BACKLOG);
        Self {
            inner: Arc::new(Inner {
                store,
                _demo_home: demo_home,
                files,
                changes,
                requests: Semaphore::new(1),
            }),
        }
    }

    /// A hub for `--demo`: the Git files are those of a temporary home folder,
    /// which holds a few values, so that the real ones are never read nor
    /// written.
    ///
    /// # Errors
    /// The failure to create the folder or its Git configuration.
    pub fn demo(store: ConfigStore) -> std::io::Result<Self> {
        let home = tempfile::tempdir()?;
        std::fs::write(home.path().join(".gitconfig"), DEMO_GIT_CONFIG)?;
        let files = GitFiles::new(home.path().to_owned(), false);
        Ok(Self::build(store, Some(files), Some(home)))
    }

    /// The snapshots sent after each update, to every subscriber.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<GitSnapshot>> {
        self.inner.changes.subscribe()
    }

    /// The saved configuration and what the computer holds besides, read now.
    ///
    /// # Errors
    /// [`GitError::Store`] when the configuration cannot be read.
    pub async fn refresh(&self) -> Result<GitSnapshot, GitError> {
        let _request = self.request_slot().await;
        let inner = Arc::clone(&self.inner);
        let config = run_blocking(move || inner.load()).await?;
        Ok(self.inner.snapshot_of(config).await)
    }

    /// Saves `config` (based on the current revision), puts what is enabled in
    /// it on the computer and gives the result to every subscriber, the author
    /// included. When the files cannot be written the configuration stays
    /// saved and subscribers are told it all the same: the revision moved.
    ///
    /// # Errors
    /// [`GitError::Store`] (stale revision, invalid configuration: nothing
    /// changed), or [`GitError::Files`] / [`GitError::NoHome`] (saved, not
    /// applied).
    pub async fn update(&self, config: GitConfig) -> Result<(), GitError> {
        let _request = self.request_slot().await;
        let inner = Arc::clone(&self.inner);
        let (saved, applied) = run_blocking(move || inner.save_and_apply(config)).await?;
        let snapshot = self.inner.snapshot_of(saved).await;
        // Nobody listening is fine.
        let _ = self.inner.changes.send(Arc::new(snapshot));
        applied
    }

    async fn request_slot(&self) -> tokio::sync::SemaphorePermit<'_> {
        // The semaphore lives as long as the hub and is never closed.
        self.inner
            .requests
            .acquire()
            .await
            .unwrap_or_else(|_| unreachable!("the request semaphore is never closed"))
    }
}

impl Inner {
    /// The saved configuration. A file that was invalid is set aside and
    /// replaced by an empty configuration: what it had put on the computer is
    /// removed too, so that the page shows what the computer holds.
    fn load(&self) -> Result<GitConfig, GitError> {
        let loaded = self.store.load_git()?;
        if matches!(loaded.outcome, LoadOutcome::ReplacedInvalid { .. })
            && let Some(files) = &self.files
            && let Err(error) = files.apply(&loaded.config)
        {
            tracing::warn!(%error, "what an invalid Git configuration had applied was not removed");
        }
        Ok(loaded.config)
    }

    /// Saves `config`; the outcome of putting it on the computer comes with
    /// the saved configuration, since it is saved either way.
    fn save_and_apply(
        &self,
        config: GitConfig,
    ) -> Result<(GitConfig, Result<(), GitError>), GitError> {
        let saved = self.store.save_git(config)?;
        let applied = match &self.files {
            Some(files) => files.apply(&saved).map_err(GitError::from),
            None => Err(GitError::NoHome),
        };
        Ok((saved, applied))
    }

    async fn snapshot_of(&self, config: GitConfig) -> GitSnapshot {
        // Without a home folder nothing is looked for, which is not `git` missing.
        let read = match &self.files {
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

async fn run_blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, GitError> + Send + 'static,
) -> Result<T, GitError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| GitError::Task(error.to_string()))?
}

#[cfg(test)]
mod tests;
