//! `bergadeck` — Stream Deck manager for Linux.
//!
//! Temporary harness (step 5a): attaches devices, loads their configuration,
//! shows a start-up screen and logs input until Ctrl-C. The page engine
//! replaces the per-device session in the next step.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use bergadeck_manager::config_store::{ConfigStore, EmptyDefaultSettings};
use bergadeck_manager::device::hardware::{HOTPLUG_POLL_INTERVAL, HardwareHotplug};
use bergadeck_manager::device::virtual_deck::{DEMO_SERIAL, VirtualDevices, virtual_plus_info};
use bergadeck_manager::device::{
    DeckBackend, DeviceDiscovery, DiscoveryEvent, InputEvent, StripLayout,
};
use bergadeck_manager::render::{DEFAULT_BACKGROUND, KeyRequest, Renderer};
use clap::Parser;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::{JoinSet, spawn_blocking};
use tracing_subscriber::EnvFilter;

/// Port of the local web UI and WebSocket server.
const DEFAULT_PORT: u16 = 7878;

/// Log level used when `RUST_LOG` is not set.
const DEFAULT_LOG_FILTER: &str = "info";

/// Text shown on every key at start-up.
const SPLASH_LABEL: &str = "Bergadeck";

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

/// Shared services of a device session.
#[derive(Clone)]
struct Services {
    store: ConfigStore,
    renderer: Arc<Renderer>,
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

    let services = Services {
        store: ConfigStore::new(&cli.config_dir, Arc::new(EmptyDefaultSettings)),
        renderer: Arc::new(Renderer::new().context("cannot initialise the renderer")?),
    };
    let discovery = DeviceDiscovery::new();
    let (initial, mut events) = discovery.subscribe();
    let source = start_source(cli.demo, &discovery).await?;
    // The first hardware scan runs right away; report its outcome once done.
    let first_scan = tokio::time::sleep(HOTPLUG_POLL_INTERVAL);
    tokio::pin!(first_scan);
    let mut first_scan_reported = cli.demo;

    let mut sessions = JoinSet::new();
    for info in initial {
        on_attached(&discovery, &services, &mut sessions, &info.serial);
    }
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
            event = events.recv() => match event {
                Ok(DiscoveryEvent::Attached(info)) => {
                    on_attached(&discovery, &services, &mut sessions, &info.serial);
                }
                Ok(DiscoveryEvent::Detached(serial)) => tracing::info!(%serial, "device detached"),
                Err(RecvError::Lagged(missed)) => tracing::warn!(missed, "missed device events"),
                Err(RecvError::Closed) => break,
            },
        }
    }

    shutdown(&discovery, source).await;
    sessions.shutdown().await;
    tracing::info!("bye");
    Ok(())
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

fn on_attached(
    discovery: &DeviceDiscovery,
    services: &Services,
    sessions: &mut JoinSet<()>,
    serial: &str,
) {
    let Some(deck) = discovery.backend(serial) else {
        return;
    };
    let info = deck.info();
    tracing::info!(
        serial = %info.serial,
        model = ?info.model,
        name = %info.display_name,
        keys = info.capabilities.key_count(),
        dials = info.capabilities.dial_count,
        "device attached"
    );
    sessions.spawn(run_session(deck, services.clone()));
}

/// Temporary per-device session: configuration, splash screen, input log.
async fn run_session(deck: Arc<dyn DeckBackend>, services: Services) {
    let mut inputs = deck.subscribe();
    if let Err(error) = prepare_device(deck.as_ref(), &services).await {
        tracing::error!(serial = %deck.info().serial, error = format!("{error:#}"), "device setup failed");
    }
    loop {
        match inputs.recv().await {
            Ok(InputEvent::Disconnected) | Err(RecvError::Closed) => break,
            Ok(event) => tracing::info!(serial = %deck.info().serial, ?event, "input"),
            Err(RecvError::Lagged(missed)) => tracing::warn!(missed, "missed input events"),
        }
    }
    tracing::debug!(serial = %deck.info().serial, "session ended");
}

async fn prepare_device(deck: &dyn DeckBackend, services: &Services) -> Result<()> {
    let info = deck.info().clone();
    let store = services.store.clone();
    let loaded = spawn_blocking(move || store.load(&info)).await??;
    tracing::info!(
        serial = %loaded.config.serial,
        revision = loaded.config.revision,
        outcome = ?loaded.outcome,
        "configuration ready"
    );
    deck.set_brightness(loaded.config.brightness).await?;
    show_splash(deck, &services.renderer).await
}

async fn show_splash(deck: &dyn DeckBackend, renderer: &Arc<Renderer>) -> Result<()> {
    let info = deck.info().clone();
    let renderer = Arc::clone(renderer);
    let (key, zones) = spawn_blocking(move || -> Result<_> {
        let key = renderer.render_key(&KeyRequest::new(
            info.capabilities.key_image_size,
            None,
            Some(SPLASH_LABEL),
        ))?;
        let zones = StripLayout::for_device(&info)
            .map(|layout| {
                (0..layout.zone_count())
                    .map(|zone| {
                        let lines = [SPLASH_LABEL.to_owned(), format!("Dial {}", zone + 1)];
                        renderer
                            .render_zone(&lines, layout.zone_size(), DEFAULT_BACKGROUND)
                            .map(|rendered| (zone, rendered))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        Ok((key, zones))
    })
    .await??;
    for index in 0..deck.info().capabilities.key_count() {
        let index = u8::try_from(index).context("key index out of range")?;
        deck.set_key_image(index, key.image().as_ref().clone())
            .await?;
    }
    for (zone, rendered) in zones {
        deck.set_touch_zone_image(zone, rendered.image().as_ref().clone())
            .await?;
    }
    deck.flush().await?;
    Ok(())
}

/// Blanks every screen, then stops the device source.
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
    async fn demo_device_gets_a_splash_screen() {
        let dir = tempfile::tempdir().unwrap();
        let services = Services {
            store: ConfigStore::new(dir.path(), Arc::new(EmptyDefaultSettings)),
            renderer: Arc::new(Renderer::new().unwrap()),
        };
        let discovery = DeviceDiscovery::new();
        let devices = VirtualDevices::new(discovery.clone());
        let deck = devices.attach(virtual_plus_info(DEMO_SERIAL));
        prepare_device(deck.as_ref(), &services).await.unwrap();
        let screen = deck.screen();
        assert!(screen.keys.iter().all(Option::is_some));
        assert_eq!(screen.brightness, 70);
        assert!(screen.strip.unwrap().pixels().any(|pixel| pixel.0[0] > 128));

        shutdown(&discovery, DeviceSource::Virtual(devices)).await;
        assert!(!deck.is_connected());
        assert!(deck.screen().keys.iter().all(Option::is_none));
    }
}
