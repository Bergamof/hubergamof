//! The services ticked on the page of a project with the contextual dial:
//! turning moves a cursor over the entries, pressing ticks or unticks the
//! one it rests on, holding the dial ticks all or none. The actions of the
//! project then work on the ticked services instead of the whole project.

use std::collections::BTreeSet;

use hubergamof_module_api::ContextDial;

use super::compose::is_service_name;
use super::pages::Page;
use super::render::Entry;
use super::{Docker, Project};

const ROTATE_HINT: &str = "Choisir";
const TICK_HINT: &str = "Cocher";
const UNTICK_HINT: &str = "Décocher";
const TICK_ALL_HINT: &str = "Tout";
const UNTICK_ALL_HINT: &str = "Aucun";

/// An entry of a project page the cursor can rest on: a container that
/// records its service, or a service without container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Selectable {
    /// Id of the item showing it.
    pub(super) item: String,
    pub(super) service: String,
}

/// What is ticked on the page of one project. Ticking is by service, what
/// compose takes: the containers of a service are ticked together.
#[derive(Debug, Default)]
pub(super) struct Selection {
    /// Project the selection is about; another project starts over.
    project: String,
    /// Item the cursor rests on.
    cursor: Option<String>,
    ticked: BTreeSet<String>,
    /// Everything that was ticked is gone (its services disappeared): the
    /// actions of the project are refused until the user ticks again, never
    /// widened to the whole project, which an empty selection means.
    lost: bool,
}

impl Selection {
    /// Starts over on `project`: nothing ticked, no cursor.
    pub(super) fn enter(&mut self, project: &str) {
        *self = Self {
            project: project.to_owned(),
            ..Self::default()
        };
    }

    /// Ticks nothing and takes the cursor away, keeping the project.
    pub(super) fn clear(&mut self) {
        self.cursor = None;
        self.ticked.clear();
        self.lost = false;
    }

    /// The selection, if it is about `project`.
    pub(super) fn of(&self, project: &str) -> Option<&Self> {
        (self.project == project).then_some(self)
    }

    /// Makes `project` the one the selection is about.
    fn adopt(&mut self, project: &str) {
        if self.of(project).is_none() {
            self.enter(project);
        }
    }

    /// The item the cursor rests on.
    pub(super) fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    pub(super) fn is_ticked(&self, service: &str) -> bool {
        self.ticked.contains(service)
    }

    /// Number of ticked services of `project`.
    pub(super) fn count(&self, project: &str) -> usize {
        self.of(project)
            .map_or(0, |selection| selection.ticked.len())
    }

    /// The ticked services of `project`, by name (so, sorted).
    pub(super) fn services(&self, project: &str) -> Vec<String> {
        self.of(project).map_or_else(Vec::new, |selection| {
            selection.ticked.iter().cloned().collect()
        })
    }

    /// Whether what was ticked in `project` has all gone.
    pub(super) fn is_lost(&self, project: &str) -> bool {
        self.of(project).is_some_and(|selection| selection.lost)
    }

    fn set_cursor(&mut self, item: &str) {
        self.cursor = Some(item.to_owned());
    }

    /// Ticks `service`, or unticks it.
    fn toggle(&mut self, service: &str) {
        if !self.ticked.remove(service) {
            self.ticked.insert(service.to_owned());
        }
        self.lost = false;
    }

    /// Ticks all of `services`, or none when they all are ticked already.
    fn toggle_all(&mut self, services: BTreeSet<String>) {
        if self.ticked == services {
            self.ticked.clear();
        } else {
            self.ticked = services;
        }
        self.lost = false;
    }

    /// Forgets what `entries` no longer offer; whether a tick went.
    fn retain(&mut self, entries: &[Selectable]) -> bool {
        let before = self.ticked.len();
        self.ticked
            .retain(|service| entries.iter().any(|entry| &entry.service == service));
        if let Some(cursor) = &self.cursor
            && !entries.iter().any(|entry| &entry.item == cursor)
        {
            self.cursor = None;
        }
        let gone = self.ticked.len() != before;
        self.lost |= gone && self.ticked.is_empty();
        gone
    }
}

/// `text` with the number of services ticked, when there are some.
pub(super) fn with_count(text: &str, ticked: usize) -> String {
    if ticked == 0 {
        text.to_owned()
    } else {
        format!("{text} ({ticked})")
    }
}

/// The entries the cursor can rest on, from `entries` of a project page: those
/// that stand for a service compose can be given by name.
pub(super) fn selectables(entries: &[Entry]) -> Vec<Selectable> {
    entries
        .iter()
        .filter_map(|entry| {
            let service = entry
                .service
                .as_ref()
                .filter(|name| is_service_name(name))?;
            Some(Selectable {
                item: entry.item.id.0.clone(),
                service: service.clone(),
            })
        })
        .collect()
}

/// Position of the cursor after `ticks` steps from `current` among `len`
/// entries (at least one), wrapping around. Without a cursor, the first step
/// clockwise lands on the first entry and the first one back on the last.
fn moved_cursor(current: Option<usize>, ticks: i32, len: usize) -> usize {
    let len = i64::try_from(len).unwrap_or(i64::MAX).max(1);
    let ticks = i64::from(ticks);
    let target = match current {
        Some(position) => i64::try_from(position).unwrap_or(0) + ticks,
        None if ticks > 0 => ticks - 1,
        None => len + ticks,
    };
    usize::try_from(target.rem_euclid(len)).unwrap_or(0)
}

impl Docker {
    fn selectable(&self, project: &str) -> Vec<Selectable> {
        self.overview()
            .and_then(|overview| overview.project(project))
            .map(|project| selectables(&self.project_entries(project)))
            .unwrap_or_default()
    }

    /// What the contextual dial does on the page of `project`, whose
    /// cursor-resting entries are `entries`.
    pub(super) fn context_dial(&self, project: &Project, entries: &[Selectable]) -> ContextDial {
        if entries.is_empty() {
            return ContextDial::none();
        }
        let selection = self.selection.of(&project.name);
        let ticked = |service: &str| selection.is_some_and(|s| s.is_ticked(service));
        let mut dial = ContextDial::none().rotating(ROTATE_HINT);
        let under_cursor = selection
            .and_then(Selection::cursor)
            .and_then(|cursor| entries.iter().find(|entry| entry.item == cursor));
        if let Some(entry) = under_cursor {
            dial = dial.pressing(if ticked(&entry.service) {
                UNTICK_HINT
            } else {
                TICK_HINT
            });
        }
        let all = entries.iter().all(|entry| ticked(&entry.service));
        dial.long_pressing(if all { UNTICK_ALL_HINT } else { TICK_ALL_HINT })
    }

    /// Moves the cursor of `project` by `ticks` entries.
    pub(super) fn move_cursor(&mut self, project: &str, ticks: i32) {
        let entries = self.selectable(project);
        if entries.is_empty() {
            return;
        }
        self.selection.adopt(project);
        let current = self
            .selection
            .cursor()
            .and_then(|cursor| entries.iter().position(|entry| entry.item == cursor));
        let next = moved_cursor(current, ticks, entries.len());
        self.selection.set_cursor(&entries[next].item);
    }

    /// Ticks the service under the cursor of `project`, or unticks it.
    pub(super) fn tick_cursor(&mut self, project: &str) {
        let entries = self.selectable(project);
        let Some(entry) = self.selection.of(project).and_then(|selection| {
            let cursor = selection.cursor()?;
            entries.iter().find(|entry| entry.item == cursor)
        }) else {
            return;
        };
        self.selection.toggle(&entry.service);
        self.confirming = None;
    }

    /// Ticks every service of `project`, or none when all are ticked.
    pub(super) fn tick_all(&mut self, project: &str) {
        let services: BTreeSet<String> = self
            .selectable(project)
            .into_iter()
            .map(|entry| entry.service)
            .collect();
        if services.is_empty() {
            return;
        }
        self.selection.adopt(project);
        self.selection.toggle_all(services);
        self.confirming = None;
    }

    /// Forgets what the selection holds of entries that are gone. A
    /// confirmation given for what was ticked is void.
    pub(super) fn prune_selection(&mut self) {
        if self.selection.cursor.is_none() && self.selection.ticked.is_empty() {
            return;
        }
        let entries = self.selectable(&self.selection.project);
        if self.selection.retain(&entries) {
            self.confirming = None;
        }
    }

    /// The ticked services of `project`: what its actions work on, the whole
    /// project when none is ticked.
    pub(super) fn ticked_services(&self, project: &str) -> Vec<String> {
        self.selection.services(project)
    }

    /// Number of services ticked that the actions of `page` work on.
    pub(super) fn ticked_on(&self, page: Page<'_>) -> usize {
        match page {
            Page::Project(project) => self.selection.count(project),
            Page::Up(scope) | Page::Down(scope) if scope.service.is_none() => {
                self.selection.count(scope.project)
            }
            _ => 0,
        }
    }

    /// The selection is spent: an action went off on what it ticked.
    pub(super) fn spend_selection(&mut self, project: &str) {
        if self.selection.of(project).is_some() {
            self.selection.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_wraps_around_in_both_directions() {
        assert_eq!(moved_cursor(Some(0), 1, 3), 1);
        assert_eq!(moved_cursor(Some(2), 1, 3), 0);
        assert_eq!(moved_cursor(Some(0), -1, 3), 2);
        assert_eq!(moved_cursor(Some(1), 7, 3), 2);
        assert_eq!(moved_cursor(Some(1), -7, 3), 0);
    }

    #[test]
    fn without_a_cursor_the_first_step_picks_an_end() {
        assert_eq!(moved_cursor(None, 1, 4), 0);
        assert_eq!(moved_cursor(None, 3, 4), 2);
        assert_eq!(moved_cursor(None, -1, 4), 3);
        assert_eq!(moved_cursor(None, -2, 4), 2);
        assert_eq!(moved_cursor(None, 1, 1), 0);
        assert_eq!(moved_cursor(None, -1, 1), 0);
    }

    fn entries(items: &[(&str, &str)]) -> Vec<Selectable> {
        items
            .iter()
            .map(|(item, service)| Selectable {
                item: (*item).into(),
                service: (*service).into(),
            })
            .collect()
    }

    #[test]
    fn ticking_is_by_service_and_sorted_by_name() {
        let mut selection = Selection::default();
        selection.enter("shop");
        selection.toggle("web");
        selection.toggle("api");
        assert_eq!(selection.services("shop"), ["api", "web"]);
        selection.toggle("web");
        assert_eq!(selection.services("shop"), ["api"]);
    }

    #[test]
    fn ticking_everything_flips_between_all_and_none() {
        let mut selection = Selection::default();
        selection.enter("shop");
        let all: BTreeSet<String> = ["api".to_owned(), "web".to_owned()].into();
        selection.toggle("api");
        selection.toggle_all(all.clone());
        assert_eq!(selection.count("shop"), 2, "one was missing: all");
        selection.toggle_all(all);
        assert_eq!(selection.count("shop"), 0, "all were ticked: none");
    }

    #[test]
    fn what_is_gone_leaves_the_selection_and_losing_it_all_is_remembered() {
        let mut selection = Selection::default();
        selection.enter("shop");
        selection.toggle("api");
        selection.toggle("web");
        selection.set_cursor("c-web");
        let kept = entries(&[("c-api", "api")]);
        assert!(selection.retain(&kept), "web went");
        assert_eq!(selection.services("shop"), ["api"]);
        assert_eq!(selection.cursor(), None, "the cursor was on web");
        assert!(!selection.is_lost("shop"), "api is still ticked");

        assert!(selection.retain(&[]));
        assert!(selection.is_lost("shop"), "nothing left of what was ticked");
        assert!(!selection.retain(&[]), "nothing more to forget");
        assert!(selection.is_lost("shop"), "stays lost");

        selection.toggle("api");
        assert!(!selection.is_lost("shop"), "ticking again starts over");
        selection.retain(&[]);
        selection.clear();
        assert!(!selection.is_lost("shop"), "spent");
    }

    #[test]
    fn a_selection_that_never_held_anything_is_not_lost() {
        let mut selection = Selection::default();
        selection.enter("shop");
        assert!(!selection.retain(&[]));
        assert!(!selection.is_lost("shop"));
    }

    #[test]
    fn counts_come_with_the_number_ticked() {
        assert_eq!(with_count("Up", 0), "Up");
        assert_eq!(with_count("Up", 2), "Up (2)");
    }

    #[test]
    fn a_selection_belongs_to_one_project() {
        let mut selection = Selection::default();
        selection.enter("shop");
        selection.toggle("web");
        assert_eq!(selection.count("shop"), 1);
        assert_eq!(selection.count("blog"), 0);
        assert_eq!(selection.services("blog"), Vec::<String>::new());
        selection.clear();
        assert!(selection.of("shop").is_some(), "cleared, not forgotten");
        selection.enter("blog");
        assert!(selection.of("shop").is_none());
    }
}
