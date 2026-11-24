//! Headphones module: the ambient sound of Sony headphones on a key (press:
//! next mode), on a dial (turn: ambient level, press: next mode) and in the
//! radial menu (one petal action per mode).
//!
//! The headphones are reached through a [`HeadsetControl`]: the manager's hub
//! of the headphones, or a simulation ([`SimulatedHeadset`]). The module asks
//! and the headphones confirm: what is shown is what they said, but a request
//! is remembered until they answer so that a dial turned fast adds up.
//! Headphones serve one settings client at a time, so the module opens the
//! link only when nobody opened it yet; closed on purpose or cut, it stays
//! closed until the person resumes it on the Headphones page, and the
//! surface shows ⚠.

mod backend;
mod icons;
mod simulated;

use std::sync::Arc;

use async_trait::async_trait;
use hubergamof_module_api::{
    Command, DialFunctionDescriptor, DialFunctionId, DialHint, HapticEventDescriptor,
    HapticWaveform, ImageData, KeyContent, Module, ModuleCapabilities, ModuleContext,
    ModuleDescriptor, ModuleError, ModuleEvent, ModuleFactory, ModuleLifetime,
    RadialActionDescriptor, RadialGlyph, RadialItem, RadialItemState,
};
use serde_json::Value;

pub use backend::{
    Ambient, Headset, HeadsetControl, HeadsetFault, Link, MAX_LEVEL, Mode, NoHeadset,
};
pub use simulated::SimulatedHeadset;

use crate::background::forward_states;

/// Kind of the headphones module.
pub const HEADPHONES_KIND: &str = "headphones";

/// Dial function: turning changes the ambient level, pressing the mode.
pub const AMBIENT_FUNCTION: &str = "ambient";

const MODULE_LABEL: &str = "Casque";
const ROTATE_HINT: &str = "Niveau ambiant";
const PRESS_HINT: &str = "Mode suivant";

/// Radial actions of the module (`on_radial_action`).
const RADIAL_NEXT_MODE: &str = "next-mode";
const RADIAL_NOISE_CANCELLING: &str = "noise-cancelling";
const RADIAL_AMBIENT: &str = "ambient";
const RADIAL_OFF: &str = "off";

/// Haptic event raised when the mode is changed from the radial menu.
const HAPTIC_MODE_CHANGED: &str = "mode-changed";

/// What a mode is called.
fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::NoiseCancelling => "Réduction de bruit",
        Mode::Ambient => "Son ambiant",
        Mode::WindReduction => "Réduction du vent",
        Mode::Off => "Normal",
    }
}

/// The mode after `mode`: noise cancelling, ambient, wind reduction when the
/// headphones have it, off.
fn next_mode(mode: Mode, wind_reduction: bool) -> Mode {
    match mode {
        Mode::NoiseCancelling => Mode::Ambient,
        Mode::Ambient if wind_reduction => Mode::WindReduction,
        Mode::Ambient | Mode::WindReduction => Mode::Off,
        Mode::Off => Mode::NoiseCancelling,
    }
}

/// A petal action's commands, and a vibration when the request was made (the
/// mouse confirms what the hand did, unless it did not happen).
fn with_vibration(mut commands: Vec<Command>) -> Vec<Command> {
    if commands != Command::alert() {
        commands.push(Command::haptic(HAPTIC_MODE_CHANGED));
    }
    commands
}

/// Creates [`Headphones`] instances, all following the same headphones.
#[derive(Debug, Clone)]
pub struct HeadphonesFactory {
    control: Arc<dyn HeadsetControl>,
}

impl HeadphonesFactory {
    /// Factory driving `control`.
    #[must_use]
    pub fn new(control: Arc<dyn HeadsetControl>) -> Self {
        Self { control }
    }
}

fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        kind: HEADPHONES_KIND.into(),
        name: "Casque".into(),
        description: "Son ambiant du casque Sony : la touche passe au mode suivant, la \
                      molette règle le niveau ambiant. Ouvrez la page Casque pour \
                      connecter le casque."
            .into(),
        capabilities: ModuleCapabilities {
            key: true,
            ..Default::default()
        },
        dial_functions: vec![DialFunctionDescriptor {
            id: AMBIENT_FUNCTION.into(),
            name: "Son ambiant".into(),
            description: "Tourner règle le niveau ambiant, appuyer passe au mode suivant.".into(),
        }],
        settings: Vec::new(),
    }
}

impl ModuleFactory for HeadphonesFactory {
    /// One instance for the whole application.
    fn lifetime(&self) -> ModuleLifetime {
        ModuleLifetime::Application
    }

    fn descriptor(&self) -> ModuleDescriptor {
        descriptor()
    }

    fn haptic_events(&self) -> Vec<HapticEventDescriptor> {
        vec![HapticEventDescriptor::new(
            HAPTIC_MODE_CHANGED,
            "Mode du casque changé",
            "Le son ambiant du casque est changé depuis le menu circulaire.",
            HapticWaveform::SharpStateChange,
        )]
    }

    fn radial_actions(&self) -> Vec<RadialActionDescriptor> {
        vec![
            RadialActionDescriptor::new(
                RADIAL_NEXT_MODE,
                "Casque : mode suivant",
                "Passe de la réduction de bruit au son ambiant, puis au mode normal.",
            ),
            RadialActionDescriptor::new(
                RADIAL_NOISE_CANCELLING,
                "Casque : réduction de bruit",
                "Active la réduction de bruit.",
            ),
            RadialActionDescriptor::new(
                RADIAL_AMBIENT,
                "Casque : son ambiant",
                "Laisse passer le son extérieur.",
            ),
            RadialActionDescriptor::new(
                RADIAL_OFF,
                "Casque : mode normal",
                "Coupe la réduction de bruit et le son ambiant.",
            ),
        ]
    }

    fn icon(&self) -> Option<ImageData> {
        Some(icons::HEADPHONES.clone())
    }

    fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
        descriptor().validate_settings(settings)?;
        let states = self.control.watch();
        let headset = states.borrow().clone();
        ctx.tasks().spawn(forward_states(
            states,
            ctx.events().clone(),
            HeadsetEvent::State,
        ));
        Ok(Box::new(Headphones {
            control: Arc::clone(&self.control),
            headset,
            asked: None,
        }))
    }
}

/// What the headphones told the instance.
#[derive(Debug)]
enum HeadsetEvent {
    State(Option<Headset>),
}

/// One instance: what the headphones said, and what was asked of them since.
pub struct Headphones {
    control: Arc<dyn HeadsetControl>,
    headset: Option<Headset>,
    /// The last ambient sound asked for, until the headphones say something:
    /// the next change starts from it, not from a state they did not reach yet.
    asked: Option<Ambient>,
}

impl Headphones {
    /// The ambient sound to start a change from.
    fn ambient(&self) -> Option<Ambient> {
        self.asked
            .or_else(|| self.headset.as_ref().and_then(|headset| headset.ambient))
    }

    fn wind_reduction(&self) -> bool {
        self.headset
            .as_ref()
            .is_some_and(|headset| headset.wind_reduction)
    }

    /// Asks for `change` of the ambient sound known; ⚠ when there is none to
    /// start from or the headphones cannot be set (and the link is opened if
    /// nobody did).
    fn request(&mut self, change: impl FnOnce(Ambient) -> Ambient) -> Vec<Command> {
        let Some(ambient) = self.ambient() else {
            self.control.open();
            return Command::alert();
        };
        let changed = change(ambient);
        if changed == ambient {
            return Vec::new();
        }
        match self.control.set_ambient(changed) {
            Ok(()) => {
                self.asked = Some(changed);
                Vec::new()
            }
            Err(HeadsetFault::NotLinked) => {
                self.control.open();
                Command::alert()
            }
            Err(HeadsetFault::Refused) => Command::alert(),
        }
    }

    fn set_mode(&mut self, mode: Mode) -> Vec<Command> {
        self.request(|ambient| Ambient { mode, ..ambient })
    }

    fn cycle_mode(&mut self) -> Vec<Command> {
        let wind_reduction = self.wind_reduction();
        self.request(|ambient| Ambient {
            mode: next_mode(ambient.mode, wind_reduction),
            ..ambient
        })
    }

    fn label(&self) -> String {
        self.ambient()
            .map_or(MODULE_LABEL, |ambient| mode_label(ambient.mode))
            .to_owned()
    }
}

#[async_trait]
impl Module for Headphones {
    fn key(&self) -> KeyContent {
        KeyContent::with_icon(&icons::HEADPHONES, self.label())
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        self.cycle_mode()
    }

    async fn on_radial_action(&mut self, action: &str) -> Vec<Command> {
        let commands = match action {
            RADIAL_NEXT_MODE => self.cycle_mode(),
            RADIAL_NOISE_CANCELLING => self.set_mode(Mode::NoiseCancelling),
            RADIAL_AMBIENT => self.set_mode(Mode::Ambient),
            RADIAL_OFF => self.set_mode(Mode::Off),
            _ => return Vec::new(),
        };
        with_vibration(commands)
    }

    fn radial_items(&self) -> Vec<RadialItem> {
        let Some(current) = self.ambient().map(|ambient| ambient.mode) else {
            return Vec::new();
        };
        let items = [
            (
                Mode::NoiseCancelling,
                RADIAL_NOISE_CANCELLING,
                RadialGlyph::Headphones,
                RadialItemState::Normal,
            ),
            (
                Mode::Ambient,
                RADIAL_AMBIENT,
                RadialGlyph::Ear,
                RadialItemState::Normal,
            ),
            (
                Mode::Off,
                RADIAL_OFF,
                RadialGlyph::Headphones,
                RadialItemState::Off,
            ),
        ];
        items
            .into_iter()
            .filter(|(mode, ..)| *mode != current)
            .map(|(mode, id, glyph, state)| {
                RadialItem::new(id, mode_label(mode), glyph).with_state(state)
            })
            .collect()
    }

    fn radial_status(&self) -> Option<String> {
        let ambient = self.ambient()?;
        Some(match ambient.mode {
            Mode::Ambient => format!("{} {}", mode_label(ambient.mode), ambient.level),
            mode => mode_label(mode).to_owned(),
        })
    }

    async fn on_dial_rotate(&mut self, function: &DialFunctionId, ticks: i32) -> Vec<Command> {
        if function.0 != AMBIENT_FUNCTION {
            return Vec::new();
        }
        self.request(|ambient| {
            let level = (i32::from(ambient.level) + ticks).clamp(0, i32::from(MAX_LEVEL));
            Ambient {
                // Turning is about the outside sound: it makes it heard.
                mode: if ambient.mode == Mode::WindReduction {
                    Mode::WindReduction
                } else {
                    Mode::Ambient
                },
                level: u8::try_from(level).unwrap_or(MAX_LEVEL),
                ..ambient
            }
        })
    }

    async fn on_dial_press(&mut self, function: &DialFunctionId) -> Vec<Command> {
        if function.0 != AMBIENT_FUNCTION {
            return Vec::new();
        }
        self.cycle_mode()
    }

    fn dial_hint(&self, function: &DialFunctionId) -> DialHint {
        if function.0 != AMBIENT_FUNCTION {
            return DialHint::none();
        }
        DialHint::none().rotating(ROTATE_HINT).pressing(PRESS_HINT)
    }

    async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
        match event.downcast::<HeadsetEvent>() {
            Ok(event) => {
                let HeadsetEvent::State(headset) = *event;
                self.headset = headset;
                self.asked = None;
                Vec::new()
            }
            Err(_) => Vec::new(),
        }
    }

    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError> {
        descriptor().validate_settings(settings)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
