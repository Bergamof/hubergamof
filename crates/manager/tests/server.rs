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
use bergadeck_manager::config_store::ConfigStore;
use bergadeck_manager::device::DeviceDiscovery;
use bergadeck_manager::device::virtual_deck::{DEMO_SERIAL, VirtualDevices, virtual_plus_info};
use bergadeck_manager::engine::{EngineServices, Supervisor};
use bergadeck_manager::render::Renderer;
use bergadeck_manager::server::{
    AccessPolicy, AssetSource, CONTENT_SECURITY_POLICY_BASE, DevOrigin, MAX_WS_MESSAGE_BYTES,
    MIN_LIVE_STATE_INTERVAL, MISSING_UI_MESSAGE, Server, ServerContext, SimulatedInput, WS_PATH,
    module_catalog, router,
};
use bergadeck_module_api::{
    Command, KeyContent, Module, ModuleCapabilities, ModuleContext, ModuleDescriptor, ModuleError,
    ModuleFactory,
};
use bergadeck_modules::{BuiltinServices, builtin_registry_with};
use bergadeck_protocol::{
    ClientMessage, DeviceConfig, ErrorCode, LiveState, ModuleInstance, ServerMessage,
};
use futures_util::{SinkExt as _, StreamExt as _};
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
const WAIT: Duration = Duration::from_secs(5);
const COUNTER_KIND: &str = "counter";

const INDEX_HTML: &str = "<!doctype html><html><head><title>Bergadeck</title></head>\
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
                dial: false,
                touch_zone: false,
            },
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
            store: ConfigStore::new(dir.path(), Arc::clone(&registry) as _),
            renderer: Arc::new(Renderer::new().unwrap()),
            registry: Arc::clone(&registry),
        };
        let catalog = module_catalog(&registry, &services.renderer);
        let discovery = DeviceDiscovery::new();
        let supervisor = Supervisor::start(&discovery, services);
        let decks = VirtualDevices::new(discovery.clone());
        let _deck = decks.attach(virtual_plus_info(DEMO_SERIAL));
        let context = ServerContext {
            discovery: discovery.clone(),
            engines: supervisor.engines(),
            catalog,
            simulated_input: SimulatedInput::Accepted,
        };
        Self {
            _dir: dir,
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

    /// Client past the `hello`, `devices` and `moduleCatalog` greeting.
    async fn connect(&self) -> Client {
        let own_origin = format!("http://127.0.0.1:{}", self.port);
        let mut client = self.connect_with(Some(&own_origin)).await.unwrap();
        for _ in 0..3 {
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
        let config = match self.receive().await {
            ServerMessage::Config { serial: of, config } if of == serial => config,
            other => panic!("expected the configuration, got {other:?}"),
        };
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
            assert!(kinds.contains(&"clock"), "{kinds:?}");
            assert!(kinds.contains(&COUNTER_KIND), "{kinds:?}");
            let clock = modules
                .iter()
                .find(|module| module.descriptor.kind.0 == "clock")
                .unwrap();
            assert!(
                clock
                    .icon
                    .as_deref()
                    .is_some_and(|icon| icon.starts_with("data:image/png;base64,")),
                "{clock:?}"
            );
        }
        other => panic!("expected the catalog, got {other:?}"),
    }
    client.ping().await;
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
    // Long enough for a live state of the new look to be due.
    tokio::time::sleep(MIN_LIVE_STATE_INTERVAL * 3).await;
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
    tokio::time::sleep(Duration::from_millis(200)).await;
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
