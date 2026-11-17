//! What the browser extension and the manager say to each other.
//!
//! The extension reads Teams (a web page: untrusted) and sends what it saw;
//! [`BrowserReport::sanitized`] bounds every text and count before the
//! module sees it. The manager only ever sends back a fixed set of actions
//! ([`CallAction`]), never text.

use std::io;

use hubergamof_module_api::is_hidden_control;
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
        /// One name per unread conversation.
        unread: Vec<String>,
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
                    .iter()
                    .map(|name| bounded_text(name))
                    .filter(|name| !name.is_empty())
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
            .collect();
        let BrowserReport::Conversations { unread } =
            BrowserReport::Conversations { unread: names }.sanitized()
        else {
            panic!("a list of conversations stays one");
        };
        assert_eq!(unread.len(), MAX_CONVERSATIONS);
        assert_eq!(unread[0], "Alice0");
        assert!(
            unread
                .iter()
                .all(|name| !name.chars().any(char::is_control))
        );

        let BrowserReport::Conversations { unread } = BrowserReport::Conversations {
            unread: vec![String::new(), "x".repeat(500)],
        }
        .sanitized() else {
            panic!("a list of conversations stays one");
        };
        assert_eq!(unread, ["x".repeat(MAX_TEXT_CHARS)]);
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
