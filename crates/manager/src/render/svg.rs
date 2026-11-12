//! Helper to build the SVG elements the strip renderer draws over a canvas.

/// Appends formatted SVG to a `String`. Writing to a `String` cannot fail,
/// hence the ignored result, stated once here.
macro_rules! push_svg {
    ($svg:expr, $($arg:tt)*) => {{
        use ::std::fmt::Write as _;
        let _ = write!($svg, $($arg)*);
    }};
}
pub(super) use push_svg;

#[cfg(test)]
mod tests {
    #[test]
    fn push_svg_appends_to_the_string() {
        let mut svg = String::from("<g>");
        push_svg!(svg, "<rect x=\"{}\"/>", 3);
        assert_eq!(svg, "<g><rect x=\"3\"/>");
    }
}
