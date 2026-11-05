//! Key icons of the Docker module (SVG, drawn by the host). Generic
//! drawings: no trademark.

use std::sync::LazyLock;

use hubergamof_module_api::ImageData;

use crate::icons::svg;

/// Stroke shared by the icons.
macro_rules! icon {
    ($body:expr) => {
        concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">"##,
            $body,
            "</svg>"
        )
    };
}

/// A box, the shape of a container, with a status dot of `$color`.
macro_rules! container {
    ($color:literal) => {
        icon!(concat!(
            r##"<path d="M3 8l9-4 9 4v8l-9 4-9-4z"/><path d="M3 8l9 4 9-4M12 12v8"/><circle cx="19" cy="19" r="3.2" fill=""##,
            $color,
            r##"" stroke="none"/>"##
        ))
    };
}

const MODULE_SVG: &str =
    icon!(r#"<path d="M3 8l9-4 9 4v8l-9 4-9-4z"/><path d="M3 8l9 4 9-4M12 12v8"/>"#);
const PROJECT_SVG: &str = icon!(
    r#"<path d="M12 3l9 4.5-9 4.5-9-4.5z"/><path d="M3 12l9 4.5 9-4.5"/><path d="M3 16.5l9 4.5 9-4.5"/>"#
);
const RUNNING_SVG: &str = container!("#3fbf6a");
const STOPPED_SVG: &str = container!("#8a8f98");
const ATTENTION_SVG: &str = container!("#ff9f2e");
const START_SVG: &str = icon!(r##"<path d="M8 5l11 7-11 7z" fill="#ffffff"/>""##);
const STOP_SVG: &str =
    icon!(r##"<rect x="6" y="6" width="12" height="12" rx="1.5" fill="#ffffff"/>""##);
const RESTART_SVG: &str = icon!(r#"<path d="M20 12a8 8 0 1 1-2.3-5.7"/><path d="M20 4v5h-5"/>"#);
const REMOVE_SVG: &str =
    icon!(r#"<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/><path d="M10 11v6M14 11v6"/>"#);
const UP_SVG: &str = icon!(r#"<path d="M12 20V5M5 12l7-7 7 7"/>"#);
const DOWN_SVG: &str = icon!(r#"<path d="M12 4v15M5 12l7 7 7-7"/>"#);

/// The module: a container.
pub(super) static MODULE: LazyLock<ImageData> = LazyLock::new(|| svg(MODULE_SVG));
/// A compose project: stacked layers.
pub(super) static PROJECT: LazyLock<ImageData> = LazyLock::new(|| svg(PROJECT_SVG));
/// A running container.
pub(super) static RUNNING: LazyLock<ImageData> = LazyLock::new(|| svg(RUNNING_SVG));
/// A container that is not running.
pub(super) static STOPPED: LazyLock<ImageData> = LazyLock::new(|| svg(STOPPED_SVG));
/// A container in trouble (failing health check, restart loop, dead).
pub(super) static ATTENTION: LazyLock<ImageData> = LazyLock::new(|| svg(ATTENTION_SVG));
/// Start, start all.
pub(super) static START: LazyLock<ImageData> = LazyLock::new(|| svg(START_SVG));
/// Stop, stop all.
pub(super) static STOP: LazyLock<ImageData> = LazyLock::new(|| svg(STOP_SVG));
/// Restart.
pub(super) static RESTART: LazyLock<ImageData> = LazyLock::new(|| svg(RESTART_SVG));
/// Remove.
pub(super) static REMOVE: LazyLock<ImageData> = LazyLock::new(|| svg(REMOVE_SVG));
/// `compose up`.
pub(super) static UP: LazyLock<ImageData> = LazyLock::new(|| svg(UP_SVG));
/// `compose down`.
pub(super) static DOWN: LazyLock<ImageData> = LazyLock::new(|| svg(DOWN_SVG));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_complete_svg() {
        for icon in [
            MODULE_SVG,
            PROJECT_SVG,
            RUNNING_SVG,
            STOPPED_SVG,
            ATTENTION_SVG,
            START_SVG,
            STOP_SVG,
            RESTART_SVG,
            REMOVE_SVG,
            UP_SVG,
            DOWN_SVG,
        ] {
            assert!(crate::icons::is_complete_svg(icon), "{icon}");
        }
        assert!(RUNNING_SVG.contains("fill=\"#3fbf6a\""));
    }
}
