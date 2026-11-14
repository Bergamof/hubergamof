use std::sync::Mutex;

use hubergamof_module_api::TaskScope;
use hubergamof_module_api::testing::TestContext;
use serde_json::json;
use tokio::sync::mpsc;

use super::*;
use crate::notifications::NotifyError;
use crate::time::testing::SimulatedClock;

const MINUTE: Duration = Duration::from_secs(60);

/// Records notifications, or fails every one of them.
#[derive(Debug, Default)]
struct FakeNotifier {
    sent: Mutex<Vec<Notification>>,
    failing: bool,
}

impl FakeNotifier {
    fn sent(&self) -> Vec<Notification> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl Notifier for FakeNotifier {
    async fn notify(&self, notification: &Notification) -> Result<(), NotifyError> {
        self.sent.lock().unwrap().push(notification.clone());
        if self.failing {
            Err(NotifyError::Timeout)
        } else {
            Ok(())
        }
    }
}

/// A pomodoro driven like the host does: events of its tasks are handed to
/// `on_event` after every simulated second.
struct Harness {
    module: Box<dyn Module>,
    events: mpsc::UnboundedReceiver<ModuleEvent>,
    notifier: Arc<FakeNotifier>,
    clock: Arc<SimulatedClock>,
    scope: TaskScope,
    /// Commands returned by the events handled so far.
    commands: Vec<Command>,
}

fn factory(notifier: Arc<FakeNotifier>, clock: Arc<SimulatedClock>) -> PomodoroFactory {
    PomodoroFactory::new(notifier, clock)
}

fn simple_factory() -> PomodoroFactory {
    factory(Arc::default(), Arc::new(SimulatedClock::new()))
}

impl Harness {
    fn new(settings: &Value) -> Self {
        Self::with_notifier(settings, FakeNotifier::default())
    }

    fn with_notifier(settings: &Value, notifier: FakeNotifier) -> Self {
        let notifier = Arc::new(notifier);
        let clock = Arc::new(SimulatedClock::new());
        let TestContext { ctx, events, scope } = TestContext::new(POMODORO_KIND);
        let factory = factory(Arc::clone(&notifier), Arc::clone(&clock));
        Self {
            module: factory.create(ctx, settings).unwrap(),
            events,
            notifier,
            clock,
            scope,
            commands: Vec::new(),
        }
    }

    /// Lets the background tasks run, then handles the events they sent;
    /// returns how many there were.
    async fn handle_events(&mut self) -> usize {
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        let mut count = 0;
        while let Ok(event) = self.events.try_recv() {
            let commands = self.module.on_event(event).await;
            self.commands.extend(commands);
            count += 1;
        }
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        count
    }

    /// Advances the time second by second, handling events as they come;
    /// returns how many events were handled.
    async fn run_for(&mut self, duration: Duration) -> usize {
        let mut count = 0;
        let mut left = duration;
        while !left.is_zero() {
            let step = left.min(TICK_PERIOD);
            tokio::time::sleep(step).await;
            left -= step;
            count += self.handle_events().await;
        }
        count
    }

    fn key(&self) -> KeyContent {
        self.module.key()
    }

    fn widget(&self) -> PomodoroWidget {
        match self.module.strip_widget() {
            Some(StripWidget::Pomodoro(widget)) => widget,
            other => panic!("expected the pomodoro widget, got {other:?}"),
        }
    }

    /// Pomodoro and cycle length of the work step shown by the widget.
    fn steps(&self) -> (u32, u32) {
        let label = self.widget().label;
        let counts = label
            .strip_prefix("Focus ")
            .and_then(|rest| rest.split(' ').next())
            .and_then(|counts| counts.split_once('/'))
            .unwrap_or_else(|| panic!("a work step shows its position, got {label:?}"));
        (counts.0.parse().unwrap(), counts.1.parse().unwrap())
    }

    fn control() -> DialFunctionId {
        CONTROL_FUNCTION.into()
    }

    async fn press_dial(&mut self) -> Vec<Command> {
        self.module.on_dial_press(&Self::control()).await
    }

    async fn rotate_dial(&mut self, ticks: i32) -> Vec<Command> {
        self.module.on_dial_rotate(&Self::control(), ticks).await
    }

    fn popup(&self) -> Option<StripPopup> {
        self.module.strip_popup()
    }

    /// The entries of the open menu, with a `›` before the selected one.
    fn menu_lines(&self) -> Vec<String> {
        let popup = self.popup().expect("the menu is open");
        popup
            .entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                if index == popup.selected() {
                    format!("› {}", entry.label)
                } else {
                    entry.label.clone()
                }
            })
            .collect()
    }
}

fn label(key: &KeyContent) -> &str {
    key.label.as_deref().unwrap_or_default()
}

#[test]
fn default_settings_are_the_classic_cycle() {
    let factory = simple_factory();
    assert_eq!(
        factory.default_settings(),
        json!({
            "workMinutes": 25,
            "shortBreakMinutes": 5,
            "longBreakMinutes": 15,
            "longBreakEvery": 4,
            "autoStart": false,
            "notificationEnabled": true,
            "notificationSeconds": 10
        })
    );
    // Pomodoro is driven by a dial: no key, one instance, and a timer that
    // stays usable on a locked computer.
    assert_eq!(
        factory.descriptor().capabilities,
        ModuleCapabilities {
            usable_when_locked: true,
            ..Default::default()
        }
    );
}

#[test]
fn settings_are_validated_against_their_bounds() {
    let factory = simple_factory();
    assert!(factory.validate_settings(&json!({})).is_ok());
    assert!(
        factory
            .validate_settings(&json!({ "workMinutes": 120, "longBreakEvery": 12 }))
            .is_ok()
    );
    for invalid in [
        json!({ "workMinutes": 121 }),
        json!({ "shortBreakMinutes": 0 }),
        json!({ "longBreakEvery": 13 }),
        json!({ "autoStart": 1 }),
        json!([]),
    ] {
        assert!(
            factory.validate_settings(&invalid).is_err(),
            "{invalid} accepted"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_new_pomodoro_shows_a_full_stopped_work_session() {
    let pomodoro = Harness::new(&json!({}));
    let key = pomodoro.key();
    assert_eq!(label(&key), "25:00 Travail");
    assert_eq!(key.state, KeyState::Normal);
    assert_ready(&pomodoro.widget(), "Focus 1/4 · prêt", "25:00");
}

#[tokio::test(start_paused = true)]
async fn pressing_the_key_starts_and_pauses_the_time() {
    let mut pomodoro = Harness::new(&json!({}));
    assert_eq!(pomodoro.module.on_key_press().await, []);
    assert_eq!(pomodoro.key().state, KeyState::Active);
    pomodoro.run_for(MINUTE + Duration::from_millis(500)).await;
    assert_eq!(label(&pomodoro.key()), "24:00 Travail");
    let widget = pomodoro.widget();
    assert_eq!(widget.state, PomodoroState::Working);
    assert_eq!(
        (widget.label.as_str(), widget.time.as_str()),
        ("Focus 1/4", "24:00")
    );
    // The progress matches the time shown (one minute spent), not the 60.5 s.
    assert_eq!(widget.progress(), 40);

    assert_eq!(pomodoro.module.on_key_press().await, []);
    assert_eq!(pomodoro.key().state, KeyState::Normal);
    assert_eq!(
        pomodoro.run_for(10 * MINUTE).await,
        0,
        "no tick while paused"
    );
    assert_eq!(label(&pomodoro.key()), "24:00 Travail");
    let paused = pomodoro.widget();
    assert_eq!(paused.state, PomodoroState::Paused);
    assert_eq!(paused.label, "Focus 1/4 · pause");
}

#[tokio::test(start_paused = true)]
async fn ticks_fall_on_whole_seconds_of_the_time_left() {
    let mut pomodoro = Harness::new(&json!({}));
    pomodoro.module.on_key_press().await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    pomodoro.module.on_key_press().await;
    pomodoro.module.on_key_press().await;
    // 24:59.6 left: the first tick comes 0.6 s later, at 24:59.
    assert_eq!(pomodoro.run_for(Duration::from_millis(500)).await, 0);
    assert_eq!(pomodoro.run_for(Duration::from_millis(200)).await, 1);
    assert_eq!(pomodoro.run_for(Duration::from_secs(3)).await, 3);
}

#[tokio::test(start_paused = true)]
async fn the_dial_press_opens_the_menu_without_touching_the_time() {
    let mut pomodoro = Harness::new(&json!({}));
    assert!(pomodoro.popup().is_none());
    assert_eq!(pomodoro.press_dial().await, []);
    assert_eq!(pomodoro.key().state, KeyState::Normal);
    let popup = pomodoro.popup().expect("the menu is open");
    assert_eq!(popup.function, Harness::control());
    assert_eq!(popup.selected(), 0);
}

#[tokio::test(start_paused = true)]
async fn the_first_menu_entry_follows_the_state_of_the_time() {
    let mut pomodoro = Harness::new(&json!({}));
    pomodoro.press_dial().await;
    assert_eq!(pomodoro.menu_lines()[0], "› Démarrer");
    assert_eq!(pomodoro.popup().unwrap().entries()[0].icon, PopupIcon::Play);
    assert_eq!(pomodoro.press_dial().await, []);
    assert_eq!(pomodoro.key().state, KeyState::Active);
    assert!(pomodoro.popup().is_none(), "menu closed");

    pomodoro.run_for(MINUTE).await;
    pomodoro.press_dial().await;
    assert_eq!(pomodoro.menu_lines()[0], "› Pause");
    assert_eq!(
        pomodoro.popup().unwrap().entries()[0].icon,
        PopupIcon::Pause
    );
    pomodoro.press_dial().await;
    assert_eq!(pomodoro.key().state, KeyState::Normal);

    pomodoro.press_dial().await;
    assert_eq!(
        pomodoro.menu_lines()[0],
        "› Reprendre",
        "some time has elapsed"
    );
}

#[tokio::test(start_paused = true)]
async fn the_hint_follows_the_menu() {
    let mut pomodoro = Harness::new(&json!({}));
    let hint = |pomodoro: &Harness| pomodoro.module.dial_hint(&Harness::control());
    assert_eq!(hint(&pomodoro), DialHint::none().pressing("Menu pomodoro"));
    pomodoro.press_dial().await;
    assert_eq!(
        hint(&pomodoro),
        DialHint::none().rotating("Choisir").pressing("Valider")
    );
    assert_eq!(pomodoro.module.dial_hint(&"other".into()), DialHint::none());
}

#[tokio::test(start_paused = true)]
async fn the_widget_takes_the_state_of_the_step() {
    let mut pomodoro = Harness::new(&json!({}));
    pomodoro.module.on_key_press().await;
    assert_eq!(pomodoro.widget().state, PomodoroState::Working);
    pomodoro.run_for(MINUTE).await;
    pomodoro.module.on_key_press().await;
    assert_eq!(pomodoro.widget().state, PomodoroState::Paused);
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(25 * MINUTE).await;
    pomodoro.module.on_key_press().await;
    let resting = pomodoro.widget();
    assert_eq!(resting.state, PomodoroState::Resting);
    assert_eq!(resting.label, "Pause");
}

#[tokio::test(start_paused = true)]
async fn a_finished_step_waits_for_a_press_and_notifies_once() {
    let mut pomodoro = Harness::new(&json!({ "workMinutes": 1 }));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(MINUTE).await;

    let key = pomodoro.key();
    assert_eq!(key.state, KeyState::Attention);
    assert_eq!(label(&key), "05:00 Pause");
    assert_ready(&pomodoro.widget(), "Pause · prêt", "05:00");
    assert_eq!(pomodoro.run_for(MINUTE).await, 0, "the ticker stopped");
    assert_eq!(
        pomodoro.notifier.sent(),
        [Notification {
            summary: "Pause !".into(),
            body: "Pomodoro 1/4 terminé. Appuyez pour lancer la pause de 5 min.".into(),
        }]
    );

    pomodoro.module.on_key_press().await;
    assert_eq!(pomodoro.key().state, KeyState::Active);
    pomodoro.run_for(5 * MINUTE).await;
    assert_eq!(label(&pomodoro.key()), "01:00 Travail");
    assert_eq!(pomodoro.steps(), (2, 4));
    assert_eq!(pomodoro.notifier.sent().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn auto_start_chains_the_steps() {
    let mut pomodoro = Harness::new(&json!({ "workMinutes": 1, "autoStart": true }));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(MINUTE + 30 * TICK_PERIOD).await;
    let key = pomodoro.key();
    assert_eq!(key.state, KeyState::Active);
    assert_eq!(label(&key), "04:30 Pause");
    assert_eq!(
        pomodoro.notifier.sent()[0].body,
        "Pomodoro 1/4 terminé. La pause de 5 min commence."
    );
}

#[tokio::test(start_paused = true)]
async fn a_failed_notification_does_not_stop_the_pomodoro() {
    let mut pomodoro = Harness::with_notifier(
        &json!({ "workMinutes": 1, "autoStart": true }),
        FakeNotifier {
            failing: true,
            ..FakeNotifier::default()
        },
    );
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(MINUTE + 5 * MINUTE).await;
    assert_eq!(pomodoro.notifier.sent().len(), 2);
    assert_eq!(pomodoro.steps(), (2, 4));
    assert_eq!(pomodoro.scope.reap_panics(), 0);
}

#[tokio::test(start_paused = true)]
async fn rotating_moves_the_selection_of_the_open_menu() {
    let mut pomodoro = Harness::new(&json!({}));
    assert_eq!(pomodoro.rotate_dial(3).await, []);
    assert!(
        pomodoro.popup().is_none(),
        "rotating does not open the menu"
    );
    pomodoro.press_dial().await;
    assert_eq!(
        pomodoro.menu_lines(),
        ["› Démarrer", "Passer à la pause", "Remise à zéro"]
    );
    pomodoro.rotate_dial(1).await;
    assert_eq!(pomodoro.menu_lines()[1], "› Passer à la pause");
    pomodoro.rotate_dial(1).await;
    assert_eq!(pomodoro.menu_lines()[2], "› Remise à zéro");
    pomodoro.rotate_dial(-2).await;
    assert_eq!(pomodoro.menu_lines()[0], "› Démarrer");
}

#[tokio::test(start_paused = true)]
async fn pressing_in_the_menu_skips_to_the_next_step() {
    let mut pomodoro = Harness::new(&json!({}));
    pomodoro.module.on_key_press().await;
    pomodoro.press_dial().await;
    pomodoro.rotate_dial(1).await;
    assert_eq!(pomodoro.press_dial().await, []);
    assert_eq!(label(&pomodoro.key()), "05:00 Pause");
    assert_eq!(pomodoro.key().state, KeyState::Active, "still running");
    pomodoro.press_dial().await;
    pomodoro.rotate_dial(1).await;
    assert_eq!(pomodoro.menu_lines()[1], "› Passer au travail");
    assert!(pomodoro.notifier.sent().is_empty(), "skipping is silent");
}

#[tokio::test(start_paused = true)]
async fn reset_from_the_menu_returns_to_the_first_pomodoro() {
    let mut pomodoro = Harness::new(&json!({}));
    pomodoro.module.on_key_press().await;
    for _ in 0..2 {
        pomodoro.press_dial().await;
        pomodoro.rotate_dial(1).await;
        pomodoro.press_dial().await;
    }
    assert_eq!(pomodoro.steps(), (2, 4));
    pomodoro.press_dial().await;
    pomodoro.rotate_dial(2).await;
    assert_eq!(pomodoro.press_dial().await, []);
    assert_ready(&pomodoro.widget(), "Focus 1/4 · prêt", "25:00");
    assert_eq!(pomodoro.key().state, KeyState::Normal);
    assert_eq!(pomodoro.run_for(MINUTE).await, 0, "the ticker stopped");
}

#[tokio::test(start_paused = true)]
async fn the_menu_closes_after_five_seconds_without_input() {
    let mut pomodoro = Harness::new(&json!({}));
    pomodoro.press_dial().await;
    pomodoro.run_for(Duration::from_secs(3)).await;
    pomodoro.rotate_dial(1).await;
    pomodoro
        .run_for(MENU_TIMEOUT.saturating_sub(TICK_PERIOD))
        .await;
    assert!(pomodoro.popup().is_some());
    pomodoro.run_for(TICK_PERIOD).await;
    assert!(pomodoro.popup().is_none());
    assert_eq!(pomodoro.run_for(MINUTE).await, 0, "the ticker stopped");
    assert_eq!(pomodoro.key().state, KeyState::Normal, "nothing was run");
}

#[tokio::test(start_paused = true)]
async fn the_key_press_toggles_the_time_even_with_the_menu_open() {
    let mut pomodoro = Harness::new(&json!({}));
    pomodoro.press_dial().await;
    pomodoro.module.on_key_press().await;
    assert_eq!(pomodoro.key().state, KeyState::Active);
    assert!(pomodoro.popup().is_some());
    assert_eq!(pomodoro.menu_lines()[0], "› Pause", "the menu follows");
}

#[tokio::test(start_paused = true)]
async fn new_settings_keep_the_current_step() {
    let mut pomodoro = Harness::new(&json!({}));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(10 * MINUTE).await;
    pomodoro
        .module
        .update_settings(&json!({ "workMinutes": 30, "longBreakEvery": 2 }))
        .unwrap();
    assert_eq!(label(&pomodoro.key()), "20:00 Travail");
    assert_eq!(pomodoro.steps(), (1, 2));
    assert_eq!(pomodoro.key().state, KeyState::Active);
    assert!(pomodoro.run_for(TICK_PERIOD).await > 0, "still ticking");

    assert!(
        pomodoro
            .module
            .update_settings(&json!({ "workMinutes": 0 }))
            .is_err()
    );
    assert_eq!(pomodoro.steps(), (1, 2));
}

#[tokio::test(start_paused = true)]
async fn ticks_stop_with_the_instance() {
    let mut pomodoro = Harness::new(&json!({}));
    pomodoro.module.on_key_press().await;
    let Harness {
        module,
        mut events,
        scope,
        ..
    } = pomodoro;
    drop(module);
    drop(scope);
    tokio::time::sleep(MINUTE).await;
    assert!(events.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn the_widget_changes_with_the_displayed_second_only() {
    let mut pomodoro = Harness::new(&json!({}));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(Duration::from_millis(1_100)).await;
    let first = pomodoro.widget();
    pomodoro.run_for(Duration::from_millis(800)).await;
    assert_eq!(pomodoro.widget(), first, "same second shown, same image");
    pomodoro.run_for(Duration::from_millis(200)).await;
    assert_ne!(pomodoro.widget().progress(), first.progress());
}

#[tokio::test(start_paused = true)]
async fn a_step_whose_deadline_passed_asleep_ends_on_waking_up() {
    let mut pomodoro = Harness::new(&json!({ "workMinutes": 25 }));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(10 * MINUTE).await;
    // An hour asleep: Tokio's timers did not see it pass.
    pomodoro.clock.sleep_machine(60 * MINUTE);
    assert_eq!(pomodoro.run_for(TICK_PERIOD).await, 1, "a single tick");
    let key = pomodoro.key();
    assert_eq!(label(&key), "05:00 Pause");
    assert_eq!(key.state, KeyState::Attention);
    assert_eq!(pomodoro.notifier.sent().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn time_asleep_counts_within_a_step() {
    let mut pomodoro = Harness::new(&json!({}));
    pomodoro.module.on_key_press().await;
    pomodoro.clock.sleep_machine(10 * MINUTE);
    pomodoro.run_for(TICK_PERIOD).await;
    assert_eq!(label(&pomodoro.key()), "14:59 Travail");
}

#[tokio::test(start_paused = true)]
async fn missed_ticks_are_not_replayed() {
    let mut pomodoro = Harness::new(&json!({}));
    pomodoro.module.on_key_press().await;
    assert_eq!(pomodoro.handle_events().await, 0, "ticker started");
    // The runtime stalls for ten seconds (busy machine): one tick only.
    tokio::time::advance(10 * TICK_PERIOD).await;
    assert_eq!(pomodoro.handle_events().await, 1);
    assert_eq!(label(&pomodoro.key()), "24:50 Travail");
}

#[tokio::test(start_paused = true)]
async fn ticks_realign_after_a_jump_within_a_second() {
    let mut pomodoro = Harness::new(&json!({}));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(Duration::from_secs(2)).await;
    // 0.3 s asleep: 24:57.7 left. The tick already due at 3 s still comes
    // (24:56.7 left), then the next ones fall on whole seconds again.
    pomodoro.clock.sleep_machine(Duration::from_millis(300));
    assert_eq!(pomodoro.run_for(Duration::from_millis(600)).await, 0);
    assert_eq!(pomodoro.run_for(Duration::from_millis(100)).await, 0);
    assert_eq!(pomodoro.run_for(Duration::from_millis(300)).await, 1);
    assert_eq!(label(&pomodoro.key()), "24:57 Travail");
    assert_eq!(pomodoro.run_for(Duration::from_millis(600)).await, 0);
    assert_eq!(pomodoro.run_for(Duration::from_millis(100)).await, 1);
    assert_eq!(label(&pomodoro.key()), "24:56 Travail");
}

#[tokio::test(start_paused = true)]
async fn shorter_lengths_below_the_time_spent_end_the_step_at_once() {
    let mut pomodoro = Harness::new(&json!({ "workMinutes": 10 }));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(5 * MINUTE).await;
    pomodoro
        .module
        .update_settings(&json!({ "workMinutes": 2 }))
        .unwrap();
    assert_eq!(pomodoro.handle_events().await, 1, "immediate tick");
    assert_eq!(label(&pomodoro.key()), "05:00 Pause");
    assert_eq!(pomodoro.notifier.sent().len(), 1);
    assert_eq!(pomodoro.run_for(MINUTE).await, 0, "no spinning ticker");
}

#[test]
fn time_left_is_rounded_up_to_the_second() {
    assert_eq!(format_remaining(Duration::ZERO), "00:00");
    assert_eq!(format_remaining(Duration::from_millis(1)), "00:01");
    assert_eq!(format_remaining(Duration::from_millis(59_500)), "01:00");
    assert_eq!(format_remaining(120 * MINUTE), "120:00");
}

#[test]
fn first_ticks_wait_for_the_next_whole_second() {
    assert_eq!(
        delay_to_whole_second(Duration::from_millis(2_300)),
        Duration::from_millis(300)
    );
    assert_eq!(delay_to_whole_second(Duration::from_secs(2)), TICK_PERIOD);
    assert_eq!(delay_to_whole_second(Duration::ZERO), Duration::ZERO);
}

#[test]
fn notifications_name_the_step_to_start() {
    let back_to_work = notification_for(
        &StepEnd {
            finished: Phase::ShortBreak,
            next: Phase::Work,
            pomodoro: 3,
            next_duration: 25 * MINUTE,
            started: false,
        },
        4,
    );
    assert_eq!(
        back_to_work,
        Notification {
            summary: "Au travail !".into(),
            body: "Pause terminée. Appuyez pour lancer le pomodoro 3/4.".into(),
        }
    );
    let long_break = notification_for(
        &StepEnd {
            finished: Phase::Work,
            next: Phase::LongBreak,
            pomodoro: 4,
            next_duration: 15 * MINUTE,
            started: true,
        },
        4,
    );
    assert_eq!(
        long_break,
        Notification {
            summary: "Pause longue !".into(),
            body: "Pomodoro 4/4 terminé. La pause longue de 15 min commence.".into(),
        }
    );
}

fn strip_notifications(pomodoro: &Harness) -> Vec<StripNotification> {
    pomodoro
        .commands
        .iter()
        .map(|command| match command {
            Command::Notify(notification) => notification.clone(),
            other => panic!("unexpected command {other:?}"),
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn the_end_of_a_step_notifies_the_strip_for_the_configured_time() {
    let mut pomodoro = Harness::new(&json!({ "workMinutes": 1, "notificationSeconds": 7 }));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(MINUTE).await;

    let notifications = strip_notifications(&pomodoro);
    assert_eq!(notifications.len(), 1);
    assert_eq!(
        notifications[0].message,
        "Pause ! Pomodoro 1/4 terminé. Appuyez pour lancer la pause de 5 min."
    );
    assert_eq!(notifications[0].background, BREAK_BACKGROUND);
    assert_eq!(notifications[0].timeout, Some(Duration::from_secs(7)));
}

#[tokio::test(start_paused = true)]
async fn zero_seconds_makes_the_strip_notification_permanent() {
    let mut pomodoro = Harness::new(&json!({ "workMinutes": 1, "notificationSeconds": 0 }));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(MINUTE).await;
    assert_eq!(strip_notifications(&pomodoro)[0].timeout, None);
}

#[tokio::test(start_paused = true)]
async fn the_strip_notification_can_be_disabled_but_not_the_desktop_one() {
    let mut pomodoro = Harness::new(&json!({ "workMinutes": 1, "notificationEnabled": false }));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(MINUTE).await;
    assert_eq!(pomodoro.commands, []);
    assert_eq!(pomodoro.notifier.sent().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn work_starting_is_announced_in_red_and_a_break_in_green() {
    let mut pomodoro = Harness::new(&json!({ "workMinutes": 1, "shortBreakMinutes": 1 }));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(MINUTE).await;
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(MINUTE).await;
    let backgrounds: Vec<_> = strip_notifications(&pomodoro)
        .iter()
        .map(|notification| notification.background)
        .collect();
    assert_eq!(backgrounds, [BREAK_BACKGROUND, WORK_BACKGROUND]);
}

#[tokio::test(start_paused = true)]
async fn new_settings_change_the_next_strip_notification() {
    let mut pomodoro = Harness::new(&json!({ "workMinutes": 1 }));
    pomodoro
        .module
        .update_settings(&json!({ "workMinutes": 1, "notificationSeconds": 3 }))
        .unwrap();
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(MINUTE).await;
    assert_eq!(
        strip_notifications(&pomodoro)[0].timeout,
        Some(Duration::from_secs(3))
    );
}

/// A step prepared and not started: nothing elapsed.
fn assert_ready(widget: &PomodoroWidget, label: &str, time: &str) {
    assert_eq!(widget.state, PomodoroState::Ready);
    assert_eq!((widget.label.as_str(), widget.time.as_str()), (label, time));
    assert_eq!(widget.progress(), 0);
}

fn press_text(notification: &StripNotification) -> Option<&str> {
    notification.context_dial.hint.press.as_deref()
}

#[tokio::test(start_paused = true)]
async fn the_notification_offers_to_start_the_next_step() {
    let mut pomodoro = Harness::new(&json!({ "workMinutes": 1, "shortBreakMinutes": 1 }));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(MINUTE).await;
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(MINUTE).await;
    let texts: Vec<_> = strip_notifications(&pomodoro)
        .iter()
        .map(|notification| press_text(notification).map(str::to_owned))
        .collect();
    assert_eq!(
        texts,
        [
            Some("Démarrer la pause".to_owned()),
            Some("Démarrer le travail".to_owned())
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn nothing_is_offered_when_the_next_step_starts_by_itself() {
    let mut pomodoro = Harness::new(&json!({ "workMinutes": 1, "autoStart": true }));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(MINUTE).await;
    let notifications = strip_notifications(&pomodoro);
    assert_eq!(notifications.len(), 1);
    assert_eq!(press_text(&notifications[0]), None);
}

#[tokio::test(start_paused = true)]
async fn pressing_the_notification_starts_the_awaited_step() {
    let mut pomodoro = Harness::new(&json!({ "workMinutes": 1 }));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(MINUTE).await;
    assert_eq!(pomodoro.module.key().state, KeyState::Attention);
    pomodoro
        .module
        .on_notification_press(NotificationId(0))
        .await;
    assert_eq!(pomodoro.module.key().state, KeyState::Active);
}

#[tokio::test(start_paused = true)]
async fn pressing_the_notification_after_a_manual_start_does_not_pause() {
    let mut pomodoro = Harness::new(&json!({ "workMinutes": 1 }));
    pomodoro.module.on_key_press().await;
    pomodoro.run_for(MINUTE).await;
    pomodoro.module.on_key_press().await;
    pomodoro
        .module
        .on_notification_press(NotificationId(0))
        .await;
    assert_eq!(pomodoro.module.key().state, KeyState::Active);
}
