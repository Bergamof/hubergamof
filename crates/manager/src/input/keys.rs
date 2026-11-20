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
    /// A combination that logs out, restarts or switches the console.
    #[error("reserved combination (Ctrl+Alt with Delete, Backspace or a function key)")]
    Reserved,
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
            // Digits only, no sign and no leading zero: `f+1` and `f01` name nothing.
            let digits = &lower[1..];
            if digits.starts_with('0') || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            FUNCTION_KEYS
                .get(digits.parse::<usize>().ok()?.checked_sub(1)?)
                .copied()
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
    if is_reserved(&keys) {
        return Err(ShortcutError::Reserved);
    }
    Ok(keys)
}

/// Ctrl+Alt with Delete (the logout dialog), Backspace (the end of the graphical
/// session on some desktops) or a function key (a switch of console): not
/// something a mouse button should do by itself.
fn is_reserved(keys: &[Key]) -> bool {
    keys.contains(&Key::LeftCtrl)
        && keys.contains(&Key::LeftAlt)
        && keys
            .iter()
            .any(|key| *key == Key::Delete || *key == Key::Backspace || FUNCTION_KEYS.contains(key))
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

    /// The names and the keys they stand for, written by hand (not read from
    /// the table they check).
    const REFERENCE: [(&str, Key); 36] = [
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

    #[test]
    fn every_name_stands_for_its_key_in_any_case() {
        for (name, key) in REFERENCE {
            assert_eq!(key_named(name), Some(key), "{name}");
            assert_eq!(key_named(&name.to_ascii_uppercase()), Some(key), "{name}");
        }
    }

    #[test]
    fn letters_digits_and_function_keys_are_each_their_own() {
        for letter in 'a'..='z' {
            let key = key_named(&letter.to_string()).unwrap();
            assert_eq!(format!("{key:?}"), letter.to_ascii_uppercase().to_string());
            assert_eq!(
                key_named(&letter.to_ascii_uppercase().to_string()),
                Some(key)
            );
        }
        for digit in 0..=9 {
            let key = key_named(&digit.to_string()).unwrap();
            assert_eq!(format!("{key:?}"), format!("Num{digit}"));
        }
        for number in 1..=12 {
            let key = key_named(&format!("f{number}")).unwrap();
            assert_eq!(format!("{key:?}"), format!("F{number}"));
            assert_eq!(key_named(&format!("F{number}")), Some(key));
        }
    }

    #[test]
    fn what_is_not_in_the_list_is_refused() {
        for name in [
            "",
            "f0",
            "f13",
            "ff",
            "f-1",
            "f+1",
            "f01",
            "f001",
            "f012",
            "f 1",
            "f１",
            "10",
            "power",
            "sysrq",
            "capslock",
            "sleep",
            "poweroff",
            "wakeup",
            "compose",
            "ab",
            "é",
            "ſ",
            "İ",
            "Ｃtrl",
            "ctrl ",
            " ctrl",
            "KEY_POWER",
            "btn_left",
            "pgup",
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
        assert_eq!(
            parse_shortcut("cTrL+sHiFt+m"),
            Ok(vec![Key::LeftCtrl, Key::LeftShift, Key::M])
        );
        assert_eq!(
            parse_shortcut("Control+Return"),
            Ok(vec![Key::LeftCtrl, Key::Enter])
        );
        assert_eq!(parse_shortcut("Win+Esc"), Ok(vec![Key::LeftMeta, Key::Esc]));
    }

    #[test]
    fn shortcuts_the_contract_accepts_are_known_or_refused_with_the_name() {
        assert_eq!(
            parse_shortcut("Ctrl+Alt+Delete"),
            Err(ShortcutError::Reserved)
        );
        assert_eq!(parse_shortcut("Super+L"), Ok(vec![Key::LeftMeta, Key::L]));
        assert_eq!(
            parse_shortcut("Ctrl+Shift+Esc"),
            Ok(vec![Key::LeftCtrl, Key::LeftShift, Key::Esc])
        );
        assert_eq!(
            parse_shortcut("a-b"),
            Err(ShortcutError::UnknownKey("a-b".to_owned()))
        );
        assert_eq!(
            parse_shortcut("x_y"),
            Err(ShortcutError::UnknownKey("x_y".to_owned()))
        );
    }

    #[test]
    fn what_the_contract_validates_never_makes_the_parser_fail_badly() {
        use hubergamof_protocol::{ButtonAction, MouseConfig, MouseControlId, MouseModel};
        for keys in [
            "Ctrl+Alt+Delete",
            "a-b",
            "x_y",
            "Ctrl+Power",
            "F13",
            "Ctrl+Shift+Alt+Super+A+B+C",
        ] {
            let mut config = MouseConfig::default_for("s", MouseModel::MxMaster4);
            config.bindings.insert(
                MouseControlId::Back,
                ButtonAction::Shortcut {
                    keys: keys.to_owned(),
                },
            );
            // Valid for the contract: the characters are right. The list decides.
            assert!(config.validate().is_ok(), "{keys}");
            let parsed = parse_shortcut(keys);
            assert!(
                matches!(
                    parsed,
                    Ok(_)
                        | Err(ShortcutError::UnknownKey(_)
                            | ShortcutError::TooLong
                            | ShortcutError::Reserved)
                ),
                "{keys}: {parsed:?}"
            );
        }
    }

    #[test]
    fn combinations_that_log_out_or_switch_console_are_reserved() {
        for shortcut in [
            "Ctrl+Alt+Delete",
            "alt+ctrl+del",
            "Ctrl+Alt+Backspace",
            "Ctrl+Alt+F1",
            "Ctrl+Alt+F12",
            "Ctrl+Alt+Shift+F3",
        ] {
            assert_eq!(
                parse_shortcut(shortcut),
                Err(ShortcutError::Reserved),
                "{shortcut}"
            );
        }
        for shortcut in [
            "Ctrl+Delete",
            "Alt+F4",
            "Ctrl+Alt+T",
            "Ctrl+Alt+Left",
            "Alt+Delete",
        ] {
            assert!(parse_shortcut(shortcut).is_ok(), "{shortcut}");
        }
    }

    #[test]
    fn badly_formed_shortcuts_are_refused() {
        let unknown = |name: &str| Err(ShortcutError::UnknownKey(name.to_owned()));
        assert_eq!(parse_shortcut(""), Err(ShortcutError::Empty));
        assert_eq!(parse_shortcut("+"), unknown(""));
        assert_eq!(parse_shortcut("+Ctrl"), unknown(""));
        assert_eq!(parse_shortcut("Ctrl+"), unknown(""));
        assert_eq!(parse_shortcut("Ctrl++M"), unknown(""));
        assert_eq!(parse_shortcut("Ctrl + M"), unknown("Ctrl "));
        assert_eq!(parse_shortcut(" Ctrl+M"), unknown(" Ctrl"));
        assert_eq!(parse_shortcut("Ctrl+Power"), unknown("Power"));
        assert_eq!(parse_shortcut("Ctrl+é"), unknown("é"));
        assert_eq!(parse_shortcut("Ｃtrl+M"), unknown("Ｃtrl"));
    }

    #[test]
    fn a_shortcut_is_at_most_six_keys_and_the_unknown_key_is_reported_first() {
        assert!(
            parse_shortcut("a+b+c+d+e+f").is_ok(),
            "six keys are allowed"
        );
        assert_eq!(parse_shortcut("a+b+c+d+e+f+g"), Err(ShortcutError::TooLong));
        assert_eq!(
            parse_shortcut("a+b+c+d+e+f+power"),
            Err(ShortcutError::UnknownKey("power".to_owned())),
            "the name is checked before the length"
        );
        assert!(
            parse_shortcut("ctrl+control").is_ok(),
            "a repeated key is pressed twice, which is harmless"
        );
    }

    #[test]
    fn the_keys_the_virtual_device_declares_are_exactly_the_ones_a_shortcut_can_name() {
        let all = all_keys();
        // 26 letters, 10 digits, 12 function keys, 30 other keys (36 names).
        assert_eq!(all.len(), 78);
        let distinct: std::collections::BTreeSet<_> = all.iter().collect();
        assert_eq!(distinct.len(), all.len());
        let nameable: std::collections::BTreeSet<Key> = ('a'..='z')
            .map(|letter| letter.to_string())
            .chain((0..=9).map(|digit| digit.to_string()))
            .chain((1..=12).map(|number| format!("f{number}")))
            .chain(REFERENCE.iter().map(|(name, _)| (*name).to_owned()))
            .filter_map(|name| key_named(&name))
            .collect();
        assert_eq!(nameable, all.iter().copied().collect());
        for dangerous in [
            Key::Power,
            Key::Sleep,
            Key::Wakeup,
            Key::Sysrq,
            Key::CapsLock,
            Key::Compose,
            Key::ButtonLeft,
            Key::ButtonRight,
        ] {
            assert!(!all.contains(&dangerous), "{dangerous:?}");
        }
    }
}
