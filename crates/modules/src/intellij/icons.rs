//! Key icons of the IntelliJ module (SVG, drawn by the host).

use std::sync::LazyLock;

use hubergamof_module_api::ImageData;

use crate::icons::{asset, svg};

const MODULE_SVG: &str = asset!("intellij");
const PROJECT_SVG: &str = asset!("intellij-project");

/// Module icon, on its home key.
pub(super) static MODULE: LazyLock<ImageData> = LazyLock::new(|| svg(MODULE_SVG));

/// A project, on the projects page.
pub(super) static PROJECT: LazyLock<ImageData> = LazyLock::new(|| svg(PROJECT_SVG));

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::is_complete_svg;

    #[test]
    fn icons_are_complete_svg_documents() {
        for source in [MODULE_SVG, PROJECT_SVG] {
            assert!(is_complete_svg(source), "{source}");
        }
    }
}
