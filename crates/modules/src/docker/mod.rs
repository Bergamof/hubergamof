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
//!   page of variants), then its containers.
//! - Container page: start, stop, restart, and remove when stopped.
//! - Removing and every `down` need a second press within
//!   [`CONFIRM_TIMEOUT`].
//!
//! Docker is reached through a [`DockerBackend`]: the Docker Engine of the
//! session ([`DockerEngine`]) or an in-memory host ([`SimulatedDocker`],
//! `--demo` and tests), listened to once for every instance through a hub
//! (module `hub`), which also runs the actions. Handlers never wait for
//! Docker: actions run in the background (several at once), the key shows
//! the action in progress, then ✓ or ⚠; their effect comes back as changes
//! of the containers. Everything is sorted by name, so that keys keep
//! their place.

mod backend;
mod compose;
mod engine;
mod hub;
mod icons;
mod model;
mod simulated;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hubergamof_module_api::{
    Command, EventSender, Feedback, ImageData, ItemId, KeyContent, KeyState, Module,
    ModuleCapabilities, ModuleContext, ModuleDescriptor, ModuleError, ModuleEvent, ModuleFactory,
    ModuleSurface, PageContent, PageId, PageItem, SettingField, SettingKind, TaskSpawner,
};
use serde::Deserialize;
use serde_json::Value;

pub use backend::{
    ComposeAction, ContainerAction, DockerBackend, DockerChanges, DockerError, UpVariant,
};
pub use engine::DockerEngine;
use hub::{DockerHub, DockerView, Request, Subscription};
pub use model::{ComposeLabels, ComposeTarget, Container, ContainerStatus, Health};
use model::{Overview, Project};
pub use simulated::SimulatedDocker;

use crate::background::forward_states;
use crate::paths::expand_user_home;

/// Kind of the Docker module.
pub const DOCKER_KIND: &str = "docker";

/// Time given for the second press confirming a removal or a `down`.
pub(crate) const CONFIRM_TIMEOUT: Duration = Duration::from_secs(3);

const COMPOSE_FILES_KEY: &str = "composeFiles";

/// Page opened by the home key.
const MAIN_PAGE: &str = "docker";
/// Prefixes of the pages (and of the items opening them) of a project, of
/// its `up` and `down` variants, of a container; followed by its name.
const PROJECT_PREFIX: &str = "project/";
const UP_PREFIX: &str = "up/";
const DOWN_PREFIX: &str = "down/";
const CONTAINER_PREFIX: &str = "container/";
/// Items of a project page opening the pages of its variants.
const UP_ITEM: &str = "up";
const DOWN_ITEM: &str = "down";

const MODULE_LABEL: &str = "Docker";
const UP_LABEL: &str = "Up";
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
        capabilities: ModuleCapabilities {
            key: true,
            dial: false,
            touch_zone: false,
        },
        settings: vec![SettingField {
            key: COMPOSE_FILES_KEY.into(),
            label: "Fichiers compose".into(),
            help: Some(
                "Fichiers compose.yaml (chemin absolu, ou commençant par ~/), proposés \
                 même quand le projet est arrêté (après un down)."
                    .into(),
            ),
            kind: SettingKind::PathList {
                default: Vec::new(),
            },
        }],
    }
}

impl ModuleFactory for DockerFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor()
    }

    fn icon(&self) -> Option<ImageData> {
        Some(icons::MODULE.clone())
    }

    fn validate_settings(&self, settings: &Value) -> Result<(), ModuleError> {
        DockerSettings::parse(settings).map(drop)
    }

    fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
        let settings = DockerSettings::parse(settings)?;
        let hub = self.hub.subscribe();
        ctx.tasks().spawn(forward_states(
            hub.states(),
            ctx.events().clone(),
            DockerEvent::View,
        ));
        Ok(Box::new(Docker {
            settings,
            view: View::Loading,
            hub,
            events: ctx.events().clone(),
            tasks: ctx.tasks().clone(),
            busy: HashSet::new(),
            confirming: None,
            next_token: 0,
        }))
    }
}

/// Settings of a Docker instance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct DockerSettings {
    /// Compose files offered even when their project is down.
    compose_files: Vec<PathBuf>,
}

impl DockerSettings {
    /// Checks the types, reads them, and makes every path absolute: `~/`
    /// stands for the home directory, any other path must be absolute
    /// (compose files are given to `docker compose`, run from the manager's
    /// directory).
    fn parse(settings: &Value) -> Result<Self, ModuleError> {
        let parsed: Self = descriptor().parse_settings(settings)?;
        let compose_files = parsed
            .compose_files
            .iter()
            .map(|file| {
                expand_user_home(file)
                    .filter(|file| file.is_absolute())
                    .ok_or_else(|| {
                        ModuleError::InvalidSettings(format!(
                            "`{COMPOSE_FILES_KEY}`: `{}` is not an absolute path",
                            file.display()
                        ))
                    })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { compose_files })
    }
}

/// A page of the module, from its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page<'a> {
    Main,
    Project(&'a str),
    Up(&'a str),
    Down(&'a str),
    Container(&'a str),
}

impl<'a> Page<'a> {
    fn parse(id: &'a str) -> Option<Self> {
        if id == MAIN_PAGE {
            return Some(Self::Main);
        }
        let named = [
            (PROJECT_PREFIX, Self::Project as fn(&'a str) -> Self),
            (UP_PREFIX, Self::Up),
            (DOWN_PREFIX, Self::Down),
            (CONTAINER_PREFIX, Self::Container),
        ];
        named
            .into_iter()
            .find_map(|(prefix, page)| id.strip_prefix(prefix).map(page))
    }

    /// The page opened by `item` of this page, if it opens one.
    fn opened_by(self, item: &str) -> Option<PageId> {
        let opens_child = item.starts_with(PROJECT_PREFIX) || item.starts_with(CONTAINER_PREFIX);
        match self {
            Self::Main if opens_child => Some(item.into()),
            Self::Project(_) if item.starts_with(CONTAINER_PREFIX) => Some(item.into()),
            Self::Project(name) if item == UP_ITEM => Some(format!("{UP_PREFIX}{name}").into()),
            Self::Project(name) if item == DOWN_ITEM => Some(format!("{DOWN_PREFIX}{name}").into()),
            _ => None,
        }
    }
}

fn project_id(name: &str) -> String {
    format!("{PROJECT_PREFIX}{name}")
}

fn container_id(name: &str) -> String {
    format!("{CONTAINER_PREFIX}{name}")
}

/// What an action key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    StartAll,
    StopAll,
    Start,
    Stop,
    Restart,
    Remove,
    Up(UpVariant),
    Down { volumes: bool },
}

const PROJECT_ACTIONS: [Action; 2] = [Action::StartAll, Action::StopAll];
const UP_ACTIONS: [Action; 3] = [
    Action::Up(UpVariant::Plain),
    Action::Up(UpVariant::Build),
    Action::Up(UpVariant::Pull),
];
const DOWN_ACTIONS: [Action; 2] = [
    Action::Down { volumes: false },
    Action::Down { volumes: true },
];
/// Actions of a container page; [`Action::Remove`] shown only when the
/// container may be removed.
const CONTAINER_ACTIONS: [Action; 4] =
    [Action::Start, Action::Stop, Action::Restart, Action::Remove];

impl Action {
    fn item(self) -> &'static str {
        match self {
            Self::StartAll => "start-all",
            Self::StopAll => "stop-all",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Remove => "remove",
            Self::Up(UpVariant::Plain) => "up-plain",
            Self::Up(UpVariant::Build) => "up-build",
            Self::Up(UpVariant::Pull) => "up-pull",
            Self::Down { volumes: false } => "down-keep",
            Self::Down { volumes: true } => "down-volumes",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::StartAll => "Tout démarrer",
            Self::StopAll => "Tout arrêter",
            Self::Start => "Démarrer",
            Self::Stop => "Arrêter",
            Self::Restart => "Redémarrer",
            Self::Remove => "Supprimer",
            Self::Up(UpVariant::Plain) => UP_LABEL,
            Self::Up(UpVariant::Build) => "Up + build",
            Self::Up(UpVariant::Pull) => "Up + pull",
            Self::Down { volumes: false } => DOWN_LABEL,
            Self::Down { volumes: true } => "Down + volumes",
        }
    }

    /// Label while it runs.
    fn busy_label(self) -> &'static str {
        match self {
            Self::StartAll | Self::Start => "Démarrage…",
            Self::StopAll | Self::Stop => "Arrêt…",
            Self::Restart => "Redémarrage…",
            Self::Remove => "Suppression…",
            Self::Up(_) => "Up…",
            Self::Down { .. } => "Down…",
        }
    }

    fn icon(self) -> &'static ImageData {
        match self {
            Self::StartAll | Self::Start => &icons::START,
            Self::StopAll | Self::Stop => &icons::STOP,
            Self::Restart => &icons::RESTART,
            Self::Remove => &icons::REMOVE,
            Self::Up(_) => &icons::UP,
            Self::Down { .. } => &icons::DOWN,
        }
    }

    /// Destroys something: needs a second press.
    fn needs_confirmation(self) -> bool {
        matches!(self, Self::Remove | Self::Down { .. })
    }

    /// Destroys data: shown in attention.
    fn is_destructive(self) -> bool {
        self == Self::Down { volumes: true }
    }

    /// The actions of `page`.
    fn of_page(page: Page<'_>) -> &'static [Self] {
        match page {
            Page::Main => &[],
            Page::Project(_) => &PROJECT_ACTIONS,
            Page::Up(_) => &UP_ACTIONS,
            Page::Down(_) => &DOWN_ACTIONS,
            Page::Container(_) => &CONTAINER_ACTIONS,
        }
    }

    /// The action of `item` on `page`.
    fn of(page: Page<'_>, item: &str) -> Option<Self> {
        Self::of_page(page)
            .iter()
            .copied()
            .find(|action| action.item() == item)
    }
}

/// What an action amounts to on the current containers.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Plan {
    /// Already done (nothing to start…).
    Nothing,
    Run(Request),
    /// Cannot be done: project or container gone, not removable…
    Refused,
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
    settings: DockerSettings,
    view: View,
    hub: Subscription,
    events: EventSender,
    tasks: TaskSpawner,
    /// Action keys whose action runs.
    busy: HashSet<ItemKey>,
    confirming: Option<Confirmation>,
    next_token: u64,
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
                overview: Overview::new(&containers, &self.settings.compose_files),
                containers,
            },
        };
    }

    fn handle(&mut self, event: DockerEvent) -> Vec<Command> {
        match event {
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

    fn perform(&mut self, key: ItemKey, page: Page<'_>, action: Action) -> Vec<Command> {
        if self.busy.contains(&key) {
            return Vec::new();
        }
        let request = match self.plan(page, action) {
            Plan::Run(request) => request,
            Plan::Nothing => return vec![key.feedback(Feedback::Ok)],
            Plan::Refused => return Command::alert(),
        };
        if action.needs_confirmation() && !self.confirmed(&key) {
            return Vec::new();
        }
        let events = self.events.clone();
        let outcome_key = key.clone();
        let started = self.hub.run(request, move |outcome| {
            if let Err(error) = &outcome {
                tracing::warn!(%error, "Docker action failed");
            }
            // An instance gone meanwhile needs no outcome.
            let _ = events.send(DockerEvent::Outcome {
                key: outcome_key,
                ok: outcome.is_ok(),
            });
        });
        if started {
            self.busy.insert(key);
            Vec::new()
        } else {
            Command::alert()
        }
    }

    /// What `action` on `page` amounts to on the containers shown.
    fn plan(&self, page: Page<'_>, action: Action) -> Plan {
        let Some(overview) = self.overview() else {
            return Plan::Refused;
        };
        let compose = |name: &str, action| {
            let target = overview.project(name).and_then(|p| p.target.clone());
            target.map_or(Plan::Refused, |target| {
                Plan::Run(Request::Compose { target, action })
            })
        };
        match (page, action) {
            (Page::Project(name), Action::StartAll | Action::StopAll) => {
                let Some(project) = overview.project(name) else {
                    return Plan::Refused;
                };
                let start = action == Action::StartAll;
                let ids: Vec<String> = project
                    .containers
                    .iter()
                    .filter(|c| c.is_running() != start)
                    .map(|c| c.id.clone())
                    .collect();
                if ids.is_empty() {
                    return Plan::Nothing;
                }
                let action = if start {
                    ContainerAction::Start
                } else {
                    ContainerAction::Stop
                };
                Plan::Run(Request::Containers { ids, action })
            }
            (Page::Up(name), Action::Up(variant)) => compose(name, ComposeAction::Up(variant)),
            (Page::Down(name), Action::Down { volumes }) => {
                compose(name, ComposeAction::Down { volumes })
            }
            (Page::Container(name), _) => {
                let Some(container) = overview.container(name) else {
                    return Plan::Refused;
                };
                let action = match action {
                    Action::Start => ContainerAction::Start,
                    Action::Stop => ContainerAction::Stop,
                    Action::Restart => ContainerAction::Restart,
                    Action::Remove if container.is_removable() => ContainerAction::Remove,
                    _ => return Plan::Refused,
                };
                Plan::Run(Request::Containers {
                    ids: vec![container.id.clone()],
                    action,
                })
            }
            _ => Plan::Refused,
        }
    }

    fn action_key(&self, page: &PageId, action: Action) -> PageItem {
        let key = ItemKey::new(page, &action.item().into());
        let (label, state) = if self.busy.contains(&key) {
            (action.busy_label(), KeyState::Normal)
        } else if self.is_confirming(&key) {
            (CONFIRM_LABEL, KeyState::Attention)
        } else if action.is_destructive() {
            (action.label(), KeyState::Attention)
        } else {
            (action.label(), KeyState::Normal)
        };
        PageItem {
            id: key.item,
            key: KeyContent::with_icon(action.icon(), label).in_state(state),
        }
    }

    fn project_page(&self, page: &PageId, project: &Project) -> PageContent {
        let actions = PROJECT_ACTIONS
            .iter()
            .map(|action| self.action_key(page, *action));
        let variants = [
            (UP_ITEM, UP_LABEL, &*icons::UP),
            (DOWN_ITEM, DOWN_LABEL, &*icons::DOWN),
        ]
        .map(|(id, label, icon)| PageItem {
            id: id.into(),
            key: KeyContent::with_icon(icon, label),
        });
        let prefix = format!("{}-", project.name);
        let containers = project.containers.iter().map(|container| {
            let short = container
                .name
                .strip_prefix(&prefix)
                .unwrap_or(&container.name);
            container_item(container, short)
        });
        PageContent {
            title: project.name.clone(),
            items: actions.chain(variants).chain(containers).collect(),
        }
    }

    fn variants_page(&self, page: &PageId, title: String, actions: &[Action]) -> PageContent {
        PageContent {
            title,
            items: actions
                .iter()
                .map(|action| self.action_key(page, *action))
                .collect(),
        }
    }

    fn container_page(&self, page: &PageId, container: &Container) -> PageContent {
        PageContent {
            title: format!(
                "{}{TITLE_SEPARATOR}{}",
                container.name,
                status_label(container)
            ),
            items: CONTAINER_ACTIONS
                .iter()
                .filter(|action| **action != Action::Remove || container.is_removable())
                .map(|action| self.action_key(page, *action))
                .collect(),
        }
    }
}

/// The projects, then the other containers.
fn main_page(overview: &Overview) -> PageContent {
    let projects = overview.projects.iter().map(|project| {
        let (icon, state) = if project.needs_attention() {
            (&icons::PROJECT_ATTENTION, KeyState::Attention)
        } else if project.running() > 0 {
            (&icons::PROJECT_RUNNING, KeyState::Active)
        } else {
            (&icons::PROJECT_STOPPED, KeyState::Normal)
        };
        let label = format!(
            "{} {}/{}",
            project.name,
            project.running(),
            project.containers.len()
        );
        PageItem {
            id: project_id(&project.name).into(),
            key: KeyContent::with_icon(icon, &label).in_state(state),
        }
    });
    let containers = overview
        .isolated
        .iter()
        .map(|container| container_item(container, &container.name));
    PageContent {
        title: MODULE_LABEL.into(),
        items: projects.chain(containers).collect(),
    }
}

fn variants_title(label: &str, project: &Project) -> String {
    format!("{label}{TITLE_SEPARATOR}{}", project.name)
}

/// Key of a container, labelled `label`, opening its page.
fn container_item(container: &Container, label: &str) -> PageItem {
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

fn status_label(container: &Container) -> &'static str {
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
            Page::Main => Some(main_page(overview)),
            Page::Project(name) => overview
                .project(name)
                .map(|project| self.project_page(page_id, project)),
            Page::Up(name) => overview.project(name).map(|project| {
                self.variants_page(page_id, variants_title(UP_LABEL, project), &UP_ACTIONS)
            }),
            Page::Down(name) => overview.project(name).map(|project| {
                self.variants_page(page_id, variants_title(DOWN_LABEL, project), &DOWN_ACTIONS)
            }),
            Page::Container(name) => overview
                .container(name)
                .map(|container| self.container_page(page_id, container)),
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
        if let Some(opened) = page.opened_by(&item.0) {
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
        self.settings = DockerSettings::parse(settings)?;
        if let View::Ready { containers, .. } = &self.view {
            let containers = Arc::clone(containers);
            self.show(DockerView::Ready(containers));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
