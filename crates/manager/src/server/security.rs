//! Who may talk to the server: `Host` and `Origin` checks, security headers.
//!
//! The server binds to `127.0.0.1` only, yet any web page the user visits can
//! still make the browser send requests to it. Two checks keep such pages
//! out, since the modules run commands on the machine:
//! - `Host` must name the server itself (`127.0.0.1:<port>`,
//!   `localhost:<port>`), which defeats DNS rebinding (a hostile domain
//!   resolved to `127.0.0.1` keeps its own name in `Host`);
//! - `Origin` must be the UI itself on `/ws` and on every non-`GET`/`HEAD`
//!   request (browsers do not apply the same-origin policy to `WebSocket`s).

use std::fmt;
use std::str::FromStr;

use axum::extract::{Request, State};
use axum::http::header::{
    CONTENT_SECURITY_POLICY, HOST, ORIGIN, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::uri::{Authority, Scheme};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

use super::{AppState, WS_PATH};

/// Content Security Policy of every response; HTML pages extend
/// `script-src` with the hashes of their inline scripts (see
/// module `assets`).
///
/// `style-src 'unsafe-inline'`: Svelte writes `style` attributes in the
/// prerendered pages (`<div style="display: contents">`).
pub const CONTENT_SECURITY_POLICY_BASE: &str = "default-src 'self'; \
     img-src 'self' data:; \
     style-src 'self' 'unsafe-inline'; \
     connect-src 'self'; \
     object-src 'none'; \
     base-uri 'none'; \
     form-action 'self'; \
     frame-ancestors 'none'";

const CROSS_ORIGIN_OPENER_POLICY: HeaderName =
    HeaderName::from_static("cross-origin-opener-policy");
const CROSS_ORIGIN_RESOURCE_POLICY: HeaderName =
    HeaderName::from_static("cross-origin-resource-policy");

/// Origin of a development server allowed to use the manager (`--dev-origin`),
/// e.g. Vite on `http://localhost:5173` proxying `/ws`.
///
/// Its authority is accepted as `Host` too: the Vite proxy forwards the
/// browser's `Host` unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevOrigin {
    origin: String,
    authority: String,
}

impl DevOrigin {
    /// Serialised origin (`scheme://host[:port]`, lowercase).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.origin
    }
}

impl fmt::Display for DevOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.origin)
    }
}

/// A `--dev-origin` value that is not a bare `http(s)://host[:port]` origin.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("invalid origin {0:?}: expected http(s)://host[:port] without path")]
pub struct InvalidDevOrigin(String);

impl FromStr for DevOrigin {
    type Err = InvalidDevOrigin;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidDevOrigin(value.to_owned());
        let uri: Uri = value.parse().map_err(|_| invalid())?;
        let scheme = uri.scheme().ok_or_else(invalid)?;
        let authority = uri.authority().ok_or_else(invalid)?;
        let bare_path = matches!(uri.path(), "" | "/");
        let web_scheme = *scheme == Scheme::HTTP || *scheme == Scheme::HTTPS;
        let has_user_info = authority.as_str().contains('@');
        if !web_scheme || !bare_path || uri.query().is_some() || has_user_info {
            return Err(invalid());
        }
        let authority = authority.as_str().to_ascii_lowercase();
        Ok(Self {
            origin: format!("{}://{authority}", scheme.as_str().to_ascii_lowercase()),
            authority,
        })
    }
}

/// Request refused by the [`AccessPolicy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AccessDenied {
    /// `Host` missing or not the server itself.
    #[error("unexpected Host header")]
    Host,
    /// `Origin` missing or foreign where it is required.
    #[error("unexpected or missing Origin header")]
    Origin,
}

impl AccessDenied {
    fn status(self) -> StatusCode {
        match self {
            Self::Host => StatusCode::MISDIRECTED_REQUEST,
            Self::Origin => StatusCode::FORBIDDEN,
        }
    }
}

impl IntoResponse for AccessDenied {
    fn into_response(self) -> Response {
        (self.status(), self.to_string()).into_response()
    }
}

/// Accepted `Host` and `Origin` values.
#[derive(Debug, Clone)]
pub struct AccessPolicy {
    hosts: Vec<String>,
    origins: Vec<String>,
}

impl AccessPolicy {
    /// Policy of a server listening on `port`, also used by `dev_origins`.
    #[must_use]
    pub fn new(port: u16, dev_origins: &[DevOrigin]) -> Self {
        let own_hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        let origins = own_hosts
            .iter()
            .map(|host| format!("http://{host}"))
            .chain(dev_origins.iter().map(|dev| dev.origin.clone()))
            .collect();
        let hosts = own_hosts
            .into_iter()
            .chain(dev_origins.iter().map(|dev| dev.authority.clone()))
            .collect();
        Self { hosts, origins }
    }

    /// Checks a request; `Origin` is required on [`WS_PATH`] and on requests
    /// that may change state.
    ///
    /// # Errors
    ///
    /// The header that failed the check.
    pub fn check(
        &self,
        method: &Method,
        path: &str,
        headers: &HeaderMap,
    ) -> Result<(), AccessDenied> {
        if !self.allows_host(headers.get(HOST)) {
            return Err(AccessDenied::Host);
        }
        let read_only = *method == Method::GET || *method == Method::HEAD;
        if (path == WS_PATH || !read_only) && !self.allows_origin(headers.get(ORIGIN)) {
            return Err(AccessDenied::Origin);
        }
        Ok(())
    }

    fn allows_host(&self, host: Option<&HeaderValue>) -> bool {
        host.and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<Authority>().ok())
            .is_some_and(|authority| {
                let host = authority.as_str().to_ascii_lowercase();
                self.hosts.contains(&host)
            })
    }

    fn allows_origin(&self, origin: Option<&HeaderValue>) -> bool {
        origin
            .and_then(|value| value.to_str().ok())
            .is_some_and(|origin| self.origins.contains(&origin.to_ascii_lowercase()))
    }
}

/// Middleware refusing requests that fail the [`AccessPolicy`].
pub(super) async fn enforce_access_policy(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    match state
        .policy
        .check(request.method(), request.uri().path(), request.headers())
    {
        Ok(()) => next.run(request).await,
        Err(denied) => {
            tracing::warn!(
                method = %request.method(),
                path = request.uri().path(),
                %denied,
                "request refused"
            );
            denied.into_response()
        }
    }
}

/// Middleware adding the security headers to every response; a
/// `Content-Security-Policy` set by the handler is kept.
pub(super) async fn add_security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers
        .entry(CONTENT_SECURITY_POLICY)
        .or_insert(HeaderValue::from_static(CONTENT_SECURITY_POLICY_BASE));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(
        CROSS_ORIGIN_OPENER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        CROSS_ORIGIN_RESOURCE_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORT: u16 = 7878;

    fn headers(host: Option<&str>, origin: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(host) = host {
            headers.insert(HOST, HeaderValue::from_str(host).unwrap());
        }
        if let Some(origin) = origin {
            headers.insert(ORIGIN, HeaderValue::from_str(origin).unwrap());
        }
        headers
    }

    fn policy() -> AccessPolicy {
        AccessPolicy::new(PORT, &["http://localhost:5173".parse().unwrap()])
    }

    #[test]
    fn own_hosts_are_accepted_case_insensitively() {
        for host in ["127.0.0.1:7878", "localhost:7878", "LocalHost:7878"] {
            assert_eq!(
                policy().check(&Method::GET, "/", &headers(Some(host), None)),
                Ok(())
            );
        }
    }

    #[test]
    fn foreign_or_missing_hosts_are_refused() {
        for host in [
            None,
            Some("evil.example:7878"),
            Some("127.0.0.1"),
            Some("127.0.0.1:7879"),
            Some("localhost:7878.evil.example"),
            Some("user@localhost:7878"),
        ] {
            assert_eq!(
                policy().check(&Method::GET, "/", &headers(host, None)),
                Err(AccessDenied::Host),
                "{host:?}"
            );
        }
    }

    #[test]
    fn websocket_requires_an_allowed_origin() {
        let check = |origin| {
            policy().check(
                &Method::GET,
                WS_PATH,
                &headers(Some("127.0.0.1:7878"), origin),
            )
        };
        assert_eq!(check(Some("http://127.0.0.1:7878")), Ok(()));
        assert_eq!(check(Some("http://localhost:7878")), Ok(()));
        assert_eq!(check(Some("http://localhost:5173")), Ok(()));
        assert_eq!(check(None), Err(AccessDenied::Origin));
        assert_eq!(check(Some("null")), Err(AccessDenied::Origin));
        assert_eq!(
            check(Some("http://evil.example")),
            Err(AccessDenied::Origin)
        );
        assert_eq!(
            check(Some("https://127.0.0.1:7878")),
            Err(AccessDenied::Origin)
        );
    }

    #[test]
    fn state_changing_requests_require_an_allowed_origin() {
        let host = Some("127.0.0.1:7878");
        assert_eq!(
            policy().check(&Method::POST, "/api/x", &headers(host, None)),
            Err(AccessDenied::Origin)
        );
        assert_eq!(
            policy().check(
                &Method::POST,
                "/api/x",
                &headers(host, Some("http://127.0.0.1:7878"))
            ),
            Ok(())
        );
        assert_eq!(
            policy().check(&Method::HEAD, "/", &headers(host, None)),
            Ok(())
        );
    }

    #[test]
    fn dev_origin_host_is_accepted() {
        assert_eq!(
            policy().check(
                &Method::GET,
                WS_PATH,
                &headers(Some("localhost:5173"), Some("http://localhost:5173"))
            ),
            Ok(())
        );
    }

    #[test]
    fn dev_origins_are_normalised() {
        let origin: DevOrigin = "HTTP://LocalHost:5173/".parse().unwrap();
        assert_eq!(origin.as_str(), "http://localhost:5173");
        assert_eq!(origin.to_string(), "http://localhost:5173");
        let origin: DevOrigin = "https://deck.example".parse().unwrap();
        assert_eq!(origin.as_str(), "https://deck.example");
    }

    #[test]
    fn invalid_dev_origins_are_rejected() {
        for value in [
            "localhost:5173",
            "ftp://localhost:5173",
            "http://localhost:5173/app",
            "http://localhost:5173/?a=b",
            "http://user@localhost:5173",
            "",
        ] {
            assert!(value.parse::<DevOrigin>().is_err(), "{value}");
        }
    }
}
