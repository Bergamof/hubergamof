//! Registration with the `org.kde.StatusNotifierWatcher`, kept across
//! restarts of the watcher (Plasma shell, `AppIndicator` extension).

use futures_util::StreamExt as _;
use zbus::{Connection, fdo, proxy};

/// Well-known name of the watcher.
pub const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";

#[proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher"
)]
trait StatusNotifierWatcher {
    /// Shows the item owned by bus name `service`.
    fn register_status_notifier_item(&self, service: &str) -> zbus::Result<()>;
}

/// Bus name of the item of this process (one item per process).
#[must_use]
pub fn item_bus_name() -> String {
    format!("org.kde.StatusNotifierItem-{}-1", std::process::id())
}

/// Watches the watcher and registers `item_name` with it now and every time
/// it (re)appears.
pub struct Registrar {
    watcher: StatusNotifierWatcherProxy<'static>,
    owner_changes: fdo::NameOwnerChangedStream,
    item_name: String,
}

impl Registrar {
    /// Subscribes to the watcher's ownership changes (before registering, so
    /// that a watcher starting in between is not missed).
    pub async fn new(connection: &Connection, item_name: String) -> zbus::Result<Self> {
        let bus = fdo::DBusProxy::new(connection).await?;
        let owner_changes = bus
            .receive_name_owner_changed_with_args(&[(0, WATCHER_NAME)])
            .await?;
        // No property cache: it would issue a `GetAll` nobody needs.
        let watcher = StatusNotifierWatcherProxy::builder(connection)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        Ok(Self {
            watcher,
            owner_changes,
            item_name,
        })
    }

    /// Registers the item; logs (does not fail) when no watcher runs yet.
    pub async fn register(&self) {
        match self
            .watcher
            .register_status_notifier_item(&self.item_name)
            .await
        {
            Ok(()) => tracing::debug!(name = %self.item_name, "tray icon registered"),
            Err(error) => tracing::warn!(
                %error,
                "no system tray available (StatusNotifierWatcher), the icon will appear when one starts"
            ),
        }
    }

    /// Re-registers whenever a new watcher takes the name; never returns
    /// unless the connection closes.
    pub async fn follow(mut self) {
        while let Some(change) = self.owner_changes.next().await {
            let appeared = change.args().is_ok_and(|args| args.new_owner().is_some());
            if appeared {
                tracing::info!("system tray (re)started, registering the icon");
                self.register().await;
            } else {
                tracing::info!("system tray stopped, waiting for it to come back");
            }
        }
    }
}
