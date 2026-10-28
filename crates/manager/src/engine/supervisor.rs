//! Starts an engine for every attached device.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{oneshot, watch};
use tokio::task::{JoinHandle, JoinSet};

use super::{DeviceEngine, EngineHandle, EngineServices};
use crate::device::{DeckBackend, DeviceDiscovery, DiscoveryEvent};

/// Longest wait for the engines to stop on shutdown.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Engines currently running, by device serial. Cheap to clone.
#[derive(Clone)]
pub struct EngineRegistry {
    engines: Arc<Mutex<BTreeMap<String, EngineHandle>>>,
    changes: Arc<watch::Sender<()>>,
}

impl Default for EngineRegistry {
    fn default() -> Self {
        Self {
            engines: Arc::default(),
            changes: Arc::new(watch::Sender::new(())),
        }
    }
}

impl EngineRegistry {
    /// Notified whenever an engine starts or stops: a device appears in the
    /// [`DeviceDiscovery`] before its engine is ready, so a consumer waiting
    /// for an engine marks the receiver seen, looks it up, then waits here.
    #[must_use]
    pub fn changes(&self) -> watch::Receiver<()> {
        self.changes.subscribe()
    }

    /// Engine of the device with this serial.
    #[must_use]
    pub fn get(&self, serial: &str) -> Option<EngineHandle> {
        self.lock().get(serial).cloned()
    }

    /// Every running engine, ordered by serial.
    #[must_use]
    pub fn handles(&self) -> Vec<EngineHandle> {
        self.lock().values().cloned().collect()
    }

    fn insert(&self, handle: EngineHandle) {
        self.lock().insert(handle.info().serial.clone(), handle);
        self.changes.send_replace(());
    }

    /// Removes the engine `id` of `serial`, unless a newer engine replaced it.
    fn remove(&self, serial: &str, id: u64) {
        let mut engines = self.lock();
        if engines.get(serial).is_some_and(|handle| handle.id() == id) {
            engines.remove(serial);
            drop(engines);
            self.changes.send_replace(());
        }
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, EngineHandle>> {
        // The map is never left half-updated: a poisoned lock is still valid.
        self.engines.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Runs one engine per attached device: started on attach (configuration
/// reloaded every time), ended when the device disconnects.
pub struct Supervisor {
    engines: EngineRegistry,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Supervisor {
    /// Starts engines for the devices already attached, then follows
    /// `discovery`.
    #[must_use]
    pub fn start(discovery: &DeviceDiscovery, services: EngineServices) -> Self {
        let engines = EngineRegistry::default();
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(supervise(
            discovery.clone(),
            services,
            engines.clone(),
            stopped,
        ));
        Self {
            engines,
            stop,
            task,
        }
    }

    /// Running engines.
    #[must_use]
    pub fn engines(&self) -> EngineRegistry {
        self.engines.clone()
    }

    /// Stops every engine (modules destroyed, screens blanked) and waits for
    /// them, at most [`SHUTDOWN_TIMEOUT`] per engine.
    pub async fn shutdown(self) {
        let _ = self.stop.send(());
        if let Err(error) = self.task.await {
            tracing::warn!(%error, "device supervisor failed");
        }
    }
}

async fn supervise(
    discovery: DeviceDiscovery,
    services: EngineServices,
    engines: EngineRegistry,
    mut stopped: oneshot::Receiver<()>,
) {
    let (initial, mut events) = discovery.subscribe();
    let mut tasks = JoinSet::new();
    for info in initial {
        spawn_engine(&discovery, &services, &engines, &mut tasks, &info.serial);
    }
    loop {
        tokio::select! {
            _ = &mut stopped => break,
            event = events.recv() => match event {
                Ok(DiscoveryEvent::Attached(info)) => {
                    spawn_engine(&discovery, &services, &engines, &mut tasks, &info.serial);
                }
                Ok(DiscoveryEvent::Detached(serial)) => tracing::info!(%serial, "device detached"),
                Err(RecvError::Lagged(missed)) => tracing::warn!(missed, "missed device events"),
                Err(RecvError::Closed) => break,
            },
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
        }
    }
    for handle in engines.handles() {
        if tokio::time::timeout(SHUTDOWN_TIMEOUT, handle.stop())
            .await
            .is_err()
        {
            tracing::warn!(serial = %handle.info().serial, "timed out stopping device engine");
        }
    }
    // Engines still starting are aborted.
    tasks.shutdown().await;
}

fn spawn_engine(
    discovery: &DeviceDiscovery,
    services: &EngineServices,
    engines: &EngineRegistry,
    tasks: &mut JoinSet<()>,
    serial: &str,
) {
    let Some(deck) = discovery.backend(serial) else {
        return;
    };
    let info = deck.info();
    tracing::info!(
        serial = %info.serial,
        model = ?info.model,
        name = %info.display_name,
        keys = info.capabilities.key_count(),
        dials = info.capabilities.dial_count,
        "device attached"
    );
    tasks.spawn(run_engine(deck, services.clone(), engines.clone()));
}

async fn run_engine(deck: Arc<dyn DeckBackend>, services: EngineServices, engines: EngineRegistry) {
    let serial = deck.info().serial.clone();
    match DeviceEngine::start(deck, services).await {
        Ok((engine, handle)) => {
            let id = handle.id();
            engines.insert(handle);
            engine.run().await;
            engines.remove(&serial, id);
        }
        Err(error) => {
            tracing::error!(%serial, error = format!("{error:#}"), "cannot start device engine");
        }
    }
}
