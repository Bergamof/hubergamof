//! The framing of the Sony MDR protocol over RFCOMM (pure, no I/O).
//!
//! A frame on the wire is `START body END` where the body is
//! `[data type][sequence][payload length, u32 big-endian][payload][checksum]`.
//! The checksum is the wrapping sum of the bytes before it. Inside the body,
//! the markers and the escape byte are sent as `ESCAPE (byte & 0xEF)`.

use thiserror::Error;

/// First byte of a frame.
pub const START: u8 = 0x3E;
/// Last byte of a frame.
pub const END: u8 = 0x3C;
/// Introduces an escaped byte.
pub const ESCAPE: u8 = 0x3D;

/// Bit cleared by the escaping (`0x3C` is sent as `0x3D 0x2C`).
const ESCAPE_MASK: u8 = 0xEF;
/// Bit set back when unescaping.
const ESCAPE_BIT: u8 = 0x10;

/// Data type of the acknowledgement of a frame.
pub const DATA_TYPE_ACK: u8 = 0x01;
/// Data type of the commands and answers of the headphones' settings (MDR).
pub const DATA_TYPE_MDR: u8 = 0x0C;

/// Bytes before the payload in an unescaped body: type, sequence, length.
const HEADER_LEN: usize = 6;
/// Longest payload accepted, in either direction (the official client stops at 2048).
pub const MAX_PAYLOAD: usize = 2048;
/// Longest escaped frame: every body byte doubled, plus checksum and markers.
const MAX_WIRE_FRAME: usize = 2 * (HEADER_LEN + MAX_PAYLOAD + 1) + 2;

/// One frame, unescaped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// What the payload is (see the `DATA_TYPE_*` constants).
    pub data_type: u8,
    /// `0` or `1`, flipped by the sender after each acknowledged frame.
    pub sequence: u8,
    /// The command or answer.
    pub payload: Vec<u8>,
}

impl Frame {
    /// A frame acknowledging one received with `received_sequence`.
    #[must_use]
    pub fn ack(received_sequence: u8) -> Self {
        Self {
            data_type: DATA_TYPE_ACK,
            sequence: received_sequence ^ 1,
            payload: Vec::new(),
        }
    }

    /// Whether this frame only acknowledges another.
    #[must_use]
    pub fn is_ack(&self) -> bool {
        self.data_type == DATA_TYPE_ACK
    }
}

/// A frame that cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FrameError {
    /// The checksum does not match the body.
    #[error("bad checksum: frame says 0x{found:02X}, bytes give 0x{expected:02X}")]
    Checksum {
        /// What the bytes add up to.
        expected: u8,
        /// What the frame carries.
        found: u8,
    },
    /// The body is shorter than its header, or its length field is wrong.
    #[error("bad length: {0}")]
    Length(String),
    /// An escape byte is followed by something that cannot be escaped.
    #[error("bad escape sequence 0x{0:02X}")]
    Escape(u8),
    /// More bytes than any frame holds arrived before the end marker.
    #[error("frame longer than {MAX_WIRE_FRAME} bytes")]
    TooLong,
}

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}

/// The bytes to write for `frame`.
///
/// # Panics
/// Never for a payload within [`MAX_PAYLOAD`] (checked by the callers).
#[must_use]
pub fn encode(frame: &Frame) -> Vec<u8> {
    debug_assert!(frame.payload.len() <= MAX_PAYLOAD);
    let mut body = Vec::with_capacity(HEADER_LEN + frame.payload.len() + 1);
    body.push(frame.data_type);
    body.push(frame.sequence);
    body.extend_from_slice(
        &u32::try_from(frame.payload.len())
            .unwrap_or(0)
            .to_be_bytes(),
    );
    body.extend_from_slice(&frame.payload);
    body.push(checksum(&body));

    let mut wire = Vec::with_capacity(body.len() + 2);
    wire.push(START);
    for byte in body {
        if matches!(byte, START | END | ESCAPE) {
            wire.push(ESCAPE);
            wire.push(byte & ESCAPE_MASK);
        } else {
            wire.push(byte);
        }
    }
    wire.push(END);
    wire
}

/// Turns a stream of bytes, cut anywhere, into frames.
#[derive(Debug, Default)]
pub struct Decoder {
    /// Escaped bytes of the frame being read, `None` between frames.
    current: Option<Vec<u8>>,
}

impl Decoder {
    /// An empty decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads `bytes`; every frame they complete is returned, readable or not.
    /// Bytes outside a frame are dropped, and a start marker inside one starts
    /// it over (the previous frame was cut).
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<Frame, FrameError>> {
        let mut frames = Vec::new();
        for &byte in bytes {
            match (byte, self.current.as_mut()) {
                (START, _) => self.current = Some(Vec::new()),
                (END, Some(_)) => {
                    let escaped = self.current.take().unwrap_or_default();
                    frames.push(decode_body(&escaped));
                }
                (_, Some(escaped)) => {
                    if escaped.len() >= MAX_WIRE_FRAME {
                        self.current = None;
                        frames.push(Err(FrameError::TooLong));
                    } else {
                        escaped.push(byte);
                    }
                }
                (_, None) => {}
            }
        }
        frames
    }
}

fn unescape(escaped: &[u8]) -> Result<Vec<u8>, FrameError> {
    let mut body = Vec::with_capacity(escaped.len());
    let mut bytes = escaped.iter();
    while let Some(&byte) = bytes.next() {
        if byte == ESCAPE {
            let &next = bytes.next().ok_or(FrameError::Escape(ESCAPE))?;
            let original = next | ESCAPE_BIT;
            if !matches!(original, START | END | ESCAPE) {
                return Err(FrameError::Escape(next));
            }
            body.push(original);
        } else {
            body.push(byte);
        }
    }
    Ok(body)
}

fn decode_body(escaped: &[u8]) -> Result<Frame, FrameError> {
    let body = unescape(escaped)?;
    if body.len() < HEADER_LEN + 1 {
        return Err(FrameError::Length(format!("{} bytes", body.len())));
    }
    let (content, tail) = body.split_at(body.len() - 1);
    let expected = checksum(content);
    let found = tail[0];
    if expected != found {
        return Err(FrameError::Checksum { expected, found });
    }
    let declared = u32::from_be_bytes([content[2], content[3], content[4], content[5]]);
    let payload = &content[HEADER_LEN..];
    if usize::try_from(declared).ok() != Some(payload.len()) || payload.len() > MAX_PAYLOAD {
        return Err(FrameError::Length(format!(
            "header says {declared}, frame holds {}",
            payload.len()
        )));
    }
    Ok(Frame {
        data_type: content[0],
        sequence: content[1],
        payload: payload.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_ambient() -> Frame {
        Frame {
            data_type: DATA_TYPE_MDR,
            sequence: 0,
            payload: vec![0x66, 0x02],
        }
    }

    #[test]
    fn a_command_is_encoded_like_the_documented_example() {
        assert_eq!(
            encode(&get_ambient()),
            [
                0x3E, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x02, 0x66, 0x02, 0x76, 0x3C
            ]
        );
    }

    #[test]
    fn an_acknowledgement_flips_the_sequence_and_has_no_payload() {
        assert_eq!(
            encode(&Frame::ack(0)),
            [0x3E, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x02, 0x3C]
        );
        assert!(Frame::ack(1).is_ack());
        assert_eq!(Frame::ack(1).sequence, 0);
    }

    #[test]
    fn markers_and_escape_bytes_are_escaped_and_come_back() {
        let frame = Frame {
            data_type: DATA_TYPE_MDR,
            sequence: 1,
            payload: vec![START, END, ESCAPE, 0x00],
        };
        let wire = encode(&frame);
        let inner = &wire[1..wire.len() - 1];
        assert!(!inner.contains(&START) && !inner.contains(&END));
        assert!(wire.windows(2).any(|w| w == [ESCAPE, 0x2E]));
        assert!(wire.windows(2).any(|w| w == [ESCAPE, 0x2C]));
        assert!(wire.windows(2).any(|w| w == [ESCAPE, 0x2D]));
        assert_eq!(Decoder::new().push(&wire), vec![Ok(frame)]);
    }

    #[test]
    fn a_frame_cut_anywhere_is_read_once_complete() {
        let wire = encode(&get_ambient());
        for cut in 1..wire.len() {
            let mut decoder = Decoder::new();
            assert!(decoder.push(&wire[..cut]).is_empty(), "cut at {cut}");
            assert_eq!(decoder.push(&wire[cut..]), vec![Ok(get_ambient())]);
        }
    }

    #[test]
    fn several_frames_in_one_read_are_all_returned() {
        let mut wire = encode(&Frame::ack(1));
        wire.extend(encode(&get_ambient()));
        let frames = Decoder::new().push(&wire);
        assert_eq!(frames.len(), 2);
        assert!(frames[0].as_ref().is_ok_and(Frame::is_ack));
    }

    #[test]
    fn noise_between_frames_is_dropped() {
        let mut wire = vec![0x00, 0xFF, END];
        wire.extend(encode(&get_ambient()));
        assert_eq!(Decoder::new().push(&wire), vec![Ok(get_ambient())]);
    }

    #[test]
    fn a_cut_frame_is_dropped_when_another_starts() {
        let mut wire = vec![START, 0x0C, 0x00];
        wire.extend(encode(&get_ambient()));
        assert_eq!(Decoder::new().push(&wire), vec![Ok(get_ambient())]);
    }

    #[test]
    fn a_wrong_checksum_is_reported() {
        let mut wire = encode(&get_ambient());
        let checksum_at = wire.len() - 2;
        wire[checksum_at] ^= 0x01;
        assert_eq!(
            Decoder::new().push(&wire),
            vec![Err(FrameError::Checksum {
                expected: 0x76,
                found: 0x77
            })]
        );
    }

    #[test]
    fn a_length_that_lies_is_reported() {
        let mut body = vec![DATA_TYPE_MDR, 0, 0, 0, 0, 9, 0x66, 0x02];
        body.push(checksum(&body));
        let mut wire = vec![START];
        wire.extend(body);
        wire.push(END);
        assert!(matches!(
            Decoder::new().push(&wire).as_slice(),
            [Err(FrameError::Length(_))]
        ));
    }

    #[test]
    fn a_body_shorter_than_its_header_is_reported() {
        let wire = [START, 0x01, 0x02, END];
        assert!(matches!(
            Decoder::new().push(&wire).as_slice(),
            [Err(FrameError::Length(_))]
        ));
    }

    #[test]
    fn a_bad_escape_is_reported() {
        let wire = [START, ESCAPE, 0x41, END];
        assert_eq!(
            Decoder::new().push(&wire),
            vec![Err(FrameError::Escape(0x41))]
        );
        assert_eq!(
            Decoder::new().push(&[START, ESCAPE, END]),
            vec![Err(FrameError::Escape(ESCAPE))]
        );
    }

    #[test]
    fn a_frame_that_never_ends_is_dropped_not_buffered_forever() {
        let mut decoder = Decoder::new();
        let mut results = decoder.push(&[START]);
        results.extend(decoder.push(&vec![0x00; MAX_WIRE_FRAME + 10]));
        assert_eq!(results, vec![Err(FrameError::TooLong)]);
        assert_eq!(
            decoder.push(&encode(&get_ambient())),
            vec![Ok(get_ambient())]
        );
    }
}
