//! Description of everything a device should show, compared between redraws
//! to rewrite only what changed.

use std::time::Duration;

use hubergamof_module_api::{
    ClockFace, Feedback, GaugeContent, ImageData, ItemId, KeyContent, KeyState, MAX_ZONE_LINES,
    ModuleInstanceId, PageId, PageZone, Rgb, UnreadNotification, ZoneContent,
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
    Clock(ClockFace),
    Page(PageZone),
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
            Some(ZoneContent::Clock(face)) => Self::Clock(face),
            Some(ZoneContent::Page(page)) => Self::Page(page),
            Some(ZoneContent::Image(image)) => Self::Image(image_source(image)),
            // No zone, or a content added later that this version cannot draw.
            _ => Self::Blank,
        }
    }

    /// Text equivalent of the zone for the web UI: its lines, the title and
    /// value of a gauge, the time and the date of a clock face, or the title
    /// and row of a page; empty for an image or a blank zone.
    pub(crate) fn text_lines(&self) -> Vec<String> {
        match self {
            Self::Lines(lines) => lines.clone(),
            Self::Gauge(gauge) => gauge_text_lines(gauge),
            Self::Clock(face) => clock_text_lines(face),
            Self::Page(page) => page_text_lines(page),
            Self::Blank | Self::Image(_) => Vec::new(),
        }
    }
}

/// Time (with seconds when displayed), then date and week of a clock face
/// as one line, like the zone it describes: `["14:05:09", "mer. 5 nov. · semaine 45"]`.
fn clock_text_lines(face: &ClockFace) -> Vec<String> {
    let time = match face.seconds {
        Some(seconds) => format!("{}:{:02}", face.time, seconds.get()),
        None => face.time.clone(),
    };
    let date = face
        .leaf
        .as_ref()
        .map(|leaf| format!("{} {} {}", leaf.weekday, leaf.day, leaf.month));
    let details: Vec<String> = date.into_iter().chain(face.caption.clone()).collect();
    let mut lines = vec![time];
    if !details.is_empty() {
        lines.push(details.join(" · "));
    }
    lines
}

/// Title (followed by the position in its steps, `Travail · 2/4`, when it has
/// some) and value of a gauge.
fn gauge_text_lines(gauge: &GaugeContent) -> Vec<String> {
    let title = match gauge.steps {
        Some(steps) => format!("{} · {}/{}", gauge.title, steps.current(), steps.total()),
        None => gauge.title.clone(),
    };
    vec![title, gauge.value.clone()]
}

/// Title of the page, then the row on display (`ligne 2/3`), when the page
/// has rows to scroll.
fn page_text_lines(page: &PageZone) -> Vec<String> {
    let mut lines = vec![page.title.clone()];
    if let Some(rows) = page.rows.filter(|rows| rows.is_scrollable()) {
        lines.push(rows.caption());
    }
    lines
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

/// A notification over the whole touch strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotificationFace {
    pub(crate) message: String,
    pub(crate) background: [u8; 3],
    pub(crate) icon: Option<ImageSource>,
    pub(crate) timeout: Option<Duration>,
    /// Time since it appeared, in animation steps (zero when nothing moves).
    pub(crate) elapsed: Duration,
}

impl NotificationFace {
    pub(crate) fn new(
        notification: &UnreadNotification,
        timeout: Option<Duration>,
        elapsed: Duration,
    ) -> Self {
        let Rgb(red, green, blue) = notification.background;
        Self {
            message: notification.message.clone(),
            background: [red, green, blue],
            icon: notification.icon.clone().map(image_source),
            timeout,
            elapsed,
        }
    }
}

/// What replaces the touch strip zones while it lasts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StripOverlay {
    /// A notification.
    Notification(NotificationFace),
    /// The label of the key being held.
    HeldLabel(String),
}

/// Everything the device shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frame {
    pub(crate) page: PageSummary,
    pub(crate) keys: Vec<FrameKey>,
    pub(crate) dials: Vec<DialFace>,
    pub(crate) overlay: Option<StripOverlay>,
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
    use hubergamof_module_api::{CalendarLeaf, GaugeLevel, GaugeSteps};

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

    #[test]
    fn gauges_with_steps_name_the_step_in_their_text() {
        let gauge = GaugeContent::new("Travail", "18:42", GaugeLevel::EMPTY)
            .with_steps(GaugeSteps::new(2, 4));
        assert_eq!(
            ZoneContentFace::Gauge(gauge).text_lines(),
            ["Travail · 2/4", "18:42"].map(String::from)
        );
    }

    #[test]
    fn page_zones_are_kept_and_described_as_text() {
        let page = PageZone::new("Conteneurs")
            .with_parent("Accueil")
            .with_rows(1, 3);
        let content = ZoneContentFace::from_content(Some(ZoneContent::Page(page.clone())));
        assert_eq!(content, ZoneContentFace::Page(page));
        assert_eq!(
            content.text_lines(),
            ["Conteneurs", "ligne 2/3"].map(String::from)
        );
        let home = ZoneContentFace::Page(PageZone::new("Accueil"));
        assert_eq!(home.text_lines(), ["Accueil"].map(String::from));
    }

    #[test]
    fn clock_faces_are_kept_and_described_as_text() {
        let face = ClockFace::new("14:05")
            .with_seconds(9)
            .with_leaf(CalendarLeaf::new("mer.", 5, "nov."))
            .with_caption("semaine 45");
        let content = ZoneContentFace::from_content(Some(ZoneContent::Clock(face.clone())));
        assert_eq!(content, ZoneContentFace::Clock(face));
        assert_eq!(
            content.text_lines(),
            ["14:05:09", "mer. 5 nov. · semaine 45"].map(String::from)
        );
        let bare = ZoneContentFace::Clock(ClockFace::new("07:30"));
        assert_eq!(bare.text_lines(), ["07:30"].map(String::from));
    }
}
