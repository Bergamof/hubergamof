//! Placement of page items on the key grid, with vertical scrolling by rows.

use bergadeck_protocol::DeviceCapabilities;

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
    use bergadeck_protocol::ImageSize;

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
