//! Helpers to build the SVG elements the zone renderers draw over a canvas.

use image::Rgba;

use super::source::svg_color;

/// Appends formatted SVG to a `String`. Writing to a `String` cannot fail,
/// hence the ignored result, stated once here.
macro_rules! push_svg {
    ($svg:expr, $($arg:tt)*) => {{
        use ::std::fmt::Write as _;
        let _ = write!($svg, $($arg)*);
    }};
}
pub(super) use push_svg;

/// A `<defs>` holding the diagonal gradient `id` from `from` to `to`, running
/// between two points of the canvas (`start` and `end`, in pixels): the shapes
/// that use it share one gradient, whatever their own size.
pub(super) fn canvas_gradient(
    id: &str,
    (from, to): (Rgba<u8>, Rgba<u8>),
    start: (f32, f32),
    end: (f32, f32),
) -> String {
    format!(
        r#"<defs><linearGradient id="{id}" gradientUnits="userSpaceOnUse" x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}">{stops}</linearGradient></defs>"#,
        x1 = start.0,
        y1 = start.1,
        x2 = end.0,
        y2 = end.1,
        stops = stops(from, to),
    )
}

/// A `<defs>` holding the diagonal gradient `id` from `from` to `to`, which
/// every shape using it spreads over its own bounding box.
pub(super) fn shape_gradient(id: &str, (from, to): (Rgba<u8>, Rgba<u8>)) -> String {
    format!(
        r#"<defs><linearGradient id="{id}" x1="0" y1="0" x2="1" y2="1">{stops}</linearGradient></defs>"#,
        stops = stops(from, to),
    )
}

fn stops(from: Rgba<u8>, to: Rgba<u8>) -> String {
    format!(
        r#"<stop offset="0" stop-color="{}"/><stop offset="1" stop-color="{}"/>"#,
        svg_color(from),
        svg_color(to),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);
    const BLUE: Rgba<u8> = Rgba([0, 0, 255, 255]);

    #[test]
    fn push_svg_appends_to_the_string() {
        let mut svg = String::from("<g>");
        push_svg!(svg, "<rect x=\"{}\"/>", 3);
        assert_eq!(svg, "<g><rect x=\"3\"/>");
    }

    #[test]
    fn gradients_run_from_the_first_to_the_second_colour() {
        let canvas = canvas_gradient("a", (RED, BLUE), (1.0, 2.0), (3.0, 4.0));
        assert!(
            canvas.contains(r#"id="a""#) && canvas.contains(r#"gradientUnits="userSpaceOnUse""#)
        );
        assert!(canvas.contains(r#"x1="1" y1="2" x2="3" y2="4""#));
        let red = canvas.find("#ff0000").unwrap();
        assert!(red < canvas.find("#0000ff").unwrap());
        let shape = shape_gradient("b", (RED, BLUE));
        assert!(shape.contains(r#"x2="1" y2="1""#) && !shape.contains("userSpaceOnUse"));
    }
}
