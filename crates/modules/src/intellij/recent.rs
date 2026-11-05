//! The recent projects of IntelliJ, from its `recentProjects.xml`.
//!
//! The file is IntelliJ's, not the user's: it is read with a parser that
//! refuses any DTD (no entities), within [`MAX_FILE_BYTES`], and only paths
//! IntelliJ writes are kept: absolute, or starting with `$USER_HOME$`; any
//! other macro is skipped.

use std::path::{Component, Path, PathBuf};

use roxmltree::{Document, Node};

use super::catalog::project_name;

/// Largest file read: far above the few kilobytes IntelliJ writes.
pub(super) const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// Most projects kept, the most recent ones.
const MAX_PROJECTS: usize = 100;

/// Macro IntelliJ writes for the home directory.
const USER_HOME_MACRO: &str = "$USER_HOME$";

/// Starts every IntelliJ macro.
const MACRO_MARK: char = '$';

/// A project of the file, before its folder is checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecentEntry {
    /// Name shown in its window titles.
    pub(super) name: String,
    /// Its folder, absolute, as written.
    pub(super) path: PathBuf,
    /// When it was last activated (milliseconds since the epoch, 0 when
    /// unknown).
    pub(super) activated: u64,
}

/// The projects of `xml`, most recently activated first, at most
/// [`MAX_PROJECTS`]; `$USER_HOME$` stands for `home`.
///
/// # Errors
///
/// A description of why the file cannot be read (not XML, DTD…).
pub(super) fn parse(xml: &str, home: &Path) -> Result<Vec<RecentEntry>, String> {
    let document = Document::parse(xml).map_err(|error| error.to_string())?;
    let mut entries: Vec<RecentEntry> = document
        .descendants()
        .filter(|node| node.has_tag_name("entry"))
        .filter_map(|entry| read_entry(entry, home))
        .collect();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.activated));
    entries.truncate(MAX_PROJECTS);
    Ok(entries)
}

fn read_entry(entry: Node<'_, '_>, home: &Path) -> Option<RecentEntry> {
    let path = expand(entry.attribute("key")?, home)?;
    let info = entry
        .descendants()
        .find(|node| node.has_tag_name("RecentProjectMetaInfo"))?;
    let activated = info
        .descendants()
        .find(|node| {
            node.has_tag_name("option") && node.attribute("name") == Some("activationTimestamp")
        })
        .and_then(|option| option.attribute("value"))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let from_title = info
        .attribute("frameTitle")
        .map(project_name)
        .filter(|name| !name.is_empty());
    let name = match from_title {
        Some(name) => name.to_owned(),
        None => path.file_name()?.to_str()?.to_owned(),
    };
    Some(RecentEntry {
        name,
        path,
        activated,
    })
}

/// `key` as an absolute path, `$USER_HOME$` replaced by `home`; `None` for
/// any other macro, a relative path or one going up (`..`).
fn expand(key: &str, home: &Path) -> Option<PathBuf> {
    let path = match key.strip_prefix(USER_HOME_MACRO) {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(key),
    };
    let plain = path.is_absolute()
        && !path.to_str()?.contains(MACRO_MARK)
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)));
    plain.then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/adrien";

    fn entry(key: &str, title: &str, activated: &str) -> String {
        format!(
            r#"<entry key="{key}"><value><RecentProjectMetaInfo frameTitle="{title}">
                <option name="activationTimestamp" value="{activated}" />
            </RecentProjectMetaInfo></value></entry>"#
        )
    }

    fn file(entries: &[String]) -> String {
        format!(
            r#"<application><component name="RecentProjectsManager">
                <option name="additionalInfo"><map>{}</map></option>
                <option name="lastOpenedProject" value="$USER_HOME$/Github/bergacast" />
            </component></application>"#,
            entries.concat()
        )
    }

    fn names(entries: &[RecentEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    #[test]
    fn projects_are_read_most_recent_first() {
        let xml = file(&[
            entry("$USER_HOME$/IdeaProjects/notes", "notes – README.md", "100"),
            entry("$USER_HOME$/Github/bergacast", "bergacast – main.rs", "300"),
            entry("/srv/projets/outil", "", "200"),
        ]);
        let entries = parse(&xml, Path::new(HOME)).unwrap();
        assert_eq!(names(&entries), ["bergacast", "outil", "notes"]);
        assert_eq!(entries[0].path, Path::new("/home/adrien/Github/bergacast"));
        assert_eq!(entries[0].activated, 300);
        assert_eq!(entries[1].path, Path::new("/srv/projets/outil"));
    }

    #[test]
    fn titles_give_the_name_and_folders_the_rest() {
        let xml = file(&[
            entry(
                "$USER_HOME$/Github/betgamof",
                "betgamof – BankrollService.kt [betgamof-backend.business.main]",
                "1",
            ),
            entry("$USER_HOME$/Github/sans-titre", "", "not a number"),
        ]);
        let entries = parse(&xml, Path::new(HOME)).unwrap();
        assert_eq!(names(&entries), ["betgamof", "sans-titre"]);
        assert_eq!(entries[1].activated, 0);
    }

    #[test]
    fn only_plain_paths_are_kept() {
        let xml = file(&[
            entry("$APPLICATION_CONFIG_DIR$/projet", "a", "1"),
            entry("relatif/projet", "b", "1"),
            entry("$USER_HOME$/../root/projet", "c", "1"),
            entry("/home/x/$USER_HOME$", "d", "1"),
            entry("$USER_HOME$/ok", "e", "1"),
        ]);
        let entries = parse(&xml, Path::new(HOME)).unwrap();
        assert_eq!(names(&entries), ["e"]);
    }

    #[test]
    fn entries_without_project_information_are_skipped() {
        let xml = r#"<application><component><map>
            <entry key="$USER_HOME$/a" />
            <entry key="$USER_HOME$/b"><value><RecentProjectMetaInfo /></value></entry>
        </map></component></application>"#;
        let entries = parse(xml, Path::new(HOME)).unwrap();
        assert_eq!(names(&entries), ["b"]);
    }

    #[test]
    fn many_projects_are_bounded() {
        let many: Vec<String> = (0..MAX_PROJECTS + 20)
            .map(|index| entry(&format!("/p/{index}"), "", &index.to_string()))
            .collect();
        let entries = parse(&file(&many), Path::new(HOME)).unwrap();
        assert_eq!(entries.len(), MAX_PROJECTS);
        assert_eq!(
            entries[0].activated,
            u64::try_from(MAX_PROJECTS + 19).unwrap()
        );
    }

    #[test]
    fn documents_with_a_dtd_or_broken_are_refused() {
        let entity = r#"<?xml version="1.0"?>
            <!DOCTYPE application [<!ENTITY home "/root">]>
            <application><map><entry key="&home;/x"><value><RecentProjectMetaInfo /></value></entry></map></application>"#;
        assert!(parse(entity, Path::new(HOME)).is_err());
        assert!(parse("<application>", Path::new(HOME)).is_err());
    }
}
