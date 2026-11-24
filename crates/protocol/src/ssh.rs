//! The SSH hosts of the user, persisted as `config/ssh.json`: the short names
//! they connect with and what each stands for.
//!
//! `ssh` runs some of what it reads from its configuration (`ProxyCommand`,
//! `LocalCommand`…), so a host holds only options that cannot run a program,
//! and what it names (a user, a host, a file) is written with a closed set of
//! characters, never as free text.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

/// SSH configuration schema version produced and understood by this crate.
pub const SSH_SCHEMA_VERSION: u32 = 1;

/// Most hosts the configuration holds.
pub const MAX_SSH_HOSTS: usize = 128;

/// Longest alias, user and path component, in characters.
pub const MAX_SSH_NAME_CHARS: usize = 64;

/// Longest host name, identity file and jump list, in characters.
pub const MAX_SSH_VALUE_CHARS: usize = 255;

/// Most hops a jump list holds.
pub const MAX_SSH_JUMPS: usize = 5;

/// Longest IPv6 address, in characters.
const MAX_IPV6_CHARS: usize = 45;

/// Where the identity files are kept: nothing outside this folder can be named.
pub const SSH_FOLDER: &str = "~/.ssh/";

/// A host the user connects to by a short name (`Host` of `~/.ssh/config`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SshHost {
    /// The name typed after `ssh`: letters, digits, `.`, `_` and `-`. No joker:
    /// one name is one host.
    pub alias: String,
    /// The machine it stands for (`HostName`): a name or an address; empty is
    /// the alias itself.
    pub host_name: String,
    /// The account (`User`); empty is the current one.
    pub user: String,
    /// The port (`Port`); none is 22.
    pub port: Option<u16>,
    /// The private key (`IdentityFile`), in `~/.ssh/`; empty lets `ssh` look
    /// at the usual ones.
    pub identity_file: String,
    /// The machines to go through (`ProxyJump`), `[user@]host[:port]` joined
    /// by `,` (an IPv6 address between brackets); empty is a direct connection.
    pub proxy_jump: String,
    /// Whether the agent of the local machine is reachable from the remote one
    /// (`ForwardAgent`): whoever controls the remote machine can use its keys
    /// while connected.
    pub forward_agent: bool,
    /// Whether the host is on the computer. A disabled one stays in the
    /// configuration and is removed from the computer.
    pub enabled: bool,
}

/// A host found in `~/.ssh/config` that HuBergamof does not manage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SshSystemHost {
    /// What follows `Host`, as written (several names when there are several).
    pub alias: String,
    /// `HostName`, empty when the host has none.
    pub host_name: String,
    /// `User`.
    pub user: String,
    /// `Port`.
    pub port: Option<u16>,
    /// `IdentityFile`.
    pub identity_file: String,
    /// `ProxyJump`.
    pub proxy_jump: String,
    /// `ForwardAgent` is `yes`.
    pub forward_agent: bool,
    /// The options of the host that HuBergamof does not manage, by name. A
    /// host that has some cannot be taken over: commenting it out would lose
    /// them.
    pub other_options: Vec<String>,
}

/// The SSH configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SshConfig {
    /// Schema version, see [`SSH_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The hosts, in the order they are shown and written.
    pub hosts: Vec<SshHost>,
    /// Revision counter, incremented on every accepted update; an update
    /// carrying an outdated revision is rejected.
    pub revision: u64,
}

/// Why an SSH configuration is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SshConfigError {
    /// The schema version is not supported by this version of HuBergamof.
    #[error("unsupported ssh schema version {found} (supported: {SSH_SCHEMA_VERSION})")]
    UnsupportedSchemaVersion {
        /// Version found in the configuration.
        found: u32,
    },
    /// There are more hosts than [`MAX_SSH_HOSTS`].
    #[error("too many ssh hosts: {count} (max {MAX_SSH_HOSTS})")]
    TooManyHosts {
        /// How many.
        count: usize,
    },
    /// An alias is not valid, see [`is_valid_ssh_alias`].
    #[error("invalid ssh alias {0:?}")]
    InvalidAlias(String),
    /// Two hosts share an alias.
    #[error("duplicate ssh alias `{0}`")]
    DuplicateAlias(String),
    /// A value of a host is not valid; `field` says which.
    #[error("invalid {field} for ssh host `{alias}`")]
    InvalidValue {
        /// The alias of the host.
        alias: String,
        /// The field.
        field: &'static str,
    },
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// A name made of letters, digits, `.`, `_` and `-`, not starting with `-` (it
/// would be read as an option by the programs that get it as an argument).
fn is_word(text: &str, max_chars: usize) -> bool {
    !text.is_empty()
        && text.chars().count() <= max_chars
        && text.chars().all(is_word_char)
        && !text.starts_with('-')
}

/// Whether `alias` can be the name of a host: a word, so no joker (`*`, `?`,
/// `!`) and no space.
#[must_use]
pub fn is_valid_ssh_alias(alias: &str) -> bool {
    is_word(alias, MAX_SSH_NAME_CHARS)
}

/// Whether `user` can be `User`.
#[must_use]
pub fn is_valid_ssh_user(user: &str) -> bool {
    is_word(user, MAX_SSH_NAME_CHARS)
}

/// Whether `host` can be `HostName`: a name or an address (IPv6 included), as
/// letters, digits, `.`, `_`, `-` and `:`.
#[must_use]
pub fn is_valid_ssh_host_name(host: &str) -> bool {
    !host.is_empty()
        && host.chars().count() <= MAX_SSH_VALUE_CHARS
        && host.chars().all(|c| is_word_char(c) || c == ':')
        && !host.starts_with('-')
}

/// Whether `path` can be `IdentityFile`: `~/.ssh/` and names (no `.` nor `..`)
/// joined by `/`.
#[must_use]
pub fn is_valid_ssh_identity_file(path: &str) -> bool {
    let Some(relative) = path.strip_prefix(SSH_FOLDER) else {
        return false;
    };
    path.chars().count() <= MAX_SSH_VALUE_CHARS
        && !relative.is_empty()
        && relative
            .split('/')
            .all(|part| is_word(part, MAX_SSH_NAME_CHARS) && part != "." && part != "..")
}

/// Whether `address` can stand between the brackets of an IPv6 hop: hexadecimal
/// digits, `:` and `.`, with at least one `:`.
fn is_ipv6_address(address: &str) -> bool {
    address.contains(':')
        && address.chars().count() <= MAX_IPV6_CHARS
        && address
            .chars()
            .all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '.'))
}

/// The machine and the port of a hop, once its user is taken off: `host`,
/// `host:port`, `[ipv6]` or `[ipv6]:port`. `ssh` refuses a whole configuration
/// for a jump it cannot read, so an address with `:` is accepted only between
/// brackets.
fn machine_and_port(rest: &str) -> Option<(&str, Option<&str>)> {
    if let Some(inner) = rest.strip_prefix('[') {
        let (address, after) = inner.split_once(']')?;
        if !is_ipv6_address(address) {
            return None;
        }
        return match after {
            "" => Some((address, None)),
            _ => Some((address, Some(after.strip_prefix(':')?))),
        };
    }
    let (host, port) = match rest.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (rest, None),
    };
    (is_valid_ssh_host_name(host) && !host.contains(':')).then_some((host, port))
}

/// Whether `entry` is a hop: `[user@]host[:port]`.
fn is_valid_jump(entry: &str) -> bool {
    let (user, rest) = entry.split_once('@').unwrap_or(("", entry));
    let user_ok = !entry.contains('@') || is_valid_ssh_user(user);
    user_ok
        && machine_and_port(rest).is_some_and(|(_, port)| {
            port.is_none_or(|port| port.parse::<u16>().is_ok_and(|port| port != 0))
        })
}

/// Whether `jumps` can be `ProxyJump`: hops joined by `,`.
#[must_use]
pub fn is_valid_ssh_proxy_jump(jumps: &str) -> bool {
    jumps.chars().count() <= MAX_SSH_VALUE_CHARS
        && !jumps.is_empty()
        && jumps.split(',').count() <= MAX_SSH_JUMPS
        && jumps.split(',').all(is_valid_jump)
}

impl SshConfig {
    /// A configuration without any host.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema_version: SSH_SCHEMA_VERSION,
            hosts: Vec::new(),
            revision: 0,
        }
    }

    /// Checks the configuration: it is read from a file the user can edit,
    /// and its hosts are written to a file `ssh` reads and acts on.
    ///
    /// # Errors
    ///
    /// The first [`SshConfigError`] found.
    pub fn validate(&self) -> Result<(), SshConfigError> {
        if self.schema_version != SSH_SCHEMA_VERSION {
            return Err(SshConfigError::UnsupportedSchemaVersion {
                found: self.schema_version,
            });
        }
        if self.hosts.len() > MAX_SSH_HOSTS {
            return Err(SshConfigError::TooManyHosts {
                count: self.hosts.len(),
            });
        }
        let mut known = HashSet::with_capacity(self.hosts.len());
        for host in &self.hosts {
            if !is_valid_ssh_alias(&host.alias) {
                return Err(SshConfigError::InvalidAlias(host.alias.clone()));
            }
            host.validate_values()?;
            if !known.insert(host.alias.as_str()) {
                return Err(SshConfigError::DuplicateAlias(host.alias.clone()));
            }
        }
        Ok(())
    }
}

impl SshHost {
    fn validate_values(&self) -> Result<(), SshConfigError> {
        let invalid = |field| {
            Err(SshConfigError::InvalidValue {
                alias: self.alias.clone(),
                field,
            })
        };
        if !self.host_name.is_empty() && !is_valid_ssh_host_name(&self.host_name) {
            return invalid("hostName");
        }
        if !self.user.is_empty() && !is_valid_ssh_user(&self.user) {
            return invalid("user");
        }
        if self.port == Some(0) {
            return invalid("port");
        }
        if !self.identity_file.is_empty() && !is_valid_ssh_identity_file(&self.identity_file) {
            return invalid("identityFile");
        }
        if !self.proxy_jump.is_empty() && !is_valid_ssh_proxy_jump(&self.proxy_jump) {
            return invalid("proxyJump");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    fn host(alias: &str) -> SshHost {
        SshHost {
            alias: alias.into(),
            host_name: String::new(),
            user: String::new(),
            port: None,
            identity_file: String::new(),
            proxy_jump: String::new(),
            forward_agent: false,
            enabled: true,
        }
    }

    fn config(hosts: Vec<SshHost>) -> SshConfig {
        SshConfig {
            hosts,
            ..SshConfig::empty()
        }
    }

    #[derive(Deserialize)]
    struct Case {
        value: String,
        valid: bool,
    }

    #[derive(Deserialize)]
    struct Cases {
        aliases: Vec<Case>,
        users: Vec<Case>,
        #[serde(rename = "hostNames")]
        host_names: Vec<Case>,
        #[serde(rename = "identityFiles")]
        identity_files: Vec<Case>,
        #[serde(rename = "proxyJumps")]
        proxy_jumps: Vec<Case>,
    }

    fn verdicts(cases: &[Case], check: fn(&str) -> bool, what: &str) {
        for case in cases {
            assert_eq!(check(&case.value), case.valid, "{what} {:?}", case.value);
        }
    }

    /// The interface checks the same cases (`ssh-validation.spec.ts`): the two
    /// validations must not drift apart.
    #[test]
    fn the_shared_cases_give_the_same_verdicts_as_the_interface() {
        let cases: Cases =
            serde_json::from_str(include_str!("../tests/fixtures/ssh-validation.json")).unwrap();
        verdicts(&cases.aliases, is_valid_ssh_alias, "alias");
        verdicts(&cases.users, is_valid_ssh_user, "user");
        verdicts(&cases.host_names, is_valid_ssh_host_name, "host name");
        verdicts(
            &cases.identity_files,
            is_valid_ssh_identity_file,
            "identity file",
        );
        verdicts(&cases.proxy_jumps, is_valid_ssh_proxy_jump, "proxy jump");
    }

    #[test]
    fn an_empty_configuration_is_valid() {
        assert_eq!(SshConfig::empty().validate(), Ok(()));
    }

    #[test]
    fn validation_reports_the_first_problem() {
        assert_eq!(
            config(vec![host("bad name")]).validate(),
            Err(SshConfigError::InvalidAlias("bad name".into()))
        );
        assert_eq!(
            config(vec![host("a"), host("a")]).validate(),
            Err(SshConfigError::DuplicateAlias("a".into()))
        );
        let mut proxied = host("p");
        proxied.proxy_jump = "-oProxyCommand=evil".into();
        assert_eq!(
            config(vec![proxied]).validate(),
            Err(SshConfigError::InvalidValue {
                alias: "p".into(),
                field: "proxyJump"
            })
        );
        let mut ported = host("q");
        ported.port = Some(0);
        assert_eq!(
            config(vec![ported]).validate(),
            Err(SshConfigError::InvalidValue {
                alias: "q".into(),
                field: "port"
            })
        );
    }

    #[test]
    fn too_many_hosts_are_refused() {
        let hosts = (0..=MAX_SSH_HOSTS)
            .map(|i| host(&format!("h{i}")))
            .collect();
        assert_eq!(
            config(hosts).validate(),
            Err(SshConfigError::TooManyHosts {
                count: MAX_SSH_HOSTS + 1
            })
        );
    }

    #[test]
    fn another_schema_version_is_refused() {
        let mut ssh = SshConfig::empty();
        ssh.schema_version = 2;
        assert_eq!(
            ssh.validate(),
            Err(SshConfigError::UnsupportedSchemaVersion { found: 2 })
        );
    }
}
