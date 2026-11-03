//! Paths written in the settings by hand (see
//! [`bergadeck_module_api::expand_home`]).

use std::path::{Path, PathBuf};

use bergadeck_module_api::expand_home;

/// `path` with a leading `~/` replaced by the home directory of the user
/// running the manager.
pub(crate) fn expand_user_home(path: &Path) -> Option<PathBuf> {
    expand_home(path, std::env::home_dir().as_deref())
}
