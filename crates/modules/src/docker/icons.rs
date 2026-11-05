//! Key icons of the Docker module (SVG, drawn by the host): the whale for the
//! module, a cube per container, stacked layers per compose project.

use std::sync::LazyLock;

use hubergamof_module_api::ImageData;

use crate::icons::{asset, svg};

const MODULE_SVG: &str = asset!("docker");
const RUNNING_SVG: &str = asset!("docker-container-running");
const STOPPED_SVG: &str = asset!("docker-container-stopped");
const ATTENTION_SVG: &str = asset!("docker-container-attention");
const PROJECT_RUNNING_SVG: &str = asset!("docker-project-running");
const PROJECT_STOPPED_SVG: &str = asset!("docker-project-stopped");
const PROJECT_ATTENTION_SVG: &str = asset!("docker-project-attention");
const START_SVG: &str = asset!("docker-start");
const STOP_SVG: &str = asset!("docker-stop");
const RESTART_SVG: &str = asset!("docker-restart");
const REMOVE_SVG: &str = asset!("docker-remove");
const UP_SVG: &str = asset!("docker-up");
const DOWN_SVG: &str = asset!("docker-down");

/// The module: the Docker whale.
pub(super) static MODULE: LazyLock<ImageData> = LazyLock::new(|| svg(MODULE_SVG));
/// A running container.
pub(super) static RUNNING: LazyLock<ImageData> = LazyLock::new(|| svg(RUNNING_SVG));
/// A container that is not running.
pub(super) static STOPPED: LazyLock<ImageData> = LazyLock::new(|| svg(STOPPED_SVG));
/// A container in trouble (failing health check, restart loop, dead).
pub(super) static ATTENTION: LazyLock<ImageData> = LazyLock::new(|| svg(ATTENTION_SVG));
/// A compose project with running containers.
pub(super) static PROJECT_RUNNING: LazyLock<ImageData> = LazyLock::new(|| svg(PROJECT_RUNNING_SVG));
/// A compose project with no running container.
pub(super) static PROJECT_STOPPED: LazyLock<ImageData> = LazyLock::new(|| svg(PROJECT_STOPPED_SVG));
/// A compose project in trouble.
pub(super) static PROJECT_ATTENTION: LazyLock<ImageData> =
    LazyLock::new(|| svg(PROJECT_ATTENTION_SVG));
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
            RUNNING_SVG,
            STOPPED_SVG,
            ATTENTION_SVG,
            PROJECT_RUNNING_SVG,
            PROJECT_STOPPED_SVG,
            PROJECT_ATTENTION_SVG,
            START_SVG,
            STOP_SVG,
            RESTART_SVG,
            REMOVE_SVG,
            UP_SVG,
            DOWN_SVG,
        ] {
            assert!(crate::icons::is_complete_svg(icon), "{icon}");
        }
    }
}
