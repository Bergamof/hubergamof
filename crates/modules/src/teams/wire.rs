//! What the browser extension and the manager say to each other.
//!
//! The extension reads Teams (a web page: untrusted) and sends what it saw;
//! [`BrowserReport::sanitized`] bounds every text and count before the
//! module sees it. The manager only ever sends back a fixed set of actions
//! ([`CallAction`]), never text.

use std::io;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hubergamof_module_api::{ImageData, is_hidden_control};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Longest frame of the native messaging protocol, in either direction
/// (the browser's own limit for messages to an extension).
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Most unread conversations kept from one report.
pub const MAX_CONVERSATIONS: usize = 50;

/// Most meetings kept from one report.
pub const MAX_MEETINGS: usize = 100;

/// Longest conversation name, caller name or meeting subject, in characters.
pub const MAX_TEXT_CHARS: usize = 120;

/// Longest avatar kept, in base64 characters: a 64 px icon is a few kilobytes,
/// so a larger one is not an icon and is ignored.
pub const MAX_ICON_BASE64: usize = 16 * 1024;

/// Largest side an avatar may announce, in pixels: icons are a few dozen
/// pixels wide, and a few kilobytes of PNG can announce a picture whose
/// decoding takes hundreds of megabytes.
pub const MAX_ICON_SIDE: u32 = 256;

/// Start of a PNG file.
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Start of a JPEG file.
const JPEG_SIGNATURE: &[u8] = &[0xFF, 0xD8, 0xFF];

/// An unread conversation: its name and, when the page gave a usable one, its
/// avatar. On the wire a name alone (an older extension) or an object with
/// the avatar in base64; what is not a PNG or a JPEG of reasonable size is
/// dropped, never an error: the conversation then shows a generic icon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "WireConversation", into = "WireConversation")]
pub struct UnreadConversation {
    /// Name of the conversation.
    pub name: String,
    /// Avatar, PNG or JPEG.
    pub icon: Option<ImageData>,
}

impl UnreadConversation {
    /// A conversation without avatar.
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            icon: None,
        }
    }
}

/// How an [`UnreadConversation`] travels.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum WireConversation {
    Name(String),
    Full {
        name: String,
        #[serde(default)]
        icon: Option<String>,
    },
}

impl From<WireConversation> for UnreadConversation {
    fn from(wire: WireConversation) -> Self {
        match wire {
            WireConversation::Name(name) => Self::named(name),
            WireConversation::Full { name, icon } => Self {
                name,
                icon: icon.as_deref().and_then(decode_icon),
            },
        }
    }
}

impl From<UnreadConversation> for WireConversation {
    fn from(conversation: UnreadConversation) -> Self {
        WireConversation::Full {
            name: conversation.name,
            icon: conversation
                .icon
                .map(|icon| STANDARD.encode(icon.bytes.as_ref())),
        }
    }
}

/// The avatar `base64` holds, if it is a PNG or a JPEG that is not too big,
/// in bytes and in announced size.
fn decode_icon(base64: &str) -> Option<ImageData> {
    if base64.len() > MAX_ICON_BASE64 {
        return None;
    }
    let bytes = STANDARD.decode(base64).ok()?;
    let (width, height) = announced_size(&bytes)?;
    (width <= MAX_ICON_SIDE && height <= MAX_ICON_SIDE).then(|| ImageData::new(bytes))
}

/// Width and height a PNG or a JPEG announces in its header; `None` for
/// anything else or a header that is cut short.
fn announced_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(PNG_SIGNATURE) {
        // After the signature: the length and the name of the first chunk (the
        // header), then the width and the height.
        let be = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
        if bytes.get(8..12)? != 13_u32.to_be_bytes() || bytes.get(12..16)? != b"IHDR" {
            return None;
        }
        return Some((be(16)?, be(20)?));
    }
    if bytes.starts_with(JPEG_SIGNATURE) {
        return jpeg_size(bytes);
    }
    None
}

/// The size of a JPEG, from the one start-of-frame segment of its header.
///
/// Strict where decoders are lenient (fill bytes, markers without length,
/// several frames): anything unusual is refused, because the size must be the
/// one the decoder will use.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const START_OF_SCAN: u8 = 0xDA;
    let mut size = None;
    let mut at = 2;
    loop {
        let [0xFF, marker, high, low] = *bytes.get(at..at + 4)? else {
            return None;
        };
        // Past the header, the frame is known: what follows is entropy-coded.
        if marker == START_OF_SCAN {
            return size;
        }
        // SOF0..SOF15 except the table markers DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            let frame = bytes.get(at + 5..at + 9)?;
            let height = u32::from(u16::from_be_bytes([frame[0], frame[1]]));
            let width = u32::from(u16::from_be_bytes([frame[2], frame[3]]));
            if size.replace((width, height)).is_some() {
                return None;
            }
        }
        // Markers without a length, and fill bytes, have no place here.
        if marker == 0xFF || marker == 0x01 || (0xD0..=0xD9).contains(&marker) {
            return None;
        }
        at += 2 + usize::from(u16::from_be_bytes([high, low]));
    }
}

/// Where a call stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallPhase {
    /// No call.
    #[default]
    Idle,
    /// Somebody is calling.
    Ringing,
    /// Before joining a call or a meeting.
    Prejoin,
    /// In a call.
    InCall,
}

/// A meeting of the calendar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    /// Subject, as shown by the calendar.
    pub subject: String,
    /// Start, in seconds since the Unix epoch.
    pub starts_at: i64,
    /// End, in seconds since the Unix epoch.
    pub ends_at: i64,
}

/// What the extension saw in Teams; each report replaces the previous one of
/// its kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BrowserReport {
    /// Number of unread conversations, from the title of the window.
    #[serde(rename_all = "camelCase")]
    Unread {
        /// The count; `0` when everything is read.
        count: u32,
    },
    /// Names of the unread conversations, read from the conversation list.
    #[serde(rename_all = "camelCase")]
    Conversations {
        /// The unread conversations, with their avatar when there is one.
        unread: Vec<UnreadConversation>,
    },
    /// State of the call.
    #[serde(rename_all = "camelCase")]
    Call {
        /// Where the call stands.
        phase: CallPhase,
        /// The microphone is muted; `None` when unknown.
        muted: Option<bool>,
        /// The camera is on; `None` when unknown.
        camera_on: Option<bool>,
        /// Who is calling, while it rings.
        caller: Option<String>,
    },
    /// Meetings of the calendar, read when the calendar is open.
    #[serde(rename_all = "camelCase")]
    Meetings {
        /// Upcoming meetings.
        items: Vec<Meeting>,
    },
}

/// What the manager asks the extension to do in Teams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallAction {
    /// Mutes or unmutes the microphone.
    ToggleMute,
    /// Turns the camera on or off.
    ToggleCamera,
    /// Leaves the call.
    HangUp,
    /// Accepts the incoming call.
    Accept,
    /// Declines the incoming call.
    Decline,
}

/// A message to the extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ManagerRequest {
    /// Do `action` in Teams.
    Do {
        /// What to do.
        action: CallAction,
    },
}

/// `text` without control characters nor bidirectional overrides (which
/// could dress one name up as another), cut to [`MAX_TEXT_CHARS`]
/// characters.
fn bounded_text(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() && !is_hidden_control(*c))
        .take(MAX_TEXT_CHARS)
        .collect::<String>()
        .trim()
        .to_owned()
}

impl CallPhase {
    /// A call the user acts on: ringing, or joined.
    #[must_use]
    pub fn is_active(self) -> bool {
        matches!(self, Self::Ringing | Self::InCall)
    }
}

impl BrowserReport {
    /// The report with every text and list bounded: what the page sent is
    /// never trusted. Empty names are dropped; a meeting ending before it
    /// starts is dropped.
    #[must_use]
    pub fn sanitized(self) -> Self {
        match self {
            Self::Unread { count } => Self::Unread { count },
            Self::Conversations { unread } => Self::Conversations {
                unread: unread
                    .into_iter()
                    .map(|conversation| UnreadConversation {
                        name: bounded_text(&conversation.name),
                        icon: conversation.icon,
                    })
                    .filter(|conversation| !conversation.name.is_empty())
                    .take(MAX_CONVERSATIONS)
                    .collect(),
            },
            Self::Call {
                phase,
                muted,
                camera_on,
                caller,
            } => Self::Call {
                phase,
                muted,
                camera_on,
                caller: caller
                    .map(|caller| bounded_text(&caller))
                    .filter(|caller| !caller.is_empty()),
            },
            Self::Meetings { items } => Self::Meetings {
                items: items
                    .into_iter()
                    .filter(|meeting| meeting.ends_at >= meeting.starts_at)
                    .map(|meeting| Meeting {
                        subject: bounded_text(&meeting.subject),
                        ..meeting
                    })
                    .take(MAX_MEETINGS)
                    .collect(),
            },
        }
    }
}

/// Reads one native messaging frame: its length (4 bytes, native byte
/// order) then that many bytes. `None` at the clean end of the stream; an
/// error for a frame over `max` bytes (not read) or a stream cut short.
///
/// # Errors
///
/// Fails on an I/O error, an oversized frame or a truncated one.
pub async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> io::Result<Option<Vec<u8>>> {
    let mut length = [0_u8; 4];
    // No byte at all is the clean end of the stream; a header cut short is not.
    if reader.read(&mut length[..1]).await? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut length[1..]).await?;
    let length = usize::try_from(u32::from_ne_bytes(length))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame length"))?;
    if length > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {length} bytes over the limit of {max}"),
        ));
    }
    let mut frame = vec![0_u8; length];
    reader.read_exact(&mut frame).await?;
    Ok(Some(frame))
}

/// Writes one native messaging frame.
///
/// # Errors
///
/// Fails on an I/O error or a frame over [`MAX_FRAME_BYTES`].
pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, frame: &[u8]) -> io::Result<()> {
    if frame.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame over the limit",
        ));
    }
    let length = u32::try_from(frame.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame length"))?;
    writer.write_all(&length.to_ne_bytes()).await?;
    writer.write_all(frame).await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_are_tagged_camel_case_json() {
        let report: BrowserReport = serde_json::from_str(
            r#"{"type":"call","phase":"inCall","muted":true,"cameraOn":null,"caller":null}"#,
        )
        .unwrap();
        assert_eq!(
            report,
            BrowserReport::Call {
                phase: CallPhase::InCall,
                muted: Some(true),
                camera_on: None,
                caller: None,
            }
        );
        let request = serde_json::to_string(&ManagerRequest::Do {
            action: CallAction::HangUp,
        })
        .unwrap();
        assert_eq!(request, r#"{"type":"do","action":"hangUp"}"#);
    }

    #[test]
    fn unknown_reports_and_actions_are_refused() {
        assert!(serde_json::from_str::<BrowserReport>(r#"{"type":"eval","code":"1"}"#).is_err());
        assert!(serde_json::from_str::<ManagerRequest>(r#"{"type":"do","action":"run"}"#).is_err());
    }

    #[test]
    fn what_the_page_sends_is_bounded() {
        let long = "é".repeat(MAX_TEXT_CHARS * 3);
        let names = (0..MAX_CONVERSATIONS * 2)
            .map(|i| format!("  Alice\u{0}\n{i} "))
            .chain([String::new(), "   ".into(), long])
            .map(UnreadConversation::named)
            .collect();
        let BrowserReport::Conversations { unread } =
            BrowserReport::Conversations { unread: names }.sanitized()
        else {
            panic!("a list of conversations stays one");
        };
        assert_eq!(unread.len(), MAX_CONVERSATIONS);
        assert_eq!(unread[0].name, "Alice0");
        assert!(
            unread
                .iter()
                .all(|conversation| !conversation.name.chars().any(char::is_control))
        );

        let BrowserReport::Conversations { unread } = BrowserReport::Conversations {
            unread: vec![String::new(), "x".repeat(500)]
                .into_iter()
                .map(UnreadConversation::named)
                .collect(),
        }
        .sanitized() else {
            panic!("a list of conversations stays one");
        };
        assert_eq!(
            unread,
            [UnreadConversation::named("x".repeat(MAX_TEXT_CHARS))]
        );
    }

    /// The start of a PNG announcing `side` × `side` pixels.
    fn png_of(side: u32) -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&side.to_be_bytes());
        bytes.extend_from_slice(&side.to_be_bytes());
        bytes
    }

    fn png() -> Vec<u8> {
        png_of(64)
    }

    /// The start of a JPEG announcing `side` × `side` pixels, after a comment,
    /// up to its scan.
    fn jpeg_of(side: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xFE, 0x00, 0x04, b'h', b'i'];
        bytes.extend_from_slice(&jpeg_frame(side));
        bytes.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);
        bytes
    }

    fn jpeg_frame(side: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xC0, 0x00, 0x0B, 8];
        bytes.extend_from_slice(&side.to_be_bytes());
        bytes.extend_from_slice(&side.to_be_bytes());
        bytes.extend_from_slice(&[1, 1, 0x11, 0]);
        bytes
    }

    fn conversation_json(icon: &str) -> String {
        format!(r#"{{"type":"conversations","unread":[{{"name":"Alice","icon":{icon}}}]}}"#)
    }

    fn only_conversation(json: &str) -> UnreadConversation {
        let report: BrowserReport = serde_json::from_str(json).unwrap();
        let BrowserReport::Conversations { mut unread } = report.sanitized() else {
            panic!("a list of conversations stays one");
        };
        assert_eq!(unread.len(), 1);
        unread.remove(0)
    }

    #[test]
    fn an_avatar_that_is_a_png_or_a_jpeg_is_kept() {
        let png_icon = STANDARD.encode(png());
        let kept = only_conversation(&conversation_json(&format!("\"{png_icon}\"")));
        assert_eq!(kept.icon, Some(ImageData::new(png())));

        let jpeg = jpeg_of(64);
        let jpeg_icon = STANDARD.encode(&jpeg);
        let kept = only_conversation(&conversation_json(&format!("\"{jpeg_icon}\"")));
        assert_eq!(kept.icon, Some(ImageData::new(jpeg)));
    }

    #[test]
    fn what_is_not_a_usable_avatar_leaves_the_name_and_drops_the_icon() {
        let svg = STANDARD.encode(b"<svg xmlns='http://www.w3.org/2000/svg'/>");
        let too_big = STANDARD.encode([png(), vec![0; MAX_ICON_BASE64]].concat());
        let announcing_a_huge_picture = STANDARD.encode(png_of(4096));
        let huge_jpeg = STANDARD.encode(jpeg_of(4096));
        let cut_short = STANDARD.encode(PNG_SIGNATURE);
        // Two frames: the decoder might use the other one than the first.
        let two_frames = STANDARD.encode(
            [
                vec![0xFF, 0xD8, 0xFF, 0xFE, 0x00, 0x02],
                jpeg_frame(64),
                jpeg_frame(4096),
                vec![0xFF, 0xDA, 0x00, 0x02],
            ]
            .concat(),
        );
        // A fill byte before a marker, which decoders skip and a parser may not.
        let fill_bytes = STANDARD.encode(
            [
                vec![0xFF, 0xD8, 0xFF, 0xFF, 0xFE, 0x00, 0x02],
                jpeg_frame(64),
                vec![0xFF, 0xDA, 0x00, 0x02],
            ]
            .concat(),
        );
        let mut wrong_header_length = png_of(64);
        wrong_header_length[8..12].copy_from_slice(&99_u32.to_be_bytes());
        let wrong_header_length = STANDARD.encode(wrong_header_length);
        for icon in [
            "null".to_owned(),
            r#""not base64 !""#.to_owned(),
            format!("\"{svg}\""),
            format!("\"{too_big}\""),
            format!("\"{announcing_a_huge_picture}\""),
            format!("\"{huge_jpeg}\""),
            format!("\"{cut_short}\""),
            format!("\"{two_frames}\""),
            format!("\"{fill_bytes}\""),
            format!("\"{wrong_header_length}\""),
        ] {
            let conversation = only_conversation(&conversation_json(&icon));
            assert_eq!(conversation, UnreadConversation::named("Alice"), "{icon}");
        }
    }

    #[test]
    fn an_avatar_that_is_not_text_makes_the_report_unreadable() {
        let json = conversation_json("42");
        assert!(serde_json::from_str::<BrowserReport>(&json).is_err());
    }

    #[test]
    fn a_name_alone_is_still_a_conversation() {
        let report: BrowserReport =
            serde_json::from_str(r#"{"type":"conversations","unread":["Alice","Bob"]}"#).unwrap();
        let BrowserReport::Conversations { unread } = report else {
            panic!("a list of conversations stays one");
        };
        assert_eq!(
            unread,
            [
                UnreadConversation::named("Alice"),
                UnreadConversation::named("Bob")
            ]
        );
    }

    #[test]
    fn a_conversation_goes_through_the_wire_with_its_avatar() {
        let conversation = UnreadConversation {
            name: "Alice".into(),
            icon: Some(ImageData::new(png())),
        };
        let json = serde_json::to_string(&conversation).unwrap();
        assert_eq!(
            serde_json::from_str::<UnreadConversation>(&json).unwrap(),
            conversation
        );
    }

    #[test]
    fn names_cannot_hide_behind_bidirectional_overrides() {
        let report = BrowserReport::Call {
            phase: CallPhase::Ringing,
            muted: None,
            camera_on: None,
            caller: Some("Ali\u{202E}ce\u{2066}".into()),
        }
        .sanitized();
        assert!(matches!(
            report,
            BrowserReport::Call { caller: Some(ref caller), .. } if caller == "Alice"
        ));
    }

    #[test]
    fn a_caller_that_is_blank_is_unknown() {
        let report = BrowserReport::Call {
            phase: CallPhase::Ringing,
            muted: None,
            camera_on: None,
            caller: Some(" \n ".into()),
        }
        .sanitized();
        assert!(matches!(report, BrowserReport::Call { caller: None, .. }));
    }

    #[test]
    fn meetings_are_bounded_and_inverted_ones_dropped() {
        let meeting = |starts_at, ends_at| Meeting {
            subject: "Point\u{7}".into(),
            starts_at,
            ends_at,
        };
        let items = std::iter::once(meeting(20, 10))
            .chain((0..MAX_MEETINGS * 2).map(|_| meeting(10, 20)))
            .collect();
        let BrowserReport::Meetings { items } = BrowserReport::Meetings { items }.sanitized()
        else {
            panic!("meetings stay meetings");
        };
        assert_eq!(items.len(), MAX_MEETINGS);
        assert_eq!(items[0].subject, "Point");
    }

    #[tokio::test]
    async fn frames_round_trip_and_the_stream_ends_cleanly() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, b"{}").await.unwrap();
        write_frame(&mut bytes, b"[1]").await.unwrap();
        let mut reader = bytes.as_slice();
        assert_eq!(
            read_frame(&mut reader, 16).await.unwrap().as_deref(),
            Some(&b"{}"[..])
        );
        assert_eq!(
            read_frame(&mut reader, 16).await.unwrap().as_deref(),
            Some(&b"[1]"[..])
        );
        assert_eq!(read_frame(&mut reader, 16).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_header_cut_short_is_an_error_not_an_end() {
        let mut reader: &[u8] = &[1, 0];
        assert!(read_frame(&mut reader, 16).await.is_err());
    }

    #[tokio::test]
    async fn an_oversized_or_truncated_frame_is_an_error() {
        let mut oversized: &[u8] = &[255, 255, 255, 255];
        assert!(read_frame(&mut oversized, MAX_FRAME_BYTES).await.is_err());

        let mut truncated = Vec::from(10_u32.to_ne_bytes());
        truncated.extend_from_slice(b"abc");
        let mut reader = truncated.as_slice();
        assert!(read_frame(&mut reader, 16).await.is_err());

        assert!(
            write_frame(&mut Vec::new(), &vec![0; MAX_FRAME_BYTES + 1])
                .await
                .is_err()
        );
    }
}
