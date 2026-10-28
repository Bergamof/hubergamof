//! What a module shows: keys, pages and touch strip zones.

use std::sync::Arc;

use crate::{ItemId, PageId};

/// Maximum number of text lines of a [`ZoneContent::Lines`] zone; extra lines
/// are not displayed.
pub const MAX_ZONE_LINES: usize = 3;

/// Encoding of an [`ImageData`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    /// Portable Network Graphics.
    Png,
    /// JPEG.
    Jpeg,
    /// Scalable Vector Graphics (uncompressed, without external resources).
    Svg,
}

/// An encoded image supplied by a module.
///
/// The host scales it to fit the target surface. The bytes are shared, so
/// returning the same `ImageData` on every call costs nothing; the host
/// compares contents to skip redrawing unchanged images.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageData {
    /// Encoded image.
    pub bytes: Arc<[u8]>,
    /// Encoding of `bytes`.
    pub format: ImageFormat,
}

impl ImageData {
    /// Wraps encoded bytes.
    #[must_use]
    pub fn new(bytes: impl Into<Arc<[u8]>>, format: ImageFormat) -> Self {
        Self {
            bytes: bytes.into(),
            format,
        }
    }
}

/// Content of one key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyContent {
    /// Image, scaled to fit the key.
    pub image: Option<ImageData>,
    /// Text drawn at the bottom of the key.
    pub label: Option<String>,
    /// Whether the key is in its active state (toggle on, timer running…).
    pub active: bool,
}

impl KeyContent {
    /// A key showing only `label`.
    #[must_use]
    pub fn with_label(label: impl Into<String>) -> Self {
        Self {
            label: Some(label.into()),
            ..Self::default()
        }
    }
}

/// One key of a module page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageItem {
    /// Identifier passed back to [`crate::Module::on_item_press`].
    pub id: ItemId,
    /// What the key shows.
    pub key: KeyContent,
}

/// A page opened by a module with [`crate::Command::OpenPage`].
///
/// Items fill the keys row by row; when they do not fit, the host lets the
/// user scroll by rows ([`crate::Command::ScrollRows`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageContent {
    /// Title, shown in the UI and by the navigation module.
    pub title: String,
    /// Keys, in display order.
    pub items: Vec<PageItem>,
}

/// Content of the touch strip zone above a dial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneContent {
    /// Up to [`MAX_ZONE_LINES`] centred text lines, the first one larger.
    Lines(Vec<String>),
    /// An image scaled to fit the zone.
    Image(ImageData),
}

/// Navigation state of the device, given to [`crate::Module::zone`].
///
/// Most modules ignore it; it lets a module show where the user is (the
/// built-in navigation module does exactly that).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationView {
    /// Title of the page shown on the keys.
    pub page_title: String,
    /// Page of the module stack shown on the keys, `None` on the home page.
    pub page: Option<PageId>,
    /// Index of the first visible row, from 0.
    pub row: u32,
    /// Number of rows of the page (at least 1).
    pub row_count: u32,
    /// Number of key rows of the device.
    pub visible_rows: u32,
    /// Number of pages above the home page (0 on the home page).
    pub depth: usize,
}

impl NavigationView {
    /// Whether the home page is displayed.
    #[must_use]
    pub fn is_home(&self) -> bool {
        self.depth == 0
    }

    /// Whether the page has more rows than the device shows.
    #[must_use]
    pub fn is_scrollable(&self) -> bool {
        self.row_count > self.visible_rows
    }
}
