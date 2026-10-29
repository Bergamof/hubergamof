//! What the audio server reports: devices, their volume and the defaults.

/// Volume of a channel at 100 % (`PA_VOLUME_NORM`), in PulseAudio units.
pub const VOLUME_NORM: u32 = 0x1_0000;

/// Hundred, to convert between ratios and percentages.
const PERCENT: u64 = 100;

/// Volume of every channel of a device, in PulseAudio units (linear,
/// [`VOLUME_NORM`] = 100 %).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Volume {
    channels: Vec<u32>,
}

impl Volume {
    /// Volume of these channels.
    #[must_use]
    pub fn new(channels: Vec<u32>) -> Self {
        Self { channels }
    }

    /// Every channel at `level`.
    #[must_use]
    pub fn uniform(level: u32, channel_count: usize) -> Self {
        Self::new(vec![level; channel_count])
    }

    /// Volume of each channel.
    #[must_use]
    pub fn channels(&self) -> &[u32] {
        &self.channels
    }

    /// Mean of the channels, the displayed volume (0 without channel).
    #[must_use]
    pub fn average(&self) -> u32 {
        let Ok(count) = u64::try_from(self.channels.len()) else {
            return 0;
        };
        if count == 0 {
            return 0;
        }
        let sum: u64 = self.channels.iter().copied().map(u64::from).sum();
        // The mean of `u32` values fits in a `u32`.
        u32::try_from(sum / count).unwrap_or(u32::MAX)
    }

    /// The displayed volume, as a rounded percentage.
    #[must_use]
    pub fn percent(&self) -> u32 {
        percent_of(self.average())
    }

    /// Clamps the change `delta` so that the loudest channel stays at or
    /// below `ceiling` and the quietest at or above 0: the channels move
    /// together and keep their balance. A volume already above `ceiling`
    /// (set elsewhere) is never raised, nor lowered by this clamp.
    #[must_use]
    pub fn clamp_change(&self, delta: i64, ceiling: u32) -> i64 {
        let loudest = self.channels.iter().copied().max().unwrap_or(0);
        let quietest = self.channels.iter().copied().min().unwrap_or(0);
        if delta > 0 {
            delta.min(i64::from(ceiling.saturating_sub(loudest)))
        } else {
            delta.max(-i64::from(quietest))
        }
    }

    /// The volume with `delta` added to every channel, within `0..=u32::MAX`.
    #[must_use]
    pub fn shifted(&self, delta: i64) -> Self {
        let channels = self
            .channels
            .iter()
            .map(|&level| {
                let shifted = (i64::from(level) + delta).clamp(0, i64::from(u32::MAX));
                u32::try_from(shifted).unwrap_or(u32::MAX)
            })
            .collect();
        Self { channels }
    }
}

/// `level` (PulseAudio units) as a rounded percentage.
#[must_use]
pub fn percent_of(level: u32) -> u32 {
    let norm = u64::from(VOLUME_NORM);
    let percent = (u64::from(level) * PERCENT + norm / 2) / norm;
    u32::try_from(percent).unwrap_or(u32::MAX)
}

/// `percent` % in PulseAudio units, rounded.
#[must_use]
pub fn level_of_percent(percent: u32) -> u32 {
    let level = (u64::from(percent) * u64::from(VOLUME_NORM) + PERCENT / 2) / PERCENT;
    u32::try_from(level).unwrap_or(u32::MAX)
}

/// An output (sink) or input (source) of the audio server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    /// Identifier given to the audio server.
    pub name: String,
    /// Human-readable name.
    pub description: String,
    /// Channel volumes.
    pub volume: Volume,
    /// Whether the device is muted.
    pub muted: bool,
}

/// Outputs, inputs and defaults of the audio server.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioSnapshot {
    /// Outputs, in server order.
    pub sinks: Vec<AudioDevice>,
    /// Inputs, in server order, without the monitors of the outputs.
    pub sources: Vec<AudioDevice>,
    /// Name of the default output.
    pub default_sink: Option<String>,
    /// Name of the default input.
    pub default_source: Option<String>,
}

/// Which list of devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Outputs (sinks).
    Output,
    /// Inputs (sources).
    Input,
}

impl AudioSnapshot {
    /// Devices of `direction`.
    #[must_use]
    pub fn devices(&self, direction: Direction) -> &[AudioDevice] {
        match direction {
            Direction::Output => &self.sinks,
            Direction::Input => &self.sources,
        }
    }

    /// Name of the default device of `direction`.
    #[must_use]
    pub fn default_name(&self, direction: Direction) -> Option<&str> {
        match direction {
            Direction::Output => self.default_sink.as_deref(),
            Direction::Input => self.default_source.as_deref(),
        }
    }

    /// The default device of `direction`, if it exists.
    #[must_use]
    pub fn default_device(&self, direction: Direction) -> Option<&AudioDevice> {
        let name = self.default_name(direction)?;
        self.devices(direction)
            .iter()
            .find(|device| device.name == name)
    }

    /// Mutable access to the default device of `direction`.
    pub fn default_device_mut(&mut self, direction: Direction) -> Option<&mut AudioDevice> {
        let name = self.default_name(direction)?.to_owned();
        self.devices_mut(direction)
            .iter_mut()
            .find(|device| device.name == name)
    }

    /// Mutable access to the device of `direction` named `name`.
    pub fn device_mut(&mut self, direction: Direction, name: &str) -> Option<&mut AudioDevice> {
        self.devices_mut(direction)
            .iter_mut()
            .find(|device| device.name == name)
    }

    /// Makes `name` the default device of `direction`.
    pub fn set_default(&mut self, direction: Direction, name: &str) {
        let default = Some(name.to_owned());
        match direction {
            Direction::Output => self.default_sink = default,
            Direction::Input => self.default_source = default,
        }
    }

    fn devices_mut(&mut self, direction: Direction) -> &mut Vec<AudioDevice> {
        match direction {
            Direction::Output => &mut self.sinks,
            Direction::Input => &mut self.sources,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIVE_PERCENT: i64 = 3277;

    #[test]
    fn percentages_round_to_the_nearest() {
        assert_eq!(percent_of(VOLUME_NORM), 100);
        assert_eq!(percent_of(16_384), 25);
        assert_eq!(percent_of(46_260), 71);
        assert_eq!(percent_of(0), 0);
        assert_eq!(level_of_percent(5), 3277);
        assert_eq!(level_of_percent(100), VOLUME_NORM);
        assert_eq!(level_of_percent(150), 98_304);
    }

    #[test]
    fn the_displayed_volume_is_the_mean_of_the_channels() {
        assert_eq!(Volume::new(vec![16_384, 32_768]).average(), 24_576);
        assert_eq!(Volume::new(vec![16_384, 32_768]).percent(), 38);
        assert_eq!(Volume::uniform(VOLUME_NORM, 1).percent(), 100);
        assert_eq!(Volume::default().average(), 0);
    }

    #[test]
    fn changes_stop_at_the_ceiling_and_at_zero() {
        let ceiling = VOLUME_NORM;
        let volume = Volume::uniform(level_of_percent(97), 2);
        assert_eq!(
            volume.clamp_change(FIVE_PERCENT, ceiling),
            i64::from(ceiling - level_of_percent(97))
        );
        assert_eq!(volume.clamp_change(-FIVE_PERCENT, ceiling), -FIVE_PERCENT);
        let quiet = Volume::uniform(1000, 2);
        assert_eq!(quiet.clamp_change(-FIVE_PERCENT, ceiling), -1000);
        assert_eq!(
            Volume::default().clamp_change(FIVE_PERCENT, ceiling),
            FIVE_PERCENT
        );
    }

    #[test]
    fn unequal_channels_keep_their_gap_and_the_loudest_caps() {
        let ceiling = VOLUME_NORM;
        let volume = Volume::new(vec![50_000, 60_000]);
        let delta = volume.clamp_change(10_000, ceiling);
        assert_eq!(delta, 5536);
        let raised = volume.shifted(delta);
        assert_eq!(raised.channels(), [55_536, VOLUME_NORM]);
        let lowered = volume.shifted(volume.clamp_change(-55_000, ceiling));
        assert_eq!(lowered.channels(), [0, 10_000]);
    }

    #[test]
    fn a_volume_above_the_ceiling_is_not_raised_but_can_be_lowered() {
        let volume = Volume::uniform(level_of_percent(140), 2);
        assert_eq!(volume.clamp_change(FIVE_PERCENT, VOLUME_NORM), 0);
        assert_eq!(
            volume.clamp_change(-FIVE_PERCENT, VOLUME_NORM),
            -FIVE_PERCENT
        );
    }

    #[test]
    fn defaults_are_found_by_name() {
        let device = |name: &str| AudioDevice {
            name: name.into(),
            description: name.into(),
            volume: Volume::default(),
            muted: false,
        };
        let mut snapshot = AudioSnapshot {
            sinks: vec![device("a"), device("b")],
            sources: vec![device("mic")],
            default_sink: Some("b".into()),
            default_source: Some("gone".into()),
        };
        assert_eq!(
            snapshot
                .default_device(Direction::Output)
                .map(|d| d.name.as_str()),
            Some("b")
        );
        assert!(snapshot.default_device(Direction::Input).is_none());
        snapshot.set_default(Direction::Input, "mic");
        assert!(snapshot.default_device_mut(Direction::Input).is_some());
        assert!(snapshot.device_mut(Direction::Output, "a").is_some());
        assert!(snapshot.device_mut(Direction::Output, "mic").is_none());
    }
}
