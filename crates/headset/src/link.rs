//! The exchange of frames: acknowledgements, sequence numbers and resends.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::channel::{Channel, ChannelError};
use crate::frame::{Decoder, Frame, FrameError, MAX_PAYLOAD, encode};

/// Time the headphones are given to acknowledge a frame before it is sent again.
pub const ACK_TIMEOUT: Duration = Duration::from_millis(1000);

/// Times a frame is sent before the headphones count as not answering.
pub const MAX_ATTEMPTS: u32 = 3;

/// Frames received while waiting for an acknowledgement, kept for the reader;
/// the oldest are dropped past this.
const MAX_PENDING: usize = 256;

/// Which way a traced frame went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// From this computer to the headphones.
    Sent,
    /// From the headphones.
    Received,
}

/// Why a frame could not be exchanged.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LinkError {
    /// The headphones never acknowledged the frame.
    #[error("no acknowledgement after {0} attempts")]
    NoAck(u32),
    /// The payload is longer than any frame holds.
    #[error("payload of {0} bytes is too long")]
    TooLong(usize),
    /// The stream failed.
    #[error("stream error: {0}")]
    Channel(#[from] ChannelError),
}

type Trace = Box<dyn FnMut(Direction, &Frame) + Send>;

/// The frames of one connection.
pub struct Link<C> {
    channel: C,
    decoder: Decoder,
    next_sequence: u8,
    ack_timeout: Duration,
    received: VecDeque<Frame>,
    trace: Option<Trace>,
}

impl<C: Channel> Link<C> {
    /// A link on a connected stream; the first frame sent has sequence 0.
    pub fn new(channel: C) -> Self {
        Self {
            channel,
            decoder: Decoder::new(),
            next_sequence: 0,
            ack_timeout: ACK_TIMEOUT,
            received: VecDeque::new(),
            trace: None,
        }
    }

    /// Waits `timeout` for an acknowledgement instead of [`ACK_TIMEOUT`].
    #[must_use]
    pub fn with_ack_timeout(mut self, timeout: Duration) -> Self {
        self.ack_timeout = timeout;
        self
    }

    /// Calls `trace` for every frame sent or received, acknowledgements included.
    pub fn set_trace(&mut self, trace: impl FnMut(Direction, &Frame) + Send + 'static) {
        self.trace = Some(Box::new(trace));
    }

    /// Sends a frame and waits for its acknowledgement, sending it again
    /// (same sequence) when none comes.
    ///
    /// # Errors
    /// [`LinkError`] when the payload is too long, the stream fails or the
    /// headphones never acknowledge.
    pub fn send(&mut self, data_type: u8, payload: &[u8]) -> Result<(), LinkError> {
        if payload.len() > MAX_PAYLOAD {
            return Err(LinkError::TooLong(payload.len()));
        }
        let frame = Frame {
            data_type,
            sequence: self.next_sequence,
            payload: payload.to_vec(),
        };
        let wire = encode(&frame);
        for _ in 0..MAX_ATTEMPTS {
            self.write_traced(&frame, &wire)?;
            if self.wait_for_ack()? {
                self.next_sequence ^= 1;
                return Ok(());
            }
        }
        Err(LinkError::NoAck(MAX_ATTEMPTS))
    }

    /// The next frame carrying data, acknowledged on its way in; `None` when
    /// none arrives within `timeout`.
    ///
    /// # Errors
    /// [`LinkError`] when the stream fails.
    pub fn receive(&mut self, timeout: Duration) -> Result<Option<Frame>, LinkError> {
        if let Some(frame) = self.received.pop_front() {
            return Ok(Some(frame));
        }
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            for frame in self.read_frames(left)? {
                if !frame.is_ack() {
                    self.received.push_back(frame);
                }
            }
            if let Some(frame) = self.received.pop_front() {
                return Ok(Some(frame));
            }
            if left.is_zero() {
                return Ok(None);
            }
        }
    }

    fn wait_for_ack(&mut self) -> Result<bool, LinkError> {
        let deadline = Instant::now() + self.ack_timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            for frame in self.read_frames(left)? {
                if frame.is_ack() {
                    return Ok(true);
                }
                if self.received.len() >= MAX_PENDING {
                    self.received.pop_front();
                }
                self.received.push_back(frame);
            }
            if left.is_zero() {
                return Ok(false);
            }
        }
    }

    /// Reads once, acknowledges every data frame and returns the frames.
    /// Frames that cannot be read are logged and dropped.
    fn read_frames(&mut self, timeout: Duration) -> Result<Vec<Frame>, LinkError> {
        let Some(bytes) = self.channel.read(timeout)? else {
            return Ok(Vec::new());
        };
        let mut frames = Vec::new();
        for decoded in self.decoder.push(&bytes) {
            match decoded {
                Ok(frame) => {
                    self.trace_frame(Direction::Received, &frame);
                    if !frame.is_ack() {
                        let ack = Frame::ack(frame.sequence);
                        self.write_traced(&ack, &encode(&ack))?;
                    }
                    frames.push(frame);
                }
                Err(error) => log_unreadable(&error),
            }
        }
        Ok(frames)
    }

    fn write_traced(&mut self, frame: &Frame, wire: &[u8]) -> Result<(), LinkError> {
        self.trace_frame(Direction::Sent, frame);
        self.channel.write(wire)?;
        Ok(())
    }

    fn trace_frame(&mut self, direction: Direction, frame: &Frame) {
        if let Some(trace) = self.trace.as_mut() {
            trace(direction, frame);
        }
    }
}

fn log_unreadable(error: &FrameError) {
    tracing::warn!(%error, "unreadable frame from the headphones, dropped");
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::frame::DATA_TYPE_MDR;
    use crate::testing::FakeHeadset;

    const SHORT: Duration = Duration::from_millis(5);

    fn link(headset: FakeHeadset) -> Link<FakeHeadset> {
        Link::new(headset).with_ack_timeout(SHORT)
    }

    #[test]
    fn a_sent_frame_is_acknowledged_and_the_sequence_flips() {
        let headset = FakeHeadset::new();
        let mut link = link(headset.clone());
        link.send(DATA_TYPE_MDR, &[0x66, 0x02]).unwrap();
        link.send(DATA_TYPE_MDR, &[0x10, 0x00]).unwrap();
        assert_eq!(link.next_sequence, 0);
        assert_eq!(headset.received(), [vec![0x66, 0x02], vec![0x10, 0x00]]);
    }

    #[test]
    fn an_answer_is_received_and_acknowledged() {
        let headset = FakeHeadset::new().answering(&[0x66, 0x02], &[&[0x67, 0x02]]);
        let mut link = link(headset.clone());
        link.send(DATA_TYPE_MDR, &[0x66, 0x02]).unwrap();
        let answer = link.receive(SHORT).unwrap().unwrap();
        assert_eq!(answer.payload, [0x67, 0x02]);
        assert_eq!(link.receive(SHORT).unwrap(), None);
    }

    #[test]
    fn an_unacknowledged_frame_is_sent_again_with_the_same_sequence() {
        let headset = FakeHeadset::new().dropping_acks(2);
        let mut link = link(headset.clone());
        link.send(DATA_TYPE_MDR, &[0x66, 0x02]).unwrap();
        assert_eq!(headset.received().len(), 3);
    }

    #[test]
    fn a_frame_never_acknowledged_is_given_up() {
        let headset = FakeHeadset::new().dropping_acks(MAX_ATTEMPTS);
        let mut link = link(headset);
        assert_eq!(
            link.send(DATA_TYPE_MDR, &[0x66, 0x02]),
            Err(LinkError::NoAck(MAX_ATTEMPTS))
        );
    }

    #[test]
    fn a_notification_during_the_wait_is_kept_for_the_reader() {
        let headset = FakeHeadset::new();
        headset.notify(&[0x69, 0x02]);
        let mut link = link(headset);
        link.send(DATA_TYPE_MDR, &[0x66, 0x02]).unwrap();
        assert_eq!(link.receive(SHORT).unwrap().unwrap().payload, [0x69, 0x02]);
    }

    #[test]
    fn a_frame_split_across_reads_is_received_whole() {
        let headset = FakeHeadset::new();
        headset.notify(&[0x69, 0x02, 0x01]);
        headset.split_next_read(4);
        let mut link = link(headset);
        assert_eq!(
            link.receive(SHORT).unwrap().unwrap().payload,
            [0x69, 0x02, 0x01]
        );
    }

    #[test]
    fn a_payload_longer_than_any_frame_is_refused() {
        let mut link = link(FakeHeadset::new());
        assert_eq!(
            link.send(DATA_TYPE_MDR, &vec![0; MAX_PAYLOAD + 1]),
            Err(LinkError::TooLong(MAX_PAYLOAD + 1))
        );
    }

    #[test]
    fn a_closed_stream_is_an_error_not_silence() {
        let headset = FakeHeadset::new();
        headset.close();
        let mut link = link(headset);
        assert!(matches!(link.receive(SHORT), Err(LinkError::Channel(_))));
    }

    #[test]
    fn every_frame_is_traced_in_both_directions() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let headset = FakeHeadset::new().answering(&[0x66, 0x02], &[&[0x67, 0x02]]);
        let mut link = link(headset);
        let log = Arc::clone(&seen);
        link.set_trace(move |direction, frame| {
            log.lock().unwrap().push((direction, frame.is_ack()));
        });
        link.send(DATA_TYPE_MDR, &[0x66, 0x02]).unwrap();
        link.receive(SHORT).unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            [
                (Direction::Sent, false),
                (Direction::Received, true),
                (Direction::Received, false),
                (Direction::Sent, true),
            ]
        );
    }
}
