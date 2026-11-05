//! Internal HTTP routes of the UI: key images and configuration archives.

#![cfg(test)]

use std::io::{Cursor, Write as _};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use bergadeck_manager::config_store::{ConfigStore, IMAGES_DIR, MAX_ARCHIVE_BYTES, read_archive};
use bergadeck_manager::device::DeviceDiscovery;
use bergadeck_manager::device::virtual_deck::{VirtualDevices, virtual_plus_info};
use bergadeck_manager::engine::{EngineHandle, EngineServices, Supervisor};
use bergadeck_manager::render::{MAX_SOURCE_BYTES, Renderer};
use bergadeck_manager::server::{
    AccessPolicy, AssetSource, ServerContext, SimulatedInput, module_catalog, router,
};
use bergadeck_modules::{BuiltinServices, builtin_registry_with};
use bergadeck_protocol::{
    ApiError, DeviceCapabilities, DeviceConfig, DeviceInfo, DeviceModel, ErrorCode, ImageSize,
    ImageUploaded,
};
use image::{ImageFormat, Rgba, RgbaImage};
use tempfile::TempDir;
use tokio::sync::watch;
use tower::ServiceExt as _;

const PORT: u16 = 7878;
const HOST: &str = "127.0.0.1:7878";
const ORIGIN: &str = "http://127.0.0.1:7878";
const FIRST: &str = "API0000000001";
const SECOND: &str = "API0000000002";
/// A Stream Deck MK.2: 5×3 keys, no dial.
const NO_DIALS: &str = "API0000000003";
const WAIT: Duration = Duration::from_secs(5);
const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2 2"><rect width="1" height="1" fill="red"/></svg>"#;

struct NoUi;

impl AssetSource for NoUi {
    fn get(&self, _path: &str) -> Option<std::borrow::Cow<'static, [u8]>> {
        None
    }
}

/// Two virtual Stream Deck+ and the routes of the server.
struct Api {
    _dir: TempDir,
    _decks: VirtualDevices,
    supervisor: Supervisor,
    context: ServerContext,
    router: axum::Router,
}

impl Api {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(builtin_registry_with(BuiltinServices::simulated()));
        let services = EngineServices {
            store: ConfigStore::new(dir.path(), Arc::clone(&registry) as _),
            renderer: Arc::new(Renderer::new().unwrap()),
            registry: Arc::clone(&registry),
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
            store,
            renderer,
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

    async fn engine(&self, serial: &str) -> EngineHandle {
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

    async fn config(&self, serial: &str) -> DeviceConfig {
        self.engine(serial).await.config().borrow().clone()
    }

    async fn send(&self, request: Request<Body>) -> Response<Body> {
        self.router.clone().oneshot(request).await.unwrap()
    }

    async fn get(&self, path: &str) -> Response<Body> {
        self.send(
            Request::get(path)
                .header("host", HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    async fn post(&self, path: &str, content_type: &str, body: impl Into<Body>) -> Response<Body> {
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

    async fn upload(&self, serial: &str, content_type: &str, bytes: Vec<u8>) -> Response<Body> {
        self.post(
            &format!("/api/devices/{serial}/images"),
            content_type,
            bytes,
        )
        .await
    }

    async fn stop(self) {
        self.supervisor.shutdown().await;
    }
}

fn mk2_info(serial: &str) -> DeviceInfo {
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

/// Image files of device `serial`.
fn stored_images(api: &Api, serial: &str) -> Vec<String> {
    let dir = api
        .context
        .store
        .device_dir(serial)
        .unwrap()
        .join(IMAGES_DIR);
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

async fn import(api: &Api, serial: &str, archive: Vec<u8>) -> Response<Body> {
    api.post(
        &format!("/api/devices/{serial}/import"),
        "application/zip",
        archive,
    )
    .await
}

async fn export(api: &Api, serial: &str) -> Vec<u8> {
    let response = api.get(&format!("/api/devices/{serial}/export")).await;
    assert_eq!(response.status(), StatusCode::OK);
    bytes(response).await
}

async fn bytes(response: Response<Body>) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
}

async fn api_error(response: Response<Body>) -> (StatusCode, ErrorCode) {
    let status = response.status();
    let error: ApiError = serde_json::from_slice(&bytes(response).await).unwrap();
    (status, error.code)
}

fn header<'a>(response: &'a Response<Body>, name: &str) -> &'a str {
    response.headers()[name].to_str().unwrap()
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    RgbaImage::from_pixel(width, height, Rgba([200, 30, 30, 255]))
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

async fn uploaded_name(response: Response<Body>) -> String {
    assert_eq!(response.status(), StatusCode::OK);
    let uploaded: ImageUploaded = serde_json::from_slice(&bytes(response).await).unwrap();
    uploaded.name
}

/// Puts image `name` on the first key of `serial`.
async fn use_image_on_first_key(api: &Api, serial: &str, name: &str) -> DeviceConfig {
    let mut config = api.config(serial).await;
    let first = config.home[0].clone();
    let module = config.modules.iter_mut().find(|m| m.id == first).unwrap();
    module.image = Some(name.to_owned());
    api.engine(serial).await.apply_config(config).await.unwrap()
}

#[tokio::test]
async fn uploaded_images_are_stored_by_content_and_served_rendered() {
    let api = Api::start().await;
    let name = uploaded_name(api.upload(FIRST, "image/png", png(40, 20)).await).await;
    assert_eq!(
        std::path::Path::new(&name).extension(),
        Some("png".as_ref()),
        "{name}"
    );
    let again = uploaded_name(api.upload(FIRST, "image/png", png(40, 20)).await).await;
    assert_eq!(again, name, "same content, same name");

    let svg = uploaded_name(
        api.upload(FIRST, "image/svg+xml; charset=utf-8", SVG.into())
            .await,
    )
    .await;
    let response = api.get(&format!("/api/devices/{FIRST}/images/{svg}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        "image/png",
        "never served as SVG"
    );
    assert!(header(&response, "cache-control").contains("immutable"));
    let rendered = image::load_from_memory(&bytes(response).await).unwrap();
    assert_eq!((rendered.width(), rendered.height()), (144, 144));
    api.stop().await;
}

#[tokio::test]
async fn uploads_are_refused_unless_they_are_images_of_the_declared_type() {
    let api = Api::start().await;
    let refusals = [
        api.upload(FIRST, "text/plain", png(4, 4)).await,
        api.upload(FIRST, "image/jpeg", png(4, 4)).await,
        api.upload(FIRST, "image/png", b"not an image".to_vec())
            .await,
        api.upload(FIRST, "image/svg+xml", b"<svg broken".to_vec())
            .await,
        api.upload(FIRST, "image/png", png(10_000, 1)).await,
    ];
    for response in refusals {
        assert_eq!(
            api_error(response).await,
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidImage)
        );
    }
    let unknown = api.upload("NOPE", "image/png", png(4, 4)).await;
    assert_eq!(
        api_error(unknown).await,
        (StatusCode::NOT_FOUND, ErrorCode::UnknownDevice)
    );
    let too_large = api
        .upload(FIRST, "image/png", vec![0; MAX_SOURCE_BYTES + 1])
        .await;
    assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        stored_images(&api, FIRST),
        Vec::<String>::new(),
        "nothing stored"
    );
    api.stop().await;
}

#[tokio::test]
async fn writes_need_the_origin_of_the_interface() {
    let api = Api::start().await;
    for path in [
        format!("/api/devices/{FIRST}/images"),
        format!("/api/devices/{FIRST}/import"),
    ] {
        let response = api
            .send(
                Request::post(path)
                    .header("host", HOST)
                    .header("origin", "http://evil.example")
                    .header("content-type", "image/png")
                    .body(Body::from(png(4, 4)))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let foreign_host = api
        .send(
            Request::get(format!("/api/devices/{FIRST}/export"))
                .header("host", "evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(foreign_host.status(), StatusCode::MISDIRECTED_REQUEST);
    api.stop().await;
}

#[tokio::test]
async fn missing_or_unsafe_image_names_are_not_served() {
    let api = Api::start().await;
    let missing = api
        .get(&format!("/api/devices/{FIRST}/images/0123.png"))
        .await;
    assert_eq!(
        api_error(missing).await,
        (StatusCode::NOT_FOUND, ErrorCode::InvalidImage)
    );
    let hidden = api
        .get(&format!("/api/devices/{FIRST}/images/.config"))
        .await;
    assert_eq!(hidden.status(), StatusCode::BAD_REQUEST);
    api.stop().await;
}

#[tokio::test]
async fn an_export_imports_on_another_device_with_its_images() {
    let api = Api::start().await;
    let name = uploaded_name(api.upload(FIRST, "image/png", png(8, 8)).await).await;
    let exported_config = use_image_on_first_key(&api, FIRST, &name).await;

    let export = api.get(&format!("/api/devices/{FIRST}/export")).await;
    assert_eq!(export.status(), StatusCode::OK);
    assert_eq!(header(&export, "content-type"), "application/zip");
    assert_eq!(header(&export, "content-disposition"), "attachment");
    let archive = bytes(export).await;
    let content = read_archive(&archive).unwrap();
    assert_eq!(content.config, exported_config);
    assert_eq!(content.images.keys().collect::<Vec<_>>(), [&name]);

    let before = api.config(SECOND).await;
    let response = api
        .post(
            &format!("/api/devices/{SECOND}/import"),
            "application/zip",
            archive,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let imported: DeviceConfig = serde_json::from_slice(&bytes(response).await).unwrap();
    assert_eq!(imported.serial, SECOND);
    assert_eq!(imported.revision, before.revision + 1);
    assert_eq!(imported.home, exported_config.home);
    assert_eq!(imported.modules, exported_config.modules);
    assert_eq!(api.config(SECOND).await, imported, "saved and applied");
    let image = api
        .get(&format!("/api/devices/{SECOND}/images/{name}"))
        .await;
    assert_eq!(image.status(), StatusCode::OK, "image copied to the device");
    api.stop().await;
}

fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[tokio::test]
async fn invalid_archives_are_refused_and_change_nothing() {
    let api = Api::start().await;
    let before = api.config(FIRST).await;
    let mut needs_image = before.clone();
    needs_image.modules[0].image = Some("absent.png".into());
    let needs_image = serde_json::to_vec(&needs_image).unwrap();
    let mut invalid = before.clone();
    invalid.brightness = 250;
    let invalid = serde_json::to_vec(&invalid).unwrap();
    let cases = [
        (b"not a zip".to_vec(), ErrorCode::InvalidArchive),
        (zip_with(&[("other.txt", b"x")]), ErrorCode::InvalidArchive),
        (
            zip_with(&[("config.json", &needs_image)]),
            ErrorCode::InvalidArchive,
        ),
        (
            zip_with(&[("config.json", &invalid)]),
            ErrorCode::InvalidConfig,
        ),
    ];
    for (archive, code) in cases {
        let response = api
            .post(
                &format!("/api/devices/{FIRST}/import"),
                "application/zip",
                archive,
            )
            .await;
        assert_eq!(api_error(response).await, (StatusCode::BAD_REQUEST, code));
    }
    assert_eq!(api.config(FIRST).await, before);
    api.stop().await;
}

fn jpeg() -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        16,
        16,
        image::Rgb([0, 90, 200]),
    ))
    .write_to(&mut bytes, ImageFormat::Jpeg)
    .unwrap();
    bytes.into_inner()
}

#[tokio::test]
async fn jpeg_and_untyped_uploads_are_read_from_their_content() {
    let api = Api::start().await;
    let name = uploaded_name(api.upload(FIRST, "image/jpeg", jpeg()).await).await;
    assert_eq!(
        std::path::Path::new(&name).extension(),
        Some("jpg".as_ref())
    );
    let untyped = uploaded_name(
        api.upload(FIRST, "application/octet-stream", png(3, 3))
            .await,
    )
    .await;
    assert_eq!(
        std::path::Path::new(&untyped).extension(),
        Some("png".as_ref())
    );
    api.stop().await;
}

#[tokio::test]
async fn reads_of_the_api_made_from_another_site_are_refused() {
    let api = Api::start().await;
    let response = api
        .send(
            Request::get(format!("/api/devices/{FIRST}/export"))
                .header("host", HOST)
                .header("sec-fetch-site", "cross-site")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    api.stop().await;
}

#[tokio::test]
async fn exports_and_imports_of_unknown_devices_fail() {
    let api = Api::start().await;
    let missing = api.get("/api/devices/NOPE/export").await;
    assert_eq!(
        api_error(missing).await,
        (StatusCode::NOT_FOUND, ErrorCode::UnknownDevice)
    );
    let archive = export(&api, FIRST).await;
    assert_eq!(
        api_error(import(&api, "NOPE", archive).await).await,
        (StatusCode::NOT_FOUND, ErrorCode::UnknownDevice)
    );
    let too_large = import(&api, FIRST, vec![0; MAX_ARCHIVE_BYTES + 1]).await;
    assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    api.stop().await;
}

#[tokio::test]
async fn imports_fit_the_target_model_both_ways() {
    let api = Api::start().await;
    let plus = api.config(FIRST).await;
    let response = import(&api, NO_DIALS, export(&api, FIRST).await).await;
    assert_eq!(response.status(), StatusCode::OK);
    let on_mk2: DeviceConfig = serde_json::from_slice(&bytes(response).await).unwrap();
    assert_eq!(on_mk2.model, DeviceModel::Mk2);
    assert_eq!(on_mk2.dials, Vec::new());
    assert_eq!(on_mk2.home, plus.home);
    assert!(
        on_mk2.modules.len() < plus.modules.len(),
        "dial-only modules dropped"
    );
    assert_eq!(api.config(NO_DIALS).await, on_mk2);

    let response = import(&api, SECOND, export(&api, NO_DIALS).await).await;
    assert_eq!(response.status(), StatusCode::OK);
    let back: DeviceConfig = serde_json::from_slice(&bytes(response).await).unwrap();
    assert_eq!(back.dials, vec![None; 4]);
    assert_eq!(back.home, plus.home);
    api.stop().await;
}

/// The default configuration of `serial` with `image` on its first key, as
/// an archive holding `content` under that name.
async fn archive_with_image(
    api: &Api,
    serial: &str,
    image: &str,
    content: &[u8],
    brightness: u8,
) -> Vec<u8> {
    let mut config = api.config(serial).await;
    config.brightness = brightness;
    let first = config.home[0].clone();
    config
        .modules
        .iter_mut()
        .find(|m| m.id == first)
        .unwrap()
        .image = Some(image.into());
    let json = serde_json::to_vec(&config).unwrap();
    zip_with(&[
        ("config.json", &json),
        (&format!("images/{image}"), content),
    ])
}

#[tokio::test]
async fn refused_imports_leave_no_image_behind() {
    let api = Api::start().await;
    // A compose file disguised as SVG: the former loose sniffing took it for one.
    let polyglot = b"# <svg>\nservices:\n  x:\n    image: alpine\n    privileged: true\n";
    let cases = [
        (
            archive_with_image(&api, FIRST, "a.svg", polyglot, 50).await,
            ErrorCode::InvalidImage,
        ),
        (
            archive_with_image(&api, FIRST, "b.png", &png(60_000, 1), 50).await,
            ErrorCode::InvalidImage,
        ),
        (
            archive_with_image(&api, FIRST, "c.png", &png(4, 4), 250).await,
            ErrorCode::InvalidConfig,
        ),
    ];
    for (archive, code) in cases {
        assert_eq!(
            api_error(import(&api, FIRST, archive).await).await,
            (StatusCode::BAD_REQUEST, code)
        );
        assert_eq!(stored_images(&api, FIRST), Vec::<String>::new());
    }
    api.stop().await;
}
