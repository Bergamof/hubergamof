//! Automatic snapshots of the configuration, taken before an import writes
//! anything: restoring one undoes the import. Each is a backup (see
//! [`super::backup`]) in `snapshots/<id>.zip`, written privately; only the
//! [`MAX_SNAPSHOTS`] latest are kept. They hold the same things as a backup
//! (Git identity, SSH hosts, alias commands) and are never part of one.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use hubergamof_protocol::SnapshotInfo;

use super::archive::{ArchiveError, MAX_ARCHIVE_BYTES};
use super::backup::{Backup, read_backup, write_backup};
use super::backup_store::{RawFile, read_regular_file};
use super::files::{io_error, restrict_existing_dir, write_bytes_atomically};
use super::{ConfigStore, ConfigStoreError};

/// How many snapshots are kept: the latest.
pub const MAX_SNAPSHOTS: usize = 3;

/// Folder of the snapshots, in the configuration folder.
const SNAPSHOTS_DIR: &str = "snapshots";

/// Extension of a snapshot.
const SNAPSHOT_EXTENSION: &str = ".zip";

/// A snapshot that was read: its id, and the backup it holds.
#[derive(Debug)]
pub struct Snapshot {
    /// Its id.
    pub id: String,
    /// What it holds.
    pub backup: Backup,
}

/// The id of a snapshot: `<milliseconds>-<number>`, which orders them in time
/// and tells two taken in the same millisecond apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SnapshotId {
    millis: u64,
    number: u32,
}

impl SnapshotId {
    /// The id that a text spells; `None` for anything else, a path included.
    fn parse(text: &str) -> Option<Self> {
        let (millis, number) = text.split_once('-')?;
        let plain = |digits: &str| {
            !digits.is_empty() && digits.len() <= 20 && digits.bytes().all(|b| b.is_ascii_digit())
        };
        if !plain(millis) || !plain(number) {
            return None;
        }
        Some(Self {
            millis: millis.parse().ok()?,
            number: number.parse().ok()?,
        })
    }

    fn text(self) -> String {
        format!("{}-{}", self.millis, self.number)
    }
}

impl ConfigStore {
    fn snapshots_dir(&self) -> PathBuf {
        self.root.join(SNAPSHOTS_DIR)
    }

    fn snapshot_path(&self, id: SnapshotId) -> PathBuf {
        self.snapshots_dir()
            .join(format!("{}{SNAPSHOT_EXTENSION}", id.text()))
    }

    /// The ids of the snapshots on the disk, the latest first. What is not a
    /// snapshot is ignored.
    fn snapshot_ids(&self) -> Result<Vec<SnapshotId>, ConfigStoreError> {
        let dir = self.snapshots_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io_error(&dir, error)),
        };
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| io_error(&dir, error))?;
            let id = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(SNAPSHOT_EXTENSION))
                .and_then(SnapshotId::parse);
            ids.extend(id);
        }
        ids.sort_unstable_by(|a, b| b.cmp(a));
        Ok(ids)
    }

    /// Saves the configuration as it is now, and forgets the snapshots beyond
    /// the latest [`MAX_SNAPSHOTS`]. Returns its id.
    ///
    /// # Errors
    ///
    /// An I/O error, or a configuration that does not fit a backup.
    pub fn take_snapshot(&self, app_version: &str) -> Result<String, ConfigStoreError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let collected = self.collect_backup(app_version, now.as_secs())?;
        if !collected.skipped.is_empty() {
            tracing::warn!(
                sections = ?collected.skipped,
                "sections left out of the snapshot"
            );
        }
        let bytes = write_backup(&collected.backup)?;
        if bytes.len() > MAX_ARCHIVE_BYTES {
            return Err(ArchiveError::TooLarge.into());
        }
        let _guard = self.lock();
        // A folder that existed before is not left more open than it should be.
        restrict_existing_dir(&self.snapshots_dir())?;
        let millis = u64::try_from(now.as_millis()).unwrap_or(u64::MAX);
        let taken = self.snapshot_ids()?;
        let mut id = SnapshotId { millis, number: 0 };
        while taken.contains(&id) {
            id.number += 1;
        }
        write_bytes_atomically(&self.snapshot_path(id), &bytes)?;
        for old in self.snapshot_ids()?.into_iter().skip(MAX_SNAPSHOTS) {
            let path = self.snapshot_path(old);
            if let Err(error) = fs::remove_file(&path) {
                tracing::warn!(path = %path.display(), %error, "old snapshot not removed");
            }
        }
        Ok(id.text())
    }

    /// The snapshot `id`, read as a backup.
    ///
    /// # Errors
    ///
    /// [`ConfigStoreError::UnknownSnapshot`] for an id that is not one or has
    /// no file (a link, a pipe or an oversized file in its place included),
    /// or the error of reading it as a backup.
    pub fn read_snapshot(&self, id: &str) -> Result<Snapshot, ConfigStoreError> {
        let unknown = || ConfigStoreError::UnknownSnapshot(id.chars().take(40).collect());
        let parsed = SnapshotId::parse(id).ok_or_else(unknown)?;
        match read_regular_file(&self.snapshot_path(parsed), MAX_ARCHIVE_BYTES)? {
            RawFile::Bytes(bytes) => Ok(Snapshot {
                id: parsed.text(),
                backup: read_backup(&bytes)?,
            }),
            RawFile::Missing | RawFile::Unreadable => Err(unknown()),
        }
    }

    /// The snapshots, the latest first. One that cannot be read is left out.
    ///
    /// # Errors
    ///
    /// An I/O error on the folder.
    pub fn list_snapshots(&self) -> Result<Vec<SnapshotInfo>, ConfigStoreError> {
        let mut infos = Vec::new();
        for id in self.snapshot_ids()? {
            match self.read_snapshot(&id.text()) {
                Ok(snapshot) => infos.push(SnapshotInfo {
                    id: snapshot.id,
                    created_at: snapshot.backup.manifest.created_at,
                    app_version: snapshot.backup.manifest.app_version,
                    sections: snapshot.backup.manifest.sections,
                }),
                Err(error) => tracing::warn!(id = %id.text(), %error, "unreadable snapshot"),
            }
        }
        Ok(infos)
    }
}

#[cfg(test)]
mod tests;
