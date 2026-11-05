//! What a module following one desktop application (Claude, IntelliJ)
//! keeps of it: its state, the attention its key shows, and its actions on
//! the desktop (bringing a window to the front, launching), whose outcomes
//! come back to the instance as [`AppEvent`]s.

use std::sync::Arc;

use bergadeck_module_api::{
    Command, EventSender, Feedback, KeyState, ModuleContext, ModuleSurface,
};

use super::attention::{AppAttention, forward_notifications};
use super::backend::DesktopError;
use super::hub::{DesktopAction, DesktopHub, DesktopState, DesktopSubscription, LaunchStart};
use super::model::{AppId, AppState, Launch, Window, WindowId};
use crate::background::forward_states;

/// What a [`FollowedApp`] hands its instance through the event sender, to
/// give back to [`FollowedApp::handle`].
#[derive(Debug)]
pub enum AppEvent {
    /// What the desktop knows now.
    Desktop(DesktopState),
    /// The application sent a notification.
    Notified,
    /// An action ended.
    Ended(ActionOutcome),
}

/// How an action started from a surface of the module ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionOutcome {
    /// Where it was started.
    pub surface: ModuleSurface,
    /// What it was.
    pub action: DesktopAction,
    /// Whether it succeeded.
    pub ok: bool,
}

impl ActionOutcome {
    /// The mark of the outcome over its surface.
    #[must_use]
    pub fn feedback(&self) -> Command {
        Command::FeedbackOn(self.surface.clone(), Feedback::of(self.ok))
    }
}

/// One application as an instance follows it.
#[derive(Debug)]
pub struct FollowedApp {
    app: AppId,
    desktop: DesktopSubscription,
    state: AppState,
    attention: AppAttention,
    events: EventSender,
}

impl FollowedApp {
    /// Follows `app` through `hub` for the instance of `ctx`, its key in
    /// `attention` as given: its states and notifications come as
    /// [`AppEvent`]s until the instance ends.
    ///
    /// # Panics
    ///
    /// In debug builds, when `hub` does not follow `app` (it would never be
    /// seen nor launched).
    #[must_use]
    pub fn follow(
        hub: &Arc<DesktopHub>,
        app: AppId,
        attention: AppAttention,
        ctx: &ModuleContext,
    ) -> Self {
        debug_assert!(hub.follows(app), "the desktop hub does not follow {app:?}");
        let desktop = hub.subscribe();
        let events = ctx.events().clone();
        ctx.tasks().spawn(forward_states(
            desktop.states(),
            events.clone(),
            AppEvent::Desktop,
        ));
        ctx.tasks().spawn(forward_notifications(
            desktop.notifications(),
            app,
            events.clone(),
            || AppEvent::Notified,
        ));
        Self {
            app,
            desktop,
            state: AppState::default(),
            attention,
            events,
        }
    }

    /// What is known of the application.
    #[must_use]
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Its windows as followed (none when they cannot be).
    #[must_use]
    pub fn windows(&self) -> &[Window] {
        self.state.windows.as_deref().unwrap_or_default()
    }

    /// State of its key: in attention (when it asks, or `also_attention`),
    /// active while it runs.
    #[must_use]
    pub fn key_state(&self, also_attention: bool) -> KeyState {
        if self.attention.is_on() || also_attention {
            KeyState::Attention
        } else if self.state.is_running() {
            KeyState::Active
        } else {
            KeyState::Normal
        }
    }

    /// The user pressed its key: the attention of its notifications ends.
    pub fn acknowledge(&mut self) {
        self.attention.acknowledge();
    }

    /// Takes `event` into account; the outcome of an action that ended.
    pub fn handle(&mut self, event: AppEvent) -> Option<ActionOutcome> {
        match event {
            AppEvent::Desktop(state) => {
                self.state = state.app(self.app);
                self.attention.app_changed(&self.state);
                None
            }
            AppEvent::Notified => {
                self.attention.notified();
                None
            }
            AppEvent::Ended(outcome) => Some(outcome),
        }
    }

    /// Brings `window` to the front, its outcome marked on `surface`.
    #[must_use]
    pub fn activate(&self, window: WindowId, surface: ModuleSurface) -> Vec<Command> {
        let report = self.reporter(surface, DesktopAction::Activate);
        if self.desktop.activate(window, report) {
            Vec::new()
        } else {
            Command::alert()
        }
    }

    /// Starts `launch`, unless one of the application is under way, its
    /// outcome marked on `surface`.
    #[must_use]
    pub fn launch(&self, launch: Launch, surface: ModuleSurface) -> LaunchStart {
        let report = self.reporter(surface, DesktopAction::Launch);
        self.desktop.launch(self.app, launch, report)
    }

    /// Hands the outcome of an action back to the instance.
    fn reporter(
        &self,
        surface: ModuleSurface,
        action: DesktopAction,
    ) -> impl FnOnce(Result<(), DesktopError>) + Send + 'static {
        let events = self.events.clone();
        let app = self.app;
        move |outcome| {
            if let Err(error) = &outcome {
                // Only the program and why: never a project path.
                tracing::warn!(%error, app = app.0, ?action, "desktop action failed");
            }
            // An instance gone meanwhile needs no outcome.
            let _ = events.send(AppEvent::Ended(ActionOutcome {
                surface,
                action,
                ok: outcome.is_ok(),
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use bergadeck_module_api::testing::TestContext;

    use super::*;
    use crate::desktop::SimulatedDesktop;

    #[tokio::test]
    #[should_panic(expected = "does not follow")]
    async fn only_an_application_of_the_hub_is_followed() {
        let hub = DesktopHub::new(Arc::new(SimulatedDesktop::empty()), Vec::new());
        let TestContext { ctx, .. } = TestContext::new("app");
        let _app = FollowedApp::follow(&hub, AppId("app"), AppAttention::with_windows(), &ctx);
    }

    #[test]
    fn an_outcome_marks_its_surface() {
        let outcome = ActionOutcome {
            surface: ModuleSurface::HomeKey,
            action: DesktopAction::Launch,
            ok: false,
        };
        assert_eq!(
            outcome.feedback(),
            Command::FeedbackOn(ModuleSurface::HomeKey, Feedback::Alert)
        );
    }
}
