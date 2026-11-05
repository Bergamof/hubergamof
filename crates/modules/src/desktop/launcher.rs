//! Starts applications outside the manager's service.
//!
//! Under systemd, stopping or restarting a service kills every process of
//! its control group: an application started as a plain child of the
//! manager would close with it. Applications are therefore started in a
//! scope of their own (`systemd-run --user --scope`), as the desktop's
//! launchers do; `systemd-run` then becomes the application (`exec`).

use std::ffi::OsString;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use super::backend::DesktopError;
use super::model::Launch;

/// How long a launch is watched: a program ending in failure within this
/// time (unknown desktop entry, missing program) is reported as failed; past
/// it, it runs on unwatched.
pub(super) const LAUNCH_CHECK: Duration = Duration::from_secs(2);

/// Puts each application in a transient scope of the user's manager, in
/// `app.slice` like the desktop's launchers.
const SCOPE_PREFIX: [&str; 7] = [
    "systemd-run",
    "--user",
    "--scope",
    "--collect",
    "--quiet",
    "--slice=app.slice",
    "--",
];

/// Starts programs, through a fixed prefix.
#[derive(Debug, Clone)]
pub(super) struct Launcher {
    prefix: Vec<OsString>,
}

impl Launcher {
    /// Programs started each in a scope of their own.
    pub(super) fn scoped() -> Self {
        Self {
            prefix: SCOPE_PREFIX.iter().map(OsString::from).collect(),
        }
    }

    /// Programs started directly (tests: no user manager).
    #[cfg(test)]
    pub(super) fn direct() -> Self {
        Self { prefix: Vec::new() }
    }

    /// Starts `launch`, without a shell, from the user's home folder (not
    /// the manager's), its outputs discarded (an application may write for
    /// as long as it runs); waits up to [`LAUNCH_CHECK`] for an early
    /// failure. The process is reaped in the background once it ends.
    pub(super) async fn launch(&self, launch: &Launch) -> Result<(), DesktopError> {
        let mut command = self.command(launch);
        if let Some(home) = crate::paths::user_home() {
            command.current_dir(home);
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|source| DesktopError::Launch {
                // The program actually run: `systemd-run` may be the one
                // missing.
                program: command
                    .as_std()
                    .get_program()
                    .to_string_lossy()
                    .into_owned(),
                reason: source.to_string(),
            })?;
        match tokio::time::timeout(LAUNCH_CHECK, child.wait()).await {
            Ok(Ok(status)) if !status.success() => Err(DesktopError::Launch {
                program: launch.program.display().to_string(),
                reason: status.to_string(),
            }),
            Ok(_) => Ok(()),
            Err(_) => {
                tokio::spawn(async move {
                    // Reaped when it ends; its status no longer matters.
                    let _ended = child.wait().await;
                });
                Ok(())
            }
        }
    }

    fn command(&self, launch: &Launch) -> Command {
        let mut command = match self.prefix.split_first() {
            Some((wrapper, wrapper_args)) => {
                let mut command = Command::new(wrapper);
                command.args(wrapper_args).arg(&launch.program);
                command
            }
            None => Command::new(&launch.program),
        };
        command.args(&launch.args);
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(program: &str, args: &[&str]) -> Launch {
        Launch {
            program: program.into(),
            args: args.iter().map(OsString::from).collect(),
        }
    }

    #[test]
    fn programs_are_scoped_with_separate_arguments() {
        let command = Launcher::scoped().command(&launch("/opt/idea/bin/idea", &["/a b; rm -rf"]));
        let std = command.as_std();
        assert_eq!(std.get_program(), "systemd-run");
        let args: Vec<_> = std.get_args().collect();
        assert_eq!(args.len(), SCOPE_PREFIX.len() + 1);
        assert_eq!(args[SCOPE_PREFIX.len() - 2], "--");
        assert_eq!(args[SCOPE_PREFIX.len() - 1], "/opt/idea/bin/idea");
        assert_eq!(args[SCOPE_PREFIX.len()], "/a b; rm -rf");
    }

    #[tokio::test]
    async fn a_successful_launch_is_fine() {
        Launcher::direct()
            .launch(&launch("true", &[]))
            .await
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_program_still_running_is_left_running() {
        let launcher = Launcher::direct();
        let started = tokio::time::Instant::now();
        launcher.launch(&launch("sleep", &["5"])).await.unwrap();
        assert!(started.elapsed() >= LAUNCH_CHECK);
    }

    #[tokio::test]
    async fn early_failures_are_reported() {
        let launcher = Launcher::direct();
        let failed = launcher.launch(&launch("false", &[])).await.unwrap_err();
        assert!(matches!(failed, DesktopError::Launch { .. }), "{failed}");
        let missing = launcher
            .launch(&launch("/nonexistent/bergadeck-app", &[]))
            .await
            .unwrap_err();
        assert!(
            missing.to_string().contains("/nonexistent/bergadeck-app"),
            "{missing}"
        );
        let without_wrapper = Launcher {
            prefix: vec!["/nonexistent/systemd-run".into()],
        };
        let error = without_wrapper
            .launch(&launch("true", &[]))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("/nonexistent/systemd-run"),
            "the program missing is named: {error}"
        );
    }
}
