//! Images already converted to the device wire format, by image identity.
//!
//! The engine hands the same `Arc<RgbaImage>` again for an unchanged look
//! (render cache), e.g. when the user goes back to a page: its JPEG/BMP
//! encoding is then reused instead of being redone.

use std::collections::VecDeque;
use std::sync::{Arc, Weak};

use image::RgbaImage;

/// Encoded images kept, most recently used last.
pub(super) const ENCODED_CACHE_CAPACITY: usize = 64;

/// Bounded cache of encodings keyed by the identity of the source image.
///
/// Entries hold a [`Weak`] to their source: the allocation cannot be reused
/// by another image while the entry exists, so pointer equality identifies
/// the very same image.
pub(super) struct EncodedCache<T> {
    entries: VecDeque<(Weak<RgbaImage>, Arc<T>)>,
    capacity: usize,
}

impl<T> EncodedCache<T> {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity: capacity.max(1),
        }
    }

    /// Encoding of `image`, computed by `encode` when not cached.
    pub(super) fn get_or_encode<E>(
        &mut self,
        image: &Arc<RgbaImage>,
        encode: impl FnOnce(&RgbaImage) -> Result<T, E>,
    ) -> Result<Arc<T>, E> {
        self.entries.retain(|(source, _)| source.strong_count() > 0);
        let position = self
            .entries
            .iter()
            .position(|(source, _)| std::ptr::eq(source.as_ptr(), Arc::as_ptr(image)));
        if let Some(entry) = position.and_then(|position| self.entries.remove(position)) {
            let encoded = Arc::clone(&entry.1);
            self.entries.push_back(entry);
            return Ok(encoded);
        }
        let encoded = Arc::new(encode(image)?);
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries
            .push_back((Arc::downgrade(image), Arc::clone(&encoded)));
        Ok(encoded)
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;

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
        let mut cache = EncodedCache::new(2);
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
    fn the_least_recently_used_entry_is_evicted() {
        let mut cache = EncodedCache::new(2);
        let mut count = 0;
        let images: Vec<_> = (0..3).map(|_| Arc::new(RgbaImage::new(1, 1))).collect();
        count_encodings(&mut cache, &images[0], &mut count);
        count_encodings(&mut cache, &images[1], &mut count);
        count_encodings(&mut cache, &images[0], &mut count);
        count_encodings(&mut cache, &images[2], &mut count);
        assert_eq!(count, 3);
        // 1 was evicted, 0 was kept.
        count_encodings(&mut cache, &images[0], &mut count);
        assert_eq!(count, 3);
        count_encodings(&mut cache, &images[1], &mut count);
        assert_eq!(count, 4);
    }

    #[test]
    fn dropped_images_leave_the_cache() {
        let mut cache = EncodedCache::new(4);
        let mut count = 0;
        let gone = Arc::new(RgbaImage::new(1, 1));
        count_encodings(&mut cache, &gone, &mut count);
        drop(gone);
        let kept = Arc::new(RgbaImage::new(1, 1));
        count_encodings(&mut cache, &kept, &mut count);
        assert_eq!(cache.entries.len(), 1);
    }
}
