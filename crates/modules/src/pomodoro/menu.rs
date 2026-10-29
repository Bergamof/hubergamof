//! Dial menu of the pomodoro: opened by a rotation, closed by a press or
//! after [`MENU_TIMEOUT`] without input.

use std::time::Duration;

use tokio::time::Instant;

/// Time without rotation after which the menu closes by itself.
pub const MENU_TIMEOUT: Duration = Duration::from_secs(5);

/// An entry of the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEntry {
    /// Ends the current step and prepares the next one.
    Skip,
    /// Back to the first pomodoro, stopped.
    Reset,
}

/// Entries, in display order.
pub const MENU_ENTRIES: [MenuEntry; 2] = [MenuEntry::Skip, MenuEntry::Reset];

/// An open menu: selected entry and time of the last input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Menu {
    selected: usize,
    last_input: Instant,
}

impl Menu {
    /// Menu opened at `now` on its first entry.
    #[must_use]
    pub fn open(now: Instant) -> Self {
        Self {
            selected: 0,
            last_input: now,
        }
    }

    /// Selected entry.
    #[must_use]
    pub fn selected(&self) -> MenuEntry {
        MENU_ENTRIES[self.selected]
    }

    /// Moves the selection by `ticks` entries (positive: down), stopping at
    /// the first and last ones.
    pub fn rotate(&mut self, ticks: i32, now: Instant) {
        let last = MENU_ENTRIES.len() - 1;
        let steps = usize::try_from(ticks.unsigned_abs()).unwrap_or(usize::MAX);
        self.selected = if ticks >= 0 {
            self.selected.saturating_add(steps).min(last)
        } else {
            self.selected.saturating_sub(steps)
        };
        self.last_input = now;
    }

    /// Whether the menu went [`MENU_TIMEOUT`] without input at `now`.
    #[must_use]
    pub fn is_expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_input) >= MENU_TIMEOUT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_moves_the_selection_within_the_entries() {
        let now = Instant::now();
        let mut menu = Menu::open(now);
        assert_eq!(menu.selected(), MenuEntry::Skip);
        menu.rotate(1, now);
        assert_eq!(menu.selected(), MenuEntry::Reset);
        menu.rotate(5, now);
        assert_eq!(menu.selected(), MenuEntry::Reset);
        menu.rotate(-1, now);
        assert_eq!(menu.selected(), MenuEntry::Skip);
        menu.rotate(i32::MIN, now);
        assert_eq!(menu.selected(), MenuEntry::Skip);
        menu.rotate(i32::MAX, now);
        assert_eq!(menu.selected(), MenuEntry::Reset);
    }

    #[test]
    fn the_menu_expires_after_the_timeout_without_input() {
        let opened = Instant::now();
        let mut menu = Menu::open(opened);
        assert!(!menu.is_expired(opened + MENU_TIMEOUT - Duration::from_millis(1)));
        menu.rotate(1, opened + Duration::from_secs(3));
        assert!(!menu.is_expired(opened + MENU_TIMEOUT));
        assert!(menu.is_expired(opened + Duration::from_secs(3) + MENU_TIMEOUT));
    }
}
