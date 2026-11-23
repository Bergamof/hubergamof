//! The configuration of the computer HuBergamof runs on, persisted as
//! `config/machine.json`. For now: the shell aliases.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

/// Machine configuration schema version produced and understood by this crate.
pub const MACHINE_SCHEMA_VERSION: u32 = 1;

/// Most aliases the configuration holds.
pub const MAX_ALIASES: usize = 256;

/// Longest alias name, in characters.
pub const MAX_ALIAS_NAME_CHARS: usize = 64;

/// Longest alias command, in characters.
pub const MAX_ALIAS_COMMAND_CHARS: usize = 1024;

/// A shell alias the user wants on the computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShellAlias {
    /// What is typed in the shell: a letter or `_`, then letters, digits,
    /// `_`, `.` or `-`.
    pub name: String,
    /// What it runs: one line, no control character.
    pub command: String,
    /// Whether the alias is on the computer. A disabled one stays in the
    /// configuration, and is removed from the computer.
    pub enabled: bool,
}

/// An alias found in the shell startup files of the user, which HuBergamof
/// does not manage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SystemAlias {
    /// Its name.
    pub name: String,
    /// Its command, as written in the file (quotes removed).
    pub command: String,
    /// The file it was read from, relative to the home folder (`.bashrc`).
    pub source: String,
}

/// The configuration of the machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MachineConfig {
    /// Schema version, see [`MACHINE_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The aliases, in the order they are shown and written.
    pub aliases: Vec<ShellAlias>,
    /// Revision counter, incremented on every accepted update; an update
    /// carrying an outdated revision is rejected.
    pub revision: u64,
}

/// Why a machine configuration is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MachineConfigError {
    /// The schema version is not supported by this version of HuBergamof.
    #[error("unsupported machine schema version {found} (supported: {MACHINE_SCHEMA_VERSION})")]
    UnsupportedSchemaVersion {
        /// Version found in the configuration.
        found: u32,
    },
    /// There are more aliases than [`MAX_ALIASES`].
    #[error("too many aliases: {count} (max {MAX_ALIASES})")]
    TooManyAliases {
        /// How many.
        count: usize,
    },
    /// An alias name is not valid, see [`is_valid_alias_name`].
    #[error("invalid alias name {0:?}")]
    InvalidName(String),
    /// An alias command is empty, too long, on several lines or holds a
    /// control character.
    #[error("invalid command for alias `{0}`")]
    InvalidCommand(String),
    /// Two aliases share a name.
    #[error("duplicate alias `{0}`")]
    DuplicateName(String),
}

/// Whether `name` can be an alias name: the shell would take anything else
/// for something other than a name, and it ends up in a file the shell runs.
#[must_use]
pub fn is_valid_alias_name(name: &str) -> bool {
    let mut chars = name.chars();
    let starts_well = chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_');
    starts_well
        && name.chars().count() <= MAX_ALIAS_NAME_CHARS
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Zero width no-break space: not white space for Rust, but for JavaScript,
/// which the interface runs on; both must agree on what a blank command is.
const BYTE_ORDER_MARK: char = '\u{FEFF}';

/// Characters that show nothing or reorder the text around them: a command
/// containing one reads as something other than what it runs (the same set as
/// the interface, `alias-validation.ts`).
fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}'
        | '\u{2028}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}'
        | BYTE_ORDER_MARK)
}

fn is_blank(command: &str) -> bool {
    command
        .chars()
        .all(|c| c.is_whitespace() || c == BYTE_ORDER_MARK)
}

/// Whether `command` can be the command of an alias.
#[must_use]
pub fn is_valid_alias_command(command: &str) -> bool {
    !is_blank(command)
        && command.chars().count() <= MAX_ALIAS_COMMAND_CHARS
        && !command.chars().any(|c| c.is_control() || is_invisible(c))
}

impl MachineConfig {
    /// A configuration without any alias.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema_version: MACHINE_SCHEMA_VERSION,
            aliases: Vec::new(),
            revision: 0,
        }
    }

    /// Checks the configuration: it is read from a file the user can edit,
    /// and its aliases are written to a file the shell runs.
    ///
    /// # Errors
    ///
    /// The first [`MachineConfigError`] found.
    pub fn validate(&self) -> Result<(), MachineConfigError> {
        if self.schema_version != MACHINE_SCHEMA_VERSION {
            return Err(MachineConfigError::UnsupportedSchemaVersion {
                found: self.schema_version,
            });
        }
        if self.aliases.len() > MAX_ALIASES {
            return Err(MachineConfigError::TooManyAliases {
                count: self.aliases.len(),
            });
        }
        let mut known = HashSet::with_capacity(self.aliases.len());
        for alias in &self.aliases {
            if !is_valid_alias_name(&alias.name) {
                return Err(MachineConfigError::InvalidName(alias.name.clone()));
            }
            if !is_valid_alias_command(&alias.command) {
                return Err(MachineConfigError::InvalidCommand(alias.name.clone()));
            }
            if !known.insert(alias.name.as_str()) {
                return Err(MachineConfigError::DuplicateName(alias.name.clone()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alias(name: &str, command: &str) -> ShellAlias {
        ShellAlias {
            name: name.into(),
            command: command.into(),
            enabled: true,
        }
    }

    fn config(aliases: Vec<ShellAlias>) -> MachineConfig {
        MachineConfig {
            aliases,
            ..MachineConfig::empty()
        }
    }

    #[test]
    fn an_empty_configuration_is_valid() {
        assert_eq!(MachineConfig::empty().validate(), Ok(()));
    }

    #[test]
    fn names_follow_the_shell_rules() {
        for good in ["ll", "_x", "git.st", "my-alias", "a1"] {
            assert!(is_valid_alias_name(good), "{good}");
        }
        for bad in [
            "", "1a", "-a", "a b", "a=b", "a;b", "a$b", "é", "a/b", "a\n",
        ] {
            assert!(!is_valid_alias_name(bad), "{bad:?}");
        }
        assert!(!is_valid_alias_name(&"a".repeat(MAX_ALIAS_NAME_CHARS + 1)));
    }

    #[test]
    fn commands_are_one_line_without_control_characters() {
        assert!(is_valid_alias_command("ls -la --color=auto"));
        assert!(is_valid_alias_command("echo 'it''s' && true"));
        for bad in ["", "   ", "a\nb", "a\rb", "a\0b", "a\tb"] {
            assert!(!is_valid_alias_command(bad), "{bad:?}");
        }
        assert!(!is_valid_alias_command(
            &"a".repeat(MAX_ALIAS_COMMAND_CHARS + 1)
        ));
    }

    #[test]
    fn validation_reports_the_first_problem() {
        assert_eq!(
            config(vec![alias("bad name", "ls")]).validate(),
            Err(MachineConfigError::InvalidName("bad name".into()))
        );
        assert_eq!(
            config(vec![alias("ll", "")]).validate(),
            Err(MachineConfigError::InvalidCommand("ll".into()))
        );
        assert_eq!(
            config(vec![alias("ll", "ls"), alias("ll", "dir")]).validate(),
            Err(MachineConfigError::DuplicateName("ll".into()))
        );
    }

    #[test]
    fn a_command_of_white_space_only_is_blank() {
        for blank in ["\u{FEFF}", "\u{A0}", "\u{2003}", " \u{FEFF} "] {
            assert!(!is_valid_alias_command(blank), "{blank:?}");
        }
        assert!(is_valid_alias_command(" ls "));
    }

    #[test]
    fn invisible_and_bidirectional_characters_are_refused_in_commands() {
        for bad in [
            "ls\u{202E}",
            "a\u{2028}b",
            "a\u{200B}b",
            "a\u{2066}b",
            "a\u{FEFF}b",
        ] {
            assert!(!is_valid_alias_command(bad), "{bad:?}");
        }
        assert!(is_valid_alias_command("echo é ✓"));
    }

    #[test]
    fn an_invalid_command_is_reported_before_a_duplicate_name() {
        assert_eq!(
            config(vec![alias("ll", "ls"), alias("ll", "")]).validate(),
            Err(MachineConfigError::InvalidCommand("ll".into()))
        );
    }

    #[test]
    fn too_many_aliases_are_refused() {
        let aliases = (0..=MAX_ALIASES)
            .map(|i| alias(&format!("a{i}"), "ls"))
            .collect();
        assert_eq!(
            config(aliases).validate(),
            Err(MachineConfigError::TooManyAliases {
                count: MAX_ALIASES + 1
            })
        );
    }

    #[test]
    fn another_schema_version_is_refused() {
        let mut machine = MachineConfig::empty();
        machine.schema_version = 2;
        assert_eq!(
            machine.validate(),
            Err(MachineConfigError::UnsupportedSchemaVersion { found: 2 })
        );
    }
}

#[cfg(test)]
mod parity {
    use serde::Deserialize;

    use super::*;

    #[derive(Deserialize)]
    struct Case {
        value: String,
        valid: bool,
    }

    #[derive(Deserialize)]
    struct Cases {
        names: Vec<Case>,
        commands: Vec<Case>,
    }

    /// The interface checks the same cases (`alias-validation.spec.ts`): the
    /// two validations must not drift apart.
    #[test]
    fn the_shared_cases_give_the_same_verdicts_as_the_interface() {
        let cases: Cases =
            serde_json::from_str(include_str!("../tests/fixtures/alias-validation.json")).unwrap();
        for case in &cases.names {
            assert_eq!(
                is_valid_alias_name(&case.value),
                case.valid,
                "name {:?}",
                case.value
            );
        }
        for case in &cases.commands {
            assert_eq!(
                is_valid_alias_command(&case.value),
                case.valid,
                "command {:?}",
                case.value.chars().take(40).collect::<String>()
            );
        }
    }
}
