//! The `up` and `down` under way, shown on the entries they work on.

use super::{ComposeScope, UpVariant};

/// What an operation does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OperationKind {
    Up(UpVariant),
    Down,
}

impl OperationKind {
    /// What the entries it works on say while it runs.
    pub(super) fn working_label(self) -> &'static str {
        match self {
            Self::Up(UpVariant::Build) => "Build…",
            Self::Up(_) => "Up…",
            Self::Down => "Down…",
        }
    }
}

/// An `up` or a `down` of a project, or of some of its services, under way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Operation {
    pub(super) id: u64,
    pub(super) project: String,
    /// What it works on: the whole project, or one service.
    pub(super) scope: ComposeScope,
    pub(super) kind: OperationKind,
}

impl Operation {
    /// Whether it works on `service` of its project (`None`: a container
    /// that records no service, only reached by a whole-project operation).
    pub(super) fn covers(&self, service: Option<&str>) -> bool {
        self.scope.covers(service)
    }
}
