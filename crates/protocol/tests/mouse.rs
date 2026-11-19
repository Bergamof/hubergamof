//! The mouse and radial menu configurations as the public contract sees them:
//! JSON as written to `mouse.json` and `radial-menu.json`, hostile or damaged.
#![cfg(test)]

use hubergamof_protocol::{
    ButtonAction, DPI_RANGE, HOLD_RANGE, HapticWaveform, MouseButton, MouseConfig,
    MouseConfigError, MouseControlId, MouseModel, RadialConfigError, RadialMenuConfig,
    RadialPrimary,
};
use serde_json::{Value, json};

fn mouse_json() -> Value {
    json!({
        "schemaVersion": 1,
        "serial": "mx-master-4-0001",
        "model": "mxMaster4",
        "bindings": {
            "hapticPanel": { "kind": "radialMenu" },
            "gesture": { "kind": "module", "module": "audio", "action": "mute" },
            "back": { "kind": "shortcut", "keys": "Alt+Left" },
            "forward": { "kind": "click", "button": "middle" },
            "thumbWheel": { "kind": "none" }
        },
        "pointer": { "dpi": 2400 },
        "wheel": { "mode": "auto", "autoThreshold": 20, "natural": false, "hiRes": true },
        "thumbWheel": { "speed": 5, "invert": false },
        "haptics": {
            "level": "medium",
            "events": { "teams.new-message": { "enabled": true, "waveform": "knock" } }
        },
        "revision": 3
    })
}

fn radial_json() -> Value {
    json!({
        "schemaVersion": 1,
        "holdMs": 280,
        "animate": true,
        "haptics": true,
        "entries": [
            { "module": "pomodoro", "enabled": true, "primary": { "kind": "action", "action": "toggle" } },
            { "module": "teams", "enabled": true, "primary": { "kind": "close" } },
            { "module": "audio", "enabled": true, "primary": { "kind": "action", "action": "mute" } },
            { "module": "docker", "enabled": false, "primary": { "kind": "close" } }
        ],
        "revision": 0
    })
}

fn with(mut json: Value, path: &[&str], value: Value) -> Value {
    let (last, parents) = path.split_last().unwrap();
    let mut node = &mut json;
    for key in parents {
        node = &mut node[*key];
    }
    node[*last] = value;
    json
}

fn without(mut json: Value, path: &[&str]) -> Value {
    let (last, parents) = path.split_last().unwrap();
    let mut node = &mut json;
    for key in parents {
        node = &mut node[*key];
    }
    node.as_object_mut().unwrap().remove(*last);
    json
}

#[test]
fn a_mouse_file_in_the_documented_format_loads_and_validates() {
    let config: MouseConfig = serde_json::from_value(mouse_json()).unwrap();
    assert_eq!(config.validate(), Ok(()));
    assert_eq!(config.model, MouseModel::MxMaster4);
    assert_eq!(
        config.bindings[&MouseControlId::HapticPanel],
        ButtonAction::RadialMenu
    );
    assert_eq!(
        config.bindings[&MouseControlId::Forward],
        ButtonAction::Click {
            button: MouseButton::Middle
        }
    );
    assert_eq!(
        config.haptics.events["teams.new-message"].waveform,
        HapticWaveform::Knock
    );
    assert_eq!(config.revision, 3);
}

#[test]
fn a_radial_menu_file_in_the_documented_format_loads_and_validates() {
    let menu: RadialMenuConfig = serde_json::from_value(radial_json()).unwrap();
    assert_eq!(menu.validate(), Ok(()));
    assert_eq!(menu.entries[1].primary, RadialPrimary::Close);
    assert_eq!(menu.shown().count(), 3);
}

#[test]
fn damaged_mouse_files_are_refused_at_reading() {
    let cases = [
        (
            "unknown control",
            with(
                mouse_json(),
                &["bindings", "turbo"],
                json!({ "kind": "none" }),
            ),
        ),
        (
            "unknown action kind",
            with(
                mouse_json(),
                &["bindings", "back"],
                json!({ "kind": "format" }),
            ),
        ),
        (
            "action without kind",
            with(mouse_json(), &["bindings", "back"], json!({ "keys": "A" })),
        ),
        (
            "shortcut without keys",
            with(
                mouse_json(),
                &["bindings", "back"],
                json!({ "kind": "shortcut" }),
            ),
        ),
        (
            "unknown model",
            with(mouse_json(), &["model"], json!("mxFoo")),
        ),
        (
            "negative dpi",
            with(mouse_json(), &["pointer", "dpi"], json!(-1)),
        ),
        (
            "dpi past u16",
            with(mouse_json(), &["pointer", "dpi"], json!(65_536)),
        ),
        (
            "threshold past u8",
            with(mouse_json(), &["wheel", "autoThreshold"], json!(256)),
        ),
        (
            "dpi as text",
            with(mouse_json(), &["pointer", "dpi"], json!("2400")),
        ),
        (
            "unknown wheel mode",
            with(mouse_json(), &["wheel", "mode"], json!("turbo")),
        ),
        (
            "unknown haptic level",
            with(mouse_json(), &["haptics", "level"], json!("loud")),
        ),
        (
            "unknown waveform",
            with(
                mouse_json(),
                &["haptics", "events", "teams.new-message", "waveform"],
                json!("fire"),
            ),
        ),
        (
            "no schema version",
            without(mouse_json(), &["schemaVersion"]),
        ),
        ("no pointer", without(mouse_json(), &["pointer"])),
        ("no bindings", without(mouse_json(), &["bindings"])),
        ("not an object", json!([])),
    ];
    for (what, json) in cases {
        assert!(
            serde_json::from_value::<MouseConfig>(json).is_err(),
            "{what} must not load"
        );
    }
}

#[test]
fn unknown_fields_are_ignored_so_that_an_older_manager_reads_a_newer_file() {
    let config: MouseConfig =
        serde_json::from_value(with(mouse_json(), &["somethingNew"], json!(true))).unwrap();
    assert_eq!(config.validate(), Ok(()));
}

#[test]
fn well_formed_but_unacceptable_mouse_files_are_refused_by_validation() {
    let load = |json: Value| serde_json::from_value::<MouseConfig>(json).unwrap();
    assert_eq!(
        load(with(
            mouse_json(),
            &["bindings", "left"],
            json!({ "kind": "none" })
        ))
        .validate(),
        Err(MouseConfigError::LockedControl(MouseControlId::Left))
    );
    assert_eq!(
        load(with(mouse_json(), &["pointer", "dpi"], json!(225))).validate(),
        Err(MouseConfigError::InvalidDpi(225))
    );
    assert_eq!(
        load(with(
            mouse_json(),
            &["bindings", "back"],
            json!({ "kind": "shortcut", "keys": "Ctrl+" })
        ))
        .validate(),
        Err(MouseConfigError::InvalidShortcut("Ctrl+".to_owned()))
    );
    assert_eq!(
        load(with(mouse_json(), &["schemaVersion"], json!(2))).validate(),
        Err(MouseConfigError::UnsupportedSchemaVersion { found: 2 })
    );
}

#[test]
fn damaged_radial_files_are_refused_at_reading() {
    let mut unknown_primary = radial_json();
    unknown_primary["entries"][0]["primary"] = json!({ "kind": "explode" });
    let mut action_without_name = radial_json();
    action_without_name["entries"][0]["primary"] = json!({ "kind": "action" });
    let cases = [
        ("unknown primary kind", unknown_primary),
        ("action without a name", action_without_name),
        ("no hold time", without(radial_json(), &["holdMs"])),
        (
            "negative hold time",
            with(radial_json(), &["holdMs"], json!(-5)),
        ),
        (
            "hold time past u16",
            with(radial_json(), &["holdMs"], json!(65_536)),
        ),
        ("no entries", without(radial_json(), &["entries"])),
        (
            "no schema version",
            without(radial_json(), &["schemaVersion"]),
        ),
    ];
    for (what, json) in cases {
        assert!(
            serde_json::from_value::<RadialMenuConfig>(json).is_err(),
            "{what} must not load"
        );
    }
}

#[test]
fn well_formed_but_unacceptable_radial_files_are_refused_by_validation() {
    let load = |json: Value| serde_json::from_value::<RadialMenuConfig>(json).unwrap();
    assert_eq!(
        load(with(radial_json(), &["holdMs"], json!(100))).validate(),
        Err(RadialConfigError::InvalidHold(100))
    );
    assert_eq!(
        load(with(radial_json(), &["entries"], json!([]))).validate(),
        Err(RadialConfigError::ShownEntries { shown: 0 })
    );
}

#[test]
fn the_documented_ranges_are_public() {
    assert_eq!((*DPI_RANGE.start(), *DPI_RANGE.end()), (200, 8000));
    assert_eq!((*HOLD_RANGE.start(), *HOLD_RANGE.end()), (150, 500));
}
