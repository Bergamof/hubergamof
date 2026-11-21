//! What the mice and the radial menu look like to the web interface: the mice
//! connected, the configuration of each, and the configuration of the menu,
//! each as a `watch` channel so that every client follows every change, and
//! the one place that saves them.
//!
//! The mouse threads ([`crate::mouse_engine`]) register their mouse here for as
//! long as they serve it. The radial menu exists whether or not a mouse is
//! connected.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use hubergamof_protocol::{
    ButtonAction, ErrorCode, HapticLevel, HapticWaveform, MouseBattery, MouseCapabilities,
    MouseConfig, MouseControlId, MouseInfo, MouseProblem, MouseStatus, RadialMenuConfig,
};
use thiserror::Error;
use tokio::sync::watch;

use crate::config_store::{ConfigStore, ConfigStoreError};
use crate::haptics::{HapticOutput, HapticPlay};
use crate::input::{ShortcutError, parse_shortcut};

/// Why a configuration update is refused.
#[derive(Debug, Error)]
pub enum HubError {
    /// No mouse with this serial is connected.
    #[error("no connected mouse with serial `{0}`")]
    UnknownMouse(String),
    /// The mouse has no haptics to try a waveform on.
    #[error("the mouse `{0}` has no haptics")]
    NoHaptics(String),
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
            Self::NoHaptics(_) => ErrorCode::InvalidInput,
            Self::Store(error) => error.error_code(),
            Self::InvalidShortcut { .. } => ErrorCode::InvalidConfig,
        }
    }
}

/// Waveforms waiting for a mouse, at most: the mouse plays one after the
/// other, and a pile means it is stuck or asleep.
const HAPTIC_QUEUE: usize = 8;

/// Shortest time between two plays of the same event (`module.event`): an
/// event that comes in bursts (a counter that wobbles, a module that loops)
/// is felt once.
const HAPTIC_MIN_GAP: Duration = Duration::from_secs(2);

/// Shortest time between two waveforms the interface asks a mouse to play (a
/// button pressed again and again is not a flood on the mouse).
const TRY_MIN_GAP: Duration = Duration::from_millis(600);

struct Entry {
    id: u64,
    info: MouseInfo,
    config: Arc<watch::Sender<MouseConfig>>,
    /// What the mouse refused of its configuration.
    status: Arc<watch::Sender<MouseStatus>>,
    haptics: SyncSender<HapticWaveform>,
    /// The link to the mouse is up: one that is switched off stays registered
    /// (its thread waits for it) but is not listed.
    connected: bool,
}

struct Shared {
    store: ConfigStore,
    entries: Mutex<Vec<Entry>>,
    mice: watch::Sender<Vec<MouseInfo>>,
    radial: watch::Sender<RadialMenuConfig>,
    next_id: AtomicU64,
    /// When each haptic event was last played, by `module.event`.
    last_played: Mutex<HashMap<String, Instant>>,
    /// When the interface last had a waveform played on each mouse, by serial.
    last_tried: Mutex<HashMap<String, Instant>>,
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
                last_played: Mutex::default(),
                last_tried: Mutex::default(),
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
        let mice = entries
            .iter()
            .filter(|entry| entry.connected)
            .map(|entry| entry.info.clone())
            .collect();
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
        let status = Arc::new(watch::channel(MouseStatus::default()).0);
        let (haptic_sender, haptics) = sync_channel(HAPTIC_QUEUE);
        let mut entries = self.entries();
        entries.retain(|entry| entry.info.serial != info.serial);
        entries.push(Entry {
            id,
            info,
            config: Arc::clone(&sender),
            status: Arc::clone(&status),
            haptics: haptic_sender,
            connected: true,
        });
        self.publish_mice(&entries);
        drop(entries);
        MouseRegistration {
            hub: self.clone(),
            id,
            sender,
            status,
            updates,
            haptics,
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

    /// Plays `waveform` on the mouse, for the person to try it: at the strength
    /// set for that mouse (nothing when it is off), and not twice within
    /// [`TRY_MIN_GAP`].
    ///
    /// # Errors
    /// [`HubError::UnknownMouse`] when it is not connected,
    /// [`HubError::NoHaptics`] when it cannot vibrate.
    pub fn try_waveform(&self, serial: &str, waveform: HapticWaveform) -> Result<(), HubError> {
        self.try_waveform_at(serial, waveform, Instant::now())
    }

    fn try_waveform_at(
        &self,
        serial: &str,
        waveform: HapticWaveform,
        now: Instant,
    ) -> Result<(), HubError> {
        let entries = self.entries();
        let entry = entries
            .iter()
            .find(|entry| entry.info.serial == serial)
            .ok_or_else(|| HubError::UnknownMouse(serial.to_owned()))?;
        if !entry.info.capabilities.haptics {
            return Err(HubError::NoHaptics(serial.to_owned()));
        }
        if entry.config.borrow().haptics.level == HapticLevel::Off {
            return Ok(());
        }
        let mut tried = self
            .shared
            .last_tried
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if tried
            .get(serial)
            .is_some_and(|before| now.saturating_duration_since(*before) < TRY_MIN_GAP)
        {
            return Ok(());
        }
        tried.insert(serial.to_owned(), now);
        // A full queue (a stuck mouse) drops it.
        let _ = entry.haptics.try_send(waveform);
        Ok(())
    }

    /// What a connected mouse refused of its configuration, as it changes;
    /// `None` when it is not connected.
    #[must_use]
    pub fn subscribe_status(&self, serial: &str) -> Option<watch::Receiver<MouseStatus>> {
        let entries = self.entries();
        let entry = entries.iter().find(|entry| entry.info.serial == serial)?;
        Some(entry.status.subscribe())
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

/// What a mouse plays for a haptic event of a module: nothing when the mouse
/// cannot, when its haptics are off, or when the user switched that event off;
/// the waveform the user chose for the event, else the one the module declares.
fn waveform_to_play(
    config: &MouseConfig,
    capabilities: &MouseCapabilities,
    play: &HapticPlay,
) -> Option<HapticWaveform> {
    if !capabilities.haptics || config.haptics.level == HapticLevel::Off {
        return None;
    }
    match config.haptics.events.get(&play.key()) {
        Some(setting) => setting.enabled.then_some(setting.waveform),
        None => Some(play.waveform),
    }
}

impl MouseHub {
    /// Whether `play` may be played at `now`: not within [`HAPTIC_MIN_GAP`] of
    /// the last time the same event was. The time is noted when it may.
    fn gap_allows(&self, play: &HapticPlay, now: Instant) -> bool {
        let mut last = self
            .shared
            .last_played
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let key = play.key();
        if last
            .get(&key)
            .is_some_and(|before| now.saturating_duration_since(*before) < HAPTIC_MIN_GAP)
        {
            return false;
        }
        last.insert(key, now);
        true
    }

    /// Signals `play` on the mice, as of `now`.
    fn play_at(&self, play: &HapticPlay, now: Instant) {
        if !self.gap_allows(play, now) {
            tracing::debug!(event = %play.key(), "haptic event repeated too soon: dropped");
            return;
        }
        for entry in self.entries().iter() {
            let config = entry.config.borrow();
            let Some(waveform) = waveform_to_play(&config, &entry.info.capabilities, play) else {
                continue;
            };
            // A mouse that was just released has nobody to signal: nothing to
            // say. A full queue means the mouse is stuck.
            if let Err(TrySendError::Full(_)) = entry.haptics.try_send(waveform) {
                tracing::debug!(event = %play.key(), "haptic queue of a mouse is full: dropped");
            }
        }
    }
}

impl HapticOutput for MouseHub {
    fn play(&self, play: &HapticPlay) {
        self.play_at(play, Instant::now());
    }

    fn play_waveform(&self, waveform: HapticWaveform) {
        for entry in self.entries().iter() {
            let config = entry.config.borrow();
            if !entry.info.capabilities.haptics || config.haptics.level == HapticLevel::Off {
                continue;
            }
            // A full queue (a stuck mouse) drops it; a mouse just released has
            // nobody to signal.
            let _ = entry.haptics.try_send(waveform);
        }
    }
}

/// A mouse listed as connected; it is removed when this is dropped.
pub struct MouseRegistration {
    hub: MouseHub,
    id: u64,
    sender: Arc<watch::Sender<MouseConfig>>,
    status: Arc<watch::Sender<MouseStatus>>,
    updates: watch::Receiver<MouseConfig>,
    haptics: Receiver<HapticWaveform>,
}

impl MouseRegistration {
    /// What reaches the mouse while it is served: the configuration the
    /// interface changes, and the waveforms the events of the modules ask it to
    /// play (already filtered by what the user chose). Both at once, which two
    /// separate borrows of the registration would not allow.
    pub fn streams(&mut self) -> (&mut watch::Receiver<MouseConfig>, &Receiver<HapticWaveform>) {
        (&mut self.updates, &self.haptics)
    }

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

    /// Where the mouse thread says what the mouse refused, for the interface.
    #[must_use]
    pub fn status_reporter(&self) -> StatusReporter {
        StatusReporter(Arc::clone(&self.status))
    }

    /// Where the mouse thread says that the mouse was switched off or is back.
    #[must_use]
    pub fn presence_reporter(&self) -> PresenceReporter {
        PresenceReporter {
            hub: self.hub.clone(),
            id: self.id,
        }
    }

    /// The configuration of the mouse, as the interface changes it.
    pub fn updates(&mut self) -> &mut watch::Receiver<MouseConfig> {
        &mut self.updates
    }
}

/// Tells the interface what is known of a mouse: what it refused of its
/// configuration, and its battery. Cheap to clone.
#[derive(Clone)]
pub struct StatusReporter(Arc<watch::Sender<MouseStatus>>);

impl StatusReporter {
    /// The settings the mouse refused the last time its configuration was
    /// applied (none: it took everything). Followers are told only of a change.
    pub fn report(&self, problems: Vec<MouseProblem>) {
        self.update(|status| &mut status.problems, problems);
    }

    /// The battery as the mouse last said it (`None`: it did not say).
    /// Followers are told only of a change.
    pub fn report_battery(&self, battery: Option<MouseBattery>) {
        self.update(|status| &mut status.battery, battery);
    }

    fn update<T: PartialEq>(&self, field: impl FnOnce(&mut MouseStatus) -> &mut T, value: T) {
        self.0.send_if_modified(|current| {
            let slot = field(current);
            let changed = *slot != value;
            if changed {
                *slot = value;
            }
            changed
        });
    }
}

/// Tells the interface that a mouse is switched off (it leaves the list) or
/// back (it is listed again). Cheap to clone.
#[derive(Clone)]
pub struct PresenceReporter {
    hub: MouseHub,
    id: u64,
}

impl PresenceReporter {
    /// Lists the mouse (`true`) or hides it (`false`); followers are told only
    /// of a change. Nothing when the registration has been replaced or dropped.
    pub fn set_connected(&self, connected: bool) {
        let mut entries = self.hub.entries();
        let Some(entry) = entries.iter_mut().find(|entry| entry.id == self.id) else {
            return;
        };
        if entry.connected != connected {
            entry.connected = connected;
            self.hub.publish_mice(&entries);
        }
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
    use hubergamof_protocol::{HapticEventSetting, MouseModel};

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

    fn step_end() -> HapticPlay {
        HapticPlay {
            module: "pomodoro".into(),
            event: "step-end".to_owned(),
            waveform: HapticWaveform::Completed,
        }
    }

    fn registered_with(hub: &MouseHub, config: MouseConfig) -> MouseRegistration {
        hub.register(info(&config.serial, MouseModel::MxMaster4), config)
    }

    fn played(registration: &MouseRegistration) -> Vec<HapticWaveform> {
        std::iter::from_fn(|| registration.haptics.try_recv().ok()).collect()
    }

    #[test]
    fn an_event_the_user_did_not_configure_is_played_as_its_module_declares() {
        let (_dir, hub) = hub();
        let mouse = register(&hub, "a");
        hub.play(&step_end());
        assert_eq!(played(&mouse), [HapticWaveform::Completed]);
    }

    #[test]
    fn the_waveform_the_user_chose_for_an_event_wins_and_a_disabled_event_is_silent() {
        let (_dir, hub) = hub();
        let mut chosen = config("a");
        chosen.haptics.events.insert(
            "pomodoro.step-end".to_owned(),
            HapticEventSetting {
                enabled: true,
                waveform: HapticWaveform::Jingle,
            },
        );
        let mut silenced = config("b");
        silenced.haptics.events.insert(
            "pomodoro.step-end".to_owned(),
            HapticEventSetting {
                enabled: false,
                waveform: HapticWaveform::Jingle,
            },
        );
        let (first, second) = (
            registered_with(&hub, chosen),
            registered_with(&hub, silenced),
        );
        hub.play(&step_end());
        assert_eq!(played(&first), [HapticWaveform::Jingle]);
        assert_eq!(played(&second), []);
    }

    #[test]
    fn a_mouse_with_its_haptics_off_plays_nothing() {
        let (_dir, hub) = hub();
        let mut off = config("a");
        off.haptics.level = HapticLevel::Off;
        let mouse = registered_with(&hub, off);
        hub.play(&step_end());
        assert_eq!(played(&mouse), []);
    }

    #[test]
    fn a_mouse_that_cannot_play_haptics_is_not_asked_to() {
        let (_dir, hub) = hub();
        let anywhere = hub.register(
            info("c", MouseModel::MxAnywhere3S),
            MouseConfig::default_for("c", MouseModel::MxAnywhere3S),
        );
        hub.play(&step_end());
        assert_eq!(played(&anywhere), []);
    }

    #[test]
    fn every_mouse_that_can_gets_the_event() {
        let (_dir, hub) = hub();
        let (first, second) = (register(&hub, "a"), register(&hub, "b"));
        hub.play(&step_end());
        assert_eq!(played(&first), [HapticWaveform::Completed]);
        assert_eq!(played(&second), [HapticWaveform::Completed]);
    }

    #[test]
    fn a_stuck_mouse_keeps_the_oldest_events_and_drops_the_rest() {
        let (_dir, hub) = hub();
        let mouse = register(&hub, "a");
        let start = Instant::now();
        for turn in 0..HAPTIC_QUEUE + 4 {
            // Spaced apart enough for each to be played.
            let turn = u32::try_from(turn).unwrap();
            hub.play_at(&step_end(), start + HAPTIC_MIN_GAP * turn);
        }
        assert_eq!(played(&mouse).len(), HAPTIC_QUEUE);
    }

    #[test]
    fn what_a_mouse_refused_is_told_only_when_it_changes() {
        let (_dir, hub) = hub();
        let mouse = register(&hub, "a");
        let mut status = hub.subscribe_status("a").unwrap();
        assert_eq!(status.borrow().problems, Vec::new());
        let reporter = mouse.status_reporter();
        let refused = vec![MouseProblem {
            setting: "wheel".to_owned(),
            message: "refused".to_owned(),
        }];
        reporter.report(refused.clone());
        assert!(status.has_changed().unwrap());
        assert_eq!(status.borrow_and_update().problems, refused);
        // The same thing again is not news; the mouse taking it all is.
        reporter.report(refused);
        assert!(!status.has_changed().unwrap());
        reporter.report(Vec::new());
        assert!(status.has_changed().unwrap());
        assert!(hub.subscribe_status("missing").is_none());
    }

    #[test]
    fn the_battery_is_told_only_when_it_changes_and_keeps_what_was_refused() {
        let (_dir, hub) = hub();
        let mouse = register(&hub, "a");
        let mut status = hub.subscribe_status("a").unwrap();
        let reporter = mouse.status_reporter();
        let refused = vec![MouseProblem {
            setting: "wheel".to_owned(),
            message: "refused".to_owned(),
        }];
        reporter.report(refused.clone());
        let battery = MouseBattery {
            percent: 80,
            charging: false,
        };
        reporter.report_battery(Some(battery));
        assert!(status.has_changed().unwrap());
        let known = status.borrow_and_update().clone();
        assert_eq!(known.battery, Some(battery));
        assert_eq!(known.problems, refused);
        reporter.report_battery(Some(battery));
        assert!(!status.has_changed().unwrap());
        reporter.report_battery(None);
        assert!(status.has_changed().unwrap());
    }

    #[test]
    fn a_mouse_switched_off_leaves_the_list_and_comes_back_with_its_registration() {
        let (_dir, hub) = hub();
        let mouse = register(&hub, "a");
        let other = register(&hub, "b");
        let presence = mouse.presence_reporter();
        let mut list = hub.subscribe_mice();
        list.mark_unchanged();
        presence.set_connected(false);
        assert!(list.has_changed().unwrap());
        let serials: Vec<String> = hub.mice().into_iter().map(|info| info.serial).collect();
        assert_eq!(serials, ["b"]);
        // Said again: not news.
        list.borrow_and_update();
        presence.set_connected(false);
        assert!(!list.has_changed().unwrap());
        presence.set_connected(true);
        assert_eq!(hub.mice().len(), 2);
        // A registration that was replaced says nothing any more.
        drop(other);
        let replaced = register(&hub, "a");
        presence.set_connected(false);
        assert_eq!(hub.mice().len(), 1);
        drop(replaced);
    }

    #[test]
    fn a_waveform_tried_from_the_interface_is_played_on_that_mouse_only() {
        let (_dir, hub) = hub();
        let tried = register(&hub, "a");
        let other = register(&hub, "b");
        hub.try_waveform("a", HapticWaveform::Knock).unwrap();
        assert_eq!(played(&tried), [HapticWaveform::Knock]);
        assert_eq!(played(&other), []);
    }

    #[test]
    fn a_waveform_is_not_tried_twice_in_a_moment_but_is_again_after_it() {
        let (_dir, hub) = hub();
        let mouse = register(&hub, "a");
        let start = Instant::now();
        hub.try_waveform_at("a", HapticWaveform::Knock, start)
            .unwrap();
        hub.try_waveform_at("a", HapticWaveform::Wave, start + TRY_MIN_GAP / 2)
            .unwrap();
        hub.try_waveform_at("a", HapticWaveform::Wave, start + TRY_MIN_GAP)
            .unwrap();
        assert_eq!(
            played(&mouse),
            [HapticWaveform::Knock, HapticWaveform::Wave]
        );
    }

    #[test]
    fn nothing_is_tried_when_the_strength_is_off() {
        let (_dir, hub) = hub();
        let mut quiet = config("a");
        quiet.haptics.level = HapticLevel::Off;
        let mouse = registered_with(&hub, quiet);
        hub.try_waveform("a", HapticWaveform::Knock).unwrap();
        assert_eq!(played(&mouse), []);
    }

    #[test]
    fn a_waveform_is_refused_for_a_mouse_that_is_gone_or_cannot_vibrate() {
        let (_dir, hub) = hub();
        let unknown = hub
            .try_waveform("missing", HapticWaveform::Knock)
            .unwrap_err();
        assert_eq!(unknown.error_code(), ErrorCode::UnknownDevice);
        let plain = MouseConfig::default_for("c", MouseModel::MxAnywhere3S);
        let _anywhere = hub.register(info("c", MouseModel::MxAnywhere3S), plain);
        let silent = hub.try_waveform("c", HapticWaveform::Knock).unwrap_err();
        assert_eq!(silent.error_code(), ErrorCode::InvalidInput);
    }

    #[test]
    fn a_waveform_is_played_on_the_mice_that_can_unless_their_haptics_are_off() {
        let (_dir, hub) = hub();
        let on = register(&hub, "a");
        let mut silenced = config("b");
        silenced.haptics.level = HapticLevel::Off;
        let off = registered_with(&hub, silenced);
        let anywhere = hub.register(
            info("c", MouseModel::MxAnywhere3S),
            MouseConfig::default_for("c", MouseModel::MxAnywhere3S),
        );
        hub.play_waveform(HapticWaveform::SubtleCollision);
        hub.play_waveform(HapticWaveform::SubtleCollision);
        // No gap applies: the feel of the menu is not an event of a module.
        assert_eq!(
            played(&on),
            [
                HapticWaveform::SubtleCollision,
                HapticWaveform::SubtleCollision
            ]
        );
        assert_eq!(played(&off), []);
        assert_eq!(played(&anywhere), []);
    }

    #[test]
    fn the_same_event_is_not_played_again_within_the_minimum_gap() {
        let (_dir, hub) = hub();
        let mouse = register(&hub, "a");
        let start = Instant::now();
        hub.play_at(&step_end(), start);
        hub.play_at(&step_end(), start + Duration::from_millis(500));
        hub.play_at(
            &step_end(),
            start
                + HAPTIC_MIN_GAP
                    .checked_sub(Duration::from_millis(1))
                    .unwrap(),
        );
        assert_eq!(played(&mouse), [HapticWaveform::Completed]);
        hub.play_at(&step_end(), start + HAPTIC_MIN_GAP);
        assert_eq!(played(&mouse), [HapticWaveform::Completed]);
    }

    #[test]
    fn two_different_events_do_not_hold_each_other_back() {
        let (_dir, hub) = hub();
        let mouse = register(&hub, "a");
        let now = Instant::now();
        let other = HapticPlay {
            module: "teams".into(),
            event: "incoming-call".to_owned(),
            waveform: HapticWaveform::Ringing,
        };
        hub.play_at(&step_end(), now);
        hub.play_at(&other, now);
        assert_eq!(
            played(&mouse),
            [HapticWaveform::Completed, HapticWaveform::Ringing]
        );
    }

    #[test]
    fn an_event_that_was_dropped_for_a_mouse_without_haptics_still_counts_as_played() {
        // The gap belongs to the event, not to a mouse: a mouse registered a
        // moment later does not get what the others were just given.
        let (_dir, hub) = hub();
        let now = Instant::now();
        hub.play_at(&step_end(), now);
        let late = register(&hub, "late");
        hub.play_at(&step_end(), now + Duration::from_millis(10));
        assert_eq!(played(&late), []);
    }

    #[test]
    fn an_event_for_a_mouse_that_is_gone_is_dropped_without_a_panic() {
        let (_dir, hub) = hub();
        let mouse = register(&hub, "a");
        drop(mouse);
        hub.play(&step_end());
    }

    #[test]
    fn a_mouse_reads_what_is_asked_of_it_from_its_registration() {
        let (_dir, hub) = hub();
        let mut mouse = register(&hub, "a");
        hub.play(&step_end());
        let (_updates, haptics) = mouse.streams();
        assert_eq!(haptics.try_recv().ok(), Some(HapticWaveform::Completed));
        assert!(haptics.try_recv().is_err());
    }
}
