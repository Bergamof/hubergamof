//! [`DockerBackend`] driving the Docker Engine of the session through its
//! API (`bollard`, local Unix socket: `DOCKER_HOST=unix://…` or
//! `/var/run/docker.sock`), and `docker compose` for the projects, on that
//! same socket.
//!
//! Access to the socket is access to the host as root. Actions come from
//! presses, physical or sent by the web interface: whoever drives the
//! manager drives Docker, so its local server must only accept the user
//! (see `crates/manager/src/server/security.rs`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use bollard::errors::Error as BollardError;
use bollard::models::{
    ContainerSummary, ContainerSummaryHealthStatusEnum as HealthEnum,
    ContainerSummaryStateEnum as StateEnum, EventMessage,
};
use bollard::query_parameters::{
    EventsOptions, ListContainersOptions, RemoveContainerOptions, StopContainerOptions,
};
use bollard::{API_DEFAULT_VERSION, Docker};
use futures_util::{Stream, StreamExt};
use tokio::sync::OnceCell;

use super::backend::{ComposeAction, ContainerAction, DockerBackend, DockerChanges, DockerError};
use super::compose::ComposeCli;
use super::model::{
    ComposeLabels, ComposeTarget, Container, ContainerStatus, Health, is_project_name,
};

/// Longest wait for an answer of the API (a stop answers once the container
/// stopped, within its own timeout, 10 s by default).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Variable naming the Docker host, as for the `docker` tool.
const HOST_VARIABLE: &str = "DOCKER_HOST";
const UNIX_SCHEME: &str = "unix://";
/// Socket of the engine without `DOCKER_HOST`.
const DEFAULT_ENDPOINT: &str = "unix:///var/run/docker.sock";

const PROJECT_LABEL: &str = "com.docker.compose.project";
const WORKING_DIR_LABEL: &str = "com.docker.compose.project.working_dir";
const CONFIG_FILES_LABEL: &str = "com.docker.compose.project.config_files";
const SERVICE_LABEL: &str = "com.docker.compose.service";
/// Separator of the files in [`CONFIG_FILES_LABEL`].
const CONFIG_FILES_SEPARATOR: char = ',';

/// Events that change what the module shows; the others (`exec_*` of every
/// health check, `attach`, `top`…) are ignored, so that a health check
/// does not cost a read of every container.
const RELEVANT_ACTIONS: [&str; 12] = [
    "create", "start", "restart", "stop", "die", "kill", "destroy", "pause", "unpause", "rename",
    "update", "oom",
];
/// Prefix of the events of health changes (`health_status: healthy`).
const HEALTH_ACTION_PREFIX: &str = "health_status";

/// The Docker Engine of the session.
#[derive(Debug)]
pub struct DockerEngine {
    endpoint: Endpoint,
    /// Connected on first use (building the services touches nothing), and
    /// again after a failure.
    client: OnceCell<Docker>,
    compose: ComposeCli,
}

impl DockerEngine {
    /// The engine of `DOCKER_HOST` (a `unix://` socket) or of
    /// `/var/run/docker.sock`, connected on first use.
    #[must_use]
    pub fn new() -> Self {
        Self::at(Endpoint::of(std::env::var(HOST_VARIABLE).ok()))
    }

    fn at(endpoint: Endpoint) -> Self {
        Self {
            compose: ComposeCli::new(endpoint.socket().ok().map(str::to_owned)),
            endpoint,
            client: OnceCell::new(),
        }
    }

    async fn client(&self) -> Result<&Docker, DockerError> {
        let endpoint = self.endpoint.socket()?;
        self.client
            .get_or_try_init(|| async {
                let client = Docker::connect_with_unix(
                    endpoint,
                    REQUEST_TIMEOUT.as_secs(),
                    API_DEFAULT_VERSION,
                )
                .map_err(|error| unreachable(&error))?;
                // Speaks the version of the daemon, older or newer.
                let client = client
                    .negotiate_version()
                    .await
                    .map_err(|error| unreachable(&error))?;
                Ok(client.with_timeout(REQUEST_TIMEOUT))
            })
            .await
    }
}

impl Default for DockerEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the engine listens.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Endpoint {
    /// A local socket, `unix://…`.
    Local(String),
    /// A `DOCKER_HOST` the module does not drive (TCP, SSH…): it never acts
    /// on a remote host.
    Unsupported(String),
}

impl Endpoint {
    /// The endpoint named by the value of `DOCKER_HOST`, the default socket
    /// without it.
    fn of(docker_host: Option<String>) -> Self {
        match docker_host.filter(|host| !host.is_empty()) {
            None => Self::Local(DEFAULT_ENDPOINT.to_owned()),
            Some(host) if host.starts_with(UNIX_SCHEME) => Self::Local(host),
            Some(host) => Self::Unsupported(host),
        }
    }

    fn socket(&self) -> Result<&str, DockerError> {
        match self {
            Self::Local(socket) => Ok(socket),
            Self::Unsupported(host) => Err(DockerError::Unreachable(format!(
                "unsupported {HOST_VARIABLE} `{host}`: only a local {UNIX_SCHEME} socket"
            ))),
        }
    }
}

#[async_trait]
impl DockerBackend for DockerEngine {
    async fn containers(&self) -> Result<Vec<Container>, DockerError> {
        let options = ListContainersOptions {
            all: true,
            ..ListContainersOptions::default()
        };
        let summaries = self
            .client()
            .await?
            .list_containers(Some(options))
            .await
            .map_err(|error| api("list the containers", error))?;
        Ok(summaries.into_iter().filter_map(container_of).collect())
    }

    async fn changes(&self) -> Result<Box<dyn DockerChanges>, DockerError> {
        let options = EventsOptions {
            filters: Some(HashMap::from([("type".into(), vec!["container".into()])])),
            ..EventsOptions::default()
        };
        let events = self.client().await?.events(Some(options));
        Ok(Box::new(EngineChanges(Box::pin(events))))
    }

    async fn container(&self, id: &str, action: ContainerAction) -> Result<(), DockerError> {
        let client = self.client().await?;
        let result = match action {
            ContainerAction::Start => client.start_container(id, None).await,
            ContainerAction::Stop => {
                // The container's own stop timeout.
                let options = StopContainerOptions::default();
                client.stop_container(id, Some(options)).await
            }
            ContainerAction::Restart => client.restart_container(id, None).await,
            ContainerAction::Remove => {
                // Never forced: a running container is refused.
                let options = RemoveContainerOptions::default();
                client.remove_container(id, Some(options)).await
            }
        };
        result.map_err(|error| api(&format!("{action:?} container {id}"), error))
    }

    async fn compose(
        &self,
        target: &ComposeTarget,
        action: ComposeAction,
    ) -> Result<(), DockerError> {
        self.compose.run(target, action).await
    }

    async fn compose_services(&self, target: &ComposeTarget) -> Result<Vec<String>, DockerError> {
        self.compose.services(target).await
    }
}

type EventStream = Pin<Box<dyn Stream<Item = Result<EventMessage, BollardError>> + Send>>;

struct EngineChanges(EventStream);

#[async_trait]
impl DockerChanges for EngineChanges {
    async fn changed(&mut self) -> bool {
        loop {
            match self.0.next().await {
                Some(Ok(event)) => {
                    if is_relevant(&event) {
                        return true;
                    }
                }
                Some(Err(error)) => {
                    tracing::debug!(%error, "Docker events stopped");
                    return false;
                }
                None => return false,
            }
        }
    }
}

fn is_relevant(event: &EventMessage) -> bool {
    event.action.as_deref().is_some_and(|action| {
        RELEVANT_ACTIONS.contains(&action) || action.starts_with(HEALTH_ACTION_PREFIX)
    })
}

/// Failure to reach Docker at all.
fn unreachable(error: &BollardError) -> DockerError {
    DockerError::Unreachable(error.to_string())
}

/// Failure of a request: refused by Docker, or Docker unreachable.
fn api(action: &str, error: BollardError) -> DockerError {
    match error {
        BollardError::DockerResponseServerError { message, .. } => DockerError::Api {
            action: action.to_owned(),
            reason: message,
        },
        other => unreachable(&other),
    }
}

/// The container described by `summary`, `None` without an id.
fn container_of(summary: ContainerSummary) -> Option<Container> {
    let id = summary.id?;
    let name = summary
        .names
        .and_then(|names| names.into_iter().next())
        .map_or_else(
            || id.clone(),
            |name| name.trim_start_matches('/').to_owned(),
        );
    let status_text = summary.status.unwrap_or_default();
    let health = summary
        .health
        .and_then(|health| health.status)
        .and_then(health_of)
        .or_else(|| health_in_status(&status_text));
    let labels = summary.labels.unwrap_or_default();
    Some(Container {
        id,
        name,
        image: summary.image.unwrap_or_default(),
        status: summary.state.map_or(ContainerStatus::Unknown, status_of),
        health,
        compose: compose_labels(&labels),
    })
}

fn status_of(state: StateEnum) -> ContainerStatus {
    match state {
        StateEnum::CREATED => ContainerStatus::Created,
        StateEnum::RUNNING => ContainerStatus::Running,
        StateEnum::PAUSED => ContainerStatus::Paused,
        StateEnum::RESTARTING => ContainerStatus::Restarting,
        StateEnum::REMOVING => ContainerStatus::Removing,
        StateEnum::EXITED => ContainerStatus::Exited,
        StateEnum::DEAD => ContainerStatus::Dead,
        StateEnum::STOPPING => ContainerStatus::Stopping,
        StateEnum::EMPTY => ContainerStatus::Unknown,
    }
}

fn health_of(status: HealthEnum) -> Option<Health> {
    match status {
        HealthEnum::STARTING => Some(Health::Starting),
        HealthEnum::HEALTHY => Some(Health::Healthy),
        HealthEnum::UNHEALTHY => Some(Health::Unhealthy),
        HealthEnum::NONE | HealthEnum::EMPTY => None,
    }
}

/// Health from the status text (`Up 5 minutes (unhealthy)`), for Docker
/// before API 1.52, which does not report it in the list.
fn health_in_status(status: &str) -> Option<Health> {
    if status.ends_with("(unhealthy)") {
        Some(Health::Unhealthy)
    } else if status.ends_with("(healthy)") {
        Some(Health::Healthy)
    } else if status.ends_with("(health: starting)") {
        Some(Health::Starting)
    } else {
        None
    }
}

fn compose_labels(labels: &HashMap<String, String>) -> Option<ComposeLabels> {
    let project = labels
        .get(PROJECT_LABEL)
        .filter(|project| is_project_name(project))?
        .clone();
    let config_files = labels
        .get(CONFIG_FILES_LABEL)
        .map(|files| {
            files
                .split(CONFIG_FILES_SEPARATOR)
                .filter(|file| !file.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default();
    Some(ComposeLabels {
        project,
        working_dir: labels.get(WORKING_DIR_LABEL).map(PathBuf::from),
        config_files,
        service: labels.get(SERVICE_LABEL).cloned(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bollard::models::ContainerSummaryHealth;

    use super::*;

    fn summary(labels: &[(&str, &str)]) -> ContainerSummary {
        ContainerSummary {
            id: Some("abc123".into()),
            names: Some(vec!["/shop-web-1".into()]),
            image: Some("nginx:1".into()),
            state: Some(StateEnum::RUNNING),
            status: Some("Up 3 minutes (unhealthy)".into()),
            labels: Some(
                labels
                    .iter()
                    .map(|(key, value)| ((*key).into(), (*value).into()))
                    .collect(),
            ),
            ..ContainerSummary::default()
        }
    }

    #[test]
    fn a_project_name_compose_would_not_allow_makes_the_container_isolated() {
        for project in ["shop/db", "-shop", "Shop", "", "a b", "shop\nINJECTED"] {
            let container = container_of(summary(&[(PROJECT_LABEL, project)])).unwrap();
            assert_eq!(container.compose, None, "{project:?}");
        }
        let container = container_of(summary(&[(PROJECT_LABEL, "my-shop_2")])).unwrap();
        assert_eq!(container.compose.unwrap().project, "my-shop_2");
    }

    #[test]
    fn summaries_become_containers_with_their_compose_labels() {
        let container = container_of(summary(&[
            (PROJECT_LABEL, "shop"),
            (WORKING_DIR_LABEL, "/srv/shop"),
            (SERVICE_LABEL, "web"),
            (
                CONFIG_FILES_LABEL,
                "/srv/shop/compose.yaml,/srv/shop/dev.yaml",
            ),
        ]))
        .unwrap();
        assert_eq!(container.id, "abc123");
        assert_eq!(container.name, "shop-web-1");
        assert_eq!(container.image, "nginx:1");
        assert_eq!(container.status, ContainerStatus::Running);
        assert_eq!(
            container.health,
            Some(Health::Unhealthy),
            "from the status text"
        );
        let labels = container.compose.unwrap();
        assert_eq!(labels.project, "shop");
        assert_eq!(labels.service.as_deref(), Some("web"));
        assert_eq!(labels.working_dir, Some(PathBuf::from("/srv/shop")));
        assert_eq!(
            labels.config_files,
            [
                PathBuf::from("/srv/shop/compose.yaml"),
                PathBuf::from("/srv/shop/dev.yaml")
            ]
        );
    }

    #[test]
    fn missing_fields_are_tolerated() {
        let bare = ContainerSummary {
            id: Some("abc".into()),
            ..ContainerSummary::default()
        };
        let container = container_of(bare).unwrap();
        assert_eq!(container.name, "abc");
        assert_eq!(container.status, ContainerStatus::Unknown);
        assert_eq!((container.health, container.compose), (None, None));
        assert!(container_of(ContainerSummary::default()).is_none());

        let mut reported = summary(&[(PROJECT_LABEL, "solo")]);
        reported.health = Some(ContainerSummaryHealth {
            status: Some(HealthEnum::HEALTHY),
            ..ContainerSummaryHealth::default()
        });
        let container = container_of(reported).unwrap();
        assert_eq!(container.health, Some(Health::Healthy), "the API wins");
        let labels = container.compose.unwrap();
        assert_eq!(
            (labels.working_dir, labels.config_files),
            (None, Vec::new())
        );
    }

    #[test]
    fn health_checks_do_not_count_as_changes() {
        let event = |action: &str| EventMessage {
            action: Some(action.into()),
            ..EventMessage::default()
        };
        assert!(is_relevant(&event("die")));
        assert!(is_relevant(&event("health_status: unhealthy")));
        assert!(!is_relevant(&event(
            "exec_start: /bin/sh -c curl localhost"
        )));
        assert!(!is_relevant(&event("attach")));
        assert!(!is_relevant(&EventMessage::default()));
    }

    #[test]
    fn only_a_local_socket_is_supported() {
        let socket = |host: Option<&str>| {
            Endpoint::of(host.map(str::to_owned))
                .socket()
                .map(str::to_owned)
        };
        assert_eq!(socket(None).unwrap(), DEFAULT_ENDPOINT);
        assert_eq!(socket(Some("")).unwrap(), DEFAULT_ENDPOINT);
        let rootless = "unix:///run/user/1000/docker.sock";
        assert_eq!(socket(Some(rootless)).unwrap(), rootless);
        assert!(socket(Some("tcp://10.0.0.2:2375")).is_err());
        assert!(socket(Some("ssh://prod")).is_err());
    }

    /// A Docker daemon answering a few requests on a Unix socket, recording
    /// them as `METHOD path?query`.
    struct FakeDaemon {
        requests: Arc<std::sync::Mutex<Vec<String>>>,
        _server: tokio::task::JoinHandle<()>,
    }

    impl FakeDaemon {
        fn serve(socket: &std::path::Path) -> Self {
            let listener = tokio::net::UnixListener::bind(socket).unwrap();
            let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
            let recorded = Arc::clone(&requests);
            let server = tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(answer(stream, Arc::clone(&recorded)));
                }
            });
            Self {
                requests,
                _server: server,
            }
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    /// Serves the requests of one connection.
    async fn answer(stream: tokio::net::UnixStream, recorded: Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        let mut stream = BufReader::new(stream);
        loop {
            let mut head = Vec::new();
            loop {
                let mut line = String::new();
                if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                    return;
                }
                if line == "\r\n" {
                    break;
                }
                head.push(line);
            }
            let length: usize = head
                .iter()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|n| n.trim().parse().unwrap())
                })
                .unwrap_or(0);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            let mut words = head[0].split_whitespace();
            let (method, target) = (words.next().unwrap(), words.next().unwrap());
            // Without the API version prefix (`/v1.47/…`).
            let target = target
                .strip_prefix("/v")
                .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
                .and_then(|rest| rest.find('/').map(|slash| &rest[slash..]))
                .unwrap_or(target);
            recorded.lock().unwrap().push(format!("{method} {target}"));
            let path = target.split('?').next().unwrap();
            let (status, body) = match (method, path) {
                (_, "/version") => (
                    "200 OK",
                    r#"{"ApiVersion":"1.47","Version":"27.5.1"}"#.to_owned(),
                ),
                ("GET", "/containers/json") => (
                    "200 OK",
                    format!(
                        r#"[{{"Id":"abc","Names":["/web"],"Image":"nginx","State":"running","Status":"Up 2 hours","Labels":{{"{PROJECT_LABEL}":"shop"}}}}]"#
                    ),
                ),
                ("GET", "/events") => (
                    "200 OK",
                    concat!(
                        r#"{"Type":"container","Action":"exec_start: true"}"#,
                        "\n",
                        r#"{"Type":"container","Action":"die"}"#,
                        "\n",
                    )
                    .to_owned(),
                ),
                ("DELETE", "/containers/busy") => (
                    "409 Conflict",
                    r#"{"message":"container is running"}"#.to_owned(),
                ),
                ("POST" | "DELETE", _) => ("204 No Content", String::new()),
                _ => ("404 Not Found", r#"{"message":"no such route"}"#.to_owned()),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            if stream
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .is_err()
            {
                return;
            }
        }
    }

    fn engine_on(socket: &std::path::Path) -> DockerEngine {
        DockerEngine::at(Endpoint::Local(format!(
            "{UNIX_SCHEME}{}",
            socket.display()
        )))
    }

    #[tokio::test]
    async fn the_engine_speaks_the_docker_api() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("docker.sock");
        let daemon = FakeDaemon::serve(&socket);
        let engine = engine_on(&socket);

        let containers = engine.containers().await.unwrap();
        assert_eq!(containers.len(), 1);
        assert_eq!(
            (containers[0].name.as_str(), containers[0].id.as_str()),
            ("web", "abc")
        );
        for action in [
            ContainerAction::Start,
            ContainerAction::Stop,
            ContainerAction::Restart,
            ContainerAction::Remove,
        ] {
            engine.container("abc", action).await.unwrap();
        }
        let refused = engine.container("busy", ContainerAction::Remove).await;
        assert!(
            matches!(refused, Err(DockerError::Api { .. })),
            "{refused:?}"
        );

        let mut changes = engine.changes().await.unwrap();
        assert!(changes.changed().await, "the die event");
        assert!(!changes.changed().await, "then the stream ends");

        let requests = daemon.requests();
        let sent = |prefix: &str| {
            requests
                .iter()
                .find(|request| request.starts_with(prefix))
                .unwrap_or_else(|| panic!("no {prefix} in {requests:?}"))
                .clone()
        };
        assert!(sent("GET /containers/json").contains("all=true"));
        sent("POST /containers/abc/start");
        sent("POST /containers/abc/stop");
        sent("POST /containers/abc/restart");
        let removal = sent("DELETE /containers/abc");
        assert!(!removal.contains("force=true"), "never forced: {removal}");
        assert!(!removal.contains("v=true"), "volumes kept: {removal}");
        let events = sent("GET /events");
        assert!(
            events.contains("container"),
            "only container events: {events}"
        );
        assert_eq!(
            requests.iter().filter(|r| r.contains("/version")).count(),
            1,
            "connected once: {requests:?}"
        );
    }

    #[tokio::test]
    async fn a_missing_socket_is_unreachable_until_it_appears() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("docker.sock");
        let engine = engine_on(&socket);
        let missing = engine.containers().await;
        assert!(
            matches!(missing, Err(DockerError::Unreachable(_))),
            "{missing:?}"
        );
        let _daemon = FakeDaemon::serve(&socket);
        assert_eq!(engine.containers().await.unwrap().len(), 1);

        let remote = DockerEngine::at(Endpoint::of(Some("tcp://10.0.0.2:2375".into())));
        let refused = remote.containers().await;
        assert!(
            matches!(refused, Err(DockerError::Unreachable(_))),
            "{refused:?}"
        );
    }

    #[test]
    fn statuses_map_one_to_one() {
        assert_eq!(status_of(StateEnum::EXITED), ContainerStatus::Exited);
        assert_eq!(
            status_of(StateEnum::RESTARTING),
            ContainerStatus::Restarting
        );
        assert_eq!(
            health_in_status("Up 1 second (health: starting)"),
            Some(Health::Starting)
        );
        assert_eq!(health_in_status("Exited (0) 2 hours ago"), None);
        assert_eq!(health_of(HealthEnum::NONE), None);
    }
}
