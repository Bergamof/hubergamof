//! Live state of a device as last drawn, for the web UI.

use std::sync::Arc;

use hubergamof_module_api::{Feedback, KeyState};
use hubergamof_protocol::{KeyView, LiveState, PageView};

use crate::render::{RenderError, Rendered};
use crate::strip::StripFace;

/// Page shown on the keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSummary {
    /// `home`, or `<module instance>/<page>`.
    pub id: String,
    /// Title (`Accueil` on the home page).
    pub title: String,
    /// First visible row.
    pub row: u32,
    /// Number of rows of the page (at least 1).
    pub row_count: u32,
}

/// A key as drawn.
#[derive(Debug, Clone)]
pub struct LiveKey {
    /// Effective label.
    pub label: Option<String>,
    /// Visual state reported by the module.
    pub state: KeyState,
    /// Outcome mark drawn over the key for a moment (part of `image`).
    pub feedback: Option<Feedback>,
    /// Drawn image, `None` for a blank key.
    pub image: Option<Arc<Rendered>>,
}

/// The touch strip as drawn.
#[derive(Debug, Clone)]
pub struct LiveStrip {
    /// What it shows, as data.
    pub face: StripFace,
    /// Drawn image, `None` while a notification or a label covers it or
    /// before it was written.
    pub image: Option<Arc<Rendered>>,
}

/// What a device shows, published by its engine after every visible change.
///
/// Cheap to clone: images are shared with the engine and encoded as data URLs
/// only by [`LiveSnapshot::to_live_state`], i.e. only when someone reads it
/// (each image is encoded once, then cached).
#[derive(Debug, Clone)]
pub struct LiveSnapshot {
    /// Device serial.
    pub serial: String,
    /// Page on the keys.
    pub page: PageSummary,
    /// Keys, by index (empty before the first draw).
    pub keys: Vec<LiveKey>,
    /// The whole touch strip, `None` before the first draw or without a strip.
    pub strip: Option<LiveStrip>,
}

impl LiveSnapshot {
    /// State before anything was drawn.
    #[must_use]
    pub fn empty(serial: &str) -> Self {
        Self {
            serial: serial.to_owned(),
            page: PageSummary {
                id: String::new(),
                title: String::new(),
                row: 0,
                row_count: 1,
            },
            keys: Vec::new(),
            strip: None,
        }
    }

    /// Wire form, with images as PNG data URLs. CPU-bound on first call for
    /// each image: call it from a blocking context.
    ///
    /// # Errors
    ///
    /// [`RenderError::Encode`].
    pub fn to_live_state(&self) -> Result<LiveState, RenderError> {
        let keys = self
            .keys
            .iter()
            .zip(0_u32..)
            .map(|(key, index)| {
                Ok(KeyView {
                    index,
                    label: key.label.clone(),
                    image: data_url(key.image.as_ref())?,
                    state: key.state,
                })
            })
            .collect::<Result<_, RenderError>>()?;
        Ok(LiveState {
            serial: self.serial.clone(),
            page: PageView {
                id: self.page.id.clone(),
                title: self.page.title.clone(),
                row: self.page.row,
                row_count: self.page.row_count,
                keys,
            },
            strip: data_url(self.strip.as_ref().and_then(|strip| strip.image.as_ref()))?,
        })
    }
}

fn data_url(image: Option<&Arc<Rendered>>) -> Result<Option<String>, RenderError> {
    image.map(|rendered| rendered.data_url()).transpose()
}
