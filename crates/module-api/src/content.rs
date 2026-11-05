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
    /// On, running, selected (toggle on, timer running…). The host draws
    /// nothing for it: the module shows it through its own icon.
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

    /// A key showing `image` over `label`.
    #[must_use]
    pub fn with_icon(image: &ImageData, label: impl Into<String>) -> Self {
        Self {
            image: Some(image.clone()),
            label: Some(label.into()),
            state: KeyState::Normal,
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
    /// A title, a large value and a horizontal level bar (volume, timer
    /// progress…).
    Gauge(GaugeContent),
}

/// Fill level of a [`GaugeContent`], in thousandths of the full bar.
///
/// Always within `0..=`[`GaugeLevel::FULL_PERMILLE`]: the constructors clamp
/// out-of-range values, so modules can pass raw ratios.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GaugeLevel(u16);

impl GaugeLevel {
    /// Thousandths of a full bar.
    pub const FULL_PERMILLE: u16 = 1000;
    /// An empty bar.
    pub const EMPTY: Self = Self(0);
    /// A full bar.
    pub const FULL: Self = Self(Self::FULL_PERMILLE);

    /// The level of `ratio` (0 = empty, 1 = full), clamped to `0..=1`
    /// and rounded to the nearest thousandth; `NaN` gives an empty bar.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn from_ratio(ratio: f32) -> Self {
        if ratio.is_nan() {
            return Self::EMPTY;
        }
        let permille = (ratio.clamp(0.0, 1.0) * f32::from(Self::FULL_PERMILLE)).round();
        // Within 0..=FULL_PERMILLE after the clamp: the cast is exact.
        Self(permille as u16)
    }

    /// The level of `permille` thousandths, clamped to a full bar.
    #[must_use]
    pub fn from_permille(permille: u16) -> Self {
        Self(permille.min(Self::FULL_PERMILLE))
    }

    /// The level of `part` out of `whole` (`whole = 0` gives an empty bar).
    #[must_use]
    pub fn from_fraction(part: u64, whole: u64) -> Self {
        if whole == 0 {
            return Self::EMPTY;
        }
        let permille =
            u128::from(part.min(whole)) * u128::from(Self::FULL_PERMILLE) / u128::from(whole);
        Self::from_permille(u16::try_from(permille).unwrap_or(Self::FULL_PERMILLE))
    }

    /// Thousandths of a full bar, within `0..=`[`Self::FULL_PERMILLE`].
    #[must_use]
    pub fn permille(self) -> u16 {
        self.0
    }

    /// The level as a ratio within `0..=1`.
    #[must_use]
    pub fn ratio(self) -> f32 {
        f32::from(self.0) / f32::from(Self::FULL_PERMILLE)
    }
}

/// A gauge drawn in a touch strip zone: [`Self::title`] on top,
/// [`Self::value`] in large text, a bar filled to [`Self::level`] at the
/// bottom.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct GaugeContent {
    /// Top line (« Volume », « Travail · 2/4 »).
    pub title: String,
    /// Large central text (« 25 % », « 18:42 », « Muet »).
    pub value: String,
    /// Fill level of the bar.
    pub level: GaugeLevel,
    /// Drawn greyed out (muted output, paused timer).
    pub dimmed: bool,
}

impl GaugeContent {
    /// A gauge titled `title` showing `value`, filled to `level`, not dimmed.
    #[must_use]
    pub fn new(title: impl Into<String>, value: impl Into<String>, level: GaugeLevel) -> Self {
        Self {
            title: title.into(),
            value: value.into(),
            level,
            dimmed: false,
        }
    }

    /// The same gauge, greyed out when `dimmed`.
    #[must_use]
    pub fn with_dimmed(self, dimmed: bool) -> Self {
        Self { dimmed, ..self }
    }
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
    fn gauge_levels_are_clamped_to_the_bar() {
        assert_eq!(GaugeLevel::from_ratio(0.25).permille(), 250);
        assert_eq!(GaugeLevel::from_ratio(0.123_4).permille(), 123);
        assert_eq!(GaugeLevel::from_ratio(-0.5), GaugeLevel::EMPTY);
        assert_eq!(GaugeLevel::from_ratio(1.5), GaugeLevel::FULL);
        assert_eq!(GaugeLevel::from_ratio(f32::NAN), GaugeLevel::EMPTY);
        assert_eq!(GaugeLevel::from_ratio(f32::INFINITY), GaugeLevel::FULL);
        assert_eq!(GaugeLevel::from_permille(1500), GaugeLevel::FULL);
        assert_eq!(GaugeLevel::from_fraction(1, 3).permille(), 333);
        assert_eq!(GaugeLevel::from_fraction(5, 0), GaugeLevel::EMPTY);
        assert_eq!(GaugeLevel::from_fraction(u64::MAX, 2), GaugeLevel::FULL);
        assert!((GaugeLevel::from_permille(500).ratio() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn gauges_are_not_dimmed_by_default() {
        let gauge = GaugeContent::new("Volume", "25 %", GaugeLevel::from_ratio(0.25));
        assert!(!gauge.dimmed);
        assert_eq!(gauge.title, "Volume");
        let muted = gauge.clone().with_dimmed(true);
        assert!(muted.dimmed);
        assert_ne!(gauge, muted);
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
