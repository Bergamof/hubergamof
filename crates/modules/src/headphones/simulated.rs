//! Simulated headphones for the tests and the services of `--demo`.

use std::sync::{Mutex, PoisonError};

use tokio::sync::watch;

use super::backend::{Ambient, Headset, HeadsetControl, HeadsetFault, Link, Mode};

/// Headphones that confirm every request at once.
#[derive(Debug)]
pub struct SimulatedHeadset {
    state: watch::Sender<Option<Headset>>,
    requests: Mutex<Vec<Ambient>>,
    opened: Mutex<u32>,
}

impl SimulatedHeadset {
    /// Connected headphones with noise cancelling on.
    #[must_use]
    pub fn demo() -> Self {
        Self::with(Some(Headset {
            link: Link::Connected,
            ambient: Some(Ambient {
                mode: Mode::NoiseCancelling,
                level: 10,
                focus_on_voice: false,
            }),
        }))
    }

    /// Headphones in `state`, or none.
    #[must_use]
    pub fn with(state: impl Into<Option<Headset>>) -> Self {
        Self {
            state: watch::channel(state.into()).0,
            requests: Mutex::default(),
            opened: Mutex::default(),
        }
    }

    /// Replaces what the headphones say, as a change they made themselves.
    pub fn say(&self, state: impl Into<Option<Headset>>) {
        self.state.send_replace(state.into());
    }

    /// The ambient sounds asked for, in order.
    #[must_use]
    pub fn requests(&self) -> Vec<Ambient> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// How many times the link was asked to open.
    #[must_use]
    pub fn opened(&self) -> u32 {
        *self.opened.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl HeadsetControl for SimulatedHeadset {
    fn watch(&self) -> watch::Receiver<Option<Headset>> {
        self.state.subscribe()
    }

    fn set_ambient(&self, ambient: Ambient) -> Result<(), HeadsetFault> {
        let linked = self
            .state
            .borrow()
            .as_ref()
            .is_some_and(|headset| headset.link == Link::Connected);
        if !linked {
            return Err(HeadsetFault::NotLinked);
        }
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ambient);
        self.state.send_modify(|state| {
            if let Some(headset) = state {
                headset.ambient = Some(ambient);
            }
        });
        Ok(())
    }

    fn open(&self) {
        *self.opened.lock().unwrap_or_else(PoisonError::into_inner) += 1;
    }
}
