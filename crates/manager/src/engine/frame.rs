//! Description of everything a device should show, compared between redraws
//! to rewrite only what changed.

use bergadeck_module_api::{ImageData, KeyContent, MAX_ZONE_LINES, ModuleInstanceId, ZoneContent};

use super::live::PageSummary;
use crate::render::ImageSource;

/// What a key shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct KeyFace {
    pub(crate) image: Option<ImageSource>,
    pub(crate) label: Option<String>,
    /// Reported in the live state; does not change the drawing.
    pub(crate) active: bool,
}

impl KeyFace {
    pub(crate) fn from_content(content: KeyContent) -> Self {
        Self {
            image: content.image.map(image_source),
            label: content.label,
            active: content.active,
        }
    }

    pub(crate) fn is_blank(&self) -> bool {
        self.image.is_none() && self.label.is_none()
    }

    /// Whether both faces draw the same pixels.
    pub(crate) fn looks_like(&self, other: &Self) -> bool {
        self.image == other.image && self.label == other.label
    }
}

/// What a touch strip zone shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum ZoneFace {
    #[default]
    Blank,
    Lines(Vec<String>),
    Image(ImageSource),
}

impl ZoneFace {
    pub(crate) fn from_content(content: ZoneContent) -> Self {
        match content {
            ZoneContent::Lines(mut lines) => {
                lines.truncate(MAX_ZONE_LINES);
                Self::Lines(lines)
            }
            ZoneContent::Image(image) => Self::Image(image_source(image)),
        }
    }
}

/// A dial: its module and, on devices with a touch strip, its zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DialFace {
    pub(crate) module: Option<ModuleInstanceId>,
    pub(crate) zone: Option<ZoneFace>,
}

/// Everything the device shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frame {
    pub(crate) page: PageSummary,
    pub(crate) keys: Vec<KeyFace>,
    pub(crate) dials: Vec<DialFace>,
}

fn image_source(image: ImageData) -> ImageSource {
    // The renderer recognises the format from the content.
    ImageSource::Bytes(image.bytes)
}
