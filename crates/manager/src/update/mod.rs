//! Check for a newer release of the manager (see `docs/decisions.md`).

mod checker;
mod source;
mod version;

pub use checker::{CHECK_INTERVAL, FIRST_CHECK_DELAY, UpdateHandle, start};
pub use source::{GithubReleases, ReleaseInfo, ReleaseSource, SimulatedReleases, UpdateError};
pub use version::{ParseVersionError, Version};
