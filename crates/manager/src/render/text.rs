//! Text drawing with the embedded font (Noto Sans `SemiBold`, SIL OFL 1.1, see
//! `assets/fonts/OFL.txt`), shaped and rasterised by `usvg`/`resvg`.

use std::fmt::Write as _;

use image::{Rgba, RgbaImage, imageops};
use resvg::usvg;

use super::RenderError;
use super::source;

/// Embedded font file.
pub const FONT_DATA: &[u8] = include_bytes!("../../assets/fonts/NotoSans-SemiBold.ttf");

/// Family name of [`FONT_DATA`], also the default font of SVG sources.
pub const FONT_FAMILY: &str = "Noto Sans";

/// Ascender of the embedded font, in em (`hhea` table: 1069/1000).
const FONT_ASCENT_EM: f32 = 1.069;
/// Line height of the embedded font, in em (ascender + descender: 1362/1000).
const FONT_LINE_HEIGHT_EM: f32 = 1.362;

/// Appended to text cut to fit.
const ELLIPSIS: char = '…';

/// One line of text to draw, horizontally centred.
#[derive(Debug, Clone)]
pub struct TextLine {
    /// Text content.
    pub text: String,
    /// Font size in pixels.
    pub size: f32,
    /// Horizontal centre.
    pub center_x: f32,
    /// Vertical position of the baseline.
    pub baseline: f32,
    /// Text colour.
    pub color: Rgba<u8>,
}

/// Measures and draws text; also holds the SVG options (font database) used
/// for SVG sources.
pub struct TextRenderer {
    options: usvg::Options<'static>,
}

impl TextRenderer {
    /// Loads the embedded font.
    ///
    /// # Errors
    ///
    /// [`RenderError::Font`] if the embedded font cannot be loaded.
    pub fn new() -> Result<Self, RenderError> {
        let options = source::svg_options(FONT_DATA, FONT_FAMILY);
        if options.fontdb.is_empty() {
            return Err(RenderError::Font);
        }
        Ok(Self { options })
    }

    /// Options for parsing untrusted SVG with the embedded font.
    #[must_use]
    pub fn svg_options(&self) -> &usvg::Options<'static> {
        &self.options
    }

    /// Height of a line at `size` pixels.
    #[must_use]
    pub fn line_height(size: f32) -> f32 {
        size * FONT_LINE_HEIGHT_EM
    }

    /// Distance from the top of a line to its baseline at `size` pixels.
    #[must_use]
    pub fn ascent(size: f32) -> f32 {
        size * FONT_ASCENT_EM
    }

    /// Width of `text` at `size` pixels.
    ///
    /// # Errors
    ///
    /// [`RenderError::Svg`] if the text cannot be laid out.
    pub fn width(&self, text: &str, size: f32) -> Result<f32, RenderError> {
        if text.trim().is_empty() {
            return Ok(0.0);
        }
        let mut svg = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg">"#);
        push_text(&mut svg, text, size, 0.0, size, None);
        svg.push_str("</svg>");
        let tree = usvg::Tree::from_str(&svg, &self.options).map_err(RenderError::Svg)?;
        Ok(tree.root().abs_bounding_box().width())
    }

    /// `text` shortened with an ellipsis so that it fits in `max_width`.
    ///
    /// # Errors
    ///
    /// [`RenderError::Svg`] if the text cannot be laid out.
    pub fn fit(&self, text: &str, size: f32, max_width: f32) -> Result<String, RenderError> {
        if self.width(text, size)? <= max_width {
            return Ok(text.to_owned());
        }
        let characters: Vec<char> = text.chars().collect();
        // Longest prefix that fits with the ellipsis, by binary search.
        let (mut fitting, mut too_long) = (0, characters.len());
        while too_long - fitting > 1 {
            let middle = usize::midpoint(fitting, too_long);
            if self.width(&with_ellipsis(&characters[..middle]), size)? <= max_width {
                fitting = middle;
            } else {
                too_long = middle;
            }
        }
        Ok(with_ellipsis(&characters[..fitting]))
    }

    /// Splits `text` on words into at most `max_lines` lines of `max_width`;
    /// the last line is shortened with an ellipsis if text remains.
    ///
    /// # Errors
    ///
    /// [`RenderError::Svg`] if the text cannot be laid out.
    pub fn wrap(
        &self,
        text: &str,
        size: f32,
        max_width: f32,
        max_lines: usize,
    ) -> Result<Vec<String>, RenderError> {
        let mut lines: Vec<String> = Vec::new();
        if max_lines == 0 {
            return Ok(lines);
        }
        let mut words = text.split_whitespace().peekable();
        while let Some(word) = words.next() {
            let mut line = word.to_owned();
            if lines.len() + 1 >= max_lines {
                for rest in words.by_ref() {
                    line = format!("{line} {rest}");
                }
            } else {
                while let Some(next) = words.peek() {
                    let candidate = format!("{line} {next}");
                    if self.width(&candidate, size)? > max_width {
                        break;
                    }
                    line = candidate;
                    words.next();
                }
            }
            lines.push(self.fit(&line, size, max_width)?);
        }
        Ok(lines)
    }

    /// Draws `lines` over `canvas`.
    ///
    /// # Errors
    ///
    /// [`RenderError::Svg`] or [`RenderError::InvalidSize`].
    pub fn draw(&self, canvas: &mut RgbaImage, lines: &[TextLine]) -> Result<(), RenderError> {
        let (width, height) = canvas.dimensions();
        let mut svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">"#
        );
        for line in lines {
            push_text(
                &mut svg,
                &line.text,
                line.size,
                line.center_x,
                line.baseline,
                Some(line.color),
            );
        }
        svg.push_str("</svg>");
        let tree = usvg::Tree::from_str(&svg, &self.options).map_err(RenderError::Svg)?;
        let layer = source::rasterize_svg(&tree, width, height)?;
        imageops::overlay(canvas, &layer, 0, 0);
        Ok(())
    }
}

fn with_ellipsis(characters: &[char]) -> String {
    let mut text: String = characters.iter().collect();
    text.truncate(text.trim_end().len());
    text.push(ELLIPSIS);
    text
}

/// Appends a `<text>` element; centred on `x` and coloured when a colour is
/// given (measurement uses the default left-aligned black text).
fn push_text(
    svg: &mut String,
    text: &str,
    size: f32,
    x: f32,
    baseline: f32,
    color: Option<Rgba<u8>>,
) {
    // Writing to a `String` cannot fail.
    let _ = write!(
        svg,
        r#"<text x="{x}" y="{baseline}" font-family="{FONT_FAMILY}" font-size="{size}""#
    );
    if let Some(Rgba([red, green, blue, alpha])) = color {
        let opacity = f32::from(alpha) / f32::from(u8::MAX);
        let _ = write!(
            svg,
            r##" text-anchor="middle" fill="#{red:02x}{green:02x}{blue:02x}" fill-opacity="{opacity}""##
        );
    }
    svg.push('>');
    escape_xml(svg, text);
    svg.push_str("</text>");
}

fn escape_xml(svg: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => svg.push_str("&amp;"),
            '<' => svg.push_str("&lt;"),
            '>' => svg.push_str("&gt;"),
            '"' => svg.push_str("&quot;"),
            '\'' => svg.push_str("&apos;"),
            other => svg.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: f32 = 20.0;

    fn text() -> TextRenderer {
        TextRenderer::new().unwrap()
    }

    #[test]
    fn longer_text_is_wider() {
        let text = text();
        assert!(text.width("Bergadeck", SIZE).unwrap() > text.width("Berg", SIZE).unwrap());
        assert!(text.width("", SIZE).unwrap().abs() < f32::EPSILON);
    }

    #[test]
    fn fit_keeps_short_text_and_ellipsizes_long_text() {
        let text = text();
        assert_eq!(text.fit("Hi", SIZE, 100.0).unwrap(), "Hi");
        let fitted = text.fit("A very long label indeed", SIZE, 60.0).unwrap();
        assert!(fitted.ends_with(ELLIPSIS));
        assert!(fitted.len() > ELLIPSIS.len_utf8());
        assert!(text.width(&fitted, SIZE).unwrap() <= 60.0);
    }

    #[test]
    fn wrap_splits_on_words_and_limits_lines() {
        let text = text();
        let lines = text
            .wrap("Docker compose up now please", SIZE, 90.0, 2)
            .unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with(ELLIPSIS));
        assert!(
            lines
                .iter()
                .all(|line| text.width(line, SIZE).unwrap() <= 90.0)
        );
        assert_eq!(text.wrap("Mic", SIZE, 90.0, 2).unwrap(), ["Mic"]);
        assert_eq!(
            text.wrap("   ", SIZE, 90.0, 2).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn markup_in_text_is_escaped() {
        assert!(text().width("<b>&\"'</b>", SIZE).unwrap() > 0.0);
    }

    #[test]
    fn drawing_changes_pixels() {
        let text = text();
        let mut canvas = RgbaImage::from_pixel(100, 40, Rgba([0, 0, 0, 255]));
        let line = TextLine {
            text: "Hello".into(),
            size: SIZE,
            center_x: 50.0,
            baseline: 28.0,
            color: Rgba([255, 255, 255, 255]),
        };
        text.draw(&mut canvas, &[line]).unwrap();
        assert!(canvas.pixels().any(|pixel| pixel.0[0] > 128));
    }
}
