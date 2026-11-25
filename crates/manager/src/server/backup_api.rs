//! Routes of the backup of the whole configuration (`/api/backup/…`): export,
//! preview of an import (nothing is written), import of the sections the
//! person chose, and the automatic snapshots taken before each import.
//!
//! - The access policy applies as everywhere (`Host`, `Origin` on writes, no
//!   cross-site fetch), nothing is done while the computer is locked, and one
//!   transfer runs at a time.
//! - An import reads the archive again: the server keeps nothing between the
//!   preview and the import.
//! - A snapshot of what is on the computer is taken **before** anything is
//!   written; if it cannot be taken, nothing is.

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use hubergamof_protocol::{ErrorCode, ImportReport};
use serde::Deserialize;

use super::api::{Failure, ZIP, blocking, ensure_unlocked, json, transfer_slot};
use super::{AppState, backup_import};
use crate::config_store::{
    Backup, BackupSection, ConfigStoreError, MAX_ARCHIVE_BYTES, MAX_BACKUP_ENTRIES, read_backup,
    write_backup,
};

/// Version written in the backups and snapshots.
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Routes under `/api/backup`.
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/backup/export", get(export))
        .route(
            "/api/backup/preview",
            post(preview).layer(DefaultBodyLimit::max(MAX_ARCHIVE_BYTES)),
        )
        .route(
            "/api/backup/import",
            post(import).layer(DefaultBodyLimit::max(MAX_ARCHIVE_BYTES)),
        )
        .route("/api/backup/snapshots", get(snapshots))
        .route("/api/backup/snapshots/{id}/restore", post(restore))
}

/// `?sections=a,b`: the sections to take, every one when absent.
#[derive(Debug, Deserialize)]
struct Selection {
    sections: Option<String>,
}

impl Selection {
    /// The sections named; `None` when the query names none.
    fn parse(&self) -> Result<Option<BTreeSet<BackupSection>>, Failure> {
        let Some(list) = &self.sections else {
            return Ok(None);
        };
        let mut sections = BTreeSet::new();
        for id in list.split(',').filter(|id| !id.is_empty()) {
            let section = BackupSection::parse(id).ok_or_else(|| {
                Failure::new(ErrorCode::InvalidInput, "unknown section in the selection")
            })?;
            sections.insert(section);
            if sections.len() > MAX_BACKUP_ENTRIES {
                return Err(Failure::new(
                    ErrorCode::InvalidInput,
                    "too many sections in the selection",
                ));
            }
        }
        Ok(Some(sections))
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// `GET /api/backup/export`: the backup of the sections asked for (all of
/// them by default) that this computer has, as a download.
async fn export(
    State(state): State<AppState>,
    Query(selection): Query<Selection>,
) -> Result<Response, Failure> {
    ensure_unlocked(&state)?;
    let selection = selection.parse()?;
    let _slot = transfer_slot(&state).await?;
    let store = state.context.store.clone();
    let archive = blocking(move || {
        let collected = store.collect_backup(APP_VERSION, now_seconds())?;
        let backup = match &selection {
            Some(sections) => collected.backup.select(sections),
            None => collected.backup,
        };
        write_backup(&backup).map_err(|error| Failure::from(ConfigStoreError::from(error)))
    })
    .await?;
    Ok((
        [(CONTENT_TYPE, ZIP), (CONTENT_DISPOSITION, "attachment")],
        archive,
    )
        .into_response())
}

/// Reads an uploaded backup.
async fn read_upload(body: Bytes) -> Result<Backup, Failure> {
    blocking(move || {
        read_backup(&body).map_err(|error| Failure::from(ConfigStoreError::from(error)))
    })
    .await
}

/// `POST /api/backup/preview`: what importing the uploaded backup would do.
async fn preview(State(state): State<AppState>, body: Bytes) -> Result<Response, Failure> {
    ensure_unlocked(&state)?;
    let _slot = transfer_slot(&state).await?;
    let backup = read_upload(body).await?;
    let store = state.context.store.clone();
    let current = blocking(move || Ok(store.collect_backup(APP_VERSION, now_seconds())?))
        .await?
        .backup
        .documents;
    let preview = backup_import::describe(&state.context, &backup, &current).await;
    Ok(json(&preview))
}

/// `POST /api/backup/import?sections=…`: takes the snapshot, then applies the
/// sections chosen (all by default) of the uploaded backup.
async fn import(
    State(state): State<AppState>,
    Query(selection): Query<Selection>,
    body: Bytes,
) -> Result<Response, Failure> {
    ensure_unlocked(&state)?;
    let selection = selection.parse()?;
    let _slot = transfer_slot(&state).await?;
    let backup = read_upload(body).await?;
    let sections = match selection {
        Some(sections) => {
            if sections.iter().any(|s| !backup.documents.contains_key(s)) {
                return Err(Failure::new(
                    ErrorCode::InvalidInput,
                    "the backup does not hold a section asked for",
                ));
            }
            sections
        }
        None => backup.documents.keys().cloned().collect(),
    };
    apply_after_snapshot(&state, &backup, &sections).await
}

/// Takes the snapshot, then applies `sections` of `backup`.
async fn apply_after_snapshot(
    state: &AppState,
    backup: &Backup,
    sections: &BTreeSet<BackupSection>,
) -> Result<Response, Failure> {
    let store = state.context.store.clone();
    let snapshot = blocking(move || Ok(store.take_snapshot(APP_VERSION)?)).await?;
    let sections = backup_import::apply(&state.context, backup, sections).await;
    Ok(json(&ImportReport { snapshot, sections }))
}

/// `GET /api/backup/snapshots`: the automatic snapshots, the latest first.
async fn snapshots(State(state): State<AppState>) -> Result<Response, Failure> {
    ensure_unlocked(&state)?;
    let store = state.context.store.clone();
    let list = blocking(move || Ok(store.list_snapshots()?)).await?;
    Ok(json(&list))
}

/// `POST /api/backup/snapshots/{id}/restore`: puts a snapshot back (after
/// taking one of what is there, so that the restore can be undone too).
async fn restore(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Failure> {
    ensure_unlocked(&state)?;
    let _slot = transfer_slot(&state).await?;
    let store = state.context.store.clone();
    // Read first: taking the new snapshot may forget the oldest.
    let snapshot = blocking(move || Ok(store.read_snapshot(&id)?)).await?;
    let sections = snapshot.backup.documents.keys().cloned().collect();
    apply_after_snapshot(&state, &snapshot.backup, &sections).await
}
