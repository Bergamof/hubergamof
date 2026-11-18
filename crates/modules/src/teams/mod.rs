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
mod icons;
pub mod native_host;
mod reminders;
pub mod wire;

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use async_trait::async_trait;
use hubergamof_module_api::{
    Command, ContextDial, ImageData, ItemId, KeyContent, KeyState, Module, ModuleCapabilities,
    ModuleContext, ModuleDescriptor, ModuleError, ModuleEvent, ModuleFactory, Notification,
    NotificationId, PageContent, PageId, PageItem, Rgb,
};
use serde_json::Value;
use tokio::sync::watch;

pub use bridge::{
    CallState, RequestError, SimulatedBridge, SocketBridge, TeamsBridge, TeamsState, is_same_user,
    socket_path,
};
use icons::{ACCEPT, CAMERA, CAMERA_OFF, CONVERSATION, HANG_UP, MICROPHONE, MICROPHONE_MUTED};
use reminders::MeetingSoon;
pub use reminders::{TeamsSettings, WallClock};
use wire::{CallAction, CallPhase, UnreadConversation};

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
const OFFLINE_LABEL: &str = "Hors ligne";
const ANSWER_LABEL: &str = "Répondre";

/// Background of the notifications of a meeting and of an incoming call.
const MEETING_BACKGROUND: Rgb = Rgb(0x46, 0x4E, 0xB8);
const CALL_BACKGROUND: Rgb = Rgb(0x16, 0xA0, 0x6E);

/// How long the notification of a meeting stays on the strip.
const MEETING_NOTICE: Duration = Duration::from_secs(60);

/// Key icon: the Teams tile with two soft-serve mascots (drawn by Adrien).
static MODULE_ICON: LazyLock<ImageData> = LazyLock::new(|| svg(asset!("teams")));

/// Teams changed, as the bridge reports: the new state is read from the
/// bridge, so an event lost in a full queue costs a redraw at most.
struct TeamsChanged;

/// Creates [`Teams`] instances.
#[derive(Debug, Clone)]
pub struct TeamsFactory {
    bridge: Arc<dyn TeamsBridge>,
    clock: WallClock,
}

impl TeamsFactory {
    /// Factory of instances reading and driving Teams through `bridge`.
    #[must_use]
    pub fn new(bridge: Arc<dyn TeamsBridge>) -> Self {
        Self::with_clock(bridge, WallClock::system())
    }

    /// Factory whose reminders read the time of day from `clock`.
    #[must_use]
    pub fn with_clock(bridge: Arc<dyn TeamsBridge>, clock: WallClock) -> Self {
        Self { bridge, clock }
    }
}

fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        kind: TEAMS_KIND.into(),
        name: MODULE_LABEL.into(),
        description: "Teams (navigateur, extension HuBergamof) : conversations non lues, \
                      appel en cours (micro, caméra, raccrocher), rappel des réunions."
            .into(),
        capabilities: ModuleCapabilities {
            key: true,
            // The call page declares what stays usable while locked.
            lock_safe_pages: true,
            ..Default::default()
        },
        dial_functions: Vec::new(),
        settings: reminders::fields(),
    }
}

impl ModuleFactory for TeamsFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor()
    }

    fn icon(&self) -> Option<ImageData> {
        Some(MODULE_ICON.clone())
    }

    fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
        let settings: TeamsSettings = descriptor().parse_settings(settings)?;
        let (lead, lead_changes) = watch::channel(settings.reminder_minutes);
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
        ctx.tasks().spawn(reminders::run(
            changes.clone(),
            lead_changes,
            self.clock.clone(),
            ctx.events().clone(),
        ));
        Ok(Box::new(Teams {
            bridge: Arc::clone(&self.bridge),
            changes,
            lead,
            call_announced: false,
            ringing_announced: false,
        }))
    }
}

/// One instance: Teams as the bridge last reported.
pub(crate) struct Teams {
    bridge: Arc<dyn TeamsBridge>,
    changes: watch::Receiver<TeamsState>,
    /// Lead time of the meeting reminders, read by their task.
    lead: watch::Sender<u32>,
    /// The page of the call was opened for the call in progress.
    call_announced: bool,
    /// The ringing call was announced on the strip.
    ringing_announced: bool,
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
        PageItem::new(self.id(), key.usable_when_locked(lock_safe))
    }
}

impl Teams {
    fn unread_page(&self) -> PageContent {
        PageContent::new(UNREAD_TITLE, unread_items(&self.changes.borrow()))
    }

    fn call_page(&self) -> PageContent {
        // One reading of the state: the call and the unread conversations of a
        // page always belong together.
        let state = self.changes.borrow();
        let call = &state.call;
        if call.phase == CallPhase::Ringing {
            // The caller is on a key, not in the title: titles are logged.
            let caller = call.caller.clone().unwrap_or_else(|| "?".to_owned());
            let items = vec![
                PageItem::new(ItemId("caller".into()), KeyContent::with_label(caller)),
                CallItem::Accept.item(KeyContent::with_icon(&ACCEPT, ANSWER_LABEL)),
                CallItem::Decline.item(KeyContent::with_icon(&HANG_UP, "Refuser")),
            ];
            let pinned = items.len();
            return PageContent::new("Appel", items).with_pinned_items(pinned);
        }
        // In a call the actions stay in place on the first row and the unread
        // conversations scroll below them.
        let actions = [
            CallItem::Mute.item(mute_key(call.muted)),
            CallItem::Camera.item(camera_key(call.camera_on)),
            CallItem::HangUp.item(KeyContent::with_icon(&HANG_UP, "Raccrocher")),
        ];
        let pinned = actions.len();
        let items = [actions.to_vec(), unread_items(&state)].concat();
        PageContent::new("Appel", items)
            .with_pinned_items(pinned)
            .keeps_device_lit(call.phase == CallPhase::InCall)
    }

    /// Asks Teams for `action`: nothing once sent (the key changes its icon
    /// when it worked), ⚠ when no extension is there.
    fn ask(&self, action: CallAction) -> Vec<Command> {
        match self.bridge.request(action) {
            Ok(()) => Vec::new(),
            Err(error) => {
                tracing::debug!(%error, ?action, "Teams action not sent");
                Command::alert()
            }
        }
    }
}

/// The notification of a ringing call; its press answers.
///
/// The caller is on the strip, which is not logged; answering is not usable
/// while the computer is locked (unlike muting and hanging up).
fn call_notification(caller: Option<&str>) -> Notification {
    let message = caller.map_or_else(
        || "Appel entrant".to_owned(),
        |name| format!("Appel : {name}"),
    );
    Notification::new(message, CALL_BACKGROUND)
        .with_context_dial(ContextDial::none().pressing(ANSWER_LABEL))
}

/// One key per unread conversation, with its avatar or a generic icon, then a
/// "+N" key for those whose name is unknown. Hidden while the computer is
/// locked: a name must not be read by someone passing by.
fn unread_items(state: &TeamsState) -> Vec<PageItem> {
    let hidden = usize::try_from(state.unread)
        .unwrap_or(usize::MAX)
        .saturating_sub(state.conversations.len());
    let mut items: Vec<PageItem> = state
        .conversations
        .iter()
        .enumerate()
        .map(|(index, conversation)| conversation_item(index, conversation))
        .collect();
    if hidden > 0 {
        items.push(
            PageItem::new(
                ItemId("more".into()),
                KeyContent::with_label(format!("+{hidden}")),
            )
            .hidden_when_locked(true),
        );
    }
    items
}

/// A conversation key: its avatar when the extension could read one.
fn conversation_item(index: usize, conversation: &UnreadConversation) -> PageItem {
    let icon = conversation.icon.as_ref().unwrap_or(&CONVERSATION);
    PageItem::new(
        ItemId(format!("c{index}")),
        KeyContent::with_icon(icon, conversation.name.clone()),
    )
    .hidden_when_locked(true)
}

/// The icon says whether the microphone is on, as the label does.
fn mute_key(muted: Option<bool>) -> KeyContent {
    match muted {
        Some(true) => KeyContent::with_icon(&MICROPHONE_MUTED, "Micro coupé"),
        Some(false) => KeyContent::with_icon(&MICROPHONE, "Micro actif"),
        None => KeyContent::with_icon(&MICROPHONE, "Micro"),
    }
}

fn camera_key(camera_on: Option<bool>) -> KeyContent {
    match camera_on {
        Some(true) => KeyContent::with_icon(&CAMERA, "Caméra on"),
        Some(false) => KeyContent::with_icon(&CAMERA_OFF, "Caméra off"),
        None => KeyContent::with_icon(&CAMERA, "Caméra"),
    }
}

#[async_trait]
impl Module for Teams {
    fn key(&self) -> KeyContent {
        let teams = self.changes.borrow();
        if !teams.connected {
            // Nothing below is current without the extension: say so rather
            // than showing a quiet "Teams" that hides the missing link.
            return KeyContent::with_icon(&MODULE_ICON, OFFLINE_LABEL);
        }
        let (label, state) = match teams.call.phase {
            CallPhase::InCall => (IN_CALL_LABEL.to_owned(), KeyState::Active),
            CallPhase::Ringing => (RINGING_LABEL.to_owned(), KeyState::Attention),
            CallPhase::Idle | CallPhase::Prejoin if teams.unread > 0 => {
                (format!("{} non lus", teams.unread), KeyState::Attention)
            }
            CallPhase::Idle | CallPhase::Prejoin => (MODULE_LABEL.to_owned(), KeyState::Normal),
        };
        // A call in progress or ringing is reached from the home key even on
        // a locked computer: it only opens the page of the call, whose keys
        // declare what stays usable.
        KeyContent::with_icon(&MODULE_ICON, label)
            .in_state(state)
            .usable_when_locked(teams.call.phase == CallPhase::InCall)
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        let (connected, active, unread) = {
            let state = self.changes.borrow();
            (state.connected, state.call.phase.is_active(), state.unread)
        };
        if !connected {
            Command::alert()
        } else if active {
            vec![Command::OpenPage(CALL_PAGE.into())]
        } else if unread > 0 {
            vec![Command::OpenPage(UNREAD_PAGE.into())]
        } else {
            // Connected with nothing to show: not a failure.
            Vec::new()
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
        if let Some(meeting) = event.downcast_ref::<MeetingSoon>() {
            let notification = Notification::new(meeting.message.clone(), MEETING_BACKGROUND)
                .with_timeout(MEETING_NOTICE);
            return vec![Command::Notify(notification)];
        }
        if !event.is::<TeamsChanged>() {
            return Vec::new();
        }
        let (phase, caller) = {
            let state = self.changes.borrow();
            (state.call.phase, state.call.caller.clone())
        };
        let mut commands = Vec::new();
        let ringing = phase == CallPhase::Ringing;
        if ringing && !self.ringing_announced {
            commands.push(Command::Notify(call_notification(caller.as_deref())));
        }
        self.ringing_announced = ringing;
        let active = phase.is_active();
        let call_starts = active && !self.call_announced;
        self.call_announced = active;
        if call_starts {
            // A call on the device wakes it and shows its page.
            commands.push(Command::OpenPage(CALL_PAGE.into()));
        }
        commands
    }

    async fn on_notification_press(&mut self, _id: NotificationId) -> Vec<Command> {
        // Only the notification of a ringing call has a press; one that
        // stopped ringing meanwhile is left alone.
        if self.changes.borrow().call.phase == CallPhase::Ringing {
            self.ask(CallAction::Accept)
        } else {
            Vec::new()
        }
    }

    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError> {
        let settings: TeamsSettings = descriptor().parse_settings(settings)?;
        self.lead.send_replace(settings.reminder_minutes);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
