//! Real Stream Decks through `elgato-streamdeck` + `hidapi` (hidraw).
//!
//! [`HardwareHotplug`] polls the HID enumeration every
//! [`HOTPLUG_POLL_INTERVAL`] and keeps the [`DeviceDiscovery`] registry in
//! sync: new devices are opened and attached, vanished (or failed) ones are
//! closed and detached.

mod deck;
mod encoded;
pub mod mapping;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use elgato_streamdeck::info::Kind;
use elgato_streamdeck::{StreamDeck, list_devices};
use hidapi::HidApi;
use tokio::sync::oneshot;
use tokio::task::{JoinHandle, spawn_blocking};
use tokio::time::{MissedTickBehavior, interval};

pub use deck::{HardwareDeck, INPUT_READ_TIMEOUT};

use super::{DeckBackend, DeviceDiscovery, DeviceError};

/// Interval between two scans of the HID enumeration.
pub const HOTPLUG_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Device found by an enumeration: hardware kind and serial.
pub type Enumerated = (Kind, String);

/// Changes to apply after an enumeration.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct HotplugChanges {
    /// Serials to close and detach.
    pub detach: Vec<String>,
    /// Devices to open and attach.
    pub attach: Vec<Enumerated>,
}

/// Compares an enumeration with the attached devices (`serial → still
/// connected`). A device whose worker stopped is detached, and re-attached at
/// the same time if it is still enumerated.
#[must_use]
pub fn plan_changes(
    enumerated: &[Enumerated],
    attached: &BTreeMap<String, bool>,
) -> HotplugChanges {
    let present: BTreeSet<&str> = enumerated
        .iter()
        .map(|(_, serial)| serial.as_str())
        .collect();
    let detach: Vec<String> = attached
        .iter()
        .filter(|(serial, connected)| !**connected || !present.contains(serial.as_str()))
        .map(|(serial, _)| serial.clone())
        .collect();
    let attach = enumerated
        .iter()
        .filter(|(_, serial)| !attached.contains_key(serial) || detach.contains(serial))
        .cloned()
        .collect();
    HotplugChanges { detach, attach }
}

/// Access to the HID bus, abstracted so that the polling logic is tested
/// without hardware.
#[async_trait]
pub trait DeviceBus: Send + 'static {
    /// Rescans and lists the Stream Decks currently plugged.
    async fn enumerate(&mut self) -> Result<Vec<Enumerated>, DeviceError>;

    /// Opens a device found by [`DeviceBus::enumerate`].
    async fn open(&mut self, kind: Kind, serial: &str)
    -> Result<Arc<dyn DeckBackend>, DeviceError>;
}

/// The real HID bus (`hidapi`, hidraw backend). Blocking calls run on the
/// tokio blocking pool.
pub struct HidBus {
    /// Moved into each blocking call and back; only `None` if one panicked.
    api: Option<HidApi>,
}

impl HidBus {
    /// Initialises `hidapi`.
    ///
    /// # Errors
    ///
    /// [`DeviceError::Hid`] when the HID subsystem cannot be initialised.
    pub async fn new() -> Result<Self, DeviceError> {
        let api = spawn_blocking(HidApi::new).await??;
        Ok(Self { api: Some(api) })
    }

    async fn with_api<T: Send + 'static>(
        &mut self,
        call: impl FnOnce(&mut HidApi) -> T + Send + 'static,
    ) -> Result<T, DeviceError> {
        let mut api = self
            .api
            .take()
            .ok_or(DeviceError::Hid(hidapi::HidError::InitializationError))?;
        let (api, result) = spawn_blocking(move || {
            let result = call(&mut api);
            (api, result)
        })
        .await?;
        self.api = Some(api);
        Ok(result)
    }
}

#[async_trait]
impl DeviceBus for HidBus {
    async fn enumerate(&mut self) -> Result<Vec<Enumerated>, DeviceError> {
        let result = self
            .with_api(|api| api.refresh_devices().map(|()| list_devices(api)))
            .await?;
        Ok(result?)
    }

    async fn open(
        &mut self,
        kind: Kind,
        serial: &str,
    ) -> Result<Arc<dyn DeckBackend>, DeviceError> {
        let serial = serial.to_owned();
        let deck = self
            .with_api(move |api| {
                StreamDeck::connect(api, kind, &serial)
                    .map_err(DeviceError::from)
                    .and_then(|deck| HardwareDeck::start(deck, mapping::device_info(kind, &serial)))
            })
            .await??;
        Ok(Arc::new(deck))
    }
}

/// Background task keeping the registry in sync with the plugged devices.
pub struct HardwareHotplug {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl HardwareHotplug {
    /// Initialises `hidapi` and starts polling every [`HOTPLUG_POLL_INTERVAL`].
    ///
    /// # Errors
    ///
    /// [`DeviceError::Hid`] when the HID subsystem cannot be initialised.
    pub async fn start(discovery: DeviceDiscovery) -> Result<Self, DeviceError> {
        Ok(Self::start_with(HidBus::new().await?, discovery))
    }

    /// Starts polling `bus`.
    #[must_use]
    pub fn start_with(bus: impl DeviceBus, discovery: DeviceDiscovery) -> Self {
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(Poller::new(bus, discovery).run(stopped));
        Self { stop, task }
    }

    /// Stops polling and closes the devices it attached.
    pub async fn stop(self) {
        let _ = self.stop.send(());
        if let Err(error) = self.task.await {
            tracing::warn!(%error, "hotplug task ended abnormally");
        }
    }
}

struct Poller<B> {
    bus: B,
    discovery: DeviceDiscovery,
    attached: BTreeMap<String, Arc<dyn DeckBackend>>,
    /// Serials that failed to open, reported once until they vanish.
    failed: BTreeSet<String>,
}

impl<B: DeviceBus> Poller<B> {
    fn new(bus: B, discovery: DeviceDiscovery) -> Self {
        Self {
            bus,
            discovery,
            attached: BTreeMap::new(),
            failed: BTreeSet::new(),
        }
    }

    async fn run(mut self, mut stopped: oneshot::Receiver<()>) {
        let mut ticks = interval(HOTPLUG_POLL_INTERVAL);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = &mut stopped => break,
                _ = ticks.tick() => {
                    if let Err(error) = self.poll().await {
                        tracing::warn!(%error, "device enumeration failed");
                    }
                }
            }
        }
        let serials: Vec<String> = self.attached.keys().cloned().collect();
        for serial in serials {
            self.detach(&serial).await;
        }
    }

    async fn poll(&mut self) -> Result<(), DeviceError> {
        let enumerated = self.bus.enumerate().await?;
        self.failed
            .retain(|serial| enumerated.iter().any(|(_, present)| present == serial));
        let status = self
            .attached
            .iter()
            .map(|(serial, deck)| (serial.clone(), deck.is_connected()))
            .collect();
        let changes = plan_changes(&enumerated, &status);
        for serial in changes.detach {
            self.detach(&serial).await;
        }
        for (kind, serial) in changes.attach {
            self.attach(kind, serial).await;
        }
        Ok(())
    }

    async fn attach(&mut self, kind: Kind, serial: String) {
        match self.bus.open(kind, &serial).await {
            Ok(deck) => {
                self.failed.remove(&serial);
                self.attached.insert(serial, Arc::clone(&deck));
                self.discovery.attach(deck);
            }
            Err(error) if self.failed.insert(serial.clone()) => {
                tracing::warn!(
                    %serial,
                    %error,
                    "cannot open Stream Deck (check the udev rule and group membership)"
                );
            }
            Err(error) => tracing::debug!(%serial, %error, "still cannot open Stream Deck"),
        }
    }

    async fn detach(&mut self, serial: &str) {
        if let Some(deck) = self.attached.remove(serial) {
            deck.close().await;
        }
        self.discovery.detach(serial);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::device::DiscoveryEvent;
    use crate::device::virtual_deck::{VirtualDeck, virtual_plus_info};

    /// Bus whose plugged devices and open failures are set by the test.
    #[derive(Clone, Default)]
    struct FakeBus {
        plugged: Arc<Mutex<Vec<String>>>,
        refuse_open: Arc<Mutex<bool>>,
        opened: Arc<Mutex<Vec<Arc<VirtualDeck>>>>,
    }

    impl FakeBus {
        fn plug(&self, serials: &[&str]) {
            *self.plugged.lock().unwrap() = serials.iter().map(|s| (*s).to_owned()).collect();
        }

        fn last_opened(&self) -> Arc<VirtualDeck> {
            Arc::clone(self.opened.lock().unwrap().last().unwrap())
        }
    }

    #[async_trait]
    impl DeviceBus for FakeBus {
        async fn enumerate(&mut self) -> Result<Vec<Enumerated>, DeviceError> {
            Ok(enumerated(
                &self
                    .plugged
                    .lock()
                    .unwrap()
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            ))
        }

        async fn open(
            &mut self,
            _kind: Kind,
            serial: &str,
        ) -> Result<Arc<dyn DeckBackend>, DeviceError> {
            if *self.refuse_open.lock().unwrap() {
                return Err(DeviceError::Disconnected);
            }
            let deck = Arc::new(VirtualDeck::new(virtual_plus_info(serial)));
            self.opened.lock().unwrap().push(Arc::clone(&deck));
            Ok(deck)
        }
    }

    fn attached_event(serial: &str) -> DiscoveryEvent {
        DiscoveryEvent::Attached(virtual_plus_info(serial))
    }

    #[tokio::test(start_paused = true)]
    async fn plugged_and_unplugged_devices_are_tracked() {
        let bus = FakeBus::default();
        let discovery = DeviceDiscovery::new();
        let (_, mut events) = discovery.subscribe();
        bus.plug(&["A"]);
        let hotplug = HardwareHotplug::start_with(bus.clone(), discovery.clone());
        assert_eq!(events.recv().await.unwrap(), attached_event("A"));

        bus.plug(&["B"]);
        assert_eq!(
            events.recv().await.unwrap(),
            DiscoveryEvent::Detached("A".into())
        );
        assert_eq!(events.recv().await.unwrap(), attached_event("B"));
        assert_eq!(discovery.devices(), [virtual_plus_info("B")]);

        hotplug.stop().await;
        assert_eq!(
            events.recv().await.unwrap(),
            DiscoveryEvent::Detached("B".into())
        );
        assert!(!bus.last_opened().is_connected());
    }

    #[tokio::test(start_paused = true)]
    async fn device_whose_worker_stopped_is_reopened() {
        let bus = FakeBus::default();
        let discovery = DeviceDiscovery::new();
        let (_, mut events) = discovery.subscribe();
        bus.plug(&["A"]);
        let hotplug = HardwareHotplug::start_with(bus.clone(), discovery.clone());
        assert_eq!(events.recv().await.unwrap(), attached_event("A"));

        let first = bus.last_opened();
        first.close().await;
        assert_eq!(
            events.recv().await.unwrap(),
            DiscoveryEvent::Detached("A".into())
        );
        assert_eq!(events.recv().await.unwrap(), attached_event("A"));
        assert!(!Arc::ptr_eq(&first, &bus.last_opened()));
        hotplug.stop().await;
    }

    #[tokio::test(start_paused = true)]
    async fn devices_that_cannot_be_opened_are_retried() {
        let bus = FakeBus::default();
        *bus.refuse_open.lock().unwrap() = true;
        let discovery = DeviceDiscovery::new();
        let (_, mut events) = discovery.subscribe();
        bus.plug(&["A"]);
        let hotplug = HardwareHotplug::start_with(bus.clone(), discovery.clone());
        tokio::time::sleep(HOTPLUG_POLL_INTERVAL * 3).await;
        assert_eq!(discovery.devices(), []);

        *bus.refuse_open.lock().unwrap() = false;
        assert_eq!(events.recv().await.unwrap(), attached_event("A"));
        hotplug.stop().await;
    }

    fn enumerated(serials: &[&str]) -> Vec<Enumerated> {
        serials
            .iter()
            .map(|serial| (Kind::Plus, (*serial).to_owned()))
            .collect()
    }

    fn attached(entries: &[(&str, bool)]) -> BTreeMap<String, bool> {
        entries
            .iter()
            .map(|(serial, connected)| ((*serial).to_owned(), *connected))
            .collect()
    }

    #[test]
    fn new_devices_are_attached() {
        let changes = plan_changes(&enumerated(&["A", "B"]), &attached(&[("A", true)]));
        assert_eq!(
            changes,
            HotplugChanges {
                detach: vec![],
                attach: enumerated(&["B"]),
            }
        );
    }

    #[test]
    fn vanished_devices_are_detached() {
        let changes = plan_changes(&enumerated(&["B"]), &attached(&[("A", true), ("B", true)]));
        assert_eq!(changes.detach, ["A"]);
        assert_eq!(changes.attach, []);
    }

    #[test]
    fn failed_devices_still_present_are_reopened() {
        let changes = plan_changes(&enumerated(&["A"]), &attached(&[("A", false)]));
        assert_eq!(changes.detach, ["A"]);
        assert_eq!(changes.attach, enumerated(&["A"]));
    }

    #[test]
    fn unchanged_enumeration_changes_nothing() {
        let changes = plan_changes(&enumerated(&["A"]), &attached(&[("A", true)]));
        assert_eq!(changes, HotplugChanges::default());
    }
}
