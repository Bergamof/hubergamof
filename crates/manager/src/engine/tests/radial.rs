//! The radial menu of the mice running an action of a module of the device.

use std::time::Duration;

use hubergamof_protocol::ModuleKind;

use super::support::*;

/// Time for the engine to handle what it was sent.
const SETTLE: Duration = Duration::from_millis(50);

#[tokio::test]
async fn an_action_reaches_the_module_of_the_device() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(0)], &["list"], &[]).await;

    assert!(
        fixture
            .engine
            .radial_action(&ModuleKind::from("list"), "toggle")
            .await
    );
    assert_eq!(*fixture.probe.radial_actions.lock().unwrap(), ["toggle"]);
}

#[tokio::test]
async fn a_device_without_the_module_says_so_and_runs_nothing() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(0)], &["list"], &[]).await;

    assert!(
        !fixture
            .engine
            .radial_action(&ModuleKind::from("pomodoro"), "toggle")
            .await
    );
    assert!(fixture.probe.radial_actions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_action_goes_to_the_instance_of_its_module_kind_only() {
    let fixture = start(no_dial_info()).await;
    fixture
        .configure(
            vec![list(0), fragile("press")],
            &["list", FRAGILE_KIND],
            &[],
        )
        .await;

    assert!(
        fixture
            .engine
            .radial_action(&ModuleKind::from(FRAGILE_KIND), "skip")
            .await
    );
    // The fragile module takes it (and ignores it): the list was not called.
    assert!(fixture.probe.radial_actions.lock().unwrap().is_empty());
    assert!(
        fixture
            .engine
            .radial_action(&ModuleKind::from("list"), "skip")
            .await
    );
    assert_eq!(*fixture.probe.radial_actions.lock().unwrap(), ["skip"]);
}

#[tokio::test]
async fn an_action_is_refused_while_the_computer_is_locked() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(0)], &["list"], &[]).await;
    fixture.probe.screen_lock.set(true);
    tokio::time::sleep(SETTLE).await;

    // The device has the module, so it answers true: the module is not called.
    assert!(
        fixture
            .engine
            .radial_action(&ModuleKind::from("list"), "toggle")
            .await
    );
    assert!(fixture.probe.radial_actions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_module_usable_while_locked_is_still_refused_from_the_menu() {
    let fixture = start(no_dial_info()).await;
    fixture
        .probe
        .list_usable_when_locked
        .store(true, std::sync::atomic::Ordering::SeqCst);
    fixture.configure(vec![list(0)], &["list"], &[]).await;
    // Unlocked, the action runs.
    assert!(
        fixture
            .engine
            .radial_action(&ModuleKind::from("list"), "before")
            .await
    );
    fixture.probe.screen_lock.set(true);
    tokio::time::sleep(SETTLE).await;
    assert!(
        fixture
            .engine
            .radial_action(&ModuleKind::from("list"), "after")
            .await
    );
    assert_eq!(*fixture.probe.radial_actions.lock().unwrap(), ["before"]);
}

#[tokio::test]
async fn a_stopped_engine_answers_false() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(0)], &["list"], &[]).await;
    fixture.engine.stop().await;

    assert!(
        !fixture
            .engine
            .radial_action(&ModuleKind::from("list"), "toggle")
            .await
    );
}
