//! What a module shows: keys, pages and touch strip zones.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{ItemId, ModuleInstanceId, PageId, Rgb};

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
    /// A title, a large value and a level drawn as a bar, an arc or a ring
    /// (volume, timer progress…).
    Gauge(GaugeContent),
    /// A clock face: calendar leaf, time, optional seconds and week.
    Clock(ClockFace),
    /// Where the user is: page title, parent page, row and way back.
    Page(PageZone),
    /// A short menu browsed with the dial: a title and a few entries, one of
    /// them selected.
    Menu(MenuZone),
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

/// How a [`GaugeContent`] draws its level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GaugeShape {
    /// Title on top, value in the middle, horizontal bar at the bottom.
    #[default]
    Bar,
    /// Value inside a half circle filled from left to right, title at the
    /// bottom (volume).
    Arc,
    /// Glyph inside a ring filled clockwise from the top, title and value
    /// beside it (timer), and optionally the position in a series of steps as
    /// one dot each under them.
    Ring {
        /// The steps, `None` for no dots.
        steps: Option<GaugeSteps>,
    },
}

/// The colours a zone is drawn in: a diagonal gradient from [`Self::from`]
/// (light, top left) to [`Self::to`] (deep, bottom right). The module picks
/// them, usually from the tints of its icon; the host derives the neutrals
/// (tracks, captions) itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ZoneAccent {
    /// Colour at the start of the gradient.
    pub from: Rgb,
    /// Colour at the end of the gradient.
    pub to: Rgb,
}

impl ZoneAccent {
    /// The gradient from `from` to `to`.
    #[must_use]
    pub const fn new(from: Rgb, to: Rgb) -> Self {
        Self { from, to }
    }

    /// A single colour.
    #[must_use]
    pub const fn solid(colour: Rgb) -> Self {
        Self::new(colour, colour)
    }
}

impl Default for ZoneAccent {
    /// A blue.
    fn default() -> Self {
        Self::solid(Rgb(0x3B, 0xA1, 0xFF))
    }
}

/// Position in a series of steps (the pomodoros of a cycle), drawn as one dot
/// per step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GaugeSteps {
    current: u8,
    total: u8,
}

impl GaugeSteps {
    /// Most steps drawn.
    pub const MAX_TOTAL: u8 = 12;

    /// Step `current` (from 1) of `total`, both clamped: `total` within
    /// `1..=`[`Self::MAX_TOTAL`], `current` within `1..=total`.
    #[must_use]
    pub fn new(current: u32, total: u32) -> Self {
        let total = u8::try_from(total.clamp(1, u32::from(Self::MAX_TOTAL))).unwrap_or(1);
        let current = u8::try_from(current.clamp(1, u32::from(total))).unwrap_or(1);
        Self { current, total }
    }

    /// Current step, within `1..=`[`Self::total`].
    #[must_use]
    pub fn current(self) -> u8 {
        self.current
    }

    /// Number of steps, within `1..=`[`Self::MAX_TOTAL`].
    #[must_use]
    pub fn total(self) -> u8 {
        self.total
    }
}

/// A gauge drawn in a touch strip zone: [`Self::title`], [`Self::value`] and
/// a level, laid out according to [`Self::shape`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct GaugeContent {
    /// Short line (« Haut-parleurs », « Travail »).
    pub title: String,
    /// Large text (« 25 % », « 18:42 », « Muet »).
    pub value: String,
    /// Fill level.
    pub level: GaugeLevel,
    /// Drawn greyed out (muted output, paused timer): the fill, the value and
    /// the dots; the glyph keeps the colours of its own image.
    pub dimmed: bool,
    /// How the level is drawn.
    pub shape: GaugeShape,
    /// Colours of the filled part (every shape: the bar uses the light end).
    pub accent: ZoneAccent,
    /// Pictogram drawn by the host, in the colours of its own SVG or image:
    /// before the title of an arc, inside a ring. Not drawn by a bar.
    pub glyph: Option<ImageData>,
}

impl GaugeContent {
    /// A bar gauge titled `title` showing `value`, filled to `level`, not
    /// dimmed.
    #[must_use]
    pub fn new(title: impl Into<String>, value: impl Into<String>, level: GaugeLevel) -> Self {
        Self {
            title: title.into(),
            value: value.into(),
            level,
            dimmed: false,
            shape: GaugeShape::default(),
            accent: ZoneAccent::default(),
            glyph: None,
        }
    }

    /// The same gauge, greyed out when `dimmed`.
    #[must_use]
    pub fn with_dimmed(self, dimmed: bool) -> Self {
        Self { dimmed, ..self }
    }

    /// The same gauge drawn as `shape`.
    #[must_use]
    pub fn with_shape(self, shape: GaugeShape) -> Self {
        Self { shape, ..self }
    }

    /// The same gauge filled with `accent`.
    #[must_use]
    pub fn with_accent(self, accent: ZoneAccent) -> Self {
        Self { accent, ..self }
    }

    /// The same gauge with `glyph` (an SVG or a bitmap, scaled to fit).
    #[must_use]
    pub fn with_glyph(self, glyph: ImageData) -> Self {
        Self {
            glyph: Some(glyph),
            ..self
        }
    }

    /// The same gauge drawn as a ring showing its position in a series of
    /// steps.
    #[must_use]
    pub fn with_steps(self, steps: GaugeSteps) -> Self {
        self.with_shape(GaugeShape::Ring { steps: Some(steps) })
    }

    /// The steps drawn beside the ring, `None` for any other shape.
    #[must_use]
    pub fn steps(&self) -> Option<GaugeSteps> {
        match self.shape {
            GaugeShape::Ring { steps } => steps,
            _ => None,
        }
    }
}

/// Seconds of a minute shown by a [`ClockFace`], always within
/// `0..=`[`ClockSeconds::MAX`]: the constructor clamps out-of-range values, so
/// modules can pass raw seconds and the host can trust the value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClockSeconds(u8);

impl ClockSeconds {
    /// Seconds in a minute, the length of the progress track.
    pub const PER_MINUTE: u8 = 60;
    /// The last second of a minute.
    pub const MAX: u8 = Self::PER_MINUTE - 1;

    /// `seconds`, clamped to [`Self::MAX`].
    #[must_use]
    pub fn new(seconds: u8) -> Self {
        Self(seconds.min(Self::MAX))
    }

    /// The value, within `0..=`[`Self::MAX`].
    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }
}

/// A clock face drawn in a touch strip zone: an optional calendar leaf on the
/// left, the time on the right, the seconds as a superscript with a
/// progress track under the time, and a caption (the week).
///
/// The module supplies the texts, in their natural case; the host does the
/// layout and the typography (capitals on the leaf).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ClockFace {
    /// Hours and minutes (« 14:05 »).
    pub time: String,
    /// Seconds of the minute, when displayed.
    pub seconds: Option<ClockSeconds>,
    /// Calendar leaf, when the date is displayed.
    pub leaf: Option<CalendarLeaf>,
    /// Short text under the time (« semaine 45 »).
    pub caption: Option<String>,
}

impl ClockFace {
    /// A face showing `time` only.
    #[must_use]
    pub fn new(time: impl Into<String>) -> Self {
        Self {
            time: time.into(),
            seconds: None,
            leaf: None,
            caption: None,
        }
    }

    /// The same face showing `seconds`, clamped to [`ClockSeconds::MAX`].
    #[must_use]
    pub fn with_seconds(self, seconds: u8) -> Self {
        Self {
            seconds: Some(ClockSeconds::new(seconds)),
            ..self
        }
    }

    /// The same face with a calendar leaf.
    #[must_use]
    pub fn with_leaf(self, leaf: CalendarLeaf) -> Self {
        Self {
            leaf: Some(leaf),
            ..self
        }
    }

    /// The same face with a caption under the time.
    #[must_use]
    pub fn with_caption(self, caption: impl Into<String>) -> Self {
        Self {
            caption: Some(caption.into()),
            ..self
        }
    }
}

/// The calendar leaf of a [`ClockFace`]: weekday band, day of the month,
/// month. The host decides how the texts are displayed (the built-in renderer
/// puts the weekday and the month in capitals).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct CalendarLeaf {
    /// Abbreviated weekday in the band (« mer. »).
    pub weekday: String,
    /// Day of the month, from 1.
    pub day: u8,
    /// Abbreviated month under the day (« nov. »).
    pub month: String,
}

impl CalendarLeaf {
    /// A leaf for `day` of `month`, under the `weekday` band.
    #[must_use]
    pub fn new(weekday: impl Into<String>, day: u8, month: impl Into<String>) -> Self {
        Self {
            weekday: weekday.into(),
            day,
            month: month.into(),
        }
    }
}

/// Position of the visible rows among the rows of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowPosition {
    row: u32,
    count: u32,
}

impl RowPosition {
    /// First visible row `row` (from 0) of `count` rows: `count` is clamped to
    /// at least 1 and `row` to the last row.
    #[must_use]
    pub fn new(row: u32, count: u32) -> Self {
        let count = count.max(1);
        Self {
            row: row.min(count - 1),
            count,
        }
    }

    /// Index of the first visible row, from 0, less than [`Self::count`].
    #[must_use]
    pub fn row(self) -> u32 {
        self.row
    }

    /// Number of rows of the page, at least 1.
    #[must_use]
    pub fn count(self) -> u32 {
        self.count
    }

    /// Whether there is more than one row to move between.
    #[must_use]
    pub fn is_scrollable(self) -> bool {
        self.count > 1
    }

    /// The caption of the position: `ligne 2/3`.
    #[must_use]
    pub fn caption(self) -> String {
        format!("ligne {}/{}", self.row + 1, self.count)
    }
}

/// The touch strip zone of the navigation dial: the page shown, the page it
/// leads back to and the row on display.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PageZone {
    /// Title of the page shown on the keys.
    pub title: String,
    /// Title of the page the dial leads back to, `None` on the home page.
    pub parent: Option<String>,
    /// Rows of the page, `None` when there is nothing to scroll.
    pub rows: Option<RowPosition>,
    /// Colours of the back hint, the parent page and the current row.
    pub accent: ZoneAccent,
}

impl PageZone {
    /// A zone titled `title`, without parent page nor rows.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            parent: None,
            rows: None,
            accent: ZoneAccent::default(),
        }
    }

    /// The same zone leading back to the page titled `parent`.
    #[must_use]
    pub fn with_parent(self, parent: impl Into<String>) -> Self {
        Self {
            parent: Some(parent.into()),
            ..self
        }
    }

    /// The same zone showing row `row` (from 0) of `count` rows.
    #[must_use]
    pub fn with_rows(self, row: u32, count: u32) -> Self {
        Self {
            rows: Some(RowPosition::new(row, count)),
            ..self
        }
    }

    /// The same zone drawn with `accent`.
    #[must_use]
    pub fn with_accent(self, accent: ZoneAccent) -> Self {
        Self { accent, ..self }
    }

    /// Whether pressing the dial goes back to another page: there is a
    /// parent page.
    #[must_use]
    pub fn can_go_back(&self) -> bool {
        self.parent.is_some()
    }
}

/// Most entries a [`MenuZone`] shows: more would not fit the zone.
pub const MAX_MENU_ENTRIES: usize = 3;

/// The touch strip zone of a dial menu: a title (the state the menu acts on)
/// and up to [`MAX_MENU_ENTRIES`] entries, the selected one highlighted.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct MenuZone {
    title: String,
    entries: Vec<String>,
    selected: usize,
    accent: ZoneAccent,
}

impl MenuZone {
    /// A menu titled `title` with `entries` (the first [`MAX_MENU_ENTRIES`]),
    /// the entry `selected` highlighted (clamped to the last entry).
    #[must_use]
    pub fn new(title: impl Into<String>, entries: Vec<String>, selected: usize) -> Self {
        let mut entries = entries;
        entries.truncate(MAX_MENU_ENTRIES);
        Self {
            title: title.into(),
            selected: selected.min(entries.len().saturating_sub(1)),
            entries,
            accent: ZoneAccent::default(),
        }
    }

    /// The same menu drawn with `accent`.
    #[must_use]
    pub fn with_accent(self, accent: ZoneAccent) -> Self {
        Self { accent, ..self }
    }

    /// Title, above the entries.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Entries, in display order.
    #[must_use]
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Index of the highlighted entry, within [`Self::entries`].
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Colours of the title and of the highlighted entry.
    #[must_use]
    pub fn accent(&self) -> ZoneAccent {
        self.accent
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
    /// Title of the page below the one shown (the home page, or the page that
    /// opened it), `None` on the home page.
    pub parent_title: Option<String>,
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
            parent_title: None,
        }
    }

    /// The same view leading back to the page titled `parent_title`.
    #[must_use]
    pub fn with_parent_title(self, parent_title: impl Into<String>) -> Self {
        Self {
            parent_title: Some(parent_title.into()),
            ..self
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
    fn clock_seconds_are_clamped_to_the_minute() {
        assert_eq!(ClockSeconds::new(42).get(), 42);
        assert_eq!(ClockSeconds::new(ClockSeconds::MAX).get(), 59);
        assert_eq!(ClockSeconds::new(60).get(), 59);
        assert_eq!(ClockSeconds::new(u8::MAX).get(), 59);
        assert_eq!(ClockFace::new("14:05").seconds, None);
        assert_eq!(
            ClockFace::new("14:05").with_seconds(42).seconds,
            Some(ClockSeconds::new(42))
        );
        assert_eq!(
            ClockFace::new("14:05").with_seconds(200).seconds,
            Some(ClockSeconds::new(59))
        );
    }

    #[test]
    fn clock_faces_carry_their_leaf_and_caption() {
        let face = ClockFace::new("14:05")
            .with_leaf(CalendarLeaf::new("mer.", 5, "nov."))
            .with_caption("semaine 45");
        assert_eq!(face.leaf.as_ref().map(|leaf| leaf.day), Some(5));
        assert_eq!(face.caption.as_deref(), Some("semaine 45"));
    }

    #[test]
    fn gauge_steps_are_clamped_to_the_series() {
        let steps = GaugeSteps::new(2, 4);
        assert_eq!((steps.current(), steps.total()), (2, 4));
        let steps = GaugeSteps::new(9, 4);
        assert_eq!((steps.current(), steps.total()), (4, 4));
        let steps = GaugeSteps::new(0, 0);
        assert_eq!((steps.current(), steps.total()), (1, 1));
        let steps = GaugeSteps::new(5, 500);
        assert_eq!(steps.total(), GaugeSteps::MAX_TOTAL);
    }

    #[test]
    fn gauges_start_as_blue_bars() {
        let gauge = GaugeContent::new("Volume", "25 %", GaugeLevel::EMPTY);
        assert_eq!(gauge.shape, GaugeShape::Bar);
        assert_eq!(gauge.accent, ZoneAccent::solid(Rgb(0x3B, 0xA1, 0xFF)));
        assert!(gauge.glyph.is_none() && gauge.steps().is_none());
        let ring = gauge.with_steps(GaugeSteps::new(1, 4));
        assert!(matches!(ring.shape, GaugeShape::Ring { steps: Some(_) }));
        assert_eq!(ring.steps().map(GaugeSteps::total), Some(4));
        let bar = GaugeContent::new("Volume", "25 %", GaugeLevel::EMPTY);
        assert_eq!(bar.with_shape(GaugeShape::Arc).steps(), None);
    }

    #[test]
    fn page_zones_know_their_parent_and_rows() {
        let home = PageZone::new("Accueil");
        assert!(!home.can_go_back() && home.parent.is_none() && home.rows.is_none());
        let page = PageZone::new("Conteneurs")
            .with_parent("Accueil")
            .with_rows(1, 0);
        assert!(page.can_go_back());
        assert_eq!(page.parent.as_deref(), Some("Accueil"));
        assert_eq!(page.rows, Some(RowPosition::new(0, 1)));
        let beyond = PageZone::new("Accueil").with_rows(7, 3);
        let rows = beyond.rows.unwrap();
        assert_eq!((rows.row(), rows.count()), (2, 3));
        assert_eq!(rows.caption(), "ligne 3/3");
        assert!(rows.is_scrollable() && !RowPosition::new(0, 1).is_scrollable());
    }

    #[test]
    fn menu_zones_keep_a_few_entries_and_a_valid_selection() {
        let entries = |count: usize| (0..count).map(|index| index.to_string()).collect();
        let menu = MenuZone::new("Travail", entries(5), 4);
        assert_eq!(menu.entries().len(), MAX_MENU_ENTRIES);
        assert_eq!(menu.selected(), MAX_MENU_ENTRIES - 1);
        assert_eq!(menu.title(), "Travail");
        assert_eq!(MenuZone::new("", entries(0), 3).selected(), 0);
        let accent = ZoneAccent::solid(Rgb(1, 2, 3));
        assert_eq!(menu.with_accent(accent).accent(), accent);
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
