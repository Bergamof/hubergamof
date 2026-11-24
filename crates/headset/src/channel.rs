//! The byte stream to the headphones.

use std::time::Duration;

use thiserror::Error;

/// A connected RFCOMM stream: writes bytes and reads what the headphones send.
pub trait Channel {
    /// Writes all of `bytes`.
    ///
    /// # Errors
    /// [`ChannelError`] when the stream refuses the write.
    fn write(&mut self, bytes: &[u8]) -> Result<(), ChannelError>;

    /// The next bytes from the headphones, `None` when none arrive within
    /// `timeout`. The cut between two reads means nothing: frames may be split
    /// or joined.
    ///
    /// # Errors
    /// [`ChannelError`] when the stream cannot be read or the headphones
    /// closed it.
    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, ChannelError>;
}

/// The stream could not be written or read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChannelError {
    /// The headphones closed the connection (another app took the settings
    /// channel, or they went away).
    #[error("the headphones closed the connection")]
    Closed,
    /// The stream failed.
    #[error("{0}")]
    Io(String),
}

impl<C: Channel + ?Sized> Channel for &mut C {
    fn write(&mut self, bytes: &[u8]) -> Result<(), ChannelError> {
        (**self).write(bytes)
    }

    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, ChannelError> {
        (**self).read(timeout)
    }
}

impl<C: Channel + ?Sized> Channel for Box<C> {
    fn write(&mut self, bytes: &[u8]) -> Result<(), ChannelError> {
        (**self).write(bytes)
    }

    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, ChannelError> {
        (**self).read(timeout)
    }
}
