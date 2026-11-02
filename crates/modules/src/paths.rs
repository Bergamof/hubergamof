//! Paths written in the settings by hand.

use std::path::{Path, PathBuf};

/// Prefix standing for the home directory of the user.
const HOME_PREFIX: &str = "~";

/// `path` with a leading `~/` replaced by `home`; `None` when it must be
/// replaced but no home is known, or for `~` alone (the home directory is
/// never what a setting names). `~user/…` is left as is.
pub(crate) fn expand_home(path: &Path, home: Option<&Path>) -> Option<PathBuf> {
    match path.strip_prefix(HOME_PREFIX) {
        Ok(rest) if rest.as_os_str().is_empty() => None,
        Ok(rest) => home.map(|home| home.join(rest)),
        Err(_) => Some(path.to_path_buf()),
    }
}

/// `path` with a leading `~` replaced by the home directory of the user
/// running the manager.
pub(crate) fn expand_user_home(path: &Path) -> Option<PathBuf> {
    expand_home(path, std::env::home_dir().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_tilde_stands_for_the_home_directory() {
        let home = Some(Path::new("/home/me"));
        let expand = |path: &str| expand_home(Path::new(path), home);
        assert_eq!(
            expand("~/projects/app"),
            Some("/home/me/projects/app".into())
        );
        assert_eq!(expand("~"), None, "the home itself");
        assert_eq!(expand("~/"), None);
        assert_eq!(expand("/srv/app"), Some("/srv/app".into()));
        assert_eq!(
            expand("~other/app"),
            Some("~other/app".into()),
            "not a user"
        );
        assert_eq!(expand("app/~/x"), Some("app/~/x".into()));
        assert_eq!(expand_home(Path::new("~/app"), None), None);
        assert_eq!(expand_home(Path::new("/app"), None), Some("/app".into()));
    }
}
