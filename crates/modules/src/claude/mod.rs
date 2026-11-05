//! Claude Desktop module: one key opening Claude, or bringing it to the
//! front, in attention while Claude asks for something.
//!
//! - Closed: pressing starts it (through its desktop entry).
//! - Open with a window: pressing brings it to the front (the one asking
//!   for attention first).
//! - Open in the system tray only, or windows not followed (no KWin):
//!   pressing starts it again, which shows the window of the running
//!   instance.
//! - In attention when one of its windows asks for it, or when Claude sent
//!   a notification (permission asked, answer ready: sent in the
//!   background) until the key is pressed or Claude is in front. The texts
//!   are neither shown nor read.
//!
//! Only ever launches a fixed program with a fixed argument, never a value
//! from the configuration; one launch at a time, whatever the instance.

mod icons;

use async_trait::async_trait;
use bergadeck_module_api::{
    Command, EventSender, Feedback, ImageData, KeyContent, KeyState, Module, ModuleCapabilities,
    ModuleContext, ModuleDescriptor, ModuleError, ModuleEvent, ModuleFactory, ModuleSurface,
};
use serde_json::Value;
use std::sync::Arc;

use crate::background::forward_states;
use crate::desktop::{
    AppAttention, AppId, AppState, ClassMatch, DesktopAction, DesktopApp, DesktopError, DesktopHub,
    DesktopState, DesktopSubscription, ExecutableMatch, Launch, LaunchStart, WindowId,
    forward_notifications,
};

/// Kind of the Claude module.
pub const CLAUDE_KIND: &str = "claude";

/// Claude among the applications of the desktop layer.
const APP: AppId = AppId(CLAUDE_KIND);

/// Class of the Claude Desktop windows, also its desktop entry.
const APP_ID: &str = "com.anthropic.Claude";

/// Main executable of Claude Desktop (Electron).
const EXECUTABLE: &str = "/usr/lib/claude-desktop/claude-desktop";

/// Argument of the Electron helper processes (renderers, GPU…), not Claude
/// itself.
const HELPER_ARGUMENT: &str = "--type=";

/// Starts a desktop entry, as the desktop's launchers do (looked up in
/// `PATH`, shipped with GTK).
const LAUNCHER: &str = "gtk-launch";

const MODULE_LABEL: &str = "Claude";
const OPENING_LABEL: &str = "Ouverture…";

/// Claude Desktop, as the desktop layer follows it.
#[must_use]
pub fn desktop_app() -> DesktopApp {
    DesktopApp {
        id: APP,
        window_class: ClassMatch::Exact(APP_ID.into()),
        executable: ExecutableMatch {
            path: EXECUTABLE.into(),
            helper_argument: Some(HELPER_ARGUMENT.into()),
        },
        desktop_entry: Some(APP_ID.into()),
    }
}

/// What starts Claude Desktop.
fn launch() -> Launch {
    Launch {
        program: LAUNCHER.into(),
        args: vec![APP_ID.into()],
    }
}

/// Creates [`Claude`] instances.
#[derive(Debug, Clone)]
pub struct ClaudeFactory {
    desktop: Arc<DesktopHub>,
}

impl ClaudeFactory {
    /// Factory following Claude through `desktop` (which follows
    /// [`desktop_app`]).
    #[must_use]
    pub fn new(desktop: Arc<DesktopHub>) -> Self {
        Self { desktop }
    }
}

impl ModuleFactory for ClaudeFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor {
            kind: CLAUDE_KIND.into(),
            name: MODULE_LABEL.into(),
            description: "Claude Desktop : l'ouvrir ou passer à sa fenêtre ; touche en attention \
                          quand Claude demande quelque chose."
                .into(),
            capabilities: ModuleCapabilities {
                key: true,
                dial: false,
                touch_zone: false,
            },
            settings: Vec::new(),
        }
    }

    fn icon(&self) -> Option<ImageData> {
        Some(icons::MODULE.clone())
    }

    fn create(
        &self,
        ctx: ModuleContext,
        _settings: &Value,
    ) -> Result<Box<dyn Module>, ModuleError> {
        let desktop = self.desktop.subscribe();
        let events = ctx.events().clone();
        ctx.tasks().spawn(forward_states(
            desktop.states(),
            events.clone(),
            ClaudeEvent::Desktop,
        ));
        ctx.tasks().spawn(forward_notifications(
            desktop.notifications(),
            APP,
            events.clone(),
            || ClaudeEvent::Notified,
        ));
        Ok(Box::new(Claude {
            desktop,
            app: AppState::default(),
            attention: AppAttention::default(),
            events,
            launching: false,
        }))
    }
}

/// What an instance receives through its event sender.
#[derive(Debug)]
enum ClaudeEvent {
    Desktop(DesktopState),
    /// Claude sent a notification.
    Notified,
    /// The action started from the key ended.
    Outcome {
        action: DesktopAction,
        ok: bool,
    },
}

/// One instance: Claude as last known.
pub struct Claude {
    desktop: DesktopSubscription,
    app: AppState,
    /// Claude asks for something (window asking, notification).
    attention: AppAttention,
    events: EventSender,
    /// This instance started a launch, under way.
    launching: bool,
}

impl Claude {
    /// The window a press brings to the front: the first one asking for
    /// attention, otherwise the first one.
    fn window_to_front(&self) -> Option<WindowId> {
        let windows = self.app.windows.as_deref().unwrap_or_default();
        windows
            .iter()
            .find(|window| window.demands_attention)
            .or_else(|| windows.first())
            .map(|window| window.id.clone())
    }

    fn handle(&mut self, event: ClaudeEvent) -> Vec<Command> {
        match event {
            ClaudeEvent::Desktop(state) => {
                self.app = state.app(APP);
                self.attention.app_changed(&self.app);
                Vec::new()
            }
            ClaudeEvent::Notified => {
                self.attention.notified();
                Vec::new()
            }
            ClaudeEvent::Outcome { action, ok } => {
                if action == DesktopAction::Launch {
                    self.launching = false;
                }
                let feedback = if ok { Feedback::Ok } else { Feedback::Alert };
                vec![Command::FeedbackOn(ModuleSurface::HomeKey, feedback)]
            }
        }
    }

    /// Hands the outcome of an action back to the instance.
    fn reporter(
        &self,
        action: DesktopAction,
    ) -> impl FnOnce(Result<(), DesktopError>) + Send + 'static {
        let events = self.events.clone();
        move |outcome| {
            if let Err(error) = &outcome {
                tracing::warn!(%error, "Claude action failed");
            }
            // An instance gone meanwhile needs no outcome.
            let _ = events.send(ClaudeEvent::Outcome {
                action,
                ok: outcome.is_ok(),
            });
        }
    }

    /// Starts Claude, unless a launch is under way (from any instance).
    fn launch(&mut self) -> Vec<Command> {
        let report = self.reporter(DesktopAction::Launch);
        match self.desktop.launch(APP, launch(), report) {
            LaunchStart::Started => {
                self.launching = true;
                Vec::new()
            }
            LaunchStart::Busy => Vec::new(),
            LaunchStart::Refused => alert(),
        }
    }

    fn activate(&self, window: WindowId) -> Vec<Command> {
        if self
            .desktop
            .activate(window, self.reporter(DesktopAction::Activate))
        {
            Vec::new()
        } else {
            alert()
        }
    }
}

fn alert() -> Vec<Command> {
    vec![Command::Feedback(Feedback::Alert)]
}

#[async_trait]
impl Module for Claude {
    fn key(&self) -> KeyContent {
        let label = if self.launching {
            OPENING_LABEL
        } else {
            MODULE_LABEL
        };
        let state = if self.attention.is_on() {
            KeyState::Attention
        } else if self.app.is_running() {
            KeyState::Active
        } else {
            KeyState::Normal
        };
        KeyContent {
            image: Some(icons::MODULE.clone()),
            label: Some(label.into()),
            state,
        }
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        self.attention.acknowledge();
        match self.window_to_front() {
            Some(window) => self.activate(window),
            // Closed, or in the system tray only.
            None => self.launch(),
        }
    }

    async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
        match event.downcast::<ClaudeEvent>() {
            Ok(event) => self.handle(*event),
            Err(_) => Vec::new(),
        }
    }

    fn update_settings(&mut self, _settings: &Value) -> Result<(), ModuleError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
