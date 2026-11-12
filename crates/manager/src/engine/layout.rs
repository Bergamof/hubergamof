//! Placement of page items on the key grid, with vertical scrolling by rows.

use hubergamof_protocol::DeviceCapabilities;

/// Key grid of a device: items fill it row by row; a page with more rows than
/// the device scrolls, `row` being the first visible one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyGrid {
    columns: u32,
    rows: u32,
}

impl KeyGrid {
    pub(crate) fn new(capabilities: &DeviceCapabilities) -> Self {
        Self {
            // A device always has at least one column; guard the divisions.
            columns: u32::from(capabilities.key_columns).max(1),
            rows: u32::from(capabilities.key_rows),
        }
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
        let visible_rows = self.rows.max(1);
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
        let grid_row = row + key / self.columns;
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
    fn keys_map_to_items_of_the_visible_rows() {
        let grid = grid(4, 2);
        assert_eq!(grid.item_index(0, 0), 0);
        assert_eq!(grid.item_index(5, 0), 5);
        assert_eq!(grid.item_index(0, 1), 4);
        assert_eq!(grid.item_index(7, 2), 15);
    }
}
