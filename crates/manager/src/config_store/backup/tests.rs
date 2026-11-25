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
            .find(|s| &s.section == section)
            .unwrap()
            .change
    };
    assert_eq!(change(&BackupSection::Ssh), SectionChange::Same);
    assert_eq!(change(&BackupSection::Machine), SectionChange::Replaced);
    assert_eq!(change(&BackupSection::Git), SectionChange::Added);
    assert_eq!(preview.kept, vec![BackupSection::Mouse("OTHER".into())]);
}

#[test]
fn the_preview_lists_what_would_be_run() {
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
    let preview = preview(&backup, &BTreeMap::new());
    let of = |section: &BackupSection| {
        preview
            .sections
            .iter()
            .find(|s| &s.section == section)
            .unwrap()
            .executables
            .clone()
    };
    let machine = of(&BackupSection::Machine);
    assert_eq!(machine.len(), 1, "a disabled alias is not run");
    assert_eq!(machine[0].kind, ExecutableKind::ShellAlias);
    assert_eq!(machine[0].detail, "git status");
    assert_eq!(of(&BackupSection::Git)[0].kind, ExecutableKind::GitAlias);
    let ssh = of(&BackupSection::Ssh);
    assert_eq!(ssh.len(), 2);
    assert!(
        ssh.iter()
            .any(|i| i.name == "prod: ProxyCommand" && i.detail == "nc %h %p")
    );
    assert!(ssh.iter().any(|i| i.name == "*: ServerAliveInterval"));
    assert_eq!(of(&BackupSection::RadialMenu), Vec::new());
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
