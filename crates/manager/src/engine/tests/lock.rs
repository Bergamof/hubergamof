//! The device while the computer is locked.

use std::time::Duration;

use hubergamof_protocol::LOCK_SLEEP_RANGE;

use super::support::*;
use crate::device::InputEvent;
use crate::device::virtual_deck::{DEMO_SERIAL, virtual_plus_info};
use crate::engine::HOME_PAGE_ID;

const SLEEP_SECONDS: u16 = 2;
const SLEEP: Duration = Duration::from_secs(SLEEP_SECONDS as u64);
/// Time for the engine to handle what it was sent.
const SETTLE: Duration = Duration::from_millis(50);

/// A device with a list on its first key, going dark `SLEEP_SECONDS` after a lock.
async fn locking_fixture(sleep_seconds: u16) -> Fixture {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    let mut config = fixture.with(vec![list(4)], &["list"], &[None; 4]);
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
async fn a_dial_wakes_the_dark_device_too() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP + SETTLE).await;

    fixture.deck.inner.inject(InputEvent::DialDown(0)).unwrap();
    fixture.deck.inner.inject(InputEvent::DialUp(0)).unwrap();
    tokio::time::sleep(SETTLE).await;

    assert_eq!(fixture.brightness(), fixture.config().brightness);
    assert!(
        fixture.strip().volume.is_none_or(|volume| !volume.muted),
        "the dial press did not mute"
    );
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
    fixture.lock(true).await;

    fixture.tap_first_key().await;
    fixture.engine.press_key(0).await.unwrap();
    fixture.engine.press_dial(0).await.unwrap();

    assert_eq!(fixture.page().id, HOME_PAGE_ID);
    assert!(fixture.strip().volume.is_none_or(|volume| !volume.muted));

    // Unlocked, the same press works again.
    fixture.lock(false).await;
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, "list/list");
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
