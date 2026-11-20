//! The closed list of keys a shortcut may press.

use input_linux::Key;
use thiserror::Error;

/// Most keys in a shortcut.
pub const MAX_SHORTCUT_KEYS: usize = 6;

const LETTERS: [Key; 26] = [
    Key::A,
    Key::B,
    Key::C,
    Key::D,
    Key::E,
    Key::F,
    Key::G,
    Key::H,
    Key::I,
    Key::J,
    Key::K,
    Key::L,
    Key::M,
    Key::N,
    Key::O,
    Key::P,
    Key::Q,
    Key::R,
    Key::S,
    Key::T,
    Key::U,
    Key::V,
    Key::W,
    Key::X,
    Key::Y,
    Key::Z,
];

const DIGITS: [Key; 10] = [
    Key::Num0,
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
];

const FUNCTION_KEYS: [Key; 12] = [
    Key::F1,
    Key::F2,
    Key::F3,
    Key::F4,
    Key::F5,
    Key::F6,
    Key::F7,
    Key::F8,
    Key::F9,
    Key::F10,
    Key::F11,
    Key::F12,
];

/// The other keys, by their lower case names (several for one key).
const NAMED: &[(&str, Key)] = &[
    ("ctrl", Key::LeftCtrl),
    ("control", Key::LeftCtrl),
    ("shift", Key::LeftShift),
    ("alt", Key::LeftAlt),
    ("super", Key::LeftMeta),
    ("meta", Key::LeftMeta),
    ("win", Key::LeftMeta),
    ("enter", Key::Enter),
    ("return", Key::Enter),
    ("esc", Key::Esc),
    ("escape", Key::Esc),
    ("tab", Key::Tab),
    ("space", Key::Space),
    ("backspace", Key::Backspace),
    ("delete", Key::Delete),
    ("del", Key::Delete),
    ("insert", Key::Insert),
    ("home", Key::Home),
    ("end", Key::End),
    ("pageup", Key::PageUp),
    ("pagedown", Key::PageDown),
    ("up", Key::Up),
    ("down", Key::Down),
    ("left", Key::Left),
    ("right", Key::Right),
    ("minus", Key::Minus),
    ("equal", Key::Equal),
    ("comma", Key::Comma),
    ("dot", Key::Dot),
    ("slash", Key::Slash),
    ("playpause", Key::PlayPause),
    ("nextsong", Key::NextSong),
    ("previoussong", Key::PreviousSong),
    ("mute", Key::Mute),
    ("volumeup", Key::VolumeUp),
    ("volumedown", Key::VolumeDown),
];

/// Why a shortcut is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ShortcutError {
    /// No keys.
    #[error("empty shortcut")]
    Empty,
    /// More than [`MAX_SHORTCUT_KEYS`].
    #[error("more than {MAX_SHORTCUT_KEYS} keys")]
    TooLong,
    /// A key that is not in the list.
    #[error("unknown key `{0}`")]
    UnknownKey(String),
}

/// The key a name stands for, in any case, `None` outside the list.
#[must_use]
pub fn key_named(name: &str) -> Option<Key> {
    let lower = name.to_ascii_lowercase();
    let mut chars = lower.chars();
    match (chars.next(), chars.next()) {
        (Some(letter @ 'a'..='z'), None) => LETTERS.get(usize::from(letter as u8 - b'a')).copied(),
        (Some(digit @ '0'..='9'), None) => DIGITS.get(usize::from(digit as u8 - b'0')).copied(),
        (Some('f'), Some(_)) => {
            let number: usize = lower[1..].parse().ok()?;
            FUNCTION_KEYS.get(number.checked_sub(1)?).copied()
        }
        _ => NAMED
            .iter()
            .find(|(candidate, _)| *candidate == lower)
            .map(|(_, key)| *key),
    }
}

/// The keys of a shortcut written `Ctrl+Shift+M`.
///
/// # Errors
/// [`ShortcutError`] for a shortcut without keys, with too many, or naming a
/// key outside the list.
pub fn parse_shortcut(text: &str) -> Result<Vec<Key>, ShortcutError> {
    if text.is_empty() {
        return Err(ShortcutError::Empty);
    }
    let keys = text
        .split('+')
        .map(|name| key_named(name).ok_or_else(|| ShortcutError::UnknownKey(name.to_owned())))
        .collect::<Result<Vec<_>, _>>()?;
    if keys.len() > MAX_SHORTCUT_KEYS {
        return Err(ShortcutError::TooLong);
    }
    Ok(keys)
}

/// Every key a shortcut can press, once each.
pub(super) fn all_keys() -> Vec<Key> {
    let mut keys: Vec<Key> = LETTERS
        .into_iter()
        .chain(DIGITS)
        .chain(FUNCTION_KEYS)
        .chain(NAMED.iter().map(|(_, key)| *key))
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_digits_and_function_keys_are_named_in_any_case() {
        assert_eq!(key_named("a"), Some(Key::A));
        assert_eq!(key_named("M"), Some(Key::M));
        assert_eq!(key_named("z"), Some(Key::Z));
        assert_eq!(key_named("0"), Some(Key::Num0));
        assert_eq!(key_named("9"), Some(Key::Num9));
        assert_eq!(key_named("F1"), Some(Key::F1));
        assert_eq!(key_named("f12"), Some(Key::F12));
    }

    #[test]
    fn named_keys_and_their_aliases() {
        assert_eq!(key_named("Ctrl"), Some(Key::LeftCtrl));
        assert_eq!(key_named("CONTROL"), Some(Key::LeftCtrl));
        assert_eq!(key_named("Super"), key_named("meta"));
        assert_eq!(key_named("Return"), Some(Key::Enter));
        assert_eq!(key_named("PageDown"), Some(Key::PageDown));
        assert_eq!(key_named("VolumeUp"), Some(Key::VolumeUp));
    }

    #[test]
    fn what_is_not_in_the_list_is_refused() {
        for name in [
            "",
            "f0",
            "f13",
            "ff",
            "10",
            "power",
            "sysrq",
            "capslock",
            "sleep",
            "poweroff",
            "ab",
            "é",
            "ctrl ",
            "KEY_POWER",
            "btn_left",
        ] {
            assert_eq!(key_named(name), None, "{name:?}");
        }
    }

    #[test]
    fn a_shortcut_is_its_keys_in_order() {
        assert_eq!(
            parse_shortcut("Ctrl+Shift+M"),
            Ok(vec![Key::LeftCtrl, Key::LeftShift, Key::M])
        );
        assert_eq!(parse_shortcut("F5"), Ok(vec![Key::F5]));
    }

    #[test]
    fn a_shortcut_with_nothing_or_too_much_or_an_unknown_key_is_refused() {
        assert_eq!(parse_shortcut(""), Err(ShortcutError::Empty));
        assert_eq!(
            parse_shortcut("Ctrl++M"),
            Err(ShortcutError::UnknownKey(String::new()))
        );
        assert_eq!(
            parse_shortcut("Ctrl+Power"),
            Err(ShortcutError::UnknownKey("Power".to_owned()))
        );
        assert_eq!(parse_shortcut("a+b+c+d+e+f+g"), Err(ShortcutError::TooLong));
        assert!(
            parse_shortcut("a+b+c+d+e+f").is_ok(),
            "six keys are allowed"
        );
    }

    #[test]
    fn everything_the_protocol_accepts_as_a_shortcut_is_either_known_or_refused_cleanly() {
        // The contract only checks characters: the list is what limits.
        for shortcut in ["Ctrl+Alt+Delete", "Super+L", "Ctrl+Shift+Esc", "a-b", "x_y"] {
            let parsed = parse_shortcut(shortcut);
            assert!(parsed.is_ok() || matches!(parsed, Err(ShortcutError::UnknownKey(_))));
        }
    }

    #[test]
    fn the_list_of_keys_has_no_duplicate_and_every_name_resolves() {
        let all = all_keys();
        let distinct: std::collections::BTreeSet<_> = all.iter().collect();
        assert_eq!(distinct.len(), all.len());
        for (name, key) in NAMED {
            assert_eq!(key_named(name), Some(*key));
        }
        assert_eq!(
            all.len(),
            26 + 10 + 12 + {
                let named: std::collections::BTreeSet<_> = NAMED.iter().map(|(_, k)| k).collect();
                named.len()
            }
        );
    }
}
