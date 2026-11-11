//! Snapping strokes to guides.
//!
//! Every visible guide whose magnet is on offers the curves a stroke could be
//! held to. Two sorts:
//!
//! - *Through the start*: a grid family's line through where the stroke went
//!   down, toward its vanishing point or along its direction. One of these is
//!   picked once the pointer has gone far enough to show which way it is
//!   heading, by how well the curve's direction there matches the motion.
//! - *Fixed*: a curve that sits somewhere on the canvas whatever the stroke
//!   does — a ruler, an ellipse. A press close enough to one takes it at
//!   once, before any motion, so "press on the ruler, draw along it" works.
//!
//! The pick is held for the rest of the stroke: re-picking per point would let
//! a wobbly hand flip between families.

use super::perspective::{normalize, project, Homography, PerspectiveGrid, Space, Vp, P};

/// A curve a stroke can be held to, in document space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SnapCurve {
    /// The infinite line through `origin` along unit `dir`. `vp` is the
    /// vanishing point it runs to, if finite — past it the line means
    /// nothing, so its guide stops there.
    Line { origin: P, dir: P, vp: Option<P> },
    /// The segment from `a` to `b`: points past an end stay on that end.
    Segment { a: P, b: P },
    /// The image under `h` of the circle of radius `r` about the middle of
    /// the unit square: a circle seen in perspective, or — when `h` is
    /// affine — a plain ellipse.
    Conic { h: Homography, r: f32 },
}

/// Points round a conic, for drawing it and for projecting onto it where the
/// map can't be undone.
pub const CONIC_SAMPLES: usize = 96;

const CENTRE: P = [0.5, 0.5];

impl SnapCurve {
    /// The point of the curve a stroke at `p` is held to.
    pub fn project(&self, p: P) -> P {
        match *self {
            SnapCurve::Line { origin, dir, .. } => project(origin, dir, p),
            SnapCurve::Segment { a, b } => {
                let d = sub(b, a);
                let l2 = dot(d, d);
                if l2 < 1e-12 {
                    return a;
                }
                let t = (dot(sub(p, a), d) / l2).clamp(0.0, 1.0);
                [a[0] + d[0] * t, a[1] + d[1] * t]
            }
            SnapCurve::Conic { h, r } => match h.unmap(p) {
                // Straight out from the circle's middle in the square, so the
                // point slides round the ring as the pen goes round it.
                Some(uv) => {
                    let a = angle_of(uv);
                    on_circle(&h, r, a).unwrap_or(p)
                }
                None => nearest_sample(&h, r, p).unwrap_or(p),
            },
        }
    }

    /// The curve's unit direction at the point nearest `p`.
    pub fn tangent_at(&self, p: P) -> Option<P> {
        match *self {
            SnapCurve::Line { dir, .. } => Some(dir),
            SnapCurve::Segment { a, b } => normalize(sub(b, a)),
            SnapCurve::Conic { h, r } => {
                let a = match h.unmap(p) {
                    Some(uv) => angle_of(uv),
                    None => {
                        let q = nearest_sample(&h, r, p)?;
                        angle_of(h.unmap(q)?)
                    }
                };
                const E: f32 = 1e-3;
                let (p0, p1) = (on_circle(&h, r, a - E)?, on_circle(&h, r, a + E)?);
                normalize(sub(p1, p0))
            }
        }
    }

    /// How far `p` is from the curve.
    pub fn distance(&self, p: P) -> f32 {
        let q = self.project(p);
        (q[0] - p[0]).hypot(q[1] - p[1])
    }

    /// The same curve, through `start` instead, for a line that runs through
    /// a stroke's start — what a shape slid mid-drag keeps to. Fixed curves
    /// stay where they are.
    pub fn through(self, start: P) -> Self {
        match self {
            SnapCurve::Line { dir, vp, .. } => SnapCurve::Line { origin: start, dir, vp },
            other => other,
        }
    }

    /// A conic as a closed polyline in document space, skipping any part
    /// that would fall behind the viewer.
    pub fn conic_points(&self) -> Vec<P> {
        let SnapCurve::Conic { h, r } = *self else {
            return Vec::new();
        };
        (0..CONIC_SAMPLES)
            .filter_map(|i| on_circle(&h, r, i as f32 / CONIC_SAMPLES as f32 * std::f32::consts::TAU))
            .collect()
    }
}

/// One way a stroke could go, and the guide that offers it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// Index of the guide in the file's list.
    pub grid: usize,
    pub curve: SnapCurve,
    /// Sits on the canvas whatever the stroke does (a ruler, an ellipse),
    /// rather than running through the stroke's start.
    pub fixed: bool,
}

/// Everything guide `g` (at `grid` in the list) offers a stroke that starts
/// at `start`. `vertical` adds a perspective plane's vertical.
pub fn candidates(g: &PerspectiveGrid, s: &Space, start: P, vertical: bool, grid: usize) -> Vec<Candidate> {
    g.snap_targets(s, vertical)
        .into_iter()
        .filter_map(|t| {
            let (dir, vp) = match t {
                Vp::Point(v) => (normalize(sub(v, start))?, Some(v)),
                Vp::Dir(d) => (d, None),
            };
            Some(Candidate {
                grid,
                curve: SnapCurve::Line { origin: start, dir, vp },
                fixed: false,
            })
        })
        .collect()
}

/// Of the curves through the start, the one whose direction there best
/// matches `motion`, either way along it. Ties go to the later one, as
/// [`super::perspective::pick_dir`] has it.
pub fn pick(cands: &[Candidate], start: P, motion: P) -> Option<Candidate> {
    let m = normalize(motion)?;
    let score = |c: &Candidate| c.curve.tangent_at(start).map_or(-1.0, |t| dot(t, m).abs());
    cands
        .iter()
        .filter(|c| !c.fixed)
        .copied()
        .max_by(|a, b| score(a).total_cmp(&score(b)))
}

/// The fixed curve nearest `p`, if one is within `tol`.
pub fn nearest_fixed(cands: &[Candidate], p: P, tol: f32) -> Option<Candidate> {
    cands
        .iter()
        .filter(|c| c.fixed)
        .map(|c| (c.curve.distance(p), *c))
        .filter(|(d, _)| *d <= tol)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, c)| c)
}

/// `raw` pulled `strength` of the way toward `to`: all the way at 1, not at
/// all at 0.
pub fn pull(raw: P, to: P, strength: f32) -> P {
    if strength >= 1.0 {
        return to;
    }
    let s = strength.max(0.0);
    [raw[0] + (to[0] - raw[0]) * s, raw[1] + (to[1] - raw[1]) * s]
}

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot(a: P, b: P) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

/// Angle of `uv` about the square's middle.
fn angle_of(uv: P) -> f32 {
    (uv[1] - CENTRE[1]).atan2(uv[0] - CENTRE[0])
}

/// The point at angle `a` round the circle, mapped onto the canvas.
fn on_circle(h: &Homography, r: f32, a: f32) -> Option<P> {
    let (s, c) = a.sin_cos();
    h.map_front(CENTRE[0] + r * c, CENTRE[1] + r * s)
}

fn nearest_sample(h: &Homography, r: f32, p: P) -> Option<P> {
    (0..CONIC_SAMPLES)
        .filter_map(|i| on_circle(h, r, i as f32 / CONIC_SAMPLES as f32 * std::f32::consts::TAU))
        .min_by(|a, b| {
            let da = (a[0] - p[0]).hypot(a[1] - p[1]);
            let db = (b[0] - p[0]).hypot(b[1] - p[1]);
            da.total_cmp(&db)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: P, b: P) -> bool {
        (a[0] - b[0]).abs() < 1e-2 && (a[1] - b[1]).abs() < 1e-2
    }

    fn line(origin: P, dir: P) -> Candidate {
        Candidate {
            grid: 0,
            curve: SnapCurve::Line { origin, dir, vp: None },
            fixed: false,
        }
    }

    #[test]
    fn a_line_projects_square_onto_itself() {
        let c = SnapCurve::Line { origin: [10.0, 10.0], dir: [1.0, 0.0], vp: None };
        assert!(near(c.project([30.0, 25.0]), [30.0, 10.0]));
    }

    #[test]
    fn a_segment_holds_points_past_its_ends_on_the_ends() {
        let c = SnapCurve::Segment { a: [0.0, 0.0], b: [100.0, 0.0] };
        assert!(near(c.project([50.0, 20.0]), [50.0, 0.0]));
        assert!(near(c.project([-30.0, 5.0]), [0.0, 0.0]));
        assert!(near(c.project([180.0, -5.0]), [100.0, 0.0]));
    }

    #[test]
    fn a_conic_in_a_square_is_its_inscribed_circle() {
        let sq = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        let c = SnapCurve::Conic { h: Homography::square_to_quad(&sq).unwrap(), r: 0.5 };
        // Out from the middle onto the ring, whichever side.
        assert!(near(c.project([90.0, 50.0]), [100.0, 50.0]));
        assert!(near(c.project([50.0, 20.0]), [50.0, 0.0]));
        assert!((c.distance([50.0, 50.0 + 30.0]) - 20.0).abs() < 0.05);
        // Its direction at the top is level.
        let t = c.tangent_at([50.0, 5.0]).unwrap();
        assert!(t[1].abs() < 1e-3, "{t:?}");
    }

    #[test]
    fn a_conic_in_perspective_touches_each_edge_at_its_middle_in_depth() {
        // A floor trapezoid: the circle meets every edge, running along it.
        let trap = [[40.0, 0.0], [60.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        let h = Homography::square_to_quad(&trap).unwrap();
        let c = SnapCurve::Conic { h, r: 0.5 };
        for (uv, edge) in [
            ([0.5, 0.0], ([40.0, 0.0], [60.0, 0.0])),
            ([1.0, 0.5], ([60.0, 0.0], [100.0, 100.0])),
            ([0.5, 1.0], ([100.0, 100.0], [0.0, 100.0])),
            ([0.0, 0.5], ([0.0, 100.0], [40.0, 0.0])),
        ] {
            let p = h.map(uv[0], uv[1]);
            assert!(near(c.project(p), p), "on the ring: {p:?}");
            let t = c.tangent_at(p).unwrap();
            let e = normalize(sub(edge.1, edge.0)).unwrap();
            let cross = t[0] * e[1] - t[1] * e[0];
            assert!(cross.abs() < 1e-2, "touches the edge there: {t:?} vs {e:?}");
        }
    }

    #[test]
    fn the_pick_follows_the_motion_and_skips_fixed_curves() {
        let start = [0.0, 0.0];
        let cands = [
            line(start, [1.0, 0.0]),
            line(start, [0.0, 1.0]),
            Candidate {
                grid: 1,
                curve: SnapCurve::Segment { a: [0.0, 0.0], b: [100.0, 100.0] },
                fixed: true,
            },
        ];
        let c = pick(&cands, start, [3.0, 20.0]).unwrap();
        assert_eq!(c.curve, cands[1].curve);
        // Heading along the diagonal still picks a line, not the ruler.
        assert!(!pick(&cands, start, [10.0, 10.0]).unwrap().fixed);
        assert!(pick(&cands, start, [0.0, 0.0]).is_none(), "no motion, no way");
    }

    #[test]
    fn a_press_takes_the_nearest_fixed_curve_in_reach() {
        let ruler = |grid, y| Candidate {
            grid,
            curve: SnapCurve::Segment { a: [0.0, y], b: [100.0, y] },
            fixed: true,
        };
        let cands = [ruler(0, 0.0), ruler(1, 10.0), line([50.0, 4.0], [1.0, 0.0])];
        assert_eq!(nearest_fixed(&cands, [50.0, 7.0], 5.0).unwrap().grid, 1);
        assert!(nearest_fixed(&cands, [50.0, 40.0], 5.0).is_none(), "too far from both");
    }

    #[test]
    fn pull_goes_part_way() {
        assert_eq!(pull([0.0, 0.0], [10.0, 20.0], 1.0), [10.0, 20.0]);
        assert!(near(pull([0.0, 0.0], [10.0, 20.0], 0.5), [5.0, 10.0]));
        assert_eq!(pull([3.0, 4.0], [10.0, 20.0], 0.0), [3.0, 4.0]);
    }
}
