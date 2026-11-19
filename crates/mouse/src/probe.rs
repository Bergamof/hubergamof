//! Read-only inventory of an HID++ 2.0 device: its features and its remappable
//! buttons.
//!
//! The probe sends nothing but getters: `IRoot` ping and `getFeature`,
//! `FeatureSet` count and id, the device name, and the count and description
//! of the buttons. It never sets a value, diverts a button or triggers an
//! effect.

use std::io::{self, Write};

use crate::channel::{Channel, Hidpp, HidppError};
use crate::features::{self, DEVICE_NAME, FEATURE_SET, REPROG_CONTROLS_V4};
use crate::frame::{DIRECT_DEVICE_INDEX, RECEIVER_DEVICE_INDEXES};

/// Byte a ping asks the device to echo.
const PING_ECHO: u8 = 0x5A;
/// Characters the device name getter returns per call.
const NAME_CHUNK: usize = 16;

/// A feature of a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureEntry {
    /// Index to address the feature with.
    pub index: u8,
    /// Feature id.
    pub id: u16,
    /// Type flags as the device reports them (obsolete, hidden, engineering…).
    pub flags: u8,
    /// Version of the feature.
    pub version: u8,
}

/// A button as `ReprogControlsV4` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    /// Control id (the button).
    pub cid: u16,
    /// Task id (what the button does by default).
    pub tid: u16,
    /// First flag byte (mouse, reprogrammable, divertable…).
    pub flags: u8,
    /// Second flag byte.
    pub flags2: u8,
    /// Remapping group.
    pub group: u8,
    /// Remapping group mask.
    pub group_mask: u8,
    /// Position on the device.
    pub position: u8,
}

impl Control {
    /// Names of the flags set in the first flag byte.
    #[must_use]
    pub fn flag_names(&self) -> Vec<&'static str> {
        const NAMES: [(u8, &str); 8] = [
            (0x01, "mouse"),
            (0x02, "fkey"),
            (0x04, "nonstandard"),
            (0x08, "fn-sensitive"),
            (0x10, "reprogrammable"),
            (0x20, "divertable"),
            (0x40, "persistent-divert"),
            (0x80, "virtual"),
        ];
        NAMES
            .iter()
            .filter(|(bit, _)| self.flags & bit != 0)
            .map(|(_, name)| *name)
            .collect()
    }
}

/// What the probe learned about one device.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceReport {
    /// Device index on the interface (`0xFF` for a device reached directly).
    pub index: u8,
    /// HID++ protocol version `(major, minor)`.
    pub protocol: (u8, u8),
    /// Device name.
    pub name: Option<String>,
    /// Features, `Root` included.
    pub features: Vec<FeatureEntry>,
    /// Remappable buttons.
    pub controls: Vec<Control>,
    /// Parts that could not be read.
    pub warnings: Vec<String>,
}

/// Where an interface comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceInfo {
    /// Device node, e.g. `/dev/hidraw3`.
    pub path: String,
    /// USB vendor id.
    pub vendor_id: u16,
    /// USB product id.
    pub product_id: u16,
    /// USB interface number (-1 when unknown).
    pub interface_number: i32,
    /// HID usage page.
    pub usage_page: u16,
    /// HID usage.
    pub usage: u16,
    /// Product string, when the interface has one.
    pub product: Option<String>,
}

/// The result of probing one interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceScan {
    /// The interface.
    pub info: InterfaceInfo,
    /// Why the interface could not be opened, if it could not.
    pub open_error: Option<String>,
    /// Devices that answered.
    pub devices: Vec<DeviceReport>,
    /// Frames exchanged, when tracing.
    pub trace: Vec<String>,
}

/// Probes every device index of an interface: the direct one, then the ones a
/// receiver hands out.
pub fn scan_interface<C: Channel>(hidpp: &mut Hidpp<C>) -> Vec<DeviceReport> {
    std::iter::once(DIRECT_DEVICE_INDEX)
        .chain(RECEIVER_DEVICE_INDEXES)
        .filter_map(|index| probe_device(hidpp, index))
        .collect()
}

/// Inventories the device at `device`, `None` when nothing answers there.
pub fn probe_device<C: Channel>(hidpp: &mut Hidpp<C>, device: u8) -> Option<DeviceReport> {
    let ping = hidpp
        .request(device, 0, 1, &[0, 0, PING_ECHO])
        .ok()
        .filter(|params| params.get(2) == Some(&PING_ECHO))?;
    let mut report = DeviceReport {
        index: device,
        protocol: (ping[0], ping[1]),
        ..DeviceReport::default()
    };
    if let Err(error) = read_features(hidpp, device, &mut report) {
        report.warnings.push(format!("feature list: {error}"));
    }
    match feature_index(hidpp, device, DEVICE_NAME) {
        Ok(Some(index)) => match read_name(hidpp, device, index) {
            Ok(name) => report.name = Some(name),
            Err(error) => report.warnings.push(format!("name: {error}")),
        },
        Ok(None) => {}
        Err(error) => report.warnings.push(format!("name: {error}")),
    }
    match feature_index(hidpp, device, REPROG_CONTROLS_V4) {
        Ok(Some(index)) => match read_controls(hidpp, device, index) {
            Ok(controls) => report.controls = controls,
            Err(error) => report.warnings.push(format!("buttons: {error}")),
        },
        Ok(None) => {}
        Err(error) => report.warnings.push(format!("buttons: {error}")),
    }
    Some(report)
}

/// `IRoot.getFeature`: index of a feature, `None` when the device lacks it.
fn feature_index<C: Channel>(
    hidpp: &mut Hidpp<C>,
    device: u8,
    id: u16,
) -> Result<Option<u8>, HidppError> {
    let [hi, lo] = id.to_be_bytes();
    let params = hidpp.request(device, 0, 0, &[hi, lo])?;
    Ok(params.first().copied().filter(|index| *index != 0))
}

fn read_features<C: Channel>(
    hidpp: &mut Hidpp<C>,
    device: u8,
    report: &mut DeviceReport,
) -> Result<(), HidppError> {
    report.features.push(FeatureEntry {
        index: 0,
        id: features::ROOT,
        flags: 0,
        version: 0,
    });
    let Some(set) = feature_index(hidpp, device, FEATURE_SET)? else {
        return Ok(());
    };
    let count = hidpp
        .request(device, set, 0, &[])?
        .first()
        .copied()
        .unwrap_or(0);
    // The set lists the features after Root: indexes 1 to count.
    for index in 1..=count {
        let params = hidpp.request(device, set, 1, &[index])?;
        if let [hi, lo, flags, version, ..] = params[..] {
            report.features.push(FeatureEntry {
                index,
                id: u16::from_be_bytes([hi, lo]),
                flags,
                version,
            });
        }
    }
    Ok(())
}

fn read_name<C: Channel>(
    hidpp: &mut Hidpp<C>,
    device: u8,
    index: u8,
) -> Result<String, HidppError> {
    let length = usize::from(
        hidpp
            .request(device, index, 0, &[])?
            .first()
            .copied()
            .unwrap_or(0),
    );
    let mut name = Vec::with_capacity(length);
    while name.len() < length {
        let offset = u8::try_from(name.len()).unwrap_or(u8::MAX);
        let chunk = hidpp.request(device, index, 1, &[offset])?;
        let wanted = (length - name.len()).min(NAME_CHUNK);
        name.extend(chunk.iter().take(wanted));
    }
    Ok(String::from_utf8_lossy(&name)
        .trim_end_matches('\0')
        .to_owned())
}

fn read_controls<C: Channel>(
    hidpp: &mut Hidpp<C>,
    device: u8,
    index: u8,
) -> Result<Vec<Control>, HidppError> {
    let count = hidpp
        .request(device, index, 0, &[])?
        .first()
        .copied()
        .unwrap_or(0);
    (0..count)
        .map(|control| {
            let params = hidpp.request(device, index, 1, &[control])?;
            let [
                c0,
                c1,
                t0,
                t1,
                flags,
                position,
                group,
                group_mask,
                flags2,
                ..,
            ] = params[..]
            else {
                return Err(HidppError::Channel(crate::channel::ChannelError(
                    "short button description".to_owned(),
                )));
            };
            Ok(Control {
                cid: u16::from_be_bytes([c0, c1]),
                tid: u16::from_be_bytes([t0, t1]),
                flags,
                flags2,
                group,
                group_mask,
                position,
            })
        })
        .collect()
}

/// Writes the report a person can paste back.
///
/// # Errors
/// I/O errors of `out`.
pub fn render(out: &mut impl Write, scans: &[InterfaceScan]) -> io::Result<()> {
    writeln!(
        out,
        "HuBergamof mouse-probe (read-only: no setting is written)"
    )?;
    let found: usize = scans.iter().map(|scan| scan.devices.len()).sum();
    writeln!(
        out,
        "{} interface(s) examined, {found} device(s) answered.\n",
        scans.len()
    )?;
    for scan in scans {
        let info = &scan.info;
        writeln!(
            out,
            "Interface {} {:04x}:{:04x} interface {} usage {:04x}/{:04x} {}",
            info.path,
            info.vendor_id,
            info.product_id,
            info.interface_number,
            info.usage_page,
            info.usage,
            info.product.as_deref().unwrap_or("")
        )?;
        if let Some(error) = &scan.open_error {
            writeln!(out, "  cannot open: {error}")?;
        }
        for device in &scan.devices {
            render_device(out, device)?;
        }
        if !scan.trace.is_empty() {
            writeln!(out, "  Frames:")?;
            for line in &scan.trace {
                writeln!(out, "    {line}")?;
            }
        }
        writeln!(out)?;
    }
    Ok(())
}

fn render_device(out: &mut impl Write, device: &DeviceReport) -> io::Result<()> {
    writeln!(
        out,
        "  Device index 0x{:02X}: {} (HID++ {}.{})",
        device.index,
        device.name.as_deref().unwrap_or("name unknown"),
        device.protocol.0,
        device.protocol.1
    )?;
    writeln!(out, "    Features ({}):", device.features.len())?;
    for feature in &device.features {
        writeln!(
            out,
            "      index 0x{:02X}  id 0x{:04X}  v{}  flags 0x{:02X}  {}",
            feature.index,
            feature.id,
            feature.version,
            feature.flags,
            features::feature_name(feature.id).unwrap_or("?")
        )?;
    }
    if !device.controls.is_empty() {
        writeln!(out, "    Buttons ({}):", device.controls.len())?;
        for control in &device.controls {
            writeln!(
                out,
                "      cid 0x{:04X}  task 0x{:04X}  flags 0x{:02X}/0x{:02X}  group {}/{}  pos {}  {}  [{}]",
                control.cid,
                control.tid,
                control.flags,
                control.flags2,
                control.group,
                control.group_mask,
                control.position,
                features::control_name(control.cid).unwrap_or("?"),
                control.flag_names().join(" ")
            )?;
        }
    }
    for warning in &device.warnings {
        writeln!(out, "    warning: {warning}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::time::Duration;

    use super::*;
    use crate::testing::{FakeDevice, FakeLink};

    fn mouse() -> FakeDevice {
        FakeDevice::new()
            .with_name("MX Test Mouse With A Long Name")
            .with_feature(0x2201, 2)
            .with_control(0x0050, 0x0038, 0x31, 0x00)
            .with_control(0x00C3, 0x00B4, 0x70, 0x01)
    }

    fn hidpp(link: FakeLink) -> Hidpp<FakeLink> {
        Hidpp::new(link).with_timeout(Duration::from_millis(1))
    }

    #[test]
    fn reads_features_name_and_buttons_of_a_direct_device() {
        let mut hidpp = hidpp(FakeLink::direct(mouse()));
        let devices = scan_interface(&mut hidpp);
        assert_eq!(devices.len(), 1);
        let device = &devices[0];
        assert_eq!(device.index, 0xFF);
        assert_eq!(device.protocol, (4, 5));
        assert_eq!(
            device.name.as_deref(),
            Some("MX Test Mouse With A Long Name")
        );
        let ids: Vec<u16> = device.features.iter().map(|f| f.id).collect();
        assert_eq!(ids, [0x0000, 0x0001, 0x0005, 0x2201, 0x1B04]);
        assert_eq!(device.controls.len(), 2);
        assert_eq!(device.controls[1].cid, 0x00C3);
        assert_eq!(
            device.controls[0].flag_names(),
            ["mouse", "reprogrammable", "divertable"]
        );
        assert!(device.warnings.is_empty(), "{:?}", device.warnings);
    }

    #[test]
    fn finds_every_device_behind_a_receiver() {
        let link = FakeLink::receiver(vec![
            (1, mouse()),
            (3, FakeDevice::new().with_name("Keyboard")),
        ]);
        let devices = scan_interface(&mut hidpp(link));
        let seen: Vec<(u8, Option<&str>)> = devices
            .iter()
            .map(|device| (device.index, device.name.as_deref()))
            .collect();
        assert_eq!(
            seen,
            [
                (1, Some("MX Test Mouse With A Long Name")),
                (3, Some("Keyboard"))
            ]
        );
    }

    #[test]
    fn a_silent_interface_yields_nothing() {
        assert_eq!(scan_interface(&mut hidpp(FakeLink::silent())).len(), 0);
    }

    #[test]
    fn a_device_without_buttons_reports_none() {
        let link = FakeLink::direct(FakeDevice::new().with_feature(REPROG_CONTROLS_V4, 4));
        let device = scan_interface(&mut hidpp(link)).remove(0);
        assert_eq!(device.controls.len(), 0);
        assert_eq!(device.warnings.len(), 0);
    }

    #[test]
    fn a_failing_part_becomes_a_warning_and_the_rest_is_kept() {
        let link = FakeLink::direct(
            FakeDevice::new()
                .with_name("Half Broken")
                .with_control(0x0050, 0x0038, 0x31, 0)
                .with_broken_feature(REPROG_CONTROLS_V4),
        );
        let device = scan_interface(&mut hidpp(link)).remove(0);
        assert_eq!(device.name.as_deref(), Some("Half Broken"));
        assert_eq!(device.controls.len(), 0);
        assert_eq!(device.warnings.len(), 1);
        assert!(device.warnings[0].starts_with("buttons: HID++ 2.0 error 0x07"));
    }

    #[test]
    fn the_report_names_what_it_knows_and_marks_the_rest() {
        let mut hidpp = hidpp(FakeLink::direct(mouse().with_feature(0xBEEF, 1)));
        let device = scan_interface(&mut hidpp).remove(0);
        let scan = InterfaceScan {
            info: InterfaceInfo {
                path: "/dev/hidraw9".to_owned(),
                vendor_id: 0x046D,
                product_id: 0xC548,
                interface_number: 2,
                usage_page: 0xFF00,
                usage: 1,
                product: Some("Bolt Receiver".to_owned()),
            },
            open_error: Some("permission denied".to_owned()),
            devices: vec![device],
            trace: vec!["-> 11 ff".to_owned()],
        };
        let mut out = Vec::new();
        render(&mut out, &[scan]).unwrap();
        let text = String::from_utf8(out).unwrap();
        for expected in [
            "1 interface(s) examined, 1 device(s) answered.",
            "046d:c548",
            "cannot open: permission denied",
            "MX Test Mouse With A Long Name (HID++ 4.5)",
            "id 0x1B04  v4  flags 0x00  ReprogControlsV4",
            "id 0xBEEF  v1  flags 0x00  ?",
            "cid 0x00C3",
            "gesture button",
            "[mouse reprogrammable divertable]",
            "-> 11 ff",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
    }
}
