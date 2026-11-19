//! Files of the store: atomic and private writes, reading a configuration,
//! setting an invalid one aside.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use hubergamof_protocol::DeviceConfig;
use tempfile::NamedTempFile;

use super::{ConfigStoreError, INVALID_BACKUP_PREFIX, LoadOutcome, migration};

pub(super) fn set_invalid_aside(
    path: &Path,
    reason: &ConfigStoreError,
) -> Result<LoadOutcome, ConfigStoreError> {
    let backup = backup_path(path);
    fs::rename(path, &backup).map_err(|source| io_error(path, source))?;
    tracing::warn!(
        path = %path.display(),
        backup = %backup.display(),
        %reason,
        "invalid configuration set aside, using defaults"
    );
    Ok(LoadOutcome::ReplacedInvalid { backup })
}

pub(super) fn read_config(path: &Path) -> Result<Option<DeviceConfig>, ConfigStoreError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(path, error)),
    };
    migration::parse_config(&bytes)
        .map(Some)
        .map_err(|source| ConfigStoreError::Json {
            path: path.to_owned(),
            source,
        })
}

pub(super) fn write_atomically(path: &Path, config: &DeviceConfig) -> Result<(), ConfigStoreError> {
    let mut json = serde_json::to_vec_pretty(config).map_err(|source| ConfigStoreError::Json {
        path: path.to_owned(),
        source,
    })?;
    json.push(b'\n');
    write_bytes_atomically(path, &json)
}

/// Creates `dir` and its missing parents, readable by the user only (the
/// service umask would let other accounts list serials and images).
pub(super) fn create_private_dir(dir: &Path) -> Result<(), ConfigStoreError> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIR_MODE)
        .create(dir)
        .map_err(|source| io_error(dir, source))
}

/// Sets an existing folder of the store to [`PRIVATE_DIR_MODE`]; a missing
/// one, or a link (never followed), is left alone.
pub(super) fn restrict_existing_dir(dir: &Path) -> Result<(), ConfigStoreError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = match fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(io_error(dir, source)),
    };
    if !metadata.is_dir() || metadata.permissions().mode() & 0o777 == PRIVATE_DIR_MODE {
        return Ok(());
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(PRIVATE_DIR_MODE))
        .map_err(|source| io_error(dir, source))
}

/// Mode of the folders the store creates.
pub(super) const PRIVATE_DIR_MODE: u32 = 0o700;

/// Writes `bytes` to a temporary file next to `path`, flushes it to disk and
/// renames it over `path`: readers see either the old or the new content.
pub(crate) fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> Result<(), ConfigStoreError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    create_private_dir(dir)?;
    let mut file = NamedTempFile::new_in(dir).map_err(|source| io_error(dir, source))?;
    file.write_all(bytes)
        .and_then(|()| file.as_file().sync_all())
        .map_err(|source| io_error(file.path(), source))?;
    file.persist(path)
        .map_err(|error| io_error(path, error.error))?;
    // Persist the rename itself.
    fs::File::open(dir)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error(dir, source))
}

pub(super) fn backup_path(path: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let base = format!("{INVALID_BACKUP_PREFIX}{timestamp}");
    let mut candidate = dir.join(&base);
    let mut attempt = 1_u32;
    while candidate.exists() {
        candidate = dir.join(format!("{base}-{attempt}"));
        attempt += 1;
    }
    candidate
}

pub(super) fn io_error(path: &Path, source: io::Error) -> ConfigStoreError {
    ConfigStoreError::Io {
        path: path.to_owned(),
        source,
    }
}
