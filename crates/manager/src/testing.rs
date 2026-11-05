//! Helpers shared by the unit tests of the crate.

use hubergamof_modules::{BuiltinServices, ModuleRegistry, builtin_registry_with};

/// The built-in modules on simulated services, so that no test touches the
/// audio or the desktop of the machine.
pub(crate) fn builtin_registry() -> ModuleRegistry {
    builtin_registry_with(BuiltinServices::simulated())
}
