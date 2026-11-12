//! `hubergamof open`, run by the desktop launcher: brings the manager back
//! when it is not running (quit from the tray, stopped, given up by systemd
//! after repeated failures), then opens the web interface.
//!
//! The manager is started as its systemd user service, never as a child of
//! the launcher: run outside the service, it would not be supervised and
//! would hold the device against a later start of the service.

use std::net::SocketAddr;
use std::process::ExitStatus;
use std::time::Duration;

use async_trait::async_trait;
use tokio::net::TcpStream;
use tokio::process::Command;
use zbus::Connection;
use zbus::OwnedObjectPath;

use crate::server::ui_url;

/// systemd user unit running the manager (`packaging/systemd/`).
pub const SERVICE_UNIT: &str = "hubergamof.service";

/// Longest wait for a started service to accept connections.
pub const START_TIMEOUT: Duration = Duration::from_secs(10);

/// Delay between two connection attempts while the service starts.
const PROBE_INTERVAL: Duration = Duration::from_millis(100);

/// Job mode of `StartUnit`: replaces a pending stop of the unit.
const REPLACE_JOB_MODE: &str = "replace";

/// Program opening a URL in the user's browser.
const URL_OPENER: &str = "xdg-open";

/// Why the interface could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum LauncherError {
    /// The service manager is unreachable or refused the start.
    #[error("cannot start {SERVICE_UNIT}: {0}")]
    Service(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The service was started but nothing listens on its address.
    #[error(
        "{SERVICE_UNIT} does not answer on {address} after {timeout:?} \
         (see `journalctl --user -u {SERVICE_UNIT}`)"
    )]
    NotListening {
        /// Address of the web interface.
        address: SocketAddr,
        /// How long it was waited for.
        timeout: Duration,
    },
    /// The browser could not be started.
    #[error("cannot run {URL_OPENER}: {0}")]
    OpenerUnavailable(#[source] std::io::Error),
    /// The browser opener reported a failure.
    #[error("{URL_OPENER} failed ({0})")]
    OpenerFailed(ExitStatus),
}

impl From<zbus::Error> for LauncherError {
    fn from(error: zbus::Error) -> Self {
        Self::Service(Box::new(error))
    }
}

/// Starts the manager's service.
#[async_trait]
pub trait ServiceControl: Send + Sync {
    /// Asks for the service to start; it may still be starting on return.
    async fn start(&self) -> Result<(), LauncherError>;
}

/// Shows a URL to the user.
#[async_trait]
pub trait UrlOpener: Send + Sync {
    /// Opens `url`, returning once the opener has handed it over.
    async fn open(&self, url: &str) -> Result<(), LauncherError>;
}

/// What [`open_ui`] did before opening the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The manager was already running.
    AlreadyRunning,
    /// The manager was started.
    Started,
}

/// Opens the interface served at `address`, starting the service first when
/// nothing listens there; waits at most `timeout` for it to listen.
///
/// # Errors
///
/// [`LauncherError`] when the service cannot be started, does not listen in
/// time, or the browser cannot be opened.
pub async fn open_ui(
    address: SocketAddr,
    service: &dyn ServiceControl,
    opener: &dyn UrlOpener,
    timeout: Duration,
) -> Result<Outcome, LauncherError> {
    let outcome = if is_listening(address).await {
        Outcome::AlreadyRunning
    } else {
        service.start().await?;
        wait_for_listener(address, timeout).await?;
        Outcome::Started
    };
    opener.open(&ui_url(address)).await?;
    Ok(outcome)
}

async fn is_listening(address: SocketAddr) -> bool {
    TcpStream::connect(address).await.is_ok()
}

async fn wait_for_listener(address: SocketAddr, timeout: Duration) -> Result<(), LauncherError> {
    let probe = async {
        while !is_listening(address).await {
            tokio::time::sleep(PROBE_INTERVAL).await;
        }
    };
    tokio::time::timeout(timeout, probe)
        .await
        .map_err(|_| LauncherError::NotListening { address, timeout })
}

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait SystemdManager {
    fn start_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn reset_failed_unit(&self, name: &str) -> zbus::Result<()>;
}

/// The manager's service in the user's systemd manager (session bus).
pub struct SystemdUserService;

#[async_trait]
impl ServiceControl for SystemdUserService {
    async fn start(&self) -> Result<(), LauncherError> {
        let connection = Connection::session().await?;
        start_unit(&connection, SERVICE_UNIT).await
    }
}

/// Starts `unit`, clearing first a failed state: after repeated failures
/// systemd refuses to start a unit until it is reset.
async fn start_unit(connection: &Connection, unit: &str) -> Result<(), LauncherError> {
    let manager = SystemdManagerProxy::new(connection).await?;
    if let Err(error) = manager.reset_failed_unit(unit).await {
        // An unknown unit is reported by the start just below.
        tracing::debug!(%error, unit, "cannot reset the failed state");
    }
    manager.start_unit(unit, REPLACE_JOB_MODE).await?;
    Ok(())
}

/// The desktop's default browser, through `xdg-open`, without a shell.
pub struct XdgOpen;

#[async_trait]
impl UrlOpener for XdgOpen {
    async fn open(&self, url: &str) -> Result<(), LauncherError> {
        run_opener(Command::new(URL_OPENER).arg(url)).await
    }
}

async fn run_opener(command: &mut Command) -> Result<(), LauncherError> {
    let status = command
        .status()
        .await
        .map_err(LauncherError::OpenerUnavailable)?;
    if status.success() {
        Ok(())
    } else {
        Err(LauncherError::OpenerFailed(status))
    }
}

#[cfg(test)]
mod tests;
