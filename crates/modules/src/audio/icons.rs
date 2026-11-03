//! Key icons of the audio module (SVG, drawn by the host).

use std::sync::LazyLock;

use bergadeck_module_api::ImageData;

use crate::icons::svg;

const SPEAKER_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 9h4l5-4v14l-5-4H4z" fill="#ffffff"/><path d="M16.5 8.5a5 5 0 0 1 0 7"/><path d="M19 6a8.5 8.5 0 0 1 0 12"/></svg>"##;

const SPEAKER_MUTED_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 9h4l5-4v14l-5-4H4z" fill="#ffffff"/><path d="M16 9l6 6M22 9l-6 6" stroke="#ff5a4f"/></svg>"##;

const MICROPHONE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="3" width="6" height="11" rx="3" fill="#ffffff"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21M8.5 21h7"/></svg>"##;

const MICROPHONE_MUTED_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="3" width="6" height="11" rx="3" fill="#ffffff"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21M8.5 21h7"/><path d="M4 4l16 16" stroke="#ff5a4f"/></svg>"##;

/// Speaker: the module, outputs.
pub(super) static SPEAKER: LazyLock<ImageData> = LazyLock::new(|| svg(SPEAKER_SVG));
/// Crossed speaker: muted output.
pub(super) static SPEAKER_MUTED: LazyLock<ImageData> = LazyLock::new(|| svg(SPEAKER_MUTED_SVG));
/// Microphone: inputs.
pub(super) static MICROPHONE: LazyLock<ImageData> = LazyLock::new(|| svg(MICROPHONE_SVG));
/// Struck microphone: muted input.
pub(super) static MICROPHONE_MUTED: LazyLock<ImageData> =
    LazyLock::new(|| svg(MICROPHONE_MUTED_SVG));
