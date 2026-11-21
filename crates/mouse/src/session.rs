//! A mouse in use: its configuration applied, its diverted buttons followed,
//! everything given back at the end.
//!
//! The session owns the [`MouseLink`] and the state that survives between
//! notifications. The caller (a thread per mouse in the manager) decides when
//! to apply a configuration, to wait for buttons and to release them.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hubergamof_protocol::{
    DPI_RANGE, DPI_STEP, HapticLevel, HapticWaveform, MouseConfig, MouseControlId, MouseModel,
    PointerSettings, ThumbWheelSettings, WHEEL_THRESHOLD_RANGE, WheelMode, WheelSettings,
};

use crate::channel::{Channel, HidppError};
use crate::controls::cid;
use crate::frame::{LinkChange, Notification};
use crate::link::{
    ButtonEvent, ButtonTracker, HapticState, MouseLink, WheelState, haptic_strength,
};

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
    /// A configuration was applied: the problems reported since the previous
    /// `Applied` are all there were, none meaning that the mouse took it all.
    Applied,
}

/// What the person asks of a mouse while it is served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Apply a configuration.
    Apply(MouseConfig),
    /// Switch the wheel between ratchet and free spin.
    ToggleWheelMode,
    /// Play a waveform on the haptic panel, at the strength configured.
    PlayHaptic(HapticWaveform),
}

/// How long [`MouseSession::serve`] waits for the buttons before looking at
/// the stop request and at what is asked of the mouse again: the delay with
/// which it reacts to them (a vibration answers the pointer crossing a petal
/// of the radial menu, so it must not lag).
const SERVE_STEP: Duration = Duration::from_millis(40);

/// How many updates in a row may find the mouse asleep before it is given up.
const MAX_WAKE_FAILURES: u32 = 3;

/// What of a configuration the mouse itself is told: its sensitivity, wheels,
/// haptic strength and which controls are diverted. The action of a button is
/// ours.
fn hardware_of(
    config: &MouseConfig,
) -> (
    PointerSettings,
    WheelSettings,
    ThumbWheelSettings,
    HapticLevel,
    BTreeSet<MouseControlId>,
) {
    (
        config.pointer,
        config.wheel,
        config.thumb_wheel,
        config.haptics.level,
        config.bindings.keys().copied().collect(),
    )
}

/// Bit of the wheel flags byte for high resolution.
const HIRES_BIT: u8 = 0x02;
/// Bit of the wheel flags byte for natural scrolling.
const NATURAL_BIT: u8 = 0x04;
/// Bit of the second thumb wheel byte for the reversed direction.
const THUMB_INVERT_BIT: u8 = 0x01;
/// The wheel mode byte of a ratchet.
const WHEEL_RATCHET: u8 = 2;
/// The wheel mode byte of a free spinning wheel.
const WHEEL_FREE: u8 = 1;
/// The auto-disengage value of a wheel that never frees itself.
const NEVER_FREE: u8 = 255;

/// What the mouse was set to, each part `None` when the mouse lacks it or
/// would not say: what is given back at the end, and what a configuration
/// created for the first time adopts so that serving a mouse changes nothing
/// the person did not ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MouseSettings {
    dpi: Option<u16>,
    wheel: Option<WheelState>,
    hires: Option<u8>,
    thumb: Option<[u8; 2]>,
    haptic: Option<HapticState>,
}

impl MouseSettings {
    /// The configuration that keeps these settings, the buttons being the
    /// defaults'.
    #[must_use]
    pub fn to_config(&self, serial: &str, model: MouseModel) -> MouseConfig {
        let mut config = MouseConfig::default_for(serial, model);
        if let Some(dpi) = self.dpi {
            let clamped = dpi.clamp(*DPI_RANGE.start(), *DPI_RANGE.end());
            let steps = (clamped - DPI_RANGE.start() + DPI_STEP / 2) / DPI_STEP;
            config.pointer.dpi = (DPI_RANGE.start() + steps * DPI_STEP).min(*DPI_RANGE.end());
        }
        if let Some(wheel) = self.wheel {
            let (mode, threshold) = wheel_mode_of(wheel);
            config.wheel.mode = mode;
            if let Some(threshold) = threshold {
                config.wheel.auto_threshold = threshold;
            }
        }
        if let Some(flags) = self.hires {
            config.wheel.hi_res = flags & HIRES_BIT != 0;
            config.wheel.natural = flags & NATURAL_BIT != 0;
        }
        if let Some([_, second]) = self.thumb {
            config.thumb_wheel.invert = second & THUMB_INVERT_BIT != 0;
        }
        if let Some(haptic) = self.haptic {
            config.haptics.level = haptic_level_of(haptic);
        }
        config
    }
}

/// How the wheel is set, as the contract says it: free spin, ratchet, or
/// ratchet that frees itself above a speed (with that speed).
fn wheel_mode_of(wheel: WheelState) -> (WheelMode, Option<u8>) {
    if wheel.mode != WHEEL_RATCHET {
        return (WheelMode::Free, None);
    }
    if wheel.auto_disengage == NEVER_FREE {
        return (WheelMode::Ratchet, None);
    }
    let threshold = wheel
        .auto_disengage
        .clamp(*WHEEL_THRESHOLD_RANGE.start(), *WHEEL_THRESHOLD_RANGE.end());
    (WheelMode::Auto, Some(threshold))
}

/// The level whose strength is the nearest to the mouse's.
fn haptic_level_of(haptic: HapticState) -> HapticLevel {
    if !haptic.enabled {
        return HapticLevel::Off;
    }
    [
        HapticLevel::Subtle,
        HapticLevel::Low,
        HapticLevel::Medium,
        HapticLevel::High,
    ]
    .into_iter()
    .min_by_key(|level| {
        haptic_strength(*level).map_or(u8::MAX, |strength| strength.abs_diff(haptic.level))
    })
    .unwrap_or(HapticLevel::Medium)
}

/// What a read gave, `None` when the mouse lacks the setting or refuses to say;
/// an error only when it stops answering.
fn readable<T>(read: Result<T, HidppError>) -> Result<Option<T>, HidppError> {
    match read {
        Ok(value) => Ok(Some(value)),
        Err(error) if is_fatal(&error) => Err(error),
        Err(_) => Ok(None),
    }
}

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
    /// The settings of the mouse before this session changed them.
    baseline: Option<MouseSettings>,
    /// The configuration last applied, to write only what an update changes.
    applied: Option<MouseConfig>,
    /// An update the mouse was too sleepy to take, to try again.
    pending: Option<MouseConfig>,
    /// Updates in a row the mouse did not wake up for.
    wake_failures: u32,
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
            baseline: None,
            applied: None,
            pending: None,
            wake_failures: 0,
            buttons_diverted: false,
        }
    }

    /// Reads what the mouse is set to now (waking it if need be). A part the
    /// mouse refuses to give is left out.
    ///
    /// # Errors
    /// [`HidppError::Timeout`] or [`HidppError::Channel`] when the mouse does
    /// not answer.
    pub fn capture(&mut self) -> Result<MouseSettings, HidppError> {
        let _ = self.link.wake();
        let link = &mut self.link;
        Ok(MouseSettings {
            dpi: readable(link.dpi())?,
            wheel: readable(link.wheel())?,
            hires: readable(link.hires_flags())?,
            thumb: readable(link.thumb_flags())?,
            haptic: if link.has_haptics() {
                readable(link.haptic_state())?
            } else {
                None
            },
        })
    }

    /// Puts back what [`MouseSession::serve`] found: every setting it
    /// captured, each one tried. Nothing to do before a capture.
    ///
    /// # Errors
    /// The first [`HidppError`] met.
    pub fn restore_settings(&mut self) -> Result<(), HidppError> {
        let Some(baseline) = self.baseline.take() else {
            return Ok(());
        };
        let _ = self.link.wake();
        let link = &mut self.link;
        let mut first_error = None;
        // Each setting is tried, until the mouse stops answering.
        let mut tried = |result: Option<Result<(), HidppError>>| match result {
            Some(Err(error)) => {
                let go_on = !is_fatal(&error);
                first_error.get_or_insert(error);
                go_on
            }
            _ => true,
        };
        let _ = tried(baseline.dpi.map(|dpi| link.write_dpi(dpi)))
            && tried(baseline.wheel.map(|wheel| link.write_wheel(wheel)))
            && tried(baseline.hires.map(|flags| link.write_hires_flags(flags)))
            && tried(baseline.thumb.map(|flags| link.write_thumb_flags(flags)))
            && tried(
                baseline
                    .haptic
                    .map(|haptic| link.set_haptic(haptic.enabled.then_some(haptic.level))),
            );
        first_error.map_or(Ok(()), Err)
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
        self.applied = Some(config.clone());
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
        if let Some(change) = frame.as_deref().and_then(LinkChange::parse)
            && change.device == self.link.device()
        {
            events.extend(self.link_changed(change));
        }
        let held = frame
            .and_then(|bytes| Notification::parse(&bytes))
            .and_then(|notification| self.link.held_buttons(&notification));
        if let Some(held) = held {
            events.extend(self.tracker.update(&held));
        }
        Ok(events)
    }

    /// The mouse was switched off or is back. Nothing it was told survives a
    /// power cycle (the buttons are no longer diverted): what was held is
    /// released, and once back the configuration is applied again, in full.
    fn link_changed(&mut self, change: LinkChange) -> Vec<ButtonEvent> {
        let released = self.tracker.release_all();
        if change.established {
            self.wake_failures = 0;
            self.pending = self.applied.take().or_else(|| self.pending.take());
        }
        released
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
        notify: impl FnMut(Notice),
    ) -> Result<(), HidppError> {
        self.serve_with_updates(config, stop, || None, notify)
    }

    /// Like [`MouseSession::serve`], and applies the configuration `updates`
    /// gives (asked about at every turn of the loop, at least every 200 ms)
    /// to the mouse being served. An update that cannot be applied is
    /// reported and the service goes on with what was applied.
    ///
    /// The buttons held when an update comes are reported as released: the
    /// update may have given their control back.
    ///
    /// # Errors
    /// See [`MouseSession::serve`].
    pub fn serve_with_updates(
        &mut self,
        config: &MouseConfig,
        stop: &AtomicBool,
        mut updates: impl FnMut() -> Option<MouseConfig>,
        notify: impl FnMut(Notice),
    ) -> Result<(), HidppError> {
        self.serve_with_requests(config, stop, || updates().map(Request::Apply), notify)
    }

    /// Like [`MouseSession::serve_with_updates`], for any [`Request`]: what
    /// the person asked of the mouse (a configuration, a toggle of the wheel)
    /// while it is served.
    ///
    /// # Errors
    /// See [`MouseSession::serve`].
    pub fn serve_with_requests(
        &mut self,
        config: &MouseConfig,
        stop: &AtomicBool,
        mut requests: impl FnMut() -> Option<Request>,
        mut notify: impl FnMut(Notice),
    ) -> Result<(), HidppError> {
        let served = self.serve_buttons(config, stop, &mut requests, &mut notify);
        for event in self.release_held() {
            notify(Notice::Button(event));
        }
        let given_back = self.give_back();
        let restored = self.restore_settings();
        served.and(given_back).and(restored)
    }

    fn serve_buttons(
        &mut self,
        config: &MouseConfig,
        stop: &AtomicBool,
        requests: &mut impl FnMut() -> Option<Request>,
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
        self.baseline = Some(self.capture()?);
        for problem in self.apply(config)? {
            notify(Notice::Problem(problem));
        }
        notify(Notice::Applied);
        while !stop.load(Ordering::Relaxed) {
            let update = match requests() {
                Some(Request::Apply(update)) => {
                    // Newer than one that had to wait.
                    self.pending = None;
                    Some(update)
                }
                Some(Request::ToggleWheelMode) => {
                    self.toggle_wheel_mode(notify)?;
                    None
                }
                Some(Request::PlayHaptic(waveform)) => {
                    self.play_haptic(waveform, notify)?;
                    None
                }
                None => self.pending.take(),
            };
            if let Some(update) = update {
                self.apply_update(&update, notify)?;
                if stop.load(Ordering::Relaxed) {
                    break;
                }
            }
            for event in self.poll(SERVE_STEP)? {
                notify(Notice::Button(event));
            }
        }
        Ok(())
    }

    /// Plays `waveform`. What the mouse refuses (a waveform it does not have,
    /// a panel it does not answer for) is reported; only a mouse that stops
    /// answering ends the service.
    fn play_haptic(
        &mut self,
        waveform: HapticWaveform,
        notify: &mut impl FnMut(Notice),
    ) -> Result<(), HidppError> {
        match self.link.play_haptic(waveform) {
            Ok(()) => Ok(()),
            Err(error) if is_fatal(&error) => Err(error),
            Err(error) => {
                notify(Notice::Problem(ApplyProblem {
                    setting: "haptic",
                    error,
                }));
                Ok(())
            }
        }
    }

    /// Switches the wheel between ratchet and free spin, as it is now (a
    /// toggle does not touch the configuration, and the mouse's own mode is
    /// back at the end). What the mouse refuses is reported.
    fn toggle_wheel_mode(&mut self, notify: &mut impl FnMut(Notice)) -> Result<(), HidppError> {
        let toggled = self.link.wheel().and_then(|wheel| {
            let mode = if wheel.mode == WHEEL_RATCHET {
                WHEEL_FREE
            } else {
                WHEEL_RATCHET
            };
            self.link.write_wheel(WheelState { mode, ..wheel })
        });
        match toggled {
            Ok(()) => Ok(()),
            Err(error) if is_fatal(&error) => Err(error),
            Err(error) => {
                notify(Notice::Problem(ApplyProblem {
                    setting: "wheel mode",
                    error,
                }));
                Ok(())
            }
        }
    }

    /// Applies a configuration received while serving. What the mouse refuses
    /// is reported; only a mouse that stops answering ends the service.
    fn apply_update(
        &mut self,
        config: &MouseConfig,
        notify: &mut impl FnMut(Notice),
    ) -> Result<(), HidppError> {
        if self
            .applied
            .as_ref()
            .is_some_and(|applied| hardware_of(applied) == hardware_of(config))
        {
            // Only what the mouse does not know changed (the action of a
            // button): nothing to write, nothing to release.
            self.applied = Some(config.clone());
            return Ok(());
        }
        match self.link.wake() {
            Ok(()) => self.wake_failures = 0,
            Err(HidppError::Timeout(_)) if self.wake_failures < MAX_WAKE_FAILURES => {
                // Asleep: tried again at the next turn, the service goes on.
                self.wake_failures += 1;
                self.pending = Some(config.clone());
                return Ok(());
            }
            Err(error) => return Err(error),
        }
        // The controls that stop being diverted, or start: a button held
        // while its control is given back will not say it is released.
        let changed = self.applied.as_ref().map_or_else(Vec::new, |applied| {
            applied
                .bindings
                .keys()
                .copied()
                .collect::<BTreeSet<_>>()
                .symmetric_difference(&config.bindings.keys().copied().collect())
                .copied()
                .collect()
        });
        let released: Vec<ButtonEvent> = changed
            .into_iter()
            .filter_map(|control| self.tracker.release(control))
            .collect();
        let applied = self.apply(config);
        for event in released {
            notify(Notice::Button(event));
        }
        match applied {
            Ok(problems) => problems
                .into_iter()
                .for_each(|problem| notify(Notice::Problem(problem))),
            Err(error) if is_fatal(&error) => return Err(error),
            Err(error) => notify(Notice::Problem(ApplyProblem {
                setting: "new configuration",
                error,
            })),
        }
        notify(Notice::Applied);
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
        let _ = self.restore_settings();
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

    const LOST: [u8; 7] = [0x10, DEVICE, 0x41, 0x10, 0x42, 0x42, 0xB0];
    const BACK: [u8; 7] = [0x10, DEVICE, 0x41, 0x10, 0x02, 0x42, 0xB0];

    #[test]
    fn a_mouse_switched_off_releases_what_was_held_and_is_configured_again_when_back() {
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        let fake = FakeLink::direct(device).with_incoming(vec![
            frame(DEVICE, reprog, 0x00, &[0, 0x53]),
            LOST.to_vec(),
            BACK.to_vec(),
        ]);
        let mut session = session_with(fake);
        session.apply(&config()).unwrap();
        assert_eq!(session.poll(Duration::from_millis(1)).unwrap().len(), 1);
        let released = session.poll(Duration::from_millis(1)).unwrap();
        assert_eq!(
            released,
            [ButtonEvent {
                control: MouseControlId::Back,
                pressed: false
            }]
        );
        assert!(
            session.applied.is_some(),
            "still switched off: nothing to redo yet"
        );
        session.poll(Duration::from_millis(1)).unwrap();
        assert!(session.applied.is_none());
        assert_eq!(session.pending.as_ref(), Some(&config()));
    }

    #[test]
    fn the_link_of_another_mouse_of_the_receiver_changes_nothing_here() {
        let mut other = LOST;
        other[1] = DEVICE.wrapping_add(1);
        let mut back = BACK;
        back[1] = other[1];
        let fake = FakeLink::direct(FakeDevice::mx_master_4())
            .with_incoming(vec![other.to_vec(), back.to_vec()]);
        let mut session = session_with(fake);
        session.apply(&config()).unwrap();
        session.poll(Duration::from_millis(1)).unwrap();
        session.poll(Duration::from_millis(1)).unwrap();
        assert!(session.applied.is_some());
        assert!(session.pending.is_none());
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
        let (mut session, shared) = shared_session(device);
        shared
            .0
            .borrow_mut()
            .push_incoming([frame(DEVICE, reprog, 0x00, &[0, 0x53])]);
        let stop = AtomicBool::new(false);
        let mut seen = Vec::new();
        let mut config = config();
        config.pointer.dpi = 2400;
        let mut during = None;
        let served = session.serve(&config, &stop, |notice| {
            if let Notice::Button(event) = notice {
                seen.push((event.control, event.pressed));
                if event.pressed {
                    during = Some(shared.0.borrow().device_at(DEVICE).unwrap().state.clone());
                }
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
        let during = during.unwrap();
        assert_eq!(
            during.dpi, 2400,
            "the configuration was applied while serving"
        );
        assert_eq!(during.diverted.len(), 1, "the menu button");
        let state = shared.0.borrow().device_at(DEVICE).unwrap().state.clone();
        assert_eq!(state.dpi, 1600, "the sensitivity it had is back");
        assert!(state.diverted.is_empty());
    }

    #[test]
    fn serving_puts_every_setting_back_as_it_found_it() {
        let mut device = FakeDevice::mx_master_4();
        // Settings that are nobody's defaults.
        device.state.dpi = 1250;
        device.state.wheel = [2, 17, 33];
        device.state.hires = 0x09;
        device.state.thumb = [0x40, 0x00];
        device.state.haptic = [1, 61];
        let (mut session, shared) = shared_session(device);
        let before = shared.0.borrow().device_at(DEVICE).unwrap().state.clone();
        let mut changed = config();
        changed.pointer.dpi = 2400;
        changed.wheel.mode = WheelMode::Free;
        changed.wheel.hi_res = false;
        changed.wheel.natural = true;
        changed.thumb_wheel.invert = true;
        changed.haptics.level = HapticLevel::High;
        let stop = AtomicBool::new(true);
        session.serve(&changed, &stop, |_| {}).unwrap();
        let after = shared.0.borrow().device_at(DEVICE).unwrap().state.clone();
        assert_eq!(after.dpi, before.dpi);
        assert_eq!(after.wheel, before.wheel);
        assert_eq!(after.hires, before.hires);
        assert_eq!(after.thumb, before.thumb);
        assert_eq!(after.haptic, before.haptic);
    }

    #[test]
    fn dropping_a_serving_session_puts_the_settings_back_too() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        let before = shared.0.borrow().device_at(DEVICE).unwrap().state.clone();
        session.baseline = Some(session.capture().unwrap());
        let mut changed = config();
        changed.pointer.dpi = 2400;
        session.apply(&changed).unwrap();
        assert_eq!(shared.0.borrow().device_at(DEVICE).unwrap().state.dpi, 2400);
        drop(session);
        let after = shared.0.borrow().device_at(DEVICE).unwrap().state.clone();
        assert_eq!((after.dpi, after.diverted.len()), (before.dpi, 0));
    }

    #[test]
    fn a_gone_mouse_is_not_asked_for_every_setting_when_restoring() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        session.baseline = Some(session.capture().unwrap());
        shared.0.borrow_mut().fall_silent();
        let written = shared.0.borrow().written().len();
        assert!(session.restore_settings().is_err());
        // The attempts to wake it, then the first write, which ends it.
        assert_eq!(shared.0.borrow().written().len() - written, 3 + 1);
    }

    #[test]
    fn the_settings_of_a_mouse_become_the_configuration_that_keeps_them() {
        let mut device = FakeDevice::mx_master_4();
        device.state.dpi = 1230;
        device.state.wheel = [2, 10, 75];
        device.state.hires = 0x07;
        device.state.thumb = [0, 1];
        device.state.haptic = [1, 60];
        let mut session = session(device);
        let settings = session.capture().unwrap();
        let config = settings.to_config("serial", MouseModel::MxMaster4);
        assert_eq!(config.pointer.dpi, 1250, "the nearest step");
        assert_eq!(
            (config.wheel.mode, config.wheel.auto_threshold),
            (WheelMode::Auto, 10)
        );
        assert!(config.wheel.hi_res && config.wheel.natural);
        assert!(config.thumb_wheel.invert);
        assert_eq!(config.haptics.level, HapticLevel::Low);
        config.validate().unwrap();
        // The buttons are the defaults'.
        assert_eq!(config.bindings.len(), 1);
    }

    #[test]
    fn the_wheel_modes_of_a_mouse_are_told_apart() {
        let of = |mode, auto_disengage| {
            let state = WheelState {
                mode,
                auto_disengage,
                torque: 50,
            };
            wheel_mode_of(state)
        };
        assert_eq!(of(1, 10), (WheelMode::Free, None));
        assert_eq!(of(2, 255), (WheelMode::Ratchet, None));
        assert_eq!(of(2, 20), (WheelMode::Auto, Some(20)));
        assert_eq!(
            of(2, 0),
            (WheelMode::Auto, Some(1)),
            "kept within the range"
        );
        assert_eq!(of(2, 200), (WheelMode::Auto, Some(50)));
    }

    #[test]
    fn the_haptic_strength_of_a_mouse_gives_the_nearest_level() {
        let level = |enabled, level| haptic_level_of(HapticState { enabled, level });
        assert_eq!(level(false, 75), HapticLevel::Off);
        assert_eq!(level(true, 0), HapticLevel::Subtle);
        assert_eq!(level(true, 40), HapticLevel::Low);
        assert_eq!(level(true, 75), HapticLevel::Medium);
        assert_eq!(level(true, 100), HapticLevel::High);
    }

    #[test]
    fn serving_reports_what_the_mouse_refused() {
        let device = FakeDevice::mx_master_4().with_broken_feature(crate::features::ADJUSTABLE_DPI);
        let mut session = session(device);
        let stop = AtomicBool::new(true);
        let mut told = Vec::new();
        session
            .serve(&config(), &stop, |notice| match notice {
                Notice::Problem(problem) => told.push(problem.setting),
                Notice::Applied => told.push("applied"),
                Notice::Button(_) => {}
            })
            .unwrap();
        // The end of the application comes after its problems.
        assert_eq!(told, ["sensitivity", "applied"]);
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

    /// What serving with updates gave: the sensitivity and the number of
    /// diverted buttons at each turn of the loop, the problems reported, and
    /// how the service ended.
    type Served = (Vec<(u16, usize)>, Vec<&'static str>, Result<(), HidppError>);

    /// Serves, giving `updates` in order, one per turn of the loop, and stops
    /// once they are given.
    fn serve_updates(
        session: &mut MouseSession<Shared>,
        shared: &Shared,
        updates: Vec<MouseConfig>,
    ) -> Served {
        let stop = AtomicBool::new(false);
        let mut remaining = updates.into_iter();
        let mut seen = Vec::new();
        let mut problems = Vec::new();
        let served = session.serve_with_updates(
            &config(),
            &stop,
            || {
                let state = shared.0.borrow().device_at(DEVICE).unwrap().state.clone();
                seen.push((state.dpi, state.diverted.len()));
                let next = remaining.next();
                if next.is_none() {
                    stop.store(true, Ordering::Relaxed);
                }
                next
            },
            |notice| {
                if let Notice::Problem(problem) = notice {
                    problems.push(problem.setting);
                }
            },
        );
        (seen, problems, served)
    }

    #[test]
    fn a_configuration_received_while_serving_is_applied_to_the_mouse() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        let mut update = config();
        update.pointer.dpi = 2400;
        update
            .bindings
            .insert(MouseControlId::Back, ButtonAction::None);
        let (seen, problems, served) = serve_updates(&mut session, &shared, vec![update]);
        served.unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        // The first turn finds the first configuration applied, the second
        // the update: new sensitivity, one more button diverted.
        assert_eq!(seen, [(1600, 1), (2400, 2)]);
        let state = shared.0.borrow().device_at(DEVICE).unwrap().state.clone();
        assert_eq!(
            (state.dpi, state.diverted.len()),
            (1600, 0),
            "all given back"
        );
    }

    #[test]
    fn successive_updates_are_applied_in_order() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        let with_dpi = |dpi| {
            let mut update = config();
            update.pointer.dpi = dpi;
            update
        };
        let (seen, _, served) =
            serve_updates(&mut session, &shared, vec![with_dpi(1200), with_dpi(1400)]);
        served.unwrap();
        assert_eq!(
            seen.iter().map(|(dpi, _)| *dpi).collect::<Vec<_>>(),
            [1600, 1200, 1400]
        );
    }

    #[test]
    fn an_update_the_mouse_cannot_take_is_reported_and_the_service_goes_on() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        let other_model = MouseConfig::default_for("serial", MouseModel::MxAnywhere3S);
        let (seen, problems, served) = serve_updates(&mut session, &shared, vec![other_model]);
        served.unwrap();
        assert_eq!(problems, ["new configuration"]);
        assert_eq!(seen.len(), 2, "it went on to the next turn");
        assert_eq!(seen[1].0, 1600, "nothing of the refused update was written");
    }

    #[test]
    fn an_update_to_a_mouse_that_stopped_answering_ends_the_service() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        let stop = AtomicBool::new(false);
        let mut changed = config();
        changed.pointer.dpi = 2400;
        let mut update = Some(changed);
        let served = session.serve_with_updates(
            &config(),
            &stop,
            || {
                shared.0.borrow_mut().fall_silent();
                update.take()
            },
            |_| {},
        );
        assert!(matches!(served, Err(HidppError::Timeout(_))), "{served:?}");
    }

    #[test]
    fn the_buttons_held_when_an_update_comes_are_reported_released_by_the_update() {
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        let (mut session, shared) = shared_session(device);
        // The button goes down, and comes up (physically) later.
        shared.0.borrow_mut().push_incoming([
            frame(DEVICE, reprog, 0x00, &[0, 0x53]),
            frame(DEVICE, reprog, 0x00, &[]),
        ]);
        let stop = AtomicBool::new(false);
        let turn = std::cell::Cell::new(0);
        let mut events = Vec::new();
        session
            .serve_with_updates(
                &config(),
                &stop,
                || {
                    turn.set(turn.get() + 1);
                    match turn.get() {
                        2 => {
                            let mut update = config();
                            update
                                .bindings
                                .insert(MouseControlId::Back, ButtonAction::None);
                            Some(update)
                        }
                        4 => {
                            stop.store(true, Ordering::Relaxed);
                            None
                        }
                        _ => None,
                    }
                },
                |notice| {
                    if let Notice::Button(event) = notice {
                        events.push((event.pressed, turn.get()));
                    }
                },
            )
            .unwrap();
        // Released at the turn of the update (not at the end), and the
        // physical release that follows is not reported a second time.
        assert_eq!(events, [(true, 1), (false, 2)]);
    }

    #[test]
    fn an_update_the_mouse_partly_refuses_is_applied_for_the_rest_and_reported() {
        let device = FakeDevice::mx_master_4().with_broken_feature(crate::features::ADJUSTABLE_DPI);
        let (mut session, shared) = shared_session(device);
        let mut update = config();
        update.pointer.dpi = 2400;
        update.haptics.level = HapticLevel::High;
        update
            .bindings
            .insert(MouseControlId::Back, ButtonAction::None);
        let (seen, problems, served) = serve_updates(&mut session, &shared, vec![update]);
        served.unwrap();
        // The sensitivity is refused at the start and by the update.
        assert_eq!(problems, ["sensitivity", "sensitivity"]);
        assert_eq!(seen[1].1, 2, "the buttons of the update are diverted");
        let state = shared.0.borrow().device_at(DEVICE).unwrap().state.clone();
        assert!(state.diverted.is_empty(), "and given back at the end");
    }

    #[test]
    fn an_update_pending_when_the_service_is_stopped_is_not_asked_for() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        let stop = AtomicBool::new(true);
        let served = session.serve_with_updates(
            &config(),
            &stop,
            || panic!("asked for an update after the stop"),
            |_| {},
        );
        served.unwrap();
        let state = shared.0.borrow().device_at(DEVICE).unwrap().state.clone();
        assert_eq!((state.dpi, state.diverted.len()), (1600, 0));
    }

    #[test]
    fn an_update_that_only_changes_the_action_of_a_button_writes_nothing_and_releases_nothing() {
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        let (mut session, shared) = shared_session(device);
        shared
            .0
            .borrow_mut()
            .push_incoming([frame(DEVICE, reprog, 0x00, &[0, 0x53])]);
        let mut bound = config();
        bound.bindings.insert(
            MouseControlId::Back,
            ButtonAction::Shortcut {
                keys: "Ctrl+M".to_owned(),
            },
        );
        let stop = AtomicBool::new(false);
        let turn = std::cell::Cell::new(0);
        let mut written = (0, 0);
        let mut events = Vec::new();
        session
            .serve_with_updates(
                &bound,
                &stop,
                || {
                    turn.set(turn.get() + 1);
                    match turn.get() {
                        2 => {
                            written.0 = shared.0.borrow().written().len();
                            let mut update = bound.clone();
                            update.bindings.insert(
                                MouseControlId::Back,
                                ButtonAction::Click {
                                    button: hubergamof_protocol::MouseButton::Middle,
                                },
                            );
                            Some(update)
                        }
                        3 => {
                            written.1 = shared.0.borrow().written().len();
                            stop.store(true, Ordering::Relaxed);
                            None
                        }
                        _ => None,
                    }
                },
                |notice| {
                    if let Notice::Button(event) = notice {
                        events.push((event.pressed, turn.get()));
                    }
                },
            )
            .unwrap();
        // The turn of the update wrote nothing; the end gave the buttons back.
        let link = shared.0.borrow();
        let during = &link.written()[written.0..written.1];
        assert!(
            during
                .iter()
                .all(|report| report.get(2) != Some(&reprog) || report[3] >> 4 != 3),
            "no button was written again"
        );
        assert_eq!(events[0], (true, 1));
        assert_eq!(
            events.get(1),
            Some(&(false, 3)),
            "released only when the service ended, not by the update"
        );
    }

    #[test]
    fn an_update_the_mouse_was_too_sleepy_for_is_applied_at_a_later_turn() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        let stop = AtomicBool::new(false);
        let turn = std::cell::Cell::new(0);
        let mut seen = Vec::new();
        let mut update = config();
        update.pointer.dpi = 2400;
        let served = session.serve_with_updates(
            &config(),
            &stop,
            || {
                turn.set(turn.get() + 1);
                let dpi = shared.0.borrow().device_at(DEVICE).unwrap().state.dpi;
                match turn.get() {
                    2 => {
                        // Asleep for the three pings of the wake up.
                        shared.0.borrow_mut().sleep_for(3);
                        Some(update.clone())
                    }
                    3 => {
                        seen.push(dpi);
                        None
                    }
                    4 => {
                        seen.push(dpi);
                        stop.store(true, Ordering::Relaxed);
                        None
                    }
                    _ => None,
                }
            },
            |_| {},
        );
        served.unwrap();
        assert_eq!(
            seen,
            [1600, 2400],
            "the service went on and the update came"
        );
    }

    #[test]
    fn a_mouse_that_never_wakes_for_an_update_ends_the_service() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        let stop = AtomicBool::new(false);
        let turn = std::cell::Cell::new(0);
        let mut update = config();
        update.pointer.dpi = 2400;
        let served = session.serve_with_updates(
            &config(),
            &stop,
            || {
                turn.set(turn.get() + 1);
                (turn.get() == 2).then(|| {
                    shared.0.borrow_mut().sleep_for(10_000);
                    update.clone()
                })
            },
            |_| {},
        );
        assert!(matches!(served, Err(HidppError::Timeout(_))), "{served:?}");
        assert!(
            turn.get() < 10,
            "given up after a few turns: {}",
            turn.get()
        );
    }

    #[test]
    fn the_wheel_is_toggled_between_ratchet_and_free_spin_and_given_back() {
        let (mut session, shared) = shared_session(FakeDevice::mx_master_4());
        let stop = AtomicBool::new(false);
        let turn = std::cell::Cell::new(0);
        let mut modes = Vec::new();
        let served = session.serve_with_requests(
            &config(),
            &stop,
            || {
                turn.set(turn.get() + 1);
                modes.push(shared.0.borrow().device_at(DEVICE).unwrap().state.wheel[0]);
                match turn.get() {
                    2 | 3 => Some(Request::ToggleWheelMode),
                    5 => {
                        stop.store(true, Ordering::Relaxed);
                        None
                    }
                    _ => None,
                }
            },
            |_| {},
        );
        served.unwrap();
        // Ratchet (2) at first, free (1) after one toggle, ratchet after two.
        assert_eq!(modes, [2, 2, 1, 2, 2]);
        let state = shared.0.borrow().device_at(DEVICE).unwrap().state.clone();
        assert_eq!(
            state.wheel,
            [2, 10, 50],
            "the mouse's own settings are back"
        );
    }

    #[test]
    fn a_toggle_keeps_the_rest_of_the_wheel_state() {
        let mut device = FakeDevice::mx_master_4();
        device.state.wheel = [2, 17, 33];
        let (mut session, shared) = shared_session(device);
        let stop = AtomicBool::new(false);
        let turn = std::cell::Cell::new(0);
        let mut during = None;
        session
            .serve_with_requests(
                &config(),
                &stop,
                || {
                    turn.set(turn.get() + 1);
                    match turn.get() {
                        1 => Some(Request::ToggleWheelMode),
                        2 => {
                            during = Some(shared.0.borrow().device_at(DEVICE).unwrap().state.wheel);
                            stop.store(true, Ordering::Relaxed);
                            None
                        }
                        _ => None,
                    }
                },
                |_| {},
            )
            .unwrap();
        // The configuration put its threshold (20); the torque is the mouse's.
        assert_eq!(during, Some([1, 20, 33]));
    }

    /// Serves `device` for two turns, the first returning `request`; the
    /// problems reported, and the ids of the waveforms the mouse played.
    fn serve_one_request(device: FakeDevice, request: &Request) -> (Vec<&'static str>, Vec<u8>) {
        let (mut session, shared) = shared_session(device);
        let stop = AtomicBool::new(false);
        let turn = std::cell::Cell::new(0);
        let mut problems = Vec::new();
        session
            .serve_with_requests(
                &config(),
                &stop,
                || {
                    turn.set(turn.get() + 1);
                    if turn.get() == 1 {
                        Some(request.clone())
                    } else {
                        stop.store(true, Ordering::Relaxed);
                        None
                    }
                },
                |notice| {
                    if let Notice::Problem(problem) = notice {
                        problems.push(problem.setting);
                    }
                },
            )
            .unwrap();
        let played = shared
            .0
            .borrow()
            .device_at(DEVICE)
            .unwrap()
            .state
            .played
            .clone();
        (problems, played)
    }

    #[test]
    fn a_waveform_asked_for_is_played_on_the_mouse() {
        let (problems, played) = serve_one_request(
            FakeDevice::mx_master_4(),
            &Request::PlayHaptic(HapticWaveform::Knock),
        );
        assert_eq!(problems, [] as [&str; 0]);
        assert_eq!(played, [0x0C]);
    }

    #[test]
    fn a_waveform_the_mouse_does_not_have_is_reported_and_the_service_goes_on() {
        let mut device = FakeDevice::mx_master_4();
        device.state.haptic_mask = 0x0000_0001;
        let (problems, played) =
            serve_one_request(device, &Request::PlayHaptic(HapticWaveform::Knock));
        assert_eq!(problems, ["haptic"]);
        assert_eq!(played, [] as [u8; 0]);
    }

    #[test]
    fn a_toggle_the_mouse_refuses_is_reported_and_the_service_goes_on() {
        let device =
            FakeDevice::mx_master_4().with_broken_feature(crate::features::SMART_SHIFT_ENHANCED);
        let (mut session, _shared) = shared_session(device);
        let stop = AtomicBool::new(false);
        let turn = std::cell::Cell::new(0);
        let mut problems = Vec::new();
        let served = session.serve_with_requests(
            &config(),
            &stop,
            || {
                turn.set(turn.get() + 1);
                match turn.get() {
                    1 => Some(Request::ToggleWheelMode),
                    3 => {
                        stop.store(true, Ordering::Relaxed);
                        None
                    }
                    _ => None,
                }
            },
            |notice| {
                if let Notice::Problem(problem) = notice {
                    problems.push(problem.setting);
                }
            },
        );
        served.unwrap();
        assert!(problems.contains(&"wheel mode"), "{problems:?}");
        assert!(turn.get() >= 3, "it went on");
    }
}
