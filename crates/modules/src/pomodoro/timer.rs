//! Pure pomodoro state machine: no I/O, no clock of its own. Every operation
//! receives the current monotonic instant, and the time left is computed from
//! a deadline rather than by counting ticks.

use std::time::Duration;

use tokio::time::Instant;

use super::settings::Durations;

/// Step of the pomodoro cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Work session.
    Work,
    /// Break after a pomodoro.
    ShortBreak,
    /// Break after every `long_break_every`-th pomodoro.
    LongBreak,
}

impl Phase {
    /// Whether the step is a break.
    #[must_use]
    pub fn is_break(self) -> bool {
        !matches!(self, Self::Work)
    }
}

/// Whether the time of the current step runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clock {
    /// Stopped with `remaining` left.
    Paused { remaining: Duration },
    /// Running until `deadline`.
    Running { deadline: Instant },
}

/// A step that ended on its own, as reported by [`Timer::tick`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepEnd {
    /// Step that ended.
    pub finished: Phase,
    /// Step now prepared (or started, see [`Self::started`]).
    pub next: Phase,
    /// Number of the pomodoro of the cycle the next step belongs to.
    pub pomodoro: u32,
    /// Length of the next step.
    pub next_duration: Duration,
    /// Whether the next step started at once (`autoStart`).
    pub started: bool,
}

/// State of a pomodoro: current step, number of the pomodoro in the cycle
/// and time left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timer {
    durations: Durations,
    auto_start: bool,
    phase: Phase,
    /// Number (from 1) of the current pomodoro, or of the one a break follows.
    pomodoro: u32,
    clock: Clock,
    /// A step ended and the next one waits for a press.
    awaiting_start: bool,
}

impl Timer {
    /// First pomodoro of a cycle, stopped.
    #[must_use]
    pub fn new(durations: Durations, auto_start: bool) -> Self {
        Self {
            durations,
            auto_start,
            phase: Phase::Work,
            pomodoro: 1,
            clock: Clock::Paused {
                remaining: durations.work,
            },
            awaiting_start: false,
        }
    }

    /// Current step.
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Number (from 1) of the current pomodoro, or of the one a break follows.
    #[must_use]
    pub fn pomodoro(&self) -> u32 {
        self.pomodoro
    }

    /// Number of pomodoros of a cycle.
    #[must_use]
    pub fn cycle_length(&self) -> u32 {
        self.durations.long_break_every
    }

    /// Whether the time runs.
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self.clock, Clock::Running { .. })
    }

    /// Whether a step ended and the next one waits for a press.
    #[must_use]
    pub fn is_awaiting_start(&self) -> bool {
        self.awaiting_start
    }

    /// Length of the current step.
    #[must_use]
    pub fn step_duration(&self) -> Duration {
        self.duration_of(self.phase)
    }

    /// Time left in the current step at `now`.
    #[must_use]
    pub fn remaining(&self, now: Instant) -> Duration {
        match self.clock {
            Clock::Paused { remaining } => remaining,
            Clock::Running { deadline } => deadline.saturating_duration_since(now),
        }
    }

    /// Time spent in the current step at `now`.
    #[must_use]
    pub fn elapsed(&self, now: Instant) -> Duration {
        self.step_duration().saturating_sub(self.remaining(now))
    }

    /// Starts or pauses the time; starting also acknowledges a finished step.
    pub fn toggle(&mut self, now: Instant) {
        let remaining = self.remaining(now);
        self.clock = if self.is_running() {
            Clock::Paused { remaining }
        } else {
            Clock::Running {
                deadline: now + remaining,
            }
        };
        self.awaiting_start = false;
    }

    /// Ends the current step early and prepares the next one, running if
    /// the current one was. Skipping a work session counts it as done.
    pub fn skip(&mut self, now: Instant) {
        let running = self.is_running();
        self.advance(now, running);
        self.awaiting_start = false;
    }

    /// Back to the first pomodoro of a cycle, stopped.
    pub fn reset(&mut self) {
        *self = Self::new(self.durations, self.auto_start);
    }

    /// Ends the current step if its time is up at `now`: the next step is
    /// prepared stopped and awaits a press, or starts at once with
    /// `autoStart`.
    pub fn tick(&mut self, now: Instant) -> Option<StepEnd> {
        let Clock::Running { deadline } = self.clock else {
            return None;
        };
        if now < deadline {
            return None;
        }
        let finished = self.phase;
        self.advance(now, self.auto_start);
        self.awaiting_start = !self.auto_start;
        Some(StepEnd {
            finished,
            next: self.phase,
            pomodoro: self.pomodoro,
            next_duration: self.step_duration(),
            started: self.auto_start,
        })
    }

    /// Applies new lengths and `autoStart` without leaving the current step:
    /// the time already spent in it is kept, so the time left becomes the
    /// new length minus that time (possibly zero: the step then ends at the
    /// next tick once running). The pomodoro number is capped to the new
    /// cycle length.
    pub fn update(&mut self, durations: Durations, auto_start: bool, now: Instant) {
        let elapsed = self.elapsed(now);
        self.durations = durations;
        self.auto_start = auto_start;
        self.pomodoro = self.pomodoro.min(durations.long_break_every);
        let remaining = self.step_duration().saturating_sub(elapsed);
        self.clock = match self.clock {
            Clock::Paused { .. } => Clock::Paused { remaining },
            Clock::Running { .. } => Clock::Running {
                deadline: now + remaining,
            },
        };
    }

    /// Moves to the step after the current one, at its full length.
    fn advance(&mut self, now: Instant, running: bool) {
        let every = self.durations.long_break_every;
        (self.phase, self.pomodoro) = match self.phase {
            Phase::Work if self.pomodoro >= every => (Phase::LongBreak, self.pomodoro),
            Phase::Work => (Phase::ShortBreak, self.pomodoro),
            Phase::ShortBreak => (Phase::Work, (self.pomodoro + 1).min(every)),
            Phase::LongBreak => (Phase::Work, 1),
        };
        let remaining = self.step_duration();
        self.clock = if running {
            Clock::Running {
                deadline: now + remaining,
            }
        } else {
            Clock::Paused { remaining }
        };
    }

    fn duration_of(&self, phase: Phase) -> Duration {
        match phase {
            Phase::Work => self.durations.work,
            Phase::ShortBreak => self.durations.short_break,
            Phase::LongBreak => self.durations.long_break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    fn durations(every: u32) -> Durations {
        Durations {
            work: 25 * MINUTE,
            short_break: 5 * MINUTE,
            long_break: 15 * MINUTE,
            long_break_every: every,
        }
    }

    /// Runs the current step to its end and returns what `tick` reported.
    fn finish_step(timer: &mut Timer, now: &mut Instant) -> StepEnd {
        if !timer.is_running() {
            timer.toggle(*now);
        }
        *now += timer.remaining(*now);
        timer.tick(*now).unwrap()
    }

    #[test]
    fn a_new_timer_is_the_first_pomodoro_stopped() {
        let now = Instant::now();
        let timer = Timer::new(durations(4), false);
        assert_eq!((timer.phase(), timer.pomodoro()), (Phase::Work, 1));
        assert!(!timer.is_running() && !timer.is_awaiting_start());
        assert_eq!(timer.remaining(now), 25 * MINUTE);
        assert_eq!(timer.elapsed(now), Duration::ZERO);
    }

    #[test]
    fn time_left_follows_the_deadline_and_pauses() {
        let mut now = Instant::now();
        let mut timer = Timer::new(durations(4), false);
        timer.toggle(now);
        assert!(timer.is_running());
        now += 10 * MINUTE + Duration::from_millis(500);
        assert_eq!(
            timer.remaining(now),
            (15 * MINUTE).saturating_sub(Duration::from_millis(500))
        );
        timer.toggle(now);
        assert!(!timer.is_running());
        now += 60 * MINUTE;
        assert_eq!(
            timer.remaining(now),
            (15 * MINUTE).saturating_sub(Duration::from_millis(500))
        );
        assert_eq!(timer.tick(now), None);
        timer.toggle(now);
        now += (15 * MINUTE).saturating_sub(Duration::from_millis(501));
        assert_eq!(timer.tick(now), None);
        assert_eq!(timer.remaining(now), Duration::from_millis(1));
    }

    #[test]
    fn a_finished_step_prepares_the_next_one_stopped() {
        let mut now = Instant::now();
        let mut timer = Timer::new(durations(4), false);
        let end = finish_step(&mut timer, &mut now);
        assert_eq!(
            end,
            StepEnd {
                finished: Phase::Work,
                next: Phase::ShortBreak,
                pomodoro: 1,
                next_duration: 5 * MINUTE,
                started: false,
            }
        );
        assert!(!timer.is_running() && timer.is_awaiting_start());
        now += 10 * MINUTE;
        assert_eq!(timer.remaining(now), 5 * MINUTE);
        assert_eq!(timer.tick(now), None, "a step ends only once");

        timer.toggle(now);
        assert!(timer.is_running() && !timer.is_awaiting_start());
    }

    #[test]
    fn a_late_tick_still_ends_the_step_once() {
        let mut now = Instant::now();
        let mut timer = Timer::new(durations(4), false);
        timer.toggle(now);
        now += 3 * 60 * MINUTE;
        assert_eq!(timer.tick(now).map(|end| end.next), Some(Phase::ShortBreak));
        assert_eq!(timer.tick(now), None);
    }

    #[test]
    fn the_nth_pomodoro_is_followed_by_a_long_break() {
        let mut now = Instant::now();
        let mut timer = Timer::new(durations(3), false);
        let phases: Vec<_> = (0..8)
            .map(|_| {
                let end = finish_step(&mut timer, &mut now);
                (end.next, end.pomodoro)
            })
            .collect();
        assert_eq!(
            phases,
            [
                (Phase::ShortBreak, 1),
                (Phase::Work, 2),
                (Phase::ShortBreak, 2),
                (Phase::Work, 3),
                (Phase::LongBreak, 3),
                (Phase::Work, 1),
                (Phase::ShortBreak, 1),
                (Phase::Work, 2),
            ]
        );
    }

    #[test]
    fn a_cycle_of_one_has_only_long_breaks() {
        let mut now = Instant::now();
        let mut timer = Timer::new(durations(1), false);
        assert_eq!(finish_step(&mut timer, &mut now).next, Phase::LongBreak);
        assert_eq!(finish_step(&mut timer, &mut now).pomodoro, 1);
        assert_eq!(finish_step(&mut timer, &mut now).next, Phase::LongBreak);
    }

    #[test]
    fn auto_start_runs_the_next_step_at_once() {
        let mut now = Instant::now();
        let mut timer = Timer::new(durations(4), true);
        let end = finish_step(&mut timer, &mut now);
        assert!(end.started);
        assert!(timer.is_running() && !timer.is_awaiting_start());
        now += MINUTE;
        assert_eq!(timer.remaining(now), 4 * MINUTE);
        now += 4 * MINUTE;
        assert_eq!(timer.tick(now).map(|end| end.next), Some(Phase::Work));
    }

    #[test]
    fn skipping_keeps_the_running_state() {
        let mut now = Instant::now();
        let mut timer = Timer::new(durations(4), false);
        timer.skip(now);
        assert_eq!(timer.phase(), Phase::ShortBreak);
        assert!(!timer.is_running());
        assert_eq!(timer.remaining(now), 5 * MINUTE);

        timer.toggle(now);
        now += MINUTE;
        timer.skip(now);
        assert_eq!((timer.phase(), timer.pomodoro()), (Phase::Work, 2));
        assert!(timer.is_running());
        assert_eq!(timer.remaining(now), 25 * MINUTE);
    }

    #[test]
    fn skipping_acknowledges_a_finished_step() {
        let mut now = Instant::now();
        let mut timer = Timer::new(durations(4), false);
        finish_step(&mut timer, &mut now);
        timer.skip(now);
        assert_eq!(timer.phase(), Phase::Work);
        assert!(!timer.is_awaiting_start());
    }

    #[test]
    fn reset_goes_back_to_the_first_pomodoro_stopped() {
        let mut now = Instant::now();
        let mut timer = Timer::new(durations(4), true);
        for _ in 0..3 {
            finish_step(&mut timer, &mut now);
        }
        assert_eq!((timer.phase(), timer.pomodoro()), (Phase::ShortBreak, 2));
        timer.reset();
        assert_eq!(timer, Timer::new(durations(4), true));
    }

    #[test]
    fn new_lengths_keep_the_time_spent_in_the_step() {
        let mut now = Instant::now();
        let mut timer = Timer::new(durations(4), false);
        timer.toggle(now);
        now += 10 * MINUTE;
        let longer = Durations {
            work: 50 * MINUTE,
            ..durations(4)
        };
        timer.update(longer, false, now);
        assert!(timer.is_running());
        assert_eq!(timer.remaining(now), 40 * MINUTE);

        let shorter = Durations {
            work: 5 * MINUTE,
            ..durations(4)
        };
        timer.update(shorter, true, now);
        assert_eq!(timer.remaining(now), Duration::ZERO);
        let end = timer.tick(now).unwrap();
        assert!(end.started, "autoStart applies from the next step");
    }

    #[test]
    fn new_lengths_apply_to_a_paused_step() {
        let now = Instant::now();
        let mut timer = Timer::new(durations(4), false);
        timer.update(
            Durations {
                work: 30 * MINUTE,
                ..durations(4)
            },
            false,
            now,
        );
        assert!(!timer.is_running());
        assert_eq!(timer.remaining(now), 30 * MINUTE);
    }

    #[test]
    fn a_shorter_cycle_caps_the_pomodoro_number() {
        let mut now = Instant::now();
        let mut timer = Timer::new(durations(4), false);
        for _ in 0..4 {
            finish_step(&mut timer, &mut now);
        }
        assert_eq!((timer.phase(), timer.pomodoro()), (Phase::Work, 3));
        timer.update(durations(2), false, now);
        assert_eq!(timer.pomodoro(), 2);
        assert_eq!(timer.cycle_length(), 2);
        assert_eq!(finish_step(&mut timer, &mut now).next, Phase::LongBreak);
    }
}
