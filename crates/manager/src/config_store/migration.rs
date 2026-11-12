//! Upgrade of configurations written by earlier versions.
//!
//! Version 1 assigned a module instance to each dial and had a clock and a
//! navigation module. Version 2 assigns functions, offered by modules or by
//! the manager, and shows time and navigation on the touch strip itself.

use hubergamof_modules::audio::{AUDIO_KIND, VOLUME_FUNCTION};
use hubergamof_modules::pomodoro::{CONTROL_FUNCTION, POMODORO_KIND};
use hubergamof_protocol::{CURRENT_SCHEMA_VERSION, DeviceConfig, DialAssignment, StripConfig};
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
        upgrade_from_first(&mut value);
    }
    serde_json::from_value(value)
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
    // The defaults are built from the contract itself: nothing to keep in
    // step by hand.
    if let Ok(strip) = serde_json::to_value(StripConfig::default()) {
        config.insert("strip".into(), strip);
    }
    config.insert("schemaVersion".into(), json!(CURRENT_SCHEMA_VERSION));
}

/// `(id, kind)` of every module instance.
fn instance_kinds(config: &Map<String, Value>) -> Vec<(String, String)> {
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
fn upgraded_dial(dial: &Value, kinds: &[(String, String)]) -> Value {
    let assignment = dial
        .as_str()
        .and_then(|id| kinds.iter().find(|(known, _)| known == id))
        .map_or(DialAssignment::None, |(id, kind)| match kind.as_str() {
            AUDIO_KIND => module_function(id, VOLUME_FUNCTION),
            POMODORO_KIND => module_function(id, CONTROL_FUNCTION),
            LEGACY_NAVIGATION_KIND => DialAssignment::Navigation,
            _ => DialAssignment::None,
        });
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
    let legacy: Vec<String> = instance_kinds(config)
        .into_iter()
        .filter(|(_, kind)| kind == LEGACY_CLOCK_KIND || kind == LEGACY_NAVIGATION_KIND)
        .map(|(id, _)| id)
        .collect();
    if let Some(modules) = config.get_mut("modules").and_then(Value::as_array_mut) {
        modules.retain(|instance| {
            let id = instance.get("id").and_then(Value::as_str);
            !id.is_some_and(|id| legacy.iter().any(|legacy| legacy == id))
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
    fn what_is_not_a_configuration_is_refused() {
        assert!(parse_config(b"[]").is_err());
        assert!(parse_config(b"{").is_err());
    }
}
