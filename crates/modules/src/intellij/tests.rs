use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bergadeck_module_api::TaskScope;
use bergadeck_module_api::testing::TestContext;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::error::TryRecvError;

use super::watch::CATALOG_POLL;
use super::*;
use crate::desktop::{DEMO_INTELLIJ_EXECUTABLE, SimulatedDesktop};

/// Longest (simulated) wait for an event.
const PATIENCE: Duration = Duration::from_secs(30);

const BERGADECK: &str = "project//home/demo/projets/bergadeck";
const BERGACAST: &str = "project//home/demo/projets/bergacast";
const NOTES: &str = "project//home/demo/projets/notes";

/// Instances of one factory, on a simulated desktop and catalog.
struct Setup {
    factory: IntellijFactory,
    desktop: Arc<SimulatedDesktop>,
    catalog: Arc<SimulatedCatalog>,
}

impl Setup {
    fn new(desktop: SimulatedDesktop, catalog: Catalog) -> Self {
        let desktop = Arc::new(desktop);
        let catalog = Arc::new(SimulatedCatalog::new(catalog));
        let hub = DesktopHub::new(Arc::clone(&desktop) as _, vec![desktop_app()]);
        Self {
            factory: IntellijFactory::new(hub, Arc::clone(&catalog) as _),
            desktop,
            catalog,
        }
    }

    fn demo() -> Self {
        Self::new(SimulatedDesktop::demo(), demo_catalog())
    }

    /// An instance, once its first catalog and desktop state are in.
    async fn instance(&self, settings: &Value) -> Harness {
        let TestContext { ctx, events, scope } = TestContext::new(INTELLIJ_KIND);
        let mut harness = Harness {
            module: self.factory.create(ctx, settings).unwrap(),
            events,
            _scope: scope,
        };
        harness.settle().await;
        harness
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

    /// Hands the events until none comes for a poll of the catalog
    /// (simulated time: the simulated desktop and catalog answer at once).
    async fn settle(&mut self) {
        while let Ok(Some(event)) = tokio::time::timeout(CATALOG_POLL, self.events.recv()).await {
            self.module.on_event(event).await;
        }
    }

    /// Hands events until `done` holds.
    async fn until(&mut self, done: impl Fn(&dyn Module) -> bool) {
        while !done(self.module.as_ref()) {
            self.next_event().await;
        }
    }

    fn items(&self) -> Vec<(String, String, KeyState)> {
        self.module
            .page(&MAIN_PAGE.into())
            .unwrap()
            .items
            .into_iter()
            .map(|item| {
                (
                    item.id.0,
                    item.key.label.unwrap_or_default(),
                    item.key.state,
                )
            })
            .collect()
    }

    fn labels(&self) -> Vec<String> {
        self.items()
            .into_iter()
            .map(|(_, label, _)| label)
            .collect()
    }

    fn home_label(&self) -> String {
        self.module.key().label.unwrap_or_default()
    }

    async fn press(&mut self, item: &str) -> Vec<Command> {
        self.module
            .on_item_press(&MAIN_PAGE.into(), &item.into())
            .await
    }
}

fn demo_catalog() -> Catalog {
    SimulatedCatalog::demo().read().unwrap()
}

fn item_feedback(item: &str, feedback: Feedback) -> Command {
    Command::FeedbackOn(
        ModuleSurface::Item {
            page: MAIN_PAGE.into(),
            item: item.into(),
        },
        feedback,
    )
}

fn home_feedback(feedback: Feedback) -> Command {
    Command::FeedbackOn(ModuleSurface::HomeKey, feedback)
}

fn launch(args: &[&str]) -> Launch {
    Launch {
        program: DEMO_INTELLIJ_EXECUTABLE.into(),
        args: args.iter().map(OsString::from).collect(),
    }
}

#[tokio::test(start_paused = true)]
async fn running_intellij_opens_the_projects_page() {
    let mut harness = Setup::demo().instance(&json!({})).await;
    let key = harness.module.key();
    assert_eq!(key.label.as_deref(), Some(MODULE_LABEL));
    assert_eq!(key.state, KeyState::Active);
    assert_eq!(
        harness.module.on_key_press().await,
        [Command::OpenPage(MAIN_PAGE.into())]
    );
    assert_eq!(harness.module.page(&"other".into()), None);
}

#[tokio::test(start_paused = true)]
async fn open_projects_come_first() {
    let harness = Setup::demo().instance(&json!({})).await;
    assert_eq!(
        harness.items(),
        [
            (BERGADECK.into(), "bergadeck".into(), KeyState::Active),
            (BERGACAST.into(), "bergacast".into(), KeyState::Normal),
            (NOTES.into(), "notes".into(), KeyState::Normal),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn an_open_project_is_brought_to_the_front() {
    let setup = Setup::demo();
    let mut harness = setup.instance(&json!({})).await;
    assert_eq!(harness.press(BERGADECK).await, []);
    assert_eq!(
        harness.next_commands().await,
        [item_feedback(BERGADECK, Feedback::Ok)]
    );
    assert_eq!(setup.desktop.activations(), [WindowId("intellij".into())]);
    assert_eq!(setup.desktop.launches(), []);
}

#[tokio::test(start_paused = true)]
async fn a_closed_project_is_opened_once_at_a_time() {
    let setup = Setup::demo();
    let mut harness = setup.instance(&json!({})).await;
    assert_eq!(harness.press(BERGACAST).await, []);
    assert_eq!(harness.labels()[1], OPENING_LABEL);
    assert_eq!(harness.press(BERGACAST).await, [], "ignored while opening");

    assert_eq!(
        harness.next_commands().await,
        [item_feedback(BERGACAST, Feedback::Ok)]
    );
    assert_eq!(
        setup.desktop.launches(),
        [launch(&["/home/demo/projets/bergacast"])]
    );
    assert_eq!(harness.labels()[1], "bergacast");
}

#[tokio::test(start_paused = true)]
async fn a_second_launch_from_another_key_is_ignored() {
    let setup = Setup::demo();
    let mut first = setup.instance(&json!({})).await;
    let mut second = setup.instance(&json!({})).await;
    assert_eq!(first.press(BERGACAST).await, []);
    assert_eq!(second.press(NOTES).await, []);
    assert_eq!(second.labels()[2], "notes", "not opening");
    first.next_commands().await;
    assert_eq!(setup.desktop.launches().len(), 1);

    // Once the first launch ended, another one may start.
    assert_eq!(second.press(NOTES).await, []);
    second.next_commands().await;
    assert_eq!(setup.desktop.launches().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn closed_intellij_is_started_from_the_home_key() {
    let setup = Setup::new(SimulatedDesktop::empty(), demo_catalog());
    let mut harness = setup.instance(&json!({})).await;
    assert_eq!(harness.module.key().state, KeyState::Normal);
    assert_eq!(harness.module.on_key_press().await, []);
    assert_eq!(harness.home_label(), OPENING_LABEL);

    assert_eq!(harness.next_commands().await, [home_feedback(Feedback::Ok)]);
    assert_eq!(setup.desktop.launches(), [launch(&[])]);
    assert_eq!(harness.home_label(), MODULE_LABEL);
}

#[tokio::test(start_paused = true)]
async fn a_failed_start_alerts_on_the_home_key() {
    let setup = Setup::new(SimulatedDesktop::empty(), demo_catalog());
    let mut harness = setup.instance(&json!({})).await;
    setup.desktop.fail_actions(true);
    assert_eq!(harness.module.on_key_press().await, []);
    assert_eq!(
        harness.next_commands().await,
        [home_feedback(Feedback::Alert)]
    );
    assert_eq!(harness.home_label(), MODULE_LABEL);
}

#[tokio::test(start_paused = true)]
async fn a_failed_launch_alerts_and_allows_a_new_one() {
    let setup = Setup::demo();
    let mut harness = setup.instance(&json!({})).await;
    setup.desktop.fail_actions(true);
    assert_eq!(harness.press(BERGACAST).await, []);
    assert_eq!(
        harness.next_commands().await,
        [item_feedback(BERGACAST, Feedback::Alert)]
    );
    assert_eq!(harness.labels()[1], "bergacast");

    setup.desktop.fail_actions(false);
    assert_eq!(harness.press(BERGACAST).await, []);
    assert_eq!(
        harness.next_commands().await,
        [item_feedback(BERGACAST, Feedback::Ok)]
    );
    assert_eq!(setup.desktop.launches().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn activating_a_window_closed_meanwhile_alerts() {
    let setup = Setup::demo();
    let mut harness = setup.instance(&json!({})).await;
    // The hub sees the window gone (simulated time only jumps once every
    // task is idle: the hub has handled the change); the instance not yet.
    setup.desktop.set_windows(Vec::new());
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(harness.items()[0].2, KeyState::Active);

    assert_eq!(harness.press(BERGADECK).await, []);
    assert_eq!(
        harness.next_commands().await,
        [item_feedback(BERGADECK, Feedback::Alert)]
    );
    assert_eq!(setup.desktop.activations(), []);
}

#[tokio::test(start_paused = true)]
async fn without_an_installation_nothing_is_launched() {
    let catalog = Catalog {
        launcher: None,
        projects: demo_catalog().projects,
    };
    let setup = Setup::new(SimulatedDesktop::empty(), catalog);
    let mut harness = setup.instance(&json!({})).await;
    let key = harness.module.key();
    assert_eq!(key.label.as_deref(), Some(UNAVAILABLE_LABEL));
    assert_eq!(key.state, KeyState::Attention);
    assert_eq!(harness.module.on_key_press().await, alert());
    assert_eq!(harness.press(BERGACAST).await, alert());
    assert_eq!(setup.desktop.launches(), []);
}

#[tokio::test(start_paused = true)]
async fn windows_asking_for_attention_show_on_their_keys() {
    let setup = Setup::demo();
    let mut harness = setup.instance(&json!({})).await;
    let mut asking = Window::new("intellij", "jetbrains-idea", "bergadeck – main.rs");
    asking.demands_attention = true;
    setup.desktop.set_windows(vec![asking]);
    harness
        .until(|module| module.key().state == KeyState::Attention)
        .await;
    assert_eq!(harness.items()[0].2, KeyState::Attention);
}

#[tokio::test(start_paused = true)]
async fn windows_of_projects_not_listed_can_be_brought_to_the_front() {
    let setup = Setup::demo();
    let mut harness = setup.instance(&json!({})).await;
    setup.desktop.set_windows(vec![
        Window::new("intellij", "jetbrains-idea", "bergadeck – main.rs"),
        Window::new("neuf", "jetbrains-idea", "nouveau-projet – lib.rs"),
    ]);
    harness
        .until(|module| module.page(&MAIN_PAGE.into()).unwrap().items.len() == 4)
        .await;
    assert_eq!(
        harness.items()[1],
        (
            "window/neuf".into(),
            "nouveau-projet".into(),
            KeyState::Active
        )
    );

    assert_eq!(harness.press("window/neuf").await, []);
    assert_eq!(
        harness.next_commands().await,
        [item_feedback("window/neuf", Feedback::Ok)]
    );
    assert_eq!(setup.desktop.activations(), [WindowId("neuf".into())]);
}

#[tokio::test(start_paused = true)]
async fn projects_with_the_same_name_share_no_window() {
    let project = |folder: &str| Project {
        name: "api".into(),
        path: PathBuf::from(folder),
    };
    let catalog = Catalog {
        launcher: Some(DEMO_INTELLIJ_EXECUTABLE.into()),
        projects: vec![project("/a/api"), project("/b/api")],
    };
    let desktop = SimulatedDesktop::empty();
    desktop.set_windows(vec![Window::new("w", "jetbrains-idea", "api – main.rs")]);
    let harness = Setup::new(desktop, catalog).instance(&json!({})).await;
    assert_eq!(
        harness.items(),
        [
            ("project//a/api".into(), "api".into(), KeyState::Active),
            ("project//b/api".into(), "api".into(), KeyState::Normal),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn unknown_items_alert() {
    let mut harness = Setup::demo().instance(&json!({})).await;
    assert_eq!(harness.press("project//nowhere").await, alert());
    assert_eq!(harness.press("window/gone").await, alert());
    let elsewhere = harness
        .module
        .on_item_press(&"other".into(), &BERGADECK.into())
        .await;
    assert_eq!(elsewhere, []);
}

#[tokio::test(start_paused = true)]
async fn the_page_is_bounded_by_the_settings() {
    let mut harness = Setup::demo()
        .instance(&json!({ MAX_PROJECTS_KEY: 2 }))
        .await;
    assert_eq!(harness.items().len(), 2);
    harness
        .module
        .update_settings(&json!({ MAX_PROJECTS_KEY: 1 }))
        .unwrap();
    assert_eq!(harness.items().len(), 1);

    let invalid = harness
        .module
        .update_settings(&json!({ MAX_PROJECTS_KEY: 0 }));
    assert!(invalid.is_err());
    assert_eq!(harness.items().len(), 1, "the current settings are kept");
}

#[tokio::test(start_paused = true)]
async fn catalog_changes_are_picked_up_at_the_next_poll() {
    let setup = Setup::demo();
    let mut harness = setup.instance(&json!({})).await;
    let mut catalog = demo_catalog();
    catalog.projects.truncate(1);
    setup.catalog.set(Ok(catalog));
    harness
        .until(|module| module.page(&MAIN_PAGE.into()).unwrap().items.len() == 1)
        .await;
}

#[tokio::test(start_paused = true)]
async fn an_unchanged_catalog_sends_no_event() {
    let setup = Setup::demo();
    let mut harness = setup.instance(&json!({})).await;
    let reads = setup.catalog.reads();
    tokio::time::sleep(CATALOG_POLL * 3).await;
    assert!(setup.catalog.reads() > reads, "still polled");
    assert!(matches!(
        harness.events.try_recv(),
        Err(TryRecvError::Empty)
    ));
}

#[tokio::test(start_paused = true)]
async fn an_unreadable_catalog_shows_unavailable_then_recovers() {
    let setup = Setup::new(SimulatedDesktop::empty(), demo_catalog());
    let mut harness = setup.instance(&json!({})).await;
    setup.catalog.set(Err("unreadable".into()));
    harness
        .until(|module| module.key().label.as_deref() == Some(UNAVAILABLE_LABEL))
        .await;
    assert_eq!(harness.items(), []);

    setup.catalog.set(Ok(demo_catalog()));
    harness
        .until(|module| module.key().label.as_deref() == Some(MODULE_LABEL))
        .await;
    assert_eq!(harness.items().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn polling_is_shared_and_stops_with_the_last_instance() {
    let setup = Setup::demo();
    let first = setup.instance(&json!({})).await;
    let second = setup.instance(&json!({})).await;
    let reads = setup.catalog.reads();
    tokio::time::sleep(CATALOG_POLL * 4).await;
    let polled = setup.catalog.reads() - reads;
    assert!((3..=5).contains(&polled), "one poller for both: {polled}");

    drop((first, second));
    tokio::time::sleep(CATALOG_POLL).await;
    let reads = setup.catalog.reads();
    tokio::time::sleep(CATALOG_POLL * 4).await;
    assert_eq!(setup.catalog.reads(), reads, "stopped");
}

#[tokio::test(start_paused = true)]
async fn a_notification_of_intellij_puts_its_key_in_attention_until_pressed() {
    let setup = Setup::demo();
    let mut harness = setup.instance(&json!({})).await;
    let idea = Path::new(DEMO_INTELLIJ_EXECUTABLE);
    setup
        .desktop
        .notify(Path::new("/usr/bin/notify-send"), None);
    harness.settle().await;
    assert_eq!(
        harness.module.key().state,
        KeyState::Active,
        "not IntelliJ's"
    );

    setup.desktop.notify(idea, None);
    harness
        .until(|module| module.key().state == KeyState::Attention)
        .await;
    assert_eq!(harness.home_label(), MODULE_LABEL, "no text shown");
    harness.module.on_key_press().await;
    assert_eq!(harness.module.key().state, KeyState::Active);
}

#[tokio::test(start_paused = true)]
async fn a_notification_while_intellij_is_in_front_is_ignored() {
    let setup = Setup::demo();
    let mut in_front = Window::new("intellij", "jetbrains-idea", "bergadeck – main.rs");
    in_front.active = true;
    setup.desktop.set_windows(vec![in_front]);
    let mut harness = setup.instance(&json!({})).await;
    setup
        .desktop
        .notify(Path::new(DEMO_INTELLIJ_EXECUTABLE), None);
    harness.settle().await;
    assert_eq!(harness.module.key().state, KeyState::Active);
}

#[tokio::test(start_paused = true)]
async fn intellij_in_front_ends_the_attention_of_a_notification() {
    let setup = Setup::demo();
    let mut harness = setup.instance(&json!({})).await;
    // Sent twice by IntelliJ: once is enough.
    for _ in 0..2 {
        setup
            .desktop
            .notify(Path::new(DEMO_INTELLIJ_EXECUTABLE), None);
    }
    harness
        .until(|module| module.key().state == KeyState::Attention)
        .await;
    let mut in_front = Window::new("intellij", "jetbrains-idea", "bergadeck – main.rs");
    in_front.active = true;
    setup.desktop.set_windows(vec![in_front]);
    harness
        .until(|module| module.key().state == KeyState::Active)
        .await;
}

#[test]
fn settings_are_bounded() {
    let factory = IntellijFactory::new(
        DesktopHub::new(Arc::new(SimulatedDesktop::empty()), vec![desktop_app()]),
        Arc::new(SimulatedCatalog::default()),
    );
    assert_eq!(
        factory.default_settings(),
        json!({ MAX_PROJECTS_KEY: DEFAULT_MAX_PROJECTS })
    );
    for refused in [
        json!({ MAX_PROJECTS_KEY: 0 }),
        json!({ MAX_PROJECTS_KEY: 33 }),
        json!({ MAX_PROJECTS_KEY: "8" }),
    ] {
        assert!(factory.validate_settings(&refused).is_err(), "{refused}");
    }
    factory
        .validate_settings(&json!({ MAX_PROJECTS_KEY: 32 }))
        .unwrap();
}

#[test]
fn intellij_is_followed_by_its_windows_and_launcher() {
    let app = desktop_app();
    assert!(app.window_class.matches("jetbrains-idea"));
    assert!(app.window_class.matches("jetbrains-idea-ce"));
    assert!(!app.window_class.matches("jetbrains-rustrover"));
    assert!(
        app.executable
            .matches_executable(&PathBuf::from(DEMO_INTELLIJ_EXECUTABLE))
    );
}
