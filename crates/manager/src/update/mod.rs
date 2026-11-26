//! Check for a newer release of the manager (see `docs/decisions.md`).

pub mod apply;
mod checker;
mod installer;
mod package;
mod policy;
mod signature;
mod source;
mod version;
pub mod wire;

pub use checker::{
    CHECK_INTERVAL, FIRST_CHECK_DELAY, MIN_CHECK_SPACING, UpdateHandle, UpdateParts, start,
    system_seconds,
};
pub use installer::{
    Downloader, InstallError, InstallStage, Installer, PackageInstaller, SimulatedInstaller,
    SocketChannel, UnsupportedInstaller, UpdateChannel,
};
pub use package::{CHECKSUMS_FILE, Installed, PackageFormat, SIGNATURE_FILE, SystemPackages};
pub use policy::{FilePolicyStore, MemoryPolicyStore, POLICY_FILE, PolicyStore, UpdatePolicy};
pub use signature::{ReleaseKey, SignatureError};
pub use source::{
    GithubReleases, ReleaseAsset, ReleaseInfo, ReleaseSource, SimulatedReleases, UpdateError,
};
pub use version::{ParseVersionError, Version};
