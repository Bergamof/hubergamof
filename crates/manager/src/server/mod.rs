//! Local web server: the UI files and the `/ws` WebSocket of the contract
//! ([`bergadeck_protocol::ClientMessage`] / [`bergadeck_protocol::ServerMessage`]).
//!
//! - Listens on `127.0.0.1` only.
//! - Every request goes through the [`AccessPolicy`] (`Host`, and `Origin`
//!   on `/ws` and state-changing requests) and gets the security headers
//!   (CSP, `nosniff`, `no-referrer`, …).
//! - WebSocket messages are limited to [`MAX_WS_MESSAGE_BYTES`].
//! - [`Server::run`] stops gracefully: WebSocket clients are sent a close
//!   frame, then in-flight requests complete.

mod assets;
mod security;
mod session;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::WebSocketUpgrade;
use axum::middleware;
use axum::response::Response;
use axum::routing::get;
use bergadeck_protocol::ModuleDescriptor;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::watch;

pub use assets::{
    AssetSource, EmbeddedUi, FALLBACK_FILE, INDEX_FILE, MISSING_UI_MESSAGE,
    html_content_security_policy,
};
pub use security::{
    AccessDenied, AccessPolicy, CONTENT_SECURITY_POLICY_BASE, DevOrigin, InvalidDevOrigin,
};
pub use session::{ENGINE_START_TIMEOUT, MIN_LIVE_STATE_INTERVAL};

use crate::device::DeviceDiscovery;
use crate::engine::EngineRegistry;

/// Path of the WebSocket endpoint.
pub const WS_PATH: &str = "/ws";

/// Largest WebSocket message (and frame) accepted from a client. A full
/// configuration is a few kilobytes; images do not travel over `/ws`.
pub const MAX_WS_MESSAGE_BYTES: usize = 1024 * 1024;

/// Whether clients may simulate presses and rotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulatedInput {
    /// Accepted (`--demo`, virtual devices only).
    Accepted,
    /// Refused: on real devices, input comes from the device only.
    Refused,
}

/// What the server exposes.
#[derive(Clone)]
pub struct ServerContext {
    /// Attached devices.
    pub discovery: DeviceDiscovery,
    /// Running device engines.
    pub engines: EngineRegistry,
    /// Module kinds offered to the UI.
    pub catalog: Arc<[ModuleDescriptor]>,
    /// Whether clients may press keys and turn dials.
    pub simulated_input: SimulatedInput,
}

/// Shared by every request handler.
#[derive(Clone)]
struct AppState {
    context: ServerContext,
    assets: Arc<dyn AssetSource>,
    policy: Arc<AccessPolicy>,
    /// Becomes `true` when the server shuts down.
    shutdown: watch::Receiver<bool>,
}

/// Failure to start the server.
#[derive(Debug, Error)]
pub enum ServerError {
    /// The port is taken or cannot be bound.
    #[error("cannot listen on {address}: {source}")]
    Bind {
        /// Requested address.
        address: SocketAddr,
        /// Cause.
        source: std::io::Error,
    },
}

/// Routes of the server: `/ws`, then the UI for every other `GET`/`HEAD`.
///
/// `policy` must name the port the server listens on; `shutdown` closes the
/// WebSocket sessions when it becomes `true`.
pub fn router(
    context: ServerContext,
    assets: Arc<dyn AssetSource>,
    policy: AccessPolicy,
    shutdown: watch::Receiver<bool>,
) -> Router {
    let state = AppState {
        context,
        assets,
        policy: Arc::new(policy),
        shutdown,
    };
    Router::new()
        .route(WS_PATH, get(upgrade_websocket))
        .fallback_service(get(assets::serve_ui).with_state(state.clone()))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security::enforce_access_policy,
        ))
        .layer(middleware::from_fn(security::add_security_headers))
        .with_state(state)
}

async fn upgrade_websocket(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade
        .max_message_size(MAX_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| session::run(socket, state))
}

/// The server, bound and ready to run.
pub struct Server {
    listener: TcpListener,
    router: Router,
    address: SocketAddr,
    shutdown: watch::Receiver<bool>,
}

impl Server {
    /// Binds `127.0.0.1:port` (`0` picks a free port). The server stops when
    /// `shutdown` becomes `true` (or its sender is dropped).
    ///
    /// # Errors
    ///
    /// [`ServerError::Bind`] when the port is taken.
    pub async fn bind(
        port: u16,
        dev_origins: &[DevOrigin],
        context: ServerContext,
        assets: Arc<dyn AssetSource>,
        shutdown: watch::Receiver<bool>,
    ) -> Result<Self, ServerError> {
        let requested = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let bind_error = |source| ServerError::Bind {
            address: requested,
            source,
        };
        let listener = TcpListener::bind(requested).await.map_err(bind_error)?;
        let address = listener.local_addr().map_err(bind_error)?;
        let policy = AccessPolicy::new(address.port(), dev_origins);
        Ok(Self {
            router: router(context, assets, policy, shutdown.clone()),
            listener,
            address,
            shutdown,
        })
    }

    /// Address actually bound.
    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// URL of the web interface.
    #[must_use]
    pub fn url(&self) -> String {
        format!("http://{}/", self.address)
    }

    /// Serves until shutdown, then waits for the open connections (WebSocket
    /// sessions close themselves on the same signal).
    ///
    /// # Errors
    ///
    /// I/O failure of the listener.
    pub async fn run(self) -> std::io::Result<()> {
        let mut shutdown = self.shutdown;
        axum::serve(self.listener, self.router)
            .with_graceful_shutdown(async move {
                // A dropped sender also means shutting down.
                let _ = shutdown.wait_for(|stopping| *stopping).await;
            })
            .await
    }
}
