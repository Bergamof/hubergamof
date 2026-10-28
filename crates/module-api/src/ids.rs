//! String identifiers.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Identifier of a module instance, unique within a device configuration.
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

/// Identifier of a page of a module, chosen by the module.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PageId(pub String);

/// Identifier of an item of a module page, chosen by the module.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(pub String);

macro_rules! string_id {
    ($($name:ident),+) => {$(
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    )+};
}

string_id!(ModuleInstanceId, ModuleKind, PageId, ItemId);
