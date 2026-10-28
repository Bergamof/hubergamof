//! One WebSocket client of the UI.
//!
//! On connection the client receives `hello`, `devices` and `moduleCatalog`,
//! then `devices` after every attach or detach. `selectDevice` subscribes it
//! to one device (the previous one is dropped): `config` and `liveState`
//! right away, then on every change, live states being sent at most every
//! [`MIN_LIVE_STATE_INTERVAL`]. The subscription ends with the engine
//! (device detached); the UI selects again from the new `devices` list.

use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket, close_code};
use bergadeck_protocol::{ClientMessage, DeviceConfig, DeviceInfo, ErrorCode, ServerMessage};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::time::Instant;

use super::AppState;
use crate::device::DiscoveryEvent;
use crate::engine::{EngineError, EngineHandle, EngineRegistry, LiveSnapshot};

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
        deadline: Instant,
    },
    Active(Subscription),
}

struct Subscription {
    serial: String,
    config: watch::Receiver<DeviceConfig>,
    live: watch::Receiver<LiveSnapshot>,
    /// A live state newer than the last one sent is waiting for `next_live`.
    live_pending: bool,
    next_live: Instant,
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
    Shutdown,
}

/// Why the session ends.
struct Closed;

/// Serves one client until it disconnects or the server shuts down.
pub(super) async fn run(socket: WebSocket, state: AppState) {
    let (devices, discovery) = state.context.discovery.subscribe();
    let engine_changes = state.context.engines.changes();
    let shutdown = state.shutdown.clone();
    let mut session = Session {
        socket,
        state,
        discovery,
        engine_changes,
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
                let live_due = subscription.live_pending.then_some(subscription.next_live);
                let live = (!subscription.live_pending).then_some(&mut subscription.live);
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
        let engines = &self.state.context.engines;
        let outcome = match request {
            ClientMessage::SelectDevice { serial } => return self.select(serial).await,
            ClientMessage::Ping => return self.send(&ServerMessage::Pong).await,
            ClientMessage::UpdateConfig { serial, config } => {
                // Subscribers (this client included) get the saved
                // configuration through the engine's `config` channel.
                with_engine(engines, &serial, async |engine| {
                    engine.apply_config(config).await?;
                    Ok(())
                })
                .await
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

    async fn select(&mut self, serial: String) -> Result<(), Closed> {
        self.selection = Selection::None;
        // Seen before the lookup: an engine registered in between still
        // wakes `next_event` up.
        self.engine_changes.borrow_and_update();
        if let Some(engine) = self.state.context.engines.get(&serial) {
            return self.subscribe(&engine).await;
        }
        if self.state.context.discovery.backend(&serial).is_some() {
            self.selection = Selection::Starting {
                serial,
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

    async fn subscribe(&mut self, engine: &EngineHandle) -> Result<(), Closed> {
        self.selection = Selection::Active(Subscription {
            serial: engine.info().serial.clone(),
            config: engine.config(),
            live: engine.live(),
            live_pending: false,
            next_live: Instant::now(),
        });
        self.send_config().await?;
        self.send_live_state().await
    }

    async fn resolve_starting_engine(&mut self) -> Result<(), Closed> {
        let Selection::Starting { serial, .. } = &self.selection else {
            return Ok(());
        };
        match self.state.context.engines.get(serial) {
            Some(engine) => self.subscribe(&engine).await,
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
        if let Selection::Active(subscription) = &mut self.selection {
            subscription.live_pending = true;
        }
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
        let Selection::Active(subscription) = &mut self.selection else {
            return Ok(());
        };
        let snapshot = subscription.live.borrow_and_update().clone();
        subscription.live_pending = false;
        subscription.next_live = Instant::now() + MIN_LIVE_STATE_INTERVAL;
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
            message: message.to_owned(),
        })
        .await
    }

    async fn send(&mut self, message: &ServerMessage) -> Result<(), Closed> {
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
async fn with_engine(
    engines: &EngineRegistry,
    serial: &str,
    action: impl AsyncFnOnce(EngineHandle) -> Result<(), RequestError>,
) -> Result<(), RequestFailure> {
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
