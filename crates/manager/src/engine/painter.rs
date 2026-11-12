//! Draws a [`Frame`] on a device, rewriting only the keys and the strip when
//! their look changed.

use std::sync::Arc;

use hubergamof_module_api::Feedback;
use hubergamof_protocol::{DeviceTheme, ImageSize};
use image::RgbaImage;
use image::imageops::crop_imm;
use tokio::task::spawn_blocking;

use super::EngineError;
use super::frame::{Frame, KeyFace, StripOverlay};
use super::live::{LiveKey, LiveSnapshot, LiveStrip};
use crate::device::{DeckBackend, StripLayout};
use crate::render::{KeyRequest, NotificationLook, RenderError, Rendered, Renderer};
use crate::strip::StripFace;

/// What was written to a key: its face, the image without the
/// feedback mark (`base`) and the image written.
struct Painted<F> {
    face: F,
    base: Arc<Rendered>,
    rendered: Arc<Rendered>,
}

/// A key face, which may carry a transient feedback mark.
trait Face: Clone + PartialEq + Send + 'static {
    fn feedback(&self) -> Option<Feedback>;

    /// Whether both faces look the same once their marks are removed.
    fn same_base(&self, other: &Self) -> bool;
}

impl Face for KeyFace {
    fn feedback(&self) -> Option<Feedback> {
        self.feedback
    }

    fn same_base(&self, other: &Self) -> bool {
        // Destructured without `..`: a field added to the face must be
        // decided here, being part of the base or not, to compile.
        let Self {
            image,
            label,
            state,
            focused,
            checked,
            feedback: _,
        } = self;
        *image == other.image
            && *label == other.label
            && *state == other.state
            && *focused == other.focused
            && *checked == other.checked
    }
}

/// A key to write, with the base image to reuse when only its
/// feedback mark changed.
struct Job<F> {
    index: u8,
    face: F,
    base: Option<Arc<Rendered>>,
}

/// Base and final images of a job.
struct Images {
    base: Arc<Rendered>,
    rendered: Arc<Rendered>,
}

const BYTES_PER_PIXEL: usize = 4;

/// What differs between two images of the strip.
#[derive(Debug, PartialEq, Eq)]
enum Changed {
    /// No previous image to compare with, or of another size.
    Whole,
    /// The columns from `x`, over the full height.
    Columns {
        x: u32,
        width: u32,
    },
    Nothing,
}

/// The columns of `after` that differ from `before`.
fn changed_columns(before: Option<&RgbaImage>, after: &RgbaImage) -> Changed {
    let Some(before) = before.filter(|before| before.dimensions() == after.dimensions()) else {
        return Changed::Whole;
    };
    let width = after.width() as usize;
    let row_bytes = width * BYTES_PER_PIXEL;
    let (mut first, mut last) = (width, 0);
    for (old, new) in before
        .as_raw()
        .chunks_exact(row_bytes)
        .zip(after.as_raw().chunks_exact(row_bytes))
    {
        if old == new {
            continue;
        }
        let (old, _) = old.as_chunks::<BYTES_PER_PIXEL>();
        let (new, _) = new.as_chunks::<BYTES_PER_PIXEL>();
        let differing = old
            .iter()
            .zip(new)
            .enumerate()
            .filter(|(_, (old, new))| old != new)
            .map(|(column, _)| column);
        for column in differing {
            first = first.min(column);
            last = last.max(column);
        }
    }
    if first > last {
        return Changed::Nothing;
    }
    let x = u32::try_from(first).unwrap_or(0);
    let right = u32::try_from(last + 1).unwrap_or(after.width());
    Changed::Columns {
        x,
        width: right - x,
    }
}

/// What was last written to the touch strip.
struct PaintedStrip {
    face: StripFace,
    rendered: Arc<Rendered>,
}

/// What was last written to each key and to the strip of a device.
pub(crate) struct Painter {
    key_size: ImageSize,
    strip_size: Option<ImageSize>,
    /// What covers the strip, `None` when the strip itself shows.
    overlay: Option<StripOverlay>,
    /// Whether the notification last drawn scrolls, until the engine took it.
    measured_scrolls: Option<bool>,
    keys: Vec<Option<Painted<KeyFace>>>,
    /// Theme the keys were drawn with, `None` before the first drawing.
    key_theme: Option<DeviceTheme>,
    strip: Option<PaintedStrip>,
}

impl Painter {
    pub(crate) fn new(deck: &dyn DeckBackend) -> Self {
        let capabilities = &deck.info().capabilities;
        let strip = StripLayout::for_device(deck.info());
        Self {
            key_size: capabilities.key_image_size,
            strip_size: strip.map(|layout| layout.strip_size()),
            overlay: None,
            measured_scrolls: None,
            keys: std::iter::repeat_with(|| None)
                .take(capabilities.key_count())
                .collect(),
            key_theme: None,
            strip: None,
        }
    }

    /// Writes what changed since the last call and returns the number of
    /// images written.
    pub(crate) async fn paint(
        &mut self,
        deck: &dyn DeckBackend,
        renderer: &Arc<Renderer>,
        frame: &Frame,
    ) -> Result<usize, EngineError> {
        if self.key_theme != Some(frame.theme) {
            // Every key sits on the surface of the theme: all are drawn again.
            self.keys.fill_with(|| None);
            self.key_theme = Some(frame.theme);
        }
        let keys = changes(frame.keys.iter().map(|key| &key.face), &self.keys);
        let strip = self.strip_to_paint(frame);
        // An overlay hides the strip drawn before: nothing of it can be reused.
        let covered = self.overlay.is_some();
        let overlay_changed = self.strip_size.is_some() && frame.overlay != self.overlay;
        if keys.is_empty() && strip.is_none() && !overlay_changed {
            return Ok(0);
        }
        let written = keys.len() + usize::from(strip.is_some()) + usize::from(overlay_changed);
        let key_size = self.key_size;
        let theme = frame.theme;
        let keys = render(renderer, keys, move |renderer, face| {
            render_key(renderer, key_size, theme, face)
        })
        .await?;
        for (job, images) in keys {
            deck.set_key_image(job.index, Arc::clone(images.rendered.image()))
                .await?;
            self.keys[usize::from(job.index)] = Some(painted(job.face, images));
        }
        if overlay_changed {
            self.paint_overlay(deck, renderer, frame.overlay.as_ref())
                .await?;
        }
        if let Some(face) = strip {
            self.paint_strip(deck, renderer, face, covered).await?;
        }
        deck.flush().await?;
        Ok(written)
    }

    /// The face to write to the strip: when it changed, or when the overlay
    /// that covered it went away. Nothing under an overlay.
    fn strip_to_paint(&self, frame: &Frame) -> Option<StripFace> {
        let face = frame.strip.as_ref()?;
        if frame.overlay.is_some() {
            return None;
        }
        let covered = self.overlay.is_some();
        let changed = self.strip.as_ref().map(|painted| &painted.face) != Some(face);
        (covered || changed).then(|| face.clone())
    }

    async fn paint_strip(
        &mut self,
        deck: &dyn DeckBackend,
        renderer: &Arc<Renderer>,
        face: StripFace,
        covered: bool,
    ) -> Result<(), EngineError> {
        let Some(size) = self.strip_size else {
            return Ok(());
        };
        let renderer = Arc::clone(renderer);
        let shown = face.clone();
        let image = spawn_blocking(move || renderer.render_strip(&shown, size)).await??;
        // Writing the columns that changed only keeps a dial turned in a menu
        // fast: the device takes less to receive and show a narrow region.
        let before = self
            .strip
            .as_ref()
            .filter(|_| !covered)
            .map(|painted| painted.rendered.image());
        match changed_columns(before.map(AsRef::as_ref), image.image()) {
            Changed::Whole => deck.set_strip_region(0, Arc::clone(image.image())).await?,
            Changed::Columns { x, width } => {
                let region = crop_imm(image.image().as_ref(), x, 0, width, size.height);
                deck.set_strip_region(x, Arc::new(region.to_image()))
                    .await?;
            }
            Changed::Nothing => {}
        }
        self.strip = Some(PaintedStrip {
            face,
            rendered: image,
        });
        Ok(())
    }

    /// Forgets what the strip shows: the next `paint` writes it again.
    pub(crate) fn forget_strip(&mut self) {
        self.strip = None;
    }

    /// Whether the notification just drawn scrolls, once per drawing: the
    /// renderer measures the message while drawing it.
    pub(crate) fn take_scrolls(&mut self) -> Option<bool> {
        self.measured_scrolls.take()
    }

    /// Covers the strip with `overlay`; `None` only forgets the previous one
    /// (the strip is drawn again by the same `paint`).
    async fn paint_overlay(
        &mut self,
        deck: &dyn DeckBackend,
        renderer: &Arc<Renderer>,
        overlay: Option<&StripOverlay>,
    ) -> Result<(), EngineError> {
        let (Some(size), Some(overlay)) = (self.strip_size, overlay) else {
            self.overlay = None;
            return Ok(());
        };
        let renderer = Arc::clone(renderer);
        let shown = overlay.clone();
        let (image, scrolls) =
            spawn_blocking(move || render_overlay(&renderer, size, &shown)).await??;
        self.measured_scrolls = scrolls;
        deck.set_strip_region(0, image).await?;
        self.overlay = Some(overlay.clone());
        Ok(())
    }

    /// Live state of `frame`, once painted.
    pub(crate) fn snapshot(&self, serial: &str, frame: &Frame) -> LiveSnapshot {
        let keys = frame
            .keys
            .iter()
            .zip(&self.keys)
            .map(|(key, painted)| LiveKey {
                label: key.face.label.clone(),
                state: key.face.state,
                feedback: key.face.feedback,
                image: painted
                    .as_ref()
                    .filter(|_| !key.face.is_blank())
                    .map(|painted| Arc::clone(&painted.rendered)),
            })
            .collect();
        LiveSnapshot {
            serial: serial.to_owned(),
            page: frame.page.clone(),
            keys,
            strip: frame.strip.as_ref().map(|face| LiveStrip {
                face: face.clone(),
                image: self
                    .strip
                    .as_ref()
                    .filter(|painted| painted.face == *face)
                    .map(|painted| Arc::clone(&painted.rendered)),
            }),
        }
    }
}

fn painted<F>(face: F, images: Images) -> Painted<F> {
    Painted {
        face,
        base: images.base,
        rendered: images.rendered,
    }
}

/// Faces differing from what was painted, with the base image to reuse when
/// only the mark changed.
fn changes<'a, F: Face>(
    faces: impl Iterator<Item = &'a F>,
    painted: &[Option<Painted<F>>],
) -> Vec<Job<F>> {
    faces
        .zip(painted)
        .zip(0_u8..)
        .filter(|((face, painted), _)| {
            painted
                .as_ref()
                .is_none_or(|painted| painted.face != **face)
        })
        .map(|((face, painted), index)| Job {
            index,
            face: face.clone(),
            base: painted
                .as_ref()
                .filter(|painted| painted.face.same_base(face))
                .map(|painted| Arc::clone(&painted.base)),
        })
        .collect()
}

/// Renders `jobs` on the blocking pool: the base with `render_base` unless
/// it is reused, then the feedback mark.
async fn render<F: Face>(
    renderer: &Arc<Renderer>,
    jobs: Vec<Job<F>>,
    render_base: impl Fn(&Renderer, &F) -> Result<Arc<Rendered>, RenderError> + Send + 'static,
) -> Result<Vec<(Job<F>, Images)>, EngineError> {
    if jobs.is_empty() {
        return Ok(Vec::new());
    }
    let renderer = Arc::clone(renderer);
    let images = spawn_blocking(move || {
        jobs.into_iter()
            .map(|job| {
                let base = match &job.base {
                    Some(base) => Arc::clone(base),
                    None => render_base(&renderer, &job.face)?,
                };
                let marked = match job.face.feedback() {
                    Some(feedback) => renderer.with_feedback(&base, feedback)?,
                    None => Arc::clone(&base),
                };
                Ok((
                    job,
                    Images {
                        base,
                        rendered: marked,
                    },
                ))
            })
            .collect::<Result<Vec<_>, RenderError>>()
    })
    .await??;
    Ok(images)
}

/// Renders the base of a key; an unreadable image falls back to the label
/// alone.
fn render_key(
    renderer: &Renderer,
    size: ImageSize,
    theme: DeviceTheme,
    face: &KeyFace,
) -> Result<Arc<Rendered>, RenderError> {
    let label = face.label.as_deref();
    let request = |image| {
        KeyRequest::new(size, image, label)
            .themed(theme)
            .in_state(face.state)
            .focused(face.focused)
            .checked(face.checked)
    };
    match renderer.render_key(&request(face.image.as_ref())) {
        Err(error) if face.image.is_some() => {
            tracing::warn!(%error, "cannot draw key image, showing the label only");
            renderer.render_key(&request(None))
        }
        result => result,
    }
}

/// Renders what covers the strip; for a notification, also whether its
/// message scrolls.
fn render_overlay(
    renderer: &Renderer,
    size: ImageSize,
    overlay: &StripOverlay,
) -> Result<(Arc<image::RgbaImage>, Option<bool>), RenderError> {
    match overlay {
        StripOverlay::HeldLabel(label) => Ok((renderer.render_strip_label(label, size)?, None)),
        StripOverlay::Notification(face) => {
            let look = NotificationLook {
                size,
                background: face.background,
                icon: face.icon.as_ref(),
                message: &face.message,
            };
            let drawn = renderer
                .render_notification(&look, face.elapsed, face.timeout)
                .or_else(|error| {
                    // An unreadable icon must not hide the message.
                    tracing::warn!(%error, "cannot draw notification icon, showing the message only");
                    let look = NotificationLook { icon: None, ..look };
                    renderer.render_notification(&look, face.elapsed, face.timeout)
                })?;
            Ok((drawn.image, Some(drawn.scrolls)))
        }
    }
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;

    fn blank() -> RgbaImage {
        RgbaImage::from_pixel(20, 4, Rgba([9, 9, 9, 255]))
    }

    #[test]
    fn without_a_comparable_image_the_whole_strip_is_written() {
        let image = blank();
        assert_eq!(changed_columns(None, &image), Changed::Whole);
        let other_size = RgbaImage::new(10, 4);
        assert_eq!(changed_columns(Some(&other_size), &image), Changed::Whole);
    }

    #[test]
    fn identical_images_write_nothing() {
        assert_eq!(changed_columns(Some(&blank()), &blank()), Changed::Nothing);
    }

    #[test]
    fn only_the_columns_that_differ_are_written() {
        let before = blank();
        let mut after = blank();
        after.put_pixel(5, 0, Rgba([1, 2, 3, 255]));
        after.put_pixel(12, 3, Rgba([1, 2, 3, 255]));
        assert_eq!(
            changed_columns(Some(&before), &after),
            Changed::Columns { x: 5, width: 8 }
        );
        let mut edge = blank();
        edge.put_pixel(0, 1, Rgba([1, 2, 3, 255]));
        edge.put_pixel(19, 1, Rgba([1, 2, 3, 255]));
        assert_eq!(
            changed_columns(Some(&before), &edge),
            Changed::Columns { x: 0, width: 20 }
        );
    }
}
