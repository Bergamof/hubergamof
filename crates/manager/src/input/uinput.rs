//! The virtual input device (`/dev/uinput`).

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

use input_linux::{
    EventKind, EventTime, InputEvent, InputId, Key, KeyEvent, KeyState, SynchronizeEvent,
    UInputHandle, sys,
};

use super::keys::all_keys;
use super::{Injector, key_of_button};
use hubergamof_protocol::MouseButton;

/// The device of the system the keys go through.
const UINPUT_PATH: &str = "/dev/uinput";

/// Name the virtual device is listed under.
const DEVICE_NAME: &[u8] = b"HuBergamof virtual input";

/// A virtual keyboard and mouse buttons device, created when first opened and
/// removed when dropped.
pub struct UinputInjector {
    device: UInputHandle<File>,
}

impl UinputInjector {
    /// Creates the virtual device at `/dev/uinput`.
    ///
    /// # Errors
    /// The I/O error when the device cannot be opened (no `uinput` module, or
    /// no permission: see `docs/security.md`) or set up.
    pub fn open() -> io::Result<Self> {
        Self::open_at(Path::new(UINPUT_PATH))
    }

    /// Creates the virtual device through the file at `path`.
    ///
    /// # Errors
    /// See [`UinputInjector::open`].
    pub fn open_at(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            // A write to the device never holds up a mouse thread.
            .custom_flags(libc::O_NONBLOCK)
            .open(path)?;
        let device = UInputHandle::new(file);
        device.set_evbit(EventKind::Key)?;
        device.set_evbit(EventKind::Synchronize)?;
        for key in all_keys() {
            device.set_keybit(key)?;
        }
        for button in [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Back,
            MouseButton::Forward,
        ] {
            device.set_keybit(key_of_button(button))?;
        }
        let id = InputId {
            bustype: sys::BUS_VIRTUAL,
            vendor: 0,
            product: 0,
            version: 0,
        };
        device.create(&id, DEVICE_NAME, 0, &[])?;
        Ok(Self { device })
    }
}

impl Injector for UinputInjector {
    fn send(&mut self, key: Key, pressed: bool) -> io::Result<()> {
        let time = EventTime::default();
        let events = [
            InputEvent::from(KeyEvent::new(time, key, KeyState::pressed(pressed))).into_raw(),
            InputEvent::from(SynchronizeEvent::report(time)).into_raw(),
        ];
        self.device.write(&events).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_that_does_not_exist_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("uinput");
        assert!(UinputInjector::open_at(&missing).is_err());
    }

    #[test]
    fn a_file_that_is_not_the_uinput_device_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-uinput");
        std::fs::write(&path, b"").unwrap();
        assert!(UinputInjector::open_at(&path).is_err());
    }
}
