//! Description of everything a device should show, compared between redraws
//! to rewrite only what changed.

use hubergamof_module_api::{
    Feedback, GaugeContent, ImageData, ItemId, KeyContent, KeyState, MAX_ZONE_LINES,
    ModuleInstanceId, PageId, ZoneContent,
};

use super::live::PageSummary;
use crate::render::ImageSource;

/// What a key shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct KeyFace {
    pub(crate) image: Option<ImageSource>,
    pub(crate) label: Option<String>,
    pub(crate) state: KeyState,
    /// Transient outcome mark drawn over the key.
    pub(crate) feedback: Option<Feedback>,
}

impl KeyFace {
    pub(crate) fn from_content(content: KeyContent) -> Self {
        Self {
            image: content.image.map(image_source),
            label: content.label,
            state: content.state,
            feedback: None,
        }
    }

    pub(crate) fn is_blank(&self) -> bool {
        self.image.is_none()
            && self.label.is_none()
            && self.state == KeyState::Normal
            && self.feedback.is_none()
    }
}

/// What a press of a key acts on: resolved from the frame on display, so a
/// press always reaches what the user saw.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum KeyTarget {
    /// The home page key of a module.
    Home(ModuleInstanceId),
    /// An item of a module page.
    Item {
        module: ModuleInstanceId,
        page: PageId,
        item: ItemId,
    },
}

impl KeyTarget {
    /// Module instance handling the press.
    pub(crate) fn module(&self) -> &ModuleInstanceId {
        match self {
            Self::Home(module) | Self::Item { module, .. } => module,
        }
    }
}

/// A key of the frame: its look and what pressing it does (`None` beyond the
/// content of the page).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FrameKey {
    pub(crate) face: KeyFace,
    pub(crate) target: Option<KeyTarget>,
}

/// What a touch strip zone shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum ZoneContentFace {
    #[default]
    Blank,
    Lines(Vec<String>),
    Gauge(GaugeContent),
    Image(ImageSource),
}

impl ZoneContentFace {
    pub(crate) fn from_content(content: Option<ZoneContent>) -> Self {
        match content {
            Some(ZoneContent::Lines(mut lines)) => {
                lines.truncate(MAX_ZONE_LINES);
                Self::Lines(lines)
            }
            Some(ZoneContent::Gauge(gauge)) => Self::Gauge(gauge),
            Some(ZoneContent::Image(image)) => Self::Image(image_source(image)),
            // No zone, or a content added later that this version cannot draw.
            _ => Self::Blank,
        }
    }

    /// Text equivalent of the zone for the web UI: its lines, or the title
    /// and value of a gauge; empty for an image or a blank zone.
    pub(crate) fn text_lines(&self) -> Vec<String> {
        match self {
            Self::Lines(lines) => lines.clone(),
            Self::Gauge(gauge) => vec![gauge.title.clone(), gauge.value.clone()],
            Self::Blank | Self::Image(_) => Vec::new(),
        }
    }
}

/// A touch strip zone: its content and the transient outcome mark over it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ZoneFace {
    pub(crate) content: ZoneContentFace,
    pub(crate) feedback: Option<Feedback>,
}

impl ZoneFace {
    pub(crate) fn is_blank(&self) -> bool {
        self.content == ZoneContentFace::Blank && self.feedback.is_none()
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
    pub(crate) keys: Vec<FrameKey>,
    pub(crate) dials: Vec<DialFace>,
}

impl Frame {
    /// What key `key` acts on, if anything.
    pub(crate) fn target(&self, key: u8) -> Option<&KeyTarget> {
        self.keys.get(usize::from(key))?.target.as_ref()
    }
}

fn image_source(image: ImageData) -> ImageSource {
    // The renderer recognises the format from the content.
    ImageSource::Bytes(image.bytes)
}

#[cfg(test)]
mod tests {
    use hubergamof_module_api::GaugeLevel;

    use super::*;

    #[test]
    fn zones_keep_at_most_the_lines_the_api_allows() {
        let lines: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();
        let face = ZoneContentFace::from_content(Some(ZoneContent::Lines(lines.clone())));
        assert_eq!(
            face,
            ZoneContentFace::Lines(lines[..MAX_ZONE_LINES].to_vec())
        );
        assert_eq!(ZoneContentFace::from_content(None), ZoneContentFace::Blank);
    }

    #[test]
    fn gauges_are_kept_as_is() {
        let gauge = GaugeContent::new("Volume", "25 %", GaugeLevel::from_ratio(0.25));
        assert_eq!(
            ZoneContentFace::from_content(Some(ZoneContent::Gauge(gauge.clone()))),
            ZoneContentFace::Gauge(gauge.clone())
        );
        assert_eq!(
            ZoneContentFace::Gauge(gauge).text_lines(),
            ["Volume", "25 %"].map(String::from)
        );
    }
}
