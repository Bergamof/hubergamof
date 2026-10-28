use bergadeck_protocol::{
    DeviceCapabilities, DeviceModel, ImageSize, ModuleInstanceId, ModuleKind,
};
use tempfile::TempDir;

use super::*;
use crate::device::virtual_deck::{DEMO_SERIAL, virtual_plus_info};

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

fn store() -> (TempDir, ConfigStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(dir.path(), Arc::new(EmptyDefaultSettings));
    (dir, store)
}

fn plus() -> DeviceInfo {
    virtual_plus_info(DEMO_SERIAL)
}

fn mk2() -> DeviceInfo {
    DeviceInfo {
        serial: "MK2SERIAL".into(),
        model: DeviceModel::Mk2,
        display_name: "Stream Deck MK.2".into(),
        capabilities: DeviceCapabilities {
            key_columns: 5,
            key_rows: 3,
            key_image_size: ImageSize {
                width: 72,
                height: 72,
            },
            dial_count: 0,
            touch_strip: None,
        },
    }
}

fn config_file(dir: &TempDir, serial: &str) -> PathBuf {
    dir.path().join(DEVICES_DIR).join(serial).join(CONFIG_FILE)
}

fn ids(names: &[&str]) -> Vec<ModuleInstanceId> {
    names
        .iter()
        .map(|name| ModuleInstanceId::from(*name))
        .collect()
}

#[test]
fn missing_config_is_created_from_defaults() {
    let (dir, store) = store();
    let loaded = store.load(&plus()).unwrap();
    assert_eq!(loaded.outcome, LoadOutcome::CreatedDefault);
    let config = loaded.config;
    assert_eq!(config.serial, DEMO_SERIAL);
    assert_eq!(config.revision, 0);
    assert_eq!(config.brightness, DEFAULT_BRIGHTNESS);
    assert_eq!(
        config.home,
        ids(&["audio", "pomodoro", "docker", "intellij", "claude"])
    );
    assert_eq!(
        config.dials,
        ids(&["clock", "audio", "pomodoro", "navigation"])
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>()
    );
    let kinds: Vec<&str> = config.modules.iter().map(|m| m.kind.0.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "audio",
            "pomodoro",
            "docker",
            "intellij",
            "claude",
            "clock",
            "navigation"
        ]
    );
    assert!(config.modules.iter().all(|m| m.id.0 == m.kind.0));
    config.validate().unwrap();

    let written: DeviceConfig =
        serde_json::from_slice(&fs::read(config_file(&dir, DEMO_SERIAL)).unwrap()).unwrap();
    assert_eq!(written, config);
    assert_eq!(store.load(&plus()).unwrap().outcome, LoadOutcome::Loaded);
}

#[test]
fn devices_without_dials_get_no_dial_modules() {
    let (_dir, store) = store();
    let config = store.load(&mk2()).unwrap().config;
    assert_eq!(config.dials.len(), 0);
    assert_eq!(config.modules.len(), DEFAULT_HOME_MODULES.len());
}

#[test]
fn default_settings_come_from_the_provider() {
    struct Provider;
    impl DefaultSettingsProvider for Provider {
        fn default_settings(&self, kind: &ModuleKind) -> serde_json::Value {
            serde_json::json!({ "kind": kind.0 })
        }
    }
    let config = default_config(&plus(), &Provider);
    let clock = config.modules.iter().find(|m| m.id.0 == "clock").unwrap();
    assert_eq!(clock.settings, serde_json::json!({ "kind": "clock" }));
}

#[test]
fn corrupt_config_is_renamed_and_replaced() {
    let (dir, store) = store();
    let path = config_file(&dir, DEMO_SERIAL);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"{ not json").unwrap();

    let loaded = store.load(&plus()).unwrap();
    let LoadOutcome::ReplacedInvalid { backup } = loaded.outcome else {
        panic!("expected the invalid file to be replaced");
    };
    assert!(
        backup
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(INVALID_BACKUP_PREFIX)
    );
    assert_eq!(fs::read(&backup).unwrap(), b"{ not json");
    assert_eq!(loaded.config.revision, 0);
    assert!(path.exists());
}

#[test]
fn config_with_wrong_dial_count_is_set_aside() {
    let (dir, store) = store();
    let mut config = default_config(&plus(), &EmptyDefaultSettings);
    config.dials.pop();
    let path = config_file(&dir, DEMO_SERIAL);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();

    let loaded = store.load(&plus()).unwrap();
    assert!(matches!(
        loaded.outcome,
        LoadOutcome::ReplacedInvalid { .. }
    ));
    assert_eq!(loaded.config.dials.len(), 4);
}

#[test]
fn two_invalid_files_get_distinct_backups() {
    let (dir, store) = store();
    let path = config_file(&dir, DEMO_SERIAL);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut backups = Vec::new();
    for _ in 0..2 {
        fs::write(&path, b"[]").unwrap();
        let LoadOutcome::ReplacedInvalid { backup } = store.load(&plus()).unwrap().outcome else {
            panic!("expected a backup");
        };
        backups.push(backup);
    }
    assert_ne!(backups[0], backups[1]);
    assert!(backups.iter().all(|backup| backup.exists()));
}

#[test]
fn save_increments_the_revision() {
    let (dir, store) = store();
    let mut config = store.load(&plus()).unwrap().config;
    config.brightness = 30;
    let saved = store.save(&plus(), config).unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.brightness, 30);
    let text = fs::read_to_string(config_file(&dir, DEMO_SERIAL)).unwrap();
    assert!(
        text.contains("\n  \"brightness\": 30,"),
        "pretty-printed JSON"
    );
    assert_eq!(store.load(&plus()).unwrap().config, saved);
}

#[test]
fn stale_revision_is_rejected() {
    let (_dir, store) = store();
    let config = store.load(&plus()).unwrap().config;
    store.save(&plus(), config.clone()).unwrap();
    let error = store.save(&plus(), config).unwrap_err();
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
fn invalid_update_is_rejected_and_not_written() {
    let (dir, store) = store();
    let mut config = store.load(&plus()).unwrap().config;
    let before = fs::read(config_file(&dir, DEMO_SERIAL)).unwrap();
    config.home.push(ModuleInstanceId::from("ghost"));
    let error = store.save(&plus(), config).unwrap_err();
    assert_eq!(error.error_code(), ErrorCode::InvalidConfig);
    assert_eq!(fs::read(config_file(&dir, DEMO_SERIAL)).unwrap(), before);
}

#[test]
fn config_for_another_device_is_rejected() {
    let (_dir, store) = store();
    let mut config = store.load(&plus()).unwrap().config;
    config.serial = "OTHER".into();
    assert!(matches!(
        store.save(&plus(), config),
        Err(ConfigStoreError::SerialMismatch { .. })
    ));
}

#[test]
fn atomic_write_leaves_no_temporary_file() {
    let (dir, store) = store();
    let config = store.load(&plus()).unwrap().config;
    store.save(&plus(), config).unwrap();
    let entries: Vec<String> = fs::read_dir(dir.path().join(DEVICES_DIR).join(DEMO_SERIAL))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(entries, [CONFIG_FILE]);
}

#[test]
fn unsafe_serials_are_rejected() {
    let (_dir, store) = store();
    for serial in ["", "..", "../etc", "a/b", "a\\b", &"x".repeat(65)] {
        assert!(
            matches!(
                store.device_dir(serial),
                Err(ConfigStoreError::InvalidSerial(_))
            ),
            "{serial}"
        );
    }
    let mut info = plus();
    info.serial = "../escape".into();
    assert!(store.load(&info).is_err());
}

#[test]
fn images_are_stored_by_content() {
    let (_dir, store) = store();
    let mut png = PNG_SIGNATURE.to_vec();
    png.extend_from_slice(b"rest");
    let name = store
        .store_image(DEMO_SERIAL, &png, ImageKind::Png)
        .unwrap();
    assert_eq!(name.len(), IMAGE_DIGEST_LENGTH + ".png".len());
    assert_eq!(Path::new(&name).extension().unwrap(), "png");
    let path = store.image_path(DEMO_SERIAL, &name).unwrap();
    assert_eq!(fs::read(&path).unwrap(), png);
    assert!(path.ends_with(format!("{DEVICES_DIR}/{DEMO_SERIAL}/{IMAGES_DIR}/{name}")));
    assert_eq!(
        store
            .store_image(DEMO_SERIAL, &png, ImageKind::Png)
            .unwrap(),
        name
    );
}

#[test]
fn image_content_must_match_its_kind() {
    let (_dir, store) = store();
    assert!(matches!(
        store.store_image(DEMO_SERIAL, PNG_SIGNATURE, ImageKind::Jpeg),
        Err(ConfigStoreError::ImageKindMismatch { .. })
    ));
    assert!(matches!(
        store.store_image(DEMO_SERIAL, &vec![0; MAX_SOURCE_BYTES + 1], ImageKind::Png),
        Err(ConfigStoreError::ImageTooLarge)
    ));
}

#[test]
fn image_path_traversal_is_rejected() {
    let (_dir, store) = store();
    for name in [
        "../config.json",
        "..",
        ".hidden",
        "a/b.png",
        "/etc/passwd",
        "a\\b",
        "",
    ] {
        assert!(
            matches!(
                store.image_path(DEMO_SERIAL, name),
                Err(ConfigStoreError::InvalidImageName(_))
            ),
            "{name}"
        );
    }
    assert!(store.image_path(DEMO_SERIAL, "logo-1.svg").is_ok());
}
