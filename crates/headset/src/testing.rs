//! A scripted headset for tests: speaks the framing, acknowledges what it
//! receives and answers the payloads it was given an answer for.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::channel::{Channel, ChannelError};
use crate::frame::{DATA_TYPE_MDR, Decoder, Frame, encode};

#[derive(Default)]
struct State {
    decoder: Decoder,
    /// Bytes the "headphones" will return to the next reads.
    outgoing: VecDeque<Vec<u8>>,
    /// Payload received → payloads answered.
    answers: HashMap<Vec<u8>, Vec<Vec<u8>>>,
    /// Data payloads received, in order.
    received: Vec<Vec<u8>>,
    /// Acknowledgements still to withhold, to test resends.
    acks_to_drop: u32,
    /// Whether the stream is closed (reads fail).
    closed: bool,
    sequence: u8,
}

/// The scripted headphones; clones share the same state, so a test keeps one
/// while the link owns the other.
#[derive(Clone, Default)]
pub struct FakeHeadset {
    state: Arc<Mutex<State>>,
}

impl FakeHeadset {
    /// Headphones that acknowledge everything and answer nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Answers `request` (a payload) with `answers` (payloads, in order).
    #[must_use]
    pub fn answering(self, request: &[u8], answers: &[&[u8]]) -> Self {
        self.lock().answers.insert(
            request.to_vec(),
            answers.iter().map(|answer| answer.to_vec()).collect(),
        );
        self
    }

    /// Withholds the next `count` acknowledgements.
    #[must_use]
    pub fn dropping_acks(self, count: u32) -> Self {
        self.lock().acks_to_drop = count;
        self
    }

    /// Sends a notification by itself, as when a button is pressed.
    pub fn notify(&self, payload: &[u8]) {
        let mut state = self.lock();
        queue_data(&mut state, payload.to_vec());
    }

    /// Makes the next bytes arrive cut at `at` (to test split reads).
    pub fn split_next_read(&self, at: usize) {
        let mut state = self.lock();
        if let Some(bytes) = state.outgoing.pop_front() {
            let at = at.min(bytes.len());
            state.outgoing.push_front(bytes[at..].to_vec());
            state.outgoing.push_front(bytes[..at].to_vec());
        }
    }

    /// Closes the stream: the next read fails.
    pub fn close(&self) {
        self.lock().closed = true;
    }

    /// The data payloads received so far.
    #[must_use]
    pub fn received(&self) -> Vec<Vec<u8>> {
        self.lock().received.clone()
    }
}

fn queue_data(state: &mut State, payload: Vec<u8>) {
    let frame = Frame {
        data_type: DATA_TYPE_MDR,
        sequence: state.sequence,
        payload,
    };
    state.sequence ^= 1;
    state.outgoing.push_back(encode(&frame));
}

impl Channel for FakeHeadset {
    fn write(&mut self, bytes: &[u8]) -> Result<(), ChannelError> {
        let mut state = self.lock();
        let frames = state.decoder.push(bytes);
        for frame in frames.into_iter().flatten() {
            if frame.is_ack() {
                continue;
            }
            state.received.push(frame.payload.clone());
            if state.acks_to_drop > 0 {
                state.acks_to_drop -= 1;
                continue;
            }
            state
                .outgoing
                .push_back(encode(&Frame::ack(frame.sequence)));
            for answer in state
                .answers
                .get(&frame.payload)
                .cloned()
                .unwrap_or_default()
            {
                queue_data(&mut state, answer);
            }
        }
        Ok(())
    }

    fn read(&mut self, _timeout: Duration) -> Result<Option<Vec<u8>>, ChannelError> {
        let mut state = self.lock();
        if state.closed {
            return Err(ChannelError("the headphones closed the stream".into()));
        }
        Ok(state.outgoing.pop_front())
    }
}
