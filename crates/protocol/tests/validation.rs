//! Validation rules of `DeviceConfig`.

#![cfg(test)]

use std::path::Path;

use hubergamof_protocol::{
    CURRENT_SCHEMA_VERSION, ConfigError, DeviceConfig, DeviceModel, MAX_BRIGHTNESS, MAX_DIALS,
    MAX_HOME_KEYS, MAX_IDENTIFIER_LENGTH, MAX_IMAGE_NAME_LENGTH, MAX_LABEL_CHARS, MAX_MODULES,
    MAX_SETTINGS_BYTES, ModuleInstance, ModuleInstanceId, is_identifier, is_safe_image_name,
};
use serde_json::json;

fn module(id: &str) -> ModuleInstance {
    ModuleInstance {
        id: id.into(),
        kind: "clock".into(),
        label: None,
        image: None,
        settings: json!({}),
    }
}

fn valid_config() -> DeviceConfig {
    DeviceConfig {
        schema_version: CURRENT_SCHEMA_VERSION,
        serial: "ABC123".into(),
        model: DeviceModel::Plus,
        brightness: MAX_BRIGHTNESS,
        home: vec!["clock".into(), "volume".into()],
        modules: vec![module("clock"), module("volume")],
        dials: vec![Some("volume".into()), None],
        revision: 0,
    }
}

fn with_image(image: &str) -> DeviceConfig {
    let mut config = valid_config();
    config.modules[0].image = Some(image.into());
    config
}

#[test]
fn valid_config_passes() {
    assert_eq!(valid_config().validate(), Ok(()));
    assert_eq!(with_image("icon.v2.png").validate(), Ok(()));
}

#[test]
fn unsupported_schema_version_is_rejected() {
    let mut config = valid_config();
    config.schema_version = CURRENT_SCHEMA_VERSION + 1;
    assert_eq!(
        config.validate(),
        Err(ConfigError::UnsupportedSchemaVersion {
            found: CURRENT_SCHEMA_VERSION + 1
        })
    );
}

#[test]
fn brightness_above_maximum_is_rejected() {
    let mut config = valid_config();
    config.brightness = MAX_BRIGHTNESS + 1;
    assert_eq!(
        config.validate(),
        Err(ConfigError::BrightnessOutOfRange(MAX_BRIGHTNESS + 1))
    );
}

#[test]
fn duplicate_module_id_is_rejected() {
    let mut config = valid_config();
    config.modules.push(module("clock"));
    assert_eq!(
        config.validate(),
        Err(ConfigError::DuplicateModuleId("clock".into()))
    );
}

#[test]
fn unknown_home_module_is_rejected() {
    let mut config = valid_config();
    config.home.push("ghost".into());
    assert_eq!(
        config.validate(),
        Err(ConfigError::UnknownHomeModule("ghost".into()))
    );
}

#[test]
fn unknown_dial_module_is_rejected() {
    let mut config = valid_config();
    config.dials.push(Some("ghost".into()));
    assert_eq!(
        config.validate(),
        Err(ConfigError::UnknownDialModule {
            dial: 2,
            module: ModuleInstanceId::from("ghost"),
        })
    );
}

#[test]
fn unsafe_image_names_are_rejected() {
    for image in [
        "",
        "../secret.png",
        "..",
        "sub/icon.png",
        "/etc/passwd",
        "a\\b.png",
        ".hidden.png",
        "icône.png",
        "with space.png",
    ] {
        assert_eq!(
            with_image(image).validate(),
            Err(ConfigError::InvalidImageName {
                module: "clock".into(),
                image: image.into(),
            }),
            "image name {image:?} should be rejected"
        );
    }
}

#[test]
fn plain_image_names_are_accepted() {
    let longest = format!("{}.png", "a".repeat(MAX_IMAGE_NAME_LENGTH - 4));
    for image in ["icon.png", "0123abcd-ef_9.jpg", longest.as_str()] {
        assert!(is_safe_image_name(image), "{image:?} should be accepted");
        assert_eq!(with_image(image).validate(), Ok(()));
    }
    assert!(!is_safe_image_name(&format!("a{longest}")));
}

#[test]
fn missing_settings_default_to_empty_object() {
    let instance: ModuleInstance = serde_json::from_value(json!({
        "id": "clock",
        "kind": "clock",
        "label": null,
        "image": null
    }))
    .unwrap();
    assert_eq!(instance.settings, json!({}));
}

#[test]
fn example_configurations_load_and_validate() {
    let example_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/example");
    let mut checked = 0;
    for entry in std::fs::read_dir(&example_dir).unwrap() {
        let device_dir = entry.unwrap().path();
        let serial = device_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let raw = std::fs::read_to_string(device_dir.join("config.json")).unwrap();
        let config: DeviceConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(config.serial, serial, "directory name must be the serial");
        assert_eq!(config.validate(), Ok(()), "{serial}");
        checked += 1;
    }
    assert!(checked > 0, "no example configuration found");
}

#[test]
fn oversized_configurations_are_rejected() {
    let mut config = valid_config();
    config.modules = (0..=MAX_MODULES)
        .map(|index| module(&format!("m{index}")))
        .collect();
    assert_eq!(
        config.validate(),
        Err(ConfigError::TooMany {
            what: "module instances",
            count: MAX_MODULES + 1,
            max: MAX_MODULES
        })
    );

    let mut config = valid_config();
    config.home = vec!["clock".into(); MAX_HOME_KEYS + 1];
    assert!(matches!(
        config.validate(),
        Err(ConfigError::TooMany {
            what: "home keys",
            ..
        })
    ));

    let mut config = valid_config();
    config.dials = vec![None; MAX_DIALS + 1];
    assert!(matches!(
        config.validate(),
        Err(ConfigError::TooMany { what: "dials", .. })
    ));
}

#[test]
fn identifiers_are_plain_and_bounded() {
    for valid in [
        "clock",
        "docker-2",
        "my_module",
        &"a".repeat(MAX_IDENTIFIER_LENGTH),
    ] {
        assert!(is_identifier(valid), "{valid}");
    }
    for invalid in [
        "",
        "a b",
        "line\nbreak",
        "é",
        "../x",
        &"a".repeat(MAX_IDENTIFIER_LENGTH + 1),
    ] {
        assert!(!is_identifier(invalid), "{invalid:?}");
    }
    let mut config = valid_config();
    config.modules[0].id = "evil\nINFO forged".into();
    config.home.clear();
    config.dials.clear();
    let error = config.validate().unwrap_err();
    assert_eq!(
        error,
        ConfigError::InvalidIdentifier("evil\nINFO forged".into())
    );
    assert!(!error.to_string().contains('\n'), "escaped: {error}");

    let mut config = valid_config();
    config.modules[0].kind = "bad kind".into();
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidIdentifier(_))
    ));
}

#[test]
fn labels_and_settings_are_bounded() {
    let mut config = valid_config();
    config.modules[0].label = Some("é".repeat(MAX_LABEL_CHARS));
    config.validate().unwrap();
    config.modules[0].label = Some("é".repeat(MAX_LABEL_CHARS + 1));
    assert_eq!(
        config.validate(),
        Err(ConfigError::LabelTooLong("clock".into()))
    );

    let mut config = valid_config();
    config.modules[0].settings = serde_json::json!({ "text": "x".repeat(MAX_SETTINGS_BYTES) });
    assert_eq!(
        config.validate(),
        Err(ConfigError::SettingsTooLarge("clock".into()))
    );
}
