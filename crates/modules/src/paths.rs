//! Paths written in the settings by hand (see
//! [`bergadeck_module_api::expand_home`]).

use std::path::{Path, PathBuf};

use bergadeck_module_api::expand_home;

/// The home folder of the user running the manager, when known and
/// absolute (a relative one would resolve against the manager's folder).
pub(crate) fn user_home() -> Option<PathBuf> {
    std::env::home_dir().filter(|home| home.is_absolute())
}

/// `path` with a leading `~/` replaced by the home directory of the user
/// running the manager.
pub(crate) fn expand_user_home(path: &Path) -> Option<PathBuf> {
    expand_home(path, user_home().as_deref())
}
