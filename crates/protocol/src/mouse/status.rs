//! What the mouse said about the configuration it was given.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A setting the mouse refused when its configuration was applied (the other
/// settings were applied all the same).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MouseProblem {
    /// Which setting, in plain English (`sensitivity`, `wheel`, `button`…).
    pub setting: String,
    /// Why, as the mouse or its receiver said it.
    pub message: String,
}

/// How full the battery of a mouse is, as the mouse says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MouseBattery {
    /// Charge, from 0 to 100.
    pub percent: u8,
    /// Plugged in and charging.
    pub charging: bool,
}

/// What is known of a connected mouse besides its configuration: what it
/// refused of the configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MouseStatus {
    /// The settings the mouse refused when its configuration was last applied.
    pub problems: Vec<MouseProblem>,
}
