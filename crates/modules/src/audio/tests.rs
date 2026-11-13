use std::time::Duration;

use hubergamof_module_api::testing::TestContext;
use hubergamof_module_api::{Feedback, StripWidget, TaskScope};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use super::*;

/// The dial function of the module.
fn volume() -> DialFunctionId {
    VOLUME_FUNCTION.into()
}

/// Longest (simulated) silence after which the instance is considered idle.
const IDLE: Duration = Duration::from_secs(1);

struct Harness {
    module: Box<dyn Module>,
    events: UnboundedReceiver<ModuleEvent>,
    audio: Arc<SimulatedAudio>,
    factory: AudioFactory,
    _scope: TaskScope,
}

impl Harness {
    /// An instance on the demo server, once it read the state.
    async fn start(settings: &Value) -> Self {
        Self::on(SimulatedAudio::demo(), settings).await
    }

    async fn on(audio: SimulatedAudio, settings: &Value) -> Self {
        let audio = Arc::new(audio);
        let factory = AudioFactory::new(Arc::clone(&audio) as _);
        let TestContext { ctx, events, scope } = TestContext::new(AUDIO_KIND);
        let mut harness = Self {
            module: factory.create(ctx, settings).unwrap(),
            events,
            audio,
            factory,
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

    fn widget(&self) -> Option<VolumeWidget> {
        match self.module.strip_widget() {
            Some(StripWidget::Volume(widget)) => Some(widget),
            None => None,
            other => panic!("volume widget expected, got {other:?}"),
        }
    }

    fn percent(&self) -> Option<u16> {
        self.widget().map(|widget| widget.percent)
    }

    fn hint(&self) -> DialHint {
        self.module.dial_hint(&volume())
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

/// Volumes set so far, in percent.
fn volume_changes(audio: &SimulatedAudio) -> Vec<u32> {
    audio
        .applied()
        .into_iter()
        .filter_map(|change| match change {
            AudioChange::SinkVolume { volume, .. } => Some(volume.percent()),
            _ => None,
        })
        .collect()
}

fn simulated_factory() -> AudioFactory {
    AudioFactory::new(Arc::new(SimulatedAudio::demo()))
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
    let factory = simulated_factory();
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
    assert!(descriptor.capabilities.key);
    let functions: Vec<_> = descriptor.dial_functions.iter().map(|f| &f.id.0).collect();
    assert_eq!(functions, [VOLUME_FUNCTION]);
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
        simulated_factory().create(ModuleContext::for_tests("a"), &json!({ "volumeStep": -1 })),
        Err(ModuleError::InvalidSettings(_))
    ));
    harness.module.on_dial_rotate(&volume(), 1).await;
    assert_eq!(harness.percent(), Some(50));
    harness.settle().await;
    assert_eq!(volume_changes(&harness.audio), [50]);
}

// ---------------------------------------------------------------- Dial and widget

#[tokio::test(start_paused = true)]
async fn the_widget_shows_the_volume_of_the_default_output() {
    let harness = Harness::start(&json!({})).await;
    assert_eq!(harness.widget(), Some(VolumeWidget::new(40, false)));
    assert_eq!(
        harness.hint(),
        DialHint::none().rotating("Volume").pressing("Muet")
    );

    // The ceiling does not change what is shown: the volume itself.
    let harness = Harness::start(&json!({ "maxVolume": 150 })).await;
    assert_eq!(harness.percent(), Some(40));
}

#[tokio::test(start_paused = true)]
async fn only_the_volume_function_is_handled() {
    let mut harness = Harness::start(&json!({})).await;
    let other: DialFunctionId = "other".into();
    assert_eq!(harness.module.on_dial_rotate(&other, 3).await, []);
    assert_eq!(harness.module.on_dial_press(&other).await, []);
    assert_eq!(harness.module.dial_hint(&other), DialHint::none());
    assert_eq!(harness.percent(), Some(40));
}

#[tokio::test(start_paused = true)]
async fn turning_the_dial_changes_the_volume_up_to_the_ceiling() {
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(harness.module.on_dial_rotate(&volume(), 2).await, []);
    // Shown at once, before the server answers.
    assert_eq!(harness.percent(), Some(50));
    assert_eq!(harness.settle().await, []);
    assert_eq!(harness.speakers().volume.percent(), 50);

    for _ in 0..20 {
        harness.module.on_dial_rotate(&volume(), 1).await;
        harness.settle().await;
    }
    assert_eq!(harness.speakers().volume.channels(), [VOLUME_NORM; 2]);
    assert_eq!(harness.percent(), Some(100));
    let changes = harness.audio.applied().len();
    // At the ceiling, turning further asks nothing.
    harness.module.on_dial_rotate(&volume(), 3).await;
    harness.settle().await;
    assert_eq!(harness.audio.applied().len(), changes);

    harness.module.on_dial_rotate(&volume(), -1).await;
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
    assert_eq!(harness.percent(), Some(94));
    harness.module.on_dial_rotate(&volume(), 1).await;
    harness.settle().await;
    assert_eq!(
        harness.speakers().volume.channels(),
        [level_of_percent(92), VOLUME_NORM]
    );
}

#[tokio::test(start_paused = true)]
async fn lowering_unequal_channels_reaches_silence() {
    let audio = SimulatedAudio::demo();
    audio.change_externally(|state| {
        state.sinks[0].volume = Volume::new(vec![level_of_percent(3), level_of_percent(20)]);
    });
    let mut harness = Harness::on(audio, &json!({})).await;
    for _ in 0..5 {
        harness.module.on_dial_rotate(&volume(), -1).await;
        harness.settle().await;
    }
    assert_eq!(harness.speakers().volume.channels(), [0, 0]);
    let changes = harness.audio.applied().len();
    harness.module.on_dial_rotate(&volume(), -1).await;
    harness.settle().await;
    assert_eq!(
        harness.audio.applied().len(),
        changes,
        "nothing left to lower"
    );
}

#[tokio::test(start_paused = true)]
async fn a_volume_above_the_ceiling_is_only_lowered() {
    let audio = SimulatedAudio::demo();
    audio.change_externally(|state| {
        state.sinks[0].volume = Volume::uniform(level_of_percent(140), 2);
    });
    let mut harness = Harness::on(audio, &json!({})).await;
    assert_eq!(harness.module.on_dial_rotate(&volume(), 3).await, []);
    harness.settle().await;
    assert_eq!(harness.audio.applied(), [], "not raised");
    assert_eq!(harness.percent(), Some(140));
    harness.module.on_dial_rotate(&volume(), -1).await;
    harness.settle().await;
    assert_eq!(volume_changes(&harness.audio), [135]);
}

#[tokio::test(start_paused = true)]
async fn a_volume_raised_elsewhere_meanwhile_never_exceeds_the_ceiling() {
    let mut harness = Harness::start(&json!({})).await;
    // Raised to 140 % by another program; the instance has not read it yet
    // when the dial turns.
    harness.audio.change_externally(|state| {
        state.sinks[0].volume = Volume::uniform(level_of_percent(140), 2);
    });
    harness.module.on_dial_rotate(&volume(), 1).await;
    harness.settle().await;
    let volume = harness.speakers().volume;
    assert!(
        volume.channels().iter().all(|&level| level <= VOLUME_NORM),
        "{volume:?}"
    );
    assert_eq!(volume.percent(), 45);
}

#[tokio::test(start_paused = true)]
async fn a_sink_without_channels_cannot_be_turned() {
    let audio = SimulatedAudio::demo();
    audio.change_externally(|state| state.sinks[0].volume = Volume::default());
    let mut harness = Harness::on(audio, &json!({})).await;
    assert_eq!(
        harness.module.on_dial_rotate(&volume(), 1).await,
        Command::alert()
    );
    harness.settle().await;
    assert_eq!(harness.audio.applied(), []);
}

#[tokio::test(start_paused = true)]
async fn a_fast_turn_sends_merged_volume_changes() {
    let mut harness = Harness::start(&json!({})).await;
    for _ in 0..10 {
        harness.module.on_dial_rotate(&volume(), 1).await;
    }
    assert_eq!(harness.percent(), Some(90));
    harness.settle().await;
    // The worker found every notch queued: one change for all of them.
    assert_eq!(volume_changes(&harness.audio), [90]);
    assert_eq!(harness.percent(), Some(90));
}

#[tokio::test(start_paused = true)]
async fn states_older_than_the_queued_changes_are_ignored() {
    let mut harness = Harness::start(&json!({})).await;
    harness.module.on_dial_rotate(&volume(), 1).await;
    // A state read before the change was applied.
    let stale = AudioEvent::State(HubState {
        server: ServerView::Ready(Arc::new(SimulatedAudio::demo().state())),
        applied: 0,
    });
    harness.module.on_event(Box::new(stale)).await;
    assert_eq!(harness.percent(), Some(45));
    harness.settle().await;
    assert_eq!(harness.percent(), Some(45));
}

#[tokio::test(start_paused = true)]
async fn pressing_the_dial_mutes_and_unmutes_the_output() {
    let mut harness = Harness::start(&json!({})).await;
    assert_eq!(harness.module.on_dial_press(&volume()).await, []);
    assert_eq!(harness.widget(), Some(VolumeWidget::new(40, true)));
    assert_eq!(
        harness.hint(),
        DialHint::none().rotating("Volume").pressing("Réactiver")
    );
    harness.settle().await;
    assert!(harness.speakers().muted);
    assert_eq!(
        harness.module.key().image,
        Some(icons::SPEAKER_MUTED.clone())
    );

    harness.module.on_dial_press(&volume()).await;
    harness.settle().await;
    assert!(!harness.speakers().muted);
    assert_eq!(harness.percent(), Some(40));
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
    assert_eq!(labels, ["Micro", "Sorties", "Entrées"]);
    // Muting the microphone and choosing an input are told apart.
    let icons: Vec<_> = page.items.iter().map(|item| &item.key.image).collect();
    assert_ne!(icons[0], icons[1]);
    assert_ne!(icons[0], icons[2]);
    assert_ne!(icons[1], icons[2]);
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
    let live = harness.item(MAIN_PAGE, MICROPHONE_ITEM);
    assert_eq!(live.state, KeyState::Normal);
    assert_eq!(harness.press(MAIN_PAGE, MICROPHONE_ITEM).await, []);
    let key = harness.item(MAIN_PAGE, MICROPHONE_ITEM);
    // The struck icon says it: no attention colour.
    assert_eq!(key.state, KeyState::Normal);
    assert_eq!(key.label.as_deref(), Some("Micro coupé"));
    assert_ne!(key.image, live.image);
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
    assert_eq!(outputs.title, "Sorties");
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

    let icon_of = |harness: &Harness, name: &str| harness.item(OUTPUTS_PAGE, name).image;
    assert_eq!(
        icon_of(&harness, "demo.speakers").as_ref(),
        Some(&*icons::SPEAKER_SELECTED)
    );
    assert_eq!(
        icon_of(&harness, "demo.headset").as_ref(),
        Some(&*icons::SPEAKER)
    );

    assert_eq!(harness.press(OUTPUTS_PAGE, "demo.headset").await, []);
    assert_eq!(
        harness.item(OUTPUTS_PAGE, "demo.headset").state,
        KeyState::Active
    );
    assert_eq!(
        icon_of(&harness, "demo.headset").as_ref(),
        Some(&*icons::SPEAKER_SELECTED)
    );
    assert_eq!(
        icon_of(&harness, "demo.speakers").as_ref(),
        Some(&*icons::SPEAKER)
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
    assert_eq!(harness.percent(), Some(25));

    assert_eq!(harness.page(INPUTS_PAGE).items.len(), 2);
    harness.press(INPUTS_PAGE, "demo.usb-mic").await;
    harness.settle().await;
    assert_eq!(
        harness.audio.state().default_source.as_deref(),
        Some("demo.usb-mic")
    );
    assert_eq!(harness.press(INPUTS_PAGE, "gone").await, Command::alert());
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
    assert_eq!(harness.percent(), Some(30));
    assert_eq!(harness.page(OUTPUTS_PAGE).items.len(), 4);
}

// ---------------------------------------------------------------- Failures

#[tokio::test(start_paused = true)]
async fn failed_changes_alert_on_their_surface_and_restore_the_state() {
    let mut harness = Harness::start(&json!({})).await;
    harness.audio.set_available(false);
    harness.module.on_dial_rotate(&volume(), 1).await;
    assert_eq!(
        harness.settle().await,
        [Command::FeedbackOn(ModuleSurface::Dial, Feedback::Alert)]
    );
    assert_eq!(harness.widget(), None);
    // Without a state, actions alert at once.
    assert_eq!(
        harness.module.on_dial_rotate(&volume(), 1).await,
        Command::alert()
    );
    assert_eq!(
        harness.module.on_dial_press(&volume()).await,
        Command::alert()
    );
    assert_eq!(
        harness.press(MAIN_PAGE, MICROPHONE_ITEM).await,
        Command::alert()
    );
    assert_eq!(
        harness.press(OUTPUTS_PAGE, "demo.headset").await,
        Command::alert()
    );
    assert_eq!(harness.page(OUTPUTS_PAGE).items, []);

    // Back: the next attempt reads the state again.
    harness.audio.set_available(true);
    harness.module.on_dial_rotate(&volume(), 1).await;
    harness.settle().await;
    assert_eq!(harness.percent(), Some(40));
}

#[tokio::test(start_paused = true)]
async fn a_rejected_change_shows_the_real_state_again() {
    let audio = SimulatedAudio::demo();
    let mut harness = Harness::on(audio, &json!({})).await;
    // The output disappears before the server sees the change.
    harness.module.on_dial_rotate(&volume(), 1).await;
    harness.audio.change_externally(|state| {
        state.sinks.remove(0);
        state.default_sink = Some("demo.headset".into());
    });
    let commands = harness.settle().await;
    assert_eq!(
        commands,
        [Command::FeedbackOn(ModuleSurface::Dial, Feedback::Alert)]
    );
    assert_eq!(harness.percent(), Some(25));
}

#[tokio::test(start_paused = true)]
async fn an_unreachable_server_is_shown_and_never_crashes() {
    let audio = SimulatedAudio::demo();
    audio.set_available(false);
    let mut harness = Harness::on(audio, &json!({})).await;
    assert_eq!(harness.widget(), None);
    assert_eq!(
        harness.item(MAIN_PAGE, MICROPHONE_ITEM).state,
        KeyState::Normal
    );
    assert_eq!(harness.module.key().label.as_deref(), Some("Audio"));
    harness.audio.set_available(true);
    // The watcher retries and the state comes back by itself.
    tokio::time::sleep(hub::MAX_BACKOFF).await;
    harness.settle().await;
    assert_eq!(harness.percent(), Some(40));
}

#[tokio::test(start_paused = true)]
async fn a_change_made_after_the_worker_stopped_is_applied() {
    let mut harness = Harness::start(&json!({})).await;
    harness.factory.hub.crash_worker();
    harness.settle().await;
    assert_eq!(harness.module.on_dial_rotate(&volume(), 1).await, []);
    assert_eq!(harness.percent(), Some(45));
    harness.settle().await;
    assert_eq!(harness.percent(), Some(45));
    assert_eq!(harness.audio.state().sinks[0].volume.percent(), 45);
}

#[tokio::test(start_paused = true)]
async fn two_instances_share_the_server() {
    let mut first = Harness::start(&json!({})).await;
    let TestContext {
        ctx,
        mut events,
        scope: _scope,
    } = TestContext::new("second");
    let mut second = first.factory.create(ctx, &json!({})).unwrap();
    first.module.on_dial_press(&volume()).await;
    first.settle().await;
    while let Ok(Some(event)) = tokio::time::timeout(IDLE, events.recv()).await {
        second.on_event(event).await;
    }
    assert!(matches!(
        second.strip_widget(),
        Some(StripWidget::Volume(widget)) if widget.muted
    ));
    assert_eq!(first.audio.subscriptions(), 1);
}

#[tokio::test(start_paused = true)]
async fn there_is_no_widget_without_an_output() {
    let audio = SimulatedAudio::new(AudioSnapshot::default());
    let mut harness = Harness::on(audio, &json!({})).await;
    assert_eq!(harness.widget(), None);
    assert_eq!(
        harness.module.on_dial_rotate(&volume(), 1).await,
        Command::alert()
    );
}

#[test]
fn the_widget_waits_for_the_first_state() {
    let module = simulated_factory()
        .create(ModuleContext::for_tests(AUDIO_KIND), &json!({}))
        .unwrap();
    assert_eq!(module.strip_widget(), None);
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

#[tokio::test(start_paused = true)]
async fn a_missing_or_old_pactl_shows_no_widget_and_recovers() {
    let missing: fn() -> AudioError = || AudioError::Spawn {
        program: "pactl".into(),
        source: std::io::ErrorKind::NotFound.into(),
    };
    let too_old: fn() -> AudioError = || AudioError::Failed {
        command: "--format=json info".into(),
        status: "exit status: 1".into(),
        stderr: "pactl: unrecognized option '--format=json'".into(),
    };
    for make in [missing, too_old] {
        let audio = SimulatedAudio::demo();
        audio.fail_with(make);
        let mut harness = Harness::on(audio, &json!({})).await;
        assert_eq!(harness.widget(), None);

        // Installed (or upgraded) since: the next attempt shows the volume.
        harness.audio.set_available(true);
        harness.module.on_dial_rotate(&volume(), 1).await;
        harness.settle().await;
        assert_eq!(harness.percent(), Some(40));
    }
}
