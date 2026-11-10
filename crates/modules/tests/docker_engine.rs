//! The Docker backend against the real Docker of the machine (feature
//! `docker-it`, `make test-docker`): what the simulated one stands for in
//! every other test. Needs a running daemon reachable as `DOCKER_HOST` or
//! `/var/run/docker.sock`, and `docker compose`.
#![cfg(test)]
#![cfg(feature = "docker-it")]

use std::fs;
use std::process::Command;
use std::time::Duration;

use hubergamof_modules::docker::{
    ComposeAction, ComposeScope, ComposeTarget, Container, ContainerAction, ContainerStatus,
    DockerBackend, DockerChanges, DockerEngine, UpVariant,
};

/// Small image, pinned by digest (busybox 1.37): the test runs what was
/// reviewed.
const IMAGE: &str =
    "busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec";
/// Longest wait for a state of Docker (pulling the image included).
const PATIENCE: Duration = Duration::from_secs(120);
/// Longest wait for a notified change before listing again: the event
/// stream may connect after the change it should have told.
const RELIST_AFTER: Duration = Duration::from_secs(1);

/// Runs `docker` (test set-up), its standard output; panics on failure.
fn docker(args: &[&str]) -> String {
    let output = Command::new("docker")
        .args(args)
        .output()
        .expect("docker runs");
    assert!(
        output.status.success(),
        "docker {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// A name of this run, so that parallel runs never meet.
fn unique(what: &str) -> String {
    format!("hubergamof-it-{what}-{}", std::process::id())
}

/// Runs `docker` with these arguments when dropped, whatever the outcome
/// of the test: removes what it created.
struct CleanUp(Vec<String>);

impl Drop for CleanUp {
    fn drop(&mut self) {
        // Best effort: the resource may already be gone.
        let _ = Command::new("docker").args(&self.0).output();
    }
}

/// Waits until the containers satisfy `condition`, reading them again at
/// each change Docker notifies, or after [`RELIST_AFTER`] without one.
async fn until(
    engine: &DockerEngine,
    changes: &mut Box<dyn DockerChanges>,
    condition: impl Fn(&[Container]) -> bool,
) -> Vec<Container> {
    tokio::time::timeout(PATIENCE, async {
        loop {
            let containers = engine.containers().await.expect("containers listed");
            if condition(&containers) {
                return containers;
            }
            if let Ok(notifying) = tokio::time::timeout(RELIST_AFTER, changes.changed()).await {
                assert!(notifying, "Docker stopped notifying");
            }
        }
    })
    .await
    .expect("Docker reached the state in time")
}

fn status_of(containers: &[Container], name: &str) -> Option<ContainerStatus> {
    containers
        .iter()
        .find(|container| container.name == name)
        .map(|container| container.status)
}

#[tokio::test]
async fn a_container_is_started_stopped_and_removed() {
    let name = unique("container");
    let _clean = CleanUp(vec!["rm".into(), "--force".into(), name.clone()]);
    let engine = DockerEngine::new();
    let mut changes = engine.changes().await.expect("changes followed");
    // `sleep` as pid 1 ignores SIGTERM: killed after 1 s.
    let id = docker(&[
        "create",
        "--name",
        &name,
        "--stop-timeout",
        "1",
        IMAGE,
        "sleep",
        "300",
    ]);
    until(&engine, &mut changes, |all| {
        status_of(all, &name) == Some(ContainerStatus::Created)
    })
    .await;

    engine.container(&id, ContainerAction::Start).await.unwrap();
    until(&engine, &mut changes, |all| {
        status_of(all, &name) == Some(ContainerStatus::Running)
    })
    .await;
    engine.container(&id, ContainerAction::Stop).await.unwrap();
    until(&engine, &mut changes, |all| {
        status_of(all, &name) == Some(ContainerStatus::Exited)
    })
    .await;
    engine
        .container(&id, ContainerAction::Remove)
        .await
        .unwrap();
    until(&engine, &mut changes, |all| status_of(all, &name).is_none()).await;
}

#[tokio::test]
async fn a_compose_project_goes_up_and_down() {
    let project = unique("compose");
    let _clean = CleanUp(vec![
        "compose".into(),
        "--project-name".into(),
        project.clone(),
        "down".into(),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("compose.yaml");
    fs::write(
        &file,
        format!(
            "services:\n  sleeper:\n    image: {IMAGE}\n    command: [sleep, \"300\"]\n    stop_grace_period: 1s\n"
        ),
    )
    .unwrap();
    let target = ComposeTarget::new(Some(project.clone()), None, vec![file]);
    let of_project = |all: &[Container]| -> Vec<Container> {
        all.iter()
            .filter(|container| {
                container
                    .compose
                    .as_ref()
                    .is_some_and(|labels| labels.project == project)
            })
            .cloned()
            .collect()
    };
    let engine = DockerEngine::new();
    let mut changes = engine.changes().await.expect("changes followed");

    engine
        .compose(
            &target,
            &ComposeScope::Project,
            ComposeAction::Up(UpVariant::Plain),
        )
        .await
        .unwrap();
    let up = until(&engine, &mut changes, |all| {
        of_project(all).iter().any(Container::is_running)
    })
    .await;
    assert_eq!(of_project(&up).len(), 1, "one service");

    engine
        .compose(
            &target,
            &ComposeScope::Project,
            ComposeAction::Down { volumes: false },
        )
        .await
        .unwrap();
    until(&engine, &mut changes, |all| of_project(all).is_empty()).await;
}
