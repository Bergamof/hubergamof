//! `hubergamof headset-test`: tries Sony headphones from a terminal.
//!
//! Reads their state, listens to what they announce, or makes the one change
//! typed. It talks to the headphones through BlueZ and changes nothing else.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Subcommand, ValueEnum};
use hubergamof_headset::bluez;
use hubergamof_headset::diagnostics::{self, Action, DataKind, Settings};
use hubergamof_headset::link::Link;
use hubergamof_headset::payload::{AmbientMode, AmbientSetting, MAX_AMBIENT_LEVEL, Query};

/// Level used when none is given.
const DEFAULT_LEVEL: u8 = 10;
/// Length of `headset-test watch` when none is given, and its longest.
const DEFAULT_WATCH_SECONDS: u64 = 60;
const MAX_WATCH_SECONDS: u64 = 600;
/// Bytes of a Bluetooth address, `AA:BB:CC:DD:EE:FF`.
const ADDRESS_GROUPS: usize = 6;

/// The data type of the frames sent (the right one for the WH-1000XM4 is not known yet).
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DataKindArg {
    /// `0x0C`, what most headphones use.
    Mdr,
    /// `0x0E`.
    Mdr2,
    /// `0x00`.
    Data,
}

impl From<DataKindArg> for DataKind {
    fn from(kind: DataKindArg) -> Self {
        match kind {
            DataKindArg::Mdr => Self::Mdr,
            DataKindArg::Mdr2 => Self::MdrNo2,
            DataKindArg::Data => Self::Data,
        }
    }
}

/// The ambient sound modes.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AmbientModeArg {
    /// Neither noise cancelling nor ambient sound.
    Off,
    /// Let outside sound in.
    Ambient,
    /// Cancel noise.
    NoiseCancelling,
    /// Reduce wind noise (only if the headphones report it).
    WindReduction,
}

impl From<AmbientModeArg> for AmbientMode {
    fn from(mode: AmbientModeArg) -> Self {
        match mode {
            AmbientModeArg::Off => Self::Off,
            AmbientModeArg::Ambient => Self::Ambient,
            AmbientModeArg::NoiseCancelling => Self::NoiseCancelling,
            AmbientModeArg::WindReduction => Self::WindReduction,
        }
    }
}

/// The queries `probe` sends, from a closed list.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum QueryArg {
    /// The first command of a session.
    ProtocolInfo,
    /// Noise cancelling and ambient sound.
    Ambient,
    /// Battery.
    Battery,
    /// Equalizer.
    Equalizer,
    /// Connected devices (payload unknown: the bare code is sent).
    ConnectionStatus,
    /// Paired devices information (payload unknown: the bare code is sent).
    DeviceInfo,
}

impl From<QueryArg> for Query {
    fn from(query: QueryArg) -> Self {
        match query {
            QueryArg::ProtocolInfo => Self::ProtocolInfo,
            QueryArg::Ambient => Self::Ambient,
            QueryArg::Battery => Self::Battery,
            QueryArg::Equalizer => Self::Equalizer,
            QueryArg::ConnectionStatus => Self::ConnectionStatus,
            QueryArg::DeviceInfo => Self::DeviceInfo,
        }
    }
}

/// What `headset-test` does.
#[derive(Debug, Subcommand)]
pub enum HeadsetTestAction {
    /// List the paired Sony headphones BlueZ knows (nothing is connected).
    List,
    /// Read and print the state of the headphones (nothing is changed).
    Info,
    /// Change the ambient sound.
    Ambient {
        /// What to do.
        #[arg(value_enum)]
        mode: AmbientModeArg,
        /// Outside sound level, 0 to 20 (used by `off` and `ambient`).
        #[arg(long, default_value_t = DEFAULT_LEVEL, value_parser = clap::value_parser!(u8).range(0..=i64::from(MAX_AMBIENT_LEVEL)))]
        level: u8,
        /// Let voices through.
        #[arg(long)]
        voice: bool,
    },
    /// Ask for the states, then print everything the headphones announce.
    Watch {
        /// How long to listen, in seconds.
        #[arg(long, default_value_t = DEFAULT_WATCH_SECONDS, value_parser = clap::value_parser!(u64).range(1..=MAX_WATCH_SECONDS))]
        seconds: u64,
    },
    /// Send one query from a closed list and print the answers.
    Probe {
        /// The query.
        #[arg(value_enum)]
        query: QueryArg,
    },
}

impl HeadsetTestAction {
    /// What to do on the headphones; `None` for `list`, which does not connect.
    pub fn into_action(self) -> Option<Action> {
        match self {
            Self::List => None,
            Self::Info => Some(Action::Info),
            Self::Ambient { mode, level, voice } => Some(Action::Ambient(AmbientSetting {
                mode: mode.into(),
                level,
                focus_on_voice: voice,
            })),
            Self::Watch { seconds } => Some(Action::Watch(Duration::from_secs(seconds))),
            Self::Probe { query } => Some(Action::Probe(query.into())),
        }
    }
}

/// Refuses an address that is not `AA:BB:CC:DD:EE:FF`, before asking BlueZ.
pub fn parse_address(text: &str) -> Result<String, String> {
    let groups: Vec<&str> = text.split(':').collect();
    let valid = groups.len() == ADDRESS_GROUPS
        && groups
            .iter()
            .all(|group| group.len() == 2 && group.bytes().all(|b| b.is_ascii_hexdigit()));
    if valid {
        Ok(text.to_ascii_uppercase())
    } else {
        Err("expected an address like AA:BB:CC:DD:EE:FF".into())
    }
}

/// Does one action on the headphones and says what happened.
///
/// Ctrl-C, SIGTERM and SIGHUP end a watch early.
pub async fn run(
    address: Option<String>,
    data_kind: DataKindArg,
    action: HeadsetTestAction,
    termination: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let Some(action) = action.into_action() else {
        return list().await;
    };
    eprintln!("Connecting through BlueZ...");
    let mut connection = bluez::connect(address.as_deref())
        .await
        .context("cannot connect to the headphones")?;
    eprintln!(
        "Connected to {} ({}).",
        connection.device.name, connection.device.address
    );
    let stop = Arc::new(AtomicBool::new(false));
    let waiter = tokio::spawn({
        let stop = Arc::clone(&stop);
        async move {
            termination.await;
            stop.store(true, Ordering::Relaxed);
        }
    });
    let settings = Settings {
        data_kind: data_kind.into(),
        ..Settings::default()
    };
    let outcome = tokio::task::spawn_blocking(move || {
        let mut link = Link::new(&mut connection.channel);
        diagnostics::run(
            &mut link,
            &action,
            settings,
            &mut std::io::stdout().lock(),
            &stop,
        )?;
        // The connection holds the profile BlueZ calls back: it lives until here.
        drop(connection);
        anyhow::Ok(())
    })
    .await;
    waiter.abort();
    outcome.context("the headphones task failed")?
}

async fn list() -> Result<()> {
    let devices = bluez::list_devices()
        .await
        .context("cannot list the headphones")?;
    let mut out = std::io::stdout().lock();
    if devices.is_empty() {
        writeln!(out, "No paired Sony headphones found.")?;
    }
    for device in devices {
        writeln!(
            out,
            "{}  {}  {}",
            device.address,
            if device.connected {
                "connected"
            } else {
                "not connected"
            },
            device.name
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, Command};
    use clap::Parser;

    fn action(args: &[&str]) -> Option<Action> {
        let mut all = vec!["hubergamof", "headset-test"];
        all.extend_from_slice(args);
        match Cli::try_parse_from(all).unwrap().command {
            Some(Command::HeadsetTest { action, .. }) => action.into_action(),
            other => panic!("not a headset-test: {other:?}"),
        }
    }

    #[test]
    fn headset_test_actions_are_parsed() {
        assert_eq!(action(&["list"]), None);
        assert_eq!(action(&["info"]), Some(Action::Info));
        assert_eq!(
            action(&["ambient", "ambient", "--level", "7", "--voice"]),
            Some(Action::Ambient(AmbientSetting {
                mode: AmbientMode::Ambient,
                level: 7,
                focus_on_voice: true
            }))
        );
        assert_eq!(
            action(&["ambient", "noise-cancelling"]),
            Some(Action::Ambient(AmbientSetting {
                mode: AmbientMode::NoiseCancelling,
                level: DEFAULT_LEVEL,
                focus_on_voice: false
            }))
        );
        assert_eq!(
            action(&["watch"]),
            Some(Action::Watch(Duration::from_secs(60)))
        );
        assert_eq!(
            action(&["probe", "connection-status"]),
            Some(Action::Probe(Query::ConnectionStatus))
        );
    }

    #[test]
    fn headset_test_refuses_what_it_does_not_know() {
        for args in [
            &["headset-test"][..],
            &["headset-test", "ambient"],
            &["headset-test", "ambient", "loud"],
            &["headset-test", "ambient", "ambient", "--level", "21"],
            &["headset-test", "ambient", "ambient", "--level", "-1"],
            &["headset-test", "watch", "--seconds", "0"],
            &["headset-test", "watch", "--seconds", "601"],
            &["headset-test", "probe", "format-disk"],
            &["headset-test", "--address", "nope", "info"],
            &["headset-test", "--address", "AA:BB:CC:DD:EE", "info"],
            &["headset-test", "--address", "AA:BB:CC:DD:EE:GG", "info"],
            &["headset-test", "--data-type", "raw", "info"],
        ] {
            let mut all = vec!["hubergamof"];
            all.extend_from_slice(args);
            assert!(Cli::try_parse_from(all).is_err(), "{args:?}");
        }
    }

    #[test]
    fn an_address_is_checked_and_normalised() {
        assert_eq!(
            parse_address("aa:bb:cc:dd:ee:0f").unwrap(),
            "AA:BB:CC:DD:EE:0F"
        );
        assert!(parse_address("AA:BB:CC:DD:EE:0F:11").is_err());
    }
}
