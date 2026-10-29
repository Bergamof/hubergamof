//! What the host gives a module instance: identity, redraw requests, events
//! sent to the instance and background tasks bound to its lifetime.

use std::any::Any;
use std::fmt;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll};

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

/// A value sent to a module instance through its [`EventSender`] and handed
/// back to [`crate::Module::on_event`]; the module downcasts it to its own
/// event type.
pub type ModuleEvent = Box<dyn Any + Send>;

/// Sends events to the module instance, typically from its background tasks:
/// the host passes each event to [`crate::Module::on_event`] on the device
/// task, then applies the returned commands and redraws. This is how a
/// background task changes the state of the instance.
///
/// Cheap to clone and `Send`. Events of an instance that was destroyed or
/// replaced are dropped.
#[derive(Clone)]
pub struct EventSender {
    send: Arc<dyn Fn(ModuleEvent) -> bool + Send + Sync>,
}

impl EventSender {
    /// Sender calling `send` with every event; `send` returns whether the
    /// event was accepted. Used by the host (and by tests to observe events).
    #[must_use]
    pub fn new(send: impl Fn(ModuleEvent) -> bool + Send + Sync + 'static) -> Self {
        Self {
            send: Arc::new(send),
        }
    }

    /// Sender dropping every event.
    #[must_use]
    pub fn noop() -> Self {
        Self::new(|_| false)
    }

    /// Sends `event` to the instance.
    ///
    /// Returns `false` when the event was dropped: the instance no longer
    /// exists (removed, replaced, or swapped for a placeholder), the device
    /// engine stopped, or its queue is full. A background task should then
    /// stop its work.
    pub fn send<E: Any + Send>(&self, event: E) -> bool {
        (self.send)(Box::new(event))
    }
}

impl fmt::Debug for EventSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventSender").finish_non_exhaustive()
    }
}

/// Panics of the tasks of a scope not yet reported to the host, and who to
/// tell as soon as one happens.
#[derive(Default)]
struct PanicSink {
    count: AtomicUsize,
    hook: Option<Box<dyn Fn() + Send + Sync>>,
}

impl PanicSink {
    fn record(&self) {
        self.count.fetch_add(1, Ordering::Relaxed);
        if let Some(hook) = &self.hook {
            hook();
        }
    }
}

/// Runs a task, turning a panic into a [`PanicSink::record`].
struct CatchPanic {
    task: Pin<Box<dyn Future<Output = ()> + Send>>,
    sink: Arc<PanicSink>,
}

impl Future for CatchPanic {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        // The task is never polled again after a panic: it is complete.
        catch_unwind(AssertUnwindSafe(|| this.task.as_mut().poll(cx))).unwrap_or_else(|_| {
            this.sink.record();
            Poll::Ready(())
        })
    }
}

/// Tasks of a scope.
struct Tasks {
    set: JoinSet<()>,
    sink: Arc<PanicSink>,
}

impl Tasks {
    /// Drops the results of finished tasks, so that a long-lived scope does
    /// not accumulate them.
    fn reap_finished(&mut self) {
        while self.set.try_join_next().is_some() {}
    }
}

type SharedTasks = Mutex<Tasks>;

/// Owner of the background tasks of one module instance, kept by the host.
///
/// Dropping the scope aborts every task spawned through its
/// [`TaskSpawner`]s: a module never outlives its instance. A task that
/// panics ends there; the panic is counted for the host.
pub struct TaskScope {
    tasks: Arc<SharedTasks>,
}

impl Default for TaskScope {
    fn default() -> Self {
        Self::with_sink(PanicSink::default())
    }
}

impl TaskScope {
    /// Scope without any task.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Scope calling `on_panic` (from the panicking task) whenever one of
    /// its tasks panics, so that the host can react at once.
    #[must_use]
    pub fn with_panic_hook(on_panic: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_sink(PanicSink {
            count: AtomicUsize::new(0),
            hook: Some(Box::new(on_panic)),
        })
    }

    fn with_sink(sink: PanicSink) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(Tasks {
                set: JoinSet::new(),
                sink: Arc::new(sink),
            })),
        }
    }

    /// Spawner of tasks bound to this scope.
    #[must_use]
    pub fn spawner(&self) -> TaskSpawner {
        TaskSpawner {
            tasks: Arc::downgrade(&self.tasks),
        }
    }

    /// Releases the finished tasks and returns how many tasks panicked since
    /// the previous call, so that the host can report them.
    #[must_use]
    pub fn reap_panics(&self) -> usize {
        let mut tasks = lock(&self.tasks);
        tasks.reap_finished();
        tasks.sink.count.swap(0, Ordering::Relaxed)
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
    tasks: Weak<SharedTasks>,
}

impl TaskSpawner {
    /// Spawner bound to no scope: every [`TaskSpawner::spawn`] returns
    /// `false`. For module tests that do not run background tasks.
    #[must_use]
    pub fn detached() -> Self {
        Self { tasks: Weak::new() }
    }

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
        tasks.reap_finished();
        let sink = Arc::clone(&tasks.sink);
        tasks.set.spawn(CatchPanic {
            task: Box::pin(task),
            sink,
        });
        true
    }
}

impl fmt::Debug for TaskSpawner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskSpawner").finish_non_exhaustive()
    }
}

fn lock(tasks: &SharedTasks) -> MutexGuard<'_, Tasks> {
    // The set is never left half-updated: a poisoned lock is still valid.
    tasks.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Everything a module instance receives from the host when it is created.
#[derive(Debug, Clone)]
pub struct ModuleContext {
    instance_id: ModuleInstanceId,
    redraw: RedrawHandle,
    events: EventSender,
    tasks: TaskSpawner,
}

impl ModuleContext {
    /// Context of instance `instance_id`; built by the host (or by tests).
    #[must_use]
    pub fn new(
        instance_id: ModuleInstanceId,
        redraw: RedrawHandle,
        events: EventSender,
        tasks: TaskSpawner,
    ) -> Self {
        Self {
            instance_id,
            redraw,
            events,
            tasks,
        }
    }

    /// Context for testing a module outside of the host: redraw requests and
    /// events are ignored and no background task runs, unless replaced with
    /// [`ModuleContext::with_redraw`], [`ModuleContext::with_events`] or
    /// [`ModuleContext::with_tasks`].
    #[must_use]
    pub fn for_tests(instance_id: impl Into<ModuleInstanceId>) -> Self {
        Self::new(
            instance_id.into(),
            RedrawHandle::noop(),
            EventSender::noop(),
            TaskSpawner::detached(),
        )
    }

    /// The same context with `redraw`.
    #[must_use]
    pub fn with_redraw(self, redraw: RedrawHandle) -> Self {
        Self { redraw, ..self }
    }

    /// The same context with `events`.
    #[must_use]
    pub fn with_events(self, events: EventSender) -> Self {
        Self { events, ..self }
    }

    /// The same context with `tasks`.
    #[must_use]
    pub fn with_tasks(self, tasks: TaskSpawner) -> Self {
        Self { tasks, ..self }
    }

    /// Identifier of the instance in the device configuration.
    #[must_use]
    pub fn instance_id(&self) -> &ModuleInstanceId {
        &self.instance_id
    }

    /// Handle to request redraws after asynchronous changes of what the
    /// getters return.
    #[must_use]
    pub fn redraw(&self) -> &RedrawHandle {
        &self.redraw
    }

    /// Sender of events to [`crate::Module::on_event`], to change the state
    /// of the instance from a background task.
    #[must_use]
    pub fn events(&self) -> &EventSender {
        &self.events
    }

    /// Spawner of background tasks aborted when the instance is destroyed.
    #[must_use]
    pub fn tasks(&self) -> &TaskSpawner {
        &self.tasks
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn event_sender_boxes_events_for_the_host() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        let sender = EventSender::new(move |event| {
            let value = *event.downcast::<u32>().unwrap();
            sink.lock().unwrap().push(value);
            true
        });
        assert!(sender.clone().send(7_u32));
        assert_eq!(*received.lock().unwrap(), [7]);
        assert!(!EventSender::noop().send(1_u32));
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

        drop(scope);
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert_eq!(ticks.load(Ordering::Relaxed), 2);
        assert!(!spawner.spawn(async {}));
    }

    #[tokio::test]
    async fn panics_of_tasks_are_counted_once() {
        let scope = TaskScope::new();
        let spawner = scope.spawner();
        assert!(spawner.spawn(async { panic!("task failure") }));
        assert!(spawner.spawn(async {}));
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut panics = 0;
            while panics == 0 {
                tokio::task::yield_now().await;
                panics = scope.reap_panics();
            }
            assert_eq!(panics, 1);
        })
        .await
        .unwrap();
        assert_eq!(scope.reap_panics(), 0);
    }

    #[tokio::test]
    async fn the_panic_hook_is_called_by_the_panicking_task() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = Mutex::new(Some(tx));
        let scope = TaskScope::with_panic_hook(move || {
            if let Some(tx) = tx.lock().unwrap().take() {
                tx.send(()).unwrap();
            }
        });
        assert!(scope.spawner().spawn(async {
            tokio::task::yield_now().await;
            panic!("task failure");
        }));
        tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(scope.reap_panics(), 1);
    }

    #[test]
    fn test_contexts_ignore_everything() {
        let ctx = ModuleContext::for_tests("counter");
        assert_eq!(ctx.instance_id(), &ModuleInstanceId::from("counter"));
        ctx.redraw().request_redraw();
        assert!(!ctx.events().send(1_u8));
        assert!(!ctx.tasks().spawn(async {}));

        let scope = TaskScope::new();
        let count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&count);
        let ctx = ctx
            .with_redraw(RedrawHandle::new(move || {
                counter.fetch_add(1, Ordering::Relaxed);
            }))
            .with_events(EventSender::new(|_| true))
            .with_tasks(scope.spawner());
        ctx.redraw().request_redraw();
        assert_eq!(count.load(Ordering::Relaxed), 1);
        assert!(ctx.events().send(1_u8));
    }
}
