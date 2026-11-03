//! The [`Module`] and [`ModuleFactory`] traits.

use std::time::Duration;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;

use crate::{
    Command, ImageData, ItemId, KeyContent, ModuleContext, ModuleDescriptor, ModuleEvent,
    NavigationView, PageContent, PageId, ZoneContent,
};

/// Longest time the host lets a handler run (see [`Module`]) before
/// dropping it and showing an alert.
pub const HANDLER_BUDGET: Duration = Duration::from_millis(500);

/// Failure of a module operation.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ModuleError {
    /// The settings do not match what the module expects.
    #[error("invalid settings: {0}")]
    InvalidSettings(String),
    /// Any other failure.
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Deserializes a settings object into the typed settings of a module,
/// without checking the bounds declared in its descriptor: modules with
/// bounded settings use [`ModuleDescriptor::parse_settings`], which does both.
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

    /// Image standing for the module in the configuration interface, on the
    /// keys and dials it is placed on: PNG, JPEG or SVG (see [`ImageData`]),
    /// drawn by the host like a key (square, on a dark background, about
    /// 144 px). `None` (the default), or an image the host cannot read
    /// (logged as a warning): the interface shows the module name instead.
    fn icon(&self) -> Option<ImageData> {
        None
    }

    /// Settings of a new instance. Defaults to the default value of every
    /// setting of [`ModuleFactory::descriptor`].
    fn default_settings(&self) -> Value {
        self.descriptor().default_settings()
    }

    /// Checks settings before the host saves them, so that an invalid
    /// configuration is refused instead of being stored and then ignored by
    /// the instance. Defaults to [`ModuleDescriptor::validate_settings`]
    /// (types, bounds, choices); override it for checks the descriptor
    /// cannot express.
    ///
    /// # Errors
    ///
    /// [`ModuleError::InvalidSettings`] explaining what is wrong.
    fn validate_settings(&self, settings: &Value) -> Result<(), ModuleError> {
        self.descriptor().validate_settings(settings)
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
/// This is the in-process module API: built-in modules implement it
/// directly, and it is also the target of the adapter that will host future
/// external modules (WASM or sub-process), so that the host only ever drives
/// this trait.
///
/// An instance may appear as a key on the home page, open pages of keys, be
/// assigned to a dial and draw the touch strip zone above that dial. Every
/// method has a default doing nothing, except
/// [`Module::update_settings`]: implement only the surfaces the module uses.
///
/// # Getters
///
/// `key`, `page` and `zone` are called on every redraw and must be cheap and
/// side-effect free: they describe the current state, the host compares the
/// result with what is displayed and only rewrites what changed.
///
/// # Handlers
///
/// Input handlers and [`Module::on_event`] are awaited one at a time by the
/// device task, within [`HANDLER_BUDGET`]: past it, the host drops the
/// future and shows an alert. Handlers must therefore be **cancel-safe**
/// (leave the instance consistent at every `.await`) and quick.
///
/// # Never block
///
/// Neither getters nor handlers may block the thread (blocking I/O, sleeps,
/// waiting on a lock held elsewhere, heavy computation): the budget can only
/// act at the next `.await`, and a blocked thread freezes the device. Long
/// work belongs in a background task spawned through
/// [`ModuleContext::tasks`], which reports back through
/// [`ModuleContext::events`] (state changes) or
/// [`crate::RedrawHandle::request_redraw`].
///
/// # Failures
///
/// An instance that panics, in a getter or a handler, is replaced by a
/// placeholder until the configuration is applied again.
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

    /// Handles a press of item `item` of page `page`, as it was displayed
    /// when the key was pressed.
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

    /// Handles an event sent through [`ModuleContext::events`], usually by a
    /// background task of the instance; downcast it to the type sent.
    async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
        let _ = event;
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
    /// Called only when the settings changed, after
    /// [`ModuleFactory::validate_settings`] accepted them: an error here is
    /// exceptional.
    ///
    /// # Errors
    ///
    /// [`ModuleError::InvalidSettings`]; the instance then keeps its previous
    /// settings.
    fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError>;
}
