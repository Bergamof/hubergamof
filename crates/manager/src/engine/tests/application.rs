//! A device that places the modules of the application shows and drives the
//! instance of the application, instead of making its own.

use std::time::Duration;

use hubergamof_module_api::KeyState;
use hubergamof_protocol::ModuleKind;
use serde_json::json;

use super::support::*;
use crate::device::virtual_deck::{DEMO_SERIAL, virtual_plus_info};
use crate::module_host::{ModuleHost, RadialReply};

/// The pomodoro dial of the default layout.
const POMODORO_DIAL: u8 = 1;

fn pomodoro() -> ModuleKind {
    "pomodoro".into()
}

/// Waits until the application shows a pomodoro key in `state`.
async fn wait_for_key_state(host: &ModuleHost, state: KeyState) {
    let mut views = host.views();
    tokio::time::timeout(WAIT, async {
        while views
            .borrow()
            .get(&pomodoro())
            .is_none_or(|view| view.key.state != state)
        {
            views.changed().await.unwrap();
        }
    })
    .await
    .expect("the application shows the pomodoro in that state");
}

#[tokio::test]
async fn the_menu_of_a_dial_drives_the_instance_of_the_application() {
    let fixture = start_applied(virtual_plus_info(DEMO_SERIAL), |_, _| {}).await;
    let host = fixture.host.clone().unwrap();
    // The application publishes what it shows a moment after it started.
    fixture.wait_for(|f| f.strip().pomodoro.is_some()).await;
    assert_eq!(
        fixture.strip().pomodoro.unwrap().widget.label,
        "Focus 1/4 · prêt"
    );

    fixture.engine.press_dial(POMODORO_DIAL).await.unwrap();
    fixture.wait_for(|f| f.strip().popup.is_some()).await;
    let popup = fixture.strip().popup.unwrap();
    let labels: Vec<_> = popup
        .entries
        .iter()
        .map(|entry| entry.label.as_str())
        .collect();
    assert_eq!(labels, ["Démarrer", "Passer à la pause", "Remise à zéro"]);

    // Validating the first entry starts the time, in the application.
    fixture.engine.press_dial(POMODORO_DIAL).await.unwrap();
    wait_for_key_state(&host, KeyState::Active).await;
    fixture.wait_for(|f| f.strip().popup.is_none()).await;
    fixture
        .wait_for(|f| {
            f.strip()
                .pomodoro
                .is_some_and(|face| face.widget.label == "Focus 1/4")
        })
        .await;
}

#[tokio::test]
async fn an_action_of_the_application_shows_on_the_device() {
    let fixture = start_applied(virtual_plus_info(DEMO_SERIAL), |_, _| {}).await;
    let host = fixture.host.clone().unwrap();

    assert_eq!(
        host.radial_action(&pomodoro(), "toggle").await,
        RadialReply::Done
    );
    fixture
        .wait_for(|f| {
            f.strip()
                .pomodoro
                .is_some_and(|face| face.widget.label == "Focus 1/4")
        })
        .await;

    // And the other way round: the device pauses what the application runs.
    fixture.engine.press_dial(POMODORO_DIAL).await.unwrap();
    fixture.wait_for(|f| f.strip().popup.is_some()).await;
    fixture.engine.press_dial(POMODORO_DIAL).await.unwrap();
    wait_for_key_state(&host, KeyState::Normal).await;
}

#[tokio::test]
async fn the_timer_goes_on_without_the_device() {
    let fixture = start_applied(virtual_plus_info(DEMO_SERIAL), |_, _| {}).await;
    let host = fixture.host.clone().unwrap();
    fixture.engine.stop().await;

    assert_eq!(
        host.radial_action(&pomodoro(), "toggle").await,
        RadialReply::Done
    );
    wait_for_key_state(&host, KeyState::Active).await;
}

#[tokio::test]
async fn the_settings_of_a_device_become_those_of_the_application() {
    let fixture = start_applied(virtual_plus_info(DEMO_SERIAL), |_, _| {}).await;
    let stored = || fixture.store.load_app_modules().unwrap().config.modules;

    // The settings the deck had are kept, since the file had none.
    wait_until(|| !stored().is_empty()).await;
    assert_eq!(stored()[0].kind, pomodoro());

    // Settings saved on the deck replace them.
    let mut config = fixture.engine.config().borrow().clone();
    let instance = config
        .modules
        .iter_mut()
        .find(|instance| instance.kind == pomodoro())
        .unwrap();
    instance.settings = json!({ "workMinutes": 30 });
    fixture.engine.apply_config(config).await.unwrap();
    wait_until(|| stored()[0].settings["workMinutes"] == 30).await;
}

/// Waits, in real time, until `done` holds.
async fn wait_until(done: impl Fn() -> bool) {
    tokio::time::timeout(WAIT, async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("it happens");
}
