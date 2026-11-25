//! Internal HTTP routes of the UI: key images and configuration archives.

#![cfg(test)]

mod support;

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use hubergamof_manager::file_chooser::{
    FilePicker, FilePickerError, NoFilePicker, SimulatedFilePicker,
};
use hubergamof_manager::render::MAX_SOURCE_BYTES;
use hubergamof_manager::screen_lock::ScreenLock;
use hubergamof_protocol::{ErrorCode, FileChooser, FileChosen, ImageUploaded};
use image::ImageFormat;
use tokio::sync::watch;
use tower::ServiceExt as _;

use support::{Api, CHOSEN_FILE, FIRST, HOST, ORIGIN, WAIT, api_error, bytes, png, stored_images};

/// The file chooser of the compose files of the Docker module.
const CHOOSER_REQUEST: &str = r#"{ "module": "docker", "setting": "composeFiles" }"#;
const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2 2"><rect width="1" height="1" fill="red"/></svg>"#;

fn header<'a>(response: &'a Response<Body>, name: &str) -> &'a str {
    response.headers()[name].to_str().unwrap()
}

async fn uploaded_name(response: Response<Body>) -> String {
    assert_eq!(response.status(), StatusCode::OK);
    let uploaded: ImageUploaded = serde_json::from_slice(&bytes(response).await).unwrap();
    uploaded.name
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
    for path in [format!("/api/devices/{FIRST}/images")] {
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
            Request::get(format!("/api/devices/{FIRST}/images/any.png"))
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
            Request::get(format!("/api/devices/{FIRST}/images/any.png"))
                .header("host", HOST)
                .header("sec-fetch-site", "cross-site")
                .body(Body::from(CHOOSER_REQUEST))
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    api.stop().await;
}

async fn choose_compose_file(api: &Api) -> Response<Body> {
    api.send(
        Request::post("/api/file-chooser")
            .header("host", HOST)
            .header("origin", ORIGIN)
            .body(Body::from(CHOOSER_REQUEST))
            .unwrap(),
    )
    .await
}

#[tokio::test]
async fn the_file_chooser_answers_the_path_the_user_chose() {
    let api = Api::start().await;
    let response = choose_compose_file(&api).await;
    assert_eq!(response.status(), StatusCode::OK);
    let chosen: FileChosen = serde_json::from_slice(&bytes(response).await).unwrap();
    assert_eq!(chosen.path.as_deref(), Some(CHOSEN_FILE));
    api.stop().await;

    let cancelled = Api::start_with(Arc::new(SimulatedFilePicker::choosing(None))).await;
    let response = choose_compose_file(&cancelled).await;
    let chosen: FileChosen = serde_json::from_slice(&bytes(response).await).unwrap();
    assert_eq!(chosen.path, None, "closing the dialog is not an error");
    cancelled.stop().await;
}

#[tokio::test]
async fn the_file_chooser_is_unavailable_without_a_desktop() {
    let api = Api::start_with(Arc::new(NoFilePicker)).await;
    assert_eq!(
        api_error(choose_compose_file(&api).await).await,
        (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::Unavailable)
    );
    api.stop().await;
}

#[tokio::test]
async fn the_file_chooser_is_refused_to_foreign_origins() {
    let api = Api::start().await;
    let response = api
        .send(
            Request::post("/api/file-chooser")
                .header("host", HOST)
                .header("origin", "http://evil.example")
                .body(Body::from(CHOOSER_REQUEST))
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    api.stop().await;
}

/// A dialog that stays open until `release` is notified.
struct HeldDialog {
    opened: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl FilePicker for HeldDialog {
    async fn pick_file(
        &self,
        _chooser: &FileChooser,
    ) -> Result<Option<std::path::PathBuf>, FilePickerError> {
        self.opened.notify_one();
        self.release.notified().await;
        Ok(Some(CHOSEN_FILE.into()))
    }
}

#[tokio::test]
async fn a_dialog_stays_the_only_one_even_when_its_request_is_dropped() {
    let opened = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let api = Api::start_with(Arc::new(HeldDialog {
        opened: Arc::clone(&opened),
        release: Arc::clone(&release),
    }))
    .await;
    let request = |router: axum::Router| {
        router.oneshot(
            Request::post("/api/file-chooser")
                .header("host", HOST)
                .header("origin", ORIGIN)
                .body(Body::from(CHOOSER_REQUEST))
                .unwrap(),
        )
    };
    let first = tokio::spawn(request(api.router.clone()));
    tokio::time::timeout(WAIT, opened.notified()).await.unwrap();

    let second = request(api.router.clone()).await.unwrap();
    assert_eq!(
        api_error(second).await,
        (StatusCode::CONFLICT, ErrorCode::Busy)
    );

    // The client goes away: the dialog is still on the screen, so still the only one.
    first.abort();
    let _ = first.await;
    let third = request(api.router.clone()).await.unwrap();
    assert_eq!(third.status(), StatusCode::CONFLICT);

    release.notify_one();
    let mut accepted = false;
    for _ in 0..50 {
        if request(api.router.clone()).await.unwrap().status() == StatusCode::OK {
            accepted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        release.notify_one();
    }
    assert!(accepted, "a new dialog opens once the first one is over");
    api.stop().await;
}

struct BrokenDialog;

#[async_trait::async_trait]
impl FilePicker for BrokenDialog {
    async fn pick_file(
        &self,
        _chooser: &FileChooser,
    ) -> Result<Option<std::path::PathBuf>, FilePickerError> {
        Err(FilePickerError::Failed(
            "secret detail /run/user/1000".into(),
        ))
    }
}

#[tokio::test]
async fn a_failing_dialog_is_an_internal_error_without_its_details() {
    let api = Api::start_with(Arc::new(BrokenDialog)).await;
    let response = choose_compose_file(&api).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = String::from_utf8(bytes(response).await).unwrap();
    assert!(!body.contains("secret detail"), "{body}");
    api.stop().await;
}

#[tokio::test]
async fn a_file_chooser_is_only_opened_for_a_setting_that_describes_one() {
    let api = Api::start().await;
    let ask = |body: String| {
        api.send(
            Request::post("/api/file-chooser")
                .header("host", HOST)
                .header("origin", ORIGIN)
                .body(Body::from(body))
                .unwrap(),
        )
    };
    for body in [
        r#"{ "module": "docker", "setting": "nothing" }"#,
        r#"{ "module": "audio", "setting": "composeFiles" }"#,
        r#"{ "module": "pomodoro", "setting": "workMinutes" }"#,
        r#"{ "module": "docker" }"#,
        "not json",
    ] {
        assert_eq!(
            api_error(ask(body.to_owned()).await).await,
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidInput),
            "{body}"
        );
    }
    let huge = format!(r#"{{ "module": "{}", "setting": "x" }}"#, "d".repeat(2000));
    assert_eq!(ask(huge).await.status(), StatusCode::PAYLOAD_TOO_LARGE);
    api.stop().await;
}

/// A computer that is locked.
struct Locked;

impl ScreenLock for Locked {
    fn watch(&self) -> watch::Receiver<bool> {
        watch::Sender::new(true).subscribe()
    }
}

#[tokio::test]
async fn nothing_is_uploaded_or_chosen_while_the_computer_is_locked() {
    let api = Api::start_on(
        Arc::new(SimulatedFilePicker::choosing(Some(CHOSEN_FILE.into()))),
        Arc::new(Locked),
    )
    .await;
    let locked = (StatusCode::LOCKED, ErrorCode::Locked);

    assert_eq!(
        api_error(api.upload(FIRST, "image/png", png(8, 8)).await).await,
        locked
    );
    assert_eq!(api_error(choose_compose_file(&api).await).await, locked);
    let image = api
        .get(&format!("/api/devices/{FIRST}/images/any.png"))
        .await;
    assert_eq!(api_error(image).await, locked);
    assert_eq!(stored_images(&api, FIRST).len(), 0);
    api.stop().await;
}
