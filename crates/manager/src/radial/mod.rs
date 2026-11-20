//! The radial menu: where its petals are ([`geometry`]) and how it answers the
//! button that opens it and the pointer ([`interaction`]). Neither draws
//! anything or reads a device: the overlay and the mouse threads feed them.

pub mod backend;
pub mod controller;
pub mod geometry;
pub mod interaction;
pub mod overlay;
pub mod overlay_x11;
pub mod paint;
pub mod preview;
pub mod service;
pub mod window;
