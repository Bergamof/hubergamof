//! Mice: models, what they offer, and the per-mouse configuration, persisted
//! as `config/devices/<serial>/mouse.json`.
//!
//! Adding a model means a variant of [`MouseModel`] plus its arms in
//! `display_name`, `capabilities` and `default_menu_control`, none of which has
//! a catch-all arm, so the compiler lists what is missing.

mod action;
mod config;
mod haptic;
mod model;
mod status;

pub use action::{ButtonAction, MAX_SHORTCUT_CHARS, MouseButton};
pub use config::{
    DEFAULT_DPI, DEFAULT_THUMB_SPEED, DEFAULT_WHEEL_THRESHOLD, DPI_RANGE, DPI_STEP,
    MOUSE_SCHEMA_VERSION, MouseConfig, MouseConfigError, PointerSettings, THUMB_SPEED_RANGE,
    ThumbWheelSettings, WHEEL_THRESHOLD_RANGE, WheelMode, WheelSettings,
};
pub use haptic::{HapticEventSetting, HapticLevel, HapticSettings, MAX_HAPTIC_EVENTS};
pub use model::{
    ControlKind, MouseCapabilities, MouseControl, MouseControlId, MouseInfo, MouseModel,
};
pub use status::MouseProblem;
