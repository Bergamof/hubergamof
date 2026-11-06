//! Page zone of the navigation dial: the title of the page shown, the page
//! the dial leads back to, a hint of the way back and the row on display.
//!
//! Ratios of the zone, designed on a 200 × 100 px zone (Stream Deck+).

use std::sync::Arc;

use hubergamof_module_api::{PageZone, RowPosition};
use hubergamof_protocol::ImageSize;
use image::{Rgba, RgbaImage};

use super::source::{f32_from, svg_color};
use super::svg::{push_svg, shape_gradient};
use super::text::TextLine;
use super::{
    CacheKey, RenderError, Rendered, Renderer, TEXT_COLOR, ZoneGeometry, check_size, keep_opaque,
    rgba, semibold,
};

/// Rows other than the current one.
const OTHER_ROW_COLOR: Rgba<u8> = Rgba([0x23, 0x2a, 0x40, u8::MAX]);
/// Row caption.
const CAPTION_COLOR: Rgba<u8> = Rgba([0x6b, 0x73, 0x85, u8::MAX]);

/// Identifier of the gradient of the hint and of the current row.
const GRADIENT_ID: &str = "indigo";

/// Font size of the parent page, relative to the zone height.
const PARENT_FONT_RATIO: f32 = 0.12;
/// Baseline of the parent page, relative to the zone height.
const PARENT_BASELINE_RATIO: f32 = 0.25;
/// Font size of the title, relative to the zone height.
const TITLE_FONT_RATIO: f32 = 0.24;
/// Baseline of the title, relative to the zone height.
const TITLE_BASELINE_RATIO: f32 = 0.58;
/// Width the title may take, relative to the zone width.
const TITLE_WIDTH_RATIO: f32 = 0.90;
/// What follows the parent page, to read as a path with the title below.
const PARENT_SEPARATOR: &str = " ›";

/// Side of the hint, relative to the zone height.
const HINT_SIDE_RATIO: f32 = 0.24;
/// Distance of the hint from the top, relative to the zone height.
const HINT_TOP_RATIO: f32 = 0.07;
/// Distance of the hint from the right edge, relative to the zone width.
const HINT_RIGHT_RATIO: f32 = 0.055;
/// Corner radius of the hint, relative to its side.
const HINT_RADIUS_RATIO: f32 = 0.33;
/// Outline of the hint, relative to its side.
const HINT_OUTLINE_RATIO: f32 = 0.075;
/// Dash and gap of the outline of a disabled hint, relative to its side.
const HINT_DASH_RATIO: f32 = 0.125;
/// Opacity of a disabled hint.
const DISABLED_HINT_OPACITY: f32 = 0.4;
/// Opacity of the fill of an enabled hint.
const HINT_FILL_OPACITY: f32 = 0.18;
/// Arrow of the hint, centred on the origin, drawn on a 24-unit square.
const ARROW_PATH: &str = "M-6 -3H3A5.5 5.5 0 0 1 3 8H-4M-3 -7L-7 -3L-3 1";
/// Size of the arrow drawing, in units.
const ARROW_UNITS: f32 = 24.0;
/// Part of the hint the arrow takes.
const ARROW_SHARE: f32 = 0.8;
/// Stroke of the arrow, in units.
const ARROW_STROKE_UNITS: f32 = 2.3;

/// Rows up to which each row has its own segment; beyond, one track with a
/// marker.
const MAX_ROW_SEGMENTS: u32 = 7;
/// Left and right margin of the rows, relative to the zone width.
const ROWS_MARGIN_RATIO: f32 = 0.08;
/// Top of the segments, relative to the zone height.
const ROWS_TOP_RATIO: f32 = 0.80;
/// Thickness of the segments, relative to the zone height.
const ROWS_THICKNESS_RATIO: f32 = 0.04;
/// Gap between two segments, relative to the zone width.
const ROWS_GAP_RATIO: f32 = 0.02;
/// Font size of the row caption, relative to the zone height.
const CAPTION_FONT_RATIO: f32 = 0.10;
/// Baseline of the row caption, relative to the zone height.
const CAPTION_BASELINE_RATIO: f32 = 0.95;

impl Renderer {
    /// Renders the page zone of the navigation dial. CPU-bound: call it from
    /// a blocking context.
    ///
    /// # Errors
    ///
    /// [`RenderError::InvalidSize`] when the size is empty.
    pub fn render_page_zone(
        &self,
        page: &PageZone,
        size: ImageSize,
        background: Rgba<u8>,
    ) -> Result<Arc<Rendered>, RenderError> {
        check_size(size)?;
        let key = CacheKey::Page {
            page: page.clone(),
            size: (size.width, size.height),
            background: background.0,
        };
        self.cached(key, || {
            let mut canvas = RgbaImage::from_pixel(size.width, size.height, background);
            let zone = ZoneGeometry::of_size(size);
            self.text
                .overlay_svg(&mut canvas, &page_shapes(page, zone))?;
            self.text.draw(&mut canvas, &self.page_lines(page, zone)?)?;
            Ok(keep_opaque(canvas, background))
        })
    }

    /// Parent page, title and row caption.
    fn page_lines(
        &self,
        page: &PageZone,
        zone: ZoneGeometry,
    ) -> Result<Vec<TextLine>, RenderError> {
        let center = zone.width / 2.0;
        let mut lines = Vec::new();
        if let Some(parent) = &page.parent {
            lines.push(self.text.fitted_line(
                &format!("{parent}{PARENT_SEPARATOR}"),
                semibold(zone.y(PARENT_FONT_RATIO), rgba(page.accent.from)),
                center,
                zone.y(PARENT_BASELINE_RATIO),
                zone.width * TITLE_WIDTH_RATIO,
            )?);
        }
        lines.push(self.text.fitted_line(
            &page.title,
            semibold(zone.y(TITLE_FONT_RATIO), TEXT_COLOR),
            center,
            zone.y(TITLE_BASELINE_RATIO),
            zone.width * TITLE_WIDTH_RATIO,
        )?);
        if let Some(rows) = scrollable_rows(page) {
            lines.push(self.text.fitted_line(
                &rows.caption(),
                semibold(zone.y(CAPTION_FONT_RATIO), CAPTION_COLOR),
                center,
                zone.y(CAPTION_BASELINE_RATIO),
                zone.width * TITLE_WIDTH_RATIO,
            )?);
        }
        Ok(lines)
    }
}

/// The rows when there is more than one to move between.
fn scrollable_rows(page: &PageZone) -> Option<RowPosition> {
    page.rows.filter(|rows| rows.is_scrollable())
}

/// The hint of the way back and the rows, which carry no text.
fn page_shapes(page: &PageZone, zone: ZoneGeometry) -> String {
    let mut svg = shape_gradient(GRADIENT_ID, (rgba(page.accent.from), rgba(page.accent.to)));
    push_hint(&mut svg, zone, page);
    if let Some(rows) = scrollable_rows(page) {
        push_rows(&mut svg, zone, rows);
    }
    svg
}

/// Rounded square with the back arrow: lit when the dial goes back, dashed
/// and faint on the home page.
fn push_hint(svg: &mut String, zone: ZoneGeometry, page: &PageZone) {
    let enabled = page.can_go_back();
    let side = zone.y(HINT_SIDE_RATIO);
    let left = zone.width * (1.0 - HINT_RIGHT_RATIO) - side;
    let top = zone.y(HINT_TOP_RATIO);
    let outline = side * HINT_OUTLINE_RATIO;
    let radius = side * HINT_RADIUS_RATIO;
    let (center_x, center_y) = (left + side / 2.0, top + side / 2.0);
    let scale = side * ARROW_SHARE / ARROW_UNITS;
    if enabled {
        push_svg!(
            svg,
            r#"<rect x="{left}" y="{top}" width="{side}" height="{side}" rx="{radius}" fill="url(#{GRADIENT_ID})" fill-opacity="{HINT_FILL_OPACITY}" stroke="url(#{GRADIENT_ID})" stroke-width="{outline}"/>"#,
        );
    } else {
        let dash = side * HINT_DASH_RATIO;
        push_svg!(
            svg,
            r#"<g opacity="{DISABLED_HINT_OPACITY}"><rect x="{left}" y="{top}" width="{side}" height="{side}" rx="{radius}" fill="none" stroke="{color}" stroke-width="{outline}" stroke-dasharray="{dash} {dash}"/></g>"#,
            color = svg_color(rgba(page.accent.from)),
        );
    }
    let arrow_color = rgba(page.accent.from);
    let opacity = if enabled { 1.0 } else { DISABLED_HINT_OPACITY };
    push_svg!(
        svg,
        r#"<path transform="translate({center_x} {center_y}) scale({scale})" d="{ARROW_PATH}" fill="none" stroke="{color}" stroke-width="{ARROW_STROKE_UNITS}" stroke-linecap="round" stroke-linejoin="round" opacity="{opacity}"/>"#,
        color = svg_color(arrow_color),
    );
}

/// One segment per row (the current one lit), or a track with a marker when
/// there are too many rows for segments.
fn push_rows(svg: &mut String, zone: ZoneGeometry, rows: RowPosition) {
    let left = zone.width * ROWS_MARGIN_RATIO;
    let length = zone.width * (1.0 - 2.0 * ROWS_MARGIN_RATIO);
    let top = zone.y(ROWS_TOP_RATIO);
    let thickness = zone.y(ROWS_THICKNESS_RATIO);
    let radius = thickness / 2.0;
    let current = rows.row();
    let segments = rows.count().min(MAX_ROW_SEGMENTS);
    if rows.count() > MAX_ROW_SEGMENTS {
        let marker = length / f32_from(rows.count());
        push_svg!(
            svg,
            r#"<rect x="{left}" y="{top}" width="{length}" height="{thickness}" rx="{radius}" fill="{track}"/><rect x="{x}" y="{top}" width="{marker}" height="{thickness}" rx="{radius}" fill="url(#{GRADIENT_ID})"/>"#,
            track = svg_color(OTHER_ROW_COLOR),
            x = left + marker * f32_from(current),
        );
        return;
    }
    let gap = zone.width * ROWS_GAP_RATIO;
    let segment = (length - gap * f32_from(segments - 1)) / f32_from(segments);
    for index in 0..segments {
        let fill = if index == current {
            format!("url(#{GRADIENT_ID})")
        } else {
            svg_color(OTHER_ROW_COLOR)
        };
        push_svg!(
            svg,
            r#"<rect x="{x}" y="{top}" width="{segment}" height="{thickness}" rx="{radius}" fill="{fill}"/>"#,
            x = left + (segment + gap) * f32_from(index),
        );
    }
}
