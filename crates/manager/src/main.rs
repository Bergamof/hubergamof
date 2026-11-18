//! `hubergamof` — Stream Deck manager for Linux.
//!
//! Attaches devices (real or `--demo`), runs a page engine per device, serves
//! the web UI on `127.0.0.1` and shows a tray icon, until Ctrl-C, SIGTERM or
//! the tray's « Quitter »; then closes the UI connections and blanks the
//! screens. `hubergamof open` (desktop launcher) starts the service when it
//! is not running and opens the web interface.

use std::io::Write as _;
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
use hubergamof_manager::screen_lock::{LogindScreenLock, NoScreenLock, ScreenLock};
use hubergamof_manager::server::{
    DevOrigin, EmbeddedUi, Server, ServerContext, SimulatedInput, local_address, module_catalog,
};
use hubergamof_manager::strip::{ProcProbe, SimulatedProbe, SystemClock, SystemProbe};
use hubergamof_manager::teams_extension::{self, Browser, Locations};
use hubergamof_manager::tray::{TrayIcon, quit_on};
use hubergamof_manager::update::{
    self, FIRST_CHECK_DELAY, GithubReleases, ReleaseSource, SimulatedReleases, UpdateHandle,
    Version,
};
use hubergamof_modules::teams;
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

    /// Relay the browser extension to the manager's Teams module (native
    /// messaging host, started by the browser, not by hand). Speaks frames
    /// on stdin and stdout; the origin the browser passes must be the extension's.
    NativeHost {
        /// Extension origin passed by the browser.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        origin: Vec<String>,
    },

    /// Install the Teams browser extension for the current user: copies it
    /// to the data directory and declares the native messaging host to the
    /// browser. The extension is then loaded once from `brave://extensions`
    /// ("Load unpacked").
    InstallExtension {
        /// Browser to declare the host to.
        #[arg(long, value_enum, default_value_t = Browser::Brave)]
        browser: Browser,
    },
}

/// Where devices come from.
enum DeviceSource {
    Virtual(VirtualDevices),
    Hardware(HardwareHotplug),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse_from(teams_extension::with_browser_launch_command(
        std::env::args(),
    ));
    init_tracing(match cli.command {
        // The native host owns stdout: its frames, nothing else.
        Some(Command::NativeHost { .. }) => LogOutput::Stderr,
        Some(Command::Open | Command::InstallExtension { .. }) | None => LogOutput::Stdout,
    });
    match cli.command {
        Some(Command::Open) => run_open(cli.port).await,
        Some(Command::NativeHost { origin }) => exit_with(run_native_host(&origin).await),
        Some(Command::InstallExtension { browser }) => install_extension(browser),
        None => run_manager(cli).await,
    }
}

/// Ends the process now, without letting the runtime shut down.
///
/// The native host reads stdin on a blocking thread that only returns when the
/// browser closes the pipe: a normal return would wait for it and leave a host
/// no longer relaying anything (and so a silent extension) after the manager
/// restarted. Exiting makes the browser notice and start a new host.
fn exit_with(outcome: Result<()>) -> ! {
    let code = match outcome {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Error: {error:?}");
            1
        }
    };
    std::process::exit(code)
}

/// Installs the Teams extension and its native messaging host.
fn install_extension(browser: Browser) -> Result<()> {
    let binary = std::env::current_exe().context("cannot find the path of this program")?;
    let installed = teams_extension::install(&Locations::from_env()?, &binary, browser)?;
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "Extension {} written to {}",
        installed.version,
        installed.extension_dir.display()
    )?;
    writeln!(
        out,
        "Native host declared in {}",
        installed.host_manifest.display()
    )?;
    writeln!(out, "Extension id: {}", installed.extension_id)?;
    writeln!(
        out,
        "Last step, once: open the browser's extensions page, turn on developer mode, \
         choose \"Load unpacked\" and select the extension folder above."
    )?;
    Ok(())
}

/// Relays the browser's frames on stdio to the manager's Teams socket.
async fn run_native_host(origin: &[String]) -> Result<()> {
    // The browser already enforces `allowed_origins`; this is a second lock.
    if let Some(origin) = origin.first() {
        anyhow::ensure!(
            teams_extension::is_own_origin(origin)?,
            "started for an origin that is not the HuBergamof extension's"
        );
    }
    let path = teams::socket_path().context("no runtime directory for the manager's socket")?;
    let stream = tokio::net::UnixStream::connect(&path)
        .await
        .with_context(|| format!("the manager is not listening on {}", path.display()))?;
    // The reports carry names: only a manager of this user gets them.
    anyhow::ensure!(
        teams::is_same_user(&stream),
        "the socket {} is served by another user",
        path.display()
    );
    let (manager_in, manager_out) = stream.into_split();
    teams::native_host::relay(
        tokio::io::stdin(),
        tokio::io::stdout(),
        manager_in,
        manager_out,
    )
    .await
    .context("the relay to the manager failed")
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

    let services = services(
        &cli.config_dir,
        builtin_services(cli.demo),
        system_probe(cli.demo),
        screen_lock(cli.demo),
    )?;
    let catalog = catalog_of(&services).await?;
    let store = services.store.clone();
    let renderer = Arc::clone(&services.renderer);
    let discovery = DeviceDiscovery::new();
    let supervisor = Supervisor::start(&discovery, services);
    let updates = start_update_check(cli.demo, stopping.clone());
    let context = ServerContext {
        discovery: discovery.clone(),
        engines: supervisor.engines(),
        catalog,
        simulated_input: simulated_input(cli.demo),
        store,
        renderer,
        file_picker: file_picker(cli.file_chooser_answer.clone()),
        updates: updates.clone(),
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
        TrayIcon::start(
            url,
            &discovery,
            supervisor.engines(),
            updates,
            quit_on(shutdown.clone()),
        )
        .await
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

/// Release announced by `--demo`: far above any real one, so the interface
/// always shows an available update.
const DEMO_RELEASE: Version = Version::new(999, 0, 0);

/// Starts the periodic check for a newer release. `--demo` never touches the
/// network: it announces [`DEMO_RELEASE`] at once.
fn start_update_check(demo: bool, stopping: watch::Receiver<bool>) -> UpdateHandle {
    let current = match Version::current() {
        Ok(current) => current,
        Err(error) => {
            tracing::warn!(%error, "update check disabled");
            return UpdateHandle::inactive();
        }
    };
    let (source, first_delay): (Arc<dyn ReleaseSource>, Duration) = if demo {
        (
            Arc::new(SimulatedReleases::announcing(DEMO_RELEASE)),
            Duration::ZERO,
        )
    } else {
        match GithubReleases::new() {
            Ok(source) => (Arc::new(source), FIRST_CHECK_DELAY),
            Err(error) => {
                tracing::warn!(%error, "update check disabled");
                return UpdateHandle::inactive();
            }
        }
    };
    // The task ends with the shutdown signal; nothing to wait for.
    update::start(source, current, first_delay, stopping).0
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

/// What the modules act on: the demo is hermetic (simulated audio…), so that
/// the UI, its tests and Lighthouse never touch the machine.
fn builtin_services(demo: bool) -> BuiltinServices {
    if demo {
        BuiltinServices::simulated()
    } else {
        BuiltinServices::system()
    }
}

/// Whether the computer is locked: read from logind, never in the demo (a
/// development machine locking must not blank the demo device).
fn screen_lock(demo: bool) -> Arc<dyn ScreenLock> {
    if demo {
        Arc::new(NoScreenLock)
    } else {
        Arc::new(LogindScreenLock::start())
    }
}

/// What the touch strip measures: the machine, or lively figures for the
/// demo. Both show the real time: only the tests freeze it.
fn system_probe(demo: bool) -> Arc<dyn SystemProbe> {
    if demo {
        Arc::new(SimulatedProbe::demo())
    } else {
        Arc::new(ProcProbe::new())
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

fn services(
    config_dir: &std::path::Path,
    builtins: BuiltinServices,
    system: Arc<dyn SystemProbe>,
    screen_lock: Arc<dyn ScreenLock>,
) -> Result<EngineServices> {
    let registry = Arc::new(builtin_registry_with(builtins));
    Ok(EngineServices {
        store: ConfigStore::new(config_dir, Arc::clone(&registry)),
        renderer: Arc::new(Renderer::new().context("cannot initialise the renderer")?),
        registry,
        wall_clock: Arc::new(SystemClock),
        system,
        screen_lock,
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

/// Where the logs go.
#[derive(Debug, Clone, Copy)]
enum LogOutput {
    Stdout,
    /// For a command whose stdout carries something else.
    Stderr,
}

fn init_tracing(output: LogOutput) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match output {
        LogOutput::Stdout => builder.init(),
        LogOutput::Stderr => builder.with_writer(std::io::stderr).init(),
    }
}

#[cfg(test)]
mod tests {
    use hubergamof_manager::device::DeckBackend as _;
    use hubergamof_manager::engine::LiveSnapshot;
    use hubergamof_module_api::VolumeWidget;

    use super::*;

    /// Dial of the audio module in the default layout.
    /// The volume is the first dial of the default layout.
    const VOLUME_DIAL: u8 = 0;
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
    fn the_native_host_accepts_the_origin_the_browser_passes() {
        let cli = Cli::try_parse_from([
            "hubergamof",
            "native-host",
            "chrome-extension://abcdefghijklmnopabcdefghijklmnop/",
        ])
        .unwrap();
        assert!(matches!(cli.command, Some(Command::NativeHost { .. })));
        let cli = Cli::try_parse_from(["hubergamof", "native-host"]).unwrap();
        assert!(matches!(cli.command, Some(Command::NativeHost { .. })));
    }

    #[test]
    fn install_extension_defaults_to_brave() {
        let cli = Cli::try_parse_from(["hubergamof", "install-extension"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::InstallExtension {
                browser: Browser::Brave
            })
        ));
        let cli = Cli::try_parse_from(["hubergamof", "install-extension", "--browser", "chromium"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::InstallExtension {
                browser: Browser::Chromium
            })
        ));
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
            services(
                dir.path(),
                builtin_services(true),
                system_probe(true),
                screen_lock(true),
            )
            .unwrap(),
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

        // The volume dial drives the simulated server, never the machine's.
        let volume = |percent: u16| {
            move |live: &LiveSnapshot| {
                live.strip.as_ref().is_some_and(|strip| {
                    strip.face.volume == Some(VolumeWidget::new(percent, false))
                })
            }
        };
        let mut live = engine.live();
        tokio::time::timeout(WAIT, live.wait_for(volume(40)))
            .await
            .unwrap()
            .unwrap();
        engine.rotate_dial(VOLUME_DIAL, 2).await.unwrap();
        tokio::time::timeout(WAIT, live.wait_for(volume(50)))
            .await
            .unwrap()
            .unwrap();

        supervisor.shutdown().await;
        assert!(deck.screen().keys.iter().all(Option::is_none));
        shutdown_devices(&discovery, DeviceSource::Virtual(devices)).await;
        assert!(!deck.is_connected());
    }
}
