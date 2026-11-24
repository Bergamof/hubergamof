//! The settings the person writes themselves, for what the application does not
//! manage natively: a list of keys and values, shared by the SSH hosts and Git.
//!
//! No key is refused for what it does: `ssh` and `git` run programs from some
//! of them, which is the same trust as a shell alias (`docs/security.md`). What
//! is checked is the form, so that a setting is one line that cannot open
//! another section nor leave the file unreadable.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::text::is_invisible;

/// Most settings a list holds.
pub const MAX_ADVANCED_SETTINGS: usize = 64;

/// Longest key, in characters.
pub const MAX_SETTING_KEY_CHARS: usize = 64;

/// Longest value, in characters.
pub const MAX_SETTING_VALUE_CHARS: usize = 1024;

/// A setting written by the person: a key, a value, and whether it is on the
/// computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AdvancedSetting {
    /// What is set: an option of `ssh_config` (`ServerAliveInterval`) or a key
    /// of Git (`push.default`).
    pub key: String,
    /// Its value, on one line.
    pub value: String,
    /// Whether the setting is on the computer. A disabled one stays in the
    /// configuration and is removed from the computer.
    pub enabled: bool,
}

/// Whether `value` is one line of text: not empty, not blank at its edges, no
/// control or invisible character, not longer than [`MAX_SETTING_VALUE_CHARS`].
#[must_use]
pub fn is_one_line_value(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_SETTING_VALUE_CHARS
        && !value.starts_with(char::is_whitespace)
        && !value.ends_with(char::is_whitespace)
        && !value.chars().any(|c| c.is_control() || is_invisible(c))
}

/// Whether the quotes of `value`, double and single, come in pairs: `ssh`
/// refuses a whole configuration for a quote it cannot close.
#[must_use]
pub fn has_paired_quotes(value: &str) -> bool {
    value.matches('"').count().is_multiple_of(2) && value.matches('\'').count().is_multiple_of(2)
}

/// Whether `settings` is a list the contract accepts: at most
/// [`MAX_ADVANCED_SETTINGS`], each key valid by `key_ok` and each value by
/// `value_ok`, and no key twice (compared by `fold`, which says which keys are
/// the same). The key of the first fault is given.
///
/// # Errors
/// The key of the first setting that is wrong or repeated, or `None` for the list
/// being too long.
pub fn check_settings(
    settings: &[AdvancedSetting],
    key_ok: impl Fn(&str) -> bool,
    value_ok: impl Fn(&str) -> bool,
    fold: impl Fn(&str) -> String,
) -> Result<(), Option<String>> {
    if settings.len() > MAX_ADVANCED_SETTINGS {
        return Err(None);
    }
    let mut known = HashSet::with_capacity(settings.len());
    for setting in settings {
        if !key_ok(&setting.key) || !value_ok(&setting.value) || !known.insert(fold(&setting.key)) {
            return Err(Some(setting.key.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting(key: &str, value: &str) -> AdvancedSetting {
        AdvancedSetting {
            key: key.into(),
            value: value.into(),
            enabled: true,
        }
    }

    #[test]
    fn a_value_is_one_line_without_blanks_at_its_edges() {
        for good in [
            "30",
            "a b",
            "ssh -W %h:%p bastion",
            "é ✓",
            &"v".repeat(MAX_SETTING_VALUE_CHARS),
        ] {
            assert!(is_one_line_value(good), "{good}");
        }
        for bad in [
            "",
            " a",
            "a ",
            "a\nb",
            "a\rb",
            "a\0b",
            "a\tb",
            "a\u{202E}b",
            "a\u{200B}b",
        ] {
            assert!(!is_one_line_value(bad), "{bad:?}");
        }
        assert!(!is_one_line_value(&"v".repeat(MAX_SETTING_VALUE_CHARS + 1)));
    }

    #[test]
    fn quotes_come_in_pairs() {
        for good in ["a", "\"a b\"", "'a b'", "\"a\" \"b\"", ""] {
            assert!(has_paired_quotes(good), "{good}");
        }
        for bad in ["\"a", "a'", "\"a\" \"", "it's"] {
            assert!(!has_paired_quotes(bad), "{bad}");
        }
    }

    #[test]
    fn a_list_is_checked_for_its_length_its_forms_and_its_repeats() {
        let ok = |key: &str| !key.is_empty();
        let fold = |key: &str| key.to_ascii_lowercase();
        let list = [setting("A", "1"), setting("b", "2")];
        assert_eq!(check_settings(&list, ok, is_one_line_value, fold), Ok(()));

        let repeated = [setting("A", "1"), setting("a", "2")];
        assert_eq!(
            check_settings(&repeated, ok, is_one_line_value, fold),
            Err(Some("a".into()))
        );
        let wrong = [setting("A", "")];
        assert_eq!(
            check_settings(&wrong, ok, is_one_line_value, fold),
            Err(Some("A".into()))
        );
        let long: Vec<_> = (0..=MAX_ADVANCED_SETTINGS)
            .map(|i| setting(&format!("k{i}"), "v"))
            .collect();
        assert_eq!(
            check_settings(&long, ok, is_one_line_value, fold),
            Err(None)
        );
    }
}
