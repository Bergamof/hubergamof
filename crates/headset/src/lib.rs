//! Sony headphones access for HuBergamof (WH-1000XM4 first).
//!
//! - [`frame`]: the MDR framing, encoded and decoded (pure).
//! - [`bluez`]: the RFCOMM connection, opened by BlueZ.
//! - [`channel`]: the byte stream to the headphones.
//! - [`diagnostics`]: the actions of `hubergamof headset-test`.
//! - [`link`]: frames exchanged on a channel (acknowledgements, resends).
//! - [`payload`]: the commands sent and the answers read, closed lists only.
//! - [`testing`]: a scripted headset (feature `test-support`).
//!
//! Only the values of [`payload`]'s types reach the headphones: nothing is
//! sent from bytes a client or a file supplied.

pub mod bluez;
pub mod channel;
pub mod diagnostics;
pub mod frame;
pub mod link;
pub mod payload;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
