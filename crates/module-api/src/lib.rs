//! API implemented by HuBergamof modules.
//!
//! A module drives part of a Stream Deck: the key that represents it on the
//! home page, the pages of keys it opens, the dial it may be assigned to and
//! the touch strip zone above that dial. Built-in modules
//! (`hubergamof-modules`) use this API exactly like future external modules
//! will.
//!
//! This crate is licensed under the MPL-2.0 so that third-party modules are
//! not bound by the GPL of the manager; it depends on no GPL code.
//!
//! # Model
//!
//! - A [`ModuleFactory`] describes a module kind ([`ModuleDescriptor`], shown
//!   in the configuration UI), validates its settings and creates one
//!   [`Module`] per configured instance.
//! - The host owns the instance, calls its input handlers and reads its
//!   surfaces ([`Module::key`], [`Module::page`], [`Module::zone`]) whenever
//!   it redraws. Handlers return [`Command`]s to navigate or show the outcome
//!   of an action ([`Feedback`]).
//! - Work that takes time (I/O, processes, timers) runs in background tasks
//!   spawned through [`ModuleContext::tasks`], aborted when the instance is
//!   destroyed. They report to the instance with [`ModuleContext::events`]
//!   ([`Module::on_event`] then runs on the device task and may change the
//!   state), or only ask for a redraw with [`ModuleContext::redraw`].
//!
//! [`Module`] is the in-process API; future external modules (WASM or
//! sub-process) will be driven through an adapter implementing it.
//!
//! # Example
//!
//! A counter: its key shows the count, a press or the dial changes it, a
//! setting chooses the step and a background task resets it every hour.
//!
//! ```
//! use std::time::Duration;
//!
//! use async_trait::async_trait;
//! use hubergamof_module_api::{
//!     Command, EventSender, Feedback, KeyContent, Module, ModuleCapabilities, ModuleContext,
//!     ModuleDescriptor, ModuleError, ModuleEvent, ModuleFactory, SettingField, SettingKind,
//!     TaskScope, parse_settings,
//! };
//! use serde::Deserialize;
//! use serde_json::{Value, json};
//!
//! #[derive(Deserialize)]
//! struct Settings {
//!     step: i64,
//! }
//!
//! /// Sent by the background task.
//! struct Reset;
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
//!         vec![Command::Feedback(Feedback::Ok)]
//!     }
//!
//!     async fn on_dial_rotate(&mut self, ticks: i32) -> Vec<Command> {
//!         self.count += i64::from(ticks) * self.step;
//!         Vec::new()
//!     }
//!
//!     async fn on_event(&mut self, event: ModuleEvent) -> Vec<Command> {
//!         if event.is::<Reset>() {
//!             self.count = 0;
//!         }
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
//! async fn reset_hourly(events: EventSender) {
//!     loop {
//!         tokio::time::sleep(Duration::from_secs(3600)).await;
//!         if !events.send(Reset) {
//!             return;
//!         }
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
//!     fn create(&self, ctx: ModuleContext, settings: &Value) -> Result<Box<dyn Module>, ModuleError> {
//!         let Settings { step } = parse_settings(settings)?;
//!         ctx.tasks().spawn(reset_hourly(ctx.events().clone()));
//!         Ok(Box::new(Counter { count: 0, step }))
//!     }
//! }
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), ModuleError> {
//! // What the host does with it:
//! let factory = CounterFactory;
//! assert_eq!(factory.default_settings(), json!({ "step": 1 }));
//! assert!(factory.validate_settings(&json!({ "step": 0 })).is_err());
//! let scope = TaskScope::new();
//! let ctx = ModuleContext::for_tests("counter").with_tasks(scope.spawner());
//! let mut counter = factory.create(ctx, &factory.default_settings())?;
//! counter.on_key_press().await;
//! counter.update_settings(&json!({ "step": 10 }))?;
//! counter.on_dial_rotate(2).await;
//! assert_eq!(counter.key().label.as_deref(), Some("21"));
//! counter.on_event(Box::new(Reset)).await;
//! assert_eq!(counter.key().label.as_deref(), Some("0"));
//! # Ok(())
//! # }
//! ```

mod catalog;
mod command;
mod content;
mod context;
mod ids;
mod module;
mod paths;
#[cfg(feature = "testing")]
pub mod testing;

pub use catalog::{ChoiceOption, ModuleCapabilities, ModuleDescriptor, SettingField, SettingKind};
pub use command::{Command, Feedback, ModuleSurface};
pub use content::{
    CalendarLeaf, ClockFace, ClockSeconds, GaugeContent, GaugeLevel, ImageData, KeyContent,
    KeyState, MAX_ZONE_LINES, NavigationView, PageContent, PageItem, ZoneContent,
};
pub use context::{EventSender, ModuleContext, ModuleEvent, RedrawHandle, TaskScope, TaskSpawner};
pub use ids::{ItemId, ModuleInstanceId, ModuleKind, PageId};
pub use module::{HANDLER_BUDGET, Module, ModuleError, ModuleFactory, parse_settings};
pub use paths::{expand_home, is_setting_path};
