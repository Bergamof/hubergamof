//! The device while the computer is locked.

use std::sync::atomic::Ordering;
use std::time::Duration;

use hubergamof_protocol::LOCK_SLEEP_RANGE;

use super::support::*;
use crate::device::virtual_deck::{DEMO_SERIAL, virtual_plus_info};
use crate::device::{InputEvent, TouchPoint};
use crate::engine::HOME_PAGE_ID;

const SLEEP_SECONDS: u16 = 2;
const SLEEP: Duration = Duration::from_secs(SLEEP_SECONDS as u64);
/// Time for the engine to handle what it was sent.
const SETTLE: Duration = Duration::from_millis(50);

/// A device with a list on its first key, going dark `SLEEP_SECONDS` after a lock.
async fn locking_fixture(sleep_seconds: u16) -> Fixture {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    let mut config = fixture.with(
        vec![list(4)],
        &["list"],
        &[Some(NAVIGATION_DIAL), None, None, None],
    );
    config.strip.lock_sleep_seconds = sleep_seconds;
    fixture.engine.apply_config(config).await.unwrap();
    fixture
}

impl Fixture {
    fn brightness(&self) -> u8 {
        self.deck.inner.screen().brightness
    }

    /// Presses and releases key 0 on the device, as a finger does.
    async fn tap_first_key(&self) {
        self.deck.inner.inject(InputEvent::KeyDown(0)).unwrap();
        self.deck.inner.inject(InputEvent::KeyUp(0)).unwrap();
        tokio::time::sleep(SETTLE).await;
    }

    async fn lock(&self, locked: bool) {
        self.probe.screen_lock.set(locked);
        tokio::time::sleep(SETTLE).await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_locked_device_goes_dark_after_the_delay() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    let lit = fixture.config().brightness;
    assert_eq!(fixture.brightness(), lit);

    fixture.lock(true).await;
    tokio::time::sleep(SLEEP.saturating_sub(SETTLE * 2)).await;
    assert_eq!(fixture.brightness(), lit, "still lit before the delay");

    tokio::time::sleep(SETTLE * 4).await;
    assert_eq!(fixture.brightness(), 0, "dark after the delay");
}

#[tokio::test(start_paused = true)]
async fn unlocking_lights_the_device_at_once() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    let lit = fixture.config().brightness;
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP + SETTLE).await;
    assert_eq!(fixture.brightness(), 0);

    fixture.lock(false).await;

    assert_eq!(fixture.brightness(), lit);
    // And it stays lit.
    tokio::time::sleep(SLEEP * 2).await;
    assert_eq!(fixture.brightness(), lit);
}

#[tokio::test(start_paused = true)]
async fn an_input_wakes_the_dark_device_without_an_action_then_it_goes_dark_again() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    let lit = fixture.config().brightness;
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP + SETTLE).await;
    assert_eq!(fixture.brightness(), 0);

    fixture.tap_first_key().await;

    assert_eq!(fixture.brightness(), lit, "woken up");
    assert_eq!(fixture.page().id, HOME_PAGE_ID, "no action was run");

    tokio::time::sleep(SLEEP + SETTLE).await;
    assert_eq!(fixture.brightness(), 0, "dark again after the delay");
}

#[tokio::test(start_paused = true)]
async fn a_dial_wakes_the_dark_device_without_going_back() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, "list/list");
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP + SETTLE).await;

    fixture.deck.inner.inject(InputEvent::DialDown(0)).unwrap();
    fixture.deck.inner.inject(InputEvent::DialUp(0)).unwrap();
    tokio::time::sleep(SETTLE).await;

    assert_eq!(fixture.brightness(), fixture.config().brightness);
    assert_eq!(fixture.page().id, "list/list", "the press did not go back");
}

#[tokio::test(start_paused = true)]
async fn an_input_keeps_the_lit_device_lit_for_another_delay() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    let lit = fixture.config().brightness;
    fixture.lock(true).await;

    tokio::time::sleep(SLEEP.saturating_sub(Duration::from_millis(500))).await;
    fixture.tap_first_key().await;
    tokio::time::sleep(SLEEP.saturating_sub(Duration::from_millis(500))).await;

    assert_eq!(fixture.brightness(), lit, "the delay restarted");
}

#[tokio::test(start_paused = true)]
async fn no_action_runs_while_locked_even_from_the_web_ui() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, "list/list");
    fixture.lock(true).await;

    // The navigation dial would go back; the key would open the list again.
    fixture.engine.press_dial(0).await.unwrap();
    fixture.deck.inner.inject(InputEvent::DialDown(0)).unwrap();
    fixture.deck.inner.inject(InputEvent::DialUp(0)).unwrap();
    fixture
        .deck
        .inner
        .inject(InputEvent::DialRotate { index: 0, ticks: 1 })
        .unwrap();
    tokio::time::sleep(SETTLE).await;
    assert_eq!(fixture.page().id, "list/list");

    // Unlocked, the same press goes back.
    fixture.lock(false).await;
    fixture.engine.press_dial(0).await.unwrap();
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
}

#[tokio::test(start_paused = true)]
async fn tapping_the_strip_while_locked_changes_nothing() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    let before = fixture.config();
    fixture.lock(true).await;

    // The system measures panel toggles the history when the computer is unlocked.
    fixture
        .deck
        .inner
        .inject(InputEvent::TouchTap {
            point: TouchPoint { x: 400, y: 50 },
            long: false,
        })
        .unwrap();
    tokio::time::sleep(SETTLE).await;

    assert_eq!(fixture.config(), before);
}

#[tokio::test(start_paused = true)]
async fn a_device_started_on_a_locked_computer_ignores_inputs_then_goes_dark() {
    let fixture = start_locked(virtual_plus_info(DEMO_SERIAL)).await;
    let mut config = fixture.with(vec![list(4)], &["list"], &[None; 4]);
    config.strip.lock_sleep_seconds = SLEEP_SECONDS;
    fixture.engine.apply_config(config).await.unwrap();
    let lit = fixture.config().brightness;

    fixture.tap_first_key().await;
    assert_eq!(fixture.page().id, HOME_PAGE_ID);

    tokio::time::sleep(SLEEP + SETTLE).await;
    assert_eq!(fixture.brightness(), 0);

    fixture.lock(false).await;
    assert_eq!(fixture.brightness(), lit);
}

#[tokio::test(start_paused = true)]
async fn the_strip_is_written_again_when_the_device_wakes_up() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP + SETTLE).await;
    let _ = fixture.deck.writes();

    fixture.tap_first_key().await;
    let (_, strip_writes) = fixture.deck.writes();
    assert!(strip_writes > 0, "woken by an input");

    tokio::time::sleep(SLEEP + SETTLE).await;
    let _ = fixture.deck.writes();
    fixture.lock(false).await;
    let (_, strip_writes) = fixture.deck.writes();
    assert!(strip_writes > 0, "woken by the unlock");
}

#[tokio::test(start_paused = true)]
async fn nothing_is_drawn_while_the_device_is_dark() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP + SETTLE).await;
    let _ = fixture.deck.writes();

    // The clock moves and a redraw is asked: the strip stays untouched.
    tokio::time::sleep(Duration::from_secs(10)).await;

    assert_eq!(fixture.deck.writes(), (0, 0));
}

#[tokio::test(start_paused = true)]
async fn unlocking_before_the_delay_cancels_the_extinction() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    let lit = fixture.config().brightness;
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP / 2).await;
    fixture.lock(false).await;

    tokio::time::sleep(SLEEP * 2).await;

    assert_eq!(fixture.brightness(), lit);
}

#[tokio::test(start_paused = true)]
async fn a_new_delay_does_not_light_a_dark_device() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP + SETTLE).await;

    let mut config = fixture.config();
    config.strip.lock_sleep_seconds = 60;
    fixture.engine.apply_config(config).await.unwrap();

    assert_eq!(fixture.brightness(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_lock_source_that_stays_locked_keeps_the_device_dark() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP + SETTLE).await;
    tokio::time::sleep(Duration::from_secs(5)).await;

    assert_eq!(fixture.brightness(), 0);
    fixture.tap_first_key().await;
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
}

#[tokio::test(start_paused = true)]
async fn a_gesture_started_before_the_lock_does_not_end_in_an_action() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    fixture.deck.inner.inject(InputEvent::KeyDown(0)).unwrap();
    tokio::time::sleep(SETTLE).await;

    fixture.lock(true).await;
    fixture.deck.inner.inject(InputEvent::KeyUp(0)).unwrap();
    tokio::time::sleep(SETTLE).await;

    assert_eq!(fixture.page().id, HOME_PAGE_ID);
}

#[tokio::test(start_paused = true)]
async fn a_delay_of_zero_never_turns_the_device_off() {
    assert!(LOCK_SLEEP_RANGE.contains(&0));
    let fixture = locking_fixture(0).await;
    let lit = fixture.config().brightness;

    fixture.lock(true).await;
    tokio::time::sleep(Duration::from_secs(600)).await;

    assert_eq!(fixture.brightness(), lit);
    // Still no action.
    fixture.tap_first_key().await;
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
}

#[tokio::test(start_paused = true)]
async fn a_new_delay_applies_to_a_device_already_locked() {
    let fixture = locking_fixture(60).await;
    let lit = fixture.config().brightness;
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP).await;
    assert_eq!(fixture.brightness(), lit);

    let mut config = fixture.config();
    config.strip.lock_sleep_seconds = SLEEP_SECONDS;
    fixture.engine.apply_config(config).await.unwrap();
    tokio::time::sleep(SLEEP + SETTLE).await;
    assert_eq!(fixture.brightness(), 0);

    // Switching the feature off lights it again.
    let mut config = fixture.config();
    config.strip.lock_sleep_seconds = 0;
    fixture.engine.apply_config(config).await.unwrap();
    assert_eq!(fixture.brightness(), lit);
}

#[tokio::test(start_paused = true)]
async fn the_brightness_chosen_while_dark_applies_when_it_wakes_up() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP + SETTLE).await;

    let mut config = fixture.config();
    config.brightness = 30;
    fixture.engine.apply_config(config).await.unwrap();
    assert_eq!(fixture.brightness(), 0, "still dark");

    fixture.lock(false).await;
    assert_eq!(fixture.brightness(), 30);
}

#[tokio::test(start_paused = true)]
async fn a_device_that_refuses_to_go_dark_keeps_drawing_and_tries_again() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    let lit = fixture.config().brightness;
    fixture.deck.fail_brightness.store(true, Ordering::SeqCst);
    fixture.lock(true).await;

    tokio::time::sleep(SLEEP + SETTLE).await;
    assert_eq!(fixture.brightness(), lit, "still lit");

    fixture.deck.fail_brightness.store(false, Ordering::SeqCst);
    fixture.tap_first_key().await;
    tokio::time::sleep(SLEEP + SETTLE).await;
    assert_eq!(fixture.brightness(), 0, "dark at the next try");
}
