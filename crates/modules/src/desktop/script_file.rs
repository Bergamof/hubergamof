//! Private files of the compositor scripts, in `$XDG_RUNTIME_DIR/bergadeck/`.
//!
//! Checks and writes go through file descriptors (`openat` relative to the
//! folder checked, no link followed), so that what is checked is what is
//! written to: the runtime folder and ours must belong to the user and be
//! closed to everybody else, and each script is a new file only the user
//! may read.
//!
//! Synchronous: called through `spawn_blocking`.

use std::fs::File;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, FileType, Mode, OFlags, fstat, mkdirat, open, openat, unlinkat};
use rustix::io::Errno;

/// Folder of the scripts, under `$XDG_RUNTIME_DIR`.
const SCRIPT_FOLDER: &str = "bergadeck";

/// Permissions of that folder: the user's only.
const PRIVATE_FOLDER_MODE: u32 = 0o700;

/// Permissions of the scripts.
const PRIVATE_FILE_MODE: u32 = 0o600;

/// Permission bits of the group and the others.
const GROUP_AND_OTHERS: u32 = 0o077;

/// A folder opened as such, without following a link.
const FOLDER_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// Writes `script` as `name` in the scripts folder of `runtime_dir`
/// (created if needed), replacing an earlier file, and returns its path.
///
/// # Errors
///
/// A description of what failed or was refused.
pub(super) fn write(runtime_dir: &Path, name: &str, script: &str) -> Result<PathBuf, String> {
    if !runtime_dir.is_absolute() {
        return Err(format!("{}: not an absolute path", runtime_dir.display()));
    }
    let runtime = open(runtime_dir, FOLDER_FLAGS, Mode::empty()).map_err(describe(runtime_dir))?;
    check_private(&runtime, runtime_dir)?;

    let folder_path = runtime_dir.join(SCRIPT_FOLDER);
    match mkdirat(
        &runtime,
        SCRIPT_FOLDER,
        Mode::from_raw_mode(PRIVATE_FOLDER_MODE),
    ) {
        Ok(()) | Err(Errno::EXIST) => {}
        Err(error) => return Err(describe(&folder_path)(error)),
    }
    let folder = openat(&runtime, SCRIPT_FOLDER, FOLDER_FLAGS, Mode::empty())
        .map_err(describe(&folder_path))?;
    check_private(&folder, &folder_path)?;

    let file_path = folder_path.join(name);
    match unlinkat(&folder, name, AtFlags::empty()) {
        Ok(()) | Err(Errno::NOENT) => {}
        Err(error) => return Err(describe(&file_path)(error)),
    }
    let flags = OFlags::WRONLY
        .union(OFlags::CREATE)
        .union(OFlags::EXCL)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);
    let file = openat(&folder, name, flags, Mode::from_raw_mode(PRIVATE_FILE_MODE))
        .map_err(describe(&file_path))?;
    File::from(file)
        .write_all(script.as_bytes())
        .map_err(|error| format!("{}: {error}", file_path.display()))?;
    Ok(file_path)
}

/// What failed with `error` on `path`.
fn describe(path: &Path) -> impl FnOnce(Errno) -> String + '_ {
    move |error| format!("{}: {error}", path.display())
}

/// Checks that the folder `fd` (at `path`) is the user's and closed to
/// everybody else.
fn check_private(fd: &OwnedFd, path: &Path) -> Result<(), String> {
    let refused = |reason: &str| format!("{}: {reason}", path.display());
    let stat = fstat(fd).map_err(|error| refused(&error.to_string()))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
        return Err(refused("not a folder"));
    }
    if stat.st_uid != rustix::process::getuid().as_raw() {
        return Err(refused("owned by another user"));
    }
    if stat.st_mode & GROUP_AND_OTHERS != 0 {
        return Err(refused("open to other users"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// A private runtime folder, as `$XDG_RUNTIME_DIR` is.
    fn runtime_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    #[test]
    fn scripts_are_private_and_replaced() {
        let runtime = runtime_dir();
        let first = write(runtime.path(), "windows.js", "first").unwrap();
        assert_eq!(first, runtime.path().join(SCRIPT_FOLDER).join("windows.js"));
        assert_eq!(
            mode(&runtime.path().join(SCRIPT_FOLDER)),
            PRIVATE_FOLDER_MODE
        );
        assert_eq!(mode(&first), PRIVATE_FILE_MODE);

        let second = write(runtime.path(), "windows.js", "second").unwrap();
        assert_eq!(fs::read_to_string(second).unwrap(), "second");
    }

    #[test]
    fn open_folders_are_refused() {
        let runtime = runtime_dir();
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let error = write(runtime.path(), "windows.js", "").unwrap_err();
        assert!(error.contains("open to other users"), "{error}");

        let runtime = runtime_dir();
        let folder = runtime.path().join(SCRIPT_FOLDER);
        fs::create_dir(&folder).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o750)).unwrap();
        let error = write(runtime.path(), "windows.js", "").unwrap_err();
        assert!(error.contains("open to other users"), "{error}");
    }

    #[test]
    fn links_and_relative_paths_are_refused() {
        let runtime = runtime_dir();
        let elsewhere = runtime_dir();
        std::os::unix::fs::symlink(elsewhere.path(), runtime.path().join(SCRIPT_FOLDER)).unwrap();
        assert!(write(runtime.path(), "windows.js", "").is_err());
        assert!(fs::read_dir(elsewhere.path()).unwrap().next().is_none());

        let runtime = runtime_dir();
        let folder = runtime.path().join(SCRIPT_FOLDER);
        fs::create_dir(&folder).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).unwrap();
        let target = elsewhere.path().join("target.js");
        std::os::unix::fs::symlink(&target, folder.join("windows.js")).unwrap();
        write(runtime.path(), "windows.js", "script").unwrap();
        assert!(
            !target.exists(),
            "the link is replaced, not written through"
        );

        let error = write(Path::new("relative"), "windows.js", "").unwrap_err();
        assert!(error.contains("absolute"), "{error}");
    }

    #[test]
    fn a_file_in_place_of_the_folder_is_refused() {
        let runtime = runtime_dir();
        fs::write(runtime.path().join(SCRIPT_FOLDER), "").unwrap();
        assert!(write(runtime.path(), "windows.js", "").is_err());
    }
}
