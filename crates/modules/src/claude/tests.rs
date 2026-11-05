use std::path::{Path, PathBuf};
use std::time::Duration;

use bergadeck_module_api::testing::TestContext;
use bergadeck_module_api::{Feedback, KeyState, TaskScope};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use super::*;
use crate::desktop::{DEMO_CLAUDE_EXECUTABLE, NotificationHints, SimulatedDesktop, Window};

const BRAVE: &str = "/opt/brave-bin/brave";

/// Longest (simulated) wait for an event.
const PATIENCE: Duration = Duration::from_secs(30);

/// Quiet time after which an instance is taken as up to date (simulated
/// time: the simulated desktop answers at once, its processes are probed
/// every few seconds).
const QUIET: Duration = Duration::from_secs(5);

/// Instances of one factory, on a simulated desktop.
struct Setup {
    factory: ClaudeFactory,
    desktop: Arc<SimulatedDesktop>,
}

impl Setup {
    fn new(desktop: SimulatedDesktop) -> Self {
        let desktop = Arc::new(desktop);
        let hub = DesktopHub::new(Arc::clone(&desktop) as _, vec![desktop_app()]);
        Self {
            factory: ClaudeFactory::new(hub),
            desktop,
        }
    }

    fn demo() -> Self {
        Self::new(SimulatedDesktop::demo())
    }

    /// Claude running in the system tray only.
    fn in_tray() -> Self {
        let desktop = SimulatedDesktop::empty();
        desktop.set_running(vec![DEMO_CLAUDE_EXECUTABLE.into()]);
        Self::new(desktop)
    }

    /// An instance, once its first desktop state is in.
    async fn instance(&self) -> Harness {
        let TestContext { ctx, events, scope } = TestContext::new(CLAUDE_KIND);
        let mut harness = Harness {
            module: self.factory.create(ctx, &json!({})).unwrap(),
            events,
            _scope: scope,
        };
        harness.settle().await;
        harness
    }

    fn notify_from(&self, executable: &str) {
        self.notify(executable, Some(APP_ID));
    }

    /// A notification from `executable`, with the `desktop-entry` hint
    /// `entry`.
    fn notify(&self, executable: &str, entry: Option<&str>) {
        self.desktop.notify(Path::new(executable), entry);
    }
}

struct Harness {
    module: Box<dyn Module>,
    events: UnboundedReceiver<ModuleEvent>,
    _scope: TaskScope,
}

impl Harness {
    async fn next_event(&mut self) -> Vec<Command> {
        let event = tokio::time::timeout(PATIENCE, self.events.recv())
            .await
            .expect("an event")
            .expect("the instance is alive");
        self.module.on_event(event).await
    }

    /// The commands of the next event returning some.
    async fn next_commands(&mut self) -> Vec<Command> {
        loop {
            let commands = self.next_event().await;
            if !commands.is_empty() {
                return commands;
            }
        }
    }

    /// Hands the events until none comes for [`QUIET`].
    async fn settle(&mut self) {
        while let Ok(Some(event)) = tokio::time::timeout(QUIET, self.events.recv()).await {
            self.module.on_event(event).await;
        }
    }

    /// Hands events until the key is in `state`.
    async fn until(&mut self, state: KeyState) {
        while self.state() != state {
            self.next_event().await;
        }
    }

    fn state(&self) -> KeyState {
        self.module.key().state
    }

    fn label(&self) -> String {
        self.module.key().label.unwrap_or_default()
    }
}

fn feedback(feedback: Feedback) -> Command {
    Command::FeedbackOn(ModuleSurface::HomeKey, feedback)
}

fn window(id: &str) -> Window {
    Window::new(id, APP_ID, "Claude")
}

#[tokio::test(start_paused = true)]
async fn open_claude_is_brought_to_the_front() {
    let setup = Setup::demo();
    let mut harness = setup.instance().await;
    assert_eq!(harness.state(), KeyState::Active);
    assert_eq!(harness.label(), MODULE_LABEL);

    assert_eq!(harness.module.on_key_press().await, []);
    assert_eq!(harness.next_commands().await, [feedback(Feedback::Ok)]);
    assert_eq!(setup.desktop.activations(), [WindowId("claude".into())]);
    assert_eq!(setup.desktop.launches(), []);
}

#[tokio::test(start_paused = true)]
async fn a_window_asking_for_attention_puts_no_attention() {
    let setup = Setup::demo();
    setup
        .desktop
        .set_windows(vec![window("first"), window("second")]);
    let mut harness = setup.instance().await;
    // Claude Desktop asks whenever it loses the focus.
    let mut asking = window("second");
    asking.demands_attention = true;
    setup.desktop.set_windows(vec![window("first"), asking]);
    harness.settle().await;
    assert_eq!(harness.state(), KeyState::Active);

    harness.module.on_key_press().await;
    harness.next_commands().await;
    assert_eq!(
        setup.desktop.activations(),
        [WindowId("first".into())],
        "the first window, asking or not"
    );
}

#[tokio::test(start_paused = true)]
async fn closed_claude_is_started_once_at_a_time() {
    let setup = Setup::new(SimulatedDesktop::empty());
    let mut harness = setup.instance().await;
    let mut other = setup.instance().await;
    assert_eq!(harness.state(), KeyState::Normal);

    assert_eq!(harness.module.on_key_press().await, []);
    assert_eq!(harness.label(), OPENING_LABEL);
    assert_eq!(other.module.on_key_press().await, [], "ignored meanwhile");
    assert_eq!(other.label(), MODULE_LABEL);

    assert_eq!(harness.next_commands().await, [feedback(Feedback::Ok)]);
    assert_eq!(harness.label(), MODULE_LABEL);
    // Fixed values, never from the configuration.
    assert_eq!(
        setup.desktop.launches(),
        [Launch {
            program: PathBuf::from("gtk-launch"),
            args: vec!["com.anthropic.Claude".into()],
        }]
    );
}

#[tokio::test(start_paused = true)]
async fn a_failed_activation_alerts_without_launching() {
    let setup = Setup::demo();
    let mut harness = setup.instance().await;
    setup.desktop.fail_actions(true);
    harness.module.on_key_press().await;
    assert_eq!(harness.next_commands().await, [feedback(Feedback::Alert)]);
    assert_eq!(setup.desktop.activations(), [WindowId("claude".into())]);
    assert_eq!(setup.desktop.launches(), []);
}

#[tokio::test(start_paused = true)]
async fn without_following_windows_running_claude_is_started_again() {
    let setup = Setup::in_tray();
    setup.desktop.set_compositor(false);
    setup.desktop.set_windows(vec![window("claude")]);
    let mut harness = setup.instance().await;
    assert_eq!(harness.state(), KeyState::Active, "running");

    harness.module.on_key_press().await;
    assert_eq!(harness.next_commands().await, [feedback(Feedback::Ok)]);
    assert_eq!(setup.desktop.launches(), [launch()], "shows its window");
    assert_eq!(setup.desktop.activations(), []);
}

#[tokio::test(start_paused = true)]
async fn claude_in_the_tray_is_started_again() {
    let setup = Setup::in_tray();
    let mut harness = setup.instance().await;
    assert_eq!(harness.state(), KeyState::Active, "running");

    harness.module.on_key_press().await;
    assert_eq!(harness.next_commands().await, [feedback(Feedback::Ok)]);
    assert_eq!(setup.desktop.launches(), [launch()]);
    assert_eq!(setup.desktop.activations(), []);
}

#[tokio::test(start_paused = true)]
async fn a_failed_launch_alerts_and_allows_a_new_one() {
    let setup = Setup::new(SimulatedDesktop::empty());
    let mut harness = setup.instance().await;
    setup.desktop.fail_actions(true);
    harness.module.on_key_press().await;
    assert_eq!(harness.next_commands().await, [feedback(Feedback::Alert)]);
    assert_eq!(harness.label(), MODULE_LABEL);

    setup.desktop.fail_actions(false);
    harness.module.on_key_press().await;
    assert_eq!(harness.next_commands().await, [feedback(Feedback::Ok)]);
    assert_eq!(setup.desktop.launches().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_notification_of_claude_puts_its_key_in_attention_until_pressed() {
    let setup = Setup::demo();
    let mut harness = setup.instance().await;
    setup.notify_from("/usr/bin/notify-send");
    harness.settle().await;
    assert_eq!(harness.state(), KeyState::Active, "not Claude's");

    setup.notify_from(DEMO_CLAUDE_EXECUTABLE);
    harness.until(KeyState::Attention).await;
    assert_eq!(harness.label(), MODULE_LABEL, "no text shown");
    harness.module.on_key_press().await;
    assert_eq!(harness.state(), KeyState::Active);
}

#[tokio::test(start_paused = true)]
async fn notifications_are_claude_s_by_their_sender_whatever_their_hint() {
    let setup = Setup::demo();
    let mut harness = setup.instance().await;
    // Another application's hint rules it out, even from Claude's
    // executable.
    setup.notify(DEMO_CLAUDE_EXECUTABLE, Some("org.kde.konsole"));
    harness.settle().await;
    assert_eq!(harness.state(), KeyState::Active, "another app's hint");

    // Without a hint (as IntelliJ sends them), the sender decides.
    setup.notify(DEMO_CLAUDE_EXECUTABLE, None);
    harness.until(KeyState::Attention).await;
}

/// Hints of a notification of Brave for `origin`.
fn from_brave(origin: Option<&str>) -> NotificationHints {
    NotificationHints {
        desktop_entry: Some("brave-browser".into()),
        origin: origin.map(String::from),
    }
}

#[tokio::test(start_paused = true)]
async fn claude_ai_notifications_in_brave_are_claude_s() {
    let setup = Setup::demo();
    let mut harness = setup.instance().await;
    let notify = |executable: &str, hints| {
        setup.desktop.notify_with(Path::new(executable), hints);
    };
    notify(BRAVE, from_brave(Some("mail.example.com")));
    notify(BRAVE, from_brave(None));
    notify("/usr/bin/notify-send", from_brave(Some("claude.ai")));
    notify(DEMO_CLAUDE_EXECUTABLE, from_brave(Some("claude.ai")));
    harness.settle().await;
    assert_eq!(
        harness.state(),
        KeyState::Active,
        "another site, no site, or not Brave"
    );

    notify("/opt/brave.com/brave/brave", from_brave(Some("claude.ai")));
    harness.until(KeyState::Attention).await;
    // Pressed: Claude Desktop comes to the front, then the user goes back
    // to Brave.
    harness.module.on_key_press().await;
    harness.next_commands().await;
    setup.desktop.set_windows(vec![window("claude")]);
    harness.settle().await;
    assert_eq!(harness.state(), KeyState::Active);
    notify(
        BRAVE,
        NotificationHints {
            desktop_entry: None,
            origin: Some("claude.ai".into()),
        },
    );
    harness.until(KeyState::Attention).await;
}

#[tokio::test(start_paused = true)]
async fn a_notification_while_claude_is_in_front_is_ignored() {
    let setup = Setup::demo();
    let mut in_front = window("claude");
    in_front.active = true;
    setup.desktop.set_windows(vec![in_front]);
    let mut harness = setup.instance().await;
    setup.notify_from(DEMO_CLAUDE_EXECUTABLE);
    harness.settle().await;
    assert_eq!(harness.state(), KeyState::Active, "already looked at");
}

#[tokio::test(start_paused = true)]
async fn claude_in_front_ends_the_attention_of_a_notification() {
    let setup = Setup::demo();
    let mut harness = setup.instance().await;
    setup.notify_from(DEMO_CLAUDE_EXECUTABLE);
    harness.until(KeyState::Attention).await;

    let mut in_front = window("claude");
    in_front.active = true;
    setup.desktop.set_windows(vec![in_front]);
    harness.until(KeyState::Active).await;
}

#[test]
fn claude_is_followed_by_its_window_executable_and_entry() {
    let app = desktop_app();
    assert!(app.window_class.matches(APP_ID));
    assert!(!app.window_class.matches("com.anthropic.Claude.Helper"));
    assert!(
        app.executable
            .matches_executable(Path::new(DEMO_CLAUDE_EXECUTABLE))
    );
    assert!(
        !app.executable
            .matches_executable(Path::new("/usr/lib/other/claude-desktop"))
    );
    let helper = [b"claude-desktop".as_slice(), b"--type=renderer".as_slice()];
    assert!(app.executable.is_helper(helper.into_iter()));
    let main = [b"claude-desktop".as_slice(), b"--startup".as_slice()];
    assert!(!app.executable.is_helper(main.into_iter()));
    assert!(app.may_have_sent(&NotificationHints::entry(Some(APP_ID))));
    assert!(app.may_have_sent(&NotificationHints::entry(None)));
    assert!(!app.may_have_sent(&NotificationHints::entry(Some("org.kde.konsole"))));
}
