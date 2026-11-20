//! WebSocket messages.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    CatalogModule, DeviceConfig, DeviceInfo, LiveState, MouseConfig, MouseInfo, RadialMenuConfig,
    UpdateSettings, UpdateStatus,
};

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
    /// Subscribe to the configuration of a connected mouse: `mouseConfig`
    /// right away, then on every change (the previous mouse is dropped).
    SelectMouse {
        /// Mouse serial.
        serial: String,
    },
    /// Replace the whole configuration of a connected mouse. Once accepted,
    /// the author gets a `mouseConfig` with the configuration sent, unchanged
    /// except for its revision, incremented, and other subscribers get it too.
    ///
    /// Rejected with [`ErrorCode::StaleRevision`] when `config.revision` is not
    /// the current one, [`ErrorCode::InvalidConfig`] when it does not fit the
    /// model, [`ErrorCode::UnknownDevice`] when the mouse is not connected.
    UpdateMouseConfig {
        /// Mouse serial.
        serial: String,
        /// New configuration.
        config: MouseConfig,
    },
    /// Replace the configuration of the radial menu, like
    /// [`ClientMessage::UpdateMouseConfig`]; answered by `radialMenu` to
    /// every client.
    UpdateRadialMenu {
        /// New configuration.
        config: RadialMenuConfig,
    },
    /// Check for a newer release now, instead of waiting for the next
    /// periodic check. The outcome arrives as [`ServerMessage::UpdateStatus`].
    CheckForUpdates,
    /// Download, verify and install the newer release found by the check.
    /// Ignored when there is none or when installing is not supported (the
    /// status then says why).
    InstallUpdate,
    /// Choose whether a newer release is installed as soon as it is found.
    /// Answered by [`ServerMessage::UpdateSettings`].
    SetAutoInstall {
        /// Install automatically.
        enabled: bool,
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
    /// Mice currently connected: sent after `moduleCatalog`, then at every
    /// attach or detach.
    Mice {
        /// Connected mice.
        mice: Vec<MouseInfo>,
    },
    /// Current configuration of a mouse the client selected.
    MouseConfig {
        /// Mouse serial.
        serial: String,
        /// Current configuration.
        config: MouseConfig,
    },
    /// Current configuration of the radial menu: sent after `mice`, then at
    /// every change.
    RadialMenu {
        /// Current configuration.
        config: RadialMenuConfig,
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
    /// How updates are handled: sent after `moduleCatalog`, then at every
    /// change.
    UpdateSettings {
        /// Current settings.
        settings: UpdateSettings,
    },
    /// Result of the check for a newer release: sent after `updateSettings`
    /// once a check has started (never while the status is still
    /// [`UpdateStatus::Unknown`]), then at every change.
    UpdateStatus {
        /// Current status.
        status: UpdateStatus,
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
    /// The configuration archive was refused: not a HuBergamof export,
    /// unexpected entries, missing image, or too large.
    InvalidArchive,
    /// The desktop feature asked for is not available here: no file chooser
    /// (no desktop portal on this desktop).
    Unavailable,
    /// The desktop feature asked for is in use: a file chooser is already
    /// open. Asking again later may work.
    Busy,
    /// Unexpected failure inside the manager.
    Internal,
}
