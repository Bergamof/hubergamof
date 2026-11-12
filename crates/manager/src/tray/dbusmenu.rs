//! `com.canonical.dbusmenu` object exposing [`TrayMenu`] to the tray host.
//!
//! Protocol: <https://github.com/AyatanaIndicators/libdbusmenu/blob/master/libdbusmenu-glib/dbus-menu.xml>.

#![expect(
    clippy::used_underscore_binding,
    reason = "ignored D-Bus arguments are still passed by the zbus dispatcher"
)]

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use zbus::object_server::SignalEmitter;
use zbus::{ObjectPath, OwnedVariant, Type, Variant, fdo, interface};

use super::menu::{LayoutNode, MenuAction, Property, PropertyValue, TrayMenu};

/// Object path of the menu.
pub const MENU_PATH: ObjectPath<'static> = ObjectPath::from_static_str_unchecked("/MenuBar");

/// Protocol version implemented.
const PROTOCOL_VERSION: u32 = 3;

/// Event id of a click (other events — hovered, opened, closed — are ignored).
const CLICKED_EVENT: &str = "clicked";

/// Reacts to the menu entries.
pub trait MenuHandler: Send + Sync {
    /// Runs the action of a clicked entry.
    fn run(&self, action: MenuAction);
}

/// Item layout as sent on the wire: `(ia{sv}av)`, the children being
/// variants holding the same structure.
#[derive(Debug, Serialize, Type)]
pub struct WireLayout {
    id: i32,
    properties: HashMap<String, OwnedVariant>,
    children: Vec<OwnedVariant>,
}

impl WireLayout {
    fn from_node(node: LayoutNode) -> fdo::Result<Self> {
        Ok(Self {
            id: node.id,
            properties: wire_properties(node.properties),
            children: node
                .children
                .into_iter()
                .map(|child| Self::from_node(child)?.into_variant())
                .collect::<fdo::Result<_>>()?,
        })
    }

    fn into_variant(self) -> fdo::Result<OwnedVariant> {
        let structure = zbus::Structure::builder()
            .add_field(self.id)
            .add_field(self.properties)
            .add_field(self.children)
            .build()
            .map_err(|error| fdo::Error::Failed(error.to_string()))?;
        OwnedVariant::try_from(Variant::from(structure))
            .map_err(|error| fdo::Error::Failed(error.to_string()))
    }
}

fn wire_value(value: PropertyValue) -> OwnedVariant {
    match value {
        PropertyValue::Text(text) => OwnedVariant::from(zbus::Str::from(text)),
        PropertyValue::Flag(flag) => OwnedVariant::from(flag),
        PropertyValue::Int(number) => OwnedVariant::from(number),
    }
}

fn wire_properties(properties: Vec<Property>) -> HashMap<String, OwnedVariant> {
    properties
        .into_iter()
        .map(|(name, value)| (name.to_owned(), wire_value(value)))
        .collect()
}

fn unknown_id(id: i32) -> fdo::Error {
    fdo::Error::InvalidArgs(format!("unknown menu item {id}"))
}

/// The exported menu.
pub struct DbusMenu {
    menu: TrayMenu,
    handler: Arc<dyn MenuHandler>,
}

impl DbusMenu {
    /// Menu running clicked entries through `handler`.
    pub fn new(menu: TrayMenu, handler: Arc<dyn MenuHandler>) -> Self {
        Self { menu, handler }
    }

    /// The menu model.
    pub fn menu_mut(&mut self) -> &mut TrayMenu {
        &mut self.menu
    }

    /// Current layout revision.
    pub fn revision(&self) -> u32 {
        self.menu.revision()
    }

    /// Properties of the theme entries, as `ItemsPropertiesUpdated` sends
    /// them.
    pub fn theme_entries(&self) -> Vec<(i32, HashMap<String, OwnedVariant>)> {
        TrayMenu::theme_entry_ids()
            .into_iter()
            .filter_map(|id| {
                self.menu
                    .properties(id, &[])
                    .map(|properties| (id, wire_properties(properties)))
            })
            .collect()
    }

    /// Handles one event; `false` when the item is unknown.
    fn handle_event(&self, id: i32, event_id: &str) -> bool {
        if !self.menu.contains(id) {
            return false;
        }
        if event_id == CLICKED_EVENT
            && let Some(action) = TrayMenu::action(id)
        {
            self.handler.run(action);
        }
        true
    }
}

#[interface(name = "com.canonical.dbusmenu")]
#[expect(
    clippy::needless_pass_by_value,
    clippy::unused_self,
    reason = "zbus dispatches on `&self` and hands owned (deserialized) arguments"
)]
impl DbusMenu {
    /// Layout below `parent_id` (`recursion_depth` < 0: whole tree).
    #[zbus(out_args("revision", "layout"))]
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> fdo::Result<(u32, WireLayout)> {
        let depth = usize::try_from(recursion_depth).ok();
        let node = self
            .menu
            .layout(parent_id, depth, &property_names)
            .ok_or_else(|| unknown_id(parent_id))?;
        Ok((self.menu.revision(), WireLayout::from_node(node)?))
    }

    /// Properties of `ids` (every item when empty); unknown ids are skipped.
    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedVariant>)> {
        let items = if ids.is_empty() {
            self.menu.all_properties(&property_names)
        } else {
            ids.into_iter()
                .filter_map(|id| {
                    self.menu
                        .properties(id, &property_names)
                        .map(|properties| (id, properties))
                })
                .collect()
        };
        items
            .into_iter()
            .map(|(id, properties)| (id, wire_properties(properties)))
            .collect()
    }

    /// One property of one item.
    fn get_property(&self, id: i32, name: String) -> fdo::Result<OwnedVariant> {
        let mut properties = self
            .menu
            .properties(id, std::slice::from_ref(&name))
            .ok_or_else(|| unknown_id(id))?;
        properties
            .pop()
            .map(|(_, value)| wire_value(value))
            .ok_or_else(|| fdo::Error::InvalidArgs(format!("no property {name} on item {id}")))
    }

    /// An event on an item (only clicks do something).
    fn event(
        &self,
        id: i32,
        event_id: &str,
        _data: OwnedVariant,
        _timestamp: u32,
    ) -> fdo::Result<()> {
        if self.handle_event(id, event_id) {
            Ok(())
        } else {
            Err(unknown_id(id))
        }
    }

    /// Several events; returns the unknown ids.
    fn event_group(&self, events: Vec<(i32, String, OwnedVariant, u32)>) -> fdo::Result<Vec<i32>> {
        let total = events.len();
        let unknown: Vec<i32> = events
            .into_iter()
            .filter(|(id, event_id, ..)| !self.handle_event(*id, event_id))
            .map(|(id, ..)| id)
            .collect();
        if total > 0 && unknown.len() == total {
            return Err(fdo::Error::InvalidArgs(
                "no known menu item in the events".into(),
            ));
        }
        Ok(unknown)
    }

    /// The menu is built ahead: never needs an update before showing. (A
    /// host told otherwise rebuilds the menu while opening it, which closes
    /// it at once: changes are announced with `LayoutUpdated` instead.)
    fn about_to_show(&self, id: i32) -> fdo::Result<bool> {
        if self.menu.contains(id) {
            Ok(false)
        } else {
            Err(unknown_id(id))
        }
    }

    /// Group form of `AboutToShow`: nothing to update, unknown ids reported.
    #[zbus(out_args("updatesNeeded", "idErrors"))]
    fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        let unknown = ids
            .into_iter()
            .filter(|id| !self.menu.contains(*id))
            .collect();
        (Vec::new(), unknown)
    }

    /// Protocol version.
    #[zbus(property(emits_changed_signal = "const"))]
    fn version(&self) -> u32 {
        PROTOCOL_VERSION
    }

    /// Left-to-right labels.
    #[zbus(property(emits_changed_signal = "const"))]
    fn text_direction(&self) -> &'static str {
        "ltr"
    }

    /// Normal (not « needs attention ») menu.
    #[zbus(property(emits_changed_signal = "const"))]
    fn status(&self) -> &'static str {
        "normal"
    }

    /// Icons come from the system theme.
    #[zbus(property(emits_changed_signal = "const"))]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    /// Properties of some items changed: hosts update them without asking
    /// for the layout again (used for the theme marks, the other changes
    /// are layout changes).
    #[zbus(signal)]
    pub async fn items_properties_updated(
        emitter: &SignalEmitter<'_>,
        updated_props: Vec<(i32, HashMap<String, OwnedVariant>)>,
        removed_props: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;

    /// The layout below `parent` changed; hosts call `GetLayout` again.
    #[zbus(signal)]
    pub async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::tray::menu::{FIRST_DEVICE_ID, OPEN_UI_ID, QUIT_ID, ROOT_ID, THEME_ID};

    #[derive(Default)]
    struct Recorder(Mutex<Vec<MenuAction>>);

    impl MenuHandler for Recorder {
        fn run(&self, action: MenuAction) {
            self.0.lock().unwrap().push(action);
        }
    }

    fn menu() -> (DbusMenu, Arc<Recorder>) {
        let recorder = Arc::new(Recorder::default());
        let menu = DbusMenu::new(TrayMenu::new(vec!["Deck (A1)".into()]), recorder.clone());
        (menu, recorder)
    }

    #[test]
    fn clicks_run_the_entry_action_and_other_events_do_not() {
        let (menu, recorder) = menu();
        assert!(menu.handle_event(OPEN_UI_ID, "hovered"));
        assert!(menu.handle_event(FIRST_DEVICE_ID, CLICKED_EVENT));
        assert!(menu.handle_event(ROOT_ID, CLICKED_EVENT));
        assert!(menu.handle_event(OPEN_UI_ID, CLICKED_EVENT));
        assert!(menu.handle_event(QUIT_ID, CLICKED_EVENT));
        assert!(!menu.handle_event(42, CLICKED_EVENT));
        assert_eq!(
            *recorder.0.lock().unwrap(),
            [MenuAction::OpenUi, MenuAction::Quit]
        );
    }

    #[test]
    fn event_group_reports_unknown_ids() {
        let (menu, _) = menu();
        let event = |id: i32| (id, CLICKED_EVENT.to_owned(), OwnedVariant::from(0_i32), 0);
        assert_eq!(
            menu.event_group(vec![event(FIRST_DEVICE_ID), event(42)])
                .unwrap(),
            [42]
        );
        assert!(menu.event_group(vec![event(42)]).is_err());
        assert!(
            menu.event(42, CLICKED_EVENT, OwnedVariant::from(0_i32), 0)
                .is_err()
        );
    }

    #[test]
    fn layout_serialises_with_the_dbusmenu_signature() {
        let (menu, _) = menu();
        let (revision, layout) = menu.get_layout(ROOT_ID, -1, Vec::new()).unwrap();
        assert_eq!(revision, 1);
        assert_eq!(WireLayout::SIGNATURE.to_string(), "(ia{sv}av)");
        assert_eq!(layout.children.len(), 6);
        let quit = Variant::from(layout.children[5].clone());
        let Variant::Structure(quit) = quit else {
            panic!("children are structures");
        };
        assert_eq!(quit.signature().to_string(), "(ia{sv}av)");
        assert_eq!(quit.fields()[0], Variant::from(QUIT_ID));
        assert!(menu.get_layout(42, -1, Vec::new()).is_err());
    }

    #[test]
    fn single_property_lookup() {
        let (menu, _) = menu();
        assert_eq!(
            menu.get_property(FIRST_DEVICE_ID, "enabled".into())
                .unwrap(),
            OwnedVariant::from(false)
        );
        assert!(
            menu.get_property(FIRST_DEVICE_ID, "icon-name".into())
                .is_err()
        );
        assert!(menu.get_property(42, "label".into()).is_err());
        let all = menu.get_group_properties(vec![QUIT_ID, 42], vec!["label".into()]);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, QUIT_ID);
    }

    #[test]
    fn about_to_show_never_requests_an_update() {
        let (menu, _) = menu();
        assert!(!menu.about_to_show(ROOT_ID).unwrap());
        assert!(!menu.about_to_show(THEME_ID).unwrap());
        assert!(menu.about_to_show(42).is_err());
        assert_eq!(
            menu.about_to_show_group(vec![ROOT_ID, THEME_ID, 42]),
            (Vec::new(), vec![42])
        );
    }
}
