//! The native messaging host, run as the browser runs it, against a real
//! Teams bridge listening in a private runtime directory.
#![cfg(test)]

use std::process::Stdio;
use std::time::Duration;

use hubergamof_modules::teams::wire::{CallAction, CallPhase, read_frame, write_frame};
use hubergamof_modules::teams::{SocketBridge, TeamsBridge};
use tokio::process::Command;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(10);

fn native_host(runtime: &std::path::Path) -> tokio::process::Child {
    // The host only starts for the extension's own origin.
    let id = hubergamof_manager::teams_extension::embedded_extension_id().unwrap();
    let origin = format!("chrome-extension://{id}/");
    Command::new(env!("CARGO_BIN_EXE_hubergamof"))
        .args(["native-host", &origin])
        .env("XDG_RUNTIME_DIR", runtime)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

#[tokio::test]
async fn the_extension_reaches_the_bridge_and_the_bridge_the_extension() {
    let runtime = tempfile::tempdir().unwrap();
    let socket = runtime.path().join("hubergamof").join("teams.sock");
    let bridge = SocketBridge::new(Some(socket.clone()));
    let mut state = bridge.state();
    timeout(WAIT, async {
        while !socket.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    let mut host = native_host(runtime.path());
    let mut to_host = host.stdin.take().unwrap();
    let mut from_host = host.stdout.take().unwrap();

    write_frame(
        &mut to_host,
        br#"{"type":"call","phase":"inCall","muted":false,"cameraOn":null,"caller":null}"#,
    )
    .await
    .unwrap();
    let seen = timeout(
        WAIT,
        state.wait_for(|state| state.call.phase == CallPhase::InCall),
    )
    .await
    .unwrap()
    .unwrap()
    .clone();
    assert!(seen.connected);

    assert_eq!(bridge.request(CallAction::HangUp), Ok(()));
    let frame = timeout(WAIT, read_frame(&mut from_host, 1024))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(frame, br#"{"type":"do","action":"hangUp"}"#);

    drop(to_host);
    let status = timeout(WAIT, host.wait()).await.unwrap().unwrap();
    assert!(status.success(), "the host ends with the browser: {status}");
}

#[tokio::test]
async fn without_a_manager_the_host_fails_instead_of_hanging() {
    let runtime = tempfile::tempdir().unwrap();
    let mut host = native_host(runtime.path());
    let status = timeout(WAIT, host.wait()).await.unwrap().unwrap();
    assert!(!status.success());
}

#[tokio::test]
async fn the_host_refuses_an_origin_that_is_not_the_extensions() {
    let runtime = tempfile::tempdir().unwrap();
    let status = timeout(
        WAIT,
        Command::new(env!("CARGO_BIN_EXE_hubergamof"))
            .args([
                "native-host",
                "chrome-extension://abcdefghijklmnopabcdefghijklmnop/",
            ])
            .env("XDG_RUNTIME_DIR", runtime.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!status.success());
}
