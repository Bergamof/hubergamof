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
/// Font size of a line of status under the label, in pixels.
const STATUS_SIZE: f64 = 11.0;
/// Distance from one line of status (or the label) to the next, in pixels.
const STATUS_LINE: f64 = 14.0;
/// Most lines of status a petal shows: more would not fit in it.
const MAX_STATUS_LINES: usize = 2;
/// Most characters of a line of status that are drawn: a petal is about a
/// hundred pixels wide, and a module that says more is not allowed to spill.
const MAX_STATUS_CHARS: usize = 24;
/// Height of the badge on an icon (a count), in pixels.
const BADGE_HEIGHT: f64 = 16.0;
/// Width of a character of a badge, and the room left around its characters,
/// in pixels: the badge widens with the count it shows.
const BADGE_CHARACTER_WIDTH: f64 = 7.0;
const BADGE_PADDING: f64 = 7.0;
/// Font size of the count of a badge, in pixels.
const BADGE_FONT_SIZE: f64 = 11.0;
/// From how many a badge shows `99+` instead of the count.
const BADGE_CAP: u32 = 99;
/// How far a badge sticks out past the top right corner of the icon, in
/// pixels.
const BADGE_OVERHANG: f64 = 4.0;
/// Red of a badge, whether or not its petal is lit; its white text reads
/// at about 5:1 on it.
const BADGE_FILL: &str = "#c8452d";
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
    /// What the module says under the name, lines separated by `\n`.
    pub status: Option<&'a str>,
    /// A count to show as a badge on the icon; none or zero shows none.
    pub badge: Option<u32>,
}

impl PetalView<'_> {
    /// The text of the badge: the count, `99+` from a hundred, nothing for
    /// none or zero.
    fn badge_text(&self) -> Option<String> {
        match self.badge? {
            0 => None,
            count if count > BADGE_CAP => Some(format!("{BADGE_CAP}+")),
            count => Some(count.to_string()),
        }
    }

    /// The lines of the status that are drawn, cut to fit the petal.
    fn status_lines(&self) -> Vec<String> {
        self.status
            .into_iter()
            .flat_map(str::lines)
            .take(MAX_STATUS_LINES)
            .map(|line| line.chars().take(MAX_STATUS_CHARS).collect())
            .collect()
    }
}

/// An icon to draw on a petal, and how many lines of status share the petal
/// with it.
struct Icon<'a> {
    image: &'a RgbaImage,
    status_lines: usize,
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
    /// The label of the petal under the pointer: dark, white on bergamot is
    /// not readable (about 2.3:1).
    pub lit_text: Rgba<u8>,
}

/// The colours of the design: tea green petals, bergamot for the one lit.
pub const DEFAULT_PALETTE: Palette = Palette {
    petal: Rgba([0x1f, 0x33, 0x2c, 0xe6]),
    lit: Rgba([0xd9, 0xa4, 0x2b, 0xf2]),
    text: Rgba([0xff, 0xff, 0xff, 0xff]),
    lit_text: Rgba([0x1b, 0x14, 0x04, 0xff]),
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
        push_label(&mut body, layout, index, petal, lit == Some(index), palette);
    }
    body.push_str("</g>");
    renderer.overlay_svg(&mut canvas, &body)?;
    for (index, petal) in petals.iter().enumerate() {
        if let Some(icon) = petal.icon {
            let lines = petal.status_lines().len();
            let icon = Icon {
                image: icon,
                status_lines: lines,
            };
            overlay_icon(&mut canvas, layout, index, &icon, scale, opening);
        }
    }
    let mut badges = String::new();
    for (index, petal) in petals.iter().enumerate() {
        push_badge(&mut badges, layout, index, petal);
    }
    if !badges.is_empty() {
        let group = format!(
            r#"<g transform="translate({middle} {middle}) scale({scale})" opacity="{opening}">{badges}</g>"#
        );
        renderer.overlay_svg(&mut canvas, &group)?;
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
/// of the menu: the top of the icon and the baseline of the label. The block
/// stays centred in the petal whatever the number of `status_lines` under the
/// label.
fn content_places(
    layout: &Layout,
    index: usize,
    has_icon: bool,
    status_lines: usize,
) -> (Point, f64, f64) {
    let ring = layout.ring();
    let centre = petal_centre(layout, index, f64::midpoint(ring.inner, ring.outer));
    let status = count_as_f64(status_lines) * STATUS_LINE;
    if has_icon {
        let height = ICON_SIZE + LABEL_GAP + LABEL_SIZE + status;
        let top = centre.y - height / 2.0;
        (centre, top, top + ICON_SIZE + LABEL_GAP + LABEL_SIZE)
    } else {
        (centre, centre.y, centre.y + LABEL_SIZE / 3.0 - status / 2.0)
    }
}

fn push_label(
    body: &mut String,
    layout: &Layout,
    index: usize,
    petal: &PetalView<'_>,
    lit: bool,
    palette: Palette,
) {
    let colour = if lit { palette.lit_text } else { palette.text };
    let lines = petal.status_lines();
    let (centre, _, baseline) = content_places(layout, index, petal.icon.is_some(), lines.len());
    let _ = write!(
        body,
        r#"<text x="{}" y="{baseline}" font-family="{}" font-size="{LABEL_SIZE}" font-weight="600" text-anchor="middle" fill="{}" fill-opacity="{}">"#,
        centre.x,
        crate::render::FONT_FAMILY,
        svg_color(colour),
        opacity_of(colour),
    );
    escape_xml(body, petal.label);
    body.push_str("</text>");
    for (line, text) in lines.into_iter().enumerate() {
        let y = baseline + count_as_f64(line + 1) * STATUS_LINE;
        let _ = write!(
            body,
            r#"<text x="{}" y="{y}" font-family="{}" font-size="{STATUS_SIZE}" font-weight="500" text-anchor="middle" fill="{}" fill-opacity="{}">"#,
            centre.x,
            crate::render::FONT_FAMILY,
            svg_color(colour),
            opacity_of(colour),
        );
        escape_xml(body, &text);
        body.push_str("</text>");
    }
}

/// Adds the badge of petal `index` to `body`, on the top right corner of its
/// icon; nothing without an icon or a count.
fn push_badge(body: &mut String, layout: &Layout, index: usize, petal: &PetalView<'_>) {
    let (Some(text), Some(_)) = (petal.badge_text(), petal.icon) else {
        return;
    };
    let lines = petal.status_lines().len();
    let (centre, top, _) = content_places(layout, index, true, lines);
    let width = count_as_f64(text.chars().count()) * BADGE_CHARACTER_WIDTH + BADGE_PADDING;
    let width = width.max(BADGE_HEIGHT);
    let right = centre.x + ICON_SIZE / 2.0 + BADGE_OVERHANG;
    let left = right - width;
    let up = top - BADGE_OVERHANG;
    let radius = BADGE_HEIGHT / 2.0;
    let _ = write!(
        body,
        r##"<rect x="{left}" y="{up}" width="{width}" height="{BADGE_HEIGHT}" rx="{radius}" fill="{BADGE_FILL}"/><text x="{}" y="{}" font-family="{}" font-size="{BADGE_FONT_SIZE}" font-weight="700" text-anchor="middle" fill="#ffffff">{text}</text>"##,
        left + width / 2.0,
        up + radius + BADGE_FONT_SIZE / 3.0,
        crate::render::FONT_FAMILY,
    );
}

/// Draws `icon` where petal `index` has it, scaled and faded like the rest of
/// the menu while it opens.
fn overlay_icon(
    canvas: &mut RgbaImage,
    layout: &Layout,
    index: usize,
    icon: &Icon<'_>,
    scale: f64,
    opening: f64,
) {
    let (centre, top, _) = content_places(layout, index, true, icon.status_lines);
    let side = side_in_pixels(ICON_SIZE * scale);
    if side == 0 {
        return;
    }
    let mut scaled = imageops::resize(icon.image, side, side, imageops::FilterType::Lanczos3);
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
        lit_text: Rgba([0, 0, 0, 255]),
    };

    fn layout(count: usize) -> Layout {
        Layout::new(count, RING).unwrap()
    }

    fn views(count: usize) -> Vec<PetalView<'static>> {
        ["Pomodoro", "Teams", "Audio", "IntelliJ", "Docker", "Claude"]
            .into_iter()
            .cycle()
            .take(count)
            .map(|label| PetalView {
                label,
                icon: None,
                status: None,
                badge: None,
            })
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
                status: None,
                badge: None,
            },
            PetalView {
                label: "a\u{1}b",
                icon: None,
                status: None,
                badge: None,
            },
            PetalView {
                label: "Audio",
                icon: None,
                status: None,
                badge: None,
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
    fn the_label_of_the_lit_petal_is_dark_and_the_others_are_light() {
        let petal = PetalView {
            label: "Pomodoro",
            icon: None,
            status: None,
            badge: None,
        };
        let colour_of = |lit| {
            let mut body = String::new();
            push_label(&mut body, &layout(3), 0, &petal, lit, PALETTE);
            body
        };
        assert!(
            colour_of(true).contains(r##"fill="#000000""##),
            "{}",
            colour_of(true)
        );
        assert!(
            colour_of(false).contains(r##"fill="#ffffff""##),
            "{}",
            colour_of(false)
        );
    }

    #[test]
    fn a_single_petal_is_a_ring_with_a_hole() {
        let image = paint_menu(1, Some(0), 1.0);
        let menu = layout(1);
        assert_eq!(pixel(&image, &menu, 0, 140.0).0[..3], LIT.0[..3]);
        assert_eq!(image.get_pixel(158, 158).0[3], 0, "the hole");
    }

    #[test]
    fn a_status_is_drawn_under_the_label_and_two_lines_at_most() {
        let renderer = TextRenderer::new().unwrap();
        let paint_with = |status: Option<&'static str>| {
            let petals: Vec<_> = views(4)
                .into_iter()
                .map(|view| PetalView { status, ..view })
                .collect();
            paint(&renderer, &layout(4), &petals, None, 1.0, PALETTE).unwrap()
        };
        let none = paint_with(None);
        let two = paint_with(Some("Focus 1/4\n25:00 · prêt"));
        let three = paint_with(Some("Focus 1/4\n25:00 · prêt\nlost"));
        assert_ne!(none, two, "the status is drawn");
        assert_eq!(two, three, "a third line is not");
        let long = paint_with(Some("Focus 1/4\n25:00 · prêt and a great deal more text"));
        let cut = paint_with(Some("Focus 1/4\n25:00 · prêt and a great"));
        assert_eq!(long, cut, "a line is cut at the width of a petal");
    }

    fn icon_image() -> RgbaImage {
        RgbaImage::from_pixel(96, 96, Rgba([0x20, 0x90, 0x40, 0xff]))
    }

    #[test]
    fn a_badge_says_the_count_up_to_ninety_nine() {
        let text = |badge| {
            PetalView {
                badge,
                ..views(1).remove(0)
            }
            .badge_text()
        };
        assert_eq!(text(None), None);
        assert_eq!(text(Some(0)), None);
        assert_eq!(text(Some(1)).as_deref(), Some("1"));
        assert_eq!(text(Some(99)).as_deref(), Some("99"));
        assert_eq!(text(Some(100)).as_deref(), Some("99+"));
        assert_eq!(text(Some(u32::MAX)).as_deref(), Some("99+"));
    }

    #[test]
    fn a_badge_is_drawn_over_the_icon_only_when_there_is_a_count() {
        let renderer = TextRenderer::new().unwrap();
        let icon = icon_image();
        let paint_with = |badge: Option<u32>| {
            let petals: Vec<_> = views(4)
                .into_iter()
                .map(|view| PetalView {
                    icon: Some(&icon),
                    badge,
                    ..view
                })
                .collect();
            paint(&renderer, &layout(4), &petals, None, 1.0, PALETTE).unwrap()
        };
        let none = paint_with(None);
        assert_eq!(none, paint_with(Some(0)), "zero draws nothing");
        let some = paint_with(Some(5));
        assert_ne!(none, some, "the badge is drawn");
        let red = |image: &RgbaImage| {
            image
                .pixels()
                .filter(|pixel| pixel.0[..3] == [0xc8, 0x45, 0x2d] && pixel.0[3] > 200)
                .count()
        };
        assert_eq!(red(&none), 0);
        assert!(red(&some) > 100, "{}", red(&some));
        // A longer count widens the badge, and a huge one is capped.
        assert!(red(&paint_with(Some(42))) > red(&some));
        assert_eq!(paint_with(Some(100)), paint_with(Some(100_000)));
    }

    #[test]
    fn a_badge_follows_the_opening_of_the_menu_and_needs_an_icon() {
        let renderer = TextRenderer::new().unwrap();
        let without_icon: Vec<_> = views(4)
            .into_iter()
            .map(|view| PetalView {
                badge: Some(7),
                ..view
            })
            .collect();
        let plain = paint(&renderer, &layout(4), &views(4), None, 1.0, PALETTE).unwrap();
        let badged = paint(&renderer, &layout(4), &without_icon, None, 1.0, PALETTE).unwrap();
        assert_eq!(plain, badged, "no icon: nowhere to put a badge");

        let icon = icon_image();
        let with_icon: Vec<_> = without_icon
            .iter()
            .map(|view| PetalView {
                icon: Some(&icon),
                ..*view
            })
            .collect();
        let opening = paint(&renderer, &layout(4), &with_icon, None, 0.0, PALETTE).unwrap();
        let open = paint(&renderer, &layout(4), &with_icon, None, 1.0, PALETTE).unwrap();
        assert_ne!(opening, open);
    }

    #[test]
    fn the_block_of_a_petal_stays_centred_whatever_the_lines_of_status() {
        let layout = layout(4);
        let top = |lines| content_places(&layout, 0, true, lines).1;
        // Two lines of status move the block up by half of their height.
        assert!((top(0) - top(2) - STATUS_LINE).abs() < 1e-9);
        let baseline = |lines| content_places(&layout, 0, false, lines).2;
        assert!((baseline(0) - baseline(2) - STATUS_LINE).abs() < 1e-9);
    }

    #[test]
    fn an_icon_is_drawn_above_the_label() {
        let renderer = TextRenderer::new().unwrap();
        let red = RgbaImage::from_pixel(8, 8, Rgba([255, 0, 0, 255]));
        let petals: Vec<_> = views(4)
            .into_iter()
            .map(|view| PetalView {
                icon: Some(&red),
                status: None,
                badge: None,
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
