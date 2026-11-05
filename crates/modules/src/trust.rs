//! Whether a file the manager hands to a program (a compose file, an
//! application to launch) may have been changed by someone else than the
//! user.
//!
//! Synchronous file system reads: async callers go through
//! `spawn_blocking`.

use std::fmt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Permission bits letting the group or others write.
const SHARED_WRITE_BITS: u32 = 0o022;
/// Sticky bit: in such a directory (`/tmp`), only the owner of an entry may
/// replace it.
const STICKY_BIT: u32 = 0o1000;
/// User id of root, trusted as the owner of system directories.
const ROOT_UID: u32 = 0;

/// A path refused, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Untrusted {
    /// The path (or the directory above it) refused.
    pub(crate) path: String,
    /// Why.
    pub(crate) reason: String,
}

impl fmt::Display for Untrusted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "untrusted path `{}`: {}", self.path, self.reason)
    }
}

/// `path` with its links resolved, once it and every directory above it
/// are found to belong to the user or to root and to be writable by nobody
/// else (a directory with the sticky bit, such as `/tmp`, is fine: nobody
/// else may replace what the user put there).
pub(crate) fn trusted(path: &Path) -> Result<PathBuf, Untrusted> {
    trusted_for(path, rustix::process::getuid().as_raw())
}

/// [`trusted`] for the user `user`.
fn trusted_for(path: &Path, user: u32) -> Result<PathBuf, Untrusted> {
    let untrusted = |path: &Path, reason: String| Untrusted {
        path: path.display().to_string(),
        reason,
    };
    let resolved =
        std::fs::canonicalize(path).map_err(|error| untrusted(path, error.to_string()))?;
    for (depth, entry) in resolved.ancestors().enumerate() {
        let metadata =
            std::fs::metadata(entry).map_err(|error| untrusted(entry, error.to_string()))?;
        let owner = metadata.uid();
        if owner != user && owner != ROOT_UID {
            return Err(untrusted(entry, format!("owned by user {owner}")));
        }
        let mode = metadata.permissions().mode();
        let shared_directory = depth > 0 && metadata.is_dir() && mode & STICKY_BIT != 0;
        if mode & SHARED_WRITE_BITS != 0 && !shared_directory {
            return Err(untrusted(
                entry,
                format!("writable by others (mode {mode:o})"),
            ));
        }
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use std::fs::Permissions;

    use super::*;

    fn set_mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn only_files_nobody_else_may_change_are_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        set_mode(&project, 0o755);
        let file = project.join("compose.yaml");
        std::fs::write(&file, "services: {}\n").unwrap();
        set_mode(&file, 0o644);
        trusted(&file).unwrap();

        set_mode(&file, 0o666);
        let error = trusted(&file).unwrap_err();
        assert!(error.reason.contains("writable by others"), "{error}");
        set_mode(&file, 0o644);

        // A directory others may write to: they could swap the file.
        set_mode(&project, 0o777);
        assert!(trusted(&file).is_err());
        // Unless it is sticky, like `/tmp`.
        set_mode(&project, 0o1777);
        trusted(&file).unwrap();
        set_mode(&project, 0o755);

        // Links are followed: the target is what is used.
        let link = dir.path().join("link.yaml");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(trusted(&link).unwrap(), trusted(&file).unwrap());
        set_mode(&file, 0o646);
        assert!(trusted(&link).is_err());

        assert!(trusted(Path::new("/nonexistent/compose.yaml")).is_err());
    }

    #[test]
    fn only_the_user_or_root_may_own_a_trusted_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("compose.yaml");
        std::fs::write(&file, "services: {}\n").unwrap();
        set_mode(&file, 0o644);
        let user = rustix::process::getuid().as_raw();
        trusted_for(&file, user).unwrap();
        // Run as root, the file is root's, always trusted: nothing to check.
        if user != 0 {
            let other = trusted_for(&file, user + 1).unwrap_err();
            assert!(other.reason.contains("owned by user"), "{other}");
        }
    }
}
