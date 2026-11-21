//! Finding a device behind a device index: a ping, and the name it gives.
//!
//! Only getters are sent: `IRoot` ping, and the device name. Nothing is set,
//! no button is diverted, no effect is triggered.

use crate::channel::{Channel, Hidpp, HidppError};
use crate::frame::DeviceError;

/// Byte a ping asks the device to echo.
const PING_ECHO: u8 = 0x5A;
/// Pings sent to an index that stays silent: a sleeping mouse needs a moment
/// (and a movement) to wake up, so each silent index is retried for a few seconds.
pub const PING_ATTEMPTS: u32 = 8;
/// Characters the device name getter returns per call.
const NAME_CHUNK: usize = 16;

/// Why nothing was found at a device index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Miss {
    /// No frame came back, in any of the attempts.
    Silent {
        /// Number of pings sent.
        attempts: u32,
    },
    /// The device or its receiver answered with an error (a receiver says so
    /// when no device is paired at that index).
    Refused(DeviceError),
    /// The answer did not carry the echo byte of the ping.
    WrongEcho,
    /// The interface itself failed.
    Interface(String),
}

impl Miss {
    /// One line a person can act on.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Silent { attempts } => format!(
                "no answer after {attempts} pings (device asleep, off, or set to another channel)"
            ),
            Self::Refused(error) => format!("refused: {error}"),
            Self::WrongEcho => "answered, but not to our ping".to_owned(),
            Self::Interface(message) => format!("interface error: {message}"),
        }
    }
}

/// Pings the device at `device`, retrying while it stays silent.
pub(crate) fn ping<C: Channel>(hidpp: &mut Hidpp<C>, device: u8) -> Result<(u8, u8), Miss> {
    ping_attempts(hidpp, device, PING_ATTEMPTS)
}

/// Pings the device at `device`, retrying at most `attempts` times while it
/// stays silent.
pub(crate) fn ping_attempts<C: Channel>(
    hidpp: &mut Hidpp<C>,
    device: u8,
    attempts: u32,
) -> Result<(u8, u8), Miss> {
    for _ in 0..attempts {
        match hidpp.request(device, 0, 1, &[0, 0, PING_ECHO]) {
            Ok(params) => {
                return match params[..] {
                    [major, minor, PING_ECHO, ..] => Ok((major, minor)),
                    _ => Err(Miss::WrongEcho),
                };
            }
            Err(HidppError::Timeout(_)) => {}
            Err(HidppError::Device(error)) => return Err(Miss::Refused(error)),
            Err(other) => return Err(Miss::Interface(other.to_string())),
        }
    }
    Err(Miss::Silent { attempts })
}

pub(crate) fn read_name<C: Channel>(
    hidpp: &mut Hidpp<C>,
    device: u8,
    index: u8,
) -> Result<String, HidppError> {
    let length = usize::from(
        hidpp
            .request(device, index, 0, &[])?
            .first()
            .copied()
            .unwrap_or(0),
    );
    let mut name = Vec::with_capacity(length);
    while name.len() < length {
        let offset = u8::try_from(name.len()).unwrap_or(u8::MAX);
        let chunk = hidpp.request(device, index, 1, &[offset])?;
        if chunk.is_empty() {
            return Err(HidppError::Malformed("empty name chunk".to_owned()));
        }
        let wanted = (length - name.len()).min(NAME_CHUNK);
        name.extend(chunk.iter().take(wanted));
    }
    Ok(String::from_utf8_lossy(&name)
        .trim_end_matches('\0')
        .to_owned())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::features::DEVICE_NAME;
    use crate::testing::{FakeDevice, FakeLink, ScriptedChannel, frame};

    /// Function bytes a scripted reply must carry for `function` (software id included).
    fn function_byte(function: u8) -> u8 {
        function << 4 | crate::frame::SOFTWARE_ID
    }

    #[test]
    fn a_ping_with_the_wrong_echo_is_not_a_device() {
        // Echo byte 0x00 instead of the 0x5A we sent.
        let reply = frame(0xFF, 0, function_byte(1), &[4, 5, 0x00]);
        let mut hidpp = Hidpp::new(ScriptedChannel::new().then_frame(reply));
        assert_eq!(ping(&mut hidpp, 0xFF), Err(Miss::WrongEcho));
    }

    #[test]
    fn a_ping_refused_with_an_hidpp20_error_is_not_a_device() {
        let reply = frame(0xFF, 0xFF, 0, &[function_byte(1), 2]);
        let mut hidpp = Hidpp::new(ScriptedChannel::new().then_frame(reply));
        assert_eq!(
            ping(&mut hidpp, 0xFF),
            Err(Miss::Refused(DeviceError::Hidpp20(2)))
        );
    }

    #[test]
    fn a_silent_index_is_retried_then_explained() {
        let mut hidpp = Hidpp::new(FakeLink::silent());
        assert_eq!(
            ping(&mut hidpp, 1),
            Err(Miss::Silent {
                attempts: PING_ATTEMPTS
            })
        );
        assert_eq!(hidpp.channel().written().len(), PING_ATTEMPTS as usize);
    }

    #[test]
    fn a_sleeping_device_that_wakes_up_is_found() {
        // Two silences, then the answer to the third ping.
        let reply = frame(0xFF, 0, function_byte(1), &[4, 5, PING_ECHO]);
        let mut hidpp = Hidpp::new(
            ScriptedChannel::new()
                .then_silence()
                .then_silence()
                .then_frame(reply),
        );
        assert_eq!(ping(&mut hidpp, 0xFF), Ok((4, 5)));
    }

    // Name reading, in 16-character chunks.

    fn name_of(len: usize) -> Option<String> {
        let name = "abcdefghij".repeat(26)[..len].to_owned();
        let device = FakeDevice::new().with_name(&name);
        let index = device.index_of(DEVICE_NAME).unwrap();
        let mut hidpp = Hidpp::new(FakeLink::direct(device));
        let read = read_name(&mut hidpp, 0xFF, index)
            .unwrap_or_else(|error| panic!("length {len}: {error}"));
        Some(read).filter(|name| !name.is_empty())
    }

    #[test]
    fn names_of_every_awkward_length_are_read_whole() {
        for len in [1, 15, 16, 17, 31, 32, 33, 255] {
            let expected = "abcdefghij".repeat(26)[..len].to_owned();
            assert_eq!(name_of(len), Some(expected), "length {len}");
        }
    }

    #[test]
    fn an_empty_name_is_no_name() {
        assert_eq!(name_of(0), None);
    }

    /// Replies for a name of `length` characters answered by `chunks`.
    fn name_script(length: u8, chunks: &[&[u8]]) -> Hidpp<ScriptedChannel> {
        let mut channel =
            ScriptedChannel::new().then_frame(frame(0xFF, 5, function_byte(0), &[length]));
        for chunk in chunks {
            channel = channel.then_frame(frame(0xFF, 5, function_byte(1), chunk));
        }
        Hidpp::new(channel)
    }

    #[test]
    fn trailing_nuls_are_dropped_and_bad_utf8_is_replaced() {
        let mut hidpp = name_script(4, &[b"ab\0\0"]);
        assert_eq!(read_name(&mut hidpp, 0xFF, 5).unwrap(), "ab");
        let mut hidpp = name_script(2, &[&[0xFF, b'a']]);
        assert_eq!(read_name(&mut hidpp, 0xFF, 5).unwrap(), "\u{FFFD}a");
    }

    #[test]
    fn a_name_longer_than_what_the_device_returns_is_an_error_not_a_loop() {
        // 20 characters announced, a 4-byte first chunk (the reply is padded to
        // the report size, so it is long), then silence.
        let mut hidpp = name_script(40, &[]).with_timeout(Duration::from_millis(5));
        assert!(matches!(
            read_name(&mut hidpp, 0xFF, 5),
            Err(HidppError::Timeout(_))
        ));
    }

    #[test]
    fn an_answer_without_parameters_cannot_stall_the_name_loop() {
        let bare = vec![0x11, 0xFF, 5, function_byte(1)];
        let channel = ScriptedChannel::new()
            .then_frame(frame(0xFF, 5, function_byte(0), &[20]))
            .then_frame(bare);
        let error = read_name(&mut Hidpp::new(channel), 0xFF, 5).unwrap_err();
        assert_eq!(error, HidppError::Malformed("empty name chunk".to_owned()));
    }
}
