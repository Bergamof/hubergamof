use std::io::Cursor;

use hubergamof_module_api::{CalendarLeaf, ClockFace, GaugeContent, GaugeLevel};

use super::clock::{CLOCK_ACCENT_COLOR, CLOCK_DEEP_ACCENT_COLOR};

use super::*;

const KEY: ImageSize = ImageSize {
    width: 120,
    height: 120,
};

const ZONE: ImageSize = ImageSize {
    width: 200,
    height: 100,
};

const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);

fn png(width: u32, height: u32, color: Rgba<u8>) -> ImageSource {
    let mut bytes = Cursor::new(Vec::new());
    RgbaImage::from_pixel(width, height, color)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    ImageSource::Bytes(bytes.into_inner().into())
}

fn jpeg(width: u32, height: u32) -> ImageSource {
    let mut bytes = Cursor::new(Vec::new());
    image::RgbImage::from_pixel(width, height, image::Rgb([0, 0, 255]))
        .write_to(&mut bytes, ImageFormat::Jpeg)
        .unwrap();
    ImageSource::Bytes(bytes.into_inner().into())
}

fn svg(body: &str) -> ImageSource {
    ImageSource::Bytes(body.as_bytes().into())
}

fn renderer() -> Renderer {
    Renderer::new().unwrap()
}

#[test]
fn key_without_source_is_background_only() {
    let rendered = renderer()
        .render_key(&KeyRequest::new(KEY, None, None))
        .unwrap();
    let image = rendered.image();
    assert_eq!(image.dimensions(), (120, 120));
    assert!(image.pixels().all(|pixel| *pixel == DEFAULT_BACKGROUND));
}

#[test]
fn wide_image_is_contained_and_centred() {
    let source = png(200, 100, RED);
    let rendered = renderer()
        .render_key(&KeyRequest::new(KEY, Some(&source), None))
        .unwrap();
    let image = rendered.image();
    assert_eq!(image.dimensions(), (120, 120));
    // 200×100 → 120×60, centred vertically: rows 30..90 are red.
    assert_eq!(*image.get_pixel(60, 10), DEFAULT_BACKGROUND);
    assert_eq!(*image.get_pixel(60, 60), RED);
    assert_eq!(*image.get_pixel(0, 31), RED);
    assert_eq!(*image.get_pixel(60, 110), DEFAULT_BACKGROUND);
}

#[test]
fn small_image_is_scaled_up() {
    let source = png(10, 10, RED);
    let rendered = renderer()
        .render_key(&KeyRequest::new(KEY, Some(&source), None))
        .unwrap();
    assert_eq!(*rendered.image().get_pixel(5, 5), RED);
    assert_eq!(*rendered.image().get_pixel(115, 115), RED);
}

#[test]
fn custom_background_is_used() {
    let background = Rgba([10, 20, 30, 255]);
    let request = KeyRequest {
        background,
        ..KeyRequest::new(KEY, None, None)
    };
    let rendered = renderer().render_key(&request).unwrap();
    assert_eq!(*rendered.image().get_pixel(0, 0), background);
}

#[test]
fn jpeg_sources_are_decoded() {
    let source = jpeg(60, 60);
    let rendered = renderer()
        .render_key(&KeyRequest::new(KEY, Some(&source), None))
        .unwrap();
    let pixel = rendered.image().get_pixel(60, 60);
    assert!(pixel.0[2] > 200 && pixel.0[0] < 50);
}

#[test]
fn label_changes_the_image() {
    let source = png(120, 120, RED);
    let subject = renderer();
    let plain = subject
        .render_key(&KeyRequest::new(KEY, Some(&source), None))
        .unwrap();
    let labelled = subject
        .render_key(&KeyRequest::new(KEY, Some(&source), Some("Volume")))
        .unwrap();
    assert_eq!(labelled.image().dimensions(), (120, 120));
    assert_ne!(plain.image(), labelled.image());
    // The image sits above the label, whole: its last row is not covered.
    let image = labelled.image();
    assert_eq!(*image.get_pixel(60, 10), RED);
    assert_eq!(*image.get_pixel(60, 85), RED);
    assert_ne!(*image.get_pixel(60, 119), RED);
    assert!(image.pixels().all(|pixel| pixel.0[3] == 255));
}

#[test]
fn a_long_label_stays_on_one_line_below_the_image() {
    let source = png(120, 120, RED);
    let rendered = renderer()
        .render_key(&KeyRequest::new(
            KEY,
            Some(&source),
            Some("Un libellé beaucoup trop long pour tenir sur une touche"),
        ))
        .unwrap();
    let image = rendered.image();
    // Text is white: none above the label strip, some inside it.
    let is_text = |pixel: &Rgba<u8>| pixel.0[1] > 200;
    let strip_top = 90;
    assert!(
        image
            .enumerate_pixels()
            .filter(|(_, y, _)| *y < strip_top)
            .all(|(_, _, pixel)| !is_text(pixel))
    );
    assert!(
        image
            .enumerate_pixels()
            .any(|(_, y, pixel)| y >= strip_top && is_text(pixel))
    );
}

#[test]
fn svg_sources_are_rendered_at_the_target_size() {
    let source = svg(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"><rect width="10" height="20" fill="#ff0000"/></svg>"##,
    );
    let rendered = renderer()
        .render_key(&KeyRequest::new(KEY, Some(&source), None))
        .unwrap();
    let image = rendered.image();
    // 10×20 → 60×120 centred horizontally.
    assert_eq!(*image.get_pixel(60, 60), RED);
    assert_eq!(*image.get_pixel(10, 60), DEFAULT_BACKGROUND);
}

#[test]
fn svg_external_images_are_not_loaded() {
    let source = svg(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10"><image width="10" height="10" xlink:href="/etc/hostname"/></svg>"#,
    );
    let rendered = renderer()
        .render_key(&KeyRequest::new(KEY, Some(&source), None))
        .unwrap();
    assert!(
        rendered
            .image()
            .pixels()
            .all(|pixel| *pixel == DEFAULT_BACKGROUND)
    );
}

#[test]
fn oversized_sources_are_rejected() {
    let source = ImageSource::Bytes(vec![0; MAX_SOURCE_BYTES + 1].into());
    assert!(matches!(
        renderer().render_key(&KeyRequest::new(KEY, Some(&source), None)),
        Err(RenderError::SourceTooLarge { .. })
    ));
}

#[test]
fn oversized_files_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.png");
    std::fs::write(&path, vec![0; MAX_SOURCE_BYTES + 1]).unwrap();
    let source = ImageSource::File(path);
    assert!(matches!(
        renderer().render_key(&KeyRequest::new(KEY, Some(&source), None)),
        Err(RenderError::SourceTooLarge { .. })
    ));
}

#[test]
fn images_with_huge_dimensions_are_rejected_before_decoding() {
    let source = png(MAX_SOURCE_DIMENSION + 1, 1, RED);
    assert!(matches!(
        renderer().render_key(&KeyRequest::new(KEY, Some(&source), None)),
        Err(RenderError::Decode(_))
    ));
}

#[test]
fn unknown_formats_are_rejected() {
    let source = ImageSource::Bytes(b"GIF89a....".as_slice().into());
    assert!(matches!(
        renderer().render_key(&KeyRequest::new(KEY, Some(&source), None)),
        Err(RenderError::UnsupportedFormat)
    ));
}

#[test]
fn file_sources_are_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("red.png");
    let ImageSource::Bytes(bytes) = png(4, 4, RED) else {
        unreachable!()
    };
    std::fs::write(&path, &bytes).unwrap();
    let source = ImageSource::File(path);
    let rendered = renderer()
        .render_key(&KeyRequest::new(KEY, Some(&source), None))
        .unwrap();
    assert_eq!(*rendered.image().get_pixel(60, 60), RED);
}

#[test]
fn empty_target_size_is_rejected() {
    let empty = ImageSize {
        width: 0,
        height: 10,
    };
    assert!(matches!(
        renderer().render_key(&KeyRequest::new(empty, None, None)),
        Err(RenderError::InvalidSize)
    ));
}

#[test]
fn identical_requests_hit_the_cache() {
    let subject = renderer();
    let first = subject
        .render_key(&KeyRequest::new(KEY, None, Some("A")))
        .unwrap();
    let second = subject
        .render_key(&KeyRequest::new(KEY, None, Some("A")))
        .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let other = subject
        .render_key(&KeyRequest::new(KEY, None, Some("B")))
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &other));
    assert_eq!(subject.cached_len(), 2);
}

#[test]
fn cache_is_bounded() {
    let two = NonZeroUsize::new(2).unwrap();
    let subject = Renderer::with_cache_capacity(two, two).unwrap();
    let first = subject
        .render_key(&KeyRequest::new(KEY, None, Some("1")))
        .unwrap();
    for label in ["2", "3"] {
        subject
            .render_key(&KeyRequest::new(KEY, None, Some(label)))
            .unwrap();
    }
    assert_eq!(subject.cached_len(), 2);
    let again = subject
        .render_key(&KeyRequest::new(KEY, None, Some("1")))
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &again));
}

#[test]
fn a_countdown_does_not_push_icons_out_of_the_cache() {
    let subject = renderer();
    let icon = png(16, 16, RED);
    let render_icon = || {
        subject
            .render_key(&KeyRequest::new(KEY, Some(&icon), Some("Audio")))
            .unwrap()
    };
    // Shown, then shown again after a page visit.
    let first = render_icon();
    assert!(Arc::ptr_eq(&first, &render_icon()));
    // 300 seconds of a pomodoro: a new key and a new gauge every second.
    for second in 0..300_u32 {
        let label = format!("{:02}:{:02} Travail", second / 60, second % 60);
        subject
            .render_key(&KeyRequest::new(KEY, None, Some(&label)))
            .unwrap();
        let level = GaugeLevel::from_fraction(u64::from(second), 1500);
        subject
            .render_gauge(
                &GaugeContent::new("Travail · 1/4", label, level),
                ZONE,
                DEFAULT_BACKGROUND,
            )
            .unwrap();
    }
    assert!(Arc::ptr_eq(&first, &render_icon()), "the icon was evicted");
    // The countdown only ever occupies the probation segment.
    assert_eq!(subject.cached_len(), 1 + RENDER_PROBATION_CAPACITY.get());
}

#[test]
fn data_url_is_a_png_encoded_once() {
    let rendered = renderer()
        .render_key(&KeyRequest::new(KEY, None, Some("A")))
        .unwrap();
    let url = rendered.data_url().unwrap();
    assert!(url.starts_with("data:image/png;base64,iVBORw0KGgo"));
    // The PNG is kept and shared; the URL is made from it each time.
    let png = rendered.png().unwrap();
    assert!(Arc::ptr_eq(&png, &rendered.png().unwrap()));
    assert_eq!(
        url,
        format!("data:image/png;base64,{}", BASE64.encode(&*png))
    );
}

#[test]
fn zone_lines_are_rendered_centred() {
    let subject = renderer();
    let lines = vec!["Volume".to_owned(), "42 %".to_owned()];
    let rendered = subject
        .render_zone(&lines, ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    let image = rendered.image();
    assert_eq!(image.dimensions(), (200, 100));
    let lit_columns: Vec<u32> = (0..200)
        .filter(|x| (0..100).any(|y| image.get_pixel(*x, y).0[0] > 128))
        .collect();
    let first = *lit_columns.first().unwrap();
    let last = *lit_columns.last().unwrap();
    assert!(first > 10 && last < 190);
    assert!((i64::from(first) + i64::from(last) - 200).abs() < 12);
}

#[test]
fn image_kind_is_sniffed_from_content() {
    let ImageSource::Bytes(png_bytes) = png(1, 1, RED) else {
        unreachable!()
    };
    let ImageSource::Bytes(jpeg_bytes) = jpeg(1, 1) else {
        unreachable!()
    };
    assert_eq!(ImageKind::sniff(&png_bytes), Some(ImageKind::Png));
    assert_eq!(ImageKind::sniff(&jpeg_bytes), Some(ImageKind::Jpeg));
    assert_eq!(
        ImageKind::sniff(b"<?xml version=\"1.0\"?><svg/>"),
        Some(ImageKind::Svg)
    );
    assert_eq!(ImageKind::sniff(b"hello"), None);
}

#[test]
fn active_keys_are_drawn_without_a_frame() {
    let renderer = renderer();
    let active = renderer
        .render_key(&KeyRequest::new(KEY, None, None).in_state(KeyState::Active))
        .unwrap();
    let normal = renderer
        .render_key(&KeyRequest::new(KEY, None, None))
        .unwrap();
    assert_eq!(active.image(), normal.image());
    assert_eq!(*active.image().get_pixel(0, 60), DEFAULT_BACKGROUND);
}

#[test]
fn keys_needing_attention_get_an_orange_background_and_no_frame() {
    let request = KeyRequest::new(KEY, None, Some("Fini")).in_state(KeyState::Attention);
    let rendered = renderer().render_key(&request).unwrap();
    let image = rendered.image();
    assert_eq!(*image.get_pixel(2, 2), ATTENTION_BACKGROUND);
    assert_eq!(*image.get_pixel(60, 30), ATTENTION_BACKGROUND);
}

#[test]
fn feedback_marks_are_drawn_over_the_surface() {
    let renderer = renderer();
    let base = renderer
        .render_zone(&["12:00".into()], ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    let ok = renderer.with_feedback(&base, Feedback::Ok).unwrap();
    let centre = ok.image().get_pixel(100, 25).0;
    assert!(centre[1] > centre[0] && centre[1] > centre[2], "{centre:?}");
    let alert = renderer.with_feedback(&base, Feedback::Alert).unwrap();
    let tip = alert.image().get_pixel(82, 71).0;
    assert!(tip[0] > 150 && tip[1] < 100, "{tip:?}");
    assert_eq!(alert.image().dimensions(), (200, 100));
    assert!(alert.image().pixels().all(|pixel| pixel.0[3] == u8::MAX));
    // The base image is left untouched.
    assert_ne!(base.image().as_ref(), ok.image().as_ref());
}

#[test]
fn zone_images_fit_the_zone() {
    let rendered = renderer().render_image(ZONE, &png(10, 10, RED)).unwrap();
    assert_eq!(rendered.image().dimensions(), (200, 100));
    assert_eq!(*rendered.image().get_pixel(100, 50), RED);
    assert_eq!(*rendered.image().get_pixel(10, 50), DEFAULT_BACKGROUND);
}

fn gauge(level: f32) -> GaugeContent {
    GaugeContent::new("Volume", "25 %", GaugeLevel::from_ratio(level))
}

/// Row crossing the middle of the gauge bar on a [`ZONE`].
const GAUGE_BAR_ROW: u32 = 84;

/// Columns of `row` painted with `color`.
fn columns_with(image: &RgbaImage, row: u32, color: Rgba<u8>) -> usize {
    (0..image.width())
        .filter(|x| *image.get_pixel(*x, row) == color)
        .count()
}

/// Columns of [`GAUGE_BAR_ROW`] painted with `color`.
fn bar_columns(image: &RgbaImage, color: Rgba<u8>) -> usize {
    columns_with(image, GAUGE_BAR_ROW, color)
}

#[test]
fn gauge_bars_are_filled_in_proportion_to_the_level() {
    let renderer = renderer();
    let quarter = renderer
        .render_gauge(&gauge(0.25), ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    let image = quarter.image();
    assert_eq!(image.dimensions(), (200, 100));
    let filled = bar_columns(image, GAUGE_FILL_COLOR);
    let track = bar_columns(image, GAUGE_TRACK_COLOR);
    // 4 % margins: a 184 px track, a quarter of it filled from the left.
    assert_eq!((filled, track), (46, 138));
    assert_eq!(*image.get_pixel(8, GAUGE_BAR_ROW), GAUGE_FILL_COLOR);
    assert_eq!(*image.get_pixel(7, GAUGE_BAR_ROW), DEFAULT_BACKGROUND);

    let empty = renderer
        .render_gauge(&gauge(0.0), ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    assert_eq!(bar_columns(empty.image(), GAUGE_FILL_COLOR), 0);
    let full = renderer
        .render_gauge(&gauge(1.0), ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    assert_eq!(bar_columns(full.image(), GAUGE_FILL_COLOR), 184);
}

#[test]
fn gauge_text_is_drawn_above_the_bar() {
    let rendered = renderer()
        .render_gauge(&gauge(0.5), ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    let image = rendered.image();
    let lit_rows = |rows: std::ops::Range<u32>| {
        rows.filter(|y| (0..200).any(|x| image.get_pixel(x, *y).0[0] > 200))
            .count()
    };
    // Title in the top quarter, value in the middle, nothing white in the bar.
    assert!(lit_rows(0..25) > 5);
    assert!(lit_rows(30..70) > 10);
    assert_eq!(lit_rows(80..90), 0);
    assert!(image.pixels().all(|pixel| pixel.0[3] == u8::MAX));
}

#[test]
fn dimmed_gauges_are_greyed_out() {
    let renderer = renderer();
    let normal = renderer
        .render_gauge(&gauge(0.5), ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    let dimmed = renderer
        .render_gauge(&gauge(0.5).with_dimmed(true), ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    assert_eq!(bar_columns(dimmed.image(), GAUGE_FILL_COLOR), 0);
    assert_eq!(bar_columns(dimmed.image(), GAUGE_DIMMED_FILL_COLOR), 92);
    let brightest = |image: &RgbaImage| image.pixels().map(|pixel| pixel.0[0]).max().unwrap();
    assert_eq!(brightest(normal.image()), u8::MAX);
    assert!(brightest(dimmed.image()) <= DIMMED_TEXT_COLOR.0[0]);
}

#[test]
fn identical_gauges_hit_the_cache() {
    let renderer = renderer();
    let first = renderer
        .render_gauge(&gauge(0.5), ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    let again = renderer
        .render_gauge(&gauge(0.5), ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    let other = renderer
        .render_gauge(&gauge(0.6), ZONE, DEFAULT_BACKGROUND)
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &other));
    assert_eq!(renderer.cached_len(), 2);
    assert!(
        renderer
            .render_gauge(
                &gauge(0.5),
                ImageSize {
                    width: 0,
                    height: 100
                },
                DEFAULT_BACKGROUND
            )
            .is_err()
    );
}

#[test]
fn only_documents_rooted_in_svg_are_sniffed_as_svg() {
    for svg in [
        "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        "\u{feff}  <?xml version=\"1.0\"?>\n<!-- logo -->\n<!DOCTYPE svg>\n<svg/>",
    ] {
        assert_eq!(
            ImageKind::sniff(svg.as_bytes()),
            Some(ImageKind::Svg),
            "{svg}"
        );
    }
    for text in [
        "# <svg>\nservices:\n  x:\n    image: alpine",
        "<html><svg/></html>",
        "<!-- <svg> never closed",
        "text <svg/>",
    ] {
        assert_eq!(ImageKind::sniff(text.as_bytes()), None, "{text}");
    }
}

#[test]
fn checked_images_are_drawn_but_not_remembered() {
    let renderer = renderer();
    let ImageSource::Bytes(bytes) = png(30, 10, Rgba([1, 2, 3, 255])) else {
        unreachable!("an in-memory source")
    };
    let size = ImageSize {
        width: 144,
        height: 144,
    };
    assert_eq!(renderer.check_image(&bytes, size).unwrap(), ImageKind::Png);
    assert_eq!(renderer.cached_len(), 0);
    let mut truncated = bytes.to_vec();
    truncated.truncate(truncated.len() / 2);
    assert!(renderer.check_image(&truncated, size).is_err());
    assert!(matches!(
        renderer.check_image(b"plain text", size),
        Err(RenderError::UnsupportedFormat)
    ));
}

fn clock_face(seconds: Option<u8>, with_leaf: bool) -> ClockFace {
    let mut face = ClockFace::new("14:05").with_caption("semaine 45");
    if let Some(seconds) = seconds {
        face = face.with_seconds(seconds);
    }
    if with_leaf {
        face = face.with_leaf(CalendarLeaf::new("mer.", 5, "nov."));
    }
    face
}

fn render_clock(face: &ClockFace) -> Arc<Rendered> {
    render_clock_on(face, ZONE)
}

fn render_clock_on(face: &ClockFace, size: ImageSize) -> Arc<Rendered> {
    renderer()
        .render_clock(face, size, DEFAULT_BACKGROUND)
        .unwrap()
}

/// Row crossing the middle of the seconds track on a zone of `size`.
fn clock_track_row(size: ImageSize) -> u32 {
    size.height * 85 / 100
}

fn count_pixels(image: &RgbaImage, color: Rgba<u8>) -> usize {
    image.pixels().filter(|pixel| **pixel == color).count()
}

/// Zones of the strips of the supported devices, and a tiny one.
const CLOCK_ZONES: [ImageSize; 3] = [
    ZONE,
    ImageSize {
        width: 120,
        height: 60,
    },
    ImageSize {
        width: 248,
        height: 58,
    },
];

#[test]
fn clock_leaf_is_drawn_only_with_a_date() {
    let with_leaf = render_clock(&clock_face(None, true));
    let without_leaf = render_clock(&clock_face(None, false));
    assert_eq!(with_leaf.image().dimensions(), (200, 100));
    // The middle of the band of the leaf, left of its weekday.
    assert_eq!(
        *with_leaf.image().get_pixel(16, 21),
        CLOCK_DEEP_ACCENT_COLOR
    );
    assert_eq!(
        count_pixels(without_leaf.image(), CLOCK_DEEP_ACCENT_COLOR),
        0
    );
    // The time is centred in the room the leaf leaves, or in the whole zone.
    let time_centre = |image: &RgbaImage| {
        let lit: Vec<u32> = (80..image.width())
            .filter(|x| (35..65).any(|y| image.get_pixel(*x, y).0[0] > 128))
            .collect();
        lit.iter().sum::<u32>() / u32::try_from(lit.len()).unwrap()
    };
    assert!(time_centre(with_leaf.image()) > 127);
    assert!(time_centre(without_leaf.image()) < 127);
}

#[test]
fn clock_leaf_texts_are_capitalised_by_the_renderer() {
    let lines = |weekday: &str, month: &str| {
        let face = ClockFace::new("14:05").with_leaf(CalendarLeaf::new(weekday, 5, month));
        let rendered = render_clock(&face);
        rendered.image().clone()
    };
    assert_eq!(lines("mer.", "nov."), lines("MER.", "NOV."));
}

#[test]
fn clock_seconds_track_fills_one_sixtieth_per_second() {
    let row = clock_track_row(ZONE);
    let filled = |seconds| {
        let rendered = render_clock(&clock_face(Some(seconds), true));
        columns_with(rendered.image(), row, CLOCK_ACCENT_COLOR)
    };
    // A 108 px track. The quarter marks cut a column out of the fill and the
    // rounded end is anti-aliased: a few pixels of tolerance.
    let mut previous = None;
    for seconds in [0_u8, 15, 29, 45, 59] {
        let count = filled(seconds);
        let expected = 108 * usize::from(seconds) / 60;
        // Each of the marks passed costs a column on both of its sides.
        let tolerance = 4 + 2 * usize::from(seconds / 15);
        assert!(
            count.abs_diff(expected) <= tolerance,
            "{seconds} s: {count} columns, expected about {expected}"
        );
        assert!(previous.is_none_or(|previous| count > previous));
        previous = Some(count);
    }
    assert_eq!(filled(0), 0);
}

#[test]
fn clock_seconds_beyond_the_minute_are_drawn_as_the_last_second() {
    let renderer = renderer();
    let render = |seconds| {
        renderer
            .render_clock(&clock_face(Some(seconds), true), ZONE, DEFAULT_BACKGROUND)
            .unwrap()
    };
    assert!(Arc::ptr_eq(&render(255), &render(59)));
}

#[test]
fn clock_without_seconds_has_no_track_and_no_superscript() {
    let row = clock_track_row(ZONE);
    let without = render_clock(&clock_face(None, true));
    assert_eq!(columns_with(without.image(), row, CLOCK_ACCENT_COLOR), 0);
    assert_eq!(columns_with(without.image(), row, GAUGE_TRACK_COLOR), 0);
    assert_eq!(count_pixels(without.image(), CLOCK_ACCENT_COLOR), 0);
    // With seconds, the superscript is drawn in the accent colour.
    let with = render_clock(&clock_face(Some(9), true));
    let superscript_pixels = (150..200)
        .flat_map(|x| (15..45).map(move |y| (x, y)))
        .filter(|(x, y)| {
            let pixel = with.image().get_pixel(*x, *y);
            pixel.0[1] > 150 && pixel.0[0] < 150
        })
        .count();
    assert!(superscript_pixels > 20, "{superscript_pixels}");
}

/// Whether the two outermost columns on each side of `image` are black.
fn edges_are_black(image: &RgbaImage) -> bool {
    let last = image.width() - 1;
    [0, 1, last - 1, last]
        .into_iter()
        .all(|x| (0..image.height()).all(|y| image.get_pixel(x, y).0[..3] == [0, 0, 0]))
}

#[test]
fn clock_widest_times_stay_inside_every_zone() {
    for size in CLOCK_ZONES {
        for time in ["00:00", "12:08", "20:08"] {
            for seconds in [0, 59] {
                let face = ClockFace::new(time)
                    .with_seconds(seconds)
                    .with_leaf(CalendarLeaf::new("mer.", 28, "sept."))
                    .with_caption("semaine 53");
                let rendered = render_clock_on(&face, size);
                assert!(
                    edges_are_black(rendered.image()),
                    "{time}:{seconds} on {size:?}"
                );
            }
        }
    }
}

#[test]
fn clock_texts_are_shortened_to_their_room() {
    let renderer = renderer();
    let face = ClockFace::new("23:59:59:59")
        .with_seconds(59)
        .with_leaf(CalendarLeaf::new("mercredi", 28, "septembre"))
        .with_caption("semaine 53 d'une année bien trop longue");
    let lines = renderer.clock_lines(&face, ZONE).unwrap();
    assert!(lines.iter().any(|line| line.text.ends_with('…')));
    for line in &lines {
        let width = renderer
            .text
            .width(&line.text, line.size, line.weight)
            .unwrap();
        // The widest room is the time area, 54 % of the zone.
        assert!(width <= 108.0, "{:?} is {width} px wide", line.text);
    }
}

#[test]
fn clock_faces_without_leaf_or_caption_are_drawn() {
    for size in CLOCK_ZONES {
        for (with_leaf, with_caption) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let mut face = ClockFace::new("14:05").with_seconds(30);
            if with_leaf {
                face = face.with_leaf(CalendarLeaf::new("mer.", 5, "nov."));
            }
            if with_caption {
                face = face.with_caption("semaine 45");
            }
            let rendered = render_clock_on(&face, size);
            assert_eq!(
                (rendered.image().width(), rendered.image().height()),
                (size.width, size.height)
            );
            assert!(edges_are_black(rendered.image()), "{face:?} on {size:?}");
        }
    }
}

#[test]
fn tiny_zones_are_drawn_without_panicking() {
    let tiny = ImageSize {
        width: 8,
        height: 4,
    };
    let rendered = render_clock_on(&clock_face(Some(59), true), tiny);
    assert_eq!(rendered.image().dimensions(), (8, 4));
}

#[test]
fn identical_clock_faces_hit_the_cache() {
    let renderer = renderer();
    let render = |seconds| {
        renderer
            .render_clock(&clock_face(Some(seconds), true), ZONE, DEFAULT_BACKGROUND)
            .unwrap()
    };
    let first = render(7);
    assert!(Arc::ptr_eq(&first, &render(7)));
    assert!(!Arc::ptr_eq(&first, &render(8)));
    assert_eq!(renderer.cached_len(), 2);
}

#[test]
fn clock_zones_of_no_size_are_refused() {
    let renderer = renderer();
    for (width, height) in [(0, 100), (200, 0)] {
        assert!(matches!(
            renderer.render_clock(
                &clock_face(None, true),
                ImageSize { width, height },
                DEFAULT_BACKGROUND
            ),
            Err(RenderError::InvalidSize)
        ));
    }
}
