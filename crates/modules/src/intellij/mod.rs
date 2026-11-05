//! IntelliJ module: the recent projects of IntelliJ IDEA, opened or brought
//! to the front from the keys.
//!
//! - Home key: `IntelliJ`, active while it runs, in attention when one of
//!   its windows asks for it, or when IntelliJ sent a notification (tests
//!   run, build done) until the key is pressed or IntelliJ is in front. Pressed while IntelliJ is closed: starts it;
//!   while it runs: opens the projects page.
//! - Projects page: the open projects first, then the recent ones, most
//!   recent first, at most `maxProjects`. Pressing an open project brings
//!   its window to the front; another one opens it in IntelliJ.
//!
//! Windows and processes come from the desktop layer
//! ([`DesktopHub`](crate::desktop::DesktopHub)); the installation and the
//! recent projects from a [`ProjectCatalog`], polled in the background
//! once for every instance. Launches only ever run the launcher of the
//! installation, found trusted, with a project folder (absolute, existing)
//! as their sole argument; one launch of IntelliJ at a time, whatever the
//! instance.

mod catalog;
mod icons;
mod recent;
mod toolbox;
mod watch;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use bergadeck_module_api::{
    Command, EventSender, Feedback, ImageData, ItemId, KeyContent, KeyState, Module,
    ModuleCapabilities, ModuleContext, ModuleDescriptor, ModuleError, ModuleEvent, ModuleFactory,
    ModuleSurface, PageContent, PageId, PageItem, SettingField, SettingKind,
};
use serde::Deserialize;
use serde_json::Value;

pub use catalog::{Catalog, Project, ProjectCatalog, SimulatedCatalog, project_name};
use toolbox::LAUNCHER;
pub use toolbox::SystemCatalog;
use watch::{CatalogHub, CatalogSubscription};

use crate::background::forward_states;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::desktop::{
    AppId, AppNotification, AppState, ClassMatch, DesktopApp, DesktopHub, DesktopState,
    DesktopSubscription, ExecutableMatch, Launch, Window, WindowId,
};

/// Kind of the IntelliJ module.
pub const INTELLIJ_KIND: &str = "intellij";

/// IntelliJ among the applications of the desktop layer.
const APP: AppId = AppId(INTELLIJ_KIND);

/// Class of the IntelliJ IDEA windows (Ultimate and Community).
const WINDOW_CLASS_PREFIX: &str = "jetbrains-idea";

const MAX_PROJECTS_KEY: &str = "maxProjects";
const DEFAULT_MAX_PROJECTS: u32 = 8;
const MIN_PROJECTS: u32 = 1;
const MAX_PROJECTS: u32 = 32;

/// Page opened by the home key.
const MAIN_PAGE: &str = "intellij";
/// Prefix of the items of the recent projects, followed by their folder.
const PROJECT_PREFIX: &str = "project/";
/// Prefix of the items of windows of no recent project, followed by their
/// id.
const WINDOW_PREFIX: &str = "window/";

const MODULE_LABEL: &str = "IntelliJ";
const UNAVAILABLE_LABEL: &str = "IntelliJ introuvable";
const OPENING_LABEL: &str = "Ouverture…";

/// IntelliJ, as the desktop layer follows it.
#[must_use]
pub fn desktop_app() -> DesktopApp {
    DesktopApp {
        id: APP,
        window_class: ClassMatch::Prefix(WINDOW_CLASS_PREFIX.into()),
        executable: ExecutableMatch {
            path: LAUNCHER.into(),
            helper_argument: None,
        },
        desktop_entry: None,
    }
}

/// Creates [`Intellij`] instances.
#[derive(Debug, Clone)]
pub struct IntellijFactory {
    desktop: Arc<DesktopHub>,
    catalog: Arc<CatalogHub>,
    /// A launch of IntelliJ is under way, from any instance: one at a time.
    launching: Arc<AtomicBool>,
}

impl IntellijFactory {
    /// Factory following IntelliJ through `desktop` (which follows
    /// [`desktop_app`]), its installation read from `catalog`.
    #[must_use]
    pub fn new(desktop: Arc<DesktopHub>, catalog: Arc<dyn ProjectCatalog>) -> Self {
        Self {
            desktop,
            catalog: CatalogHub::new(catalog),
            launching: Arc::default(),
        }
    }
}

/// Kind, capabilities and settings of the IntelliJ module.
fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        kind: INTELLIJ_KIND.into(),
        name: "IntelliJ".into(),
        description: "Projets récents d'IntelliJ IDEA : ouvrir un projet ou passer à sa fenêtre."
            .into(),
        capabilities: ModuleCapabilities {
            key: true,
            dial: false,
            touch_zone: false,
        },
        settings: vec![SettingField {
            key: MAX_PROJECTS_KEY.into(),
            label: "Projets affichés".into(),
            help: Some("Nombre maximal de projets sur la page, les ouverts d'abord.".into()),
            kind: SettingKind::Integer {
                default: DEFAULT_MAX_PROJECTS.into(),
                min: Some(MIN_PROJECTS.into()),
                max: Some(MAX_PROJECTS.into()),
                unit: None,
            },
        }],
    }
}

/// Settings of an IntelliJ instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct IntellijSettings {
    max_projects: u32,
}

impl Default for IntellijSettings {
    fn default() -> Self {
        Self {
            max_projects: DEFAULT_MAX_PROJECTS,
        }
    }
}

impl IntellijSettings {
    fn parse(settings: &Value) -> Result<Self, ModuleError> {
        descriptor().parse_settings(settings)
    }

    fn max_projects(self) -> usize {
        usize::try_from(self.max_projects).unwrap_or(usize::MAX)
    }
}

impl ModuleFactory for IntellijFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor()
    }

    fn icon(&self) -> Option<ImageData> {
        Some(icons::MODULE.clone())
    }

    fn validate_settings(&self, settings: &Value) -> Result<(), ModuleError> {
        IntellijSettings::parse(settings).map(drop)
    }

    fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
        let settings = IntellijSettings::parse(settings)?;
        let desktop = self.desktop.subscribe();
        let events = ctx.events().clone();
        ctx.tasks().spawn(forward_states(
            desktop.states(),
            events.clone(),
            IntellijEvent::Desktop,
        ));
        ctx.tasks().spawn(forward_notifications(
            desktop.notifications(),
            events.clone(),
        ));
        let catalog = self.catalog.subscribe();
        ctx.tasks().spawn(forward_states(
            catalog.states(),
            events.clone(),
            IntellijEvent::Catalog,
        ));
        Ok(Box::new(Intellij {
            settings,
            desktop,
            _catalog_subscription: catalog,
            app: AppState::default(),
            catalog: None,
            events,
            launching: None,
            app_launching: Arc::clone(&self.launching),
            notified: false,
        }))
    }
}

/// Tells the instance each time IntelliJ sends a notification (tests run,
/// build done…: IntelliJ only sends them while in the background); their
/// texts are neither kept nor read. Ends with the instance.
async fn forward_notifications(
    mut notifications: broadcast::Receiver<AppNotification>,
    events: EventSender,
) {
    loop {
        match notifications.recv().await {
            Ok(notification) if notification.app == APP => {
                if !events.send(IntellijEvent::Notified) {
                    return;
                }
            }
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
        }
    }
}

/// What an instance receives through its event sender.
#[derive(Debug)]
enum IntellijEvent {
    Desktop(DesktopState),
    /// `None` until first read.
    Catalog(Option<Arc<Catalog>>),
    /// IntelliJ sent a notification.
    Notified,
    /// The action started from `surface` ended.
    Outcome {
        surface: ModuleSurface,
        launch: bool,
        ok: bool,
    },
}

/// What a launch opens.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Opening {
    /// IntelliJ itself, from the home key.
    Application,
    /// The project of this item.
    Project(ItemId),
}

/// What pressing an entry acts on.
enum Target {
    /// Brought to the front.
    Window(WindowId),
    /// Opened in IntelliJ.
    Folder(std::path::PathBuf),
}

/// One instance: IntelliJ as last known.
pub struct Intellij {
    settings: IntellijSettings,
    desktop: DesktopSubscription,
    _catalog_subscription: CatalogSubscription,
    app: AppState,
    /// `None` until first read.
    catalog: Option<Arc<Catalog>>,
    events: EventSender,
    /// The launch this instance started, under way.
    launching: Option<Opening>,
    /// A launch is under way, from any instance.
    app_launching: Arc<AtomicBool>,
    /// IntelliJ sent a notification since it was last in front or the home
    /// key last pressed.
    notified: bool,
}

/// An entry of the projects page.
#[derive(Debug, Clone, Copy)]
enum Entry<'a> {
    /// A recent project, with its window if open.
    Project(&'a Project, Option<&'a Window>),
    /// A window of no recent project (opened since the list was written).
    Window(&'a Window),
}

impl<'a> Entry<'a> {
    fn item(self) -> ItemId {
        match self {
            Self::Project(project, _) => {
                format!("{PROJECT_PREFIX}{}", project.path.to_string_lossy()).into()
            }
            Self::Window(window) => format!("{WINDOW_PREFIX}{}", window.id.0).into(),
        }
    }

    fn window(self) -> Option<&'a Window> {
        match self {
            Self::Project(_, window) => window,
            Self::Window(window) => Some(window),
        }
    }
}

impl Intellij {
    fn windows(&self) -> &[Window] {
        self.app.windows.as_deref().unwrap_or_default()
    }

    /// The entries of the projects page, in order: open projects (recent
    /// ones first, then other windows), then closed ones; at most
    /// `maxProjects`.
    fn entries(&self) -> Vec<Entry<'_>> {
        let windows = self.windows();
        let projects = self
            .catalog
            .as_deref()
            .map_or(&[][..], |catalog| catalog.projects.as_slice());
        // Each window goes to one project only (two recent projects may
        // share a name).
        let mut used: Vec<&WindowId> = Vec::new();
        let mut open = Vec::new();
        let mut closed = Vec::new();
        for project in projects {
            let window = windows
                .iter()
                .find(|window| !used.contains(&&window.id) && project.shown_in(window));
            if let Some(window) = window {
                used.push(&window.id);
                open.push(Entry::Project(project, Some(window)));
            } else {
                closed.push(Entry::Project(project, None));
            }
        }
        let other_windows = windows
            .iter()
            .filter(|window| !used.contains(&&window.id))
            .map(Entry::Window);
        open.into_iter()
            .chain(other_windows)
            .chain(closed)
            .take(self.settings.max_projects())
            .collect()
    }

    fn launcher(&self) -> Option<&std::path::Path> {
        self.catalog.as_deref()?.launcher.as_deref()
    }

    fn unavailable(&self) -> bool {
        self.catalog.is_some() && self.launcher().is_none() && !self.app.is_running()
    }

    fn handle(&mut self, event: IntellijEvent) -> Vec<Command> {
        match event {
            IntellijEvent::Desktop(state) => {
                self.app = state.app(APP);
                let in_front = self.windows().iter().any(|window| window.active);
                self.notified &= !in_front;
                Vec::new()
            }
            IntellijEvent::Notified => {
                self.notified = true;
                Vec::new()
            }
            IntellijEvent::Catalog(catalog) => {
                if catalog.is_some() {
                    self.catalog = catalog;
                }
                Vec::new()
            }
            IntellijEvent::Outcome {
                surface,
                launch,
                ok,
            } => {
                if launch {
                    self.launching = None;
                }
                let feedback = if ok { Feedback::Ok } else { Feedback::Alert };
                vec![Command::FeedbackOn(surface, feedback)]
            }
        }
    }

    /// Hands the outcome of an action started from `surface` back to the
    /// instance.
    fn reporter(
        &self,
        surface: ModuleSurface,
        launch: bool,
    ) -> impl FnOnce(Result<(), crate::desktop::DesktopError>) + Send + 'static {
        let events = self.events.clone();
        move |outcome| {
            if let Err(error) = &outcome {
                // No project path in the logs: only the program and why.
                tracing::warn!(%error, "IntelliJ action failed");
            }
            // An instance gone meanwhile needs no outcome.
            let _ = events.send(IntellijEvent::Outcome {
                surface,
                launch,
                ok: outcome.is_ok(),
            });
        }
    }

    /// Starts IntelliJ with `args`, unless a launch is under way (from any
    /// instance).
    fn launch(
        &mut self,
        opening: Opening,
        surface: ModuleSurface,
        args: Vec<std::ffi::OsString>,
    ) -> Vec<Command> {
        let Some(program) = self.launcher().map(std::path::Path::to_path_buf) else {
            return alert();
        };
        let free =
            self.app_launching
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire);
        if free.is_err() {
            return Vec::new();
        }
        let report = self.reporter(surface, true);
        let app_launching = Arc::clone(&self.app_launching);
        let started = self
            .desktop
            .launch(Launch { program, args }, move |outcome| {
                app_launching.store(false, Ordering::Release);
                report(outcome);
            });
        if !started {
            self.app_launching.store(false, Ordering::Release);
            return alert();
        }
        self.launching = Some(opening);
        Vec::new()
    }

    fn activate(&self, window: WindowId, surface: ModuleSurface) -> Vec<Command> {
        if self.desktop.activate(window, self.reporter(surface, false)) {
            Vec::new()
        } else {
            alert()
        }
    }

    fn entry_key(&self, entry: Entry<'_>) -> KeyContent {
        let item = entry.item();
        let label = if self.launching == Some(Opening::Project(item)) {
            OPENING_LABEL.to_owned()
        } else {
            match entry {
                Entry::Project(project, _) => project.name.clone(),
                Entry::Window(window) => project_name(&window.caption).to_owned(),
            }
        };
        let state = match entry.window() {
            Some(window) if window.demands_attention => KeyState::Attention,
            Some(_) => KeyState::Active,
            None => KeyState::Normal,
        };
        icon_key(&icons::PROJECT, &label).in_state(state)
    }

    fn press_entry(&mut self, page: &PageId, item: &ItemId) -> Vec<Command> {
        let surface = ModuleSurface::Item {
            page: page.clone(),
            item: item.clone(),
        };
        let target = self
            .entries()
            .into_iter()
            .find(|entry| entry.item() == *item)
            .map(|entry| match entry {
                Entry::Project(project, None) => Target::Folder(project.path.clone()),
                Entry::Project(_, Some(window)) | Entry::Window(window) => {
                    Target::Window(window.id.clone())
                }
            });
        match target {
            Some(Target::Window(window)) => self.activate(window, surface),
            Some(Target::Folder(folder)) => {
                let args = vec![folder.into_os_string()];
                self.launch(Opening::Project(item.clone()), surface, args)
            }
            None => alert(),
        }
    }
}

fn alert() -> Vec<Command> {
    vec![Command::Feedback(Feedback::Alert)]
}

fn icon_key(icon: &ImageData, label: &str) -> KeyContent {
    KeyContent {
        image: Some(icon.clone()),
        label: Some(label.into()),
        state: KeyState::Normal,
    }
}

#[async_trait]
impl Module for Intellij {
    fn key(&self) -> KeyContent {
        let label = if self.launching == Some(Opening::Application) {
            OPENING_LABEL
        } else if self.unavailable() {
            UNAVAILABLE_LABEL
        } else {
            MODULE_LABEL
        };
        let state = if self.notified || self.app.demands_attention() || self.unavailable() {
            KeyState::Attention
        } else if self.app.is_running() {
            KeyState::Active
        } else {
            KeyState::Normal
        };
        icon_key(&icons::MODULE, label).in_state(state)
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        self.notified = false;
        if self.app.is_running() {
            return vec![Command::OpenPage(MAIN_PAGE.into())];
        }
        self.launch(Opening::Application, ModuleSurface::HomeKey, Vec::new())
    }

    fn page(&self, page: &PageId) -> Option<PageContent> {
        (page.0 == MAIN_PAGE).then(|| PageContent {
            title: MODULE_LABEL.into(),
            items: self
                .entries()
                .into_iter()
                .map(|entry| PageItem {
                    id: entry.item(),
                    key: self.entry_key(entry),
                })
                .collect(),
        })
    }

    async fn on_item_press(&mut self, page: &PageId, item: &ItemId) -> Vec<Command> {
        if page.0 != MAIN_PAGE {
            return Vec::new();
        }
        self.press_entry(page, item)
    }

    async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
        match event.downcast::<IntellijEvent>() {
            Ok(event) => self.handle(*event),
            Err(_) => Vec::new(),
        }
    }

    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError> {
        self.settings = IntellijSettings::parse(settings)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
