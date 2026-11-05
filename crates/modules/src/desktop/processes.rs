//! Which applications are running, from `/proc`: the executable of each
//! process of the user (`/proc/<pid>/exe`), and its arguments only when the
//! executable matches (to set the helpers aside).
//!
//! Synchronous file system reads: called through `spawn_blocking`.

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::model::ExecutableMatch;

/// Where the kernel lists the processes.
pub(super) const PROC_ROOT: &str = "/proc";

/// Added by the kernel to the executable of a process whose file was
/// replaced or removed since it started (an application updated while it
/// runs).
const DELETED_SUFFIX: &[u8] = b" (deleted)";

/// The executable of the process `pid`, if it can be read.
pub(super) fn executable_of(pid: u32) -> Option<PathBuf> {
    executable_in(&Path::new(PROC_ROOT).join(pid.to_string()))
}

/// The executable of the process listed at `process`, without the mark of
/// a replaced file.
fn executable_in(process: &Path) -> Option<PathBuf> {
    let link = fs::read_link(process.join("exe")).ok()?;
    let bytes = link.as_os_str().as_bytes();
    Some(match bytes.strip_suffix(DELETED_SUFFIX) {
        Some(live) => PathBuf::from(OsStr::from_bytes(live)),
        None => link,
    })
}

/// For each of `executables`, in the same order, whether a process of `uid`
/// listed under `proc_root` runs it (helpers aside).
///
/// Processes of other users are skipped (their executable cannot be read
/// anyway), as are those that end while being looked at.
///
/// # Errors
///
/// [`std::io::Error`] when `proc_root` cannot be listed.
pub(super) fn running(
    proc_root: &Path,
    uid: u32,
    executables: &[ExecutableMatch],
) -> std::io::Result<Vec<bool>> {
    let mut found = vec![false; executables.len()];
    for entry in fs::read_dir(proc_root)?.flatten() {
        let is_process = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.bytes().all(|byte| byte.is_ascii_digit()));
        if !is_process || entry.metadata().map(|meta| meta.uid()).ok() != Some(uid) {
            continue;
        }
        let process = entry.path();
        let Some(executable) = executable_in(&process) else {
            continue;
        };
        for (index, matcher) in executables.iter().enumerate() {
            if !found[index]
                && matcher.matches_executable(&executable)
                && !is_helper(&process, matcher)
            {
                found[index] = true;
            }
        }
        if found.iter().all(|&found| found) {
            break;
        }
    }
    Ok(found)
}

/// Whether `process` is a helper of the application `matcher` describes; a
/// process whose arguments cannot be read (ended meanwhile) counts as one.
fn is_helper(process: &Path, matcher: &ExecutableMatch) -> bool {
    if matcher.helper_argument.is_none() {
        return false;
    }
    fs::read(process.join("cmdline")).map_or(true, |arguments| {
        matcher.is_helper(arguments.split(|&byte| byte == 0))
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    /// A fake `/proc` with the given processes: pid, executable, arguments.
    fn fake_proc(processes: &[(u32, &str, &[&str])]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for (pid, executable, arguments) in processes {
            let process = root.path().join(pid.to_string());
            fs::create_dir(&process).unwrap();
            symlink(executable, process.join("exe")).unwrap();
            fs::write(process.join("cmdline"), arguments.join("\0")).unwrap();
        }
        fs::create_dir(root.path().join("self-not-a-pid")).unwrap();
        root
    }

    fn claude() -> ExecutableMatch {
        ExecutableMatch {
            path: "/usr/lib/claude-desktop/claude-desktop".into(),
            helper_argument: Some("--type=".into()),
        }
    }

    fn idea() -> ExecutableMatch {
        ExecutableMatch {
            path: "bin/idea".into(),
            helper_argument: None,
        }
    }

    fn own_uid() -> u32 {
        rustix::process::getuid().as_raw()
    }

    #[test]
    fn applications_are_found_by_their_executable() {
        let proc = fake_proc(&[
            (10, "/usr/bin/bash", &["bash"]),
            (11, "/opt/idea/bin/idea", &["idea"]),
            (
                12,
                "/usr/lib/claude-desktop/claude-desktop",
                &["claude", "--startup"],
            ),
        ]);
        let found = running(proc.path(), own_uid(), &[claude(), idea()]).unwrap();
        assert_eq!(found, [true, true]);
    }

    #[test]
    fn helpers_and_borrowed_runtimes_do_not_count() {
        let proc = fake_proc(&[
            (
                20,
                "/usr/lib/claude-desktop/claude-desktop",
                &["claude", "--type=zygote"],
            ),
            (
                21,
                "/opt/idea/jbr/bin/java",
                &["java", "-cp", "/opt/idea/lib/idea.jar"],
            ),
        ]);
        let found = running(proc.path(), own_uid(), &[claude(), idea()]).unwrap();
        assert_eq!(found, [false, false]);
    }

    #[test]
    fn an_application_updated_while_running_still_counts() {
        let proc = fake_proc(&[(40, "/opt/idea/bin/idea (deleted)", &["idea"])]);
        let found = running(proc.path(), own_uid(), &[idea()]).unwrap();
        assert_eq!(found, [true]);
        assert_eq!(
            executable_in(&proc.path().join("40")),
            Some(PathBuf::from("/opt/idea/bin/idea"))
        );
    }

    #[test]
    fn the_executable_of_this_process_is_found() {
        let own = std::process::id();
        assert_eq!(executable_of(own), std::env::current_exe().ok());
        assert_eq!(executable_of(u32::MAX), None);
    }

    #[test]
    fn processes_of_other_users_are_skipped() {
        let proc = fake_proc(&[(30, "/opt/idea/bin/idea", &["idea"])]);
        let found = running(proc.path(), own_uid() + 1, &[idea()]).unwrap();
        assert_eq!(found, [false]);
    }

    #[test]
    fn a_process_ending_while_looked_at_counts_as_a_helper() {
        let process = tempfile::tempdir().unwrap();
        assert!(is_helper(process.path(), &claude()));
        assert!(!is_helper(process.path(), &idea()));
    }

    #[test]
    fn an_unreadable_proc_is_an_error() {
        let missing = PathBuf::from("/nonexistent/hubergamof-proc");
        assert!(running(&missing, own_uid(), &[idea()]).is_err());
    }

    #[test]
    fn the_real_proc_is_readable() {
        let found = running(Path::new(PROC_ROOT), own_uid(), &[idea()]).unwrap();
        assert_eq!(found.len(), 1);
    }
}
