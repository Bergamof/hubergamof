//! IntelliJ module: the recent projects of IntelliJ IDEA, opened or brought
//! to the front from the keys.
//!
//! - Home key: `IntelliJ`, active while it runs, in attention when one of
//!   its windows asks for it, or when IntelliJ sent a notification (tests
//!   run, build done) until the key is pressed or IntelliJ is in front.
//!   Pressed while IntelliJ is closed: starts it; while it runs: opens the
//!   projects page.
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

use async_trait::async_trait;
use hubergamof_module_api::{
    Command, ImageData, ItemId, KeyContent, KeyState, Module, ModuleCapabilities, ModuleContext,
    ModuleDescriptor, ModuleError, ModuleEvent, ModuleFactory, ModuleSurface, PageContent, PageId,
    PageItem, SettingField, SettingKind,
};
use serde::Deserialize;
use serde_json::Value;

pub(crate) use catalog::project_name;
pub use catalog::{Catalog, Project, ProjectCatalog, SimulatedCatalog};
use toolbox::LAUNCHER;
pub use toolbox::SystemCatalog;
use watch::{CatalogHub, CatalogSubscription};

use crate::background::forward_states;
use crate::desktop::{
    AppAttention, AppEvent, AppId, ClassMatch, DesktopAction, DesktopApp, DesktopHub,
    ExecutableMatch, FollowedApp, Launch, LaunchStart, Window, WindowId,
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
pub(crate) fn desktop_app() -> DesktopApp {
    DesktopApp {
        id: APP,
        window_class: ClassMatch::Prefix(WINDOW_CLASS_PREFIX.into()),
        executable: ExecutableMatch {
            path: LAUNCHER.into(),
            helper_argument: None,
        },
        desktop_entry: None,
        relays: Vec::new(),
    }
}

/// Creates [`Intellij`] instances.
#[derive(Debug, Clone)]
pub struct IntellijFactory {
    desktop: Arc<DesktopHub>,
    catalog: Arc<CatalogHub>,
}

impl IntellijFactory {
    /// Factory following IntelliJ through `desktop` (which follows
    /// [`desktop_app`]), its installation read from `catalog`.
    #[must_use]
    pub(crate) fn new(desktop: Arc<DesktopHub>, catalog: Arc<dyn ProjectCatalog>) -> Self {
        Self {
            desktop,
            catalog: CatalogHub::new(catalog),
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
            ..Default::default()
        },
        dial_functions: Vec::new(),
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

    fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
        let settings = IntellijSettings::parse(settings)?;
        let catalog = self.catalog.subscribe();
        ctx.tasks().spawn(forward_states(
            catalog.states(),
            ctx.events().clone(),
            CatalogRead,
        ));
        Ok(Box::new(Intellij {
            settings,
            app: FollowedApp::follow(&self.desktop, APP, AppAttention::with_windows(), &ctx),
            _catalog_subscription: catalog,
            catalog: None,
            launching: None,
        }))
    }
}

/// The installation as read, `None` until first read (besides the
/// [`AppEvent`]s of IntelliJ).
#[derive(Debug)]
struct CatalogRead(Option<Arc<Catalog>>);

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
pub(crate) struct Intellij {
    settings: IntellijSettings,
    /// IntelliJ; in attention when a window asks or after a notification
    /// (tests run, build done).
    app: FollowedApp,
    _catalog_subscription: CatalogSubscription,
    /// `None` until first read.
    catalog: Option<Arc<Catalog>>,
    /// The launch this instance started, under way.
    launching: Option<Opening>,
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
        self.app.windows()
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
        self.catalog.is_some() && self.launcher().is_none() && !self.app.state().is_running()
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
            return Command::alert();
        };
        match self.app.launch(Launch { program, args }, surface) {
            LaunchStart::Started => {
                self.launching = Some(opening);
                Vec::new()
            }
            LaunchStart::Busy => Vec::new(),
            LaunchStart::Refused => Command::alert(),
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
        KeyContent::with_icon(&icons::PROJECT, &label).in_state(state)
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
            Some(Target::Window(window)) => self.app.activate(window, surface),
            Some(Target::Folder(folder)) => {
                let args = vec![folder.into_os_string()];
                self.launch(Opening::Project(item.clone()), surface, args)
            }
            None => Command::alert(),
        }
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
        KeyContent::with_icon(&icons::MODULE, label)
            .in_state(self.app.key_state(self.unavailable()))
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        self.app.acknowledge();
        if self.app.state().is_running() {
            return vec![Command::OpenPage(MAIN_PAGE.into())];
        }
        self.launch(Opening::Application, ModuleSurface::HomeKey, Vec::new())
    }

    fn page(&self, page: &PageId) -> Option<PageContent> {
        (page.0 == MAIN_PAGE).then(|| {
            let items = self
                .entries()
                .into_iter()
                .map(|entry| PageItem {
                    id: entry.item(),
                    key: self.entry_key(entry),
                })
                .collect();
            PageContent::new(MODULE_LABEL, items)
        })
    }

    async fn on_item_press(&mut self, page: &PageId, item: &ItemId) -> Vec<Command> {
        if page.0 != MAIN_PAGE {
            return Vec::new();
        }
        self.press_entry(page, item)
    }

    async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
        let event = match event.downcast::<CatalogRead>() {
            Ok(read) => {
                if read.0.is_some() {
                    self.catalog = read.0;
                }
                return Vec::new();
            }
            Err(event) => event,
        };
        let Ok(event) = event.downcast::<AppEvent>() else {
            return Vec::new();
        };
        let Some(outcome) = self.app.handle(*event) else {
            return Vec::new();
        };
        if outcome.action == DesktopAction::Launch {
            self.launching = None;
        }
        vec![outcome.feedback()]
    }

    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError> {
        self.settings = IntellijSettings::parse(settings)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
