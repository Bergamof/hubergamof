//! Upgrade of configurations written by earlier versions.
//!
//! Version 1 assigned a module instance to each dial and had a clock and a
//! navigation module. Version 2 assigns functions, offered by modules or by
//! the manager, and shows time and navigation on the touch strip itself.

use hubergamof_modules::audio::{AUDIO_KIND, VOLUME_FUNCTION};
use hubergamof_modules::pomodoro::{CONTROL_FUNCTION, POMODORO_KIND};
use std::collections::{HashMap, HashSet};

use hubergamof_protocol::{
    CURRENT_SCHEMA_VERSION, DeviceConfig, DialAssignment, MAX_DIALS, MAX_HOME_KEYS, MAX_MODULES,
};
use serde::de::Error as _;
use serde_json::{Map, Value, json};

/// Kind of the clock module of version 1, now part of the touch strip.
const LEGACY_CLOCK_KIND: &str = "clock";
/// Kind of the navigation module of version 1, now a dial function.
const LEGACY_NAVIGATION_KIND: &str = "navigation";

/// Version of the first configurations.
const FIRST_SCHEMA_VERSION: u64 = 1;

/// Parses a stored or imported configuration, upgrading it to the current
/// schema version first when it is an older one.
///
/// # Errors
///
/// The JSON is invalid, or does not describe a [`DeviceConfig`].
pub(super) fn parse_config(bytes: &[u8]) -> Result<DeviceConfig, serde_json::Error> {
    let mut value: Value = serde_json::from_slice(bytes)?;
    if schema_version(&value) == Some(FIRST_SCHEMA_VERSION) {
        // Before anything walks them: validation comes after the upgrade.
        check_sizes(&value)?;
        upgrade_from_first(&mut value);
    }
    serde_json::from_value(value)
}

/// Refuses a configuration of the first version whose lists are longer than
/// a configuration may hold: the upgrade would walk them for nothing.
fn check_sizes(config: &Value) -> Result<(), serde_json::Error> {
    for (key, max) in [
        ("modules", MAX_MODULES),
        ("home", MAX_HOME_KEYS),
        ("dials", MAX_DIALS),
    ] {
        let length = config
            .get(key)
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        if length > max {
            return Err(serde_json::Error::custom(format!(
                "too many `{key}`: {length} (at most {max})"
            )));
        }
    }
    Ok(())
}

fn schema_version(config: &Value) -> Option<u64> {
    config.get("schemaVersion").and_then(Value::as_u64)
}

fn upgrade_from_first(config: &mut Value) {
    let Some(config) = config.as_object_mut() else {
        return;
    };
    let kinds = instance_kinds(config);
    let dials = config.get("dials").and_then(Value::as_array).map(|dials| {
        dials
            .iter()
            .map(|dial| upgraded_dial(dial, &kinds))
            .collect()
    });
    drop_legacy_instances(config);
    if let Some(dials) = dials {
        config.insert("dials".into(), Value::Array(dials));
    }
    config.insert("schemaVersion".into(), json!(CURRENT_SCHEMA_VERSION));
}

/// Kind of every module instance, by id.
fn instance_kinds(config: &Map<String, Value>) -> HashMap<String, String> {
    let text =
        |instance: &Value, key: &str| instance.get(key).and_then(Value::as_str).map(str::to_owned);
    config
        .get("modules")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|instance| Some((text(instance, "id")?, text(instance, "kind")?)))
        .collect()
}

/// What a dial of version 1 (a module instance id, or `null`) does now.
fn upgraded_dial(dial: &Value, kinds: &HashMap<String, String>) -> Value {
    let assignment = dial.as_str().and_then(|id| kinds.get_key_value(id)).map_or(
        DialAssignment::None,
        |(id, kind)| match kind.as_str() {
            AUDIO_KIND => module_function(id, VOLUME_FUNCTION),
            POMODORO_KIND => module_function(id, CONTROL_FUNCTION),
            LEGACY_NAVIGATION_KIND => DialAssignment::Navigation,
            _ => DialAssignment::None,
        },
    );
    serde_json::to_value(assignment).unwrap_or(Value::Null)
}

fn module_function(instance: &str, function: &str) -> DialAssignment {
    DialAssignment::Module {
        instance: instance.into(),
        function: function.into(),
    }
}

/// Removes the instances of the clock and navigation modules, and their keys.
fn drop_legacy_instances(config: &mut Map<String, Value>) {
    let legacy: HashSet<String> = instance_kinds(config)
        .into_iter()
        .filter(|(_, kind)| kind == LEGACY_CLOCK_KIND || kind == LEGACY_NAVIGATION_KIND)
        .map(|(id, _)| id)
        .collect();
    if let Some(modules) = config.get_mut("modules").and_then(Value::as_array_mut) {
        modules.retain(|instance| {
            let id = instance.get("id").and_then(Value::as_str);
            !id.is_some_and(|id| legacy.contains(id))
        });
    }
    if let Some(home) = config.get_mut("home").and_then(Value::as_array_mut) {
        home.retain(|id| {
            !id.as_str()
                .is_some_and(|id| legacy.iter().any(|legacy| legacy == id))
        });
    }
}

#[cfg(test)]
mod tests {
    use hubergamof_protocol::StripConfig;

    use super::*;

    const FIRST_CONFIG: &str = r#"{
        "schemaVersion": 1,
        "serial": "ABC",
        "model": "plus",
        "brightness": 50,
        "home": ["pomodoro", "clock", "docker"],
        "modules": [
            { "id": "clock", "kind": "clock", "label": null, "image": null, "settings": {} },
            { "id": "pomodoro", "kind": "pomodoro", "label": null, "image": null, "settings": {} },
            { "id": "docker", "kind": "docker", "label": null, "image": null, "settings": {} },
            { "id": "volume", "kind": "audio", "label": "Volume", "image": null, "settings": {} },
            { "id": "nav", "kind": "navigation", "label": null, "image": null, "settings": {} }
        ],
        "dials": ["volume", "pomodoro", "clock", "nav"],
        "revision": 9
    }"#;

    fn upgraded() -> DeviceConfig {
        parse_config(FIRST_CONFIG.as_bytes()).unwrap()
    }

    #[test]
    fn dials_become_the_functions_of_their_modules() {
        assert_eq!(
            upgraded().dials,
            [
                module_function("volume", VOLUME_FUNCTION),
                module_function("pomodoro", CONTROL_FUNCTION),
                DialAssignment::None,
                DialAssignment::Navigation,
            ]
        );
    }

    #[test]
    fn the_clock_and_navigation_modules_are_dropped_with_their_keys() {
        let config = upgraded();
        let ids: Vec<_> = config
            .modules
            .iter()
            .map(|module| module.id.0.as_str())
            .collect();
        assert_eq!(ids, ["pomodoro", "docker", "volume"]);
        let home: Vec<_> = config.home.iter().map(|id| id.0.as_str()).collect();
        assert_eq!(home, ["pomodoro", "docker"]);
    }

    #[test]
    fn the_rest_is_kept_and_the_strip_takes_its_defaults() {
        let config = upgraded();
        assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(
            (config.serial.as_str(), config.brightness, config.revision),
            ("ABC", 50, 9)
        );
        assert_eq!(config.strip, StripConfig::default());
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn a_current_configuration_is_read_as_it_is() {
        let current = upgraded();
        let bytes = serde_json::to_vec(&current).unwrap();
        assert_eq!(parse_config(&bytes).unwrap(), current);
    }

    #[test]
    fn an_unknown_dial_instance_becomes_nothing() {
        let first = FIRST_CONFIG.replace(r#""dials": ["volume""#, r#""dials": ["ghost""#);
        let config = parse_config(first.as_bytes()).unwrap();
        assert_eq!(config.dials[0], DialAssignment::None);
    }

    #[test]
    fn an_upgraded_configuration_is_read_again_as_it_is() {
        let once = upgraded();
        let twice = parse_config(&serde_json::to_vec(&once).unwrap()).unwrap();
        assert_eq!(twice, once);
    }

    #[test]
    fn other_versions_are_not_upgraded() {
        // A future version is left for validation to refuse; nothing is guessed.
        let future = FIRST_CONFIG.replace(r#""schemaVersion": 1"#, r#""schemaVersion": 3"#);
        assert!(parse_config(future.as_bytes()).is_err());
        let text = FIRST_CONFIG.replace(r#""schemaVersion": 1"#, r#""schemaVersion": "1""#);
        assert!(parse_config(text.as_bytes()).is_err());
    }

    #[test]
    fn an_oversized_configuration_is_refused_before_it_is_walked() {
        let instances: Vec<Value> = (0..=MAX_MODULES)
            .map(|index| json!({ "id": format!("m{index}"), "kind": "clock" }))
            .collect();
        let oversized =
            json!({ "schemaVersion": 1, "modules": instances, "home": [], "dials": [] });
        let error = parse_config(oversized.to_string().as_bytes()).unwrap_err();
        assert!(error.to_string().contains("too many `modules`"), "{error}");
        for (key, max) in [("home", MAX_HOME_KEYS), ("dials", MAX_DIALS)] {
            let list = vec![Value::Null; max + 1];
            let config = json!({ "schemaVersion": 1, "modules": [], key: list });
            assert!(
                parse_config(config.to_string().as_bytes()).is_err(),
                "{key}"
            );
        }
    }

    #[test]
    fn what_is_not_a_configuration_is_refused() {
        assert!(parse_config(b"[]").is_err());
        assert!(parse_config(b"{").is_err());
    }
}
