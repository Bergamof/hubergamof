//! The contextual dial: the module of the displayed page says what it does.

use std::time::Duration;

use hubergamof_module_api::{Command, ContextDial, Feedback, Notification, Rgb};
use hubergamof_protocol::DialAssignment;

use super::support::*;
use crate::device::InputEvent;
use crate::device::virtual_deck::virtual_plus_info;
use crate::engine::HOME_PAGE_ID;
use crate::engine::device_engine::HOLD_THRESHOLD;

/// The contextual dial of the fixture.
const DIAL: u8 = 0;
/// Longer than [`crate::engine::device_engine::HOLD_THRESHOLD`].
const HOLD: Duration = Duration::from_millis(600);
const SETTLE: Duration = Duration::from_millis(100);
/// Long enough for the engine to take a press, much shorter than a hold.
const PRESS_TAKEN: Duration = Duration::from_millis(10);

/// A second contextual dial, for the tests that tell dials apart.
const OTHER_DIAL: u8 = 1;

/// A Stream Deck+ whose dial 0 is contextual, with a list of `items` items
/// on the home page.
async fn contextual_fixture(items: usize) -> Fixture {
    let fixture = start(virtual_plus_info("PLUS")).await;
    let mut config = fixture.with(
        vec![list(items), fragile("none")],
        &["list"],
        &[None, None, None, Some(FRAGILE_KIND)],
    );
    config.dials[usize::from(DIAL)] = DialAssignment::Contextual;
    fixture.engine.apply_config(config).await.unwrap();
    fixture
}

impl Fixture {
    /// What the pages of the list declare from now on, shown after a redraw.
    async fn declare(&self, dial: ContextDial) {
        let expected = dial.hint.clone();
        *self.probe.context_dial.lock().unwrap() = dial;
        self.request_redraw();
        self.wait_for(|fixture| {
            let hint = fixture.hint(usize::from(DIAL));
            hint.rotate == expected.rotate && hint.press == expected.press
        })
        .await;
    }

    fn request_redraw(&self) {
        let redraw = self.probe.redraw.lock().unwrap().clone().unwrap();
        redraw.request_redraw();
    }

    async fn open_list(&self) {
        self.engine.press_key(0).await.unwrap();
        assert_eq!(self.page().id, "list/list");
    }

    /// Shows a notification, sent by the fragile module.
    fn raise_notification(&self) {
        let events = self.probe.fragile_events.lock().unwrap().clone().unwrap();
        let notification = Notification::new("Pause !", Rgb(200, 30, 30));
        assert!(events.send(vec![Command::Notify(notification)]));
    }

    /// Waits until a notification covers the strip.
    async fn wait_until_covered(&self) {
        self.wait_for(|fixture| {
            fixture
                .live()
                .strip
                .is_some_and(|strip| strip.image.is_none())
        })
        .await;
    }

    fn context_calls(&self) -> Vec<String> {
        self.probe.context_calls.lock().unwrap().clone()
    }

    fn send_input(&self, event: InputEvent) {
        self.deck.inner.inject(event).unwrap();
    }

    async fn wait_for_calls(&self, expected: &[&str]) {
        self.wait_for(|fixture| fixture.context_calls() == expected)
            .await;
    }
}

#[tokio::test(start_paused = true)]
async fn nothing_is_shown_nor_done_where_nothing_is_declared() {
    let fixture = contextual_fixture(4).await;
    // The home page has no module page to ask.
    assert_eq!(fixture.hint(usize::from(DIAL)).rotate, None);
    fixture.engine.rotate_dial(DIAL, 1).await.unwrap();
    fixture.engine.press_dial(DIAL).await.unwrap();

    // Neither has a page that declares nothing.
    fixture.open_list().await;
    assert_eq!(fixture.hint(usize::from(DIAL)).press, None);
    fixture.engine.rotate_dial(DIAL, 1).await.unwrap();
    fixture.engine.press_dial(DIAL).await.unwrap();
    fixture.send_input(InputEvent::DialDown(DIAL));
    tokio::time::sleep(HOLD).await;
    fixture.send_input(InputEvent::DialUp(DIAL));
    tokio::time::sleep(SETTLE).await;

    assert_eq!(fixture.context_calls(), Vec::<String>::new());
    assert_eq!(fixture.dial_mark(usize::from(DIAL)), None);
}

#[tokio::test(start_paused = true)]
async fn the_page_says_what_the_dial_does_and_its_module_handles_it() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    fixture
        .declare(ContextDial::none().rotating("Choisir").pressing("Cocher"))
        .await;

    fixture.engine.rotate_dial(DIAL, 3).await.unwrap();
    fixture.engine.press_dial(DIAL).await.unwrap();
    assert_eq!(fixture.context_calls(), ["rotate list 3", "press list"]);
    // The outcome shows on the reminder of the dial.
    assert_eq!(
        fixture.dial_mark(usize::from(DIAL)),
        Some(Feedback::Ok),
        "the module answered with a check mark"
    );

    // Back on the home page, the reminder is gone and the dial inert.
    // (the first item of the list goes back)
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
    assert_eq!(fixture.hint(usize::from(DIAL)).press, None);
    fixture.engine.rotate_dial(DIAL, 1).await.unwrap();
    fixture.engine.press_dial(DIAL).await.unwrap();
    assert_eq!(fixture.context_calls().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_gesture_the_page_does_not_declare_does_nothing() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    fixture
        .declare(ContextDial::none().pressing("Cocher"))
        .await;
    assert_eq!(fixture.hint(usize::from(DIAL)).rotate, None);

    fixture.engine.rotate_dial(DIAL, 2).await.unwrap();
    fixture.engine.press_dial(DIAL).await.unwrap();
    assert_eq!(fixture.context_calls(), ["press list"]);
}

#[tokio::test(start_paused = true)]
async fn a_declared_long_press_acts_when_held_long_enough() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    fixture
        .declare(ContextDial::none().pressing("Cocher").long_pressing("Tout"))
        .await;

    // Held: the long press comes without waiting for the release...
    fixture.send_input(InputEvent::DialDown(DIAL));
    tokio::time::sleep(SETTLE).await;
    assert_eq!(fixture.context_calls(), Vec::<String>::new());
    tokio::time::sleep(HOLD).await;
    fixture.wait_for_calls(&["long list"]).await;
    // ...and the release then does nothing more.
    fixture.send_input(InputEvent::DialUp(DIAL));
    tokio::time::sleep(SETTLE).await;
    assert_eq!(fixture.context_calls(), ["long list"]);

    // Released early: a plain press.
    fixture.send_input(InputEvent::DialDown(DIAL));
    fixture.send_input(InputEvent::DialUp(DIAL));
    fixture.wait_for_calls(&["long list", "press list"]).await;
}

#[tokio::test(start_paused = true)]
async fn a_short_press_of_a_dial_with_only_a_long_press_does_nothing() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    fixture
        .declare(
            ContextDial::none()
                .rotating("Choisir")
                .long_pressing("Tout"),
        )
        .await;

    fixture.send_input(InputEvent::DialDown(DIAL));
    fixture.send_input(InputEvent::DialUp(DIAL));
    tokio::time::sleep(HOLD).await;
    assert_eq!(fixture.context_calls(), Vec::<String>::new());
}

#[tokio::test(start_paused = true)]
async fn without_a_declared_long_press_the_press_acts_at_once() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    fixture
        .declare(ContextDial::none().pressing("Cocher"))
        .await;

    fixture.send_input(InputEvent::DialDown(DIAL));
    fixture.wait_for_calls(&["press list"]).await;
    // The release adds nothing.
    fixture.send_input(InputEvent::DialUp(DIAL));
    tokio::time::sleep(SETTLE).await;
    assert_eq!(fixture.context_calls(), ["press list"]);
}

#[tokio::test(start_paused = true)]
async fn a_simulated_press_is_a_plain_press_even_with_a_long_press() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    fixture
        .declare(ContextDial::none().pressing("Cocher").long_pressing("Tout"))
        .await;

    fixture.engine.press_dial(DIAL).await.unwrap();
    assert_eq!(fixture.context_calls(), ["press list"]);
}

#[tokio::test(start_paused = true)]
async fn the_page_scrolls_to_the_key_holding_the_cursor() {
    // 20 items on 4 columns: 5 rows, 2 of them visible.
    let fixture = contextual_fixture(20).await;
    fixture.open_list().await;
    assert_eq!(fixture.page().row, 0);

    let focus = |item: Option<usize>| {
        *fixture.probe.focus.lock().unwrap() = item;
        fixture.request_redraw();
    };
    focus(Some(12));
    fixture.wait_for(|fixture| fixture.page().row == 2).await;
    // Still in view: no movement.
    focus(Some(9));
    fixture.wait_for(|fixture| fixture.page().row == 2).await;
    focus(Some(1));
    fixture.wait_for(|fixture| fixture.page().row == 0).await;
}

#[tokio::test(start_paused = true)]
async fn a_page_scrolled_by_hand_is_left_alone_until_the_cursor_moves() {
    let fixture = contextual_fixture(20).await;
    fixture.open_list().await;
    *fixture.probe.focus.lock().unwrap() = Some(1);
    fixture.request_redraw();
    tokio::time::sleep(SETTLE).await;
    assert_eq!(fixture.page().row, 0);

    // The user scrolls away (item 2 scrolls one row), the cursor stays.
    fixture.engine.press_key(2).await.unwrap();
    assert_eq!(fixture.page().row, 1);
    fixture.request_redraw();
    tokio::time::sleep(SETTLE).await;
    assert_eq!(fixture.page().row, 1);
}

#[tokio::test(start_paused = true)]
async fn the_cursor_and_the_ticks_are_drawn_on_the_keys() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    // Pixel of the top edge of key `index`: the frame of the cursor crosses it.
    let edge = |index: usize| {
        let image = fixture.live().keys[index].image.clone().unwrap();
        *image.image().get_pixel(60, 1)
    };
    let plain = edge(2);
    assert_eq!(edge(1), plain);

    *fixture.probe.focus.lock().unwrap() = Some(1);
    fixture.request_redraw();
    fixture.wait_for(|_| edge(1) != plain).await;
    assert_eq!(edge(2), plain, "only the focused key is framed");

    *fixture.probe.focus.lock().unwrap() = Some(2);
    fixture.request_redraw();
    fixture.wait_for(|_| edge(2) != plain).await;
    assert_eq!(edge(1), plain, "the cursor left the first key");
}

#[tokio::test(start_paused = true)]
async fn a_notification_during_the_hold_cancels_the_long_press() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    fixture
        .declare(ContextDial::none().pressing("Cocher").long_pressing("Tout"))
        .await;

    fixture.send_input(InputEvent::DialDown(DIAL));
    tokio::time::sleep(PRESS_TAKEN).await;
    fixture.raise_notification();
    fixture.wait_until_covered().await;
    tokio::time::sleep(HOLD).await;
    fixture.send_input(InputEvent::DialUp(DIAL));
    tokio::time::sleep(SETTLE).await;
    assert_eq!(fixture.context_calls(), Vec::<String>::new());
}

#[tokio::test(start_paused = true)]
async fn a_notification_during_the_hold_cancels_the_press_of_the_release() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    fixture
        .declare(ContextDial::none().pressing("Cocher").long_pressing("Tout"))
        .await;

    fixture.send_input(InputEvent::DialDown(DIAL));
    tokio::time::sleep(PRESS_TAKEN).await;
    fixture.raise_notification();
    fixture.wait_until_covered().await;
    fixture.send_input(InputEvent::DialUp(DIAL));
    tokio::time::sleep(SETTLE).await;
    assert_eq!(fixture.context_calls(), Vec::<String>::new());
}

#[tokio::test(start_paused = true)]
async fn the_release_of_another_dial_does_not_end_the_hold() {
    let fixture = contextual_fixture(4).await;
    let mut config = fixture.config();
    config.dials[usize::from(OTHER_DIAL)] = DialAssignment::Contextual;
    fixture.engine.apply_config(config).await.unwrap();
    fixture.open_list().await;
    fixture
        .declare(ContextDial::none().pressing("Cocher").long_pressing("Tout"))
        .await;

    fixture.send_input(InputEvent::DialDown(DIAL));
    fixture.send_input(InputEvent::DialUp(OTHER_DIAL));
    tokio::time::sleep(HOLD).await;
    fixture.wait_for_calls(&["long list"]).await;
}

#[tokio::test(start_paused = true)]
async fn a_release_without_a_press_does_nothing() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    fixture
        .declare(ContextDial::none().pressing("Cocher").long_pressing("Tout"))
        .await;
    fixture.send_input(InputEvent::DialUp(DIAL));
    tokio::time::sleep(HOLD).await;
    assert_eq!(fixture.context_calls(), Vec::<String>::new());
}

#[tokio::test(start_paused = true)]
async fn turning_a_dial_held_pressed_is_neither_a_press_nor_a_long_one() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    fixture
        .declare(
            ContextDial::none()
                .rotating("Choisir")
                .pressing("Cocher")
                .long_pressing("Tout"),
        )
        .await;

    fixture.send_input(InputEvent::DialDown(DIAL));
    fixture.send_input(InputEvent::DialRotate {
        index: DIAL,
        ticks: 1,
    });
    fixture.wait_for_calls(&["rotate list 1"]).await;
    tokio::time::sleep(HOLD).await;
    fixture.send_input(InputEvent::DialUp(DIAL));
    tokio::time::sleep(SETTLE).await;
    assert_eq!(fixture.context_calls(), ["rotate list 1"]);
}

#[tokio::test(start_paused = true)]
async fn the_long_press_comes_at_the_threshold_and_not_before() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    fixture
        .declare(ContextDial::none().pressing("Cocher").long_pressing("Tout"))
        .await;

    // Just short of it: a press, on release.
    fixture.send_input(InputEvent::DialDown(DIAL));
    tokio::time::sleep(HOLD_THRESHOLD.saturating_sub(Duration::from_millis(1))).await;
    fixture.send_input(InputEvent::DialUp(DIAL));
    fixture.wait_for_calls(&["press list"]).await;

    // Just past it: a long press, before the release.
    fixture.send_input(InputEvent::DialDown(DIAL));
    tokio::time::sleep(HOLD_THRESHOLD + Duration::from_millis(1)).await;
    fixture.wait_for_calls(&["press list", "long list"]).await;
    fixture.send_input(InputEvent::DialUp(DIAL));
    tokio::time::sleep(SETTLE).await;
    assert_eq!(fixture.context_calls(), ["press list", "long list"]);
}

#[tokio::test(start_paused = true)]
async fn a_held_dial_acts_on_the_page_it_was_pressed_on() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    fixture
        .declare(ContextDial::none().pressing("Cocher").long_pressing("Tout"))
        .await;

    fixture.send_input(InputEvent::DialDown(DIAL));
    // The press is taken before the page goes, whatever the order of wake-ups.
    tokio::time::sleep(PRESS_TAKEN).await;
    *fixture.probe.gone_page.lock().unwrap() = Some("list".into());
    fixture.request_redraw();
    // Less than the hold: the page is gone before the dial is released.
    tokio::time::sleep(SETTLE).await;
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
    fixture.send_input(InputEvent::DialUp(DIAL));
    fixture.wait_for_calls(&["press list"]).await;
}

#[tokio::test(start_paused = true)]
async fn a_new_configuration_ends_the_hold() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    fixture
        .declare(ContextDial::none().pressing("Cocher").long_pressing("Tout"))
        .await;

    fixture.send_input(InputEvent::DialDown(DIAL));
    tokio::time::sleep(PRESS_TAKEN).await;
    let mut config = fixture.config();
    config.dials[usize::from(DIAL)] = DialAssignment::None;
    fixture.engine.apply_config(config).await.unwrap();
    tokio::time::sleep(HOLD).await;
    fixture.send_input(InputEvent::DialUp(DIAL));
    tokio::time::sleep(SETTLE).await;
    assert_eq!(fixture.context_calls(), Vec::<String>::new());
}

#[tokio::test(start_paused = true)]
async fn the_ticks_are_drawn_on_the_keys() {
    let fixture = contextual_fixture(4).await;
    fixture.open_list().await;
    // Inside the badge of a key, near its left edge (120 px keys).
    let badge = |index: usize| {
        let image = fixture.live().keys[index].image.clone().unwrap();
        *image.image().get_pixel(76, 29)
    };
    let plain = badge(2);
    assert_eq!(badge(1), plain);

    *fixture.probe.checked.lock().unwrap() = vec![1];
    fixture.request_redraw();
    fixture.wait_for(|_| badge(1) != plain).await;
    assert_eq!(badge(2), plain, "only the ticked key");
}
