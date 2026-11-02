//! What Docker reports, as the module sees it: containers, and the compose
//! projects they belong to.

use std::collections::BTreeMap;
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
}

impl ComposeLabels {
    /// Labels of a container of `project`.
    #[must_use]
    pub fn new(project: String, working_dir: Option<PathBuf>, config_files: Vec<PathBuf>) -> Self {
        Self {
            project,
            working_dir,
            config_files,
        }
    }
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

/// A compose project: found from the labels of its containers, or declared
/// in the settings (then possibly without containers, after a `down`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Project {
    pub(crate) name: String,
    /// What compose needs to act on it, `None` when its containers disagree
    /// on its files (compose is then never run on it).
    pub(crate) target: Option<ComposeTarget>,
    /// Its containers, by name.
    pub(crate) containers: Vec<Container>,
}

impl Project {
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
    pub(crate) fn new(containers: &[Container], declared: &[PathBuf]) -> Self {
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
                    .is_some_and(|target| target.files.contains(file))
            });
            if !claimed {
                let name = unique_name(&project_name_of(file), &projects);
                let target = ComposeTarget::new(None, None, vec![file.clone()]);
                projects.insert(
                    name.clone(),
                    Project {
                        name,
                        target: Some(target),
                        containers: Vec::new(),
                    },
                );
            }
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

/// The project `name` of the containers `members`, with their labels.
fn found_project(
    name: &str,
    members: &[(&Container, &ComposeLabels)],
    declared: &[PathBuf],
) -> Project {
    let own_files: Vec<PathBuf> = declared
        .iter()
        .filter(|file| {
            members
                .iter()
                .any(|(_, labels)| labels.config_files.contains(file))
        })
        .cloned()
        .collect();
    let target = if let Some(first_file) = own_files.first() {
        let directory = first_file.parent().map(PathBuf::from);
        Some(ComposeTarget::new(
            Some(name.to_owned()),
            directory,
            own_files,
        ))
    } else {
        agreed_target(name, members)
    };
    let mut containers: Vec<Container> = members.iter().map(|(c, _)| (*c).clone()).collect();
    containers.sort_by(|a, b| a.name.cmp(&b.name));
    Project {
        name: name.to_owned(),
        target,
        containers,
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
        let overview = Overview::new(&containers, &[]);
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
        let declared = [
            PathBuf::from("/srv/shop/compose.yaml"),
            PathBuf::from("/home/me/My Blog!/compose.yaml"),
        ];
        let overview = Overview::new(&containers, &declared);
        assert_eq!(
            names(&overview),
            ["myblog", "shop"],
            "a running project is not repeated"
        );
        let blog = overview.project("myblog").unwrap();
        assert_eq!(blog.containers, []);
        let target = blog.target.as_ref().unwrap();
        assert_eq!(target.project, None, "compose names it");
        assert_eq!(target.files, [declared[1].clone()]);
    }

    #[test]
    fn a_declared_file_named_like_another_project_gets_a_name_of_its_own() {
        let containers = [container(
            "shop-web-1",
            ContainerStatus::Running,
            Some("shop"),
        )];
        let declared = [
            PathBuf::from("/home/me/shop/compose.yaml"),
            PathBuf::from("/opt/shop/compose.yaml"),
        ];
        let overview = Overview::new(&containers, &declared);
        assert_eq!(names(&overview), ["shop", "shop (2)", "shop (3)"]);
        let running = overview.project("shop").unwrap();
        assert_eq!(running.containers.len(), 1, "the running one is kept");
        let declared_target = overview
            .project("shop (2)")
            .unwrap()
            .target
            .as_ref()
            .unwrap();
        assert_eq!(declared_target.files, [declared[0].clone()]);
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
        let declared = [PathBuf::from("/srv/shop/compose.yaml")];
        let overview = Overview::new(&[web], &declared);
        let target = overview.project("shop").unwrap().target.clone().unwrap();
        assert_eq!(target.files, declared);
        assert_eq!(target.directory, Some(PathBuf::from("/srv/shop")));
        assert_eq!(target.project.as_deref(), Some("shop"));
    }

    #[test]
    fn containers_disagreeing_on_their_files_leave_no_target() {
        let web = container("shop-web-1", ContainerStatus::Running, Some("shop"));
        let mut impostor = container("shop-db-1", ContainerStatus::Running, None);
        impostor.compose = Some(labels("shop", "/tmp/evil"));
        let overview = Overview::new(&[web.clone(), impostor], &[]);
        let shop = overview.project("shop").unwrap();
        assert_eq!(shop.target, None);
        assert_eq!(shop.containers.len(), 2, "still shown and controllable");
        // Declaring the real file settles it.
        let declared = [PathBuf::from("/srv/shop/compose.yaml")];
        let mut impostor = container("shop-db-1", ContainerStatus::Running, None);
        impostor.compose = Some(labels("shop", "/tmp/evil"));
        let settled = Overview::new(&[web, impostor], &declared);
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
        let overview = Overview::new(&[looping, stopped], &[]);
        assert!(overview.needs_attention());
        assert!(overview.project("p").unwrap().needs_attention());
    }
}
