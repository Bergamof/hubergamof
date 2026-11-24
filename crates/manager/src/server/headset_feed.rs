//! What a WebSocket client follows about the headphones: the list of those
//! connected and what is known of the one it selected.

use hubergamof_protocol::{
    AmbientSound, EqualizerChange, ErrorCode, HeadsetInfo, HeadsetState, ServerMessage,
};
use tokio::sync::watch;

use super::errors::client_message;
use super::mouse_feed::Failure;
use crate::headset_hub::{HeadsetHub, HubError};

fn failure(code: ErrorCode, detail: impl Into<String>) -> Failure {
    Failure {
        code,
        message: client_message(code, detail, "headphones request"),
    }
}

impl From<HubError> for Failure {
    fn from(error: HubError) -> Self {
        failure(error.error_code(), error.to_string())
    }
}

/// The headphones a client follows.
struct Selected {
    address: String,
    state: watch::Receiver<HeadsetState>,
}

/// One client's view of the headphones. Without a hub (the manager does not
/// serve headphones) it tells nothing and refuses every request.
pub(super) struct HeadsetFeed {
    hub: Option<HeadsetHub>,
    headsets: Option<watch::Receiver<Vec<HeadsetInfo>>>,
    selected: Option<Selected>,
}

impl HeadsetFeed {
    pub(super) fn new(hub: Option<&HeadsetHub>) -> Self {
        Self {
            hub: hub.cloned(),
            headsets: hub.map(HeadsetHub::subscribe_headsets),
            selected: None,
        }
    }

    /// What a new client is told: the headphones connected. Nothing when
    /// headphones are not served.
    pub(super) fn initial(&mut self) -> Option<ServerMessage> {
        self.headsets.as_mut().map(headsets_message)
    }

    /// Waits for the next change of what the client follows. `None` when the
    /// headphones it selected went away (the new list tells it). Never ready
    /// when headphones are not served.
    pub(super) async fn next_change(&mut self) -> Option<ServerMessage> {
        let Some(headsets) = self.headsets.as_mut() else {
            return std::future::pending().await;
        };
        tokio::select! {
            Ok(()) = headsets.changed() => Some(headsets_message(headsets)),
            change = selected_change(&mut self.selected) => change,
        }
    }

    fn hub(&self) -> Result<&HeadsetHub, Failure> {
        self.hub.as_ref().ok_or_else(|| {
            failure(
                ErrorCode::Unavailable,
                "this manager does not serve headphones",
            )
        })
    }

    /// Follows connected headphones instead of the previous ones, and opens
    /// the link to their settings channel if it is not open; answers with
    /// what is known of them.
    pub(super) fn select(&mut self, address: &str) -> Result<Vec<ServerMessage>, Failure> {
        self.selected = None;
        let hub = self.hub()?;
        let Some(mut state) = hub.subscribe_state(address) else {
            return Err(failure(
                ErrorCode::UnknownDevice,
                "no connected headphones with this address",
            ));
        };
        // Followed before the link is asked for: nothing it does is missed.
        hub.select(address)?;
        let message = state_message(address, &mut state);
        self.selected = Some(Selected {
            address: address.to_owned(),
            state,
        });
        Ok(vec![message])
    }

    /// Changes the ambient sound; the headphones confirm it, and the client
    /// that follows them is told then.
    pub(super) fn set_ambient(
        &self,
        address: &str,
        ambient: AmbientSound,
    ) -> Result<Vec<ServerMessage>, Failure> {
        self.hub()?.set_ambient(address, ambient)?;
        Ok(Vec::new())
    }

    /// Changes the equalizer, like [`HeadsetFeed::set_ambient`].
    pub(super) fn set_equalizer(
        &self,
        address: &str,
        change: EqualizerChange,
    ) -> Result<Vec<ServerMessage>, Failure> {
        self.hub()?.set_equalizer(address, change)?;
        Ok(Vec::new())
    }

    /// Closes the link on purpose.
    pub(super) fn release(&self, address: &str) -> Result<Vec<ServerMessage>, Failure> {
        self.hub()?.release(address)?;
        Ok(Vec::new())
    }

    /// Opens the link again.
    pub(super) fn resume(&self, address: &str) -> Result<Vec<ServerMessage>, Failure> {
        self.hub()?.resume(address)?;
        Ok(Vec::new())
    }
}

fn headsets_message(headsets: &mut watch::Receiver<Vec<HeadsetInfo>>) -> ServerMessage {
    ServerMessage::Headsets {
        headsets: headsets.borrow_and_update().clone(),
    }
}

fn state_message(address: &str, state: &mut watch::Receiver<HeadsetState>) -> ServerMessage {
    ServerMessage::HeadsetState {
        address: address.to_owned(),
        state: *state.borrow_and_update(),
    }
}

/// The next change of what is known of the selected headphones; the
/// selection ends with the headphones. Never ready without a selection.
async fn selected_change(selected: &mut Option<Selected>) -> Option<ServerMessage> {
    let Some(current) = selected else {
        return std::future::pending().await;
    };
    if current.state.changed().await.is_err() {
        *selected = None;
        return None;
    }
    Some(state_message(&current.address, &mut current.state))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hubergamof_protocol::{AmbientMode, HeadsetLink};

    use super::*;
    use crate::headset_hub::Action;

    const ADDRESS: &str = "AC:80:0A:0B:2E:61";

    fn info() -> HeadsetInfo {
        HeadsetInfo {
            address: ADDRESS.into(),
            name: "WH-1000XM4".into(),
        }
    }

    async fn heard(feed: &mut HeadsetFeed) -> Option<ServerMessage> {
        tokio::time::timeout(Duration::from_secs(5), feed.next_change())
            .await
            .expect("a change")
    }

    #[tokio::test]
    async fn without_headphones_served_nothing_is_told_and_everything_is_refused() {
        let mut feed = HeadsetFeed::new(None);
        assert_eq!(feed.initial(), None);
        assert!(
            tokio::time::timeout(Duration::from_millis(30), feed.next_change())
                .await
                .is_err(),
            "never ready"
        );
        let ambient = AmbientSound {
            mode: AmbientMode::Off,
            level: 0,
            focus_on_voice: false,
        };
        for failure in [
            feed.select(ADDRESS).unwrap_err(),
            feed.set_ambient(ADDRESS, ambient).unwrap_err(),
            feed.release(ADDRESS).unwrap_err(),
            feed.resume(ADDRESS).unwrap_err(),
        ] {
            assert_eq!(failure.code, ErrorCode::Unavailable);
        }
    }

    #[tokio::test]
    async fn a_new_client_is_told_the_headphones_connected_then_every_change() {
        let (hub, _requests) = HeadsetHub::new();
        let mut feed = HeadsetFeed::new(Some(&hub));
        assert_eq!(
            feed.initial(),
            Some(ServerMessage::Headsets { headsets: vec![] })
        );
        hub.present(vec![info()]);
        assert_eq!(
            heard(&mut feed).await,
            Some(ServerMessage::Headsets {
                headsets: vec![info()]
            })
        );
    }

    #[tokio::test]
    async fn selecting_follows_the_headphones_and_asks_for_the_link() {
        let (hub, mut requests) = HeadsetHub::new();
        hub.present(vec![info()]);
        let mut feed = HeadsetFeed::new(Some(&hub));
        let answer = feed.select(ADDRESS).unwrap();
        assert_eq!(
            answer,
            vec![ServerMessage::HeadsetState {
                address: ADDRESS.into(),
                state: HeadsetState::unknown(HeadsetLink::Idle),
            }]
        );
        assert_eq!(requests.try_recv().unwrap().action, Action::Open);

        hub.update_state(ADDRESS, |state| state.link = HeadsetLink::Connecting);
        let Some(ServerMessage::HeadsetState { address, state }) = heard(&mut feed).await else {
            panic!("a state");
        };
        assert_eq!(
            (address.as_str(), state.link),
            (ADDRESS, HeadsetLink::Connecting)
        );
    }

    #[tokio::test]
    async fn headphones_that_are_not_connected_cannot_be_selected() {
        let (hub, mut requests) = HeadsetHub::new();
        let mut feed = HeadsetFeed::new(Some(&hub));
        let failure = feed.select(ADDRESS).unwrap_err();
        assert_eq!(failure.code, ErrorCode::UnknownDevice);
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn the_selection_ends_with_the_headphones() {
        let (hub, _requests) = HeadsetHub::new();
        hub.present(vec![info()]);
        let mut feed = HeadsetFeed::new(Some(&hub));
        feed.select(ADDRESS).unwrap();
        hub.present(vec![]);
        // The list changed too: both are heard, the selection's end as `None`.
        let mut ended = false;
        for _ in 0..2 {
            if heard(&mut feed).await.is_none() {
                ended = true;
            }
        }
        assert!(ended, "the end of the selection is reported");
        assert!(feed.selected.is_none());
    }

    #[tokio::test]
    async fn requests_are_refused_with_the_code_of_the_hub() {
        let (hub, _requests) = HeadsetHub::new();
        hub.present(vec![info()]);
        let feed = HeadsetFeed::new(Some(&hub));
        let ambient = AmbientSound {
            mode: AmbientMode::Ambient,
            level: 3,
            focus_on_voice: false,
        };
        // The link is not open yet.
        assert_eq!(
            feed.set_ambient(ADDRESS, ambient).unwrap_err().code,
            ErrorCode::Unavailable
        );
        assert_eq!(
            feed.set_ambient("00:00:00:00:00:00", ambient)
                .unwrap_err()
                .code,
            ErrorCode::UnknownDevice
        );
        hub.update_state(ADDRESS, |state| state.link = HeadsetLink::Connected);
        let too_loud = AmbientSound {
            level: 21,
            ..ambient
        };
        assert_eq!(
            feed.set_ambient(ADDRESS, too_loud).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        assert_eq!(feed.set_ambient(ADDRESS, ambient).unwrap(), Vec::new());
    }
}
