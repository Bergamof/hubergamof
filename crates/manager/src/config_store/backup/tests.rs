use hubergamof_protocol::{
    AdvancedSetting, GitAlias, MouseConfig, MouseModel, ShellAlias, SshHost,
};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use super::*;
use crate::device::virtual_deck::{DEMO_SERIAL, virtual_plus_info};
use crate::testing::builtin_registry;

const SERIAL: &str = "MX4-0042";

fn device_config() -> DeviceConfig {
    let dir = tempfile::tempdir().unwrap();
    let store = super::super::ConfigStore::new(dir.path(), std::sync::Arc::new(builtin_registry()));
    store.load(&virtual_plus_info(DEMO_SERIAL)).unwrap().config
}

fn full_backup() -> Backup {
    let mut documents = BTreeMap::new();
    documents.insert(
        BackupSection::Device(DEMO_SERIAL.to_owned()),
        BackupDocument::Device(Box::new(device_config())),
    );
    documents.insert(
        BackupSection::Mouse(SERIAL.to_owned()),
        BackupDocument::Mouse(Box::new(MouseConfig::default_for(
            SERIAL,
            MouseModel::MxMaster4,
        ))),
    );
    documents.insert(
        BackupSection::RadialMenu,
        BackupDocument::RadialMenu(RadialMenuConfig::default_menu()),
    );
    documents.insert(
        BackupSection::Modules,
        BackupDocument::Modules(AppModulesConfig::empty()),
    );
    documents.insert(
        BackupSection::Machine,
        BackupDocument::Machine(MachineConfig::empty()),
    );
    documents.insert(
        BackupSection::Git,
        BackupDocument::Git(Box::new(GitConfig::empty())),
    );
    documents.insert(BackupSection::Ssh, BackupDocument::Ssh(SshConfig::empty()));
    let mut images = BTreeMap::new();
    images.insert(
        DEMO_SERIAL.to_owned(),
        ArchiveImages::from([("logo.png".to_owned(), b"\x89PNG\r\n\x1a\nrest".to_vec())]),
    );
    Backup::new("1.4.2", 1_700_000_000, documents, images)
}

/// A zip with the given entries, whatever they are.
fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn manifest_json(sections: &[&str], format: u32) -> Vec<u8> {
    serde_json::to_vec(&BackupManifest {
        format,
        app_version: "1.4.2".into(),
        created_at: 1,
        sections: sections.iter().map(|s| (*s).to_owned()).collect(),
    })
    .unwrap()
}

#[test]
fn a_backup_survives_a_round_trip() {
    let backup = full_backup();
    let read = read_backup(&write_backup(&backup).unwrap()).unwrap();
    assert_eq!(read, backup);
    assert_eq!(read.manifest.format, BACKUP_FORMAT);
    assert!(read.manifest.sections.contains(&"ssh".to_owned()));
}

#[test]
fn the_documents_are_readable_json_in_the_archive() {
    let bytes = write_backup(&full_backup()).unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let names: Vec<String> = zip
        .file_names()
        .filter_map(|name| name.ok().map(std::borrow::Cow::into_owned))
        .collect();
    for expected in [
        "manifest.json",
        "config/modules.json",
        "config/ssh.json",
        "config/mice/MX4-0042.json",
        &format!("config/devices/{DEMO_SERIAL}.json"),
        &format!("images/{DEMO_SERIAL}/logo.png"),
    ] {
        assert!(names.iter().any(|n| n == expected), "missing {expected}");
    }
    let mut text = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("config/ssh.json").unwrap(), &mut text).unwrap();
    assert!(text.contains('\n'), "pretty-printed");
}

#[test]
fn section_names_round_trip_and_refuse_unsafe_serials() {
    for section in [
        BackupSection::Device("A1".into()),
        BackupSection::Mouse("M-1".into()),
        BackupSection::RadialMenu,
        BackupSection::Ssh,
    ] {
        assert_eq!(BackupSection::parse(&section.id()), Some(section));
    }
    assert_eq!(BackupSection::parse("devices/../x"), None);
    assert_eq!(BackupSection::parse("devices/"), None);
    assert_eq!(BackupSection::parse("updates"), None);
}

#[test]
fn a_backup_without_a_manifest_is_refused() {
    let ssh = serde_json::to_vec(&SshConfig::empty()).unwrap();
    let bytes = zip_of(&[("config/ssh.json", &ssh)]);
    assert!(matches!(
        read_backup(&bytes),
        Err(BackupError::MissingManifest)
    ));
}

#[test]
fn a_newer_format_is_refused_with_its_number() {
    let bytes = zip_of(&[("manifest.json", &manifest_json(&[], BACKUP_FORMAT + 1))]);
    assert!(matches!(
        read_backup(&bytes),
        Err(BackupError::NewerFormat(3))
    ));
}

#[test]
fn an_older_unknown_format_is_refused() {
    let bytes = zip_of(&[("manifest.json", &manifest_json(&[], 1))]);
    assert!(matches!(
        read_backup(&bytes),
        Err(BackupError::UnknownFormat(1))
    ));
}

#[test]
fn a_manifest_that_does_not_match_the_entries_is_refused() {
    let ssh = serde_json::to_vec(&SshConfig::empty()).unwrap();
    let listed_but_absent = zip_of(&[
        ("manifest.json", &manifest_json(&["ssh", "git"], 2)),
        ("config/ssh.json", &ssh),
    ]);
    assert!(matches!(
        read_backup(&listed_but_absent),
        Err(BackupError::ManifestMismatch)
    ));
    let present_but_unlisted = zip_of(&[
        ("manifest.json", &manifest_json(&[], 2)),
        ("config/ssh.json", &ssh),
    ]);
    assert!(matches!(
        read_backup(&present_but_unlisted),
        Err(BackupError::ManifestMismatch)
    ));
}

#[test]
fn entries_that_are_not_part_of_a_backup_are_refused() {
    let manifest = manifest_json(&[], 2);
    for name in [
        "../escape",
        "/etc/passwd",
        "config/../x.json",
        "config/updates.json",
        "config/devices/../x.json",
        "config/devices/a/b.json",
        "images/a/../b.png",
        "images/a/b/c.png",
        "images/../b.png",
        "notes.txt",
    ] {
        let bytes = zip_of(&[("manifest.json", &manifest), (name, b"x")]);
        assert!(
            matches!(
                read_backup(&bytes),
                Err(BackupError::Archive(ArchiveError::UnexpectedEntry(_)))
            ),
            "{name} should be refused"
        );
    }
}

#[test]
fn a_document_of_another_type_or_section_is_refused() {
    let machine = serde_json::to_vec(&MachineConfig::empty()).unwrap();
    let wrong_type = zip_of(&[
        ("manifest.json", &manifest_json(&["ssh"], 2)),
        ("config/ssh.json", &machine),
    ]);
    assert!(matches!(
        read_backup(&wrong_type),
        Err(BackupError::Document { .. })
    ));

    let mouse =
        serde_json::to_vec(&MouseConfig::default_for("OTHER", MouseModel::MxMaster4)).unwrap();
    let wrong_serial = zip_of(&[
        ("manifest.json", &manifest_json(&["mice/MX4-0042"], 2)),
        ("config/mice/MX4-0042.json", &mouse),
    ]);
    assert!(matches!(
        read_backup(&wrong_serial),
        Err(BackupError::Refused { .. })
    ));
}

#[test]
fn images_of_a_device_the_backup_does_not_hold_are_refused() {
    let bytes = zip_of(&[
        ("manifest.json", &manifest_json(&[], 2)),
        ("images/GHOST/logo.png", b"\x89PNG\r\n\x1a\n"),
    ]);
    assert!(matches!(
        read_backup(&bytes),
        Err(BackupError::OrphanImages)
    ));
}

#[test]
fn an_oversized_document_is_refused_whatever_it_declares() {
    let big = vec![b' '; MAX_ARCHIVE_CONFIG_BYTES + 1];
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("manifest.json", deflated).unwrap();
    zip.write_all(&manifest_json(&["ssh"], 2)).unwrap();
    zip.start_file("config/ssh.json", deflated).unwrap();
    zip.write_all(&big).unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    assert!(matches!(
        read_backup(&bytes),
        Err(BackupError::Archive(ArchiveError::EntryTooLarge(_)))
    ));
}

#[test]
fn too_many_entries_are_refused() {
    let mut entries: Vec<(String, Vec<u8>)> = vec![("manifest.json".into(), manifest_json(&[], 2))];
    entries.extend((0..MAX_BACKUP_ENTRIES).map(|i| (format!("images/GHOST/{i}.png"), vec![0])));
    let borrowed: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    assert!(matches!(
        read_backup(&zip_of(&borrowed)),
        Err(BackupError::Archive(ArchiveError::TooManyEntries))
    ));
}

#[test]
fn something_that_is_not_a_zip_is_refused() {
    assert!(read_backup(b"not an archive").is_err());
    assert!(read_backup(&[]).is_err());
}

#[test]
fn the_preview_says_what_would_change() {
    let backup = full_backup();
    let mut current = backup.documents.clone();
    // Same content, other revision: unchanged.
    if let Some(BackupDocument::Ssh(ssh)) = current.get_mut(&BackupSection::Ssh) {
        ssh.revision = 41;
    }
    // Other content: replaced.
    if let Some(BackupDocument::Machine(machine)) = current.get_mut(&BackupSection::Machine) {
        machine.aliases.push(ShellAlias {
            name: "gs".into(),
            command: "git status".into(),
            enabled: true,
        });
    }
    // Missing here: added.
    current.remove(&BackupSection::Git);
    // Here only: kept.
    current.insert(
        BackupSection::Mouse("OTHER".into()),
        BackupDocument::Mouse(Box::new(MouseConfig::default_for(
            "OTHER",
            MouseModel::MxMaster4,
        ))),
    );

    let preview = preview(&backup, &current);
    let change = |section: &BackupSection| {
        preview
            .sections
            .iter()
            .find(|s| s.section == section.id())
            .unwrap()
            .change
    };
    assert_eq!(change(&BackupSection::Ssh), SectionChange::Same);
    assert_eq!(change(&BackupSection::Machine), SectionChange::Replaced);
    assert_eq!(change(&BackupSection::Git), SectionChange::Added);
    assert_eq!(preview.kept, vec!["mice/OTHER".to_owned()]);
}

/// A backup whose documents hold what runs on a computer, enabled or not.
fn risky_backup() -> Backup {
    let mut backup = full_backup();
    backup.documents.insert(
        BackupSection::Machine,
        BackupDocument::Machine(MachineConfig {
            aliases: vec![
                ShellAlias {
                    name: "gs".into(),
                    command: "git status".into(),
                    enabled: true,
                },
                ShellAlias {
                    name: "off".into(),
                    command: "rm -rf /".into(),
                    enabled: false,
                },
            ],
            ..MachineConfig::empty()
        }),
    );
    backup.documents.insert(
        BackupSection::Git,
        BackupDocument::Git(Box::new(GitConfig {
            aliases: vec![GitAlias {
                name: "co".into(),
                command: "checkout".into(),
                enabled: true,
            }],
            extra_settings: vec![
                AdvancedSetting {
                    key: "core.fsmonitor".into(),
                    value: "/tmp/x.sh".into(),
                    enabled: true,
                },
                AdvancedSetting {
                    key: "credential.https://example.org.helper".into(),
                    value: "!curl x".into(),
                    enabled: false,
                },
            ],
            ..GitConfig::empty()
        })),
    );
    let option = |key: &str, value: &str, enabled| AdvancedSetting {
        key: key.into(),
        value: value.into(),
        enabled,
    };
    backup.documents.insert(
        BackupSection::Ssh,
        BackupDocument::Ssh(SshConfig {
            hosts: vec![SshHost {
                alias: "prod".into(),
                host_name: "evil.example.org".into(),
                forward_agent: true,
                proxy_jump: "bastion".into(),
                options: vec![
                    option("ProxyCommand", "nc %h %p", true),
                    option("LocalCommand", "x", false),
                ],
                enabled: true,
                ..sample_host()
            }],
            common_options: vec![option("ServerAliveInterval", "60", true)],
            ..SshConfig::empty()
        }),
    );
    backup
}

/// What the preview of `backup` lists for `section`.
fn executables_of(backup: &Backup, section: &BackupSection) -> Vec<ExecutableItem> {
    preview(backup, &BTreeMap::new())
        .sections
        .into_iter()
        .find(|s| s.section == section.id())
        .unwrap()
        .executables
}

#[test]
fn the_preview_lists_the_shell_aliases_enabled_or_not() {
    let of = |section: &BackupSection| executables_of(&risky_backup(), section);
    let machine = of(&BackupSection::Machine);
    assert_eq!(machine.len(), 2, "a disabled alias is listed too");
    assert_eq!(machine[0].kind, ExecutableKind::ShellAlias);
    assert_eq!(machine[0].detail, "git status");
    assert!(machine[0].enabled);
    assert!(!machine[1].enabled);
}

#[test]
fn the_preview_lists_the_aliases_and_free_settings_of_git() {
    let of = |section: &BackupSection| executables_of(&risky_backup(), section);
    let git = of(&BackupSection::Git);
    assert_eq!(git[0].kind, ExecutableKind::GitAlias);
    let fsmonitor = git
        .iter()
        .find(|item| item.name == "core.fsmonitor")
        .expect("a free setting of Git runs code without any alias");
    assert_eq!(fsmonitor.kind, ExecutableKind::GitSetting);
    assert!(fsmonitor.enabled);
    assert!(
        git.iter()
            .any(|item| item.name.starts_with("credential.") && !item.enabled)
    );
}

#[test]
fn the_preview_lists_the_hosts_and_options_of_ssh() {
    let of = |section: &BackupSection| executables_of(&risky_backup(), section);
    let ssh = of(&BackupSection::Ssh);
    let host = ssh
        .iter()
        .find(|item| item.kind == ExecutableKind::SshHost)
        .expect("a host that names a machine and forwards the agent is listed");
    assert_eq!(host.name, "prod");
    assert!(host.detail.contains("evil.example.org"));
    assert!(host.detail.contains("via bastion"));
    assert!(host.detail.contains("agent forwarded"));
    assert!(
        ssh.iter()
            .any(|i| i.name == "prod: ProxyCommand" && i.detail == "nc %h %p" && i.enabled)
    );
    assert!(
        ssh.iter()
            .any(|i| i.name == "prod: LocalCommand" && !i.enabled)
    );
    assert!(ssh.iter().any(|i| i.name == "*: ServerAliveInterval"));
}

#[test]
fn the_preview_lists_nothing_for_what_runs_nothing() {
    assert_eq!(
        executables_of(&risky_backup(), &BackupSection::RadialMenu),
        Vec::new()
    );
}

fn sample_host() -> SshHost {
    SshHost {
        alias: "h".into(),
        host_name: String::new(),
        user: String::new(),
        port: None,
        identity_file: String::new(),
        proxy_jump: String::new(),
        forward_agent: false,
        options: Vec::new(),
        enabled: true,
    }
}

#[test]
fn a_version_that_is_not_one_is_refused() {
    for version in ["", "1.0\u{1b}[31m", "a b", &"9".repeat(65)] {
        let manifest = serde_json::to_vec(&BackupManifest {
            format: 2,
            app_version: version.to_owned(),
            created_at: 1,
            sections: Vec::new(),
        })
        .unwrap();
        let bytes = zip_of(&[("manifest.json", &manifest)]);
        assert!(
            matches!(read_backup(&bytes), Err(BackupError::InvalidVersion)),
            "{version:?}"
        );
    }
}

#[test]
fn the_same_document_twice_is_refused_through_a_backup() {
    // The zip writer refuses duplicate names: rename one in place, as the archive tests do.
    let ssh = serde_json::to_vec(&SshConfig::empty()).unwrap();
    let (name, renamed) = ("config/ssh.json", "config/ssh.jsox");
    let mut bytes = zip_of(&[
        ("manifest.json", &manifest_json(&["ssh"], 2)),
        (name, &ssh),
        (renamed, &ssh),
    ]);
    let mut patched = 0;
    let mut start = 0;
    while let Some(found) = bytes[start..]
        .windows(renamed.len())
        .position(|window| window == renamed.as_bytes())
    {
        let at = start + found;
        bytes[at..at + renamed.len()].copy_from_slice(name.as_bytes());
        patched += 1;
        start = at + renamed.len();
    }
    assert_eq!(patched, 2, "local header and central directory");
    assert!(matches!(
        read_backup(&bytes),
        Err(BackupError::Archive(ArchiveError::DuplicateEntry))
    ));
}

#[test]
fn the_whole_expanded_size_is_bounded_through_a_backup() {
    // Each image is under its own limit, all of them together are not.
    let image = vec![b'x'; MAX_SOURCE_BYTES];
    let count = MAX_ARCHIVE_EXPANDED_BYTES / MAX_SOURCE_BYTES + 2;
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("manifest.json", deflated).unwrap();
    zip.write_all(&manifest_json(&[], 2)).unwrap();
    for index in 0..count {
        zip.start_file(format!("images/GHOST/{index}.png"), deflated)
            .unwrap();
        zip.write_all(&image).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    assert!(bytes.len() < MAX_ARCHIVE_BYTES, "it compresses well");
    assert!(matches!(
        read_backup(&bytes),
        Err(BackupError::Archive(ArchiveError::EntryTooLarge(_)))
    ));
}

#[test]
fn an_ambiguous_end_of_archive_is_refused_through_a_backup() {
    let mut bytes = write_backup(&full_backup()).unwrap();
    // Anything after the comment of the record leaves two ways to read the end.
    bytes.extend_from_slice(b"trailing bytes");
    assert!(matches!(
        read_backup(&bytes),
        Err(BackupError::Archive(
            ArchiveError::AmbiguousEnd | ArchiveError::NoEnd
        ))
    ));
}

#[test]
fn a_selection_keeps_the_chosen_sections_and_the_images_of_the_chosen_devices() {
    let backup = full_backup();
    let device = BackupSection::Device(DEMO_SERIAL.to_owned());

    let with_device = backup.select(&BTreeSet::from([device.clone(), BackupSection::Ssh]));
    assert_eq!(
        with_device.manifest.sections,
        vec![device.id(), "ssh".to_owned()]
    );
    assert_eq!(with_device.images.len(), 1);
    assert_eq!(with_device.manifest.app_version, "1.4.2");
    assert_eq!(with_device.manifest.created_at, 1_700_000_000);

    // Images are not carried by other sections.
    let without_device = backup.select(&BTreeSet::from([BackupSection::Git]));
    assert_eq!(without_device.manifest.sections, vec!["git".to_owned()]);
    assert_eq!(without_device.images.len(), 0);

    // What is selected reads back as itself.
    let archive = write_backup(&with_device).unwrap();
    assert_eq!(read_backup(&archive).unwrap(), with_device);
}

#[test]
fn an_old_export_that_holds_nothing_acceptable_says_why() {
    // Not a configuration at all: the error of reading it as one, not "unexpected entry".
    let garbage = zip_of(&[("config.json", b"not json")]);
    assert!(matches!(
        read_backup_or_archive(&garbage),
        Err(BackupError::Archive(ArchiveError::Config(_)))
    ));
    // A configuration the device refuses.
    let mut invalid = device_config();
    invalid.brightness = 250;
    let archive = zip_of(&[("config.json", &serde_json::to_vec(&invalid).unwrap())]);
    assert!(matches!(
        read_backup_or_archive(&archive),
        Err(BackupError::Refused { .. })
    ));
    // A zip that is neither keeps the error of reading a backup.
    let other = zip_of(&[("other.txt", b"x")]);
    assert!(matches!(
        read_backup_or_archive(&other),
        Err(BackupError::Archive(ArchiveError::UnexpectedEntry(_)))
    ));
}

#[test]
fn an_old_export_reads_as_the_backup_of_one_device_with_its_images() {
    let config = device_config();
    let archive = crate::config_store::write_archive(
        &config,
        &ArchiveImages::from([("logo.png".to_owned(), b"\x89PNG\r\n\x1a\nrest".to_vec())]),
    )
    .unwrap();
    let backup = read_backup_or_archive(&archive).unwrap();
    let section = BackupSection::Device(config.serial.clone());
    assert_eq!(backup.documents.keys().collect::<Vec<_>>(), vec![&section]);
    assert_eq!(backup.images[&config.serial].len(), 1);
    // A real backup is read as itself.
    let full = write_backup(&full_backup()).unwrap();
    assert_eq!(read_backup_or_archive(&full).unwrap(), full_backup());
}
