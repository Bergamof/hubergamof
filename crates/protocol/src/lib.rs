//! Bergadeck wire contract.
//!
//! This crate is the single source of truth for everything exchanged between
//! the `bergadeck` manager and its web UI: the device configuration model
//! persisted as JSON, the module catalog, the live device state and the
//! WebSocket messages. Every public type derives [`ts_rs::TS`] and is exported
//! to TypeScript (`ui/src/lib/generated/`) by
//! `cargo test -p bergadeck-protocol export_bindings`.
//!
//! JSON conventions: fields are `camelCase`, enums carrying data are
//! internally tagged with a `type` field, optional values are `null`.

mod api;
mod catalog;
mod config;
mod device;
mod live;
mod messages;

pub use api::{ApiError, ImageUploaded};
pub use catalog::CatalogModule;
pub use config::{
    CURRENT_SCHEMA_VERSION, ConfigError, DeviceConfig, MAX_BRIGHTNESS, MAX_DIALS, MAX_HOME_KEYS,
    MAX_IDENTIFIER_LENGTH, MAX_IMAGE_NAME_LENGTH, MAX_LABEL_CHARS, MAX_MODULES, MAX_SETTINGS_BYTES,
    ModuleInstance, is_identifier, is_safe_image_name,
};
pub use device::{DeviceCapabilities, DeviceInfo, DeviceModel, ImageSize};
pub use live::{DialView, KeyView, LiveState, PageView, ZoneView};
pub use messages::{ClientMessage, ErrorCode, ServerMessage};

// Defined in the MPL-2.0 module API so that modules do not link GPL code;
// re-exported here because they are part of the wire contract.
pub use bergadeck_module_api::{
    ChoiceOption, KeyState, ModuleCapabilities, ModuleDescriptor, ModuleInstanceId, ModuleKind,
    SettingField, SettingKind,
};

#[cfg(test)]
mod bindings;
