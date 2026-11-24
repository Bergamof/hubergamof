//! The hosts written as the file `ssh` reads. Pure text functions.

use std::fmt::Write as _;

use hubergamof_protocol::{
    AdvancedSetting, SshConfig, SshHost, is_valid_ssh_alias, is_valid_ssh_host_name,
    is_valid_ssh_identity_file, is_valid_ssh_option_key, is_valid_ssh_option_value,
    is_valid_ssh_proxy_jump, is_valid_ssh_user,
};
use thiserror::Error;

/// Folder, relative to the home folder, of the files HuBergamof keeps.
pub(super) const MANAGED_DIR: &str = ".config/hubergamof";

/// The file holding the hosts, in [`MANAGED_DIR`].
pub(super) const MANAGED_FILE: &str = "ssh_config";

/// What stands for the hosts in the section of the common options, and in the
/// error about them.
pub(super) const COMMON_PATTERN: &str = "*";

const MANAGED_HEADER: &str = "# Managed by HuBergamof, do not edit: changes are overwritten.\n";

/// A host that cannot be written to the SSH configuration safely.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("the host `{alias}` cannot be written to the SSH configuration: {field}")]
pub struct UnwritableHost {
    /// Its alias.
    pub alias: String,
    /// What is wrong.
    pub field: &'static str,
}

/// Each value is written bare: the characters that the contract allows need no
/// quoting. They are checked again here, the file being read by `ssh`.
fn checked(host: &SshHost) -> Result<(), UnwritableHost> {
    let wrong = |field| {
        Err(UnwritableHost {
            alias: host.alias.clone(),
            field,
        })
    };
    if !is_valid_ssh_alias(&host.alias) {
        return wrong("alias");
    }
    if !host.host_name.is_empty() && !is_valid_ssh_host_name(&host.host_name) {
        return wrong("hostName");
    }
    if !host.user.is_empty() && !is_valid_ssh_user(&host.user) {
        return wrong("user");
    }
    if host.port == Some(0) {
        return wrong("port");
    }
    if !host.identity_file.is_empty() && !is_valid_ssh_identity_file(&host.identity_file) {
        return wrong("identityFile");
    }
    if !host.proxy_jump.is_empty() && !is_valid_ssh_proxy_jump(&host.proxy_jump) {
        return wrong("proxyJump");
    }
    checked_options(&host.alias, &host.options)
}

/// The options written by the person are written as they are: they are
/// checked again here, the file being read by `ssh`.
fn checked_options(scope: &str, options: &[AdvancedSetting]) -> Result<(), UnwritableHost> {
    let writable = |option: &AdvancedSetting| {
        is_valid_ssh_option_key(&option.key) && is_valid_ssh_option_value(&option.value)
    };
    if options.iter().all(writable) {
        Ok(())
    } else {
        Err(UnwritableHost {
            alias: scope.to_owned(),
            field: "options",
        })
    }
}

fn write_options(text: &mut String, options: &[AdvancedSetting]) {
    for option in options.iter().filter(|option| option.enabled) {
        let _ = writeln!(text, "\t{} {}", option.key, option.value);
    }
}

fn section(host: &SshHost) -> String {
    let mut text = format!("Host {}\n", host.alias);
    let mut option = |key: &str, value: &str| {
        let _ = writeln!(text, "\t{key} {value}");
    };
    if !host.host_name.is_empty() {
        option("HostName", &host.host_name);
    }
    if !host.user.is_empty() {
        option("User", &host.user);
    }
    if let Some(port) = host.port {
        option("Port", &port.to_string());
    }
    if !host.identity_file.is_empty() {
        option("IdentityFile", &host.identity_file);
    }
    if !host.proxy_jump.is_empty() {
        option("ProxyJump", &host.proxy_jump);
    }
    if host.forward_agent {
        option("ForwardAgent", "yes");
    }
    write_options(&mut text, &host.options);
    text
}

/// The section of the options common to every host, last of the file: the
/// hosts above keep what they set, as `ssh` takes the first value.
fn common_section(options: &[AdvancedSetting]) -> Option<String> {
    options.iter().any(|option| option.enabled).then(|| {
        let mut text = format!("Host {COMMON_PATTERN}\n");
        write_options(&mut text, options);
        text
    })
}

/// The content of the managed file for the enabled hosts and the common
/// options; `None` when there is none (the file and the include go away).
///
/// # Errors
/// [`UnwritableHost`] for a host the contract would refuse: a validated
/// configuration never has one.
pub(super) fn render(config: &SshConfig) -> Result<Option<String>, UnwritableHost> {
    let hosts: Vec<&SshHost> = config.hosts.iter().filter(|host| host.enabled).collect();
    checked_options(COMMON_PATTERN, &config.common_options)?;
    let common = common_section(&config.common_options);
    if hosts.is_empty() && common.is_none() {
        return Ok(None);
    }
    let mut sections = vec![MANAGED_HEADER.to_owned()];
    for host in hosts {
        checked(host)?;
        sections.push(section(host));
    }
    sections.extend(common);
    Ok(Some(sections.join("\n")))
}

#[cfg(test)]
mod tests {
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
            options: Vec::new(),
        }
    }

    fn option(key: &str, value: &str, enabled: bool) -> AdvancedSetting {
        AdvancedSetting {
            key: key.into(),
            value: value.into(),
            enabled,
        }
    }

    fn config(hosts: Vec<SshHost>) -> SshConfig {
        SshConfig {
            hosts,
            ..SshConfig::empty()
        }
    }

    #[test]
    fn no_enabled_host_writes_nothing() {
        assert_eq!(render(&SshConfig::empty()), Ok(None));
        let mut off = host("a");
        off.enabled = false;
        assert_eq!(render(&config(vec![off])), Ok(None));
    }

    #[test]
    fn a_host_is_written_with_the_options_it_sets_only() {
        let mut full = host("prod");
        full.host_name = "10.0.0.1".into();
        full.user = "deploy".into();
        full.port = Some(2222);
        full.identity_file = "~/.ssh/id_ed25519".into();
        full.proxy_jump = "me@bastion:22,other".into();
        full.forward_agent = true;
        let rendered = render(&config(vec![full, host("bare")])).unwrap().unwrap();
        assert_eq!(
            rendered,
            format!(
                "{MANAGED_HEADER}\nHost prod\n\tHostName 10.0.0.1\n\tUser deploy\n\tPort 2222\n\
                 \tIdentityFile ~/.ssh/id_ed25519\n\tProxyJump me@bastion:22,other\n\
                 \tForwardAgent yes\n\nHost bare\n"
            )
        );
    }

    #[test]
    fn the_options_of_a_host_follow_its_own_and_only_enabled_ones_are_written() {
        let mut with_options = host("prod");
        with_options.user = "deploy".into();
        with_options.options = vec![
            option("Compression", "yes", true),
            option("LogLevel", "ERROR", false),
            option("ProxyCommand", "ssh -W %h:%p bastion", true),
        ];
        let rendered = render(&config(vec![with_options])).unwrap().unwrap();
        assert_eq!(
            rendered,
            format!(
                "{MANAGED_HEADER}\nHost prod\n\tUser deploy\n\tCompression yes\n\
                 \tProxyCommand ssh -W %h:%p bastion\n"
            )
        );
    }

    #[test]
    fn the_common_options_make_a_last_section_even_without_a_host() {
        let mut common = config(vec![host("a")]);
        common.common_options = vec![
            option("ServerAliveInterval", "30", true),
            option("Compression", "yes", false),
        ];
        assert_eq!(
            render(&common).unwrap().unwrap(),
            format!("{MANAGED_HEADER}\nHost a\n\nHost *\n\tServerAliveInterval 30\n")
        );
        common.hosts.clear();
        assert_eq!(
            render(&common).unwrap().unwrap(),
            format!("{MANAGED_HEADER}\nHost *\n\tServerAliveInterval 30\n")
        );
        common.common_options = vec![option("Compression", "yes", false)];
        assert_eq!(render(&common), Ok(None));
    }

    #[test]
    fn options_the_contract_would_refuse_are_refused_even_when_off_the_file() {
        let mut bad = host("a");
        bad.options = vec![option("Host", "evil", true)];
        assert_eq!(render(&config(vec![bad])).unwrap_err().field, "options");
        let mut newline = config(vec![]);
        newline.common_options = vec![option("A", "x\nProxyCommand evil", true)];
        let error = render(&newline).unwrap_err();
        assert_eq!((error.alias.as_str(), error.field), ("*", "options"));
    }

    #[test]
    fn only_enabled_hosts_are_written() {
        let mut off = host("off");
        off.enabled = false;
        let rendered = render(&config(vec![host("on"), off])).unwrap().unwrap();
        assert!(rendered.contains("Host on\n"));
        assert!(!rendered.contains("off"));
    }

    #[test]
    fn what_the_contract_would_refuse_is_refused_whatever_validation_said() {
        let mut jump = host("a");
        jump.proxy_jump = "-oProxyCommand=evil".into();
        let mut name = host("b");
        name.host_name = "x\nProxyCommand evil".into();
        let mut key = host("c");
        key.identity_file = "/etc/shadow".into();
        let mut alias = host("d e");
        alias.user = "u".into();
        for (bad, field) in [
            (jump, "proxyJump"),
            (name, "hostName"),
            (key, "identityFile"),
            (alias, "alias"),
        ] {
            let error = render(&config(vec![bad])).unwrap_err();
            assert_eq!(error.field, field);
        }
    }
}
