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

/// A frame a device sends by itself (a button pressed, a battery change…):
/// its function byte carries no software id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// Device index.
    pub device: u8,
    /// Index of the feature that sent it.
    pub feature_index: u8,
    /// Event number within the feature.
    pub event: u8,
    /// Parameters.
    pub params: Vec<u8>,
}

impl Notification {
    /// Reads a notification from a frame; `None` for anything else (an answer
    /// carries our software id, an error frame has a reserved feature index).
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let [report_id, device, feature_index, function_byte, params @ ..] = bytes else {
            return None;
        };
        let long_enough_report = matches!(*report_id, 0x10..=0x12);
        let carries_no_software_id = function_byte.is_multiple_of(16);
        let reserved = matches!(*feature_index, ERROR_20 | ERROR_10);
        (long_enough_report && carries_no_software_id && !reserved).then(|| Self {
            device: *device,
            feature_index: *feature_index,
            event: function_byte >> 4,
            params: params.to_vec(),
        })
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
    use super::*;
    use crate::testing::frame as answer_frame;

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

    #[test]
    fn recognises_its_answer_and_nothing_else() {
        let request = Request::new(1, 4, 2, &[]);
        let frame = answer_frame(1, 4, request.function_byte(), &[9, 8]);
        let Some(Answer::Params(params)) = request.answered_by(&frame) else {
            panic!("answer expected");
        };
        assert_eq!(&params[..2], &[9, 8]);
        let other_device = answer_frame(2, 4, request.function_byte(), &[]);
        let other_feature = answer_frame(1, 5, request.function_byte(), &[]);
        let other_function = answer_frame(1, 4, 0x3B, &[]);
        let other_software_id = answer_frame(1, 4, 0x2A, &[]);
        for frame in [
            other_device,
            other_feature,
            other_function,
            other_software_id,
        ] {
            assert_eq!(request.answered_by(&frame), None, "{frame:?}");
        }
        assert_eq!(request.answered_by(&[0x11, 1]), None);
        assert_eq!(request.answered_by(&[]), None);
    }

    #[test]
    fn a_bare_header_is_an_answer_without_parameters() {
        let request = Request::new(1, 4, 2, &[]);
        let bare = [LONG_REPORT_ID, 1, 4, request.function_byte()];
        assert_eq!(request.answered_by(&bare), Some(Answer::Params(Vec::new())));
    }

    #[test]
    fn the_report_id_is_not_checked() {
        // Receivers answer on short, long or very long reports alike.
        let request = Request::new(1, 4, 2, &[]);
        let mut short = answer_frame(1, 4, request.function_byte(), &[1]);
        short[0] = 0x10;
        assert!(matches!(
            request.answered_by(&short),
            Some(Answer::Params(_))
        ));
    }

    #[test]
    fn recognises_an_hidpp20_error() {
        let request = Request::new(0xFF, 6, 1, &[]);
        // [report, device, 0xFF, feature index, function byte, code]
        let frame = answer_frame(0xFF, 0xFF, 6, &[request.function_byte(), 7]);
        assert_eq!(
            request.answered_by(&frame),
            Some(Answer::Failed(DeviceError::Hidpp20(7)))
        );
        let other_feature = answer_frame(0xFF, 0xFF, 7, &[request.function_byte(), 7]);
        let other_function = answer_frame(0xFF, 0xFF, 6, &[0x2B, 7]);
        assert_eq!(request.answered_by(&other_feature), None);
        assert_eq!(request.answered_by(&other_function), None);
    }

    #[test]
    fn recognises_an_hidpp10_error_from_a_receiver() {
        let request = Request::new(3, 0, 1, &[]);
        // [report, device, 0x8F, sub id (our feature), address (our function), code]
        let padded = answer_frame(3, 0x8F, 0, &[request.function_byte(), 8]);
        assert_eq!(
            request.answered_by(&padded),
            Some(Answer::Failed(DeviceError::Hidpp10(8)))
        );
        // The short report a receiver really sends, unpadded.
        let short = [0x10, 3, 0x8F, 0, request.function_byte(), 8, 0];
        assert_eq!(
            request.answered_by(&short),
            Some(Answer::Failed(DeviceError::Hidpp10(8)))
        );
        let other_feature = answer_frame(3, 0x8F, 9, &[request.function_byte(), 8]);
        let other_function = answer_frame(3, 0x8F, 0, &[0x2B, 8]);
        assert_eq!(request.answered_by(&other_feature), None);
        assert_eq!(request.answered_by(&other_function), None);
    }

    #[test]
    fn truncated_error_frames_are_not_taken_for_answers_to_other_requests() {
        let request = Request::new(3, 6, 1, &[]);
        // An error header with no function byte nor code: not ours.
        assert_eq!(request.answered_by(&[0x11, 3, 0xFF, 6]), None);
        assert_eq!(request.answered_by(&[0x11, 3, 0x8F, 6]), None);
    }

    #[test]
    fn notifications_carry_no_software_id() {
        let frame = answer_frame(1, 7, 0x10, &[0, 0x50, 0, 0x52]);
        let notification = Notification::parse(&frame).unwrap();
        assert_eq!(
            (
                notification.device,
                notification.feature_index,
                notification.event
            ),
            (1, 7, 1)
        );
        assert_eq!(&notification.params[..4], &[0, 0x50, 0, 0x52]);
        let short = [0x10, 2, 5, 0x00, 9, 9, 0];
        assert_eq!(Notification::parse(&short).unwrap().params, [9, 9, 0]);
    }

    #[test]
    fn answers_errors_and_odd_frames_are_not_notifications() {
        let answer = answer_frame(1, 7, 0x1B, &[1]);
        assert_eq!(Notification::parse(&answer), None, "software id present");
        assert_eq!(
            Notification::parse(&answer_frame(1, 0xFF, 0x00, &[1])),
            None,
            "hidpp 2.0 error"
        );
        assert_eq!(
            Notification::parse(&answer_frame(1, 0x8F, 0x00, &[1])),
            None,
            "hidpp 1.0 error"
        );
        let mut other_report = answer_frame(1, 7, 0x00, &[1]);
        other_report[0] = 0x20;
        assert_eq!(Notification::parse(&other_report), None);
        assert_eq!(Notification::parse(&[0x11, 1, 7]), None);
        assert_eq!(Notification::parse(&[]), None);
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
