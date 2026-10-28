//! What the host gives a module instance: identity, redraw requests and
//! background tasks bound to the instance lifetime.

use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use tokio::task::JoinSet;

use crate::ModuleInstanceId;

/// Asks the host to redraw, from anywhere (timers, watchers, other threads).
///
/// Cheap to clone. Requests are coalesced: calling it many times before the
/// host redraws costs one redraw. The host only rewrites what changed.
#[derive(Clone)]
pub struct RedrawHandle {
    request: Arc<dyn Fn() + Send + Sync>,
}

impl RedrawHandle {
    /// Handle calling `request` on every [`RedrawHandle::request_redraw`];
    /// used by the host (and by tests to observe redraw requests).
    #[must_use]
    pub fn new(request: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            request: Arc::new(request),
        }
    }

    /// Handle ignoring every request.
    #[must_use]
    pub fn noop() -> Self {
        Self::new(|| {})
    }

    /// Requests a redraw of the device surfaces.
    pub fn request_redraw(&self) {
        (self.request)();
    }
}

impl fmt::Debug for RedrawHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedrawHandle").finish_non_exhaustive()
    }
}

type Tasks = Mutex<JoinSet<()>>;

/// Owner of the background tasks of one module instance, kept by the host.
///
/// Dropping the scope aborts every task spawned through its
/// [`TaskSpawner`]s: a module never outlives its instance.
#[derive(Default)]
pub struct TaskScope {
    tasks: Arc<Tasks>,
}

impl TaskScope {
    /// Scope without any task.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawner of tasks bound to this scope.
    #[must_use]
    pub fn spawner(&self) -> TaskSpawner {
        TaskSpawner {
            tasks: Arc::downgrade(&self.tasks),
        }
    }

    /// Number of tasks still running.
    #[must_use]
    pub fn running(&self) -> usize {
        let mut tasks = lock(&self.tasks);
        reap_finished(&mut tasks);
        tasks.len()
    }
}

impl fmt::Debug for TaskScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskScope").finish_non_exhaustive()
    }
}

/// Spawns background tasks bound to a [`TaskScope`]. Cheap to clone.
#[derive(Clone)]
pub struct TaskSpawner {
    tasks: Weak<Tasks>,
}

impl TaskSpawner {
    /// Runs `task` on the Tokio runtime until it completes or the scope is
    /// dropped. Returns `false` (and drops `task`) when the scope is gone.
    ///
    /// # Panics
    ///
    /// When called outside a Tokio runtime. The host always calls module
    /// code from within its runtime.
    pub fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) -> bool {
        let Some(tasks) = self.tasks.upgrade() else {
            return false;
        };
        let mut tasks = lock(&tasks);
        reap_finished(&mut tasks);
        tasks.spawn(task);
        true
    }
}

impl fmt::Debug for TaskSpawner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskSpawner").finish_non_exhaustive()
    }
}

/// Drops the results of finished tasks so that a long-lived scope does not
/// accumulate them.
fn reap_finished(tasks: &mut JoinSet<()>) {
    while tasks.try_join_next().is_some() {}
}

fn lock(tasks: &Tasks) -> std::sync::MutexGuard<'_, JoinSet<()>> {
    // The set is never left half-updated: a poisoned lock is still valid.
    tasks.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Everything a module instance receives from the host when it is created.
#[derive(Debug, Clone)]
pub struct ModuleContext {
    instance_id: ModuleInstanceId,
    redraw: RedrawHandle,
    tasks: TaskSpawner,
}

impl ModuleContext {
    /// Context of instance `instance_id`; built by the host (or by tests).
    #[must_use]
    pub fn new(instance_id: ModuleInstanceId, redraw: RedrawHandle, tasks: TaskSpawner) -> Self {
        Self {
            instance_id,
            redraw,
            tasks,
        }
    }

    /// Identifier of the instance in the device configuration.
    #[must_use]
    pub fn instance_id(&self) -> &ModuleInstanceId {
        &self.instance_id
    }

    /// Handle to request redraws after asynchronous changes.
    #[must_use]
    pub fn redraw(&self) -> &RedrawHandle {
        &self.redraw
    }

    /// Spawner of background tasks aborted when the instance is destroyed.
    #[must_use]
    pub fn tasks(&self) -> &TaskSpawner {
        &self.tasks
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    #[test]
    fn redraw_handle_calls_its_callback() {
        let count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&count);
        let handle = RedrawHandle::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        handle.clone().request_redraw();
        handle.request_redraw();
        RedrawHandle::noop().request_redraw();
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_the_scope_aborts_its_tasks() {
        let scope = TaskScope::new();
        let spawner = scope.spawner();
        let ticks = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&ticks);
        assert!(spawner.spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                counter.fetch_add(1, Ordering::Relaxed);
            }
        }));
        assert!(spawner.spawn(async {}));
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert_eq!(ticks.load(Ordering::Relaxed), 2);
        assert_eq!(scope.running(), 1);

        drop(scope);
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert_eq!(ticks.load(Ordering::Relaxed), 2);
        assert!(!spawner.spawn(async {}));
    }
}
