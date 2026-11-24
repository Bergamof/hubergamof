//! HuBergamof wire contract.
//!
//! This crate is the single source of truth for everything exchanged between
//! the `hubergamof` manager and its web UI: the device configuration model
//! persisted as JSON, the module catalog, the live device state and the
//! WebSocket messages. Every public type derives [`ts_rs::TS`] and is exported
//! to TypeScript (`ui/src/lib/generated/`) by
//! `cargo test -p hubergamof-protocol export_bindings`.
//!
//! JSON conventions: fields are `camelCase`, enums carrying data are
//! internally tagged with a `type` field, optional values are `null`.

mod api;
mod app_modules;
mod catalog;
mod config;
mod device;
mod git;
mod live;
mod machine;
mod messages;
mod mouse;
mod radial;
mod update;

pub use api::{ApiError, FileChooserRequest, FileChosen, ImageUploaded};
pub use app_modules::{
    APP_MODULES_SCHEMA_VERSION, AppModulesConfig, AppModulesConfigError, ApplicationModuleView,
};
pub use catalog::CatalogModule;
pub use config::{
    CURRENT_SCHEMA_VERSION, ConfigError, DeviceConfig, DeviceTheme, DialAssignment,
    LOAD_ALERT_RANGE, LOCK_SLEEP_RANGE, MAX_BRIGHTNESS, MAX_DIALS, MAX_HOME_KEYS,
    MAX_IDENTIFIER_LENGTH, MAX_LABEL_CHARS, MAX_MODULES, MAX_SETTINGS_BYTES, ModuleInstance,
    SYSTEM_REFRESH_CHOICES, StripConfig, default_lock_sleep_seconds, is_identifier,
};
pub use device::{DeviceCapabilities, DeviceInfo, DeviceModel, ImageSize};
pub use git::{
    CREDENTIAL_CACHE_SECONDS, DEFAULT_CREDENTIAL_CACHE_SECONDS, GIT_EDITORS, GIT_SCHEMA_VERSION,
    GitAlias, GitConfig, GitConfigError, GitCredentialHelper, GitFolderIdentity, GitIdentity,
    GitPullMode, GitSettings, GitSigning, GitSigningFormat, GitSystemEntry,
    MAX_GIT_ALIAS_COMMAND_CHARS, MAX_GIT_ALIAS_NAME_CHARS, MAX_GIT_ALIASES,
    MAX_GIT_FOLDER_IDENTITIES, MAX_GIT_VALUE_CHARS, is_valid_git_alias_command,
    is_valid_git_alias_name, is_valid_git_branch_name, is_valid_git_editor, is_valid_git_folder,
    is_valid_git_signing_key, is_valid_git_user_email, is_valid_git_user_name,
};
pub use live::{KeyView, LiveState, PageView};
pub use machine::{
    MACHINE_SCHEMA_VERSION, MAX_ALIAS_COMMAND_CHARS, MAX_ALIAS_NAME_CHARS, MAX_ALIASES,
    MachineConfig, MachineConfigError, ShellAlias, SystemAlias, is_valid_alias_command,
    is_valid_alias_name,
};
pub use messages::{ClientMessage, ErrorCode, ServerMessage};
pub use mouse::{
    ButtonAction, ControlKind, DEFAULT_DPI, DEFAULT_THUMB_SPEED, DEFAULT_WHEEL_THRESHOLD,
    DPI_RANGE, DPI_STEP, HapticEventSetting, HapticLevel, HapticSettings, MAX_HAPTIC_EVENTS,
    MAX_SHORTCUT_CHARS, MOUSE_SCHEMA_VERSION, MouseBattery, MouseButton, MouseCapabilities,
    MouseConfig, MouseConfigError, MouseControl, MouseControlId, MouseInfo, MouseModel,
    MouseProblem, MouseStatus, PointerSettings, THUMB_SPEED_RANGE, ThumbWheelSettings,
    WHEEL_THRESHOLD_RANGE, WheelMode, WheelSettings,
};
pub use radial::{
    DEFAULT_HOLD_MS, HOLD_RANGE, MAX_LISTED_ENTRIES, MAX_SHOWN_ENTRIES, MIN_SHOWN_ENTRIES,
    RADIAL_SCHEMA_VERSION, RadialConfigError, RadialEntryConfig, RadialMenuConfig, RadialPrimary,
};
pub use update::{InstallSupport, UpdateSettings, UpdateStatus};

// Defined in the MPL-2.0 module API so that modules do not link GPL code;
// re-exported here because they are part of the wire contract.
pub use hubergamof_module_api::{
    ChoiceOption, DialFunctionDescriptor, DialFunctionId, EntryNaming, FileChooser, FileEntry,
    FileFilter, HapticEventDescriptor, HapticWaveform, KeyState, MAX_IMAGE_NAME_LENGTH,
    ModuleCapabilities, ModuleDescriptor, ModuleInstanceId, ModuleKind, RadialActionDescriptor,
    SettingField, SettingKind, is_safe_image_name,
};

#[cfg(test)]
mod bindings;
