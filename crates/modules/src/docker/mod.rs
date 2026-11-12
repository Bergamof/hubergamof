//! Docker module: the compose projects and containers of the Docker host,
//! started, stopped, restarted, removed, brought up or down from the keys.
//!
//! - Home key: running containers out of all (`Docker 3/5`), in attention
//!   when one is in trouble; opens the main page.
//! - Main page: the compose projects (found from the labels of their
//!   containers, plus the compose files declared in the settings, so that
//!   a project taken down can be brought up again), then the other
//!   containers.
//! - Project page: start all, stop all, `up` and `down` (each opening a
//!   page of variants), then its containers and, drawn apart, the services
//!   its files define that have no container yet (read with `docker compose
//!   config --services`, in the background).
//! - Container page: start, stop, restart, remove when stopped, and for a
//!   compose container its own `up` and `down`, which work on its service
//!   only. A service without container has a page with `up` alone.
//! - Removing and every `down` need a second press within
//!   [`CONFIRM_TIMEOUT`]. Pressing a variant of `up` or `down` goes back to
//!   the page of the project, where the entries the operation works on show
//!   what it does (`Build…`, `Pull…`, `Up…`, `Down…`).
//!
//! Docker is reached through a [`DockerBackend`]: the Docker Engine of the
//! session ([`DockerEngine`]) or an in-memory host ([`SimulatedDocker`],
//! `--demo` and tests), listened to once for every instance through a hub
//! (module `hub`), which also runs the actions. Handlers never wait for
//! Docker: actions run in the background (several at once), the key shows
//! the action in progress, then ✓ or ⚠; their effect comes back as changes
//! of the containers. Everything is sorted by name, so that keys keep
//! their place.

mod actions;
mod backend;
mod compose;
mod engine;
mod hub;
mod icons;
mod model;
mod operation;
mod pages;
mod render;
mod settings;
mod simulated;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hubergamof_module_api::{
    Command, EventSender, Feedback, ImageData, ImageLibrary, ItemId, KeyContent, KeyState, Module,
    ModuleCapabilities, ModuleContext, ModuleDescriptor, ModuleError, ModuleEvent, ModuleFactory,
    ModuleSurface, PageContent, PageId, PageItem, TaskSpawner,
};
use serde_json::Value;

pub use backend::{
    ComposeAction, ComposeScope, ContainerAction, DockerBackend, DockerChanges, DockerError,
    UpVariant,
};
pub use engine::DockerEngine;
use hub::{DockerHub, DockerView, Request, Subscription};
pub use model::{ComposeLabels, ComposeTarget, Container, ContainerStatus, Health};
use model::{Overview, Project};
pub use simulated::SimulatedDocker;

use actions::{Action, UP_ACTIONS};
use operation::{Operation, OperationKind};
use pages::{
    DOWN_ITEM, MAIN_PAGE, PROJECT_PREFIX, Page, Scope, UP_ITEM, container_id, project_id,
    service_id,
};
use render::{main_page, variants_title};
use settings::{COMPOSE_FILES_KEY, ParsedSettings};

use crate::background::forward_states;

/// Kind of the Docker module.
pub const DOCKER_KIND: &str = "docker";

/// Time given for the second press confirming a removal or a `down`.
pub(crate) const CONFIRM_TIMEOUT: Duration = Duration::from_secs(3);

const MODULE_LABEL: &str = "Docker";
const UP_LABEL: &str = "Up";
const NOT_BUILT_LABEL: &str = "Non créé";
const DOWN_LABEL: &str = "Down";
const UNAVAILABLE_TEXT: &str = "Docker indisponible";
const CONFIRM_LABEL: &str = "Confirmer ?";
/// Separates a name from its state in page titles.
const TITLE_SEPARATOR: &str = " · ";

/// Creates [`Docker`] instances, all following the same Docker host through
/// one hub.
#[derive(Debug, Clone)]
pub struct DockerFactory {
    hub: Arc<DockerHub>,
}

impl DockerFactory {
    /// Factory driving `backend`.
    #[must_use]
    pub fn new(backend: Arc<dyn DockerBackend>) -> Self {
        Self {
            hub: DockerHub::new(backend),
        }
    }
}

/// Kind, capabilities and settings of the Docker module.
fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        kind: DOCKER_KIND.into(),
        name: "Docker".into(),
        description: "Projets compose et conteneurs : démarrer, arrêter, redémarrer, \
                      supprimer, up et down."
            .into(),
        capabilities: ModuleCapabilities { key: true },
        dial_functions: Vec::new(),
        settings: vec![settings::compose_files_field()],
    }
}

impl ModuleFactory for DockerFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor()
    }

    fn icon(&self) -> Option<ImageData> {
        Some(icons::MODULE.clone())
    }

    fn setting_item_icon(&self, key: &str) -> Option<ImageData> {
        // The layers of a project that has no image of its own.
        (key == COMPOSE_FILES_KEY).then(|| icons::PROJECT_STOPPED.clone())
    }

    fn validate_settings(&self, settings: &Value) -> Result<(), ModuleError> {
        ParsedSettings::parse(settings).map(drop)
    }

    fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
        let settings = ParsedSettings::parse(settings)?;
        let images = ctx.images().clone();
        let hub = self.hub.subscribe();
        ctx.tasks().spawn(forward_states(
            hub.states(),
            ctx.events().clone(),
            DockerEvent::View,
        ));
        let mut docker = Docker {
            settings,
            view: View::Loading,
            hub,
            events: ctx.events().clone(),
            tasks: ctx.tasks().clone(),
            images,
            icons: HashMap::new(),
            busy: HashSet::new(),
            confirming: None,
            next_token: 0,
            service_names: HashMap::new(),
            services_requested: HashSet::new(),
            operations: Vec::new(),
            next_operation: 0,
        };
        docker.load_missing_icons();
        Ok(Box::new(docker))
    }
}

/// An item of a page.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ItemKey {
    page: PageId,
    item: ItemId,
}

impl ItemKey {
    fn new(page: &PageId, item: &ItemId) -> Self {
        Self {
            page: page.clone(),
            item: item.clone(),
        }
    }

    fn feedback(self, feedback: Feedback) -> Command {
        Command::FeedbackOn(
            ModuleSurface::Item {
                page: self.page,
                item: self.item,
            },
            feedback,
        )
    }
}

/// What an instance receives through its event sender.
#[derive(Debug)]
enum DockerEvent {
    /// A state published by the hub.
    View(DockerView),
    /// The action of `key` ended.
    Outcome { key: ItemKey, ok: bool },
    /// The confirmation `token` is over.
    ConfirmExpired(u64),
    /// Icons read from the stored images, by name.
    Icons(HashMap<String, ImageData>),
    /// The services the files of `project` define; `None`: they could not be read.
    Services {
        project: String,
        services: Option<Vec<String>>,
    },
    /// The operation `id` ended.
    OperationEnded { id: u64, ok: bool },
}

/// A key pressed once, waiting for the second press.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Confirmation {
    key: ItemKey,
    token: u64,
}

/// What the instance knows of the containers.
#[derive(Debug, Clone)]
enum View {
    Loading,
    Unavailable,
    Ready {
        containers: Arc<[Container]>,
        /// `containers` grouped with the declared files.
        overview: Overview,
    },
}

/// One instance: the containers shown and its access to the hub.
pub struct Docker {
    settings: ParsedSettings,
    view: View,
    hub: Subscription,
    events: EventSender,
    tasks: TaskSpawner,
    images: ImageLibrary,
    /// The icons the user gave to compose files, once read: a project whose
    /// icon is not (yet) here shows the default one.
    icons: HashMap<String, ImageData>,
    /// Action keys whose action runs.
    busy: HashSet<ItemKey>,
    confirming: Option<Confirmation>,
    next_token: u64,
    /// The services each project's files define, by project name, once read.
    service_names: HashMap<String, Vec<String>>,
    /// Projects whose services were asked for (read or being read).
    services_requested: HashSet<String>,
    /// The `up` and `down` running, shown on the entries they work on.
    operations: Vec<Operation>,
    next_operation: u64,
}

impl Docker {
    fn overview(&self) -> Option<&Overview> {
        match &self.view {
            View::Ready { overview, .. } => Some(overview),
            View::Loading | View::Unavailable => None,
        }
    }

    fn show(&mut self, view: DockerView) {
        self.view = match view {
            DockerView::Loading => View::Loading,
            DockerView::Unavailable => View::Unavailable,
            DockerView::Ready(containers) => View::Ready {
                overview: Overview::new(
                    &containers,
                    &self.settings.compose_files,
                    &self.service_names,
                ),
                containers,
            },
        };
        self.forget_gone_projects();
        self.request_declared_services();
    }

    /// Lists again what is known of the containers, with the services now read.
    fn reshow(&mut self) {
        if let View::Ready { containers, .. } = &self.view {
            let containers = Arc::clone(containers);
            self.show(DockerView::Ready(containers));
        }
    }

    /// Reads in the background the services of the projects of the settings
    /// not asked for yet: those the user chose, not whatever labels name.
    fn request_declared_services(&mut self) {
        let declared: HashSet<&PathBuf> = self
            .settings
            .compose_files
            .iter()
            .map(|file| &file.path)
            .collect();
        let Some(overview) = self.overview() else {
            return;
        };
        let wanted: Vec<String> = overview
            .projects
            .iter()
            .filter(|project| {
                project
                    .target
                    .as_ref()
                    .is_some_and(|target| target.files.iter().any(|file| declared.contains(file)))
            })
            .map(|project| project.name.clone())
            .collect();
        for project in wanted {
            self.request_services(&project);
        }
    }

    /// Reads in the background the services of `project`, if not asked for yet.
    fn request_services(&mut self, project: &str) {
        let Some(target) = self
            .overview()
            .and_then(|overview| overview.project(project)?.target.clone())
        else {
            return;
        };
        if !self.services_requested.insert(project.to_owned()) {
            return;
        }
        let project = project.to_owned();
        let events = self.events.clone();
        self.hub.services(target, move |outcome| {
            if let Err(error) = &outcome {
                tracing::warn!(%error, "cannot read the services of a compose project");
            }
            let _ = events.send(DockerEvent::Services {
                project,
                services: outcome.ok(),
            });
        });
    }

    /// Reads the services of `project` again, its files may have changed:
    /// when the user opens it, wherever its files were named.
    fn refresh_services(&mut self, project: &str) {
        self.services_requested.remove(project);
        self.request_services(project);
    }

    /// Forgets the services of the projects no longer listed.
    fn forget_gone_projects(&mut self) {
        let Some(overview) = self.overview() else {
            return;
        };
        let listed: HashSet<String> = overview.projects.iter().map(|p| p.name.clone()).collect();
        self.service_names.retain(|name, _| listed.contains(name));
        self.services_requested.retain(|name| listed.contains(name));
    }

    /// Reads in the background the icons of the settings not read yet.
    fn load_missing_icons(&mut self) {
        let missing: Vec<String> = self
            .settings
            .icon_names()
            .filter(|name| !self.icons.contains_key(*name))
            .map(str::to_owned)
            .collect();
        if missing.is_empty() {
            return;
        }
        let images = self.images.clone();
        let events = self.events.clone();
        self.tasks.spawn(async move {
            let mut loaded = HashMap::new();
            for name in missing {
                if let Some(image) = images.load(&name).await {
                    loaded.insert(name, image);
                }
            }
            events.send(DockerEvent::Icons(loaded));
        });
    }

    fn handle(&mut self, event: DockerEvent) -> Vec<Command> {
        match event {
            DockerEvent::Services { project, services } => {
                if let Some(services) = services {
                    self.service_names.insert(project, services);
                    self.reshow();
                }
                Vec::new()
            }
            DockerEvent::OperationEnded { id, ok } => {
                let Some(position) = self.operations.iter().position(|op| op.id == id) else {
                    return Vec::new();
                };
                let operation = self.operations.remove(position);
                // The page of the project is where the operation was left running: the
                // outcome shows on what it worked on.
                let project_page: PageId = project_id(&operation.project).into();
                self.outcome_items(&operation)
                    .into_iter()
                    .map(|item| {
                        ItemKey::new(&project_page, &item.into()).feedback(Feedback::of(ok))
                    })
                    .collect()
            }
            DockerEvent::Icons(loaded) => {
                // Settings may have changed while the images were read.
                let wanted: HashSet<&str> = self.settings.icon_names().collect();
                self.icons.extend(
                    loaded
                        .into_iter()
                        .filter(|(name, _)| wanted.contains(name.as_str())),
                );
                Vec::new()
            }
            DockerEvent::View(view) => {
                self.show(view);
                Vec::new()
            }
            DockerEvent::Outcome { key, ok } => {
                self.busy.remove(&key);
                let feedback = Feedback::of(ok);
                vec![key.feedback(feedback)]
            }
            DockerEvent::ConfirmExpired(token) => {
                if self
                    .confirming
                    .as_ref()
                    .is_some_and(|confirming| confirming.token == token)
                {
                    self.confirming = None;
                }
                Vec::new()
            }
        }
    }

    /// The items of the page of its project an operation marks when it ends:
    /// the key of its kind when it was about the whole project, else its
    /// service, as the entry it is at that moment (a container, or a service
    /// without one; the list of containers may not be up to date yet, so
    /// both are named, the one that is not shown is ignored).
    fn outcome_items(&self, operation: &Operation) -> Vec<String> {
        let Some(service) = operation.scope.service() else {
            let key = match operation.kind {
                OperationKind::Up(_) => UP_ITEM,
                OperationKind::Down => DOWN_ITEM,
            };
            return vec![key.to_owned()];
        };
        let container = self
            .overview()
            .and_then(|overview| overview.project(&operation.project))
            .and_then(|project| {
                project.containers.iter().find(|c| {
                    c.compose.as_ref().and_then(|l| l.service.as_deref()) == Some(service)
                })
            });
        container
            .map(|container| container_id(&container.name))
            .into_iter()
            .chain([service_id(&operation.project, service)])
            .collect()
    }

    fn is_confirming(&self, key: &ItemKey) -> bool {
        self.confirming
            .as_ref()
            .is_some_and(|confirming| confirming.key == *key)
    }

    /// Whether `key` was pressed once already, within the timeout; waits
    /// for the second press otherwise.
    fn confirmed(&mut self, key: &ItemKey) -> bool {
        if self.is_confirming(key) {
            self.confirming = None;
            return true;
        }
        self.next_token += 1;
        let token = self.next_token;
        let events = self.events.clone();
        let expiry = self.tasks.spawn(async move {
            tokio::time::sleep(CONFIRM_TIMEOUT).await;
            let _ = events.send(DockerEvent::ConfirmExpired(token));
        });
        self.confirming = expiry.then(|| Confirmation {
            key: key.clone(),
            token,
        });
        false
    }
}

#[async_trait]
impl Module for Docker {
    fn key(&self) -> KeyContent {
        let (label, state) = match &self.view {
            View::Unavailable => (UNAVAILABLE_TEXT.to_owned(), KeyState::Attention),
            View::Loading => (MODULE_LABEL.to_owned(), KeyState::Normal),
            View::Ready { overview, .. } => {
                let label = format!("{MODULE_LABEL} {}/{}", overview.running(), overview.total());
                let state = if overview.needs_attention() {
                    KeyState::Attention
                } else {
                    KeyState::Normal
                };
                (label, state)
            }
        };
        KeyContent::with_icon(&icons::MODULE, &label).in_state(state)
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        vec![Command::OpenPage(MAIN_PAGE.into())]
    }

    fn page(&self, page_id: &PageId) -> Option<PageContent> {
        let page = Page::parse(&page_id.0)?;
        let Some(overview) = self.overview() else {
            // Not listed (yet): the page stays, empty, until Docker answers.
            let title = match self.view {
                View::Unavailable => UNAVAILABLE_TEXT,
                View::Loading | View::Ready { .. } => MODULE_LABEL,
            };
            return Some(PageContent {
                title: title.into(),
                items: Vec::new(),
            });
        };
        // A project or container gone (removed, taken down): back.
        match page {
            Page::Main => Some(main_page(overview, &self.icons, &self.operations)),
            Page::Project(name) => overview
                .project(name)
                .map(|project| self.project_page(page_id, project)),
            Page::Up(scope) => overview.project(scope.project).map(|project| {
                let title = variants_title(UP_LABEL, project, scope.service);
                self.variants_page(page_id, title, &UP_ACTIONS)
            }),
            Page::Down(scope) => overview.project(scope.project).map(|project| {
                let title = variants_title(DOWN_LABEL, project, scope.service);
                self.variants_page(page_id, title, Action::of_page(page))
            }),
            Page::Container(name) => overview
                .container(name)
                .map(|container| self.container_page(page_id, container, overview)),
            Page::Service { project, service } => overview
                .project(project)
                .and_then(|project| Self::service_page(project, service)),
        }
    }

    async fn on_item_press(&mut self, page_id: &PageId, item: &ItemId) -> Vec<Command> {
        let Some(page) = Page::parse(&page_id.0) else {
            return Vec::new();
        };
        let key = ItemKey::new(page_id, item);
        // Any other key cancels a pending confirmation.
        if !self.is_confirming(&key) {
            self.confirming = None;
        }
        let opened = self
            .overview()
            .and_then(|overview| page.opened_by(&item.0, overview));
        if let Some(opened) = opened {
            // The files may have changed since the services were read.
            if page == Page::Main
                && let Some(project) = item.0.strip_prefix(PROJECT_PREFIX)
            {
                self.refresh_services(project);
            }
            return vec![Command::OpenPage(opened)];
        }
        match Action::of(page, &item.0) {
            Some(action) => self.perform(key, page, action),
            None => Vec::new(),
        }
    }

    async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
        match event.downcast::<DockerEvent>() {
            Ok(event) => self.handle(*event),
            Err(_) => Vec::new(),
        }
    }

    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError> {
        self.settings = ParsedSettings::parse(settings)?;
        // The files may be other ones: their services are read again.
        self.service_names.clear();
        self.services_requested.clear();
        let wanted: HashSet<&str> = self.settings.icon_names().collect();
        self.icons.retain(|name, _| wanted.contains(name.as_str()));
        self.load_missing_icons();
        self.reshow();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
