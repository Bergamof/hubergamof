//! How the radial menu answers what the person does: the button that opens it,
//! the pointer, a click, Escape.
//!
//! Two ways to use it, told apart by how long the opening button is held:
//! pressed and released within the hold time, the menu stays open and a click
//! chooses (a click in the void closes it); held longer, releasing the button
//! on a petal chooses it, and releasing in the void closes the menu.
//!
//! No clock is read here: each input carries the time it happened at.

use std::time::Duration;

/// What reaches the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// The button that opens the menu goes down (or down again, once the menu
    /// stays open: that counts as a click).
    Press {
        /// When, as a time since any fixed origin.
        at: Duration,
    },
    /// The button that opens the menu comes up.
    Release {
        /// When, on the same clock as the presses.
        at: Duration,
    },
    /// The pointer is on this petal, or on none (`None`: in the void).
    Over(Option<usize>),
    /// Another button is clicked while the menu is open.
    Click,
    /// The person gives up.
    Escape,
}

/// What the menu asks the overlay and the host to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// Show the menu.
    Opened,
    /// Highlight this petal, or none.
    Hovered(Option<usize>),
    /// This petal is chosen: carry out its entry and hide the menu.
    Chosen(usize),
    /// Hide the menu, nothing chosen.
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Closed,
    /// Open, the button still down since `since`.
    Pressed {
        since: Duration,
    },
    /// Open, the button came up early: clicks choose.
    Staying,
}

/// The state of the radial menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    hold: Duration,
    phase: Phase,
    hovered: Option<usize>,
}

impl Menu {
    /// A closed menu whose button must be held `hold` for a release to choose.
    #[must_use]
    pub const fn new(hold: Duration) -> Self {
        Self {
            hold,
            phase: Phase::Closed,
            hovered: None,
        }
    }

    /// The menu is shown.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.phase != Phase::Closed
    }

    /// The petal under the pointer, if any.
    #[must_use]
    pub const fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// Changes how long the button must be held, from the next opening on.
    pub const fn set_hold(&mut self, hold: Duration) {
        self.hold = hold;
    }

    /// Takes one input; what the overlay and the host must do, if anything.
    pub fn handle(&mut self, input: Input) -> Option<Output> {
        match (self.phase, input) {
            (Phase::Closed, Input::Press { at }) => {
                self.phase = Phase::Pressed { since: at };
                self.hovered = None;
                Some(Output::Opened)
            }
            (Phase::Closed, _) => None,
            (Phase::Pressed { since }, Input::Release { at }) => {
                if at.saturating_sub(since) >= self.hold {
                    Some(self.finish())
                } else {
                    self.phase = Phase::Staying;
                    None
                }
            }
            (Phase::Staying, Input::Press { .. }) | (_, Input::Click) => Some(self.finish()),
            (_, Input::Over(petal)) => self.hover(petal),
            (_, Input::Escape) => Some(self.close()),
            // A second press while the button is down, a release of a button
            // that stays open: nothing to do.
            (
                Phase::Pressed { .. } | Phase::Staying,
                Input::Press { .. } | Input::Release { .. },
            ) => None,
        }
    }

    fn hover(&mut self, petal: Option<usize>) -> Option<Output> {
        (self.hovered != petal).then(|| {
            self.hovered = petal;
            Output::Hovered(petal)
        })
    }

    /// Chooses the petal under the pointer, or closes in the void.
    fn finish(&mut self) -> Output {
        let chosen = self.hovered;
        self.close();
        chosen.map_or(Output::Closed, Output::Chosen)
    }

    fn close(&mut self) -> Output {
        self.phase = Phase::Closed;
        self.hovered = None;
        Output::Closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOLD: Duration = Duration::from_millis(280);

    fn at(milliseconds: u64) -> Duration {
        Duration::from_millis(milliseconds)
    }

    fn press(ms: u64) -> Input {
        Input::Press { at: at(ms) }
    }

    fn release(ms: u64) -> Input {
        Input::Release { at: at(ms) }
    }

    fn open_menu() -> Menu {
        let mut menu = Menu::new(HOLD);
        assert_eq!(menu.handle(press(0)), Some(Output::Opened));
        menu
    }

    #[test]
    fn pressing_opens_the_menu() {
        let menu = open_menu();
        assert!(menu.is_open());
        assert_eq!(menu.hovered(), None);
    }

    #[test]
    fn a_closed_menu_ignores_everything_but_the_press() {
        let mut menu = Menu::new(HOLD);
        for input in [
            release(10),
            Input::Over(Some(1)),
            Input::Click,
            Input::Escape,
        ] {
            assert_eq!(menu.handle(input), None);
        }
        assert!(!menu.is_open());
    }

    #[test]
    fn the_pointer_is_reported_only_when_it_changes_petal() {
        let mut menu = open_menu();
        assert_eq!(
            menu.handle(Input::Over(Some(2))),
            Some(Output::Hovered(Some(2)))
        );
        assert_eq!(menu.handle(Input::Over(Some(2))), None);
        assert_eq!(menu.handle(Input::Over(None)), Some(Output::Hovered(None)));
        assert_eq!(menu.handle(Input::Over(None)), None);
    }

    #[test]
    fn held_long_enough_releasing_on_a_petal_chooses_it() {
        let mut menu = open_menu();
        menu.handle(Input::Over(Some(1)));
        assert_eq!(menu.handle(release(280)), Some(Output::Chosen(1)));
        assert!(!menu.is_open());
    }

    #[test]
    fn held_long_enough_releasing_in_the_void_closes_the_menu() {
        let mut menu = open_menu();
        menu.handle(Input::Over(Some(1)));
        menu.handle(Input::Over(None));
        assert_eq!(menu.handle(release(900)), Some(Output::Closed));
        assert!(!menu.is_open());
    }

    #[test]
    fn released_early_the_menu_stays_open_even_on_a_petal() {
        let mut menu = open_menu();
        menu.handle(Input::Over(Some(1)));
        assert_eq!(menu.handle(release(279)), None);
        assert!(menu.is_open());
    }

    #[test]
    fn a_menu_that_stays_open_chooses_on_a_click_on_a_petal() {
        let mut menu = open_menu();
        menu.handle(release(100));
        menu.handle(Input::Over(Some(3)));
        assert_eq!(menu.handle(Input::Click), Some(Output::Chosen(3)));
        assert!(!menu.is_open());
    }

    #[test]
    fn a_menu_that_stays_open_closes_on_a_click_in_the_void() {
        let mut menu = open_menu();
        menu.handle(release(100));
        menu.handle(Input::Over(Some(3)));
        menu.handle(Input::Over(None));
        assert_eq!(menu.handle(Input::Click), Some(Output::Closed));
    }

    #[test]
    fn pressing_the_opening_button_again_counts_as_a_click() {
        let mut menu = open_menu();
        menu.handle(release(100));
        menu.handle(Input::Over(Some(0)));
        assert_eq!(menu.handle(press(500)), Some(Output::Chosen(0)));
        assert!(!menu.is_open());
    }

    #[test]
    fn escape_closes_the_menu_whatever_is_under_the_pointer() {
        for released_early in [false, true] {
            let mut menu = open_menu();
            if released_early {
                menu.handle(release(100));
            }
            menu.handle(Input::Over(Some(2)));
            assert_eq!(menu.handle(Input::Escape), Some(Output::Closed));
            assert!(!menu.is_open());
            assert_eq!(menu.hovered(), None);
        }
    }

    #[test]
    fn a_click_while_the_button_is_still_down_chooses_too() {
        let mut menu = open_menu();
        menu.handle(Input::Over(Some(1)));
        assert_eq!(menu.handle(Input::Click), Some(Output::Chosen(1)));
    }

    #[test]
    fn the_menu_opens_again_after_it_closed_with_nothing_kept() {
        let mut menu = open_menu();
        menu.handle(Input::Over(Some(1)));
        menu.handle(release(400));
        assert_eq!(menu.handle(press(1000)), Some(Output::Opened));
        assert_eq!(menu.hovered(), None);
    }

    #[test]
    fn the_hold_time_applies_from_the_next_opening() {
        let mut menu = Menu::new(HOLD);
        menu.set_hold(at(500));
        menu.handle(press(0));
        menu.handle(Input::Over(Some(0)));
        assert_eq!(
            menu.handle(release(400)),
            None,
            "shorter than 500: it stays"
        );
    }

    #[test]
    fn a_release_dated_before_the_press_is_a_short_press_not_a_panic() {
        let mut menu = Menu::new(HOLD);
        menu.handle(press(1000));
        assert_eq!(menu.handle(release(500)), None);
        assert!(menu.is_open());
    }
}
