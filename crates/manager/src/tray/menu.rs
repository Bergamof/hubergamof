//! Tray menu model, independent of D-Bus: entries, stable ids, layout
//! revision and the `com.canonical.dbusmenu` item properties.
//!
//! Entries: open the web UI, separator, the connected devices (informative,
//! disabled), separator, quit.

/// Menu entry opening the web UI.
pub const OPEN_UI_LABEL: &str = "Ouvrir l'interface";

/// Menu entry stopping the manager.
pub const QUIT_LABEL: &str = "Quitter";

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
/// Id of the first device line (or of the « no device » line); the next ones
/// follow.
pub const FIRST_DEVICE_ID: i32 = 100;

const OPEN_UI_ICON: &str = "internet-web-browser";
const QUIT_ICON: &str = "application-exit";

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
    /// Stops the manager.
    Quit,
}

/// The tray menu: device lines plus the layout revision announced through
/// `LayoutUpdated`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayMenu {
    devices: Vec<String>,
    revision: u32,
}

impl TrayMenu {
    /// Menu listing `devices` (labels, see [`super::device_labels`]).
    #[must_use]
    pub fn new(devices: Vec<String>) -> Self {
        Self {
            devices,
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
        if parent_id == ROOT_ID {
            let children = if depth == Some(0) {
                Vec::new()
            } else {
                self.child_ids()
                    .into_iter()
                    .filter_map(|id| self.leaf(id, filter))
                    .collect()
            };
            return Some(LayoutNode {
                id: ROOT_ID,
                properties: filtered(root_properties(), filter),
                children,
            });
        }
        self.leaf(parent_id, filter)
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

    /// Action run by a click on `id`; `None` for the root, separators and the
    /// (disabled) device lines.
    #[must_use]
    pub fn action(id: i32) -> Option<MenuAction> {
        match id {
            OPEN_UI_ID => Some(MenuAction::OpenUi),
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
        let mut ids = vec![OPEN_UI_ID, TOP_SEPARATOR_ID];
        ids.extend(self.device_ids());
        ids.extend([BOTTOM_SEPARATOR_ID, QUIT_ID]);
        ids
    }

    fn leaf(&self, id: i32, filter: &[String]) -> Option<LayoutNode> {
        self.properties(id, filter).map(|properties| LayoutNode {
            id,
            properties,
            children: Vec::new(),
        })
    }

    fn item_properties(&self, id: i32) -> Option<Vec<Property>> {
        match id {
            OPEN_UI_ID => Some(action_properties(OPEN_UI_LABEL, OPEN_UI_ICON)),
            QUIT_ID => Some(action_properties(QUIT_LABEL, QUIT_ICON)),
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
            summary(&menu)[2],
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
            5
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
                TOP_SEPARATOR_ID,
                FIRST_DEVICE_ID,
                BOTTOM_SEPARATOR_ID,
                QUIT_ID
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
        assert_eq!(summary(&menu)[2].1, "Deck (AB__12)");
    }
}
