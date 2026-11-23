//! Draws the radial menu into an image: the petals, the one under the pointer
//! lit, the icon and the label of each. The overlay shows the image; nothing
//! here talks to the screen.
//!
//! The drawing is an SVG document drawn by the renderer of the keys, so the
//! menu has the same text and the same icons as the deck.

use std::f64::consts::TAU;
use std::fmt::Write as _;

use hubergamof_module_api::{MAX_BADGE_CHARS, MAX_RADIAL_ITEMS, RadialItem, RadialItemState};
use image::{Rgba, RgbaImage, imageops};

use super::geometry::{ARC_GAP, ARC_INNER, ARC_OUTER, Layout, Outline, Point};
use super::glyphs;
use crate::render::{FontWeight, RenderError, TextRenderer, escape_xml, svg_color};

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
/// Smallest the menu is while it opens, as a fraction of its size.
const OPENING_SCALE: f64 = 0.85;
/// Width of the thin edge along the outside of the petal that has actions, in
/// pixels.
const EDGE_WIDTH: f64 = 4.0;
/// Size of the glyph of an action, in pixels, when its slice has room.
const GLYPH_SIZE: f64 = 20.0;
/// Smallest a glyph is made, in pixels, on a narrow slice.
const MIN_GLYPH_SIZE: f64 = 10.0;
/// Height of the badge on a glyph, in pixels.
const BADGE_HEIGHT: f64 = 14.0;
/// Width the badge gains with each character after the first, in pixels.
const BADGE_CHAR_WIDTH: f64 = 5.5;
/// Font size of a badge, in pixels.
const BADGE_FONT_SIZE: f64 = 9.0;
/// Height of the pill that names the action under the pointer, in pixels.
const PILL_HEIGHT: f64 = 24.0;
/// Space between the text of the pill and its ends, in pixels.
const PILL_PADDING: f64 = 10.0;
/// Font size of the label in the pill, in pixels.
const PILL_FONT_SIZE: f64 = 12.0;
/// Space between the arc and the pill, in pixels.
const PILL_GAP: f64 = 6.0;
/// Widest the pill is, in pixels: a longer label is cut with an ellipsis.
const PILL_MAX_WIDTH: f64 = 120.0;
/// Height of a letter above the baseline, as a fraction of the font size, to
/// centre text on a line.
const TEXT_CENTRE: f64 = 0.35;

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
    /// The actions drawn in front of the petal when it is the one lit.
    pub items: &'a [RadialItem],
}

impl PetalView<'_> {
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
    /// The pill that names the action under the pointer.
    pub pill: Rgba<u8>,
    /// The text in that pill.
    pub pill_text: Rgba<u8>,
    /// The glyph of an action that is off (a muted microphone): the cross-out
    /// stands out from the others.
    pub off: Rgba<u8>,
}

/// The colours of the design: tea green petals, bergamot for the one lit.
pub const DEFAULT_PALETTE: Palette = Palette {
    petal: Rgba([0x1f, 0x33, 0x2c, 0xe6]),
    lit: Rgba([0xd9, 0xa4, 0x2b, 0xf2]),
    text: Rgba([0xff, 0xff, 0xff, 0xff]),
    lit_text: Rgba([0x1b, 0x14, 0x04, 0xff]),
    pill: Rgba([0xff, 0xf7, 0xe0, 0xff]),
    pill_text: Rgba([0x1b, 0x14, 0x04, 0xff]),
    off: Rgba([0xff, 0x8a, 0x7a, 0xff]),
};

/// Side of the image that holds a menu of `layout`, in pixels: the ring, or,
/// with the arc of actions of a petal (`with_arc`), the arc and the pill that
/// names an action beyond it.
#[must_use]
pub fn image_side(layout: &Layout, with_arc: bool) -> u32 {
    let reach = if with_arc {
        ARC_OUTER + PILL_GAP + PILL_MAX_WIDTH
    } else {
        layout.ring().outer
    };
    side_in_pixels(2.0 * (reach + MARGIN))
}

/// Draws the menu. `lit` is the petal under the pointer, with its arc of
/// actions drawn when it has some, and `lit_item` the action of that arc the
/// pointer is on; `opening` goes from 0 (just opened) to 1 (fully open) when
/// the menu is animated, and is 1 when it is not.
///
/// # Errors
///
/// [`RenderError`] if the drawing cannot be parsed or rasterised.
pub fn paint(
    renderer: &TextRenderer,
    layout: &Layout,
    petals: &[PetalView<'_>],
    lit: Option<usize>,
    lit_item: Option<usize>,
    opening: f32,
    palette: Palette,
) -> Result<RgbaImage, RenderError> {
    let arc_of = lit.and_then(|petal| {
        let view = petals.get(petal).filter(|view| !view.items.is_empty())?;
        Some((petal, view.items))
    });
    let side = image_side(layout, arc_of.is_some());
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
    if let Some((petal, items)) = arc_of {
        let arc = Arc {
            layout,
            petal,
            items,
            lit_item,
            palette,
            limit: f64::from(side) / 2.0 - MARGIN,
        };
        push_arc(&mut body, renderer, &arc)?;
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
    push_shape(body, &shape, fill);
}

/// Fills the path `shape` with `fill`.
fn push_shape(body: &mut String, shape: &str, fill: Rgba<u8>) {
    let _ = write!(
        body,
        r#"<path d="{shape}" fill="{}" fill-opacity="{}" fill-rule="evenodd"/>"#,
        svg_color(fill),
        opacity_of(fill),
    );
}

/// What the arc of actions in front of a petal is drawn from.
struct Arc<'a> {
    layout: &'a Layout,
    petal: usize,
    /// The actions, more than [`MAX_RADIAL_ITEMS`] being ignored.
    items: &'a [RadialItem],
    lit_item: Option<usize>,
    palette: Palette,
    /// How far from the middle the pill may go and still be in the image.
    limit: f64,
}

impl Arc<'_> {
    fn shown(&self) -> &[RadialItem] {
        &self.items[..self.items.len().min(MAX_RADIAL_ITEMS)]
    }
}

/// The arc in front of a petal: the thin edge that says whom it belongs to, a
/// slice per action with its glyph and badge, and the label of the action
/// under the pointer.
fn push_arc(body: &mut String, renderer: &TextRenderer, arc: &Arc<'_>) -> Result<(), RenderError> {
    push_edge(body, arc);
    let count = arc.shown().len();
    for (index, item) in arc.shown().iter().enumerate() {
        push_slice(body, arc, index, item, arc.lit_item == Some(index), count);
    }
    if let Some(index) = arc.lit_item
        && let Some(item) = arc.shown().get(index)
    {
        push_pill(
            body,
            renderer,
            arc,
            item,
            slice_direction(arc, index, count),
        )?;
    }
    Ok(())
}

/// The thin bergamot edge along the outside of the petal.
fn push_edge(body: &mut String, arc: &Arc<'_>) {
    let ring = arc.layout.ring();
    let (inner, outer) = (ring.outer, ring.outer + EDGE_WIDTH);
    let shape = arc
        .layout
        .band_outline(arc.petal, inner, outer)
        .map_or_else(
            || whole_ring(inner, outer),
            |outline| petal_path(&outline, inner, outer),
        );
    push_shape(body, &shape, arc.palette.lit);
}

/// Unit vector of the middle line of slice `index`.
fn slice_direction(arc: &Arc<'_>, index: usize, count: usize) -> Point {
    let angle = arc.layout.slice_angle(arc.petal, index, count);
    Point::new(angle.sin(), -angle.cos())
}

/// Radius the glyphs of the arc are drawn at.
fn glyph_radius() -> f64 {
    f64::midpoint(ARC_INNER, ARC_OUTER)
}

/// Size of the glyph on a slice of `count`: [`GLYPH_SIZE`], less when the
/// slice is too narrow for it.
fn glyph_size(layout: &Layout, count: usize) -> f64 {
    (layout.slice_width(count) * glyph_radius() - ARC_GAP).clamp(MIN_GLYPH_SIZE, GLYPH_SIZE)
}

fn push_slice(
    body: &mut String,
    arc: &Arc<'_>,
    index: usize,
    item: &RadialItem,
    lit: bool,
    count: usize,
) {
    let Some(outline) = arc.layout.slice_outline(arc.petal, index, count) else {
        return;
    };
    let palette = arc.palette;
    push_shape(
        body,
        &petal_path(&outline, ARC_INNER, ARC_OUTER),
        if lit { palette.lit } else { palette.petal },
    );
    let direction = slice_direction(arc, index, count);
    let centre = Point::new(direction.x * glyph_radius(), direction.y * glyph_radius());
    let size = glyph_size(arc.layout, count);
    let colour = match (lit, item.state) {
        (true, _) => palette.lit_text,
        (false, RadialItemState::Off) => palette.off,
        (false, _) => palette.text,
    };
    body.push_str(&glyphs::svg(
        item.glyph,
        item.state,
        (centre.x, centre.y),
        size,
        &svg_color(colour),
    ));
    if let Some(badge) = &item.badge {
        push_badge(
            body,
            badge,
            Point::new(centre.x + size / 2.0, centre.y - size / 2.0),
            lit,
            palette,
        );
    }
}

/// A small pill with `badge` (cut at [`MAX_BADGE_CHARS`]) centred on `at`:
/// bergamot, or dark on the slice that is already bergamot.
fn push_badge(body: &mut String, badge: &str, at: Point, on_lit: bool, palette: Palette) {
    let text: String = badge.chars().take(MAX_BADGE_CHARS).collect();
    if text.is_empty() {
        return;
    }
    let (fill, ink) = if on_lit {
        (palette.lit_text, palette.lit)
    } else {
        (palette.lit, palette.lit_text)
    };
    let width = BADGE_HEIGHT + count_as_f64(text.chars().count() - 1) * BADGE_CHAR_WIDTH;
    let _ = write!(
        body,
        r#"<rect x="{}" y="{}" width="{width}" height="{BADGE_HEIGHT}" rx="{}" fill="{}"/>"#,
        at.x - width / 2.0,
        at.y - BADGE_HEIGHT / 2.0,
        BADGE_HEIGHT / 2.0,
        svg_color(fill),
    );
    let _ = write!(
        body,
        r#"<text x="{}" y="{}" font-family="{}" font-size="{BADGE_FONT_SIZE}" font-weight="700" text-anchor="middle" fill="{}">"#,
        at.x,
        at.y + BADGE_FONT_SIZE * TEXT_CENTRE,
        crate::render::FONT_FAMILY,
        svg_color(ink),
    );
    escape_xml(body, &text);
    body.push_str("</text>");
}

/// Where the pill of the label goes, from the middle of the menu.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pill {
    centre: Point,
    width: f64,
}

/// The pill of `width` that names the action in the direction `direction`
/// (a unit vector): just beyond the arc, on the outside, then moved back in so
/// that it stays within `limit` of the middle on both axes.
fn pill_place(direction: Point, width: f64, limit: f64) -> Pill {
    let anchor = ARC_OUTER + PILL_GAP;
    let place = |toward: f64, length: f64| {
        let reach = (limit - length / 2.0).max(0.0);
        (toward * (anchor + length / 2.0)).clamp(-reach, reach)
    };
    Pill {
        centre: Point::new(place(direction.x, width), place(direction.y, PILL_HEIGHT)),
        width,
    }
}

fn push_pill(
    body: &mut String,
    renderer: &TextRenderer,
    arc: &Arc<'_>,
    item: &RadialItem,
    direction: Point,
) -> Result<(), RenderError> {
    if item.label.trim().is_empty() {
        return Ok(());
    }
    let room = PILL_MAX_WIDTH - 2.0 * PILL_PADDING;
    #[allow(clippy::cast_possible_truncation)]
    let text = renderer.fit(
        &item.label,
        PILL_FONT_SIZE as f32,
        FontWeight::SemiBold,
        room as f32,
    )?;
    #[allow(clippy::cast_possible_truncation)]
    let measured = f64::from(renderer.width(&text, PILL_FONT_SIZE as f32, FontWeight::SemiBold)?);
    let pill = pill_place(direction, measured + 2.0 * PILL_PADDING, arc.limit);
    let palette = arc.palette;
    let _ = write!(
        body,
        r#"<rect x="{}" y="{}" width="{}" height="{PILL_HEIGHT}" rx="{}" fill="{}" fill-opacity="{}"/>"#,
        pill.centre.x - pill.width / 2.0,
        pill.centre.y - PILL_HEIGHT / 2.0,
        pill.width,
        PILL_HEIGHT / 2.0,
        svg_color(palette.pill),
        opacity_of(palette.pill),
    );
    let _ = write!(
        body,
        r#"<text x="{}" y="{}" font-family="{}" font-size="{PILL_FONT_SIZE}" font-weight="600" text-anchor="middle" fill="{}" fill-opacity="{}">"#,
        pill.centre.x,
        pill.centre.y + PILL_FONT_SIZE * TEXT_CENTRE,
        crate::render::FONT_FAMILY,
        svg_color(palette.pill_text),
        opacity_of(palette.pill_text),
    );
    escape_xml(body, &text);
    body.push_str("</text>");
    Ok(())
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
        pill: Rgba([250, 240, 210, 255]),
        pill_text: Rgba([10, 10, 0, 255]),
        off: Rgba([255, 130, 120, 255]),
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
                items: &[],
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
            None,
            opening,
            PALETTE,
        )
        .unwrap()
    }

    #[test]
    fn the_image_holds_the_ring_and_a_margin() {
        let menu = layout(4);
        assert_eq!(image_side(&menu, false), 316);
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
                items: &[],
            },
            PetalView {
                label: "a\u{1}b",
                icon: None,
                status: None,
                items: &[],
            },
            PetalView {
                label: "Audio",
                icon: None,
                status: None,
                items: &[],
            },
        ];
        let image = paint(&renderer, &layout(3), &petals, None, None, 1.0, PALETTE).unwrap();
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
            items: &[],
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
            paint(&renderer, &layout(4), &petals, None, None, 1.0, PALETTE).unwrap()
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
                ..view
            })
            .collect();
        let image = paint(&renderer, &layout(4), &petals, None, None, 1.0, PALETTE).unwrap();
        let reds = image
            .pixels()
            .filter(|pixel| pixel.0[0] > 200 && pixel.0[1] < 60 && pixel.0[3] > 200)
            .count();
        assert!(reds > 4 * 20 * 20, "four icons of about 36 px: {reds}");
    }

    use hubergamof_module_api::RadialGlyph;

    fn teams_items() -> Vec<RadialItem> {
        vec![
            RadialItem::new("chat", "Conversations", RadialGlyph::Chat).with_badge("3"),
            RadialItem::new("mute", "Micro", RadialGlyph::Microphone)
                .with_state(RadialItemState::Off),
            RadialItem::new("hang-up", "Raccrocher", RadialGlyph::HangUp),
        ]
    }

    /// A menu of `count` petals, each with `items`, painted with the petal
    /// `lit` and its action `lit_item`.
    fn paint_arc(
        count: usize,
        items: &[RadialItem],
        lit: Option<usize>,
        lit_item: Option<usize>,
    ) -> RgbaImage {
        let petals: Vec<_> = views(count)
            .into_iter()
            .map(|view| PetalView { items, ..view })
            .collect();
        paint(
            &TextRenderer::new().unwrap(),
            &layout(count),
            &petals,
            lit,
            lit_item,
            1.0,
            PALETTE,
        )
        .unwrap()
    }

    /// Saves the image in the folder `RADIAL_PICTURES` names, for a look at it.
    fn keep(image: &RgbaImage, name: &str) {
        if let Some(dir) = std::env::var_os("RADIAL_PICTURES") {
            image.save(std::path::Path::new(&dir).join(name)).unwrap();
        }
    }

    fn pill_pixels(image: &RgbaImage) -> usize {
        image
            .pixels()
            .filter(|pixel| pixel.0[..3] == PALETTE.pill.0[..3] && pixel.0[3] > 200)
            .count()
    }

    #[test]
    fn the_arc_makes_the_image_bigger_only_for_a_lit_petal_that_has_actions() {
        let items = teams_items();
        let plain = paint_arc(4, &[], Some(0), None);
        assert_eq!(plain.dimensions(), (316, 316));
        let with_arc = paint_arc(4, &items, Some(0), None);
        let side = image_side(&layout(4), true);
        assert_eq!(with_arc.dimensions(), (side, side));
        assert!(side > 600, "room for the pill beyond the arc: {side}");
        let not_lit = paint_arc(4, &items, None, None);
        assert_eq!(
            not_lit.dimensions(),
            (316, 316),
            "no arc without a lit petal"
        );
    }

    #[test]
    fn the_slice_under_the_pointer_is_bergamot_and_the_others_are_not() {
        let items = teams_items();
        let image = paint_arc(4, &items, Some(0), Some(1));
        let menu = layout(4);
        // The middle one of three is on the axis of the petal; 208 is clear of
        // the glyph (at 187, 10 px each way) and of its badge.
        assert_eq!(pixel(&image, &menu, 0, 208.0).0[..3], LIT.0[..3]);
        let beside = |item: usize| {
            let angle = menu.slice_angle(0, item, 3);
            let middle = f64::from(image.width()) / 2.0;
            let spot = Point::new(208.0 * angle.sin(), -208.0 * angle.cos());
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let at = ((middle + spot.x) as u32, (middle + spot.y) as u32);
            *image.get_pixel(at.0, at.1)
        };
        assert_eq!(beside(0).0[..3], PETAL.0[..3]);
        assert_eq!(beside(2).0[..3], PETAL.0[..3]);
        keep(&image, "arc-lit-item.png");
    }

    #[test]
    fn nothing_of_the_arc_is_drawn_for_a_petal_that_is_not_lit() {
        let items = teams_items();
        let image = paint_arc(4, &items, Some(1), None);
        let menu = layout(4);
        // Where the arc of the first petal would be: transparent.
        assert_eq!(pixel(&image, &menu, 0, 190.0).0[3], 0);
        // The arc of the second one is there.
        assert_eq!(pixel(&image, &menu, 1, 208.0).0[..3], PETAL.0[..3]);
    }

    #[test]
    fn the_lit_petal_has_a_bergamot_edge_that_the_others_have_not() {
        let items = teams_items();
        let image = paint_arc(4, &items, Some(0), None);
        let menu = layout(4);
        // Between 150 and 154, on the axis.
        assert_eq!(pixel(&image, &menu, 0, 152.0).0[..3], LIT.0[..3]);
        assert_eq!(pixel(&image, &menu, 0, 157.0).0[3], 0, "then a clear gap");
        assert_eq!(pixel(&image, &menu, 1, 152.0).0[3], 0, "not on the others");
    }

    #[test]
    fn the_label_of_the_action_is_in_a_light_pill_only_while_one_is_under_the_pointer() {
        let items = teams_items();
        let named = paint_arc(4, &items, Some(0), Some(0));
        let unnamed = paint_arc(4, &items, Some(0), None);
        assert!(pill_pixels(&named) > 500, "{}", pill_pixels(&named));
        assert_eq!(pill_pixels(&unnamed), 0);
        keep(&named, "arc-pill.png");
    }

    #[test]
    fn the_arc_of_a_petal_on_the_side_keeps_its_pill_beside_it() {
        let items = vec![
            RadialItem::new("chat", "Conversations", RadialGlyph::Chat).with_badge("3"),
            RadialItem::new("activity", "Activités", RadialGlyph::Bell).with_badge("1"),
            RadialItem::new("mute", "Micro", RadialGlyph::Microphone)
                .with_state(RadialItemState::Off),
            RadialItem::new("camera", "Caméra", RadialGlyph::Camera),
            RadialItem::new("hang-up", "Raccrocher", RadialGlyph::HangUp),
            RadialItem::new("return", "Retour à l'appel", RadialGlyph::ReturnToCall),
        ];
        let image = paint_arc(4, &items, Some(1), Some(5));
        assert!(pill_pixels(&image) > 500);
        keep(&image, "arc-side-pill.png");
        keep(&paint_arc(4, &items, Some(1), Some(2)), "arc-side-mic.png");
        keep(&paint_arc(4, &items, Some(0), Some(0)), "arc-top.png");
    }

    #[test]
    fn an_action_that_went_away_under_the_pointer_draws_the_arc_without_a_name() {
        let items = teams_items();
        let image = paint_arc(4, &items, Some(0), Some(5));
        assert_eq!(pill_pixels(&image), 0);
    }

    #[test]
    fn a_muted_glyph_is_tinted_and_a_badge_is_bergamot() {
        let items = teams_items();
        let image = paint_arc(4, &items, Some(0), None);
        let near = |wanted: Rgba<u8>| {
            image
                .pixels()
                .filter(|pixel| {
                    pixel.0[3] > 200
                        && pixel.0[..3]
                            .iter()
                            .zip(&wanted.0[..3])
                            .all(|(have, want)| have.abs_diff(*want) < 24)
                })
                .count()
        };
        assert!(near(PALETTE.off) > 20, "the crossed microphone");
        let chat = || RadialItem::new("chat", "Conversations", RadialGlyph::Chat);
        let bergamot = |image: &RgbaImage| {
            image
                .pixels()
                .filter(|pixel| pixel.0[3] > 200 && pixel.0[..3] == LIT.0[..3])
                .count()
        };
        let without = paint_arc(4, &[chat()], Some(0), None);
        let with_badge = paint_arc(4, &[chat().with_badge("3")], Some(0), None);
        assert!(bergamot(&with_badge) > bergamot(&without) + 30);
    }

    #[test]
    fn a_badge_is_cut_at_the_characters_the_arc_allows() {
        let draw = |badge: &str| {
            let item =
                RadialItem::new("chat", "Conversations", RadialGlyph::Chat).with_badge(badge);
            paint_arc(4, &[item], Some(0), None)
        };
        assert_eq!(draw("123"), draw("123456"));
        assert_ne!(draw("12"), draw("123"));
        let bare = RadialItem::new("chat", "Conversations", RadialGlyph::Chat);
        assert_eq!(draw(""), paint_arc(4, &[bare], Some(0), None));
    }

    #[test]
    fn six_actions_on_every_petal_of_every_menu_are_drawn_and_the_margin_stays_clear() {
        let items: Vec<_> = [
            RadialGlyph::Microphone,
            RadialGlyph::Camera,
            RadialGlyph::HangUp,
            RadialGlyph::Chat,
            RadialGlyph::Bell,
            RadialGlyph::ReturnToCall,
        ]
        .into_iter()
        .map(|glyph| {
            RadialItem::new("a", "Retour à l'appel et un libellé bien trop long", glyph)
                .with_badge("99+")
        })
        .collect();
        for count in [1, 2, 4, 8] {
            let side = image_side(&layout(count), true);
            for petal in 0..count {
                for item in 0..6 {
                    let image = paint_arc(count, &items, Some(petal), Some(item));
                    assert_eq!(image.dimensions(), (side, side));
                    assert!(pill_pixels(&image) > 500, "{count}/{petal}/{item}");
                    // Nothing on the outermost rows and columns: the margin is
                    // kept clear.
                    for at in 0..side {
                        for (x, y) in [(at, 0), (at, side - 1), (0, at), (side - 1, at)] {
                            assert_eq!(
                                image.get_pixel(x, y).0[3],
                                0,
                                "{count}/{petal}/{item} at {x},{y}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_pill_goes_beyond_the_arc_and_never_out_of_the_image() {
        let limit = f64::from(image_side(&layout(4), true)) / 2.0 - MARGIN;
        for step in 0..360 {
            let angle = f64::from(step).to_radians();
            let direction = Point::new(angle.sin(), -angle.cos());
            for width in [40.0, 90.0, PILL_MAX_WIDTH] {
                let pill = pill_place(direction, width, limit);
                assert!(
                    pill.centre.x.abs() + width / 2.0 <= limit + 1e-9,
                    "{step} {width}"
                );
                assert!(
                    pill.centre.y.abs() + PILL_HEIGHT / 2.0 <= limit + 1e-9,
                    "{step} {width}"
                );
            }
        }
        // Straight up: just above the arc, centred.
        let up = pill_place(Point::new(0.0, -1.0), 80.0, limit);
        assert!(up.centre.x.abs() < 1e-9);
        assert!((up.centre.y + (ARC_OUTER + PILL_GAP + PILL_HEIGHT / 2.0)).abs() < 1e-9);
        // To the right: its left end is just past the arc.
        let right = pill_place(Point::new(1.0, 0.0), 80.0, limit);
        assert!((right.centre.x - 40.0 - (ARC_OUTER + PILL_GAP)).abs() < 1e-9);
    }

    #[test]
    fn a_glyph_keeps_its_size_while_the_slice_has_room_for_it() {
        assert!((glyph_size(&layout(8), 3) - GLYPH_SIZE).abs() < 1e-9);
        let narrow = glyph_size(&layout(8), 6);
        assert!((MIN_GLYPH_SIZE..=GLYPH_SIZE).contains(&narrow));
    }

    #[test]
    fn markup_in_the_label_and_the_badge_of_an_action_does_not_break_the_drawing() {
        let item = RadialItem::new("a", "<b>&\"'", RadialGlyph::Play).with_badge("<&>");
        let image = paint_arc(4, &[item], Some(0), Some(0));
        assert!(pill_pixels(&image) > 100);
    }
}
