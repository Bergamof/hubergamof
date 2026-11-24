//! Headphones that behave like the real ones, for `--demo` and the tests of
//! the manager: they answer the queries, take the changes and announce them,
//! and can be cut or have a button pressed from outside.
//!
//! What they say is what a WH-1000XM4 was recorded saying (`docs/headset-sony.md`).

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::channel::{Channel, ChannelError};
use crate::frame::{DATA_TYPE_MDR, Decoder, Frame, encode};
use crate::payload::AmbientMode;

/// Battery the simulated headphones report.
const BATTERY_PERCENT: u8 = 80;

const ASK_PROTOCOL: u8 = 0x00;
const ASK_BATTERY: u8 = 0x10;
const ASK_EQUALIZER: u8 = 0x56;
const SET_EQUALIZER: u8 = 0x58;
const ASK_AMBIENT: u8 = 0x66;
const SET_AMBIENT: u8 = 0x68;

/// Preset codes the simulation knows the curve of, and the manual one.
const SPEECH: u8 = 0x17;
const BASS_BOOST: u8 = 0x16;
const MANUAL: u8 = 0xA0;
/// Preset byte of a manual setting of the bands.
const MANUAL_BANDS: u8 = 0xFF;
/// Equalizer values are sent as `value + OFFSET`.
const OFFSET: i16 = 10;

#[derive(Clone, Copy)]
struct Equalizer {
    preset: u8,
    clear_bass: i8,
    bands: [i8; 5],
}

struct State {
    decoder: Decoder,
    outgoing: VecDeque<Vec<u8>>,
    sequence: u8,
    closed: bool,
    mode: AmbientMode,
    level: u8,
    focus_on_voice: bool,
    equalizer: Equalizer,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// Simulated headphones; clones share the same headphones, so a test or the
/// demo keeps one to act on them while the session owns the other.
#[derive(Clone)]
pub struct SimulatedHeadset {
    shared: Arc<Shared>,
}

impl Default for SimulatedHeadset {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulatedHeadset {
    /// Headphones with the ambient sound on, as first recorded.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    decoder: Decoder::new(),
                    outgoing: VecDeque::new(),
                    sequence: 0,
                    closed: false,
                    mode: AmbientMode::Ambient,
                    level: 5,
                    focus_on_voice: false,
                    equalizer: preset_curve(SPEECH),
                }),
                changed: Condvar::new(),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The headphones close the connection, as when another app takes the
    /// settings channel: the next reads fail.
    pub fn cut(&self) {
        self.lock().closed = true;
        self.shared.changed.notify_all();
    }

    /// The person presses the noise cancelling button of the headphones: they
    /// announce the new mode on their own.
    pub fn press_button(&self, mode: AmbientMode) {
        let mut state = self.lock();
        state.mode = mode;
        let payload = ambient_payload(&state, NOTIFY_AMBIENT);
        queue_data(&mut state, payload);
        drop(state);
        self.shared.changed.notify_all();
    }

    /// A new connection to the same headphones, after a cut: the transport
    /// starts over, what the headphones were set to stays.
    #[must_use]
    pub fn reconnect(&self) -> Self {
        let mut state = self.lock();
        state.closed = false;
        state.decoder = Decoder::new();
        state.outgoing.clear();
        state.sequence = 0;
        drop(state);
        self.clone()
    }

    /// The mode the headphones are in now.
    #[must_use]
    pub fn mode(&self) -> AmbientMode {
        self.lock().mode
    }
}

const ANSWER_AMBIENT: u8 = 0x67;
const NOTIFY_AMBIENT: u8 = 0x69;
const ANSWER_EQUALIZER: u8 = 0x57;
const NOTIFY_EQUALIZER: u8 = 0x59;

fn preset_curve(preset: u8) -> Equalizer {
    match preset {
        SPEECH => Equalizer {
            preset,
            clear_bass: -10,
            bands: [4, 3, 1, 2, -10],
        },
        BASS_BOOST => Equalizer {
            preset,
            clear_bass: 7,
            bands: [0; 5],
        },
        _ => Equalizer {
            preset,
            clear_bass: 0,
            bands: [0; 5],
        },
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

fn ambient_payload(state: &State, code: u8) -> Vec<u8> {
    let (effect, mode) = match state.mode {
        AmbientMode::Off => (0x00, 0x00),
        AmbientMode::Ambient => (0x01, 0x00),
        AmbientMode::WindReduction => (0x01, 0x01),
        AmbientMode::NoiseCancelling => (0x01, 0x02),
    };
    vec![
        code,
        0x02,
        effect,
        0x02,
        mode,
        0x01,
        u8::from(state.focus_on_voice),
        state.level,
    ]
}

fn equalizer_payload(equalizer: Equalizer, code: u8) -> Vec<u8> {
    let encode = |value: i8| u8::try_from(i16::from(value) + OFFSET).unwrap_or(0);
    let mut payload = vec![
        code,
        0x01,
        equalizer.preset,
        0x06,
        encode(equalizer.clear_bass),
    ];
    payload.extend(equalizer.bands.map(encode));
    payload
}

/// Applies a change of the ambient sound sent as `68 02 effect capability
/// mode 01 voice level`; false when the command is not that.
fn apply_ambient(state: &mut State, command: &[u8]) -> bool {
    let &[_, _, effect, _, mode, _, voice, level] = command else {
        return false;
    };
    state.mode = match (effect, mode) {
        (0, _) => AmbientMode::Off,
        (_, 0) => AmbientMode::Ambient,
        (_, 1) => AmbientMode::WindReduction,
        _ => AmbientMode::NoiseCancelling,
    };
    state.focus_on_voice = voice != 0;
    state.level = level;
    true
}

/// Applies a change of the equalizer, by preset (`58 01 code 00`) or by hand
/// (`58 01 FF 06 six values`); false when the command is not either.
fn apply_equalizer(state: &mut State, command: &[u8]) -> bool {
    match command {
        [_, _, MANUAL_BANDS, _, bass, b1, b2, b3, b4, b5] => {
            let decode = |raw: u8| i8::try_from(i16::from(raw) - OFFSET).unwrap_or(0);
            state.equalizer = Equalizer {
                preset: MANUAL,
                clear_bass: decode(*bass),
                bands: [*b1, *b2, *b3, *b4, *b5].map(decode),
            };
            true
        }
        [_, _, code, _] => {
            state.equalizer = preset_curve(*code);
            true
        }
        _ => false,
    }
}

fn answer(state: &mut State, payload: &[u8]) {
    match payload.first().copied() {
        Some(ASK_PROTOCOL) => queue_data(state, vec![0x01, 0x00, 0x70, 0x00]),
        Some(ASK_AMBIENT) => {
            let reply = ambient_payload(state, ANSWER_AMBIENT);
            queue_data(state, reply);
        }
        Some(ASK_BATTERY) => queue_data(state, vec![0x11, 0x00, BATTERY_PERCENT, 0x00]),
        Some(ASK_EQUALIZER) => {
            let reply = equalizer_payload(state.equalizer, ANSWER_EQUALIZER);
            queue_data(state, reply);
        }
        Some(SET_AMBIENT) if apply_ambient(state, payload) => {
            let notice = ambient_payload(state, NOTIFY_AMBIENT);
            queue_data(state, notice);
        }
        Some(SET_EQUALIZER) if apply_equalizer(state, payload) => {
            let notice = equalizer_payload(state.equalizer, NOTIFY_EQUALIZER);
            queue_data(state, notice);
        }
        _ => {}
    }
}

impl Channel for SimulatedHeadset {
    fn write(&mut self, bytes: &[u8]) -> Result<(), ChannelError> {
        let mut state = self.lock();
        if state.closed {
            return Err(ChannelError::Closed);
        }
        let frames = state.decoder.push(bytes);
        for frame in frames.into_iter().flatten().filter(|frame| !frame.is_ack()) {
            state
                .outgoing
                .push_back(encode(&Frame::ack(frame.sequence)));
            answer(&mut state, &frame.payload);
        }
        drop(state);
        self.shared.changed.notify_all();
        Ok(())
    }

    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, ChannelError> {
        let state = self.lock();
        let (mut state, _) = self
            .shared
            .changed
            .wait_timeout_while(state, timeout, |state| {
                state.outgoing.is_empty() && !state.closed
            })
            .unwrap_or_else(PoisonError::into_inner);
        if state.closed {
            return Err(ChannelError::Closed);
        }
        Ok(state.outgoing.pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::Link;
    use crate::payload::{
        AmbientSetting, EqualizerPreset, EqualizerSetting, Event, MAX_AMBIENT_LEVEL,
    };
    use crate::session::{Command, Session};

    const SHORT: Duration = Duration::from_millis(50);

    fn opened(headset: &SimulatedHeadset) -> Session<SimulatedHeadset> {
        let mut session = Session::new(headset.clone());
        session.start().unwrap();
        session
    }

    fn drain(session: &mut Session<SimulatedHeadset>) -> Vec<Event> {
        let mut events = Vec::new();
        while let Some(event) = session.poll(SHORT).unwrap() {
            events.push(event);
        }
        events
    }

    fn ambient_of(events: &[Event]) -> crate::payload::AmbientState {
        events
            .iter()
            .rev()
            .find_map(|event| match event {
                Event::Ambient(state) => Some(*state),
                _ => None,
            })
            .expect("an ambient answer")
    }

    #[test]
    fn opening_the_session_tells_everything() {
        let headset = SimulatedHeadset::new();
        let events = drain(&mut opened(&headset));
        let ambient = ambient_of(&events);
        assert_eq!(ambient.mode, AmbientMode::Ambient);
        assert!(ambient.wind_capable);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Battery(_)))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Equalizer(_)))
        );
    }

    #[test]
    fn an_ambient_change_is_applied_and_announced() {
        let headset = SimulatedHeadset::new();
        let mut session = opened(&headset);
        drain(&mut session);
        session
            .apply(&Command::SetAmbient(AmbientSetting {
                mode: AmbientMode::NoiseCancelling,
                level: 0,
                focus_on_voice: true,
            }))
            .unwrap();
        let ambient = ambient_of(&drain(&mut session));
        assert_eq!(ambient.mode, AmbientMode::NoiseCancelling);
        assert!(ambient.focus_on_voice);
        assert_eq!(headset.mode(), AmbientMode::NoiseCancelling);
    }

    #[test]
    fn an_ambient_level_is_kept() {
        let headset = SimulatedHeadset::new();
        let mut session = opened(&headset);
        drain(&mut session);
        session
            .apply(&Command::SetAmbient(AmbientSetting {
                mode: AmbientMode::Ambient,
                level: MAX_AMBIENT_LEVEL,
                focus_on_voice: false,
            }))
            .unwrap();
        assert_eq!(ambient_of(&drain(&mut session)).level, MAX_AMBIENT_LEVEL);
    }

    #[test]
    fn a_button_pressed_on_the_headphones_is_announced_unasked() {
        let headset = SimulatedHeadset::new();
        let mut session = opened(&headset);
        drain(&mut session);
        headset.press_button(AmbientMode::Off);
        let ambient = ambient_of(&drain(&mut session));
        assert_eq!(ambient.mode, AmbientMode::Off);
    }

    #[test]
    fn the_equalizer_takes_presets_and_manual_bands() {
        let headset = SimulatedHeadset::new();
        let mut session = opened(&headset);
        drain(&mut session);
        session
            .apply(&Command::SetEqualizer(EqualizerSetting::Preset(
                EqualizerPreset::BassBoost,
            )))
            .unwrap();
        let equalizer = |events: Vec<Event>| {
            events
                .into_iter()
                .rev()
                .find_map(|event| match event {
                    Event::Equalizer(state) => Some(state),
                    _ => None,
                })
                .expect("an equalizer answer")
        };
        let bass = equalizer(drain(&mut session));
        assert_eq!(bass.preset, EqualizerPreset::BassBoost);
        assert_eq!(bass.clear_bass, 7);

        session
            .apply(&Command::SetEqualizer(EqualizerSetting::Manual {
                clear_bass: -2,
                bands: [1, 2, 3, 4, 5],
            }))
            .unwrap();
        let manual = equalizer(drain(&mut session));
        assert_eq!(manual.preset, EqualizerPreset::Manual);
        assert_eq!((manual.clear_bass, manual.bands), (-2, [1, 2, 3, 4, 5]));
    }

    #[test]
    fn a_cut_headset_fails_the_next_read_and_write() {
        let headset = SimulatedHeadset::new();
        let mut session = opened(&headset);
        drain(&mut session);
        headset.cut();
        assert!(session.poll(SHORT).is_err());
        let mut link = Link::new(headset);
        assert!(link.send(DATA_TYPE_MDR, &[ASK_BATTERY, 0x00]).is_err());
    }

    #[test]
    fn a_cut_headset_can_be_connected_again_with_its_settings() {
        let headset = SimulatedHeadset::new();
        let mut session = opened(&headset);
        drain(&mut session);
        headset.press_button(AmbientMode::NoiseCancelling);
        headset.cut();

        let mut again = opened(&headset.reconnect());
        let ambient = ambient_of(&drain(&mut again));
        assert_eq!(ambient.mode, AmbientMode::NoiseCancelling);
    }

    #[test]
    fn waiting_for_nothing_takes_the_time_asked_not_the_processor() {
        let headset = SimulatedHeadset::new();
        let mut session = opened(&headset);
        drain(&mut session);
        let started = std::time::Instant::now();
        assert!(session.poll(Duration::from_millis(80)).unwrap().is_none());
        assert!(started.elapsed() >= Duration::from_millis(70));
    }
}
