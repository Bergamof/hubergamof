//! Keys and buttons the manager presses on behalf of a mouse button: the ones
//! a person bound (a click, a shortcut), sent to the system through a virtual
//! input device.
//!
//! What can be pressed is a closed list ([`key_named`]): a shortcut names keys
//! from it or is refused, whatever a configuration says.

mod keys;
mod uinput;

use std::io;

use hubergamof_protocol::MouseButton;
pub use input_linux::Key;
pub use keys::{MAX_SHORTCUT_KEYS, ShortcutError, key_named, parse_shortcut};
pub use uinput::UinputInjector;

/// Sends key and button events to the system.
pub trait Injector: Send {
    /// Presses (`true`) or releases (`false`) a key or a button.
    ///
    /// # Errors
    /// The I/O error of the virtual device.
    fn send(&mut self, key: Key, pressed: bool) -> io::Result<()>;

    /// Presses then releases a key or a button.
    ///
    /// # Errors
    /// See [`Injector::send`].
    fn tap(&mut self, key: Key) -> io::Result<()> {
        self.send(key, true)?;
        self.send(key, false)
    }

    /// Presses the keys of a shortcut in order and releases them in reverse,
    /// whatever happens in between: a modifier left down would stay down for
    /// the whole session.
    ///
    /// # Errors
    /// The first error met.
    fn tap_shortcut(&mut self, keys: &[Key]) -> io::Result<()> {
        let mut first_error = None;
        let mut pressed = 0;
        for key in keys {
            match self.send(*key, true) {
                Ok(()) => pressed += 1,
                Err(error) => {
                    first_error = Some(error);
                    break;
                }
            }
        }
        for key in keys[..pressed].iter().rev() {
            if let Err(error) = self.send(*key, false) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

/// The key of the virtual device a mouse button is.
#[must_use]
pub const fn key_of_button(button: MouseButton) -> Key {
    match button {
        MouseButton::Left => Key::ButtonLeft,
        MouseButton::Right => Key::ButtonRight,
        MouseButton::Middle => Key::ButtonMiddle,
        MouseButton::Back => Key::ButtonSide,
        MouseButton::Forward => Key::ButtonExtra,
    }
}

/// An injector that records what it is told, for the tests.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct RecordingInjector {
    /// Every event, in order: the key and whether it went down.
    pub events: Vec<(Key, bool)>,
    /// The key whose pressing fails, when set.
    pub failing: Option<Key>,
}

#[cfg(test)]
impl Injector for RecordingInjector {
    fn send(&mut self, key: Key, pressed: bool) -> io::Result<()> {
        if pressed && self.failing == Some(key) {
            return Err(io::Error::other("refused"));
        }
        self.events.push((key, pressed));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tap_presses_then_releases() {
        let mut injector = RecordingInjector::default();
        injector.tap(Key::ButtonLeft).unwrap();
        assert_eq!(
            injector.events,
            [(Key::ButtonLeft, true), (Key::ButtonLeft, false)]
        );
    }

    #[test]
    fn a_shortcut_presses_in_order_and_releases_in_reverse() {
        let mut injector = RecordingInjector::default();
        injector
            .tap_shortcut(&[Key::LeftCtrl, Key::LeftShift, Key::M])
            .unwrap();
        assert_eq!(
            injector.events,
            [
                (Key::LeftCtrl, true),
                (Key::LeftShift, true),
                (Key::M, true),
                (Key::M, false),
                (Key::LeftShift, false),
                (Key::LeftCtrl, false),
            ]
        );
    }

    #[test]
    fn a_shortcut_that_fails_still_releases_what_it_pressed() {
        let mut injector = RecordingInjector {
            failing: Some(Key::M),
            ..RecordingInjector::default()
        };
        let result = injector.tap_shortcut(&[Key::LeftCtrl, Key::M]);
        assert!(result.is_err());
        assert_eq!(
            injector.events,
            [(Key::LeftCtrl, true), (Key::LeftCtrl, false)],
            "the modifier is not left down"
        );
    }

    #[test]
    fn every_mouse_button_has_its_own_key() {
        let keys = [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Back,
            MouseButton::Forward,
        ]
        .map(key_of_button);
        let distinct: std::collections::BTreeSet<_> = keys.iter().collect();
        assert_eq!(distinct.len(), keys.len());
    }
}
