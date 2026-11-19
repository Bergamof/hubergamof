//! Operations on one mouse behind a [`Hidpp`]: what the manager reads from it
//! and writes to it. This is the only part of the crate that sends anything
//! but getters.
//!
//! Function numbers and byte layouts follow what Solaar (`lib/logitech_receiver`)
//! does with these features; each write checks its argument first, and the
//! read-modify-write ones keep the bits they do not own.

use std::collections::BTreeSet;

use hubergamof_protocol::{
    DPI_RANGE, DPI_STEP, HapticLevel, HapticWaveform, WHEEL_THRESHOLD_RANGE, WheelMode,
};

use crate::channel::{Channel, Hidpp, HidppError};
use crate::controls::is_locked_cid;
use crate::features::{
    ADJUSTABLE_DPI, DEVICE_FIRMWARE, HAPTIC, HIRES_WHEEL, REPROG_CONTROLS_V4, SMART_SHIFT_ENHANCED,
    THUMB_WHEEL, UNIFIED_BATTERY,
};
use crate::frame::Notification;

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

/// Control ids a diverted-button notification reports as held down.
#[must_use]
pub fn diverted_buttons(notification: &Notification, reprog_index: u8) -> Option<Vec<u16>> {
    if notification.feature_index != reprog_index || notification.event != 0 {
        return None;
    }
    let held = notification
        .params
        .as_chunks::<2>()
        .0
        .iter()
        .take(DIVERTED_SLOTS)
        .map(|pair| u16::from_be_bytes(*pair))
        .filter(|cid| *cid != 0)
        .collect();
    Some(held)
}

/// A press or a release of a diverted button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonEvent {
    /// The button, by the mouse's control id.
    pub cid: u16,
    /// `true` when pressed, `false` when released.
    pub pressed: bool,
}

/// Turns the held-down sets of successive notifications into presses and
/// releases.
#[derive(Debug, Default)]
pub struct ButtonTracker {
    down: BTreeSet<u16>,
}

impl ButtonTracker {
    /// Events that bring the previous state to `held`: releases first.
    pub fn update(&mut self, held: &[u16]) -> Vec<ButtonEvent> {
        let now: BTreeSet<u16> = held.iter().copied().collect();
        let released = self.down.difference(&now).map(|cid| ButtonEvent {
            cid: *cid,
            pressed: false,
        });
        let pressed = now.difference(&self.down).map(|cid| ButtonEvent {
            cid: *cid,
            pressed: true,
        });
        let events: Vec<ButtonEvent> = released.chain(pressed).collect();
        self.down = now;
        events
    }
}

impl<C: Channel> MouseLink<C> {
    /// Looks up the features of the device at `device`.
    ///
    /// # Errors
    /// [`HidppError`] when the device does not answer the lookups.
    pub fn connect(mut hidpp: Hidpp<C>, device: u8) -> Result<Self, HidppError> {
        let mut lookup = |id: u16| -> Result<Option<u8>, HidppError> {
            let [hi, lo] = id.to_be_bytes();
            let params = hidpp.request(device, 0, 0, &[hi, lo])?;
            Ok(params.first().copied().filter(|index| *index != 0))
        };
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
        let mut link = Self {
            hidpp,
            device,
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

    /// Index of the button feature, to recognise its notifications.
    #[must_use]
    pub fn reprog_index(&self) -> Option<u8> {
        self.features.reprog
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

    /// Sets the sensitivity.
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

    /// Sets how the wheel turns; the torque is left as the device has it.
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

    /// Sets natural scrolling (`invert`) and high resolution, keeping the other
    /// flag of the byte (whether the wheel is diverted).
    ///
    /// # Errors
    /// [`HidppError`] when the device lacks the feature or does not answer.
    pub fn set_hires_wheel(&mut self, high_res: bool, invert: bool) -> Result<(), HidppError> {
        let params = self.call(self.features.hires, "high resolution wheel", 1, &[])?;
        let current = *params
            .first()
            .ok_or_else(|| HidppError::Malformed("empty wheel flags".to_owned()))?;
        let flags =
            (current & 0x01) | if high_res { 0x02 } else { 0 } | if invert { 0x04 } else { 0 };
        self.call(self.features.hires, "high resolution wheel", 2, &[flags])
            .map(|_| ())
    }

    /// Reverses the thumb wheel, keeping whether it is diverted.
    ///
    /// # Errors
    /// [`HidppError`] when the device lacks the feature or does not answer.
    pub fn set_thumb_invert(&mut self, invert: bool) -> Result<(), HidppError> {
        let params = self.call(self.features.thumb, "thumb wheel", 1, &[])?;
        let diverted = params.first().copied().unwrap_or(0) & 0x01;
        self.call(
            self.features.thumb,
            "thumb wheel",
            2,
            &[diverted, u8::from(invert)],
        )
        .map(|_| ())
    }

    /// Whether the button with this control id is diverted to us.
    ///
    /// # Errors
    /// [`HidppError`] when the device lacks the feature or does not answer.
    pub fn is_diverted(&mut self, cid: u16) -> Result<bool, HidppError> {
        let [hi, lo] = cid.to_be_bytes();
        let params = self.call(self.features.reprog, "remappable buttons", 2, &[hi, lo])?;
        Ok(params.get(2).is_some_and(|flags| flags & 0x01 != 0))
    }

    /// Diverts a button to us, or gives it back. Left and right click are
    /// refused whatever the caller says.
    ///
    /// # Errors
    /// [`HidppError::InvalidArgument`] for a locked button, other
    /// [`HidppError`] when the device lacks the feature or does not answer.
    pub fn set_diverted(&mut self, cid: u16, diverted: bool) -> Result<(), HidppError> {
        if is_locked_cid(cid) {
            return Err(HidppError::InvalidArgument(format!(
                "control {cid:#06x} is never diverted"
            )));
        }
        let [hi, lo] = cid.to_be_bytes();
        // Bit 0 says "divert", bit 1 says that bit 0 is meant.
        let flags = 0x02 | u8::from(diverted);
        self.call(
            self.features.reprog,
            "remappable buttons",
            3,
            &[hi, lo, flags, 0, 0],
        )
        .map(|_| ())
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
    use super::*;
    use crate::frame::SOFTWARE_ID;
    use crate::testing::{FakeDevice, FakeLink, frame};

    const DEVICE: u8 = 0xFF;

    fn link(device: FakeDevice) -> MouseLink<FakeLink> {
        MouseLink::connect(Hidpp::new(FakeLink::direct(device)), DEVICE).unwrap()
    }

    fn state(link: &MouseLink<FakeLink>) -> &crate::testing::FakeState {
        &link.hidpp.channel().device_at(DEVICE).unwrap().state
    }

    fn writes(link: &MouseLink<FakeLink>) -> usize {
        link.hidpp.channel().written().len()
    }

    fn last_write(link: &MouseLink<FakeLink>) -> Vec<u8> {
        link.hidpp.channel().written().last().unwrap().clone()
    }

    fn function_byte(function: u8) -> u8 {
        function << 4 | SOFTWARE_ID
    }

    #[test]
    fn the_features_of_each_model_are_found() {
        let master = link(FakeDevice::mx_master_4());
        assert!(master.has_haptics() && master.has_thumb_wheel());
        assert!(master.reprog_index().is_some());
        let anywhere = link(FakeDevice::mx_anywhere_3s());
        assert!(!anywhere.has_haptics() && !anywhere.has_thumb_wheel());
        assert_eq!(master.device(), DEVICE);
    }

    #[test]
    fn a_silent_device_cannot_be_linked() {
        let result = MouseLink::connect(
            Hidpp::new(FakeLink::silent()).with_timeout(std::time::Duration::from_millis(1)),
            DEVICE,
        );
        assert!(matches!(result, Err(HidppError::Timeout(_))));
    }

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
    fn the_sensitivity_is_read_and_set_with_the_documented_bytes() {
        let mut link = link(FakeDevice::mx_master_4());
        assert_eq!(link.dpi().unwrap(), 1600);
        link.set_dpi(2400).unwrap();
        let report = last_write(&link);
        // report id, device, feature index, function 3, then sensor 0 and the dpi big-endian
        assert_eq!(report[3], function_byte(3));
        assert_eq!(&report[4..7], &[0, 0x09, 0x60]);
        assert_eq!(state(&link).dpi, 2400);
        assert_eq!(link.dpi().unwrap(), 2400);
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
        assert_eq!(state(&link).wheel, [2, 30, 50]);
        link.set_wheel(WheelMode::Ratchet, 30).unwrap();
        assert_eq!(state(&link).wheel, [2, 255, 50]);
        link.set_wheel(WheelMode::Free, 30).unwrap();
        // The threshold is left alone (zero), the mode is free spin.
        assert_eq!(state(&link).wheel, [1, 255, 50]);
        assert_eq!(&last_write(&link)[4..7], &[1, 0, 50]);
    }

    #[test]
    fn a_wheel_threshold_out_of_range_is_never_sent() {
        let mut link = link(FakeDevice::mx_master_4());
        let before = writes(&link);
        for threshold in [0, 51, u8::MAX] {
            assert!(matches!(
                link.set_wheel(WheelMode::Auto, threshold),
                Err(HidppError::InvalidArgument(_))
            ));
        }
        assert_eq!(writes(&link), before);
    }

    #[test]
    fn natural_scrolling_and_resolution_keep_the_divert_bit() {
        let mut link = link(FakeDevice::mx_master_4());
        link.set_hires_wheel(false, true).unwrap();
        assert_eq!(
            state(&link).hires,
            0x05,
            "divert kept, no high resolution, inverted"
        );
        link.set_hires_wheel(true, false).unwrap();
        assert_eq!(state(&link).hires, 0x03);
    }

    #[test]
    fn the_thumb_wheel_inverts_without_losing_its_divert() {
        let mut device = FakeDevice::mx_master_4();
        device.state.thumb = [1, 0];
        let mut link = link(device);
        link.set_thumb_invert(true).unwrap();
        assert_eq!(state(&link).thumb, [1, 1]);
        link.set_thumb_invert(false).unwrap();
        assert_eq!(state(&link).thumb, [1, 0]);
    }

    #[test]
    fn a_button_is_diverted_and_given_back_with_the_documented_bytes() {
        let mut link = link(FakeDevice::mx_master_4());
        assert!(!link.is_diverted(0x0052).unwrap());
        link.set_diverted(0x0052, true).unwrap();
        let report = last_write(&link);
        assert_eq!(report[3], function_byte(3));
        assert_eq!(
            &report[4..9],
            &[0x00, 0x52, 0x03, 0, 0],
            "divert on: 0x02 valid | 0x01 divert"
        );
        assert!(link.is_diverted(0x0052).unwrap());
        assert!(state(&link).diverted.contains(&0x0052));
        link.set_diverted(0x0052, false).unwrap();
        assert_eq!(
            &last_write(&link)[4..9],
            &[0x00, 0x52, 0x02, 0, 0],
            "divert off"
        );
        assert!(!link.is_diverted(0x0052).unwrap());
    }

    #[test]
    fn left_and_right_click_are_never_diverted() {
        let mut link = link(FakeDevice::mx_master_4());
        let before = writes(&link);
        for cid in [0x0050, 0x0051] {
            assert!(
                matches!(
                    link.set_diverted(cid, true),
                    Err(HidppError::InvalidArgument(_))
                ),
                "{cid:#06x}"
            );
        }
        assert_eq!(writes(&link), before);
        assert!(state(&link).diverted.is_empty());
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
        assert!(link.supports_waveform(HapticWaveform::Knock));
        assert!(link.supports_waveform(HapticWaveform::WhisperCollision));
        link.play_haptic(HapticWaveform::Knock).unwrap();
        link.play_haptic(HapticWaveform::WhisperCollision).unwrap();
        assert_eq!(state(&link).played, [0x0C, 0x1B]);
        assert_eq!(last_write(&link)[3], function_byte(4));
    }

    #[test]
    fn a_waveform_the_device_does_not_list_is_not_played() {
        let mut device = FakeDevice::mx_master_4();
        device.state.haptic_mask = 0x0000_0001;
        let mut link = link(device);
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

    fn button_notification(feature: u8, event: u8, params: &[u8]) -> Notification {
        Notification::parse(&frame(DEVICE, feature, event << 4, params)).unwrap()
    }

    #[test]
    fn diverted_buttons_are_read_from_the_four_slots() {
        let held = button_notification(9, 0, &[0, 0x52, 0, 0x56, 0, 0, 0, 0]);
        assert_eq!(diverted_buttons(&held, 9), Some(vec![0x0052, 0x0056]));
        let none = button_notification(9, 0, &[0; 8]);
        assert_eq!(diverted_buttons(&none, 9), Some(Vec::new()));
        let other_feature = button_notification(8, 0, &[0, 0x52]);
        assert_eq!(diverted_buttons(&other_feature, 9), None);
        let other_event = button_notification(9, 1, &[0, 0x52]);
        assert_eq!(diverted_buttons(&other_event, 9), None);
        let long = button_notification(9, 0, &[0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6]);
        assert_eq!(diverted_buttons(&long, 9).unwrap().len(), DIVERTED_SLOTS);
    }

    #[test]
    fn presses_and_releases_come_from_the_changes_between_notifications() {
        let mut tracker = ButtonTracker::default();
        assert_eq!(tracker.update(&[]), []);
        assert_eq!(
            tracker.update(&[0x52]),
            [ButtonEvent {
                cid: 0x52,
                pressed: true
            }]
        );
        assert_eq!(tracker.update(&[0x52]), [], "still held");
        assert_eq!(
            tracker.update(&[0x52, 0x56]),
            [ButtonEvent {
                cid: 0x56,
                pressed: true
            }]
        );
        assert_eq!(
            tracker.update(&[0x56, 0x53]),
            [
                ButtonEvent {
                    cid: 0x52,
                    pressed: false
                },
                ButtonEvent {
                    cid: 0x53,
                    pressed: true
                }
            ],
            "releases come first"
        );
        assert_eq!(
            tracker.update(&[]),
            [
                ButtonEvent {
                    cid: 0x53,
                    pressed: false
                },
                ButtonEvent {
                    cid: 0x56,
                    pressed: false
                }
            ]
        );
    }
}
