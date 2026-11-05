//! The user's IntelliJ: installed by JetBrains Toolbox
//! (`~/.local/share/JetBrains/Toolbox/apps/intellij-idea-*`) or in `/opt`.
//!
//! - Installation: the one whose `product-info.json` changed last. Its
//!   launcher (`launch[].launcherPath`) must stay inside the installation
//!   and be a trusted executable (see `trust`): it is what the keys run.
//! - Recent projects: `recentProjects.xml` of the configuration folder the
//!   installation names (`dataDirectoryName`), or else of the
//!   `IntelliJIdea*` folder changed last. Only existing folders are kept,
//!   their links resolved.
//!
//! Both files are read again only when they change (path, time, size), so
//! that polling costs a few `stat` calls.

use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use serde::Deserialize;

use super::catalog::{Catalog, Project, ProjectCatalog};
use super::recent::{self, MAX_FILE_BYTES};
use crate::trust;

/// Folders of the Toolbox installations, under the user's data folder.
/// Home folder standing for an unknown one: never exists.
const NO_HOME: &str = "/nonexistent";

const TOOLBOX_APPS: &str = "JetBrains/Toolbox/apps";

/// Folder of system-wide installations.
const SYSTEM_APPS: &str = "/opt";

/// Names of IntelliJ installation folders.
const INSTALL_PREFIXES: [&str; 3] = ["intellij-idea", "idea", "IDEA"];

/// File describing an installation.
const PRODUCT_INFO: &str = "product-info.json";

/// Product codes of IntelliJ IDEA (Ultimate, Community).
const INTELLIJ_PRODUCTS: [&str; 2] = ["IU", "IC"];

/// Operating system of the launchers used.
const LINUX: &str = "Linux";

/// Configuration folders, under the user's configuration folder.
const JETBRAINS_CONFIG: &str = "JetBrains";

/// Name of IntelliJ IDEA configuration folders (`IntelliJIdea2026.2`).
const CONFIG_PREFIX: &str = "IntelliJIdea";

/// Recent projects, in a configuration folder.
const RECENT_PROJECTS: &str = "options/recentProjects.xml";

/// Launcher of IntelliJ, relative to its installation: the only program
/// the module runs.
pub(super) const LAUNCHER: &str = "bin/idea";

/// Execute permission bits.
const EXECUTABLE_BITS: u32 = 0o111;

/// The installation and projects of the user, read from their files.
#[derive(Debug)]
pub struct SystemCatalog {
    home: PathBuf,
    data_home: PathBuf,
    config_home: PathBuf,
    system_apps: PathBuf,
    cache: Mutex<Cache>,
}

/// A file as last seen.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: u64,
}

impl Stamp {
    /// The regular file `path` (never a FIFO or a device, which could
    /// block the read).
    fn of(path: &Path) -> Option<Self> {
        let metadata = fs::metadata(path).ok().filter(fs::Metadata::is_file)?;
        Some(Self {
            path: path.to_owned(),
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

/// What an installation says of itself.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Installation {
    /// Its folder.
    folder: PathBuf,
    /// Whether it declares the expected launcher ([`LAUNCHER`]).
    has_launcher: bool,
    data_directory: Option<String>,
}

#[derive(Debug, Default)]
struct Cache {
    installation: Option<(Stamp, Option<Installation>)>,
    projects: Option<(Stamp, Vec<Project>)>,
    /// Why the launcher was last refused, to log only changes.
    launcher_refused: Option<String>,
}

impl SystemCatalog {
    /// The catalog of the user running the manager (`HOME`, and the XDG
    /// folders when set). Nothing is read before [`ProjectCatalog::read`].
    #[must_use]
    pub fn new() -> Self {
        // Without a home folder, nothing of the user's is read (no folder
        // relative to the manager's): `/nonexistent`, as for the system
        // users without one.
        let home = crate::paths::user_home().unwrap_or_else(|| PathBuf::from(NO_HOME));
        let xdg = |variable: &str, fallback: &str| {
            std::env::var_os(variable)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .unwrap_or_else(|| home.join(fallback))
        };
        Self::with_folders(
            home.clone(),
            xdg("XDG_DATA_HOME", ".local/share"),
            xdg("XDG_CONFIG_HOME", ".config"),
            PathBuf::from(SYSTEM_APPS),
        )
    }

    fn with_folders(
        home: PathBuf,
        data_home: PathBuf,
        config_home: PathBuf,
        system_apps: PathBuf,
    ) -> Self {
        Self {
            home,
            data_home,
            config_home,
            system_apps,
            cache: Mutex::default(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Cache> {
        // Every update leaves the cache consistent: a poisoned lock is valid.
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The `product-info.json` changed last among the installations.
    fn newest_installation(&self) -> Option<Stamp> {
        let roots = [self.data_home.join(TOOLBOX_APPS), self.system_apps.clone()];
        roots
            .iter()
            .filter_map(|root| fs::read_dir(root).ok())
            .flatten()
            .flatten()
            .filter(|entry| {
                entry.file_name().to_str().is_some_and(|name| {
                    INSTALL_PREFIXES
                        .iter()
                        .any(|prefix| name.starts_with(prefix))
                })
            })
            .filter_map(|entry| Stamp::of(&entry.path().join(PRODUCT_INFO)))
            .max_by_key(|stamp| stamp.modified)
    }

    fn installation(&self, cache: &mut Cache) -> Option<Installation> {
        let stamp = self.newest_installation()?;
        if let Some((seen, installation)) = &cache.installation
            && *seen == stamp
        {
            return installation.clone();
        }
        let installation = read_installation(&stamp.path)
            .inspect_err(|error| tracing::warn!(%error, "IntelliJ installation unreadable"))
            .ok();
        cache.installation = Some((stamp, installation.clone()));
        installation
    }

    /// The launcher of `installation`, checked again at each read (its
    /// permissions may change without its installation changing).
    fn launcher(cache: &mut Cache, installation: &Installation) -> Option<PathBuf> {
        if !installation.has_launcher {
            return None;
        }
        let checked = trusted_launcher(&installation.folder, LAUNCHER);
        let refused = checked.as_ref().err().cloned();
        if refused != cache.launcher_refused
            && let Some(reason) = &refused
        {
            tracing::warn!(%reason, "IntelliJ launcher refused");
        }
        cache.launcher_refused = refused;
        checked.ok()
    }

    /// `recentProjects.xml` of `data_directory`, or of the configuration
    /// folder changed last.
    fn recent_projects_file(&self, data_directory: Option<&str>) -> Option<PathBuf> {
        let config = self.config_home.join(JETBRAINS_CONFIG);
        let folder = match data_directory {
            Some(name) => config.join(name),
            None => fs::read_dir(&config)
                .ok()?
                .flatten()
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with(CONFIG_PREFIX))
                })
                .max_by_key(|entry| entry.metadata().and_then(|meta| meta.modified()).ok())?
                .path(),
        };
        Some(folder.join(RECENT_PROJECTS))
    }

    /// The projects of `file`, read and resolved without holding the cache
    /// (a folder on an unreachable mount may take long); an unreadable file
    /// gives none, logged once per version of the file.
    fn projects(&self, file: &Path) -> Vec<Project> {
        let Some(stamp) = Stamp::of(file) else {
            return Vec::new();
        };
        if let Some((seen, projects)) = &self.lock().projects
            && *seen == stamp
        {
            return projects.clone();
        }
        let projects = read_projects(file, &self.home).unwrap_or_else(|error| {
            tracing::warn!(%error, "IntelliJ recent projects unreadable");
            Vec::new()
        });
        self.lock().projects = Some((stamp, projects.clone()));
        projects
    }
}

/// The recent projects of `file` whose folder exists, links resolved.
fn read_projects(file: &Path, home: &Path) -> Result<Vec<Project>, String> {
    let xml = read_bounded(file)?;
    let entries =
        recent::parse(&xml, home).map_err(|error| format!("{}: {error}", file.display()))?;
    Ok(entries
        .into_iter()
        .filter_map(|entry| {
            let path = fs::canonicalize(&entry.path).ok()?;
            (path.is_dir() && path.to_str().is_some()).then_some(Project {
                name: entry.name,
                path,
            })
        })
        .collect())
}

impl Default for SystemCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectCatalog for SystemCatalog {
    fn read(&self) -> Result<Catalog, String> {
        let (launcher, file) = {
            let mut cache = self.lock();
            let installation = self.installation(&mut cache);
            let launcher = installation
                .as_ref()
                .and_then(|installation| Self::launcher(&mut cache, installation));
            let data_directory = installation.and_then(|installation| installation.data_directory);
            (
                launcher,
                self.recent_projects_file(data_directory.as_deref()),
            )
        };
        let projects = file.map(|file| self.projects(&file)).unwrap_or_default();
        Ok(Catalog { launcher, projects })
    }
}

/// The parts of `product-info.json` used.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProductInfo {
    product_code: String,
    data_directory_name: Option<String>,
    #[serde(default)]
    launch: Vec<LaunchInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LaunchInfo {
    os: String,
    launcher_path: String,
}

/// The installation described by `product_info`, a file nobody else may
/// change (it names the launcher).
fn read_installation(product_info: &Path) -> Result<Installation, String> {
    let product_info = trust::trusted(product_info).map_err(|error| error.to_string())?;
    let info: ProductInfo = serde_json::from_str(&read_bounded(&product_info)?)
        .map_err(|error| format!("{}: {error}", product_info.display()))?;
    if !INTELLIJ_PRODUCTS.contains(&info.product_code.as_str()) {
        return Err(format!("not IntelliJ IDEA: {}", info.product_code));
    }
    let install = product_info
        .parent()
        .ok_or_else(|| "installation without folder".to_owned())?;
    // Only the IDE itself: never another program of the installation
    // (`bin/format.sh`…), whatever the file names.
    let has_launcher = info
        .launch
        .iter()
        .any(|launch| launch.os == LINUX && launch.launcher_path == LAUNCHER);
    let data_directory = info
        .data_directory_name
        .filter(|name| is_single_name(Path::new(name)));
    Ok(Installation {
        folder: install.to_owned(),
        has_launcher,
        data_directory,
    })
}

/// The launcher `relative` of `install`, once found to stay inside it and
/// to be a trusted executable file.
fn trusted_launcher(install: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    let inside = !relative.as_os_str().is_empty()
        && relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if !inside {
        return Err(format!(
            "launcher outside the installation: {}",
            relative.display()
        ));
    }
    let launcher = trust::trusted(&install.join(relative)).map_err(|error| error.to_string())?;
    let install = fs::canonicalize(install).map_err(|error| error.to_string())?;
    if !launcher.starts_with(&install) {
        return Err(format!(
            "launcher outside the installation: {}",
            launcher.display()
        ));
    }
    let metadata = fs::metadata(&launcher).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.permissions().mode() & EXECUTABLE_BITS == 0 {
        return Err(format!("launcher not executable: {}", launcher.display()));
    }
    Ok(launcher)
}

/// Whether `path` is a single plain name (a folder name).
fn is_single_name(path: &Path) -> bool {
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

/// The text of `path`, at most [`MAX_FILE_BYTES`].
fn read_bounded(path: &Path) -> Result<String, String> {
    let describe = |error: std::io::Error| format!("{}: {error}", path.display());
    let mut text = String::new();
    fs::File::open(path)
        .map_err(describe)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(describe)?;
    if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
        return Err(format!("{}: over {MAX_FILE_BYTES} bytes", path.display()));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use std::fs::Permissions;

    use serde_json::json;

    use super::*;

    /// A fake home with an installation and its configuration.
    struct Fixture {
        root: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            fs::set_permissions(root.path(), Permissions::from_mode(0o755)).unwrap();
            let fixture = Self { root };
            fixture.install("intellij-idea-ultimate", "IU", "bin/idea");
            fixture.projects(&["bergadeck", "bergacast"]);
            fixture
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.root.path().join(relative)
        }

        fn catalog(&self) -> SystemCatalog {
            SystemCatalog::with_folders(
                self.path("home"),
                self.path("home/.local/share"),
                self.path("home/.config"),
                self.path("opt"),
            )
        }

        fn install(&self, name: &str, product: &str, launcher: &str) -> PathBuf {
            let install = self.path("home/.local/share").join(TOOLBOX_APPS).join(name);
            fs::create_dir_all(install.join("bin")).unwrap();
            let program = install.join("bin/idea");
            fs::write(&program, "#!/bin/true\n").unwrap();
            fs::set_permissions(&program, Permissions::from_mode(0o755)).unwrap();
            let info = json!({
                "productCode": product,
                "dataDirectoryName": "IntelliJIdea2026.2",
                "launch": [{ "os": "Linux", "launcherPath": launcher }],
            });
            fs::write(install.join(PRODUCT_INFO), info.to_string()).unwrap();
            install
        }

        /// Writes the recent projects `names`, the first one most recent,
        /// each with its folder.
        fn projects(&self, names: &[&str]) {
            let entries: Vec<String> = names
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    fs::create_dir_all(self.path("home/projets").join(name)).unwrap();
                    format!(
                        r#"<entry key="$USER_HOME$/projets/{name}"><value>
                        <RecentProjectMetaInfo frameTitle="{name} – main.rs">
                        <option name="activationTimestamp" value="{}" />
                        </RecentProjectMetaInfo></value></entry>"#,
                        names.len() - index
                    )
                })
                .collect();
            let file = self
                .path("home/.config")
                .join(JETBRAINS_CONFIG)
                .join("IntelliJIdea2026.2")
                .join(RECENT_PROJECTS);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let entries = entries.concat();
            fs::write(
                file,
                format!("<application><map>{entries}</map></application>"),
            )
            .unwrap();
        }
    }

    fn names(catalog: &Catalog) -> Vec<&str> {
        catalog.projects.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn the_installation_and_its_projects_are_found() {
        let fixture = Fixture::new();
        let catalog = fixture.catalog().read().unwrap();
        let launcher = catalog.launcher.clone().unwrap();
        assert!(
            launcher.ends_with("intellij-idea-ultimate/bin/idea"),
            "{launcher:?}"
        );
        assert_eq!(names(&catalog), ["bergadeck", "bergacast"]);
        assert!(catalog.projects[0].path.is_absolute());
    }

    #[test]
    fn projects_are_read_again_when_the_file_changes() {
        let fixture = Fixture::new();
        let catalog = fixture.catalog();
        assert_eq!(names(&catalog.read().unwrap()).len(), 2);
        fixture.projects(&["notes", "bergadeck", "bergacast"]);
        assert_eq!(
            names(&catalog.read().unwrap()),
            ["notes", "bergadeck", "bergacast"]
        );
    }

    #[test]
    fn removed_project_folders_are_skipped() {
        let fixture = Fixture::new();
        fs::remove_dir(fixture.path("home/projets/bergacast")).unwrap();
        assert_eq!(names(&fixture.catalog().read().unwrap()), ["bergadeck"]);
    }

    #[test]
    fn launchers_outside_the_installation_or_untrusted_are_refused() {
        let fixture = Fixture::new();
        let install = fixture
            .path("home/.local/share")
            .join(TOOLBOX_APPS)
            .join("intellij-idea-ultimate");
        for escaping in ["../../../../../../bin/sh", "/bin/sh", ""] {
            assert!(trusted_launcher(&install, escaping).is_err(), "{escaping}");
        }
        let elsewhere = fixture.path("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        fs::write(elsewhere.join("idea"), "").unwrap();
        fs::set_permissions(elsewhere.join("idea"), Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(elsewhere.join("idea"), install.join("bin/link")).unwrap();
        assert!(
            trusted_launcher(&install, "bin/link")
                .unwrap_err()
                .contains("outside")
        );

        let program = install.join("bin/idea");
        fs::set_permissions(&program, Permissions::from_mode(0o775)).unwrap();
        assert!(
            trusted_launcher(&install, "bin/idea").is_err(),
            "group writable"
        );
        fs::set_permissions(&program, Permissions::from_mode(0o644)).unwrap();
        assert!(
            trusted_launcher(&install, "bin/idea")
                .unwrap_err()
                .contains("executable")
        );
    }

    #[test]
    fn without_a_usable_installation_there_is_no_launcher() {
        let fixture = Fixture::new();
        let install = fixture.install("intellij-idea-ultimate", "IU", "../outside");
        let catalog = fixture.catalog().read().unwrap();
        assert_eq!(catalog.launcher, None);
        assert_eq!(names(&catalog).len(), 2, "projects still listed");

        fs::write(
            install.join(PRODUCT_INFO),
            json!({"productCode": "RR"}).to_string(),
        )
        .unwrap();
        assert_eq!(fixture.catalog().read().unwrap().launcher, None);
    }

    #[test]
    fn without_installation_the_newest_configuration_is_used() {
        let fixture = Fixture::new();
        fs::remove_dir_all(fixture.path("home/.local/share")).unwrap();
        let catalog = fixture.catalog().read().unwrap();
        assert_eq!(catalog.launcher, None);
        assert_eq!(names(&catalog), ["bergadeck", "bergacast"]);
    }

    fn toolbox_install(fixture: &Fixture) -> PathBuf {
        fixture
            .path("home/.local/share")
            .join(TOOLBOX_APPS)
            .join("intellij-idea-ultimate")
    }

    #[test]
    fn only_the_ide_launcher_is_accepted() {
        let fixture = Fixture::new();
        let install = fixture.install("intellij-idea-ultimate", "IU", "bin/format.sh");
        let format = install.join("bin/format.sh");
        fs::write(&format, "").unwrap();
        fs::set_permissions(&format, Permissions::from_mode(0o755)).unwrap();
        assert_eq!(fixture.catalog().read().unwrap().launcher, None);
    }

    #[test]
    fn a_product_info_others_may_change_is_refused() {
        let fixture = Fixture::new();
        let info = toolbox_install(&fixture).join(PRODUCT_INFO);
        fs::set_permissions(&info, Permissions::from_mode(0o666)).unwrap();
        let catalog = fixture.catalog().read().unwrap();
        assert_eq!(catalog.launcher, None);
        assert_eq!(names(&catalog).len(), 2, "the newest configuration is used");
    }

    #[test]
    fn the_launcher_is_checked_again_at_each_read() {
        let fixture = Fixture::new();
        let catalog = fixture.catalog();
        assert!(catalog.read().unwrap().launcher.is_some());
        let program = toolbox_install(&fixture).join(LAUNCHER);
        fs::set_permissions(&program, Permissions::from_mode(0o775)).unwrap();
        assert_eq!(catalog.read().unwrap().launcher, None, "group writable now");
        fs::set_permissions(&program, Permissions::from_mode(0o755)).unwrap();
        assert!(catalog.read().unwrap().launcher.is_some());
    }

    #[test]
    fn a_broken_recent_projects_file_keeps_the_launcher() {
        let fixture = Fixture::new();
        let file = fixture
            .path("home/.config")
            .join(JETBRAINS_CONFIG)
            .join("IntelliJIdea2026.2")
            .join(RECENT_PROJECTS);
        fs::write(&file, "<application>").unwrap();
        let catalog = fixture.catalog().read().unwrap();
        assert!(catalog.launcher.is_some());
        assert_eq!(catalog.projects, []);
    }

    #[test]
    fn special_files_are_never_read() {
        let fixture = Fixture::new();
        let file = fixture
            .path("home/.config")
            .join(JETBRAINS_CONFIG)
            .join("IntelliJIdea2026.2")
            .join(RECENT_PROJECTS);
        fs::remove_file(&file).unwrap();
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &file,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        // Would block on the pipe if it were opened.
        assert_eq!(fixture.catalog().read().unwrap().projects, []);
    }

    #[test]
    fn the_newest_installation_wins() {
        let fixture = Fixture::new();
        let older = toolbox_install(&fixture).join(PRODUCT_INFO);
        let past = SystemTime::now() - std::time::Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(&older)
            .unwrap()
            .set_modified(past)
            .unwrap();
        let newer = fixture.path("opt/idea-IU-262");
        fs::create_dir_all(newer.join("bin")).unwrap();
        fs::copy(
            toolbox_install(&fixture).join(LAUNCHER),
            newer.join(LAUNCHER),
        )
        .unwrap();
        fs::copy(&older, newer.join(PRODUCT_INFO)).unwrap();
        let launcher = fixture.catalog().read().unwrap().launcher.unwrap();
        assert!(launcher.starts_with(fixture.path("opt")), "{launcher:?}");
    }

    #[test]
    fn an_invalid_data_directory_falls_back_to_the_newest_configuration() {
        let fixture = Fixture::new();
        let info = json!({
            "productCode": "IU",
            "dataDirectoryName": "../..",
            "launch": [{ "os": "Linux", "launcherPath": LAUNCHER }],
        });
        fs::write(
            toolbox_install(&fixture).join(PRODUCT_INFO),
            info.to_string(),
        )
        .unwrap();
        let catalog = fixture.catalog().read().unwrap();
        assert_eq!(names(&catalog), ["bergadeck", "bergacast"]);
        assert!(catalog.launcher.is_some());
    }

    #[test]
    fn a_missing_configuration_gives_no_project() {
        let fixture = Fixture::new();
        let info = json!({
            "productCode": "IU",
            "dataDirectoryName": "IntelliJIdea2099.1",
            "launch": [{ "os": "Linux", "launcherPath": LAUNCHER }],
        });
        fs::write(
            toolbox_install(&fixture).join(PRODUCT_INFO),
            info.to_string(),
        )
        .unwrap();
        assert_eq!(fixture.catalog().read().unwrap().projects, []);
    }

    #[test]
    fn project_links_are_resolved() {
        let fixture = Fixture::new();
        let real = fixture.path("home/projets/bergadeck");
        let link = fixture.path("home/projets/raccourci");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        fixture.projects(&["raccourci"]);
        let catalog = fixture.catalog().read().unwrap();
        assert_eq!(catalog.projects[0].path, fs::canonicalize(&real).unwrap());
    }

    #[test]
    fn an_oversized_product_info_is_refused() {
        let fixture = Fixture::new();
        let info = toolbox_install(&fixture).join(PRODUCT_INFO);
        let padding = " ".repeat(usize::try_from(MAX_FILE_BYTES).unwrap());
        fs::write(&info, format!(r#"{{"productCode": "IU"{padding}}}"#)).unwrap();
        assert_eq!(fixture.catalog().read().unwrap().launcher, None);
    }

    #[test]
    fn nothing_installed_gives_an_empty_catalog() {
        let root = tempfile::tempdir().unwrap();
        let catalog = SystemCatalog::with_folders(
            root.path().into(),
            root.path().join("data"),
            root.path().join("config"),
            root.path().join("opt"),
        );
        assert_eq!(catalog.read().unwrap(), Catalog::default());
    }

    #[test]
    fn configuration_names_are_single_folder_names() {
        assert!(is_single_name(Path::new("IntelliJIdea2026.2")));
        for name in ["", "..", "a/b", "/etc", "."] {
            assert!(!is_single_name(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn oversized_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("big.xml");
        fs::write(
            &file,
            vec![b' '; usize::try_from(MAX_FILE_BYTES).unwrap() + 1],
        )
        .unwrap();
        assert!(read_bounded(&file).unwrap_err().contains("over"));
    }
}
