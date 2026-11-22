//! The radial menu as the manager runs it: one thread owns the menu and its
//! window, and takes what the mice, the window, the configuration and the
//! screen lock tell it, one event at a time.
//!
//! What a chosen entry does is up to the `chooser` the service is given.

use std::sync::Arc;
use std::time::Instant;

use hubergamof_module_api::HapticWaveform;
use hubergamof_protocol::{RadialEntryConfig, RadialMenuConfig};
use tokio::sync::{mpsc, watch};

use super::backend;
use super::controller::{Controller, Effect, WindowEvent};
use super::geometry::{DEFAULT_RING, Size};
use super::looks::{Looks, Statuses};
use super::paint::DEFAULT_PALETTE;
use super::window::Surface;
use crate::haptics::HapticOutput;

/// What the screen is taken to be until the window reports its size.
const UNKNOWN_SCREEN: Size = Size {
    width: 1920.0,
    height: 1080.0,
};

/// Time between two nudges of the pointer while the menu waits to be placed,
/// the first one included: the window needs a moment to be shown.
pub(super) const NUDGE_PERIOD: std::time::Duration = std::time::Duration::from_millis(80);

/// Nudges tried at most for one opening.
pub(super) const NUDGE_TRIES: u32 = 6;

/// Time between two readings of what the modules say under their names while
/// the menu is open.
pub(super) const REFRESH_PERIOD: std::time::Duration = std::time::Duration::from_millis(500);

/// The waveforms of the feel of the menu: it opens, the pointer comes onto a
/// petal, an entry is chosen.
const OPEN_WAVEFORM: HapticWaveform = HapticWaveform::SubtleCollision;
const HOVER_WAVEFORM: HapticWaveform = HapticWaveform::WhisperCollision;
const CHOICE_WAVEFORM: HapticWaveform = HapticWaveform::SharpStateChange;

/// What the menu is asked to do and told about.
#[derive(Debug)]
enum Event {
    /// The button that opens the menu goes down.
    Press(Instant),
    /// The button that opens the menu comes up.
    Release(Instant),
    /// The mouse that held the button is gone: the menu gives up.
    Cancel,
    Window(WindowEvent),
    /// Time to nudge the pointer, if the menu is still waiting for it.
    Nudge,
    /// Time to read again what the modules say, if the menu is open.
    Refresh,
    Config(Box<RadialMenuConfig>),
    Locked(bool),
    /// The service is dropped.
    Stop,
}

/// Where the mice tell the menu about its button.
#[derive(Debug, Clone)]
pub struct RadialHandle {
    events: mpsc::UnboundedSender<Event>,
}

impl RadialHandle {
    /// The button that opens the menu goes down.
    pub fn press(&self) {
        self.send(Event::Press(Instant::now()));
    }

    /// The button that opens the menu comes up.
    pub fn release(&self) {
        self.send(Event::Release(Instant::now()));
    }

    /// The mouse is gone with the button down: the menu closes without
    /// choosing.
    pub fn cancel(&self) {
        self.send(Event::Cancel);
    }

    fn send(&self, event: Event) {
        // The service is gone at shutdown: nothing left to tell.
        let _ = self.events.send(event);
    }

    /// A handle and what it sends, for the tests of those that use it.
    #[cfg(test)]
    pub(crate) fn probe() -> (Self, Probe) {
        let (events, received) = mpsc::unbounded_channel();
        (Self { events }, Probe(received))
    }
}

/// What a [`RadialHandle`] sent, as the tests read it.
#[cfg(test)]
pub(crate) struct Probe(mpsc::UnboundedReceiver<Event>);

#[cfg(test)]
impl Probe {
    /// What was sent so far: `"press"`, `"release"`, `"cancel"`.
    pub(crate) fn sent(&mut self) -> Vec<&'static str> {
        let mut sent = Vec::new();
        while let Ok(event) = self.0.try_recv() {
            sent.push(match event {
                Event::Press(_) => "press",
                Event::Release(_) => "release",
                Event::Cancel => "cancel",
                Event::Stop => "stop",
                Event::Nudge => "nudge",
                _ => "other",
            });
        }
        sent
    }
}

/// The running menu: its thread, stopped when this is dropped.
#[derive(Debug)]
pub struct RadialService {
    handle: RadialHandle,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RadialService {
    /// Starts the menu on its window. The configuration and the lock are
    /// followed from `config` and `locked`, for as long as the service runs.
    /// The window is made on the thread of the menu, so that a compositor that
    /// is slow to answer does not hold the manager back. Without a window (no
    /// Wayland, no layer-shell) the service runs and the menu simply never
    /// opens, which is said once.
    #[must_use]
    pub fn start(
        config: watch::Receiver<RadialMenuConfig>,
        locked: watch::Receiver<bool>,
        runtime: &tokio::runtime::Handle,
        nudge: impl Fn() + Send + Sync + 'static,
        chooser: impl Fn(&RadialEntryConfig) + Send + 'static,
        presentation: Presentation,
    ) -> Self {
        let parts = Parts {
            nudge: Arc::new(nudge),
            chooser: Box::new(chooser),
            presentation,
        };
        Self::start_on(
            |events| {
                let report = Arc::new(move |event| events.send(Event::Window(event)).is_ok());
                match backend::start(report) {
                    Ok(window) => Some(window),
                    Err(error) => {
                        tracing::warn!(%error, "the radial menu has no window: it will not open");
                        None
                    }
                }
            },
            parts,
            config,
            locked,
            runtime,
        )
    }

    /// `make_surface` runs on the menu's thread, and is given where the window
    /// reports to.
    fn start_on<S: Surface + Send + 'static>(
        make_surface: impl FnOnce(mpsc::UnboundedSender<Event>) -> Option<S> + Send + 'static,
        parts: Parts,
        config: watch::Receiver<RadialMenuConfig>,
        locked: watch::Receiver<bool>,
        runtime: &tokio::runtime::Handle,
    ) -> Self {
        let (events, received) = mpsc::unbounded_channel();
        let first = config.borrow().clone();
        runtime.spawn(follow(config, locked.clone(), events.clone()));
        let window_events = events.clone();
        let Parts {
            nudge,
            chooser,
            presentation,
        } = parts;
        let wake = PointerWake {
            nudge,
            schedule: {
                let (runtime, events) = (runtime.clone(), events.clone());
                Box::new(move || {
                    runtime.spawn(nudges(events.clone()));
                })
            },
            refresh: {
                let (runtime, events) = (runtime.clone(), events.clone());
                Box::new(move || {
                    Box::new(StopOnDrop(runtime.spawn(refreshes(events.clone())))) as RefreshTimer
                })
            },
        };
        let thread = std::thread::Builder::new()
            .name("radial-menu".to_owned())
            .spawn(move || {
                let surface = make_surface(window_events);
                let mut menu = Menu::new(&first, locked, surface, chooser, wake, presentation);
                run(&mut menu, received);
            })
            .map_err(|error| tracing::error!(%error, "cannot start the radial menu thread"))
            .ok();
        Self {
            handle: RadialHandle { events },
            thread,
        }
    }

    /// Where the mice tell the menu about its button.
    #[must_use]
    pub fn handle(&self) -> RadialHandle {
        self.handle.clone()
    }
}

impl Drop for RadialService {
    fn drop(&mut self) {
        // Handles are still held (by the mice, by the task that follows the
        // configuration): the thread is told to stop rather than left to wait
        // for the channel to close.
        self.handle.send(Event::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Asks, a few times, that the pointer be nudged: the window is not shown at
/// once, and one that is shown under a pointer that does not move is not told
/// where it is.
async fn nudges(events: mpsc::UnboundedSender<Event>) {
    for _ in 0..NUDGE_TRIES {
        tokio::time::sleep(NUDGE_PERIOD).await;
        if events.send(Event::Nudge).is_err() {
            return;
        }
    }
}

/// Asks, until it is stopped (the menu closes) or the menu is gone, that what
/// the modules say be read again.
async fn refreshes(events: mpsc::UnboundedSender<Event>) {
    loop {
        tokio::time::sleep(REFRESH_PERIOD).await;
        if events.send(Event::Refresh).is_err() {
            return;
        }
    }
}

/// What stops the readings when it is dropped.
type RefreshTimer = Box<dyn Send>;

/// Aborts its task when dropped.
struct StopOnDrop(tokio::task::JoinHandle<()>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// What the menu shows and feels, besides its configuration.
pub struct Presentation {
    /// Where the feel of the menu is played.
    pub feel: Arc<dyn HapticOutput>,
    /// What the modules always look like: names and icons.
    pub looks: Looks,
    /// What the modules say now under their names.
    pub statuses: Statuses,
}

/// How the menu learns where the pointer is when the desktop does not say.
struct PointerWake {
    /// Moves the pointer one pixel and back.
    nudge: Arc<dyn Fn() + Send + Sync>,
    /// Asks for [`Event::Nudge`]s to come, after the window is opened.
    schedule: Box<dyn Fn() + Send>,
    /// Asks for [`Event::Refresh`]es to come, after the window is opened,
    /// until the timer it returns is dropped.
    refresh: Box<dyn Fn() -> RefreshTimer + Send>,
}

/// Passes the changes of the configuration and of the lock to the menu, until
/// the menu is gone.
async fn follow(
    mut config: watch::Receiver<RadialMenuConfig>,
    mut locked: watch::Receiver<bool>,
    events: mpsc::UnboundedSender<Event>,
) {
    loop {
        let event = tokio::select! {
            () = events.closed() => return,
            changed = config.changed() => match changed {
                Ok(()) => Event::Config(Box::new(config.borrow_and_update().clone())),
                Err(_) => return,
            },
            changed = locked.changed() => match changed {
                Ok(()) => Event::Locked(*locked.borrow_and_update()),
                Err(_) => return,
            },
        };
        if events.send(event).is_err() {
            return;
        }
    }
}

/// What is done with the entry that is chosen.
type Chooser = Box<dyn Fn(&RadialEntryConfig) + Send>;

/// What the menu is given besides its window and its configuration.
struct Parts {
    /// Moves the pointer a pixel and back.
    nudge: Arc<dyn Fn() + Send + Sync>,
    chooser: Chooser,
    presentation: Presentation,
}

/// The menu and what it is shown on.
struct Menu<S: Surface> {
    controller: Controller,
    surface: Option<S>,
    chooser: Chooser,
    wake: PointerWake,
    /// Where the feel of the menu is played, when the configuration wants it.
    feel: Arc<dyn HapticOutput>,
    haptics: bool,
    animate: bool,
    /// Read when a press comes and when an entry is chosen, not only when the
    /// change is told: a press or a click queued before the news of a lock
    /// must not get through.
    locked: watch::Receiver<bool>,
    started: Instant,
    /// What the petals say, as the window shows it: a reading that changes
    /// nothing draws nothing.
    shown_statuses: Vec<Option<String>>,
    /// The readings that go on while the menu is open.
    refreshing: Option<RefreshTimer>,
    /// A press that found no window is said once.
    warned: bool,
}

impl<S: Surface> Menu<S> {
    fn new(
        config: &RadialMenuConfig,
        locked: watch::Receiver<bool>,
        surface: Option<S>,
        chooser: Chooser,
        wake: PointerWake,
        presentation: Presentation,
    ) -> Self {
        let Presentation {
            feel,
            looks,
            statuses,
        } = presentation;
        let mut controller = Controller::new(config, DEFAULT_RING, UNKNOWN_SCREEN);
        controller.set_looks(looks);
        controller.set_statuses(statuses);
        Self {
            controller,
            surface,
            chooser,
            wake,
            feel,
            haptics: config.haptics,
            animate: config.animate,
            locked,
            started: Instant::now(),
            shown_statuses: Vec::new(),
            refreshing: None,
            warned: false,
        }
    }

    fn handle(&mut self, event: Event) {
        let effects = match event {
            Event::Press(at) => self.press(at),
            Event::Release(at) => self
                .controller
                .release(at.saturating_duration_since(self.started)),
            Event::Nudge => {
                // Still waiting for the pointer to say where it is.
                if self.controller.is_open() && self.controller.view().is_none() {
                    (self.wake.nudge)();
                }
                Vec::new()
            }
            Event::Refresh => {
                self.refresh_statuses();
                Vec::new()
            }
            Event::Stop => Vec::new(),
            Event::Cancel | Event::Window(WindowEvent::Escape) => {
                self.controller.window(WindowEvent::Escape)
            }
            Event::Window(WindowEvent::Gone) => {
                tracing::warn!("the window of the radial menu is gone: the menu will not open");
                let effects = self.controller.window(WindowEvent::Gone);
                self.apply(effects);
                // Nothing to show the menu on any more, and nothing is chosen
                // from a menu nobody sees.
                self.surface = None;
                self.warned = true;
                return;
            }
            Event::Window(event) => self.controller.window(event),
            Event::Config(config) => {
                self.animate = config.animate;
                self.haptics = config.haptics;
                self.controller.configure(&config);
                Vec::new()
            }
            Event::Locked(locked) => {
                // A lock closes an open menu: it holds the keyboard.
                if locked {
                    self.controller.window(WindowEvent::Escape)
                } else {
                    Vec::new()
                }
            }
        };
        self.apply(effects);
    }

    fn press(&mut self, at: Instant) -> Vec<Effect> {
        if *self.locked.borrow() {
            return Vec::new();
        }
        if self.surface.is_none() {
            if !self.warned {
                self.warned = true;
                tracing::warn!("the radial menu button was pressed, but there is no window");
            }
            return Vec::new();
        }
        self.controller
            .press(at.saturating_duration_since(self.started))
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Open => {
                    let content = self.controller.content(DEFAULT_PALETTE, self.animate);
                    if let (Some(surface), Some(content)) = (&self.surface, content) {
                        self.shown_statuses = content
                            .petals
                            .iter()
                            .map(|petal| petal.status.clone())
                            .collect();
                        surface.open(content);
                        (self.wake.schedule)();
                        // Replacing the timer of an earlier opening stops it.
                        self.refreshing = Some((self.wake.refresh)());
                        self.feel(OPEN_WAVEFORM);
                    }
                }
                Effect::Place(view) => self.with_surface(|surface| surface.place(view)),
                Effect::Redraw(view) => {
                    self.with_surface(|surface| surface.redraw(view));
                    // A petal is felt when the pointer comes onto it, not when
                    // it leaves it for the void.
                    if view.lit.is_some() {
                        self.feel(HOVER_WAVEFORM);
                    }
                }
                Effect::Choose(entry) => {
                    if *self.locked.borrow() {
                        tracing::debug!(
                            "a radial entry was chosen as the computer locked: dropped"
                        );
                    } else {
                        (self.chooser)(&entry);
                        self.feel(CHOICE_WAVEFORM);
                    }
                }
                Effect::Close => {
                    self.refreshing = None;
                    self.with_surface(Surface::close);
                }
            }
        }
    }

    /// Reads again what the modules say, and shows it if it changed.
    fn refresh_statuses(&mut self) {
        if !self.controller.is_open() {
            return;
        }
        let statuses = self.controller.statuses();
        if statuses != self.shown_statuses {
            self.with_surface(|surface| surface.update_statuses(statuses.clone()));
            self.shown_statuses = statuses;
        }
    }

    /// Plays `waveform` on the haptic mice, when the configuration of the menu
    /// asks for haptics.
    fn feel(&self, waveform: HapticWaveform) {
        if self.haptics {
            self.feel.play_waveform(waveform);
        }
    }

    fn with_surface(&self, job: impl FnOnce(&S)) {
        if let Some(surface) = &self.surface {
            job(surface);
        }
    }
}

fn run<S: Surface>(menu: &mut Menu<S>, mut received: mpsc::UnboundedReceiver<Event>) {
    while let Some(event) = received.blocking_recv() {
        if matches!(event, Event::Stop) {
            // An open menu is closed with the window.
            menu.handle(Event::Cancel);
            return;
        }
        menu.handle(event);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;
    use crate::radial::controller::View;
    use crate::radial::geometry::Point;
    use crate::radial::window::Content;

    /// A window that notes what it is asked.
    #[derive(Clone, Default)]
    struct Notes(Arc<Mutex<Vec<String>>>, Arc<Mutex<Vec<String>>>);

    impl Notes {
        fn push(&self, note: &str) {
            self.0.lock().unwrap().push(note.to_owned());
        }

        fn all(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }

        /// What the pointer wake was asked, apart from what the window was.
        fn push_wake(&self, note: &str) {
            self.1.lock().unwrap().push(note.to_owned());
        }

        fn wakes(&self) -> Vec<String> {
            self.1.lock().unwrap().clone()
        }
    }

    impl HapticOutput for Notes {
        fn play(&self, _: &crate::haptics::HapticPlay) {}

        fn play_waveform(&self, waveform: HapticWaveform) {
            self.push_wake(&format!("feel {waveform:?}"));
        }
    }

    impl Surface for Notes {
        fn open(&self, content: Content) {
            self.push(&format!("open {}", content.petals.len()));
        }

        fn place(&self, _: View) {
            self.push("place");
        }

        fn redraw(&self, view: View) {
            self.push(&format!("redraw {:?}", view.lit));
        }

        fn update_statuses(&self, statuses: Vec<Option<String>>) {
            self.push(&format!("statuses {statuses:?}"));
        }

        fn close(&self) {
            self.push("close");
        }
    }

    /// Notes that the readings stopped when it is dropped.
    struct StoppedNote(Notes);

    impl Drop for StoppedNote {
        fn drop(&mut self) {
            self.0.push_wake("refresh stopped");
        }
    }

    /// A pointer wake that writes `nudge` and `schedule` in `notes`.
    fn quiet_wake(notes: &Notes) -> PointerWake {
        let (for_nudge, for_schedule, for_refresh) = (notes.clone(), notes.clone(), notes.clone());
        PointerWake {
            nudge: Arc::new(move || for_nudge.push_wake("nudge")),
            schedule: Box::new(move || for_schedule.push_wake("schedule")),
            refresh: Box::new(move || {
                for_refresh.push_wake("refresh");
                Box::new(StoppedNote(for_refresh.clone())) as RefreshTimer
            }),
        }
    }

    /// A menu on `notes`, and what sets whether the computer is locked.
    fn menu(notes: &Notes, locked: bool, chosen: &Notes) -> (Menu<Notes>, watch::Sender<bool>) {
        let chosen = chosen.clone();
        let (lock, locked) = watch::channel(locked);
        let menu = Menu::new(
            &RadialMenuConfig::default_menu(),
            locked,
            Some(notes.clone()),
            Box::new(move |entry| chosen.push(&entry.module.to_string())),
            quiet_wake(notes),
            presentation(notes, Statuses::default()),
        );
        (menu, lock)
    }

    fn presentation(notes: &Notes, statuses: Statuses) -> Presentation {
        Presentation {
            feel: Arc::new(notes.clone()),
            looks: Looks::default(),
            statuses,
        }
    }

    /// A menu whose pomodoro petal says what `says` holds.
    fn menu_saying(notes: &Notes, says: &Arc<Mutex<String>>) -> (Menu<Notes>, watch::Sender<bool>) {
        let reader = Arc::clone(says);
        let (lock, locked) = watch::channel(false);
        let menu = Menu::new(
            &RadialMenuConfig::default_menu(),
            locked,
            Some(notes.clone()),
            Box::new(|_| {}),
            quiet_wake(notes),
            presentation(
                notes,
                Statuses::new(move |module| {
                    (module.0 == "pomodoro").then(|| reader.lock().unwrap().clone())
                }),
            ),
        );
        (menu, lock)
    }

    #[test]
    fn what_the_petals_say_is_read_again_while_the_menu_is_open_and_drawn_when_it_changed() {
        let notes = Notes::default();
        let says = Arc::new(Mutex::new("Focus 1/4".to_owned()));
        let (mut menu, _lock) = menu_saying(&notes, &says);

        // Closed: nothing to update.
        menu.handle(Event::Refresh);
        assert_eq!(notes.all().len(), 0);

        open(&mut menu);
        assert_eq!(notes.all(), ["open 4", "place"]);
        assert!(
            notes
                .wakes()
                .starts_with(&["schedule".to_owned(), "refresh".to_owned()])
        );

        menu.handle(Event::Refresh);
        assert_eq!(notes.all().len(), 2, "nothing changed: nothing drawn");

        "Focus 2/4".clone_into(&mut says.lock().unwrap());
        menu.handle(Event::Refresh);
        assert_eq!(
            notes.all().last().unwrap(),
            r#"statuses [Some("Focus 2/4"), None, None, None]"#
        );
        menu.handle(Event::Refresh);
        assert_eq!(notes.all().len(), 3, "said once");
    }

    #[test]
    fn the_readings_stop_when_the_menu_closes_and_a_new_opening_replaces_them() {
        let notes = Notes::default();
        let says = Arc::new(Mutex::new("Focus 1/4".to_owned()));
        let (mut menu, _lock) = menu_saying(&notes, &says);
        let stopped = |notes: &Notes| {
            notes
                .wakes()
                .iter()
                .filter(|note| *note == "refresh stopped")
                .count()
        };

        open(&mut menu);
        assert_eq!(stopped(&notes), 0);
        menu.handle(Event::Cancel);
        assert_eq!(stopped(&notes), 1, "the menu closed");

        open(&mut menu);
        menu.handle(Event::Cancel);
        assert_eq!(stopped(&notes), 2, "one timer per opening, none left over");
    }

    fn at(menu: &Menu<Notes>, milliseconds: u64) -> Instant {
        menu.started + Duration::from_millis(milliseconds)
    }

    /// Opens the menu with the pointer at (900, 500); the menu is then centred
    /// there and its petals are the first at (900, 410) and the second at (990, 500).
    fn open(menu: &mut Menu<Notes>) {
        menu.handle(Event::Press(at(menu, 0)));
        menu.handle(Event::Window(WindowEvent::Pointer(Point::new(
            900.0, 500.0,
        ))));
    }

    #[test]
    fn a_press_opens_the_window_and_the_first_pointer_places_the_menu() {
        let (notes, chosen) = (Notes::default(), Notes::default());
        let (mut menu, _lock) = menu(&notes, false, &chosen);
        open(&mut menu);
        assert_eq!(notes.all(), ["open 4", "place"]);
    }

    #[test]
    fn held_long_enough_releasing_on_a_petal_chooses_and_closes() {
        let (notes, chosen) = (Notes::default(), Notes::default());
        let (mut menu, _lock) = menu(&notes, false, &chosen);
        open(&mut menu);
        menu.handle(Event::Window(WindowEvent::Pointer(Point::new(
            990.0, 500.0,
        ))));
        menu.handle(Event::Release(at(&menu, 600)));
        assert_eq!(notes.all(), ["open 4", "place", "redraw Some(1)", "close"]);
        assert_eq!(chosen.all(), ["teams"]);
    }

    #[test]
    fn released_early_the_menu_stays_and_a_click_chooses() {
        let (notes, chosen) = (Notes::default(), Notes::default());
        let (mut menu, _lock) = menu(&notes, false, &chosen);
        open(&mut menu);
        menu.handle(Event::Window(WindowEvent::Pointer(Point::new(
            900.0, 590.0,
        ))));
        menu.handle(Event::Release(at(&menu, 50)));
        assert_eq!(chosen.all(), [] as [String; 0]);
        menu.handle(Event::Window(WindowEvent::Click));
        assert_eq!(chosen.all(), ["audio"]);
        assert_eq!(notes.all().last().map(String::as_str), Some("close"));
    }

    #[test]
    fn the_mouse_going_away_closes_the_menu_without_choosing() {
        let (notes, chosen) = (Notes::default(), Notes::default());
        let (mut menu, _lock) = menu(&notes, false, &chosen);
        open(&mut menu);
        menu.handle(Event::Window(WindowEvent::Pointer(Point::new(
            990.0, 500.0,
        ))));
        menu.handle(Event::Cancel);
        assert_eq!(notes.all().last().map(String::as_str), Some("close"));
        assert_eq!(chosen.all(), [] as [String; 0]);
    }

    #[test]
    fn nothing_opens_while_the_computer_is_locked() {
        let (notes, chosen) = (Notes::default(), Notes::default());
        let (mut menu, _lock) = menu(&notes, true, &chosen);
        menu.handle(Event::Press(at(&menu, 0)));
        assert_eq!(notes.all(), [] as [String; 0]);
    }

    #[test]
    fn a_lock_that_comes_while_the_menu_is_open_closes_it() {
        let (notes, chosen) = (Notes::default(), Notes::default());
        let (mut menu, lock) = menu(&notes, false, &chosen);
        open(&mut menu);
        lock.send_replace(true);
        menu.handle(Event::Locked(true));
        assert_eq!(notes.all().last().map(String::as_str), Some("close"));
        // Unlocked again, a press opens the menu again.
        lock.send_replace(false);
        menu.handle(Event::Locked(false));
        menu.handle(Event::Press(at(&menu, 5000)));
        assert_eq!(notes.all().last().map(String::as_str), Some("open 4"));
    }

    #[test]
    fn a_menu_without_a_window_stays_closed_and_says_so_once() {
        let chosen = Notes::default();
        let chosen_for_menu = chosen.clone();
        let (_lock, locked) = watch::channel(false);
        let mut menu: Menu<Notes> = Menu::new(
            &RadialMenuConfig::default_menu(),
            locked,
            None,
            Box::new(move |entry| chosen_for_menu.push(&entry.module.to_string())),
            quiet_wake(&Notes::default()),
            Presentation {
                feel: Arc::new(Notes::default()),
                looks: Looks::default(),
                statuses: Statuses::default(),
            },
        );
        menu.handle(Event::Press(menu.started));
        menu.handle(Event::Press(menu.started));
        assert!(!menu.controller.is_open());
        assert!(menu.warned);
        assert_eq!(chosen.all(), [] as [String; 0]);
    }

    #[test]
    fn the_configuration_that_comes_is_used_from_the_next_opening() {
        let (notes, chosen) = (Notes::default(), Notes::default());
        let (mut menu, _lock) = menu(&notes, false, &chosen);
        let mut config = RadialMenuConfig::default_menu();
        config.entries[0].enabled = false;
        config.animate = false;
        menu.handle(Event::Config(Box::new(config)));
        assert!(!menu.animate);
        menu.handle(Event::Press(at(&menu, 0)));
        assert_eq!(notes.all(), ["open 3"]);
    }

    #[test]
    fn opening_asks_for_nudges_and_a_nudge_only_comes_while_the_menu_waits_for_the_pointer() {
        let (notes, chosen) = (Notes::default(), Notes::default());
        let (mut menu, _lock) = menu(&notes, false, &chosen);
        menu.handle(Event::Nudge);
        assert_eq!(
            wake_calls(&notes),
            [] as [String; 0],
            "closed: nothing to nudge for"
        );
        menu.handle(Event::Press(at(&menu, 0)));
        assert_eq!(wake_calls(&notes), ["schedule", "refresh"]);
        menu.handle(Event::Nudge);
        menu.handle(Event::Nudge);
        // The pointer says where it is: the menu is placed, no more nudges.
        menu.handle(Event::Window(WindowEvent::Pointer(Point::new(
            900.0, 500.0,
        ))));
        menu.handle(Event::Nudge);
        assert_eq!(
            wake_calls(&notes),
            ["schedule", "refresh", "nudge", "nudge"]
        );
        assert_eq!(notes.all(), ["open 4", "place"]);
    }

    /// What the pointer wake was asked, leaving the feel of the menu aside.
    fn wake_calls(notes: &Notes) -> Vec<String> {
        notes
            .wakes()
            .into_iter()
            .filter(|note| !note.starts_with("feel"))
            .collect()
    }

    fn feels(notes: &Notes) -> Vec<String> {
        notes
            .wakes()
            .into_iter()
            .filter(|note| note.starts_with("feel"))
            .collect()
    }

    #[test]
    fn the_menu_is_felt_opening_hovered_and_chosen() {
        let (notes, chosen) = (Notes::default(), Notes::default());
        let (mut menu, _lock) = menu(&notes, false, &chosen);
        open(&mut menu);
        assert_eq!(feels(&notes), ["feel SubtleCollision"]);
        menu.handle(Event::Window(WindowEvent::Pointer(Point::new(
            990.0, 500.0,
        ))));
        assert_eq!(
            feels(&notes),
            ["feel SubtleCollision", "feel WhisperCollision"]
        );
        // Leaving for the void is not felt.
        menu.handle(Event::Window(WindowEvent::Pointer(Point::new(
            900.0, 500.0,
        ))));
        assert_eq!(feels(&notes).len(), 2);
        menu.handle(Event::Window(WindowEvent::Pointer(Point::new(
            990.0, 500.0,
        ))));
        menu.handle(Event::Release(at(&menu, 600)));
        let all = feels(&notes);
        assert_eq!(
            all.last().map(String::as_str),
            Some("feel SharpStateChange")
        );
        assert_eq!(chosen.all(), ["teams"]);
    }

    #[test]
    fn nothing_is_felt_when_the_configuration_has_no_haptics() {
        let (notes, chosen) = (Notes::default(), Notes::default());
        let (mut menu, _lock) = menu(&notes, false, &chosen);
        let mut config = RadialMenuConfig::default_menu();
        config.haptics = false;
        menu.handle(Event::Config(Box::new(config)));
        open(&mut menu);
        menu.handle(Event::Window(WindowEvent::Pointer(Point::new(
            990.0, 500.0,
        ))));
        menu.handle(Event::Release(at(&menu, 600)));
        assert_eq!(feels(&notes), [] as [String; 0]);
        assert_eq!(chosen.all(), ["teams"], "the choice is made all the same");
    }

    #[test]
    fn a_choice_dropped_for_a_lock_is_not_felt() {
        let (notes, chosen) = (Notes::default(), Notes::default());
        let (mut menu, lock) = menu(&notes, false, &chosen);
        open(&mut menu);
        menu.handle(Event::Window(WindowEvent::Pointer(Point::new(
            990.0, 500.0,
        ))));
        lock.send_replace(true);
        menu.handle(Event::Release(at(&menu, 600)));
        assert!(
            !feels(&notes)
                .iter()
                .any(|note| note.contains("SharpStateChange"))
        );
        assert_eq!(chosen.all(), [] as [String; 0]);
    }

    #[test]
    fn a_click_queued_before_the_news_of_a_lock_chooses_nothing() {
        let (notes, chosen) = (Notes::default(), Notes::default());
        let (mut menu, lock) = menu(&notes, false, &chosen);
        open(&mut menu);
        menu.handle(Event::Window(WindowEvent::Pointer(Point::new(
            990.0, 500.0,
        ))));
        menu.handle(Event::Release(at(&menu, 50)));
        // The lock came, but its news is behind the click in the queue.
        lock.send_replace(true);
        menu.handle(Event::Window(WindowEvent::Click));
        assert_eq!(chosen.all(), [] as [String; 0]);
        assert_eq!(notes.all().last().map(String::as_str), Some("close"));
    }

    #[test]
    fn a_press_queued_before_the_news_of_a_lock_opens_nothing() {
        let (notes, chosen) = (Notes::default(), Notes::default());
        let (mut menu, lock) = menu(&notes, false, &chosen);
        lock.send_replace(true);
        menu.handle(Event::Press(at(&menu, 0)));
        assert_eq!(notes.all(), [] as [String; 0]);
    }

    #[test]
    fn a_window_that_is_gone_ends_the_menu_and_nothing_is_chosen_blind() {
        let (notes, chosen) = (Notes::default(), Notes::default());
        let (mut menu, _lock) = menu(&notes, false, &chosen);
        open(&mut menu);
        menu.handle(Event::Window(WindowEvent::Pointer(Point::new(
            990.0, 500.0,
        ))));
        menu.handle(Event::Release(at(&menu, 50)));
        menu.handle(Event::Window(WindowEvent::Gone));
        // What would have chosen petal 1 now finds no menu and no window.
        menu.handle(Event::Press(at(&menu, 100)));
        menu.handle(Event::Release(at(&menu, 900)));
        menu.handle(Event::Window(WindowEvent::Click));
        assert_eq!(chosen.all(), [] as [String; 0]);
        assert!(!menu.controller.is_open());
        assert!(menu.surface.is_none());
    }

    #[tokio::test]
    async fn the_service_follows_the_button_the_lock_and_stops_with_its_window() {
        let notes = Notes::default();
        let (config_sender, config) = watch::channel(RadialMenuConfig::default_menu());
        let (lock_sender, locked) = watch::channel(false);
        let for_menu = notes.clone();
        let service = RadialService::start_on(
            move |_| Some(for_menu),
            Parts {
                nudge: Arc::new(|| {}),
                chooser: Box::new(|_| {}),
                presentation: Presentation {
                    feel: Arc::new(crate::haptics::NoHaptics),
                    looks: Looks::default(),
                    statuses: Statuses::default(),
                },
            },
            config,
            locked,
            &tokio::runtime::Handle::current(),
        );
        let handle = service.handle();
        let wait_for = |note: &'static str| {
            let notes = notes.clone();
            async move {
                tokio::time::timeout(Duration::from_secs(5), async {
                    while notes.all().last().map(String::as_str) != Some(note) {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await
                .unwrap();
            }
        };
        handle.press();
        wait_for("open 4").await;
        // The lock closes the menu the press opened; the configuration that
        // comes meanwhile is taken without harm.
        config_sender.send_replace(RadialMenuConfig::default_menu());
        lock_sender.send_replace(true);
        wait_for("close").await;
        assert_eq!(notes.all().first().map(String::as_str), Some("open 4"));
        drop(service);
        handle.press();
    }
}
