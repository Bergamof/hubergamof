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
    BackupPreview, ErrorCode, ImportReport, MachineConfig, MouseConfig, MouseModel, SectionChange,
    SectionResult, ShellAlias, SnapshotInfo,
};
use support::{Api, CHOSEN_FILE, FIRST, HOST, NO_DIALS, SECOND, api_error, bytes};
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
    // A device export (format 1) is not a backup.
    let device_archive = api.get(&format!("/api/devices/{FIRST}/export")).await;
    let device_archive = bytes(device_archive).await;
    let (status, code) = api_error(import(&api, "", device_archive).await).await;
    assert_eq!(
        (status, code),
        (StatusCode::BAD_REQUEST, ErrorCode::InvalidArchive)
    );
    assert_eq!(brightness(&api, FIRST).await, 17);
    assert_eq!(snapshots(&api).await.len(), 0);
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
