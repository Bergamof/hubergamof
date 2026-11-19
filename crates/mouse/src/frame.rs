//! HID++ long frames: requests built by this crate and answers decoded from
//! the device.
//!
//! Layout of a long report (20 bytes): report id `0x11`, device index, feature
//! index, `function << 4 | software id`, then 16 parameter bytes.

use thiserror::Error;

/// Report id of a HID++ long report.
pub const LONG_REPORT_ID: u8 = 0x11;
/// Size in bytes of a HID++ long report, report id included.
pub const LONG_REPORT_LEN: usize = 20;
/// Number of parameter bytes of a long report.
pub const PARAMS_LEN: usize = LONG_REPORT_LEN - 4;
/// Software id put in the low nibble of the function byte. A device echoes it
/// in its answer, which tells our answers from notifications; it must not be 0.
pub const SOFTWARE_ID: u8 = 0x0B;
/// Feature index of an HID++ 2.0 error answer.
const ERROR_20: u8 = 0xFF;
/// Sub id of an HID++ 1.0 error answer (receivers).
const ERROR_10: u8 = 0x8F;

/// Device index of a device reached directly (Bluetooth, USB cable).
pub const DIRECT_DEVICE_INDEX: u8 = 0xFF;
/// Device indexes a receiver gives to its paired devices (Logi Bolt: 1 to 6).
pub const RECEIVER_DEVICE_INDEXES: std::ops::RangeInclusive<u8> = 1..=6;

/// A request to a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    device: u8,
    feature_index: u8,
    function: u8,
    params: Vec<u8>,
}

impl Request {
    /// Builds a request. `function` is the function number (0 to 15) of the
    /// feature at `feature_index`; at most [`PARAMS_LEN`] parameters are kept.
    #[must_use]
    pub fn new(device: u8, feature_index: u8, function: u8, params: &[u8]) -> Self {
        Self {
            device,
            feature_index,
            function: function & 0x0F,
            params: params.iter().copied().take(PARAMS_LEN).collect(),
        }
    }

    /// The `function << 4 | software id` byte.
    #[must_use]
    pub fn function_byte(&self) -> u8 {
        self.function << 4 | SOFTWARE_ID
    }

    /// The report to write to the device.
    #[must_use]
    pub fn encode(&self) -> [u8; LONG_REPORT_LEN] {
        let mut report = [0; LONG_REPORT_LEN];
        report[0] = LONG_REPORT_ID;
        report[1] = self.device;
        report[2] = self.feature_index;
        report[3] = self.function_byte();
        report[4..4 + self.params.len()].copy_from_slice(&self.params);
        report
    }

    /// Whether `bytes` is the device's answer to this request: the answer
    /// itself, or an error frame naming this request.
    #[must_use]
    pub fn answered_by(&self, bytes: &[u8]) -> Option<Answer> {
        let [_, device, third, fourth, rest @ ..] = bytes else {
            return None;
        };
        if *device != self.device {
            return None;
        }
        let params = || rest.to_vec();
        match (*third, rest) {
            (ERROR_20, [function, code, ..])
                if *fourth == self.feature_index && *function == self.function_byte() =>
            {
                Some(Answer::Failed(DeviceError::Hidpp20(*code)))
            }
            (ERROR_10, [function, code, ..])
                if *fourth == self.feature_index && *function == self.function_byte() =>
            {
                Some(Answer::Failed(DeviceError::Hidpp10(*code)))
            }
            (feature, _) if feature == self.feature_index && *fourth == self.function_byte() => {
                Some(Answer::Params(params()))
            }
            _ => None,
        }
    }
}

/// What a device answered to a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Parameters of the answer.
    Params(Vec<u8>),
    /// The device refused the request.
    Failed(DeviceError),
}

/// Error code a device (or the receiver in front of it) answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DeviceError {
    /// HID++ 2.0 error.
    #[error("HID++ 2.0 error 0x{:02X} ({})", .0, name_20(*.0))]
    Hidpp20(u8),
    /// HID++ 1.0 error, from a receiver (typically: no device at this index).
    #[error("HID++ 1.0 error 0x{:02X} ({})", .0, name_10(*.0))]
    Hidpp10(u8),
}

fn name_20(code: u8) -> &'static str {
    match code {
        1 => "unknown",
        2 => "invalid argument",
        3 => "out of range",
        4 => "hardware error",
        5 => "internal error",
        6 => "invalid feature index",
        7 => "invalid function",
        8 => "busy",
        9 => "unsupported",
        _ => "unlisted",
    }
}

fn name_10(code: u8) -> &'static str {
    match code {
        1 => "invalid sub id",
        2 => "invalid address",
        3 => "invalid value",
        4 => "connection failed",
        5 => "too many devices",
        6 => "already exists",
        7 => "busy",
        8 => "unknown device",
        9 => "resource error",
        10 => "request unavailable",
        11 => "invalid parameter value",
        12 => "wrong PIN code",
        _ => "unlisted",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn encodes_a_long_report() {
        let request = Request::new(0x02, 0x05, 0x01, &[0xAA, 0xBB]);
        let bytes = request.encode();
        assert_eq!(bytes.len(), LONG_REPORT_LEN);
        assert_eq!(&bytes[..6], &[0x11, 0x02, 0x05, 0x1B, 0xAA, 0xBB]);
        assert!(bytes[6..].iter().all(|b| *b == 0));
    }

    #[test]
    fn function_is_masked_to_a_nibble_and_params_are_capped() {
        let request = Request::new(1, 0, 0x1F, &[7; 40]);
        assert_eq!(request.function_byte(), 0xFB);
        assert_eq!(request.encode()[4..], [7; PARAMS_LEN]);
    }

    fn answer_frame(device: u8, feature: u8, function_byte: u8, params: &[u8]) -> Vec<u8> {
        let mut bytes = vec![LONG_REPORT_ID, device, feature, function_byte];
        bytes.extend_from_slice(params);
        bytes.resize(LONG_REPORT_LEN, 0);
        bytes
    }

    #[test]
    fn recognises_its_answer_and_nothing_else() {
        let request = Request::new(1, 4, 2, &[]);
        let frame = answer_frame(1, 4, request.function_byte(), &[9, 8]);
        let Some(Answer::Params(params)) = request.answered_by(&frame) else {
            panic!("answer expected");
        };
        assert_eq!(&params[..2], &[9, 8]);
        // Another device, another feature, another function, another software id.
        assert_eq!(
            request.answered_by(&answer_frame(2, 4, request.function_byte(), &[])),
            None
        );
        assert_eq!(
            request.answered_by(&answer_frame(1, 5, request.function_byte(), &[])),
            None
        );
        assert_eq!(request.answered_by(&answer_frame(1, 4, 0x3B, &[])), None);
        assert_eq!(request.answered_by(&answer_frame(1, 4, 0x2A, &[])), None);
        assert_eq!(request.answered_by(&[0x11, 1]), None);
    }

    #[test]
    fn recognises_an_hidpp20_error() {
        let request = Request::new(0xFF, 6, 1, &[]);
        let frame = answer_frame(0xFF, 0xFF, 6, &[request.function_byte(), 7]);
        // [report, device, 0xFF, feature index, function byte, code]
        assert_eq!(
            request.answered_by(&frame),
            Some(Answer::Failed(DeviceError::Hidpp20(7)))
        );
        let other = answer_frame(0xFF, 0xFF, 7, &[request.function_byte(), 7]);
        assert_eq!(request.answered_by(&other), None);
    }

    #[test]
    fn recognises_an_hidpp10_error_from_a_receiver() {
        let request = Request::new(3, 0, 1, &[]);
        let mut frame = vec![0x10, 3, 0x8F, 0, request.function_byte(), 8, 0];
        frame.resize(LONG_REPORT_LEN, 0);
        assert_eq!(
            request.answered_by(&frame),
            Some(Answer::Failed(DeviceError::Hidpp10(8)))
        );
    }

    #[test]
    fn errors_are_described() {
        assert_eq!(
            DeviceError::Hidpp20(9).to_string(),
            "HID++ 2.0 error 0x09 (unsupported)"
        );
        assert_eq!(
            DeviceError::Hidpp10(8).to_string(),
            "HID++ 1.0 error 0x08 (unknown device)"
        );
        assert!(DeviceError::Hidpp20(0x77).to_string().contains("unlisted"));
    }
}
