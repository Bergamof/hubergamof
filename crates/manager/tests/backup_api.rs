//! Backup routes (`/api/backup/…`): export, preview, import by sections, and
//! the snapshots that undo an import.

#![cfg(test)]

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use hubergamof_manager::config_store::{
    Backup, BackupDocument, BackupSection, read_backup, write_backup,
};
use hubergamof_manager::file_chooser::SimulatedFilePicker;
use hubergamof_manager::screen_lock::ScreenLock;
use hubergamof_protocol::{
    BackupPreview, DeviceConfig, DeviceModel, DialAssignment, ErrorCode, ImportReport,
    MachineConfig, MouseConfig, MouseModel, SectionChange, SectionResult, ShellAlias, SnapshotInfo,
};
use support::{
    Api, CHOSEN_FILE, FIRST, HOST, NO_DIALS, SECOND, api_error, bytes, png, stored_images,
    use_image_on_first_key, zip_with,
};
use tokio::sync::watch;

const ABSENT_DECK: &str = "ABSENT0000001";
const ABSENT_MOUSE: &str = "MX-GONE-0001";

async fn set_brightness(api: &Api, serial: &str, brightness: u8) {
    let engine = api.engine(serial).await;
    let mut config = api.config(serial).await;
    config.brightness = brightness;
    engine.apply_config(config).await.unwrap();
}

async fn brightness(api: &Api, serial: &str) -> u8 {
    api.config(serial).await.brightness
}

async fn export(api: &Api, query: &str) -> Vec<u8> {
    let response = api.get(&format!("/api/backup/export{query}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/zip");
    assert_eq!(response.headers()["content-disposition"], "attachment");
    bytes(response).await
}

async fn json<T: serde::de::DeserializeOwned>(response: Response<Body>) -> T {
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&bytes(response).await).unwrap()
}

async fn import(api: &Api, query: &str, archive: Vec<u8>) -> Response<Body> {
    api.post(
        &format!("/api/backup/import{query}"),
        "application/zip",
        archive,
    )
    .await
}

async fn snapshots(api: &Api) -> Vec<SnapshotInfo> {
    json(api.get("/api/backup/snapshots").await).await
}

fn outcome<'a>(report: &'a ImportReport, section: &str) -> &'a SectionResult {
    &report
        .sections
        .iter()
        .find(|outcome| outcome.section == section)
        .unwrap_or_else(|| panic!("no outcome for {section}"))
        .outcome
}

/// A backup of `documents`, as an archive.
fn archive_of(documents: Vec<(BackupSection, BackupDocument)>) -> Vec<u8> {
    let documents: BTreeMap<_, _> = documents.into_iter().collect();
    write_backup(&Backup::new("1.0.0", 0, documents, BTreeMap::new())).unwrap()
}

#[tokio::test]
async fn an_export_holds_every_device_and_an_import_puts_it_back() {
    let api = Api::start().await;
    let backup = read_backup(&export(&api, "").await).unwrap();
    for serial in [FIRST, SECOND, NO_DIALS] {
        assert!(
            backup
                .documents
                .contains_key(&BackupSection::Device(serial.to_owned())),
            "{serial}"
        );
    }
    let original = brightness(&api, FIRST).await;
    set_brightness(&api, FIRST, original.wrapping_add(7) % 100).await;
    assert_ne!(brightness(&api, FIRST).await, original);

    let response = import(&api, "", write_backup(&backup).unwrap()).await;
    let report: ImportReport = json(response).await;
    assert_eq!(
        outcome(&report, &format!("devices/{FIRST}")),
        &SectionResult::Applied
    );
    assert_eq!(brightness(&api, FIRST).await, original);
    api.stop().await;
}

#[tokio::test]
async fn an_import_takes_only_the_sections_asked_for() {
    let api = Api::start().await;
    let archive = export(&api, "").await;
    let (first, second) = (
        brightness(&api, FIRST).await,
        brightness(&api, SECOND).await,
    );
    set_brightness(&api, FIRST, first.wrapping_add(11) % 100).await;
    set_brightness(&api, SECOND, second.wrapping_add(11) % 100).await;
    let second_changed = brightness(&api, SECOND).await;

    let report: ImportReport = json(
        import(
            &api,
            &format!("?sections=devices%2F{FIRST}"),
            archive.clone(),
        )
        .await,
    )
    .await;
    assert_eq!(report.sections.len(), 1);
    assert_eq!(brightness(&api, FIRST).await, first);
    assert_eq!(brightness(&api, SECOND).await, second_changed);

    // The same selection, written with a comma list and a plain slash.
    let report: ImportReport = json(
        import(
            &api,
            &format!("?sections=devices/{FIRST},devices/{SECOND}"),
            archive,
        )
        .await,
    )
    .await;
    assert_eq!(report.sections.len(), 2);
    assert_eq!(brightness(&api, SECOND).await, second);
    api.stop().await;
}

#[tokio::test]
async fn an_export_can_be_limited_to_some_sections() {
    let api = Api::start().await;
    let archive = export(&api, &format!("?sections=devices/{FIRST}")).await;
    let backup = read_backup(&archive).unwrap();
    assert_eq!(backup.manifest.sections, vec![format!("devices/{FIRST}")]);
    api.stop().await;
}

#[tokio::test]
async fn a_selection_that_is_not_one_is_refused_and_changes_nothing() {
    let api = Api::start().await;
    let archive = export(&api, "").await;
    for query in [
        "?sections=nonsense",
        "?sections=devices/..%2Fetc",
        "?sections=mice/MISSING-0001",
    ] {
        let response = import(&api, query, archive.clone()).await;
        let (status, code) = api_error(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(code, ErrorCode::InvalidInput, "{query}");
    }
    let response = api.get("/api/backup/export?sections=nonsense").await;
    assert_eq!(api_error(response).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(snapshots(&api).await.len(), 0);
    api.stop().await;
}

#[tokio::test]
async fn a_preview_says_what_would_change_and_writes_nothing() {
    let api = Api::start().await;
    let archive = export(&api, "").await;
    set_brightness(&api, FIRST, 13).await;
    let revisions = (
        api.config(FIRST).await.revision,
        api.config(SECOND).await.revision,
    );

    let response = api
        .post("/api/backup/preview", "application/zip", archive)
        .await;
    let preview: BackupPreview = json(response).await;
    let change = |section: String| {
        preview
            .sections
            .iter()
            .find(|s| s.section == section)
            .unwrap()
    };
    assert_eq!(
        change(format!("devices/{FIRST}")).change,
        SectionChange::Replaced
    );
    assert_eq!(
        change(format!("devices/{SECOND}")).change,
        SectionChange::Same
    );
    assert!(change(format!("devices/{FIRST}")).present);

    assert_eq!(
        (
            api.config(FIRST).await.revision,
            api.config(SECOND).await.revision
        ),
        revisions
    );
    assert_eq!(snapshots(&api).await.len(), 0);
    api.stop().await;
}

#[tokio::test]
async fn a_preview_names_the_commands_and_the_devices_that_are_away() {
    let api = Api::start().await;
    let mut machine = MachineConfig::empty();
    machine.aliases.push(ShellAlias {
        name: "deploy".into(),
        command: "make deploy".into(),
        enabled: false,
    });
    let mut away = api.config(FIRST).await;
    away.serial = ABSENT_DECK.into();
    let archive = archive_of(vec![
        (
            BackupSection::Device(ABSENT_DECK.into()),
            BackupDocument::Device(Box::new(away)),
        ),
        (BackupSection::Machine, BackupDocument::Machine(machine)),
    ]);
    let preview: BackupPreview = json(
        api.post("/api/backup/preview", "application/zip", archive)
            .await,
    )
    .await;
    let deck = preview
        .sections
        .iter()
        .find(|s| s.section == format!("devices/{ABSENT_DECK}"))
        .unwrap();
    assert!(!deck.present);
    assert_eq!(deck.change, SectionChange::Added);
    let machine = preview
        .sections
        .iter()
        .find(|s| s.section == "machine")
        .unwrap();
    assert_eq!(machine.executables.len(), 1);
    assert_eq!(machine.executables[0].name, "deploy");
    assert!(!machine.executables[0].enabled);
    api.stop().await;
}

#[tokio::test]
async fn what_is_not_connected_is_kept_for_its_next_connection() {
    let api = Api::start().await;
    let mut away = api.config(FIRST).await;
    away.serial = ABSENT_DECK.into();
    away.brightness = 42;
    let archive = archive_of(vec![
        (
            BackupSection::Device(ABSENT_DECK.into()),
            BackupDocument::Device(Box::new(away)),
        ),
        (
            BackupSection::Mouse(ABSENT_MOUSE.into()),
            BackupDocument::Mouse(Box::new(MouseConfig::default_for(
                ABSENT_MOUSE,
                MouseModel::MxMaster4,
            ))),
        ),
    ]);
    let report: ImportReport = json(import(&api, "", archive).await).await;
    assert_eq!(
        outcome(&report, &format!("devices/{ABSENT_DECK}")),
        &SectionResult::Stored
    );
    assert_eq!(
        outcome(&report, &format!("mice/{ABSENT_MOUSE}")),
        &SectionResult::Stored
    );
    let dir = |serial: &str| api.context.store.device_dir(serial).unwrap();
    assert!(dir(ABSENT_DECK).join("config.json").is_file());
    assert!(dir(ABSENT_MOUSE).join("mouse.json").is_file());
    api.stop().await;
}

#[tokio::test]
async fn the_machine_is_saved_even_when_it_cannot_be_applied() {
    // The server of the tests has no home folder to put the aliases in.
    let api = Api::start().await;
    let mut machine = MachineConfig::empty();
    machine.aliases.push(ShellAlias {
        name: "gs".into(),
        command: "git status".into(),
        enabled: true,
    });
    let archive = archive_of(vec![(
        BackupSection::Machine,
        BackupDocument::Machine(machine),
    )]);
    let report: ImportReport = json(import(&api, "", archive).await).await;
    assert!(matches!(
        outcome(&report, "machine"),
        SectionResult::NotApplied { .. }
    ));
    let stored = api.context.store.load_machine().unwrap().config;
    assert_eq!(stored.aliases.len(), 1);
    api.stop().await;
}

#[tokio::test]
async fn a_failing_section_does_not_stop_the_others() {
    let api = Api::start().await;
    let mut one = api.config(FIRST).await;
    one.brightness = 33;
    // Modules of an application that is not there: refused, the deck goes on.
    let archive = archive_of(vec![
        (
            BackupSection::Device(FIRST.into()),
            BackupDocument::Device(Box::new(one)),
        ),
        (
            BackupSection::Modules,
            BackupDocument::Modules(hubergamof_protocol::AppModulesConfig::empty()),
        ),
    ]);
    let report: ImportReport = json(import(&api, "", archive).await).await;
    assert_eq!(
        outcome(&report, &format!("devices/{FIRST}")),
        &SectionResult::Applied
    );
    assert!(matches!(
        outcome(&report, "modules"),
        SectionResult::Refused {
            code: ErrorCode::Unavailable,
            ..
        }
    ));
    assert_eq!(brightness(&api, FIRST).await, 33);
    api.stop().await;
}

#[tokio::test]
async fn an_import_is_undone_by_restoring_its_snapshot() {
    let api = Api::start().await;
    let before = export(&api, "").await;
    set_brightness(&api, FIRST, 21).await;

    let report: ImportReport = json(import(&api, "", before).await).await;
    assert_ne!(brightness(&api, FIRST).await, 21);
    let listed = snapshots(&api).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, report.snapshot);
    assert!(listed[0].sections.contains(&format!("devices/{FIRST}")));

    let response = api
        .post(
            &format!("/api/backup/snapshots/{}/restore", report.snapshot),
            "application/json",
            Body::empty(),
        )
        .await;
    let restored: ImportReport = json(response).await;
    assert_eq!(brightness(&api, FIRST).await, 21);
    // The restore took a snapshot too: it can be undone as well.
    assert_ne!(restored.snapshot, report.snapshot);
    assert_eq!(snapshots(&api).await.len(), 2);
    api.stop().await;
}

#[tokio::test]
async fn only_the_latest_three_snapshots_are_kept() {
    let api = Api::start().await;
    let archive = export(&api, "").await;
    for _ in 0..5 {
        json::<ImportReport>(import(&api, "", archive.clone()).await).await;
    }
    assert_eq!(snapshots(&api).await.len(), 3);
    api.stop().await;
}

#[tokio::test]
async fn an_unknown_snapshot_is_refused() {
    let api = Api::start().await;
    for id in ["1700000000000-0", "nonsense", "..%2F..%2Fconfig"] {
        let response = api
            .post(
                &format!("/api/backup/snapshots/{id}/restore"),
                "application/json",
                Body::empty(),
            )
            .await;
        assert!(response.status().is_client_error(), "{id}");
    }
    api.stop().await;
}

#[tokio::test]
async fn an_archive_that_is_not_a_backup_changes_nothing_and_takes_no_snapshot() {
    let api = Api::start().await;
    set_brightness(&api, FIRST, 17).await;
    for body in [b"not a zip".to_vec(), Vec::new()] {
        let (status, code) = api_error(import(&api, "", body.clone()).await).await;
        assert_eq!(
            (status, code),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidArchive)
        );
        let response = api
            .post("/api/backup/preview", "application/zip", body)
            .await;
        assert_eq!(api_error(response).await.0, StatusCode::BAD_REQUEST);
    }
    // Neither is an old export of a device that holds nothing acceptable.
    let before = api.config(FIRST).await;
    let mut needs_image = before.clone();
    needs_image.modules[0].image = Some("absent.png".into());
    let mut invalid = before.clone();
    invalid.brightness = 250;
    let config = |config: &DeviceConfig| serde_json::to_vec(config).unwrap();
    for archive in [
        zip_with(&[("other.txt", b"x")]),
        zip_with(&[("config.json", &config(&invalid))]),
    ] {
        let (status, code) = api_error(import(&api, "", archive).await).await;
        assert_eq!(
            (status, code),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidArchive)
        );
    }
    assert_eq!(api.config(FIRST).await, before);
    assert_eq!(brightness(&api, FIRST).await, 17);
    assert_eq!(snapshots(&api).await.len(), 0);
    // One that uses an image it does not hold reads as a device, and is refused
    // for that section, which is left as it was.
    let archive = zip_with(&[("config.json", &config(&needs_image))]);
    let report: ImportReport = json(import(&api, "", archive).await).await;
    assert!(matches!(
        outcome(&report, &format!("devices/{FIRST}")),
        SectionResult::Refused {
            code: ErrorCode::InvalidArchive,
            ..
        }
    ));
    assert_eq!(api.config(FIRST).await, before);
    api.stop().await;
}

#[tokio::test]
async fn writes_need_the_origin_of_the_interface() {
    let api = Api::start().await;
    for path in [
        "/api/backup/preview".to_owned(),
        "/api/backup/import".to_owned(),
        "/api/backup/snapshots/1700000000000-0/restore".to_owned(),
    ] {
        for origin in [Some("http://evil.example"), None] {
            let mut request = Request::post(&path)
                .header("host", HOST)
                .header("content-type", "application/zip");
            if let Some(origin) = origin {
                request = request.header("origin", origin);
            }
            let response = api.send(request.body(Body::empty()).unwrap()).await;
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{path} {origin:?}"
            );
        }
    }
    assert_eq!(snapshots(&api).await.len(), 0);
    api.stop().await;
}

/// A computer that is locked.
struct Locked;

impl ScreenLock for Locked {
    fn watch(&self) -> watch::Receiver<bool> {
        watch::Sender::new(true).subscribe()
    }
}

#[tokio::test]
async fn nothing_is_backed_up_or_restored_while_the_computer_is_locked() {
    let api = Api::start_on(
        Arc::new(SimulatedFilePicker::choosing(Some(CHOSEN_FILE.into()))),
        Arc::new(Locked),
    )
    .await;
    let locked = |response: Response<Body>| async move {
        assert_eq!(
            api_error(response).await,
            (StatusCode::LOCKED, ErrorCode::Locked)
        );
    };
    locked(api.get("/api/backup/export").await).await;
    locked(api.get("/api/backup/snapshots").await).await;
    locked(
        api.get("/api/backup/snapshots/1700000000000-0/preview")
            .await,
    )
    .await;
    for path in [
        "/api/backup/preview",
        "/api/backup/import",
        "/api/backup/snapshots/1700000000000-0/restore",
    ] {
        locked(api.post(path, "application/zip", Body::empty()).await).await;
    }
    api.stop().await;
}

#[tokio::test]
async fn an_archive_over_the_limit_is_refused() {
    let api = Api::start().await;
    let big = vec![0_u8; hubergamof_manager::config_store::MAX_ARCHIVE_BYTES + 1];
    let response = import(&api, "", big).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    api.stop().await;
}

#[tokio::test]
async fn an_empty_selection_is_refused_and_takes_no_snapshot() {
    let api = Api::start().await;
    let archive = export(&api, "").await;
    for query in ["?sections=", "?sections=,,"] {
        let (status, code) = api_error(import(&api, query, archive.clone()).await).await;
        assert_eq!(
            (status, code),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidInput),
            "{query}"
        );
        let response = api.get(&format!("/api/backup/export{query}")).await;
        assert_eq!(
            api_error(response).await.0,
            StatusCode::BAD_REQUEST,
            "{query}"
        );
    }
    assert_eq!(snapshots(&api).await.len(), 0);
    api.stop().await;
}

#[tokio::test]
async fn a_restore_can_be_previewed_before_it_is_made() {
    let api = Api::start().await;
    let archive = export(&api, "").await;
    let report: ImportReport = json(import(&api, "", archive).await).await;
    set_brightness(&api, FIRST, 9).await;
    let preview: BackupPreview = json(
        api.get(&format!(
            "/api/backup/snapshots/{}/preview",
            report.snapshot
        ))
        .await,
    )
    .await;
    let first = preview
        .sections
        .iter()
        .find(|s| s.section == format!("devices/{FIRST}"))
        .unwrap();
    assert_eq!(first.change, SectionChange::Replaced);
    // Nothing was written, and no snapshot was taken for a preview.
    assert_eq!(brightness(&api, FIRST).await, 9);
    assert_eq!(snapshots(&api).await.len(), 1);
    let response = api
        .get("/api/backup/snapshots/1700000000000-0/preview")
        .await;
    assert_eq!(api_error(response).await.0, StatusCode::BAD_REQUEST);
    api.stop().await;
}

/// The old export of a device, as an earlier version made it.
async fn old_export(api: &Api, serial: &str) -> Vec<u8> {
    api.context
        .store
        .export(serial, &api.config(serial).await)
        .unwrap()
}

#[tokio::test]
async fn the_old_export_of_a_device_goes_to_another_one_with_its_images() {
    let api = Api::start().await;
    let name = {
        let response = api.upload(FIRST, "image/png", png(8, 8)).await;
        let uploaded: hubergamof_protocol::ImageUploaded = json(response).await;
        uploaded.name
    };
    let exported = use_image_on_first_key(&api, FIRST, &name).await;
    let archive = old_export(&api, FIRST).await;

    let before = api.config(SECOND).await;
    let query = format!("?map={FIRST}:{SECOND}");
    let report: ImportReport = json(import(&api, &query, archive).await).await;
    assert_eq!(
        outcome(&report, &format!("devices/{FIRST}")),
        &SectionResult::Applied
    );
    let imported = api.config(SECOND).await;
    assert_eq!(imported.serial, SECOND);
    assert_eq!(imported.revision, before.revision + 1);
    assert_eq!(imported.home, exported.home);
    assert_eq!(imported.modules, exported.modules);
    let image = api
        .get(&format!("/api/devices/{SECOND}/images/{name}"))
        .await;
    assert_eq!(image.status(), StatusCode::OK, "image copied to the device");
    api.stop().await;
}

#[tokio::test]
async fn a_device_of_a_backup_goes_to_another_one_and_leaves_the_rest() {
    let api = Api::start().await;
    set_brightness(&api, FIRST, 31).await;
    let archive = export(&api, "").await;
    set_brightness(&api, FIRST, 77).await;
    set_brightness(&api, SECOND, 55).await;

    let query = format!("?sections=devices/{FIRST}&map={FIRST}:{SECOND}");
    let report: ImportReport = json(import(&api, &query, archive).await).await;
    assert_eq!(report.sections.len(), 1);
    assert_eq!(brightness(&api, SECOND).await, 31);
    assert_eq!(brightness(&api, FIRST).await, 77, "not the target");
    api.stop().await;
}

#[tokio::test]
async fn an_import_fits_the_target_model_both_ways() {
    let api = Api::start().await;
    let plus = api.config(FIRST).await;
    let query = format!("?map={FIRST}:{NO_DIALS}");
    let report: ImportReport =
        json(import(&api, &query, old_export(&api, FIRST).await).await).await;
    assert_eq!(
        outcome(&report, &format!("devices/{FIRST}")),
        &SectionResult::Applied
    );
    let on_mk2 = api.config(NO_DIALS).await;
    assert_eq!(on_mk2.model, DeviceModel::Mk2);
    assert_eq!(on_mk2.dials, Vec::new());
    assert_eq!(on_mk2.home, plus.home);
    // The pomodoro has no key and the device no dial: it is the only module dropped.
    assert_eq!(on_mk2.modules.len(), plus.modules.len() - 1);

    let query = format!("?map={NO_DIALS}:{SECOND}");
    json::<ImportReport>(import(&api, &query, old_export(&api, NO_DIALS).await).await).await;
    let back = api.config(SECOND).await;
    assert_eq!(back.dials, vec![DialAssignment::None; 4]);
    assert_eq!(back.home, plus.home);
    api.stop().await;
}

/// The default configuration of `serial` with `image` on its first key, as an
/// old export holding `content` under that name.
async fn old_export_with_image(
    api: &Api,
    serial: &str,
    image: &str,
    content: &[u8],
    brightness: u8,
) -> Vec<u8> {
    let mut config = api.config(serial).await;
    config.brightness = brightness;
    let first = config.home[0].clone();
    config
        .modules
        .iter_mut()
        .find(|m| m.id == first)
        .unwrap()
        .image = Some(image.into());
    let json = serde_json::to_vec(&config).unwrap();
    zip_with(&[
        ("config.json", &json),
        (&format!("images/{image}"), content),
    ])
}

#[tokio::test]
async fn refused_images_leave_nothing_behind() {
    let api = Api::start().await;
    // A compose file disguised as SVG: the former loose sniffing took it for one.
    let polyglot = b"# <svg>\nservices:\n  x:\n    image: alpine\n    privileged: true\n";
    for archive in [
        old_export_with_image(&api, FIRST, "a.svg", polyglot, 50).await,
        old_export_with_image(&api, FIRST, "b.png", &png(60_000, 1), 50).await,
    ] {
        let report: ImportReport = json(import(&api, "", archive).await).await;
        assert!(matches!(
            outcome(&report, &format!("devices/{FIRST}")),
            SectionResult::Refused {
                code: ErrorCode::InvalidImage,
                ..
            }
        ));
        assert_eq!(stored_images(&api, FIRST), Vec::<String>::new());
    }
    // A configuration that does not validate never gets as far as the images.
    let archive = old_export_with_image(&api, FIRST, "c.png", &png(4, 4), 250).await;
    let (status, code) = api_error(import(&api, "", archive).await).await;
    assert_eq!(
        (status, code),
        (StatusCode::BAD_REQUEST, ErrorCode::InvalidArchive)
    );
    assert_eq!(stored_images(&api, FIRST), Vec::<String>::new());
    api.stop().await;
}

#[tokio::test]
async fn a_device_map_that_does_not_hold_together_is_refused_and_changes_nothing() {
    let api = Api::start().await;
    let archive = export(&api, "").await;
    let cases = [
        // Not a pair, not serials, a target named twice.
        (format!("?map={FIRST}"), ErrorCode::InvalidInput),
        (format!("?map=..%2Fx:{SECOND}"), ErrorCode::InvalidInput),
        (
            format!("?map={FIRST}:{SECOND},{NO_DIALS}:{SECOND}"),
            ErrorCode::InvalidInput,
        ),
        // A target that is not connected.
        (format!("?map={FIRST}:NOPE"), ErrorCode::UnknownDevice),
        // A source that is not imported.
        (
            format!("?sections=devices/{SECOND}&map={FIRST}:{NO_DIALS}"),
            ErrorCode::InvalidInput,
        ),
        // A device that would get its own configuration and another's.
        (format!("?map={FIRST}:{SECOND}"), ErrorCode::InvalidInput),
    ];
    for (query, code) in cases {
        let (_, found) = api_error(import(&api, &query, archive.clone()).await).await;
        assert_eq!(found, code, "{query}");
    }
    assert_eq!(snapshots(&api).await.len(), 0);
    api.stop().await;
}

#[tokio::test]
async fn every_device_or_every_mouse_can_be_named_at_once() {
    let api = Api::start().await;
    let devices = read_backup(&export(&api, "?sections=devices").await).unwrap();
    assert_eq!(devices.manifest.sections.len(), 3);
    assert!(
        devices
            .manifest
            .sections
            .iter()
            .all(|s| s.starts_with("devices/"))
    );
    let mice = read_backup(&export(&api, "?sections=mice").await).unwrap();
    assert_eq!(mice.manifest.sections.len(), 0);

    let archive = export(&api, "").await;
    let report: ImportReport = json(import(&api, "?sections=devices", archive).await).await;
    assert_eq!(report.sections.len(), 3);
    // A group that names nothing the backup holds is refused.
    let archive = export(&api, "?sections=devices").await;
    let (status, code) = api_error(import(&api, "?sections=mice", archive).await).await;
    assert_eq!(
        (status, code),
        (StatusCode::BAD_REQUEST, ErrorCode::InvalidInput)
    );
    api.stop().await;
}

#[tokio::test]
async fn the_per_device_routes_are_gone() {
    let api = Api::start().await;
    for (path, post) in [
        (format!("/api/devices/{FIRST}/export"), false),
        (format!("/api/devices/{FIRST}/import"), true),
    ] {
        let response = if post {
            api.post(&path, "application/zip", Body::empty()).await
        } else {
            api.get(&path).await
        };
        let content_type = response.headers().get("content-type");
        assert_ne!(
            content_type.map(axum::http::HeaderValue::as_bytes),
            Some("application/zip".as_bytes()),
            "{path}"
        );
        assert!(
            !response.status().is_success(),
            "{path}: {}",
            response.status()
        );
    }
    api.stop().await;
}
