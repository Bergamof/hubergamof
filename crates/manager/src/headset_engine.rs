//! Sony headphones: the supervisor that lists the headphones connected to
//! this computer, and the thread that serves the link to one of them.
//!
//! The headphones serve one settings client at a time (the Sony app on a
//! phone loses the channel when this one opens), so a link is opened only on
//! request (the person selected the headphones, or resumed the link) and
//! never opened again by itself: one that the headphones closed stays cut
//! until the person resumes it.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Duration;

use async_trait::async_trait;
use hubergamof_headset::bluez::{BluezHeadsets, HeadsetDevice};
use hubergamof_headset::channel::Channel;
use hubergamof_headset::payload::{
    AmbientMode as WireMode, AmbientSetting, AmbientState, Battery, EqualizerPreset as WirePreset,
    EqualizerSetting, EqualizerState, Event,
};
use hubergamof_headset::session::{Command, Session, SessionError};
use hubergamof_headset::simulated::SimulatedHeadset;
use hubergamof_protocol::{
    AmbientMode, AmbientSound, Equalizer, EqualizerChange, EqualizerPreset, HeadsetBattery,
    HeadsetInfo, HeadsetLink, HeadsetState,
};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::headset_hub::{Action, HeadsetHub, Request};

/// Delay between two looks at the headphones connected to this computer.
pub const SCAN_INTERVAL: Duration = Duration::from_secs(5);

/// How long a serving thread waits for the headphones before looking at the
/// requests again: the longest a request waits to be carried out.
const POLL: Duration = Duration::from_millis(100);

/// Where headphones come from: BlueZ, or simulated ones.
#[async_trait]
pub trait HeadsetSource: Send + Sync + 'static {
    /// The headphones connected to this computer now.
    async fn present(&self) -> Vec<HeadsetInfo>;

    /// Opens the settings channel of the headphones at `address`.
    ///
    /// # Errors
    /// Why the channel could not be opened, as text for the log.
    async fn open(&self, address: &str) -> Result<Box<dyn Channel + Send>, String>;
}

/// The Sony headphones BlueZ lists as connected.
#[derive(Default)]
pub struct BluezSource {
    /// Reached at the first look, and again after a failure: BlueZ may start
    /// after this manager.
    bluez: Mutex<Option<Arc<BluezHeadsets>>>,
}

impl BluezSource {
    async fn bluez(&self) -> Result<Arc<BluezHeadsets>, String> {
        let mut bluez = self.bluez.lock().await;
        if let Some(bluez) = bluez.as_ref() {
            return Ok(Arc::clone(bluez));
        }
        let started = Arc::new(BluezHeadsets::start().await.map_err(|e| e.to_string())?);
        *bluez = Some(Arc::clone(&started));
        Ok(started)
    }

    async fn connected(&self) -> Result<Vec<HeadsetDevice>, String> {
        let devices = self.bluez().await?.devices().await;
        match devices {
            Ok(devices) => Ok(devices.into_iter().filter(|d| d.connected).collect()),
            Err(error) => {
                *self.bluez.lock().await = None;
                Err(error.to_string())
            }
        }
    }
}

#[async_trait]
impl HeadsetSource for BluezSource {
    async fn present(&self) -> Vec<HeadsetInfo> {
        match self.connected().await {
            Ok(devices) => devices
                .into_iter()
                .map(|device| HeadsetInfo {
                    address: device.address,
                    name: device.name,
                })
                .collect(),
            Err(error) => {
                // At every look while Bluetooth is off: not a warning.
                tracing::debug!(%error, "cannot look for headphones");
                Vec::new()
            }
        }
    }

    async fn open(&self, address: &str) -> Result<Box<dyn Channel + Send>, String> {
        let device = self
            .connected()
            .await?
            .into_iter()
            .find(|device| device.address == address)
            .ok_or_else(|| format!("the headphones {address} are not connected"))?;
        let channel = self
            .bluez()
            .await?
            .connect(&device)
            .await
            .map_err(|error| error.to_string())?;
        Ok(Box::new(channel))
    }
}

/// Simulated headphones, for `--demo` and the tests.
pub struct SimulatedSource {
    headsets: std::sync::Mutex<Vec<(HeadsetInfo, SimulatedHeadset)>>,
}

impl SimulatedSource {
    /// A source of simulated headphones, all connected.
    #[must_use]
    pub fn new(headsets: Vec<(HeadsetInfo, SimulatedHeadset)>) -> Self {
        Self {
            headsets: std::sync::Mutex::new(headsets),
        }
    }

    /// Makes the headphones at `address` leave (or come back with
    /// [`SimulatedSource::attach`]).
    pub fn detach(&self, address: &str) -> Option<(HeadsetInfo, SimulatedHeadset)> {
        let mut headsets = self.lock();
        let index = headsets
            .iter()
            .position(|(info, _)| info.address == address)?;
        Some(headsets.remove(index))
    }

    /// Lists simulated headphones as connected.
    pub fn attach(&self, headset: (HeadsetInfo, SimulatedHeadset)) {
        self.lock().push(headset);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(HeadsetInfo, SimulatedHeadset)>> {
        // Plain data: nothing to recover from a poisoned lock.
        self.headsets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[async_trait]
impl HeadsetSource for SimulatedSource {
    async fn present(&self) -> Vec<HeadsetInfo> {
        self.lock().iter().map(|(info, _)| info.clone()).collect()
    }

    async fn open(&self, address: &str) -> Result<Box<dyn Channel + Send>, String> {
        let headsets = self.lock();
        let (_, headset) = headsets
            .iter()
            .find(|(info, _)| info.address == address)
            .ok_or_else(|| format!("the headphones {address} are not connected"))?;
        Ok(Box::new(headset.reconnect()))
    }
}

/// Why a serving thread ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The person closed the link.
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

/// What a serving thread is told.
enum Order {
    /// Close the link.
    Close,
    /// Change something.
    Apply(Command),
}

/// A link being opened or served.
struct Slot {
    id: u64,
    orders: Sender<Order>,
    task: JoinHandle<()>,
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
        requests: mpsc::UnboundedReceiver<Request>,
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
        self.task.abort();
        // Dropping the slots with the task ended the serving threads; they
        // finish within a poll.
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
        mut requests: mpsc::UnboundedReceiver<Request>,
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
        self.slots.retain(|address, slot| {
            let stays = present.iter().any(|info| &info.address == address);
            if !stays {
                // Dropping the slot ends its thread.
                slot.task.abort();
            }
            stays
        });
        self.hub.present(present);
    }

    fn handle(&mut self, request: Request) {
        let Request { address, action } = request;
        match action {
            Action::Open => {
                if self.hub.state(&address).map(|state| state.link) == Some(HeadsetLink::Idle) {
                    self.open(address);
                }
            }
            Action::Resume => {
                let link = self.hub.state(&address).map(|state| state.link);
                if matches!(
                    link,
                    Some(HeadsetLink::Idle | HeadsetLink::Released | HeadsetLink::Cut)
                ) {
                    self.open(address);
                }
            }
            Action::Release => self.order(&address, Order::Close),
            Action::SetAmbient(ambient) => {
                self.order(
                    &address,
                    Order::Apply(Command::SetAmbient(ambient_setting(ambient))),
                );
            }
            Action::SetEqualizer(change) => {
                if let Some(setting) = equalizer_setting(change) {
                    self.order(&address, Order::Apply(Command::SetEqualizer(setting)));
                } else {
                    tracing::debug!("an equalizer change the headphones cannot take, dropped");
                }
            }
        }
    }

    fn order(&self, address: &str, order: Order) {
        if let Some(slot) = self.slots.get(address) {
            // The thread is gone when it ended: its end is already reported.
            let _ = slot.orders.send(order);
        }
    }

    fn open(&mut self, address: String) {
        if self.slots.contains_key(&address) {
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        let (orders, received) = channel();
        let task = tokio::spawn(serve(
            Arc::clone(&self.source),
            self.hub.clone(),
            address.clone(),
            id,
            received,
            self.ended_sender.clone(),
        ));
        self.slots.insert(address, Slot { id, orders, task });
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
    orders: Receiver<Order>,
    ended: mpsc::UnboundedSender<Ended>,
) {
    hub.update_state(&address, |state| state.link = HeadsetLink::Connecting);
    let outcome = match source.open(&address).await {
        Ok(channel) => {
            let (hub, address) = (hub.clone(), address.clone());
            tokio::task::spawn_blocking(move || run_session(channel, &hub, &address, &orders))
                .await
                .unwrap_or(Outcome::Cut)
        }
        Err(error) => {
            tracing::warn!(%error, %address, "cannot open the link to the headphones");
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
    orders: &Receiver<Order>,
) -> Outcome {
    let mut session = Session::new(channel);
    if let Err(error) = session.start() {
        tracing::warn!(%error, %address, "the headphones did not answer");
        return Outcome::Cut;
    }
    hub.update_state(address, |state| state.link = HeadsetLink::Connected);
    loop {
        match orders.try_recv() {
            Ok(Order::Close) | Err(TryRecvError::Disconnected) => return Outcome::Released,
            Ok(Order::Apply(command)) => match session.apply(&command) {
                Ok(()) => {}
                Err(SessionError::Link(error)) => {
                    tracing::debug!(%error, %address, "the link to the headphones failed");
                    return Outcome::Cut;
                }
                Err(error) => tracing::debug!(%error, %address, "change not sent"),
            },
            Err(TryRecvError::Empty) => {}
        }
        match session.poll(POLL) {
            Ok(Some(event)) => hub.update_state(address, |state| apply_event(state, &event)),
            Ok(None) => {}
            Err(error) => {
                tracing::debug!(%error, %address, "the headphones closed the link");
                return Outcome::Cut;
            }
        }
    }
}

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

fn ambient_setting(ambient: AmbientSound) -> AmbientSetting {
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

fn equalizer_setting(change: EqualizerChange) -> Option<EqualizerSetting> {
    Some(match change {
        EqualizerChange::Preset { preset } => EqualizerSetting::Preset(wire_preset(preset)?),
        EqualizerChange::Manual { clear_bass, bands } => {
            EqualizerSetting::Manual { clear_bass, bands }
        }
    })
}

/// What an answer or a notification of the headphones changes of the state.
fn apply_event(state: &mut HeadsetState, event: &Event) {
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
    use std::time::Duration;

    use hubergamof_headset::payload::AmbientMode as WireMode;
    use hubergamof_protocol::{EqualizerChange, EqualizerPreset};
    use tokio::sync::watch;

    use super::*;

    const ADDRESS: &str = "AC:80:0A:0B:2E:61";
    const SCAN: Duration = Duration::from_millis(20);
    const PATIENCE: Duration = Duration::from_secs(5);

    fn info() -> HeadsetInfo {
        HeadsetInfo {
            address: ADDRESS.into(),
            name: "WH-1000XM4".into(),
        }
    }

    struct Rig {
        hub: HeadsetHub,
        source: Arc<SimulatedSource>,
        headset: SimulatedHeadset,
        supervisor: HeadsetSupervisor,
    }

    impl Rig {
        async fn listed() -> Self {
            let headset = SimulatedHeadset::new();
            let source = Arc::new(SimulatedSource::new(vec![(info(), headset.clone())]));
            let (hub, requests) = HeadsetHub::new();
            let supervisor =
                HeadsetSupervisor::start(Arc::clone(&source) as _, hub.clone(), requests, SCAN);
            let rig = Self {
                hub,
                source,
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

        /// Gives the scans time to prove nothing happens by itself.
        async fn let_scans_pass(&self) {
            tokio::time::sleep(SCAN * 8).await;
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
        rig.let_scans_pass().await;
        assert_eq!(rig.hub.state(ADDRESS).unwrap().link, HeadsetLink::Idle);
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn selecting_the_headphones_opens_the_link_and_tells_everything() {
        let rig = Rig::listed().await;
        let state = rig.connected().await;
        assert_eq!(
            state.ambient.unwrap().mode,
            AmbientMode::Ambient,
            "as the headphones are"
        );
        assert!(state.wind_reduction);
        assert_eq!(state.battery.unwrap().percent, 80);
        assert_eq!(state.equalizer.unwrap().preset, EqualizerPreset::Speech);
        rig.supervisor.shutdown().await;
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
        rig.let_scans_pass().await;
        assert_eq!(rig.hub.state(ADDRESS).unwrap().link, HeadsetLink::Released);

        rig.hub.resume(ADDRESS).unwrap();
        rig.until("connected again", |state| {
            state.link == HeadsetLink::Connected
        })
        .await;
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn a_link_the_headphones_cut_is_not_opened_again_until_resumed() {
        let rig = Rig::listed().await;
        rig.connected().await;
        rig.headset.cut();
        rig.until("cut", |state| state.link == HeadsetLink::Cut)
            .await;

        rig.let_scans_pass().await;
        rig.hub.select(ADDRESS).unwrap();
        rig.let_scans_pass().await;
        assert_eq!(rig.hub.state(ADDRESS).unwrap().link, HeadsetLink::Cut);

        rig.hub.resume(ADDRESS).unwrap();
        rig.until("connected again", |state| {
            state.link == HeadsetLink::Connected
        })
        .await;
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
        rig.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn a_link_that_cannot_be_opened_is_cut_not_retried() {
        struct Closed;

        #[async_trait]
        impl HeadsetSource for Closed {
            async fn present(&self) -> Vec<HeadsetInfo> {
                vec![info()]
            }

            async fn open(&self, _address: &str) -> Result<Box<dyn Channel + Send>, String> {
                Err("refused".into())
            }
        }

        let (hub, requests) = HeadsetHub::new();
        let supervisor = HeadsetSupervisor::start(Arc::new(Closed), hub.clone(), requests, SCAN);
        let mut state = loop {
            if let Some(state) = hub.subscribe_state(ADDRESS) {
                break state;
            }
            tokio::time::sleep(SCAN).await;
        };
        hub.select(ADDRESS).unwrap();
        tokio::time::timeout(PATIENCE, state.wait_for(|s| s.link == HeadsetLink::Cut))
            .await
            .expect("cut")
            .unwrap();
        supervisor.shutdown().await;
    }
}
