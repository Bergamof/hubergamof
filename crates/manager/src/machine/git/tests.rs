//! The Git hub and the files it writes, checked against the real `git`: what
//! is written must be read back by Git as it was meant.

use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::Command;

use hubergamof_protocol::{
    AdvancedSetting, GitAlias, GitCredentialHelper, GitFolderIdentity, GitPullMode,
    GitSigningFormat,
};

use super::*;
use crate::testing::builtin_registry;

struct Fixture {
    config_dir: tempfile::TempDir,
    home: tempfile::TempDir,
    hub: GitConfigHub,
}

fn fixture() -> Fixture {
    require_git();
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

/// Git must be installed to run these tests: they use it as the judge of what is written.
fn require_git() {
    let found = Command::new("git").arg("--version").output();
    assert!(
        found.is_ok_and(|output| output.status.success()),
        "the git program is needed to run these tests"
    );
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
        // A repository above the temporary folder must not be read.
        .env("GIT_CEILING_DIRECTORIES", home.parent().unwrap_or(home))
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
async fn the_demo_works_in_a_temporary_home_and_never_touches_the_real_one() {
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
    let demo_home = hub.target().unwrap().home().to_owned();
    assert!(demo_home.starts_with(std::env::temp_dir()));
    assert_ne!(Some(demo_home.clone()), std::env::home_dir());
    assert!(read(&demo_home, ".config/hubergamof/gitconfig").contains("Grace Hopper"));
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
    assert!(matches!(error, GitError::Apply(_)), "{error}");
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
        matches!(error, GitError::Apply(GitFilesError::LinkedFolder(_))),
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
    assert!(matches!(error, GitError::Apply(_)), "{error}");
    assert_eq!(error.error_code(), hubergamof_protocol::ErrorCode::Internal);
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
    assert!(snapshot.git_available, "no home is not a missing git");
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

#[tokio::test]
async fn with_both_git_files_the_block_goes_in_dotgitconfig_and_the_other_stays_as_it_was() {
    let fixture = fixture();
    let home = fixture.home.path();
    let xdg = "[core]\n\tpager = less\n";
    fs::create_dir_all(home.join(".config/git")).unwrap();
    fs::write(home.join(".config/git/config"), xdg).unwrap();
    fs::write(home.join(".gitconfig"), "[user]\n\tname = Before\n").unwrap();

    saved(&fixture, |config| config.identity.name = "After".into()).await;
    assert!(read(home, ".gitconfig").contains("hubergamof git"));
    assert_eq!(read(home, ".config/git/config"), xdg);
    assert_eq!(git_get(home, home, "user.name").as_deref(), Some("After"));
}

#[tokio::test]
async fn the_block_moves_when_the_file_that_gets_it_changes_and_never_stays_twice() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::create_dir_all(home.join(".config/git")).unwrap();
    fs::write(home.join(".config/git/config"), "[core]\n\tpager = less\n").unwrap();
    saved(&fixture, |config| config.identity.name = "Ada".into()).await;
    assert!(read(home, ".config/git/config").contains("hubergamof git"));

    // The person makes a ~/.gitconfig: it comes first now.
    fs::write(home.join(".gitconfig"), "[user]\n\temail = a@b.c\n").unwrap();
    saved(&fixture, |config| config.identity.name = "Grace".into()).await;
    let blocks = |relative: &str| read(home, relative).matches("hubergamof git >>>").count();
    assert_eq!(blocks(".gitconfig"), 1);
    assert_eq!(blocks(".config/git/config"), 0);
    assert_eq!(read(home, ".config/git/config"), "[core]\n\tpager = less\n");

    // Switched off with a block in each file: both are cleaned.
    let block = "# >>> hubergamof git >>>\n[include]\n\tpath = x\n# <<< hubergamof git <<<\n";
    fs::write(home.join(".config/git/config"), format!("[core]\n{block}")).unwrap();
    saved(&fixture, |config| config.identity.name.clear()).await;
    assert_eq!(blocks(".gitconfig"), 0);
    assert_eq!(blocks(".config/git/config"), 0);
    assert_eq!(read(home, ".gitconfig"), "[user]\n\temail = a@b.c\n");
}

#[tokio::test]
async fn an_empty_gitconfig_of_the_user_is_never_removed() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::write(home.join(".gitconfig"), "").unwrap();

    // Nothing to write: the file was never ours.
    saved(&fixture, |_| ()).await;
    assert!(home.join(".gitconfig").is_file());

    // Written then switched off: the block goes. An empty file and no file
    // are the same to Git, so the file may go with it, but never keeps a block.
    saved(&fixture, |config| config.identity.name = "Ada".into()).await;
    saved(&fixture, |config| config.identity.name.clear()).await;
    let left = fs::read_to_string(home.join(".gitconfig")).unwrap_or_default();
    assert_eq!(left.trim(), "");
}

/// Why a file of the user cannot be rewritten, and what is left then.
#[tokio::test]
async fn a_gitconfig_that_cannot_be_rewritten_is_left_untouched_and_the_error_says_why() {
    type Prepare = fn(&Path);
    let cases: [(&str, Prepare); 3] = [
        ("not UTF-8", |home| {
            fs::write(home.join(".gitconfig"), [0xff, 0xfe, b'\n']).unwrap();
        }),
        ("hard link", |home| {
            fs::write(home.join("other"), "[core]\n").unwrap();
            fs::hard_link(home.join("other"), home.join(".gitconfig")).unwrap();
        }),
        ("too large", |home| {
            let big = "#".repeat(usize::try_from(crate::machine::MAX_TEXT_FILE_BYTES).unwrap() + 1);
            fs::write(home.join(".gitconfig"), big).unwrap();
        }),
    ];
    for (what, prepare) in cases {
        let fixture = fixture();
        let home = fixture.home.path();
        prepare(home);
        let before = fs::read(home.join(".gitconfig")).unwrap();
        let mut changes = fixture.hub.subscribe();
        let mut config = fixture.hub.refresh().await.unwrap().config;
        config.identity.name = "Ada".into();

        let error = fixture.hub.update(config).await.unwrap_err();
        assert!(
            matches!(error, GitError::Apply(GitFilesError::File(_))),
            "{what}: {error}"
        );
        assert_eq!(fs::read(home.join(".gitconfig")).unwrap(), before, "{what}");
        assert_eq!(changes.recv().await.unwrap().config.revision, 1, "{what}");
    }
}

#[tokio::test]
async fn switching_off_with_an_unreadable_gitconfig_keeps_the_files_the_include_may_still_load() {
    let fixture = fixture();
    let home = fixture.home.path();
    saved(&fixture, |config| config.identity.name = "Ada".into()).await;
    let with_block = fs::read(home.join(".gitconfig")).unwrap();
    fs::write(home.join(".gitconfig"), [0xff, 0xfe]).unwrap();

    let mut config = fixture.hub.refresh().await.unwrap().config;
    config.identity.name.clear();
    assert!(fixture.hub.update(config).await.is_err());
    assert!(
        home.join(".config/hubergamof/gitconfig").is_file(),
        "no include is left pointing at a removed file"
    );
    drop(with_block);
}

#[tokio::test]
async fn the_folder_of_hubergamof_is_tightened_and_a_file_in_its_place_is_refused() {
    let fixture = fixture();
    let home = fixture.home.path();
    let dir = home.join(".config/hubergamof");
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    saved(&fixture, |config| config.identity.name = "Ada".into()).await;
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );

    let other = fixture_without_folder();
    let mut config = other.hub.refresh().await.unwrap().config;
    config.identity.name = "Ada".into();
    fs::create_dir_all(other.home.path().join(".config")).unwrap();
    fs::write(other.home.path().join(".config/hubergamof"), "x").unwrap();
    let error = other.hub.update(config).await.unwrap_err();
    assert!(
        matches!(error, GitError::Apply(GitFilesError::File(_))),
        "{error}"
    );
}

fn fixture_without_folder() -> Fixture {
    fixture()
}

#[tokio::test]
async fn saving_twice_leaves_one_block_and_an_identical_update_rewrites_nothing() {
    let fixture = fixture();
    let home = fixture.home.path();
    saved(&fixture, |config| config.identity.name = "Ada".into()).await;
    saved(&fixture, |config| config.identity.email = "a@b.c".into()).await;
    assert_eq!(
        read(home, ".gitconfig")
            .matches("hubergamof git >>>")
            .count(),
        1
    );
}

#[tokio::test]
async fn folder_rules_follow_the_boundaries_git_gives_them() {
    let fixture = fixture();
    let home = fixture.home.path();
    let mut off = folder_rule("~/off", "Off", "off@x.org");
    off.enabled = false;
    let mut key_only = folder_rule("~/signed", "", "");
    key_only.signing_key = "KEY1".into();
    saved(&fixture, |config| {
        config.identity.email = "global@x.org".into();
        config.folder_identities = vec![
            folder_rule("~/work", "Work", "work@x.org"),
            folder_rule("~/work/acme", "Acme", "acme@x.org"),
            folder_rule("~/my projects", "Spaced", "spaced@x.org"),
            off,
            key_only,
        ];
    })
    .await;

    let repository = |relative: &str| {
        let path = home.join(relative);
        fs::create_dir_all(&path).unwrap();
        let status = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&path)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(status.success());
        path
    };
    let email = |path: &Path| git_get(home, path, "user.email");
    assert_eq!(email(&repository("work/a")).as_deref(), Some("work@x.org"));
    assert_eq!(
        email(&repository("workshop/a")).as_deref(),
        Some("global@x.org")
    );
    assert_eq!(
        email(&repository("work/acme/a")).as_deref(),
        Some("acme@x.org"),
        "the last rule wins"
    );
    assert_eq!(
        email(&repository("my projects/a")).as_deref(),
        Some("spaced@x.org")
    );
    assert_eq!(email(&repository("off/a")).as_deref(), Some("global@x.org"));
    let signed = repository("signed/a");
    assert_eq!(
        git_get(home, &signed, "user.signingkey").as_deref(),
        Some("KEY1")
    );
    assert_eq!(email(&signed).as_deref(), Some("global@x.org"));
}

#[tokio::test]
async fn git_reads_back_every_escape_and_both_pull_modes() {
    let fixture = fixture();
    let home = fixture.home.path();
    saved(&fixture, |config| {
        config.identity.name = r"Zoë \ ; # \ end".into();
        config.settings.pull = GitPullMode::Merge;
    })
    .await;
    assert_eq!(
        git_get(home, home, "user.name").as_deref(),
        Some(r"Zoë \ ; # \ end")
    );
    assert_eq!(git_get(home, home, "pull.rebase").as_deref(), Some("false"));
    saved(&fixture, |config| {
        config.settings.pull = GitPullMode::Rebase;
    })
    .await;
    assert_eq!(git_get(home, home, "pull.rebase").as_deref(), Some("true"));
}

#[tokio::test]
async fn a_folder_that_is_or_goes_through_a_link_is_refused_and_a_rule_that_is_off_is_not_checked()
{
    let elsewhere = tempfile::tempdir().unwrap();
    for (rule_folder, link_at, enabled) in
        [("~/work", "work", true), ("~/deep/er/work", "deep", true)]
    {
        let fixture = fixture();
        let home = fixture.home.path();
        symlink(elsewhere.path(), home.join(link_at)).unwrap();
        let mut rule = folder_rule(rule_folder, "A", "a@x.org");
        rule.enabled = enabled;
        let mut config = fixture.hub.refresh().await.unwrap().config;
        config.folder_identities = vec![rule];
        let error = fixture.hub.update(config).await.unwrap_err();
        assert!(
            matches!(error, GitError::Apply(GitFilesError::LinkedFolder(_))),
            "{rule_folder} {enabled}: {error}"
        );
    }

    // Off, it writes nothing: it may stay, so that it can be switched on later
    // or removed.
    let fixture = fixture();
    symlink(elsewhere.path(), fixture.home.path().join("work")).unwrap();
    let mut off = folder_rule("~/work", "A", "a@x.org");
    off.enabled = false;
    saved(&fixture, |config| config.folder_identities = vec![off]).await;
}

#[tokio::test]
async fn an_absent_git_is_told_and_nothing_else_fails() {
    let fixture = fixture();
    let mut files = GitFiles::new(fixture.home.path().to_owned(), false);
    files.program = "/nonexistent/git".into();
    let read = files.read_system().await;
    assert_eq!(
        read,
        SystemRead {
            entries: Vec::new(),
            git_available: false
        }
    );

    let store = ConfigStore::new(fixture.config_dir.path(), Arc::new(builtin_registry()));
    let hub = GitConfigHub::new(store, Some(files));
    let snapshot = hub.refresh().await.unwrap();
    assert!(!snapshot.git_available);
}

#[tokio::test(start_paused = true)]
async fn a_git_that_takes_too_long_is_given_up_and_killed() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("slow-git");
    fs::write(&script, "#!/bin/sh\nexec sleep 3600\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let mut files = GitFiles::new(dir.path().to_owned(), false);
    files.program = script.into_os_string();
    let read = files.read_system().await;
    assert!(!read.git_available);
}

#[tokio::test]
async fn a_huge_configuration_is_listed_within_its_limit() {
    let fixture = fixture();
    let home = fixture.home.path();
    let mut text = String::from("[alias]\n");
    for index in 0..(MAX_SYSTEM_GIT_ENTRIES + 100) {
        writeln!(text, "\ta{index} = status").unwrap();
    }
    fs::write(home.join(".gitconfig"), text).unwrap();
    let snapshot = fixture.hub.refresh().await.unwrap();
    assert_eq!(snapshot.system_entries.len(), MAX_SYSTEM_GIT_ENTRIES);
}

#[tokio::test]
async fn a_circular_include_is_stopped_by_git_itself() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::write(
        home.join(".gitconfig"),
        "[include]\n\tpath = ~/.gitconfig\n[user]\n\tname = Loop\n",
    )
    .unwrap();
    let snapshot = fixture.hub.refresh().await.unwrap();
    assert!(snapshot.git_available);
    assert!(
        snapshot.system_entries.is_empty()
            || snapshot.system_entries.len() <= MAX_SYSTEM_GIT_ENTRIES
    );
}

#[tokio::test]
async fn an_invalid_saved_configuration_takes_what_it_had_applied_with_it() {
    let fixture = fixture();
    let home = fixture.home.path();
    saved(&fixture, |config| config.identity.name = "Ada".into()).await;
    assert!(home.join(".config/hubergamof/gitconfig").is_file());

    // The saved file is damaged (or comes from a newer version).
    let path = fixture.config_dir.path().join("git.json");
    fs::write(&path, "{ not json").unwrap();
    let snapshot = fixture.hub.refresh().await.unwrap();
    assert_eq!(snapshot.config, GitConfig::empty());
    assert!(!home.join(".config/hubergamof/gitconfig").exists());
    assert!(
        !home.join(".gitconfig").exists(),
        "the file only held our block"
    );
}

const ORIGINAL_ALIASES: &str = "[user]\n\tname = Ada\n[alias]\n\tco = checkout\n\tst = status -s\n";

#[tokio::test]
async fn an_alias_taken_over_is_commented_out_where_it_came_from_and_really_switches_off() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::write(home.join(".gitconfig"), ORIGINAL_ALIASES).unwrap();

    // Taken over, on: Git reads it from the file of HuBergamof, the original
    // line is a comment, the other alias is as it was.
    saved(&fixture, |config| {
        config.aliases = vec![alias("co", "checkout --quiet", true)];
    })
    .await;
    let content = read(home, ".gitconfig");
    assert!(content.contains("#hubergamof-off# \tco = checkout\n"));
    assert!(content.contains("\n\tst = status -s\n"));
    assert_eq!(
        git_get(home, home, "alias.co").as_deref(),
        Some("checkout --quiet")
    );
    assert_eq!(
        git_get(home, home, "alias.st").as_deref(),
        Some("status -s")
    );

    // Off: nothing defines it any more, which is the point.
    saved(&fixture, |config| config.aliases[0].enabled = false).await;
    assert_eq!(git_get(home, home, "alias.co"), None);
    assert_eq!(
        git_get(home, home, "alias.st").as_deref(),
        Some("status -s")
    );

    // Back on, then removed from the configuration: the original comes back.
    saved(&fixture, |config| config.aliases[0].enabled = true).await;
    assert_eq!(
        git_get(home, home, "alias.co").as_deref(),
        Some("checkout --quiet")
    );
    saved(&fixture, |config| config.aliases.clear()).await;
    assert_eq!(read(home, ".gitconfig"), ORIGINAL_ALIASES);
    assert_eq!(git_get(home, home, "alias.co").as_deref(), Some("checkout"));
}

#[tokio::test]
async fn an_alias_that_is_off_from_the_start_is_commented_out_with_nothing_else_written() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::write(home.join(".gitconfig"), ORIGINAL_ALIASES).unwrap();

    saved(&fixture, |config| {
        config.aliases = vec![alias("co", "checkout", false)];
    })
    .await;
    assert_eq!(git_get(home, home, "alias.co"), None);
    assert!(!home.join(".config/hubergamof/gitconfig").exists());
    assert!(
        !read(home, ".gitconfig").contains("hubergamof git"),
        "no include for nothing"
    );

    saved(&fixture, |config| config.aliases.clear()).await;
    assert_eq!(read(home, ".gitconfig"), ORIGINAL_ALIASES);
}

#[tokio::test]
async fn the_xdg_configuration_and_the_gitconfig_are_both_claimed() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::create_dir_all(home.join(".config/git")).unwrap();
    fs::write(
        home.join(".config/git/config"),
        "[alias]\n\tco = from-xdg\n",
    )
    .unwrap();
    fs::write(home.join(".gitconfig"), "[alias]\n\tco = from-home\n").unwrap();

    saved(&fixture, |config| {
        config.aliases = vec![alias("co", "mine", false)];
    })
    .await;
    assert_eq!(git_get(home, home, "alias.co"), None);
    saved(&fixture, |config| config.aliases.clear()).await;
    assert_eq!(read(home, ".gitconfig"), "[alias]\n\tco = from-home\n");
    assert_eq!(
        read(home, ".config/git/config"),
        "[alias]\n\tco = from-xdg\n"
    );
}

#[tokio::test]
async fn a_gitconfig_that_cannot_be_rewritten_leaves_its_aliases_alone_and_the_error_says_so() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::write(home.join(".gitconfig"), [0xff, 0xfe]).unwrap();
    let mut config = fixture.hub.refresh().await.unwrap().config;
    config.aliases = vec![alias("co", "mine", false)];
    let error = fixture.hub.update(config).await.unwrap_err();
    assert!(
        matches!(error, GitError::Apply(GitFilesError::File(_))),
        "{error}"
    );
    assert_eq!(fs::read(home.join(".gitconfig")).unwrap(), [0xff, 0xfe]);
}

#[tokio::test]
async fn an_invalid_saved_configuration_gives_the_aliases_back_too() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::write(home.join(".gitconfig"), ORIGINAL_ALIASES).unwrap();
    saved(&fixture, |config| {
        config.aliases = vec![alias("co", "mine", true)];
    })
    .await;
    assert!(read(home, ".gitconfig").contains("#hubergamof-off# "));

    fs::write(fixture.config_dir.path().join("git.json"), "{ broken").unwrap();
    fixture.hub.refresh().await.unwrap();
    assert_eq!(read(home, ".gitconfig"), ORIGINAL_ALIASES);
}

#[tokio::test]
async fn an_alias_found_in_an_included_file_is_listed_and_left_where_it_is() {
    let fixture = fixture();
    let home = fixture.home.path();
    fs::write(home.join("aliases.inc"), "[alias]\n\tco = checkout\n").unwrap();
    fs::write(
        home.join(".gitconfig"),
        "[include]\n\tpath = ~/aliases.inc\n",
    )
    .unwrap();
    let snapshot = fixture.hub.refresh().await.unwrap();
    let found = snapshot
        .system_entries
        .iter()
        .find(|e| e.key == "alias.co")
        .unwrap();
    assert_eq!(found.origin, "~/aliases.inc");

    saved(&fixture, |config| {
        config.aliases = vec![alias("co", "mine", false)];
    })
    .await;
    assert_eq!(read(home, "aliases.inc"), "[alias]\n\tco = checkout\n");
}

fn setting(key: &str, value: &str, enabled: bool) -> AdvancedSetting {
    AdvancedSetting {
        key: key.into(),
        value: value.into(),
        enabled,
    }
}

#[tokio::test]
async fn git_reads_back_the_settings_the_person_wrote() {
    let fixture = fixture();
    let home = fixture.home.path();
    saved(&fixture, |config| {
        config.extra_settings = vec![
            setting("push.default", "current", true),
            setting("core.autocrlf", "input", false),
            setting("url.https://example.org/.insteadOf", "ex:", true),
            setting("diff.tool", "say \"hi\" ; # not a comment \\", true),
        ];
    })
    .await;

    assert_eq!(
        git_get(home, home, "push.default").as_deref(),
        Some("current")
    );
    assert_eq!(
        git_get(home, home, "core.autocrlf"),
        None,
        "off is not written"
    );
    assert_eq!(
        git_get(home, home, "url.https://example.org/.insteadof").as_deref(),
        Some("ex:")
    );
    assert_eq!(
        git_get(home, home, "diff.tool").as_deref(),
        Some("say \"hi\" ; # not a comment \\")
    );
}

#[tokio::test]
async fn settings_alone_make_a_file_and_leave_with_the_last_one() {
    let fixture = fixture();
    let home = fixture.home.path();
    saved(&fixture, |config| {
        config.extra_settings = vec![setting("push.default", "current", true)];
    })
    .await;
    assert!(read(home, ".gitconfig").contains("hubergamof"));
    saved(&fixture, |config| config.extra_settings.clear()).await;
    assert!(!home.join(".gitconfig").exists());
    assert!(!home.join(".config/hubergamof/gitconfig").exists());
}

#[tokio::test]
async fn a_setting_cannot_be_an_alias_a_managed_key_or_an_include() {
    for key in [
        "alias.x",
        "user.email",
        "include.path",
        "includeIf.gitdir:~/x/.path",
    ] {
        let fixture = fixture();
        let mut config = fixture.hub.refresh().await.unwrap().config;
        config.extra_settings = vec![setting(key, "!rm -rf ~", true)];
        let error = fixture.hub.update(config).await.unwrap_err();
        assert!(matches!(error, GitError::Store(_)), "{key}: {error}");
    }
}
