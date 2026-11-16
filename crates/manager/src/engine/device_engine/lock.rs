//! The device while the computer is locked: nothing it receives triggers an
//! action, and it goes dark after the configured delay. An input wakes it up
//! (and does nothing else); unlocking lights it at once. Two things a module
//! can declare lift this: a key whose press acts anyway, and a page that
//! keeps the device lit (and open) for as long as it is displayed.

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
    /// The page displayed keeps the device lit: it does not go dark, and the
    /// lock does not close the page.
    page_keeps_lit: bool,
}

impl LockState {
    pub(super) fn new(screen: watch::Receiver<bool>) -> Self {
        let phase = if *screen.borrow() {
            Phase::Lit { sleep_at: None }
        } else {
            Phase::Unlocked
        };
        Self {
            screen,
            phase,
            page_keeps_lit: false,
        }
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

    /// Whether a press may reach `module`: as `may_act`, or because the page
    /// declares it usable when locked and the module may declare that. The
    /// one rule, shared by the call, the padlock drawn on keys and the
    /// reminders of the dials.
    pub(super) fn may_press(&self, module: &ModuleInstanceId, declared: bool) -> bool {
        self.may_act(module) || (declared && self.instances.lock_safe_pages(module))
    }

    /// Leaves a trace, for audit, of what runs on a locked computer only
    /// because a page declared it.
    pub(super) fn trace_declared_while_locked(&self, module: &ModuleInstanceId, declared: bool) {
        if declared
            && self.lock.is_locked()
            && self.instances.lock_safe_pages(module)
            && !self.instances.usable_when_locked(module)
        {
            tracing::info!(
                serial = %self.serial(),
                %module,
                "action run on a locked computer by page declaration"
            );
        }
    }

    /// While locked, a page of a module that may not act is not left on
    /// display (nor reached again by going back, by the end of a page or by
    /// an event), except one the module keeps lit for the user to act on.
    /// Read from the page itself, not from what was last drawn.
    pub(super) fn close_refused_page(&mut self) {
        if !self.lock.is_locked() {
            return;
        }
        let page = self.displayed_page();
        let refused = page.context.is_some_and(|context| {
            !page.keeps_lit && !self.instances.usable_when_locked(&context.module)
        });
        if refused {
            self.nav = Navigation::default();
        }
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
            self.close_refused_page();
            let keeps_lit = self.displayed_page().keeps_lit;
            self.lock.page_keeps_lit = keeps_lit;
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

    /// The page displayed now keeps the device lit on a locked computer, or
    /// no longer does: the delay before going dark starts again or stops.
    pub(super) fn page_keeps_lit_changed(&mut self, keeps_lit: bool) {
        if self.lock.page_keeps_lit == keeps_lit {
            return;
        }
        self.lock.page_keeps_lit = keeps_lit;
        self.schedule_sleep();
    }

    /// Whether the page displayed while the device is dark has just started
    /// to keep it lit (an incoming call): the device is then lit again.
    pub(super) async fn page_now_keeps_lit(&mut self) -> bool {
        let keeps_lit = self.displayed_page().keeps_lit;
        self.page_keeps_lit_changed(keeps_lit);
        if keeps_lit {
            self.wake_up().await;
        }
        keeps_lit
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
        (seconds > 0 && !self.lock.page_keeps_lit).then(|| Duration::from_secs(u64::from(seconds)))
    }
}
