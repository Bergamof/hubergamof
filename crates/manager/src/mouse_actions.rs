//! What a mouse button does when it is pressed: a click, a double click, a
//! shortcut, a switch of the wheel (the radial menu and the actions of the
//! modules come later).
//!
//! Keys and clicks go through one virtual input device shared by the mice, created
//! when first needed. Nothing is sent while the computer is locked or the
//! session is not the active one (the rule of the devices, `docs/security.md`),
//! and the device is closed then, which releases whatever it held.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use hubergamof_mouse::link::ButtonEvent;
use hubergamof_mouse::session::Request;
use hubergamof_protocol::{ButtonAction, MouseControlId};
use tokio::sync::watch;

use crate::input::{Injector, Key, UinputInjector, key_of_button, parse_shortcut};

/// Opens the device keys and clicks are sent through.
type Opener = dyn Fn() -> io::Result<Box<dyn Injector>> + Send + Sync;

struct Shared {
    opener: Box<Opener>,
    /// The open device, `None` before the first action and while locked.
    injector: Mutex<Option<Box<dyn Injector>>>,
    /// `true` while the computer is locked or the session is not the active one.
    locked: watch::Receiver<bool>,
}

/// The device the actions of the buttons go through, shared by the mice.
#[derive(Clone)]
pub struct ButtonActions {
    shared: Arc<Shared>,
}

impl ButtonActions {
    /// Actions that open their device with `opener`, held off while `locked`
    /// is `true`.
    pub fn new(
        locked: watch::Receiver<bool>,
        opener: impl Fn() -> io::Result<Box<dyn Injector>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                opener: Box::new(opener),
                injector: Mutex::new(None),
                locked,
            }),
        }
    }

    /// Actions through `/dev/uinput`.
    #[must_use]
    pub fn uinput(locked: watch::Receiver<bool>) -> Self {
        Self::new(locked, || {
            UinputInjector::open().map(|device| Box::new(device) as Box<dyn Injector>)
        })
    }

    fn injector(&self) -> MutexGuard<'_, Option<Box<dyn Injector>>> {
        // A device and nothing else: nothing to recover from a poisoned lock.
        self.shared
            .injector
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Closes the device when the computer is locked (what it held is released
    /// with it). Cheap: asked about at every turn of the mouse threads.
    pub fn close_if_locked(&self) {
        if *self.shared.locked.borrow() {
            self.injector().take();
        }
    }

    /// Runs `job` on the device, opening it first; `None` (and nothing done)
    /// while locked. The lock is read with the device held, so that no thread
    /// reopens a device another one has just closed for the lock.
    fn with_device<T>(
        &self,
        job: impl FnOnce(&mut dyn Injector) -> io::Result<T>,
    ) -> Option<io::Result<T>> {
        let mut device = self.injector();
        if *self.shared.locked.borrow() {
            device.take();
            return None;
        }
        if device.is_none() {
            match (self.shared.opener)() {
                Ok(opened) => *device = Some(opened),
                Err(error) => return Some(Err(error)),
            }
            // Opening takes a moment: the lock may have come meanwhile.
            if *self.shared.locked.borrow() {
                device.take();
                return None;
            }
        }
        device.as_mut().map(|injector| job(injector.as_mut()))
    }

    /// Releases a key held down, if the device is still there (a closed
    /// device has released everything). A release that fails closes the device
    /// for the same reason: the system then lets go of the key.
    fn release(&self, key: Key) -> io::Result<()> {
        let mut device = self.injector();
        if *self.shared.locked.borrow() {
            device.take();
            return Ok(());
        }
        let Some(injector) = device.as_mut() else {
            return Ok(());
        };
        let outcome = injector.send(key, false);
        if outcome.is_err() {
            device.take();
        }
        outcome
    }
}

/// The actions of one mouse: which key each held button holds down, and what it
/// asked of the session.
pub struct ButtonDispatcher {
    actions: ButtonActions,
    held: BTreeMap<MouseControlId, Key>,
    requests: VecDeque<Request>,
}

impl ButtonDispatcher {
    /// A dispatcher on the shared `actions`.
    #[must_use]
    pub fn new(actions: ButtonActions) -> Self {
        Self {
            actions,
            held: BTreeMap::new(),
            requests: VecDeque::new(),
        }
    }

    /// What the session is asked to do (a switch of the wheel), in order.
    pub fn next_request(&mut self) -> Option<Request> {
        self.actions.close_if_locked();
        self.requests.pop_front()
    }

    /// Carries out what a press or a release of a button asks, `action` being
    /// what the configuration binds to its control (`None` for a control left
    /// as the mouse has it).
    pub fn handle(&mut self, event: ButtonEvent, action: Option<&ButtonAction>) {
        if event.pressed {
            if let Some(action) = action {
                self.press(event.control, action);
            }
        } else {
            self.release_control(event.control);
        }
    }

    /// Lets go of the key `control` holds, unless another button of this
    /// mouse still holds the same one.
    fn release_control(&mut self, control: MouseControlId) {
        let Some(key) = self.held.remove(&control) else {
            return;
        };
        if self.held.values().any(|held| *held == key) {
            return;
        }
        if let Err(error) = self.actions.release(key) {
            tracing::warn!(%error, "cannot release a key of a mouse button");
        }
    }

    fn press(&mut self, control: MouseControlId, action: &ButtonAction) {
        // A press without the release before it (a lost event) must not leave
        // the earlier key held with nobody to let it go.
        self.release_control(control);
        match action {
            ButtonAction::None => {}
            ButtonAction::Click { button } => {
                let key = key_of_button(*button);
                // Held down until the button comes up, so that a drag works.
                if let Some(Ok(())) = self.send(|device| device.send(key, true)) {
                    self.held.insert(control, key);
                }
            }
            ButtonAction::DoubleClick => {
                let key = key_of_button(hubergamof_protocol::MouseButton::Left);
                self.send(|device| {
                    device.tap(key)?;
                    device.tap(key)
                });
            }
            ButtonAction::Shortcut { keys } => match parse_shortcut(keys) {
                Ok(keys) => {
                    self.send(|device| device.tap_shortcut(&keys));
                }
                Err(error) => tracing::warn!(%error, "a shortcut that cannot be pressed"),
            },
            ButtonAction::ToggleWheelMode => {
                // Two toggles cancel out: one waiting is enough, and the mouse
                // is written to once.
                if !self.requests.contains(&Request::ToggleWheelMode) {
                    self.requests.push_back(Request::ToggleWheelMode);
                }
            }
            ButtonAction::RadialMenu | ButtonAction::Module { .. } => {
                tracing::debug!(%control, "the radial menu and the module actions are not available yet");
            }
        }
    }

    /// Runs `job` on the device, reporting a failure; `None` while locked.
    fn send<T>(
        &self,
        job: impl FnOnce(&mut dyn Injector) -> io::Result<T>,
    ) -> Option<io::Result<T>> {
        let outcome = self.actions.with_device(job)?;
        if let Err(error) = &outcome {
            tracing::warn!(%error, "cannot send keys or clicks (is /dev/uinput open to you? docs/security.md)");
        }
        Some(outcome)
    }
}

impl Drop for ButtonDispatcher {
    /// A thread that ends in a panic with a button held must not leave its
    /// key down on the device the other mice keep open.
    fn drop(&mut self) {
        let controls: Vec<_> = self.held.keys().copied().collect();
        for control in controls {
            self.release_control(control);
        }
    }
}

#[cfg(test)]
mod tests {
    use hubergamof_protocol::MouseButton;

    use super::*;

    /// A device that records what it is told, kept by the test.
    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<(Key, bool)>>>);

    impl Recorder {
        fn events(&self) -> Vec<(Key, bool)> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Injector for Recorder {
        fn send(&mut self, key: Key, pressed: bool) -> io::Result<()> {
            self.0.lock().unwrap().push((key, pressed));
            Ok(())
        }
    }

    /// The device as the dispatcher holds it: counts its closings, and can be
    /// made to fail on a release.
    struct Device {
        recorder: Recorder,
        closed: Arc<Mutex<u32>>,
        fail_release: bool,
    }

    impl Injector for Device {
        fn send(&mut self, key: Key, pressed: bool) -> io::Result<()> {
            if !pressed && self.fail_release {
                return Err(io::Error::from(io::ErrorKind::WriteZero));
            }
            self.recorder.send(key, pressed)
        }
    }

    impl Drop for Device {
        fn drop(&mut self) {
            *self.closed.lock().unwrap() += 1;
        }
    }

    struct Rig {
        recorder: Recorder,
        lock: watch::Sender<bool>,
        opened: Arc<Mutex<u32>>,
        closed: Arc<Mutex<u32>>,
        dispatcher: ButtonDispatcher,
    }

    fn rig() -> Rig {
        rig_with(false)
    }

    fn rig_with(fail_release: bool) -> Rig {
        let recorder = Recorder::default();
        let (lock, locked) = watch::channel(false);
        let opened = Arc::new(Mutex::new(0));
        let closed = Arc::new(Mutex::new(0));
        let actions = ButtonActions::new(locked, {
            let recorder = recorder.clone();
            let opened = Arc::clone(&opened);
            let closed = Arc::clone(&closed);
            move || {
                *opened.lock().unwrap() += 1;
                Ok(Box::new(Device {
                    recorder: recorder.clone(),
                    closed: Arc::clone(&closed),
                    fail_release,
                }) as Box<dyn Injector>)
            }
        });
        Rig {
            recorder,
            lock,
            opened,
            closed,
            dispatcher: ButtonDispatcher::new(actions),
        }
    }

    fn press(control: MouseControlId) -> ButtonEvent {
        ButtonEvent {
            control,
            pressed: true,
        }
    }

    fn release(control: MouseControlId) -> ButtonEvent {
        ButtonEvent {
            control,
            pressed: false,
        }
    }

    const BACK: MouseControlId = MouseControlId::Back;

    #[test]
    fn a_click_is_held_down_until_the_button_comes_up() {
        let mut rig = rig();
        let click = ButtonAction::Click {
            button: MouseButton::Middle,
        };
        rig.dispatcher.handle(press(BACK), Some(&click));
        assert_eq!(rig.recorder.events(), [(Key::ButtonMiddle, true)]);
        // The binding changes while the button is held: the release is the same.
        rig.dispatcher
            .handle(release(BACK), Some(&ButtonAction::None));
        assert_eq!(
            rig.recorder.events(),
            [(Key::ButtonMiddle, true), (Key::ButtonMiddle, false)]
        );
    }

    #[test]
    fn a_double_click_is_two_clicks_of_the_left_button_on_the_press_only() {
        let mut rig = rig();
        rig.dispatcher
            .handle(press(BACK), Some(&ButtonAction::DoubleClick));
        rig.dispatcher
            .handle(release(BACK), Some(&ButtonAction::DoubleClick));
        let click = [(Key::ButtonLeft, true), (Key::ButtonLeft, false)];
        assert_eq!(rig.recorder.events(), [click, click].concat());
    }

    #[test]
    fn a_shortcut_is_pressed_on_the_press_and_released_in_reverse() {
        let mut rig = rig();
        let shortcut = ButtonAction::Shortcut {
            keys: "Ctrl+Shift+M".to_owned(),
        };
        rig.dispatcher.handle(press(BACK), Some(&shortcut));
        rig.dispatcher.handle(release(BACK), Some(&shortcut));
        assert_eq!(
            rig.recorder.events(),
            [
                (Key::LeftCtrl, true),
                (Key::LeftShift, true),
                (Key::M, true),
                (Key::M, false),
                (Key::LeftShift, false),
                (Key::LeftCtrl, false),
            ]
        );
    }

    #[test]
    fn a_shortcut_that_cannot_be_pressed_sends_nothing() {
        let mut rig = rig();
        for keys in ["Ctrl+Power", "Ctrl+Alt+Delete", ""] {
            let shortcut = ButtonAction::Shortcut {
                keys: keys.to_owned(),
            };
            rig.dispatcher.handle(press(BACK), Some(&shortcut));
        }
        assert_eq!(rig.recorder.events(), []);
        assert_eq!(
            *rig.opened.lock().unwrap(),
            0,
            "the device is not even opened"
        );
    }

    #[test]
    fn a_control_without_an_action_or_with_none_does_nothing() {
        let mut rig = rig();
        rig.dispatcher.handle(press(BACK), None);
        rig.dispatcher
            .handle(press(BACK), Some(&ButtonAction::None));
        rig.dispatcher.handle(release(BACK), None);
        assert_eq!(rig.recorder.events(), []);
        assert_eq!(*rig.opened.lock().unwrap(), 0);
    }

    #[test]
    fn the_device_is_opened_once_when_first_needed() {
        let mut rig = rig();
        let click = ButtonAction::Click {
            button: MouseButton::Left,
        };
        for _ in 0..3 {
            rig.dispatcher.handle(press(BACK), Some(&click));
            rig.dispatcher.handle(release(BACK), Some(&click));
        }
        assert_eq!(*rig.opened.lock().unwrap(), 1);
    }

    #[test]
    fn the_wheel_toggle_is_asked_of_the_session_in_order() {
        let mut rig = rig();
        rig.dispatcher
            .handle(press(BACK), Some(&ButtonAction::ToggleWheelMode));
        rig.dispatcher.handle(
            press(MouseControlId::Forward),
            Some(&ButtonAction::ToggleWheelMode),
        );
        assert_eq!(
            rig.dispatcher.next_request(),
            Some(Request::ToggleWheelMode),
            "two toggles waiting cancel out: one is asked"
        );
        assert_eq!(rig.dispatcher.next_request(), None);
        rig.dispatcher
            .handle(press(BACK), Some(&ButtonAction::ToggleWheelMode));
        assert_eq!(
            rig.dispatcher.next_request(),
            Some(Request::ToggleWheelMode)
        );
        assert_eq!(rig.recorder.events(), []);
    }

    #[test]
    fn the_radial_menu_and_module_actions_do_nothing_yet() {
        let mut rig = rig();
        rig.dispatcher
            .handle(press(BACK), Some(&ButtonAction::RadialMenu));
        assert_eq!(rig.dispatcher.next_request(), None);
        assert_eq!(rig.recorder.events(), []);
    }

    #[test]
    fn nothing_is_sent_while_locked_and_the_device_is_closed() {
        let mut rig = rig();
        let click = ButtonAction::Click {
            button: MouseButton::Left,
        };
        rig.dispatcher.handle(press(BACK), Some(&click));
        assert_eq!(*rig.opened.lock().unwrap(), 1);
        rig.lock.send_replace(true);
        // The session is asked about every turn: that closes the device.
        rig.dispatcher.next_request();
        let before = rig.recorder.events().len();
        // The button held when the lock came is released without a device.
        rig.dispatcher.handle(release(BACK), Some(&click));
        rig.dispatcher.handle(press(BACK), Some(&click));
        rig.dispatcher.handle(
            press(MouseControlId::Forward),
            Some(&ButtonAction::DoubleClick),
        );
        assert_eq!(rig.recorder.events().len(), before, "nothing was sent");
        assert_eq!(*rig.closed.lock().unwrap(), 1, "the device was closed");
        // Unlocked, the next action opens a new device.
        rig.lock.send_replace(false);
        rig.dispatcher.handle(press(BACK), Some(&click));
        assert_eq!(*rig.opened.lock().unwrap(), 2);
    }

    #[test]
    fn a_device_that_cannot_be_opened_is_reported_without_a_panic_and_tried_again() {
        let (_lock, locked) = watch::channel(false);
        let attempts = Arc::new(Mutex::new(0));
        let actions = ButtonActions::new(locked, {
            let attempts = Arc::clone(&attempts);
            move || {
                *attempts.lock().unwrap() += 1;
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            }
        });
        let mut dispatcher = ButtonDispatcher::new(actions);
        let click = ButtonAction::Click {
            button: MouseButton::Left,
        };
        dispatcher.handle(press(BACK), Some(&click));
        dispatcher.handle(release(BACK), Some(&click));
        dispatcher.handle(press(BACK), Some(&click));
        assert_eq!(*attempts.lock().unwrap(), 2);
    }

    fn left_click() -> ButtonAction {
        ButtonAction::Click {
            button: MouseButton::Left,
        }
    }

    #[test]
    fn a_key_held_by_two_buttons_is_released_with_the_last_one() {
        let mut rig = rig();
        let click = left_click();
        rig.dispatcher.handle(press(BACK), Some(&click));
        rig.dispatcher
            .handle(press(MouseControlId::Forward), Some(&click));
        rig.dispatcher.handle(release(BACK), Some(&click));
        assert_eq!(rig.recorder.events().last(), Some(&(Key::ButtonLeft, true)));
        rig.dispatcher
            .handle(release(MouseControlId::Forward), Some(&click));
        assert_eq!(
            rig.recorder.events().last(),
            Some(&(Key::ButtonLeft, false))
        );
    }

    #[test]
    fn a_press_without_the_release_before_it_lets_go_of_the_earlier_key() {
        let mut rig = rig();
        rig.dispatcher.handle(press(BACK), Some(&left_click()));
        rig.dispatcher.handle(
            press(BACK),
            Some(&ButtonAction::Click {
                button: MouseButton::Right,
            }),
        );
        assert_eq!(
            rig.recorder.events(),
            [
                (Key::ButtonLeft, true),
                (Key::ButtonLeft, false),
                (Key::ButtonRight, true),
            ]
        );
    }

    #[test]
    fn a_dispatcher_that_ends_lets_go_of_what_it_holds() {
        let mut rig = rig();
        rig.dispatcher.handle(press(BACK), Some(&left_click()));
        drop(rig.dispatcher);
        assert_eq!(
            rig.recorder.events(),
            [(Key::ButtonLeft, true), (Key::ButtonLeft, false)]
        );
    }

    #[test]
    fn a_release_that_fails_closes_the_device_so_that_the_key_comes_up() {
        let mut rig = rig_with(true);
        rig.dispatcher.handle(press(BACK), Some(&left_click()));
        rig.dispatcher.handle(release(BACK), Some(&left_click()));
        assert_eq!(*rig.closed.lock().unwrap(), 1);
        rig.dispatcher.handle(press(BACK), Some(&left_click()));
        assert_eq!(*rig.opened.lock().unwrap(), 2, "reopened when needed");
    }

    #[test]
    fn a_lock_that_comes_while_the_device_opens_stops_the_keys() {
        let recorder = Recorder::default();
        let (lock, locked) = watch::channel(false);
        let actions = ButtonActions::new(locked, {
            let recorder = recorder.clone();
            move || {
                lock.send_replace(true);
                Ok(Box::new(recorder.clone()) as Box<dyn Injector>)
            }
        });
        let mut dispatcher = ButtonDispatcher::new(actions);
        dispatcher.handle(press(BACK), Some(&left_click()));
        dispatcher.handle(press(BACK), Some(&left_click()));
        assert_eq!(recorder.events(), []);
        assert!(dispatcher.held.is_empty());
    }
}
