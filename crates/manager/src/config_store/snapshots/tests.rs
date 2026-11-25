use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::sync::Arc;

use super::*;
use crate::config_store::ArchiveImages;
use crate::config_store::BackupSection;
use crate::device::virtual_deck::{DEMO_SERIAL, virtual_plus_info};
use crate::testing::builtin_registry;

fn store() -> (tempfile::TempDir, ConfigStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(dir.path(), Arc::new(builtin_registry()));
    store.load(&virtual_plus_info(DEMO_SERIAL)).unwrap();
    (dir, store)
}

#[test]
fn a_snapshot_holds_the_configuration_and_is_private() {
    let (dir, store) = store();
    let id = store.take_snapshot("1.2.3").unwrap();
    let snapshot = store.read_snapshot(&id).unwrap();
    assert_eq!(snapshot.id, id);
    assert_eq!(snapshot.backup.manifest.app_version, "1.2.3");
    assert!(
        snapshot
            .backup
            .documents
            .contains_key(&BackupSection::Device(DEMO_SERIAL.to_owned()))
    );
    let file = dir.path().join("snapshots").join(format!("{id}.zip"));
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(dir.path().join("snapshots"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn only_the_latest_snapshots_are_kept() {
    let (_dir, store) = store();
    let ids: Vec<String> = (0..MAX_SNAPSHOTS + 2)
        .map(|_| store.take_snapshot("1.0.0").unwrap())
        .collect();
    let listed: Vec<String> = store
        .list_snapshots()
        .unwrap()
        .into_iter()
        .map(|info| info.id)
        .collect();
    assert_eq!(listed.len(), MAX_SNAPSHOTS);
    // The latest first.
    let expected: Vec<String> = ids.iter().rev().take(MAX_SNAPSHOTS).cloned().collect();
    assert_eq!(listed, expected);
}

#[test]
fn a_snapshot_is_not_part_of_a_backup() {
    let (_dir, store) = store();
    store.take_snapshot("1.0.0").unwrap();
    let collected = store.collect_backup("1.0.0", 0).unwrap();
    assert_eq!(collected.skipped.len(), 0);
    assert_eq!(collected.backup.documents.len(), 1);
}

#[test]
fn an_id_that_is_not_one_is_unknown_and_never_a_path() {
    let (_dir, store) = store();
    for id in [
        "",
        "../config",
        "1-2/../../x",
        "1700000000000",
        "a-b",
        "1-2.zip",
        "-1",
        "1-",
        "99999999999999999999999999-1",
        "1\n-2",
    ] {
        assert!(
            matches!(
                store.read_snapshot(id),
                Err(ConfigStoreError::UnknownSnapshot(_))
            ),
            "{id:?}"
        );
    }
}

#[test]
fn an_id_without_a_file_is_unknown() {
    let (_dir, store) = store();
    assert!(matches!(
        store.read_snapshot("1700000000000-0"),
        Err(ConfigStoreError::UnknownSnapshot(_))
    ));
}

#[test]
fn a_link_in_the_place_of_a_snapshot_is_not_followed() {
    let (dir, store) = store();
    let id = store.take_snapshot("1.0.0").unwrap();
    let file = dir.path().join("snapshots").join(format!("{id}.zip"));
    let target = dir.path().join("elsewhere.zip");
    fs::rename(&file, &target).unwrap();
    symlink(&target, &file).unwrap();
    assert!(matches!(
        store.read_snapshot(&id),
        Err(ConfigStoreError::UnknownSnapshot(_))
    ));
    // Listed or not, it does not break the list.
    assert_eq!(store.list_snapshots().unwrap().len(), 0);
}

#[test]
fn what_is_not_a_snapshot_in_the_folder_is_ignored() {
    let (dir, store) = store();
    let id = store.take_snapshot("1.0.0").unwrap();
    fs::write(dir.path().join("snapshots").join("notes.txt"), b"x").unwrap();
    fs::write(dir.path().join("snapshots").join("12.zip"), b"x").unwrap();
    let listed = store.list_snapshots().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
}

#[test]
fn two_snapshots_in_a_row_have_two_ids() {
    let (_dir, store) = store();
    let first = store.take_snapshot("1.0.0").unwrap();
    let second = store.take_snapshot("1.0.0").unwrap();
    assert_ne!(first, second);
}

#[test]
fn a_mouse_replaced_while_away_follows_the_stored_revision() {
    use hubergamof_protocol::{MouseConfig, MouseModel};

    let (_dir, store) = store();
    let mut config = MouseConfig::default_for("MX-AWAY-0001", MouseModel::MxMaster4);
    config.revision = 99;
    let first = store.replace_mouse(config.clone()).unwrap();
    assert_eq!(first.revision, 1);
    // Whatever revision the document comes with.
    config.revision = 0;
    assert_eq!(store.replace_mouse(config).unwrap().revision, 2);
}

#[test]
fn a_device_imported_while_away_is_stored_and_its_images_go_with_it() {
    let (dir, store) = store();
    let mut away = store.load(&virtual_plus_info(DEMO_SERIAL)).unwrap().config;
    "ABSENT-0001".clone_into(&mut away.serial);
    away.revision = 55;
    let stored = store
        .import_absent_device(away.clone(), &ArchiveImages::new(), |_| Err("no".into()))
        .unwrap();
    assert_eq!(stored.revision, 1);
    assert!(dir.path().join("devices/ABSENT-0001/config.json").is_file());
    let again = store
        .import_absent_device(away, &ArchiveImages::new(), |_| Err("no".into()))
        .unwrap();
    assert_eq!(again.revision, 2);
}

#[test]
fn a_device_that_does_not_validate_is_not_stored_while_away() {
    let (dir, store) = store();
    let mut away = store.load(&virtual_plus_info(DEMO_SERIAL)).unwrap().config;
    "ABSENT-0002".clone_into(&mut away.serial);
    away.brightness = 250;
    assert!(
        store
            .import_absent_device(away, &ArchiveImages::new(), |_| Err("no".into()))
            .is_err()
    );
    assert!(!dir.path().join("devices/ABSENT-0002").exists());
}
