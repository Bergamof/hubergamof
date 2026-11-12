//! What a module contributes to the touch strip, which the host draws as a
//! whole: the reminder of what a dial does, the widgets (volume, pomodoro)
//! and the popup menu opened from a dial.

use crate::DialFunctionId;

/// Most entries of a [`StripPopup`].
pub const MAX_POPUP_ENTRIES: usize = 3;

/// Reminder of what a dial does with a function of the module, shown under
/// the strip: one short text for the rotation and one for the press, either
/// of them missing when the dial does nothing that way. It describes the
/// current state ("Muet" then "Réactiver"), so it is read on every redraw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DialHint {
    /// What turning the dial does.
    pub rotate: Option<String>,
    /// What pressing the dial does.
    pub press: Option<String>,
}

impl DialHint {
    /// A hint with neither a rotation nor a press.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// The same hint with `rotate` for the rotation.
    #[must_use]
    pub fn rotating(self, rotate: impl Into<String>) -> Self {
        Self {
            rotate: Some(rotate.into()),
            ..self
        }
    }

    /// The same hint with `press` for the press.
    #[must_use]
    pub fn pressing(self, press: impl Into<String>) -> Self {
        Self {
            press: Some(press.into()),
            ..self
        }
    }
}

/// What the contextual dial does on a page of a module, and the reminder
/// shown under the strip for it. The page declares it with its content
/// ([`crate::PageContent::context_dial`]); the host calls a handler only for
/// the gestures whose text is set, so a module that reacts to a gesture
/// always says what it does, and a gesture left out does nothing.
///
/// The long press has no room in the reminder: its text is declared for the
/// sake of the rule above and kept for the interface. Built with
/// [`ContextDial::none`] and its builders: fields may be added without
/// breaking modules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContextDial {
    /// What turning the dial does, and what pressing it does.
    pub hint: DialHint,
    /// What holding the dial pressed does.
    pub long_press: Option<String>,
}

impl ContextDial {
    /// A dial that does nothing.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// The same dial, whose rotation does `rotate`.
    #[must_use]
    pub fn rotating(self, rotate: impl Into<String>) -> Self {
        Self {
            hint: self.hint.rotating(rotate),
            ..self
        }
    }

    /// The same dial, whose press does `press`.
    #[must_use]
    pub fn pressing(self, press: impl Into<String>) -> Self {
        Self {
            hint: self.hint.pressing(press),
            ..self
        }
    }

    /// The same dial, whose long press does `long_press`.
    #[must_use]
    pub fn long_pressing(self, long_press: impl Into<String>) -> Self {
        Self {
            long_press: Some(long_press.into()),
            ..self
        }
    }

    /// Whether turning the dial does something.
    #[must_use]
    pub fn reacts_to_rotation(&self) -> bool {
        self.hint.rotate.is_some()
    }

    /// Whether pressing the dial does something.
    #[must_use]
    pub fn reacts_to_press(&self) -> bool {
        self.hint.press.is_some()
    }

    /// Whether holding the dial pressed does something.
    #[must_use]
    pub fn reacts_to_long_press(&self) -> bool {
        self.long_press.is_some()
    }
}

/// What a module shows in its own place of the touch strip. The host
/// decides where and how it is drawn, and keeps one place for each kind of
/// widget it knows: when several instances show the same kind, the first one
/// (in the order of the configuration) is shown. A new kind of widget is a
/// new variant, drawn by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StripWidget {
    /// The volume of the audio output.
    Volume(VolumeWidget),
    /// The step of a pomodoro and the time left.
    Pomodoro(PomodoroWidget),
}

/// Volume of the audio output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeWidget {
    /// Volume in percent; may go beyond 100 when the output is amplified.
    pub percent: u16,
    /// Whether the output is muted.
    pub muted: bool,
}

impl VolumeWidget {
    /// A volume of `percent`.
    #[must_use]
    pub fn new(percent: u16, muted: bool) -> Self {
        Self { percent, muted }
    }
}

/// State of the timer of a [`PomodoroWidget`], which sets its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PomodoroState {
    /// A work step is running.
    Working,
    /// A break is running.
    Resting,
    /// The time is stopped in the middle of a step.
    Paused,
    /// A step is prepared and not started.
    Ready,
}

/// The step of a pomodoro, its time left and its progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PomodoroWidget {
    /// State of the timer.
    pub state: PomodoroState,
    /// Name of the step, short (`Focus 2/4`, `Pause`).
    pub label: String,
    /// Time left, as shown (`18:24`).
    pub time: String,
    progress: u16,
}

impl PomodoroWidget {
    /// Progress of a finished step, in thousandths.
    pub const FULL_PROGRESS: u16 = 1000;

    /// A widget whose step is `progress` thousandths done (clamped to
    /// [`Self::FULL_PROGRESS`]).
    #[must_use]
    pub fn new(
        state: PomodoroState,
        label: impl Into<String>,
        time: impl Into<String>,
        progress: u16,
    ) -> Self {
        Self {
            state,
            label: label.into(),
            time: time.into(),
            progress: progress.min(Self::FULL_PROGRESS),
        }
    }

    /// Share of the step elapsed, in thousandths of the step.
    #[must_use]
    pub fn progress(&self) -> u16 {
        self.progress
    }
}

/// Icon of an entry of a [`StripPopup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PopupIcon {
    /// Start or resume.
    Play,
    /// Pause.
    Pause,
    /// Go to the next step.
    Skip,
    /// Start over.
    Reset,
}

/// An entry of a [`StripPopup`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopupEntry {
    /// Icon drawn above the label.
    pub icon: PopupIcon,
    /// Short label.
    pub label: String,
}

impl PopupEntry {
    /// An entry showing `icon` over `label`.
    #[must_use]
    pub fn new(icon: PopupIcon, label: impl Into<String>) -> Self {
        Self {
            icon,
            label: label.into(),
        }
    }
}

/// A menu the host shows as a card over the touch strip, above the dial
/// that opened it, while the module returns it from
/// [`crate::Module::strip_popup`]. The module handles the dial: it moves the
/// selection on rotation, runs the entry on press and closes the popup
/// (after a delay without input, for instance).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripPopup {
    /// Dial function that opened it: the card points at its dial.
    pub function: DialFunctionId,
    entries: Vec<PopupEntry>,
    selected: usize,
}

impl StripPopup {
    /// A popup of the entries (at most [`MAX_POPUP_ENTRIES`] are kept) with
    /// entry `selected` chosen (the last one if beyond them).
    #[must_use]
    pub fn new(function: DialFunctionId, mut entries: Vec<PopupEntry>, selected: usize) -> Self {
        entries.truncate(MAX_POPUP_ENTRIES);
        let selected = selected.min(entries.len().saturating_sub(1));
        Self {
            function,
            entries,
            selected,
        }
    }

    /// The entries, in display order.
    #[must_use]
    pub fn entries(&self) -> &[PopupEntry] {
        &self.entries
    }

    /// Index of the selected entry.
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_start_empty_and_take_each_part() {
        assert_eq!(DialHint::none(), DialHint::default());
        let hint = DialHint::none().rotating("Volume").pressing("Muet");
        assert_eq!(hint.rotate.as_deref(), Some("Volume"));
        assert_eq!(hint.press.as_deref(), Some("Muet"));
    }

    #[test]
    fn the_contextual_dial_reacts_only_to_what_it_declares() {
        let nothing = ContextDial::none();
        assert!(!nothing.reacts_to_rotation());
        assert!(!nothing.reacts_to_press());
        assert!(!nothing.reacts_to_long_press());
        let dial = ContextDial::none()
            .rotating("Choisir")
            .long_pressing("Tout");
        assert!(dial.reacts_to_rotation());
        assert!(!dial.reacts_to_press());
        assert!(dial.reacts_to_long_press());
        assert_eq!(dial.hint.rotate.as_deref(), Some("Choisir"));
        assert_eq!(dial.long_press.as_deref(), Some("Tout"));
        assert!(dial.pressing("Cocher").reacts_to_press());
    }

    #[test]
    fn pomodoro_progress_is_clamped() {
        let widget = PomodoroWidget::new(PomodoroState::Working, "Focus 1/4", "25:00", 5000);
        assert_eq!(widget.progress(), PomodoroWidget::FULL_PROGRESS);
    }

    #[test]
    fn popups_keep_a_few_entries_and_a_valid_selection() {
        let entries = ["a", "b", "c", "d"]
            .map(|label| PopupEntry::new(PopupIcon::Play, label))
            .to_vec();
        let popup = StripPopup::new("control".into(), entries, 9);
        assert_eq!(popup.entries().len(), MAX_POPUP_ENTRIES);
        assert_eq!(popup.selected(), MAX_POPUP_ENTRIES - 1);
        let empty = StripPopup::new("control".into(), Vec::new(), 3);
        assert_eq!(empty.selected(), 0);
    }
}
