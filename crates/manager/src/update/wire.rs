//! What the manager and the update helper say to each other, over the
//! helper's socket: three length-prefixed frames one way (the signed
//! checksums, their signature, the package), one line back.
//!
//! The helper runs as root and the socket is open to every local user: it
//! reads bytes, never paths, and refuses any frame larger than its limit
//! before reading it.

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use thiserror::Error;

/// Where the helper listens (systemd socket unit).
pub const SOCKET_PATH: &str = "/run/hubergamof-update.sock";

/// Largest `SHA256SUMS`: it lists two packages.
pub const MAX_CHECKSUMS_BYTES: usize = 64 * 1024;

/// Largest signature: an Ed25519 signature is 64 bytes.
pub const MAX_SIGNATURE_BYTES: usize = 64;

/// Largest package, held in memory by the helper: the real ones are around
/// 15 MiB.
pub const MAX_PACKAGE_BYTES: usize = 64 * 1024 * 1024;

/// Longest the helper waits for the whole bundle: whoever connects cannot
/// keep the (single) update slot busy longer than this.
pub const RECEIVE_DEADLINE: Duration = Duration::from_mins(5);

/// Longest answer line read by the manager.
pub const MAX_RESPONSE_BYTES: usize = 512;

/// Longest failure description sent back.
const MAX_MESSAGE_CHARS: usize = 200;

/// Read in pieces this large, so that the deadline is checked as bytes arrive.
const CHUNK_BYTES: usize = 64 * 1024;

const OK_PREFIX: &str = "ok ";
const ERROR_PREFIX: &str = "error ";

/// A release as the helper receives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// `SHA256SUMS`.
    pub checksums: Vec<u8>,
    /// Ed25519 signature of `SHA256SUMS`.
    pub signature: Vec<u8>,
    /// The package.
    pub package: Vec<u8>,
}

/// Why a bundle could not be read.
#[derive(Debug, Error)]
pub enum WireError {
    /// A frame announces more than it may carry.
    #[error("the {0} frame is larger than allowed")]
    TooLarge(&'static str),
    /// The bundle did not arrive in time.
    #[error("the update was not received in time")]
    TooSlow,
    /// The connection ended early or failed.
    #[error("cannot read the update: {0}")]
    Io(#[from] io::Error),
}

/// The three frames, in the order they travel.
fn frames(bundle: &Bundle) -> [&[u8]; 3] {
    [&bundle.checksums, &bundle.signature, &bundle.package]
}

/// The bundle as it travels: each frame preceded by its length (u32, big endian).
///
/// # Panics
///
/// Never in practice: the frames are bounded far below `u32::MAX` by their
/// callers (a larger one is refused by the helper anyway).
#[must_use]
pub fn encode_bundle(bundle: &Bundle) -> Vec<u8> {
    let mut bytes = Vec::new();
    for frame in frames(bundle) {
        let length = u32::try_from(frame.len()).unwrap_or(u32::MAX);
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.extend_from_slice(frame);
    }
    bytes
}

/// Reads a bundle, refusing oversized frames before reading them and giving up
/// at `deadline`.
///
/// # Errors
///
/// [`WireError`].
pub fn read_bundle(reader: &mut impl Read, deadline: Instant) -> Result<Bundle, WireError> {
    Ok(Bundle {
        checksums: read_frame(reader, "checksums", MAX_CHECKSUMS_BYTES, deadline)?,
        signature: read_frame(reader, "signature", MAX_SIGNATURE_BYTES, deadline)?,
        package: read_frame(reader, "package", MAX_PACKAGE_BYTES, deadline)?,
    })
}

fn read_frame(
    reader: &mut impl Read,
    name: &'static str,
    limit: usize,
    deadline: Instant,
) -> Result<Vec<u8>, WireError> {
    let mut length = [0_u8; 4];
    read_exact_until(reader, &mut length, deadline)?;
    let length = usize::try_from(u32::from_be_bytes(length)).unwrap_or(usize::MAX);
    if length > limit {
        return Err(WireError::TooLarge(name));
    }
    let mut frame = Vec::with_capacity(length.min(CHUNK_BYTES));
    let mut chunk = vec![0_u8; length.min(CHUNK_BYTES)];
    while frame.len() < length {
        let wanted = (length - frame.len()).min(CHUNK_BYTES);
        read_exact_until(reader, &mut chunk[..wanted], deadline)?;
        frame.extend_from_slice(&chunk[..wanted]);
    }
    Ok(frame)
}

/// `read_exact` that also stops at `deadline` (the reader itself times out on
/// a silent peer, see the helper).
fn read_exact_until(
    reader: &mut impl Read,
    buffer: &mut [u8],
    deadline: Instant,
) -> Result<(), WireError> {
    let mut filled = 0;
    while filled < buffer.len() {
        if Instant::now() >= deadline {
            return Err(WireError::TooSlow);
        }
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(WireError::TooSlow);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// What the helper answers: one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// The version installed.
    Installed(String),
    /// Why nothing was installed.
    Failed(String),
}

impl Response {
    /// The line sent, newline included; a failure message is reduced to one
    /// short line of plain text.
    #[must_use]
    pub fn to_line(&self) -> String {
        match self {
            Self::Installed(version) => format!("{OK_PREFIX}{version}\n"),
            Self::Failed(message) => {
                let plain: String = message
                    .chars()
                    .map(|character| {
                        if character.is_control() {
                            ' '
                        } else {
                            character
                        }
                    })
                    .take(MAX_MESSAGE_CHARS)
                    .collect();
                format!("{ERROR_PREFIX}{plain}\n")
            }
        }
    }

    /// Reads a line of at most [`MAX_RESPONSE_BYTES`]; anything that is not an
    /// answer of the helper is a failure with a generic description.
    #[must_use]
    pub fn parse(line: &[u8]) -> Self {
        let unexpected = || Self::Failed("unexpected answer from the update helper".to_owned());
        let Ok(text) = std::str::from_utf8(line) else {
            return unexpected();
        };
        let text = text.trim_end_matches(['\n', '\r']);
        if let Some(version) = text.strip_prefix(OK_PREFIX) {
            Self::Installed(version.to_owned())
        } else if let Some(message) = text.strip_prefix(ERROR_PREFIX) {
            let message: String = message
                .chars()
                .filter(|character| !character.is_control())
                .take(MAX_MESSAGE_CHARS)
                .collect();
            Self::Failed(message)
        } else {
            unexpected()
        }
    }

    /// Writes the line to `writer`.
    ///
    /// # Errors
    ///
    /// The writer's.
    pub fn write_to(&self, writer: &mut impl Write) -> io::Result<()> {
        writer.write_all(self.to_line().as_bytes())?;
        writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> Bundle {
        Bundle {
            checksums: b"sums".to_vec(),
            signature: vec![7; 64],
            package: b"package bytes".to_vec(),
        }
    }

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    #[test]
    fn a_bundle_survives_the_trip() {
        let bytes = encode_bundle(&bundle());
        assert_eq!(
            read_bundle(&mut bytes.as_slice(), soon()).unwrap(),
            bundle()
        );
    }

    #[test]
    fn an_empty_frame_is_a_frame() {
        let empty = Bundle {
            checksums: Vec::new(),
            signature: Vec::new(),
            package: Vec::new(),
        };
        let bytes = encode_bundle(&empty);
        assert_eq!(read_bundle(&mut bytes.as_slice(), soon()).unwrap(), empty);
    }

    #[test]
    fn a_frame_over_its_limit_is_refused_before_it_is_read() {
        for (index, name) in [(0, "checksums"), (1, "signature"), (2, "package")] {
            let mut bytes = Vec::new();
            for _ in 0..index {
                bytes.extend_from_slice(&0_u32.to_be_bytes());
            }
            // Announces u32::MAX bytes and sends none: the length alone is refused.
            bytes.extend_from_slice(&u32::MAX.to_be_bytes());
            let error = read_bundle(&mut bytes.as_slice(), soon()).unwrap_err();
            assert!(
                matches!(error, WireError::TooLarge(found) if found == name),
                "{error}"
            );
        }
    }

    #[test]
    fn the_limits_are_exact() {
        let at_limit = Bundle {
            checksums: vec![0; MAX_CHECKSUMS_BYTES],
            signature: vec![0; MAX_SIGNATURE_BYTES],
            package: Vec::new(),
        };
        assert!(read_bundle(&mut encode_bundle(&at_limit).as_slice(), soon()).is_ok());
        let over = Bundle {
            signature: vec![0; MAX_SIGNATURE_BYTES + 1],
            ..at_limit
        };
        let error = read_bundle(&mut encode_bundle(&over).as_slice(), soon()).unwrap_err();
        assert!(matches!(error, WireError::TooLarge("signature")), "{error}");
    }

    #[test]
    fn a_truncated_bundle_is_an_error() {
        let mut bytes = encode_bundle(&bundle());
        bytes.truncate(bytes.len() - 1);
        let error = read_bundle(&mut bytes.as_slice(), soon()).unwrap_err();
        assert!(matches!(error, WireError::Io(_)), "{error}");
        let error = read_bundle(&mut [0_u8, 0].as_slice(), soon()).unwrap_err();
        assert!(matches!(error, WireError::Io(_)), "{error}");
    }

    #[test]
    fn a_bundle_arriving_after_the_deadline_is_refused() {
        let bytes = encode_bundle(&bundle());
        let past = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        let error = read_bundle(&mut bytes.as_slice(), past).unwrap_err();
        assert!(matches!(error, WireError::TooSlow), "{error}");
    }

    /// A reader that always reports a timeout, like a silent socket peer.
    struct Silent;

    impl Read for Silent {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::WouldBlock.into())
        }
    }

    #[test]
    fn a_silent_peer_times_out() {
        let error = read_bundle(&mut Silent, soon()).unwrap_err();
        assert!(matches!(error, WireError::TooSlow), "{error}");
    }

    #[test]
    fn answers_are_one_plain_line() {
        assert_eq!(Response::Installed("1.2.0".into()).to_line(), "ok 1.2.0\n");
        let line = Response::Failed("bad\nthing\u{1b}[0m".into()).to_line();
        assert_eq!(line.matches('\n').count(), 1);
        assert!(!line.contains('\u{1b}'));
        assert!(Response::Failed("x".repeat(1000)).to_line().len() < 300);
    }

    #[test]
    fn answers_are_read_back_and_anything_else_is_a_failure() {
        assert_eq!(
            Response::parse(b"ok 1.2.0\n"),
            Response::Installed("1.2.0".into())
        );
        assert_eq!(
            Response::parse(b"error signature mismatch\r\n"),
            Response::Failed("signature mismatch".into())
        );
        for garbage in [&b""[..], b"hello", b"\xff\xfe", b"OK 1.0.0"] {
            assert!(
                matches!(Response::parse(garbage), Response::Failed(message) if message.contains("unexpected")),
                "{garbage:?}"
            );
        }
    }

    #[test]
    fn an_answer_can_be_written() {
        let mut out = Vec::new();
        Response::Installed("1.0.1".into())
            .write_to(&mut out)
            .unwrap();
        assert_eq!(out, b"ok 1.0.1\n");
    }
}
