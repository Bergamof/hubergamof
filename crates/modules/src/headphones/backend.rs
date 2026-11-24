//! What the headphones module needs of the headphones: their ambient sound
//! and whether the link to them is open, whoever keeps them (the manager's
//! hub, a simulation).

use std::fmt::Debug;

use tokio::sync::watch;

/// Highest ambient level the headphones take.
pub const MAX_LEVEL: u8 = 20;

/// What noise cancelling and ambient sound do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Outside noise cancelled.
    NoiseCancelling,
    /// Outside sound let through.
    Ambient,
    /// Outside sound let through, wind reduced.
    WindReduction,
    /// Neither.
    Off,
}

/// The ambient sound setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ambient {
    /// What to do.
    pub mode: Mode,
    /// How much outside sound, from 0 to [`MAX_LEVEL`].
    pub level: u8,
    /// Whether voices are let through.
    pub focus_on_voice: bool,
}

/// How the settings channel of the headphones is linked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// Not opened yet: nobody asked for it.
    Idle,
    /// Opening.
    Connecting,
    /// Open: the headphones can be set.
    Connected,
    /// Closed on purpose.
    Released,
    /// Closed by the headphones or by another application.
    Cut,
}

/// What is known of the headphones the module follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Headset {
    /// The link to their settings channel.
    pub link: Link,
    /// Their ambient sound, once read.
    pub ambient: Option<Ambient>,
    /// Whether they offer [`Mode::WindReduction`].
    pub wind_reduction: bool,
}

/// Why the headphones were not set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HeadsetFault {
    /// No headphones connected, or their link is not open.
    #[error("the headphones are not linked")]
    NotLinked,
    /// The request was not taken (too many waiting, value refused).
    #[error("the headphones did not take the request")]
    Refused,
}

/// The headphones, as the module sees them.
pub trait HeadsetControl: Send + Sync + Debug {
    /// What is known of the headphones, as it changes; `None` while none is
    /// connected.
    fn watch(&self) -> watch::Receiver<Option<Headset>>;

    /// Asks for an ambient sound; the headphones confirm it through
    /// [`HeadsetControl::watch`].
    ///
    /// # Errors
    /// [`HeadsetFault`] when the request cannot be made.
    fn set_ambient(&self, ambient: Ambient) -> Result<(), HeadsetFault>;

    /// Opens the link if nobody opened it yet; a link closed on purpose or
    /// cut is left alone (the person resumes it on the Headphones page).
    fn open(&self);
}

/// Nothing to talk to: `--no-headsets`, or a system without them.
#[derive(Debug)]
pub struct NoHeadset(watch::Sender<Option<Headset>>);

impl Default for NoHeadset {
    fn default() -> Self {
        Self(watch::channel(None).0)
    }
}

impl HeadsetControl for NoHeadset {
    fn watch(&self) -> watch::Receiver<Option<Headset>> {
        self.0.subscribe()
    }

    fn set_ambient(&self, _ambient: Ambient) -> Result<(), HeadsetFault> {
        Err(HeadsetFault::NotLinked)
    }

    fn open(&self) {}
}
