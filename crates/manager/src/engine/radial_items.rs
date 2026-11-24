//! The actions shown around the petal of a module in the radial menu, read
//! from the engines of the devices and from the application.
//!
//! A module of a device lives in the task of its engine, which nobody else
//! may call. Each engine therefore publishes a snapshot of the items of its
//! modules after every wake-up that may have changed them (the same moment it
//! redraws), and the menu reads the snapshots synchronously.

use std::collections::BTreeMap;

use hubergamof_module_api::RadialItem;
use hubergamof_protocol::ModuleKind;

use super::EngineRegistry;
use crate::module_host::ModuleHost;

/// What one engine publishes: the items of the first instance of each module
/// kind it holds that declares radial actions. A kind with no item at the
/// moment is present with an empty list, so that it tells "this device holds
/// the module" apart from "it does not".
pub(super) type RadialItemsByKind = BTreeMap<ModuleKind, Vec<RadialItem>>;

/// The items to draw around the petal of `kind` now: those of the first
/// device (in serial order) that holds the module, otherwise those of the
/// instance of the application. Same priority as running a radial action
/// (`EngineHandle::radial_action`, then the host), so the items shown are the
/// ones that will answer. Empty when nothing holds the module or it has no
/// item.
///
/// Synchronous and cheap: it only reads snapshots, no module is called, so it
/// may be called on every redraw of the menu.
#[must_use]
pub fn first_radial_items(
    engines: &EngineRegistry,
    host: &ModuleHost,
    kind: &ModuleKind,
) -> Vec<RadialItem> {
    engines
        .radial_items_of_holder(kind)
        .unwrap_or_else(|| host.radial_items(kind))
}
