//! The machine HuBergamof runs on, as the web interface sees it: its
//! configuration (`config/machine.json`, for now the shell aliases) and the
//! aliases already in the shell startup files.
//!
//! - [`hub`]: what the features of the machine share, see [`Hub`].
//! - [`git`]: the Git configuration, a feature of its own.
//! - [`ssh`]: the SSH hosts, another.
//! - [`shell`]: the files, read without running a shell, written atomically.
//! - [`aliases`]: parsing and quoting.
//! - [`rc_block`]: the block that sources the managed file.

mod aliases;
mod block;
mod files;
pub mod git;
mod hub;
mod rc_block;
mod shell;
pub mod ssh;

use std::sync::Arc;

use hubergamof_protocol::{MachineConfig, SystemAlias};

pub use aliases::UnwritableAlias;
pub use files::{FileError, MAX_TEXT_FILE_BYTES};
pub use hub::{Feature, Hub, HubError};
pub use shell::{MAX_SYSTEM_ALIASES, MachineShell, STARTUP_FILES, ShellError};

use crate::config_store::{ConfigStore, ConfigStoreError, Loaded};

/// Aliases the demo's startup file holds, so that the interface has some to list.
const DEMO_STARTUP_FILE: &str = "alias gs='git status'\nalias gl='git log --oneline'\n";

/// The configuration of the machine and what the startup files hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineSnapshot {
    /// The saved configuration.
    pub config: MachineConfig,
    /// The aliases of the startup files that HuBergamof does not manage.
    pub system_aliases: Vec<SystemAlias>,
}

/// Why a request about the machine failed.
pub type MachineError = HubError<ShellError>;

/// The shell aliases of the machine.
pub struct ShellAliases;

impl Feature for ShellAliases {
    type Config = MachineConfig;
    type Target = MachineShell;
    type Snapshot = MachineSnapshot;
    type ApplyError = ShellError;

    fn load(store: &ConfigStore) -> Result<Loaded<MachineConfig>, ConfigStoreError> {
        store.load_machine()
    }

    fn save(store: &ConfigStore, config: MachineConfig) -> Result<MachineConfig, ConfigStoreError> {
        store.save_machine(config)
    }

    fn apply(shell: &MachineShell, config: &MachineConfig) -> Result<(), ShellError> {
        shell.apply(&config.aliases)
    }

    async fn snapshot(shell: Option<Arc<MachineShell>>, config: MachineConfig) -> MachineSnapshot {
        // The startup files are read on a blocking thread, which the hub
        // does not hold while it waits for a request slot.
        let system_aliases = match shell {
            Some(shell) => tokio::task::spawn_blocking(move || shell.read_system_aliases())
                .await
                .unwrap_or_default(),
            None => Vec::new(),
        };
        MachineSnapshot {
            config,
            system_aliases,
        }
    }
}

/// The one place that reads and changes the shell aliases of the machine.
pub type MachineHub = Hub<ShellAliases>;

impl MachineHub {
    /// A hub for `--demo`: the shell files are those of a temporary home
    /// folder, which holds a few aliases, so that the real ones are never
    /// read nor written.
    ///
    /// # Errors
    /// The failure to create the folder or its startup file.
    pub fn demo(store: ConfigStore) -> std::io::Result<Self> {
        Self::with_demo_home(store, |home| {
            std::fs::write(home.join(".bashrc"), DEMO_STARTUP_FILE)?;
            Ok(MachineShell::new(home.to_owned()))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use hubergamof_protocol::{ErrorCode, ShellAlias};

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
        assert!(matches!(error, HubError::Apply(_)));
        assert_eq!(error.error_code(), ErrorCode::Internal);
        assert_eq!(changes.recv().await.unwrap().config.revision, 1);
        assert_eq!(fixture.hub.refresh().await.unwrap().config.revision, 1);
    }

    #[tokio::test]
    async fn an_invalid_saved_configuration_takes_the_aliases_it_had_applied_with_it() {
        let fixture = fixture();
        let bashrc = fixture.home.path().join(".bashrc");
        fs::write(&bashrc, "alias ll='ls'\n").unwrap();
        fixture
            .hub
            .update(with_alias(MachineConfig::empty(), "ll", true))
            .await
            .unwrap();
        assert!(!fs::read_to_string(&bashrc).unwrap().starts_with("alias ll"));

        fs::write(fixture.config_dir.path().join("machine.json"), "{ broken").unwrap();
        fixture.hub.refresh().await.unwrap();
        assert_eq!(fs::read_to_string(&bashrc).unwrap(), "alias ll='ls'\n");
        assert!(
            !fixture
                .home
                .path()
                .join(".config/hubergamof/aliases.sh")
                .exists()
        );
    }
}
