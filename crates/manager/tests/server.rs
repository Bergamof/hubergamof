//! Web server: access policy, UI files and the WebSocket protocol, driven
//! with virtual devices.
#![cfg(test)]

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use futures_util::{SinkExt as _, StreamExt as _};
use hubergamof_manager::config_store::ConfigStore;
use hubergamof_manager::device::DeviceDiscovery;
use hubergamof_manager::device::virtual_deck::{DEMO_SERIAL, VirtualDevices, virtual_plus_info};
use hubergamof_manager::engine::{EngineServices, Supervisor};
use hubergamof_manager::file_chooser::NoFilePicker;
use hubergamof_manager::machine::{MachineHub, MachineShell};
use hubergamof_manager::module_host::{HostServices, ModuleHost};
use hubergamof_manager::mouse_hub::MouseHub;
use hubergamof_manager::render::Renderer;
use hubergamof_manager::screen_lock::ScreenLock;
use hubergamof_manager::server::{
    AccessPolicy, AssetSource, CONTENT_SECURITY_POLICY_BASE, DevOrigin, MAX_WS_MESSAGE_BYTES,
    MISSING_UI_MESSAGE, Server, ServerContext, SimulatedInput, WS_PATH, module_catalog, router,
};
use hubergamof_manager::strip::{FixedClock, SimulatedProbe};
use hubergamof_manager::update::{
    self, MemoryPolicyStore, SimulatedInstaller, SimulatedReleases, UpdateHandle, UpdateParts,
    UpdatePolicy, Version,
};
use hubergamof_module_api::{
    Command, KeyContent, Module, ModuleCapabilities, ModuleContext, ModuleDescriptor, ModuleError,
    ModuleFactory,
};
use hubergamof_modules::{BuiltinServices, builtin_registry_with};
use hubergamof_protocol::{
    ClientMessage, DeviceConfig, ErrorCode, InstallSupport, LiveState, MachineConfig,
    ModuleInstance, MouseConfig, MouseInfo, MouseModel, MouseProblem, RadialMenuConfig,
    ServerMessage, ShellAlias, SystemAlias, UpdateSettings, UpdateStatus,
};
use serde_json::Value;
use tempfile::TempDir;
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tower::ServiceExt as _;

const PORT: u16 = 7878;
const HOST: &str = "127.0.0.1:7878";
const DEV_ORIGIN: &str = "http://localhost:5173";
/// Messages a client receives on connection: `hello`, `devices`,
/// `moduleCatalog`, `locked`, `mice`, `radialMenu` and `updateSettings`.
const GREETING_MESSAGES: usize = 7;
const WAIT: Duration = Duration::from_secs(5);
const COUNTER_KIND: &str = "counter";

const INDEX_HTML: &str = "<!doctype html><html><head><title>HuBergamof</title></head>\
     <body><script>start();</script></body></html>";
const FALLBACK_HTML: &str = "<!doctype html><html><body>shell</body></html>";
const IMMUTABLE_JS: &str = "_app/immutable/entry/start.abc123.js";

// ---------------------------------------------------------------- Fixtures

/// In-memory UI build.
struct TestUi(BTreeMap<&'static str, &'static str>);

impl TestUi {
    fn built() -> Arc<Self> {
        Arc::new(Self(BTreeMap::from([
            ("index.html", INDEX_HTML),
            ("200.html", FALLBACK_HTML),
            (IMMUTABLE_JS, "export const start = () => {};"),
            ("robots.txt", "User-agent: *"),
        ])))
    }

    fn missing() -> Arc<Self> {
        Arc::new(Self(BTreeMap::new()))
    }
}

impl AssetSource for TestUi {
    fn get(&self, path: &str) -> Option<Cow<'static, [u8]>> {
        self.0
            .get(path)
            .map(|content| Cow::Borrowed(content.as_bytes()))
    }
}

/// Key showing how many times it was pressed.
struct Counter {
    presses: u32,
}

#[async_trait]
impl Module for Counter {
    fn key(&self) -> KeyContent {
        KeyContent::with_label(format!("count {}", self.presses))
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        self.presses += 1;
        Vec::new()
    }

    fn update_settings(&mut self, _settings: &Value) -> Result<(), ModuleError> {
        Ok(())
    }
}

struct CounterFactory;

impl ModuleFactory for CounterFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor {
            kind: COUNTER_KIND.into(),
            name: "Counter".into(),
            description: String::new(),
            capabilities: ModuleCapabilities {
                key: true,
                ..Default::default()
            },
            dial_functions: Vec::new(),
            settings: Vec::new(),
        }
    }

    fn create(
        &self,
        _ctx: ModuleContext,
        _settings: &Value,
    ) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(Counter { presses: 0 }))
    }
}

/// Engines for virtual devices, the demo device attached.
struct Devices {
    _dir: TempDir,
    /// The home folder whose shell files the machine reads and writes.
    home: TempDir,
    discovery: DeviceDiscovery,
    supervisor: Supervisor,
    decks: VirtualDevices,
    context: ServerContext,
}

impl Devices {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = builtin_registry_with(BuiltinServices::simulated());
        registry.register(Arc::new(CounterFactory));
        let registry = Arc::new(registry);
        let services = EngineServices {
            store: ConfigStore::new(dir.path(), Arc::clone(&registry)),
            renderer: Arc::new(Renderer::new().unwrap()),
            registry: Arc::clone(&registry),
            wall_clock: Arc::new(FixedClock::default()),
            system: Arc::new(SimulatedProbe::default()),
            screen_lock: Arc::new(hubergamof_manager::screen_lock::NoScreenLock),
            haptics: Arc::new(hubergamof_manager::haptics::NoHaptics),
            application: None,
        };
        let catalog = module_catalog(&registry, &services.renderer);
        let store = services.store.clone();
        let renderer = Arc::clone(&services.renderer);
        let discovery = DeviceDiscovery::new();
        let supervisor = Supervisor::start(&discovery, services);
        let decks = VirtualDevices::new(discovery.clone());
        let _deck = decks.attach(virtual_plus_info(DEMO_SERIAL));
        let home = tempfile::tempdir().unwrap();
        let machine = MachineHub::new(
            store.clone(),
            Some(MachineShell::new(home.path().to_owned())),
        );
        let context = ServerContext {
            discovery: discovery.clone(),
            engines: supervisor.engines(),
            catalog,
            simulated_input: SimulatedInput::Accepted,
            store: store.clone(),
            renderer,
            file_picker: Arc::new(NoFilePicker),
            updates: UpdateHandle::inactive(),
            mice: MouseHub::new(store.clone()).unwrap(),
            application: None,
            machine,
            screen_lock: Arc::new(hubergamof_manager::screen_lock::NoScreenLock),
        };
        Self {
            _dir: dir,
            home,
            discovery,
            supervisor,
            decks,
            context,
        }
    }
}

/// A server on an ephemeral port.
struct Running {
    devices: Devices,
    port: u16,
    shutdown: watch::Sender<bool>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Running {
    /// A demo server: simulated input accepted.
    async fn start() -> Self {
        Self::accepting(SimulatedInput::Accepted).await
    }

    async fn accepting(simulated_input: SimulatedInput) -> Self {
        let mut devices = Devices::start();
        devices.context.simulated_input = simulated_input;
        Self::with_devices(devices).await
    }

    async fn with_devices(devices: Devices) -> Self {
        let (shutdown, stopping) = watch::channel(false);
        let dev_origins = [DEV_ORIGIN.parse::<DevOrigin>().unwrap()];
        let server = Server::bind(
            0,
            &dev_origins,
            devices.context.clone(),
            TestUi::built(),
            stopping,
        )
        .await
        .unwrap();
        let port = server.address().port();
        assert!(server.address().ip().is_loopback());
        assert_eq!(server.url(), format!("http://127.0.0.1:{port}/"));
        let task = tokio::spawn(server.run());
        Self {
            devices,
            port,
            shutdown,
            task,
        }
    }

    async fn connect_with(&self, origin: Option<&str>) -> Result<Client, tungstenite::Error> {
        let mut request = format!("ws://127.0.0.1:{}{WS_PATH}", self.port)
            .into_client_request()
            .unwrap();
        if let Some(origin) = origin {
            request
                .headers_mut()
                .insert("origin", origin.parse().unwrap());
        }
        let (stream, _) = tokio_tungstenite::connect_async(request).await?;
        Ok(Client(stream))
    }

    /// Client past the `hello`, `devices`, `moduleCatalog`, `mice`, `radialMenu` and
    /// `updateSettings` greeting.
    async fn connect(&self) -> Client {
        let own_origin = format!("http://127.0.0.1:{}", self.port);
        let mut client = self.connect_with(Some(&own_origin)).await.unwrap();
        for _ in 0..GREETING_MESSAGES {
            client.receive().await;
        }
        client
    }

    async fn stop(self) {
        self.shutdown.send_replace(true);
        tokio::time::timeout(WAIT, self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        self.devices.supervisor.shutdown().await;
    }
}

struct Client(WebSocketStream<MaybeTlsStream<TcpStream>>);

impl Client {
    async fn send(&mut self, message: &ClientMessage) {
        self.send_text(&serde_json::to_string(message).unwrap())
            .await;
    }

    async fn send_text(&mut self, text: &str) {
        self.0.send(Message::text(text)).await.unwrap();
    }

    async fn receive(&mut self) -> ServerMessage {
        loop {
            let message = tokio::time::timeout(WAIT, self.0.next())
                .await
                .expect("no message from the server")
                .unwrap()
                .unwrap();
            if let Message::Text(text) = message {
                return serde_json::from_str(text.as_str()).unwrap();
            }
        }
    }

    /// Skips messages until one matches.
    async fn receive_until<T>(&mut self, mut pick: impl FnMut(ServerMessage) -> Option<T>) -> T {
        loop {
            if let Some(found) = pick(self.receive().await) {
                return found;
            }
        }
    }

    async fn live_state_where(&mut self, condition: impl Fn(&LiveState) -> bool) -> LiveState {
        self.receive_until(|message| match message {
            ServerMessage::LiveState { state } if condition(&state) => Some(state),
            _ => None,
        })
        .await
    }

    /// The next error, skipping the live states published meanwhile (the
    /// modules update their keys in the background).
    async fn error(&mut self) -> ErrorCode {
        self.receive_until(|message| match message {
            ServerMessage::Error { code, .. } => Some(code),
            ServerMessage::LiveState { .. } => None,
            other => panic!("expected an error, got {other:?}"),
        })
        .await
    }

    /// Selects `serial` and returns the `config` then `liveState` that follow.
    async fn select(&mut self, serial: &str) -> (DeviceConfig, LiveState) {
        self.send(&ClientMessage::SelectDevice {
            serial: serial.into(),
            live_state: true,
        })
        .await;
        // Live states of the device selected before may still be on their
        // way (sent before the selection was handled): skipped.
        let config = self
            .receive_until(|message| match message {
                ServerMessage::Config { serial: of, config } if of == serial => Some(config),
                ServerMessage::LiveState { state } if state.serial != serial => None,
                other => panic!("expected the configuration, got {other:?}"),
            })
            .await;
        let live = match self.receive().await {
            ServerMessage::LiveState { state } if state.serial == serial => state,
            other => panic!("expected the live state, got {other:?}"),
        };
        (config, live)
    }

    /// Proves that the connection is alive and no other message is queued.
    async fn ping(&mut self) {
        self.send(&ClientMessage::Ping).await;
        assert_eq!(self.receive().await, ServerMessage::Pong);
    }
}

fn http_router(devices: &Devices, assets: Arc<dyn AssetSource>) -> axum::Router {
    let policy = AccessPolicy::new(PORT, &[DEV_ORIGIN.parse().unwrap()]);
    router(
        devices.context.clone(),
        assets,
        policy,
        watch::channel(false).1,
    )
}

async fn get(router: &axum::Router, path: &str) -> Response<Body> {
    request(router, Request::get(path).header("host", HOST)).await
}

async fn request(router: &axum::Router, builder: axum::http::request::Builder) -> Response<Body> {
    router
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn body_text(response: Response<Body>) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn header<'a>(response: &'a Response<Body>, name: &str) -> &'a str {
    response
        .headers()
        .get(name)
        .unwrap_or_else(|| panic!("missing {name}"))
        .to_str()
        .unwrap()
}

fn counter_on_first_key(mut config: DeviceConfig) -> DeviceConfig {
    config.modules.push(ModuleInstance {
        id: "counter-1".into(),
        kind: COUNTER_KIND.into(),
        label: None,
        image: None,
        settings: Value::Object(serde_json::Map::new()),
    });
    config.home[0] = "counter-1".into();
    config
}

// ---------------------------------------------------------------- HTTP

#[tokio::test]
async fn home_page_is_served_with_security_headers() {
    let devices = Devices::start();
    let router = http_router(&devices, TestUi::built());

    let response = get(&router, "/").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        "text/html; charset=utf-8"
    );
    assert_eq!(header(&response, "cache-control"), "no-cache");
    assert_eq!(header(&response, "x-content-type-options"), "nosniff");
    assert_eq!(header(&response, "referrer-policy"), "no-referrer");
    let policy = header(&response, "content-security-policy");
    assert!(policy.starts_with(CONTENT_SECURITY_POLICY_BASE), "{policy}");
    assert!(policy.contains("frame-ancestors 'none'"), "{policy}");
    // echo -n 'start();' | openssl dgst -sha256 -binary | base64
    assert!(
        policy.ends_with("script-src 'self' 'sha256-E6jQvrT2ZivWG9GJ4AAEZf4MhI7l3QkLroW4hgvUT4c='"),
        "{policy}"
    );
    assert_eq!(body_text(response).await, INDEX_HTML);
    devices.supervisor.shutdown().await;
}

#[tokio::test]
async fn immutable_assets_are_cached_for_long() {
    let devices = Devices::start();
    let router = http_router(&devices, TestUi::built());

    let response = get(&router, &format!("/{IMMUTABLE_JS}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "cache-control"),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        header(&response, "content-type"),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(header(&response, "x-content-type-options"), "nosniff");

    let response = get(&router, "/robots.txt").await;
    assert_eq!(header(&response, "cache-control"), "no-cache");
    assert_eq!(
        header(&response, "content-type"),
        "text/plain; charset=utf-8"
    );
    devices.supervisor.shutdown().await;
}

#[tokio::test]
async fn client_routes_get_the_spa_shell_and_missing_files_404() {
    let devices = Devices::start();
    let router = http_router(&devices, TestUi::built());

    let response = get(&router, "/devices/DEMO0000001").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "cache-control"), "no-cache");
    assert!(header(&response, "content-security-policy").contains("script-src 'self'"));
    assert_eq!(body_text(response).await, FALLBACK_HTML);

    for path in ["/missing.png", "/_app/immutable/gone.js", "/_app/version"] {
        let response = get(&router, path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(header(&response, "x-content-type-options"), "nosniff");
    }
    let response = get(&router, "/../Cargo.toml").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    devices.supervisor.shutdown().await;
}

#[tokio::test]
async fn head_requests_get_headers_only() {
    let devices = Devices::start();
    let router = http_router(&devices, TestUi::built());

    let response = request(&router, Request::head("/").header("host", HOST)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        "text/html; charset=utf-8"
    );
    assert_eq!(body_text(response).await, "");
    devices.supervisor.shutdown().await;
}

#[tokio::test]
async fn missing_ui_build_is_explained() {
    let devices = Devices::start();
    let router = http_router(&devices, TestUi::missing());

    for path in ["/", "/devices/x"] {
        let response = get(&router, path).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(header(&response, "content-type").starts_with("text/plain"));
        let text = body_text(response).await;
        assert_eq!(text, MISSING_UI_MESSAGE);
        assert!(text.contains("make ui-build"));
    }
    devices.supervisor.shutdown().await;
}

#[tokio::test]
async fn foreign_hosts_are_refused() {
    let devices = Devices::start();
    let router = http_router(&devices, TestUi::built());

    for host in ["evil.example:7878", "127.0.0.1:9999", "evil.example"] {
        let response = request(&router, Request::get("/").header("host", host)).await;
        assert_eq!(response.status(), StatusCode::MISDIRECTED_REQUEST, "{host}");
        assert_eq!(header(&response, "x-content-type-options"), "nosniff");
    }
    let response = request(&router, Request::get("/")).await;
    assert_eq!(response.status(), StatusCode::MISDIRECTED_REQUEST);

    for host in ["localhost:7878", "localhost:5173"] {
        let response = request(&router, Request::get("/").header("host", host)).await;
        assert_eq!(response.status(), StatusCode::OK, "{host}");
    }
    devices.supervisor.shutdown().await;
}

#[tokio::test]
async fn state_changing_requests_need_an_allowed_origin() {
    let devices = Devices::start();
    let router = http_router(&devices, TestUi::built());

    let response = request(&router, Request::post("/").header("host", HOST)).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = request(
        &router,
        Request::post("/")
            .header("host", HOST)
            .header("origin", "http://evil.example"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    // Allowed origin: no such route.
    let response = request(
        &router,
        Request::post("/")
            .header("host", HOST)
            .header("origin", "http://127.0.0.1:7878"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    devices.supervisor.shutdown().await;
}

// ---------------------------------------------------------------- WebSocket

#[tokio::test]
async fn websocket_needs_an_allowed_origin() {
    let server = Running::start().await;
    for origin in [None, Some("http://evil.example"), Some("null")] {
        match server.connect_with(origin).await {
            Err(tungstenite::Error::Http(response)) => {
                assert_eq!(response.status(), StatusCode::FORBIDDEN, "{origin:?}");
            }
            Err(other) => panic!("unexpected error {other}"),
            Ok(_) => panic!("{origin:?} was accepted"),
        }
    }
    let mut dev_client = server.connect_with(Some(DEV_ORIGIN)).await.unwrap();
    assert!(matches!(
        dev_client.receive().await,
        ServerMessage::Hello { .. }
    ));
    server.stop().await;
}

#[tokio::test]
async fn clients_are_greeted_with_devices_and_catalog() {
    let server = Running::start().await;
    let own_origin = format!("http://localhost:{}", server.port);
    let mut client = server.connect_with(Some(&own_origin)).await.unwrap();

    assert_eq!(
        client.receive().await,
        ServerMessage::Hello {
            version: env!("CARGO_PKG_VERSION").into()
        }
    );
    match client.receive().await {
        ServerMessage::Devices { devices } => {
            let serials: Vec<_> = devices
                .iter()
                .map(|device| device.serial.as_str())
                .collect();
            assert_eq!(serials, [DEMO_SERIAL]);
        }
        other => panic!("expected devices, got {other:?}"),
    }
    match client.receive().await {
        ServerMessage::ModuleCatalog { modules } => {
            let kinds: Vec<_> = modules
                .iter()
                .map(|module| module.descriptor.kind.0.as_str())
                .collect();
            assert!(kinds.contains(&"audio"), "{kinds:?}");
            assert!(kinds.contains(&COUNTER_KIND), "{kinds:?}");
            let audio = modules
                .iter()
                .find(|module| module.descriptor.kind.0 == "audio")
                .unwrap();
            assert!(
                audio
                    .icon
                    .as_deref()
                    .is_some_and(|icon| icon.starts_with("data:image/png;base64,")),
                "{audio:?}"
            );
            let functions: Vec<_> = audio
                .descriptor
                .dial_functions
                .iter()
                .map(|function| function.id.0.as_str())
                .collect();
            assert_eq!(functions, ["volume"]);
        }
        other => panic!("expected the catalog, got {other:?}"),
    }
    assert_eq!(
        client.receive().await,
        ServerMessage::Locked { locked: false }
    );
    assert_eq!(client.receive().await, ServerMessage::Mice { mice: vec![] });
    assert!(matches!(
        client.receive().await,
        ServerMessage::RadialMenu { config } if config.revision == 0
    ));
    // Nothing can be installed through the inactive handle of the test server.
    assert_eq!(
        client.receive().await,
        ServerMessage::UpdateSettings {
            settings: UpdateSettings {
                auto_install: true,
                install_support: InstallSupport::Unpackaged,
            }
        }
    );
    client.ping().await;
    server.stop().await;
}

#[tokio::test]
async fn the_radial_menu_is_followed_by_every_client_and_unknown_mice_are_refused() {
    let server = Running::start().await;
    let mut author = server.connect().await;
    let mut observer = server.connect().await;

    author
        .send(&ClientMessage::SelectMouse {
            serial: "nobody".into(),
        })
        .await;
    assert!(matches!(
        author.receive().await,
        ServerMessage::Error {
            code: ErrorCode::UnknownDevice,
            ..
        }
    ));

    let mut menu = RadialMenuConfig::default_menu();
    menu.animate = !menu.animate;
    author
        .send(&ClientMessage::UpdateRadialMenu {
            config: menu.clone(),
        })
        .await;
    for client in [&mut author, &mut observer] {
        assert!(matches!(
            client.receive().await,
            ServerMessage::RadialMenu { config } if config.revision == 1 && config.animate == menu.animate
        ));
    }
    // The same revision again is now outdated.
    author
        .send(&ClientMessage::UpdateRadialMenu { config: menu })
        .await;
    assert!(matches!(
        author.receive().await,
        ServerMessage::Error {
            code: ErrorCode::StaleRevision,
            ..
        }
    ));
    server.stop().await;
}

/// A mouse connected to the test server, as a mouse thread lists it.
fn connect_mouse(
    server: &Running,
    serial: &str,
) -> hubergamof_manager::mouse_hub::MouseRegistration {
    let model = MouseModel::MxMaster4;
    server.devices.context.mice.register(
        MouseInfo {
            serial: serial.to_owned(),
            model,
            display_name: model.display_name().to_owned(),
            capabilities: model.capabilities(),
        },
        MouseConfig::default_for(serial, model),
    )
}

#[tokio::test]
async fn a_client_connecting_while_a_mouse_is_there_is_told_about_it() {
    let server = Running::start().await;
    let _mouse = connect_mouse(&server, "aaaa0001");
    let own_origin = format!("http://127.0.0.1:{}", server.port);
    let mut client = server.connect_with(Some(&own_origin)).await.unwrap();
    let mut greeting = Vec::new();
    for _ in 0..GREETING_MESSAGES {
        greeting.push(client.receive().await);
    }
    let [
        _,
        _,
        _,
        _,
        ServerMessage::Mice { mice },
        ServerMessage::RadialMenu { .. },
        _,
    ] = &greeting[..]
    else {
        panic!("unexpected greeting: {greeting:?}");
    };
    assert_eq!(mice.len(), 1);
    assert_eq!(mice[0].serial, "aaaa0001");
    server.stop().await;
}

#[tokio::test]
async fn a_mouse_is_selected_configured_by_one_client_and_followed_by_another() {
    let server = Running::start().await;
    let mouse = connect_mouse(&server, "aaaa0001");
    let mut author = server.connect().await;
    let mut observer = server.connect().await;
    let select = ClientMessage::SelectMouse {
        serial: "aaaa0001".into(),
    };
    author.send(&select).await;
    observer.send(&select).await;
    let ServerMessage::MouseConfig { config, .. } = author.receive().await else {
        panic!("no configuration for the author");
    };
    assert_eq!(config.revision, 0);
    // What the mouse refused of its configuration comes with it: nothing yet.
    let ServerMessage::MouseStatus {
        serial, problems, ..
    } = author.receive().await
    else {
        panic!("no status for the author");
    };
    assert_eq!((serial.as_str(), problems.len()), ("aaaa0001", 0));
    observer.receive().await;
    observer.receive().await;

    let mut update = config.clone();
    update.pointer.dpi = 2400;
    author
        .send(&ClientMessage::UpdateMouseConfig {
            serial: "aaaa0001".into(),
            config: update.clone(),
        })
        .await;
    for client in [&mut author, &mut observer] {
        let ServerMessage::MouseConfig { config, .. } = client.receive().await else {
            panic!("the update was not given");
        };
        assert_eq!((config.revision, config.pointer.dpi), (1, 2400));
    }

    // An outdated revision, a configuration of another model and a serial
    // that is not the one of the message are refused.
    author
        .send(&ClientMessage::UpdateMouseConfig {
            serial: "aaaa0001".into(),
            config: update,
        })
        .await;
    let other_model = MouseConfig::default_for("aaaa0001", MouseModel::MxAnywhere3S);
    author
        .send(&ClientMessage::UpdateMouseConfig {
            serial: "aaaa0001".into(),
            config: other_model,
        })
        .await;
    author
        .send(&ClientMessage::UpdateMouseConfig {
            serial: "aaaa0001".into(),
            config: MouseConfig::default_for("bbbb0001", MouseModel::MxMaster4),
        })
        .await;
    for expected in [
        ErrorCode::StaleRevision,
        ErrorCode::InvalidConfig,
        ErrorCode::InvalidConfig,
    ] {
        let ServerMessage::Error { code, .. } = author.receive().await else {
            panic!("expected {expected:?}");
        };
        assert_eq!(code, expected);
    }

    // What the mouse refuses is told to those who follow it.
    mouse.status_reporter().report(vec![MouseProblem {
        setting: "wheel".into(),
        message: "refused".into(),
    }]);
    for client in [&mut author, &mut observer] {
        let ServerMessage::MouseStatus { problems, .. } = client.receive().await else {
            panic!("the refusal was not told");
        };
        assert_eq!(problems[0].setting, "wheel");
    }

    // The mouse leaves: the clients are told, and it cannot be selected.
    drop(mouse);
    let ServerMessage::Mice { mice } = author.receive().await else {
        panic!("the list of mice was not sent");
    };
    assert_eq!(mice, []);
    author.send(&select).await;
    assert!(matches!(
        author.receive().await,
        ServerMessage::Error {
            code: ErrorCode::UnknownDevice,
            ..
        }
    ));
    // A malformed mouse request is refused and keeps the connection.
    author.send_text(r#"{"type":"selectMouse"}"#).await;
    assert!(matches!(
        author.receive().await,
        ServerMessage::Error {
            code: ErrorCode::InvalidMessage,
            ..
        }
    ));
    author.ping().await;
    server.stop().await;
}

#[tokio::test]
async fn selected_device_streams_config_and_live_state() {
    let server = Running::start().await;
    let mut client = server.connect().await;
    let mut observer = server.connect().await;

    let (config, live) = client.select(DEMO_SERIAL).await;
    assert_eq!(config.serial, DEMO_SERIAL);
    assert_eq!(live.page.id, "home");
    let (_, _) = observer.select(DEMO_SERIAL).await;

    // Accepted configuration: every subscriber gets it, then the new look.
    client
        .send(&ClientMessage::UpdateConfig {
            serial: DEMO_SERIAL.into(),
            config: counter_on_first_key(config.clone()),
        })
        .await;
    for subscriber in [&mut client, &mut observer] {
        // The new configuration and the new look, in either order.
        let mut saved = None;
        let mut redrawn = false;
        while saved.is_none() || !redrawn {
            match subscriber.receive().await {
                ServerMessage::Config { config, .. } => saved = Some(config),
                ServerMessage::LiveState { state } => {
                    redrawn |= state.page.keys[0].label.as_deref() == Some("count 0");
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        // Saved as sent, revision aside: the UI recognises its update by it.
        let sent = counter_on_first_key(config.clone());
        assert_eq!(
            saved.unwrap(),
            DeviceConfig {
                revision: sent.revision + 1,
                ..sent
            }
        );
    }

    // Simulated press, handled like a hardware one.
    client
        .send(&ClientMessage::PressKey {
            serial: DEMO_SERIAL.into(),
            index: 0,
        })
        .await;
    client
        .live_state_where(|state| state.page.keys[0].label.as_deref() == Some("count 1"))
        .await;

    // Outdated revision.
    client
        .send(&ClientMessage::UpdateConfig {
            serial: DEMO_SERIAL.into(),
            config,
        })
        .await;
    assert_eq!(
        client
            .receive_until(|message| match message {
                ServerMessage::Error { code, .. } => Some(code),
                _ => None,
            })
            .await,
        ErrorCode::StaleRevision
    );
    server.stop().await;
}

#[tokio::test]
async fn live_states_are_only_sent_to_clients_asking_for_them() {
    let server = Running::start().await;
    // Asks for them: proves a live state of the new look was sent.
    let mut watcher = server.connect().await;
    let (_, _) = watcher.select(DEMO_SERIAL).await;
    let mut client = server.connect().await;
    client
        .send(&ClientMessage::SelectDevice {
            serial: DEMO_SERIAL.into(),
            live_state: false,
        })
        .await;
    let config = match client.receive().await {
        ServerMessage::Config { config, .. } => config,
        other => panic!("expected the configuration, got {other:?}"),
    };

    client
        .send(&ClientMessage::UpdateConfig {
            serial: DEMO_SERIAL.into(),
            config: counter_on_first_key(config),
        })
        .await;
    assert!(matches!(
        client.receive().await,
        ServerMessage::Config { .. }
    ));
    client
        .send(&ClientMessage::PressKey {
            serial: DEMO_SERIAL.into(),
            index: 0,
        })
        .await;
    watcher
        .live_state_where(|state| state.page.keys[0].label.as_deref() == Some("count 1"))
        .await;
    // A later look reaching the watcher means the throttling interval has passed: the first one
    // would have been sent to both by now, and would come before the answer to the ping.
    client
        .send(&ClientMessage::PressKey {
            serial: DEMO_SERIAL.into(),
            index: 0,
        })
        .await;
    watcher
        .live_state_where(|state| state.page.keys[0].label.as_deref() == Some("count 2"))
        .await;
    client.ping().await;
    server.stop().await;
}

#[tokio::test]
async fn real_devices_refuse_simulated_input_but_not_configuration() {
    let server = Running::accepting(SimulatedInput::Refused).await;
    // Allowed Host and Origin: still refused, input comes from the device.
    let mut client = server.connect().await;
    let (_, _) = client.select(DEMO_SERIAL).await;
    let inputs = [
        ClientMessage::PressKey {
            serial: DEMO_SERIAL.into(),
            index: 0,
        },
        ClientMessage::RotateDial {
            serial: DEMO_SERIAL.into(),
            index: 0,
            ticks: 1,
        },
        ClientMessage::PressDial {
            serial: DEMO_SERIAL.into(),
            index: 0,
        },
    ];
    for input in inputs {
        client.send(&input).await;
        assert_eq!(client.error().await, ErrorCode::InputRefused, "{input:?}");
    }
    client.ping().await;
    server.stop().await;
}

#[tokio::test]
async fn invalid_requests_get_errors_without_closing() {
    let server = Running::start().await;
    let mut client = server.connect().await;
    let (_, _) = client.select(DEMO_SERIAL).await;

    client.send_text("{not json").await;
    assert_eq!(client.error().await, ErrorCode::InvalidMessage);
    client.send_text(r#"{"type":"explode"}"#).await;
    assert_eq!(client.error().await, ErrorCode::InvalidMessage);
    client.0.send(Message::binary(vec![1, 2, 3])).await.unwrap();
    assert_eq!(client.error().await, ErrorCode::InvalidMessage);

    for index in [300, 8] {
        client
            .send(&ClientMessage::PressKey {
                serial: DEMO_SERIAL.into(),
                index,
            })
            .await;
        assert_eq!(client.error().await, ErrorCode::InvalidInput, "key {index}");
    }
    client
        .send(&ClientMessage::RotateDial {
            serial: DEMO_SERIAL.into(),
            index: 4,
            ticks: 1,
        })
        .await;
    assert_eq!(client.error().await, ErrorCode::InvalidInput);
    client
        .send(&ClientMessage::PressDial {
            serial: "NOPE".into(),
            index: 0,
        })
        .await;
    assert_eq!(client.error().await, ErrorCode::UnknownDevice);
    client
        .send(&ClientMessage::SelectDevice {
            serial: "NOPE".into(),
            live_state: false,
        })
        .await;
    assert_eq!(client.error().await, ErrorCode::UnknownDevice);
    client.ping().await;
    server.stop().await;
}

#[tokio::test]
async fn oversized_messages_close_the_connection() {
    let server = Running::start().await;
    let mut client = server.connect().await;

    let huge = format!(
        r#"{{"type":"ping","padding":"{}"}}"#,
        "x".repeat(MAX_WS_MESSAGE_BYTES)
    );
    // The server may close before the whole message is written.
    let _ = client.0.send(Message::text(huge)).await;
    let closed = tokio::time::timeout(WAIT, async {
        loop {
            match client.0.next().await {
                None | Some(Err(_) | Ok(Message::Close(_))) => break,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(closed.is_ok());
    server.stop().await;
}

#[tokio::test]
async fn device_changes_are_announced_and_followed() {
    let server = Running::start().await;
    let mut client = server.connect().await;
    let (_, _) = client.select(DEMO_SERIAL).await;

    let second = "SECOND00001";
    let _deck = server.devices.decks.attach(virtual_plus_info(second));
    let devices = client
        .receive_until(|message| match message {
            ServerMessage::Devices { devices } => Some(devices),
            _ => None,
        })
        .await;
    let serials: Vec<_> = devices
        .iter()
        .map(|device| device.serial.as_str())
        .collect();
    assert_eq!(serials, [DEMO_SERIAL, second]);

    // Selected right away, possibly before its engine is ready.
    let (config, live) = client.select(second).await;
    assert_eq!(config.serial, second);
    assert_eq!(live.serial, second);

    // Unplugged: the list is updated and the subscription ends.
    assert!(server.devices.decks.detach(second).await);
    let devices = client
        .receive_until(|message| match message {
            ServerMessage::Devices { devices } => Some(devices),
            _ => None,
        })
        .await;
    assert_eq!(devices.len(), 1);
    assert_eq!(server.devices.discovery.devices().len(), 1);
    let engines = &server.devices.context.engines;
    let mut changes = engines.changes();
    tokio::time::timeout(WAIT, async {
        while engines.get(second).is_some() {
            changes.changed().await.unwrap();
        }
    })
    .await
    .expect("the engine of the unplugged device stops");
    client
        .send(&ClientMessage::SelectDevice {
            serial: second.into(),
            live_state: true,
        })
        .await;
    assert_eq!(client.error().await, ErrorCode::UnknownDevice);
    client.ping().await;
    server.stop().await;
}

#[tokio::test]
async fn shutdown_closes_websocket_sessions() {
    let server = Running::start().await;
    let mut client = server.connect().await;
    server.shutdown.send_replace(true);

    let close = tokio::time::timeout(WAIT, client.0.next()).await.unwrap();
    assert!(
        matches!(close, Some(Ok(Message::Close(Some(_))))),
        "{close:?}"
    );
    server.stop().await;
}

/// A running server whose update service announces 999.0.0 and simulates the
/// installation; the first check is far away, so that only requests run it.
async fn server_with_updates(auto_install: bool) -> (Running, watch::Sender<bool>) {
    let mut devices = Devices::start();
    let (stop, stopping) = watch::channel(false);
    let parts = UpdateParts {
        source: Arc::new(SimulatedReleases::announcing(Version::new(999, 0, 0))),
        installer: Arc::new(SimulatedInstaller::succeeding()),
        policy: Arc::new(MemoryPolicyStore::holding(UpdatePolicy { auto_install })),
        current: Version::new(1, 0, 0),
        first_delay: Duration::from_secs(3600),
    };
    let (updates, _task) = update::start(parts, stopping);
    devices.context.updates = updates;
    (Running::with_devices(devices).await, stop)
}

#[tokio::test]
async fn clients_follow_the_update_check_and_can_ask_for_one() {
    let (server, stop) = server_with_updates(false).await;
    let mut client = server.connect().await;

    // Nothing is known before the first check: no status at greeting.
    client.ping().await;
    client.send(&ClientMessage::CheckForUpdates).await;
    let available = client
        .receive_until(|message| match message {
            ServerMessage::UpdateStatus {
                status: status @ UpdateStatus::Available { .. },
            } => Some(status),
            ServerMessage::UpdateStatus { .. } => None,
            other => panic!("expected an update status, got {other:?}"),
        })
        .await;
    assert!(
        matches!(&available, UpdateStatus::Available { version, .. } if version == "999.0.0"),
        "{available:?}"
    );

    // A client connecting later is told right after the greeting.
    let mut late = server.connect().await;
    let status = late
        .receive_until(|message| match message {
            ServerMessage::UpdateStatus { status } => Some(status),
            _ => None,
        })
        .await;
    assert_eq!(status, available);
    stop.send_replace(true);
    server.stop().await;
}

#[tokio::test]
async fn clients_install_an_update_and_choose_the_policy() {
    let (server, stop) = server_with_updates(false).await;
    let mut client = server.connect().await;
    client.send(&ClientMessage::CheckForUpdates).await;
    client
        .receive_until(|message| match message {
            ServerMessage::UpdateStatus {
                status: UpdateStatus::Available { .. },
            } => Some(()),
            _ => None,
        })
        .await;

    client.send(&ClientMessage::InstallUpdate).await;
    let installed = client
        .receive_until(|message| match message {
            ServerMessage::UpdateStatus {
                status: status @ UpdateStatus::Installed { .. },
            } => Some(status),
            _ => None,
        })
        .await;
    assert_eq!(
        installed,
        UpdateStatus::Installed {
            version: "999.0.0".into()
        }
    );

    // The choice is applied and told to every client.
    let mut observer = server.connect().await;
    client
        .send(&ClientMessage::SetAutoInstall { enabled: true })
        .await;
    for watcher in [&mut client, &mut observer] {
        let settings = watcher
            .receive_until(|message| match message {
                ServerMessage::UpdateSettings { settings } if settings.auto_install => {
                    Some(settings)
                }
                _ => None,
            })
            .await;
        assert_eq!(settings.install_support, InstallSupport::Demo);
    }
    stop.send_replace(true);
    server.stop().await;
}

// ------------------------------------------------- Modules of the application

/// A computer that locks and unlocks when told to.
struct Lock(watch::Sender<bool>);

impl ScreenLock for Lock {
    fn watch(&self) -> watch::Receiver<bool> {
        self.0.subscribe()
    }
}

/// A server whose modules of the application run, on a computer that can lock.
async fn running_with_application() -> (Running, watch::Sender<bool>) {
    let mut devices = Devices::start();
    let registry = Arc::new(builtin_registry_with(BuiltinServices::simulated()));
    let locked = watch::Sender::new(false);
    let lock = Arc::new(Lock(locked.clone()));
    let (host, _task) = ModuleHost::start(HostServices {
        registry,
        store: devices.context.store.clone(),
        haptics: Arc::new(hubergamof_manager::haptics::NoHaptics),
        screen_lock: Arc::clone(&lock) as Arc<dyn ScreenLock>,
    });
    devices.context.application = Some(host);
    devices.context.screen_lock = lock;
    (Running::with_devices(devices).await, locked)
}

/// The pomodoro as the application last showed it.
async fn pomodoro_view(client: &mut Client) -> hubergamof_protocol::ApplicationModuleView {
    client
        .receive_until(|message| match message {
            ServerMessage::ApplicationModules { modules } => modules
                .into_iter()
                .find(|module| module.kind.0 == "pomodoro"),
            _ => None,
        })
        .await
}

/// The next `machine` message: its configuration and system aliases.
async fn next_machine(client: &mut Client) -> (MachineConfig, Vec<SystemAlias>) {
    client
        .receive_until(|message| match message {
            ServerMessage::Machine {
                config,
                system_aliases,
            } => Some((config, system_aliases)),
            _ => None,
        })
        .await
}

fn shell_alias(name: &str, command: &str) -> ShellAlias {
    ShellAlias {
        name: name.into(),
        command: command.into(),
        enabled: true,
    }
}

#[tokio::test]
async fn the_machine_is_read_on_request_and_updates_reach_every_client() {
    let server = Running::start().await;
    let home = server.devices.home.path().to_owned();
    std::fs::write(home.join(".bashrc"), "alias gs='git status'\n").unwrap();
    let mut author = server.connect().await;
    let mut other = server.connect().await;

    author.send(&ClientMessage::RefreshMachine).await;
    let (config, system) = next_machine(&mut author).await;
    assert_eq!(config, MachineConfig::empty());
    assert_eq!(system.len(), 1);
    assert_eq!(
        (system[0].name.as_str(), system[0].source.as_str()),
        ("gs", ".bashrc")
    );

    let mut wanted = config.clone();
    wanted.aliases.push(shell_alias("ll", "ls -la"));
    author
        .send(&ClientMessage::UpdateMachine {
            config: wanted.clone(),
        })
        .await;
    for client in [&mut author, &mut other] {
        let (saved, system) = next_machine(client).await;
        assert_eq!(saved.revision, 1);
        assert_eq!(saved.aliases, wanted.aliases);
        assert_eq!(system.len(), 1, "the managed alias is not a system one");
    }
    let managed = std::fs::read_to_string(home.join(".config/hubergamof/aliases.sh")).unwrap();
    assert!(managed.contains("alias ll='ls -la'"));
    assert!(
        std::fs::read_to_string(home.join(".bashrc"))
            .unwrap()
            .contains("hubergamof")
    );
    assert!(!home.join(".zshrc").exists());

    // Based on the old revision: refused, nothing told.
    author
        .send(&ClientMessage::UpdateMachine { config: wanted })
        .await;
    assert_eq!(expect_error(&mut author).await, ErrorCode::StaleRevision);

    // Invalid: refused.
    let mut invalid = next_revision(&mut author).await;
    invalid.aliases.push(shell_alias("bad name", "ls"));
    author
        .send(&ClientMessage::UpdateMachine { config: invalid })
        .await;
    assert_eq!(expect_error(&mut author).await, ErrorCode::InvalidConfig);
    server.stop().await;
}

/// The saved configuration, read back by a refresh.
async fn next_revision(client: &mut Client) -> MachineConfig {
    client.send(&ClientMessage::RefreshMachine).await;
    next_machine(client).await.0
}

async fn expect_error(client: &mut Client) -> ErrorCode {
    client
        .receive_until(|message| match message {
            ServerMessage::Error { code, .. } => Some(code),
            _ => None,
        })
        .await
}

#[tokio::test]
async fn clients_see_and_drive_the_modules_of_the_application() {
    let (server, _locked) = running_with_application().await;
    let mut client = server.connect().await;
    let view = pomodoro_view(&mut client).await;
    assert_eq!(view.status.as_deref(), Some("Focus 1/4\n25:00 · prêt"));

    // An action of the module starts the time, and every client is told.
    let mut other = server.connect().await;
    client
        .send(&ClientMessage::ModuleAction {
            module: "pomodoro".into(),
            action: "toggle".into(),
        })
        .await;
    for watcher in [&mut client, &mut other] {
        let running = watcher
            .receive_until(|message| match message {
                ServerMessage::ApplicationModules { modules } => modules
                    .into_iter()
                    .find(|module| module.kind.0 == "pomodoro")
                    .filter(|module| {
                        module
                            .status
                            .as_deref()
                            .is_some_and(|status| !status.contains("prêt"))
                    }),
                _ => None,
            })
            .await;
        assert!(running.status.unwrap().starts_with("Focus 1/4\n"));
    }

    // Settings the module takes become its settings, for everyone.
    client
        .send(&ClientMessage::UpdateModuleSettings {
            module: "pomodoro".into(),
            settings: serde_json::json!({ "workMinutes": 30 }),
        })
        .await;
    let saved = other
        .receive_until(|message| match message {
            ServerMessage::ApplicationModules { modules } => modules
                .into_iter()
                .find(|module| module.settings["workMinutes"] == 30),
            _ => None,
        })
        .await;
    assert_eq!(saved.kind.0, "pomodoro");
    server.stop().await;
}

#[tokio::test]
async fn what_the_modules_of_the_application_do_not_offer_is_refused() {
    let (server, _locked) = running_with_application().await;
    let mut client = server.connect().await;
    pomodoro_view(&mut client).await;

    client
        .send(&ClientMessage::ModuleAction {
            module: "pomodoro".into(),
            action: "format-the-disk".into(),
        })
        .await;
    assert_eq!(expect_error(&mut client).await, ErrorCode::InvalidInput);

    client
        .send(&ClientMessage::ModuleAction {
            module: "nothing".into(),
            action: "toggle".into(),
        })
        .await;
    assert_eq!(expect_error(&mut client).await, ErrorCode::Unavailable);

    client
        .send(&ClientMessage::UpdateModuleSettings {
            module: "pomodoro".into(),
            settings: serde_json::json!({ "workMinutes": "long" }),
        })
        .await;
    assert_eq!(expect_error(&mut client).await, ErrorCode::InvalidConfig);

    client
        .send(&ClientMessage::UpdateModuleSettings {
            module: "docker".into(),
            settings: serde_json::json!({}),
        })
        .await;
    assert_eq!(expect_error(&mut client).await, ErrorCode::Unavailable);
    server.stop().await;
}

#[tokio::test]
async fn nothing_is_done_from_the_interface_while_the_computer_is_locked() {
    let (server, locked) = running_with_application().await;
    let mut client = server.connect().await;
    pomodoro_view(&mut client).await;

    locked.send_replace(true);
    client
        .receive_until(|message| {
            matches!(message, ServerMessage::Locked { locked: true }).then_some(())
        })
        .await;

    let acting = [
        ClientMessage::ModuleAction {
            module: "pomodoro".into(),
            action: "toggle".into(),
        },
        ClientMessage::UpdateModuleSettings {
            module: "pomodoro".into(),
            settings: serde_json::json!({ "workMinutes": 30 }),
        },
        ClientMessage::PressKey {
            serial: DEMO_SERIAL.into(),
            index: 0,
        },
        ClientMessage::PressDial {
            serial: DEMO_SERIAL.into(),
            index: 1,
        },
        ClientMessage::UpdateRadialMenu {
            config: RadialMenuConfig::default_menu(),
        },
        ClientMessage::RefreshMachine,
        ClientMessage::UpdateMachine {
            config: MachineConfig::empty(),
        },
        ClientMessage::CheckForUpdates,
        // Following a device shows its configuration: refused too.
        ClientMessage::SelectDevice {
            serial: DEMO_SERIAL.into(),
            live_state: false,
        },
    ];
    for request in &acting {
        client.send(request).await;
        assert_eq!(
            expect_error(&mut client).await,
            ErrorCode::Locked,
            "{request:?}"
        );
    }

    // Following what is shown still works.
    client.send(&ClientMessage::Ping).await;
    client
        .receive_until(|message| matches!(message, ServerMessage::Pong).then_some(()))
        .await;

    // Back to the desk: what was not told meanwhile is told again, and the
    // same request goes through.
    locked.send_replace(false);
    client
        .receive_until(|message| {
            matches!(message, ServerMessage::Locked { locked: false }).then_some(())
        })
        .await;
    client
        .receive_until(|message| matches!(message, ServerMessage::Devices { .. }).then_some(()))
        .await;
    client
        .receive_until(|message| {
            matches!(message, ServerMessage::ModuleCatalog { .. }).then_some(())
        })
        .await;
    client
        .send(&ClientMessage::ModuleAction {
            module: "pomodoro".into(),
            action: "toggle".into(),
        })
        .await;
    client
        .receive_until(|message| match message {
            ServerMessage::ApplicationModules { modules } => modules.into_iter().find(|module| {
                module
                    .status
                    .as_deref()
                    .is_some_and(|s| !s.contains("prêt"))
            }),
            _ => None,
        })
        .await;
    server.stop().await;
}

#[tokio::test]
async fn a_client_connecting_while_the_computer_is_locked_hears_nothing_but_that() {
    let (server, locked) = running_with_application().await;
    locked.send_replace(true);
    // Not `connect`: it waits for the whole greeting, which a locked computer cuts short.
    let own_origin = format!("http://127.0.0.1:{}", server.port);
    let mut client = server.connect_with(Some(&own_origin)).await.unwrap();
    assert!(matches!(
        client.receive().await,
        ServerMessage::Hello { .. }
    ));
    assert_eq!(
        client.receive().await,
        ServerMessage::Locked { locked: true }
    );
    // Nothing else is sent: the next message is the answer to the keep-alive.
    client.send(&ClientMessage::Ping).await;
    assert_eq!(client.receive().await, ServerMessage::Pong);
    server.stop().await;
}
