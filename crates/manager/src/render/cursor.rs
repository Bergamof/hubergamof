//! What the host draws over a key for the contextual dial: the frame of the
//! cursor, around the key.

use image::{Rgba, RgbaImage, imageops};

use super::source::f32_from;

/// Thickness of the frame, relative to the smaller side of the key.
const THICKNESS_RATIO: f32 = 0.06;
/// Radius of the outer corners, relative to the smaller side of the key.
const RADIUS_RATIO: f32 = 0.14;

/// Draws a rounded frame of `color` along the edge of `canvas`, over what it
/// shows. The edges are smoothed over one pixel.
pub(super) fn draw_frame(canvas: &mut RgbaImage, color: Rgba<u8>) {
    let (width, height) = canvas.dimensions();
    let side = f32_from(width.min(height));
    let thickness = side * THICKNESS_RATIO;
    let radius = side * RADIUS_RATIO;
    let outer = Shape::new(width, height, 0.0, radius);
    let inner = Shape::new(width, height, thickness, (radius - thickness).max(0.0));
    let layer = RgbaImage::from_fn(width, height, |x, y| {
        let at = (f32_from(x) + 0.5, f32_from(y) + 0.5);
        let covered = outer.coverage(at) * (1.0 - inner.coverage(at));
        Rgba([
            color.0[0],
            color.0[1],
            color.0[2],
            opacity(covered, color.0[3]),
        ])
    });
    imageops::overlay(canvas, &layer, 0, 0);
}

/// A rounded rectangle centred on a canvas.
struct Shape {
    centre: (f32, f32),
    /// Half sizes of the rectangle the corners are rounded from.
    core: (f32, f32),
    radius: f32,
}

impl Shape {
    /// The rounded rectangle of `width` × `height` shrunk by `inset` on each
    /// side, with corners of `radius`.
    fn new(width: u32, height: u32, inset: f32, radius: f32) -> Self {
        let (half_width, half_height) = (f32_from(width) / 2.0, f32_from(height) / 2.0);
        Self {
            centre: (half_width, half_height),
            core: (half_width - inset - radius, half_height - inset - radius),
            radius,
        }
    }

    /// How much of the pixel centred on `at` the shape covers, `0.0..=1.0`.
    fn coverage(&self, at: (f32, f32)) -> f32 {
        let beyond_x = (at.0 - self.centre.0).abs() - self.core.0;
        let beyond_y = (at.1 - self.centre.1).abs() - self.core.1;
        let outside = beyond_x.max(0.0).hypot(beyond_y.max(0.0));
        let inside = beyond_x.max(beyond_y).min(0.0);
        let distance = outside + inside - self.radius;
        (0.5 - distance).clamp(0.0, 1.0)
    }
}

/// `coverage` of an `alpha` channel, as a channel value.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn opacity(coverage: f32, alpha: u8) -> u8 {
    // Both factors are within 0..=1 and 0..=255: the product fits a u8.
    (coverage * f32_from(u32::from(alpha))).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS: u32 = 100;
    const BLUE: Rgba<u8> = Rgba([0, 0, 255, 255]);
    const BLACK: Rgba<u8> = Rgba([0, 0, 0, 255]);

    fn framed() -> RgbaImage {
        let mut canvas = RgbaImage::from_pixel(CANVAS, CANVAS, BLACK);
        draw_frame(&mut canvas, BLUE);
        canvas
    }

    #[test]
    fn the_frame_runs_along_the_edge_and_leaves_the_middle_alone() {
        let canvas = framed();
        for (x, y) in [(50, 1), (50, 98), (1, 50), (98, 50)] {
            assert_eq!(*canvas.get_pixel(x, y), BLUE, "edge at {x},{y}");
        }
        assert_eq!(*canvas.get_pixel(50, 50), BLACK);
        assert_eq!(*canvas.get_pixel(50, 12), BLACK, "well inside the frame");
    }

    #[test]
    fn the_corners_are_rounded() {
        let canvas = framed();
        assert_eq!(*canvas.get_pixel(0, 0), BLACK);
        assert_eq!(*canvas.get_pixel(CANVAS - 1, CANVAS - 1), BLACK);
    }

    #[test]
    fn the_frame_follows_a_key_of_any_size_and_shape() {
        for (width, height) in [(72, 72), (80, 80), (96, 96), (120, 60)] {
            let mut canvas = RgbaImage::from_pixel(width, height, BLACK);
            draw_frame(&mut canvas, BLUE);
            assert_eq!(canvas.dimensions(), (width, height));
            // The middle of the top edge is framed whatever the size.
            assert_eq!(*canvas.get_pixel(width / 2, 0), BLUE, "{width}x{height}");
        }
        // A canvas too small for a frame is only drawn on, never refused.
        for side in [1, 4] {
            let mut canvas = RgbaImage::from_pixel(side, side, BLACK);
            draw_frame(&mut canvas, BLUE);
            assert_eq!(canvas.dimensions(), (side, side));
        }
        let mut wide = RgbaImage::from_pixel(120, 60, BLACK);
        draw_frame(&mut wide, BLUE);
        assert_eq!(*wide.get_pixel(60, 59), BLUE, "bottom edge");
        assert_eq!(*wide.get_pixel(60, 30), BLACK, "inside");
    }

    #[test]
    fn a_translucent_frame_lets_the_key_show_through() {
        let mut canvas = RgbaImage::from_pixel(CANVAS, CANVAS, BLACK);
        draw_frame(&mut canvas, Rgba([0, 0, 255, 128]));
        let edge = canvas.get_pixel(50, 1).0;
        assert!(
            edge[2] > 100 && edge[2] < 155,
            "{edge:?}: half blue over black"
        );
        assert!(edge[3] >= 254, "{edge:?}: still opaque");
    }

    #[test]
    fn a_frame_on_a_transparent_canvas_stays_transparent_elsewhere() {
        let mut canvas = RgbaImage::from_pixel(CANVAS, CANVAS, Rgba([0, 0, 0, 0]));
        draw_frame(&mut canvas, BLUE);
        assert_eq!(*canvas.get_pixel(50, 1), BLUE);
        assert_eq!(canvas.get_pixel(50, 50).0[3], 0);
    }
}
