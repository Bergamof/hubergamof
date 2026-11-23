//! The shell side of the machine: which startup files are read, the managed
//! aliases file, and the block that sources it. Synchronous: callers run it
//! on a blocking task.
//!
//! Startup files are the user's own: they may be links to a dotfiles
//! repository, so a link is followed (and the file behind it rewritten, the
//! link kept), but only a regular file of reasonable size is ever read, and
//! nothing is written to a file that is not valid text or whose block is
//! damaged.

use std::collections::HashSet;
use std::fs::{self, OpenOptions, Permissions};
use std::io::{self, Read as _, Write as _};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use hubergamof_protocol::{ShellAlias, SystemAlias};
use tempfile::NamedTempFile;
use thiserror::Error;

use super::aliases::{UnwritableAlias, claim_originals, parse_aliases, render_managed_file};
use super::rc_block::{UnterminatedBlock, with_block, without_block};

/// Files, relative to the home folder, that the aliases are read from.
pub const STARTUP_FILES: [&str; 4] = [".bash_aliases", ".bashrc", ".zshrc", ".zsh_aliases"];

/// Files that get the block sourcing the managed file, when they exist.
const FILES_WITH_BLOCK: [&str; 2] = [".bashrc", ".zshrc"];

/// The managed aliases file, relative to the home folder.
pub const MANAGED_FILE: &str = ".config/hubergamof/aliases.sh";

/// Largest startup file read or rewritten: a bigger one is not a shell
/// configuration, and is not loaded in memory.
pub const MAX_STARTUP_FILE_BYTES: u64 = 256 * 1024;

/// Most aliases listed from the startup files.
pub const MAX_SYSTEM_ALIASES: usize = 512;

/// Permissions of the managed file: the user's only.
const MANAGED_FILE_MODE: u32 = 0o600;

/// Permissions of the folder holding it.
const MANAGED_DIR_MODE: u32 = 0o700;

/// Permission bits kept when a file is rewritten: setuid, setgid and sticky
/// are not copied onto the new file.
const MODE_MASK: u32 = 0o777;

/// Why the shell files could not be read or written.
#[derive(Debug, Error)]
pub enum ShellError {
    /// A file operation failed.
    #[error("cannot access {path}: {source}")]
    Io {
        /// Path involved.
        path: PathBuf,
        /// Underlying error.
        source: io::Error,
    },
    /// The path is not a regular file (or, for the managed folder, a folder).
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
    /// An alias that cannot be written safely.
    #[error(transparent)]
    Unwritable(#[from] UnwritableAlias),
}

fn io_error(path: &Path, source: io::Error) -> ShellError {
    ShellError::Io {
        path: path.to_owned(),
        source,
    }
}

/// The shell files of one user, under `home`.
#[derive(Debug, Clone)]
pub struct MachineShell {
    home: PathBuf,
}

impl MachineShell {
    /// The shell files under `home` (a test or demo folder, or the real one).
    #[must_use]
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }

    /// The shell files of the user running the manager, when its home folder
    /// is known and absolute (a relative one would resolve against the
    /// manager's folder).
    #[must_use]
    pub fn of_current_user() -> Option<Self> {
        std::env::home_dir()
            .filter(|home| home.is_absolute())
            .map(Self::new)
    }

    /// The aliases of the startup files, which HuBergamof does not manage. A
    /// file that cannot be read is logged and skipped. Nothing is run.
    #[must_use]
    pub fn read_system_aliases(&self) -> Vec<SystemAlias> {
        let mut found = Vec::new();
        for relative in STARTUP_FILES {
            let path = self.home.join(relative);
            let bytes = match read_startup_file(&path) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(%error, "startup file skipped");
                    continue;
                }
            };
            let content = String::from_utf8_lossy(&bytes);
            found.extend(
                parse_aliases(&content)
                    .into_iter()
                    .map(|(name, command)| SystemAlias {
                        name,
                        command,
                        source: relative.to_owned(),
                    }),
            );
        }
        found.truncate(MAX_SYSTEM_ALIASES);
        found
    }

    /// Puts the enabled `aliases` on the machine: the managed file holds
    /// them, and the startup files that exist source it. With none enabled
    /// the managed file and the blocks are removed. Then the original
    /// definitions of every alias of the configuration, enabled or not, are
    /// commented out in the startup files (so that a disabled one is really
    /// gone), and those of an alias that left the configuration are given back.
    ///
    /// # Errors
    /// [`ShellError`] for the first file that cannot be written; each write
    /// is atomic, so no file is left half written.
    pub fn apply(&self, aliases: &[ShellAlias]) -> Result<(), ShellError> {
        let enabled: Vec<&ShellAlias> = aliases.iter().filter(|alias| alias.enabled).collect();
        if enabled.is_empty() {
            // The file first: a startup file that cannot be rewritten must not
            // leave the removed aliases loaded by the other one.
            self.remove_managed_file()?;
            self.update_blocks(without_block)?;
        } else {
            let content = render_managed_file(&enabled)?;
            self.write_managed_file(&content)?;
            self.update_blocks(with_block)?;
        }
        // After the managed file, so that an enabled alias is never missing.
        self.claim_originals(aliases)
    }

    fn claim_originals(&self, aliases: &[ShellAlias]) -> Result<(), ShellError> {
        let names: HashSet<&str> = aliases.iter().map(|alias| alias.name.as_str()).collect();
        for relative in STARTUP_FILES {
            update_startup_file(&self.home.join(relative), |content| {
                Ok(claim_originals(content, &names))
            })?;
        }
        Ok(())
    }

    fn update_blocks(
        &self,
        change: fn(&str) -> Result<String, UnterminatedBlock>,
    ) -> Result<(), ShellError> {
        for relative in FILES_WITH_BLOCK {
            update_startup_file(&self.home.join(relative), change)?;
        }
        Ok(())
    }

    fn write_managed_file(&self, content: &str) -> Result<(), ShellError> {
        let path = self.home.join(MANAGED_FILE);
        if let Some(dir) = path.parent() {
            ensure_private_dir(dir)?;
        }
        write_replacing(&path, content.as_bytes(), MANAGED_FILE_MODE)
    }

    fn remove_managed_file(&self) -> Result<(), ShellError> {
        let path = self.home.join(MANAGED_FILE);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(&path, error)),
        }
    }
}

/// The bytes of a startup file; `None` when it does not exist.
///
/// Opened without blocking, so that a named pipe in its place is refused
/// instead of waiting for a writer.
fn read_startup_file(path: &Path) -> Result<Option<Vec<u8>>, ShellError> {
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
        return Err(ShellError::NotRegular(path.to_owned()));
    }
    let mut bytes = Vec::new();
    // One byte more than the limit tells a file that grew since the check.
    file.take(MAX_STARTUP_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(path, error))?;
    if bytes.len() as u64 > MAX_STARTUP_FILE_BYTES {
        return Err(ShellError::TooLarge(path.to_owned()));
    }
    Ok(Some(bytes))
}

/// Rewrites a startup file through `change`, keeping its mode and any link
/// to it. A missing file is not created; an unchanged one is not written.
fn update_startup_file(
    path: &Path,
    change: impl FnOnce(&str) -> Result<String, UnterminatedBlock>,
) -> Result<(), ShellError> {
    // Resolved once: what is read is what is written, even if the link moves.
    let target = match fs::canonicalize(path) {
        Ok(target) => target,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(path, error)),
    };
    let Some(bytes) = read_startup_file(&target)? else {
        return Ok(());
    };
    let content = String::from_utf8(bytes).map_err(|_| ShellError::NotUtf8(path.to_owned()))?;
    let updated =
        change(&content).map_err(|UnterminatedBlock| ShellError::BrokenBlock(path.to_owned()))?;
    if updated == content {
        return Ok(());
    }
    let metadata = fs::metadata(&target).map_err(|error| io_error(&target, error))?;
    if metadata.nlink() > 1 {
        return Err(ShellError::HardLinked(target));
    }
    let mode = metadata.permissions().mode() & MODE_MASK;
    write_replacing(&target, updated.as_bytes(), mode)
}

/// Creates the folder of the managed file readable by the user only, or
/// tightens it. A link or a file in its place is refused.
fn ensure_private_dir(dir: &Path) -> Result<(), ShellError> {
    match fs::symlink_metadata(dir) {
        Ok(metadata) if metadata.is_dir() => {
            if metadata.permissions().mode() & MODE_MASK != MANAGED_DIR_MODE {
                fs::set_permissions(dir, Permissions::from_mode(MANAGED_DIR_MODE))
                    .map_err(|error| io_error(dir, error))?;
            }
            Ok(())
        }
        Ok(_) => Err(ShellError::NotRegular(dir.to_owned())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::DirBuilder::new()
            .recursive(true)
            .mode(MANAGED_DIR_MODE)
            .create(dir)
            .map_err(|error| io_error(dir, error)),
        Err(error) => Err(io_error(dir, error)),
    }
}

/// Writes `bytes` to a temporary file next to `path` with `mode`, then
/// renames it over `path`: readers see the old or the new content, never
/// half of it, and a failure leaves `path` as it was.
fn write_replacing(path: &Path, bytes: &[u8], mode: u32) -> Result<(), ShellError> {
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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    fn home() -> (tempfile::TempDir, MachineShell) {
        let dir = tempfile::tempdir().unwrap();
        let shell = MachineShell::new(dir.path().to_owned());
        (dir, shell)
    }

    fn alias(name: &str, command: &str, enabled: bool) -> ShellAlias {
        ShellAlias {
            name: name.into(),
            command: command.into(),
            enabled,
        }
    }

    fn read(dir: &tempfile::TempDir, relative: &str) -> String {
        fs::read_to_string(dir.path().join(relative)).unwrap()
    }

    fn mode(dir: &tempfile::TempDir, relative: &str) -> u32 {
        fs::metadata(dir.path().join(relative))
            .unwrap()
            .permissions()
            .mode()
            & MODE_MASK
    }

    #[test]
    fn aliases_are_listed_with_their_file() {
        let (dir, shell) = home();
        fs::write(dir.path().join(".bashrc"), "alias ll='ls -l'\n").unwrap();
        fs::write(
            dir.path().join(".zshrc"),
            "alias ll='ls -lh'\nalias g=git\n",
        )
        .unwrap();
        let found = shell.read_system_aliases();
        let listed: Vec<_> = found
            .iter()
            .map(|a| (a.name.as_str(), a.command.as_str(), a.source.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("ll", "ls -l", ".bashrc"),
                ("ll", "ls -lh", ".zshrc"),
                ("g", "git", ".zshrc")
            ]
        );
    }

    #[test]
    fn a_home_without_startup_files_has_no_aliases() {
        let (_dir, shell) = home();
        assert_eq!(shell.read_system_aliases(), []);
    }

    #[test]
    fn an_unreadable_or_oversized_startup_file_is_skipped() {
        let (dir, shell) = home();
        fs::create_dir(dir.path().join(".bashrc")).unwrap();
        let limit = usize::try_from(MAX_STARTUP_FILE_BYTES).unwrap();
        let big = format!("alias a=b\n{}", "#".repeat(limit));
        fs::write(dir.path().join(".zshrc"), big).unwrap();
        fs::write(dir.path().join(".bash_aliases"), "alias ok=1\n").unwrap();
        let found = shell.read_system_aliases();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "ok");
    }

    #[test]
    fn a_linked_startup_file_is_read() {
        let (dir, shell) = home();
        fs::create_dir(dir.path().join("dotfiles")).unwrap();
        fs::write(dir.path().join("dotfiles/bashrc"), "alias a=b\n").unwrap();
        symlink(
            dir.path().join("dotfiles/bashrc"),
            dir.path().join(".bashrc"),
        )
        .unwrap();
        assert_eq!(shell.read_system_aliases().len(), 1);
    }

    #[test]
    fn many_aliases_are_capped() {
        let (dir, shell) = home();
        let content = (0..MAX_SYSTEM_ALIASES + 10)
            .map(|i| format!("alias a{i}=x\n"))
            .collect::<Vec<_>>()
            .concat();
        fs::write(dir.path().join(".bashrc"), content).unwrap();
        assert_eq!(shell.read_system_aliases().len(), MAX_SYSTEM_ALIASES);
    }

    #[test]
    fn enabled_aliases_are_written_privately_and_sourced_from_existing_files_only() {
        let (dir, shell) = home();
        fs::write(dir.path().join(".bashrc"), "export A=1\n").unwrap();
        shell
            .apply(&[
                alias("on", "echo 'hi'", true),
                alias("off", "echo no", false),
            ])
            .unwrap();
        let managed = read(&dir, MANAGED_FILE);
        assert!(managed.contains("alias on='echo '\\''hi'\\'''"));
        assert!(!managed.contains("off"));
        assert_eq!(mode(&dir, MANAGED_FILE), MANAGED_FILE_MODE);
        assert_eq!(mode(&dir, ".config/hubergamof"), MANAGED_DIR_MODE);
        let bashrc = read(&dir, ".bashrc");
        assert!(bashrc.starts_with("export A=1\n"));
        assert!(bashrc.contains("# >>> hubergamof aliases >>>"));
        assert!(!dir.path().join(".zshrc").exists(), "never created");
    }

    #[test]
    fn applying_twice_is_idempotent() {
        let (dir, shell) = home();
        fs::write(dir.path().join(".zshrc"), "setopt x").unwrap();
        let aliases = [alias("a", "ls", true)];
        shell.apply(&aliases).unwrap();
        let first = read(&dir, ".zshrc");
        shell.apply(&aliases).unwrap();
        assert_eq!(read(&dir, ".zshrc"), first);
        assert_eq!(first.matches("# >>> hubergamof aliases >>>").count(), 1);
    }

    #[test]
    fn disabling_everything_removes_the_file_and_the_blocks() {
        let (dir, shell) = home();
        fs::write(dir.path().join(".bashrc"), "export A=1\n").unwrap();
        fs::write(dir.path().join(".zshrc"), "setopt x\n").unwrap();
        shell.apply(&[alias("a", "ls", true)]).unwrap();
        shell.apply(&[alias("a", "ls", false)]).unwrap();
        assert!(!dir.path().join(MANAGED_FILE).exists());
        assert_eq!(read(&dir, ".bashrc"), "export A=1\n");
        assert_eq!(read(&dir, ".zshrc"), "setopt x\n");
        shell.apply(&[]).unwrap();
    }

    #[test]
    fn the_mode_and_the_link_of_a_startup_file_are_kept() {
        let (dir, shell) = home();
        let real = dir.path().join("dotfiles-bashrc");
        fs::write(&real, "export A=1\n").unwrap();
        fs::set_permissions(&real, Permissions::from_mode(0o640)).unwrap();
        symlink(&real, dir.path().join(".bashrc")).unwrap();
        shell.apply(&[alias("a", "ls", true)]).unwrap();
        assert!(
            fs::symlink_metadata(dir.path().join(".bashrc"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(
            fs::metadata(&real).unwrap().permissions().mode() & MODE_MASK,
            0o640
        );
        assert!(fs::read_to_string(&real).unwrap().contains("hubergamof"));
    }

    #[test]
    fn a_damaged_block_leaves_the_file_untouched_and_is_reported() {
        let (dir, shell) = home();
        let damaged = "export A=1\n# >>> hubergamof aliases >>>\nno end\n";
        fs::write(dir.path().join(".bashrc"), damaged).unwrap();
        let error = shell.apply(&[alias("a", "ls", true)]).unwrap_err();
        assert!(matches!(error, ShellError::BrokenBlock(_)), "{error}");
        assert_eq!(read(&dir, ".bashrc"), damaged);
    }

    #[test]
    fn disabling_everything_removes_the_file_even_when_a_startup_file_is_damaged() {
        let (dir, shell) = home();
        shell.apply(&[alias("a", "ls", true)]).unwrap();
        fs::write(
            dir.path().join(".bashrc"),
            "# >>> hubergamof aliases >>>\nno end\n",
        )
        .unwrap();
        let error = shell.apply(&[alias("a", "ls", false)]).unwrap_err();
        assert!(matches!(error, ShellError::BrokenBlock(_)), "{error}");
        assert!(!dir.path().join(MANAGED_FILE).exists());
    }

    #[test]
    fn a_startup_file_with_other_hard_links_is_left_untouched() {
        let (dir, shell) = home();
        let original = dir.path().join("elsewhere");
        fs::write(&original, "export A=1\n").unwrap();
        fs::hard_link(&original, dir.path().join(".bashrc")).unwrap();
        let error = shell.apply(&[alias("a", "ls", true)]).unwrap_err();
        assert!(matches!(error, ShellError::HardLinked(_)), "{error}");
        assert_eq!(read(&dir, ".bashrc"), "export A=1\n");
    }

    #[test]
    fn special_mode_bits_are_not_copied_onto_a_rewritten_file() {
        let (dir, shell) = home();
        let path = dir.path().join(".bashrc");
        fs::write(&path, "export A=1\n").unwrap();
        fs::set_permissions(&path, Permissions::from_mode(0o4755)).unwrap();
        shell.apply(&[alias("a", "ls", true)]).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o755
        );
    }

    #[test]
    fn a_startup_file_that_is_not_text_is_left_untouched() {
        let (dir, shell) = home();
        fs::write(dir.path().join(".bashrc"), [0xff, 0xfe, b'\n']).unwrap();
        let error = shell.apply(&[alias("a", "ls", true)]).unwrap_err();
        assert!(matches!(error, ShellError::NotUtf8(_)), "{error}");
        assert_eq!(
            fs::read(dir.path().join(".bashrc")).unwrap(),
            [0xff, 0xfe, b'\n']
        );
    }

    #[test]
    fn an_unwritable_destination_is_an_error_not_a_corruption() {
        let (dir, shell) = home();
        fs::create_dir_all(dir.path().join(".config")).unwrap();
        fs::write(dir.path().join(".config/hubergamof"), "in the way").unwrap();
        let error = shell.apply(&[alias("a", "ls", true)]).unwrap_err();
        assert!(matches!(error, ShellError::NotRegular(_)), "{error}");
        assert_eq!(read(&dir, ".config/hubergamof"), "in the way");
    }

    #[test]
    fn a_linked_managed_folder_is_refused() {
        let (dir, shell) = home();
        fs::create_dir_all(dir.path().join(".config")).unwrap();
        fs::create_dir(dir.path().join("elsewhere")).unwrap();
        symlink(
            dir.path().join("elsewhere"),
            dir.path().join(".config/hubergamof"),
        )
        .unwrap();
        assert!(shell.apply(&[alias("a", "ls", true)]).is_err());
        assert!(
            fs::read_dir(dir.path().join("elsewhere"))
                .unwrap()
                .next()
                .is_none()
        );
    }

    #[test]
    fn a_linked_managed_file_is_replaced_not_written_through() {
        let (dir, shell) = home();
        let target = dir.path().join("victim");
        fs::write(&target, "precious").unwrap();
        fs::create_dir_all(dir.path().join(".config/hubergamof")).unwrap();
        symlink(&target, dir.path().join(MANAGED_FILE)).unwrap();
        shell.apply(&[alias("a", "ls", true)]).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "precious");
        assert!(read(&dir, MANAGED_FILE).contains("alias a='ls'"));
    }

    #[test]
    fn a_looser_managed_folder_is_tightened() {
        let (dir, shell) = home();
        let managed_dir = dir.path().join(".config/hubergamof");
        fs::create_dir_all(&managed_dir).unwrap();
        fs::set_permissions(&managed_dir, Permissions::from_mode(0o755)).unwrap();
        shell.apply(&[alias("a", "ls", true)]).unwrap();
        assert_eq!(mode(&dir, ".config/hubergamof"), MANAGED_DIR_MODE);
    }

    #[test]
    fn an_integrated_alias_is_commented_out_in_its_file_and_defined_by_ours() {
        let (dir, shell) = home();
        fs::write(
            dir.path().join(".bashrc"),
            "alias ll='ls -la'\nexport A=1\n",
        )
        .unwrap();
        fs::write(dir.path().join(".bash_aliases"), "alias ll=other\n").unwrap();
        shell.apply(&[alias("ll", "ls -la", true)]).unwrap();
        let bashrc = read(&dir, ".bashrc");
        assert!(bashrc.starts_with("#hubergamof-off# alias ll='ls -la'\nexport A=1\n"));
        assert_eq!(
            read(&dir, ".bash_aliases"),
            "#hubergamof-off# alias ll=other\n"
        );
        assert!(read(&dir, ".config/hubergamof/aliases.sh").contains("alias ll='ls -la'"));
        assert_eq!(shell.read_system_aliases(), []);
    }

    #[test]
    fn a_disabled_alias_is_gone_from_the_machine_and_comes_back_enabled() {
        let (dir, shell) = home();
        fs::write(dir.path().join(".bashrc"), "alias ll=ls\n").unwrap();
        shell.apply(&[alias("ll", "ls", false)]).unwrap();
        assert!(read(&dir, ".bashrc").starts_with("#hubergamof-off# alias ll=ls\n"));
        assert!(!dir.path().join(MANAGED_FILE).exists());
        shell.apply(&[alias("ll", "ls", true)]).unwrap();
        assert!(read(&dir, ".config/hubergamof/aliases.sh").contains("alias ll='ls'"));
    }

    #[test]
    fn a_deleted_alias_gives_its_original_line_back() {
        let (dir, shell) = home();
        fs::write(dir.path().join(".bashrc"), "alias ll=ls\n").unwrap();
        shell.apply(&[alias("ll", "ls", true)]).unwrap();
        shell.apply(&[]).unwrap();
        assert_eq!(read(&dir, ".bashrc"), "alias ll=ls\n");
        assert_eq!(shell.read_system_aliases().len(), 1);
    }

    #[test]
    fn startup_files_without_a_claimed_alias_are_not_rewritten() {
        let (dir, shell) = home();
        let path = dir.path().join(".zsh_aliases");
        fs::write(&path, "alias g=git\n").unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        shell.apply(&[alias("ll", "ls", true)]).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    }
}
