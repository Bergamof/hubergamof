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

/// Shows `seen` unless it is what is shown: a module takes every change for a
/// state of the headphones, which ends what it asked of them.
fn publish(view: &watch::Sender<Option<Headset>>, seen: Option<Headset>) {
    view.send_if_modified(|shown| {
        let changed = *shown != seen;
        *shown = seen;
        changed
    });
}

/// Follows the first headphones of the hub: what they say, as it changes;
/// `None` while there are none.
async fn follow(hub: HeadsetHub, view: watch::Sender<Option<Headset>>) {
    let mut list = hub.subscribe_headsets();
    loop {
        let first = list.borrow_and_update().first().cloned();
        let state = first.and_then(|first| hub.subscribe_state(&first.address));
        let Some(mut state) = state else {
            publish(&view, None);
            if list.changed().await.is_err() {
                return;
            }
            continue;
        };
        loop {
            publish(&view, Some(headset(*state.borrow_and_update())));
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
    use tokio::sync::mpsc;
    use tokio::time::timeout;

    use super::*;
    use crate::headset_hub::{Action, Request};

    /// Longest a test waits for what it expects: only reached when it fails.
    const WAIT: Duration = Duration::from_secs(5);

    fn info(address: &str) -> HeadsetInfo {
        HeadsetInfo {
            address: address.into(),
            name: "XM4".into(),
        }
    }

    fn say(hub: &HeadsetHub, address: &str, link: HeadsetLink, ambient: AmbientSound) {
        hub.update_state(address, |state| {
            state.link = link;
            state.ambient = Some(ambient);
            state.wind_reduction = true;
        });
    }

    fn sound(mode: AmbientMode, level: u8) -> AmbientSound {
        AmbientSound {
            mode,
            level,
            focus_on_voice: true,
        }
    }

    /// Waits until the view shows what `matches` accepts.
    async fn seen(
        view: &mut watch::Receiver<Option<Headset>>,
        matches: impl Fn(&Headset) -> bool,
    ) -> Headset {
        timeout(
            WAIT,
            view.wait_for(|seen| seen.as_ref().is_some_and(&matches)),
        )
        .await
        .expect("the view did not show it")
        .unwrap()
        .clone()
        .unwrap()
    }

    async fn request(requests: &mut mpsc::Receiver<Request>) -> Request {
        timeout(WAIT, requests.recv()).await.unwrap().unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn it_follows_the_first_headphones_and_their_departure() {
        let (hub, _requests) = HeadsetHub::new();
        let control = HubControl::start(hub.clone());
        let mut view = control.watch();
        assert_eq!(*view.borrow(), None);

        hub.present(vec![info("AA"), info("BB")]);
        say(
            &hub,
            "AA",
            HeadsetLink::Connected,
            sound(AmbientMode::Ambient, 6),
        );
        let first = seen(&mut view, |seen| seen.ambient.is_some()).await;
        assert_eq!(first.link, Link::Connected);
        assert_eq!(
            first.ambient,
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

    #[tokio::test(start_paused = true)]
    async fn it_moves_to_the_next_headphones_when_the_first_leave() {
        let (hub, _requests) = HeadsetHub::new();
        let control = HubControl::start(hub.clone());
        let mut view = control.watch();
        hub.present(vec![info("AA"), info("BB")]);
        say(
            &hub,
            "AA",
            HeadsetLink::Connected,
            sound(AmbientMode::Ambient, 1),
        );
        say(
            &hub,
            "BB",
            HeadsetLink::Connected,
            sound(AmbientMode::Off, 2),
        );
        // The view really showed the first before they left.
        seen(&mut view, |seen| {
            seen.ambient.map(|ambient| ambient.mode) == Some(Mode::Ambient)
        })
        .await;

        hub.present(vec![info("BB")]);
        let second = seen(&mut view, |seen| {
            seen.ambient.map(|ambient| ambient.mode) == Some(Mode::Off)
        })
        .await;
        assert_eq!(second.ambient.map(|ambient| ambient.level), Some(2));
    }

    #[tokio::test(start_paused = true)]
    async fn it_follows_what_the_first_headphones_say_while_the_list_stays() {
        let (hub, _requests) = HeadsetHub::new();
        let control = HubControl::start(hub.clone());
        let mut view = control.watch();
        hub.present(vec![info("AA")]);
        say(
            &hub,
            "AA",
            HeadsetLink::Connected,
            sound(AmbientMode::Off, 0),
        );
        seen(&mut view, |seen| seen.link == Link::Connected).await;
        say(&hub, "AA", HeadsetLink::Cut, sound(AmbientMode::Off, 0));
        seen(&mut view, |seen| seen.link == Link::Cut).await;
    }

    #[tokio::test(start_paused = true)]
    async fn an_unchanged_state_is_not_shown_again() {
        let (hub, _requests) = HeadsetHub::new();
        let control = HubControl::start(hub.clone());
        let mut view = control.watch();
        hub.present(vec![info("AA")]);
        say(
            &hub,
            "AA",
            HeadsetLink::Connected,
            sound(AmbientMode::Off, 0),
        );
        seen(&mut view, |seen| seen.link == Link::Connected).await;
        view.borrow_and_update();

        // A second headset joining changes the list, not what the first says.
        hub.present(vec![info("AA"), info("BB")]);
        assert!(
            timeout(Duration::from_secs(1), view.changed())
                .await
                .is_err()
        );
    }

    #[test]
    fn every_link_and_mode_is_turned_into_its_own() {
        for (wire, own) in [
            (HeadsetLink::Idle, Link::Idle),
            (HeadsetLink::Connecting, Link::Connecting),
            (HeadsetLink::Connected, Link::Connected),
            (HeadsetLink::Released, Link::Released),
            (HeadsetLink::Cut, Link::Cut),
        ] {
            assert_eq!(link(wire), own);
        }
        for (wire, own) in [
            (AmbientMode::Off, Mode::Off),
            (AmbientMode::NoiseCancelling, Mode::NoiseCancelling),
            (AmbientMode::Ambient, Mode::Ambient),
            (AmbientMode::WindReduction, Mode::WindReduction),
        ] {
            assert_eq!(mode(wire), own);
            assert_eq!(wire_mode(own), wire);
        }
    }

    #[tokio::test(start_paused = true)]
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
        say(
            &hub,
            "AA",
            HeadsetLink::Connected,
            sound(AmbientMode::Off, 0),
        );
        assert_eq!(control.set_ambient(ambient), Ok(()));
        let sent = request(&mut requests).await;
        assert_eq!(sent.address, "AA");
        assert_eq!(
            sent.action,
            Action::SetAmbient(AmbientSound {
                mode: AmbientMode::NoiseCancelling,
                level: 3,
                focus_on_voice: false
            })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn what_the_headphones_would_not_take_is_refused() {
        let (hub, _requests) = HeadsetHub::new();
        let control = HubControl::start(hub.clone());
        hub.present(vec![info("AA")]);
        hub.update_state("AA", |state| {
            state.link = HeadsetLink::Connected;
            state.wind_reduction = false;
        });
        let wind = Ambient {
            mode: Mode::WindReduction,
            level: 0,
            focus_on_voice: false,
        };
        assert_eq!(control.set_ambient(wind), Err(HeadsetFault::Refused));
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_queue_is_refused_and_a_departed_headset_is_not_linked() {
        let (hub, _requests) = HeadsetHub::new();
        let control = HubControl::start(hub.clone());
        hub.present(vec![info("AA")]);
        say(
            &hub,
            "AA",
            HeadsetLink::Connected,
            sound(AmbientMode::Off, 0),
        );
        let ambient = Ambient {
            mode: Mode::Ambient,
            level: 1,
            focus_on_voice: false,
        };
        // Nobody takes the requests: the queue fills up.
        let outcomes: Vec<_> = (0..200).map(|_| control.set_ambient(ambient)).collect();
        assert!(outcomes.contains(&Err(HeadsetFault::Refused)));
        assert_eq!(outcomes[0], Ok(()));

        hub.present(Vec::new());
        assert_eq!(control.set_ambient(ambient), Err(HeadsetFault::NotLinked));
    }

    #[tokio::test(start_paused = true)]
    async fn opening_asks_the_hub_to_open_the_first_headphones_only_when_there_are_some() {
        let (hub, mut requests) = HeadsetHub::new();
        let control = HubControl::start(hub.clone());
        control.open();
        assert!(requests.try_recv().is_err());
        hub.present(vec![info("AA")]);
        control.open();
        let sent = request(&mut requests).await;
        assert_eq!((sent.address.as_str(), sent.action), ("AA", Action::Open));
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_the_control_stops_following() {
        let (hub, _requests) = HeadsetHub::new();
        let control = HubControl::start(hub.clone());
        let mut view = control.watch();
        let follower = control.follower.abort_handle();
        drop(control);
        timeout(WAIT, async {
            while !follower.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // Nothing publishes any more: the view is closed, not waiting.
        assert!(timeout(WAIT, view.changed()).await.unwrap().is_err());
    }
}
