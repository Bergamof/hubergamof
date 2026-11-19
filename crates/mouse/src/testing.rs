//! A scripted HID++ device for the tests.
//!
//! It encodes the same function numbers and bytes as the code it tests, from the
//! same source (Solaar): a green test shows the code is consistent, not that a
//! real mouse agrees. That is what the checks on the hardware are for
//! (`docs/mouse-hidpp.md`).

use std::collections::{BTreeSet, VecDeque};
use std::time::Duration;

use crate::channel::{Channel, ChannelError};
use crate::features::{
    ADJUSTABLE_DPI, DEVICE_FIRMWARE, DEVICE_NAME, FEATURE_SET, HAPTIC, HIRES_WHEEL,
    REPROG_CONTROLS_V4, SMART_SHIFT_ENHANCED, THUMB_WHEEL, UNIFIED_BATTERY,
};
use crate::frame::{LONG_REPORT_ID, LONG_REPORT_LEN};

/// One answer frame, padded to a long report.
pub fn frame(device: u8, feature: u8, function_byte: u8, params: &[u8]) -> Vec<u8> {
    let mut bytes = vec![LONG_REPORT_ID, device, feature, function_byte];
    bytes.extend_from_slice(params);
    bytes.resize(LONG_REPORT_LEN, 0);
    bytes
}

/// What the scripted device is: features, name and buttons.
#[derive(Debug, Clone)]
pub struct FakeDevice {
    /// Features after `FeatureSet` (index 1): `(id, version)`, indexes from 2.
    features: Vec<(u16, u8)>,
    name: String,
    controls: Vec<[u8; 9]>,
    /// Features whose functions all fail with "invalid function".
    broken: Vec<u16>,
    /// Features whose lookup at the root fails with an error.
    lookup_errors: Vec<u16>,
    /// What the settings of the device currently are.
    pub state: FakeState,
}

/// The settings a scripted mouse keeps, as a real one would.
#[derive(Debug, Clone)]
pub struct FakeState {
    pub unit_id: [u8; 4],
    pub battery: [u8; 3],
    pub dpi: u16,
    pub wheel: [u8; 3],
    pub hires: u8,
    pub thumb: [u8; 2],
    pub diverted: BTreeSet<u16>,
    pub haptic: [u8; 2],
    pub haptic_mask: u32,
    /// Waveform ids played, in order.
    pub played: Vec<u8>,
}

impl Default for FakeState {
    fn default() -> Self {
        Self {
            unit_id: [0xDE, 0xAD, 0xBE, 0xEF],
            battery: [82, 4, 0],
            dpi: 1600,
            wheel: [2, 10, 50],
            hires: 0x03,
            thumb: [0, 0],
            diverted: BTreeSet::new(),
            haptic: [1, 75],
            haptic_mask: 0x0800_7FFF,
            played: Vec::new(),
        }
    }
}

impl FakeDevice {
    pub fn new() -> Self {
        Self {
            features: Vec::new(),
            name: String::new(),
            controls: Vec::new(),
            broken: Vec::new(),
            lookup_errors: Vec::new(),
            state: FakeState::default(),
        }
    }

    /// A mouse with the features of the MX Master 4 and the buttons the probe
    /// read from it.
    pub fn mx_master_4() -> Self {
        let device = Self::new()
            .with_feature(DEVICE_FIRMWARE, 8)
            .with_name("MX Master 4")
            .with_feature(UNIFIED_BATTERY, 5)
            .with_feature(HAPTIC, 0)
            .with_feature(ADJUSTABLE_DPI, 2)
            .with_feature(SMART_SHIFT_ENHANCED, 0)
            .with_feature(HIRES_WHEEL, 1)
            .with_feature(THUMB_WHEEL, 0);
        [
            (0x0050, 0x0038, 0x01, 0x04),
            (0x0051, 0x0039, 0x01, 0x04),
            (0x0052, 0x003A, 0x31, 0x05),
            (0x0053, 0x003C, 0x31, 0x05),
            (0x0056, 0x003E, 0x31, 0x05),
            (0x00C3, 0x009C, 0x31, 0x05),
            (0x00C4, 0x009D, 0x31, 0x05),
            (0x01A0, 0x0109, 0x31, 0x05),
        ]
        .into_iter()
        .fold(device, |device, (cid, tid, flags, flags2)| {
            device.with_control(cid, tid, flags, flags2)
        })
    }

    /// A mouse with the features of the MX Anywhere 3S: no haptics, no thumb wheel.
    pub fn mx_anywhere_3s() -> Self {
        let device = Self::new()
            .with_feature(DEVICE_FIRMWARE, 4)
            .with_name("MX Anywhere 3S")
            .with_feature(UNIFIED_BATTERY, 3)
            .with_feature(ADJUSTABLE_DPI, 2)
            .with_feature(SMART_SHIFT_ENHANCED, 0)
            .with_feature(HIRES_WHEEL, 1);
        [
            (0x0050, 0x0038, 0x01, 0x04),
            (0x0051, 0x0039, 0x01, 0x04),
            (0x0052, 0x003A, 0x31, 0x05),
            (0x0053, 0x003C, 0x31, 0x0D),
            (0x0056, 0x003E, 0x31, 0x0D),
            (0x00C4, 0x009D, 0x31, 0x05),
        ]
        .into_iter()
        .fold(device, |device, (cid, tid, flags, flags2)| {
            device.with_control(cid, tid, flags, flags2)
        })
    }

    pub fn with_feature(mut self, id: u16, version: u8) -> Self {
        self.features.push((id, version));
        self
    }

    pub fn with_name(mut self, name: &str) -> Self {
        self.name = name.to_owned();
        self.with_feature(DEVICE_NAME, 0)
    }

    pub fn with_control(self, cid: u16, tid: u16, flags: u8, flags2: u8) -> Self {
        self.with_control_layout(cid, tid, flags, flags2, [0, 0, 0])
    }

    /// A button with its position, group and group mask, in that order.
    pub fn with_control_layout(
        mut self,
        cid: u16,
        tid: u16,
        flags: u8,
        flags2: u8,
        [position, group, mask]: [u8; 3],
    ) -> Self {
        if !self
            .features
            .iter()
            .any(|(id, _)| *id == REPROG_CONTROLS_V4)
        {
            self.features.push((REPROG_CONTROLS_V4, 4));
        }
        let [c0, c1] = cid.to_be_bytes();
        let [t0, t1] = tid.to_be_bytes();
        self.controls
            .push([c0, c1, t0, t1, flags, position, group, mask, flags2]);
        self
    }

    /// The root answers an error, not an index, when asked for this feature.
    pub fn with_lookup_error(mut self, id: u16) -> Self {
        self.lookup_errors.push(id);
        self
    }

    pub fn with_broken_feature(mut self, id: u16) -> Self {
        self.broken.push(id);
        self
    }

    /// Feature id at an HID++ feature index (0 is `Root`, 1 is `FeatureSet`).
    fn feature_at(&self, index: u8) -> Option<u16> {
        match index {
            0 => Some(0),
            1 => Some(FEATURE_SET),
            other => self.features.get(usize::from(other) - 2).map(|(id, _)| *id),
        }
    }

    fn index_of(&self, id: u16) -> Option<u8> {
        (1..=u8::try_from(self.features.len() + 1).unwrap_or(u8::MAX))
            .find(|index| self.feature_at(*index) == Some(id))
    }

    /// Params of the answer to `function` of the feature at `index`.
    fn answer(&mut self, index: u8, function: u8, params: &[u8]) -> Option<Vec<u8>> {
        let feature = self.feature_at(index)?;
        if self.broken.contains(&feature) {
            return None;
        }
        match (feature, function) {
            (0, 0) => {
                let wanted = u16::from_be_bytes([*params.first()?, *params.get(1)?]);
                if self.lookup_errors.contains(&wanted) {
                    return None;
                }
                let found = if wanted == 0 {
                    Some(0)
                } else {
                    self.index_of(wanted)
                };
                Some(vec![found.unwrap_or(0), 0, 0])
            }
            (0, 1) => Some(vec![4, 5, *params.get(2)?]),
            (FEATURE_SET, 0) => Some(vec![u8::try_from(self.features.len() + 1).ok()?]),
            (FEATURE_SET, 1) => {
                let wanted = *params.first()?;
                let id = self.feature_at(wanted)?;
                let version = usize::from(wanted)
                    .checked_sub(2)
                    .map_or(0, |at| self.features[at].1);
                let [hi, lo] = id.to_be_bytes();
                Some(vec![hi, lo, 0, version])
            }
            (DEVICE_NAME, 0) => Some(vec![u8::try_from(self.name.len()).ok()?]),
            (DEVICE_NAME, 1) => {
                let offset = usize::from(*params.first()?);
                Some(self.name.bytes().skip(offset).take(16).collect())
            }
            (REPROG_CONTROLS_V4, 0) => Some(vec![u8::try_from(self.controls.len()).ok()?]),
            (REPROG_CONTROLS_V4, 1) => {
                Some(self.controls.get(usize::from(*params.first()?))?.to_vec())
            }
            (REPROG_CONTROLS_V4, 2) => {
                let cid = u16::from_be_bytes([*params.first()?, *params.get(1)?]);
                let [hi, lo] = cid.to_be_bytes();
                Some(vec![
                    hi,
                    lo,
                    u8::from(self.state.diverted.contains(&cid)),
                    0,
                    0,
                ])
            }
            (REPROG_CONTROLS_V4, 3) => {
                let cid = u16::from_be_bytes([*params.first()?, *params.get(1)?]);
                let flags = *params.get(2)?;
                if flags & 0x02 != 0 {
                    if flags & 0x01 != 0 {
                        self.state.diverted.insert(cid);
                    } else {
                        self.state.diverted.remove(&cid);
                    }
                }
                Some(params.iter().copied().take(5).collect())
            }
            _ => self.answer_setting(feature, function, params),
        }
    }
    /// The getters and setters of the settings a mouse keeps.
    fn answer_setting(&mut self, feature: u16, function: u8, params: &[u8]) -> Option<Vec<u8>> {
        match (feature, function) {
            (DEVICE_FIRMWARE, 0) => {
                let [a, b, c, d] = self.state.unit_id;
                Some(vec![1, a, b, c, d, 0, 0x04])
            }
            (UNIFIED_BATTERY, 1) => Some(self.state.battery.to_vec()),
            (ADJUSTABLE_DPI, 2) => {
                let [hi, lo] = self.state.dpi.to_be_bytes();
                Some(vec![0, hi, lo, 0x03, 0xE8])
            }
            (ADJUSTABLE_DPI, 3) => {
                self.state.dpi = u16::from_be_bytes([*params.get(1)?, *params.get(2)?]);
                Some(params.iter().copied().take(3).collect())
            }
            (SMART_SHIFT_ENHANCED, 1) => Some(self.state.wheel.to_vec()),
            (SMART_SHIFT_ENHANCED, 2) => {
                // A zero leaves the setting as it is.
                for (slot, value) in self.state.wheel.iter_mut().zip(params.iter().take(3)) {
                    if *value != 0 {
                        *slot = *value;
                    }
                }
                Some(params.iter().copied().take(3).collect())
            }
            (HIRES_WHEEL, 1) => Some(vec![self.state.hires]),
            (HIRES_WHEEL, 2) => {
                self.state.hires = *params.first()?;
                Some(vec![self.state.hires])
            }
            (THUMB_WHEEL, 1) => Some(self.state.thumb.to_vec()),
            (THUMB_WHEEL, 2) => {
                self.state.thumb = [*params.first()?, *params.get(1)?];
                Some(self.state.thumb.to_vec())
            }
            (HAPTIC, 0) => {
                let [a, b, c, d] = self.state.haptic_mask.to_be_bytes();
                Some(vec![0, 0, 0, 0, a, b, c, d])
            }
            (HAPTIC, 1) => Some(self.state.haptic.to_vec()),
            (HAPTIC, 2) => {
                self.state.haptic = [*params.first()?, *params.get(1)?];
                Some(self.state.haptic.to_vec())
            }
            (HAPTIC, 4) => {
                self.state.played.push(*params.first()?);
                Some(vec![])
            }
            _ => None,
        }
    }
}

/// An interface with devices behind it, scripted.
#[derive(Debug)]
pub struct FakeLink {
    devices: Vec<(u8, FakeDevice)>,
    /// A receiver answers an HID++ 1.0 error for an index nobody has.
    receiver: bool,
    silent: bool,
    failing: bool,
    noise: Option<Vec<u8>>,
    queue: VecDeque<Vec<u8>>,
    written: Vec<Vec<u8>>,
}

impl FakeLink {
    fn base(devices: Vec<(u8, FakeDevice)>, receiver: bool) -> Self {
        Self {
            devices,
            receiver,
            silent: false,
            failing: false,
            noise: None,
            queue: VecDeque::new(),
            written: Vec::new(),
        }
    }

    /// A device reached directly (index `0xFF`).
    pub fn direct(device: FakeDevice) -> Self {
        Self::base(vec![(0xFF, device)], false)
    }

    /// A receiver with devices at the given indexes.
    pub fn receiver(devices: Vec<(u8, FakeDevice)>) -> Self {
        Self::base(devices, true)
    }

    /// An interface that never answers.
    pub fn silent() -> Self {
        Self {
            silent: true,
            ..Self::base(Vec::new(), false)
        }
    }

    /// An interface that refuses every write.
    pub fn failing() -> Self {
        Self {
            failing: true,
            ..Self::base(Vec::new(), false)
        }
    }

    /// The scripted device at an index.
    pub fn device_at(&self, index: u8) -> Option<&FakeDevice> {
        self.devices
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, device)| device)
    }

    /// Every report written so far.
    pub fn written(&self) -> &[Vec<u8>] {
        &self.written
    }

    /// A frame sent before every answer (a notification).
    pub fn with_noise(mut self, frame: Vec<u8>) -> Self {
        self.noise = Some(frame);
        self
    }
}

impl Channel for FakeLink {
    fn write(&mut self, report: &[u8]) -> Result<(), ChannelError> {
        if self.failing {
            return Err(ChannelError("write refused".to_owned()));
        }
        self.written.push(report.to_vec());
        if self.silent {
            return Ok(());
        }
        let [_, device, feature, function_byte, params @ ..] = report else {
            return Err(ChannelError("short report".to_owned()));
        };
        self.queue.extend(self.noise.clone());
        match self.devices.iter_mut().find(|(index, _)| index == device) {
            Some((_, scripted)) => {
                if let Some(answer) = scripted.answer(*feature, function_byte >> 4, params) {
                    self.queue
                        .push_back(frame(*device, *feature, *function_byte, &answer));
                } else {
                    self.queue
                        .push_back(frame(*device, 0xFF, *feature, &[*function_byte, 7]));
                }
            }
            None if self.receiver => {
                // HID++ 1.0 error: sub id (our feature index), address (our
                // function byte), then the code.
                self.queue
                    .push_back(frame(*device, 0x8F, *feature, &[*function_byte, 8]));
            }
            None => {}
        }
        Ok(())
    }

    fn read(&mut self, _timeout: Duration) -> Result<Option<Vec<u8>>, ChannelError> {
        Ok(self.queue.pop_front())
    }
}

/// A channel that replays raw replies, whatever is written: for the cases a
/// well-behaved fake cannot produce (short frames, floods, read errors).
#[derive(Debug, Default)]
pub struct ScriptedChannel {
    replies: VecDeque<Result<Option<Vec<u8>>, ChannelError>>,
    /// Frame returned forever once `replies` is empty.
    flood: Option<Vec<u8>>,
}

impl ScriptedChannel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn then_frame(mut self, frame: Vec<u8>) -> Self {
        self.replies.push_back(Ok(Some(frame)));
        self
    }

    pub fn then_silence(mut self) -> Self {
        self.replies.push_back(Ok(None));
        self
    }

    pub fn then_error(mut self, message: &str) -> Self {
        self.replies
            .push_back(Err(ChannelError(message.to_owned())));
        self
    }

    pub fn then_flood(mut self, frame: Vec<u8>) -> Self {
        self.flood = Some(frame);
        self
    }
}

impl Channel for ScriptedChannel {
    fn write(&mut self, _report: &[u8]) -> Result<(), ChannelError> {
        Ok(())
    }

    fn read(&mut self, _timeout: Duration) -> Result<Option<Vec<u8>>, ChannelError> {
        match self.replies.pop_front() {
            Some(reply) => reply,
            None => Ok(self.flood.clone()),
        }
    }
}
