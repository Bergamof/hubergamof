//! Bodies of the backup routes (`/api/backup/…`): what an import would do, what
//! it did, and the automatic snapshots taken before it. A section is named by
//! its id: `devices/<serial>`, `mice/<serial>`, `radial-menu`, `modules`,
//! `machine`, `git` or `ssh`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ErrorCode;

/// What importing a section would do to this computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SectionChange {
    /// This computer has nothing there yet.
    Added,
    /// The content differs: it would be replaced.
    Replaced,
    /// The same content (revisions apart): nothing would change.
    Same,
}

/// What kind of command an item of a backup would run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExecutableKind {
    /// A shell alias.
    ShellAlias,
    /// A Git alias.
    GitAlias,
    /// A free option of an SSH host.
    SshOption,
    /// A free setting of Git.
    GitSetting,
    /// An SSH host that names a machine, a jump or the forwarding of the agent.
    SshHost,
}

/// Something a backup would have this computer run, which the person is
/// shown before anything is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExecutableItem {
    /// What kind of command it is.
    pub kind: ExecutableKind,
    /// The alias, or the host and the option.
    pub name: String,
    /// The command or the value.
    pub detail: String,
    /// Whether it is enabled: a disabled one is imported all the same, one
    /// switch from being run.
    pub enabled: bool,
    /// Whether its name is one this computer already uses outside the
    /// application (a shell alias, an SSH host): importing it comments the
    /// original line out.
    pub takes_over: bool,
}

/// What an import would do to one section of the backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SectionPreview {
    /// Id of the section.
    pub section: String,
    /// What an import would do to it.
    pub change: SectionChange,
    /// Whether the device or mouse of the section is connected now: when it
    /// is not, its configuration is kept for its next connection. `true`
    /// for what does not depend on a device.
    pub present: bool,
    /// What it would have this computer run (the whole of it, even when the
    /// section would not change).
    pub executables: Vec<ExecutableItem>,
}

/// What an import would do (`POST /api/backup/preview`). Nothing is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupPreview {
    /// The sections of the backup.
    pub sections: Vec<SectionPreview>,
    /// Ids of the sections that this computer has and the backup does not:
    /// an import leaves them as they are.
    pub kept: Vec<String>,
}

/// What an import did to one section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "result", rename_all = "camelCase")]
#[ts(export)]
pub enum SectionResult {
    /// Written and in use.
    Applied,
    /// Written for a device that is not connected: used at its next connection.
    Stored,
    /// Written, but the files of the machine could not be updated.
    NotApplied {
        /// Why.
        message: String,
    },
    /// Left as it was.
    Refused {
        /// Kind of the refusal.
        code: ErrorCode,
        /// Why.
        message: String,
    },
}

/// The outcome of one section of an import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SectionOutcome {
    /// Id of the section.
    pub section: String,
    /// What happened to it.
    pub outcome: SectionResult,
}

/// What an import did (`POST /api/backup/import`, `…/restore`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportReport {
    /// Id of the snapshot taken before anything was written: restoring it
    /// undoes the import.
    pub snapshot: String,
    /// One outcome per imported section.
    pub sections: Vec<SectionOutcome>,
}

/// An automatic snapshot (`GET /api/backup/snapshots`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SnapshotInfo {
    /// Its id, to give to the restore route.
    pub id: String,
    /// When it was taken (seconds since the Unix epoch).
    pub created_at: u64,
    /// Version of the application that took it.
    pub app_version: String,
    /// Ids of the sections it holds.
    pub sections: Vec<String>,
}
