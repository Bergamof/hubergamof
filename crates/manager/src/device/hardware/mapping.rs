//! Translation between `elgato-streamdeck` types and HuBergamof types. Pure
//! functions, tested without hardware.

use elgato_streamdeck::StreamDeckInput;
use elgato_streamdeck::info::Kind;
use hubergamof_protocol::{DeviceCapabilities, DeviceInfo, DeviceModel, ImageSize};

use crate::device::{InputEvent, TouchPoint};

/// HuBergamof model of a hardware kind.
#[must_use]
pub fn model_of(kind: Kind) -> DeviceModel {
    match kind {
        Kind::Original => DeviceModel::Original,
        Kind::OriginalV2 => DeviceModel::OriginalV2,
        Kind::Mini => DeviceModel::Mini,
        Kind::MiniMk2 => DeviceModel::MiniMk2,
        Kind::MiniDiscord => DeviceModel::MiniDiscord,
        Kind::MiniMk2Module => DeviceModel::MiniMk2Module,
        Kind::Xl => DeviceModel::Xl,
        Kind::XlV2 => DeviceModel::XlV2,
        Kind::XlV2Module => DeviceModel::XlV2Module,
        Kind::Mk2 => DeviceModel::Mk2,
        Kind::Mk2Scissor => DeviceModel::Mk2Scissor,
        Kind::Mk2Module => DeviceModel::Mk2Module,
        Kind::Mk2ModuleScissor => DeviceModel::Mk2ModuleScissor,
        Kind::Neo => DeviceModel::Neo,
        Kind::Pedal => DeviceModel::Pedal,
        Kind::Plus => DeviceModel::Plus,
        Kind::PlusXl => DeviceModel::PlusXl,
    }
}

/// Human-readable name of a hardware kind.
#[must_use]
pub fn display_name_of(kind: Kind) -> &'static str {
    match kind {
        Kind::Original | Kind::OriginalV2 => "Stream Deck",
        Kind::Mini | Kind::MiniMk2 => "Stream Deck Mini",
        Kind::MiniDiscord => "Stream Deck Mini Discord Edition",
        Kind::MiniMk2Module => "Stream Deck Mini Module",
        Kind::Xl | Kind::XlV2 => "Stream Deck XL",
        Kind::XlV2Module => "Stream Deck XL Module",
        Kind::Mk2 | Kind::Mk2Scissor => "Stream Deck MK.2",
        Kind::Mk2Module | Kind::Mk2ModuleScissor => "Stream Deck Module",
        Kind::Neo => "Stream Deck Neo",
        Kind::Pedal => "Stream Deck Pedal",
        Kind::Plus => "Stream Deck+",
        Kind::PlusXl => "Stream Deck+ XL",
    }
}

/// Capabilities of a hardware kind.
#[must_use]
pub fn capabilities_of(kind: Kind) -> DeviceCapabilities {
    let (key_width, key_height) = kind.key_image_format().size;
    DeviceCapabilities {
        key_columns: kind.column_count(),
        key_rows: kind.row_count(),
        key_image_size: image_size(key_width, key_height),
        dial_count: kind.encoder_count(),
        touch_strip: kind
            .lcd_strip_size()
            .map(|(width, height)| image_size(width, height)),
    }
}

/// Full description of a hardware device.
#[must_use]
pub fn device_info(kind: Kind, serial: &str) -> DeviceInfo {
    DeviceInfo {
        serial: serial.to_owned(),
        model: model_of(kind),
        display_name: display_name_of(kind).to_owned(),
        capabilities: capabilities_of(kind),
    }
}

fn image_size(width: usize, height: usize) -> ImageSize {
    // Device surfaces are at most a few thousand pixels wide.
    ImageSize {
        width: u32::try_from(width).unwrap_or(u32::MAX),
        height: u32::try_from(height).unwrap_or(u32::MAX),
    }
}

/// Turns the raw reports of `elgato-streamdeck` (full button/dial states)
/// into press/release events by remembering the previous state.
#[derive(Debug, Clone)]
pub struct InputTracker {
    key_count: usize,
    buttons: Vec<bool>,
    dials: Vec<bool>,
}

impl InputTracker {
    /// Tracker for a device with `key_count` keys, all released.
    ///
    /// Button states beyond `key_count` (Stream Deck Neo touch points) are
    /// not keys and are ignored.
    #[must_use]
    pub fn new(key_count: usize) -> Self {
        Self {
            key_count,
            buttons: Vec::new(),
            dials: Vec::new(),
        }
    }

    /// Events implied by one report.
    pub fn apply(&mut self, input: StreamDeckInput) -> Vec<InputEvent> {
        match input {
            StreamDeckInput::NoData => Vec::new(),
            StreamDeckInput::ButtonStateChange(states) => {
                let keys = &states[..states.len().min(self.key_count)];
                let events =
                    transitions(&self.buttons, keys, InputEvent::KeyDown, InputEvent::KeyUp);
                self.buttons = keys.to_vec();
                events
            }
            StreamDeckInput::EncoderStateChange(states) => {
                let events = transitions(
                    &self.dials,
                    &states,
                    InputEvent::DialDown,
                    InputEvent::DialUp,
                );
                self.dials = states;
                events
            }
            StreamDeckInput::EncoderTwist(changes) => rotations(&changes),
            StreamDeckInput::TouchScreenPress(x, y) => vec![InputEvent::TouchTap {
                point: TouchPoint { x, y },
                long: false,
            }],
            StreamDeckInput::TouchScreenLongPress(x, y) => vec![InputEvent::TouchTap {
                point: TouchPoint { x, y },
                long: true,
            }],
            StreamDeckInput::TouchScreenSwipe((from_x, from_y), (to_x, to_y)) => {
                vec![InputEvent::TouchSwipe {
                    from: TouchPoint {
                        x: from_x,
                        y: from_y,
                    },
                    to: TouchPoint { x: to_x, y: to_y },
                }]
            }
        }
    }
}

fn transitions(
    previous: &[bool],
    current: &[bool],
    down: fn(u8) -> InputEvent,
    up: fn(u8) -> InputEvent,
) -> Vec<InputEvent> {
    current
        .iter()
        .enumerate()
        .filter_map(|(index, &pressed)| {
            let was_pressed = previous.get(index).copied().unwrap_or(false);
            let index = u8::try_from(index).ok()?;
            match (was_pressed, pressed) {
                (false, true) => Some(down(index)),
                (true, false) => Some(up(index)),
                _ => None,
            }
        })
        .collect()
}

fn rotations(changes: &[i8]) -> Vec<InputEvent> {
    changes
        .iter()
        .enumerate()
        .filter(|(_, ticks)| **ticks != 0)
        .filter_map(|(index, &ticks)| {
            Some(InputEvent::DialRotate {
                index: u8::try_from(index).ok()?,
                ticks: i32::from(ticks),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_deck_plus_is_described_like_the_virtual_one() {
        let info = device_info(Kind::Plus, "CL123");
        assert_eq!(info.serial, "CL123");
        assert_eq!(info.model, DeviceModel::Plus);
        assert_eq!(info.display_name, "Stream Deck+");
        assert_eq!(
            info.capabilities,
            crate::device::virtual_deck::STREAM_DECK_PLUS_CAPABILITIES
        );
    }

    #[test]
    fn devices_without_dials_or_strip_report_none() {
        let capabilities = capabilities_of(Kind::Mk2);
        assert_eq!(capabilities.key_count(), 15);
        assert_eq!(capabilities.dial_count, 0);
        assert_eq!(capabilities.touch_strip, None);
    }

    #[test]
    fn button_reports_become_presses_and_releases() {
        let mut tracker = InputTracker::new(8);
        let mut states = vec![false; 8];
        states[2] = true;
        assert_eq!(
            tracker.apply(StreamDeckInput::ButtonStateChange(states.clone())),
            [InputEvent::KeyDown(2)]
        );
        states[5] = true;
        assert_eq!(
            tracker.apply(StreamDeckInput::ButtonStateChange(states.clone())),
            [InputEvent::KeyDown(5)]
        );
        states[2] = false;
        states[5] = false;
        assert_eq!(
            tracker.apply(StreamDeckInput::ButtonStateChange(states)),
            [InputEvent::KeyUp(2), InputEvent::KeyUp(5)]
        );
    }

    #[test]
    fn touch_point_buttons_beyond_keys_are_ignored() {
        let mut tracker = InputTracker::new(2);
        assert_eq!(
            tracker.apply(StreamDeckInput::ButtonStateChange(vec![false, false, true])),
            []
        );
    }

    #[test]
    fn encoder_reports_become_dial_events() {
        let mut tracker = InputTracker::new(8);
        assert_eq!(
            tracker.apply(StreamDeckInput::EncoderStateChange(vec![
                false, true, false, false
            ])),
            [InputEvent::DialDown(1)]
        );
        assert_eq!(
            tracker.apply(StreamDeckInput::EncoderStateChange(vec![false; 4])),
            [InputEvent::DialUp(1)]
        );
        assert_eq!(
            tracker.apply(StreamDeckInput::EncoderTwist(vec![0, -3, 0, 2])),
            [
                InputEvent::DialRotate {
                    index: 1,
                    ticks: -3
                },
                InputEvent::DialRotate { index: 3, ticks: 2 }
            ]
        );
    }

    #[test]
    fn touch_screen_reports_become_touch_events() {
        let mut tracker = InputTracker::new(8);
        assert_eq!(
            tracker.apply(StreamDeckInput::TouchScreenPress(250, 40)),
            [InputEvent::TouchTap {
                point: TouchPoint { x: 250, y: 40 },
                long: false
            }]
        );
        assert_eq!(
            tracker.apply(StreamDeckInput::TouchScreenLongPress(10, 5)),
            [InputEvent::TouchTap {
                point: TouchPoint { x: 10, y: 5 },
                long: true
            }]
        );
        assert_eq!(
            tracker.apply(StreamDeckInput::TouchScreenSwipe((700, 50), (100, 60))),
            [InputEvent::TouchSwipe {
                from: TouchPoint { x: 700, y: 50 },
                to: TouchPoint { x: 100, y: 60 }
            }]
        );
        assert_eq!(tracker.apply(StreamDeckInput::NoData), []);
    }
}
