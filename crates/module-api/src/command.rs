//! Requests a module returns to the host after handling an input.

use crate::PageId;

/// What the host should do after a module handled an input.
///
/// Commands are applied in order.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Redraws every surface. The host already compares and redraws what
    /// changed after each input, so this is only an explicit request; changes
    /// made outside an input handler use [`crate::RedrawHandle`] instead.
    Redraw,
}
