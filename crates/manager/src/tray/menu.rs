//! Tray menu model, independent of D-Bus: entries, stable ids, layout
//! revision and the `com.canonical.dbusmenu` item properties.
//!
//! Entries: open the web UI, the theme submenu (light, dark: the current one is
//! marked in its label), separator, the connected devices (informative,
//! disabled), separator, quit.

use hubergamof_protocol::DeviceTheme;

/// Menu entry opening the web UI.
pub const OPEN_UI_LABEL: &str = "Ouvrir l'interface";

/// Menu entry stopping the manager.
pub const QUIT_LABEL: &str = "Quitter";

/// Submenu choosing the theme of the devices.
pub const THEME_LABEL: &str = "Thème";

/// Theme entry of the light theme.
pub const THEME_LIGHT_LABEL: &str = "Clair";

/// Theme entry of the dark theme.
pub const THEME_DARK_LABEL: &str = "Sombre";

/// Shown in the menu when no device is connected.
pub const NO_DEVICE_LABEL: &str = "Aucun appareil connecté";

/// Id of the (invisible) root item, fixed by the dbusmenu protocol.
pub const ROOT_ID: i32 = 0;
/// Id of « Ouvrir l'interface ».
pub const OPEN_UI_ID: i32 = 1;
/// Id of the separator below « Ouvrir l'interface ».
const TOP_SEPARATOR_ID: i32 = 2;
/// Id of the separator above « Quitter ».
const BOTTOM_SEPARATOR_ID: i32 = 3;
/// Id of « Quitter ».
pub const QUIT_ID: i32 = 4;
/// Id of the « Thème » submenu.
pub const THEME_ID: i32 = 5;
/// Id of « Clair », in the theme submenu.
pub const THEME_LIGHT_ID: i32 = 6;
/// Id of « Sombre », in the theme submenu.
pub const THEME_DARK_ID: i32 = 7;
/// Id of the first device line (or of the « no device » line); the next ones
/// follow.
pub const FIRST_DEVICE_ID: i32 = 100;

/// The entries of the theme submenu.
const THEME_CHOICE_IDS: [i32; 2] = [THEME_LIGHT_ID, THEME_DARK_ID];

/// Before the label of the theme in use.
pub const SELECTED_MARK: &str = "●";
/// Before the label of the other theme.
pub const UNSELECTED_MARK: &str = "○";

const OPEN_UI_ICON: &str = "internet-web-browser";
const QUIT_ICON: &str = "application-exit";
const THEME_ICON: &str = "preferences-desktop-theme";

/// Value of a dbusmenu item property (`a{sv}` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyValue {
    /// A string (`s`).
    Text(String),
    /// A boolean (`b`).
    Flag(bool),
}

/// One property of a menu item: its dbusmenu name and value.
pub type Property = (&'static str, PropertyValue);

/// A menu item and its children, as returned by `GetLayout`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutNode {
    /// Item id.
    pub id: i32,
    /// Properties differing from the dbusmenu defaults, filtered.
    pub properties: Vec<Property>,
    /// Children, empty past the requested depth.
    pub children: Vec<LayoutNode>,
}

/// What clicking an item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Opens the web UI.
    OpenUi,
    /// Applies a theme to the connected devices.
    SetTheme(DeviceTheme),
    /// Stops the manager.
    Quit,
}

/// The tray menu: device lines plus the layout revision announced through
/// `LayoutUpdated`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayMenu {
    devices: Vec<String>,
    /// Theme shared by all the devices, `None` when they differ or none is
    /// connected.
    theme: Option<DeviceTheme>,
    revision: u32,
}

impl TrayMenu {
    /// Menu listing `devices` (labels, see [`super::device_labels`]).
    #[must_use]
    pub fn new(devices: Vec<String>) -> Self {
        Self {
            devices,
            theme: None,
            revision: 1,
        }
    }

    /// Current layout revision.
    #[must_use]
    pub fn revision(&self) -> u32 {
        self.revision
    }

    /// Replaces the device lines; returns `true` (and bumps the revision)
    /// when they changed.
    pub fn set_devices(&mut self, devices: Vec<String>) -> bool {
        if devices == self.devices {
            return false;
        }
        self.devices = devices;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    /// Sets the theme marked as current; returns `true` (and bumps the
    /// revision) when it changed.
    pub fn set_theme(&mut self, theme: Option<DeviceTheme>) -> bool {
        if theme == self.theme {
            return false;
        }
        self.theme = theme;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    /// Layout below `parent_id`, `depth` levels deep (`None`: unlimited),
    /// keeping only the properties named in `filter` (all when empty).
    /// `None` when `parent_id` is unknown.
    #[must_use]
    pub fn layout(
        &self,
        parent_id: i32,
        depth: Option<usize>,
        filter: &[String],
    ) -> Option<LayoutNode> {
        self.node(parent_id, depth, filter)
    }

    /// Properties of item `id` filtered by `filter` (all when empty); `None`
    /// when the item does not exist.
    #[must_use]
    pub fn properties(&self, id: i32, filter: &[String]) -> Option<Vec<Property>> {
        if id == ROOT_ID {
            return Some(filtered(root_properties(), filter));
        }
        self.item_properties(id)
            .map(|properties| filtered(properties, filter))
    }

    /// Every item with its filtered properties (`GetGroupProperties` with no
    /// id).
    #[must_use]
    pub fn all_properties(&self, filter: &[String]) -> Vec<(i32, Vec<Property>)> {
        std::iter::once(ROOT_ID)
            .chain(self.child_ids())
            .chain(THEME_CHOICE_IDS)
            .filter_map(|id| {
                self.properties(id, filter)
                    .map(|properties| (id, properties))
            })
            .collect()
    }

    /// Whether item `id` exists.
    #[must_use]
    pub fn contains(&self, id: i32) -> bool {
        id == ROOT_ID || self.item_properties(id).is_some()
    }

    /// Whether the children of `id` change while the menu lives, so that a
    /// host showing it must ask for them again.
    #[must_use]
    pub fn changes_while_open(id: i32) -> bool {
        id == THEME_ID
    }

    /// Action run by a click on `id`; `None` for the root, separators and the
    /// (disabled) device lines.
    #[must_use]
    pub fn action(id: i32) -> Option<MenuAction> {
        match id {
            OPEN_UI_ID => Some(MenuAction::OpenUi),
            THEME_LIGHT_ID => Some(MenuAction::SetTheme(DeviceTheme::Light)),
            THEME_DARK_ID => Some(MenuAction::SetTheme(DeviceTheme::Dark)),
            QUIT_ID => Some(MenuAction::Quit),
            _ => None,
        }
    }

    fn device_lines(&self) -> Vec<&str> {
        if self.devices.is_empty() {
            vec![NO_DEVICE_LABEL]
        } else {
            self.devices.iter().map(String::as_str).collect()
        }
    }

    fn device_ids(&self) -> impl Iterator<Item = i32> {
        (FIRST_DEVICE_ID..).take(self.device_lines().len())
    }

    fn child_ids(&self) -> Vec<i32> {
        let mut ids = vec![OPEN_UI_ID, THEME_ID, TOP_SEPARATOR_ID];
        ids.extend(self.device_ids());
        ids.extend([BOTTOM_SEPARATOR_ID, QUIT_ID]);
        ids
    }

    /// Items directly below `id`.
    fn children_of(&self, id: i32) -> Vec<i32> {
        match id {
            ROOT_ID => self.child_ids(),
            THEME_ID => THEME_CHOICE_IDS.to_vec(),
            _ => Vec::new(),
        }
    }

    /// Item `id` with its descendants, `depth` levels deep (`None`:
    /// unlimited).
    fn node(&self, id: i32, depth: Option<usize>, filter: &[String]) -> Option<LayoutNode> {
        let properties = self.properties(id, filter)?;
        let children = match depth {
            Some(0) => Vec::new(),
            _ => self
                .children_of(id)
                .into_iter()
                .filter_map(|child| self.node(child, depth.map(|depth| depth - 1), filter))
                .collect(),
        };
        Some(LayoutNode {
            id,
            properties,
            children,
        })
    }

    fn theme_properties(&self, id: i32) -> Option<Vec<Property>> {
        let (label, theme) = match id {
            THEME_LIGHT_ID => (THEME_LIGHT_LABEL, DeviceTheme::Light),
            THEME_DARK_ID => (THEME_DARK_LABEL, DeviceTheme::Dark),
            _ => return None,
        };
        let mark = if self.theme == Some(theme) {
            SELECTED_MARK
        } else {
            UNSELECTED_MARK
        };
        // The mark is part of the label: neither an icon name nor a radio
        // toggle is drawn by every tray host.
        Some(vec![
            ("label", PropertyValue::Text(format!("{mark} {label}"))),
            ("enabled", PropertyValue::Flag(self.has_devices())),
        ])
    }

    fn has_devices(&self) -> bool {
        !self.devices.is_empty()
    }

    fn item_properties(&self, id: i32) -> Option<Vec<Property>> {
        match id {
            OPEN_UI_ID => Some(action_properties(OPEN_UI_LABEL, OPEN_UI_ICON)),
            QUIT_ID => Some(action_properties(QUIT_LABEL, QUIT_ICON)),
            THEME_ID => {
                let mut properties = action_properties(THEME_LABEL, THEME_ICON);
                properties.push(("children-display", PropertyValue::Text("submenu".into())));
                properties.push(("enabled", PropertyValue::Flag(self.has_devices())));
                Some(properties)
            }
            THEME_LIGHT_ID | THEME_DARK_ID => self.theme_properties(id),
            TOP_SEPARATOR_ID | BOTTOM_SEPARATOR_ID => {
                Some(vec![("type", PropertyValue::Text("separator".into()))])
            }
            _ => {
                let index = usize::try_from(id.checked_sub(FIRST_DEVICE_ID)?).ok()?;
                let label = self.device_lines().get(index).copied()?;
                Some(vec![
                    ("label", PropertyValue::Text(escape_mnemonics(label))),
                    ("enabled", PropertyValue::Flag(false)),
                ])
            }
        }
    }
}

fn root_properties() -> Vec<Property> {
    vec![("children-display", PropertyValue::Text("submenu".into()))]
}

fn action_properties(label: &str, icon: &str) -> Vec<Property> {
    vec![
        ("label", PropertyValue::Text(label.into())),
        ("icon-name", PropertyValue::Text(icon.into())),
    ]
}

fn filtered(properties: Vec<Property>, filter: &[String]) -> Vec<Property> {
    if filter.is_empty() {
        return properties;
    }
    properties
        .into_iter()
        .filter(|(name, _)| filter.iter().any(|wanted| wanted == name))
        .collect()
}

/// dbusmenu labels use `_` to mark access keys: device names and serials are
/// shown verbatim.
fn escape_mnemonics(label: &str) -> String {
    label.replace('_', "__")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(node: &LayoutNode) -> Option<&str> {
        node.properties
            .iter()
            .find_map(|(name, value)| match (name, value) {
                (&"label", PropertyValue::Text(label)) => Some(label.as_str()),
                _ => None,
            })
    }

    fn is_enabled(node: &LayoutNode) -> bool {
        !node
            .properties
            .contains(&("enabled", PropertyValue::Flag(false)))
    }

    fn is_separator(node: &LayoutNode) -> bool {
        node.properties
            .contains(&("type", PropertyValue::Text("separator".into())))
    }

    fn summary(menu: &TrayMenu) -> Vec<(i32, String, bool)> {
        menu.layout(ROOT_ID, None, &[])
            .unwrap()
            .children
            .iter()
            .map(|node| {
                let text = if is_separator(node) {
                    "-".to_owned()
                } else {
                    label(node).unwrap().to_owned()
                };
                (node.id, text, is_enabled(node))
            })
            .collect()
    }

    #[test]
    fn layout_lists_devices_between_open_and_quit() {
        let menu = TrayMenu::new(vec!["Deck (A1)".into(), "Deck (B2)".into()]);
        assert_eq!(
            summary(&menu),
            [
                (OPEN_UI_ID, OPEN_UI_LABEL.into(), true),
                (THEME_ID, THEME_LABEL.into(), true),
                (TOP_SEPARATOR_ID, "-".into(), true),
                (FIRST_DEVICE_ID, "Deck (A1)".into(), false),
                (FIRST_DEVICE_ID + 1, "Deck (B2)".into(), false),
                (BOTTOM_SEPARATOR_ID, "-".into(), true),
                (QUIT_ID, QUIT_LABEL.into(), true),
            ]
        );
    }

    #[test]
    fn layout_without_device_shows_a_disabled_placeholder() {
        let menu = TrayMenu::new(Vec::new());
        assert_eq!(
            summary(&menu)[3],
            (FIRST_DEVICE_ID, NO_DEVICE_LABEL.into(), false)
        );
    }

    #[test]
    fn root_declares_a_submenu_and_respects_depth() {
        let menu = TrayMenu::new(Vec::new());
        let root = menu.layout(ROOT_ID, Some(0), &[]).unwrap();
        assert_eq!(
            root.properties,
            [("children-display", PropertyValue::Text("submenu".into()))]
        );
        assert_eq!(root.children, []);
        assert_eq!(
            menu.layout(ROOT_ID, Some(1), &[]).unwrap().children.len(),
            6
        );
    }

    #[test]
    fn layout_of_an_item_and_of_an_unknown_id() {
        let menu = TrayMenu::new(Vec::new());
        let quit = menu.layout(QUIT_ID, None, &[]).unwrap();
        assert_eq!(quit.id, QUIT_ID);
        assert_eq!(label(&quit), Some(QUIT_LABEL));
        assert!(menu.layout(FIRST_DEVICE_ID + 1, None, &[]).is_none());
        assert!(menu.layout(-1, None, &[]).is_none());
        assert!(!menu.contains(42));
    }

    #[test]
    fn property_filter_keeps_only_requested_names() {
        let menu = TrayMenu::new(Vec::new());
        assert_eq!(
            menu.properties(OPEN_UI_ID, &["label".into()]).unwrap(),
            [("label", PropertyValue::Text(OPEN_UI_LABEL.into()))]
        );
        assert_eq!(menu.properties(QUIT_ID, &["visible".into()]).unwrap(), []);
        let all = menu.all_properties(&[]);
        assert_eq!(
            all.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [
                ROOT_ID,
                OPEN_UI_ID,
                THEME_ID,
                TOP_SEPARATOR_ID,
                FIRST_DEVICE_ID,
                BOTTOM_SEPARATOR_ID,
                QUIT_ID,
                THEME_LIGHT_ID,
                THEME_DARK_ID
            ]
        );
    }

    #[test]
    fn ids_of_open_and_quit_stay_stable_when_devices_change() {
        let mut menu = TrayMenu::new(Vec::new());
        assert!(menu.set_devices(vec!["A".into(), "B".into(), "C".into()]));
        let ids: Vec<i32> = summary(&menu).into_iter().map(|(id, ..)| id).collect();
        assert_eq!(ids.first(), Some(&OPEN_UI_ID));
        assert_eq!(ids.last(), Some(&QUIT_ID));
        assert_eq!(TrayMenu::action(OPEN_UI_ID), Some(MenuAction::OpenUi));
        assert_eq!(TrayMenu::action(QUIT_ID), Some(MenuAction::Quit));
        assert_eq!(TrayMenu::action(FIRST_DEVICE_ID), None);
        assert_eq!(TrayMenu::action(ROOT_ID), None);
    }

    fn theme_entries(menu: &TrayMenu) -> Vec<(String, bool, bool)> {
        let submenu = menu.layout(THEME_ID, None, &[]).unwrap();
        submenu
            .children
            .iter()
            .map(|node| {
                let (mark, name) = label(node).unwrap().split_once(' ').unwrap();
                (name.to_owned(), mark == SELECTED_MARK, is_enabled(node))
            })
            .collect()
    }

    #[test]
    fn the_theme_submenu_offers_both_themes() {
        let menu = TrayMenu::new(vec!["A".into()]);
        let submenu = menu.layout(THEME_ID, None, &[]).unwrap();
        assert!(
            submenu
                .properties
                .contains(&("children-display", PropertyValue::Text("submenu".into())))
        );
        assert_eq!(
            submenu
                .children
                .iter()
                .map(|node| node.id)
                .collect::<Vec<_>>(),
            [THEME_LIGHT_ID, THEME_DARK_ID]
        );
        assert_eq!(menu.layout(THEME_ID, Some(0), &[]).unwrap().children, []);
        assert_eq!(
            TrayMenu::action(THEME_LIGHT_ID),
            Some(MenuAction::SetTheme(DeviceTheme::Light))
        );
        assert_eq!(
            TrayMenu::action(THEME_DARK_ID),
            Some(MenuAction::SetTheme(DeviceTheme::Dark))
        );
        assert_eq!(TrayMenu::action(THEME_ID), None);
        assert!(menu.contains(THEME_DARK_ID));
    }

    #[test]
    fn the_theme_in_use_carries_a_check_mark() {
        let mut menu = TrayMenu::new(vec!["A".into()]);
        let entry = |label: &str, marked| (label.to_owned(), marked, true);
        assert_eq!(
            theme_entries(&menu),
            [
                entry(THEME_LIGHT_LABEL, false),
                entry(THEME_DARK_LABEL, false)
            ]
        );
        assert!(menu.set_theme(Some(DeviceTheme::Dark)));
        assert_eq!(
            theme_entries(&menu),
            [
                entry(THEME_LIGHT_LABEL, false),
                entry(THEME_DARK_LABEL, true)
            ]
        );
        assert!(menu.set_theme(Some(DeviceTheme::Light)));
        assert_eq!(
            theme_entries(&menu),
            [
                entry(THEME_LIGHT_LABEL, true),
                entry(THEME_DARK_LABEL, false)
            ]
        );
    }

    #[test]
    fn setting_the_same_theme_keeps_the_revision() {
        let mut menu = TrayMenu::new(vec!["A".into()]);
        assert!(menu.set_theme(Some(DeviceTheme::Light)));
        let revision = menu.revision();
        assert!(!menu.set_theme(Some(DeviceTheme::Light)));
        assert_eq!(menu.revision(), revision);
        assert!(menu.set_theme(None));
        assert_eq!(menu.revision(), revision + 1);
    }

    #[test]
    fn the_theme_cannot_be_chosen_without_a_device() {
        let menu = TrayMenu::new(Vec::new());
        assert!(!is_enabled(&menu.layout(THEME_ID, Some(0), &[]).unwrap()));
        assert_eq!(
            theme_entries(&menu),
            [
                (THEME_LIGHT_LABEL.to_owned(), false, false),
                (THEME_DARK_LABEL.to_owned(), false, false)
            ]
        );
    }

    #[test]
    fn revision_increments_only_on_change() {
        let mut menu = TrayMenu::new(vec!["A".into()]);
        let first = menu.revision();
        assert!(!menu.set_devices(vec!["A".into()]));
        assert_eq!(menu.revision(), first);
        assert!(menu.set_devices(Vec::new()));
        assert_eq!(menu.revision(), first + 1);
    }

    #[test]
    fn device_labels_escape_access_key_markers() {
        let menu = TrayMenu::new(vec!["Deck (AB_12)".into()]);
        assert_eq!(summary(&menu)[3].1, "Deck (AB__12)");
    }
}
