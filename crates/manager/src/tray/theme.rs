//! Theme of the devices from the tray: applies the choice made in the menu,
//! and follows the configurations to keep the theme in use marked.

use hubergamof_protocol::DeviceTheme;
use tokio::sync::watch;
use tokio::task::JoinSet;
use zbus::Connection;

use super::dbusmenu::{DbusMenu, MENU_PATH};
use super::menu::THEME_ID;
use crate::config_store::ConfigStoreError;
use crate::engine::{EngineError, EngineHandle, EngineRegistry};

/// Tries to save a theme: once more when the interface saved a configuration
/// in the meantime (stale revision).
const APPLY_ATTEMPTS: usize = 2;

/// Theme of all `themes`, `None` when they differ or there is none.
pub(super) fn shared_theme(themes: impl IntoIterator<Item = DeviceTheme>) -> Option<DeviceTheme> {
    let mut themes = themes.into_iter();
    let first = themes.next()?;
    themes.all(|theme| theme == first).then_some(first)
}

/// Gives every connected device `theme`, through its engine: saved and shown
/// like a change made in the web interface.
pub(super) async fn apply_theme(engines: &EngineRegistry, theme: DeviceTheme) {
    for handle in engines.handles() {
        apply_to(&handle, theme).await;
    }
}

async fn apply_to(handle: &EngineHandle, theme: DeviceTheme) {
    for attempt in 1..=APPLY_ATTEMPTS {
        let mut config = handle.config().borrow().clone();
        if config.theme == theme {
            return;
        }
        config.theme = theme;
        match handle.apply_config(config).await {
            Ok(_) => return,
            Err(EngineError::Config(ConfigStoreError::StaleRevision { .. }))
                if attempt < APPLY_ATTEMPTS => {}
            Err(error) => {
                tracing::warn!(
                    serial = %handle.info().serial,
                    %error,
                    "cannot apply the theme chosen in the system tray"
                );
                return;
            }
        }
    }
}

/// Marks in the menu the theme shared by the devices, as long as the
/// engines run: follows the engines coming and going and their changes of
/// configuration.
pub(super) async fn follow_theme(connection: Connection, engines: EngineRegistry) {
    let mut registry = engines.changes();
    loop {
        let (themes, configurations) = current_themes(&engines);
        let shared = shared_theme(themes.iter().copied());
        tracing::debug!(?themes, ?shared, "theme shown in the system tray menu");
        if let Err(error) = publish_theme(&connection, shared).await {
            tracing::warn!(%error, "cannot update the theme in the system tray menu");
        }
        let mut changes = JoinSet::new();
        for mut configuration in configurations {
            changes.spawn(async move { configuration.changed().await.is_ok() });
        }
        tokio::select! {
            result = registry.changed() => {
                if result.is_err() {
                    return;
                }
            }
            Some(_) = changes.join_next() => {}
        }
    }
}

/// The theme of each running engine, with the receivers to wait for their
/// next change: read from the same value, so no change is missed in between.
fn current_themes(
    engines: &EngineRegistry,
) -> (
    Vec<DeviceTheme>,
    Vec<watch::Receiver<hubergamof_protocol::DeviceConfig>>,
) {
    engines
        .handles()
        .iter()
        .map(|handle| {
            let mut configuration = handle.config();
            let theme = configuration.borrow_and_update().theme;
            (theme, configuration)
        })
        .unzip()
}

/// Marks `theme` in the menu and tells the host when that changed it.
pub(super) async fn publish_theme(
    connection: &Connection,
    theme: Option<DeviceTheme>,
) -> zbus::Result<()> {
    let menu = connection
        .object_server()
        .interface::<_, DbusMenu>(MENU_PATH)
        .await?;
    let (revision, entries) = {
        let mut menu = menu.get_mut().await;
        if !menu.menu_mut().set_theme(theme) {
            return Ok(());
        }
        (menu.revision(), menu.theme_entries())
    };
    // Hosts keep what they read when they registered, often before the
    // devices were ready: both signals, whichever one a host follows.
    DbusMenu::items_properties_updated(menu.signal_emitter(), entries, Vec::new()).await?;
    // The entries of the submenu changed, not the layout of the root: hosts
    // that reload only the first level below the announced parent would
    // keep the old marks.
    DbusMenu::layout_updated(menu.signal_emitter(), revision, THEME_ID).await
}

#[cfg(test)]
pub(super) mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use hubergamof_modules::{BuiltinServices, builtin_registry_with};
    use tempfile::TempDir;

    use super::*;
    use crate::config_store::ConfigStore;
    use crate::device::DeviceDiscovery;
    use crate::device::virtual_deck::{VirtualDeck, VirtualDevices, virtual_plus_info};
    use crate::engine::{EngineServices, Supervisor};
    use crate::render::Renderer;
    use crate::strip::{FixedClock, SimulatedProbe};

    const WAIT: Duration = Duration::from_secs(10);

    /// Engines of virtual devices, as the manager runs them.
    pub(in crate::tray) struct Rig {
        pub(in crate::tray) engines: EngineRegistry,
        supervisor: Supervisor,
        _decks: Vec<Arc<VirtualDeck>>,
        _dir: TempDir,
    }

    impl Rig {
        /// One engine per serial, started and ready.
        pub(in crate::tray) async fn start(serials: &[&str]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let registry = Arc::new(builtin_registry_with(BuiltinServices::simulated()));
            let services = EngineServices {
                store: ConfigStore::new(dir.path(), Arc::clone(&registry)),
                renderer: Arc::new(Renderer::new().unwrap()),
                registry,
                wall_clock: Arc::new(FixedClock::default()),
                system: Arc::new(SimulatedProbe::default()),
            };
            let discovery = DeviceDiscovery::new();
            let supervisor = Supervisor::start(&discovery, services);
            let devices = VirtualDevices::new(discovery);
            let decks = serials
                .iter()
                .map(|serial| devices.attach(virtual_plus_info(serial)))
                .collect();
            let engines = supervisor.engines();
            let mut changes = engines.changes();
            tokio::time::timeout(WAIT, async {
                while engines.handles().len() < serials.len() {
                    changes.changed().await.unwrap();
                }
            })
            .await
            .expect("the engines start");
            Self {
                engines,
                supervisor,
                _decks: decks,
                _dir: dir,
            }
        }

        pub(in crate::tray) fn theme_of(&self, serial: &str) -> DeviceTheme {
            self.engines.get(serial).unwrap().config().borrow().theme
        }

        pub(in crate::tray) async fn wait_for_theme(&self, serial: &str, theme: DeviceTheme) {
            let mut config = self.engines.get(serial).unwrap().config();
            tokio::time::timeout(WAIT, config.wait_for(|config| config.theme == theme))
                .await
                .expect("the theme is applied")
                .unwrap();
        }

        pub(in crate::tray) async fn stop(self) {
            self.supervisor.shutdown().await;
        }
    }

    #[test]
    fn devices_share_a_theme_only_when_they_agree() {
        use DeviceTheme::{Dark, Light};
        assert_eq!(shared_theme([]), None);
        assert_eq!(shared_theme([Dark]), Some(Dark));
        assert_eq!(shared_theme([Light, Light]), Some(Light));
        assert_eq!(shared_theme([Light, Dark]), None);
    }

    #[tokio::test]
    async fn the_chosen_theme_is_applied_to_every_device() {
        let rig = Rig::start(&["A1", "B2"]).await;
        assert_eq!(rig.theme_of("A1"), DeviceTheme::Light);

        apply_theme(&rig.engines, DeviceTheme::Dark).await;
        assert_eq!(rig.theme_of("A1"), DeviceTheme::Dark);
        assert_eq!(rig.theme_of("B2"), DeviceTheme::Dark);
        rig.stop().await;
    }

    #[tokio::test]
    async fn the_theme_already_in_use_changes_nothing() {
        let rig = Rig::start(&["A1"]).await;
        let revision = |rig: &Rig| rig.engines.get("A1").unwrap().config().borrow().revision;
        let before = revision(&rig);

        apply_theme(&rig.engines, DeviceTheme::Light).await;
        assert_eq!(revision(&rig), before);
        apply_theme(&rig.engines, DeviceTheme::Dark).await;
        assert_eq!(revision(&rig), before + 1);
        rig.stop().await;
    }
}
