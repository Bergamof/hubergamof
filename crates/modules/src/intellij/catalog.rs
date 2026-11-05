//! What the IntelliJ module needs to know of the installation: the
//! launcher, and the recent projects. Read through a [`ProjectCatalog`]: the
//! user's files ([`SystemCatalog`](super::toolbox::SystemCatalog)) or a fixed
//! demo one ([`SimulatedCatalog`]).

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::desktop::{DEMO_INTELLIJ_EXECUTABLE, Window};

/// Separates the project name from the file in IntelliJ window titles
/// (`bergadeck – main.rs`).
const TITLE_SEPARATOR: &str = " – ";

/// A recent project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Name shown by IntelliJ in its window titles.
    pub name: String,
    /// Its folder: absolute, links resolved, existing.
    pub path: PathBuf,
}

impl Project {
    /// Whether `window` is this project's.
    #[must_use]
    pub fn shown_in(&self, window: &Window) -> bool {
        project_name(&window.caption) == self.name
    }
}

/// The name of the project a window title shows.
#[must_use]
pub fn project_name(caption: &str) -> &str {
    caption
        .split(TITLE_SEPARATOR)
        .next()
        .unwrap_or(caption)
        .trim()
}

/// What is known of the installation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    /// Program starting IntelliJ, found trusted (see `trust`); `None` when
    /// no installation was found.
    pub launcher: Option<PathBuf>,
    /// Recent projects, most recently used first.
    pub projects: Vec<Project>,
}

/// Reads the installation. Synchronous file system reads: called through
/// `spawn_blocking`.
pub trait ProjectCatalog: Send + Sync + fmt::Debug {
    /// The installation as it is now.
    ///
    /// # Errors
    ///
    /// A description of what could not be read; callers log it and go on.
    fn read(&self) -> Result<Catalog, String>;
}

/// An installation set by hand (`--demo` and tests), counting its reads.
#[derive(Debug)]
pub struct SimulatedCatalog {
    read: Mutex<Result<Catalog, String>>,
    reads: AtomicUsize,
}

impl Default for SimulatedCatalog {
    /// An installation without launcher nor project.
    fn default() -> Self {
        Self::new(Catalog::default())
    }
}

impl SimulatedCatalog {
    /// An installation holding `catalog`.
    #[must_use]
    pub fn new(catalog: Catalog) -> Self {
        Self {
            read: Mutex::new(Ok(catalog)),
            reads: AtomicUsize::new(0),
        }
    }

    /// The demo installation: three projects, the first one open in the
    /// demo desktop.
    #[must_use]
    pub fn demo() -> Self {
        let project = |name: &str| Project {
            name: name.into(),
            path: PathBuf::from("/home/demo/projets").join(name),
        };
        Self::new(Catalog {
            launcher: Some(DEMO_INTELLIJ_EXECUTABLE.into()),
            projects: vec![project("bergadeck"), project("bergacast"), project("notes")],
        })
    }

    /// What the next reads give.
    pub fn set(&self, read: Result<Catalog, String>) {
        *self.lock() = read;
    }

    /// How many times it was read.
    #[must_use]
    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }

    fn lock(&self) -> MutexGuard<'_, Result<Catalog, String>> {
        // Only ever replaced whole: a poisoned lock is still valid.
        self.read.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ProjectCatalog for SimulatedCatalog {
    fn read(&self) -> Result<Catalog, String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(caption: &str) -> Window {
        Window::new("1", "jetbrains-idea", caption)
    }

    #[test]
    fn windows_are_matched_by_the_project_in_their_title() {
        assert_eq!(project_name("bergadeck – main.rs"), "bergadeck");
        assert_eq!(
            project_name("betgamof – BankrollService.kt [betgamof-backend.business.main]"),
            "betgamof"
        );
        assert_eq!(project_name("bergadeck"), "bergadeck");
        let project = &SimulatedCatalog::demo().read().unwrap().projects[0];
        assert!(project.shown_in(&window("bergadeck – main.rs")));
        assert!(!project.shown_in(&window("bergacast – main.rs")));
        assert!(!project.shown_in(&window("Welcome to IntelliJ IDEA")));
    }

    #[test]
    fn the_demo_catalog_has_a_launcher_and_projects() {
        let catalog = SimulatedCatalog::demo().read().unwrap();
        assert!(catalog.launcher.is_some());
        assert_eq!(catalog.projects.len(), 3);
    }
}
