//! Asking `ssh` itself whether it can read what is about to be written.
//!
//! The options written by the person are free: a typo or an option this
//! version of `ssh` does not know makes `ssh` refuse the whole configuration,
//! for every host. The file is therefore tried before it replaces the one
//! `ssh` reads: `ssh -G` prints the configuration it would use without
//! connecting, and fails on what it cannot read.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read as _};
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use hubergamof_protocol::SshConfig;
use rustix::process::{Pid, Signal, kill_process_group};

use super::apply::{SshFiles, SshFilesError};
use super::render::{COMMON_PATTERN, MANAGED_DIR};
use crate::machine::files::{
    PRIVATE_FILE_MODE, ensure_private_dir, remove_file_if_present, write_replacing,
};

/// The program that reads the configuration.
const SSH_PROGRAM: &str = "ssh";

/// The file tried, in [`MANAGED_DIR`], removed afterwards.
const CHECK_FILE: &str = "ssh_config.check";

/// Where `ssh` writes what it says while it tries the file, next to it.
const CHECK_ERRORS_FILE: &str = "ssh_config.check.err";

/// A name that no host of the configuration has, to try the common options.
const CHECK_HOST: &str = "hubergamof-check";

/// How long `ssh -G` may take: it only reads a few small files, but an
/// `Include`d file may hold a `Match exec`, which runs a program.
const SSH_TIMEOUT: Duration = Duration::from_secs(5);

/// How often the program is looked at while it runs.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Where programs are looked for when the manager has no `PATH`.
const DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// Longest reason kept of what `ssh` said, in characters.
const MAX_REASON_CHARS: usize = 200;

/// Most of what `ssh` says that is read.
const MAX_SSH_OUTPUT_BYTES: u64 = 64 * 1024;

/// What `ssh` said of a file.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Accepted,
    Refused(String),
    /// `ssh` is not installed or could not be run: nothing to break.
    Unavailable,
}

/// What a file is tried for: a host of the configuration, or the common options.
#[derive(Debug, Clone, Copy)]
enum Scope<'a> {
    Host(&'a str),
    Common,
}

impl Scope<'_> {
    /// The name `ssh -G` is asked about.
    fn target(self) -> String {
        match self {
            Self::Host(alias) => alias.to_owned(),
            Self::Common => CHECK_HOST.to_owned(),
        }
    }

    /// What the person is told the refusal is about.
    fn label(self) -> String {
        match self {
            Self::Host(alias) => alias.to_owned(),
            Self::Common => COMMON_PATTERN.to_owned(),
        }
    }
}

/// The program that tries the files, and how long it may take: both can be
/// replaced in tests, so that the cases where it is missing or too slow are
/// exercised.
#[derive(Debug, Clone)]
pub(super) struct SshProbe {
    program: OsString,
    timeout: Duration,
}

impl Default for SshProbe {
    fn default() -> Self {
        Self {
            program: SSH_PROGRAM.into(),
            timeout: SSH_TIMEOUT,
        }
    }
}

/// The reason in the first line `ssh` printed about the file tried (some
/// lines come before it, about the terminal or deprecated options), without
/// the name of the file nor the number of its line, which mean nothing to the
/// person.
fn reason(stderr: &str, file: &Path) -> String {
    let name = file.display().to_string();
    let line = stderr
        .lines()
        .find(|line| line.contains(&name))
        .or_else(|| stderr.lines().find(|line| !line.trim().is_empty()))
        .unwrap_or("ssh refused the configuration");
    // Some messages put a colon after the file name (`file: line 3: …`),
    // others do not (`file line 3: …`).
    let line = line.replace(&name, "");
    let line = line.trim_start_matches([':', ' ']);
    let line = match line
        .strip_prefix("line ")
        .and_then(|rest| rest.split_once(':'))
    {
        Some((_, rest)) => rest.trim_start(),
        None => line,
    };
    line.chars().take(MAX_REASON_CHARS).collect()
}

/// Waits for `child` up to `timeout`; at the limit, the whole process group is
/// killed (a `Match exec` of an included file starts programs of its own), and
/// `None` is given.
fn wait_for(child: &mut Child, timeout: Duration) -> io::Result<Option<ExitStatus>> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if started.elapsed() >= timeout {
            kill_group(child);
            child.wait()?;
            return Ok(None);
        }
        sleep(POLL_INTERVAL);
    }
}

fn kill_group(child: &Child) {
    let group = i32::try_from(child.id()).ok().and_then(Pid::from_raw);
    if let Some(group) = group {
        // The group may be gone already: nothing is left to kill then.
        let _ = kill_process_group(group, Signal::KILL);
    }
}

/// What the program wrote in `errors`, at most [`MAX_SSH_OUTPUT_BYTES`].
fn read_errors(errors: &Path) -> String {
    let mut text = String::new();
    if let Ok(file) = File::open(errors) {
        let mut bytes = Vec::new();
        let _ = file.take(MAX_SSH_OUTPUT_BYTES).read_to_end(&mut bytes);
        text = String::from_utf8_lossy(&bytes).into_owned();
    }
    text
}

impl SshProbe {
    /// What `ssh -G -F file host` says: arguments separated, no shell, an
    /// empty environment but `PATH`, nothing is connected to, in a process
    /// group of its own, and what it says goes to the file `errors` rather than
    /// a pipe that a program it started could keep open.
    fn ask(&self, file: &Path, errors: &Path, host: &str) -> Verdict {
        let path = std::env::var_os("PATH").unwrap_or_else(|| DEFAULT_PATH.into());
        let Ok(errors_out) = File::create(errors) else {
            return Verdict::Unavailable;
        };
        let spawned = Command::new(&self.program)
            .args(["-G", "-F"])
            .arg(file)
            .arg("--")
            .arg(host)
            .env_clear()
            .env("PATH", path)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(errors_out))
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => {
                tracing::warn!(%error, "ssh could not be run, its verdict on the options is missing");
                return Verdict::Unavailable;
            }
        };
        match wait_for(&mut child, self.timeout) {
            Ok(Some(status)) if status.success() => Verdict::Accepted,
            Ok(Some(_)) => Verdict::Refused(reason(&read_errors(errors), file)),
            Ok(None) => Verdict::Refused("ssh took too long to read the configuration".to_owned()),
            Err(error) => {
                tracing::warn!(%error, "ssh could not be waited for");
                Verdict::Unavailable
            }
        }
    }
}

impl SshFiles {
    /// Refuses `content` (what [`super::render::render`] made of `config`)
    /// when `ssh` cannot read it, before anything is written. Only the hosts
    /// that carry options of the person, and the common options, are tried:
    /// the other values are the ones the contract checks. Without `ssh`
    /// nothing is checked.
    ///
    /// # Errors
    /// [`SshFilesError::Rejected`] with what `ssh` said, and the file errors of
    /// writing the trial file.
    pub(super) fn check_with_ssh(
        &self,
        config: &SshConfig,
        content: &str,
    ) -> Result<(), SshFilesError> {
        let mut tried: Vec<Scope> = config
            .hosts
            .iter()
            .filter(|host| host.enabled && host.options.iter().any(|option| option.enabled))
            .map(|host| Scope::Host(&host.alias))
            .collect();
        if config.common_options.iter().any(|option| option.enabled) {
            tried.push(Scope::Common);
        }
        if tried.is_empty() {
            return Ok(());
        }
        let dir = self.home.join(MANAGED_DIR);
        ensure_private_dir(&dir)?;
        let file = dir.join(CHECK_FILE);
        let errors = dir.join(CHECK_ERRORS_FILE);
        write_replacing(&file, content.as_bytes(), PRIVATE_FILE_MODE)?;
        let refusal =
            tried.iter().find_map(
                |scope| match self.probe.ask(&file, &errors, &scope.target()) {
                    Verdict::Refused(message) => Some((scope.label(), message)),
                    Verdict::Accepted | Verdict::Unavailable => None,
                },
            );
        remove_file_if_present(&file)?;
        remove_file_if_present(&errors)?;
        match refusal {
            Some((scope, message)) => Err(SshFilesError::Rejected { scope, message }),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[test]
    fn the_reason_is_what_ssh_said_of_the_file_without_the_file_nor_its_line() {
        let file = Path::new("/h/.config/hubergamof/ssh_config.check");
        for (stderr, expected) in [
            (
                "/h/.config/hubergamof/ssh_config.check: line 3: Bad configuration option: foo\n",
                "Bad configuration option: foo",
            ),
            (
                "/h/.config/hubergamof/ssh_config.check line 3: invalid quotes\n",
                "invalid quotes",
            ),
            (
                "Pseudo-terminal will not be allocated\n\n/h/.config/hubergamof/ssh_config.check line 9: unsupported option \"maybe\".\n",
                "unsupported option \"maybe\".",
            ),
            ("\n\nsomething else\n", "something else"),
            ("", "ssh refused the configuration"),
        ] {
            let found = reason(stderr, file);
            assert_eq!(found, expected, "{stderr:?}");
            assert!(!found.contains("/h/"));
        }
    }

    #[test]
    fn a_long_reason_is_cut_to_its_limit_in_characters() {
        let file = Path::new("/x/check");
        let long = "é".repeat(MAX_REASON_CHARS + 50);
        assert_eq!(
            reason(&format!("/x/check: line 1: {long}"), file)
                .chars()
                .count(),
            MAX_REASON_CHARS
        );
    }

    /// A program that stands for `ssh`: a script run by the shell.
    fn fake(dir: &Path, script: &str) -> SshProbe {
        let path = dir.join("fake-ssh");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        SshProbe {
            program: path.into_os_string(),
            timeout: Duration::from_millis(300),
        }
    }

    /// A script just written may be refused by `exec` while another test thread forks
    /// (`ETXTBSY`): the program is asked again.
    fn ask(probe: &SshProbe, dir: &Path) -> Verdict {
        for _ in 0..20 {
            let verdict = probe.ask(&dir.join("file"), &dir.join("file.err"), "host");
            if verdict != Verdict::Unavailable || !Path::new(&probe.program).exists() {
                return verdict;
            }
            sleep(Duration::from_millis(25));
        }
        Verdict::Unavailable
    }

    #[test]
    fn a_program_that_is_missing_gives_no_verdict() {
        let dir = tempfile::tempdir().unwrap();
        let probe = SshProbe {
            program: dir.path().join("nowhere").into_os_string(),
            timeout: Duration::from_secs(1),
        };
        assert_eq!(ask(&probe, dir.path()), Verdict::Unavailable);
    }

    #[test]
    fn a_program_that_succeeds_accepts_and_one_that_fails_says_why() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            ask(&fake(dir.path(), "exit 0"), dir.path()),
            Verdict::Accepted
        );
        let refusing = fake(
            dir.path(),
            &format!(
                "echo '{}: line 2: bad thing' >&2; exit 255",
                dir.path().join("file").display()
            ),
        );
        assert_eq!(
            ask(&refusing, dir.path()),
            Verdict::Refused("bad thing".to_owned())
        );
    }

    #[test]
    fn a_program_that_takes_too_long_is_killed_with_what_it_started() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("grandchild-alive");
        // What a `Match exec` does: a child that outlives `ssh` and keeps its descriptors.
        let slow = fake(
            dir.path(),
            &format!("(sleep 1; touch '{}') &\nsleep 30", marker.display()),
        );
        let started = Instant::now();
        let verdict = ask(&slow, dir.path());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "given up in time"
        );
        assert_eq!(
            verdict,
            Verdict::Refused("ssh took too long to read the configuration".to_owned())
        );
        sleep(Duration::from_millis(1500));
        assert!(!marker.exists(), "the process group was killed");
    }

    #[test]
    fn an_orphan_holding_the_output_does_not_block_the_verdict() {
        let dir = tempfile::tempdir().unwrap();
        // `ssh` is gone at once while its child keeps writing where it wrote.
        let orphan = fake(dir.path(), "(sleep 3) &\nexit 0");
        let started = Instant::now();
        assert_eq!(ask(&orphan, dir.path()), Verdict::Accepted);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
