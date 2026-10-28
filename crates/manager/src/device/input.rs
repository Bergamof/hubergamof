//! Normalised input events, independent of the hardware library.

/// A point on the touch strip, in strip pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TouchPoint {
    /// Horizontal position, from the left edge.
    pub x: u16,
    /// Vertical position, from the top edge.
    pub y: u16,
}

/// Input produced by a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputEvent {
    /// Key `index` (row-major) was pressed.
    KeyDown(u8),
    /// Key `index` was released.
    KeyUp(u8),
    /// Dial was turned.
    DialRotate {
        /// Dial index, from the left.
        index: u8,
        /// Rotation steps, positive clockwise.
        ticks: i32,
    },
    /// Dial `index` was pressed.
    DialDown(u8),
    /// Dial `index` was released.
    DialUp(u8),
    /// Touch strip was tapped.
    TouchTap {
        /// Tap position.
        point: TouchPoint,
        /// Whether it was a long press.
        long: bool,
    },
    /// Finger swiped across the touch strip.
    TouchSwipe {
        /// Start of the swipe.
        from: TouchPoint,
        /// End of the swipe.
        to: TouchPoint,
    },
    /// The device was unplugged or closed; no event follows.
    Disconnected,
}
