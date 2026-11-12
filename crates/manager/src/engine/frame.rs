//! Description of everything a device should show, compared between redraws
//! to rewrite only what changed.

use std::time::Duration;

use hubergamof_module_api::{
    Feedback, ImageData, ItemId, KeyContent, KeyState, ModuleInstanceId, PageId, Rgb,
    UnreadNotification,
};

use super::live::PageSummary;
use crate::render::ImageSource;
use crate::strip::StripFace;

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

/// What replaces the touch strip while it lasts.
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
    /// The touch strip, `None` on a device without one.
    pub(crate) strip: Option<StripFace>,
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
