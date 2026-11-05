//! Page engine: one task per attached device, owning its configuration, its
//! module instances and its navigation, and drawing what they show.
//!
//! - The home page shows the key of each module of `config.home`; a module
//!   opens pages of keys ([`hubergamof_module_api::Command::OpenPage`]). Pages
//!   larger than the key grid scroll by rows.
//! - Dial `i` drives the module `config.dials[i]`; the touch strip zone above
//!   it shows that module's zone.
//! - Only the keys and zones whose look changed are rendered (on the blocking
//!   pool) and written. A key press acts on what the last drawn frame shows.
//! - Module code is isolated: a panic replaces the instance by a placeholder,
//!   a handler past its budget is abandoned; both show an alert mark.
//! - [`EngineHandle`] is the entry point of the web UI: simulated
//!   inputs handled exactly like hardware ones, configuration updates, and
//!   the live state ([`LiveSnapshot`]) and configuration as `watch` channels.
//! - [`Supervisor`] starts an engine for every attached device and stops it
//!   on detach or shutdown.

mod device_engine;
mod feedback;
mod frame;
mod handle;
mod instances;
mod layout;
mod live;
mod navigation;
mod painter;
mod supervisor;
mod validation;

use std::sync::Arc;

use hubergamof_module_api::ModuleInstanceId;
use hubergamof_modules::ModuleRegistry;
use hubergamof_protocol::ErrorCode;
use thiserror::Error;

pub use device_engine::DeviceEngine;
pub use handle::EngineHandle;
pub use live::{LiveDial, LiveKey, LiveSnapshot, LiveZone, PageSummary};
pub use supervisor::{EngineRegistry, Supervisor};

use crate::config_store::{ConfigStore, ConfigStoreError};
use crate::device::DeviceError;
use crate::render::{RenderError, Renderer};

/// Id of the home page in [`PageSummary::id`].
pub const HOME_PAGE_ID: &str = "home";

/// Title of the home page.
pub const HOME_PAGE_TITLE: &str = "Accueil";

/// Services shared by every engine. Cheap to clone.
#[derive(Clone)]
pub struct EngineServices {
    /// Configuration storage.
    pub store: ConfigStore,
    /// Key and zone renderer.
    pub renderer: Arc<Renderer>,
    /// Available modules.
    pub registry: Arc<ModuleRegistry>,
}

/// Failure of an engine operation.
#[derive(Debug, Error)]
pub enum EngineError {
    /// The engine is no longer running (device detached or shut down).
    #[error("device engine stopped")]
    Stopped,
    /// A device operation failed, or the simulated input does not exist on
    /// the device.
    #[error(transparent)]
    Device(#[from] DeviceError),
    /// A module refuses the configuration: invalid settings, or placed where
    /// it cannot work.
    #[error("module `{module}`: {reason}")]
    InvalidConfig {
        /// Offending module instance.
        module: ModuleInstanceId,
        /// What is wrong.
        reason: String,
    },
    /// Loading or saving the configuration failed.
    #[error(transparent)]
    Config(#[from] ConfigStoreError),
    /// Rendering failed.
    #[error(transparent)]
    Render(#[from] RenderError),
    /// A blocking task panicked or was cancelled.
    #[error("background task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

impl EngineError {
    /// Error code reported to the UI.
    #[must_use]
    pub fn error_code(&self) -> ErrorCode {
        match self {
            Self::Stopped => ErrorCode::UnknownDevice,
            Self::Device(
                DeviceError::InvalidKey { .. }
                | DeviceError::InvalidDial { .. }
                | DeviceError::InvalidZone { .. },
            ) => ErrorCode::InvalidInput,
            Self::InvalidConfig { .. } => ErrorCode::InvalidConfig,
            Self::Config(error) => error.error_code(),
            Self::Device(_) | Self::Render(_) | Self::Task(_) => ErrorCode::Internal,
        }
    }
}

#[cfg(test)]
mod tests;
