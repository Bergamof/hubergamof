//! Key icons of the IntelliJ module (SVG, drawn by the host). Generic
//! drawings: no trademark.

use std::sync::LazyLock;

use bergadeck_module_api::ImageData;

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

/// A code editor: a window with angle brackets.
const MODULE_SVG: &str = icon!(
    r#"<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 8h18"/><path d="M10 12l-2.5 2.5L10 17M14 12l2.5 2.5L14 17"/>"#
);

/// A folder.
const PROJECT_SVG: &str = icon!(
    r#"<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>"#
);

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
