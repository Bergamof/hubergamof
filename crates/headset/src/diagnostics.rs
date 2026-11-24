//! What `hubergamof headset-test` does with headphones, one action at a time:
//! reads their state, listens to what they announce, or makes the one change
//! the person typed, and says what happened.
//!
//! The raw payloads are always shown next to their reading: this is how the
//! values nobody documented for the WH-1000XM4 are found.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::channel::Channel;
use crate::frame::{DATA_TYPE_MDR, Frame};
use crate::link::{Link, LinkError};
use crate::payload::{AmbientSetting, AmbientState, Event, Query, SettingError, decode};

/// How long to wait for more answers after the last one (or for the first).
pub const REPLY_WAIT: Duration = Duration::from_millis(600);

/// Data type of the second MDR channel some headphones use.
const DATA_TYPE_MDR_NO2: u8 = 0x0E;
/// Data type of plain data frames.
const DATA_TYPE_DATA: u8 = 0x00;

/// The data type frames are sent with: unknown for the WH-1000XM4, so the
/// three the other projects use can be tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataKind {
    /// `0x0C`, what most headphones use.
    Mdr,
    /// `0x0E`.
    MdrNo2,
    /// `0x00`.
    Data,
}

impl DataKind {
    fn byte(self) -> u8 {
        match self {
            Self::Mdr => DATA_TYPE_MDR,
            Self::MdrNo2 => DATA_TYPE_MDR_NO2,
            Self::Data => DATA_TYPE_DATA,
        }
    }
}

/// One thing to do to the headphones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Read and print everything known (nothing is changed).
    Info,
    /// Change the ambient sound.
    Ambient(AmbientSetting),
    /// Ask for the states, then print everything the headphones announce.
    Watch(Duration),
    /// Send one query from the closed list and print the answers.
    Probe(Query),
}

/// How the action talks to the headphones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// The data type of the frames sent.
    pub data_kind: DataKind,
    /// How long to wait for answers.
    pub reply_wait: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            data_kind: DataKind::Mdr,
            reply_wait: REPLY_WAIT,
        }
    }
}

/// Why an action failed.
#[derive(Debug, Error)]
pub enum DiagnosticError {
    /// A frame could not be exchanged.
    #[error(transparent)]
    Link(#[from] LinkError),
    /// The change asked for is not one the headphones accept.
    #[error(transparent)]
    Setting(#[from] SettingError),
    /// The headphones did not say whether they have wind noise reduction.
    #[error("the headphones did not report their ambient sound state: run `info` first")]
    NoAmbientState,
    /// The output could not be written.
    #[error("cannot write the report: {0}")]
    Output(#[from] io::Error),
}

/// `00 1F ab` for a payload.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Does the action on the headphones and writes what it found or did. A watch
/// ends early when `stop` is raised.
///
/// # Errors
/// [`DiagnosticError`] when a frame cannot be exchanged, the change is refused
/// or the output cannot be written.
pub fn run<C: Channel>(
    link: &mut Link<C>,
    action: &Action,
    settings: Settings,
    out: &mut impl Write,
    stop: &AtomicBool,
) -> Result<(), DiagnosticError> {
    let mut session = Session {
        link,
        settings,
        out,
    };
    match action {
        Action::Info => session.info(),
        Action::Ambient(setting) => session.ambient(*setting),
        Action::Watch(duration) => session.watch(*duration, stop),
        Action::Probe(query) => session.ask(*query).map(|_| ()),
    }
}

struct Session<'a, C, W> {
    link: &'a mut Link<C>,
    settings: Settings,
    out: &'a mut W,
}

impl<C: Channel, W: Write> Session<'_, C, W> {
    fn send(&mut self, payload: &[u8]) -> Result<(), DiagnosticError> {
        writeln!(self.out, "-> {}", hex(payload))?;
        self.link.send(self.settings.data_kind.byte(), payload)?;
        Ok(())
    }

    fn show(&mut self, frame: &Frame) -> Result<Option<Event>, DiagnosticError> {
        write!(
            self.out,
            "<- type 0x{:02X} seq {}: {}",
            frame.data_type,
            frame.sequence,
            hex(&frame.payload)
        )?;
        match decode(&frame.payload) {
            Ok(Event::Other(_)) => {
                writeln!(self.out)?;
                Ok(None)
            }
            Ok(event) => {
                writeln!(self.out, "\n   {event:?}")?;
                Ok(Some(event))
            }
            Err(error) => {
                writeln!(self.out, "\n   {error}")?;
                Ok(None)
            }
        }
    }

    /// Sends a query and shows every answer that follows it.
    fn ask(&mut self, query: Query) -> Result<Vec<Event>, DiagnosticError> {
        writeln!(self.out, "[{}]", query.name())?;
        self.send(&query.payload())?;
        self.show_replies()
    }

    fn show_replies(&mut self) -> Result<Vec<Event>, DiagnosticError> {
        let mut events = Vec::new();
        while let Some(frame) = self.link.receive(self.settings.reply_wait)? {
            events.extend(self.show(&frame)?);
        }
        Ok(events)
    }

    fn info(&mut self) -> Result<(), DiagnosticError> {
        for query in [
            Query::ProtocolInfo,
            Query::Ambient,
            Query::Battery,
            Query::Equalizer,
        ] {
            self.ask(query)?;
        }
        Ok(())
    }

    fn ambient_state(&mut self) -> Result<AmbientState, DiagnosticError> {
        self.ask(Query::Ambient)?
            .into_iter()
            .find_map(|event| match event {
                Event::Ambient(state) => Some(state),
                _ => None,
            })
            .ok_or(DiagnosticError::NoAmbientState)
    }

    fn ambient(&mut self, setting: AmbientSetting) -> Result<(), DiagnosticError> {
        let state = self.ambient_state()?;
        let payload = setting.payload(state.wind_capable)?;
        writeln!(self.out, "[set ambient {setting:?}]")?;
        self.send(&payload)?;
        self.show_replies()?;
        self.ask(Query::Ambient).map(|_| ())
    }

    fn watch(&mut self, duration: Duration, stop: &AtomicBool) -> Result<(), DiagnosticError> {
        // The headphones announce a change only after the matching query.
        for query in [Query::Ambient, Query::Battery, Query::Equalizer] {
            self.ask(query)?;
        }
        writeln!(
            self.out,
            "Listening for {} s: change the settings on the headphones or in the Sony app.",
            duration.as_secs()
        )?;
        let end = Instant::now() + duration;
        while Instant::now() < end && !stop.load(Ordering::Relaxed) {
            let wait = end
                .saturating_duration_since(Instant::now())
                .min(self.settings.reply_wait);
            if let Some(frame) = self.link.receive(wait)? {
                self.show(&frame)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::payload::AmbientMode;
    use crate::testing::FakeHeadset;

    const AMBIENT_REPLY: [u8; 8] = [0x67, 0x02, 0x01, 0x02, 0x00, 0x01, 0x00, 0x05];

    fn settings() -> Settings {
        Settings {
            data_kind: DataKind::Mdr,
            reply_wait: Duration::from_millis(5),
        }
    }

    fn run_on(headset: &FakeHeadset, action: &Action) -> (Result<(), DiagnosticError>, String) {
        let mut link = Link::new(headset.clone()).with_ack_timeout(Duration::from_millis(5));
        let mut out = Vec::new();
        let result = run(
            &mut link,
            action,
            settings(),
            &mut out,
            &AtomicBool::new(false),
        );
        (result, String::from_utf8(out).unwrap())
    }

    fn talkative() -> FakeHeadset {
        FakeHeadset::new()
            .answering(&Query::Ambient.payload(), &[&AMBIENT_REPLY])
            .answering(&Query::Battery.payload(), &[&[0x11, 0x00, 0x50, 0x00]])
    }

    #[test]
    fn info_shows_each_answer_raw_and_read() {
        let (result, report) = run_on(&talkative(), &Action::Info);
        result.unwrap();
        assert!(report.contains("-> 66 02"));
        assert!(report.contains("<- type 0x0C seq 0: 67 02 01 02 00 01 00 05"));
        assert!(report.contains("Ambient(AmbientState"));
        assert!(report.contains("Battery(Single"));
    }

    #[test]
    fn a_change_uses_the_capability_the_headphones_reported() {
        let headset = talkative();
        let setting = AmbientSetting {
            mode: AmbientMode::NoiseCancelling,
            level: 0,
            focus_on_voice: false,
        };
        let (result, _) = run_on(&headset, &Action::Ambient(setting));
        result.unwrap();
        assert!(
            headset
                .received()
                .contains(&vec![0x68, 0x02, 0x11, 0x02, 0x02, 0x01, 0x00, 0x00])
        );
    }

    #[test]
    fn a_change_is_not_sent_when_the_state_is_unknown() {
        let headset = FakeHeadset::new();
        let setting = AmbientSetting {
            mode: AmbientMode::Ambient,
            level: 3,
            focus_on_voice: false,
        };
        let (result, _) = run_on(&headset, &Action::Ambient(setting));
        assert!(matches!(result, Err(DiagnosticError::NoAmbientState)));
        assert_eq!(headset.received(), [Query::Ambient.payload()]);
    }

    #[test]
    fn a_refused_setting_is_not_sent() {
        let setting = AmbientSetting {
            mode: AmbientMode::WindReduction,
            level: 0,
            focus_on_voice: false,
        };
        let basic = FakeHeadset::new().answering(
            &Query::Ambient.payload(),
            &[&[0x67, 0x02, 0x01, 0x00, 0x00, 0x01, 0x00, 0x05]],
        );
        let (result, _) = run_on(&basic, &Action::Ambient(setting));
        assert!(matches!(result, Err(DiagnosticError::Setting(_))));
        assert_eq!(basic.received().len(), 1);
    }

    #[test]
    fn watching_asks_first_then_prints_what_is_announced() {
        let headset = talkative();
        headset.notify(&[0x69, 0x02, 0x01, 0x02, 0x01, 0x01, 0x00, 0x00]);
        let (result, report) = run_on(&headset, &Action::Watch(Duration::from_millis(30)));
        result.unwrap();
        assert!(report.contains("Listening for"));
        assert!(report.contains("WindReduction"));
    }

    #[test]
    fn watching_ends_when_asked() {
        let mut link = Link::new(talkative()).with_ack_timeout(Duration::from_millis(5));
        let mut out = Vec::new();
        let stop = AtomicBool::new(true);
        run(
            &mut link,
            &Action::Watch(Duration::from_secs(3600)),
            settings(),
            &mut out,
            &stop,
        )
        .unwrap();
    }

    #[test]
    fn a_probe_sends_only_its_query_and_shows_unknown_answers() {
        let headset = FakeHeadset::new().answering(&[0x24], &[&[0x25, 0x01, 0x02]]);
        let (result, report) = run_on(&headset, &Action::Probe(Query::ConnectionStatus));
        result.unwrap();
        assert_eq!(headset.received(), [vec![0x24]]);
        assert!(report.contains("<- type 0x0C seq 0: 25 01 02"));
    }

    #[test]
    fn headphones_that_never_acknowledge_give_a_clear_error() {
        let headset = FakeHeadset::new().dropping_acks(100);
        let (result, _) = run_on(&headset, &Action::Info);
        assert!(matches!(
            result,
            Err(DiagnosticError::Link(LinkError::NoAck(_)))
        ));
    }

    #[test]
    fn the_data_type_is_the_one_asked_for() {
        assert_eq!(DataKind::Mdr.byte(), 0x0C);
        assert_eq!(DataKind::MdrNo2.byte(), 0x0E);
        assert_eq!(DataKind::Data.byte(), 0x00);
        assert_eq!(hex(&[0x00, 0x1F, 0xAB]), "00 1F AB");
    }
}
