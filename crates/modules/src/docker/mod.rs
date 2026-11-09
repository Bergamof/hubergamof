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

mod backend;
mod compose;
mod engine;
mod hub;
mod icons;
mod model;
mod simulated;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hubergamof_module_api::{
    Command, EventSender, Feedback, FileEntry, ImageData, ImageLibrary, ItemId, KeyContent,
    KeyState, Module, ModuleCapabilities, ModuleContext, ModuleDescriptor, ModuleError,
    ModuleEvent, ModuleFactory, ModuleSurface, PageContent, PageId, PageItem, SettingField,
    SettingKind, TaskSpawner,
};
use serde::Deserialize;
use serde_json::Value;

pub use backend::{
    ComposeAction, ContainerAction, DockerBackend, DockerChanges, DockerError, UpVariant,
};
pub use engine::DockerEngine;
use hub::{DockerHub, DockerView, Request, Subscription};
pub use model::{ComposeLabels, ComposeTarget, Container, ContainerStatus, Health};
use model::{DeclaredFile, Overview, Project};
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
const SERVICE_PREFIX: &str = "service/";
/// Items of a project page opening the pages of its variants.
const UP_ITEM: &str = "up";
const DOWN_ITEM: &str = "down";

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
        capabilities: ModuleCapabilities {
            key: true,
            dial: false,
            touch_zone: false,
        },
        settings: vec![SettingField {
            key: COMPOSE_FILES_KEY.into(),
            label: "Fichiers compose".into(),
            help: Some(
                "Fichiers compose.yaml, avec un nom et une icône au choix, proposés \
                 même quand le projet est arrêté (après un down)."
                    .into(),
            ),
            kind: SettingKind::FileEntryList {
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

/// Settings of a Docker instance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct DockerSettings {
    /// Compose files offered even when their project is down.
    compose_files: Vec<ComposeFileSetting>,
}

/// A compose file as stored: a plain path (settings saved before files had
/// a name and an icon) or an entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum ComposeFileSetting {
    Path(PathBuf),
    Entry(FileEntry),
}

impl ComposeFileSetting {
    fn into_entry(self) -> FileEntry {
        match self {
            Self::Path(path) => FileEntry {
                path: path.to_string_lossy().into_owned(),
                name: None,
                icon: None,
            },
            Self::Entry(entry) => entry,
        }
    }
}

/// What an instance reads of its settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ParsedSettings {
    compose_files: Vec<DeclaredFile>,
}

impl ParsedSettings {
    /// Checks the types, reads them, and makes every path absolute: `~/`
    /// stands for the home directory, any other path must be absolute
    /// (compose files are given to `docker compose`, run from the manager's
    /// directory). A blank name counts as none.
    fn parse(settings: &Value) -> Result<Self, ModuleError> {
        let parsed: DockerSettings = descriptor().parse_settings(settings)?;
        let compose_files = parsed
            .compose_files
            .into_iter()
            .map(|file| {
                let entry = file.into_entry();
                let path = expand_user_home(entry.path.as_ref())
                    .filter(|file| file.is_absolute())
                    .ok_or_else(|| {
                        ModuleError::InvalidSettings(format!(
                            "`{COMPOSE_FILES_KEY}`: `{}` is not an absolute path",
                            entry.path
                        ))
                    })?;
                Ok(DeclaredFile {
                    path,
                    name: entry
                        .name
                        .map(|name| name.trim().to_owned())
                        .filter(|name| !name.is_empty()),
                    icon: entry.icon,
                })
            })
            .collect::<Result<_, ModuleError>>()?;
        Ok(Self { compose_files })
    }

    /// Names of the icons given to the files.
    fn icon_names(&self) -> impl Iterator<Item = &str> {
        self.compose_files
            .iter()
            .filter_map(|file| file.icon.as_deref())
    }
}

/// What an `up` or `down` page is about: a whole project, or one service of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Scope<'a> {
    project: &'a str,
    service: Option<&'a str>,
}

impl<'a> Scope<'a> {
    /// `project`, or `project/service`.
    fn parse(rest: &'a str) -> Self {
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

    /// Pages to go back through to reach the page of the project: the
    /// variants page of a service was opened from its own page.
    fn pages_to_project(self) -> usize {
        1 + usize::from(self.service.is_some())
    }
}

/// A page of the module, from its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page<'a> {
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
    fn parse(id: &'a str) -> Option<Self> {
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
    fn scope(self) -> Option<Scope<'a>> {
        match self {
            Self::Up(scope) | Self::Down(scope) => Some(scope),
            _ => None,
        }
    }

    /// The page opened by `item` of this page, if it opens one.
    fn opened_by(self, item: &str, overview: &Overview) -> Option<PageId> {
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

/// The project and service a compose container runs.
fn compose_service(container: &Container) -> Option<(&str, &str)> {
    let labels = container.compose.as_ref()?;
    Some((labels.project.as_str(), labels.service.as_deref()?))
}

fn project_id(name: &str) -> String {
    format!("{PROJECT_PREFIX}{name}")
}

fn container_id(name: &str) -> String {
    format!("{CONTAINER_PREFIX}{name}")
}

fn service_id(project: &str, service: &str) -> String {
    format!("{SERVICE_PREFIX}{project}/{service}")
}

fn scoped_id(prefix: &str, project: &str, service: Option<&str>) -> String {
    match service {
        Some(service) => format!("{prefix}{project}/{service}"),
        None => format!("{prefix}{project}"),
    }
}

fn up_id(project: &str, service: Option<&str>) -> String {
    scoped_id(UP_PREFIX, project, service)
}

fn down_id(project: &str, service: Option<&str>) -> String {
    scoped_id(DOWN_PREFIX, project, service)
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
const UP_ACTIONS: [Action; 4] = [
    Action::Up(UpVariant::Plain),
    Action::Up(UpVariant::Build),
    Action::Up(UpVariant::Pull),
    Action::Up(UpVariant::NoStart),
];
const DOWN_ACTIONS: [Action; 2] = [
    Action::Down { volumes: false },
    Action::Down { volumes: true },
];
/// The `down` of one service: never with the volumes, which compose
/// removes for the whole project whatever the service named.
const SERVICE_DOWN_ACTIONS: [Action; 1] = [Action::Down { volumes: false }];
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
            Self::Up(UpVariant::NoStart) => "up-no-start",
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
            Self::Up(UpVariant::Build) => "Rebuild",
            Self::Up(UpVariant::Pull) => "Pull",
            Self::Up(UpVariant::NoStart) => "No start",
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
            Self::Up(_) | Self::Down { .. } => {
                self.operation().map_or("", OperationKind::working_label)
            }
        }
    }

    /// The operation of a project (or of one of its services) it starts.
    fn operation(self) -> Option<OperationKind> {
        match self {
            Self::Up(variant) => Some(OperationKind::Up(variant)),
            Self::Down { .. } => Some(OperationKind::Down),
            _ => None,
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
            Page::Main | Page::Service { .. } => &[],
            Page::Project(_) => &PROJECT_ACTIONS,
            Page::Up(_) => &UP_ACTIONS,
            Page::Down(Scope {
                service: Some(_), ..
            }) => &SERVICE_DOWN_ACTIONS,
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

/// What an operation does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperationKind {
    Up(UpVariant),
    Down,
}

impl OperationKind {
    /// What the entries it works on say while it runs.
    fn working_label(self) -> &'static str {
        match self {
            Self::Up(UpVariant::Build) => "Build…",
            Self::Up(UpVariant::Pull) => "Pull…",
            Self::Up(_) => "Up…",
            Self::Down => "Down…",
        }
    }
}

/// An `up` or a `down` of a project, or of some of its services, under way.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Operation {
    id: u64,
    project: String,
    /// Its services; empty: the whole project.
    services: Vec<String>,
    kind: OperationKind,
}

impl Operation {
    /// Whether it works on `service` of its project (`None`: a container
    /// that records no service, only reached by a whole-project operation).
    fn covers(&self, service: Option<&str>) -> bool {
        self.services.is_empty()
            || service.is_some_and(|service| self.services.iter().any(|name| name == service))
    }
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
                // The project page is where the operation was left running.
                let item = match operation.kind {
                    OperationKind::Up(_) => UP_ITEM,
                    OperationKind::Down => DOWN_ITEM,
                };
                let key = ItemKey::new(&project_id(&operation.project).into(), &item.into());
                vec![key.feedback(Feedback::of(ok))]
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
        if let (Some(scope), Some(kind)) = (page.scope(), action.operation()) {
            return self.start_operation(scope, kind, request);
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

    /// Starts an `up` or a `down`, and leaves its page for the one of the
    /// project, where what it works on shows.
    fn start_operation(
        &mut self,
        scope: Scope<'_>,
        kind: OperationKind,
        request: Request,
    ) -> Vec<Command> {
        if self.operations.iter().any(|op| op.project == scope.project) {
            return Command::alert();
        }
        self.next_operation += 1;
        let id = self.next_operation;
        let events = self.events.clone();
        let started = self.hub.run(request, move |outcome| {
            if let Err(error) = &outcome {
                tracing::warn!(%error, "Docker action failed");
            }
            let _ = events.send(DockerEvent::OperationEnded {
                id,
                ok: outcome.is_ok(),
            });
        });
        if !started {
            return Command::alert();
        }
        self.operations.push(Operation {
            id,
            project: scope.project.to_owned(),
            services: scope.service.map(str::to_owned).into_iter().collect(),
            kind,
        });
        vec![Command::Back; scope.pages_to_project()]
    }

    /// What `action` on `page` amounts to on the containers shown.
    fn plan(&self, page: Page<'_>, action: Action) -> Plan {
        let Some(overview) = self.overview() else {
            return Plan::Refused;
        };
        let compose = |scope: Scope<'_>, action| {
            let target = overview
                .project(scope.project)
                .and_then(|p| p.target.clone());
            target.map_or(Plan::Refused, |target| {
                let target = match scope.service {
                    Some(service) => target.with_services(vec![service.to_owned()]),
                    None => target,
                };
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
            (Page::Up(scope), Action::Up(variant)) => compose(scope, ComposeAction::Up(variant)),
            (Page::Down(scope), Action::Down { volumes }) => {
                compose(scope, ComposeAction::Down { volumes })
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
        let variants = [up_item(), down_item()];
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
            let item = self.working(project, service, container_id(&container.name), short);
            (
                short,
                item.unwrap_or_else(|| container_item(container, short)),
            )
        });
        let planned = project.planned.iter().map(|service| {
            let item = self.working(
                project,
                Some(service),
                service_id(&project.name, service),
                service,
            );
            (
                service.as_str(),
                item.unwrap_or_else(|| planned_item(&project.name, service)),
            )
        });
        let mut entries: Vec<(&str, PageItem)> = built.chain(planned).collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        PageContent {
            title: project.shown_name().to_owned(),
            items: actions
                .chain(variants)
                .chain(entries.into_iter().map(|(_, item)| item))
                .collect(),
        }
    }

    /// The key of an entry of `project` (a container, or a service without
    /// one) that an operation is working on: its name and what is done.
    fn working(
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

    fn variants_page(&self, page: &PageId, title: String, actions: &[Action]) -> PageContent {
        PageContent {
            title,
            items: actions
                .iter()
                .map(|action| self.action_key(page, *action))
                .collect(),
        }
    }

    fn container_page(
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
        PageContent {
            title: format!(
                "{}{TITLE_SEPARATOR}{}",
                container.name,
                status_label(container)
            ),
            items: actions.chain(scoped).collect(),
        }
    }

    /// The page of a service no container runs: it can only be brought up.
    fn service_page(project: &Project, service: &str) -> Option<PageContent> {
        project
            .planned
            .iter()
            .any(|name| name == service)
            .then(|| PageContent {
                title: format!("{service}{TITLE_SEPARATOR}{NOT_BUILT_LABEL}"),
                items: project
                    .target
                    .as_ref()
                    .map(|_| vec![up_item()])
                    .unwrap_or_default(),
            })
    }
}

/// The key opening the page of the variants of `up`.
fn up_item() -> PageItem {
    PageItem {
        id: UP_ITEM.into(),
        key: KeyContent::with_icon(&icons::UP, UP_LABEL),
    }
}

/// The key opening the page of the variants of `down`.
fn down_item() -> PageItem {
    PageItem {
        id: DOWN_ITEM.into(),
        key: KeyContent::with_icon(&icons::DOWN, DOWN_LABEL),
    }
}

/// The projects, then the other containers.
fn main_page(
    overview: &Overview,
    custom_icons: &HashMap<String, ImageData>,
    operations: &[Operation],
) -> PageContent {
    let projects = overview.projects.iter().map(|project| {
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
            .and_then(|name| custom_icons.get(name))
            .unwrap_or(icon);
        let label = format!(
            "{} {}/{}",
            project.shown_name(),
            project.running(),
            project.total()
        );
        if let Some(operation) = operations.iter().find(|op| op.project == project.name) {
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

/// Title of an `up` or `down` page: `label` and what it is about.
fn variants_title(label: &str, project: &Project, service: Option<&str>) -> String {
    format!(
        "{label}{TITLE_SEPARATOR}{}",
        service.unwrap_or_else(|| project.shown_name())
    )
}

/// Key of a service of a project that has no container.
fn planned_item(project: &str, service: &str) -> PageItem {
    PageItem {
        id: service_id(project, service).into(),
        key: KeyContent::with_icon(&icons::SERVICE_PLANNED, service),
    }
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
