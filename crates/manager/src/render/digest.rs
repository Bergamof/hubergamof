//! Content digests of image sources, remembered so that a render cache hit
//! costs neither a file read nor a hash.

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::SystemTime;

use lru::LruCache;
use sha2::{Digest, Sha256};

use super::RenderError;
use super::source::{ImageSource, check_source_length};

/// SHA-256 of an image content.
pub(super) type ContentDigest = [u8; 32];

/// Digest of a source, with its content when it had to be read for it.
pub(super) struct SourceDigest {
    pub(super) digest: ContentDigest,
    pub(super) bytes: Option<Arc<[u8]>>,
}

/// Identity of a source whose content cannot have changed since its digest
/// was computed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum SourceId {
    /// Address of a live shared buffer (immutable by construction).
    Bytes(usize),
    /// File with this modification time and length. Stored images are
    /// content-addressed and never rewritten in place.
    File {
        path: PathBuf,
        modified: SystemTime,
        len: u64,
    },
}

struct Remembered {
    digest: ContentDigest,
    /// Keeps the allocation of a [`SourceId::Bytes`] reserved, so that its
    /// address cannot designate another buffer while the entry exists.
    _buffer: Option<Weak<[u8]>>,
}

/// Bounded memory of source digests.
pub(super) struct DigestMemo {
    entries: Mutex<LruCache<SourceId, Remembered>>,
}

impl DigestMemo {
    pub(super) fn new(capacity: NonZeroUsize) -> Self {
        Self {
            entries: Mutex::new(LruCache::new(capacity)),
        }
    }

    /// Digest of `source`; its content is read only when the digest is not
    /// remembered.
    ///
    /// # Errors
    ///
    /// [`RenderError::SourceTooLarge`] or [`RenderError::Io`].
    pub(super) fn digest(&self, source: &ImageSource) -> Result<SourceDigest, RenderError> {
        let (id, buffer) = match source {
            ImageSource::Bytes(bytes) => {
                check_source_length(bytes.len())?;
                let id = SourceId::Bytes(Arc::as_ptr(bytes).cast::<u8>().addr());
                (Some(id), Some(Arc::downgrade(bytes)))
            }
            ImageSource::File(path) => (file_id(path), None),
        };
        if let Some(remembered) = id
            .as_ref()
            .and_then(|id| self.lock().get(id).map(|r| r.digest))
        {
            return Ok(SourceDigest {
                digest: remembered,
                bytes: None,
            });
        }
        let bytes = source.read()?;
        let digest: ContentDigest = Sha256::digest(&bytes).into();
        if let Some(id) = id {
            self.lock().put(
                id,
                Remembered {
                    digest,
                    _buffer: buffer,
                },
            );
        }
        Ok(SourceDigest {
            digest,
            bytes: Some(bytes),
        })
    }

    fn lock(&self) -> MutexGuard<'_, LruCache<SourceId, Remembered>> {
        // The cache is never left half-updated: a poisoned lock is still valid.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Identity of a file, `None` when its metadata is unavailable (the read
/// that follows reports the error).
fn file_id(path: &std::path::Path) -> Option<SourceId> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(SourceId::File {
        path: path.to_owned(),
        modified: metadata.modified().ok()?,
        len: metadata.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPACITY: NonZeroUsize = NonZeroUsize::MIN.saturating_add(7);

    #[test]
    fn shared_buffers_are_hashed_once() {
        let memo = DigestMemo::new(CAPACITY);
        let bytes: Arc<[u8]> = Arc::from(&b"image"[..]);
        let source = ImageSource::Bytes(Arc::clone(&bytes));
        let first = memo.digest(&source).unwrap();
        assert!(first.bytes.is_some());
        let second = memo.digest(&source).unwrap();
        assert!(second.bytes.is_none());
        assert_eq!(first.digest, second.digest);
        // Same content in another buffer: same digest, computed again.
        let copy = memo
            .digest(&ImageSource::Bytes(Arc::from(&b"image"[..])))
            .unwrap();
        assert!(copy.bytes.is_some());
        assert_eq!(copy.digest, first.digest);
    }

    #[test]
    fn files_are_read_again_when_they_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("icon.png");
        std::fs::write(&path, b"first").unwrap();
        let memo = DigestMemo::new(CAPACITY);
        let source = ImageSource::File(path.clone());
        let first = memo.digest(&source).unwrap();
        assert!(memo.digest(&source).unwrap().bytes.is_none());

        std::fs::write(&path, b"second, longer").unwrap();
        let second = memo.digest(&source).unwrap();
        assert_eq!(second.bytes.as_deref(), Some(&b"second, longer"[..]));
        assert_ne!(second.digest, first.digest);

        std::fs::remove_file(&path).unwrap();
        assert!(matches!(memo.digest(&source), Err(RenderError::Io { .. })));
    }
}
