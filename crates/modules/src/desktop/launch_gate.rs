//! One launch of an application at a time, whatever the instance or the
//! device it comes from: a second press while the first launch is under
//! way would start the application twice.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Held by the desktop hub, one per application.
#[derive(Debug, Clone, Default)]
pub(super) struct LaunchGate {
    busy: Arc<AtomicBool>,
}

impl LaunchGate {
    /// The pass of a new launch, `None` while another one is under way.
    #[must_use]
    pub(super) fn enter(&self) -> Option<LaunchPass> {
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| LaunchPass {
                busy: Arc::clone(&self.busy),
            })
    }
}

/// A launch under way; dropping it (launch reported, or never started)
/// lets the next one in.
#[derive(Debug)]
pub(super) struct LaunchPass {
    busy: Arc<AtomicBool>,
}

impl Drop for LaunchPass {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_launch_at_a_time_across_clones() {
        let gate = LaunchGate::default();
        let other = gate.clone();
        let pass = gate.enter().expect("free");
        assert!(other.enter().is_none(), "under way");
        drop(pass);
        assert!(other.enter().is_some(), "free again");
    }
}
