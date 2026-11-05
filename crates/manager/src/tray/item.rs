//! `org.kde.StatusNotifierItem` object: the tray icon itself.
//!
//! Specification: <https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/StatusNotifierItem/>.

#![expect(
    clippy::used_underscore_binding,
    reason = "ignored D-Bus arguments are still passed by the zbus dispatcher"
)]

use std::sync::Arc;

use serde::Serialize;
use zbus::object_server::SignalEmitter;
use zbus::{ObjectPath, Type, interface};

use super::dbusmenu::{MENU_PATH, MenuHandler};
use super::menu::{MenuAction, NO_DEVICE_LABEL};

/// Object path of the item, where hosts look for it.
pub const ITEM_PATH: ObjectPath<'static> =
    ObjectPath::from_static_str_unchecked("/StatusNotifierItem");

/// Id of the tray item on D-Bus.
pub const TRAY_ID: &str = "hubergamof";

/// Title of the tray item.
pub const TRAY_TITLE: &str = "HuBergamof";

/// Generic freedesktop icon (present in every icon theme).
pub const TRAY_ICON: &str = "input-keyboard";

/// Generic application (neither communication, system service nor hardware).
const CATEGORY: &str = "ApplicationStatus";

/// Always shown.
const STATUS: &str = "Active";

/// A left click opens the web UI (`Activate`) instead of the menu; the menu
/// stays on the right click.
const ITEM_IS_MENU: bool = false;

/// ARGB32 image, `(iiay)`.
#[derive(Debug, Clone, Serialize, Type)]
pub struct Pixmap {
    width: i32,
    height: i32,
    data: Vec<u8>,
}

/// Tooltip, `(sa(iiay)ss)`.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ToolTip {
    icon_name: String,
    icon_pixmap: Vec<Pixmap>,
    title: String,
    description: String,
}

/// The exported tray item.
pub struct StatusNotifierItem {
    devices: Vec<String>,
    handler: Arc<dyn MenuHandler>,
}

impl StatusNotifierItem {
    /// Item listing `devices` in its tooltip and opening the UI through
    /// `handler`.
    pub fn new(devices: Vec<String>, handler: Arc<dyn MenuHandler>) -> Self {
        Self { devices, handler }
    }

    /// Replaces the devices shown in the tooltip.
    pub fn set_devices(&mut self, devices: Vec<String>) {
        self.devices = devices;
    }

    fn tool_tip_description(&self) -> String {
        if self.devices.is_empty() {
            NO_DEVICE_LABEL.to_owned()
        } else {
            escape_markup(&self.devices.join("\n"))
        }
    }
}

#[interface(name = "org.kde.StatusNotifierItem")]
#[expect(
    clippy::unused_self,
    reason = "zbus dispatches methods and properties on `&self`, constant ones included"
)]
impl StatusNotifierItem {
    /// Primary click: opens the web UI.
    fn activate(&self, _x: i32, _y: i32) {
        self.handler.run(MenuAction::OpenUi);
    }

    /// Middle click: nothing.
    fn secondary_activate(&self, _x: i32, _y: i32) {}

    /// The host shows the dbusmenu at `Menu` itself: nothing to draw here.
    fn context_menu(&self, _x: i32, _y: i32) {}

    /// Wheel over the icon: nothing.
    fn scroll(&self, _delta: i32, _orientation: &str) {}

    /// Item category.
    #[zbus(property(emits_changed_signal = "const"))]
    fn category(&self) -> &'static str {
        CATEGORY
    }

    /// Application id.
    #[zbus(property(emits_changed_signal = "const"))]
    fn id(&self) -> &'static str {
        TRAY_ID
    }

    /// Application name.
    #[zbus(property(emits_changed_signal = "const"))]
    fn title(&self) -> &'static str {
        TRAY_TITLE
    }

    /// Item status.
    #[zbus(property(emits_changed_signal = "const"))]
    fn status(&self) -> &'static str {
        STATUS
    }

    /// Themed icon name.
    #[zbus(property(emits_changed_signal = "const"))]
    fn icon_name(&self) -> &'static str {
        TRAY_ICON
    }

    /// No raster icon: the themed one is used.
    #[zbus(property(emits_changed_signal = "const"))]
    fn icon_pixmap(&self) -> Vec<Pixmap> {
        Vec::new()
    }

    /// No extra icon theme directory.
    #[zbus(property(emits_changed_signal = "const"))]
    fn icon_theme_path(&self) -> &'static str {
        ""
    }

    /// Title and connected devices.
    #[zbus(property(emits_changed_signal = "false"))]
    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            icon_name: TRAY_ICON.to_owned(),
            icon_pixmap: Vec::new(),
            title: TRAY_TITLE.to_owned(),
            description: self.tool_tip_description(),
        }
    }

    /// Whether a left click shows the menu.
    #[zbus(property(emits_changed_signal = "const"))]
    fn item_is_menu(&self) -> bool {
        ITEM_IS_MENU
    }

    /// Path of the dbusmenu object.
    #[zbus(property(emits_changed_signal = "const"))]
    fn menu(&self) -> ObjectPath<'static> {
        MENU_PATH
    }

    /// The tooltip changed.
    #[zbus(signal)]
    pub async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// The tooltip description accepts a subset of HTML.
fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<MenuAction>>);

    impl MenuHandler for Recorder {
        fn run(&self, action: MenuAction) {
            self.0.lock().unwrap().push(action);
        }
    }

    #[test]
    fn activate_opens_the_ui_and_other_clicks_do_nothing() {
        let recorder = Arc::new(Recorder::default());
        let item = StatusNotifierItem::new(Vec::new(), recorder.clone());
        item.secondary_activate(0, 0);
        item.context_menu(0, 0);
        item.scroll(1, "vertical");
        item.activate(0, 0);
        assert_eq!(*recorder.0.lock().unwrap(), [MenuAction::OpenUi]);
    }

    #[test]
    fn tooltip_lists_the_devices_escaped() {
        let mut item = StatusNotifierItem::new(Vec::new(), Arc::new(Recorder::default()));
        assert_eq!(item.tool_tip().description, NO_DEVICE_LABEL);
        item.set_devices(vec!["A <1>".into(), "B & C".into()]);
        let tool_tip = item.tool_tip();
        assert_eq!(tool_tip.title, TRAY_TITLE);
        assert_eq!(tool_tip.description, "A &lt;1&gt;\nB &amp; C");
        assert_eq!(ToolTip::SIGNATURE.to_string(), "(sa(iiay)ss)");
    }
}
