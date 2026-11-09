//! Bodies of the internal HTTP routes of the UI (`/api/…`): images and
//! configuration archives, which do not fit WebSocket messages.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ErrorCode;

/// Answer to an image upload (`POST /api/devices/{serial}/images`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImageUploaded {
    /// Name of the stored image, to put in `ModuleInstance.image`.
    pub name: String,
}

/// Answer to a file chooser request (`POST /api/file-chooser/…`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileChosen {
    /// Absolute path of the file the user chose; `None` when they closed the
    /// chooser without choosing.
    pub path: Option<String>,
}

/// Body of a failed `/api/…` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApiError {
    /// Machine-readable error code.
    pub code: ErrorCode,
    /// Human-readable details.
    pub message: String,
}
