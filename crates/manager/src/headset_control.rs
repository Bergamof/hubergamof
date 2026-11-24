//! The headphones module's view of the headphones: the first of those the hub
//! lists, turned into the module's own types. The module crate knows nothing
//! of the wire contract; every `match` lists its variants, so that a new one
//! fails to compile until it is handled.

use std::sync::Arc;

use hubergamof_modules::headphones::{Ambient, Headset, HeadsetControl, HeadsetFault, Link, Mode};
use hubergamof_protocol::{AmbientMode, AmbientSound, HeadsetLink, HeadsetState};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::headset_hub::{HeadsetHub, HubError};

fn mode(mode: AmbientMode) -> Mode {
    match mode {
        AmbientMode::Off => Mode::Off,
        AmbientMode::NoiseCancelling => Mode::NoiseCancelling,
        AmbientMode::Ambient => Mode::Ambient,
        AmbientMode::WindReduction => Mode::WindReduction,
    }
}

fn wire_mode(mode: Mode) -> AmbientMode {
    match mode {
        Mode::Off => AmbientMode::Off,
        Mode::NoiseCancelling => AmbientMode::NoiseCancelling,
        Mode::Ambient => AmbientMode::Ambient,
        Mode::WindReduction => AmbientMode::WindReduction,
    }
}

fn link(link: HeadsetLink) -> Link {
    match link {
        HeadsetLink::Idle => Link::Idle,
        HeadsetLink::Connecting => Link::Connecting,
        HeadsetLink::Connected => Link::Connected,
        HeadsetLink::Released => Link::Released,
        HeadsetLink::Cut => Link::Cut,
    }
}

fn headset(state: HeadsetState) -> Headset {
    Headset {
        link: link(state.link),
        ambient: state.ambient.map(|ambient| Ambient {
            mode: mode(ambient.mode),
            level: ambient.level,
            focus_on_voice: ambient.focus_on_voice,
        }),
        wind_reduction: state.wind_reduction,
    }
}

/// Follows the first headphones of the hub: what they say, as it changes;
/// `None` while there are none.
async fn follow(hub: HeadsetHub, view: watch::Sender<Option<Headset>>) {
    let mut list = hub.subscribe_headsets();
    loop {
        let first = list.borrow_and_update().first().cloned();
        let state = first.and_then(|first| hub.subscribe_state(&first.address));
        let Some(mut state) = state else {
            view.send_replace(None);
            if list.changed().await.is_err() {
                return;
            }
            continue;
        };
        loop {
            view.send_replace(Some(headset(*state.borrow_and_update())));
            tokio::select! {
                gone = state.changed() => if gone.is_err() { break },
                changed = list.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    break;
                }
            }
        }
    }
}

/// The headphones of the hub, for the headphones module.
pub struct HubControl {
    hub: HeadsetHub,
    view: watch::Sender<Option<Headset>>,
    follower: JoinHandle<()>,
}

impl HubControl {
    /// Starts following `hub`; needs a Tokio runtime.
    #[must_use]
    pub fn start(hub: HeadsetHub) -> Arc<Self> {
        let view = watch::channel(None).0;
        let follower = tokio::spawn(follow(hub.clone(), view.clone()));
        Arc::new(Self {
            hub,
            view,
            follower,
        })
    }

    fn address(&self) -> Option<String> {
        self.hub
            .headsets()
            .first()
            .map(|first| first.address.clone())
    }
}

impl std::fmt::Debug for HubControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubControl").finish_non_exhaustive()
    }
}

impl Drop for HubControl {
    fn drop(&mut self) {
        self.follower.abort();
    }
}

impl HeadsetControl for HubControl {
    fn watch(&self) -> watch::Receiver<Option<Headset>> {
        self.view.subscribe()
    }

    fn set_ambient(&self, ambient: Ambient) -> Result<(), HeadsetFault> {
        let address = self.address().ok_or(HeadsetFault::NotLinked)?;
        let ambient = AmbientSound {
            mode: wire_mode(ambient.mode),
            level: ambient.level,
            focus_on_voice: ambient.focus_on_voice,
        };
        self.hub
            .set_ambient(&address, ambient)
            .map_err(|error| match error {
                HubError::UnknownHeadset(_) | HubError::NotLinked(_) => HeadsetFault::NotLinked,
                HubError::Invalid(_) | HubError::Busy => HeadsetFault::Refused,
            })
    }

    fn open(&self) {
        if let Some(address) = self.address() {
            // Idle only: a link closed on purpose or cut stays as it is.
            let _ = self.hub.select(&address);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hubergamof_protocol::HeadsetInfo;
    use tokio::time::timeout;

    use super::*;
    use crate::headset_hub::Action;

    const WAIT: Duration = Duration::from_secs(5);

    fn info(address: &str) -> HeadsetInfo {
        HeadsetInfo {
            address: address.into(),
            name: "XM4".into(),
        }
    }

    fn connected(hub: &HeadsetHub, address: &str, mode: AmbientMode) {
        hub.update_state(address, |state| {
            state.link = HeadsetLink::Connected;
            state.ambient = Some(AmbientSound {
                mode,
                level: 6,
                focus_on_voice: true,
            });
        });
    }

    #[tokio::test]
    async fn it_follows_the_first_headphones_and_their_departure() {
        let (hub, _requests) = HeadsetHub::new();
        let control = HubControl::start(hub.clone());
        let mut view = control.watch();
        assert_eq!(*view.borrow(), None);

        hub.present(vec![info("AA"), info("BB")]);
        connected(&hub, "AA", AmbientMode::Ambient);
        let seen = timeout(
            WAIT,
            view.wait_for(|seen| seen.as_ref().is_some_and(|seen| seen.ambient.is_some())),
        )
        .await
        .unwrap()
        .unwrap()
        .clone()
        .unwrap();
        assert_eq!(seen.link, Link::Connected);
        assert_eq!(
            seen.ambient,
            Some(Ambient {
                mode: Mode::Ambient,
                level: 6,
                focus_on_voice: true
            })
        );

        hub.present(Vec::new());
        timeout(WAIT, view.wait_for(Option::is_none))
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn it_moves_to_the_next_headphones_when_the_first_leave() {
        let (hub, _requests) = HeadsetHub::new();
        let control = HubControl::start(hub.clone());
        let mut view = control.watch();
        hub.present(vec![info("AA"), info("BB")]);
        connected(&hub, "BB", AmbientMode::Off);
        hub.present(vec![info("BB")]);
        let seen = timeout(
            WAIT,
            view.wait_for(|seen| seen.as_ref().is_some_and(|seen| seen.ambient.is_some())),
        )
        .await
        .unwrap()
        .unwrap()
        .clone()
        .unwrap();
        assert_eq!(seen.ambient.map(|ambient| ambient.mode), Some(Mode::Off));
    }

    #[tokio::test]
    async fn requests_go_to_the_first_headphones_once_linked() {
        let (hub, mut requests) = HeadsetHub::new();
        let control = HubControl::start(hub.clone());
        let ambient = Ambient {
            mode: Mode::NoiseCancelling,
            level: 3,
            focus_on_voice: false,
        };
        assert_eq!(control.set_ambient(ambient), Err(HeadsetFault::NotLinked));

        hub.present(vec![info("AA")]);
        assert_eq!(control.set_ambient(ambient), Err(HeadsetFault::NotLinked));
        connected(&hub, "AA", AmbientMode::Off);
        assert_eq!(control.set_ambient(ambient), Ok(()));
        let request = requests.recv().await.unwrap();
        assert_eq!(request.address, "AA");
        assert_eq!(
            request.action,
            Action::SetAmbient(AmbientSound {
                mode: AmbientMode::NoiseCancelling,
                level: 3,
                focus_on_voice: false
            })
        );
    }

    #[tokio::test]
    async fn opening_asks_the_hub_to_open_the_first_headphones_only_when_there_are_some() {
        let (hub, mut requests) = HeadsetHub::new();
        let control = HubControl::start(hub.clone());
        control.open();
        assert!(requests.try_recv().is_err());
        hub.present(vec![info("AA")]);
        control.open();
        let request = requests.recv().await.unwrap();
        assert_eq!(
            (request.address.as_str(), request.action),
            ("AA", Action::Open)
        );
    }
}
