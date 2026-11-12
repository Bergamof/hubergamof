//! Message shown on the device when its stored configuration was brought
//! back to the placement rules of the modules.

use std::time::Duration;

use hubergamof_module_api::{ModuleInstanceId, Notification, Rgb};

use crate::config_store::PlacementChange;

/// Source of the notification: the manager, not a module instance.
pub(super) const SOURCE: &str = "hubergamof";

/// How long the notice stays on the strip; it remains among the unread ones.
const NOTICE_TIMEOUT: Duration = Duration::from_secs(20);

/// Blue, apart from the colours modules use for their own states.
const NOTICE_BACKGROUND: Rgb = Rgb(0x2B, 0x6C, 0xB0);

pub(super) fn source() -> ModuleInstanceId {
    SOURCE.into()
}

/// Explains `changes` in one line.
pub(super) fn notice(changes: &[PlacementChange]) -> Notification {
    let details: Vec<String> = changes.iter().map(describe).collect();
    Notification::new(
        format!("Configuration adaptée : {}", details.join(" · ")),
        NOTICE_BACKGROUND,
    )
    .with_timeout(NOTICE_TIMEOUT)
}

fn describe(change: &PlacementChange) -> String {
    match change {
        PlacementChange::RemovedFromKeys { module } => {
            format!("{module} n'a pas de touche, retiré de l'accueil")
        }
        PlacementChange::MergedInstances { module } => {
            format!("{module} n'existe qu'une fois, réglages partagés")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_change_is_explained_on_one_line() {
        let notification = notice(&[
            PlacementChange::RemovedFromKeys {
                module: "Pomodoro".into(),
            },
            PlacementChange::MergedInstances {
                module: "Audio".into(),
            },
        ]);
        assert_eq!(
            notification.message,
            "Configuration adaptée : Pomodoro n'a pas de touche, retiré de l'accueil \
             · Audio n'existe qu'une fois, réglages partagés"
        );
        assert_eq!(notification.timeout, Some(NOTICE_TIMEOUT));
    }
}
