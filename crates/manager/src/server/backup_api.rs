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

use std::collections::{BTreeMap, BTreeSet};
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
    Backup, BackupSection, ConfigStoreError, MAX_ARCHIVE_BYTES, MAX_BACKUP_ENTRIES,
    read_backup_or_archive, write_backup,
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
        .route("/api/backup/snapshots/{id}/preview", get(preview_restore))
        .route("/api/backup/snapshots/{id}/restore", post(restore))
}

/// What `?sections=` names: a section, or every device or every mouse (the
/// interface knows the devices that are connected, not the ones a backup or
/// this computer holds).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Choice {
    Section(BackupSection),
    AllDevices,
    AllMice,
}

/// `?sections=a,b`: the sections to take, every one when absent; and for an
/// import, `?map=OLD:NEW,…`: the configuration of the device `OLD` of the
/// backup goes to the connected device `NEW` (a replaced Stream Deck).
#[derive(Debug, Deserialize)]
struct Selection {
    sections: Option<String>,
    map: Option<String>,
}

/// The choices of a selection, never none.
struct Choices(Vec<Choice>);

impl Choices {
    /// Whether `section` is one of the choices.
    fn matches(&self, section: &BackupSection) -> bool {
        self.0.iter().any(|choice| match (choice, section) {
            (Choice::Section(chosen), _) => chosen == section,
            (Choice::AllDevices, BackupSection::Device(_))
            | (Choice::AllMice, BackupSection::Mouse(_)) => true,
            _ => false,
        })
    }

    /// The sections of `held` that the choices name.
    fn resolve<'a>(
        &self,
        held: impl Iterator<Item = &'a BackupSection>,
    ) -> BTreeSet<BackupSection> {
        held.filter(|section| self.matches(section))
            .cloned()
            .collect()
    }

    /// A section named exactly that `held` does not have.
    fn missing(&self, held: &BTreeSet<&BackupSection>) -> bool {
        self.0.iter().any(|choice| match choice {
            Choice::Section(section) => !held.contains(section),
            Choice::AllDevices | Choice::AllMice => false,
        })
    }
}

/// Most devices a backup is applied to other devices.
const MAX_DEVICE_MAP: usize = 64;

impl Selection {
    /// `OLD:NEW` pairs of serials: none when the query has none. Both serials
    /// must be plain ones, no device may be the target twice.
    fn device_map(&self) -> Result<BTreeMap<String, String>, Failure> {
        let refused = |reason: &str| Failure::new(ErrorCode::InvalidInput, reason);
        let Some(list) = &self.map else {
            return Ok(BTreeMap::new());
        };
        let mut map = BTreeMap::new();
        let mut targets = BTreeSet::new();
        for pair in list.split(',').filter(|pair| !pair.is_empty()) {
            let (old, new) = pair
                .split_once(':')
                .ok_or_else(|| refused("a device map is `OLD:NEW`"))?;
            let plain = |serial: &str| BackupSection::parse(&format!("devices/{serial}")).is_some();
            if !plain(old) || !plain(new) {
                return Err(refused("a device map names a serial that is not one"));
            }
            if map.insert(old.to_owned(), new.to_owned()).is_some() || !targets.insert(new) {
                return Err(refused("a device is named twice in the device map"));
            }
            if map.len() > MAX_DEVICE_MAP {
                return Err(refused("too many devices in the device map"));
            }
        }
        Ok(map)
    }

    /// The choices named; `None` when the query names none at all.
    fn parse(&self) -> Result<Option<Choices>, Failure> {
        let Some(list) = &self.sections else {
            return Ok(None);
        };
        let mut choices = Vec::new();
        for id in list.split(',').filter(|id| !id.is_empty()) {
            let choice = match id {
                "devices" => Choice::AllDevices,
                "mice" => Choice::AllMice,
                other => Choice::Section(BackupSection::parse(other).ok_or_else(|| {
                    Failure::new(ErrorCode::InvalidInput, "unknown section in the selection")
                })?),
            };
            if !choices.contains(&choice) {
                choices.push(choice);
            }
            if choices.len() > MAX_BACKUP_ENTRIES {
                return Err(Failure::new(
                    ErrorCode::InvalidInput,
                    "too many sections in the selection",
                ));
            }
        }
        // An empty selection is not "everything": it would take a snapshot for
        // nothing, and three of them push out the state before the first import.
        if choices.is_empty() {
            return Err(Failure::new(
                ErrorCode::InvalidInput,
                "the selection names no section",
            ));
        }
        Ok(Some(Choices(choices)))
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
    // The computer may have been locked while the request waited its turn.
    ensure_unlocked(&state)?;
    let store = state.context.store.clone();
    let archive = blocking(move || {
        let collected = store.collect_backup(APP_VERSION, now_seconds())?;
        let backup = match &selection {
            Some(choices) => {
                let chosen = choices.resolve(collected.backup.documents.keys());
                collected.backup.select(&chosen)
            }
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
        read_backup_or_archive(&body).map_err(|error| Failure::from(ConfigStoreError::from(error)))
    })
    .await
}

/// `POST /api/backup/preview`: what importing the uploaded backup would do.
async fn preview(State(state): State<AppState>, body: Bytes) -> Result<Response, Failure> {
    ensure_unlocked(&state)?;
    let _slot = transfer_slot(&state).await?;
    ensure_unlocked(&state)?;
    let backup = read_upload(body).await?;
    let store = state.context.store.clone();
    let current = blocking(move || Ok(store.collect_backup(APP_VERSION, now_seconds())?))
        .await?
        .backup
        .documents;
    let preview = backup_import::describe(&state.context, &backup, &current).await;
    Ok(json(&preview))
}

/// `POST /api/backup/import?sections=…&map=…`: takes the snapshot, then
/// applies the sections chosen (all by default) of the uploaded backup.
async fn import(
    State(state): State<AppState>,
    Query(selection): Query<Selection>,
    body: Bytes,
) -> Result<Response, Failure> {
    ensure_unlocked(&state)?;
    let map = selection.device_map()?;
    let choices = selection.parse()?;
    let _slot = transfer_slot(&state).await?;
    ensure_unlocked(&state)?;
    let backup = read_upload(body).await?;
    let sections = match &choices {
        Some(choices) => {
            let held = backup.documents.keys().collect();
            if choices.missing(&held) {
                return Err(Failure::new(
                    ErrorCode::InvalidInput,
                    "the backup does not hold a section asked for",
                ));
            }
            choices.resolve(backup.documents.keys())
        }
        None => backup.documents.keys().cloned().collect(),
    };
    if sections.is_empty() {
        return Err(Failure::new(
            ErrorCode::InvalidInput,
            "the backup holds nothing of what was asked",
        ));
    }
    check_device_map(&state, &map, &sections)?;
    apply_after_snapshot(&state, &backup, &sections, &map).await
}

/// A device map sends sections that are imported to devices that are
/// connected, and no device receives its own configuration and another's.
fn check_device_map(
    state: &AppState,
    map: &BTreeMap<String, String>,
    sections: &BTreeSet<BackupSection>,
) -> Result<(), Failure> {
    let refused = |reason: &str| Failure::new(ErrorCode::InvalidInput, reason);
    for (old, new) in map {
        if !sections.contains(&BackupSection::Device(old.clone())) {
            return Err(refused(
                "the device map names a section that is not imported",
            ));
        }
        if state.context.engines.get(new).is_none() {
            return Err(Failure::new(
                ErrorCode::UnknownDevice,
                "the device map names a device that is not connected",
            ));
        }
        let receives_its_own =
            sections.contains(&BackupSection::Device(new.clone())) && !map.contains_key(new);
        if receives_its_own {
            return Err(refused(
                "a device would receive its own configuration and another's",
            ));
        }
    }
    Ok(())
}

/// Takes the snapshot, then applies `sections` of `backup`.
async fn apply_after_snapshot(
    state: &AppState,
    backup: &Backup,
    sections: &BTreeSet<BackupSection>,
    map: &BTreeMap<String, String>,
) -> Result<Response, Failure> {
    let store = state.context.store.clone();
    let snapshot = blocking(move || Ok(store.take_snapshot(APP_VERSION)?)).await?;
    let sections = backup_import::apply(&state.context, backup, sections, map).await;
    Ok(json(&ImportReport { snapshot, sections }))
}

/// `GET /api/backup/snapshots`: the automatic snapshots, the latest first.
async fn snapshots(State(state): State<AppState>) -> Result<Response, Failure> {
    ensure_unlocked(&state)?;
    let store = state.context.store.clone();
    let list = blocking(move || Ok(store.list_snapshots()?)).await?;
    Ok(json(&list))
}

/// `GET /api/backup/snapshots/{id}/preview`: what restoring a snapshot would
/// do, with the names it would take over, shown before it is restored.
async fn preview_restore(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Failure> {
    ensure_unlocked(&state)?;
    let _slot = transfer_slot(&state).await?;
    ensure_unlocked(&state)?;
    let store = state.context.store.clone();
    let (snapshot, current) = blocking(move || {
        let snapshot = store.read_snapshot(&id)?;
        let current = store.collect_backup(APP_VERSION, now_seconds())?;
        Ok((snapshot, current.backup.documents))
    })
    .await?;
    Ok(json(
        &backup_import::describe(&state.context, &snapshot.backup, &current).await,
    ))
}

/// `POST /api/backup/snapshots/{id}/restore`: puts a snapshot back (after
/// taking one of what is there, so that the restore can be undone too).
async fn restore(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Failure> {
    ensure_unlocked(&state)?;
    let _slot = transfer_slot(&state).await?;
    ensure_unlocked(&state)?;
    let store = state.context.store.clone();
    // Read first: taking the new snapshot may forget the oldest.
    let snapshot = blocking(move || Ok(store.read_snapshot(&id)?)).await?;
    let sections = snapshot.backup.documents.keys().cloned().collect();
    apply_after_snapshot(&state, &snapshot.backup, &sections, &BTreeMap::new()).await
}
