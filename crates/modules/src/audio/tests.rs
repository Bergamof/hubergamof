use std::time::Duration;

use bergadeck_module_api::{EventSender, TaskScope};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use super::*;

/// Longest (simulated) silence after which the instance is considered idle.
const IDLE: Duration = Duration::from_secs(1);

struct Harness {
    module: Box<dyn Module>,
    events: UnboundedReceiver<ModuleEvent>,
    audio: Arc<SimulatedAudio>,
    _scope: TaskScope,
}

impl Harness {
    /// An instance on the demo server, once it read the state.
    async fn start(settings: &Value) -> Self {
        Self::on(SimulatedAudio::demo(), settings).await
    }

    async fn on(audio: SimulatedAudio, settings: &Value) -> Self {
        let audio = Arc::new(audio);
        let factory = AudioFactory::with_backend(Arc::clone(&audio) as _);
        let (sender, events) = mpsc::unbounded_channel();
        let scope = TaskScope::new();
        let ctx = ModuleContext::for_tests(AUDIO_KIND)
            .with_events(EventSender::new(move |event| sender.send(event).is_ok()))
            .with_tasks(scope.spawner());
        let mut harness = Self {
            module: factory.create(ctx, settings).unwrap(),
            events,
            audio,
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

    fn zone(&self) -> ZoneContent {
        self.module
            .zone(&NavigationView::new("Accueil", 0, 1, 2))
            .unwrap()
    }

    fn gauge(&self) -> GaugeContent {
        match self.zone() {
            ZoneContent::Gauge(gauge) => gauge,
            other => panic!("gauge expected, got {other:?}"),
        }
    }

    fn page(&self, page: &str) -> PageContent {
        self.module.page(&page.into()).unwrap()
    }

    fn item(&self, page: &str, item: &str) -> KeyContent {
        self.page(page)
            .items
            .into_iter()
            .find(|entry| entry.id.0 == item)
            .unwrap()
            .key
    }

    async fn press(&mut self, page: &str, item: &str) -> Vec<Command> {
        self.module.on_item_press(&page.into(), &item.into()).await
    }

    fn speakers(&self) -> AudioDevice {
        self.audio.state().sinks[0].clone()
    }
}

fn volume_changes(audio: &SimulatedAudio) -> Vec<i64> {
    audio
        .applied()
        .into_iter()
        .filter_map(|change| match change {
            AudioChange::SinkVolume { delta, .. } => Some(delta),
            _ => None,
        })
        .collect()
}

fn item_surface(page: &str, item: &str) -> ModuleSurface {
    ModuleSurface::Item {
        page: page.into(),
        item: item.into(),
    }
}

// ---------------------------------------------------------------- Settings

#[test]
fn settings_have_bounded_defaults() {
    let factory = AudioFactory::simulated();
    assert_eq!(
        factory.default_settings(),
        json!({ "volumeStep": 5, "maxVolume": 100 })
    );
    for valid in [json!({}), json!({ "volumeStep": 25, "maxVolume": 150 })] {
        assert!(factory.validate_settings(&valid).is_ok(), "{valid}");
    }
    for invalid in [
        json!({ "volumeStep": 0 }),
        json!({ "volumeStep": 26 }),
        json!({ "maxVolume": 99 }),
        json!({ "maxVolume": 151 }),
        json!({ "maxVolume": "loud" }),
    ] {
        assert!(factory.validate_settings(&invalid).is_err(), "{invalid}");
    }
    let descriptor = factory.descriptor();
    assert_eq!(descriptor.kind, AUDIO_KIND.into());
    assert!(descriptor.capabilities.key && descriptor.capabilities.dial);
    assert!(descriptor.capabilities.touch_zone);
}

#[tokio::test(start_paused = true)]
async fn settings_changes_keep_the_state_and_refuse_bad_values() {
    let mut harness = Harness::start(&json!({})).await;
    harness
        .module
        .update_settings(&json!({ "volumeStep": 10, "maxVolume": 150 }))
        .unwrap();
    assert!(matches!(
        harness.module.update_settings(&json!({ "maxVolume": 200 })),
        Err(ModuleError::InvalidSettings(_))
    ));
    assert!(matches!(
        AudioFactory::simulated()
            .create(ModuleContext::for_tests("a"), &json!({ "volumeStep": -1 })),
        Err(ModuleError::InvalidSettings(_))
    ));
    harness.module.on_dial_rotate(1).await;
    assert_eq!(harness.gauge().value, "50 %");
    harness.settle().await;
    assert_eq!(
        volume_changes(&harness.audio),
        [i64::from(level_of_percent(10))]
    );
}

// ---------------------------------------------------------------- Dial and zone

#[tokio::test(start_paused = true)]
async fn the_zone_shows_the_default_output_as_a_gauge() {
    let harness = Harness::start(&json!({})).await;
    let gauge = harness.gauge();
    assert_eq!(gauge.title, "Haut-parleurs");
    assert_eq!(gauge.value, "40 %");
    assert_eq!(gauge.level.permille(), 400);
    assert!(!gauge.dimmed);

    // With a higher ceiling, the bar spans up to it.
    let harness = Harness::start(&json!({ "maxVolume": 150 })).await;
    assert_eq!(harness.gauge().value, "40 %");
    assert_eq!(harness.gauge().level.permille(), 267);
}

#[tokio::test(start_paused = true)]
async fn turning_the_dial_changes_the_volume_up_to_the_ceiling() {
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(harness.module.on_dial_rotate(2).await, []);
    // Shown at once, before the server answers.
    assert_eq!(harness.gauge().value, "50 %");
    assert_eq!(harness.settle().await, []);
    assert_eq!(harness.speakers().volume.percent(), 50);

    for _ in 0..20 {
        harness.module.on_dial_rotate(1).await;
        harness.settle().await;
    }
    assert_eq!(harness.speakers().volume.channels(), [VOLUME_NORM; 2]);
    assert_eq!(harness.gauge().value, "100 %");
    assert_eq!(harness.gauge().level, GaugeLevel::FULL);
    let changes = harness.audio.applied().len();
    // At the ceiling, turning further asks nothing.
    harness.module.on_dial_rotate(3).await;
    harness.settle().await;
    assert_eq!(harness.audio.applied().len(), changes);

    harness.module.on_dial_rotate(-1).await;
    harness.settle().await;
    assert_eq!(harness.speakers().volume.percent(), 95);
}

#[tokio::test(start_paused = true)]
async fn unequal_channels_keep_their_balance() {
    let audio = SimulatedAudio::demo();
    audio.change_externally(|state| {
        state.sinks[0].volume = Volume::new(vec![level_of_percent(90), level_of_percent(98)]);
    });
    let mut harness = Harness::on(audio, &json!({})).await;
    assert_eq!(harness.gauge().value, "94 %");
    harness.module.on_dial_rotate(1).await;
    harness.settle().await;
    assert_eq!(
        harness.speakers().volume.channels(),
        [level_of_percent(92), VOLUME_NORM]
    );
}

#[tokio::test(start_paused = true)]
async fn a_fast_turn_sends_merged_volume_changes() {
    let mut harness = Harness::start(&json!({})).await;
    for _ in 0..10 {
        harness.module.on_dial_rotate(1).await;
    }
    assert_eq!(harness.gauge().value, "90 %");
    harness.settle().await;
    // The worker found every notch queued: one change for all of them.
    assert_eq!(
        volume_changes(&harness.audio),
        [10 * i64::from(level_of_percent(5))]
    );
    assert_eq!(harness.gauge().value, "90 %");
}

#[tokio::test(start_paused = true)]
async fn states_older_than_the_queued_changes_are_ignored() {
    let mut harness = Harness::start(&json!({})).await;
    harness.module.on_dial_rotate(1).await;
    // A state read before the change was applied.
    let stale = AudioEvent::State {
        snapshot: Some(SimulatedAudio::demo().state()),
        applied: 0,
    };
    harness.module.on_event(Box::new(stale)).await;
    assert_eq!(harness.gauge().value, "45 %");
    harness.settle().await;
    assert_eq!(harness.gauge().value, "45 %");
}

#[tokio::test(start_paused = true)]
async fn pressing_the_dial_mutes_and_unmutes_the_output() {
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(harness.module.on_dial_press().await, []);
    let gauge = harness.gauge();
    assert_eq!(gauge.value, "Muet");
    assert!(gauge.dimmed);
    assert_eq!(gauge.level.permille(), 400);
    harness.settle().await;
    assert!(harness.speakers().muted);
    assert_eq!(
        harness.module.key().image,
        Some(icons::SPEAKER_MUTED.clone())
    );

    harness.module.on_dial_press().await;
    harness.settle().await;
    assert!(!harness.speakers().muted);
    assert_eq!(harness.gauge().value, "40 %");
}

// ---------------------------------------------------------------- Pages

#[tokio::test(start_paused = true)]
async fn the_home_key_opens_the_audio_page() {
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(harness.module.key().label.as_deref(), Some("Audio"));
    assert_eq!(
        harness.module.on_key_press().await,
        [Command::OpenPage(MAIN_PAGE.into())]
    );
    let page = harness.page(MAIN_PAGE);
    assert_eq!(page.title, "Audio");
    let labels: Vec<_> = page
        .items
        .iter()
        .map(|item| item.key.label.clone().unwrap())
        .collect();
    assert_eq!(labels, ["Micro", "Sortie", "Entrée"]);
    assert_eq!(
        harness.press(MAIN_PAGE, OUTPUTS_ITEM).await,
        [Command::OpenPage(OUTPUTS_PAGE.into())]
    );
    assert_eq!(
        harness.press(MAIN_PAGE, INPUTS_ITEM).await,
        [Command::OpenPage(INPUTS_PAGE.into())]
    );
    assert_eq!(harness.press(MAIN_PAGE, "unknown").await, []);
    assert!(harness.module.page(&"unknown".into()).is_none());
}

#[tokio::test(start_paused = true)]
async fn the_microphone_item_mutes_the_default_input() {
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(
        harness.item(MAIN_PAGE, MICROPHONE_ITEM).state,
        KeyState::Normal
    );
    assert_eq!(harness.press(MAIN_PAGE, MICROPHONE_ITEM).await, []);
    let key = harness.item(MAIN_PAGE, MICROPHONE_ITEM);
    assert_eq!(key.state, KeyState::Attention);
    assert_eq!(key.label.as_deref(), Some("Micro coupé"));
    harness.settle().await;
    assert!(harness.audio.state().sources[0].muted);
    harness.press(MAIN_PAGE, MICROPHONE_ITEM).await;
    harness.settle().await;
    assert!(!harness.audio.state().sources[0].muted);
}

#[tokio::test(start_paused = true)]
async fn device_pages_list_devices_and_select_the_default() {
    let mut harness = Harness::start(&json!({})).await;
    let outputs = harness.page(OUTPUTS_PAGE);
    assert_eq!(outputs.title, "Sortie");
    let listed: Vec<_> = outputs
        .items
        .iter()
        .map(|item| {
            (
                item.id.0.as_str(),
                item.key.label.clone().unwrap(),
                item.key.state,
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            (
                "demo.speakers",
                "Haut-parleurs".to_owned(),
                KeyState::Active
            ),
            (
                "demo.headset",
                "Casque Bluetooth".to_owned(),
                KeyState::Normal
            ),
            ("demo.hdmi", "Écran HDMI".to_owned(), KeyState::Normal),
        ]
    );

    assert_eq!(harness.press(OUTPUTS_PAGE, "demo.headset").await, []);
    assert_eq!(
        harness.item(OUTPUTS_PAGE, "demo.headset").state,
        KeyState::Active
    );
    assert_eq!(
        harness.item(OUTPUTS_PAGE, "demo.speakers").state,
        KeyState::Normal
    );
    assert_eq!(
        harness.settle().await,
        [Command::FeedbackOn(
            item_surface(OUTPUTS_PAGE, "demo.headset"),
            Feedback::Ok
        )]
    );
    assert_eq!(
        harness.audio.state().default_sink.as_deref(),
        Some("demo.headset")
    );
    // The dial now drives the new default.
    assert_eq!(harness.gauge().title, "Casque Bluetooth");

    assert_eq!(harness.page(INPUTS_PAGE).items.len(), 2);
    harness.press(INPUTS_PAGE, "demo.usb-mic").await;
    harness.settle().await;
    assert_eq!(
        harness.audio.state().default_source.as_deref(),
        Some("demo.usb-mic")
    );
    assert_eq!(harness.press(INPUTS_PAGE, "gone").await, alert());
}

#[tokio::test(start_paused = true)]
async fn external_changes_update_the_display() {
    let mut harness = Harness::start(&json!({})).await;
    harness.audio.change_externally(|state| {
        state.sinks.push(AudioDevice {
            name: "bluez.new".into(),
            description: "Écouteurs".into(),
            volume: Volume::uniform(level_of_percent(30), 2),
            muted: false,
        });
        state.default_sink = Some("bluez.new".into());
    });
    harness.settle().await;
    assert_eq!(harness.gauge().title, "Écouteurs");
    assert_eq!(harness.gauge().value, "30 %");
    assert_eq!(harness.page(OUTPUTS_PAGE).items.len(), 4);
}

// ---------------------------------------------------------------- Failures

#[tokio::test(start_paused = true)]
async fn failed_changes_alert_on_their_surface_and_restore_the_state() {
    let mut harness = Harness::start(&json!({})).await;
    harness.audio.set_available(false);
    harness.module.on_dial_rotate(1).await;
    assert_eq!(
        harness.settle().await,
        [Command::FeedbackOn(ModuleSurface::Zone, Feedback::Alert)]
    );
    assert_eq!(
        harness.zone(),
        ZoneContent::Lines(vec!["Audio indisponible".into()])
    );
    // Without a state, actions alert at once.
    assert_eq!(harness.module.on_dial_rotate(1).await, alert());
    assert_eq!(harness.module.on_dial_press().await, alert());
    assert_eq!(harness.press(MAIN_PAGE, MICROPHONE_ITEM).await, alert());
    assert_eq!(harness.press(OUTPUTS_PAGE, "demo.headset").await, alert());
    assert_eq!(harness.page(OUTPUTS_PAGE).items, []);

    // Back: the next attempt reads the state again.
    harness.audio.set_available(true);
    harness.module.on_dial_rotate(1).await;
    harness.settle().await;
    assert_eq!(harness.gauge().value, "40 %");
}

#[tokio::test(start_paused = true)]
async fn a_rejected_change_shows_the_real_state_again() {
    let audio = SimulatedAudio::demo();
    let mut harness = Harness::on(audio, &json!({})).await;
    // The output disappears before the server sees the change.
    harness.module.on_dial_rotate(1).await;
    harness.audio.change_externally(|state| {
        state.sinks.remove(0);
        state.default_sink = Some("demo.headset".into());
    });
    let commands = harness.settle().await;
    assert_eq!(
        commands,
        [Command::FeedbackOn(ModuleSurface::Zone, Feedback::Alert)]
    );
    assert_eq!(harness.gauge().title, "Casque Bluetooth");
    assert_eq!(harness.gauge().value, "25 %");
}

#[tokio::test(start_paused = true)]
async fn an_unreachable_server_is_shown_and_never_crashes() {
    let audio = SimulatedAudio::demo();
    audio.set_available(false);
    let mut harness = Harness::on(audio, &json!({})).await;
    assert_eq!(
        harness.zone(),
        ZoneContent::Lines(vec!["Audio indisponible".into()])
    );
    assert_eq!(
        harness.item(MAIN_PAGE, MICROPHONE_ITEM).state,
        KeyState::Normal
    );
    assert_eq!(harness.module.key().label.as_deref(), Some("Audio"));
    harness.audio.set_available(true);
    // The watcher retries and the state comes back by itself.
    tokio::time::sleep(worker::MAX_BACKOFF).await;
    harness.settle().await;
    assert_eq!(harness.gauge().value, "40 %");
}

#[tokio::test(start_paused = true)]
async fn the_zone_tells_when_there_is_no_output() {
    let audio = SimulatedAudio::new(AudioSnapshot::default());
    let mut harness = Harness::on(audio, &json!({})).await;
    assert_eq!(
        harness.zone(),
        ZoneContent::Lines(vec!["Aucune sortie".into()])
    );
    assert_eq!(harness.module.on_dial_rotate(1).await, alert());
}

#[test]
fn the_zone_waits_for_the_first_state() {
    let module = AudioFactory::simulated()
        .create(ModuleContext::for_tests(AUDIO_KIND), &json!({}))
        .unwrap();
    let zone = module.zone(&NavigationView::new("Accueil", 0, 1, 2));
    assert_eq!(zone, Some(ZoneContent::Lines(vec!["Audio".into()])));
}

#[tokio::test(start_paused = true)]
async fn the_tasks_stop_with_the_instance() {
    let harness = Harness::start(&json!({})).await;
    let audio = Arc::clone(&harness.audio);
    let reads = audio.snapshot_reads();
    drop(harness);
    audio.change_externally(|_| {});
    tokio::time::sleep(IDLE).await;
    assert_eq!(audio.snapshot_reads(), reads);
}
