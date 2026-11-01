//! Bounded cache that volatile content cannot flush: a segmented LRU with a
//! history of recently evicted keys.
//!
//! Some surfaces change every second (a countdown, a level): each of their
//! images is used once and never again. In a plain LRU they would push out
//! the images that come back (icons, labels of the pages one returns to).
//! Here a new entry goes to a small *probation* segment; it reaches the
//! large *protected* segment only once it is used again:
//!
//! - on a hit while still in probation;
//! - or when it is inserted again soon after leaving probation (its key is
//!   in the *history*): a key shown once then redrawn later — the home key
//!   after a visit to a page — is not used in between, and probation may
//!   have turned over meanwhile.
//!
//! One-shot entries only ever cycle through probation, so they cost at most
//! its capacity. The protected segment is an LRU of its own; its least
//! recently used entry goes back to probation for a last chance.

use std::hash::Hash;
use std::num::NonZeroUsize;

use lru::LruCache;

/// Segmented LRU of at most `protected + probation` values, remembering a
/// history of up as many evicted keys, each with a ghost `G` derived from
/// its value (`()` usually; see [`SegmentedLru::with_ghosts`]).
pub(crate) struct SegmentedLru<K: Hash + Eq, V, G = ()> {
    probation: LruCache<K, V>,
    protected: LruCache<K, V>,
    history: LruCache<K, G>,
    ghost_of: fn(&V) -> G,
}

impl<K: Hash + Eq + Clone, V> SegmentedLru<K, V> {
    /// Cache keeping `protected` reused values and `probation` new ones.
    pub(crate) fn new(protected: NonZeroUsize, probation: NonZeroUsize) -> Self {
        Self::with_ghosts(protected, probation, |_| ())
    }
}

impl<K: Hash + Eq + Clone, V, G> SegmentedLru<K, V, G> {
    /// Like [`SegmentedLru::new`], the history keeping `ghost_of(value)`
    /// with each evicted key: for keys whose meaning depends on the value
    /// staying alive (a pointer identity kept valid by a weak reference).
    pub(crate) fn with_ghosts(
        protected: NonZeroUsize,
        probation: NonZeroUsize,
        ghost_of: fn(&V) -> G,
    ) -> Self {
        Self {
            probation: LruCache::new(probation),
            protected: LruCache::new(protected),
            history: LruCache::new(protected.saturating_add(probation.get())),
            ghost_of,
        }
    }

    /// Number of cached values.
    pub(crate) fn len(&self) -> usize {
        self.probation.len() + self.protected.len()
    }

    /// The value of `key`, promoted to the protected segment if it was in
    /// probation.
    pub(crate) fn get(&mut self, key: &K) -> Option<&V> {
        if !self.protected.contains(key) {
            let value = self.probation.pop(key)?;
            self.protect(key.clone(), value);
        }
        self.protected.get(key)
    }

    /// Caches `value` under `key`: in probation, or protected when the key
    /// was evicted recently (it is being used again).
    pub(crate) fn insert(&mut self, key: K, value: V) {
        if self.history.pop(&key).is_some() || self.protected.contains(&key) {
            self.probation.pop(&key);
            self.protect(key, value);
        } else {
            self.probation_push(key, value);
        }
    }

    /// Drops the values, and the ghosts of the history, failing the tests.
    pub(crate) fn retain(&mut self, keep: impl Fn(&V) -> bool, keep_ghost: impl Fn(&G) -> bool) {
        retain_in(&mut self.probation, &keep);
        retain_in(&mut self.protected, &keep);
        retain_in(&mut self.history, &keep_ghost);
    }

    fn protect(&mut self, key: K, value: V) {
        // `push` returns the replaced entry, or else the one evicted.
        let replacing = self.protected.contains(&key);
        if let Some((demoted, value)) = self.protected.push(key, value)
            && !replacing
        {
            self.probation_push(demoted, value);
        }
    }

    fn probation_push(&mut self, key: K, value: V) {
        let replacing = self.probation.contains(&key);
        if let Some((evicted, value)) = self.probation.push(key, value)
            && !replacing
        {
            let ghost = (self.ghost_of)(&value);
            self.history.put(evicted, ghost);
        }
    }
}

fn retain_in<K: Hash + Eq + Clone, V>(cache: &mut LruCache<K, V>, keep: impl Fn(&V) -> bool) {
    let dropped: Vec<K> = cache
        .iter()
        .filter(|(_, value)| !keep(value))
        .map(|(key, _)| key.clone())
        .collect();
    for key in dropped {
        cache.pop(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capacity(value: usize) -> NonZeroUsize {
        NonZeroUsize::new(value).unwrap()
    }

    /// A cache of 4 protected and 2 probation entries holding `key * 10`.
    fn cache() -> SegmentedLru<u32, u32> {
        SegmentedLru::new(capacity(4), capacity(2))
    }

    fn fill(cache: &mut SegmentedLru<u32, u32>, keys: impl IntoIterator<Item = u32>) {
        for key in keys {
            cache.insert(key, key * 10);
        }
    }

    #[test]
    fn a_reused_entry_survives_any_number_of_one_shot_entries() {
        let mut cache = cache();
        fill(&mut cache, [1]);
        assert_eq!(cache.get(&1), Some(&10));
        fill(&mut cache, 100..400);
        assert_eq!(cache.get(&1), Some(&10));
        assert_eq!(cache.len(), 3, "one-shot entries stay within probation");
    }

    #[test]
    fn an_entry_inserted_again_after_its_eviction_is_protected() {
        let mut cache = cache();
        fill(&mut cache, [1]);
        // Never hit, pushed out of probation: only its key is remembered.
        fill(&mut cache, [2, 3]);
        assert_eq!(cache.get(&1), None);
        fill(&mut cache, [1]);
        fill(&mut cache, 100..400);
        assert_eq!(cache.get(&1), Some(&10));
    }

    #[test]
    fn one_shot_entries_are_forgotten() {
        let mut cache = cache();
        fill(&mut cache, [1, 2, 3]);
        assert_eq!(cache.get(&1), None);
        assert_eq!(cache.get(&3), Some(&30));
        assert_eq!(cache.get(&2), Some(&20));
    }

    #[test]
    fn the_protected_segment_is_bounded_and_demotes_to_probation() {
        let mut cache = cache();
        for key in 1..=5 {
            fill(&mut cache, [key]);
            assert!(cache.get(&key).is_some());
        }
        // 1 left the protected segment for probation: still there.
        assert_eq!(cache.len(), 5);
        assert_eq!(cache.get(&1), Some(&10));
        // Bounded in all cases.
        for key in 10..100 {
            fill(&mut cache, [key]);
            cache.get(&key);
        }
        assert_eq!(cache.len(), 6);
    }

    #[test]
    fn replacing_a_value_keeps_its_segment() {
        let mut cache = cache();
        fill(&mut cache, [1]);
        cache.get(&1);
        cache.insert(1, 11);
        fill(&mut cache, 100..200);
        assert_eq!(cache.get(&1), Some(&11));
        cache.insert(7, 70);
        cache.insert(7, 71);
        assert_eq!(cache.get(&7), Some(&71));
    }

    #[test]
    fn retain_drops_values_and_ghosts() {
        let mut cache = SegmentedLru::with_ghosts(capacity(4), capacity(1), |value: &u32| *value);
        cache.insert(1, 10);
        cache.get(&1);
        cache.insert(2, 20);
        cache.insert(3, 30);
        cache.retain(|value| *value != 10, |ghost| *ghost != 20);
        assert_eq!(cache.get(&1), None);
        // 2 was evicted to the history, then its ghost dropped: probation.
        cache.insert(2, 20);
        cache.insert(4, 40);
        assert_eq!(cache.get(&2), None);
        assert_eq!(cache.get(&4), Some(&40));
    }
}
