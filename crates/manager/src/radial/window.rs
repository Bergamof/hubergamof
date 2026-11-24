//! What the windows of the radial menu share: what they draw, how it gets
//! into a buffer, and how long they may stay.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hubergamof_module_api::RadialItem;
use image::RgbaImage;

use super::controller::{View, WindowEvent};
use super::geometry::{Layout, Point};
use super::paint::Palette;

/// How long the menu takes to open when it is animated.
pub(super) const OPENING_DURATION: Duration = Duration::from_millis(140);

/// Bytes of a pixel of the buffers (`Argb8888`, and the 32-bit images of X11).
pub(super) const BYTES_PER_PIXEL: usize = 4;

/// How long the window may stay open: the menu is a moment's gesture, and the
/// window holds the keyboard, so a window nobody closes (a stuck host) closes
/// itself.
pub(super) const MAX_OPEN: Duration = Duration::from_secs(30);

/// Largest side of the surface accepted from the compositor, in pixels:
/// beyond it the buffer is not worth allocating.
pub(super) const MAX_SURFACE_SIDE: u32 = 16_384;

/// What one petal shows.
#[derive(Debug, Clone)]
pub struct Petal {
    /// Name of the entry.
    pub label: String,
    /// Icon, if the entry has one.
    pub icon: Option<RgbaImage>,
    /// What the module says on the petal, changing while the menu is open.
    pub note: PetalNote,
    /// The actions of the arc in front of the petal, in order, changing while
    /// the menu is open.
    pub items: Vec<RadialItem>,
}

/// What a module says on its petal while the menu is open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PetalNote {
    /// Up to two lines under the name, separated by `\n`.
    pub status: Option<String>,
    /// The count drawn as a badge on the icon; none (preferred) or zero
    /// draws no badge.
    pub badge: Option<u32>,
}

/// What the window draws while the menu is open.
#[derive(Debug, Clone)]
pub struct Content {
    /// The petals, in order.
    pub petals: Vec<Petal>,
    /// Where they are.
    pub layout: Layout,
    /// Their colours.
    pub palette: Palette,
    /// Open with an animation.
    pub animate: bool,
}

/// Tells the host what happens in the window; `false` when the host is gone.
pub type Report = Arc<dyn Fn(WindowEvent) -> bool + Send + Sync>;

/// What the menu is shown on: the window.
pub trait Surface {
    /// Shows the window; the menu is drawn once it is placed.
    fn open(&self, content: Content);
    /// Draws the menu at its place.
    fn place(&self, view: View);
    /// Draws the menu again.
    fn redraw(&self, view: View);
    /// Gives the petals what their modules say now (one per petal, in order)
    /// and draws the menu again.
    fn update_notes(&self, notes: Vec<PetalNote>);
    /// Gives the petals the actions their modules offer now (one list per
    /// petal, in order) and draws the menu again.
    fn update_items(&self, items: Vec<Vec<RadialItem>>);
    /// Hides the window.
    fn close(&self);
}

/// Whether a window shown at `opened_at` has been open longer than [`MAX_OPEN`].
pub(super) fn has_expired(opened_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(opened_at) > MAX_OPEN
}

/// How far the opening animation is, from 0 to 1.
pub(super) fn opening_progress(content: &Content, placed_at: Option<Instant>) -> f32 {
    match placed_at {
        Some(at) if content.animate => {
            (at.elapsed().as_secs_f32() / OPENING_DURATION.as_secs_f32()).clamp(0.0, 1.0)
        }
        _ => 1.0,
    }
}

/// Puts `notes` on `petals`, one each in order; a petal without one keeps
/// none.
pub(super) fn apply_notes(petals: &mut [Petal], notes: Vec<PetalNote>) {
    debug_assert!(
        notes.len() == petals.len(),
        "one note per petal: the entries do not change while the menu is open"
    );
    let mut notes = notes.into_iter();
    for petal in petals {
        petal.note = notes.next().unwrap_or_default();
    }
}

/// Puts `items` on `petals`, one list each in order; a petal without one keeps
/// none.
pub(super) fn apply_items(petals: &mut [Petal], items: Vec<Vec<RadialItem>>) {
    debug_assert!(
        items.len() == petals.len(),
        "one list of items per petal: the entries do not change while the menu is open"
    );
    let mut items = items.into_iter();
    for petal in petals {
        petal.items = items.next().unwrap_or_default();
    }
}

/// Copies `image` (straight alpha) into `canvas` (`Argb8888`, premultiplied),
/// centred on `centre` and cut at the edges of the canvas.
pub(super) fn blit(canvas: &mut [u8], width: u32, height: u32, image: &RgbaImage, centre: Point) {
    let left = whole(centre.x) - signed(image.width()) / 2;
    let top = whole(centre.y) - signed(image.height()) / 2;
    for (x, y, pixel) in image.enumerate_pixels() {
        let (column, row) = (left + signed(x), top + signed(y));
        if column < 0 || row < 0 || column >= signed(width) || row >= signed(height) {
            continue;
        }
        let [red, green, blue, alpha] = pixel.0;
        if alpha == 0 {
            continue;
        }
        let at = (usize::try_from(row).unwrap_or(0) * width as usize
            + usize::try_from(column).unwrap_or(0))
            * BYTES_PER_PIXEL;
        if let Some(target) = canvas.get_mut(at..at + BYTES_PER_PIXEL) {
            target.copy_from_slice(&[
                premultiply(blue, alpha),
                premultiply(green, alpha),
                premultiply(red, alpha),
                alpha,
            ]);
        }
    }
}

/// `channel` with `alpha` applied to it.
pub(super) fn premultiply(channel: u8, alpha: u8) -> u8 {
    u8::try_from(u16::from(channel) * u16::from(alpha) / 255).unwrap_or(u8::MAX)
}

/// A size of the surface, in pixels: whole and not negative.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn pixels(length: f64) -> u32 {
    length.max(0.0) as u32
}

/// A length that fits the signed sizes Wayland uses.
pub(super) fn signed(length: u32) -> i32 {
    i32::try_from(length).unwrap_or(i32::MAX)
}

/// A position, rounded to the pixel.
#[allow(clippy::cast_possible_truncation)]
pub(super) fn whole(position: f64) -> i32 {
    position.round() as i32
}

/// A window picked at run time is a window.
impl Surface for Box<dyn Surface + Send> {
    fn open(&self, content: Content) {
        (**self).open(content);
    }

    fn place(&self, view: View) {
        (**self).place(view);
    }

    fn redraw(&self, view: View) {
        (**self).redraw(view);
    }

    fn update_notes(&self, notes: Vec<PetalNote>) {
        (**self).update_notes(notes);
    }

    fn update_items(&self, items: Vec<Vec<RadialItem>>) {
        (**self).update_items(items);
    }

    fn close(&self) {
        (**self).close();
    }
}

/// Tells the host, when dropped, that the window is gone: the host must not go
/// on with a menu that has no window.
pub struct Gone(pub Report);

impl Drop for Gone {
    fn drop(&mut self) {
        // The host may be gone too.
        let _ = (self.0)(WindowEvent::Gone);
    }
}

/// `image` as the bytes of 32-bit buffers: blue, green, red, alpha, the colours
/// premultiplied by the alpha.
#[must_use]
pub fn premultiplied_bgra(image: &RgbaImage) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(image.pixels().len() * BYTES_PER_PIXEL);
    for pixel in image.pixels() {
        let [red, green, blue, alpha] = pixel.0;
        bytes.extend_from_slice(&[
            premultiply(blue, alpha),
            premultiply(green, alpha),
            premultiply(red, alpha),
            alpha,
        ]);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;
    use crate::radial::geometry::{DEFAULT_RING, Layout};
    use crate::radial::paint::DEFAULT_PALETTE;

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "one note per petal")]
    fn notes_that_do_not_match_the_petals_are_refused_in_debug() {
        let mut petals = vec![Petal {
            label: "a".to_owned(),
            icon: None,
            note: PetalNote::default(),
            items: Vec::new(),
        }];
        apply_notes(&mut petals, Vec::new());
    }

    #[test]
    fn statuses_are_given_to_the_petals_in_order_and_replace_what_they_said() {
        let petal = |label: &str| Petal {
            label: label.to_owned(),
            icon: None,
            note: PetalNote {
                status: Some("old".to_owned()),
                badge: Some(9),
            },
            items: Vec::new(),
        };
        let mut petals = vec![petal("a"), petal("b"), petal("c")];
        apply_notes(
            &mut petals,
            vec![
                PetalNote {
                    status: Some("new".to_owned()),
                    badge: Some(2),
                },
                PetalNote::default(),
                PetalNote {
                    status: Some("last".to_owned()),
                    badge: None,
                },
            ],
        );
        let badges: Vec<_> = petals.iter().map(|petal| petal.note.badge).collect();
        assert_eq!(badges, [Some(2), None, None]);
        let notes: Vec<_> = petals
            .iter()
            .map(|petal| petal.note.status.clone())
            .collect();
        assert_eq!(
            notes,
            [Some("new".to_owned()), None, Some("last".to_owned())]
        );
    }

    #[test]
    fn items_are_given_to_the_petals_in_order_and_replace_what_they_had() {
        use hubergamof_module_api::RadialGlyph;
        let item = |id: &str| RadialItem::new(id, id, RadialGlyph::Play);
        let petal = |items| Petal {
            label: String::new(),
            icon: None,
            note: PetalNote::default(),
            items,
        };
        let mut petals = vec![petal(vec![item("old")]), petal(Vec::new())];
        apply_items(&mut petals, vec![Vec::new(), vec![item("a"), item("b")]]);
        assert_eq!(petals[0].items, Vec::new());
        assert_eq!(petals[1].items, [item("a"), item("b")]);
    }

    fn content(animate: bool) -> Content {
        Content {
            petals: Vec::new(),
            layout: Layout::new(4, DEFAULT_RING).unwrap(),
            palette: DEFAULT_PALETTE,
            animate,
        }
    }

    /// The four bytes of the pixel (`column`, `row`) of a canvas `width` wide.
    fn at(canvas: &[u8], width: u32, column: usize, row: usize) -> [u8; 4] {
        let start = (row * width as usize + column) * BYTES_PER_PIXEL;
        canvas[start..start + BYTES_PER_PIXEL].try_into().unwrap()
    }

    #[test]
    fn an_image_is_copied_centred_with_its_colours_premultiplied_in_bgra_order() {
        let mut canvas = vec![0_u8; 10 * 10 * BYTES_PER_PIXEL];
        let image = RgbaImage::from_pixel(2, 2, Rgba([200, 100, 50, 128]));
        blit(&mut canvas, 10, 10, &image, Point::new(5.0, 5.0));
        // Covers the columns and rows 4 and 5.
        assert_eq!(at(&canvas, 10, 4, 4), [25, 50, 100, 128]);
        assert_eq!(at(&canvas, 10, 5, 5), [25, 50, 100, 128]);
        assert_eq!(at(&canvas, 10, 3, 4), [0, 0, 0, 0]);
        assert_eq!(at(&canvas, 10, 6, 5), [0, 0, 0, 0]);
    }

    #[test]
    fn an_image_that_overflows_the_canvas_is_cut_without_a_panic() {
        let mut canvas = vec![0_u8; 4 * 4 * BYTES_PER_PIXEL];
        let image = RgbaImage::from_pixel(6, 6, Rgba([255, 255, 255, 255]));
        for centre in [
            Point::new(0.0, 0.0),
            Point::new(4.0, 4.0),
            Point::new(-20.0, 2.0),
            Point::new(2.0, 90.0),
        ] {
            blit(&mut canvas, 4, 4, &image, centre);
        }
        assert_eq!(at(&canvas, 4, 0, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn transparent_pixels_leave_the_canvas_alone() {
        let mut canvas = vec![7_u8; 4 * 4 * BYTES_PER_PIXEL];
        let image = RgbaImage::from_pixel(2, 2, Rgba([255, 255, 255, 0]));
        blit(&mut canvas, 4, 4, &image, Point::new(2.0, 2.0));
        assert!(canvas.iter().all(|byte| *byte == 7));
    }

    #[test]
    fn premultiplying_scales_a_channel_by_the_alpha() {
        assert_eq!(premultiply(255, 255), 255);
        assert_eq!(premultiply(255, 0), 0);
        assert_eq!(premultiply(200, 128), 100);
    }

    #[test]
    fn the_opening_goes_from_nothing_to_whole_in_its_duration() {
        let long_ago = Instant::now().checked_sub(OPENING_DURATION * 2);
        assert!((opening_progress(&content(true), long_ago) - 1.0).abs() < f32::EPSILON);
        let just_now = opening_progress(&content(true), Some(Instant::now()));
        assert!(just_now < 0.5, "{just_now}");
    }

    #[test]
    fn a_menu_that_is_not_animated_or_not_placed_is_whole() {
        assert!(
            (opening_progress(&content(false), Some(Instant::now())) - 1.0).abs() < f32::EPSILON
        );
        assert!((opening_progress(&content(true), None) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_window_open_too_long_has_expired_and_a_recent_one_has_not() {
        let now = Instant::now();
        assert!(!has_expired(now, now));
        assert!(!has_expired(now, now + MAX_OPEN));
        assert!(has_expired(now, now + MAX_OPEN + Duration::from_millis(1)));
        // A clock that went backwards is not an expiry.
        assert!(!has_expired(now + MAX_OPEN, now));
    }

    #[test]
    fn lengths_are_made_whole_without_going_negative_or_overflowing() {
        assert_eq!(pixels(-3.0), 0);
        assert_eq!(pixels(1280.4), 1280);
        assert_eq!(signed(u32::MAX), i32::MAX);
        assert_eq!(whole(4.5), 5);
    }
}
