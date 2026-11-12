//! Images already converted to the device wire format, by image identity.
//!
//! The engine hands the same `Arc<RgbaImage>` again for an unchanged look
//! (render cache), e.g. when the user goes back to a page: its JPEG/BMP
//! encoding is then reused instead of being redone. Like the render cache,
//! this one is segmented (see [`crate::cache`]): the images of a countdown,
//! encoded once each, do not push out the encodings of the icons.

use std::num::NonZeroUsize;
use std::sync::{Arc, Weak};

use image::RgbaImage;

use crate::cache::SegmentedLru;

/// Encodings of images written more than once kept.
pub(super) const ENCODED_CACHE_CAPACITY: NonZeroUsize = match NonZeroUsize::new(48) {
    Some(capacity) => capacity,
    None => NonZeroUsize::MIN,
};

/// Encodings of new images kept until they are written again.
pub(super) const ENCODED_PROBATION_CAPACITY: NonZeroUsize = match NonZeroUsize::new(16) {
    Some(capacity) => capacity,
    None => NonZeroUsize::MIN,
};

/// Encodings of strip images kept, whatever their use: the strip changes
/// every second, so one image is rarely written twice; two are enough for
/// the one shown again when a notification goes away.
pub(super) const ENCODED_STRIP_CAPACITY: NonZeroUsize = match NonZeroUsize::new(2) {
    Some(capacity) => capacity,
    None => NonZeroUsize::MIN,
};

/// Address of a source image.
type Identity = usize;

/// An encoding and the image it encodes.
type Entry<T> = (Weak<RgbaImage>, Arc<T>);

/// Bounded cache of encodings keyed by the identity of the source image.
///
/// Entries, and the keys remembered after an eviction, hold a [`Weak`] to
/// their source: the allocation cannot be reused by another image while
/// they exist, so the address identifies the very same image. Those whose
/// image is gone are dropped before every lookup.
pub(super) struct EncodedCache<T> {
    entries: SegmentedLru<Identity, Entry<T>, Weak<RgbaImage>>,
}

impl<T> EncodedCache<T> {
    pub(super) fn new(reused: NonZeroUsize, probation: NonZeroUsize) -> Self {
        Self {
            entries: SegmentedLru::with_ghosts(reused, probation, |(source, _)| {
                Weak::clone(source)
            }),
        }
    }

    /// Encoding of `image`, computed by `encode` when not cached.
    pub(super) fn get_or_encode<E>(
        &mut self,
        image: &Arc<RgbaImage>,
        encode: impl FnOnce(&RgbaImage) -> Result<T, E>,
    ) -> Result<Arc<T>, E> {
        self.entries.retain(
            |(source, _)| source.strong_count() > 0,
            |source| source.strong_count() > 0,
        );
        let identity = Arc::as_ptr(image).addr();
        if let Some((_, encoded)) = self.entries.get(&identity) {
            return Ok(Arc::clone(encoded));
        }
        let encoded = Arc::new(encode(image)?);
        self.entries
            .insert(identity, (Arc::downgrade(image), Arc::clone(&encoded)));
        Ok(encoded)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;

    fn capacity(value: usize) -> NonZeroUsize {
        NonZeroUsize::new(value).unwrap()
    }

    fn count_encodings(
        cache: &mut EncodedCache<u32>,
        image: &Arc<RgbaImage>,
        count: &mut u32,
    ) -> u32 {
        *cache
            .get_or_encode(image, |_| {
                *count += 1;
                Ok::<_, Infallible>(*count)
            })
            .unwrap()
    }

    #[test]
    fn the_same_image_is_encoded_once() {
        let mut cache = EncodedCache::new(capacity(2), capacity(2));
        let mut count = 0;
        let image = Arc::new(RgbaImage::new(2, 2));
        let twin = Arc::new(RgbaImage::new(2, 2));
        assert_eq!(count_encodings(&mut cache, &image, &mut count), 1);
        assert_eq!(
            count_encodings(&mut cache, &Arc::clone(&image), &mut count),
            1
        );
        // Equal pixels but another image: identity, not content, is the key.
        assert_eq!(count_encodings(&mut cache, &twin, &mut count), 2);
    }

    #[test]
    fn images_written_once_do_not_push_out_those_written_again() {
        let mut cache = EncodedCache::new(capacity(4), capacity(2));
        let mut count = 0;
        let icon = Arc::new(RgbaImage::new(1, 1));
        count_encodings(&mut cache, &icon, &mut count);
        count_encodings(&mut cache, &icon, &mut count);
        // A countdown: every image is new, kept alive meanwhile by the
        // render cache.
        let countdown: Vec<_> = (0..300).map(|_| Arc::new(RgbaImage::new(1, 1))).collect();
        for image in &countdown {
            count_encodings(&mut cache, image, &mut count);
        }
        assert_eq!(count, 301);
        count_encodings(&mut cache, &icon, &mut count);
        assert_eq!(count, 301, "the icon encoding was kept");
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn an_image_written_again_after_its_eviction_is_kept() {
        let mut cache = EncodedCache::new(capacity(4), capacity(1));
        let mut count = 0;
        let icon = Arc::new(RgbaImage::new(1, 1));
        let other = Arc::new(RgbaImage::new(1, 1));
        count_encodings(&mut cache, &icon, &mut count);
        count_encodings(&mut cache, &other, &mut count);
        count_encodings(&mut cache, &icon, &mut count);
        assert_eq!(count, 3, "evicted from probation");
        let countdown: Vec<_> = (0..10).map(|_| Arc::new(RgbaImage::new(1, 1))).collect();
        for image in &countdown {
            count_encodings(&mut cache, image, &mut count);
        }
        count_encodings(&mut cache, &icon, &mut count);
        assert_eq!(count, 13, "protected since its second encoding");
    }

    #[test]
    fn dropped_images_leave_the_cache() {
        let mut cache = EncodedCache::new(capacity(4), capacity(4));
        let mut count = 0;
        let gone = Arc::new(RgbaImage::new(1, 1));
        count_encodings(&mut cache, &gone, &mut count);
        drop(gone);
        let kept = Arc::new(RgbaImage::new(1, 1));
        count_encodings(&mut cache, &kept, &mut count);
        assert_eq!(cache.len(), 1);
    }
}
