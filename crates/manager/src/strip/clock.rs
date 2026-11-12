//! Local time and date of the touch strip.

use std::time::Duration;

use jiff::civil::DateTime;

/// French abbreviations of the days of the week, from Monday.
const WEEKDAYS: [&str; 7] = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];

/// French abbreviations of the months, from January.
const MONTHS: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];

const NANOS_PER_SECOND: u64 = 1_000_000_000;
const SECONDS_PER_MINUTE: u64 = 60;

/// Source of the current local time; replaced in tests and in the demo.
pub trait WallClock: std::fmt::Debug + Send + Sync {
    /// Current date and time in the local time zone.
    fn now(&self) -> DateTime;
}

/// Wall clock of the system, in its configured time zone.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl WallClock for SystemClock {
    fn now(&self) -> DateTime {
        jiff::Zoned::now().datetime()
    }
}

/// Wall clock stopped at one instant: reproducible, whatever the system time.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(DateTime);

impl FixedClock {
    /// Clock always reading `at`.
    #[must_use]
    pub const fn at(at: DateTime) -> Self {
        Self(at)
    }
}

impl Default for FixedClock {
    /// A Wednesday at 14:05, mid-minute, so a test straddling real time
    /// never meets a boundary.
    fn default() -> Self {
        Self(jiff::civil::date(2026, 10, 28).at(14, 5, 30, 0))
    }
}

impl WallClock for FixedClock {
    fn now(&self) -> DateTime {
        self.0
    }
}

/// What the clock of the strip shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockReading {
    /// `14:05`.
    pub time: String,
    /// Seconds of the minute, when they are shown.
    pub seconds: Option<u8>,
    /// Abbreviated weekday, day and month: `mer. 28 oct.`.
    pub date: String,
    /// ISO week of the year.
    pub week: u8,
}

impl ClockReading {
    /// The reading at `now`, with its seconds when `show_seconds`.
    #[must_use]
    pub fn at(now: DateTime, show_seconds: bool) -> Self {
        Self {
            time: format!("{:02}:{:02}", now.hour(), now.minute()),
            seconds: show_seconds.then(|| u8::try_from(now.second()).unwrap_or(0)),
            date: format_date(now),
            week: u8::try_from(now.date().iso_week_date().week()).unwrap_or(1),
        }
    }
}

/// `mer. 28 oct.`
fn format_date(now: DateTime) -> String {
    let weekday = usize::try_from(now.weekday().to_monday_zero_offset())
        .ok()
        .and_then(|index| WEEKDAYS.get(index));
    let month = usize::try_from(now.month() - 1)
        .ok()
        .and_then(|index| MONTHS.get(index));
    // Unreachable with a valid civil date: empty texts rather than a panic.
    format!(
        "{} {} {}",
        weekday.copied().unwrap_or_default(),
        now.day(),
        month.copied().unwrap_or_default()
    )
}

/// Time left until the next second (or minute boundary) after `now`: when
/// the reading changes.
#[must_use]
pub fn delay_until_next_change(now: DateTime, show_seconds: bool) -> Duration {
    let subsec = u64::try_from(now.subsec_nanosecond()).unwrap_or(0);
    let (elapsed_nanos, period_nanos) = if show_seconds {
        (subsec, NANOS_PER_SECOND)
    } else {
        let seconds = u64::try_from(now.second()).unwrap_or(0);
        (
            seconds * NANOS_PER_SECOND + subsec,
            SECONDS_PER_MINUTE * NANOS_PER_SECOND,
        )
    };
    Duration::from_nanos(period_nanos - elapsed_nanos.min(period_nanos - 1))
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    #[test]
    fn the_reading_is_in_french_with_the_iso_week() {
        let reading = ClockReading::at(date(2026, 10, 28).at(14, 5, 9, 0), true);
        assert_eq!(reading.time, "14:05");
        assert_eq!(reading.seconds, Some(9));
        assert_eq!(reading.date, "mer. 28 oct.");
        assert_eq!(reading.week, 44);
        assert_eq!(
            ClockReading::at(date(2026, 1, 1).at(0, 0, 0, 0), false).seconds,
            None
        );
    }

    #[test]
    fn the_next_change_is_the_next_second_or_minute() {
        let now = date(2026, 10, 28).at(14, 5, 30, 250_000_000);
        assert_eq!(
            delay_until_next_change(now, true),
            Duration::from_millis(750)
        );
        assert_eq!(
            delay_until_next_change(now, false),
            Duration::from_millis(29_750)
        );
        let boundary = date(2026, 10, 28).at(14, 5, 0, 0);
        assert_eq!(
            delay_until_next_change(boundary, true),
            Duration::from_secs(1)
        );
        assert_eq!(
            delay_until_next_change(boundary, false),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn the_fixed_clock_never_moves() {
        let clock = FixedClock::default();
        assert_eq!(clock.now(), clock.now());
        assert_eq!(ClockReading::at(clock.now(), false).time, "14:05");
    }
}
