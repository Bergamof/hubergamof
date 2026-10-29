//! Pages, navigation, configuration and drawing.

use bergadeck_module_api::KeyState;
use bergadeck_protocol::ErrorCode;
use image::RgbaImage;
use serde_json::json;

use super::support::*;
use crate::device::virtual_deck::{DEMO_SERIAL, virtual_plus_info};
use crate::device::{DeckBackend, InputEvent, StripLayout};
use crate::engine::*;
use crate::render::ImageKind;

#[tokio::test]
async fn default_configuration_shows_placeholders_and_dial_zones() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    let live = fixture.live();
    assert_eq!(live.page.id, HOME_PAGE_ID);
    assert_eq!(live.page.title, HOME_PAGE_TITLE);
    assert_eq!(
        fixture.labels(),
        labels(&[
            Some("audio indisponible"),
            Some("pomodoro indisponible"),
            Some("docker indisponible"),
            Some("intellij indisponible"),
            Some("claude indisponible"),
            None,
            None,
            None,
        ])
    );
    assert!(live.keys[5].image.is_none());
    let modules: Vec<_> = live
        .dials
        .iter()
        .map(|dial| dial.module_id.clone().unwrap().0)
        .collect();
    assert_eq!(modules, ["clock", "audio", "pomodoro", "navigation"]);
    let clock = fixture.zone_lines(0);
    assert_eq!(clock.len(), 2);
    assert!(clock[0].contains(':'));
    // A placeholder has no zone: black, without image.
    assert_eq!(fixture.zone_lines(1), Vec::<String>::new());
    assert!(live.dials[1].zone.as_ref().unwrap().image.is_none());
    assert_eq!(fixture.zone_lines(3), ["Accueil"]);
}

#[tokio::test]
async fn overflowing_pages_scroll_by_rows_within_bounds() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(
            vec![list(11), instance("nav", "navigation", json!({}))],
            &["list"],
            &[Some("nav"), None, None, None],
        )
        .await;

    fixture.engine.press_key(0).await.unwrap();
    let page = fixture.page();
    assert_eq!(page.id, "list/list");
    assert_eq!(page.title, "list (list)");
    assert_eq!((page.row, page.row_count), (0, 3));
    assert_eq!(fixture.labels()[7].as_deref(), Some("item 7"));
    assert_eq!(fixture.zone_lines(0), ["list (list)", "ligne 1/3"]);

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
    assert_eq!(fixture.zone_lines(0), ["list (list)", "ligne 2/3"]);
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

    assert!(fixture.live().dials.is_empty());
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
            vec![list(4), instance("nav", "navigation", json!({}))],
            &["list"],
            &[None, None, None, Some("nav")],
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
    assert_eq!(fixture.zone_lines(3), ["Accueil"]);
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
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
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
async fn dial_zones_follow_their_modules() {
    let fixture = start(virtual_plus_info(DEMO_SERIAL)).await;
    fixture
        .configure(
            vec![instance("nav", "navigation", json!({}))],
            &[],
            &[Some("nav"), None, None, None],
        )
        .await;
    let strip = fixture.deck.inner.screen().strip.unwrap();
    let layout = StripLayout::for_device(fixture.deck.info()).unwrap();
    let zone_width = layout.zone_size().width;
    let lit = |zone: u32| {
        (zone * zone_width..(zone + 1) * zone_width)
            .any(|x| (0..strip.height()).any(|y| strip.get_pixel(x, y).0[0] > 128))
    };
    assert!(lit(0));
    assert!(!lit(1));
    let live = fixture.live();
    assert_eq!(live.dials[0].module_id, Some("nav".into()));
    assert_eq!(live.dials[1].module_id, None);
    assert_eq!(fixture.zone_lines(1), Vec::<String>::new());
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
