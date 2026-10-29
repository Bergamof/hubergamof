//! Image rendering for keys and touch strip zones.
//!
//! [`Renderer`] turns an optional source image (PNG, JPEG, SVG) plus an
//! optional label into an RGBA image of the target size, scaled to fit
//! ("contain") and centred on a background colour, with the frame of its
//! [`KeyState`]; it also renders strip zones from text lines or an image, and
//! the transient [`Feedback`] mark drawn over a surface. Results are cached by
//! content (bounded LRU) and encoded to a PNG data URL only on demand.

mod digest;
mod source;
mod text;

use std::collections::HashMap;
use std::io::Cursor;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use bergadeck_module_api::{Feedback, KeyState};
use bergadeck_protocol::ImageSize;
use image::imageops::{self, FilterType};
use image::{ImageFormat, Rgba, RgbaImage};
use lru::LruCache;
use resvg::usvg;
use thiserror::Error;

pub use source::{
    ImageKind, ImageSource, MAX_SOURCE_BYTES, MAX_SOURCE_DIMENSION, check_source_length,
};
pub use text::{FONT_DATA, FONT_FAMILY, TextLine, TextRenderer};

use digest::{ContentDigest, DigestMemo};
use source::{Decoded, f32_from};

/// Background used when none is configured.
pub const DEFAULT_BACKGROUND: Rgba<u8> = Rgba([0, 0, 0, u8::MAX]);

/// Colour of label and zone text.
pub const TEXT_COLOR: Rgba<u8> = Rgba([u8::MAX, u8::MAX, u8::MAX, u8::MAX]);

/// Translucent band behind a key label, keeping it readable on any image.
pub const LABEL_BAND_COLOR: Rgba<u8> = Rgba([0, 0, 0, 170]);

/// Frame of an [`KeyState::Active`] key.
pub const ACTIVE_BORDER_COLOR: Rgba<u8> = Rgba([0x3b, 0xa1, 0xff, u8::MAX]);

/// Frame of a [`KeyState::Attention`] key.
pub const ATTENTION_BORDER_COLOR: Rgba<u8> = Rgba([0xff, 0x8c, 0x00, u8::MAX]);

/// Background of a [`KeyState::Attention`] key (dark orange), visible around
/// and behind a transparent image.
pub const ATTENTION_BACKGROUND: Rgba<u8> = Rgba([0x5a, 0x2a, 0x00, u8::MAX]);

/// Opacity of the black veil darkening a surface under a feedback mark.
pub const FEEDBACK_VEIL_ALPHA: u8 = 150;

/// Number of rendered images kept in the cache.
pub const RENDER_CACHE_CAPACITY: NonZeroUsize = match NonZeroUsize::new(512) {
    Some(capacity) => capacity,
    None => NonZeroUsize::MIN,
};

/// Width of the frame of an active key, relative to the key width.
const ACTIVE_BORDER_RATIO: f32 = 0.05;
/// Width of the frame of a key needing attention, relative to the key width.
const ATTENTION_BORDER_RATIO: f32 = 0.08;
/// Size of a feedback mark, relative to the smaller side of the surface.
const FEEDBACK_MARK_RATIO: f32 = 0.7;

/// Green disc with a white check mark ([`Feedback::Ok`]).
const OK_MARK_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="46" fill="#2e9d4a"/><path d="M27 52 L44 69 L74 34" fill="none" stroke="#ffffff" stroke-width="11" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
/// Red warning triangle with a white exclamation mark ([`Feedback::Alert`]).
const ALERT_MARK_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M50 6 L96 90 H4 Z" fill="#d93025" stroke="#d93025" stroke-width="6" stroke-linejoin="round"/><rect x="44.5" y="32" width="11" height="34" rx="5" fill="#ffffff"/><circle cx="50" cy="77" r="6.5" fill="#ffffff"/></svg>"##;

/// Key label font size, relative to the key height.
const LABEL_FONT_RATIO: f32 = 0.16;
/// Maximum number of lines of a key label.
const LABEL_MAX_LINES: usize = 2;
/// Horizontal and vertical padding around a label, relative to the key width.
const LABEL_PADDING_RATIO: f32 = 0.05;
/// Font size of the first zone line, relative to the zone height.
const ZONE_TITLE_FONT_RATIO: f32 = 0.30;
/// Font size of the other zone lines, relative to the zone height.
const ZONE_DETAIL_FONT_RATIO: f32 = 0.20;
/// Horizontal padding of zone lines, relative to the zone width.
const ZONE_PADDING_RATIO: f32 = 0.04;

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
    /// Visual state, drawn as a frame (and a background for
    /// [`KeyState::Attention`]).
    pub state: KeyState,
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
            state: KeyState::Normal,
        }
    }

    /// The same request in state `state`.
    #[must_use]
    pub fn in_state(self, state: KeyState) -> Self {
        Self { state, ..self }
    }
}

/// How a [`KeyState`] is drawn: background and frame.
struct StateStyle {
    background: Option<Rgba<u8>>,
    border: Option<(Rgba<u8>, f32)>,
}

impl StateStyle {
    fn of(state: KeyState) -> Self {
        match state {
            KeyState::Active => Self {
                background: None,
                border: Some((ACTIVE_BORDER_COLOR, ACTIVE_BORDER_RATIO)),
            },
            KeyState::Attention => Self {
                background: Some(ATTENTION_BACKGROUND),
                border: Some((ATTENTION_BORDER_COLOR, ATTENTION_BORDER_RATIO)),
            },
            // Normal, and states added later that this version cannot draw.
            _ => Self {
                background: None,
                border: None,
            },
        }
    }
}

/// A rendered image, with its data URL encoded lazily once.
#[derive(Debug)]
pub struct Rendered {
    image: Arc<RgbaImage>,
    data_url: OnceLock<String>,
}

impl Rendered {
    fn new(image: RgbaImage) -> Self {
        Self {
            image: Arc::new(image),
            data_url: OnceLock::new(),
        }
    }

    /// The pixels.
    #[must_use]
    pub fn image(&self) -> &Arc<RgbaImage> {
        &self.image
    }

    /// The image as a `data:image/png;base64,…` URL, encoded on first call.
    ///
    /// # Errors
    ///
    /// [`RenderError::Encode`].
    pub fn data_url(&self) -> Result<&str, RenderError> {
        if let Some(url) = self.data_url.get() {
            return Ok(url);
        }
        let url = to_data_url(&self.image)?;
        // A concurrent caller may have won the race; both values are equal.
        Ok(self.data_url.get_or_init(|| url))
    }
}

/// Encodes `image` as a PNG data URL.
///
/// # Errors
///
/// [`RenderError::Encode`].
pub fn to_data_url(image: &RgbaImage) -> Result<String, RenderError> {
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, ImageFormat::Png)
        .map_err(RenderError::Encode)?;
    Ok(format!(
        "{PNG_DATA_URL_PREFIX}{}",
        BASE64.encode(png.into_inner())
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CacheKey {
    Key {
        source: Option<ContentDigest>,
        size: (u32, u32),
        label: Option<String>,
        background: [u8; 4],
        state: KeyState,
    },
    Zone {
        lines: Vec<String>,
        size: (u32, u32),
        background: [u8; 4],
    },
}

/// Renders key and strip zone images; shareable between threads.
pub struct Renderer {
    text: TextRenderer,
    cache: Mutex<LruCache<CacheKey, Arc<Rendered>>>,
    digests: DigestMemo,
    /// Rasterised feedback marks by outcome and side.
    marks: Mutex<HashMap<(Feedback, u32), Arc<RgbaImage>>>,
}

impl Renderer {
    /// Creates a renderer with an empty cache of [`RENDER_CACHE_CAPACITY`].
    ///
    /// # Errors
    ///
    /// [`RenderError::Font`] if the embedded font is corrupt.
    pub fn new() -> Result<Self, RenderError> {
        Self::with_cache_capacity(RENDER_CACHE_CAPACITY)
    }

    /// Creates a renderer keeping at most `capacity` images.
    ///
    /// # Errors
    ///
    /// [`RenderError::Font`] if the embedded font is corrupt.
    pub fn with_cache_capacity(capacity: NonZeroUsize) -> Result<Self, RenderError> {
        Ok(Self {
            text: TextRenderer::new()?,
            cache: Mutex::new(LruCache::new(capacity)),
            digests: DigestMemo::new(capacity),
            marks: Mutex::new(HashMap::new()),
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
        let style = StateStyle::of(request.state);
        let background = style.background.unwrap_or(request.background);
        let key = CacheKey::Key {
            source: digest.as_ref().map(|digest| digest.digest),
            size: (request.size.width, request.size.height),
            label: request.label.map(str::to_owned),
            background: background.0,
            state: request.state,
        };
        self.cached(key, || {
            // Read now if the digest was remembered without reading.
            let bytes = match (request.source, digest.and_then(|digest| digest.bytes)) {
                (Some(_), Some(bytes)) => Some(bytes),
                (Some(source), None) => Some(source.read()?),
                (None, _) => None,
            };
            let mut canvas = self.compose(bytes.as_deref(), request.size, background)?;
            if let Some(label) = request.label.filter(|label| !label.trim().is_empty()) {
                self.draw_label(&mut canvas, label)?;
            }
            if let Some((color, ratio)) = style.border {
                draw_border(&mut canvas, color, ratio);
            }
            Ok(keep_opaque(canvas, background))
        })
    }

    /// Renders `source` scaled to fit `size` on the default background (a
    /// touch strip zone image). CPU-bound: call it from a blocking context.
    ///
    /// # Errors
    ///
    /// Any [`RenderError`] from reading or decoding the source.
    pub fn render_image(
        &self,
        size: ImageSize,
        source: &ImageSource,
    ) -> Result<Arc<Rendered>, RenderError> {
        self.render_key(&KeyRequest::new(size, Some(source), None))
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
        let tree = usvg::Tree::from_str(svg, self.text.svg_options()).map_err(RenderError::Svg)?;
        let mark = Arc::new(source::rasterize_svg(&tree, side, side)?);
        marks.insert((feedback, side), Arc::clone(&mark));
        Ok(mark)
    }

    /// Renders a strip zone with centred lines, the first one larger. The
    /// layout is made for at most [`bergadeck_module_api::MAX_ZONE_LINES`] lines (the engine
    /// truncates longer zones); more would overflow the zone.
    ///
    /// # Errors
    ///
    /// [`RenderError::InvalidSize`] when the size is empty.
    pub fn render_zone(
        &self,
        lines: &[String],
        size: ImageSize,
        background: Rgba<u8>,
    ) -> Result<Arc<Rendered>, RenderError> {
        check_size(size)?;
        let key = CacheKey::Zone {
            lines: lines.to_vec(),
            size: (size.width, size.height),
            background: background.0,
        };
        self.cached(key, || {
            let mut canvas = RgbaImage::from_pixel(size.width, size.height, background);
            self.draw_zone_lines(&mut canvas, lines)?;
            Ok(keep_opaque(canvas, background))
        })
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
        self.lock_cache().put(key, Arc::clone(&rendered));
        Ok(rendered)
    }

    fn compose(
        &self,
        bytes: Option<&[u8]>,
        size: ImageSize,
        background: Rgba<u8>,
    ) -> Result<RgbaImage, RenderError> {
        let mut canvas = RgbaImage::from_pixel(size.width, size.height, background);
        let Some(bytes) = bytes else {
            return Ok(canvas);
        };
        let foreground = match source::decode(bytes, self.text.svg_options())? {
            Decoded::Raster(image) => contain(&image, size),
            Decoded::Vector(tree) => source::rasterize_svg(&tree, size.width, size.height)?,
        };
        let x = i64::from((size.width - foreground.width()) / 2);
        let y = i64::from((size.height - foreground.height()) / 2);
        imageops::overlay(&mut canvas, &foreground, x, y);
        Ok(canvas)
    }

    fn draw_label(&self, canvas: &mut RgbaImage, label: &str) -> Result<(), RenderError> {
        let width = f32_from(canvas.width());
        let height = f32_from(canvas.height());
        let size = height * LABEL_FONT_RATIO;
        let padding = width * LABEL_PADDING_RATIO;
        let lines = self
            .text
            .wrap(label, size, width - 2.0 * padding, LABEL_MAX_LINES)?;
        let line_height = TextRenderer::line_height(size);
        let text_top = height - padding - line_height * count_f32(lines.len());
        darken_bottom(canvas, text_top - padding);
        let lines: Vec<TextLine> = lines
            .into_iter()
            .enumerate()
            .map(|(index, text)| TextLine {
                text,
                size,
                center_x: width / 2.0,
                baseline: text_top + line_height * count_f32(index) + TextRenderer::ascent(size),
                color: TEXT_COLOR,
            })
            .collect();
        self.text.draw(canvas, &lines)
    }

    fn draw_zone_lines(&self, canvas: &mut RgbaImage, lines: &[String]) -> Result<(), RenderError> {
        let width = f32_from(canvas.width());
        let height = f32_from(canvas.height());
        let max_width = width * (1.0 - 2.0 * ZONE_PADDING_RATIO);
        let sizes = (0..lines.len()).map(|index| {
            let ratio = if index == 0 {
                ZONE_TITLE_FONT_RATIO
            } else {
                ZONE_DETAIL_FONT_RATIO
            };
            height * ratio
        });
        let total: f32 = sizes.clone().map(TextRenderer::line_height).sum();
        let mut top = (height - total) / 2.0;
        let mut drawn = Vec::with_capacity(lines.len());
        for (line, size) in lines.iter().zip(sizes) {
            drawn.push(TextLine {
                text: self.text.fit(line, size, max_width)?,
                size,
                center_x: width / 2.0,
                baseline: top + TextRenderer::ascent(size),
                color: TEXT_COLOR,
            });
            top += TextRenderer::line_height(size);
        }
        self.text.draw(canvas, &drawn)
    }

    fn lock_cache(&self) -> std::sync::MutexGuard<'_, LruCache<CacheKey, Arc<Rendered>>> {
        // Cache entries are immutable: a poisoned lock is still consistent.
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
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
        imageops::resize(image, width, height, FilterType::Lanczos3)
    }
}

/// `dimension × scale`, rounded, within `1..=max`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn scaled_dimension(dimension: u32, scale: f32, max: u32) -> u32 {
    // Clamped to 1..=max (a device size) before the cast.
    (f32_from(dimension) * scale)
        .round()
        .clamp(1.0, f32_from(max)) as u32
}

/// Blends [`LABEL_BAND_COLOR`] over the rows from `top` (in pixels, may be
/// fractional or negative) to the bottom of `canvas`.
fn darken_bottom(canvas: &mut RgbaImage, top: f32) {
    let first_row = (0..canvas.height())
        .find(|row| f32_from(*row) >= top.floor())
        .unwrap_or(canvas.height());
    let band = RgbaImage::from_pixel(
        canvas.width(),
        canvas.height() - first_row,
        LABEL_BAND_COLOR,
    );
    imageops::overlay(canvas, &band, 0, i64::from(first_row));
}

/// Draws a frame of `color` along the edges of `canvas`, `ratio` of its
/// width thick (at least one pixel).
fn draw_border(canvas: &mut RgbaImage, color: Rgba<u8>, ratio: f32) {
    let (width, height) = canvas.dimensions();
    let thickness = scaled_dimension(width, ratio, (width.min(height) / 2).max(1));
    for (x, y, pixel) in canvas.enumerate_pixels_mut() {
        let inside =
            x >= thickness && y >= thickness && x < width - thickness && y < height - thickness;
        if !inside {
            *pixel = color;
        }
    }
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

/// Small counts (line numbers) as `f32`.
fn count_f32(value: usize) -> f32 {
    f32_from(u32::try_from(value).unwrap_or(u32::MAX))
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
