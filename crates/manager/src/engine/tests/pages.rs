//! Pages, navigation, configuration and drawing.

use hubergamof_module_api::{KeyState, VolumeWidget};
use hubergamof_protocol::ErrorCode;
use image::RgbaImage;
use serde_json::json;

use super::support::*;
use crate::device::virtual_deck::{DEMO_SERIAL, virtual_plus_info};
use crate::device::{DeckBackend, InputEvent};
use crate::engine::*;
use crate::render::ImageKind;
use crate::strip::HintFace;

#[tokio::test]
async fn default_configuration_shows_the_builtin_modules_and_the_strip() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    let live = fixture.live();
    assert_eq!(live.page.id, HOME_PAGE_ID);
    assert_eq!(live.page.title, HOME_PAGE_TITLE);
    // The Docker key counts the containers once the simulated host answers.
    fixture
        .wait_for(|fixture| fixture.labels()[2].as_deref() == Some("Docker 4/7"))
        .await;
    assert_eq!(
        fixture.labels(),
        labels(&[
            Some("Audio"),
            Some("25:00 Travail"),
            Some("Docker 4/7"),
            Some("IntelliJ"),
            Some("Claude"),
            None,
            None,
            None,
        ])
    );
    assert!(live.keys[5].image.is_none());

    // The strip is the manager's: time, measures and page come with it.
    let strip = fixture.strip();
    assert_eq!(strip.clock.time, "14:05");
    assert_eq!(strip.clock.seconds, Some(30));
    assert_eq!(strip.clock.date, "mer. 28 oct.");
    assert_eq!(strip.page.trail, HOME_PAGE_TITLE);
    let measures: Vec<_> = strip
        .measures
        .iter()
        .map(|measure| (measure.label, measure.percent))
        .collect();
    assert_eq!(
        measures,
        [("CPU", Some(22)), ("RAM", Some(61)), ("SWAP", Some(8))]
    );
    assert!(live.strip.unwrap().image.is_some());

    // The pomodoro is shown, ready; the dials remind what they do, in the
    // default order: volume, pomodoro, contextual, navigation.
    let pomodoro = strip.pomodoro.expect("the pomodoro is configured");
    assert_eq!(pomodoro.widget.label, "Focus 1/4 · prêt");
    assert_eq!(pomodoro.widget.time, "25:00");
    assert_eq!(fixture.hint(1).press.as_deref(), Some("Menu pomodoro"));
    assert_eq!(fixture.hint(2).rotate.as_deref(), Some("Contexte"));
    let navigation = fixture.hint(3);
    assert_eq!(navigation.rotate.as_deref(), Some("Navigation"));
    // Nothing to go back to on the home page.
    assert_eq!(navigation.press, None);
    // The (simulated) audio module shows its volume once it read the state.
    fixture
        .wait_for(|f| f.strip().volume == Some(VolumeWidget::new(40, false)))
        .await;
    assert_eq!(fixture.hint(0).rotate.as_deref(), Some("Volume"));
    assert_eq!(fixture.hint(0).press.as_deref(), Some("Muet"));
}

#[tokio::test]
async fn overflowing_pages_scroll_by_rows_within_bounds() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(
            vec![list(11)],
            &["list"],
            &[Some(NAVIGATION_DIAL), None, None, None],
        )
        .await;

    fixture.engine.press_key(0).await.unwrap();
    let page = fixture.page();
    assert_eq!(page.id, "list/list");
    assert_eq!(page.title, "list (list)");
    assert_eq!((page.row, page.row_count), (0, 3));
    assert_eq!(fixture.labels()[7].as_deref(), Some("item 7"));
    let page = fixture.strip().page;
    assert_eq!(page.trail, "Accueil › list (list)");
    // Three rows, two of them visible: two scroll positions.
    assert_eq!((page.row, page.positions), (0, 2));
    assert_eq!(fixture.hint(0).press.as_deref(), Some("Retour"));

    fixture.engine.rotate_dial(0, 5).await.unwrap();
    assert_eq!(fixture.page().row, 1);
    assert_eq!(
        fixture.labels(),
        labels(&[
            Some("item 4"),
            Some("item 5"),
            Some("item 6"),
            Some("item 7"),
            Some("item 8"),
            Some("item 9"),
            Some("item 10"),
            None,
        ])
    );
    assert_eq!(fixture.strip().page.row, 1);
    // Beyond the content: blank.
    assert!(fixture.live().keys[7].image.is_none());
    assert!(
        fixture
            .deck
            .inner
            .key_image(7)
            .unwrap()
            .pixels()
            .all(|p| p.0 == [0, 0, 0, 255])
    );

    fixture.engine.rotate_dial(0, -9).await.unwrap();
    assert_eq!(fixture.page().row, 0);
}

#[tokio::test]
async fn devices_without_dials_scroll_through_module_commands() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(8)], &["list"], &[]).await;
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!((fixture.page().row, fixture.page().row_count), (0, 3));

    // Item i2 scrolls one row; the page then shows items 3..8.
    fixture.engine.press_key(2).await.unwrap();
    assert_eq!(fixture.page().row, 1);
    assert_eq!(
        fixture.labels(),
        labels(&[
            Some("item 3"),
            Some("item 4"),
            Some("item 5"),
            Some("item 6"),
            Some("item 7"),
            None,
        ])
    );
    // A key beyond the content does nothing.
    fixture.engine.press_key(5).await.unwrap();
    assert_eq!(fixture.page().row, 1);

    assert!(fixture.live().strip.is_none());
    let error = fixture.engine.rotate_dial(0, 1).await.unwrap_err();
    assert_eq!(error.error_code(), ErrorCode::InvalidInput);
    let error = fixture.engine.press_key(6).await.unwrap_err();
    assert_eq!(error.error_code(), ErrorCode::InvalidInput);
}

#[tokio::test]
async fn back_and_home_unwind_the_navigation_stack() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(
            vec![list(4)],
            &["list"],
            &[None, None, None, Some(NAVIGATION_DIAL)],
        )
        .await;
    // Back on the home page does nothing.
    fixture.engine.press_dial(3).await.unwrap();
    assert_eq!(fixture.page().id, HOME_PAGE_ID);

    fixture.engine.press_key(0).await.unwrap();
    fixture.engine.press_key(3).await.unwrap();
    assert_eq!(fixture.page().id, "list/nested");
    // Item i0 goes back.
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, "list/list");

    fixture.engine.press_key(3).await.unwrap();
    // Item i1 goes home.
    fixture.engine.press_key(1).await.unwrap();
    assert_eq!(fixture.page().id, HOME_PAGE_ID);

    fixture.engine.press_key(0).await.unwrap();
    fixture.engine.press_dial(3).await.unwrap();
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
    assert_eq!(fixture.strip().page.trail, "Accueil");
}

#[tokio::test]
async fn pages_gone_after_an_event_unwind_to_the_closest_one_left() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(vec![list(4)], &["list"], &[None; 4])
        .await;
    fixture.engine.press_key(0).await.unwrap();
    // Item i3 opens a nested page, twice: list → nested → nested.
    fixture.engine.press_key(3).await.unwrap();
    fixture.engine.press_key(3).await.unwrap();
    assert_eq!(fixture.page().id, "list/nested");

    *fixture.probe.gone_page.lock().unwrap() = Some("nested".into());
    let events = fixture.probe.events.lock().unwrap().clone().unwrap();
    assert!(events.send(Bump));
    fixture
        .wait_for(|fixture| fixture.page().id == "list/list")
        .await;
    // Not home: the list page is still there.
    assert_eq!(fixture.page().title, "list (list)");
}

#[tokio::test]
async fn hardware_inputs_are_handled_like_simulated_ones() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(vec![list(4)], &["list"], &[None; 4])
        .await;
    let mut live = fixture.engine.live();
    live.mark_unchanged();
    fixture.deck.inner.inject(InputEvent::KeyDown(0)).unwrap();
    fixture.deck.inner.inject(InputEvent::KeyUp(0)).unwrap();
    tokio::time::timeout(WAIT, live.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fixture.page().id, "list/list");
}

#[tokio::test]
async fn unknown_kinds_get_a_placeholder_key() {
    let fixture = start(no_dial_info()).await;
    let mut weird = instance("weird", "teleporter", json!({}));
    weird.label = None;
    fixture
        .configure(vec![weird, list(0)], &["weird", "list"], &[])
        .await;
    assert_eq!(
        fixture.labels()[..2],
        labels(&[Some("teleporter indisponible"), Some("list 0")])
    );
    // Pressing it does nothing.
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
}

#[tokio::test]
async fn applying_a_config_keeps_the_state_of_unchanged_instances() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(2)], &["list"], &[]).await;
    fixture.engine.press_key(0).await.unwrap();
    fixture.engine.press_key(1).await.unwrap();
    assert_eq!(fixture.labels()[0].as_deref(), Some("list 1"));
    assert_eq!(fixture.live().keys[0].state, KeyState::Active);

    // Same id and kind: new settings, same press count.
    let renamed = instance("list", LIST_KIND, json!({ "label": "Liste", "items": 2 }));
    let saved = fixture.configure(vec![renamed], &["list"], &[]).await;
    assert_eq!(fixture.labels()[0].as_deref(), Some("Liste 1"));
    assert_eq!(*fixture.engine.config().borrow(), saved);

    // The configuration label wins over the module's.
    let mut labelled = list(2);
    labelled.label = Some("Ma liste".into());
    fixture.configure(vec![labelled], &["list"], &[]).await;
    assert_eq!(fixture.labels()[0].as_deref(), Some("Ma liste"));

    // Another id: a new instance.
    let other = instance("other", LIST_KIND, json!({}));
    fixture.configure(vec![other], &["other"], &[]).await;
    assert_eq!(fixture.labels()[0].as_deref(), Some("list 0"));
}

#[tokio::test]
async fn removing_the_displayed_module_returns_home() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(2)], &["list"], &[]).await;
    fixture.engine.press_key(0).await.unwrap();
    assert_eq!(fixture.page().id, "list/list");
    fixture.configure(Vec::new(), &[], &[]).await;
    assert_eq!(fixture.page().id, HOME_PAGE_ID);
    assert_eq!(fixture.labels(), vec![None; 6]);
}

#[tokio::test]
async fn stale_revisions_are_rejected_without_change() {
    let fixture = start(no_dial_info()).await;
    let current = fixture.configure(vec![list(2)], &["list"], &[]).await;
    let mut stale = current.clone();
    stale.revision -= 1;
    stale.home.clear();
    let error = fixture.engine.apply_config(stale).await.unwrap_err();
    assert_eq!(error.error_code(), ErrorCode::StaleRevision);
    assert_eq!(*fixture.engine.config().borrow(), current);
    assert_eq!(fixture.labels()[0].as_deref(), Some("list 0"));
}

#[tokio::test]
async fn only_changed_keys_are_rewritten() {
    let fixture = start_settled(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(vec![list(0)], &["list"], &[None; 4])
        .await;
    let _ = fixture.deck.writes();

    let mut live = fixture.engine.live();
    live.mark_unchanged();
    *fixture.probe.key_label.lock().unwrap() = Some("changed".into());
    let redraw = fixture.probe.redraw.lock().unwrap().clone().unwrap();
    redraw.request_redraw();
    tokio::time::timeout(WAIT, live.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fixture.deck.writes(), (1, 0));
    assert_eq!(fixture.labels()[0].as_deref(), Some("changed"));

    // Nothing changed: nothing written.
    redraw.request_redraw();
    fixture.engine.press_key(7).await.unwrap();
    assert_eq!(fixture.deck.writes(), (0, 0));
}

#[tokio::test]
async fn the_strip_shows_the_path_of_the_displayed_page() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(vec![list(4)], &["list"], &[None; 4])
        .await;
    assert_eq!(fixture.strip().page.trail, "Accueil");

    fixture.engine.press_key(0).await.unwrap();
    let list_title = fixture.page().title;
    assert_eq!(
        fixture.strip().page.trail,
        format!("Accueil › {list_title}")
    );

    fixture.engine.press_key(3).await.unwrap();
    assert_eq!(fixture.page().id, "list/nested");
    let nested = fixture.page().title;
    assert_eq!(
        fixture.strip().page.trail,
        format!("{list_title} › {nested}")
    );
}

#[tokio::test]
async fn the_strip_is_rewritten_only_when_it_changes() {
    let fixture = start_settled(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(
            vec![list(0), fragile("none")],
            &["list"],
            &[Some(FRAGILE_KIND), None, None, None],
        )
        .await;
    let redraw = fixture.probe.redraw.lock().unwrap().clone().unwrap();
    let show = |percent: u16| {
        *fixture.probe.volume.lock().unwrap() = Some(VolumeWidget::new(percent, false));
        redraw.request_redraw();
    };
    let mut live = fixture.engine.live();
    let mut redrawn = async |expected: u16| {
        live.mark_unchanged();
        tokio::time::timeout(WAIT, live.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            fixture.strip().volume,
            Some(VolumeWidget::new(expected, false))
        );
    };
    let _ = fixture.deck.writes();

    show(25);
    redrawn(25).await;
    assert_eq!(fixture.deck.writes(), (0, 1));

    // The same volume: nothing written.
    show(25);
    fixture.engine.press_key(7).await.unwrap();
    assert_eq!(fixture.deck.writes(), (0, 0));

    // Another volume: the strip is written once.
    show(50);
    redrawn(50).await;
    assert_eq!(fixture.deck.writes(), (0, 1));
}

#[tokio::test]
async fn the_strip_follows_the_clock() {
    let fixture = start_settled(virtual_plus_info(DEMO_SERIAL)).await;
    let redraw = {
        fixture
            .configure(vec![list(0)], &["list"], &[None; 4])
            .await;
        fixture.probe.redraw.lock().unwrap().clone().unwrap()
    };
    let mut live = fixture.engine.live();
    let _ = fixture.deck.writes();
    assert_eq!(fixture.strip().clock.time, "14:05");

    fixture
        .probe
        .clock
        .set(jiff::civil::date(2026, 10, 28).at(14, 6, 1, 0));
    live.mark_unchanged();
    redraw.request_redraw();
    tokio::time::timeout(WAIT, live.changed())
        .await
        .unwrap()
        .unwrap();
    let clock = fixture.strip().clock;
    assert_eq!((clock.time.as_str(), clock.seconds), ("14:06", Some(1)));
    assert_eq!(fixture.deck.writes(), (0, 1));
    // The face is drawn on the strip, not left blank.
    let strip = fixture.deck.inner.screen().strip.unwrap();
    assert!(strip.pixels().any(|pixel| pixel.0[0] < 100));

    // The same time: nothing written.
    redraw.request_redraw();
    fixture.engine.press_key(7).await.unwrap();
    assert_eq!(fixture.deck.writes(), (0, 0));
}

#[tokio::test]
async fn the_dials_remind_what_their_functions_do() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(
            vec![list(0)],
            &["list"],
            &[Some(NAVIGATION_DIAL), None, None, None],
        )
        .await;
    let navigation = fixture.hint(0);
    assert_eq!(navigation.rotate.as_deref(), Some("Navigation"));
    // On the home page there is nothing to go back to.
    assert_eq!(navigation.press, None);
    assert_eq!(fixture.hint(1), HintFace::default());
    assert_eq!(fixture.strip().hints.len(), 4);
}

#[tokio::test]
async fn configured_images_replace_module_images() {
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
    with_image.image = Some(name);
    fixture.configure(vec![with_image], &["list"], &[]).await;
    let key = fixture.deck.inner.key_image(0).unwrap();
    assert_eq!(key.get_pixel(36, 10).0, [255, 0, 0, 255]);
}

#[tokio::test]
async fn live_state_is_published_after_each_visible_change() {
    let fixture = start(no_dial_info()).await;
    fixture.configure(vec![list(1)], &["list"], &[]).await;
    let mut live = fixture.engine.live();
    live.mark_unchanged();

    fixture.engine.press_key(0).await.unwrap();
    assert!(live.has_changed().unwrap());
    let state = live.borrow_and_update().to_live_state().unwrap();
    assert_eq!(state.serial, "NODIAL");
    assert_eq!(state.page.title, "list (list)");
    assert_eq!(state.page.keys.len(), 6);
    assert!(
        state.page.keys[0]
            .image
            .as_deref()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
    assert_eq!(state.page.keys[1].image, None);

    // An input changing nothing publishes nothing.
    fixture.engine.press_key(4).await.unwrap();
    assert!(!live.has_changed().unwrap());
}

#[tokio::test]
async fn disconnecting_the_device_stops_the_engine() {
    let fixture = start(no_dial_info()).await;
    fixture.deck.close().await;
    tokio::time::timeout(WAIT, fixture.task)
        .await
        .unwrap()
        .unwrap();
    let error = fixture.engine.press_key(0).await.unwrap_err();
    assert_eq!(error.error_code(), ErrorCode::UnknownDevice);
}

#[tokio::test]
async fn stopping_the_engine_blanks_the_device() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    assert!(fixture.deck.inner.key_image(0).is_some());
    fixture.engine.stop().await;
    tokio::time::timeout(WAIT, fixture.task)
        .await
        .unwrap()
        .unwrap();
    assert!(fixture.deck.inner.screen().keys.iter().all(Option::is_none));
    // Stopping again returns at once.
    fixture.engine.stop().await;
}

#[tokio::test]
async fn the_whole_strip_is_written_again_once_the_device_settled() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    // A device just connected may lose the first writes.
    fixture.deck.inner.clear_all().await.unwrap();
    let last_zone_pixel = || {
        let screen = fixture.deck.inner.screen();
        *screen.strip.unwrap().get_pixel(700, 50)
    };
    assert_eq!(last_zone_pixel().0[3], 0, "the strip was blanked");
    tokio::time::timeout(WAIT, async {
        while last_zone_pixel().0[3] == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the whole strip is written again");
}
