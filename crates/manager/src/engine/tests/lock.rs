//! The device while the computer is locked.

use std::sync::atomic::Ordering;
use std::time::Duration;

use hubergamof_module_api::{Command, ContextDial};
use hubergamof_protocol::{DialAssignment, LOCK_SLEEP_RANGE};

use serde_json::json;

use super::support::*;
use crate::device::virtual_deck::{DEMO_SERIAL, virtual_plus_info};
use crate::device::{InputEvent, TouchPoint};
use crate::engine::HOME_PAGE_ID;

/// The pomodoro dial of the default layout.
const POMODORO_DIAL: u8 = 1;
const VOLUME_DIAL: u8 = 0;
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
    config.lock_sleep_seconds = sleep_seconds;
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
async fn a_dial_wakes_the_dark_device_without_acting() {
    // The pomodoro dial of the default layout works on a locked computer.
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    let mut config = fixture.config();
    config.lock_sleep_seconds = SLEEP_SECONDS;
    fixture.engine.apply_config(config).await.unwrap();
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP + SETTLE).await;
    assert_eq!(fixture.brightness(), 0);

    fixture
        .deck
        .inner
        .inject(InputEvent::DialDown(POMODORO_DIAL))
        .unwrap();
    fixture
        .deck
        .inner
        .inject(InputEvent::DialUp(POMODORO_DIAL))
        .unwrap();
    tokio::time::sleep(SETTLE).await;

    assert_eq!(fixture.brightness(), fixture.config().brightness);
    assert!(
        fixture.strip().popup.is_none(),
        "the waking press opened nothing"
    );

    // Pressed again, lit, it acts.
    fixture
        .deck
        .inner
        .inject(InputEvent::DialDown(POMODORO_DIAL))
        .unwrap();
    fixture
        .deck
        .inner
        .inject(InputEvent::DialUp(POMODORO_DIAL))
        .unwrap();
    tokio::time::sleep(SETTLE).await;
    assert!(fixture.strip().popup.is_some());
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
async fn the_keys_of_a_module_not_usable_when_locked_do_nothing() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, "list/list");

    // Locking goes back home: the page of a refused module is not left shown.
    fixture.lock(true).await;
    assert_eq!(fixture.page().id, HOME_PAGE_ID);

    fixture.engine.press_key(0).await.unwrap();
    fixture.tap_first_key().await;
    assert_eq!(fixture.page().id, HOME_PAGE_ID);

    // Unlocked, the same press works again.
    fixture.lock(false).await;
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, "list/list");
}

#[tokio::test(start_paused = true)]
async fn a_refused_key_wears_a_padlock_only_while_locked() {
    let fixture = locking_fixture(60).await;
    let open = fixture.deck.inner.key_image(0).unwrap();

    fixture.lock(true).await;
    let locked = fixture.deck.inner.key_image(0).unwrap();
    assert_ne!(open, locked, "a padlock is drawn");

    fixture.lock(false).await;
    assert_eq!(fixture.deck.inner.key_image(0).unwrap(), open);
}

#[tokio::test(start_paused = true)]
async fn a_module_usable_when_locked_keeps_its_key() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    let config = fixture.with(
        vec![list(4), instance("inbox", "notifications", json!({}))],
        &["list", "inbox"],
        &[None; 4],
    );
    fixture.engine.apply_config(config).await.unwrap();
    let open = fixture.deck.inner.key_image(1).unwrap();
    fixture.lock(true).await;

    assert_eq!(fixture.deck.inner.key_image(1).unwrap(), open, "no padlock");
    fixture.engine.press_key(1).await.unwrap();
    assert_ne!(fixture.page().id, HOME_PAGE_ID, "the notifications open");
}

#[tokio::test(start_paused = true)]
async fn dials_are_refused_with_a_padlock_unless_their_module_is_usable_when_locked() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture.wait_for(|f| f.strip().volume.is_some()).await;
    assert!(!fixture.hint(usize::from(VOLUME_DIAL)).locked);

    fixture.lock(true).await;

    assert!(
        fixture.hint(usize::from(VOLUME_DIAL)).locked,
        "the volume is refused"
    );
    assert!(
        !fixture.hint(usize::from(POMODORO_DIAL)).locked,
        "the pomodoro is not"
    );
    // The refused volume dial does nothing, the pomodoro dial opens its menu.
    fixture.engine.press_dial(VOLUME_DIAL).await.unwrap();
    assert!(!fixture.strip().volume.unwrap().muted);
    fixture.engine.press_dial(POMODORO_DIAL).await.unwrap();
    assert!(fixture.strip().popup.is_some());

    fixture.lock(false).await;
    assert!(!fixture.hint(usize::from(VOLUME_DIAL)).locked);
}

/// A device with a list of four items on its first key, whose page declares
/// what the arguments say, going dark `SLEEP_SECONDS` after a lock.
async fn declaring_fixture(lock_safe: bool, keeps_lit: bool) -> Fixture {
    declaring_fixture_of(lock_safe, keeps_lit, true).await
}

/// The same, the module declaring or not that its pages may declare.
async fn declaring_fixture_of(lock_safe: bool, keeps_lit: bool, module_allows: bool) -> Fixture {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .probe
        .list_lock_safe_pages
        .store(module_allows, Ordering::SeqCst);
    let mut module = list(4);
    module.settings = json!({ "items": 4, "lockSafe": lock_safe, "keepsLit": keeps_lit });
    let mut config = fixture.with(vec![module], &["list"], &[None; 4]);
    config.lock_sleep_seconds = SLEEP_SECONDS;
    fixture.engine.apply_config(config).await.unwrap();
    fixture
}

#[tokio::test(start_paused = true)]
async fn a_key_declared_usable_when_locked_acts_and_has_no_padlock() {
    let fixture = declaring_fixture(true, true).await;
    fixture.engine.press_key(0).await.unwrap();
    let open = fixture.deck.inner.key_image(1).unwrap();
    fixture.lock(true).await;
    // The page stays open, without padlock, and its first item (`i0`, which
    // goes back) acts.
    assert_eq!(fixture.page().id, "list/list");
    assert!(
        fixture.deck.inner.key_image(1).unwrap() == open,
        "no padlock"
    );
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, HOME_PAGE_ID, "the declared press acted");
}

#[tokio::test(start_paused = true)]
async fn an_item_not_declared_is_refused_with_a_padlock_even_on_a_page_kept_lit() {
    let fixture = declaring_fixture(false, true).await;
    fixture.engine.press_key(0).await.unwrap();
    let open = fixture.deck.inner.key_image(1).unwrap();
    fixture.lock(true).await;

    assert_eq!(fixture.page().id, "list/list", "the page is kept");
    assert!(fixture.deck.inner.key_image(1).unwrap() != open, "padlock");
    fixture.engine.press_key(0).await.unwrap();
    fixture.tap_first_key().await;
    assert_eq!(fixture.page().id, "list/list", "nothing acted");
}

#[tokio::test(start_paused = true)]
async fn a_page_that_keeps_the_device_lit_is_neither_closed_nor_darkened_by_the_lock() {
    let fixture = declaring_fixture(true, true).await;
    let lit = fixture.config().brightness;
    fixture.engine.press_key(0).await.unwrap();

    fixture.lock(true).await;
    tokio::time::sleep(SLEEP * 3).await;
    assert_eq!(fixture.brightness(), lit, "still lit long after the delay");
    assert_eq!(fixture.page().id, "list/list");
}

#[tokio::test(start_paused = true)]
async fn leaving_the_page_kept_lit_restarts_the_delay_before_going_dark() {
    let fixture = declaring_fixture(true, true).await;
    let lit = fixture.config().brightness;
    fixture.engine.press_key(0).await.unwrap();
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP * 3).await;

    // Back home (the first item goes back): the device goes dark after the delay.
    fixture.engine.press_key(0).await.unwrap();
    tokio::time::sleep(SLEEP.saturating_sub(SETTLE * 2)).await;
    assert_eq!(fixture.brightness(), lit, "the delay restarted");
    tokio::time::sleep(SETTLE * 4).await;
    assert_eq!(fixture.brightness(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_page_that_does_not_keep_the_device_lit_is_closed_and_the_device_goes_dark() {
    let fixture = declaring_fixture(true, false).await;
    fixture.engine.press_key(0).await.unwrap();
    fixture.lock(true).await;
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
    tokio::time::sleep(SLEEP + SETTLE * 2).await;
    assert_eq!(fixture.brightness(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_dark_device_lights_up_when_a_page_that_keeps_it_lit_opens() {
    let fixture = declaring_fixture(true, true).await;
    let lit = fixture.config().brightness;
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP + SETTLE * 2).await;
    assert_eq!(fixture.brightness(), 0, "dark on the home page");

    // An incoming call, say: the module opens its page while the device is dark.
    let events = fixture.probe.events.lock().unwrap().clone().unwrap();
    assert!(events.send(vec![Command::OpenPage(LIST_PAGE.into())]));
    tokio::time::sleep(SETTLE).await;
    assert_eq!(fixture.brightness(), lit);
    assert_eq!(fixture.page().id, "list/list");
    tokio::time::sleep(SLEEP * 3).await;
    assert_eq!(fixture.brightness(), lit, "and stays lit");
}

#[tokio::test(start_paused = true)]
async fn a_page_refused_while_locked_does_not_come_back_by_navigation() {
    let fixture = declaring_fixture(false, false).await;
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, "list/list");
    fixture.lock(true).await;
    assert_eq!(fixture.page().id, HOME_PAGE_ID);

    // A page opened by an event of its module is closed at once.
    let events = fixture.probe.events.lock().unwrap().clone().unwrap();
    assert!(events.send(vec![Command::OpenPage(LIST_PAGE.into())]));
    tokio::time::sleep(SETTLE).await;
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
}

fn tap_measures(fixture: &Fixture) {
    fixture
        .deck
        .inner
        .inject(InputEvent::TouchTap {
            point: TouchPoint { x: 400, y: 50 },
            long: false,
        })
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn tapping_the_measures_while_locked_and_lit_still_swaps_history_and_values() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    let before = fixture.config().strip.show_history;
    fixture.lock(true).await;

    tap_measures(&fixture);
    tokio::time::sleep(SETTLE).await;

    assert_eq!(fixture.config().strip.show_history, !before);
}

#[tokio::test(start_paused = true)]
async fn the_tap_that_wakes_a_dark_device_does_not_swap_the_measures() {
    let fixture = locking_fixture(SLEEP_SECONDS).await;
    let before = fixture.config().strip.show_history;
    fixture.lock(true).await;
    tokio::time::sleep(SLEEP + SETTLE).await;

    tap_measures(&fixture);
    tokio::time::sleep(SETTLE).await;

    assert_eq!(fixture.brightness(), fixture.config().brightness);
    assert_eq!(fixture.config().strip.show_history, before);
}

#[tokio::test(start_paused = true)]
async fn a_device_started_on_a_locked_computer_ignores_inputs_then_goes_dark() {
    let fixture = start_locked(virtual_plus_info(DEMO_SERIAL)).await;
    let mut config = fixture.with(vec![list(4)], &["list"], &[None; 4]);
    config.lock_sleep_seconds = SLEEP_SECONDS;
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
    config.lock_sleep_seconds = 60;
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
    config.lock_sleep_seconds = SLEEP_SECONDS;
    fixture.engine.apply_config(config).await.unwrap();
    tokio::time::sleep(SLEEP + SETTLE).await;
    assert_eq!(fixture.brightness(), 0);

    // Switching the feature off lights it again.
    let mut config = fixture.config();
    config.lock_sleep_seconds = 0;
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

#[tokio::test(start_paused = true)]
async fn what_a_page_declares_is_ignored_for_a_module_that_may_not() {
    let fixture = declaring_fixture_of(true, true, false).await;
    fixture.engine.press_key(0).await.unwrap();
    fixture.lock(true).await;
    // Neither kept open nor acting: the page is closed, then the device dark.
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
    tokio::time::sleep(SLEEP + SETTLE * 2).await;
    assert_eq!(fixture.brightness(), 0);
}

/// Contextual dial on dial 0, on the page of a list that declares `dial`.
async fn contextual_dial_fixture(dial: ContextDial) -> Fixture {
    contextual_dial_fixture_of(dial, true).await
}

async fn contextual_dial_fixture_of(dial: ContextDial, module_allows: bool) -> Fixture {
    let fixture = declaring_fixture_of(false, true, module_allows).await;
    let mut config = fixture.config();
    config.dials[0] = DialAssignment::Contextual;
    fixture.engine.apply_config(config).await.unwrap();
    *fixture.probe.context_dial.lock().unwrap() = dial;
    fixture.engine.press_key(0).await.unwrap();
    fixture.lock(true).await;
    fixture
}

fn context_calls(fixture: &Fixture) -> Vec<String> {
    fixture.probe.context_calls.lock().unwrap().clone()
}

#[tokio::test(start_paused = true)]
async fn a_contextual_dial_declared_usable_when_locked_acts_without_padlock() {
    let fixture = contextual_dial_fixture(
        ContextDial::none()
            .pressing("Répondre")
            .usable_when_locked(true),
    )
    .await;
    fixture.engine.press_dial(0).await.unwrap();
    assert_eq!(context_calls(&fixture), ["press list"]);
    assert!(!fixture.hint(0).locked);
}

#[tokio::test(start_paused = true)]
async fn a_contextual_dial_not_declared_is_refused_with_a_padlock_when_locked() {
    let fixture = contextual_dial_fixture(ContextDial::none().pressing("Répondre")).await;
    fixture.engine.press_dial(0).await.unwrap();
    assert_eq!(context_calls(&fixture), Vec::<String>::new());
    assert!(fixture.hint(0).locked);
}

#[tokio::test(start_paused = true)]
async fn a_contextual_dial_declaration_is_ignored_for_a_module_that_may_not() {
    let fixture = contextual_dial_fixture_of(
        ContextDial::none()
            .pressing("Répondre")
            .usable_when_locked(true),
        false,
    )
    .await;
    // Not even kept open: the page of a module that may not declare is closed.
    fixture.engine.press_dial(0).await.unwrap();
    assert_eq!(context_calls(&fixture), Vec::<String>::new());
}

#[tokio::test(start_paused = true)]
async fn items_hidden_when_locked_are_left_out_and_come_back() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .probe
        .list_lock_safe_pages
        .store(true, Ordering::SeqCst);
    let mut module = list(6);
    module.settings =
        json!({ "items": 6, "lockSafe": true, "keepsLit": true, "pinned": 2, "hiddenLast": 4 });
    let mut config = fixture.with(vec![module], &["list"], &[None; 4]);
    config.lock_sleep_seconds = SLEEP_SECONDS;
    fixture.engine.apply_config(config).await.unwrap();
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.labels()[4].as_deref(), Some("item 2"));

    fixture.lock(true).await;
    let locked = fixture.labels();
    assert_eq!(fixture.page().id, "list/list", "the page is kept");
    assert_eq!(locked[0].as_deref(), Some("item 0"), "pinned items stay");
    assert!(
        locked[4..].iter().all(Option::is_none),
        "the others are not shown: {locked:?}"
    );

    fixture.lock(false).await;
    assert_eq!(fixture.labels()[4].as_deref(), Some("item 2"));
}

#[tokio::test(start_paused = true)]
async fn a_home_key_declared_usable_when_locked_opens_its_page_without_padlock() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .probe
        .list_lock_safe_pages
        .store(true, Ordering::SeqCst);
    let mut module = list(4);
    module.settings =
        json!({ "items": 4, "lockSafe": true, "keepsLit": true, "keyLockSafe": true });
    let mut config = fixture.with(vec![module], &["list"], &[None; 4]);
    config.lock_sleep_seconds = SLEEP_SECONDS;
    fixture.engine.apply_config(config).await.unwrap();
    let open = fixture.deck.inner.key_image(0).unwrap();

    fixture.lock(true).await;
    assert_eq!(fixture.deck.inner.key_image(0).unwrap(), open, "no padlock");
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, "list/list", "the page opened");
}

#[tokio::test(start_paused = true)]
async fn a_home_key_declaration_is_ignored_without_the_page_capability() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .probe
        .list_lock_safe_pages
        .store(false, Ordering::SeqCst);
    let mut module = list(4);
    module.settings = json!({ "items": 4, "keyLockSafe": true });
    let mut config = fixture.with(vec![module], &["list"], &[None; 4]);
    config.lock_sleep_seconds = SLEEP_SECONDS;
    fixture.engine.apply_config(config).await.unwrap();

    fixture.lock(true).await;
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, HOME_PAGE_ID, "refused");
}
