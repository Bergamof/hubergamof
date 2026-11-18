//! Placement of page items on the key grid, with vertical scrolling by rows
//! below the rows a page pins.

use hubergamof_protocol::DeviceCapabilities;

/// Key grid of a device: items fill it row by row; a page with more rows than
/// the device scrolls, `row` being the first visible one of the rows that are
/// not pinned. The first `pinned_rows` rows never scroll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyGrid {
    columns: u32,
    rows: u32,
    pinned_rows: u32,
}

impl KeyGrid {
    pub(crate) fn new(capabilities: &DeviceCapabilities) -> Self {
        Self {
            // A device always has at least one column; guard the divisions.
            columns: u32::from(capabilities.key_columns).max(1),
            rows: u32::from(capabilities.key_rows),
            pinned_rows: 0,
        }
    }

    /// The same grid with its first rows pinned, as many as `pinned_items`
    /// items fill (the last one completed with blank keys). At least one row
    /// stays free to scroll, so a one-row device pins nothing.
    pub(crate) fn pinning(self, pinned_items: usize) -> Self {
        let wanted =
            u32::try_from(pinned_items.div_ceil(self.columns as usize)).unwrap_or(u32::MAX);
        Self {
            pinned_rows: wanted.min(self.rows.saturating_sub(1)),
            ..self
        }
    }

    /// Number of items the pinned rows hold, blank keys included.
    pub(crate) fn pinned_len(self) -> usize {
        (self.pinned_rows * self.columns) as usize
    }

    /// Number of rows taken by `items` items (at least one, possibly empty).
    pub(crate) fn row_count(self, items: usize) -> u32 {
        let items = u32::try_from(items).unwrap_or(u32::MAX);
        items.div_ceil(self.columns).max(1)
    }

    /// Highest first visible row for `items` items.
    pub(crate) fn max_row(self, items: usize) -> u32 {
        self.row_count(items).saturating_sub(self.rows)
    }

    /// `row` moved by `delta` rows, kept within `0..=max_row(items)`.
    pub(crate) fn scroll(self, row: u32, delta: i32, items: usize) -> u32 {
        let target = i64::from(row) + i64::from(delta);
        let clamped = target.clamp(0, i64::from(self.max_row(items)));
        u32::try_from(clamped).unwrap_or(0)
    }

    /// First visible row that shows item `index` of `items` items, moving
    /// `row` as little as possible.
    pub(crate) fn reveal(self, row: u32, index: usize, items: usize) -> u32 {
        let item_row = u32::try_from(index).unwrap_or(u32::MAX) / self.columns;
        if item_row < self.pinned_rows {
            return row.min(self.max_row(items));
        }
        // Rows of the scrolling area, which `row` counts from its start.
        let item_row = item_row - self.pinned_rows;
        let visible_rows = (self.rows - self.pinned_rows).max(1);
        let target = if item_row < row {
            item_row
        } else if item_row >= row + visible_rows {
            item_row + 1 - visible_rows
        } else {
            row
        };
        target.min(self.max_row(items))
    }

    /// Index of the item shown on `key` when `row` is the first visible row
    /// (whether or not such an item exists).
    pub(crate) fn item_index(self, key: u8, row: u32) -> usize {
        let key = u32::from(key);
        let key_row = key / self.columns;
        // A pinned row shows the same items whatever the scroll.
        let scrolled = if key_row < self.pinned_rows { 0 } else { row };
        let grid_row = scrolled + key_row;
        let index = grid_row * self.columns + key % self.columns;
        usize::try_from(index).unwrap_or(usize::MAX)
    }
}

#[cfg(test)]
mod tests {
    use hubergamof_protocol::ImageSize;

    use super::*;

    fn grid(columns: u8, rows: u8) -> KeyGrid {
        KeyGrid::new(&DeviceCapabilities {
            key_columns: columns,
            key_rows: rows,
            key_image_size: ImageSize {
                width: 72,
                height: 72,
            },
            dial_count: 0,
            touch_strip: None,
        })
    }

    #[test]
    fn row_count_rounds_up_and_is_at_least_one() {
        let grid = grid(4, 2);
        assert_eq!(grid.row_count(0), 1);
        assert_eq!(grid.row_count(4), 1);
        assert_eq!(grid.row_count(5), 2);
        assert_eq!(grid.row_count(9), 3);
    }

    #[test]
    fn revealing_an_item_moves_the_rows_as_little_as_possible() {
        let grid = grid(4, 2);
        // 20 items: 5 rows, the first visible one is at most 3.
        assert_eq!(grid.reveal(1, 5, 20), 1, "already visible");
        assert_eq!(grid.reveal(1, 11, 20), 1, "last visible row");
        assert_eq!(grid.reveal(1, 12, 20), 2, "just below");
        assert_eq!(grid.reveal(0, 19, 20), 3, "clamped to the last row");
        assert_eq!(grid.reveal(3, 0, 20), 0, "above");
        assert_eq!(grid.reveal(2, 100, 8), 0, "beyond the page");
    }

    #[test]
    fn scrolling_stays_within_the_page() {
        let grid = grid(4, 2);
        assert_eq!(grid.max_row(8), 0);
        assert_eq!(grid.max_row(13), 2);
        assert_eq!(grid.scroll(0, 1, 13), 1);
        assert_eq!(grid.scroll(1, 5, 13), 2);
        assert_eq!(grid.scroll(1, -3, 13), 0);
        assert_eq!(grid.scroll(0, 1, 8), 0);
        assert_eq!(grid.scroll(0, i32::MIN, 8), 0);
    }

    #[test]
    fn pinned_rows_stay_while_the_rest_scrolls() {
        // 4 columns, 3 rows, the first one pinned: 4 pinned items, 12 others.
        let grid = grid(4, 3).pinning(3);
        assert_eq!(grid.pinned_len(), 4);
        // The pinned row is item 0..4 at any scroll, the others follow `row`.
        assert_eq!(grid.item_index(2, 0), 2);
        assert_eq!(grid.item_index(2, 2), 2);
        assert_eq!(grid.item_index(4, 0), 4);
        assert_eq!(grid.item_index(4, 2), 12);
        assert_eq!(grid.item_index(11, 2), 19);
        // 16 items: 4 rows, 3 visible, so one scroll step.
        assert_eq!(grid.max_row(16), 1);
        assert_eq!(grid.scroll(0, 5, 16), 1);
    }

    #[test]
    fn revealing_counts_rows_from_the_scrolling_area() {
        let grid = grid(4, 3).pinning(4);
        // 24 items: 6 rows; the area has 5 rows, 2 visible at a time.
        assert_eq!(grid.reveal(0, 2, 24), 0, "a pinned item never scrolls");
        assert_eq!(grid.reveal(1, 2, 24), 1, "…and keeps the row as it is");
        assert_eq!(grid.reveal(0, 4, 24), 0, "first row of the area");
        assert_eq!(
            grid.reveal(0, 12, 24),
            1,
            "third row of the area: scrolls by one"
        );
        assert_eq!(grid.reveal(3, 5, 24), 0, "back up");
        assert_eq!(grid.reveal(0, 100, 24), 3, "clamped to the last row");
    }

    #[test]
    fn a_device_keeps_one_row_free_to_scroll() {
        assert_eq!(grid(4, 2).pinning(100).pinned_len(), 4);
        assert_eq!(grid(4, 1).pinning(1).pinned_len(), 0);
        assert_eq!(grid(4, 2).pinning(0).pinned_len(), 0);
        assert_eq!(grid(4, 3).pinning(5).pinned_len(), 8);
    }

    #[test]
    fn keys_map_to_items_of_the_visible_rows() {
        let grid = grid(4, 2);
        assert_eq!(grid.item_index(0, 0), 0);
        assert_eq!(grid.item_index(5, 0), 5);
        assert_eq!(grid.item_index(0, 1), 4);
        assert_eq!(grid.item_index(7, 2), 15);
    }
}
