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
/// Action reminder font size, relative to the strip height.
const ACTION_FONT_RATIO: f32 = 0.15;
/// Distance between the reminder baseline and the strip bottom, relative to
/// the strip height (above the time bar).
const ACTION_BASELINE_RATIO: f32 = 0.10;
/// Icon side when a reminder is drawn, relative to the strip height.
const ACTION_ICON_RATIO: f32 = 0.56;
/// Space above the icon when a reminder is drawn, relative to the strip
/// height.
const ACTION_ICON_TOP_RATIO: f32 = 0.07;
/// Share of the dial column the reminder may fill.
const ACTION_WIDTH_SHARE: f32 = 0.92;
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

/// Reminder of what a dial does, drawn at the bottom of the
/// notification above the dial.
#[derive(Debug, Clone, Copy)]
pub struct NotificationActionLook<'a> {
    /// Text of the reminder.
    pub text: &'a str,
    /// Index of the dial among `dials`.
    pub dial: usize,
    /// Number of dials, which split the strip into as many columns.
    pub dials: usize,
}

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
    /// What the dials do, drawn at the bottom above them.
    pub actions: &'a [NotificationActionLook<'a>],
}

impl NotificationLook<'_> {
    /// Whether the reminder of the contextual dial is drawn: it needs a dial
    /// to sit above.
    fn shows_action(&self) -> bool {
        self.actions.iter().any(|action| action.dials > 0)
    }
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

    /// Side, left and top of the icon. With an action reminder at the bottom
    /// the icon is smaller and sits higher, centred in the room it keeps, so
    /// that it does not touch the reminder; the message follows it
    /// ([`Self::message_rise`]).
    fn icon_placement(&self, with_action: bool) -> (u32, u32, u32) {
        if !with_action {
            return (self.icon, self.edge, self.edge);
        }
        let height = f32_from(self.height);
        let side = pixels(height * ACTION_ICON_RATIO, self.icon);
        let left = self.edge + (self.icon - side) / 2;
        (
            side,
            left,
            pixels(height * ACTION_ICON_TOP_RATIO, self.height),
        )
    }

    /// How far the message moves up, in pixels: with a reminder at the
    /// bottom, its line is centred on the raised icon.
    fn message_rise(&self, with_action: bool) -> u32 {
        if !with_action {
            return 0;
        }
        let (side, _, top) = self.icon_placement(true);
        (self.height / 2).saturating_sub(top + side / 2)
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
            let (side, left, top) = metrics.icon_placement(look.shows_action());
            let side_size = ImageSize {
                width: side,
                height: side,
            };
            let rendered =
                self.render_key(&KeyRequest::new(side_size, Some(icon), None).transparent())?;
            imageops::overlay(
                &mut canvas,
                rendered.image().as_ref(),
                i64::from(left),
                i64::from(top),
            );
        }
        let scrolls = self.draw_message(&mut canvas, look, &metrics, elapsed)?;
        for action in look.actions {
            self.draw_action(&mut canvas, look, action)?;
        }
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

    /// The reminder of the contextual dial, centred above its column.
    fn draw_action(
        &self,
        canvas: &mut RgbaImage,
        look: &NotificationLook<'_>,
        action: &NotificationActionLook<'_>,
    ) -> Result<(), RenderError> {
        let Some(dials) = u16::try_from(action.dials).ok().filter(|dials| *dials > 0) else {
            return Ok(());
        };
        let height = f32_from(look.size.height);
        let column = f32_from(look.size.width) / f32::from(dials);
        let centre = (f32::from(u16::try_from(action.dial).unwrap_or(0)) + 0.5) * column;
        let style = LineStyle {
            size: height * ACTION_FONT_RATIO,
            weight: FontWeight::SemiBold,
            color: readable_text_color(look.background),
        };
        let baseline = height * (1.0 - ACTION_BASELINE_RATIO);
        let line = self.text.fitted_line(
            action.text.trim(),
            style,
            centre,
            baseline,
            column * ACTION_WIDTH_SHARE,
        )?;
        self.text.draw(canvas, &[line])
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
            let fade = pixels(f32_from(metrics.height) * FADE_RATIO, metrics.visible);
            // The start of the message is not faded while it still stands
            // still: the fade on the left grows as the text moves away.
            fade_ends(&mut window, fade.min(offset), fade);
        }
        let rise = metrics.message_rise(look.shows_action());
        imageops::overlay(
            canvas,
            &window,
            i64::from(metrics.text_left),
            -i64::from(rise),
        );
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

/// Fades the first `left` and the last `right` columns of `window` to
/// transparent.
fn fade_ends(window: &mut RgbaImage, left: u32, right: u32) {
    let columns = window.width();
    if columns < left + right {
        return;
    }
    for (x, _, pixel) in window.enumerate_pixels_mut() {
        let from_right = columns - 1 - x;
        let (distance, width) = if x < left {
            (x, left)
        } else if from_right < right {
            (from_right, right)
        } else {
            continue;
        };
        let alpha = u32::from(pixel.0[3]) * distance / width;
        pixel.0[3] = u8::try_from(alpha).unwrap_or(u8::MAX);
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
            actions: &[],
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
    fn the_start_of_a_scrolling_message_is_not_faded_until_it_moves() {
        let opaque = |width, height| RgbaImage::from_pixel(width, height, Rgba([9, 9, 9, 255]));
        let mut still = opaque(100, 1);
        fade_ends(&mut still, 0, 30);
        assert_eq!(still.get_pixel(0, 0).0[3], 255);
        assert!(still.get_pixel(99, 0).0[3] < 10);
        let mut moving = opaque(100, 1);
        fade_ends(&mut moving, 30, 30);
        assert_eq!(moving.get_pixel(0, 0).0[3], 0);
        assert_eq!(moving.get_pixel(50, 0).0[3], 255);
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

    /// Pixels of `image` that differ from `other`, as `(min_x, max_x)`
    /// within the rows from `from_y`.
    fn changed_columns(image: &RgbaImage, other: &RgbaImage, from_y: u32) -> Option<(u32, u32)> {
        let changed: Vec<u32> = image
            .enumerate_pixels()
            .filter(|(x, y, pixel)| *y >= from_y && other.get_pixel(*x, *y) != *pixel)
            .map(|(x, _, _)| x)
            .collect();
        Some((*changed.iter().min()?, *changed.iter().max()?))
    }

    #[test]
    fn the_action_is_drawn_at_the_bottom_above_its_dial() {
        let renderer = Renderer::new().unwrap();
        let bare = renderer
            .render_notification(&look("Pause !"), Duration::ZERO, None)
            .unwrap();
        let with_action = NotificationLook {
            actions: &[NotificationActionLook {
                text: "Démarrer la pause",
                dial: 2,
                dials: 4,
            }],
            ..look("Pause !")
        };
        let drawn = renderer
            .render_notification(&with_action, Duration::ZERO, None)
            .unwrap();
        // Below the message, only the third column changes.
        let (left, right) = changed_columns(&drawn.image, &bare.image, 75).unwrap();
        assert!(left >= 400 && right < 600, "{left}..{right}");
        assert!(!drawn.scrolls);
    }

    #[test]
    fn an_action_on_a_device_without_dials_is_not_drawn() {
        let renderer = Renderer::new().unwrap();
        let bare = renderer
            .render_notification(&look("Pause !"), Duration::ZERO, None)
            .unwrap();
        let with_action = NotificationLook {
            actions: &[NotificationActionLook {
                text: "Démarrer",
                dial: 0,
                dials: 0,
            }],
            ..look("Pause !")
        };
        let drawn = renderer
            .render_notification(&with_action, Duration::ZERO, None)
            .unwrap();
        assert_eq!(drawn.image, bare.image);
    }

    #[test]
    fn the_icon_rises_and_shrinks_when_an_action_is_shown() {
        let metrics = Metrics::of(STRIP);
        let (side, left, top) = metrics.icon_placement(true);
        let (full, full_left, full_top) = metrics.icon_placement(false);
        assert!(side < full && top < full_top && left >= full_left);
        // Still centred on the same column and clear of the reminder line.
        assert_eq!(left * 2 + side, full_left * 2 + full);
        assert!(top + side < 76, "{}", top + side);
    }

    #[test]
    fn the_message_is_centred_on_the_raised_icon() {
        let metrics = Metrics::of(STRIP);
        assert_eq!(metrics.message_rise(false), 0);
        let (side, _, top) = metrics.icon_placement(true);
        assert_eq!(
            metrics.height / 2 - metrics.message_rise(true),
            top + side / 2
        );
    }

    #[test]
    fn the_message_rises_with_the_action_line() {
        let renderer = Renderer::new().unwrap();
        let ink_rows = |look: &NotificationLook<'_>| {
            let image = renderer
                .render_notification(look, Duration::ZERO, None)
                .unwrap()
                .image;
            let rows: Vec<u32> = image
                .enumerate_pixels()
                .filter(|(x, y, pixel)| {
                    *x >= 200 && *x < 400 && *y < 75 && pixel.0[..3] != look.background
                })
                .map(|(_, y, _)| y)
                .collect();
            (*rows.iter().min().unwrap(), *rows.iter().max().unwrap())
        };
        let bare = ink_rows(&look("Pause"));
        let raised = ink_rows(&NotificationLook {
            actions: &[NotificationActionLook {
                text: "Démarrer",
                dial: 2,
                dials: 4,
            }],
            ..look("Pause")
        });
        assert!(
            raised.0 < bare.0 && raised.1 < bare.1,
            "{raised:?} {bare:?}"
        );
    }

    #[test]
    fn every_reminder_is_drawn_in_its_own_column() {
        let renderer = Renderer::new().unwrap();
        let bare = renderer
            .render_notification(&look("Pause !"), Duration::ZERO, None)
            .unwrap();
        let reminders = [
            NotificationActionLook {
                text: "Démarrer la pause",
                dial: 2,
                dials: 4,
            },
            NotificationActionLook {
                text: "Masquer",
                dial: 3,
                dials: 4,
            },
        ];
        let drawn = renderer
            .render_notification(
                &NotificationLook {
                    actions: &reminders,
                    ..look("Pause !")
                },
                Duration::ZERO,
                None,
            )
            .unwrap();
        let (left, right) = changed_columns(&drawn.image, &bare.image, 75).unwrap();
        assert!((400..600).contains(&left), "third column inked: {left}");
        assert!((600..800).contains(&right), "fourth column inked: {right}");
    }
}
