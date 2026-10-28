//! Core of the `bergadeck` manager: device access, image rendering and
//! per-device configuration storage.
//!
//! - [`device`]: the [`device::DeckBackend`] hardware abstraction, the
//!   [`device::DeviceDiscovery`] registry fed by the real (`hidapi`) or the
//!   virtual (`--demo`) backend, and normalised [`device::InputEvent`]s.
//! - [`render`]: decoding of PNG/JPEG/SVG sources into key or touch strip
//!   images, label overlay, data URL export and a bounded cache.
//! - [`config_store`]: `config/devices/<serial>/` persistence with revision
//!   checks, atomic writes and content-addressed images.

pub mod config_store;
pub mod device;
pub mod render;
