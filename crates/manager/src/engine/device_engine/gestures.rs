//! Keys and dials held down: what releasing or holding them does.

use super::{
    ContextGesture, DeviceEngine, DialAssignment, DialInput, HeldDial, HeldDialKind, HeldKey,
    Instant,
};

impl DeviceEngine {
    /// Starts holding `key`: its action waits for the release, and is
    /// dropped if the key is held until its label shows.
    pub(super) fn hold_key(&mut self, key: u8) {
        let Some(frame) = &self.last_frame else {
            return;
        };
        let Some(target) = frame.target(key).cloned() else {
            return;
        };
        let usable_when_locked = frame.usable_when_locked(key);
        let label = frame
            .keys
            .get(usize::from(key))
            .and_then(|key| key.face.label.as_deref())
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .map(str::to_owned);
        self.held = Some(HeldKey {
            key,
            target,
            usable_when_locked,
            label,
            since: Instant::now(),
            long: false,
        });
    }

    /// Releases `key`: runs its action unless it was held long enough for
    /// the label to show.
    pub(super) async fn release_key(&mut self, key: u8) {
        let Some(held) = self.held.take_if(|held| held.key == key) else {
            return;
        };
        if held.is_hold(Instant::now()) {
            return;
        }
        self.press_target(held.target, held.usable_when_locked)
            .await;
    }

    /// A press of a dial of the device. The navigation dial always starts
    /// holding; the contextual dial does when the page displayed declares a
    /// long press. Their press then waits for the release; otherwise the
    /// press acts at once.
    pub(super) async fn dial_down(&mut self, dial: u8) {
        if let Some(held) = self.start_holding_dial(dial) {
            self.held_dial = Some(held);
            return;
        }
        self.dial_input(dial, DialInput::Press).await;
    }

    pub(super) fn start_holding_dial(&self, dial: u8) -> Option<HeldDial> {
        if self.notifications.shown().is_some() {
            return None;
        }
        let kind = match self.dial_assignment(dial) {
            DialAssignment::Navigation => HeldDialKind::Navigation,
            DialAssignment::Contextual => {
                let context = self.last_frame.as_ref()?.context.clone()?;
                if !context.dial.reacts_to_long_press() {
                    return None;
                }
                HeldDialKind::Context(context)
            }
            _ => return None,
        };
        Some(HeldDial {
            dial,
            kind,
            since: Instant::now(),
        })
    }

    /// The release of `dial`: a press, if it was held for less than a long
    /// press.
    pub(super) async fn release_dial(&mut self, dial: u8) {
        let Some(held) = self.held_dial.take_if(|held| held.dial == dial) else {
            return;
        };
        if self.notifications.shown().is_some() {
            return;
        }
        match &held.kind {
            HeldDialKind::Context(context) => {
                self.run_context(dial, context, ContextGesture::Press).await;
            }
            HeldDialKind::Navigation => self.navigate(DialInput::Press),
        }
    }

    /// The long press of a dial held until it is due.
    pub(super) async fn long_press_dial(&mut self) {
        let Some(held) = self.held_dial.take() else {
            return;
        };
        if self.notifications.shown().is_some() {
            return;
        }
        match &held.kind {
            HeldDialKind::Context(context) => {
                self.run_context(held.dial, context, ContextGesture::LongPress)
                    .await;
            }
            HeldDialKind::Navigation => self.nav.home(),
        }
    }
}
