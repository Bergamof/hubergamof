//! The Git hub and the files it writes, checked against the real `git`: what
//! is written must be read back by Git as it was meant.

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::Command;

use hubergamof_protocol::{
    GitAlias, GitCredentialHelper, GitFolderIdentity, GitPullMode, GitSigningFormat,
};

use super::*;
use crate::testing::builtin_registry;

struct Fixture {
    config_dir: tempfile::TempDir,
    home: tempfile::TempDir,
    hub: GitConfigHub,
}

fn fixture() -> Fixture {
    let config_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(config_dir.path(), Arc::new(builtin_registry()));
    let hub = GitConfigHub::new(store, Some(GitFiles::new(home.path().to_owned(), false)));
    Fixture {
        config_dir,
        home,
        hub,
    }
}

fn read(home: &Path, relative: &str) -> String {
    fs::read_to_string(home.join(relative)).unwrap()
}

/// What Git itself reads for `key`, run in `directory` (a repository or not),
/// with only the files of `home`.
fn git_get(home: &Path, directory: &Path, key: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["config", "--get", key])
        .current_dir(directory)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    output.status.success().then(|| {
        String::from_utf8(output.stdout)
            .unwrap()
            .trim_end()
            .to_owned()
    })
}

fn alias(name: &str, command: &str, enabled: bool) -> GitAlias {
    GitAlias {
        name: name.into(),
        command: command.into(),
        enabled,
    }
}

fn folder_rule(folder: &str, name: &str, email: &str) -> GitFolderIdentity {
    GitFolderIdentity {
        folder: folder.into(),
        name: name.into(),
        email: email.into(),
        signing_key: String::new(),
        enabled: true,
    }
}

async fn saved(fixture: &Fixture, change: impl FnOnce(&mut GitConfig)) -> GitConfig {
    let mut config = fixture.hub.refresh().await.unwrap().config;
    change(&mut config);
    fixture.hub.update(config).await.unwrap();
    fixture.hub.refresh().await.unwrap().config
}

#[tokio::test]
async fn the_demo_never_touches_the_real_home() {
    let config_dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(config_dir.path(), Arc::new(builtin_registry()));
    let hub = GitConfigHub::demo(store).unwrap();
    let snapshot = hub.refresh().await.unwrap();
    assert!(snapshot.git_available);
    let keys: Vec<_> = snapshot
        .system_entries
        .iter()
        .map(|e| e.key.as_str())
        .collect();
    assert_eq!(keys, ["user.name", "user.email", "alias.st", "alias.lg"]);

    let mut config = snapshot.config;
    config.identity.name = "Grace Hopper".into();
    hub.update(config).await.unwrap();
    let after = hub.refresh().await.unwrap();
    assert_eq!(
        after.system_entries.len(),
        4,
        "our own values are not listed"
    );
}

#[tokio::test]
async fn git_reads_back_what_is_written() {
    let fixture = fixture();
    let home = fixture.home.path();
    let config = saved(&fixture, |config| {
        config.identity.name = r#"Ada "The Countess" Lovelace"#.into();
        config.identity.email = "ada@example.org".into();
        config.signing.enabled = true;
        config.signing.format = GitSigningFormat::Ssh;
        config.signing.key = "~/.ssh/id_ed25519.pub".into();
        config.settings.default_branch = "main".into();
        config.settings.pull = GitPullMode::FastForwardOnly;
        config.settings.editor = "nano".into();
        config.settings.credential_helper = GitCredentialHelper::Cache {
            timeout_seconds: 600,
        };
        config.aliases = vec![
            alias("co", "checkout", true),
            alias("lg", "log --format=\"%h # %s\"", true),
            alias("off", "status", false),
        ];
    })
    .await;
    assert_eq!(config.revision, 1);

    let get = |key: &str| git_get(home, home, key);
    assert_eq!(
        get("user.name").as_deref(),
        Some(r#"Ada "The Countess" Lovelace"#)
    );
    assert_eq!(get("user.email").as_deref(), Some("ada@example.org"));
    assert_eq!(get("commit.gpgsign").as_deref(), Some("true"));
    assert_eq!(get("gpg.format").as_deref(), Some("ssh"));
    assert_eq!(
        get("user.signingkey").as_deref(),
        Some("~/.ssh/id_ed25519.pub")
    );
    assert_eq!(get("init.defaultBranch").as_deref(), Some("main"));
    assert_eq!(get("pull.ff").as_deref(), Some("only"));
    assert_eq!(get("core.editor").as_deref(), Some("nano"));
    assert_eq!(
        get("credential.helper").as_deref(),
        Some("cache --timeout=600")
    );
    assert_eq!(get("alias.co").as_deref(), Some("checkout"));
    assert_eq!(get("alias.lg").as_deref(), Some("log --format=\"%h # %s\""));
    assert_eq!(
        get("alias.off"),
        None,
        "a disabled alias is not on the computer"
    );
}

#[tokio::test]
async fn the_identity_of_a_folder_wins_inside_it_and_only_there() {
    let fixture = fixture();
    let home = fixture.home.path();
    saved(&fixture, |config| {
        config.identity.name = "Ada".into();
        config.identity.email = "ada@home.org".into();
        config.folder_identities = vec![folder_rule("~/work", "Ada Work", "ada@work.org")];
    })
    .await;

    let inside = home.join("work/project");
    let outside = home.join("perso/project");
    for repository in [&inside, &outside] {
        fs::create_dir_all(repository).unwrap();
        let status = Command::new("git")
            .args(["init", "-q"])
            .current_dir(repository)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(status.success());
    }
    assert_eq!(
        git_get(home, &inside, "user.email").as_deref(),
        Some("ada@work.org")
    );
    assert_eq!(
        git_get(home, &inside, "user.name").as_deref(),
        Some("Ada Work")
    );
    assert_eq!(
        git_get(home, &outside, "user.email").as_deref(),
        Some("ada@home.org")
    );
    assert_eq!(git_get(home, &outside, "user.name").as_deref(), Some("Ada"));
}

#[tokio::test]
async fn the_gitconfig_of_the_user_only_gets_a_block_and_gets_it_back_as_it_was() {
    let fixture = fixture();
    let home = fixture.home.path();
    let original = "[core]\n\tpager = less\n[user]\n\tname = Before\n";
    fs::write(home.join(".gitconfig"), original).unwrap();

    saved(&fixture, |config| config.identity.name = "After".into()).await;
    let with_block = read(home, ".gitconfig");
    assert!(with_block.starts_with(original));
    assert!(with_block.contains(
        "# >>> hubergamof git >>>\n[include]\n\tpath = ~/.config/hubergamof/gitconfig\n"
    ));
    assert_eq!(
        git_get(home, home, "user.name").as_deref(),
        Some("After"),
        "ours comes last"
    );
    assert_eq!(git_get(home, home, "core.pager").as_deref(), Some("less"));

    saved(&fixture, |config| config.identity.name.clear()).await;
    assert_eq!(read(home, ".gitconfig"), original);
    assert!(!home.join(".config/hubergamof/gitconfig").exists());
    assert_eq!(git_get(home, home, "user.name").as_deref(), Some("Before"));
}

#[tokio::test]
async fn a_gitconfig_that_did_not_exist_is_created_then_removed() {
    let fixture = fixture();
    let home = fixture.home.path();
    saved(&fixture, |config| {
        config.identity.email = "ada@example.org".into();
    })
    .await;
    assert!(home.join(".gitconfig").is_file());
    assert_eq!(
        fs::metadata(home.join(".gitconfig"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );

    saved(&fixture, |config| config.identity.email.clear()).await;
    assert!(!home.join(".gitconfig").exists());
}

#[tokio::test]
async fn the_xdg_configuration_gets_the_block_when_it_is_the_one_that_exists() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::create_dir_all(home.join(".config/git")).unwrap();
    fs::write(home.join(".config/git/config"), "[core]\n\tpager = less\n").unwrap();
    saved(&fixture, |config| config.identity.name = "Ada".into()).await;
    assert!(read(home, ".config/git/config").contains("hubergamof git"));
    assert!(!home.join(".gitconfig").exists());
    assert_eq!(git_get(home, home, "user.name").as_deref(), Some("Ada"));
}

#[tokio::test]
async fn a_linked_gitconfig_keeps_its_link() {
    let fixture = fixture();
    let home = fixture.home.path();
    let dotfiles = tempfile::tempdir().unwrap();
    let real = dotfiles.path().join("gitconfig");
    fs::write(&real, "[core]\n\tpager = less\n").unwrap();
    symlink(&real, home.join(".gitconfig")).unwrap();
    saved(&fixture, |config| config.identity.name = "Ada".into()).await;
    assert!(
        fs::symlink_metadata(home.join(".gitconfig"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::read_to_string(&real)
            .unwrap()
            .contains("hubergamof git")
    );
}

#[tokio::test]
async fn a_dangling_link_is_refused_not_replaced() {
    let fixture = fixture();
    let home = fixture.home.path();
    symlink(home.join("nowhere"), home.join(".gitconfig")).unwrap();
    let mut config = fixture.hub.refresh().await.unwrap().config;
    config.identity.name = "Ada".into();
    let error = fixture.hub.update(config).await.unwrap_err();
    assert!(matches!(error, GitError::Files(_)), "{error}");
    assert!(
        fs::symlink_metadata(home.join(".gitconfig"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[tokio::test]
async fn the_files_of_hubergamof_are_private_and_stale_folder_files_go() {
    let fixture = fixture();
    let home = fixture.home.path();
    saved(&fixture, |config| {
        config.folder_identities = vec![
            folder_rule("~/a", "A", "a@x.org"),
            folder_rule("~/b", "B", "b@x.org"),
        ];
    })
    .await;
    let dir = home.join(".config/hubergamof");
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for name in ["gitconfig", "gitconfig-folder-1", "gitconfig-folder-2"] {
        assert_eq!(
            fs::metadata(dir.join(name)).unwrap().permissions().mode() & 0o777,
            0o600,
            "{name}"
        );
    }

    saved(&fixture, |config| {
        config.folder_identities.pop();
    })
    .await;
    assert!(dir.join("gitconfig-folder-1").is_file());
    assert!(!dir.join("gitconfig-folder-2").exists());

    saved(&fixture, |config| config.folder_identities.clear()).await;
    assert!(!dir.join("gitconfig-folder-1").exists());
    assert!(!dir.join("gitconfig").exists());
}

#[tokio::test]
async fn the_aliases_of_the_shell_are_left_alone() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::create_dir_all(home.join(".config/hubergamof")).unwrap();
    fs::write(
        home.join(".config/hubergamof/aliases.sh"),
        "alias ll='ls -l'\n",
    )
    .unwrap();
    saved(&fixture, |config| config.identity.name = "Ada".into()).await;
    saved(&fixture, |config| config.identity.name.clear()).await;
    assert_eq!(
        read(home, ".config/hubergamof/aliases.sh"),
        "alias ll='ls -l'\n"
    );
}

#[tokio::test]
async fn a_folder_that_is_a_link_is_refused() {
    let fixture = fixture();
    let home = fixture.home.path();
    let elsewhere = tempfile::tempdir().unwrap();
    symlink(elsewhere.path(), home.join("work")).unwrap();
    let mut config = fixture.hub.refresh().await.unwrap().config;
    config.folder_identities = vec![folder_rule("~/work/clients", "A", "a@x.org")];
    let error = fixture.hub.update(config).await.unwrap_err();
    assert!(
        matches!(error, GitError::Files(GitFilesError::LinkedFolder(_))),
        "{error}"
    );
    assert!(!home.join(".config/hubergamof/gitconfig").exists());
}

#[tokio::test]
async fn a_folder_that_does_not_exist_yet_is_accepted() {
    let fixture = fixture();
    saved(&fixture, |config| {
        config.folder_identities = vec![folder_rule("~/not/created/yet", "A", "a@x.org")];
    })
    .await;
    assert!(
        fixture
            .home
            .path()
            .join(".config/hubergamof/gitconfig-folder-1")
            .is_file()
    );
}

#[tokio::test]
async fn a_damaged_block_is_left_untouched_and_the_configuration_stays_saved() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::write(
        home.join(".gitconfig"),
        "# >>> hubergamof git >>>\n[user]\n",
    )
    .unwrap();
    let mut changes = fixture.hub.subscribe();
    let mut config = fixture.hub.refresh().await.unwrap().config;
    config.identity.name = "Ada".into();
    let error = fixture.hub.update(config).await.unwrap_err();
    assert!(matches!(error, GitError::Files(_)), "{error}");
    assert_eq!(error.error_code(), ErrorCode::Internal);
    assert_eq!(
        read(home, ".gitconfig"),
        "# >>> hubergamof git >>>\n[user]\n"
    );
    assert_eq!(changes.recv().await.unwrap().config.revision, 1);
    assert_eq!(fixture.hub.refresh().await.unwrap().config.revision, 1);
}

#[tokio::test]
async fn an_update_is_saved_applied_and_told_to_subscribers() {
    let fixture = fixture();
    let mut changes = fixture.hub.subscribe();
    let mut config = GitConfig::empty();
    config.identity.name = "Ada".into();
    fixture.hub.update(config).await.unwrap();
    let told = changes.recv().await.unwrap();
    assert_eq!(told.config.revision, 1);
    assert_eq!(told.config.identity.name, "Ada");
    assert!(fixture.config_dir.path().join("git.json").is_file());
    assert!(told.git_available);
    assert_eq!(
        told.system_entries,
        [],
        "what we wrote is not listed as found"
    );
}

#[tokio::test]
async fn a_stale_or_invalid_update_changes_nothing() {
    let fixture = fixture();
    let mut changes = fixture.hub.subscribe();
    let mut first = GitConfig::empty();
    first.identity.name = "Ada".into();
    fixture.hub.update(first.clone()).await.unwrap();
    changes.recv().await.unwrap();

    let stale = fixture.hub.update(first).await.unwrap_err();
    assert_eq!(stale.error_code(), ErrorCode::StaleRevision);

    let mut shell = fixture.hub.refresh().await.unwrap().config;
    shell.aliases.push(alias("pwn", "!rm -rf ~", true));
    let refused = fixture.hub.update(shell).await.unwrap_err();
    assert_eq!(refused.error_code(), ErrorCode::InvalidConfig);
    assert!(changes.try_recv().is_err(), "nothing was told");
    assert!(!read(fixture.home.path(), ".config/hubergamof/gitconfig").contains("pwn"));
}

#[tokio::test]
async fn without_a_home_folder_the_configuration_is_saved_but_not_applied() {
    let config_dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(config_dir.path(), Arc::new(builtin_registry()));
    let hub = GitConfigHub::new(store, None);
    let mut config = GitConfig::empty();
    config.identity.name = "Ada".into();
    let error = hub.update(config).await.unwrap_err();
    assert!(matches!(error, GitError::NoHome));
    let snapshot = hub.refresh().await.unwrap();
    assert_eq!(snapshot.config.revision, 1);
    assert_eq!(snapshot.system_entries, []);
    assert!(!snapshot.git_available);
}

#[tokio::test]
async fn two_updates_of_the_same_revision_let_only_one_through() {
    let fixture = fixture();
    let mut config = GitConfig::empty();
    config.identity.name = "Ada".into();
    let (first, second) = tokio::join!(
        fixture.hub.update(config.clone()),
        fixture.hub.update(config)
    );
    let outcomes = [first, second];
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    let refused = outcomes.into_iter().find_map(Result::err).unwrap();
    assert_eq!(refused.error_code(), ErrorCode::StaleRevision);
}

#[tokio::test]
async fn a_slow_subscriber_still_gets_the_last_configuration() {
    let fixture = fixture();
    let mut changes = fixture.hub.subscribe();
    let mut config = GitConfig::empty();
    for round in 0..=CHANGE_BACKLOG {
        config
            .aliases
            .push(alias(&format!("a{round}"), "status", true));
        fixture.hub.update(config.clone()).await.unwrap();
        config = fixture.hub.refresh().await.unwrap().config;
    }
    let mut last = None;
    loop {
        match changes.try_recv() {
            Ok(snapshot) => last = Some(snapshot),
            Err(broadcast::error::TryRecvError::Lagged(_)) => {}
            Err(_) => break,
        }
    }
    assert_eq!(last.unwrap().config, config);
}

#[tokio::test]
async fn values_found_elsewhere_are_listed_for_the_page() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::write(
        home.join(".gitconfig"),
        "[user]\n\tname = Existing\n[alias]\n\tco = checkout\n\tdanger = !echo hi\n",
    )
    .unwrap();
    let snapshot = fixture.hub.refresh().await.unwrap();
    let listed: Vec<_> = snapshot
        .system_entries
        .iter()
        .map(|e| (e.key.as_str(), e.value.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            ("user.name", "Existing"),
            ("alias.co", "checkout"),
            ("alias.danger", "!echo hi")
        ]
    );
}
