//! Where headphones come from: BlueZ for the real ones, or simulated ones for
//! `--demo` and the tests.

use std::sync::Arc;

use async_trait::async_trait;
use hubergamof_headset::bluez::{BluezHeadsets, HeadsetDevice};
use hubergamof_headset::channel::Channel;
use hubergamof_headset::simulated::SimulatedHeadset;
use hubergamof_protocol::HeadsetInfo;
use tokio::sync::Mutex;

/// The headphones a manager can serve.
#[async_trait]
pub trait HeadsetSource: Send + Sync + 'static {
    /// The headphones connected to this computer now.
    async fn present(&self) -> Vec<HeadsetInfo>;

    /// Opens the settings channel of the headphones at `address`.
    ///
    /// # Errors
    /// Why the channel could not be opened, as text for the log (never the
    /// address: it identifies a device).
    async fn open(&self, address: &str) -> Result<Box<dyn Channel + Send>, String>;
}

/// The Sony headphones BlueZ lists as connected.
#[derive(Default)]
pub struct BluezSource {
    /// Reached at the first look, and again after a failure: BlueZ may start
    /// after this manager.
    bluez: Mutex<Option<Arc<BluezHeadsets>>>,
}

impl BluezSource {
    async fn bluez(&self) -> Result<Arc<BluezHeadsets>, String> {
        let mut bluez = self.bluez.lock().await;
        if let Some(bluez) = bluez.as_ref() {
            return Ok(Arc::clone(bluez));
        }
        let started = Arc::new(
            BluezHeadsets::start()
                .await
                .map_err(|error| error.to_string())?,
        );
        *bluez = Some(Arc::clone(&started));
        Ok(started)
    }

    async fn connected(&self) -> Result<Vec<HeadsetDevice>, String> {
        let devices = self.bluez().await?.devices().await;
        match devices {
            Ok(devices) => Ok(devices.into_iter().filter(|d| d.connected).collect()),
            Err(error) => {
                // Reached again at the next look.
                *self.bluez.lock().await = None;
                Err(error.to_string())
            }
        }
    }
}

#[async_trait]
impl HeadsetSource for BluezSource {
    async fn present(&self) -> Vec<HeadsetInfo> {
        match self.connected().await {
            Ok(devices) => devices
                .into_iter()
                .map(|device| HeadsetInfo {
                    address: device.address,
                    name: device.name,
                })
                .collect(),
            Err(error) => {
                // At every look while Bluetooth is off: not a warning.
                tracing::debug!(%error, "cannot look for headphones");
                Vec::new()
            }
        }
    }

    async fn open(&self, address: &str) -> Result<Box<dyn Channel + Send>, String> {
        let device = self
            .connected()
            .await?
            .into_iter()
            .find(|device| device.address == address)
            .ok_or("the headphones are not connected")?;
        let channel = self
            .bluez()
            .await?
            .connect(&device)
            .await
            .map_err(|error| error.to_string())?;
        Ok(Box::new(channel))
    }
}

/// Simulated headphones, for `--demo` and the tests.
pub struct SimulatedSource {
    headsets: std::sync::Mutex<Vec<(HeadsetInfo, SimulatedHeadset)>>,
}

impl SimulatedSource {
    /// A source of simulated headphones, all connected.
    #[must_use]
    pub fn new(headsets: Vec<(HeadsetInfo, SimulatedHeadset)>) -> Self {
        Self {
            headsets: std::sync::Mutex::new(headsets),
        }
    }

    /// Makes the headphones at `address` leave, to bring them back with
    /// [`SimulatedSource::attach`].
    #[cfg(test)]
    pub(crate) fn detach(&self, address: &str) -> Option<(HeadsetInfo, SimulatedHeadset)> {
        let mut headsets = self.lock();
        let index = headsets
            .iter()
            .position(|(info, _)| info.address == address)?;
        Some(headsets.remove(index))
    }

    /// Lists simulated headphones as connected.
    #[cfg(test)]
    pub(crate) fn attach(&self, headset: (HeadsetInfo, SimulatedHeadset)) {
        self.lock().push(headset);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(HeadsetInfo, SimulatedHeadset)>> {
        // Plain data: nothing to recover from a poisoned lock.
        self.headsets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[async_trait]
impl HeadsetSource for SimulatedSource {
    async fn present(&self) -> Vec<HeadsetInfo> {
        self.lock().iter().map(|(info, _)| info.clone()).collect()
    }

    async fn open(&self, address: &str) -> Result<Box<dyn Channel + Send>, String> {
        let headsets = self.lock();
        let (_, headset) = headsets
            .iter()
            .find(|(info, _)| info.address == address)
            .ok_or("the headphones are not connected")?;
        Ok(Box::new(headset.reconnect()))
    }
}
