use std::os::unix::net::UnixStream;
use std::sync::Mutex;

use tokio::net::TcpListener;
use zbus::connection::Builder;
use zbus::{Guid, interface};

use super::*;

/// Short enough for a test, long enough for a loaded machine.
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// A service that starts listening on `address` when asked, unless told to fail.
struct FakeService {
    address: SocketAddr,
    listens_once_started: bool,
    starts: Mutex<u32>,
    listener: Mutex<Option<std::net::TcpListener>>,
}

impl FakeService {
    fn at(address: SocketAddr) -> Self {
        Self {
            address,
            listens_once_started: true,
            starts: Mutex::new(0),
            listener: Mutex::new(None),
        }
    }

    fn silent(address: SocketAddr) -> Self {
        Self {
            listens_once_started: false,
            ..Self::at(address)
        }
    }

    fn starts(&self) -> u32 {
        *self.starts.lock().unwrap()
    }
}

#[async_trait]
impl ServiceControl for FakeService {
    async fn start(&self) -> Result<(), LauncherError> {
        *self.starts.lock().unwrap() += 1;
        if self.listens_once_started {
            *self.listener.lock().unwrap() =
                Some(std::net::TcpListener::bind(self.address).unwrap());
        }
        Ok(())
    }
}

/// A service whose start is refused, as for a unit that is not installed.
struct MissingService;

#[async_trait]
impl ServiceControl for MissingService {
    async fn start(&self) -> Result<(), LauncherError> {
        Err(zbus::Error::Failure("Unit hubergamof.service not found.".into()).into())
    }
}

#[derive(Default)]
struct RecordingOpener {
    urls: Mutex<Vec<String>>,
}

impl RecordingOpener {
    fn urls(&self) -> Vec<String> {
        self.urls.lock().unwrap().clone()
    }
}

#[async_trait]
impl UrlOpener for RecordingOpener {
    async fn open(&self, url: &str) -> Result<(), LauncherError> {
        self.urls.lock().unwrap().push(url.to_owned());
        Ok(())
    }
}

/// A loopback address nothing listens on.
fn free_address() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

#[tokio::test]
async fn a_running_manager_is_opened_without_starting_the_service() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let service = FakeService::at(address);
    let opener = RecordingOpener::default();

    let outcome = open_ui(address, &service, &opener, TEST_TIMEOUT)
        .await
        .unwrap();

    assert_eq!(outcome, Outcome::AlreadyRunning);
    assert_eq!(service.starts(), 0);
    assert_eq!(opener.urls(), [format!("http://{address}/")]);
}

#[tokio::test]
async fn a_stopped_manager_is_started_then_opened() {
    let address = free_address();
    let service = FakeService::at(address);
    let opener = RecordingOpener::default();

    let outcome = open_ui(address, &service, &opener, TEST_TIMEOUT)
        .await
        .unwrap();

    assert_eq!(outcome, Outcome::Started);
    assert_eq!(service.starts(), 1);
    assert_eq!(opener.urls(), [ui_url(address)]);
}

#[tokio::test]
async fn a_refused_start_opens_nothing() {
    let opener = RecordingOpener::default();

    let error = open_ui(free_address(), &MissingService, &opener, TEST_TIMEOUT)
        .await
        .unwrap_err();

    assert!(matches!(error, LauncherError::Service(_)), "{error}");
    assert!(error.to_string().contains(SERVICE_UNIT), "{error}");
    assert_eq!(opener.urls(), Vec::<String>::new());
}

#[tokio::test]
async fn a_service_that_never_listens_times_out_with_a_hint() {
    let address = free_address();
    let service = FakeService::silent(address);
    let opener = RecordingOpener::default();
    let timeout = Duration::from_millis(300);

    let error = open_ui(address, &service, &opener, timeout)
        .await
        .unwrap_err();

    assert!(
        matches!(error, LauncherError::NotListening { address: a, timeout: t } if a == address && t == timeout),
        "{error}"
    );
    assert!(
        error.to_string().contains("journalctl --user -u"),
        "{error}"
    );
    assert_eq!(service.starts(), 1);
    assert_eq!(opener.urls(), Vec::<String>::new());
}

#[tokio::test]
async fn the_opener_reports_its_exit_status() {
    run_opener(&mut Command::new("true")).await.unwrap();
    let failed = run_opener(&mut Command::new("false")).await.unwrap_err();
    assert!(matches!(failed, LauncherError::OpenerFailed(status) if !status.success()));
    let missing = run_opener(&mut Command::new("/nonexistent/hubergamof-opener"))
        .await
        .unwrap_err();
    assert!(
        matches!(missing, LauncherError::OpenerUnavailable(_)),
        "{missing}"
    );
}

/// What the fake systemd manager was asked, in order.
#[derive(Default, Clone)]
struct Calls(std::sync::Arc<Mutex<Vec<String>>>);

impl Calls {
    fn push(&self, call: String) {
        self.0.lock().unwrap().push(call);
    }

    fn list(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// `org.freedesktop.systemd1.Manager`, knowing only `known_unit`.
struct FakeSystemd {
    known_unit: &'static str,
    calls: Calls,
}

impl FakeSystemd {
    fn check(&self, name: &str) -> zbus::fdo::Result<()> {
        if name == self.known_unit {
            Ok(())
        } else {
            Err(zbus::fdo::Error::Failed(format!("Unit {name} not found.")))
        }
    }
}

#[interface(name = "org.freedesktop.systemd1.Manager")]
impl FakeSystemd {
    fn start_unit(&self, name: &str, mode: &str) -> zbus::fdo::Result<OwnedObjectPath> {
        self.calls.push(format!("start {name} {mode}"));
        self.check(name)?;
        Ok(OwnedObjectPath::try_from("/org/freedesktop/systemd1/job/1").unwrap())
    }

    fn reset_failed_unit(&self, name: &str) -> zbus::fdo::Result<()> {
        self.calls.push(format!("reset {name}"));
        self.check(name)
    }
}

/// Peer-to-peer connection to a fake systemd manager (no bus daemon).
async fn connect_to_fake_systemd(calls: &Calls) -> (Connection, Connection) {
    let (server_socket, client_socket) = UnixStream::pair().unwrap();
    let fake = FakeSystemd {
        known_unit: SERVICE_UNIT,
        calls: calls.clone(),
    };
    let server = Builder::unix_stream(server_socket)
        .serve_at("/org/freedesktop/systemd1", fake)
        .server(Guid::generate())
        .p2p()
        .build();
    let client = Builder::unix_stream(client_socket).p2p().build();
    let (server, client) = tokio::join!(server, client);
    (server.unwrap(), client.unwrap())
}

#[tokio::test]
async fn the_unit_is_reset_then_started_in_replace_mode() {
    let calls = Calls::default();
    let (_server, client) = connect_to_fake_systemd(&calls).await;

    start_unit(&client, SERVICE_UNIT).await.unwrap();

    assert_eq!(
        calls.list(),
        [
            format!("reset {SERVICE_UNIT}"),
            format!("start {SERVICE_UNIT} replace")
        ]
    );
}

#[tokio::test]
async fn an_unknown_unit_is_reported_by_its_start() {
    let calls = Calls::default();
    let (_server, client) = connect_to_fake_systemd(&calls).await;

    let error = start_unit(&client, "other.service").await.unwrap_err();

    assert!(matches!(error, LauncherError::Service(_)), "{error}");
    assert!(
        error.to_string().contains("other.service not found"),
        "{error}"
    );
    assert_eq!(
        calls.list().len(),
        2,
        "the failed reset does not stop the start"
    );
}
