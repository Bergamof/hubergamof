//! How a model numbers its buttons, and which model a device name means.
//!
//! The control ids come from `hubergamof mouse-probe` (`docs/mouse-hidpp.md`);
//! the haptic panel's, `0x01A0`, is named "Haptic" by Solaar but not confirmed
//! on the mouse itself.

use hubergamof_protocol::{MouseControlId, MouseModel};

/// Control id of the left click.
pub const CID_LEFT: u16 = 0x0050;
/// Control id of the right click.
pub const CID_RIGHT: u16 = 0x0051;
const CID_WHEEL_CLICK: u16 = 0x0052;
const CID_BACK: u16 = 0x0053;
const CID_FORWARD: u16 = 0x0056;
const CID_GESTURE: u16 = 0x00C3;
const CID_WHEEL_MODE: u16 = 0x00C4;
const CID_HAPTIC_PANEL: u16 = 0x01A0;

/// The mouse's own id for a control, `None` when the model lacks it or it is
/// not a button (the thumb wheel).
#[must_use]
pub fn cid(model: MouseModel, control: MouseControlId) -> Option<u16> {
    model.capabilities().control(control)?;
    Some(match control {
        MouseControlId::Left => CID_LEFT,
        MouseControlId::Right => CID_RIGHT,
        MouseControlId::WheelClick => CID_WHEEL_CLICK,
        MouseControlId::WheelMode => CID_WHEEL_MODE,
        MouseControlId::Back => CID_BACK,
        MouseControlId::Forward => CID_FORWARD,
        MouseControlId::Gesture => CID_GESTURE,
        MouseControlId::HapticPanel => CID_HAPTIC_PANEL,
        MouseControlId::ThumbWheel => return None,
    })
}

/// The control a model gives this id to.
#[must_use]
pub fn control_of(model: MouseModel, cid_value: u16) -> Option<MouseControlId> {
    model
        .capabilities()
        .controls
        .iter()
        .map(|control| control.id)
        .find(|id| cid(model, *id) == Some(cid_value))
}

/// Left and right click are never diverted, whatever asks for it.
#[must_use]
pub fn is_locked_cid(cid_value: u16) -> bool {
    matches!(cid_value, CID_LEFT | CID_RIGHT)
}

/// The model a device calls itself (`DeviceNameAndType`), `None` for any other.
#[must_use]
pub fn identify(name: &str) -> Option<MouseModel> {
    [MouseModel::MxMaster4, MouseModel::MxAnywhere3S]
        .into_iter()
        .find(|model| model.display_name().eq_ignore_ascii_case(name.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODELS: [MouseModel; 2] = [MouseModel::MxMaster4, MouseModel::MxAnywhere3S];

    #[test]
    fn every_button_of_every_model_has_its_own_id_and_comes_back() {
        for model in MODELS {
            let mut seen = Vec::new();
            for control in model.capabilities().controls {
                let Some(id) = cid(model, control.id) else {
                    assert_eq!(control.id, MouseControlId::ThumbWheel, "{model}");
                    continue;
                };
                assert!(!seen.contains(&id), "{model}: {id:#06x} twice");
                seen.push(id);
                assert_eq!(control_of(model, id), Some(control.id), "{model}");
            }
        }
    }

    #[test]
    fn ids_are_the_ones_the_probe_read() {
        assert_eq!(
            cid(MouseModel::MxMaster4, MouseControlId::Gesture),
            Some(0x00C3)
        );
        assert_eq!(
            cid(MouseModel::MxMaster4, MouseControlId::HapticPanel),
            Some(0x01A0)
        );
        assert_eq!(
            cid(MouseModel::MxAnywhere3S, MouseControlId::WheelMode),
            Some(0x00C4)
        );
        assert_eq!(
            cid(MouseModel::MxAnywhere3S, MouseControlId::Back),
            Some(0x0053)
        );
    }

    #[test]
    fn a_control_the_model_lacks_has_no_id() {
        assert_eq!(cid(MouseModel::MxAnywhere3S, MouseControlId::Gesture), None);
        assert_eq!(
            cid(MouseModel::MxAnywhere3S, MouseControlId::HapticPanel),
            None
        );
        assert_eq!(cid(MouseModel::MxMaster4, MouseControlId::ThumbWheel), None);
        assert_eq!(control_of(MouseModel::MxAnywhere3S, 0x00C3), None);
        assert_eq!(control_of(MouseModel::MxMaster4, 0xFFFF), None);
    }

    #[test]
    fn only_the_two_clicks_are_locked_ids() {
        for model in MODELS {
            for control in model.capabilities().controls {
                let locked = cid(model, control.id).is_some_and(is_locked_cid);
                assert_eq!(locked, control.locked, "{model} {:?}", control.id);
            }
        }
    }

    #[test]
    fn models_are_recognised_by_their_name() {
        assert_eq!(identify("MX Master 4"), Some(MouseModel::MxMaster4));
        assert_eq!(identify(" mx anywhere 3s "), Some(MouseModel::MxAnywhere3S));
        for other in ["MX Master 3S", "MX Keys", "", "MX Master 4 for Business"] {
            assert_eq!(identify(other), None, "{other:?}");
        }
    }
}
