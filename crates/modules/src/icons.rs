//! Helpers shared by the SVG icons of the built-in modules.

/// Source of the icon `assets/icons/<name>.svg`, embedded at compile time.
macro_rules! asset {
    ($name:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/icons/",
            $name,
            ".svg"
        ))
    };
}
pub(crate) use asset;

use hubergamof_module_api::ImageData;

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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    /// The icons are drawn over the surface of the device theme: none paints
    /// its own background.
    #[test]
    fn no_icon_paints_a_background() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
        let mut checked = 0;
        for entry in fs::read_dir(directory).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "svg") {
                continue;
            }
            let source = fs::read_to_string(&path).unwrap();
            assert!(
                !source.contains(r#"<rect width="128" height="128" rx="24""#),
                "{} paints a background",
                path.display()
            );
            checked += 1;
        }
        assert!(checked > 20, "{checked} icons found");
    }
}
