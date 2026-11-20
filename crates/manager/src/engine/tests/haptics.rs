//! The haptic events of the modules, raised on a device engine.

use hubergamof_module_api::HapticWaveform;
use hubergamof_protocol::ModuleKind;

use super::support::*;
use crate::haptics::HapticPlay;

#[tokio::test]
async fn a_declared_event_a_module_raises_goes_out_with_its_default_waveform() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(0)], &["list"], &[]).await;

    assert!(
        fixture
            .engine
            .radial_action(&ModuleKind::from("list"), "haptic")
            .await
    );
    assert_eq!(
        fixture.probe.haptics.played(),
        [HapticPlay {
            module: "list".into(),
            event: "ping".to_owned(),
            waveform: HapticWaveform::Knock,
        }]
    );
}

#[tokio::test]
async fn an_event_the_module_does_not_declare_is_dropped() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(0)], &["list"], &[]).await;

    fixture
        .engine
        .radial_action(&ModuleKind::from("list"), "haptic-undeclared")
        .await;
    assert_eq!(fixture.probe.haptics.played(), []);
}

#[tokio::test]
async fn an_action_that_raises_nothing_plays_nothing() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(0)], &["list"], &[]).await;

    fixture
        .engine
        .radial_action(&ModuleKind::from("list"), "toggle")
        .await;
    assert_eq!(fixture.probe.haptics.played(), []);
}
