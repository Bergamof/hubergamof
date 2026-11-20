//! Operations on one mouse behind a [`Hidpp`]: what the manager reads from it
//! and writes to it. This is the only part of the crate that sends anything
//! but getters.
//!
//! Function numbers and byte layouts follow what Solaar
//! (`lib/logitech_receiver`) does with these features and have not been tried
//! on the mice yet (`docs/mouse-hidpp.md`). A link only drives the models this
//! project knows, buttons are named by control of the model (never by a raw
//! control id), every argument is checked before anything is sent, and the
//! writes that share a byte with other settings keep the bits they do not own.

use std::collections::BTreeSet;

use hubergamof_protocol::{
    DPI_RANGE, DPI_STEP, HapticLevel, HapticWaveform, MouseControlId, MouseModel,
    WHEEL_THRESHOLD_RANGE, WheelMode,
};

use crate::channel::{Channel, Hidpp, HidppError};
use crate::controls::{cid, control_of, identify, is_locked_cid};
use crate::features::{
    ADJUSTABLE_DPI, DEVICE_FIRMWARE, DEVICE_NAME, HAPTIC, HIRES_WHEEL, REPROG_CONTROLS_V4,
    SMART_SHIFT_ENHANCED, THUMB_WHEEL, UNIFIED_BATTERY,
};
use crate::frame::Notification;
use crate::probe::read_name;

/// How many control ids a diverted-button notification carries.
const DIVERTED_SLOTS: usize = 4;
/// Wheel mode byte of free spin.
const WHEEL_FREE: u8 = 1;
/// Wheel mode byte of the ratchet.
const WHEEL_RATCHET: u8 = 2;
/// Auto-disengage byte meaning "never leave the ratchet".
const NEVER_FREE: u8 = 255;
/// Level the haptic feature is parked at when switched off (what Solaar writes).
const HAPTIC_PARKED_LEVEL: u8 = 50;
/// Highest haptic level.
const HAPTIC_MAX_LEVEL: u8 = 100;
/// Bits of the high resolution wheel byte that are ours: resolution and invert
/// (bit 0, "diverted", belongs to whoever diverts the wheel).
const HIRES_OWNED: u8 = 0x06;
/// Bit of the second thumb wheel byte that is ours: invert.
const THUMB_INVERT_BIT: u8 = 0x01;

/// Unit id of a device: four bytes that tell two mice of one model apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnitId(pub [u8; 4]);

impl std::fmt::Display for UnitId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

/// How full the battery is, in the device's own steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryLevel {
    /// About to stop.
    Critical,
    /// Low.
    Low,
    /// Good.
    Good,
    /// Full.
    Full,
    /// Anything else the device says.
    Empty,
}

/// Battery state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    /// Charge in percent, when the device says it.
    pub percent: Option<u8>,
    /// Coarse level.
    pub level: BatteryLevel,
    /// Charging status as the device codes it.
    pub status: u8,
}

/// Haptic feedback state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HapticState {
    /// Feedback is on.
    pub enabled: bool,
    /// Strength, 0 to 100.
    pub level: u8,
}

/// The wheel as the device reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WheelState {
    /// Mode byte: free spin or ratchet.
    pub mode: u8,
    /// Speed above which a ratcheted wheel frees itself; `255` never does.
    pub auto_disengage: u8,
    /// Torque of the ratchet.
    pub torque: u8,
}

/// Indexes of the features a mouse offers, `None` for those it lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Features {
    firmware: Option<u8>,
    battery: Option<u8>,
    reprog: Option<u8>,
    dpi: Option<u8>,
    wheel: Option<u8>,
    hires: Option<u8>,
    thumb: Option<u8>,
    haptic: Option<u8>,
}

/// A mouse reached through a channel.
#[derive(Debug)]
pub struct MouseLink<C> {
    hidpp: Hidpp<C>,
    device: u8,
    model: MouseModel,
    features: Features,
    haptic_waveforms: Option<u32>,
}

/// Id of a waveform in the haptic feature; `None` for one this crate does not
/// know yet.
#[must_use]
pub fn waveform_id(waveform: HapticWaveform) -> Option<u8> {
    Some(match waveform {
        HapticWaveform::SharpStateChange => 0x00,
        HapticWaveform::DampStateChange => 0x01,
        HapticWaveform::SharpCollision => 0x02,
        HapticWaveform::DampCollision => 0x03,
        HapticWaveform::SubtleCollision => 0x04,
        HapticWaveform::HappyAlert => 0x05,
        HapticWaveform::AngryAlert => 0x06,
        HapticWaveform::Completed => 0x07,
        HapticWaveform::Square => 0x08,
        HapticWaveform::Wave => 0x09,
        HapticWaveform::Firework => 0x0A,
        HapticWaveform::Mad => 0x0B,
        HapticWaveform::Knock => 0x0C,
        HapticWaveform::Jingle => 0x0D,
        HapticWaveform::Ringing => 0x0E,
        HapticWaveform::WhisperCollision => 0x1B,
        _ => return None,
    })
}

/// The haptic strength, 0 to 100, a level means; `None` when it is off.
#[must_use]
pub fn haptic_strength(level: HapticLevel) -> Option<u8> {
    match level {
        HapticLevel::Off => None,
        HapticLevel::Subtle => Some(25),
        HapticLevel::Low => Some(50),
        HapticLevel::Medium => Some(75),
        HapticLevel::High => Some(HAPTIC_MAX_LEVEL),
    }
}

/// A press or a release of a diverted button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonEvent {
    /// The control.
    pub control: MouseControlId,
    /// `true` when pressed, `false` when released.
    pub pressed: bool,
}

/// Turns the held-down sets of successive notifications into presses and
/// releases. Start it again ([`ButtonTracker::default`]) after anything that
/// may have lost notifications: a reconnection, a channel error, a growing
/// [`Hidpp::dropped_frames`].
#[derive(Debug, Default)]
pub struct ButtonTracker {
    down: BTreeSet<MouseControlId>,
}

impl ButtonTracker {
    /// Events that bring the previous state to `held`: releases first, each
    /// group in the order of the controls.
    pub fn update(&mut self, held: &[MouseControlId]) -> Vec<ButtonEvent> {
        let now: BTreeSet<MouseControlId> = held.iter().copied().collect();
        let released = self.down.difference(&now).map(|control| ButtonEvent {
            control: *control,
            pressed: false,
        });
        let pressed = now.difference(&self.down).map(|control| ButtonEvent {
            control: *control,
            pressed: true,
        });
        let events = released.chain(pressed).collect();
        self.down = now;
        events
    }

    /// Releases everything held, for a mouse that went away.
    pub fn release_all(&mut self) -> Vec<ButtonEvent> {
        self.update(&[])
    }
}

impl<C: Channel> MouseLink<C> {
    /// Looks up the features of the device at `device` and checks that it is
    /// one of the supported mice. A feature the device answers an error for
    /// counts as absent.
    ///
    /// # Errors
    /// [`HidppError::NotSupportedMouse`] for any other device, other
    /// [`HidppError`] when the device does not answer the lookups.
    pub fn connect(mut hidpp: Hidpp<C>, device: u8) -> Result<Self, HidppError> {
        let mut lookup = |id: u16| -> Result<Option<u8>, HidppError> {
            let [hi, lo] = id.to_be_bytes();
            match hidpp.request(device, 0, 0, &[hi, lo]) {
                Ok(params) => Ok(params.first().copied().filter(|index| *index != 0)),
                Err(HidppError::Device(_)) => Ok(None),
                Err(other) => Err(other),
            }
        };
        let name_index = lookup(DEVICE_NAME)?;
        let features = Features {
            firmware: lookup(DEVICE_FIRMWARE)?,
            battery: lookup(UNIFIED_BATTERY)?,
            reprog: lookup(REPROG_CONTROLS_V4)?,
            dpi: lookup(ADJUSTABLE_DPI)?,
            wheel: lookup(SMART_SHIFT_ENHANCED)?,
            hires: lookup(HIRES_WHEEL)?,
            thumb: lookup(THUMB_WHEEL)?,
            haptic: lookup(HAPTIC)?,
        };
        let name_index = name_index.ok_or(HidppError::Unsupported("device name"))?;
        let name = read_name(&mut hidpp, device, name_index)?;
        let model = identify(&name).ok_or(HidppError::NotSupportedMouse(name))?;
        let mut link = Self {
            hidpp,
            device,
            model,
            features,
            haptic_waveforms: None,
        };
        if features.haptic.is_some() {
            link.haptic_waveforms = link.read_haptic_waveforms().ok();
        }
        Ok(link)
    }

    /// The device index this link talks to.
    #[must_use]
    pub fn device(&self) -> u8 {
        self.device
    }

    /// The model of the mouse.
    #[must_use]
    pub fn model(&self) -> MouseModel {
        self.model
    }

    /// Whether the device has haptic feedback.
    #[must_use]
    pub fn has_haptics(&self) -> bool {
        self.features.haptic.is_some()
    }

    /// Whether the device has a thumb wheel.
    #[must_use]
    pub fn has_thumb_wheel(&self) -> bool {
        self.features.thumb.is_some()
    }

    /// The channel, to wait for notifications.
    pub fn hidpp_mut(&mut self) -> &mut Hidpp<C> {
        &mut self.hidpp
    }

    /// The underlying channel, for tests to look at what was written.
    #[cfg(test)]
    pub(crate) fn hidpp_channel(&self) -> &C {
        self.hidpp.channel()
    }

    /// The controls a notification says are held down, when it is a
    /// diverted-button notification **of this mouse** (a receiver carries
    /// several, and their feature indexes are alike). Control ids this model
    /// does not know are left out.
    #[must_use]
    pub fn held_buttons(&self, notification: &Notification) -> Option<Vec<MouseControlId>> {
        let reprog = self.features.reprog?;
        if notification.device != self.device
            || notification.feature_index != reprog
            || notification.event != 0
        {
            return None;
        }
        let held = notification
            .params
            .as_chunks::<2>()
            .0
            .iter()
            .take(DIVERTED_SLOTS)
            .map(|pair| u16::from_be_bytes(*pair))
            .filter_map(|cid_value| control_of(self.model, cid_value))
            .collect();
        Some(held)
    }

    fn call(
        &mut self,
        feature: Option<u8>,
        name: &'static str,
        function: u8,
        params: &[u8],
    ) -> Result<Vec<u8>, HidppError> {
        let index = feature.ok_or(HidppError::Unsupported(name))?;
        self.hidpp.request(self.device, index, function, params)
    }

    /// The unit id of the device.
    ///
    /// # Errors
    /// [`HidppError`] when the device lacks the feature or does not answer.
    pub fn unit_id(&mut self) -> Result<UnitId, HidppError> {
        let params = self.call(self.features.firmware, "firmware information", 0, &[])?;
        match params[..] {
            [_, a, b, c, d, ..] => Ok(UnitId([a, b, c, d])),
            _ => Err(HidppError::Malformed("short unit id".to_owned())),
        }
    }

    /// The battery state.
    ///
    /// # Errors
    /// [`HidppError`] when the device lacks the feature or does not answer.
    pub fn battery(&mut self) -> Result<Battery, HidppError> {
        let params = self.call(self.features.battery, "battery", 1, &[])?;
        let [percent, level, status, ..] = params[..] else {
            return Err(HidppError::Malformed("short battery state".to_owned()));
        };
        Ok(Battery {
            percent: (percent != 0).then_some(percent),
            level: match level {
                8 => BatteryLevel::Full,
                4 => BatteryLevel::Good,
                2 => BatteryLevel::Low,
                1 => BatteryLevel::Critical,
                _ => BatteryLevel::Empty,
            },
            status,
        })
    }

    /// The sensitivity, in dots per inch.
    ///
    /// # Errors
    /// [`HidppError`] when the device lacks the feature or does not answer.
    pub fn dpi(&mut self) -> Result<u16, HidppError> {
        let params = self.call(self.features.dpi, "adjustable sensitivity", 2, &[0])?;
        let [_, current_hi, current_lo, default_hi, default_lo, ..] = params[..] else {
            return Err(HidppError::Malformed("short sensitivity".to_owned()));
        };
        let current = u16::from_be_bytes([current_hi, current_lo]);
        Ok(if current == 0 {
            u16::from_be_bytes([default_hi, default_lo])
        } else {
            current
        })
    }

    /// Sets the sensitivity. The range is the contract's, not one the mouse
    /// announced: a mouse that refuses a value says so.
    ///
    /// # Errors
    /// [`HidppError::InvalidArgument`] for a sensitivity the mice do not offer,
    /// other [`HidppError`] when the device lacks the feature or does not answer.
    pub fn set_dpi(&mut self, dpi: u16) -> Result<(), HidppError> {
        if !DPI_RANGE.contains(&dpi) || !(dpi - DPI_RANGE.start()).is_multiple_of(DPI_STEP) {
            return Err(HidppError::InvalidArgument(format!("{dpi} dpi")));
        }
        let [hi, lo] = dpi.to_be_bytes();
        self.call(self.features.dpi, "adjustable sensitivity", 3, &[0, hi, lo])
            .map(|_| ())
    }

    /// The wheel mode as the device reports it.
    ///
    /// # Errors
    /// [`HidppError`] when the device lacks the feature or does not answer.
    pub fn wheel(&mut self) -> Result<WheelState, HidppError> {
        let params = self.call(self.features.wheel, "wheel mode", 1, &[])?;
        let [mode, auto_disengage, torque, ..] = params[..] else {
            return Err(HidppError::Malformed("short wheel state".to_owned()));
        };
        Ok(WheelState {
            mode,
            auto_disengage,
            torque,
        })
    }

    /// Sets how the wheel turns; the torque is written back as the device has it.
    ///
    /// # Errors
    /// [`HidppError::InvalidArgument`] for a threshold out of range, other
    /// [`HidppError`] when the device lacks the feature or does not answer.
    pub fn set_wheel(&mut self, mode: WheelMode, threshold: u8) -> Result<(), HidppError> {
        if !WHEEL_THRESHOLD_RANGE.contains(&threshold) {
            return Err(HidppError::InvalidArgument(format!(
                "wheel threshold {threshold}"
            )));
        }
        let current = self.wheel()?;
        // A zero in the first two bytes means "leave it as it is".
        let (mode_byte, auto_disengage) = match mode {
            WheelMode::Ratchet => (WHEEL_RATCHET, NEVER_FREE),
            WheelMode::Free => (WHEEL_FREE, 0),
            WheelMode::Auto => (WHEEL_RATCHET, threshold),
        };
        self.call(
            self.features.wheel,
            "wheel mode",
            2,
            &[mode_byte, auto_disengage, current.torque],
        )
        .map(|_| ())
    }

    /// Sets high resolution and natural scrolling (`invert`), leaving every
    /// other bit of the byte as the device has it.
    ///
    /// # Errors
    /// [`HidppError`] when the device lacks the feature, does not answer or
    /// answers nothing to the read.
    pub fn set_hires_wheel(&mut self, high_res: bool, invert: bool) -> Result<(), HidppError> {
        let params = self.call(self.features.hires, "high resolution wheel", 1, &[])?;
        let current = *params
            .first()
            .ok_or_else(|| HidppError::Malformed("empty wheel flags".to_owned()))?;
        let owned = if high_res { 0x02 } else { 0 } | if invert { 0x04 } else { 0 };
        let flags = (current & !HIRES_OWNED) | owned;
        self.call(self.features.hires, "high resolution wheel", 2, &[flags])
            .map(|_| ())
    }

    /// Reverses the thumb wheel, leaving every other bit as the device has it.
    ///
    /// # Errors
    /// [`HidppError`] when the device lacks the feature, does not answer or
    /// answers too little to the read.
    pub fn set_thumb_invert(&mut self, invert: bool) -> Result<(), HidppError> {
        let params = self.call(self.features.thumb, "thumb wheel", 1, &[])?;
        let [first, second, ..] = params[..] else {
            return Err(HidppError::Malformed("short thumb wheel flags".to_owned()));
        };
        let second = (second & !THUMB_INVERT_BIT) | if invert { THUMB_INVERT_BIT } else { 0 };
        self.call(self.features.thumb, "thumb wheel", 2, &[first, second])
            .map(|_| ())
    }

    /// The mouse's own id of a control, `None` for one that is not a button of
    /// this model.
    fn button_cid(&self, control: MouseControlId) -> Result<u16, HidppError> {
        cid(self.model, control).ok_or_else(|| {
            HidppError::InvalidArgument(format!("{control} is not a button of {}", self.model))
        })
    }

    /// Whether a button is diverted to us.
    ///
    /// # Errors
    /// [`HidppError::InvalidArgument`] for a control that is not a button of the
    /// model, [`HidppError::Malformed`] when the answer is about another
    /// button, other [`HidppError`] when the device lacks the feature or does
    /// not answer.
    pub fn is_diverted(&mut self, control: MouseControlId) -> Result<bool, HidppError> {
        let wanted = self.button_cid(control)?;
        let [hi, lo] = wanted.to_be_bytes();
        let params = self.call(self.features.reprog, "remappable buttons", 2, &[hi, lo])?;
        let [answered_hi, answered_lo, flags, ..] = params[..] else {
            return Err(HidppError::Malformed("short button reporting".to_owned()));
        };
        if u16::from_be_bytes([answered_hi, answered_lo]) != wanted {
            return Err(HidppError::Malformed(
                "answer about another button".to_owned(),
            ));
        }
        Ok(flags & 0x01 != 0)
    }

    /// Diverts a button to us, or gives it back. Left and right click can be
    /// given back (another program may have taken them) but are never diverted.
    /// The diversion is temporary: the mouse forgets it when it reconnects, so
    /// it must be applied again then.
    ///
    /// # Errors
    /// [`HidppError::InvalidArgument`] for a control that is not a button of the
    /// model or a locked one being diverted, other [`HidppError`] when the
    /// device lacks the feature or does not answer.
    pub fn set_diverted(
        &mut self,
        control: MouseControlId,
        diverted: bool,
    ) -> Result<(), HidppError> {
        let wanted = self.button_cid(control)?;
        if diverted && is_locked_cid(wanted) {
            return Err(HidppError::InvalidArgument(format!(
                "the {control} is never diverted"
            )));
        }
        let [hi, lo] = wanted.to_be_bytes();
        // Bit 0 says "divert", bit 1 says that bit 0 is meant; the persistence
        // bit is never set.
        let flags = 0x02 | u8::from(diverted);
        self.call(
            self.features.reprog,
            "remappable buttons",
            3,
            &[hi, lo, flags, 0, 0],
        )
        .map(|_| ())
    }

    /// Gives back every button of the model that is diverted: for the start of
    /// the manager (after a crash that left some diverted), its end, and a
    /// mouse that comes back.
    ///
    /// # Errors
    /// The first [`HidppError`] met, after trying every button.
    pub fn release_buttons(&mut self) -> Result<(), HidppError> {
        let controls: Vec<MouseControlId> = self
            .model
            .capabilities()
            .controls
            .iter()
            .map(|control| control.id)
            .filter(|id| cid(self.model, *id).is_some())
            .collect();
        // Best effort: one control that answers badly must not leave the
        // following ones diverted, this being the recovery path.
        let mut first_error = None;
        for control in controls {
            let given_back = match self.is_diverted(control) {
                Ok(true) => self.set_diverted(control, false),
                Ok(false) => Ok(()),
                Err(error) => Err(error),
            };
            if let Err(error) = given_back {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn read_haptic_waveforms(&mut self) -> Result<u32, HidppError> {
        let params = self.call(self.features.haptic, "haptic feedback", 0, &[])?;
        match params[..] {
            [_, _, _, _, a, b, c, d, ..] => Ok(u32::from_be_bytes([a, b, c, d])),
            _ => Err(HidppError::Malformed(
                "short haptic capabilities".to_owned(),
            )),
        }
    }

    /// Whether the device plays this waveform.
    #[must_use]
    pub fn supports_waveform(&self, waveform: HapticWaveform) -> bool {
        match (waveform_id(waveform), self.haptic_waveforms) {
            (Some(id), Some(mask)) => mask
                .checked_shr(u32::from(id))
                .is_some_and(|bit| bit & 1 == 1),
            _ => false,
        }
    }

    /// The haptic feedback state.
    ///
    /// # Errors
    /// [`HidppError`] when the device lacks the feature or does not answer.
    pub fn haptic_state(&mut self) -> Result<HapticState, HidppError> {
        let params = self.call(self.features.haptic, "haptic feedback", 1, &[])?;
        let [enabled, level, ..] = params[..] else {
            return Err(HidppError::Malformed("short haptic state".to_owned()));
        };
        Ok(HapticState {
            enabled: enabled & 0x01 != 0,
            level,
        })
    }

    /// Switches haptic feedback on at a strength (0 to 100), or off.
    ///
    /// # Errors
    /// [`HidppError::InvalidArgument`] for a strength above 100, other
    /// [`HidppError`] when the device lacks the feature or does not answer.
    pub fn set_haptic(&mut self, strength: Option<u8>) -> Result<(), HidppError> {
        let payload = match strength {
            Some(level) if level > HAPTIC_MAX_LEVEL => {
                return Err(HidppError::InvalidArgument(format!(
                    "haptic strength {level}"
                )));
            }
            Some(level) => [1, level],
            None => [0, HAPTIC_PARKED_LEVEL],
        };
        self.call(self.features.haptic, "haptic feedback", 2, &payload)
            .map(|_| ())
    }

    /// Plays a waveform now.
    ///
    /// # Errors
    /// [`HidppError::InvalidArgument`] for a waveform the device does not play,
    /// other [`HidppError`] when the device lacks the feature or does not answer.
    pub fn play_haptic(&mut self, waveform: HapticWaveform) -> Result<(), HidppError> {
        let id = waveform_id(waveform)
            .filter(|_| self.supports_waveform(waveform))
            .ok_or_else(|| HidppError::InvalidArgument(format!("waveform {waveform:?}")))?;
        self.call(self.features.haptic, "haptic feedback", 4, &[id])
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::features::{
        ADJUSTABLE_DPI, HAPTIC, HIRES_WHEEL, REPROG_CONTROLS_V4, SMART_SHIFT_ENHANCED, THUMB_WHEEL,
    };
    use crate::frame::{DeviceError, LONG_REPORT_ID, SOFTWARE_ID};
    use crate::testing::{FakeDevice, FakeLink, FakeState, ScriptedChannel, frame};

    const DEVICE: u8 = 0xFF;
    const MASTER: MouseModel = MouseModel::MxMaster4;

    fn link(device: FakeDevice) -> MouseLink<FakeLink> {
        MouseLink::connect(Hidpp::new(FakeLink::direct(device)), DEVICE).unwrap()
    }

    fn state(link: &MouseLink<FakeLink>) -> &FakeState {
        &link.hidpp.channel().device_at(DEVICE).unwrap().state
    }

    fn writes(link: &MouseLink<FakeLink>) -> usize {
        link.hidpp.channel().written().len()
    }

    fn last_write(link: &MouseLink<FakeLink>) -> Vec<u8> {
        link.hidpp.channel().written().last().unwrap().clone()
    }

    /// The report written `back` writes ago (0 is the last).
    fn write_before(link: &MouseLink<FakeLink>, back: usize) -> Vec<u8> {
        let written = link.hidpp.channel().written();
        written[written.len() - 1 - back].clone()
    }

    fn function_byte(function: u8) -> u8 {
        function << 4 | SOFTWARE_ID
    }

    /// A link whose features sit at fixed indexes and whose channel replays
    /// raw replies: for answers a well-behaved fake never gives.
    fn scripted(channel: ScriptedChannel) -> MouseLink<ScriptedChannel> {
        let hidpp = Hidpp::new(channel).with_timeout(Duration::from_millis(5));
        let features = Features {
            firmware: Some(2),
            battery: Some(3),
            reprog: Some(4),
            dpi: Some(5),
            wheel: Some(6),
            hires: Some(7),
            thumb: Some(8),
            haptic: Some(9),
        };
        MouseLink {
            hidpp,
            device: DEVICE,
            model: MASTER,
            features,
            haptic_waveforms: Some(u32::MAX),
        }
    }

    /// An answer shorter than the long report: only `params` after the header.
    fn short(feature: u8, function: u8, params: &[u8]) -> Vec<u8> {
        let mut bytes = vec![LONG_REPORT_ID, DEVICE, feature, function_byte(function)];
        bytes.extend_from_slice(params);
        bytes
    }

    // Connection.

    #[test]
    fn the_model_and_the_features_of_each_mouse_are_found() {
        let master = link(FakeDevice::mx_master_4());
        assert_eq!(master.model(), MASTER);
        assert!(master.has_haptics() && master.has_thumb_wheel());
        let anywhere = link(FakeDevice::mx_anywhere_3s());
        assert_eq!(anywhere.model(), MouseModel::MxAnywhere3S);
        assert!(!anywhere.has_haptics() && !anywhere.has_thumb_wheel());
        assert_eq!(master.device(), DEVICE);
    }

    #[test]
    fn a_device_that_is_not_a_supported_mouse_is_refused() {
        let keyboard = FakeDevice::new().with_name("MX Keys");
        let result = MouseLink::connect(Hidpp::new(FakeLink::direct(keyboard)), DEVICE);
        assert!(matches!(result, Err(HidppError::NotSupportedMouse(name)) if name == "MX Keys"));
        let nameless = FakeDevice::new();
        let result = MouseLink::connect(Hidpp::new(FakeLink::direct(nameless)), DEVICE);
        assert!(matches!(
            result,
            Err(HidppError::Unsupported("device name"))
        ));
    }

    #[test]
    fn a_silent_device_cannot_be_linked() {
        let result = MouseLink::connect(
            Hidpp::new(FakeLink::silent()).with_timeout(Duration::from_millis(1)),
            DEVICE,
        );
        assert!(matches!(result, Err(HidppError::Timeout(_))));
    }

    #[test]
    fn a_feature_the_device_answers_an_error_for_counts_as_absent() {
        let device = FakeDevice::mx_master_4().with_lookup_error(HAPTIC);
        let link = link(device);
        assert!(!link.has_haptics());
        assert_eq!(link.model(), MASTER);
    }

    #[test]
    fn haptics_without_a_readable_capability_list_play_nothing() {
        let device = FakeDevice::mx_master_4().with_broken_feature(HAPTIC);
        let mut link = link(device);
        assert!(link.has_haptics());
        assert!(!link.supports_waveform(HapticWaveform::Knock));
        assert!(matches!(
            link.play_haptic(HapticWaveform::Knock),
            Err(HidppError::InvalidArgument(_))
        ));
    }

    // Reads.

    #[test]
    fn the_unit_id_tells_mice_apart() {
        let mut link = link(FakeDevice::mx_master_4());
        let id = link.unit_id().unwrap();
        assert_eq!(id, UnitId([0xDE, 0xAD, 0xBE, 0xEF]));
        assert_eq!(id.to_string(), "deadbeef");
    }

    #[test]
    fn the_battery_is_read_in_percent_and_level() {
        let mut device = FakeDevice::mx_master_4();
        assert_eq!(
            link(device.clone()).battery().unwrap(),
            Battery {
                percent: Some(82),
                level: BatteryLevel::Good,
                status: 0
            }
        );
        for (code, expected) in [
            (8, BatteryLevel::Full),
            (4, BatteryLevel::Good),
            (2, BatteryLevel::Low),
            (1, BatteryLevel::Critical),
            (0, BatteryLevel::Empty),
            (3, BatteryLevel::Empty),
        ] {
            device.state.battery = [0, code, 1];
            let battery = link(device.clone()).battery().unwrap();
            assert_eq!(
                (battery.percent, battery.level, battery.status),
                (None, expected, 1),
                "code {code}"
            );
        }
    }

    #[test]
    fn short_answers_to_the_getters_are_malformed_not_guessed() {
        type Getter = Box<dyn Fn(&mut MouseLink<ScriptedChannel>) -> Result<(), HidppError>>;
        let cases: [(&str, Getter, Vec<u8>); 6] = [
            (
                "unit id",
                Box::new(|l| l.unit_id().map(|_| ())),
                short(2, 0, &[1, 2]),
            ),
            (
                "battery",
                Box::new(|l| l.battery().map(|_| ())),
                short(3, 1, &[80, 4]),
            ),
            (
                "sensitivity",
                Box::new(|l| l.dpi().map(|_| ())),
                short(5, 2, &[0, 6, 64]),
            ),
            (
                "wheel",
                Box::new(|l| l.wheel().map(|_| ())),
                short(6, 1, &[2, 10]),
            ),
            (
                "haptic state",
                Box::new(|l| l.haptic_state().map(|_| ())),
                short(9, 1, &[1]),
            ),
            (
                "haptic capabilities",
                Box::new(|l| l.read_haptic_waveforms().map(|_| ())),
                short(9, 0, &[0; 6]),
            ),
        ];
        for (what, call, reply) in cases {
            let mut link = scripted(ScriptedChannel::new().then_frame(reply));
            let result = call(&mut link);
            assert!(
                matches!(result, Err(HidppError::Malformed(_))),
                "{what}: {result:?}"
            );
        }
    }

    #[test]
    fn a_missing_feature_is_reported_not_guessed() {
        let mut link = link(FakeDevice::mx_anywhere_3s());
        assert_eq!(
            link.set_haptic(Some(50)),
            Err(HidppError::Unsupported("haptic feedback"))
        );
        assert_eq!(
            link.set_thumb_invert(true),
            Err(HidppError::Unsupported("thumb wheel"))
        );
        assert_eq!(
            link.haptic_state(),
            Err(HidppError::Unsupported("haptic feedback"))
        );
        assert!(!link.supports_waveform(HapticWaveform::Knock));
        assert_eq!(
            link.play_haptic(HapticWaveform::Knock),
            Err(HidppError::InvalidArgument("waveform Knock".to_owned()))
        );
    }

    // Sensitivity.

    #[test]
    fn the_sensitivity_is_read_and_set_with_the_documented_bytes() {
        let mut link = link(FakeDevice::mx_master_4());
        assert_eq!(link.dpi().unwrap(), 1600);
        link.set_dpi(2400).unwrap();
        let report = last_write(&link);
        assert_eq!(report[0], LONG_REPORT_ID);
        assert_eq!(report[1], DEVICE);
        assert_eq!(report[3], function_byte(3));
        assert_eq!(&report[4..7], &[0, 0x09, 0x60]);
        assert_eq!(state(&link).dpi, 2400);
        assert_eq!(link.dpi().unwrap(), 2400);
    }

    #[test]
    fn the_ends_of_the_sensitivity_range_are_sent_big_endian() {
        let mut link = link(FakeDevice::mx_master_4());
        link.set_dpi(200).unwrap();
        assert_eq!(&last_write(&link)[4..7], &[0, 0x00, 0xC8]);
        link.set_dpi(8000).unwrap();
        assert_eq!(&last_write(&link)[4..7], &[0, 0x1F, 0x40]);
    }

    #[test]
    fn an_unavailable_sensitivity_is_never_sent() {
        let mut link = link(FakeDevice::mx_master_4());
        let before = writes(&link);
        for dpi in [0, 199, 225, 8050, u16::MAX] {
            assert_eq!(
                link.set_dpi(dpi),
                Err(HidppError::InvalidArgument(format!("{dpi} dpi")))
            );
        }
        assert_eq!(writes(&link), before);
        assert_eq!(state(&link).dpi, 1600);
    }

    #[test]
    fn a_zero_sensitivity_reads_as_the_default() {
        let mut device = FakeDevice::mx_master_4();
        device.state.dpi = 0;
        assert_eq!(link(device).dpi().unwrap(), 1000);
    }

    // Wheel.

    #[test]
    fn the_wheel_modes_write_the_documented_bytes_and_keep_the_torque() {
        let mut link = link(FakeDevice::mx_master_4());
        assert_eq!(
            link.wheel().unwrap(),
            WheelState {
                mode: 2,
                auto_disengage: 10,
                torque: 50
            }
        );
        link.set_wheel(WheelMode::Auto, 30).unwrap();
        assert_eq!(&last_write(&link)[4..7], &[2, 30, 50]);
        assert_eq!(state(&link).wheel, [2, 30, 50]);
        link.set_wheel(WheelMode::Ratchet, 30).unwrap();
        assert_eq!(&last_write(&link)[4..7], &[2, 255, 50]);
        assert_eq!(state(&link).wheel, [2, 255, 50]);
        link.set_wheel(WheelMode::Free, 30).unwrap();
        // The threshold is left alone (zero), the mode is free spin.
        assert_eq!(&last_write(&link)[4..7], &[1, 0, 50]);
        assert_eq!(state(&link).wheel, [1, 255, 50]);
        // The read before each write is function 1 of the same feature.
        assert_eq!(write_before(&link, 1)[3], function_byte(1));
        assert_eq!(write_before(&link, 1)[2], last_write(&link)[2]);
    }

    #[test]
    fn the_ends_of_the_wheel_threshold_range_are_accepted() {
        let mut link = link(FakeDevice::mx_master_4());
        for threshold in [1, 50] {
            link.set_wheel(WheelMode::Auto, threshold).unwrap();
            assert_eq!(state(&link).wheel[1], threshold);
        }
    }

    #[test]
    fn a_wheel_threshold_out_of_range_is_never_sent() {
        let mut link = link(FakeDevice::mx_master_4());
        let before = writes(&link);
        for threshold in [0, 51, u8::MAX] {
            assert_eq!(
                link.set_wheel(WheelMode::Auto, threshold),
                Err(HidppError::InvalidArgument(format!(
                    "wheel threshold {threshold}"
                )))
            );
        }
        assert_eq!(writes(&link), before);
    }

    #[test]
    fn a_wheel_that_cannot_be_read_is_not_written() {
        let device = FakeDevice::mx_master_4().with_broken_feature(SMART_SHIFT_ENHANCED);
        let mut link = link(device);
        let before = writes(&link);
        assert_eq!(
            link.set_wheel(WheelMode::Free, 10),
            Err(HidppError::Device(DeviceError::Hidpp20(7)))
        );
        assert_eq!(writes(&link), before + 1, "the read, nothing after");
        assert_eq!(state(&link).wheel, [2, 10, 50]);
    }

    // Scrolling flags.

    #[test]
    fn natural_scrolling_and_resolution_leave_the_other_bits_alone() {
        let mut device = FakeDevice::mx_master_4();
        device.state.hires = 0xF9;
        let mut link = link(device);
        link.set_hires_wheel(false, true).unwrap();
        assert_eq!(&last_write(&link)[4..5], &[0xF9 & !0x06 | 0x04]);
        link.set_hires_wheel(true, false).unwrap();
        assert_eq!(&last_write(&link)[4..5], &[0xF9 & !0x06 | 0x02]);
        link.set_hires_wheel(false, false).unwrap();
        assert_eq!(state(&link).hires, 0xF9 & !0x06);
        link.set_hires_wheel(true, true).unwrap();
        assert_eq!(state(&link).hires, 0xFF);
    }

    #[test]
    fn the_divert_bit_of_the_scrolling_byte_is_kept_either_way() {
        for divert in [0x00, 0x01] {
            let mut device = FakeDevice::mx_master_4();
            device.state.hires = divert;
            let mut link = link(device);
            link.set_hires_wheel(true, true).unwrap();
            assert_eq!(state(&link).hires, divert | 0x06);
        }
    }

    #[test]
    fn the_thumb_wheel_inverts_and_keeps_the_rest_of_both_bytes() {
        let mut device = FakeDevice::mx_master_4();
        device.state.thumb = [0x03, 0x04];
        let mut link = link(device);
        link.set_thumb_invert(true).unwrap();
        assert_eq!(&last_write(&link)[4..6], &[0x03, 0x05]);
        link.set_thumb_invert(false).unwrap();
        assert_eq!(&last_write(&link)[4..6], &[0x03, 0x04]);
    }

    #[test]
    fn short_answers_to_the_flag_reads_stop_the_write() {
        let mut thumb = scripted(ScriptedChannel::new().then_frame(short(8, 1, &[1])));
        assert!(matches!(
            thumb.set_thumb_invert(true),
            Err(HidppError::Malformed(_))
        ));
        let mut hires = scripted(ScriptedChannel::new().then_frame(short(7, 1, &[])));
        assert_eq!(
            hires.set_hires_wheel(true, true),
            Err(HidppError::Malformed("empty wheel flags".to_owned()))
        );
    }

    // Buttons.

    #[test]
    fn a_button_is_diverted_and_given_back_with_the_documented_bytes() {
        let mut link = link(FakeDevice::mx_master_4());
        assert!(!link.is_diverted(MouseControlId::WheelClick).unwrap());
        link.set_diverted(MouseControlId::WheelClick, true).unwrap();
        let report = last_write(&link);
        assert_eq!(report[3], function_byte(3));
        assert_eq!(
            &report[4..9],
            &[0x00, 0x52, 0x03, 0, 0],
            "divert on: 0x02 valid | 0x01 divert"
        );
        assert!(link.is_diverted(MouseControlId::WheelClick).unwrap());
        assert!(state(&link).diverted.contains(&0x0052));
        link.set_diverted(MouseControlId::WheelClick, false)
            .unwrap();
        assert_eq!(
            &last_write(&link)[4..9],
            &[0x00, 0x52, 0x02, 0, 0],
            "divert off"
        );
        assert!(!link.is_diverted(MouseControlId::WheelClick).unwrap());
    }

    #[test]
    fn the_persistence_bit_is_never_set() {
        let mut link = link(FakeDevice::mx_master_4());
        for control in [
            MouseControlId::Back,
            MouseControlId::Forward,
            MouseControlId::HapticPanel,
        ] {
            for diverted in [true, false] {
                link.set_diverted(control, diverted).unwrap();
                let flags = last_write(&link)[6];
                assert!(
                    flags == 0x02 || flags == 0x03,
                    "{control} {diverted}: {flags:#04x}"
                );
            }
        }
    }

    #[test]
    fn buttons_are_named_by_control_and_must_exist_on_the_model() {
        let mut anywhere = link(FakeDevice::mx_anywhere_3s());
        let before = writes(&anywhere);
        for control in [
            MouseControlId::Gesture,
            MouseControlId::HapticPanel,
            MouseControlId::ThumbWheel,
        ] {
            assert!(
                matches!(
                    anywhere.set_diverted(control, true),
                    Err(HidppError::InvalidArgument(_))
                ),
                "{control}"
            );
            assert!(
                matches!(
                    anywhere.is_diverted(control),
                    Err(HidppError::InvalidArgument(_))
                ),
                "{control}"
            );
        }
        assert_eq!(writes(&anywhere), before);
        anywhere
            .set_diverted(MouseControlId::WheelMode, true)
            .unwrap();
        assert_eq!(&last_write(&anywhere)[4..6], &[0x00, 0xC4]);
    }

    #[test]
    fn left_and_right_click_are_never_diverted_but_can_be_given_back() {
        let mut link = link(FakeDevice::mx_master_4());
        let before = writes(&link);
        for control in [MouseControlId::Left, MouseControlId::Right] {
            assert!(
                matches!(
                    link.set_diverted(control, true),
                    Err(HidppError::InvalidArgument(_))
                ),
                "{control}"
            );
        }
        assert_eq!(writes(&link), before);
        assert!(state(&link).diverted.is_empty());
        link.set_diverted(MouseControlId::Left, false).unwrap();
        assert_eq!(&last_write(&link)[4..9], &[0x00, 0x50, 0x02, 0, 0]);
    }

    #[test]
    fn an_answer_about_another_button_is_not_believed() {
        let reply = short(4, 2, &[0x00, 0x53, 0x01, 0, 0]);
        let mut link = scripted(ScriptedChannel::new().then_frame(reply));
        assert_eq!(
            link.is_diverted(MouseControlId::WheelClick),
            Err(HidppError::Malformed(
                "answer about another button".to_owned()
            ))
        );
    }

    #[test]
    fn every_diverted_button_is_given_back_and_nothing_else_is_written() {
        let mut device = FakeDevice::mx_master_4();
        device.state.diverted.extend([0x0052, 0x00C3, 0x01A0]);
        let mut link = link(device);
        link.release_buttons().unwrap();
        assert!(state(&link).diverted.is_empty());
        let before = writes(&link);
        link.release_buttons().unwrap();
        let set_writes = link.hidpp.channel().written()[before..]
            .iter()
            .filter(|report| report[3] == function_byte(3))
            .count();
        assert_eq!(set_writes, 0, "nothing diverted, nothing to give back");
    }

    #[test]
    fn a_failing_release_stops_at_the_first_error() {
        let device = FakeDevice::mx_master_4().with_broken_feature(REPROG_CONTROLS_V4);
        let mut link = link(device);
        assert_eq!(
            link.release_buttons(),
            Err(HidppError::Device(DeviceError::Hidpp20(7)))
        );
    }

    fn notification(device: u8, feature: u8, event: u8, params: &[u8]) -> Notification {
        Notification::parse(&frame(device, feature, event << 4, params)).unwrap()
    }

    #[test]
    fn held_buttons_come_from_the_four_slots_and_only_for_this_mouse() {
        let link = link(FakeDevice::mx_master_4());
        let reprog = link.features.reprog.unwrap();
        let held = notification(DEVICE, reprog, 0, &[0, 0x52, 0, 0x56, 0, 0, 0, 0]);
        assert_eq!(
            link.held_buttons(&held),
            Some(vec![MouseControlId::WheelClick, MouseControlId::Forward])
        );
        let none = notification(DEVICE, reprog, 0, &[0; 8]);
        assert_eq!(link.held_buttons(&none), Some(Vec::new()));
        // Another device of the same receiver, with the same feature index.
        assert_eq!(
            link.held_buttons(&notification(1, reprog, 0, &[0, 0x52])),
            None
        );
        assert_eq!(
            link.held_buttons(&notification(DEVICE, reprog + 1, 0, &[0, 0x52])),
            None
        );
        assert_eq!(
            link.held_buttons(&notification(DEVICE, reprog, 1, &[0, 0x52])),
            None
        );
    }

    #[test]
    fn held_buttons_ignore_unknown_ids_and_odd_or_short_frames() {
        let link = link(FakeDevice::mx_anywhere_3s());
        let reprog = link.features.reprog.unwrap();
        // 0x00C3 is not a button of the 3S; zero slots are empty; a lone last byte is dropped.
        let params = [0, 0xC3, 0, 0, 0, 0x53, 0, 0x53, 0x99];
        assert_eq!(
            link.held_buttons(&notification(DEVICE, reprog, 0, &params)),
            Some(vec![MouseControlId::Back, MouseControlId::Back])
        );
        let short = Notification::parse(&[0x10, DEVICE, reprog, 0x00, 0, 0x56, 0]).unwrap();
        assert_eq!(
            link.held_buttons(&short),
            Some(vec![MouseControlId::Forward])
        );
        let more = [0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6];
        assert_eq!(
            link.held_buttons(&notification(DEVICE, reprog, 0, &more)),
            Some(Vec::new())
        );
    }

    #[test]
    fn only_four_slots_are_read() {
        let link = link(FakeDevice::mx_master_4());
        let reprog = link.features.reprog.unwrap();
        let params = [0, 0x52, 0, 0x53, 0, 0x56, 0, 0xC3, 0, 0xC4];
        let held = link
            .held_buttons(&notification(DEVICE, reprog, 0, &params))
            .unwrap();
        assert_eq!(held.len(), DIVERTED_SLOTS);
        assert!(!held.contains(&MouseControlId::WheelMode));
    }

    #[test]
    fn presses_and_releases_come_from_the_changes_between_notifications() {
        use MouseControlId::{Back, Forward, WheelClick};
        let event = |control, pressed| ButtonEvent { control, pressed };
        let mut tracker = ButtonTracker::default();
        assert_eq!(tracker.update(&[]), []);
        assert_eq!(tracker.update(&[WheelClick]), [event(WheelClick, true)]);
        assert_eq!(tracker.update(&[WheelClick]), [], "still held");
        assert_eq!(
            tracker.update(&[WheelClick, Forward]),
            [event(Forward, true)]
        );
        assert_eq!(
            tracker.update(&[Forward, Back]),
            [event(WheelClick, false), event(Back, true)],
            "releases come first"
        );
        assert_eq!(
            tracker.update(&[]),
            [event(Back, false), event(Forward, false)]
        );
    }

    #[test]
    fn a_repeated_control_counts_once_and_a_reconnection_releases_everything() {
        use MouseControlId::{Back, Forward, Gesture, WheelClick};
        let event = |control, pressed| ButtonEvent { control, pressed };
        let mut tracker = ButtonTracker::default();
        assert_eq!(tracker.update(&[Back, Back]), [event(Back, true)]);
        let four = [WheelClick, Back, Forward, Gesture];
        let pressed = tracker.update(&four);
        assert_eq!(pressed.len(), 3, "Back was already down");
        assert_eq!(
            tracker.update(&[WheelClick, Back, Gesture]),
            [event(Forward, false)]
        );
        assert_eq!(
            tracker.release_all(),
            [
                event(WheelClick, false),
                event(Back, false),
                event(Gesture, false)
            ]
        );
        assert_eq!(tracker.release_all(), []);
    }

    // Haptics.

    #[test]
    fn haptic_feedback_is_switched_and_read_with_the_documented_bytes() {
        let mut link = link(FakeDevice::mx_master_4());
        assert_eq!(
            link.haptic_state().unwrap(),
            HapticState {
                enabled: true,
                level: 75
            }
        );
        link.set_haptic(Some(50)).unwrap();
        assert_eq!(&last_write(&link)[4..6], &[1, 50]);
        link.set_haptic(None).unwrap();
        assert_eq!(
            &last_write(&link)[4..6],
            &[0, 50],
            "off parks the level at 50"
        );
        assert_eq!(
            link.haptic_state().unwrap(),
            HapticState {
                enabled: false,
                level: 50
            }
        );
        let before = writes(&link);
        assert!(matches!(
            link.set_haptic(Some(101)),
            Err(HidppError::InvalidArgument(_))
        ));
        assert_eq!(writes(&link), before);
    }

    #[test]
    fn a_waveform_is_played_by_its_id_when_the_device_supports_it() {
        let mut link = link(FakeDevice::mx_master_4());
        link.play_haptic(HapticWaveform::Knock).unwrap();
        assert_eq!(&last_write(&link)[3..5], &[function_byte(4), 0x0C]);
        link.play_haptic(HapticWaveform::WhisperCollision).unwrap();
        assert_eq!(&last_write(&link)[3..5], &[function_byte(4), 0x1B]);
        assert_eq!(state(&link).played, [0x0C, 0x1B]);
    }

    #[test]
    fn the_capability_list_is_read_bit_by_bit() {
        let mut device = FakeDevice::mx_master_4();
        device.state.haptic_mask = 0x0000_7FFF;
        let link = link(device);
        assert!(link.supports_waveform(HapticWaveform::Knock));
        assert!(link.supports_waveform(HapticWaveform::Ringing));
        assert!(
            !link.supports_waveform(HapticWaveform::WhisperCollision),
            "bit 27 is not set"
        );
        let mut only_first = FakeDevice::mx_master_4();
        only_first.state.haptic_mask = 0x0000_0001;
        let mut link = self::link(only_first);
        assert!(link.supports_waveform(HapticWaveform::SharpStateChange));
        assert!(!link.supports_waveform(HapticWaveform::Knock));
        assert!(matches!(
            link.play_haptic(HapticWaveform::Knock),
            Err(HidppError::InvalidArgument(_))
        ));
        assert_eq!(state(&link).played.len(), 0);
    }

    #[test]
    fn waveform_ids_are_distinct_and_the_documented_ones() {
        let ids: Vec<u8> = HapticWaveform::ALL
            .iter()
            .map(|w| waveform_id(*w).unwrap())
            .collect();
        let mut distinct = ids.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), HapticWaveform::ALL.len());
        assert!(
            ids.iter().all(|id| *id < 32),
            "the capability list is 32 bits"
        );
        assert_eq!(waveform_id(HapticWaveform::SharpStateChange), Some(0x00));
        assert_eq!(waveform_id(HapticWaveform::Ringing), Some(0x0E));
        assert_eq!(waveform_id(HapticWaveform::WhisperCollision), Some(0x1B));
    }

    #[test]
    fn levels_map_to_strengths() {
        assert_eq!(haptic_strength(HapticLevel::Off), None);
        assert_eq!(haptic_strength(HapticLevel::Subtle), Some(25));
        assert_eq!(haptic_strength(HapticLevel::Low), Some(50));
        assert_eq!(haptic_strength(HapticLevel::Medium), Some(75));
        assert_eq!(haptic_strength(HapticLevel::High), Some(100));
    }

    // Errors from the device on a write: nothing changes, nothing panics.

    #[test]
    fn a_device_that_refuses_a_write_changes_nothing() {
        type Setter = Box<dyn Fn(&mut MouseLink<FakeLink>) -> Result<(), HidppError>>;
        let cases: [(&str, u16, Setter); 5] = [
            ("sensitivity", ADJUSTABLE_DPI, Box::new(|l| l.set_dpi(800))),
            (
                "scrolling",
                HIRES_WHEEL,
                Box::new(|l| l.set_hires_wheel(false, true)),
            ),
            (
                "thumb wheel",
                THUMB_WHEEL,
                Box::new(|l| l.set_thumb_invert(true)),
            ),
            ("haptic level", HAPTIC, Box::new(|l| l.set_haptic(Some(10)))),
            (
                "diversion",
                REPROG_CONTROLS_V4,
                Box::new(|l| l.set_diverted(MouseControlId::Back, true)),
            ),
        ];
        for (what, feature, set) in cases {
            let mut device = FakeDevice::mx_master_4().with_broken_feature(feature);
            device.state.haptic_mask = 0;
            let mut link = link(device);
            let before = state(&link).clone();
            assert_eq!(
                set(&mut link),
                Err(HidppError::Device(DeviceError::Hidpp20(7))),
                "{what}"
            );
            let after = state(&link);
            assert_eq!(
                (
                    after.dpi,
                    after.hires,
                    after.thumb,
                    after.haptic,
                    &after.diverted
                ),
                (
                    before.dpi,
                    before.hires,
                    before.thumb,
                    before.haptic,
                    &before.diverted
                ),
                "{what}"
            );
        }
    }

    #[test]
    fn a_receiver_that_does_not_know_the_device_refuses_every_operation() {
        let link = MouseLink::connect(
            Hidpp::new(FakeLink::receiver(vec![(1, FakeDevice::mx_master_4())])),
            1,
        )
        .unwrap();
        let mut stranger = MouseLink { device: 3, ..link };
        assert_eq!(
            stranger.set_dpi(800),
            Err(HidppError::Device(DeviceError::Hidpp10(8)))
        );
    }
}
