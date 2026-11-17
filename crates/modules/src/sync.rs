//! Small synchronisation helpers shared by the modules.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Locks `mutex`, ignoring poisoning: what the modules keep behind these
/// locks is updated in single steps, so a panic elsewhere leaves it valid
/// and must not take every later reader down with it.
pub(crate) fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
