//! The touch strip: dial functions, reminders, popup menu and measures.

use std::sync::atomic::Ordering;

use hubergamof_protocol::DialAssignment;

use super::support::*;
use crate::device::virtual_deck::{DEMO_SERIAL, virtual_plus_info};

/// The pomodoro dial of the default layout.
const POMODORO_DIAL: u8 = 1;
const VOLUME_DIAL: u8 = 0;

#[tokio::test]
async fn the_pomodoro_menu_is_a_popup_over_its_dial() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    assert!(fixture.strip().popup.is_none());

    fixture.engine.press_dial(POMODORO_DIAL).await.unwrap();
    let popup = fixture.strip().popup.expect("the menu is open");
    assert_eq!(popup.dial, Some(usize::from(POMODORO_DIAL)));
    assert_eq!(popup.selected, 0);
    let labels: Vec<_> = popup
        .entries
        .iter()
        .map(|entry| entry.label.as_str())
        .collect();
    assert_eq!(labels, ["Démarrer", "Passer à la pause", "Remise à zéro"]);
    let hint = fixture.hint(usize::from(POMODORO_DIAL));
    assert_eq!(hint.rotate.as_deref(), Some("Choisir"));
    assert_eq!(hint.press.as_deref(), Some("Valider"));

    fixture.engine.rotate_dial(POMODORO_DIAL, 1).await.unwrap();
    assert_eq!(fixture.strip().popup.unwrap().selected, 1);

    // Validating runs the entry (skip to the break) and closes the menu.
    fixture.engine.press_dial(POMODORO_DIAL).await.unwrap();
    assert!(fixture.strip().popup.is_none());
    let pomodoro = fixture.strip().pomodoro.unwrap();
    assert_eq!(pomodoro.widget.label, "Pause · prêt");
    assert_eq!(
        fixture.hint(usize::from(POMODORO_DIAL)).press.as_deref(),
        Some("Menu pomodoro")
    );
}

#[tokio::test]
async fn the_volume_reminder_follows_the_mute() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    // Once the (simulated) audio server was read.
    fixture.wait_for(|f| f.strip().volume.is_some()).await;
    assert_eq!(fixture.hint(0).press.as_deref(), Some("Muet"));
    fixture.engine.press_dial(VOLUME_DIAL).await.unwrap();
    fixture
        .wait_for(|f| f.hint(0).press.as_deref() == Some("Réactiver"))
        .await;
    assert!(fixture.strip().volume.unwrap().muted);
}

#[tokio::test]
async fn empty_dials_and_the_contextual_dial_on_the_home_page_do_nothing() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    let mut config = fixture.config();
    config.dials = vec![
        DialAssignment::Contextual,
        DialAssignment::None,
        DialAssignment::Contextual,
        DialAssignment::None,
    ];
    fixture.engine.apply_config(config).await.unwrap();
    let before = fixture.live().page;
    for dial in 0..4 {
        fixture.engine.press_dial(dial).await.unwrap();
        fixture.engine.rotate_dial(dial, 3).await.unwrap();
        assert_eq!(fixture.dial_mark(usize::from(dial)), None);
    }
    assert_eq!(fixture.live().page, before);
    assert!(fixture.strip().popup.is_none());
    // No module page is displayed: no dial says anything.
    for dial in 0..4 {
        assert_eq!(fixture.hint(dial).rotate, None);
    }
}

#[tokio::test(start_paused = true)]
async fn the_system_is_measured_at_the_configured_period() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    let samples = || fixture.probe.system_samples.load(Ordering::SeqCst);
    // Measured at start, so that the strip never shows empty measures.
    assert_eq!(samples(), 1);
    assert!(fixture.strip().measures[0].percent.is_some());

    // The default period is two seconds.
    fixture.wait_for(|_| samples() == 2).await;
    fixture
        .wait_for(|f| f.strip().measures[0].history.len() == 2)
        .await;

    let mut config = fixture.config();
    config.strip.system_refresh_seconds = 10;
    fixture.engine.apply_config(config).await.unwrap();
    let after_change = samples();
    tokio::time::sleep(std::time::Duration::from_secs(9)).await;
    assert_eq!(samples(), after_change, "the new period is not over yet");
    fixture.wait_for(|_| samples() > after_change).await;
}

#[tokio::test]
async fn the_history_is_hidden_on_request() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    assert_ne!(fixture.strip().measures[0].history, Vec::<u8>::new());
    let mut config = fixture.config();
    config.strip.show_history = false;
    config.strip.show_seconds = false;
    fixture.engine.apply_config(config).await.unwrap();
    fixture
        .wait_for(|f| f.strip().measures[0].history.is_empty())
        .await;
    assert_eq!(fixture.strip().clock.seconds, None);
}

#[tokio::test]
async fn a_load_over_the_threshold_raises_the_alert() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    assert!(
        fixture
            .strip()
            .measures
            .iter()
            .all(|measure| !measure.alert)
    );
    let mut config = fixture.config();
    // The first demo sample of the CPU is 22 %, the memory is at 61 %.
    config.strip.load_alert_percent = 50;
    fixture.engine.apply_config(config).await.unwrap();
    fixture
        .wait_for(|f| f.strip().measures.iter().any(|measure| measure.alert))
        .await;
    let alerts: Vec<_> = fixture
        .strip()
        .measures
        .iter()
        .map(|measure| (measure.label, measure.alert))
        .collect();
    assert_eq!(alerts, [("CPU", false), ("RAM", true), ("SWAP", false)]);
}
