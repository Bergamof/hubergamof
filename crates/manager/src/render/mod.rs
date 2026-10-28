//! Image rendering for keys and touch strip zones.
//!
//! [`Renderer`] turns an optional source image (PNG, JPEG, SVG) plus an
//! optional label into an RGBA image of the target size, scaled to fit
//! ("contain") and centred on a background colour; it also renders strip
//! zones from text lines. Results are cached by content (bounded LRU) and
//! encoded to a PNG data URL only on demand.

mod source;
mod text;

use std::io::Cursor;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use bergadeck_protocol::ImageSize;
use image::imageops::{self, FilterType};
use image::{ImageFormat, Rgba, RgbaImage};
use lru::LruCache;
use resvg::usvg;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub use source::{
    ImageKind, ImageSource, MAX_SOURCE_BYTES, MAX_SOURCE_DIMENSION, check_source_length,
};
pub use text::{FONT_DATA, FONT_FAMILY, TextLine, TextRenderer};

use source::{Decoded, f32_from};

/// Background used when none is configured.
pub const DEFAULT_BACKGROUND: Rgba<u8> = Rgba([0, 0, 0, u8::MAX]);

/// Colour of label and zone text.
pub const TEXT_COLOR: Rgba<u8> = Rgba([u8::MAX, u8::MAX, u8::MAX, u8::MAX]);

/// Translucent band behind a key label, keeping it readable on any image.
pub const LABEL_BAND_COLOR: Rgba<u8> = Rgba([0, 0, 0, 170]);

/// Number of rendered images kept in the cache.
pub const RENDER_CACHE_CAPACITY: NonZeroUsize = match NonZeroUsize::new(512) {
    Some(capacity) => capacity,
    None => NonZeroUsize::MIN,
};

/// Key label font size, relative to the key height.
const LABEL_FONT_RATIO: f32 = 0.16;
/// Maximum number of lines of a key label.
const LABEL_MAX_LINES: usize = 2;
/// Horizontal and vertical padding around a label, relative to the key width.
const LABEL_PADDING_RATIO: f32 = 0.05;

/// Maximum number of text lines in a strip zone.
pub const ZONE_MAX_LINES: usize = 3;
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
    /// Background colour.
    pub background: Rgba<u8>,
}

impl<'a> KeyRequest<'a> {
    /// Request with the default background.
    #[must_use]
    pub fn new(size: ImageSize, source: Option<&'a ImageSource>, label: Option<&'a str>) -> Self {
        Self {
            source,
            size,
            label,
            background: DEFAULT_BACKGROUND,
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
        source: Option<[u8; 32]>,
        size: (u32, u32),
        label: Option<String>,
        background: [u8; 4],
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
        let bytes = request.source.map(ImageSource::read).transpose()?;
        let key = CacheKey::Key {
            source: bytes.as_deref().map(content_digest),
            size: (request.size.width, request.size.height),
            label: request.label.map(str::to_owned),
            background: request.background.0,
        };
        self.cached(key, || {
            let mut canvas = self.compose(bytes.as_deref(), request.size, request.background)?;
            if let Some(label) = request.label.filter(|label| !label.trim().is_empty()) {
                self.draw_label(&mut canvas, label)?;
            }
            Ok(keep_opaque(canvas, request.background))
        })
    }

    /// Renders a strip zone with up to [`ZONE_MAX_LINES`] centred lines, the
    /// first one larger; extra lines are ignored.
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
        let lines = &lines[..lines.len().min(ZONE_MAX_LINES)];
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

fn content_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
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
