//! Helpers for testing a module outside of the host (feature `testing`).

use tokio::sync::mpsc::{self, UnboundedReceiver};

use crate::{EventSender, ModuleContext, ModuleEvent, ModuleInstanceId, TaskScope};

/// A context whose events are collected and whose background tasks run,
/// as the host would provide, with what a test needs to observe them.
///
/// The host hands each event to [`crate::Module::on_event`]; a test does
/// the same with the events received on [`TestContext::events`]. Dropping
/// [`TestContext::scope`] aborts the background tasks, like destroying the
/// instance does.
#[derive(Debug)]
pub struct TestContext {
    /// Context to give to [`crate::ModuleFactory::create`].
    pub ctx: ModuleContext,
    /// Events sent by the instance (usually by its background tasks).
    pub events: UnboundedReceiver<ModuleEvent>,
    /// Owner of the background tasks of the instance.
    pub scope: TaskScope,
}

impl TestContext {
    /// Context of instance `instance_id`, collecting events and running
    /// background tasks. Redraw requests are ignored.
    #[must_use]
    pub fn new(instance_id: impl Into<ModuleInstanceId>) -> Self {
        let (sender, events) = mpsc::unbounded_channel();
        let scope = TaskScope::new();
        let ctx = ModuleContext::for_tests(instance_id)
            .with_events(EventSender::new(move |event| sender.send(event).is_ok()))
            .with_tasks(scope.spawner());
        Self { ctx, events, scope }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn events_are_collected_and_tasks_run() {
        let mut test = TestContext::new("probe");
        assert_eq!(test.ctx.instance_id(), &ModuleInstanceId::from("probe"));
        let events = test.ctx.events().clone();
        assert!(test.ctx.tasks().spawn(async move {
            assert!(events.send(7_u32));
        }));
        let event = test.events.recv().await.unwrap();
        assert_eq!(*event.downcast::<u32>().unwrap(), 7);

        drop(test.scope);
        drop(test.events);
        assert!(!test.ctx.events().send(1_u8));
        assert!(!test.ctx.tasks().spawn(async {}));
    }
}
