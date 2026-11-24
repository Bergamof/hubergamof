//! The types of the headphones crate and those of the wire contract describe
//! the same things; this is where one is turned into the other. The crate
//! knows nothing of the contract on purpose, and every `match` here lists its
//! variants, so that a new one on either side fails to compile until it is
//! handled.

use hubergamof_headset::payload::{
    AmbientMode as WireMode, AmbientSetting, AmbientState, Battery, EqualizerPreset as WirePreset,
    EqualizerSetting, EqualizerState, Event,
};
use hubergamof_protocol::{
    AmbientMode, AmbientSound, Equalizer, EqualizerChange, EqualizerPreset, HeadsetBattery,
    HeadsetState,
};

fn ambient_mode(mode: WireMode) -> AmbientMode {
    match mode {
        WireMode::Off => AmbientMode::Off,
        WireMode::Ambient => AmbientMode::Ambient,
        WireMode::NoiseCancelling => AmbientMode::NoiseCancelling,
        WireMode::WindReduction => AmbientMode::WindReduction,
    }
}

fn wire_mode(mode: AmbientMode) -> WireMode {
    match mode {
        AmbientMode::Off => WireMode::Off,
        AmbientMode::Ambient => WireMode::Ambient,
        AmbientMode::NoiseCancelling => WireMode::NoiseCancelling,
        AmbientMode::WindReduction => WireMode::WindReduction,
    }
}

/// The ambient sound the person asked for, as the headphones take it.
pub(crate) fn ambient_setting(ambient: AmbientSound) -> AmbientSetting {
    AmbientSetting {
        mode: wire_mode(ambient.mode),
        level: ambient.level,
        focus_on_voice: ambient.focus_on_voice,
    }
}

fn preset(preset: WirePreset) -> EqualizerPreset {
    match preset {
        WirePreset::Off => EqualizerPreset::Off,
        WirePreset::Bright => EqualizerPreset::Bright,
        WirePreset::Excited => EqualizerPreset::Excited,
        WirePreset::Mellow => EqualizerPreset::Mellow,
        WirePreset::Relaxed => EqualizerPreset::Relaxed,
        WirePreset::Vocal => EqualizerPreset::Vocal,
        WirePreset::TrebleBoost => EqualizerPreset::TrebleBoost,
        WirePreset::BassBoost => EqualizerPreset::BassBoost,
        WirePreset::Speech => EqualizerPreset::Speech,
        WirePreset::Manual => EqualizerPreset::Manual,
        WirePreset::Custom1 => EqualizerPreset::Custom1,
        WirePreset::Custom2 => EqualizerPreset::Custom2,
        WirePreset::Unknown(_) => EqualizerPreset::Other,
    }
}

fn wire_preset(preset: EqualizerPreset) -> Option<WirePreset> {
    Some(match preset {
        EqualizerPreset::Off => WirePreset::Off,
        EqualizerPreset::Bright => WirePreset::Bright,
        EqualizerPreset::Excited => WirePreset::Excited,
        EqualizerPreset::Mellow => WirePreset::Mellow,
        EqualizerPreset::Relaxed => WirePreset::Relaxed,
        EqualizerPreset::Vocal => WirePreset::Vocal,
        EqualizerPreset::TrebleBoost => WirePreset::TrebleBoost,
        EqualizerPreset::BassBoost => WirePreset::BassBoost,
        EqualizerPreset::Speech => WirePreset::Speech,
        EqualizerPreset::Manual => WirePreset::Manual,
        EqualizerPreset::Custom1 => WirePreset::Custom1,
        EqualizerPreset::Custom2 => WirePreset::Custom2,
        EqualizerPreset::Other => return None,
    })
}

/// The equalizer change the person asked for, as the headphones take it;
/// `None` for a preset the headphones cannot be asked for.
pub(crate) fn equalizer_setting(change: EqualizerChange) -> Option<EqualizerSetting> {
    Some(match change {
        EqualizerChange::Preset { preset } => EqualizerSetting::Preset(wire_preset(preset)?),
        EqualizerChange::Manual { clear_bass, bands } => {
            EqualizerSetting::Manual { clear_bass, bands }
        }
    })
}

/// What an answer or a notification of the headphones changes of the state.
pub(crate) fn apply_event(state: &mut HeadsetState, event: &Event) {
    match event {
        Event::Ambient(AmbientState {
            mode,
            wind_capable,
            focus_on_voice,
            level,
        }) => {
            state.ambient = Some(AmbientSound {
                mode: ambient_mode(*mode),
                level: *level,
                focus_on_voice: *focus_on_voice,
            });
            state.wind_reduction = *wind_capable;
        }
        Event::Battery(battery) => state.battery = Some(headset_battery(*battery)),
        Event::Equalizer(EqualizerState {
            preset: wire,
            clear_bass,
            bands,
        }) => {
            state.equalizer = Some(Equalizer {
                preset: preset(*wire),
                clear_bass: *clear_bass,
                bands: *bands,
            });
        }
        Event::Other(_) => {}
    }
}

/// Headphones with two batteries show the weaker, charging if either is.
fn headset_battery(battery: Battery) -> HeadsetBattery {
    match battery {
        Battery::Single(cell) => HeadsetBattery {
            percent: cell.level,
            charging: cell.charging,
        },
        Battery::LeftRight { left, right } => HeadsetBattery {
            percent: left.level.min(right.level),
            charging: left.charging || right.charging,
        },
    }
}

#[cfg(test)]
mod tests {
    use hubergamof_headset::payload::BatteryCell;
    use hubergamof_protocol::HeadsetLink;

    use super::*;

    fn cell(level: u8, charging: bool) -> BatteryCell {
        BatteryCell { level, charging }
    }

    #[test]
    fn every_ambient_mode_goes_both_ways() {
        for mode in [
            AmbientMode::Off,
            AmbientMode::Ambient,
            AmbientMode::NoiseCancelling,
            AmbientMode::WindReduction,
        ] {
            assert_eq!(ambient_mode(wire_mode(mode)), mode);
        }
    }

    #[test]
    fn an_ambient_request_keeps_its_values() {
        let setting = ambient_setting(AmbientSound {
            mode: AmbientMode::Ambient,
            level: 7,
            focus_on_voice: true,
        });
        assert_eq!(setting.mode, WireMode::Ambient);
        assert_eq!((setting.level, setting.focus_on_voice), (7, true));
    }

    #[test]
    fn presets_go_both_ways_and_an_unknown_one_is_never_asked_for() {
        for chosen in EqualizerPreset::CHOOSABLE {
            let wire = wire_preset(chosen).expect("choosable");
            assert_eq!(preset(wire), chosen);
        }
        assert_eq!(wire_preset(EqualizerPreset::Other), None);
        assert_eq!(preset(WirePreset::Unknown(0x42)), EqualizerPreset::Other);
        assert_eq!(
            equalizer_setting(EqualizerChange::Preset {
                preset: EqualizerPreset::Other
            }),
            None
        );
    }

    #[test]
    fn manual_bands_are_passed_as_they_are() {
        assert_eq!(
            equalizer_setting(EqualizerChange::Manual {
                clear_bass: -3,
                bands: [1, 2, 3, 4, 5],
            }),
            Some(EqualizerSetting::Manual {
                clear_bass: -3,
                bands: [1, 2, 3, 4, 5],
            })
        );
    }

    #[test]
    fn two_batteries_show_the_weaker_and_charge_if_either_does() {
        let both = headset_battery(Battery::LeftRight {
            left: cell(80, false),
            right: cell(35, true),
        });
        assert_eq!((both.percent, both.charging), (35, true));
        let single = headset_battery(Battery::Single(cell(60, false)));
        assert_eq!((single.percent, single.charging), (60, false));
    }

    #[test]
    fn what_the_headphones_say_changes_the_matching_part_of_the_state() {
        let mut state = HeadsetState::unknown(HeadsetLink::Connected);
        apply_event(
            &mut state,
            &Event::Ambient(AmbientState {
                mode: WireMode::NoiseCancelling,
                wind_capable: true,
                focus_on_voice: true,
                level: 9,
            }),
        );
        apply_event(&mut state, &Event::Battery(Battery::Single(cell(42, true))));
        apply_event(
            &mut state,
            &Event::Equalizer(EqualizerState {
                preset: WirePreset::Speech,
                clear_bass: -10,
                bands: [4, 3, 1, 2, -10],
            }),
        );
        assert_eq!(
            state.ambient,
            Some(AmbientSound {
                mode: AmbientMode::NoiseCancelling,
                level: 9,
                focus_on_voice: true,
            })
        );
        assert!(state.wind_reduction);
        assert_eq!(state.battery.map(|b| b.percent), Some(42));
        assert_eq!(
            state.equalizer.map(|e| e.preset),
            Some(EqualizerPreset::Speech)
        );
    }

    #[test]
    fn an_answer_this_project_does_not_read_changes_nothing() {
        let mut state = HeadsetState::unknown(HeadsetLink::Connected);
        let before = state;
        apply_event(&mut state, &Event::Other(vec![0x25, 0x01]));
        assert_eq!(state, before);
    }
}
