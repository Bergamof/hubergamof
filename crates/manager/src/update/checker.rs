//! The update service: checks for a newer release shortly after startup, then
//! once a day (unless the user switched that off) or on request, and installs it
//! when the user's policy says so (or on request).
//!
//! One background task owns everything; the outcome is published as an
//! [`UpdateStatus`] (and the policy as [`UpdateSettings`]) that the WebSocket
//! sessions and the tray follow. A failed check or installation is logged and
//! published, never fatal: the next check retries.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hubergamof_protocol::{InstallSupport, UpdateSettings, UpdateStatus};
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::installer::{InstallStage, Installer};
use super::policy::{PolicyStore, UpdatePolicy};
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

/// Seconds since the Unix epoch, as the checks are dated.
#[must_use]
pub fn system_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Follows and drives the update service; cheap to clone.
#[derive(Clone)]
pub struct UpdateHandle {
    /// Held by every handle so that followers never see a channel close,
    /// whether or not the service task still runs.
    status: Arc<watch::Sender<UpdateStatus>>,
    settings: Arc<watch::Sender<UpdateSettings>>,
    check_trigger: Arc<Notify>,
    install_trigger: Arc<Notify>,
}

impl UpdateHandle {
    /// A handle without a service behind it: stays [`UpdateStatus::Unknown`]
    /// and cannot install (tests, tools that need a context only).
    #[must_use]
    pub fn inactive() -> Self {
        Self::new(UpdateSettings {
            auto_install: UpdatePolicy::default().auto_install,
            check_daily: UpdatePolicy::default().check_daily,
            install_support: InstallSupport::Unpackaged,
        })
    }

    fn new(settings: UpdateSettings) -> Self {
        Self {
            status: Arc::new(watch::channel(UpdateStatus::Unknown).0),
            settings: Arc::new(watch::channel(settings).0),
            check_trigger: Arc::new(Notify::new()),
            install_trigger: Arc::new(Notify::new()),
        }
    }

    /// Current status.
    #[must_use]
    pub fn status(&self) -> UpdateStatus {
        self.status.borrow().clone()
    }

    /// Receiver notified at every status change.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<UpdateStatus> {
        self.status.subscribe()
    }

    /// Current settings.
    #[must_use]
    pub fn settings(&self) -> UpdateSettings {
        *self.settings.borrow()
    }

    /// Receiver notified at every settings change.
    #[must_use]
    pub fn subscribe_settings(&self) -> watch::Receiver<UpdateSettings> {
        self.settings.subscribe()
    }

    /// Asks for a check now; requests made during a check are merged, and a
    /// check never starts less than [`MIN_CHECK_SPACING`] after the previous one.
    pub fn check_now(&self) {
        self.check_trigger.notify_one();
    }

    /// Asks to install the newer release found by the last check.
    pub fn install_now(&self) {
        self.install_trigger.notify_one();
    }

    /// Chooses whether the manager looks for a newer release once a day (a
    /// check asked for is made all the same); the service saves the choice.
    pub fn set_check_daily(&self, enabled: bool) {
        self.settings.send_if_modified(|settings| {
            let changed = settings.check_daily != enabled;
            settings.check_daily = enabled;
            changed
        });
    }

    /// Chooses whether a newer release is installed as soon as it is found;
    /// the service saves the choice.
    pub fn set_auto_install(&self, enabled: bool) {
        self.settings.send_if_modified(|settings| {
            let changed = settings.auto_install != enabled;
            settings.auto_install = enabled;
            changed
        });
    }
}

/// What the service works with.
pub struct UpdateParts {
    /// Where the latest release comes from.
    pub source: Arc<dyn ReleaseSource>,
    /// How a release is installed.
    pub installer: Arc<dyn Installer>,
    /// Where the user's choice is kept.
    pub policy: Arc<dyn PolicyStore>,
    /// Version of the running manager.
    pub current: Version,
    /// Delay before the first check.
    pub first_delay: Duration,
    /// What time it is, in seconds since the Unix epoch ([`system_seconds`]).
    pub now: fn() -> u64,
}

/// Starts the service; it ends when `shutdown` turns `true`.
#[must_use]
pub fn start(
    parts: UpdateParts,
    shutdown: watch::Receiver<bool>,
) -> (UpdateHandle, JoinHandle<()>) {
    let policy = parts.policy.load();
    let handle = UpdateHandle::new(UpdateSettings {
        auto_install: policy.auto_install,
        check_daily: policy.check_daily,
        install_support: parts.installer.support(),
    });
    let service = Service {
        parts,
        handle: handle.clone(),
        stopping: shutdown,
        found: None,
        failed: None,
        last_check: None,
    };
    (handle, tokio::spawn(service.run()))
}

/// Resolves when the manager shuts down (a dropped sender counts too). Not
/// `wait_for` inline: its guard would be held across the other branches.
async fn wait_for_stop(shutdown: &mut watch::Receiver<bool>) {
    let _ = shutdown.wait_for(|stopping| *stopping).await;
}

struct Service {
    parts: UpdateParts,
    handle: UpdateHandle,
    stopping: watch::Receiver<bool>,
    /// The newer release the last check found.
    found: Option<ReleaseInfo>,
    /// Version whose automatic installation failed: not retried by itself.
    failed: Option<Version>,
    last_check: Option<Instant>,
}

impl Service {
    async fn run(mut self) {
        let mut shutdown = self.stopping.clone();
        let mut settings = self.handle.subscribe_settings();
        settings.mark_unchanged();
        let mut delay = self.parts.first_delay;
        loop {
            // The checks of the clock are the ones the user can switch off; the ones asked for are not.
            let scheduled = self.handle.settings().check_daily;
            tokio::select! {
                () = tokio::time::sleep(delay), if scheduled => self.check().await,
                () = self.handle.check_trigger.notified() => self.check().await,
                () = self.handle.install_trigger.notified() => self.install_on_request().await,
                Ok(()) = settings.changed() => self.on_settings_changed().await,
                // A dropped sender also means shutting down.
                () = wait_for_stop(&mut shutdown) => return,
            }
            // Before any check the first delay applies again, whether the checks were off or just not due.
            delay = if self.last_check.is_none() {
                self.parts.first_delay
            } else {
                CHECK_INTERVAL
            };
            if *shutdown.borrow() {
                return;
            }
        }
    }

    async fn check(&mut self) {
        if let Some(last) = self.last_check {
            let mut shutdown = self.stopping.clone();
            tokio::select! {
                () = tokio::time::sleep_until(last + MIN_CHECK_SPACING) => {}
                () = wait_for_stop(&mut shutdown) => return,
            }
        }
        self.last_check = Some(Instant::now());
        self.handle.status.send_replace(UpdateStatus::Checking);
        let outcome = self.parts.source.latest().await;
        let checked_at = (self.parts.now)();
        match outcome {
            Ok(release) if release.version > self.parts.current => {
                self.handle.status.send_replace(UpdateStatus::Available {
                    version: release.version.to_string(),
                    notes_url: release.notes_url.clone(),
                    checked_at,
                });
                self.found = Some(release);
                self.install_if_automatic().await;
            }
            Ok(_) => {
                self.found = None;
                self.handle
                    .status
                    .send_replace(UpdateStatus::UpToDate { checked_at });
            }
            Err(error) => {
                tracing::warn!(%error, "update check failed");
                self.handle.status.send_replace(UpdateStatus::CheckFailed {
                    message: error.public_message(),
                });
            }
        }
    }

    /// Installs the release just found when the policy says so, once per
    /// version: a failure is shown and waits for the user.
    async fn install_if_automatic(&mut self) {
        let settings = self.handle.settings();
        let Some(release) = self.found.clone() else {
            return;
        };
        if settings.auto_install
            && settings.install_support != InstallSupport::Unpackaged
            && self.failed != Some(release.version)
        {
            self.install(&release).await;
        }
    }

    async fn install_on_request(&mut self) {
        if let Some(release) = self.found.clone() {
            self.install(&release).await;
        }
    }

    /// The policy was changed: save it, and install at once what is waiting
    /// when it now says to.
    async fn on_settings_changed(&mut self) {
        let settings = self.handle.settings();
        let policy = UpdatePolicy {
            auto_install: settings.auto_install,
            check_daily: settings.check_daily,
        };
        let store = Arc::clone(&self.parts.policy);
        if let Err(error) = tokio::task::spawn_blocking(move || store.save(policy)).await {
            tracing::warn!(%error, "cannot save the update policy");
        }
        if matches!(self.handle.status(), UpdateStatus::Available { .. }) {
            self.install_if_automatic().await;
        }
    }

    async fn install(&mut self, release: &ReleaseInfo) {
        let version = release.version.to_string();
        let status = Arc::clone(&self.handle.status);
        let progress = {
            let version = version.clone();
            move |stage: InstallStage| {
                status.send_replace(match stage {
                    InstallStage::Downloading => UpdateStatus::Downloading {
                        version: version.clone(),
                    },
                    InstallStage::Installing => UpdateStatus::Installing {
                        version: version.clone(),
                    },
                });
            }
        };
        match self.parts.installer.install(release, &progress).await {
            Ok(()) => {
                tracing::info!(%version, "update installed");
                self.found = None;
                self.handle
                    .status
                    .send_replace(UpdateStatus::Installed { version });
            }
            Err(error) => {
                tracing::warn!(%error, %version, "update installation failed");
                self.failed = Some(release.version);
                self.handle
                    .status
                    .send_replace(UpdateStatus::InstallFailed {
                        version,
                        message: error.public_message(),
                    });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;

    use super::*;
    use crate::update::installer::{InstallError, SimulatedInstaller, UnsupportedInstaller};
    use crate::update::policy::MemoryPolicyStore;
    use crate::update::source::{SimulatedReleases, UpdateError};

    /// What time the checks of the tests say it is.
    const CHECKED_AT: u64 = 1_700_000_000;

    fn version(text: &str) -> Version {
        text.parse().unwrap()
    }

    fn parts(
        source: Arc<dyn ReleaseSource>,
        installer: Arc<dyn Installer>,
        policy: Arc<dyn PolicyStore>,
    ) -> UpdateParts {
        UpdateParts {
            source,
            installer,
            policy,
            current: version("1.0.0"),
            first_delay: FIRST_CHECK_DELAY,
            now: || CHECKED_AT,
        }
    }

    /// Announces `latest`; installation unsupported, automatic by default.
    fn checking_only(latest: &str) -> UpdateParts {
        parts(
            Arc::new(SimulatedReleases::announcing(version(latest))),
            Arc::new(UnsupportedInstaller),
            Arc::new(MemoryPolicyStore::default()),
        )
    }

    fn started(parts: UpdateParts) -> (UpdateHandle, watch::Sender<bool>, JoinHandle<()>) {
        let (stop, stopping) = watch::channel(false);
        let (handle, task) = start(parts, stopping);
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

    async fn status_where(
        handle: &UpdateHandle,
        wanted: impl Fn(&UpdateStatus) -> bool,
    ) -> UpdateStatus {
        let mut receiver = handle.subscribe();
        let found = receiver.wait_for(wanted).await.unwrap();
        found.clone()
    }

    async fn status_settings(
        handle: &UpdateHandle,
        wanted: impl Fn(&UpdateSettings) -> bool,
    ) -> UpdateSettings {
        let mut receiver = handle.subscribe_settings();
        let found = receiver.wait_for(wanted).await.unwrap();
        *found
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_known_before_the_first_check() {
        let (handle, _stop, _task) = started(checking_only("2.0.0"));
        assert_eq!(handle.status(), UpdateStatus::Unknown);
    }

    #[tokio::test(start_paused = true)]
    async fn a_newer_release_is_announced_after_the_first_delay() {
        let (handle, _stop, _task) = started(checking_only("1.1.0"));
        assert_eq!(
            next_settled(&handle).await,
            UpdateStatus::Available {
                version: "1.1.0".into(),
                notes_url: "https://github.com/Bergamof/hubergamof/releases/latest".into(),
                checked_at: CHECKED_AT,
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_same_or_an_older_release_is_up_to_date() {
        for latest in ["1.0.0", "0.9.0"] {
            let (handle, _stop, _task) = started(checking_only(latest));
            assert_eq!(
                next_settled(&handle).await,
                UpdateStatus::UpToDate {
                    checked_at: CHECKED_AT
                }
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_failure_is_published_and_retried_at_the_next_check() {
        let (handle, _stop, _task) = started(parts(
            Arc::new(SimulatedReleases::failing("offline")),
            Arc::new(UnsupportedInstaller),
            Arc::new(MemoryPolicyStore::default()),
        ));
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
        let (handle, _stop, _task) = started(checking_only("1.0.1"));
        handle.check_now();
        let status = tokio::time::timeout(FIRST_CHECK_DELAY / 2, next_settled(&handle))
            .await
            .expect("answered before the first scheduled check");
        assert!(matches!(status, UpdateStatus::Available { .. }));
    }

    #[tokio::test(start_paused = true)]
    async fn without_the_daily_check_only_a_request_checks() {
        let policy = Arc::new(MemoryPolicyStore::holding(UpdatePolicy {
            auto_install: false,
            check_daily: false,
        }));
        let (handle, _stop, _task) = installing(SimulatedInstaller::succeeding(), policy);
        assert!(!handle.settings().check_daily);
        tokio::time::sleep(CHECK_INTERVAL * 3).await;
        assert_eq!(handle.status(), UpdateStatus::Unknown);
        handle.check_now();
        assert!(matches!(
            next_settled(&handle).await,
            UpdateStatus::Available {
                checked_at: CHECKED_AT,
                ..
            }
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn turning_the_daily_check_off_is_saved_and_stops_the_schedule() {
        let policy = Arc::new(MemoryPolicyStore::default());
        let (handle, _stop, _task) =
            installing(SimulatedInstaller::succeeding(), Arc::clone(&policy));
        assert!(handle.settings().check_daily);
        // The service marks the settings as seen when it starts: let it start first.
        tokio::task::yield_now().await;
        handle.set_check_daily(false);
        let settings = status_settings(&handle, |settings| !settings.check_daily).await;
        assert!(!settings.check_daily);
        tokio::time::sleep(CHECK_INTERVAL * 3).await;
        assert_eq!(handle.status(), UpdateStatus::Unknown);
        tokio::time::timeout(Duration::from_secs(10), async {
            while policy.load().check_daily {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the choice is saved");
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_ends_the_task() {
        let (_handle, stop, task) = started(checking_only("1.0.1"));
        stop.send_replace(true);
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("the task stops")
            .unwrap();
    }

    #[test]
    fn an_inactive_handle_stays_unknown_and_cannot_install() {
        let handle = UpdateHandle::inactive();
        handle.check_now();
        handle.install_now();
        assert_eq!(handle.status(), UpdateStatus::Unknown);
        assert_eq!(
            handle.settings().install_support,
            InstallSupport::Unpackaged
        );
    }

    /// Counts the questions asked.
    struct CountingSource(AtomicUsize);

    #[async_trait]
    impl ReleaseSource for CountingSource {
        async fn latest(&self) -> Result<ReleaseInfo, UpdateError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ReleaseInfo {
                version: version("1.0.0"),
                notes_url: String::new(),
                assets: Vec::new(),
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_requests_cannot_exhaust_the_quota() {
        let source = Arc::new(CountingSource(AtomicUsize::new(0)));
        let mut parts = parts(
            Arc::clone(&source) as Arc<dyn ReleaseSource>,
            Arc::new(UnsupportedInstaller),
            Arc::new(MemoryPolicyStore::default()),
        );
        parts.first_delay = Duration::ZERO;
        let (handle, _stop, _task) = started(parts);
        for _ in 0..100 {
            handle.check_now();
            tokio::task::yield_now().await;
        }
        // The first check starts at once, then at most one per spacing.
        tokio::time::sleep(MIN_CHECK_SPACING * 2 + Duration::from_secs(1)).await;
        let asked = source.0.load(Ordering::SeqCst);
        assert!((1..=3).contains(&asked), "{asked} checks in 2 spacings");
    }

    fn installing(
        installer: SimulatedInstaller,
        policy: Arc<MemoryPolicyStore>,
    ) -> (UpdateHandle, watch::Sender<bool>, JoinHandle<()>) {
        started(parts(
            Arc::new(SimulatedReleases::announcing(version("1.1.0"))),
            Arc::new(installer),
            policy,
        ))
    }

    #[tokio::test(start_paused = true)]
    async fn a_release_is_installed_by_itself_when_the_policy_says_so() {
        let (handle, _stop, _task) = installing(
            SimulatedInstaller::succeeding(),
            Arc::new(MemoryPolicyStore::default()),
        );
        let status = status_where(&handle, |status| {
            matches!(status, UpdateStatus::Installed { .. })
        })
        .await;
        assert_eq!(
            status,
            UpdateStatus::Installed {
                version: "1.1.0".into()
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn without_the_automatic_policy_it_waits_for_a_request() {
        let policy = Arc::new(MemoryPolicyStore::holding(UpdatePolicy {
            auto_install: false,
            ..UpdatePolicy::default()
        }));
        let (handle, _stop, _task) = installing(SimulatedInstaller::succeeding(), policy);
        assert!(matches!(
            next_settled(&handle).await,
            UpdateStatus::Available { .. }
        ));
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(matches!(handle.status(), UpdateStatus::Available { .. }));
        handle.install_now();
        let status = status_where(&handle, |status| {
            matches!(status, UpdateStatus::Installed { .. })
        })
        .await;
        assert_eq!(
            status,
            UpdateStatus::Installed {
                version: "1.1.0".into()
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn turning_the_policy_on_installs_what_is_waiting_and_is_saved() {
        let policy = Arc::new(MemoryPolicyStore::holding(UpdatePolicy {
            auto_install: false,
            ..UpdatePolicy::default()
        }));
        let (handle, _stop, _task) =
            installing(SimulatedInstaller::succeeding(), Arc::clone(&policy));
        assert!(matches!(
            next_settled(&handle).await,
            UpdateStatus::Available { .. }
        ));
        handle.set_auto_install(true);
        status_where(&handle, |status| {
            matches!(status, UpdateStatus::Installed { .. })
        })
        .await;
        assert!(policy.load().auto_install);
        handle.set_auto_install(false);
        let mut settings = handle.subscribe_settings();
        settings
            .wait_for(|settings| !settings.auto_install)
            .await
            .unwrap();
        // `spawn_blocking` runs on a real thread: wait for it to have saved,
        // with simulated time that can run out (a missing save fails the test).
        tokio::time::timeout(Duration::from_secs(5), async {
            while policy.load().auto_install {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the choice is saved");
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_installation_is_shown_and_not_retried_by_itself() {
        let (handle, _stop, _task) = installing(
            SimulatedInstaller::failing("signature mismatch"),
            Arc::new(MemoryPolicyStore::default()),
        );
        let failed = status_where(&handle, |status| {
            matches!(status, UpdateStatus::InstallFailed { .. })
        })
        .await;
        assert_eq!(
            failed,
            UpdateStatus::InstallFailed {
                version: "1.1.0".into(),
                message: "signature mismatch".into()
            }
        );
        // The next daily check finds the same release: it is not tried again.
        handle.check_now();
        tokio::time::sleep(MIN_CHECK_SPACING * 2).await;
        assert!(matches!(
            handle.status(),
            UpdateStatus::Available { .. } | UpdateStatus::InstallFailed { .. }
        ));
        assert!(!matches!(
            handle.status(),
            UpdateStatus::Installing { .. } | UpdateStatus::Downloading { .. }
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn an_installation_that_cannot_happen_says_why() {
        let (handle, _stop, _task) = started(checking_only("1.1.0"));
        assert!(matches!(
            next_settled(&handle).await,
            UpdateStatus::Available { .. }
        ));
        handle.install_now();
        let failed = status_where(&handle, |status| {
            matches!(status, UpdateStatus::InstallFailed { .. })
        })
        .await;
        assert_eq!(
            failed,
            UpdateStatus::InstallFailed {
                version: "1.1.0".into(),
                message: InstallError::Unsupported.public_message()
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_with_nothing_found_does_nothing() {
        let (handle, _stop, _task) = started(checking_only("1.0.0"));
        assert_eq!(
            next_settled(&handle).await,
            UpdateStatus::UpToDate {
                checked_at: CHECKED_AT
            }
        );
        handle.install_now();
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(
            handle.status(),
            UpdateStatus::UpToDate {
                checked_at: CHECKED_AT
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_saved_policy_is_the_starting_setting() {
        let policy = Arc::new(MemoryPolicyStore::holding(UpdatePolicy {
            auto_install: false,
            ..UpdatePolicy::default()
        }));
        let (handle, _stop, _task) = installing(SimulatedInstaller::succeeding(), policy);
        assert!(!handle.settings().auto_install);
        assert_eq!(handle.settings().install_support, InstallSupport::Demo);
    }
}
