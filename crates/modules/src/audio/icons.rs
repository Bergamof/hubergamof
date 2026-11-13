//! Key icons of the audio module (SVG, drawn by the host).

use std::sync::LazyLock;

use hubergamof_module_api::ImageData;

use crate::icons::{asset, svg};

/// Speaker: the module, outputs.
pub(super) static SPEAKER: LazyLock<ImageData> = LazyLock::new(|| svg(asset!("audio-speaker")));
/// Crossed speaker: muted output.
pub(super) static SPEAKER_MUTED: LazyLock<ImageData> =
    LazyLock::new(|| svg(asset!("audio-speaker-muted")));
/// Microphone: an input.
pub(super) static MICROPHONE: LazyLock<ImageData> =
    LazyLock::new(|| svg(asset!("audio-microphone")));
/// Microphone with a list: choosing the default input.
pub(super) static INPUTS: LazyLock<ImageData> = LazyLock::new(|| svg(asset!("audio-inputs")));
/// Speaker with a list: choosing the default output.
pub(super) static OUTPUTS: LazyLock<ImageData> = LazyLock::new(|| svg(asset!("audio-outputs")));
/// Speaker with a green tick: the selected output.
pub(super) static SPEAKER_SELECTED: LazyLock<ImageData> =
    LazyLock::new(|| svg(asset!("audio-speaker-selected")));
/// Microphone with a green tick: the selected input.
pub(super) static MICROPHONE_SELECTED: LazyLock<ImageData> =
    LazyLock::new(|| svg(asset!("audio-microphone-selected")));
/// Struck microphone: muted input.
pub(super) static MICROPHONE_MUTED: LazyLock<ImageData> =
    LazyLock::new(|| svg(asset!("audio-microphone-muted")));
