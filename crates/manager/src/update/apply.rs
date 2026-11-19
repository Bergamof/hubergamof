//! What the update helper does, as root, with a bundle it does not trust:
//! verifies the release signature, then the package against it, and only then
//! hands the package it checked to the system's package manager.
//!
//! The bundle arrives as bytes on the helper's socket (see [`super::wire`]),
//! so there is no file of the caller's to race against: what is hashed is what
//! is written to a folder only root can write in, and installed from there.

use std::fs;
use std::io::{self, Write as _};
use std::os::fd::AsFd as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rustix::fs::Mode;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use super::package::{PackageFormat, SystemPackages};
use super::signature::{ReleaseKey, SignatureError};
use super::version::Version;
use super::wire::Bundle;

/// Permissions of the folder holding the package: readable by the package
/// manager's unprivileged download user.
const STAGING_DIR_MODE: u32 = 0o755;
const STAGED_FILE_MODE: u32 = 0o644;

/// The umask the package manager (and the hooks it starts) run with.
const PACKAGE_MANAGER_UMASK: u32 = 0o022;

/// Prefix of the folders staging a package under the staging root.
const STAGING_PREFIX: &str = "update-";

/// Mode of the staged package while it is written: the helper only.
const STAGED_FILE_WRITE_MODE: u32 = 0o600;

/// Owner a staging root must have (root).
pub const ROOT_UID: u32 = 0;

/// Length of a SHA-256 in hexadecimal.
const SHA256_HEX_LENGTH: usize = 64;

/// Why an update is refused or fails.
#[derive(Debug, Error)]
pub enum ApplyError {
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
    /// A name that is not a plain file name.
    #[error("{0} is not an acceptable file name")]
    Unacceptable(String),
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
        // The helper runs with a private umask for what it creates itself; the
        // package manager and the hooks it runs get the usual one.
        rustix::process::umask(Mode::from_raw_mode(PACKAGE_MANAGER_UMASK));
        // The helper's standard output is the socket the answer goes back on:
        // what the package manager prints goes to the log (standard error).
        let log = io::stderr()
            .as_fd()
            .try_clone_to_owned()
            .map_err(|error| format!("cannot reach the log: {error}"))?;
        let status = install_command(format, package)
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                log.try_clone()
                    .map_err(|error| format!("cannot reach the log: {error}"))?,
            ))
            .stderr(Stdio::from(log))
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
    /// exist (systemd makes it for the unit), belong to `staging_owner` and be
    /// writable by nobody else.
    pub staging_root: &'a Path,
    /// Uid that must own `staging_root`: root, except in tests.
    pub staging_owner: u32,
}

/// Verifies and installs the release in `bundle`; returns the version
/// installed.
///
/// # Errors
///
/// [`ApplyError`], without having installed anything, unless the package
/// manager itself fails.
pub fn apply(bundle: &Bundle, environment: &Environment<'_>) -> Result<Version, ApplyError> {
    environment
        .key
        .verify(&bundle.checksums, &bundle.signature)?;

    let format = environment
        .packages
        .installed_format()
        .ok_or(ApplyError::NotPackaged)?;
    let (name, version, expected) = pick_package(&bundle.checksums, format)?;
    if version <= environment.installed {
        return Err(ApplyError::NotNewer {
            offered: version,
            installed: environment.installed,
        });
    }
    if hex(&Sha256::digest(&bundle.package)) != expected {
        return Err(ApplyError::ChecksumMismatch);
    }
    install_staged(environment, format, &name, &bundle.package)?;
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
    remove_stale_staging(environment.staging_root);
    let staging = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
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

/// Removes what an interrupted run left under the staging root (only the
/// helper writes there, and one run at a time reaches this point).
fn remove_stale_staging(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .file_name()
            .to_string_lossy()
            .starts_with(STAGING_PREFIX);
        if stale && let Err(error) = fs::remove_dir_all(entry.path()) {
            eprintln!("cannot remove {}: {error}", entry.path().display());
        }
    }
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

    struct Release {
        staging: TempDir,
        key: ReleaseKey,
        signing: ed25519_dalek::SigningKey,
        bundle: Bundle,
    }

    fn checksums_for(format: PackageFormat, version: Version, bytes: &[u8]) -> String {
        format!(
            "{}  {}\n",
            hex(&Sha256::digest(bytes)),
            format.file_name(version)
        )
    }

    impl Release {
        /// A correctly signed release of `OFFERED`, for the deb format.
        fn signed() -> Self {
            let (signing, pem) = test_keys(1);
            let mut release = Self {
                staging: TempDir::new().unwrap(),
                key: ReleaseKey::from_pem(&pem).unwrap(),
                signing,
                bundle: Bundle {
                    checksums: Vec::new(),
                    signature: Vec::new(),
                    package: b"package bytes".to_vec(),
                },
            };
            release.sign(&checksums_for(
                PackageFormat::Deb,
                OFFERED,
                b"package bytes",
            ));
            release
        }

        /// Replaces the checksums by `text`, correctly signed.
        fn sign(&mut self, text: &str) {
            self.bundle.checksums = text.as_bytes().to_vec();
            self.bundle.signature = self.signing.sign(text.as_bytes()).to_bytes().to_vec();
        }

        fn apply(
            &self,
            format: Option<PackageFormat>,
            installer: &RecordingInstaller,
        ) -> Result<Version, ApplyError> {
            apply(
                &self.bundle,
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
        assert_eq!(calls[0].2, release.bundle.package);
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
        let mut release = Release::signed();
        let (other, _) = test_keys(2);
        release.bundle.signature = other.sign(&release.bundle.checksums).to_bytes().to_vec();
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
        let mut release = Release::signed();
        release.bundle.checksums.push(b'\n');
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::Signature(_)), "{error}");
    }

    #[test]
    fn a_malformed_signature_is_refused() {
        let mut release = Release::signed();
        release.bundle.signature.truncate(10);
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(
            matches!(
                error,
                ApplyError::Signature(SignatureError::MalformedSignature)
            ),
            "{error}"
        );
    }

    #[test]
    fn a_swapped_package_fails_the_checksum() {
        let mut release = Release::signed();
        release.bundle.package = b"malicious bytes".to_vec();
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
            let mut release = Release::signed();
            release.sign(&checksums_for(
                PackageFormat::Deb,
                version,
                b"package bytes",
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
    fn the_version_is_judged_before_the_package_is_looked_at() {
        let mut release = Release::signed();
        release.sign(&checksums_for(
            PackageFormat::Deb,
            INSTALLED,
            b"package bytes",
        ));
        // Not even the right package: the (not newer) version is what is refused.
        release.bundle.package = b"other".to_vec();
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::NotNewer { .. }), "{error}");
    }

    #[test]
    fn the_signature_is_checked_before_anything_else() {
        // Forged signature on checksums that also list nothing usable.
        let mut release = Release::signed();
        release.bundle.checksums = b"not even checksums".to_vec();
        let installer = RecordingInstaller::default();
        let error = release.apply(None, &installer).unwrap_err();
        assert!(matches!(error, ApplyError::Signature(_)), "{error}");
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
        let mut release = Release::signed();
        let first = checksums_for(PackageFormat::Deb, OFFERED, b"package bytes");
        let second = checksums_for(PackageFormat::Deb, Version::new(1, 2, 0), b"package bytes");
        release.sign(&format!("{first}{second}"));
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::NoPackage), "{error}");
    }

    #[test]
    fn the_same_package_listed_twice_is_ambiguous() {
        let mut release = Release::signed();
        let line = checksums_for(PackageFormat::Deb, OFFERED, b"package bytes");
        release.sign(&format!("{line}{line}"));
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::NoPackage), "{error}");
    }

    #[test]
    fn checksum_lines_with_bad_hashes_or_paths_are_ignored() {
        let name = PackageFormat::Deb.file_name(OFFERED);
        for line in [
            format!("{}  {name}\n", "G".repeat(64)),
            format!("{}  {name}\n", hex(&Sha256::digest(b"x")).to_uppercase()),
            format!("abcd  {name}\n"),
            format!("{}  ../{name}\n", "a".repeat(64)),
        ] {
            let mut release = Release::signed();
            release.sign(&line);
            let installer = RecordingInstaller::default();
            let error = release
                .apply(Some(PackageFormat::Deb), &installer)
                .unwrap_err();
            assert!(matches!(error, ApplyError::NoPackage), "{line:?}: {error}");
        }
    }

    #[test]
    fn binary_mode_checksum_lines_are_understood() {
        let mut release = Release::signed();
        let name = PackageFormat::Deb.file_name(OFFERED);
        release.sign(&format!(
            "{} *{name}\n",
            hex(&Sha256::digest(b"package bytes"))
        ));
        let installer = RecordingInstaller::default();
        assert_eq!(
            release.apply(Some(PackageFormat::Deb), &installer).unwrap(),
            OFFERED
        );
    }

    #[test]
    fn non_text_checksums_are_refused() {
        let mut release = Release::signed();
        let bytes = vec![0xff, 0xfe, b'\n'];
        release.bundle.signature = release.signing.sign(&bytes).to_bytes().to_vec();
        release.bundle.checksums = bytes;
        let installer = RecordingInstaller::default();
        let error = release
            .apply(Some(PackageFormat::Deb), &installer)
            .unwrap_err();
        assert!(matches!(error, ApplyError::NoPackage), "{error}");
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
                &release.bundle,
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
    fn what_an_interrupted_run_left_is_removed_and_nothing_else() {
        let release = Release::signed();
        let stale = release.staging.path().join("update-OLD");
        fs::create_dir(&stale).unwrap();
        fs::write(stale.join("hubergamof_0.0.1_amd64.deb"), b"old").unwrap();
        let other = release.staging.path().join("keep-me");
        fs::create_dir(&other).unwrap();
        let installer = RecordingInstaller::default();
        release.apply(Some(PackageFormat::Deb), &installer).unwrap();
        assert!(!stale.exists());
        assert!(other.exists());
    }

    #[test]
    fn the_package_managers_are_called_without_a_shell_with_the_file_last() {
        let package = Path::new("/run/hubergamof-update/update-x/hubergamof_1.1.0_amd64.deb");
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
}
