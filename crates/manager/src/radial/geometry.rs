//! Where the petals of the radial menu are: which one a pointer position falls
//! on, and where the menu goes so that it stays on the screen.
//!
//! Pure arithmetic in pixels, with the y axis pointing down like a screen's.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use hubergamof_module_api::MAX_RADIAL_ITEMS;

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

/// The ring of the menu as it is drawn: 150 px from the middle to the edge, a
/// hole of 44 px, 8 px between two petals.
pub const DEFAULT_RING: Ring = Ring {
    inner: 44.0,
    outer: 150.0,
    gap: 8.0,
};

/// Radius of the inner edge of the arc of actions that opens in front of a petal.
pub const ARC_INNER: f64 = 160.0;

const _: () = assert!(
    ARC_INNER > DEFAULT_RING.outer,
    "the arc opens beyond the petals"
);

/// Radius of the outer edge of the arc of actions.
pub const ARC_OUTER: f64 = 214.0;

/// Width of the gap between two slices of the arc, parallel like the one
/// between two petals.
pub const ARC_GAP: f64 = 4.0;

/// Widest a slice of the arc is, in radians (15 degrees): a few actions on a
/// wide petal stay together rather than stretch over it.
pub const MAX_SLICE_ANGLE: f64 = 15.0 * PI / 180.0;

/// Radius the menu is kept on the screen with: the arc, not only the ring, is
/// to stay in sight near an edge.
pub const FIT_RADIUS: f64 = 230.0;

/// What the pointer is on: the body of a petal, or one of the actions of the
/// arc in front of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// The body of this petal: its primary action.
    Petal(usize),
    /// A slice of the arc of a petal.
    Item {
        /// The petal the arc belongs to.
        petal: usize,
        /// Position of the action, in the order the module gave them.
        item: usize,
    },
}

impl Hit {
    /// The petal the pointer is on, or in front of.
    #[must_use]
    pub const fn petal(self) -> usize {
        match self {
            Self::Petal(petal) | Self::Item { petal, .. } => petal,
        }
    }

    /// The action of the arc the pointer is on, if it is on one.
    #[must_use]
    pub const fn item(self) -> Option<usize> {
        match self {
            Self::Petal(_) => None,
            Self::Item { item, .. } => Some(item),
        }
    }
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

    /// Number of petals.
    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
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

    /// What `offset` is on, given how many actions each petal shows
    /// (`item_counts`, one per petal; a missing one is none) and which petal is
    /// `active`, the one the pointer was on.
    ///
    /// The body of a petal is as in [`Self::petal_at`]. The active petal, when
    /// it has actions, also owns a corridor: everything in its angular sector
    /// (the gaps between petals included) out to the edge of the arc, so that
    /// the pointer crosses the space between the body and the arc without
    /// losing the petal. On a slice it is [`Hit::Item`]. Past the arc, outside
    /// the sector, or when the petal has no action, there is nothing.
    #[must_use]
    pub fn hit_at(
        &self,
        offset: Point,
        item_counts: &[usize],
        active: Option<usize>,
    ) -> Option<Hit> {
        if let Some(petal) = self.petal_at(offset) {
            return Some(Hit::Petal(petal));
        }
        let radius = offset.x.hypot(offset.y);
        if radius <= self.ring.outer || radius > ARC_OUTER {
            return None;
        }
        let petal = active?;
        let items = shown_items(item_counts.get(petal).copied().unwrap_or(0));
        if petal >= self.count || items == 0 {
            return None;
        }
        let step = self.step();
        // Signed angle from the axis of the petal, in [-PI, PI).
        let from_axis =
            (offset.x.atan2(-offset.y) - count_as_f64(petal) * step + PI).rem_euclid(TAU) - PI;
        if from_axis.abs() > step / 2.0 {
            return None;
        }
        let width = self.slice_width(items);
        let from_first_side = from_axis + count_as_f64(items) * width / 2.0;
        let position = from_first_side / width;
        if !(0.0..count_as_f64(items)).contains(&position) {
            return Some(Hit::Petal(petal));
        }
        let item = floor_index(position);
        let from_side = from_first_side - count_as_f64(item) * width;
        let nearest_side = from_side.min(width - from_side);
        let on_slice = radius >= ARC_INNER && radius * nearest_side.sin() >= ARC_GAP / 2.0;
        Some(if on_slice {
            Hit::Item { petal, item }
        } else {
            Hit::Petal(petal)
        })
    }

    /// Angle of a petal, in radians.
    fn step(&self) -> f64 {
        TAU / count_as_f64(self.count)
    }

    /// Angle of a slice of the arc of a petal that shows `items` actions: the
    /// sector of the petal shared between them, at most [`MAX_SLICE_ANGLE`].
    #[must_use]
    pub fn slice_width(&self, items: usize) -> f64 {
        let items = shown_items(items).max(1);
        MAX_SLICE_ANGLE.min(self.step() / count_as_f64(items))
    }

    /// The middle line of slice `item` of the arc of `petal` (of `items`
    /// slices), as the angle clockwise from the top.
    #[must_use]
    pub fn slice_angle(&self, petal: usize, item: usize, items: usize) -> f64 {
        let items = shown_items(items);
        let offset = count_as_f64(item) - (count_as_f64(items) - 1.0) / 2.0;
        count_as_f64(petal) * self.step() + offset * self.slice_width(items)
    }

    /// The shape of slice `item` of the arc of `petal` when it has `items`
    /// slices; `None` for a slice that does not exist.
    #[must_use]
    pub fn slice_outline(&self, petal: usize, item: usize, items: usize) -> Option<Outline> {
        let items = shown_items(items);
        if petal >= self.count || item >= items {
            return None;
        }
        let middle = self.slice_angle(petal, item, items);
        let half_width = self.slice_width(items) / 2.0;
        Some(sector_outline(
            middle - half_width,
            middle + half_width,
            ARC_INNER,
            ARC_OUTER,
            ARC_GAP,
        ))
    }
}

/// How many actions of a petal are drawn: the arc has room for
/// [`MAX_RADIAL_ITEMS`].
fn shown_items(count: usize) -> usize {
    count.min(MAX_RADIAL_ITEMS)
}

/// The four corners of one petal, from the middle of the menu: the sides are
/// the straight lines between the inner and the outer corners, the edges are
/// arcs of the inner and outer circles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outline {
    /// Inner corner of the side the petal starts at (its anticlockwise side).
    pub inner_start: Point,
    /// Outer corner of the same side.
    pub outer_start: Point,
    /// Outer corner of the side the petal ends at (its clockwise side).
    pub outer_end: Point,
    /// Inner corner of the same side.
    pub inner_end: Point,
}

impl Layout {
    /// The shape of petal `petal`, to draw it. `None` for a menu of a single
    /// petal (a whole ring, no sides) and for a petal that does not exist.
    #[must_use]
    pub fn outline(&self, petal: usize) -> Option<Outline> {
        self.band_outline(petal, self.ring.inner, self.ring.outer)
    }

    /// The shape of the band of petal `petal` between the radii `inner` and
    /// `outer`, with the same sides as the petal itself: the thin edge drawn
    /// along the outside of a petal that has actions. `None` as for
    /// [`Self::outline`].
    #[must_use]
    pub fn band_outline(&self, petal: usize, inner: f64, outer: f64) -> Option<Outline> {
        if self.count < 2 || petal >= self.count {
            return None;
        }
        let step = self.step();
        let start = (count_as_f64(petal) - 0.5) * step;
        Some(sector_outline(
            start,
            start + step,
            inner,
            outer,
            self.ring.gap,
        ))
    }
}

/// The corners of the sector between the angles `start` and `end` (clockwise
/// from the top) and the radii `inner` and `outer`, each side pulled half of
/// `gap` into the sector, along its normal.
fn sector_outline(start: f64, end: f64, inner: f64, outer: f64, gap: f64) -> Outline {
    let half_gap = gap / 2.0;
    let corner = |angle: f64, radius: f64, normal_sign: f64| {
        let along = (radius * radius - half_gap * half_gap).max(0.0).sqrt();
        let (sin, cos) = angle.sin_cos();
        // `outward` is the direction of the side, `clockwise` its normal.
        let outward = Point::new(sin, -cos);
        let clockwise = Point::new(cos, sin);
        Point::new(
            along * outward.x + normal_sign * half_gap * clockwise.x,
            along * outward.y + normal_sign * half_gap * clockwise.y,
        )
    };
    Outline {
        inner_start: corner(start, inner, 1.0),
        outer_start: corner(start, outer, 1.0),
        outer_end: corner(end, outer, -1.0),
        inner_end: corner(end, inner, -1.0),
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

    #[test]
    fn the_corners_of_a_petal_are_on_its_side_of_the_gap() {
        let menu = layout(4);
        let outline = menu.outline(0).unwrap();
        for corner in [
            outline.inner_start,
            outline.outer_start,
            outline.outer_end,
            outline.inner_end,
        ] {
            let radius = corner.x.hypot(corner.y);
            assert!(
                (radius - RING.inner).abs() < 1e-9 || (radius - RING.outer).abs() < 1e-9,
                "on the inner or the outer circle"
            );
        }
        // The side is parallel to the diagonal, half a gap away from it.
        let distance = |corner: Point| (corner.x + corner.y).abs() / 2.0_f64.sqrt();
        assert!((distance(outline.outer_end) - RING.gap / 2.0).abs() < 1e-9);
        assert!((distance(outline.inner_end) - RING.gap / 2.0).abs() < 1e-9);
        // One pixel from the middle of the side along its normal (the side is at
        // 45 degrees): inside the petal on one side, in the gap on the other.
        let middle = Point::new(
            f64::midpoint(outline.outer_end.x, outline.inner_end.x),
            f64::midpoint(outline.outer_end.y, outline.inner_end.y),
        );
        let normal = 1.0 / 2.0_f64.sqrt();
        let from_middle = |toward_gap: f64| {
            Point::new(
                middle.x + toward_gap * normal,
                middle.y + toward_gap * normal,
            )
        };
        assert_eq!(menu.petal_at(from_middle(-1.0)), Some(0));
        assert_eq!(menu.petal_at(from_middle(1.0)), None);
    }

    #[test]
    fn a_single_petal_or_a_missing_one_has_no_outline() {
        assert_eq!(layout(1).outline(0), None);
        assert_eq!(layout(4).outline(4), None);
    }

    /// The point `radius` from the middle, `angle` radians clockwise from the top.
    fn at(angle: f64, radius: f64) -> Point {
        Point::new(radius * angle.sin(), -radius * angle.cos())
    }

    const SLICE_MIDDLE: f64 = 187.0;

    /// `count` petals of the real ring, as the menu has them.
    fn menu(count: usize) -> Layout {
        Layout::new(count, DEFAULT_RING).unwrap()
    }

    fn counts(petals: usize, items: usize) -> Vec<usize> {
        vec![items; petals]
    }

    #[test]
    fn without_actions_the_hit_is_the_petal_of_before_everywhere() {
        for count in 1..=8 {
            let menu = menu(count);
            for step in 0..720 {
                let angle = f64::from(step) * TAU / 720.0;
                for radius in [30.0, 44.0, 100.0, 150.0, 155.0, 190.0, 214.0, 230.0] {
                    let offset = at(angle, radius);
                    assert_eq!(
                        menu.hit_at(offset, &counts(count, 0), Some(0)),
                        menu.petal_at(offset).map(Hit::Petal),
                        "{count} petals, angle {step}/720, radius {radius}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_body_of_any_petal_is_a_petal_hit_even_when_another_is_active() {
        let menu = menu(4);
        assert_eq!(
            menu.hit_at(at(TAU / 4.0, 100.0), &counts(4, 3), Some(0)),
            Some(Hit::Petal(1))
        );
    }

    #[test]
    fn the_corridor_keeps_the_active_petal_from_its_body_to_the_edge_of_the_arc() {
        let menu = menu(4);
        let items = counts(4, 3);
        // The sector of the first petal is 45 degrees to each side of the top.
        for degrees in [-44.0_f64, -30.0, 0.0, 30.0, 44.0] {
            for radius in [150.5, 155.0, 159.9, 190.0, 214.0] {
                let offset = at(degrees.to_radians(), radius);
                let hit = menu.hit_at(offset, &items, Some(0));
                assert!(
                    matches!(hit, Some(Hit::Petal(0) | Hit::Item { petal: 0, .. })),
                    "{degrees} degrees, radius {radius}: {hit:?}"
                );
            }
        }
        assert_eq!(
            menu.hit_at(at(0.0, 155.0), &items, Some(0)),
            Some(Hit::Petal(0)),
            "between the body and the arc"
        );
    }

    #[test]
    fn the_corridor_ends_at_the_arc_and_at_the_sector() {
        let menu = menu(4);
        let items = counts(4, 3);
        assert_eq!(menu.hit_at(at(0.0, 214.5), &items, Some(0)), None, "past");
        assert_eq!(menu.hit_at(at(0.0, 300.0), &items, Some(0)), None);
        // Past the diagonal that ends the sector, outside the ring.
        assert_eq!(
            menu.hit_at(at(46.0_f64.to_radians(), 190.0), &items, Some(0)),
            None
        );
        assert_eq!(
            menu.hit_at(at((-46.0_f64).to_radians(), 155.0), &items, Some(0)),
            None
        );
    }

    #[test]
    fn the_corridor_belongs_to_the_active_petal_only() {
        let menu = menu(4);
        let items = counts(4, 3);
        assert_eq!(menu.hit_at(at(0.0, 190.0), &items, None), None, "no petal");
        assert_eq!(
            menu.hit_at(at(0.0, 190.0), &items, Some(1)),
            None,
            "another petal"
        );
        assert_eq!(
            menu.hit_at(at(0.0, 190.0), &items, Some(9)),
            None,
            "a petal that does not exist"
        );
    }

    #[test]
    fn a_petal_without_actions_has_no_corridor() {
        let menu = menu(4);
        assert_eq!(menu.hit_at(at(0.0, 190.0), &[0, 3, 3, 3], Some(0)), None);
        assert_eq!(menu.hit_at(at(0.0, 155.0), &[0, 3, 3, 3], Some(0)), None);
        assert_eq!(menu.hit_at(at(0.0, 190.0), &[], Some(0)), None, "no counts");
    }

    #[test]
    fn every_slice_is_hit_in_its_middle_for_one_to_six_actions_on_every_menu() {
        for petals in 1..=8 {
            let menu = menu(petals);
            for items in 1..=6 {
                for petal in 0..petals {
                    for item in 0..items {
                        let angle = menu.slice_angle(petal, item, items);
                        let hit = menu.hit_at(
                            at(angle, SLICE_MIDDLE),
                            &counts(petals, items),
                            Some(petal),
                        );
                        assert_eq!(
                            hit,
                            Some(Hit::Item { petal, item }),
                            "{petals} petals, {items} actions, petal {petal}, action {item}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_edges_of_a_slice_are_the_arc_and_a_gap_of_four_pixels_between_slices() {
        let menu = menu(4);
        let items = counts(4, 3);
        let slice = |radius: f64, angle: f64| menu.hit_at(at(angle, radius), &items, Some(0));
        let width = menu.slice_width(3);
        // 159 is the corridor, 160 and 214 are the slice.
        assert_eq!(slice(159.0, menu.slice_angle(0, 1, 3)), Some(Hit::Petal(0)));
        assert_eq!(
            slice(160.0, menu.slice_angle(0, 1, 3)),
            Some(Hit::Item { petal: 0, item: 1 })
        );
        assert_eq!(
            slice(214.0, menu.slice_angle(0, 1, 3)),
            Some(Hit::Item { petal: 0, item: 1 })
        );
        // The gap between the first and the second slice is parallel sides 4
        // apart: nothing but the corridor inside it, at any radius.
        let side = menu.slice_angle(0, 0, 3) + width / 2.0;
        for radius in [165.0_f64, 190.0, 213.0] {
            let nearly = |away: f64| slice(radius, side + (away / radius).asin());
            assert_eq!(nearly(0.0), Some(Hit::Petal(0)), "radius {radius}");
            assert_eq!(nearly(1.9), Some(Hit::Petal(0)), "radius {radius}");
            assert_eq!(nearly(-1.9), Some(Hit::Petal(0)), "radius {radius}");
            assert_eq!(
                nearly(2.2),
                Some(Hit::Item { petal: 0, item: 1 }),
                "radius {radius}"
            );
            assert_eq!(
                nearly(-2.2),
                Some(Hit::Item { petal: 0, item: 0 }),
                "radius {radius}"
            );
        }
    }

    #[test]
    fn beside_the_slices_but_in_the_sector_is_still_the_petal() {
        // Three slices of 15 degrees fill 45 of the 90 of a petal of four.
        let menu = menu(4);
        let items = counts(4, 3);
        for degrees in [-40.0_f64, -30.0, 30.0, 40.0] {
            assert_eq!(
                menu.hit_at(at(degrees.to_radians(), 190.0), &items, Some(0)),
                Some(Hit::Petal(0)),
                "{degrees} degrees"
            );
        }
    }

    #[test]
    fn a_slice_is_a_fifth_of_a_quarter_at_most_and_does_not_stretch() {
        let degrees = |petals: usize, items: usize| menu(petals).slice_width(items).to_degrees();
        let approx = |left: f64, right: f64| (left - right).abs() < 1e-9;
        assert!(approx(degrees(4, 6), 15.0), "six on a quarter");
        assert!(approx(degrees(4, 3), 15.0), "three do not stretch");
        assert!(approx(degrees(4, 1), 15.0), "nor one");
        assert!(approx(degrees(8, 3), 15.0), "three on an eighth");
        assert!(approx(degrees(8, 6), 7.5), "six on an eighth share it");
        assert!(approx(degrees(1, 6), 15.0), "a single petal is wide");
        assert!(approx(degrees(4, 9), 15.0), "extra actions are not drawn");
    }

    #[test]
    fn the_slices_are_centred_on_the_axis_of_the_petal() {
        let menu = menu(4);
        let axis = TAU / 4.0;
        let angle = |item, items| menu.slice_angle(1, item, items);
        assert!((angle(0, 1) - axis).abs() < 1e-12);
        assert!((angle(1, 3) - axis).abs() < 1e-12, "the middle of three");
        let width = menu.slice_width(2);
        assert!((angle(0, 2) - (axis - width / 2.0)).abs() < 1e-12);
        assert!((angle(1, 2) - (axis + width / 2.0)).abs() < 1e-12);
    }

    #[test]
    fn extra_actions_beyond_the_sixth_are_not_slices() {
        let menu = menu(4);
        assert!(menu.slice_outline(0, 5, 9).is_some());
        assert_eq!(menu.slice_outline(0, 6, 9), None);
        assert_eq!(
            menu.hit_at(at(menu.slice_angle(0, 5, 9), 190.0), &counts(4, 9), Some(0)),
            Some(Hit::Item { petal: 0, item: 5 })
        );
    }

    #[test]
    fn the_corners_of_a_slice_are_on_the_arc_and_the_gap_is_four_pixels() {
        let menu = menu(4);
        let first = menu.slice_outline(0, 0, 3).unwrap();
        let second = menu.slice_outline(0, 1, 3).unwrap();
        for corner in [first.inner_start, first.inner_end, second.inner_start] {
            let radius = corner.x.hypot(corner.y);
            assert!((radius - ARC_INNER).abs() < 1e-9, "inner: {radius}");
        }
        for corner in [first.outer_start, first.outer_end, second.outer_start] {
            let radius = corner.x.hypot(corner.y);
            assert!((radius - ARC_OUTER).abs() < 1e-9, "outer: {radius}");
        }
        // The sides facing each other are 4 pixels apart, all along.
        let apart = |near: Point, far: Point| (near.x - far.x).hypot(near.y - far.y);
        assert!((apart(first.inner_end, second.inner_start) - ARC_GAP).abs() < 0.05);
        assert!((apart(first.outer_end, second.outer_start) - ARC_GAP).abs() < 0.05);
        assert_eq!(menu.slice_outline(0, 3, 3), None, "no fourth slice");
        assert_eq!(menu.slice_outline(4, 0, 3), None, "no fifth petal");
    }

    #[test]
    fn the_band_along_a_petal_has_the_sides_of_the_petal() {
        let menu = menu(4);
        let petal = menu.outline(1).unwrap();
        let band = menu.band_outline(1, 150.0, 154.0).unwrap();
        // The outer corners of the petal are the inner corners of the band.
        assert!((petal.outer_start.x - band.inner_start.x).abs() < 1e-9);
        assert!((petal.outer_end.y - band.inner_end.y).abs() < 1e-9);
        assert_eq!(
            menu.band_outline(0, DEFAULT_RING.inner, DEFAULT_RING.outer),
            menu.outline(0)
        );
        assert_eq!(self::menu(1).band_outline(0, 150.0, 154.0), None);
        assert_eq!(menu.band_outline(9, 150.0, 154.0), None);
    }

    #[test]
    fn a_hit_names_its_petal_and_perhaps_its_action() {
        assert_eq!(Hit::Petal(2).petal(), 2);
        assert_eq!(Hit::Petal(2).item(), None);
        let item = Hit::Item { petal: 3, item: 1 };
        assert_eq!((item.petal(), item.item()), (3, Some(1)));
    }

    #[test]
    fn the_arc_stays_on_the_screen_with_the_fit_radius() {
        assert_eq!(
            fit_on_screen(Point::new(10.0, 540.0), FIT_RADIUS, SCREEN),
            Point::new(230.0, 540.0)
        );
        const { assert!(FIT_RADIUS > ARC_OUTER) };
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

    /// The corners of every petal of menus of 2 to 8 entries, and the ring: what
    /// `ui/src/lib/deck/radial-geometry.ts` must draw. Regenerate the file with
    /// `UPDATE_RADIAL_FIXTURE=1 cargo test -p hubergamof radial_geometry_fixture`.
    #[test]
    fn the_geometry_of_the_ui_preview_is_the_one_of_the_menu() {
        use serde_json::{Value, json};
        let round = |value: f64| (value * 1e6).round() / 1e6;
        let point = |p: Point| json!([round(p.x), round(p.y)]);
        let petals: Vec<Value> = (2..=8)
            .map(|count| {
                let layout = Layout::new(count, DEFAULT_RING).unwrap();
                let outlines: Vec<Value> = (0..count)
                    .map(|petal| {
                        let outline = layout.outline(petal).unwrap();
                        json!({
                            "innerStart": point(outline.inner_start),
                            "outerStart": point(outline.outer_start),
                            "outerEnd": point(outline.outer_end),
                            "innerEnd": point(outline.inner_end),
                        })
                    })
                    .collect();
                json!({ "count": count, "outlines": outlines })
            })
            .collect();
        let generated = json!({
            "ring": {
                "inner": DEFAULT_RING.inner,
                "outer": DEFAULT_RING.outer,
                "gap": DEFAULT_RING.gap,
            },
            "menus": petals,
        });
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/src/lib/deck/radial-geometry.fixture.json");
        if std::env::var_os("UPDATE_RADIAL_FIXTURE").is_some() {
            std::fs::write(
                &path,
                serde_json::to_string_pretty(&generated).unwrap() + "\n",
            )
            .unwrap();
        }
        let committed: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            committed, generated,
            "the menu changed: regenerate the fixture (see this test)"
        );
    }
}
