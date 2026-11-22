//! Core of the `hubergamof` manager: device access, image rendering,
//! per-device configuration storage, the page engine, the web server and the
//! tray icon.
//!
//! - [`device`]: the [`device::DeckBackend`] hardware abstraction, the
//!   [`device::DeviceDiscovery`] registry fed by the real (`hidapi`) or the
//!   virtual (`--demo`) backend, and normalised [`device::InputEvent`]s.
//! - [`render`]: decoding of PNG/JPEG/SVG sources into key or touch strip
//!   images, label overlay, data URL export and a bounded cache.
//! - [`config_store`]: `config/devices/<serial>/` persistence with revision
//!   checks, atomic writes and content-addressed images.
//! - [`engine`]: one task per device running its modules, navigation and
//!   incremental drawing, observable and controllable through
//!   [`engine::EngineHandle`].
//! - [`server`]: the local web server (UI files and `/ws` WebSocket).
//! - [`tray`]: the system tray icon (`StatusNotifierItem`).
//! - [`mouse_engine`]: one thread per Logitech mouse, applying its
//!   configuration and following its buttons.
//! - [`mouse_actions`]: what a mouse button does when pressed (clicks, shortcuts,
//!   the wheel toggle).
//! - [`mouse_hub`]: the mice connected and the mouse and radial menu
//!   configurations, as the web interface follows them.
//! - [`input`]: the keys and buttons pressed on behalf of a mouse button,
//!   through a virtual input device.
//! - [`launcher`]: `hubergamof open`, starting the service and opening the UI
//!   from the desktop launcher.

mod cache;
pub mod config_store;
pub mod device;
pub mod engine;
pub mod file_chooser;
pub mod haptics;
pub mod input;
pub mod launcher;
pub mod module_host;
pub mod mouse_actions;
pub mod mouse_engine;
pub mod mouse_hub;
pub mod placement;
pub mod radial;
pub mod render;
pub mod screen_lock;
pub mod server;
pub mod strip;
pub mod teams_extension;
#[cfg(test)]
mod testing;
pub mod tray;
pub mod update;
