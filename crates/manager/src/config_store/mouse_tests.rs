use hubergamof_protocol::{
    ButtonAction, MouseConfig, MouseControlId, MouseModel, RadialMenuConfig,
};
use tempfile::TempDir;

use super::*;
use crate::testing::builtin_registry;

const SERIAL: &str = "0123abcd";

fn store() -> (TempDir, ConfigStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(dir.path(), Arc::new(builtin_registry()));
    (dir, store)
}

fn mouse_path(dir: &TempDir) -> PathBuf {
    dir.path().join(DEVICES_DIR).join(SERIAL).join(MOUSE_FILE)
}

#[test]
fn a_new_mouse_gets_its_default_configuration_on_disk() {
    let (dir, store) = store();
    let loaded = store.load_mouse(SERIAL, MouseModel::MxMaster4).unwrap();
    assert_eq!(loaded.outcome, LoadOutcome::CreatedDefault);
    assert_eq!(
        loaded.config,
        MouseConfig::default_for(SERIAL, MouseModel::MxMaster4)
    );
    assert!(mouse_path(&dir).is_file());
    let again = store.load_mouse(SERIAL, MouseModel::MxMaster4).unwrap();
    assert_eq!(again.outcome, LoadOutcome::Loaded);
    assert_eq!(again.config, loaded.config);
}

#[test]
fn saving_bumps_the_revision_and_is_read_back() {
    let (_dir, store) = store();
    let mut config = store
        .load_mouse(SERIAL, MouseModel::MxMaster4)
        .unwrap()
        .config;
    config.pointer.dpi = 2400;
    config
        .bindings
        .insert(MouseControlId::Back, ButtonAction::None);
    let saved = store.save_mouse(config).unwrap();
    assert_eq!(saved.revision, 1);
    let loaded = store.load_mouse(SERIAL, MouseModel::MxMaster4).unwrap();
    assert_eq!(loaded.config, saved);
}

#[test]
fn an_outdated_revision_is_refused_and_changes_nothing() {
    let (dir, store) = store();
    let first = store
        .load_mouse(SERIAL, MouseModel::MxMaster4)
        .unwrap()
        .config;
    store.save_mouse(first.clone()).unwrap();
    let before = fs::read(mouse_path(&dir)).unwrap();
    let error = store.save_mouse(first).unwrap_err();
    assert_eq!(fs::read(mouse_path(&dir)).unwrap(), before);
    assert!(matches!(
        error,
        ConfigStoreError::StaleRevision {
            current: 1,
            provided: 0
        }
    ));
    assert_eq!(error.error_code(), ErrorCode::StaleRevision);
}

#[test]
fn an_inconsistent_mouse_configuration_is_refused() {
    let (_dir, store) = store();
    let mut config = store
        .load_mouse(SERIAL, MouseModel::MxMaster4)
        .unwrap()
        .config;
    config.pointer.dpi = 225;
    let error = store.save_mouse(config).unwrap_err();
    assert!(
        matches!(error, ConfigStoreError::InvalidMouse(_)),
        "{error}"
    );
    assert_eq!(error.error_code(), ErrorCode::InvalidConfig);
}

#[test]
fn an_unsafe_serial_never_reaches_the_disk() {
    let (dir, store) = store();
    let error = store
        .load_mouse("../etc", MouseModel::MxMaster4)
        .unwrap_err();
    assert!(matches!(error, ConfigStoreError::InvalidSerial(_)));
    let config = MouseConfig::default_for("../etc", MouseModel::MxMaster4);
    assert!(matches!(
        store.save_mouse(config).unwrap_err(),
        ConfigStoreError::InvalidSerial(_)
    ));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn a_broken_mouse_file_is_set_aside_not_overwritten() {
    let (dir, store) = store();
    let path = mouse_path(&dir);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"{ not json").unwrap();
    let loaded = store.load_mouse(SERIAL, MouseModel::MxMaster4).unwrap();
    let LoadOutcome::ReplacedInvalid { backup } = loaded.outcome else {
        panic!("expected the invalid file to be replaced");
    };
    assert!(
        backup
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("mouse.json.invalid-")
    );
    assert_eq!(fs::read(&backup).unwrap(), b"{ not json");
    assert_eq!(loaded.config.revision, 0);
}

#[test]
fn another_mouses_file_under_this_serial_is_set_aside() {
    let (dir, store) = store();
    let path = mouse_path(&dir);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let foreign = MouseConfig::default_for("someone-else", MouseModel::MxMaster4);
    fs::write(&path, serde_json::to_vec(&foreign).unwrap()).unwrap();
    let loaded = store.load_mouse(SERIAL, MouseModel::MxMaster4).unwrap();
    assert!(matches!(
        loaded.outcome,
        LoadOutcome::ReplacedInvalid { .. }
    ));
    assert_eq!(loaded.config.serial, SERIAL);
}

#[test]
fn a_mouse_file_that_fails_validation_is_set_aside() {
    let (dir, store) = store();
    let path = mouse_path(&dir);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bad = MouseConfig::default_for(SERIAL, MouseModel::MxMaster4);
    bad.pointer.dpi = 1;
    fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
    let loaded = store.load_mouse(SERIAL, MouseModel::MxMaster4).unwrap();
    assert!(matches!(
        loaded.outcome,
        LoadOutcome::ReplacedInvalid { .. }
    ));
    assert_eq!(loaded.config.pointer.dpi, hubergamof_protocol::DEFAULT_DPI);
}

#[test]
fn the_radial_menu_has_a_default_and_round_trips() {
    let (dir, store) = store();
    let loaded = store.load_radial_menu().unwrap();
    assert_eq!(loaded.outcome, LoadOutcome::CreatedDefault);
    assert_eq!(loaded.config, RadialMenuConfig::default_menu());
    assert!(dir.path().join(RADIAL_MENU_FILE).is_file());
    let mut config = loaded.config;
    config.animate = !config.animate;
    let saved = store.save_radial_menu(config).unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(store.load_radial_menu().unwrap().config, saved);
}

#[test]
fn a_stale_or_invalid_radial_menu_is_refused() {
    let (_dir, store) = store();
    let config = store.load_radial_menu().unwrap().config;
    store.save_radial_menu(config.clone()).unwrap();
    assert!(matches!(
        store.save_radial_menu(config.clone()).unwrap_err(),
        ConfigStoreError::StaleRevision { .. }
    ));
    let mut invalid = store.load_radial_menu().unwrap().config;
    invalid.hold_ms = 10;
    assert!(matches!(
        store.save_radial_menu(invalid).unwrap_err(),
        ConfigStoreError::InvalidRadialMenu(_)
    ));
}

#[test]
fn a_broken_radial_menu_file_is_set_aside() {
    let (dir, store) = store();
    fs::write(dir.path().join(RADIAL_MENU_FILE), b"[]").unwrap();
    let loaded = store.load_radial_menu().unwrap();
    assert!(matches!(
        loaded.outcome,
        LoadOutcome::ReplacedInvalid { .. }
    ));
    assert_eq!(loaded.config, RadialMenuConfig::default_menu());
}

#[cfg(unix)]
#[test]
fn the_documents_are_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, store) = store();
    store.load_mouse(SERIAL, MouseModel::MxMaster4).unwrap();
    let mode = fs::metadata(mouse_path(&dir).parent().unwrap())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[test]
fn a_file_of_another_model_is_set_aside() {
    let (dir, store) = store();
    let path = mouse_path(&dir);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let other = MouseConfig::default_for(SERIAL, MouseModel::MxAnywhere3S);
    fs::write(&path, serde_json::to_vec(&other).unwrap()).unwrap();
    let loaded = store.load_mouse(SERIAL, MouseModel::MxMaster4).unwrap();
    assert!(matches!(
        loaded.outcome,
        LoadOutcome::ReplacedInvalid { .. }
    ));
    assert_eq!(loaded.config.model, MouseModel::MxMaster4);
}

#[test]
fn a_file_of_an_unknown_schema_is_set_aside_not_lost() {
    let (dir, store) = store();
    let path = mouse_path(&dir);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    for schema in [0, 2] {
        let mut newer = MouseConfig::default_for(SERIAL, MouseModel::MxMaster4);
        newer.schema_version = schema;
        let bytes = serde_json::to_vec(&newer).unwrap();
        fs::write(&path, &bytes).unwrap();
        let loaded = store.load_mouse(SERIAL, MouseModel::MxMaster4).unwrap();
        let LoadOutcome::ReplacedInvalid { backup } = loaded.outcome else {
            panic!("schema {schema} was accepted");
        };
        assert_eq!(fs::read(backup).unwrap(), bytes);
    }
}

#[test]
fn a_stored_revision_is_read_back_and_guards_the_next_save() {
    let (dir, store) = store();
    let path = mouse_path(&dir);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut stored = MouseConfig::default_for(SERIAL, MouseModel::MxMaster4);
    stored.revision = 5;
    fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    let loaded = store.load_mouse(SERIAL, MouseModel::MxMaster4).unwrap();
    assert_eq!(loaded.config.revision, 5);
    let stale = MouseConfig::default_for(SERIAL, MouseModel::MxMaster4);
    assert!(matches!(
        store.save_mouse(stale).unwrap_err(),
        ConfigStoreError::StaleRevision {
            current: 5,
            provided: 0
        }
    ));
    assert_eq!(store.save_mouse(loaded.config).unwrap().revision, 6);
}

#[cfg(unix)]
#[test]
fn an_existing_open_folder_is_tightened_on_load() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, store) = store();
    let folder = mouse_path(&dir).parent().unwrap().to_owned();
    fs::create_dir_all(&folder).unwrap();
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
    store.load_mouse(SERIAL, MouseModel::MxMaster4).unwrap();
    assert_eq!(
        fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let file = fs::metadata(mouse_path(&dir)).unwrap().permissions().mode();
    assert_eq!(file & 0o077, 0, "the file is private too");
}

#[test]
fn saving_over_a_corrupt_file_fails_and_keeps_it() {
    let (dir, store) = store();
    let config = store
        .load_mouse(SERIAL, MouseModel::MxMaster4)
        .unwrap()
        .config;
    fs::write(mouse_path(&dir), b"{ not json").unwrap();
    assert!(matches!(
        store.save_mouse(config).unwrap_err(),
        ConfigStoreError::Json { .. }
    ));
    assert_eq!(fs::read(mouse_path(&dir)).unwrap(), b"{ not json");
}
