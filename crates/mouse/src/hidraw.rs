//! Discovery and `hidapi` channel on real hardware.

use std::collections::HashSet;
use std::time::Duration;

use hidapi::{DeviceInfo, HidApi, HidDevice};
use thiserror::Error;

use hubergamof_protocol::MouseModel;

use crate::channel::{Channel, ChannelError, Hidpp, HidppError};
use crate::frame::{DIRECT_DEVICE_INDEX, RECEIVER_DEVICE_INDEXES};
use crate::link::{MouseLink, UnitId};
use crate::probe::{InterfaceInfo, InterfaceScan, ping, scan_interface_with_misses};

/// USB vendor id of Logitech.
pub const LOGITECH_VENDOR_ID: u16 = 0x046D;
/// HID++ collections as `(usage page, usage)`: short and long reports on the
/// vendor page of receivers and wired devices, and the Bluetooth LE one.
const HIDPP_COLLECTIONS: [(u16, u16); 3] = [(0xFF00, 0x0001), (0xFF00, 0x0002), (0xFF43, 0x0202)];
/// Product ids of Logitech bootloaders (firmware update), which speak another protocol.
const BOOTLOADER_PRODUCTS: std::ops::RangeInclusive<u16> = 0xAA00..=0xAAFF;
/// Largest report read from an interface (very long HID++ report).
const READ_BUFFER_LEN: usize = 64;

/// The HID layer could not be started.
#[derive(Debug, Error)]
#[error("cannot list HID devices: {0}")]
pub struct HidError(String);

/// What [`HidrawChannel`] needs from an open HID node.
pub trait RawHid {
    /// Writes one report.
    ///
    /// # Errors
    /// The message of the failure.
    fn write_report(&mut self, report: &[u8]) -> Result<(), String>;

    /// Reads one report into `buffer`, waiting at most `millis`; the number of
    /// bytes read, 0 on timeout.
    ///
    /// # Errors
    /// The message of the failure.
    fn read_report(&mut self, buffer: &mut [u8], millis: i32) -> Result<usize, String>;
}

impl RawHid for HidDevice {
    fn write_report(&mut self, report: &[u8]) -> Result<(), String> {
        self.write(report)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn read_report(&mut self, buffer: &mut [u8], millis: i32) -> Result<usize, String> {
        self.read_timeout(buffer, millis)
            .map_err(|error| error.to_string())
    }
}

/// [`Channel`] over an open `hidraw` node.
#[derive(Debug)]
pub struct HidrawChannel<D = HidDevice> {
    device: D,
}

impl<D: RawHid> HidrawChannel<D> {
    /// Wraps an open node.
    #[must_use]
    pub fn new(device: D) -> Self {
        Self { device }
    }
}

/// Milliseconds to wait for a read: rounded up, and at least 1 because 0 would
/// make the read non-blocking.
#[must_use]
pub fn timeout_millis(timeout: Duration) -> i32 {
    let millis = timeout.as_micros().div_ceil(1000).max(1);
    i32::try_from(millis).unwrap_or(i32::MAX)
}

impl<D: RawHid> Channel for HidrawChannel<D> {
    fn write(&mut self, report: &[u8]) -> Result<(), ChannelError> {
        self.device.write_report(report).map_err(ChannelError)
    }

    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, ChannelError> {
        let mut buffer = [0_u8; READ_BUFFER_LEN];
        match self
            .device
            .read_report(&mut buffer, timeout_millis(timeout))
        {
            Ok(0) => Ok(None),
            Ok(read) => Ok(Some(buffer[..read.min(READ_BUFFER_LEN)].to_vec())),
            Err(message) => Err(ChannelError(message)),
        }
    }
}

/// Why an interface is not probed, `None` when it is: a Logitech HID++
/// collection that is not a bootloader. Input interfaces (keyboard, mouse) and
/// other protocols that share a usage page are never written to.
#[must_use]
pub fn skip_reason(
    vendor_id: u16,
    product_id: u16,
    usage_page: u16,
    usage: u16,
) -> Option<&'static str> {
    if vendor_id != LOGITECH_VENDOR_ID {
        Some("not a Logitech device")
    } else if BOOTLOADER_PRODUCTS.contains(&product_id) {
        Some("firmware update mode")
    } else if !HIDPP_COLLECTIONS.contains(&(usage_page, usage)) {
        Some("not an HID++ collection")
    } else {
        None
    }
}

fn describe(info: &DeviceInfo) -> InterfaceInfo {
    InterfaceInfo {
        path: info.path().to_string_lossy().into_owned(),
        vendor_id: info.vendor_id(),
        product_id: info.product_id(),
        interface_number: info.interface_number(),
        usage_page: info.usage_page(),
        usage: info.usage(),
        product: info.product_string().map(str::to_owned),
    }
}

/// Probes every Logitech HID++ interface of the machine.
///
/// With `trace`, each interface's scan carries the frames exchanged.
///
/// # Errors
/// [`HidError`] when the HID layer cannot be started; an interface that cannot
/// be opened is reported in its own scan instead.
pub fn scan_hardware(trace: bool) -> Result<Vec<InterfaceScan>, HidError> {
    let api = HidApi::new().map_err(|error| HidError(error.to_string()))?;
    let mut scans = Vec::new();
    let mut probed = HashSet::new();
    for info in api
        .device_list()
        .filter(|info| info.vendor_id() == LOGITECH_VENDOR_ID)
    {
        let mut scan = InterfaceScan {
            info: describe(info),
            skipped: skip_reason(
                info.vendor_id(),
                info.product_id(),
                info.usage_page(),
                info.usage(),
            )
            .map(str::to_owned),
            open_error: None,
            misses: Vec::new(),
            devices: Vec::new(),
            trace: Vec::new(),
        };
        // hidapi lists one entry per collection: a node is probed once.
        if scan.skipped.is_none() && !probed.insert(info.path().to_owned()) {
            continue;
        }
        if scan.skipped.is_none() {
            match api.open_path(info.path()) {
                Ok(device) => {
                    let mut hidpp = Hidpp::new(HidrawChannel::new(device));
                    if trace {
                        hidpp = hidpp.with_trace();
                    }
                    let probe = scan_interface_with_misses(&mut hidpp);
                    scan.devices = probe.devices;
                    scan.misses = probe
                        .misses
                        .iter()
                        .map(|(index, miss)| format!("index 0x{index:02X}: {}", miss.describe()))
                        .collect();
                    scan.trace = hidpp.take_trace();
                }
                Err(error) => scan.open_error = Some(error.to_string()),
            }
        }
        scans.push(scan);
    }
    Ok(scans)
}

/// Why no mouse could be chosen.
#[derive(Debug, Error)]
pub enum FindError {
    /// The HID layer could not be started.
    #[error(transparent)]
    Hid(#[from] HidError),
    /// No supported mouse answered.
    #[error(
        "no supported mouse answered: move it to wake it, check that it is paired with the Logi Bolt receiver and that its hidraw node is open to you (docs/mouse-probe.md)"
    )]
    NoMouse,
    /// More than one answered.
    #[error("several mice answered ({0}): say which with --model")]
    Several(String),
    /// The mouse refused something while being set up.
    #[error(transparent)]
    Mouse(#[from] HidppError),
}

/// Where a mouse was found: its hidraw node and its index on that node (the
/// receiver gives one index to each paired mouse).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MouseKey {
    /// The hidraw node.
    pub path: String,
    /// The device index on it.
    pub index: u8,
}

/// A connected mouse and where it was found.
pub struct FoundMouse {
    /// Where.
    pub key: MouseKey,
    /// Its unit id, `None` when it could not be read: what tells one physical
    /// mouse from another when two paths lead to it.
    pub unit: Option<UnitId>,
    /// The mouse.
    pub link: MouseLink<HidrawChannel>,
}

/// Every supported mouse on the Logitech HID++ interfaces, except the places
/// in `owned`, which are not even spoken to: another thread reads their
/// answers, and a request here could be mistaken for one of its own.
///
/// A mouse that answers but cannot be set up is left out; the last such
/// failure is returned with the mice. A mouse reached twice (two nodes) is
/// returned once, the first found.
///
/// Requests carry one fixed software id: another program speaking HID++ to
/// the same mouse at the same time (`mouse-test`, Solaar, logiops) can be
/// mistaken for this one. Stop the others first.
///
/// # Errors
/// [`FindError::Hid`] when the HID layer cannot start.
pub fn find_mice(owned: &[MouseKey]) -> Result<(Vec<FoundMouse>, Option<HidppError>), FindError> {
    let api = HidApi::new().map_err(|error| HidError(error.to_string()))?;
    let mut probed = HashSet::new();
    let mut found: Vec<FoundMouse> = Vec::new();
    let mut setup_error = None;
    for info in api.device_list().filter(|info| {
        info.vendor_id() == LOGITECH_VENDOR_ID
            && skip_reason(
                info.vendor_id(),
                info.product_id(),
                info.usage_page(),
                info.usage(),
            )
            .is_none()
    }) {
        if !probed.insert(info.path().to_owned()) {
            continue;
        }
        let path = info.path().to_string_lossy().into_owned();
        for index in std::iter::once(DIRECT_DEVICE_INDEX).chain(RECEIVER_DEVICE_INDEXES) {
            let key = MouseKey {
                path: path.clone(),
                index,
            };
            if owned.contains(&key) {
                continue;
            }
            let Ok(device) = api.open_path(info.path()) else {
                break;
            };
            let mut hidpp = Hidpp::new(HidrawChannel::new(device));
            if ping(&mut hidpp, index).is_err() {
                continue;
            }
            match MouseLink::connect(hidpp, index) {
                Ok(mut link) => {
                    let unit = link.unit_id().ok();
                    if unit.is_some() && found.iter().any(|other| other.unit == unit) {
                        continue;
                    }
                    found.push(FoundMouse { key, unit, link });
                }
                // A device that is not a supported mouse: not worth reporting.
                Err(HidppError::NotSupportedMouse(_)) => {}
                Err(error) => setup_error = Some(error),
            }
        }
    }
    Ok((found, setup_error))
}

/// Finds a supported mouse among the Logitech HID++ interfaces: the one of the
/// wanted model, or the only one when none is wanted.
///
/// # Errors
/// [`FindError`] when none, or more than one, answers.
pub fn find_mouse(wanted: Option<MouseModel>) -> Result<MouseLink<HidrawChannel>, FindError> {
    let (found, setup_error) = find_mice(&[])?;
    let candidates = found
        .into_iter()
        .map(|mouse| (mouse.link.model(), mouse.link.device(), mouse.link))
        .collect();
    pick(candidates, wanted, setup_error)
}

/// The mouse to use among those that answered: the only one of the wanted
/// model (or the only one at all when none is wanted).
fn pick<T>(
    mut found: Vec<(MouseModel, u8, T)>,
    wanted: Option<MouseModel>,
    setup_error: Option<HidppError>,
) -> Result<T, FindError> {
    found.retain(|(model, _, _)| wanted.is_none_or(|wanted| wanted == *model));
    match found.len() {
        0 => Err(setup_error.map_or(FindError::NoMouse, FindError::Mouse)),
        1 => Ok(found.remove(0).2),
        _ => Err(FindError::Several(
            found
                .iter()
                .map(|(model, index, _)| format!("{model} at index {index}"))
                .collect::<Vec<_>>()
                .join(", "),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scripted node: what `read_report` returns, and the writes it saw.
    struct FakeHid {
        read: Result<Vec<u8>, String>,
        seen_millis: Option<i32>,
        written: Vec<Vec<u8>>,
        write_error: Option<String>,
    }

    impl FakeHid {
        fn answering(read: Result<Vec<u8>, String>) -> Self {
            Self {
                read,
                seen_millis: None,
                written: Vec::new(),
                write_error: None,
            }
        }
    }

    impl RawHid for FakeHid {
        fn write_report(&mut self, report: &[u8]) -> Result<(), String> {
            if let Some(error) = &self.write_error {
                return Err(error.clone());
            }
            self.written.push(report.to_vec());
            Ok(())
        }

        fn read_report(&mut self, buffer: &mut [u8], millis: i32) -> Result<usize, String> {
            self.seen_millis = Some(millis);
            let bytes = self.read.clone()?;
            let count = bytes.len().min(buffer.len());
            buffer[..count].copy_from_slice(&bytes[..count]);
            Ok(count)
        }
    }

    #[test]
    fn timeouts_round_up_and_never_become_non_blocking() {
        assert_eq!(timeout_millis(Duration::ZERO), 1);
        assert_eq!(timeout_millis(Duration::from_micros(1)), 1);
        assert_eq!(timeout_millis(Duration::from_micros(1500)), 2);
        assert_eq!(timeout_millis(Duration::from_millis(500)), 500);
        assert_eq!(timeout_millis(Duration::MAX), i32::MAX);
    }

    #[test]
    fn only_hidpp_collections_of_logitech_devices_are_probed() {
        let probed =
            |vendor, product, page, usage| skip_reason(vendor, product, page, usage).is_none();
        assert!(probed(0x046D, 0xC548, 0xFF00, 1));
        assert!(probed(0x046D, 0xC548, 0xFF00, 2));
        assert!(probed(0x046D, 0xB042, 0xFF43, 0x0202));
        assert!(!probed(0x046D, 0xC548, 0xFF00, 4), "DJ collection");
        assert!(
            !probed(0x046D, 0xC548, 0x0001, 2),
            "generic desktop (input)"
        );
        assert!(!probed(0x046D, 0xC548, 0x000C, 1), "consumer control");
        assert!(!probed(0x046D, 0xAA01, 0xFF00, 1), "bootloader");
        assert!(!probed(0x0FD9, 0x0080, 0xFF00, 1), "another vendor");
        assert_eq!(
            skip_reason(0x046D, 0xAA01, 0xFF00, 1),
            Some("firmware update mode")
        );
    }

    #[test]
    fn a_read_returns_the_bytes_and_asks_for_the_rounded_timeout() {
        let mut channel = HidrawChannel::new(FakeHid::answering(Ok(vec![0x11, 1, 2])));
        let read = channel.read(Duration::from_micros(2500)).unwrap();
        assert_eq!(read, Some(vec![0x11, 1, 2]));
        assert_eq!(channel.device.seen_millis, Some(3));
    }

    #[test]
    fn an_empty_read_is_a_timeout_and_errors_are_passed_on() {
        let mut quiet = HidrawChannel::new(FakeHid::answering(Ok(Vec::new())));
        assert_eq!(quiet.read(Duration::from_millis(5)).unwrap(), None);
        let mut broken = HidrawChannel::new(FakeHid::answering(Err("unplugged".to_owned())));
        assert_eq!(
            broken.read(Duration::from_millis(5)).unwrap_err(),
            ChannelError("unplugged".to_owned())
        );
    }

    #[test]
    fn a_write_reaches_the_node_and_its_error_is_passed_on() {
        let mut channel = HidrawChannel::new(FakeHid::answering(Ok(Vec::new())));
        channel.write(&[0x11, 0xFF]).unwrap();
        assert_eq!(channel.device.written, [vec![0x11, 0xFF]]);
        channel.device.write_error = Some("EIO".to_owned());
        assert_eq!(
            channel.write(&[0x11]).unwrap_err(),
            ChannelError("EIO".to_owned())
        );
    }

    #[test]
    fn nothing_answering_is_reported_as_no_mouse() {
        let error = pick::<()>(Vec::new(), None, None).unwrap_err();
        assert!(matches!(error, FindError::NoMouse), "{error}");
    }

    #[test]
    fn a_setup_failure_is_reported_when_nothing_else_answered() {
        let error =
            pick::<()>(Vec::new(), None, Some(HidppError::Timeout(Duration::ZERO))).unwrap_err();
        assert!(
            matches!(error, FindError::Mouse(HidppError::Timeout(Duration::ZERO))),
            "{error}"
        );
    }

    #[test]
    fn the_only_mouse_is_chosen_with_or_without_a_wanted_model() {
        let found = || vec![(MouseModel::MxMaster4, 2, "master")];
        assert_eq!(pick(found(), None, None).unwrap(), "master");
        assert_eq!(
            pick(found(), Some(MouseModel::MxMaster4), None).unwrap(),
            "master"
        );
    }

    #[test]
    fn the_wanted_model_picks_among_several_and_a_missing_one_finds_nothing() {
        let found = || {
            vec![
                (MouseModel::MxAnywhere3S, 1, "anywhere"),
                (MouseModel::MxMaster4, 2, "master"),
            ]
        };
        assert_eq!(
            pick(found(), Some(MouseModel::MxAnywhere3S), None).unwrap(),
            "anywhere"
        );
        let error = pick(
            vec![(MouseModel::MxMaster4, 2, ())],
            Some(MouseModel::MxAnywhere3S),
            None,
        )
        .unwrap_err();
        assert!(matches!(error, FindError::NoMouse), "{error}");
    }

    #[test]
    fn several_mice_without_a_wanted_model_are_named() {
        let error = pick(
            vec![
                (MouseModel::MxAnywhere3S, 1, ()),
                (MouseModel::MxMaster4, 2, ()),
            ],
            None,
            None,
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(matches!(error, FindError::Several(_)));
        assert!(
            message.contains("at index 1") && message.contains("at index 2"),
            "{message}"
        );
        assert!(message.contains("--model"), "{message}");
    }

    #[test]
    fn scanning_without_hardware_ends_cleanly() {
        // A machine (or container) without udev or Logitech devices gives an
        // empty list or a HID error, never a panic.
        match scan_hardware(false) {
            Ok(scans) => assert!(
                scans
                    .iter()
                    .all(|scan| scan.info.vendor_id == LOGITECH_VENDOR_ID)
            ),
            Err(error) => assert!(error.to_string().starts_with("cannot list HID devices")),
        }
    }
}
