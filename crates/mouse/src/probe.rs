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
use crate::frame::{DIRECT_DEVICE_INDEX, DeviceError, RECEIVER_DEVICE_INDEXES};

/// Byte a ping asks the device to echo.
const PING_ECHO: u8 = 0x5A;
/// Pings sent to an index that stays silent: a sleeping mouse needs a moment
/// (and a movement) to wake up, so each silent index is retried for a few seconds.
pub const PING_ATTEMPTS: u32 = 8;
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
    /// Why the interface was not probed, when it was left alone on purpose.
    pub skipped: Option<String>,
    /// Why the interface could not be opened, if it could not.
    pub open_error: Option<String>,
    /// Device indexes where nothing was found, with the reason.
    pub misses: Vec<String>,
    /// Devices that answered.
    pub devices: Vec<DeviceReport>,
    /// Frames exchanged, when tracing.
    pub trace: Vec<String>,
}

/// Why nothing was found at a device index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Miss {
    /// No frame came back, in any of the attempts.
    Silent {
        /// Number of pings sent.
        attempts: u32,
    },
    /// The device or its receiver answered with an error (a receiver says so
    /// when no device is paired at that index).
    Refused(DeviceError),
    /// The answer did not carry the echo byte of the ping.
    WrongEcho,
    /// The interface itself failed.
    Interface(String),
}

impl Miss {
    /// One line a person can act on.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Silent { attempts } => format!(
                "no answer after {attempts} pings (device asleep, off, or set to another channel)"
            ),
            Self::Refused(error) => format!("refused: {error}"),
            Self::WrongEcho => "answered, but not to our ping".to_owned(),
            Self::Interface(message) => format!("interface error: {message}"),
        }
    }
}

/// What probing every index of an interface found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InterfaceProbe {
    /// Devices that answered.
    pub devices: Vec<DeviceReport>,
    /// Indexes where nothing was found, in the order tried.
    pub misses: Vec<(u8, Miss)>,
}

/// Probes every device index of an interface: the direct one, then the ones a
/// receiver hands out.
pub fn scan_interface_with_misses<C: Channel>(hidpp: &mut Hidpp<C>) -> InterfaceProbe {
    let mut probe = InterfaceProbe::default();
    for index in std::iter::once(DIRECT_DEVICE_INDEX).chain(RECEIVER_DEVICE_INDEXES) {
        match probe_device(hidpp, index) {
            Ok(device) => probe.devices.push(device),
            Err(miss) => probe.misses.push((index, miss)),
        }
    }
    probe
}

/// The devices found on an interface, without the reasons for the misses.
pub fn scan_interface<C: Channel>(hidpp: &mut Hidpp<C>) -> Vec<DeviceReport> {
    scan_interface_with_misses(hidpp).devices
}

/// Pings the device at `device`, retrying while it stays silent.
pub(crate) fn ping<C: Channel>(hidpp: &mut Hidpp<C>, device: u8) -> Result<(u8, u8), Miss> {
    ping_attempts(hidpp, device, PING_ATTEMPTS)
}

/// Pings the device at `device`, retrying at most `attempts` times while it
/// stays silent.
pub(crate) fn ping_attempts<C: Channel>(
    hidpp: &mut Hidpp<C>,
    device: u8,
    attempts: u32,
) -> Result<(u8, u8), Miss> {
    for _ in 0..attempts {
        match hidpp.request(device, 0, 1, &[0, 0, PING_ECHO]) {
            Ok(params) => {
                return match params[..] {
                    [major, minor, PING_ECHO, ..] => Ok((major, minor)),
                    _ => Err(Miss::WrongEcho),
                };
            }
            Err(HidppError::Timeout(_)) => {}
            Err(HidppError::Device(error)) => return Err(Miss::Refused(error)),
            Err(other) => return Err(Miss::Interface(other.to_string())),
        }
    }
    Err(Miss::Silent { attempts })
}

/// Inventories the device at `device`, or says why nothing was found there.
///
/// # Errors
/// [`Miss`] when the device does not answer its ping.
pub fn probe_device<C: Channel>(hidpp: &mut Hidpp<C>, device: u8) -> Result<DeviceReport, Miss> {
    let protocol = ping(hidpp, device)?;
    let mut report = DeviceReport {
        index: device,
        protocol,
        ..DeviceReport::default()
    };
    if let Err(error) = read_features(hidpp, device, &mut report) {
        report.warnings.push(format!("feature list: {error}"));
    }
    match feature_index(hidpp, device, DEVICE_NAME) {
        Ok(Some(index)) => match read_name(hidpp, device, index) {
            Ok(name) => report.name = Some(name).filter(|name| !name.is_empty()),
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
    Ok(report)
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
        let [hi, lo, flags, version, ..] = params[..] else {
            return Err(HidppError::Malformed(format!(
                "feature entry of {} bytes",
                params.len()
            )));
        };
        report.features.push(FeatureEntry {
            index,
            id: u16::from_be_bytes([hi, lo]),
            flags,
            version,
        });
    }
    Ok(())
}

pub(crate) fn read_name<C: Channel>(
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
        if chunk.is_empty() {
            return Err(HidppError::Malformed("empty name chunk".to_owned()));
        }
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
                return Err(HidppError::Malformed(format!(
                    "button description of {} bytes",
                    params.len()
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

/// Text from a device or the system, safe to print: control characters (terminal
/// escape sequences) and bidirectional overrides become `?`.
#[must_use]
pub fn clean(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control()
                || ('\u{202A}'..='\u{202E}').contains(&c)
                || ('\u{2066}'..='\u{2069}').contains(&c)
            {
                '?'
            } else {
                c
            }
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
            clean(info.product.as_deref().unwrap_or(""))
        )?;
        if let Some(reason) = &scan.skipped {
            writeln!(out, "  not probed: {reason}")?;
        }
        if let Some(error) = &scan.open_error {
            writeln!(out, "  cannot open: {}", clean(error))?;
        }
        for device in &scan.devices {
            render_device(out, device)?;
        }
        for miss in &scan.misses {
            writeln!(out, "  {}", clean(miss))?;
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
        clean(device.name.as_deref().unwrap_or("name unknown")),
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
        writeln!(out, "    warning: {}", clean(warning))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::testing::{FakeDevice, FakeLink, ScriptedChannel, frame};

    fn mouse() -> FakeDevice {
        FakeDevice::new()
            .with_name("MX Test Mouse With A Long Name")
            .with_feature(0x2201, 2)
            .with_control(0x0050, 0x0038, 0x31, 0x00)
            .with_control(0x00C3, 0x00B4, 0x70, 0x01)
    }

    /// Function bytes a scripted reply must carry for `function` (software id included).
    fn function_byte(function: u8) -> u8 {
        function << 4 | crate::frame::SOFTWARE_ID
    }

    #[test]
    fn reads_features_name_and_buttons_of_a_direct_device() {
        let mut hidpp = Hidpp::new(FakeLink::direct(mouse()));
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
        assert_eq!(device.warnings, Vec::<String>::new());
    }

    #[test]
    fn button_fields_are_decoded_from_their_own_bytes() {
        let link = FakeLink::direct(FakeDevice::new().with_control_layout(
            0x1234,
            0x5678,
            0xA5,
            0x5A,
            [7, 8, 9],
        ));
        let device = scan_interface(&mut Hidpp::new(link)).remove(0);
        assert_eq!(
            device.controls,
            [Control {
                cid: 0x1234,
                tid: 0x5678,
                flags: 0xA5,
                flags2: 0x5A,
                group: 8,
                group_mask: 9,
                position: 7,
            }]
        );
    }

    #[test]
    fn finds_every_device_behind_a_receiver() {
        let link = FakeLink::receiver(vec![
            (1, mouse()),
            (3, FakeDevice::new().with_name("Keyboard")),
        ]);
        let devices = scan_interface(&mut Hidpp::new(link));
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
        let mut hidpp = Hidpp::new(FakeLink::silent());
        assert_eq!(scan_interface(&mut hidpp).len(), 0);
    }

    #[test]
    fn a_ping_with_the_wrong_echo_is_not_a_device() {
        // Echo byte 0x00 instead of the 0x5A we sent.
        let reply = frame(0xFF, 0, function_byte(1), &[4, 5, 0x00]);
        let mut hidpp = Hidpp::new(ScriptedChannel::new().then_frame(reply));
        assert_eq!(probe_device(&mut hidpp, 0xFF), Err(Miss::WrongEcho));
    }

    #[test]
    fn a_ping_refused_with_an_hidpp20_error_is_not_a_device() {
        let reply = frame(0xFF, 0xFF, 0, &[function_byte(1), 2]);
        let mut hidpp = Hidpp::new(ScriptedChannel::new().then_frame(reply));
        assert_eq!(
            probe_device(&mut hidpp, 0xFF),
            Err(Miss::Refused(DeviceError::Hidpp20(2)))
        );
    }

    #[test]
    fn a_silent_index_is_retried_then_explained() {
        let mut hidpp = Hidpp::new(FakeLink::silent());
        assert_eq!(
            probe_device(&mut hidpp, 1),
            Err(Miss::Silent {
                attempts: PING_ATTEMPTS
            })
        );
        assert_eq!(hidpp.channel().written().len(), PING_ATTEMPTS as usize);
    }

    #[test]
    fn a_sleeping_device_that_wakes_up_is_found() {
        // Two silences, then the answer to the third ping.
        let reply = frame(0xFF, 0, function_byte(1), &[4, 5, PING_ECHO]);
        let mut hidpp = Hidpp::new(
            ScriptedChannel::new()
                .then_silence()
                .then_silence()
                .then_frame(reply),
        );
        assert_eq!(ping(&mut hidpp, 0xFF), Ok((4, 5)));
    }

    #[test]
    fn the_misses_of_a_receiver_say_why() {
        let link = FakeLink::receiver(vec![(2, mouse())]);
        let probe = scan_interface_with_misses(&mut Hidpp::new(link));
        assert_eq!(probe.devices.len(), 1);
        let missed: Vec<u8> = probe.misses.iter().map(|(index, _)| *index).collect();
        assert_eq!(missed, [0xFF, 1, 3, 4, 5, 6]);
        assert!(
            probe
                .misses
                .iter()
                .all(|(_, miss)| matches!(miss, Miss::Refused(DeviceError::Hidpp10(8))))
        );
        assert!(
            probe.misses[0]
                .1
                .describe()
                .starts_with("refused: HID++ 1.0 error 0x08")
        );
        let silent = Miss::Silent { attempts: 3 }.describe();
        assert!(
            silent.contains("3 pings") && silent.contains("asleep"),
            "{silent}"
        );
        assert!(Miss::WrongEcho.describe().contains("not to our ping"));
        assert!(Miss::Interface("EIO".to_owned()).describe().contains("EIO"));
    }

    #[test]
    fn a_device_without_buttons_reports_none() {
        let link = FakeLink::direct(FakeDevice::new().with_feature(REPROG_CONTROLS_V4, 4));
        let device = scan_interface(&mut Hidpp::new(link)).remove(0);
        assert_eq!(device.controls.len(), 0);
        assert_eq!(device.warnings, Vec::<String>::new());
    }

    #[test]
    fn a_failing_part_becomes_a_warning_and_the_rest_is_kept() {
        let link = FakeLink::direct(
            FakeDevice::new()
                .with_name("Half Broken")
                .with_control(0x0050, 0x0038, 0x31, 0)
                .with_broken_feature(REPROG_CONTROLS_V4),
        );
        let device = scan_interface(&mut Hidpp::new(link)).remove(0);
        assert_eq!(device.name.as_deref(), Some("Half Broken"));
        assert_eq!(device.controls.len(), 0);
        assert_eq!(device.warnings.len(), 1);
        assert!(device.warnings[0].starts_with("buttons: HID++ 2.0 error 0x07"));
    }

    #[test]
    fn the_probe_only_sends_getters_of_the_features_it_reads() {
        let mut hidpp = Hidpp::new(FakeLink::direct(mouse()));
        let device = scan_interface(&mut hidpp).remove(0);
        let index_of = |id: u16| device.features.iter().find(|f| f.id == id).map(|f| f.index);
        let readable = [
            Some(0),
            index_of(FEATURE_SET),
            index_of(DEVICE_NAME),
            index_of(REPROG_CONTROLS_V4),
        ];
        let written = hidpp.channel().written();
        assert!(written.len() > 10, "{} reports", written.len());
        for report in written {
            let (feature_index, function) = (report[2], report[3] >> 4);
            assert!(
                readable.contains(&Some(feature_index)) && function <= 1,
                "not a getter of a read feature: {report:02x?}"
            );
        }
    }

    // Name reading, in 16-character chunks.

    fn name_of(len: usize) -> Option<String> {
        let name = "abcdefghij".repeat(26)[..len].to_owned();
        let link = FakeLink::direct(FakeDevice::new().with_name(&name));
        let device = scan_interface(&mut Hidpp::new(link)).remove(0);
        assert_eq!(device.warnings, Vec::<String>::new(), "length {len}");
        device.name
    }

    #[test]
    fn names_of_every_awkward_length_are_read_whole() {
        for len in [1, 15, 16, 17, 31, 32, 33, 255] {
            let expected = "abcdefghij".repeat(26)[..len].to_owned();
            assert_eq!(name_of(len), Some(expected), "length {len}");
        }
    }

    #[test]
    fn an_empty_name_is_no_name() {
        assert_eq!(name_of(0), None);
    }

    /// Replies for a name of `length` characters answered by `chunks`.
    fn name_script(length: u8, chunks: &[&[u8]]) -> Hidpp<ScriptedChannel> {
        let mut channel =
            ScriptedChannel::new().then_frame(frame(0xFF, 5, function_byte(0), &[length]));
        for chunk in chunks {
            channel = channel.then_frame(frame(0xFF, 5, function_byte(1), chunk));
        }
        Hidpp::new(channel)
    }

    #[test]
    fn trailing_nuls_are_dropped_and_bad_utf8_is_replaced() {
        let mut hidpp = name_script(4, &[b"ab\0\0"]);
        assert_eq!(read_name(&mut hidpp, 0xFF, 5).unwrap(), "ab");
        let mut hidpp = name_script(2, &[&[0xFF, b'a']]);
        assert_eq!(read_name(&mut hidpp, 0xFF, 5).unwrap(), "\u{FFFD}a");
    }

    #[test]
    fn a_name_longer_than_what_the_device_returns_is_an_error_not_a_loop() {
        // 20 characters announced, a 4-byte first chunk (the reply is padded to
        // the report size, so it is long), then silence.
        let mut hidpp = name_script(40, &[]).with_timeout(Duration::from_millis(5));
        assert!(matches!(
            read_name(&mut hidpp, 0xFF, 5),
            Err(HidppError::Timeout(_))
        ));
    }

    #[test]
    fn an_answer_without_parameters_cannot_stall_the_name_loop() {
        let bare = vec![0x11, 0xFF, 5, function_byte(1)];
        let channel = ScriptedChannel::new()
            .then_frame(frame(0xFF, 5, function_byte(0), &[20]))
            .then_frame(bare);
        let error = read_name(&mut Hidpp::new(channel), 0xFF, 5).unwrap_err();
        assert_eq!(error, HidppError::Malformed("empty name chunk".to_owned()));
    }

    #[test]
    fn a_short_button_description_is_malformed() {
        let short = vec![0x11, 0xFF, 6, function_byte(1), 0, 0x50, 0];
        let channel = ScriptedChannel::new()
            .then_frame(frame(0xFF, 6, function_byte(0), &[1]))
            .then_frame(short);
        let error = read_controls(&mut Hidpp::new(channel), 0xFF, 6).unwrap_err();
        assert_eq!(
            error,
            HidppError::Malformed("button description of 3 bytes".to_owned())
        );
    }

    #[test]
    fn a_short_feature_entry_is_reported_as_a_warning() {
        let short = vec![0x11, 0xFF, 1, function_byte(1), 0x10];
        let channel = ScriptedChannel::new()
            // ping, getFeature(FeatureSet) -> index 1, count 1, entry 1 (short)
            .then_frame(frame(0xFF, 0, function_byte(1), &[4, 5, PING_ECHO]))
            .then_frame(frame(0xFF, 0, function_byte(0), &[1, 0, 0]))
            .then_frame(frame(0xFF, 1, function_byte(0), &[1]))
            .then_frame(short);
        let device = probe_device(&mut Hidpp::new(channel), 0xFF).unwrap();
        assert!(device.warnings[0].starts_with("feature list: malformed answer"));
    }

    // Report format.

    fn interface(devices: Vec<DeviceReport>) -> InterfaceScan {
        InterfaceScan {
            info: InterfaceInfo {
                path: "/dev/hidraw9".to_owned(),
                vendor_id: 0x046D,
                product_id: 0xC548,
                interface_number: 2,
                usage_page: 0xFF00,
                usage: 1,
                product: Some("Bolt Receiver".to_owned()),
            },
            skipped: None,
            open_error: None,
            misses: Vec::new(),
            devices,
            trace: Vec::new(),
        }
    }

    fn rendered(scans: &[InterfaceScan]) -> String {
        let mut out = Vec::new();
        render(&mut out, scans).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn the_report_is_exactly_what_a_person_pastes_back() {
        let link = FakeLink::direct(FakeDevice::new().with_name("Mini").with_control_layout(
            0x0053,
            0x0038,
            0x31,
            0,
            [1, 2, 3],
        ));
        let device = scan_interface(&mut Hidpp::new(link)).remove(0);
        let expected = "\
HuBergamof mouse-probe (read-only: no setting is written)
1 interface(s) examined, 1 device(s) answered.

Interface /dev/hidraw9 046d:c548 interface 2 usage ff00/0001 Bolt Receiver
  Device index 0xFF: Mini (HID++ 4.5)
    Features (4):
      index 0x00  id 0x0000  v0  flags 0x00  Root
      index 0x01  id 0x0001  v0  flags 0x00  FeatureSet
      index 0x02  id 0x0005  v0  flags 0x00  DeviceNameAndType
      index 0x03  id 0x1B04  v4  flags 0x00  ReprogControlsV4
    Buttons (1):
      cid 0x0053  task 0x0038  flags 0x31/0x00  group 2/3  pos 1  back  [mouse reprogrammable divertable]

";
        assert_eq!(rendered(&[interface(vec![device])]), expected);
    }

    #[test]
    fn the_report_marks_unknown_names_and_shows_errors_warnings_and_frames() {
        let mut device = scan_interface(&mut Hidpp::new(FakeLink::direct(
            mouse().with_feature(0xBEEF, 1),
        )))
        .remove(0);
        device.warnings.push("buttons: broken".to_owned());
        let mut scan = interface(vec![device]);
        scan.open_error = Some("permission denied".to_owned());
        scan.trace = vec!["-> 11 ff".to_owned()];
        scan.info.product = None;
        let text = rendered(&[scan]);
        for expected in [
            "cannot open: permission denied",
            "id 0xBEEF  v1  flags 0x00  ?",
            "gesture button",
            "warning: buttons: broken",
            "  Frames:\n    -> 11 ff",
            "usage ff00/0001 \n",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
    }

    #[test]
    fn an_empty_scan_and_a_bare_device_render_without_gaps() {
        assert_eq!(
            rendered(&[]),
            "HuBergamof mouse-probe (read-only: no setting is written)\n0 interface(s) examined, 0 device(s) answered.\n\n"
        );
        let bare = DeviceReport {
            index: 1,
            ..DeviceReport::default()
        };
        let text = rendered(&[interface(vec![bare])]);
        assert!(
            text.contains("Device index 0x01: name unknown (HID++ 0.0)"),
            "{text}"
        );
        assert!(!text.contains("Buttons"), "{text}");
    }

    #[test]
    fn device_text_cannot_inject_terminal_sequences() {
        assert_eq!(clean("MX\u{1b}[31m \u{202E}evil\u{2066}"), "MX?[31m ?evil?");
        assert_eq!(clean("plain é"), "plain é");
        let mut device = DeviceReport {
            index: 1,
            name: Some("Mouse\u{1b}]0;title\u{7}".to_owned()),
            ..DeviceReport::default()
        };
        device.warnings.push("bad\u{1b}[2J".to_owned());
        let mut scan = interface(vec![device]);
        scan.info.product = Some("Prod\u{1b}[0m".to_owned());
        scan.skipped = Some("because".to_owned());
        scan.misses = vec!["index 0x01: no\u{1b}[0m answer".to_owned()];
        let text = rendered(&[scan]);
        assert!(!text.contains('\u{1b}'), "{text:?}");
        assert!(text.contains("Mouse?]0;title?"), "{text}");
        assert!(text.contains("not probed: because"), "{text}");
        assert!(text.contains("  index 0x01: no?[0m answer"), "{text}");
    }

    #[test]
    fn a_failing_writer_is_reported() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("closed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(render(&mut Broken, &[]).is_err());
    }
}
