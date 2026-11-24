//! Freezes the JSON wire format of the WebSocket messages.

#![cfg(test)]

use hubergamof_protocol::{
    AmbientMode, AmbientSound, CatalogModule, ChoiceOption, ClientMessage, DeviceCapabilities,
    DeviceConfig, DeviceInfo, DeviceModel, DeviceTheme, DialAssignment, DialFunctionDescriptor,
    Equalizer, EqualizerChange, EqualizerPreset, ErrorCode, GitConfig, GitCredentialHelper,
    GitSystemEntry, HapticEventDescriptor, HapticWaveform, HeadsetBattery, HeadsetInfo,
    HeadsetLink, HeadsetState, ImageSize, InstallSupport, KeyState, KeyView, LiveState,
    ModuleCapabilities, ModuleDescriptor, ModuleInstance, MouseBattery, MouseProblem, PageView,
    RadialActionDescriptor, ServerMessage, SettingField, SettingKind, SshConfig, SshHost,
    SshSystemHost, StripConfig, UpdateSettings, UpdateStatus,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// Asserts that `message` serializes exactly to `expected` and back.
fn assert_wire_format<T>(message: &T, expected: &Value)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    assert_eq!(&serde_json::to_value(message).unwrap(), expected);
    let parsed: T = serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(&parsed, message);
}

fn sample_config() -> DeviceConfig {
    DeviceConfig {
        schema_version: 3,
        serial: "ABC123".into(),
        model: DeviceModel::Plus,
        brightness: 60,
        lock_sleep_seconds: 30,
        home: vec!["pomodoro".into()],
        modules: vec![ModuleInstance {
            id: "pomodoro".into(),
            kind: "pomodoro".into(),
            label: Some("Time".into()),
            image: None,
            settings: json!({ "workMinutes": 25 }),
        }],
        dials: vec![
            DialAssignment::Module {
                instance: "pomodoro".into(),
                function: "control".into(),
            },
            DialAssignment::Navigation,
            DialAssignment::Contextual,
            DialAssignment::None,
        ],
        strip: StripConfig {
            show_seconds: false,
            show_pomodoro_progress: true,
            system_refresh_seconds: 5,
            show_history: false,
            load_alert_percent: 90,
        },
        theme: DeviceTheme::Dark,
        revision: 7,
    }
}

fn sample_config_json() -> Value {
    json!({
        "schemaVersion": 3,
        "serial": "ABC123",
        "model": "plus",
        "brightness": 60,
        "lockSleepSeconds": 30,
        "home": ["pomodoro"],
        "modules": [{
            "id": "pomodoro",
            "kind": "pomodoro",
            "label": "Time",
            "image": null,
            "settings": { "workMinutes": 25 }
        }],
        "dials": [
            { "kind": "module", "instance": "pomodoro", "function": "control" },
            { "kind": "navigation" },
            { "kind": "contextual" },
            { "kind": "none" }
        ],
        "strip": {
            "showSeconds": false,
            "showPomodoroProgress": true,
            "systemRefreshSeconds": 5,
            "showHistory": false,
            "loadAlertPercent": 90
        },
        "theme": "dark",
        "revision": 7
    })
}

#[test]
fn catalog_module_without_icon() {
    assert_wire_format(
        &CatalogModule {
            descriptor: ModuleDescriptor {
                kind: "audio".into(),
                name: "Audio".into(),
                description: "Volume".into(),
                capabilities: ModuleCapabilities {
                    key: false,
                    multiple_instances: true,
                    usable_when_locked: true,
                    lock_safe_pages: false,
                },
                dial_functions: vec![DialFunctionDescriptor {
                    id: "volume".into(),
                    name: "Contrôle du volume".into(),
                    description: "Tourner change le volume.".into(),
                }],
                settings: Vec::new(),
            },
            icon: None,
            setting_icons: std::collections::BTreeMap::new(),
            default_settings: json!({}),
            radial_actions: vec![RadialActionDescriptor::new(
                "mute",
                "Couper",
                "Coupe le son",
            )],
            haptic_events: vec![HapticEventDescriptor::new(
                "muted",
                "Son coupé",
                "Quand le son est coupé",
                HapticWaveform::SharpStateChange,
            )],
        },
        &json!({
            "kind": "audio",
            "name": "Audio",
            "description": "Volume",
            "capabilities": { "key": false, "multipleInstances": true, "usableWhenLocked": true, "lockSafePages": false },
            "dialFunctions": [{
                "id": "volume",
                "name": "Contrôle du volume",
                "description": "Tourner change le volume."
            }],
            "settings": [],
            "icon": null,
            "settingIcons": {},
            "defaultSettings": {},
            "radialActions": [{ "id": "mute", "name": "Couper", "description": "Coupe le son" }],
            "hapticEvents": [{
                "id": "muted",
                "name": "Son coupé",
                "description": "Quand le son est coupé",
                "waveform": "sharpStateChange"
            }]
        }),
    );
}

#[test]
fn client_select_device() {
    assert_wire_format(
        &ClientMessage::SelectDevice {
            serial: "ABC123".into(),
            live_state: true,
        },
        &json!({ "type": "selectDevice", "serial": "ABC123", "liveState": true }),
    );
}

#[test]
fn client_update_config() {
    assert_wire_format(
        &ClientMessage::UpdateConfig {
            serial: "ABC123".into(),
            config: sample_config(),
        },
        &json!({ "type": "updateConfig", "serial": "ABC123", "config": sample_config_json() }),
    );
}

#[test]
fn client_simulated_inputs() {
    assert_wire_format(
        &ClientMessage::PressKey {
            serial: "ABC123".into(),
            index: 3,
        },
        &json!({ "type": "pressKey", "serial": "ABC123", "index": 3 }),
    );
    assert_wire_format(
        &ClientMessage::RotateDial {
            serial: "ABC123".into(),
            index: 1,
            ticks: -2,
        },
        &json!({ "type": "rotateDial", "serial": "ABC123", "index": 1, "ticks": -2 }),
    );
    assert_wire_format(
        &ClientMessage::PressDial {
            serial: "ABC123".into(),
            index: 0,
        },
        &json!({ "type": "pressDial", "serial": "ABC123", "index": 0 }),
    );
}

#[test]
fn client_play_haptic() {
    assert_wire_format(
        &ClientMessage::PlayHaptic {
            serial: "aaaa0001".into(),
            waveform: HapticWaveform::SharpStateChange,
        },
        &json!({ "type": "playHaptic", "serial": "aaaa0001", "waveform": "sharpStateChange" }),
    );
}

#[test]
fn server_mouse_status_with_and_without_a_battery() {
    assert_wire_format(
        &ServerMessage::MouseStatus {
            serial: "aaaa0001".into(),
            problems: vec![MouseProblem {
                setting: "sensitivity".into(),
                message: "no answer".into(),
            }],
            battery: Some(MouseBattery {
                percent: 80,
                charging: true,
            }),
        },
        &json!({
            "type": "mouseStatus",
            "serial": "aaaa0001",
            "problems": [{ "setting": "sensitivity", "message": "no answer" }],
            "battery": { "percent": 80, "charging": true }
        }),
    );
    assert_wire_format(
        &ServerMessage::MouseStatus {
            serial: "aaaa0001".into(),
            problems: vec![],
            battery: None,
        },
        &json!({ "type": "mouseStatus", "serial": "aaaa0001", "problems": [] }),
    );
}

#[test]
fn client_ping() {
    assert_wire_format(&ClientMessage::Ping, &json!({ "type": "ping" }));
}

#[test]
fn client_check_for_updates() {
    assert_wire_format(
        &ClientMessage::CheckForUpdates,
        &json!({ "type": "checkForUpdates" }),
    );
}

#[test]
fn client_install_requests() {
    assert_wire_format(
        &ClientMessage::InstallUpdate,
        &json!({ "type": "installUpdate" }),
    );
    assert_wire_format(
        &ClientMessage::SetAutoInstall { enabled: false },
        &json!({ "type": "setAutoInstall", "enabled": false }),
    );
}

#[test]
fn server_update_settings() {
    assert_wire_format(
        &ServerMessage::UpdateSettings {
            settings: UpdateSettings {
                auto_install: true,
                install_support: InstallSupport::Unpackaged,
            },
        },
        &json!({
            "type": "updateSettings",
            "settings": { "autoInstall": true, "installSupport": "unpackaged" }
        }),
    );
}

#[test]
fn server_install_progress_statuses() {
    for (status, expected) in [
        (
            UpdateStatus::Downloading {
                version: "1.2.0".into(),
            },
            json!({ "type": "downloading", "version": "1.2.0" }),
        ),
        (
            UpdateStatus::Installing {
                version: "1.2.0".into(),
            },
            json!({ "type": "installing", "version": "1.2.0" }),
        ),
        (
            UpdateStatus::Installed {
                version: "1.2.0".into(),
            },
            json!({ "type": "installed", "version": "1.2.0" }),
        ),
        (
            UpdateStatus::InstallFailed {
                version: "1.2.0".into(),
                message: "signature mismatch".into(),
            },
            json!({ "type": "installFailed", "version": "1.2.0", "message": "signature mismatch" }),
        ),
    ] {
        assert_wire_format(&status, &expected);
    }
}

#[test]
fn server_update_status() {
    assert_wire_format(
        &ServerMessage::UpdateStatus {
            status: UpdateStatus::Available {
                version: "1.2.0".into(),
                notes_url: "https://example.org/notes".into(),
            },
        },
        &json!({
            "type": "updateStatus",
            "status": {
                "type": "available",
                "version": "1.2.0",
                "notesUrl": "https://example.org/notes"
            }
        }),
    );
    assert_wire_format(
        &ServerMessage::UpdateStatus {
            status: UpdateStatus::CheckFailed {
                message: "offline".into(),
            },
        },
        &json!({
            "type": "updateStatus",
            "status": { "type": "checkFailed", "message": "offline" }
        }),
    );
    assert_wire_format(
        &ServerMessage::UpdateStatus {
            status: UpdateStatus::UpToDate,
        },
        &json!({ "type": "updateStatus", "status": { "type": "upToDate" } }),
    );
}

#[test]
fn unknown_client_message_is_rejected() {
    let result = serde_json::from_value::<ClientMessage>(json!({ "type": "reboot" }));
    assert!(result.is_err());
}

#[test]
fn server_hello_and_pong() {
    assert_wire_format(
        &ServerMessage::Hello {
            version: "0.1.0".into(),
        },
        &json!({ "type": "hello", "version": "0.1.0" }),
    );
    assert_wire_format(&ServerMessage::Pong, &json!({ "type": "pong" }));
}

#[test]
fn server_devices() {
    assert_wire_format(
        &ServerMessage::Devices {
            devices: vec![DeviceInfo {
                serial: "ABC123".into(),
                model: DeviceModel::Plus,
                display_name: "Stream Deck+".into(),
                capabilities: DeviceCapabilities {
                    key_columns: 4,
                    key_rows: 2,
                    key_image_size: ImageSize {
                        width: 120,
                        height: 120,
                    },
                    dial_count: 4,
                    touch_strip: Some(ImageSize {
                        width: 800,
                        height: 100,
                    }),
                },
            }],
        },
        &json!({
            "type": "devices",
            "devices": [{
                "serial": "ABC123",
                "model": "plus",
                "displayName": "Stream Deck+",
                "capabilities": {
                    "keyColumns": 4,
                    "keyRows": 2,
                    "keyImageSize": { "width": 120, "height": 120 },
                    "dialCount": 4,
                    "touchStrip": { "width": 800, "height": 100 }
                }
            }]
        }),
    );
}

#[test]
fn server_module_catalog() {
    let integer = SettingField {
        key: "workMinutes".into(),
        label: "Work".into(),
        help: Some("Length of a work session".into()),
        kind: SettingKind::Integer {
            default: 25,
            min: Some(1),
            max: None,
            unit: Some("min".into()),
        },
    };
    let choice = SettingField {
        key: "target".into(),
        label: "Target".into(),
        help: None,
        kind: SettingKind::Choice {
            default: "sink".into(),
            options: vec![ChoiceOption {
                value: "sink".into(),
                label: "Output".into(),
            }],
        },
    };
    assert_wire_format(
        &ServerMessage::ModuleCatalog {
            modules: vec![CatalogModule {
                descriptor: ModuleDescriptor {
                    kind: "pomodoro".into(),
                    name: "Pomodoro".into(),
                    description: "Focus timer".into(),
                    capabilities: ModuleCapabilities {
                        key: true,
                        ..Default::default()
                    },
                    dial_functions: Vec::new(),
                    settings: vec![integer, choice],
                },
                icon: Some("data:image/png;base64,AA==".into()),
                setting_icons: std::collections::BTreeMap::from([(
                    "files".to_owned(),
                    "data:image/png;base64,Ag==".to_owned(),
                )]),
                default_settings: json!({ "workMinutes": 25, "target": "sink" }),
                radial_actions: Vec::new(),
                haptic_events: Vec::new(),
            }],
        },
        &json!({
            "type": "moduleCatalog",
            "modules": [{
                "kind": "pomodoro",
                "name": "Pomodoro",
                "description": "Focus timer",
                "capabilities": { "key": true, "multipleInstances": false, "usableWhenLocked": false, "lockSafePages": false },
                "dialFunctions": [],
                "settings": [
                    {
                        "key": "workMinutes",
                        "label": "Work",
                        "help": "Length of a work session",
                        "kind": { "type": "integer", "default": 25, "min": 1, "max": null, "unit": "min" }
                    },
                    {
                        "key": "target",
                        "label": "Target",
                        "help": null,
                        "kind": {
                            "type": "choice",
                            "default": "sink",
                            "options": [{ "value": "sink", "label": "Output" }]
                        }
                    }
                ],
                "icon": "data:image/png;base64,AA==",
                "settingIcons": { "files": "data:image/png;base64,Ag==" },
                "defaultSettings": { "workMinutes": 25, "target": "sink" },
                "radialActions": [],
                "hapticEvents": []
            }]
        }),
    );
}

#[test]
fn setting_kinds_without_options() {
    let cases = [
        (
            SettingKind::Boolean { default: true },
            json!({ "type": "boolean", "default": true }),
        ),
        (
            SettingKind::Text {
                default: "hi".into(),
            },
            json!({ "type": "text", "default": "hi" }),
        ),
        (
            SettingKind::Path {
                default: "~/projects".into(),
                directory: true,
            },
            json!({ "type": "path", "default": "~/projects", "directory": true }),
        ),
        (
            SettingKind::PathList {
                default: vec!["a.yaml".into()],
            },
            json!({ "type": "pathList", "default": ["a.yaml"] }),
        ),
    ];
    for (kind, expected) in &cases {
        assert_wire_format(kind, expected);
    }
}

#[test]
fn server_config() {
    assert_wire_format(
        &ServerMessage::Config {
            serial: "ABC123".into(),
            config: sample_config(),
        },
        &json!({ "type": "config", "serial": "ABC123", "config": sample_config_json() }),
    );
}

#[test]
fn server_live_state() {
    assert_wire_format(
        &ServerMessage::LiveState {
            state: LiveState {
                serial: "ABC123".into(),
                page: PageView {
                    id: "home".into(),
                    title: "Home".into(),
                    row: 0,
                    row_count: 2,
                    keys: vec![KeyView {
                        index: 0,
                        label: Some("12:00".into()),
                        image: Some("data:image/png;base64,AAAA".into()),
                        state: KeyState::Active,
                    }],
                },
                strip: Some("data:image/png;base64,BBBB".into()),
            },
        },
        &json!({
            "type": "liveState",
            "state": {
                "serial": "ABC123",
                "page": {
                    "id": "home",
                    "title": "Home",
                    "row": 0,
                    "rowCount": 2,
                    "keys": [{
                        "index": 0,
                        "label": "12:00",
                        "image": "data:image/png;base64,AAAA",
                        "state": "active"
                    }]
                },
                "strip": "data:image/png;base64,BBBB"
            }
        }),
    );
}

#[test]
fn server_error() {
    assert_wire_format(
        &ServerMessage::Error {
            code: ErrorCode::StaleRevision,
            message: "configuration changed meanwhile".into(),
        },
        &json!({
            "type": "error",
            "code": "staleRevision",
            "message": "configuration changed meanwhile"
        }),
    );
}

#[test]
fn every_device_model_has_a_camel_case_name() {
    let names = [
        (DeviceModel::Original, "original"),
        (DeviceModel::OriginalV2, "originalV2"),
        (DeviceModel::MiniMk2, "miniMk2"),
        (DeviceModel::XlV2Module, "xlV2Module"),
        (DeviceModel::Mk2ModuleScissor, "mk2ModuleScissor"),
        (DeviceModel::Plus, "plus"),
        (DeviceModel::PlusXl, "plusXl"),
        (DeviceModel::Virtual, "virtual"),
    ];
    for (model, name) in names {
        assert_wire_format(&model, &json!(name));
    }
}

#[test]
fn a_configuration_without_a_lock_sleep_delay_gets_the_default() {
    let mut json = sample_config_json();
    json.as_object_mut().unwrap().remove("lockSleepSeconds");

    let config: DeviceConfig = serde_json::from_value(json).unwrap();

    assert_eq!(config.lock_sleep_seconds, 5);
}

#[test]
fn git_messages_and_configuration() {
    assert_wire_format(&ClientMessage::RefreshGit, &json!({ "type": "refreshGit" }));

    let mut config = GitConfig::empty();
    config.settings.credential_helper = GitCredentialHelper::Cache {
        timeout_seconds: 900,
    };
    let expected_config = json!({
        "schemaVersion": 1,
        "identity": { "name": "", "email": "" },
        "signing": { "enabled": false, "format": "openPgp", "key": "" },
        "settings": {
            "defaultBranch": "",
            "pull": "default",
            "editor": "",
            "credentialHelper": { "kind": "cache", "timeoutSeconds": 900 }
        },
        "aliases": [],
        "folderIdentities": [],
        "revision": 0
    });
    assert_wire_format(
        &ClientMessage::UpdateGit {
            config: config.clone(),
        },
        &json!({ "type": "updateGit", "config": expected_config }),
    );
    assert_wire_format(
        &ServerMessage::Git {
            config,
            system_entries: vec![GitSystemEntry {
                key: "user.name".into(),
                value: "Ada".into(),
                origin: "~/.gitconfig".into(),
            }],
            git_available: true,
        },
        &json!({
            "type": "git",
            "config": expected_config,
            "systemEntries": [
                { "key": "user.name", "value": "Ada", "origin": "~/.gitconfig" }
            ],
            "gitAvailable": true
        }),
    );
}

const HEADSET: &str = "AC:80:0A:0B:2E:61";

#[test]
fn client_headset_requests() {
    assert_wire_format(
        &ClientMessage::SelectHeadset {
            address: HEADSET.into(),
        },
        &json!({ "type": "selectHeadset", "address": HEADSET }),
    );
    assert_wire_format(
        &ClientMessage::ReleaseHeadset {
            address: HEADSET.into(),
        },
        &json!({ "type": "releaseHeadset", "address": HEADSET }),
    );
    assert_wire_format(
        &ClientMessage::ResumeHeadset {
            address: HEADSET.into(),
        },
        &json!({ "type": "resumeHeadset", "address": HEADSET }),
    );
}

#[test]
fn client_set_headset_ambient() {
    assert_wire_format(
        &ClientMessage::SetHeadsetAmbient {
            address: HEADSET.into(),
            ambient: AmbientSound {
                mode: AmbientMode::NoiseCancelling,
                level: 12,
                focus_on_voice: true,
            },
        },
        &json!({
            "type": "setHeadsetAmbient",
            "address": HEADSET,
            "ambient": { "mode": "noiseCancelling", "level": 12, "focusOnVoice": true }
        }),
    );
}

#[test]
fn client_set_headset_equalizer_by_preset_or_by_hand() {
    assert_wire_format(
        &ClientMessage::SetHeadsetEqualizer {
            address: HEADSET.into(),
            equalizer: EqualizerChange::Preset {
                preset: EqualizerPreset::BassBoost,
            },
        },
        &json!({
            "type": "setHeadsetEqualizer",
            "address": HEADSET,
            "equalizer": { "type": "preset", "preset": "bassBoost" }
        }),
    );
    assert_wire_format(
        &ClientMessage::SetHeadsetEqualizer {
            address: HEADSET.into(),
            equalizer: EqualizerChange::Manual {
                clear_bass: -3,
                bands: [4, 3, 1, 2, -10],
            },
        },
        &json!({
            "type": "setHeadsetEqualizer",
            "address": HEADSET,
            "equalizer": { "type": "manual", "clearBass": -3, "bands": [4, 3, 1, 2, -10] }
        }),
    );
}

#[test]
fn server_headsets() {
    assert_wire_format(
        &ServerMessage::Headsets {
            headsets: vec![HeadsetInfo {
                address: HEADSET.into(),
                name: "WH-1000XM4".into(),
            }],
        },
        &json!({
            "type": "headsets",
            "headsets": [{ "address": HEADSET, "name": "WH-1000XM4" }]
        }),
    );
}

#[test]
fn server_headset_state_known_and_unknown() {
    assert_wire_format(
        &ServerMessage::HeadsetState {
            address: HEADSET.into(),
            state: HeadsetState {
                link: HeadsetLink::Connected,
                battery: Some(HeadsetBattery {
                    percent: 60,
                    charging: false,
                }),
                ambient: Some(AmbientSound {
                    mode: AmbientMode::Ambient,
                    level: 5,
                    focus_on_voice: false,
                }),
                wind_reduction: true,
                equalizer: Some(Equalizer {
                    preset: EqualizerPreset::Speech,
                    clear_bass: -10,
                    bands: [4, 3, 1, 2, -10],
                }),
            },
        },
        &json!({
            "type": "headsetState",
            "address": HEADSET,
            "state": {
                "link": "connected",
                "battery": { "percent": 60, "charging": false },
                "ambient": { "mode": "ambient", "level": 5, "focusOnVoice": false },
                "windReduction": true,
                "equalizer": { "preset": "speech", "clearBass": -10, "bands": [4, 3, 1, 2, -10] }
            }
        }),
    );
    assert_wire_format(
        &ServerMessage::HeadsetState {
            address: HEADSET.into(),
            state: HeadsetState::unknown(HeadsetLink::Cut),
        },
        &json!({
            "type": "headsetState",
            "address": HEADSET,
            "state": { "link": "cut", "windReduction": false }
        }),
    );
}

#[test]
fn headset_links_and_modes_are_camel_case() {
    for (link, name) in [
        (HeadsetLink::Idle, "idle"),
        (HeadsetLink::Connecting, "connecting"),
        (HeadsetLink::Connected, "connected"),
        (HeadsetLink::Released, "released"),
        (HeadsetLink::Cut, "cut"),
    ] {
        assert_eq!(serde_json::to_value(link).unwrap(), json!(name));
    }
    for (mode, name) in [
        (AmbientMode::Off, "off"),
        (AmbientMode::NoiseCancelling, "noiseCancelling"),
        (AmbientMode::Ambient, "ambient"),
        (AmbientMode::WindReduction, "windReduction"),
    ] {
        assert_eq!(serde_json::to_value(mode).unwrap(), json!(name));
    }
}

#[test]
fn an_equalizer_with_the_wrong_number_of_bands_is_not_a_message() {
    let bad = json!({
        "type": "setHeadsetEqualizer",
        "address": HEADSET,
        "equalizer": { "type": "manual", "clearBass": 0, "bands": [0, 0, 0, 0] }
    });
    assert!(serde_json::from_value::<ClientMessage>(bad).is_err());
}

#[test]
fn ssh_messages_and_configuration() {
    assert_wire_format(&ClientMessage::RefreshSsh, &json!({ "type": "refreshSsh" }));

    let mut config = SshConfig::empty();
    config.hosts.push(SshHost {
        alias: "prod".into(),
        host_name: "10.0.0.1".into(),
        user: "deploy".into(),
        port: Some(2222),
        identity_file: "~/.ssh/id".into(),
        proxy_jump: String::new(),
        forward_agent: true,
        enabled: true,
    });
    let expected = json!({
        "schemaVersion": 1,
        "hosts": [{
            "alias": "prod", "hostName": "10.0.0.1", "user": "deploy", "port": 2222,
            "identityFile": "~/.ssh/id", "proxyJump": "", "forwardAgent": true, "enabled": true
        }],
        "revision": 0
    });
    assert_wire_format(
        &ClientMessage::UpdateSsh {
            config: config.clone(),
        },
        &json!({ "type": "updateSsh", "config": expected }),
    );
    assert_wire_format(
        &ServerMessage::Ssh {
            config,
            system_hosts: vec![SshSystemHost {
                alias: "web".into(),
                host_name: String::new(),
                user: String::new(),
                port: None,
                identity_file: String::new(),
                proxy_jump: String::new(),
                forward_agent: false,
                other_options: vec!["proxycommand".into()],
            }],
        },
        &json!({
            "type": "ssh",
            "config": expected,
            "systemHosts": [{
                "alias": "web", "hostName": "", "user": "", "port": null,
                "identityFile": "", "proxyJump": "", "forwardAgent": false,
                "otherOptions": ["proxycommand"]
            }]
        }),
    );
}
