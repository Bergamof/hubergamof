//! Access to the audio server, behind a trait so that the module runs on the
//! real server (`pactl`) or on a simulated one (`--demo`, tests).

use std::fmt;
use std::ops::BitOr;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use super::state::{Direction, PartialSnapshot, Volume};

/// A change requested from the audio server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioChange {
    /// Sets the volume of every channel of output `sink`. Absolute rather
    /// than relative: a relative change applied to a volume changed
    /// elsewhere meanwhile could go beyond the ceiling of the dial.
    SinkVolume {
        /// Output name.
        sink: String,
        /// New volume of each channel.
        volume: Volume,
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
    /// `self` followed by `next` as a single change, when the later one
    /// makes the earlier one pointless: a volume of the same output, a mute
    /// of the same device, a default of the same direction.
    pub(super) fn merged_with(&self, next: &Self) -> Option<Self> {
        let replaces = match (self, next) {
            (
                Self::SinkVolume { sink, .. },
                Self::SinkVolume {
                    sink: next_sink, ..
                },
            ) => sink == next_sink,
            (
                Self::Mute {
                    direction, device, ..
                },
                Self::Mute {
                    direction: next_direction,
                    device: next_device,
                    ..
                },
            ) => direction == next_direction && device == next_device,
            (
                Self::Default { direction, .. },
                Self::Default {
                    direction: next_direction,
                    ..
                },
            ) => direction == next_direction,
            _ => false,
        };
        replaces.then(|| next.clone())
    }

    /// What the change alters, to read again once it is applied.
    pub(super) fn scope(&self) -> ReadScope {
        match self {
            Self::SinkVolume { .. }
            | Self::Mute {
                direction: Direction::Output,
                ..
            } => ReadScope::OUTPUTS,
            Self::Mute {
                direction: Direction::Input,
                ..
            } => ReadScope::INPUTS,
            Self::Default { .. } => ReadScope::DEFAULTS,
        }
    }
}

/// Parts of the server state to read: each costs a call to the server
/// (a `pactl` process), so only those that changed are read again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent parts, combined with `|`"
)]
pub struct ReadScope {
    /// The outputs (sinks) and their volumes.
    pub outputs: bool,
    /// The inputs (sources) and their volumes.
    pub inputs: bool,
    /// The default output and input.
    pub defaults: bool,
}

impl ReadScope {
    /// Nothing to read.
    pub const NONE: Self = Self {
        outputs: false,
        inputs: false,
        defaults: false,
    };
    /// The outputs only.
    pub const OUTPUTS: Self = Self {
        outputs: true,
        ..Self::NONE
    };
    /// The inputs only.
    pub const INPUTS: Self = Self {
        inputs: true,
        ..Self::NONE
    };
    /// The defaults only.
    pub const DEFAULTS: Self = Self {
        defaults: true,
        ..Self::NONE
    };
    /// Everything.
    pub const ALL: Self = Self {
        outputs: true,
        inputs: true,
        defaults: true,
    };

    /// Whether there is nothing to read.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self == Self::NONE
    }
}

impl BitOr for ReadScope {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self {
            outputs: self.outputs || other.outputs,
            inputs: self.inputs || other.inputs,
            defaults: self.defaults || other.defaults,
        }
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
        /// What it reported (truncated).
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
    /// The backend panicked (a bug): the call is treated as failed.
    #[error("the audio backend panicked")]
    Panicked,
    /// The (simulated) audio server is unavailable.
    #[error("audio server unavailable")]
    Unavailable,
}

/// Notifications of changes made on the audio server, by anyone.
#[async_trait]
pub trait AudioChanges: Send {
    /// Waits for the next change of devices, volumes or defaults and tells
    /// what to read again; `None` when the notifications ended (the server
    /// or the tool stopped). Cancel-safe: no notification is lost when the
    /// future is dropped before completing.
    async fn next_change(&mut self) -> Option<ReadScope>;
}

/// The audio server.
///
/// Calls may take a while (a process per call): the module only makes them
/// from its background tasks.
#[async_trait]
pub trait AudioBackend: Send + Sync + fmt::Debug {
    /// Reads the parts of the state in `scope`.
    async fn read(&self, scope: ReadScope) -> Result<PartialSnapshot, AudioError>;

    /// Applies `change`.
    async fn apply(&self, change: &AudioChange) -> Result<(), AudioError>;

    /// Starts listening to changes.
    async fn subscribe(&self) -> Result<Box<dyn AudioChanges>, AudioError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(sink: &str, level: u32) -> AudioChange {
        AudioChange::SinkVolume {
            sink: sink.into(),
            volume: Volume::uniform(level, 2),
        }
    }

    fn mute(direction: Direction, device: &str, muted: bool) -> AudioChange {
        AudioChange::Mute {
            direction,
            device: device.into(),
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
    fn a_later_volume_of_the_same_output_replaces_the_earlier_one() {
        assert_eq!(
            volume("a", 3).merged_with(&volume("a", 5)),
            Some(volume("a", 5))
        );
        assert_eq!(volume("a", 3).merged_with(&volume("b", 1)), None);
    }

    #[test]
    fn later_mutes_and_defaults_replace_earlier_ones() {
        let (output, input) = (Direction::Output, Direction::Input);
        assert_eq!(
            mute(output, "d", true).merged_with(&mute(output, "d", false)),
            Some(mute(output, "d", false))
        );
        assert_eq!(
            mute(output, "d", true).merged_with(&mute(input, "d", false)),
            None
        );
        // Two devices of the same direction are muted separately.
        assert_eq!(
            mute(input, "a", true).merged_with(&mute(input, "b", true)),
            None
        );
        assert_eq!(
            default(output, "a").merged_with(&default(output, "b")),
            Some(default(output, "b"))
        );
        assert_eq!(default(output, "a").merged_with(&default(input, "b")), None);
        assert_eq!(volume("d", 1).merged_with(&mute(output, "d", true)), None);
    }

    #[test]
    fn changes_name_what_to_read_again() {
        assert_eq!(volume("a", 1).scope(), ReadScope::OUTPUTS);
        assert_eq!(
            mute(Direction::Output, "a", true).scope(),
            ReadScope::OUTPUTS
        );
        assert_eq!(mute(Direction::Input, "a", true).scope(), ReadScope::INPUTS);
        assert_eq!(default(Direction::Input, "a").scope(), ReadScope::DEFAULTS);
    }

    #[test]
    fn scopes_combine() {
        assert!(ReadScope::NONE.is_empty());
        assert!(!ReadScope::INPUTS.is_empty());
        assert_eq!(
            ReadScope::OUTPUTS | ReadScope::INPUTS | ReadScope::DEFAULTS,
            ReadScope::ALL
        );
        assert_eq!(ReadScope::NONE | ReadScope::INPUTS, ReadScope::INPUTS);
    }
}
