//! Notifications sent by the followed applications, observed on the session
//! bus.
//!
//! A dedicated connection becomes a monitor of the `Notify` calls to
//! `org.freedesktop.Notifications` (a monitor can no longer send anything:
//! senders are looked up through the shared connection). Applications do
//! not tell the manager when they need the user; their notifications do.
//!
//! A notification is attributed by the executable of its sender: the
//! `desktop-entry` hint, which any program may give, only rules out the
//! notifications naming another application (no lookup for those).
//!
//! Messages are read, and senders looked up, by a task of their own: the
//! stream only receives the result, so that waiting for it may be cancelled
//! without losing a notification.
//!
//! Privacy: the monitor sees every notification of the session. Those of
//! other applications are dropped at once, never kept nor logged.

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use futures_util::{Stream, StreamExt};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use zbus::fdo::{DBusProxy, MonitoringProxy};
use zbus::message::Type;
use zbus::names::{BusName, InterfaceName, MemberName, UniqueName};
use zbus::{MatchRule, Message, MessageStream, Variant};

use super::backend::{DesktopError, NotificationFilter, NotificationStream};
use super::model::{AppNotification, bounded};
use super::processes::executable_of;
use crate::session_bus::SessionBus;

/// Interface of the notification server.
const NOTIFICATIONS_INTERFACE: &str = "org.freedesktop.Notifications";

/// Method showing a notification.
const NOTIFY_MEMBER: &str = "Notify";

/// Hint naming the desktop entry of the sender.
const DESKTOP_ENTRY_HINT: &str = "desktop-entry";

/// Notifications attributed but not yet taken, past which the observer
/// waits.
const PENDING_CAPACITY: usize = 16;

/// Arguments of `Notify`: application name, notification replaced, icon,
/// summary, body, actions, hints, expiration.
type NotifyArguments<'a> = (
    &'a str,
    u32,
    &'a str,
    &'a str,
    &'a str,
    Vec<&'a str>,
    HashMap<&'a str, Variant<'a>>,
    i32,
);

/// Starts observing the notifications `filter` attributes.
pub(super) async fn observe(
    bus: &SessionBus,
    filter: &NotificationFilter,
) -> Result<Box<dyn NotificationStream>, DesktopError> {
    let lookup = bus.connection().await?.clone();
    let monitor = zbus::connection::Builder::session().build().await?;
    let rule = MatchRule::builder()
        .msg_type(Type::MethodCall)
        .interface(NOTIFICATIONS_INTERFACE)
        .member(NOTIFY_MEMBER)
        .build()?;
    MonitoringProxy::new(&monitor)
        .await?
        .become_monitor(&[rule], 0)
        .await?;
    let senders = BusSenders {
        dbus: DBusProxy::new(&lookup).await?,
    };
    Ok(Box::new(spawn_observer(
        MessageStream::from(monitor),
        filter.clone(),
        senders,
    )))
}

/// Finds the executable of a sender on the bus.
#[async_trait]
trait Senders: Send + Sync + 'static {
    /// The executable of the process behind `sender`, if it can be found.
    async fn executable(&self, sender: &UniqueName<'_>) -> Option<PathBuf>;
}

/// Senders looked up through the bus, then `/proc`.
struct BusSenders {
    dbus: DBusProxy<'static>,
}

#[async_trait]
impl Senders for BusSenders {
    async fn executable(&self, sender: &UniqueName<'_>) -> Option<PathBuf> {
        let pid = self
            .dbus
            .get_connection_unix_process_id(BusName::from(sender.clone()))
            .await
            .ok()?;
        tokio::task::spawn_blocking(move || executable_of(pid))
            .await
            .ok()
            .flatten()
    }
}

/// Reads `messages` in a task of its own, handing the notifications
/// `filter` attributes to the returned stream; the task ends with the
/// stream, or when the messages end.
fn spawn_observer(
    messages: impl Stream<Item = zbus::Result<Message>> + Send + Unpin + 'static,
    filter: NotificationFilter,
    senders: impl Senders,
) -> Observed {
    let (pending, received) = mpsc::channel(PENDING_CAPACITY);
    let task = tokio::spawn(read(messages, filter, senders, pending));
    Observed {
        received,
        task: task.abort_handle(),
    }
}

async fn read(
    mut messages: impl Stream<Item = zbus::Result<Message>> + Unpin,
    filter: NotificationFilter,
    senders: impl Senders,
    pending: mpsc::Sender<AppNotification>,
) {
    while let Some(message) = messages.next().await {
        let Ok(message) = message else {
            continue;
        };
        if let Some(notification) = attributed(&message, &filter, &senders).await
            && pending.send(notification).await.is_err()
        {
            return;
        }
    }
}

/// The notifications of an observer task.
struct Observed {
    received: mpsc::Receiver<AppNotification>,
    task: AbortHandle,
}

impl Drop for Observed {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[async_trait]
impl NotificationStream for Observed {
    async fn next(&mut self) -> Option<AppNotification> {
        self.received.recv().await
    }
}

/// The notification `message` shows, if `filter` attributes it; anything
/// else (other messages, other applications, unreadable calls, senders
/// gone) is `None`.
async fn attributed(
    message: &Message,
    filter: &NotificationFilter,
    senders: &impl Senders,
) -> Option<AppNotification> {
    let header = message.header();
    let is_notify = header.message_type() == Type::MethodCall
        && header.interface().map(InterfaceName::as_str) == Some(NOTIFICATIONS_INTERFACE)
        && header.member().map(MemberName::as_str) == Some(NOTIFY_MEMBER);
    if !is_notify {
        return None;
    }
    let body = message.body();
    let (_, _, _, summary, text, _, hints, _): NotifyArguments<'_> = body.deserialize().ok()?;
    let entry = match hints.get(DESKTOP_ENTRY_HINT) {
        Some(Variant::Str(entry)) => Some(entry.as_str()),
        _ => None,
    };
    if !filter.may_attribute(entry) {
        return None;
    }
    let executable = senders.executable(header.sender()?).await?;
    let app = filter.attribute(entry, &executable)?;
    Some(AppNotification {
        app,
        summary: bounded(summary.to_owned()),
        body: bounded(text.to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::stream;

    use super::*;
    use crate::desktop::model::{AppId, ClassMatch, DesktopApp, ExecutableMatch, MAX_TEXT_CHARS};

    const CLAUDE_EXE: &str = "/usr/lib/claude-desktop/claude-desktop";
    const CLAUDE_SENDER: &str = ":1.20";
    const CLAUDE_ENTRY: &str = "com.anthropic.Claude";

    /// Claude sends from `:1.20`; every other sender is another program.
    /// Each lookup takes `delay`.
    struct FakeSenders {
        delay: Duration,
    }

    const INSTANT: FakeSenders = FakeSenders {
        delay: Duration::ZERO,
    };

    #[async_trait]
    impl Senders for FakeSenders {
        async fn executable(&self, sender: &UniqueName<'_>) -> Option<PathBuf> {
            tokio::time::sleep(self.delay).await;
            Some(if sender.as_str() == CLAUDE_SENDER {
                CLAUDE_EXE.into()
            } else {
                "/usr/bin/notify-send".into()
            })
        }
    }

    fn filter() -> NotificationFilter {
        NotificationFilter::of(&[DesktopApp {
            id: AppId("claude"),
            window_class: ClassMatch::Exact(CLAUDE_ENTRY.into()),
            executable: ExecutableMatch {
                path: CLAUDE_EXE.into(),
                helper_argument: None,
            },
            desktop_entry: Some(CLAUDE_ENTRY.into()),
        }])
    }

    fn call(sender: &str, member: &str, body: &(impl serde::Serialize + zbus::Type)) -> Message {
        Message::method_call("/org/freedesktop/Notifications", member)
            .interface(NOTIFICATIONS_INTERFACE)
            .sender(sender)
            .build(body)
            .unwrap()
    }

    fn notify(sender: &str, summary: &str, entry: Option<&str>) -> Message {
        let mut hints: HashMap<&str, Variant<'_>> = HashMap::new();
        if let Some(entry) = entry {
            hints.insert(DESKTOP_ENTRY_HINT, Variant::from(entry));
        }
        let arguments: NotifyArguments<'_> =
            ("Claude", 0, "", summary, "Body", Vec::new(), hints, -1);
        call(sender, NOTIFY_MEMBER, &arguments)
    }

    #[tokio::test]
    async fn notifications_are_attributed_by_their_sender() {
        for entry in [Some(CLAUDE_ENTRY), None] {
            let message = notify(CLAUDE_SENDER, "Claude needs you", entry);
            let notification = attributed(&message, &filter(), &INSTANT).await.unwrap();
            assert_eq!(notification.app, AppId("claude"));
            assert_eq!(notification.summary, "Claude needs you");
            assert_eq!(notification.body, "Body");
        }
    }

    #[tokio::test]
    async fn forged_and_other_notifications_are_dropped() {
        let forged = notify(":1.30", "Claude needs you", Some(CLAUDE_ENTRY));
        assert_eq!(attributed(&forged, &filter(), &INSTANT).await, None);
        let other = notify(CLAUDE_SENDER, "New mail", Some("org.mozilla.Thunderbird"));
        assert_eq!(attributed(&other, &filter(), &INSTANT).await, None);
        let unnamed = notify(":1.30", "New mail", None);
        assert_eq!(attributed(&unnamed, &filter(), &INSTANT).await, None);

        let close = call(CLAUDE_SENDER, "CloseNotification", &(1_u32,));
        assert_eq!(attributed(&close, &filter(), &INSTANT).await, None);
        let unreadable = call(CLAUDE_SENDER, NOTIFY_MEMBER, &("only a name",));
        assert_eq!(attributed(&unreadable, &filter(), &INSTANT).await, None);
    }

    #[tokio::test]
    async fn texts_are_bounded() {
        let long = "x".repeat(MAX_TEXT_CHARS * 3);
        let message = notify(CLAUDE_SENDER, &long, Some(CLAUDE_ENTRY));
        let notification = attributed(&message, &filter(), &INSTANT).await.unwrap();
        assert_eq!(notification.summary.len(), MAX_TEXT_CHARS);
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_may_be_cancelled_during_a_slow_lookup() {
        let messages = stream::iter([
            Ok(notify(":1.30", "ignored", None)),
            Ok(notify(CLAUDE_SENDER, "first", None)),
            Ok(notify(CLAUDE_SENDER, "second", None)),
        ]);
        let slow = FakeSenders {
            delay: Duration::from_secs(5),
        };
        let mut observed = spawn_observer(messages, filter(), slow);
        // The hub gives up waiting many times while the lookups go on.
        for _ in 0..12 {
            let cancelled = tokio::time::timeout(Duration::from_millis(500), observed.next()).await;
            assert!(cancelled.is_err());
        }
        let first = observed.next().await.unwrap();
        let second = observed.next().await.unwrap();
        assert_eq!([first.summary, second.summary], ["first", "second"]);
        assert_eq!(observed.next().await, None, "the messages ended");
    }

    #[tokio::test]
    async fn the_observer_stops_with_its_stream() {
        let observed = spawn_observer(stream::pending(), filter(), INSTANT);
        let task = observed.task.clone();
        drop(observed);
        tokio::task::yield_now().await;
        assert!(task.is_finished());
    }
}
