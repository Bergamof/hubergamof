//! The SSH hosts of the computer, as the web interface sees it: what HuBergamof
//! saved (`config/ssh.json`) and the hosts `~/.ssh/config` holds besides. It is
//! a feature of the machine like the others, see [`Hub`].
//!
//! - [`render`]: the hosts as the file `ssh` reads.
//! - [`apply`]: that file on the computer, and the include that loads it.
//! - [`sections`]: `~/.ssh/config` cut into the sections `ssh` sees.
//! - [`claim`]: the hosts taken over, commented out where they came from.
//! - [`system`]: the hosts found besides.

mod apply;
mod claim;
mod render;
mod sections;
mod system;

use std::sync::Arc;

use hubergamof_protocol::{SshConfig, SshSystemHost};

pub use apply::{SshFiles, SshFilesError};
pub use render::UnwritableHost;
pub use system::MAX_SYSTEM_SSH_HOSTS;

use super::hub::{Feature, Hub, HubError};
use crate::config_store::{ConfigStore, ConfigStoreError, Loaded};

/// The configuration file of the demo's home folder, so that the interface has
/// something to list.
const DEMO_SSH_CONFIG: &str = "Host demo-server\n\tHostName 192.0.2.10\n\tUser ada\n\tPort 2222\n\n\
                               Host demo-tunnel\n\tHostName 192.0.2.20\n\tProxyCommand nc %h %p\n";

/// The SSH configuration and what `~/.ssh/config` holds besides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshSnapshot {
    /// The saved configuration.
    pub config: SshConfig,
    /// The hosts that HuBergamof does not manage.
    pub system_hosts: Vec<SshSystemHost>,
}

/// Why a request about SSH failed.
pub type SshError = HubError<SshFilesError>;

/// The SSH hosts of the machine.
pub struct SshHosts;

impl Feature for SshHosts {
    type Config = SshConfig;
    type Target = SshFiles;
    type Snapshot = SshSnapshot;
    type ApplyError = SshFilesError;

    fn load(store: &ConfigStore) -> Result<Loaded<SshConfig>, ConfigStoreError> {
        store.load_ssh()
    }

    fn save(store: &ConfigStore, config: SshConfig) -> Result<SshConfig, ConfigStoreError> {
        store.save_ssh(config)
    }

    fn apply(files: &SshFiles, config: &SshConfig) -> Result<(), SshFilesError> {
        files.apply(config)
    }

    async fn snapshot(files: Option<Arc<SshFiles>>, config: SshConfig) -> SshSnapshot {
        let system_hosts = match files {
            Some(files) => tokio::task::spawn_blocking(move || files.read_system_hosts())
                .await
                .unwrap_or_default(),
            None => Vec::new(),
        };
        SshSnapshot {
            config,
            system_hosts,
        }
    }
}

/// The one place that reads and changes the SSH hosts.
pub type SshConfigHub = Hub<SshHosts>;

impl SshConfigHub {
    /// A hub for `--demo`: the SSH files are those of a temporary home folder,
    /// which holds a few hosts, so that the real ones are never read nor
    /// written.
    ///
    /// # Errors
    /// The failure to create the folder or its SSH configuration.
    pub fn demo(store: ConfigStore) -> std::io::Result<Self> {
        Self::with_demo_home(store, |home| {
            std::fs::create_dir(home.join(".ssh"))?;
            std::fs::write(home.join(".ssh/config"), DEMO_SSH_CONFIG)?;
            Ok(SshFiles::new(home.to_owned()))
        })
    }
}

#[cfg(test)]
mod tests;
