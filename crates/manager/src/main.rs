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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use hubergamof_manager::config_store::ConfigStore;
use hubergamof_manager::device::hardware::{HOTPLUG_POLL_INTERVAL, HardwareHotplug};
use hubergamof_manager::device::virtual_deck::{DEMO_SERIAL, VirtualDevices, virtual_plus_info};
use hubergamof_manager::device::{self, DeviceDiscovery};
use hubergamof_manager::engine::{EngineRegistry, EngineServices, Supervisor, first_radial_items};
use hubergamof_manager::file_chooser::{FilePicker, PortalFilePicker, SimulatedFilePicker};
use hubergamof_manager::haptics::{HapticOutput, HapticRelay};
use hubergamof_manager::launcher::{self, Outcome, SystemdUserService, XdgOpen};
use hubergamof_manager::machine::git::{GitConfigHub, GitFiles};
use hubergamof_manager::machine::{MachineHub, MachineShell};
use hubergamof_manager::module_host::{HostServices, ModuleHost};
use hubergamof_manager::mouse_actions::ButtonActions;
use hubergamof_manager::mouse_engine::{HidrawSource, MouseSupervisor, SCAN_INTERVAL};
use hubergamof_manager::mouse_hub::{MouseHub, MouseRegistration};
use hubergamof_manager::radial::{
    self,
    actions::ModuleActions,
    looks::{Items, Looks, PetalNotes},
    service::{Presentation, RadialService},
    window::PetalNote,
};
use hubergamof_manager::render::Renderer;
use hubergamof_manager::screen_lock::{LogindScreenLock, NoScreenLock, ScreenLock};
use hubergamof_manager::server::{
    DevOrigin, EmbeddedUi, Server, ServerContext, SimulatedInput, local_address, module_catalog,
};
use hubergamof_manager::strip::{ProcProbe, SimulatedProbe, SystemClock, SystemProbe};
use hubergamof_manager::teams_extension::{self, Browser, Locations};
use hubergamof_manager::tray::{TrayIcon, quit_on};
use hubergamof_manager::update::{
    self, Downloader, FIRST_CHECK_DELAY, FilePolicyStore, GithubReleases, Installed, Installer,
    MemoryPolicyStore, PackageInstaller, SimulatedInstaller, SimulatedReleases, SocketChannel,
    SystemPackages, UnsupportedInstaller, UpdateHandle, UpdateParts, UpdatePolicy, Version, wire,
};
use hubergamof_modules::teams;
use hubergamof_modules::{BuiltinServices, builtin_registry_with};
use hubergamof_mouse::diagnostics::{self, Action, parse_waveform};
use hubergamof_protocol::CatalogModule;
use hubergamof_protocol::{
    DPI_RANGE, DPI_STEP, HapticLevel, HapticWaveform, MouseBattery, MouseInfo, MouseModel,
    WHEEL_THRESHOLD_RANGE, WheelMode,
};
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

/// Longest wait for the task of the module host to end once it was told to
/// stop.
const HOST_TASK_TIMEOUT: Duration = Duration::from_secs(1);

/// Longest wait for the web server to close its connections on shutdown.
const SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Command-line options.
#[derive(Debug, Parser)]
// The manager's options make no sense with a command and are refused before
// it; the global `--port` goes after it (`hubergamof open --port 9000`).
#[command(version, about, args_conflicts_with_subcommands = true)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each command-line switch is its own flag"
)]
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

    /// Serve Logitech MX mice (Logi Bolt receiver): apply their configuration
    /// and take over their buttons. On by default, so that this is the same as
    /// not giving the option; kept so that existing command lines still work.
    #[arg(long, conflicts_with = "no_mice")]
    mice: bool,

    /// Do not touch the Logitech mice: no configuration applied, no button
    /// taken over (to run Solaar or logiops, which divert the same buttons).
    #[arg(long)]
    no_mice: bool,

    /// Never look for a newer release: the manager then makes no connection
    /// to the Internet at all.
    #[arg(long)]
    no_update_check: bool,

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

    /// Try a Logitech mouse: read its state or make one change you ask for
    /// (sensitivity, wheel mode, a haptic waveform, button diversion). The
    /// mouse is looked for on the Logi Bolt receiver; move it to wake it.
    MouseTest {
        /// Which mouse, when more than one answers.
        #[arg(long, value_enum)]
        model: Option<MouseModelArg>,
        #[command(subcommand)]
        action: MouseTestAction,
    },

    /// Show the radial menu for a few seconds over the desktop (Wayland with
    /// layer-shell: KDE, wlroots compositors) to try its window: hover lights
    /// a petal, a click chooses it, Escape closes.
    RadialPreview {
        /// How long the menu stays, in seconds.
        #[arg(
            long,
            default_value_t = 15,
            value_parser = clap::value_parser!(u64).range(1..=PREVIEW_MAX_SECONDS)
        )]
        seconds: u64,
    },
}

/// Mouse models `mouse-test` knows.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum MouseModelArg {
    /// Logitech MX Master 4.
    Master4,
    /// Logitech MX Anywhere 3S.
    Anywhere3s,
}

impl From<MouseModelArg> for MouseModel {
    fn from(model: MouseModelArg) -> Self {
        match model {
            MouseModelArg::Master4 => Self::MxMaster4,
            MouseModelArg::Anywhere3s => Self::MxAnywhere3S,
        }
    }
}

/// Haptic strengths `mouse-test` knows.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum HapticLevelArg {
    /// No feedback.
    Off,
    /// Barely felt.
    Subtle,
    /// Light.
    Low,
    /// Medium.
    Medium,
    /// Strong.
    High,
}

impl From<HapticLevelArg> for HapticLevel {
    fn from(level: HapticLevelArg) -> Self {
        match level {
            HapticLevelArg::Off => Self::Off,
            HapticLevelArg::Subtle => Self::Subtle,
            HapticLevelArg::Low => Self::Low,
            HapticLevelArg::Medium => Self::Medium,
            HapticLevelArg::High => Self::High,
        }
    }
}

/// Wheel modes `mouse-test` knows.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum WheelModeArg {
    /// One notch at a time.
    Ratchet,
    /// Free spin.
    Free,
    /// Free spin once thrown fast enough.
    Auto,
}

impl From<WheelModeArg> for WheelMode {
    fn from(mode: WheelModeArg) -> Self {
        match mode {
            WheelModeArg::Ratchet => Self::Ratchet,
            WheelModeArg::Free => Self::Free,
            WheelModeArg::Auto => Self::Auto,
        }
    }
}

/// What `mouse-test` does.
#[derive(Debug, Subcommand)]
enum MouseTestAction {
    /// Read and print the state of the mouse (nothing is changed).
    Info,
    /// Play a haptic waveform (`knock`, `ringing`, `sharp-state-change`…).
    Haptic {
        /// Waveform name.
        #[arg(value_parser = parse_waveform_arg)]
        waveform: HapticWaveform,
        /// Set this strength first.
        #[arg(long, value_enum)]
        level: Option<HapticLevelArg>,
    },
    /// Set the sensitivity, 200 to 8000 dpi in steps of 50.
    Dpi {
        /// Dots per inch.
        #[arg(value_parser = parse_dpi)]
        value: u16,
    },
    /// Set how the wheel turns.
    Wheel {
        /// Mode.
        #[arg(value_enum)]
        mode: WheelModeArg,
        /// Speed from which the automatic mode frees the wheel, 1 to 50.
        #[arg(long, default_value_t = DEFAULT_WHEEL_THRESHOLD, value_parser = parse_wheel_threshold)]
        threshold: u8,
    },
    /// Divert the buttons and print their presses, then give them back.
    Buttons {
        /// How long to watch, in seconds.
        #[arg(long, default_value_t = DEFAULT_WATCH_SECONDS, value_parser = clap::value_parser!(u64).range(1..=MAX_WATCH_SECONDS))]
        seconds: u64,
    },
    /// Print every frame the receiver sends, changing nothing (what a mouse that sleeps or wakes announces).
    Events {
        /// How long to listen, in seconds.
        #[arg(long, default_value_t = DEFAULT_EVENTS_SECONDS, value_parser = clap::value_parser!(u64).range(1..=MAX_WATCH_SECONDS))]
        seconds: u64,
    },
    /// Give back every diverted button (after a crash).
    Release,
}

/// Threshold of the automatic wheel mode when none is given.
const DEFAULT_WHEEL_THRESHOLD: u8 = 20;
/// Longest `radial-preview`: its window holds the keyboard meanwhile.
const PREVIEW_MAX_SECONDS: u64 = 120;

/// Length of `mouse-test buttons` when none is given, and its longest.
const DEFAULT_WATCH_SECONDS: u64 = 20;
const MAX_WATCH_SECONDS: u64 = 300;
/// Length of `mouse-test events` when none is given: long enough for a mouse to fall asleep.
const DEFAULT_EVENTS_SECONDS: u64 = 120;

/// Refuses a sensitivity the mouse would refuse, before looking for it.
fn parse_dpi(text: &str) -> Result<u16, String> {
    let dpi: u16 = text
        .parse()
        .map_err(|_| format!("{text:?} is not a number"))?;
    if DPI_RANGE.contains(&dpi) && (dpi - DPI_RANGE.start()).is_multiple_of(DPI_STEP) {
        Ok(dpi)
    } else {
        Err(format!(
            "{} to {} dpi in steps of {DPI_STEP}",
            DPI_RANGE.start(),
            DPI_RANGE.end()
        ))
    }
}

fn parse_wheel_threshold(text: &str) -> Result<u8, String> {
    let threshold: u8 = text
        .parse()
        .map_err(|_| format!("{text:?} is not a number"))?;
    if WHEEL_THRESHOLD_RANGE.contains(&threshold) {
        Ok(threshold)
    } else {
        Err(format!(
            "{} to {}",
            WHEEL_THRESHOLD_RANGE.start(),
            WHEEL_THRESHOLD_RANGE.end()
        ))
    }
}

fn parse_waveform_arg(text: &str) -> Result<HapticWaveform, String> {
    parse_waveform(text).ok_or_else(|| {
        let names: Vec<String> = HapticWaveform::ALL
            .iter()
            .map(|w| format!("{w:?}"))
            .collect();
        format!("unknown waveform; one of {}", names.join(", "))
    })
}

impl From<MouseTestAction> for Action {
    fn from(action: MouseTestAction) -> Self {
        match action {
            MouseTestAction::Info => Self::Info,
            MouseTestAction::Haptic { waveform, level } => Self::Haptic {
                waveform,
                level: level.map(Into::into),
            },
            MouseTestAction::Dpi { value } => Self::Dpi(value),
            MouseTestAction::Wheel { mode, threshold } => Self::Wheel {
                mode: mode.into(),
                threshold,
            },
            MouseTestAction::Buttons { seconds } => Self::Buttons(Duration::from_secs(seconds)),
            MouseTestAction::Events { seconds } => Self::Events(Duration::from_secs(seconds)),
            MouseTestAction::Release => Self::Release,
        }
    }
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
        Some(
            Command::Open
            | Command::InstallExtension { .. }
            | Command::MouseTest { .. }
            | Command::RadialPreview { .. },
        )
        | None => LogOutput::Stdout,
    });
    match cli.command {
        Some(Command::Open) => run_open(cli.port).await,
        Some(Command::NativeHost { origin }) => exit_with(run_native_host(&origin).await),
        Some(Command::InstallExtension { browser }) => install_extension(browser),
        Some(Command::MouseTest { model, action }) => run_mouse_test(model, action.into()).await,
        Some(Command::RadialPreview { seconds }) => run_radial_preview(seconds).await,
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

/// Does one action on a Logitech mouse and says what happened (`mouse-test`).
///
/// Ctrl-C, SIGTERM and SIGHUP end a watch of the buttons early, after giving
/// them back: a dead process would leave them diverted.
async fn run_mouse_test(model: Option<MouseModelArg>, action: Action) -> Result<()> {
    if matches!(action, Action::Buttons(_) | Action::Release) {
        eprintln!(
            "Stop the HuBergamof service, Solaar and logiops first: they divert the same buttons."
        );
    }
    eprintln!("Looking for the mouse (move it to wake it)...");
    let stop = Arc::new(AtomicBool::new(false));
    let waiter = tokio::spawn({
        let stop = Arc::clone(&stop);
        async move {
            termination_requested().await;
            stop.store(true, Ordering::Relaxed);
        }
    });
    let outcome = tokio::task::spawn_blocking(move || {
        let mut link = hubergamof_mouse::hidraw::find_mouse(model.map(Into::into))?;
        diagnostics::run(&mut link, &action, &mut std::io::stdout().lock(), &stop)?;
        anyhow::Ok(())
    })
    .await;
    waiter.abort();
    outcome.context("the mouse task failed")?
}

/// Shows the radial menu for a few seconds (`radial-preview`).
async fn run_radial_preview(seconds: u64) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let actions = ButtonActions::uinput(watch::channel(false).1);
        // Only the names and the icons are read: nothing of the system is touched.
        let registry = builtin_registry_with(BuiltinServices::simulated());
        let looks = Looks::of_modules(
            &registry,
            &Renderer::new().context("cannot initialise the renderer")?,
        );
        radial::preview::run(
            Duration::from_secs(seconds),
            &mut std::io::stdout().lock(),
            || actions.nudge_pointer(),
            looks,
        )
        .context("cannot show the radial menu")
    })
    .await
    .context("the radial menu task failed")?
}

/// Returns when the person asks the program to stop.
async fn termination_requested() {
    let (Ok(mut terminate), Ok(mut hangup)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::hangup()),
    ) else {
        return std::future::pending().await;
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
        _ = hangup.recv() => {}
    }
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
    // A closed terminal must not kill the process before the mice are given
    // back.
    let mut hangup = signal(SignalKind::hangup()).context("cannot listen for SIGHUP")?;
    let (shutdown, stopping) = watch::channel(false);

    // The mice, started after the devices, play their haptic events.
    let haptics = Arc::new(HapticRelay::new());
    let mut services = real_services(&cli, Arc::clone(&haptics) as Arc<dyn HapticOutput>)?;
    let catalog = catalog_of(&services).await?;
    let store = services.store.clone();
    let renderer = Arc::clone(&services.renderer);
    let discovery = DeviceDiscovery::new();
    // The menu runs actions on the engines, known before they start.
    let engines = EngineRegistry::default();
    let (host, mut host_task) = start_module_host(&services);
    // The devices place the modules of the application instead of making their own.
    services.application = Some(host.clone());
    let (mouse_hub, mice) =
        start_mice_or_stop_host((&cli, &services, &engines, &haptics), &host, &mut host_task)
            .await?;
    let screen_lock = Arc::clone(&services.screen_lock);
    let supervisor = Supervisor::start_in(&discovery, services, engines);
    let updates = start_update_service(&cli, stopping.clone());
    let context = ServerContext {
        discovery: discovery.clone(),
        engines: supervisor.engines(),
        catalog,
        simulated_input: simulated_input(cli.demo),
        store: store.clone(),
        renderer,
        file_picker: file_picker(cli.file_chooser_answer.clone()),
        updates: updates.clone(),
        mice: mouse_hub,
        application: Some(host.clone()),
        machine: start_machine(cli.demo, store.clone())?,
        git: start_git(cli.demo, store.clone())?,
        screen_lock,
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
            stop_mice(mice).await;
            stop_host(&host, &mut host_task).await;
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
            stop_mice(mice).await;
            stop_host(&host, &mut host_task).await;
            return Err(error);
        }
    };
    if !cli.demo {
        tokio::spawn(report_first_scan(discovery.clone()));
    }
    let tray = start_tray(
        &cli,
        url,
        &discovery,
        supervisor.engines(),
        updates,
        &shutdown,
    )
    .await;

    let outcome = wait_for_stop(&mut terminate, &mut hangup, stopping, &mut server_task).await;
    tracing::info!("shutting down");
    shutdown.send_replace(true);
    stop_server(server_task).await;
    supervisor.shutdown().await;
    stop_mice(mice).await;
    // After the menu, which hands it actions: the pomodoro pauses where it is.
    stop_host(&host, &mut host_task).await;
    shutdown_devices(&discovery, source).await;
    if let Some(tray) = tray {
        tray.stop().await;
    }
    tracing::info!("bye");
    outcome
}

/// Waits for whatever ends the manager: a signal, the tray's quit, or the web
/// server stopping on its own.
async fn wait_for_stop(
    terminate: &mut tokio::signal::unix::Signal,
    hangup: &mut tokio::signal::unix::Signal,
    stopping: watch::Receiver<bool>,
    server_task: &mut JoinHandle<std::io::Result<()>>,
) -> Result<()> {
    tokio::select! {
        signal = tokio::signal::ctrl_c() => signal.context("cannot listen for Ctrl-C"),
        _ = terminate.recv() => Ok(()),
        _ = hangup.recv() => Ok(()),
        () = wait_for_quit(stopping) => Ok(()),
        result = server_task => server_outcome(result),
    }
}

/// The modules that live in the application, apart from any device.
fn start_module_host(services: &EngineServices) -> (ModuleHost, tokio::task::JoinHandle<()>) {
    ModuleHost::start(HostServices {
        registry: Arc::clone(&services.registry),
        store: services.store.clone(),
        haptics: Arc::clone(&services.haptics),
        screen_lock: Arc::clone(&services.screen_lock),
    })
}

/// The mice, or, when they cannot start, the end of the module host (so that its modules are told
/// that the manager stops) and the error.
async fn start_mice_or_stop_host(
    (cli, services, engines, haptics): (&Cli, &EngineServices, &EngineRegistry, &HapticRelay),
    host: &ModuleHost,
    host_task: &mut tokio::task::JoinHandle<()>,
) -> Result<(MouseHub, Mice)> {
    match start_mouse_service(cli, services, engines, haptics, host) {
        Ok(started) => Ok(started),
        Err(error) => {
            stop_host(host, host_task).await;
            Err(error)
        }
    }
}

/// Stops the module host, then waits for its task: it is aborted past
/// [`HOST_TASK_TIMEOUT`], so that a stuck module cannot hold the manager back.
async fn stop_host(host: &ModuleHost, task: &mut tokio::task::JoinHandle<()>) {
    host.shutdown().await;
    match tokio::time::timeout(HOST_TASK_TIMEOUT, &mut *task).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(%error, "module host failed"),
        Err(_) => {
            tracing::warn!("module host did not end in time");
            task.abort();
        }
    }
}

/// What the modules of the application say on their petals in the menu (a status under the
/// name, a badge on the icon), read from what the application shows.
fn petal_notes(host: &ModuleHost) -> PetalNotes {
    let views = host.views();
    PetalNotes::new(move |kind| {
        views
            .borrow()
            .get(kind)
            .map(|view| PetalNote {
                status: view.radial_status.clone(),
                badge: view.radial_badge,
            })
            .unwrap_or_default()
    })
}

/// What the modules offer around their petals in the menu: a device that
/// holds the module first, the application otherwise, like the actions.
fn petal_items(engines: &EngineRegistry, host: &ModuleHost) -> Items {
    let engines = engines.clone();
    let host = host.clone();
    Items::new(move |kind| first_radial_items(&engines, &host, kind))
}

/// The mouse hub, and the mice it lists: the real ones unless `--no-mice`, a simulated one with `--demo`.
fn start_mouse_service(
    cli: &Cli,
    services: &EngineServices,
    engines: &EngineRegistry,
    haptics: &HapticRelay,
    host: &ModuleHost,
) -> Result<(MouseHub, Mice)> {
    let hub = MouseHub::new(services.store.clone()).context("cannot load the radial menu")?;
    let mice = if serves_mice(cli) {
        Mice::Served(start_served_mice(&hub, services, engines, haptics, host))
    } else if cli.demo {
        Mice::Simulated {
            _listed: register_simulated_mice(&hub, &services.store)?,
        }
    } else {
        Mice::None
    };
    Ok((hub, mice))
}

/// Serves the mice found on the hardware, and the radial menu their button opens.
fn start_served_mice(
    hub: &MouseHub,
    services: &EngineServices,
    engines: &EngineRegistry,
    haptics: &HapticRelay,
    host: &ModuleHost,
) -> MiceService {
    // The events of the modules are played on the mice from now on.
    haptics.connect(Arc::new(hub.clone()));
    let locked = services.screen_lock.watch();
    let actions = ButtonActions::uinput(locked.clone());
    let menu = RadialService::start(
        hub.subscribe_radial_menu(),
        locked,
        &tokio::runtime::Handle::current(),
        {
            let actions = actions.clone();
            move || actions.nudge_pointer()
        },
        {
            let module_actions = ModuleActions::new(
                engines.clone(),
                host.clone(),
                &tokio::runtime::Handle::current(),
                Arc::clone(&services.registry),
            );
            move |entry, action| {
                // The action runs on its own: the menu does not wait for it.
                let _ = match action {
                    Some(action) => module_actions.run_action(&entry.module, action),
                    None => module_actions.run(entry),
                };
            }
        },
        Presentation {
            feel: Arc::new(hub.clone()),
            looks: Looks::of_modules(&services.registry, &services.renderer),
            notes: petal_notes(host),
            items: petal_items(engines, host),
        },
    );
    let actions = actions.with_radial(menu.handle());
    let supervisor = MouseSupervisor::start(
        HidrawSource,
        services.store.clone(),
        hub.clone(),
        actions,
        SCAN_INTERVAL,
    );
    MiceService { supervisor, menu }
}

/// The machine: the shell files of the user, or with `--demo` those of a
/// temporary home folder, so that the real startup files are never touched.
fn start_machine(demo: bool, store: ConfigStore) -> Result<MachineHub> {
    if demo {
        MachineHub::demo(store).context("cannot create the demo home folder")
    } else {
        Ok(MachineHub::new(store, MachineShell::of_current_user()))
    }
}

/// The icon in the notification area, unless `--no-tray`: it opens the interface at `url`
/// and quits the manager through `shutdown`.
async fn start_tray(
    cli: &Cli,
    url: String,
    discovery: &DeviceDiscovery,
    engines: EngineRegistry,
    updates: UpdateHandle,
    shutdown: &watch::Sender<bool>,
) -> Option<TrayIcon> {
    if cli.no_tray {
        return None;
    }
    TrayIcon::start(url, discovery, engines, updates, quit_on(shutdown.clone())).await
}

/// The Git configuration: the files of the user, or with `--demo` those of a
/// temporary home folder, so that the real ones are never touched.
fn start_git(demo: bool, store: ConfigStore) -> Result<GitConfigHub> {
    if demo {
        GitConfigHub::demo(store).context("cannot create the demo home folder")
    } else {
        Ok(GitConfigHub::new(store, GitFiles::of_current_user()))
    }
}

/// The mice `--demo` lists, with the serial of each.
const SIMULATED_MICE: [(MouseModel, &str, MouseBattery); 2] = [
    (
        MouseModel::MxMaster4,
        "demo-mx-master-4",
        MouseBattery {
            percent: 76,
            charging: false,
        },
    ),
    (
        MouseModel::MxAnywhere3S,
        "demo-mx-anywhere-3s",
        MouseBattery {
            percent: 18,
            charging: true,
        },
    ),
];

/// Lists mice that do not exist, for the interface to configure (one with a haptic panel, one without): the
/// configuration is saved as for a real one, nothing is ever sent to a device.
fn register_simulated_mice(hub: &MouseHub, store: &ConfigStore) -> Result<Vec<MouseRegistration>> {
    SIMULATED_MICE
        .into_iter()
        .map(|(model, serial, battery)| {
            let config = store
                .load_mouse(serial, model)
                .context("cannot load the simulated mouse configuration")?
                .config;
            let registration = hub.register(
                MouseInfo {
                    serial: serial.to_owned(),
                    model,
                    display_name: model.display_name().to_owned(),
                    capabilities: model.capabilities(),
                },
                config,
            );
            registration.status_reporter().report_battery(Some(battery));
            Ok(registration)
        })
        .collect()
}

/// `--demo` never touches the hardware, whatever else is asked.
fn serves_mice(cli: &Cli) -> bool {
    !cli.no_mice && !cli.demo
}

/// The mice the hub lists, and what keeps them so.
enum Mice {
    None,
    /// The mice found on the hardware.
    Served(MiceService),
    /// The simulated mice of `--demo`, listed while the registrations live.
    Simulated {
        /// Dropped with the manager, which takes the mice off the list.
        _listed: Vec<MouseRegistration>,
    },
}

/// The mice being served and the radial menu their button opens.
struct MiceService {
    supervisor: MouseSupervisor,
    menu: RadialService,
}

/// Gives the buttons of the mice back and stops serving them, then closes the
/// menu (a button held at that moment cancels it).
async fn stop_mice(mice: Mice) {
    if let Mice::Served(MiceService { supervisor, menu }) = mice {
        supervisor.shutdown().await;
        // Stopping the menu waits for its threads: not on the async workers.
        if let Err(error) = tokio::task::spawn_blocking(move || drop(menu)).await {
            tracing::warn!(%error, "the radial menu did not stop cleanly");
        }
    }
}

/// Release announced by `--demo`: far above any real one, so the interface
/// always shows an available update.
const DEMO_RELEASE: Version = Version::new(999, 0, 0);

/// Starts the update service: the periodic check for a newer release and, when
/// the installation is a system package, its installation. `--demo` never
/// touches the network nor the system: it announces [`DEMO_RELEASE`] at once
/// and simulates the installation, on request only.
fn start_update_service(cli: &Cli, stopping: watch::Receiver<bool>) -> UpdateHandle {
    if cli.no_update_check {
        tracing::info!("update check disabled by --no-update-check");
        return UpdateHandle::inactive();
    }
    let current = match Version::current() {
        Ok(current) => current,
        Err(error) => {
            tracing::warn!(%error, "update check disabled");
            return UpdateHandle::inactive();
        }
    };
    let parts = if cli.demo {
        demo_update_parts(current)
    } else {
        match system_update_parts(current, &cli.config_dir) {
            Ok(parts) => parts,
            Err(error) => {
                tracing::warn!(%error, "update check disabled");
                return UpdateHandle::inactive();
            }
        }
    };
    // The task ends with the shutdown signal; nothing to wait for.
    update::start(parts, stopping).0
}

fn demo_update_parts(current: Version) -> UpdateParts {
    UpdateParts {
        source: Arc::new(SimulatedReleases::announcing(DEMO_RELEASE)),
        installer: Arc::new(SimulatedInstaller::succeeding()),
        // On request only, so that the interface shows the release first.
        policy: Arc::new(MemoryPolicyStore::holding(UpdatePolicy {
            auto_install: false,
        })),
        current,
        first_delay: Duration::ZERO,
    }
}

fn system_update_parts(
    current: Version,
    config_dir: &std::path::Path,
) -> Result<UpdateParts, update::UpdateError> {
    Ok(UpdateParts {
        source: Arc::new(GithubReleases::new()?),
        installer: system_installer(),
        policy: Arc::new(FilePolicyStore::in_dir(config_dir)),
        current,
        first_delay: FIRST_CHECK_DELAY,
    })
}

/// Installs through the helper when this is a system package whose update
/// socket is there; otherwise (sources, container) the release is only
/// announced.
fn system_installer() -> Arc<dyn Installer> {
    use std::os::unix::fs::FileTypeExt as _;
    let socket_present =
        std::fs::metadata(wire::SOCKET_PATH).is_ok_and(|metadata| metadata.file_type().is_socket());
    let Some(format) = Installed.installed_format().filter(|_| socket_present) else {
        return Arc::new(UnsupportedInstaller);
    };
    match Downloader::new() {
        Ok(downloader) => Arc::new(PackageInstaller::new(
            format,
            downloader,
            Box::new(SocketChannel::system()),
        )),
        Err(error) => {
            tracing::warn!(%error, "updates cannot be installed");
            Arc::new(UnsupportedInstaller)
        }
    }
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

/// The services of the engines for this run (simulated ones with `--demo`).
fn real_services(cli: &Cli, haptics: Arc<dyn HapticOutput>) -> Result<EngineServices> {
    services(
        &cli.config_dir,
        builtin_services(cli.demo),
        system_probe(cli.demo),
        screen_lock(cli.demo),
        haptics,
    )
}

fn services(
    config_dir: &std::path::Path,
    builtins: BuiltinServices,
    system: Arc<dyn SystemProbe>,
    screen_lock: Arc<dyn ScreenLock>,
    haptics: Arc<dyn HapticOutput>,
) -> Result<EngineServices> {
    let registry = Arc::new(builtin_registry_with(builtins));
    Ok(EngineServices {
        application: None,
        store: ConfigStore::new(config_dir, Arc::clone(&registry)),
        renderer: Arc::new(Renderer::new().context("cannot initialise the renderer")?),
        registry,
        wall_clock: Arc::new(SystemClock),
        system,
        screen_lock,
        haptics,
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
    fn the_mice_are_served_unless_refused() {
        let serves = |args: &[&str]| {
            let mut all = vec!["hubergamof"];
            all.extend_from_slice(args);
            serves_mice(&Cli::try_parse_from(all).unwrap())
        };
        assert!(serves(&[]));
        assert!(serves(&["--mice"]), "the old option still works");
        assert!(!serves(&["--no-mice"]));
        assert!(
            !serves(&["--demo"]) && !serves(&["--demo", "--mice"]),
            "the demo never serves mice"
        );
        assert!(Cli::try_parse_from(["hubergamof", "--mice", "--no-mice"]).is_err());
    }

    #[test]
    fn mouse_test_actions_are_parsed() {
        let parse = |args: &[&str]| {
            let mut all = vec!["hubergamof", "mouse-test"];
            all.extend_from_slice(args);
            Cli::try_parse_from(all)
        };
        let action = |args: &[&str]| match parse(args).unwrap().command {
            Some(Command::MouseTest { action, .. }) => Action::from(action),
            other => panic!("not a mouse-test: {other:?}"),
        };
        assert_eq!(action(&["info"]), Action::Info);
        assert_eq!(action(&["dpi", "2400"]), Action::Dpi(2400));
        assert_eq!(action(&["release"]), Action::Release);
        assert_eq!(
            action(&["haptic", "sharp-state-change", "--level", "low"]),
            Action::Haptic {
                waveform: HapticWaveform::SharpStateChange,
                level: Some(HapticLevel::Low)
            }
        );
        assert_eq!(
            action(&["wheel", "auto", "--threshold", "30"]),
            Action::Wheel {
                mode: WheelMode::Auto,
                threshold: 30
            }
        );
        assert_eq!(
            action(&["wheel", "free"]),
            Action::Wheel {
                mode: WheelMode::Free,
                threshold: 20
            }
        );
        assert_eq!(
            action(&["buttons"]),
            Action::Buttons(Duration::from_secs(20))
        );
        assert_eq!(
            action(&["buttons", "--seconds", "5"]),
            Action::Buttons(Duration::from_secs(5))
        );
        assert_eq!(
            action(&["events"]),
            Action::Events(Duration::from_secs(120))
        );
        assert_eq!(
            action(&["events", "--seconds", "30"]),
            Action::Events(Duration::from_secs(30))
        );
    }

    #[test]
    fn mouse_test_refuses_what_it_does_not_know() {
        for args in [
            &["mouse-test"][..],
            &["mouse-test", "haptic", "fire"],
            &["mouse-test", "dpi"],
            &["mouse-test", "dpi", "fast"],
            &["mouse-test", "dpi", "225"],
            &["mouse-test", "dpi", "70000"],
            &["mouse-test", "wheel", "auto", "--threshold", "0"],
            &["mouse-test", "wheel", "auto", "--threshold", "300"],
            &["mouse-test", "buttons", "--seconds", "0"],
            &["mouse-test", "buttons", "--seconds", "301"],
            &["mouse-test", "buttons", "--seconds", "-1"],
            &["mouse-test", "haptic", "knock", "--level", "loud"],
            &["mouse-test", "wheel", "turbo"],
            &["mouse-test", "--model", "mx-keys", "info"],
        ] {
            let mut all = vec!["hubergamof"];
            all.extend_from_slice(args);
            assert!(Cli::try_parse_from(all).is_err(), "{args:?}");
        }
        let cli = Cli::try_parse_from(["hubergamof", "mouse-test", "--model", "master4", "info"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::MouseTest {
                model: Some(MouseModelArg::Master4),
                ..
            })
        ));
        assert_eq!(
            MouseModel::from(MouseModelArg::Anywhere3s),
            MouseModel::MxAnywhere3S
        );
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

    #[test]
    fn the_demo_lists_simulated_mice_that_can_be_configured() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(
            dir.path(),
            Arc::new(builtin_registry_with(builtin_services(true))),
        );
        let hub = MouseHub::new(store.clone()).unwrap();
        let listed = register_simulated_mice(&hub, &store).unwrap();
        let models: Vec<MouseModel> = hub.mice().iter().map(|mouse| mouse.model).collect();
        assert_eq!(models, [MouseModel::MxMaster4, MouseModel::MxAnywhere3S]);
        let (model, serial, _) = SIMULATED_MICE[0];
        let mut config = store.load_mouse(serial, model).unwrap().config;
        config.pointer.dpi = 800;
        let saved = hub.update_mouse(serial, config).unwrap();
        assert_eq!(saved.pointer.dpi, 800);
        let mut status = hub.subscribe_status(serial).unwrap();
        assert!(status.borrow_and_update().battery.is_some());
        drop(listed);
        assert_eq!(hub.mice().len(), 0);
        // The next run of the demo finds what the interface saved.
        let again = register_simulated_mice(&hub, &store).unwrap();
        let mut updates = hub.subscribe_config(serial).unwrap();
        assert_eq!(updates.borrow_and_update().pointer.dpi, 800);
        drop(again);
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
                Arc::new(hubergamof_manager::haptics::NoHaptics),
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
