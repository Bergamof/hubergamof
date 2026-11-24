//! What the features of the machine (the shell aliases, Git, SSH) share: a
//! hub that saves a configuration, puts it on the computer, reads the computer
//! when a client asks, and tells every client what it did.
//!
//! The computer is not watched: it is read when a client asks
//! ([`Hub::refresh`]) and after an update, which every client hears through
//! [`Hub::subscribe`]. A feature says only what is its own: how its
//! configuration is stored, how it is applied to the computer and what it
//! shows besides the configuration ([`Feature`]).

use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::Arc;

use hubergamof_protocol::ErrorCode;
use thiserror::Error;
use tokio::sync::{Semaphore, SemaphorePermit, broadcast};

use crate::config_store::{ConfigStore, ConfigStoreError, LoadOutcome, Loaded};

/// Snapshots a slow client may miss before it is told only the latest one.
const CHANGE_BACKLOG: usize = 8;

/// What a feature of the machine is made of.
pub trait Feature: Send + Sync + 'static {
    /// What the person edits and the store saves.
    type Config: Clone + Send + 'static;
    /// What the configuration is put on: the files of the user under a home
    /// folder.
    type Target: Send + Sync + 'static;
    /// What clients are told: the configuration and what the computer holds
    /// besides.
    type Snapshot: Send + Sync + 'static;
    /// Why the configuration could not be put on the computer.
    type ApplyError: std::error::Error + Send + 'static;

    /// Loads the saved configuration.
    ///
    /// # Errors
    /// What the store says.
    fn load(store: &ConfigStore) -> Result<Loaded<Self::Config>, ConfigStoreError>;

    /// Saves `config` if it is valid and based on the current revision.
    ///
    /// # Errors
    /// What the store says: a stale revision, an invalid configuration.
    fn save(store: &ConfigStore, config: Self::Config) -> Result<Self::Config, ConfigStoreError>;

    /// Puts `config` on the computer. Synchronous: it runs on a blocking task.
    ///
    /// # Errors
    /// [`Self::ApplyError`] for what cannot be written.
    fn apply(target: &Self::Target, config: &Self::Config) -> Result<(), Self::ApplyError>;

    /// What clients are told for `config`: the computer is read now. `target`
    /// is `None` when the home folder is unknown.
    fn snapshot(
        target: Option<Arc<Self::Target>>,
        config: Self::Config,
    ) -> impl Future<Output = Self::Snapshot> + Send;
}

/// Why a request about a feature of the machine failed.
#[derive(Debug, Error)]
pub enum HubError<E: std::error::Error> {
    /// The configuration was refused or could not be stored.
    #[error(transparent)]
    Store(#[from] ConfigStoreError),
    /// The configuration was saved but could not be put on the computer.
    #[error("the configuration is saved but not applied: {0}")]
    Apply(E),
    /// The home folder is unknown: nothing can be applied.
    #[error("the home folder of the user is unknown: the configuration cannot be applied")]
    NoHome,
    /// The blocking task failed.
    #[error("the task failed: {0}")]
    Task(String),
}

impl<E: std::error::Error> HubError<E> {
    /// Error code reported to the UI.
    #[must_use]
    pub fn error_code(&self) -> ErrorCode {
        match self {
            Self::Store(error) => error.error_code(),
            Self::Apply(_) | Self::NoHome | Self::Task(_) => ErrorCode::Internal,
        }
    }
}

struct Inner<F: Feature> {
    store: ConfigStore,
    /// The home folder of a demo, removed with the hub.
    _demo_home: Option<tempfile::TempDir>,
    /// `None` when the home folder is unknown.
    target: Option<Arc<F::Target>>,
    changes: broadcast::Sender<Arc<F::Snapshot>>,
    /// One request at a time: what reaches the clients follows the order of
    /// the revisions, and the blocking threads that the renderer and the store
    /// share are not flooded by a client sending requests in a burst.
    requests: Semaphore,
}

/// The one place that reads and changes a feature of the machine.
pub struct Hub<F: Feature> {
    inner: Arc<Inner<F>>,
}

impl<F: Feature> Clone for Hub<F> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<F: Feature> Hub<F> {
    /// A hub saving in `store` and applying to `target`.
    #[must_use]
    pub fn new(store: ConfigStore, target: Option<F::Target>) -> Self {
        Self::build(store, target, None)
    }

    /// A hub for `--demo`: the target is made by `prepare` in a temporary
    /// home folder (which holds a few things to list), so that the real files
    /// are never read nor written.
    ///
    /// # Errors
    /// The failure to create the folder or what `prepare` puts in it.
    pub fn with_demo_home(
        store: ConfigStore,
        prepare: impl FnOnce(&Path) -> io::Result<F::Target>,
    ) -> io::Result<Self> {
        let home = tempfile::tempdir()?;
        let target = prepare(home.path())?;
        Ok(Self::build(store, Some(target), Some(home)))
    }

    fn build(
        store: ConfigStore,
        target: Option<F::Target>,
        demo_home: Option<tempfile::TempDir>,
    ) -> Self {
        let (changes, _) = broadcast::channel(CHANGE_BACKLOG);
        Self {
            inner: Arc::new(Inner {
                store,
                _demo_home: demo_home,
                target: target.map(Arc::new),
                changes,
                requests: Semaphore::new(1),
            }),
        }
    }

    /// What the hub applies to, for a test to look at it.
    #[cfg(test)]
    pub(super) fn target(&self) -> Option<&F::Target> {
        self.inner.target.as_deref()
    }

    /// The snapshots sent after each update, to every subscriber.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<F::Snapshot>> {
        self.inner.changes.subscribe()
    }

    /// The saved configuration and what the computer holds, read now.
    ///
    /// A saved configuration that was invalid is set aside and replaced by an
    /// empty one: what the invalid one had put on the computer is removed too,
    /// so that the page shows what the computer holds.
    ///
    /// # Errors
    /// [`HubError::Store`] when the configuration cannot be read.
    pub async fn refresh(&self) -> Result<F::Snapshot, HubError<F::ApplyError>> {
        let _request = self.request_slot().await;
        let inner = Arc::clone(&self.inner);
        let config = run_blocking(move || inner.load()).await?;
        Ok(F::snapshot(self.inner.target.clone(), config).await)
    }

    /// Saves `config` (based on the current revision), puts it on the computer
    /// and gives the result to every subscriber, the author included. When it
    /// cannot be put on the computer the configuration stays saved and
    /// subscribers are told it all the same: the revision moved.
    ///
    /// # Errors
    /// [`HubError::Store`] (stale revision, invalid configuration: nothing
    /// changed), or [`HubError::Apply`] / [`HubError::NoHome`] (saved, not
    /// applied).
    pub async fn update(&self, config: F::Config) -> Result<(), HubError<F::ApplyError>> {
        let _request = self.request_slot().await;
        let inner = Arc::clone(&self.inner);
        let (saved, applied) = run_blocking(move || inner.save_and_apply(config)).await?;
        let snapshot = F::snapshot(self.inner.target.clone(), saved).await;
        // Nobody listening is fine.
        let _ = self.inner.changes.send(Arc::new(snapshot));
        applied
    }

    async fn request_slot(&self) -> SemaphorePermit<'_> {
        // The semaphore lives as long as the hub and is never closed.
        self.inner
            .requests
            .acquire()
            .await
            .unwrap_or_else(|_| unreachable!("the request semaphore is never closed"))
    }
}

impl<F: Feature> Inner<F> {
    fn load(&self) -> Result<F::Config, HubError<F::ApplyError>> {
        let loaded = F::load(&self.store)?;
        if matches!(loaded.outcome, LoadOutcome::ReplacedInvalid { .. })
            && let Some(target) = &self.target
            && let Err(error) = F::apply(target, &loaded.config)
        {
            tracing::warn!(%error, "what an invalid configuration had applied was not removed");
        }
        Ok(loaded.config)
    }

    /// Saves `config`; the outcome of putting it on the computer comes with
    /// the saved configuration, since it is saved either way.
    #[allow(clippy::type_complexity)]
    fn save_and_apply(
        &self,
        config: F::Config,
    ) -> Result<(F::Config, Result<(), HubError<F::ApplyError>>), HubError<F::ApplyError>> {
        let saved = F::save(&self.store, config)?;
        let applied = match &self.target {
            Some(target) => F::apply(target, &saved).map_err(HubError::Apply),
            None => Err(HubError::NoHome),
        };
        Ok((saved, applied))
    }
}

async fn run_blocking<T: Send + 'static, E: std::error::Error + Send + 'static>(
    work: impl FnOnce() -> Result<T, HubError<E>> + Send + 'static,
) -> Result<T, HubError<E>> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| HubError::Task(error.to_string()))?
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use hubergamof_protocol::MachineConfig;

    use super::*;
    use crate::testing::builtin_registry;

    /// A feature that keeps the machine configuration and records what is
    /// applied, so that what the hub does is seen without any file.
    struct Recording;

    #[derive(Debug, Error)]
    #[error("cannot apply")]
    struct CannotApply;

    #[derive(Default)]
    struct Applied {
        revisions: Mutex<Vec<u64>>,
        fail: Mutex<bool>,
    }

    impl Feature for Recording {
        type Config = MachineConfig;
        type Target = Applied;
        type Snapshot = MachineConfig;
        type ApplyError = CannotApply;

        fn load(store: &ConfigStore) -> Result<Loaded<MachineConfig>, ConfigStoreError> {
            store.load_machine()
        }

        fn save(
            store: &ConfigStore,
            config: MachineConfig,
        ) -> Result<MachineConfig, ConfigStoreError> {
            store.save_machine(config)
        }

        fn apply(target: &Applied, config: &MachineConfig) -> Result<(), CannotApply> {
            target.revisions.lock().unwrap().push(config.revision);
            if *target.fail.lock().unwrap() {
                Err(CannotApply)
            } else {
                Ok(())
            }
        }

        fn snapshot(
            _target: Option<Arc<Applied>>,
            config: MachineConfig,
        ) -> impl Future<Output = MachineConfig> + Send {
            std::future::ready(config)
        }
    }

    struct Fixture {
        config_dir: tempfile::TempDir,
        hub: Hub<Recording>,
    }

    fn fixture() -> Fixture {
        let config_dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(config_dir.path(), Arc::new(builtin_registry()));
        let hub = Hub::new(store, Some(Applied::default()));
        Fixture { config_dir, hub }
    }

    fn applied(hub: &Hub<Recording>) -> Vec<u64> {
        let target = hub.inner.target.as_ref().unwrap();
        target.revisions.lock().unwrap().clone()
    }

    fn with_alias(mut config: MachineConfig, name: &str) -> MachineConfig {
        config.aliases.push(hubergamof_protocol::ShellAlias {
            name: name.into(),
            command: "ls -la".into(),
            enabled: true,
        });
        config
    }

    #[tokio::test]
    async fn an_update_is_saved_applied_and_told_to_subscribers() {
        let fixture = fixture();
        let mut changes = fixture.hub.subscribe();
        fixture
            .hub
            .update(with_alias(MachineConfig::empty(), "ll"))
            .await
            .unwrap();
        let told = changes.recv().await.unwrap();
        assert_eq!(told.revision, 1);
        assert_eq!(told.aliases.len(), 1);
        assert_eq!(applied(&fixture.hub), [1]);
        assert!(fixture.config_dir.path().join("machine.json").is_file());
    }

    #[tokio::test]
    async fn a_stale_or_invalid_update_changes_nothing() {
        let fixture = fixture();
        let mut changes = fixture.hub.subscribe();
        let first = with_alias(MachineConfig::empty(), "ll");
        fixture.hub.update(first.clone()).await.unwrap();
        changes.recv().await.unwrap();

        let stale = fixture.hub.update(first).await.unwrap_err();
        assert_eq!(stale.error_code(), ErrorCode::StaleRevision);

        let current = fixture.hub.refresh().await.unwrap();
        let refused = fixture
            .hub
            .update(with_alias(current, "ll"))
            .await
            .unwrap_err();
        assert_eq!(refused.error_code(), ErrorCode::InvalidConfig);
        assert!(changes.try_recv().is_err(), "nothing was told");
        assert_eq!(applied(&fixture.hub), [1], "nothing more was applied");
    }

    #[tokio::test]
    async fn a_failure_to_apply_keeps_the_saved_configuration_and_tells_it() {
        let fixture = fixture();
        *fixture
            .hub
            .inner
            .target
            .as_ref()
            .unwrap()
            .fail
            .lock()
            .unwrap() = true;
        let mut changes = fixture.hub.subscribe();
        let error = fixture
            .hub
            .update(with_alias(MachineConfig::empty(), "ll"))
            .await
            .unwrap_err();
        assert!(matches!(error, HubError::Apply(_)));
        assert_eq!(error.error_code(), ErrorCode::Internal);
        assert_eq!(changes.recv().await.unwrap().revision, 1);
        assert_eq!(fixture.hub.refresh().await.unwrap().revision, 1);
    }

    #[tokio::test]
    async fn without_a_home_folder_the_configuration_is_saved_but_not_applied() {
        let config_dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(config_dir.path(), Arc::new(builtin_registry()));
        let hub: Hub<Recording> = Hub::new(store, None);
        let error = hub
            .update(with_alias(MachineConfig::empty(), "ll"))
            .await
            .unwrap_err();
        assert!(matches!(error, HubError::NoHome));
        assert_eq!(hub.refresh().await.unwrap().revision, 1);
    }

    #[tokio::test]
    async fn two_updates_of_the_same_revision_let_only_one_through() {
        let fixture = fixture();
        let config = with_alias(MachineConfig::empty(), "ll");
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
            config = with_alias(config, &format!("a{round}"));
            fixture.hub.update(config.clone()).await.unwrap();
            config = fixture.hub.refresh().await.unwrap();
        }
        let mut last = None;
        loop {
            match changes.try_recv() {
                Ok(snapshot) => last = Some(snapshot),
                Err(broadcast::error::TryRecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
        assert_eq!(*last.unwrap(), config);
    }

    #[tokio::test]
    async fn an_invalid_saved_configuration_is_replaced_and_what_it_applied_is_taken_back() {
        let fixture = fixture();
        fixture
            .hub
            .update(with_alias(MachineConfig::empty(), "ll"))
            .await
            .unwrap();
        std::fs::write(fixture.config_dir.path().join("machine.json"), "{ broken").unwrap();

        let snapshot = fixture.hub.refresh().await.unwrap();
        assert_eq!(snapshot, MachineConfig::empty());
        assert_eq!(applied(&fixture.hub), [1, 0], "the empty one was applied");
    }

    #[tokio::test]
    async fn a_demo_hub_works_in_a_temporary_home_folder() {
        let config_dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(config_dir.path(), Arc::new(builtin_registry()));
        let seen = Arc::new(Mutex::new(None));
        let hub: Hub<Recording> = Hub::with_demo_home(store, {
            let seen = Arc::clone(&seen);
            move |home| {
                *seen.lock().unwrap() = Some(home.to_owned());
                Ok(Applied::default())
            }
        })
        .unwrap();
        let home = seen.lock().unwrap().clone().unwrap();
        assert!(home.starts_with(std::env::temp_dir()));
        assert!(home.is_dir());
        drop(hub);
        assert!(!home.exists(), "the folder goes with the hub");
    }
}
