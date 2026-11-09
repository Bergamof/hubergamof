//! In-memory [`DockerBackend`]: the hermetic Docker of `--demo` and of the
//! tests. It never touches the Docker of the machine.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::broadcast;

use super::backend::{
    ComposeAction, ContainerAction, DockerBackend, DockerChanges, DockerError, UpVariant,
};
use super::model::{ComposeLabels, ComposeTarget, Container, ContainerStatus, Health};

/// Room for change notifications not yet read; a listener that lags behind
/// reads everything again.
const CHANGES_CAPACITY: usize = 16;

/// What listeners are told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Notice {
    Changed,
    /// Docker "restarted": current listeners stop.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only the tests take Docker away")
    )]
    Disconnected,
}

/// A compose project the simulation can bring up.
#[derive(Debug, Clone)]
struct ProjectDefinition {
    name: String,
    file: PathBuf,
    /// Services, with their image.
    services: Vec<(String, String)>,
}

impl ProjectDefinition {
    fn container_name(&self, service: &str) -> String {
        format!("{}-{service}-1", self.name)
    }

    fn labels(&self, service: &str) -> ComposeLabels {
        ComposeLabels {
            project: self.name.clone(),
            working_dir: self.file.parent().map(PathBuf::from),
            config_files: vec![self.file.clone()],
            service: Some(service.to_owned()),
        }
    }

    fn is(&self, target: &ComposeTarget) -> bool {
        target.project.as_deref() == Some(&self.name) || target.files.contains(&self.file)
    }
}

#[derive(Debug, Default)]
struct State {
    containers: Vec<Container>,
    projects: Vec<ProjectDefinition>,
    next_id: u64,
}

impl State {
    fn add(&mut self, name: String, image: &str, compose: Option<ComposeLabels>) -> &mut Container {
        self.next_id += 1;
        self.containers.push(Container {
            id: format!("{:012x}", self.next_id),
            name,
            image: image.into(),
            status: ContainerStatus::Created,
            health: None,
            compose,
        });
        let last = self.containers.len() - 1;
        &mut self.containers[last]
    }

    fn set_running(container: &mut Container, running: bool) {
        container.status = if running {
            ContainerStatus::Running
        } else {
            ContainerStatus::Exited
        };
        container.health = None;
    }
}

/// Simulated Docker host.
#[derive(Debug)]
pub struct SimulatedDocker {
    state: Mutex<State>,
    notices: broadcast::Sender<Notice>,
    available: AtomicBool,
    lists: AtomicUsize,
    /// Time every action takes (none by default).
    latency: Mutex<Duration>,
    /// Actions applied, as `start shop-web-1` or `up shop`, oldest first.
    actions: Mutex<Vec<String>>,
}

impl SimulatedDocker {
    /// A host without containers.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            state: Mutex::new(State::default()),
            notices: broadcast::channel(CHANGES_CAPACITY).0,
            available: AtomicBool::new(true),
            lists: AtomicUsize::new(0),
            latency: Mutex::new(Duration::ZERO),
            actions: Mutex::new(Vec::new()),
        }
    }

    /// Host of the demo: a running shop, a stopped monitoring project and
    /// two containers of their own.
    #[must_use]
    pub fn demo() -> Self {
        let docker = Self::empty();
        {
            let mut state = lock(&docker.state);
            let shop = ProjectDefinition {
                name: "boutique".into(),
                file: "/srv/boutique/compose.yaml".into(),
                services: [
                    ("api", "boutique-api:dev"),
                    ("db", "postgres:18"),
                    ("web", "nginx:1.29"),
                ]
                .map(|(service, image)| (service.into(), image.into()))
                .to_vec(),
            };
            let monitoring = ProjectDefinition {
                name: "supervision".into(),
                file: "/srv/supervision/compose.yaml".into(),
                services: [
                    ("grafana", "grafana/grafana:12"),
                    ("prometheus", "prom/prometheus:v3"),
                ]
                .map(|(service, image)| (service.into(), image.into()))
                .to_vec(),
            };
            for (project, running) in [(&shop, true), (&monitoring, false)] {
                for (service, image) in &project.services {
                    let container = state.add(
                        project.container_name(service),
                        image,
                        Some(project.labels(service)),
                    );
                    State::set_running(container, running);
                }
            }
            for container in &mut state.containers {
                if container.name == "boutique-db-1" {
                    container.health = Some(Health::Healthy);
                }
            }
            State::set_running(state.add("cache-redis".into(), "redis:8", None), true);
            State::set_running(
                state.add("outil-migration".into(), "migrate:4", None),
                false,
            );
            state.projects = vec![shop, monitoring];
        }
        docker
    }

    fn notify(&self) {
        // Without listener, nobody needs telling.
        let _ = self.notices.send(Notice::Changed);
    }

    /// Waits for the latency of an action.
    async fn take_time(&self) {
        let latency = *lock(&self.latency);
        if !latency.is_zero() {
            tokio::time::sleep(latency).await;
        }
    }

    fn record(&self, action: String) {
        lock(&self.actions).push(action);
    }

    fn check_available(&self) -> Result<(), DockerError> {
        if self.available.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(DockerError::Unreachable("simulated Docker stopped".into()))
        }
    }

    fn apply_to_container(
        state: &mut State,
        id: &str,
        action: ContainerAction,
    ) -> Result<String, DockerError> {
        let refused = |reason: &str| DockerError::Api {
            action: format!("{action:?} container {id}"),
            reason: reason.into(),
        };
        let index = state
            .containers
            .iter()
            .position(|container| container.id == id)
            .ok_or_else(|| refused("No such container"))?;
        let container = &mut state.containers[index];
        let name = container.name.clone();
        match action {
            ContainerAction::Start | ContainerAction::Restart => {
                State::set_running(container, true);
            }
            ContainerAction::Stop => State::set_running(container, false),
            ContainerAction::Remove => {
                if container.is_running() {
                    return Err(refused("cannot remove a running container"));
                }
                state.containers.remove(index);
            }
        }
        Ok(format!("{} {name}", verb(action)))
    }

    fn apply_to_project(
        state: &mut State,
        target: &ComposeTarget,
        action: ComposeAction,
    ) -> Result<String, DockerError> {
        let project = state
            .projects
            .iter()
            .find(|project| project.is(target))
            .cloned()
            .ok_or_else(|| DockerError::InvalidTarget("unknown project".into()))?;
        let belongs = |container: &Container| {
            container
                .compose
                .as_ref()
                .is_some_and(|labels| labels.project == project.name)
        };
        match action {
            ComposeAction::Up(variant) => {
                let wanted = |service: &str| {
                    target.services.is_empty() || target.services.iter().any(|name| name == service)
                };
                for (service, image) in project.services.iter().filter(|(name, _)| wanted(name)) {
                    let name = project.container_name(service);
                    // `--no-start` creates what is missing and starts nothing.
                    let start = variant != UpVariant::NoStart;
                    match state.containers.iter_mut().find(|c| c.name == name) {
                        Some(container) if start => State::set_running(container, true),
                        Some(_) => {}
                        None => {
                            let created = state.add(name, image, Some(project.labels(service)));
                            if start {
                                State::set_running(created, true);
                            }
                        }
                    }
                }
                let suffix = match variant {
                    UpVariant::Plain => "",
                    UpVariant::Build => " --build",
                    UpVariant::Pull => " --pull",
                    UpVariant::NoStart => " --no-start",
                };
                let scope = scope_suffix(target);
                Ok(format!("up {}{suffix}{scope}", project.name))
            }
            ComposeAction::Down { volumes } => {
                state.containers.retain(|container| {
                    let in_scope = target.services.is_empty()
                        || container
                            .compose
                            .as_ref()
                            .and_then(|labels| labels.service.as_ref())
                            .is_some_and(|service| target.services.contains(service));
                    !(belongs(container) && in_scope)
                });
                let suffix = if volumes { " --volumes" } else { "" };
                let scope = scope_suffix(target);
                Ok(format!("down {}{suffix}{scope}", project.name))
            }
        }
    }
}

/// Inspection and control, for the tests of the modules.
#[cfg(test)]
impl SimulatedDocker {
    /// The containers as they are.
    pub(crate) fn state(&self) -> Vec<Container> {
        lock(&self.state).containers.clone()
    }

    /// The container named `name`.
    pub(crate) fn named(&self, name: &str) -> Option<Container> {
        self.state()
            .into_iter()
            .find(|container| container.name == name)
    }

    /// Actions applied so far, oldest first.
    pub(crate) fn actions(&self) -> Vec<String> {
        lock(&self.actions).clone()
    }

    /// Number of lists so far.
    pub(crate) fn lists(&self) -> usize {
        self.lists.load(Ordering::SeqCst)
    }

    /// Number of listeners currently listening.
    pub(crate) fn listeners(&self) -> usize {
        self.notices.receiver_count()
    }

    /// Changes the containers as another program would, and notifies.
    pub(crate) fn change_externally(&self, change: impl FnOnce(&mut Vec<Container>)) {
        change(&mut lock(&self.state).containers);
        self.notify();
    }

    /// Makes every action take `latency`.
    pub(crate) fn set_latency(&self, latency: Duration) {
        *lock(&self.latency) = latency;
    }

    /// Makes every call fail (`false`) or work again (`true`); going away
    /// stops the listeners.
    pub(crate) fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::SeqCst);
        if !available {
            let _ = self.notices.send(Notice::Disconnected);
        }
    }
}

#[async_trait]
impl DockerBackend for SimulatedDocker {
    async fn containers(&self) -> Result<Vec<Container>, DockerError> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        self.check_available()?;
        Ok(lock(&self.state).containers.clone())
    }

    async fn changes(&self) -> Result<Box<dyn DockerChanges>, DockerError> {
        self.check_available()?;
        Ok(Box::new(SimulatedChanges(self.notices.subscribe())))
    }

    async fn container(&self, id: &str, action: ContainerAction) -> Result<(), DockerError> {
        self.take_time().await;
        self.check_available()?;
        let done = Self::apply_to_container(&mut lock(&self.state), id, action)?;
        self.record(done);
        self.notify();
        Ok(())
    }

    async fn compose(
        &self,
        target: &ComposeTarget,
        action: ComposeAction,
    ) -> Result<(), DockerError> {
        self.take_time().await;
        self.check_available()?;
        let done = Self::apply_to_project(&mut lock(&self.state), target, action)?;
        self.record(done);
        self.notify();
        Ok(())
    }

    async fn compose_services(&self, target: &ComposeTarget) -> Result<Vec<String>, DockerError> {
        self.check_available()?;
        lock(&self.state)
            .projects
            .iter()
            .find(|project| project.is(target))
            .map(|project| {
                project
                    .services
                    .iter()
                    .map(|(name, _)| name.clone())
                    .collect()
            })
            .ok_or_else(|| DockerError::InvalidTarget("unknown project".into()))
    }
}

/// The services a compose action is narrowed to, as the end of its record.
fn scope_suffix(target: &ComposeTarget) -> String {
    if target.services.is_empty() {
        String::new()
    } else {
        format!(" {}", target.services.join(" "))
    }
}

fn verb(action: ContainerAction) -> &'static str {
    match action {
        ContainerAction::Start => "start",
        ContainerAction::Stop => "stop",
        ContainerAction::Restart => "restart",
        ContainerAction::Remove => "remove",
    }
}

struct SimulatedChanges(broadcast::Receiver<Notice>);

#[async_trait]
impl DockerChanges for SimulatedChanges {
    async fn changed(&mut self) -> bool {
        match self.0.recv().await {
            Ok(Notice::Changed) | Err(broadcast::error::RecvError::Lagged(_)) => true,
            Ok(Notice::Disconnected) | Err(broadcast::error::RecvError::Closed) => false,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every update leaves the state consistent: a poisoned lock is still
    // valid.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id_of(docker: &SimulatedDocker, name: &str) -> String {
        docker.named(name).unwrap().id
    }

    #[tokio::test]
    async fn containers_start_stop_and_are_removed_only_when_stopped() {
        let docker = SimulatedDocker::demo();
        let mut changes = docker.changes().await.unwrap();
        let redis = id_of(&docker, "cache-redis");
        let refused = docker.container(&redis, ContainerAction::Remove).await;
        assert!(
            matches!(refused, Err(DockerError::Api { .. })),
            "{refused:?}"
        );
        docker
            .container(&redis, ContainerAction::Stop)
            .await
            .unwrap();
        assert!(changes.changed().await);
        assert_eq!(
            docker.named("cache-redis").unwrap().status,
            ContainerStatus::Exited
        );
        docker
            .container(&redis, ContainerAction::Remove)
            .await
            .unwrap();
        assert!(docker.named("cache-redis").is_none());
        assert_eq!(docker.actions(), ["stop cache-redis", "remove cache-redis"]);
        assert!(
            docker
                .container("nope", ContainerAction::Start)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn projects_go_down_and_up_again() {
        let docker = SimulatedDocker::demo();
        let shop = ComposeTarget {
            project: Some("boutique".into()),
            ..ComposeTarget::default()
        };
        docker
            .compose(&shop, ComposeAction::Down { volumes: true })
            .await
            .unwrap();
        assert!(docker.named("boutique-web-1").is_none());
        // Back from its file only, as a declared project is.
        let by_file = ComposeTarget {
            files: vec!["/srv/boutique/compose.yaml".into()],
            ..ComposeTarget::default()
        };
        docker
            .compose(&by_file, ComposeAction::Up(UpVariant::Pull))
            .await
            .unwrap();
        assert!(docker.named("boutique-web-1").unwrap().is_running());
        docker
            .compose(&by_file, ComposeAction::Up(UpVariant::Build))
            .await
            .unwrap();
        docker
            .compose(
                &ComposeTarget::default(),
                ComposeAction::Up(UpVariant::Plain),
            )
            .await
            .unwrap_err();
        assert_eq!(
            docker.actions(),
            [
                "down boutique --volumes",
                "up boutique --pull",
                "up boutique --build"
            ]
        );
    }

    #[tokio::test]
    async fn an_absent_docker_fails_and_stops_its_listeners() {
        let docker = SimulatedDocker::demo();
        let mut changes = docker.changes().await.unwrap();
        assert_eq!(docker.listeners(), 1);
        docker.set_available(false);
        assert!(!changes.changed().await);
        assert!(docker.containers().await.is_err());
        assert!(docker.changes().await.is_err());
        assert_eq!(docker.lists(), 1);
        docker.set_available(true);
        assert_eq!(docker.containers().await.unwrap().len(), 7);
    }
}
