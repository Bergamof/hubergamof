//! Installing a newer release: download its package and the signed
//! checksums, then send them to the update helper, a root service that systemd
//! starts for each connection to its socket. Nothing here is trusted for the
//! installation: the helper checks the signature itself.

use std::io::ErrorKind;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use hubergamof_protocol::InstallSupport;
use reqwest::Client;
use reqwest::redirect::Policy;
use thiserror::Error;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;

use super::package::{CHECKSUMS_FILE, PackageFormat, SIGNATURE_FILE};
use super::source::{ReleaseAsset, ReleaseInfo};
use super::wire::{
    Bundle, MAX_CHECKSUMS_BYTES, MAX_PACKAGE_BYTES, MAX_RESPONSE_BYTES, MAX_SIGNATURE_BYTES,
    Response, SOCKET_PATH, encode_bundle,
};

/// Longest wait for the helper's answer (the installation itself): a stuck
/// package manager must not leave the update « installing » for ever. The
/// helper runs as root and is not killed: only no longer waited for.
const HELPER_TIMEOUT: Duration = Duration::from_mins(20);

/// Longest download (connection included).
const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(10);

/// Redirections followed: the files are served from GitHub's own hosts.
const MAX_REDIRECTS: usize = 3;

/// Hosts a download may be redirected to.
const GITHUB_HOST: &str = "github.com";
const GITHUB_CONTENT_SUFFIX: &str = ".githubusercontent.com";

/// Where an installation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallStage {
    /// Fetching the files.
    Downloading,
    /// The helper verifies and installs.
    Installing,
}

/// Why an installation failed.
#[derive(Debug, Error)]
pub enum InstallError {
    /// This installation cannot update itself.
    #[error("this installation cannot update itself")]
    Unsupported,
    /// The release does not carry a needed file.
    #[error("the release has no {0}")]
    MissingAsset(String),
    /// A download failed.
    #[error("download failed: {0}")]
    Download(#[from] reqwest::Error),
    /// A file is larger than any release file.
    #[error("a downloaded file is too large")]
    TooLarge,
    /// The helper failed or could not be reached.
    #[error("{0}")]
    Helper(String),
}

impl InstallError {
    /// Short description for the interface; details stay in the logs.
    #[must_use]
    pub fn public_message(&self) -> String {
        match self {
            Self::Unsupported => "this installation cannot update itself".to_owned(),
            Self::MissingAsset(_) => "the release lacks a package for this system".to_owned(),
            Self::Download(_) | Self::TooLarge => "download failed".to_owned(),
            Self::Helper(message) => message.clone(),
        }
    }
}

/// Installs a release.
#[async_trait]
pub trait Installer: Send + Sync {
    /// Whether this installer can install at all.
    fn support(&self) -> InstallSupport;

    /// Installs `release`, telling `progress` where it is.
    ///
    /// # Errors
    ///
    /// [`InstallError`]; the installed version is then unchanged.
    async fn install(
        &self,
        release: &ReleaseInfo,
        progress: &(dyn Fn(InstallStage) + Send + Sync),
    ) -> Result<(), InstallError>;
}

/// For installations that cannot update themselves (sources, containers).
pub struct UnsupportedInstaller;

#[async_trait]
impl Installer for UnsupportedInstaller {
    fn support(&self) -> InstallSupport {
        InstallSupport::Unpackaged
    }

    async fn install(
        &self,
        _release: &ReleaseInfo,
        _progress: &(dyn Fn(InstallStage) + Send + Sync),
    ) -> Result<(), InstallError> {
        Err(InstallError::Unsupported)
    }
}

/// `--demo` and tests: goes through the stages, installs nothing.
pub struct SimulatedInstaller {
    failure: Option<String>,
}

impl SimulatedInstaller {
    /// Succeeds.
    #[must_use]
    pub fn succeeding() -> Self {
        Self { failure: None }
    }

    /// Fails with `message`.
    #[must_use]
    pub fn failing(message: &str) -> Self {
        Self {
            failure: Some(message.to_owned()),
        }
    }
}

#[async_trait]
impl Installer for SimulatedInstaller {
    fn support(&self) -> InstallSupport {
        InstallSupport::Demo
    }

    async fn install(
        &self,
        _release: &ReleaseInfo,
        progress: &(dyn Fn(InstallStage) + Send + Sync),
    ) -> Result<(), InstallError> {
        progress(InstallStage::Downloading);
        progress(InstallStage::Installing);
        self.failure
            .clone()
            .map_or(Ok(()), |message| Err(InstallError::Helper(message)))
    }
}

/// Sends a bundle to the update helper and waits for its answer.
#[async_trait]
pub trait UpdateChannel: Send + Sync {
    /// Has the helper verify and install `bundle`.
    ///
    /// # Errors
    ///
    /// [`InstallError::Helper`] with a short description.
    async fn send(&self, bundle: &Bundle) -> Result<(), InstallError>;
}

/// The helper's socket: systemd starts the helper (as root, with its own clean
/// environment) for each connection.
pub struct SocketChannel {
    path: PathBuf,
}

impl SocketChannel {
    /// The helper's socket at `path`.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The real one, [`SOCKET_PATH`].
    #[must_use]
    pub fn system() -> Self {
        Self::at(SOCKET_PATH)
    }

    async fn exchange(&self, bundle: &Bundle) -> Result<Response, InstallError> {
        let mut stream = UnixStream::connect(&self.path).await.map_err(|error| {
            tracing::warn!(%error, path = %self.path.display(), "cannot reach the update helper");
            InstallError::Helper(match error.kind() {
                ErrorKind::NotFound | ErrorKind::ConnectionRefused => {
                    "the update service is not running (or already busy)".to_owned()
                }
                _ => "cannot reach the update service".to_owned(),
            })
        })?;
        // The helper may refuse and hang up while the package is still being
        // sent: its answer is then what counts, not the broken pipe.
        let sent = stream.write_all(&encode_bundle(bundle)).await;
        if let Err(error) = &sent {
            tracing::debug!(%error, "the update helper stopped reading");
        }
        let _ = stream.shutdown().await;
        let mut line = Vec::new();
        let read = stream
            .take(MAX_RESPONSE_BYTES as u64)
            .read_to_end(&mut line)
            .await;
        // A helper that refused without reading everything closes with data
        // pending: the connection is then reset, after the answer was delivered.
        if line.is_empty() {
            // Accepted then dropped (another update runs: one at a time) or gone.
            tracing::warn!(?read, "no answer from the update service");
            return Err(InstallError::Helper(
                "the update service is busy or unavailable, try again later".to_owned(),
            ));
        }
        Ok(Response::parse(&line))
    }
}

#[async_trait]
impl UpdateChannel for SocketChannel {
    async fn send(&self, bundle: &Bundle) -> Result<(), InstallError> {
        let response = tokio::time::timeout(HELPER_TIMEOUT, self.exchange(bundle))
            .await
            .map_err(|_| {
                InstallError::Helper(
                    "the installation is taking long: its outcome is unknown, check the installed version"
                        .to_owned(),
                )
            })??;
        match response {
            Response::Installed(_) => Ok(()),
            Response::Failed(message) => {
                tracing::warn!(%message, "the update helper refused or failed");
                Err(InstallError::Helper(message))
            }
        }
    }
}

/// Downloads from the project's releases only.
pub struct Downloader {
    client: Client,
}

impl Downloader {
    /// HTTPS only, redirections only to GitHub's own hosts.
    ///
    /// # Errors
    ///
    /// [`InstallError::Download`] when the client cannot be built.
    pub fn new() -> Result<Self, InstallError> {
        let policy = Policy::custom(|attempt| {
            let allowed = attempt.url().scheme() == "https"
                && attempt.url().host_str().is_some_and(|host| {
                    host == GITHUB_HOST || host.ends_with(GITHUB_CONTENT_SUFFIX)
                });
            if attempt.previous().len() >= MAX_REDIRECTS || !allowed {
                attempt.stop()
            } else {
                attempt.follow()
            }
        });
        Self::with_policy(policy, true)
    }

    /// Plain HTTP and any redirection (tests against a local server).
    ///
    /// # Errors
    ///
    /// [`InstallError::Download`] when the client cannot be built.
    pub fn unrestricted() -> Result<Self, InstallError> {
        Self::with_policy(Policy::limited(MAX_REDIRECTS), false)
    }

    fn with_policy(policy: Policy, https_only: bool) -> Result<Self, InstallError> {
        let client = Client::builder()
            .user_agent(concat!("hubergamof/", env!("CARGO_PKG_VERSION")))
            .timeout(DOWNLOAD_TIMEOUT)
            .https_only(https_only)
            .redirect(policy)
            .build()?;
        Ok(Self { client })
    }

    /// Downloads `url`, at most `limit` bytes.
    async fn fetch(&self, url: &str, limit: usize) -> Result<Vec<u8>, InstallError> {
        let mut response = self.client.get(url).send().await?.error_for_status()?;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > limit {
                return Err(InstallError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

/// Installs from the system's package, through the helper.
pub struct PackageInstaller {
    format: PackageFormat,
    downloader: Downloader,
    channel: Box<dyn UpdateChannel>,
}

impl PackageInstaller {
    /// Installer of packages of `format`.
    #[must_use]
    pub fn new(
        format: PackageFormat,
        downloader: Downloader,
        channel: Box<dyn UpdateChannel>,
    ) -> Self {
        Self {
            format,
            downloader,
            channel,
        }
    }

    async fn download(&self, release: &ReleaseInfo) -> Result<Bundle, InstallError> {
        let package = self.format.file_name(release.version);
        let fetch = async |name: &str, limit: usize| {
            let asset = find_asset(release, name)?;
            self.downloader.fetch(&asset.url, limit).await
        };
        Ok(Bundle {
            checksums: fetch(CHECKSUMS_FILE, MAX_CHECKSUMS_BYTES).await?,
            signature: fetch(SIGNATURE_FILE, MAX_SIGNATURE_BYTES).await?,
            package: fetch(&package, MAX_PACKAGE_BYTES).await?,
        })
    }
}

fn find_asset<'a>(release: &'a ReleaseInfo, name: &str) -> Result<&'a ReleaseAsset, InstallError> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .ok_or_else(|| InstallError::MissingAsset(name.to_owned()))
}

#[async_trait]
impl Installer for PackageInstaller {
    fn support(&self) -> InstallSupport {
        InstallSupport::Available
    }

    async fn install(
        &self,
        release: &ReleaseInfo,
        progress: &(dyn Fn(InstallStage) + Send + Sync),
    ) -> Result<(), InstallError> {
        progress(InstallStage::Downloading);
        let bundle = self.download(release).await?;
        progress(InstallStage::Installing);
        self.channel.send(&bundle).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::routing::get;
    use tokio::net::{TcpListener, UnixListener};

    use super::*;
    use crate::update::version::Version;
    use crate::update::wire::read_bundle;

    const VERSION: Version = Version::new(1, 1, 0);

    /// Remembers the bundle it was sent, and answers as told.
    #[derive(Default)]
    struct FakeChannel {
        sent: Mutex<Vec<Bundle>>,
        failure: Option<String>,
    }

    #[async_trait]
    impl UpdateChannel for Arc<FakeChannel> {
        async fn send(&self, bundle: &Bundle) -> Result<(), InstallError> {
            self.sent.lock().unwrap().push(bundle.clone());
            self.failure
                .clone()
                .map_or(Ok(()), |message| Err(InstallError::Helper(message)))
        }
    }

    /// Serves `files` on a local port; returns the base URL.
    async fn serve(files: &[(&str, Vec<u8>)]) -> String {
        let mut app = Router::new();
        for (name, body) in files {
            let body = body.clone();
            app = app.route(&format!("/{name}"), get(move || async move { body }));
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        format!("http://{address}")
    }

    fn release_at(base: &str, names: &[&str]) -> ReleaseInfo {
        ReleaseInfo {
            version: VERSION,
            notes_url: String::new(),
            assets: names
                .iter()
                .map(|name| ReleaseAsset {
                    name: (*name).to_owned(),
                    url: format!("{base}/{name}"),
                })
                .collect(),
        }
    }

    fn installer(channel: &Arc<FakeChannel>) -> PackageInstaller {
        PackageInstaller::new(
            PackageFormat::Deb,
            Downloader::unrestricted().unwrap(),
            Box::new(Arc::clone(channel)),
        )
    }

    fn files() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            (CHECKSUMS_FILE, b"sums".to_vec()),
            (SIGNATURE_FILE, b"sig".to_vec()),
            ("hubergamof_1.1.0_amd64.deb", b"deb".to_vec()),
        ]
    }

    fn names() -> [&'static str; 3] {
        [CHECKSUMS_FILE, SIGNATURE_FILE, "hubergamof_1.1.0_amd64.deb"]
    }

    #[tokio::test]
    async fn the_three_files_reach_the_helper_in_order() {
        let base = serve(&files()).await;
        let channel = Arc::new(FakeChannel::default());
        let stages = Mutex::new(Vec::new());
        installer(&channel)
            .install(&release_at(&base, &names()), &|stage| {
                stages.lock().unwrap().push(stage);
            })
            .await
            .unwrap();
        assert_eq!(
            *stages.lock().unwrap(),
            [InstallStage::Downloading, InstallStage::Installing]
        );
        let sent = channel.sent.lock().unwrap();
        assert_eq!(
            *sent,
            [Bundle {
                checksums: b"sums".to_vec(),
                signature: b"sig".to_vec(),
                package: b"deb".to_vec(),
            }]
        );
    }

    #[tokio::test]
    async fn a_missing_asset_is_reported_without_contacting_the_helper() {
        let base = serve(&files()).await;
        let channel = Arc::new(FakeChannel::default());
        let error = installer(&channel)
            .install(&release_at(&base, &names()[..2]), &|_| {})
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::MissingAsset(name) if name.contains("amd64")));
        assert!(channel.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failed_download_stops_before_the_helper() {
        let base = serve(&files()[..2]).await; // the package is a 404
        let channel = Arc::new(FakeChannel::default());
        let error = installer(&channel)
            .install(&release_at(&base, &names()), &|_| {})
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::Download(_)), "{error}");
        assert_eq!(error.public_message(), "download failed");
        assert!(channel.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_oversized_checksums_file_is_refused() {
        let mut served = files();
        served[0].1 = vec![b'x'; MAX_CHECKSUMS_BYTES + 1];
        let base = serve(&served).await;
        let channel = Arc::new(FakeChannel::default());
        let error = installer(&channel)
            .install(&release_at(&base, &names()), &|_| {})
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::TooLarge), "{error}");
        assert!(channel.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_helper_failure_is_passed_on() {
        let base = serve(&files()).await;
        let channel = Arc::new(FakeChannel {
            failure: Some("signature mismatch".into()),
            ..FakeChannel::default()
        });
        let error = installer(&channel)
            .install(&release_at(&base, &names()), &|_| {})
            .await
            .unwrap_err();
        assert_eq!(error.public_message(), "signature mismatch");
    }

    /// A server answering `body` as a stream, so without a `Content-Length`.
    async fn serve_streamed(path: &str, body: Vec<u8>) -> String {
        let app = Router::new().route(
            path,
            get(move || {
                let chunks: Vec<Result<Vec<u8>, std::io::Error>> =
                    body.chunks(1024).map(|chunk| Ok(chunk.to_vec())).collect();
                async move { axum::body::Body::from_stream(futures_util::stream::iter(chunks)) }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn the_size_limit_holds_for_a_stream_without_content_length() {
        let base = serve_streamed("/big", vec![b'x'; 5000]).await;
        let downloader = Downloader::unrestricted().unwrap();
        let error = downloader
            .fetch(&format!("{base}/big"), 4000)
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::TooLarge), "{error}");
        assert_eq!(
            downloader
                .fetch(&format!("{base}/big"), 5000)
                .await
                .unwrap()
                .len(),
            5000
        );
    }

    #[tokio::test]
    async fn the_real_downloader_refuses_plain_http() {
        let base = serve(&files()).await;
        let error = Downloader::new()
            .unwrap()
            .fetch(&format!("{base}/{CHECKSUMS_FILE}"), 1024)
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::Download(_)), "{error}");
    }

    /// A socket answering `answer` after reading the whole bundle; hands the
    /// bundle it read back through the join handle.
    fn fake_helper(
        dir: &std::path::Path,
        answer: &'static [u8],
    ) -> (PathBuf, tokio::task::JoinHandle<Bundle>) {
        let path = dir.join("update.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            stream.read_to_end(&mut received).await.unwrap();
            stream.write_all(answer).await.unwrap();
            read_bundle(
                &mut received.as_slice(),
                std::time::Instant::now() + Duration::from_secs(60),
            )
            .unwrap()
        });
        (path, task)
    }

    fn bundle() -> Bundle {
        Bundle {
            checksums: b"sums".to_vec(),
            signature: vec![1; 64],
            package: b"pkg".to_vec(),
        }
    }

    #[tokio::test]
    async fn the_socket_channel_sends_the_bundle_and_reads_the_answer() {
        let dir = tempfile::tempdir().unwrap();
        let (path, helper) = fake_helper(dir.path(), b"ok 1.1.0\n");
        SocketChannel::at(path).send(&bundle()).await.unwrap();
        assert_eq!(helper.await.unwrap(), bundle());
    }

    #[tokio::test]
    async fn a_refusal_from_the_helper_is_an_error_with_its_reason() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _helper) = fake_helper(dir.path(), b"error the signature does not match\n");
        let error = SocketChannel::at(path).send(&bundle()).await.unwrap_err();
        assert_eq!(error.public_message(), "the signature does not match");
    }

    #[tokio::test]
    async fn an_answer_that_is_not_the_helpers_is_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _helper) = fake_helper(dir.path(), b"\x1b[31mnonsense");
        let error = SocketChannel::at(path).send(&bundle()).await.unwrap_err();
        assert!(error.public_message().contains("unexpected"), "{error}");
    }

    #[tokio::test]
    async fn a_connection_dropped_at_once_is_reported_as_busy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.sock");
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            // Like systemd with a second connection while one update runs.
            let (stream, _) = listener.accept().await.unwrap();
            drop(stream);
        });
        let error = SocketChannel::at(path).send(&bundle()).await.unwrap_err();
        assert!(
            error.public_message().contains("busy or unavailable"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn without_the_service_the_error_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let error = SocketChannel::at(dir.path().join("absent.sock"))
            .send(&bundle())
            .await
            .unwrap_err();
        assert!(error.public_message().contains("not running"), "{error}");
    }

    #[tokio::test]
    async fn a_helper_that_hangs_up_early_still_gets_its_answer_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.sock");
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            // Refuses at once, without reading the (large) package.
            stream.write_all(b"error too large\n").await.unwrap();
        });
        let big = Bundle {
            package: vec![0; 8 * 1024 * 1024],
            ..bundle()
        };
        let error = SocketChannel::at(path).send(&big).await.unwrap_err();
        assert_eq!(error.public_message(), "too large");
    }

    #[tokio::test]
    async fn the_simulated_and_unsupported_installers_report_their_support() {
        let release = release_at("http://unused", &[]);
        let simulated = SimulatedInstaller::succeeding();
        assert_eq!(simulated.support(), InstallSupport::Demo);
        simulated.install(&release, &|_| {}).await.unwrap();
        let failing = SimulatedInstaller::failing("nope");
        assert_eq!(
            failing
                .install(&release, &|_| {})
                .await
                .unwrap_err()
                .public_message(),
            "nope"
        );
        assert_eq!(UnsupportedInstaller.support(), InstallSupport::Unpackaged);
        assert!(matches!(
            UnsupportedInstaller.install(&release, &|_| {}).await,
            Err(InstallError::Unsupported)
        ));
    }
}
