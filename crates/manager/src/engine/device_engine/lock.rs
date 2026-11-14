//! The device while the computer is locked: nothing it receives triggers an
//! action, and it goes dark after the configured delay. An input wakes it up
//! (and does nothing else); unlocking lights it at once.

use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;

use super::{DeviceEngine, apply_brightness};

/// Brightness of a device gone dark, in percent.
const DARK_BRIGHTNESS: u8 = 0;

/// What the engine knows of the lock of the computer.
pub(super) struct LockState {
    screen: watch::Receiver<bool>,
    locked: bool,
    asleep: bool,
    /// When the device goes dark, `None` unless it is locked and lit.
    sleep_at: Option<Instant>,
}

impl LockState {
    pub(super) fn new(screen: watch::Receiver<bool>) -> Self {
        let locked = *screen.borrow();
        Self {
            screen,
            locked,
            asleep: false,
            sleep_at: None,
        }
    }

    /// Whether the computer is locked: the device triggers no action. Read
    /// from the source itself, not from what the engine handled so far: an
    /// input queued with the lock must not slip through.
    pub(super) fn is_locked(&self) -> bool {
        *self.screen.borrow()
    }

    /// Whether the device is dark: nothing is drawn on it.
    pub(super) fn is_asleep(&self) -> bool {
        self.asleep
    }

    /// When the device goes dark.
    pub(super) fn sleep_at(&self) -> Option<Instant> {
        self.sleep_at
    }

    /// Waits for the next change of the lock; never returns once the lock
    /// source is gone (the state then stays as it is).
    pub(super) async fn changed(&mut self) {
        if self.screen.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

impl DeviceEngine {
    /// Plans the extinction of a device starting on a locked computer.
    pub(super) fn schedule_initial_sleep(&mut self) {
        self.schedule_sleep();
    }

    /// The lock of the computer changed.
    pub(super) async fn on_lock_changed(&mut self) {
        let locked = *self.lock.screen.borrow_and_update();
        if locked == self.lock.locked {
            return;
        }
        self.lock.locked = locked;
        tracing::info!(serial = %self.serial(), locked, "computer lock changed");
        // A gesture started before the lock must not end in an action.
        self.held = None;
        self.held_dial = None;
        if locked {
            self.schedule_sleep();
        } else {
            self.lock.sleep_at = None;
            if self.lock.asleep {
                self.wake_up().await;
            }
        }
    }

    /// An input while locked: it lights the device if dark, and restarts the
    /// delay before it goes dark again.
    pub(super) async fn input_while_locked(&mut self) {
        if self.lock.asleep {
            self.wake_up().await;
        } else {
            self.schedule_sleep();
        }
    }

    /// The configuration may have changed the delay.
    pub(super) async fn lock_config_changed(&mut self) {
        if self.lock.asleep && self.sleep_delay().is_none() {
            self.wake_up().await;
        } else if self.lock.locked && !self.lock.asleep {
            self.schedule_sleep();
        }
    }

    /// The delay before going dark expired.
    pub(super) async fn fall_asleep(&mut self) {
        self.lock.sleep_at = None;
        self.applied_brightness = apply_brightness(self.deck.as_ref(), DARK_BRIGHTNESS).await;
        if self.applied_brightness.is_none() {
            // Still lit: try again at the next input or configuration.
            tracing::warn!(serial = %self.serial(), "cannot turn the device off");
            return;
        }
        self.lock.asleep = true;
        tracing::debug!(serial = %self.serial(), "device dark while locked");
    }

    async fn wake_up(&mut self) {
        self.lock.asleep = false;
        self.applied_brightness =
            apply_brightness(self.deck.as_ref(), self.config.brightness).await;
        // The device loses writes right after its brightness changes (as after
        // connecting): the strip is written again in full.
        self.painter.forget_strip();
        self.schedule_sleep();
    }

    fn schedule_sleep(&mut self) {
        self.lock.sleep_at = match self.sleep_delay() {
            Some(delay) if self.lock.locked && !self.lock.asleep => Some(Instant::now() + delay),
            _ => None,
        };
    }

    /// Delay before going dark, `None` when the device stays lit.
    fn sleep_delay(&self) -> Option<Duration> {
        let seconds = self.config.strip.lock_sleep_seconds;
        (seconds > 0).then(|| Duration::from_secs(u64::from(seconds)))
    }
}
