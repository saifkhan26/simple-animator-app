//! The outline both lassos draw, the Lasso selection's and the Fill tool's.
//!
//! A drag draws it freehand, and letting go closes it. A tap starts a
//! polygon instead, a corner per tap; a tap on the first corner, a
//! double-click or Enter closes that. While corners are being tapped, a
//! press that drags adds a freehand stretch from its corner, so one shape
//! can have straight sides and curves.
//!
//! In document space and document units: callers turn the screen distances
//! that tell a tap from a drag into those.

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// An outline being drawn.
#[derive(Clone, Debug)]
pub struct TapLasso {
    /// Every point so far: corners, and freehand samples after them.
    pub pts: Vec<(f32, f32)>,
    /// Which of `pts` are tapped corners.
    pub corners: Vec<usize>,
    /// Tapped out corner by corner, not drawn in one drag.
    pub polygon: bool,
    /// The pen is down.
    pub pressed: bool,
    /// Where the pen went down.
    press: (f32, f32),
    /// The pen has gone far enough from `press` to be drawing.
    drawing: bool,
}

impl TapLasso {
    /// The pen down at `at`, starting an outline.
    pub fn start(at: (f32, f32)) -> Self {
        Self {
            pts: vec![at],
            corners: Vec::new(),
            polygon: false,
            pressed: true,
            press: at,
            drawing: false,
        }
    }

    /// Whether a press near the first corner would close the polygon.
    pub fn closable(&self) -> bool {
        self.polygon && self.pts.len() >= 3
    }

    /// A press on the polygon at `at`. Within `close` of the first corner,
    /// that closes it: `true`, and the caller [`finish`](Self::finish)es it.
    /// Anywhere else it puts a corner at `corner` (`at`, or `at` snapped).
    pub fn press(&mut self, at: (f32, f32), corner: (f32, f32), close: f32) -> bool {
        if self.closable() && dist(self.pts[0], at) <= close {
            return true;
        }
        self.pts.push(corner);
        self.corners.push(self.pts.len() - 1);
        self.pressed = true;
        self.press = corner;
        self.drawing = false;
        false
    }

    /// The pen, still down, at `at`. Until it has gone `tap` from where it
    /// went down it is a tap; after, it draws, a sample every `step`.
    pub fn drag(&mut self, at: (f32, f32), tap: f32, step: f32) {
        if !self.pressed || (!self.drawing && dist(self.press, at) < tap) {
            return;
        }
        self.drawing = true;
        if self.pts.last().map_or(true, |&p| dist(p, at) >= step) {
            self.pts.push(at);
        }
    }

    /// The pen came up. `true` when that finished a freehand outline, for
    /// the caller to [`finish`](Self::finish). A tap that started the
    /// outline makes it a polygon with its first corner there.
    pub fn release(&mut self) -> bool {
        self.pressed = false;
        if self.polygon {
            return false;
        }
        if self.drawing {
            return true;
        }
        self.polygon = true;
        self.pts.truncate(1);
        self.corners = vec![0];
        false
    }

    /// The closed outline, if it makes a shape: three distinct points or
    /// more. A double-click's second press lands on top of its first.
    pub fn finish(mut self) -> Option<Vec<(f32, f32)>> {
        self.pts.dedup_by(|a, b| dist(*a, *b) < 1e-3);
        (self.pts.len() >= 3).then_some(self.pts)
    }

    /// Take back the newest corner, and any freehand stretch drawn from it.
    /// `false` when that leaves nothing.
    pub fn back(&mut self) -> bool {
        match self.corners.pop() {
            Some(i) => self.pts.truncate(i),
            None => self.pts.clear(),
        }
        !self.pts.is_empty()
    }

    /// Whether the two newest corners sit within `near` of each other: a
    /// double-click, its second press on its first. egui counts any two
    /// quick clicks as one, so corners tapped fast would look the same.
    pub fn doubled(&self, near: f32) -> bool {
        match self.corners[..] {
            [.., a, b] => dist(self.pts[a], self.pts[b]) <= near,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A press and a release where it went down.
    fn tap(l: &mut Option<TapLasso>, at: (f32, f32)) -> Option<Vec<(f32, f32)>> {
        match l {
            None => *l = Some(TapLasso::start(at)),
            Some(t) => {
                if t.press(at, at, 5.0) {
                    return l.take().unwrap().finish();
                }
            }
        }
        if l.as_mut().unwrap().release() {
            return l.take().unwrap().finish();
        }
        None
    }

    #[test]
    fn a_drag_closes_on_letting_go() {
        let mut l = TapLasso::start((0.0, 0.0));
        for p in [(1.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)] {
            l.drag(p, 3.0, 1.0);
        }
        assert!(l.release());
        assert_eq!(l.finish().unwrap(), vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    }

    #[test]
    fn taps_make_a_polygon_closed_on_its_first_corner() {
        let mut l = None;
        for p in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)] {
            assert_eq!(tap(&mut l, p), None);
        }
        assert!(l.as_ref().unwrap().polygon);
        assert_eq!(tap(&mut l, (1.0, 1.0)), Some(vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]));
        assert!(l.is_none());
    }

    #[test]
    fn a_drag_between_corners_adds_a_freehand_stretch() {
        let mut l = None;
        tap(&mut l, (0.0, 0.0));
        let t = l.as_mut().unwrap();
        assert!(!t.press((10.0, 0.0), (10.0, 0.0), 5.0));
        for p in [(10.0, 2.0), (12.0, 6.0), (10.0, 10.0)] {
            t.drag(p, 3.0, 1.0);
        }
        assert!(!t.release(), "the polygon stays open");
        assert_eq!(t.corners, vec![0, 1]);
        assert_eq!(t.pts.len(), 4, "corner, then the stretch past the tap reach");
        assert!(t.doubled(100.0) && !t.doubled(5.0));
        // Back takes the corner and its stretch together.
        assert!(t.back());
        assert_eq!(t.pts, vec![(0.0, 0.0)]);
        assert!(!t.back(), "nothing left");
    }
}
