//! A mouse in use: its configuration applied, its diverted buttons followed,
//! everything given back at the end.
//!
//! The session owns the [`MouseLink`] and the state that survives between
//! notifications. The caller (a thread per mouse in the manager) decides when
//! to apply a configuration, to wait for buttons and to release them.

use std::sync::atomic::{AtomicBool, Ordering};
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

/// What [`MouseSession::serve`] has to report.
#[derive(Debug)]
pub enum Notice {
    /// A setting the mouse refused when the configuration was applied.
    Problem(ApplyProblem),
    /// A press or a release of a diverted button.
    Button(ButtonEvent),
}

/// How long [`MouseSession::serve`] waits for the buttons before looking at
/// the stop request again: the delay with which it reacts to it.
const SERVE_STEP: Duration = Duration::from_millis(200);

/// An error after which the mouse cannot be served: it stopped answering or
/// its channel failed. Any other is a refusal, and the rest can go on.
fn is_fatal(error: &HidppError) -> bool {
    matches!(error, HidppError::Timeout(_) | HidppError::Channel(_))
}

/// A mouse and what is known about its buttons.
///
/// Dropping the session gives the diverted buttons back, as well as a gone
/// mouse allows (a channel that fails ends it at once): a thread that panics
/// or returns early must not leave them dead. Call [`MouseSession::give_back`]
/// first to see the outcome.
pub struct MouseSession<C: Channel> {
    link: MouseLink<C>,
    tracker: ButtonTracker,
    dropped_frames: u64,
    /// Buttons may be diverted: set by [`MouseSession::apply`], cleared once
    /// they are given back.
    buttons_diverted: bool,
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
            buttons_diverted: false,
        }
    }

    /// The link, for what a session does not cover.
    pub fn link_mut(&mut self) -> &mut MouseLink<C> {
        &mut self.link
    }

    /// Puts the mouse in the state `config` describes: sensitivity, wheel,
    /// thumb wheel, haptic strength, and the buttons with a binding diverted
    /// (the others given back). A setting the mouse lacks is skipped; one the
    /// mouse refuses does not stop the others and is reported.
    ///
    /// The caller picks the configuration of *this* mouse (by its unit id): a
    /// session only checks the model. A binding of a rotation (the thumb wheel)
    /// has nothing to divert and is left alone.
    ///
    /// # Errors
    /// [`HidppError::InvalidArgument`] when the configuration is invalid or
    /// for another model, nothing being written then; [`HidppError::Timeout`]
    /// or [`HidppError::Channel`] as soon as the mouse stops answering (the
    /// caller then gives back and reconnects).
    pub fn apply(&mut self, config: &MouseConfig) -> Result<Vec<ApplyProblem>, HidppError> {
        config
            .validate()
            .map_err(|error| HidppError::InvalidArgument(error.to_string()))?;
        if config.model != self.link.model() {
            return Err(HidppError::InvalidArgument(format!(
                "configuration of a {}, mouse is a {}",
                config.model,
                self.link.model()
            )));
        }
        // A mouse idle for a while answers late: wake it, or the first
        // request would end the service.
        let _ = self.link.wake();
        let mut problems = Vec::new();
        let mut attempt = |setting: &'static str, result: Result<(), HidppError>| match result {
            Ok(()) | Err(HidppError::Unsupported(_)) => Ok(()),
            Err(error) if is_fatal(&error) => Err(error),
            Err(error) => {
                problems.push(ApplyProblem { setting, error });
                Ok(())
            }
        };
        attempt("sensitivity", self.link.set_dpi(config.pointer.dpi))?;
        attempt(
            "wheel",
            self.link
                .set_wheel(config.wheel.mode, config.wheel.auto_threshold),
        )?;
        attempt(
            "high resolution wheel",
            self.link
                .set_hires_wheel(config.wheel.hi_res, config.wheel.natural),
        )?;
        if self.link.has_thumb_wheel() {
            attempt(
                "thumb wheel",
                self.link.set_thumb_invert(config.thumb_wheel.invert),
            )?;
        }
        if self.link.has_haptics() {
            attempt(
                "haptic strength",
                self.link.set_haptic(haptic_strength(config.haptics.level)),
            )?;
        }
        for control in self.link.model().capabilities().controls {
            if cid(self.link.model(), control.id).is_none() || control.locked {
                continue;
            }
            let wanted = config.bindings.contains_key(&control.id);
            // From the first write on, buttons may be diverted, even when a
            // later step fails.
            self.buttons_diverted |= wanted;
            attempt("button", self.set_button(control.id, wanted))?;
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
    /// [`HidppError`] when the channel fails; the caller then undoes what the
    /// held buttons started with [`MouseSession::release_held`].
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

    /// The releases of the buttons believed held, for the caller to undo what
    /// they started (a pressed key, a held menu). No I/O: it works with the
    /// mouse gone, which is when it is needed.
    pub fn release_held(&mut self) -> Vec<ButtonEvent> {
        self.tracker.release_all()
    }

    /// Serves the mouse until `stop` is raised or the mouse is gone: gives
    /// back what a previous run left diverted, applies `config`, then reports
    /// the buttons. Everything is given back at the end, and the releases of
    /// the buttons still held are reported before.
    ///
    /// # Errors
    /// The [`HidppError`] that ended the service (the mouse stopped answering
    /// or left), `Ok` when `stop` did.
    pub fn serve(
        &mut self,
        config: &MouseConfig,
        stop: &AtomicBool,
        mut notify: impl FnMut(Notice),
    ) -> Result<(), HidppError> {
        let served = self.serve_buttons(config, stop, &mut notify);
        for event in self.release_held() {
            notify(Notice::Button(event));
        }
        let given_back = self.give_back();
        served.and(given_back)
    }

    fn serve_buttons(
        &mut self,
        config: &MouseConfig,
        stop: &AtomicBool,
        notify: &mut impl FnMut(Notice),
    ) -> Result<(), HidppError> {
        // What a previous run left diverted: a refusal is reported, a mouse that
        // does not answer ends the service.
        if let Err(error) = self.give_back() {
            if is_fatal(&error) {
                return Err(error);
            }
            notify(Notice::Problem(ApplyProblem {
                setting: "buttons left by a previous run",
                error,
            }));
        }
        for problem in self.apply(config)? {
            notify(Notice::Problem(problem));
        }
        while !stop.load(Ordering::Relaxed) {
            for event in self.poll(SERVE_STEP)? {
                notify(Notice::Button(event));
            }
        }
        Ok(())
    }

    /// Gives every diverted button back, every one being tried.
    ///
    /// # Errors
    /// The first [`HidppError`] met.
    pub fn give_back(&mut self) -> Result<(), HidppError> {
        // Asleep, a mouse does not answer the first requests and its buttons
        // would stay as they are: wake it first.
        let _ = self.link.wake();
        let result = self.link.release_buttons();
        // A mouse that does not answer will not at the drop either; one that
        // refused something might.
        if !matches!(&result, Err(error) if !is_fatal(error)) {
            self.buttons_diverted = false;
        }
        result
    }
}

impl<C: Channel> Drop for MouseSession<C> {
    fn drop(&mut self) {
        if self.buttons_diverted {
            // Nothing to do with a failure: a mouse that is gone cannot be asked,
            // and is expected to forget them when it reconnects (to be checked
            // on the hardware, docs/mouse-hidpp.md).
            let _ = self.give_back();
        }
    }
}

#[cfg(test)]
mod tests {
    use hubergamof_protocol::{
        ButtonAction, HapticLevel, MouseConfig, MouseControlId, MouseModel, WheelMode,
    };

    use super::*;
    use crate::channel::Hidpp;
    use crate::controls::cid;
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

    /// Reports written to the button reporting function (3) of the mouse.
    fn button_writes(session: &MouseSession<FakeLink>, from: usize) -> usize {
        session.link.hidpp_channel().written()[from..]
            .iter()
            .filter(|report| report.get(2) == Some(&reprog_index()) && report[3] >> 4 == 3)
            .count()
    }

    #[test]
    fn applying_twice_writes_the_buttons_only_once() {
        let mut session = session(FakeDevice::mx_master_4());
        session.apply(&config()).unwrap();
        assert_eq!(button_writes(&session, 0), 1, "the menu button is diverted");
        let written = session.link.hidpp_channel().written().len();
        session.apply(&config()).unwrap();
        assert!(session.link.hidpp_channel().written().len() > written);
        assert_eq!(button_writes(&session, written), 0, "nothing to change");
    }

    fn reprog_index() -> u8 {
        FakeDevice::mx_master_4()
            .index_of(REPROG_CONTROLS_V4)
            .unwrap()
    }

    #[test]
    fn every_wheel_mode_reaches_the_mouse_with_its_threshold() {
        for (mode, expected) in [
            (WheelMode::Ratchet, [2, 255]),
            // A zero means "leave it": the mouse keeps its own threshold.
            (WheelMode::Free, [1, 10]),
            (WheelMode::Auto, [2, 7]),
        ] {
            let mut session = session(FakeDevice::mx_master_4());
            let mut config = config();
            config.wheel.mode = mode;
            config.wheel.auto_threshold = 7;
            session.apply(&config).unwrap();
            assert_eq!(state(&session).wheel[..2], expected, "{mode:?}");
        }
    }

    #[test]
    fn the_wheel_flags_follow_the_configuration_and_keep_the_other_bits() {
        let mut device = FakeDevice::mx_master_4();
        device.state.hires = 0x09;
        let mut session = session(device);
        let mut config = config();
        config.wheel.hi_res = false;
        config.wheel.natural = false;
        session.apply(&config).unwrap();
        assert_eq!(state(&session).hires, 0x09);
        config.wheel.hi_res = true;
        session.apply(&config).unwrap();
        assert_eq!(state(&session).hires, 0x0B);
        config.wheel.natural = true;
        session.apply(&config).unwrap();
        assert_eq!(state(&session).hires, 0x0F);
    }

    #[test]
    fn the_thumb_wheel_is_reversed_and_set_straight_again() {
        let mut device = FakeDevice::mx_master_4();
        device.state.thumb = [0x40, 0x00];
        let mut session = session(device);
        let mut config = config();
        config.thumb_wheel.invert = true;
        session.apply(&config).unwrap();
        assert_eq!(state(&session).thumb, [0x40, 0x01]);
        config.thumb_wheel.invert = false;
        session.apply(&config).unwrap();
        assert_eq!(state(&session).thumb, [0x40, 0x00]);
    }

    #[test]
    fn the_menu_button_of_the_default_is_the_one_diverted() {
        let mut session = session(FakeDevice::mx_master_4());
        session.apply(&config()).unwrap();
        let menu = cid(
            MouseModel::MxMaster4,
            MouseModel::MxMaster4.default_menu_control(),
        );
        assert_eq!(
            state(&session)
                .diverted
                .iter()
                .copied()
                .map(Some)
                .collect::<Vec<_>>(),
            [menu]
        );
    }

    #[test]
    fn two_failing_settings_are_both_reported_in_order() {
        let device = FakeDevice::mx_master_4()
            .with_broken_feature(crate::features::ADJUSTABLE_DPI)
            .with_broken_feature(crate::features::SMART_SHIFT_ENHANCED);
        let mut session = session(device);
        let problems = session.apply(&config()).unwrap();
        let named: Vec<_> = problems.iter().map(|problem| problem.setting).collect();
        assert_eq!(named, ["sensitivity", "wheel"]);
        assert_eq!(
            state(&session).diverted.len(),
            1,
            "the buttons were still set"
        );
    }

    #[test]
    fn a_mouse_without_haptics_or_thumb_wheel_is_not_asked_for_them() {
        let session = session(FakeDevice::mx_anywhere_3s());
        assert!(!session.link.has_haptics());
        assert!(!session.link.has_thumb_wheel());
    }

    #[test]
    fn several_buttons_held_at_once_are_reported_together() {
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        let fake = FakeLink::direct(device).with_incoming(vec![
            frame(DEVICE, reprog, 0x00, &[0, 0x53, 0, 0x56, 0xFF, 0xFF]),
            frame(DEVICE, reprog, 0x00, &[0, 0x56]),
        ]);
        let mut session = session_with(fake);
        let controls = |events: Vec<ButtonEvent>| -> Vec<(MouseControlId, bool)> {
            events
                .into_iter()
                .map(|event| (event.control, event.pressed))
                .collect()
        };
        let first = controls(session.poll(Duration::from_millis(1)).unwrap());
        assert_eq!(
            first,
            [
                (MouseControlId::Back, true),
                (MouseControlId::Forward, true)
            ],
            "an id the model does not know is left out"
        );
        let second = controls(session.poll(Duration::from_millis(1)).unwrap());
        assert_eq!(second, [(MouseControlId::Back, false)]);
    }

    #[test]
    fn a_failing_channel_ends_the_poll_with_an_error() {
        let fake = FakeLink::direct(FakeDevice::mx_master_4()).with_broken_reads();
        let mut session = session_with(fake);
        assert!(session.poll(Duration::from_millis(1)).is_err());
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
        let events = session.release_held();
        assert_eq!(
            events,
            [ButtonEvent {
                control: MouseControlId::Back,
                pressed: false
            }]
        );
        session.give_back().unwrap();
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

    /// A link tests keep a hand on after the session owns it.
    #[derive(Clone)]
    struct Shared(std::rc::Rc<std::cell::RefCell<FakeLink>>);

    impl Channel for Shared {
        fn write(&mut self, report: &[u8]) -> Result<(), crate::channel::ChannelError> {
            self.0.borrow_mut().write(report)
        }

        fn read(
            &mut self,
            timeout: Duration,
        ) -> Result<Option<Vec<u8>>, crate::channel::ChannelError> {
            self.0.borrow_mut().read(timeout)
        }
    }

    fn shared_session(device: FakeDevice) -> (MouseSession<Shared>, Shared) {
        let shared = Shared(std::rc::Rc::new(std::cell::RefCell::new(FakeLink::direct(
            device,
        ))));
        let hidpp = Hidpp::new(shared.clone()).with_timeout(Duration::from_millis(5));
        let session = MouseSession::new(MouseLink::connect(hidpp, DEVICE).unwrap());
        (session, shared)
    }

    fn diverted(shared: &Shared) -> usize {
        shared
            .0
            .borrow()
            .device_at(DEVICE)
            .unwrap()
            .state
            .diverted
            .len()
    }

    #[test]
    fn dropping_the_session_gives_the_buttons_back() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        session.apply(&config()).unwrap();
        assert_eq!(diverted(&shared), 1);
        drop(session);
        assert_eq!(diverted(&shared), 0);
    }

    #[test]
    fn a_session_that_diverted_nothing_writes_nothing_when_dropped() {
        let (session, shared) = shared_session(FakeDevice::mx_master_4());
        let written = shared.0.borrow().written().len();
        drop(session);
        assert_eq!(shared.0.borrow().written().len(), written);
    }

    #[test]
    fn an_invalid_configuration_writes_nothing() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        let written = shared.0.borrow().written().len();
        let mut invalid = config();
        invalid.pointer.dpi = 225;
        assert!(matches!(
            session.apply(&invalid),
            Err(HidppError::InvalidArgument(_))
        ));
        assert_eq!(shared.0.borrow().written().len(), written);
    }

    #[test]
    fn a_mouse_that_stops_answering_ends_the_application_at_once() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        shared.0.borrow_mut().fall_silent();
        let written = shared.0.borrow().written().len();
        assert!(matches!(
            session.apply(&config()),
            Err(HidppError::Timeout(_))
        ));
        // The attempts to wake it, then the first request, which ends it.
        assert_eq!(shared.0.borrow().written().len(), written + 3 + 1);
    }

    #[test]
    fn a_vanished_mouse_still_gives_back_what_its_buttons_started() {
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        let (mut session, shared) = shared_session(device);
        session.apply(&config()).unwrap();
        shared
            .0
            .borrow_mut()
            .push_incoming([frame(DEVICE, reprog, 0x00, &[0, 0x53])]);
        session.poll(Duration::from_millis(1)).unwrap();
        shared.0.borrow_mut().refuse_writes();
        assert!(session.give_back().is_err());
        assert_eq!(
            session.release_held(),
            [ButtonEvent {
                control: MouseControlId::Back,
                pressed: false
            }]
        );
    }

    #[test]
    fn serving_applies_reports_and_gives_everything_back_when_stopped() {
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        let fake =
            FakeLink::direct(device).with_incoming(vec![frame(DEVICE, reprog, 0x00, &[0, 0x53])]);
        let mut session = session_with(fake);
        let stop = AtomicBool::new(false);
        let mut seen = Vec::new();
        let mut config = config();
        config.pointer.dpi = 2400;
        let served = session.serve(&config, &stop, |notice| {
            if let Notice::Button(event) = notice {
                seen.push((event.control, event.pressed));
                // The mouse is put down after the press has been seen.
                stop.store(true, Ordering::Relaxed);
            }
        });
        assert!(served.is_ok(), "{served:?}");
        assert_eq!(
            seen,
            [(MouseControlId::Back, true), (MouseControlId::Back, false)],
            "the button still held when stopping is released"
        );
        assert_eq!(state(&session).dpi, 2400);
        assert!(state(&session).diverted.is_empty());
    }

    #[test]
    fn serving_reports_what_the_mouse_refused() {
        let device = FakeDevice::mx_master_4().with_broken_feature(crate::features::ADJUSTABLE_DPI);
        let mut session = session(device);
        let stop = AtomicBool::new(true);
        let mut problems = Vec::new();
        session
            .serve(&config(), &stop, |notice| {
                if let Notice::Problem(problem) = notice {
                    problems.push(problem.setting);
                }
            })
            .unwrap();
        assert_eq!(problems, ["sensitivity"]);
    }

    #[test]
    fn serving_ends_with_an_error_when_the_mouse_is_gone() {
        let fake = FakeLink::direct(FakeDevice::mx_master_4()).with_broken_reads();
        let mut session = session_with(fake);
        let stop = AtomicBool::new(false);
        let served = session.serve(&config(), &stop, |_| {});
        assert!(matches!(served, Err(HidppError::Channel(_))), "{served:?}");
        assert!(state(&session).diverted.is_empty());
    }

    #[test]
    fn serving_with_a_configuration_of_another_model_ends_at_once() {
        let mut session = session(FakeDevice::mx_master_4());
        let other = MouseConfig::default_for("serial", MouseModel::MxAnywhere3S);
        let stop = AtomicBool::new(false);
        let served = session.serve(&other, &stop, |_| {});
        assert!(
            matches!(served, Err(HidppError::InvalidArgument(_))),
            "{served:?}"
        );
    }

    #[test]
    fn serving_reports_a_refusal_to_give_back_and_goes_on() {
        let device = FakeDevice::mx_master_4().with_broken_feature(REPROG_CONTROLS_V4);
        let mut session = session(device);
        let stop = AtomicBool::new(true);
        let mut problems = Vec::new();
        let served = session.serve(&config(), &stop, |notice| {
            if let Notice::Problem(problem) = notice {
                problems.push(problem.setting);
            }
        });
        assert_eq!(problems.first(), Some(&"buttons left by a previous run"));
        assert!(served.is_err(), "the final give back is refused too");
    }

    #[test]
    fn a_failed_final_give_back_is_returned_after_the_releases_were_reported() {
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        let (mut session, shared) = shared_session(device);
        shared
            .0
            .borrow_mut()
            .push_incoming([frame(DEVICE, reprog, 0x00, &[0, 0x53])]);
        let stop = AtomicBool::new(false);
        let mut seen = Vec::new();
        let served = session.serve(&config(), &stop, |notice| {
            if let Notice::Button(event) = notice {
                seen.push(event.pressed);
                if event.pressed {
                    shared.0.borrow_mut().refuse_writes();
                    stop.store(true, Ordering::Relaxed);
                }
            }
        });
        assert_eq!(seen, [true, false], "the release is reported first");
        assert!(matches!(served, Err(HidppError::Channel(_))), "{served:?}");
    }

    #[test]
    fn a_session_whose_mouse_stopped_answering_does_not_retry_when_dropped() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        session.apply(&config()).unwrap();
        shared.0.borrow_mut().fall_silent();
        assert!(matches!(session.give_back(), Err(HidppError::Timeout(_))));
        let written = shared.0.borrow().written().len();
        drop(session);
        assert_eq!(shared.0.borrow().written().len(), written);
    }

    #[test]
    fn a_sleeping_mouse_is_woken_before_its_buttons_are_given_back() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        session.apply(&config()).unwrap();
        assert_eq!(diverted(&shared), 1);
        // Two requests go unanswered while the mouse wakes up.
        shared.0.borrow_mut().sleep_for(2);
        session.give_back().unwrap();
        assert_eq!(diverted(&shared), 0);
    }

    #[test]
    fn a_sleeping_mouse_is_woken_before_a_configuration_is_applied() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        shared.0.borrow_mut().sleep_for(2);
        let problems = session.apply(&config()).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(diverted(&shared), 1);
    }

    #[test]
    fn dropping_the_session_of_a_sleeping_mouse_still_gives_the_buttons_back() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        session.apply(&config()).unwrap();
        shared.0.borrow_mut().sleep_for(2);
        drop(session);
        assert_eq!(diverted(&shared), 0);
    }

    #[test]
    fn a_mouse_that_never_answers_is_given_up_after_a_few_attempts() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        session.apply(&config()).unwrap();
        shared.0.borrow_mut().fall_silent();
        let written = shared.0.borrow().written().len();
        assert!(matches!(session.give_back(), Err(HidppError::Timeout(_))));
        let attempts = shared.0.borrow().written().len() - written;
        assert!(
            attempts < 3 + 20,
            "a gone mouse is not asked for every button for long: {attempts}"
        );
    }
}
