//! Key icons of the audio module (SVG, drawn by the host).

use std::sync::LazyLock;

use hubergamof_module_api::ImageData;

use crate::icons::{asset, svg};

/// Speaker: the module, outputs.
pub(super) static SPEAKER: LazyLock<ImageData> = LazyLock::new(|| svg(asset!("audio-speaker")));
/// Crossed speaker: muted output.
pub(super) static SPEAKER_MUTED: LazyLock<ImageData> =
    LazyLock::new(|| svg(asset!("audio-speaker-muted")));
/// Microphone: inputs.
pub(super) static MICROPHONE: LazyLock<ImageData> =
    LazyLock::new(|| svg(asset!("audio-microphone")));
/// Struck microphone: muted input.
pub(super) static MICROPHONE_MUTED: LazyLock<ImageData> =
    LazyLock::new(|| svg(asset!("audio-microphone-muted")));
