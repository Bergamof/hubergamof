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

mod config;
mod device;
mod live;
mod messages;

pub use config::{
    CURRENT_SCHEMA_VERSION, ConfigError, DeviceConfig, MAX_BRIGHTNESS, MAX_IMAGE_NAME_LENGTH,
    ModuleInstance, is_safe_image_name,
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
