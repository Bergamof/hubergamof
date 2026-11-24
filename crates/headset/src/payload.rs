//! The commands this project sends and the answers it reads (pure).
//!
//! Byte values come from other projects' reverse engineering (Gadgetbridge,
//! SonyHeadphonesClient, mdr-protocol), not from a capture on a WH-1000XM4:
//! `docs/headset-sony.md` records what was checked on the real headphones.
//! Nothing here is built from bytes a client sent: only the closed values of
//! these types reach the headphones.

use thiserror::Error;

/// Highest ambient sound level.
pub const MAX_AMBIENT_LEVEL: u8 = 20;

const GET_BATTERY: u8 = 0x10;
const RET_BATTERY: u8 = 0x11;
const NTFY_BATTERY: u8 = 0x13;
const GET_EQUALIZER: u8 = 0x56;
const RET_EQUALIZER: u8 = 0x57;
const NTFY_EQUALIZER: u8 = 0x59;
const GET_AMBIENT: u8 = 0x66;
const RET_AMBIENT: u8 = 0x67;
const SET_AMBIENT: u8 = 0x68;
const NTFY_AMBIENT: u8 = 0x69;

/// Inquired type of the noise cancelling and ambient sound setting.
const AMBIENT_INQUIRED_TYPE: u8 = 0x02;
/// Inquired type of a single battery (headband headphones).
const SINGLE_BATTERY: u8 = 0x00;
/// Inquired type of the equalizer.
const EQUALIZER_INQUIRED_TYPE: u8 = 0x01;

/// Capability byte: only noise cancelling and ambient sound.
const CAPABILITY_BASIC: u8 = 0x00;
/// Capability byte: wind noise reduction too.
const CAPABILITY_WIND: u8 = 0x02;
/// Effect byte of a setting that turns the mode on.
const EFFECT_ON: u8 = 0x11;
/// Setting type byte: the level is adjusted.
const SETTING_LEVEL: u8 = 0x01;

/// What a read command asks, from a closed list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Query {
    /// First command of a session: what protocol the headphones speak.
    ProtocolInfo,
    /// Noise cancelling and ambient sound.
    Ambient,
    /// Battery level.
    Battery,
    /// Equalizer preset and bands.
    Equalizer,
    /// Which devices are connected (payload unknown: the bare code is sent).
    ConnectionStatus,
    /// Information about the paired devices (payload unknown: the bare code is sent).
    DeviceInfo,
}

impl Query {
    /// Every query, in the order `probe` lists them.
    pub const ALL: [Self; 6] = [
        Self::ProtocolInfo,
        Self::Ambient,
        Self::Battery,
        Self::Equalizer,
        Self::ConnectionStatus,
        Self::DeviceInfo,
    ];

    /// The name a person types.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::ProtocolInfo => "protocol-info",
            Self::Ambient => "ambient",
            Self::Battery => "battery",
            Self::Equalizer => "equalizer",
            Self::ConnectionStatus => "connection-status",
            Self::DeviceInfo => "device-info",
        }
    }

    /// The query named `name`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|query| query.name() == name)
    }

    /// The payload to send.
    #[must_use]
    pub fn payload(self) -> Vec<u8> {
        match self {
            Self::ProtocolInfo => vec![0x00, 0x00],
            Self::Ambient => vec![GET_AMBIENT, AMBIENT_INQUIRED_TYPE],
            Self::Battery => vec![GET_BATTERY, SINGLE_BATTERY],
            Self::Equalizer => vec![GET_EQUALIZER, EQUALIZER_INQUIRED_TYPE],
            Self::ConnectionStatus => vec![0x24],
            Self::DeviceInfo => vec![0x1C],
        }
    }
}

/// What noise cancelling and ambient sound do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbientMode {
    /// Neither: the headphones play as they are.
    Off,
    /// Outside sound is let in.
    Ambient,
    /// Noise is cancelled.
    NoiseCancelling,
    /// Wind noise is reduced (only on headphones that report it).
    WindReduction,
}

/// A request to change the ambient sound, with the closed values the headphones accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmbientSetting {
    /// What to do.
    pub mode: AmbientMode,
    /// How much outside sound, `0..=MAX_AMBIENT_LEVEL` (used by `Off` and `Ambient`).
    pub level: u8,
    /// Whether voices are let through.
    pub focus_on_voice: bool,
}

/// Why a setting cannot be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SettingError {
    /// The level is above [`MAX_AMBIENT_LEVEL`].
    #[error("ambient level {0} is above {MAX_AMBIENT_LEVEL}")]
    Level(u8),
    /// These headphones did not report wind noise reduction.
    #[error("these headphones have no wind noise reduction")]
    NoWindReduction,
}

impl AmbientSetting {
    /// The payload of the command, for headphones that report (or not) wind
    /// noise reduction.
    ///
    /// # Errors
    /// [`SettingError`] for a level out of range or an unsupported mode.
    pub fn payload(self, wind_capable: bool) -> Result<Vec<u8>, SettingError> {
        if self.level > MAX_AMBIENT_LEVEL {
            return Err(SettingError::Level(self.level));
        }
        let capability = if wind_capable {
            CAPABILITY_WIND
        } else {
            CAPABILITY_BASIC
        };
        let focus = u8::from(self.focus_on_voice);
        let (effect, mode_byte, focus_byte, level_byte) = match self.mode {
            AmbientMode::Off => (0x00, 0x00, 0x00, self.level),
            AmbientMode::Ambient => (EFFECT_ON, 0x00, focus, self.level),
            AmbientMode::NoiseCancelling => (
                EFFECT_ON,
                if wind_capable { 0x02 } else { 0x01 },
                focus,
                0x00,
            ),
            AmbientMode::WindReduction if wind_capable => (EFFECT_ON, 0x01, focus, 0x00),
            AmbientMode::WindReduction => return Err(SettingError::NoWindReduction),
        };
        Ok(vec![
            SET_AMBIENT,
            AMBIENT_INQUIRED_TYPE,
            effect,
            capability,
            mode_byte,
            SETTING_LEVEL,
            focus_byte,
            level_byte,
        ])
    }
}

/// The ambient sound state the headphones report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmbientState {
    /// Current mode.
    pub mode: AmbientMode,
    /// Whether the headphones report wind noise reduction.
    pub wind_capable: bool,
    /// Whether voices are let through.
    pub focus_on_voice: bool,
    /// Outside sound level, `0..=MAX_AMBIENT_LEVEL`.
    pub level: u8,
}

/// One battery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryCell {
    /// Percent, `0..=100`.
    pub level: u8,
    /// Whether it is charging.
    pub charging: bool,
}

/// The battery state the headphones report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Battery {
    /// One battery (headband headphones).
    Single(BatteryCell),
    /// Two batteries.
    LeftRight {
        /// Left earbud.
        left: BatteryCell,
        /// Right earbud.
        right: BatteryCell,
    },
}

/// The equalizer as the headphones report it (preset codes are not named yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EqualizerState {
    /// The preset code, as sent.
    pub preset: u8,
    /// Bass boost, `-10..=10`.
    pub clear_bass: i8,
    /// The five bands, `-10..=10`, in the order the headphones send them.
    pub bands: [i8; 5],
}

/// What the headphones said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Noise cancelling and ambient sound.
    Ambient(AmbientState),
    /// Battery.
    Battery(Battery),
    /// Equalizer.
    Equalizer(EqualizerState),
    /// Something this project does not read (kept to be shown by `watch`).
    Other(Vec<u8>),
}

/// An answer that is not what its code promises.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("malformed answer 0x{code:02X}: {reason}")]
pub struct MalformedAnswer {
    /// The first byte of the payload.
    pub code: u8,
    /// What is wrong.
    pub reason: String,
}

fn malformed(code: u8, reason: impl Into<String>) -> MalformedAnswer {
    MalformedAnswer {
        code,
        reason: reason.into(),
    }
}

/// Reads one answer or notification.
///
/// # Errors
/// [`MalformedAnswer`] when a known code carries impossible values.
pub fn decode(payload: &[u8]) -> Result<Event, MalformedAnswer> {
    match payload.first().copied() {
        Some(RET_AMBIENT | NTFY_AMBIENT) => decode_ambient(payload).map(Event::Ambient),
        Some(RET_BATTERY | NTFY_BATTERY) => decode_battery(payload).map(Event::Battery),
        Some(RET_EQUALIZER | NTFY_EQUALIZER) => decode_equalizer(payload).map(Event::Equalizer),
        _ => Ok(Event::Other(payload.to_vec())),
    }
}

const AMBIENT_LEN: usize = 8;

fn decode_ambient(payload: &[u8]) -> Result<AmbientState, MalformedAnswer> {
    let code = payload[0];
    let [_, kind, effect, capability, mode, _, focus, level] = payload[..] else {
        return Err(malformed(
            code,
            format!("{} bytes, expected {AMBIENT_LEN}", payload.len()),
        ));
    };
    if kind != AMBIENT_INQUIRED_TYPE {
        return Err(malformed(code, format!("inquired type 0x{kind:02X}")));
    }
    if level > MAX_AMBIENT_LEVEL {
        return Err(malformed(code, format!("level {level}")));
    }
    let wind_capable = match capability {
        CAPABILITY_BASIC => false,
        CAPABILITY_WIND => true,
        other => return Err(malformed(code, format!("capability 0x{other:02X}"))),
    };
    let mode = if effect == 0 {
        AmbientMode::Off
    } else {
        match (wind_capable, mode) {
            (_, 0x00) => AmbientMode::Ambient,
            (false, 0x01) | (true, 0x02) => AmbientMode::NoiseCancelling,
            (true, 0x01) => AmbientMode::WindReduction,
            _ => return Err(malformed(code, format!("mode 0x{mode:02X}"))),
        }
    };
    Ok(AmbientState {
        mode,
        wind_capable,
        focus_on_voice: focus != 0,
        level,
    })
}

fn battery_cell(code: u8, level: u8, charging: u8) -> Result<BatteryCell, MalformedAnswer> {
    if level > 100 {
        return Err(malformed(code, format!("battery level {level}")));
    }
    Ok(BatteryCell {
        level,
        charging: charging != 0,
    })
}

fn decode_battery(payload: &[u8]) -> Result<Battery, MalformedAnswer> {
    let code = payload[0];
    match payload[..] {
        [_, SINGLE_BATTERY, level, charging, ..] => {
            battery_cell(code, level, charging).map(Battery::Single)
        }
        [_, 0x01, left, left_charging, right, right_charging, ..] => Ok(Battery::LeftRight {
            left: battery_cell(code, left, left_charging)?,
            right: battery_cell(code, right, right_charging)?,
        }),
        _ => Err(malformed(code, format!("{} bytes", payload.len()))),
    }
}

/// Equalizer values are sent as `value + EQUALIZER_OFFSET`.
const EQUALIZER_OFFSET: i16 = 10;
const EQUALIZER_LEN: usize = 10;

fn equalizer_value(code: u8, raw: u8) -> Result<i8, MalformedAnswer> {
    let value = i16::from(raw) - EQUALIZER_OFFSET;
    i8::try_from(value)
        .ok()
        .filter(|value| (-10..=10).contains(value))
        .ok_or_else(|| malformed(code, format!("equalizer value {raw}")))
}

fn decode_equalizer(payload: &[u8]) -> Result<EqualizerState, MalformedAnswer> {
    let code = payload[0];
    let [_, _, preset, _, clear_bass, b1, b2, b3, b4, b5] = payload[..] else {
        return Err(malformed(
            code,
            format!("{} bytes, expected {EQUALIZER_LEN}", payload.len()),
        ));
    };
    Ok(EqualizerState {
        preset,
        clear_bass: equalizer_value(code, clear_bass)?,
        bands: [
            equalizer_value(code, b1)?,
            equalizer_value(code, b2)?,
            equalizer_value(code, b3)?,
            equalizer_value(code, b4)?,
            equalizer_value(code, b5)?,
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting(mode: AmbientMode, level: u8, focus_on_voice: bool) -> AmbientSetting {
        AmbientSetting {
            mode,
            level,
            focus_on_voice,
        }
    }

    #[test]
    fn queries_have_their_documented_payloads() {
        assert_eq!(Query::Ambient.payload(), [0x66, 0x02]);
        assert_eq!(Query::Battery.payload(), [0x10, 0x00]);
        assert_eq!(Query::Equalizer.payload(), [0x56, 0x01]);
    }

    #[test]
    fn a_query_is_found_by_its_name() {
        for query in Query::ALL {
            assert_eq!(Query::from_name(query.name()), Some(query));
        }
        assert_eq!(Query::from_name("format-disk"), None);
    }

    #[test]
    fn ambient_commands_follow_the_documented_layouts() {
        assert_eq!(
            setting(AmbientMode::Ambient, 7, true)
                .payload(true)
                .unwrap(),
            [0x68, 0x02, 0x11, 0x02, 0x00, 0x01, 0x01, 0x07]
        );
        assert_eq!(
            setting(AmbientMode::NoiseCancelling, 7, false)
                .payload(true)
                .unwrap(),
            [0x68, 0x02, 0x11, 0x02, 0x02, 0x01, 0x00, 0x00]
        );
        assert_eq!(
            setting(AmbientMode::NoiseCancelling, 7, false)
                .payload(false)
                .unwrap(),
            [0x68, 0x02, 0x11, 0x00, 0x01, 0x01, 0x00, 0x00]
        );
        assert_eq!(
            setting(AmbientMode::Off, 3, false).payload(true).unwrap(),
            [0x68, 0x02, 0x00, 0x02, 0x00, 0x01, 0x00, 0x03]
        );
        assert_eq!(
            setting(AmbientMode::WindReduction, 0, true)
                .payload(true)
                .unwrap(),
            [0x68, 0x02, 0x11, 0x02, 0x01, 0x01, 0x01, 0x00]
        );
    }

    #[test]
    fn a_setting_the_headphones_would_refuse_is_not_built() {
        assert_eq!(
            setting(AmbientMode::Ambient, 21, false).payload(true),
            Err(SettingError::Level(21))
        );
        assert_eq!(
            setting(AmbientMode::WindReduction, 0, false).payload(false),
            Err(SettingError::NoWindReduction)
        );
    }

    #[test]
    fn an_ambient_answer_is_read() {
        let state = decode(&[0x67, 0x02, 0x01, 0x00, 0x00, 0x01, 0x01, 0x0A]).unwrap();
        assert_eq!(
            state,
            Event::Ambient(AmbientState {
                mode: AmbientMode::Ambient,
                wind_capable: false,
                focus_on_voice: true,
                level: 10,
            })
        );
    }

    #[test]
    fn the_mode_byte_depends_on_the_capability() {
        let nc = |payload: [u8; 8]| match decode(&payload).unwrap() {
            Event::Ambient(state) => state.mode,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            nc([0x69, 0x02, 0x01, 0x00, 0x01, 0x01, 0x00, 0x00]),
            AmbientMode::NoiseCancelling
        );
        assert_eq!(
            nc([0x69, 0x02, 0x01, 0x02, 0x02, 0x01, 0x00, 0x00]),
            AmbientMode::NoiseCancelling
        );
        assert_eq!(
            nc([0x69, 0x02, 0x01, 0x02, 0x01, 0x01, 0x00, 0x00]),
            AmbientMode::WindReduction
        );
        assert_eq!(
            nc([0x69, 0x02, 0x00, 0x02, 0x02, 0x01, 0x00, 0x05]),
            AmbientMode::Off
        );
    }

    #[test]
    fn impossible_ambient_answers_are_refused() {
        for payload in [
            vec![0x67, 0x02, 0x01],
            vec![0x67, 0x02, 0x01, 0x00, 0x00, 0x01, 0x00, 0x15],
            vec![0x67, 0x02, 0x01, 0x07, 0x00, 0x01, 0x00, 0x01],
            vec![0x67, 0x02, 0x01, 0x00, 0x05, 0x01, 0x00, 0x01],
            vec![0x67, 0x09, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01],
        ] {
            assert!(decode(&payload).is_err(), "{payload:?}");
        }
    }

    #[test]
    fn a_battery_answer_is_read() {
        assert_eq!(
            decode(&[0x11, 0x00, 0x50, 0x01]).unwrap(),
            Event::Battery(Battery::Single(BatteryCell {
                level: 80,
                charging: true
            }))
        );
        assert_eq!(
            decode(&[0x13, 0x01, 0x10, 0x00, 0x20, 0x01]).unwrap(),
            Event::Battery(Battery::LeftRight {
                left: BatteryCell {
                    level: 16,
                    charging: false
                },
                right: BatteryCell {
                    level: 32,
                    charging: true
                },
            })
        );
        assert!(decode(&[0x11, 0x00, 0x65, 0x00]).is_err());
        assert!(decode(&[0x11, 0x00]).is_err());
    }

    #[test]
    fn an_equalizer_answer_is_read_with_its_offset() {
        assert_eq!(
            decode(&[0x57, 0x01, 0xA0, 0x06, 0x0A, 0x0F, 0x05, 0x0A, 0x14, 0x00]).unwrap(),
            Event::Equalizer(EqualizerState {
                preset: 0xA0,
                clear_bass: 0,
                bands: [5, -5, 0, 10, -10],
            })
        );
        assert!(decode(&[0x57, 0x01, 0xA0, 0x06, 0x0A, 0x0F, 0x05, 0x0A, 0x15, 0x00]).is_err());
        assert!(decode(&[0x57, 0x01]).is_err());
    }

    #[test]
    fn what_is_not_known_is_kept_as_it_came() {
        assert_eq!(
            decode(&[0x25, 0x01]).unwrap(),
            Event::Other(vec![0x25, 0x01])
        );
        assert_eq!(decode(&[]).unwrap(), Event::Other(Vec::new()));
    }
}
