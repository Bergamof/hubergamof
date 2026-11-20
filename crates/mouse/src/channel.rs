//! Byte-level channel to a HID++ interface and the request/answer exchange on
//! top of it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::frame::{Answer, DeviceError, Notification, Request};

/// Frames kept in a trace; the rest is counted, not stored.
pub const MAX_TRACE_LINES: usize = 10_000;

/// Unrelated frames (notifications) kept while a request waits for its answer;
/// the oldest are dropped past this.
pub const MAX_PENDING_FRAMES: usize = 256;

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
    /// The device answered, but it is not one of the mice this project drives.
    #[error("`{0}` is not a supported mouse")]
    NotSupportedMouse(String),
    /// The interface failed.
    #[error("interface error: {0}")]
    Channel(#[from] ChannelError),
    /// The device answered something the protocol does not allow.
    #[error("malformed answer: {0}")]
    Malformed(String),
    /// The device does not have the feature the operation needs.
    #[error("the device has no {0}")]
    Unsupported(&'static str),
    /// The operation was asked something the device must not be sent.
    #[error("refused: {0}")]
    InvalidArgument(String),
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
    pending: VecDeque<Vec<u8>>,
    dropped: u64,
}

impl<C: Channel> Hidpp<C> {
    /// Wraps a channel with the default answer timeout and no trace.
    #[must_use]
    pub fn new(channel: C) -> Self {
        Self {
            channel,
            timeout: ANSWER_TIMEOUT,
            trace: None,
            pending: VecDeque::new(),
            dropped: 0,
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

    /// The underlying channel, for tests to feed it.
    #[cfg(test)]
    pub(crate) fn channel_mut(&mut self) -> &mut C {
        &mut self.channel
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
                None => self.keep(bytes),
            }
        }
    }

    /// The next frame that is not the answer to a request: first the ones
    /// set aside while a request waited, then whatever the device sends within
    /// `timeout`. `None` when nothing comes.
    ///
    /// # Errors
    /// [`HidppError::Channel`] when the interface fails.
    pub fn next_frame(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, HidppError> {
        if let Some(frame) = self.pending.pop_front() {
            return Ok(Some(frame));
        }
        let frame = self.channel.read(timeout)?;
        if let Some(bytes) = &frame {
            self.record(Direction::Received, bytes);
        }
        Ok(frame)
    }

    /// Sets a frame aside when it is a notification: a late answer to a request
    /// that gave up is of no use to anyone.
    fn keep(&mut self, frame: Vec<u8>) {
        if Notification::parse(&frame).is_none() {
            return;
        }
        if self.pending.len() == MAX_PENDING_FRAMES {
            self.pending.pop_front();
            self.dropped += 1;
        }
        self.pending.push_back(frame);
    }

    /// How many notifications were thrown away for lack of room. When it
    /// grows, a state built from notifications (which buttons are held) can no
    /// longer be trusted and must be started again.
    #[must_use]
    pub fn dropped_frames(&self) -> u64 {
        self.dropped
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
    fn frames_that_are_not_the_answer_are_kept_for_later() {
        let request = Request::new(0xFF, 4, 1, &[]);
        let notification = frame(0xFF, 4, 0x00, &[7]);
        let channel = ScriptedChannel::new()
            .then_frame(notification.clone())
            .then_frame(frame(0xFF, 4, request.function_byte(), &[42]));
        let mut hidpp = Hidpp::new(channel).with_timeout(Duration::from_millis(5));
        assert_eq!(hidpp.request(0xFF, 4, 1, &[]).unwrap()[0], 42);
        assert_eq!(
            hidpp.next_frame(Duration::from_millis(1)).unwrap(),
            Some(notification)
        );
        assert_eq!(hidpp.next_frame(Duration::from_millis(1)).unwrap(), None);
    }

    /// A notification whose parameters name its position: two bytes, so they
    /// stay distinct past 255.
    fn numbered(index: usize) -> Vec<u8> {
        let [hi, lo] = u16::try_from(index).unwrap().to_be_bytes();
        frame(0xFF, 9, 0x00, &[hi, lo])
    }

    #[test]
    fn the_oldest_notifications_are_the_ones_dropped_past_the_limit() {
        let mut channel = ScriptedChannel::new();
        for index in 0..=MAX_PENDING_FRAMES {
            channel = channel.then_frame(numbered(index));
        }
        let request = Request::new(0xFF, 4, 1, &[]);
        channel = channel.then_frame(frame(0xFF, 4, request.function_byte(), &[1]));
        let mut hidpp = Hidpp::new(channel);
        hidpp.request(0xFF, 4, 1, &[]).unwrap();
        assert_eq!(hidpp.dropped_frames(), 1);
        let mut kept = Vec::new();
        while let Some(frame) = hidpp.next_frame(Duration::from_millis(1)).unwrap() {
            kept.push(frame);
        }
        assert_eq!(kept.len(), MAX_PENDING_FRAMES);
        assert_eq!(kept.first(), Some(&numbered(1)), "frame 0 was dropped");
        assert_eq!(kept.last(), Some(&numbered(MAX_PENDING_FRAMES)));
    }

    #[test]
    fn notifications_come_back_in_the_order_they_arrived() {
        let request = Request::new(0xFF, 4, 1, &[]);
        let channel = ScriptedChannel::new()
            .then_frame(numbered(1))
            .then_frame(numbered(2))
            .then_frame(frame(0xFF, 4, request.function_byte(), &[7]))
            .then_frame(numbered(3));
        let mut hidpp = Hidpp::new(channel);
        hidpp.request(0xFF, 4, 1, &[]).unwrap();
        let mut seen = Vec::new();
        while let Some(frame) = hidpp.next_frame(Duration::from_millis(1)).unwrap() {
            seen.push(frame);
        }
        assert_eq!(
            seen,
            [numbered(1), numbered(2), numbered(3)],
            "the third was read after the answer"
        );
        assert_eq!(hidpp.dropped_frames(), 0);
    }

    #[test]
    fn a_late_answer_to_a_request_that_gave_up_is_not_kept() {
        let first = Request::new(0xFF, 4, 1, &[]);
        let late_answer = frame(0xFF, 4, first.function_byte(), &[1]);
        let channel = ScriptedChannel::new()
            .then_silence()
            .then_frame(late_answer.clone());
        let mut hidpp = Hidpp::new(channel).with_timeout(Duration::from_millis(5));
        assert!(matches!(
            hidpp.request(0xFF, 4, 1, &[]),
            Err(HidppError::Timeout(_))
        ));
        // The second request reads the late answer to the first (the same
        // feature and function, so it takes it for its own: a device answers one
        // request at a time), and nothing is left over.
        assert!(hidpp.request(0xFF, 4, 1, &[]).is_ok());
        assert_eq!(hidpp.next_frame(Duration::from_millis(1)).unwrap(), None);
        let unrelated = frame(0xFF, 5, 0x1B, &[1]);
        let mut other = Hidpp::new(ScriptedChannel::new().then_frame(unrelated))
            .with_timeout(Duration::from_millis(5));
        assert!(other.request(0xFF, 4, 1, &[]).is_err());
        assert_eq!(
            other.next_frame(Duration::from_millis(1)).unwrap(),
            None,
            "an answer to something else is no notification"
        );
    }

    #[test]
    fn a_flood_of_notifications_ends_at_the_deadline_without_growing() {
        let channel = ScriptedChannel::new().then_flood(numbered(1));
        let mut hidpp = Hidpp::new(channel).with_timeout(Duration::from_millis(5));
        assert!(matches!(
            hidpp.request(0xFF, 4, 1, &[]),
            Err(HidppError::Timeout(_))
        ));
        let mut kept = 0;
        while kept <= MAX_PENDING_FRAMES + 1
            && hidpp
                .next_frame(Duration::from_millis(1))
                .unwrap()
                .is_some()
        {
            kept += 1;
        }
        assert!(kept > MAX_PENDING_FRAMES, "the flood never stops: {kept}");
        assert!(hidpp.dropped_frames() > 0);
    }

    #[test]
    fn frames_set_aside_are_traced_once_when_they_arrive() {
        let request = Request::new(0xFF, 4, 1, &[]);
        let channel = ScriptedChannel::new()
            .then_frame(numbered(1))
            .then_frame(frame(0xFF, 4, request.function_byte(), &[7]));
        let mut hidpp = Hidpp::new(channel).with_trace();
        hidpp.request(0xFF, 4, 1, &[]).unwrap();
        assert!(
            hidpp
                .next_frame(Duration::from_millis(1))
                .unwrap()
                .is_some()
        );
        assert_eq!(
            hidpp.take_trace().len(),
            3,
            "request, notification, answer: no double count"
        );
    }

    #[test]
    fn a_frame_read_directly_is_returned_and_traced() {
        let channel = ScriptedChannel::new().then_frame(frame(0xFF, 4, 0x00, &[1]));
        let mut hidpp = Hidpp::new(channel).with_trace();
        assert!(
            hidpp
                .next_frame(Duration::from_millis(1))
                .unwrap()
                .is_some()
        );
        assert_eq!(hidpp.take_trace().len(), 1);
    }

    #[test]
    fn the_channel_and_its_writes_can_be_inspected() {
        let mut hidpp = Hidpp::new(FakeLink::direct(FakeDevice::new()));
        hidpp.request(0xFF, 0, 1, &[0, 0, 0x5A]).unwrap();
        assert_eq!(hidpp.channel().written().len(), 1);
    }
}
