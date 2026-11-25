//! Reading the whole configuration from the disk, as a [`Backup`] to write or
//! as the documents an import is compared with.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use hubergamof_protocol::{
    AppModulesConfig, GitConfig, MachineConfig, MouseConfig, RadialMenuConfig, SshConfig,
};
use serde::de::DeserializeOwned;

use super::backup::{Backup, BackupDocument, BackupSection};
use super::files::{io_error, read_config};
use super::{
    APP_MODULES_FILE, ConfigStore, ConfigStoreError, DEVICES_DIR, GIT_FILE, MACHINE_FILE,
    MOUSE_FILE, RADIAL_MENU_FILE, SSH_FILE, is_safe_serial,
};

/// The document of `path`: `None` when there is none, and also when it is not
/// readable as a `T` (it is set aside by the owner of the file when that one
/// loads it, and one broken file must not keep the rest from being saved).
fn read_document<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, ConfigStoreError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(path, error)),
    };
    match serde_json::from_slice(&bytes) {
        Ok(document) => Ok(Some(document)),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "unreadable document left out of the backup");
            Ok(None)
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
    /// of the application and the machine. A part that does not exist yet, or
    /// that cannot be read, is left out.
    ///
    /// # Errors
    ///
    /// An I/O error.
    pub fn collect_backup(
        &self,
        app_version: &str,
        created_at: u64,
    ) -> Result<Backup, ConfigStoreError> {
        let _guard = self.lock();
        let mut documents = BTreeMap::new();
        let mut images = BTreeMap::new();
        for serial in self.stored_serials()? {
            if let Some(config) = read_config(&self.config_path(&serial)?)? {
                let (exported, device_images) = self.exported_parts(&serial, &config)?;
                documents.insert(
                    BackupSection::Device(serial.clone()),
                    BackupDocument::Device(Box::new(exported)),
                );
                if !device_images.is_empty() {
                    images.insert(serial.clone(), device_images);
                }
            }
            let mouse = self.device_dir(&serial)?.join(MOUSE_FILE);
            if let Some(config) = read_document::<MouseConfig>(&mouse)? {
                documents.insert(
                    BackupSection::Mouse(serial),
                    BackupDocument::Mouse(Box::new(config)),
                );
            }
        }
        if let Some(menu) = read_document::<RadialMenuConfig>(&self.root.join(RADIAL_MENU_FILE))? {
            documents.insert(BackupSection::RadialMenu, BackupDocument::RadialMenu(menu));
        }
        if let Some(modules) = read_document::<AppModulesConfig>(&self.root.join(APP_MODULES_FILE))?
        {
            documents.insert(BackupSection::Modules, BackupDocument::Modules(modules));
        }
        if let Some(machine) = read_document::<MachineConfig>(&self.root.join(MACHINE_FILE))? {
            documents.insert(BackupSection::Machine, BackupDocument::Machine(machine));
        }
        if let Some(git) = read_document::<GitConfig>(&self.root.join(GIT_FILE))? {
            documents.insert(BackupSection::Git, BackupDocument::Git(Box::new(git)));
        }
        if let Some(ssh) = read_document::<SshConfig>(&self.root.join(SSH_FILE))? {
            documents.insert(BackupSection::Ssh, BackupDocument::Ssh(ssh));
        }
        Ok(Backup::new(app_version, created_at, documents, images))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use hubergamof_protocol::MouseModel;

    use super::*;
    use crate::config_store::backup::{SectionChange, preview, read_backup, write_backup};
    use crate::device::virtual_deck::{DEMO_SERIAL, virtual_plus_info};
    use crate::testing::builtin_registry;

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path(), Arc::new(builtin_registry()));
        (dir, store)
    }

    #[test]
    fn an_empty_store_gives_an_empty_backup() {
        let (_dir, store) = store();
        let backup = store.collect_backup("1.4.2", 1).unwrap();
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

        let collected = store.collect_backup("1.4.2", 7).unwrap();
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
    fn a_broken_document_is_left_out_without_stopping_the_backup() {
        let (dir, store) = store();
        store.load_radial_menu().unwrap();
        fs::write(dir.path().join(SSH_FILE), b"not json").unwrap();
        let backup = store.collect_backup("1.4.2", 1).unwrap();
        assert!(backup.documents.contains_key(&BackupSection::RadialMenu));
        assert!(!backup.documents.contains_key(&BackupSection::Ssh));
    }

    #[test]
    fn folders_that_are_not_devices_are_ignored() {
        let (dir, store) = store();
        fs::create_dir_all(dir.path().join(DEVICES_DIR).join("bad name")).unwrap();
        fs::write(dir.path().join(DEVICES_DIR).join("a-file"), b"x").unwrap();
        let backup = store.collect_backup("1.4.2", 1).unwrap();
        assert!(backup.documents.is_empty());
    }
}
