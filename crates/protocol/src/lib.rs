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
mod catalog;
mod config;
mod device;
mod live;
mod messages;

pub use api::{ApiError, FileChooserRequest, FileChosen, ImageUploaded};
pub use catalog::CatalogModule;
pub use config::{
    CURRENT_SCHEMA_VERSION, ConfigError, DeviceConfig, MAX_BRIGHTNESS, MAX_DIALS, MAX_HOME_KEYS,
    MAX_IDENTIFIER_LENGTH, MAX_LABEL_CHARS, MAX_MODULES, MAX_SETTINGS_BYTES, ModuleInstance,
    is_identifier,
};
pub use device::{DeviceCapabilities, DeviceInfo, DeviceModel, ImageSize};
pub use live::{DialView, KeyView, LiveState, PageView, ZoneView};
pub use messages::{ClientMessage, ErrorCode, ServerMessage};

// Defined in the MPL-2.0 module API so that modules do not link GPL code;
// re-exported here because they are part of the wire contract.
pub use hubergamof_module_api::{
    ChoiceOption, EntryNaming, FileChooser, FileEntry, FileFilter, KeyState, MAX_IMAGE_NAME_LENGTH,
    ModuleCapabilities, ModuleDescriptor, ModuleInstanceId, ModuleKind, SettingField, SettingKind,
    is_safe_image_name,
};

#[cfg(test)]
mod bindings;
