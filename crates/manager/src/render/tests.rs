use std::io::Cursor;

use super::*;

const KEY: ImageSize = ImageSize {
    width: 120,
    height: 120,
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
    // 300 seconds of a pomodoro: a new key every second.
    for second in 0..300_u32 {
        let label = format!("{:02}:{:02} Travail", second / 60, second % 60);
        subject
            .render_key(&KeyRequest::new(KEY, None, Some(&label)))
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

/// `value` as a pixel coordinate (tests only draw within a few hundred).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn pixel_at(value: f32) -> u32 {
    value.max(0.0) as u32
}

#[test]
fn feedback_marks_are_drawn_over_the_surface() {
    let renderer = renderer();
    let base = renderer
        .render_key(&KeyRequest::new(KEY, None, Some("12:00")))
        .unwrap();
    // The marks fill 70 % of the smaller side, centred.
    let side = f32_from(KEY.width.min(KEY.height)) * FEEDBACK_MARK_RATIO;
    let at = |ratio_x: f32, ratio_y: f32| {
        let left = (f32_from(KEY.width) - side) / 2.0;
        let top = (f32_from(KEY.height) - side) / 2.0;
        (
            pixel_at(left + side * ratio_x),
            pixel_at(top + side * ratio_y),
        )
    };
    let ok = renderer.with_feedback(&base, Feedback::Ok).unwrap();
    let (x, y) = at(0.12, 0.5);
    let disc = ok.image().get_pixel(x, y).0;
    assert!(disc[1] > disc[0] && disc[1] > disc[2], "{disc:?}");
    let alert = renderer.with_feedback(&base, Feedback::Alert).unwrap();
    let (x, y) = at(0.2, 0.85);
    let triangle = alert.image().get_pixel(x, y).0;
    assert!(triangle[0] > 150 && triangle[1] < 100, "{triangle:?}");
    assert_eq!(alert.image().dimensions(), (KEY.width, KEY.height));
    assert!(alert.image().pixels().all(|pixel| pixel.0[3] == u8::MAX));
    // The base image is left untouched.
    assert_ne!(base.image().as_ref(), ok.image().as_ref());
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
