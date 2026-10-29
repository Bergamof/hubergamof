//! Engines started and stopped with their devices.

use super::support::*;
use crate::device::DeviceDiscovery;
use crate::device::virtual_deck::{DEMO_SERIAL, VirtualDevices, virtual_plus_info};
use crate::engine::*;

/// Waits until the engine of the demo device satisfies `ready` (`None`: no
/// engine), checking at every registry change.
async fn wait_for_engine(
    engines: &EngineRegistry,
    ready: impl Fn(Option<&EngineHandle>) -> bool,
) -> Option<EngineHandle> {
    let mut changes = engines.changes();
    tokio::time::timeout(WAIT, async {
        loop {
            changes.mark_unchanged();
            let engine = engines.get(DEMO_SERIAL);
            if ready(engine.as_ref()) {
                return engine;
            }
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn supervisor_restarts_engines_on_reattach() {
    let dir = tempfile::tempdir().unwrap();
    let discovery = DeviceDiscovery::new();
    let supervisor = Supervisor::start(&discovery, services(&dir, &Probe::default()));
    let devices = VirtualDevices::new(discovery.clone());
    let engines = supervisor.engines();

    let _deck = devices.attach(virtual_plus_info(DEMO_SERIAL));
    let first = wait_for_engine(&engines, |engine| engine.is_some())
        .await
        .unwrap();
    assert!(devices.detach(DEMO_SERIAL).await);
    wait_for_engine(&engines, |engine| engine.is_none()).await;

    let deck = devices.attach(virtual_plus_info(DEMO_SERIAL));
    let second = wait_for_engine(&engines, |engine| {
        engine.is_some_and(|engine| engine.id() != first.id())
    })
    .await
    .unwrap();
    assert_eq!(second.live().borrow().page.id, HOME_PAGE_ID);

    supervisor.shutdown().await;
    assert!(deck.screen().keys.iter().all(Option::is_none));
    assert!(engines.handles().is_empty());
}

#[tokio::test]
async fn engine_registry_notifies_engine_starts_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let discovery = DeviceDiscovery::new();
    let supervisor = Supervisor::start(&discovery, services(&dir, &Probe::default()));
    let devices = VirtualDevices::new(discovery.clone());
    let engines = supervisor.engines();
    let mut changes = engines.changes();

    let _deck = devices.attach(virtual_plus_info(DEMO_SERIAL));
    tokio::time::timeout(WAIT, changes.changed())
        .await
        .unwrap()
        .unwrap();
    assert!(engines.get(DEMO_SERIAL).is_some());

    assert!(devices.detach(DEMO_SERIAL).await);
    tokio::time::timeout(WAIT, changes.changed())
        .await
        .unwrap()
        .unwrap();
    assert!(engines.get(DEMO_SERIAL).is_none());
    supervisor.shutdown().await;
}
