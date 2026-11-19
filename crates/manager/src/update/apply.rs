//! What the update helper does, as root, with a download folder it does not
//! trust: verifies the release signature, then the package against it, and
//! only then hands the package it checked to the system's package manager.
//!
//! The folder belongs to the user who asked: every file is read once, from a
//! regular file, into memory; what is hashed is what is installed (copied
//! into a folder of its own), so nothing can be swapped in between.

use std::fs::{self, File};
use std::io::{self, Read as _, Write as _};
use std::os::fd::OwnedFd;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rustix::fs::{FileType, Mode, OFlags};
use rustix::io::Errno;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use super::package::{CHECKSUMS_FILE, PackageFormat, SIGNATURE_FILE, SystemPackages};
use super::signature::{ReleaseKey, SignatureError};
use super::version::Version;

/// Largest `SHA256SUMS` read: it lists two packages.
const MAX_CHECKSUMS_BYTES: u64 = 64 * 1024;

/// Largest signature read: an Ed25519 signature is 64 bytes.
const MAX_SIGNATURE_BYTES: u64 = 64;

/// Largest package read, into memory, as root: the real ones are around 15 MiB.
const MAX_PACKAGE_BYTES: u64 = 64 * 1024 * 1024;

/// Permissions of the folder holding the package: readable by the package
/// manager's unprivileged download user.
const STAGING_DIR_MODE: u32 = 0o755;
const STAGED_FILE_MODE: u32 = 0o644;

/// Mode of the staged package while it is written: the helper only.
const STAGED_FILE_WRITE_MODE: u32 = 0o600;

/// Owner a staging root must have (root).
pub const ROOT_UID: u32 = 0;

/// Length of a SHA-256 in hexadecimal.
const SHA256_HEX_LENGTH: usize = 64;

/// Why an update is refused or fails.
#[derive(Debug, Error)]
pub enum ApplyError {
    /// A file of the download folder cannot be read.
    #[error("cannot read {name}: {source}")]
    Read {
        /// File name.
        name: String,
        /// Cause.
        source: io::Error,
    },
    /// A file is not a plain file, or is larger than any release file.
    #[error("{0} is not an acceptable file")]
    Unacceptable(String),
    /// The signature of `SHA256SUMS` is not the release key's.
    #[error("{0}")]
    Signature(#[from] SignatureError),
    /// This system has no `hubergamof` package.
    #[error("hubergamof is not installed from a package")]
    NotPackaged,
    /// `SHA256SUMS` lists no package for this system, or several.
    #[error("the checksums list no single package for this system")]
    NoPackage,
    /// The package is not newer than the installed one (also refuses a replay
    /// of an older signed release).
    #[error("version {offered} is not newer than {installed}")]
    NotNewer {
        /// Version of the package offered.
        offered: Version,
        /// Version installed.
        installed: Version,
    },
    /// The package is not the one the signed checksums describe.
    #[error("the package does not match the signed checksums")]
    ChecksumMismatch,
    /// The staging folder is missing, or others could write in it.
    #[error("{0} is not a safe staging folder")]
    UnsafeStaging(String),
    /// The package could not be staged or installed.
    #[error("installation failed: {0}")]
    Install(String),
}

/// Installs a package file with the system's package manager.
pub trait PackageInstaller {
    /// Installs `package`.
    ///
    /// # Errors
    ///
    /// A description of the failure.
    fn install(&self, format: PackageFormat, package: &Path) -> Result<(), String>;
}

/// The real `apt-get` and `pacman`.
pub struct SystemInstaller;

impl PackageInstaller for SystemInstaller {
    fn install(&self, format: PackageFormat, package: &Path) -> Result<(), String> {
        let status = install_command(format, package)
            .stdin(Stdio::null())
            .status()
            .map_err(|error| format!("cannot run the package manager: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("the package manager failed ({status})"))
        }
    }
}

/// The package manager's command line: arguments apart (no shell), the file
/// last, after the options.
fn install_command(format: PackageFormat, package: &Path) -> Command {
    let mut command = match format {
        PackageFormat::Deb => {
            let mut command = Command::new("apt-get");
            // No new Recommends pulled in by an upgrade, existing configuration
            // files kept without asking, and a wait (not a failure) when
            // another package manager holds the lock.
            command.env("DEBIAN_FRONTEND", "noninteractive").args([
                "install",
                "--yes",
                "--no-install-recommends",
                "--option",
                "Dpkg::Options::=--force-confdef",
                "--option",
                "Dpkg::Options::=--force-confold",
                "--option",
                "DPkg::Lock::Timeout=120",
            ]);
            command
        }
        PackageFormat::Pacman => {
            let mut command = Command::new("pacman");
            command.args(["--upgrade", "--noconfirm"]);
            command
        }
    };
    command.arg(package);
    command
}

/// What an update needs from the system.
pub struct Environment<'a> {
    /// Key the release must be signed with.
    pub key: &'a ReleaseKey,
    /// Version of the installed helper (and so of the manager).
    pub installed: Version,
    /// Tells how `hubergamof` is installed.
    pub packages: &'a dyn SystemPackages,
    /// Installs the checked package.
    pub installer: &'a dyn PackageInstaller,
    /// Folder under which the checked package is staged: it must already
    /// exist (the package ships it), belong to `staging_owner` and be
    /// writable by nobody else.
    pub staging_root: &'a Path,
    /// Uid that must own `staging_root`: root, except in tests.
    pub staging_owner: u32,
}

/// Verifies and installs the release downloaded in `download_dir`; returns
/// the version installed.
///
/// # Errors
///
/// [`ApplyError`], without having installed anything, unless the package
/// manager itself fails.
pub fn apply(download_dir: &Path, environment: &Environment<'_>) -> Result<Version, ApplyError> {
    let directory = open_download_dir(download_dir)?;
    let checksums = read_plain_file(&directory, CHECKSUMS_FILE, MAX_CHECKSUMS_BYTES)?;
    let signature = read_plain_file(&directory, SIGNATURE_FILE, MAX_SIGNATURE_BYTES)?;
    environment.key.verify(&checksums, &signature)?;

    let format = environment
        .packages
        .installed_format()
        .ok_or(ApplyError::NotPackaged)?;
    let (name, version, expected) = pick_package(&checksums, format)?;
    if version <= environment.installed {
        return Err(ApplyError::NotNewer {
            offered: version,
            installed: environment.installed,
        });
    }

    let package = read_plain_file(&directory, &name, MAX_PACKAGE_BYTES)?;
    if hex(&Sha256::digest(&package)) != expected {
        return Err(ApplyError::ChecksumMismatch);
    }
    install_staged(environment, format, &name, &package)?;
    Ok(version)
}

/// The package of `format` listed by `checksums`: its file name, version and
/// SHA-256. Exactly one is expected (the release has one per format).
fn pick_package(
    checksums: &[u8],
    format: PackageFormat,
) -> Result<(String, Version, String), ApplyError> {
    let text = std::str::from_utf8(checksums).map_err(|_| ApplyError::NoPackage)?;
    let mut found = text.lines().filter_map(|line| {
        // `sha256sum` output: `<hash>  <name>` (or ` *<name>`, binary mode).
        let (hash, name) = line.split_once(' ')?;
        let name = name.trim_start_matches([' ', '*']);
        let version = format.version_of(name)?;
        let valid_hash = hash.len() == SHA256_HEX_LENGTH
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        valid_hash.then(|| (name.to_owned(), version, hash.to_owned()))
    });
    match (found.next(), found.next()) {
        (Some(package), None) => Ok(package),
        _ => Err(ApplyError::NoPackage),
    }
}

/// Writes the checked bytes to a folder of its own and installs them.
///
/// The folder is made by root inside a folder only root can write in, and the
/// file is created private (0600), so that nobody holds a way to write into
/// it between the check and the installation, whatever umask the caller
/// handed down.
fn install_staged(
    environment: &Environment<'_>,
    format: PackageFormat,
    name: &str,
    package: &[u8],
) -> Result<(), ApplyError> {
    let failed = |error: io::Error| ApplyError::Install(error.to_string());
    // `version_of` only lets plain file names through; this keeps the write
    // inside the staging folder even if that ever changes.
    if name.contains(['/', '\0']) {
        return Err(ApplyError::Unacceptable(name.to_owned()));
    }
    check_staging_root(environment.staging_root, environment.staging_owner)?;
    let staging = tempfile::Builder::new()
        .prefix("update-")
        .tempdir_in(environment.staging_root)
        .map_err(failed)?;
    fs::set_permissions(staging.path(), fs::Permissions::from_mode(STAGING_DIR_MODE))
        .map_err(failed)?;
    let path: PathBuf = staging.path().join(name);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(STAGED_FILE_WRITE_MODE)
        .open(&path)
        .map_err(failed)?;
    file.write_all(package).map_err(failed)?;
    // On the descriptor itself: the package manager's unprivileged download
    // user can then read the file, and no path is involved.
    file.set_permissions(fs::Permissions::from_mode(STAGED_FILE_MODE))
        .map_err(failed)?;
    drop(file);
    environment
        .installer
        .install(format, &path)
        .map_err(ApplyError::Install)
}

/// The staging root must be a real folder (not a link) of `owner`, closed to
/// everybody else: whoever could write in it could swap the staged package.
fn check_staging_root(root: &Path, owner: u32) -> Result<(), ApplyError> {
    let unsafe_root = || ApplyError::UnsafeStaging(root.display().to_string());
    let metadata = fs::symlink_metadata(root).map_err(|_| unsafe_root())?;
    if !metadata.is_dir() || metadata.uid() != owner || metadata.mode() & 0o022 != 0 {
        return Err(unsafe_root());
    }
    Ok(())
}

/// Opens the download folder once; every file is then read through this
/// descriptor, so the folder cannot be swapped for another afterwards. The
/// path must be absolute (`pkexec` starts in `/root`).
fn open_download_dir(dir: &Path) -> Result<OwnedFd, ApplyError> {
    let name = dir.display().to_string();
    if !dir.is_absolute() {
        return Err(ApplyError::Unacceptable(name));
    }
    rustix::fs::open(
        dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|errno| ApplyError::Read {
        name,
        source: errno.into(),
    })
}

/// Reads `name` from the download folder (at most `limit` bytes): a regular
/// file, opened without following a link and without blocking (a FIFO would
/// otherwise hold the helper), and checked on the descriptor actually opened.
fn read_plain_file(directory: &OwnedFd, name: &str, limit: u64) -> Result<Vec<u8>, ApplyError> {
    let read_error = |source: io::Error| ApplyError::Read {
        name: name.to_owned(),
        source,
    };
    let unacceptable = || ApplyError::Unacceptable(name.to_owned());
    let descriptor = rustix::fs::openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|errno| match errno {
        // A link, or a device node that refuses to open that way.
        Errno::LOOP | Errno::NXIO => unacceptable(),
        other => read_error(other.into()),
    })?;
    let status = rustix::fs::fstat(&descriptor).map_err(|errno| read_error(errno.into()))?;
    let is_file = FileType::from_raw_mode(status.st_mode) == FileType::RegularFile;
    if !is_file || u64::try_from(status.st_size).map_or(true, |size| size > limit) {
        return Err(unacceptable());
    }
    let mut bytes = Vec::new();
    File::from(descriptor)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    if bytes.len() as u64 > limit {
        return Err(unacceptable());
    }
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::os::unix::fs::symlink;

    use ed25519_dalek::Signer as _;
    use tempfile::TempDir;

    use super::*;
    use crate::update::signature::tests::test_keys;

    const INSTALLED: Version = Version::new(1, 0, 0);
    const OFFERED: Version = Version::new(1, 1, 0);

    fn owner_of(path: &Path) -> u32 {
        fs::metadata(path).unwrap().uid()
    }

    struct Packages(Option<PackageFormat>);

    impl SystemPackages for Packages {
        fn installed_format(&self) -> Option<PackageFormat> {
            self.0
        }
    }

    /// Records what it was asked to install, with the bytes it saw.
    #[derive(Default)]
    struct RecordingInstaller {
        installed: RefCell<Vec<(PackageFormat, String, Vec<u8>)>>,
        fail: bool,
    }

    impl PackageInstaller for RecordingInstaller {
        fn install(&self, format: PackageFormat, package: &Path) -> Result<(), String> {
            let name = package.file_name().unwrap().to_string_lossy().into_owned();
            let bytes = fs::read(package).unwrap();
            self.installed.borrow_mut().push((format, name, bytes));
            if self.fail {
                Err("boom".into())
            } else {
                Ok(())
            }
        }
    }

    struct Release {
        download: TempDir,
        staging: TempDir,
        key: ReleaseKey,
        signing: ed25519_dalek::SigningKey,
        package: Vec<u8>,
    }

    impl Release {
        /// A correctly signed release of `OFFERED`, for the deb format.
        fn signed() -> Self {
            let (signing, pem) = test_keys(1);
            let release = Self {
                download: TempDir::new().unwrap(),
                staging: TempDir::new().unwrap(),
                key: ReleaseKey::from_pem(&pem).unwrap(),
                signing,
                package: b"package bytes".to_vec(),
            };
            release.write_package(PackageFormat::Deb, OFFERED, &release.package);
            release.write_checksums(&Release::checksums_for(
                PackageFormat::Deb,
                OFFERED,
                &release.package,
            ));
            release
        }

        fn checksums_for(format: PackageFormat, version: Version, bytes: &[u8]) -> String {
            format!(
                "{}  {}\n",
                hex(&Sha256::digest(bytes)),
                format.file_name(version)
            )
        }

        fn write_package(&self, format: PackageFormat, version: Version, bytes: &[u8]) {
            fs::write(self.download.path().join(format.file_name(version)), bytes).unwrap();
        }

        fn write_checksums(&self, text: &str) {
            fs::write(self.download.path().join(CHECKSUMS_FILE), text).unwrap();
            let signature = self.signing.sign(text.as_bytes()).to_bytes();
            fs::write(self.download.path().join(SIGNATURE_FILE), signature).unwrap();
        }

        fn apply(
            &self,
            format: Option<PackageFormat>,
            installer: &RecordingInstaller,
        ) -> Result<Version, ApplyError> {
            apply(
                self.download.path(),
                &Environment {
                    key: &self.key,
                    installed: INSTALLED,
                    packages: &Packages(format),
                    installer,
                    staging_root: self.staging.path(),
                    staging_owner: owner_of(self.staging.path()),
                },
            )
        }
    }

    #[test]
    fn a_signed_newer_package_is_installed_from_the_checked_bytes() {
        let release = Release::signed();
        let installer = RecordingInstaller::default();
        assert_eq!(
            release.apply(Some(PackageFormat::Deb), &installer).unwrap(),
            OFFERED
        );
        let calls = installer.installed.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, PackageFormat::Deb);
        assert_eq!(calls[0].1, PackageFormat::Deb.file_name(OFFERED));
        assert_eq!(calls[0].2, release.package);
        // Nothing is left behind once the staging folder is dropped.
        assert_eq!(fs::read_dir(release.staging.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_package_of_the_other_format_is_never_installed() {
        let release = Release::signed();
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Pacman), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::NoPackage), "{error}");
        assert!(installer.installed.borrow().is_empty());
    }

    #[test]
    fn a_forged_signature_installs_nothing() {
        let release = Release::signed();
        let (other, _) = test_keys(2);
        let signature =
            other.sign(&fs::read(release.download.path().join(CHECKSUMS_FILE)).unwrap());
        fs::write(
            release.download.path().join(SIGNATURE_FILE),
            signature.to_bytes(),
        )
        .unwrap();
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(
            matches!(error, ApplyError::Signature(SignatureError::Mismatch)),
            "{error}"
        );
        assert!(installer.installed.borrow().is_empty());
    }

    #[test]
    fn edited_checksums_no_longer_match_their_signature() {
        let release = Release::signed();
        let path = release.download.path().join(CHECKSUMS_FILE);
        let mut text = fs::read_to_string(&path).unwrap();
        text.push('\n');
        fs::write(&path, text).unwrap();
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::Signature(_)), "{error}");
    }

    #[test]
    fn a_swapped_package_fails_the_checksum() {
        let release = Release::signed();
        release.write_package(PackageFormat::Deb, OFFERED, b"malicious bytes");
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::ChecksumMismatch), "{error}");
        assert!(installer.installed.borrow().is_empty());
    }

    #[test]
    fn an_older_or_same_signed_release_is_refused() {
        for version in [INSTALLED, Version::new(0, 9, 0)] {
            let release = Release::signed();
            release.write_package(PackageFormat::Deb, version, &release.package);
            release.write_checksums(&Release::checksums_for(
                PackageFormat::Deb,
                version,
                &release.package,
            ));
            let installer = RecordingInstaller::default();
            let error = release
                .apply(Some(PackageFormat::Deb), &installer)
                .unwrap_err();
            assert!(matches!(error, ApplyError::NotNewer { .. }), "{error}");
            assert!(installer.installed.borrow().is_empty());
        }
    }

    #[test]
    fn a_system_without_the_package_refuses() {
        let release = Release::signed();
        let installer = RecordingInstaller::default();
        let error = release.apply(None, &installer).unwrap_err();
        assert!(matches!(error, ApplyError::NotPackaged), "{error}");
    }

    #[test]
    fn two_packages_of_one_format_are_ambiguous() {
        let release = Release::signed();
        let first = Release::checksums_for(PackageFormat::Deb, OFFERED, &release.package);
        let second =
            Release::checksums_for(PackageFormat::Deb, Version::new(1, 2, 0), &release.package);
        release.write_checksums(&format!("{first}{second}"));
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::NoPackage), "{error}");
    }

    #[test]
    fn checksum_lines_with_bad_hashes_or_paths_are_ignored() {
        let release = Release::signed();
        let name = PackageFormat::Deb.file_name(OFFERED);
        for line in [
            format!("{}  {name}\n", "G".repeat(64)),
            format!("abcd  {name}\n"),
            format!("{}  ../{name}\n", "a".repeat(64)),
        ] {
            release.write_checksums(&line);
            let installer = RecordingInstaller::default();
            let error = release
                .apply(Some(PackageFormat::Deb), &installer)
                .unwrap_err();
            assert!(matches!(error, ApplyError::NoPackage), "{line:?}: {error}");
        }
    }

    #[test]
    fn binary_mode_checksum_lines_are_understood() {
        let release = Release::signed();
        let name = PackageFormat::Deb.file_name(OFFERED);
        release.write_checksums(&format!(
            "{} *{name}\n",
            hex(&Sha256::digest(&release.package))
        ));
        let installer = RecordingInstaller::default();
        assert_eq!(
            release.apply(Some(PackageFormat::Deb), &installer).unwrap(),
            OFFERED
        );
    }

    #[test]
    fn symbolic_links_and_oversized_files_are_refused() {
        let release = Release::signed();
        let target = release.download.path().join("elsewhere");
        fs::write(&target, b"x").unwrap();
        let link = release.download.path().join(SIGNATURE_FILE);
        fs::remove_file(&link).unwrap();
        symlink(&target, &link).unwrap();
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::Unacceptable(_)), "{error}");

        fs::remove_file(&link).unwrap();
        fs::write(&link, [0_u8; 65]).unwrap();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::Unacceptable(_)), "{error}");
    }

    #[test]
    fn missing_files_are_reported_by_name() {
        let release = Release::signed();
        fs::remove_file(release.download.path().join(SIGNATURE_FILE)).unwrap();
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(error.to_string().contains(SIGNATURE_FILE), "{error}");
    }

    #[test]
    fn the_signature_is_checked_before_anything_else_is_read() {
        // Forged signature, and neither the package nor anything else present:
        // the verdict is the signature's, not a missing file's.
        let release = Release::signed();
        fs::remove_file(
            release
                .download
                .path()
                .join(PackageFormat::Deb.file_name(OFFERED)),
        )
        .unwrap();
        fs::write(release.download.path().join(SIGNATURE_FILE), [1_u8; 64]).unwrap();
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::Signature(_)), "{error}");
    }

    #[test]
    fn the_version_is_judged_before_the_package_is_read() {
        let release = Release::signed();
        release.write_checksums(&Release::checksums_for(
            PackageFormat::Deb,
            INSTALLED,
            &release.package,
        ));
        // The package of that (not newer) version is absent.
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::NotNewer { .. }), "{error}");
    }

    #[test]
    fn the_same_package_listed_twice_is_ambiguous() {
        let release = Release::signed();
        let line = Release::checksums_for(PackageFormat::Deb, OFFERED, &release.package);
        release.write_checksums(&format!("{line}{line}"));
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::NoPackage), "{error}");
    }

    #[test]
    fn uppercase_hashes_are_not_accepted() {
        let release = Release::signed();
        let line = Release::checksums_for(PackageFormat::Deb, OFFERED, &release.package);
        release.write_checksums(&line.to_uppercase().replace(
            "HUBERGAMOF_1.1.0_AMD64.DEB",
            &PackageFormat::Deb.file_name(OFFERED),
        ));
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::NoPackage), "{error}");
    }

    #[test]
    fn a_directory_or_a_fifo_instead_of_a_file_is_refused_without_blocking() {
        let release = Release::signed();
        let checksums = release.download.path().join(CHECKSUMS_FILE);
        fs::remove_file(&checksums).unwrap();
        fs::create_dir(&checksums).unwrap();
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::Unacceptable(_)), "{error}");

        fs::remove_dir(&checksums).unwrap();
        let made = Command::new("mkfifo").arg(&checksums).status().unwrap();
        assert!(made.success());
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::Unacceptable(_)), "{error}");
    }

    #[test]
    fn a_symbolic_link_for_the_package_is_refused() {
        let release = Release::signed();
        let package = release
            .download
            .path()
            .join(PackageFormat::Deb.file_name(OFFERED));
        let target = release.download.path().join("elsewhere");
        fs::rename(&package, &target).unwrap();
        symlink(&target, &package).unwrap();
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::Unacceptable(_)), "{error}");
        assert!(installer.installed.borrow().is_empty());
    }

    #[test]
    fn the_size_limits_are_exact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        fs::write(&path, vec![0_u8; 10]).unwrap();
        let directory = open_download_dir(dir.path()).unwrap();
        assert_eq!(read_plain_file(&directory, "file", 10).unwrap().len(), 10);
        let error = read_plain_file(&directory, "file", 9).unwrap_err();
        assert!(matches!(error, ApplyError::Unacceptable(_)), "{error}");
    }

    #[test]
    fn the_staged_package_is_readable_by_the_download_user_and_a_name_cannot_escape() {
        let release = Release::signed();
        let staging = release.staging.path();
        let environment = Environment {
            key: &release.key,
            installed: INSTALLED,
            packages: &Packages(Some(PackageFormat::Deb)),
            installer: &ModeRecorder,
            staging_root: staging,
            staging_owner: owner_of(staging),
        };
        install_staged(&environment, PackageFormat::Deb, "p.deb", b"x").unwrap();
        for name in ["../escape", "a/b", "a\0b"] {
            let error = install_staged(&environment, PackageFormat::Deb, name, b"x").unwrap_err();
            assert!(
                matches!(error, ApplyError::Unacceptable(_)),
                "{name:?}: {error}"
            );
        }
        assert!(!staging.join("escape").exists());
    }

    /// Checks the modes of the staged folder and file while "installing".
    struct ModeRecorder;

    impl PackageInstaller for ModeRecorder {
        fn install(&self, _format: PackageFormat, package: &Path) -> Result<(), String> {
            let file_mode = fs::metadata(package).unwrap().permissions().mode() & 0o777;
            let dir_mode = fs::metadata(package.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!((dir_mode, file_mode), (STAGING_DIR_MODE, STAGED_FILE_MODE));
            Ok(())
        }
    }

    #[test]
    fn a_relative_download_folder_or_a_link_to_one_is_refused() {
        let error = open_download_dir(Path::new("relative/dir")).unwrap_err();
        assert!(matches!(error, ApplyError::Unacceptable(_)), "{error}");

        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        symlink(&real, &link).unwrap();
        assert!(open_download_dir(&real).is_ok());
        assert!(open_download_dir(&link).is_err());
        assert!(open_download_dir(&dir.path().join("absent")).is_err());
    }

    #[test]
    fn a_link_to_a_fifo_is_refused_without_blocking() {
        let release = Release::signed();
        let checksums = release.download.path().join(CHECKSUMS_FILE);
        fs::remove_file(&checksums).unwrap();
        let fifo = release.download.path().join("pipe");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        symlink(&fifo, &checksums).unwrap();
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::Unacceptable(_)), "{error}");
    }

    #[test]
    fn a_staging_root_others_could_write_in_is_refused() {
        let release = Release::signed();
        let root = release.staging.path();
        let installer = RecordingInstaller::default();
        for mode in [0o777, 0o775, 0o757] {
            fs::set_permissions(root, fs::Permissions::from_mode(mode)).unwrap();
            let error = release
                .apply(Some(PackageFormat::Deb), &installer)
                .unwrap_err();
            assert!(
                matches!(error, ApplyError::UnsafeStaging(_)),
                "{mode:o}: {error}"
            );
        }
        assert!(installer.installed.borrow().is_empty());
        // Restored, it works.
        fs::set_permissions(root, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(release.apply(Some(PackageFormat::Deb), &installer).is_ok());
    }

    #[test]
    fn a_missing_wrongly_owned_or_linked_staging_root_is_refused() {
        let release = Release::signed();
        let installer = RecordingInstaller::default();
        let base = tempfile::tempdir().unwrap();
        let missing = base.path().join("absent");
        let real = base.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = base.path().join("link");
        symlink(&real, &link).unwrap();
        let owner = owner_of(&real);
        for (root, owner) in [(&missing, owner), (&link, owner), (&real, owner + 1)] {
            let error = apply(
                release.download.path(),
                &Environment {
                    key: &release.key,
                    installed: INSTALLED,
                    packages: &Packages(Some(PackageFormat::Deb)),
                    installer: &installer,
                    staging_root: root,
                    staging_owner: owner,
                },
            )
            .unwrap_err();
            assert!(
                matches!(error, ApplyError::UnsafeStaging(_)),
                "{}: {error}",
                root.display()
            );
        }
        assert!(!missing.exists(), "a missing staging root is not created");
    }

    #[test]
    fn the_package_managers_are_called_without_a_shell_with_the_file_last() {
        let package = Path::new("/var/lib/hubergamof/update-x/hubergamof_1.1.0_amd64.deb");
        let deb = install_command(PackageFormat::Deb, package);
        assert_eq!(deb.get_program(), "apt-get");
        let arguments: Vec<_> = deb.get_args().collect();
        assert_eq!(
            arguments,
            [
                "install",
                "--yes",
                "--no-install-recommends",
                "--option",
                "Dpkg::Options::=--force-confdef",
                "--option",
                "Dpkg::Options::=--force-confold",
                "--option",
                "DPkg::Lock::Timeout=120",
                package.to_str().unwrap()
            ]
        );
        let pacman = install_command(PackageFormat::Pacman, package);
        assert_eq!(pacman.get_program(), "pacman");
        let arguments: Vec<_> = pacman.get_args().collect();
        assert_eq!(
            arguments,
            ["--upgrade", "--noconfirm", package.to_str().unwrap()]
        );
        // An absolute path can never be taken for an option.
        assert!(package.is_absolute());
    }

    #[test]
    fn a_failing_package_manager_is_reported() {
        let release = Release::signed();
        let installer = RecordingInstaller {
            fail: true,
            ..RecordingInstaller::default()
        };
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::Install(message) if message == "boom"));
    }
}
