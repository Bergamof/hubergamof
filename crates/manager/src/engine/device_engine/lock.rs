//! The device while the computer is locked: nothing it receives triggers an
//! action, and it goes dark after the configured delay. An input wakes it up
//! (and does nothing else); unlocking lights it at once.

use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;

use hubergamof_module_api::ModuleInstanceId;

use super::{DeviceEngine, Navigation, apply_brightness};

/// Brightness of a device gone dark, in percent.
const DARK_BRIGHTNESS: u8 = 0;

/// Where the device stands relative to the lock of the computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// The computer is not locked.
    Unlocked,
    /// Locked, the device lit; it goes dark at `sleep_at` (never if `None`).
    Lit { sleep_at: Option<Instant> },
    /// Locked, the device dark: nothing is drawn on it.
    Dark,
}

/// What the engine knows of the lock of the computer.
pub(super) struct LockState {
    screen: watch::Receiver<bool>,
    phase: Phase,
}

impl LockState {
    pub(super) fn new(screen: watch::Receiver<bool>) -> Self {
        let phase = if *screen.borrow() {
            Phase::Lit { sleep_at: None }
        } else {
            Phase::Unlocked
        };
        Self { screen, phase }
    }

    /// Whether the computer is locked: only what modules declare usable when
    /// locked acts. Read from the source itself, not from what the engine
    /// handled so far: an input queued with the lock must not slip through.
    pub(super) fn is_locked(&self) -> bool {
        *self.screen.borrow()
    }

    /// Whether the device is dark: nothing is drawn on it.
    pub(super) fn is_asleep(&self) -> bool {
        self.phase == Phase::Dark
    }

    /// When the device goes dark.
    pub(super) fn sleep_at(&self) -> Option<Instant> {
        match self.phase {
            Phase::Lit { sleep_at } => sleep_at,
            Phase::Unlocked | Phase::Dark => None,
        }
    }

    /// Waits for the next change of the lock; never returns once the lock
    /// source is gone (the state then stays as it is).
    pub(super) async fn changed(&mut self) {
        if self.screen.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// The device is lit on a locked computer: it goes dark `delay` from
    /// `now`, never without a delay.
    fn light(&mut self, delay: Option<Duration>, now: Instant) {
        self.phase = Phase::Lit {
            sleep_at: delay.map(|delay| now + delay),
        };
    }
}

impl DeviceEngine {
    /// Whether `module` may act now: always on an unlocked computer, on a
    /// locked one only what its module declares usable when locked.
    pub(super) fn may_act(&self, module: &ModuleInstanceId) -> bool {
        !self.lock.is_locked() || self.instances.usable_when_locked(module)
    }

    /// An input on a locked computer: it lights a dark device and does
    /// nothing else (`true`), or restarts the delay before the device goes
    /// dark and goes on to act where the module allows it (`false`).
    pub(super) async fn absorbed_while_locked(&mut self) -> bool {
        if !self.lock.is_locked() {
            return false;
        }
        if self.lock.is_asleep() {
            self.wake_up().await;
            true
        } else {
            self.schedule_sleep();
            false
        }
    }

    /// Plans the extinction of a device starting on a locked computer.
    pub(super) fn schedule_initial_sleep(&mut self) {
        self.schedule_sleep();
    }

    /// The lock of the computer changed.
    pub(super) async fn on_lock_changed(&mut self) {
        let locked = *self.lock.screen.borrow_and_update();
        if locked == (self.lock.phase != Phase::Unlocked) {
            return;
        }
        tracing::info!(serial = %self.serial(), locked, "computer lock changed");
        // A gesture started before the lock must not end in an action.
        self.held = None;
        self.held_dial = None;
        if locked {
            // Pages of the modules refused while locked are not left on display.
            self.nav = Navigation::default();
            self.lock.phase = Phase::Lit { sleep_at: None };
            self.schedule_sleep();
        } else {
            let was_dark = self.lock.is_asleep();
            self.lock.phase = Phase::Unlocked;
            if was_dark {
                self.restore_brightness().await;
            }
        }
    }

    /// The configuration may have changed the delay.
    pub(super) async fn lock_config_changed(&mut self) {
        if self.lock.is_asleep() && self.sleep_delay().is_none() {
            self.wake_up().await;
        } else {
            self.schedule_sleep();
        }
    }

    /// The delay before going dark expired.
    pub(super) async fn fall_asleep(&mut self) {
        self.lock.phase = Phase::Lit { sleep_at: None };
        self.applied_brightness = apply_brightness(self.deck.as_ref(), DARK_BRIGHTNESS).await;
        if self.applied_brightness.is_none() {
            // Still lit: try again at the next input or configuration.
            tracing::warn!(serial = %self.serial(), "cannot turn the device off");
            return;
        }
        self.lock.phase = Phase::Dark;
        tracing::debug!(serial = %self.serial(), "device dark while locked");
    }

    /// Lights the device on a locked computer: it goes dark again after the
    /// delay.
    async fn wake_up(&mut self) {
        self.lock.phase = Phase::Lit { sleep_at: None };
        self.restore_brightness().await;
        self.schedule_sleep();
    }

    async fn restore_brightness(&mut self) {
        self.applied_brightness =
            apply_brightness(self.deck.as_ref(), self.config.brightness).await;
        // The device loses writes right after its brightness changes (as after
        // connecting): the strip is written again in full.
        self.painter.forget_strip();
    }

    /// (Re)starts the delay before a lit, locked device goes dark; a dark or
    /// unlocked one is left as it is.
    fn schedule_sleep(&mut self) {
        match self.lock.phase {
            Phase::Unlocked | Phase::Dark => {}
            Phase::Lit { .. } => self.lock.light(self.sleep_delay(), Instant::now()),
        }
    }

    /// Delay before going dark, `None` when the device stays lit.
    fn sleep_delay(&self) -> Option<Duration> {
        let seconds = self.config.lock_sleep_seconds;
        (seconds > 0).then(|| Duration::from_secs(u64::from(seconds)))
    }
}
