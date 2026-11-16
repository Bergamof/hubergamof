//! Microsoft Teams module, driven through a browser extension.

pub mod bridge;
pub mod native_host;
pub mod wire;

pub use bridge::{CallState, SimulatedBridge, SocketBridge, TeamsBridge, TeamsState, socket_path};
