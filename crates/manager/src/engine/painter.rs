//! Draws a [`Frame`] on a device, rewriting only the keys and zones whose
//! look changed.

use std::sync::Arc;

use bergadeck_module_api::Feedback;
use bergadeck_protocol::ImageSize;
use tokio::task::spawn_blocking;

use super::EngineError;
use super::frame::{Frame, KeyFace, ZoneContentFace, ZoneFace};
use super::live::{LiveDial, LiveKey, LiveSnapshot, LiveZone};
use crate::device::{DeckBackend, StripLayout};
use crate::render::{DEFAULT_BACKGROUND, KeyRequest, RenderError, Rendered, Renderer};

/// What was written to a key or zone: its face, the image without the
/// feedback mark (`base`) and the image written.
struct Painted<F> {
    face: F,
    base: Arc<Rendered>,
    rendered: Arc<Rendered>,
}

/// A key or zone face, which may carry a transient feedback mark.
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

impl Face for ZoneFace {
    fn feedback(&self) -> Option<Feedback> {
        self.feedback
    }

    fn same_base(&self, other: &Self) -> bool {
        self.content == other.content
    }
}

/// A key or zone to write, with the base image to reuse when only its
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

/// What was last written to each key and zone of a device.
pub(crate) struct Painter {
    key_size: ImageSize,
    zone_size: Option<ImageSize>,
    keys: Vec<Option<Painted<KeyFace>>>,
    zones: Vec<Option<Painted<ZoneFace>>>,
}

impl Painter {
    pub(crate) fn new(deck: &dyn DeckBackend) -> Self {
        let capabilities = &deck.info().capabilities;
        let strip = StripLayout::for_device(deck.info());
        Self {
            key_size: capabilities.key_image_size,
            zone_size: strip.map(|layout| layout.zone_size()),
            keys: std::iter::repeat_with(|| None)
                .take(capabilities.key_count())
                .collect(),
            zones: std::iter::repeat_with(|| None)
                .take(strip.map_or(0, |layout| usize::from(layout.zone_count())))
                .collect(),
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
        let zones = changes(
            frame.dials.iter().filter_map(|dial| dial.zone.as_ref()),
            &self.zones,
        );
        if keys.is_empty() && zones.is_empty() {
            return Ok(0);
        }
        let written = keys.len() + zones.len();
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
        if let Some(zone_size) = self.zone_size {
            let zones = render(renderer, zones, move |renderer, face| {
                render_zone(renderer, zone_size, face)
            })
            .await?;
            for (job, images) in zones {
                deck.set_touch_zone_image(job.index, Arc::clone(images.rendered.image()))
                    .await?;
                self.zones[usize::from(job.index)] = Some(painted(job.face, images));
            }
        }
        deck.flush().await?;
        Ok(written)
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
        let dials = frame
            .dials
            .iter()
            .enumerate()
            .map(|(index, dial)| LiveDial {
                module_id: dial.module.clone(),
                zone: dial.zone.as_ref().map(|zone| LiveZone {
                    feedback: zone.feedback,
                    lines: match &zone.content {
                        ZoneContentFace::Lines(lines) => lines.clone(),
                        ZoneContentFace::Blank | ZoneContentFace::Image(_) => Vec::new(),
                    },
                    image: self
                        .zones
                        .get(index)
                        .and_then(Option::as_ref)
                        .filter(|_| !zone.is_blank())
                        .map(|painted| Arc::clone(&painted.rendered)),
                }),
            })
            .collect();
        LiveSnapshot {
            serial: serial.to_owned(),
            page: frame.page.clone(),
            keys,
            dials,
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

/// Renders the base of a zone; an unreadable image falls back to a blank
/// zone.
fn render_zone(
    renderer: &Renderer,
    size: ImageSize,
    face: &ZoneFace,
) -> Result<Arc<Rendered>, RenderError> {
    let blank = || renderer.render_zone(&[], size, DEFAULT_BACKGROUND);
    match &face.content {
        ZoneContentFace::Blank => blank(),
        ZoneContentFace::Lines(lines) => renderer.render_zone(lines, size, DEFAULT_BACKGROUND),
        ZoneContentFace::Image(source) => renderer.render_image(size, source).or_else(|error| {
            tracing::warn!(%error, "cannot draw zone image, leaving it blank");
            blank()
        }),
    }
}
