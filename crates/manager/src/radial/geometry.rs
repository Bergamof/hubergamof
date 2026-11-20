//! Where the petals of the radial menu are: which one a pointer position falls
//! on, and where the menu goes so that it stays on the screen.
//!
//! Pure arithmetic in pixels, with the y axis pointing down like a screen's.

use std::f64::consts::{FRAC_PI_2, TAU};

/// A position or an offset, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// Distance to the right.
    pub x: f64,
    /// Distance downwards.
    pub y: f64,
}

impl Point {
    /// A point at (`x`, `y`).
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Size of the screen, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// The ring the petals are cut from, and what keeps them apart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ring {
    /// Radius of the hole in the middle: the pointer is on no petal there.
    pub inner: f64,
    /// Radius of the outer edge.
    pub outer: f64,
    /// Width of the gap between two petals, the same all along their sides
    /// (the sides are parallel, not radial).
    pub gap: f64,
}

/// The petals of a menu with `count` entries, the first one centred on the top
/// and the others following clockwise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    count: usize,
    ring: Ring,
}

impl Layout {
    /// Petals for `count` entries on `ring`; `None` for no entry.
    #[must_use]
    pub fn new(count: usize, ring: Ring) -> Option<Self> {
        (count > 0).then_some(Self { count, ring })
    }

    /// The ring the petals are cut from.
    #[must_use]
    pub const fn ring(&self) -> Ring {
        self.ring
    }

    /// The petal `offset` (from the middle of the menu) is on. `None` in the
    /// hole, past the edge, and in a gap: the void selects nothing.
    #[must_use]
    pub fn petal_at(&self, offset: Point) -> Option<usize> {
        let radius = offset.x.hypot(offset.y);
        if radius < self.ring.inner || radius > self.ring.outer {
            return None;
        }
        let step = TAU / count_as_f64(self.count);
        // Clockwise from the top; the first petal is centred there, so the
        // sectors start half a step before it.
        let angle = (offset.x.atan2(-offset.y) + step / 2.0).rem_euclid(TAU);
        let petal = floor_index(angle / step).min(self.count - 1);
        let from_side = angle - count_as_f64(petal) * step;
        let nearest_side = from_side.min(step - from_side);
        // A single petal has no side to keep away from.
        let gap_applies = self.count > 1 && nearest_side < FRAC_PI_2;
        let distance_to_side = radius * nearest_side.sin();
        (!gap_applies || distance_to_side >= self.ring.gap / 2.0).then_some(petal)
    }
}

/// Where to centre a menu of radius `radius` so that it fits on `screen`, the
/// pointer being at `wanted`. The pointer is then put in the middle of the
/// menu. A screen too small for the menu has it centred.
#[must_use]
pub fn fit_on_screen(wanted: Point, radius: f64, screen: Size) -> Point {
    Point::new(
        fit_axis(wanted.x, radius, screen.width),
        fit_axis(wanted.y, radius, screen.height),
    )
}

fn fit_axis(wanted: f64, radius: f64, length: f64) -> f64 {
    if length < 2.0 * radius {
        length / 2.0
    } else {
        wanted.clamp(radius, length - radius)
    }
}

/// Entries are counted in the single digits: the conversions never lose a bit.
#[allow(clippy::cast_precision_loss)]
const fn count_as_f64(count: usize) -> f64 {
    count as f64
}

/// `value` is a sector number of the ring: small and not negative.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn floor_index(value: f64) -> usize {
    value.floor() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    const RING: Ring = Ring {
        inner: 40.0,
        outer: 140.0,
        gap: 8.0,
    };

    fn layout(count: usize) -> Layout {
        Layout::new(count, RING).unwrap()
    }

    #[test]
    fn no_entry_has_no_layout() {
        assert_eq!(Layout::new(0, RING), None);
    }

    #[test]
    fn petals_follow_clockwise_from_the_top() {
        let menu = layout(4);
        assert_eq!(menu.petal_at(Point::new(0.0, -90.0)), Some(0), "up");
        assert_eq!(menu.petal_at(Point::new(90.0, 0.0)), Some(1), "right");
        assert_eq!(menu.petal_at(Point::new(0.0, 90.0)), Some(2), "down");
        assert_eq!(menu.petal_at(Point::new(-90.0, 0.0)), Some(3), "left");
    }

    #[test]
    fn the_hole_and_the_outside_select_nothing() {
        let menu = layout(4);
        assert_eq!(menu.petal_at(Point::new(0.0, 0.0)), None);
        assert_eq!(menu.petal_at(Point::new(0.0, -39.0)), None);
        assert_eq!(menu.petal_at(Point::new(0.0, -141.0)), None);
        assert_eq!(menu.petal_at(Point::new(0.0, -140.0)), Some(0), "the edge");
    }

    #[test]
    fn a_gap_selects_nothing_and_it_is_as_wide_all_along_the_side() {
        let menu = layout(4);
        // The side between the first and the second petal is the diagonal.
        for radius in [60.0_f64, 100.0, 135.0] {
            let along = radius / 2.0_f64.sqrt();
            // Gap of 8: 4 on each side of the diagonal.
            let inside = |away: f64| {
                let toward_first = Point::new(
                    along - away / 2.0_f64.sqrt(),
                    -along - away / 2.0_f64.sqrt(),
                );
                menu.petal_at(toward_first)
            };
            assert_eq!(inside(1.0), None, "in the gap at radius {radius}");
            assert_eq!(inside(3.9), None, "still in the gap at radius {radius}");
        }
    }

    #[test]
    fn just_past_the_gap_is_the_petal_on_that_side() {
        let menu = layout(4);
        let diagonal = 100.0 / 2.0_f64.sqrt();
        let step = 5.0 / 2.0_f64.sqrt();
        // 5 away from the diagonal, on the side of the second petal.
        assert_eq!(
            menu.petal_at(Point::new(diagonal + step, -diagonal + step)),
            Some(1)
        );
        assert_eq!(
            menu.petal_at(Point::new(diagonal - step, -diagonal - step)),
            Some(0)
        );
    }

    #[test]
    fn a_single_petal_is_the_whole_ring() {
        let menu = layout(1);
        for point in [
            Point::new(0.0, -90.0),
            Point::new(90.0, 0.0),
            Point::new(0.0, 90.0),
            Point::new(-90.0, 0.0),
        ] {
            assert_eq!(menu.petal_at(point), Some(0));
        }
    }

    #[test]
    fn three_petals_are_a_third_of_the_circle_each() {
        let menu = layout(3);
        assert_eq!(menu.petal_at(Point::new(0.0, -90.0)), Some(0));
        // Down and to the right is the second petal, down and to the left the third.
        assert_eq!(menu.petal_at(Point::new(60.0, 50.0)), Some(1));
        assert_eq!(menu.petal_at(Point::new(-60.0, 50.0)), Some(2));
    }

    const SCREEN: Size = Size {
        width: 1920.0,
        height: 1080.0,
    };

    #[test]
    fn a_menu_in_the_middle_of_the_screen_stays_where_the_pointer_is() {
        let wanted = Point::new(900.0, 500.0);
        assert_eq!(fit_on_screen(wanted, 150.0, SCREEN), wanted);
    }

    #[test]
    fn a_menu_near_an_edge_is_pushed_back_in() {
        assert_eq!(
            fit_on_screen(Point::new(10.0, 1075.0), 150.0, SCREEN),
            Point::new(150.0, 930.0)
        );
        assert_eq!(
            fit_on_screen(Point::new(1915.0, 5.0), 150.0, SCREEN),
            Point::new(1770.0, 150.0)
        );
    }

    #[test]
    fn a_screen_too_small_for_the_menu_has_it_centred() {
        let small = Size {
            width: 200.0,
            height: 1080.0,
        };
        assert_eq!(
            fit_on_screen(Point::new(10.0, 500.0), 150.0, small),
            Point::new(100.0, 500.0)
        );
    }
}
