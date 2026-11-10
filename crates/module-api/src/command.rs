//! Requests a module returns to the host after handling an input or an event.

use crate::{ItemId, Notification, NotificationId, PageId};

/// What the host should do after a module handled an input or an event.
///
/// Commands are applied in order. Every handler is followed by a redraw of
/// what changed, so there is no explicit redraw command; changes made outside
/// a handler use [`crate::RedrawHandle`] or [`crate::EventSender`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Command {
    /// Opens a page of the module that returned the command (see
    /// [`crate::Module::page`]) on top of the navigation stack.
    OpenPage(PageId),
    /// Returns to the previous page; does nothing on the home page.
    Back,
    /// Returns to a page of the module that returned the command: the pages
    /// opened above it are closed. Does nothing when that page is not on the
    /// navigation stack below the displayed one (it was never opened, or
    /// is the displayed page).
    BackTo(PageId),
    /// Returns to the home page.
    GoHome,
    /// Scrolls the displayed page by this many rows (positive: down); the
    /// host keeps the visible rows within the page.
    ScrollRows(i32),
    /// Briefly marks the surface that triggered the handler (the pressed
    /// key, the zone of the turned dial; for [`crate::Module::on_event`], the
    /// home key of the module when it is displayed, or else the zones of its
    /// dials) with the outcome of the action.
    Feedback(Feedback),
    /// Briefly marks a given surface of the module with the outcome of an
    /// action. Meant for [`crate::Module::on_event`], when a background task
    /// reports the outcome of an action started by an earlier input (the
    /// dial zone for a volume change, the pressed item for a selection).
    /// Only surfaces displayed when the command is applied are marked: a
    /// surface shown afterwards (its page opened within the mark duration)
    /// carries no mark.
    FeedbackOn(ModuleSurface, Feedback),
    /// Shows a notification over the touch strip (queued after the one
    /// already shown), and counts it as unread until the user touches it.
    Notify(Notification),
    /// Shows an unread notification again, with no timeout (see
    /// [`crate::UnreadNotification::id`]); ignored if it was read since.
    ShowNotification(NotificationId),
}

impl Command {
    /// Marks the surface that triggered the handler as failed: what a
    /// handler returns when it cannot even start its action.
    #[must_use]
    pub fn alert() -> Vec<Self> {
        vec![Self::Feedback(Feedback::Alert)]
    }
}

/// A surface of the module returning a [`Command::FeedbackOn`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ModuleSurface {
    /// Its key on the home page.
    HomeKey,
    /// The touch strip zones of the dials it is assigned to.
    Zone,
    /// The key showing item `item` of its page `page`.
    Item {
        /// Page of the item.
        page: PageId,
        /// The item.
        item: ItemId,
    },
}

/// Outcome of an action, shown for a moment over its surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Feedback {
    /// The action succeeded (green check mark).
    Ok,
    /// The action failed (red warning sign).
    Alert,
}

impl Feedback {
    /// The mark of an action that succeeded (`ok`) or failed.
    #[must_use]
    pub fn of(ok: bool) -> Self {
        if ok { Self::Ok } else { Self::Alert }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_follows_the_outcome() {
        assert_eq!(Feedback::of(true), Feedback::Ok);
        assert_eq!(Feedback::of(false), Feedback::Alert);
        assert_eq!(Command::alert(), [Command::Feedback(Feedback::Alert)]);
    }
}
