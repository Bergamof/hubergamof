//! Asking `ssh` itself whether it can read what is about to be written.
//!
//! The options written by the person are free: a typo or an option this
//! version of `ssh` does not know makes `ssh` refuse the whole configuration,
//! for every host. The file is therefore tried before it replaces the one
//! `ssh` reads: `ssh -G` prints the configuration it would use without
//! connecting, and fails on what it cannot read.

use std::io::{self, Read as _};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use hubergamof_protocol::SshConfig;

use super::apply::{SshFiles, SshFilesError};
use super::render::{COMMON_PATTERN, MANAGED_DIR};
use crate::machine::files::{
    PRIVATE_FILE_MODE, ensure_private_dir, remove_file_if_present, write_replacing,
};

/// The program that reads the configuration.
const SSH_PROGRAM: &str = "ssh";

/// The file tried, in [`MANAGED_DIR`], removed afterwards.
const CHECK_FILE: &str = "ssh_config.check";

/// A name that no host of the configuration has, to try the common options.
const CHECK_HOST: &str = "hubergamof-check";

/// How long `ssh -G` may take: it only reads a few small files.
const SSH_TIMEOUT: Duration = Duration::from_secs(5);

/// How often the program is looked at while it runs.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Where programs are looked for when the manager has no `PATH`.
const DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// Longest reason kept of what `ssh` said, in characters.
const MAX_REASON_CHARS: usize = 200;

/// What `ssh` said of a file.
enum Verdict {
    Accepted,
    Refused(String),
    /// `ssh` is not installed or could not be run: nothing to break.
    Unavailable,
}

/// The reason in the first line `ssh` printed, without the name of the file
/// tried nor the number of its line, which mean nothing to the person.
fn reason(stderr: &str, file: &Path) -> String {
    let line = stderr.lines().find(|line| !line.trim().is_empty());
    let line = line.unwrap_or("ssh refused the configuration");
    // Some messages put a colon after the file name (`file: line 3: …`),
    // others do not (`file line 3: …`).
    let line = line.replace(&file.display().to_string(), "");
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

fn wait_for(child: &mut std::process::Child) -> io::Result<Option<std::process::ExitStatus>> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if started.elapsed() >= SSH_TIMEOUT {
            child.kill()?;
            child.wait()?;
            return Ok(None);
        }
        sleep(POLL_INTERVAL);
    }
}

/// What `ssh -G -F file host` says: arguments separated, no shell, an empty
/// environment but `PATH`, and nothing is connected to.
fn ask(file: &Path, host: &str) -> Verdict {
    let path = std::env::var_os("PATH").unwrap_or_else(|| DEFAULT_PATH.into());
    let spawned = Command::new(SSH_PROGRAM)
        .args(["-G", "-F"])
        .arg(file)
        .arg("--")
        .arg(host)
        .env_clear()
        .env("PATH", path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!(%error, "ssh could not be run, its verdict on the options is missing");
            return Verdict::Unavailable;
        }
    };
    let status = wait_for(&mut child);
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    match status {
        Ok(Some(status)) if status.success() => Verdict::Accepted,
        Ok(Some(_)) => Verdict::Refused(reason(&stderr, file)),
        Ok(None) => Verdict::Refused("ssh took too long to read the configuration".to_owned()),
        Err(error) => {
            tracing::warn!(%error, "ssh could not be waited for");
            Verdict::Unavailable
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
        let mut tried: Vec<&str> = config
            .hosts
            .iter()
            .filter(|host| host.enabled && host.options.iter().any(|option| option.enabled))
            .map(|host| host.alias.as_str())
            .collect();
        if config.common_options.iter().any(|option| option.enabled) {
            tried.push(CHECK_HOST);
        }
        if tried.is_empty() {
            return Ok(());
        }
        let dir = self.home.join(MANAGED_DIR);
        ensure_private_dir(&dir)?;
        let file = dir.join(CHECK_FILE);
        write_replacing(&file, content.as_bytes(), PRIVATE_FILE_MODE)?;
        let verdict = tried.iter().find_map(|host| match ask(&file, host) {
            Verdict::Accepted => None,
            Verdict::Refused(message) => Some(Some((*host, message))),
            Verdict::Unavailable => Some(None),
        });
        remove_file_if_present(&file)?;
        match verdict {
            Some(Some((host, message))) => Err(SshFilesError::Rejected {
                scope: if host == CHECK_HOST {
                    COMMON_PATTERN
                } else {
                    host
                }
                .to_owned(),
                message,
            }),
            _ => Ok(()),
        }
    }
}
