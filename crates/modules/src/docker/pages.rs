//! The pages of the module: their ids, and the page an item opens.

use hubergamof_module_api::PageId;

use super::model::{Container, Overview};

/// Page opened by the home key.
pub(super) const MAIN_PAGE: &str = "docker";
/// Prefixes of the pages (and of the items opening them) of a project, of
/// its `up` and `down` variants, of a container; followed by its name.
pub(super) const PROJECT_PREFIX: &str = "project/";
pub(super) const UP_PREFIX: &str = "up/";
pub(super) const DOWN_PREFIX: &str = "down/";
pub(super) const CONTAINER_PREFIX: &str = "container/";
pub(super) const SERVICE_PREFIX: &str = "service/";
/// Items of a project page opening the pages of its variants.
pub(super) const UP_ITEM: &str = "up";
pub(super) const DOWN_ITEM: &str = "down";

/// What an `up` or `down` page is about: a whole project, or one service of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Scope<'a> {
    pub(super) project: &'a str,
    pub(super) service: Option<&'a str>,
}

impl<'a> Scope<'a> {
    /// `project`, or `project/service`.
    pub(super) fn parse(rest: &'a str) -> Self {
        match rest.split_once('/') {
            Some((project, service)) => Self {
                project,
                service: Some(service),
            },
            None => Self {
                project: rest,
                service: None,
            },
        }
    }
}

/// A page of the module, from its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Page<'a> {
    Main,
    Project(&'a str),
    Up(Scope<'a>),
    Down(Scope<'a>),
    Container(&'a str),
    /// A service of a project that has no container.
    Service {
        project: &'a str,
        service: &'a str,
    },
}

impl<'a> Page<'a> {
    pub(super) fn parse(id: &'a str) -> Option<Self> {
        if id == MAIN_PAGE {
            return Some(Self::Main);
        }
        if let Some(rest) = id.strip_prefix(SERVICE_PREFIX) {
            let (project, service) = rest.split_once('/')?;
            return Some(Self::Service { project, service });
        }
        let named = [
            (PROJECT_PREFIX, Self::Project as fn(&'a str) -> Self),
            (UP_PREFIX, |rest| Self::Up(Scope::parse(rest))),
            (DOWN_PREFIX, |rest| Self::Down(Scope::parse(rest))),
            (CONTAINER_PREFIX, Self::Container),
        ];
        named
            .into_iter()
            .find_map(|(prefix, page)| id.strip_prefix(prefix).map(page))
    }

    /// What an `up` or `down` page is about.
    pub(super) fn scope(self) -> Option<Scope<'a>> {
        match self {
            Self::Up(scope) | Self::Down(scope) => Some(scope),
            _ => None,
        }
    }

    /// The page opened by `item` of this page, if it opens one.
    pub(super) fn opened_by(self, item: &str, overview: &Overview) -> Option<PageId> {
        let opens_child = item.starts_with(PROJECT_PREFIX) || item.starts_with(CONTAINER_PREFIX);
        match self {
            Self::Main if opens_child => Some(item.into()),
            Self::Project(_)
                if item.starts_with(CONTAINER_PREFIX) || item.starts_with(SERVICE_PREFIX) =>
            {
                Some(item.into())
            }
            Self::Project(name) if item == UP_ITEM => Some(up_id(name, None).into()),
            Self::Project(name) if item == DOWN_ITEM => Some(down_id(name, None).into()),
            Self::Container(name) => {
                let (project, service) = compose_service(overview.container(name)?)?;
                match item {
                    UP_ITEM => Some(up_id(project, Some(service)).into()),
                    DOWN_ITEM => Some(down_id(project, Some(service)).into()),
                    _ => None,
                }
            }
            Self::Service { project, service } if item == UP_ITEM => {
                Some(up_id(project, Some(service)).into())
            }
            _ => None,
        }
    }
}

/// The page listing the container `name`, the one its page is opened from:
/// the page of its project, else the main page.
pub(super) fn container_parent(overview: &Overview, name: &str) -> PageId {
    overview.project_of(name).map_or_else(
        || MAIN_PAGE.into(),
        |project| project_id(&project.name).into(),
    )
}

/// The project and service a compose container runs.
pub(super) fn compose_service(container: &Container) -> Option<(&str, &str)> {
    let labels = container.compose.as_ref()?;
    Some((labels.project.as_str(), labels.service.as_deref()?))
}

pub(super) fn project_id(name: &str) -> String {
    format!("{PROJECT_PREFIX}{name}")
}

pub(super) fn container_id(name: &str) -> String {
    format!("{CONTAINER_PREFIX}{name}")
}

pub(super) fn service_id(project: &str, service: &str) -> String {
    format!("{SERVICE_PREFIX}{project}/{service}")
}

pub(super) fn scoped_id(prefix: &str, project: &str, service: Option<&str>) -> String {
    match service {
        Some(service) => format!("{prefix}{project}/{service}"),
        None => format!("{prefix}{project}"),
    }
}

pub(super) fn up_id(project: &str, service: Option<&str>) -> String {
    scoped_id(UP_PREFIX, project, service)
}

pub(super) fn down_id(project: &str, service: Option<&str>) -> String {
    scoped_id(DOWN_PREFIX, project, service)
}
