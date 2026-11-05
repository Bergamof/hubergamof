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
    /// Several end of central directory records could be the archive's
    /// (one hidden in the comment), or a ZIP64 one: which entries would be
    /// read is unclear, never needed by an export.
    #[error("ambiguous or ZIP64 end of archive")]
    AmbiguousEnd,
    /// No end of central directory record where an export puts it: not a
    /// zip, or one the zip reader would look for further back.
    #[error("no end of archive")]
    NoEnd,
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
/// Offset of its comment length.
const COMMENT_LENGTH_OFFSET: usize = 20;
/// Longest comment, hence how far from the end the record may start.
const MAX_COMMENT_LEN: usize = u16::MAX as usize;
/// How much further back than the longest comment the records are looked
/// for: the zip reader (9.3) starts its own search a few bytes earlier,
/// and none of the records it could pick may escape the check.
const SEARCH_MARGIN: usize = 64;
/// Signature of the ZIP64 end of central directory locator, right before
/// the record.
const ZIP64_LOCATOR: [u8; 4] = [0x50, 0x4b, 0x06, 0x07];
/// Size of that locator.
const ZIP64_LOCATOR_LEN: usize = 20;

/// A little-endian `u16` of `bytes` at `offset`.
fn u16_at(bytes: &[u8], offset: usize) -> Option<usize> {
    let field = bytes.get(offset..offset + 2)?;
    Some(usize::from(u16::from_le_bytes([field[0], field[1]])))
}

/// Entry count the archive declares, read from its end of central
/// directory record before parsing the directory itself (which allocates
/// for every entry).
///
/// Like the zip reader, a record is any signature whose comment fits in
/// the archive. Exactly one is accepted, its comment running to the end:
/// another one (hidden in the comment), bytes after the comment or a ZIP64
/// record could make the zip reader use another count, and are refused.
fn declared_entry_count(bytes: &[u8]) -> Result<usize, ArchiveError> {
    let last_start = bytes
        .len()
        .checked_sub(END_OF_CENTRAL_DIRECTORY_LEN)
        .ok_or(ArchiveError::NoEnd)?;
    let first_start = last_start.saturating_sub(MAX_COMMENT_LEN + SEARCH_MARGIN);
    let mut records = (first_start..=last_start).filter_map(|start| {
        let end =
            start + END_OF_CENTRAL_DIRECTORY_LEN + u16_at(bytes, start + COMMENT_LENGTH_OFFSET)?;
        (bytes[start..].starts_with(&END_OF_CENTRAL_DIRECTORY) && end <= bytes.len())
            .then_some((start, end))
    });
    let (start, end) = records.next().ok_or(ArchiveError::NoEnd)?;
    let zip64 = start
        .checked_sub(ZIP64_LOCATOR_LEN)
        .is_some_and(|locator| bytes[locator..].starts_with(&ZIP64_LOCATOR));
    if records.next().is_some() || end != bytes.len() || zip64 {
        return Err(ArchiveError::AmbiguousEnd);
    }
    u16_at(bytes, start + ENTRY_COUNT_OFFSET).ok_or(ArchiveError::NoEnd)
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
    let declared = declared_entry_count(bytes)?;
    if declared > MAX_ARCHIVE_ENTRIES {
        return Err(ArchiveError::TooManyEntries);
    }
    let mut zip = ZipArchive::new(Cursor::new(bytes))?;
    if zip.len() > MAX_ARCHIVE_ENTRIES {
        return Err(ArchiveError::TooManyEntries);
    }
    // The zip reader keeps one entry per name: fewer entries than declared
    // means a name appears twice, and which one would be read is ambiguous.
    if declared != zip.len() {
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
            Err(ArchiveError::NoEnd)
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

    /// `archive` with `comment` appended, its end record saying so.
    fn with_comment(mut archive: Vec<u8>, comment: &[u8]) -> Vec<u8> {
        let length_at = archive.len() - 2;
        let length = u16::try_from(comment.len()).unwrap().to_le_bytes();
        archive[length_at..].copy_from_slice(&length);
        archive.extend_from_slice(comment);
        archive
    }

    #[test]
    fn an_end_record_hidden_in_the_comment_is_refused() {
        let archive = zip_with(&[(ARCHIVE_CONFIG_ENTRY, b"{}")]);
        // A plain comment is fine (read further on, refused as a config).
        let commented = with_comment(archive.clone(), b"exported");
        assert!(!matches!(
            read_archive(&commented),
            Err(ArchiveError::AmbiguousEnd)
        ));

        // A forged record declaring 3 entries, its empty comment running
        // to the end, like the real one.
        let mut forged = END_OF_CENTRAL_DIRECTORY.to_vec();
        forged.extend_from_slice(&[0, 0, 0, 0, 3, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(forged.len(), END_OF_CENTRAL_DIRECTORY_LEN);
        assert!(matches!(
            read_archive(&with_comment(archive.clone(), &forged)),
            Err(ArchiveError::AmbiguousEnd)
        ));

        // The same record followed by bytes the comment does not cover.
        let mut trailing = with_comment(archive, &forged);
        trailing.push(0);
        assert!(matches!(
            read_archive(&trailing),
            Err(ArchiveError::AmbiguousEnd)
        ));
    }

    #[test]
    fn an_end_record_the_check_does_not_reach_is_refused() {
        // The only record, before the window a full comment allows, its
        // comment ending 3 bytes before the end: the zip reader would take it.
        let mut archive = END_OF_CENTRAL_DIRECTORY.to_vec();
        archive.extend_from_slice(&[0; 16]);
        archive.extend_from_slice(&u16::MAX.to_le_bytes());
        archive.resize(archive.len() + MAX_COMMENT_LEN + 3, 0);
        assert!(matches!(
            read_archive(&archive),
            Err(ArchiveError::AmbiguousEnd)
        ));
    }

    #[test]
    fn bytes_after_the_end_record_are_refused() {
        let mut archive = zip_with(&[(ARCHIVE_CONFIG_ENTRY, b"{}")]);
        archive.push(0);
        assert!(matches!(
            read_archive(&archive),
            Err(ArchiveError::AmbiguousEnd)
        ));
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
