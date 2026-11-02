//! Runs an external program with a time limit and bounded outputs, so that
//! a stuck or hostile tool can neither stall a module nor make the manager
//! hold unbounded memory.
//!
//! Programs are always run directly (never through a shell), each argument
//! passed separately.

use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};

/// Put in front of an error output whose beginning was cut.
pub(crate) const TRUNCATION_MARK: &str = "…";

/// How much of the outputs of a program is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutputLimits {
    /// Largest standard output, past which the run fails; `None`: the
    /// output is not read (sent to `/dev/null`).
    pub(crate) stdout: Option<usize>,
    /// Error output kept: its last bytes, where tools report what failed
    /// after their progress; the rest is read and dropped.
    pub(crate) stderr: usize,
}

/// A program that ran to its end.
#[derive(Debug)]
pub(crate) struct Finished {
    pub(crate) status: ExitStatus,
    /// Empty when not read.
    pub(crate) stdout: Vec<u8>,
    /// End of the error output ([`OutputLimits::stderr`]), trimmed.
    pub(crate) stderr: String,
}

/// Why a program gave no usable result.
#[derive(Debug)]
pub(crate) enum RunError {
    /// It could not be started.
    Spawn(std::io::Error),
    /// It did not end in time; it was killed.
    Timeout,
    /// Its output is unusable (too long, unreadable).
    Output(String),
}

/// Runs `command` to its end, within `timeout`, stdin closed; the process
/// is killed on timeout or when the future is dropped.
pub(crate) async fn run(
    mut command: Command,
    timeout: Duration,
    limits: OutputLimits,
) -> Result<Finished, RunError> {
    let stdout = if limits.stdout.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(RunError::Spawn)?;
    match tokio::time::timeout(timeout, collect(&mut child, limits)).await {
        Err(_) => Err(RunError::Timeout),
        Ok(collected) => collected.map_err(RunError::Output),
    }
}

/// Waits for `child` while reading its outputs within their bounds.
async fn collect(child: &mut Child, limits: OutputLimits) -> Result<Finished, String> {
    let Some(stderr) = child.stderr.take() else {
        return Err("error output not captured".into());
    };
    let stdout = child.stdout.take();
    let read_stdout = async {
        match (stdout, limits.stdout) {
            (Some(stdout), Some(limit)) => read_bounded(stdout, limit).await,
            (None, None) => Ok(Vec::new()),
            _ => Err("standard output not captured".into()),
        }
    };
    let (stdout, stderr, status) =
        tokio::try_join!(read_stdout, read_tail(stderr, limits.stderr), async {
            child.wait().await.map_err(|error| error.to_string())
        },)?;
    Ok(Finished {
        status,
        stdout,
        stderr,
    })
}

/// Everything `reader` produces, or an error past `limit` bytes.
async fn read_bounded(reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut bounded = reader.take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1));
    bounded
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| error.to_string())?;
    if bytes.len() > limit {
        return Err(format!("more than {limit} bytes"));
    }
    Ok(bytes)
}

/// Room to read a chunk of an error output.
const CHUNK_BYTES: usize = 8 * 1024;

/// The last `limit` bytes of `reader` as text, everything read (so that the
/// process never blocks on a full pipe) but at most `limit` + one chunk
/// held.
async fn read_tail(mut reader: impl AsyncRead + Unpin, limit: usize) -> Result<String, String> {
    let mut kept = Vec::new();
    let mut chunk = vec![0; CHUNK_BYTES];
    let mut cut = false;
    loop {
        let read = reader
            .read(&mut chunk)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        kept.extend_from_slice(&chunk[..read]);
        if kept.len() > limit {
            kept.drain(..kept.len() - limit);
            cut = true;
        }
    }
    let text = String::from_utf8_lossy(&kept).trim().to_owned();
    Ok(if cut {
        format!("{TRUNCATION_MARK}{text}")
    } else {
        text
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: OutputLimits = OutputLimits {
        stdout: Some(16),
        stderr: 8,
    };
    const PATIENCE: Duration = Duration::from_secs(10);

    fn sh(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    #[tokio::test]
    async fn outputs_and_status_are_collected() {
        let finished = run(
            sh("printf out; printf ' err ' >&2; exit 3"),
            PATIENCE,
            LIMITS,
        )
        .await
        .unwrap();
        assert_eq!(finished.status.code(), Some(3));
        assert_eq!(finished.stdout, b"out");
        assert_eq!(finished.stderr, "err");
    }

    #[tokio::test]
    async fn a_long_output_fails_and_a_long_error_keeps_its_end() {
        let flood = run(sh("head -c 17 /dev/zero"), PATIENCE, LIMITS).await;
        assert!(matches!(flood, Err(RunError::Output(_))), "{flood:?}");
        let finished = run(sh("printf 0123456789 >&2"), PATIENCE, LIMITS)
            .await
            .unwrap();
        assert_eq!(finished.stderr, format!("{TRUNCATION_MARK}23456789"));
        let script = "head -c 100000 /dev/zero; printf failed >&2; \
                      head -c 50000 /dev/zero >&2; printf ' the end' >&2";
        let limits = OutputLimits {
            stdout: None,
            stderr: 7,
        };
        let long = run(sh(script), PATIENCE, limits).await.unwrap();
        assert_eq!(long.stderr, format!("{TRUNCATION_MARK}the end"));
    }

    #[tokio::test]
    async fn an_unread_output_may_be_long() {
        let limits = OutputLimits {
            stdout: None,
            ..LIMITS
        };
        let finished = run(sh("head -c 100000 /dev/zero"), PATIENCE, limits)
            .await
            .unwrap();
        assert!(finished.status.success());
        assert_eq!(finished.stdout, b"");
    }

    #[tokio::test]
    async fn a_stuck_program_times_out_and_a_missing_one_fails_to_start() {
        let stuck = run(sh("sleep 30"), Duration::from_millis(50), LIMITS).await;
        assert!(matches!(stuck, Err(RunError::Timeout)), "{stuck:?}");
        let missing = run(Command::new("/nonexistent/program"), PATIENCE, LIMITS).await;
        assert!(matches!(missing, Err(RunError::Spawn(_))), "{missing:?}");
    }
}
