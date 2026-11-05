//! Export archives of a device configuration: a zip holding the
//! pretty-printed `config.json` and the images it uses under `images/`, so
//! that both stay readable and editable with ordinary tools.
//!
//! Archives come from the user, possibly from elsewhere: reading accepts
//! nothing but those entries (no path leaving the archive, no other file),
//! and bounds the number of entries and every decompressed size as it reads,
//! without trusting the sizes the archive declares.

use std::collections::BTreeMap;
use std::io::{self, Cursor, Read, Write};

use bergadeck_protocol::{DeviceConfig, is_safe_image_name};
use thiserror::Error;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::render::{ImageKind, MAX_SOURCE_BYTES};

/// Entry of the configuration.
pub const ARCHIVE_CONFIG_ENTRY: &str = "config.json";

/// Directory entry prefix of the images.
pub const ARCHIVE_IMAGES_PREFIX: &str = "images/";

/// Largest archive accepted.
pub const MAX_ARCHIVE_BYTES: usize = 32 * 1024 * 1024;

/// Most entries an archive may hold (directories included).
pub const MAX_ARCHIVE_ENTRIES: usize = 256;

/// Largest decompressed `config.json`.
pub const MAX_ARCHIVE_CONFIG_BYTES: usize = 1024 * 1024;

/// Largest decompressed content of a whole archive.
pub const MAX_ARCHIVE_EXPANDED_BYTES: usize = 64 * 1024 * 1024;

/// Images by file name.
pub type ArchiveImages = BTreeMap<String, Vec<u8>>;

/// Content of an export archive.
#[derive(Debug, Clone, PartialEq)]
pub struct Archive {
    /// Exported configuration, as written in the archive.
    pub config: DeviceConfig,
    /// Images of the archive, by file name (not checked as images yet).
    pub images: ArchiveImages,
}

/// Why an archive cannot be read or written.
#[derive(Debug, Error)]
pub enum ArchiveError {
    /// Larger than [`MAX_ARCHIVE_BYTES`].
    #[error("archive is larger than {MAX_ARCHIVE_BYTES} bytes")]
    TooLarge,
    /// More than [`MAX_ARCHIVE_ENTRIES`] entries.
    #[error("archive has more than {MAX_ARCHIVE_ENTRIES} entries")]
    TooManyEntries,
    /// An entry, or all of them, decompress beyond their limit.
    #[error("archive entry `{0}` is too large once decompressed")]
    EntryTooLarge(String),
    /// An entry that is neither the configuration nor an image.
    #[error("unexpected archive entry `{0}`")]
    UnexpectedEntry(String),
    /// The same entry twice.
    #[error("an archive entry appears twice")]
    DuplicateEntry,
    /// No `config.json`.
    #[error("archive has no {ARCHIVE_CONFIG_ENTRY}")]
    MissingConfig,
    /// `config.json` is not a configuration.
    #[error("invalid {ARCHIVE_CONFIG_ENTRY}: {0}")]
    Config(#[source] serde_json::Error),
    /// Not a readable zip (or an unsupported feature: encryption, another
    /// compression method…).
    #[error("not a readable zip archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// Reading or writing an entry failed.
    #[error("archive entry: {0}")]
    Io(#[from] io::Error),
}

/// Writes the archive of `config` and its `images`.
///
/// # Errors
///
/// [`ArchiveError::Zip`] or [`ArchiveError::Io`] (not expected in memory).
pub fn write_archive(
    config: &DeviceConfig,
    images: &ArchiveImages,
) -> Result<Vec<u8>, ArchiveError> {
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    // PNG and JPEG are compressed already: deflating them costs time for nothing.
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(ARCHIVE_CONFIG_ENTRY, deflated)?;
    let json = serde_json::to_vec_pretty(config).map_err(io::Error::from)?;
    zip.write_all(&json)?;
    for (name, bytes) in images {
        let options = match ImageKind::sniff(bytes) {
            Some(ImageKind::Png | ImageKind::Jpeg) => stored,
            Some(ImageKind::Svg) | None => deflated,
        };
        zip.start_file(format!("{ARCHIVE_IMAGES_PREFIX}{name}"), options)?;
        zip.write_all(bytes)?;
    }
    Ok(zip.finish()?.into_inner())
}

/// Signature of the end of central directory record.
const END_OF_CENTRAL_DIRECTORY: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
/// Size of that record without its trailing comment.
const END_OF_CENTRAL_DIRECTORY_LEN: usize = 22;
/// Offset of its total entry count.
const ENTRY_COUNT_OFFSET: usize = 10;

/// Entry count the archive declares, read from its last end of central
/// directory record before parsing the directory itself (which allocates
/// for every entry). `None` when there is no such record.
fn declared_entry_count(bytes: &[u8]) -> Option<usize> {
    let last_start = bytes.len().checked_sub(END_OF_CENTRAL_DIRECTORY_LEN)?;
    let start = (0..=last_start)
        .rev()
        .find(|&start| bytes[start..].starts_with(&END_OF_CENTRAL_DIRECTORY))?;
    let count = bytes.get(start + ENTRY_COUNT_OFFSET..start + ENTRY_COUNT_OFFSET + 2)?;
    Some(usize::from(u16::from_le_bytes([count[0], count[1]])))
}

/// What an entry of an archive holds.
enum Entry {
    Config,
    Image(String),
    Directory,
}

fn classify(name: &str) -> Result<Entry, ArchiveError> {
    if name == ARCHIVE_CONFIG_ENTRY {
        return Ok(Entry::Config);
    }
    match name.strip_prefix(ARCHIVE_IMAGES_PREFIX) {
        Some("") => Ok(Entry::Directory),
        Some(image) if is_safe_image_name(image) => Ok(Entry::Image(image.to_owned())),
        _ => Err(ArchiveError::UnexpectedEntry(name.to_owned())),
    }
}

/// Reads at most `limit` bytes of `entry`, failing beyond.
fn read_bounded(entry: impl Read, name: &str, limit: usize) -> Result<Vec<u8>, ArchiveError> {
    let mut bytes = Vec::new();
    let cap = u64::try_from(limit).map_or(u64::MAX, |limit| limit + 1);
    entry.take(cap).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(ArchiveError::EntryTooLarge(name.to_owned()));
    }
    Ok(bytes)
}

/// Reads an export archive.
///
/// # Errors
///
/// Any [`ArchiveError`]: the archive is refused as a whole.
pub fn read_archive(bytes: &[u8]) -> Result<Archive, ArchiveError> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(ArchiveError::TooLarge);
    }
    // 0xFFFF (ZIP64, whose real count lies elsewhere) is refused too.
    let declared = declared_entry_count(bytes);
    if declared.is_some_and(|count| count > MAX_ARCHIVE_ENTRIES) {
        return Err(ArchiveError::TooManyEntries);
    }
    let mut zip = ZipArchive::new(Cursor::new(bytes))?;
    if zip.len() > MAX_ARCHIVE_ENTRIES {
        return Err(ArchiveError::TooManyEntries);
    }
    // The zip reader keeps one entry per name: fewer entries than declared
    // means a name appears twice, and which one would be read is ambiguous.
    if declared.is_some_and(|count| count != zip.len()) {
        return Err(ArchiveError::DuplicateEntry);
    }
    let mut config = None;
    let mut images = ArchiveImages::new();
    let mut expanded = 0_usize;
    for index in 0..zip.len() {
        let entry = zip.by_index(index)?;
        // The raw name, not a sanitised one: anything unexpected is refused.
        let name = entry.name()?.into_owned();
        let (kind, limit) = match classify(&name)? {
            Entry::Directory => continue,
            Entry::Config => (Entry::Config, MAX_ARCHIVE_CONFIG_BYTES),
            image @ Entry::Image(_) => (image, MAX_SOURCE_BYTES),
        };
        let remaining = MAX_ARCHIVE_EXPANDED_BYTES - expanded;
        let content = read_bounded(entry, &name, limit.min(remaining))?;
        expanded += content.len();
        match kind {
            Entry::Config if config.is_none() => {
                config = Some(serde_json::from_slice(&content).map_err(ArchiveError::Config)?);
            }
            Entry::Image(image) if !images.contains_key(&image) => {
                images.insert(image, content);
            }
            _ => return Err(ArchiveError::DuplicateEntry),
        }
    }
    Ok(Archive {
        config: config.ok_or(ArchiveError::MissingConfig)?,
        images,
    })
}

#[cfg(test)]
mod tests {
    use bergadeck_protocol::DeviceModel;

    use super::*;

    fn config() -> DeviceConfig {
        DeviceConfig {
            schema_version: 1,
            serial: "ABC".into(),
            model: DeviceModel::Plus,
            brightness: 40,
            home: Vec::new(),
            modules: Vec::new(),
            dials: vec![None; 4],
            revision: 3,
        }
    }

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(content).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn config_json() -> Vec<u8> {
        serde_json::to_vec(&config()).unwrap()
    }

    #[test]
    fn archives_round_trip_with_a_readable_config() {
        let images = ArchiveImages::from([("a.png".to_owned(), vec![1, 2, 3])]);
        let bytes = write_archive(&config(), &images).unwrap();
        assert_eq!(
            read_archive(&bytes).unwrap(),
            Archive {
                config: config(),
                images
            }
        );
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut text = String::new();
        zip.by_name(ARCHIVE_CONFIG_ENTRY)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert!(text.contains("\n  \"brightness\": 40,"), "pretty-printed");
    }

    #[test]
    fn entries_other_than_the_config_and_plain_image_names_are_refused() {
        for name in [
            "../config.json",
            "/etc/passwd",
            "notes.txt",
            "images/../config.json",
            "images/.hidden",
            "images/sub/a.png",
            "Config.json",
        ] {
            let archive = zip_with(&[(ARCHIVE_CONFIG_ENTRY, &config_json()), (name, b"x")]);
            assert!(
                matches!(read_archive(&archive), Err(ArchiveError::UnexpectedEntry(entry)) if entry == name),
                "{name}"
            );
        }
        let with_directory = zip_with(&[(ARCHIVE_CONFIG_ENTRY, &config_json()), ("images/", b"")]);
        assert!(read_archive(&with_directory).is_ok());
    }

    #[test]
    fn missing_or_invalid_configs_are_refused() {
        assert!(matches!(
            read_archive(&zip_with(&[("images/a.png", b"x")])),
            Err(ArchiveError::MissingConfig)
        ));
        assert!(matches!(
            read_archive(&zip_with(&[(ARCHIVE_CONFIG_ENTRY, b"{}")])),
            Err(ArchiveError::Config(_))
        ));
        assert!(matches!(
            read_archive(b"not a zip"),
            Err(ArchiveError::Zip(_))
        ));
    }

    #[test]
    fn decompression_is_bounded_whatever_the_archive_declares() {
        // Zeros compress about a thousand times: a small archive, a large image.
        let bomb = vec![0; MAX_SOURCE_BYTES + 1];
        let archive = zip_with(&[
            (ARCHIVE_CONFIG_ENTRY, &config_json()),
            ("images/a.png", &bomb),
        ]);
        assert!(archive.len() < MAX_SOURCE_BYTES / 100);
        assert!(matches!(
            read_archive(&archive),
            Err(ArchiveError::EntryTooLarge(name)) if name == "images/a.png"
        ));
        let big_config = vec![b' '; MAX_ARCHIVE_CONFIG_BYTES + 1];
        assert!(matches!(
            read_archive(&zip_with(&[(ARCHIVE_CONFIG_ENTRY, &big_config)])),
            Err(ArchiveError::EntryTooLarge(_))
        ));
    }

    #[test]
    fn the_whole_archive_is_bounded_once_decompressed() {
        let image = vec![0; MAX_SOURCE_BYTES];
        let names: Vec<String> = (0..=MAX_ARCHIVE_EXPANDED_BYTES / MAX_SOURCE_BYTES)
            .map(|index| format!("{ARCHIVE_IMAGES_PREFIX}{index}.png"))
            .collect();
        let config = config_json();
        let mut entries: Vec<(&str, &[u8])> = vec![(ARCHIVE_CONFIG_ENTRY, &config)];
        entries.extend(names.iter().map(|name| (name.as_str(), image.as_slice())));
        let archive = zip_with(&entries);
        assert!(archive.len() < MAX_ARCHIVE_BYTES, "a small archive");
        assert!(matches!(
            read_archive(&archive),
            Err(ArchiveError::EntryTooLarge(_))
        ));
    }

    #[test]
    fn an_entry_present_twice_is_refused() {
        // The zip writer refuses duplicate names: rename one in place.
        let renamed = "config.jsox";
        let mut archive = zip_with(&[
            (ARCHIVE_CONFIG_ENTRY, &config_json()),
            (renamed, &config_json()),
        ]);
        let mut patched = 0;
        let mut start = 0;
        while let Some(found) = archive[start..]
            .windows(renamed.len())
            .position(|window| window == renamed.as_bytes())
        {
            let at = start + found;
            archive[at..at + renamed.len()].copy_from_slice(ARCHIVE_CONFIG_ENTRY.as_bytes());
            patched += 1;
            start = at + renamed.len();
        }
        assert_eq!(patched, 2, "local header and central directory");
        assert!(matches!(
            read_archive(&archive),
            Err(ArchiveError::DuplicateEntry)
        ));
    }

    #[test]
    fn images_already_compressed_are_stored_as_they_are() {
        let png = b"\x89PNG\r\n\x1a\n-pixels".to_vec();
        let images = ArchiveImages::from([("a.png".to_owned(), png)]);
        let bytes = write_archive(&config(), &images).unwrap();
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(
            zip.by_name("images/a.png").unwrap().compression(),
            CompressionMethod::Stored
        );
        assert_eq!(
            zip.by_name(ARCHIVE_CONFIG_ENTRY).unwrap().compression(),
            CompressionMethod::Deflated
        );
    }

    #[test]
    fn archives_too_large_or_with_too_many_entries_are_refused() {
        assert!(matches!(
            read_archive(&vec![0; MAX_ARCHIVE_BYTES + 1]),
            Err(ArchiveError::TooLarge)
        ));
        let names: Vec<String> = (0..=MAX_ARCHIVE_ENTRIES)
            .map(|index| format!("images/{index}.png"))
            .collect();
        let entries: Vec<(&str, &[u8])> = names
            .iter()
            .map(|name| (name.as_str(), &b"x"[..]))
            .collect();
        assert!(matches!(
            read_archive(&zip_with(&entries)),
            Err(ArchiveError::TooManyEntries)
        ));
    }
}
