use std::time::Duration;

use hubergamof_module_api::TaskScope;
use hubergamof_module_api::testing::TestContext;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use super::*;

const IDLE: Duration = Duration::from_secs(1);

fn dial() -> DialFunctionId {
    AMBIENT_FUNCTION.into()
}

fn ambient(mode: Mode, level: u8) -> Ambient {
    Ambient {
        mode,
        level,
        focus_on_voice: false,
    }
}

fn headset(link: Link, ambient: Option<Ambient>) -> Headset {
    Headset {
        link,
        ambient,
        wind_reduction: true,
    }
}

struct Harness {
    module: Box<dyn Module>,
    events: UnboundedReceiver<ModuleEvent>,
    headset: Arc<SimulatedHeadset>,
    _scope: TaskScope,
}

impl Harness {
    async fn on(headset: SimulatedHeadset) -> Self {
        let headset = Arc::new(headset);
        let factory = HeadphonesFactory::new(Arc::clone(&headset) as _);
        let TestContext { ctx, events, scope } = TestContext::new(HEADPHONES_KIND);
        let mut harness = Self {
            module: factory.create(ctx, &json!({})).unwrap(),
            events,
            headset,
            _scope: scope,
        };
        harness.settle().await;
        harness
    }

    async fn start() -> Self {
        Self::on(SimulatedHeadset::demo()).await
    }

    /// Hands the events to the instance until none comes for [`IDLE`].
    async fn settle(&mut self) {
        while let Ok(Some(event)) = tokio::time::timeout(IDLE, self.events.recv()).await {
            self.module.on_event(event).await;
        }
    }

    fn label(&self) -> String {
        self.module.key().label.unwrap()
    }
}

#[tokio::test(start_paused = true)]
async fn the_key_names_the_mode_the_headphones_are_in() {
    let mut harness = Harness::start().await;
    assert_eq!(harness.label(), "Réduction de bruit");
    harness
        .headset
        .say(headset(Link::Connected, Some(ambient(Mode::Off, 3))));
    harness.settle().await;
    assert_eq!(harness.label(), "Normal");
}

#[tokio::test(start_paused = true)]
async fn the_key_is_named_after_the_module_while_nothing_is_known() {
    let harness = Harness::on(SimulatedHeadset::with(None::<Headset>)).await;
    assert_eq!(harness.label(), "Casque");
}

#[tokio::test(start_paused = true)]
async fn pressing_the_key_goes_through_the_modes_and_around() {
    let mut harness = Harness::start().await;
    let mut seen = Vec::new();
    for _ in 0..4 {
        assert_eq!(harness.module.on_key_press().await, Vec::new());
        harness.settle().await;
        seen.push(harness.label());
    }
    assert_eq!(
        seen,
        [
            "Son ambiant",
            "Réduction du vent",
            "Normal",
            "Réduction de bruit"
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn the_wind_reduction_is_skipped_without_it() {
    let mut harness = Harness::on(SimulatedHeadset::with(Some(Headset {
        link: Link::Connected,
        ambient: Some(ambient(Mode::Ambient, 5)),
        wind_reduction: false,
    })))
    .await;
    harness.module.on_key_press().await;
    assert_eq!(harness.headset.requests(), [ambient(Mode::Off, 5)]);
}

#[tokio::test(start_paused = true)]
async fn turning_the_dial_sets_the_level_and_makes_the_outside_heard() {
    let mut harness = Harness::start().await;
    assert_eq!(harness.module.on_dial_rotate(&dial(), 3).await, Vec::new());
    assert_eq!(harness.headset.requests(), [ambient(Mode::Ambient, 13)]);
}

#[tokio::test(start_paused = true)]
async fn turns_before_the_answer_add_up_and_stay_within_the_levels() {
    let mut harness = Harness::start().await;
    for _ in 0..3 {
        harness.module.on_dial_rotate(&dial(), 4).await;
    }
    assert_eq!(
        harness.headset.requests().last(),
        Some(&ambient(Mode::Ambient, MAX_LEVEL))
    );
    harness.settle().await;
    harness.module.on_dial_rotate(&dial(), -100).await;
    assert_eq!(
        harness.headset.requests().last(),
        Some(&ambient(Mode::Ambient, 0))
    );
}

#[tokio::test(start_paused = true)]
async fn the_dial_keeps_the_wind_reduction_it_finds() {
    let mut harness = Harness::on(SimulatedHeadset::with(headset(
        Link::Connected,
        Some(ambient(Mode::WindReduction, 4)),
    )))
    .await;
    harness.module.on_dial_rotate(&dial(), 1).await;
    assert_eq!(
        harness.headset.requests(),
        [ambient(Mode::WindReduction, 5)]
    );
}

#[tokio::test(start_paused = true)]
async fn a_turn_that_changes_nothing_asks_nothing() {
    let mut harness = Harness::on(SimulatedHeadset::with(headset(
        Link::Connected,
        Some(ambient(Mode::Ambient, MAX_LEVEL)),
    )))
    .await;
    assert_eq!(harness.module.on_dial_rotate(&dial(), 2).await, Vec::new());
    assert_eq!(harness.headset.requests(), Vec::new());
}

#[tokio::test(start_paused = true)]
async fn pressing_the_dial_changes_the_mode_and_other_functions_do_nothing() {
    let mut harness = Harness::start().await;
    harness.module.on_dial_press(&dial()).await;
    assert_eq!(harness.headset.requests(), [ambient(Mode::Ambient, 10)]);
    let other = DialFunctionId::from("other");
    assert_eq!(harness.module.on_dial_press(&other).await, Vec::new());
    assert_eq!(harness.module.on_dial_rotate(&other, 1).await, Vec::new());
    assert_eq!(harness.headset.requests().len(), 1);
    assert_eq!(
        harness.module.dial_hint(&dial()),
        DialHint::none()
            .rotating("Niveau ambiant")
            .pressing("Mode suivant")
    );
    assert_eq!(harness.module.dial_hint(&other), DialHint::none());
}

#[tokio::test(start_paused = true)]
async fn an_unopened_link_is_opened_once_and_the_press_is_reported_failed() {
    let mut harness = Harness::on(SimulatedHeadset::with(headset(
        Link::Idle,
        Some(ambient(Mode::Off, 0)),
    )))
    .await;
    assert_eq!(harness.module.on_key_press().await, Command::alert());
    assert_eq!(harness.headset.opened(), 1);
    assert_eq!(harness.headset.requests(), Vec::new());
}

#[tokio::test(start_paused = true)]
async fn nothing_is_known_when_no_headphones_are_connected() {
    let mut harness = Harness::on(SimulatedHeadset::with(None::<Headset>)).await;
    assert_eq!(harness.module.on_key_press().await, Command::alert());
    assert_eq!(harness.headset.opened(), 1);
    assert_eq!(harness.module.radial_items(), Vec::new());
    assert_eq!(harness.module.radial_status(), None);
}

#[tokio::test(start_paused = true)]
async fn the_petal_offers_the_other_modes_and_says_where_it_is() {
    let mut harness = Harness::start().await;
    let ids = |harness: &Harness| {
        harness
            .module
            .radial_items()
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&harness), ["ambient", "off"]);
    assert_eq!(
        harness.module.radial_status().as_deref(),
        Some("Réduction de bruit")
    );
    harness.module.on_radial_action("ambient").await;
    harness.settle().await;
    assert_eq!(ids(&harness), ["noise-cancelling", "off"]);
    assert_eq!(
        harness.module.radial_status().as_deref(),
        Some("Son ambiant 10")
    );
    let off = harness
        .module
        .radial_items()
        .into_iter()
        .find(|item| item.id == "off")
        .unwrap();
    assert_eq!(off.state, RadialItemState::Off);
}

#[tokio::test(start_paused = true)]
async fn a_petal_action_vibrates_unless_it_failed_and_unknown_ones_do_nothing() {
    let mut harness = Harness::start().await;
    assert_eq!(
        harness.module.on_radial_action("next-mode").await,
        vec![Command::haptic("mode-changed")]
    );
    assert_eq!(harness.module.on_radial_action("nothing").await, Vec::new());

    let mut idle = Harness::on(SimulatedHeadset::with(headset(
        Link::Cut,
        Some(ambient(Mode::Ambient, 0)),
    )))
    .await;
    assert_eq!(idle.module.on_radial_action("off").await, Command::alert());
    assert_eq!(
        idle.module.on_radial_action("noise-cancelling").await,
        Command::alert()
    );
}

#[tokio::test(start_paused = true)]
async fn the_module_only_asks_to_open_a_cut_link_the_hub_decides() {
    let mut harness = Harness::on(SimulatedHeadset::with(headset(
        Link::Cut,
        Some(ambient(Mode::Off, 0)),
    )))
    .await;
    assert_eq!(harness.module.on_key_press().await, Command::alert());
    assert_eq!(harness.headset.opened(), 1);
}

/// Headphones linked that take no request.
#[derive(Debug)]
struct Refusing {
    state: tokio::sync::watch::Sender<Option<Headset>>,
    opened: std::sync::atomic::AtomicU32,
    asked: std::sync::atomic::AtomicU32,
}

impl HeadsetControl for Refusing {
    fn watch(&self) -> tokio::sync::watch::Receiver<Option<Headset>> {
        self.state.subscribe()
    }

    fn set_ambient(&self, _: Ambient) -> Result<(), HeadsetFault> {
        self.asked
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Err(HeadsetFault::Refused)
    }

    fn open(&self) {
        self.opened
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[tokio::test(start_paused = true)]
async fn a_request_the_headphones_refuse_is_reported_and_not_remembered() {
    let control = Arc::new(Refusing {
        state: tokio::sync::watch::channel(Some(headset(
            Link::Connected,
            Some(ambient(Mode::Off, 0)),
        )))
        .0,
        opened: 0.into(),
        asked: 0.into(),
    });
    let factory = HeadphonesFactory::new(Arc::clone(&control) as _);
    let TestContext { ctx, scope, .. } = TestContext::new(HEADPHONES_KIND);
    let _scope = scope;
    let mut module = factory.create(ctx, &json!({})).unwrap();
    assert_eq!(module.on_key_press().await, Command::alert());
    // A refusal is not a closed link, and the next press starts from what the
    // headphones said.
    assert_eq!(module.on_key_press().await, Command::alert());
    assert_eq!(module.on_radial_action("ambient").await, Command::alert());
    assert_eq!(control.opened.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(control.asked.load(std::sync::atomic::Ordering::Relaxed), 3);
}

#[tokio::test(start_paused = true)]
async fn what_the_headphones_say_ends_what_was_asked_of_them() {
    let mut harness = Harness::start().await;
    harness.module.on_dial_rotate(&dial(), 3).await;
    // The headphones end up elsewhere (a button, another app): the next turn
    // starts from there, not from what was asked.
    harness
        .headset
        .say(headset(Link::Connected, Some(ambient(Mode::Off, 3))));
    harness.settle().await;
    harness.module.on_dial_rotate(&dial(), 1).await;
    assert_eq!(
        harness.headset.requests().last(),
        Some(&ambient(Mode::Ambient, 4))
    );
}

#[tokio::test(start_paused = true)]
async fn losing_the_headphones_forgets_everything() {
    let mut harness = Harness::start().await;
    harness.module.on_dial_rotate(&dial(), 1).await;
    harness.headset.say(None::<Headset>);
    harness.settle().await;
    assert_eq!(harness.label(), "Casque");
    assert_eq!(harness.module.radial_items(), Vec::new());
    assert_eq!(harness.module.radial_status(), None);
    assert_eq!(
        harness.module.on_dial_rotate(&dial(), 1).await,
        Command::alert()
    );
    assert_eq!(harness.headset.opened(), 1);
}

#[tokio::test(start_paused = true)]
async fn nothing_is_shown_or_set_until_the_link_is_open() {
    for link in [Link::Idle, Link::Connecting, Link::Released, Link::Cut] {
        let mut harness = Harness::on(SimulatedHeadset::with(headset(
            link,
            Some(ambient(Mode::Ambient, 5)),
        )))
        .await;
        assert_eq!(harness.label(), "Casque", "{link:?}");
        assert_eq!(harness.module.radial_items(), Vec::new(), "{link:?}");
        assert_eq!(
            harness.module.on_key_press().await,
            Command::alert(),
            "{link:?}"
        );
        assert_eq!(harness.headset.requests(), Vec::new(), "{link:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_dial_never_overflows_and_ignores_no_ticks() {
    let mut harness = Harness::start().await;
    assert_eq!(harness.module.on_dial_rotate(&dial(), 0).await, Vec::new());
    assert_eq!(harness.headset.requests(), Vec::new());
    harness.module.on_dial_rotate(&dial(), i32::MAX).await;
    harness.settle().await;
    harness.module.on_dial_rotate(&dial(), i32::MIN).await;
    assert_eq!(
        harness.headset.requests(),
        [ambient(Mode::Ambient, MAX_LEVEL), ambient(Mode::Ambient, 0)]
    );
}

#[tokio::test(start_paused = true)]
async fn turning_down_at_the_bottom_of_ambient_asks_nothing_but_wakes_other_modes() {
    let mut ambient_mode = Harness::on(SimulatedHeadset::with(headset(
        Link::Connected,
        Some(ambient(Mode::Ambient, 0)),
    )))
    .await;
    ambient_mode.module.on_dial_rotate(&dial(), -1).await;
    assert_eq!(ambient_mode.headset.requests(), Vec::new());

    let mut off = Harness::on(SimulatedHeadset::with(headset(
        Link::Connected,
        Some(ambient(Mode::Off, 0)),
    )))
    .await;
    off.module.on_dial_rotate(&dial(), -1).await;
    assert_eq!(off.headset.requests(), [ambient(Mode::Ambient, 0)]);
}

#[tokio::test(start_paused = true)]
async fn the_wind_reduction_in_use_is_shown_and_left_by_the_next_mode() {
    let mut harness = Harness::on(SimulatedHeadset::with(Headset {
        link: Link::Connected,
        ambient: Some(ambient(Mode::WindReduction, 7)),
        wind_reduction: false,
    }))
    .await;
    assert_eq!(
        harness.module.radial_status().as_deref(),
        Some("Réduction du vent")
    );
    assert_eq!(harness.module.radial_items().len(), 3);
    harness.module.on_key_press().await;
    assert_eq!(harness.headset.requests(), [ambient(Mode::Off, 7)]);
}

#[tokio::test(start_paused = true)]
async fn a_petal_action_that_changes_nothing_does_not_vibrate() {
    let mut harness = Harness::start().await;
    assert_eq!(
        harness.module.on_radial_action("noise-cancelling").await,
        Vec::new()
    );
    assert_eq!(harness.headset.requests(), Vec::new());
}

#[test]
fn the_catalog_describes_the_module() {
    let factory = HeadphonesFactory::new(Arc::new(NoHeadset::default()));
    let descriptor = factory.descriptor();
    assert_eq!(descriptor.kind, HEADPHONES_KIND.into());
    assert!(descriptor.capabilities.key);
    assert_eq!(descriptor.dial_functions[0].id, AMBIENT_FUNCTION.into());
    assert_eq!(factory.lifetime(), ModuleLifetime::Application);
    assert!(factory.icon().is_some());
    assert_eq!(factory.radial_actions().len(), 4);
    assert_eq!(factory.haptic_events()[0].id, "mode-changed");
}

#[tokio::test(start_paused = true)]
async fn the_module_has_no_settings_to_change() {
    let mut harness = Harness::start().await;
    assert!(harness.module.update_settings(&json!({})).is_ok());
}
