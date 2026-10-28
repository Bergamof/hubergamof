//! The [`Module`] and [`ModuleFactory`] traits.

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;

use crate::{
    Command, ItemId, KeyContent, ModuleContext, ModuleDescriptor, NavigationView, PageContent,
    PageId, ZoneContent,
};

/// Failure of a module operation.
#[derive(Debug, Error)]
pub enum ModuleError {
    /// The settings do not match what the module expects.
    #[error("invalid settings: {0}")]
    InvalidSettings(String),
    /// Any other failure.
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Deserializes a settings object into the typed settings of a module.
///
/// Missing fields take their `#[serde(default)]` values, so a module stays
/// usable with settings written by an older version.
///
/// # Errors
///
/// [`ModuleError::InvalidSettings`] when `settings` does not match `T`.
pub fn parse_settings<T: DeserializeOwned>(settings: &Value) -> Result<T, ModuleError> {
    T::deserialize(settings).map_err(|error| ModuleError::InvalidSettings(error.to_string()))
}

/// Creates instances of one module kind and describes it in the catalog.
pub trait ModuleFactory: Send + Sync {
    /// Kind, name, capabilities and settings of the module.
    fn descriptor(&self) -> ModuleDescriptor;

    /// Settings of a new instance. Defaults to the default value of every
    /// setting of [`ModuleFactory::descriptor`].
    fn default_settings(&self) -> Value {
        self.descriptor().default_settings()
    }

    /// Creates an instance configured with `settings`.
    ///
    /// Called from within the host Tokio runtime, so background tasks can be
    /// spawned right away through [`ModuleContext::tasks`].
    ///
    /// # Errors
    ///
    /// [`ModuleError::InvalidSettings`] when the settings are unusable, or
    /// any failure preventing the instance from working. The host then shows
    /// a placeholder key instead of the module.
    fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError>;
}

/// A configured module instance driving part of a device.
///
/// An instance may appear as a key on the home page, open pages of keys, be
/// assigned to a dial and draw the touch strip zone above that dial. Every
/// method has a default doing nothing, except
/// [`Module::update_settings`]: implement only the surfaces the module uses.
///
/// Getters (`key`, `page`, `zone`) are called on every redraw and must be
/// cheap: they describe the current state, the host compares the result with
/// what is displayed and only rewrites what changed. Input handlers are
/// awaited one at a time by the device task: long work (I/O, processes)
/// belongs in a background task spawned through [`ModuleContext::tasks`],
/// which then calls [`crate::RedrawHandle::request_redraw`].
#[async_trait]
pub trait Module: Send {
    /// Key shown on the home page. The device configuration may override its
    /// label and image.
    fn key(&self) -> KeyContent {
        KeyContent::default()
    }

    /// Handles a press of the home page key.
    async fn on_key_press(&mut self) -> Vec<Command> {
        Vec::new()
    }

    /// Content of page `page`, previously opened with
    /// [`Command::OpenPage`]; `None` when the page no longer exists, in which
    /// case the host goes back to a page that does.
    fn page(&self, page: &PageId) -> Option<PageContent> {
        let _ = page;
        None
    }

    /// Handles a press of item `item` of page `page`.
    async fn on_item_press(&mut self, page: &PageId, item: &ItemId) -> Vec<Command> {
        let _ = (page, item);
        Vec::new()
    }

    /// Handles a rotation of the dial the instance is assigned to, by
    /// `ticks` steps (positive clockwise).
    async fn on_dial_rotate(&mut self, ticks: i32) -> Vec<Command> {
        let _ = ticks;
        Vec::new()
    }

    /// Handles a press of the dial the instance is assigned to.
    async fn on_dial_press(&mut self) -> Vec<Command> {
        Vec::new()
    }

    /// Content of the touch strip zone above the dial the instance is
    /// assigned to; `None` leaves the zone black.
    fn zone(&self, nav: &NavigationView) -> Option<ZoneContent> {
        let _ = nav;
        None
    }

    /// Applies new settings while keeping the running state (a timer in
    /// progress, a connection…). Required, so that no module silently
    /// ignores a settings change; a module without settings returns `Ok(())`.
    ///
    /// # Errors
    ///
    /// [`ModuleError::InvalidSettings`]; the instance then keeps its previous
    /// settings.
    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError>;
}
