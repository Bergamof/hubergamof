//! Decoding of untrusted image sources (PNG, JPEG, SVG) with size guards.

use std::fs::File;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::{ImageFormat, ImageReader, Limits, RgbaImage};
use resvg::{tiny_skia, usvg};

use super::RenderError;

/// Largest accepted source, in bytes.
pub const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;

/// Largest accepted raster width or height, checked from the header before
/// decoding (protection against decompression bombs).
pub const MAX_SOURCE_DIMENSION: u32 = 4096;

/// Upper bound of the memory a raster decoder may allocate.
const MAX_DECODER_ALLOCATION: u64 = 128 * 1024 * 1024;

/// Number of leading bytes inspected to recognise an SVG document.
const SVG_SNIFF_LENGTH: usize = 1024;

/// Supported image formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageKind {
    /// Portable Network Graphics.
    Png,
    /// JPEG.
    Jpeg,
    /// Scalable Vector Graphics (uncompressed).
    Svg,
}

impl ImageKind {
    /// Recognises the format from the content, never from a file name.
    #[must_use]
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        match image::guess_format(bytes) {
            Ok(ImageFormat::Png) => Some(Self::Png),
            Ok(ImageFormat::Jpeg) => Some(Self::Jpeg),
            Ok(_) => None,
            Err(_) => looks_like_svg(bytes).then_some(Self::Svg),
        }
    }

    /// Usual file extension, without the dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Svg => "svg",
        }
    }
}

/// Whether `bytes` start like an SVG document: its root `<svg` element,
/// after an optional byte order mark, XML declaration, comments and doctype
/// only. A text merely mentioning `<svg` (a YAML or shell file…) is not one.
fn looks_like_svg(bytes: &[u8]) -> bool {
    const PROLOG: [(&str, &str); 3] = [("<?xml", "?>"), ("<!--", "-->"), ("<!DOCTYPE", ">")];
    let head = &bytes[..bytes.len().min(SVG_SNIFF_LENGTH)];
    let text = String::from_utf8_lossy(head);
    let mut rest = text.trim_start_matches('\u{feff}').trim_start();
    loop {
        let Some((_, end)) = PROLOG.iter().find(|(start, _)| rest.starts_with(start)) else {
            return rest.starts_with("<svg");
        };
        let Some(close) = rest.find(end) else {
            return false;
        };
        rest = rest[close + end.len()..].trim_start();
    }
}

/// Where an image comes from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImageSource {
    /// In-memory content.
    Bytes(Arc<[u8]>),
    /// File on disk, read at render time.
    File(PathBuf),
}

impl ImageSource {
    /// Content of the source, at most [`MAX_SOURCE_BYTES`].
    ///
    /// # Errors
    ///
    /// [`RenderError::SourceTooLarge`] or [`RenderError::Io`].
    pub fn read(&self) -> Result<Arc<[u8]>, RenderError> {
        match self {
            Self::Bytes(bytes) => {
                check_source_length(bytes.len())?;
                Ok(Arc::clone(bytes))
            }
            Self::File(path) => read_bounded(path),
        }
    }
}

/// Fails when `length` exceeds [`MAX_SOURCE_BYTES`].
///
/// # Errors
///
/// [`RenderError::SourceTooLarge`].
pub fn check_source_length(length: usize) -> Result<(), RenderError> {
    if length <= MAX_SOURCE_BYTES {
        Ok(())
    } else {
        Err(RenderError::SourceTooLarge {
            max: MAX_SOURCE_BYTES,
        })
    }
}

fn read_bounded(path: &Path) -> Result<Arc<[u8]>, RenderError> {
    let io_error = |source| RenderError::Io {
        path: path.to_owned(),
        source,
    };
    let file = File::open(path).map_err(io_error)?;
    let mut bytes = Vec::new();
    // Reading one byte past the limit detects oversized files without
    // trusting metadata (the file may change in between).
    let limit = u64::try_from(MAX_SOURCE_BYTES).map_or(u64::MAX, |max| max + 1);
    file.take(limit).read_to_end(&mut bytes).map_err(io_error)?;
    check_source_length(bytes.len())?;
    Ok(bytes.into())
}

/// A decoded source, ready to be scaled.
pub enum Decoded {
    /// Raster pixels at their native size.
    Raster(RgbaImage),
    /// Parsed vector document, rendered at the target size.
    Vector(Box<usvg::Tree>),
}

/// Decodes `bytes` according to their sniffed format.
///
/// # Errors
///
/// [`RenderError::UnsupportedFormat`], [`RenderError::Decode`] (including
/// dimension limits) or [`RenderError::Svg`].
pub fn decode(bytes: &[u8], svg_options: &usvg::Options<'_>) -> Result<Decoded, RenderError> {
    check_source_length(bytes.len())?;
    match ImageKind::sniff(bytes).ok_or(RenderError::UnsupportedFormat)? {
        ImageKind::Png | ImageKind::Jpeg => decode_raster(bytes).map(Decoded::Raster),
        ImageKind::Svg => usvg::Tree::from_data(bytes, svg_options)
            .map(|tree| Decoded::Vector(Box::new(tree)))
            .map_err(RenderError::Svg),
    }
}

fn decode_raster(bytes: &[u8]) -> Result<RgbaImage, RenderError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| RenderError::Decode(image::ImageError::IoError(error)))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_DIMENSION);
    limits.max_image_height = Some(MAX_SOURCE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODER_ALLOCATION);
    reader.limits(limits);
    Ok(reader.decode().map_err(RenderError::Decode)?.into_rgba8())
}

/// Options for untrusted SVG: no external file, no embedded raster image, no
/// system font; text uses the embedded `fonts` only, all of `font_family`.
#[must_use]
pub fn svg_options(fonts: &[&'static [u8]], font_family: &str) -> usvg::Options<'static> {
    let mut options = usvg::Options {
        resources_dir: None,
        font_family: font_family.to_owned(),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let database = options.fontdb_mut();
    for font in fonts {
        database.load_font_data(font.to_vec());
    }
    database.set_sans_serif_family(font_family);
    options
}

/// Renders `tree` scaled to fit ("contain") and centred in a transparent
/// `width`×`height` image.
///
/// # Errors
///
/// [`RenderError::InvalidSize`] when the size is zero.
pub fn rasterize_svg(tree: &usvg::Tree, width: u32, height: u32) -> Result<RgbaImage, RenderError> {
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(RenderError::InvalidSize)?;
    let size = tree.size();
    let scale = (f32_from(width) / size.width()).min(f32_from(height) / size.height());
    let offset_x = (f32_from(width) - size.width() * scale) / 2.0;
    let offset_y = (f32_from(height) - size.height() * scale) / 2.0;
    let transform =
        tiny_skia::Transform::from_scale(scale, scale).post_translate(offset_x, offset_y);
    resvg::render(tree, transform, &mut pixmap.as_mut());
    RgbaImage::from_raw(width, height, pixmap.take_demultiplied()).ok_or(RenderError::InvalidSize)
}

/// Lossy conversion of a pixel dimension, exact up to 2^24.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn f32_from(value: u32) -> f32 {
    value as f32
}
