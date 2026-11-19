//! Installing a newer release: download its package and the signed
//! checksums, then hand them to the update helper (root, through `pkexec`),
//! which verifies and installs. Nothing here is trusted for that: the helper
//! checks the signature itself.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use hubergamof_modules::desktop::{Launch, scoped_command};
use hubergamof_protocol::InstallSupport;
use reqwest::Client;
use reqwest::redirect::Policy;
use thiserror::Error;

use super::package::{CHECKSUMS_FILE, PackageFormat, SIGNATURE_FILE};
use super::source::{ReleaseAsset, ReleaseInfo};

/// Where the package puts the update helper.
pub const HELPER_PATH: &str = "/usr/lib/hubergamof/hubergamof-update-helper";

/// Program running the helper as root.
const PRIVILEGE_PROGRAM: &str = "pkexec";

/// `pkexec` exit codes when polkit refuses or the user dismissed the dialog.
const POLKIT_REFUSED: [i32; 2] = [126, 127];

/// Largest signed checksums or signature downloaded.
const MAX_SMALL_FILE_BYTES: u64 = 64 * 1024;

/// Largest package downloaded; the real ones are around 15 MiB.
const MAX_PACKAGE_BYTES: u64 = 128 * 1024 * 1024;

/// Longest download (connection included).
const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(10);

/// Redirections followed: the files are served from GitHub's own hosts.
const MAX_REDIRECTS: usize = 3;

/// Hosts a download may be redirected to.
const GITHUB_HOST: &str = "github.com";
const GITHUB_CONTENT_SUFFIX: &str = ".githubusercontent.com";

/// Longest failure description shown.
const MAX_MESSAGE_CHARS: usize = 200;

/// Mode of the download folder: the user only.
const DOWNLOAD_DIR_MODE: u32 = 0o700;

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
    /// The download folder cannot be prepared or written.
    #[error("cannot store the download: {0}")]
    Storage(#[from] std::io::Error),
    /// The helper failed or polkit refused it.
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
            Self::Storage(_) => "cannot store the download".to_owned(),
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

/// Runs the update helper on a download folder.
#[async_trait]
pub trait HelperRunner: Send + Sync {
    /// Runs the helper on `download_dir`.
    ///
    /// # Errors
    ///
    /// [`InstallError::Helper`] with a short description.
    async fn run(&self, download_dir: &Path) -> Result<(), InstallError>;
}

/// `pkexec` on the helper, started outside the manager's service: installing
/// the package restarts the service, which must not stop the installation.
pub struct PkexecHelper;

#[async_trait]
impl HelperRunner for PkexecHelper {
    async fn run(&self, download_dir: &Path) -> Result<(), InstallError> {
        let launch = Launch {
            program: PRIVILEGE_PROGRAM.into(),
            args: vec![HELPER_PATH.into(), download_dir.as_os_str().to_owned()],
        };
        let output = tokio::task::spawn_blocking(move || {
            scoped_command(&launch)
                .stdin(std::process::Stdio::null())
                .output()
        })
        .await
        .map_err(|error| InstallError::Helper(format!("cannot run the helper: {error}")))?
        .map_err(|error| {
            InstallError::Helper(format!("cannot run {PRIVILEGE_PROGRAM}: {error}"))
        })?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(status = %output.status, %stderr, "the update helper failed");
        Err(InstallError::Helper(helper_failure(
            output.status.code(),
            &stderr,
        )))
    }
}

/// Short description of a failed helper run: polkit's refusal, else the
/// helper's last line.
fn helper_failure(code: Option<i32>, stderr: &str) -> String {
    if code.is_some_and(|code| POLKIT_REFUSED.contains(&code)) {
        return "the system refused the installation (polkit)".to_owned();
    }
    let line = stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("the update helper failed");
    line.chars()
        .filter(|character| !character.is_control())
        .take(MAX_MESSAGE_CHARS)
        .collect()
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

    /// Downloads `url` to `path`, at most `limit` bytes.
    async fn fetch(&self, url: &str, path: &Path, limit: u64) -> Result<(), InstallError> {
        let mut response = self.client.get(url).send().await?.error_for_status()?;
        let mut file = fs::File::create_new(path)?;
        let mut written: u64 = 0;
        while let Some(chunk) = response.chunk().await? {
            written += chunk.len() as u64;
            if written > limit {
                return Err(InstallError::TooLarge);
            }
            file.write_all(&chunk)?;
        }
        Ok(file.sync_all()?)
    }
}

/// Installs from the system's package, through the helper.
pub struct PackageInstaller {
    format: PackageFormat,
    downloader: Downloader,
    helper: Box<dyn HelperRunner>,
    work_dir: PathBuf,
}

impl PackageInstaller {
    /// Installer of packages of `format`, downloading under `work_dir`.
    #[must_use]
    pub fn new(
        format: PackageFormat,
        downloader: Downloader,
        helper: Box<dyn HelperRunner>,
        work_dir: PathBuf,
    ) -> Self {
        Self {
            format,
            downloader,
            helper,
            work_dir,
        }
    }

    /// An empty, private download folder.
    fn prepare_work_dir(&self) -> Result<(), InstallError> {
        match fs::remove_dir_all(&self.work_dir) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        if let Some(parent) = self.work_dir.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(fs::DirBuilder::new()
            .mode(DOWNLOAD_DIR_MODE)
            .create(&self.work_dir)?)
    }

    async fn download(&self, release: &ReleaseInfo) -> Result<(), InstallError> {
        let package = self.format.file_name(release.version);
        for (name, limit) in [
            (CHECKSUMS_FILE, MAX_SMALL_FILE_BYTES),
            (SIGNATURE_FILE, MAX_SMALL_FILE_BYTES),
            (package.as_str(), MAX_PACKAGE_BYTES),
        ] {
            let asset = find_asset(release, name)?;
            self.downloader
                .fetch(&asset.url, &self.work_dir.join(name), limit)
                .await?;
        }
        Ok(())
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
        self.prepare_work_dir()?;
        let outcome = async {
            self.download(release).await?;
            progress(InstallStage::Installing);
            self.helper.run(&self.work_dir).await
        }
        .await;
        if let Err(error) = fs::remove_dir_all(&self.work_dir) {
            tracing::debug!(%error, "cannot remove the download folder");
        }
        outcome
    }
}

/// The user's cache folder for downloads: `$XDG_CACHE_HOME`, else `~/.cache`.
#[must_use]
pub fn download_dir_from_env() -> Option<PathBuf> {
    let absolute = |variable: &str| {
        std::env::var_os(variable)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let cache =
        absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|home| home.join(".cache")))?;
    Some(cache.join("hubergamof").join("update"))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::routing::get;
    use tokio::net::TcpListener;

    use super::*;
    use crate::update::version::Version;

    const VERSION: Version = Version::new(1, 1, 0);

    /// What the fake helper saw in the download folder.
    #[derive(Default)]
    struct FakeHelper {
        seen: Mutex<Vec<(String, Vec<u8>)>>,
        failure: Option<String>,
    }

    #[async_trait]
    impl HelperRunner for Arc<FakeHelper> {
        async fn run(&self, download_dir: &Path) -> Result<(), InstallError> {
            let mut files: Vec<_> = fs::read_dir(download_dir)
                .unwrap()
                .map(|entry| {
                    let entry = entry.unwrap();
                    (
                        entry.file_name().to_string_lossy().into_owned(),
                        fs::read(entry.path()).unwrap(),
                    )
                })
                .collect();
            files.sort();
            *self.seen.lock().unwrap() = files;
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

    fn installer(work: &Path, helper: &Arc<FakeHelper>) -> PackageInstaller {
        PackageInstaller::new(
            PackageFormat::Deb,
            Downloader::unrestricted().unwrap(),
            Box::new(Arc::clone(helper)),
            work.join("update"),
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
    async fn the_three_files_reach_the_helper_then_the_folder_is_removed() {
        let work = tempfile::tempdir().unwrap();
        let base = serve(&files()).await;
        let helper = Arc::new(FakeHelper::default());
        let stages = Mutex::new(Vec::new());
        installer(work.path(), &helper)
            .install(&release_at(&base, &names()), &|stage| {
                stages.lock().unwrap().push(stage);
            })
            .await
            .unwrap();
        assert_eq!(
            *stages.lock().unwrap(),
            [InstallStage::Downloading, InstallStage::Installing]
        );
        let seen = helper.seen.lock().unwrap();
        let seen: Vec<_> = seen
            .iter()
            .map(|(name, body)| (name.as_str(), body.clone()))
            .collect();
        let mut expected = files();
        expected.sort();
        assert_eq!(seen, expected);
        assert!(!work.path().join("update").exists());
    }

    #[tokio::test]
    async fn a_missing_asset_is_reported_without_running_the_helper() {
        let work = tempfile::tempdir().unwrap();
        let base = serve(&files()).await;
        let helper = Arc::new(FakeHelper::default());
        let error = installer(work.path(), &helper)
            .install(&release_at(&base, &names()[..2]), &|_| {})
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::MissingAsset(name) if name.contains("amd64")));
        assert!(helper.seen.lock().unwrap().is_empty());
        assert!(!work.path().join("update").exists());
    }

    #[tokio::test]
    async fn a_failed_download_stops_before_the_helper() {
        let work = tempfile::tempdir().unwrap();
        let base = serve(&files()[..2]).await; // the package is a 404
        let helper = Arc::new(FakeHelper::default());
        let error = installer(work.path(), &helper)
            .install(&release_at(&base, &names()), &|_| {})
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::Download(_)), "{error}");
        assert_eq!(error.public_message(), "download failed");
        assert!(helper.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_oversized_checksums_file_is_refused() {
        let work = tempfile::tempdir().unwrap();
        let big = vec![b'x'; usize::try_from(MAX_SMALL_FILE_BYTES).unwrap() + 1];
        let mut served = files();
        served[0].1 = big;
        let base = serve(&served).await;
        let helper = Arc::new(FakeHelper::default());
        let error = installer(work.path(), &helper)
            .install(&release_at(&base, &names()), &|_| {})
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::TooLarge), "{error}");
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
        let dir = tempfile::tempdir().unwrap();
        let base = serve_streamed("/big", vec![b'x'; 5000]).await;
        let downloader = Downloader::unrestricted().unwrap();
        let error = downloader
            .fetch(&format!("{base}/big"), &dir.path().join("out"), 4000)
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::TooLarge), "{error}");
        downloader
            .fetch(&format!("{base}/big"), &dir.path().join("fits"), 5000)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn the_real_downloader_refuses_plain_http() {
        let dir = tempfile::tempdir().unwrap();
        let base = serve(&files()).await;
        let error = Downloader::new()
            .unwrap()
            .fetch(
                &format!("{base}/{CHECKSUMS_FILE}"),
                &dir.path().join("out"),
                1024,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::Download(_)), "{error}");
    }

    #[tokio::test]
    async fn the_helper_failure_is_passed_on_and_the_folder_still_removed() {
        let work = tempfile::tempdir().unwrap();
        let base = serve(&files()).await;
        let helper = Arc::new(FakeHelper {
            failure: Some("signature mismatch".into()),
            ..FakeHelper::default()
        });
        let error = installer(work.path(), &helper)
            .install(&release_at(&base, &names()), &|_| {})
            .await
            .unwrap_err();
        assert_eq!(error.public_message(), "signature mismatch");
        assert!(!work.path().join("update").exists());
    }

    #[tokio::test]
    async fn leftovers_of_an_earlier_attempt_are_cleared() {
        let work = tempfile::tempdir().unwrap();
        let stale = work.path().join("update");
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("old"), b"old").unwrap();
        let base = serve(&files()).await;
        let helper = Arc::new(FakeHelper::default());
        installer(work.path(), &helper)
            .install(&release_at(&base, &names()), &|_| {})
            .await
            .unwrap();
        assert!(
            helper
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|(name, _)| name != "old")
        );
    }

    #[test]
    fn polkit_refusals_and_helper_messages_are_summarised() {
        assert_eq!(
            helper_failure(Some(126), "Error executing command as another user"),
            "the system refused the installation (polkit)"
        );
        assert_eq!(
            helper_failure(
                Some(1),
                "apt output\nhubergamof-update-helper: bad\u{1b}[0m\n\n"
            ),
            "hubergamof-update-helper: bad[0m"
        );
        assert_eq!(helper_failure(Some(1), ""), "the update helper failed");
        assert_eq!(
            helper_failure(None, &"x".repeat(1000)).chars().count(),
            MAX_MESSAGE_CHARS
        );
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

    #[test]
    fn the_download_folder_follows_the_xdg_cache() {
        // Only the shape: the environment of the test process is not touched.
        if let Some(dir) = download_dir_from_env() {
            assert!(dir.ends_with("hubergamof/update"));
            assert!(dir.is_absolute());
        }
    }
}
