//! Names of the HID++ 2.0 features and button identifiers this project knows.
//! Anything else is shown by its number, which is what the probe is for.

/// Feature id of `IRoot`, always at index 0.
pub const ROOT: u16 = 0x0000;
/// Feature id of `IFeatureSet`, which lists the other features.
pub const FEATURE_SET: u16 = 0x0001;
/// Feature id of the device name and type.
pub const DEVICE_NAME: u16 = 0x0005;
/// Feature id of the remappable buttons (version 4).
pub const REPROG_CONTROLS_V4: u16 = 0x1B04;

/// Name of a feature id, when known.
#[must_use]
pub fn feature_name(id: u16) -> Option<&'static str> {
    Some(match id {
        ROOT => "Root",
        FEATURE_SET => "FeatureSet",
        0x0002 => "FeatureInfo",
        0x0003 => "DeviceFirmwareInformation",
        DEVICE_NAME => "DeviceNameAndType",
        0x1000 => "BatteryStatus",
        0x1004 => "UnifiedBattery",
        0x1814 => "ChangeHost",
        0x1815 => "HostsInfo",
        0x1B00 => "ReprogControls",
        REPROG_CONTROLS_V4 => "ReprogControlsV4",
        0x1D4B => "WirelessDeviceStatus",
        0x2100 => "VerticalScrolling",
        0x2110 => "SmartShift",
        0x2111 => "SmartShiftEnhanced",
        0x2120 => "HiResScrolling",
        0x2121 => "HiResWheel",
        0x2150 => "ThumbWheel",
        0x2200 => "MousePointer",
        0x2201 => "AdjustableDpi",
        0x2205 => "PointerSpeed",
        0x8100 => "OnboardProfiles",
        0x8110 => "MouseButtonSpy",
        _ => return None,
    })
}

/// Name of a button identifier (control id), when known.
#[must_use]
pub fn control_name(cid: u16) -> Option<&'static str> {
    Some(match cid {
        0x0050 => "left click",
        0x0051 => "right click",
        0x0052 => "middle click",
        0x0053 => "back",
        0x0056 => "forward",
        0x00C3 => "gesture button",
        0x00C4 => "SmartShift (wheel mode) button",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown_ids() {
        assert_eq!(feature_name(0x1B04), Some("ReprogControlsV4"));
        assert_eq!(feature_name(0xBEEF), None);
        assert_eq!(control_name(0x0053), Some("back"));
        assert_eq!(control_name(0x9999), None);
    }
}
