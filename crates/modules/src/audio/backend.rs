//! Access to the audio server, behind a trait so that the module runs on the
//! real server (`pactl`) or on a simulated one (`--demo`, tests).

use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use super::state::{AudioSnapshot, Direction};

/// A change requested from the audio server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioChange {
    /// Adds `delta` (PulseAudio units) to every channel of output `sink`.
    SinkVolume {
        /// Output name.
        sink: String,
        /// Change of every channel.
        delta: i64,
    },
    /// Mutes or unmutes a device.
    Mute {
        /// Output or input.
        direction: Direction,
        /// Device name.
        device: String,
        /// New state.
        muted: bool,
    },
    /// Makes a device the default one of its direction.
    Default {
        /// Output or input.
        direction: Direction,
        /// Device name.
        device: String,
    },
}

impl AudioChange {
    /// `self` followed by `next` as a single change, when they combine:
    /// volume changes of the same output add up, a later mute or default
    /// selection of the same kind replaces an earlier one.
    #[must_use]
    pub fn merged_with(&self, next: &Self) -> Option<Self> {
        match (self, next) {
            (
                Self::SinkVolume { sink, delta },
                Self::SinkVolume {
                    sink: next_sink,
                    delta: next_delta,
                },
            ) if sink == next_sink => Some(Self::SinkVolume {
                sink: sink.clone(),
                delta: delta.saturating_add(*next_delta),
            }),
            (
                Self::Mute {
                    direction, device, ..
                },
                Self::Mute {
                    direction: next_direction,
                    device: next_device,
                    ..
                },
            ) if direction == next_direction && device == next_device => Some(next.clone()),
            (
                Self::Default { direction, .. },
                Self::Default {
                    direction: next_direction,
                    ..
                },
            ) if direction == next_direction => Some(next.clone()),
            _ => None,
        }
    }

    /// Whether the change does nothing (a volume change of 0).
    #[must_use]
    pub fn is_noop(&self) -> bool {
        matches!(self, Self::SinkVolume { delta: 0, .. })
    }
}

/// Failure to talk to the audio server.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AudioError {
    /// The audio tool cannot be started (not installed…).
    #[error("cannot run `{program}`: {source}")]
    Spawn {
        /// Program run.
        program: String,
        /// Why it could not start.
        source: std::io::Error,
    },
    /// The audio tool failed.
    #[error("`{command}` failed ({status}): {stderr}")]
    Failed {
        /// Sub-command run.
        command: String,
        /// Exit status.
        status: String,
        /// What it reported.
        stderr: String,
    },
    /// The audio tool did not answer in time.
    #[error("`{command}` did not answer within {timeout:?}")]
    Timeout {
        /// Sub-command run.
        command: String,
        /// Time given.
        timeout: Duration,
    },
    /// The audio tool printed something unexpected.
    #[error("unexpected output of `{command}`: {reason}")]
    InvalidOutput {
        /// Sub-command run.
        command: String,
        /// What is wrong.
        reason: String,
    },
    /// The (simulated) audio server is unavailable.
    #[error("audio server unavailable")]
    Unavailable,
}

/// Notifications of changes made on the audio server, by anyone.
#[async_trait]
pub trait AudioChanges: Send {
    /// Waits for the next change of devices, volumes or defaults; `false`
    /// when the notifications ended (the server or the tool stopped).
    async fn changed(&mut self) -> bool;
}

/// The audio server.
///
/// Calls may take a while (a process per call): the module only makes them
/// from its background tasks.
#[async_trait]
pub trait AudioBackend: Send + Sync {
    /// Reads the devices and the defaults.
    async fn snapshot(&self) -> Result<AudioSnapshot, AudioError>;

    /// Applies `change`.
    async fn apply(&self, change: &AudioChange) -> Result<(), AudioError>;

    /// Starts listening to changes.
    async fn subscribe(&self) -> Result<Box<dyn AudioChanges>, AudioError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(sink: &str, delta: i64) -> AudioChange {
        AudioChange::SinkVolume {
            sink: sink.into(),
            delta,
        }
    }

    fn mute(direction: Direction, muted: bool) -> AudioChange {
        AudioChange::Mute {
            direction,
            device: "d".into(),
            muted,
        }
    }

    fn default(direction: Direction, device: &str) -> AudioChange {
        AudioChange::Default {
            direction,
            device: device.into(),
        }
    }

    #[test]
    fn volume_changes_of_one_output_add_up() {
        assert_eq!(
            volume("a", 3).merged_with(&volume("a", -5)),
            Some(volume("a", -2))
        );
        assert_eq!(volume("a", 3).merged_with(&volume("b", 1)), None);
        assert!(volume("a", 0).is_noop());
        assert!(!volume("a", 1).is_noop());
    }

    #[test]
    fn later_mutes_and_defaults_replace_earlier_ones() {
        let (output, input) = (Direction::Output, Direction::Input);
        assert_eq!(
            mute(output, true).merged_with(&mute(output, false)),
            Some(mute(output, false))
        );
        assert_eq!(mute(output, true).merged_with(&mute(input, false)), None);
        assert_eq!(
            default(output, "a").merged_with(&default(output, "b")),
            Some(default(output, "b"))
        );
        assert_eq!(default(output, "a").merged_with(&default(input, "b")), None);
        assert_eq!(volume("d", 1).merged_with(&mute(output, true)), None);
    }
}
