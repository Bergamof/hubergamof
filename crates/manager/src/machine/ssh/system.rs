//! The hosts `~/.ssh/config` holds that HuBergamof does not manage, read
//! without running `ssh`.
//!
//! Only that file is read: the files it includes are not followed, so their
//! hosts are not listed.

use hubergamof_protocol::SshSystemHost;

use super::apply::SshFiles;
use super::sections::{Header, lines_of, option_of, sections};
use crate::machine::files::{FileError, read_text_file};

/// Most hosts listed.
pub const MAX_SYSTEM_SSH_HOSTS: usize = 256;

/// Longest value kept of an option, in characters.
const MAX_OPTION_CHARS: usize = 255;

/// What stands for the several names or the jokers of a `Host` among the
/// options it cannot be taken over for.
const SEVERAL_PATTERNS: &str = "plusieurs motifs ou jokers";

fn shown(value: &str) -> String {
    value.chars().take(MAX_OPTION_CHARS).collect()
}

/// The hosts of the content of a configuration file, in order: the sections
/// HuBergamof did not comment out.
fn parse_hosts(content: &str) -> Vec<SshSystemHost> {
    let lines = lines_of(content);
    let mut found = Vec::new();
    for section in sections(&lines) {
        let Header::Host(patterns) = &section.header else {
            continue;
        };
        if section.marked {
            continue;
        }
        let mut host = SshSystemHost {
            alias: shown(&patterns.join(" ")),
            host_name: String::new(),
            user: String::new(),
            port: None,
            identity_file: String::new(),
            proxy_jump: String::new(),
            forward_agent: false,
            other_options: Vec::new(),
        };
        if patterns.len() != 1 || patterns.iter().any(|name| name.contains(['*', '?', '!'])) {
            host.other_options.push(SEVERAL_PATTERNS.to_owned());
        }
        let mut seen: Vec<String> = Vec::new();
        for line in &lines[section.lines.start + 1..section.lines.end] {
            let Some((keyword, value)) = option_of(line) else {
                continue;
            };
            // `ssh` takes the first value of an option.
            if seen.contains(&keyword) {
                continue;
            }
            seen.push(keyword.clone());
            match keyword.as_str() {
                "hostname" => host.host_name = shown(&value),
                "user" => host.user = shown(&value),
                "port" => match value.parse::<u16>() {
                    Ok(port) => host.port = Some(port),
                    Err(_) => host.other_options.push(keyword),
                },
                "identityfile" => host.identity_file = shown(&value),
                "proxyjump" => host.proxy_jump = shown(&value),
                "forwardagent" => host.forward_agent = value.eq_ignore_ascii_case("yes"),
                _ => host.other_options.push(keyword),
            }
        }
        found.push(host);
        if found.len() == MAX_SYSTEM_SSH_HOSTS {
            break;
        }
    }
    found
}

impl SshFiles {
    /// The hosts of `~/.ssh/config` that HuBergamof does not manage. A file
    /// that cannot be read is logged and leaves the list empty.
    #[must_use]
    pub fn read_system_hosts(&self) -> Vec<SshSystemHost> {
        match read_text_file(&self.config_file()) {
            Ok(Some(bytes)) => parse_hosts(&String::from_utf8_lossy(&bytes)),
            Ok(None) => Vec::new(),
            Err(error) => {
                log_unreadable(&error);
                Vec::new()
            }
        }
    }
}

fn log_unreadable(error: &FileError) {
    tracing::warn!(%error, "the SSH configuration was not read");
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    #[test]
    fn a_host_is_listed_with_what_it_sets() {
        let content = "Host prod\n\tHostName 10.0.0.1\n\tUser deploy\n\tPort 2222\n\
                       \tIdentityFile ~/.ssh/id\n\tProxyJump bastion\n\tForwardAgent yes\n";
        let hosts = parse_hosts(content);
        assert_eq!(hosts.len(), 1);
        let host = &hosts[0];
        assert_eq!(host.alias, "prod");
        assert_eq!(host.host_name, "10.0.0.1");
        assert_eq!(host.user, "deploy");
        assert_eq!(host.port, Some(2222));
        assert_eq!(host.identity_file, "~/.ssh/id");
        assert_eq!(host.proxy_jump, "bastion");
        assert!(host.forward_agent);
        assert_eq!(host.other_options, Vec::<String>::new());
    }

    #[test]
    fn other_options_are_named_and_keep_the_host_from_being_taken() {
        let hosts = parse_hosts("Host a\n\tUser u\n\tProxyCommand nc %h %p\n\tCompression yes\n");
        assert_eq!(hosts[0].other_options, ["proxycommand", "compression"]);
    }

    #[test]
    fn several_names_and_jokers_cannot_be_taken() {
        let hosts = parse_hosts("Host a b\n\tUser u\nHost web-*\n\tUser w\n");
        assert_eq!(hosts[0].alias, "a b");
        assert_eq!(hosts[0].other_options, [SEVERAL_PATTERNS]);
        assert_eq!(hosts[1].other_options, [SEVERAL_PATTERNS]);
    }

    #[test]
    fn the_first_value_of_an_option_is_the_one_ssh_uses() {
        let hosts = parse_hosts("Host a\n\tUser first\n\tUser second\n");
        assert_eq!(hosts[0].user, "first");
        assert_eq!(hosts[0].other_options, Vec::<String>::new());
    }

    #[test]
    fn a_port_that_is_not_a_number_is_an_option_to_leave_alone() {
        let hosts = parse_hosts("Host a\n\tPort ssh\n");
        assert_eq!(hosts[0].port, None);
        assert_eq!(hosts[0].other_options, ["port"]);
    }

    #[test]
    fn what_hubergamof_commented_out_and_match_sections_are_not_listed() {
        let content =
            "#hubergamof-off# Host a\n#hubergamof-off# \tUser u\nMatch all\n\tUser m\nHost b\n";
        let hosts = parse_hosts(content);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].alias, "b");
    }

    #[test]
    fn the_list_is_bounded() {
        let mut content = String::new();
        for i in 0..MAX_SYSTEM_SSH_HOSTS + 10 {
            writeln!(content, "Host h{i}").unwrap();
        }
        assert_eq!(parse_hosts(&content).len(), MAX_SYSTEM_SSH_HOSTS);
    }

    #[test]
    fn a_file_that_is_missing_or_unreadable_lists_nothing() {
        let home = tempfile::tempdir().unwrap();
        let files = SshFiles::new(home.path().to_owned());
        assert_eq!(files.read_system_hosts(), []);
        std::fs::create_dir(home.path().join(".ssh")).unwrap();
        std::fs::create_dir(home.path().join(".ssh/config")).unwrap();
        assert_eq!(files.read_system_hosts(), []);
    }
}
