//! Registry of the devices currently attached.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bergadeck_protocol::DeviceInfo;
use tokio::sync::broadcast;

use super::DeckBackend;

/// Capacity of the attach/detach event channel.
pub const DISCOVERY_EVENT_CAPACITY: usize = 64;

/// Change in the set of attached devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// A device was attached; its backend is available through
    /// [`DeviceDiscovery::backend`].
    Attached(DeviceInfo),
    /// The device with this serial was detached.
    Detached(String),
}

/// Devices currently attached, keyed by serial, with change notifications.
///
/// Cheap to clone: clones share the same registry. Sources
/// ([`super::hardware::HardwareHotplug`], [`super::virtual_deck::VirtualDevices`])
/// call [`DeviceDiscovery::attach`] / [`DeviceDiscovery::detach`]; consumers
/// use [`DeviceDiscovery::subscribe`].
#[derive(Clone)]
pub struct DeviceDiscovery {
    inner: Arc<Inner>,
}

struct Inner {
    devices: Mutex<BTreeMap<String, Arc<dyn DeckBackend>>>,
    events: broadcast::Sender<DiscoveryEvent>,
}

impl Default for DeviceDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceDiscovery {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(DISCOVERY_EVENT_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                devices: Mutex::new(BTreeMap::new()),
                events,
            }),
        }
    }

    /// Current devices together with a receiver of the changes that follow,
    /// taken atomically so that no attach or detach is missed or seen twice.
    #[must_use]
    pub fn subscribe(&self) -> (Vec<DeviceInfo>, broadcast::Receiver<DiscoveryEvent>) {
        let devices = self.lock();
        let snapshot = devices.values().map(|deck| deck.info().clone()).collect();
        (snapshot, self.inner.events.subscribe())
    }

    /// Current devices, ordered by serial.
    #[must_use]
    pub fn devices(&self) -> Vec<DeviceInfo> {
        self.subscribe().0
    }

    /// Backend of an attached device.
    #[must_use]
    pub fn backend(&self, serial: &str) -> Option<Arc<dyn DeckBackend>> {
        self.lock().get(serial).cloned()
    }

    /// Backends of every attached device.
    #[must_use]
    pub fn backends(&self) -> Vec<Arc<dyn DeckBackend>> {
        self.lock().values().cloned().collect()
    }

    /// Registers a device and notifies subscribers. A device already attached
    /// under the same serial is replaced (and reported detached first).
    pub fn attach(&self, deck: Arc<dyn DeckBackend>) {
        let info = deck.info().clone();
        let mut devices = self.lock();
        if devices.insert(info.serial.clone(), deck).is_some() {
            self.notify(DiscoveryEvent::Detached(info.serial.clone()));
        }
        self.notify(DiscoveryEvent::Attached(info));
    }

    /// Unregisters a device and notifies subscribers; does nothing when the
    /// serial is not attached.
    pub fn detach(&self, serial: &str) {
        // Notifying while holding the lock keeps `subscribe` snapshots exact.
        let mut devices = self.lock();
        let removed = devices.remove(serial);
        if removed.is_some() {
            self.notify(DiscoveryEvent::Detached(serial.to_owned()));
        }
    }

    fn notify(&self, event: DiscoveryEvent) {
        // An error only means there is no subscriber right now.
        let _ = self.inner.events.send(event);
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Arc<dyn DeckBackend>>> {
        // The map is never left half-updated, so a poisoned lock is still valid.
        self.inner
            .devices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
