//! Arc and ring gauges, and the page zone: what is drawn where, on a
//! 200 × 100 px zone.

use hubergamof_module_api::{GaugeAccent, GaugeShape, GaugeSteps, PageZone};

use super::*;

const ZONE: ImageSize = ImageSize {
    width: 200,
    height: 100,
};

/// A red square, as the glyph of a gauge.
const RED_GLYPH: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#ff0000"/></svg>"##;

const TRACK: Rgba<u8> = GAUGE_TRACK_COLOR;
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
        .render_page_zone(page, ZONE, DEFAULT_BACKGROUND)
        .unwrap()
}

fn arc(level: f32) -> GaugeContent {
    GaugeContent::new("Haut-parleurs", "25 %", GaugeLevel::from_ratio(level))
        .with_shape(GaugeShape::Arc)
        .with_accent(GaugeAccent::solid(ORANGE))
}

fn ring(level: f32) -> GaugeContent {
    GaugeContent::new("Travail", "18:42", GaugeLevel::from_ratio(level))
        .with_shape(GaugeShape::Ring)
        .with_accent(GaugeAccent::solid(ORANGE))
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
    assert!(is(pixel(&rendered, 40, 82), Rgba([0x23, 0x2a, 0x40, 255])));
    assert!(is(pixel(&rendered, 160, 82), Rgba([0x23, 0x2a, 0x40, 255])));
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
    assert!(is(pixel(&rendered, 40, 82), Rgba([0x23, 0x2a, 0x40, 255])));
    assert!(pixel(&rendered, 107, 82).0[2] > 0xe0);
    assert!(is(pixel(&rendered, 130, 82), Rgba([0x23, 0x2a, 0x40, 255])));
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
