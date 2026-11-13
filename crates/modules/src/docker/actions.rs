//! What the keys of the action pages do, and what it amounts to.

use hubergamof_module_api::Command;

use super::pages::{compose_service, container_id, container_parent};
use super::selection::with_count;
use super::{
    ComposeAction, ComposeScope, ContainerAction, DOWN_LABEL, Docker, DockerEvent, Feedback,
    ImageData, ItemKey, Operation, OperationKind, Page, Request, Scope, UP_LABEL, UpVariant, icons,
    project_id,
};

/// What an action key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    StartAll,
    StopAll,
    Start,
    Stop,
    Restart,
    Remove,
    Up(UpVariant),
    Down { volumes: bool },
}

pub(super) const PROJECT_ACTIONS: [Action; 2] = [Action::StartAll, Action::StopAll];

pub(super) const UP_ACTIONS: [Action; 4] = [
    Action::Up(UpVariant::Plain),
    Action::Up(UpVariant::Build),
    Action::Up(UpVariant::Pull),
    Action::Up(UpVariant::NoStart),
];

pub(super) const DOWN_ACTIONS: [Action; 2] = [
    Action::Down { volumes: false },
    Action::Down { volumes: true },
];

/// Actions of a container page; [`Action::Remove`] shown only when the
/// container may be removed.
pub(super) const CONTAINER_ACTIONS: [Action; 4] =
    [Action::Start, Action::Stop, Action::Restart, Action::Remove];

impl Action {
    pub(super) fn item(self) -> &'static str {
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

    pub(super) fn label(self) -> &'static str {
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
            Self::Down { volumes: true } => "Avec volumes",
        }
    }

    /// The label when `ticked` services are selected: the actions of a
    /// project then say how many they work on.
    pub(super) fn label_for(self, ticked: usize) -> String {
        match self {
            Self::StartAll if ticked > 0 => with_count(Self::Start.label(), ticked),
            Self::StopAll if ticked > 0 => with_count(Self::Stop.label(), ticked),
            _ => self.label().to_owned(),
        }
    }

    /// Label while it runs.
    pub(super) fn busy_label(self) -> &'static str {
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
    pub(super) fn operation(self) -> Option<OperationKind> {
        match self {
            Self::Up(variant) => Some(OperationKind::Up(variant)),
            Self::Down { .. } => Some(OperationKind::Down),
            _ => None,
        }
    }

    pub(super) fn icon(self) -> &'static ImageData {
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
    pub(super) fn needs_confirmation(self) -> bool {
        matches!(self, Self::Remove | Self::Down { .. })
    }

    /// The actions of `page`.
    pub(super) fn of_page(page: Page<'_>) -> &'static [Self] {
        match page {
            Page::Main | Page::Service { .. } => &[],
            Page::Project(_) => &PROJECT_ACTIONS,
            Page::Up(_) => &UP_ACTIONS,
            Page::Down(_) => &DOWN_ACTIONS,
            Page::Container(_) => &CONTAINER_ACTIONS,
        }
    }

    /// The action of `item` on `page`.
    pub(super) fn of(page: Page<'_>, item: &str) -> Option<Self> {
        Self::of_page(page)
            .iter()
            .copied()
            .find(|action| action.item() == item)
    }
}

/// What an action amounts to on the current containers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Plan {
    /// Already done (nothing to start…).
    Nothing,
    Run(Request),
    /// Cannot be done: project or container gone, not removable…
    Refused,
}

impl Docker {
    pub(super) fn perform(&mut self, key: ItemKey, page: Page<'_>, action: Action) -> Vec<Command> {
        if self.is_running(&key, page) {
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
        if let Page::Container(name) = page {
            return self.start_container_action(name, action, request);
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
        if !started {
            return Command::alert();
        }
        self.busy.insert(key);
        if let Page::Project(project) = page {
            self.spend_selection(project);
        }
        Vec::new()
    }

    /// Whether the action of `key` already runs; on the page of a container,
    /// whether any of its actions does: one at a time.
    fn is_running(&self, key: &ItemKey, page: Page<'_>) -> bool {
        match page {
            Page::Container(name) => self.container_actions.contains_key(name),
            _ => self.busy.contains(key),
        }
    }

    /// Starts `action` of the page of the container `name`, and goes back to
    /// the page listing it, whose entry shows the action then its outcome.
    fn start_container_action(
        &mut self,
        name: &str,
        action: Action,
        request: Request,
    ) -> Vec<Command> {
        let Some(parent) = self
            .overview()
            .map(|overview| container_parent(overview, name))
        else {
            return Command::alert();
        };
        // Chosen now: a container removed meanwhile is listed nowhere.
        let shown_on = ItemKey::new(&parent, &container_id(name).into());
        let events = self.events.clone();
        let container = name.to_owned();
        let started = self.hub.run(request, move |outcome| {
            if let Err(error) = &outcome {
                tracing::warn!(%error, "Docker action failed");
            }
            // An instance gone meanwhile needs no outcome.
            let _ = events.send(DockerEvent::ContainerOutcome {
                container,
                shown_on,
                ok: outcome.is_ok(),
            });
        });
        if !started {
            return Command::alert();
        }
        self.container_actions.insert(name.to_owned(), action);
        vec![Command::BackTo(parent)]
    }

    /// The action of its page running on the container `name`, if any.
    pub(super) fn container_action(&self, name: &str) -> Option<Action> {
        self.container_actions.get(name).copied()
    }

    /// Starts the compose `request` of an `up` or a `down`, and leaves its
    /// page for the one of the project, where what it works on shows.
    pub(super) fn start_operation(
        &mut self,
        scope: Scope<'_>,
        kind: OperationKind,
        request: Request,
    ) -> Vec<Command> {
        let Request::Compose {
            scope: worked_on, ..
        } = &request
        else {
            return Command::alert();
        };
        let worked_on = worked_on.clone();
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
            scope: worked_on,
            kind,
        });
        if scope.service.is_none() {
            // What the page of the project had ticked has been used.
            self.spend_selection(scope.project);
        }
        vec![Command::BackTo(project_id(scope.project).into())]
    }

    /// What `action` on `page` amounts to on the containers shown.
    pub(super) fn plan(&self, page: Page<'_>, action: Action) -> Plan {
        let Some(overview) = self.overview() else {
            return Plan::Refused;
        };
        let compose = |scope: Scope<'_>, action| {
            if scope.service.is_none() && self.selection.is_lost(scope.project) {
                return Plan::Refused;
            }
            let target = overview
                .project(scope.project)
                .and_then(|p| p.target.clone());
            // The services ticked on the page of the project, unless the page
            // is about one service.
            let worked_on = match scope.service {
                Some(service) => ComposeScope::of(Some(service)),
                None => ComposeScope::of_services(self.ticked_services(scope.project)),
            };
            target.map_or(Plan::Refused, |target| {
                Plan::Run(Request::Compose {
                    target,
                    scope: worked_on,
                    action,
                })
            })
        };
        match (page, action) {
            (Page::Project(name), Action::StartAll | Action::StopAll) => {
                let Some(project) = overview.project(name) else {
                    return Plan::Refused;
                };
                if self.selection.is_lost(name) {
                    return Plan::Refused;
                }
                let start = action == Action::StartAll;
                let ticked = self.ticked_services(name);
                let ids: Vec<String> = project
                    .containers
                    .iter()
                    .filter(|c| {
                        ticked.is_empty()
                            || compose_service(c).is_some_and(|(_, service)| {
                                ticked.iter().any(|name| name == service)
                            })
                    })
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
}
