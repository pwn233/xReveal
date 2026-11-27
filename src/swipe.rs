//! The Drawer swipe (#141, docs/spec/v1.md §2.3): a sideways two-finger swipe on a trackpad pulls
//! the Settings drawer in, and the opposite swipe pushes it away.
//!
//! - By finger direction, not by the natural-scrolling setting: fingers moving away from the
//!   drawer's side open it.
//! - It fires once per gesture, the moment the fingers have moved [`THRESHOLD`] sideways, mostly
//!   sideways (at least twice the up-down travel), without waiting for them to lift.
//! - Travel after the fingers lift (momentum) never counts: only moves between a gesture's start
//!   and its end do.
//! - Only precise (trackpad, Magic Mouse) scrolling: a mouse wheel's line steps never count.

use gpui_kit::TouchPhase;

/// How far the fingers travel sideways before the swipe fires, in logical pixels.
pub const THRESHOLD: f32 = 80.;

/// Which way the fingers went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fingers {
    Left,
    Right,
}

/// One gesture's travel so far.
#[derive(Debug, Default)]
pub struct Swipe {
    tracking: bool,
    x: f32,
    y: f32,
    fired: bool,
}

impl Swipe {
    /// Feeds one scroll event: its phase and the fingers' movement (right and down positive).
    /// Returns which way the fingers swiped, once per gesture.
    pub fn feed(&mut self, phase: TouchPhase, fingers: (f32, f32)) -> Option<Fingers> {
        match phase {
            // A new gesture (also after one macOS cancelled without an end).
            TouchPhase::Started => *self = Swipe { tracking: true, ..Swipe::default() },
            TouchPhase::Ended | TouchPhase::Cancelled => self.tracking = false,
            TouchPhase::Moved if self.tracking && !self.fired => {
                self.x += fingers.0;
                self.y += fingers.1;
                if self.x.abs() >= THRESHOLD && self.x.abs() >= 2. * self.y.abs() {
                    self.fired = true;
                    return Some(if self.x < 0. { Fingers::Left } else { Fingers::Right });
                }
            }
            TouchPhase::Moved => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Fingers, Swipe, THRESHOLD};
    use gpui_kit::TouchPhase::{Ended, Moved, Started};

    #[test]
    fn fires_once_when_the_fingers_have_gone_far_enough_sideways() {
        let mut swipe = Swipe::default();
        assert_eq!(swipe.feed(Started, (0., 0.)), None);
        assert_eq!(swipe.feed(Moved, (-30., 2.)), None);
        assert_eq!(swipe.feed(Moved, (-60., 3.)), Some(Fingers::Left), "fires on crossing, before the fingers lift");
        assert_eq!(swipe.feed(Moved, (-200., 0.)), None, "once per gesture");
        assert_eq!(swipe.feed(Ended, (0., 0.)), None);
        swipe.feed(Started, (0., 0.));
        assert_eq!(swipe.feed(Moved, (THRESHOLD, 0.)), Some(Fingers::Right), "the next gesture fires again");
    }

    #[test]
    fn a_mostly_vertical_scroll_never_fires() {
        let mut swipe = Swipe::default();
        swipe.feed(Started, (0., 0.));
        assert_eq!(swipe.feed(Moved, (-90., 60.)), None, "sideways must be at least twice the up-down travel");
    }

    #[test]
    fn momentum_after_the_fingers_lift_never_counts() {
        let mut swipe = Swipe::default();
        swipe.feed(Started, (0., 0.));
        swipe.feed(Moved, (-40., 0.));
        swipe.feed(Ended, (0., 0.));
        // macOS momentum: more moves with no new start.
        assert_eq!(swipe.feed(Moved, (-200., 0.)), None);
    }

    #[test]
    fn moves_without_a_start_are_ignored() {
        // A mouse wheel, or a Magic Mouse's momentum, with no gesture begun.
        let mut swipe = Swipe::default();
        assert_eq!(swipe.feed(Moved, (-500., 0.)), None);
    }
}
