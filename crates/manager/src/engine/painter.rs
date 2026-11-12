//! Draws a [`Frame`] on a device, rewriting only the keys and the strip when
//! their look changed.

use std::sync::Arc;

use hubergamof_module_api::Feedback;
use hubergamof_protocol::ImageSize;
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
        self.image == other.image && self.label == other.label && self.state == other.state
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
        let keys = changes(frame.keys.iter().map(|key| &key.face), &self.keys);
        let strip = self.strip_to_paint(frame);
        let overlay_changed = self.strip_size.is_some() && frame.overlay != self.overlay;
        if keys.is_empty() && strip.is_none() && !overlay_changed {
            return Ok(0);
        }
        let written = keys.len() + usize::from(strip.is_some()) + usize::from(overlay_changed);
        let key_size = self.key_size;
        let keys = render(renderer, keys, move |renderer, face| {
            render_key(renderer, key_size, face)
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
            self.paint_strip(deck, renderer, face).await?;
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
    ) -> Result<(), EngineError> {
        let Some(size) = self.strip_size else {
            return Ok(());
        };
        let renderer = Arc::clone(renderer);
        let shown = face.clone();
        let image = spawn_blocking(move || renderer.render_strip(&shown, size)).await??;
        deck.set_strip_region(0, Arc::clone(image.image())).await?;
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
    face: &KeyFace,
) -> Result<Arc<Rendered>, RenderError> {
    let label = face.label.as_deref();
    let request = |image| KeyRequest::new(size, image, label).in_state(face.state);
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
