//! Internal HTTP routes of the UI, for what does not fit WebSocket messages:
//! key images and configuration archives (`/api/devices/{serial}/…`).
//!
//! - The access policy applies as everywhere: `Host` always, `Origin` on
//!   uploads and imports, no cross-site fetch.
//! - Bodies are bounded per route, and one transfer runs at a time.
//! - Every stored image (uploaded or imported) was first drawn by the
//!   renderer; images are only ever served rendered as PNG, never as stored:
//!   an SVG opened from the UI origin could run scripts there.
//! - An import writes nothing before its configuration is validated, and
//!   removes the images it added if the engine refuses it.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use bergadeck_protocol::{ApiError, ErrorCode, ImageUploaded};
use serde::Serialize;
use tokio::sync::SemaphorePermit;

use super::{AppState, UI_IMAGE_SIZE};
use crate::config_store::{ConfigStoreError, MAX_ARCHIVE_BYTES, prepare_import};
use crate::engine::{EngineError, EngineHandle};
use crate::render::{ImageKind, ImageSource, MAX_SOURCE_BYTES, RenderError, Renderer};

/// Images and their URLs never change (names come from the content).
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const JSON: &str = "application/json";
const PNG: &str = "image/png";
const ZIP: &str = "application/zip";
/// Upload of unknown type: the format is read from the content.
const UNKNOWN_TYPE: &str = "application/octet-stream";
/// What clients learn of an internal failure; details go to the logs.
const INTERNAL_MESSAGE: &str = "internal error, see the manager logs";

/// Routes under `/api`.
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/devices/{serial}/images",
            post(upload_image).layer(DefaultBodyLimit::max(MAX_SOURCE_BYTES)),
        )
        .route("/api/devices/{serial}/images/{name}", get(rendered_image))
        .route("/api/devices/{serial}/export", get(export))
        .route(
            "/api/devices/{serial}/import",
            post(import).layer(DefaultBodyLimit::max(MAX_ARCHIVE_BYTES)),
        )
}

/// A failed request, answered with an [`ApiError`] body.
#[derive(Debug)]
struct Failure {
    code: ErrorCode,
    message: String,
    /// HTTP status, when not the one of the code.
    status: Option<StatusCode>,
}

impl Failure {
    fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        let message = if code == ErrorCode::Internal {
            tracing::error!(error = %message.into(), "API request failed");
            INTERNAL_MESSAGE.to_owned()
        } else {
            message.into()
        };
        Self {
            code,
            message,
            status: None,
        }
    }

    fn unknown_device(serial: &str) -> Self {
        Self::new(ErrorCode::UnknownDevice, format!("unknown device {serial}"))
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self::new(ErrorCode::Internal, error.to_string())
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
            ErrorCode::StaleRevision => StatusCode::CONFLICT,
            ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
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
fn json(value: &impl Serialize) -> Response {
    match serde_json::to_vec(value) {
        Ok(body) => ([(CONTENT_TYPE, JSON)], body).into_response(),
        Err(error) => Failure::internal(error).into_response(),
    }
}

/// Runs CPU- or disk-bound work off the async runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Failure> + Send + 'static,
) -> Result<T, Failure> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(Failure::internal)?
}

/// Waits for the transfer slot.
async fn transfer_slot(state: &AppState) -> Result<SemaphorePermit<'_>, Failure> {
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

/// `GET /api/devices/{serial}/export`: the configuration archive, as a
/// download named by the UI.
async fn export(
    State(state): State<AppState>,
    Path(serial): Path<String>,
) -> Result<Response, Failure> {
    let config = engine(&state, &serial)?.config().borrow().clone();
    let _slot = transfer_slot(&state).await?;
    let store = state.context.store.clone();
    let archive = blocking(move || Ok(store.export(&serial, &config)?)).await?;
    Ok((
        [(CONTENT_TYPE, ZIP), (CONTENT_DISPOSITION, "attachment")],
        archive,
    )
        .into_response())
}

/// `POST /api/devices/{serial}/import`: replaces the configuration by an
/// archived one, fitted to the device, through the engine like any update
/// (validated, saved, broadcast). Answers the saved configuration.
async fn import(
    State(state): State<AppState>,
    Path(serial): Path<String>,
    body: Bytes,
) -> Result<Response, Failure> {
    let engine = engine(&state, &serial)?;
    let _slot = transfer_slot(&state).await?;
    let store = state.context.store.clone();
    let renderer = Arc::clone(&state.context.renderer);
    let device = engine.info().clone();
    let revision = engine.config().borrow().revision;
    let target = serial.clone();
    let (config, created) = blocking(move || {
        let check =
            |bytes: &[u8]| checked_kind(&renderer, bytes).map_err(|error| error.to_string());
        let prepared = prepare_import(&body, &device, revision, check)?;
        let created = store.write_import_images(&target, &prepared)?;
        Ok((prepared.config, created))
    })
    .await?;
    match engine.apply_config(config).await {
        Ok(saved) => Ok(json(&saved)),
        Err(error) => {
            let store = state.context.store.clone();
            blocking(move || {
                store.remove_images(&serial, &created);
                Ok(())
            })
            .await?;
            Err(error.into())
        }
    }
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
