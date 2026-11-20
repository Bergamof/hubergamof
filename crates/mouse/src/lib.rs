//! Logitech HID++ 2.0 access for HuBergamof.
//!
//! - [`frame`]: encoding of requests and decoding of answers (pure).
//! - [`channel`]: the byte-level [`channel::Channel`] and the request/answer
//!   exchange [`channel::Hidpp`] on top of it.
//! - [`features`]: names of the HID++ features this project knows.
//! - [`probe`]: the **read-only** inventory of a device (features, buttons).
//! - [`controls`]: the mouse's own ids for the controls of each model.
//! - [`diagnostics`]: the actions of `hubergamof mouse-test`, run on a link.
//! - [`link`]: the operations on a mouse (battery, sensitivity, wheel, buttons,
//!   haptics), the part of this crate that writes.
//! - [`hidraw`]: discovery and `hidapi` channel on real hardware.
//!
//! The probe only sends the HID++ *getter* functions listed in [`probe`]; only
//! [`link`] writes, and only what its operations name.

pub mod channel;
pub mod controls;
pub mod diagnostics;
pub mod features;
pub mod frame;
pub mod hidraw;
pub mod link;
pub mod probe;
#[cfg(test)]
mod testing;
