//! The items shown around a petal of the radial menu, published by the
//! engines and read from another thread.

use std::sync::atomic::Ordering;

use hubergamof_module_api::{MAX_RADIAL_ITEMS, ModuleKind, RadialGlyph, RadialItem};
use serde_json::json;

use super::support::*;
use crate::device::DeviceDiscovery;
use crate::device::virtual_deck::{VirtualDevices, virtual_plus_info};
use crate::engine::{EngineRegistry, Supervisor};

fn kind() -> ModuleKind {
    ModuleKind::from("list")
}

fn item(label: &str) -> RadialItem {
    RadialItem::new("toggle", label, RadialGlyph::Play)
}

fn labels(items: Option<Vec<RadialItem>>) -> Option<Vec<String>> {
    items.map(|items| items.into_iter().map(|item| item.label).collect())
}

fn owned(labels: &[&str]) -> Vec<String> {
    labels.iter().map(|label| (*label).to_owned()).collect()
}

/// Asks the list instances to redraw, as a module whose state changed on its
/// own (a call ended) does.
fn request_redraw(probe: &Probe) {
    let redraw = probe.redraw.lock().unwrap().clone().unwrap();
    redraw.request_redraw();
}

async fn wait_for_items(fixture: &Fixture, expected: Option<Vec<String>>) {
    let mut published = fixture.engine.radial_items_watch();
    tokio::time::timeout(WAIT, async {
        while labels(fixture.engine.radial_items(&kind())) != expected {
            published.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}

async fn wait_for_reads(probe: &Probe, more_than: usize) {
    let mut live_poll = tokio::time::interval(std::time::Duration::from_millis(5));
    tokio::time::timeout(WAIT, async {
        while probe.radial_items_reads.load(Ordering::SeqCst) <= more_than {
            live_poll.tick().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn the_items_follow_the_state_of_the_module() {
    let fixture = start(no_dial_info()).await;
    *fixture.probe.radial_items.lock().unwrap() = vec![item("Start")];
    fixture.configure(vec![list(0)], &["list"], &[]).await;
    assert_eq!(
        labels(fixture.engine.radial_items(&kind())),
        Some(owned(&["Start"]))
    );

    *fixture.probe.radial_items.lock().unwrap() = vec![item("Stop"), item("Reset")];
    request_redraw(&fixture.probe);
    wait_for_items(&fixture, Some(owned(&["Stop", "Reset"]))).await;
}

#[tokio::test]
async fn an_item_that_runs_no_declared_action_is_not_shown() {
    let fixture = start(no_dial_info()).await;
    *fixture.probe.radial_items.lock().unwrap() = vec![
        item("Declared"),
        RadialItem::new("undeclared", "Stray", RadialGlyph::Play),
    ];
    fixture.configure(vec![list(0)], &["list"], &[]).await;
    assert_eq!(
        labels(fixture.engine.radial_items(&kind())),
        Some(owned(&["Declared"]))
    );
}

#[tokio::test]
async fn the_items_are_cut_after_the_most_a_petal_shows() {
    let fixture = start(no_dial_info()).await;
    *fixture.probe.radial_items.lock().unwrap() =
        (0..10).map(|index| item(&index.to_string())).collect();
    fixture.configure(vec![list(0)], &["list"], &[]).await;
    assert_eq!(
        fixture.engine.radial_items(&kind()).unwrap().len(),
        MAX_RADIAL_ITEMS
    );
}

#[tokio::test]
async fn a_module_without_item_is_held_with_an_empty_list_and_a_missing_one_is_not() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(0)], &["list"], &[]).await;
    assert_eq!(fixture.engine.radial_items(&kind()), Some(Vec::new()));

    fixture.configure(Vec::new(), &[], &[]).await;
    assert_eq!(fixture.engine.radial_items(&kind()), None);
}

#[tokio::test]
async fn a_module_is_read_once_per_refresh() {
    let fixture = start(no_dial_info()).await;
    *fixture.probe.radial_items.lock().unwrap() = vec![item("Start")];
    fixture.configure(vec![list(0)], &["list"], &[]).await;

    let reads = fixture.probe.radial_items_reads.load(Ordering::SeqCst);
    request_redraw(&fixture.probe);
    wait_for_reads(&fixture.probe, reads).await;
    // Let a possible second read of the same refresh happen before counting.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        fixture.probe.radial_items_reads.load(Ordering::SeqCst) - reads,
        1
    );
}

#[tokio::test]
async fn a_module_that_declares_no_radial_action_is_not_published() {
    let fixture = start(no_dial_info()).await;
    fixture
        .configure(vec![fragile("press")], &[FRAGILE_KIND], &[])
        .await;
    assert_eq!(
        fixture.engine.radial_items(&ModuleKind::from(FRAGILE_KIND)),
        None
    );
}

#[tokio::test]
async fn nothing_is_published_when_the_items_did_not_change() {
    let fixture = start(no_dial_info()).await;
    *fixture.probe.radial_items.lock().unwrap() = vec![item("Start")];
    fixture.configure(vec![list(0)], &["list"], &[]).await;
    let mut published = fixture.engine.radial_items_watch();
    published.mark_unchanged();
    let reads = fixture.probe.radial_items_reads.load(Ordering::SeqCst);

    request_redraw(&fixture.probe);
    wait_for_reads(&fixture.probe, reads).await;
    assert!(!published.has_changed().unwrap());

    *fixture.probe.radial_items.lock().unwrap() = vec![item("Stop")];
    request_redraw(&fixture.probe);
    tokio::time::timeout(WAIT, published.changed())
        .await
        .unwrap()
        .unwrap();
}

async fn wait_for_engine(engines: &EngineRegistry, serial: &str, present: bool) {
    let mut changes = engines.changes();
    tokio::time::timeout(WAIT, async {
        loop {
            changes.mark_unchanged();
            if engines.get(serial).is_some() == present {
                return;
            }
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn the_registry_reads_the_first_device_in_serial_order_that_holds_the_module() {
    let probe = Probe::default();
    *probe.radial_items.lock().unwrap() = vec![item("Start")];
    let dir = tempfile::tempdir().unwrap();
    let discovery = DeviceDiscovery::new();
    let supervisor = Supervisor::start(&discovery, services(&dir, &probe));
    let engines = supervisor.engines();
    let devices = VirtualDevices::new(discovery);

    // Attached in the reverse order of their serials.
    let _second = devices.attach(virtual_plus_info("B-second"));
    wait_for_engine(&engines, "B-second", true).await;
    let _first = devices.attach(virtual_plus_info("A-first"));
    wait_for_engine(&engines, "A-first", true).await;
    assert_eq!(engines.radial_items(&kind()), Vec::new(), "nobody holds it");

    for (serial, label) in [("B-second", "B"), ("A-first", "A")] {
        let handle = engines.get(serial).unwrap();
        let mut config = handle.config().borrow().clone();
        config.modules = vec![instance("l", "list", json!({ "label": label }))];
        config.home = vec!["l".into()];
        config.dials = vec![hubergamof_protocol::DialAssignment::None; 4];
        handle.apply_config(config).await.unwrap();
    }
    let badge = |engines: &EngineRegistry| engines.radial_items(&kind())[0].badge.clone();
    assert_eq!(badge(&engines).as_deref(), Some("A"));

    // The first one unplugged: the snapshot goes with its engine.
    assert!(devices.detach("A-first").await);
    wait_for_engine(&engines, "A-first", false).await;
    assert_eq!(badge(&engines).as_deref(), Some("B"));

    assert!(devices.detach("B-second").await);
    wait_for_engine(&engines, "B-second", false).await;
    assert_eq!(engines.radial_items(&kind()), Vec::new());
    supervisor.shutdown().await;
}

#[tokio::test]
async fn the_combined_read_prefers_the_devices_over_the_application() {
    let probe = Probe::default();
    *probe.radial_items.lock().unwrap() = vec![item("Start")];
    let dir = tempfile::tempdir().unwrap();
    let discovery = DeviceDiscovery::new();
    let services = services(&dir, &probe);
    let (host, _task) = crate::module_host::ModuleHost::start(crate::module_host::HostServices {
        registry: std::sync::Arc::clone(&services.registry),
        store: services.store.clone(),
        haptics: std::sync::Arc::clone(&services.haptics),
        screen_lock: std::sync::Arc::clone(&services.screen_lock),
    });
    let supervisor = Supervisor::start(&discovery, services);
    let engines = supervisor.engines();
    let devices = VirtualDevices::new(discovery);
    let _deck = devices.attach(virtual_plus_info("A-first"));
    wait_for_engine(&engines, "A-first", true).await;
    assert_eq!(
        crate::engine::first_radial_items(&engines, &host, &kind()),
        Vec::new()
    );

    let handle = engines.get("A-first").unwrap();
    let mut config = handle.config().borrow().clone();
    config.modules = vec![instance("l", "list", json!({ "label": "A" }))];
    config.home = vec!["l".into()];
    config.dials = vec![hubergamof_protocol::DialAssignment::None; 4];
    handle.apply_config(config).await.unwrap();

    let items = crate::engine::first_radial_items(&engines, &host, &kind());
    assert_eq!(labels(Some(items)), Some(owned(&["Start"])));
    supervisor.shutdown().await;
}
