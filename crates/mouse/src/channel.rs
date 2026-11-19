//! Byte-level channel to a HID++ interface and the request/answer exchange on
//! top of it.

use std::time::{Duration, Instant};

use thiserror::Error;

use crate::frame::{Answer, DeviceError, Request};

/// Frames kept in a trace; the rest is counted, not stored.
pub const MAX_TRACE_LINES: usize = 10_000;

/// Time a device is given to answer before a request counts as unanswered.
pub const ANSWER_TIMEOUT: Duration = Duration::from_millis(500);

/// A HID++ interface: writes reports and reads the ones the device sends.
pub trait Channel {
    /// Writes one report (report id first).
    ///
    /// # Errors
    /// [`ChannelError`] when the interface refuses the write.
    fn write(&mut self, report: &[u8]) -> Result<(), ChannelError>;

    /// Next report from the device, `None` when none arrives within `timeout`.
    ///
    /// # Errors
    /// [`ChannelError`] when the interface cannot be read.
    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, ChannelError>;
}

/// The interface could not be written or read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct ChannelError(pub String);

/// Why a request got no usable answer.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HidppError {
    /// Nothing matching arrived in time.
    #[error("no answer within {0:?}")]
    Timeout(Duration),
    /// The device (or its receiver) answered with an error.
    #[error(transparent)]
    Device(#[from] DeviceError),
    /// The interface failed.
    #[error("interface error: {0}")]
    Channel(#[from] ChannelError),
    /// The device answered something the protocol does not allow.
    #[error("malformed answer: {0}")]
    Malformed(String),
}

/// Direction of a traced frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Written to the device.
    Sent,
    /// Read from the device.
    Received,
}

/// Requests to devices behind one [`Channel`].
#[derive(Debug)]
pub struct Hidpp<C> {
    channel: C,
    timeout: Duration,
    trace: Option<Vec<String>>,
}

impl<C: Channel> Hidpp<C> {
    /// Wraps a channel with the default answer timeout and no trace.
    #[must_use]
    pub fn new(channel: C) -> Self {
        Self {
            channel,
            timeout: ANSWER_TIMEOUT,
            trace: None,
        }
    }

    /// Sets the time given to a device to answer.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Records every frame sent and received, see [`Hidpp::take_trace`].
    #[must_use]
    pub fn with_trace(mut self) -> Self {
        self.trace = Some(Vec::new());
        self
    }

    /// The underlying channel.
    #[must_use]
    pub fn channel(&self) -> &C {
        &self.channel
    }

    /// The recorded frames, one line each, oldest first (empty without trace).
    pub fn take_trace(&mut self) -> Vec<String> {
        self.trace.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// Sends a request and waits for its answer; frames that are not the
    /// answer (notifications, answers to other requests) are skipped.
    ///
    /// # Errors
    /// [`HidppError`] when the device does not answer, refuses, or the
    /// interface fails.
    pub fn request(
        &mut self,
        device: u8,
        feature_index: u8,
        function: u8,
        params: &[u8],
    ) -> Result<Vec<u8>, HidppError> {
        let request = Request::new(device, feature_index, function, params);
        let report = request.encode();
        self.record(Direction::Sent, &report);
        self.channel.write(&report)?;
        let deadline = Instant::now() + self.timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(HidppError::Timeout(self.timeout));
            }
            let Some(bytes) = self.channel.read(remaining)? else {
                return Err(HidppError::Timeout(self.timeout));
            };
            self.record(Direction::Received, &bytes);
            match request.answered_by(&bytes) {
                Some(Answer::Params(params)) => return Ok(params),
                Some(Answer::Failed(error)) => return Err(error.into()),
                None => {}
            }
        }
    }

    fn record(&mut self, direction: Direction, bytes: &[u8]) {
        let Some(trace) = &mut self.trace else {
            return;
        };
        match trace.len().cmp(&MAX_TRACE_LINES) {
            std::cmp::Ordering::Less => {
                let arrow = match direction {
                    Direction::Sent => "->",
                    Direction::Received => "<-",
                };
                let hex: Vec<String> = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
                trace.push(format!("{arrow} {}", hex.join(" ")));
            }
            std::cmp::Ordering::Equal => trace.push("... trace truncated".to_owned()),
            std::cmp::Ordering::Greater => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeDevice, FakeLink, ScriptedChannel, frame};

    #[test]
    fn answers_are_matched_and_unrelated_frames_skipped() {
        let link = FakeLink::direct(FakeDevice::new().with_feature(0x0005, 0))
            .with_noise(vec![0x11, 0xFF, 0x03, 0x00, 1, 2, 3]);
        let mut hidpp = Hidpp::new(link).with_trace();
        let params = hidpp.request(0xFF, 0, 0, &[0x00, 0x05]).unwrap();
        assert_eq!(params[0], 2, "FeatureSet takes index 1");
        let trace = hidpp.take_trace();
        assert_eq!(trace.len(), 3, "request, noise, answer: {trace:?}");
        assert!(trace[0].starts_with("-> 11 ff 00 0b"));
        assert_eq!(hidpp.take_trace().len(), 0);
    }

    #[test]
    fn several_unrelated_frames_before_the_answer_are_skipped() {
        let request = Request::new(0xFF, 4, 1, &[]);
        let answer = frame(0xFF, 4, request.function_byte(), &[42]);
        let channel = ScriptedChannel::new()
            // Same feature, another software id (a notification).
            .then_frame(frame(0xFF, 4, 0x1A, &[1]))
            // Another device.
            .then_frame(frame(0x02, 4, request.function_byte(), &[2]))
            // Late answer to another request.
            .then_frame(frame(0xFF, 5, 0x0B, &[3]))
            .then_frame(answer);
        let params = Hidpp::new(channel).request(0xFF, 4, 1, &[]).unwrap();
        assert_eq!(params[0], 42);
    }

    #[test]
    fn a_receiver_without_that_device_answers_with_an_error() {
        let mut hidpp = Hidpp::new(FakeLink::receiver(Vec::new()));
        let error = hidpp.request(2, 0, 1, &[0, 0, 0x5A]).unwrap_err();
        assert!(matches!(error, HidppError::Device(DeviceError::Hidpp10(8))));
    }

    #[test]
    fn a_silent_device_times_out() {
        let mut silent = Hidpp::new(FakeLink::silent()).with_timeout(Duration::from_millis(5));
        assert!(matches!(
            silent.request(2, 0, 1, &[]).unwrap_err(),
            HidppError::Timeout(_)
        ));
    }

    #[test]
    fn a_flood_of_unrelated_frames_ends_at_the_deadline() {
        let channel = ScriptedChannel::new().then_flood(frame(0xFF, 9, 0x0B, &[]));
        let mut hidpp = Hidpp::new(channel).with_timeout(Duration::from_millis(5));
        assert!(matches!(
            hidpp.request(0xFF, 4, 1, &[]).unwrap_err(),
            HidppError::Timeout(_)
        ));
    }

    #[test]
    fn a_read_error_in_the_middle_is_reported() {
        let channel = ScriptedChannel::new()
            .then_frame(frame(0xFF, 9, 0x0B, &[]))
            .then_error("unplugged");
        let error = Hidpp::new(channel).request(0xFF, 4, 1, &[]).unwrap_err();
        assert_eq!(
            error,
            HidppError::Channel(ChannelError("unplugged".to_owned()))
        );
    }

    #[test]
    fn a_refused_write_is_reported() {
        let mut hidpp = Hidpp::new(FakeLink::failing());
        let error = hidpp.request(1, 0, 1, &[]).unwrap_err();
        assert!(matches!(error, HidppError::Channel(_)));
        assert!(error.to_string().contains("interface error"));
    }

    #[test]
    fn the_trace_is_capped() {
        let mut hidpp = Hidpp::new(FakeLink::direct(FakeDevice::new())).with_trace();
        for _ in 0..MAX_TRACE_LINES {
            hidpp.request(0xFF, 0, 1, &[0, 0, 0x5A]).unwrap();
        }
        let trace = hidpp.take_trace();
        assert_eq!(trace.len(), MAX_TRACE_LINES + 1);
        assert_eq!(
            trace.last().map(String::as_str),
            Some("... trace truncated")
        );
    }

    #[test]
    fn the_channel_and_its_writes_can_be_inspected() {
        let mut hidpp = Hidpp::new(FakeLink::direct(FakeDevice::new()));
        hidpp.request(0xFF, 0, 1, &[0, 0, 0x5A]).unwrap();
        assert_eq!(hidpp.channel().written().len(), 1);
    }
}
