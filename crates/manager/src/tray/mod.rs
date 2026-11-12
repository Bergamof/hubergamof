//! System tray icon: freedesktop `StatusNotifierItem` and its
//! `com.canonical.dbusmenu` menu, served with `zbus`.
//!
//! Menu: open the web UI, the theme of the devices (submenu marking the theme
//! in use), the connected devices (informative, updated live), quit. Without a session bus the manager runs without the icon; without a
//! `StatusNotifierWatcher` (headless session, GNOME without the
//! `AppIndicator` extension, Plasma not started yet) the item waits for one
//! and registers as soon as it appears, and again after each restart of it.

mod dbusmenu;
mod icon;
mod item;
mod menu;
mod theme;
mod watcher;

use hubergamof_modules::desktop::{Launch, scoped_command};
use std::sync::Arc;
use std::time::Duration;

use hubergamof_protocol::DeviceInfo;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use zbus::Connection;
use zbus::connection::Builder;

pub use item::{TRAY_ID, TRAY_TITLE};
pub use menu::{NO_DEVICE_LABEL, OPEN_UI_LABEL, QUIT_LABEL};

use dbusmenu::{DbusMenu, MENU_PATH, MenuHandler};
use item::{ITEM_PATH, StatusNotifierItem};
use menu::{MenuAction, TrayMenu};
use theme::{apply_theme, follow_theme};

use crate::device::{DeviceDiscovery, DiscoveryEvent};
use crate::engine::EngineRegistry;
use watcher::{Registrar, item_bus_name};

/// Longest wait for the session bus connection and the first registration.
const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Program opening a URL in the user's browser.
const URL_OPENER: &str = "xdg-open";

/// Runs the menu actions: opens `url`, changes the theme of the devices of
/// `engines`, or calls `quit`.
struct UiActions {
    url: String,
    engines: EngineRegistry,
    quit: Arc<dyn Fn() + Send + Sync>,
}

impl MenuHandler for UiActions {
    fn run(&self, action: MenuAction) {
        match action {
            MenuAction::OpenUi => open_url(&self.url),
            MenuAction::SetTheme(theme) => {
                let engines = self.engines.clone();
                tokio::spawn(async move { apply_theme(&engines, theme).await });
            }
            MenuAction::Quit => (self.quit)(),
        }
    }
}

/// Menu labels of the connected devices.
#[must_use]
pub fn device_labels(devices: &[DeviceInfo]) -> Vec<String> {
    devices
        .iter()
        .map(|device| format!("{} ({})", device.display_name, device.serial))
        .collect()
}

/// Opens `url` in the browser without a shell, in a scope of its own (a
/// browser it starts outlives the service); the child is reaped on a
/// separate thread.
fn open_url(url: &str) {
    let opener = Launch {
        program: URL_OPENER.into(),
        args: vec![url.into()],
    };
    match scoped_command(&opener).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                if let Err(error) = child.wait() {
                    tracing::warn!(%error, "cannot wait for {URL_OPENER}");
                }
            });
        }
        Err(error) => tracing::warn!(%error, "cannot run {URL_OPENER}"),
    }
}

/// Adds the tray item and its menu to a connection being built.
fn serve<'a>(
    builder: Builder<'a>,
    devices: &[String],
    handler: &Arc<dyn MenuHandler>,
) -> Builder<'a> {
    builder
        .serve_at(
            ITEM_PATH,
            StatusNotifierItem::new(devices.to_vec(), Arc::clone(handler)),
        )
        .serve_at(
            MENU_PATH,
            DbusMenu::new(TrayMenu::new(devices.to_vec()), Arc::clone(handler)),
        )
}

/// Running tray icon, following the attached devices.
pub struct TrayIcon {
    connection: Connection,
    tasks: Vec<JoinHandle<()>>,
}

impl TrayIcon {
    /// Shows the tray icon; `None` (with a warning) without a session bus.
    pub async fn start(
        url: String,
        discovery: &DeviceDiscovery,
        engines: EngineRegistry,
        quit: Arc<dyn Fn() + Send + Sync>,
    ) -> Option<Self> {
        let (devices, events) = discovery.subscribe();
        let handler: Arc<dyn MenuHandler> = Arc::new(UiActions {
            url,
            engines: engines.clone(),
            quit,
        });
        let started = tokio::time::timeout(
            REGISTRATION_TIMEOUT,
            connect(&device_labels(&devices), &handler),
        )
        .await;
        let (connection, registrar) = match started {
            Ok(Ok(started)) => started,
            Ok(Err(error)) => {
                tracing::warn!(%error, "no session bus for the system tray, running without it");
                return None;
            }
            Err(_) => {
                tracing::warn!("system tray registration timed out, running without it");
                return None;
            }
        };
        let tasks = vec![
            tokio::spawn(registrar.follow()),
            tokio::spawn(follow_devices(
                connection.clone(),
                discovery.clone(),
                events,
            )),
            tokio::spawn(follow_theme(connection.clone(), engines)),
        ];
        Some(Self { connection, tasks })
    }

    /// Removes the icon (the watcher drops it when our bus name goes away).
    pub async fn stop(self) {
        // Stopped before closing: an update under way would otherwise hit
        // the closed connection (devices go away during shutdown).
        stop_tasks(self.tasks).await;
        if let Err(error) = self.connection.close().await {
            tracing::warn!(%error, "cannot close the system tray connection");
        }
    }
}

/// Cancels `tasks` and waits until they are gone: `abort` alone lets a task
/// being polled on another worker run until its next suspension point.
async fn stop_tasks(tasks: Vec<JoinHandle<()>>) {
    for task in &tasks {
        task.abort();
    }
    for task in tasks {
        if let Err(error) = task.await
            && error.is_panic()
        {
            tracing::warn!(%error, "system tray task panicked");
        }
    }
}

/// Connects to the session bus with the item and menu exported, then
/// registers with the watcher if one runs.
async fn connect(
    devices: &[String],
    handler: &Arc<dyn MenuHandler>,
) -> zbus::Result<(Connection, Registrar)> {
    let name = item_bus_name();
    let connection = serve(Builder::session(), devices, handler)
        .name(name.as_str())
        .build()
        .await?;
    let registrar = Registrar::new(&connection, name).await?;
    registrar.register().await;
    Ok((connection, registrar))
}

/// Mirrors the attached devices in the menu and the tooltip.
async fn follow_devices(
    connection: Connection,
    discovery: DeviceDiscovery,
    mut events: tokio::sync::broadcast::Receiver<DiscoveryEvent>,
) {
    loop {
        match events.recv().await {
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
        }
        let labels = device_labels(&discovery.devices());
        if let Err(error) = publish_devices(&connection, labels).await {
            tracing::warn!(%error, "cannot update the system tray menu");
        }
    }
}

/// Updates the menu and the tooltip, and tells the host when they changed.
async fn publish_devices(connection: &Connection, labels: Vec<String>) -> zbus::Result<()> {
    let objects = connection.object_server();
    let menu = objects.interface::<_, DbusMenu>(MENU_PATH).await?;
    let revision = {
        let mut menu = menu.get_mut().await;
        if !menu.menu_mut().set_devices(labels.clone()) {
            return Ok(());
        }
        menu.revision()
    };
    DbusMenu::layout_updated(menu.signal_emitter(), revision, menu::ROOT_ID).await?;
    let item = objects
        .interface::<_, StatusNotifierItem>(ITEM_PATH)
        .await?;
    item.get_mut().await.set_devices(labels);
    StatusNotifierItem::new_tool_tip(item.signal_emitter()).await
}

/// `quit` callback sending `true` on `shutdown`.
#[must_use]
pub fn quit_on(shutdown: watch::Sender<bool>) -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(move || {
        shutdown.send_replace(true);
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::os::unix::net::UnixStream;

    use zbus::{Guid, ObjectPath, OwnedVariant, Variant};

    use super::*;
    use crate::device::virtual_deck::virtual_plus_info;
    use menu::{FIRST_DEVICE_ID, QUIT_ID, ROOT_ID};

    const MENU_INTERFACE: &str = "com.canonical.dbusmenu";
    const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
    const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";

    type WireLayout = (i32, HashMap<String, OwnedVariant>, Vec<OwnedVariant>);

    #[test]
    fn device_labels_show_name_and_serial() {
        let name = virtual_plus_info("A1").display_name;
        assert_eq!(
            device_labels(&[virtual_plus_info("A1")]),
            [format!("{name} (A1)")]
        );
    }

    /// Sets its flag when dropped.
    struct DropFlag(Arc<std::sync::atomic::AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stopping_waits_for_tasks_busy_on_another_worker() {
        const BUSY_POLL: Duration = Duration::from_millis(200);
        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (running, mut started) = tokio::sync::mpsc::channel(1);
        let guard = DropFlag(Arc::clone(&stopped));
        let task = tokio::spawn(async move {
            let _guard = guard;
            running.send(()).await.unwrap();
            loop {
                // A poll that does not yield, like an update under way.
                std::thread::sleep(BUSY_POLL);
                tokio::task::yield_now().await;
            }
        });
        started.recv().await.unwrap();
        stop_tasks(vec![task]).await;
        assert!(stopped.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn quit_action_triggers_shutdown() {
        let (shutdown, stopping) = watch::channel(false);
        let actions = UiActions {
            url: String::new(),
            engines: EngineRegistry::default(),
            quit: quit_on(shutdown),
        };
        actions.run(MenuAction::Quit);
        assert!(*stopping.borrow());
    }

    /// Peer-to-peer connections (no bus daemon): the tray side serves the
    /// objects, the host side calls them as a tray host would.
    async fn connected_pair(
        devices: &[String],
        engines: EngineRegistry,
        quit: Arc<dyn Fn() + Send + Sync>,
    ) -> (Connection, Connection) {
        let (tray_socket, host_socket) = UnixStream::pair().unwrap();
        let handler: Arc<dyn MenuHandler> = Arc::new(UiActions {
            url: String::new(),
            engines,
            quit,
        });
        let tray = serve(Builder::unix_stream(tray_socket), devices, &handler)
            .server(Guid::generate())
            .p2p()
            .build();
        let host = Builder::unix_stream(host_socket).p2p().build();
        let (tray, host) = tokio::join!(tray, host);
        (tray.unwrap(), host.unwrap())
    }

    async fn call<B, R>(
        host: &Connection,
        path: ObjectPath<'static>,
        interface: &str,
        method: &str,
        body: &B,
    ) -> R
    where
        B: serde::Serialize + zbus::wire::DynamicType,
        R: for<'d> zbus::wire::DynamicDeserialize<'d>,
    {
        host.call_method(None::<&str>, path, Some(interface), method, body)
            .await
            .unwrap()
            .body()
            .deserialize()
            .unwrap()
    }

    async fn property(
        host: &Connection,
        path: ObjectPath<'static>,
        interface: &str,
        name: &str,
    ) -> OwnedVariant {
        call(host, path, PROPERTIES_INTERFACE, "Get", &(interface, name)).await
    }

    fn label(properties: &HashMap<String, OwnedVariant>) -> String {
        String::try_from(properties["label"].clone()).unwrap()
    }

    #[tokio::test]
    async fn host_reads_the_item_and_the_menu_over_dbus() {
        let devices = ["Deck (A1)".to_owned()];
        let (_tray, host) =
            connected_pair(&devices, EngineRegistry::default(), Arc::new(|| {})).await;

        let menu_path = property(&host, ITEM_PATH, ITEM_INTERFACE, "Menu").await;
        assert_eq!(
            zbus::OwnedObjectPath::try_from(menu_path).unwrap().as_str(),
            MENU_PATH.as_str()
        );
        let id = property(&host, ITEM_PATH, ITEM_INTERFACE, "Id").await;
        assert_eq!(String::try_from(id).unwrap(), TRAY_ID);
        let icon_name = property(&host, ITEM_PATH, ITEM_INTERFACE, "IconName").await;
        assert_eq!(String::try_from(icon_name).unwrap(), "");
        let icon = property(&host, ITEM_PATH, ITEM_INTERFACE, "IconPixmap").await;
        let sizes: Vec<i32> = Vec::<(i32, i32, Vec<u8>)>::try_from(icon)
            .unwrap()
            .iter()
            .map(|(width, _, _)| *width)
            .collect();
        assert_eq!(sizes, [22, 32, 48, 64]);
        let version = property(&host, MENU_PATH, MENU_INTERFACE, "Version").await;
        assert_eq!(u32::try_from(version).unwrap(), 3);

        let (revision, (root_id, _, children)): (u32, WireLayout) = call(
            &host,
            MENU_PATH,
            MENU_INTERFACE,
            "GetLayout",
            &(ROOT_ID, -1_i32, Vec::<String>::new()),
        )
        .await;
        assert_eq!((revision, root_id), (1, ROOT_ID));
        let children: Vec<WireLayout> = children
            .into_iter()
            .map(|child| WireLayout::try_from(Variant::from(child)).unwrap())
            .collect();
        assert_eq!(children.len(), 6);
        assert_eq!(label(&children[0].1), OPEN_UI_LABEL);
        assert_eq!(label(&children[1].1), menu::THEME_LABEL);
        assert_eq!(children[3].0, FIRST_DEVICE_ID);
        assert_eq!(label(&children[3].1), "Deck (A1)");
        assert!(!bool::try_from(children[3].1["enabled"].clone()).unwrap());
        assert_eq!(children[5].0, QUIT_ID);
        assert_eq!(label(&children[5].1), QUIT_LABEL);
    }

    #[tokio::test]
    async fn clicking_quit_over_dbus_requests_shutdown() {
        let (shutdown, stopping) = watch::channel(false);
        let (_tray, host) = connected_pair(&[], EngineRegistry::default(), quit_on(shutdown)).await;
        let () = call(
            &host,
            MENU_PATH,
            MENU_INTERFACE,
            "Event",
            &(QUIT_ID, "clicked", Variant::from(""), 0_u32),
        )
        .await;
        assert!(*stopping.borrow());
    }

    #[tokio::test]
    async fn device_changes_bump_the_layout_revision() {
        let (tray, host) = connected_pair(&[], EngineRegistry::default(), Arc::new(|| {})).await;
        publish_devices(&tray, vec!["Deck (B2)".into()])
            .await
            .unwrap();
        publish_devices(&tray, vec!["Deck (B2)".into()])
            .await
            .unwrap();
        let (revision, (_, _, children)): (u32, WireLayout) = call(
            &host,
            MENU_PATH,
            MENU_INTERFACE,
            "GetLayout",
            &(ROOT_ID, -1_i32, vec!["label".to_owned()]),
        )
        .await;
        assert_eq!(revision, 2);
        let device = WireLayout::try_from(Variant::from(children[3].clone())).unwrap();
        assert_eq!(label(&device.1), "Deck (B2)");
        assert!(!device.1.contains_key("enabled"));
    }

    /// Layout revision and the theme entry marked as current, as a host
    /// reads them.
    async fn marked_theme(host: &Connection) -> (u32, Option<String>) {
        let (revision, (_, _, children)): (u32, WireLayout) = call(
            host,
            MENU_PATH,
            MENU_INTERFACE,
            "GetLayout",
            &(menu::THEME_ID, -1_i32, Vec::<String>::new()),
        )
        .await;
        let marked = children
            .into_iter()
            .map(|child| WireLayout::try_from(Variant::from(child)).unwrap().1)
            .map(|properties| label(&properties))
            .find_map(|text| {
                text.strip_prefix(menu::SELECTED_MARK)
                    .map(|name| name.trim().to_owned())
            });
        (revision, marked)
    }

    /// Waits until the menu marks `expected`.
    async fn wait_for_mark(host: &Connection, expected: Option<&str>) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if marked_theme(host).await.1.as_deref() == expected {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the menu shows the theme");
    }

    #[tokio::test]
    async fn clicking_a_theme_over_dbus_applies_it() {
        let rig = theme::tests::Rig::start(&["A1"]).await;
        let (_tray, host) = connected_pair(
            &["Deck (A1)".to_owned()],
            rig.engines.clone(),
            Arc::new(|| {}),
        )
        .await;
        let () = call(
            &host,
            MENU_PATH,
            MENU_INTERFACE,
            "Event",
            &(menu::THEME_DARK_ID, "clicked", Variant::from(""), 0_u32),
        )
        .await;
        rig.wait_for_theme("A1", hubergamof_protocol::DeviceTheme::Dark)
            .await;
        rig.stop().await;
    }

    #[tokio::test]
    async fn the_menu_marks_the_theme_of_the_devices_and_follows_changes() {
        use hubergamof_protocol::DeviceTheme::{Dark, Light};
        let rig = theme::tests::Rig::start(&["A1"]).await;
        let (tray, host) = connected_pair(
            &["Deck (A1)".to_owned()],
            rig.engines.clone(),
            Arc::new(|| {}),
        )
        .await;
        let follower = tokio::spawn(follow_theme(tray.clone(), rig.engines.clone()));

        wait_for_mark(&host, Some(menu::THEME_LIGHT_LABEL)).await;
        theme::apply_theme(&rig.engines, Dark).await;
        wait_for_mark(&host, Some(menu::THEME_DARK_LABEL)).await;
        let (revision, _) = marked_theme(&host).await;
        theme::apply_theme(&rig.engines, Light).await;
        wait_for_mark(&host, Some(menu::THEME_LIGHT_LABEL)).await;
        assert!(marked_theme(&host).await.0 > revision);

        stop_tasks(vec![follower]).await;
        rig.stop().await;
    }

    #[tokio::test]
    async fn the_menu_marks_nothing_when_the_devices_differ() {
        use hubergamof_protocol::DeviceTheme::Dark;
        let rig = theme::tests::Rig::start(&["A1", "B2"]).await;
        let (tray, host) = connected_pair(
            &["Deck (A1)".to_owned()],
            rig.engines.clone(),
            Arc::new(|| {}),
        )
        .await;
        let follower = tokio::spawn(follow_theme(tray.clone(), rig.engines.clone()));
        wait_for_mark(&host, Some(menu::THEME_LIGHT_LABEL)).await;

        let one = rig.engines.get("A1").unwrap();
        let mut config = one.config().borrow().clone();
        config.theme = Dark;
        one.apply_config(config).await.unwrap();
        wait_for_mark(&host, None).await;

        stop_tasks(vec![follower]).await;
        rig.stop().await;
    }

    #[tokio::test]
    async fn a_theme_change_announces_the_new_marks_to_the_host() {
        use futures_util::StreamExt as _;
        use hubergamof_protocol::DeviceTheme::Dark;
        type Updated = (
            Vec<(i32, HashMap<String, OwnedVariant>)>,
            Vec<(i32, Vec<String>)>,
        );
        let (tray, host) = connected_pair(
            &["Deck (A1)".to_owned()],
            EngineRegistry::default(),
            Arc::new(|| {}),
        )
        .await;
        let mut messages = zbus::MessageStream::from(&host);

        theme::publish_theme(&tray, Some(Dark)).await.unwrap();
        let mut marks = None;
        let mut layout = None;
        tokio::time::timeout(Duration::from_secs(10), async {
            while marks.is_none() || layout.is_none() {
                let message = messages.next().await.unwrap().unwrap();
                match message
                    .header()
                    .member()
                    .map(zbus::names::MemberName::as_str)
                {
                    Some("ItemsPropertiesUpdated") => {
                        marks = Some(message.body().deserialize::<Updated>().unwrap());
                    }
                    Some("LayoutUpdated") => {
                        layout = Some(message.body().deserialize::<(u32, i32)>().unwrap());
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("the host is told");

        assert_eq!(layout, Some((2, menu::THEME_ID)));
        let (updated, removed) = marks.unwrap();
        assert_eq!(removed, []);
        let labels: Vec<(i32, String)> = updated
            .iter()
            .map(|(id, properties)| (*id, label(properties)))
            .collect();
        assert_eq!(
            labels,
            [
                (menu::THEME_LIGHT_ID, "○ Clair".to_owned()),
                (menu::THEME_DARK_ID, "● Sombre".to_owned())
            ]
        );
    }
}
