//! What the Docker module needs from a Docker host: the [`DockerBackend`]
//! trait, its changes and failures.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use super::model::{ComposeTarget, Container};

/// Action on one container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerAction {
    /// Starts it (nothing to do when running).
    Start,
    /// Stops it, within its own stop timeout (nothing to do when stopped).
    Stop,
    /// Stops then starts it.
    Restart,
    /// Removes it, with its anonymous volumes kept; refused while running.
    Remove,
}

/// What `docker compose up` does besides creating and starting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpVariant {
    /// `up --detach`.
    Plain,
    /// `up --detach --build`: rebuilds the images first.
    Build,
    /// `up --detach --pull=always`: pulls the images first.
    Pull,
    /// `up --no-start`: creates the containers (and builds or pulls what is
    /// missing) without starting them.
    NoStart,
}

/// What a compose action is about: the whole project, or one service.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ComposeScope {
    /// Every service of the project.
    Project,
    /// This service only.
    Service(String),
}

impl ComposeScope {
    /// The scope of an action on `service` (`None`: the project).
    #[must_use]
    pub fn of(service: Option<&str>) -> Self {
        service.map_or(Self::Project, |service| Self::Service(service.to_owned()))
    }

    /// Whether an action of this scope works on `service` of the project
    /// (`None`: a container that records no service, only reached by an
    /// action on the whole project).
    #[must_use]
    pub fn covers(&self, service: Option<&str>) -> bool {
        match self {
            Self::Project => true,
            Self::Service(scoped) => service == Some(scoped.as_str()),
        }
    }

    /// The service, if the scope is one.
    #[must_use]
    pub fn service(&self) -> Option<&str> {
        match self {
            Self::Project => None,
            Self::Service(service) => Some(service),
        }
    }
}

/// Action on a compose project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComposeAction {
    /// `docker compose up`.
    Up(UpVariant),
    /// `docker compose down`, with `--volumes` when `volumes` is set
    /// (removes the data of the named volumes of the project).
    Down {
        /// Also removes the volumes.
        volumes: bool,
    },
}

/// Failure to talk to Docker.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DockerError {
    /// Docker cannot be reached (not running, socket missing, permission
    /// denied, `DOCKER_HOST` other than a local `unix://` socket).
    #[error("cannot reach Docker: {0}")]
    Unreachable(String),
    /// Docker refused or failed a request.
    #[error("Docker failed to {action}: {reason}")]
    Api {
        /// What was asked.
        action: String,
        /// What Docker answered.
        reason: String,
    },
    /// The compose tool cannot be started.
    #[error("cannot run `{program}`: {source}")]
    Spawn {
        /// Program run.
        program: String,
        /// Why it could not start.
        source: std::io::Error,
    },
    /// `docker compose` failed.
    #[error("`{command}` failed ({status}): {stderr}")]
    ComposeFailed {
        /// Arguments given to `docker`.
        command: String,
        /// Exit status.
        status: String,
        /// End of what it reported.
        stderr: String,
    },
    /// What `docker compose` printed could not be read.
    #[error("cannot read the output of `{command}`: {reason}")]
    Unreadable {
        /// Arguments given to `docker`.
        command: String,
        /// What went wrong.
        reason: String,
    },
    /// A compose file, or one of its directories, could be changed by
    /// someone else than the user: compose is not run on it.
    #[error("untrusted compose path `{path}`: {reason}")]
    Untrusted {
        /// The path refused.
        path: String,
        /// Why.
        reason: String,
    },
    /// `docker compose` did not end in time.
    #[error("`{command}` did not end within {timeout:?}")]
    Timeout {
        /// Arguments given to `docker`.
        command: String,
        /// Time given.
        timeout: Duration,
    },
    /// The project cannot be acted on this way (no compose file known,
    /// relative path…).
    #[error("invalid compose project: {0}")]
    InvalidTarget(String),
    /// The backend panicked (a bug): the call is treated as failed.
    #[error("the Docker backend panicked")]
    Panicked,
}

/// Changes notified by Docker, until the stream ends.
#[async_trait]
pub trait DockerChanges: Send {
    /// Waits for a change of a container (created, started, stopped,
    /// health…); `false` when Docker stopped notifying (to listen again).
    async fn changed(&mut self) -> bool;
}

/// A Docker host.
///
/// Calls may take a while (a container stops within its timeout, `compose
/// up` may build images): the module only makes them from its background
/// tasks.
#[async_trait]
pub trait DockerBackend: Send + Sync + fmt::Debug {
    /// Every container, running or not.
    async fn containers(&self) -> Result<Vec<Container>, DockerError>;

    /// Starts listening to the changes of the containers.
    async fn changes(&self) -> Result<Box<dyn DockerChanges>, DockerError>;

    /// Applies `action` to the container `id`.
    async fn container(&self, id: &str, action: ContainerAction) -> Result<(), DockerError>;

    /// Applies `action` to the compose project `target`, within `scope`.
    async fn compose(
        &self,
        target: &ComposeTarget,
        scope: &ComposeScope,
        action: ComposeAction,
    ) -> Result<(), DockerError>;

    /// The services the compose files of `target` define, by name, whether
    /// or not they have containers.
    async fn compose_services(&self, target: &ComposeTarget) -> Result<Vec<String>, DockerError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scope_covers_the_services_it_names() {
        assert!(ComposeScope::Project.covers(Some("web")));
        assert!(
            ComposeScope::Project.covers(None),
            "a container with no service"
        );
        let web = ComposeScope::of(Some("web"));
        assert!(web.covers(Some("web")));
        assert!(!web.covers(Some("db")));
        assert!(!web.covers(None));
        assert_eq!(
            (web.service(), ComposeScope::of(None)),
            (Some("web"), ComposeScope::Project)
        );
    }
}
