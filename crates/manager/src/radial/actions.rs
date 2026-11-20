//! What an entry chosen in the radial menu does: its action runs on the first
//! device that has the module (a Stream Deck: that is where the modules, and
//! their state, live).

use hubergamof_protocol::{RadialEntryConfig, RadialPrimary};
use tokio::runtime::Handle;

use crate::engine::EngineRegistry;

/// Runs the actions of the entries chosen in the menu.
#[derive(Clone)]
pub struct ModuleActions {
    engines: EngineRegistry,
    runtime: Handle,
}

impl ModuleActions {
    /// Actions run on the devices of `engines`, on `runtime`.
    #[must_use]
    pub fn new(engines: EngineRegistry, runtime: Handle) -> Self {
        Self { engines, runtime }
    }

    /// Does what a click on the petal of `entry` does. Never waits: the action
    /// runs on the runtime, and the menu thread goes on.
    pub fn run(&self, entry: &RadialEntryConfig) {
        let RadialPrimary::Action { action } = &entry.primary else {
            return;
        };
        let (engines, module, action) =
            (self.engines.clone(), entry.module.clone(), action.clone());
        self.runtime.spawn(async move {
            for engine in engines.handles() {
                if engine.radial_action(&module, &action).await {
                    return;
                }
            }
            tracing::info!(%module, %action, "no device has the module of the radial entry");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(primary: RadialPrimary) -> RadialEntryConfig {
        RadialEntryConfig {
            module: "pomodoro".into(),
            enabled: true,
            primary,
        }
    }

    #[tokio::test]
    async fn an_entry_that_only_closes_the_menu_does_nothing() {
        let actions = ModuleActions::new(EngineRegistry::default(), Handle::current());
        actions.run(&entry(RadialPrimary::Close));
    }

    #[tokio::test]
    async fn an_action_with_no_device_is_said_and_does_not_fail() {
        let actions = ModuleActions::new(EngineRegistry::default(), Handle::current());
        actions.run(&entry(RadialPrimary::Action {
            action: "toggle".to_owned(),
        }));
        // The task ends by itself: nothing to wait for but the runtime.
        tokio::task::yield_now().await;
    }
}
