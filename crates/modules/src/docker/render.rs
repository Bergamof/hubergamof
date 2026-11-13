//! What the pages and keys of the module show.

use super::actions::{Action, CONTAINER_ACTIONS, PROJECT_ACTIONS};
use super::pages::{Page, compose_service};
use super::selection::{selectables, with_count};
use super::{
    CONFIRM_LABEL, Container, ContainerStatus, DOWN_ITEM, DOWN_LABEL, Docker, Health, ItemKey,
    KeyContent, KeyState, MODULE_LABEL, NOT_BUILT_LABEL, Overview, PageContent, PageId, PageItem,
    Project, TITLE_SEPARATOR, UP_ITEM, UP_LABEL, container_id, icons, project_id, service_id,
};

/// An entry of a project page: a container, or a service that has none.
pub(super) struct Entry {
    /// What the entries are sorted by.
    name: String,
    /// The service it stands for, `None` for a container that records none.
    pub(super) service: Option<String>,
    pub(super) item: PageItem,
}

impl Docker {
    pub(super) fn action_key(&self, page: &PageId, action: Action) -> PageItem {
        let key = ItemKey::new(page, &action.item().into());
        let ticked = Page::parse(&page.0).map_or(0, |page| self.ticked_on(page));
        let (label, state) = if self.busy.contains(&key) {
            (action.busy_label().to_owned(), KeyState::Normal)
        } else if self.is_confirming(&key) {
            (CONFIRM_LABEL.to_owned(), KeyState::Attention)
        } else {
            (action.label_for(ticked), KeyState::Normal)
        };
        PageItem {
            id: key.item,
            key: KeyContent::with_icon(action.icon(), label).in_state(state),
        }
    }

    pub(super) fn project_page(&self, page: &PageId, project: &Project) -> PageContent {
        let actions = PROJECT_ACTIONS
            .iter()
            .map(|action| self.action_key(page, *action));
        let variants = [up_item(), down_item()];
        let entries = self.project_entries(project);
        let context_dial = self.context_dial(project, &selectables(&entries));
        let entries = entries
            .into_iter()
            .map(|entry| self.mark_selection(&project.name, entry));
        let items = actions.chain(variants).chain(entries).collect();
        PageContent::new(project.shown_name(), items).with_context_dial(context_dial)
    }

    /// The containers of `project` and its services without container, sorted
    /// by name, as the keys after its actions.
    pub(super) fn project_entries(&self, project: &Project) -> Vec<Entry> {
        let prefix = format!("{}-", project.name);
        let built = project.containers.iter().map(|container| {
            let short = container
                .name
                .strip_prefix(&prefix)
                .unwrap_or(&container.name);
            let service = container
                .compose
                .as_ref()
                .and_then(|c| c.service.as_deref());
            let item = self
                .working(project, service, container_id(&container.name), short)
                .unwrap_or_else(|| self.container_entry(container, short));
            Entry {
                name: short.to_owned(),
                service: service.map(str::to_owned),
                item,
            }
        });
        let planned = project.planned.iter().map(|service| {
            let item = self.working(
                project,
                Some(service),
                service_id(&project.name, service),
                service,
            );
            Entry {
                name: service.clone(),
                service: Some(service.clone()),
                item: item.unwrap_or_else(|| planned_item(&project.name, service)),
            }
        });
        let mut entries: Vec<Entry> = built.chain(planned).collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    /// The item of `entry` showing the cursor and the tick of the selection.
    fn mark_selection(&self, project: &str, entry: Entry) -> PageItem {
        let (Some(service), Some(selection)) = (entry.service, self.selection.of(project)) else {
            return entry.item;
        };
        let focused = selection.cursor() == Some(entry.item.id.0.as_str());
        PageItem {
            id: entry.item.id,
            key: entry
                .item
                .key
                .checked(selection.is_ticked(&service))
                .focused(focused),
        }
    }

    /// The key of an entry of `project` (a container, or a service without
    /// one) that an operation is working on: its name and what is done.
    pub(super) fn working(
        &self,
        project: &Project,
        service: Option<&str>,
        id: String,
        label: &str,
    ) -> Option<PageItem> {
        let operation = self
            .operations
            .iter()
            .find(|op| op.project == project.name && op.covers(service))?;
        let label = format!("{label} {}", operation.kind.working_label());
        Some(PageItem {
            id: id.into(),
            key: KeyContent::with_icon(&icons::WORKING, &label),
        })
    }

    /// Key of a container, labelled `label`: what it shows while an action
    /// of its page runs, else its state.
    pub(super) fn container_entry(&self, container: &Container, label: &str) -> PageItem {
        let Some(action) = self.container_action(&container.name) else {
            return container_item(container, label);
        };
        let label = format!("{label} {}", action.busy_label());
        PageItem {
            id: container_id(&container.name).into(),
            key: KeyContent::with_icon(&icons::WORKING, &label),
        }
    }

    /// The projects, then the other containers.
    pub(super) fn main_page(&self, overview: &Overview) -> PageContent {
        let projects = overview
            .projects
            .iter()
            .map(|project| self.project_item(project));
        let containers = overview
            .isolated
            .iter()
            .map(|container| self.container_entry(container, &container.name));
        PageContent::new(MODULE_LABEL, projects.chain(containers).collect())
    }

    /// Key of a project on the main page.
    fn project_item(&self, project: &Project) -> PageItem {
        if let Some(operation) = self.operations.iter().find(|op| op.project == project.name) {
            let label = format!(
                "{} {}",
                project.shown_name(),
                operation.kind.working_label()
            );
            return PageItem {
                id: project_id(&project.name).into(),
                key: KeyContent::with_icon(&icons::WORKING, &label),
            };
        }
        let (icon, state) = if project.needs_attention() {
            (&icons::PROJECT_ATTENTION, KeyState::Attention)
        } else if project.running() > 0 {
            (&icons::PROJECT_RUNNING, KeyState::Active)
        } else {
            (&icons::PROJECT_STOPPED, KeyState::Normal)
        };
        // The user's icon replaces the layers; the state then shows in the
        // counts of the label (and the background of an attention).
        let icon = project
            .icon
            .as_ref()
            .and_then(|name| self.icons.get(name))
            .unwrap_or(icon);
        let label = format!(
            "{} {}/{}",
            project.shown_name(),
            project.running(),
            project.total()
        );
        PageItem {
            id: project_id(&project.name).into(),
            key: KeyContent::with_icon(icon, &label).in_state(state),
        }
    }

    pub(super) fn variants_page(
        &self,
        page: &PageId,
        title: String,
        actions: &[Action],
    ) -> PageContent {
        let items = actions
            .iter()
            .map(|action| self.action_key(page, *action))
            .collect();
        PageContent::new(title, items)
    }

    pub(super) fn container_page(
        &self,
        page: &PageId,
        container: &Container,
        overview: &Overview,
    ) -> PageContent {
        let actions = CONTAINER_ACTIONS
            .iter()
            .filter(|action| **action != Action::Remove || container.is_removable())
            .map(|action| self.action_key(page, *action));
        // A compose container also has its own `up` and `down`.
        let scoped = compose_service(container)
            .and_then(|(project, _)| overview.project(project)?.target.as_ref())
            .map(|_| [up_item(), down_item()])
            .into_iter()
            .flatten();
        let title = format!(
            "{}{TITLE_SEPARATOR}{}",
            container.name,
            status_label(container)
        );
        PageContent::new(title, actions.chain(scoped).collect())
    }

    /// The page of a service no container runs: it can only be brought up.
    pub(super) fn service_page(project: &Project, service: &str) -> Option<PageContent> {
        project.planned.iter().any(|name| name == service).then(|| {
            let items = project
                .target
                .as_ref()
                .map(|_| vec![up_item()])
                .unwrap_or_default();
            let title = format!("{service}{TITLE_SEPARATOR}{NOT_BUILT_LABEL}");
            PageContent::new(title, items)
        })
    }
}

/// The key opening the page of the variants of `up`.
pub(super) fn up_item() -> PageItem {
    PageItem {
        id: UP_ITEM.into(),
        key: KeyContent::with_icon(&icons::UP, UP_LABEL),
    }
}

/// The key opening the page of the variants of `down`.
pub(super) fn down_item() -> PageItem {
    PageItem {
        id: DOWN_ITEM.into(),
        key: KeyContent::with_icon(&icons::DOWN, DOWN_LABEL),
    }
}

/// Title of an `up` or `down` page: `label` and what it is about, with the
/// number of services ticked when it works on them (`ticked` above zero).
pub(super) fn variants_title(
    label: &str,
    project: &Project,
    service: Option<&str>,
    ticked: usize,
) -> String {
    let target = service.unwrap_or_else(|| project.shown_name());
    with_count(&format!("{label}{TITLE_SEPARATOR}{target}"), ticked)
}

/// Key of a service of a project that has no container.
pub(super) fn planned_item(project: &str, service: &str) -> PageItem {
    PageItem {
        id: service_id(project, service).into(),
        key: KeyContent::with_icon(&icons::SERVICE_PLANNED, service),
    }
}

/// Key of a container, labelled `label`, opening its page.
pub(super) fn container_item(container: &Container, label: &str) -> PageItem {
    let (icon, state) = if container.needs_attention() {
        (&icons::ATTENTION, KeyState::Attention)
    } else if container.is_running() {
        (&icons::RUNNING, KeyState::Active)
    } else {
        (&icons::STOPPED, KeyState::Normal)
    };
    PageItem {
        id: container_id(&container.name).into(),
        key: KeyContent::with_icon(icon, label).in_state(state),
    }
}

pub(super) fn status_label(container: &Container) -> &'static str {
    match (container.status, container.health) {
        (ContainerStatus::Running, Some(Health::Unhealthy)) => "Défaillant",
        (ContainerStatus::Running, Some(Health::Starting)) => "Démarrage",
        (ContainerStatus::Running, _) => "En marche",
        (ContainerStatus::Created, _) => "Créé",
        (ContainerStatus::Paused, _) => "En pause",
        (ContainerStatus::Restarting, _) => "Redémarre",
        (ContainerStatus::Removing, _) => "Suppression",
        (ContainerStatus::Exited, _) => "Arrêté",
        (ContainerStatus::Dead, _) => "Mort",
        (ContainerStatus::Stopping, _) => "Arrêt",
        (ContainerStatus::Unknown, _) => "Inconnu",
    }
}
