//! Helpers shared by the SVG icons of the built-in modules.

use bergadeck_module_api::ImageData;

/// Wraps the source of an SVG icon (drawn by the host).
pub(crate) fn svg(source: &str) -> ImageData {
    ImageData::new(source.as_bytes())
}

/// Whether `source` looks like a complete, self-contained SVG document, for
/// the tests of the icon constants.
#[cfg(test)]
pub(crate) fn is_complete_svg(source: &str) -> bool {
    source.starts_with("<svg ") && source.ends_with("</svg>") && !source.contains('$')
}
