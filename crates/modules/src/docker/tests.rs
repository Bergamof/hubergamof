use hubergamof_module_api::testing::TestContext;
use hubergamof_module_api::{ContextDial, ImageLibrary, TaskScope};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use super::pages::{CONTAINER_PREFIX, SERVICE_PREFIX};
use super::render::{container_item, status_label};
use super::*;

/// Longest (simulated) silence after which the instance is considered idle;
/// shorter than [`CONFIRM_TIMEOUT`].
const IDLE: Duration = Duration::from_secs(1);

const SHOP: &str = "project/boutique";
const SHOP_UP: &str = "up/boutique";
const SHOP_DOWN: &str = "down/boutique";
const MONITORING: &str = "project/supervision";
const REDIS: &str = "container/cache-redis";
const MIGRATION: &str = "container/outil-migration";

struct Harness {
    module: Box<dyn Module>,
    events: UnboundedReceiver<ModuleEvent>,
    docker: Arc<SimulatedDocker>,
    _scope: TaskScope,
}

impl Harness {
    /// An instance on the demo host, once it listed the containers.
    async fn start(settings: &Value) -> Self {
        Self::start_with_images(settings, ImageLibrary::empty()).await
    }

    /// The same, with `images` as the stored images of the device.
    async fn start_with_images(settings: &Value, images: ImageLibrary) -> Self {
        let docker = Arc::new(SimulatedDocker::demo());
        let factory = DockerFactory::new(Arc::clone(&docker) as _);
        let TestContext { ctx, events, scope } = TestContext::new(DOCKER_KIND);
        let mut harness = Self {
            module: factory.create(ctx.with_images(images), settings).unwrap(),
            events,
            docker,
            _scope: scope,
        };
        harness.settle().await;
        harness
    }

    /// Hands every event to the instance until none comes for [`IDLE`];
    /// returns the commands of the event handlers.
    async fn settle(&mut self) -> Vec<Command> {
        let mut commands = Vec::new();
        while let Ok(Some(event)) = tokio::time::timeout(IDLE, self.events.recv()).await {
            commands.extend(self.module.on_event(event).await);
        }
        commands
    }

    fn page(&self, page: &str) -> Option<PageContent> {
        self.module.page(&page.into())
    }

    fn items(&self, page: &str) -> Vec<(String, String, KeyState)> {
        self.page(page)
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

    fn item(&self, page: &str, item: &str) -> KeyContent {
        self.page(page)
            .unwrap()
            .items
            .into_iter()
            .find(|entry| entry.id.0 == item)
            .unwrap_or_else(|| panic!("no {item} on {page}"))
            .key
    }

    /// Hands the events already sent to the instance, without waiting.
    async fn drain(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.module.on_event(event).await;
        }
    }

    async fn press(&mut self, page: &str, item: &str) -> Vec<Command> {
        self.module.on_item_press(&page.into(), &item.into()).await
    }

    fn status(&self, name: &str) -> Option<ContainerStatus> {
        self.docker.named(name).map(|container| container.status)
    }
}

fn feedback(page: &str, item: &str, feedback: Feedback) -> Command {
    Command::FeedbackOn(
        ModuleSurface::Item {
            page: page.into(),
            item: item.into(),
        },
        feedback,
    )
}

fn label(key: &KeyContent) -> &str {
    key.label.as_deref().unwrap_or_default()
}

// ---------------------------------------------------------------- Settings

#[test]
fn compose_files_must_be_absolute_paths() {
    let factory = DockerFactory::new(Arc::new(SimulatedDocker::empty()));
    assert_eq!(factory.default_settings(), json!({ "composeFiles": [] }));
    assert!(
        factory
            .validate_settings(&json!({ "composeFiles": ["/srv/a/compose.yaml"] }))
            .is_ok()
    );
    assert!(
        factory
            .validate_settings(&json!({ "composeFiles": ["~/app/compose.yaml"] }))
            .is_ok()
    );
    for invalid in [
        json!({ "composeFiles": ["compose.yaml"] }),
        json!({ "composeFiles": ["~other/compose.yaml"] }),
        json!({ "composeFiles": "/srv/a/compose.yaml" }),
        json!({ "composeFiles": [3] }),
    ] {
        let error = factory.validate_settings(&invalid).unwrap_err();
        assert!(
            matches!(error, ModuleError::InvalidSettings(_)),
            "{invalid}"
        );
    }
    let descriptor = factory.descriptor();
    assert!(descriptor.capabilities.key);
    assert_eq!(descriptor.dial_functions, []);
}

#[test]
fn compose_files_take_a_name_and_an_icon_and_old_settings_still_load() {
    let factory = DockerFactory::new(Arc::new(SimulatedDocker::empty()));
    for valid in [
        json!({ "composeFiles": [{ "path": "/srv/a/compose.yaml" }] }),
        json!({ "composeFiles": [{ "path": "~/a.yaml", "name": "A", "icon": "a.png" }] }),
        json!({ "composeFiles": ["/srv/a/compose.yaml", { "path": "~/b.yaml", "name": null }] }),
    ] {
        assert!(factory.validate_settings(&valid).is_ok(), "{valid}");
    }
    for invalid in [
        json!({ "composeFiles": [{ "name": "sans chemin" }] }),
        json!({ "composeFiles": [{ "path": "a.yaml" }] }),
        json!({ "composeFiles": [{ "path": "/a.yaml", "icon": "../x.png" }] }),
        json!({ "composeFiles": [{ "path": "/a.yaml", "name": "x".repeat(41) }] }),
    ] {
        assert!(factory.validate_settings(&invalid).is_err(), "{invalid}");
    }
}

#[tokio::test(start_paused = true)]
async fn an_instance_refuses_relative_compose_files() {
    let factory = DockerFactory::new(Arc::new(SimulatedDocker::demo()));
    let created = factory.create(
        TestContext::new("d").ctx,
        &json!({ "composeFiles": ["a.yaml"] }),
    );
    assert!(created.is_err());
    let mut harness = Harness::start(&json!({})).await;
    let update = harness
        .module
        .update_settings(&json!({ "composeFiles": ["a.yaml"] }));
    assert!(update.is_err());
}

// ---------------------------------------------------------------- Home and main page

#[tokio::test(start_paused = true)]
async fn the_home_key_counts_the_running_containers_and_opens_the_main_page() {
    let mut harness = Harness::start(&json!({})).await;
    let key = harness.module.key();
    assert_eq!(label(&key), "Docker 4/7");
    assert_eq!(key.state, KeyState::Normal);
    assert_eq!(key.image.as_ref(), Some(&*icons::MODULE));
    assert_eq!(
        harness.module.on_key_press().await,
        [Command::OpenPage(MAIN_PAGE.into())]
    );

    harness.docker.change_externally(|containers| {
        let web = containers
            .iter_mut()
            .find(|c| c.name == "boutique-web-1")
            .unwrap();
        web.health = Some(Health::Unhealthy);
    });
    harness.settle().await;
    assert_eq!(harness.module.key().state, KeyState::Attention);
}

#[tokio::test(start_paused = true)]
async fn the_main_page_shows_projects_then_containers_by_name() {
    let harness = Harness::start(&json!({})).await;
    assert_eq!(
        harness.items(MAIN_PAGE),
        [
            (SHOP.into(), "boutique 3/3".into(), KeyState::Active),
            (
                MONITORING.into(),
                "supervision 0/2".into(),
                KeyState::Normal
            ),
            (REDIS.into(), "cache-redis".into(), KeyState::Active),
            (MIGRATION.into(), "outil-migration".into(), KeyState::Normal),
        ]
    );
    assert_eq!(
        harness.item(MAIN_PAGE, SHOP).image.as_ref(),
        Some(&*icons::PROJECT_RUNNING)
    );
    assert_eq!(
        harness.item(MAIN_PAGE, MONITORING).image.as_ref(),
        Some(&*icons::PROJECT_STOPPED)
    );
    assert_eq!(
        harness.item(MAIN_PAGE, REDIS).image.as_ref(),
        Some(&*icons::RUNNING)
    );
    assert_eq!(
        harness.item(MAIN_PAGE, MIGRATION).image.as_ref(),
        Some(&*icons::STOPPED)
    );
    assert_eq!(harness.page(MAIN_PAGE).unwrap().title, "Docker");
}

#[tokio::test(start_paused = true)]
async fn projects_and_containers_open_their_pages() {
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(
        harness.press(MAIN_PAGE, SHOP).await,
        [Command::OpenPage(SHOP.into())]
    );
    assert_eq!(
        harness.press(MAIN_PAGE, REDIS).await,
        [Command::OpenPage(REDIS.into())]
    );
    assert_eq!(
        harness.press(SHOP, "container/boutique-db-1").await,
        [Command::OpenPage("container/boutique-db-1".into())]
    );
    assert_eq!(
        harness.press(SHOP, "up").await,
        [Command::OpenPage(SHOP_UP.into())]
    );
    assert_eq!(
        harness.press(SHOP, "down").await,
        [Command::OpenPage(SHOP_DOWN.into())]
    );
    assert_eq!(harness.press("elsewhere", "x").await, []);
    assert!(harness.page("elsewhere").is_none());
}

#[tokio::test(start_paused = true)]
async fn an_unreachable_docker_shows_on_the_key_and_empties_the_pages() {
    let mut harness = Harness::start(&json!({})).await;
    harness.docker.set_available(false);
    harness.settle().await;
    let key = harness.module.key();
    assert_eq!(
        (label(&key), key.state),
        ("Docker indisponible", KeyState::Attention)
    );
    let page = harness.page(SHOP).unwrap();
    assert_eq!(
        (page.title.as_str(), page.items.len()),
        ("Docker indisponible", 0)
    );
    assert_eq!(harness.press(REDIS, "stop").await, Command::alert());
}

// ---------------------------------------------------------------- Project page

#[tokio::test(start_paused = true)]
async fn the_project_page_shows_its_actions_then_its_containers() {
    let harness = Harness::start(&json!({})).await;
    let page = harness.page(SHOP).unwrap();
    assert_eq!(page.title, "boutique");
    let items: Vec<_> = harness
        .items(SHOP)
        .into_iter()
        .map(|(id, label, _)| (id, label))
        .collect();
    assert_eq!(
        items,
        [
            ("start-all".into(), "Tout démarrer".into()),
            ("stop-all".into(), "Tout arrêter".into()),
            ("up".into(), "Up".into()),
            ("down".into(), "Down".into()),
            ("container/boutique-api-1".into(), "api-1".into()),
            ("container/boutique-db-1".into(), "db-1".into()),
            ("container/boutique-web-1".into(), "web-1".into()),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn start_all_starts_the_stopped_containers_at_once_and_reports_its_outcome() {
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(harness.press(MONITORING, "start-all").await, []);
    assert_eq!(label(&harness.item(MONITORING, "start-all")), "Démarrage…");
    // A second press while it runs does nothing more.
    assert_eq!(harness.press(MONITORING, "start-all").await, []);

    let commands = harness.settle().await;
    assert_eq!(commands, [feedback(MONITORING, "start-all", Feedback::Ok)]);
    assert_eq!(
        label(&harness.item(MONITORING, "start-all")),
        "Tout démarrer"
    );
    let mut actions = harness.docker.actions();
    actions.sort();
    assert_eq!(
        actions,
        [
            "start supervision-grafana-1",
            "start supervision-prometheus-1"
        ]
    );
    assert_eq!(
        label(&harness.item(MAIN_PAGE, MONITORING)),
        "supervision 2/2"
    );

    // Nothing left to start: done at once.
    assert_eq!(
        harness.press(MONITORING, "start-all").await,
        [feedback(MONITORING, "start-all", Feedback::Ok)]
    );
    assert_eq!(harness.docker.actions().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn stop_all_stops_the_running_containers() {
    let mut harness = Harness::start(&json!({})).await;
    harness.press(SHOP, "stop-all").await;
    harness.settle().await;
    assert_eq!(
        harness.status("boutique-web-1"),
        Some(ContainerStatus::Exited)
    );
    assert_eq!(
        harness.status("boutique-db-1"),
        Some(ContainerStatus::Exited)
    );
    assert_eq!(harness.item(MAIN_PAGE, SHOP).state, KeyState::Normal);
}

// ---------------------------------------------------------------- Up and down

#[tokio::test(start_paused = true)]
async fn up_variants_are_offered_and_run_without_confirmation() {
    let mut harness = Harness::start(&json!({})).await;
    let labels: Vec<_> = harness
        .items(SHOP_UP)
        .into_iter()
        .map(|(_, l, _)| l)
        .collect();
    assert_eq!(labels, ["Up", "Rebuild", "Pull", "No start"]);
    assert_eq!(harness.page(SHOP_UP).unwrap().title, "Up · boutique");
    // Pressing a variant goes back to the project page, where the work shows.
    assert_eq!(
        harness.press(SHOP_UP, "up-build").await,
        [Command::BackTo(SHOP.into())],
        "back to the page of the project, whichever pages lead from it"
    );
    let working = harness.item(SHOP, "container/boutique-web-1");
    assert_eq!(label(&working), "web-1 Build…");
    assert_eq!(working.image, Some(icons::WORKING.clone()));
    assert_eq!(label(&harness.item(MAIN_PAGE, SHOP)), "boutique Build…");
    assert_eq!(harness.settle().await, [feedback(SHOP, "up", Feedback::Ok)]);
    assert_eq!(harness.docker.actions(), ["up boutique --build"]);
    assert_eq!(
        label(&harness.item(SHOP, "container/boutique-web-1")),
        "web-1",
        "the work is over"
    );
}

#[tokio::test(start_paused = true)]
async fn down_needs_a_second_press_and_only_the_confirmation_is_in_attention() {
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(
        harness.items(SHOP_DOWN),
        [
            ("down-keep".into(), "Down".into(), KeyState::Normal),
            (
                "down-volumes".into(),
                "Avec volumes".into(),
                KeyState::Normal
            ),
        ],
        "no permanent attention on the volumes variant"
    );
    assert_eq!(harness.press(SHOP_DOWN, "down-volumes").await, []);
    let confirming = harness.item(SHOP_DOWN, "down-volumes");
    assert_eq!(
        (label(&confirming), confirming.state),
        ("Confirmer ?", KeyState::Attention)
    );
    harness.settle().await;
    assert_eq!(
        harness.docker.actions(),
        [] as [String; 0],
        "not before the second press"
    );

    harness.press(SHOP_DOWN, "down-volumes").await;
    harness.settle().await;
    assert_eq!(harness.docker.actions(), ["down boutique --volumes"]);
    // Taken down without a declared file: the project is gone, its pages too.
    assert!(harness.page(SHOP).is_none());
    assert!(harness.page(SHOP_DOWN).is_none());
    assert_eq!(label(&harness.module.key()), "Docker 1/4");
}

#[tokio::test(start_paused = true)]
async fn a_declared_project_stays_after_down_and_comes_back_up() {
    let settings = json!({ "composeFiles": ["/srv/boutique/compose.yaml"] });
    let mut harness = Harness::start(&settings).await;
    harness.press(SHOP_DOWN, "down-keep").await;
    harness.press(SHOP_DOWN, "down-keep").await;
    harness.settle().await;
    assert_eq!(label(&harness.item(MAIN_PAGE, SHOP)), "boutique 0/3");
    let actions: Vec<_> = harness
        .items(SHOP)
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(
        actions,
        [
            "start-all",
            "stop-all",
            "up",
            "down",
            "service/boutique/api",
            "service/boutique/db",
            "service/boutique/web"
        ],
        "the services of the file, without container since the down"
    );

    harness.press(SHOP_UP, "up-plain").await;
    harness.settle().await;
    assert_eq!(label(&harness.item(MAIN_PAGE, SHOP)), "boutique 3/3");
    assert_eq!(harness.docker.actions(), ["down boutique", "up boutique"]);
}

#[tokio::test(start_paused = true)]
async fn a_declared_file_shows_its_custom_name() {
    let harness = Harness::start(&json!({ "composeFiles": [
        { "path": "/srv/boutique/compose.yaml", "name": "  Ma boutique " },
        { "path": "/home/me/blog/compose.yaml", "name": "   " },
    ] }))
    .await;
    assert_eq!(
        label(&harness.item(MAIN_PAGE, SHOP)),
        "Ma boutique 3/3",
        "the running project, identified by its compose name"
    );
    assert_eq!(label(&harness.item(MAIN_PAGE, "project/blog")), "blog 0/0");
    assert_eq!(harness.page(SHOP).unwrap().title, "Ma boutique");
}

#[tokio::test(start_paused = true)]
async fn a_declared_file_shows_its_icon_once_read() {
    let custom = ImageData::new(b"custom icon".to_vec());
    let library = {
        let custom = custom.clone();
        ImageLibrary::new(move |name| {
            let image = (name == "shop.png").then(|| custom.clone());
            Box::pin(std::future::ready(image))
        })
    };
    let mut harness = Harness::start_with_images(
        &json!({ "composeFiles": [
            { "path": "/srv/boutique/compose.yaml", "icon": "shop.png" },
            { "path": "/home/me/blog/compose.yaml", "icon": "missing.png" },
        ] }),
        library,
    )
    .await;
    assert_eq!(harness.item(MAIN_PAGE, SHOP).image, Some(custom.clone()));
    assert_eq!(
        harness.item(MAIN_PAGE, "project/blog").image,
        Some(icons::PROJECT_STOPPED.clone()),
        "an image that cannot be read leaves the default icon"
    );

    harness
        .module
        .update_settings(&json!({ "composeFiles": ["/srv/boutique/compose.yaml"] }))
        .unwrap();
    harness.settle().await;
    assert_eq!(
        harness.item(MAIN_PAGE, SHOP).image,
        Some(icons::PROJECT_RUNNING.clone()),
        "removing the icon restores the default one"
    );
}

#[tokio::test(start_paused = true)]
async fn declaring_files_regroups_at_once() {
    let mut harness = Harness::start(&json!({})).await;
    harness
        .module
        .update_settings(&json!({ "composeFiles": ["/home/me/blog/compose.yaml"] }))
        .unwrap();
    assert_eq!(label(&harness.item(MAIN_PAGE, "project/blog")), "blog 0/0");
    // Not known to the simulation: up fails, with a ⚠ on the key.
    harness.press("up/blog", "up-plain").await;
    assert_eq!(
        harness.settle().await,
        [feedback("project/blog", "up", Feedback::Alert)]
    );
}

// ---------------------------------------------------------------- Container page

#[tokio::test(start_paused = true)]
async fn a_container_page_offers_removal_only_when_stopped() {
    let mut harness = Harness::start(&json!({})).await;
    let page = harness.page(REDIS).unwrap();
    assert_eq!(page.title, "cache-redis · En marche");
    let ids: Vec<_> = harness
        .items(REDIS)
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(ids, ["start", "stop", "restart"]);
    assert_eq!(
        harness.press(REDIS, "remove").await,
        Command::alert(),
        "not shown, refused"
    );

    harness.press(REDIS, "stop").await;
    harness.settle().await;
    assert_eq!(harness.page(REDIS).unwrap().title, "cache-redis · Arrêté");
    let ids: Vec<_> = harness
        .items(REDIS)
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(ids, ["start", "stop", "restart", "remove"]);
}

#[tokio::test(start_paused = true)]
async fn removal_waits_for_a_second_press_within_the_timeout() {
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(harness.press(MIGRATION, "remove").await, []);
    assert_eq!(label(&harness.item(MIGRATION, "remove")), "Confirmer ?");
    // Too late: the key is back, the next press asks again.
    tokio::time::sleep(CONFIRM_TIMEOUT).await;
    harness.settle().await;
    assert_eq!(label(&harness.item(MIGRATION, "remove")), "Supprimer");
    harness.press(MIGRATION, "remove").await;
    harness.settle().await;
    assert!(harness.status("outil-migration").is_some());

    assert_eq!(
        harness.press(MIGRATION, "remove").await,
        [Command::BackTo(MAIN_PAGE.into())],
        "removing: back to the page of the container's entry"
    );
    assert_eq!(label(&harness.item(MIGRATION, "remove")), "Suppression…");
    assert_eq!(
        label(&harness.item(MAIN_PAGE, MIGRATION)),
        "outil-migration Suppression…"
    );
    harness.settle().await;
    assert!(harness.status("outil-migration").is_none());
    assert!(
        harness.page(MIGRATION).is_none(),
        "gone: the host goes back"
    );
    assert_eq!(harness.docker.actions(), ["remove outil-migration"]);
}

#[tokio::test(start_paused = true)]
async fn another_press_cancels_a_pending_confirmation() {
    let mut harness = Harness::start(&json!({})).await;
    harness.press(MIGRATION, "remove").await;
    harness.press(MIGRATION, "start").await;
    assert_eq!(label(&harness.item(MIGRATION, "start")), "Démarrage…");
    harness.settle().await;
    assert_eq!(
        harness.status("outil-migration"),
        Some(ContainerStatus::Running)
    );
    // Now running: removal is no longer offered nor done.
    assert_eq!(harness.press(MIGRATION, "remove").await, Command::alert());
    assert!(harness.status("outil-migration").is_some());
}

#[tokio::test(start_paused = true)]
async fn a_container_action_goes_back_and_shows_on_the_entry_of_the_container() {
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(
        harness.press(REDIS, "stop").await,
        [Command::BackTo(MAIN_PAGE.into())]
    );
    // Its entry on the main page, where the host went back, says it works.
    let entry = harness.item(MAIN_PAGE, REDIS);
    assert_eq!(label(&entry), "cache-redis Arrêt…");
    assert_eq!(entry.image.as_ref(), Some(&*icons::WORKING));
    // Its page, opened again, too.
    assert_eq!(label(&harness.item(REDIS, "stop")), "Arrêt…");
    assert_eq!(harness.press(REDIS, "stop").await, [], "already running");
    // One action at a time on a container: the others wait for its end.
    assert_eq!(harness.press(REDIS, "restart").await, []);
    assert_eq!(label(&harness.item(REDIS, "restart")), "Redémarrer");

    assert_eq!(
        harness.settle().await,
        [feedback(MAIN_PAGE, REDIS, Feedback::Ok)]
    );
    assert_eq!(label(&harness.item(MAIN_PAGE, REDIS)), "cache-redis");
    assert_eq!(harness.status("cache-redis"), Some(ContainerStatus::Exited));
    assert_eq!(harness.docker.actions(), ["stop cache-redis"]);
}

#[tokio::test(start_paused = true)]
async fn an_action_of_a_compose_container_shows_on_its_project_page() {
    const API: &str = "container/boutique-api-1";
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(
        harness.press(API, "restart").await,
        [Command::BackTo(SHOP.into())]
    );
    assert_eq!(label(&harness.item(SHOP, API)), "api-1 Redémarrage…");
    assert_eq!(harness.settle().await, [feedback(SHOP, API, Feedback::Ok)]);
    assert_eq!(label(&harness.item(SHOP, API)), "api-1");
}

#[tokio::test(start_paused = true)]
async fn the_removal_of_a_compose_container_reports_on_its_project_page() {
    const GRAFANA: &str = "container/supervision-grafana-1";
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(harness.press(GRAFANA, "remove").await, [], "confirm first");
    assert_eq!(
        harness.press(GRAFANA, "remove").await,
        [Command::BackTo(MONITORING.into())]
    );
    // Decided at the start: once removed, the container is listed nowhere.
    assert_eq!(
        harness.settle().await,
        [feedback(MONITORING, GRAFANA, Feedback::Ok)]
    );
    assert!(harness.status("supervision-grafana-1").is_none());
}

#[tokio::test(start_paused = true)]
async fn a_failed_action_of_a_compose_container_alerts_on_its_project_page() {
    const API: &str = "container/boutique-api-1";
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(
        harness.press(API, "restart").await,
        [Command::BackTo(SHOP.into())]
    );
    harness.docker.set_available(false);
    assert_eq!(
        harness.settle().await,
        [feedback(SHOP, API, Feedback::Alert)]
    );
}

#[tokio::test(start_paused = true)]
async fn a_failed_container_action_alerts_on_its_entry() {
    let mut harness = Harness::start(&json!({})).await;
    harness.press(REDIS, "restart").await;
    harness.docker.set_available(false);
    let commands = harness.settle().await;
    assert_eq!(commands, [feedback(MAIN_PAGE, REDIS, Feedback::Alert)]);
    assert_eq!(harness.docker.actions(), [] as [String; 0]);
}

#[tokio::test(start_paused = true)]
async fn a_container_gone_meanwhile_is_not_acted_on() {
    let mut harness = Harness::start(&json!({})).await;
    harness.docker.change_externally(|containers| {
        containers.retain(|c| c.name != "cache-redis");
    });
    harness.settle().await;
    assert!(harness.page(REDIS).is_none());
    assert_eq!(harness.press(REDIS, "stop").await, Command::alert());
}

#[test]
fn statuses_have_french_labels() {
    let mut container = Container {
        id: "1".into(),
        name: "c".into(),
        image: "i".into(),
        status: ContainerStatus::Running,
        health: Some(Health::Unhealthy),
        compose: None,
    };
    assert_eq!(status_label(&container), "Défaillant");
    container.health = Some(Health::Starting);
    assert_eq!(status_label(&container), "Démarrage");
    container.status = ContainerStatus::Restarting;
    assert_eq!(status_label(&container), "Redémarre");
    assert_eq!(
        container_item(&container, "c").key.state,
        KeyState::Attention
    );
}

// ---------------------------------------------------------------- Review follow-ups

#[tokio::test(start_paused = true)]
async fn pressing_another_down_variant_asks_for_its_own_confirmation() {
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(harness.press(SHOP_DOWN, "down-keep").await, []);
    assert_eq!(harness.press(SHOP_DOWN, "down-volumes").await, []);
    assert_eq!(
        label(&harness.item(SHOP_DOWN, "down-volumes")),
        "Confirmer ?"
    );
    assert_eq!(label(&harness.item(SHOP_DOWN, "down-keep")), "Down");
    harness.settle().await;
    assert_eq!(harness.docker.actions(), [] as [String; 0]);
}

#[tokio::test(start_paused = true)]
async fn removing_one_container_then_another_asks_again() {
    let mut harness = Harness::start(&json!({})).await;
    let grafana = "container/supervision-grafana-1";
    harness.press(MIGRATION, "remove").await;
    harness.press(grafana, "remove").await;
    harness.settle().await;
    assert_eq!(harness.docker.actions(), [] as [String; 0]);
    assert!(harness.status("outil-migration").is_some());
    assert!(harness.status("supervision-grafana-1").is_some());
}

#[tokio::test(start_paused = true)]
async fn every_up_variant_reaches_compose() {
    let mut harness = Harness::start(&json!({})).await;
    for item in ["up-plain", "up-build", "up-pull", "up-no-start"] {
        harness.press(SHOP_UP, item).await;
        harness.settle().await;
    }
    assert_eq!(
        harness.docker.actions(),
        [
            "up boutique",
            "up boutique --build",
            "up boutique --pull",
            "up boutique --no-start"
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn up_without_start_creates_the_missing_containers_and_starts_nothing() {
    let mut harness =
        Harness::start(&json!({ "composeFiles": ["/srv/boutique/compose.yaml"] })).await;
    harness.press(SHOP_DOWN, "down-keep").await;
    harness.press(SHOP_DOWN, "down-keep").await;
    harness.settle().await;
    assert!(harness.status("boutique-web-1").is_none());

    harness.press(SHOP_UP, "up-no-start").await;
    harness.settle().await;
    assert_eq!(
        harness.status("boutique-web-1"),
        Some(ContainerStatus::Created),
        "created, not running"
    );
    assert_eq!(label(&harness.item(MAIN_PAGE, SHOP)), "boutique 0/3");
}

#[tokio::test(start_paused = true)]
async fn an_older_expiry_leaves_a_newer_confirmation() {
    let mut harness = Harness::start(&json!({})).await;
    harness.press(SHOP_DOWN, "down-keep").await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    harness.press(SHOP_DOWN, "down-volumes").await;
    // The expiry of the first press comes at 3 s, the second press at 3.5 s.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    harness.drain().await;
    assert_eq!(
        label(&harness.item(SHOP_DOWN, "down-volumes")),
        "Confirmer ?"
    );
    harness.press(SHOP_DOWN, "down-volumes").await;
    harness.settle().await;
    assert_eq!(harness.docker.actions(), ["down boutique --volumes"]);
}

#[tokio::test(start_paused = true)]
async fn a_compose_file_in_the_home_directory_is_offered() {
    let harness = Harness::start(&json!({ "composeFiles": ["~/blog/compose.yaml"] })).await;
    assert_eq!(label(&harness.item(MAIN_PAGE, "project/blog")), "blog 0/0");
}

#[tokio::test(start_paused = true)]
async fn actions_of_two_keys_run_at_once_each_showing_its_progress() {
    let mut harness = Harness::start(&json!({})).await;
    harness.docker.set_latency(Duration::from_secs(2));
    harness.press(REDIS, "stop").await;
    harness.press(MIGRATION, "start").await;
    assert_eq!(label(&harness.item(MAIN_PAGE, REDIS)), "cache-redis Arrêt…");
    assert_eq!(
        label(&harness.item(MAIN_PAGE, MIGRATION)),
        "outil-migration Démarrage…"
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut commands = harness.settle().await;
    commands.sort_by_key(|command| format!("{command:?}"));
    assert_eq!(
        commands,
        [
            feedback(MAIN_PAGE, REDIS, Feedback::Ok),
            feedback(MAIN_PAGE, MIGRATION, Feedback::Ok),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn an_action_goes_on_once_its_instance_is_gone() {
    let Harness {
        mut module,
        docker,
        _scope: scope,
        events,
    } = Harness::start(&json!({})).await;
    docker.set_latency(Duration::from_secs(5));
    module
        .on_item_press(&SHOP_DOWN.into(), &"down-keep".into())
        .await;
    module
        .on_item_press(&SHOP_DOWN.into(), &"down-keep".into())
        .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    drop((module, scope, events));
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(docker.actions(), ["down boutique"]);
}

#[tokio::test(start_paused = true)]
async fn compose_is_refused_when_the_containers_disagree_on_their_files() {
    let mut harness = Harness::start(&json!({})).await;
    harness.docker.change_externally(|containers| {
        let db = containers
            .iter_mut()
            .find(|c| c.name == "boutique-db-1")
            .unwrap();
        db.compose = Some(ComposeLabels::new(
            "boutique".into(),
            Some("/tmp/evil".into()),
            vec!["/tmp/evil/compose.yaml".into()],
        ));
    });
    harness.settle().await;
    assert_eq!(harness.press(SHOP_UP, "up-plain").await, Command::alert());
    harness.press(SHOP_DOWN, "down-keep").await;
    assert_eq!(
        harness.press(SHOP_DOWN, "down-keep").await,
        Command::alert()
    );
    // Its containers are still controlled one by one.
    harness.press(SHOP, "stop-all").await;
    harness.settle().await;
    assert_eq!(
        harness.status("boutique-db-1"),
        Some(ContainerStatus::Exited)
    );
    assert_eq!(harness.docker.actions().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn plain_path_settings_show_their_projects_without_name_or_icon() {
    let harness = Harness::start(&json!({ "composeFiles": [
        "/srv/boutique/compose.yaml",
        { "path": "~/blog/compose.yaml", "name": "   " },
    ] }))
    .await;
    assert_eq!(label(&harness.item(MAIN_PAGE, SHOP)), "boutique 3/3");
    assert_eq!(
        harness.item(MAIN_PAGE, SHOP).image,
        Some(icons::PROJECT_RUNNING.clone())
    );
    assert_eq!(label(&harness.item(MAIN_PAGE, "project/blog")), "blog 0/0");
}

/// An image library whose reads wait until `release` is notified.
fn held_library(release: Arc<tokio::sync::Notify>, image: ImageData) -> ImageLibrary {
    ImageLibrary::new(move |_| {
        let release = Arc::clone(&release);
        let image = image.clone();
        Box::pin(async move {
            release.notified().await;
            Some(image)
        })
    })
}

#[tokio::test(start_paused = true)]
async fn an_icon_arriving_after_its_setting_was_removed_is_not_shown() {
    let release = Arc::new(tokio::sync::Notify::new());
    let custom = ImageData::new(b"late icon".to_vec());
    let mut harness = Harness::start_with_images(
        &json!({ "composeFiles": [{ "path": "/srv/boutique/compose.yaml", "icon": "shop.png" }] }),
        held_library(Arc::clone(&release), custom.clone()),
    )
    .await;
    assert_eq!(
        harness.item(MAIN_PAGE, SHOP).image,
        Some(icons::PROJECT_RUNNING.clone()),
        "the default icon shows while the image is being read"
    );

    harness
        .module
        .update_settings(&json!({ "composeFiles": ["/srv/boutique/compose.yaml"] }))
        .unwrap();
    release.notify_waiters();
    harness.settle().await;
    assert_eq!(
        harness.item(MAIN_PAGE, SHOP).image,
        Some(icons::PROJECT_RUNNING.clone())
    );
    assert_ne!(harness.item(MAIN_PAGE, SHOP).image, Some(custom));
}

#[tokio::test(start_paused = true)]
async fn an_icon_that_could_not_be_read_is_tried_again_with_the_next_settings() {
    let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let custom = ImageData::new(b"second try".to_vec());
    let library = {
        let (reads, custom) = (Arc::clone(&reads), custom.clone());
        ImageLibrary::new(move |_| {
            let first = reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
            let image = (!first).then(|| custom.clone());
            Box::pin(std::future::ready(image))
        })
    };
    let settings = json!({ "composeFiles": [
        { "path": "/srv/boutique/compose.yaml", "icon": "shop.png" }
    ] });
    let mut harness = Harness::start_with_images(&settings, library).await;
    assert_eq!(
        harness.item(MAIN_PAGE, SHOP).image,
        Some(icons::PROJECT_RUNNING.clone())
    );

    harness.module.update_settings(&settings).unwrap();
    harness.settle().await;
    assert_eq!(harness.item(MAIN_PAGE, SHOP).image, Some(custom));
}

// ---------------------------------------------------------------- Services without container

const WEB_PLANNED: &str = "service/boutique/web";
const WEB_UP: &str = "up/boutique/web";
const WEB_DOWN: &str = "down/boutique/web";
const WEB: &str = "container/boutique-web-1";

/// The declared shop, taken down: its services remain, without containers.
async fn shop_taken_down() -> Harness {
    let mut harness =
        Harness::start(&json!({ "composeFiles": ["/srv/boutique/compose.yaml"] })).await;
    harness.press(SHOP_DOWN, "down-keep").await;
    harness.press(SHOP_DOWN, "down-keep").await;
    harness.settle().await;
    harness
}

#[tokio::test(start_paused = true)]
async fn services_without_container_show_beside_the_containers_and_apart_from_stopped_ones() {
    let mut harness = shop_taken_down().await;
    let ids: Vec<_> = harness
        .items(SHOP)
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(
        ids[4..],
        [
            "service/boutique/api",
            "service/boutique/db",
            "service/boutique/web"
        ],
        "after the actions, sorted by name like containers"
    );
    let planned = harness.item(SHOP, WEB_PLANNED);
    assert_eq!(planned.image, Some(icons::SERVICE_PLANNED.clone()));
    assert_eq!(label(&planned), "web");

    // Once built (not started), it is a stopped container, drawn as such.
    harness.press(SHOP_UP, "up-no-start").await;
    harness.settle().await;
    let created = harness.item(SHOP, WEB);
    assert_eq!(created.image, Some(icons::STOPPED.clone()));
    assert!(harness.page(WEB_PLANNED).is_none(), "its page is gone");
}

#[tokio::test(start_paused = true)]
async fn a_service_without_container_can_only_be_brought_up() {
    let mut harness = shop_taken_down().await;
    let page = harness.page(WEB_PLANNED).unwrap();
    assert_eq!(page.title, "web · Non créé");
    assert_eq!(
        harness.items(WEB_PLANNED),
        [("up".into(), "Up".into(), KeyState::Normal)]
    );
    assert_eq!(
        harness.press(WEB_PLANNED, "up").await,
        [Command::OpenPage(WEB_UP.into())]
    );
    assert_eq!(harness.page(WEB_UP).unwrap().title, "Up · web");

    assert_eq!(
        harness.press(WEB_UP, "up-build").await,
        [Command::BackTo(SHOP.into())]
    );
    let working = harness.item(SHOP, WEB_PLANNED);
    assert_eq!(label(&working), "web Build…");
    assert_eq!(
        label(&harness.item(SHOP, "service/boutique/db")),
        "db",
        "only the service the operation is about"
    );
    assert_eq!(
        harness.settle().await,
        [feedback(SHOP, WEB_PLANNED, Feedback::Ok)],
        "the outcome shows on the service entry shown when it ends"
    );
    assert_eq!(
        harness.docker.actions().last().unwrap(),
        "up boutique --build web"
    );
    assert_eq!(
        harness.status("boutique-web-1"),
        Some(ContainerStatus::Running)
    );
    assert!(
        harness.status("boutique-db-1").is_none(),
        "the other services were left alone"
    );
}

#[tokio::test(start_paused = true)]
async fn a_container_has_its_own_up_and_down_for_its_service_only() {
    let mut harness = Harness::start(&json!({})).await;
    let ids: Vec<_> = harness
        .items(WEB)
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(ids[ids.len() - 2..], ["up", "down"]);
    assert_eq!(
        harness.press(WEB, "down").await,
        [Command::OpenPage(WEB_DOWN.into())]
    );
    assert_eq!(harness.page(WEB_DOWN).unwrap().title, "Down · web");

    // A down asks twice, then goes back to the project page.
    assert_eq!(
        harness.press(WEB_DOWN, "down-keep").await,
        Vec::<Command>::new()
    );
    assert_eq!(
        harness.press(WEB_DOWN, "down-keep").await,
        [Command::BackTo(SHOP.into())]
    );
    assert_eq!(
        harness.settle().await,
        [
            feedback(SHOP, WEB, Feedback::Ok),
            feedback(SHOP, WEB_PLANNED, Feedback::Ok)
        ],
        "both entries of the service are marked: the list of containers is not up to date yet"
    );
    assert!(harness.status("boutique-web-1").is_none());
    assert!(
        harness.status("boutique-db-1").is_some(),
        "the other services stay"
    );
    assert_eq!(
        harness.docker.actions().last().unwrap(),
        "down boutique web"
    );
}

#[tokio::test(start_paused = true)]
async fn a_container_outside_compose_has_no_up_and_down() {
    let harness = Harness::start(&json!({})).await;
    let ids: Vec<_> = harness
        .items(REDIS)
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    assert!(!ids.contains(&"up".to_owned()), "{ids:?}");
}

#[tokio::test(start_paused = true)]
async fn one_operation_at_a_time_on_a_project() {
    let mut harness = Harness::start(&json!({})).await;
    harness.docker.set_latency(Duration::from_secs(2));
    assert_eq!(
        harness.press(SHOP_UP, "up-pull").await,
        [Command::BackTo(SHOP.into())]
    );
    assert_eq!(
        harness.press(SHOP_UP, "up-build").await,
        Command::alert(),
        "refused while the first one runs"
    );
    assert_eq!(label(&harness.item(SHOP, WEB)), "web-1 Pull…");
    tokio::time::sleep(Duration::from_secs(3)).await;
    harness.settle().await;
    assert_eq!(harness.docker.actions(), ["up boutique --pull"]);
}

#[tokio::test(start_paused = true)]
async fn the_down_of_one_service_has_the_volumes_variant_too_and_asks_twice() {
    let mut harness = Harness::start(&json!({})).await;
    let expected = [
        ("down-keep".into(), "Down".into(), KeyState::Normal),
        (
            "down-volumes".into(),
            "Avec volumes".into(),
            KeyState::Normal,
        ),
    ];
    assert_eq!(harness.items(WEB_DOWN), expected);
    assert_eq!(harness.items(SHOP_DOWN), expected);

    assert_eq!(
        harness.press(WEB_DOWN, "down-volumes").await,
        Vec::<Command>::new(),
        "the first press only asks"
    );
    assert_eq!(
        label(&harness.item(WEB_DOWN, "down-volumes")),
        "Confirmer ?"
    );
    assert_eq!(
        harness.press(WEB_DOWN, "down-volumes").await,
        [Command::BackTo(SHOP.into())]
    );
    harness.settle().await;
    assert_eq!(
        harness.docker.actions().last().unwrap(),
        "down boutique --volumes web"
    );
}

#[tokio::test(start_paused = true)]
async fn services_of_projects_not_in_the_settings_are_read_when_the_project_is_opened() {
    const MONITORING_PROMETHEUS: &str = "service/supervision/prometheus";
    let mut harness = Harness::start(&json!({})).await;
    harness.docker.change_externally(|containers| {
        containers.retain(|c| c.name != "supervision-prometheus-1");
    });
    harness.settle().await;
    assert!(
        harness.page(MONITORING).is_some_and(|page| page
            .items
            .iter()
            .all(|item| item.id.0 != MONITORING_PROMETHEUS)),
        "not read by itself: the project is not one of the settings"
    );

    harness.press(MAIN_PAGE, MONITORING).await;
    harness.settle().await;
    assert_eq!(
        harness.item(MONITORING, MONITORING_PROMETHEUS).image,
        Some(icons::SERVICE_PLANNED.clone())
    );
}

// ---------------------------------------------------------------- Selection

const API: &str = "container/boutique-api-1";
const DB: &str = "container/boutique-db-1";
const PROMETHEUS: &str = "container/supervision-prometheus-1";

impl Harness {
    async fn rotate(&mut self, page: &str, ticks: i32) {
        self.module.on_context_rotate(&page.into(), ticks).await;
    }

    async fn tick(&mut self, page: &str) {
        self.module.on_context_press(&page.into()).await;
    }

    async fn tick_all(&mut self, page: &str) {
        self.module.on_context_long_press(&page.into()).await;
    }

    /// Moves the cursor of the shop page to `item` and presses the dial:
    /// ticks the service, or unticks it.
    async fn toggle_item(&mut self, item: &str) {
        let entries = self
            .keys_where(SHOP, |_| true)
            .into_iter()
            .filter(|id| !is_action(id))
            .count();
        // From wherever the cursor is, forward until it rests on `item`.
        for _ in 0..entries {
            if self.keys_where(SHOP, |key| key.focused) == [item] {
                break;
            }
            self.rotate(SHOP, 1).await;
        }
        assert_eq!(self.keys_where(SHOP, |key| key.focused), [item]);
        self.tick(SHOP).await;
    }

    fn dial(&self, page: &str) -> ContextDial {
        self.page(page).unwrap().context_dial
    }

    /// The ids of the items of `page` whose key satisfies `matching`.
    fn keys_where(&self, page: &str, matching: impl Fn(&KeyContent) -> bool) -> Vec<String> {
        self.page(page)
            .unwrap()
            .items
            .into_iter()
            .filter(|item| matching(&item.key))
            .map(|item| item.id.0)
            .collect()
    }

    fn focused(&self, page: &str) -> Vec<String> {
        self.keys_where(page, |key| key.focused)
    }

    fn ticked(&self, page: &str) -> Vec<String> {
        self.keys_where(page, |key| key.checked)
    }
}

fn is_action(item: &str) -> bool {
    !item.starts_with(CONTAINER_PREFIX) && !item.starts_with(SERVICE_PREFIX)
}

#[tokio::test(start_paused = true)]
async fn only_the_page_of_a_project_declares_the_contextual_dial() {
    let harness = Harness::start(&json!({})).await;
    assert_eq!(
        harness.dial(SHOP),
        ContextDial::none()
            .rotating("Choisir")
            .long_pressing("Tout"),
        "nothing to press until the cursor rests on an entry"
    );
    for page in [MAIN_PAGE, SHOP_UP, SHOP_DOWN, REDIS, WEB_UP] {
        assert_eq!(harness.dial(page), ContextDial::none(), "{page}");
    }
    assert_eq!(harness.focused(SHOP), Vec::<String>::new());
    assert_eq!(harness.ticked(SHOP), Vec::<String>::new());
}

#[tokio::test(start_paused = true)]
async fn turning_moves_the_cursor_over_the_entries_and_wraps_around() {
    let mut harness = Harness::start(&json!({})).await;
    harness.rotate(SHOP, 1).await;
    assert_eq!(harness.focused(SHOP), [API]);
    harness.rotate(SHOP, 1).await;
    assert_eq!(harness.focused(SHOP), [DB]);
    harness.rotate(SHOP, 2).await;
    assert_eq!(harness.focused(SHOP), [API], "after the last, the first");
    harness.rotate(SHOP, -1).await;
    assert_eq!(harness.focused(SHOP), [WEB], "before the first, the last");

    // The first step back with no cursor lands on the last entry.
    let mut other = Harness::start(&json!({})).await;
    other.rotate(SHOP, -1).await;
    assert_eq!(other.focused(SHOP), [WEB]);
}

#[tokio::test(start_paused = true)]
async fn pressing_ticks_and_unticks_the_entry_under_the_cursor() {
    let mut harness = Harness::start(&json!({})).await;
    harness.tick(SHOP).await;
    assert_eq!(harness.ticked(SHOP), Vec::<String>::new(), "no cursor yet");

    harness.rotate(SHOP, 1).await;
    assert_eq!(harness.dial(SHOP).hint.press.as_deref(), Some("Cocher"));
    harness.tick(SHOP).await;
    assert_eq!(harness.ticked(SHOP), [API]);
    assert_eq!(harness.focused(SHOP), [API], "the cursor stays on it");
    assert_eq!(harness.dial(SHOP).hint.press.as_deref(), Some("Décocher"));
    assert_eq!(label(&harness.item(SHOP, "start-all")), "Démarrer (1)");
    assert_eq!(label(&harness.item(SHOP, "stop-all")), "Arrêter (1)");

    harness.tick(SHOP).await;
    assert_eq!(harness.ticked(SHOP), Vec::<String>::new());
    assert_eq!(label(&harness.item(SHOP, "start-all")), "Tout démarrer");
}

#[tokio::test(start_paused = true)]
async fn holding_ticks_every_entry_then_none() {
    let mut harness = Harness::start(&json!({})).await;
    harness.toggle_item(API).await;
    harness.tick_all(SHOP).await;
    assert_eq!(harness.ticked(SHOP), [API, DB, WEB], "not all were ticked");
    assert_eq!(harness.dial(SHOP).long_press.as_deref(), Some("Aucun"));
    assert_eq!(label(&harness.item(SHOP, "start-all")), "Démarrer (3)");

    harness.tick_all(SHOP).await;
    assert_eq!(
        harness.ticked(SHOP),
        Vec::<String>::new(),
        "all were ticked"
    );
    assert_eq!(harness.dial(SHOP).long_press.as_deref(), Some("Tout"));
}

#[tokio::test(start_paused = true)]
async fn an_up_works_on_the_ticked_services_only_and_spends_the_selection() {
    let mut harness = Harness::start(&json!({})).await;
    harness.press(MAIN_PAGE, SHOP).await;
    harness.toggle_item(WEB).await;
    harness.toggle_item(API).await;
    assert_eq!(harness.page(SHOP_UP).unwrap().title, "Up · boutique (2)");

    assert_eq!(
        harness.press(SHOP_UP, "up-build").await,
        [Command::BackTo(SHOP.into())]
    );
    // The selection is spent: nothing ticked, no cursor, whole-project labels.
    assert_eq!(harness.ticked(SHOP), Vec::<String>::new());
    assert_eq!(harness.focused(SHOP), Vec::<String>::new());
    assert_eq!(label(&harness.item(SHOP, "start-all")), "Tout démarrer");
    assert_eq!(harness.page(SHOP_UP).unwrap().title, "Up · boutique");
    // Only the services it works on show it; the outcome marks each of them.
    assert_eq!(label(&harness.item(SHOP, API)), "api-1 Build…");
    assert_eq!(label(&harness.item(SHOP, WEB)), "web-1 Build…");
    assert_eq!(label(&harness.item(SHOP, DB)), "db-1");
    assert_eq!(
        harness.settle().await,
        [
            feedback(SHOP, API, Feedback::Ok),
            feedback(SHOP, "service/boutique/api", Feedback::Ok),
            feedback(SHOP, WEB, Feedback::Ok),
            feedback(SHOP, "service/boutique/web", Feedback::Ok),
        ]
    );
    assert_eq!(
        harness.docker.actions().last().unwrap(),
        "up boutique --build api web"
    );
}

#[tokio::test(start_paused = true)]
async fn without_a_selection_an_up_is_the_plain_one() {
    let mut harness = Harness::start(&json!({})).await;
    harness.press(SHOP_UP, "up-plain").await;
    harness.settle().await;
    assert_eq!(harness.docker.actions().last().unwrap(), "up boutique");
}

#[tokio::test(start_paused = true)]
async fn a_down_works_on_the_ticked_services_once_confirmed() {
    let mut harness = Harness::start(&json!({})).await;
    harness.toggle_item(DB).await;
    assert_eq!(
        harness.page(SHOP_DOWN).unwrap().title,
        "Down · boutique (1)"
    );
    assert_eq!(harness.press(SHOP_DOWN, "down-keep").await, []);
    assert_eq!(
        harness.press(SHOP_DOWN, "down-keep").await,
        [Command::BackTo(SHOP.into())]
    );
    harness.settle().await;
    assert_eq!(harness.docker.actions().last().unwrap(), "down boutique db");
    assert!(harness.status("boutique-db-1").is_none());
    assert_eq!(
        harness.status("boutique-api-1"),
        Some(ContainerStatus::Running)
    );
}

#[tokio::test(start_paused = true)]
async fn the_pages_of_a_service_ignore_the_selection() {
    let mut harness = Harness::start(&json!({})).await;
    harness.toggle_item(API).await;
    harness.toggle_item(DB).await;
    assert_eq!(harness.page(WEB_UP).unwrap().title, "Up · web");
    harness.press(WEB_UP, "up-plain").await;
    harness.settle().await;
    assert_eq!(harness.docker.actions().last().unwrap(), "up boutique web");
    assert_eq!(
        harness.ticked(SHOP),
        [API, DB],
        "an action on one service leaves the selection alone"
    );
}

#[tokio::test(start_paused = true)]
async fn stop_and_start_work_on_the_ticked_services_only() {
    let mut harness = Harness::start(&json!({})).await;
    harness.toggle_item(API).await;
    harness.toggle_item(WEB).await;
    assert_eq!(harness.press(SHOP, "stop-all").await, []);
    harness.settle().await;
    assert_eq!(
        harness.status("boutique-api-1"),
        Some(ContainerStatus::Exited)
    );
    assert_eq!(
        harness.status("boutique-web-1"),
        Some(ContainerStatus::Exited)
    );
    assert_eq!(
        harness.status("boutique-db-1"),
        Some(ContainerStatus::Running),
        "not ticked"
    );
    assert_eq!(harness.ticked(SHOP), Vec::<String>::new(), "spent");

    harness.toggle_item(WEB).await;
    harness.press(SHOP, "start-all").await;
    harness.settle().await;
    assert_eq!(
        harness.status("boutique-web-1"),
        Some(ContainerStatus::Running)
    );
    assert_eq!(
        harness.status("boutique-api-1"),
        Some(ContainerStatus::Exited)
    );
}

#[tokio::test(start_paused = true)]
async fn nothing_to_do_on_the_ticked_services_is_a_success_that_keeps_the_selection() {
    let mut harness = Harness::start(&json!({})).await;
    harness.toggle_item(API).await;
    // Already running: nothing to start.
    assert_eq!(
        harness.press(SHOP, "start-all").await,
        [feedback(SHOP, "start-all", Feedback::Ok)]
    );
    assert_eq!(harness.ticked(SHOP), [API]);
}

#[tokio::test(start_paused = true)]
async fn the_selection_starts_over_with_the_project_page() {
    let mut harness = Harness::start(&json!({})).await;
    harness.toggle_item(API).await;
    // Opening the project again from the main page.
    harness.press(MAIN_PAGE, SHOP).await;
    assert_eq!(harness.ticked(SHOP), Vec::<String>::new());
    assert_eq!(harness.focused(SHOP), Vec::<String>::new());

    // Another project takes the selection over: the first one's is gone.
    harness.toggle_item(DB).await;
    harness.press(MAIN_PAGE, MONITORING).await;
    harness.rotate(MONITORING, 1).await;
    harness.tick(MONITORING).await;
    assert_eq!(harness.ticked(MONITORING).len(), 1);
    assert_eq!(harness.ticked(SHOP), Vec::<String>::new());
    assert_eq!(label(&harness.item(SHOP, "start-all")), "Tout démarrer");
}

#[tokio::test(start_paused = true)]
async fn entries_that_are_gone_leave_the_selection() {
    let mut harness = Harness::start(&json!({})).await;
    harness.toggle_item(API).await;
    harness.toggle_item(WEB).await;
    harness.docker.change_externally(|containers| {
        containers.retain(|c| c.name != "boutique-web-1");
    });
    harness.settle().await;
    assert_eq!(harness.ticked(SHOP), [API]);
    assert_eq!(
        harness.focused(SHOP),
        Vec::<String>::new(),
        "the cursor was on web"
    );
    assert_eq!(label(&harness.item(SHOP, "stop-all")), "Arrêter (1)");
}

#[tokio::test(start_paused = true)]
async fn the_services_of_a_stopped_project_are_ticked_too() {
    let mut harness = Harness::start(&json!({})).await;
    harness.press(MAIN_PAGE, MONITORING).await;
    harness.rotate(MONITORING, 1).await;
    harness.rotate(MONITORING, 1).await;
    harness.tick(MONITORING).await;
    assert_eq!(harness.ticked(MONITORING), [PROMETHEUS]);
    harness.press(MONITORING, "start-all").await;
    harness.settle().await;
    assert_eq!(
        harness.status("supervision-prometheus-1"),
        Some(ContainerStatus::Running)
    );
    assert_eq!(
        harness.status("supervision-grafana-1"),
        Some(ContainerStatus::Exited),
        "not ticked"
    );
}

#[tokio::test(start_paused = true)]
async fn services_without_container_are_ticked_and_brought_up_with_the_rest() {
    let mut harness = shop_taken_down().await;
    harness.press(MAIN_PAGE, SHOP).await;
    harness.toggle_item("service/boutique/db").await;
    harness.toggle_item("service/boutique/api").await;
    assert_eq!(
        harness.ticked(SHOP),
        ["service/boutique/api", "service/boutique/db"]
    );
    harness.press(SHOP_UP, "up-plain").await;
    harness.settle().await;
    assert_eq!(
        harness.docker.actions().last().unwrap(),
        "up boutique api db"
    );
    assert_eq!(
        harness.status("boutique-api-1"),
        Some(ContainerStatus::Running)
    );
    assert!(harness.status("boutique-web-1").is_none());
}

#[tokio::test(start_paused = true)]
async fn the_replicas_of_a_service_are_ticked_together() {
    let mut harness = Harness::start(&json!({})).await;
    harness.docker.change_externally(|containers| {
        let mut replica = containers
            .iter()
            .find(|c| c.name == "boutique-web-1")
            .unwrap()
            .clone();
        replica.id = "web-2".into();
        replica.name = "boutique-web-2".into();
        containers.push(replica);
    });
    harness.settle().await;
    harness.toggle_item("container/boutique-web-2").await;
    assert_eq!(
        harness.ticked(SHOP),
        [WEB, "container/boutique-web-2"],
        "both containers of the service"
    );
    assert_eq!(label(&harness.item(SHOP, "stop-all")), "Arrêter (1)");
    assert_eq!(
        harness.focused(SHOP),
        ["container/boutique-web-2"],
        "the cursor is on one of them"
    );
}

#[tokio::test(start_paused = true)]
async fn the_contextual_dial_does_nothing_off_the_page_of_a_project() {
    let mut harness = Harness::start(&json!({})).await;
    for page in [MAIN_PAGE, SHOP_UP, REDIS, "elsewhere"] {
        harness.rotate(page, 1).await;
        harness.tick(page).await;
        harness.tick_all(page).await;
    }
    assert_eq!(harness.ticked(SHOP), Vec::<String>::new());
    assert_eq!(harness.focused(SHOP), Vec::<String>::new());
}

#[tokio::test(start_paused = true)]
async fn a_confirmation_does_not_survive_a_change_of_selection() {
    let mut harness = Harness::start(&json!({})).await;
    harness.toggle_item(DB).await;
    assert_eq!(harness.press(SHOP_DOWN, "down-keep").await, []);

    // Back on the page of the project, another service is ticked.
    harness.toggle_item(API).await;
    assert_eq!(
        harness.press(SHOP_DOWN, "down-keep").await,
        [],
        "what was confirmed is not what would go: confirm again"
    );
    harness.settle().await;
    assert!(
        harness
            .docker
            .actions()
            .iter()
            .all(|action| !action.starts_with("down")),
        "nothing was taken down yet"
    );
    assert_eq!(
        harness.press(SHOP_DOWN, "down-keep").await,
        [Command::BackTo(SHOP.into())]
    );
    harness.settle().await;
    assert_eq!(
        harness.docker.actions().last().unwrap(),
        "down boutique api db"
    );
}

#[tokio::test(start_paused = true)]
async fn a_selection_emptied_by_vanished_services_refuses_instead_of_widening() {
    let mut harness = Harness::start(&json!({})).await;
    harness.toggle_item(WEB).await;
    assert_eq!(harness.press(SHOP_DOWN, "down-keep").await, []);
    harness.docker.change_externally(|containers| {
        containers.retain(|c| c.name != "boutique-web-1");
    });
    harness.settle().await;

    // The whole project is not what the user meant: nothing goes.
    assert_eq!(
        harness.press(SHOP_DOWN, "down-keep").await,
        Command::alert()
    );
    assert_eq!(
        harness.press(SHOP_DOWN, "down-keep").await,
        Command::alert()
    );
    assert_eq!(harness.press(SHOP_UP, "up-plain").await, Command::alert());
    assert_eq!(harness.press(SHOP, "stop-all").await, Command::alert());
    assert_eq!(harness.press(SHOP, "start-all").await, Command::alert());
    harness.settle().await;
    assert_eq!(harness.docker.actions(), Vec::<String>::new());
    assert_eq!(
        harness.status("boutique-api-1"),
        Some(ContainerStatus::Running)
    );

    // Ticking again starts over, on what is there.
    harness.toggle_item(API).await;
    harness.press(SHOP_UP, "up-plain").await;
    harness.settle().await;
    assert_eq!(harness.docker.actions().last().unwrap(), "up boutique api");
}

#[tokio::test(start_paused = true)]
async fn a_lost_selection_is_forgotten_when_the_project_is_opened_again() {
    let mut harness = Harness::start(&json!({})).await;
    harness.toggle_item(WEB).await;
    harness.docker.change_externally(|containers| {
        containers.retain(|c| c.name != "boutique-web-1");
    });
    harness.settle().await;
    assert_eq!(harness.press(SHOP_UP, "up-plain").await, Command::alert());
    harness.press(MAIN_PAGE, SHOP).await;
    assert_eq!(
        harness.press(SHOP_UP, "up-plain").await,
        [Command::BackTo(SHOP.into())],
        "a plain up of the project"
    );
}

#[tokio::test(start_paused = true)]
async fn a_service_with_an_odd_name_cannot_be_ticked() {
    let mut harness = Harness::start(&json!({})).await;
    harness.docker.change_externally(|containers| {
        let mut odd = containers
            .iter()
            .find(|c| c.name == "boutique-web-1")
            .unwrap()
            .clone();
        odd.id = "odd".into();
        odd.name = "boutique-odd-1".into();
        odd.compose = odd.compose.map(|labels| {
            ComposeLabels::new(labels.project, labels.working_dir, labels.config_files)
                .with_service("--rm")
        });
        containers.push(odd);
    });
    harness.settle().await;
    harness.tick_all(SHOP).await;
    assert_eq!(harness.ticked(SHOP), [API, DB, WEB], "not the odd one");
    harness.press(SHOP_UP, "up-plain").await;
    harness.settle().await;
    assert_eq!(
        harness.docker.actions().last().unwrap(),
        "up boutique api db web"
    );
}

#[tokio::test(start_paused = true)]
async fn a_project_with_nothing_to_tick_declares_nothing() {
    let mut harness = Harness::start(&json!({})).await;
    harness.docker.change_externally(|containers| {
        for container in containers
            .iter_mut()
            .filter(|c| c.name.starts_with("boutique-"))
        {
            container.compose = container
                .compose
                .take()
                .map(|labels| ComposeLabels::new(labels.project, None, labels.config_files));
        }
    });
    harness.settle().await;
    assert_eq!(harness.dial(SHOP), ContextDial::none());
    harness.rotate(SHOP, 1).await;
    harness.tick_all(SHOP).await;
    assert_eq!(harness.focused(SHOP), Vec::<String>::new());
    assert_eq!(harness.ticked(SHOP), Vec::<String>::new());
}

#[tokio::test(start_paused = true)]
async fn a_project_that_vanishes_takes_its_selection_along() {
    let mut harness = Harness::start(&json!({})).await;
    harness.toggle_item(API).await;
    harness.docker.change_externally(|containers| {
        containers.retain(|c| !c.name.starts_with("boutique-"));
    });
    harness.settle().await;
    assert!(harness.page(SHOP).is_none(), "back to the main page");
    harness.rotate(SHOP, 1).await;
    harness.tick(SHOP).await;
    harness.tick_all(SHOP).await;
    assert!(harness.page(SHOP).is_none());
}

#[tokio::test(start_paused = true)]
async fn a_replica_that_goes_leaves_its_service_ticked_and_the_cursor_behind() {
    let mut harness = Harness::start(&json!({})).await;
    harness.docker.change_externally(|containers| {
        let mut replica = containers
            .iter()
            .find(|c| c.name == "boutique-web-1")
            .unwrap()
            .clone();
        replica.id = "web-2".into();
        replica.name = "boutique-web-2".into();
        containers.push(replica);
    });
    harness.settle().await;
    harness.toggle_item("container/boutique-web-2").await;
    harness.docker.change_externally(|containers| {
        containers.retain(|c| c.name != "boutique-web-2");
    });
    harness.settle().await;
    assert_eq!(harness.ticked(SHOP), [WEB], "the other replica remains");
    assert_eq!(harness.focused(SHOP), Vec::<String>::new());
}

#[tokio::test(start_paused = true)]
async fn the_replicas_of_a_ticked_service_are_all_stopped() {
    let mut harness = Harness::start(&json!({})).await;
    harness.docker.change_externally(|containers| {
        let mut replica = containers
            .iter()
            .find(|c| c.name == "boutique-web-1")
            .unwrap()
            .clone();
        replica.id = "web-2".into();
        replica.name = "boutique-web-2".into();
        containers.push(replica);
    });
    harness.settle().await;
    harness.toggle_item("container/boutique-web-2").await;
    harness.press(SHOP, "stop-all").await;
    harness.settle().await;
    for name in ["boutique-web-1", "boutique-web-2"] {
        assert_eq!(
            harness.status(name),
            Some(ContainerStatus::Exited),
            "{name}"
        );
    }
    assert_eq!(
        harness.status("boutique-api-1"),
        Some(ContainerStatus::Running)
    );
    assert_eq!(
        harness.status("boutique-db-1"),
        Some(ContainerStatus::Running)
    );

    // A service named once in the command, whatever its replicas.
    harness.toggle_item(WEB).await;
    harness.press(SHOP_UP, "up-plain").await;
    harness.settle().await;
    assert_eq!(harness.docker.actions().last().unwrap(), "up boutique web");
}

#[tokio::test(start_paused = true)]
async fn an_operation_already_running_refuses_another_and_keeps_the_selection() {
    let mut harness = Harness::start(&json!({})).await;
    harness.toggle_item(API).await;
    assert_eq!(
        harness.press(SHOP_UP, "up-plain").await,
        [Command::BackTo(SHOP.into())]
    );
    harness.toggle_item(WEB).await;
    assert_eq!(harness.press(SHOP_UP, "up-build").await, Command::alert());
    assert_eq!(harness.ticked(SHOP), [WEB], "kept for the next try");
}

#[tokio::test(start_paused = true)]
async fn only_services_that_are_planned_are_a_success_that_keeps_the_selection() {
    let mut harness = shop_taken_down().await;
    harness.press(MAIN_PAGE, SHOP).await;
    harness.toggle_item("service/boutique/api").await;
    assert_eq!(
        harness.press(SHOP, "stop-all").await,
        [feedback(SHOP, "stop-all", Feedback::Ok)],
        "no container to stop"
    );
    assert_eq!(harness.ticked(SHOP), ["service/boutique/api"]);
}

#[tokio::test(start_paused = true)]
async fn the_selection_survives_a_visit_to_a_sub_page() {
    let mut harness = Harness::start(&json!({})).await;
    harness.toggle_item(API).await;
    assert_eq!(
        harness.press(SHOP, "up").await,
        [Command::OpenPage(SHOP_UP.into())]
    );
    assert_eq!(
        harness.press(SHOP, API).await,
        [Command::OpenPage(API.into())]
    );
    assert_eq!(harness.ticked(SHOP), [API]);
    assert_eq!(harness.dial(API), ContextDial::none());
}
