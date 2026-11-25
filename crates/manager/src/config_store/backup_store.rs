//! Reading the whole configuration from the disk, as a [`Backup`] to write or
//! as the documents an import is compared with.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

use hubergamof_protocol::{
    AppModulesConfig, GitConfig, MachineConfig, MouseConfig, RadialMenuConfig, SshConfig,
};
use serde::de::DeserializeOwned;

use super::archive::MAX_ARCHIVE_CONFIG_BYTES;
use super::backup::{Backup, BackupDocument, BackupSection};
use super::files::{io_error, read_config};
use super::{
    APP_MODULES_FILE, ConfigStore, ConfigStoreError, DEVICES_DIR, GIT_FILE, MACHINE_FILE,
    MOUSE_FILE, RADIAL_MENU_FILE, SSH_FILE, is_safe_serial,
};

/// What reading a document found.
enum Found<T> {
    /// No such file.
    Missing,
    /// A file that is not a `T`, or not a file to read.
    Unreadable,
    /// The document.
    Document(T),
}

/// The bytes of a file read up to a limit.
pub(super) enum RawFile {
    /// No such file.
    Missing,
    /// A link, a named pipe or a folder in its place, or more than the limit.
    Unreadable,
    /// What the file holds.
    Bytes(Vec<u8>),
}

/// Reads at most `limit` bytes of the regular file `path`.
///
/// Opened without blocking and without following a link: a named pipe or a
/// huge file in its place must not hold the store.
pub(super) fn read_regular_file(path: &Path, limit: usize) -> Result<RawFile, ConfigStoreError> {
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(RawFile::Missing),
        // A link in the place of the file (`O_NOFOLLOW`) is not one to read.
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => return Ok(RawFile::Unreadable),
        Err(error) => return Err(io_error(path, error)),
    };
    let is_file = file.metadata().is_ok_and(|metadata| metadata.is_file());
    if !is_file {
        return Ok(RawFile::Unreadable);
    }
    let mut bytes = Vec::new();
    let cap = u64::try_from(limit).map_or(u64::MAX, |limit| limit.saturating_add(1));
    file.take(cap)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(path, error))?;
    if bytes.len() > limit {
        return Ok(RawFile::Unreadable);
    }
    Ok(RawFile::Bytes(bytes))
}

/// The document of `path`. A file that cannot be read as a `T` is reported,
/// not an error: one broken file must not keep the rest from being saved.
fn read_document<T: DeserializeOwned>(path: &Path) -> Result<Found<T>, ConfigStoreError> {
    Ok(match read_regular_file(path, MAX_ARCHIVE_CONFIG_BYTES)? {
        RawFile::Missing => Found::Missing,
        RawFile::Unreadable => Found::Unreadable,
        RawFile::Bytes(bytes) => {
            serde_json::from_slice(&bytes).map_or(Found::Unreadable, Found::Document)
        }
    })
}

/// What was read of the disk: the backup, and the parts left out of it.
#[derive(Debug)]
pub struct CollectedBackup {
    /// What could be saved: every document that is readable and valid.
    pub backup: Backup,
    /// The sections left out (see [`BackupSection::id`]) because their file
    /// could not be read as what it should be, or would not pass the checks
    /// of a backup: saved, the backup could not be restored.
    pub skipped: Vec<String>,
}

/// Documents being collected, and the sections that did not pass.
#[derive(Default)]
struct Collection {
    documents: BTreeMap<BackupSection, BackupDocument>,
    skipped: Vec<String>,
}

impl Collection {
    /// Keeps `document` when it passes the checks of a backup, else leaves
    /// its section out.
    fn keep(&mut self, section: BackupSection, document: BackupDocument) -> bool {
        match document.validate(&section) {
            Ok(()) => {
                self.documents.insert(section, document);
                true
            }
            Err(error) => {
                tracing::warn!(section = %section.id(), %error, "document left out of the backup");
                self.skipped.push(section.id());
                false
            }
        }
    }

    /// Keeps what `found` holds, or notes that the section is left out.
    fn found<T>(
        &mut self,
        section: BackupSection,
        found: Found<T>,
        wrap: impl FnOnce(T) -> BackupDocument,
    ) {
        match found {
            Found::Missing => {}
            Found::Unreadable => {
                tracing::warn!(section = %section.id(), "unreadable document left out of the backup");
                self.skipped.push(section.id());
            }
            Found::Document(document) => {
                self.keep(section, wrap(document));
            }
        }
    }
}

impl ConfigStore {
    /// The serials of the devices that have a folder, in order.
    fn stored_serials(&self) -> Result<Vec<String>, ConfigStoreError> {
        let dir = self.root.join(DEVICES_DIR);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io_error(&dir, error)),
        };
        let mut serials = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| io_error(&dir, error))?;
            let is_folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
            if let Some(name) = entry
                .file_name()
                .to_str()
                .filter(|name| is_safe_serial(name))
                && is_folder
            {
                serials.push(name.to_owned());
            }
        }
        serials.sort();
        Ok(serials)
    }

    /// The whole configuration on the disk: every device (with the images it
    /// uses) and mouse that has a folder, then the radial menu, the modules
    /// of the application and the machine. A part that does not exist yet is
    /// not in it; one that cannot be read, or that a backup would refuse, is
    /// left out and named in [`CollectedBackup::skipped`].
    ///
    /// # Errors
    ///
    /// An I/O error.
    pub fn collect_backup(
        &self,
        app_version: &str,
        created_at: u64,
    ) -> Result<CollectedBackup, ConfigStoreError> {
        let _guard = self.lock();
        let mut collection = Collection::default();
        let mut images = BTreeMap::new();
        for serial in self.stored_serials()? {
            self.collect_device(&serial, &mut collection, &mut images)?;
            let mouse = self.device_dir(&serial)?.join(MOUSE_FILE);
            collection.found(
                BackupSection::Mouse(serial),
                read_document::<MouseConfig>(&mouse)?,
                |config| BackupDocument::Mouse(Box::new(config)),
            );
        }
        let root = &self.root;
        collection.found(
            BackupSection::RadialMenu,
            read_document::<RadialMenuConfig>(&root.join(RADIAL_MENU_FILE))?,
            BackupDocument::RadialMenu,
        );
        collection.found(
            BackupSection::Modules,
            read_document::<AppModulesConfig>(&root.join(APP_MODULES_FILE))?,
            BackupDocument::Modules,
        );
        collection.found(
            BackupSection::Machine,
            read_document::<MachineConfig>(&root.join(MACHINE_FILE))?,
            BackupDocument::Machine,
        );
        collection.found(
            BackupSection::Git,
            read_document::<GitConfig>(&root.join(GIT_FILE))?,
            |config| BackupDocument::Git(Box::new(config)),
        );
        collection.found(
            BackupSection::Ssh,
            read_document::<SshConfig>(&root.join(SSH_FILE))?,
            BackupDocument::Ssh,
        );
        Ok(CollectedBackup {
            backup: Backup::new(app_version, created_at, collection.documents, images),
            skipped: collection.skipped,
        })
    }

    /// Adds the configuration of a device, and the images it uses, to what is collected.
    fn collect_device(
        &self,
        serial: &str,
        collection: &mut Collection,
        images: &mut BTreeMap<String, super::ArchiveImages>,
    ) -> Result<(), ConfigStoreError> {
        let config = match read_config(&self.config_path(serial)?) {
            Ok(Some(config)) => config,
            Ok(None) => return Ok(()),
            Err(ConfigStoreError::Json { .. }) => {
                tracing::warn!(%serial, "unreadable configuration left out of the backup");
                collection
                    .skipped
                    .push(BackupSection::Device(serial.to_owned()).id());
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let (exported, device_images) = self.exported_parts(serial, &config)?;
        let section = BackupSection::Device(serial.to_owned());
        if collection.keep(section, BackupDocument::Device(Box::new(exported)))
            && !device_images.is_empty()
        {
            images.insert(serial.to_owned(), device_images);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use hubergamof_protocol::MouseModel;

    use super::*;
    use crate::config_store::backup::{preview, read_backup, write_backup};
    use crate::device::virtual_deck::{DEMO_SERIAL, virtual_plus_info};
    use crate::testing::builtin_registry;
    use hubergamof_protocol::SectionChange;

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path(), Arc::new(builtin_registry()));
        (dir, store)
    }

    #[test]
    fn an_empty_store_gives_an_empty_backup() {
        let (_dir, store) = store();
        let backup = store.collect_backup("1.4.2", 1).unwrap().backup;
        assert!(backup.documents.is_empty());
        assert!(backup.images.is_empty());
    }

    #[test]
    fn everything_on_the_disk_is_collected_and_survives_a_backup() {
        let (_dir, store) = store();
        store.load(&virtual_plus_info(DEMO_SERIAL)).unwrap();
        store.load_mouse("MX4-0042", MouseModel::MxMaster4).unwrap();
        store.load_radial_menu().unwrap();
        store.load_app_modules().unwrap();

        let collected = store.collect_backup("1.4.2", 7).unwrap().backup;
        for section in [
            BackupSection::Device(DEMO_SERIAL.to_owned()),
            BackupSection::Mouse("MX4-0042".to_owned()),
            BackupSection::RadialMenu,
            BackupSection::Modules,
        ] {
            assert!(collected.documents.contains_key(&section), "{section:?}");
        }

        let read = read_backup(&write_backup(&collected).unwrap()).unwrap();
        assert_eq!(read, collected);
        // Compared with itself, nothing would change.
        let current = collected.documents.clone();
        assert!(
            preview(&read, &current)
                .sections
                .iter()
                .all(|section| section.change == SectionChange::Same)
        );
    }

    #[test]
    fn a_broken_document_is_left_out_and_named() {
        let (dir, store) = store();
        store.load_radial_menu().unwrap();
        fs::write(dir.path().join(SSH_FILE), b"not json").unwrap();
        let collected = store.collect_backup("1.4.2", 1).unwrap();
        assert!(
            collected
                .backup
                .documents
                .contains_key(&BackupSection::RadialMenu)
        );
        assert!(!collected.backup.documents.contains_key(&BackupSection::Ssh));
        assert_eq!(collected.skipped, vec!["ssh".to_owned()]);
    }

    #[test]
    fn a_readable_document_that_a_backup_would_refuse_is_left_out_too() {
        let (dir, store) = store();
        // Readable as an SSH configuration, not acceptable: the backup would be refused whole.
        let mut ssh = SshConfig::empty();
        ssh.hosts.push(hubergamof_protocol::SshHost {
            alias: "bad alias with spaces".into(),
            host_name: String::new(),
            user: String::new(),
            port: None,
            identity_file: String::new(),
            proxy_jump: String::new(),
            forward_agent: false,
            options: Vec::new(),
            enabled: true,
        });
        fs::write(dir.path().join(SSH_FILE), serde_json::to_vec(&ssh).unwrap()).unwrap();
        let collected = store.collect_backup("1.4.2", 1).unwrap();
        assert_eq!(collected.skipped, vec!["ssh".to_owned()]);
        // What is collected always reads back.
        read_backup(&write_backup(&collected.backup).unwrap()).unwrap();
    }

    #[test]
    fn an_unreadable_device_configuration_does_not_stop_the_export() {
        let (dir, store) = store();
        store.load(&virtual_plus_info(DEMO_SERIAL)).unwrap();
        fs::write(
            dir.path()
                .join(DEVICES_DIR)
                .join(DEMO_SERIAL)
                .join(super::super::CONFIG_FILE),
            b"{broken",
        )
        .unwrap();
        let collected = store.collect_backup("1.4.2", 1).unwrap();
        assert!(collected.backup.documents.is_empty());
        assert_eq!(collected.skipped, vec![format!("devices/{DEMO_SERIAL}")]);
    }

    #[test]
    fn a_link_or_a_pipe_in_place_of_a_document_is_left_out_without_waiting() {
        use std::os::unix::fs::symlink;
        let (dir, store) = store();
        let target = dir.path().join("elsewhere.json");
        fs::write(
            &target,
            serde_json::to_vec(&hubergamof_protocol::RadialMenuConfig::default_menu()).unwrap(),
        )
        .unwrap();
        symlink(&target, dir.path().join(RADIAL_MENU_FILE)).unwrap();
        let fifo = dir.path().join(MACHINE_FILE);
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        let collected = store.collect_backup("1.4.2", 1).unwrap();
        assert!(collected.backup.documents.is_empty());
        let mut skipped = collected.skipped;
        skipped.sort();
        assert_eq!(
            skipped,
            vec!["machine".to_owned(), "radial-menu".to_owned()]
        );
    }

    #[test]
    fn folders_that_are_not_devices_are_ignored() {
        let (dir, store) = store();
        fs::create_dir_all(dir.path().join(DEVICES_DIR).join("bad name")).unwrap();
        fs::write(dir.path().join(DEVICES_DIR).join("a-file"), b"x").unwrap();
        let backup = store.collect_backup("1.4.2", 1).unwrap().backup;
        assert!(backup.documents.is_empty());
    }
}
