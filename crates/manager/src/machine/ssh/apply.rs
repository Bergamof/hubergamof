//! Putting the SSH hosts on the computer: the file HuBergamof keeps in
//! `~/.config/hubergamof`, and the block that includes it at the top of
//! `~/.ssh/config`. Synchronous: callers run it on a blocking task.
//!
//! `ssh` takes the first value it finds, and an `Include` read after a `Host`
//! would only be read for that host: the block goes at the very start of the
//! file, so the hosts of HuBergamof come first and win.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use hubergamof_protocol::SshConfig;
use thiserror::Error;

use super::claim::claim_originals;
use super::render::{MANAGED_DIR, MANAGED_FILE, UnwritableHost, render};
use crate::machine::block::Block;
use crate::machine::files::{
    FileError, PRIVATE_FILE_MODE, ensure_private_dir, io_error, remove_file_if_present,
    update_text_file, write_replacing,
};

/// The folder of the SSH files of the user, relative to the home folder.
const SSH_DIR: &str = ".ssh";

/// The configuration file of the user, in [`SSH_DIR`].
const CONFIG_FILE: &str = "config";

/// First line of the block.
const BLOCK_START: &str = "# >>> hubergamof ssh >>>";

/// Last line of the block.
const BLOCK_END: &str = "# <<< hubergamof ssh <<<";

/// Why the SSH hosts could not be put on the computer.
#[derive(Debug, Error)]
pub enum SshFilesError {
    /// A file could not be read or written.
    #[error(transparent)]
    File(#[from] FileError),
    /// A host that cannot be written safely.
    #[error(transparent)]
    Unwritable(#[from] UnwritableHost),
}

/// The SSH files of one user, under `home`.
#[derive(Debug, Clone)]
pub struct SshFiles {
    pub(super) home: PathBuf,
}

/// What the block holds: the include of the managed file.
fn include_body() -> String {
    format!("Include ~/{MANAGED_DIR}/{MANAGED_FILE}\n")
}

fn include_block(body: &str) -> Block<'_> {
    Block {
        start: BLOCK_START,
        end: BLOCK_END,
        body,
    }
}

impl SshFiles {
    /// The SSH files under `home` (a test or demo folder, or the real one).
    #[must_use]
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }

    /// The SSH files of the user running the manager, when its home folder is
    /// known and absolute (a relative one would resolve against the
    /// manager's folder).
    #[must_use]
    pub fn of_current_user() -> Option<Self> {
        std::env::home_dir()
            .filter(|home| home.is_absolute())
            .map(Self::new)
    }

    /// The home folder whose files are read and written.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.home
    }

    fn managed_file(&self) -> PathBuf {
        self.home.join(MANAGED_DIR).join(MANAGED_FILE)
    }

    pub(super) fn config_file(&self) -> PathBuf {
        self.home.join(SSH_DIR).join(CONFIG_FILE)
    }

    /// Puts `config` on the computer: its enabled hosts are written in the file
    /// of HuBergamof and included from `~/.ssh/config`. With none enabled the
    /// file and the include are removed. Then the original section of every
    /// host of the configuration, enabled or not, is commented out in
    /// `~/.ssh/config` (so that a host that is off is really gone), and those
    /// of a host that left the configuration are given back.
    ///
    /// # Errors
    /// [`SshFilesError`] for the first file that cannot be written; each write
    /// is atomic, so no file is left half written.
    pub fn apply(&self, config: &SshConfig) -> Result<(), SshFilesError> {
        if let Some(content) = render(config)? {
            let dir = self.home.join(MANAGED_DIR);
            ensure_private_dir(&dir)?;
            write_replacing(&self.managed_file(), content.as_bytes(), PRIVATE_FILE_MODE)?;
            self.include()?;
        } else {
            // The include first: a configuration that cannot be rewritten must
            // not keep loading what is removed.
            self.exclude()?;
            remove_file_if_present(&self.managed_file())?;
        }
        // After the file of HuBergamof, so that an enabled host is never missing.
        self.claim_originals(config)
    }

    fn claim_originals(&self, config: &SshConfig) -> Result<(), SshFilesError> {
        let aliases: HashSet<&str> = config
            .hosts
            .iter()
            .map(|host| host.alias.as_str())
            .collect();
        update_text_file(&self.config_file(), |content| {
            Ok(claim_originals(content, &aliases))
        })?;
        Ok(())
    }

    /// Puts the include first in `~/.ssh/config`, creating it (and `~/.ssh`)
    /// when there is none.
    fn include(&self) -> Result<(), SshFilesError> {
        let path = self.config_file();
        let body = include_body();
        let block = include_block(&body);
        if exists(&path)? {
            update_text_file(&path, |content| block.place_first(content))?;
        } else {
            ensure_private_dir(&self.home.join(SSH_DIR))?;
            write_replacing(&path, block.text().as_bytes(), PRIVATE_FILE_MODE)?;
        }
        Ok(())
    }

    /// Removes the include from `~/.ssh/config`. A file that only held it (one
    /// this manager created) goes away.
    fn exclude(&self) -> Result<(), SshFilesError> {
        let path = self.config_file();
        let body = include_body();
        let block = include_block(&body);
        let mut had_block = false;
        update_text_file(&path, |content| {
            had_block = content.contains(BLOCK_START);
            block.remove(content)
        })?;
        // An empty file the user made is theirs: only one that held our block,
        // and nothing else, was ours to make.
        if had_block {
            remove_if_empty(&path)?;
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
