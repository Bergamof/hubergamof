//! Text drawing with the embedded font (Noto Sans `SemiBold`, SIL
//! OFL 1.1, see `assets/fonts/OFL.txt`), shaped and rasterised by
//! `usvg`/`resvg`.
//!
//! Provenance and checksums of the font files: `assets/fonts/README.md`.

use std::fmt::Write as _;

use image::{Rgba, RgbaImage, imageops};
use resvg::usvg;

use super::RenderError;
use super::source;

/// Embedded `SemiBold` font file, the default weight.
const SEMIBOLD_FONT_DATA: &[u8] = include_bytes!("../../assets/fonts/NotoSans-SemiBold.ttf");

/// Embedded font files, all of the family [`FONT_FAMILY`].
pub const FONT_DATA: [&[u8]; 1] = [SEMIBOLD_FONT_DATA];

/// Family name of [`FONT_DATA`], also the default font of SVG sources.
pub const FONT_FAMILY: &str = "Noto Sans";

/// Weight of the text, among the embedded fonts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontWeight {
    /// `SemiBold` (CSS weight 600), used almost everywhere.
    SemiBold,
}

impl FontWeight {
    /// CSS `font-weight` value of the weight.
    pub(super) const fn css(self) -> u16 {
        match self {
            Self::SemiBold => 600,
        }
    }
}

/// Ascender of the embedded font, in em (`hhea` table: 1069/1000).
const FONT_ASCENT_EM: f32 = 1.069;
/// Line height of the embedded font, in em (ascender + descender: 1362/1000).
const FONT_LINE_HEIGHT_EM: f32 = 1.362;

/// Appended to text cut to fit.
const ELLIPSIS: char = '…';

/// How a line of text looks.
#[derive(Debug, Clone, Copy)]
pub struct LineStyle {
    /// Font size in pixels.
    pub size: f32,
    /// Font weight.
    pub weight: FontWeight,
    /// Text colour.
    pub color: Rgba<u8>,
}

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
    /// Font weight.
    pub weight: FontWeight,
}

/// Measures and draws text; also holds the SVG options (font database) used
/// for SVG sources and for the text it draws.
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
        let options = source::svg_options(&FONT_DATA, FONT_FAMILY);
        if options.fontdb.len() < FONT_DATA.len() {
            return Err(RenderError::Font);
        }
        Ok(Self { options })
    }

    /// Options for parsing untrusted SVG sources: the embedded font only.
    #[must_use]
    pub fn source_svg_options(&self) -> &usvg::Options<'static> {
        &self.options
    }

    /// A line of `text` as it is, in `style`, centred on `center_x`.
    #[must_use]
    pub fn line(text: &str, style: LineStyle, center_x: f32, baseline: f32) -> TextLine {
        TextLine {
            text: text.to_owned(),
            size: style.size,
            center_x,
            baseline,
            color: style.color,
            weight: style.weight,
        }
    }

    /// A line of `text` in `style`, shortened with an ellipsis if it is wider
    /// than `max_width`. The text is measured in the weight it is drawn in.
    ///
    /// # Errors
    ///
    /// [`RenderError::Svg`] if the text cannot be laid out.
    pub fn fitted_line(
        &self,
        text: &str,
        style: LineStyle,
        center_x: f32,
        baseline: f32,
        max_width: f32,
    ) -> Result<TextLine, RenderError> {
        let fitted = self.fit(text, style.size, style.weight, max_width)?;
        Ok(Self::line(&fitted, style, center_x, baseline))
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

    /// Width of `text` at `size` pixels in `weight`.
    ///
    /// # Errors
    ///
    /// [`RenderError::Svg`] if the text cannot be laid out.
    pub fn width(&self, text: &str, size: f32, weight: FontWeight) -> Result<f32, RenderError> {
        if text.trim().is_empty() {
            return Ok(0.0);
        }
        let mut svg = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg">"#);
        push_text(&mut svg, text, size, weight, 0.0, size, None);
        svg.push_str("</svg>");
        let tree = usvg::Tree::from_str(&svg, &self.options).map_err(RenderError::Svg)?;
        Ok(tree.root().abs_bounding_box().width())
    }

    /// `text` shortened with an ellipsis so that it fits in `max_width` in
    /// `weight`.
    ///
    /// # Errors
    ///
    /// [`RenderError::Svg`] if the text cannot be laid out.
    pub fn fit(
        &self,
        text: &str,
        size: f32,
        weight: FontWeight,
        max_width: f32,
    ) -> Result<String, RenderError> {
        if self.width(text, size, weight)? <= max_width {
            return Ok(text.to_owned());
        }
        let characters: Vec<char> = text.chars().collect();
        // Longest prefix that fits with the ellipsis, by binary search.
        let (mut fitting, mut too_long) = (0, characters.len());
        while too_long - fitting > 1 {
            let middle = usize::midpoint(fitting, too_long);
            if self.width(&with_ellipsis(&characters[..middle]), size, weight)? <= max_width {
                fitting = middle;
            } else {
                too_long = middle;
            }
        }
        Ok(with_ellipsis(&characters[..fitting]))
    }

    /// Draws `lines` over `canvas`.
    ///
    /// # Errors
    ///
    /// [`RenderError::Svg`] or [`RenderError::InvalidSize`].
    pub fn draw(&self, canvas: &mut RgbaImage, lines: &[TextLine]) -> Result<(), RenderError> {
        let mut body = String::new();
        for line in lines {
            push_text(
                &mut body,
                &line.text,
                line.size,
                line.weight,
                line.center_x,
                line.baseline,
                Some(line.color),
            );
        }
        self.overlay_svg(canvas, &body)
    }

    /// Draws the SVG elements `body`, in the pixel coordinates of `canvas`,
    /// over it. Text in `body` may use every embedded weight.
    ///
    /// # Errors
    ///
    /// [`RenderError::Svg`] or [`RenderError::InvalidSize`].
    pub fn overlay_svg(&self, canvas: &mut RgbaImage, body: &str) -> Result<(), RenderError> {
        let (width, height) = canvas.dimensions();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{body}</svg>"#
        );
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
    weight: FontWeight,
    x: f32,
    baseline: f32,
    color: Option<Rgba<u8>>,
) {
    // Writing to a `String` cannot fail.
    let _ = write!(
        svg,
        r#"<text x="{x}" y="{baseline}" font-family="{FONT_FAMILY}" font-size="{size}" font-weight="{}""#,
        weight.css()
    );
    if let Some(color) = color {
        let opacity = f32::from(color.0[3]) / f32::from(u8::MAX);
        let _ = write!(
            svg,
            r#" text-anchor="middle" fill="{}" fill-opacity="{opacity}""#,
            source::svg_color(color)
        );
    }
    svg.push('>');
    escape_xml(svg, text);
    svg.push_str("</text>");
}

pub(super) fn escape_xml(svg: &mut String, text: &str) {
    for character in text.chars().filter(|character| is_xml_char(*character)) {
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

/// Whether XML 1.0 allows `character`: a text with another one (a control
/// character) is not a document, and would stop the whole drawing.
fn is_xml_char(character: char) -> bool {
    matches!(character,
        '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
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
        assert!(
            text.width("HuBergamof", SIZE, FontWeight::SemiBold)
                .unwrap()
                > text.width("Berg", SIZE, FontWeight::SemiBold).unwrap()
        );
        assert!(text.width("", SIZE, FontWeight::SemiBold).unwrap().abs() < f32::EPSILON);
    }

    #[test]
    fn fit_keeps_short_text_and_ellipsizes_long_text() {
        let text = text();
        assert_eq!(
            text.fit("Hi", SIZE, FontWeight::SemiBold, 100.0).unwrap(),
            "Hi"
        );
        let fitted = text
            .fit("A very long label indeed", SIZE, FontWeight::SemiBold, 60.0)
            .unwrap();
        assert!(fitted.ends_with(ELLIPSIS));
        assert!(fitted.len() > ELLIPSIS.len_utf8());
        assert!(text.width(&fitted, SIZE, FontWeight::SemiBold).unwrap() <= 60.0);
    }

    /// Width of `text` drawn by a SVG source, which is parsed with the
    /// options of untrusted SVG.
    fn source_text_width(text: &TextRenderer, attributes: &str) -> f32 {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text y="{SIZE}" font-size="{SIZE}" {attributes}>00:00</text></svg>"#
        );
        let tree = usvg::Tree::from_str(&svg, text.source_svg_options()).unwrap();
        tree.root().abs_bounding_box().width()
    }

    #[test]
    fn svg_sources_see_the_embedded_font_whatever_weight_they_ask() {
        let text = text();
        assert_eq!(text.source_svg_options().fontdb.len(), FONT_DATA.len());
        let semibold = text.width("00:00", SIZE, FontWeight::SemiBold).unwrap();
        // Without a weight (400) or asking for another one: SemiBold anyway.
        for attributes in ["", r#"font-weight="300""#, r#"font-weight="100""#] {
            let width = source_text_width(&text, attributes);
            assert!((width - semibold).abs() < 0.01, "{attributes}: {width}");
        }
    }

    #[test]
    fn control_characters_are_dropped_so_that_the_text_stays_drawable() {
        let text = text();
        let plain = text.width("ab", SIZE, FontWeight::SemiBold).unwrap();
        let polluted = text
            .width("a\u{1}\u{0}b\u{FFFE}", SIZE, FontWeight::SemiBold)
            .unwrap();
        assert!(
            (plain - polluted).abs() < f32::EPSILON,
            "{plain} {polluted}"
        );
        assert!(is_xml_char('\n') && is_xml_char('é') && !is_xml_char('\u{1}'));
    }

    #[test]
    fn markup_in_text_is_escaped() {
        assert!(
            text()
                .width("<b>&\"'</b>", SIZE, FontWeight::SemiBold)
                .unwrap()
                > 0.0
        );
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
            weight: FontWeight::SemiBold,
        };
        text.draw(&mut canvas, &[line]).unwrap();
        assert!(canvas.pixels().any(|pixel| pixel.0[0] > 128));
    }
}
