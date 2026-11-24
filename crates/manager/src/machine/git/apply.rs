//! Putting the Git configuration on the computer: the files HuBergamof keeps
//! in `~/.config/hubergamof`, and the block that includes them in the Git
//! configuration of the user. Synchronous: callers run it on a blocking task.
//!
//! Git reads `~/.gitconfig` and `~/.config/git/config`; the later a value,
//! the more it counts, so the block goes at the end of the file, and HuBergamof's
//! values win over the ones before it. Git ignores an include whose file does
//! not exist, so no condition is needed.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hubergamof_protocol::GitConfig;
use thiserror::Error;

use super::render::{
    FOLDER_FILE_PREFIX, MAIN_FILE, MANAGED_DIR, Rendered, UnwritableValue, render,
};
use super::system::{GIT_PROGRAM, GIT_TIMEOUT};
use crate::machine::block::Block;
use crate::machine::files::{
    FileError, PRIVATE_FILE_MODE, ensure_private_dir, io_error, remove_file_if_present,
    update_text_file, write_replacing,
};

/// The Git configuration files of the user, relative to the home folder, the
/// first one that exists getting the block.
const GLOBAL_FILES: [&str; 2] = [".gitconfig", ".config/git/config"];

/// First line of the block.
const BLOCK_START: &str = "# >>> hubergamof git >>>";

/// Last line of the block.
const BLOCK_END: &str = "# <<< hubergamof git <<<";

/// Permissions of a Git configuration file the manager has to create: Git's
/// usual ones, since it holds nothing secret.
const CREATED_GLOBAL_MODE: u32 = 0o644;

/// Why the Git configuration could not be put on the computer.
#[derive(Debug, Error)]
pub enum GitFilesError {
    /// A file could not be read or written.
    #[error(transparent)]
    File(#[from] FileError),
    /// A value that cannot be written safely.
    #[error(transparent)]
    Unwritable(#[from] UnwritableValue),
    /// A folder of an identity is, or goes through, a link: the rule would
    /// apply to repositories somewhere else than where it says.
    #[error("the folder {0} goes through a link")]
    LinkedFolder(String),
}

/// The Git files of one user, under `home`.
#[derive(Debug, Clone)]
pub struct GitFiles {
    pub(super) home: PathBuf,
    /// Whether `/etc/gitconfig` is read besides the user's files.
    pub(super) with_system_config: bool,
    /// The program that lists the configuration: `git`, unless a test says otherwise.
    pub(super) program: OsString,
    /// How long it may take.
    pub(super) timeout: Duration,
}

/// What the block holds: the include of the main file.
fn include_body() -> String {
    format!("[include]\n\tpath = ~/{MANAGED_DIR}/{MAIN_FILE}\n")
}

fn include_block(body: &str) -> Block<'_> {
    Block {
        start: BLOCK_START,
        end: BLOCK_END,
        body,
    }
}

impl GitFiles {
    /// The Git files under `home` (a test or demo folder, or the real one).
    /// Only the user's files are read when `with_system_config` is false: the
    /// listing of a demo or a test does not depend on the computer.
    #[must_use]
    pub fn new(home: PathBuf, with_system_config: bool) -> Self {
        Self {
            home,
            with_system_config,
            program: OsString::from(GIT_PROGRAM),
            timeout: GIT_TIMEOUT,
        }
    }

    /// The home folder whose files are read and written.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The Git files of the user running the manager, when its home folder is
    /// known and absolute (a relative one would resolve against the
    /// manager's folder).
    #[must_use]
    pub fn of_current_user() -> Option<Self> {
        std::env::home_dir()
            .filter(|home| home.is_absolute())
            .map(|home| Self::new(home, true))
    }

    fn managed_dir(&self) -> PathBuf {
        self.home.join(MANAGED_DIR)
    }

    /// Puts `config` on the computer: its enabled values are written in the
    /// files of HuBergamof and included from the Git configuration of the
    /// user. With nothing enabled the files and the include are removed, and
    /// the Git configuration of the user is as it was.
    ///
    /// # Errors
    /// [`GitFilesError`] for the first file that cannot be written; each
    /// write is atomic, so no file is left half written.
    pub fn apply(&self, config: &GitConfig) -> Result<(), GitFilesError> {
        if let Some(rendered) = render(config)? {
            self.check_folders(config)?;
            self.write(&rendered)?;
            self.include()
        } else {
            // The include first: a Git configuration that cannot be
            // rewritten must not keep loading what is removed.
            self.exclude()?;
            self.remove_stale_folder_files(&[])?;
            Ok(remove_file_if_present(&self.managed_dir().join(MAIN_FILE))?)
        }
    }

    fn write(&self, rendered: &Rendered) -> Result<(), GitFilesError> {
        let dir = self.managed_dir();
        ensure_private_dir(&dir)?;
        // The folder files first: the main file only names them.
        for file in rendered.folders.iter().chain([&rendered.main]) {
            write_replacing(
                &dir.join(&file.name),
                file.content.as_bytes(),
                PRIVATE_FILE_MODE,
            )?;
        }
        let kept: Vec<&str> = rendered
            .folders
            .iter()
            .map(|file| file.name.as_str())
            .collect();
        self.remove_stale_folder_files(&kept)
    }

    /// Removes the files of identities by folder that are not in `kept`.
    fn remove_stale_folder_files(&self, kept: &[&str]) -> Result<(), GitFilesError> {
        let dir = self.managed_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io_error(&dir, error).into()),
        };
        for entry in entries {
            let entry = entry.map_err(|error| io_error(&dir, error))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(FOLDER_FILE_PREFIX) && !kept.contains(&name.as_str()) {
                remove_file_if_present(&entry.path())?;
            }
        }
        Ok(())
    }

    /// Refuses the folder of an enabled rule that exists as a link or under
    /// one: Git matches the real path too, so the rule would reach
    /// repositories the user did not mean. A folder not created yet is fine,
    /// and a rule that is off writes nothing, so it can stay (or be removed).
    fn check_folders(&self, config: &GitConfig) -> Result<(), GitFilesError> {
        for rule in config.folder_identities.iter().filter(|rule| rule.enabled) {
            let Some(relative) = rule.folder.strip_prefix("~/") else {
                continue;
            };
            let mut path = self.home.clone();
            for part in relative.split('/') {
                path.push(part);
                match fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err(GitFilesError::LinkedFolder(rule.folder.clone()));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                    Err(error) => return Err(io_error(&path, error).into()),
                }
            }
        }
        Ok(())
    }

    /// The file of the user's Git configuration that gets the block, and the
    /// others that may hold one: the first that exists, else `~/.gitconfig`.
    fn global_files(&self) -> Result<(PathBuf, Vec<PathBuf>), GitFilesError> {
        let paths: Vec<PathBuf> = GLOBAL_FILES
            .iter()
            .map(|relative| self.home.join(relative))
            .collect();
        let mut target = None;
        for path in &paths {
            if exists(path)? {
                target = Some(path.clone());
                break;
            }
        }
        let target = target.unwrap_or_else(|| paths[0].clone());
        let others = paths.into_iter().filter(|path| *path != target).collect();
        Ok((target, others))
    }

    /// Adds the include to the Git configuration of the user (creating
    /// `~/.gitconfig` when there is none), and removes it from the other file.
    fn include(&self) -> Result<(), GitFilesError> {
        let (target, others) = self.global_files()?;
        let body = include_body();
        let block = include_block(&body);
        if exists(&target)? {
            update_text_file(&target, |content| block.insert(content))?;
        } else {
            write_replacing(&target, block.text().as_bytes(), CREATED_GLOBAL_MODE)?;
        }
        for other in others {
            update_text_file(&other, |content| block.remove(content))?;
        }
        Ok(())
    }

    /// Removes the include from the Git configuration files of the user. A
    /// file that only held it (one this manager created) goes away.
    fn exclude(&self) -> Result<(), GitFilesError> {
        let body = include_body();
        let block = include_block(&body);
        for relative in GLOBAL_FILES {
            let path = self.home.join(relative);
            let mut had_block = false;
            update_text_file(&path, |content| {
                had_block = content.contains(BLOCK_START);
                block.remove(content)
            })?;
            // An empty file the user made is theirs: only one that held our
            // block, and nothing else, was ours to make.
            if had_block {
                remove_if_empty(&path)?;
            }
        }
        Ok(())
    }
}

/// Whether `path` exists, a link followed. A link to nothing is refused rather
/// than replaced: it is the user's.
fn exists(path: &Path) -> Result<bool, FileError> {
    match fs::symlink_metadata(path) {
        Ok(_) => match fs::canonicalize(path) {
            Ok(_) => Ok(true),
            Err(_) => Err(FileError::DanglingLink(path.to_owned())),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(path, error)),
    }
}

/// Removes `path` when it is an empty regular file (not a link).
fn remove_if_empty(path: &Path) -> Result<(), FileError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() == 0 => remove_file_if_present(path),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(path, error)),
    }
}
