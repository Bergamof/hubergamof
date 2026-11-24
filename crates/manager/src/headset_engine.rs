//! Sony headphones: the supervisor that lists the headphones connected to
//! this computer, and the thread that serves the link to one of them.
//!
//! The headphones serve one settings client at a time (the Sony app on a
//! phone loses the channel when this one opens), so a link is opened only on
//! request (the person selected the headphones, or resumed the link) and
//! never opened again by itself: one that the headphones closed stays cut
//! until the person resumes it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use hubergamof_headset::channel::Channel;
use hubergamof_headset::payload::{AmbientSetting, EqualizerSetting};
use hubergamof_headset::session::{Command, Session, SessionError};
use hubergamof_protocol::HeadsetLink;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::headset_convert::{ambient_setting, apply_event, equalizer_setting};
use crate::headset_hub::{Action, HeadsetHub, Request};
use crate::headset_source::HeadsetSource;

/// Delay between two looks at the headphones connected to this computer.
pub const SCAN_INTERVAL: Duration = Duration::from_secs(5);

/// How long a serving thread waits for the headphones before looking at its
/// orders again: the longest an order waits to be carried out.
const POLL: Duration = Duration::from_millis(100);

/// Why a serving thread ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The person closed the link (or the supervisor let it go).
    Released,
    /// The headphones closed it, or it failed.
    Cut,
}

impl Outcome {
    fn link(self) -> HeadsetLink {
        match self {
            Self::Released => HeadsetLink::Released,
            Self::Cut => HeadsetLink::Cut,
        }
    }
}

/// What a serving thread is asked, as the latest of each kind: a flood of
/// changes (a slider dragged) is not worth carrying out one by one, and holds
/// no memory.
#[derive(Default)]
struct Pending {
    close: bool,
    ambient: Option<AmbientSetting>,
    equalizer: Option<EqualizerSetting>,
}

impl Pending {
    fn commands(&self) -> impl Iterator<Item = Command> {
        self.ambient
            .map(Command::SetAmbient)
            .into_iter()
            .chain(self.equalizer.map(Command::SetEqualizer))
    }
}

/// The orders of the engine to a serving thread, and the word that ends it.
#[derive(Default)]
struct Orders {
    pending: Mutex<Pending>,
    /// Raised when nobody wants the link any more (the slot is dropped): the
    /// thread ends without telling anything.
    cancelled: AtomicBool,
}

impl Orders {
    fn lock(&self) -> MutexGuard<'_, Pending> {
        // Plain data: nothing to recover from a poisoned lock.
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn ask(&self, change: impl FnOnce(&mut Pending)) {
        change(&mut self.lock());
    }

    fn take(&self) -> Pending {
        std::mem::take(&mut *self.lock())
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// A link being opened or served.
struct Slot {
    id: u64,
    orders: Arc<Orders>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        // Not an abort of the task: one that is opening the channel finishes,
        // so that the profile it registered with BlueZ is given back.
        self.orders.cancel();
    }
}

/// A serving task that ended.
struct Ended {
    address: String,
    id: u64,
    outcome: Outcome,
}

/// Looks for the headphones connected to this computer and carries out the
/// requests made of them.
pub struct HeadsetSupervisor {
    task: JoinHandle<()>,
}

impl HeadsetSupervisor {
    /// Starts supervising: `source` is looked at every `scan_interval`.
    #[must_use]
    pub fn start(
        source: Arc<dyn HeadsetSource>,
        hub: HeadsetHub,
        requests: mpsc::Receiver<Request>,
        scan_interval: Duration,
    ) -> Self {
        let (ended_sender, ended) = mpsc::unbounded_channel();
        let engine = Engine {
            source,
            hub,
            slots: HashMap::new(),
            next_id: 0,
            ended_sender,
        };
        Self {
            task: tokio::spawn(engine.run(requests, ended, scan_interval)),
        }
    }

    /// Stops supervising and closes every link.
    pub async fn shutdown(self) {
        // The engine dropped with its task lets go of every slot, which ends
        // the serving threads within a poll.
        self.task.abort();
        let _ = self.task.await;
    }
}

struct Engine {
    source: Arc<dyn HeadsetSource>,
    hub: HeadsetHub,
    slots: HashMap<String, Slot>,
    next_id: u64,
    ended_sender: mpsc::UnboundedSender<Ended>,
}

impl Engine {
    async fn run(
        mut self,
        mut requests: mpsc::Receiver<Request>,
        mut ended: mpsc::UnboundedReceiver<Ended>,
        scan_interval: Duration,
    ) {
        let mut scan = tokio::time::interval(scan_interval);
        scan.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = scan.tick() => self.scan().await,
                Some(request) = requests.recv() => self.handle(request),
                Some(end) = ended.recv() => self.finish(&end),
                else => return,
            }
        }
    }

    /// Lists the headphones connected now; those that left lose their link.
    async fn scan(&mut self) {
        let present = self.source.present().await;
        self.slots
            .retain(|address, _| present.iter().any(|info| &info.address == address));
        self.hub.present(present);
    }

    fn handle(&mut self, request: Request) {
        let Request { address, action } = request;
        let link = self.hub.state(&address).map(|state| state.link);
        match action {
            Action::Open if link == Some(HeadsetLink::Idle) => self.open(address),
            Action::Resume
                if matches!(
                    link,
                    Some(HeadsetLink::Idle | HeadsetLink::Released | HeadsetLink::Cut)
                ) =>
            {
                self.open(address);
            }
            // The link is already open, being opened, or was not asked for.
            Action::Open | Action::Resume => {}
            Action::Release => self.ask(&address, |pending| pending.close = true),
            Action::SetAmbient(ambient) => {
                let setting = ambient_setting(ambient);
                self.ask(&address, |pending| pending.ambient = Some(setting));
            }
            Action::SetEqualizer(change) => {
                if let Some(setting) = equalizer_setting(change) {
                    self.ask(&address, |pending| pending.equalizer = Some(setting));
                } else {
                    tracing::debug!("an equalizer change the headphones cannot take, dropped");
                }
            }
        }
    }

    fn ask(&self, address: &str, change: impl FnOnce(&mut Pending)) {
        if let Some(slot) = self.slots.get(address) {
            slot.orders.ask(change);
        }
    }

    fn open(&mut self, address: String) {
        if self.slots.contains_key(&address) {
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        let orders = Arc::new(Orders::default());
        tokio::spawn(serve(
            Arc::clone(&self.source),
            self.hub.clone(),
            address.clone(),
            id,
            Arc::clone(&orders),
            self.ended_sender.clone(),
        ));
        self.slots.insert(address, Slot { id, orders });
    }

    fn finish(&mut self, end: &Ended) {
        // A link opened again meanwhile is not the one that ended.
        if self
            .slots
            .get(&end.address)
            .is_some_and(|slot| slot.id == end.id)
        {
            self.slots.remove(&end.address);
            self.hub
                .update_state(&end.address, |state| state.link = end.outcome.link());
        }
    }
}

/// Opens the channel, serves it until it ends and reports how.
async fn serve(
    source: Arc<dyn HeadsetSource>,
    hub: HeadsetHub,
    address: String,
    id: u64,
    orders: Arc<Orders>,
    ended: mpsc::UnboundedSender<Ended>,
) {
    // Checked before each write: headphones that left and came back are not
    // written to by the link of their previous visit.
    if !orders.cancelled() {
        hub.update_state(&address, |state| state.link = HeadsetLink::Connecting);
    }
    let outcome = match source.open(&address).await {
        Ok(_) if orders.cancelled() => Outcome::Released,
        Ok(channel) => {
            let (hub, address, orders) = (hub.clone(), address.clone(), Arc::clone(&orders));
            tokio::task::spawn_blocking(move || run_session(channel, &hub, &address, &orders))
                .await
                .unwrap_or(Outcome::Cut)
        }
        Err(error) => {
            tracing::warn!(%error, "cannot open the link to the headphones");
            Outcome::Cut
        }
    };
    // The supervisor is gone at shutdown: nothing more to do then.
    let _ = ended.send(Ended {
        address,
        id,
        outcome,
    });
}

/// Serves an open channel: asks for what the interface shows, then follows
/// the headphones and carries out the orders, until one of them ends it.
fn run_session(
    channel: Box<dyn Channel + Send>,
    hub: &HeadsetHub,
    address: &str,
    orders: &Orders,
) -> Outcome {
    let mut session = Session::new(channel);
    if let Err(error) = session.start() {
        tracing::warn!(%error, "the headphones did not answer");
        return Outcome::Cut;
    }
    if !orders.cancelled() {
        hub.update_state(address, |state| state.link = HeadsetLink::Connected);
    }
    loop {
        if orders.cancelled() {
            return Outcome::Released;
        }
        let pending = orders.take();
        if pending.close {
            return Outcome::Released;
        }
        for command in pending.commands() {
            match session.apply(&command) {
                Ok(()) => {}
                Err(SessionError::Link(error)) => {
                    tracing::debug!(%error, "the link to the headphones failed");
                    return Outcome::Cut;
                }
                // A change the headphones would refuse: the link stays.
                Err(error) => tracing::debug!(%error, "change not sent"),
            }
        }
        match session.poll(POLL) {
            Ok(Some(event)) if !orders.cancelled() => {
                hub.update_state(address, |state| apply_event(state, &event));
            }
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(%error, "the headphones closed the link");
                return Outcome::Cut;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    use async_trait::async_trait;
    use hubergamof_headset::payload::AmbientMode as WireMode;
    use hubergamof_headset::simulated::SimulatedHeadset;
    use hubergamof_protocol::{
        AmbientMode, AmbientSound, EqualizerChange, EqualizerPreset, HeadsetInfo, HeadsetState,
    };
    use tokio::sync::{Notify, watch};

    use super::*;
    use crate::headset_source::SimulatedSource;

    const ADDRESS: &str = "AC:80:0A:0B:2E:61";
    const SCAN: Duration = Duration::from_millis(20);
    const PATIENCE: Duration = Duration::from_secs(5);

    fn info() -> HeadsetInfo {
        HeadsetInfo {
            address: ADDRESS.into(),
            name: "WH-1000XM4".into(),
        }
    }

    /// What the source of the tests is asked, and what it is told to do.
    #[derive(Default)]
    struct Probe {
        scans: AtomicUsize,
        opens: AtomicUsize,
        /// Opening waits for [`Probe::gate`].
        gated: AtomicBool,
        gate: Notify,
        /// Opening is refused.
        refusing: AtomicBool,
    }

    struct ProbedSource {
        source: Arc<SimulatedSource>,
        probe: Arc<Probe>,
    }

    #[async_trait]
    impl HeadsetSource for ProbedSource {
        async fn present(&self) -> Vec<HeadsetInfo> {
            self.probe.scans.fetch_add(1, Ordering::Relaxed);
            self.source.present().await
        }

        async fn open(&self, address: &str) -> Result<Box<dyn Channel + Send>, String> {
            self.probe.opens.fetch_add(1, Ordering::Relaxed);
            if self.probe.gated.load(Ordering::Relaxed) {
                self.probe.gate.notified().await;
            }
            if self.probe.refusing.load(Ordering::Relaxed) {
                return Err("refused".into());
            }
            self.source.open(address).await
        }
    }

    struct Rig {
        hub: HeadsetHub,
        source: Arc<SimulatedSource>,
        probe: Arc<Probe>,
        headset: SimulatedHeadset,
        supervisor: HeadsetSupervisor,
    }

    impl Rig {
        async fn listed() -> Self {
            Self::with(SimulatedHeadset::new()).await
        }

        async fn with(headset: SimulatedHeadset) -> Self {
            let source = Arc::new(SimulatedSource::new(vec![(info(), headset.clone())]));
            let probe = Arc::new(Probe::default());
            let probed = ProbedSource {
                source: Arc::clone(&source),
                probe: Arc::clone(&probe),
            };
            let (hub, requests) = HeadsetHub::new();
            let supervisor =
                HeadsetSupervisor::start(Arc::new(probed), hub.clone(), requests, SCAN);
            let rig = Self {
                hub,
                source,
                probe,
                headset,
                supervisor,
            };
            tokio::time::timeout(
                PATIENCE,
                rig.hub
                    .subscribe_headsets()
                    .wait_for(|list| !list.is_empty()),
            )
            .await
            .expect("the headphones are listed")
            .unwrap();
            rig
        }

        fn state(&self) -> watch::Receiver<HeadsetState> {
            self.hub.subscribe_state(ADDRESS).expect("listed")
        }

        async fn until(&self, what: &str, wanted: impl Fn(&HeadsetState) -> bool) -> HeadsetState {
            let mut state = self.state();
            let reached = tokio::time::timeout(PATIENCE, state.wait_for(|s| wanted(s)))
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for: {what}"))
                .unwrap();
            *reached
        }

        async fn connected(&self) -> HeadsetState {
            self.hub.select(ADDRESS).unwrap();
            self.until("everything is known", |state| {
                state.link == HeadsetLink::Connected
                    && state.ambient.is_some()
                    && state.battery.is_some()
                    && state.equalizer.is_some()
            })
            .await
        }

        /// Waits for `count` more looks at the headphones: proof that the
        /// engine went on, which a fixed pause is not.
        async fn scans_pass(&self, count: usize) {
            let target = self.probe.scans.load(Ordering::Relaxed) + count;
            tokio::time::timeout(PATIENCE, async {
                while self.probe.scans.load(Ordering::Relaxed) < target {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            })
            .await
            .expect("the scans go on");
        }

        fn opens(&self) -> usize {
            self.probe.opens.load(Ordering::Relaxed)
        }

        fn link(&self) -> HeadsetLink {
            self.hub.state(ADDRESS).expect("listed").link
        }
    }

    fn noise_cancelling() -> AmbientSound {
        AmbientSound {
            mode: AmbientMode::NoiseCancelling,
            level: 0,
            focus_on_voice: true,
        }
    }

    #[tokio::test]
    async fn headphones_are_listed_but_their_link_is_not_opened_by_itself() {
        let rig = Rig::listed().await;
        rig.scans_pass(8).await;
        assert_eq!(rig.link(), HeadsetLink::Idle);
        assert_eq!(rig.opens(), 0);
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn selecting_the_headphones_opens_the_link_and_tells_everything() {
        let rig = Rig::listed().await;
        let state = rig.connected().await;
        assert_eq!(state.ambient.unwrap().mode, AmbientMode::Ambient);
        assert!(state.wind_reduction);
        assert_eq!(state.battery.unwrap().percent, 80);
        assert_eq!(state.equalizer.unwrap().preset, EqualizerPreset::Speech);
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn the_link_works_over_a_stream_that_cuts_and_joins_frames() {
        for headset in [
            SimulatedHeadset::new().fragmenting(1),
            SimulatedHeadset::new().fragmenting(3),
            SimulatedHeadset::new().joining_reads(),
        ] {
            let rig = Rig::with(headset).await;
            rig.connected().await;
            rig.hub.set_ambient(ADDRESS, noise_cancelling()).unwrap();
            rig.until("noise cancelling", |state| {
                state
                    .ambient
                    .is_some_and(|a| a.mode == AmbientMode::NoiseCancelling)
            })
            .await;
            rig.supervisor.shutdown().await;
        }
    }

    #[tokio::test]
    async fn an_ambient_change_reaches_the_headphones_and_comes_back_confirmed() {
        let rig = Rig::listed().await;
        rig.connected().await;
        rig.hub.set_ambient(ADDRESS, noise_cancelling()).unwrap();
        let state = rig
            .until("noise cancelling", |state| {
                state
                    .ambient
                    .is_some_and(|a| a.mode == AmbientMode::NoiseCancelling)
            })
            .await;
        assert!(state.ambient.unwrap().focus_on_voice);
        assert_eq!(rig.headset.mode(), WireMode::NoiseCancelling);
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn a_burst_of_changes_ends_on_the_last_one() {
        let rig = Rig::listed().await;
        rig.connected().await;
        let ambient = |mode, level| AmbientSound {
            mode,
            level,
            focus_on_voice: false,
        };
        for level in 0..=20 {
            rig.hub
                .set_ambient(ADDRESS, ambient(AmbientMode::Ambient, level))
                .unwrap();
        }
        rig.hub
            .set_ambient(ADDRESS, ambient(AmbientMode::NoiseCancelling, 0))
            .unwrap();
        rig.until("the last change", |state| {
            state
                .ambient
                .is_some_and(|a| a.mode == AmbientMode::NoiseCancelling)
        })
        .await;
        assert_eq!(rig.headset.mode(), WireMode::NoiseCancelling);
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn an_equalizer_change_comes_back_confirmed() {
        let rig = Rig::listed().await;
        rig.connected().await;
        rig.hub
            .set_equalizer(
                ADDRESS,
                EqualizerChange::Manual {
                    clear_bass: -2,
                    bands: [1, 2, 3, 4, 5],
                },
            )
            .unwrap();
        let state = rig
            .until("manual bands", |state| {
                state
                    .equalizer
                    .is_some_and(|e| e.preset == EqualizerPreset::Manual)
            })
            .await;
        let equalizer = state.equalizer.unwrap();
        assert_eq!(
            (equalizer.clear_bass, equalizer.bands),
            (-2, [1, 2, 3, 4, 5])
        );
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn a_button_pressed_on_the_headphones_is_followed() {
        let rig = Rig::listed().await;
        rig.connected().await;
        rig.headset.press_button(WireMode::Off);
        rig.until("off", |state| {
            state.ambient.is_some_and(|a| a.mode == AmbientMode::Off)
        })
        .await;
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn releasing_closes_the_link_and_it_stays_closed_until_resumed() {
        let rig = Rig::listed().await;
        rig.connected().await;
        rig.hub.release(ADDRESS).unwrap();
        rig.until("released", |state| state.link == HeadsetLink::Released)
            .await;

        // Selecting again does not take the channel back: only resuming does.
        rig.hub.select(ADDRESS).unwrap();
        rig.scans_pass(8).await;
        assert_eq!(rig.link(), HeadsetLink::Released);
        assert_eq!(rig.opens(), 1);

        rig.hub.resume(ADDRESS).unwrap();
        rig.until("connected again", |state| {
            state.link == HeadsetLink::Connected
        })
        .await;
        assert_eq!(rig.opens(), 2);
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn a_link_the_headphones_cut_is_not_opened_again_until_resumed() {
        let rig = Rig::listed().await;
        rig.connected().await;
        rig.headset.cut();
        rig.until("cut", |state| state.link == HeadsetLink::Cut)
            .await;

        rig.scans_pass(8).await;
        rig.hub.select(ADDRESS).unwrap();
        rig.scans_pass(8).await;
        assert_eq!(rig.link(), HeadsetLink::Cut);
        assert_eq!(rig.opens(), 1);

        rig.hub.resume(ADDRESS).unwrap();
        rig.until("connected again", |state| {
            state.link == HeadsetLink::Connected
        })
        .await;
        assert_eq!(rig.opens(), 2);
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn what_was_known_stays_shown_when_the_link_is_cut() {
        let rig = Rig::listed().await;
        rig.connected().await;
        rig.headset.cut();
        let state = rig
            .until("cut", |state| state.link == HeadsetLink::Cut)
            .await;
        assert!(state.ambient.is_some() && state.battery.is_some());
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn releasing_while_the_link_is_being_opened_ends_released() {
        let rig = Rig::listed().await;
        rig.probe.gated.store(true, Ordering::Relaxed);
        rig.hub.select(ADDRESS).unwrap();
        rig.until("connecting", |state| state.link == HeadsetLink::Connecting)
            .await;

        rig.hub.release(ADDRESS).unwrap();
        rig.probe.gate.notify_one();
        rig.until("released", |state| state.link == HeadsetLink::Released)
            .await;
        assert_eq!(rig.opens(), 1);
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn the_link_is_opened_once_however_often_it_is_asked_for() {
        let rig = Rig::listed().await;
        rig.probe.gated.store(true, Ordering::Relaxed);
        rig.hub.select(ADDRESS).unwrap();
        rig.until("connecting", |state| state.link == HeadsetLink::Connecting)
            .await;
        // While it is being opened...
        rig.hub.select(ADDRESS).unwrap();
        rig.hub.resume(ADDRESS).unwrap();
        rig.probe.gate.notify_one();
        rig.until("connected", |state| state.link == HeadsetLink::Connected)
            .await;
        // ...and once it is open.
        rig.hub.resume(ADDRESS).unwrap();
        rig.hub.select(ADDRESS).unwrap();
        rig.scans_pass(4).await;
        assert_eq!(rig.link(), HeadsetLink::Connected);
        assert_eq!(rig.opens(), 1);
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn releasing_a_link_that_is_not_open_changes_nothing() {
        let rig = Rig::listed().await;
        rig.hub.release(ADDRESS).unwrap();
        rig.scans_pass(4).await;
        assert_eq!(rig.link(), HeadsetLink::Idle);
        assert_eq!(rig.opens(), 0);
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn a_link_that_cannot_be_opened_is_cut_not_retried() {
        let rig = Rig::listed().await;
        rig.probe.refusing.store(true, Ordering::Relaxed);
        rig.hub.select(ADDRESS).unwrap();
        rig.until("cut", |state| state.link == HeadsetLink::Cut)
            .await;
        rig.scans_pass(8).await;
        assert_eq!(rig.link(), HeadsetLink::Cut);
        assert_eq!(rig.opens(), 1);
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn headphones_that_leave_are_forgotten_and_come_back_idle() {
        let rig = Rig::listed().await;
        rig.connected().await;
        let gone = rig.source.detach(ADDRESS).expect("listed");
        tokio::time::timeout(
            PATIENCE,
            rig.hub.subscribe_headsets().wait_for(Vec::is_empty),
        )
        .await
        .expect("forgotten")
        .unwrap();
        assert!(rig.hub.state(ADDRESS).is_none());

        rig.source.attach(gone);
        tokio::time::timeout(
            PATIENCE,
            rig.hub
                .subscribe_headsets()
                .wait_for(|list| !list.is_empty()),
        )
        .await
        .expect("listed again")
        .unwrap();
        let state = rig
            .until("back, idle and unknown", |state| {
                state.link == HeadsetLink::Idle
            })
            .await;
        assert!(state.ambient.is_none() && state.battery.is_none());

        // Their link of the last visit is not in the way.
        rig.connected().await;
        rig.supervisor.shutdown().await;
    }

    fn present_hub() -> HeadsetHub {
        let (hub, requests) = HeadsetHub::new();
        // Nobody carries the requests out: the thread is driven directly.
        std::mem::forget(requests);
        hub.present(vec![info()]);
        hub
    }

    #[test]
    fn a_cancelled_link_writes_nothing_and_ends_released() {
        let hub = present_hub();
        let orders = Orders::default();
        orders.cancel();
        let outcome = run_session(Box::new(SimulatedHeadset::new()), &hub, ADDRESS, &orders);
        assert_eq!(outcome, Outcome::Released);
        assert_eq!(
            hub.state(ADDRESS).unwrap(),
            HeadsetState::unknown(HeadsetLink::Idle)
        );
    }

    #[test]
    fn a_change_the_session_refuses_does_not_end_the_link() {
        let hub = present_hub();
        let orders = Arc::new(Orders::default());
        // Before the headphones have said anything, an ambient change cannot
        // be built: the session refuses it.
        orders.ask(|pending| {
            pending.ambient = Some(ambient_setting(noise_cancelling()));
        });
        let thread = {
            let (hub, orders) = (hub.clone(), Arc::clone(&orders));
            std::thread::spawn(move || {
                run_session(Box::new(SimulatedHeadset::new()), &hub, ADDRESS, &orders)
            })
        };
        let deadline = std::time::Instant::now() + PATIENCE;
        while hub.state(ADDRESS).unwrap().ambient.is_none() {
            assert!(std::time::Instant::now() < deadline, "the link went on");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(hub.state(ADDRESS).unwrap().link, HeadsetLink::Connected);
        orders.ask(|pending| pending.close = true);
        assert_eq!(thread.join().unwrap(), Outcome::Released);
    }

    #[test]
    fn orders_keep_only_the_latest_change_of_each_kind() {
        let orders = Orders::default();
        let ambient = |level| AmbientSetting {
            mode: WireMode::Ambient,
            level,
            focus_on_voice: false,
        };
        orders.ask(|pending| pending.ambient = Some(ambient(3)));
        orders.ask(|pending| pending.ambient = Some(ambient(9)));
        orders.ask(|pending| pending.equalizer = None);
        let taken = orders.take();
        assert_eq!(taken.ambient, Some(ambient(9)));
        assert_eq!(taken.commands().count(), 1);
        let nothing = orders.take();
        assert!(!nothing.close && nothing.commands().count() == 0);
    }
}
