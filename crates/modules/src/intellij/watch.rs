//! The installation, read once for every IntelliJ instance of every device:
//! a worker, started with the first [`CatalogSubscription`] and stopped
//! with the last one, reads the [`ProjectCatalog`] every [`CATALOG_POLL`]
//! and publishes it when it changed.
//!
//! A read stuck on the file system (a project on an unreachable network
//! mount) is given up after [`READ_TIMEOUT`] and never doubled: the next
//! read waits for it to end.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::{AbortHandle, JoinHandle};

use super::catalog::{Catalog, ProjectCatalog};
use crate::background::{WorkerControl, spawn_worker};

/// Time between two reads of the installation (cached by the catalog while
/// its files do not change).
pub(super) const CATALOG_POLL: Duration = Duration::from_secs(5);

/// Longest wait for a read.
pub(super) const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Shared reading of the installation, see the [module
/// documentation](self).
#[derive(Debug)]
pub(super) struct CatalogHub {
    catalog: Arc<dyn ProjectCatalog>,
    /// `None` until first read.
    states: watch::Sender<Option<Arc<Catalog>>>,
    control: Mutex<WorkerControl>,
}

impl CatalogHub {
    pub(super) fn new(catalog: Arc<dyn ProjectCatalog>) -> Arc<Self> {
        Arc::new(Self {
            catalog,
            states: watch::Sender::new(None),
            control: Mutex::default(),
        })
    }

    /// A subscription; the first one starts the worker (on the current
    /// Tokio runtime, if any), as does one finding it stopped.
    pub(super) fn subscribe(self: &Arc<Self>) -> CatalogSubscription {
        self.lock().subscribe(|| self.start_worker());
        CatalogSubscription {
            hub: Arc::clone(self),
        }
    }

    fn start_worker(&self) -> Option<AbortHandle> {
        spawn_worker(
            "intellij catalog",
            poll(Arc::clone(&self.catalog), self.states.clone()),
        )
    }

    fn lock(&self) -> MutexGuard<'_, WorkerControl> {
        // Every update keeps the control consistent: a poisoned lock is
        // still valid.
        self.control.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// An instance's access to the installation; dropping it unsubscribes.
#[derive(Debug)]
pub(super) struct CatalogSubscription {
    hub: Arc<CatalogHub>,
}

impl CatalogSubscription {
    /// The installation, the current one first.
    pub(super) fn states(&self) -> watch::Receiver<Option<Arc<Catalog>>> {
        self.hub.states.subscribe()
    }
}

impl Drop for CatalogSubscription {
    fn drop(&mut self) {
        // The worker only reads: stopping it mid-read loses nothing.
        self.hub.lock().unsubscribe();
    }
}

/// Reads `catalog` every [`CATALOG_POLL`], publishing what changed; a
/// failed read publishes an empty catalog (logged once).
async fn poll(catalog: Arc<dyn ProjectCatalog>, states: watch::Sender<Option<Arc<Catalog>>>) {
    let mut reading: Option<JoinHandle<Result<Catalog, String>>> = None;
    let mut failing = false;
    loop {
        let mut read = reading.take().unwrap_or_else(|| {
            let reader = Arc::clone(&catalog);
            tokio::task::spawn_blocking(move || reader.read())
        });
        let outcome = if let Ok(joined) = tokio::time::timeout(READ_TIMEOUT, &mut read).await {
            Some(joined.unwrap_or_else(|error| Err(error.to_string())))
        } else {
            reading = Some(read);
            None
        };
        match outcome {
            // Still reading: waited for at the next poll.
            None => {
                if !failing {
                    tracing::warn!("reading the IntelliJ projects takes too long");
                }
                failing = true;
            }
            Some(Ok(read)) => {
                failing = false;
                publish(&states, read);
            }
            Some(Err(error)) => {
                if !failing {
                    tracing::warn!(%error, "cannot read the IntelliJ projects");
                }
                failing = true;
                publish(&states, Catalog::default());
            }
        }
        tokio::time::sleep(CATALOG_POLL).await;
    }
}

fn publish(states: &watch::Sender<Option<Arc<Catalog>>>, read: Catalog) {
    states.send_if_modified(|current| {
        let changed = current.as_deref() != Some(&read);
        if changed {
            *current = Some(Arc::new(read));
        }
        changed
    });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    use super::*;

    /// A catalog whose reads wait for a release, counted.
    #[derive(Debug)]
    struct StuckCatalog {
        release: Mutex<mpsc::Receiver<()>>,
        reads: AtomicUsize,
    }

    impl ProjectCatalog for StuckCatalog {
        fn read(&self) -> Result<Catalog, String> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let release = self.release.lock().unwrap();
            release.recv().map_err(|error| error.to_string())?;
            Ok(Catalog::default())
        }
    }

    /// Lets `duration` pass: time does not advance by itself while a
    /// blocking read runs.
    async fn pass(duration: Duration) {
        tokio::time::advance(duration).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_stuck_read_is_never_doubled() {
        let (release, waiting) = mpsc::channel();
        let catalog = Arc::new(StuckCatalog {
            release: Mutex::new(waiting),
            reads: AtomicUsize::new(0),
        });
        let hub = CatalogHub::new(Arc::clone(&catalog) as _);
        let subscription = hub.subscribe();
        let states = subscription.states();
        for _ in 0..4 {
            pass(READ_TIMEOUT + CATALOG_POLL).await;
        }
        assert_eq!(catalog.reads.load(Ordering::SeqCst), 1, "never doubled");
        assert!(states.borrow().is_none(), "nothing published");

        // Released: the read waited for is published, and polling goes on.
        release.send(()).unwrap();
        while states.borrow().is_none() {
            pass(CATALOG_POLL).await;
        }
        assert_eq!(states.borrow().as_deref(), Some(&Catalog::default()));
        pass(CATALOG_POLL + CATALOG_POLL).await;
        assert_eq!(catalog.reads.load(Ordering::SeqCst), 2, "polling goes on");
        // The runtime waits for blocking reads when it shuts down.
        drop(release);
        drop(subscription);
    }
}
