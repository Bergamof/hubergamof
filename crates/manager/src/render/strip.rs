//! The touch strip as one image: time and date, machine load, page, volume
//! and pomodoro, the reminders of the dials under them, and the popup menu
//! over everything.
//!
//! Laid out for the 800 × 100 px strip of the Stream Deck+, in pixels: the
//! text is drawn by the same engine as everything else (Noto Sans).

use std::sync::Arc;

use hubergamof_module_api::{Feedback, PomodoroState, PopupIcon, VolumeWidget};
use hubergamof_protocol::{DeviceTheme, ImageSize};
use image::{Rgba, RgbaImage};

use super::source::{f32_from, svg_color};
use super::svg::push_svg;
use super::text::{FontWeight, escape_xml};
use super::{RenderError, Rendered, Renderer, check_size, keep_opaque};
use crate::strip::{HintFace, MeasureFace, PageFace, PomodoroFace, PopupFace, StripFace};

/// Colours of a theme.
#[derive(Clone, Copy)]
struct Palette {
    background: Rgba<u8>,
    card: Rgba<u8>,
    ink: Rgba<u8>,
    muted: Rgba<u8>,
    track: Rgba<u8>,
    line: Rgba<u8>,
    cpu: Rgba<u8>,
    ram: Rgba<u8>,
    swap: Rgba<u8>,
    volume: Rgba<u8>,
    working: Rgba<u8>,
    resting: Rgba<u8>,
    paused: Rgba<u8>,
    alert: Rgba<u8>,
    ok: Rgba<u8>,
}

const fn rgb(red: u8, green: u8, blue: u8) -> Rgba<u8> {
    Rgba([red, green, blue, u8::MAX])
}

const LIGHT: Palette = Palette {
    background: rgb(0xe7, 0xeb, 0xf1),
    card: rgb(0xf6, 0xf8, 0xfb),
    ink: rgb(0x13, 0x21, 0x3b),
    muted: rgb(0x56, 0x61, 0x78),
    track: rgb(0xcd, 0xd5, 0xe0),
    line: rgb(0xc3, 0xcc, 0xd9),
    cpu: rgb(0xe0, 0x50, 0x2a),
    ram: rgb(0x1d, 0x7f, 0xd6),
    swap: rgb(0x7b, 0x50, 0xd4),
    volume: rgb(0x0d, 0x8f, 0x7b),
    working: rgb(0xd8, 0x3a, 0x1f),
    resting: rgb(0x1a, 0x8f, 0x5a),
    paused: rgb(0xb7, 0x79, 0x1f),
    alert: rgb(0xc6, 0x28, 0x28),
    ok: rgb(0x2e, 0x9d, 0x4a),
};

const DARK: Palette = Palette {
    background: rgb(0x15, 0x1a, 0x23),
    card: rgb(0x22, 0x2a, 0x37),
    ink: rgb(0xee, 0xf2, 0xf8),
    muted: rgb(0x8e, 0x9a, 0xb0),
    track: rgb(0x2a, 0x33, 0x42),
    line: rgb(0x2c, 0x35, 0x44),
    cpu: rgb(0xff, 0x7a, 0x52),
    ram: rgb(0x4a, 0xa8, 0xff),
    swap: rgb(0xa9, 0x8b, 0xff),
    volume: rgb(0x34, 0xc9, 0xae),
    working: rgb(0xff, 0x5a, 0x3d),
    resting: rgb(0x4f, 0xd1, 0x8a),
    paused: rgb(0xf5, 0xb8, 0x3d),
    alert: rgb(0xff, 0x52, 0x52),
    ok: rgb(0x3f, 0xbf, 0x62),
};

/// Background and ink of the surface of `theme`: the keys are drawn on the
/// background of the touch strip.
pub(super) fn surface(theme: DeviceTheme) -> (Rgba<u8>, Rgba<u8>) {
    let palette = Palette::of(theme);
    (palette.background, palette.ink)
}

/// Accent colour of `theme`: the one of what is selected.
pub(super) fn accent(theme: DeviceTheme) -> Rgba<u8> {
    Palette::of(theme).working
}

const WHITE: Rgba<u8> = rgb(0xff, 0xff, 0xff);

impl Palette {
    fn of(theme: DeviceTheme) -> &'static Self {
        match theme {
            DeviceTheme::Light => &LIGHT,
            DeviceTheme::Dark => &DARK,
        }
    }

    fn measure(&self, measure: &MeasureFace) -> Rgba<u8> {
        if measure.alert {
            return self.alert;
        }
        match measure.label {
            "RAM" => self.ram,
            "SWAP" => self.swap,
            _ => self.cpu,
        }
    }

    fn pomodoro(&self, state: PomodoroState) -> Rgba<u8> {
        match state {
            PomodoroState::Working => self.working,
            PomodoroState::Resting => self.resting,
            PomodoroState::Paused => self.paused,
            _ => self.track,
        }
    }
}

/// Left margin and width taken by the clock.
const CLOCK_LEFT: f32 = 18.0;
const TIME_SIZE: f32 = 56.0;
const TIME_BASELINE: f32 = 54.0;
const TIME_SPACING: f32 = -1.5;
const SECONDS_SIZE: f32 = 22.0;
const SECONDS_GAP: f32 = 8.0;
const DATE_SIZE: f32 = 11.0;
const DATE_BASELINE: f32 = 71.0;

/// Vertical rules between the groups.
const CLOCK_RULE_X: f32 = 224.0;
const SYSTEM_RULE_X: f32 = 528.0;

/// Whether a touch at horizontal position `x` lands on the system measures,
/// the group between the two vertical rules.
#[must_use]
pub fn is_system_panel(x: u16) -> bool {
    (CLOCK_RULE_X..SYSTEM_RULE_X).contains(&f32::from(x))
}
const RULE_TOP: f32 = 12.0;
const RULE_HEIGHT: f32 = 58.0;

/// System measures: three rows.
const MEASURE_LABEL_X: f32 = 240.0;
const MEASURE_CURVE_X: f32 = 282.0;
const MEASURE_CURVE_WIDTH: f32 = 170.0;
const MEASURE_CURVE_HEIGHT: f32 = 14.0;
const MEASURE_VALUE_RIGHT: f32 = 508.0;
const MEASURE_FIRST_BASELINE: f32 = 26.0;
const MEASURE_PITCH: f32 = 20.0;
const MEASURE_BAR_HEIGHT: f32 = 5.0;
const MEASURE_LABEL_SIZE: f32 = 10.0;
const MEASURE_VALUE_SIZE: f32 = 15.0;

/// Right column: page, volume, pomodoro.
const COLUMN_LEFT: f32 = 546.0;
const COLUMN_RIGHT: f32 = 782.0;
const PAGE_BASELINE: f32 = 21.0;
const PAGE_SIZE: f32 = 12.5;
const PAGE_DOT_Y: f32 = 17.0;
const PAGE_DOT_RADIUS: f32 = 3.2;
const PAGE_DOT_PITCH: f32 = 12.0;
const PAGE_GAP: f32 = 10.0;
/// Most scroll positions drawn as dots; beyond, the position is written.
const MAX_PAGE_DOTS: u32 = 8;
const VOLUME_ICON_Y: f32 = 30.0;
const VOLUME_ICON_SIZE: f32 = 12.0;
const VOLUME_BAR_Y: f32 = 35.0;
const VOLUME_BAR_HEIGHT: f32 = 3.0;
const VOLUME_VALUE_BASELINE: f32 = 40.0;
const VOLUME_VALUE_SIZE: f32 = 11.0;
const VOLUME_VALUE_ROOM: f32 = 30.0;
const POMODORO_DOT_Y: f32 = 56.0;
const POMODORO_DOT_RADIUS: f32 = 4.5;
const POMODORO_LABEL_BASELINE: f32 = 60.0;
const POMODORO_LABEL_SIZE: f32 = 12.0;
const POMODORO_TIME_BASELINE: f32 = 63.0;
const POMODORO_TIME_SIZE: f32 = 26.0;
const PROGRESS_X: f32 = 16.0;
const PROGRESS_Y: f32 = 75.0;
const PROGRESS_HEIGHT: f32 = 4.0;

/// Reminders of the dials, under everything.
/// Share of the ink (out of 255) of the text of a reminder over a notification.
const REMINDER_TEXT_SHARE: u8 = 200;
/// Same, for the rule above the reminders.
const REMINDER_RULE_SHARE: u8 = 90;
const HINT_MARGIN: f32 = 16.0;
/// Top of the reminders of the dials, also the bottom of what a surface
/// drawn over the strip may keep for itself.
pub(super) const HINT_RULE_Y: f32 = 83.5;
const HINT_ICON_Y: f32 = 86.5;
const HINT_ICON_SIZE: f32 = 10.0;
const HINT_BASELINE: f32 = 95.0;
const HINT_SIZE: f32 = 10.5;
const HINT_ICON_GAP: f32 = 4.0;
const HINT_PART_GAP: f32 = 10.0;
const HINT_MARK_RADIUS: f32 = 5.0;
/// Bottom of the area the popup dims: above the reminders.
const POPUP_DIMMED_HEIGHT: f32 = 83.0;
const POPUP_DIM_OPACITY: f32 = 0.62;
const POPUP_TOP: f32 = 8.0;
const POPUP_HEIGHT: f32 = 70.0;
const POPUP_PADDING: f32 = 10.0;
const POPUP_CELL_WIDTH: f32 = 130.0;
const POPUP_CELL_GAP: f32 = 6.0;
const POPUP_CELL_TOP: f32 = 14.0;
const POPUP_CELL_HEIGHT: f32 = 58.0;
const POPUP_ICON_SIZE: f32 = 18.0;
const POPUP_ICON_Y: f32 = 21.0;
const POPUP_LABEL_BASELINE: f32 = 60.0;
const POPUP_LABEL_SIZE: f32 = 11.5;
const POPUP_LABEL_MARGIN: f32 = 8.0;
const POPUP_POINTER_HALF: f32 = 8.0;
const POPUP_POINTER_HEIGHT: f32 = 8.0;
/// Layers of the shadow under the menu, how far each reaches further, its offset and opacity.
const POPUP_SHADOW_LAYERS: u32 = 4;
const POPUP_SHADOW_STEP: f32 = 2.0;
const POPUP_SHADOW_OFFSET: f32 = 3.0;
const POPUP_SHADOW_OPACITY: f32 = 0.07;
const POPUP_RADIUS: f32 = 14.0;
/// Space kept between the popup and the ends of the strip.
const POPUP_EDGE: f32 = 8.0;

/// The glyphs drawn on the strip, in a 24 × 24 box.
#[derive(Debug, Clone, Copy)]
enum Glyph {
    Rotate,
    Press,
    Volume,
    Muted,
    Play,
    Pause,
    Skip,
    Reset,
}

impl Glyph {
    /// SVG elements of the glyph, in `color`.
    fn markup(self, color: &str) -> String {
        let stroke = |width: &str| {
            format!(
                r#"fill="none" stroke="{color}" stroke-width="{width}" stroke-linecap="round" stroke-linejoin="round""#
            )
        };
        let speaker = "M11 5 6 9H2v6h4l5 4z";
        match self {
            Self::Rotate => format!(
                r#"<g {}><path d="M20 12a8 8 0 1 1-2.5-5.8"/><path d="M20 4v5h-5"/></g>"#,
                stroke("2.6")
            ),
            Self::Press => format!(
                r#"<g {}><circle cx="12" cy="12" r="9"/></g><circle cx="12" cy="12" r="3.2" fill="{color}"/>"#,
                stroke("2.6")
            ),
            Self::Volume => format!(
                r#"<g {}><path d="{speaker}"/><path d="M15.5 8.5a5 5 0 0 1 0 7M19 5a10 10 0 0 1 0 14"/></g>"#,
                stroke("2.4")
            ),
            Self::Muted => format!(
                r#"<g {}><path d="{speaker}"/><path d="m22 9-6 6M16 9l6 6"/></g>"#,
                stroke("2.4")
            ),
            Self::Play => format!(
                r#"<path d="M7 4.5 20 12 7 19.5z" fill="{color}" stroke="{color}" stroke-width="2" stroke-linejoin="round"/>"#
            ),
            Self::Pause => format!(r#"<g {}><path d="M8 5v14M16 5v14"/></g>"#, stroke("3")),
            Self::Skip => format!(
                r#"<path d="M5 5l11 7-11 7z" fill="{color}" stroke="{color}" stroke-width="2.4" stroke-linejoin="round"/><g {}><path d="M20 5v14"/></g>"#,
                stroke("2.4")
            ),
            Self::Reset => format!(
                r#"<g {}><path d="M4 12a8 8 0 1 0 2.5-5.8"/><path d="M4 4v5h5"/></g>"#,
                stroke("2.6")
            ),
        }
    }
}

impl From<PopupIcon> for Glyph {
    fn from(icon: PopupIcon) -> Self {
        match icon {
            PopupIcon::Pause => Self::Pause,
            PopupIcon::Skip => Self::Skip,
            PopupIcon::Reset => Self::Reset,
            // `Play`, and the icons the host does not know.
            _ => Self::Play,
        }
    }
}

/// Where and how a text is drawn.
#[derive(Clone, Copy)]
enum Anchor {
    Start,
    Middle,
    End,
}

impl Anchor {
    const fn attribute(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Middle => "middle",
            Self::End => "end",
        }
    }
}

/// A text to draw.
struct Label<'a> {
    text: &'a str,
    x: f32,
    baseline: f32,
    size: f32,
    color: Rgba<u8>,
    anchor: Anchor,
    spacing: f32,
}

impl<'a> Label<'a> {
    fn new(text: &'a str, x: f32, baseline: f32, size: f32, color: Rgba<u8>) -> Self {
        Self {
            text,
            x,
            baseline,
            size,
            color,
            anchor: Anchor::Start,
            spacing: 0.0,
        }
    }

    fn anchored(self, anchor: Anchor) -> Self {
        Self { anchor, ..self }
    }

    fn spaced(self, spacing: f32) -> Self {
        Self { spacing, ..self }
    }
}

/// Builder of the SVG body of the strip.
struct Canvas {
    svg: String,
}

impl Canvas {
    fn new() -> Self {
        Self { svg: String::new() }
    }

    fn rect(&mut self, (x, y, width, height): (f32, f32, f32, f32), radius: f32, fill: Rgba<u8>) {
        push_svg!(
            self.svg,
            r#"<rect x="{x}" y="{y}" width="{width}" height="{height}" rx="{radius}" fill="{}"/>"#,
            svg_color(fill)
        );
    }

    fn circle(&mut self, (x, y): (f32, f32), radius: f32, fill: Rgba<u8>) {
        push_svg!(
            self.svg,
            r#"<circle cx="{x}" cy="{y}" r="{radius}" fill="{}"/>"#,
            svg_color(fill)
        );
    }

    /// A bar filled up to `fraction` of its width.
    fn bar(
        &mut self,
        (x, y, width, height): (f32, f32, f32, f32),
        fraction: f32,
        fill: Rgba<u8>,
        track: Rgba<u8>,
    ) {
        self.rect((x, y, width, height), height / 2.0, track);
        if fraction > 0.0 {
            let filled = (width * fraction.clamp(0.0, 1.0)).max(height);
            self.rect((x, y, filled, height), height / 2.0, fill);
        }
    }

    fn text(&mut self, label: &Label<'_>) {
        self.text_with(label, "");
    }

    /// `label`, then `suffix` in `<tspan>` markup (already escaped).
    fn text_with(&mut self, label: &Label<'_>, suffix: &str) {
        push_svg!(
            self.svg,
            r#"<text x="{}" y="{}" font-family="{}" font-size="{}" font-weight="{}" fill="{}" text-anchor="{}" letter-spacing="{}">"#,
            label.x,
            label.baseline,
            super::FONT_FAMILY,
            label.size,
            FontWeight::SemiBold.css(),
            svg_color(label.color),
            label.anchor.attribute(),
            label.spacing
        );
        escape_xml(&mut self.svg, label.text);
        self.svg.push_str(suffix);
        self.svg.push_str("</text>");
    }

    fn glyph(&mut self, glyph: Glyph, (x, y): (f32, f32), size: f32, color: Rgba<u8>) {
        let scale = size / GLYPH_BOX;
        push_svg!(
            self.svg,
            r#"<g transform="translate({x} {y}) scale({scale})">{}</g>"#,
            glyph.markup(&svg_color(color))
        );
    }

    fn vertical_rule(&mut self, x: f32, color: Rgba<u8>) {
        self.rect((x, RULE_TOP, 1.0, RULE_HEIGHT), 0.0, color);
    }
}

/// Side of the box the glyphs are drawn in.
const GLYPH_BOX: f32 = 24.0;

impl Renderer {
    /// Renders the whole touch strip of `size`. CPU-bound: call it from a
    /// blocking context.
    ///
    /// # Errors
    ///
    /// [`RenderError::InvalidSize`] when the size is empty, or
    /// [`RenderError::Svg`].
    pub fn render_strip(
        &self,
        face: &StripFace,
        size: ImageSize,
    ) -> Result<Arc<Rendered>, RenderError> {
        check_size(size)?;
        let palette = Palette::of(face.theme);
        let mut canvas = Canvas::new();
        self.draw_clock(&mut canvas, face, palette)?;
        Self::draw_measures(&mut canvas, face, palette);
        self.draw_page(&mut canvas, &face.page, palette)?;
        if let Some(volume) = face.volume {
            draw_volume(&mut canvas, volume, palette);
        }
        if let Some(pomodoro) = &face.pomodoro {
            draw_pomodoro(&mut canvas, pomodoro, palette);
        }
        self.draw_hints(&mut canvas, &face.hints, size, palette)?;
        if let Some(popup) = &face.popup {
            self.draw_popup(&mut canvas, popup, face.hints.len(), size, palette)?;
        }
        let mut image = RgbaImage::from_pixel(size.width, size.height, palette.background);
        self.text.overlay_svg(&mut image, &canvas.svg)?;
        Ok(Arc::new(Rendered::new(keep_opaque(
            image,
            palette.background,
        ))))
    }

    /// Draws the reminders of the dials over `image`, a surface covering the
    /// strip (a notification), in the format of the strip: a rule, then an
    /// icon and a text under each dial. They are drawn in shades of `ink`
    /// over `background`, which stay legible whatever the colours.
    ///
    /// # Errors
    ///
    /// [`RenderError::Svg`] when a text cannot be drawn.
    pub(super) fn draw_dial_reminders(
        &self,
        image: &mut RgbaImage,
        hints: &[HintFace],
        (ink, background): (Rgba<u8>, Rgba<u8>),
    ) -> Result<(), RenderError> {
        let shade = |alpha| {
            let mixed = |from: u8, to: u8| {
                let (from, to, alpha) = (u16::from(from), u16::from(to), u16::from(alpha));
                u8::try_from((from * (u16::from(u8::MAX) - alpha) + to * alpha) / 255)
                    .unwrap_or(u8::MAX)
            };
            let [red, green, blue, _] = background.0;
            let [ink_red, ink_green, ink_blue, _] = ink.0;
            Rgba([
                mixed(red, ink_red),
                mixed(green, ink_green),
                mixed(blue, ink_blue),
                u8::MAX,
            ])
        };
        let palette = Palette {
            muted: shade(REMINDER_TEXT_SHARE),
            line: shade(REMINDER_RULE_SHARE),
            ..*Palette::of(DeviceTheme::Light)
        };
        let (width, height) = image.dimensions();
        let size = ImageSize { width, height };
        let mut canvas = Canvas::new();
        self.draw_hints(&mut canvas, hints, size, &palette)?;
        self.text.overlay_svg(image, &canvas.svg)
    }

    fn draw_clock(
        &self,
        canvas: &mut Canvas,
        face: &StripFace,
        palette: &Palette,
    ) -> Result<(), RenderError> {
        let clock = &face.clock;
        canvas.text(
            &Label::new(
                &clock.time,
                CLOCK_LEFT,
                TIME_BASELINE,
                TIME_SIZE,
                palette.ink,
            )
            .spaced(TIME_SPACING),
        );
        if let Some(seconds) = clock.seconds {
            let width = self
                .text
                .width(&clock.time, TIME_SIZE, FontWeight::SemiBold)?;
            let x = CLOCK_LEFT + width + SECONDS_GAP + TIME_SPACING * 5.0;
            canvas.text(&Label::new(
                &format!("{seconds:02}"),
                x,
                TIME_BASELINE,
                SECONDS_SIZE,
                palette.muted,
            ));
        }
        let date = format!("{} · SEM. {}", clock.date, clock.week).to_uppercase();
        canvas.text(
            &Label::new(
                &date,
                CLOCK_LEFT + 2.0,
                DATE_BASELINE,
                DATE_SIZE,
                palette.muted,
            )
            .spaced(1.0),
        );
        canvas.vertical_rule(CLOCK_RULE_X, palette.line);
        Ok(())
    }

    fn draw_measures(canvas: &mut Canvas, face: &StripFace, palette: &Palette) {
        for (measure, row) in face.measures.iter().zip(0_u8..) {
            let baseline = MEASURE_FIRST_BASELINE + f32::from(row) * MEASURE_PITCH;
            let color = palette.measure(measure);
            canvas.text(
                &Label::new(
                    measure.label,
                    MEASURE_LABEL_X,
                    baseline,
                    MEASURE_LABEL_SIZE,
                    palette.muted,
                )
                .spaced(1.0),
            );
            if measure.history.len() > 1 {
                draw_curve(canvas, &measure.history, baseline, color);
            } else {
                let fraction = f32::from(measure.percent.unwrap_or(0)) / 100.0;
                canvas.bar(
                    (
                        MEASURE_CURVE_X,
                        baseline - MEASURE_BAR_HEIGHT - 2.0,
                        MEASURE_CURVE_WIDTH,
                        MEASURE_BAR_HEIGHT,
                    ),
                    fraction,
                    color,
                    palette.track,
                );
            }
            let value = measure
                .percent
                .map_or_else(|| "–".to_owned(), |percent| percent.to_string());
            let value_color = if measure.alert {
                palette.alert
            } else {
                palette.ink
            };
            let unit = measure.percent.map_or_else(String::new, |_| {
                format!(
                    r#"<tspan font-size="10" fill="{}"> %</tspan>"#,
                    svg_color(palette.muted)
                )
            });
            canvas.text_with(
                &Label::new(
                    &value,
                    MEASURE_VALUE_RIGHT,
                    baseline + 1.0,
                    MEASURE_VALUE_SIZE,
                    value_color,
                )
                .anchored(Anchor::End),
                &unit,
            );
        }
        canvas.vertical_rule(SYSTEM_RULE_X, palette.line);
    }

    fn draw_page(
        &self,
        canvas: &mut Canvas,
        page: &PageFace,
        palette: &Palette,
    ) -> Result<(), RenderError> {
        let dots_width = if (2..=MAX_PAGE_DOTS).contains(&page.positions) {
            f32_from(page.positions) * PAGE_DOT_PITCH
        } else {
            0.0
        };
        let room = COLUMN_RIGHT - COLUMN_LEFT - dots_width - PAGE_GAP;
        let trail = self
            .text
            .fit(&page.trail, PAGE_SIZE, FontWeight::SemiBold, room)?;
        canvas.text(&Label::new(
            &trail,
            COLUMN_LEFT,
            PAGE_BASELINE,
            PAGE_SIZE,
            palette.ink,
        ));
        if (2..=MAX_PAGE_DOTS).contains(&page.positions) {
            for row in 0..page.positions {
                let from_right = f32_from(page.positions - 1 - row);
                let x = COLUMN_RIGHT - PAGE_DOT_RADIUS - from_right * PAGE_DOT_PITCH;
                let fill = if row == page.row {
                    palette.ink
                } else {
                    palette.track
                };
                canvas.circle((x, PAGE_DOT_Y), PAGE_DOT_RADIUS, fill);
            }
        } else if page.positions > MAX_PAGE_DOTS {
            let position = format!("{}/{}", page.row + 1, page.positions);
            canvas.text(
                &Label::new(
                    &position,
                    COLUMN_RIGHT,
                    PAGE_BASELINE,
                    VOLUME_VALUE_SIZE,
                    palette.muted,
                )
                .anchored(Anchor::End),
            );
        }
        Ok(())
    }

    fn draw_hints(
        &self,
        canvas: &mut Canvas,
        hints: &[HintFace],
        size: ImageSize,
        palette: &Palette,
    ) -> Result<(), RenderError> {
        canvas.rect(
            (
                HINT_MARGIN,
                HINT_RULE_Y,
                f32_from(size.width) - 2.0 * HINT_MARGIN,
                1.0,
            ),
            0.0,
            palette.line,
        );
        let Some(count) = u32::try_from(hints.len()).ok().filter(|count| *count > 0) else {
            return Ok(());
        };
        let cell = f32_from(size.width) / f32_from(count);
        for (hint, index) in hints.iter().zip(0_u32..) {
            let left = HINT_MARGIN + f32_from(index) * cell;
            if let Some(feedback) = hint.feedback {
                draw_mark(canvas, feedback, left, palette);
                continue;
            }
            // The parts share the room of the cell: the turn part takes at most
            // half of it when both are there, the click part what is left. A
            // long text is shortened, never drawn over the next dial.
            let total = cell - 2.0 * HINT_MARGIN;
            let chrome = HINT_ICON_SIZE + HINT_ICON_GAP;
            let half = (total - HINT_PART_GAP) / 2.0 - chrome;
            let mut x = left;
            if let Some(rotate) = &hint.rotate {
                let room = if hint.press.is_some() {
                    half
                } else {
                    total - chrome
                };
                x = self.draw_hint_part(canvas, Glyph::Rotate, rotate, (x, room), palette)?;
            }
            if let Some(press) = &hint.press {
                let room = total - (x - left) - chrome;
                self.draw_hint_part(canvas, Glyph::Press, press, (x, room), palette)?;
            }
        }
        Ok(())
    }

    /// An icon followed by its text (shortened to `room`), from `x`; the
    /// position after them.
    fn draw_hint_part(
        &self,
        canvas: &mut Canvas,
        glyph: Glyph,
        text: &str,
        (x, room): (f32, f32),
        palette: &Palette,
    ) -> Result<f32, RenderError> {
        canvas.glyph(glyph, (x, HINT_ICON_Y), HINT_ICON_SIZE, palette.muted);
        let text_x = x + HINT_ICON_SIZE + HINT_ICON_GAP;
        let text = self.text.fit(text, HINT_SIZE, FontWeight::SemiBold, room)?;
        canvas.text(&Label::new(
            &text,
            text_x,
            HINT_BASELINE,
            HINT_SIZE,
            palette.muted,
        ));
        let width = self.text.width(&text, HINT_SIZE, FontWeight::SemiBold)?;
        Ok(text_x + width + HINT_PART_GAP)
    }

    fn draw_popup(
        &self,
        canvas: &mut Canvas,
        popup: &PopupFace,
        dial_count: usize,
        size: ImageSize,
        palette: &Palette,
    ) -> Result<(), RenderError> {
        let strip_width = f32_from(size.width);
        let entries = u32::try_from(popup.entries.len()).unwrap_or(0).max(1);
        let cells_width =
            f32_from(entries) * POPUP_CELL_WIDTH + f32_from(entries - 1) * POPUP_CELL_GAP;
        let card_width = cells_width + 2.0 * POPUP_PADDING;
        // The hints split the strip like the dials do: the card points at
        // the middle of the part above its dial.
        let target = popup
            .dial
            .and_then(|dial| Some((u32::try_from(dial).ok()?, u32::try_from(dial_count).ok()?)))
            .filter(|(dial, count)| dial < count)
            .map_or(strip_width / 2.0, |(dial, count)| {
                (f32_from(dial) + 0.5) * strip_width / f32_from(count)
            });
        let left =
            (target - card_width / 2.0).clamp(POPUP_EDGE, strip_width - POPUP_EDGE - card_width);
        let pointer_x = target.clamp(left + POPUP_PADDING, left + card_width - POPUP_PADDING);

        push_svg!(
            canvas.svg,
            r#"<rect width="{strip_width}" height="{POPUP_DIMMED_HEIGHT}" fill="{}" fill-opacity="{POPUP_DIM_OPACITY}"/>"#,
            svg_color(palette.background)
        );
        // A few translucent layers fake the shadow: a blur filter costs more than
        // the rest of the strip together, and the menu is redrawn at each notch.
        for layer in 1..=POPUP_SHADOW_LAYERS {
            let spread = f32_from(layer) * POPUP_SHADOW_STEP;
            push_svg!(
                canvas.svg,
                r#"<rect x="{x}" y="{y}" width="{width}" height="{height}" rx="{radius}" fill="black" fill-opacity="{POPUP_SHADOW_OPACITY}"/>"#,
                x = left - spread,
                y = POPUP_TOP - spread + POPUP_SHADOW_OFFSET,
                width = card_width + 2.0 * spread,
                height = POPUP_HEIGHT + 2.0 * spread,
                radius = POPUP_RADIUS + spread,
            );
        }
        push_svg!(
            canvas.svg,
            r#"<rect x="{left}" y="{POPUP_TOP}" width="{card_width}" height="{POPUP_HEIGHT}" rx="{POPUP_RADIUS}" fill="{card}"/><path d="M{a} {y} L{pointer_x} {tip} L{b} {y} Z" fill="{card}"/>"#,
            card = svg_color(palette.card),
            a = pointer_x - POPUP_POINTER_HALF,
            b = pointer_x + POPUP_POINTER_HALF,
            y = POPUP_TOP + POPUP_HEIGHT - 1.0,
            tip = POPUP_TOP + POPUP_HEIGHT + POPUP_POINTER_HEIGHT - 1.0,
        );
        for ((entry, index), x) in
            popup
                .entries
                .iter()
                .zip(0_usize..)
                .zip(0_u32..)
                .map(|((entry, index), step)| {
                    (
                        (entry, index),
                        left + POPUP_PADDING + f32_from(step) * (POPUP_CELL_WIDTH + POPUP_CELL_GAP),
                    )
                })
        {
            let selected = index == popup.selected;
            let (foreground, icon_color) = if selected {
                (WHITE, WHITE)
            } else {
                (palette.ink, palette.muted)
            };
            if selected {
                canvas.rect(
                    (x, POPUP_CELL_TOP, POPUP_CELL_WIDTH, POPUP_CELL_HEIGHT),
                    10.0,
                    palette.working,
                );
            }
            canvas.glyph(
                entry.icon.into(),
                (x + (POPUP_CELL_WIDTH - POPUP_ICON_SIZE) / 2.0, POPUP_ICON_Y),
                POPUP_ICON_SIZE,
                icon_color,
            );
            let label = self.text.fit(
                &entry.label,
                POPUP_LABEL_SIZE,
                FontWeight::SemiBold,
                POPUP_CELL_WIDTH - 2.0 * POPUP_LABEL_MARGIN,
            )?;
            canvas.text(
                &Label::new(
                    &label,
                    x + POPUP_CELL_WIDTH / 2.0,
                    POPUP_LABEL_BASELINE,
                    POPUP_LABEL_SIZE,
                    foreground,
                )
                .anchored(Anchor::Middle),
            );
        }
        Ok(())
    }
}

fn draw_curve(canvas: &mut Canvas, history: &[u8], baseline: f32, color: Rgba<u8>) {
    let count = u32::try_from(history.len()).unwrap_or(2).max(2);
    let step = MEASURE_CURVE_WIDTH / f32_from(count - 1);
    let points: Vec<(f32, f32)> = history
        .iter()
        .zip(0_u32..)
        .map(|(value, index)| {
            (
                MEASURE_CURVE_X + f32_from(index) * step,
                baseline - f32::from(*value) / 100.0 * MEASURE_CURVE_HEIGHT,
            )
        })
        .collect();
    let line: String = points
        .iter()
        .map(|(x, y)| format!("{x:.1},{y:.1}"))
        .collect::<Vec<_>>()
        .join(" ");
    let (end_x, end_y) = points
        .last()
        .copied()
        .unwrap_or((MEASURE_CURVE_X, baseline));
    let fill = svg_color(color);
    push_svg!(
        canvas.svg,
        r#"<polygon points="{line} {end_x:.1},{baseline} {MEASURE_CURVE_X},{baseline}" fill="{fill}" fill-opacity="0.16"/><polyline points="{line}" fill="none" stroke="{fill}" stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/><circle cx="{end_x:.1}" cy="{end_y:.1}" r="2.8" fill="{fill}"/>"#
    );
}

fn draw_volume(canvas: &mut Canvas, volume: VolumeWidget, palette: &Palette) {
    let glyph = if volume.muted {
        Glyph::Muted
    } else {
        Glyph::Volume
    };
    let color = if volume.muted {
        palette.paused
    } else {
        palette.volume
    };
    canvas.glyph(
        glyph,
        (COLUMN_LEFT, VOLUME_ICON_Y),
        VOLUME_ICON_SIZE,
        palette.muted,
    );
    let bar_x = COLUMN_LEFT + VOLUME_ICON_SIZE + 8.0;
    let fraction = f32::from(volume.percent.min(100)) / 100.0;
    canvas.bar(
        (
            bar_x,
            VOLUME_BAR_Y,
            COLUMN_RIGHT - VOLUME_VALUE_ROOM - bar_x,
            VOLUME_BAR_HEIGHT,
        ),
        if volume.muted { 0.0 } else { fraction },
        color,
        palette.track,
    );
    let value = if volume.muted {
        "muet".to_owned()
    } else {
        volume.percent.to_string()
    };
    canvas.text(
        &Label::new(
            &value,
            COLUMN_RIGHT,
            VOLUME_VALUE_BASELINE,
            VOLUME_VALUE_SIZE,
            palette.muted,
        )
        .anchored(Anchor::End),
    );
}

fn draw_pomodoro(canvas: &mut Canvas, pomodoro: &PomodoroFace, palette: &Palette) {
    let widget = &pomodoro.widget;
    let color = palette.pomodoro(widget.state);
    let ready = matches!(widget.state, PomodoroState::Ready);
    canvas.circle(
        (COLUMN_LEFT + POMODORO_DOT_RADIUS, POMODORO_DOT_Y),
        POMODORO_DOT_RADIUS,
        color,
    );
    canvas.text(&Label::new(
        &widget.label,
        COLUMN_LEFT + 16.0,
        POMODORO_LABEL_BASELINE,
        POMODORO_LABEL_SIZE,
        palette.muted,
    ));
    let time_color = if ready { palette.muted } else { palette.ink };
    canvas.text(
        &Label::new(
            &widget.time,
            COLUMN_RIGHT,
            POMODORO_TIME_BASELINE,
            POMODORO_TIME_SIZE,
            time_color,
        )
        .anchored(Anchor::End),
    );
    if pomodoro.show_progress {
        let fraction = f32::from(widget.progress())
            / f32::from(hubergamof_module_api::PomodoroWidget::FULL_PROGRESS);
        canvas.bar(
            (
                PROGRESS_X,
                PROGRESS_Y,
                800.0 - 2.0 * PROGRESS_X,
                PROGRESS_HEIGHT,
            ),
            fraction,
            color,
            palette.track,
        );
    }
}

/// The outcome mark of a dial, in place of its reminder.
fn draw_mark(canvas: &mut Canvas, feedback: Feedback, left: f32, palette: &Palette) {
    let center = (left + HINT_MARK_RADIUS, HINT_BASELINE - HINT_SIZE / 2.0);
    match feedback {
        Feedback::Ok => {
            canvas.circle(center, HINT_MARK_RADIUS, palette.ok);
            push_svg!(
                canvas.svg,
                r##"<path d="M{} {} L{} {} L{} {}" fill="none" stroke="#ffffff" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>"##,
                center.0 - 2.6,
                center.1,
                center.0 - 0.7,
                center.1 + 2.0,
                center.0 + 2.8,
                center.1 - 2.2
            );
        }
        _ => {
            push_svg!(
                canvas.svg,
                r##"<path d="M{} {} L{} {} H{} Z" fill="{}"/><rect x="{}" y="{}" width="1.4" height="3.6" fill="#ffffff"/><circle cx="{}" cy="{}" r="0.9" fill="#ffffff"/>"##,
                center.0,
                center.1 - HINT_MARK_RADIUS,
                center.0 + HINT_MARK_RADIUS + 1.0,
                center.1 + HINT_MARK_RADIUS - 0.5,
                center.0 - HINT_MARK_RADIUS - 1.0,
                svg_color(palette.alert),
                center.0 - 0.7,
                center.1 - 2.0,
                center.0,
                center.1 + 2.6
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_system_panel_lies_between_the_two_rules() {
        assert!(!is_system_panel(223));
        assert!(is_system_panel(224));
        assert!(is_system_panel(527));
        assert!(!is_system_panel(528));
    }

    use hubergamof_module_api::{PomodoroWidget, PopupEntry};

    use super::*;
    use crate::strip::ClockReading;

    const STRIP: ImageSize = ImageSize {
        width: 800,
        height: 100,
    };

    fn face() -> StripFace {
        StripFace {
            theme: DeviceTheme::Light,
            clock: ClockReading {
                time: "14:37".into(),
                seconds: Some(42),
                date: "jeu. 12 nov.".into(),
                week: 46,
            },
            measures: ["CPU", "RAM", "SWAP"]
                .into_iter()
                .zip([34, 61, 8])
                .map(|(label, percent)| MeasureFace {
                    label,
                    percent: Some(percent),
                    history: vec![20, 30, 25, 40, percent],
                    alert: false,
                })
                .collect(),
            page: PageFace {
                trail: "Docker › api-server".into(),
                row: 1,
                positions: 3,
            },
            volume: Some(VolumeWidget::new(62, false)),
            pomodoro: Some(PomodoroFace {
                widget: PomodoroWidget::new(PomodoroState::Working, "Focus 2/4", "18:24", 260),
                show_progress: true,
            }),
            hints: ["Volume", "Navigation"]
                .into_iter()
                .map(|rotate| HintFace {
                    rotate: Some(rotate.into()),
                    press: Some("Muet".into()),
                    feedback: None,
                })
                .chain([HintFace::default(), HintFace::default()])
                .collect(),
            popup: None,
        }
    }

    fn render(face: &StripFace) -> Arc<Rendered> {
        Renderer::new().unwrap().render_strip(face, STRIP).unwrap()
    }

    /// Pixels differing from `background` in the box `x`, `y` of `width` × `height`.
    fn drawn_in(
        image: &RgbaImage,
        background: Rgba<u8>,
        (x, y, width, height): (u32, u32, u32, u32),
    ) -> usize {
        (x..x + width)
            .flat_map(|x| (y..y + height).map(move |y| (x, y)))
            .filter(|(x, y)| *image.get_pixel(*x, *y) != background)
            .count()
    }

    #[test]
    fn the_strip_is_one_opaque_image_of_the_strip_size() {
        let rendered = render(&face());
        let image = rendered.image();
        assert_eq!(image.dimensions(), (800, 100));
        assert!(image.pixels().all(|pixel| pixel.0[3] == u8::MAX));
        assert_eq!(*image.get_pixel(0, 0), LIGHT.background);
    }

    #[test]
    fn the_themes_have_their_own_background() {
        let mut dark = face();
        dark.theme = DeviceTheme::Dark;
        assert_eq!(*render(&dark).image().get_pixel(0, 0), DARK.background);
        assert_ne!(DARK.background, LIGHT.background);
    }

    #[test]
    fn every_group_of_the_strip_is_drawn() {
        let image = render(&face());
        let image = image.image();
        let background = LIGHT.background;
        for (name, area) in [
            ("clock", (10, 10, 210, 70)),
            ("measures", (230, 10, 290, 66)),
            ("page", (540, 10, 250, 16)),
            ("volume", (540, 28, 250, 16)),
            ("pomodoro", (540, 46, 250, 30)),
            ("progress", (16, 75, 768, 4)),
            ("hints", (10, 84, 780, 14)),
        ] {
            assert!(drawn_in(image, background, area) > 0, "{name} is blank");
        }
    }

    #[test]
    fn groups_left_out_leave_their_place_blank() {
        let mut bare = face();
        bare.volume = None;
        bare.pomodoro = None;
        bare.hints.clear();
        let image = render(&bare);
        let image = image.image();
        let background = LIGHT.background;
        assert_eq!(drawn_in(image, background, (540, 28, 250, 16)), 0);
        assert_eq!(drawn_in(image, background, (540, 46, 250, 30)), 0);
        // The rule above the (missing) hints is still there.
        assert!(drawn_in(image, background, (10, 83, 780, 2)) > 0);
    }

    #[test]
    fn the_progress_bar_is_optional_and_follows_the_pomodoro() {
        let mut without = face();
        without.pomodoro.as_mut().unwrap().show_progress = false;
        let background = LIGHT.background;
        let bar = (16, 75, 768, 4);
        assert_eq!(drawn_in(render(&without).image(), background, bar), 0);

        let started = |progress| {
            let mut face = face();
            let widget = PomodoroWidget::new(PomodoroState::Working, "Focus", "10:00", progress);
            face.pomodoro.as_mut().unwrap().widget = widget;
            let image = render(&face);
            // Columns of the bar in the pomodoro colour.
            (16..784)
                .filter(|x| *image.image().get_pixel(*x, 77) == LIGHT.working)
                .count()
        };
        assert_eq!(started(0), 0);
        assert!(started(500) > 300 && started(500) < 450);
        assert!(started(1000) > 750);
    }

    #[test]
    fn seconds_and_curves_can_be_left_out() {
        let background = LIGHT.background;
        let seconds_area = (170, 20, 50, 40);
        let with = render(&face());
        assert!(drawn_in(with.image(), background, seconds_area) > 0);
        let mut plain = face();
        plain.clock.seconds = None;
        for measure in &mut plain.measures {
            measure.history.clear();
        }
        let without = render(&plain);
        assert_eq!(drawn_in(without.image(), background, seconds_area), 0);
        assert_ne!(with.image().as_ref(), without.image().as_ref());
    }

    #[test]
    fn a_load_over_the_threshold_is_drawn_in_the_alert_colour() {
        let mut alert = face();
        alert.measures[0].alert = true;
        let image = render(&alert);
        let row = |image: &Rendered, color| {
            (282..452)
                .flat_map(|x| (10..30).map(move |y| (x, y)))
                .filter(|(x, y)| *image.image().get_pixel(*x, *y) == color)
                .count()
        };
        assert!(row(&image, LIGHT.alert) > 0);
        assert_eq!(row(&render(&face()), LIGHT.alert), 0);
    }

    #[test]
    fn the_page_shows_dots_for_a_few_positions_and_a_position_for_many() {
        let background = LIGHT.background;
        let dots = (700, 12, 90, 10);
        let drawn = |positions| {
            let mut page = face();
            page.page.positions = positions;
            drawn_in(render(&page).image(), background, dots)
        };
        assert_eq!(
            drawn(1),
            drawn_in(render(&face_without_page_dots()).image(), background, dots)
        );
        assert!(drawn(3) > drawn(1));
        // Beyond the dots, the position is written instead.
        assert!(drawn(20) > 0);
        assert_ne!(drawn(20), drawn(3));
    }

    fn face_without_page_dots() -> StripFace {
        let mut page = face();
        page.page.positions = 1;
        page
    }

    #[test]
    fn a_long_reminder_stays_in_the_cell_of_its_dial() {
        let mut long = face();
        long.hints[0].rotate = Some("Un rappel vraiment beaucoup trop long".into());
        long.hints[0].press = Some("et un clic encore bien plus long que cela".into());
        let image = render(&long);
        // Nothing of the first dial's reminder reaches the second one (216 px).
        assert_eq!(
            drawn_in(image.image(), LIGHT.background, (212, 86, 4, 12)),
            0
        );
    }

    #[test]
    fn a_lone_reminder_may_take_the_whole_cell() {
        let mut lone = face();
        lone.hints[1].rotate = None;
        lone.hints[1].press = Some("Menu pomodoro".into());
        let mut short = lone.clone();
        short.hints[1].press = Some("Menu".into());
        // Past the half of the cell where a reminder shared with another is cut.
        let beyond_half = (200 + 16 + 14 + 70, 88, 12, 8);
        let ink = |face: &StripFace| drawn_in(render(face).image(), LIGHT.background, beyond_half);
        assert!(ink(&lone) > 0, "the whole text is drawn");
        assert_eq!(ink(&short), 0);
    }

    #[test]
    fn a_path_with_control_characters_still_draws_the_strip() {
        let mut polluted = face();
        polluted.page.trail = "Do\u{1}cker\u{0}".into();
        polluted.hints[0].rotate = Some("\u{1b}Vol".into());
        let image = render(&polluted);
        assert!(drawn_in(image.image(), LIGHT.background, (540, 10, 250, 16)) > 0);
    }

    #[test]
    fn a_long_path_is_shortened_to_its_room() {
        let mut long = face();
        long.page.trail = "Un projet au nom vraiment très très long › et sa page".into();
        let image = render(&long);
        // Nothing is drawn beyond the right edge of the column.
        let background = LIGHT.background;
        assert_eq!(drawn_in(image.image(), background, (785, 10, 15, 16)), 0);
    }

    #[test]
    fn a_feedback_mark_replaces_the_reminder_of_its_dial() {
        let background = LIGHT.background;
        let cell = (16, 84, 180, 14);
        let plain = render(&face());
        for feedback in [Feedback::Ok, Feedback::Alert] {
            let mut marked = face();
            marked.hints[0].feedback = Some(feedback);
            let image = render(&marked);
            assert_ne!(plain.image().as_ref(), image.image().as_ref());
            assert!(drawn_in(image.image(), background, (16, 84, 12, 14)) > 0);
            // The reminder text is gone: only the mark remains.
            assert_eq!(drawn_in(image.image(), background, (40, 86, 150, 12)), 0);
        }
        assert!(drawn_in(plain.image(), background, cell) > 0);
    }

    fn popup_face(dial: Option<usize>, selected: usize) -> StripFace {
        let mut face = face();
        face.popup = Some(PopupFace {
            entries: vec![
                PopupEntry::new(PopupIcon::Pause, "Pause"),
                PopupEntry::new(PopupIcon::Skip, "Passer à la pause"),
                PopupEntry::new(PopupIcon::Reset, "Remise à zéro"),
            ],
            selected,
            dial,
        });
        face
    }

    #[test]
    fn the_popup_dims_the_strip_and_highlights_the_selected_entry() {
        let plain = render(&face());
        let popup = render(&popup_face(Some(1), 0));
        assert_ne!(plain.image().as_ref(), popup.image().as_ref());
        // The first entry is selected: its cell is in the accent colour.
        let accent = |image: &Rendered, x: u32| *image.image().get_pixel(x, 20) == LIGHT.working;
        let cell_width = 130;
        let first = 400 - 211 + 10 + 4;
        assert!(accent(&popup, first));
        assert!(!accent(&popup, first + cell_width + 6));
        let second = render(&popup_face(Some(1), 1));
        assert!(!accent(&second, first));
        assert!(accent(&second, first + cell_width + 6));
    }

    #[test]
    fn the_popup_points_at_its_dial_and_stays_on_the_strip() {
        let at = |dial: Option<usize>| render(&popup_face(dial, 0));
        // The pointer, under the card, is above the middle of the dial.
        let tip = |image: &Rendered| {
            (0..800)
                .filter(|x| *image.image().get_pixel(*x, 82) == LIGHT.card)
                .map(i64::from)
                .sum::<i64>()
        };
        let pointer = |image: &Rendered| {
            let count = (0..800)
                .filter(|x| *image.image().get_pixel(*x, 82) == LIGHT.card)
                .count();
            tip(image) / i64::try_from(count.max(1)).unwrap()
        };
        assert!(
            (pointer(&at(Some(1))) - 300).abs() < 6,
            "{}",
            pointer(&at(Some(1)))
        );
        // At the far left, the card is kept within the strip.
        let first = at(Some(0));
        assert_eq!(*first.image().get_pixel(9, 40), LIGHT.card);
        assert!(at(None).image().pixels().any(|pixel| *pixel == LIGHT.card));
        // A dial the strip does not have: centred, not a panic.
        assert!(
            at(Some(9))
                .image()
                .pixels()
                .any(|pixel| *pixel == LIGHT.card)
        );
    }

    #[test]
    fn an_empty_strip_size_is_refused() {
        let renderer = Renderer::new().unwrap();
        let empty = ImageSize {
            width: 0,
            height: 100,
        };
        assert!(matches!(
            renderer.render_strip(&face(), empty),
            Err(RenderError::InvalidSize)
        ));
    }
}
