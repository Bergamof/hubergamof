//! Microsoft Teams module, driven through a browser extension (the
//! bridge, see [`bridge`]): one key showing the unread conversations or the
//! call, a page of the unread conversations and a page of the call (mute,
//! camera, hang up, or answer while it rings).
//!
//! The extension reads Teams only; nothing here reads the content of a
//! message. Muting and hanging up stay usable while the computer is locked
//! (declared per item, see `docs/security.md`), and a call keeps the device
//! lit while its page is displayed.

mod bridge;
pub mod native_host;
pub mod wire;

use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use hubergamof_module_api::{
    Command, Feedback, ImageData, ItemId, KeyContent, KeyState, Module, ModuleCapabilities,
    ModuleContext, ModuleDescriptor, ModuleError, ModuleEvent, ModuleFactory, PageContent, PageId,
    PageItem,
};
use serde_json::Value;
use tokio::sync::watch;

pub use bridge::{
    CallState, RequestError, SimulatedBridge, SocketBridge, TeamsBridge, TeamsState, is_same_user,
    socket_path,
};
use wire::{CallAction, CallPhase};

use crate::icons::{asset, svg};

/// Kind of the Teams module.
pub const TEAMS_KIND: &str = "teams";

/// Page of the unread conversations.
const UNREAD_PAGE: &str = "unread";

/// Page of the call in progress or ringing.
const CALL_PAGE: &str = "call";

const MODULE_LABEL: &str = "Teams";
const UNREAD_TITLE: &str = "Teams · non lus";
const IN_CALL_LABEL: &str = "En appel";
const RINGING_LABEL: &str = "Appel";

/// Key icon: two speech bubbles (a generic drawing: no trademark).
static MODULE_ICON: LazyLock<ImageData> = LazyLock::new(|| svg(asset!("teams")));

/// Teams changed, as the bridge reports: the new state is read from the
/// bridge, so an event lost in a full queue costs a redraw at most.
struct TeamsChanged;

/// Creates [`Teams`] instances.
#[derive(Debug, Clone)]
pub struct TeamsFactory {
    bridge: Arc<dyn TeamsBridge>,
}

impl TeamsFactory {
    /// Factory of instances reading and driving Teams through `bridge`.
    #[must_use]
    pub fn new(bridge: Arc<dyn TeamsBridge>) -> Self {
        Self { bridge }
    }
}

impl ModuleFactory for TeamsFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor {
            kind: TEAMS_KIND.into(),
            name: MODULE_LABEL.into(),
            description: "Teams (navigateur, extension HuBergamof) : conversations non lues, \
                          appel en cours (micro, caméra, raccrocher)."
                .into(),
            capabilities: ModuleCapabilities {
                key: true,
                // The call page declares what stays usable while locked.
                lock_safe_pages: true,
                ..Default::default()
            },
            dial_functions: Vec::new(),
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
        let changes = self.bridge.state();
        let mut watched = changes.clone();
        let events = ctx.events().clone();
        ctx.tasks().spawn(async move {
            while watched.changed().await.is_ok() {
                // A refusal is a full queue as much as a dead instance: the
                // next change tries again, and the tasks of a dead instance
                // are stopped with it.
                events.send(TeamsChanged);
            }
        });
        Ok(Box::new(Teams {
            bridge: Arc::clone(&self.bridge),
            changes,
            call_announced: false,
        }))
    }
}

/// One instance: Teams as the bridge last reported.
pub(crate) struct Teams {
    bridge: Arc<dyn TeamsBridge>,
    changes: watch::Receiver<TeamsState>,
    /// The page of the call was opened for the call in progress.
    call_announced: bool,
}

/// A key of the call page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallItem {
    Mute,
    Camera,
    HangUp,
    Accept,
    Decline,
}

impl CallItem {
    const ALL: [Self; 5] = [
        Self::Mute,
        Self::Camera,
        Self::HangUp,
        Self::Accept,
        Self::Decline,
    ];

    fn id(self) -> ItemId {
        ItemId(
            match self {
                Self::Mute => "mute",
                Self::Camera => "camera",
                Self::HangUp => "hangup",
                Self::Accept => "accept",
                Self::Decline => "decline",
            }
            .into(),
        )
    }

    fn from_id(id: &ItemId) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.id() == *id)
    }

    /// What Teams is asked when the key is pressed.
    fn action(self) -> CallAction {
        match self {
            Self::Mute => CallAction::ToggleMute,
            Self::Camera => CallAction::ToggleCamera,
            Self::HangUp => CallAction::HangUp,
            Self::Accept => CallAction::Accept,
            Self::Decline => CallAction::Decline,
        }
    }

    /// The key, built from what it shows; only muting and hanging up are
    /// declared usable while the computer is locked.
    fn item(self, key: KeyContent) -> PageItem {
        let lock_safe = matches!(self, Self::Mute | Self::HangUp);
        PageItem::new(self.id(), key).usable_when_locked(lock_safe)
    }
}

impl Teams {
    fn unread_page(&self) -> PageContent {
        let state = self.changes.borrow();
        let known = state.conversations.len();
        let hidden = usize::try_from(state.unread)
            .unwrap_or(usize::MAX)
            .saturating_sub(known);
        let mut items: Vec<PageItem> = state
            .conversations
            .iter()
            .enumerate()
            .map(|(index, name)| {
                PageItem::new(
                    ItemId(format!("c{index}")),
                    KeyContent::with_label(name.clone()),
                )
            })
            .collect();
        if hidden > 0 {
            items.push(PageItem::new(
                ItemId("more".into()),
                KeyContent::with_label(format!("+{hidden}")),
            ));
        }
        PageContent::new(UNREAD_TITLE, items)
    }

    fn call_page(&self) -> PageContent {
        let state = self.changes.borrow();
        let call = &state.call;
        let ringing = call.phase == CallPhase::Ringing;
        let items = if ringing {
            // The caller is on a key, not in the title: titles are logged.
            let caller = call.caller.clone().unwrap_or_else(|| "?".to_owned());
            vec![
                PageItem::new(ItemId("caller".into()), KeyContent::with_label(caller)),
                CallItem::Accept.item(KeyContent::with_label("Répondre")),
                CallItem::Decline.item(KeyContent::with_label("Refuser")),
            ]
        } else {
            vec![
                CallItem::Mute.item(mute_key(call.muted)),
                CallItem::Camera.item(camera_key(call.camera_on)),
                CallItem::HangUp
                    .item(KeyContent::with_label("Raccrocher").in_state(KeyState::Attention)),
            ]
        };
        PageContent::new("Appel", items).keeps_device_lit(call.phase == CallPhase::InCall)
    }

    /// Asks Teams for `action`: ✓ once sent, ⚠ when no extension is there.
    fn ask(&self, action: CallAction) -> Vec<Command> {
        match self.bridge.request(action) {
            Ok(()) => vec![Command::Feedback(Feedback::Ok)],
            Err(error) => {
                tracing::debug!(%error, ?action, "Teams action not sent");
                Command::alert()
            }
        }
    }
}

fn mute_key(muted: Option<bool>) -> KeyContent {
    match muted {
        Some(true) => KeyContent::with_label("Micro coupé").in_state(KeyState::Attention),
        Some(false) => KeyContent::with_label("Micro actif"),
        None => KeyContent::with_label("Micro"),
    }
}

fn camera_key(camera_on: Option<bool>) -> KeyContent {
    match camera_on {
        Some(true) => KeyContent::with_label("Caméra on").in_state(KeyState::Active),
        Some(false) => KeyContent::with_label("Caméra off"),
        None => KeyContent::with_label("Caméra"),
    }
}

#[async_trait]
impl Module for Teams {
    fn key(&self) -> KeyContent {
        let teams = self.changes.borrow();
        let (label, state) = match teams.call.phase {
            CallPhase::InCall => (IN_CALL_LABEL.to_owned(), KeyState::Active),
            CallPhase::Ringing => (RINGING_LABEL.to_owned(), KeyState::Attention),
            CallPhase::Idle | CallPhase::Prejoin if teams.unread > 0 => {
                (format!("{} non lus", teams.unread), KeyState::Attention)
            }
            CallPhase::Idle | CallPhase::Prejoin => (MODULE_LABEL.to_owned(), KeyState::Normal),
        };
        KeyContent::with_icon(&MODULE_ICON, label).in_state(state)
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        let (active, unread) = {
            let state = self.changes.borrow();
            (state.call.phase.is_active(), state.unread)
        };
        if active {
            vec![Command::OpenPage(CALL_PAGE.into())]
        } else if unread > 0 {
            vec![Command::OpenPage(UNREAD_PAGE.into())]
        } else {
            Command::alert()
        }
    }

    fn page(&self, page: &PageId) -> Option<PageContent> {
        match page.0.as_str() {
            UNREAD_PAGE => Some(self.unread_page()),
            CALL_PAGE if self.changes.borrow().call.phase.is_active() => Some(self.call_page()),
            _ => None,
        }
    }

    async fn on_item_press(&mut self, page: &PageId, item: &ItemId) -> Vec<Command> {
        if page.0 != CALL_PAGE {
            return Vec::new();
        }
        let Some(item) = CallItem::from_id(item) else {
            return Vec::new();
        };
        let action = item.action();
        self.ask(action)
    }

    async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
        if !event.is::<TeamsChanged>() {
            return Vec::new();
        }
        let active = self.changes.borrow().call.phase.is_active();
        let call_starts = active && !self.call_announced;
        self.call_announced = active;
        if call_starts {
            // A call on the device wakes it and shows its page.
            vec![Command::OpenPage(CALL_PAGE.into())]
        } else {
            Vec::new()
        }
    }

    fn update_settings(&mut self, _settings: &Value) -> Result<(), ModuleError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
