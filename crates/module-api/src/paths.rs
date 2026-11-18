//! Paths written in the settings by hand: absolute, or relative to the home
//! directory with a leading `~/`. The same rule checks the settings
//! ([`crate::SettingKind`]) and resolves them in modules.

use std::path::{Path, PathBuf};

use crate::file_entries::is_hidden_control;

/// Prefix standing for the home directory of the user.
const HOME_PREFIX: &str = "~";

/// `path` with a leading `~/` replaced by `home`; `None` when it must be
/// replaced but no home is known, or for `~` alone (the home directory is
/// never what a setting names). Any other path, `~user/…` included, is left
/// as is.
#[must_use]
pub fn expand_home(path: &Path, home: Option<&Path>) -> Option<PathBuf> {
    match path.strip_prefix(HOME_PREFIX) {
        Ok(rest) if rest.as_os_str().is_empty() => None,
        Ok(rest) => home.map(|home| home.join(rest)),
        Err(_) => Some(path.to_path_buf()),
    }
}

/// Whether a setting may hold `path`: absolute once [`expand_home`]d, and free
/// of characters that could forge lines in the logs where it is reported.
#[must_use]
pub fn is_setting_path(path: &str) -> bool {
    !path.chars().any(is_hidden_control)
        && expand_home(Path::new(path), Some(Path::new("/"))).is_some_and(|path| path.is_absolute())
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

    #[test]
    fn settings_hold_absolute_or_home_paths_only() {
        for path in ["/srv/app", "~/app", "~/a/b.yaml"] {
            assert!(is_setting_path(path), "{path}");
        }
        for path in ["", "app", "./app", "~", "~/", "~/.", "~/./", "~other/app"] {
            assert!(!is_setting_path(path), "{path}");
        }
    }

    #[test]
    fn settings_refuse_paths_that_could_forge_log_lines() {
        for path in ["/a\nb", "/a\u{1b}[31mb", "~/a\u{202E}b", "/a\u{2066}b"] {
            assert!(!is_setting_path(path), "{path:?}");
        }
    }
}
