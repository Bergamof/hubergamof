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

use crate::channel::{Channel, ChannelError};
use crate::frame::{DATA_TYPE_MDR, Frame};
use crate::link::{Link, LinkError};
use crate::payload::{
    AmbientSetting, AmbientState, EqualizerError, EqualizerSetting, Event, ProbeError, Query,
    SettingError, decode,
};

/// How long to wait for more answers after the last one (or for the first).
pub const REPLY_WAIT: Duration = Duration::from_millis(1000);

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
    /// Change the equalizer, then read it back.
    Equalizer(EqualizerSetting),
    /// Send one query from the closed list (with argument bytes for the
    /// undocumented ones) and print the answers.
    Probe {
        /// The query.
        query: Query,
        /// Bytes appended after its code.
        args: Vec<u8>,
    },
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
    Link(LinkError),
    /// The headphones closed the connection.
    #[error(
        "the headphones closed the connection: another app (the Sony app on a phone?) probably took the settings channel"
    )]
    Closed,
    /// The change asked for is not one the headphones accept.
    #[error(transparent)]
    Setting(#[from] SettingError),
    /// The equalizer change asked for is not one the headphones accept.
    #[error(transparent)]
    Equalizer(#[from] EqualizerError),
    /// The probe asked for cannot be built.
    #[error(transparent)]
    Probe(#[from] ProbeError),
    /// The headphones did not say whether they have wind noise reduction.
    #[error("the headphones did not report their ambient sound state: run `info` first")]
    NoAmbientState,
    /// The output could not be written.
    #[error("cannot write the report: {0}")]
    Output(#[from] io::Error),
}

impl From<LinkError> for DiagnosticError {
    fn from(error: LinkError) -> Self {
        match error {
            LinkError::Channel(ChannelError::Closed) => Self::Closed,
            other => Self::Link(other),
        }
    }
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
        Action::Equalizer(setting) => session.equalizer(*setting),
        Action::Probe { query, args } => session.probe(*query, args),
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
                writeln!(self.out)?;
                self.describe(&event)?;
                Ok(Some(event))
            }
            Err(error) => {
                writeln!(self.out, "\n   {error}")?;
                Ok(None)
            }
        }
    }

    fn describe(&mut self, event: &Event) -> Result<(), DiagnosticError> {
        match event {
            Event::PairedDevices { devices, trailing } => {
                writeln!(self.out, "   {} paired device(s):", devices.len())?;
                for device in devices {
                    writeln!(
                        self.out,
                        "   {}  state 0x{:02X}  {}",
                        device.address, device.state, device.name
                    )?;
                }
                writeln!(self.out, "   after the last one: {}", hex(trailing))?;
            }
            other => writeln!(self.out, "   {other:?}")?,
        }
        Ok(())
    }

    /// Sends a query and shows every answer that follows it.
    fn ask(&mut self, query: Query) -> Result<Vec<Event>, DiagnosticError> {
        self.ask_with(query, &[])
    }

    fn ask_with(&mut self, query: Query, args: &[u8]) -> Result<Vec<Event>, DiagnosticError> {
        let payload = query.payload_with(args)?;
        writeln!(self.out, "[{}]", query.name())?;
        self.send(&payload)?;
        self.show_replies()
    }

    fn probe(&mut self, query: Query, args: &[u8]) -> Result<(), DiagnosticError> {
        self.ask_with(query, args).map(|_| ())
    }

    fn equalizer(&mut self, setting: EqualizerSetting) -> Result<(), DiagnosticError> {
        let payload = setting.payload()?;
        writeln!(self.out, "[set equalizer {setting:?}]")?;
        self.send(&payload)?;
        self.show_replies()?;
        self.ask(Query::Equalizer).map(|_| ())
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
    use crate::payload::{AmbientMode, EqualizerPreset};
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
        let (result, report) = run_on(
            &headset,
            &Action::Probe {
                query: Query::ConnectionStatus,
                args: Vec::new(),
            },
        );
        result.unwrap();
        assert_eq!(headset.received(), [vec![0x24]]);
        assert!(report.contains("<- type 0x0C seq 0: 25 01 02"));
    }

    #[test]
    fn a_probe_appends_its_arguments_only_where_allowed() {
        let headset = FakeHeadset::new().answering(&[0x24, 0x01], &[&[0x25, 0x31, 0x01, 0x01]]);
        let probe = |query, args: &[u8]| Action::Probe {
            query,
            args: args.to_vec(),
        };
        let (result, report) = run_on(&headset, &probe(Query::ConnectionStatus, &[0x01]));
        result.unwrap();
        assert_eq!(headset.received(), [vec![0x24, 0x01]]);
        assert!(report.contains("<- type 0x0C seq 0: 25 31 01 01"));

        let (result, _) = run_on(&headset, &probe(Query::Battery, &[0x01]));
        assert!(matches!(result, Err(DiagnosticError::Probe(_))));
        assert_eq!(headset.received().len(), 1, "nothing was sent");
    }

    #[test]
    fn an_equalizer_change_is_sent_then_read_back() {
        let headset = FakeHeadset::new().answering(
            &Query::Equalizer.payload(),
            &[&[0x57, 0x01, 0x16, 0x06, 0x0A, 0x0A, 0x0A, 0x0A, 0x0A, 0x0A]],
        );
        let setting = EqualizerSetting::Preset(EqualizerPreset::BassBoost);
        let (result, report) = run_on(&headset, &Action::Equalizer(setting));
        result.unwrap();
        assert_eq!(
            headset.received(),
            [vec![0x58, 0x01, 0x16, 0x00], Query::Equalizer.payload()]
        );
        assert!(report.contains("preset: BassBoost"));
    }

    #[test]
    fn an_equalizer_value_out_of_range_is_not_sent() {
        let headset = FakeHeadset::new();
        let setting = EqualizerSetting::Manual {
            clear_bass: 11,
            bands: [0; 5],
        };
        let (result, _) = run_on(&headset, &Action::Equalizer(setting));
        assert!(matches!(result, Err(DiagnosticError::Equalizer(_))));
        assert_eq!(headset.received(), Vec::<Vec<u8>>::new());
    }

    #[test]
    fn a_connection_closed_by_the_headphones_is_said_plainly() {
        let headset = talkative();
        headset.close();
        let (result, _) = run_on(&headset, &Action::Info);
        let error = result.unwrap_err();
        assert!(matches!(error, DiagnosticError::Closed));
        assert!(error.to_string().contains("another app"));
    }

    #[test]
    fn paired_devices_are_listed_one_per_line() {
        let list = [
            0x39, 0x01, 0x01, b'0', b'0', b':', b'1', b'1', b':', b'2', b'2', b':', b'3', b'3',
            b':', b'4', b'4', b':', b'5', b'5', 0x02, 0x04, b'T', b'e', b's', b't', 0x02,
        ];
        let headset = FakeHeadset::new().answering(&[0x38], &[&list]);
        let probe = Action::Probe {
            query: Query::PairedDevices,
            args: Vec::new(),
        };
        let (result, report) = run_on(&headset, &probe);
        result.unwrap();
        assert!(report.contains("1 paired device(s):"));
        assert!(report.contains("00:11:22:33:44:55  state 0x02  Test"));
        assert!(report.contains("after the last one: 02"));
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
