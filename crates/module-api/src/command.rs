//! Requests a module returns to the host after handling an input or an event.

use crate::{ItemId, PageId};

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
    /// Returns to the home page.
    GoHome,
    /// Scrolls the displayed page by this many rows (positive: down); the
    /// host keeps the visible rows within the page.
    ScrollRows(i32),
    /// Briefly marks the surface that triggered the handler (the pressed
    /// key, the zone of the turned dial; for [`crate::Module::on_event`], the
    /// home key of the module when it is displayed) with the outcome of the
    /// action.
    Feedback(Feedback),
    /// Briefly marks a given surface of the module with the outcome of an
    /// action. Meant for [`crate::Module::on_event`], when a background task
    /// reports the outcome of an action started by an earlier input (the
    /// dial zone for a volume change, the pressed item for a selection).
    /// Surfaces not displayed at that moment show nothing.
    FeedbackOn(ModuleSurface, Feedback),
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
