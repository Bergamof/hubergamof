//! The places where the outcome of a call is shown: keys, dial reminders.

use super::{
    DeviceEngine, Feedback, Instant, KeyTarget, ModuleInstanceId, ModuleSurface, Outcome, Surface,
};

impl DeviceEngine {
    /// The reminder of `dial` shows the outcome of what it does.
    pub(super) fn pressed_dial_surfaces(&self, dial: u8) -> Vec<Surface> {
        if self.has_strip {
            vec![Surface::Dial(dial)]
        } else {
            Vec::new()
        }
    }

    /// Surfaces showing the outcome of an event of `module`: its home key
    /// when displayed, or else the reminders of its dials.
    pub(super) fn event_surfaces(&self, module: &ModuleInstanceId) -> Vec<Surface> {
        let home = self.displayed(vec![Self::home_surface(module)]);
        if home.is_empty() {
            self.displayed_dials(module)
        } else {
            home
        }
    }

    /// Surfaces alerting that `module` was replaced by a placeholder: its
    /// home key if it is on the home page, or else the reminders of its dials.
    /// Not limited to the last frame: the replacement happens while the next
    /// one is composed.
    pub(super) fn failure_surfaces(&self, module: &ModuleInstanceId) -> Vec<Surface> {
        if self.config.home.contains(module) {
            vec![Self::home_surface(module)]
        } else {
            self.dial_surfaces(module)
        }
    }

    pub(super) fn home_surface(module: &ModuleInstanceId) -> Surface {
        Surface::Key(KeyTarget::Home(module.clone()))
    }

    /// The reminders of the dials assigned a function of `module`.
    pub(super) fn dial_surfaces(&self, module: &ModuleInstanceId) -> Vec<Surface> {
        if !self.has_strip {
            return Vec::new();
        }
        self.config
            .dials
            .iter()
            .zip(0_u8..)
            .filter(|(assigned, _)| assigned.instance() == Some(module))
            .map(|(_, dial)| Surface::Dial(dial))
            .collect()
    }

    /// The displayed reminders of the dials assigned a function of `module`.
    pub(super) fn displayed_dials(&self, module: &ModuleInstanceId) -> Vec<Surface> {
        self.displayed(self.dial_surfaces(module))
    }

    /// Surfaces of `module` named by a [`Command::FeedbackOn`], among those
    /// displayed.
    pub(super) fn named_surfaces(
        &self,
        module: &ModuleInstanceId,
        surface: ModuleSurface,
    ) -> Vec<Surface> {
        match surface {
            ModuleSurface::HomeKey => self.displayed(vec![Self::home_surface(module)]),
            ModuleSurface::Dial => self.displayed_dials(module),
            ModuleSurface::Item { page, item } => {
                self.displayed(vec![Surface::Key(KeyTarget::Item {
                    module: module.clone(),
                    page,
                    item,
                })])
            }
            // Surfaces added to the API after this version of the host.
            unknown => {
                tracing::warn!(%module, surface = ?unknown, "unsupported feedback surface");
                Vec::new()
            }
        }
    }

    /// The `surfaces` shown on the last drawn frame: a mark is only ever
    /// drawn over what the user sees when the outcome comes, never later
    /// over a surface shown afterwards.
    pub(super) fn displayed(&self, mut surfaces: Vec<Surface>) -> Vec<Surface> {
        let Some(frame) = &self.last_frame else {
            return Vec::new();
        };
        surfaces.retain(|surface| match surface {
            Surface::Key(target) => frame
                .keys
                .iter()
                .any(|key| key.target.as_ref() == Some(target)),
            Surface::Dial(dial) => frame
                .strip
                .as_ref()
                .is_some_and(|strip| usize::from(*dial) < strip.hints.len()),
        });
        surfaces
    }

    pub(super) fn show_feedback(&mut self, surfaces: &[Surface], feedback: Feedback) {
        let now = Instant::now();
        for surface in surfaces {
            self.marks.show(surface.clone(), feedback, now);
        }
    }

    pub(super) fn conclude(
        &mut self,
        module: &ModuleInstanceId,
        surfaces: &[Surface],
        outcome: Outcome,
    ) {
        match outcome {
            Outcome::Done(commands) => self.apply_commands(module, surfaces, commands),
            Outcome::Failed => self.show_feedback(surfaces, Feedback::Alert),
        }
    }
}
