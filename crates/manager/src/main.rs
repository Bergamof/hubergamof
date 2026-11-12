//! `hubergamof` — Stream Deck manager for Linux.
//!
//! Attaches devices (real or `--demo`), runs a page engine per device, serves
//! the web UI on `127.0.0.1` and shows a tray icon, until Ctrl-C, SIGTERM or
//! the tray's « Quitter »; then closes the UI connections and blanks the
//! screens. `hubergamof open` (desktop launcher) starts the service when it
//! is not running and opens the web interface.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use hubergamof_manager::config_store::ConfigStore;
use hubergamof_manager::device::hardware::{HOTPLUG_POLL_INTERVAL, HardwareHotplug};
use hubergamof_manager::device::virtual_deck::{DEMO_SERIAL, VirtualDevices, virtual_plus_info};
use hubergamof_manager::device::{self, DeviceDiscovery};
use hubergamof_manager::engine::{EngineServices, Supervisor};
use hubergamof_manager::file_chooser::{FilePicker, PortalFilePicker, SimulatedFilePicker};
use hubergamof_manager::launcher::{self, Outcome, SystemdUserService, XdgOpen};
use hubergamof_manager::render::Renderer;
use hubergamof_manager::server::{
    DevOrigin, EmbeddedUi, Server, ServerContext, SimulatedInput, local_address, module_catalog,
};
use hubergamof_manager::tray::{TrayIcon, quit_on};
use hubergamof_modules::clock::SystemClock;
use hubergamof_modules::{BuiltinServices, builtin_registry_with};
use hubergamof_protocol::CatalogModule;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing_subscriber::EnvFilter;

/// Port of the local web UI and WebSocket server.
const DEFAULT_PORT: u16 = 7878;

/// Log level used when `RUST_LOG` is not set.
const DEFAULT_LOG_FILTER: &str = "info";

/// Longest wait for a device to blank its screens on shutdown.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Longest wait for the web server to close its connections on shutdown.
const SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Command-line options.
#[derive(Debug, Parser)]
// The manager's options make no sense with a command and are refused before
// it; the global `--port` goes after it (`hubergamof open --port 9000`).
#[command(version, about, args_conflicts_with_subcommands = true)]
struct Cli {
    /// Use a simulated device instead of real hardware.
    #[arg(long)]
    demo: bool,

    /// Port of the local web interface (bound to 127.0.0.1 only).
    #[arg(long, default_value_t = DEFAULT_PORT, global = true)]
    port: u16,

    /// Directory holding the per-device configuration.
    #[arg(long, default_value = "config")]
    config_dir: PathBuf,

    /// Extra origin allowed to use the interface, e.g. the UI dev server
    /// (`http://localhost:5173`). Repeatable.
    #[arg(long = "dev-origin", value_name = "URL")]
    dev_origins: Vec<DevOrigin>,

    /// Answer this file at once when the interface asks for a file, instead of
    /// opening the desktop's file dialog (tests, screenshots).
    #[arg(long, value_name = "PATH")]
    file_chooser_answer: Option<PathBuf>,

    /// Do not show the system tray icon.
    #[arg(long)]
    no_tray: bool,

    /// Run a command instead of the manager.
    #[command(subcommand)]
    command: Option<Command>,
}

/// Commands run instead of the manager.
#[derive(Debug, Subcommand)]
enum Command {
    /// Open the web interface on `--port`, starting the manager's systemd
    /// user service first when it is not running (desktop launcher).
    Open,
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
    match cli.command {
        Some(Command::Open) => run_open(cli.port).await,
        None => run_manager(cli).await,
    }
}

/// Opens the web interface, bringing the service back first if needed.
async fn run_open(port: u16) -> Result<()> {
    let outcome = launcher::open_ui(
        local_address(port),
        &SystemdUserService,
        &XdgOpen,
        launcher::START_TIMEOUT,
    )
    .await?;
    if outcome == Outcome::Started {
        tracing::info!("{} started", launcher::SERVICE_UNIT);
    }
    Ok(())
}

async fn run_manager(cli: Cli) -> Result<()> {
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        demo = cli.demo,
        port = cli.port,
        config_dir = %cli.config_dir.display(),
        "hubergamof starting"
    );
    let mut terminate = signal(SignalKind::terminate()).context("cannot listen for SIGTERM")?;
    let (shutdown, stopping) = watch::channel(false);

    let services = services(&cli.config_dir, builtin_services(cli.demo))?;
    let catalog = catalog_of(&services).await?;
    let store = services.store.clone();
    let renderer = Arc::clone(&services.renderer);
    let discovery = DeviceDiscovery::new();
    let supervisor = Supervisor::start(&discovery, services);
    let context = ServerContext {
        discovery: discovery.clone(),
        engines: supervisor.engines(),
        catalog,
        simulated_input: simulated_input(cli.demo),
        store,
        renderer,
        file_picker: file_picker(cli.file_chooser_answer.clone()),
    };
    let server = match Server::bind(
        cli.port,
        &cli.dev_origins,
        context,
        Arc::new(EmbeddedUi),
        stopping.clone(),
    )
    .await
    {
        Ok(server) => server,
        Err(error) => {
            supervisor.shutdown().await;
            return Err(error).context("cannot start the web interface");
        }
    };
    let url = server.url();
    tracing::info!(%url, "web interface ready");
    let mut server_task = tokio::spawn(server.run());
    let source = match start_source(cli.demo, &discovery).await {
        Ok(source) => source,
        Err(error) => {
            shutdown.send_replace(true);
            stop_server(server_task).await;
            supervisor.shutdown().await;
            return Err(error);
        }
    };
    if !cli.demo {
        tokio::spawn(report_first_scan(discovery.clone()));
    }
    let tray = if cli.no_tray {
        None
    } else {
        TrayIcon::start(url, &discovery, quit_on(shutdown.clone())).await
    };

    let outcome = tokio::select! {
        signal = tokio::signal::ctrl_c() => signal.context("cannot listen for Ctrl-C"),
        _ = terminate.recv() => Ok(()),
        () = wait_for_quit(stopping) => Ok(()),
        result = &mut server_task => server_outcome(result),
    };
    tracing::info!("shutting down");
    shutdown.send_replace(true);
    stop_server(server_task).await;
    supervisor.shutdown().await;
    shutdown_devices(&discovery, source).await;
    if let Some(tray) = tray {
        tray.stop().await;
    }
    tracing::info!("bye");
    outcome
}

/// Resolves when the tray's « Quitter » (or anything else) requests shutdown.
async fn wait_for_quit(mut stopping: watch::Receiver<bool>) {
    let _ = stopping.wait_for(|stopping| *stopping).await;
}

/// Tells whether the first hardware scan (run right away) found a device.
async fn report_first_scan(discovery: DeviceDiscovery) {
    tokio::time::sleep(HOTPLUG_POLL_INTERVAL).await;
    if discovery.devices().is_empty() {
        tracing::info!("no Stream Deck connected, waiting for one to be plugged in");
    }
}

fn server_outcome(result: Result<std::io::Result<()>, tokio::task::JoinError>) -> Result<()> {
    match result {
        Ok(Ok(())) => anyhow::bail!("the web server stopped unexpectedly"),
        Ok(Err(error)) => Err(error).context("the web server failed"),
        Err(error) => Err(error).context("the web server task failed"),
    }
}

async fn stop_server(server_task: JoinHandle<std::io::Result<()>>) {
    let abort = server_task.abort_handle();
    match tokio::time::timeout(SERVER_SHUTDOWN_TIMEOUT, server_task).await {
        // A task error was already reported, or the task aborted.
        Ok(Ok(Ok(())) | Err(_)) => {}
        Ok(Ok(Err(error))) => tracing::warn!(%error, "web server failed"),
        Err(_) => {
            tracing::warn!("timed out closing web connections");
            abort.abort();
        }
    }
}

/// What the modules act on: the demo is hermetic (simulated audio…), so that
/// the UI, its tests and Lighthouse never touch the machine.
/// Only the demo, on virtual devices, accepts presses from the web
/// interface: on real devices, input comes from the device only.
fn simulated_input(demo: bool) -> SimulatedInput {
    if demo {
        SimulatedInput::Accepted
    } else {
        SimulatedInput::Refused
    }
}

/// File dialog of the UI: the desktop's own (also with `--demo`, so that `make dev` tries
/// the real one), or a user always choosing `answer` when one is given.
fn file_picker(answer: Option<PathBuf>) -> Arc<dyn FilePicker> {
    match answer {
        Some(file) => Arc::new(SimulatedFilePicker::choosing(Some(file))),
        None => Arc::new(PortalFilePicker),
    }
}

fn builtin_services(demo: bool) -> BuiltinServices {
    if demo {
        // A demo shows a living clock: only the tests freeze the time.
        BuiltinServices {
            wall_clock: Arc::new(SystemClock),
            ..BuiltinServices::simulated()
        }
    } else {
        BuiltinServices::system()
    }
}

/// Module catalog of the UI, its icons rendered off the async runtime.
async fn catalog_of(services: &EngineServices) -> Result<Arc<[CatalogModule]>> {
    let registry = Arc::clone(&services.registry);
    let renderer = Arc::clone(&services.renderer);
    tokio::task::spawn_blocking(move || module_catalog(&registry, &renderer))
        .await
        .context("cannot build the module catalog")
}

fn services(config_dir: &std::path::Path, builtins: BuiltinServices) -> Result<EngineServices> {
    let registry = Arc::new(builtin_registry_with(builtins));
    Ok(EngineServices {
        store: ConfigStore::new(config_dir, Arc::clone(&registry)),
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
async fn shutdown_devices(discovery: &DeviceDiscovery, source: DeviceSource) {
    for deck in discovery.backends() {
        device::blank(deck.as_ref(), SHUTDOWN_TIMEOUT).await;
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
    use hubergamof_manager::device::DeckBackend as _;
    use hubergamof_manager::engine::LiveSnapshot;

    use super::*;

    /// Dial of the audio module in the default layout.
    const AUDIO_DIAL_INDEX: u8 = 1;
    const AUDIO_DIAL: usize = AUDIO_DIAL_INDEX as usize;
    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn only_the_demo_accepts_simulated_input() {
        assert_eq!(simulated_input(true), SimulatedInput::Accepted);
        assert_eq!(simulated_input(false), SimulatedInput::Refused);
    }

    #[test]
    fn cli_definition_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn defaults_apply_without_arguments() {
        let cli = Cli::try_parse_from(["hubergamof"]).unwrap();
        assert!(!cli.demo);
        assert_eq!(cli.port, DEFAULT_PORT);
        assert_eq!(cli.config_dir, PathBuf::from("config"));
        assert_eq!(cli.dev_origins, []);
        assert!(!cli.no_tray);
        assert!(cli.command.is_none(), "runs the manager");
        assert_eq!(
            cli.file_chooser_answer, None,
            "the desktop's own dialog, also in --demo"
        );
    }

    #[test]
    fn the_file_chooser_can_be_answered_in_advance() {
        let cli =
            Cli::try_parse_from(["hubergamof", "--file-chooser-answer", "/srv/a.yaml"]).unwrap();
        assert_eq!(cli.file_chooser_answer, Some(PathBuf::from("/srv/a.yaml")));
    }

    #[test]
    fn options_are_parsed() {
        let cli = Cli::try_parse_from([
            "hubergamof",
            "--demo",
            "--port",
            "9000",
            "--config-dir",
            "/tmp/deck",
            "--dev-origin",
            "http://localhost:5173",
            "--dev-origin",
            "http://127.0.0.1:5173/",
            "--no-tray",
        ])
        .unwrap();
        assert!(cli.demo);
        assert!(cli.no_tray);
        let origins: Vec<_> = cli.dev_origins.iter().map(DevOrigin::as_str).collect();
        assert_eq!(origins, ["http://localhost:5173", "http://127.0.0.1:5173"]);
        assert_eq!(cli.port, 9000);
        assert_eq!(cli.config_dir, PathBuf::from("/tmp/deck"));
    }

    #[test]
    fn open_uses_the_port_of_the_manager() {
        let cli = Cli::try_parse_from(["hubergamof", "open"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Open)));
        assert_eq!(cli.port, DEFAULT_PORT);
        let cli = Cli::try_parse_from(["hubergamof", "open", "--port", "9000"]).unwrap();
        assert_eq!(cli.port, 9000);
    }

    #[test]
    fn open_refuses_the_options_of_the_manager() {
        for option in [
            &["--demo"][..],
            &["--no-tray"],
            &["--config-dir", "/tmp/deck"],
            &["--dev-origin", "http://localhost:5173"],
            &["--file-chooser-answer", "/srv/a.yaml"],
            &["--port", "9000"],
        ] {
            let args = [&["hubergamof"][..], option, &["open"]].concat();
            assert!(Cli::try_parse_from(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn invalid_port_is_rejected() {
        assert!(Cli::try_parse_from(["hubergamof", "--port", "70000"]).is_err());
    }

    #[test]
    fn invalid_dev_origin_is_rejected() {
        assert!(Cli::try_parse_from(["hubergamof", "--dev-origin", "localhost:5173"]).is_err());
    }

    #[tokio::test]
    async fn demo_device_shows_the_home_page_until_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let discovery = DeviceDiscovery::new();
        let supervisor = Supervisor::start(
            &discovery,
            services(dir.path(), builtin_services(true)).unwrap(),
        );
        let devices = VirtualDevices::new(discovery.clone());
        let engines = supervisor.engines();
        let mut changes = engines.changes();
        let deck = devices.attach(virtual_plus_info(DEMO_SERIAL));
        let engine = tokio::time::timeout(WAIT, async {
            loop {
                if let Some(engine) = engines.get(DEMO_SERIAL) {
                    break engine;
                }
                changes.changed().await.unwrap();
            }
        })
        .await
        .expect("the demo engine starts");
        assert_eq!(engine.live().borrow().page.title, "Accueil");
        let screen = deck.screen();
        assert!(screen.keys.iter().all(Option::is_some));
        assert_eq!(screen.brightness, 70);
        assert!(screen.strip.unwrap().pixels().any(|pixel| pixel.0[0] > 128));

        // The audio dial drives the simulated server, never the machine's.
        let audio_zone = |volume: &str| {
            let lines = vec!["Haut-parleurs".to_owned(), volume.to_owned()];
            move |live: &LiveSnapshot| {
                live.dials[AUDIO_DIAL]
                    .zone
                    .as_ref()
                    .is_some_and(|zone| zone.lines == lines)
            }
        };
        let mut live = engine.live();
        tokio::time::timeout(WAIT, live.wait_for(audio_zone("40 %")))
            .await
            .unwrap()
            .unwrap();
        engine.rotate_dial(AUDIO_DIAL_INDEX, 2).await.unwrap();
        tokio::time::timeout(WAIT, live.wait_for(audio_zone("50 %")))
            .await
            .unwrap()
            .unwrap();

        supervisor.shutdown().await;
        assert!(deck.screen().keys.iter().all(Option::is_none));
        shutdown_devices(&discovery, DeviceSource::Virtual(devices)).await;
        assert!(!deck.is_connected());
    }
}
