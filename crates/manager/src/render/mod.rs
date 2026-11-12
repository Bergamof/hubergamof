//! Image rendering for keys and the touch strip.
//!
//! [`Renderer`] turns an optional source image (PNG, JPEG, SVG) plus an
//! optional label into an RGBA image of the target size, scaled to fit
//! ("contain") and centred on a background colour, with the frame of an
//! attention [`KeyState`]; it also renders the whole touch strip (module `strip`),
//! full-strip notifications and the transient [`Feedback`] mark drawn over a key. Key images are cached by
//! content (bounded segmented LRU, module `cache`: images used once,
//! like those of a countdown, do not push out those that come back) and
//! encoded to a PNG data URL only on demand.

mod cursor;
mod digest;
mod notification;
mod source;
mod strip;
mod svg;
mod text;

use std::collections::HashMap;
use std::io::Cursor;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use hubergamof_module_api::{Feedback, KeyState};
use hubergamof_protocol::{DeviceTheme, ImageSize};
use image::imageops::{self, FilterType};
use image::{ImageFormat, Rgba, RgbaImage};
use resvg::usvg;
use thiserror::Error;

pub use notification::{NotificationImage, NotificationLook};
pub use source::{
    ImageKind, ImageSource, MAX_SOURCE_BYTES, MAX_SOURCE_DIMENSION, check_source_length,
};
pub use text::{FONT_DATA, FONT_FAMILY, FontWeight, LineStyle, TextLine, TextRenderer};

use crate::cache::SegmentedLru;
use digest::{ContentDigest, DigestMemo};
use source::{Decoded, f32_from};

/// Background used when none is configured.
pub const DEFAULT_BACKGROUND: Rgba<u8> = Rgba([0, 0, 0, u8::MAX]);

/// Colour of label text.
pub const TEXT_COLOR: Rgba<u8> = Rgba([u8::MAX, u8::MAX, u8::MAX, u8::MAX]);

/// Background of a fully transparent image, to draw one over another surface.
pub const TRANSPARENT: Rgba<u8> = Rgba([0, 0, 0, 0]);

/// Background of a [`KeyState::Attention`] key (dark orange), visible around
/// and behind a transparent image.
pub const ATTENTION_BACKGROUND: Rgba<u8> = Rgba([0x5a, 0x2a, 0x00, u8::MAX]);

/// Opacity of the black veil darkening a surface under a feedback mark.
pub const FEEDBACK_VEIL_ALPHA: u8 = 150;

/// Number of reused rendered images kept in the cache (see
/// module `cache`): the faces of a few pages of the largest device (32 keys)
/// on several devices. Bounded in number, the images of one surface being
/// alike (counting the pixels of these and of the probation images, and
/// their PNG).
pub const RENDER_CACHE_CAPACITY: NonZeroUsize = match NonZeroUsize::new(192) {
    Some(capacity) => capacity,
    None => NonZeroUsize::MIN,
};

/// Number of new rendered images kept in the cache until they are used
/// again: what images used once (a countdown) can occupy.
pub const RENDER_PROBATION_CAPACITY: NonZeroUsize = match NonZeroUsize::new(32) {
    Some(capacity) => capacity,
    None => NonZeroUsize::MIN,
};

/// Size of a feedback mark, relative to the smaller side of the surface.
const FEEDBACK_MARK_RATIO: f32 = 0.7;

/// Size of the check mark of a ticked key, relative to the smaller side.
const CHECK_BADGE_RATIO: f32 = 0.3;
/// Distance of the check mark from the corner of the key, relative to the
/// smaller side: clear of the frame of the cursor.
const CHECK_BADGE_MARGIN_RATIO: f32 = 0.09;

/// Green disc with a white check mark ([`Feedback::Ok`]).
const OK_MARK_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="46" fill="#2e9d4a"/><path d="M27 52 L44 69 L74 34" fill="none" stroke="#ffffff" stroke-width="11" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
/// Red warning triangle with a white exclamation mark ([`Feedback::Alert`]).
const ALERT_MARK_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M50 6 L96 90 H4 Z" fill="#d93025" stroke="#d93025" stroke-width="6" stroke-linejoin="round"/><rect x="44.5" y="32" width="11" height="34" rx="5" fill="#ffffff"/><circle cx="50" cy="77" r="6.5" fill="#ffffff"/></svg>"##;

/// Key label font size, relative to the key height.
const LABEL_FONT_RATIO: f32 = 0.13;
/// Horizontal padding around a label, relative to the key width.
const LABEL_PADDING_RATIO: f32 = 0.05;
/// Vertical padding above and below a label, relative to the key height.
const LABEL_VERTICAL_PADDING_RATIO: f32 = 0.03;
const PNG_DATA_URL_PREFIX: &str = "data:image/png;base64,";

/// Rendering failure.
#[derive(Debug, Error)]
pub enum RenderError {
    /// The source is larger than [`MAX_SOURCE_BYTES`].
    #[error("image source is larger than {max} bytes")]
    SourceTooLarge {
        /// Accepted maximum.
        max: usize,
    },
    /// The source is not PNG, JPEG or SVG.
    #[error("unsupported image format (expected PNG, JPEG or SVG)")]
    UnsupportedFormat,
    /// The raster source could not be decoded or exceeds the dimension limits.
    #[error("cannot decode image: {0}")]
    Decode(#[source] image::ImageError),
    /// The SVG source could not be parsed.
    #[error("cannot parse SVG: {0}")]
    Svg(#[source] usvg::Error),
    /// The source file could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// File path.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The PNG encoding for a data URL failed.
    #[error("cannot encode PNG: {0}")]
    Encode(#[source] image::ImageError),
    /// The embedded font is corrupt.
    #[error("embedded font cannot be loaded")]
    Font,
    /// The target size is empty.
    #[error("target size must not be empty")]
    InvalidSize,
}

/// What to draw on a key.
#[derive(Debug, Clone, Copy)]
pub struct KeyRequest<'a> {
    /// Image scaled to fit the key, if any.
    pub source: Option<&'a ImageSource>,
    /// Target size.
    pub size: ImageSize,
    /// Label drawn at the bottom, if any.
    pub label: Option<&'a str>,
    /// Background colour, used when the state does not impose one.
    pub background: Rgba<u8>,
    /// Colour of the label, used when the state does not impose a background.
    pub text_color: Rgba<u8>,
    /// Visual state: only [`KeyState::Attention`] is drawn (background).
    pub state: KeyState,
    /// Whether the key holds the cursor of the contextual dial: a frame is
    /// drawn around it.
    pub focused: bool,
    /// Colour of the frame of the cursor, used when the state does not impose
    /// a background.
    pub accent: Rgba<u8>,
    /// Whether the key is ticked: a check mark is drawn in a corner.
    pub checked: bool,
}

impl<'a> KeyRequest<'a> {
    /// Request in the normal state with the default background.
    #[must_use]
    pub fn new(size: ImageSize, source: Option<&'a ImageSource>, label: Option<&'a str>) -> Self {
        Self {
            source,
            size,
            label,
            background: DEFAULT_BACKGROUND,
            text_color: TEXT_COLOR,
            state: KeyState::Normal,
            focused: false,
            accent: strip::accent(DeviceTheme::Light),
            checked: false,
        }
    }

    /// The same request on the surface of `theme`: the background of the
    /// touch strip, with its ink for the label.
    #[must_use]
    pub fn themed(self, theme: DeviceTheme) -> Self {
        let (background, text_color) = strip::surface(theme);
        Self {
            background,
            text_color,
            accent: strip::accent(theme),
            ..self
        }
    }

    /// The same request on a transparent background, for an image drawn
    /// over another surface (the page of the UI, a notification).
    #[must_use]
    pub fn transparent(self) -> Self {
        Self {
            background: TRANSPARENT,
            ..self
        }
    }

    /// The same request in state `state`.
    #[must_use]
    pub fn in_state(self, state: KeyState) -> Self {
        Self { state, ..self }
    }

    /// The same request for a key holding the cursor of the contextual dial
    /// (or not).
    #[must_use]
    pub fn focused(self, focused: bool) -> Self {
        Self { focused, ..self }
    }

    /// The same request for a key that is ticked (or not).
    #[must_use]
    pub fn checked(self, checked: bool) -> Self {
        Self { checked, ..self }
    }
}

/// Background of a key in `state`, if the state imposes one. No state draws
/// a frame: the icons carry the state.
fn state_background(state: KeyState) -> Option<Rgba<u8>> {
    match state {
        KeyState::Attention => Some(ATTENTION_BACKGROUND),
        // Normal, Active (the module's own icon shows it), and states added
        // later that this version cannot draw.
        _ => None,
    }
}

/// A rendered image, with its PNG encoding made lazily once (the data URL
/// is derived from it on demand rather than kept: base64 is cheap, a kept
/// copy would cost more than the PNG itself).
#[derive(Debug)]
pub struct Rendered {
    image: Arc<RgbaImage>,
    png: OnceLock<Arc<[u8]>>,
}

impl Rendered {
    fn new(image: RgbaImage) -> Self {
        Self {
            image: Arc::new(image),
            png: OnceLock::new(),
        }
    }

    /// The pixels.
    #[must_use]
    pub fn image(&self) -> &Arc<RgbaImage> {
        &self.image
    }

    /// The image as a `data:image/png;base64,…` URL, from its PNG.
    ///
    /// # Errors
    ///
    /// [`RenderError::Encode`].
    pub fn data_url(&self) -> Result<String, RenderError> {
        Ok(format!(
            "{PNG_DATA_URL_PREFIX}{}",
            BASE64.encode(self.png()?)
        ))
    }

    /// The image encoded as PNG, on first call.
    ///
    /// # Errors
    ///
    /// [`RenderError::Encode`].
    pub fn png(&self) -> Result<Arc<[u8]>, RenderError> {
        if let Some(png) = self.png.get() {
            return Ok(Arc::clone(png));
        }
        let png: Arc<[u8]> = to_png(&self.image)?.into();
        Ok(Arc::clone(self.png.get_or_init(|| png)))
    }
}

/// Encodes `image` as a PNG data URL.
///
/// # Errors
///
/// [`RenderError::Encode`].
pub fn to_data_url(image: &RgbaImage) -> Result<String, RenderError> {
    Ok(format!(
        "{PNG_DATA_URL_PREFIX}{}",
        BASE64.encode(to_png(image)?)
    ))
}

/// Encodes `image` as PNG.
///
/// # Errors
///
/// [`RenderError::Encode`].
pub fn to_png(image: &RgbaImage) -> Result<Vec<u8>, RenderError> {
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, ImageFormat::Png)
        .map_err(RenderError::Encode)?;
    Ok(png.into_inner())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CacheKey {
    Key {
        source: Option<ContentDigest>,
        size: (u32, u32),
        label: Option<String>,
        background: [u8; 4],
        text_color: [u8; 4],
        state: KeyState,
        /// Colour of the frame of the cursor, if the key holds it.
        frame: Option<[u8; 4]>,
        checked: bool,
    },
}

/// Renders key and strip images; shareable between threads.
pub struct Renderer {
    text: TextRenderer,
    cache: Mutex<SegmentedLru<CacheKey, Arc<Rendered>>>,
    digests: DigestMemo,
    /// Rasterised feedback marks by outcome and side.
    marks: Mutex<HashMap<(Feedback, u32), Arc<RgbaImage>>>,
    /// Text of the notification last drawn (see `notification`).
    notification_text: Mutex<Option<notification::TextLayer>>,
}

impl Renderer {
    /// Creates a renderer with an empty cache of [`RENDER_CACHE_CAPACITY`]
    /// reused and [`RENDER_PROBATION_CAPACITY`] new images.
    ///
    /// # Errors
    ///
    /// [`RenderError::Font`] if the embedded font is corrupt.
    pub fn new() -> Result<Self, RenderError> {
        Self::with_cache_capacity(RENDER_CACHE_CAPACITY, RENDER_PROBATION_CAPACITY)
    }

    /// Creates a renderer keeping at most `reused` images used more than
    /// once and `probation` new ones.
    ///
    /// # Errors
    ///
    /// [`RenderError::Font`] if the embedded font is corrupt.
    pub fn with_cache_capacity(
        reused: NonZeroUsize,
        probation: NonZeroUsize,
    ) -> Result<Self, RenderError> {
        Ok(Self {
            text: TextRenderer::new()?,
            cache: Mutex::new(SegmentedLru::new(reused, probation)),
            digests: DigestMemo::new(reused.saturating_add(probation.get())),
            marks: Mutex::new(HashMap::new()),
            notification_text: Mutex::new(None),
        })
    }

    /// Number of cached images.
    #[must_use]
    pub fn cached_len(&self) -> usize {
        self.lock_cache().len()
    }

    /// Renders a key image. CPU-bound: call it from a blocking context.
    ///
    /// # Errors
    ///
    /// Any [`RenderError`] from reading or decoding the source.
    pub fn render_key(&self, request: &KeyRequest<'_>) -> Result<Arc<Rendered>, RenderError> {
        check_size(request.size)?;
        let digest = request
            .source
            .map(|source| self.digests.digest(source))
            .transpose()?;
        let imposed = state_background(request.state);
        let background = imposed.unwrap_or(request.background);
        // The label stays white on the background a state imposes.
        let text_color = if imposed.is_some() {
            TEXT_COLOR
        } else {
            request.text_color
        };
        // The label's white shows on the background a state imposes: so does
        // the frame, the accent being too close to the orange.
        let frame = request.focused.then(|| {
            if imposed.is_some() {
                TEXT_COLOR
            } else {
                request.accent
            }
        });
        let key = CacheKey::Key {
            source: digest.as_ref().map(|digest| digest.digest),
            size: (request.size.width, request.size.height),
            label: request.label.map(str::to_owned),
            background: background.0,
            text_color: text_color.0,
            state: request.state,
            frame: frame.map(|color| color.0),
            checked: request.checked,
        };
        self.cached(key, || {
            // Read now if the digest was remembered without reading.
            let bytes = match (request.source, digest.and_then(|digest| digest.bytes)) {
                (Some(_), Some(bytes)) => Some(bytes),
                (Some(source), None) => Some(source.read()?),
                (None, _) => None,
            };
            let label = request
                .label
                .map(str::trim)
                .filter(|label| !label.is_empty());
            let icon_area = icon_area(request.size, label.is_some());
            let mut canvas = self.compose(bytes.as_deref(), request.size, icon_area, background)?;
            if let Some(label) = label {
                self.draw_label(&mut canvas, label, text_color)?;
            }
            if let Some(color) = frame {
                cursor::draw_frame(&mut canvas, color);
            }
            if request.checked {
                self.draw_check(&mut canvas)?;
            }
            Ok(keep_opaque(canvas, background))
        })
    }

    /// Renders `source` scaled to fit `size` on a transparent background (an
    /// image shown by the UI, which brings its own surface). CPU-bound: call it from a blocking context.
    ///
    /// # Errors
    ///
    /// Any [`RenderError`] from reading or decoding the source.
    pub fn render_image(
        &self,
        size: ImageSize,
        source: &ImageSource,
    ) -> Result<Arc<Rendered>, RenderError> {
        self.render_key(&KeyRequest::new(size, Some(source), None).transparent())
    }

    /// Checks that `bytes` are an image the renderer can draw at `size`
    /// (format, dimensions, SVG content) and returns its format. Nothing is
    /// cached nor remembered: meant for untrusted, one-off content (uploads,
    /// imports). CPU-bound.
    ///
    /// # Errors
    ///
    /// [`RenderError::SourceTooLarge`], [`RenderError::UnsupportedFormat`],
    /// [`RenderError::Decode`] or [`RenderError::Svg`].
    pub fn check_image(&self, bytes: &[u8], size: ImageSize) -> Result<ImageKind, RenderError> {
        check_size(size)?;
        check_source_length(bytes.len())?;
        let kind = ImageKind::sniff(bytes).ok_or(RenderError::UnsupportedFormat)?;
        self.compose(Some(bytes), size, size, DEFAULT_BACKGROUND)?;
        Ok(kind)
    }

    /// `base` with `feedback` drawn over it: darkened, with a green check
    /// mark ([`Feedback::Ok`]) or a red warning sign ([`Feedback::Alert`])
    /// in the middle. The result is not cached (marks are transient), the
    /// rasterised mark is. CPU-bound.
    ///
    /// # Errors
    ///
    /// [`RenderError::Svg`] or [`RenderError::InvalidSize`] (not expected
    /// with the built-in marks).
    pub fn with_feedback(
        &self,
        base: &Rendered,
        feedback: Feedback,
    ) -> Result<Arc<Rendered>, RenderError> {
        let mut canvas = base.image().as_ref().clone();
        let (width, height) = canvas.dimensions();
        darken(&mut canvas, FEEDBACK_VEIL_ALPHA);
        let side = scaled_dimension(width.min(height), FEEDBACK_MARK_RATIO, width.min(height));
        let mark = self.mark(feedback, side)?;
        imageops::overlay(
            &mut canvas,
            mark.as_ref(),
            i64::from((width - side) / 2),
            i64::from((height - side) / 2),
        );
        Ok(Arc::new(Rendered::new(keep_opaque(
            canvas,
            DEFAULT_BACKGROUND,
        ))))
    }

    /// The mark of `feedback`, `side` pixels wide.
    fn mark(&self, feedback: Feedback, side: u32) -> Result<Arc<RgbaImage>, RenderError> {
        let mut marks = self.marks.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(mark) = marks.get(&(feedback, side)) {
            return Ok(Arc::clone(mark));
        }
        let svg = match feedback {
            Feedback::Ok => OK_MARK_SVG,
            // Alert, and outcomes added later: a warning is the safe reading.
            _ => ALERT_MARK_SVG,
        };
        let tree =
            usvg::Tree::from_str(svg, self.text.source_svg_options()).map_err(RenderError::Svg)?;
        let mark = Arc::new(source::rasterize_svg(&tree, side, side)?);
        marks.insert((feedback, side), Arc::clone(&mark));
        Ok(mark)
    }

    fn cached(
        &self,
        key: CacheKey,
        render: impl FnOnce() -> Result<RgbaImage, RenderError>,
    ) -> Result<Arc<Rendered>, RenderError> {
        if let Some(hit) = self.lock_cache().get(&key) {
            return Ok(Arc::clone(hit));
        }
        let rendered = Arc::new(Rendered::new(render()?));
        self.lock_cache().insert(key, Arc::clone(&rendered));
        Ok(rendered)
    }

    /// Draws `bytes` scaled to fit `icon_area`, centred in it, on a canvas of
    /// `size`.
    fn compose(
        &self,
        bytes: Option<&[u8]>,
        size: ImageSize,
        icon_area: ImageSize,
        background: Rgba<u8>,
    ) -> Result<RgbaImage, RenderError> {
        let mut canvas = RgbaImage::from_pixel(size.width, size.height, background);
        let Some(bytes) = bytes else {
            return Ok(canvas);
        };
        let foreground = match source::decode(bytes, self.text.source_svg_options())? {
            Decoded::Raster(image) => contain(&image, icon_area),
            Decoded::Vector(tree) => {
                source::rasterize_svg(&tree, icon_area.width, icon_area.height)?
            }
        };
        let x = i64::from((size.width - foreground.width()) / 2);
        let y = i64::from((icon_area.height - foreground.height()) / 2);
        imageops::overlay(&mut canvas, &foreground, x, y);
        Ok(canvas)
    }

    /// Draws the check mark of a ticked key in its top right corner.
    fn draw_check(&self, canvas: &mut RgbaImage) -> Result<(), RenderError> {
        let (width, height) = canvas.dimensions();
        let smaller = width.min(height);
        let side = scaled_dimension(smaller, CHECK_BADGE_RATIO, smaller);
        let margin = scaled_dimension(smaller, CHECK_BADGE_MARGIN_RATIO, smaller);
        let badge = self.mark(Feedback::Ok, side)?;
        imageops::overlay(
            canvas,
            badge.as_ref(),
            i64::from(width.saturating_sub(side + margin)),
            i64::from(margin),
        );
        Ok(())
    }

    /// Draws `label` on one line, shortened with an ellipsis if needed, at the
    /// bottom of `canvas`.
    fn draw_label(
        &self,
        canvas: &mut RgbaImage,
        label: &str,
        color: Rgba<u8>,
    ) -> Result<(), RenderError> {
        let width = f32_from(canvas.width());
        let height = f32_from(canvas.height());
        let size = height * LABEL_FONT_RATIO;
        let max_width = width * (1.0 - 2.0 * LABEL_PADDING_RATIO);
        let text_top =
            height * (1.0 - LABEL_VERTICAL_PADDING_RATIO) - TextRenderer::line_height(size);
        let line = self.text.fitted_line(
            label,
            semibold(size, color),
            width / 2.0,
            text_top + TextRenderer::ascent(size),
            max_width,
        )?;
        self.text.draw(canvas, &[line])
    }

    fn lock_cache(&self) -> std::sync::MutexGuard<'_, SegmentedLru<CacheKey, Arc<Rendered>>> {
        // Cache entries are immutable: a poisoned lock is still consistent.
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The look of most texts: `SemiBold` of `size` pixels.
fn semibold(size: f32, color: Rgba<u8>) -> LineStyle {
    LineStyle {
        size,
        weight: FontWeight::SemiBold,
        color,
    }
}

/// Scales `image` to fit within `size` keeping its aspect ratio.
fn contain(image: &RgbaImage, size: ImageSize) -> RgbaImage {
    let scale = (f32_from(size.width) / f32_from(image.width()))
        .min(f32_from(size.height) / f32_from(image.height()));
    let width = scaled_dimension(image.width(), scale, size.width);
    let height = scaled_dimension(image.height(), scale, size.height);
    if (width, height) == image.dimensions() {
        image.clone()
    } else {
        // Resized premultiplied: the colour of transparent pixels must not
        // bleed into the edges of the image.
        let mut scaled = image.clone();
        premultiply(&mut scaled);
        let mut scaled = imageops::resize(&scaled, width, height, FilterType::Lanczos3);
        unpremultiply(&mut scaled);
        scaled
    }
}

/// Multiplies the colour channels of `image` by its opacity, in place.
fn premultiply(image: &mut RgbaImage) {
    for pixel in image.pixels_mut() {
        let alpha = u16::from(pixel.0[3]);
        for channel in &mut pixel.0[..3] {
            *channel = scale_channel(u16::from(*channel), alpha, u16::from(u8::MAX));
        }
    }
}

/// Undoes [`premultiply`], in place.
fn unpremultiply(image: &mut RgbaImage) {
    for pixel in image.pixels_mut() {
        let alpha = u16::from(pixel.0[3]);
        if alpha == 0 {
            continue;
        }
        for channel in &mut pixel.0[..3] {
            *channel = scale_channel(u16::from(*channel), u16::from(u8::MAX), alpha);
        }
    }
}

/// `value × numerator / denominator`, rounded, clamped to a channel.
fn scale_channel(value: u16, numerator: u16, denominator: u16) -> u8 {
    let scaled = (u32::from(value) * u32::from(numerator) + u32::from(denominator) / 2)
        / u32::from(denominator);
    u8::try_from(scaled).unwrap_or(u8::MAX)
}

/// `dimension × scale`, rounded, within `1..=max`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn scaled_dimension(dimension: u32, scale: f32, max: u32) -> u32 {
    // Clamped to 1..=max (a device size) before the cast.
    (f32_from(dimension) * scale)
        .round()
        .clamp(1.0, f32_from(max).max(1.0)) as u32
}

/// Blends black over `canvas` with opacity `alpha`, in place.
fn darken(canvas: &mut RgbaImage, alpha: u8) {
    let keep = u16::from(u8::MAX - alpha);
    for pixel in canvas.pixels_mut() {
        for channel in &mut pixel.0[..3] {
            // At most 255 · 255 / 255: always fits a u8.
            *channel =
                u8::try_from(u16::from(*channel) * keep / u16::from(u8::MAX)).unwrap_or(u8::MAX);
        }
    }
}

/// Restores full opacity lost to rounding in alpha blending when the
/// background is opaque.
fn keep_opaque(mut canvas: RgbaImage, background: Rgba<u8>) -> RgbaImage {
    if background.0[3] == u8::MAX {
        for pixel in canvas.pixels_mut() {
            pixel.0[3] = u8::MAX;
        }
    }
    canvas
}

/// Part of a key of `size` left to its image: all of it, or what is above
/// the label line, so that the image is never covered.
fn icon_area(size: ImageSize, labelled: bool) -> ImageSize {
    if !labelled {
        return size;
    }
    let height = f32_from(size.height);
    let line = TextRenderer::line_height(height * LABEL_FONT_RATIO);
    let strip = line + height * 2.0 * LABEL_VERTICAL_PADDING_RATIO;
    ImageSize {
        width: size.width,
        height: scaled_dimension(size.height, 1.0 - strip / height, size.height),
    }
}

fn check_size(size: ImageSize) -> Result<(), RenderError> {
    if size.width == 0 || size.height == 0 {
        Err(RenderError::InvalidSize)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
