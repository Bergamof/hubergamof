//! One WebSocket client of the UI.
//!
//! On connection the client receives `hello`, `devices` and `moduleCatalog`,
//! then `devices` after every attach or detach. `selectDevice` subscribes it
//! to one device (the previous one is dropped): `config` right away, then on
//! every change; with `liveState: true`, `liveState` too, sent at most every
//! [`MIN_LIVE_STATE_INTERVAL`]. The subscription ends with the engine
//! (device detached); the UI selects again from the new `devices` list.

use std::sync::Arc;
use std::time::Duration;

use super::errors::client_message;
use super::mouse_feed::{Failure, MouseFeed};
use axum::extract::ws::{CloseFrame, Message, WebSocket, close_code};
use hubergamof_protocol::{
    ApplicationModuleView, ClientMessage, DeviceConfig, DeviceInfo, ErrorCode, MachineConfig,
    ModuleKind, ServerMessage, UpdateSettings, UpdateStatus,
};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::time::Instant;

use super::{AppState, SimulatedInput};
use crate::device::DiscoveryEvent;
use crate::engine::{EngineError, EngineHandle, EngineRegistry, LiveSnapshot};
use crate::machine::MachineSnapshot;
use crate::module_host::{ModuleHost, ModuleViews, RadialReply, SettingsRefusal};

/// Shortest delay between two `liveState` messages to a client: bursts of
/// redraws (dial rotation, scrolling) are coalesced into the latest state.
pub const MIN_LIVE_STATE_INTERVAL: Duration = Duration::from_millis(100);

/// Longest wait for the engine of a device that was just attached.
pub const ENGINE_START_TIMEOUT: Duration = Duration::from_secs(5);

/// Device the client follows.
enum Selection {
    None,
    /// The device is attached but its engine is still starting.
    Starting {
        serial: String,
        live_state: bool,
        deadline: Instant,
    },
    Active(Subscription),
}

struct Subscription {
    serial: String,
    config: watch::Receiver<DeviceConfig>,
    /// `None` when the client did not ask for live states.
    live: Option<LiveFeed>,
}

/// Live states streamed to a client that asked for them.
struct LiveFeed {
    receiver: watch::Receiver<LiveSnapshot>,
    /// A live state newer than the last one sent is waiting for `next`.
    pending: bool,
    next: Instant,
}

/// What woke the session up.
enum Event {
    Client(Option<Result<Message, axum::Error>>),
    Discovery(Result<DiscoveryEvent, RecvError>),
    EnginesChanged,
    EngineStartTimedOut,
    ConfigChanged(Result<(), watch::error::RecvError>),
    LiveChanged(Result<(), watch::error::RecvError>),
    LiveDue,
    UpdateChanged(Result<(), watch::error::RecvError>),
    SettingsChanged(Result<(), watch::error::RecvError>),
    ApplicationChanged,
    MachineChanged(Arc<MachineSnapshot>),
    LockChanged,
    Mouse(Option<ServerMessage>),
    Shutdown,
}

/// Why the session ends.
struct Closed;

/// Whether `request` is refused while the computer is locked: everything
/// but the keep-alive. Following a device or a mouse is refused too, it shows
/// their configuration: the interface is not to be used then.
fn acts(request: &ClientMessage) -> bool {
    !matches!(request, ClientMessage::Ping)
}

/// What a locked computer still lets the client hear: that it is locked, the
/// answer to its keep-alive, why a request was refused, and the greeting.
fn heard_while_locked(message: &ServerMessage) -> bool {
    matches!(
        message,
        ServerMessage::Hello { .. }
            | ServerMessage::Locked { .. }
            | ServerMessage::Pong
            | ServerMessage::Error { .. }
    )
}

/// Resolves when the application shows something else, never without one.
async fn application_changed(views: Option<&mut watch::Receiver<ModuleViews>>) {
    if let Some(views) = views
        && views.changed().await.is_ok()
    {
        return;
    }
    std::future::pending::<()>().await;
}

/// Resolves with the next snapshot of the machine, never when the hub is gone
/// (a client that missed some is told the latest one).
async fn machine_changed(
    changes: &mut broadcast::Receiver<Arc<MachineSnapshot>>,
) -> Arc<MachineSnapshot> {
    loop {
        match changes.recv().await {
            Ok(snapshot) => return snapshot,
            Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => std::future::pending::<()>().await,
        }
    }
}

/// Serves one client until it disconnects or the server shuts down.
pub(super) async fn run(socket: WebSocket, state: AppState) {
    let (devices, discovery) = state.context.discovery.subscribe();
    let engine_changes = state.context.engines.changes();
    let shutdown = state.shutdown.clone();
    let updates = state.context.updates.subscribe();
    let update_settings = state.context.updates.subscribe_settings();
    let mouse = MouseFeed::new(&state.context.mice);
    let application = state.context.application.as_ref().map(ModuleHost::views);
    let machine = state.context.machine.subscribe();
    let locked = state.context.screen_lock.watch();
    let mut session = Session {
        socket,
        state,
        discovery,
        engine_changes,
        updates,
        update_settings,
        mouse,
        application,
        told_modules: Vec::new(),
        machine,
        locked,
        shutdown,
        selection: Selection::None,
    };
    let _ = session.serve(devices).await;
}

struct Session {
    socket: WebSocket,
    state: AppState,
    discovery: broadcast::Receiver<DiscoveryEvent>,
    engine_changes: watch::Receiver<()>,
    updates: watch::Receiver<UpdateStatus>,
    update_settings: watch::Receiver<UpdateSettings>,
    mouse: MouseFeed,
    /// What the modules of the application show, when the manager has them.
    application: Option<watch::Receiver<ModuleViews>>,
    /// What the client was last told of them: the views also change for what
    /// the interface does not show (the pages), which is not worth a message.
    told_modules: Vec<ApplicationModuleView>,
    /// The machine as updates leave it, from any client.
    machine: broadcast::Receiver<Arc<MachineSnapshot>>,
    /// Whether the computer is locked.
    locked: watch::Receiver<bool>,
    shutdown: watch::Receiver<bool>,
    selection: Selection,
}

impl Session {
    async fn serve(&mut self, devices: Vec<DeviceInfo>) -> Result<(), Closed> {
        self.send(&ServerMessage::Hello {
            version: env!("CARGO_PKG_VERSION").to_owned(),
        })
        .await?;
        self.send(&ServerMessage::Devices { devices }).await?;
        self.send(&ServerMessage::ModuleCatalog {
            modules: self.state.context.catalog.to_vec(),
        })
        .await?;
        self.send_locked().await?;
        self.send_application_modules().await?;
        for message in self.mouse.initial() {
            self.send(&message).await?;
        }
        self.send_update_settings().await?;
        if *self.updates.borrow() != UpdateStatus::Unknown {
            self.send_update_status().await?;
        }
        loop {
            match self.next_event().await {
                Event::Client(Some(Ok(message))) => self.on_client_message(message).await?,
                Event::Client(None | Some(Err(_))) => return Err(Closed),
                Event::Discovery(event) => self.on_discovery_event(event).await?,
                Event::EnginesChanged => self.resolve_starting_engine().await?,
                Event::EngineStartTimedOut => self.on_engine_start_timeout().await?,
                Event::ConfigChanged(Ok(())) => self.send_config().await?,
                Event::LiveChanged(Ok(())) => self.mark_live_pending(),
                Event::ConfigChanged(Err(_)) | Event::LiveChanged(Err(_)) => {
                    // The engine stopped: the device was detached.
                    self.selection = Selection::None;
                }
                Event::LiveDue => self.send_live_state().await?,
                Event::Mouse(Some(message)) => self.send(&message).await?,
                Event::UpdateChanged(Ok(())) => self.send_update_status().await?,
                Event::SettingsChanged(Ok(())) => self.send_update_settings().await?,
                Event::ApplicationChanged => self.send_application_modules_if_changed().await?,
                Event::MachineChanged(snapshot) => self.send_machine(&snapshot).await?,
                Event::LockChanged => {
                    self.send_locked().await?;
                    if !*self.locked.borrow() {
                        self.resync().await?;
                    }
                }
                // Unreachable while the context holds its handle (it keeps the
                // channels open); harmless if it ever happens. A mouse feed with
                // nothing to say: the mouse followed went away, the list of mice
                // tells the client.
                Event::SettingsChanged(Err(_))
                | Event::UpdateChanged(Err(_))
                | Event::Mouse(None) => {}
                Event::Shutdown => {
                    self.close().await;
                    return Err(Closed);
                }
            }
        }
    }

    async fn next_event(&mut self) -> Event {
        let (starting_deadline, config, live, live_due) = match &mut self.selection {
            Selection::None => (None, None, None, None),
            Selection::Starting { deadline, .. } => (Some(*deadline), None, None, None),
            Selection::Active(subscription) => {
                let (live, live_due) = match &mut subscription.live {
                    Some(feed) => {
                        let due = feed.pending.then_some(feed.next);
                        ((!feed.pending).then_some(&mut feed.receiver), due)
                    }
                    None => (None, None),
                };
                (None, Some(&mut subscription.config), live, live_due)
            }
        };
        tokio::select! {
            message = self.socket.recv() => Event::Client(message),
            event = self.discovery.recv() => Event::Discovery(event),
            // A dropped sender also means shutting down.
            _ = self.shutdown.wait_for(|stopping| *stopping) => Event::Shutdown,
            Ok(()) = changed_or_pending(starting_deadline.map(|_| &mut self.engine_changes)) => {
                Event::EnginesChanged
            }
            () = sleep_until_some(starting_deadline) => Event::EngineStartTimedOut,
            result = changed_or_pending(config) => Event::ConfigChanged(result),
            result = changed_or_pending(live) => Event::LiveChanged(result),
            () = sleep_until_some(live_due) => Event::LiveDue,
            result = self.updates.changed() => Event::UpdateChanged(result),
            result = self.update_settings.changed() => Event::SettingsChanged(result),
            message = self.mouse.next_change() => Event::Mouse(message),
            () = application_changed(self.application.as_mut()) => Event::ApplicationChanged,
            snapshot = machine_changed(&mut self.machine) => Event::MachineChanged(snapshot),
            Ok(()) = self.locked.changed() => Event::LockChanged,
        }
    }

    async fn on_client_message(&mut self, message: Message) -> Result<(), Closed> {
        let text = match message {
            Message::Text(text) => text,
            Message::Binary(_) => {
                return self
                    .send_error(
                        ErrorCode::InvalidMessage,
                        "binary messages are not supported",
                    )
                    .await;
            }
            Message::Close(_) => return Err(Closed),
            // Pings are answered by the WebSocket layer.
            Message::Ping(_) | Message::Pong(_) => return Ok(()),
        };
        match serde_json::from_str::<ClientMessage>(text.as_str()) {
            Ok(request) => self.on_request(request).await,
            Err(error) => {
                self.send_error(ErrorCode::InvalidMessage, &error.to_string())
                    .await
            }
        }
    }

    async fn on_request(&mut self, request: ClientMessage) -> Result<(), Closed> {
        // Nothing is done from the interface while the computer is locked: it
        // is not to be used then, and must not be a way to act on the computer.
        if acts(&request) && *self.locked.borrow() {
            return self
                .send_error(
                    ErrorCode::Locked,
                    "the computer is locked: nothing is done from the interface",
                )
                .await;
        }
        let engines = &self.state.context.engines;
        let outcome = match request {
            ClientMessage::SelectDevice { serial, live_state } => {
                return self.select(serial, live_state).await;
            }
            ClientMessage::Ping => return self.send(&ServerMessage::Pong).await,
            ClientMessage::CheckForUpdates => {
                self.state.context.updates.check_now();
                return Ok(());
            }
            ClientMessage::InstallUpdate => {
                self.state.context.updates.install_now();
                return Ok(());
            }
            ClientMessage::SetAutoInstall { enabled } => {
                self.state.context.updates.set_auto_install(enabled);
                return Ok(());
            }
            ClientMessage::UpdateConfig { serial, config } => {
                return self.update_config(serial, config).await;
            }
            ClientMessage::ModuleAction { module, action } => {
                return self.module_action(&module, &action).await;
            }
            ClientMessage::UpdateModuleSettings { module, settings } => {
                return self.update_module_settings(&module, &settings).await;
            }
            ClientMessage::RefreshMachine => return self.refresh_machine().await,
            ClientMessage::UpdateMachine { config } => return self.update_machine(config).await,
            ClientMessage::SelectMouse { serial } => {
                // The configuration, then what the mouse refused of it.
                let selected = self.mouse.select(&serial).map(|config| {
                    let mut messages = vec![config];
                    messages.extend(self.mouse.status());
                    messages
                });
                return self.answer_mouse(selected).await;
            }
            ClientMessage::UpdateMouseConfig { serial, config } => {
                let saved = self.mouse.update_mouse(serial, config).await;
                return self.answer_mouse(saved).await;
            }
            ClientMessage::PlayHaptic { serial, waveform } => {
                let played = self.mouse.play_haptic(&serial, waveform);
                return self.answer_mouse(played).await;
            }
            ClientMessage::UpdateRadialMenu { config } => {
                let saved = self.mouse.update_radial_menu(config).await;
                return self.answer_mouse(saved).await;
            }
            ClientMessage::PressKey { .. }
            | ClientMessage::RotateDial { .. }
            | ClientMessage::PressDial { .. }
                if self.state.context.simulated_input == SimulatedInput::Refused =>
            {
                Err(RequestFailure::input_refused())
            }
            ClientMessage::PressKey { serial, index } => {
                with_engine(engines, &serial, async |engine| {
                    Ok(engine.press_key(input_index(index)?).await?)
                })
                .await
            }
            ClientMessage::RotateDial {
                serial,
                index,
                ticks,
            } => {
                with_engine(engines, &serial, async |engine| {
                    Ok(engine.rotate_dial(input_index(index)?, ticks).await?)
                })
                .await
            }
            ClientMessage::PressDial { serial, index } => {
                with_engine(engines, &serial, async |engine| {
                    Ok(engine.press_dial(input_index(index)?).await?)
                })
                .await
            }
        };
        match outcome {
            Ok(()) => Ok(()),
            Err(failure) => self.send_error(failure.code, &failure.message).await,
        }
    }

    /// Runs an action of a module of the application, the same ones the radial
    /// menu runs: only those the module offers.
    async fn module_action(&mut self, module: &ModuleKind, action: &str) -> Result<(), Closed> {
        let Some(host) = self.state.context.application.clone() else {
            return self
                .send_error(ErrorCode::Unavailable, "the application holds no modules")
                .await;
        };
        let offered = self
            .state
            .context
            .catalog
            .iter()
            .find(|entry| entry.descriptor.kind == *module)
            .map(|entry| {
                entry
                    .radial_actions
                    .iter()
                    .any(|offered| offered.id == action)
            });
        let (code, message) = match offered {
            None => (ErrorCode::Unavailable, "no such module"),
            Some(false) => (ErrorCode::InvalidInput, "the module offers no such action"),
            Some(true) => match host.radial_action(module, action).await {
                RadialReply::Done => return Ok(()),
                RadialReply::NotHeld => (
                    ErrorCode::Unavailable,
                    "the application holds no such module",
                ),
                RadialReply::Refused => (ErrorCode::Locked, "the computer is locked"),
            },
        };
        self.send_error(code, message).await
    }

    /// Saves the settings of a module of the application; every client is told
    /// by the new state of the module.
    async fn update_module_settings(
        &mut self,
        module: &ModuleKind,
        settings: &serde_json::Value,
    ) -> Result<(), Closed> {
        let Some(host) = self.state.context.application.clone() else {
            return self
                .send_error(ErrorCode::Unavailable, "the application holds no modules")
                .await;
        };
        match host.set_settings(module, settings).await {
            // Told to the author even when nothing changed (the other clients
            // hear it from the change): it is how the interface knows the
            // settings were taken.
            Ok(()) => self.send_application_modules().await,
            Err(SettingsRefusal::NotHeld) => {
                self.send_error(
                    ErrorCode::Unavailable,
                    "the application holds no such module",
                )
                .await
            }
            Err(SettingsRefusal::Refused(reason)) => {
                self.send_error(ErrorCode::InvalidConfig, &reason).await
            }
            Err(SettingsRefusal::Locked) => {
                self.send_error(ErrorCode::Locked, "the computer is locked")
                    .await
            }
            Err(SettingsRefusal::NotSaved) => {
                self.send_error(ErrorCode::Internal, "the settings could not be saved")
                    .await
            }
        }
    }

    /// Reads the machine now and tells this client only.
    async fn refresh_machine(&mut self) -> Result<(), Closed> {
        match self.state.context.machine.refresh().await {
            Ok(snapshot) => self.send_machine(&snapshot).await,
            Err(error) => {
                self.send_error(error.error_code(), &error.to_string())
                    .await
            }
        }
    }

    /// Saves and applies a configuration of the machine. Every client, this
    /// one included, hears the result as the machine changes; only a failure
    /// is answered here.
    async fn update_machine(&mut self, config: MachineConfig) -> Result<(), Closed> {
        match self.state.context.machine.update(config).await {
            Ok(()) => Ok(()),
            Err(error) => {
                self.send_error(error.error_code(), &error.to_string())
                    .await
            }
        }
    }

    async fn send_machine(&mut self, snapshot: &MachineSnapshot) -> Result<(), Closed> {
        self.send(&ServerMessage::Machine {
            config: snapshot.config.clone(),
            system_aliases: snapshot.system_aliases.clone(),
        })
        .await
    }

    /// What the modules of the application show now, if the manager has any.
    async fn send_application_modules(&mut self) -> Result<(), Closed> {
        let Some(modules) = self.application_modules() else {
            return Ok(());
        };
        self.told_modules.clone_from(&modules);
        self.send(&ServerMessage::ApplicationModules { modules })
            .await
    }

    /// The same, unless the client already knows it.
    async fn send_application_modules_if_changed(&mut self) -> Result<(), Closed> {
        match self.application_modules() {
            Some(modules) if modules != self.told_modules => {
                self.told_modules.clone_from(&modules);
                self.send(&ServerMessage::ApplicationModules { modules })
                    .await
            }
            _ => Ok(()),
        }
    }

    /// What the interface shows of the modules of the application now.
    fn application_modules(&mut self) -> Option<Vec<ApplicationModuleView>> {
        let views = self.application.as_mut()?;
        let modules = views
            .borrow_and_update()
            .iter()
            .map(|(kind, view)| ApplicationModuleView {
                kind: kind.clone(),
                status: view.radial_status.clone(),
                settings: view.settings.clone(),
            })
            .collect();
        Some(modules)
    }

    /// Tells the client what it could not hear while the computer was locked.
    /// The interface selects its device and mouse again on the lists, like
    /// after any other change of them.
    async fn resync(&mut self) -> Result<(), Closed> {
        let devices = self.state.context.discovery.devices();
        self.send(&ServerMessage::Devices { devices }).await?;
        self.send(&ServerMessage::ModuleCatalog {
            modules: self.state.context.catalog.to_vec(),
        })
        .await?;
        self.send_application_modules().await?;
        for message in self.mouse.initial() {
            self.send(&message).await?;
        }
        self.send_update_settings().await?;
        if *self.updates.borrow() != UpdateStatus::Unknown {
            self.send_update_status().await?;
        }
        self.send_config().await?;
        self.send_live_state().await
    }

    async fn send_locked(&mut self) -> Result<(), Closed> {
        let locked = *self.locked.borrow_and_update();
        self.send(&ServerMessage::Locked { locked }).await
    }

    /// Applies a configuration sent by the client and answers with the saved one.
    ///
    /// Sent back to the author even if a later update overtakes it in the
    /// engine's `config` channel (which keeps the latest only): the UI
    /// recognises its own update by it. Other subscribers get it through the
    /// channel.
    async fn update_config(&mut self, serial: String, config: DeviceConfig) -> Result<(), Closed> {
        let engines = &self.state.context.engines;
        let saved = with_engine(engines, &serial, async |engine| {
            Ok(engine.apply_config(config).await?)
        })
        .await;
        match saved {
            Ok(config) => {
                let newer = self.take_newer_config(&serial, &config);
                self.send(&ServerMessage::Config {
                    serial: serial.clone(),
                    config,
                })
                .await?;
                match newer {
                    Some(config) => self.send(&ServerMessage::Config { serial, config }).await,
                    None => Ok(()),
                }
            }
            Err(failure) => self.send_error(failure.code, &failure.message).await,
        }
    }

    /// Sends what a mouse request answers, or the failure.
    async fn answer_mouse(
        &mut self,
        answer: Result<Vec<ServerMessage>, Failure>,
    ) -> Result<(), Closed> {
        match answer {
            Ok(messages) => {
                for message in messages {
                    self.send(&message).await?;
                }
                Ok(())
            }
            Err(failure) => self.send_error(failure.code, &failure.message).await,
        }
    }

    async fn select(&mut self, serial: String, live_state: bool) -> Result<(), Closed> {
        self.selection = Selection::None;
        // Seen before the lookup: an engine registered in between still
        // wakes `next_event` up.
        self.engine_changes.borrow_and_update();
        if let Some(engine) = self.state.context.engines.get(&serial) {
            return self.subscribe(&engine, live_state).await;
        }
        if self.state.context.discovery.backend(&serial).is_some() {
            self.selection = Selection::Starting {
                serial,
                live_state,
                deadline: Instant::now() + ENGINE_START_TIMEOUT,
            };
            return Ok(());
        }
        self.send_error(
            ErrorCode::UnknownDevice,
            &RequestFailure::unknown_device(&serial).message,
        )
        .await
    }

    async fn subscribe(&mut self, engine: &EngineHandle, live_state: bool) -> Result<(), Closed> {
        self.selection = Selection::Active(Subscription {
            serial: engine.info().serial.clone(),
            config: engine.config(),
            live: live_state.then(|| LiveFeed {
                receiver: engine.live(),
                pending: false,
                next: Instant::now(),
            }),
        });
        self.send_config().await?;
        self.send_live_state().await
    }

    async fn resolve_starting_engine(&mut self) -> Result<(), Closed> {
        let Selection::Starting {
            serial, live_state, ..
        } = &self.selection
        else {
            return Ok(());
        };
        let live_state = *live_state;
        match self.state.context.engines.get(serial) {
            Some(engine) => self.subscribe(&engine, live_state).await,
            None => Ok(()),
        }
    }

    async fn on_engine_start_timeout(&mut self) -> Result<(), Closed> {
        let Selection::Starting { serial, .. } =
            std::mem::replace(&mut self.selection, Selection::None)
        else {
            return Ok(());
        };
        self.send_error(
            ErrorCode::UnknownDevice,
            &format!("device {serial} did not start"),
        )
        .await
    }

    async fn on_discovery_event(
        &mut self,
        event: Result<DiscoveryEvent, RecvError>,
    ) -> Result<(), Closed> {
        match event {
            Ok(DiscoveryEvent::Detached(serial)) => {
                if matches!(&self.selection, Selection::Starting { serial: waiting, .. } if *waiting == serial)
                {
                    self.selection = Selection::None;
                }
            }
            // Missed events: the list below is current anyway.
            Ok(DiscoveryEvent::Attached(_)) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return Err(Closed),
        }
        let devices = self.state.context.discovery.devices();
        self.send(&ServerMessage::Devices { devices }).await
    }

    fn mark_live_pending(&mut self) {
        if let Selection::Active(Subscription {
            live: Some(feed), ..
        }) = &mut self.selection
        {
            feed.pending = true;
        }
    }

    /// Consumes the channel's latest configuration of `serial`, so that the
    /// client does not get `saved` twice; returns it when a later update
    /// already overtook `saved`, to be sent after it.
    fn take_newer_config(&mut self, serial: &str, saved: &DeviceConfig) -> Option<DeviceConfig> {
        let Selection::Active(subscription) = &mut self.selection else {
            return None;
        };
        if subscription.serial != serial {
            return None;
        }
        let latest = subscription.config.borrow_and_update();
        (*latest != *saved).then(|| latest.clone())
    }

    async fn send_config(&mut self) -> Result<(), Closed> {
        let Selection::Active(subscription) = &mut self.selection else {
            return Ok(());
        };
        let message = ServerMessage::Config {
            serial: subscription.serial.clone(),
            config: subscription.config.borrow_and_update().clone(),
        };
        self.send(&message).await
    }

    async fn send_live_state(&mut self) -> Result<(), Closed> {
        let Selection::Active(Subscription {
            live: Some(feed), ..
        }) = &mut self.selection
        else {
            return Ok(());
        };
        let snapshot = feed.receiver.borrow_and_update().clone();
        feed.pending = false;
        feed.next = Instant::now() + MIN_LIVE_STATE_INTERVAL;
        // Encoding new images as PNG data URLs is CPU-bound.
        match tokio::task::spawn_blocking(move || snapshot.to_live_state()).await {
            Ok(Ok(state)) => self.send(&ServerMessage::LiveState { state }).await,
            Ok(Err(error)) => {
                tracing::error!(%error, "cannot encode the live state");
                self.send_error(ErrorCode::Internal, &error.to_string())
                    .await
            }
            Err(error) => {
                tracing::error!(%error, "live state encoding task failed");
                self.send_error(ErrorCode::Internal, &error.to_string())
                    .await
            }
        }
    }

    async fn send_error(&mut self, code: ErrorCode, message: &str) -> Result<(), Closed> {
        self.send(&ServerMessage::Error {
            code,
            message: client_message(code, message, "WebSocket request"),
        })
        .await
    }

    async fn send_update_settings(&mut self) -> Result<(), Closed> {
        let settings = *self.update_settings.borrow_and_update();
        self.send(&ServerMessage::UpdateSettings { settings }).await
    }

    async fn send_update_status(&mut self) -> Result<(), Closed> {
        let status = self.updates.borrow_and_update().clone();
        self.send(&ServerMessage::UpdateStatus { status }).await
    }

    async fn send(&mut self, message: &ServerMessage) -> Result<(), Closed> {
        // Nothing of the configuration or of the state is told while the
        // computer is locked: what was missed is told again at the unlocking.
        if *self.locked.borrow() && !heard_while_locked(message) {
            return Ok(());
        }
        let text = match serde_json::to_string(message) {
            Ok(text) => text,
            Err(error) => {
                tracing::error!(%error, "cannot serialise a server message");
                return Ok(());
            }
        };
        self.socket
            .send(Message::Text(text.into()))
            .await
            .map_err(|_| Closed)
    }

    async fn close(&mut self) {
        let frame = CloseFrame {
            code: close_code::AWAY,
            reason: "server shutting down".into(),
        };
        let _ = self.socket.send(Message::Close(Some(frame))).await;
    }
}

/// Failure of a client request, reported as `error`.
struct RequestFailure {
    code: ErrorCode,
    message: String,
}

impl RequestFailure {
    fn input_refused() -> Self {
        Self {
            code: ErrorCode::InputRefused,
            message: "presses come from the device only (simulated input needs --demo)".into(),
        }
    }

    fn unknown_device(serial: &str) -> Self {
        Self {
            code: ErrorCode::UnknownDevice,
            message: format!("unknown device {serial}"),
        }
    }
}

/// Failure of an engine request, before reporting.
enum RequestError {
    /// The key or dial index does not fit the device protocol (`u8`).
    IndexOutOfRange(u32),
    Engine(EngineError),
}

impl From<EngineError> for RequestError {
    fn from(error: EngineError) -> Self {
        Self::Engine(error)
    }
}

impl From<RequestError> for RequestFailure {
    fn from(error: RequestError) -> Self {
        match error {
            RequestError::IndexOutOfRange(index) => Self {
                code: ErrorCode::InvalidInput,
                message: format!("no input with index {index}"),
            },
            RequestError::Engine(error) => Self {
                code: error.error_code(),
                message: error.to_string(),
            },
        }
    }
}

/// Runs `action` on the engine of `serial`.
async fn with_engine<T>(
    engines: &EngineRegistry,
    serial: &str,
    action: impl AsyncFnOnce(EngineHandle) -> Result<T, RequestError>,
) -> Result<T, RequestFailure> {
    let engine = engines
        .get(serial)
        .ok_or_else(|| RequestFailure::unknown_device(serial))?;
    action(engine).await.map_err(RequestFailure::from)
}

fn input_index(index: u32) -> Result<u8, RequestError> {
    u8::try_from(index).map_err(|_| RequestError::IndexOutOfRange(index))
}

async fn changed_or_pending<T>(
    receiver: Option<&mut watch::Receiver<T>>,
) -> Result<(), watch::error::RecvError> {
    match receiver {
        Some(receiver) => receiver.changed().await,
        None => std::future::pending().await,
    }
}

async fn sleep_until_some(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}
