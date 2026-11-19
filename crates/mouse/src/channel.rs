//! Byte-level channel to a HID++ interface and the request/answer exchange on
//! top of it.

use std::time::{Duration, Instant};

use thiserror::Error;

use crate::frame::{Answer, DeviceError, Request};

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
        if let Some(trace) = &mut self.trace {
            let arrow = match direction {
                Direction::Sent => "->",
                Direction::Received => "<-",
            };
            let hex: Vec<String> = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            trace.push(format!("{arrow} {}", hex.join(" ")));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::testing::{FakeDevice, FakeLink};

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
    fn a_silent_device_times_out() {
        let link = FakeLink::receiver(Vec::new());
        let mut hidpp = Hidpp::new(link).with_timeout(Duration::from_millis(1));
        // The receiver itself answers for an unknown index.
        let error = hidpp.request(2, 0, 1, &[0, 0, 0x5A]).unwrap_err();
        assert!(matches!(error, HidppError::Device(DeviceError::Hidpp10(8))));
        let mut silent = Hidpp::new(FakeLink::silent()).with_timeout(Duration::from_millis(1));
        assert!(matches!(
            silent.request(2, 0, 1, &[]).unwrap_err(),
            HidppError::Timeout(_)
        ));
    }

    #[test]
    fn a_refused_write_is_reported() {
        let mut hidpp = Hidpp::new(FakeLink::failing());
        let error = hidpp.request(1, 0, 1, &[]).unwrap_err();
        assert!(matches!(error, HidppError::Channel(_)));
        assert!(error.to_string().contains("interface error"));
    }
}
