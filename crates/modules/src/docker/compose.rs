//! `docker compose`, run as a program: the Docker API knows nothing of
//! compose projects.
//!
//! The tool is run directly (never through a shell), each argument passed
//! separately. Every value is given in the `--option=value` form, and paths
//! must be absolute, so that no value (a project name or a path read from
//! container labels, which anyone allowed to create containers can set) is
//! ever taken for an option. `docker` is looked up in the `PATH` of the
//! manager, trusted like the rest of the user's session.
//!
//! Compose talks to the Docker host the API reads ([`ComposeCli::new`]),
//! never to the active Docker context: the keys show the local containers,
//! a `down` must not reach another host. Before compose reads a project,
//! its files, its directory and its `.env` must belong to the user (or
//! root) and be writable by nobody else ([`check_trusted`]), and compose is
//! given their resolved paths (no link left to swap): a compose file
//! dropped by someone else would run their containers with the user's
//! access. Known limit: what the compose files themselves name (`include`,
//! `extends.file`, `env_file`) is not checked; the user chose those files.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command;

use super::backend::{ComposeAction, ComposeScope, DockerError, UpVariant};
use super::model::ComposeTarget;
use crate::process::{self, OutputLimits, RunError};
use crate::trust;

/// Program run, looked up in `PATH`.
const DOCKER: &str = "docker";

/// Environment variable selecting a Docker context, removed for compose.
const CONTEXT_VARIABLE: &str = "DOCKER_CONTEXT";

/// Longest wait for a compose command: `up` may build images.
const COMPOSE_TIMEOUT: Duration = Duration::from_mins(15);

/// Longest wait for the list of services: it only reads the files.
const SERVICES_TIMEOUT: Duration = Duration::from_secs(30);

/// End of the error output kept in errors and logs (compose reports its
/// progress there, and what failed last).
const MAX_ERROR_BYTES: usize = 4 * 1024;

/// What lists the services of a project.
const SERVICES_COMMAND: &str = "config --services";

/// Largest list of services read from compose.
const MAX_SERVICES_BYTES: usize = 64 * 1024;

/// Most services of a project taken into account.
const MAX_SERVICES: usize = 200;

/// Longest service name.
const MAX_SERVICE_NAME_BYTES: usize = 128;

/// Variables of a project, read by compose from its directory.
const ENV_FILE: &str = ".env";

/// Runs `docker compose`.
#[derive(Debug, Clone)]
pub(super) struct ComposeCli {
    program: OsString,
    /// Arguments given before those of each call (empty for `docker`).
    leading_args: Vec<OsString>,
    /// Docker host, `unix://…`; `None` when the API reaches none.
    host: Option<String>,
    timeout: Duration,
    /// Whether the files must be trusted ([`check_trusted`]).
    check_files: bool,
}

impl ComposeCli {
    /// Runs `docker` from `PATH` on `host` (`unix://…`, the one the API
    /// reads; `None`: compose is refused).
    pub(super) fn new(host: Option<String>) -> Self {
        Self {
            program: DOCKER.into(),
            leading_args: Vec::new(),
            host,
            timeout: COMPOSE_TIMEOUT,
            check_files: true,
        }
    }

    /// Runs the shell script `script` instead of `docker` (tests), on files
    /// that need not exist; read by `/bin/sh` rather than executed, see
    /// `PactlBackend::shell_script`.
    #[cfg(test)]
    pub(super) fn shell_script(script: impl Into<OsString>, timeout: Duration) -> Self {
        Self {
            program: "/bin/sh".into(),
            leading_args: vec![script.into()],
            host: Some("unix:///run/test.sock".into()),
            timeout,
            check_files: false,
        }
    }

    /// Applies `action` to `target`.
    pub(super) async fn run(
        &self,
        target: &ComposeTarget,
        scope: &ComposeScope,
        action: ComposeAction,
    ) -> Result<(), DockerError> {
        let limits = OutputLimits {
            stdout: None,
            stderr: MAX_ERROR_BYTES,
        };
        self.invoke(target, limits, self.timeout, |host, target| {
            compose_args(host, target, scope, action)
        })
        .await
        .map(drop)
    }

    /// The services the files of `target` define (`config --services`),
    /// with or without containers.
    pub(super) async fn services(
        &self,
        target: &ComposeTarget,
    ) -> Result<Vec<String>, DockerError> {
        let limits = OutputLimits {
            stdout: Some(MAX_SERVICES_BYTES),
            stderr: MAX_ERROR_BYTES,
        };
        let timeout = self.timeout.min(SERVICES_TIMEOUT);
        let finished = self.invoke(target, limits, timeout, services_args).await?;
        let listed = String::from_utf8_lossy(&finished.stdout);
        listed
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(MAX_SERVICES)
            .map(|name| {
                if is_service_name(name) {
                    Ok(name.to_owned())
                } else {
                    Err(DockerError::Unreadable {
                        command: SERVICES_COMMAND.into(),
                        reason: "a service name is not usable".into(),
                    })
                }
            })
            .collect()
    }

    /// Runs the compose command `arguments` builds for `target`, checking
    /// its files first, and returns it when it succeeds.
    async fn invoke(
        &self,
        target: &ComposeTarget,
        limits: OutputLimits,
        timeout: Duration,
        arguments: impl FnOnce(&str, &ComposeTarget) -> Result<Vec<OsString>, DockerError>,
    ) -> Result<process::Finished, DockerError> {
        let host = self
            .host
            .as_deref()
            .ok_or_else(|| DockerError::Unreachable("no local Docker socket for compose".into()))?;
        let trusted;
        let target = if self.check_files {
            trusted = trusted_target(target).await?;
            &trusted
        } else {
            target
        };
        let args = arguments(host, target)?;
        let command = self.command(&args);
        let line = || command_line(&args);
        let finished =
            process::run(command, timeout, limits)
                .await
                .map_err(|error| match error {
                    RunError::Spawn(source) => DockerError::Spawn {
                        program: self.program.to_string_lossy().into_owned(),
                        source,
                    },
                    RunError::Timeout => DockerError::Timeout {
                        command: line(),
                        timeout,
                    },
                    RunError::Output(reason) => DockerError::Unreadable {
                        command: line(),
                        reason,
                    },
                })?;
        if finished.status.success() {
            Ok(finished)
        } else {
            Err(DockerError::ComposeFailed {
                command: line(),
                status: finished.status.to_string(),
                stderr: finished.stderr,
            })
        }
    }
}

impl ComposeCli {
    /// `docker args…`, without the Docker context of the session.
    fn command(&self, args: &[OsString]) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.leading_args)
            .args(args)
            .env_remove(CONTEXT_VARIABLE);
        command
    }
}

/// Arguments of `docker` for `action` on `target`, through `host`.
fn compose_args(
    host: &str,
    target: &ComposeTarget,
    scope: &ComposeScope,
    action: ComposeAction,
) -> Result<Vec<OsString>, DockerError> {
    let needs_files = target.project.is_none() || matches!(action, ComposeAction::Up(_));
    let mut args = base_args(host, target, needs_files)?;
    match action {
        ComposeAction::Up(variant) => {
            args.push("up".into());
            match variant {
                UpVariant::Plain => args.push("--detach".into()),
                UpVariant::Build => args.extend(["--detach".into(), "--build".into()]),
                // Nothing runs, so there is nothing to detach from.
                UpVariant::NoStart => args.push("--no-start".into()),
            }
        }
        ComposeAction::Down { volumes } => {
            args.push("down".into());
            if volumes {
                args.push("--volumes".into());
            }
        }
    }
    let services = scope.services();
    if !services.is_empty() {
        // Even a name that passed the check is never read as an option.
        args.push("--".into());
        for service in services {
            args.push(service_argument(service)?);
        }
    }
    Ok(args)
}

/// Arguments of `docker` listing the services the files of `target` define.
fn services_args(host: &str, target: &ComposeTarget) -> Result<Vec<OsString>, DockerError> {
    let mut args = base_args(host, target, true)?;
    args.extend(SERVICES_COMMAND.split(' ').map(OsString::from));
    Ok(args)
}

/// The arguments naming the host, the project and its files.
fn base_args(
    host: &str,
    target: &ComposeTarget,
    needs_files: bool,
) -> Result<Vec<OsString>, DockerError> {
    if target.files.is_empty() && needs_files {
        return Err(DockerError::InvalidTarget(
            "no compose file known for this project".into(),
        ));
    }
    let mut args: Vec<OsString> = vec![
        option("--host", host),
        "compose".into(),
        "--ansi=never".into(),
    ];
    if let Some(project) = &target.project {
        args.push(option("--project-name", project));
    }
    if let Some(directory) = &target.directory {
        args.push(option("--project-directory", absolute(directory)?));
    }
    for file in &target.files {
        args.push(option("--file", absolute(file)?));
    }
    Ok(args)
}

/// The service of an action, last: checked, since a name read from container
/// labels (anyone allowed to create containers sets them) must never be
/// taken for an option.
fn service_argument(service: &str) -> Result<OsString, DockerError> {
    if is_service_name(service) {
        Ok(service.into())
    } else {
        Err(DockerError::InvalidTarget(format!(
            "{service:?} is not a usable service name"
        )))
    }
}

/// Whether `name` is a service name compose allows (letters, digits, `.`,
/// `_` and `-`, not starting with a symbol): never an option.
pub(super) fn is_service_name(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && name.len() <= MAX_SERVICE_NAME_BYTES
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `name=value` as one argument.
fn option(name: &str, value: impl AsRef<OsStr>) -> OsString {
    let mut argument = OsString::from(name);
    argument.push("=");
    argument.push(value);
    argument
}

fn absolute(path: &Path) -> Result<&Path, DockerError> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(DockerError::InvalidTarget(format!(
            "`{}` is not an absolute path",
            path.display()
        )))
    }
}

/// `target` with its paths resolved, once they and the `.env` of the
/// project are trusted ([`check_trusted`]).
async fn trusted_target(target: &ComposeTarget) -> Result<ComposeTarget, DockerError> {
    for path in target.directory.iter().chain(&target.files) {
        absolute(path)?;
    }
    let directory = match &target.directory {
        Some(directory) => Some(check_trusted(directory).await?),
        None => None,
    };
    let mut files = Vec::with_capacity(target.files.len());
    for file in &target.files {
        files.push(check_trusted(file).await?);
    }
    let project_directory = directory.clone().or_else(|| {
        files
            .first()
            .and_then(|file| file.parent())
            .map(PathBuf::from)
    });
    if let Some(env) = project_directory.map(|directory| directory.join(ENV_FILE))
        && tokio::fs::symlink_metadata(&env).await.is_ok()
    {
        check_trusted(&env).await?;
    }
    Ok(ComposeTarget::new(target.project.clone(), directory, files))
}

/// `path` with its links resolved, once found trusted (see
/// [`trust::trusted`]).
async fn check_trusted(path: &Path) -> Result<PathBuf, DockerError> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || trust::trusted(&path))
        .await
        .map_err(|error| DockerError::Untrusted {
            path: String::new(),
            reason: error.to_string(),
        })?
        .map_err(|untrusted| DockerError::Untrusted {
            path: untrusted.path,
            reason: untrusted.reason,
        })
}

/// The arguments of a call as one line, for errors.
fn command_line(args: &[OsString]) -> String {
    args.iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    const HOST: &str = "unix:///run/user/1000/docker.sock";
    const PATIENCE: Duration = Duration::from_secs(10);

    fn running(project: &str) -> ComposeTarget {
        ComposeTarget::new(
            Some(project.into()),
            Some(format!("/srv/{project}").into()),
            vec![
                format!("/srv/{project}/compose.yaml").into(),
                format!("/srv/{project}/compose.override.yaml").into(),
            ],
        )
    }

    fn declared(file: &str) -> ComposeTarget {
        ComposeTarget::new(None, None, vec![file.into()])
    }

    fn line(target: &ComposeTarget, action: ComposeAction) -> String {
        command_line(&compose_args(HOST, target, &ComposeScope::Project, action).unwrap())
    }

    fn line_for(service: &str, action: ComposeAction) -> String {
        let scope = ComposeScope::of(Some(service));
        command_line(&compose_args(HOST, &running("shop"), &scope, action).unwrap())
    }

    #[test]
    fn every_value_is_given_as_one_option_argument_on_the_api_host() {
        assert_eq!(
            line(&running("shop"), ComposeAction::Up(UpVariant::Build)),
            format!(
                "--host={HOST} compose --ansi=never --project-name=shop \
                 --project-directory=/srv/shop --file=/srv/shop/compose.yaml \
                 --file=/srv/shop/compose.override.yaml up --detach --build"
            )
        );
        let blog = declared("/home/me/-blog/compose.yaml");
        assert_eq!(
            line(&blog, ComposeAction::Up(UpVariant::Plain)),
            format!(
                "--host={HOST} compose --ansi=never --file=/home/me/-blog/compose.yaml \
                 up --detach"
            )
        );
        assert!(line(&blog, ComposeAction::Up(UpVariant::NoStart)).ends_with(" up --no-start"));
        // A project whose labels name no file can still be taken down.
        let unknown_files = ComposeTarget::new(Some("--rm".into()), None, Vec::new());
        assert_eq!(
            line(&unknown_files, ComposeAction::Down { volumes: true }),
            format!("--host={HOST} compose --ansi=never --project-name=--rm down --volumes")
        );
        assert!(line(&running("shop"), ComposeAction::Down { volumes: false }).ends_with(" down"));
    }

    #[test]
    fn relative_paths_and_projects_without_files_are_refused() {
        let up = ComposeAction::Up(UpVariant::Plain);
        let refused = |target: &ComposeTarget, action| {
            compose_args(HOST, target, &ComposeScope::Project, action).is_err()
        };
        assert!(matches!(
            compose_args(HOST, &declared("compose.yaml"), &ComposeScope::Project, up),
            Err(DockerError::InvalidTarget(_))
        ));
        let mut relative_directory = running("shop");
        relative_directory.directory = Some("srv".into());
        assert!(refused(&relative_directory, up));
        let without_files = ComposeTarget::new(Some("shop".into()), None, Vec::new());
        assert!(refused(&without_files, up), "up needs the files");
        let nothing = ComposeTarget::default();
        assert!(refused(&nothing, ComposeAction::Down { volumes: false }));
    }

    /// A fake `docker` writing its arguments, one per line, then the Docker
    /// context it was given (`none`), in `calls.log`; then running `then`.
    fn fake_docker(dir: &Path, then: &str, timeout: Duration) -> ComposeCli {
        let script = dir.join("docker.sh");
        let log = dir.join("calls.log").display().to_string();
        let body = format!(
            "printf '%s\\n' \"$@\" > '{log}'\n\
             printf '%s\\n' \"${{{CONTEXT_VARIABLE}:-none}}\" >> '{log}'\n\
             {then}\n"
        );
        std::fs::write(&script, body).unwrap();
        ComposeCli::shell_script(script, timeout)
    }

    fn logged(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls.log"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[tokio::test]
    async fn compose_receives_each_argument_apart_without_the_docker_context() {
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_docker(dir.path(), "exit 0", PATIENCE);
        let spaced = declared("/home/me/my projects/compose.yaml");
        cli.run(
            &spaced,
            &ComposeScope::Project,
            ComposeAction::Down { volumes: false },
        )
        .await
        .unwrap();
        assert_eq!(
            logged(dir.path()),
            [
                "--host=unix:///run/test.sock",
                "compose",
                "--ansi=never",
                "--file=/home/me/my projects/compose.yaml",
                "down",
                "none",
            ]
        );
    }

    #[test]
    fn a_service_scope_narrows_the_action_and_comes_last() {
        assert!(
            line_for("web", ComposeAction::Up(UpVariant::Build))
                .ends_with(" up --detach --build -- web")
        );
        assert!(
            line_for("web", ComposeAction::Up(UpVariant::NoStart))
                .ends_with(" up --no-start -- web")
        );
        assert!(
            line_for("db-2", ComposeAction::Down { volumes: true })
                .ends_with(" down --volumes -- db-2")
        );
        assert!(
            line(&running("shop"), ComposeAction::Down { volumes: false }).ends_with(" down"),
            "no service: the whole project"
        );
    }

    #[test]
    fn several_services_share_one_separator_and_keep_their_order() {
        let scope = ComposeScope::of_services(vec!["app2".into(), "app1".into()]);
        let line =
            |action| command_line(&compose_args(HOST, &running("shop"), &scope, action).unwrap());
        assert!(line(ComposeAction::Up(UpVariant::Plain)).ends_with(" up --detach -- app2 app1"));
        assert!(
            line(ComposeAction::Down { volumes: true }).ends_with(" down --volumes -- app2 app1")
        );
    }

    #[test]
    fn one_invalid_service_among_several_refuses_the_whole_action() {
        let scope = ComposeScope::of_services(vec!["app1".into(), "--rm".into(), "app2".into()]);
        assert!(matches!(
            compose_args(
                HOST,
                &running("shop"),
                &scope,
                ComposeAction::Up(UpVariant::Plain)
            ),
            Err(DockerError::InvalidTarget(_))
        ));
    }

    #[test]
    fn a_service_name_is_never_taken_for_an_option() {
        for name in [
            "--rm",
            "-d",
            "",
            "a b",
            "web;ls",
            "../web",
            &"x".repeat(MAX_SERVICE_NAME_BYTES + 1),
        ] {
            let scope = ComposeScope::of(Some(name));
            assert!(
                matches!(
                    compose_args(
                        HOST,
                        &running("shop"),
                        &scope,
                        ComposeAction::Up(UpVariant::Plain)
                    ),
                    Err(DockerError::InvalidTarget(_))
                ),
                "{name:?}"
            );
        }
    }

    #[tokio::test]
    async fn the_services_are_read_from_config_and_each_name_is_checked() {
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_docker(
            dir.path(),
            "printf 'web\n\n  db  \nredis-cache\n'",
            PATIENCE,
        );
        let services = cli.services(&running("shop")).await.unwrap();
        assert_eq!(services, ["web", "db", "redis-cache"]);
        let logged = logged(dir.path());
        assert_eq!(logged[logged.len() - 3..], ["config", "--services", "none"]);

        let hostile = fake_docker(dir.path(), "printf 'web\n--rm\n'", PATIENCE);
        assert!(matches!(
            hostile.services(&running("shop")).await,
            Err(DockerError::Unreadable { .. })
        ));
        let failing = fake_docker(dir.path(), "echo 'invalid compose' >&2; exit 1", PATIENCE);
        assert!(matches!(
            failing.services(&running("shop")).await,
            Err(DockerError::ComposeFailed { .. })
        ));
        assert!(
            matches!(
                cli.services(&ComposeTarget::default()).await,
                Err(DockerError::InvalidTarget(_))
            ),
            "the files are needed to read the services"
        );
    }

    #[tokio::test]
    async fn a_failure_is_reported_with_the_end_of_its_errors() {
        let dir = tempfile::tempdir().unwrap();
        let failing = fake_docker(
            dir.path(),
            "head -c 20000 /dev/zero | tr '\\0' p >&2; echo 'no such service' >&2; exit 17",
            PATIENCE,
        );
        let error = failing
            .run(
                &running("shop"),
                &ComposeScope::Project,
                ComposeAction::Up(UpVariant::Plain),
            )
            .await
            .unwrap_err();
        let DockerError::ComposeFailed { status, stderr, .. } = &error else {
            panic!("failure expected, got {error}");
        };
        assert!(status.contains("17"), "{status}");
        assert!(stderr.ends_with("no such service"), "{stderr}");
        assert!(stderr.len() <= MAX_ERROR_BYTES + process::TRUNCATION_MARK.len());
    }

    #[tokio::test]
    async fn a_stuck_compose_times_out_and_a_missing_one_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let stuck = fake_docker(dir.path(), "sleep 30", Duration::from_millis(50));
        let error = stuck
            .run(
                &running("shop"),
                &ComposeScope::Project,
                ComposeAction::Down { volumes: false },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, DockerError::Timeout { .. }), "{error}");

        let mut missing = ComposeCli::new(Some(HOST.into()));
        missing.program = "/nonexistent/docker".into();
        missing.check_files = false;
        let error = missing
            .run(
                &running("shop"),
                &ComposeScope::Project,
                ComposeAction::Down { volumes: false },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, DockerError::Spawn { .. }), "{error}");
    }

    #[tokio::test]
    async fn without_a_local_host_compose_is_refused() {
        let cli = ComposeCli::new(None);
        let error = cli
            .run(
                &running("shop"),
                &ComposeScope::Project,
                ComposeAction::Down { volumes: false },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, DockerError::Unreachable(_)), "{error}");
    }

    #[tokio::test]
    async fn untrusted_files_are_never_given_to_compose() {
        let cli = ComposeCli::new(Some(HOST.into()));
        let error = cli
            .run(
                &declared("/nonexistent/compose.yaml"),
                &ComposeScope::Project,
                ComposeAction::Up(UpVariant::Plain),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, DockerError::Untrusted { .. }), "{error}");
    }

    fn set_mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, Permissions::from_mode(mode)).unwrap();
    }

    #[tokio::test]
    async fn compose_is_given_resolved_paths_and_a_trusted_env_file() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        set_mode(&project, 0o755);
        let file = project.join("compose.yaml");
        std::fs::write(&file, "services: {}\n").unwrap();
        set_mode(&file, 0o644);
        let link = dir.path().join("link.yaml");
        std::os::unix::fs::symlink(&file, &link).unwrap();

        let resolved = trusted_target(&declared(link.to_str().unwrap()))
            .await
            .unwrap();
        assert_eq!(
            resolved.files,
            [file.canonicalize().unwrap()],
            "not the link"
        );

        let env = project.join(ENV_FILE);
        std::fs::write(&env, "DATA=/\n").unwrap();
        set_mode(&env, 0o666);
        let error = trusted_target(&declared(file.to_str().unwrap()))
            .await
            .unwrap_err();
        assert!(matches!(error, DockerError::Untrusted { .. }), "{error}");
        set_mode(&env, 0o600);
        trusted_target(&declared(file.to_str().unwrap()))
            .await
            .unwrap();
        let relative = trusted_target(&declared("compose.yaml")).await;
        assert!(
            matches!(relative, Err(DockerError::InvalidTarget(_))),
            "{relative:?}"
        );
    }

    #[test]
    fn compose_never_inherits_the_docker_context() {
        let cli = ComposeCli::new(Some(HOST.into()));
        let command = cli.command(&["compose".into()]);
        let removed: Vec<_> = command
            .as_std()
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_owned())
            .collect();
        assert_eq!(removed, [OsString::from(CONTEXT_VARIABLE)]);
    }

    #[tokio::test]
    async fn the_project_directory_is_resolved_and_checked_with_its_env_file() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        let elsewhere = dir.path().join("elsewhere");
        for directory in [&project, &elsewhere] {
            std::fs::create_dir(directory).unwrap();
            set_mode(directory, 0o755);
        }
        let file = elsewhere.join("compose.yaml");
        std::fs::write(&file, "services: {}\n").unwrap();
        set_mode(&file, 0o644);
        let link = dir.path().join("project-link");
        std::os::unix::fs::symlink(&project, &link).unwrap();
        let running =
            ComposeTarget::new(Some("shop".into()), Some(link.clone()), vec![file.clone()]);

        let resolved = trusted_target(&running).await.unwrap();
        assert_eq!(
            resolved.directory,
            Some(project.canonicalize().unwrap()),
            "not the link"
        );

        // The `.env` read is the one of the project directory, not of the file's.
        let env = project.join(ENV_FILE);
        std::fs::write(&env, "DATA=/\n").unwrap();
        set_mode(&env, 0o666);
        assert!(trusted_target(&running).await.is_err());
        std::fs::remove_file(&env).unwrap();

        set_mode(&project, 0o777);
        assert!(trusted_target(&running).await.is_err());
        // Sticky is fine above, not for the project itself: others could
        // add their own `.env`.
        set_mode(&project, 0o1777);
        assert!(trusted_target(&running).await.is_err());
        set_mode(&project, 0o755);
        trusted_target(&running).await.unwrap();
    }
}
