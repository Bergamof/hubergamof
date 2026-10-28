//! Draws a [`Frame`] on a device, rewriting only the keys and zones whose
//! look changed.

use std::sync::Arc;

use bergadeck_protocol::ImageSize;
use tokio::task::spawn_blocking;

use super::EngineError;
use super::frame::{Frame, KeyFace, ZoneFace};
use super::live::{LiveDial, LiveKey, LiveSnapshot, LiveZone};
use crate::device::{DeckBackend, StripLayout};
use crate::render::{DEFAULT_BACKGROUND, KeyRequest, RenderError, Rendered, Renderer};

struct Painted<F> {
    face: F,
    rendered: Arc<Rendered>,
}

/// What was last written to each key and zone of a device.
pub(crate) struct Painter {
    key_size: ImageSize,
    zone_size: Option<ImageSize>,
    keys: Vec<Option<Painted<KeyFace>>>,
    zones: Vec<Option<Painted<ZoneFace>>>,
}

/// Images to write, by key or zone index.
struct Batch {
    keys: Vec<(u8, KeyFace)>,
    zones: Vec<(u8, ZoneFace)>,
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
        let batch = self.changes(frame);
        if batch.keys.is_empty() && batch.zones.is_empty() {
            return Ok(0);
        }
        let images = self.render(renderer, &batch).await?;
        let mut written = 0;
        for ((index, face), rendered) in batch.keys.into_iter().zip(images.keys) {
            deck.set_key_image(index, rendered.image().as_ref().clone())
                .await?;
            self.keys[usize::from(index)] = Some(Painted { face, rendered });
            written += 1;
        }
        for ((index, face), rendered) in batch.zones.into_iter().zip(images.zones) {
            deck.set_touch_zone_image(index, rendered.image().as_ref().clone())
                .await?;
            self.zones[usize::from(index)] = Some(Painted { face, rendered });
            written += 1;
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
            .map(|(face, painted)| LiveKey {
                label: face.label.clone(),
                active: face.active,
                image: painted
                    .as_ref()
                    .filter(|_| !face.is_blank())
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
                    lines: match zone {
                        ZoneFace::Lines(lines) => lines.clone(),
                        ZoneFace::Blank | ZoneFace::Image(_) => Vec::new(),
                    },
                    image: self
                        .zones
                        .get(index)
                        .and_then(Option::as_ref)
                        .filter(|_| *zone != ZoneFace::Blank)
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

    fn changes(&self, frame: &Frame) -> Batch {
        let keys = frame
            .keys
            .iter()
            .zip(&self.keys)
            .zip(0_u8..)
            .filter(|((face, painted), _)| {
                painted
                    .as_ref()
                    .is_none_or(|painted| !painted.face.looks_like(face))
            })
            .map(|((face, _), index)| (index, face.clone()))
            .collect();
        let zones = frame
            .dials
            .iter()
            .filter_map(|dial| dial.zone.as_ref())
            .zip(&self.zones)
            .zip(0_u8..)
            .filter(|((face, painted), _)| {
                painted
                    .as_ref()
                    .is_none_or(|painted| painted.face != **face)
            })
            .map(|((face, _), index)| (index, face.clone()))
            .collect();
        Batch { keys, zones }
    }

    async fn render(&self, renderer: &Arc<Renderer>, batch: &Batch) -> Result<Images, EngineError> {
        let renderer = Arc::clone(renderer);
        let key_size = self.key_size;
        let zone_size = self.zone_size;
        let keys: Vec<KeyFace> = batch.keys.iter().map(|(_, face)| face.clone()).collect();
        let zones: Vec<ZoneFace> = batch.zones.iter().map(|(_, face)| face.clone()).collect();
        let images = spawn_blocking(move || -> Result<Images, RenderError> {
            let keys = keys
                .iter()
                .map(|face| render_key(&renderer, key_size, face))
                .collect::<Result<_, _>>()?;
            let zones = match zone_size {
                Some(size) => zones
                    .iter()
                    .map(|face| render_zone(&renderer, size, face))
                    .collect::<Result<_, _>>()?,
                None => Vec::new(),
            };
            Ok(Images { keys, zones })
        })
        .await??;
        Ok(images)
    }
}

struct Images {
    keys: Vec<Arc<Rendered>>,
    zones: Vec<Arc<Rendered>>,
}

/// Renders a key; an unreadable image falls back to the label alone.
fn render_key(
    renderer: &Renderer,
    size: ImageSize,
    face: &KeyFace,
) -> Result<Arc<Rendered>, RenderError> {
    let label = face.label.as_deref();
    match renderer.render_key(&KeyRequest::new(size, face.image.as_ref(), label)) {
        Err(error) if face.image.is_some() => {
            tracing::warn!(%error, "cannot draw key image, showing the label only");
            renderer.render_key(&KeyRequest::new(size, None, label))
        }
        result => result,
    }
}

/// Renders a zone; an unreadable image falls back to a blank zone.
fn render_zone(
    renderer: &Renderer,
    size: ImageSize,
    face: &ZoneFace,
) -> Result<Arc<Rendered>, RenderError> {
    let blank = || renderer.render_zone(&[], size, DEFAULT_BACKGROUND);
    match face {
        ZoneFace::Blank => blank(),
        ZoneFace::Lines(lines) => renderer.render_zone(lines, size, DEFAULT_BACKGROUND),
        ZoneFace::Image(source) => renderer
            .render_key(&KeyRequest::new(size, Some(source), None))
            .or_else(|error| {
                tracing::warn!(%error, "cannot draw zone image, leaving it blank");
                blank()
            }),
    }
}
