//! Arc and ring gauges, and the page zone: what is drawn where, on a
//! 200 × 100 px zone.

use std::io::Cursor;

use hubergamof_module_api::{GaugeShape, GaugeSteps, PageZone, ZoneAccent};
use image::ImageFormat;

use super::*;

const ZONE: ImageSize = ImageSize {
    width: 200,
    height: 100,
};

/// A red square, as the glyph of a gauge.
const RED_GLYPH: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#ff0000"/></svg>"##;

const TRACK: Rgba<u8> = GAUGE_TRACK_COLOR;
/// Rows other than the current one, in the page zone.
const OTHER_ROW: Rgba<u8> = Rgba([0x23, 0x2a, 0x40, u8::MAX]);
/// The indigo of the navigation module, which the page zones are drawn in.
const INDIGO: ZoneAccent = ZoneAccent::new(Rgb(0xa5, 0xb4, 0xfc), Rgb(0x63, 0x66, 0xf1));
const ORANGE: Rgb = Rgb(0xff, 0x80, 0x00);
const ORANGE_PIXEL: Rgba<u8> = Rgba([0xff, 0x80, 0x00, u8::MAX]);

fn renderer() -> Renderer {
    Renderer::new().unwrap()
}

fn render_gauge(gauge: &GaugeContent) -> Arc<Rendered> {
    renderer()
        .render_gauge(gauge, ZONE, DEFAULT_BACKGROUND)
        .unwrap()
}

fn render_page(page: &PageZone) -> Arc<Rendered> {
    renderer()
        .render_page_zone(&page.clone().with_accent(INDIGO), ZONE, DEFAULT_BACKGROUND)
        .unwrap()
}

fn arc(level: f32) -> GaugeContent {
    GaugeContent::new("Haut-parleurs", "25 %", GaugeLevel::from_ratio(level))
        .with_shape(GaugeShape::Arc)
        .with_accent(ZoneAccent::solid(ORANGE))
}

fn ring(level: f32) -> GaugeContent {
    GaugeContent::new("Travail", "18:42", GaugeLevel::from_ratio(level))
        .with_shape(GaugeShape::Ring { steps: None })
        .with_accent(ZoneAccent::solid(ORANGE))
}

/// Whether `pixel` is `expected`, give or take the rounding of blending.
fn is(pixel: Rgba<u8>, expected: Rgba<u8>) -> bool {
    pixel
        .0
        .iter()
        .zip(expected.0)
        .all(|(got, want)| got.abs_diff(want) <= 2)
}

fn pixel(rendered: &Rendered, x: u32, y: u32) -> Rgba<u8> {
    *rendered.image().get_pixel(x, y)
}

/// Pixels of `rows` and `columns` for which `test` holds.
fn count(
    rendered: &Rendered,
    columns: std::ops::Range<u32>,
    rows: std::ops::Range<u32>,
    test: impl Fn(Rgba<u8>) -> bool,
) -> usize {
    rows.flat_map(|y| columns.clone().map(move |x| (x, y)))
        .filter(|&(x, y)| test(*rendered.image().get_pixel(x, y)))
        .count()
}

const fn is_white(pixel: Rgba<u8>) -> bool {
    pixel.0[0] > 200 && pixel.0[1] > 200 && pixel.0[2] > 200
}

const fn is_red(pixel: Rgba<u8>) -> bool {
    pixel.0[0] > 200 && pixel.0[1] < 60 && pixel.0[2] < 60
}

#[test]
fn arcs_are_filled_from_the_left_up_to_the_level() {
    // The arc rests on row 64 from column 56 to 144, and peaks at (100, 20).
    let (left_end, top, right_end) = ((56, 64), (100, 20), (144, 64));
    let empty = render_gauge(&arc(0.0));
    for (x, y) in [left_end, top, right_end] {
        assert!(is(pixel(&empty, x, y), TRACK), "({x}, {y}) is track");
    }
    let half = render_gauge(&arc(0.5));
    assert!(is(pixel(&half, left_end.0, left_end.1), ORANGE_PIXEL));
    assert!(
        pixel(&half, top.0, top.1).0[0] > 200,
        "the end of the fill is on the top (and its dot is white)"
    );
    assert!(is(pixel(&half, right_end.0, right_end.1), TRACK));
    let full = render_gauge(&arc(1.0));
    assert!(is(pixel(&full, top.0, top.1), ORANGE_PIXEL));
}

#[test]
fn dimmed_arcs_are_greyed_out() {
    let dimmed = render_gauge(&arc(1.0).with_dimmed(true));
    assert!(is(pixel(&dimmed, 56, 64), GAUGE_DIMMED_FILL_COLOR));
    let lit = (0..100).any(|y| (0..200).any(|x| is_white(pixel(&dimmed, x, y))));
    assert!(!lit, "the value of a dimmed gauge is grey");
}

#[test]
fn arcs_show_the_value_inside_and_the_title_below() {
    let rendered = render_gauge(&arc(0.5));
    assert!(count(&rendered, 70..130, 38..70, is_white) > 20, "value");
    let title = |pixel: Rgba<u8>| pixel.0[0] > 100 && pixel.0[0] == pixel.0[2];
    assert!(count(&rendered, 20..180, 78..96, title) > 20, "title");
    assert!(rendered.image().pixels().all(|pixel| pixel.0[3] == u8::MAX));
}

#[test]
fn a_value_too_wide_shrinks_instead_of_being_cut() {
    let narrow = GaugeContent::new("Sortie", "100 %", GaugeLevel::from_ratio(0.5))
        .with_shape(GaugeShape::Arc);
    let rendered = render_gauge(&narrow);
    // The whole value stays inside the half circle.
    assert_eq!(count(&rendered, 0..60, 38..70, is_white), 0);
    assert_eq!(count(&rendered, 140..200, 38..70, is_white), 0);
    assert!(count(&rendered, 60..140, 38..70, is_white) > 30);
}

#[test]
fn the_glyph_of_an_arc_stands_before_its_title() {
    let glyph = ImageData::new(RED_GLYPH.as_bytes());
    let with = render_gauge(&arc(0.5).with_glyph(glyph));
    let without = render_gauge(&arc(0.5));
    assert!(count(&with, 0..200, 74..100, is_red) > 20);
    assert_eq!(count(&without, 0..200, 0..100, is_red), 0);
}

#[test]
fn rings_are_filled_clockwise_from_the_top() {
    // Centre (46, 50), radius 30: top (46, 20), right (76, 50), bottom (46, 80)
    // and left (16, 50).
    let (top, right, bottom, left) = ((46, 20), (76, 50), (46, 80), (16, 50));
    let quarter = render_gauge(&ring(0.25));
    assert!(is(pixel(&quarter, top.0, top.1), ORANGE_PIXEL));
    assert!(is(pixel(&quarter, right.0, right.1), ORANGE_PIXEL));
    assert!(is(pixel(&quarter, bottom.0, bottom.1), TRACK));
    assert!(is(pixel(&quarter, left.0, left.1), TRACK));
    let three_quarters = render_gauge(&ring(0.75));
    assert!(is(pixel(&three_quarters, bottom.0, bottom.1), ORANGE_PIXEL));
    assert!(is(pixel(&three_quarters, left.0, left.1), ORANGE_PIXEL));
    let full = render_gauge(&ring(1.0));
    assert!(is(pixel(&full, left.0, left.1), ORANGE_PIXEL));
    let empty = render_gauge(&ring(0.0));
    assert!(is(pixel(&empty, top.0, top.1), TRACK));
}

#[test]
fn rings_draw_the_glyph_inside_and_the_texts_beside() {
    let glyph = ImageData::new(RED_GLYPH.as_bytes());
    let rendered = render_gauge(&ring(0.5).with_glyph(glyph));
    assert!(is_red(pixel(&rendered, 46, 50)), "glyph in the middle");
    assert!(count(&rendered, 92..192, 36..70, is_white) > 30, "value");
    let label = |pixel: Rgba<u8>| pixel.0[0] > 200 && (100..160).contains(&pixel.0[1]);
    assert!(count(&rendered, 92..192, 14..32, label) > 10, "label");
}

#[test]
fn rings_draw_one_dot_per_step() {
    let rendered = render_gauge(&ring(0.5).with_steps(GaugeSteps::new(2, 4)));
    // Dots at 118, 134, 150 and 166 on row 80.
    assert!(is(pixel(&rendered, 118, 80), ORANGE_PIXEL), "finished");
    assert!(is(pixel(&rendered, 134, 80), ORANGE_PIXEL), "current core");
    assert!(
        is(pixel(&rendered, 134 + 4, 80), ORANGE_PIXEL)
            || is(pixel(&rendered, 134 - 4, 80), ORANGE_PIXEL),
        "current outline"
    );
    assert!(is(pixel(&rendered, 150, 80), TRACK), "to come");
    assert!(is(pixel(&rendered, 166, 80), TRACK), "to come");
    let without = render_gauge(&ring(0.5));
    assert_eq!(
        count(&without, 100..190, 74..87, |p| p != DEFAULT_BACKGROUND),
        0
    );
}

#[test]
fn dimmed_rings_are_greyed_out() {
    let rendered = render_gauge(
        &ring(0.5)
            .with_dimmed(true)
            .with_steps(GaugeSteps::new(1, 4)),
    );
    assert!(is(pixel(&rendered, 46, 20), GAUGE_DIMMED_FILL_COLOR));
    assert!(is(pixel(&rendered, 118, 80), GAUGE_DIMMED_FILL_COLOR));
    assert_eq!(count(&rendered, 0..200, 0..100, is_white), 0);
}

#[test]
fn many_steps_are_squeezed_into_the_area_beside_the_ring() {
    let rendered = render_gauge(&ring(0.5).with_steps(GaugeSteps::new(12, 12)));
    // Twelve dots in the 100 px area: 8.33 px apart, from 96 to 188.
    assert_eq!(
        count(&rendered, 82..88, 74..87, |p| p != DEFAULT_BACKGROUND),
        0
    );
    assert!(count(&rendered, 92..196, 74..87, |p| p != DEFAULT_BACKGROUND) > 100);
}

#[test]
fn identical_shaped_gauges_hit_the_cache() {
    let renderer = renderer();
    let first = renderer
        .render_gauge(&ring(0.5), ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    let second = renderer
        .render_gauge(&ring(0.5), ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let arc = renderer
        .render_gauge(&arc(0.5), ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &arc), "the shape is part of the key");
}

const INDIGO_PIXEL_MIN_BLUE: u8 = 0xe0;

/// Pixels of the hint of the way back that are lit (light blue arrow and
/// outline).
fn hint_lit(rendered: &Rendered) -> usize {
    count(rendered, 165..190, 7..32, |pixel| {
        pixel.0[2] >= INDIGO_PIXEL_MIN_BLUE
    })
}

#[test]
fn the_hint_of_the_way_back_lights_up_inside_a_module() {
    let home = render_page(&PageZone::new("Accueil"));
    let inside = render_page(&PageZone::new("Conteneurs").with_parent("Accueil"));
    assert!(hint_lit(&inside) > 15, "arrow and outline are lit");
    assert_eq!(hint_lit(&home), 0, "the hint is faint on the home page");
    assert!(
        count(&home, 165..190, 7..32, |pixel| pixel != DEFAULT_BACKGROUND) > 10,
        "but it is there"
    );
}

#[test]
fn the_title_and_the_parent_page_are_drawn_centred() {
    let page = render_page(&PageZone::new("Conteneurs").with_parent("Accueil"));
    assert!(count(&page, 20..180, 34..62, is_white) > 50, "title");
    let parent = |pixel: Rgba<u8>| pixel.0[2] > 200 && pixel.0[0] > 120 && pixel.0[0] < 200;
    assert!(count(&page, 60..140, 8..30, parent) > 10, "parent page");
    let home = render_page(&PageZone::new("Accueil"));
    assert_eq!(count(&home, 40..160, 4..28, |p| p != DEFAULT_BACKGROUND), 0);
}

#[test]
fn rows_are_one_segment_each_with_the_current_one_lit() {
    // Three segments over columns 16 to 184 on row 82: 16-69, 73-127, 131-184.
    let rendered = render_page(&PageZone::new("Accueil").with_rows(1, 3));
    assert!(is(pixel(&rendered, 40, 82), OTHER_ROW));
    assert!(is(pixel(&rendered, 160, 82), OTHER_ROW));
    let current = pixel(&rendered, 100, 82);
    assert!(current.0[2] > 0xe0 && current.0[0] > 0x60, "{current:?}");
    let caption = |pixel: Rgba<u8>| pixel.0[0] > 0x40;
    assert!(
        count(&rendered, 60..140, 88..100, caption) > 10,
        "ligne 2 sur 3"
    );
}

#[test]
fn a_single_row_shows_no_segments() {
    let rendered = render_page(&PageZone::new("Conteneurs").with_rows(0, 1));
    assert_eq!(
        count(&rendered, 0..200, 78..100, |p| p != DEFAULT_BACKGROUND),
        0
    );
}

#[test]
fn many_rows_share_one_track_with_a_marker() {
    // Twelve rows, the seventh shown: the marker is 14 px wide from column 100.
    let rendered = render_page(&PageZone::new("Accueil").with_rows(6, 12));
    assert!(is(pixel(&rendered, 40, 82), OTHER_ROW));
    assert!(pixel(&rendered, 107, 82).0[2] > 0xe0);
    assert!(is(pixel(&rendered, 130, 82), OTHER_ROW));
}

#[test]
fn identical_page_zones_hit_the_cache() {
    let renderer = renderer();
    let page = PageZone::new("Accueil").with_rows(0, 3);
    let first = renderer
        .render_page_zone(&page, ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    let second = renderer
        .render_page_zone(&page.clone(), ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn rows_beyond_the_last_one_show_the_last_one() {
    let page = PageZone::new("Accueil").with_rows(5, 3);
    let rendered = render_page(&page);
    // Third of three segments lit, and the caption agrees.
    assert!(pixel(&rendered, 160, 82).0[2] > 0xe0);
    assert!(is(pixel(&rendered, 40, 82), OTHER_ROW));
    assert_eq!(
        rendered.image().as_ref(),
        render_page(&PageZone::new("Accueil").with_rows(2, 3))
            .image()
            .as_ref()
    );
}

/// Columns of row 82 (the middle of the row segments) that are not background.
fn segment_columns(rendered: &Rendered) -> Vec<u32> {
    (0..200)
        .filter(|&x| pixel(rendered, x, 82) != DEFAULT_BACKGROUND)
        .collect()
}

/// Number of runs of columns, a run ending at a gap wider than `max_gap`.
fn runs_with_gap(columns: &[u32], max_gap: u32) -> usize {
    columns
        .iter()
        .enumerate()
        .filter(|&(index, &column)| index == 0 || columns[index - 1] + max_gap < column)
        .count()
}

/// Number of runs of consecutive columns.
fn runs(columns: &[u32]) -> usize {
    runs_with_gap(columns, 1)
}

#[test]
fn up_to_seven_rows_have_a_segment_each_and_more_share_a_track() {
    let rows = |row, count| {
        segment_columns(&render_page(
            &PageZone::new("Accueil").with_rows(row, count),
        ))
    };
    assert_eq!(runs(&rows(0, 2)), 2);
    assert_eq!(runs(&rows(3, 7)), 7);
    assert_eq!(runs(&rows(3, 8)), 1, "a continuous track");
    assert_eq!(runs(&rows(0, 1)), 0);
}

#[test]
fn a_glyph_that_cannot_be_drawn_is_left_out() {
    let broken = ImageData::new(&b"not an image"[..]);
    let with = render_gauge(&ring(0.5).with_glyph(broken.clone()));
    assert_eq!(
        with.image().as_ref(),
        render_gauge(&ring(0.5)).image().as_ref()
    );
    let arc = render_gauge(&arc(0.5).with_glyph(broken));
    assert!(
        count(&arc, 70..130, 38..70, is_white) > 20,
        "the value is drawn"
    );
}

#[test]
fn a_bitmap_glyph_is_scaled_into_place() {
    let mut png = Cursor::new(Vec::new());
    RgbaImage::from_pixel(20, 10, Rgba([255, 0, 0, 255]))
        .write_to(&mut png, ImageFormat::Png)
        .unwrap();
    let rendered = render_gauge(&ring(0.5).with_glyph(ImageData::new(png.into_inner())));
    assert!(is_red(pixel(&rendered, 46, 50)));
    // Contained: twice as wide as high, centred.
    assert!(is_red(pixel(&rendered, 40, 50)) && is_red(pixel(&rendered, 52, 50)));
    assert!(!is_red(pixel(&rendered, 46, 40)));
}

#[test]
fn every_shape_copes_with_unusual_zone_sizes() {
    let renderer = renderer();
    let glyph = ImageData::new(RED_GLYPH.as_bytes());
    let steps = GaugeSteps::new(3, 12);
    let long = "Une sortie audio au nom très très long, vraiment trop long";
    let gauges = [
        GaugeContent::new(long, long, GaugeLevel::from_ratio(0.5)),
        arc(0.5).with_glyph(glyph.clone()),
        GaugeContent::new(long, long, GaugeLevel::FULL)
            .with_shape(GaugeShape::Arc)
            .with_glyph(glyph.clone()),
        ring(0.5).with_glyph(glyph.clone()).with_steps(steps),
        GaugeContent::new(long, long, GaugeLevel::from_ratio(0.999))
            .with_shape(GaugeShape::Ring { steps: None })
            .with_steps(steps),
    ];
    let pages = [
        PageZone::new(long).with_parent(long).with_rows(3, 12),
        PageZone::new("").with_rows(0, 2),
        PageZone::new("Accueil"),
    ];
    for (width, height) in [(1, 1), (30, 20), (100, 200), (400, 200), (800, 100)] {
        let size = ImageSize { width, height };
        let mut images = Vec::new();
        for gauge in &gauges {
            images.push(
                renderer
                    .render_gauge(gauge, size, DEFAULT_BACKGROUND)
                    .unwrap(),
            );
        }
        for page in &pages {
            images.push(
                renderer
                    .render_page_zone(page, size, DEFAULT_BACKGROUND)
                    .unwrap(),
            );
        }
        for image in images {
            assert_eq!(image.image().dimensions(), (width, height));
            assert!(image.image().pixels().all(|pixel| pixel.0[3] == u8::MAX));
        }
    }
}

#[test]
fn rings_switch_from_an_arc_to_a_full_circle_at_the_end() {
    // Just under full, over half: the long way round; full: a whole circle.
    let almost = render_gauge(&ring(0.999));
    let full = render_gauge(&ring(1.0));
    for rendered in [&almost, &full] {
        assert!(is(pixel(rendered, 16, 50), ORANGE_PIXEL));
        assert!(is(pixel(rendered, 46, 80), ORANGE_PIXEL));
    }
    let over_half = render_gauge(&ring(0.51));
    assert!(is(pixel(&over_half, 46, 80), ORANGE_PIXEL));
    let half = render_gauge(&ring(0.5));
    assert!(is(pixel(&half, 46, 80), ORANGE_PIXEL));
    assert!(is(pixel(&half, 16, 50), TRACK));
}

#[test]
fn a_single_step_and_the_dot_pitch_boundary_are_drawn() {
    // 6 dots fit at the 16 px pitch in the 100 px area, 7 are squeezed.
    for total in [1_u32, 6, 7] {
        let rendered = render_gauge(&ring(0.5).with_steps(GaugeSteps::new(1, total)));
        let dots = segment_dots(&rendered);
        assert_eq!(dots, total, "{total} dots");
    }
}

/// Dots on row 80 to the right of the ring: runs of non-background pixels.
fn segment_dots(rendered: &Rendered) -> u32 {
    let columns: Vec<u32> = (84..200)
        .filter(|&x| pixel(rendered, x, 80) != DEFAULT_BACKGROUND)
        .collect();
    // The outline and the core of the current dot are one dot.
    u32::try_from(runs_with_gap(&columns, 3)).unwrap()
}

#[test]
fn changing_any_field_of_a_gauge_changes_its_image() {
    let renderer = renderer();
    let base = ring(0.5)
        .with_glyph(ImageData::new(RED_GLYPH.as_bytes()))
        .with_steps(GaugeSteps::new(2, 4));
    let render = |gauge: &GaugeContent, size: ImageSize, background| {
        renderer.render_gauge(gauge, size, background).unwrap()
    };
    let reference = render(&base, ZONE, DEFAULT_BACKGROUND);
    let variants = [
        base.clone().with_dimmed(true),
        base.clone().with_steps(GaugeSteps::new(3, 4)),
        base.clone().with_accent(ZoneAccent::solid(Rgb(0, 0xff, 0))),
        base.clone().with_glyph(ImageData::new(
            RED_GLYPH.replace("ff0000", "00ff00").into_bytes(),
        )),
        base.clone().with_shape(GaugeShape::Arc),
    ];
    for variant in &variants {
        let other = render(variant, ZONE, DEFAULT_BACKGROUND);
        assert!(!Arc::ptr_eq(&reference, &other), "{variant:?}");
    }
    let bigger = ImageSize {
        width: 400,
        height: 200,
    };
    assert!(!Arc::ptr_eq(
        &reference,
        &render(&base, bigger, DEFAULT_BACKGROUND)
    ));
    assert!(!Arc::ptr_eq(
        &reference,
        &render(&base, ZONE, Rgba([9, 9, 9, 255]))
    ));
}
