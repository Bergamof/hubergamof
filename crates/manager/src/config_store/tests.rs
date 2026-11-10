use hubergamof_modules::DEFAULT_HOME_MODULES;
use hubergamof_protocol::{
    DeviceCapabilities, DeviceModel, ImageSize, ModuleInstanceId, ModuleKind,
};
use tempfile::TempDir;

use super::*;
use crate::device::virtual_deck::{DEMO_SERIAL, virtual_plus_info};
use crate::testing::builtin_registry;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

fn store() -> (TempDir, ConfigStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(dir.path(), Arc::new(builtin_registry()));
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
fn device_folders_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, store) = store();
    store.load(&plus()).unwrap();
    let folder = config_file(&dir, &plus().serial);
    let mode = fs::metadata(folder.parent().unwrap())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, PRIVATE_DIR_MODE);
}

#[test]
fn folders_of_an_older_version_become_private() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, store) = store();
    store.load(&plus()).unwrap();
    let device = config_file(&dir, &plus().serial)
        .parent()
        .unwrap()
        .to_path_buf();
    let images = device.join(IMAGES_DIR);
    fs::create_dir_all(&images).unwrap();
    for folder in [&device, &images] {
        fs::set_permissions(folder, fs::Permissions::from_mode(0o755)).unwrap();
    }

    store.load(&plus()).unwrap();
    for folder in [&device, &images] {
        let mode = fs::metadata(folder).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, PRIVATE_DIR_MODE, "{}", folder.display());
    }
}

#[test]
fn devices_without_dials_get_no_dial_modules() {
    let (_dir, store) = store();
    let config = store.load(&mk2()).unwrap().config;
    assert_eq!(config.dials.len(), 0);
    assert_eq!(config.modules.len(), DEFAULT_HOME_MODULES.len());
}

#[test]
fn layout_and_settings_come_from_the_defaults() {
    struct Defaults;
    impl ConfigDefaults for Defaults {
        fn default_settings(&self, kind: &ModuleKind) -> serde_json::Value {
            serde_json::json!({ "kind": kind.0 })
        }

        fn home_modules(&self) -> Vec<ModuleKind> {
            vec!["a".into(), "b".into()]
        }

        fn dial_modules(&self) -> Vec<ModuleKind> {
            vec!["b".into(), "c".into()]
        }
    }
    let config = default_config(&plus(), &Defaults);
    assert_eq!(config.home, ids(&["a", "b"]));
    assert_eq!(
        config.dials,
        [Some("b".into()), Some("c".into()), None, None]
    );
    let settings: Vec<_> = config.modules.iter().map(|m| m.settings.clone()).collect();
    assert_eq!(
        settings,
        ["a", "b", "c"].map(|kind| serde_json::json!({ "kind": kind }))
    );
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
    let mut config = default_config(&plus(), &builtin_registry());
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
    let saved = store.save(&plus(), config.clone()).unwrap();
    // Stored as given, revision aside: clients recognise their update by it.
    assert_eq!(
        saved,
        DeviceConfig {
            revision: 1,
            ..config
        }
    );
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

fn png_image() -> Vec<u8> {
    let mut png = PNG_SIGNATURE.to_vec();
    png.extend_from_slice(b"rest");
    png
}

/// The default configuration of the Stream Deck+ with `image` on its first key.
fn with_first_key_image(store: &ConfigStore, image: &str) -> DeviceConfig {
    let mut config = store.load(&plus()).unwrap().config;
    let first = config.home[0].clone();
    let module = config.modules.iter_mut().find(|m| m.id == first).unwrap();
    module.image = Some(image.to_owned());
    config
}

#[test]
fn an_export_holds_the_images_in_use_and_drops_missing_ones() {
    let (_dir, store) = store();
    let name = store
        .store_image(DEMO_SERIAL, &png_image(), ImageKind::Png)
        .unwrap();
    let config = with_first_key_image(&store, &name);
    let archive = read_archive(&store.export(DEMO_SERIAL, &config).unwrap()).unwrap();
    assert_eq!(archive.config, config);
    assert_eq!(archive.images, ArchiveImages::from([(name, png_image())]));

    let dangling = with_first_key_image(&store, "gone.png");
    let archive = read_archive(&store.export(DEMO_SERIAL, &dangling).unwrap()).unwrap();
    assert!(archive.images.is_empty());
    assert!(
        archive
            .config
            .modules
            .iter()
            .all(|module| module.image.is_none())
    );
}

/// Accepts anything sniffed as an image (the renderer check is tested apart).
fn sniffed(bytes: &[u8]) -> Result<ImageKind, String> {
    ImageKind::sniff(bytes).ok_or_else(|| "not an image".to_owned())
}

fn archive_of(config: &DeviceConfig, images: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let images = images
        .iter()
        .map(|(name, bytes)| ((*name).to_owned(), bytes.clone()))
        .collect();
    write_archive(config, &images).unwrap()
}

#[test]
fn imported_images_are_named_by_content_whatever_their_archived_name() {
    let (_dir, store) = store();
    let config = with_first_key_image(&store, "renamed-by-hand.png");
    let archive = archive_of(&config, &[("renamed-by-hand.png", png_image())]);
    let prepared = store
        .prepare_import(&archive, &plus(), config.revision, sniffed)
        .unwrap();
    let name = image_name(&png_image(), ImageKind::Png);
    assert_eq!(prepared.images, vec![(name.clone(), png_image())]);
    assert!(
        prepared
            .config
            .modules
            .iter()
            .any(|m| m.image.as_deref() == Some(name.as_str()))
    );
    assert!(
        !store.image_path(DEMO_SERIAL, &name).unwrap().exists(),
        "nothing written yet"
    );

    assert_eq!(
        store.write_import_images(DEMO_SERIAL, &prepared).unwrap(),
        std::slice::from_ref(&name)
    );
    assert!(store.image_path(DEMO_SERIAL, &name).unwrap().is_file());
    assert_eq!(
        store.write_import_images(DEMO_SERIAL, &prepared).unwrap(),
        Vec::<String>::new(),
        "already there: not created again"
    );
    store.remove_images(DEMO_SERIAL, std::slice::from_ref(&name));
    assert!(!store.image_path(DEMO_SERIAL, &name).unwrap().exists());
}

#[test]
fn imports_need_every_image_they_use_drawable_and_a_valid_config() {
    let (_dir, store) = store();
    let missing = with_first_key_image(&store, "absent.png");
    assert!(matches!(
        store.prepare_import(&archive_of(&missing, &[]), &plus(), 0, sniffed),
        Err(ConfigStoreError::MissingImage(name)) if name == "absent.png"
    ));
    let not_an_image = with_first_key_image(&store, "a.png");
    assert!(matches!(
        store.prepare_import(
            &archive_of(&not_an_image, &[("a.png", b"text".to_vec())]),
            &plus(),
            0,
            sniffed
        ),
        Err(ConfigStoreError::UnreadableImage { .. })
    ));
    let mut invalid = with_first_key_image(&store, "a.png");
    invalid.brightness = 250;
    assert!(matches!(
        store.prepare_import(
            &archive_of(&invalid, &[("a.png", png_image())]),
            &plus(),
            0,
            sniffed
        ),
        Err(ConfigStoreError::Invalid(_))
    ));
}

#[test]
fn images_of_instances_dropped_by_the_fit_are_left_out() {
    let (_dir, store) = store();
    let mut config = store.load(&plus()).unwrap().config;
    let dial_only = config.dials[0].clone().unwrap();
    assert!(!config.home.contains(&dial_only));
    let module = config
        .modules
        .iter_mut()
        .find(|m| m.id == dial_only)
        .unwrap();
    module.image = Some("a.png".into());
    let prepared = store
        .prepare_import(
            &archive_of(&config, &[("a.png", png_image())]),
            &mk2(),
            0,
            sniffed,
        )
        .unwrap();
    assert_eq!(prepared.images, Vec::new());
}

#[test]
fn unused_images_are_removed_when_the_config_is_loaded() {
    let (_dir, store) = store();
    let used = store
        .store_image(DEMO_SERIAL, &png_image(), ImageKind::Png)
        .unwrap();
    let mut other = png_image();
    other.push(0);
    let unused = store
        .store_image(DEMO_SERIAL, &other, ImageKind::Png)
        .unwrap();
    let config = with_first_key_image(&store, &used);
    store.save(&plus(), config).unwrap();

    store.load(&plus()).unwrap();
    assert!(store.image_path(DEMO_SERIAL, &used).unwrap().is_file());
    assert!(!store.image_path(DEMO_SERIAL, &unused).unwrap().exists());
}

#[test]
fn imported_configs_are_fitted_to_the_target_device() {
    let (_dir, store) = store();
    let plus_config = store.load(&plus()).unwrap().config;
    let on_mk2 = fit_to_device(plus_config.clone(), &mk2(), 7);
    assert_eq!(on_mk2.serial, "MK2SERIAL");
    assert_eq!(on_mk2.model, DeviceModel::Mk2);
    assert_eq!(on_mk2.revision, 7);
    assert_eq!(on_mk2.dials, Vec::new());
    assert_eq!(on_mk2.home, plus_config.home);
    let kept: Vec<_> = on_mk2.modules.iter().map(|m| m.id.clone()).collect();
    assert!(
        kept.iter().all(|id| on_mk2.home.contains(id)),
        "dial-only instances dropped: {kept:?}"
    );
    check_config(&on_mk2, &mk2()).unwrap();

    let mk2_config = store.load(&mk2()).unwrap().config;
    let on_plus = fit_to_device(mk2_config, &plus(), 0);
    assert_eq!(on_plus.dials, vec![None; 4]);
    check_config(&on_plus, &plus()).unwrap();
}

#[test]
fn stored_images_are_bounded_in_number() {
    let (_dir, store) = store();
    let images = store.device_dir(DEMO_SERIAL).unwrap().join(IMAGES_DIR);
    fs::create_dir_all(&images).unwrap();
    for index in 0..MAX_STORED_IMAGES {
        fs::write(images.join(format!("{index}.png")), b"").unwrap();
    }
    let mut png = PNG_SIGNATURE.to_vec();
    png.extend_from_slice(b"rest");
    assert!(matches!(
        store.store_image(DEMO_SERIAL, &png, ImageKind::Png),
        Err(ConfigStoreError::TooManyImages)
    ));
    fs::remove_file(images.join("0.png")).unwrap();
    store
        .store_image(DEMO_SERIAL, &png, ImageKind::Png)
        .unwrap();
}

/// The default configuration, the Docker module declaring one compose file
/// with the icon `icon`.
fn with_docker_icon(store: &ConfigStore, icon: &str) -> DeviceConfig {
    let mut config = store.load(&plus()).unwrap().config;
    let docker = config
        .modules
        .iter_mut()
        .find(|module| module.kind.0 == "docker")
        .unwrap();
    docker.settings = serde_json::json!({ "composeFiles": [
        { "path": "/srv/shop/compose.yaml", "name": "Boutique", "icon": icon }
    ] });
    config
}

fn docker_icon(config: &DeviceConfig) -> Option<String> {
    let docker = config
        .modules
        .iter()
        .find(|module| module.kind.0 == "docker")?;
    docker.settings["composeFiles"][0]["icon"]
        .as_str()
        .map(str::to_owned)
}

#[test]
fn icons_named_by_the_settings_are_kept_when_the_config_is_loaded() {
    let (_dir, store) = store();
    let used = store
        .store_image(DEMO_SERIAL, &png_image(), ImageKind::Png)
        .unwrap();
    let mut other = png_image();
    other.push(0);
    let unused = store
        .store_image(DEMO_SERIAL, &other, ImageKind::Png)
        .unwrap();
    store
        .save(&plus(), with_docker_icon(&store, &used))
        .unwrap();

    store.load(&plus()).unwrap();
    assert!(store.image_path(DEMO_SERIAL, &used).unwrap().is_file());
    assert!(!store.image_path(DEMO_SERIAL, &unused).unwrap().exists());
}

#[test]
fn an_export_holds_the_icons_of_the_settings_and_drops_missing_ones() {
    let (_dir, store) = store();
    let name = store
        .store_image(DEMO_SERIAL, &png_image(), ImageKind::Png)
        .unwrap();
    let archive = read_archive(
        &store
            .export(DEMO_SERIAL, &with_docker_icon(&store, &name))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(archive.images.len(), 1);
    assert_eq!(docker_icon(&archive.config), Some(name));

    let dangling = read_archive(
        &store
            .export(DEMO_SERIAL, &with_docker_icon(&store, "gone.png"))
            .unwrap(),
    )
    .unwrap();
    assert!(dangling.images.is_empty());
    assert_eq!(docker_icon(&dangling.config), None);
}

#[test]
fn imported_icons_are_named_by_content_like_key_images() {
    let (_dir, store) = store();
    let config = with_docker_icon(&store, "renamed-by-hand.png");
    let archive = archive_of(&config, &[("renamed-by-hand.png", png_image())]);
    let prepared = store.prepare_import(&archive, &plus(), 0, sniffed).unwrap();
    let name = image_name(&png_image(), ImageKind::Png);
    assert_eq!(prepared.images, vec![(name.clone(), png_image())]);
    assert_eq!(docker_icon(&prepared.config), Some(name));

    assert!(matches!(
        store.prepare_import(
            &archive_of(&config, &[]),
            &plus(),
            0,
            sniffed
        ),
        Err(ConfigStoreError::MissingImage(name)) if name == "renamed-by-hand.png"
    ));
}

#[tokio::test]
async fn instances_read_the_stored_images_of_their_device_only() {
    let (_dir, store) = store();
    let name = store
        .store_image(DEMO_SERIAL, &png_image(), ImageKind::Png)
        .unwrap();
    let library = store.image_library(DEMO_SERIAL);
    assert_eq!(
        library.load(&name).await.unwrap().bytes.as_ref(),
        png_image().as_slice()
    );
    assert!(library.load("absent.png").await.is_none());
    for unsafe_name in ["../config.json", "a/b.png", ".hidden", ""] {
        assert!(library.load(unsafe_name).await.is_none(), "{unsafe_name}");
    }
    assert!(
        store
            .image_library("OTHER0000001")
            .load(&name)
            .await
            .is_none(),
        "images belong to a device"
    );
}

#[tokio::test]
async fn an_icon_larger_than_a_source_image_is_not_read() {
    let (_dir, store) = store();
    let images = store.device_dir(DEMO_SERIAL).unwrap().join(IMAGES_DIR);
    fs::create_dir_all(&images).unwrap();
    fs::write(images.join("huge.png"), vec![0; MAX_SOURCE_BYTES + 1]).unwrap();
    assert!(
        store
            .image_library(DEMO_SERIAL)
            .load("huge.png")
            .await
            .is_none()
    );
}

#[test]
fn settings_saved_with_plain_compose_paths_still_load_export_and_import() {
    let (_dir, store) = store();
    let mut config = store.load(&plus()).unwrap().config;
    let docker = config
        .modules
        .iter_mut()
        .find(|module| module.kind.0 == "docker")
        .unwrap();
    docker.settings = serde_json::json!({ "composeFiles": ["/srv/shop/compose.yaml"] });
    let saved = store.save(&plus(), config).unwrap();

    let loaded = store.load(&plus()).unwrap().config;
    assert_eq!(
        loaded, saved,
        "kept as written, nothing pruned or rewritten"
    );
    let archive = store.export(DEMO_SERIAL, &loaded).unwrap();
    let prepared = store.prepare_import(&archive, &plus(), 0, sniffed).unwrap();
    let docker = prepared
        .config
        .modules
        .iter()
        .find(|module| module.kind.0 == "docker")
        .unwrap();
    assert_eq!(
        docker.settings,
        serde_json::json!({ "composeFiles": ["/srv/shop/compose.yaml"] })
    );
}

#[test]
fn an_archive_naming_a_hostile_icon_is_refused_without_echoing_it_raw() {
    let (_dir, store) = store();
    let config = with_docker_icon(&store, "../x\nINJECTED");
    let archive = archive_of(&config, &[("../x\nINJECTED", png_image())]);
    let error = store
        .prepare_import(&archive, &plus(), 0, sniffed)
        .unwrap_err();
    assert!(!error.to_string().contains('\n'), "{error}");
}

#[test]
fn one_image_used_as_icon_and_as_key_image_is_kept_and_archived_once() {
    let (_dir, store) = store();
    let name = store
        .store_image(DEMO_SERIAL, &png_image(), ImageKind::Png)
        .unwrap();
    let mut config = with_docker_icon(&store, &name);
    let first = config.home[0].clone();
    config
        .modules
        .iter_mut()
        .find(|module| module.id == first)
        .unwrap()
        .image = Some(name.clone());
    store.save(&plus(), config.clone()).unwrap();
    store.load(&plus()).unwrap();
    assert!(store.image_path(DEMO_SERIAL, &name).unwrap().is_file());

    let archive = read_archive(&store.export(DEMO_SERIAL, &config).unwrap()).unwrap();
    assert_eq!(archive.images.len(), 1);
}
