use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hubergamof_module_api::testing::TestContext;
use hubergamof_module_api::{Command, Feedback, KeyState, Notification, NotificationId, TaskScope};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use super::wire::{BrowserReport, Meeting, Panel, UnreadConversation};
use super::*;
use crate::desktop::{SimulatedDesktop, Window, WindowId};

const PATIENCE: Duration = Duration::from_secs(5);

/// Simulated time after which the simulated desktop has said all it had to.
const DESKTOP_QUIET: Duration = Duration::from_secs(5);

fn hub() -> Arc<DesktopHub> {
    DesktopHub::new(Arc::new(SimulatedDesktop::empty()), vec![desktop_app()])
}

/// Two unread conversations, both named, and no rail badge (what an older
/// extension reports).
fn named_unread_bridge() -> SimulatedBridge {
    let bridge = SimulatedBridge::new();
    bridge.report(BrowserReport::Unread { count: 2 });
    bridge.report(BrowserReport::Conversations {
        unread: vec![
            UnreadConversation::named("Équipe produit"),
            UnreadConversation::named("Camille"),
        ],
    });
    bridge
}

fn teams_window() -> Window {
    Window::new(
        "teams",
        "brave-ompifgpmddkgmclendfeacglnodjjndh-Default",
        "Teams",
    )
}

/// Simulated time to wait for a reminder.
const MEETING_WAIT: Duration = Duration::from_secs(600);

struct Harness {
    bridge: Arc<SimulatedBridge>,
    module: Box<dyn Module>,
    events: UnboundedReceiver<ModuleEvent>,
    desktop: Arc<SimulatedDesktop>,
    /// The computer is locked (read by the module when it acts).
    locked: Arc<AtomicBool>,
    _scope: TaskScope,
}

impl Harness {
    fn new(bridge: SimulatedBridge) -> Self {
        Self::with(bridge, WallClock::system(), &json!({}))
    }

    fn with(bridge: SimulatedBridge, clock: WallClock, settings: &Value) -> Self {
        let bridge = Arc::new(bridge);
        let TestContext { ctx, events, scope } = TestContext::new(TEAMS_KIND);
        let desktop = Arc::new(SimulatedDesktop::empty());
        let hub = DesktopHub::new(Arc::clone(&desktop) as _, vec![desktop_app()]);
        let locked = Arc::new(AtomicBool::new(false));
        let ctx = ctx.with_screen_lock(ScreenLockState::new({
            let locked = Arc::clone(&locked);
            move || locked.load(Ordering::SeqCst)
        }));
        let module = TeamsFactory::with_clock(Arc::clone(&bridge) as _, clock, hub)
            .create(ctx, settings)
            .unwrap();
        Self {
            bridge,
            module,
            events,
            desktop,
            locked,
            _scope: scope,
        }
    }

    /// The commands the module answers to what the extension reported.
    async fn report(&mut self, report: BrowserReport) -> Vec<Command> {
        self.bridge.report(report);
        self.next_event().await
    }

    async fn next_event(&mut self) -> Vec<Command> {
        self.next_event_within(PATIENCE).await
    }

    /// The commands of the next event of Teams itself: what the desktop
    /// reports on the way is handed to the module and left aside.
    async fn next_event_within(&mut self, patience: Duration) -> Vec<Command> {
        loop {
            let event = tokio::time::timeout(patience, self.events.recv())
                .await
                .expect("an event")
                .expect("the instance is alive");
            // The desktop and the clock of the call report on their own: not
            // what a test waits for.
            let aside = event.is::<AppEvent>() || event.is::<ElapsedChanged>();
            let commands = self.module.on_event(event).await;
            if !aside {
                return commands;
            }
        }
    }

    /// Hands the module what the desktop reports until it is quiet, and what
    /// the commands among them are.
    async fn settle_desktop(&mut self) -> Vec<Command> {
        let mut commands = Vec::new();
        let mut quiet_at = tokio::time::Instant::now() + DESKTOP_QUIET;
        while let Ok(Some(event)) = tokio::time::timeout_at(quiet_at, self.events.recv()).await {
            // A call's clock ticks every second: it never makes the desktop
            // any less quiet.
            if !event.is::<ElapsedChanged>() {
                quiet_at = tokio::time::Instant::now() + DESKTOP_QUIET;
            }
            commands.extend(self.module.on_event(event).await);
        }
        commands
    }

    async fn in_call(&mut self) -> Vec<Command> {
        self.report(BrowserReport::Call {
            phase: CallPhase::InCall,
            muted: Some(false),
            camera_on: Some(false),
            caller: None,
            elapsed_seconds: None,
        })
        .await
    }

    fn call_page(&self) -> PageContent {
        self.module.page(&CALL_PAGE.into()).expect("a call page")
    }
}

fn open_page(commands: &[Command], page: &str) -> bool {
    commands
        .iter()
        .any(|command| matches!(command, Command::OpenPage(opened) if opened.0 == page))
}

#[test]
fn the_module_may_declare_lock_safe_pages_on_a_single_key_instance() {
    let capabilities = TeamsFactory::new(Arc::new(SimulatedBridge::new()), hub())
        .descriptor()
        .capabilities;
    assert!(capabilities.key && capabilities.lock_safe_pages);
    assert!(!capabilities.multiple_instances && !capabilities.usable_when_locked);
}

#[tokio::test]
async fn the_key_counts_the_unread_conversations() {
    let harness = Harness::new(named_unread_bridge());
    let key = harness.module.key();
    assert_eq!(key.label.as_deref(), Some("2 non lus"));
    assert_eq!(key.state, KeyState::Attention);

    let quiet = Harness::new(SimulatedBridge::new());
    assert_eq!(quiet.module.key().label.as_deref(), Some("Teams"));
    assert_eq!(quiet.module.key().state, KeyState::Normal);
}

#[tokio::test]
async fn pressing_the_key_opens_the_unread_page_and_does_nothing_when_there_is_nothing() {
    let mut harness = Harness::new(named_unread_bridge());
    assert!(open_page(&harness.module.on_key_press().await, UNREAD_PAGE));
    let page = harness.module.page(&UNREAD_PAGE.into()).unwrap();
    let labels: Vec<_> = page
        .items
        .iter()
        .map(|item| item.key.label.clone().unwrap())
        .collect();
    assert_eq!(labels, ["Équipe produit", "Camille"]);

    let mut quiet = Harness::new(SimulatedBridge::new());
    assert_eq!(quiet.module.on_key_press().await, []);
}

#[tokio::test(start_paused = true)]
async fn without_an_extension_the_key_says_so_and_brings_teams_to_the_front() {
    let mut harness = Harness::new(named_unread_bridge());
    harness.desktop.set_windows(vec![teams_window()]);
    harness.settle_desktop().await;
    harness.bridge.set_connected(false);
    harness.next_event().await;
    let key = harness.module.key();
    assert_eq!(key.label.as_deref(), Some("Hors ligne"));
    assert_eq!(key.state, KeyState::Attention);

    assert_eq!(harness.module.on_key_press().await, []);
    assert_eq!(harness.settle_desktop().await, []);
    assert_eq!(harness.desktop.activations(), [WindowId("teams".into())]);
}

#[tokio::test(start_paused = true)]
async fn the_key_with_nothing_to_show_brings_teams_to_the_front_or_starts_it() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.desktop.set_windows(vec![teams_window()]);
    harness.settle_desktop().await;
    assert_eq!(harness.module.on_key_press().await, []);
    // The success is not marked: the window coming up is the answer.
    assert_eq!(harness.settle_desktop().await, []);
    assert_eq!(harness.desktop.activations(), [WindowId("teams".into())]);
    assert_eq!(harness.desktop.launches(), []);

    // Without window, Teams is started (a fixed program and argument).
    let mut closed = Harness::new(SimulatedBridge::new());
    closed.settle_desktop().await;
    assert_eq!(closed.module.on_key_press().await, []);
    closed.settle_desktop().await;
    assert_eq!(closed.desktop.launches(), [launch()]);
    assert_eq!(closed.desktop.activations(), []);
}

#[tokio::test(start_paused = true)]
async fn a_locked_computer_gets_neither_a_window_nor_a_launch() {
    // The key let the press through because a call was showing; the call
    // ended (or the extension left) before it acted.
    for window in [true, false] {
        let mut harness = Harness::new(SimulatedBridge::new());
        if window {
            harness.desktop.set_windows(vec![teams_window()]);
        }
        harness.settle_desktop().await;
        harness.locked.store(true, Ordering::SeqCst);
        assert_eq!(harness.module.on_key_press().await, []);
        assert_eq!(harness.settle_desktop().await, []);
        assert_eq!(harness.desktop.activations(), []);
        assert_eq!(harness.desktop.launches(), []);
    }
}

#[tokio::test(start_paused = true)]
async fn a_locked_computer_does_not_open_a_conversation_nor_return_to_the_call() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.desktop.set_windows(vec![teams_window()]);
    harness.settle_desktop().await;
    harness
        .report(BrowserReport::Conversations {
            unread: vec![conversation_with_id("Alice", "19:alice@thread.v2")],
        })
        .await;
    harness.in_call().await;
    harness.locked.store(true, Ordering::SeqCst);
    let page = harness.call_page();
    let returning = page.items[3].id.clone();
    harness
        .module
        .on_item_press(&CALL_PAGE.into(), &returning)
        .await;
    harness
        .module
        .on_item_press(&CALL_PAGE.into(), &"chat:19:alice@thread.v2".into())
        .await;
    harness.settle_desktop().await;
    assert_eq!(harness.desktop.activations(), []);
    assert_eq!(
        harness.bridge.requests(),
        [],
        "nothing asked of Teams either"
    );
    assert_eq!(harness.bridge.opened_conversations(), Vec::<String>::new());
}

#[tokio::test(start_paused = true)]
async fn the_window_of_the_default_profile_is_the_one_brought_to_the_front() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.desktop.set_windows(vec![
        Window::new(
            "other",
            "brave-ompifgpmddkgmclendfeacglnodjjndh-Profile 1",
            "Teams",
        ),
        teams_window(),
    ]);
    harness.settle_desktop().await;
    harness.module.on_key_press().await;
    harness.settle_desktop().await;
    assert_eq!(harness.desktop.activations(), [WindowId("teams".into())]);
}

#[tokio::test(start_paused = true)]
async fn a_window_that_cannot_be_raised_is_marked_with_a_warning() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.desktop.set_windows(vec![teams_window()]);
    harness.settle_desktop().await;
    harness.desktop.fail_actions(true);
    assert_eq!(harness.module.on_key_press().await, []);
    assert_eq!(
        harness.settle_desktop().await,
        [Command::FeedbackOn(ModuleSurface::HomeKey, Feedback::Alert)]
    );
}

fn conversation_with_id(name: &str, id: &str) -> UnreadConversation {
    UnreadConversation {
        id: Some(id.to_owned()),
        ..UnreadConversation::named(name)
    }
}

#[tokio::test(start_paused = true)]
async fn a_conversation_is_opened_in_teams_and_its_window_brought_to_the_front() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.desktop.set_windows(vec![teams_window()]);
    harness.settle_desktop().await;
    harness.bridge.report(BrowserReport::Unread { count: 1 });
    harness
        .report(BrowserReport::Conversations {
            unread: vec![conversation_with_id("Alice", "19:alice@thread.v2")],
        })
        .await;
    let page = harness.module.page(&UNREAD_PAGE.into()).unwrap();
    let item = page.items[0].id.clone();

    let commands = harness
        .module
        .on_item_press(&UNREAD_PAGE.into(), &item)
        .await;
    assert_eq!(commands, []);
    assert_eq!(harness.settle_desktop().await, []);
    assert_eq!(
        harness.bridge.opened_conversations(),
        ["19:alice@thread.v2"]
    );
    assert_eq!(harness.desktop.activations(), [WindowId("teams".into())]);
}

#[tokio::test(start_paused = true)]
async fn a_conversation_the_extension_did_not_report_is_not_asked_for() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.desktop.set_windows(vec![teams_window()]);
    harness.settle_desktop().await;
    harness
        .report(BrowserReport::Conversations {
            unread: vec![conversation_with_id("Alice", "19:alice@thread.v2")],
        })
        .await;
    // The list changed between the drawing and the press.
    let commands = harness
        .module
        .on_item_press(&UNREAD_PAGE.into(), &"chat:19:gone@thread.v2".into())
        .await;
    assert_eq!(commands, Command::alert());
    assert_eq!(harness.bridge.opened_conversations(), Vec::<String>::new());
    assert_eq!(harness.desktop.activations(), []);
}

#[tokio::test(start_paused = true)]
async fn without_an_extension_a_conversation_is_not_opened() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness
        .report(BrowserReport::Conversations {
            unread: vec![conversation_with_id("Alice", "19:alice@thread.v2")],
        })
        .await;
    harness.bridge.set_connected(false);
    harness.next_event().await;
    let commands = harness
        .module
        .on_item_press(&UNREAD_PAGE.into(), &"chat:19:alice@thread.v2".into())
        .await;
    assert_eq!(commands, Command::alert());
}

#[tokio::test(start_paused = true)]
async fn the_conversations_of_a_call_page_open_too() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.desktop.set_windows(vec![teams_window()]);
    harness.settle_desktop().await;
    harness
        .report(BrowserReport::Conversations {
            unread: vec![conversation_with_id("Alice", "19:alice@thread.v2")],
        })
        .await;
    harness.bridge.report(BrowserReport::Unread { count: 1 });
    harness.next_event().await;
    harness.in_call().await;
    let page = harness.call_page();
    let item = page.items.last().unwrap().id.clone();
    harness.module.on_item_press(&CALL_PAGE.into(), &item).await;
    harness.settle_desktop().await;
    assert_eq!(
        harness.bridge.opened_conversations(),
        ["19:alice@thread.v2"]
    );
}

#[tokio::test(start_paused = true)]
async fn return_to_the_call_asks_teams_to_enlarge_it_and_brings_its_window_to_the_front() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.desktop.set_windows(vec![teams_window()]);
    harness.settle_desktop().await;
    harness.in_call().await;
    let page = harness.call_page();
    assert_eq!(
        page.items[3].key.label.as_deref(),
        Some("Revenir à l'appel")
    );
    let commands = harness
        .module
        .on_item_press(&CALL_PAGE.into(), &page.items[3].id)
        .await;
    assert_eq!(commands, []);
    assert_eq!(harness.settle_desktop().await, []);
    assert_eq!(harness.desktop.activations(), [WindowId("teams".into())]);
    assert_eq!(harness.bridge.requests(), [CallAction::ReturnToCall]);
}

#[tokio::test(start_paused = true)]
async fn return_to_the_call_without_extension_still_brings_the_window_and_warns() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.desktop.set_windows(vec![teams_window()]);
    harness.settle_desktop().await;
    harness.in_call().await;
    let item = harness.call_page().items[3].id.clone();
    harness.bridge.set_connected(false);
    harness.next_event().await;
    let commands = harness.module.on_item_press(&CALL_PAGE.into(), &item).await;
    assert_eq!(commands, Command::alert());
    harness.settle_desktop().await;
    assert_eq!(harness.desktop.activations(), [WindowId("teams".into())]);
}

#[tokio::test]
async fn the_unread_page_is_gone_once_nothing_is_unread() {
    let mut harness = Harness::new(named_unread_bridge());
    assert!(harness.module.page(&UNREAD_PAGE.into()).is_some());

    // The last conversation is read: the page no longer exists, so that the
    // host leaves it for the page below.
    harness.report(BrowserReport::Unread { count: 0 }).await;
    harness
        .report(BrowserReport::Conversations { unread: Vec::new() })
        .await;
    assert!(harness.module.page(&UNREAD_PAGE.into()).is_none());

    // Unread conversations whose names are unknown keep it.
    harness.report(BrowserReport::Unread { count: 2 }).await;
    assert!(harness.module.page(&UNREAD_PAGE.into()).is_some());
}

#[tokio::test]
async fn unread_conversations_without_a_name_are_counted() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.report(BrowserReport::Unread { count: 4 }).await;
    harness
        .report(BrowserReport::Conversations {
            unread: vec![UnreadConversation::named("Camille")],
        })
        .await;
    let page = harness.module.page(&UNREAD_PAGE.into()).unwrap();
    assert_eq!(page.items[0].key.label.as_deref(), Some("+3"));
}

#[tokio::test]
async fn a_call_opens_its_page_once_and_keeps_the_device_lit() {
    let mut harness = Harness::new(SimulatedBridge::new());
    assert!(harness.module.page(&CALL_PAGE.into()).is_none());

    let commands = harness.in_call().await;
    assert!(open_page(&commands, CALL_PAGE));
    assert!(harness.call_page().keeps_device_lit);
    assert_eq!(harness.module.key().label.as_deref(), Some("En appel"));

    // Mute toggled: the page stays, nothing is opened again.
    let commands = harness
        .report(BrowserReport::Call {
            phase: CallPhase::InCall,
            muted: Some(true),
            camera_on: Some(false),
            caller: None,
            elapsed_seconds: None,
        })
        .await;
    assert!(!open_page(&commands, CALL_PAGE));
}

#[tokio::test]
async fn only_muting_and_hanging_up_are_declared_usable_when_locked() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.in_call().await;
    let declared: Vec<_> = harness
        .call_page()
        .items
        .iter()
        .map(|item| (item.id.0.clone(), item.key.usable_when_locked))
        .collect();
    assert_eq!(
        declared,
        [
            ("mute".to_owned(), true),
            ("camera".to_owned(), false),
            ("hangup".to_owned(), true),
            ("return".to_owned(), false)
        ]
    );
}

#[tokio::test]
async fn the_call_page_follows_the_microphone() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.in_call().await;
    assert_eq!(
        harness.call_page().items[0].key.label.as_deref(),
        Some("Micro actif")
    );
    harness
        .report(BrowserReport::Call {
            phase: CallPhase::InCall,
            muted: Some(true),
            camera_on: None,
            caller: None,
            elapsed_seconds: None,
        })
        .await;
    let mute = &harness.call_page().items[0].key;
    assert_eq!(mute.label.as_deref(), Some("Micro coupé"));
    assert_eq!(mute.image.as_ref(), Some(&*icons::MICROPHONE_MUTED));
    assert_ne!(mute.image.as_ref(), Some(&*icons::MICROPHONE));
}

#[tokio::test]
async fn the_actions_of_a_call_have_icons_and_no_attention_state() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness
        .report(BrowserReport::Call {
            phase: CallPhase::InCall,
            muted: Some(true),
            camera_on: Some(false),
            caller: None,
            elapsed_seconds: None,
        })
        .await;
    let page = harness.call_page();
    let actions = &page.items[..3];
    let images: Vec<_> = actions.iter().map(|item| item.key.image.clone()).collect();
    assert_eq!(
        images,
        [
            Some(icons::MICROPHONE_MUTED.clone()),
            Some(icons::CAMERA_OFF.clone()),
            Some(icons::HANG_UP.clone())
        ]
    );
    assert!(
        actions
            .iter()
            .all(|item| item.key.state == KeyState::Normal)
    );

    harness
        .report(BrowserReport::Call {
            phase: CallPhase::InCall,
            muted: Some(false),
            camera_on: Some(true),
            caller: None,
            elapsed_seconds: None,
        })
        .await;
    let page = harness.call_page();
    assert_eq!(page.items[0].key.image.as_ref(), Some(&*icons::MICROPHONE));
    assert_eq!(page.items[1].key.image.as_ref(), Some(&*icons::CAMERA));
}

#[tokio::test]
async fn the_unread_conversations_follow_the_actions_of_a_call_and_hide_when_locked() {
    let mut harness = Harness::new(named_unread_bridge());
    harness.in_call().await;
    let page = harness.call_page();
    // The four actions are pinned on the first row, then the conversations.
    assert_eq!(page.pinned_items, 4);
    let after: Vec<_> = page.items[4..]
        .iter()
        .map(|item| (item.key.label.clone().unwrap(), item.hidden_when_locked))
        .collect();
    assert_eq!(
        after,
        [
            ("Équipe produit".to_owned(), true),
            ("Camille".to_owned(), true)
        ]
    );
    assert!(page.items[..4].iter().all(|item| !item.hidden_when_locked));
}

#[tokio::test]
async fn a_ringing_call_shows_no_conversations() {
    let mut harness = Harness::new(named_unread_bridge());
    harness
        .report(BrowserReport::Call {
            phase: CallPhase::Ringing,
            muted: None,
            camera_on: None,
            caller: Some("Camille".into()),
            elapsed_seconds: None,
        })
        .await;
    let page = harness.call_page();
    assert_eq!(page.items.len(), 3);
    assert_eq!(page.pinned_items, 3);
}

#[tokio::test]
async fn a_conversation_shows_its_avatar_or_a_generic_icon() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.bridge.report(BrowserReport::Unread { count: 2 });
    let avatar = ImageData::new(b"avatar".as_slice());
    harness
        .report(BrowserReport::Conversations {
            unread: vec![
                UnreadConversation {
                    name: "Alice".into(),
                    icon: Some(avatar.clone()),
                    id: None,
                },
                UnreadConversation::named("Équipe"),
            ],
        })
        .await;
    let page = harness.module.page(&UNREAD_PAGE.into()).unwrap();
    let shown: Vec<_> = page
        .items
        .iter()
        .map(|item| (item.key.label.clone().unwrap(), item.key.image.clone()))
        .collect();
    assert_eq!(
        shown,
        [
            ("Alice".to_owned(), Some(avatar)),
            ("Équipe".to_owned(), Some(icons::CONVERSATION.clone()))
        ]
    );
}

#[tokio::test]
async fn the_home_key_is_usable_when_locked_only_during_a_call() {
    let mut harness = Harness::new(named_unread_bridge());
    assert!(!harness.module.key().usable_when_locked, "unread names");
    harness.in_call().await;
    assert!(harness.module.key().usable_when_locked);
    harness
        .report(BrowserReport::Call {
            phase: CallPhase::Idle,
            muted: None,
            camera_on: None,
            caller: None,
            elapsed_seconds: None,
        })
        .await;
    assert!(!harness.module.key().usable_when_locked);
}

#[tokio::test]
async fn items_of_the_call_page_ask_teams_for_the_matching_action() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.in_call().await;
    for (item, action) in [
        ("mute", CallAction::ToggleMute),
        ("camera", CallAction::ToggleCamera),
        ("hangup", CallAction::HangUp),
    ] {
        let commands = harness
            .module
            .on_item_press(&CALL_PAGE.into(), &item.into())
            .await;
        assert_eq!(commands, [], "{item}: the icon shows it worked");
        assert_eq!(harness.bridge.requests().last(), Some(&action));
    }
    assert_eq!(
        harness
            .module
            .on_item_press(&CALL_PAGE.into(), &"unknown".into())
            .await,
        []
    );
    assert_eq!(harness.bridge.requests().len(), 3);
}

#[tokio::test]
async fn an_action_without_an_extension_alerts() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.in_call().await;
    harness.bridge.set_connected(false);
    harness.next_event().await;
    // The call is gone with the extension: no page, and a late press alerts.
    assert!(harness.module.page(&CALL_PAGE.into()).is_none());
    assert_eq!(
        harness
            .module
            .on_item_press(&CALL_PAGE.into(), &"hangup".into())
            .await,
        Command::alert()
    );
}

#[tokio::test]
async fn a_ringing_call_offers_to_answer_or_decline() {
    let mut harness = Harness::new(SimulatedBridge::new());
    let commands = harness
        .report(BrowserReport::Call {
            phase: CallPhase::Ringing,
            muted: None,
            camera_on: None,
            caller: Some("Camille".into()),
            elapsed_seconds: None,
        })
        .await;
    assert!(open_page(&commands, CALL_PAGE));
    let page = harness.call_page();
    // The caller is on a key: page titles are logged.
    assert_eq!(page.title, "Appel");
    assert!(!page.keeps_device_lit);
    let ids: Vec<_> = page.items.iter().map(|item| item.id.0.as_str()).collect();
    assert_eq!(ids, ["caller", "accept", "decline"]);
    assert_eq!(page.items[0].key.label.as_deref(), Some("Camille"));
    assert!(page.items.iter().all(|item| !item.key.usable_when_locked));
    assert_eq!(harness.module.key().state, KeyState::Attention);
}

#[tokio::test]
async fn meetings_do_not_disturb_the_key() {
    let mut harness = Harness::new(SimulatedBridge::new());
    let commands = harness
        .report(BrowserReport::Meetings {
            items: vec![Meeting {
                subject: "Point".into(),
                starts_at: 1,
                ends_at: 2,
            }],
        })
        .await;
    assert_eq!(commands, []);
    assert_eq!(harness.module.key().label.as_deref(), Some("Teams"));
}

#[tokio::test]
async fn an_event_lost_in_a_full_queue_costs_nothing_but_the_page_opening() {
    let mut harness = Harness::new(SimulatedBridge::new());
    // The engine's queue was full: the events of this burst never arrived.
    harness.bridge.report(BrowserReport::Unread { count: 7 });
    harness.bridge.report(BrowserReport::Call {
        phase: CallPhase::InCall,
        muted: Some(true),
        camera_on: None,
        caller: None,
        elapsed_seconds: None,
    });
    while harness.events.try_recv().is_ok() {}

    // What is shown still follows the bridge.
    assert_eq!(harness.module.key().label.as_deref(), Some("En appel"));
    assert!(harness.module.page(&CALL_PAGE.into()).is_some());

    // And the module keeps listening: the next change opens the call page.
    harness.bridge.report(BrowserReport::Unread { count: 8 });
    let commands = harness.next_event().await;
    assert!(open_page(&commands, CALL_PAGE));
}

fn notification(commands: &[Command]) -> Option<&Notification> {
    commands.iter().find_map(|command| match command {
        Command::Notify(notification) => Some(notification),
        _ => None,
    })
}

fn meeting(subject: &str, starts_at: i64) -> Meeting {
    Meeting {
        subject: subject.into(),
        starts_at,
        ends_at: starts_at + 1800,
    }
}

/// A clock that starts at `origin` and follows the (simulated) tokio time.
fn clock_from(origin: i64) -> WallClock {
    let start = tokio::time::Instant::now();
    WallClock::from_fn(move || origin + i64::try_from(start.elapsed().as_secs()).unwrap())
}

#[tokio::test(start_paused = true)]
async fn a_meeting_is_announced_on_the_strip_when_it_comes_within_the_lead_time() {
    let mut harness = Harness::with(
        SimulatedBridge::new(),
        clock_from(10_000),
        &json!({ "reminderMinutes": 5 }),
    );
    // Starts in ten minutes: nothing yet; the report itself is only a change.
    harness.bridge.report(BrowserReport::Meetings {
        items: vec![meeting("Point d'équipe", 10_600)],
    });
    harness.next_event().await;
    // Five minutes later the reminder comes by itself.
    let commands = harness.next_event_within(MEETING_WAIT).await;
    let notification = notification(&commands).expect("a reminder");
    assert_eq!(notification.message, "Réunion dans 5 min : Point d'équipe");
    assert!(notification.timeout.is_some());
    assert!(commands.contains(&Command::haptic("meeting-soon")));
}

#[tokio::test(start_paused = true)]
async fn no_reminder_when_the_lead_time_is_zero() {
    let mut harness = Harness::with(
        SimulatedBridge::new(),
        clock_from(10_000),
        &json!({ "reminderMinutes": 0 }),
    );
    harness.bridge.report(BrowserReport::Meetings {
        items: vec![meeting("Point", 10_100)],
    });
    harness.next_event().await;
    // The desktop may report meanwhile; Teams itself has nothing to say.
    let nothing = tokio::time::timeout(Duration::from_secs(600), async {
        loop {
            let event = harness.events.recv().await;
            if event.is_some_and(|event| !event.is::<AppEvent>()) {
                break;
            }
        }
    })
    .await;
    assert!(nothing.is_err());
}

#[tokio::test(start_paused = true)]
async fn the_lead_time_can_change_while_the_module_runs() {
    let mut harness = Harness::with(
        SimulatedBridge::new(),
        clock_from(10_000),
        &json!({ "reminderMinutes": 0 }),
    );
    harness.bridge.report(BrowserReport::Meetings {
        items: vec![meeting("Point", 10_100)],
    });
    harness.next_event().await;
    harness
        .module
        .update_settings(&json!({ "reminderMinutes": 10 }))
        .unwrap();
    let commands = harness.next_event().await;
    assert!(notification(&commands).is_some());
}

#[test]
fn a_lead_time_out_of_range_is_refused() {
    let TestContext { ctx, .. } = TestContext::new(TEAMS_KIND);
    let factory = TeamsFactory::new(Arc::new(SimulatedBridge::new()), hub());
    assert!(
        factory
            .create(ctx, &json!({ "reminderMinutes": 61 }))
            .is_err()
    );
}

#[tokio::test]
async fn an_incoming_call_is_announced_and_its_notification_answers_it() {
    let mut harness = Harness::new(SimulatedBridge::new());
    let commands = harness
        .report(BrowserReport::Call {
            phase: CallPhase::Ringing,
            muted: None,
            camera_on: None,
            caller: Some("Alice".into()),
            elapsed_seconds: None,
        })
        .await;
    let announced = notification(&commands).expect("a notification");
    assert_eq!(announced.message, "Appel : Alice");
    assert!(announced.context_dial.hint.press.is_some());
    // Answering is not something to do on a locked computer.
    assert!(!announced.context_dial.usable_when_locked);
    assert!(open_page(&commands, CALL_PAGE));

    let answer = harness
        .module
        .on_notification_press(NotificationId(1))
        .await;
    assert_eq!(answer, []);
    assert_eq!(harness.bridge.requests(), [CallAction::Accept]);
}

#[tokio::test]
async fn a_call_that_stopped_ringing_is_not_answered() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.in_call().await;
    let answer = harness
        .module
        .on_notification_press(NotificationId(1))
        .await;
    assert_eq!(answer, []);
    assert_eq!(harness.bridge.requests(), []);
}

#[tokio::test]
async fn a_ringing_call_is_announced_once() {
    let mut harness = Harness::new(SimulatedBridge::new());
    let ringing = |caller: &str| BrowserReport::Call {
        phase: CallPhase::Ringing,
        muted: None,
        camera_on: None,
        caller: Some(caller.into()),
        elapsed_seconds: None,
    };
    assert!(notification(&harness.report(ringing("Alice")).await).is_some());
    assert!(notification(&harness.report(ringing("Alice B.")).await).is_none());
}

/// A harness whose bridge already holds what the rail says (reported
/// directly: nothing waits for the module to be told).
fn harness_with_badges(activity: u32, chats: u32, named: &[&str]) -> Harness {
    let harness = Harness::new(SimulatedBridge::new());
    harness.bridge.report(BrowserReport::Unread {
        count: activity + chats,
    });
    harness.bridge.report(BrowserReport::Conversations {
        unread: named
            .iter()
            .map(|name| UnreadConversation::named(*name))
            .collect(),
    });
    harness
        .bridge
        .report(BrowserReport::Badges { activity, chats });
    harness
}

fn unread_labels(harness: &Harness) -> Vec<String> {
    harness
        .module
        .page(&UNREAD_PAGE.into())
        .map(|page| {
            page.items
                .iter()
                .map(|item| item.key.label.clone().unwrap())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn the_activity_and_the_missing_conversations_come_before_the_conversations() {
    // The badge says 3 conversations, 2 are named: "+1".
    let harness = harness_with_badges(2, 3, &["Adam", "Benjamin"]);
    assert_eq!(unread_labels(&harness), ["2", "+1", "Adam", "Benjamin"]);
    let page = harness.module.page(&UNREAD_PAGE.into()).unwrap();
    assert_eq!(page.items[0].key.image.as_ref(), Some(&*icons::ACTIVITY));
    assert_eq!(page.items[0].key.state, KeyState::Attention);
    assert_eq!(
        page.items[1].key.image.as_ref(),
        Some(&*icons::CONVERSATION)
    );
    assert!(page.items[..2].iter().all(|item| item.hidden_when_locked));
    // The home key still counts everything.
    assert_eq!(harness.module.key().label.as_deref(), Some("5 non lus"));
}

#[tokio::test]
async fn the_conversations_key_goes_when_every_conversation_is_shown() {
    let harness = harness_with_badges(0, 2, &["Adam", "Benjamin"]);
    assert_eq!(unread_labels(&harness), ["Adam", "Benjamin"]);
}

#[tokio::test]
async fn activity_alone_keeps_the_unread_page_and_the_home_key_opens_it() {
    let mut harness = harness_with_badges(1, 0, &[]);
    assert_eq!(unread_labels(&harness), ["1"]);
    assert!(open_page(&harness.module.on_key_press().await, UNREAD_PAGE));
}

#[tokio::test]
async fn without_the_badges_the_title_count_gives_the_missing_conversations() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.report(BrowserReport::Unread { count: 3 }).await;
    harness
        .report(BrowserReport::Conversations {
            unread: vec![UnreadConversation::named("Adam")],
        })
        .await;
    assert_eq!(unread_labels(&harness), ["+2", "Adam"]);
}

#[tokio::test(start_paused = true)]
async fn the_activity_key_shows_the_panel_and_brings_teams_to_the_front() {
    let mut harness = harness_with_badges(2, 0, &[]);
    harness.desktop.set_windows(vec![teams_window()]);
    harness.settle_desktop().await;
    let item = ItemId(ACTIVITY_ITEM.into());
    let commands = harness
        .module
        .on_item_press(&UNREAD_PAGE.into(), &item)
        .await;
    assert_eq!(commands, []);
    assert_eq!(harness.bridge.opened_panels(), [Panel::Activity]);
    harness.settle_desktop().await;
    assert_eq!(harness.desktop.activations(), [WindowId("teams".into())]);
}

#[tokio::test(start_paused = true)]
async fn the_conversations_key_shows_the_panel_without_leaving_the_device() {
    let mut harness = harness_with_badges(0, 3, &["Adam"]);
    harness.desktop.set_windows(vec![teams_window()]);
    harness.settle_desktop().await;
    let item = ItemId(CONVERSATIONS_ITEM.into());
    let commands = harness
        .module
        .on_item_press(&UNREAD_PAGE.into(), &item)
        .await;
    assert_eq!(commands, []);
    assert_eq!(harness.bridge.opened_panels(), [Panel::Chat]);
    harness.settle_desktop().await;
    assert_eq!(harness.desktop.activations(), []);
    assert_eq!(harness.desktop.launches(), []);
}

#[tokio::test]
async fn the_panel_keys_do_nothing_when_locked_and_alert_without_extension() {
    let mut harness = harness_with_badges(1, 2, &[]);
    harness.locked.store(true, Ordering::SeqCst);
    for id in [ACTIVITY_ITEM, CONVERSATIONS_ITEM] {
        let commands = harness
            .module
            .on_item_press(&UNREAD_PAGE.into(), &ItemId(id.into()))
            .await;
        assert_eq!(commands, []);
    }
    assert_eq!(harness.bridge.opened_panels(), []);

    harness.locked.store(false, Ordering::SeqCst);
    harness.bridge.set_connected(false);
    harness.next_event().await;
    let commands = harness
        .module
        .on_item_press(&UNREAD_PAGE.into(), &ItemId(CONVERSATIONS_ITEM.into()))
        .await;
    assert_eq!(commands, Command::alert());
}

#[tokio::test(start_paused = true)]
async fn a_closed_window_is_told_even_with_the_extension_connected_and_pressing_starts_teams() {
    let mut harness = harness_with_badges(0, 1, &["Adam"]);
    // Windows not followed yet: nothing is claimed.
    assert_eq!(harness.module.key().label.as_deref(), Some("1 non lus"));

    harness.desktop.set_windows(vec![teams_window()]);
    harness.settle_desktop().await;
    assert_eq!(harness.module.key().label.as_deref(), Some("1 non lus"));

    harness.desktop.set_windows(Vec::new());
    harness.settle_desktop().await;
    let key = harness.module.key();
    assert_eq!(key.label.as_deref(), Some("Teams fermé"));
    assert_eq!(key.state, KeyState::Attention);

    // The unread page is not opened over a window that is not there.
    assert_eq!(harness.module.on_key_press().await, []);
    harness.settle_desktop().await;
    assert_eq!(harness.desktop.launches(), [launch()]);

    harness.desktop.set_windows(vec![teams_window()]);
    harness.settle_desktop().await;
    assert_eq!(harness.module.key().label.as_deref(), Some("1 non lus"));
}

#[tokio::test(start_paused = true)]
async fn a_call_shows_even_when_the_windows_are_not_listed() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.desktop.set_windows(Vec::new());
    harness.settle_desktop().await;
    harness.in_call().await;
    assert_eq!(harness.module.key().label.as_deref(), Some("En appel"));
}

#[tokio::test]
async fn the_conversations_key_shows_a_negative_gap_when_the_counter_is_lower() {
    // Three named conversations, the badge says one.
    let harness = harness_with_badges(0, 1, &["Adam", "Benjamin", "Camille"]);
    assert_eq!(
        unread_labels(&harness),
        ["-2", "Adam", "Benjamin", "Camille"]
    );
}

#[tokio::test(start_paused = true)]
async fn a_negative_gap_key_shows_the_conversation_panel_all_the_same() {
    let mut harness = harness_with_badges(0, 1, &["Adam", "Benjamin"]);
    harness.desktop.set_windows(vec![teams_window()]);
    harness.settle_desktop().await;
    let item = ItemId(CONVERSATIONS_ITEM.into());
    assert_eq!(
        harness
            .module
            .on_item_press(&UNREAD_PAGE.into(), &item)
            .await,
        []
    );
    assert_eq!(harness.bridge.opened_panels(), [Panel::Chat]);
    harness.settle_desktop().await;
    assert_eq!(harness.desktop.activations(), []);
}

#[tokio::test]
async fn the_radial_actions_ask_teams_for_the_matching_call_action() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.in_call().await;
    for (radial, action) in [
        ("mute", CallAction::ToggleMute),
        ("camera", CallAction::ToggleCamera),
        ("hang-up", CallAction::HangUp),
    ] {
        assert_eq!(
            harness.module.on_radial_action(radial).await,
            [],
            "{radial}"
        );
        assert_eq!(harness.bridge.requests().last(), Some(&action));
    }
    assert_eq!(harness.module.on_radial_action("unknown").await, []);
    assert_eq!(
        harness.bridge.requests().len(),
        3,
        "an unknown one asks nothing"
    );
}

#[tokio::test]
async fn a_radial_action_without_an_extension_alerts() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.in_call().await;
    harness.bridge.set_connected(false);
    harness.next_event().await;
    assert_eq!(
        harness.module.on_radial_action("mute").await,
        Command::alert()
    );
}

#[test]
fn the_module_declares_the_events_it_raises() {
    let ids: Vec<String> = TeamsFactory::new(Arc::new(SimulatedBridge::new()), hub())
        .haptic_events()
        .into_iter()
        .map(|event| event.id)
        .collect();
    assert_eq!(ids, ["incoming-call", "meeting-soon", "new-message"]);
}

#[tokio::test]
async fn a_call_that_starts_ringing_raises_the_haptic_event_once() {
    let mut harness = Harness::new(SimulatedBridge::new());
    let ringing = BrowserReport::Call {
        phase: CallPhase::Ringing,
        muted: None,
        camera_on: None,
        caller: Some("Camille".into()),
        elapsed_seconds: None,
    };
    let commands = harness.report(ringing).await;
    assert!(commands.contains(&Command::haptic("incoming-call")));
    // The same call, still ringing, reported with a change: not rung again.
    let again = harness
        .report(BrowserReport::Call {
            phase: CallPhase::Ringing,
            muted: None,
            camera_on: None,
            caller: Some("Camille D.".into()),
            elapsed_seconds: None,
        })
        .await;
    assert!(!again.contains(&Command::haptic("incoming-call")));
}

#[tokio::test]
async fn the_unread_count_growing_raises_the_haptic_event_and_shrinking_does_not() {
    let mut harness = Harness::new(SimulatedBridge::new());
    let first = harness.report(BrowserReport::Unread { count: 2 }).await;
    assert!(first.contains(&Command::haptic("new-message")));
    let grown = harness.report(BrowserReport::Unread { count: 3 }).await;
    assert!(grown.contains(&Command::haptic("new-message")));
    let read = harness.report(BrowserReport::Unread { count: 1 }).await;
    assert!(!read.contains(&Command::haptic("new-message")));
    let none = harness.report(BrowserReport::Unread { count: 0 }).await;
    assert!(!none.contains(&Command::haptic("new-message")));
}

fn call_report(elapsed_seconds: Option<u64>) -> BrowserReport {
    BrowserReport::Call {
        phase: CallPhase::InCall,
        muted: Some(false),
        camera_on: Some(false),
        caller: None,
        elapsed_seconds,
    }
}

fn hang_up_caption(harness: &Harness) -> Option<String> {
    let page = harness.call_page();
    let hang_up = page
        .items
        .iter()
        .find(|item| item.id.0 == "hangup")
        .expect("a hang up key");
    hang_up.key.caption.clone()
}

#[tokio::test(start_paused = true)]
async fn the_hang_up_key_shows_the_duration_teams_shows_and_counts_on() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.report(call_report(Some(65))).await;
    assert_eq!(hang_up_caption(&harness).as_deref(), Some("1:05"));
    tokio::time::advance(Duration::from_secs(5)).await;
    assert_eq!(hang_up_caption(&harness).as_deref(), Some("1:10"));
    // Teams shows the duration again: its value wins over the count.
    harness.report(call_report(Some(3600))).await;
    assert_eq!(hang_up_caption(&harness).as_deref(), Some("1:00:00"));
}

#[tokio::test(start_paused = true)]
async fn the_instance_is_woken_when_a_new_second_shows() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.report(call_report(Some(0))).await;
    // The first sight of the duration wakes the instance once; that wake-up
    // must not stand for the one of the next second.
    tokio::time::advance(Duration::from_millis(300)).await;
    while harness.events.try_recv().is_ok() {}
    tokio::time::advance(Duration::from_secs(2)).await;
    // Without the wake-up the page would keep the second it was drawn with.
    let event = tokio::time::timeout(Duration::from_secs(2), harness.events.recv())
        .await
        .expect("an event")
        .expect("the instance is alive");
    assert!(event.is::<ElapsedChanged>());
    assert_eq!(hang_up_caption(&harness).as_deref(), Some("0:02"));
}

#[tokio::test(start_paused = true)]
async fn a_call_without_a_shown_duration_is_timed_from_when_it_was_seen() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.report(call_report(None)).await;
    assert_eq!(hang_up_caption(&harness).as_deref(), Some("0:00"));
    tokio::time::advance(Duration::from_secs(12)).await;
    assert_eq!(hang_up_caption(&harness).as_deref(), Some("0:12"));
    // A later report without a duration does not restart the count (and is
    // no change worth an event: only the clock wakes the instance).
    harness.bridge.report(call_report(None));
    assert_eq!(hang_up_caption(&harness).as_deref(), Some("0:12"));
}

#[tokio::test(start_paused = true)]
async fn only_a_call_in_progress_has_a_duration() {
    let mut state = TeamsState::default();
    state.apply(call_report(Some(30)));
    assert!(state.call.elapsed.is_some());
    state.apply(BrowserReport::Call {
        phase: CallPhase::Ringing,
        muted: None,
        camera_on: None,
        caller: Some("Camille".into()),
        elapsed_seconds: Some(30),
    });
    assert!(state.call.elapsed.is_none());
    state.apply(call_report(None));
    assert_eq!(state.call.elapsed.map(|clock| clock.seconds()), Some(0));
}

#[tokio::test(start_paused = true)]
async fn a_report_that_agrees_with_the_count_is_not_a_change() {
    let mut state = TeamsState::default();
    assert!(state.apply(call_report(Some(30))));
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(!state.apply(call_report(Some(32))));
    assert!(state.apply(call_report(Some(90))));
}

#[tokio::test(start_paused = true)]
async fn the_duration_ends_with_the_call_and_a_new_call_starts_again() {
    let mut state = TeamsState::default();
    state.apply(call_report(None));
    tokio::time::advance(Duration::from_secs(40)).await;
    state.apply(BrowserReport::Call {
        phase: CallPhase::Idle,
        muted: None,
        camera_on: None,
        caller: None,
        elapsed_seconds: None,
    });
    assert!(state.call.elapsed.is_none());
    state.apply(call_report(None));
    assert_eq!(state.call.elapsed.map(|clock| clock.seconds()), Some(0));
}
