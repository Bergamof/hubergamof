//! How the engine drives module code: events from background tasks,
//! validation of settings, presses resolved on what is displayed, feedback
//! marks, panics and slow handlers.

use std::sync::atomic::Ordering;

use bergadeck_module_api::{Command, Feedback, HANDLER_BUDGET, ItemId, KeyState, ModuleSurface};
use bergadeck_protocol::{ErrorCode, ModuleInstance};
use image::RgbaImage;
use serde_json::json;
use tokio::time::Instant;

use super::support::*;
use crate::device::virtual_deck::{DEMO_SERIAL, virtual_plus_info};
use crate::device::{DeckBackend, InputEvent};
use crate::engine::device_engine::MODULE_EVENT_CAPACITY;
use crate::engine::feedback::FEEDBACK_DURATION;
use crate::engine::*;
use crate::render::ImageKind;

/// Lets the engine task run until it is idle.
async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

fn fragile_with(settings: serde_json::Value) -> ModuleInstance {
    instance(FRAGILE_KIND, FRAGILE_KIND, settings)
}

// ---------------------------------------------------------------- Events

#[tokio::test]
async fn events_from_background_tasks_change_the_module_state() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(0)], &["list"], &[]).await;
    let events = fixture.probe.events.lock().unwrap().clone().unwrap();

    fixture
        .after_change(|| {
            tokio::spawn(async move { assert!(events.send(Bump)) });
        })
        .await;
    assert_eq!(fixture.labels()[0].as_deref(), Some("list 10"));
    // The module reported success on its home key.
    assert_eq!(fixture.key_mark(0), Some(Feedback::Ok));
}

#[tokio::test]
async fn events_of_a_replaced_instance_are_not_handled() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(0)], &["list"], &[]).await;
    let stale = fixture.probe.events.lock().unwrap().clone().unwrap();
    fixture.configure(Vec::new(), &[], &[]).await;
    fixture.configure(vec![list(0)], &["list"], &[]).await;
    let current = fixture.probe.events.lock().unwrap().clone().unwrap();

    assert!(!stale.send(Bump));
    fixture.after_change(|| assert!(current.send(Bump))).await;
    assert_eq!(fixture.probe.list_events.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.labels()[0].as_deref(), Some("list 10"));
}

#[tokio::test]
async fn senders_tell_when_their_instance_is_gone() {
    let fixture = start(no_dial_info()).await;
    fixture
        .configure(
            vec![list(0), fragile("press")],
            &["list", FRAGILE_KIND],
            &[],
        )
        .await;
    let list_events = fixture.probe.events.lock().unwrap().clone().unwrap();
    let fragile_events = fixture
        .probe
        .fragile_events
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    fixture
        .after_change(|| assert!(fragile_events.send(())))
        .await;

    // The fragile instance panicked handling the event: replaced.
    assert!(!fragile_events.send(()));
    assert!(list_events.send(Bump));

    fixture.engine.stop().await;
    assert!(!list_events.send(Bump));
}

#[tokio::test]
async fn senders_tell_when_the_queue_is_full() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(0)], &["list"], &[]).await;
    let events = fixture.probe.events.lock().unwrap().clone().unwrap();
    // The engine cannot run in between: nothing leaves the queue.
    let accepted = (0..=MODULE_EVENT_CAPACITY)
        .filter(|_| events.send(Bump))
        .count();
    assert_eq!(accepted, MODULE_EVENT_CAPACITY);
    assert!(!events.send(Bump));
    fixture.after_change(|| {}).await;
    assert!(events.send(Bump));
}

#[tokio::test]
async fn events_of_a_dial_module_mark_its_zone() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(
            vec![fragile("none")],
            &[],
            &[Some(FRAGILE_KIND), None, None, None],
        )
        .await;
    let events = fixture
        .probe
        .fragile_events
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    fixture.after_change(|| assert!(events.send(()))).await;
    assert_eq!(fixture.zone_mark(0), Some(Feedback::Ok));
    assert_eq!(fixture.zone_mark(1), None);
}

#[tokio::test]
async fn events_can_mark_a_named_surface_of_their_module() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(
            vec![fragile("none")],
            &[FRAGILE_KIND],
            &[Some(FRAGILE_KIND), None, None, None],
        )
        .await;
    let events = fixture
        .probe
        .fragile_events
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    let mark = |surface| vec![Command::FeedbackOn(surface, Feedback::Alert)];

    let on_zone = events.clone();
    fixture
        .after_change(|| assert!(on_zone.send(mark(ModuleSurface::Zone))))
        .await;
    assert_eq!(fixture.zone_mark(0), Some(Feedback::Alert));
    assert_eq!(fixture.key_mark(0), None);

    let on_home = events.clone();
    fixture
        .after_change(|| assert!(on_home.send(mark(ModuleSurface::HomeKey))))
        .await;
    assert_eq!(fixture.key_mark(0), Some(Feedback::Alert));

    fixture.engine.press_key(0).await.unwrap();
    fixture.wait_for(|f| f.page().id != HOME_PAGE_ID).await;
    let item = ModuleSurface::Item {
        page: FRAGILE_PAGE.into(),
        item: "x".into(),
    };
    fixture
        .after_change(|| assert!(events.send(mark(item))))
        .await;
    assert_eq!(fixture.key_mark(0), Some(Feedback::Alert));
}

// ---------------------------------------------------------------- Validation

#[tokio::test]
async fn invalid_settings_are_refused_before_saving() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    let current = fixture
        .configure(vec![list(2)], &["list"], &[None; 4])
        .await;

    let cases = [
        fixture.with(vec![list(1000)], &["list"], &[None; 4]),
        fixture.with(
            vec![
                list(1),
                instance("clock", "clock", json!({ "showSeconds": "yes" })),
            ],
            &["list"],
            &[Some("clock"), None, None, None],
        ),
    ];
    for config in cases {
        let error = fixture.engine.apply_config(config).await.unwrap_err();
        assert_eq!(error.error_code(), ErrorCode::InvalidConfig, "{error}");
        assert!(matches!(error, EngineError::InvalidConfig { .. }));
    }
    assert_eq!(fixture.config(), current);
    let stored = fixture.store.load(fixture.deck.info()).unwrap().config;
    assert_eq!(stored.revision, current.revision);
}

#[tokio::test]
async fn modules_are_placed_only_where_they_work() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    let current = fixture.config();
    let nav = || instance("nav", "navigation", json!({}));
    let cases = [
        // Navigation has no key.
        (fixture.with(vec![nav()], &["nav"], &[None; 4]), "nav"),
        // The list has no dial.
        (
            fixture.with(vec![list(0)], &[], &[Some("list"), None, None, None]),
            "list",
        ),
    ];
    for (config, offender) in cases {
        match fixture.engine.apply_config(config).await.unwrap_err() {
            EngineError::InvalidConfig { module, .. } => assert_eq!(module.0, offender),
            other => panic!("unexpected error {other}"),
        }
    }
    assert_eq!(fixture.config(), current);
    // Unknown kinds are not checked: they show a placeholder.
    fixture
        .configure(
            vec![instance("x", "teleporter", json!({ "any": 1 }))],
            &["x"],
            &[Some("x"), None, None, None],
        )
        .await;
}

#[tokio::test]
async fn settings_are_given_to_the_module_only_when_they_change() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(2)], &["list"], &[]).await;
    let updates = || fixture.probe.settings_updates.load(Ordering::SeqCst);
    assert_eq!(updates(), 0);

    let mut relabelled = list(2);
    relabelled.label = Some("Liste".into());
    fixture.configure(vec![relabelled], &["list"], &[]).await;
    assert_eq!(updates(), 0);

    fixture.configure(vec![list(3)], &["list"], &[]).await;
    assert_eq!(updates(), 1);
}

#[tokio::test]
async fn settings_refused_by_the_module_keep_the_previous_ones() {
    let fixture = start(no_dial_info()).await;
    let tagged = |tag: &str, reject: bool| {
        fragile_with(json!({ "failure": "none", "tag": tag, "reject": reject }))
    };
    fixture
        .configure(vec![tagged("a", false)], &[FRAGILE_KIND], &[])
        .await;
    fixture
        .configure(vec![tagged("b", true)], &[FRAGILE_KIND], &[])
        .await;
    assert_eq!(fixture.labels()[0].as_deref(), Some("a"));
    // The refused settings were not taken as current: they are offered again.
    fixture
        .configure(vec![tagged("b", false)], &[FRAGILE_KIND], &[])
        .await;
    assert_eq!(fixture.labels()[0].as_deref(), Some("b"));
}

// ---------------------------------------------------------------- Displayed targets

#[tokio::test]
async fn presses_act_on_the_item_displayed() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(6)], &["list"], &[]).await;
    fixture.engine.press_key(0).await.unwrap();

    // The module renumbers its items without asking for a redraw: the key
    // still shows `i5`, so that is what a press reaches.
    fixture.probe.item_shift.store(100, Ordering::SeqCst);
    fixture.engine.press_key(5).await.unwrap();
    assert_eq!(*fixture.probe.pressed.lock().unwrap(), [ItemId::from("i5")]);
    // The redraw following the press shows the new items.
    fixture.engine.press_key(5).await.unwrap();
    assert_eq!(
        fixture.probe.pressed.lock().unwrap().last(),
        Some(&ItemId::from("i105"))
    );
}

#[tokio::test]
async fn queued_hardware_inputs_cost_one_redraw() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(
            vec![fragile("none")],
            &[],
            &[Some(FRAGILE_KIND), None, None, None],
        )
        .await;
    let _ = fixture.deck.flushes();
    fixture
        .after_change(|| {
            for _ in 0..3 {
                let rotate = InputEvent::DialRotate { index: 0, ticks: 1 };
                fixture.deck.inner.inject(rotate).unwrap();
            }
        })
        .await;
    assert_eq!(fixture.zone_lines(0), ["fragile 3"]);
    assert_eq!(fixture.deck.flushes(), 1);
}

// ---------------------------------------------------------------- Feedback and state

#[tokio::test(start_paused = true)]
async fn feedback_marks_the_pressed_key_for_a_moment() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(5)], &["list"], &[]).await;
    fixture.engine.press_key(0).await.unwrap();

    let pressed = Instant::now();
    fixture.engine.press_key(4).await.unwrap();
    assert_eq!(fixture.key_mark(4), Some(Feedback::Ok));
    assert_eq!(fixture.key_mark(3), None);
    let marked = fixture.live().keys[4].image.clone().unwrap();

    // The mark goes away by itself.
    fixture.after_change(|| {}).await;
    assert!(pressed.elapsed() >= FEEDBACK_DURATION);
    assert_eq!(fixture.key_mark(4), None);
    let unmarked = fixture.live().keys[4].image.clone().unwrap();
    assert_ne!(marked.image(), unmarked.image());
    assert_eq!(fixture.labels()[4].as_deref(), Some("item 4"));
}

#[tokio::test]
async fn marks_follow_their_item_and_leave_with_its_page() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(8)], &["list"], &[]).await;
    fixture.engine.press_key(0).await.unwrap();
    fixture.engine.press_key(4).await.unwrap();
    assert_eq!(fixture.key_mark(4), Some(Feedback::Ok));

    // Item i2 scrolls one row: item 4 moves to key 1, its mark with it.
    fixture.engine.press_key(2).await.unwrap();
    assert_eq!(fixture.labels()[1].as_deref(), Some("item 4"));
    assert_eq!(fixture.key_mark(1), Some(Feedback::Ok));
    assert_eq!(fixture.key_mark(4), None);

    // Item i3 (key 0) opens another page: its keys carry no mark.
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, "list/nested");
    assert!(fixture.live().keys.iter().all(|key| key.feedback.is_none()));
}

#[tokio::test(start_paused = true)]
async fn dial_handlers_mark_the_zone_of_their_dial() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(
            vec![fragile("none")],
            &[],
            &[Some(FRAGILE_KIND), None, None, None],
        )
        .await;
    fixture.engine.press_dial(0).await.unwrap();
    assert_eq!(fixture.zone_mark(0), Some(Feedback::Ok));
    assert_eq!(fixture.zone_mark(1), None);
    fixture.after_change(|| {}).await;
    assert_eq!(fixture.zone_mark(0), None);

    fixture.engine.rotate_dial(0, 2).await.unwrap();
    assert_eq!(fixture.zone_mark(0), Some(Feedback::Ok));
    assert_eq!(fixture.zone_lines(0), ["fragile 2"]);
}

#[tokio::test(start_paused = true)]
async fn failing_dial_handlers_alert_on_their_zone() {
    for failure in ["press", "slow"] {
        let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
        fixture
            .configure(
                vec![fragile(failure)],
                &[],
                &[Some(FRAGILE_KIND), None, None, None],
            )
            .await;
        fixture.engine.rotate_dial(0, 1).await.unwrap();
        assert_eq!(fixture.zone_mark(0), Some(Feedback::Alert), "{failure}");
    }
}

#[tokio::test]
async fn dials_without_a_zone_show_no_mark() {
    let fixture = start(dials_without_strip_info()).await;
    fixture
        .configure(vec![fragile("none")], &[], &[Some(FRAGILE_KIND), None])
        .await;
    fixture.engine.press_dial(0).await.unwrap();
    fixture.engine.rotate_dial(0, 1).await.unwrap();
    let live = fixture.live();
    assert_eq!(live.dials.len(), 2);
    assert!(live.dials.iter().all(|dial| dial.zone.is_none()));
    assert!(live.keys.iter().all(|key| key.feedback.is_none()));
}

#[tokio::test]
async fn a_new_mark_reuses_the_image_under_it() {
    let fixture = start(no_dial_info()).await;
    let mut png = std::io::Cursor::new(Vec::new());
    RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let name = fixture
        .store
        .store_image("NODIAL", png.get_ref(), ImageKind::Png)
        .unwrap();
    let mut with_image = list(0);
    with_image.image = Some(name.clone());
    fixture.configure(vec![with_image], &["list"], &[]).await;

    // Reading the image again would now fail and show the label alone.
    std::fs::remove_file(fixture.store.image_path("NODIAL", &name).unwrap()).unwrap();
    let events = fixture.probe.events.lock().unwrap().clone().unwrap();
    fixture.after_change(|| assert!(events.send(()))).await;
    assert_eq!(fixture.key_mark(0), Some(Feedback::Ok));
    // Darkened red above the mark: the image is still there.
    let [red, green, ..] = fixture.deck.inner.key_image(0).unwrap().get_pixel(36, 4).0;
    assert!(red > 50 && green < 20, "{red} {green}");
}

#[tokio::test]
async fn key_states_are_drawn_and_published() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(1)], &["list"], &[]).await;
    let normal = fixture.deck.inner.key_image(0).unwrap();
    fixture.engine.press_key(0).await.unwrap();
    fixture.engine.press_key(0).await.unwrap();
    // Back home after one press: the key is active.
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
    assert_eq!(fixture.live().keys[0].state, KeyState::Active);
    let state = fixture.live().to_live_state().unwrap();
    assert_eq!(state.page.keys[0].state, KeyState::Active);
    let active = fixture.deck.inner.key_image(0).unwrap();
    assert_ne!(normal.get_pixel(0, 36), active.get_pixel(0, 36));
}

// ---------------------------------------------------------------- Panics

#[tokio::test]
async fn a_module_panicking_in_its_zone_is_replaced() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(
            vec![list(0), fragile("zone")],
            &["list"],
            &[Some(FRAGILE_KIND), None, None, None],
        )
        .await;
    // Placeholder without zone, with an alert over the zone of its dial.
    assert_eq!(fixture.zone_lines(0), Vec::<String>::new());
    assert_eq!(fixture.zone_mark(0), Some(Feedback::Alert));
    assert_eq!(fixture.live().dials[0].module_id, Some(FRAGILE_KIND.into()));

    // The device keeps working.
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, "list/list");
    fixture.engine.press_dial(0).await.unwrap();
}

#[tokio::test]
async fn a_module_panicking_in_a_handler_is_replaced() {
    let fixture = start(no_dial_info()).await;
    fixture
        .configure(
            vec![fragile("press"), list(2)],
            &[FRAGILE_KIND, "list"],
            &[],
        )
        .await;
    assert_eq!(fixture.labels()[0].as_deref(), Some(FRAGILE_KIND));

    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.labels()[0].as_deref(), Some("fragile indisponible"));
    assert_eq!(fixture.key_mark(0), Some(Feedback::Alert));
    // Pressing the placeholder does nothing; the other modules work.
    fixture.engine.press_key(0).await.unwrap();
    fixture.engine.press_key(1).await.unwrap();
    assert_eq!(fixture.page().id, "list/list");

    // Item i1 goes home; applying the configuration again retries the module.
    fixture.engine.press_key(1).await.unwrap();
    fixture.engine.apply_config(fixture.config()).await.unwrap();
    assert_eq!(fixture.labels()[0].as_deref(), Some(FRAGILE_KIND));
}

#[tokio::test]
async fn modules_panicking_anywhere_are_replaced() {
    for failure in ["key", "page", "settings", "create"] {
        let fixture = start(no_dial_info()).await;
        let initial = if failure == "settings" {
            "none"
        } else {
            failure
        };
        fixture
            .configure(
                vec![fragile(initial), list(0)],
                &[FRAGILE_KIND, "list"],
                &[],
            )
            .await;
        match failure {
            // Opening its page fails: back home.
            "page" => fixture.engine.press_key(0).await.unwrap(),
            "settings" => {
                fixture
                    .configure(
                        vec![fragile("settings"), list(0)],
                        &[FRAGILE_KIND, "list"],
                        &[],
                    )
                    .await;
            }
            _ => {}
        }
        assert_eq!(fixture.page().id, HOME_PAGE_ID, "{failure}");
        assert_eq!(
            fixture.labels()[0].as_deref(),
            Some("fragile indisponible"),
            "{failure}"
        );
        assert_eq!(fixture.key_mark(0), Some(Feedback::Alert), "{failure}");
        fixture.engine.press_key(1).await.unwrap();
        assert_eq!(fixture.page().id, "list/list", "{failure}");
    }
}

#[tokio::test]
async fn a_failed_creation_leaves_no_task_behind() {
    let fixture = start(no_dial_info()).await;
    fixture
        .configure(vec![fragile("abandon")], &[FRAGILE_KIND], &[])
        .await;
    settle().await;
    assert_eq!(fixture.labels()[0].as_deref(), Some("fragile indisponible"));
    assert_eq!(fixture.probe.abandoned_tasks.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_panicking_background_task_raises_an_alert() {
    let fixture = start(no_dial_info()).await;
    fixture
        .configure(vec![fragile("task"), list(0)], &[FRAGILE_KIND, "list"], &[])
        .await;
    fixture.after_change(|| {}).await;
    assert_eq!(fixture.key_mark(0), Some(Feedback::Alert));
    // The module itself keeps running.
    assert_eq!(fixture.labels()[0].as_deref(), Some(FRAGILE_KIND));
    fixture.engine.press_key(1).await.unwrap();
    assert_eq!(fixture.page().id, "list/list");
}

// ---------------------------------------------------------------- Budget

#[tokio::test(start_paused = true)]
async fn slow_handlers_are_abandoned_within_the_budget() {
    let fixture = start(no_dial_info()).await;
    fixture
        .configure(vec![fragile("slow"), list(0)], &[FRAGILE_KIND, "list"], &[])
        .await;

    let pressed = Instant::now();
    fixture.engine.press_key(0).await.unwrap();
    let waited = pressed.elapsed();
    assert!(waited >= HANDLER_BUDGET, "{waited:?}");
    assert!(waited < SLOW_HANDLER, "{waited:?}");
    assert_eq!(fixture.key_mark(0), Some(Feedback::Alert));
    // Not replaced: a slow module is not a broken one.
    assert_eq!(fixture.labels()[0].as_deref(), Some(FRAGILE_KIND));

    fixture.engine.press_key(1).await.unwrap();
    assert_eq!(fixture.page().id, "list/list");
}

#[tokio::test(start_paused = true)]
async fn every_handler_has_the_budget() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    let modules = |failure| vec![fragile(failure), list(0)];
    let dials = [Some(FRAGILE_KIND), None, None, None];
    fixture
        .configure(modules("slow"), &[FRAGILE_KIND, "list"], &dials)
        .await;

    // An event, on the home page where the module key is shown.
    let events = fixture
        .probe
        .fragile_events
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    let sent = Instant::now();
    fixture.after_change(|| assert!(events.send(()))).await;
    assert!(sent.elapsed() < SLOW_HANDLER);
    assert_eq!(fixture.key_mark(0), Some(Feedback::Alert));
    assert_eq!(
        fixture.probe.fragile_events_handled.load(Ordering::SeqCst),
        0
    );

    // The dial.
    let started = Instant::now();
    fixture.engine.press_dial(0).await.unwrap();
    fixture.engine.rotate_dial(0, 1).await.unwrap();
    assert!(started.elapsed() < SLOW_HANDLER);
    assert_eq!(fixture.zone_mark(0), Some(Feedback::Alert));
    assert_eq!(fixture.zone_lines(0), ["fragile 0"]);

    // An item of its page, opened while the module was quick.
    fixture
        .configure(modules("none"), &[FRAGILE_KIND, "list"], &dials)
        .await;
    fixture.engine.press_key(0).await.unwrap();
    fixture
        .configure(modules("slow"), &[FRAGILE_KIND, "list"], &dials)
        .await;
    assert_eq!(fixture.page().id, "fragile/p");
    let started = Instant::now();
    fixture.engine.press_key(0).await.unwrap();
    assert!(started.elapsed() < SLOW_HANDLER);
    assert_eq!(fixture.key_mark(0), Some(Feedback::Alert));
}

#[tokio::test(start_paused = true)]
async fn events_sent_during_a_slow_handler_are_handled_after_it() {
    let fixture = start(no_dial_info()).await;
    fixture
        .configure(vec![fragile("slow"), list(0)], &[FRAGILE_KIND, "list"], &[])
        .await;
    let events = fixture.probe.events.lock().unwrap().clone().unwrap();
    tokio::spawn(async move {
        tokio::time::sleep(HANDLER_BUDGET / 2).await;
        assert!(events.send(Bump));
    });
    fixture.engine.press_key(0).await.unwrap();
    fixture
        .wait_for(|fixture| fixture.labels()[1].as_deref() == Some("list 10"))
        .await;
}

// ---------------------------------------------------------------- Brightness

#[tokio::test]
async fn a_refused_brightness_is_tried_again_with_the_next_configuration() {
    let fixture = start(no_dial_info()).await;
    fixture.deck.fail_brightness.store(true, Ordering::SeqCst);
    let mut config = fixture.with(vec![list(0)], &["list"], &[]);
    config.brightness = 30;
    let saved = fixture.engine.apply_config(config).await.unwrap();
    // Saved and applied, except for the brightness.
    assert_eq!(saved.brightness, 30);
    assert_eq!(fixture.config(), saved);
    assert_eq!(fixture.labels()[0].as_deref(), Some("list 0"));
    assert_eq!(
        fixture.store.load(fixture.deck.info()).unwrap().config,
        saved
    );
    assert_ne!(fixture.deck.inner.screen().brightness, 30);

    // Same brightness, device back: applied this time.
    fixture.deck.fail_brightness.store(false, Ordering::SeqCst);
    fixture.engine.apply_config(fixture.config()).await.unwrap();
    assert_eq!(fixture.deck.inner.screen().brightness, 30);
}

#[tokio::test]
async fn an_engine_starts_even_if_the_brightness_fails() {
    let fixture = start_with(no_dial_info(), true).await;
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
    fixture.deck.fail_brightness.store(false, Ordering::SeqCst);
    let brightness = fixture.config().brightness;
    fixture.engine.apply_config(fixture.config()).await.unwrap();
    assert_eq!(fixture.deck.inner.screen().brightness, brightness);
}
