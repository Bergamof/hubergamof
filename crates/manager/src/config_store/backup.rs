//! Backup of the whole configuration of the application: one zip holding a
//! manifest, one document per part of the configuration (every device, every
//! mouse, the radial menu, the modules of the application, the machine: shell
//! aliases, Git and SSH) and the images of the devices, all pretty-printed
//! JSON so that they stay readable with ordinary tools.
//!
//! ```text
//! manifest.json                      format, version of the application, date, parts
//! config/devices/<serial>.json       DeviceConfig
//! config/mice/<serial>.json          MouseConfig
//! config/radial-menu.json            RadialMenuConfig
//! config/modules.json                AppModulesConfig
//! config/machine.json                MachineConfig (shell aliases)
//! config/git.json                    GitConfig
//! config/ssh.json                    SshConfig
//! images/<serial>/<name>             images of a device
//! ```
//!
//! The update settings are not part of it: whether the application installs
//! its updates by itself is a decision about this computer, not something a
//! file should be able to turn on.
//!
//! A backup comes from the user, possibly from elsewhere, and the machine
//! documents end up running code (aliases, SSH options): reading accepts
//! nothing but the entries above, checks every document against its type, and
//! bounds the number of entries and every decompressed size as it reads (see
//! [`super::archive`]). Nothing is written here: [`preview`] says what an
//! import would do, so that the person decides first.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Cursor, Write};

use hubergamof_protocol::{
    AppModulesConfig, BackupPreview, DeviceConfig, ExecutableItem, ExecutableKind, GitConfig,
    MachineConfig, MouseConfig, RadialMenuConfig, SectionChange, SectionPreview, SshConfig,
    SshHost, is_safe_image_name,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::archive::{
    ArchiveError, ArchiveImages, Limits, MAX_ARCHIVE_BYTES, MAX_ARCHIVE_CONFIG_BYTES,
    MAX_ARCHIVE_EXPANDED_BYTES, read_entries,
};
use super::is_safe_serial;
use crate::render::{ImageKind, MAX_SOURCE_BYTES};

/// Entry of the manifest.
pub const BACKUP_MANIFEST_ENTRY: &str = "manifest.json";

/// Format written, and the newest one that is read.
pub const BACKUP_FORMAT: u32 = 2;

/// Directory of the documents.
const CONFIG_PREFIX: &str = "config/";

/// Directory of the images.
const IMAGES_PREFIX: &str = "images/";

/// Extension of a document.
const DOCUMENT_EXTENSION: &str = ".json";

/// Most entries a backup may hold: a device has a document, a mouse has one,
/// and a device brings its images.
pub const MAX_BACKUP_ENTRIES: usize = 1024;

/// Longest version written in a manifest.
const MAX_VERSION_LEN: usize = 64;

/// Largest decompressed manifest.
const MAX_MANIFEST_BYTES: usize = 64 * 1024;

/// The limits of a backup: as large as an export of a device, with more entries.
const BACKUP_LIMITS: Limits = Limits {
    archive_bytes: MAX_ARCHIVE_BYTES,
    entries: MAX_BACKUP_ENTRIES,
    expanded_bytes: MAX_ARCHIVE_EXPANDED_BYTES,
};

/// A part of the configuration, which a backup holds and an import replaces.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BackupSection {
    /// The configuration of a Stream Deck, with its images.
    Device(String),
    /// The configuration of a mouse.
    Mouse(String),
    /// The radial menu, shared by the mice.
    RadialMenu,
    /// The modules that live in the application.
    Modules,
    /// The shell aliases.
    Machine,
    /// The Git configuration.
    Git,
    /// The SSH hosts.
    Ssh,
}

impl BackupSection {
    /// Name of the section in the manifest and in the interface.
    #[must_use]
    pub fn id(&self) -> String {
        match self {
            Self::Device(serial) => format!("devices/{serial}"),
            Self::Mouse(serial) => format!("mice/{serial}"),
            Self::RadialMenu => "radial-menu".to_owned(),
            Self::Modules => "modules".to_owned(),
            Self::Machine => "machine".to_owned(),
            Self::Git => "git".to_owned(),
            Self::Ssh => "ssh".to_owned(),
        }
    }

    /// The section a name designates; `None` for a name that is not one, or
    /// whose serial is not a safe directory name.
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        let section = match id {
            "radial-menu" => Self::RadialMenu,
            "modules" => Self::Modules,
            "machine" => Self::Machine,
            "git" => Self::Git,
            "ssh" => Self::Ssh,
            other => match other.split_once('/')? {
                ("devices", serial) => Self::Device(serial.to_owned()),
                ("mice", serial) => Self::Mouse(serial.to_owned()),
                _ => return None,
            },
        };
        match &section {
            Self::Device(serial) | Self::Mouse(serial) if !is_safe_serial(serial) => None,
            _ => Some(section),
        }
    }

    /// Name of the entry of its document.
    fn entry_name(&self) -> String {
        format!("{CONFIG_PREFIX}{}{DOCUMENT_EXTENSION}", self.id())
    }

    /// The section whose document an entry is.
    fn from_entry_name(name: &str) -> Option<Self> {
        let id = name
            .strip_prefix(CONFIG_PREFIX)?
            .strip_suffix(DOCUMENT_EXTENSION)?;
        Self::parse(id)
    }
}

/// A document of a backup, checked against its type.
#[derive(Debug, Clone, PartialEq)]
pub enum BackupDocument {
    /// The configuration of a Stream Deck.
    Device(Box<DeviceConfig>),
    /// The configuration of a mouse.
    Mouse(Box<MouseConfig>),
    /// The radial menu.
    RadialMenu(RadialMenuConfig),
    /// The modules of the application.
    Modules(AppModulesConfig),
    /// The shell aliases.
    Machine(MachineConfig),
    /// The Git configuration.
    Git(Box<GitConfig>),
    /// The SSH hosts.
    Ssh(SshConfig),
}

impl BackupDocument {
    /// The document of `section` that `bytes` hold.
    fn parse(section: &BackupSection, bytes: &[u8]) -> Result<Self, BackupError> {
        let invalid = |source| BackupError::Document {
            section: section.id(),
            source,
        };
        let document = match section {
            BackupSection::Device(_) => Self::Device(Box::new(
                super::migration::parse_config(bytes).map_err(invalid)?,
            )),
            BackupSection::Mouse(_) => {
                Self::Mouse(Box::new(serde_json::from_slice(bytes).map_err(invalid)?))
            }
            BackupSection::RadialMenu => {
                Self::RadialMenu(serde_json::from_slice(bytes).map_err(invalid)?)
            }
            BackupSection::Modules => {
                Self::Modules(serde_json::from_slice(bytes).map_err(invalid)?)
            }
            BackupSection::Machine => {
                Self::Machine(serde_json::from_slice(bytes).map_err(invalid)?)
            }
            BackupSection::Git => {
                Self::Git(Box::new(serde_json::from_slice(bytes).map_err(invalid)?))
            }
            BackupSection::Ssh => Self::Ssh(serde_json::from_slice(bytes).map_err(invalid)?),
        };
        document.validate(section)?;
        Ok(document)
    }

    /// Checks what can be checked without the hardware: the checks of the
    /// protocol, and that a document speaks of its own section.
    pub(super) fn validate(&self, section: &BackupSection) -> Result<(), BackupError> {
        let refused = |reason: String| BackupError::Refused {
            section: section.id(),
            reason,
        };
        match (self, section) {
            (Self::Device(config), BackupSection::Device(serial)) => {
                if &config.serial != serial {
                    return Err(refused("the document is of another device".to_owned()));
                }
                config.validate().map_err(|e| refused(e.to_string()))
            }
            (Self::Mouse(config), BackupSection::Mouse(serial)) => {
                if &config.serial != serial {
                    return Err(refused("the document is of another mouse".to_owned()));
                }
                config.validate().map_err(|e| refused(e.to_string()))
            }
            (Self::RadialMenu(config), _) => config.validate().map_err(|e| refused(e.to_string())),
            (Self::Modules(config), _) => config.validate().map_err(|e| refused(e.to_string())),
            (Self::Machine(config), _) => config.validate().map_err(|e| refused(e.to_string())),
            (Self::Git(config), _) => config.validate().map_err(|e| refused(e.to_string())),
            (Self::Ssh(config), _) => config.validate().map_err(|e| refused(e.to_string())),
            _ => Err(refused("the document is not of this section".to_owned())),
        }
    }

    /// The document as JSON, pretty-printed.
    fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        match self {
            Self::Device(config) => serde_json::to_vec_pretty(config),
            Self::Mouse(config) => serde_json::to_vec_pretty(config),
            Self::RadialMenu(config) => serde_json::to_vec_pretty(config),
            Self::Modules(config) => serde_json::to_vec_pretty(config),
            Self::Machine(config) => serde_json::to_vec_pretty(config),
            Self::Git(config) => serde_json::to_vec_pretty(config),
            Self::Ssh(config) => serde_json::to_vec_pretty(config),
        }
    }

    /// The document as it is compared: without its revision, which counts
    /// the saves of this computer and says nothing of the content.
    fn comparable(&self) -> serde_json::Value {
        let mut value = match self.to_json() {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
            Err(_) => serde_json::Value::Null,
        };
        if let Some(object) = value.as_object_mut() {
            object.remove("revision");
        }
        value
    }

    /// What the document would have this computer run or reach once applied:
    /// shell aliases, Git aliases and free settings, the SSH hosts that name
    /// a machine, forward the agent or go through another one, and the free
    /// options of SSH. Every entry is listed, with whether it is enabled: a
    /// disabled one is one click from being run.
    fn executables(&self) -> Vec<ExecutableItem> {
        match self {
            Self::Machine(config) => config
                .aliases
                .iter()
                .map(|alias| {
                    executable(
                        ExecutableKind::ShellAlias,
                        &alias.name,
                        &alias.command,
                        alias.enabled,
                    )
                })
                .collect(),
            Self::Git(config) => {
                let aliases = config.aliases.iter().map(|alias| {
                    executable(
                        ExecutableKind::GitAlias,
                        &alias.name,
                        &alias.command,
                        alias.enabled,
                    )
                });
                // Any key: `core.fsmonitor`, `core.hooksPath`, `credential.<url>.helper`,
                // `filter.*`, `diff.*` run something without an alias being typed, and a list
                // of the dangerous ones would always miss one.
                let settings = config.extra_settings.iter().map(|setting| {
                    executable(
                        ExecutableKind::GitSetting,
                        &setting.key,
                        &setting.value,
                        setting.enabled,
                    )
                });
                aliases.chain(settings).collect()
            }
            Self::Ssh(config) => {
                let common = config
                    .common_options
                    .iter()
                    .map(|option| ("*", option, true));
                let host_options = config.hosts.iter().flat_map(|host| {
                    host.options
                        .iter()
                        .map(|option| (host.alias.as_str(), option, host.enabled))
                });
                let options = common.chain(host_options).map(|(host, option, host_on)| {
                    executable(
                        ExecutableKind::SshOption,
                        &format!("{host}: {}", option.key),
                        &option.value,
                        option.enabled && host_on,
                    )
                });
                // A host that names a machine, forwards the agent or goes through another one
                // takes the place of what the user typed after `ssh`, and may reach the keys.
                let hosts = config
                    .hosts
                    .iter()
                    .filter(|host| {
                        !host.host_name.is_empty()
                            || host.forward_agent
                            || !host.proxy_jump.is_empty()
                    })
                    .map(|host| {
                        executable(
                            ExecutableKind::SshHost,
                            &host.alias,
                            &ssh_host_detail(host),
                            host.enabled,
                        )
                    });
                hosts.chain(options).collect()
            }
            _ => Vec::new(),
        }
    }
}

/// What an SSH host leads to: `machine`, `via jumps`, `agent forwarded`.
fn ssh_host_detail(host: &SshHost) -> String {
    let mut parts = Vec::new();
    if !host.host_name.is_empty() {
        parts.push(host.host_name.clone());
    }
    if !host.proxy_jump.is_empty() {
        parts.push(format!("via {}", host.proxy_jump));
    }
    if host.forward_agent {
        parts.push("agent forwarded".to_owned());
    }
    parts.join(", ")
}

/// What the manifest says of a backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupManifest {
    /// Format of the archive, [`BACKUP_FORMAT`] when written.
    pub format: u32,
    /// Version of the application that wrote it.
    pub app_version: String,
    /// When it was written, in seconds since the Unix epoch.
    pub created_at: u64,
    /// The sections it holds, by [`BackupSection::id`].
    pub sections: Vec<String>,
}

/// A backup: its documents, and the images of its devices.
#[derive(Debug, Clone, PartialEq)]
pub struct Backup {
    /// What the manifest says of the backup.
    pub manifest: BackupManifest,
    /// The documents, by section.
    pub documents: BTreeMap<BackupSection, BackupDocument>,
    /// Images by serial of the device, then by file name (not checked as
    /// images yet).
    pub images: BTreeMap<String, ArchiveImages>,
}

impl Backup {
    /// A backup of `documents` and `images`, written by `app_version` at
    /// `created_at`.
    #[must_use]
    pub fn new(
        app_version: &str,
        created_at: u64,
        documents: BTreeMap<BackupSection, BackupDocument>,
        images: BTreeMap<String, ArchiveImages>,
    ) -> Self {
        let sections = documents.keys().map(BackupSection::id).collect();
        Self {
            manifest: BackupManifest {
                format: BACKUP_FORMAT,
                app_version: app_version.to_owned(),
                created_at,
                sections,
            },
            documents,
            images,
        }
    }
}

impl Backup {
    /// The part of the backup that holds `sections` (and the images of its
    /// devices), written by the same version at the same time.
    #[must_use]
    pub fn select(&self, sections: &BTreeSet<BackupSection>) -> Self {
        let documents: BTreeMap<_, _> = self
            .documents
            .iter()
            .filter(|(section, _)| sections.contains(section))
            .map(|(section, document)| (section.clone(), document.clone()))
            .collect();
        let images = self
            .images
            .iter()
            .filter(|(serial, _)| documents.contains_key(&BackupSection::Device((*serial).clone())))
            .map(|(serial, images)| (serial.clone(), images.clone()))
            .collect();
        Self::new(
            &self.manifest.app_version,
            self.manifest.created_at,
            documents,
            images,
        )
    }
}

/// Why a backup cannot be read or written.
#[derive(Debug, Error)]
pub enum BackupError {
    /// The zip itself (size, entries, names, ends) is refused.
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    /// No `manifest.json`.
    #[error("backup has no {BACKUP_MANIFEST_ENTRY}")]
    MissingManifest,
    /// `manifest.json` is not a manifest.
    #[error("invalid {BACKUP_MANIFEST_ENTRY}: {0}")]
    Manifest(#[source] serde_json::Error),
    /// The version of the manifest is not a version.
    #[error("the version in the manifest is not a version")]
    InvalidVersion,
    /// Written by a newer version of the application.
    #[error("backup of format {0}, written by a newer version of the application")]
    NewerFormat(u32),
    /// Written in a format that this application does not know.
    #[error("backup of unknown format {0}")]
    UnknownFormat(u32),
    /// The manifest and the entries do not list the same sections.
    #[error("the manifest and the entries of the backup do not match")]
    ManifestMismatch,
    /// A document is not of the type of its section.
    #[error("invalid document of {section}: {source}")]
    Document {
        /// Name of the section whose document is not of its type.
        section: String,
        #[source]
        /// What the document is not.
        source: serde_json::Error,
    },
    /// A document is of its type but not acceptable.
    #[error("document of {section} refused: {reason}")]
    Refused {
        /// Name of the section whose document is refused.
        section: String,
        /// Why.
        reason: String,
    },
    /// Images of a device the backup does not hold.
    #[error("images of a device that the backup does not hold")]
    OrphanImages,
    /// Writing an entry failed.
    #[error("backup entry: {0}")]
    Io(#[from] io::Error),
    /// A document could not be written as JSON.
    #[error("backup document: {0}")]
    Json(#[from] serde_json::Error),
    /// Not writable as a zip.
    #[error("backup archive: {0}")]
    Zip(#[from] zip::result::ZipError),
}

/// Writes `backup`.
///
/// # Errors
///
/// [`BackupError::Json`], [`BackupError::Zip`] or [`BackupError::Io`] (not
/// expected in memory).
pub fn write_backup(backup: &Backup) -> Result<Vec<u8>, BackupError> {
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    // PNG and JPEG are compressed already: deflating them costs time for nothing.
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(BACKUP_MANIFEST_ENTRY, deflated)?;
    zip.write_all(&serde_json::to_vec_pretty(&backup.manifest)?)?;
    for (section, document) in &backup.documents {
        zip.start_file(section.entry_name(), deflated)?;
        zip.write_all(&document.to_json()?)?;
    }
    for (serial, images) in &backup.images {
        for (name, bytes) in images {
            let options = match ImageKind::sniff(bytes) {
                Some(ImageKind::Png | ImageKind::Jpeg) => stored,
                Some(ImageKind::Svg) | None => deflated,
            };
            zip.start_file(format!("{IMAGES_PREFIX}{serial}/{name}"), options)?;
            zip.write_all(bytes)?;
        }
    }
    Ok(zip.finish()?.into_inner())
}

/// What an entry of a backup is.
enum Entry {
    Manifest,
    Document(BackupSection),
    Image { serial: String, name: String },
    Directory,
}

fn classify(name: &str) -> Result<Entry, ArchiveError> {
    let unexpected = || ArchiveError::UnexpectedEntry(name.to_owned());
    if name == BACKUP_MANIFEST_ENTRY {
        return Ok(Entry::Manifest);
    }
    if name.ends_with('/') {
        let folder = name.trim_end_matches('/');
        let known = [
            CONFIG_PREFIX.trim_end_matches('/'),
            "config/devices",
            "config/mice",
            IMAGES_PREFIX.trim_end_matches('/'),
        ];
        let serial_folder = folder
            .strip_prefix(IMAGES_PREFIX)
            .is_some_and(is_safe_serial);
        return if known.contains(&folder) || serial_folder {
            Ok(Entry::Directory)
        } else {
            Err(unexpected())
        };
    }
    if let Some(section) = BackupSection::from_entry_name(name) {
        return Ok(Entry::Document(section));
    }
    let (serial, image) = name
        .strip_prefix(IMAGES_PREFIX)
        .and_then(|rest| rest.split_once('/'))
        .ok_or_else(unexpected)?;
    if is_safe_serial(serial) && is_safe_image_name(image) {
        Ok(Entry::Image {
            serial: serial.to_owned(),
            name: image.to_owned(),
        })
    } else {
        Err(unexpected())
    }
}

/// Reads a backup.
///
/// # Errors
///
/// Any [`BackupError`]: the backup is refused as a whole.
pub fn read_backup(bytes: &[u8]) -> Result<Backup, BackupError> {
    let entries = read_entries(bytes, BACKUP_LIMITS, |name| {
        Ok(match classify(name)? {
            Entry::Directory => None,
            Entry::Manifest => Some(MAX_MANIFEST_BYTES),
            Entry::Document(_) => Some(MAX_ARCHIVE_CONFIG_BYTES),
            Entry::Image { .. } => Some(MAX_SOURCE_BYTES),
        })
    })?;
    let mut manifest = None;
    let mut documents = BTreeMap::new();
    let mut images: BTreeMap<String, ArchiveImages> = BTreeMap::new();
    for (name, content) in entries {
        match classify(&name)? {
            Entry::Manifest => {
                manifest = Some(parse_manifest(&content)?);
            }
            Entry::Document(section) => {
                let document = BackupDocument::parse(&section, &content)?;
                documents.insert(section, document);
            }
            Entry::Image { serial, name } => {
                images.entry(serial).or_default().insert(name, content);
            }
            Entry::Directory => {}
        }
    }
    let manifest = manifest.ok_or(BackupError::MissingManifest)?;
    let listed: BTreeSet<&str> = manifest.sections.iter().map(String::as_str).collect();
    let held: Vec<String> = documents.keys().map(BackupSection::id).collect();
    if listed.len() != manifest.sections.len()
        || listed != held.iter().map(String::as_str).collect::<BTreeSet<_>>()
    {
        return Err(BackupError::ManifestMismatch);
    }
    if images
        .keys()
        .any(|serial| !documents.contains_key(&BackupSection::Device(serial.clone())))
    {
        return Err(BackupError::OrphanImages);
    }
    Ok(Backup {
        manifest,
        documents,
        images,
    })
}

/// A version as the application writes it: letters, digits and `.-+`, not
/// long. The manifest comes from the file, and its text will be shown and logged.
fn is_plain_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= MAX_VERSION_LEN
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
}

fn parse_manifest(bytes: &[u8]) -> Result<BackupManifest, BackupError> {
    let manifest: BackupManifest = serde_json::from_slice(bytes).map_err(BackupError::Manifest)?;
    if !is_plain_version(&manifest.app_version) {
        return Err(BackupError::InvalidVersion);
    }
    match manifest.format {
        BACKUP_FORMAT => Ok(manifest),
        newer if newer > BACKUP_FORMAT => Err(BackupError::NewerFormat(newer)),
        older => Err(BackupError::UnknownFormat(older)),
    }
}

/// An item of a document that runs something. Whether it takes over a name of
/// the computer is for the server to say: it alone knows the machine.
fn executable(kind: ExecutableKind, name: &str, detail: &str, enabled: bool) -> ExecutableItem {
    ExecutableItem {
        kind,
        name: name.to_owned(),
        detail: detail.to_owned(),
        enabled,
        takes_over: false,
    }
}

/// What importing `backup` would do to a computer whose configuration is
/// `current`. Nothing is written. Every section is reported present: whether
/// a device is connected is not known here.
#[must_use]
pub fn preview(
    backup: &Backup,
    current: &BTreeMap<BackupSection, BackupDocument>,
) -> BackupPreview {
    let sections = backup
        .documents
        .iter()
        .map(|(section, document)| SectionPreview {
            section: section.id(),
            change: match current.get(section) {
                None => SectionChange::Added,
                Some(now) if now.comparable() == document.comparable() => SectionChange::Same,
                Some(_) => SectionChange::Replaced,
            },
            present: true,
            executables: document.executables(),
        })
        .collect();
    let kept = current
        .keys()
        .filter(|section| !backup.documents.contains_key(section))
        .map(BackupSection::id)
        .collect();
    BackupPreview { sections, kept }
}

#[cfg(test)]
mod tests;
