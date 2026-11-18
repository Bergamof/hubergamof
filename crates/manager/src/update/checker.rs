//! Periodic check for a newer release.
//!
//! One background task asks the [`ReleaseSource`] shortly after startup, then
//! once a day, or at once on request; the outcome is published as an
//! [`UpdateStatus`] that the WebSocket sessions and the tray follow. A failed
//! check is logged and published, never fatal: the next one retries.

use std::sync::Arc;
use std::time::Duration;

use hubergamof_protocol::UpdateStatus;
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::source::{ReleaseInfo, ReleaseSource};
use super::version::Version;

/// Delay before the first check: the manager is busy attaching devices.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(60);

/// Time between two automatic checks.
pub const CHECK_INTERVAL: Duration = Duration::from_hours(24);

/// Shortest time between the starts of two checks, whoever asks: the
/// unauthenticated GitHub quota (60 requests an hour per address) is shared
/// with every other tool of the machine.
pub const MIN_CHECK_SPACING: Duration = Duration::from_mins(5);

/// Follows and triggers the check; cheap to clone.
#[derive(Clone)]
pub struct UpdateHandle {
    /// Held by every handle so that followers never see the channel close,
    /// whether or not the checking task still runs.
    sender: Arc<watch::Sender<UpdateStatus>>,
    trigger: Arc<Notify>,
}

impl UpdateHandle {
    /// A handle that never checks and stays [`UpdateStatus::Unknown`]
    /// (tests, tools that need a context but no update check).
    #[must_use]
    pub fn inactive() -> Self {
        Self {
            sender: Arc::new(watch::channel(UpdateStatus::Unknown).0),
            trigger: Arc::new(Notify::new()),
        }
    }

    /// Current status.
    #[must_use]
    pub fn status(&self) -> UpdateStatus {
        self.sender.borrow().clone()
    }

    /// Receiver notified at every status change.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<UpdateStatus> {
        self.sender.subscribe()
    }

    /// Asks for a check now; requests made during a check are merged, and a
    /// check never starts less than [`MIN_CHECK_SPACING`] after the previous one.
    pub fn check_now(&self) {
        self.trigger.notify_one();
    }
}

/// Starts the checking task, first check after `first_delay`; it ends when `shutdown` turns `true`.
#[must_use]
pub fn start(
    source: Arc<dyn ReleaseSource>,
    current: Version,
    first_delay: Duration,
    shutdown: watch::Receiver<bool>,
) -> (UpdateHandle, JoinHandle<()>) {
    let handle = UpdateHandle::inactive();
    let task = tokio::spawn(run(source, current, first_delay, handle.clone(), shutdown));
    (handle, task)
}

async fn run(
    source: Arc<dyn ReleaseSource>,
    current: Version,
    first_delay: Duration,
    handle: UpdateHandle,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut delay = first_delay;
    let mut last_start: Option<Instant> = None;
    loop {
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            () = handle.trigger.notified() => {}
            // A dropped sender also means shutting down.
            _ = shutdown.wait_for(|stopping| *stopping) => return,
        }
        if let Some(last) = last_start {
            tokio::select! {
                () = tokio::time::sleep_until(last + MIN_CHECK_SPACING) => {}
                _ = shutdown.wait_for(|stopping| *stopping) => return,
            }
        }
        delay = CHECK_INTERVAL;
        last_start = Some(Instant::now());
        handle.sender.send_replace(UpdateStatus::Checking);
        let outcome = check(source.as_ref(), current).await;
        handle.sender.send_replace(outcome);
    }
}

async fn check(source: &dyn ReleaseSource, current: Version) -> UpdateStatus {
    match source.latest().await {
        Ok(release) => status_of(&release, current),
        Err(error) => {
            tracing::warn!(%error, "update check failed");
            UpdateStatus::CheckFailed {
                message: error.public_message(),
            }
        }
    }
}

fn status_of(release: &ReleaseInfo, current: Version) -> UpdateStatus {
    if release.version > current {
        UpdateStatus::Available {
            version: release.version.to_string(),
            notes_url: release.notes_url.clone(),
        }
    } else {
        UpdateStatus::UpToDate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::source::SimulatedReleases;

    fn version(text: &str) -> Version {
        text.parse().unwrap()
    }

    fn started(
        source: SimulatedReleases,
        current: &str,
    ) -> (UpdateHandle, watch::Sender<bool>, JoinHandle<()>) {
        let (stop, stopping) = watch::channel(false);
        let (handle, task) = start(
            Arc::new(source),
            version(current),
            FIRST_CHECK_DELAY,
            stopping,
        );
        (handle, stop, task)
    }

    async fn next_settled(handle: &UpdateHandle) -> UpdateStatus {
        let mut receiver = handle.subscribe();
        let settled = receiver
            .wait_for(|status| !matches!(status, UpdateStatus::Unknown | UpdateStatus::Checking))
            .await
            .unwrap();
        settled.clone()
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_known_before_the_first_check() {
        let (handle, _stop, _task) =
            started(SimulatedReleases::announcing(version("2.0.0")), "1.0.0");
        assert_eq!(handle.status(), UpdateStatus::Unknown);
    }

    #[tokio::test(start_paused = true)]
    async fn a_newer_release_is_announced_after_the_first_delay() {
        let (handle, _stop, _task) =
            started(SimulatedReleases::announcing(version("1.1.0")), "1.0.0");
        assert_eq!(
            next_settled(&handle).await,
            UpdateStatus::Available {
                version: "1.1.0".into(),
                notes_url: "https://github.com/Bergamof/hubergamof/releases/latest".into(),
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_same_or_an_older_release_is_up_to_date() {
        for latest in ["1.0.0", "0.9.0"] {
            let (handle, _stop, _task) =
                started(SimulatedReleases::announcing(version(latest)), "1.0.0");
            assert_eq!(next_settled(&handle).await, UpdateStatus::UpToDate);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_failure_is_published_and_retried_at_the_next_check() {
        let (handle, _stop, _task) = started(SimulatedReleases::failing("offline"), "1.0.0");
        assert_eq!(
            next_settled(&handle).await,
            UpdateStatus::CheckFailed {
                message: "offline".into()
            }
        );
        let mut receiver = handle.subscribe();
        receiver.mark_unchanged();
        tokio::time::timeout(CHECK_INTERVAL * 2, receiver.changed())
            .await
            .expect("a retry starts within the interval")
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_checks_without_waiting() {
        let (handle, _stop, _task) =
            started(SimulatedReleases::announcing(version("1.0.1")), "1.0.0");
        handle.check_now();
        let status = tokio::time::timeout(FIRST_CHECK_DELAY / 2, next_settled(&handle))
            .await
            .expect("answered before the first scheduled check");
        assert!(matches!(status, UpdateStatus::Available { .. }));
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_ends_the_task() {
        let (_handle, stop, task) =
            started(SimulatedReleases::announcing(version("1.0.1")), "1.0.0");
        stop.send_replace(true);
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("the task stops")
            .unwrap();
    }

    /// Counts the questions asked.
    struct CountingSource(std::sync::atomic::AtomicUsize);

    #[async_trait::async_trait]
    impl ReleaseSource for CountingSource {
        async fn latest(&self) -> Result<ReleaseInfo, crate::update::UpdateError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(ReleaseInfo {
                version: version("1.0.0"),
                notes_url: String::new(),
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_requests_cannot_exhaust_the_quota() {
        let source = Arc::new(CountingSource(std::sync::atomic::AtomicUsize::new(0)));
        let (stop, stopping) = watch::channel(false);
        let (handle, _task) = start(
            Arc::clone(&source) as Arc<dyn ReleaseSource>,
            version("1.0.0"),
            Duration::ZERO,
            stopping,
        );
        for _ in 0..100 {
            handle.check_now();
            tokio::task::yield_now().await;
        }
        // The first check starts at once, then at most one per spacing.
        tokio::time::sleep(MIN_CHECK_SPACING * 2 + Duration::from_secs(1)).await;
        let asked = source.0.load(std::sync::atomic::Ordering::SeqCst);
        assert!((1..=3).contains(&asked), "{asked} checks in 2 spacings");
        stop.send_replace(true);
    }

    #[test]
    fn an_inactive_handle_stays_unknown() {
        let handle = UpdateHandle::inactive();
        handle.check_now();
        assert_eq!(handle.status(), UpdateStatus::Unknown);
    }
}
