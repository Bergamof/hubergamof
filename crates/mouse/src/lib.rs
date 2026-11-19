//! Logitech HID++ 2.0 access for HuBergamof.
//!
//! - [`frame`]: encoding of requests and decoding of answers (pure).
//! - [`channel`]: the byte-level [`channel::Channel`] and the request/answer
//!   exchange [`channel::Hidpp`] on top of it.
//! - [`features`]: names of the HID++ features this project knows.
//! - [`probe`]: the **read-only** inventory of a device (features, buttons).
//! - [`hidraw`]: discovery and `hidapi` channel on real hardware.
//!
//! Nothing in this crate writes a setting to a device: the probe only sends
//! the HID++ *getter* functions listed in [`probe`].

pub mod channel;
pub mod features;
pub mod frame;
pub mod hidraw;
pub mod probe;
#[cfg(test)]
mod testing;
