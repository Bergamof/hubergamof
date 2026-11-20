//! The virtual input device (`/dev/uinput`).

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

use input_linux::{
    EventKind, EventTime, InputEvent, InputId, Key, KeyEvent, KeyState, RelativeAxis,
    SynchronizeEvent, UInputHandle, sys,
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
        // Relative axes, so that the desktop takes the device for a mouse (a
        // device with buttons but no axis is a keyboard, whose buttons it
        // ignores); only used for a one pixel nudge of the pointer.
        device.set_evbit(EventKind::Relative)?;
        device.set_relbit(RelativeAxis::X)?;
        device.set_relbit(RelativeAxis::Y)?;
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

/// Linux `REL_X` step of a nudge, in device units: the smallest there is.
const NUDGE_STEP: i32 = 1;

impl UinputInjector {
    fn write_all(&self, events: &[sys::input_event]) -> io::Result<()> {
        let written = self.device.write(events)?;
        if written == events.len() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "the virtual device took only part of the events",
            ))
        }
    }

    fn move_pointer_by(&self, dx: i32) -> io::Result<()> {
        let time = EventTime::default();
        self.write_all(&[
            InputEvent::from(input_linux::RelativeEvent::new(time, RelativeAxis::X, dx)).into_raw(),
            InputEvent::from(SynchronizeEvent::report(time)).into_raw(),
        ])
    }
}

impl Injector for UinputInjector {
    fn nudge_pointer(&mut self) -> io::Result<()> {
        self.move_pointer_by(NUDGE_STEP)?;
        self.move_pointer_by(-NUDGE_STEP)
    }

    fn send(&mut self, key: Key, pressed: bool) -> io::Result<()> {
        let time = EventTime::default();
        let events = [
            InputEvent::from(KeyEvent::new(time, key, KeyState::pressed(pressed))).into_raw(),
            InputEvent::from(SynchronizeEvent::report(time)).into_raw(),
        ];
        let written = self.device.write(&events)?;
        if written == events.len() {
            Ok(())
        } else {
            // A key without its report would stay pending.
            Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "the virtual device took only part of the events",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    /// Size of a `struct input_event` on a 64-bit system: a 16 byte time, then
    /// the type, the code (2 bytes each) and the value (4 bytes).
    const EVENT_SIZE: usize = 24;

    #[test]
    fn a_device_that_does_not_exist_is_a_not_found_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("uinput");
        let error = UinputInjector::open_at(&missing).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_file_that_is_not_the_uinput_device_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-uinput");
        std::fs::write(&path, b"").unwrap();
        // The device ioctls do not exist on a plain file.
        assert!(UinputInjector::open_at(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_device_that_cannot_be_written_is_a_permission_error() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uinput");
        std::fs::write(&path, b"").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let error = UinputInjector::open_at(&path).err().unwrap();
        // Root opens anything: the case is only checked for ordinary users.
        if error.kind() != io::ErrorKind::PermissionDenied {
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0
            );
        }
    }

    /// What an injector writes to a plain file, as (type, code, value) triples.
    fn written_by(send: impl FnOnce(&mut UinputInjector)) -> Vec<(u16, u16, i32)> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events");
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .read(true)
            .open(&path)
            .unwrap();
        let mut injector = UinputInjector {
            device: UInputHandle::new(file),
        };
        send(&mut injector);
        let mut bytes = Vec::new();
        File::open(&path).unwrap().read_to_end(&mut bytes).unwrap();
        bytes
            .as_chunks::<EVENT_SIZE>()
            .0
            .iter()
            .map(|event| {
                (
                    u16::from_ne_bytes([event[16], event[17]]),
                    u16::from_ne_bytes([event[18], event[19]]),
                    i32::from_ne_bytes([event[20], event[21], event[22], event[23]]),
                )
            })
            .collect()
    }

    #[test]
    fn a_key_is_sent_as_a_key_event_followed_by_a_report() {
        let key_event = u16::try_from(sys::EV_KEY).unwrap();
        let report = u16::try_from(sys::EV_SYN).unwrap();
        let code = u16::try_from(sys::KEY_A).unwrap();
        let events = written_by(|injector| {
            injector.send(Key::A, true).unwrap();
            injector.send(Key::A, false).unwrap();
        });
        assert_eq!(
            events,
            [
                (key_event, code, 1),
                (report, 0, 0),
                (key_event, code, 0),
                (report, 0, 0),
            ]
        );
    }

    #[test]
    fn a_nudge_is_one_unit_right_then_one_left_each_with_its_report() {
        let relative = u16::try_from(sys::EV_REL).unwrap();
        let report = u16::try_from(sys::EV_SYN).unwrap();
        let axis = u16::try_from(sys::REL_X).unwrap();
        let events = written_by(|injector| injector.nudge_pointer().unwrap());
        assert_eq!(
            events,
            [
                (relative, axis, 1),
                (report, 0, 0),
                (relative, axis, -1),
                (report, 0, 0),
            ]
        );
    }
}
