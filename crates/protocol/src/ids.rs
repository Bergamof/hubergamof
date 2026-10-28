//! String identifiers used across the contract.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Identifier of a module instance, unique within a [`crate::DeviceConfig`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModuleInstanceId(pub String);

/// Kind of module an instance is built from (`"clock"`, `"audio"`, `"docker"`…).
///
/// Kept as an open string rather than an enum so that future external modules
/// can be configured without changing the contract.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModuleKind(pub String);

impl fmt::Display for ModuleInstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for ModuleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ModuleInstanceId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<&str> for ModuleKind {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}
