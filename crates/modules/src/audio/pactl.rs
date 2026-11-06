//! [`AudioBackend`] driving the session audio server (PulseAudio, or
//! PipeWire's PulseAudio service) through `pactl` ≥ 16 (`--format=json`).
//!
//! Every call runs `pactl` directly (never through a shell), each argument
//! passed separately: device names are never interpolated into a command
//! line. `pactl` stops parsing its options at the sub-command, so a device
//! name is never taken for an option; volumes are absolute (unsigned).
//!
//! `pactl` is looked up in the `PATH` of the manager. The manager is a user
//! service: its `PATH` is the user's own (systemd user manager or session),
//! which is trusted like any program the user runs; a `PATH` controlled by
//! someone else would already let them run code as the user.
//!
//! What `pactl` prints is bounded ([`MAX_OUTPUT_BYTES`],
//! [`MAX_ERROR_BYTES`], [`MAX_EVENT_LINE_BYTES`]): a broken or hostile tool
//! cannot make the manager hold unbounded memory.

use std::ffi::OsString;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdout, Command};

use super::backend::{AudioBackend, AudioChange, AudioChanges, AudioError, ReadScope};
use super::state::{AudioDevice, Defaults, Direction, PartialSnapshot, Volume};
use crate::process::{self, OutputLimits, RunError};

/// Program run, looked up in `PATH`.
const PACTL: &str = "pactl";

/// Longest wait for a `pactl` call (a stuck server must not stall the module).
pub(super) const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// Largest standard output of a call: far above the JSON of any real audio
/// setup (a few kilobytes per device).
const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// Part of the error output of a call kept in errors and logs.
const MAX_ERROR_BYTES: usize = 4 * 1024;

/// Longest line of `pactl subscribe` kept; the end of a longer line is
/// dropped (relevant lines are about 40 bytes).
const MAX_EVENT_LINE_BYTES: usize = 256;

/// Messages of `pactl subscribe` are translated: force the untranslated
/// ones, which [`event_scope`] recognises.
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

/// Audio server reached through `pactl`.
#[derive(Debug, Clone)]
pub struct PactlBackend {
    program: OsString,
    /// Arguments given before those of each call (empty for `pactl`).
    leading_args: Vec<OsString>,
}

impl PactlBackend {
    /// Backend running `pactl` from `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            program: PACTL.into(),
            leading_args: Vec::new(),
        }
    }

    /// Backend running the shell script `script` instead of `pactl` (tests).
    /// The script is read by `/bin/sh` rather than executed: executing a
    /// file just written fails (`ETXTBSY`) when another thread forks
    /// meanwhile.
    #[cfg(test)]
    pub(super) fn shell_script(script: impl Into<OsString>) -> Self {
        Self {
            program: "/bin/sh".into(),
            leading_args: vec![script.into()],
        }
    }

    /// Backend running `program` itself, possibly absent (tests).
    #[cfg(test)]
    pub(super) fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            leading_args: Vec::new(),
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

    /// Runs `pactl args…` and returns what it printed; the process is killed
    /// if it takes longer than [`COMMAND_TIMEOUT`] or prints more than
    /// [`MAX_OUTPUT_BYTES`].
    async fn run(&self, args: &[String]) -> Result<Vec<u8>, AudioError> {
        let limits = OutputLimits {
            stdout: Some(MAX_OUTPUT_BYTES),
            stderr: MAX_ERROR_BYTES,
        };
        let output = process::run(self.command(args), COMMAND_TIMEOUT, limits)
            .await
            .map_err(|error| match error {
                RunError::Spawn(source) => self.spawn_error(source),
                RunError::Timeout => AudioError::Timeout {
                    command: command_line(args),
                    timeout: COMMAND_TIMEOUT,
                },
                RunError::Output(reason) => AudioError::InvalidOutput {
                    command: command_line(args),
                    reason,
                },
            })?;
        if !output.status.success() {
            return Err(AudioError::Failed {
                command: command_line(args),
                status: output.status.to_string(),
                stderr: output.stderr,
            });
        }
        Ok(output.stdout)
    }

    async fn run_json(&self, args: &[String]) -> Result<Value, AudioError> {
        let output = self.run(args).await?;
        serde_json::from_slice(&output).map_err(|error| AudioError::InvalidOutput {
            command: command_line(args),
            reason: error.to_string(),
        })
    }

    /// Reads one part of the state, `None` when `wanted` is false.
    async fn read_part(
        &self,
        wanted: bool,
        args: Vec<String>,
    ) -> Result<Option<Value>, AudioError> {
        if wanted {
            self.run_json(&args).await.map(Some)
        } else {
            Ok(None)
        }
    }
}

impl Default for PactlBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// The arguments of a call as one line, for errors.
fn command_line(args: &[String]) -> String {
    args.join(" ")
}

#[async_trait]
impl AudioBackend for PactlBackend {
    async fn read(&self, scope: ReadScope) -> Result<PartialSnapshot, AudioError> {
        let (info, sinks, sources) = tokio::try_join!(
            self.read_part(scope.defaults, info_args()),
            self.read_part(scope.outputs, list_args(Direction::Output)),
            self.read_part(scope.inputs, list_args(Direction::Input)),
        )?;
        Ok(PartialSnapshot {
            sinks: sinks.map(|list| parse_devices(&list, false)),
            sources: sources.map(|list| parse_devices(&list, true)),
            defaults: info.map(|info| parse_defaults(&info)),
        })
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
                command: command_line(&subscribe_args()),
                reason: "no standard output".into(),
            })?;
        Ok(Box::new(PactlChanges {
            _child: child,
            lines: BoundedLines::new(BufReader::new(stdout), MAX_EVENT_LINE_BYTES),
        }))
    }
}

/// Lines of a running `pactl subscribe`, killed when dropped.
struct PactlChanges {
    _child: Child,
    lines: BoundedLines<BufReader<ChildStdout>>,
}

#[async_trait]
impl AudioChanges for PactlChanges {
    async fn next_change(&mut self) -> Option<ReadScope> {
        loop {
            match self.lines.next_line().await {
                Ok(Some(line)) => {
                    if let Some(scope) = event_scope(&String::from_utf8_lossy(&line)) {
                        return Some(scope);
                    }
                }
                Ok(None) | Err(_) => return None,
            }
        }
    }
}

/// Lines of `reader`, each cut to `limit` bytes. Cancel-safe: the part of a
/// line read before a cancellation is kept for the next call.
struct BoundedLines<R> {
    reader: R,
    limit: usize,
    line: Vec<u8>,
}

impl<R: AsyncBufRead + Unpin> BoundedLines<R> {
    fn new(reader: R, limit: usize) -> Self {
        Self {
            reader,
            limit,
            line: Vec::new(),
        }
    }

    /// The next line without its end, `None` at the end of the input.
    async fn next_line(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                return Ok((!self.line.is_empty()).then(|| std::mem::take(&mut self.line)));
            }
            let end = available.iter().position(|&byte| byte == b'\n');
            let content = &available[..end.unwrap_or(available.len())];
            let room = self.limit.saturating_sub(self.line.len());
            self.line
                .extend_from_slice(&content[..content.len().min(room)]);
            let consumed = end.map_or(available.len(), |end| end + 1);
            self.reader.consume(consumed);
            if end.is_some() {
                return Ok(Some(std::mem::take(&mut self.line)));
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
        // One unsigned integer per channel: absolute volumes in PulseAudio
        // units, channel by channel.
        AudioChange::SinkVolume { sink, volume } => {
            let mut args = vec!["set-sink-volume".to_owned(), sink.clone()];
            args.extend(volume.channels().iter().map(u32::to_string));
            args
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

/// What to read again after a line of `pactl subscribe` (untranslated:
/// `Event 'change' on sink #54`), `None` for events that do not alter what
/// the module shows: clients, streams and modules (every `pactl` call of the
/// module itself creates and removes a client).
fn event_scope(line: &str) -> Option<ReadScope> {
    let target = line.strip_prefix("Event ")?.split_once(" on ")?.1;
    match target.split_whitespace().next()? {
        "sink" => Some(ReadScope::OUTPUTS),
        "source" => Some(ReadScope::INPUTS),
        "server" => Some(ReadScope::DEFAULTS),
        // A card profile change adds and removes devices.
        "card" => Some(ReadScope::ALL),
        _ => None,
    }
}

/// Defaults from the JSON of `info`; tolerant, like [`parse_devices`].
fn parse_defaults(info: &Value) -> Defaults {
    let info = ServerInfo::deserialize(info).unwrap_or_default();
    Defaults {
        sink: info.default_sink_name.filter(|name| is_known(name)),
        source: info.default_source_name.filter(|name| is_known(name)),
    }
}

/// Devices from the JSON of `list sinks|sources`. Tolerant: entries without
/// a name are skipped, missing fields take neutral values.
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

    use super::super::backend::AudioFault;
    use serde_json::json;

    use super::super::state::AudioSnapshot;
    use super::*;
    use crate::process::TRUNCATION_MARK;

    const INFO: &str = include_str!("fixtures/pipewire-info.json");
    const SINKS: &str = include_str!("fixtures/pipewire-sinks.json");
    const SOURCES: &str = include_str!("fixtures/pipewire-sources.json");
    const INTERNAL_OUTPUT: &str = "alsa_output.pci-0000_00_1f.3.analog-stereo";
    const INTERNAL_INPUT: &str = "alsa_input.pci-0000_00_1f.3.analog-stereo";

    fn json(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// The snapshot built from the JSON of `info`, `list sinks` and `list
    /// sources`.
    fn parse_snapshot(info: &Value, sinks: &Value, sources: &Value) -> AudioSnapshot {
        let mut snapshot = AudioSnapshot::default();
        snapshot.update(PartialSnapshot {
            sinks: Some(parse_devices(sinks, false)),
            sources: Some(parse_devices(sources, true)),
            defaults: Some(parse_defaults(info)),
        });
        snapshot
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
                    volume: Volume::new(vec![3277, 65_536]),
                },
                vec!["set-sink-volume", name, "3277", "65536"],
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
    fn device_and_server_events_name_what_changed() {
        assert_eq!(
            event_scope("Event 'change' on sink #54"),
            Some(ReadScope::OUTPUTS)
        );
        assert_eq!(
            event_scope("Event 'new' on source #60"),
            Some(ReadScope::INPUTS)
        );
        assert_eq!(
            event_scope("Event 'change' on server #4294967295"),
            Some(ReadScope::DEFAULTS)
        );
        assert_eq!(
            event_scope("Event 'remove' on card #3"),
            Some(ReadScope::ALL)
        );
        assert_eq!(event_scope("Event 'new' on client #201"), None);
        assert_eq!(event_scope("Event 'change' on sink-input #12"), None);
        assert_eq!(event_scope("Event 'change' on source-output #12"), None);
        // Only lines starting with `Event `.
        assert_eq!(event_scope("Got SIGINT, on sink exiting."), None);
        assert_eq!(event_scope("x Event 'change' on sink #1"), None);
        assert_eq!(event_scope(""), None);
    }

    #[tokio::test]
    async fn event_lines_are_cut_to_their_bound() {
        let long = "x".repeat(MAX_EVENT_LINE_BYTES * 3);
        let input = format!("short\n{long}\nEvent 'change' on sink #1\nlast");
        let mut lines = BoundedLines::new(input.as_bytes(), MAX_EVENT_LINE_BYTES);
        assert_eq!(lines.next_line().await.unwrap().unwrap(), b"short");
        let cut = lines.next_line().await.unwrap().unwrap();
        assert_eq!(cut.len(), MAX_EVENT_LINE_BYTES);
        assert_eq!(
            lines.next_line().await.unwrap().unwrap(),
            b"Event 'change' on sink #1"
        );
        assert_eq!(lines.next_line().await.unwrap().unwrap(), b"last");
        assert_eq!(lines.next_line().await.unwrap(), None);
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
    async fn each_part_of_the_state_costs_one_json_call() {
        let dir = tempfile::tempdir().unwrap();
        let backend = PactlBackend::shell_script(fake_pactl(dir.path(), FIXTURE_LISTER));
        let full = backend.read(ReadScope::ALL).await.unwrap();
        assert_eq!(full.sinks.unwrap().len(), 1);
        assert_eq!(full.sources.unwrap().len(), 1);
        assert_eq!(
            full.defaults.unwrap().sink.as_deref(),
            Some(INTERNAL_OUTPUT)
        );
        let log = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
        assert!(log.contains("LC_ALL=C"));
        assert_eq!(log.matches("--format=json").count(), 3);

        let outputs = backend.read(ReadScope::OUTPUTS).await.unwrap();
        assert!(outputs.sinks.is_some());
        assert_eq!((outputs.sources, outputs.defaults), (None, None));
        let log = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
        assert_eq!(log.matches("--format=json").count(), 4);
        assert_eq!(log.matches("sinks").count(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_stuck_pactl_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let backend = PactlBackend::shell_script(fake_pactl(dir.path(), "exec sleep 30"));
        let started = tokio::time::Instant::now();
        let error = backend.read(ReadScope::DEFAULTS).await.unwrap_err();
        assert!(
            matches!(&error, AudioError::Timeout { command, timeout }
                if command == "--format=json info" && *timeout == COMMAND_TIMEOUT),
            "{error}"
        );
        assert_eq!(started.elapsed(), COMMAND_TIMEOUT);
    }

    #[tokio::test]
    async fn oversized_outputs_are_refused_and_errors_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let flood = format!("head -c {} /dev/zero", MAX_OUTPUT_BYTES + 1);
        let backend = PactlBackend::shell_script(fake_pactl(dir.path(), &flood));
        let error = backend.read(ReadScope::OUTPUTS).await.unwrap_err();
        assert!(
            matches!(&error, AudioError::InvalidOutput { reason, .. } if reason.starts_with("more than")),
            "{error}"
        );

        let other = tempfile::tempdir().unwrap();
        let noisy = format!(
            "head -c {} /dev/zero | tr '\\0' e >&2\nexit 1",
            MAX_ERROR_BYTES * 10
        );
        let backend = PactlBackend::shell_script(fake_pactl(other.path(), &noisy));
        let error = backend.read(ReadScope::OUTPUTS).await.unwrap_err();
        let AudioError::Failed { stderr, .. } = &error else {
            panic!("failure expected, got {error}");
        };
        assert_eq!(stderr.len(), MAX_ERROR_BYTES + TRUNCATION_MARK.len());
        assert!(stderr.starts_with(TRUNCATION_MARK));
    }

    #[tokio::test]
    async fn dropping_a_subscription_kills_pactl() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let body = format!("echo $$ > '{}'\nexec sleep 30", pid_file.display());
        let backend = PactlBackend::shell_script(fake_pactl(dir.path(), &body));
        let changes = backend.subscribe().await.unwrap();
        let pid = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(pid) = std::fs::read_to_string(&pid_file)
                    && let Ok(pid) = pid.trim().parse::<u32>()
                {
                    return pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(is_running(pid));
        drop(changes);
        tokio::time::timeout(Duration::from_secs(5), async {
            while is_running(pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("pactl subscribe still running");
    }

    /// Whether process `pid` exists and is not a zombie (killed, not reaped).
    fn is_running(pid: u32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            // `pid (comm) state …`: the state follows the last parenthesis.
            stat.rsplit_once(") ")
                .is_some_and(|(_, rest)| !rest.starts_with('Z'))
        })
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
        let error = failing.read(ReadScope::ALL).await.unwrap_err();
        assert!(
            matches!(&error, AudioError::Failed { stderr, .. } if stderr == "Connection failure"),
            "{error}"
        );

        let other = tempfile::tempdir().unwrap();
        let garbage = PactlBackend::shell_script(fake_pactl(other.path(), "echo 'not json'"));
        assert!(matches!(
            garbage.read(ReadScope::ALL).await,
            Err(AudioError::InvalidOutput { .. })
        ));

        let missing = PactlBackend {
            program: "/nonexistent/hubergamof/pactl".into(),
            leading_args: Vec::new(),
        };
        assert!(matches!(
            missing.read(ReadScope::ALL).await,
            Err(AudioError::Spawn { .. })
        ));
        assert!(missing.subscribe().await.is_err());
    }

    #[tokio::test]
    async fn faults_tell_a_missing_or_old_pactl_from_an_unreachable_server() {
        let missing = PactlBackend::with_program("/nonexistent/hubergamof/pactl");
        let error = missing.read(ReadScope::ALL).await.unwrap_err();
        assert_eq!(error.fault(), AudioFault::ToolMissing);

        // pactl 15 (Ubuntu 22.04) does not know `--format`.
        let dir = tempfile::tempdir().unwrap();
        let old = PactlBackend::shell_script(fake_pactl(
            dir.path(),
            "echo \"pactl: unrecognized option '--format=json'\" >&2\nexit 1",
        ));
        let error = old.read(ReadScope::ALL).await.unwrap_err();
        assert_eq!(error.fault(), AudioFault::ToolTooOld);

        let other = tempfile::tempdir().unwrap();
        let down = PactlBackend::shell_script(fake_pactl(
            other.path(),
            "echo 'Connection failure: Connection refused' >&2\nexit 1",
        ));
        let error = down.read(ReadScope::ALL).await.unwrap_err();
        assert_eq!(error.fault(), AudioFault::Unreachable);
    }

    #[tokio::test]
    async fn subscriptions_report_relevant_events_until_pactl_exits() {
        let dir = tempfile::tempdir().unwrap();
        let body = "echo \"Event 'new' on client #1\"\necho \"Event 'change' on sink #54\"\necho \"Event 'remove' on client #1\"";
        let backend = PactlBackend::shell_script(fake_pactl(dir.path(), body));
        let mut changes = backend.subscribe().await.unwrap();
        assert_eq!(changes.next_change().await, Some(ReadScope::OUTPUTS));
        assert_eq!(changes.next_change().await, None);
    }
}
