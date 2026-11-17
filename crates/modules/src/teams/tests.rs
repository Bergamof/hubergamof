use std::time::Duration;

use hubergamof_module_api::testing::TestContext;
use hubergamof_module_api::{Command, Feedback, KeyState, Notification, NotificationId, TaskScope};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use super::wire::{BrowserReport, Meeting};
use super::*;

const PATIENCE: Duration = Duration::from_secs(5);

/// Simulated time to wait for a reminder.
const MEETING_WAIT: Duration = Duration::from_secs(600);

struct Harness {
    bridge: Arc<SimulatedBridge>,
    module: Box<dyn Module>,
    events: UnboundedReceiver<ModuleEvent>,
    _scope: TaskScope,
}

impl Harness {
    fn new(bridge: SimulatedBridge) -> Self {
        Self::with(bridge, WallClock::system(), &json!({}))
    }

    fn with(bridge: SimulatedBridge, clock: WallClock, settings: &Value) -> Self {
        let bridge = Arc::new(bridge);
        let TestContext { ctx, events, scope } = TestContext::new(TEAMS_KIND);
        let module = TeamsFactory::with_clock(Arc::clone(&bridge) as _, clock)
            .create(ctx, settings)
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
        self.next_event_within(PATIENCE).await
    }

    async fn next_event_within(&mut self, patience: Duration) -> Vec<Command> {
        let event = tokio::time::timeout(patience, self.events.recv())
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
    let nothing = tokio::time::timeout(Duration::from_secs(600), harness.events.recv()).await;
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
    let factory = TeamsFactory::new(Arc::new(SimulatedBridge::new()));
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
    assert!(matches!(
        answer.as_slice(),
        [Command::Feedback(Feedback::Ok)]
    ));
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
    };
    assert!(notification(&harness.report(ringing("Alice")).await).is_some());
    assert!(notification(&harness.report(ringing("Alice B.")).await).is_none());
}
