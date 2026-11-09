use hubergamof_module_api::testing::TestContext;
use hubergamof_module_api::{ImageLibrary, TaskScope};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

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
    assert!(!descriptor.capabilities.dial && !descriptor.capabilities.touch_zone);
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
    assert_eq!(labels, ["Up", "Up + build", "Up + pull"]);
    assert_eq!(harness.page(SHOP_UP).unwrap().title, "Up · boutique");
    harness.press(SHOP_UP, "up-build").await;
    assert_eq!(label(&harness.item(SHOP_UP, "up-build")), "Up…");
    assert_eq!(
        harness.settle().await,
        [feedback(SHOP_UP, "up-build", Feedback::Ok)]
    );
    assert_eq!(harness.docker.actions(), ["up boutique --build"]);
}

#[tokio::test(start_paused = true)]
async fn down_needs_a_second_press_and_the_volumes_variant_is_in_attention() {
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(
        harness.items(SHOP_DOWN),
        [
            ("down-keep".into(), "Down".into(), KeyState::Normal),
            (
                "down-volumes".into(),
                "Down + volumes".into(),
                KeyState::Attention
            ),
        ]
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
    assert_eq!(label(&harness.item(MAIN_PAGE, SHOP)), "boutique 0/0");
    let actions: Vec<_> = harness
        .items(SHOP)
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(actions, ["start-all", "stop-all", "up", "down"]);

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
        [feedback("up/blog", "up-plain", Feedback::Alert)]
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
    assert_eq!(label(&harness.item(REDIS, "stop")), "Arrêt…");
    assert_eq!(
        harness.settle().await,
        [feedback(REDIS, "stop", Feedback::Ok)]
    );
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

    harness.press(MIGRATION, "remove").await;
    assert_eq!(label(&harness.item(MIGRATION, "remove")), "Suppression…");
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
async fn a_failed_action_alerts_on_its_key() {
    let mut harness = Harness::start(&json!({})).await;
    harness.press(REDIS, "restart").await;
    harness.docker.set_available(false);
    let commands = harness.settle().await;
    assert_eq!(commands, [feedback(REDIS, "restart", Feedback::Alert)]);
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
    for item in ["up-plain", "up-build", "up-pull"] {
        harness.press(SHOP_UP, item).await;
        harness.settle().await;
    }
    assert_eq!(
        harness.docker.actions(),
        ["up boutique", "up boutique --build", "up boutique --pull"]
    );
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
    assert_eq!(label(&harness.item(REDIS, "stop")), "Arrêt…");
    assert_eq!(label(&harness.item(MIGRATION, "start")), "Démarrage…");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut commands = harness.settle().await;
    commands.sort_by_key(|command| format!("{command:?}"));
    assert_eq!(
        commands,
        [
            feedback(REDIS, "stop", Feedback::Ok),
            feedback(MIGRATION, "start", Feedback::Ok),
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
