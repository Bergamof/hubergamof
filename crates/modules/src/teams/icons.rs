//! Key icons of the Teams module (SVG, drawn by the host).

use std::sync::LazyLock;

use hubergamof_module_api::ImageData;

use crate::icons::{asset, svg};

/// Speech bubble: a conversation without avatar of its own (none, or not
/// readable).
pub(super) static CONVERSATION: LazyLock<ImageData> =
    LazyLock::new(|| svg(asset!("teams-conversation")));
/// Bell: the unread items of the Activity panel.
pub(super) static ACTIVITY: LazyLock<ImageData> = LazyLock::new(|| svg(asset!("teams-activity")));
/// Microphone: the microphone is on.
pub(super) static MICROPHONE: LazyLock<ImageData> =
    LazyLock::new(|| svg(asset!("audio-microphone")));
/// Struck microphone: the microphone is muted.
pub(super) static MICROPHONE_MUTED: LazyLock<ImageData> =
    LazyLock::new(|| svg(asset!("audio-microphone-muted")));
/// Camera: the camera is on, or its state is unknown.
pub(super) static CAMERA: LazyLock<ImageData> = LazyLock::new(|| svg(asset!("teams-camera")));
/// Struck camera: the camera is off.
pub(super) static CAMERA_OFF: LazyLock<ImageData> =
    LazyLock::new(|| svg(asset!("teams-camera-off")));
/// Red handset: leave or decline the call.
pub(super) static HANG_UP: LazyLock<ImageData> = LazyLock::new(|| svg(asset!("teams-hangup")));
/// Window with an arrow: back to the window of the call.
pub(super) static RETURN: LazyLock<ImageData> = LazyLock::new(|| svg(asset!("teams-return")));
/// Green handset: answer the call.
pub(super) static ACCEPT: LazyLock<ImageData> = LazyLock::new(|| svg(asset!("teams-accept")));

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::is_complete_svg;

    #[test]
    fn every_icon_is_a_complete_svg_document() {
        for icon in [
            &CONVERSATION,
            &ACTIVITY,
            &MICROPHONE,
            &MICROPHONE_MUTED,
            &CAMERA,
            &CAMERA_OFF,
            &HANG_UP,
            &ACCEPT,
            &RETURN,
        ] {
            let source = std::str::from_utf8(&icon.bytes).unwrap();
            assert!(is_complete_svg(source), "{source}");
        }
    }
}
