//! Reading and writing the text files of the user that the machine pages
//! touch (shell startup files, Git configuration), and the private files
//! HuBergamof keeps beside them. Synchronous: callers run it on a blocking
//! task.
//!
//! Those files are the user's own: they may be links to a dotfiles
//! repository, so a link is followed (and the file behind it rewritten, the
//! link kept), but only a regular file of reasonable size is ever read, and
//! nothing is written to a file that is not valid text or whose block is
//! damaged.

use std::fs::{self, OpenOptions, Permissions};
use std::io::{self, Read as _, Write as _};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;
use thiserror::Error;

use super::block::UnterminatedBlock;

/// Largest startup file read or rewritten: a bigger one is not a
/// configuration file, and is not loaded in memory.
pub const MAX_STARTUP_FILE_BYTES: u64 = 256 * 1024;

/// Permissions of the files HuBergamof keeps: the user's only.
pub(super) const PRIVATE_FILE_MODE: u32 = 0o600;

/// Permissions of the folder holding them.
pub(super) const PRIVATE_DIR_MODE: u32 = 0o700;

/// Permission bits kept when a file is rewritten: setuid, setgid and sticky
/// are not copied onto the new file.
pub(super) const MODE_MASK: u32 = 0o777;

/// Why a file could not be read or written.
#[derive(Debug, Error)]
pub enum FileError {
    /// A file operation failed.
    #[error("cannot access {path}: {source}")]
    Io {
        /// Path involved.
        path: PathBuf,
        /// Underlying error.
        source: io::Error,
    },
    /// The path is not a regular file (or, for the private folder, a folder).
    #[error("{0} is not a regular file")]
    NotRegular(PathBuf),
    /// Another name links to the same file: replacing it would only change
    /// one of them.
    #[error("{0} has other hard links")]
    HardLinked(PathBuf),
    /// The file is larger than [`MAX_STARTUP_FILE_BYTES`].
    #[error("{0} is larger than {MAX_STARTUP_FILE_BYTES} bytes")]
    TooLarge(PathBuf),
    /// The file is not valid UTF-8: it is left untouched.
    #[error("{0} is not valid UTF-8, left untouched")]
    NotUtf8(PathBuf),
    /// The file has a HuBergamof block without its end: it is left untouched.
    #[error("{0} holds an unterminated HuBergamof block, left untouched")]
    BrokenBlock(PathBuf),
}

pub(super) fn io_error(path: &Path, source: io::Error) -> FileError {
    FileError::Io {
        path: path.to_owned(),
        source,
    }
}

/// The bytes of a startup file; `None` when it does not exist.
///
/// Opened without blocking, so that a named pipe in its place is refused
/// instead of waiting for a writer.
pub(super) fn read_text_file(path: &Path) -> Result<Option<Vec<u8>>, FileError> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(path, error)),
    };
    let metadata = file.metadata().map_err(|error| io_error(path, error))?;
    if !metadata.is_file() {
        return Err(FileError::NotRegular(path.to_owned()));
    }
    let mut bytes = Vec::new();
    // One byte more than the limit tells a file that grew since the check.
    file.take(MAX_STARTUP_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(path, error))?;
    if bytes.len() as u64 > MAX_STARTUP_FILE_BYTES {
        return Err(FileError::TooLarge(path.to_owned()));
    }
    Ok(Some(bytes))
}

/// Rewrites a startup file through `change`, keeping its mode and any link
/// to it. A missing file is not created; an unchanged one is not written.
pub(super) fn update_text_file(
    path: &Path,
    change: impl FnOnce(&str) -> Result<String, UnterminatedBlock>,
) -> Result<(), FileError> {
    // Resolved once: what is read is what is written, even if the link moves.
    let target = match fs::canonicalize(path) {
        Ok(target) => target,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(path, error)),
    };
    let Some(bytes) = read_text_file(&target)? else {
        return Ok(());
    };
    let content = String::from_utf8(bytes).map_err(|_| FileError::NotUtf8(path.to_owned()))?;
    let updated =
        change(&content).map_err(|UnterminatedBlock| FileError::BrokenBlock(path.to_owned()))?;
    if updated == content {
        return Ok(());
    }
    let metadata = fs::metadata(&target).map_err(|error| io_error(&target, error))?;
    if metadata.nlink() > 1 {
        return Err(FileError::HardLinked(target));
    }
    let mode = metadata.permissions().mode() & MODE_MASK;
    write_replacing(&target, updated.as_bytes(), mode)
}

/// Creates the folder of the managed file readable by the user only, or
/// tightens it. A link or a file in its place is refused.
pub(super) fn ensure_private_dir(dir: &Path) -> Result<(), FileError> {
    match fs::symlink_metadata(dir) {
        Ok(metadata) if metadata.is_dir() => {
            if metadata.permissions().mode() & MODE_MASK != PRIVATE_DIR_MODE {
                fs::set_permissions(dir, Permissions::from_mode(PRIVATE_DIR_MODE))
                    .map_err(|error| io_error(dir, error))?;
            }
            Ok(())
        }
        Ok(_) => Err(FileError::NotRegular(dir.to_owned())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::DirBuilder::new()
            .recursive(true)
            .mode(PRIVATE_DIR_MODE)
            .create(dir)
            .map_err(|error| io_error(dir, error)),
        Err(error) => Err(io_error(dir, error)),
    }
}

/// Writes `bytes` to a temporary file next to `path` with `mode`, then
/// renames it over `path`: readers see the old or the new content, never
/// half of it, and a failure leaves `path` as it was.
pub(super) fn write_replacing(path: &Path, bytes: &[u8], mode: u32) -> Result<(), FileError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut file = NamedTempFile::new_in(dir).map_err(|error| io_error(dir, error))?;
    let written = file
        .as_file()
        .set_permissions(Permissions::from_mode(mode))
        .and_then(|()| file.write_all(bytes))
        .and_then(|()| file.as_file().sync_all());
    written.map_err(|error| io_error(file.path(), error))?;
    file.persist(path)
        .map_err(|error| io_error(path, error.error))?;
    Ok(())
}

/// Removes `path`; a file that is not there is already removed.
pub(super) fn remove_file_if_present(path: &Path) -> Result<(), FileError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(path, error)),
    }
}
