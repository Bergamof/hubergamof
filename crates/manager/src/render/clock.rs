//! Clock face of a strip zone: calendar leaf on the left, light time on the
//! right with the seconds as a superscript, week caption and a 60 s progress
//! track.
//!
//! The layout is expressed in ratios of the zone, like the gauge, and was
//! designed on a 200 × 100 px zone (Stream Deck+).

use std::sync::Arc;

use hubergamof_module_api::{CalendarLeaf, ClockFace, ClockSeconds};
use hubergamof_protocol::ImageSize;
use image::{Rgba, RgbaImage};

use super::source::svg_color;
use super::svg::push_svg;
use super::text::{FontWeight, LineStyle, TextLine, TextRenderer};
use super::{
    CacheKey, GAUGE_TRACK_COLOR, RenderError, Rendered, Renderer, TEXT_COLOR, ZoneGeometry,
    check_size, keep_opaque,
};

/// Seconds, minutes and hours accent: the teal of the clock icon.
pub(super) const CLOCK_ACCENT_COLOR: Rgba<u8> = Rgba([0x5e, 0xea, 0xd4, u8::MAX]);
/// Band of the calendar leaf and filled part of the seconds track.
pub(super) const CLOCK_DEEP_ACCENT_COLOR: Rgba<u8> = Rgba([0x14, 0xb8, 0xa6, u8::MAX]);
/// Body of the calendar leaf.
const LEAF_COLOR: Rgba<u8> = Rgba([0x15, 0x1b, 0x1a, u8::MAX]);
/// Weekday on the band of the leaf: dark on the deep accent.
const LEAF_BAND_TEXT_COLOR: Rgba<u8> = Rgba([0x04, 0x20, 0x1c, u8::MAX]);
/// Month, caption: secondary text.
const DETAIL_COLOR: Rgba<u8> = Rgba([0x8f, 0xa1, 0x9d, u8::MAX]);

/// Left of the leaf, relative to the zone width.
const LEAF_LEFT_RATIO: f32 = 0.05;
/// Width of the leaf, relative to the zone width.
const LEAF_WIDTH_RATIO: f32 = 0.31;
/// Top of the leaf, relative to the zone height.
const LEAF_TOP_RATIO: f32 = 0.13;
/// Height of the leaf, relative to the zone height.
const LEAF_HEIGHT_RATIO: f32 = 0.74;
/// Height of the weekday band, relative to the zone height.
const LEAF_BAND_RATIO: f32 = 0.16;
/// Corner radius of the leaf, relative to the zone height.
const LEAF_RADIUS_RATIO: f32 = 0.08;
/// Baseline of the weekday, relative to the zone height.
const LEAF_WEEKDAY_BASELINE_RATIO: f32 = 0.26;
/// Font size of the weekday, relative to the zone height.
const LEAF_WEEKDAY_FONT_RATIO: f32 = 0.12;
/// Baseline of the day, relative to the zone height.
const LEAF_DAY_BASELINE_RATIO: f32 = 0.64;
/// Font size of the day, relative to the zone height.
const LEAF_DAY_FONT_RATIO: f32 = 0.30;
/// Baseline of the month, relative to the zone height.
const LEAF_MONTH_BASELINE_RATIO: f32 = 0.80;
/// Font size of the month, relative to the zone height.
const LEAF_MONTH_FONT_RATIO: f32 = 0.13;

/// Left of the time area when the leaf is displayed, relative to the width.
const AREA_LEFT_WITH_LEAF_RATIO: f32 = 0.41;
/// Width of the time area when the leaf is displayed, relative to the width.
const AREA_WIDTH_WITH_LEAF_RATIO: f32 = 0.54;
/// Left (and right) padding of the time area without leaf, relative to the
/// width.
const AREA_PADDING_RATIO: f32 = 0.04;

/// Font size of the time without seconds, relative to the zone height.
const TIME_FONT_RATIO: f32 = 0.40;
/// Baseline of the time without seconds, relative to the zone height.
const TIME_BASELINE_RATIO: f32 = 0.58;
/// Font size of the time with seconds: smaller to leave room for them.
const TIME_WITH_SECONDS_FONT_RATIO: f32 = 0.34;
/// Baseline of the time with seconds, relative to the zone height.
const TIME_WITH_SECONDS_BASELINE_RATIO: f32 = 0.52;
/// Font size of the seconds superscript, relative to the zone height.
const SECONDS_FONT_RATIO: f32 = 0.15;
/// How far the seconds baseline is above the time baseline, relative to the
/// zone height.
const SECONDS_RAISE_RATIO: f32 = 0.18;
/// Gap between the time and the seconds, relative to the zone height.
const SECONDS_GAP_RATIO: f32 = 0.03;

/// Font size of the caption, relative to the zone height.
const CAPTION_FONT_RATIO: f32 = 0.13;
/// Baseline of the caption without seconds, relative to the zone height.
const CAPTION_BASELINE_RATIO: f32 = 0.80;
/// Baseline of the caption with seconds, relative to the zone height.
const CAPTION_WITH_SECONDS_BASELINE_RATIO: f32 = 0.72;
/// Text measured for the room kept for the seconds: digits have the same
/// width, so the time does not move from one second to the next.
const SECONDS_WIDTH_SAMPLE: &str = "00";

/// Top of the seconds track, relative to the zone height.
const TRACK_TOP_RATIO: f32 = 0.83;
/// Thickness of the seconds track, relative to the zone height.
const TRACK_THICKNESS_RATIO: f32 = 0.04;
/// Height of the quarter marks, relative to the zone height.
const TRACK_MARK_HEIGHT_RATIO: f32 = 0.08;
/// Width of the quarter marks, relative to the zone width (1 px on 200).
const TRACK_MARK_WIDTH_RATIO: f32 = 0.005;
/// Seconds between two quarter marks of the track.
const TRACK_MARK_STEP_SECONDS: u8 = ClockSeconds::PER_MINUTE / 4;

/// Geometry of a clock face on a zone of a given size.
struct Layout {
    zone: ZoneGeometry,
    /// Left and width of the area holding the time.
    area_left: f32,
    area_width: f32,
}

impl Layout {
    fn new(size: ImageSize, has_leaf: bool) -> Self {
        let zone = ZoneGeometry::of_size(size);
        let width = zone.width;
        let (area_left, area_width) = if has_leaf {
            (
                width * AREA_LEFT_WITH_LEAF_RATIO,
                width * AREA_WIDTH_WITH_LEAF_RATIO,
            )
        } else {
            (
                width * AREA_PADDING_RATIO,
                width * (1.0 - 2.0 * AREA_PADDING_RATIO),
            )
        };
        Self {
            zone,
            area_left,
            area_width,
        }
    }

    fn area_center(&self) -> f32 {
        self.area_left + self.area_width / 2.0
    }

    /// Vertical position at `ratio` of the zone height.
    fn y(&self, ratio: f32) -> f32 {
        self.zone.y(ratio)
    }

    fn leaf_left(&self) -> f32 {
        self.zone.width * LEAF_LEFT_RATIO
    }

    fn leaf_width(&self) -> f32 {
        self.zone.width * LEAF_WIDTH_RATIO
    }

    fn leaf_center(&self) -> f32 {
        self.leaf_left() + self.leaf_width() / 2.0
    }
}

impl Renderer {
    /// Renders a clock face on a strip zone. CPU-bound: call it from a
    /// blocking context.
    ///
    /// # Errors
    ///
    /// [`RenderError::InvalidSize`] when the size is empty.
    pub fn render_clock(
        &self,
        face: &ClockFace,
        size: ImageSize,
        background: Rgba<u8>,
    ) -> Result<Arc<Rendered>, RenderError> {
        check_size(size)?;
        let key = CacheKey::Clock {
            face: face.clone(),
            size: (size.width, size.height),
            background: background.0,
        };
        self.cached(key, || {
            let mut canvas = RgbaImage::from_pixel(size.width, size.height, background);
            let layout = Layout::new(size, face.leaf.is_some());
            let shapes = clock_shapes(face, &layout, background);
            self.text.overlay_svg(&mut canvas, &shapes)?;
            self.text
                .draw(&mut canvas, &self.clock_lines(face, size)?)?;
            Ok(keep_opaque(canvas, background))
        })
    }

    /// The texts of the face: leaf, time with its seconds, caption.
    pub(super) fn clock_lines(
        &self,
        face: &ClockFace,
        size: ImageSize,
    ) -> Result<Vec<TextLine>, RenderError> {
        let layout = &Layout::new(size, face.leaf.is_some());
        let mut lines = Vec::new();
        if let Some(leaf) = &face.leaf {
            self.push_leaf_lines(&mut lines, leaf, layout)?;
        }
        self.push_time_lines(&mut lines, face, layout)?;
        if let Some(caption) = &face.caption {
            let baseline = if face.seconds.is_some() {
                CAPTION_WITH_SECONDS_BASELINE_RATIO
            } else {
                CAPTION_BASELINE_RATIO
            };
            lines.push(self.text.fitted_line(
                caption,
                semibold(layout.y(CAPTION_FONT_RATIO), DETAIL_COLOR),
                layout.area_center(),
                layout.y(baseline),
                layout.area_width,
            )?);
        }
        Ok(lines)
    }

    /// Weekday, day and month of the leaf; the weekday and the month in
    /// capitals, whatever the case the module gives.
    fn push_leaf_lines(
        &self,
        lines: &mut Vec<TextLine>,
        leaf: &CalendarLeaf,
        layout: &Layout,
    ) -> Result<(), RenderError> {
        let parts = [
            (
                leaf.weekday.to_uppercase(),
                LEAF_WEEKDAY_FONT_RATIO,
                LEAF_WEEKDAY_BASELINE_RATIO,
                LEAF_BAND_TEXT_COLOR,
            ),
            (
                leaf.day.to_string(),
                LEAF_DAY_FONT_RATIO,
                LEAF_DAY_BASELINE_RATIO,
                TEXT_COLOR,
            ),
            (
                leaf.month.to_uppercase(),
                LEAF_MONTH_FONT_RATIO,
                LEAF_MONTH_BASELINE_RATIO,
                DETAIL_COLOR,
            ),
        ];
        for (text, font_ratio, baseline_ratio, color) in parts {
            lines.push(self.text.fitted_line(
                &text,
                semibold(layout.y(font_ratio), color),
                layout.leaf_center(),
                layout.y(baseline_ratio),
                layout.leaf_width(),
            )?);
        }
        Ok(())
    }

    /// The time, centred in its area, with the seconds as a superscript when
    /// displayed: the pair is centred as a whole.
    fn push_time_lines(
        &self,
        lines: &mut Vec<TextLine>,
        face: &ClockFace,
        layout: &Layout,
    ) -> Result<(), RenderError> {
        let Some(seconds) = face.seconds else {
            lines.push(self.text.fitted_line(
                &face.time,
                light(layout.y(TIME_FONT_RATIO), TEXT_COLOR),
                layout.area_center(),
                layout.y(TIME_BASELINE_RATIO),
                layout.area_width,
            )?);
            return Ok(());
        };
        let seconds_style = semibold(layout.y(SECONDS_FONT_RATIO), CLOCK_ACCENT_COLOR);
        let gap = layout.y(SECONDS_GAP_RATIO);
        let seconds_width = self.text.width(
            SECONDS_WIDTH_SAMPLE,
            seconds_style.size,
            seconds_style.weight,
        )?;
        let time_baseline = layout.y(TIME_WITH_SECONDS_BASELINE_RATIO);
        let time = self.text.fitted_line(
            &face.time,
            light(layout.y(TIME_WITH_SECONDS_FONT_RATIO), TEXT_COLOR),
            0.0,
            time_baseline,
            layout.area_width - gap - seconds_width,
        )?;
        let time_width = self.text.width(&time.text, time.size, time.weight)?;
        let start = layout.area_center() - (time_width + gap + seconds_width) / 2.0;
        lines.push(TextLine {
            center_x: start + time_width / 2.0,
            ..time
        });
        lines.push(TextRenderer::line(
            &format!("{:02}", seconds.get()),
            seconds_style,
            start + time_width + gap + seconds_width / 2.0,
            time_baseline - layout.y(SECONDS_RAISE_RATIO),
        ));
        Ok(())
    }
}

fn semibold(size: f32, color: Rgba<u8>) -> LineStyle {
    LineStyle {
        size,
        weight: FontWeight::SemiBold,
        color,
    }
}

fn light(size: f32, color: Rgba<u8>) -> LineStyle {
    LineStyle {
        size,
        weight: FontWeight::Light,
        color,
    }
}

/// The leaf and the seconds track, which carry no text, as SVG elements.
fn clock_shapes(face: &ClockFace, layout: &Layout, background: Rgba<u8>) -> String {
    let mut shapes = String::new();
    if face.leaf.is_some() {
        push_leaf_shapes(&mut shapes, layout);
    }
    if let Some(seconds) = face.seconds {
        push_track_shapes(&mut shapes, layout, seconds, background);
    }
    shapes
}

/// Body of the leaf, with its band at the top.
fn push_leaf_shapes(svg: &mut String, layout: &Layout) {
    let left = layout.leaf_left();
    let width = layout.leaf_width();
    let top = layout.y(LEAF_TOP_RATIO);
    let height = layout.y(LEAF_HEIGHT_RATIO);
    let band = layout.y(LEAF_BAND_RATIO);
    let radius = layout.y(LEAF_RADIUS_RATIO);
    let right = left + width;
    push_svg!(
        svg,
        r#"<rect x="{left}" y="{top}" width="{width}" height="{height}" rx="{radius}" fill="{body}"/><path d="M{left} {band_bottom}V{arc_start}a{radius} {radius} 0 0 1 {radius} -{radius}H{band_end}a{radius} {radius} 0 0 1 {radius} {radius}V{band_bottom}Z" fill="{band_color}"/>"#,
        band_bottom = top + band,
        arc_start = top + radius,
        band_end = right - radius,
        body = svg_color(LEAF_COLOR),
        band_color = svg_color(CLOCK_DEEP_ACCENT_COLOR),
    );
}

/// Track filled to `seconds` sixtieths (empty when a minute starts), with a
/// mark every quarter of a minute cut out of it in the colour of the
/// `background`, so that it stays visible on the fill.
fn push_track_shapes(
    svg: &mut String,
    layout: &Layout,
    seconds: ClockSeconds,
    background: Rgba<u8>,
) {
    let thickness = layout.y(TRACK_THICKNESS_RATIO);
    let top = layout.y(TRACK_TOP_RATIO);
    let left = layout.area_left;
    let length = layout.area_width;
    let minute = f32::from(ClockSeconds::PER_MINUTE);
    let filled = length * f32::from(seconds.get()) / minute;
    let radius = thickness / 2.0;
    push_svg!(
        svg,
        r#"<rect x="{left}" y="{top}" width="{length}" height="{thickness}" rx="{radius}" fill="{track}"/>"#,
        track = svg_color(GAUGE_TRACK_COLOR),
    );
    if seconds.get() > 0 {
        push_svg!(
            svg,
            r#"<rect x="{left}" y="{top}" width="{filled}" height="{thickness}" rx="{radius}" fill="{fill}"/>"#,
            fill = svg_color(CLOCK_ACCENT_COLOR),
        );
    }
    let mark_width = layout.zone.width * TRACK_MARK_WIDTH_RATIO;
    let mark_height = layout.y(TRACK_MARK_HEIGHT_RATIO);
    let mark_top = top + (thickness - mark_height) / 2.0;
    for mark in (TRACK_MARK_STEP_SECONDS..ClockSeconds::PER_MINUTE)
        .step_by(usize::from(TRACK_MARK_STEP_SECONDS))
    {
        let x = left + length * f32::from(mark) / minute - mark_width / 2.0;
        push_svg!(
            svg,
            r#"<rect x="{x}" y="{mark_top}" width="{mark_width}" height="{mark_height}" fill="{color}"/>"#,
            color = svg_color(background),
        );
    }
}
