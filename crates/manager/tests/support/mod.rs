//! What the tests of the HTTP routes share: two virtual Stream Deck+ and a
//! Stream Deck MK.2 behind the routes of the server.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use hubergamof_manager::config_store::ConfigStore;
use hubergamof_manager::device::DeviceDiscovery;
use hubergamof_manager::device::virtual_deck::{VirtualDevices, virtual_plus_info};
use hubergamof_manager::engine::{EngineHandle, EngineServices, Supervisor};
use hubergamof_manager::file_chooser::{FilePicker, SimulatedFilePicker};
use hubergamof_manager::machine::MachineHub;
use hubergamof_manager::machine::git::GitConfigHub;
use hubergamof_manager::machine::ssh::SshConfigHub;
use hubergamof_manager::mouse_hub::MouseHub;
use hubergamof_manager::render::Renderer;
use hubergamof_manager::screen_lock::{NoScreenLock, ScreenLock};
use hubergamof_manager::server::{
    AccessPolicy, AssetSource, ServerContext, SimulatedInput, module_catalog, router,
};
use hubergamof_manager::strip::{FixedClock, SimulatedProbe};
use hubergamof_manager::update::UpdateHandle;
use hubergamof_modules::{BuiltinServices, builtin_registry_with};
use hubergamof_protocol::{
    ApiError, DeviceCapabilities, DeviceConfig, DeviceInfo, DeviceModel, ErrorCode, ImageSize,
};
use tempfile::TempDir;
use tokio::sync::watch;
use tower::ServiceExt as _;

pub const PORT: u16 = 7878;
pub const HOST: &str = "127.0.0.1:7878";
pub const ORIGIN: &str = "http://127.0.0.1:7878";
pub const FIRST: &str = "API0000000001";
pub const SECOND: &str = "API0000000002";
/// A Stream Deck MK.2: 5×3 keys, no dial.
pub const NO_DIALS: &str = "API0000000003";
pub const CHOSEN_FILE: &str = "/srv/shop/compose.yaml";
pub const WAIT: Duration = Duration::from_secs(5);

pub struct NoUi;

impl AssetSource for NoUi {
    fn get(&self, _path: &str) -> Option<std::borrow::Cow<'static, [u8]>> {
        None
    }
}

/// Two virtual Stream Deck+ and the routes of the server.
pub struct Api {
    pub _dir: TempDir,
    pub _decks: VirtualDevices,
    pub supervisor: Supervisor,
    pub context: ServerContext,
    pub router: axum::Router,
}

impl Api {
    pub async fn start() -> Self {
        Self::start_with(Arc::new(SimulatedFilePicker::choosing(Some(
            CHOSEN_FILE.into(),
        ))))
        .await
    }

    pub async fn start_with(file_picker: Arc<dyn FilePicker>) -> Self {
        Self::start_on(file_picker, Arc::new(NoScreenLock)).await
    }

    /// On a computer that `screen_lock` says locked or not.
    pub async fn start_on(
        file_picker: Arc<dyn FilePicker>,
        screen_lock: Arc<dyn ScreenLock>,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(builtin_registry_with(BuiltinServices::simulated()));
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
        let context = ServerContext {
            discovery: discovery.clone(),
            engines: supervisor.engines(),
            catalog,
            simulated_input: SimulatedInput::Refused,
            mice: MouseHub::new(store.clone()).unwrap(),
            headsets: None,
            machine: MachineHub::new(store.clone(), None),
            git: GitConfigHub::new(store.clone(), None),
            ssh: SshConfigHub::new(store.clone(), None),
            store,
            renderer,
            file_picker,
            updates: UpdateHandle::inactive(),
            application: None,
            screen_lock,
        };
        let decks = VirtualDevices::new(discovery);
        for serial in [FIRST, SECOND] {
            let _deck = decks.attach(virtual_plus_info(serial));
        }
        let _deck = decks.attach(mk2_info(NO_DIALS));
        let router = router(
            context.clone(),
            Arc::new(NoUi),
            AccessPolicy::new(PORT, &[]),
            watch::channel(false).1,
        );
        let api = Self {
            _dir: dir,
            _decks: decks,
            supervisor,
            context,
            router,
        };
        for serial in [FIRST, SECOND, NO_DIALS] {
            api.engine(serial).await;
        }
        api
    }

    pub async fn engine(&self, serial: &str) -> EngineHandle {
        tokio::time::timeout(WAIT, async {
            loop {
                if let Some(engine) = self.context.engines.get(serial) {
                    return engine;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("engine started")
    }

    pub async fn config(&self, serial: &str) -> DeviceConfig {
        self.engine(serial).await.config().borrow().clone()
    }

    pub async fn send(&self, request: Request<Body>) -> Response<Body> {
        self.router.clone().oneshot(request).await.unwrap()
    }

    pub async fn get(&self, path: &str) -> Response<Body> {
        self.send(
            Request::get(path)
                .header("host", HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    pub async fn post(
        &self,
        path: &str,
        content_type: &str,
        body: impl Into<Body>,
    ) -> Response<Body> {
        self.send(
            Request::post(path)
                .header("host", HOST)
                .header("origin", ORIGIN)
                .header("content-type", content_type)
                .body(body.into())
                .unwrap(),
        )
        .await
    }

    pub async fn upload(&self, serial: &str, content_type: &str, bytes: Vec<u8>) -> Response<Body> {
        self.post(
            &format!("/api/devices/{serial}/images"),
            content_type,
            bytes,
        )
        .await
    }

    pub async fn stop(self) {
        self.supervisor.shutdown().await;
    }
}

pub fn mk2_info(serial: &str) -> DeviceInfo {
    DeviceInfo {
        serial: serial.into(),
        model: DeviceModel::Mk2,
        display_name: "Stream Deck MK.2".into(),
        capabilities: DeviceCapabilities {
            key_columns: 5,
            key_rows: 3,
            key_image_size: ImageSize {
                width: 72,
                height: 72,
            },
            dial_count: 0,
            touch_strip: None,
        },
    }
}

pub async fn bytes(response: Response<Body>) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
}

pub async fn api_error(response: Response<Body>) -> (StatusCode, ErrorCode) {
    let status = response.status();
    let error: ApiError = serde_json::from_slice(&bytes(response).await).unwrap();
    (status, error.code)
}
