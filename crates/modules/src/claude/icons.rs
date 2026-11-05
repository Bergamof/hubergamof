//! Key icon of the Claude module (SVG, drawn by the host). A generic
//! drawing: no trademark.

use std::sync::LazyLock;

use bergadeck_module_api::ImageData;

use crate::icons::svg;

/// A speech bubble with a spark: a conversation with an assistant.
const MODULE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 5h16a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H9l-4 3v-3H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z"/><path d="M12 8v5M9.5 10.5h5"/></svg>"##;

/// Module icon, on its key.
pub(super) static MODULE: LazyLock<ImageData> = LazyLock::new(|| svg(MODULE_SVG));

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::is_complete_svg;

    #[test]
    fn the_icon_is_a_complete_svg_document() {
        assert!(is_complete_svg(MODULE_SVG), "{MODULE_SVG}");
    }
}
