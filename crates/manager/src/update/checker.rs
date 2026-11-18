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

use super::source::{ReleaseInfo, ReleaseSource};
use super::version::Version;

/// Delay before the first check: the manager is busy attaching devices.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(60);

/// Time between two automatic checks.
pub const CHECK_INTERVAL: Duration = Duration::from_hours(24);

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

    /// Asks for a check now; requests made during a check are merged.
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
    loop {
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            () = handle.trigger.notified() => {}
            // A dropped sender also means shutting down.
            _ = shutdown.wait_for(|stopping| *stopping) => return,
        }
        delay = CHECK_INTERVAL;
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
                message: error.to_string(),
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
                notes_url: "https://example.org/releases/latest".into(),
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

    #[test]
    fn an_inactive_handle_stays_unknown() {
        let handle = UpdateHandle::inactive();
        handle.check_now();
        assert_eq!(handle.status(), UpdateStatus::Unknown);
    }
}
