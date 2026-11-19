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
}
