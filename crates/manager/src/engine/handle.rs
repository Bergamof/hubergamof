//! Handle to a running [`super::DeviceEngine`].

use std::sync::atomic::{AtomicU64, Ordering};

use hubergamof_protocol::{DeviceConfig, DeviceInfo};
use tokio::sync::{mpsc, oneshot, watch};

use super::{EngineError, LiveSnapshot};
use crate::device::{DeviceError, InputEvent};

/// Capacity of the request queue of an engine.
pub(crate) const REQUEST_CAPACITY: usize = 32;

/// Request sent to an engine.
pub(crate) enum Request {
    Input(InputEvent, oneshot::Sender<Result<(), EngineError>>),
    ApplyConfig(
        Box<DeviceConfig>,
        oneshot::Sender<Result<DeviceConfig, EngineError>>,
    ),
    Stop(oneshot::Sender<()>),
}

/// Controls a device engine and observes it. Cheap to clone.
#[derive(Clone)]
pub struct EngineHandle {
    id: u64,
    info: DeviceInfo,
    requests: mpsc::Sender<Request>,
    live: watch::Receiver<LiveSnapshot>,
    config: watch::Receiver<DeviceConfig>,
}

impl EngineHandle {
    pub(crate) fn new(
        info: DeviceInfo,
        requests: mpsc::Sender<Request>,
        live: watch::Receiver<LiveSnapshot>,
        config: watch::Receiver<DeviceConfig>,
    ) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            info,
            requests,
            live,
            config,
        }
    }

    /// Identifies the engine: a device plugged in again gets a new engine.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The device driven by the engine.
    #[must_use]
    pub fn info(&self) -> &DeviceInfo {
        &self.info
    }

    /// Live state, updated after every visible change.
    #[must_use]
    pub fn live(&self) -> watch::Receiver<LiveSnapshot> {
        self.live.clone()
    }

    /// Current configuration, updated after every accepted change.
    #[must_use]
    pub fn config(&self) -> watch::Receiver<DeviceConfig> {
        self.config.clone()
    }

    /// Simulates a press of key `index`, handled like a hardware press; returns
    /// once the device is redrawn.
    ///
    /// # Errors
    ///
    /// [`DeviceError::InvalidKey`] (wrapped) or [`EngineError::Stopped`].
    pub async fn press_key(&self, index: u8) -> Result<(), EngineError> {
        let count = self.info.capabilities.key_count();
        if usize::from(index) >= count {
            return Err(DeviceError::InvalidKey { index, count }.into());
        }
        self.input(InputEvent::KeyDown(index)).await
    }

    /// Simulates turning dial `index` by `ticks` steps (positive clockwise).
    ///
    /// # Errors
    ///
    /// [`DeviceError::InvalidDial`] (wrapped) or [`EngineError::Stopped`].
    pub async fn rotate_dial(&self, index: u8, ticks: i32) -> Result<(), EngineError> {
        self.check_dial(index)?;
        self.input(InputEvent::DialRotate { index, ticks }).await
    }

    /// Simulates a press of dial `index`.
    ///
    /// # Errors
    ///
    /// [`DeviceError::InvalidDial`] (wrapped) or [`EngineError::Stopped`].
    pub async fn press_dial(&self, index: u8) -> Result<(), EngineError> {
        self.check_dial(index)?;
        self.input(InputEvent::DialDown(index)).await
    }

    /// Saves `config` (which must carry the current revision), applies it and
    /// returns it with its new revision.
    ///
    /// # Errors
    ///
    /// [`EngineError::Config`] (stale revision, invalid configuration, I/O)
    /// or [`EngineError::Stopped`].
    pub async fn apply_config(&self, config: DeviceConfig) -> Result<DeviceConfig, EngineError> {
        let (reply, response) = oneshot::channel();
        self.send(Request::ApplyConfig(Box::new(config), reply))
            .await?;
        response.await.map_err(|_| EngineError::Stopped)?
    }

    /// Stops the engine: modules are destroyed and the screens blanked.
    /// Returns once done, immediately if the engine already stopped.
    pub async fn stop(&self) {
        let (reply, done) = oneshot::channel();
        if self.send(Request::Stop(reply)).await.is_ok() {
            let _ = done.await;
        }
    }

    fn check_dial(&self, index: u8) -> Result<(), EngineError> {
        let count = self.info.capabilities.dial_count;
        if index < count {
            Ok(())
        } else {
            Err(DeviceError::InvalidDial { index, count }.into())
        }
    }

    async fn input(&self, event: InputEvent) -> Result<(), EngineError> {
        let (reply, response) = oneshot::channel();
        self.send(Request::Input(event, reply)).await?;
        response.await.map_err(|_| EngineError::Stopped)?
    }

    async fn send(&self, request: Request) -> Result<(), EngineError> {
        self.requests
            .send(request)
            .await
            .map_err(|_| EngineError::Stopped)
    }
}
