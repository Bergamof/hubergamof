//! Claude Desktop module: one key opening Claude, or bringing it to the
//! front, in attention while Claude asks for something.
//!
//! - Closed: pressing starts it (through its desktop entry).
//! - Open with a window: pressing brings its first window to the front.
//! - Open in the system tray only, or windows not followed (no KWin):
//!   pressing starts it again, which shows the window of the running
//!   instance.
//! - In attention when Claude sent a notification in the background — the
//!   desktop application, or claude.ai in Brave (recognised by the browser's
//!   executable and the site it names) — until the key is pressed or Claude
//!   Desktop is in front. The texts are neither shown nor read. Its windows
//!   asking for attention do not count: Claude Desktop asks whenever it
//!   loses the focus.
//!
//! Only ever launches a fixed program with a fixed argument, never a value
//! from the configuration; one launch at a time, whatever the instance.

use async_trait::async_trait;
use hubergamof_module_api::{
    Command, ImageData, KeyContent, Module, ModuleCapabilities, ModuleContext, ModuleDescriptor,
    ModuleError, ModuleEvent, ModuleFactory, ModuleSurface,
};
use serde_json::Value;
use std::sync::{Arc, LazyLock};

use crate::desktop::{
    AppAttention, AppEvent, AppId, ClassMatch, DesktopAction, DesktopApp, DesktopHub,
    ExecutableMatch, FollowedApp, Launch, LaunchStart, NotificationRelay, WindowId,
};
use crate::icons::{asset, svg};

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

/// Brave's main executable: Arch/Manjaro package, then Brave's own
/// packages.
const BRAVE_EXECUTABLES: [&str; 2] = ["/opt/brave-bin/brave", "/opt/brave.com/brave/brave"];

/// Desktop entry of Brave.
const BRAVE_ENTRY: &str = "brave-browser";

/// Site of Claude's web version.
const WEB_ORIGIN: &str = "claude.ai";

/// Starts a desktop entry, as the desktop's launchers do (looked up in
/// `PATH`, shipped with GTK).
const LAUNCHER: &str = "gtk-launch";

/// Key icon: a speech bubble with a spark, a conversation with an assistant
/// (a generic drawing: no trademark).
const MODULE_SVG: &str = asset!("claude");

static MODULE_ICON: LazyLock<ImageData> = LazyLock::new(|| svg(MODULE_SVG));

const MODULE_LABEL: &str = "Claude";
const OPENING_LABEL: &str = "Ouverture…";

/// Claude Desktop, as the desktop layer follows it.
#[must_use]
pub(crate) fn desktop_app() -> DesktopApp {
    DesktopApp {
        id: APP,
        window_class: ClassMatch::Exact(APP_ID.into()),
        executable: ExecutableMatch {
            path: EXECUTABLE.into(),
            helper_argument: Some(HELPER_ARGUMENT.into()),
        },
        desktop_entry: Some(APP_ID.into()),
        relays: vec![web_in_brave()],
    }
}

/// claude.ai in Brave: its notifications are Claude's too.
fn web_in_brave() -> NotificationRelay {
    NotificationRelay {
        executables: BRAVE_EXECUTABLES
            .iter()
            .map(|path| ExecutableMatch {
                path: path.into(),
                helper_argument: None,
            })
            .collect(),
        desktop_entry: BRAVE_ENTRY.into(),
        origin: WEB_ORIGIN.into(),
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
    pub(crate) fn new(desktop: Arc<DesktopHub>) -> Self {
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
        Some(MODULE_ICON.clone())
    }

    fn create(
        &self,
        ctx: ModuleContext,
        _settings: &Value,
    ) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(Claude {
            app: FollowedApp::follow(&self.desktop, APP, AppAttention::notifications_only(), &ctx),
            launching: false,
        }))
    }
}

/// One instance: Claude as last known.
pub(crate) struct Claude {
    /// Claude Desktop; in attention on notifications only.
    app: FollowedApp,
    /// This instance started a launch, under way.
    launching: bool,
}

impl Claude {
    /// The window a press brings to the front.
    fn window_to_front(&self) -> Option<WindowId> {
        let windows = self.app.windows();
        windows.first().map(|window| window.id.clone())
    }

    /// Starts Claude, unless a launch is under way (from any instance).
    fn launch(&mut self) -> Vec<Command> {
        match self.app.launch(launch(), ModuleSurface::HomeKey) {
            LaunchStart::Started => {
                self.launching = true;
                Vec::new()
            }
            LaunchStart::Busy => Vec::new(),
            LaunchStart::Refused => Command::alert(),
        }
    }
}

#[async_trait]
impl Module for Claude {
    fn key(&self) -> KeyContent {
        let label = if self.launching {
            OPENING_LABEL
        } else {
            MODULE_LABEL
        };
        KeyContent::with_icon(&MODULE_ICON, label).in_state(self.app.key_state(false))
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        self.app.acknowledge();
        match self.window_to_front() {
            Some(window) => self.app.activate(window, ModuleSurface::HomeKey),
            // Closed, or in the system tray only.
            None => self.launch(),
        }
    }

    async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
        let Ok(event) = event.downcast::<AppEvent>() else {
            return Vec::new();
        };
        let Some(outcome) = self.app.handle(*event) else {
            return Vec::new();
        };
        if outcome.action == DesktopAction::Launch {
            self.launching = false;
        }
        vec![outcome.feedback()]
    }

    fn update_settings(&mut self, _settings: &Value) -> Result<(), ModuleError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
