//! What Docker reports, as the module sees it: containers, and the compose
//! projects they belong to.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Lifecycle state of a container, as Docker reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContainerStatus {
    /// Created, never started.
    Created,
    /// Running (possibly failing its health check).
    Running,
    /// Frozen by `docker pause`.
    Paused,
    /// Restarted by its restart policy (often in a loop).
    Restarting,
    /// Being removed.
    Removing,
    /// Stopped.
    Exited,
    /// Could not be stopped or removed properly.
    Dead,
    /// Being stopped.
    Stopping,
    /// Reported as something else (newer Docker).
    Unknown,
}

/// Result of the health check of a running container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Health {
    /// Not checked enough yet.
    Starting,
    /// Passing its health check.
    Healthy,
    /// Failing its health check.
    Unhealthy,
}

/// What compose recorded on a container it created (its labels). Anyone
/// allowed to create containers may set them: the files they name are only
/// used when every container of the project agrees, and checked before
/// compose reads them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ComposeLabels {
    /// Name of the project.
    pub project: String,
    /// Directory the project was started from.
    pub working_dir: Option<PathBuf>,
    /// Compose files of the project.
    pub config_files: Vec<PathBuf>,
    /// Service of the project the container runs, `None` when not recorded.
    pub service: Option<String>,
}

impl ComposeLabels {
    /// Labels of a container of `project`.
    #[must_use]
    pub fn new(project: String, working_dir: Option<PathBuf>, config_files: Vec<PathBuf>) -> Self {
        Self {
            project,
            working_dir,
            config_files,
            service: None,
        }
    }

    /// The same labels, for a container of `service`.
    #[must_use]
    pub fn with_service(self, service: impl Into<String>) -> Self {
        Self {
            service: Some(service.into()),
            ..self
        }
    }
}

/// Whether `name` is a project name compose allows (lowercase letters,
/// digits, `-` and `_`, not starting with a symbol). The name of a project
/// found from labels is part of page ids and of the arguments of compose:
/// one that is not is not trusted.
#[must_use]
pub(crate) fn is_project_name(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'))
}

/// A container.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Container {
    /// Id, what actions address.
    pub id: String,
    /// Name, unique on the host, without the leading `/`.
    pub name: String,
    /// Image it runs.
    pub image: String,
    /// Lifecycle state.
    pub status: ContainerStatus,
    /// `None` without health check, or on Docker before API 1.52 when the
    /// status text does not tell.
    pub health: Option<Health>,
    /// `None` for a container not created by compose.
    pub compose: Option<ComposeLabels>,
}

impl Container {
    /// A container without health check nor compose labels.
    #[must_use]
    pub fn new(id: String, name: String, image: String, status: ContainerStatus) -> Self {
        Self {
            id,
            name,
            image,
            status,
            health: None,
            compose: None,
        }
    }

    /// Whether it runs.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.status == ContainerStatus::Running
    }

    /// Something is wrong: failing health check, restart loop, dead.
    #[must_use]
    pub fn needs_attention(&self) -> bool {
        matches!(
            self.status,
            ContainerStatus::Restarting | ContainerStatus::Dead
        ) || self.health == Some(Health::Unhealthy)
    }

    /// Whether it may be removed: stopped (a running one is never removed,
    /// it must be stopped first).
    #[must_use]
    pub fn is_removable(&self) -> bool {
        matches!(
            self.status,
            ContainerStatus::Created | ContainerStatus::Exited | ContainerStatus::Dead
        )
    }
}

/// What `docker compose` needs to act on a project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ComposeTarget {
    /// Name of the project, for a project found running; `None` for a
    /// declared file, whose project compose names itself.
    pub project: Option<String>,
    /// Directory of the project, `None`: that of the first file.
    pub directory: Option<PathBuf>,
    /// Compose files, absolute.
    pub files: Vec<PathBuf>,
}

impl ComposeTarget {
    /// The project of `files` (absolute), named `project` and run from
    /// `directory` when known.
    #[must_use]
    pub fn new(project: Option<String>, directory: Option<PathBuf>, files: Vec<PathBuf>) -> Self {
        Self {
            project,
            directory,
            files,
        }
    }
}

/// A compose file declared in the settings, with the name and icon the user
/// gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclaredFile {
    /// Absolute path.
    pub(crate) path: PathBuf,
    /// Name shown for its project, instead of the project name.
    pub(crate) name: Option<String>,
    /// Name of the stored image drawn for its project.
    pub(crate) icon: Option<String>,
}

impl DeclaredFile {
    /// A file with neither name nor icon.
    #[cfg(test)]
    pub(crate) fn plain(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            name: None,
            icon: None,
        }
    }
}

/// A compose project: found from the labels of its containers, or declared
/// in the settings (then possibly without containers, after a `down`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Project {
    /// What identifies it (page ids): its compose name, never the custom one.
    pub(crate) name: String,
    /// Name given by the user to its declared file, shown in place of `name`.
    pub(crate) custom_name: Option<String>,
    /// Icon given by the user to its declared file.
    pub(crate) icon: Option<String>,
    /// What compose needs to act on it, `None` when its containers disagree
    /// on its files (compose is then never run on it).
    pub(crate) target: Option<ComposeTarget>,
    /// Its containers, by name.
    pub(crate) containers: Vec<Container>,
    /// The services its files define that have no container (not built
    /// yet, or removed), by name.
    pub(crate) planned: Vec<String>,
}

impl Project {
    /// Containers and services without container.
    #[must_use]
    pub(crate) fn total(&self) -> usize {
        self.containers.len() + self.planned.len()
    }

    /// The name shown to the user.
    #[must_use]
    pub(crate) fn shown_name(&self) -> &str {
        self.custom_name.as_deref().unwrap_or(&self.name)
    }

    #[must_use]
    pub(crate) fn running(&self) -> usize {
        self.containers.iter().filter(|c| c.is_running()).count()
    }

    #[must_use]
    pub(crate) fn needs_attention(&self) -> bool {
        self.containers.iter().any(Container::needs_attention)
    }
}

/// Containers grouped as the module shows them: compose projects (found or
/// declared), then the other containers; each by name, so that keys keep
/// their place when states change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Overview {
    pub(crate) projects: Vec<Project>,
    pub(crate) isolated: Vec<Container>,
}

impl Overview {
    /// Groups `containers`, with the projects of the compose files
    /// `declared` in the settings.
    ///
    /// A declared file the containers of a project name is what compose
    /// is run on for that project, whatever their labels say. A declared
    /// file of no running project appears as its own project, named after
    /// its directory (made unique when another project has that name).
    #[must_use]
    pub(crate) fn new(
        containers: &[Container],
        declared: &[DeclaredFile],
        services: &HashMap<String, Vec<String>>,
    ) -> Self {
        let mut found: BTreeMap<String, Vec<(&Container, &ComposeLabels)>> = BTreeMap::new();
        let mut isolated = Vec::new();
        for container in containers {
            match &container.compose {
                Some(labels) => found
                    .entry(labels.project.clone())
                    .or_default()
                    .push((container, labels)),
                None => isolated.push(container.clone()),
            }
        }
        let mut projects: BTreeMap<String, Project> = found
            .into_iter()
            .map(|(name, members)| {
                let project = found_project(&name, &members, declared);
                (name, project)
            })
            .collect();
        for file in declared {
            let claimed = projects.values().any(|project| {
                project
                    .target
                    .as_ref()
                    .is_some_and(|target| target.files.contains(&file.path))
            });
            if !claimed {
                let name = unique_name(&project_name_of(&file.path), &projects);
                let target = ComposeTarget::new(None, None, vec![file.path.clone()]);
                projects.insert(
                    name.clone(),
                    Project {
                        name,
                        custom_name: file.name.clone(),
                        icon: file.icon.clone(),
                        target: Some(target),
                        containers: Vec::new(),
                        planned: Vec::new(),
                    },
                );
            }
        }
        for project in projects.values_mut() {
            project.planned =
                planned_services(project, services.get(&project.name).map(Vec::as_slice));
        }
        isolated.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            projects: projects.into_values().collect(),
            isolated,
        }
    }

    #[must_use]
    pub(crate) fn project(&self, name: &str) -> Option<&Project> {
        self.projects.iter().find(|project| project.name == name)
    }

    /// The project whose page lists the container `name`.
    #[must_use]
    pub(crate) fn project_of(&self, name: &str) -> Option<&Project> {
        self.projects.iter().find(|project| {
            project
                .containers
                .iter()
                .any(|container| container.name == name)
        })
    }

    /// Any container, by name.
    #[must_use]
    pub(crate) fn container(&self, name: &str) -> Option<&Container> {
        self.projects
            .iter()
            .flat_map(|project| &project.containers)
            .chain(&self.isolated)
            .find(|container| container.name == name)
    }

    #[must_use]
    pub(crate) fn total(&self) -> usize {
        self.isolated.len()
            + self
                .projects
                .iter()
                .map(|project| project.containers.len())
                .sum::<usize>()
    }

    #[must_use]
    pub(crate) fn running(&self) -> usize {
        self.isolated.iter().filter(|c| c.is_running()).count()
            + self.projects.iter().map(Project::running).sum::<usize>()
    }

    #[must_use]
    pub(crate) fn needs_attention(&self) -> bool {
        self.isolated.iter().any(Container::needs_attention)
            || self.projects.iter().any(Project::needs_attention)
    }
}

/// The `services` of `project` that no container runs, in the order they
/// were defined.
fn planned_services(project: &Project, services: Option<&[String]>) -> Vec<String> {
    let built: HashSet<&str> = project
        .containers
        .iter()
        .filter_map(|container| container.compose.as_ref()?.service.as_deref())
        .collect();
    services
        .into_iter()
        .flatten()
        .filter(|service| !built.contains(service.as_str()))
        .cloned()
        .collect()
}

/// The project `name` of the containers `members`, with their labels.
fn found_project(
    name: &str,
    members: &[(&Container, &ComposeLabels)],
    declared: &[DeclaredFile],
) -> Project {
    let own: Vec<&DeclaredFile> = declared
        .iter()
        .filter(|file| {
            members
                .iter()
                .any(|(_, labels)| labels.config_files.contains(&file.path))
        })
        .collect();
    let target = if let Some(first_file) = own.first() {
        let directory = first_file.path.parent().map(PathBuf::from);
        Some(ComposeTarget::new(
            Some(name.to_owned()),
            directory,
            own.iter().map(|file| file.path.clone()).collect(),
        ))
    } else {
        agreed_target(name, members)
    };
    let mut containers: Vec<Container> = members.iter().map(|(c, _)| (*c).clone()).collect();
    containers.sort_by(|a, b| a.name.cmp(&b.name));
    Project {
        name: name.to_owned(),
        custom_name: own.iter().find_map(|file| file.name.clone()),
        icon: own.iter().find_map(|file| file.icon.clone()),
        target,
        containers,
        planned: Vec::new(),
    }
}

/// The target the labels of `members` agree on, `None` when they disagree
/// (or when there is no member).
fn agreed_target(name: &str, members: &[(&Container, &ComposeLabels)]) -> Option<ComposeTarget> {
    let [(_, first), others @ ..] = members else {
        return None;
    };
    let agree = others.iter().all(|(_, labels)| {
        labels.working_dir == first.working_dir && labels.config_files == first.config_files
    });
    if !agree {
        tracing::warn!(project = name, "containers disagree on the compose files");
        return None;
    }
    Some(ComposeTarget::new(
        Some(name.to_owned()),
        first.working_dir.clone(),
        first.config_files.clone(),
    ))
}

/// `name`, or `name (2)`, `name (3)`… when a project already has it.
fn unique_name(name: &str, projects: &BTreeMap<String, Project>) -> String {
    if !projects.contains_key(name) {
        return name.to_owned();
    }
    // One more rank than projects: one of them is free.
    let unique = (2..=projects.len() + 1)
        .map(|rank| format!("{name} ({rank})"))
        .find(|candidate| !projects.contains_key(candidate))
        .unwrap_or_else(|| name.to_owned());
    tracing::debug!(name, unique, "declared compose project renamed");
    unique
}

/// Name compose gives by default to the project of `file`: the name of its
/// directory, lowercased, without the characters compose does not allow.
/// Only shown; compose names the project itself when acting on the file.
fn project_name_of(file: &Path) -> String {
    let directory = file
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    directory
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_files<const N: usize>(paths: [&str; N]) -> Vec<DeclaredFile> {
        paths.into_iter().map(DeclaredFile::plain).collect()
    }

    fn labels(project: &str, directory: &str) -> ComposeLabels {
        ComposeLabels::new(
            project.into(),
            Some(directory.into()),
            vec![format!("{directory}/compose.yaml").into()],
        )
    }

    fn container(name: &str, status: ContainerStatus, project: Option<&str>) -> Container {
        let mut container =
            Container::new(format!("id-{name}"), name.into(), "image".into(), status);
        container.compose = project.map(|project| labels(project, &format!("/srv/{project}")));
        container
    }

    fn names(overview: &Overview) -> Vec<&str> {
        overview.projects.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn containers_are_grouped_by_project_and_sorted() {
        let containers = [
            container("zeta", ContainerStatus::Running, None),
            container("shop-web-1", ContainerStatus::Running, Some("shop")),
            container("alpha", ContainerStatus::Exited, None),
            container("shop-db-1", ContainerStatus::Exited, Some("shop")),
            container("logs-app-1", ContainerStatus::Running, Some("logs")),
        ];
        let overview = Overview::new(&containers, &[], &HashMap::new());
        assert_eq!(names(&overview), ["logs", "shop"]);
        let shop = overview.project("shop").unwrap();
        let shop_names: Vec<_> = shop.containers.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(shop_names, ["shop-db-1", "shop-web-1"]);
        let target = shop.target.as_ref().unwrap();
        assert_eq!(target.project.as_deref(), Some("shop"));
        assert_eq!(target.directory, Some(PathBuf::from("/srv/shop")));
        assert_eq!(target.files, [PathBuf::from("/srv/shop/compose.yaml")]);
        assert_eq!((shop.running(), shop.containers.len()), (1, 2));
        let isolated: Vec<_> = overview.isolated.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(isolated, ["alpha", "zeta"]);
        assert_eq!((overview.running(), overview.total()), (3, 5));
        assert_eq!(overview.container("shop-db-1").unwrap().id, "id-shop-db-1");
        assert!(overview.container("nope").is_none());
    }

    #[test]
    fn declared_files_add_the_projects_that_are_down() {
        let containers = [container(
            "shop-web-1",
            ContainerStatus::Running,
            Some("shop"),
        )];
        let declared = plain_files(["/srv/shop/compose.yaml", "/home/me/My Blog!/compose.yaml"]);
        let overview = Overview::new(&containers, &declared, &HashMap::new());
        assert_eq!(
            names(&overview),
            ["myblog", "shop"],
            "a running project is not repeated"
        );
        let blog = overview.project("myblog").unwrap();
        assert_eq!(blog.containers, []);
        let target = blog.target.as_ref().unwrap();
        assert_eq!(target.project, None, "compose names it");
        assert_eq!(target.files, [declared[1].path.clone()]);
    }

    #[test]
    fn a_declared_file_named_like_another_project_gets_a_name_of_its_own() {
        let containers = [container(
            "shop-web-1",
            ContainerStatus::Running,
            Some("shop"),
        )];
        let declared = plain_files(["/home/me/shop/compose.yaml", "/opt/shop/compose.yaml"]);
        let overview = Overview::new(&containers, &declared, &HashMap::new());
        assert_eq!(names(&overview), ["shop", "shop (2)", "shop (3)"]);
        let running = overview.project("shop").unwrap();
        assert_eq!(running.containers.len(), 1, "the running one is kept");
        let declared_target = overview
            .project("shop (2)")
            .unwrap()
            .target
            .as_ref()
            .unwrap();
        assert_eq!(declared_target.files, [declared[0].path.clone()]);
    }

    #[test]
    fn a_declared_file_wins_over_the_labels_of_its_project() {
        let mut web = container("shop-web-1", ContainerStatus::Running, Some("shop"));
        web.compose = Some(ComposeLabels::new(
            "shop".into(),
            Some("/elsewhere".into()),
            vec![
                "/srv/shop/compose.yaml".into(),
                "/elsewhere/extra.yaml".into(),
            ],
        ));
        let declared = plain_files(["/srv/shop/compose.yaml"]);
        let overview = Overview::new(&[web], &declared, &HashMap::new());
        let target = overview.project("shop").unwrap().target.clone().unwrap();
        assert_eq!(target.files, [PathBuf::from("/srv/shop/compose.yaml")]);
        assert_eq!(target.directory, Some(PathBuf::from("/srv/shop")));
        assert_eq!(target.project.as_deref(), Some("shop"));
    }

    #[test]
    fn containers_disagreeing_on_their_files_leave_no_target() {
        let web = container("shop-web-1", ContainerStatus::Running, Some("shop"));
        let mut impostor = container("shop-db-1", ContainerStatus::Running, None);
        impostor.compose = Some(labels("shop", "/tmp/evil"));
        let overview = Overview::new(&[web.clone(), impostor], &[], &HashMap::new());
        let shop = overview.project("shop").unwrap();
        assert_eq!(shop.target, None);
        assert_eq!(shop.containers.len(), 2, "still shown and controllable");
        // Declaring the real file settles it.
        let declared = plain_files(["/srv/shop/compose.yaml"]);
        let mut impostor = container("shop-db-1", ContainerStatus::Running, None);
        impostor.compose = Some(labels("shop", "/tmp/evil"));
        let settled = Overview::new(&[web, impostor], &declared, &HashMap::new());
        assert!(settled.project("shop").unwrap().target.is_some());
    }

    #[test]
    fn states_tell_what_needs_attention_and_what_may_be_removed() {
        let mut unhealthy = container("a", ContainerStatus::Running, None);
        unhealthy.health = Some(Health::Unhealthy);
        assert!(unhealthy.needs_attention() && unhealthy.is_running());
        assert!(!unhealthy.is_removable());
        let looping = container("b", ContainerStatus::Restarting, Some("p"));
        assert!(looping.needs_attention());
        for status in [
            ContainerStatus::Paused,
            ContainerStatus::Restarting,
            ContainerStatus::Stopping,
            ContainerStatus::Removing,
            ContainerStatus::Unknown,
        ] {
            assert!(!container("x", status, None).is_removable(), "{status:?}");
        }
        for status in [
            ContainerStatus::Created,
            ContainerStatus::Exited,
            ContainerStatus::Dead,
        ] {
            assert!(container("x", status, None).is_removable(), "{status:?}");
        }
        let stopped = container("c", ContainerStatus::Exited, None);
        assert!(!stopped.needs_attention());
        let overview = Overview::new(&[looping, stopped], &[], &HashMap::new());
        assert!(overview.needs_attention());
        assert!(overview.project("p").unwrap().needs_attention());
    }

    #[test]
    fn a_declared_file_gives_its_name_and_icon_to_its_project() {
        let declared = [DeclaredFile {
            path: "/srv/shop/compose.yaml".into(),
            name: Some("Ma boutique".into()),
            icon: Some("shop.png".into()),
        }];
        let overview = Overview::new(&[], &declared, &HashMap::new());
        let project = overview.project("shop").unwrap();
        assert_eq!(project.shown_name(), "Ma boutique");
        assert_eq!(project.icon.as_deref(), Some("shop.png"));
    }

    #[test]
    fn a_running_project_takes_the_name_and_icon_of_its_declared_file() {
        let web = container("shop-web-1", ContainerStatus::Running, Some("shop"));
        let declared = [DeclaredFile {
            path: "/srv/shop/compose.yaml".into(),
            name: Some("Ma boutique".into()),
            icon: None,
        }];
        let overview = Overview::new(&[web], &declared, &HashMap::new());
        let project = overview.project("shop").unwrap();
        assert_eq!(project.shown_name(), "Ma boutique");
        assert_eq!(project.name, "shop", "the identity does not change");
    }

    #[test]
    fn a_project_without_custom_name_shows_its_own() {
        let overview = Overview::new(
            &[],
            &plain_files(["/srv/shop/compose.yaml"]),
            &HashMap::new(),
        );
        assert_eq!(overview.project("shop").unwrap().shown_name(), "shop");
    }

    #[test]
    fn services_without_container_are_the_planned_ones() {
        let web = container("shop-web-1", ContainerStatus::Exited, Some("shop"));
        let mut web = web;
        web.compose = web.compose.map(|labels| labels.with_service("web"));
        let services = HashMap::from([(
            "shop".to_owned(),
            vec!["db".to_owned(), "web".to_owned(), "cache".to_owned()],
        )]);
        let overview = Overview::new(&[web], &[], &services);
        let shop = overview.project("shop").unwrap();
        assert_eq!(shop.planned, ["db", "cache"], "in the order of the files");
        assert_eq!((shop.containers.len(), shop.total()), (1, 3));
    }

    #[test]
    fn a_declared_project_has_all_its_services_planned() {
        let declared = plain_files(["/srv/shop/compose.yaml"]);
        let services = HashMap::from([("shop".to_owned(), vec!["web".to_owned()])]);
        let overview = Overview::new(&[], &declared, &services);
        assert_eq!(overview.project("shop").unwrap().planned, ["web"]);
        let unknown = Overview::new(&[], &declared, &HashMap::new());
        assert_eq!(
            unknown.project("shop").unwrap().planned,
            Vec::<String>::new()
        );
    }
}
