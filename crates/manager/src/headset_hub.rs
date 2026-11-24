//! What the Sony headphones look like to the web interface: the headphones
//! connected to this computer, what is known of each, and the requests the
//! interface makes of them, each state a `watch` channel so that every client
//! follows every change.
//!
//! The engine ([`crate::headset_engine`]) keeps the list and the states up to
//! date and carries out the requests; this hub checks them first.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use hubergamof_protocol::{
    AmbientSound, EqualizerChange, ErrorCode, HeadsetInfo, HeadsetLink, HeadsetRequestError,
    HeadsetState,
};
use thiserror::Error;
use tokio::sync::{mpsc, watch};

/// Why a request for the headphones is refused.
#[derive(Debug, Error)]
pub enum HubError {
    /// These headphones are not connected to this computer.
    #[error("no connected headphones with address `{0}`")]
    UnknownHeadset(String),
    /// The link to the settings channel is not open: nothing can be changed.
    #[error("the link to the headphones `{0}` is not open")]
    NotLinked(String),
    /// The headphones would not accept this.
    #[error(transparent)]
    Invalid(#[from] HeadsetRequestError),
    /// Too many requests are waiting for the engine: asking again later may
    /// work.
    #[error("too many requests are waiting")]
    Busy,
}

impl HubError {
    /// Error code reported to the UI.
    #[must_use]
    pub fn error_code(&self) -> ErrorCode {
        match self {
            Self::UnknownHeadset(_) => ErrorCode::UnknownDevice,
            Self::NotLinked(_) => ErrorCode::Unavailable,
            Self::Invalid(_) => ErrorCode::InvalidInput,
            Self::Busy => ErrorCode::Busy,
        }
    }
}

/// What the engine is asked to do with the headphones at `address`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Bluetooth address of the headphones.
    pub address: String,
    /// What to do.
    pub action: Action,
}

/// What can be asked of the headphones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Open the link if it is not open and was never closed (the person
    /// selected the headphones).
    Open,
    /// Close the link on purpose.
    Release,
    /// Open the link again after it was released or cut. Ignored while the
    /// link is open or being opened, and until a release has ended it.
    Resume,
    /// Change the ambient sound.
    SetAmbient(AmbientSound),
    /// Change the equalizer.
    SetEqualizer(EqualizerChange),
}

/// Requests waiting for the engine at most: it carries them out one after the
/// other, and a pile means a client that floods it.
const REQUEST_QUEUE: usize = 64;

struct Entry {
    info: HeadsetInfo,
    state: Arc<watch::Sender<HeadsetState>>,
}

struct Shared {
    entries: Mutex<Vec<Entry>>,
    headsets: watch::Sender<Vec<HeadsetInfo>>,
    requests: mpsc::Sender<Request>,
}

/// The headphones, for the server and the engine. Cheap to clone.
#[derive(Clone)]
pub struct HeadsetHub {
    shared: Arc<Shared>,
}

impl HeadsetHub {
    /// An empty hub, and the requests the engine is to carry out.
    #[must_use]
    pub fn new() -> (Self, mpsc::Receiver<Request>) {
        let (requests, receiver) = mpsc::channel(REQUEST_QUEUE);
        let hub = Self {
            shared: Arc::new(Shared {
                entries: Mutex::default(),
                headsets: watch::channel(Vec::new()).0,
                requests,
            }),
        };
        (hub, receiver)
    }

    fn entries(&self) -> MutexGuard<'_, Vec<Entry>> {
        // Plain data: nothing to recover from a poisoned lock.
        self.shared
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The headphones connected now.
    #[must_use]
    pub fn headsets(&self) -> Vec<HeadsetInfo> {
        self.shared.headsets.borrow().clone()
    }

    /// The list of connected headphones, as it changes.
    #[must_use]
    pub fn subscribe_headsets(&self) -> watch::Receiver<Vec<HeadsetInfo>> {
        self.shared.headsets.subscribe()
    }

    /// What is known of the headphones at `address`, as it changes; `None`
    /// when they are not connected.
    #[must_use]
    pub fn subscribe_state(&self, address: &str) -> Option<watch::Receiver<HeadsetState>> {
        let entries = self.entries();
        let entry = entries.iter().find(|entry| entry.info.address == address)?;
        Some(entry.state.subscribe())
    }

    /// What is known of the headphones at `address` now.
    #[must_use]
    pub fn state(&self, address: &str) -> Option<HeadsetState> {
        self.subscribe_state(address).map(|state| *state.borrow())
    }

    /// Makes the list of connected headphones `present`: the new ones are
    /// listed with their link idle, those that went away are forgotten with
    /// what was known of them. Headphones are the same when their address is:
    /// one that BlueZ renames keeps what is known of it.
    pub fn present(&self, present: Vec<HeadsetInfo>) {
        let mut entries = self.entries();
        entries.retain(|entry| {
            present
                .iter()
                .any(|info| info.address == entry.info.address)
        });
        for info in present {
            match entries
                .iter_mut()
                .find(|entry| entry.info.address == info.address)
            {
                Some(entry) => entry.info.name = info.name,
                None => entries.push(Entry {
                    info,
                    state: Arc::new(watch::channel(HeadsetState::unknown(HeadsetLink::Idle)).0),
                }),
            }
        }
        let listed: Vec<HeadsetInfo> = entries.iter().map(|entry| entry.info.clone()).collect();
        drop(entries);
        self.shared.headsets.send_if_modified(|current| {
            let changed = *current != listed;
            if changed {
                *current = listed;
            }
            changed
        });
    }

    /// Changes what is known of the headphones at `address`; nothing when
    /// they are gone. Followers are told only if something changed.
    pub fn update_state(&self, address: &str, change: impl FnOnce(&mut HeadsetState)) {
        let entries = self.entries();
        let Some(entry) = entries.iter().find(|entry| entry.info.address == address) else {
            return;
        };
        let state = Arc::clone(&entry.state);
        drop(entries);
        state.send_if_modified(|current| {
            let before = *current;
            change(current);
            *current != before
        });
    }

    /// The person selected the headphones: their link is opened if it is
    /// idle.
    ///
    /// # Errors
    /// [`HubError::UnknownHeadset`] when they are not connected.
    pub fn select(&self, address: &str) -> Result<(), HubError> {
        self.send(address, Action::Open)
    }

    /// Closes the link on purpose.
    ///
    /// # Errors
    /// [`HubError::UnknownHeadset`] when they are not connected.
    pub fn release(&self, address: &str) -> Result<(), HubError> {
        self.send(address, Action::Release)
    }

    /// Opens the link again.
    ///
    /// # Errors
    /// [`HubError::UnknownHeadset`] when they are not connected.
    pub fn resume(&self, address: &str) -> Result<(), HubError> {
        self.send(address, Action::Resume)
    }

    /// Changes the ambient sound.
    ///
    /// # Errors
    /// [`HubError`] when the headphones are not connected, the link is not
    /// open, or they would not accept it.
    pub fn set_ambient(&self, address: &str, ambient: AmbientSound) -> Result<(), HubError> {
        let state = self.linked_state(address)?;
        ambient.validate(state.wind_reduction)?;
        self.send(address, Action::SetAmbient(ambient))
    }

    /// Changes the equalizer.
    ///
    /// # Errors
    /// [`HubError`] when the headphones are not connected, the link is not
    /// open, or they would not accept it.
    pub fn set_equalizer(&self, address: &str, change: EqualizerChange) -> Result<(), HubError> {
        self.linked_state(address)?;
        change.validate()?;
        self.send(address, Action::SetEqualizer(change))
    }

    fn linked_state(&self, address: &str) -> Result<HeadsetState, HubError> {
        let state = self
            .state(address)
            .ok_or_else(|| HubError::UnknownHeadset(address.to_owned()))?;
        if state.link == HeadsetLink::Connected {
            Ok(state)
        } else {
            Err(HubError::NotLinked(address.to_owned()))
        }
    }

    fn send(&self, address: &str, action: Action) -> Result<(), HubError> {
        if self.state(address).is_none() {
            return Err(HubError::UnknownHeadset(address.to_owned()));
        }
        let request = Request {
            address: address.to_owned(),
            action,
        };
        match self.shared.requests.try_send(request) {
            Err(mpsc::error::TrySendError::Full(_)) => Err(HubError::Busy),
            // The engine is gone only at shutdown: nothing more to do then.
            Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use hubergamof_protocol::{AmbientMode, EqualizerPreset};

    use super::*;

    const ADDRESS: &str = "AC:80:0A:0B:2E:61";

    fn info(address: &str) -> HeadsetInfo {
        HeadsetInfo {
            address: address.into(),
            name: "WH-1000XM4".into(),
        }
    }

    fn hub_with_headset() -> (HeadsetHub, mpsc::Receiver<Request>) {
        let (hub, requests) = HeadsetHub::new();
        hub.present(vec![info(ADDRESS)]);
        (hub, requests)
    }

    fn linked(hub: &HeadsetHub, wind_reduction: bool) {
        hub.update_state(ADDRESS, |state| {
            state.link = HeadsetLink::Connected;
            state.wind_reduction = wind_reduction;
        });
    }

    fn ambient(mode: AmbientMode, level: u8) -> AmbientSound {
        AmbientSound {
            mode,
            level,
            focus_on_voice: false,
        }
    }

    #[test]
    fn headphones_that_appear_are_listed_idle_and_those_that_leave_are_forgotten() {
        let (hub, _requests) = HeadsetHub::new();
        assert_eq!(hub.headsets(), Vec::<HeadsetInfo>::new());
        hub.present(vec![info(ADDRESS)]);
        assert_eq!(hub.headsets(), [info(ADDRESS)]);
        assert_eq!(hub.state(ADDRESS).unwrap().link, HeadsetLink::Idle);

        hub.present(vec![]);
        assert_eq!(hub.headsets(), Vec::<HeadsetInfo>::new());
        assert!(hub.state(ADDRESS).is_none());
        assert!(hub.subscribe_state(ADDRESS).is_none());
    }

    #[test]
    fn what_is_known_survives_a_new_listing_that_still_holds_the_headphones() {
        let (hub, _requests) = hub_with_headset();
        linked(&hub, true);
        hub.present(vec![info(ADDRESS)]);
        assert_eq!(hub.state(ADDRESS).unwrap().link, HeadsetLink::Connected);
    }

    #[test]
    fn headphones_renamed_while_connected_keep_what_is_known_of_them() {
        let (hub, _requests) = hub_with_headset();
        linked(&hub, true);
        let mut renamed = info(ADDRESS);
        renamed.name = "Mes écouteurs".into();
        hub.present(vec![renamed.clone()]);
        assert_eq!(hub.headsets(), [renamed]);
        let state = hub.state(ADDRESS).unwrap();
        assert_eq!(state.link, HeadsetLink::Connected);
        assert!(state.wind_reduction);
    }

    #[test]
    fn a_pile_of_requests_is_refused_as_busy_until_the_engine_catches_up() {
        let (hub, mut requests) = hub_with_headset();
        for _ in 0..REQUEST_QUEUE {
            hub.select(ADDRESS).unwrap();
        }
        let error = hub.select(ADDRESS).unwrap_err();
        assert!(matches!(error, HubError::Busy), "{error}");
        assert_eq!(error.error_code(), ErrorCode::Busy);
        requests.try_recv().unwrap();
        hub.select(ADDRESS).unwrap();
    }

    #[test]
    fn followers_are_told_only_of_real_changes() {
        let (hub, _requests) = hub_with_headset();
        let mut list = hub.subscribe_headsets();
        let mut state = hub.subscribe_state(ADDRESS).unwrap();
        list.mark_unchanged();
        state.mark_unchanged();

        hub.present(vec![info(ADDRESS)]);
        hub.update_state(ADDRESS, |state| state.link = HeadsetLink::Idle);
        assert!(!list.has_changed().unwrap());
        assert!(!state.has_changed().unwrap());

        hub.update_state(ADDRESS, |state| state.link = HeadsetLink::Connecting);
        assert!(state.has_changed().unwrap());
        assert_eq!(state.borrow().link, HeadsetLink::Connecting);
    }

    #[test]
    fn selecting_releasing_and_resuming_are_queued_for_the_engine() {
        let (hub, mut requests) = hub_with_headset();
        hub.select(ADDRESS).unwrap();
        hub.release(ADDRESS).unwrap();
        hub.resume(ADDRESS).unwrap();
        let actions: Vec<Action> = std::iter::from_fn(|| requests.try_recv().ok())
            .map(|request| {
                assert_eq!(request.address, ADDRESS);
                request.action
            })
            .collect();
        assert_eq!(actions, [Action::Open, Action::Release, Action::Resume]);
    }

    #[test]
    fn headphones_that_are_not_connected_are_refused() {
        let (hub, mut requests) = hub_with_headset();
        let unknown = "00:00:00:00:00:00";
        for result in [
            hub.select(unknown),
            hub.release(unknown),
            hub.resume(unknown),
            hub.set_ambient(unknown, ambient(AmbientMode::Off, 0)),
            hub.set_equalizer(
                unknown,
                EqualizerChange::Preset {
                    preset: EqualizerPreset::Speech,
                },
            ),
        ] {
            let error = result.unwrap_err();
            assert!(matches!(error, HubError::UnknownHeadset(_)), "{error}");
            assert_eq!(error.error_code(), ErrorCode::UnknownDevice);
        }
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn nothing_is_changed_while_the_link_is_not_open() {
        let (hub, mut requests) = hub_with_headset();
        for link in [
            HeadsetLink::Idle,
            HeadsetLink::Connecting,
            HeadsetLink::Released,
            HeadsetLink::Cut,
        ] {
            hub.update_state(ADDRESS, |state| state.link = link);
            let error = hub
                .set_ambient(ADDRESS, ambient(AmbientMode::Off, 0))
                .unwrap_err();
            assert!(matches!(error, HubError::NotLinked(_)), "{link:?}");
            assert_eq!(error.error_code(), ErrorCode::Unavailable);
        }
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn a_change_the_headphones_would_refuse_is_not_sent() {
        let (hub, mut requests) = hub_with_headset();
        linked(&hub, false);
        for error in [
            hub.set_ambient(ADDRESS, ambient(AmbientMode::Ambient, 21))
                .unwrap_err(),
            hub.set_ambient(ADDRESS, ambient(AmbientMode::WindReduction, 0))
                .unwrap_err(),
            hub.set_equalizer(
                ADDRESS,
                EqualizerChange::Manual {
                    clear_bass: 11,
                    bands: [0; 5],
                },
            )
            .unwrap_err(),
            hub.set_equalizer(
                ADDRESS,
                EqualizerChange::Preset {
                    preset: EqualizerPreset::Manual,
                },
            )
            .unwrap_err(),
        ] {
            assert!(matches!(error, HubError::Invalid(_)), "{error}");
            assert_eq!(error.error_code(), ErrorCode::InvalidInput);
        }
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn a_valid_change_is_queued_and_wind_reduction_needs_the_capability() {
        let (hub, mut requests) = hub_with_headset();
        linked(&hub, true);
        let wind = ambient(AmbientMode::WindReduction, 0);
        hub.set_ambient(ADDRESS, wind).unwrap();
        let change = EqualizerChange::Manual {
            clear_bass: -10,
            bands: [10, -10, 0, 1, 2],
        };
        hub.set_equalizer(ADDRESS, change).unwrap();
        assert_eq!(
            requests.try_recv().unwrap().action,
            Action::SetAmbient(wind)
        );
        assert_eq!(
            requests.try_recv().unwrap().action,
            Action::SetEqualizer(change)
        );
    }
}
