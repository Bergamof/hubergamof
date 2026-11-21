//! Logitech HID++ 2.0 access for HuBergamof.
//!
//! - [`frame`]: encoding of requests and decoding of answers (pure).
//! - [`channel`]: the byte-level [`channel::Channel`] and the request/answer
//!   exchange [`channel::Hidpp`] on top of it.
//! - [`features`]: names of the HID++ features this project knows.
//! - [`probe`]: finding a device behind an index (ping and name, getters only).
//! - [`controls`]: the mouse's own ids for the controls of each model.
//! - [`diagnostics`]: the actions of `hubergamof mouse-test`, run on a link.
//! - [`link`]: the operations on a mouse (battery, sensitivity, wheel, buttons,
//!   haptics), the part of this crate that writes.
//! - [`session`]: a mouse in use, its configuration applied and its buttons
//!   followed.
//! - [`hidraw`]: discovery and `hidapi` channel on real hardware.
//!
//! [`probe`] only sends the HID++ *getter* functions it lists; only [`link`]
//! writes, and only what its operations name.

pub mod channel;
pub mod controls;
pub mod diagnostics;
pub mod features;
pub mod frame;
pub mod hidraw;
pub mod link;
pub mod probe;
pub mod session;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
