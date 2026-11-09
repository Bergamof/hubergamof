//! Choosing a file with the desktop's own dialog, for the UI: a browser never
//! reveals the path of a file it opens, and a compose file needs its real
//! path (its `.env`, build contexts and `env_file`s sit next to it).
//!
//! The dialog is the one of the `org.freedesktop.portal.FileChooser`
//! interface of `xdg-desktop-portal` (KDE, GNOME…), opened on the user's
//! screen; the user, not the UI, picks the file, and only its path comes
//! back. Without a portal (no session bus, headless) the UI falls back to
//! typing the path.

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt as _;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt as _;
use thiserror::Error;
use zbus::{Connection, ObjectPath, OwnedObjectPath, OwnedVariant, Proxy, Variant, proxy};

/// Longest time the dialog may stay open before it is given up.
pub const DIALOG_TIMEOUT: Duration = Duration::from_mins(10);

/// What the chosen file is for: sets the dialog's title and filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePurpose {
    /// A compose file of the Docker module.
    ComposeFile,
}

impl FilePurpose {
    fn title(self) -> &'static str {
        match self {
            Self::ComposeFile => "Choisir un fichier compose",
        }
    }

    /// Dialog filters: a name and its globs, the file kinds of the purpose
    /// first, then everything (compose files may have any name).
    fn filters(self) -> Vec<(&'static str, Vec<&'static str>)> {
        match self {
            Self::ComposeFile => vec![
                ("Fichiers compose", vec!["*.yaml", "*.yml"]),
                ("Tous les fichiers", vec!["*"]),
            ],
        }
    }
}

/// Why no file came back.
#[derive(Debug, Error)]
pub enum FilePickerError {
    /// There is no dialog to open (no desktop portal).
    #[error("no file chooser is available on this desktop")]
    Unavailable,
    /// The dialog failed or was not answered in time.
    #[error("the file chooser failed: {0}")]
    Failed(String),
}

/// Opens a dialog and returns the file the user chose.
#[async_trait]
pub trait FilePicker: Send + Sync {
    /// The absolute path of the chosen file, `None` when the user closed the
    /// dialog without choosing.
    ///
    /// # Errors
    ///
    /// [`FilePickerError`] when there is no dialog, or it failed.
    async fn pick_file(&self, purpose: FilePurpose) -> Result<Option<PathBuf>, FilePickerError>;
}

/// No dialog (used where no desktop is wanted).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoFilePicker;

#[async_trait]
impl FilePicker for NoFilePicker {
    async fn pick_file(&self, _purpose: FilePurpose) -> Result<Option<PathBuf>, FilePickerError> {
        Err(FilePickerError::Unavailable)
    }
}

/// A dialog answering at once with a given file (`--demo` and tests).
#[derive(Debug, Clone)]
pub struct SimulatedFilePicker {
    chosen: Option<PathBuf>,
}

impl SimulatedFilePicker {
    /// A user always choosing `chosen`; `None`: always closing the dialog.
    #[must_use]
    pub fn choosing(chosen: Option<PathBuf>) -> Self {
        Self { chosen }
    }
}

#[async_trait]
impl FilePicker for SimulatedFilePicker {
    async fn pick_file(&self, _purpose: FilePurpose) -> Result<Option<PathBuf>, FilePickerError> {
        Ok(self.chosen.clone())
    }
}

/// The dialog of `xdg-desktop-portal`, over the session bus.
#[derive(Debug, Clone, Copy, Default)]
pub struct PortalFilePicker;

#[proxy(
    interface = "org.freedesktop.portal.FileChooser",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait PortalFileChooser {
    /// Opens the dialog; the answer comes as the `Response` signal of the
    /// returned request.
    fn open_file(
        &self,
        parent_window: &str,
        title: &str,
        options: HashMap<&str, Variant<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;
}

const PORTAL_SERVICE: &str = "org.freedesktop.portal.Desktop";
const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
const REQUEST_PREFIX: &str = "/org/freedesktop/portal/desktop/request";
/// `Response` code of a chosen file (1: the user cancelled, 2: anything else).
const RESPONSE_SUCCESS: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;

#[async_trait]
impl FilePicker for PortalFilePicker {
    async fn pick_file(&self, purpose: FilePurpose) -> Result<Option<PathBuf>, FilePickerError> {
        let connection = Connection::session()
            .await
            .map_err(|_| FilePickerError::Unavailable)?;
        let token = request_token();
        let path = request_path(&connection, &token)?;
        let answer = with_timeout(
            DIALOG_TIMEOUT,
            ask_portal(&connection, purpose, &token, path.clone()),
        )
        .await;
        if matches!(answer, Err(FilePickerError::Failed(_))) {
            // The dialog may still be on screen (no answer in time): close it, so that the
            // next one is the only one.
            close_request(&connection, path).await;
        }
        answer
    }
}

/// `future`, given up after `limit`.
async fn with_timeout<T>(
    limit: Duration,
    future: impl std::future::Future<Output = Result<T, FilePickerError>>,
) -> Result<T, FilePickerError> {
    tokio::time::timeout(limit, future)
        .await
        .map_err(|_| FilePickerError::Failed("no answer from the dialog".to_owned()))?
}

/// Closes the dialog of request `path`, best effort (it may be gone already).
async fn close_request(connection: &Connection, path: OwnedObjectPath) {
    let closed = async {
        Proxy::new(connection, PORTAL_SERVICE, path, REQUEST_INTERFACE)
            .await?
            .call_method("Close", &())
            .await
    };
    if let Err(error) = closed.await {
        tracing::debug!(%error, "file dialog request already closed");
    }
}

async fn ask_portal(
    connection: &Connection,
    purpose: FilePurpose,
    token: &str,
    path: OwnedObjectPath,
) -> Result<Option<PathBuf>, FilePickerError> {
    let chooser = PortalFileChooserProxy::new(connection)
        .await
        .map_err(|_| FilePickerError::Unavailable)?;
    // Listen before asking: the answer must not be missed.
    let request = Proxy::new(connection, PORTAL_SERVICE, path, REQUEST_INTERFACE)
        .await
        .map_err(failed)?;
    let mut responses = request.receive_signal("Response").await.map_err(failed)?;

    let options = HashMap::from([
        ("handle_token", Variant::from(token)),
        ("modal", Variant::from(true)),
        ("multiple", Variant::from(false)),
        ("filters", Variant::new(portal_filters(purpose))),
    ]);
    chooser
        .open_file("", purpose.title(), options)
        .await
        .map_err(|error| match error {
            zbus::Error::MethodError(..) | zbus::Error::NameTaken => FilePickerError::Unavailable,
            other => failed(other),
        })?;

    let response = responses
        .next()
        .await
        .ok_or_else(|| FilePickerError::Failed("the dialog closed without answering".to_owned()))?;
    let (code, results): (u32, HashMap<String, OwnedVariant>) =
        response.body().deserialize().map_err(failed)?;
    interpret_response(code, &results)
}

/// What the `Response` of the portal says: the chosen file, `None` when the user closed the
/// dialog, an error for anything else.
fn interpret_response(
    code: u32,
    results: &HashMap<String, OwnedVariant>,
) -> Result<Option<PathBuf>, FilePickerError> {
    match code {
        RESPONSE_SUCCESS => Ok(Some(chosen_path(results)?)),
        RESPONSE_CANCELLED => Ok(None),
        other => Err(FilePickerError::Failed(format!("response code {other}"))),
    }
}

fn failed(error: impl std::fmt::Display) -> FilePickerError {
    FilePickerError::Failed(error.to_string())
}

type PortalFilter = (String, Vec<(u32, String)>);

/// The filters as the portal reads them: `a(sa(us))`, globs of type 0.
fn portal_filters(purpose: FilePurpose) -> Vec<PortalFilter> {
    purpose
        .filters()
        .into_iter()
        .map(|(name, globs)| {
            (
                name.to_owned(),
                globs.into_iter().map(|glob| (0, glob.to_owned())).collect(),
            )
        })
        .collect()
}

/// A token of a request, unique to this process and call.
fn request_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "hubergamof_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Path of the request the portal creates for `token`, from the unique bus name of the
/// caller.
fn request_path(connection: &Connection, token: &str) -> Result<OwnedObjectPath, FilePickerError> {
    let sender = connection
        .unique_name()
        .ok_or_else(|| FilePickerError::Failed("no bus name".to_owned()))?;
    request_path_of(sender.as_str(), token)
}

/// The caller's name without its colon and with `_` for dots, then the token.
fn request_path_of(sender: &str, token: &str) -> Result<OwnedObjectPath, FilePickerError> {
    let sender = sender.trim_start_matches(':').replace('.', "_");
    ObjectPath::try_from(format!("{REQUEST_PREFIX}/{sender}/{token}"))
        .map(Into::into)
        .map_err(failed)
}

/// The one file of the `uris` of a successful response.
fn chosen_path(results: &HashMap<String, OwnedVariant>) -> Result<PathBuf, FilePickerError> {
    let uris: Vec<String> = results
        .get("uris")
        .ok_or_else(|| FilePickerError::Failed("no file in the answer".to_owned()))?
        .try_clone()
        .and_then(Vec::<String>::try_from)
        .map_err(failed)?;
    uris.first()
        .and_then(|uri| path_of_uri(uri))
        .ok_or_else(|| FilePickerError::Failed("the file has no local path".to_owned()))
}

/// The absolute path of a `file://` URI, percent-decoded; `None` for any
/// other URI, a remote host, or a relative path.
fn path_of_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    let mut bytes = Vec::with_capacity(path.len());
    let mut remaining = path.bytes();
    while let Some(byte) = remaining.next() {
        if byte == b'%' {
            let high = hex_digit(remaining.next()?)?;
            let low = hex_digit(remaining.next()?)?;
            bytes.push(high << 4 | low);
        } else {
            bytes.push(byte);
        }
    }
    (!bytes.contains(&0)).then(|| PathBuf::from(OsString::from_vec(bytes)))
}

fn hex_digit(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_uri_is_a_percent_decoded_absolute_path() {
        let path = |uri| path_of_uri(uri);
        assert_eq!(
            path("file:///home/me/app/compose.yaml"),
            Some("/home/me/app/compose.yaml".into())
        );
        assert_eq!(
            path("file:///home/me/My%20Apps/%C3%A9t%C3%A9.yml"),
            Some("/home/me/My Apps/été.yml".into())
        );
        assert_eq!(
            path("file://localhost/srv/a.yaml"),
            Some("/srv/a.yaml".into())
        );
    }

    #[test]
    fn other_uris_are_refused() {
        for uri in [
            "",
            "https://example.com/a.yaml",
            "file://remote-host/a.yaml",
            "file://relative/a.yaml",
            "file:///a%zz.yaml",
            "file:///a%2",
            "file:///a%00b.yaml",
        ] {
            assert_eq!(path_of_uri(uri), None, "{uri}");
        }
    }

    #[test]
    fn the_portal_names_its_requests_after_the_caller() {
        let path = request_path_of(":1.42", "tok").unwrap();
        assert_eq!(
            path.as_str(),
            "/org/freedesktop/portal/desktop/request/1_42/tok"
        );
        assert!(request_path_of(":1.42", "bad token").is_err());
    }

    fn variants(uris: &[&str]) -> HashMap<String, OwnedVariant> {
        let uris: Vec<String> = uris.iter().map(|uri| (*uri).to_owned()).collect();
        HashMap::from([(
            "uris".to_owned(),
            OwnedVariant::try_from(Variant::new(uris)).unwrap(),
        )])
    }

    #[test]
    fn the_response_code_tells_chosen_closed_or_failed() {
        let chosen = variants(&["file:///srv/a.yaml", "file:///srv/b.yaml"]);
        assert_eq!(
            interpret_response(RESPONSE_SUCCESS, &chosen).unwrap(),
            Some("/srv/a.yaml".into()),
            "the first file"
        );
        assert_eq!(
            interpret_response(RESPONSE_CANCELLED, &HashMap::new()).unwrap(),
            None
        );
        assert!(matches!(
            interpret_response(2, &HashMap::new()),
            Err(FilePickerError::Failed(_))
        ));
        for results in [
            HashMap::new(),
            variants(&[]),
            variants(&["https://example.com/a.yaml"]),
        ] {
            assert!(matches!(
                interpret_response(RESPONSE_SUCCESS, &results),
                Err(FilePickerError::Failed(_))
            ));
        }
    }

    #[test]
    fn the_filters_have_the_signature_of_the_portal() {
        use zbus::zvariant::Type as _;
        assert_eq!(<Vec<PortalFilter>>::SIGNATURE.to_string(), "a(sa(us))");
    }

    #[tokio::test(start_paused = true)]
    async fn a_dialog_nobody_answers_is_given_up() {
        let waiting = std::future::pending::<Result<(), FilePickerError>>();
        assert!(matches!(
            with_timeout(DIALOG_TIMEOUT, waiting).await,
            Err(FilePickerError::Failed(message)) if message.contains("no answer")
        ));
        assert!(
            with_timeout(DIALOG_TIMEOUT, std::future::ready(Ok(())))
                .await
                .is_ok()
        );
    }

    #[test]
    fn requests_have_distinct_tokens() {
        assert_ne!(request_token(), request_token());
    }

    #[test]
    fn the_filters_follow_the_portal_signature() {
        let filters = portal_filters(FilePurpose::ComposeFile);
        assert_eq!(filters[0].0, "Fichiers compose");
        assert_eq!(
            filters[0].1,
            [(0, "*.yaml".to_owned()), (0, "*.yml".to_owned())]
        );
        assert_eq!(filters.last().unwrap().1, [(0, "*".to_owned())]);
    }

    #[tokio::test]
    async fn simulated_and_missing_pickers_answer_without_a_desktop() {
        let chosen = SimulatedFilePicker::choosing(Some("/srv/a.yaml".into()));
        assert_eq!(
            chosen.pick_file(FilePurpose::ComposeFile).await.unwrap(),
            Some("/srv/a.yaml".into())
        );
        assert_eq!(
            SimulatedFilePicker::choosing(None)
                .pick_file(FilePurpose::ComposeFile)
                .await
                .unwrap(),
            None
        );
        assert!(matches!(
            NoFilePicker.pick_file(FilePurpose::ComposeFile).await,
            Err(FilePickerError::Unavailable)
        ));
    }
}
