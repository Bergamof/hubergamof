//! The RFCOMM connection to the headphones, through BlueZ.
//!
//! This crate cannot open a Bluetooth socket itself (`unsafe` is forbidden).
//! BlueZ does it: this process exports an `org.bluez.Profile1` object for the
//! Sony service, asks BlueZ to connect that profile on the chosen device, and
//! BlueZ hands over the connected socket as a file descriptor.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use thiserror::Error;
use tokio::sync::{Mutex, mpsc};
use zbus::fdo::{DBusProxy, ObjectManagerProxy};
use zbus::names::BusName;
use zbus::{Connection, proxy};
use zbus::{ObjectPath, OwnedObjectPath, OwnedVariant, Variant};

use crate::channel::{Channel, ChannelError};

/// The Bluetooth service the Sony MDR protocol is served on.
pub const SONY_SERVICE_UUID: &str = "96cc203e-5068-46ad-b32d-e316f5e069ba";

/// Where this process exports its profile object.
const PROFILE_PATH: &str = "/com/hubergamof/headset";
/// How long BlueZ is given to connect the profile.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Bytes read at once.
const READ_BUFFER: usize = 4096;
/// A read timeout of zero would mean "no timeout".
const MIN_READ_TIMEOUT: Duration = Duration::from_millis(1);
/// Longest device name kept: names come from the headphones.
const MAX_NAME_CHARS: usize = 64;

const BLUEZ_SERVICE: &str = "org.bluez";

/// Why the connection failed.
#[derive(Debug, Error)]
pub enum BluezError {
    /// The system bus or BlueZ cannot be reached.
    #[error("BlueZ is not reachable: {0}")]
    Bus(#[from] zbus::Error),
    /// A D-Bus call failed.
    #[error("BlueZ refused: {0}")]
    Call(#[from] zbus::fdo::Error),
    /// No paired device offers the Sony service.
    #[error("no paired Sony headphones found (is Bluetooth on and the headphones paired?)")]
    NoDevice,
    /// The wanted address is not among them.
    #[error("no Sony headphones with address {0}")]
    UnknownAddress(String),
    /// Several offer it and none was chosen.
    #[error("several Sony headphones: choose one with --address ({0})")]
    Ambiguous(String),
    /// BlueZ did not hand over a connection in time.
    #[error("the headphones did not open the connection in time (is another app using it?)")]
    Timeout,
}

/// Headphones BlueZ knows and that offer the Sony service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadsetDevice {
    /// D-Bus path of the device.
    pub path: String,
    /// Bluetooth address, `AA:BB:CC:DD:EE:FF`.
    pub address: String,
    /// Name to show (from the headphones: cleaned).
    pub name: String,
    /// Whether the headphones are connected to this computer now.
    pub connected: bool,
}

/// Characters that draw nothing or turn the text around (zero-width marks,
/// bidirectional controls, the soft hyphen, the byte order mark): a name that
/// holds them can pass for another one.
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
    )
}

/// What the headphones sent, cleaned of control and invisible characters.
fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control() && !is_invisible(*c))
        .take(MAX_NAME_CHARS)
        .collect()
}

type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedVariant>>>;

fn property<T: TryFrom<OwnedVariant>>(
    properties: &HashMap<String, OwnedVariant>,
    name: &str,
) -> Option<T> {
    T::try_from(properties.get(name)?.try_clone().ok()?).ok()
}

/// The Sony headphones among BlueZ's objects.
#[must_use]
pub fn sony_devices(objects: &ManagedObjects) -> Vec<HeadsetDevice> {
    let mut devices: Vec<HeadsetDevice> = objects
        .iter()
        .filter_map(|(path, interfaces)| {
            let device = interfaces.get("org.bluez.Device1")?;
            let uuids: Vec<String> = property(device, "UUIDs")?;
            if !uuids
                .iter()
                .any(|uuid| uuid.eq_ignore_ascii_case(SONY_SERVICE_UUID))
            {
                return None;
            }
            let address: String = property(device, "Address")?;
            let name: String = property(device, "Alias")
                .or_else(|| property(device, "Name"))
                .unwrap_or_else(|| address.clone());
            Some(HeadsetDevice {
                path: path.to_string(),
                address,
                name: clean_name(&name),
                connected: property(device, "Connected").unwrap_or(false),
            })
        })
        .collect();
    devices.sort_by(|a, b| a.address.cmp(&b.address));
    devices
}

/// Which of `devices` to use: the one at `address`, else the only one, else
/// the only connected one.
///
/// # Errors
/// [`BluezError`] when none fits or the choice is ambiguous.
pub fn choose<'a>(
    devices: &'a [HeadsetDevice],
    address: Option<&str>,
) -> Result<&'a HeadsetDevice, BluezError> {
    if let Some(address) = address {
        return devices
            .iter()
            .find(|device| device.address.eq_ignore_ascii_case(address))
            .ok_or_else(|| BluezError::UnknownAddress(address.to_owned()));
    }
    let connected: Vec<&HeadsetDevice> = devices.iter().filter(|d| d.connected).collect();
    match (devices, connected.as_slice()) {
        ([], _) => Err(BluezError::NoDevice),
        ([only], _) => Ok(only),
        (_, [only]) => Ok(*only),
        _ => Err(BluezError::Ambiguous(
            devices
                .iter()
                .map(|d| format!("{} {}", d.address, d.name))
                .collect::<Vec<_>>()
                .join(", "),
        )),
    }
}

#[proxy(
    interface = "org.bluez.ProfileManager1",
    default_service = "org.bluez",
    default_path = "/org/bluez"
)]
trait ProfileManager {
    fn register_profile(
        &self,
        profile: &ObjectPath<'_>,
        uuid: &str,
        options: HashMap<&str, Variant<'_>>,
    ) -> zbus::Result<()>;

    fn unregister_profile(&self, profile: &ObjectPath<'_>) -> zbus::Result<()>;
}

#[proxy(interface = "org.bluez.Device1", default_service = "org.bluez")]
trait Device {
    fn connect_profile(&self, uuid: &str) -> zbus::Result<()>;
}

mod profile {
    //! The profile object BlueZ calls back with the connected socket.
    //!
    //! The three methods and their arguments are BlueZ's `Profile1` interface:
    //! the signatures cannot change, whether this profile uses an argument or
    //! not, and the macro reads the underscore-prefixed names.
    #![allow(
        clippy::needless_pass_by_value,
        clippy::unused_self,
        clippy::used_underscore_binding
    )]

    use std::collections::HashMap;
    use std::os::fd::OwnedFd;

    use tokio::sync::mpsc;
    use zbus::message::Header;
    use zbus::{ObjectPath, OwnedVariant, interface};

    /// Hands the connected socket of the Sony service to whoever opens the
    /// link, if it is the connection asked for.
    pub struct SonyProfile {
        pub connections: mpsc::UnboundedSender<OwnedFd>,
        /// D-Bus path of the headphones the link is opened to.
        pub device: String,
        /// Unique bus name of BlueZ, the only caller accepted; `None` when the
        /// sender cannot be known (a peer-to-peer connection, in the tests).
        pub bluez: Option<String>,
    }

    #[interface(name = "org.bluez.Profile1")]
    impl SonyProfile {
        fn release(&self) {}

        fn new_connection(
            &self,
            device: ObjectPath<'_>,
            fd: zbus::OwnedFd,
            _properties: HashMap<String, OwnedVariant>,
            #[zbus(header)] header: Header<'_>,
        ) {
            let from_bluez = self.bluez.as_deref().is_none_or(|owner| {
                header
                    .sender()
                    .is_some_and(|sender| sender.as_str() == owner)
            });
            if device.as_str() == self.device && from_bluez {
                // The reader is gone when the attempt timed out: nothing to do.
                let _ = self.connections.send(fd.into());
            } else {
                // The socket is closed by dropping it.
                tracing::warn!("a Sony connection that was not asked for was refused");
            }
        }

        fn request_disconnection(&self, _device: ObjectPath<'_>) {}
    }
}

use profile::SonyProfile;

/// A connected RFCOMM socket as a [`Channel`].
#[derive(Debug)]
pub struct RfcommChannel {
    stream: UnixStream,
}

impl RfcommChannel {
    /// A channel on a connected socket (read and written with plain socket calls).
    #[must_use]
    pub fn new(socket: OwnedFd) -> Self {
        Self {
            stream: UnixStream::from(socket),
        }
    }
}

impl Channel for RfcommChannel {
    fn write(&mut self, bytes: &[u8]) -> Result<(), ChannelError> {
        self.stream
            .write_all(bytes)
            .map_err(|error| io_error(&error))
    }

    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, ChannelError> {
        self.stream
            .set_read_timeout(Some(timeout.max(MIN_READ_TIMEOUT)))
            .map_err(|error| io_error(&error))?;
        let mut buffer = [0u8; READ_BUFFER];
        match self.stream.read(&mut buffer) {
            Ok(0) => Err(ChannelError::Closed),
            Ok(count) => Ok(Some(buffer[..count].to_vec())),
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                Ok(None)
            }
            Err(error) => Err(io_error(&error)),
        }
    }
}

/// A reset or broken connection is the headphones closing it; the rest is a failure.
fn io_error(error: &std::io::Error) -> ChannelError {
    match error.kind() {
        ErrorKind::ConnectionReset | ErrorKind::BrokenPipe | ErrorKind::ConnectionAborted => {
            ChannelError::Closed
        }
        _ => ChannelError::Io(error.to_string()),
    }
}

/// The system bus, to look for Sony headphones and to open the link to one.
///
/// The profile BlueZ calls back with the socket is registered only while a
/// link is being opened, so that nothing holds the Sony service the rest of
/// the time (`hubergamof headset-test` takes it too).
pub struct BluezHeadsets {
    bus: Connection,
    /// One link is opened at a time: they share the profile path.
    opening: Mutex<()>,
}

impl BluezHeadsets {
    /// Connects to the system bus.
    ///
    /// # Errors
    /// [`BluezError`] when the bus cannot be reached.
    pub async fn start() -> Result<Self, BluezError> {
        Ok(Self {
            bus: Connection::system().await?,
            opening: Mutex::new(()),
        })
    }

    /// The Sony headphones BlueZ knows.
    ///
    /// # Errors
    /// [`BluezError`] when BlueZ cannot be reached.
    pub async fn devices(&self) -> Result<Vec<HeadsetDevice>, BluezError> {
        Ok(sony_devices(&managed_objects(&self.bus).await?))
    }

    /// Opens the link to `device`: BlueZ connects the Sony service and hands
    /// the socket over.
    ///
    /// # Errors
    /// [`BluezError`] when BlueZ refuses or does not open the connection in time.
    pub async fn connect(&self, device: &HeadsetDevice) -> Result<RfcommChannel, BluezError> {
        let _opening = self.opening.lock().await;
        let (sender, mut connections) = mpsc::unbounded_channel();
        let owner = DBusProxy::new(&self.bus)
            .await?
            .get_name_owner(BusName::try_from(BLUEZ_SERVICE)?)
            .await?;
        self.bus
            .object_server()
            .at(
                PROFILE_PATH,
                SonyProfile {
                    connections: sender,
                    device: device.path.clone(),
                    bluez: Some(owner.to_string()),
                },
            )
            .await?;
        let outcome = self.connect_registered(device, &mut connections).await;
        // Whatever happened, the profile is not kept: the socket outlives it.
        self.release_profile().await;
        outcome
    }

    async fn connect_registered(
        &self,
        device: &HeadsetDevice,
        connections: &mut mpsc::UnboundedReceiver<OwnedFd>,
    ) -> Result<RfcommChannel, BluezError> {
        let options = HashMap::from([
            ("Role", Variant::from("client")),
            ("RequireAuthentication", Variant::from(false)),
            ("RequireAuthorization", Variant::from(false)),
        ]);
        ProfileManagerProxy::new(&self.bus)
            .await?
            .register_profile(
                &ObjectPath::try_from(PROFILE_PATH)?,
                SONY_SERVICE_UUID,
                options,
            )
            .await?;
        let proxy = DeviceProxy::builder(&self.bus)
            .path(device.path.as_str())
            .build()
            .await?;
        let socket = tokio::time::timeout(CONNECT_TIMEOUT, async {
            let (connected, socket) =
                tokio::join!(proxy.connect_profile(SONY_SERVICE_UUID), connections.recv());
            connected?;
            Ok::<_, BluezError>(socket)
        })
        .await
        .map_err(|_| BluezError::Timeout)??
        .ok_or(BluezError::Timeout)?;
        Ok(RfcommChannel::new(socket))
    }

    /// Unregisters and removes the profile; failures are only logged (BlueZ
    /// forgets a profile whose owner leaves the bus).
    async fn release_profile(&self) {
        match ObjectPath::try_from(PROFILE_PATH) {
            Ok(path) => {
                if let Ok(manager) = ProfileManagerProxy::new(&self.bus).await
                    && let Err(error) = manager.unregister_profile(&path).await
                {
                    tracing::debug!(%error, "the Sony profile was not unregistered");
                }
            }
            Err(error) => tracing::debug!(%error, "invalid profile path"),
        }
        if let Err(error) = self
            .bus
            .object_server()
            .remove::<SonyProfile, _>(PROFILE_PATH)
            .await
        {
            tracing::debug!(%error, "the Sony profile object was not removed");
        }
    }
}

/// A connection to the headphones: keep it while the channel is in use.
pub struct BluezConnection {
    _headsets: BluezHeadsets,
    /// The headphones this is connected to.
    pub device: HeadsetDevice,
    /// The stream to them.
    pub channel: RfcommChannel,
}

async fn managed_objects(bus: &Connection) -> Result<ManagedObjects, BluezError> {
    let manager = ObjectManagerProxy::builder(bus)
        .destination(BLUEZ_SERVICE)
        .path("/")
        .build()
        .await?;
    Ok(manager
        .get_managed_objects()
        .await?
        .into_iter()
        .map(|(path, interfaces)| {
            (
                path,
                interfaces
                    .into_iter()
                    .map(|(name, properties)| (name.to_string(), properties))
                    .collect(),
            )
        })
        .collect())
}

/// The Sony headphones BlueZ knows.
///
/// # Errors
/// [`BluezError`] when BlueZ cannot be reached.
pub async fn list_devices() -> Result<Vec<HeadsetDevice>, BluezError> {
    BluezHeadsets::start().await?.devices().await
}

/// Connects to the headphones at `address` (or the only ones there are).
///
/// # Errors
/// [`BluezError`] when BlueZ cannot be reached, no device fits, or the
/// connection is not opened in time.
pub async fn connect(address: Option<&str>) -> Result<BluezConnection, BluezError> {
    let headsets = BluezHeadsets::start().await?;
    let devices = headsets.devices().await?;
    let device = choose(&devices, address)?.clone();
    let channel = headsets.connect(&device).await?;
    Ok(BluezConnection {
        _headsets: headsets,
        device,
        channel,
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use zbus::{Fd, Guid, connection::Builder};

    use super::*;

    fn device_properties(
        uuids: &[&str],
        name: &str,
        connected: bool,
    ) -> HashMap<String, HashMap<String, OwnedVariant>> {
        let value = |v: Variant<'_>| OwnedVariant::try_from(v).unwrap();
        HashMap::from([(
            "org.bluez.Device1".to_owned(),
            HashMap::from([
                (
                    "Address".to_owned(),
                    value(Variant::from("AA:BB:CC:DD:EE:01")),
                ),
                ("Alias".to_owned(), value(Variant::from(name))),
                ("Connected".to_owned(), value(Variant::from(connected))),
                (
                    "UUIDs".to_owned(),
                    value(Variant::from(
                        uuids.iter().map(|u| (*u).to_owned()).collect::<Vec<_>>(),
                    )),
                ),
            ]),
        )])
    }

    fn path(text: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from(text).unwrap()
    }

    #[test]
    fn only_devices_offering_the_sony_service_are_listed() {
        let objects: ManagedObjects = HashMap::from([
            (
                path("/org/bluez/hci0/dev_01"),
                device_properties(
                    &[
                        "0000110b-0000-1000-8000-00805f9b34fb",
                        "96CC203E-5068-46AD-B32D-E316F5E069BA",
                    ],
                    "WH-1000XM4",
                    true,
                ),
            ),
            (
                path("/org/bluez/hci0/dev_02"),
                device_properties(&["0000110b-0000-1000-8000-00805f9b34fb"], "Speaker", true),
            ),
            (path("/org/bluez/hci0"), HashMap::new()),
        ]);
        assert_eq!(
            sony_devices(&objects),
            [HeadsetDevice {
                path: "/org/bluez/hci0/dev_01".into(),
                address: "AA:BB:CC:DD:EE:01".into(),
                name: "WH-1000XM4".into(),
                connected: true,
            }]
        );
    }

    #[test]
    fn a_name_from_the_headphones_is_cleaned_and_bounded() {
        assert_eq!(clean_name("WH\u{1b}[2J-1000\nXM4"), "WH[2J-1000XM4");
        assert_eq!(clean_name(&"x".repeat(500)).chars().count(), MAX_NAME_CHARS);
    }

    fn device(address: &str, connected: bool) -> HeadsetDevice {
        HeadsetDevice {
            path: format!("/dev_{address}"),
            address: address.into(),
            name: "WH".into(),
            connected,
        }
    }

    #[test]
    fn the_device_is_chosen_by_address_then_by_being_alone_or_connected() {
        let one = [device("A", false)];
        assert_eq!(choose(&one, None).unwrap().address, "A");
        let two = [device("A", false), device("B", true)];
        assert_eq!(choose(&two, None).unwrap().address, "B");
        assert_eq!(choose(&two, Some("a")).unwrap().address, "A");
        assert!(matches!(
            choose(&two, Some("C")),
            Err(BluezError::UnknownAddress(_))
        ));
        let both = [device("A", true), device("B", true)];
        assert!(matches!(choose(&both, None), Err(BluezError::Ambiguous(_))));
        assert!(matches!(choose(&[], None), Err(BluezError::NoDevice)));
    }

    const DEVICE_PATH: &str = "/org/bluez/hci0/dev_01";

    /// A profile served on one end of a peer-to-peer connection, and the
    /// other end to call it from.
    async fn served(
        bluez: Option<&str>,
    ) -> (
        zbus::Connection,
        zbus::Connection,
        mpsc::UnboundedReceiver<OwnedFd>,
    ) {
        let (connections, handed_over) = mpsc::unbounded_channel();
        let (server_socket, client_socket) = UnixStream::pair().unwrap();
        let profile = SonyProfile {
            connections,
            device: DEVICE_PATH.into(),
            bluez: bluez.map(str::to_owned),
        };
        let server = Builder::unix_stream(server_socket)
            .serve_at(PROFILE_PATH, profile)
            .server(Guid::generate())
            .p2p()
            .build();
        let client = Builder::unix_stream(client_socket).p2p().build();
        let (server, client) = tokio::join!(server, client);
        (server.unwrap(), client.unwrap(), handed_over)
    }

    /// Calls `NewConnection` for `device` with one end of a socket pair;
    /// gives the other end back.
    async fn new_connection(client: &zbus::Connection, device: &str) -> UnixStream {
        let (bluez_end, headphones_end) = UnixStream::pair().unwrap();
        client
            .call_method(
                None::<&str>,
                PROFILE_PATH,
                Some("org.bluez.Profile1"),
                "NewConnection",
                &(
                    ObjectPath::try_from(device).unwrap(),
                    Fd::from(&bluez_end),
                    HashMap::<String, Variant<'_>>::new(),
                ),
            )
            .await
            .unwrap();
        headphones_end
    }

    #[tokio::test]
    async fn the_socket_bluez_hands_over_becomes_a_working_channel() {
        let (_server, client, mut handed_over) = served(None).await;
        let mut headphones = new_connection(&client, DEVICE_PATH).await;

        let mut channel = RfcommChannel::new(handed_over.recv().await.unwrap());
        headphones.write_all(&[1, 2, 3]).unwrap();
        assert_eq!(
            channel.read(Duration::from_secs(1)).unwrap(),
            Some(vec![1, 2, 3])
        );
        channel.write(&[9]).unwrap();
        let mut byte = [0u8; 1];
        headphones.read_exact(&mut byte).unwrap();
        assert_eq!(byte, [9]);

        assert_eq!(channel.read(Duration::from_millis(5)).unwrap(), None);
        drop(headphones);
        assert!(channel.read(Duration::from_secs(1)).is_err());
    }

    #[tokio::test]
    async fn a_connection_of_other_headphones_is_refused_and_closed() {
        let (_server, client, mut handed_over) = served(None).await;
        let mut headphones = new_connection(&client, "/org/bluez/hci0/dev_02").await;
        assert!(handed_over.try_recv().is_err(), "nothing was handed over");
        // The refused socket was dropped: its other end sees the end of the stream.
        let mut byte = [0u8; 1];
        assert_eq!(headphones.read(&mut byte).unwrap(), 0);
    }

    #[tokio::test]
    async fn a_connection_not_sent_by_bluez_is_refused() {
        // Without a bus there is no sender: it cannot be BlueZ.
        let (_server, client, mut handed_over) = served(Some(":1.42")).await;
        new_connection(&client, DEVICE_PATH).await;
        assert!(handed_over.try_recv().is_err());
    }

    #[test]
    fn invisible_characters_are_removed_from_a_name() {
        let name = "WH\u{202E}-1000\u{200B}XM4\u{FEFF}\u{2066}";
        assert_eq!(clean_name(name), "WH-1000XM4");
        assert_eq!(clean_name("Écouteurs d'Adrien"), "Écouteurs d'Adrien");
    }
}
