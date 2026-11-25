//! WebSocket messages.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    AmbientSound, ApplicationModuleView, CatalogModule, DeviceConfig, DeviceInfo, EqualizerChange,
    GitConfig, GitSystemEntry, HapticWaveform, HeadsetInfo, HeadsetState, LiveState, MachineConfig,
    ModuleKind, MouseConfig, MouseInfo, MouseProblem, RadialMenuConfig, SshConfig, SshSystemHost,
    SystemAlias, UpdateSettings, UpdateStatus,
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
    /// Play a waveform on a connected haptic mouse, at the strength set for it, to
    /// let the person try it. Nothing is played when the strength is off, or when
    /// the same mouse was asked a moment ago; answered by nothing, or by
    /// [`ErrorCode::UnknownDevice`] when the mouse is not connected and
    /// [`ErrorCode::InvalidInput`] when it has no haptics.
    PlayHaptic {
        /// Mouse serial.
        serial: String,
        /// The pattern to play.
        waveform: HapticWaveform,
    },
    /// Follow a pair of headphones and open the link to their settings channel
    /// if it is not open (the Sony app on a phone loses the channel then):
    /// `headsetState` right away, then at every change. The link is not
    /// opened again by itself once it is closed, see
    /// [`ClientMessage::ResumeHeadset`]. Refused with
    /// [`ErrorCode::UnknownDevice`] when the headphones are not connected to
    /// this computer, [`ErrorCode::Unavailable`] when the manager does not
    /// serve headphones.
    SelectHeadset {
        /// Bluetooth address of the headphones.
        address: String,
    },
    /// Set the ambient sound of headphones the link is open to. Answered by
    /// the `headsetState` that follows; refused with
    /// [`ErrorCode::InvalidInput`] for a value the headphones do not accept,
    /// [`ErrorCode::Unavailable`] when the link is not open, like
    /// [`ErrorCode::UnknownDevice`] and [`ErrorCode::Locked`] otherwise.
    SetHeadsetAmbient {
        /// Bluetooth address of the headphones.
        address: String,
        /// What to set.
        ambient: AmbientSound,
    },
    /// Change the equalizer of headphones the link is open to, answered and
    /// refused like [`ClientMessage::SetHeadsetAmbient`].
    SetHeadsetEqualizer {
        /// Bluetooth address of the headphones.
        address: String,
        /// What to change.
        equalizer: EqualizerChange,
    },
    /// Close the link to the headphones on purpose, to leave their settings
    /// channel to another app. Answered by `headsetState`.
    ReleaseHeadset {
        /// Bluetooth address of the headphones.
        address: String,
    },
    /// Open the link again after it was released or cut (by the Sony app, for
    /// instance, which then loses the channel). Answered by `headsetState`.
    ResumeHeadset {
        /// Bluetooth address of the headphones.
        address: String,
    },
    /// Replace the configuration of the radial menu, like
    /// [`ClientMessage::UpdateMouseConfig`]; answered by `radialMenu` to
    /// every client.
    UpdateRadialMenu {
        /// New configuration.
        config: RadialMenuConfig,
    },
    /// Run an action of a module that lives in the application (start or pause
    /// the pomodoro, skip, reset): one of the `radialActions` its catalog
    /// entry offers, as the radial menu runs them. The new state arrives as
    /// `applicationModules`. Refused with [`ErrorCode::Locked`] while the
    /// computer is locked, [`ErrorCode::InvalidInput`] for an action the module
    /// does not offer, [`ErrorCode::Unavailable`] when the application holds no
    /// such module.
    ModuleAction {
        /// The module.
        module: ModuleKind,
        /// The action, one of its `radialActions`.
        action: String,
    },
    /// Save other settings for a module that lives in the application; the
    /// last device or client to save wins. Answered by `applicationModules`
    /// to every client; refused with [`ErrorCode::Locked`] while the computer
    /// is locked, [`ErrorCode::InvalidConfig`] when the module refuses the
    /// settings, [`ErrorCode::Unavailable`] when the application holds no such
    /// module.
    UpdateModuleSettings {
        /// The module.
        module: ModuleKind,
        /// Its new settings, as described by its catalog entry.
        settings: serde_json::Value,
    },
    /// Read the machine again: its configuration and the aliases found on the
    /// computer. Sent when the machine page is shown, the only time the
    /// computer is looked at. Answered by `machine`.
    RefreshMachine,
    /// Replace the configuration of the machine, like
    /// [`ClientMessage::UpdateRadialMenu`]; the aliases enabled are written on
    /// the computer, the others removed from it. Answered by `machine` to
    /// every client; refused with [`ErrorCode::Locked`] while the computer is
    /// locked, [`ErrorCode::InvalidConfig`] or [`ErrorCode::StaleRevision`].
    UpdateMachine {
        /// The new configuration, based on the current revision.
        config: MachineConfig,
    },
    /// Read the Git configuration again: what HuBergamof saved and what the
    /// computer holds besides. Sent when the Git page is shown, the only time
    /// the computer is looked at. Answered by `git`.
    RefreshGit,
    /// Replace the Git configuration, like [`ClientMessage::UpdateMachine`]:
    /// what is enabled is written on the computer, the rest removed from it.
    /// Answered by `git` to every client; refused with [`ErrorCode::Locked`],
    /// [`ErrorCode::InvalidConfig`] or [`ErrorCode::StaleRevision`].
    UpdateGit {
        /// The new configuration, based on the current revision.
        config: GitConfig,
    },
    /// Read the SSH configuration again: what HuBergamof saved and the hosts
    /// the computer holds besides. Sent when the SSH page is shown, the only
    /// time the computer is looked at. Answered by `ssh`.
    RefreshSsh,
    /// Replace the SSH configuration, like [`ClientMessage::UpdateGit`]: the
    /// hosts that are enabled are written on the computer, the rest removed
    /// from it. Answered by `ssh` to every client; refused with
    /// [`ErrorCode::Locked`], [`ErrorCode::InvalidConfig`] or
    /// [`ErrorCode::StaleRevision`].
    UpdateSsh {
        /// The new configuration, based on the current revision.
        config: SshConfig,
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
    /// What a mouse the client selected refused of its configuration: sent
    /// after its `mouseConfig`, then each time it changes (no problem means the
    /// mouse took everything). The battery is in `mice`.
    MouseStatus {
        /// Mouse serial.
        serial: String,
        /// The settings the mouse refused.
        problems: Vec<MouseProblem>,
    },
    /// Headphones currently connected to this computer: sent after `mice`,
    /// then at every connection or disconnection. Not sent when the manager
    /// does not serve headphones.
    Headsets {
        /// Connected headphones.
        headsets: Vec<HeadsetInfo>,
    },
    /// What is known of headphones the client selected: sent after
    /// `selectHeadset`, then at every change.
    HeadsetState {
        /// Bluetooth address of the headphones.
        address: String,
        /// What is known of them.
        state: HeadsetState,
    },
    /// Current configuration of the radial menu: sent after `mice`, then at
    /// every change.
    RadialMenu {
        /// Current configuration.
        config: RadialMenuConfig,
    },
    /// The modules that live in the application and what they show: sent
    /// after `moduleCatalog`, then at every change (a second of the pomodoro
    /// included). Not sent when the manager runs without them.
    ApplicationModules {
        /// One entry per module.
        modules: Vec<ApplicationModuleView>,
    },
    /// The configuration of the machine and the aliases found on the computer
    /// that it does not manage: sent after `refreshMachine` and `updateMachine`
    /// (never otherwise: the computer is not watched).
    Machine {
        /// Current configuration.
        config: MachineConfig,
        /// Aliases of the shell startup files, not managed by HuBergamof.
        system_aliases: Vec<SystemAlias>,
    },
    /// The Git configuration and what the computer holds besides: sent after
    /// `refreshGit` and `updateGit` (never otherwise: the computer is not
    /// watched).
    Git {
        /// Current configuration.
        config: GitConfig,
        /// Values HuBergamof manages that Git also finds elsewhere.
        system_entries: Vec<GitSystemEntry>,
        /// Whether the `git` program was found: without it nothing can be read
        /// from the computer.
        git_available: bool,
    },
    /// The SSH configuration and the hosts the computer holds besides: sent
    /// after `refreshSsh` and `updateSsh` (never otherwise: the computer is not
    /// watched).
    Ssh {
        /// Current configuration.
        config: SshConfig,
        /// Hosts of `~/.ssh/config` that HuBergamof does not manage.
        system_hosts: Vec<SshSystemHost>,
    },
    /// Whether the computer is locked: sent after `moduleCatalog`, then at
    /// every change. While it is, the manager refuses everything that acts or
    /// changes something ([`ErrorCode::Locked`]) and the interface is not to
    /// be used.
    Locked {
        /// `true` while the computer is locked.
        locked: bool,
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
    /// The computer is locked: nothing is done from the interface until it
    /// is unlocked.
    Locked,
    /// The configuration was saved but could not be put on the computer (a
    /// file that cannot be written, an option `ssh` refuses): it stays saved,
    /// and what is wrong is to be corrected or removed.
    NotApplied,
    /// Unexpected failure inside the manager.
    Internal,
}
