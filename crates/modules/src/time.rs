//! Monotonic time that keeps counting while the machine sleeps, behind the
//! [`Clock`] trait so that timers can be tested with a simulated clock.
//!
//! Tokio's clock (`CLOCK_MONOTONIC` on Linux) stops during a suspend: a
//! deadline computed with it would be pushed back by the time spent asleep.
//! A pomodoro follows real time instead, so it reads `CLOCK_BOOTTIME`, which
//! is monotonic too (no jump when the wall clock is set or corrected) but
//! includes the suspended time.

use std::fmt;
use std::ops::{Add, AddAssign};
use std::time::Duration;

/// A point in time of a [`Clock`]: time elapsed since an arbitrary origin
/// of that clock. Only instants of the same clock are comparable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClockInstant(Duration);

impl ClockInstant {
    /// Instant `since_origin` after the origin of its clock.
    #[must_use]
    pub const fn from_origin(since_origin: Duration) -> Self {
        Self(since_origin)
    }

    /// Time from `earlier` to `self`, zero if `earlier` is later.
    #[must_use]
    pub fn saturating_duration_since(self, earlier: Self) -> Duration {
        self.0.saturating_sub(earlier.0)
    }
}

impl Add<Duration> for ClockInstant {
    type Output = Self;

    fn add(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(duration))
    }
}

impl AddAssign<Duration> for ClockInstant {
    fn add_assign(&mut self, duration: Duration) {
        *self = *self + duration;
    }
}

/// Source of monotonic instants, counting the time the machine sleeps.
pub trait Clock: Send + Sync + fmt::Debug {
    /// The current instant.
    fn now(&self) -> ClockInstant;
}

/// `CLOCK_BOOTTIME`: time since boot, suspended time included.
#[derive(Debug, Clone, Copy, Default)]
pub struct BootClock;

impl Clock for BootClock {
    fn now(&self) -> ClockInstant {
        let now = rustix::time::clock_gettime(rustix::time::ClockId::Boottime);
        // The kernel never reports a negative time since boot.
        let seconds = u64::try_from(now.tv_sec).unwrap_or_default();
        let nanos = u32::try_from(now.tv_nsec).unwrap_or_default();
        ClockInstant(Duration::new(seconds, nanos))
    }
}

/// Clocks for the tests of the modules.
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Mutex;

    use super::*;

    /// Follows Tokio's clock (paused in tests), plus suspends simulated with
    /// [`SimulatedClock::sleep_machine`]: time Tokio does not see pass.
    #[derive(Debug)]
    pub(crate) struct SimulatedClock {
        origin: tokio::time::Instant,
        asleep: Mutex<Duration>,
    }

    impl SimulatedClock {
        pub(crate) fn new() -> Self {
            Self {
                origin: tokio::time::Instant::now(),
                asleep: Mutex::new(Duration::ZERO),
            }
        }

        /// Simulates a suspend of `duration`: this clock moves forward,
        /// Tokio's timers do not.
        pub(crate) fn sleep_machine(&self, duration: Duration) {
            *self.asleep.lock().unwrap() += duration;
        }
    }

    impl Clock for SimulatedClock {
        fn now(&self) -> ClockInstant {
            ClockInstant(self.origin.elapsed() + *self.asleep.lock().unwrap())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::SimulatedClock;
    use super::*;

    #[test]
    fn boot_time_never_goes_back() {
        let clock = BootClock;
        let first = clock.now();
        let second = clock.now();
        assert!(second >= first);
        assert!(
            first.saturating_duration_since(ClockInstant::from_origin(Duration::ZERO))
                > Duration::ZERO
        );
    }

    #[test]
    fn instants_add_up_and_saturate() {
        let start = ClockInstant::from_origin(Duration::from_secs(5));
        let mut later = start + Duration::from_secs(2);
        later += Duration::from_secs(1);
        assert_eq!(
            later.saturating_duration_since(start),
            Duration::from_secs(3)
        );
        assert_eq!(start.saturating_duration_since(later), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn the_simulated_clock_counts_tokio_time_and_suspends() {
        let clock = SimulatedClock::new();
        let start = clock.now();
        tokio::time::sleep(Duration::from_secs(2)).await;
        clock.sleep_machine(Duration::from_secs(60));
        assert_eq!(
            clock.now().saturating_duration_since(start),
            Duration::from_secs(62)
        );
    }
}
