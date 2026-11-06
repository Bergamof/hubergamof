//! Menu zone of a dial: a title in capitals and up to three entries, the
//! selected one in a pill with a chevron.
//!
//! Ratios of the zone, designed on a 200 × 100 px zone (Stream Deck+).

use std::sync::Arc;

use hubergamof_module_api::MenuZone;
use hubergamof_protocol::ImageSize;
use image::{Rgba, RgbaImage};

use super::source::svg_color;
use super::svg::{push_svg, shape_gradient};
use super::text::{LineStyle, TextLine};
use super::{
    CacheKey, DIMMED_TEXT_COLOR, RenderError, Rendered, Renderer, TEXT_COLOR, ZoneGeometry,
    check_size, keep_opaque, rgba, semibold,
};

/// Identifier of the gradient of the selected entry.
const GRADIENT_ID: &str = "selection";

/// Left of the title, relative to the zone width.
const TITLE_LEFT_RATIO: f32 = 0.08;
/// Font size of the title, relative to the zone height.
const TITLE_FONT_RATIO: f32 = 0.095;
/// Baseline of the title, relative to the zone height.
const TITLE_BASELINE_RATIO: f32 = 0.19;

/// Top of the first entry, relative to the zone height.
const ENTRIES_TOP_RATIO: f32 = 0.26;
/// Distance between the tops of two entries, relative to the zone height.
const ENTRY_PITCH_RATIO: f32 = 0.24;
/// Height of an entry, relative to the zone height.
const ENTRY_HEIGHT_RATIO: f32 = 0.21;
/// Left of the pill of the selected entry, relative to the zone width.
const ENTRY_LEFT_RATIO: f32 = 0.05;
/// Width of the pill, relative to the zone width.
const ENTRY_WIDTH_RATIO: f32 = 0.90;
/// Fill opacity of the pill.
const PILL_FILL_OPACITY: f32 = 0.2;
/// Outline of the pill, relative to the zone height.
const PILL_OUTLINE_RATIO: f32 = 0.014;

/// Left of the text of the selected entry, relative to the zone width.
const SELECTED_TEXT_LEFT_RATIO: f32 = 0.17;
/// Left of the text of the other entries, relative to the zone width.
const OTHER_TEXT_LEFT_RATIO: f32 = 0.12;
/// Right margin of the entry texts, relative to the zone width.
const TEXT_RIGHT_MARGIN_RATIO: f32 = 0.08;
/// Font size of the entries, relative to the zone height.
const ENTRY_FONT_RATIO: f32 = 0.12;
/// Baseline of an entry text below its top, relative to the zone height.
const ENTRY_BASELINE_RATIO: f32 = 0.148;

/// Left of the chevron, relative to the zone width.
const CHEVRON_LEFT_RATIO: f32 = 0.11;
/// Width of the chevron, relative to the zone width.
const CHEVRON_WIDTH_RATIO: f32 = 0.025;
/// Top and bottom of the chevron below the top of the entry, relative to the
/// zone height.
const CHEVRON_TOP_RATIO: f32 = 0.065;
const CHEVRON_BOTTOM_RATIO: f32 = 0.145;
/// Stroke of the chevron, relative to the zone height.
const CHEVRON_STROKE_RATIO: f32 = 0.02;

impl Renderer {
    /// Renders the menu zone of a dial. CPU-bound: call it from a blocking
    /// context.
    ///
    /// # Errors
    ///
    /// [`RenderError::InvalidSize`] when the size is empty.
    pub fn render_menu_zone(
        &self,
        menu: &MenuZone,
        size: ImageSize,
        background: Rgba<u8>,
    ) -> Result<Arc<Rendered>, RenderError> {
        check_size(size)?;
        let key = CacheKey::Menu {
            menu: menu.clone(),
            size: (size.width, size.height),
            background: background.0,
        };
        self.cached(key, || {
            let mut canvas = RgbaImage::from_pixel(size.width, size.height, background);
            let zone = ZoneGeometry::of_size(size);
            self.text
                .overlay_svg(&mut canvas, &menu_shapes(menu, zone))?;
            self.text.draw(&mut canvas, &self.menu_lines(menu, zone)?)?;
            Ok(keep_opaque(canvas, background))
        })
    }

    /// The title in capitals, then the entries.
    fn menu_lines(
        &self,
        menu: &MenuZone,
        zone: ZoneGeometry,
    ) -> Result<Vec<TextLine>, RenderError> {
        let mut lines = vec![self.left_line(
            &menu.title().to_uppercase(),
            semibold(zone.y(TITLE_FONT_RATIO), rgba(menu.accent().from)),
            (zone.width * TITLE_LEFT_RATIO, zone.y(TITLE_BASELINE_RATIO)),
            zone.width * (1.0 - TITLE_LEFT_RATIO - TEXT_RIGHT_MARGIN_RATIO),
        )?];
        for (index, entry) in menu.entries().iter().enumerate() {
            let selected = index == menu.selected();
            let (left_ratio, color) = if selected {
                (SELECTED_TEXT_LEFT_RATIO, TEXT_COLOR)
            } else {
                (OTHER_TEXT_LEFT_RATIO, DIMMED_TEXT_COLOR)
            };
            let left = zone.width * left_ratio;
            let style = semibold(zone.y(ENTRY_FONT_RATIO), color);
            lines.push(self.left_line(
                entry,
                style,
                (left, entry_top(zone, index) + zone.y(ENTRY_BASELINE_RATIO)),
                zone.width * (1.0 - TEXT_RIGHT_MARGIN_RATIO) - left,
            )?);
        }
        Ok(lines)
    }

    /// `text` starting at `origin.0` (the text renderer centres lines), at
    /// baseline `origin.1`, shortened to `max_width`.
    fn left_line(
        &self,
        text: &str,
        style: LineStyle,
        origin: (f32, f32),
        max_width: f32,
    ) -> Result<TextLine, RenderError> {
        let line = self
            .text
            .fitted_line(text, style, 0.0, origin.1, max_width)?;
        let width = self.text.width(&line.text, line.size, line.weight)?;
        Ok(TextLine {
            center_x: origin.0 + width / 2.0,
            ..line
        })
    }
}

fn entry_top(zone: ZoneGeometry, index: usize) -> f32 {
    zone.y(ENTRIES_TOP_RATIO + ENTRY_PITCH_RATIO * f32::from(u8::try_from(index).unwrap_or(u8::MAX)))
}

/// The pill and the chevron of the selected entry.
fn menu_shapes(menu: &MenuZone, zone: ZoneGeometry) -> String {
    let mut svg = shape_gradient(
        GRADIENT_ID,
        (rgba(menu.accent().from), rgba(menu.accent().to)),
    );
    if menu.entries().is_empty() {
        return svg;
    }
    let top = entry_top(zone, menu.selected());
    let height = zone.y(ENTRY_HEIGHT_RATIO);
    push_svg!(
        svg,
        r#"<rect x="{x}" y="{top}" width="{width}" height="{height}" rx="{radius}" fill="url(#{GRADIENT_ID})" fill-opacity="{PILL_FILL_OPACITY}" stroke="url(#{GRADIENT_ID})" stroke-width="{outline}"/>"#,
        x = zone.width * ENTRY_LEFT_RATIO,
        width = zone.width * ENTRY_WIDTH_RATIO,
        radius = height / 2.0,
        outline = zone.y(PILL_OUTLINE_RATIO),
    );
    let chevron_left = zone.width * CHEVRON_LEFT_RATIO;
    push_svg!(
        svg,
        r#"<path d="M{chevron_left} {upper}L{tip} {middle}L{chevron_left} {lower}" fill="none" stroke="{color}" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round"/>"#,
        upper = top + zone.y(CHEVRON_TOP_RATIO),
        middle = top + height / 2.0,
        lower = top + zone.y(CHEVRON_BOTTOM_RATIO),
        tip = chevron_left + zone.width * CHEVRON_WIDTH_RATIO,
        color = svg_color(rgba(menu.accent().from)),
        stroke = zone.y(CHEVRON_STROKE_RATIO),
    );
    svg
}
