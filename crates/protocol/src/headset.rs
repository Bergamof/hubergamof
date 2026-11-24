//! Sony headphones: what the interface shows of them and what it can ask.
//!
//! The headphones keep their own settings: nothing here is persisted. What the
//! interface asks goes to the headphones as closed values only, checked by the
//! `validate` methods below before anything is sent.

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

/// Highest ambient sound level.
pub const MAX_AMBIENT_LEVEL: u8 = 20;

/// Lowest and highest value of an equalizer band or of the clear bass.
pub const EQUALIZER_RANGE: RangeInclusive<i8> = -10..=10;

/// Number of bands of the equalizer.
pub const EQUALIZER_BANDS: usize = 5;

/// Longest name kept for headphones, which they choose themselves.
pub const MAX_HEADSET_NAME_CHARS: usize = 64;

/// Headphones the manager can talk to: paired, connected to this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HeadsetInfo {
    /// Bluetooth address, upper case: `AA:BB:CC:DD:EE:FF`. Identifies the
    /// headphones in every message.
    pub address: String,
    /// The name to show, as the headphones call themselves.
    pub name: String,
}

/// How the manager is linked to the settings channel of the headphones.
///
/// The headphones serve one settings client at a time: the Sony app on a phone
/// and the manager take turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum HeadsetLink {
    /// Not linked: the link is opened when the person selects the headphones.
    Idle,
    /// Being opened.
    Connecting,
    /// Linked: what the headphones say below is up to date.
    Connected,
    /// Closed by the person, to leave the channel to another app.
    Released,
    /// Closed by the headphones (another app took the channel, or they went
    /// away): not retried, the person resumes it.
    Cut,
}

/// What noise cancelling and ambient sound do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AmbientMode {
    /// Neither: the headphones play as they are.
    Off,
    /// Noise is cancelled.
    NoiseCancelling,
    /// Outside sound is let in.
    Ambient,
    /// Wind noise is reduced (only on headphones that offer it, see
    /// [`HeadsetState::wind_reduction`]).
    WindReduction,
}

/// The ambient sound setting, as shown and as asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AmbientSound {
    /// What to do.
    pub mode: AmbientMode,
    /// How much outside sound, from 0 to [`MAX_AMBIENT_LEVEL`] (kept by the
    /// headphones in the other modes).
    pub level: u8,
    /// Whether voices are let through.
    pub focus_on_voice: bool,
}

/// An equalizer preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EqualizerPreset {
    /// No equalizer.
    Off,
    /// Bright.
    Bright,
    /// Excited.
    Excited,
    /// Mellow.
    Mellow,
    /// Relaxed.
    Relaxed,
    /// Vocal.
    Vocal,
    /// Treble boost.
    TrebleBoost,
    /// Bass boost.
    BassBoost,
    /// Speech.
    Speech,
    /// The bands set by hand: shown, not chosen (see [`EqualizerChange::Manual`]).
    Manual,
    /// First preset of the person.
    Custom1,
    /// Second preset of the person.
    Custom2,
    /// A preset this version does not know: shown, never asked for.
    Other,
}

impl EqualizerPreset {
    /// The presets a person can choose: [`Self::Manual`] is reached by setting
    /// the bands, [`Self::Other`] is not one.
    pub const CHOOSABLE: [Self; 11] = [
        Self::Off,
        Self::Bright,
        Self::Excited,
        Self::Mellow,
        Self::Relaxed,
        Self::Vocal,
        Self::TrebleBoost,
        Self::BassBoost,
        Self::Speech,
        Self::Custom1,
        Self::Custom2,
    ];
}

/// The equalizer as the headphones report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Equalizer {
    /// The preset in use.
    pub preset: EqualizerPreset,
    /// Bass boost, from -10 to 10.
    pub clear_bass: i8,
    /// The five bands from low to high (400 Hz, 1, 2.5, 6.3 and 16 kHz), from
    /// -10 to 10.
    pub bands: [i8; EQUALIZER_BANDS],
}

/// A change of the equalizer the person asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum EqualizerChange {
    /// Choose a preset (one of [`EqualizerPreset::CHOOSABLE`]).
    Preset {
        /// The preset.
        preset: EqualizerPreset,
    },
    /// Set the clear bass and the bands by hand.
    Manual {
        /// Bass boost, from -10 to 10.
        clear_bass: i8,
        /// The five bands, from -10 to 10.
        bands: [i8; EQUALIZER_BANDS],
    },
}

/// The battery of the headphones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HeadsetBattery {
    /// Charge, from 0 to 100.
    pub percent: u8,
    /// Plugged in and charging.
    pub charging: bool,
}

/// Everything known of the headphones the client selected. What the
/// headphones said is absent until they have said it, and stays what it was
/// when the link closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HeadsetState {
    /// The link to the settings channel.
    pub link: HeadsetLink,
    /// Battery, once read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<HeadsetBattery>,
    /// Ambient sound, once read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient: Option<AmbientSound>,
    /// Whether these headphones offer [`AmbientMode::WindReduction`].
    pub wind_reduction: bool,
    /// Equalizer, once read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equalizer: Option<Equalizer>,
}

impl HeadsetState {
    /// Headphones nothing is known of, with their link in the given state.
    #[must_use]
    pub fn unknown(link: HeadsetLink) -> Self {
        Self {
            link,
            battery: None,
            ambient: None,
            wind_reduction: false,
            equalizer: None,
        }
    }
}

/// Why a request for the headphones is refused before it is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum HeadsetRequestError {
    /// The ambient level is above [`MAX_AMBIENT_LEVEL`].
    #[error("ambient level {0} is above {MAX_AMBIENT_LEVEL}")]
    AmbientLevel(u8),
    /// The wind noise reduction is asked of headphones that do not offer it.
    #[error("these headphones have no wind noise reduction")]
    NoWindReduction,
    /// An equalizer value is outside [`EQUALIZER_RANGE`].
    #[error("equalizer value {0} is outside -10 to 10")]
    EqualizerValue(i8),
    /// The preset is not one a person can choose.
    #[error("this equalizer preset cannot be chosen")]
    PresetNotChoosable,
}

impl AmbientSound {
    /// Checks the setting against what the headphones accept.
    ///
    /// # Errors
    /// [`HeadsetRequestError`] for a level that is too high, or the wind
    /// reduction when the headphones do not offer it (`wind_reduction`).
    pub fn validate(&self, wind_reduction: bool) -> Result<(), HeadsetRequestError> {
        if self.level > MAX_AMBIENT_LEVEL {
            return Err(HeadsetRequestError::AmbientLevel(self.level));
        }
        if self.mode == AmbientMode::WindReduction && !wind_reduction {
            return Err(HeadsetRequestError::NoWindReduction);
        }
        Ok(())
    }
}

impl EqualizerChange {
    /// Checks the change against what the headphones accept.
    ///
    /// # Errors
    /// [`HeadsetRequestError`] for a value out of range or a preset that is
    /// not choosable.
    pub fn validate(&self) -> Result<(), HeadsetRequestError> {
        match self {
            Self::Preset { preset } if EqualizerPreset::CHOOSABLE.contains(preset) => Ok(()),
            Self::Preset { .. } => Err(HeadsetRequestError::PresetNotChoosable),
            Self::Manual { clear_bass, bands } => std::iter::once(clear_bass)
                .chain(bands)
                .find(|value| !EQUALIZER_RANGE.contains(value))
                .map_or(Ok(()), |value| {
                    Err(HeadsetRequestError::EqualizerValue(*value))
                }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ambient(mode: AmbientMode, level: u8) -> AmbientSound {
        AmbientSound {
            mode,
            level,
            focus_on_voice: false,
        }
    }

    #[test]
    fn an_ambient_setting_within_range_is_accepted() {
        for mode in [
            AmbientMode::Off,
            AmbientMode::NoiseCancelling,
            AmbientMode::Ambient,
        ] {
            for level in [0, 10, MAX_AMBIENT_LEVEL] {
                assert_eq!(ambient(mode, level).validate(false), Ok(()));
            }
        }
    }

    #[test]
    fn an_ambient_level_too_high_is_refused() {
        assert_eq!(
            ambient(AmbientMode::Ambient, MAX_AMBIENT_LEVEL + 1).validate(true),
            Err(HeadsetRequestError::AmbientLevel(MAX_AMBIENT_LEVEL + 1))
        );
    }

    #[test]
    fn the_wind_reduction_needs_headphones_that_offer_it() {
        let wind = ambient(AmbientMode::WindReduction, 0);
        assert_eq!(wind.validate(true), Ok(()));
        assert_eq!(
            wind.validate(false),
            Err(HeadsetRequestError::NoWindReduction)
        );
    }

    #[test]
    fn every_choosable_preset_is_accepted_and_the_others_refused() {
        for preset in EqualizerPreset::CHOOSABLE {
            assert_eq!(EqualizerChange::Preset { preset }.validate(), Ok(()));
        }
        for preset in [EqualizerPreset::Manual, EqualizerPreset::Other] {
            assert_eq!(
                EqualizerChange::Preset { preset }.validate(),
                Err(HeadsetRequestError::PresetNotChoosable)
            );
        }
    }

    #[test]
    fn manual_bands_inside_the_range_are_accepted() {
        let change = EqualizerChange::Manual {
            clear_bass: -10,
            bands: [10, -10, 0, 3, -3],
        };
        assert_eq!(change.validate(), Ok(()));
    }

    #[test]
    fn a_manual_value_outside_the_range_is_refused_wherever_it_is() {
        for (clear_bass, bands, wrong) in [
            (11, [0; EQUALIZER_BANDS], 11),
            (-11, [0; EQUALIZER_BANDS], -11),
            (0, [0, 0, 0, 0, 12], 12),
            (0, [-12, 0, 0, 0, 0], -12),
        ] {
            assert_eq!(
                EqualizerChange::Manual { clear_bass, bands }.validate(),
                Err(HeadsetRequestError::EqualizerValue(wrong))
            );
        }
    }

    #[test]
    fn headphones_nothing_is_known_of_say_nothing() {
        let state = HeadsetState::unknown(HeadsetLink::Idle);
        assert_eq!(state.link, HeadsetLink::Idle);
        assert!(state.battery.is_none() && state.ambient.is_none() && state.equalizer.is_none());
        assert!(!state.wind_reduction);
    }
}
