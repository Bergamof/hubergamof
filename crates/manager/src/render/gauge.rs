//! Gauges of a strip zone: the bar (title, value, bar), the arc (volume) and
//! the ring (timer).
//!
//! Layouts are expressed in ratios of the zone, like the clock face, and were
//! designed on a 200 × 100 px zone (Stream Deck+).

use std::cmp::Ordering;
use std::f32::consts::{PI, TAU};
use std::fmt::Write as _;
use std::sync::Arc;

use hubergamof_module_api::{GaugeContent, GaugeLevel, GaugeShape, GaugeSteps, ImageData, Rgb};
use hubergamof_protocol::ImageSize;
use image::{Rgba, RgbaImage, imageops};

use super::source::{self, Decoded, f32_from, svg_color};
use super::text::{FontWeight, TextLine, TextRenderer};
use super::{
    CacheKey, DIMMED_TEXT_COLOR, GAUGE_DIMMED_FILL_COLOR, GAUGE_TRACK_COLOR, RenderError, Rendered,
    Renderer, TEXT_COLOR, ZONE_PADDING_RATIO, check_size, contain, keep_opaque, scaled_dimension,
    semibold,
};

/// Identifier of the gradient of the filled part, in the SVG of a gauge.
const ACCENT_GRADIENT_ID: &str = "accent";

/// Font size of a bar gauge title, relative to the zone height.
const BAR_TITLE_FONT_RATIO: f32 = 0.20;
/// Font size of a bar gauge value, relative to the zone height.
const BAR_VALUE_FONT_RATIO: f32 = 0.34;
/// Space above the bar gauge title, relative to the zone height.
const BAR_TOP_MARGIN_RATIO: f32 = 0.05;
/// Thickness of the bar, relative to the zone height.
const BAR_HEIGHT_RATIO: f32 = 0.12;
/// Space below the bar, relative to the zone height.
const BAR_BOTTOM_MARGIN_RATIO: f32 = 0.10;

/// Vertical centre of the arc (its ends rest on it), relative to the height.
const ARC_CENTER_Y_RATIO: f32 = 0.64;
/// Radius of the arc, relative to the zone height.
const ARC_RADIUS_RATIO: f32 = 0.44;
/// Thickness of the arc, relative to the zone height.
const ARC_THICKNESS_RATIO: f32 = 0.09;
/// Radius of the dot at the end of the filled part, relative to the height.
const ARC_KNOB_RADIUS_RATIO: f32 = 0.032;
/// Part of the inside of the arc the value may take.
const ARC_VALUE_WIDTH_SHARE: f32 = 0.9;
/// Font size of the value under the arc, relative to the zone height.
const ARC_VALUE_FONT_RATIO: f32 = 0.30;
/// Baseline of the value, relative to the zone height.
const ARC_VALUE_BASELINE_RATIO: f32 = 0.62;
/// Font size of the title below the arc, relative to the zone height.
const ARC_TITLE_FONT_RATIO: f32 = 0.13;
/// Baseline of the title, relative to the zone height.
const ARC_TITLE_BASELINE_RATIO: f32 = 0.91;
/// Side of the glyph before the title, relative to the zone height.
const ARC_GLYPH_RATIO: f32 = 0.16;
/// Gap between the glyph and the title, relative to the zone height.
const ARC_GLYPH_GAP_RATIO: f32 = 0.03;
/// Height of the middle of the lowercase letters above the baseline,
/// relative to the font size: where the glyph is centred next to the title.
const TEXT_MIDDLE_RATIO: f32 = 0.36;

/// Horizontal centre of the ring, relative to the zone width.
const RING_CENTER_X_RATIO: f32 = 0.23;
/// Vertical centre of the ring, relative to the zone height.
const RING_CENTER_Y_RATIO: f32 = 0.5;
/// Radius of the ring, relative to the zone height.
const RING_RADIUS_RATIO: f32 = 0.30;
/// Thickness of the ring, relative to the zone height.
const RING_THICKNESS_RATIO: f32 = 0.08;
/// Side of the glyph inside the ring, relative to the zone height.
const RING_GLYPH_RATIO: f32 = 0.30;
/// Horizontal centre of the texts beside the ring, relative to the width.
const RING_TEXT_CENTER_X_RATIO: f32 = 0.71;
/// Width of the texts beside the ring, relative to the zone width.
const RING_TEXT_WIDTH_RATIO: f32 = 0.50;
/// Font size of the label (the title in capitals), relative to the height.
const RING_LABEL_FONT_RATIO: f32 = 0.12;
/// Baseline of the label, relative to the zone height.
const RING_LABEL_BASELINE_RATIO: f32 = 0.30;
/// Font size of the value, relative to the zone height.
const RING_VALUE_FONT_RATIO: f32 = 0.31;
/// Baseline of the value, relative to the zone height.
const RING_VALUE_BASELINE_RATIO: f32 = 0.62;
/// Vertical centre of the dots, relative to the zone height.
const RING_DOTS_Y_RATIO: f32 = 0.80;
/// Radius of a dot, relative to the zone height.
const RING_DOT_RADIUS_RATIO: f32 = 0.045;
/// Distance between two dots, relative to the zone height (less when there
/// are too many to fit the texts area).
const RING_DOT_PITCH_RATIO: f32 = 0.16;
/// Thickness of the outline of the current dot, relative to the zone height.
const RING_DOT_OUTLINE_RATIO: f32 = 0.018;
/// Radius of the core of the current dot, relative to the zone height.
const RING_DOT_CORE_RATIO: f32 = 0.018;

/// The dot at the end of the filled part of an arc.
const KNOB_COLOR: Rgba<u8> = Rgba([u8::MAX, u8::MAX, u8::MAX, u8::MAX]);

/// Geometry shared by the shapes of a gauge on a zone of a given size.
struct Zone {
    width: f32,
    height: f32,
}

impl Zone {
    fn of(canvas: &RgbaImage) -> Self {
        Self {
            width: f32_from(canvas.width()),
            height: f32_from(canvas.height()),
        }
    }

    fn y(&self, ratio: f32) -> f32 {
        self.height * ratio
    }
}

impl Renderer {
    /// Renders a strip zone gauge in the shape the content asks for. A bar
    /// gauge has its title on top, its value in the middle and a bar at the
    /// bottom; an arc a half circle with the value inside and the title
    /// below; a ring a glyph inside the ring with the title, the value and
    /// the steps beside it. Everything is greyed out when the gauge is
    /// dimmed. CPU-bound: call it from a blocking context.
    ///
    /// # Errors
    ///
    /// [`RenderError::InvalidSize`] when the size is empty.
    pub fn render_gauge(
        &self,
        gauge: &GaugeContent,
        size: ImageSize,
        background: Rgba<u8>,
    ) -> Result<Arc<Rendered>, RenderError> {
        check_size(size)?;
        let key = CacheKey::Gauge {
            gauge: gauge.clone(),
            size: (size.width, size.height),
            background: background.0,
        };
        self.cached(key, || {
            let mut canvas = RgbaImage::from_pixel(size.width, size.height, background);
            match gauge.shape {
                GaugeShape::Arc => self.draw_arc_gauge(&mut canvas, gauge)?,
                GaugeShape::Ring => self.draw_ring_gauge(&mut canvas, gauge)?,
                _ => self.draw_bar_gauge(&mut canvas, gauge)?,
            }
            Ok(keep_opaque(canvas, background))
        })
    }

    fn draw_bar_gauge(
        &self,
        canvas: &mut RgbaImage,
        gauge: &GaugeContent,
    ) -> Result<(), RenderError> {
        let bar_top = draw_bar(canvas, gauge);
        self.draw_bar_text(canvas, gauge, bar_top)
    }

    /// Draws the title at the top of `canvas` and the value centred between
    /// the title and the bar starting at row `bar_top`.
    fn draw_bar_text(
        &self,
        canvas: &mut RgbaImage,
        gauge: &GaugeContent,
        bar_top: f32,
    ) -> Result<(), RenderError> {
        let zone = Zone::of(canvas);
        let max_width = zone.width * (1.0 - 2.0 * ZONE_PADDING_RATIO);
        let color = text_color(gauge);
        let title_size = zone.y(BAR_TITLE_FONT_RATIO);
        let value_size = zone.y(BAR_VALUE_FONT_RATIO);
        let title_top = zone.y(BAR_TOP_MARGIN_RATIO);
        let title_bottom = title_top + TextRenderer::line_height(title_size);
        let value_top =
            title_bottom + (bar_top - title_bottom - TextRenderer::line_height(value_size)) / 2.0;
        let lines = [
            self.text.fitted_line(
                &gauge.title,
                semibold(title_size, color),
                zone.width / 2.0,
                title_top + TextRenderer::ascent(title_size),
                max_width,
            )?,
            self.text.fitted_line(
                &gauge.value,
                semibold(value_size, color),
                zone.width / 2.0,
                value_top + TextRenderer::ascent(value_size),
                max_width,
            )?,
        ];
        self.text.draw(canvas, &lines)
    }

    /// Half circle filled from the left, the value inside it, the title (and
    /// its glyph) below.
    fn draw_arc_gauge(
        &self,
        canvas: &mut RgbaImage,
        gauge: &GaugeContent,
    ) -> Result<(), RenderError> {
        let zone = Zone::of(canvas);
        let thickness = zone.y(ARC_THICKNESS_RATIO);
        let radius = zone.y(ARC_RADIUS_RATIO).min(zone.width / 2.0 - thickness);
        let (center_x, center_y) = (zone.width / 2.0, zone.y(ARC_CENTER_Y_RATIO));
        let ratio = gauge.level.ratio();
        let mut shapes = accent_gradient(
            gauge,
            (center_x - radius, center_y - radius),
            (center_x + radius, center_y),
        );
        push_arc_shapes(
            &mut shapes,
            &ArcGeometry {
                center: (center_x, center_y),
                radius,
                thickness,
            },
            ratio,
        );
        if ratio > 0.0 && !gauge.dimmed {
            let angle = PI * ratio;
            let _ = write!(
                shapes,
                r#"<circle cx="{x}" cy="{y}" r="{knob}" fill="{color}"/>"#,
                x = center_x - radius * angle.cos(),
                y = center_y - radius * angle.sin(),
                knob = zone.y(ARC_KNOB_RADIUS_RATIO),
                color = svg_color(KNOB_COLOR),
            );
        }
        self.text.overlay_svg(canvas, &shapes)?;

        let inside = (radius - thickness) * 2.0 * ARC_VALUE_WIDTH_SHARE;
        let value = self.shrunk_line(
            &gauge.value,
            zone.y(ARC_VALUE_FONT_RATIO),
            text_color(gauge),
            (center_x, zone.y(ARC_VALUE_BASELINE_RATIO)),
            inside,
        )?;
        let mut lines = vec![value];
        self.push_arc_title(canvas, &mut lines, gauge, &zone)?;
        self.text.draw(canvas, &lines)
    }

    /// The title centred at the bottom, preceded by the glyph when there is
    /// one (drawn on `canvas` straight away).
    fn push_arc_title(
        &self,
        canvas: &mut RgbaImage,
        lines: &mut Vec<TextLine>,
        gauge: &GaugeContent,
        zone: &Zone,
    ) -> Result<(), RenderError> {
        let size = zone.y(ARC_TITLE_FONT_RATIO);
        let baseline = zone.y(ARC_TITLE_BASELINE_RATIO);
        let glyph_side = zone.y(ARC_GLYPH_RATIO);
        let glyph_room = gauge
            .glyph
            .as_ref()
            .map_or(0.0, |_| glyph_side + zone.y(ARC_GLYPH_GAP_RATIO));
        let max_width = zone.width * (1.0 - 2.0 * ZONE_PADDING_RATIO) - glyph_room;
        let title = self.text.fitted_line(
            &gauge.title,
            semibold(size, DIMMED_TEXT_COLOR),
            0.0,
            baseline,
            max_width,
        )?;
        let title_width = self.text.width(&title.text, title.size, title.weight)?;
        let start = (zone.width - (glyph_room + title_width)) / 2.0;
        if let Some(glyph) = &gauge.glyph {
            let top = baseline - size * TEXT_MIDDLE_RATIO - glyph_side / 2.0;
            self.draw_glyph(canvas, glyph, (start, top), glyph_side)?;
        }
        lines.push(TextLine {
            center_x: start + glyph_room + title_width / 2.0,
            ..title
        });
        Ok(())
    }

    /// Ring filled clockwise from the top with the glyph inside, the title
    /// in capitals, the value and the steps beside it.
    fn draw_ring_gauge(
        &self,
        canvas: &mut RgbaImage,
        gauge: &GaugeContent,
    ) -> Result<(), RenderError> {
        let zone = Zone::of(canvas);
        let center = (
            zone.width * RING_CENTER_X_RATIO,
            zone.y(RING_CENTER_Y_RATIO),
        );
        let radius = zone.y(RING_RADIUS_RATIO);
        let text_center = zone.width * RING_TEXT_CENTER_X_RATIO;
        let text_width = zone.width * RING_TEXT_WIDTH_RATIO;
        let mut shapes = accent_gradient(
            gauge,
            (center.0 - radius, center.1 - radius),
            (center.0 + radius, center.1 + radius),
        );
        push_ring_shapes(
            &mut shapes,
            &ArcGeometry {
                center,
                radius,
                thickness: zone.y(RING_THICKNESS_RATIO),
            },
            gauge.level,
        );
        if let Some(steps) = gauge.steps {
            push_step_dots(&mut shapes, gauge, steps, (text_center, text_width), &zone);
        }
        self.text.overlay_svg(canvas, &shapes)?;
        if let Some(glyph) = &gauge.glyph {
            let side = zone.y(RING_GLYPH_RATIO);
            let top_left = (center.0 - side / 2.0, center.1 - side / 2.0);
            self.draw_glyph(canvas, glyph, top_left, side)?;
        }
        let label_color = if gauge.dimmed {
            DIMMED_TEXT_COLOR
        } else {
            rgba(gauge.accent.from)
        };
        let lines = [
            self.text.fitted_line(
                &gauge.title.to_uppercase(),
                semibold(zone.y(RING_LABEL_FONT_RATIO), label_color),
                text_center,
                zone.y(RING_LABEL_BASELINE_RATIO),
                text_width,
            )?,
            self.shrunk_line(
                &gauge.value,
                zone.y(RING_VALUE_FONT_RATIO),
                text_color(gauge),
                (text_center, zone.y(RING_VALUE_BASELINE_RATIO)),
                text_width,
            )?,
        ];
        self.text.draw(canvas, &lines)
    }

    /// `text` centred on `anchor.0` at baseline `anchor.1`, in a font as
    /// large as `size` allows within `max_width`: a value that is too wide
    /// shrinks instead of being cut.
    fn shrunk_line(
        &self,
        text: &str,
        size: f32,
        color: Rgba<u8>,
        anchor: (f32, f32),
        max_width: f32,
    ) -> Result<TextLine, RenderError> {
        let width = self.text.width(text, size, FontWeight::SemiBold)?;
        let fitted = if width > max_width && width > 0.0 {
            size * max_width / width
        } else {
            size
        };
        Ok(TextRenderer::line(
            text,
            semibold(fitted, color),
            anchor.0,
            anchor.1,
        ))
    }

    /// Draws `glyph` in a square of `side` pixels whose top left corner is
    /// `top_left`, keeping the colours of the image.
    fn draw_glyph(
        &self,
        canvas: &mut RgbaImage,
        glyph: &ImageData,
        top_left: (f32, f32),
        side: f32,
    ) -> Result<(), RenderError> {
        let pixels = rounded_pixels(side);
        let square = ImageSize {
            width: pixels,
            height: pixels,
        };
        let layer = match source::decode(&glyph.bytes, self.text.source_svg_options())? {
            Decoded::Raster(image) => contain(&image, square),
            Decoded::Vector(tree) => source::rasterize_svg(&tree, pixels, pixels)?,
        };
        imageops::overlay(
            canvas,
            &layer,
            rounded_offset(top_left.0),
            rounded_offset(top_left.1),
        );
        Ok(())
    }
}

/// Colour of the value (and of the bar gauge title).
fn text_color(gauge: &GaugeContent) -> Rgba<u8> {
    if gauge.dimmed {
        DIMMED_TEXT_COLOR
    } else {
        TEXT_COLOR
    }
}

fn rgba(Rgb(red, green, blue): Rgb) -> Rgba<u8> {
    Rgba([red, green, blue, u8::MAX])
}

/// Pixels of a length, rounded, at least 1.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn rounded_pixels(length: f32) -> u32 {
    // A positive length of a few hundred pixels at most.
    length.round().max(1.0) as u32
}

/// A position in pixels, rounded.
#[allow(clippy::cast_possible_truncation)]
fn rounded_offset(position: f32) -> i64 {
    // Within the zone, a few hundred pixels at most.
    position.round() as i64
}

/// The gradient of the filled part, running from `start` to `end` in zone
/// coordinates, greyed out when the gauge is dimmed.
fn accent_gradient(gauge: &GaugeContent, start: (f32, f32), end: (f32, f32)) -> String {
    let (from, to) = if gauge.dimmed {
        (GAUGE_DIMMED_FILL_COLOR, GAUGE_DIMMED_FILL_COLOR)
    } else {
        (rgba(gauge.accent.from), rgba(gauge.accent.to))
    };
    format!(
        r#"<defs><linearGradient id="{ACCENT_GRADIENT_ID}" gradientUnits="userSpaceOnUse" x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}"><stop offset="0" stop-color="{from}"/><stop offset="1" stop-color="{to}"/></linearGradient></defs>"#,
        x1 = start.0,
        y1 = start.1,
        x2 = end.0,
        y2 = end.1,
        from = svg_color(from),
        to = svg_color(to),
    )
}

/// Where and how thick an arc or a ring is.
struct ArcGeometry {
    center: (f32, f32),
    radius: f32,
    thickness: f32,
}

/// Track of the half circle, then the part filled up to `ratio`.
fn push_arc_shapes(svg: &mut String, arc: &ArcGeometry, ratio: f32) {
    let (center_x, center_y) = arc.center;
    let (radius, thickness) = (arc.radius, arc.thickness);
    let start_x = center_x - radius;
    // Writing to a `String` cannot fail.
    let _ = write!(
        svg,
        r#"<path d="M{start_x} {center_y}A{radius} {radius} 0 0 1 {end_x} {center_y}" fill="none" stroke="{track}" stroke-width="{thickness}" stroke-linecap="round"/>"#,
        end_x = center_x + radius,
        track = svg_color(GAUGE_TRACK_COLOR),
    );
    if ratio > 0.0 {
        let angle = PI * ratio;
        let _ = write!(
            svg,
            r#"<path d="M{start_x} {center_y}A{radius} {radius} 0 0 1 {x} {y}" fill="none" stroke="url(#{ACCENT_GRADIENT_ID})" stroke-width="{thickness}" stroke-linecap="round"/>"#,
            x = center_x - radius * angle.cos(),
            y = center_y - radius * angle.sin(),
        );
    }
}

/// Track of the ring, then the part filled clockwise from the top.
fn push_ring_shapes(svg: &mut String, ring: &ArcGeometry, level: GaugeLevel) {
    let (center_x, center_y) = ring.center;
    let (radius, thickness) = (ring.radius, ring.thickness);
    // Writing to a `String` cannot fail.
    let _ = write!(
        svg,
        r#"<circle cx="{center_x}" cy="{center_y}" r="{radius}" fill="none" stroke="{track}" stroke-width="{thickness}"/>"#,
        track = svg_color(GAUGE_TRACK_COLOR),
    );
    let ratio = level.ratio();
    if level >= GaugeLevel::FULL {
        let _ = write!(
            svg,
            r#"<circle cx="{center_x}" cy="{center_y}" r="{radius}" fill="none" stroke="url(#{ACCENT_GRADIENT_ID})" stroke-width="{thickness}"/>"#,
        );
    } else if ratio > 0.0 {
        let angle = TAU * ratio;
        let _ = write!(
            svg,
            r#"<path d="M{center_x} {top}A{radius} {radius} 0 {large} 1 {x} {y}" fill="none" stroke="url(#{ACCENT_GRADIENT_ID})" stroke-width="{thickness}" stroke-linecap="round"/>"#,
            top = center_y - radius,
            large = u8::from(ratio > 0.5),
            x = center_x + radius * angle.sin(),
            y = center_y - radius * angle.cos(),
        );
    }
}

/// One dot per step under the texts: filled for the finished steps, outlined
/// for the current one, in the track colour for those to come.
fn push_step_dots(
    svg: &mut String,
    gauge: &GaugeContent,
    steps: GaugeSteps,
    area: (f32, f32),
    zone: &Zone,
) {
    let (center_x, width) = area;
    let total = f32::from(steps.total());
    let pitch = zone.y(RING_DOT_PITCH_RATIO).min(width / total);
    let first = center_x - pitch * (total - 1.0) / 2.0;
    let y = zone.y(RING_DOTS_Y_RATIO);
    let radius = zone.y(RING_DOT_RADIUS_RATIO);
    let accent = if gauge.dimmed {
        GAUGE_DIMMED_FILL_COLOR
    } else {
        rgba(gauge.accent.from)
    };
    for step in 1..=steps.total() {
        let x = first + pitch * f32::from(step - 1);
        // Writing to a `String` cannot fail.
        let _ = match step.cmp(&steps.current()) {
            Ordering::Less => write!(
                svg,
                r#"<circle cx="{x}" cy="{y}" r="{radius}" fill="{color}"/>"#,
                color = svg_color(accent),
            ),
            Ordering::Equal => write!(
                svg,
                r#"<circle cx="{x}" cy="{y}" r="{radius}" fill="none" stroke="{color}" stroke-width="{outline}"/><circle cx="{x}" cy="{y}" r="{core}" fill="{color}"/>"#,
                color = svg_color(accent),
                outline = zone.y(RING_DOT_OUTLINE_RATIO),
                core = zone.y(RING_DOT_CORE_RATIO),
            ),
            Ordering::Greater => write!(
                svg,
                r#"<circle cx="{x}" cy="{y}" r="{radius}" fill="{color}"/>"#,
                color = svg_color(GAUGE_TRACK_COLOR),
            ),
        };
    }
}

/// Draws the bar at the bottom of `canvas`, filled to the level from the
/// left; returns its top row.
fn draw_bar(canvas: &mut RgbaImage, gauge: &GaugeContent) -> f32 {
    let (width, height) = canvas.dimensions();
    let margin = scaled_dimension(width, ZONE_PADDING_RATIO, width / 2);
    let thickness = scaled_dimension(height, BAR_HEIGHT_RATIO, height);
    let bottom = height.saturating_sub(scaled_dimension(height, BAR_BOTTOM_MARGIN_RATIO, height));
    let top = bottom.saturating_sub(thickness);
    let track_end = width.saturating_sub(margin).max(margin);
    let filled_end = margin + bar_fill_width(track_end - margin, gauge.level);
    let fill = if gauge.dimmed {
        GAUGE_DIMMED_FILL_COLOR
    } else {
        super::GAUGE_FILL_COLOR
    };
    for y in top..bottom {
        for x in margin..track_end {
            let color = if x < filled_end {
                fill
            } else {
                GAUGE_TRACK_COLOR
            };
            canvas.put_pixel(x, y, color);
        }
    }
    f32_from(top)
}

/// Number of filled pixels of a bar `track` pixels long at `level`, rounded.
fn bar_fill_width(track: u32, level: GaugeLevel) -> u32 {
    let full = u64::from(GaugeLevel::FULL_PERMILLE);
    let filled = (u64::from(track) * u64::from(level.permille()) + full / 2) / full;
    // At most `track` since the level is at most a full bar.
    u32::try_from(filled).unwrap_or(track)
}
