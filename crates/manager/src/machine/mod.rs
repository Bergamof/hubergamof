//! The machine HuBergamof runs on, as the web interface sees it: its
//! configuration (`config/machine.json`, for now the shell aliases) and the
//! aliases already in the shell startup files.
//!
//! The computer is not watched: the aliases of the startup files are read when
//! a client asks ([`MachineHub::refresh`]) and after an update, which every
//! client hears through [`MachineHub::subscribe`].
//!
//! - [`git`]: the Git configuration, which has its own hub.
//! - [`shell`]: the files, read without running a shell, written atomically.
//! - [`aliases`]: parsing and quoting.
//! - [`rc_block`]: the block that sources the managed file.

mod aliases;
mod block;
mod files;
pub mod git;
mod rc_block;
mod shell;

use std::sync::{Arc, Mutex, PoisonError};

use hubergamof_protocol::{ErrorCode, MachineConfig, SystemAlias};
use thiserror::Error;
use tokio::sync::{Semaphore, broadcast};

pub use aliases::UnwritableAlias;
pub use files::{FileError, MAX_STARTUP_FILE_BYTES};
pub use shell::{MAX_SYSTEM_ALIASES, MachineShell, STARTUP_FILES, ShellError};

use crate::config_store::{ConfigStore, ConfigStoreError};

/// Aliases the demo's startup file holds, so that the interface has some to list.
const DEMO_STARTUP_FILE: &str = "alias gs='git status'\nalias gl='git log --oneline'\n";

/// Snapshots a slow client may miss before it is told only the latest one.
const CHANGE_BACKLOG: usize = 8;

/// The configuration of the machine and what the startup files hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineSnapshot {
    /// The saved configuration.
    pub config: MachineConfig,
    /// The aliases of the startup files that HuBergamof does not manage.
    pub system_aliases: Vec<SystemAlias>,
}

/// Why a request about the machine failed.
#[derive(Debug, Error)]
pub enum MachineError {
    /// The configuration was refused or could not be stored.
    #[error(transparent)]
    Store(#[from] ConfigStoreError),
    /// The configuration was saved but the shell files could not be written.
    #[error("the configuration is saved but not applied: {0}")]
    Shell(#[from] ShellError),
    /// The home folder is unknown: nothing can be applied.
    #[error("the home folder of the user is unknown: the aliases cannot be applied")]
    NoHome,
    /// The blocking task failed.
    #[error("the task failed: {0}")]
    Task(String),
}

impl MachineError {
    /// Error code reported to the UI.
    #[must_use]
    pub fn error_code(&self) -> ErrorCode {
        match self {
            Self::Store(error) => error.error_code(),
            Self::Shell(_) | Self::NoHome | Self::Task(_) => ErrorCode::Internal,
        }
    }
}

struct Inner {
    store: ConfigStore,
    /// The home folder of a demo, removed with the hub.
    _demo_home: Option<tempfile::TempDir>,
    /// `None` when the home folder is unknown.
    shell: Option<MachineShell>,
    changes: broadcast::Sender<Arc<MachineSnapshot>>,
    /// One update at a time, so that what reaches the clients follows the
    /// order of the revisions.
    turn: Mutex<()>,
    /// One request at a time reaches the blocking threads, that the renderer
    /// and the store share: a client sending them in a burst waits its turn.
    requests: Semaphore,
}

/// The one place that reads and changes the machine.
#[derive(Clone)]
pub struct MachineHub {
    inner: Arc<Inner>,
}

impl MachineHub {
    /// A hub saving in `store` and applying to `shell`.
    #[must_use]
    pub fn new(store: ConfigStore, shell: Option<MachineShell>) -> Self {
        Self::build(store, shell, None)
    }

    fn build(
        store: ConfigStore,
        shell: Option<MachineShell>,
        demo_home: Option<tempfile::TempDir>,
    ) -> Self {
        let (changes, _) = broadcast::channel(CHANGE_BACKLOG);
        Self {
            inner: Arc::new(Inner {
                store,
                _demo_home: demo_home,
                shell,
                changes,
                turn: Mutex::new(()),
                requests: Semaphore::new(1),
            }),
        }
    }

    /// A hub for `--demo`: the shell files are those of a temporary home
    /// folder, which holds a few aliases, so that the real ones are never
    /// read nor written.
    ///
    /// # Errors
    /// The failure to create the folder or its startup file.
    pub fn demo(store: ConfigStore) -> std::io::Result<Self> {
        let home = tempfile::tempdir()?;
        std::fs::write(home.path().join(".bashrc"), DEMO_STARTUP_FILE)?;
        let shell = MachineShell::new(home.path().to_owned());
        Ok(Self::build(store, Some(shell), Some(home)))
    }

    /// The snapshots sent after each update, to every subscriber.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<MachineSnapshot>> {
        self.inner.changes.subscribe()
    }

    /// The saved configuration and the aliases of the startup files, read now.
    ///
    /// # Errors
    /// [`MachineError::Store`] when the configuration cannot be read.
    pub async fn refresh(&self) -> Result<MachineSnapshot, MachineError> {
        let _request = self.request_slot().await;
        let inner = Arc::clone(&self.inner);
        run_blocking(move || {
            let config = inner.store.load_machine()?.config;
            Ok(inner.snapshot_of(config))
        })
        .await
    }

    /// Saves `config` (based on the current revision), puts its enabled
    /// aliases on the machine and gives the result to every subscriber, the
    /// author included. When the shell files cannot be written the
    /// configuration stays saved and subscribers are told it all the same:
    /// the revision moved.
    ///
    /// # Errors
    /// [`MachineError::Store`] (stale revision, invalid configuration: nothing
    /// changed), or [`MachineError::Shell`] / [`MachineError::NoHome`]
    /// (saved, not applied).
    pub async fn update(&self, config: MachineConfig) -> Result<(), MachineError> {
        let _request = self.request_slot().await;
        let inner = Arc::clone(&self.inner);
        run_blocking(move || inner.update(config)).await
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
    fn update(&self, config: MachineConfig) -> Result<(), MachineError> {
        let _turn = self.turn.lock().unwrap_or_else(PoisonError::into_inner);
        let saved = self.store.save_machine(config)?;
        let applied = match &self.shell {
            Some(shell) => shell.apply(&saved.aliases).map_err(MachineError::from),
            None => Err(MachineError::NoHome),
        };
        // Nobody listening is fine.
        let _ = self.changes.send(Arc::new(self.snapshot_of(saved)));
        applied
    }

    fn snapshot_of(&self, config: MachineConfig) -> MachineSnapshot {
        let system_aliases = self
            .shell
            .as_ref()
            .map(MachineShell::read_system_aliases)
            .unwrap_or_default();
        MachineSnapshot {
            config,
            system_aliases,
        }
    }
}

async fn run_blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, MachineError> + Send + 'static,
) -> Result<T, MachineError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| MachineError::Task(error.to_string()))?
}

#[cfg(test)]
mod tests {
    use std::fs;

    use hubergamof_protocol::ShellAlias;

    use super::*;
    use crate::testing::builtin_registry;

    struct Fixture {
        config_dir: tempfile::TempDir,
        home: tempfile::TempDir,
        hub: MachineHub,
    }

    fn fixture() -> Fixture {
        let config_dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(config_dir.path(), Arc::new(builtin_registry()));
        let hub = MachineHub::new(store, Some(MachineShell::new(home.path().to_owned())));
        Fixture {
            config_dir,
            home,
            hub,
        }
    }

    fn with_alias(mut config: MachineConfig, name: &str, enabled: bool) -> MachineConfig {
        config.aliases.push(ShellAlias {
            name: name.into(),
            command: "ls -la".into(),
            enabled,
        });
        config
    }

    #[tokio::test]
    async fn the_demo_never_touches_the_real_home() {
        let config_dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(config_dir.path(), Arc::new(builtin_registry()));
        let hub = MachineHub::demo(store).unwrap();
        let snapshot = hub.refresh().await.unwrap();
        assert_eq!(snapshot.system_aliases.len(), 2);
        hub.update(with_alias(snapshot.config, "ll", true))
            .await
            .unwrap();
        assert_eq!(hub.refresh().await.unwrap().system_aliases.len(), 2);
    }

    #[tokio::test]
    async fn refresh_reads_the_configuration_and_the_system_aliases() {
        let fixture = fixture();
        fs::write(fixture.home.path().join(".bashrc"), "alias g=git\n").unwrap();
        let snapshot = fixture.hub.refresh().await.unwrap();
        assert_eq!(snapshot.config, MachineConfig::empty());
        assert_eq!(snapshot.system_aliases.len(), 1);
        assert_eq!(snapshot.system_aliases[0].source, ".bashrc");
    }

    #[tokio::test]
    async fn an_update_is_saved_applied_and_told_to_subscribers() {
        let fixture = fixture();
        let mut changes = fixture.hub.subscribe();
        let config = with_alias(MachineConfig::empty(), "ll", true);
        fixture.hub.update(config).await.unwrap();
        let told = changes.recv().await.unwrap();
        assert_eq!(told.config.revision, 1);
        assert_eq!(told.config.aliases.len(), 1);
        let managed = fs::read_to_string(fixture.home.path().join(".config/hubergamof/aliases.sh"));
        assert!(managed.unwrap().contains("alias ll='ls -la'"));
        assert!(fixture.config_dir.path().join("machine.json").is_file());
    }

    #[tokio::test]
    async fn a_disabled_alias_is_kept_in_the_configuration_but_not_applied() {
        let fixture = fixture();
        let config = with_alias(MachineConfig::empty(), "ll", false);
        fixture.hub.update(config).await.unwrap();
        assert!(!fixture.home.path().join(".config/hubergamof").exists());
        assert_eq!(fixture.hub.refresh().await.unwrap().config.aliases.len(), 1);
    }

    #[tokio::test]
    async fn a_stale_or_invalid_update_changes_nothing() {
        let fixture = fixture();
        let mut changes = fixture.hub.subscribe();
        let first = with_alias(MachineConfig::empty(), "ll", true);
        fixture.hub.update(first.clone()).await.unwrap();
        changes.recv().await.unwrap();

        let stale = fixture.hub.update(first).await.unwrap_err();
        assert_eq!(stale.error_code(), ErrorCode::StaleRevision);

        let current = fixture.hub.refresh().await.unwrap().config;
        let invalid = with_alias(current, "ll", true);
        let refused = fixture.hub.update(invalid).await.unwrap_err();
        assert_eq!(refused.error_code(), ErrorCode::InvalidConfig);
        assert!(changes.try_recv().is_err(), "nothing was told");
    }

    #[tokio::test]
    async fn a_failure_to_apply_keeps_the_saved_configuration_and_tells_it() {
        let fixture = fixture();
        fs::write(
            fixture.home.path().join(".bashrc"),
            "# >>> hubergamof aliases >>>\n",
        )
        .unwrap();
        let mut changes = fixture.hub.subscribe();
        let config = with_alias(MachineConfig::empty(), "ll", true);
        let error = fixture.hub.update(config).await.unwrap_err();
        assert!(matches!(error, MachineError::Shell(_)));
        assert_eq!(error.error_code(), ErrorCode::Internal);
        assert_eq!(changes.recv().await.unwrap().config.revision, 1);
        assert_eq!(fixture.hub.refresh().await.unwrap().config.revision, 1);
    }

    #[tokio::test]
    async fn without_a_home_folder_the_configuration_is_saved_but_not_applied() {
        let config_dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(config_dir.path(), Arc::new(builtin_registry()));
        let hub = MachineHub::new(store, None);
        let error = hub
            .update(with_alias(MachineConfig::empty(), "ll", true))
            .await
            .unwrap_err();
        assert!(matches!(error, MachineError::NoHome));
        let snapshot = hub.refresh().await.unwrap();
        assert_eq!(snapshot.config.revision, 1);
        assert_eq!(snapshot.system_aliases, []);
    }

    #[tokio::test]
    async fn two_updates_of_the_same_revision_let_only_one_through() {
        let fixture = fixture();
        let config = with_alias(MachineConfig::empty(), "ll", true);
        let (first, second) = tokio::join!(
            fixture.hub.update(config.clone()),
            fixture.hub.update(config)
        );
        let outcomes = [first, second];
        assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
        let refused = outcomes.into_iter().find_map(Result::err).unwrap();
        assert_eq!(refused.error_code(), ErrorCode::StaleRevision);
    }

    #[tokio::test]
    async fn a_slow_subscriber_still_gets_the_last_configuration() {
        let fixture = fixture();
        let mut changes = fixture.hub.subscribe();
        let mut config = MachineConfig::empty();
        for round in 0..=CHANGE_BACKLOG {
            config = with_alias(config, &format!("a{round}"), true);
            fixture.hub.update(config.clone()).await.unwrap();
            config = fixture.hub.refresh().await.unwrap().config;
        }
        let mut last = None;
        loop {
            match changes.try_recv() {
                Ok(snapshot) => last = Some(snapshot),
                Err(broadcast::error::TryRecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
        assert_eq!(last.unwrap().config, config);
    }
}
