use std::time::Duration;

use hubergamof_module_api::testing::TestContext;
use hubergamof_module_api::{Command, Feedback, KeyState, TaskScope};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use super::wire::{BrowserReport, Meeting};
use super::*;

const PATIENCE: Duration = Duration::from_secs(5);

struct Harness {
    bridge: Arc<SimulatedBridge>,
    module: Box<dyn Module>,
    events: UnboundedReceiver<ModuleEvent>,
    _scope: TaskScope,
}

impl Harness {
    fn new(bridge: SimulatedBridge) -> Self {
        let bridge = Arc::new(bridge);
        let TestContext { ctx, events, scope } = TestContext::new(TEAMS_KIND);
        let module = TeamsFactory::new(Arc::clone(&bridge) as _)
            .create(ctx, &json!({}))
            .unwrap();
        Self {
            bridge,
            module,
            events,
            _scope: scope,
        }
    }

    /// The commands the module answers to what the extension reported.
    async fn report(&mut self, report: BrowserReport) -> Vec<Command> {
        self.bridge.report(report);
        self.next_event().await
    }

    async fn next_event(&mut self) -> Vec<Command> {
        let event = tokio::time::timeout(PATIENCE, self.events.recv())
            .await
            .expect("an event")
            .expect("the instance is alive");
        self.module.on_event(event).await
    }

    async fn in_call(&mut self) -> Vec<Command> {
        self.report(BrowserReport::Call {
            phase: CallPhase::InCall,
            muted: Some(false),
            camera_on: Some(false),
            caller: None,
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
    let capabilities = TeamsFactory::new(Arc::new(SimulatedBridge::new()))
        .descriptor()
        .capabilities;
    assert!(capabilities.key && capabilities.lock_safe_pages);
    assert!(!capabilities.multiple_instances && !capabilities.usable_when_locked);
}

#[tokio::test]
async fn the_key_counts_the_unread_conversations() {
    let harness = Harness::new(SimulatedBridge::demo());
    let key = harness.module.key();
    assert_eq!(key.label.as_deref(), Some("2 non lus"));
    assert_eq!(key.state, KeyState::Attention);

    let quiet = Harness::new(SimulatedBridge::new());
    assert_eq!(quiet.module.key().label.as_deref(), Some("Teams"));
    assert_eq!(quiet.module.key().state, KeyState::Normal);
}

#[tokio::test]
async fn pressing_the_key_opens_the_unread_page_or_alerts_when_there_is_nothing() {
    let mut harness = Harness::new(SimulatedBridge::demo());
    assert!(open_page(&harness.module.on_key_press().await, UNREAD_PAGE));
    let page = harness.module.page(&UNREAD_PAGE.into()).unwrap();
    let labels: Vec<_> = page
        .items
        .iter()
        .map(|item| item.key.label.clone().unwrap())
        .collect();
    assert_eq!(labels, ["Équipe produit", "Camille"]);

    let mut quiet = Harness::new(SimulatedBridge::new());
    assert_eq!(quiet.module.on_key_press().await, Command::alert());
}

#[tokio::test]
async fn unread_conversations_without_a_name_are_counted() {
    let mut harness = Harness::new(SimulatedBridge::new());
    harness.report(BrowserReport::Unread { count: 4 }).await;
    harness
        .report(BrowserReport::Conversations {
            unread: vec!["Camille".into()],
        })
        .await;
    let page = harness.module.page(&UNREAD_PAGE.into()).unwrap();
    assert_eq!(page.items.last().unwrap().key.label.as_deref(), Some("+3"));
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
        .map(|item| (item.id.0.clone(), item.usable_when_locked))
        .collect();
    assert_eq!(
        declared,
        [
            ("mute".to_owned(), true),
            ("camera".to_owned(), false),
            ("hangup".to_owned(), true)
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
        })
        .await;
    let mute = &harness.call_page().items[0].key;
    assert_eq!(mute.label.as_deref(), Some("Micro coupé"));
    assert_eq!(mute.state, KeyState::Attention);
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
        assert_eq!(commands, [Command::Feedback(Feedback::Ok)], "{item}");
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
    assert!(page.items.iter().all(|item| !item.usable_when_locked));
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
