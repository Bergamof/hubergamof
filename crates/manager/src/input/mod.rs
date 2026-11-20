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

    /// Moves the pointer one pixel and back, so that the desktop reports where
    /// it is to the window under it (a compositor that gives a client the
    /// position of the pointer only when it moves). Does nothing by default.
    ///
    /// # Errors
    /// See [`Injector::send`].
    fn nudge_pointer(&mut self) -> io::Result<()> {
        Ok(())
    }

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
    /// The key whose releasing fails, when set.
    pub failing_release: Option<Key>,
}

#[cfg(test)]
impl Injector for RecordingInjector {
    fn send(&mut self, key: Key, pressed: bool) -> io::Result<()> {
        if pressed && self.failing == Some(key) {
            return Err(io::Error::other("refused"));
        }
        if !pressed && self.failing_release == Some(key) {
            self.events.push((key, false));
            return Err(io::Error::other("release refused"));
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
    fn every_mouse_button_is_its_own_key() {
        assert_eq!(key_of_button(MouseButton::Left), Key::ButtonLeft);
        assert_eq!(key_of_button(MouseButton::Right), Key::ButtonRight);
        assert_eq!(key_of_button(MouseButton::Middle), Key::ButtonMiddle);
        assert_eq!(key_of_button(MouseButton::Back), Key::ButtonSide);
        assert_eq!(key_of_button(MouseButton::Forward), Key::ButtonExtra);
    }

    #[test]
    fn a_tap_that_cannot_press_does_not_release() {
        let mut injector = RecordingInjector {
            failing: Some(Key::ButtonLeft),
            ..RecordingInjector::default()
        };
        assert!(injector.tap(Key::ButtonLeft).is_err());
        assert_eq!(injector.events, []);
    }

    #[test]
    fn a_shortcut_that_fails_on_its_first_key_sends_nothing() {
        let mut injector = RecordingInjector {
            failing: Some(Key::LeftCtrl),
            ..RecordingInjector::default()
        };
        assert!(injector.tap_shortcut(&[Key::LeftCtrl, Key::M]).is_err());
        assert_eq!(injector.events, []);
    }

    #[test]
    fn a_shortcut_that_fails_to_release_still_tries_every_key_and_keeps_the_first_error() {
        let mut injector = RecordingInjector {
            failing_release: Some(Key::M),
            ..RecordingInjector::default()
        };
        let error = injector.tap_shortcut(&[Key::LeftCtrl, Key::M]).unwrap_err();
        assert_eq!(error.to_string(), "release refused");
        assert_eq!(
            injector.events,
            [
                (Key::LeftCtrl, true),
                (Key::M, true),
                (Key::M, false),
                (Key::LeftCtrl, false),
            ],
            "the modifier is released although the key was not"
        );
    }

    #[test]
    fn an_empty_shortcut_sends_nothing() {
        let mut injector = RecordingInjector::default();
        injector.tap_shortcut(&[]).unwrap();
        assert_eq!(injector.events, []);
    }
}
