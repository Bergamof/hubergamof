//! Full-strip surfaces: notifications (module icon, message that scrolls
//! when too long, remaining-time bar) and the label of a held key.

use std::sync::{Arc, PoisonError};
use std::time::Duration;

use hubergamof_protocol::ImageSize;
use image::{Rgba, RgbaImage, imageops};

use super::source::f32_from;
use super::{
    FontWeight, ImageSource, KeyRequest, LineStyle, RenderError, Renderer, TEXT_COLOR,
    TextRenderer, check_size, keep_opaque,
};

/// Icon side, relative to the strip height.
const ICON_RATIO: f32 = 0.72;
/// Space between the strip edge and the icon, relative to the strip height.
const EDGE_RATIO: f32 = 0.14;
/// Space between the icon and the message, relative to the strip height.
const ICON_GAP_RATIO: f32 = 0.20;
/// Message font size, relative to the strip height.
const MESSAGE_FONT_RATIO: f32 = 0.40;
/// Label font size, relative to the strip height.
const LABEL_FONT_RATIO: f32 = 0.34;
/// Height of the remaining-time bar, relative to the strip height.
const BAR_RATIO: f32 = 0.06;
/// Opacity of the remaining-time bar.
const BAR_ALPHA: u8 = 190;
/// Width of the fade at both ends of a scrolling message, relative to the
/// strip height.
const FADE_RATIO: f32 = 0.30;
/// Space between the end of a scrolling message and its next occurrence,
/// relative to the strip height.
const LOOP_GAP_RATIO: f32 = 1.2;
/// Scrolling speed, in strip heights per second.
const SCROLL_SPEED_RATIO: f32 = 1.2;
/// Time a long message stays still before it scrolls.
pub const SCROLL_PAUSE: Duration = Duration::from_millis(1500);

/// Widest text layer, in pixels: a message is cut well before
/// (`MAX_NOTIFICATION_CHARS`), this only bounds the allocation.
const MAX_LAYER_WIDTH: u32 = 16_384;

/// `value` rounded down to whole pixels, within `0..=max`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn pixels(value: f32, max: u32) -> u32 {
    // Clamped to the range of the cast.
    value.clamp(0.0, f32_from(max)) as u32
}

/// Text on a background lighter than this (perceived luminance, 0 to 255)
/// is drawn in black.
const LIGHT_BACKGROUND_LUMINANCE: f32 = 150.0;
const DARK_TEXT: Rgba<u8> = Rgba([0x11, 0x11, 0x11, u8::MAX]);

/// What a notification looks like, apart from time.
#[derive(Debug, Clone, Copy)]
pub struct NotificationLook<'a> {
    /// Size of the whole strip.
    pub size: ImageSize,
    /// Background colour.
    pub background: [u8; 3],
    /// Icon of the sending module.
    pub icon: Option<&'a ImageSource>,
    /// Message.
    pub message: &'a str,
}

/// A drawn notification, and whether its message is too wide to stand still
/// (it then scrolls, and the host keeps drawing it).
#[derive(Debug)]
pub struct NotificationImage {
    /// The whole strip.
    pub image: Arc<RgbaImage>,
    /// Whether the message scrolls.
    pub scrolls: bool,
}

/// The message drawn on a transparent layer: twice when it scrolls, one
/// loop apart, so that cropping at any offset wraps around.
pub(super) struct TextLayer {
    key: (String, u32, u32, [u8; 4]),
    image: Arc<RgbaImage>,
    /// Width of the message, measured when the layer was drawn.
    text_width: f32,
}

/// Geometry of a notification on a strip of a given size.
#[derive(Debug, Clone, Copy)]
struct Metrics {
    height: u32,
    font: f32,
    icon: u32,
    edge: u32,
    text_left: u32,
    visible: u32,
}

impl Metrics {
    fn of(size: ImageSize) -> Self {
        let ratio = |ratio| pixels(f32_from(size.height) * ratio, size.width);
        let icon = ratio(ICON_RATIO);
        let edge = ratio(EDGE_RATIO);
        let text_left = edge + icon + ratio(ICON_GAP_RATIO);
        Self {
            height: size.height,
            font: f32_from(size.height) * MESSAGE_FONT_RATIO,
            icon,
            edge,
            text_left,
            visible: size.width.saturating_sub(text_left + edge),
        }
    }

    fn loop_length(&self, text_width: f32) -> f32 {
        text_width + f32_from(self.height) * LOOP_GAP_RATIO
    }
}

/// Text colour readable on `background`: black on light colours, white
/// otherwise.
#[must_use]
pub fn readable_text_color(background: [u8; 3]) -> Rgba<u8> {
    let [red, green, blue] = background.map(f32::from);
    let luminance = 0.299 * red + 0.587 * green + 0.114 * blue;
    if luminance > LIGHT_BACKGROUND_LUMINANCE {
        DARK_TEXT
    } else {
        TEXT_COLOR
    }
}

/// Offset, in pixels, of a message scrolling in a loop of `loop_length`
/// pixels after `elapsed`: still for [`SCROLL_PAUSE`], then constant speed.
#[must_use]
pub fn scroll_offset(elapsed: Duration, loop_length: f32, strip_height: f32) -> f32 {
    let moving = elapsed.saturating_sub(SCROLL_PAUSE);
    if loop_length <= 0.0 {
        return 0.0;
    }
    (moving.as_secs_f32() * strip_height * SCROLL_SPEED_RATIO) % loop_length
}

impl Renderer {
    /// Renders a notification `elapsed` after it appeared; `timeout` draws
    /// the bar of the time left. Not cached (it changes at every step):
    /// CPU-bound, call it from a blocking context.
    ///
    /// # Errors
    ///
    /// Any [`RenderError`] from the icon or the text.
    pub fn render_notification(
        &self,
        look: &NotificationLook<'_>,
        elapsed: Duration,
        timeout: Option<Duration>,
    ) -> Result<NotificationImage, RenderError> {
        check_size(look.size)?;
        let metrics = Metrics::of(look.size);
        let [red, green, blue] = look.background;
        let background = Rgba([red, green, blue, u8::MAX]);
        let mut canvas = RgbaImage::from_pixel(look.size.width, look.size.height, background);
        if let Some(icon) = look.icon {
            let side = ImageSize {
                width: metrics.icon,
                height: metrics.icon,
            };
            let rendered = self.render_key(&KeyRequest::new(side, Some(icon), None))?;
            imageops::overlay(
                &mut canvas,
                rendered.image().as_ref(),
                i64::from(metrics.edge),
                i64::from(metrics.edge),
            );
        }
        let scrolls = self.draw_message(&mut canvas, look, &metrics, elapsed)?;
        if let Some(timeout) = timeout {
            draw_time_bar(&mut canvas, look.background, elapsed, timeout);
        }
        Ok(NotificationImage {
            image: Arc::new(keep_opaque(canvas, background)),
            scrolls,
        })
    }

    /// Renders the label of a held key over the whole strip: white on
    /// black, centred.
    ///
    /// # Errors
    ///
    /// [`RenderError::InvalidSize`] or [`RenderError::Svg`].
    pub fn render_strip_label(
        &self,
        label: &str,
        size: ImageSize,
    ) -> Result<Arc<RgbaImage>, RenderError> {
        check_size(size)?;
        let height = f32_from(size.height);
        let font = height * LABEL_FONT_RATIO;
        let max_width = f32_from(size.width) * 0.9;
        let style = LineStyle {
            size: font,
            weight: FontWeight::SemiBold,
            color: TEXT_COLOR,
        };
        let baseline =
            (height - TextRenderer::line_height(font)) / 2.0 + TextRenderer::ascent(font);
        let line = self.text.fitted_line(
            label.trim(),
            style,
            f32_from(size.width) / 2.0,
            baseline,
            max_width,
        )?;
        let mut canvas = RgbaImage::from_pixel(size.width, size.height, Rgba([0, 0, 0, u8::MAX]));
        self.text.draw(&mut canvas, &[line])?;
        Ok(Arc::new(canvas))
    }

    fn draw_message(
        &self,
        canvas: &mut RgbaImage,
        look: &NotificationLook<'_>,
        metrics: &Metrics,
        elapsed: Duration,
    ) -> Result<bool, RenderError> {
        let color = readable_text_color(look.background);
        let (layer, text_width) = self.text_layer(look, metrics, color)?;
        let scrolls = text_width > f32_from(metrics.visible);
        let offset = if scrolls {
            pixels(
                scroll_offset(
                    elapsed,
                    metrics.loop_length(text_width),
                    f32_from(metrics.height),
                ),
                u32::MAX,
            )
        } else {
            0
        };
        let mut window =
            imageops::crop_imm(layer.as_ref(), offset, 0, metrics.visible, metrics.height)
                .to_image();
        if scrolls {
            fade_ends(
                &mut window,
                pixels(f32_from(metrics.height) * FADE_RATIO, metrics.visible),
            );
        }
        imageops::overlay(canvas, &window, i64::from(metrics.text_left), 0);
        Ok(scrolls)
    }

    /// The text layer of `look` and the width of its message, drawn again
    /// (and measured) only when the message, the size or the colour changed.
    fn text_layer(
        &self,
        look: &NotificationLook<'_>,
        metrics: &Metrics,
        color: Rgba<u8>,
    ) -> Result<(Arc<RgbaImage>, f32), RenderError> {
        let mut cached = self
            .notification_text
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let same = |layer: &&TextLayer| {
            layer.key.0 == look.message
                && layer.key.1 == look.size.width
                && layer.key.2 == look.size.height
                && layer.key.3 == color.0
        };
        if let Some(layer) = cached.as_ref().filter(same) {
            return Ok((Arc::clone(&layer.image), layer.text_width));
        }
        let text_width = self
            .text
            .width(look.message, metrics.font, FontWeight::SemiBold)?;
        let scrolls = text_width > f32_from(metrics.visible);
        let loop_length = metrics.loop_length(text_width);
        let width = if scrolls {
            pixels(loop_length.ceil(), MAX_LAYER_WIDTH) + metrics.visible
        } else {
            metrics.visible
        };
        let mut layer = RgbaImage::new(width.max(1), metrics.height);
        let style = LineStyle {
            size: metrics.font,
            weight: FontWeight::SemiBold,
            color,
        };
        let baseline = (f32_from(metrics.height) - TextRenderer::line_height(metrics.font)) / 2.0
            + TextRenderer::ascent(metrics.font);
        let occurrences: u8 = if scrolls { 2 } else { 1 };
        let lines: Vec<_> = (0..occurrences)
            .map(|index| {
                let left = loop_length * f32::from(index);
                TextRenderer::line(look.message, style, left + text_width / 2.0, baseline)
            })
            .collect();
        self.text.draw(&mut layer, &lines)?;
        let image = Arc::new(layer);
        *cached = Some(TextLayer {
            key: (
                look.message.to_owned(),
                look.size.width,
                look.size.height,
                color.0,
            ),
            image: Arc::clone(&image),
            text_width,
        });
        Ok((image, text_width))
    }
}

/// Fades the first and last `width` columns of `window` to transparent.
fn fade_ends(window: &mut RgbaImage, width: u32) {
    let columns = window.width();
    if width == 0 || columns < 2 * width {
        return;
    }
    for (x, _, pixel) in window.enumerate_pixels_mut() {
        let distance = x.min(columns - 1 - x);
        if distance < width {
            let alpha = u32::from(pixel.0[3]) * distance / width;
            pixel.0[3] = u8::try_from(alpha).unwrap_or(u8::MAX);
        }
    }
}

/// `base` moved towards `over` by `alpha` out of 255.
fn blend(base: u8, over: u8, alpha: u8) -> u8 {
    let alpha = u16::from(alpha);
    let mixed = (u16::from(base) * (255 - alpha) + u16::from(over) * alpha) / 255;
    u8::try_from(mixed).unwrap_or(u8::MAX)
}

/// Draws the bar of the time left at the bottom of `canvas`, shrinking
/// towards the left.
fn draw_time_bar(
    canvas: &mut RgbaImage,
    background: [u8; 3],
    elapsed: Duration,
    timeout: Duration,
) {
    let (width, height) = canvas.dimensions();
    let left = 1.0 - (elapsed.as_secs_f32() / timeout.as_secs_f32()).clamp(0.0, 1.0);
    let bar_width = pixels((f32_from(width) * left).round(), width);
    let bar_height = pixels(f32_from(height) * BAR_RATIO, height).max(1);
    let color = readable_text_color(background).0;
    for y in height - bar_height..height {
        for x in 0..bar_width {
            let pixel = canvas.get_pixel_mut(x, y);
            for (channel, over) in pixel.0.iter_mut().zip(color).take(3) {
                *channel = blend(*channel, over, BAR_ALPHA);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRIP: ImageSize = ImageSize {
        width: 800,
        height: 100,
    };

    fn look(message: &str) -> NotificationLook<'_> {
        NotificationLook {
            size: STRIP,
            background: [0xF0, 0x3E, 0x2F],
            icon: None,
            message,
        }
    }

    #[test]
    fn text_is_black_on_light_backgrounds_and_white_on_dark_ones() {
        assert_eq!(readable_text_color([0xFD, 0xE6, 0x8A]), DARK_TEXT);
        assert_eq!(readable_text_color([0xF0, 0x3E, 0x2F]), TEXT_COLOR);
        assert_eq!(readable_text_color([0, 0, 0]), TEXT_COLOR);
    }

    #[test]
    fn a_message_waits_then_scrolls_in_a_loop() {
        assert!(scroll_offset(Duration::ZERO, 900.0, 100.0).abs() < f32::EPSILON);
        assert!(scroll_offset(SCROLL_PAUSE, 900.0, 100.0).abs() < f32::EPSILON);
        let moved = scroll_offset(SCROLL_PAUSE + Duration::from_secs(1), 900.0, 100.0);
        assert!((moved - 120.0).abs() < 0.01);
        let wrapped = scroll_offset(SCROLL_PAUSE + Duration::from_secs(10), 900.0, 100.0);
        assert!(wrapped < 900.0);
    }

    #[test]
    fn only_long_messages_scroll() {
        let renderer = Renderer::new().unwrap();
        let drawn = |message: &str| {
            renderer
                .render_notification(&look(message), Duration::ZERO, None)
                .unwrap()
                .scrolls
        };
        assert!(!drawn("Pause terminée"));
        let long = "Un message vraiment très long qui ne tient pas sur la bande tactile ".repeat(3);
        assert!(drawn(&long));
    }

    #[test]
    fn a_notification_fills_the_strip_with_its_background() {
        let renderer = Renderer::new().unwrap();
        let image = renderer
            .render_notification(&look("Pause"), Duration::ZERO, None)
            .unwrap()
            .image;
        assert_eq!(image.dimensions(), (800, 100));
        assert_eq!(*image.get_pixel(799, 0), Rgba([0xF0, 0x3E, 0x2F, 255]));
        assert!(image.pixels().any(|pixel| *pixel == TEXT_COLOR));
    }

    #[test]
    fn the_time_bar_shrinks_with_the_time_left() {
        let renderer = Renderer::new().unwrap();
        let timeout = Duration::from_secs(10);
        let background = Rgba([0xF0, 0x3E, 0x2F, 255]);
        let full = renderer
            .render_notification(&look("x"), Duration::ZERO, Some(timeout))
            .unwrap()
            .image;
        let half = renderer
            .render_notification(&look("x"), timeout / 2, Some(timeout))
            .unwrap()
            .image;
        assert_ne!(*full.get_pixel(700, 99), background);
        assert_eq!(*half.get_pixel(700, 99), background);
        assert_ne!(*half.get_pixel(100, 99), background);
    }

    #[test]
    fn a_long_message_moves_once_the_pause_is_over() {
        let renderer = Renderer::new().unwrap();
        let long = "Un message vraiment très long qui ne tient pas sur la bande tactile ".repeat(3);
        let still = renderer
            .render_notification(&look(&long), Duration::ZERO, None)
            .unwrap()
            .image;
        let moved = renderer
            .render_notification(&look(&long), SCROLL_PAUSE + Duration::from_secs(1), None)
            .unwrap()
            .image;
        assert_ne!(still.as_raw(), moved.as_raw());
    }

    #[test]
    fn a_held_label_is_white_on_black() {
        let renderer = Renderer::new().unwrap();
        let image = renderer.render_strip_label("Docker", STRIP).unwrap();
        assert_eq!(image.dimensions(), (800, 100));
        assert_eq!(*image.get_pixel(0, 0), Rgba([0, 0, 0, 255]));
        assert!(image.pixels().any(|pixel| pixel.0[0] > 200));
    }
}
