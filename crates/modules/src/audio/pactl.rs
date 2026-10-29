//! [`AudioBackend`] driving the session audio server (PulseAudio, or
//! PipeWire's PulseAudio service) through `pactl` ≥ 16 (`--format=json`).
//!
//! Every call runs `pactl` directly (never through a shell), each argument
//! passed separately: device names are never interpolated into a command
//! line. `pactl` stops parsing its options at the sub-command, so a device
//! name or a negative volume (`-3277`) is never taken for an option.

use std::ffi::OsString;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value};
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};

use super::backend::{AudioBackend, AudioChange, AudioChanges, AudioError};
use super::state::{AudioDevice, AudioSnapshot, Direction, Volume};

/// Program run, looked up in `PATH`.
const PACTL: &str = "pactl";

/// Longest wait for a `pactl` call (a stuck server must not stall the module).
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// Messages of `pactl subscribe` are translated: force the untranslated
/// ones, which [`is_relevant_event`] recognises.
const UNTRANSLATED_LOCALE: (&str, &str) = ("LC_ALL", "C");

/// What `pactl` prints for a missing string (it also does so for strings it
/// cannot encode in JSON, such as non-ASCII descriptions).
const NULL_STRING: &str = "(null)";

/// Properties holding a readable device name, by preference.
const DESCRIPTION_PROPERTIES: [&str; 3] = ["device.description", "node.nick", "device.nick"];

/// Property telling a monitor source apart.
const DEVICE_CLASS_PROPERTY: &str = "device.class";
const MONITOR_CLASS: &str = "monitor";
const MONITOR_SUFFIX: &str = ".monitor";

/// Facilities of `pactl subscribe` whose changes alter what the module shows.
/// Others (clients, streams, modules) are ignored: every `pactl` call of the
/// module itself creates and removes a client.
const RELEVANT_FACILITIES: [&str; 4] = ["sink", "source", "server", "card"];

/// Audio server reached through `pactl`.
#[derive(Debug, Clone)]
pub struct PactlBackend {
    program: OsString,
    /// Arguments given before those of each call (empty for `pactl`).
    leading_args: Vec<OsString>,
}

impl Default for PactlBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl PactlBackend {
    /// Backend running `pactl` from `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self::with_program(PACTL)
    }

    /// Backend running `program` instead of `pactl` (tests).
    #[must_use]
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            leading_args: Vec::new(),
        }
    }

    /// Backend running the shell script `script` instead of `pactl`. The
    /// script is read by `/bin/sh` rather than executed: executing a file
    /// just written fails (`ETXTBSY`) when another thread forks meanwhile.
    #[cfg(test)]
    fn shell_script(script: impl Into<OsString>) -> Self {
        Self {
            program: "/bin/sh".into(),
            leading_args: vec![script.into()],
        }
    }

    fn command(&self, args: &[String]) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.leading_args)
            .args(args)
            .env(UNTRANSLATED_LOCALE.0, UNTRANSLATED_LOCALE.1)
            .stdin(Stdio::null())
            .kill_on_drop(true);
        command
    }

    fn spawn_error(&self, source: std::io::Error) -> AudioError {
        AudioError::Spawn {
            program: self.program.to_string_lossy().into_owned(),
            source,
        }
    }

    /// Runs `pactl args…` and returns what it printed.
    async fn run(&self, args: &[String]) -> Result<Vec<u8>, AudioError> {
        let name = args.join(" ");
        let mut command = self.command(args);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let output = tokio::time::timeout(COMMAND_TIMEOUT, command.output())
            .await
            .map_err(|_| AudioError::Timeout {
                command: name.clone(),
                timeout: COMMAND_TIMEOUT,
            })?
            .map_err(|source| self.spawn_error(source))?;
        if !output.status.success() {
            return Err(AudioError::Failed {
                command: name,
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(output.stdout)
    }

    async fn run_json(&self, args: &[String]) -> Result<Value, AudioError> {
        let output = self.run(args).await?;
        serde_json::from_slice(&output).map_err(|error| AudioError::InvalidOutput {
            command: args.join(" "),
            reason: error.to_string(),
        })
    }
}

#[async_trait]
impl AudioBackend for PactlBackend {
    async fn snapshot(&self) -> Result<AudioSnapshot, AudioError> {
        let (info_args, sinks_args, sources_args) = (
            info_args(),
            list_args(Direction::Output),
            list_args(Direction::Input),
        );
        let (info, sinks, sources) = tokio::try_join!(
            self.run_json(&info_args),
            self.run_json(&sinks_args),
            self.run_json(&sources_args),
        )?;
        Ok(parse_snapshot(&info, &sinks, &sources))
    }

    async fn apply(&self, change: &AudioChange) -> Result<(), AudioError> {
        self.run(&change_args(change)).await.map(drop)
    }

    async fn subscribe(&self) -> Result<Box<dyn AudioChanges>, AudioError> {
        let mut child = self
            .command(&subscribe_args())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|source| self.spawn_error(source))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AudioError::InvalidOutput {
                command: subscribe_args().join(" "),
                reason: "no standard output".into(),
            })?;
        Ok(Box::new(PactlChanges {
            _child: child,
            lines: BufReader::new(stdout).lines(),
        }))
    }
}

/// Lines of a running `pactl subscribe`, killed when dropped.
struct PactlChanges {
    _child: Child,
    lines: Lines<BufReader<ChildStdout>>,
}

#[async_trait]
impl AudioChanges for PactlChanges {
    async fn changed(&mut self) -> bool {
        loop {
            match self.lines.next_line().await {
                Ok(Some(line)) if is_relevant_event(&line) => return true,
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => return false,
            }
        }
    }
}

fn strings<const N: usize>(args: [&str; N]) -> Vec<String> {
    args.map(str::to_owned).to_vec()
}

/// `pactl --format=json info`.
fn info_args() -> Vec<String> {
    strings(["--format=json", "info"])
}

/// `pactl --format=json list sinks|sources`.
fn list_args(direction: Direction) -> Vec<String> {
    let list = match direction {
        Direction::Output => "sinks",
        Direction::Input => "sources",
    };
    strings(["--format=json", "list", list])
}

/// `pactl subscribe`.
fn subscribe_args() -> Vec<String> {
    strings(["subscribe"])
}

/// Arguments applying `change`.
fn change_args(change: &AudioChange) -> Vec<String> {
    match change {
        // A signed integer is a relative change in PulseAudio units, added
        // to every channel.
        AudioChange::SinkVolume { sink, delta } => {
            vec!["set-sink-volume".into(), sink.clone(), format!("{delta:+}")]
        }
        AudioChange::Mute {
            direction,
            device,
            muted,
        } => {
            let command = match direction {
                Direction::Output => "set-sink-mute",
                Direction::Input => "set-source-mute",
            };
            let state = if *muted { "1" } else { "0" };
            vec![command.into(), device.clone(), state.into()]
        }
        AudioChange::Default { direction, device } => {
            let command = match direction {
                Direction::Output => "set-default-sink",
                Direction::Input => "set-default-source",
            };
            vec![command.into(), device.clone()]
        }
    }
}

/// Whether a line of `pactl subscribe` (untranslated: `Event 'change' on
/// sink #54`) reports a change of a [`RELEVANT_FACILITIES`].
fn is_relevant_event(line: &str) -> bool {
    let Some((_, target)) = line.split_once(" on ") else {
        return false;
    };
    let facility = target.split_whitespace().next().unwrap_or_default();
    line.starts_with("Event ") && RELEVANT_FACILITIES.contains(&facility)
}

/// Builds the snapshot from the JSON of `info`, `list sinks` and `list
/// sources`. Tolerant: entries without a name are skipped, missing fields
/// take neutral values.
fn parse_snapshot(info: &Value, sinks: &Value, sources: &Value) -> AudioSnapshot {
    let info = ServerInfo::deserialize(info).unwrap_or_default();
    AudioSnapshot {
        sinks: parse_devices(sinks, false),
        sources: parse_devices(sources, true),
        default_sink: info.default_sink_name.filter(|name| is_known(name)),
        default_source: info.default_source_name.filter(|name| is_known(name)),
    }
}

fn parse_devices(list: &Value, skip_monitors: bool) -> Vec<AudioDevice> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| RawDevice::deserialize(entry).ok())
        .filter(|device| !(skip_monitors && device.is_monitor()))
        .filter_map(RawDevice::into_device)
        .collect()
}

fn is_known(text: &str) -> bool {
    !text.is_empty() && text != NULL_STRING
}

/// Output of `pactl --format=json info`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ServerInfo {
    #[serde(alias = "default_sink")]
    default_sink_name: Option<String>,
    #[serde(alias = "default_source")]
    default_source_name: Option<String>,
}

/// One entry of `pactl --format=json list sinks|sources`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawDevice {
    name: Option<String>,
    description: Option<String>,
    mute: Value,
    volume: Map<String, Value>,
    /// For a source: the output it monitors (empty for a real input).
    monitor_source: Option<String>,
    properties: Map<String, Value>,
}

impl RawDevice {
    fn property(&self, key: &str) -> Option<&str> {
        self.properties
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| is_known(value))
    }

    fn is_monitor(&self) -> bool {
        self.monitor_source.as_deref().is_some_and(is_known)
            || self.property(DEVICE_CLASS_PROPERTY) == Some(MONITOR_CLASS)
            || self
                .name
                .as_deref()
                .is_some_and(|name| name.ends_with(MONITOR_SUFFIX))
    }

    fn description(&self, name: &str) -> String {
        self.description
            .as_deref()
            .filter(|description| is_known(description))
            .or_else(|| {
                DESCRIPTION_PROPERTIES
                    .iter()
                    .find_map(|key| self.property(key))
            })
            .unwrap_or(name)
            .to_owned()
    }

    fn muted(&self) -> bool {
        match &self.mute {
            Value::Bool(muted) => *muted,
            Value::String(text) => matches!(text.as_str(), "yes" | "true" | "1"),
            Value::Number(number) => number.as_u64().is_some_and(|value| value != 0),
            _ => false,
        }
    }

    fn volume(&self) -> Volume {
        let channels = self
            .volume
            .values()
            .filter_map(|channel| channel.get("value")?.as_u64())
            .filter_map(|level| u32::try_from(level).ok())
            .collect();
        Volume::new(channels)
    }

    fn into_device(self) -> Option<AudioDevice> {
        let name = self.name.clone().filter(|name| is_known(name))?;
        Some(AudioDevice {
            description: self.description(&name),
            volume: self.volume(),
            muted: self.muted(),
            name,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use serde_json::json;

    use super::*;

    const INFO: &str = include_str!("fixtures/pipewire-info.json");
    const SINKS: &str = include_str!("fixtures/pipewire-sinks.json");
    const SOURCES: &str = include_str!("fixtures/pipewire-sources.json");
    const INTERNAL_OUTPUT: &str = "alsa_output.pci-0000_00_1f.3.analog-stereo";
    const INTERNAL_INPUT: &str = "alsa_input.pci-0000_00_1f.3.analog-stereo";

    fn json(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn real_pipewire_output_is_parsed() {
        let snapshot = parse_snapshot(&json(INFO), &json(SINKS), &json(SOURCES));
        assert_eq!(snapshot.default_sink.as_deref(), Some(INTERNAL_OUTPUT));
        assert_eq!(snapshot.default_source.as_deref(), Some(INTERNAL_INPUT));
        let [sink] = snapshot.sinks.as_slice() else {
            panic!("one sink expected: {:?}", snapshot.sinks);
        };
        // `description` is "(null)" (non-ASCII text): taken from the properties.
        assert_eq!(sink.description, "Audio interne");
        assert!(sink.muted);
        assert_eq!(sink.volume.channels(), [16_384, 16_384]);
        assert_eq!(sink.volume.percent(), 25);
        // The monitor of the output is not an input.
        let names: Vec<_> = snapshot.sources.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, [INTERNAL_INPUT]);
        assert_eq!(snapshot.sources[0].volume.percent(), 71);
        assert!(!snapshot.sources[0].muted);
    }

    #[test]
    fn parsing_tolerates_missing_and_variant_fields() {
        let sinks = json!([
            { "name": "bluez_output.headset", "description": "Casque", "mute": "yes",
              "volume": { "mono": { "value": 40_000 } } },
            { "name": "hdmi", "mute": "no", "properties": { "node.nick": "HDMI" } },
            { "name": "bare", "volume": { "front-left": { "value_percent": "10%" } } },
            { "description": "no name" },
            "garbage",
        ]);
        let sources = json!([
            { "name": "monitor", "monitor_source": "hdmi" },
            { "name": "classed", "properties": { "device.class": "monitor" } },
            { "name": "suffixed.monitor" },
            { "name": "usb_mic", "monitor_source": "", "mute": true },
        ]);
        let snapshot = parse_snapshot(&json!({ "default_sink": "hdmi" }), &sinks, &sources);
        let sinks: Vec<_> = snapshot
            .sinks
            .iter()
            .map(|d| {
                (
                    d.name.as_str(),
                    d.description.as_str(),
                    d.muted,
                    d.volume.channels().len(),
                )
            })
            .collect();
        assert_eq!(
            sinks,
            [
                ("bluez_output.headset", "Casque", true, 1),
                ("hdmi", "HDMI", false, 0),
                ("bare", "bare", false, 0),
            ]
        );
        assert_eq!(snapshot.sources.len(), 1);
        assert!(snapshot.sources[0].muted);
        assert_eq!(snapshot.default_sink.as_deref(), Some("hdmi"));
        assert_eq!(snapshot.default_source, None);

        let empty = parse_snapshot(&json!("?"), &json!({}), &Value::Null);
        assert_eq!(empty, AudioSnapshot::default());
        let null = parse_snapshot(
            &json!({ "default_sink_name": "(null)" }),
            &json!([]),
            &json!([]),
        );
        assert_eq!(null.default_sink, None);
    }

    #[test]
    fn changes_become_separate_arguments() {
        let name = "weird name; $(rm -rf ~)";
        let cases = [
            (
                AudioChange::SinkVolume {
                    sink: name.into(),
                    delta: -3277,
                },
                vec!["set-sink-volume", name, "-3277"],
            ),
            (
                AudioChange::SinkVolume {
                    sink: "s".into(),
                    delta: 3277,
                },
                vec!["set-sink-volume", "s", "+3277"],
            ),
            (
                AudioChange::Mute {
                    direction: Direction::Output,
                    device: "s".into(),
                    muted: true,
                },
                vec!["set-sink-mute", "s", "1"],
            ),
            (
                AudioChange::Mute {
                    direction: Direction::Input,
                    device: "m".into(),
                    muted: false,
                },
                vec!["set-source-mute", "m", "0"],
            ),
            (
                AudioChange::Default {
                    direction: Direction::Output,
                    device: "s".into(),
                },
                vec!["set-default-sink", "s"],
            ),
            (
                AudioChange::Default {
                    direction: Direction::Input,
                    device: "m".into(),
                },
                vec!["set-default-source", "m"],
            ),
        ];
        for (change, expected) in cases {
            assert_eq!(change_args(&change), expected);
        }
        assert_eq!(info_args(), ["--format=json", "info"]);
        assert_eq!(
            list_args(Direction::Input),
            ["--format=json", "list", "sources"]
        );
        assert_eq!(subscribe_args(), ["subscribe"]);
    }

    #[test]
    fn only_device_and_server_events_are_relevant() {
        assert!(is_relevant_event("Event 'change' on sink #54"));
        assert!(is_relevant_event("Event 'new' on source #60"));
        assert!(is_relevant_event("Event 'change' on server #4294967295"));
        assert!(is_relevant_event("Event 'remove' on card #3"));
        assert!(!is_relevant_event("Event 'new' on client #201"));
        assert!(!is_relevant_event("Event 'change' on sink-input #12"));
        assert!(!is_relevant_event("Event 'change' on source-output #12"));
        assert!(!is_relevant_event("Got SIGINT, exiting."));
        assert!(!is_relevant_event(""));
    }

    /// A fake `pactl`: a shell script printing the fixtures and recording
    /// its arguments, one per line, in `args.log`.
    fn fake_pactl(dir: &Path, body: &str) -> PathBuf {
        for (name, content) in [
            ("info.json", INFO),
            ("sinks.json", SINKS),
            ("sources.json", SOURCES),
        ] {
            std::fs::write(dir.join(name), content).unwrap();
        }
        let script = dir.join("pactl");
        let log = dir.join("args.log");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncd '{}'\nfor arg in \"$@\"; do echo \"$arg\" >> '{}'; done\necho \"LC_ALL=$LC_ALL\" >> '{}'\n{body}\n",
                dir.display(),
                log.display(),
                log.display()
            ),
        )
        .unwrap();
        script
    }

    const FIXTURE_LISTER: &str = r#"case "$2" in
  info) cat info.json ;;
  list) cat "$3.json" ;;
esac"#;

    #[tokio::test]
    async fn the_snapshot_comes_from_three_json_calls() {
        let dir = tempfile::tempdir().unwrap();
        let backend = PactlBackend::shell_script(fake_pactl(dir.path(), FIXTURE_LISTER));
        let snapshot = backend.snapshot().await.unwrap();
        assert_eq!(snapshot.sinks.len(), 1);
        assert_eq!(snapshot.sources.len(), 1);
        let log = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
        assert!(log.contains("LC_ALL=C"));
        assert_eq!(log.matches("--format=json").count(), 3);
    }

    #[tokio::test]
    async fn device_names_reach_pactl_as_one_argument() {
        let dir = tempfile::tempdir().unwrap();
        let backend = PactlBackend::shell_script(fake_pactl(dir.path(), ""));
        let name = "a 'quoted' $(name)";
        backend
            .apply(&AudioChange::Default {
                direction: Direction::Output,
                device: name.into(),
            })
            .await
            .unwrap();
        let log = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
        assert_eq!(log, format!("set-default-sink\n{name}\nLC_ALL=C\n"));
    }

    #[tokio::test]
    async fn failures_and_garbage_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let failing = PactlBackend::shell_script(fake_pactl(
            dir.path(),
            "echo 'Connection failure' >&2\nexit 1",
        ));
        let error = failing.snapshot().await.unwrap_err();
        assert!(
            matches!(&error, AudioError::Failed { stderr, .. } if stderr == "Connection failure"),
            "{error}"
        );

        let other = tempfile::tempdir().unwrap();
        let garbage = PactlBackend::shell_script(fake_pactl(other.path(), "echo 'not json'"));
        assert!(matches!(
            garbage.snapshot().await,
            Err(AudioError::InvalidOutput { .. })
        ));

        let missing = PactlBackend::with_program("/nonexistent/bergadeck/pactl");
        assert!(matches!(
            missing.snapshot().await,
            Err(AudioError::Spawn { .. })
        ));
        assert!(missing.subscribe().await.is_err());
    }

    #[tokio::test]
    async fn subscriptions_report_relevant_events_until_pactl_exits() {
        let dir = tempfile::tempdir().unwrap();
        let body = "echo \"Event 'new' on client #1\"\necho \"Event 'change' on sink #54\"\necho \"Event 'remove' on client #1\"";
        let backend = PactlBackend::shell_script(fake_pactl(dir.path(), body));
        let mut changes = backend.subscribe().await.unwrap();
        assert!(changes.changed().await);
        assert!(!changes.changed().await);
    }
}
