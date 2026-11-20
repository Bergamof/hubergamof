//! What the mice and the radial menu look like to the web interface: the mice
//! connected, the configuration of each, and the configuration of the menu,
//! each as a `watch` channel so that every client follows every change, and
//! the one place that saves them.
//!
//! The mouse threads ([`crate::mouse_engine`]) register their mouse here for as
//! long as they serve it. The radial menu exists whether or not a mouse is
//! connected.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use hubergamof_protocol::{
    ButtonAction, ErrorCode, MouseConfig, MouseControlId, MouseInfo, RadialMenuConfig,
};
use thiserror::Error;
use tokio::sync::watch;

use crate::config_store::{ConfigStore, ConfigStoreError};
use crate::input::{ShortcutError, parse_shortcut};

/// Why a configuration update is refused.
#[derive(Debug, Error)]
pub enum HubError {
    /// No mouse with this serial is connected.
    #[error("no connected mouse with serial `{0}`")]
    UnknownMouse(String),
    /// The store refused or failed.
    #[error(transparent)]
    Store(#[from] ConfigStoreError),
    /// A shortcut that cannot be pressed (a key outside the list, a reserved
    /// combination): refused when it is saved, not when it is needed.
    #[error("shortcut of the {control}: {error}")]
    InvalidShortcut {
        /// The control it is bound to.
        control: MouseControlId,
        /// Why it is refused.
        error: ShortcutError,
    },
}

impl HubError {
    /// Error code reported to the UI.
    #[must_use]
    pub fn error_code(&self) -> ErrorCode {
        match self {
            Self::UnknownMouse(_) => ErrorCode::UnknownDevice,
            Self::Store(error) => error.error_code(),
            Self::InvalidShortcut { .. } => ErrorCode::InvalidConfig,
        }
    }
}

struct Entry {
    id: u64,
    info: MouseInfo,
    config: Arc<watch::Sender<MouseConfig>>,
}

struct Shared {
    store: ConfigStore,
    entries: Mutex<Vec<Entry>>,
    mice: watch::Sender<Vec<MouseInfo>>,
    radial: watch::Sender<RadialMenuConfig>,
    next_id: AtomicU64,
}

/// The mice and the radial menu, for the server and the mouse threads. Cheap
/// to clone.
#[derive(Clone)]
pub struct MouseHub {
    shared: Arc<Shared>,
}

impl MouseHub {
    /// A hub over `store`, with the radial menu loaded (created from its
    /// default the first time).
    ///
    /// # Errors
    /// [`ConfigStoreError`] when the menu file cannot be read or written.
    pub fn new(store: ConfigStore) -> Result<Self, ConfigStoreError> {
        let radial = store.load_radial_menu()?.config;
        Ok(Self {
            shared: Arc::new(Shared {
                store,
                entries: Mutex::default(),
                mice: watch::channel(Vec::new()).0,
                radial: watch::channel(radial).0,
                next_id: AtomicU64::new(0),
            }),
        })
    }

    fn entries(&self) -> MutexGuard<'_, Vec<Entry>> {
        // Plain data: nothing to recover from a poisoned lock.
        self.shared
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn publish_mice(&self, entries: &[Entry]) {
        let mice = entries.iter().map(|entry| entry.info.clone()).collect();
        self.shared.mice.send_replace(mice);
    }

    /// Lists a mouse as connected, with the configuration it is served with,
    /// until the returned registration is dropped. A mouse registered again
    /// under the same serial replaces the previous entry.
    #[must_use]
    pub fn register(&self, info: MouseInfo, config: MouseConfig) -> MouseRegistration {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, updates) = watch::channel(config);
        let sender = Arc::new(sender);
        let mut entries = self.entries();
        entries.retain(|entry| entry.info.serial != info.serial);
        entries.push(Entry {
            id,
            info,
            config: Arc::clone(&sender),
        });
        self.publish_mice(&entries);
        drop(entries);
        MouseRegistration {
            hub: self.clone(),
            id,
            sender,
            updates,
        }
    }

    /// The mice connected now.
    #[must_use]
    pub fn mice(&self) -> Vec<MouseInfo> {
        self.shared.mice.borrow().clone()
    }

    /// The list of connected mice, as it changes.
    #[must_use]
    pub fn subscribe_mice(&self) -> watch::Receiver<Vec<MouseInfo>> {
        self.shared.mice.subscribe()
    }

    /// The configuration of a connected mouse, as it changes; `None` when it
    /// is not connected.
    #[must_use]
    pub fn subscribe_config(&self, serial: &str) -> Option<watch::Receiver<MouseConfig>> {
        let entries = self.entries();
        let entry = entries.iter().find(|entry| entry.info.serial == serial)?;
        Some(entry.config.subscribe())
    }

    /// Saves a new configuration of a connected mouse and gives it to the
    /// ones following it. Writes a file: call it from a blocking task.
    ///
    /// # Errors
    /// [`HubError::UnknownMouse`] when it is not connected, or the store's
    /// refusal (outdated revision, a configuration that does not fit the
    /// model).
    pub fn update_mouse(&self, serial: &str, config: MouseConfig) -> Result<MouseConfig, HubError> {
        if config.serial != serial {
            return Err(ConfigStoreError::SerialMismatch {
                expected: serial.to_owned(),
                found: config.serial,
            }
            .into());
        }
        // The lock is for the lookup only: the store has its own for the write,
        // and the revisions order what is given to the followers.
        let (sender, model) = {
            let entries = self.entries();
            let entry = entries
                .iter()
                .find(|entry| entry.info.serial == serial)
                .ok_or_else(|| HubError::UnknownMouse(serial.to_owned()))?;
            (Arc::clone(&entry.config), entry.info.model)
        };
        // The model of the mouse is what it is, whatever the client says: a
        // configuration of another model would end its service.
        if config.model != model {
            return Err(ConfigStoreError::ModelMismatch {
                expected: model,
                found: config.model,
            }
            .into());
        }
        for (control, action) in &config.bindings {
            if let ButtonAction::Shortcut { keys } = action {
                parse_shortcut(keys).map_err(|error| HubError::InvalidShortcut {
                    control: *control,
                    error,
                })?;
            }
        }
        let saved = self.shared.store.save_mouse(config)?;
        sender.send_if_modified(|current| {
            let newer = saved.revision > current.revision;
            if newer {
                current.clone_from(&saved);
            }
            newer
        });
        Ok(saved)
    }

    /// The configuration of the radial menu, as it changes.
    #[must_use]
    pub fn subscribe_radial_menu(&self) -> watch::Receiver<RadialMenuConfig> {
        self.shared.radial.subscribe()
    }

    /// Saves a new configuration of the radial menu and gives it to the
    /// clients. Writes a file: call it from a blocking task.
    ///
    /// # Errors
    /// The store's refusal (outdated revision, invalid menu).
    pub fn update_radial_menu(
        &self,
        config: RadialMenuConfig,
    ) -> Result<RadialMenuConfig, ConfigStoreError> {
        let saved = self.shared.store.save_radial_menu(config)?;
        self.shared.radial.send_if_modified(|current| {
            let newer = saved.revision > current.revision;
            if newer {
                current.clone_from(&saved);
            }
            newer
        });
        Ok(saved)
    }
}

/// A mouse listed as connected; it is removed when this is dropped.
pub struct MouseRegistration {
    hub: MouseHub,
    id: u64,
    sender: Arc<watch::Sender<MouseConfig>>,
    updates: watch::Receiver<MouseConfig>,
}

impl MouseRegistration {
    /// Offers a configuration read from the store after the registration: one
    /// saved by the interface between the moment the mouse's configuration was
    /// loaded and the moment it was listed would otherwise be missed. Only a
    /// newer revision is taken.
    pub fn refresh(&self, config: MouseConfig) {
        self.sender.send_if_modified(|current| {
            let newer = config.revision > current.revision;
            if newer {
                *current = config;
            }
            newer
        });
    }

    /// The configuration of the mouse, as the interface changes it.
    pub fn updates(&mut self) -> &mut watch::Receiver<MouseConfig> {
        &mut self.updates
    }
}

impl Drop for MouseRegistration {
    fn drop(&mut self) {
        let mut entries = self.hub.entries();
        // Not an entry that replaced this one.
        entries.retain(|entry| entry.id != self.id);
        self.hub.publish_mice(&entries);
    }
}

#[cfg(test)]
mod tests {
    use hubergamof_protocol::MouseModel;

    use super::*;
    use crate::testing::builtin_registry;

    fn hub() -> (tempfile::TempDir, MouseHub) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path(), Arc::new(builtin_registry()));
        let hub = MouseHub::new(store).unwrap();
        (dir, hub)
    }

    fn info(serial: &str, model: MouseModel) -> MouseInfo {
        MouseInfo {
            serial: serial.to_owned(),
            model,
            display_name: model.display_name().to_owned(),
            capabilities: model.capabilities(),
        }
    }

    fn config(serial: &str) -> MouseConfig {
        MouseConfig::default_for(serial, MouseModel::MxMaster4)
    }

    fn register(hub: &MouseHub, serial: &str) -> MouseRegistration {
        hub.register(info(serial, MouseModel::MxMaster4), config(serial))
    }

    #[test]
    fn registered_mice_are_listed_until_their_registration_is_dropped() {
        let (_dir, hub) = hub();
        let changes = hub.subscribe_mice();
        assert_eq!(hub.mice(), []);
        let first = register(&hub, "aaaa0001");
        let second = register(&hub, "aaaa0002");
        assert!(changes.has_changed().unwrap());
        let serials = |hub: &MouseHub| -> Vec<String> {
            hub.mice().into_iter().map(|mouse| mouse.serial).collect()
        };
        assert_eq!(serials(&hub), ["aaaa0001", "aaaa0002"]);
        drop(first);
        assert_eq!(serials(&hub), ["aaaa0002"]);
        drop(second);
        assert_eq!(hub.mice(), []);
    }

    #[test]
    fn an_older_registration_does_not_remove_the_one_that_replaced_it() {
        let (_dir, hub) = hub();
        let older = register(&hub, "aaaa0001");
        let newer = register(&hub, "aaaa0001");
        assert_eq!(hub.mice().len(), 1);
        drop(older);
        assert_eq!(hub.mice().len(), 1, "the replacement is still there");
        drop(newer);
        assert_eq!(hub.mice(), []);
    }

    #[test]
    fn an_update_is_saved_and_given_to_the_followers_and_the_thread() {
        let (_dir, hub) = hub();
        let mut registration = register(&hub, "aaaa0001");
        let mut follower = hub.subscribe_config("aaaa0001").unwrap();
        let mut update = config("aaaa0001");
        update.pointer.dpi = 2400;
        let saved = hub.update_mouse("aaaa0001", update).unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(saved.pointer.dpi, 2400);
        assert_eq!(*follower.borrow_and_update(), saved);
        assert_eq!(*registration.updates().borrow_and_update(), saved);
    }

    #[test]
    fn an_update_is_refused_for_an_unknown_mouse_a_stale_revision_or_a_wrong_serial() {
        let (_dir, hub) = hub();
        let _registration = register(&hub, "aaaa0001");
        assert!(matches!(
            hub.update_mouse("bbbb0001", config("bbbb0001")),
            Err(HubError::UnknownMouse(_))
        ));
        hub.update_mouse("aaaa0001", config("aaaa0001")).unwrap();
        let stale = hub
            .update_mouse("aaaa0001", config("aaaa0001"))
            .unwrap_err();
        assert_eq!(stale.error_code(), ErrorCode::StaleRevision);
        let wrong = hub
            .update_mouse("aaaa0001", config("bbbb0001"))
            .unwrap_err();
        assert_eq!(wrong.error_code(), ErrorCode::InvalidConfig);
        let mut invalid = hub.subscribe_config("aaaa0001").unwrap().borrow().clone();
        invalid.pointer.dpi = 225;
        let invalid = hub.update_mouse("aaaa0001", invalid).unwrap_err();
        assert_eq!(invalid.error_code(), ErrorCode::InvalidConfig);
    }

    #[test]
    fn a_configuration_of_another_model_is_refused_and_changes_nothing() {
        let (dir, hub) = hub();
        let _registration = register(&hub, "aaaa0001");
        let mut follower = hub.subscribe_config("aaaa0001").unwrap();
        follower.borrow_and_update();
        let other = MouseConfig::default_for("aaaa0001", MouseModel::MxAnywhere3S);
        let refused = hub.update_mouse("aaaa0001", other).unwrap_err();
        assert!(matches!(
            refused,
            HubError::Store(ConfigStoreError::ModelMismatch { .. })
        ));
        assert_eq!(refused.error_code(), ErrorCode::InvalidConfig);
        assert!(!follower.has_changed().unwrap(), "nobody was told");
        assert!(
            !dir.path().join("devices/aaaa0001/mouse.json").exists(),
            "nothing was written"
        );
    }

    #[test]
    fn a_shortcut_that_cannot_be_pressed_is_refused_when_it_is_saved() {
        let (dir, hub) = hub();
        let _registration = register(&hub, "aaaa0001");
        let bound = |keys: &str| {
            let mut config = config("aaaa0001");
            config.bindings.insert(
                MouseControlId::Back,
                ButtonAction::Shortcut {
                    keys: keys.to_owned(),
                },
            );
            config
        };
        for keys in [
            "Ctrl+Power",
            "Alt+Left_Arrow",
            "Ctrl+Alt+Delete",
            "a+b+c+d+e+f+g",
        ] {
            let refused = hub.update_mouse("aaaa0001", bound(keys)).unwrap_err();
            assert!(
                matches!(refused, HubError::InvalidShortcut { .. }),
                "{keys}"
            );
            assert_eq!(refused.error_code(), ErrorCode::InvalidConfig);
        }
        assert!(!dir.path().join("devices/aaaa0001/mouse.json").exists());
        let saved = hub.update_mouse("aaaa0001", bound("Ctrl+Shift+M")).unwrap();
        assert_eq!(saved.revision, 1);
    }

    #[test]
    fn successive_updates_leave_the_followers_on_the_highest_revision() {
        let (_dir, hub) = hub();
        let _registration = register(&hub, "aaaa0001");
        let mut follower = hub.subscribe_config("aaaa0001").unwrap();
        let mut current = config("aaaa0001");
        for dpi in [1000, 1200, 1400] {
            current.pointer.dpi = dpi;
            current = hub.update_mouse("aaaa0001", current).unwrap();
        }
        let seen = follower.borrow_and_update().clone();
        assert_eq!((seen.revision, seen.pointer.dpi), (3, 1400));
    }

    #[test]
    fn a_configuration_saved_before_the_registration_is_offered_to_the_mouse() {
        let (_dir, hub) = hub();
        let mut registration = register(&hub, "aaaa0001");
        let mut newer = config("aaaa0001");
        newer.revision = 3;
        newer.pointer.dpi = 2400;
        registration.refresh(newer.clone());
        assert_eq!(*registration.updates().borrow_and_update(), newer);
        // An older one is not taken back.
        registration.refresh(config("aaaa0001"));
        assert!(!registration.updates().has_changed().unwrap());
        assert_eq!(registration.updates().borrow().revision, 3);
    }

    #[test]
    fn a_mouse_that_is_not_connected_cannot_be_followed() {
        let (_dir, hub) = hub();
        assert!(hub.subscribe_config("aaaa0001").is_none());
        let registration = register(&hub, "aaaa0001");
        drop(registration);
        assert!(hub.subscribe_config("aaaa0001").is_none());
    }

    #[test]
    fn the_radial_menu_exists_without_any_mouse_and_updates_reach_the_followers() {
        let (dir, hub) = hub();
        assert!(dir.path().join("radial-menu.json").is_file());
        let mut follower = hub.subscribe_radial_menu();
        let mut menu = follower.borrow_and_update().clone();
        let outdated = menu.clone();
        menu.animate = !menu.animate;
        let saved = hub.update_radial_menu(menu).unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(*follower.borrow_and_update(), saved);
        assert!(matches!(
            hub.update_radial_menu(outdated).unwrap_err(),
            ConfigStoreError::StaleRevision { .. }
        ));
        let mut invalid = saved;
        invalid.hold_ms = 10;
        assert!(matches!(
            hub.update_radial_menu(invalid).unwrap_err(),
            ConfigStoreError::InvalidRadialMenu(_)
        ));
    }

    #[test]
    fn the_radial_menu_is_read_back_by_a_new_hub() {
        let (dir, hub) = hub();
        let mut menu = hub.subscribe_radial_menu().borrow().clone();
        menu.animate = !menu.animate;
        let saved = hub.update_radial_menu(menu).unwrap();
        let store = ConfigStore::new(dir.path(), Arc::new(builtin_registry()));
        let again = MouseHub::new(store).unwrap();
        assert_eq!(*again.subscribe_radial_menu().borrow(), saved);
    }
}
