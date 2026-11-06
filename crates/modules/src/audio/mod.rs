//! Audio module: volume and mute of the default output on a dial (gauge in
//! the zone above it), microphone mute and choice of the default output and
//! input on its pages.
//!
//! The audio server is reached through an [`AudioBackend`]: `pactl` on the
//! session server ([`PactlBackend`]) or an in-memory server
//! ([`SimulatedAudio`], `--demo` and tests), shared by every instance
//! through one hub (module `hub`). Handlers never wait for it: they update
//! what is shown at once and submit the change; failures come back as a ⚠
//! on the surface concerned, and what changed is read again after every
//! change, made here or elsewhere.

mod backend;
mod hub;
mod icons;
mod pactl;
mod simulated;
mod state;

use std::sync::Arc;

use async_trait::async_trait;
use hubergamof_module_api::{
    Command, GaugeAccent, GaugeContent, GaugeLevel, GaugeShape, ImageData, ItemId, KeyContent,
    KeyState, Module, ModuleCapabilities, ModuleContext, ModuleDescriptor, ModuleError,
    ModuleEvent, ModuleFactory, ModuleSurface, NavigationView, PageContent, PageId, PageItem, Rgb,
    SettingField, SettingKind, ZoneContent,
};
use serde::Deserialize;
use serde_json::Value;

pub use backend::{AudioBackend, AudioChange, AudioChanges, AudioError, AudioFault, ReadScope};
use hub::{AudioEvent, AudioHub, HubState, Sequence, ServerView, Subscription};
pub use pactl::PactlBackend;
pub use simulated::SimulatedAudio;
pub use state::{
    AudioDevice, AudioSnapshot, Defaults, Direction, PartialSnapshot, VOLUME_NORM, Volume,
};
use state::{level_of_percent, percent_of};

use crate::background::forward_states;

/// Kind of the audio module.
pub const AUDIO_KIND: &str = "audio";

const VOLUME_STEP_KEY: &str = "volumeStep";
const MAX_VOLUME_KEY: &str = "maxVolume";

/// Volume change per dial notch, in percent.
pub(crate) const DEFAULT_VOLUME_STEP: u32 = 5;
/// Smallest allowed volume step.
pub(crate) const MIN_VOLUME_STEP: u32 = 1;
/// Largest allowed volume step.
pub(crate) const MAX_VOLUME_STEP: u32 = 25;
/// Volume the dial does not go beyond, in percent.
pub const DEFAULT_MAX_VOLUME: u32 = 100;
/// Smallest allowed ceiling: the dial always reaches 100 %.
pub(crate) const MIN_MAX_VOLUME: u32 = 100;
/// Largest allowed ceiling (amplification beyond distorts).
pub(crate) const MAX_MAX_VOLUME: u32 = 150;

const PERCENT_UNIT: &str = "%";

/// Page opened by the home key.
const MAIN_PAGE: &str = "audio";
const OUTPUTS_PAGE: &str = "outputs";
const INPUTS_PAGE: &str = "inputs";
/// Items of [`MAIN_PAGE`].
const MICROPHONE_ITEM: &str = "microphone";
const OUTPUTS_ITEM: &str = OUTPUTS_PAGE;
const INPUTS_ITEM: &str = INPUTS_PAGE;

const MODULE_LABEL: &str = "Audio";
const MICROPHONE_LABEL: &str = "Micro";
const MICROPHONE_MUTED_LABEL: &str = "Micro coupé";
const OUTPUTS_LABEL: &str = "Sortie";
const INPUTS_LABEL: &str = "Entrée";
const MUTED_VALUE: &str = "Muet";
const UNAVAILABLE_TEXT: &str = "Audio indisponible";
const TOOL_MISSING_TEXT: &str = "pactl absent";
const TOOL_TOO_OLD_TEXT: &str = "pactl trop ancien";
const NO_OUTPUT_TEXT: &str = "Aucune sortie";

/// Creates [`Audio`] instances, all driving the same audio server through
/// one hub.
#[derive(Clone)]
pub struct AudioFactory {
    hub: Arc<AudioHub>,
}

impl AudioFactory {
    /// Factory driving `backend`.
    #[must_use]
    pub fn new(backend: Arc<dyn AudioBackend>) -> Self {
        Self {
            hub: AudioHub::new(backend),
        }
    }
}

impl std::fmt::Debug for AudioFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioFactory")
            .field("hub", &self.hub)
            .finish()
    }
}

/// Kind, capabilities and settings of the audio module.
fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        kind: AUDIO_KIND.into(),
        name: "Audio".into(),
        description: "Molette : volume de la sortie (appui : couper). Touche : micro, choix \
                      de la sortie et de l'entrée."
            .into(),
        capabilities: ModuleCapabilities {
            key: true,
            dial: true,
            touch_zone: true,
        },
        settings: vec![
            percent_setting(
                VOLUME_STEP_KEY,
                "Pas de la molette",
                DEFAULT_VOLUME_STEP,
                (MIN_VOLUME_STEP, MAX_VOLUME_STEP),
            ),
            SettingField {
                help: Some("La molette ne monte pas le volume au-delà.".into()),
                ..percent_setting(
                    MAX_VOLUME_KEY,
                    "Volume maximal",
                    DEFAULT_MAX_VOLUME,
                    (MIN_MAX_VOLUME, MAX_MAX_VOLUME),
                )
            },
        ],
    }
}

impl ModuleFactory for AudioFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        descriptor()
    }

    fn icon(&self) -> Option<ImageData> {
        Some(icons::SPEAKER.clone())
    }

    fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
        let settings = AudioSettings::parse(settings)?;
        let hub = self.hub.subscribe(ctx.events().clone());
        ctx.tasks().spawn(forward_states(
            hub.states(),
            ctx.events().clone(),
            AudioEvent::State,
        ));
        Ok(Box::new(Audio {
            settings,
            view: View::Loading,
            hub,
            last_sent: 0,
        }))
    }
}

fn percent_setting(key: &str, label: &str, default: u32, (min, max): (u32, u32)) -> SettingField {
    SettingField {
        key: key.into(),
        label: label.into(),
        help: None,
        kind: SettingKind::Integer {
            default: default.into(),
            min: Some(min.into()),
            max: Some(max.into()),
            unit: Some(PERCENT_UNIT.into()),
        },
    }
}

/// Settings of an audio instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct AudioSettings {
    /// Percent per dial notch.
    volume_step: u32,
    /// Ceiling of the dial, in percent.
    max_volume: u32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            volume_step: DEFAULT_VOLUME_STEP,
            max_volume: DEFAULT_MAX_VOLUME,
        }
    }
}

impl AudioSettings {
    /// Checks the bounds (also when the host did not), then reads them.
    fn parse(settings: &Value) -> Result<Self, ModuleError> {
        descriptor().parse_settings(settings)
    }

    fn step(self) -> i64 {
        level_of_percent(self.volume_step).into()
    }

    fn ceiling(self) -> u32 {
        level_of_percent(self.max_volume)
    }
}

/// What the instance knows of the audio server.
#[derive(Debug, Clone, PartialEq, Eq)]
enum View {
    /// Not read yet.
    Loading,
    /// Last state read, with the changes queued since applied.
    Ready(AudioSnapshot),
    /// The server could not be read.
    Unavailable(AudioFault),
}

/// One instance: the state shown and its access to the hub.
pub struct Audio {
    settings: AudioSettings,
    view: View,
    hub: Subscription,
    /// Sequence number of the last change submitted.
    last_sent: Sequence,
}

impl Audio {
    fn snapshot(&self) -> Option<&AudioSnapshot> {
        match &self.view {
            View::Ready(snapshot) => Some(snapshot),
            View::Loading | View::Unavailable(_) => None,
        }
    }

    fn snapshot_mut(&mut self) -> Option<&mut AudioSnapshot> {
        match &mut self.view {
            View::Ready(snapshot) => Some(snapshot),
            View::Loading | View::Unavailable(_) => None,
        }
    }

    /// Submits `change`; `surface` will show its outcome. Should the hub
    /// refuse it (no Tokio runtime), the change shown at once is not kept:
    /// the instance shows the server as unavailable until the next state.
    fn request(&mut self, change: AudioChange, surface: ModuleSurface) -> Vec<Command> {
        if let Some(seq) = self.hub.submit(change, surface) {
            self.last_sent = seq;
            Vec::new()
        } else {
            self.view = View::Unavailable(AudioFault::Unreachable);
            Command::alert()
        }
    }

    /// The action needs a state the instance does not have: alert, and read
    /// the state again in case the server came back.
    fn unavailable(&self) -> Vec<Command> {
        self.hub.refresh();
        Command::alert()
    }

    fn toggle_mute(&mut self, direction: Direction, surface: ModuleSurface) -> Vec<Command> {
        let Some(device) = self
            .snapshot_mut()
            .and_then(|snapshot| snapshot.default_device_mut(direction))
        else {
            return self.unavailable();
        };
        device.muted = !device.muted;
        let change = AudioChange::Mute {
            direction,
            device: device.name.clone(),
            muted: device.muted,
        };
        self.request(change, surface)
    }

    fn select_default(
        &mut self,
        direction: Direction,
        page: &PageId,
        item: &ItemId,
    ) -> Vec<Command> {
        let Some(snapshot) = self.snapshot_mut() else {
            return self.unavailable();
        };
        if snapshot.device_mut(direction, &item.0).is_none() {
            return self.unavailable();
        }
        snapshot.set_default(direction, &item.0);
        let change = AudioChange::Default {
            direction,
            device: item.0.clone(),
        };
        let surface = ModuleSurface::Item {
            page: page.clone(),
            item: item.clone(),
        };
        self.request(change, surface)
    }

    fn main_page(&self) -> PageContent {
        let microphone_muted = self
            .snapshot()
            .and_then(|snapshot| snapshot.default_device(Direction::Input))
            .is_some_and(|device| device.muted);
        let microphone = if microphone_muted {
            KeyContent {
                image: Some(icons::MICROPHONE_MUTED.clone()),
                label: Some(MICROPHONE_MUTED_LABEL.into()),
                state: KeyState::Attention,
            }
        } else {
            KeyContent::with_icon(&icons::MICROPHONE, MICROPHONE_LABEL)
        };
        PageContent {
            title: MODULE_LABEL.into(),
            items: vec![
                item(MICROPHONE_ITEM, microphone),
                item(
                    OUTPUTS_ITEM,
                    KeyContent::with_icon(&icons::SPEAKER, OUTPUTS_LABEL),
                ),
                item(
                    INPUTS_ITEM,
                    KeyContent::with_icon(&icons::MICROPHONE, INPUTS_LABEL),
                ),
            ],
        }
    }

    fn devices_page(&self, direction: Direction) -> PageContent {
        let (title, icon, selected_icon) = match direction {
            Direction::Output => (OUTPUTS_LABEL, &icons::SPEAKER, &icons::SPEAKER_SELECTED),
            Direction::Input => (
                INPUTS_LABEL,
                &icons::MICROPHONE,
                &icons::MICROPHONE_SELECTED,
            ),
        };
        let items = self
            .snapshot()
            .map(|snapshot| {
                let default = snapshot.default_name(direction);
                snapshot
                    .devices(direction)
                    .iter()
                    .map(|device| {
                        let (icon, state) = if default == Some(device.name.as_str()) {
                            (selected_icon, KeyState::Active)
                        } else {
                            (icon, KeyState::Normal)
                        };
                        item(
                            &device.name,
                            KeyContent::with_icon(icon, &device.description).in_state(state),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        PageContent {
            title: title.into(),
            items,
        }
    }

    fn handle(&mut self, event: AudioEvent) -> Vec<Command> {
        match event {
            AudioEvent::State(HubState { server, applied }) => {
                // An older state would undo the changes still queued.
                if applied >= self.last_sent {
                    self.view = match server {
                        ServerView::Loading => View::Loading,
                        ServerView::Ready(snapshot) => View::Ready((*snapshot).clone()),
                        ServerView::Unavailable(fault) => View::Unavailable(fault),
                    };
                }
                Vec::new()
            }
            AudioEvent::Outcome { surface, feedback } => {
                vec![Command::FeedbackOn(surface, feedback)]
            }
        }
    }
}

fn unavailable_text(fault: AudioFault) -> &'static str {
    match fault {
        AudioFault::ToolMissing => TOOL_MISSING_TEXT,
        AudioFault::ToolTooOld => TOOL_TOO_OLD_TEXT,
        AudioFault::Unreachable => UNAVAILABLE_TEXT,
    }
}

fn item(id: &str, key: KeyContent) -> PageItem {
    PageItem { id: id.into(), key }
}

fn direction_of(page: &PageId) -> Option<Direction> {
    match page.0.as_str() {
        OUTPUTS_PAGE => Some(Direction::Output),
        INPUTS_PAGE => Some(Direction::Input),
        _ => None,
    }
}

#[async_trait]
impl Module for Audio {
    fn key(&self) -> KeyContent {
        let muted = self
            .snapshot()
            .and_then(|snapshot| snapshot.default_device(Direction::Output))
            .is_some_and(|device| device.muted);
        let icon = if muted {
            &icons::SPEAKER_MUTED
        } else {
            &icons::SPEAKER
        };
        KeyContent::with_icon(icon, MODULE_LABEL)
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        vec![Command::OpenPage(MAIN_PAGE.into())]
    }

    fn page(&self, page: &PageId) -> Option<PageContent> {
        if page.0 == MAIN_PAGE {
            return Some(self.main_page());
        }
        direction_of(page).map(|direction| self.devices_page(direction))
    }

    async fn on_item_press(&mut self, page: &PageId, item: &ItemId) -> Vec<Command> {
        if let Some(direction) = direction_of(page) {
            return self.select_default(direction, page, item);
        }
        match item.0.as_str() {
            MICROPHONE_ITEM => {
                let surface = ModuleSurface::Item {
                    page: page.clone(),
                    item: item.clone(),
                };
                self.toggle_mute(Direction::Input, surface)
            }
            OUTPUTS_ITEM => vec![Command::OpenPage(OUTPUTS_PAGE.into())],
            INPUTS_ITEM => vec![Command::OpenPage(INPUTS_PAGE.into())],
            _ => Vec::new(),
        }
    }

    async fn on_dial_rotate(&mut self, ticks: i32) -> Vec<Command> {
        let settings = self.settings;
        let Some(sink) = self
            .snapshot_mut()
            .and_then(|snapshot| snapshot.default_device_mut(Direction::Output))
        else {
            return self.unavailable();
        };
        if sink.volume.channels().is_empty() {
            // Nothing known of the channels: no volume to set.
            return Command::alert();
        }
        let delta = i64::from(ticks).saturating_mul(settings.step());
        let volume = sink.volume.stepped(delta, settings.ceiling());
        if volume == sink.volume {
            return Vec::new();
        }
        // Absolute, from the state shown: never beyond the ceiling, even if
        // the volume changed elsewhere meanwhile.
        sink.volume = volume.clone();
        let change = AudioChange::SinkVolume {
            sink: sink.name.clone(),
            volume,
        };
        self.request(change, ModuleSurface::Zone)
    }

    async fn on_dial_press(&mut self) -> Vec<Command> {
        self.toggle_mute(Direction::Output, ModuleSurface::Zone)
    }

    async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
        match event.downcast::<AudioEvent>() {
            Ok(event) => self.handle(*event),
            Err(_) => Vec::new(),
        }
    }

    fn zone(&self, _nav: &NavigationView) -> Option<ZoneContent> {
        let content = match &self.view {
            View::Loading => ZoneContent::Lines(vec![MODULE_LABEL.into()]),
            View::Unavailable(fault) => ZoneContent::Lines(vec![unavailable_text(*fault).into()]),
            View::Ready(snapshot) => match snapshot.default_device(Direction::Output) {
                Some(sink) => ZoneContent::Gauge(gauge(sink, self.settings)),
                None => ZoneContent::Lines(vec![NO_OUTPUT_TEXT.into()]),
            },
        };
        Some(content)
    }

    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError> {
        self.settings = AudioSettings::parse(settings)?;
        Ok(())
    }
}

/// Cyan of the speaker icon, from its light to its deep end.
const VOLUME_ACCENT: GaugeAccent = GaugeAccent::new(Rgb(0x67, 0xE8, 0xF9), Rgb(0x0E, 0xA5, 0xE9));

/// Gauge of the default output, as an arc. The arc spans the reach of the
/// dial (0 to the ceiling), so that a full arc means « the dial goes no
/// further ».
fn gauge(sink: &AudioDevice, settings: AudioSettings) -> GaugeContent {
    let average = sink.volume.average();
    let value = if sink.muted {
        MUTED_VALUE.to_owned()
    } else {
        format!("{} %", percent_of(average))
    };
    let glyph = if sink.muted {
        &icons::GLYPH_MUTED
    } else {
        &icons::GLYPH
    };
    GaugeContent::new(&sink.description, value, level(average, settings.ceiling()))
        .with_shape(GaugeShape::Arc)
        .with_accent(VOLUME_ACCENT)
        .with_glyph(ImageData::clone(glyph))
        .with_dimmed(sink.muted)
}

/// `volume` out of `ceiling`, rounded to the nearest thousandth (volumes set
/// in whole percents fall just below them in PulseAudio units).
fn level(volume: u32, ceiling: u32) -> GaugeLevel {
    let full = u64::from(GaugeLevel::FULL_PERMILLE);
    let ceiling = u64::from(ceiling.max(1));
    let permille = (u64::from(volume).min(ceiling) * full + ceiling / 2) / ceiling;
    GaugeLevel::from_permille(u16::try_from(permille).unwrap_or(GaugeLevel::FULL_PERMILLE))
}

#[cfg(test)]
mod tests;
