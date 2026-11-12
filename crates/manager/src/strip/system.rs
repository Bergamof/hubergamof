//! CPU load, memory and swap use of the machine, sampled for the touch strip.

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

/// Number of samples kept for the curves of the strip.
pub const HISTORY_LEN: usize = 14;

const PERCENT: u64 = 100;
/// Where the kernel publishes its figures.
const PROC_ROOT: &str = "/proc";
const STAT_FILE: &str = "stat";
const MEMINFO_FILE: &str = "meminfo";

/// Use of the machine at one instant, in percent of the whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemSample {
    /// Share of the processors busy since the previous sample.
    pub cpu: u8,
    /// Share of the memory in use (what is not available).
    pub ram: u8,
    /// Share of the swap in use, 0 without swap.
    pub swap: u8,
}

/// Source of [`SystemSample`]s; replaced in tests and in the demo.
pub trait SystemProbe: std::fmt::Debug + Send + Sync {
    /// Measures the machine now. Reads files: call it from a blocking
    /// context. `None` when it cannot be measured.
    fn sample(&self) -> Option<SystemSample>;
}

/// The last samples, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemHistory {
    samples: VecDeque<SystemSample>,
}

impl SystemHistory {
    /// Adds `sample`, forgetting the oldest beyond [`HISTORY_LEN`].
    pub fn push(&mut self, sample: SystemSample) {
        if self.samples.len() == HISTORY_LEN {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// The most recent sample.
    #[must_use]
    pub fn latest(&self) -> Option<SystemSample> {
        self.samples.back().copied()
    }

    /// One measure of every sample, oldest first.
    #[must_use]
    pub fn series(&self, measure: impl Fn(&SystemSample) -> u8) -> Vec<u8> {
        self.samples.iter().map(measure).collect()
    }
}

/// Cumulative processor times, in the unit of `/proc/stat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CpuTimes {
    busy: u64,
    total: u64,
}

/// The aggregate `cpu` line of `/proc/stat`: user, nice, system, idle,
/// iowait, irq, softirq and steal (guest times are already in user's).
fn parse_cpu_times(stat: &str) -> Option<CpuTimes> {
    let line = stat.lines().find(|line| line.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let [user, nice, system, idle, iowait, irq, softirq, steal] = fields[..] else {
        return None;
    };
    let idle_time = idle.saturating_add(iowait);
    let total = [user, nice, system, irq, softirq, steal]
        .into_iter()
        .fold(idle_time, u64::saturating_add);
    Some(CpuTimes {
        busy: total - idle_time,
        total,
    })
}

/// Load between two readings; the average since boot without a previous one.
fn cpu_percent(previous: Option<CpuTimes>, now: CpuTimes) -> u8 {
    let (busy, total) = match previous {
        Some(previous) => (
            now.busy.saturating_sub(previous.busy),
            now.total.saturating_sub(previous.total),
        ),
        None => (now.busy, now.total),
    };
    percent_of(busy, total)
}

/// `part` out of `whole` in percent, 0 for an empty whole, at most 100.
fn percent_of(part: u64, whole: u64) -> u8 {
    if whole == 0 {
        return 0;
    }
    let percent = part.saturating_mul(PERCENT) / whole;
    u8::try_from(percent.min(PERCENT)).unwrap_or(u8::MAX)
}

/// Memory and swap use from `/proc/meminfo`, in percent.
fn parse_memory(meminfo: &str) -> Option<(u8, u8)> {
    let kilobytes = |key: &str| -> Option<u64> {
        meminfo
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    };
    let total = kilobytes("MemTotal")?;
    let available = kilobytes("MemAvailable")?;
    let swap_total = kilobytes("SwapTotal")?;
    let swap_free = kilobytes("SwapFree")?;
    Some((
        percent_of(total.saturating_sub(available), total),
        percent_of(swap_total.saturating_sub(swap_free), swap_total),
    ))
}

/// Measures the machine from `/proc`.
#[derive(Debug)]
pub struct ProcProbe {
    root: PathBuf,
    previous: Mutex<Option<CpuTimes>>,
}

impl ProcProbe {
    /// A probe of this machine, whose first sample is the load since boot.
    #[must_use]
    pub fn new() -> Self {
        Self::reading(PROC_ROOT)
    }

    /// A probe reading `stat` and `meminfo` from `root` (tests give it
    /// files they wrote).
    #[must_use]
    pub fn reading(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            previous: Mutex::new(None),
        }
    }
}

impl Default for ProcProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemProbe for ProcProbe {
    fn sample(&self) -> Option<SystemSample> {
        let times = parse_cpu_times(&fs::read_to_string(self.root.join(STAT_FILE)).ok()?)?;
        let (ram, swap) = parse_memory(&fs::read_to_string(self.root.join(MEMINFO_FILE)).ok()?)?;
        let mut previous = self.previous.lock().unwrap_or_else(PoisonError::into_inner);
        let cpu = cpu_percent(*previous, times);
        *previous = Some(times);
        Some(SystemSample { cpu, ram, swap })
    }
}

/// Plausible figures leaving the machine alone (`--demo`, tests).
#[derive(Debug)]
pub struct SimulatedProbe {
    samples: Vec<SystemSample>,
    next: AtomicUsize,
}

impl SimulatedProbe {
    /// A probe always returning `sample`.
    #[must_use]
    pub fn fixed(sample: SystemSample) -> Self {
        Self {
            samples: vec![sample],
            next: AtomicUsize::new(0),
        }
    }

    /// A probe cycling through lively figures, for the demo.
    #[must_use]
    pub fn demo() -> Self {
        let cpu = [22, 30, 26, 40, 55, 38, 30, 28, 44, 62, 48, 36, 30, 34];
        Self {
            samples: cpu
                .into_iter()
                .map(|cpu| SystemSample {
                    cpu,
                    ram: 61,
                    swap: 8,
                })
                .collect(),
            next: AtomicUsize::new(0),
        }
    }
}

impl Default for SimulatedProbe {
    /// The figures of the demo, held still: tests do not depend on when
    /// they sample.
    fn default() -> Self {
        Self::fixed(SystemSample {
            cpu: 34,
            ram: 61,
            swap: 8,
        })
    }
}

impl SystemProbe for SimulatedProbe {
    fn sample(&self) -> Option<SystemSample> {
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.samples.len();
        self.samples.get(index).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "cpu  100 10 50 800 40 0 0 0 0 0\ncpu0 50 5 25 400 20 0 0 0 0 0\nintr 1";

    #[test]
    fn the_cpu_line_gives_busy_and_total_times() {
        let times = parse_cpu_times(STAT).unwrap();
        // Idle is idle + iowait: 840 of 1000.
        assert_eq!((times.busy, times.total), (160, 1000));
        assert_eq!(parse_cpu_times("cpu0 1 2 3 4 5 6 7 8\n"), None);
        assert_eq!(parse_cpu_times("cpu  1 2 3\n"), None);
        assert_eq!(parse_cpu_times("cpu  a b c d e f g h\n"), None);
    }

    #[test]
    fn the_load_is_measured_between_two_readings() {
        let first = CpuTimes {
            busy: 100,
            total: 400,
        };
        let second = CpuTimes {
            busy: 160,
            total: 500,
        };
        assert_eq!(cpu_percent(Some(first), second), 60);
        assert_eq!(cpu_percent(None, second), 32);
        // Nothing elapsed, or counters going back: no load, not a panic.
        assert_eq!(cpu_percent(Some(second), second), 0);
        assert_eq!(cpu_percent(Some(second), first), 0);
    }

    #[test]
    fn memory_and_swap_use_come_from_meminfo() {
        let meminfo = "MemTotal:       16000000 kB\nMemFree: 100 kB\nMemAvailable:    4000000 kB\n\
                       SwapTotal:       2000000 kB\nSwapFree:        1500000 kB\n";
        assert_eq!(parse_memory(meminfo), Some((75, 25)));
        let no_swap = "MemTotal: 100 kB\nMemAvailable: 40 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n";
        assert_eq!(parse_memory(no_swap), Some((60, 0)));
        assert_eq!(parse_memory("MemTotal: 100 kB\n"), None);
    }

    #[test]
    fn percentages_stay_within_the_scale() {
        assert_eq!(percent_of(5, 0), 0);
        assert_eq!(percent_of(300, 200), 100);
        assert_eq!(percent_of(u64::MAX, 1), 100);
    }

    #[test]
    fn the_history_keeps_the_latest_samples() {
        let mut history = SystemHistory::default();
        assert_eq!(history.latest(), None);
        for cpu in 0..20 {
            history.push(SystemSample {
                cpu,
                ram: 1,
                swap: 2,
            });
        }
        let series = history.series(|sample| sample.cpu);
        assert_eq!(series.len(), HISTORY_LEN);
        assert_eq!(series.first(), Some(&6));
        assert_eq!(history.latest().map(|sample| sample.cpu), Some(19));
    }

    #[test]
    fn the_simulated_probes_are_steady_or_cycle() {
        let steady = SimulatedProbe::default();
        assert_eq!(steady.sample(), steady.sample());
        let demo = SimulatedProbe::demo();
        let first: Vec<_> = (0..HISTORY_LEN).filter_map(|_| demo.sample()).collect();
        assert_ne!(first[0], first[1]);
        assert_eq!(demo.sample(), Some(first[0]), "the cycle starts over");
    }

    fn proc_with(stat: &str, meminfo: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(STAT_FILE), stat).unwrap();
        fs::write(dir.path().join(MEMINFO_FILE), meminfo).unwrap();
        dir
    }

    const MEMINFO: &str =
        "MemTotal: 1000 kB\nMemAvailable: 250 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n";

    #[test]
    fn the_proc_probe_measures_between_its_readings() {
        let dir = proc_with("cpu  100 0 100 800 0 0 0 0\n", MEMINFO);
        let probe = ProcProbe::reading(dir.path());
        // First reading: the load since boot, 20 %.
        assert_eq!(
            probe.sample(),
            Some(SystemSample {
                cpu: 20,
                ram: 75,
                swap: 0
            })
        );
        // 100 more ticks, 50 of them busy.
        fs::write(dir.path().join(STAT_FILE), "cpu  125 0 125 850 0 0 0 0\n").unwrap();
        assert_eq!(probe.sample().map(|sample| sample.cpu), Some(50));
    }

    #[test]
    fn the_proc_probe_gives_nothing_for_files_it_cannot_use() {
        let missing = tempfile::tempdir().unwrap();
        assert_eq!(ProcProbe::reading(missing.path()).sample(), None);
        let garbled = proc_with("nonsense", MEMINFO);
        assert_eq!(ProcProbe::reading(garbled.path()).sample(), None);
        let no_memory = proc_with("cpu  1 1 1 1 1 1 1 1\n", "nonsense");
        assert_eq!(ProcProbe::reading(no_memory.path()).sample(), None);
    }

    #[test]
    fn the_proc_probe_runs_on_this_machine() {
        let sample = ProcProbe::new()
            .sample()
            .expect("/proc is readable on Linux");
        assert!(sample.cpu <= 100 && sample.ram <= 100 && sample.swap <= 100);
    }
}
