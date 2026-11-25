//! Internal HTTP routes of the UI, for what does not fit WebSocket messages:
//! key images (`/api/devices/{serial}/…`) and the desktop's file dialog
//! (`/api/file-chooser/…`). The backup of the configuration has its own routes
//! (`backup_api.rs`).
//!
//! - The access policy applies as everywhere: `Host` always, `Origin` on
//!   uploads, no cross-site fetch.
//! - Bodies are bounded per route, and one transfer runs at a time.
//! - Every stored image (uploaded or imported) was first drawn by the
//!   renderer; images are only ever served rendered as PNG, never as stored:
//!   an SVG opened from the UI origin could run scripts there.

use std::sync::Arc;

#[cfg(test)]
use super::errors::INTERNAL_MESSAGE;
use super::errors::client_message;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use hubergamof_protocol::{
    ApiError, ErrorCode, FileChooser, FileChooserRequest, FileChosen, ImageUploaded, SettingKind,
};
use serde::Serialize;
use tokio::sync::SemaphorePermit;

use super::{AppState, UI_IMAGE_SIZE};
use crate::config_store::ConfigStoreError;
use crate::engine::{EngineError, EngineHandle};
use crate::file_chooser::FilePickerError;
use crate::render::{ImageKind, ImageSource, MAX_SOURCE_BYTES, RenderError, Renderer};

/// Images and their URLs never change (names come from the content).
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const JSON: &str = "application/json";
const PNG: &str = "image/png";
pub(super) const ZIP: &str = "application/zip";
/// Upload of unknown type: the format is read from the content.
const UNKNOWN_TYPE: &str = "application/octet-stream";

/// Routes under `/api`.
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/devices/{serial}/images",
            post(upload_image).layer(DefaultBodyLimit::max(MAX_SOURCE_BYTES)),
        )
        .route("/api/devices/{serial}/images/{name}", get(rendered_image))
        .route(
            "/api/file-chooser",
            post(choose_file).layer(DefaultBodyLimit::max(MAX_CHOOSER_REQUEST_BYTES)),
        )
}

/// A failed request, answered with an [`ApiError`] body.
#[derive(Debug)]
pub(super) struct Failure {
    code: ErrorCode,
    message: String,
    /// HTTP status, when not the one of the code.
    status: Option<StatusCode>,
}

impl Failure {
    pub(super) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: client_message(code, message, "API request"),
            status: None,
        }
    }

    fn unknown_device(serial: &str) -> Self {
        Self::new(ErrorCode::UnknownDevice, format!("unknown device {serial}"))
    }

    pub(super) fn internal(error: impl std::fmt::Display) -> Self {
        Self::new(ErrorCode::Internal, error.to_string())
    }

    fn locked() -> Self {
        Self::new(
            ErrorCode::Locked,
            "the computer is locked: nothing is done from the interface",
        )
    }

    fn missing_image(name: &str) -> Self {
        Self {
            status: Some(StatusCode::NOT_FOUND),
            ..Self::new(ErrorCode::InvalidImage, format!("no image {name}"))
        }
    }

    fn status(&self) -> StatusCode {
        if let Some(status) = self.status {
            return status;
        }
        match self.code {
            ErrorCode::UnknownDevice => StatusCode::NOT_FOUND,
            ErrorCode::Locked => StatusCode::LOCKED,
            ErrorCode::StaleRevision | ErrorCode::Busy => StatusCode::CONFLICT,
            ErrorCode::Internal | ErrorCode::NotApplied => StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::InvalidMessage
            | ErrorCode::InvalidConfig
            | ErrorCode::InvalidInput
            | ErrorCode::InputRefused
            | ErrorCode::InvalidImage
            | ErrorCode::InvalidArchive => StatusCode::BAD_REQUEST,
        }
    }
}

impl From<ConfigStoreError> for Failure {
    fn from(error: ConfigStoreError) -> Self {
        Self::new(error.error_code(), error.to_string())
    }
}

impl From<EngineError> for Failure {
    fn from(error: EngineError) -> Self {
        Self::new(error.error_code(), error.to_string())
    }
}

impl From<FilePickerError> for Failure {
    fn from(error: FilePickerError) -> Self {
        match error {
            FilePickerError::Unavailable => Self::new(ErrorCode::Unavailable, error.to_string()),
            FilePickerError::Failed(_) => Self::internal(error),
        }
    }
}

impl From<RenderError> for Failure {
    /// The image cannot be drawn.
    fn from(error: RenderError) -> Self {
        Self::new(ErrorCode::InvalidImage, error.to_string())
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = ApiError {
            code: self.code,
            message: self.message,
        };
        (status, json(&body)).into_response()
    }
}

/// `value` as a JSON response body.
pub(super) fn json(value: &impl Serialize) -> Response {
    match serde_json::to_vec(value) {
        Ok(body) => ([(CONTENT_TYPE, JSON)], body).into_response(),
        Err(error) => Failure::internal(error).into_response(),
    }
}

/// Runs CPU- or disk-bound work off the async runtime.
pub(super) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Failure> + Send + 'static,
) -> Result<T, Failure> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(Failure::internal)?
}

/// Waits for the transfer slot.
/// Nothing is done from the interface while the computer is locked.
pub(super) fn ensure_unlocked(state: &AppState) -> Result<(), Failure> {
    if *state.context.screen_lock.watch().borrow() {
        Err(Failure::locked())
    } else {
        Ok(())
    }
}

pub(super) async fn transfer_slot(state: &AppState) -> Result<SemaphorePermit<'_>, Failure> {
    state.transfers.acquire().await.map_err(Failure::internal)
}

fn engine(state: &AppState, serial: &str) -> Result<EngineHandle, Failure> {
    state
        .context
        .engines
        .get(serial)
        .ok_or_else(|| Failure::unknown_device(serial))
}

/// The check every stored image passes: drawn at the UI size, uncached.
fn checked_kind(renderer: &Renderer, bytes: &[u8]) -> Result<ImageKind, RenderError> {
    renderer.check_image(bytes, UI_IMAGE_SIZE)
}

/// Format declared by the `Content-Type` of an upload; `None` when unknown
/// (browsers send no type for files the system cannot identify).
fn declared_kind(headers: &HeaderMap) -> Result<Option<ImageKind>, Failure> {
    let declared = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase());
    match declared.as_deref() {
        Some("image/png") => Ok(Some(ImageKind::Png)),
        Some("image/jpeg") => Ok(Some(ImageKind::Jpeg)),
        Some("image/svg+xml") => Ok(Some(ImageKind::Svg)),
        None | Some(UNKNOWN_TYPE) => Ok(None),
        Some(_) => Err(Failure::new(
            ErrorCode::InvalidImage,
            "expected an image/png, image/jpeg or image/svg+xml body",
        )),
    }
}

/// `POST /api/devices/{serial}/images`: stores a key image once it renders.
async fn upload_image(
    State(state): State<AppState>,
    Path(serial): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Failure> {
    ensure_unlocked(&state)?;
    let declared = declared_kind(&headers)?;
    engine(&state, &serial)?;
    let _slot = transfer_slot(&state).await?;
    let store = state.context.store.clone();
    let renderer = Arc::clone(&state.context.renderer);
    let name = blocking(move || {
        let kind = checked_kind(&renderer, &body)?;
        Ok(store.store_image(&serial, &body, declared.unwrap_or(kind))?)
    })
    .await?;
    Ok(json(&ImageUploaded { name }))
}

/// `GET /api/devices/{serial}/images/{name}`: the image rendered like a key.
/// Any device with a configuration on disk, attached or not: the UI may
/// still show a device being unplugged.
async fn rendered_image(
    State(state): State<AppState>,
    Path((serial, name)): Path<(String, String)>,
) -> Result<Response, Failure> {
    ensure_unlocked(&state)?;
    let store = state.context.store.clone();
    let renderer = Arc::clone(&state.context.renderer);
    let png = blocking(move || {
        let path = store.image_path(&serial, &name)?;
        if !path.is_file() {
            return Err(Failure::missing_image(&name));
        }
        Ok(renderer
            .render_image(UI_IMAGE_SIZE, &ImageSource::File(path))?
            .png()?)
    })
    .await?;
    Ok((
        [(CONTENT_TYPE, PNG), (CACHE_CONTROL, IMMUTABLE)],
        Bytes::from_owner(png),
    )
        .into_response())
}

/// Largest body of a file chooser request.
const MAX_CHOOSER_REQUEST_BYTES: usize = 1024;

/// `POST /api/file-chooser`: opens the desktop's file dialog of a setting
/// (`{ module, setting }`) and answers the path of the file the user chose
/// (`null` when they closed it). The dialog is the one the setting
/// describes; needs the user at the screen, and only one dialog at a time.
async fn choose_file(State(state): State<AppState>, body: Bytes) -> Result<Response, Failure> {
    ensure_unlocked(&state)?;
    let chooser = chooser_of(&state, &body)?;
    let dialog = Arc::clone(&state.file_dialog)
        .try_acquire_owned()
        .map_err(|_| Failure::new(ErrorCode::Busy, "a file chooser is already open"))?;
    let picker = Arc::clone(&state.context.file_picker);
    // The dialog lives on the user's screen whatever becomes of this request: it runs apart,
    // holding its place until it is answered or closed, so that a client dropping its
    // connection cannot pile dialogs up.
    let chosen = tokio::spawn(async move {
        let chosen = picker.pick_file(&chooser).await;
        drop(dialog);
        chosen
    })
    .await
    .map_err(Failure::internal)??;
    Ok(json(&FileChosen {
        path: chosen.map(|path| path.to_string_lossy().into_owned()),
    }))
}

/// The dialog the setting named by `body` describes.
fn chooser_of(state: &AppState, body: &[u8]) -> Result<FileChooser, Failure> {
    let invalid = |reason: &str| Failure::new(ErrorCode::InvalidInput, reason);
    let request: FileChooserRequest = (body.len() <= MAX_CHOOSER_REQUEST_BYTES)
        .then(|| serde_json::from_slice(body).ok())
        .flatten()
        .ok_or_else(|| invalid("expected { module, setting }"))?;
    state
        .context
        .catalog
        .iter()
        .filter(|module| module.descriptor.kind.0 == request.module)
        .flat_map(|module| &module.descriptor.settings)
        .filter(|field| field.key == request.setting)
        .find_map(|field| match &field.kind {
            SettingKind::FileEntryList { chooser, .. } => chooser.clone(),
            _ => None,
        })
        .ok_or_else(|| invalid("no file chooser for this setting"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_map_to_http_statuses() {
        let status = |code| Failure::new(code, "x").status();
        assert_eq!(status(ErrorCode::UnknownDevice), StatusCode::NOT_FOUND);
        assert_eq!(status(ErrorCode::StaleRevision), StatusCode::CONFLICT);
        assert_eq!(status(ErrorCode::InvalidArchive), StatusCode::BAD_REQUEST);
        assert_eq!(
            status(ErrorCode::Unavailable),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(status(ErrorCode::Busy), StatusCode::CONFLICT);
        assert_eq!(
            status(ErrorCode::Internal),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            Failure::missing_image("a.png").status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn internal_details_stay_in_the_logs() {
        let failure = Failure::internal("cannot write /home/me/config/devices/A/images/x.png");
        assert_eq!(failure.message, INTERNAL_MESSAGE);
        assert_eq!(
            Failure::new(ErrorCode::InvalidImage, "bad png").message,
            "bad png"
        );
    }
}
