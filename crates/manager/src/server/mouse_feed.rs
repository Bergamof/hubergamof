//! What a WebSocket client follows about the mice: the list of those
//! connected, the configuration of the radial menu, and the configuration of
//! the one mouse it selected.

use hubergamof_protocol::{
    ErrorCode, HapticWaveform, MouseConfig, MouseInfo, MouseStatus, RadialMenuConfig, ServerMessage,
};
use tokio::sync::watch;

use super::errors::Failure;
use crate::mouse_hub::MouseHub;

/// What a failure of a request about the mice is logged as.
const CONTEXT: &str = "mouse request";

/// The mouse a client follows.
struct Selected {
    serial: String,
    config: watch::Receiver<MouseConfig>,
    status: watch::Receiver<MouseStatus>,
}

/// One client's view of the mice.
pub(super) struct MouseFeed {
    hub: MouseHub,
    mice: watch::Receiver<Vec<MouseInfo>>,
    radial: watch::Receiver<RadialMenuConfig>,
    selected: Option<Selected>,
}

impl MouseFeed {
    pub(super) fn new(hub: &MouseHub) -> Self {
        Self {
            hub: hub.clone(),
            mice: hub.subscribe_mice(),
            radial: hub.subscribe_radial_menu(),
            selected: None,
        }
    }

    /// What a new client is told: the mice connected and the radial menu.
    pub(super) fn initial(&mut self) -> [ServerMessage; 2] {
        [self.mice_message(), self.radial_message()]
    }

    fn mice_message(&mut self) -> ServerMessage {
        ServerMessage::Mice {
            mice: self.mice.borrow_and_update().clone(),
        }
    }

    fn radial_message(&mut self) -> ServerMessage {
        ServerMessage::RadialMenu {
            config: self.radial.borrow_and_update().clone(),
        }
    }

    /// Waits for the next change of what the client follows. `None` when the
    /// mouse it selected went away (the new list of mice tells it).
    pub(super) async fn next_change(&mut self) -> Option<ServerMessage> {
        tokio::select! {
            Ok(()) = self.mice.changed() => Some(self.mice_message()),
            Ok(()) = self.radial.changed() => Some(self.radial_message()),
            change = selected_change(&self.hub, &mut self.selected) => change,
        }
    }

    /// Follows a connected mouse instead of the previous one; answers with its
    /// configuration.
    pub(super) fn select(&mut self, serial: &str) -> Result<ServerMessage, Failure> {
        self.selected = None;
        let (Some(mut config), Some(status)) = (
            self.hub.subscribe_config(serial),
            self.hub.subscribe_status(serial),
        ) else {
            return Err(Failure::of(
                ErrorCode::UnknownDevice,
                "no connected mouse with this serial",
                CONTEXT,
            ));
        };
        let message = ServerMessage::MouseConfig {
            serial: serial.to_owned(),
            config: config.borrow_and_update().clone(),
        };
        self.selected = Some(Selected {
            serial: serial.to_owned(),
            config,
            status,
        });
        Ok(message)
    }

    /// What the selected mouse refused of its configuration, to send after
    /// the configuration itself; `None` without a selection.
    pub(super) fn status(&mut self) -> Option<ServerMessage> {
        let selected = self.selected.as_mut()?;
        Some(status_message(selected))
    }

    /// Saves a configuration of a mouse and answers the author with the saved
    /// one (plus a newer one, if another update overtook it before it was
    /// sent). The other clients get it through their own feed.
    pub(super) async fn update_mouse(
        &mut self,
        serial: String,
        config: MouseConfig,
    ) -> Result<Vec<ServerMessage>, Failure> {
        let hub = self.hub.clone();
        let saved = blocking(move || {
            hub.update_mouse(&serial, config)
                .map_err(|error| (error.error_code(), error.to_string()))
        })
        .await?;
        let serial = saved.serial.clone();
        // Without a selection of this mouse, nothing overtook the update for
        // this client.
        let latest = self
            .selected
            .as_mut()
            .filter(|selected| selected.serial == serial)
            .map_or_else(
                || saved.clone(),
                |selected| selected.config.borrow_and_update().clone(),
            );
        Ok(answers(saved, latest, |config| {
            ServerMessage::MouseConfig {
                serial: serial.clone(),
                config,
            }
        }))
    }

    /// Has a waveform played on a mouse, for the person to try it: nothing to
    /// answer unless it cannot be.
    pub(super) fn play_haptic(
        &self,
        serial: &str,
        waveform: HapticWaveform,
    ) -> Result<Vec<ServerMessage>, Failure> {
        self.hub
            .try_waveform(serial, waveform)
            .map(|()| Vec::new())
            .map_err(|error| Failure::of(error.error_code(), error.to_string(), CONTEXT))
    }

    /// Saves the radial menu, like [`MouseFeed::update_mouse`].
    pub(super) async fn update_radial_menu(
        &mut self,
        config: RadialMenuConfig,
    ) -> Result<Vec<ServerMessage>, Failure> {
        let hub = self.hub.clone();
        let saved = blocking(move || {
            hub.update_radial_menu(config)
                .map_err(|error| (error.error_code(), error.to_string()))
        })
        .await?;
        let latest = self.radial.borrow_and_update().clone();
        Ok(answers(saved, latest, |config| ServerMessage::RadialMenu {
            config,
        }))
    }
}

fn status_message(selected: &mut Selected) -> ServerMessage {
    let MouseStatus { problems, battery } = selected.status.borrow_and_update().clone();
    ServerMessage::MouseStatus {
        serial: selected.serial.clone(),
        problems,
        battery,
    }
}

/// The next change of the selected mouse (its configuration, or what it
/// refused of it); the selection ends with the mouse. Never ready without a
/// selection.
///
/// A mouse registered again under its serial (it came back before the old
/// entry was released) is followed again, and its configuration sent, its
/// status following.
async fn selected_change(hub: &MouseHub, selected: &mut Option<Selected>) -> Option<ServerMessage> {
    let Some(current) = selected else {
        return std::future::pending().await;
    };
    tokio::select! {
        config = current.config.changed() => {
            if config.is_err() {
                let (Some(config), Some(status)) = (
                    hub.subscribe_config(&current.serial),
                    hub.subscribe_status(&current.serial),
                ) else {
                    *selected = None;
                    return None;
                };
                current.config = config;
                current.status = status;
                // What the new registration refused is told next.
                current.status.mark_changed();
            }
            Some(ServerMessage::MouseConfig {
                serial: current.serial.clone(),
                config: current.config.borrow_and_update().clone(),
            })
        }
        Ok(()) = current.status.changed() => Some(status_message(current)),
    }
}

/// What the author of an update is told: the configuration saved, and the
/// latest one when another update overtook it before the answer was sent.
fn answers<T: PartialEq>(
    saved: T,
    latest: T,
    message: impl Fn(T) -> ServerMessage,
) -> Vec<ServerMessage> {
    if latest == saved {
        vec![message(saved)]
    } else {
        vec![message(saved), message(latest)]
    }
}

/// Runs a store operation off the async threads.
async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, (ErrorCode, String)> + Send + 'static,
) -> Result<T, Failure> {
    match tokio::task::spawn_blocking(operation).await {
        Ok(result) => result.map_err(|(code, detail)| Failure::of(code, detail, CONTEXT)),
        Err(error) => Err(Failure::of(ErrorCode::Internal, error.to_string(), CONTEXT)),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use hubergamof_protocol::{MouseBattery, MouseModel};

    use super::*;
    use crate::config_store::ConfigStore;
    use crate::mouse_hub::MouseRegistration;
    use crate::testing::builtin_registry;

    const SERIAL: &str = "aaaa0001";

    fn hub() -> (tempfile::TempDir, MouseHub) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path(), Arc::new(builtin_registry()));
        (dir, MouseHub::new(store).unwrap())
    }

    fn connect(hub: &MouseHub, serial: &str) -> MouseRegistration {
        let model = MouseModel::MxMaster4;
        hub.register(
            MouseInfo {
                serial: serial.to_owned(),
                model,
                display_name: model.display_name().to_owned(),
                capabilities: model.capabilities(),
            },
            MouseConfig::default_for(serial, model),
        )
    }

    async fn quiet(feed: &mut MouseFeed) -> bool {
        tokio::time::timeout(Duration::from_millis(50), feed.next_change())
            .await
            .is_err()
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_client_gets_the_mice_and_the_menu_then_every_change() {
        let (_dir, hub) = hub();
        let mut feed = MouseFeed::new(&hub);
        let [mice, menu] = feed.initial();
        assert_eq!(mice, ServerMessage::Mice { mice: vec![] });
        assert!(matches!(menu, ServerMessage::RadialMenu { .. }));
        assert!(quiet(&mut feed).await, "nothing changed since");
        let registration = connect(&hub, SERIAL);
        let Some(ServerMessage::Mice { mice }) = feed.next_change().await else {
            panic!("the list of mice was not sent");
        };
        assert_eq!(mice.len(), 1);
        drop(registration);
        let Some(ServerMessage::Mice { mice }) = feed.next_change().await else {
            panic!("the list of mice was not sent");
        };
        assert_eq!(mice, []);
    }

    #[tokio::test(start_paused = true)]
    async fn selecting_a_mouse_sends_its_configuration_and_then_every_change() {
        let (_dir, hub) = hub();
        let _registration = connect(&hub, SERIAL);
        let mut feed = MouseFeed::new(&hub);
        feed.initial();
        let Ok(ServerMessage::MouseConfig { serial, config }) = feed.select(SERIAL) else {
            panic!("the configuration was not sent");
        };
        assert_eq!((serial.as_str(), config.revision), (SERIAL, 0));
        // Another client changes it.
        let mut other = config.clone();
        other.pointer.dpi = 2400;
        hub.update_mouse(SERIAL, other).unwrap();
        let Some(ServerMessage::MouseConfig { config, .. }) = feed.next_change().await else {
            panic!("the change was not sent");
        };
        assert_eq!((config.revision, config.pointer.dpi), (1, 2400));
    }

    #[tokio::test(start_paused = true)]
    async fn trying_a_waveform_answers_nothing_unless_the_mouse_is_gone() {
        let (_dir, hub) = hub();
        let _registration = connect(&hub, SERIAL);
        let feed = MouseFeed::new(&hub);
        assert_eq!(feed.play_haptic(SERIAL, HapticWaveform::Knock).unwrap(), []);
        let failure = feed
            .play_haptic("bbbb0001", HapticWaveform::Knock)
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::UnknownDevice);
    }

    #[tokio::test(start_paused = true)]
    async fn trying_a_waveform_on_a_mouse_without_haptics_is_an_invalid_input() {
        let (_dir, hub) = hub();
        let model = MouseModel::MxAnywhere3S;
        let _registration = hub.register(
            MouseInfo {
                serial: SERIAL.to_owned(),
                model,
                display_name: model.display_name().to_owned(),
                capabilities: model.capabilities(),
            },
            MouseConfig::default_for(SERIAL, model),
        );
        let feed = MouseFeed::new(&hub);
        let failure = feed.play_haptic(SERIAL, HapticWaveform::Knock).unwrap_err();
        assert_eq!(failure.code, ErrorCode::InvalidInput);
    }

    #[tokio::test(start_paused = true)]
    async fn the_battery_of_the_selected_mouse_is_sent_when_it_is_read_and_when_it_is_lost() {
        let (_dir, hub) = hub();
        let registration = connect(&hub, SERIAL);
        let mut feed = MouseFeed::new(&hub);
        feed.select(SERIAL).unwrap();
        let status = feed.status().expect("the status of the selected mouse");
        assert!(matches!(
            status,
            ServerMessage::MouseStatus { battery: None, .. }
        ));
        let battery = MouseBattery {
            percent: 64,
            charging: true,
        };
        registration.status_reporter().report_battery(Some(battery));
        let Some(ServerMessage::MouseStatus { battery: sent, .. }) = feed.next_change().await
        else {
            panic!("the battery was not sent");
        };
        assert_eq!(sent, Some(battery));
        registration.status_reporter().report_battery(None);
        let Some(ServerMessage::MouseStatus { battery: sent, .. }) = feed.next_change().await
        else {
            panic!("the lost battery was not sent");
        };
        assert_eq!(sent, None);
    }

    #[tokio::test(start_paused = true)]
    async fn selecting_a_mouse_that_is_not_connected_fails_and_drops_the_previous_one() {
        let (_dir, hub) = hub();
        let _registration = connect(&hub, SERIAL);
        let mut feed = MouseFeed::new(&hub);
        feed.select(SERIAL).unwrap();
        let Err(failure) = feed.select("bbbb0001") else {
            panic!("an unknown mouse was selected");
        };
        assert_eq!(failure.code, ErrorCode::UnknownDevice);
        let mut config = hub.subscribe_config(SERIAL).unwrap().borrow().clone();
        config.pointer.dpi = 2400;
        hub.update_mouse(SERIAL, config).unwrap();
        assert!(quiet(&mut feed).await, "the previous mouse is not followed");
    }

    #[tokio::test(start_paused = true)]
    async fn the_selection_ends_with_the_mouse() {
        let (_dir, hub) = hub();
        let registration = connect(&hub, SERIAL);
        let mut feed = MouseFeed::new(&hub);
        feed.initial();
        feed.select(SERIAL).unwrap();
        drop(registration);
        // The list changes first, then the selection reports its end.
        let mut messages = Vec::new();
        for _ in 0..2 {
            messages.push(feed.next_change().await);
        }
        assert!(messages.contains(&None));
        assert!(messages.iter().any(
            |message| matches!(message, Some(ServerMessage::Mice { mice }) if mice.is_empty())
        ));
        assert!(feed.selected.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn the_author_of_an_update_gets_it_once_and_the_others_through_their_feed() {
        let (_dir, hub) = hub();
        let _registration = connect(&hub, SERIAL);
        let mut author = MouseFeed::new(&hub);
        let mut other = MouseFeed::new(&hub);
        let ServerMessage::MouseConfig { config, .. } = author.select(SERIAL).unwrap() else {
            panic!("no configuration");
        };
        other.select(SERIAL).unwrap();
        let mut update = config;
        update.pointer.dpi = 3200;
        let answers = author
            .update_mouse(SERIAL.to_owned(), update)
            .await
            .unwrap();
        assert_eq!(answers.len(), 1);
        assert!(
            matches!(&answers[0], ServerMessage::MouseConfig { config, .. } if config.revision == 1)
        );
        assert!(quiet(&mut author).await, "the author is not told twice");
        let Some(ServerMessage::MouseConfig { config, .. }) = other.next_change().await else {
            panic!("the other client was not told");
        };
        assert_eq!(config.pointer.dpi, 3200);
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_mouse_update_reports_why() {
        let (_dir, hub) = hub();
        let _registration = connect(&hub, SERIAL);
        let mut feed = MouseFeed::new(&hub);
        let mut config = MouseConfig::default_for(SERIAL, MouseModel::MxMaster4);
        let unknown = feed
            .update_mouse(
                "bbbb0001".to_owned(),
                MouseConfig::default_for("bbbb0001", MouseModel::MxMaster4),
            )
            .await;
        assert!(matches!(
            unknown,
            Err(Failure {
                code: ErrorCode::UnknownDevice,
                ..
            })
        ));
        config.pointer.dpi = 225;
        let invalid = feed.update_mouse(SERIAL.to_owned(), config.clone()).await;
        assert!(matches!(
            invalid,
            Err(Failure {
                code: ErrorCode::InvalidConfig,
                ..
            })
        ));
        config.pointer.dpi = 1600;
        feed.update_mouse(SERIAL.to_owned(), config.clone())
            .await
            .unwrap();
        let stale = feed.update_mouse(SERIAL.to_owned(), config).await;
        assert!(matches!(
            stale,
            Err(Failure {
                code: ErrorCode::StaleRevision,
                ..
            })
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn every_client_follows_the_radial_menu() {
        let (_dir, hub) = hub();
        let mut author = MouseFeed::new(&hub);
        let mut other = MouseFeed::new(&hub);
        let [_, ServerMessage::RadialMenu { config }] = author.initial() else {
            panic!("no menu");
        };
        other.initial();
        let mut menu = config;
        menu.animate = !menu.animate;
        let answers = author.update_radial_menu(menu).await.unwrap();
        assert_eq!(answers.len(), 1);
        assert!(quiet(&mut author).await, "the author is not told twice");
        assert!(matches!(
            other.next_change().await,
            Some(ServerMessage::RadialMenu { config }) if config.revision == 1
        ));
        let mut invalid = hub.subscribe_radial_menu().borrow().clone();
        invalid.hold_ms = 10;
        let refused = author.update_radial_menu(invalid).await;
        assert!(matches!(
            refused,
            Err(Failure {
                code: ErrorCode::InvalidConfig,
                ..
            })
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn a_mouse_registered_again_is_followed_again() {
        let (_dir, hub) = hub();
        let first = connect(&hub, SERIAL);
        let mut feed = MouseFeed::new(&hub);
        feed.initial();
        feed.select(SERIAL).unwrap();
        // The mouse comes back before the old entry is released.
        let _second = connect(&hub, SERIAL);
        drop(first);
        let mut sent = Vec::new();
        while !sent
            .iter()
            .any(|message| matches!(message, Some(ServerMessage::MouseConfig { .. })))
        {
            assert!(sent.len() < 3, "the configuration never came: {sent:?}");
            sent.push(feed.next_change().await);
        }
        let mut config = hub.subscribe_config(SERIAL).unwrap().borrow().clone();
        config.pointer.dpi = 2400;
        hub.update_mouse(SERIAL, config).unwrap();
        // The list of mice may still be waiting to be sent, before it.
        let followed = loop {
            match feed.next_change().await {
                Some(ServerMessage::MouseConfig { config, .. }) => break config,
                // The status of the new registration comes after its configuration.
                Some(ServerMessage::Mice { .. } | ServerMessage::MouseStatus { .. }) => {}
                other => panic!("the new mouse is not followed: {other:?}"),
            }
        };
        assert_eq!(followed.pointer.dpi, 2400);
    }

    #[test]
    fn an_update_overtaken_before_the_answer_is_followed_by_the_latest() {
        let message = |value: u32| ServerMessage::Mice {
            mice: vec![MouseInfo {
                serial: value.to_string(),
                model: MouseModel::MxMaster4,
                display_name: String::new(),
                capabilities: MouseModel::MxMaster4.capabilities(),
            }],
        };
        assert_eq!(answers(1, 1, message), [message(1)]);
        assert_eq!(answers(1, 2, message), [message(1), message(2)]);
    }

    #[tokio::test(start_paused = true)]
    async fn updating_a_mouse_that_just_left_is_refused() {
        let (_dir, hub) = hub();
        let registration = connect(&hub, SERIAL);
        let mut feed = MouseFeed::new(&hub);
        drop(registration);
        let refused = feed
            .update_mouse(
                SERIAL.to_owned(),
                MouseConfig::default_for(SERIAL, MouseModel::MxMaster4),
            )
            .await;
        assert!(matches!(
            refused,
            Err(Failure {
                code: ErrorCode::UnknownDevice,
                ..
            })
        ));
    }
}
