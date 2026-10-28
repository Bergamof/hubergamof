//! API implemented by Bergadeck modules.
//!
//! A module drives part of a Stream Deck: the key that represents it on the
//! home page, the pages of keys it opens, the dial it may be assigned to and
//! the touch strip zone above that dial. Built-in modules
//! (`bergadeck-modules`) use this API exactly like future external modules
//! will.
//!
//! This crate is licensed under the MPL-2.0 so that third-party modules are
//! not bound by the GPL of the manager; it depends on no GPL code.
//!
//! # Model
//!
//! - A [`ModuleFactory`] describes a module kind ([`ModuleDescriptor`], shown
//!   in the configuration UI) and creates one [`Module`] per configured
//!   instance.
//! - The host owns the instance, calls its input handlers and reads its
//!   surfaces ([`Module::key`], [`Module::page`], [`Module::zone`]) whenever
//!   it redraws. Handlers return [`Command`]s to navigate.
//! - State changing on its own (clock, timer, watcher) is pushed with the
//!   [`RedrawHandle`] of the [`ModuleContext`], typically from a background
//!   task spawned through [`ModuleContext::tasks`]; such tasks are aborted
//!   when the instance is destroyed.
//!
//! # Example
//!
//! A counter: its key shows how many times it was pressed, its dial changes
//! the count and a setting chooses the step.
//!
//! ```
//! use async_trait::async_trait;
//! use bergadeck_module_api::{
//!     Command, KeyContent, Module, ModuleCapabilities, ModuleContext, ModuleDescriptor,
//!     ModuleError, ModuleFactory, RedrawHandle, SettingField, SettingKind, TaskScope,
//!     parse_settings,
//! };
//! use serde::Deserialize;
//! use serde_json::{Value, json};
//!
//! #[derive(Deserialize)]
//! struct Settings {
//!     step: i64,
//! }
//!
//! struct Counter {
//!     count: i64,
//!     step: i64,
//! }
//!
//! #[async_trait]
//! impl Module for Counter {
//!     fn key(&self) -> KeyContent {
//!         KeyContent::with_label(self.count.to_string())
//!     }
//!
//!     async fn on_key_press(&mut self) -> Vec<Command> {
//!         self.count += self.step;
//!         Vec::new()
//!     }
//!
//!     async fn on_dial_rotate(&mut self, ticks: i32) -> Vec<Command> {
//!         self.count += i64::from(ticks) * self.step;
//!         Vec::new()
//!     }
//!
//!     fn update_settings(&mut self, settings: &Value) -> Result<(), ModuleError> {
//!         // The count survives a settings change.
//!         self.step = parse_settings::<Settings>(settings)?.step;
//!         Ok(())
//!     }
//! }
//!
//! struct CounterFactory;
//!
//! impl ModuleFactory for CounterFactory {
//!     fn descriptor(&self) -> ModuleDescriptor {
//!         ModuleDescriptor {
//!             kind: "counter".into(),
//!             name: "Compteur".into(),
//!             description: "Compte les appuis.".into(),
//!             capabilities: ModuleCapabilities { key: true, dial: true, touch_zone: false },
//!             settings: vec![SettingField {
//!                 key: "step".into(),
//!                 label: "Pas".into(),
//!                 help: None,
//!                 kind: SettingKind::Integer { default: 1, min: Some(1), max: None, unit: None },
//!             }],
//!         }
//!     }
//!
//!     fn create(&self, _ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
//!         let Settings { step } = parse_settings(settings)?;
//!         Ok(Box::new(Counter { count: 0, step }))
//!     }
//! }
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), ModuleError> {
//! // What the host does with it:
//! let factory = CounterFactory;
//! assert_eq!(factory.default_settings(), json!({ "step": 1 }));
//! let scope = TaskScope::new();
//! let ctx = ModuleContext::new("counter".into(), RedrawHandle::noop(), scope.spawner());
//! let mut counter = factory.create(ctx, &factory.default_settings())?;
//! counter.on_key_press().await;
//! counter.update_settings(&json!({ "step": 10 }))?;
//! counter.on_dial_rotate(2).await;
//! assert_eq!(counter.key().label.as_deref(), Some("21"));
//! # Ok(())
//! # }
//! ```

mod catalog;
mod command;
mod content;
mod context;
mod ids;
mod module;

pub use catalog::{ChoiceOption, ModuleCapabilities, ModuleDescriptor, SettingField, SettingKind};
pub use command::Command;
pub use content::{
    ImageData, ImageFormat, KeyContent, MAX_ZONE_LINES, NavigationView, PageContent, PageItem,
    ZoneContent,
};
pub use context::{ModuleContext, RedrawHandle, TaskScope, TaskSpawner};
pub use ids::{ItemId, ModuleInstanceId, ModuleKind, PageId};
pub use module::{Module, ModuleError, ModuleFactory, parse_settings};
