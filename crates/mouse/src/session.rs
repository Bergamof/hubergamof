//! A mouse in use: its configuration applied, its diverted buttons followed,
//! everything given back at the end.
//!
//! The session owns the [`MouseLink`] and the state that survives between
//! notifications. The caller (a thread per mouse in the manager) decides when
//! to apply a configuration, to wait for buttons and to release them.

use std::time::Duration;

use hubergamof_protocol::{MouseConfig, MouseControlId};

use crate::channel::{Channel, HidppError};
use crate::controls::cid;
use crate::frame::Notification;
use crate::link::{ButtonEvent, ButtonTracker, MouseLink, haptic_strength};

/// A setting that could not be applied, the others having been.
#[derive(Debug)]
pub struct ApplyProblem {
    /// Which setting.
    pub setting: &'static str,
    /// Why.
    pub error: HidppError,
}

/// A mouse and what is known about its buttons.
pub struct MouseSession<C> {
    link: MouseLink<C>,
    tracker: ButtonTracker,
    dropped_frames: u64,
}

impl<C: Channel> MouseSession<C> {
    /// A session on a connected mouse.
    #[must_use]
    pub fn new(mut link: MouseLink<C>) -> Self {
        let dropped_frames = link.hidpp_mut().dropped_frames();
        Self {
            link,
            tracker: ButtonTracker::default(),
            dropped_frames,
        }
    }

    /// The link, for what a session does not cover.
    pub fn link_mut(&mut self) -> &mut MouseLink<C> {
        &mut self.link
    }

    /// Puts the mouse in the state `config` describes: sensitivity, wheel,
    /// thumb wheel, haptic strength, and the buttons with a binding diverted
    /// (the others given back). A setting the mouse lacks is skipped; one that
    /// fails does not stop the others and is reported.
    ///
    /// # Errors
    /// [`HidppError::InvalidArgument`] when the configuration is for another
    /// model, nothing being written then.
    pub fn apply(&mut self, config: &MouseConfig) -> Result<Vec<ApplyProblem>, HidppError> {
        if config.model != self.link.model() {
            return Err(HidppError::InvalidArgument(format!(
                "configuration of a {}, mouse is a {}",
                config.model,
                self.link.model()
            )));
        }
        let mut problems = Vec::new();
        let mut attempt = |setting: &'static str, result: Result<(), HidppError>| match result {
            Ok(()) | Err(HidppError::Unsupported(_)) => {}
            Err(error) => problems.push(ApplyProblem { setting, error }),
        };
        attempt("sensitivity", self.link.set_dpi(config.pointer.dpi));
        attempt(
            "wheel",
            self.link
                .set_wheel(config.wheel.mode, config.wheel.auto_threshold),
        );
        attempt(
            "high resolution wheel",
            self.link
                .set_hires_wheel(config.wheel.hi_res, config.wheel.natural),
        );
        if self.link.has_thumb_wheel() {
            attempt(
                "thumb wheel",
                self.link.set_thumb_invert(config.thumb_wheel.invert),
            );
        }
        if self.link.has_haptics() {
            attempt(
                "haptic strength",
                self.link.set_haptic(haptic_strength(config.haptics.level)),
            );
        }
        for control in self.link.model().capabilities().controls {
            if cid(self.link.model(), control.id).is_none() || control.locked {
                continue;
            }
            let wanted = config.bindings.contains_key(&control.id);
            attempt("button", self.set_button(control.id, wanted));
        }
        Ok(problems)
    }

    /// Diverts or gives back one button, writing only when it differs.
    fn set_button(&mut self, control: MouseControlId, wanted: bool) -> Result<(), HidppError> {
        if self.link.is_diverted(control)? != wanted {
            self.link.set_diverted(control, wanted)?;
        }
        Ok(())
    }

    /// Waits up to `timeout` for what the buttons do: the presses and releases
    /// since the last call. Notifications lost to a full queue count as
    /// releases of what was held, so that nothing stays pressed.
    ///
    /// # Errors
    /// [`HidppError`] when the channel fails.
    pub fn poll(&mut self, timeout: Duration) -> Result<Vec<ButtonEvent>, HidppError> {
        let frame = self.link.hidpp_mut().next_frame(timeout)?;
        let mut events = Vec::new();
        let dropped = self.link.hidpp_mut().dropped_frames();
        if dropped != self.dropped_frames {
            self.dropped_frames = dropped;
            events.extend(self.tracker.release_all());
        }
        let held = frame
            .and_then(|bytes| Notification::parse(&bytes))
            .and_then(|notification| self.link.held_buttons(&notification));
        if let Some(held) = held {
            events.extend(self.tracker.update(&held));
        }
        Ok(events)
    }

    /// Gives every diverted button back. Returns the releases of the buttons
    /// that were believed held, for the caller to undo what they started.
    ///
    /// # Errors
    /// The first [`HidppError`] met, every button having been tried.
    pub fn release(&mut self) -> Result<Vec<ButtonEvent>, HidppError> {
        let events = self.tracker.release_all();
        self.link.release_buttons()?;
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use hubergamof_protocol::{
        ButtonAction, HapticLevel, MouseConfig, MouseControlId, MouseModel, WheelMode,
    };

    use super::*;
    use crate::channel::Hidpp;
    use crate::features::REPROG_CONTROLS_V4;
    use crate::testing::{FakeDevice, FakeLink, FakeState, frame};

    const DEVICE: u8 = 0xFF;

    fn session(device: FakeDevice) -> MouseSession<FakeLink> {
        session_with(FakeLink::direct(device))
    }

    fn session_with(fake: FakeLink) -> MouseSession<FakeLink> {
        MouseSession::new(MouseLink::connect(Hidpp::new(fake), DEVICE).unwrap())
    }

    fn state(session: &MouseSession<FakeLink>) -> &FakeState {
        &session
            .link
            .hidpp_channel()
            .device_at(DEVICE)
            .unwrap()
            .state
    }

    fn config() -> MouseConfig {
        MouseConfig::default_for("serial", MouseModel::MxMaster4)
    }

    #[test]
    fn applying_the_default_configuration_sets_every_setting() {
        let mut session = session(FakeDevice::mx_master_4());
        let problems = session.apply(&config()).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        let state = state(&session);
        assert_eq!(state.dpi, 1600);
        assert_eq!(state.haptic, [1, 75]);
        // Only the control that opens the radial menu is diverted.
        assert_eq!(state.diverted.len(), 1);
    }

    #[test]
    fn the_values_of_the_configuration_reach_the_mouse() {
        let mut session = session(FakeDevice::mx_master_4());
        let mut config = config();
        config.pointer.dpi = 2400;
        config.wheel.mode = WheelMode::Ratchet;
        config.haptics.level = HapticLevel::Low;
        config.thumb_wheel.invert = true;
        config.wheel.natural = true;
        assert!(session.apply(&config).unwrap().is_empty());
        let state = state(&session);
        assert_eq!(state.dpi, 2400);
        assert_eq!(state.wheel[0], 2, "ratchet");
        assert_eq!(state.haptic, [1, 50]);
        assert_ne!(state.thumb[1] & 0x01, 0, "thumb wheel reversed");
        assert_ne!(state.hires & 0x04, 0, "natural scrolling");
    }

    #[test]
    fn haptics_off_switches_the_feedback_off() {
        let mut session = session(FakeDevice::mx_master_4());
        let mut config = config();
        config.haptics.level = HapticLevel::Off;
        session.apply(&config).unwrap();
        assert_eq!(state(&session).haptic[0], 0);
    }

    #[test]
    fn buttons_follow_the_bindings_and_the_others_are_given_back() {
        let mut session = session(FakeDevice::mx_master_4());
        let mut config = config();
        config
            .bindings
            .insert(MouseControlId::Back, ButtonAction::None);
        session.apply(&config).unwrap();
        let first = state(&session).diverted.clone();
        assert_eq!(first.len(), 2);
        config.bindings.remove(&MouseControlId::Back);
        session.apply(&config).unwrap();
        assert_eq!(state(&session).diverted.len(), 1);
    }

    #[test]
    fn applying_twice_writes_the_buttons_only_once() {
        let mut session = session(FakeDevice::mx_master_4());
        session.apply(&config()).unwrap();
        let written = session.link.hidpp_channel().written().len();
        session.apply(&config()).unwrap();
        let again = session.link.hidpp_channel().written().len() - written;
        let set_diverted = session
            .link
            .hidpp_channel()
            .written()
            .iter()
            .skip(written)
            .filter(|report| report.get(2) == Some(&reprog_index()) && report[3] >> 4 == 3)
            .count();
        assert!(again > 0);
        assert_eq!(set_diverted, 0, "no button was written again");
    }

    fn reprog_index() -> u8 {
        FakeDevice::mx_master_4()
            .index_of(REPROG_CONTROLS_V4)
            .unwrap()
    }

    #[test]
    fn a_configuration_of_another_model_writes_nothing() {
        let mut session = session(FakeDevice::mx_master_4());
        let other = MouseConfig::default_for("serial", MouseModel::MxAnywhere3S);
        let before = session.link.hidpp_channel().written().len();
        assert!(matches!(
            session.apply(&other),
            Err(HidppError::InvalidArgument(_))
        ));
        assert_eq!(session.link.hidpp_channel().written().len(), before);
    }

    #[test]
    fn a_mouse_without_haptics_or_thumb_wheel_skips_them() {
        let mut session = session(FakeDevice::mx_anywhere_3s());
        let config = MouseConfig::default_for("serial", MouseModel::MxAnywhere3S);
        let problems = session.apply(&config).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(state(&session).dpi, 1600);
    }

    #[test]
    fn a_failing_setting_is_reported_and_the_others_still_apply() {
        let device = FakeDevice::mx_master_4().with_broken_feature(crate::features::ADJUSTABLE_DPI);
        let mut session = session(device);
        let mut config = config();
        config.haptics.level = HapticLevel::High;
        let problems = session.apply(&config).unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].setting, "sensitivity");
        assert_eq!(state(&session).haptic, [1, 100]);
    }

    #[test]
    fn presses_and_releases_come_out_of_the_notifications() {
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        let fake = FakeLink::direct(device).with_incoming(vec![
            frame(DEVICE, reprog, 0x00, &[0, 0x53]),
            frame(DEVICE, reprog, 0x00, &[]),
        ]);
        let mut session = session_with(fake);
        let pressed = session.poll(Duration::from_millis(1)).unwrap();
        assert_eq!(
            pressed,
            [ButtonEvent {
                control: MouseControlId::Back,
                pressed: true
            }]
        );
        let released = session.poll(Duration::from_millis(1)).unwrap();
        assert_eq!(
            released,
            [ButtonEvent {
                control: MouseControlId::Back,
                pressed: false
            }]
        );
        assert_eq!(session.poll(Duration::from_millis(1)).unwrap(), []);
    }

    #[test]
    fn releasing_reports_what_was_held_and_gives_the_buttons_back() {
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        let fake =
            FakeLink::direct(device).with_incoming(vec![frame(DEVICE, reprog, 0x00, &[0, 0x53])]);
        let mut session = session_with(fake);
        session.apply(&config()).unwrap();
        session.poll(Duration::from_millis(1)).unwrap();
        let events = session.release().unwrap();
        assert_eq!(
            events,
            [ButtonEvent {
                control: MouseControlId::Back,
                pressed: false
            }]
        );
        assert!(state(&session).diverted.is_empty());
    }

    #[test]
    fn a_notification_of_another_device_is_ignored() {
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        let fake =
            FakeLink::direct(device).with_incoming(vec![frame(0x02, reprog, 0x00, &[0, 0x53])]);
        let mut session = session_with(fake);
        assert_eq!(session.poll(Duration::from_millis(1)).unwrap(), []);
    }

    #[test]
    fn notifications_lost_to_a_full_queue_release_what_was_held() {
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        let fake =
            FakeLink::direct(device).with_incoming(vec![frame(DEVICE, reprog, 0x00, &[0, 0x53])]);
        let mut session = session_with(fake);
        session.poll(Duration::from_millis(1)).unwrap();
        // The queue overflows while a request waits for its answer.
        let other = frame(0x02, reprog, 0x00, &[0, 0x56]);
        let flood = vec![other; crate::channel::MAX_PENDING_FRAMES + 10];
        session.link.hidpp_channel_mut().push_incoming(flood);
        session.link.dpi().unwrap();
        let events = session.poll(Duration::from_millis(1)).unwrap();
        assert_eq!(
            events,
            [ButtonEvent {
                control: MouseControlId::Back,
                pressed: false
            }]
        );
    }
}
