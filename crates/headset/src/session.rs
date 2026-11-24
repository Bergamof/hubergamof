//! Headphones in use: the states asked for when the link opens, the changes
//! sent, and what the headphones say decoded as it arrives.

use std::time::Duration;

use thiserror::Error;

use crate::channel::Channel;
use crate::frame::DATA_TYPE_MDR;
use crate::link::{Link, LinkError};
use crate::payload::{
    AmbientSetting, AmbientState, EqualizerError, EqualizerSetting, Event, Query, SettingError,
    decode,
};

/// A change the person asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Change the ambient sound.
    SetAmbient(AmbientSetting),
    /// Change the equalizer.
    SetEqualizer(EqualizerSetting),
}

/// Why a command could not be sent.
#[derive(Debug, Error)]
pub enum SessionError {
    /// A frame could not be exchanged.
    #[error(transparent)]
    Link(#[from] LinkError),
    /// The ambient setting is not one the headphones accept.
    #[error(transparent)]
    Setting(#[from] SettingError),
    /// The equalizer change is not one the headphones accept.
    #[error(transparent)]
    Equalizer(#[from] EqualizerError),
    /// The headphones have not said yet whether they offer wind noise
    /// reduction, which the ambient command depends on.
    #[error("the ambient sound state of the headphones is not known yet")]
    AmbientUnknown,
}

/// What the session asks for when the link opens: the headphones announce a
/// change only after the matching query.
const OPENING_QUERIES: [Query; 4] = [
    Query::ProtocolInfo,
    Query::Ambient,
    Query::Battery,
    Query::Equalizer,
];

/// The settings channel of one pair of headphones.
pub struct Session<C> {
    link: Link<C>,
    ambient: Option<AmbientState>,
}

impl<C: Channel> Session<C> {
    /// A session on a connected stream.
    pub fn new(channel: C) -> Self {
        Self {
            link: Link::new(channel),
            ambient: None,
        }
    }

    /// Asks for everything the interface shows; the answers come through
    /// [`Session::poll`].
    ///
    /// # Errors
    /// [`SessionError`] when the headphones do not acknowledge or the stream fails.
    pub fn start(&mut self) -> Result<(), SessionError> {
        for query in OPENING_QUERIES {
            self.link.send(DATA_TYPE_MDR, &query.payload())?;
        }
        Ok(())
    }

    /// Sends a change; the headphones confirm it with a notification that
    /// [`Session::poll`] returns.
    ///
    /// # Errors
    /// [`SessionError`] for a change the headphones would refuse (nothing is
    /// sent then), or when the stream fails.
    pub fn apply(&mut self, command: &Command) -> Result<(), SessionError> {
        let payload = match command {
            Command::SetAmbient(setting) => {
                let ambient = self.ambient.ok_or(SessionError::AmbientUnknown)?;
                setting.payload(ambient.wind_capable)?
            }
            Command::SetEqualizer(setting) => setting.payload()?,
        };
        self.link.send(DATA_TYPE_MDR, &payload)?;
        Ok(())
    }

    /// What the headphones said within `timeout`: nothing, or one frame read.
    /// A payload this project cannot read is logged and skipped.
    ///
    /// # Errors
    /// [`SessionError`] when the stream fails or the headphones closed it.
    pub fn poll(&mut self, timeout: Duration) -> Result<Option<Event>, SessionError> {
        let Some(frame) = self.link.receive(timeout)? else {
            return Ok(None);
        };
        match decode(&frame.payload) {
            Ok(event) => {
                if let Event::Ambient(state) = &event {
                    self.ambient = Some(*state);
                }
                Ok(Some(event))
            }
            Err(error) => {
                tracing::debug!(%error, "unreadable answer from the headphones, skipped");
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{AmbientMode, EqualizerPreset};
    use crate::testing::FakeHeadset;

    const AMBIENT_REPLY: [u8; 8] = [0x67, 0x02, 0x01, 0x02, 0x00, 0x01, 0x00, 0x05];
    const SHORT: Duration = Duration::from_millis(5);

    fn session(headset: &FakeHeadset) -> Session<FakeHeadset> {
        let mut session = Session::new(headset.clone());
        session.link = Link::new(headset.clone()).with_ack_timeout(SHORT);
        session
    }

    fn talkative() -> FakeHeadset {
        FakeHeadset::new()
            .answering(&Query::Ambient.payload(), &[&AMBIENT_REPLY])
            .answering(&Query::Battery.payload(), &[&[0x11, 0x00, 0x50, 0x00]])
    }

    fn noise_cancelling() -> Command {
        Command::SetAmbient(AmbientSetting {
            mode: AmbientMode::NoiseCancelling,
            level: 0,
            focus_on_voice: false,
        })
    }

    #[test]
    fn opening_asks_for_everything_the_interface_shows() {
        let headset = talkative();
        session(&headset).start().unwrap();
        assert_eq!(
            headset.received(),
            OPENING_QUERIES.map(Query::payload).to_vec()
        );
    }

    #[test]
    fn the_answers_are_decoded_as_they_arrive() {
        let headset = talkative();
        let mut session = session(&headset);
        session.start().unwrap();
        let mut events = Vec::new();
        while let Some(event) = session.poll(SHORT).unwrap() {
            events.push(event);
        }
        assert_eq!(events.len(), 2, "ambient and battery answered");
        assert!(matches!(events[0], Event::Ambient(_)));
        assert!(matches!(events[1], Event::Battery(_)));
    }

    #[test]
    fn an_ambient_change_needs_the_state_the_headphones_reported() {
        let headset = FakeHeadset::new();
        let mut session = session(&headset);
        assert!(matches!(
            session.apply(&noise_cancelling()),
            Err(SessionError::AmbientUnknown)
        ));
        assert_eq!(headset.received(), Vec::<Vec<u8>>::new());
    }

    #[test]
    fn an_ambient_change_uses_the_capability_the_headphones_reported() {
        let headset = talkative();
        let mut session = session(&headset);
        session.start().unwrap();
        while session.poll(SHORT).unwrap().is_some() {}
        session.apply(&noise_cancelling()).unwrap();
        assert_eq!(
            headset.received().last().unwrap(),
            &[0x68, 0x02, 0x11, 0x02, 0x02, 0x01, 0x00, 0x00]
        );
    }

    #[test]
    fn a_wind_change_is_refused_without_the_capability() {
        let headset = FakeHeadset::new().answering(
            &Query::Ambient.payload(),
            &[&[0x67, 0x02, 0x01, 0x00, 0x00, 0x01, 0x00, 0x05]],
        );
        let mut session = session(&headset);
        session.start().unwrap();
        while session.poll(SHORT).unwrap().is_some() {}
        let wind = Command::SetAmbient(AmbientSetting {
            mode: AmbientMode::WindReduction,
            level: 0,
            focus_on_voice: false,
        });
        assert!(matches!(
            session.apply(&wind),
            Err(SessionError::Setting(SettingError::NoWindReduction))
        ));
        assert_eq!(headset.received().len(), OPENING_QUERIES.len());
    }

    #[test]
    fn an_equalizer_change_is_sent_as_it_is() {
        let headset = FakeHeadset::new();
        let mut session = session(&headset);
        session
            .apply(&Command::SetEqualizer(EqualizerSetting::Preset(
                EqualizerPreset::BassBoost,
            )))
            .unwrap();
        assert_eq!(headset.received(), [vec![0x58, 0x01, 0x16, 0x00]]);
    }

    #[test]
    fn an_unreadable_answer_is_skipped_not_fatal() {
        let headset = FakeHeadset::new();
        headset.notify(&[0x67, 0x02]);
        headset.notify(&[0x11, 0x00, 0x50, 0x00]);
        let mut session = session(&headset);
        assert_eq!(session.poll(SHORT).unwrap(), None);
        assert!(matches!(
            session.poll(SHORT).unwrap(),
            Some(Event::Battery(_))
        ));
    }

    #[test]
    fn a_stream_the_headphones_closed_is_an_error() {
        let headset = FakeHeadset::new();
        headset.close();
        let mut session = session(&headset);
        assert!(matches!(
            session.poll(SHORT),
            Err(SessionError::Link(LinkError::Channel(_)))
        ));
    }
}
