//! What `hubergamof mouse-test` does to a mouse, one action at a time: reads its
//! state, or makes one change the person asked for, and says what happened.
//!
//! Nothing here runs by itself: every write is an action a person typed.

use std::io::{self, Write};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hubergamof_protocol::{HapticLevel, HapticWaveform, MouseControlId, WheelMode};
use thiserror::Error;

use crate::channel::{Channel, HidppError};
use crate::controls::cid;
use crate::frame::Notification;
use crate::link::{ButtonTracker, MouseLink, haptic_strength};

/// How long to wait for a notification before looking at the clock again.
const WATCH_STEP: Duration = Duration::from_millis(200);

/// One thing to do to a mouse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Read and print its state.
    Info,
    /// Play a waveform, after setting the haptic strength when a level is given.
    Haptic {
        /// The pattern.
        waveform: HapticWaveform,
        /// The strength to set first, `None` to leave it.
        level: Option<HapticLevel>,
    },
    /// Set the sensitivity.
    Dpi(u16),
    /// Set how the wheel turns.
    Wheel {
        /// The mode.
        mode: WheelMode,
        /// The threshold of the automatic mode.
        threshold: u8,
    },
    /// Divert the buttons and print their presses for a while, then give them back.
    Buttons(Duration),
    /// Give back every diverted button.
    Release,
}

/// A waveform from what a person typed: its name in any case, with or without
/// `-`, `_` or spaces (`knock`, `sharp-state-change`, `SharpStateChange`).
#[must_use]
pub fn parse_waveform(text: &str) -> Option<HapticWaveform> {
    let simple = |name: &str| -> String {
        name.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    let wanted = simple(text);
    HapticWaveform::ALL
        .into_iter()
        .find(|waveform| simple(&format!("{waveform:?}")) == wanted)
}

/// Why an action failed.
#[derive(Debug, Error)]
pub enum DiagnosticError {
    /// The mouse refused or did not answer.
    #[error(transparent)]
    Mouse(#[from] HidppError),
    /// The output could not be written.
    #[error("cannot write the report: {0}")]
    Output(#[from] io::Error),
}

/// Does the action on the mouse and writes what it found or did. Watching the
/// buttons ends early, giving them back, when `stop` is raised.
///
/// # Errors
/// [`DiagnosticError`] when the mouse refuses, does not answer, or the output
/// cannot be written.
pub fn run<C: Channel>(
    link: &mut MouseLink<C>,
    action: &Action,
    out: &mut impl Write,
    stop: &AtomicBool,
) -> Result<(), DiagnosticError> {
    match action {
        Action::Info => info(link, out),
        Action::Haptic { waveform, level } => haptic(link, *waveform, *level, out),
        Action::Dpi(dpi) => {
            link.set_dpi(*dpi)?;
            writeln!(
                out,
                "Sensitivity set to {dpi} dpi, the mouse reads {} dpi.",
                link.dpi()?
            )?;
            Ok(())
        }
        Action::Wheel { mode, threshold } => {
            link.set_wheel(*mode, *threshold)?;
            let wheel = link.wheel()?;
            writeln!(
                out,
                "Wheel set to {mode:?}: the mouse reads mode {}, auto-disengage {}, torque {}.",
                wheel.mode, wheel.auto_disengage, wheel.torque
            )?;
            Ok(())
        }
        Action::Buttons(duration) => watch_buttons(link, *duration, out, stop),
        Action::Release => {
            link.release_buttons()?;
            writeln!(out, "Every button of the {} is given back.", link.model())?;
            Ok(())
        }
    }
}

fn info<C: Channel>(link: &mut MouseLink<C>, out: &mut impl Write) -> Result<(), DiagnosticError> {
    writeln!(
        out,
        "{} at device index 0x{:02X}",
        link.model(),
        link.device()
    )?;
    let battery = link.battery()?;
    writeln!(
        out,
        "  battery: {:?}, level {:?}, status {}",
        battery.percent, battery.level, battery.status
    )?;
    writeln!(out, "  sensitivity: {} dpi", link.dpi()?)?;
    let wheel = link.wheel()?;
    writeln!(
        out,
        "  wheel: mode {} (1 free, 2 ratchet), auto-disengage {} (255 never), torque {}",
        wheel.mode, wheel.auto_disengage, wheel.torque
    )?;
    match link.hires_flags() {
        Ok(flags) => writeln!(
            out,
            "  wheel flags: 0x{flags:02X} (bit 1 high resolution, bit 2 natural scrolling)"
        )?,
        Err(HidppError::Unsupported(_)) => {}
        Err(error) => return Err(error.into()),
    }
    match link.thumb_flags() {
        Ok([first, second]) => writeln!(out, "  thumb wheel flags: 0x{first:02X} 0x{second:02X}")?,
        Err(HidppError::Unsupported(_)) => {}
        Err(error) => return Err(error.into()),
    }
    if link.has_haptics() {
        let haptic = link.haptic_state()?;
        writeln!(
            out,
            "  haptics: {}, level {}",
            if haptic.enabled { "on" } else { "off" },
            haptic.level
        )?;
        let played: Vec<String> = HapticWaveform::ALL
            .iter()
            .filter(|waveform| link.supports_waveform(**waveform))
            .map(|waveform| format!("{waveform:?}"))
            .collect();
        writeln!(out, "  waveforms: {}", played.join(", "))?;
    }
    let mut diverted = Vec::new();
    for control in link.model().capabilities().controls {
        if cid(link.model(), control.id).is_some() && link.is_diverted(control.id)? {
            diverted.push(control.id.to_string());
        }
    }
    writeln!(
        out,
        "  diverted buttons: {}",
        if diverted.is_empty() {
            "none".to_owned()
        } else {
            diverted.join(", ")
        }
    )?;
    Ok(())
}

fn haptic<C: Channel>(
    link: &mut MouseLink<C>,
    waveform: HapticWaveform,
    level: Option<HapticLevel>,
    out: &mut impl Write,
) -> Result<(), DiagnosticError> {
    if let Some(level) = level {
        link.set_haptic(haptic_strength(level))?;
        writeln!(out, "Haptic strength set to {level:?}.")?;
    }
    link.play_haptic(waveform)?;
    writeln!(out, "Played {waveform:?}: did you feel it?")?;
    Ok(())
}

/// Diverts the buttons that can be, prints each press and release until the
/// time is up, and gives them back whatever happens.
fn watch_buttons<C: Channel>(
    link: &mut MouseLink<C>,
    duration: Duration,
    out: &mut impl Write,
    stop: &AtomicBool,
) -> Result<(), DiagnosticError> {
    link.release_buttons()?;
    let watched: Vec<MouseControlId> = link
        .model()
        .capabilities()
        .controls
        .iter()
        .filter(|control| !control.locked && cid(link.model(), control.id).is_some())
        .map(|control| control.id)
        .collect();
    // A panic must not leave the buttons dead either.
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        divert_and_print(link, &watched, duration, out, stop)
    }));
    let released = link.release_buttons();
    let result = outcome.unwrap_or_else(|panic| resume_unwind(panic));
    result?;
    released?;
    writeln!(out, "Buttons given back.")?;
    Ok(())
}

fn divert_and_print<C: Channel>(
    link: &mut MouseLink<C>,
    watched: &[MouseControlId],
    duration: Duration,
    out: &mut impl Write,
    stop: &AtomicBool,
) -> Result<(), DiagnosticError> {
    for control in watched {
        link.set_diverted(*control, true)?;
    }
    let names: Vec<String> = watched.iter().map(ToString::to_string).collect();
    writeln!(
        out,
        "Diverted: {}. Press them for {} s (left and right click are left alone).",
        names.join(", "),
        duration.as_secs()
    )?;
    let started = Instant::now();
    let mut tracker = ButtonTracker::default();
    let mut dropped = link.hidpp_mut().dropped_frames();
    while started.elapsed() < duration && !stop.load(Ordering::Relaxed) {
        let Some(bytes) = link.hidpp_mut().next_frame(WATCH_STEP)? else {
            continue;
        };
        if link.hidpp_mut().dropped_frames() != dropped {
            dropped = link.hidpp_mut().dropped_frames();
            tracker.release_all();
            writeln!(out, "(notifications were lost: buttons tracked again)")?;
        }
        let Some(held) = Notification::parse(&bytes).and_then(|n| link.held_buttons(&n)) else {
            continue;
        };
        for event in tracker.update(&held) {
            let sign = if event.pressed {
                "pressed "
            } else {
                "released"
            };
            writeln!(
                out,
                "  {:>6} ms  {sign}  {}",
                started.elapsed().as_millis(),
                event.control
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use hubergamof_protocol::MouseModel;

    use super::*;
    use crate::channel::Hidpp;
    use crate::features::REPROG_CONTROLS_V4;
    use crate::testing::{FakeDevice, FakeLink, frame};

    const DEVICE: u8 = 0xFF;
    /// A stop request that never comes.
    static NEVER: AtomicBool = AtomicBool::new(false);

    fn link(device: FakeDevice) -> MouseLink<FakeLink> {
        MouseLink::connect(Hidpp::new(FakeLink::direct(device)), DEVICE).unwrap()
    }

    fn output(link: &mut MouseLink<FakeLink>, action: &Action) -> String {
        let mut out = Vec::new();
        run(link, action, &mut out, &NEVER).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn state(link: &MouseLink<FakeLink>) -> &crate::testing::FakeState {
        &link.hidpp_channel().device_at(DEVICE).unwrap().state
    }

    /// An output that refuses everything.
    struct Closed;

    impl Write for Closed {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn waveform_names_are_forgiving() {
        for typed in [
            "knock",
            "Knock",
            "KNOCK",
            "sharp-state-change",
            "sharp_state_change",
            "SharpStateChange",
            "sharp state change",
        ] {
            assert_eq!(
                parse_waveform(typed).map(|w| format!("{w:?}").to_lowercase()),
                Some(typed.replace(['-', '_', ' '], "").to_lowercase()),
                "{typed}"
            );
        }
        assert_eq!(
            parse_waveform("whisper-collision"),
            Some(HapticWaveform::WhisperCollision)
        );
        for typed in ["", "fire", "knock knock", "sharp"] {
            assert_eq!(parse_waveform(typed), None, "{typed:?}");
        }
        for waveform in HapticWaveform::ALL {
            assert_eq!(parse_waveform(&format!("{waveform:?}")), Some(waveform));
        }
    }

    #[test]
    fn info_lists_everything_the_mouse_reports() {
        let mut link = link(FakeDevice::mx_master_4());
        let text = output(&mut link, &Action::Info);
        for expected in [
            "MX Master 4 at device index 0xFF",
            "battery: Some(82)",
            "sensitivity: 1600 dpi",
            "wheel: mode 2",
            "wheel flags: 0x03",
            "thumb wheel flags: 0x00 0x00",
            "haptics: on, level 75",
            "waveforms: SharpStateChange",
            "WhisperCollision",
            "diverted buttons: none",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
    }

    #[test]
    fn info_of_a_mouse_without_haptics_says_nothing_about_them() {
        let mut link = link(FakeDevice::mx_anywhere_3s());
        let text = output(&mut link, &Action::Info);
        assert!(text.contains("MX Anywhere 3S"), "{text}");
        assert!(!text.contains("haptics"), "{text}");
    }

    #[test]
    fn info_names_the_diverted_buttons() {
        let mut device = FakeDevice::mx_master_4();
        device.state.diverted.extend([0x0053, 0x01A0]);
        let text = output(&mut link(device), &Action::Info);
        assert!(
            text.contains("diverted buttons: back button, haptic panel"),
            "{text}"
        );
    }

    #[test]
    fn a_waveform_is_played_after_setting_the_strength() {
        let mut link = link(FakeDevice::mx_master_4());
        let action = Action::Haptic {
            waveform: HapticWaveform::Knock,
            level: Some(HapticLevel::Low),
        };
        let text = output(&mut link, &action);
        assert!(
            text.contains("strength set to Low") && text.contains("Played Knock"),
            "{text}"
        );
        assert_eq!(state(&link).haptic, [1, 50]);
        assert_eq!(state(&link).played, [0x0C]);
        let quiet = Action::Haptic {
            waveform: HapticWaveform::Wave,
            level: None,
        };
        output(&mut link, &quiet);
        assert_eq!(state(&link).haptic, [1, 50], "the strength is left alone");
        let off = Action::Haptic {
            waveform: HapticWaveform::Wave,
            level: Some(HapticLevel::Off),
        };
        output(&mut link, &off);
        assert_eq!(state(&link).haptic[0], 0, "off switches the feedback off");
    }

    #[test]
    fn the_sensitivity_and_the_wheel_are_set_and_read_back() {
        let mut link = link(FakeDevice::mx_master_4());
        let text = output(&mut link, &Action::Dpi(3200));
        assert!(
            text.contains("set to 3200 dpi, the mouse reads 3200 dpi"),
            "{text}"
        );
        let text = output(
            &mut link,
            &Action::Wheel {
                mode: WheelMode::Free,
                threshold: 20,
            },
        );
        assert!(
            text.contains("Wheel set to Free: the mouse reads mode 1"),
            "{text}"
        );
    }

    #[test]
    fn a_refused_change_is_an_error_and_prints_no_success() {
        let mut link = link(FakeDevice::mx_master_4());
        let mut out = Vec::new();
        let error = run(&mut link, &Action::Dpi(225), &mut out, &NEVER).unwrap_err();
        assert!(matches!(
            error,
            DiagnosticError::Mouse(HidppError::InvalidArgument(_))
        ));
        assert_eq!(out.len(), 0);
        let mut anywhere = self::link(FakeDevice::mx_anywhere_3s());
        let play = Action::Haptic {
            waveform: HapticWaveform::Knock,
            level: None,
        };
        assert!(run(&mut anywhere, &play, &mut out, &NEVER).is_err());
    }

    #[test]
    fn release_gives_back_what_was_diverted() {
        let mut device = FakeDevice::mx_master_4();
        device.state.diverted.extend([0x0052, 0x00C3]);
        let mut link = link(device);
        let text = output(&mut link, &Action::Release);
        assert!(
            text.contains("Every button of the MX Master 4 is given back"),
            "{text}"
        );
        assert!(state(&link).diverted.is_empty());
    }

    #[test]
    fn watching_prints_presses_and_gives_every_button_back() {
        // A press of back, a press of forward on top, then both released.
        let device = FakeDevice::mx_master_4();
        let reprog = device.index_of(REPROG_CONTROLS_V4).unwrap();
        let notification = |cids: &[u8]| frame(DEVICE, reprog, 0x00, cids);
        let incoming = vec![
            notification(&[0, 0x53]),
            notification(&[0, 0x53, 0, 0x56]),
            notification(&[]),
        ];
        let fake = FakeLink::direct(device).with_incoming(incoming);
        let mut link = MouseLink::connect(Hidpp::new(fake), DEVICE).unwrap();
        let mut out = Vec::new();
        run(
            &mut link,
            &Action::Buttons(Duration::from_millis(300)),
            &mut out,
            &NEVER,
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Diverted: wheel click, wheel mode button, back button, forward button, gesture button, haptic panel"), "{text}");
        assert!(!text.contains("left click"), "{text}");
        let events: Vec<&str> = text
            .lines()
            .filter(|l| l.contains("ed  "))
            .map(str::trim)
            .collect();
        assert_eq!(events.len(), 4, "{text}");
        assert!(events[0].ends_with("pressed   back button"), "{events:?}");
        assert!(
            events[1].ends_with("pressed   forward button"),
            "{events:?}"
        );
        assert!(
            events[2].ends_with("released  back button")
                || events[2].ends_with("released  forward button"),
            "{events:?}"
        );
        assert!(text.ends_with("Buttons given back.\n"), "{text}");
        assert!(state(&link).diverted.is_empty(), "everything is given back");
    }

    #[test]
    fn a_failure_while_watching_still_gives_the_buttons_back() {
        let mut link = link(FakeDevice::mx_master_4());
        // The failure comes from an output that is closed.
        let error = run(
            &mut link,
            &Action::Buttons(Duration::from_millis(50)),
            &mut Closed,
            &NEVER,
        )
        .unwrap_err();
        assert!(matches!(error, DiagnosticError::Output(_)));
        assert!(
            state(&link).diverted.is_empty(),
            "released although the output failed"
        );
        assert_eq!(link.model(), MouseModel::MxMaster4);
    }

    #[test]
    fn a_stop_request_ends_the_watch_and_gives_the_buttons_back() {
        let mut link = link(FakeDevice::mx_master_4());
        let stop = AtomicBool::new(true);
        let mut out = Vec::new();
        let started = Instant::now();
        run(
            &mut link,
            &Action::Buttons(Duration::from_secs(3600)),
            &mut out,
            &stop,
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            String::from_utf8(out)
                .unwrap()
                .ends_with("Buttons given back.\n")
        );
        assert!(state(&link).diverted.is_empty());
    }

    #[test]
    fn a_read_failure_while_watching_still_gives_the_buttons_back() {
        let fake = FakeLink::direct(FakeDevice::mx_master_4()).with_broken_reads();
        let mut link = MouseLink::connect(Hidpp::new(fake), DEVICE).unwrap();
        let mut out = Vec::new();
        let error = run(
            &mut link,
            &Action::Buttons(Duration::from_secs(60)),
            &mut out,
            &NEVER,
        )
        .unwrap_err();
        assert!(matches!(error, DiagnosticError::Mouse(_)), "{error}");
        assert!(state(&link).diverted.is_empty());
    }

    #[test]
    fn the_sensitivity_bounds_and_steps_are_enforced() {
        let mut link = link(FakeDevice::mx_master_4());
        for dpi in [0, 199, 225, 8050, u16::MAX] {
            let mut out = Vec::new();
            assert!(
                run(&mut link, &Action::Dpi(dpi), &mut out, &NEVER).is_err(),
                "{dpi}"
            );
        }
        for dpi in [200, 8000] {
            output(&mut link, &Action::Dpi(dpi));
            assert_eq!(state(&link).dpi, dpi);
        }
    }

    #[test]
    fn the_wheel_threshold_is_bounded_and_every_mode_is_set() {
        let mut link = link(FakeDevice::mx_master_4());
        for threshold in [0, 51, u8::MAX] {
            let action = Action::Wheel {
                mode: WheelMode::Auto,
                threshold,
            };
            assert!(
                run(&mut link, &action, &mut Vec::new(), &NEVER).is_err(),
                "{threshold}"
            );
        }
        for mode in [WheelMode::Ratchet, WheelMode::Free, WheelMode::Auto] {
            let text = output(
                &mut link,
                &Action::Wheel {
                    mode,
                    threshold: 30,
                },
            );
            assert!(text.contains(&format!("Wheel set to {mode:?}")), "{text}");
        }
    }

    #[test]
    fn watching_for_no_time_diverts_and_gives_back() {
        let mut link = link(FakeDevice::mx_master_4());
        let text = output(&mut link, &Action::Buttons(Duration::ZERO));
        assert!(text.ends_with("Buttons given back.\n"), "{text}");
        assert!(state(&link).diverted.is_empty());
    }
}
