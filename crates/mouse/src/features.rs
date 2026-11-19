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
/// Feature id of the firmware information, which also gives the unit id.
pub const DEVICE_FIRMWARE: u16 = 0x0003;
/// Feature id of the unified battery.
pub const UNIFIED_BATTERY: u16 = 0x1004;
/// Feature id of the adjustable sensitivity.
pub const ADJUSTABLE_DPI: u16 = 0x2201;
/// Feature id of the wheel mode (ratchet, free spin, `SmartShift`).
pub const SMART_SHIFT_ENHANCED: u16 = 0x2111;
/// Feature id of the high resolution wheel.
pub const HIRES_WHEEL: u16 = 0x2121;
/// Feature id of the thumb wheel.
pub const THUMB_WHEEL: u16 = 0x2150;
/// Feature id of the haptic feedback (seen on the MX Master 4).
pub const HAPTIC: u16 = 0x19B0;

/// Name of a feature id, when known.
#[must_use]
pub fn feature_name(id: u16) -> Option<&'static str> {
    Some(match id {
        ROOT => "Root",
        FEATURE_SET => "FeatureSet",
        0x0002 => "FeatureInfo",
        DEVICE_FIRMWARE => "DeviceFirmwareInformation",
        DEVICE_NAME => "DeviceNameAndType",
        0x1000 => "BatteryStatus",
        UNIFIED_BATTERY => "UnifiedBattery",
        0x1814 => "ChangeHost",
        0x1815 => "HostsInfo",
        0x1B00 => "ReprogControls",
        REPROG_CONTROLS_V4 => "ReprogControlsV4",
        0x1D4B => "WirelessDeviceStatus",
        HAPTIC => "Haptic",
        0x19C0 => "ForceSensingButton",
        0x2100 => "VerticalScrolling",
        0x2110 => "SmartShift",
        SMART_SHIFT_ENHANCED => "SmartShiftEnhanced",
        0x2120 => "HiResScrolling",
        HIRES_WHEEL => "HiResWheel",
        THUMB_WHEEL => "ThumbWheel",
        0x2200 => "MousePointer",
        ADJUSTABLE_DPI => "AdjustableDpi",
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
