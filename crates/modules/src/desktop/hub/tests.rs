use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use tokio::sync::oneshot;

use super::*;
use crate::desktop::model::ClassMatch;
use crate::desktop::simulated::{
    DEMO_CLAUDE_CLASS, DEMO_CLAUDE_EXECUTABLE, DEMO_INTELLIJ_CLASS, DEMO_INTELLIJ_EXECUTABLE,
    SimulatedDesktop,
};

/// Long enough for the worker to handle everything pending.
const SETTLE: Duration = Duration::from_millis(100);

const CLAUDE: AppId = AppId("claude");
const INTELLIJ: AppId = AppId("intellij");

fn apps() -> Vec<DesktopApp> {
    vec![
        DesktopApp {
            id: CLAUDE,
            window_class: ClassMatch::Exact(DEMO_CLAUDE_CLASS.into()),
            executable: ExecutableMatch {
                path: DEMO_CLAUDE_EXECUTABLE.into(),
                helper_argument: Some("--type=".into()),
            },
            desktop_entry: Some("com.anthropic.Claude".into()),
        },
        DesktopApp {
            id: INTELLIJ,
            window_class: ClassMatch::Prefix("jetbrains-".into()),
            executable: ExecutableMatch {
                path: "bin/idea".into(),
                helper_argument: None,
            },
            desktop_entry: None,
        },
    ]
}

fn hub_on(desktop: &Arc<SimulatedDesktop>) -> Arc<DesktopHub> {
    DesktopHub::new(Arc::clone(desktop) as _, apps())
}

fn window(id: &str, class: &str) -> Window {
    Window {
        id: WindowId(id.into()),
        class: class.into(),
        caption: id.into(),
        pid: 0,
        demands_attention: false,
        active: false,
        minimized: false,
    }
}

fn window_ids(state: &AppState) -> Vec<String> {
    state
        .windows
        .as_deref()
        .expect("windows followed")
        .iter()
        .map(|window| window.id.0.clone())
        .collect()
}

#[tokio::test(start_paused = true)]
async fn each_application_gets_its_windows_and_processes() {
    let desktop = Arc::new(SimulatedDesktop::demo());
    desktop.set_windows(vec![
        window("claude", DEMO_CLAUDE_CLASS),
        window("idea", DEMO_INTELLIJ_CLASS),
        window("mail", "org.mozilla.Thunderbird"),
    ]);
    let hub = hub_on(&desktop);
    let subscription = hub.subscribe();
    tokio::time::sleep(SETTLE).await;

    let state = subscription.states().borrow().clone();
    let claude = state.app(CLAUDE);
    assert_eq!(window_ids(&claude), ["claude"]);
    assert_eq!(claude.running, Some(true));
    assert_eq!(window_ids(&state.app(INTELLIJ)), ["idea"]);
    assert_eq!(state.app(INTELLIJ).running, Some(true));
    assert_eq!(state.app(AppId("unknown")), AppState::default());
}

#[tokio::test(start_paused = true)]
async fn changes_are_published_as_they_come() {
    let desktop = Arc::new(SimulatedDesktop::empty());
    let hub = hub_on(&desktop);
    let subscription = hub.subscribe();
    let mut states = subscription.states();
    tokio::time::sleep(SETTLE).await;
    assert_eq!(states.borrow_and_update().app(CLAUDE).running, Some(false));

    let mut attention = window("claude", DEMO_CLAUDE_CLASS);
    attention.demands_attention = true;
    desktop.set_windows(vec![attention]);
    tokio::time::sleep(SETTLE).await;
    assert!(states.borrow_and_update().app(CLAUDE).demands_attention());

    desktop.set_running(vec![DEMO_CLAUDE_EXECUTABLE.into()]);
    tokio::time::sleep(PROCESS_PROBE_INTERVAL).await;
    assert_eq!(states.borrow_and_update().app(CLAUDE).running, Some(true));

    // Nothing new: nothing published.
    tokio::time::sleep(PROCESS_PROBE_INTERVAL * 3).await;
    assert!(!states.has_changed().unwrap());
}

#[tokio::test(start_paused = true)]
async fn notifications_are_attributed_to_their_application() {
    let desktop = Arc::new(SimulatedDesktop::empty());
    let hub = hub_on(&desktop);
    let subscription = hub.subscribe();
    let mut notifications = subscription.notifications();
    tokio::time::sleep(SETTLE).await;

    let claude = Path::new(DEMO_CLAUDE_EXECUTABLE);
    let idea = Path::new("/opt/idea/bin/idea");
    desktop.notify(claude, Some("org.kde.konsole"), "another entry", "");
    desktop.notify(
        Path::new("/usr/bin/notify-send"),
        Some("com.anthropic.Claude"),
        "forged",
        "",
    );
    desktop.notify(claude, Some("com.anthropic.Claude"), "Claude asks", "body");
    desktop.notify(idea, None, "Build done", "body");
    tokio::time::sleep(SETTLE).await;

    let first = notifications.try_recv().unwrap();
    assert_eq!((first.app, first.summary.as_str()), (CLAUDE, "Claude asks"));
    let second = notifications.try_recv().unwrap();
    assert_eq!(
        (second.app, second.summary.as_str()),
        (INTELLIJ, "Build done")
    );
    assert!(notifications.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn actions_run_in_the_background_and_report() {
    let desktop = Arc::new(SimulatedDesktop::demo());
    let hub = hub_on(&desktop);
    let subscription = hub.subscribe();
    tokio::time::sleep(SETTLE).await;

    let (activated, outcome) = oneshot::channel();
    assert!(
        subscription.activate(WindowId("claude".into()), move |result| {
            activated.send(result).unwrap();
        })
    );
    outcome.await.unwrap().unwrap();
    assert_eq!(desktop.activations(), [WindowId("claude".into())]);

    let (failed, outcome) = oneshot::channel();
    subscription.activate(WindowId("gone".into()), move |result| {
        failed.send(result).unwrap();
    });
    let error = outcome.await.unwrap().unwrap_err();
    assert!(matches!(error, DesktopError::UnknownWindow(_)), "{error}");
    assert_eq!(
        desktop.activations().len(),
        1,
        "unknown windows never reach the desktop"
    );

    let launch = Launch {
        program: DEMO_INTELLIJ_EXECUTABLE.into(),
        args: vec!["/home/me/project".into()],
    };
    let (launched, outcome) = oneshot::channel();
    drop(subscription);
    let subscription = hub.subscribe();
    subscription.launch(launch.clone(), move |result| {
        launched.send(result).unwrap();
    });
    drop(subscription);
    outcome.await.unwrap().unwrap();
    assert_eq!(desktop.launches(), [launch]);
}

/// A session without compositor nor bus, whose processes cannot be read and
/// whose launches panic.
#[derive(Debug, Default)]
struct BrokenDesktop {
    follow_attempts: AtomicUsize,
}

#[async_trait]
impl DesktopBackend for BrokenDesktop {
    async fn windows(&self, _: &WindowFilter) -> Result<Box<dyn WindowChanges>, DesktopError> {
        self.follow_attempts.fetch_add(1, Ordering::SeqCst);
        Err(DesktopError::NoCompositor("GNOME".into()))
    }
    async fn activate(&self, _: &WindowId) -> Result<(), DesktopError> {
        Ok(())
    }
    async fn running(&self, _: &[ExecutableMatch]) -> Result<Vec<bool>, DesktopError> {
        Err(DesktopError::Processes("no /proc".into()))
    }
    async fn notifications(
        &self,
        _: &NotificationFilter,
    ) -> Result<Box<dyn NotificationStream>, DesktopError> {
        Err(DesktopError::Bus(zbus::Error::Failure("no bus".into())))
    }
    async fn launch(&self, _: &Launch) -> Result<(), DesktopError> {
        panic!("fragile desktop backend");
    }
}

#[tokio::test(start_paused = true)]
async fn a_broken_session_shows_nothing_known_and_is_retried_with_a_backoff() {
    let desktop = Arc::new(BrokenDesktop::default());
    let hub = DesktopHub::new(Arc::clone(&desktop) as _, apps());
    let subscription = hub.subscribe();
    tokio::time::sleep(SETTLE).await;

    let state = subscription.states().borrow().app(CLAUDE);
    assert_eq!(state, AppState::default(), "neither windows nor processes");
    assert_eq!(desktop.follow_attempts.load(Ordering::SeqCst), 1);
    // Retried after 1, 2 and 4 s.
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert_eq!(desktop.follow_attempts.load(Ordering::SeqCst), 4);

    let (activated, outcome) = oneshot::channel();
    subscription.activate(WindowId("any".into()), move |result| {
        activated.send(result).unwrap();
    });
    let error = outcome.await.unwrap().unwrap_err();
    assert!(matches!(error, DesktopError::UnknownWindow(_)), "{error}");

    let (launched, outcome) = oneshot::channel();
    let launch = Launch {
        program: "idea".into(),
        args: Vec::new(),
    };
    subscription.launch(launch, move |result| {
        launched.send(result).unwrap();
    });
    let error = outcome.await.unwrap().unwrap_err();
    assert!(matches!(error, DesktopError::Panicked), "{error}");
}

#[test]
#[should_panic(expected = "unique")]
fn application_identifiers_are_unique() {
    let mut apps = apps();
    apps[1].id = CLAUDE;
    let _hub = DesktopHub::new(Arc::new(SimulatedDesktop::empty()), apps);
}

#[tokio::test(start_paused = true)]
async fn the_worker_stops_with_its_last_subscriber() {
    let desktop = Arc::new(BrokenDesktop::default());
    let hub = DesktopHub::new(Arc::clone(&desktop) as _, apps());
    let first = hub.subscribe();
    let second = hub.subscribe();
    tokio::time::sleep(SETTLE).await;
    drop(first);
    tokio::time::sleep(Duration::from_secs(1) + SETTLE).await;
    assert_eq!(
        desktop.follow_attempts.load(Ordering::SeqCst),
        2,
        "still running"
    );

    drop(second);
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(desktop.follow_attempts.load(Ordering::SeqCst), 2, "stopped");

    let _again = hub.subscribe();
    tokio::time::sleep(SETTLE).await;
    assert_eq!(
        desktop.follow_attempts.load(Ordering::SeqCst),
        3,
        "started again"
    );
}

#[test]
fn without_a_runtime_nothing_is_followed_nor_run() {
    let desktop = Arc::new(SimulatedDesktop::demo());
    let hub = hub_on(&desktop);
    let subscription = hub.subscribe();
    assert!(!subscription.activate(WindowId("claude".into()), |_| {}));
    assert_eq!(
        subscription.states().borrow().clone(),
        DesktopState::default()
    );
}
