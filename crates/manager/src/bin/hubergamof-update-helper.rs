//! `hubergamof-update-helper` — installs a release the manager sends, as root.
//!
//! Started by systemd for each connection to `hubergamof-update.socket`
//! (`/run/hubergamof-update.sock`): the connection is its standard input and
//! output. It reads three frames from it — the signed checksums, their
//! signature, the package — and trusts none of them: the signature is checked
//! against the key embedded here, the package against the signed checksums,
//! and only a newer version of the package this system already has is handed
//! to `apt-get` or `pacman`. One line goes back: `ok <version>` or
//! `error <reason>`.
//!
//! It takes no argument, reads no configuration and no file of the caller's:
//! it is what lets an unprivileged process update the package, whoever
//! connects (the signature is the only thing that matters).

use std::io;
use std::os::fd::AsFd as _;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use hubergamof_manager::update::apply::{Environment, ROOT_UID, SystemInstaller, apply};
use hubergamof_manager::update::wire::{RECEIVE_DEADLINE, Response, peer_allowed, read_bundle};
use hubergamof_manager::update::{Installed, ReleaseKey, Version};

/// Where the checked package is staged: made by systemd for the unit
/// (`RuntimeDirectory=`, root, 0755), never created here.
const STAGING_ROOT: &str = "/run/hubergamof-update";

/// What the helper creates is closed to everybody else.
const PRIVATE_UMASK: u32 = 0o077;

/// Longest silence tolerated on the socket between two reads.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

fn main() -> ExitCode {
    rustix::process::umask(rustix::fs::Mode::from_raw_mode(PRIVATE_UMASK));
    let response = match run() {
        Ok(version) => Response::Installed(version.to_string()),
        Err(message) => {
            eprintln!("{message}");
            Response::Failed(message)
        }
    };
    let delivered = response.write_to(&mut io::stdout()).is_ok();
    if matches!(response, Response::Installed(_)) && delivered {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Logs who connected and refuses system accounts: the socket is open to every
/// local user, but a web server or a daemon has no business updating the
/// package. (Not a socket, in tests: no credentials to read.)
fn check_peer() -> Result<(), String> {
    let Ok(credentials) = rustix::net::sockopt::socket_peercred(io::stdin().as_fd()) else {
        return Ok(());
    };
    let uid = credentials.uid.as_raw();
    eprintln!(
        "update requested by uid {uid} (pid {})",
        credentials.pid.as_raw_nonzero()
    );
    if peer_allowed(uid) {
        Ok(())
    } else {
        Err("system accounts may not request an update".to_owned())
    }
}

fn run() -> Result<Version, String> {
    let key = ReleaseKey::embedded().map_err(|error| error.to_string())?;
    let installed = Version::current().map_err(|error| error.to_string())?;
    check_peer()?;
    // A peer that stops sending must not hold the single update slot: reads on
    // the socket time out (on a pipe, in tests, they cannot: the overall
    // deadline stays).
    if let Err(error) = rustix::net::sockopt::set_socket_timeout(
        io::stdin().as_fd(),
        rustix::net::sockopt::Timeout::Recv,
        Some(IDLE_TIMEOUT),
    ) {
        eprintln!("no idle timeout on the connection: {error}");
    }
    let bundle = read_bundle(&mut io::stdin().lock(), Instant::now() + RECEIVE_DEADLINE)
        .map_err(|error| error.to_string())?;
    apply(
        &bundle,
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
