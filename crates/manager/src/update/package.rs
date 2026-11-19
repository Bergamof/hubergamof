//! The release packages: their file names, and which format this system uses.

use std::process::{Command, Stdio};

use super::version::Version;

/// Name of the file listing the SHA-256 of every release package.
pub const CHECKSUMS_FILE: &str = "SHA256SUMS";

/// Name of the Ed25519 signature of [`CHECKSUMS_FILE`].
pub const SIGNATURE_FILE: &str = "SHA256SUMS.sig";

/// Name of the package, for both formats.
const PACKAGE_NAME: &str = "hubergamof";

/// Release packages are built for this architecture only.
const DEB_ARCHITECTURE: &str = "amd64";
const PACMAN_ARCHITECTURE: &str = "x86_64";

/// Package revision of the pacman package (`pkgrel`).
const PACMAN_RELEASE: &str = "1";
const PACMAN_EXTENSION: &str = ".pkg.tar.zst";

/// How the manager was installed, hence which package fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageFormat {
    /// Debian, Ubuntu: `hubergamof_X.Y.Z_amd64.deb`.
    Deb,
    /// Arch, Manjaro: `hubergamof-X.Y.Z-1-x86_64.pkg.tar.zst`.
    Pacman,
}

impl PackageFormat {
    /// File name of the release package of `version`.
    #[must_use]
    pub fn file_name(self, version: Version) -> String {
        match self {
            Self::Deb => format!("{PACKAGE_NAME}_{version}_{DEB_ARCHITECTURE}.deb"),
            Self::Pacman => format!(
                "{PACKAGE_NAME}-{version}-{PACMAN_RELEASE}-{PACMAN_ARCHITECTURE}{PACMAN_EXTENSION}"
            ),
        }
    }

    /// Version of a release package of this format, from its file name;
    /// `None` for any other name (so a name never carries a path).
    #[must_use]
    pub fn version_of(self, file_name: &str) -> Option<Version> {
        let (prefix, suffix) = match self {
            Self::Deb => (
                format!("{PACKAGE_NAME}_"),
                format!("_{DEB_ARCHITECTURE}.deb"),
            ),
            Self::Pacman => (
                format!("{PACKAGE_NAME}-"),
                format!("-{PACMAN_RELEASE}-{PACMAN_ARCHITECTURE}{PACMAN_EXTENSION}"),
            ),
        };
        let version: Version = file_name
            .strip_prefix(&prefix)?
            .strip_suffix(&suffix)?
            .parse()
            .ok()?;
        // Round trip: refuses anything the parser tolerates but the build
        // never produces (a leading `v`).
        (self.file_name(version) == file_name).then_some(version)
    }
}

/// What the system's package managers are asked.
pub trait SystemPackages: Send + Sync {
    /// Format of the installed `hubergamof` package; `None` when it was not
    /// installed from a package.
    fn installed_format(&self) -> Option<PackageFormat>;
}

/// The real `dpkg` and `pacman`.
pub struct Installed;

impl SystemPackages for Installed {
    fn installed_format(&self) -> Option<PackageFormat> {
        if queried("dpkg-query", &["-W", PACKAGE_NAME]) {
            Some(PackageFormat::Deb)
        } else if queried("pacman", &["-Q", PACKAGE_NAME]) {
            Some(PackageFormat::Pacman)
        } else {
            None
        }
    }
}

/// Whether `program args` succeeds; a missing program counts as no.
fn queried(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: Version = Version::new(1, 4, 2);

    #[test]
    fn names_follow_the_release_artifacts() {
        assert_eq!(
            PackageFormat::Deb.file_name(VERSION),
            "hubergamof_1.4.2_amd64.deb"
        );
        assert_eq!(
            PackageFormat::Pacman.file_name(VERSION),
            "hubergamof-1.4.2-1-x86_64.pkg.tar.zst"
        );
    }

    #[test]
    fn a_name_gives_back_its_version() {
        for format in [PackageFormat::Deb, PackageFormat::Pacman] {
            assert_eq!(format.version_of(&format.file_name(VERSION)), Some(VERSION));
        }
    }

    #[test]
    fn anything_else_is_not_a_release_package() {
        for name in [
            "",
            "SHA256SUMS",
            "../hubergamof_1.4.2_amd64.deb",
            "/etc/hubergamof_1.4.2_amd64.deb",
            "hubergamof_v1.4.2_amd64.deb",
            "hubergamof_1.4.2_arm64.deb",
            "hubergamof_1.4_amd64.deb",
            "hubergamof_1.4.2_amd64.deb.sig",
            "other_1.4.2_amd64.deb",
            "hubergamof-1.4.2-2-x86_64.pkg.tar.zst",
        ] {
            assert_eq!(PackageFormat::Deb.version_of(name), None, "{name:?}");
            assert_eq!(PackageFormat::Pacman.version_of(name), None, "{name:?}");
        }
        // A package of the other format is not one of this format.
        assert_eq!(
            PackageFormat::Pacman.version_of("hubergamof_1.4.2_amd64.deb"),
            None
        );
    }

    #[test]
    fn a_missing_program_is_not_installed() {
        assert!(!queried("hubergamof-no-such-program", &[]));
        assert!(queried("true", &[]));
        assert!(!queried("false", &[]));
    }
}
