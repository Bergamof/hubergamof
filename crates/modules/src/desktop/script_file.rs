//! Private files of the compositor scripts, in `$XDG_RUNTIME_DIR/hubergamof/`
//! (see [`crate::runtime_files`] for what is checked).
//!
//! Synchronous: called through `spawn_blocking`.

use std::path::{Path, PathBuf};

use crate::runtime_files::PrivateFolder;

/// Writes `script` as `name` in the private folder of `runtime_dir`
/// (created if needed), replacing an earlier file, and returns its path.
///
/// # Errors
///
/// A description of what failed or was refused.
pub(super) fn write(runtime_dir: &Path, name: &str, script: &str) -> Result<PathBuf, String> {
    PrivateFolder::open(runtime_dir)?.write_new(name, script.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::runtime_files::testing::runtime_dir;

    use super::*;

    #[test]
    fn scripts_are_written_in_the_private_folder_and_replaced() {
        let runtime = runtime_dir();
        let first = write(runtime.path(), "windows.js", "first").unwrap();
        assert_eq!(
            first,
            runtime
                .path()
                .join(crate::runtime_files::FOLDER_NAME)
                .join("windows.js")
        );
        let second = write(runtime.path(), "windows.js", "second").unwrap();
        assert_eq!(fs::read_to_string(second).unwrap(), "second");
    }
}
