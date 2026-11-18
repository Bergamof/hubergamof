//! Where the latest release comes from: the GitHub releases API, or a
//! simulated answer for `--demo` and tests.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use thiserror::Error;

use super::version::{ParseVersionError, Version};

/// Public repository publishing the releases.
const RELEASES_URL: &str = "https://api.github.com/repos/Bergamof/hubergamof/releases/latest";

/// Release notes pages of the project: the only links offered to the user.
const NOTES_URL_PREFIX: &str = "https://github.com/Bergamof/hubergamof/releases/";

/// Longest release notes link kept.
const MAX_NOTES_URL_BYTES: usize = 512;

/// Redirections followed (GitHub redirects a renamed repository).
const MAX_REDIRECTS: usize = 3;

/// Longest wait for the whole request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Largest answer read: a release description is a few kilobytes.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// The latest stable release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    /// Its version.
    pub version: Version,
    /// Its release notes page.
    pub notes_url: String,
}

/// Why the latest release could not be determined.
#[derive(Debug, Error)]
pub enum UpdateError {
    /// The request failed (network down, TLS, timeout…).
    #[error("cannot reach the release server: {0}")]
    Network(#[from] reqwest::Error),
    /// The server answered with an error status (no release yet, rate limit…).
    #[error("the release server answered {0}")]
    Status(StatusCode),
    /// The answer is larger than any release description.
    #[error("the release server answer is too large")]
    TooLarge,
    /// The answer is not the expected JSON.
    #[error("unreadable release description: {0}")]
    Malformed(#[from] serde_json::Error),
    /// The release tag is not a `vX.Y.Z` version.
    #[error("unexpected release tag: {0}")]
    Tag(#[from] ParseVersionError),
    /// The latest release is a draft or a pre-release: nothing to offer.
    #[error("the latest release is not a stable one")]
    Unstable,
    /// Failure announced by a simulated source.
    #[error("{0}")]
    Simulated(String),
    /// The page announced as release notes is not an `https` link.
    #[error("unexpected release notes link")]
    NotesLink,
}

/// Source of the latest release.
#[async_trait]
pub trait ReleaseSource: Send + Sync {
    /// The latest stable release.
    ///
    /// # Errors
    ///
    /// [`UpdateError`] when it cannot be determined.
    async fn latest(&self) -> Result<ReleaseInfo, UpdateError>;
}

/// The GitHub releases API of the project, without authentication.
pub struct GithubReleases {
    client: Client,
    url: String,
}

impl GithubReleases {
    /// Client of the project's repository.
    ///
    /// # Errors
    ///
    /// [`UpdateError::Network`] when the HTTP client cannot be built.
    pub fn new() -> Result<Self, UpdateError> {
        Self::build(RELEASES_URL, true)
    }

    /// Client of another endpoint, plain HTTP allowed (tests against a local
    /// server).
    ///
    /// # Errors
    ///
    /// [`UpdateError::Network`] when the HTTP client cannot be built.
    pub fn with_url(url: &str) -> Result<Self, UpdateError> {
        Self::build(url, false)
    }

    fn build(url: &str, https_only: bool) -> Result<Self, UpdateError> {
        // Generic user agent (GitHub requires one): nothing about the user.
        let client = Client::builder()
            .user_agent(concat!("hubergamof/", env!("CARGO_PKG_VERSION")))
            .timeout(REQUEST_TIMEOUT)
            .https_only(https_only)
            .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
            .build()?;
        Ok(Self {
            client,
            url: url.to_owned(),
        })
    }

    async fn read_body(&self) -> Result<Vec<u8>, UpdateError> {
        let mut response = self
            .client
            .get(&self.url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(UpdateError::Status(response.status()));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(UpdateError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

#[async_trait]
impl ReleaseSource for GithubReleases {
    async fn latest(&self) -> Result<ReleaseInfo, UpdateError> {
        parse_release(&self.read_body().await?)
    }
}

#[derive(Deserialize)]
struct ReleaseJson {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// Reads a GitHub release; drafts and pre-releases are not offered.
fn parse_release(body: &[u8]) -> Result<ReleaseInfo, UpdateError> {
    let release: ReleaseJson = serde_json::from_slice(body)?;
    if release.draft || release.prerelease {
        return Err(UpdateError::Unstable);
    }
    if !is_notes_link(&release.html_url) {
        return Err(UpdateError::NotesLink);
    }
    Ok(ReleaseInfo {
        version: release.tag_name.parse()?,
        notes_url: release.html_url,
    })
}

/// Whether `url` is a release notes page of the project, of a sane length and
/// without control characters.
fn is_notes_link(url: &str) -> bool {
    url.starts_with(NOTES_URL_PREFIX)
        && url.len() <= MAX_NOTES_URL_BYTES
        && !url.chars().any(char::is_control)
}

impl UpdateError {
    /// Short description for the interface: the details (which come from a
    /// remote server) stay in the logs.
    #[must_use]
    pub fn public_message(&self) -> String {
        match self {
            Self::Network(_) => "release server unreachable".to_owned(),
            Self::Status(status) => format!("release server answered {status}"),
            Self::Unstable => "the latest release is not a stable one".to_owned(),
            Self::Simulated(message) => message.clone(),
            Self::TooLarge | Self::Malformed(_) | Self::Tag(_) | Self::NotesLink => {
                "unexpected answer from the release server".to_owned()
            }
        }
    }
}

/// Answers a fixed release, without network (`--demo`, tests).
pub struct SimulatedReleases {
    answer: Result<ReleaseInfo, String>,
}

impl SimulatedReleases {
    /// Always announces `version`.
    #[must_use]
    pub fn announcing(version: Version) -> Self {
        Self {
            answer: Ok(ReleaseInfo {
                version,
                notes_url: format!("{NOTES_URL_PREFIX}latest"),
            }),
        }
    }

    /// Always fails with `message`.
    #[must_use]
    pub fn failing(message: &str) -> Self {
        Self {
            answer: Err(message.to_owned()),
        }
    }
}

#[async_trait]
impl ReleaseSource for SimulatedReleases {
    async fn latest(&self) -> Result<ReleaseInfo, UpdateError> {
        self.answer.clone().map_err(UpdateError::Simulated)
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::{HeaderMap, StatusCode as AxumStatus};
    use axum::routing::get;
    use serde_json::json;
    use tokio::net::TcpListener;

    use super::*;

    fn release_json(tag: &str, url: &str, draft: bool, prerelease: bool) -> String {
        json!({
            "tag_name": tag,
            "html_url": url,
            "draft": draft,
            "prerelease": prerelease,
            "name": "ignored extra field",
        })
        .to_string()
    }

    /// Serves `status` and `body` on a local port; returns the endpoint and
    /// the `User-Agent` / `Accept` the client sent, once it has asked.
    async fn serve(
        status: AxumStatus,
        body: String,
    ) -> (String, tokio::sync::watch::Receiver<Option<String>>) {
        let (sent, seen) = tokio::sync::watch::channel(None);
        let app = Router::new().route(
            "/latest",
            get(move |headers: HeaderMap| {
                let agent = headers
                    .get("user-agent")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                sent.send_replace(agent);
                async move { (status, body) }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        (format!("http://{address}/latest"), seen)
    }

    async fn latest_from(status: AxumStatus, body: String) -> Result<ReleaseInfo, UpdateError> {
        let (url, _) = serve(status, body).await;
        GithubReleases::with_url(&url).unwrap().latest().await
    }

    #[tokio::test]
    async fn reads_the_latest_release() {
        let body = release_json(
            "v1.4.2",
            "https://github.com/Bergamof/hubergamof/releases/tag/v1.4.2",
            false,
            false,
        );
        let release = latest_from(AxumStatus::OK, body).await.unwrap();
        assert_eq!(release.version, Version::new(1, 4, 2));
        assert_eq!(
            release.notes_url,
            "https://github.com/Bergamof/hubergamof/releases/tag/v1.4.2"
        );
    }

    #[tokio::test]
    async fn the_request_carries_a_generic_user_agent() {
        let body = release_json(
            "v1.0.0",
            "https://github.com/Bergamof/hubergamof/releases/tag/v1.0.0",
            false,
            false,
        );
        let (url, mut seen) = serve(AxumStatus::OK, body).await;
        GithubReleases::with_url(&url)
            .unwrap()
            .latest()
            .await
            .unwrap();
        let agent = seen.borrow_and_update().clone().unwrap();
        assert_eq!(agent, concat!("hubergamof/", env!("CARGO_PKG_VERSION")));
    }

    #[tokio::test]
    async fn an_error_status_is_reported() {
        let error = latest_from(AxumStatus::NOT_FOUND, String::new())
            .await
            .unwrap_err();
        assert!(
            matches!(error, UpdateError::Status(StatusCode::NOT_FOUND)),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_rate_limit_is_reported_as_a_status() {
        let error = latest_from(AxumStatus::FORBIDDEN, "{}".into())
            .await
            .unwrap_err();
        assert!(
            matches!(error, UpdateError::Status(StatusCode::FORBIDDEN)),
            "{error}"
        );
    }

    #[tokio::test]
    async fn an_oversized_answer_is_refused() {
        let body = "x".repeat(MAX_RESPONSE_BYTES + 1);
        let error = latest_from(AxumStatus::OK, body).await.unwrap_err();
        assert!(matches!(error, UpdateError::TooLarge), "{error}");
    }

    #[tokio::test]
    async fn an_unreachable_server_is_a_network_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/latest", listener.local_addr().unwrap());
        drop(listener);
        let error = GithubReleases::with_url(&url)
            .unwrap()
            .latest()
            .await
            .unwrap_err();
        assert!(matches!(error, UpdateError::Network(_)), "{error}");
    }

    #[test]
    fn drafts_and_prereleases_are_not_offered() {
        for (draft, prerelease) in [(true, false), (false, true)] {
            let body = release_json(
                "v2.0.0",
                "https://github.com/Bergamof/hubergamof/releases/tag/v1.0.0",
                draft,
                prerelease,
            );
            let error = parse_release(body.as_bytes()).unwrap_err();
            assert!(matches!(error, UpdateError::Unstable), "{error}");
        }
    }

    #[test]
    fn malformed_answers_are_refused() {
        assert!(matches!(
            parse_release(b"not json").unwrap_err(),
            UpdateError::Malformed(_)
        ));
        assert!(matches!(
            parse_release(b"{\"tag_name\":\"v1.0.0\"}").unwrap_err(),
            UpdateError::Malformed(_)
        ));
        let body = release_json(
            "nightly",
            "https://github.com/Bergamof/hubergamof/releases/tag/v1.0.0",
            false,
            false,
        );
        assert!(matches!(
            parse_release(body.as_bytes()).unwrap_err(),
            UpdateError::Tag(_)
        ));
    }

    #[test]
    fn the_public_message_hides_what_the_server_said() {
        let error = parse_release(br#"{"tag_name":"line\nforged","html_url":"x"}"#).unwrap_err();
        assert_eq!(
            error.public_message(),
            "unexpected answer from the release server"
        );
        assert_eq!(
            UpdateError::Status(StatusCode::FORBIDDEN).public_message(),
            "release server answered 403 Forbidden"
        );
    }

    #[test]
    fn a_notes_link_must_be_a_release_page_of_the_project() {
        for url in [
            "http://example.org/r",
            "javascript:alert(1)",
            "file:///etc/passwd",
        ] {
            let body = release_json("v1.0.0", url, false, false);
            let error = parse_release(body.as_bytes()).unwrap_err();
            assert!(matches!(error, UpdateError::NotesLink), "{url}: {error}");
        }
    }

    #[tokio::test]
    async fn the_simulated_sources_answer_without_network() {
        let release = SimulatedReleases::announcing(Version::new(3, 0, 0))
            .latest()
            .await
            .unwrap();
        assert_eq!(release.version, Version::new(3, 0, 0));
        let error = SimulatedReleases::failing("boom")
            .latest()
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "boom");
    }
}
