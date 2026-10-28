//! Live state of a device as last drawn, for the web UI.

use std::sync::Arc;

use bergadeck_module_api::ModuleInstanceId;
use bergadeck_protocol::{DialView, KeyView, LiveState, PageView, ZoneView};

use crate::render::{RenderError, Rendered};

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
    /// Active state reported by the module.
    pub active: bool,
    /// Drawn image, `None` for a blank key.
    pub image: Option<Arc<Rendered>>,
}

/// A touch strip zone as drawn.
#[derive(Debug, Clone)]
pub struct LiveZone {
    /// Text lines, empty for an image or a blank zone.
    pub lines: Vec<String>,
    /// Drawn image, `None` for a blank zone.
    pub image: Option<Arc<Rendered>>,
}

/// A dial and its zone.
#[derive(Debug, Clone)]
pub struct LiveDial {
    /// Module instance assigned to the dial.
    pub module_id: Option<ModuleInstanceId>,
    /// Zone above the dial, `None` when the device has no touch strip.
    pub zone: Option<LiveZone>,
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
    /// Dials, by index.
    pub dials: Vec<LiveDial>,
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
            dials: Vec::new(),
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
                    active: key.active,
                })
            })
            .collect::<Result<_, RenderError>>()?;
        let dials = self
            .dials
            .iter()
            .zip(0_u32..)
            .map(|(dial, index)| {
                let zone = dial
                    .zone
                    .as_ref()
                    .map(|zone| {
                        Ok::<_, RenderError>(ZoneView {
                            lines: zone.lines.clone(),
                            image: data_url(zone.image.as_ref())?,
                        })
                    })
                    .transpose()?;
                Ok(DialView {
                    index,
                    module_id: dial.module_id.clone(),
                    zone,
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
            dials,
        })
    }
}

fn data_url(image: Option<&Arc<Rendered>>) -> Result<Option<String>, RenderError> {
    image
        .map(|rendered| rendered.data_url().map(str::to_owned))
        .transpose()
}
