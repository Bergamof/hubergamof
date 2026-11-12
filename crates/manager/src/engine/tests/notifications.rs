//! Notifications over the touch strip, and the label of a held key.

use std::time::Duration;

use hubergamof_module_api::{Command, ContextDial, Notification, NotificationId, Rgb};
use hubergamof_protocol::DialAssignment;
use image::Rgba;

use super::support::*;
use crate::device::InputEvent;
use crate::device::TouchPoint;
use crate::device::virtual_deck::virtual_plus_info;

const BACKGROUND: Rgb = Rgb(200, 30, 30);
const BACKGROUND_PIXEL: Rgba<u8> = Rgba([200, 30, 30, 255]);
const NOTICE_PIXEL: Rgba<u8> = Rgba([0x2B, 0x6C, 0xB0, 255]);
/// Longer than [`crate::engine::device_engine::HOLD_THRESHOLD`].
const HOLD: Duration = Duration::from_millis(600);

/// Fragile on dial 0, navigation on dial 1, a list on the home page.
async fn plus_fixture() -> Fixture {
    let fixture = start(virtual_plus_info("PLUS")).await;
    fixture
        .configure(
            vec![list(0), fragile("none")],
            &["list"],
            &[Some(FRAGILE_KIND), Some(NAVIGATION_DIAL), None, None],
        )
        .await;
    fixture
}

impl Fixture {
    fn send_commands(&self, commands: Vec<Command>) {
        let events = self.probe.fragile_events.lock().unwrap().clone().unwrap();
        assert!(events.send(commands));
    }

    fn notify(&self, notification: Notification) {
        self.send_commands(vec![Command::Notify(notification)]);
    }

    fn unread_messages(&self) -> Vec<String> {
        let unread = self.probe.unread.lock().unwrap().clone().unwrap();
        unread.list().into_iter().map(|item| item.message).collect()
    }

    fn unread_ids(&self) -> Vec<NotificationId> {
        let unread = self.probe.unread.lock().unwrap().clone().unwrap();
        unread.list().into_iter().map(|item| item.id).collect()
    }

    fn strip_pixel(&self, x: u32, y: u32) -> Rgba<u8> {
        let screen = self.deck.inner.screen();
        *screen.strip.unwrap().get_pixel(x, y)
    }

    /// Waits until the strip shows (or stops showing) the notification
    /// background.
    async fn wait_for_notification(&self, shown: bool) {
        let mut live = self.engine.live();
        tokio::time::timeout(WAIT, async {
            while (self.strip_pixel(799, 0) == BACKGROUND_PIXEL) != shown {
                // Redraws not changing the live state do not wake us: poll.
                tokio::time::sleep(Duration::from_millis(10)).await;
                live.mark_unchanged();
            }
        })
        .await
        .unwrap();
    }

    fn inject(&self, event: InputEvent) {
        self.deck.inner.inject(event).unwrap();
    }

    fn tap_strip(&self) {
        self.inject(InputEvent::TouchTap {
            point: TouchPoint { x: 400, y: 50 },
            long: false,
        });
    }
}

fn permanent(message: &str) -> Notification {
    Notification::new(message, BACKGROUND)
}

#[tokio::test]
async fn a_notification_covers_the_strip_until_it_is_touched() {
    let fixture = plus_fixture().await;
    fixture.notify(permanent("Pause !"));
    fixture.wait_for_notification(true).await;
    assert_eq!(fixture.unread_messages(), ["Pause !"]);

    fixture.tap_strip();
    fixture.wait_for_notification(false).await;
    // Touched: read, and the tap, on the measures, only did that.
    assert_eq!(fixture.unread_messages(), Vec::<String>::new());
    assert!(fixture.config().strip.show_history);
}

#[tokio::test]
async fn a_notification_vanishes_after_its_timeout_and_stays_unread() {
    let fixture = plus_fixture().await;
    fixture.notify(permanent("Pause !").with_timeout(Duration::from_millis(300)));
    fixture.wait_for_notification(true).await;
    fixture.wait_for_notification(false).await;
    assert_eq!(fixture.unread_messages(), ["Pause !"]);
}

#[tokio::test]
async fn under_a_notification_only_the_navigation_dial_press_acts() {
    let fixture = plus_fixture().await;
    fixture.notify(permanent("Pause !"));
    fixture.wait_for_notification(true).await;

    // The dial of a module does nothing...
    fixture.inject(InputEvent::DialRotate { index: 0, ticks: 1 });
    fixture.inject(InputEvent::DialDown(0));
    // ...nor does turning the navigation dial.
    fixture.inject(InputEvent::DialRotate { index: 1, ticks: 1 });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fixture.strip_pixel(799, 0), BACKGROUND_PIXEL);
    assert_eq!(fixture.dial_mark(0), None);

    // Its press removes the notification, which stays unread.
    fixture.inject(InputEvent::DialDown(1));
    fixture.wait_for_notification(false).await;
    assert_eq!(fixture.unread_messages(), ["Pause !"]);
    // The module was never told about the dial.
    assert_eq!(fixture.hint_rotation(0).as_deref(), Some("fragile 0"));
}

#[tokio::test(start_paused = true)]
async fn the_contextual_dial_does_nothing_under_a_notification() {
    let fixture = plus_fixture().await;
    let mut config = fixture.config();
    config.dials[2] = DialAssignment::Contextual;
    fixture.engine.apply_config(config).await.unwrap();
    *fixture.probe.context_dial.lock().unwrap() = ContextDial::none()
        .rotating("Choisir")
        .pressing("Cocher")
        .long_pressing("Tout");
    fixture.engine.press_key(0).await.unwrap();
    fixture.notify(permanent("Pause !"));
    fixture.wait_for_notification(true).await;

    fixture.inject(InputEvent::DialRotate { index: 2, ticks: 1 });
    fixture.inject(InputEvent::DialDown(2));
    tokio::time::sleep(HOLD).await;
    fixture.inject(InputEvent::DialUp(2));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fixture.strip_pixel(799, 0), BACKGROUND_PIXEL);
    assert_eq!(
        *fixture.probe.context_calls.lock().unwrap(),
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn an_unread_notification_is_shown_again_without_timeout() {
    let fixture = plus_fixture().await;
    fixture.notify(permanent("Pause !").with_timeout(Duration::from_millis(200)));
    fixture.wait_for_notification(true).await;
    fixture.wait_for_notification(false).await;

    let id = fixture.unread_ids()[0];
    fixture.send_commands(vec![Command::ShowNotification(id)]);
    fixture.wait_for_notification(true).await;
    // Still there long after the original timeout.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(fixture.strip_pixel(799, 0), BACKGROUND_PIXEL);

    fixture.tap_strip();
    fixture.wait_for_notification(false).await;
    assert_eq!(fixture.unread_messages(), Vec::<String>::new());
}

#[tokio::test]
async fn notifications_wait_for_their_turn() {
    let fixture = plus_fixture().await;
    fixture.notify(permanent("un"));
    fixture.notify(Notification::new("deux", Rgb(10, 120, 40)));
    fixture.wait_for_notification(true).await;
    assert_eq!(fixture.unread_messages(), ["un", "deux"]);

    fixture.tap_strip();
    tokio::time::timeout(WAIT, async {
        while fixture.strip_pixel(799, 0) != Rgba([10, 120, 40, 255]) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.unread_messages(), ["deux"]);
}

#[tokio::test]
async fn a_device_without_a_strip_only_counts_notifications() {
    let fixture = start(dials_without_strip_info()).await;
    fixture
        .configure(
            vec![list(0), fragile("none")],
            &["list"],
            &[Some(FRAGILE_KIND), None],
        )
        .await;
    fixture.notify(permanent("Pause !"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fixture.unread_messages(), ["Pause !"]);
    // Nothing is shown, so nothing is blocked.
    fixture.inject(InputEvent::KeyDown(0));
    fixture.inject(InputEvent::KeyUp(0));
    fixture
        .wait_for(|fixture| fixture.page().id == "list/list")
        .await;
}

#[tokio::test]
async fn a_key_held_shows_its_label_and_runs_no_action() {
    let fixture = plus_fixture().await;
    assert_eq!(fixture.labels()[0].as_deref(), Some("list 0"));
    fixture.inject(InputEvent::KeyDown(0));
    tokio::time::sleep(HOLD).await;
    // White text over black.
    let screen = fixture.deck.inner.screen().strip.unwrap();
    assert_eq!(*screen.get_pixel(0, 0), Rgba([0, 0, 0, 255]));
    assert!(screen.pixels().any(|pixel| pixel.0[0] > 200));

    fixture.inject(InputEvent::KeyUp(0));
    tokio::time::sleep(Duration::from_millis(200)).await;
    // The strip shows again, the key did nothing.
    assert_eq!(fixture.hint_rotation(0).as_deref(), Some("fragile 0"));
    assert_eq!(fixture.labels()[0].as_deref(), Some("list 0"));
    assert_eq!(fixture.page().id, "home");
}

#[tokio::test]
async fn a_key_released_quickly_runs_its_action_on_release() {
    let fixture = plus_fixture().await;
    fixture.inject(InputEvent::KeyDown(0));
    tokio::time::sleep(Duration::from_millis(50)).await;
    // Nothing yet: the action waits for the release.
    assert_eq!(fixture.page().id, "home");
    fixture.inject(InputEvent::KeyUp(0));
    fixture
        .wait_for(|fixture| fixture.page().id == "list/list")
        .await;
}

#[tokio::test]
async fn the_notifications_module_counts_lists_and_shows_again() {
    let fixture = start(virtual_plus_info("PLUS")).await;
    fixture
        .configure(
            vec![
                list(0),
                fragile("none"),
                instance("notifications", "notifications", serde_json::json!({})),
            ],
            &["notifications"],
            &[Some(FRAGILE_KIND), Some(NAVIGATION_DIAL), None, None],
        )
        .await;
    let bell = fixture.live().keys[0].image.clone().unwrap().png().unwrap();

    fixture.notify(permanent("Pause !"));
    fixture.wait_for_notification(true).await;
    // The bell now carries a badge.
    fixture
        .wait_for(|fixture| {
            fixture.live().keys[0]
                .image
                .as_ref()
                .is_some_and(|image| image.png().unwrap() != bell)
        })
        .await;

    // Dismissed with the navigation dial: unread, listed.
    fixture.inject(InputEvent::DialDown(1));
    fixture.wait_for_notification(false).await;
    fixture.inject(InputEvent::KeyDown(0));
    fixture.inject(InputEvent::KeyUp(0));
    fixture
        .wait_for(|fixture| fixture.page().id == "notifications/unread")
        .await;
    assert_eq!(fixture.labels()[0].as_deref(), Some("Pause !"));

    // Choosing it shows it again.
    fixture.inject(InputEvent::KeyDown(0));
    fixture.inject(InputEvent::KeyUp(0));
    fixture.wait_for_notification(true).await;
}

#[tokio::test]
async fn a_key_without_a_label_acts_on_release_however_long_it_is_held() {
    let fixture = plus_fixture().await;
    fixture
        .probe
        .key_label
        .lock()
        .unwrap()
        .replace(String::new());
    fixture
        .probe
        .redraw
        .lock()
        .unwrap()
        .clone()
        .unwrap()
        .request_redraw();
    tokio::time::sleep(Duration::from_millis(100)).await;
    fixture.inject(InputEvent::KeyDown(0));
    tokio::time::sleep(HOLD).await;
    fixture.inject(InputEvent::KeyUp(0));
    fixture
        .wait_for(|fixture| fixture.page().id == "list/list")
        .await;
}

#[tokio::test]
async fn releasing_another_key_keeps_the_hold() {
    let fixture = plus_fixture().await;
    fixture.inject(InputEvent::KeyDown(0));
    fixture.inject(InputEvent::KeyUp(1));
    fixture.inject(InputEvent::KeyUp(0));
    fixture
        .wait_for(|fixture| fixture.page().id == "list/list")
        .await;
}

#[tokio::test]
async fn a_release_without_a_press_does_nothing() {
    let fixture = plus_fixture().await;
    fixture.inject(InputEvent::KeyUp(0));
    // An empty key: nothing to hold.
    fixture.inject(InputEvent::KeyDown(5));
    fixture.inject(InputEvent::KeyUp(5));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fixture.page().id, "home");
}

#[tokio::test]
async fn simulated_presses_are_immediate_even_under_a_notification() {
    let fixture = plus_fixture().await;
    fixture.notify(permanent("Pause !"));
    fixture.wait_for_notification(true).await;
    fixture.engine.press_key(0).await.unwrap();
    fixture
        .wait_for(|fixture| fixture.page().id == "list/list")
        .await;
}

#[tokio::test]
async fn a_tap_under_a_held_label_does_not_read_the_notification() {
    let fixture = plus_fixture().await;
    fixture.notify(permanent("Pause !"));
    fixture.wait_for_notification(true).await;
    fixture.inject(InputEvent::KeyDown(0));
    tokio::time::sleep(HOLD).await;
    fixture.tap_strip();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fixture.unread_messages(), ["Pause !"]);
}

#[tokio::test]
async fn a_configuration_brought_back_to_the_rules_is_explained_on_the_strip() {
    let info = virtual_plus_info("PLUS");
    let fixture = start_with_stored(info, |info, store| {
        let mut config = store.load(info).unwrap().config;
        // Pomodoro cannot be placed on a key.
        config.home.push("pomodoro".into());
        store.save(info, config).unwrap();
    })
    .await;
    assert!(!fixture.config().home.iter().any(|id| id.0 == "pomodoro"));
    let mut live = fixture.engine.live();
    tokio::time::timeout(WAIT, async {
        while fixture.strip_pixel(799, 0) != NOTICE_PIXEL {
            tokio::time::sleep(Duration::from_millis(10)).await;
            live.mark_unchanged();
        }
    })
    .await
    .unwrap();
}

/// The contextual dial on dial 2, the one that reacts under a notification.
async fn contextual_fixture() -> Fixture {
    let fixture = plus_fixture().await;
    let mut config = fixture.config();
    config.dials[2] = DialAssignment::Contextual;
    fixture.engine.apply_config(config).await.unwrap();
    fixture
}

fn with_action(message: &str, press: &str) -> Notification {
    permanent(message).with_context_dial(ContextDial::none().pressing(press))
}

impl Fixture {
    async fn press_dial(&self, index: u8) {
        self.inject(InputEvent::DialDown(index));
        self.inject(InputEvent::DialUp(index));
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    fn notification_presses(&self) -> Vec<NotificationId> {
        self.probe.notification_presses.lock().unwrap().clone()
    }
}

#[tokio::test(start_paused = true)]
async fn the_contextual_dial_press_runs_the_notification_action_and_reads_it() {
    let fixture = contextual_fixture().await;
    fixture.notify(with_action("Pause !", "Démarrer la pause"));
    fixture.wait_for_notification(true).await;
    let id = fixture.unread_ids()[0];

    fixture.press_dial(2).await;

    assert_eq!(fixture.notification_presses(), [id]);
    assert_eq!(fixture.unread_messages(), Vec::<String>::new());
    fixture.wait_for_notification(false).await;
}

#[tokio::test(start_paused = true)]
async fn a_notification_without_action_ignores_the_contextual_dial() {
    let fixture = contextual_fixture().await;
    fixture.notify(permanent("Pause !"));
    fixture.wait_for_notification(true).await;

    fixture.press_dial(2).await;

    assert_eq!(fixture.notification_presses(), []);
    assert_eq!(fixture.unread_messages(), ["Pause !"]);
    assert_eq!(fixture.strip_pixel(799, 0), BACKGROUND_PIXEL);
}

#[tokio::test(start_paused = true)]
async fn only_the_press_of_the_contextual_dial_acts_on_the_notification() {
    let fixture = contextual_fixture().await;
    fixture.notify(with_action("Pause !", "Démarrer la pause"));
    fixture.wait_for_notification(true).await;

    fixture.inject(InputEvent::DialRotate { index: 2, ticks: 1 });
    fixture.press_dial(3).await;

    assert_eq!(fixture.notification_presses(), []);
    assert_eq!(fixture.unread_messages(), ["Pause !"]);
}

#[tokio::test(start_paused = true)]
async fn a_failed_action_leaves_the_notification_unread() {
    let fixture = contextual_fixture().await;
    fixture.notify(with_action("Pause !", "Démarrer la pause"));
    fixture.wait_for_notification(true).await;
    let mut config = fixture.config();
    config.modules[1] = fragile("press");
    fixture.engine.apply_config(config).await.unwrap();

    fixture.press_dial(2).await;

    assert_eq!(fixture.notification_presses(), []);
    assert_eq!(fixture.unread_messages(), ["Pause !"]);
}

#[tokio::test(start_paused = true)]
async fn an_unread_notification_shown_again_keeps_its_action() {
    let fixture = contextual_fixture().await;
    fixture.notify(
        with_action("Pause !", "Démarrer la pause").with_timeout(Duration::from_millis(200)),
    );
    fixture.wait_for_notification(true).await;
    fixture.wait_for_notification(false).await;
    let id = fixture.unread_ids()[0];
    fixture.send_commands(vec![Command::ShowNotification(id)]);
    fixture.wait_for_notification(true).await;

    fixture.press_dial(2).await;

    assert_eq!(fixture.notification_presses(), [id]);
    assert_eq!(fixture.unread_messages(), Vec::<String>::new());
}
