//! The Git configuration written as the files Git reads: one holding the
//! global values, and one per identity by folder. Pure text functions.

use std::fmt::Write as _;

use hubergamof_protocol::{
    GIT_MANAGED_KEYS, GitConfig, GitCredentialHelper, GitPullMode, GitSigningFormat, git_key_parts,
    is_valid_git_setting_key, is_valid_git_setting_value,
};
use thiserror::Error;

/// Folder, relative to the home folder, of the files HuBergamof keeps.
pub(super) const MANAGED_DIR: &str = ".config/hubergamof";

/// The file holding the global values, in [`MANAGED_DIR`].
pub(super) const MAIN_FILE: &str = "gitconfig";

/// What the name of the file of an identity by folder starts with, in
/// [`MANAGED_DIR`]: followed by its number.
pub(super) const FOLDER_FILE_PREFIX: &str = "gitconfig-folder-";

/// Keys, in lower case, that HuBergamof writes besides the aliases: those of
/// the same keys found in other files are listed for the person.
pub(super) const MANAGED_KEYS: [&str; 10] = GIT_MANAGED_KEYS;

/// What the keys of the aliases start with, in lower case.
pub(super) const ALIAS_KEY_PREFIX: &str = "alias.";

const MANAGED_HEADER: &str = "# Managed by HuBergamof, do not edit: changes are overwritten.\n";

/// A value that cannot be written to a Git configuration file safely.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("the value of {key} cannot be written to a Git configuration file")]
pub struct UnwritableValue {
    /// The key it was for.
    pub key: String,
}

/// A file to write in [`MANAGED_DIR`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RenderedFile {
    pub name: String,
    pub content: String,
}

/// Everything the configuration puts on the computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Rendered {
    /// The file holding the global values, which includes the folder files.
    pub main: RenderedFile,
    /// One file per enabled identity by folder.
    pub folders: Vec<RenderedFile>,
}

/// A section of a Git configuration file: only written when it has a line.
struct Section {
    header: String,
    lines: Vec<String>,
}

impl Section {
    fn new(header: &str) -> Self {
        Self {
            header: header.to_owned(),
            lines: Vec::new(),
        }
    }

    /// Adds `key = "value"`; nothing when `value` is empty.
    fn text(&mut self, key: &str, value: &str) -> Result<(), UnwritableValue> {
        if !value.is_empty() {
            self.lines
                .push(format!("{key} = {}", quote(&self.header, key, value)?));
        }
        Ok(())
    }

    /// Adds `key = value` for a word Git reads as it is (`true`, `ssh`).
    fn word(&mut self, key: &str, word: &str) {
        self.lines.push(format!("{key} = {word}"));
    }

    fn write_to(&self, out: &mut String) {
        if self.lines.is_empty() {
            return;
        }
        let _ = writeln!(out, "[{}]", self.header);
        for line in &self.lines {
            let _ = writeln!(out, "\t{line}");
        }
    }
}

/// `value` between double quotes, `\` and `"` escaped: what Git reads back
/// as it was, `#` and `;` included.
fn quote(section: &str, key: &str, value: &str) -> Result<String, UnwritableValue> {
    if value.chars().any(char::is_control) {
        return Err(UnwritableValue {
            key: format!("{section}.{key}"),
        });
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for c in value.chars() {
        if matches!(c, '\\' | '"') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    Ok(quoted)
}

fn signing_format(format: GitSigningFormat) -> &'static str {
    match format {
        GitSigningFormat::OpenPgp => "openpgp",
        GitSigningFormat::Ssh => "ssh",
        GitSigningFormat::X509 => "x509",
    }
}

fn user_section(name: &str, email: &str, signing_key: &str) -> Result<Section, UnwritableValue> {
    let mut user = Section::new("user");
    user.text("name", name)?;
    user.text("email", email)?;
    user.text("signingkey", signing_key)?;
    Ok(user)
}

/// The sections of the global values, in the order Git is told them.
fn global_sections(config: &GitConfig) -> Result<Vec<Section>, UnwritableValue> {
    let identity = &config.identity;
    let signing = &config.signing;
    let settings = &config.settings;

    let signing_key = if signing.enabled { &signing.key } else { "" };
    let user = user_section(&identity.name, &identity.email, signing_key)?;

    let mut commit = Section::new("commit");
    let mut gpg = Section::new("gpg");
    if signing.enabled {
        commit.word("gpgsign", "true");
        gpg.word("format", signing_format(signing.format));
    }

    let mut init = Section::new("init");
    init.text("defaultBranch", &settings.default_branch)?;

    let mut pull = Section::new("pull");
    match settings.pull {
        GitPullMode::Default => {}
        GitPullMode::Merge => pull.word("rebase", "false"),
        GitPullMode::Rebase => pull.word("rebase", "true"),
        GitPullMode::FastForwardOnly => pull.word("ff", "only"),
    }

    let mut core = Section::new("core");
    core.text("editor", &settings.editor)?;

    let mut credential = Section::new("credential");
    match settings.credential_helper {
        GitCredentialHelper::None => {}
        GitCredentialHelper::Cache { timeout_seconds } => {
            credential.text("helper", &format!("cache --timeout={timeout_seconds}"))?;
        }
        GitCredentialHelper::Store => credential.text("helper", "store")?,
        GitCredentialHelper::Libsecret => credential.text("helper", "libsecret")?,
        GitCredentialHelper::Manager => credential.text("helper", "manager")?,
    }

    let mut aliases = Section::new("alias");
    for alias in config.aliases.iter().filter(|alias| alias.enabled) {
        aliases.text(&alias.name, &alias.command)?;
    }

    let mut sections = vec![user, commit, gpg, init, pull, core, credential, aliases];
    sections.extend(extra_sections(config)?);
    Ok(sections)
}

/// One section per setting the person wrote, in their order. A section name
/// is written bare and a sub-section between quotes: both are checked again
/// here, the file being read by Git.
fn extra_sections(config: &GitConfig) -> Result<Vec<Section>, UnwritableValue> {
    let mut sections = Vec::new();
    for setting in config
        .extra_settings
        .iter()
        .filter(|setting| setting.enabled)
    {
        let writable =
            is_valid_git_setting_key(&setting.key) && is_valid_git_setting_value(&setting.value);
        let Some((section, subsection, name)) = git_key_parts(&setting.key).filter(|_| writable)
        else {
            return Err(UnwritableValue {
                key: setting.key.clone(),
            });
        };
        let header = match subsection {
            Some(subsection) => format!("{section} \"{subsection}\""),
            None => section.to_owned(),
        };
        let mut extra = Section::new(&header);
        extra.text(name, &setting.value)?;
        sections.push(extra);
    }
    Ok(sections)
}

fn file_text(sections: &[Section]) -> String {
    let mut out = String::from(MANAGED_HEADER);
    for section in sections {
        section.write_to(&mut out);
    }
    out
}

/// The files the configuration puts on the computer; `None` when it puts
/// nothing (the files and the include of the user's configuration go away).
///
/// A folder identity comes after the global values, so that it wins there.
///
/// # Errors
/// [`UnwritableValue`] for a value holding a control character: a validated
/// configuration never does.
pub(super) fn render(config: &GitConfig) -> Result<Option<Rendered>, UnwritableValue> {
    let mut sections = global_sections(config)?;
    let mut folders = Vec::new();
    for (index, rule) in config
        .folder_identities
        .iter()
        .filter(|rule| rule.enabled)
        .enumerate()
    {
        let name = format!("{FOLDER_FILE_PREFIX}{}", index + 1);
        let user = user_section(&rule.name, &rule.email, &rule.signing_key)?;
        folders.push(RenderedFile {
            name: name.clone(),
            content: file_text(&[user]),
        });
        // Written as a sub-section name, between quotes and unescaped: the
        // validation has no `"` nor `\` in a folder, which is checked again here.
        if rule
            .folder
            .chars()
            .any(|c| matches!(c, '"' | '\\') || c.is_control())
        {
            return Err(UnwritableValue {
                key: "includeIf".to_owned(),
            });
        }
        let mut include = Section::new(&format!("includeIf \"gitdir:{}/\"", rule.folder));
        include.text("path", &format!("~/{MANAGED_DIR}/{name}"))?;
        sections.push(include);
    }
    if sections.iter().all(|section| section.lines.is_empty()) {
        return Ok(None);
    }
    Ok(Some(Rendered {
        main: RenderedFile {
            name: MAIN_FILE.to_owned(),
            content: file_text(&sections),
        },
        folders,
    }))
}

#[cfg(test)]
mod tests {
    use hubergamof_protocol::{GitAlias, GitFolderIdentity};

    use super::*;

    fn folder_rule(folder: &str) -> GitFolderIdentity {
        GitFolderIdentity {
            folder: folder.into(),
            name: String::new(),
            email: "a@b.c".into(),
            signing_key: String::new(),
            enabled: true,
        }
    }

    fn rendered(config: &GitConfig) -> Rendered {
        render(config).unwrap().unwrap()
    }

    /// The keys the listing of what is found elsewhere looks for must be the
    /// ones that are written: a key added to the rendering and not to the
    /// list would silently never be shown.
    #[test]
    fn every_key_written_is_a_managed_key() {
        let mut config = GitConfig::empty();
        config.identity.name = "Ada".into();
        config.identity.email = "ada@example.org".into();
        config.signing.enabled = true;
        config.signing.key = "k".into();
        config.settings.default_branch = "main".into();
        config.settings.pull = GitPullMode::Rebase;
        config.settings.editor = "vim".into();
        config.settings.credential_helper = GitCredentialHelper::Store;
        config.aliases = vec![GitAlias {
            name: "co".into(),
            command: "checkout".into(),
            enabled: true,
        }];
        let mut content = rendered(&config).main.content;
        config.settings.pull = GitPullMode::FastForwardOnly;
        content.push_str(&rendered(&config).main.content);
        let mut written = Vec::new();
        let mut section = String::new();
        for line in content.lines().filter(|line| !line.starts_with('#')) {
            if let Some(header) = line.strip_prefix('[') {
                section = header.trim_end_matches(']').to_ascii_lowercase();
            } else if let Some((key, _)) = line.trim().split_once(" = ") {
                written.push(format!("{section}.{}", key.to_ascii_lowercase()));
            }
        }
        assert!(written.len() >= MANAGED_KEYS.len());
        for key in &written {
            assert!(
                MANAGED_KEYS.contains(&key.as_str()) || key.starts_with(ALIAS_KEY_PREFIX),
                "{key} is written but not listed as managed"
            );
        }
        for key in MANAGED_KEYS {
            assert!(written.iter().any(|w| w == key), "{key} is never written");
        }
    }

    #[test]
    fn a_folder_that_would_break_out_of_its_section_name_is_refused() {
        for folder in ["~/a\"b", "~/a\\b", "~/a\nb"] {
            let mut config = GitConfig::empty();
            config.folder_identities = vec![folder_rule(folder)];
            assert!(render(&config).is_err(), "{folder:?}");
        }
    }

    #[test]
    fn a_configuration_that_sets_nothing_writes_nothing() {
        assert_eq!(render(&GitConfig::empty()), Ok(None));
    }

    #[test]
    fn identity_and_settings_become_sections() {
        let mut config = GitConfig::empty();
        config.identity.name = "Ada Lovelace".into();
        config.identity.email = "ada@example.org".into();
        config.settings.default_branch = "main".into();
        config.settings.pull = GitPullMode::Rebase;
        config.settings.editor = "code --wait".into();
        config.settings.credential_helper = GitCredentialHelper::Cache {
            timeout_seconds: 900,
        };
        assert_eq!(
            rendered(&config).main.content,
            format!(
                "{MANAGED_HEADER}\
                 [user]\n\tname = \"Ada Lovelace\"\n\temail = \"ada@example.org\"\n\
                 [init]\n\tdefaultBranch = \"main\"\n\
                 [pull]\n\trebase = true\n\
                 [core]\n\teditor = \"code --wait\"\n\
                 [credential]\n\thelper = \"cache --timeout=900\"\n"
            )
        );
    }

    #[test]
    fn pull_modes_and_helpers_map_to_their_keys() {
        for (mode, line) in [
            (GitPullMode::Merge, "rebase = false"),
            (GitPullMode::Rebase, "rebase = true"),
            (GitPullMode::FastForwardOnly, "ff = only"),
        ] {
            let mut config = GitConfig::empty();
            config.settings.pull = mode;
            assert!(rendered(&config).main.content.contains(line), "{line}");
        }
        for (helper, line) in [
            (GitCredentialHelper::Store, "helper = \"store\""),
            (GitCredentialHelper::Libsecret, "helper = \"libsecret\""),
            (GitCredentialHelper::Manager, "helper = \"manager\""),
        ] {
            let mut config = GitConfig::empty();
            config.settings.credential_helper = helper;
            assert!(rendered(&config).main.content.contains(line), "{line}");
        }
    }

    #[test]
    fn signing_is_written_only_when_enabled() {
        let mut config = GitConfig::empty();
        config.signing.format = GitSigningFormat::Ssh;
        config.signing.key = "~/.ssh/id_ed25519.pub".into();
        assert_eq!(render(&config), Ok(None));

        config.signing.enabled = true;
        let content = rendered(&config).main.content;
        assert!(content.contains("signingkey = \"~/.ssh/id_ed25519.pub\""));
        assert!(content.contains("[commit]\n\tgpgsign = true"));
        assert!(content.contains("[gpg]\n\tformat = ssh"));
    }

    #[test]
    fn only_enabled_aliases_are_written() {
        let mut config = GitConfig::empty();
        config.aliases = vec![
            GitAlias {
                name: "co".into(),
                command: "checkout".into(),
                enabled: true,
            },
            GitAlias {
                name: "st".into(),
                command: "status -s".into(),
                enabled: false,
            },
        ];
        let content = rendered(&config).main.content;
        assert!(content.contains("[alias]\n\tco = \"checkout\"\n"));
        assert!(!content.contains("st = "));
    }

    #[test]
    fn values_are_quoted_so_that_git_reads_them_back_as_they_are() {
        let mut config = GitConfig::empty();
        config.identity.name = r#"A "B" \ C # D ; E"#.into();
        assert!(
            rendered(&config)
                .main
                .content
                .contains(r#"name = "A \"B\" \\ C # D ; E""#)
        );
    }

    #[test]
    fn a_control_character_is_refused_whatever_validation_said() {
        let mut config = GitConfig::empty();
        config.identity.name = "Ada\nname = Mallory".into();
        let error = render(&config).unwrap_err();
        assert_eq!(error.key, "user.name");
    }

    #[test]
    fn a_folder_identity_gets_its_file_and_an_include_after_the_global_values() {
        let mut config = GitConfig::empty();
        config.identity.email = "ada@home.org".into();
        config.folder_identities = vec![
            GitFolderIdentity {
                folder: "~/work".into(),
                name: "Ada".into(),
                email: "ada@work.org".into(),
                signing_key: "ABCD".into(),
                enabled: true,
            },
            GitFolderIdentity {
                folder: "~/off".into(),
                name: String::new(),
                email: "x@y.z".into(),
                signing_key: String::new(),
                enabled: false,
            },
            GitFolderIdentity {
                folder: "~/my projects".into(),
                name: String::new(),
                email: "p@q.r".into(),
                signing_key: String::new(),
                enabled: true,
            },
        ];
        let rendered = rendered(&config);
        let main = rendered.main.content;
        assert!(
            main.find("[user]").unwrap() < main.find("[includeIf").unwrap(),
            "the global identity comes first so that a folder wins"
        );
        assert!(main.contains(
            "[includeIf \"gitdir:~/work/\"]\n\tpath = \"~/.config/hubergamof/gitconfig-folder-1\""
        ));
        assert!(main.contains("[includeIf \"gitdir:~/my projects/\"]"));
        assert!(!main.contains("~/off"), "a disabled rule is not written");
        assert_eq!(
            rendered
                .folders
                .iter()
                .map(|file| file.name.as_str())
                .collect::<Vec<_>>(),
            ["gitconfig-folder-1", "gitconfig-folder-2"]
        );
        assert_eq!(
            rendered.folders[0].content,
            format!(
                "{MANAGED_HEADER}[user]\n\tname = \"Ada\"\n\temail = \"ada@work.org\"\n\
                 \tsigningkey = \"ABCD\"\n"
            )
        );
    }

    #[test]
    fn only_folder_identities_are_enough_to_write_a_file() {
        let mut config = GitConfig::empty();
        config.folder_identities = vec![GitFolderIdentity {
            folder: "~/work".into(),
            name: String::new(),
            email: "ada@work.org".into(),
            signing_key: String::new(),
            enabled: true,
        }];
        assert_eq!(rendered(&config).folders.len(), 1);
    }
}
