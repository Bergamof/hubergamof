//! API implemented by Bergadeck modules.
//!
//! A module drives part of a Stream Deck: the key that represents it on the
//! home page, the pages it opens, the dial it may be assigned to and the touch
//! strip zone above that dial. Built-in modules (`bergadeck-modules`) use this
//! API exactly like future external modules will.
//!
//! The `Module` trait itself is not designed yet; this crate only reserves
//! the boundary and its license (MPL-2.0, so that third-party modules are not
//! bound by the GPL of the manager).
