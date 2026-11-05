//! WebSocket messages.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{CatalogModule, DeviceConfig, DeviceInfo, LiveState};

/// Message sent by the UI to the manager.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ClientMessage {
    /// Subscribe to the configuration and live state of a device.
    SelectDevice {
        /// Device serial.
        serial: String,
        /// Also stream what the device displays (`liveState` messages).
        live_state: bool,
    },
    /// Replace the whole configuration of a device. Once accepted, the
    /// author gets a `config` with the configuration sent, unchanged except
    /// for its revision, incremented: clients recognise their own update by
    /// it. Other subscribers get it too, unless a later update overtakes it.
    ///
    /// Rejected with [`ErrorCode::StaleRevision`] when `config.revision` is not
    /// the current one.
    UpdateConfig {
        /// Device serial.
        serial: String,
        /// New configuration.
        config: DeviceConfig,
    },
    /// Simulate a key press: `--demo` only, refused with
    /// [`ErrorCode::InputRefused`] on real devices.
    PressKey {
        /// Device serial.
        serial: String,
        /// Key index.
        index: u32,
    },
    /// Simulate a dial rotation: `--demo` only, see [`ClientMessage::PressKey`].
    RotateDial {
        /// Device serial.
        serial: String,
        /// Dial index.
        index: u32,
        /// Rotation steps, positive clockwise.
        ticks: i32,
    },
    /// Simulate a dial press: `--demo` only, see [`ClientMessage::PressKey`].
    PressDial {
        /// Device serial.
        serial: String,
        /// Dial index.
        index: u32,
    },
    /// Keep-alive request, answered by [`ServerMessage::Pong`].
    Ping,
}

/// Message sent by the manager to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ServerMessage {
    /// First message after connection.
    Hello {
        /// Manager version.
        version: String,
    },
    /// Devices currently connected.
    Devices {
        /// Connected devices.
        devices: Vec<DeviceInfo>,
    },
    /// Module kinds available for configuration.
    ModuleCatalog {
        /// Available modules.
        modules: Vec<CatalogModule>,
    },
    /// Current configuration of a device.
    Config {
        /// Device serial.
        serial: String,
        /// Current configuration.
        config: DeviceConfig,
    },
    /// Current live state of a device.
    LiveState {
        /// Live state.
        state: LiveState,
    },
    /// A client request failed.
    Error {
        /// Machine-readable error code.
        code: ErrorCode,
        /// Human-readable details.
        message: String,
    },
    /// Answer to [`ClientMessage::Ping`].
    Pong,
}

/// Machine-readable error categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ErrorCode {
    /// The message could not be parsed.
    InvalidMessage,
    /// No connected or configured device has this serial.
    UnknownDevice,
    /// The configuration failed validation.
    InvalidConfig,
    /// The configuration was based on an outdated revision.
    StaleRevision,
    /// The requested key or dial does not exist on the device.
    InvalidInput,
    /// Simulated input outside `--demo`: on real devices, presses and
    /// rotations come from the device only, so that no local program can
    /// act through the manager (start containers…).
    InputRefused,
    /// The image was refused: unsupported or undecodable format, content not
    /// matching the declared type, or too large.
    InvalidImage,
    /// The configuration archive was refused: not a Bergadeck export,
    /// unexpected entries, missing image, or too large.
    InvalidArchive,
    /// Unexpected failure inside the manager.
    Internal,
}
