//! What Git finds in the configuration of the computer besides what
//! HuBergamof wrote: the values it manages, read with `git config --list`.
//!
//! Git runs with an empty environment but the few variables it needs, from a
//! folder that is not a repository, so that only the files of the user (and
//! the system's) are read, never a repository's, and nothing in the
//! environment of the manager changes what is read.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use hubergamof_protocol::GitSystemEntry;
use tokio::process::Command;
use tokio::time::timeout;

use super::apply::GitFiles;
use super::render::MANAGED_DIR;

/// Most entries listed.
pub const MAX_SYSTEM_GIT_ENTRIES: usize = 256;

/// Longest value kept of an entry, in characters.
const MAX_ENTRY_VALUE_CHARS: usize = 1024;

/// How long `git config` may take: it only reads a few small files.
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Where programs are looked for when the manager has no `PATH`.
const DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// What the computer holds, as far as Git could tell.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SystemRead {
    /// The managed values found in files other than HuBergamof's.
    pub entries: Vec<GitSystemEntry>,
    /// Whether the `git` program ran.
    pub git_available: bool,
}

/// Whether the configuration key `key` (lower case) is one HuBergamof manages.
fn is_managed_key(key: &str) -> bool {
    matches!(
        key,
        "user.name"
            | "user.email"
            | "user.signingkey"
            | "commit.gpgsign"
            | "gpg.format"
            | "init.defaultbranch"
            | "pull.rebase"
            | "pull.ff"
            | "core.editor"
            | "credential.helper"
    ) || key.starts_with("alias.")
}

/// The entries of the output of `git config --list --show-origin --null`:
/// `file:<path> NUL <key> LF <value> NUL`, and for a key without a value
/// (`true`) `file:<path> NUL <key> NUL`. Keeps the managed keys that do not
/// come from HuBergamof's files, with the home folder written `~`.
fn parse_entries(output: &[u8], home: &Path) -> Vec<GitSystemEntry> {
    let managed_dir = home.join(MANAGED_DIR);
    let mut tokens = output.split(|byte| *byte == 0);
    let mut entries = Vec::new();
    while let (Some(origin), Some(entry)) = (tokens.next(), tokens.next()) {
        let origin = String::from_utf8_lossy(origin);
        let Some(path) = origin.strip_prefix("file:") else {
            continue;
        };
        let entry = String::from_utf8_lossy(entry);
        let (key, value) = entry.split_once('\n').unwrap_or((entry.as_ref(), "true"));
        if !is_managed_key(key) || Path::new(path).starts_with(&managed_dir) {
            continue;
        }
        entries.push(GitSystemEntry {
            key: key.to_owned(),
            value: value.chars().take(MAX_ENTRY_VALUE_CHARS).collect(),
            origin: tilde(path, home),
        });
        if entries.len() == MAX_SYSTEM_GIT_ENTRIES {
            break;
        }
    }
    entries
}

/// `path` with the home folder written `~`.
fn tilde(path: &str, home: &Path) -> String {
    match Path::new(path).strip_prefix(home) {
        Ok(relative) => format!("~/{}", relative.display()),
        Err(_) => path.to_owned(),
    }
}

impl GitFiles {
    /// Reads the managed values the computer holds besides HuBergamof's. A
    /// missing `git`, a failure or a timeout leave the list empty (logged).
    pub async fn read_system(&self) -> SystemRead {
        let path = std::env::var_os("PATH").unwrap_or_else(|| OsString::from(DEFAULT_PATH));
        let mut command = Command::new("git");
        command
            .args(["config", "--list", "--show-origin", "--null"])
            .env_clear()
            .env("PATH", path)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("LC_ALL", "C")
            .current_dir("/")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if !self.with_system_config {
            command.env("GIT_CONFIG_NOSYSTEM", "1");
        }
        match timeout(GIT_TIMEOUT, command.output()).await {
            Ok(Ok(output)) if output.status.success() => SystemRead {
                entries: parse_entries(&output.stdout, &self.home),
                git_available: true,
            },
            Ok(Ok(output)) => {
                tracing::warn!(status = %output.status, "git could not list its configuration");
                SystemRead {
                    entries: Vec::new(),
                    git_available: true,
                }
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "git could not be run");
                SystemRead::default()
            }
            Err(_) => {
                tracing::warn!("git took too long to list its configuration");
                SystemRead::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const HOME: &str = "/home/ada";

    fn output(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for (origin, entry) in entries {
            bytes.extend_from_slice(origin.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(entry.as_bytes());
            bytes.push(0);
        }
        bytes
    }

    fn parse(entries: &[(&str, &str)]) -> Vec<GitSystemEntry> {
        parse_entries(&output(entries), Path::new(HOME))
    }

    #[test]
    fn managed_keys_are_listed_with_their_origin() {
        let found = parse(&[
            ("file:/home/ada/.gitconfig", "user.name\nAda"),
            ("file:/etc/gitconfig", "credential.helper\nstore"),
            ("file:/home/ada/.gitconfig", "alias.co\ncheckout"),
        ]);
        let listed: Vec<_> = found
            .iter()
            .map(|e| (e.key.as_str(), e.value.as_str(), e.origin.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("user.name", "Ada", "~/.gitconfig"),
                ("credential.helper", "store", "/etc/gitconfig"),
                ("alias.co", "checkout", "~/.gitconfig"),
            ]
        );
    }

    #[test]
    fn other_keys_and_other_origins_are_left_out() {
        let found = parse(&[
            ("file:/home/ada/.gitconfig", "core.pager\nless"),
            (
                "file:/home/ada/.gitconfig",
                "filter.lfs.clean\ngit-lfs clean",
            ),
            ("command line:", "user.name\nCmd"),
            ("standard input:", "user.name\nStdin"),
            ("blob:abcd", "user.name\nBlob"),
        ]);
        assert_eq!(found, []);
    }

    #[test]
    fn what_hubergamof_wrote_is_not_listed_as_found_elsewhere() {
        let found = parse(&[
            (
                "file:/home/ada/.config/hubergamof/gitconfig",
                "user.name\nAda",
            ),
            (
                "file:/home/ada/.config/hubergamof/gitconfig-folder-1",
                "user.email\nada@work.org",
            ),
        ]);
        assert_eq!(found, []);
    }

    #[test]
    fn a_key_without_value_is_true_and_values_keep_their_newlines_free_text() {
        let found = parse(&[("file:/home/ada/.gitconfig", "commit.gpgsign")]);
        assert_eq!(found[0].value, "true");
        let found = parse(&[("file:/home/ada/.gitconfig", "user.name\nA\nB")]);
        assert_eq!(found[0].value, "A\nB");
    }

    #[test]
    fn the_list_and_its_values_are_bounded() {
        let many: Vec<(&str, String)> = (0..MAX_SYSTEM_GIT_ENTRIES + 10)
            .map(|i| ("file:/home/ada/.gitconfig", format!("alias.a{i}\nstatus")))
            .collect();
        let borrowed: Vec<(&str, &str)> = many.iter().map(|(o, e)| (*o, e.as_str())).collect();
        assert_eq!(parse(&borrowed).len(), MAX_SYSTEM_GIT_ENTRIES);

        let long = format!("user.name\n{}", "x".repeat(MAX_ENTRY_VALUE_CHARS + 50));
        let found = parse(&[("file:/home/ada/.gitconfig", &long)]);
        assert_eq!(found[0].value.chars().count(), MAX_ENTRY_VALUE_CHARS);
    }

    #[test]
    fn a_truncated_output_is_read_as_far_as_it_goes() {
        let mut bytes = output(&[("file:/home/ada/.gitconfig", "user.name\nAda")]);
        bytes.extend_from_slice(b"file:/home/ada/.gitconfig");
        assert_eq!(parse_entries(&bytes, Path::new(HOME)).len(), 1);
    }

    #[tokio::test]
    async fn git_lists_what_the_user_wrote_and_ignores_the_environment() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join(".gitconfig"),
            "[user]\n\tname = Ada\n[alias]\n\tco = checkout\n[core]\n\tpager = less\n",
        )
        .unwrap();
        let files = GitFiles::new(home.path().to_owned(), false);
        let read = files.read_system().await;
        assert!(read.git_available);
        let keys: Vec<_> = read.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["user.name", "alias.co"]);
        assert_eq!(read.entries[0].origin, "~/.gitconfig");
    }

    #[tokio::test]
    async fn a_home_without_git_files_lists_nothing() {
        let home = tempfile::tempdir().unwrap();
        let files = GitFiles::new(PathBuf::from(home.path()), false);
        let read = files.read_system().await;
        assert!(read.git_available);
        assert_eq!(read.entries, []);
    }

    #[tokio::test]
    async fn a_git_configuration_git_cannot_read_leaves_the_list_empty() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join(".gitconfig"), "[user\nname = \n").unwrap();
        let files = GitFiles::new(home.path().to_owned(), false);
        let read = files.read_system().await;
        assert!(read.git_available);
        assert_eq!(read.entries, []);
    }
}
