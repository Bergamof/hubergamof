//! `hubergamof-update-helper` — installs a downloaded release, as root.
//!
//! Started by the manager through `pkexec` (polkit action
//! `io.github.bergamof.hubergamof.update`) with the folder holding the
//! download: `SHA256SUMS`, its signature and the package. It trusts none of
//! it: the signature is checked against the key embedded here, the package
//! against the signed checksums, and only a newer version of the package this
//! system already has is handed to `apt-get` or `pacman`.
//!
//! It does one thing, takes one argument and reads no configuration: it is
//! what lets an unprivileged process update the package.

use std::path::Path;
use std::process::ExitCode;

use hubergamof_manager::update::apply::{Environment, ROOT_UID, SystemInstaller, apply};
use hubergamof_manager::update::{Installed, ReleaseKey, Version};

/// Where the checked package is staged: shipped by the package (root, 0755),
/// never created here, so nobody else can ever have made it.
const STAGING_ROOT: &str = "/var/lib/hubergamof";

const PROGRAM: &str = "hubergamof-update-helper";

/// What the helper creates is closed to everybody else: `pkexec` hands down
/// the caller's umask, which could be 0.
const PRIVATE_UMASK: u32 = 0o077;

fn main() -> ExitCode {
    rustix::process::umask(rustix::fs::Mode::from_raw_mode(PRIVATE_UMASK));
    let mut arguments = std::env::args_os().skip(1);
    let (Some(download_dir), None) = (arguments.next(), arguments.next()) else {
        eprintln!("{PROGRAM}: usage: {PROGRAM} <download folder>");
        return ExitCode::from(2);
    };
    match run(Path::new(&download_dir)) {
        Ok(version) => {
            eprintln!("{PROGRAM}: installed {version}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{PROGRAM}: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(download_dir: &Path) -> Result<Version, String> {
    let key = ReleaseKey::embedded().map_err(|error| error.to_string())?;
    let installed = Version::current().map_err(|error| error.to_string())?;
    apply(
        download_dir,
        &Environment {
            key: &key,
            installed,
            packages: &Installed,
            installer: &SystemInstaller,
            staging_root: Path::new(STAGING_ROOT),
            staging_owner: ROOT_UID,
        },
    )
    .map_err(|error| error.to_string())
}
