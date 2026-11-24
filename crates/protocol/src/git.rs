//! The Git configuration of the user, persisted as `config/git.json`: who
//! they commit as, how Git behaves, their aliases and the identities that
//! depend on the folder a repository is in.
//!
//! Git runs some of what it reads from its configuration (an editor, a
//! credential helper, an alias starting with `!`), so those values are not
//! free text: the helper and the editor come from closed lists, and an alias
//! cannot start a shell command directly. That is not a barrier: the options
//! of a Git command can run a program (`fetch --upload-pack=…`), so an alias
//! is as trusted as a shell alias (`docs/security.md`).

use std::collections::HashSet;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::advanced::{AdvancedSetting, check_settings, is_one_line_value};
use crate::text::{is_blank, is_invisible};

/// Git configuration schema version produced and understood by this crate.
pub const GIT_SCHEMA_VERSION: u32 = 1;

/// Most aliases the configuration holds.
pub const MAX_GIT_ALIASES: usize = 128;

/// Most folder identities the configuration holds.
pub const MAX_GIT_FOLDER_IDENTITIES: usize = 32;

/// Longest name, e-mail address, key, folder, branch or editor, in characters.
pub const MAX_GIT_VALUE_CHARS: usize = 256;

/// Longest alias name, in characters.
pub const MAX_GIT_ALIAS_NAME_CHARS: usize = 64;

/// Longest alias command, in characters.
pub const MAX_GIT_ALIAS_COMMAND_CHARS: usize = 1024;

/// How long, in seconds, the `cache` credential helper may keep a password.
pub const CREDENTIAL_CACHE_SECONDS: RangeInclusive<u32> = 30..=604_800;

/// The timeout `cache` gets when it is chosen.
pub const DEFAULT_CREDENTIAL_CACHE_SECONDS: u32 = 900;

/// Editors Git may be told to use, as the command it runs. Any other editor
/// is an absolute path to its program, without argument.
pub const GIT_EDITORS: [&str; 12] = [
    "vi",
    "vim",
    "nvim",
    "nano",
    "emacs",
    "micro",
    "hx",
    "code --wait",
    "codium --wait",
    "subl --wait",
    "kate --block",
    "gedit --wait",
];

/// Editors that can be given by the absolute path of their program: a program
/// that is not one of them (a shell, an interpreter) would run the text it is
/// given to edit, and a commit message comes from anyone.
pub const GIT_PLAIN_EDITORS: [&str; 7] = ["vi", "vim", "nvim", "nano", "emacs", "micro", "hx"];

/// Who the commits are made as.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitIdentity {
    /// `user.name`; empty leaves it as it is.
    pub name: String,
    /// `user.email`; empty leaves it as it is.
    pub email: String,
}

/// The format of the signatures of commits (`gpg.format`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum GitSigningFormat {
    /// An `OpenPGP` key, signed by `GnuPG`.
    #[default]
    OpenPgp,
    /// An SSH key, signed by `ssh-keygen`.
    Ssh,
    /// An X.509 certificate.
    X509,
}

/// The signature of the commits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitSigning {
    /// Whether the commits are signed (`commit.gpgsign`). When it is not,
    /// nothing of the signature is written.
    pub enabled: bool,
    /// `gpg.format`.
    pub format: GitSigningFormat,
    /// `user.signingkey`: a key identifier, or for SSH the path of a public
    /// key or `key::` and the key. Empty lets Git choose.
    pub key: String,
}

/// What `git pull` does when the branches diverged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum GitPullMode {
    /// Leaves Git's own choice.
    #[default]
    Default,
    /// Merges (`pull.rebase = false`).
    Merge,
    /// Rebases (`pull.rebase = true`).
    Rebase,
    /// Only fast-forwards (`pull.ff = only`).
    FastForwardOnly,
}

/// Where Git keeps the passwords it was given (`credential.helper`): a closed
/// list, since Git runs the helper.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum GitCredentialHelper {
    /// Leaves it as it is.
    #[default]
    None,
    /// In memory for a while (`cache`).
    Cache {
        /// How long, in seconds, within [`CREDENTIAL_CACHE_SECONDS`].
        timeout_seconds: u32,
    },
    /// In clear text in `~/.git-credentials` (`store`).
    Store,
    /// In the keyring of the desktop (`libsecret`).
    Libsecret,
    /// In Git Credential Manager (`manager`).
    Manager,
}

/// How Git behaves.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitSettings {
    /// `init.defaultBranch`; empty leaves it as it is.
    pub default_branch: String,
    /// What `git pull` does.
    pub pull: GitPullMode,
    /// `core.editor`: one of [`GIT_EDITORS`] or the absolute path of the
    /// program of one of [`GIT_PLAIN_EDITORS`]; empty leaves it as it is.
    pub editor: String,
    /// `credential.helper`.
    pub credential_helper: GitCredentialHelper,
}

/// A Git alias. It stands for a Git command: one starting with `!` would run a
/// shell command and is refused, and so is one starting with `-`. The options
/// of the command itself are not checked (`docs/security.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitAlias {
    /// What is typed after `git`: a letter, then letters, digits or `-`.
    pub name: String,
    /// The Git command it stands for, on one line.
    pub command: String,
    /// Whether the alias is on the computer. A disabled one stays in the
    /// configuration and is removed from the computer.
    pub enabled: bool,
}

/// An identity for the repositories under a folder (`includeIf "gitdir:"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitFolderIdentity {
    /// The folder, under the home folder: `~/` and its path, without a
    /// trailing `/`.
    pub folder: String,
    /// `user.name` there; empty keeps the global one.
    pub name: String,
    /// `user.email` there; empty keeps the global one.
    pub email: String,
    /// `user.signingkey` there; empty keeps the global one.
    pub signing_key: String,
    /// Whether the rule is on the computer.
    pub enabled: bool,
}

/// A value of the Git configuration that HuBergamof also manages, found
/// elsewhere (`~/.gitconfig`, `/etc/gitconfig`…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitSystemEntry {
    /// The key, in lower case (`user.name`, `alias.co`).
    pub key: String,
    /// The value, as Git reads it.
    pub value: String,
    /// The file it comes from, the home folder written `~`.
    pub origin: String,
}

/// The Git configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitConfig {
    /// Schema version, see [`GIT_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The global identity.
    pub identity: GitIdentity,
    /// The signature of the commits.
    pub signing: GitSigning,
    /// How Git behaves.
    pub settings: GitSettings,
    /// The aliases, in the order they are shown and written.
    pub aliases: Vec<GitAlias>,
    /// The identities by folder, in the order they are shown and written.
    pub folder_identities: Vec<GitFolderIdentity>,
    /// The other keys of Git, written by the person (`push.default`).
    #[serde(default)]
    pub extra_settings: Vec<AdvancedSetting>,
    /// Revision counter, incremented on every accepted update; an update
    /// carrying an outdated revision is rejected.
    pub revision: u64,
}

/// Why a Git configuration is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GitConfigError {
    /// The schema version is not supported by this version of HuBergamof.
    #[error("unsupported git schema version {found} (supported: {GIT_SCHEMA_VERSION})")]
    UnsupportedSchemaVersion {
        /// Version found in the configuration.
        found: u32,
    },
    /// A value is not valid; `field` says which (`identity.email`…).
    #[error("invalid git value for {field}")]
    InvalidValue {
        /// The field.
        field: &'static str,
    },
    /// There are more aliases than [`MAX_GIT_ALIASES`].
    #[error("too many git aliases: {count} (max {MAX_GIT_ALIASES})")]
    TooManyAliases {
        /// How many.
        count: usize,
    },
    /// An alias name is not valid, see [`is_valid_git_alias_name`].
    #[error("invalid git alias name {0:?}")]
    InvalidAliasName(String),
    /// An alias command is not valid, see [`is_valid_git_alias_command`].
    #[error("invalid command for git alias `{0}`")]
    InvalidAliasCommand(String),
    /// Two aliases share a name (Git ignores the case of alias names).
    #[error("duplicate git alias `{0}`")]
    DuplicateAlias(String),
    /// There are more folder identities than [`MAX_GIT_FOLDER_IDENTITIES`].
    #[error("too many folder identities: {count} (max {MAX_GIT_FOLDER_IDENTITIES})")]
    TooManyFolderIdentities {
        /// How many.
        count: usize,
    },
    /// A folder is not valid, see [`is_valid_git_folder`].
    #[error("invalid folder {0:?}")]
    InvalidFolder(String),
    /// A folder identity sets nothing.
    #[error("the identity of folder {0:?} sets nothing")]
    EmptyFolderIdentity(String),
    /// Two identities share a folder.
    #[error("duplicate folder {0:?}")]
    DuplicateFolder(String),
    /// A setting written by the person is not valid, or written twice, or the
    /// list is too long (the key is then empty).
    #[error("invalid git setting {0:?}")]
    InvalidSetting(String),
}

fn is_plain_text(text: &str) -> bool {
    text.chars().count() <= MAX_GIT_VALUE_CHARS
        && !text.chars().any(|c| c.is_control() || is_invisible(c))
}

/// Whether `name` can be `user.name`: on one line, and without the `<` and
/// `>` that delimit an e-mail address in a commit.
#[must_use]
pub fn is_valid_git_user_name(name: &str) -> bool {
    is_plain_text(name) && !name.contains(['<', '>']) && !name.starts_with(char::is_whitespace)
}

/// Whether `email` can be `user.email`: one `@` between two non-empty parts,
/// no white space nor `<`, `>`.
#[must_use]
pub fn is_valid_git_user_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    is_plain_text(email)
        && !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && !email.contains(['<', '>'])
        && !email.chars().any(char::is_whitespace)
}

/// Whether `key` can be `user.signingkey`: on one line, not an option for the
/// program that signs.
#[must_use]
pub fn is_valid_git_signing_key(key: &str) -> bool {
    is_plain_text(key) && !key.starts_with('-') && !key.starts_with(char::is_whitespace)
}

/// Whether `branch` can be `init.defaultBranch`: letters, digits, `.`, `_`,
/// `-` and `/`, as a Git reference name made of them.
#[must_use]
pub fn is_valid_git_branch_name(branch: &str) -> bool {
    !branch.is_empty()
        && branch.chars().count() <= MAX_GIT_VALUE_CHARS
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
        && !branch.starts_with(['-', '/', '.'])
        && !branch.ends_with(['/', '.'])
        && !branch.contains("..")
        && !branch.contains("//")
        && !branch.contains("/.")
        && !branch.to_ascii_lowercase().ends_with(".lock")
        && !branch.to_ascii_lowercase().contains(".lock/")
}

/// Whether `editor` can be `core.editor`: one of [`GIT_EDITORS`], or the
/// absolute path of the program of one of [`GIT_PLAIN_EDITORS`] (no space, no
/// shell character: Git runs the value through a shell).
#[must_use]
pub fn is_valid_git_editor(editor: &str) -> bool {
    if GIT_EDITORS.contains(&editor) {
        return true;
    }
    editor.chars().count() <= MAX_GIT_VALUE_CHARS
        && editor.starts_with('/')
        && editor.len() > 1
        && editor
            .rsplit('/')
            .next()
            .is_some_and(|program| GIT_PLAIN_EDITORS.contains(&program))
        && editor
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | '/'))
        && editor
            .split('/')
            .skip(1)
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// The keys of Git that the forms manage (lowercase): the person does not write
/// them twice. The aliases (`alias.*`) and the includes are managed too.
pub const GIT_MANAGED_KEYS: [&str; 10] = [
    "user.name",
    "user.email",
    "user.signingkey",
    "commit.gpgsign",
    "gpg.format",
    "init.defaultbranch",
    "pull.rebase",
    "pull.ff",
    "core.editor",
    "credential.helper",
];

/// Longest sub-section of a key, in characters.
const MAX_GIT_SUBSECTION_CHARS: usize = 128;

fn is_git_word(word: &str) -> bool {
    word.chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && word.chars().count() <= crate::advanced::MAX_SETTING_KEY_CHARS
        && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The parts of `key` (`section`, `subsection`, `name`): a key is
/// `section.name` or `section.subsection.name`, the sub-section being whatever
/// is between the first and the last `.`.
#[must_use]
pub fn git_key_parts(key: &str) -> Option<(&str, Option<&str>, &str)> {
    let (section, rest) = key.split_once('.')?;
    match rest.rsplit_once('.') {
        Some((subsection, name)) => Some((section, Some(subsection), name)),
        None => Some((section, None, rest)),
    }
}

/// Whether `key` can be the key of a setting written by the person: a Git key
/// made of closed characters, which none of the forms manages.
#[must_use]
pub fn is_valid_git_setting_key(key: &str) -> bool {
    let Some((section, subsection, name)) = git_key_parts(key) else {
        return false;
    };
    let subsection_ok = subsection.is_none_or(|sub| {
        !sub.is_empty()
            && sub.chars().count() <= MAX_GIT_SUBSECTION_CHARS
            && sub.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(c, '.' | '_' | '/' | ':' | '@' | '*' | '~' | '+' | '-')
            })
    });
    let lower = key.to_ascii_lowercase();
    let section = section.to_ascii_lowercase();
    is_git_word(&section)
        && is_git_word(name)
        && subsection_ok
        && !GIT_MANAGED_KEYS.contains(&fold_git_key(key).as_str())
        && !matches!(section.as_str(), "alias" | "include" | "includeif")
        && !lower.is_empty()
}

/// A key as Git compares it: the section and the name without regard to case,
/// the sub-section as it is.
#[must_use]
pub fn fold_git_key(key: &str) -> String {
    match git_key_parts(key) {
        Some((section, Some(sub), name)) => {
            format!(
                "{}.{sub}.{}",
                section.to_ascii_lowercase(),
                name.to_ascii_lowercase()
            )
        }
        Some((section, None, name)) => {
            format!(
                "{}.{}",
                section.to_ascii_lowercase(),
                name.to_ascii_lowercase()
            )
        }
        None => key.to_owned(),
    }
}

/// Whether `value` can be the value of a setting: one line. Git reads it between
/// quotes, escaped, so whatever it holds is safe to write.
#[must_use]
pub fn is_valid_git_setting_value(value: &str) -> bool {
    is_one_line_value(value)
}

/// Whether `name` can be the name of a Git alias: what Git accepts in the
/// name of a configuration variable (a letter, then letters, digits or `-`).
#[must_use]
pub fn is_valid_git_alias_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && name.chars().count() <= MAX_GIT_ALIAS_NAME_CHARS
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Whether `command` can be the command of a Git alias: on one line, and a
/// Git command. One starting with `!` runs a shell command, and one starting
/// with `-` carries options for Git itself (`-c core.pager=…` runs a command).
#[must_use]
pub fn is_valid_git_alias_command(command: &str) -> bool {
    let command = command.trim_start();
    !is_blank(command)
        && command.chars().count() <= MAX_GIT_ALIAS_COMMAND_CHARS
        && !command.chars().any(|c| c.is_control() || is_invisible(c))
        && !command.starts_with(['!', '-'])
}

/// Whether `folder` can be the folder of an identity: `~/`, then one or more
/// names (no `.` nor `..`) joined by `/`, without a trailing `/`, and without
/// what Git reads as a pattern (`*`, `?`, `[`) or as an escape (`\`, `"`).
#[must_use]
pub fn is_valid_git_folder(folder: &str) -> bool {
    let Some(path) = folder.strip_prefix("~/") else {
        return false;
    };
    is_plain_text(folder)
        && !path.is_empty()
        && !path.contains(['*', '?', '[', ']', '\\', '"'])
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

impl GitConfig {
    /// A configuration that sets nothing.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema_version: GIT_SCHEMA_VERSION,
            identity: GitIdentity::default(),
            signing: GitSigning::default(),
            settings: GitSettings::default(),
            aliases: Vec::new(),
            folder_identities: Vec::new(),
            extra_settings: Vec::new(),
            revision: 0,
        }
    }

    /// Checks the configuration: it is read from a file the user can edit,
    /// and what it holds is written to files Git reads and acts on.
    ///
    /// # Errors
    ///
    /// The first [`GitConfigError`] found.
    pub fn validate(&self) -> Result<(), GitConfigError> {
        if self.schema_version != GIT_SCHEMA_VERSION {
            return Err(GitConfigError::UnsupportedSchemaVersion {
                found: self.schema_version,
            });
        }
        self.validate_values()?;
        self.validate_aliases()?;
        self.validate_folder_identities()?;
        check_settings(
            &self.extra_settings,
            is_valid_git_setting_key,
            is_valid_git_setting_value,
            fold_git_key,
        )
        .map_err(|key| GitConfigError::InvalidSetting(key.unwrap_or_default()))
    }

    fn validate_values(&self) -> Result<(), GitConfigError> {
        let invalid = |field| Err(GitConfigError::InvalidValue { field });
        let identity = &self.identity;
        if !identity.name.is_empty() && !is_valid_git_user_name(&identity.name) {
            return invalid("identity.name");
        }
        if !identity.email.is_empty() && !is_valid_git_user_email(&identity.email) {
            return invalid("identity.email");
        }
        if !self.signing.key.is_empty() && !is_valid_git_signing_key(&self.signing.key) {
            return invalid("signing.key");
        }
        let settings = &self.settings;
        if !settings.default_branch.is_empty()
            && !is_valid_git_branch_name(&settings.default_branch)
        {
            return invalid("settings.defaultBranch");
        }
        if !settings.editor.is_empty() && !is_valid_git_editor(&settings.editor) {
            return invalid("settings.editor");
        }
        if let GitCredentialHelper::Cache { timeout_seconds } = settings.credential_helper
            && !CREDENTIAL_CACHE_SECONDS.contains(&timeout_seconds)
        {
            return invalid("settings.credentialHelper");
        }
        Ok(())
    }

    fn validate_aliases(&self) -> Result<(), GitConfigError> {
        if self.aliases.len() > MAX_GIT_ALIASES {
            return Err(GitConfigError::TooManyAliases {
                count: self.aliases.len(),
            });
        }
        let mut known = HashSet::with_capacity(self.aliases.len());
        for alias in &self.aliases {
            if !is_valid_git_alias_name(&alias.name) {
                return Err(GitConfigError::InvalidAliasName(alias.name.clone()));
            }
            if !is_valid_git_alias_command(&alias.command) {
                return Err(GitConfigError::InvalidAliasCommand(alias.name.clone()));
            }
            if !known.insert(alias.name.to_ascii_lowercase()) {
                return Err(GitConfigError::DuplicateAlias(alias.name.clone()));
            }
        }
        Ok(())
    }

    fn validate_folder_identities(&self) -> Result<(), GitConfigError> {
        if self.folder_identities.len() > MAX_GIT_FOLDER_IDENTITIES {
            return Err(GitConfigError::TooManyFolderIdentities {
                count: self.folder_identities.len(),
            });
        }
        let mut known = HashSet::with_capacity(self.folder_identities.len());
        for rule in &self.folder_identities {
            let folder = &rule.folder;
            if !is_valid_git_folder(folder) {
                return Err(GitConfigError::InvalidFolder(folder.clone()));
            }
            let name_ok = rule.name.is_empty() || is_valid_git_user_name(&rule.name);
            let email_ok = rule.email.is_empty() || is_valid_git_user_email(&rule.email);
            let key_ok = rule.signing_key.is_empty() || is_valid_git_signing_key(&rule.signing_key);
            if !(name_ok && email_ok && key_ok) {
                return Err(GitConfigError::InvalidValue {
                    field: "folderIdentities",
                });
            }
            if rule.name.is_empty() && rule.email.is_empty() && rule.signing_key.is_empty() {
                return Err(GitConfigError::EmptyFolderIdentity(folder.clone()));
            }
            if !known.insert(folder.as_str()) {
                return Err(GitConfigError::DuplicateFolder(folder.clone()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    fn alias(name: &str, command: &str) -> GitAlias {
        GitAlias {
            name: name.into(),
            command: command.into(),
            enabled: true,
        }
    }

    fn rule(folder: &str, email: &str) -> GitFolderIdentity {
        GitFolderIdentity {
            folder: folder.into(),
            name: String::new(),
            email: email.into(),
            signing_key: String::new(),
            enabled: true,
        }
    }

    #[derive(Deserialize)]
    struct Case {
        value: String,
        valid: bool,
    }

    #[derive(Deserialize)]
    struct Cases {
        names: Vec<Case>,
        emails: Vec<Case>,
        keys: Vec<Case>,
        branches: Vec<Case>,
        editors: Vec<Case>,
        #[serde(rename = "knownEditors")]
        known_editors: Vec<String>,
        #[serde(rename = "plainEditors")]
        plain_editors: Vec<String>,
        #[serde(rename = "aliasNames")]
        alias_names: Vec<Case>,
        #[serde(rename = "aliasCommands")]
        alias_commands: Vec<Case>,
        folders: Vec<Case>,
        #[serde(rename = "settingKeys")]
        setting_keys: Vec<Case>,
        #[serde(rename = "settingValues")]
        setting_values: Vec<Case>,
    }

    fn verdicts(cases: &[Case], check: fn(&str) -> bool, what: &str) {
        for case in cases {
            assert_eq!(check(&case.value), case.valid, "{what} {:?}", case.value);
        }
    }

    /// The interface checks the same cases (`git-validation.spec.ts`): the two
    /// validations must not drift apart.
    #[test]
    fn the_shared_cases_give_the_same_verdicts_as_the_interface() {
        let cases: Cases =
            serde_json::from_str(include_str!("../tests/fixtures/git-validation.json")).unwrap();
        verdicts(&cases.names, is_valid_git_user_name, "name");
        verdicts(&cases.emails, is_valid_git_user_email, "email");
        verdicts(&cases.keys, is_valid_git_signing_key, "key");
        verdicts(&cases.branches, is_valid_git_branch_name, "branch");
        verdicts(&cases.editors, is_valid_git_editor, "editor");
        verdicts(&cases.alias_names, is_valid_git_alias_name, "alias name");
        verdicts(
            &cases.alias_commands,
            is_valid_git_alias_command,
            "alias command",
        );
        verdicts(&cases.folders, is_valid_git_folder, "folder");
        verdicts(&cases.setting_keys, is_valid_git_setting_key, "setting key");
        verdicts(
            &cases.setting_values,
            is_valid_git_setting_value,
            "setting value",
        );
        assert_eq!(cases.known_editors, GIT_EDITORS);
        assert_eq!(cases.plain_editors, GIT_PLAIN_EDITORS);
    }

    #[test]
    fn an_empty_configuration_is_valid() {
        assert_eq!(GitConfig::empty().validate(), Ok(()));
    }

    #[test]
    fn a_shell_alias_is_refused() {
        for command in [
            "!rm -rf ~",
            "  !echo",
            "-c core.pager=evil log",
            "--exec-path=/x",
        ] {
            assert!(!is_valid_git_alias_command(command), "{command}");
        }
        assert!(is_valid_git_alias_command("log --oneline --graph"));
    }

    #[test]
    fn validation_reports_the_first_problem() {
        let mut config = GitConfig::empty();
        config.identity.email = "nobody".into();
        assert_eq!(
            config.validate(),
            Err(GitConfigError::InvalidValue {
                field: "identity.email"
            })
        );

        let mut config = GitConfig::empty();
        config.aliases = vec![alias("st", "status"), alias("ST", "status -s")];
        assert_eq!(
            config.validate(),
            Err(GitConfigError::DuplicateAlias("ST".into()))
        );

        config.aliases = vec![alias("st", "!ls")];
        assert_eq!(
            config.validate(),
            Err(GitConfigError::InvalidAliasCommand("st".into()))
        );
    }

    #[test]
    fn the_cache_timeout_must_be_within_its_range() {
        let mut config = GitConfig::empty();
        for (seconds, valid) in [(29, false), (30, true), (604_800, true), (604_801, false)] {
            config.settings.credential_helper = GitCredentialHelper::Cache {
                timeout_seconds: seconds,
            };
            assert_eq!(config.validate().is_ok(), valid, "{seconds}");
        }
    }

    #[test]
    fn folder_identities_are_checked() {
        let mut config = GitConfig::empty();
        config.folder_identities = vec![rule("~/work", "a@work.org"), rule("~/work", "b@x.org")];
        assert_eq!(
            config.validate(),
            Err(GitConfigError::DuplicateFolder("~/work".into()))
        );

        config.folder_identities = vec![rule("/etc", "a@work.org")];
        assert_eq!(
            config.validate(),
            Err(GitConfigError::InvalidFolder("/etc".into()))
        );

        config.folder_identities = vec![rule("~/work", "")];
        assert_eq!(
            config.validate(),
            Err(GitConfigError::EmptyFolderIdentity("~/work".into()))
        );

        config.folder_identities = vec![rule("~/work", "not an email")];
        assert_eq!(
            config.validate(),
            Err(GitConfigError::InvalidValue {
                field: "folderIdentities"
            })
        );
    }

    #[test]
    fn the_settings_written_by_the_person_are_checked() {
        let setting = |key: &str, value: &str| AdvancedSetting {
            key: key.into(),
            value: value.into(),
            enabled: true,
        };
        let mut config = GitConfig::empty();
        config.extra_settings = vec![
            setting("push.default", "current"),
            setting("core.autocrlf", "input"),
        ];
        assert_eq!(config.validate(), Ok(()));

        config
            .extra_settings
            .push(setting("Push.Default", "simple"));
        assert_eq!(
            config.validate(),
            Err(GitConfigError::InvalidSetting("Push.Default".into()))
        );
        // A key a form manages, and an alias that runs a shell command by the back door.
        for key in ["user.email", "alias.x", "include.path"] {
            config.extra_settings = vec![setting(key, "!rm -rf ~")];
            assert_eq!(
                config.validate(),
                Err(GitConfigError::InvalidSetting(key.into())),
                "{key}"
            );
        }
    }

    #[test]
    fn a_configuration_saved_before_the_settings_existed_is_still_read() {
        let old = r#"{"schemaVersion":1,"identity":{"name":"","email":""},
            "signing":{"enabled":false,"format":"openPgp","key":""},
            "settings":{"defaultBranch":"","pull":"default","editor":"","credentialHelper":{"kind":"none"}},
            "aliases":[],"folderIdentities":[],"revision":2}"#;
        let config: GitConfig = serde_json::from_str(old).unwrap();
        assert_eq!(config.extra_settings, []);
    }

    #[test]
    fn too_many_items_are_refused() {
        let mut config = GitConfig::empty();
        config.aliases = (0..=MAX_GIT_ALIASES)
            .map(|i| alias(&format!("a{i}"), "status"))
            .collect();
        assert_eq!(
            config.validate(),
            Err(GitConfigError::TooManyAliases {
                count: MAX_GIT_ALIASES + 1
            })
        );
        config.aliases.clear();
        config.folder_identities = (0..=MAX_GIT_FOLDER_IDENTITIES)
            .map(|i| rule(&format!("~/f{i}"), "a@b.c"))
            .collect();
        assert_eq!(
            config.validate(),
            Err(GitConfigError::TooManyFolderIdentities {
                count: MAX_GIT_FOLDER_IDENTITIES + 1
            })
        );
    }

    #[test]
    fn another_schema_version_is_refused() {
        let mut config = GitConfig::empty();
        config.schema_version = 2;
        assert_eq!(
            config.validate(),
            Err(GitConfigError::UnsupportedSchemaVersion { found: 2 })
        );
    }
}
