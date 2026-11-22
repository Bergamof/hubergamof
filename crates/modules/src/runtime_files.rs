//! Private files in `$XDG_RUNTIME_DIR/hubergamof/`: scripts handed to the
//! compositor, state that must outlive the manager but not the machine.
//!
//! Checks, reads and writes go through file descriptors (`openat` relative to
//! the folder checked, no link followed), so that what is checked is what is
//! used: the runtime folder and ours must belong to the user and be closed to
//! everybody else, and each file is one only the user may read.
//!
//! Synchronous: called through `spawn_blocking`, or from a dedicated thread.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use rustix::fs::{
    AtFlags, FileType, Mode, OFlags, fstat, mkdirat, open, openat, renameat, unlinkat,
};
use rustix::io::Errno;

/// Folder under `$XDG_RUNTIME_DIR`.
pub(crate) const FOLDER_NAME: &str = "hubergamof";

/// Permissions of that folder: the user's only.
pub(crate) const PRIVATE_FOLDER_MODE: u32 = 0o700;

/// Permissions of the files.
pub(crate) const PRIVATE_FILE_MODE: u32 = 0o600;

/// Permission bits of the group and the others.
const GROUP_AND_OTHERS: u32 = 0o077;

/// A folder opened as such, without following a link.
const FOLDER_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// The private folder of the manager, checked and opened.
#[derive(Debug)]
pub(crate) struct PrivateFolder {
    fd: OwnedFd,
    path: PathBuf,
}

impl PrivateFolder {
    /// Opens the folder under `runtime_dir`, creating it if needed.
    ///
    /// # Errors
    ///
    /// A description of what failed or was refused (relative path, link,
    /// folder owned by another user or open to others).
    pub(crate) fn open(runtime_dir: &Path) -> Result<Self, String> {
        if !runtime_dir.is_absolute() {
            return Err(format!("{}: not an absolute path", runtime_dir.display()));
        }
        let runtime =
            open(runtime_dir, FOLDER_FLAGS, Mode::empty()).map_err(describe(runtime_dir))?;
        check_private_folder(&runtime, runtime_dir)?;

        let path = runtime_dir.join(FOLDER_NAME);
        match mkdirat(
            &runtime,
            FOLDER_NAME,
            Mode::from_raw_mode(PRIVATE_FOLDER_MODE),
        ) {
            Ok(()) | Err(Errno::EXIST) => {}
            Err(error) => return Err(describe(&path)(error)),
        }
        let fd =
            openat(&runtime, FOLDER_NAME, FOLDER_FLAGS, Mode::empty()).map_err(describe(&path))?;
        check_private_folder(&fd, &path)?;
        Ok(Self { fd, path })
    }

    /// Writes `bytes` as the new file `name`, replacing an earlier file
    /// (or link: it is replaced, never written through). Returns its path.
    ///
    /// # Errors
    ///
    /// A description of what failed.
    pub(crate) fn write_new(&self, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
        let file_path = self.file_path(name)?;
        remove_if_present(&self.fd, name, &file_path)?;
        self.create(name, bytes, &file_path)?;
        Ok(file_path)
    }

    /// Writes `bytes` as `name` so that a reader sees the old content or the
    /// new, never half of it: a new file is written aside, then renamed.
    ///
    /// # Errors
    ///
    /// A description of what failed.
    pub(crate) fn write_atomic(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let file_path = self.file_path(name)?;
        let aside = format!("{name}.tmp");
        let aside_path = self.file_path(&aside)?;
        remove_if_present(&self.fd, &aside, &aside_path)?;
        if let Err(error) = self.create(&aside, bytes, &aside_path) {
            // A full disk must not leave a half-written file behind.
            let _ = remove_if_present(&self.fd, &aside, &aside_path);
            return Err(error);
        }
        renameat(&self.fd, aside.as_str(), &self.fd, name).map_err(describe(&file_path))
    }

    /// Content of the file `name`, `None` when there is none.
    ///
    /// # Errors
    ///
    /// A description of what failed or was refused: a link, something that
    /// is not a regular file, a file of another user or open to others, or
    /// one longer than `limit` bytes.
    pub(crate) fn read(&self, name: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
        let file_path = self.file_path(name)?;
        // Without `NONBLOCK`, opening a FIFO put there waits for a writer
        // before the type is checked below.
        let flags = OFlags::RDONLY
            .union(OFlags::NOFOLLOW)
            .union(OFlags::NONBLOCK)
            .union(OFlags::NOCTTY)
            .union(OFlags::CLOEXEC);
        let fd = match openat(&self.fd, name, flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return Ok(None),
            Err(error) => return Err(describe(&file_path)(error)),
        };
        let refused = |reason: &str| format!("{}: {reason}", file_path.display());
        let stat = fstat(&fd).map_err(|error| refused(&error.to_string()))?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
            return Err(refused("not a regular file"));
        }
        if stat.st_uid != rustix::process::getuid().as_raw() {
            return Err(refused("owned by another user"));
        }
        if stat.st_mode & GROUP_AND_OTHERS != 0 {
            return Err(refused("open to other users"));
        }
        let mut content = Vec::new();
        File::from(fd)
            .take(limit.saturating_add(1))
            .read_to_end(&mut content)
            .map_err(|error| refused(&error.to_string()))?;
        if content.len() as u64 > limit {
            return Err(refused("too long"));
        }
        Ok(Some(content))
    }

    fn create(&self, name: &str, bytes: &[u8], file_path: &Path) -> Result<(), String> {
        let flags = OFlags::WRONLY
            .union(OFlags::CREATE)
            .union(OFlags::EXCL)
            .union(OFlags::NOFOLLOW)
            .union(OFlags::CLOEXEC);
        let file = openat(
            &self.fd,
            name,
            flags,
            Mode::from_raw_mode(PRIVATE_FILE_MODE),
        )
        .map_err(describe(file_path))?;
        File::from(file)
            .write_all(bytes)
            .map_err(|error| format!("{}: {error}", file_path.display()))
    }

    /// Path of `name` in the folder; a name that is not a plain file name is
    /// refused (the callers pass fixed names or checked identifiers, this is
    /// the last line).
    fn file_path(&self, name: &str) -> Result<PathBuf, String> {
        let plain = !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\0']);
        if plain {
            Ok(self.path.join(name))
        } else {
            Err(format!("{name:?}: not a plain file name"))
        }
    }
}

fn remove_if_present(folder: &OwnedFd, name: &str, file_path: &Path) -> Result<(), String> {
    match unlinkat(folder, name, AtFlags::empty()) {
        Ok(()) | Err(Errno::NOENT) => Ok(()),
        Err(error) => Err(describe(file_path)(error)),
    }
}

/// What failed with `error` on `path`.
fn describe(path: &Path) -> impl FnOnce(Errno) -> String + '_ {
    move |error| format!("{}: {error}", path.display())
}

/// Checks that the folder `fd` (at `path`) is the user's and closed to
/// everybody else.
fn check_private_folder(fd: &OwnedFd, path: &Path) -> Result<(), String> {
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
pub(crate) mod testing {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    /// A private runtime folder, as `$XDG_RUNTIME_DIR` is.
    pub(crate) fn runtime_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::testing::runtime_dir;
    use super::*;

    const LIMIT: u64 = 1024;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn the_folder_and_its_files_are_private() {
        let runtime = runtime_dir();
        let folder = PrivateFolder::open(runtime.path()).unwrap();
        let path = folder.write_new("a.txt", b"content").unwrap();
        assert_eq!(mode(&runtime.path().join(FOLDER_NAME)), PRIVATE_FOLDER_MODE);
        assert_eq!(mode(&path), PRIVATE_FILE_MODE);
    }

    #[test]
    fn an_atomic_write_replaces_the_content_and_leaves_nothing_aside() {
        let runtime = runtime_dir();
        let folder = PrivateFolder::open(runtime.path()).unwrap();
        folder.write_atomic("state.json", b"first").unwrap();
        folder.write_atomic("state.json", b"second").unwrap();
        assert_eq!(
            folder.read("state.json", LIMIT).unwrap().unwrap(),
            b"second"
        );
        let names: Vec<_> = fs::read_dir(runtime.path().join(FOLDER_NAME))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["state.json"]);
    }

    #[test]
    fn a_missing_file_reads_as_nothing() {
        let runtime = runtime_dir();
        let folder = PrivateFolder::open(runtime.path()).unwrap();
        assert_eq!(folder.read("none", LIMIT).unwrap(), None);
    }

    #[test]
    fn unsafe_files_are_not_read() {
        let runtime = runtime_dir();
        let folder = PrivateFolder::open(runtime.path()).unwrap();
        let inside = runtime.path().join(FOLDER_NAME);

        let open_file = folder.write_new("open", b"x").unwrap();
        fs::set_permissions(open_file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            folder
                .read("open", LIMIT)
                .unwrap_err()
                .contains("open to other users")
        );

        let elsewhere = runtime_dir();
        let target = elsewhere.path().join("target");
        fs::write(&target, "secret").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&target, inside.join("link")).unwrap();
        assert!(folder.read("link", LIMIT).is_err());

        fs::create_dir(inside.join("dir")).unwrap();
        assert!(folder.read("dir", LIMIT).is_err());

        let fifo = inside.join("fifo");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &fifo,
            FileType::Fifo,
            Mode::from_raw_mode(PRIVATE_FILE_MODE),
            0,
        )
        .unwrap();
        assert!(
            folder
                .read("fifo", LIMIT)
                .unwrap_err()
                .contains("not a regular file"),
            "a FIFO is refused, not waited for"
        );

        folder.write_new("big", &[b'x'; 2048]).unwrap();
        assert!(folder.read("big", LIMIT).unwrap_err().contains("too long"));
    }

    #[test]
    fn names_that_leave_the_folder_are_refused() {
        let runtime = runtime_dir();
        let folder = PrivateFolder::open(runtime.path()).unwrap();
        for name in ["", ".", "..", "a/b", "../x", "a\0b"] {
            assert!(folder.write_new(name, b"").is_err(), "{name:?}");
            assert!(folder.read(name, LIMIT).is_err(), "{name:?}");
        }
    }

    #[test]
    fn open_folders_links_and_relative_paths_are_refused() {
        let runtime = runtime_dir();
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let error = PrivateFolder::open(runtime.path()).unwrap_err();
        assert!(error.contains("open to other users"), "{error}");

        let runtime = runtime_dir();
        let inside = runtime.path().join(FOLDER_NAME);
        fs::create_dir(&inside).unwrap();
        fs::set_permissions(&inside, fs::Permissions::from_mode(0o750)).unwrap();
        let error = PrivateFolder::open(runtime.path()).unwrap_err();
        assert!(error.contains("open to other users"), "{error}");

        let runtime = runtime_dir();
        let elsewhere = runtime_dir();
        std::os::unix::fs::symlink(elsewhere.path(), runtime.path().join(FOLDER_NAME)).unwrap();
        assert!(PrivateFolder::open(runtime.path()).is_err());
        assert!(fs::read_dir(elsewhere.path()).unwrap().next().is_none());

        let runtime = runtime_dir();
        fs::write(runtime.path().join(FOLDER_NAME), "").unwrap();
        assert!(PrivateFolder::open(runtime.path()).is_err());

        let error = PrivateFolder::open(Path::new("relative")).unwrap_err();
        assert!(error.contains("absolute"), "{error}");
    }

    #[test]
    fn a_link_in_place_of_a_file_is_replaced_not_written_through() {
        let runtime = runtime_dir();
        let folder = PrivateFolder::open(runtime.path()).unwrap();
        let elsewhere = runtime_dir();
        let target = elsewhere.path().join("target");
        std::os::unix::fs::symlink(&target, runtime.path().join(FOLDER_NAME).join("f")).unwrap();
        folder.write_new("f", b"script").unwrap();
        folder.write_atomic("f", b"state").unwrap();
        assert!(!target.exists());
    }
}
