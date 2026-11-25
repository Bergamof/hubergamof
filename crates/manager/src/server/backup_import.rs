//! What an import does to the computer: every section of a backup goes
//! through the one that owns it (the engine of the device, the mouse hub, the
//! module host, the hubs of the machine), never written under its feet, and
//! with the revision it holds now: the revisions of a file from elsewhere say
//! nothing here. A section that fails leaves the others to go on: the
//! snapshot taken before is what undoes them.

use std::collections::BTreeSet;

use hubergamof_protocol::{
    BackupPreview, ErrorCode, ExecutableKind, SectionOutcome, SectionResult,
};
use serde_json::Value;

use super::errors::client_message;
use super::{ServerContext, UI_IMAGE_SIZE};
use crate::config_store::{
    ArchiveImages, Backup, BackupDocument, BackupSection, ConfigStoreError, preview,
};
use crate::engine::EngineError;
use crate::machine::{Feature, Hub, HubError};
use crate::module_host::SettingsRefusal;
use crate::render::ImageKind;

/// Why a section of an import did not go through, for the person.
fn refused(code: ErrorCode, detail: impl Into<String>) -> SectionResult {
    SectionResult::Refused {
        code,
        message: client_message(code, detail, "backup import"),
    }
}

fn refused_by_store(error: &ConfigStoreError) -> SectionResult {
    refused(error.error_code(), error.to_string())
}

/// Applies the `sections` of `backup`, in the order of the sections (devices,
/// mice, radial menu, modules, then the machine), and says what became of each.
pub(super) async fn apply(
    context: &ServerContext,
    backup: &Backup,
    sections: &BTreeSet<BackupSection>,
) -> Vec<SectionOutcome> {
    let mut outcomes = Vec::new();
    for (section, document) in &backup.documents {
        if !sections.contains(section) {
            continue;
        }
        // Checked before each section: the computer may lock during an import,
        // and what is left is not written then.
        let outcome = if *context.screen_lock.watch().borrow() {
            refused(ErrorCode::Locked, "the computer is locked")
        } else {
            apply_section(context, backup, section, document).await
        };
        outcomes.push(SectionOutcome {
            section: section.id(),
            outcome,
        });
    }
    outcomes
}

async fn apply_section(
    context: &ServerContext,
    backup: &Backup,
    section: &BackupSection,
    document: &BackupDocument,
) -> SectionResult {
    match (section, document) {
        (BackupSection::Device(serial), BackupDocument::Device(config)) => {
            let images = backup.images.get(serial).cloned().unwrap_or_default();
            device(context, serial, (**config).clone(), images).await
        }
        (BackupSection::Mouse(serial), BackupDocument::Mouse(config)) => {
            mouse(context, serial, (**config).clone()).await
        }
        (BackupSection::RadialMenu, BackupDocument::RadialMenu(config)) => {
            radial_menu(context, config.clone()).await
        }
        (BackupSection::Modules, BackupDocument::Modules(config)) => {
            modules(
                context,
                config.modules.iter().map(|m| (&m.kind, &m.settings)),
            )
            .await
        }
        (BackupSection::Machine, BackupDocument::Machine(config)) => {
            update_hub(
                &context.machine,
                config.clone(),
                |snapshot| snapshot.config.revision,
                |config, revision| config.revision = revision,
            )
            .await
        }
        (BackupSection::Git, BackupDocument::Git(config)) => {
            update_hub(
                &context.git,
                (**config).clone(),
                |snapshot| snapshot.config.revision,
                |config, revision| config.revision = revision,
            )
            .await
        }
        (BackupSection::Ssh, BackupDocument::Ssh(config)) => {
            update_hub(
                &context.ssh,
                config.clone(),
                |snapshot| snapshot.config.revision,
                |config, revision| config.revision = revision,
            )
            .await
        }
        _ => refused(
            ErrorCode::InvalidArchive,
            "the document is not of its section",
        ),
    }
}

/// A device: through its engine when it is connected, the images first and
/// removed again if the engine refuses; else stored for its next connection.
async fn device(
    context: &ServerContext,
    serial: &str,
    config: hubergamof_protocol::DeviceConfig,
    images: ArchiveImages,
) -> SectionResult {
    let store = context.store.clone();
    let renderer = std::sync::Arc::clone(&context.renderer);
    let check = move |bytes: &[u8]| -> Result<ImageKind, String> {
        renderer
            .check_image(bytes, UI_IMAGE_SIZE)
            .map_err(|error| error.to_string())
    };
    let Some(engine) = context.engines.get(serial) else {
        let stored =
            tokio::task::spawn_blocking(move || store.import_absent_device(config, &images, check))
                .await;
        return match stored {
            Ok(Ok(_)) => SectionResult::Stored,
            Ok(Err(error)) => refused_by_store(&error),
            Err(error) => refused(ErrorCode::Internal, error.to_string()),
        };
    };
    let revision = engine.config().borrow().revision;
    let info = engine.info().clone();
    let target = serial.to_owned();
    let prepared = {
        let store = store.clone();
        tokio::task::spawn_blocking(move || {
            let prepared = store.prepare_device_import(config, &images, &info, revision, check)?;
            let created = store.write_import_images(&target, &prepared)?;
            Ok::<_, ConfigStoreError>((prepared, created))
        })
        .await
    };
    let (prepared, created) = match prepared {
        Ok(Ok(prepared)) => prepared,
        Ok(Err(error)) => return refused_by_store(&error),
        Err(error) => return refused(ErrorCode::Internal, error.to_string()),
    };
    match engine.apply_config(prepared.config).await {
        Ok(_) => SectionResult::Applied,
        Err(error) => {
            let owner = serial.to_owned();
            let _ =
                tokio::task::spawn_blocking(move || store.remove_images(&owner, &created)).await;
            refused_by_engine(&error)
        }
    }
}

fn refused_by_engine(error: &EngineError) -> SectionResult {
    refused(error.error_code(), error.to_string())
}

/// A mouse: through the hub when it is connected, else stored.
async fn mouse(
    context: &ServerContext,
    serial: &str,
    mut config: hubergamof_protocol::MouseConfig,
) -> SectionResult {
    let hub = context.mice.clone();
    let store = context.store.clone();
    let serial = serial.to_owned();
    let result = tokio::task::spawn_blocking(move || {
        let connected = hub.mice().iter().any(|mouse| mouse.serial == serial);
        let revision = hub
            .subscribe_config(&serial)
            .filter(|_| connected)
            .map(|config| config.borrow().revision);
        match revision {
            Some(revision) => {
                config.revision = revision;
                hub.update_mouse(&serial, config)
                    .map(|_| SectionResult::Applied)
                    .map_err(|error| refused(error.error_code(), error.to_string()))
            }
            None => store
                .replace_mouse(config)
                .map(|_| SectionResult::Stored)
                .map_err(|error| refused_by_store(&error)),
        }
    })
    .await;
    match result {
        Ok(Ok(done) | Err(done)) => done,
        Err(error) => refused(ErrorCode::Internal, error.to_string()),
    }
}

async fn radial_menu(
    context: &ServerContext,
    mut config: hubergamof_protocol::RadialMenuConfig,
) -> SectionResult {
    let hub = context.mice.clone();
    let result = tokio::task::spawn_blocking(move || {
        config.revision = hub.subscribe_radial_menu().borrow().revision;
        hub.update_radial_menu(config)
    })
    .await;
    match result {
        Ok(Ok(_)) => SectionResult::Applied,
        Ok(Err(error)) => refused_by_store(&error),
        Err(error) => refused(ErrorCode::Internal, error.to_string()),
    }
}

/// The modules of the application, one kind at a time through the host that
/// holds them. What the host does not hold is said, not skipped silently.
async fn modules<'a>(
    context: &ServerContext,
    instances: impl Iterator<Item = (&'a hubergamof_protocol::ModuleKind, &'a Value)>,
) -> SectionResult {
    let Some(host) = &context.application else {
        return refused(
            ErrorCode::Unavailable,
            "the application runs without modules",
        );
    };
    let (mut applied, mut failures) = (0_usize, Vec::new());
    for (kind, settings) in instances {
        match host.set_settings(kind, settings).await {
            Ok(()) => applied += 1,
            Err(SettingsRefusal::Locked) => {
                return refused(ErrorCode::Locked, SettingsRefusal::Locked.to_string());
            }
            Err(error) => failures.push(format!("{kind}: {error}")),
        }
    }
    if failures.is_empty() {
        SectionResult::Applied
    } else if applied == 0 {
        refused(ErrorCode::InvalidConfig, failures.join("; "))
    } else {
        SectionResult::NotApplied {
            message: failures.join("; "),
        }
    }
}

/// A section of the machine: saved with the revision it has now, then put on
/// the computer by the hub, as an edit of the person would be. What the hub
/// found in the files of the user is taken over there, as for any edit.
async fn update_hub<F: Feature>(
    hub: &Hub<F>,
    mut config: F::Config,
    revision_of: fn(&F::Snapshot) -> u64,
    set_revision: fn(&mut F::Config, u64),
) -> SectionResult {
    match hub.refresh().await {
        Ok(snapshot) => set_revision(&mut config, revision_of(&snapshot)),
        Err(error) => return refused(error.error_code(), error.to_string()),
    }
    match hub.update(config).await {
        Ok(()) => SectionResult::Applied,
        Err(error @ (HubError::Apply(_) | HubError::NoHome)) => SectionResult::NotApplied {
            message: error.to_string(),
        },
        Err(error) => refused(error.error_code(), error.to_string()),
    }
}

/// What importing `backup` would do: [`preview`] against what is on the disk,
/// with whether each device is connected and which names of the backup
/// would take over a shell alias or an SSH host that the computer has.
/// Nothing is written.
pub(super) async fn describe(
    context: &ServerContext,
    backup: &Backup,
    current: &std::collections::BTreeMap<BackupSection, BackupDocument>,
) -> BackupPreview {
    let mut described = preview(backup, current);
    let mice = context.mice.mice();
    let aliases = system_names(context.machine.refresh().await, |snapshot| {
        snapshot
            .system_aliases
            .iter()
            .map(|a| a.name.clone())
            .collect()
    });
    let hosts = system_names(context.ssh.refresh().await, |snapshot| {
        snapshot
            .system_hosts
            .iter()
            .map(|h| h.alias.clone())
            .collect()
    });
    for section in &mut described.sections {
        section.present = match BackupSection::parse(&section.section) {
            Some(BackupSection::Device(serial)) => context.engines.get(&serial).is_some(),
            Some(BackupSection::Mouse(serial)) => mice.iter().any(|mouse| mouse.serial == serial),
            _ => true,
        };
        for item in &mut section.executables {
            item.takes_over = match item.kind {
                ExecutableKind::ShellAlias => aliases.contains(&item.name),
                ExecutableKind::SshHost => hosts.contains(&item.name),
                _ => false,
            };
        }
    }
    described
}

/// The names that `names` takes from a snapshot of the machine; none when it
/// could not be read (logged: a takeover would then go unflagged).
fn system_names<S, E: std::error::Error>(
    snapshot: Result<S, HubError<E>>,
    names: impl FnOnce(&S) -> BTreeSet<String>,
) -> BTreeSet<String> {
    match snapshot {
        Ok(snapshot) => names(&snapshot),
        Err(error) => {
            tracing::warn!(%error, "the files of the machine were not read for the preview");
            BTreeSet::new()
        }
    }
}
