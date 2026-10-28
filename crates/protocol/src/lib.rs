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

mod catalog;
mod config;
mod device;
mod ids;
mod live;
mod messages;

pub use catalog::{ChoiceOption, ModuleCapabilities, ModuleDescriptor, SettingField, SettingKind};
pub use config::{
    CURRENT_SCHEMA_VERSION, ConfigError, DeviceConfig, MAX_BRIGHTNESS, ModuleInstance,
};
pub use device::{DeviceCapabilities, DeviceInfo, DeviceModel, ImageSize};
pub use ids::{ModuleInstanceId, ModuleKind};
pub use live::{DialView, KeyView, LiveState, PageView, ZoneView};
pub use messages::{ClientMessage, ErrorCode, ServerMessage};

#[cfg(test)]
mod bindings;
