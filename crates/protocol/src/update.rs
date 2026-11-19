//! State of the check for a newer version of the manager.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Result of the check for a newer release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum UpdateStatus {
    /// No check has run yet.
    Unknown,
    /// A check is running.
    Checking,
    /// The running version is the latest release.
    UpToDate,
    /// A newer release exists.
    Available {
        /// Version of the newer release, without the leading `v`.
        version: String,
        /// Release notes page.
        notes_url: String,
    },
    /// The last check failed (network down, rate limit…); the next one retries.
    CheckFailed {
        /// Human-readable details.
        message: String,
    },
    /// The package of a newer release is being downloaded.
    Downloading {
        /// Version being installed.
        version: String,
    },
    /// The package of a newer release is verified and installed by the
    /// system's package manager.
    Installing {
        /// Version being installed.
        version: String,
    },
    /// The package is installed; the service restarts into it.
    Installed {
        /// Version installed.
        version: String,
    },
    /// The installation failed; the running version is unchanged.
    InstallFailed {
        /// Version that could not be installed.
        version: String,
        /// Short description of the failure.
        message: String,
    },
}

/// Whether this manager can install a newer release by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum InstallSupport {
    /// Installed from a system package: updates can be installed.
    Available,
    /// Not installed from a package (built from sources, container…): the
    /// release can be announced but not installed.
    Unpackaged,
    /// `--demo`: installation is simulated.
    Demo,
}

/// How updates are handled, sent after `moduleCatalog` and at every change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateSettings {
    /// Install a newer release as soon as it is found (otherwise on request).
    pub auto_install: bool,
    /// Whether installing is possible at all.
    pub install_support: InstallSupport,
}
