//! A scripted HID++ device for the tests: answers the getters the probe sends.

use std::collections::VecDeque;
use std::time::Duration;

use crate::channel::{Channel, ChannelError};
use crate::features::{DEVICE_NAME, FEATURE_SET, REPROG_CONTROLS_V4};
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
}

impl FakeDevice {
    pub fn new() -> Self {
        Self {
            features: Vec::new(),
            name: String::new(),
            controls: Vec::new(),
            broken: Vec::new(),
        }
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
    fn answer(&self, index: u8, function: u8, params: &[u8]) -> Option<Vec<u8>> {
        let feature = self.feature_at(index)?;
        if self.broken.contains(&feature) {
            return None;
        }
        match (feature, function) {
            (0, 0) => {
                let wanted = u16::from_be_bytes([*params.first()?, *params.get(1)?]);
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
        match self.devices.iter().find(|(index, _)| index == device) {
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
