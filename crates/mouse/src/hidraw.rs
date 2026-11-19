//! Discovery and `hidapi` channel on real hardware.

use std::time::Duration;

use hidapi::{DeviceInfo, HidApi, HidDevice};
use thiserror::Error;

use crate::channel::{Channel, ChannelError, Hidpp};
use crate::probe::{InterfaceInfo, InterfaceScan, scan_interface};

/// USB vendor id of Logitech.
pub const LOGITECH_VENDOR_ID: u16 = 0x046D;
/// HID usage pages that carry HID++ reports: vendor page of receivers and
/// wired devices, and the one of Bluetooth LE devices.
const HIDPP_USAGE_PAGES: [u16; 2] = [0xFF00, 0xFF43];
/// Largest report read from an interface (very long HID++ report).
const READ_BUFFER_LEN: usize = 64;

/// The HID layer could not be started.
#[derive(Debug, Error)]
#[error("cannot list HID devices: {0}")]
pub struct HidError(String);

/// [`Channel`] over an open `hidraw` node.
#[derive(Debug)]
pub struct HidrawChannel {
    device: HidDevice,
}

impl Channel for HidrawChannel {
    fn write(&mut self, report: &[u8]) -> Result<(), ChannelError> {
        self.device
            .write(report)
            .map(|_| ())
            .map_err(|error| ChannelError(error.to_string()))
    }

    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, ChannelError> {
        let mut buffer = [0_u8; READ_BUFFER_LEN];
        let millis = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
        match self.device.read_timeout(&mut buffer, millis) {
            Ok(0) => Ok(None),
            Ok(read) => Ok(Some(buffer[..read].to_vec())),
            Err(error) => Err(ChannelError(error.to_string())),
        }
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
    for info in api.device_list().filter(|info| {
        info.vendor_id() == LOGITECH_VENDOR_ID && HIDPP_USAGE_PAGES.contains(&info.usage_page())
    }) {
        let mut scan = InterfaceScan {
            info: describe(info),
            open_error: None,
            devices: Vec::new(),
            trace: Vec::new(),
        };
        match api.open_path(info.path()) {
            Ok(device) => {
                let mut hidpp = Hidpp::new(HidrawChannel { device });
                if trace {
                    hidpp = hidpp.with_trace();
                }
                scan.devices = scan_interface(&mut hidpp);
                scan.trace = hidpp.take_trace();
            }
            Err(error) => scan.open_error = Some(error.to_string()),
        }
        scans.push(scan);
    }
    Ok(scans)
}
