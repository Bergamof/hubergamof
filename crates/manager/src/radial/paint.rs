//! Draws the radial menu into an image: the petals, the one under the pointer
//! lit, the icon and the label of each. The overlay shows the image; nothing
//! here talks to the screen.
//!
//! The drawing is an SVG document drawn by the renderer of the keys, so the
//! menu has the same text and the same icons as the deck.

use std::f64::consts::TAU;
use std::fmt::Write as _;

use image::{Rgba, RgbaImage, imageops};

use super::geometry::{Layout, Outline, Point};
use crate::render::{RenderError, TextRenderer, escape_xml, svg_color};

/// Space left around the ring so that nothing is cut at the edge, in pixels.
pub const MARGIN: f64 = 8.0;

/// Size of an icon, in pixels.
const ICON_SIZE: f64 = 36.0;
/// Font size of a label, in pixels.
const LABEL_SIZE: f64 = 13.0;
/// Space between an icon and its label, in pixels.
const LABEL_GAP: f64 = 6.0;
/// Smallest the menu is while it opens, as a fraction of its size.
const OPENING_SCALE: f64 = 0.85;

/// What one petal shows.
#[derive(Debug, Clone, Copy)]
pub struct PetalView<'a> {
    /// Name of the entry.
    pub label: &'a str,
    /// Icon, already rendered, if the entry has one. It is drawn on top of the
    /// petals: the renderer of the drawing reads no images.
    pub icon: Option<&'a RgbaImage>,
}

/// Colours of the menu.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// A petal.
    pub petal: Rgba<u8>,
    /// The petal under the pointer.
    pub lit: Rgba<u8>,
    /// Labels.
    pub text: Rgba<u8>,
}

/// The colours of the design: tea green petals, bergamot for the one lit.
pub const DEFAULT_PALETTE: Palette = Palette {
    petal: Rgba([0x1f, 0x33, 0x2c, 0xe6]),
    lit: Rgba([0xd9, 0xa4, 0x2b, 0xf2]),
    text: Rgba([0xff, 0xff, 0xff, 0xff]),
};

/// Side of the image that holds a menu of `layout`, in pixels.
#[must_use]
pub fn image_side(layout: &Layout) -> u32 {
    side_in_pixels(2.0 * (layout.ring().outer + MARGIN))
}

/// Draws the menu. `lit` is the petal under the pointer; `opening` goes from 0
/// (just opened) to 1 (fully open) when the menu is animated, and is 1 when it
/// is not.
///
/// # Errors
///
/// [`RenderError`] if the drawing cannot be parsed or rasterised.
pub fn paint(
    renderer: &TextRenderer,
    layout: &Layout,
    petals: &[PetalView<'_>],
    lit: Option<usize>,
    opening: f32,
    palette: Palette,
) -> Result<RgbaImage, RenderError> {
    let side = image_side(layout);
    let mut canvas = RgbaImage::new(side, side);
    let opening = f64::from(opening.clamp(0.0, 1.0));
    let middle = f64::from(side) / 2.0;
    let scale = OPENING_SCALE + (1.0 - OPENING_SCALE) * opening;
    let mut body = String::new();
    // Writing to a `String` cannot fail.
    let _ = write!(
        body,
        r#"<g transform="translate({middle} {middle}) scale({scale})" opacity="{opening}">"#
    );
    for (index, petal) in petals.iter().enumerate() {
        push_petal(&mut body, layout, index, lit == Some(index), palette);
        push_label(&mut body, layout, index, petal, palette);
    }
    body.push_str("</g>");
    renderer.overlay_svg(&mut canvas, &body)?;
    for (index, petal) in petals.iter().enumerate() {
        if let Some(icon) = petal.icon {
            overlay_icon(&mut canvas, layout, index, icon, scale, opening);
        }
    }
    Ok(canvas)
}

fn side_in_pixels(length: f64) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pixels = length.ceil() as u32;
    pixels
}

fn push_petal(body: &mut String, layout: &Layout, index: usize, lit: bool, palette: Palette) {
    let fill = if lit { palette.lit } else { palette.petal };
    let ring = layout.ring();
    let shape = layout.outline(index).map_or_else(
        || whole_ring(ring.inner, ring.outer),
        |outline| petal_path(&outline, ring.inner, ring.outer),
    );
    let _ = write!(
        body,
        r#"<path d="{shape}" fill="{}" fill-opacity="{}" fill-rule="evenodd"/>"#,
        svg_color(fill),
        opacity_of(fill),
    );
}

/// A petal: side, outer arc, side back, inner arc back.
fn petal_path(outline: &Outline, inner: f64, outer: f64) -> String {
    let Outline {
        inner_start,
        outer_start,
        outer_end,
        inner_end,
    } = *outline;
    format!(
        "M {} {} L {} {} A {outer} {outer} 0 0 1 {} {} L {} {} A {inner} {inner} 0 0 0 {} {} Z",
        inner_start.x,
        inner_start.y,
        outer_start.x,
        outer_start.y,
        outer_end.x,
        outer_end.y,
        inner_end.x,
        inner_end.y,
        inner_start.x,
        inner_start.y,
    )
}

/// The ring of a menu with one petal: two circles, the inner one a hole.
fn whole_ring(inner: f64, outer: f64) -> String {
    let circle = |radius: f64| {
        format!(
            "M {radius} 0 A {radius} {radius} 0 1 1 {} 0 A {radius} {radius} 0 1 1 {radius} 0 Z",
            -radius
        )
    };
    format!("{} {}", circle(outer), circle(inner))
}

/// Where the icon (if any) and the label of petal `index` go, from the middle
/// of the menu: the top of the icon and the baseline of the label.
fn content_places(layout: &Layout, index: usize, has_icon: bool) -> (Point, f64, f64) {
    let ring = layout.ring();
    let centre = petal_centre(layout, index, f64::midpoint(ring.inner, ring.outer));
    if has_icon {
        let top = centre.y - (ICON_SIZE + LABEL_GAP + LABEL_SIZE) / 2.0;
        (centre, top, top + ICON_SIZE + LABEL_GAP + LABEL_SIZE)
    } else {
        (centre, centre.y, centre.y + LABEL_SIZE / 3.0)
    }
}

fn push_label(
    body: &mut String,
    layout: &Layout,
    index: usize,
    petal: &PetalView<'_>,
    palette: Palette,
) {
    let (centre, _, baseline) = content_places(layout, index, petal.icon.is_some());
    let _ = write!(
        body,
        r#"<text x="{}" y="{baseline}" font-family="{}" font-size="{LABEL_SIZE}" font-weight="600" text-anchor="middle" fill="{}" fill-opacity="{}">"#,
        centre.x,
        crate::render::FONT_FAMILY,
        svg_color(palette.text),
        opacity_of(palette.text),
    );
    escape_xml(body, petal.label);
    body.push_str("</text>");
}

/// Draws `icon` where petal `index` has it, scaled and faded like the rest of
/// the menu while it opens.
fn overlay_icon(
    canvas: &mut RgbaImage,
    layout: &Layout,
    index: usize,
    icon: &RgbaImage,
    scale: f64,
    opening: f64,
) {
    let (centre, top, _) = content_places(layout, index, true);
    let side = side_in_pixels(ICON_SIZE * scale);
    if side == 0 {
        return;
    }
    let mut scaled = imageops::resize(icon, side, side, imageops::FilterType::Lanczos3);
    for pixel in scaled.pixels_mut() {
        pixel.0[3] = fade(pixel.0[3], opening);
    }
    let middle = f64::from(canvas.width()) / 2.0;
    let left = middle + (centre.x - ICON_SIZE / 2.0) * scale;
    let up = middle + top * scale;
    imageops::overlay(canvas, &scaled, pixel_offset(left), pixel_offset(up));
}

/// `alpha` weakened to `opening` (0 to 1) of what it was.
fn fade(alpha: u8, opening: f64) -> u8 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let faded = (f64::from(alpha) * opening).round() as u8;
    faded
}

fn pixel_offset(position: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    let offset = position.round() as i64;
    offset
}

/// The point of petal `index` at `radius`, on its middle line.
fn petal_centre(layout: &Layout, index: usize, radius: f64) -> Point {
    let step = TAU / count_of(layout);
    let angle = count_as_f64(index) * step;
    Point::new(radius * angle.sin(), -radius * angle.cos())
}

fn count_of(layout: &Layout) -> f64 {
    count_as_f64(layout.count())
}

/// Entries are counted in the single digits: the conversion loses nothing.
#[allow(clippy::cast_precision_loss)]
const fn count_as_f64(count: usize) -> f64 {
    count as f64
}

fn opacity_of(color: Rgba<u8>) -> f32 {
    f32::from(color.0[3]) / f32::from(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::radial::geometry::Ring;

    const RING: Ring = Ring {
        inner: 44.0,
        outer: 150.0,
        gap: 8.0,
    };
    const PETAL: Rgba<u8> = Rgba([40, 60, 50, 255]);
    const LIT: Rgba<u8> = Rgba([200, 150, 40, 255]);
    const PALETTE: Palette = Palette {
        petal: PETAL,
        lit: LIT,
        text: Rgba([255, 255, 255, 255]),
    };

    fn layout(count: usize) -> Layout {
        Layout::new(count, RING).unwrap()
    }

    fn views(count: usize) -> Vec<PetalView<'static>> {
        ["Pomodoro", "Teams", "Audio", "IntelliJ", "Docker", "Claude"]
            .into_iter()
            .cycle()
            .take(count)
            .map(|label| PetalView { label, icon: None })
            .collect()
    }

    /// The pixel `radius` from the middle, at petal `index`'s middle line.
    fn pixel(image: &RgbaImage, layout: &Layout, index: usize, radius: f64) -> Rgba<u8> {
        let spot = petal_centre(layout, index, radius);
        let middle = f64::from(image.width()) / 2.0;
        let (x, y) = (middle + spot.x, middle + spot.y);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let at = (x as u32, y as u32);
        *image.get_pixel(at.0, at.1)
    }

    fn paint_menu(count: usize, lit: Option<usize>, opening: f32) -> RgbaImage {
        let renderer = TextRenderer::new().unwrap();
        paint(
            &renderer,
            &layout(count),
            &views(count),
            lit,
            opening,
            PALETTE,
        )
        .unwrap()
    }

    #[test]
    fn the_image_holds_the_ring_and_a_margin() {
        let menu = layout(4);
        assert_eq!(image_side(&menu), 316);
        assert_eq!(paint_menu(4, None, 1.0).dimensions(), (316, 316));
    }

    #[test]
    fn the_petal_under_the_pointer_is_lit_and_the_others_are_not() {
        let image = paint_menu(4, Some(1), 1.0);
        let menu = layout(4);
        // Near the outer edge, clear of the icon and the label.
        assert_eq!(pixel(&image, &menu, 1, 140.0).0[..3], LIT.0[..3]);
        assert_eq!(pixel(&image, &menu, 0, 140.0).0[..3], PETAL.0[..3]);
        assert_eq!(pixel(&image, &menu, 2, 140.0).0[..3], PETAL.0[..3]);
    }

    #[test]
    fn no_petal_is_lit_when_the_pointer_is_in_the_void() {
        let image = paint_menu(4, None, 1.0);
        let menu = layout(4);
        for petal in 0..4 {
            assert_eq!(pixel(&image, &menu, petal, 140.0).0[..3], PETAL.0[..3]);
        }
    }

    #[test]
    fn the_hole_and_the_corners_of_the_image_stay_transparent() {
        let image = paint_menu(4, None, 1.0);
        assert_eq!(image.get_pixel(158, 158).0[3], 0, "the hole");
        assert_eq!(image.get_pixel(0, 0).0[3], 0, "a corner");
    }

    #[test]
    fn a_gap_between_two_petals_is_transparent() {
        let image = paint_menu(4, None, 1.0);
        // On the diagonal between the first and the second petal.
        let along = 100.0 / 2.0_f64.sqrt();
        let middle = 158.0;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let at = ((middle + along) as u32, (middle - along) as u32);
        assert_eq!(image.get_pixel(at.0, at.1).0[3], 0);
    }

    #[test]
    fn a_menu_that_has_not_started_to_open_is_invisible() {
        let image = paint_menu(4, None, 0.0);
        assert!(image.pixels().all(|pixel| pixel.0[3] == 0));
    }

    #[test]
    fn a_menu_that_is_opening_is_smaller_and_fainter_than_an_open_one() {
        let opening = paint_menu(4, None, 0.5);
        let open = paint_menu(4, None, 1.0);
        let weight = |image: &RgbaImage| {
            image
                .pixels()
                .map(|pixel| u64::from(pixel.0[3]))
                .sum::<u64>()
        };
        assert!(weight(&opening) > 0);
        assert!(weight(&opening) < weight(&open));
    }

    #[test]
    fn labels_are_drawn_and_text_with_markup_does_not_break_the_drawing() {
        let renderer = TextRenderer::new().unwrap();
        let petals = [
            PetalView {
                label: "<b>&\"'",
                icon: None,
            },
            PetalView {
                label: "a\u{1}b",
                icon: None,
            },
            PetalView {
                label: "Audio",
                icon: None,
            },
        ];
        let image = paint(&renderer, &layout(3), &petals, None, 1.0, PALETTE).unwrap();
        let white = image
            .pixels()
            .filter(|pixel| pixel.0[..3] == [255, 255, 255] && pixel.0[3] > 0)
            .count();
        assert!(white > 50, "the text is there: {white}");
    }

    #[test]
    fn a_single_petal_is_a_ring_with_a_hole() {
        let image = paint_menu(1, Some(0), 1.0);
        let menu = layout(1);
        assert_eq!(pixel(&image, &menu, 0, 140.0).0[..3], LIT.0[..3]);
        assert_eq!(image.get_pixel(158, 158).0[3], 0, "the hole");
    }

    #[test]
    fn an_icon_is_drawn_above_the_label() {
        let renderer = TextRenderer::new().unwrap();
        let red = RgbaImage::from_pixel(8, 8, Rgba([255, 0, 0, 255]));
        let petals: Vec<_> = views(4)
            .into_iter()
            .map(|view| PetalView {
                icon: Some(&red),
                ..view
            })
            .collect();
        let image = paint(&renderer, &layout(4), &petals, None, 1.0, PALETTE).unwrap();
        let reds = image
            .pixels()
            .filter(|pixel| pixel.0[0] > 200 && pixel.0[1] < 60 && pixel.0[3] > 200)
            .count();
        assert!(reds > 4 * 20 * 20, "four icons of about 36 px: {reds}");
    }
}
