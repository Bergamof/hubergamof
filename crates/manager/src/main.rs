//! `bergadeck` — Stream Deck manager for Linux.
//!
//! Attaches devices (real or `--demo`), runs a page engine per device until
//! Ctrl-C, then blanks the screens.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use bergadeck_manager::config_store::ConfigStore;
use bergadeck_manager::device::DeviceDiscovery;
use bergadeck_manager::device::hardware::{HOTPLUG_POLL_INTERVAL, HardwareHotplug};
use bergadeck_manager::device::virtual_deck::{DEMO_SERIAL, VirtualDevices, virtual_plus_info};
use bergadeck_manager::engine::{EngineServices, Supervisor};
use bergadeck_manager::render::Renderer;
use bergadeck_modules::builtin_registry;
use clap::Parser;
use tracing_subscriber::EnvFilter;

/// Port of the local web UI and WebSocket server.
const DEFAULT_PORT: u16 = 7878;

/// Log level used when `RUST_LOG` is not set.
const DEFAULT_LOG_FILTER: &str = "info";

/// Longest wait for a device to blank its screens on shutdown.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Command-line options.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Use a simulated device instead of real hardware.
    #[arg(long)]
    demo: bool,

    /// Port of the local web interface (bound to 127.0.0.1 only).
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,

    /// Directory holding the per-device configuration.
    #[arg(long, default_value = "config")]
    config_dir: PathBuf,
}

/// Where devices come from.
enum DeviceSource {
    Virtual(VirtualDevices),
    Hardware(HardwareHotplug),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        demo = cli.demo,
        port = cli.port,
        config_dir = %cli.config_dir.display(),
        "bergadeck starting"
    );

    let services = services(&cli.config_dir)?;
    let discovery = DeviceDiscovery::new();
    let supervisor = Supervisor::start(&discovery, services);
    let source = start_source(cli.demo, &discovery).await?;
    // The first hardware scan runs right away; report its outcome once done.
    let first_scan = tokio::time::sleep(HOTPLUG_POLL_INTERVAL);
    tokio::pin!(first_scan);
    let mut first_scan_reported = cli.demo;
    loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal.context("cannot listen for Ctrl-C")?;
                tracing::info!("shutting down");
                break;
            }
            () = &mut first_scan, if !first_scan_reported => {
                first_scan_reported = true;
                if discovery.devices().is_empty() {
                    tracing::info!("no Stream Deck connected, waiting for one to be plugged in");
                }
            }
        }
    }

    supervisor.shutdown().await;
    shutdown(&discovery, source).await;
    tracing::info!("bye");
    Ok(())
}

fn services(config_dir: &std::path::Path) -> Result<EngineServices> {
    let registry = Arc::new(builtin_registry());
    Ok(EngineServices {
        store: ConfigStore::new(config_dir, Arc::clone(&registry) as _),
        renderer: Arc::new(Renderer::new().context("cannot initialise the renderer")?),
        registry,
    })
}

async fn start_source(demo: bool, discovery: &DeviceDiscovery) -> Result<DeviceSource> {
    if demo {
        let devices = VirtualDevices::new(discovery.clone());
        let _demo_deck = devices.attach(virtual_plus_info(DEMO_SERIAL));
        Ok(DeviceSource::Virtual(devices))
    } else {
        let hotplug = HardwareHotplug::start(discovery.clone())
            .await
            .context("cannot initialise HID access")?;
        Ok(DeviceSource::Hardware(hotplug))
    }
}

/// Blanks every screen (engines already did, unless one was still starting),
/// then stops the device source.
async fn shutdown(discovery: &DeviceDiscovery, source: DeviceSource) {
    for deck in discovery.backends() {
        let blank = async {
            deck.clear_all().await?;
            deck.flush().await
        };
        match tokio::time::timeout(SHUTDOWN_TIMEOUT, blank).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(serial = %deck.info().serial, %error, "cannot blank device");
            }
            Err(_) => tracing::warn!(serial = %deck.info().serial, "timed out blanking device"),
        }
    }
    match source {
        DeviceSource::Hardware(hotplug) => hotplug.stop().await,
        DeviceSource::Virtual(devices) => {
            for info in discovery.devices() {
                devices.detach(&info.serial).await;
            }
        }
    }
}

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[cfg(test)]
mod tests {
    use bergadeck_manager::device::DeckBackend as _;

    use super::*;

    #[test]
    fn cli_definition_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn defaults_apply_without_arguments() {
        let cli = Cli::try_parse_from(["bergadeck"]).unwrap();
        assert!(!cli.demo);
        assert_eq!(cli.port, DEFAULT_PORT);
        assert_eq!(cli.config_dir, PathBuf::from("config"));
    }

    #[test]
    fn options_are_parsed() {
        let cli = Cli::try_parse_from([
            "bergadeck",
            "--demo",
            "--port",
            "9000",
            "--config-dir",
            "/tmp/deck",
        ])
        .unwrap();
        assert!(cli.demo);
        assert_eq!(cli.port, 9000);
        assert_eq!(cli.config_dir, PathBuf::from("/tmp/deck"));
    }

    #[test]
    fn invalid_port_is_rejected() {
        assert!(Cli::try_parse_from(["bergadeck", "--port", "70000"]).is_err());
    }

    #[tokio::test]
    async fn demo_device_shows_the_home_page_until_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let discovery = DeviceDiscovery::new();
        let supervisor = Supervisor::start(&discovery, services(dir.path()).unwrap());
        let devices = VirtualDevices::new(discovery.clone());
        let deck = devices.attach(virtual_plus_info(DEMO_SERIAL));
        let engine = loop {
            if let Some(engine) = supervisor.engines().get(DEMO_SERIAL) {
                break engine;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        assert_eq!(engine.live().borrow().page.title, "Accueil");
        let screen = deck.screen();
        assert!(screen.keys.iter().all(Option::is_some));
        assert_eq!(screen.brightness, 70);
        assert!(screen.strip.unwrap().pixels().any(|pixel| pixel.0[0] > 128));

        supervisor.shutdown().await;
        assert!(deck.screen().keys.iter().all(Option::is_none));
        shutdown(&discovery, DeviceSource::Virtual(devices)).await;
        assert!(!deck.is_connected());
    }
}
