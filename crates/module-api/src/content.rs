//! What a module shows: keys, pages and touch strip zones.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{ItemId, ModuleInstanceId, PageId};

/// Maximum number of text lines of a [`ZoneContent::Lines`] zone; extra lines
/// are not displayed.
pub const MAX_ZONE_LINES: usize = 3;

/// An encoded image supplied by a module: PNG, JPEG or SVG (uncompressed,
/// without external resources). The host recognises the format from the
/// content.
///
/// The host scales it to fit the target surface. The bytes are shared, so
/// returning the same `ImageData` on every call costs nothing; the host
/// compares contents to skip redrawing unchanged images.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageData {
    /// Encoded image.
    pub bytes: Arc<[u8]>,
}

impl ImageData {
    /// Wraps encoded bytes.
    #[must_use]
    pub fn new(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }
}

/// Visual state of a key, drawn by the host on top of its content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[non_exhaustive]
pub enum KeyState {
    /// Nothing special.
    #[default]
    Normal,
    /// On, running, selected (toggle on, timer running…).
    Active,
    /// Needs the user's attention (finished timer, failed container…).
    Attention,
}

/// Content of one key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyContent {
    /// Image, scaled to fit the key.
    pub image: Option<ImageData>,
    /// Text drawn at the bottom of the key.
    pub label: Option<String>,
    /// Visual state.
    pub state: KeyState,
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

    /// The same key in state `state`.
    #[must_use]
    pub fn in_state(self, state: KeyState) -> Self {
        Self { state, ..self }
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
#[non_exhaustive]
pub enum ZoneContent {
    /// Up to [`MAX_ZONE_LINES`] centred text lines, the first one larger.
    Lines(Vec<String>),
    /// An image scaled to fit the zone.
    Image(ImageData),
}

/// Navigation state of the device, given to [`crate::Module::zone`].
///
/// Most modules ignore it; it lets a module show where the user is (the
/// built-in navigation module does exactly that). Built by the host with
/// [`NavigationView::new`] and [`NavigationView::on_page`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NavigationView {
    /// Title of the page shown on the keys.
    pub page_title: String,
    /// Module instance owning the page shown on the keys, `None` on the home
    /// page.
    pub module: Option<ModuleInstanceId>,
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
    /// The home page titled `page_title`, scrolled to `row` of `row_count`
    /// rows, on a device showing `visible_rows` rows.
    #[must_use]
    pub fn new(page_title: impl Into<String>, row: u32, row_count: u32, visible_rows: u32) -> Self {
        Self {
            page_title: page_title.into(),
            module: None,
            page: None,
            row,
            row_count,
            visible_rows,
            depth: 0,
        }
    }

    /// The same view showing page `page` of `module`, `depth` pages above
    /// the home page.
    #[must_use]
    pub fn on_page(self, module: ModuleInstanceId, page: PageId, depth: usize) -> Self {
        Self {
            module: Some(module),
            page: Some(page),
            depth,
            ..self
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_views_describe_home_and_module_pages() {
        let home = NavigationView::new("Accueil", 0, 3, 2);
        assert!(home.is_home());
        assert!(home.is_scrollable());
        assert_eq!((home.module.as_ref(), home.page.as_ref()), (None, None));

        let page = NavigationView::new("Conteneurs", 1, 1, 2).on_page(
            "docker".into(),
            "containers".into(),
            1,
        );
        assert!(!page.is_home());
        assert!(!page.is_scrollable());
        assert_eq!(page.module, Some("docker".into()));
        assert_eq!(page.page, Some("containers".into()));
        assert_eq!(page.row, 1);
    }

    #[test]
    fn key_states_use_camel_case_on_the_wire() {
        assert_eq!(
            serde_json::to_value(KeyState::Attention).unwrap(),
            serde_json::json!("attention")
        );
        assert_eq!(KeyContent::default().state, KeyState::Normal);
        assert_eq!(
            KeyContent::with_label("x").in_state(KeyState::Active).state,
            KeyState::Active
        );
    }
}
