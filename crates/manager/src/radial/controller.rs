//! Ties the radial menu together: the entries that are shown, the petals they
//! make, the button that opens the menu and what the window reports. It says
//! what the window and the host must do ([`Effect`]); it draws nothing and
//! reads no device, so the window is the only part that needs a screen.
//!
//! The menu opens where the pointer is, which a Wayland client only learns
//! when the pointer enters the window: the window is shown first, and the menu
//! is placed at the first position reported.

use std::time::Duration;

use hubergamof_protocol::{RadialEntryConfig, RadialMenuConfig};

use super::geometry::{Layout, Point, Ring, Size, fit_on_screen};
use super::interaction::{Input, Menu, Output};
use super::looks::{Looks, PetalNotes};
use super::paint::Palette;
use super::window::Content;
use super::window::PetalNote;

/// What the window must show of the menu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// Middle of the menu, in the coordinates of the window.
    pub centre: Point,
    /// The petal under the pointer, if any.
    pub lit: Option<usize>,
}

/// What the window and the host must do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Show the window, the menu is not placed yet.
    Open,
    /// The menu has its place: draw it from now on.
    Place(View),
    /// The petal under the pointer changed: draw again.
    Redraw(View),
    /// An entry is chosen: carry out its primary action. The window is hidden
    /// by the [`Effect::Close`] that follows.
    Choose(RadialEntryConfig),
    /// Hide the window.
    Close,
}

/// What the window reports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowEvent {
    /// The window has this size: the size of the screen the menu must fit.
    Screen(Size),
    /// The pointer is here, in the coordinates of the window.
    Pointer(Point),
    /// A button is clicked (the one that opens the menu aside).
    Click,
    /// Escape is pressed.
    Escape,
    /// The window is gone (its thread ended): the menu is given up, as with
    /// Escape.
    Gone,
}

/// The radial menu of one run: its configuration and its state.
#[derive(Debug)]
pub struct Controller {
    ring: Ring,
    screen: Size,
    /// The latest configuration, applied while the menu is closed.
    wanted: RadialMenuConfig,
    menu: Menu,
    entries: Vec<RadialEntryConfig>,
    layout: Option<Layout>,
    centre: Option<Point>,
    looks: Looks,
    notes: PetalNotes,
}

impl Controller {
    /// A closed menu of `config`, whose petals are cut from `ring`, on a
    /// screen of `screen`. `config` must have been validated
    /// ([`RadialMenuConfig::validate`]): the names and the number of petals
    /// are taken as they are.
    #[must_use]
    pub fn new(config: &RadialMenuConfig, ring: Ring, screen: Size) -> Self {
        let mut controller = Self {
            ring,
            screen,
            wanted: config.clone(),
            menu: Menu::new(Duration::from_millis(u64::from(config.hold_ms))),
            entries: Vec::new(),
            layout: None,
            centre: None,
            looks: Looks::default(),
            notes: PetalNotes::default(),
        };
        controller.apply_wanted();
        controller
    }

    /// Takes a new configuration, from the next opening on (an open menu keeps
    /// the one it was opened with).
    pub fn configure(&mut self, config: &RadialMenuConfig) {
        self.wanted = config.clone();
        if !self.menu.is_open() {
            self.apply_wanted();
        }
    }

    fn apply_wanted(&mut self) {
        self.menu
            .set_hold(Duration::from_millis(u64::from(self.wanted.hold_ms)));
        self.entries = self.wanted.shown().cloned().collect();
        self.layout = Layout::new(self.entries.len(), self.ring);
    }

    /// The entries shown, in the order of the petals.
    #[must_use]
    pub fn entries(&self) -> &[RadialEntryConfig] {
        &self.entries
    }

    /// The petals of the menu; `None` when no entry is shown.
    #[must_use]
    pub const fn layout(&self) -> Option<&Layout> {
        self.layout.as_ref()
    }

    /// What the modules look like: their names and icons, as the petals show them.
    pub fn set_looks(&mut self, looks: Looks) {
        self.looks = looks;
    }

    /// What the modules say under their names while the menu is open.
    pub fn set_notes(&mut self, notes: PetalNotes) {
        self.notes = notes;
    }

    /// What the modules of the shown entries say now, in the order of the
    /// petals.
    #[must_use]
    pub fn notes(&self) -> Vec<PetalNote> {
        self.entries
            .iter()
            .map(|entry| self.notes.of(&entry.module))
            .collect()
    }

    /// What the window draws: a petal per shown entry, with the name and the
    /// icon of its module, and what it says now.
    #[must_use]
    pub fn content(&self, palette: Palette, animate: bool) -> Option<Content> {
        let layout = self.layout?;
        let petals = self
            .entries
            .iter()
            .zip(self.notes())
            .map(|(entry, status)| self.looks.petal(&entry.module, status))
            .collect();
        Some(Content {
            petals,
            layout,
            palette,
            animate,
        })
    }

    /// The menu is shown.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.menu.is_open()
    }

    /// What the window shows now; `None` before the menu is placed.
    #[must_use]
    pub fn view(&self) -> Option<View> {
        self.centre.map(|centre| View {
            centre,
            lit: self.menu.hovered(),
        })
    }

    /// The button that opens the menu goes down, `at` being the time on the
    /// clock of the releases.
    pub fn press(&mut self, at: Duration) -> Vec<Effect> {
        if self.layout.is_none() {
            return Vec::new();
        }
        self.feed(Input::Press { at })
    }

    /// The button that opens the menu comes up.
    pub fn release(&mut self, at: Duration) -> Vec<Effect> {
        self.feed(Input::Release { at })
    }

    /// Takes what the window reports.
    pub fn window(&mut self, event: WindowEvent) -> Vec<Effect> {
        match event {
            WindowEvent::Screen(screen) => {
                self.screen = screen;
                Vec::new()
            }
            WindowEvent::Pointer(position) => self.pointer(position),
            WindowEvent::Click => self.feed(Input::Click),
            WindowEvent::Escape | WindowEvent::Gone => self.feed(Input::Escape),
        }
    }

    fn pointer(&mut self, position: Point) -> Vec<Effect> {
        if !self.menu.is_open() {
            return Vec::new();
        }
        let (Some(layout), Some(centre)) = (self.layout, self.centre) else {
            return self.place(position);
        };
        let offset = Point::new(position.x - centre.x, position.y - centre.y);
        self.feed(Input::Over(layout.petal_at(offset)))
    }

    /// Puts the menu where the pointer is first seen, kept on the screen.
    fn place(&mut self, pointer: Point) -> Vec<Effect> {
        let centre = fit_on_screen(pointer, self.ring.outer, self.screen);
        self.centre = Some(centre);
        let view = View { centre, lit: None };
        // The pointer may already be on a petal when the menu is pushed back
        // from an edge.
        let mut effects = vec![Effect::Place(view)];
        effects.extend(self.pointer(pointer));
        effects
    }

    fn feed(&mut self, input: Input) -> Vec<Effect> {
        let Some(output) = self.menu.handle(input) else {
            return Vec::new();
        };
        match output {
            Output::Opened => {
                self.centre = None;
                vec![Effect::Open]
            }
            Output::Hovered(_) => self.view().map(Effect::Redraw).into_iter().collect(),
            Output::Chosen(petal) => {
                let chosen = self.entries.get(petal).cloned();
                self.finish();
                chosen
                    .map(Effect::Choose)
                    .into_iter()
                    .chain([Effect::Close])
                    .collect()
            }
            Output::Closed => {
                self.finish();
                vec![Effect::Close]
            }
        }
    }

    /// The menu is closed: forgets its place and takes the configuration that
    /// came while it was open.
    fn finish(&mut self) {
        self.centre = None;
        self.apply_wanted();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    const RING: Ring = Ring {
        inner: 44.0,
        outer: 150.0,
        gap: 8.0,
    };
    const SCREEN: Size = Size {
        width: 1920.0,
        height: 1080.0,
    };

    fn at(milliseconds: u64) -> Duration {
        Duration::from_millis(milliseconds)
    }

    fn controller() -> Controller {
        Controller::new(&RadialMenuConfig::default_menu(), RING, SCREEN)
    }

    /// Opens the menu with the pointer at (900, 500), the middle of the menu.
    fn opened() -> Controller {
        let mut controller = controller();
        assert_eq!(controller.press(at(0)), [Effect::Open]);
        controller.window(WindowEvent::Pointer(Point::new(900.0, 500.0)));
        controller
    }

    /// Moves the pointer onto petal `petal` of a menu centred on (900, 500).
    fn over(controller: &mut Controller, petal: usize) -> Vec<Effect> {
        let spots = [
            Point::new(900.0, 410.0),
            Point::new(990.0, 500.0),
            Point::new(900.0, 590.0),
            Point::new(810.0, 500.0),
        ];
        controller.window(WindowEvent::Pointer(spots[petal]))
    }

    fn entry_module(effect: &Effect) -> String {
        match effect {
            Effect::Choose(entry) => entry.module.to_string(),
            other => panic!("not a choice: {other:?}"),
        }
    }

    #[test]
    fn the_size_the_window_reports_is_what_the_menu_must_fit() {
        let mut controller = controller();
        controller.window(WindowEvent::Screen(Size {
            width: 800.0,
            height: 600.0,
        }));
        controller.press(at(0));
        let effects = controller.window(WindowEvent::Pointer(Point::new(790.0, 10.0)));
        assert_eq!(
            effects.first(),
            Some(&Effect::Place(View {
                centre: Point::new(650.0, 150.0),
                lit: None,
            }))
        );
    }

    #[test]
    fn the_shown_entries_are_the_petals_in_order() {
        let controller = controller();
        let modules: Vec<_> = controller
            .entries()
            .iter()
            .map(|entry| entry.module.to_string())
            .collect();
        assert_eq!(modules, ["pomodoro", "teams", "audio", "intellij"]);
        assert_eq!(controller.layout().unwrap().count(), 4);
    }

    #[test]
    fn the_petals_say_what_their_modules_say_now() {
        let mut controller = controller();
        let says = Arc::new(Mutex::new("Focus 1/4".to_owned()));
        let reader = Arc::clone(&says);
        controller.set_notes(PetalNotes::new(move |module| PetalNote {
            status: (module.0 == "pomodoro").then(|| reader.lock().unwrap().clone()),
            badge: None,
        }));
        let petals = |controller: &Controller| {
            controller
                .content(crate::radial::paint::DEFAULT_PALETTE, false)
                .unwrap()
                .petals
        };
        let statuses: Vec<_> = petals(&controller)
            .iter()
            .map(|petal| petal.status.clone())
            .collect();
        assert_eq!(statuses, [Some("Focus 1/4".to_owned()), None, None, None]);
        let said: Vec<_> = controller
            .notes()
            .into_iter()
            .map(|note| note.status)
            .collect();
        assert_eq!(said, statuses);

        "Focus 2/4".clone_into(&mut says.lock().unwrap());
        assert_eq!(controller.notes()[0].status.as_deref(), Some("Focus 2/4"));
    }

    #[test]
    fn the_menu_is_placed_where_the_pointer_first_shows() {
        let mut controller = controller();
        controller.press(at(0));
        assert_eq!(controller.view(), None, "not placed yet");
        let effects = controller.window(WindowEvent::Pointer(Point::new(900.0, 500.0)));
        let view = View {
            centre: Point::new(900.0, 500.0),
            lit: None,
        };
        assert_eq!(effects, [Effect::Place(view)]);
        assert_eq!(controller.view(), Some(view));
    }

    #[test]
    fn a_menu_opened_near_an_edge_is_pushed_back_onto_the_screen() {
        let mut controller = controller();
        controller.press(at(0));
        let effects = controller.window(WindowEvent::Pointer(Point::new(10.0, 1075.0)));
        assert_eq!(
            effects.first(),
            Some(&Effect::Place(View {
                centre: Point::new(150.0, 930.0),
                lit: None,
            }))
        );
    }

    #[test]
    fn a_pointer_that_starts_on_a_petal_after_the_push_back_lights_it() {
        let mut controller = controller();
        controller.press(at(0));
        // Pushed to (150, 930): the pointer, at (10, 1075), is outside the ring.
        let effects = controller.window(WindowEvent::Pointer(Point::new(10.0, 1075.0)));
        assert_eq!(effects.len(), 1, "no petal under it");
        // Pushed to (1770, 930) from (1915, 1075): outside as well; the pointer
        // inside the ring is reported as a change.
        let mut controller = controller_for_edge();
        let effects = controller.window(WindowEvent::Pointer(Point::new(150.0, 840.0)));
        assert!(matches!(effects.as_slice(), [Effect::Redraw(view)] if view.lit == Some(0)));
    }

    fn controller_for_edge() -> Controller {
        let mut controller = controller();
        controller.press(at(0));
        controller.window(WindowEvent::Pointer(Point::new(10.0, 1075.0)));
        controller
    }

    #[test]
    fn the_pointer_over_a_petal_lights_it_once() {
        let mut controller = opened();
        let effects = over(&mut controller, 1);
        assert!(matches!(effects.as_slice(), [Effect::Redraw(view)] if view.lit == Some(1)));
        assert_eq!(over(&mut controller, 1), [], "same petal: nothing to draw");
    }

    #[test]
    fn held_long_enough_releasing_on_a_petal_chooses_its_entry_and_closes() {
        let mut controller = opened();
        over(&mut controller, 2);
        let effects = controller.release(at(400));
        assert_eq!(entry_module(&effects[0]), "audio");
        assert_eq!(effects[1], Effect::Close);
        assert!(!controller.is_open());
        assert_eq!(controller.view(), None);
    }

    #[test]
    fn released_early_the_menu_stays_and_a_click_on_a_petal_chooses() {
        let mut controller = opened();
        over(&mut controller, 3);
        assert_eq!(controller.release(at(100)), []);
        assert!(controller.is_open());
        let effects = controller.window(WindowEvent::Click);
        assert_eq!(entry_module(&effects[0]), "intellij");
        assert_eq!(effects[1], Effect::Close);
    }

    #[test]
    fn a_click_in_the_void_closes_without_choosing() {
        let mut controller = opened();
        controller.release(at(100));
        assert_eq!(controller.window(WindowEvent::Click), [Effect::Close]);
    }

    #[test]
    fn escape_closes_the_menu() {
        let mut controller = opened();
        over(&mut controller, 0);
        assert_eq!(controller.window(WindowEvent::Escape), [Effect::Close]);
        assert!(!controller.is_open());
    }

    #[test]
    fn a_window_that_is_gone_closes_the_menu_without_choosing() {
        let mut controller = opened();
        over(&mut controller, 1);
        controller.release(at(50));
        // Released early, over a petal: a press would now choose it. The window
        // going away must close the menu instead, so that nothing is chosen
        // later without a menu in sight.
        assert_eq!(controller.window(WindowEvent::Gone), [Effect::Close]);
        assert!(!controller.is_open());
        assert_eq!(controller.press(at(100)), [Effect::Open]);
        assert_eq!(controller.release(at(110)), []);
    }

    #[test]
    fn a_closed_menu_ignores_the_window() {
        let mut controller = controller();
        for event in [
            WindowEvent::Screen(SCREEN),
            WindowEvent::Pointer(Point::new(1.0, 1.0)),
            WindowEvent::Click,
            WindowEvent::Escape,
        ] {
            assert_eq!(controller.window(event), []);
        }
        assert_eq!(controller.release(at(10)), []);
    }

    #[test]
    fn a_menu_without_a_shown_entry_does_not_open() {
        let mut config = RadialMenuConfig::default_menu();
        for entry in &mut config.entries {
            entry.enabled = false;
        }
        let mut controller = Controller::new(&config, RING, SCREEN);
        assert_eq!(controller.press(at(0)), []);
        assert!(!controller.is_open());
    }

    #[test]
    fn a_configuration_that_comes_while_open_applies_after_the_menu_closed() {
        let mut controller = opened();
        let mut config = RadialMenuConfig::default_menu();
        config.entries[0].enabled = false;
        controller.configure(&config);
        assert_eq!(controller.entries().len(), 4, "the open menu is unchanged");
        controller.window(WindowEvent::Escape);
        assert_eq!(controller.entries().len(), 3);
        assert_eq!(controller.layout().unwrap().count(), 3);
    }

    #[test]
    fn the_hold_time_of_the_configuration_decides_between_choosing_and_staying() {
        let mut config = RadialMenuConfig::default_menu();
        config.hold_ms = 500;
        let mut controller = Controller::new(&config, RING, SCREEN);
        controller.press(at(0));
        controller.window(WindowEvent::Pointer(Point::new(900.0, 500.0)));
        over(&mut controller, 0);
        assert_eq!(controller.release(at(400)), [], "400 is a short press");
        assert!(controller.is_open());
    }
}
