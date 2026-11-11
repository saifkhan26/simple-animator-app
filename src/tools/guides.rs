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

use super::perspective::{
    self, add, cross, dot, inside, intersect, len, line_quad, normalize, project, sub, GridGrab,
    GridKind, Homography, PerspectiveGrid, Space, Vp, P,
};

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
///
/// - A ruler is fixed: its segment, or with `infinite_ruler` its whole line.
/// - Parallel lines run through the start along the guide's line.
/// - A vanishing point's line runs from the start to the point.
/// - An ellipse is fixed; with `rings`, a stroke starting inside its quad
///   may also go round the ring through its start.
pub fn candidates(g: &PerspectiveGrid, s: &Space, start: P, vertical: bool, grid: usize) -> Vec<Candidate> {
    let c = g.doc_corners(s);
    let one = |curve, fixed| vec![Candidate { grid, curve, fixed }];
    match g.kind {
        GridKind::Ruler => {
            let (a, b) = (c[0], c[1]);
            let Some(dir) = normalize(sub(b, a)) else {
                return Vec::new();
            };
            if g.infinite_ruler {
                one(SnapCurve::Line { origin: a, dir, vp: None }, true)
            } else {
                one(SnapCurve::Segment { a, b }, true)
            }
        }
        GridKind::Parallel => match normalize(sub(c[1], c[0])) {
            Some(dir) => one(SnapCurve::Line { origin: start, dir, vp: None }, false),
            None => Vec::new(),
        },
        GridKind::Point => {
            let Some(v) = point_vp(&c) else {
                return Vec::new();
            };
            match normalize(sub(v, start)) {
                Some(dir) => one(SnapCurve::Line { origin: start, dir, vp: Some(v) }, false),
                None => Vec::new(),
            }
        }
        GridKind::Ellipse => {
            let Some(h) = Homography::square_to_quad(&c) else {
                return Vec::new();
            };
            let mut out = one(SnapCurve::Conic { h, r: 0.5 }, true);
            let inner = h
                .unmap(start)
                .filter(|uv| (0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]));
            if let Some(uv) = inner.filter(|_| g.rings) {
                let r = len(sub(uv, CENTRE));
                if r > 1e-3 {
                    out.push(Candidate { grid, curve: SnapCurve::Conic { h, r }, fixed: false });
                }
            }
            out
        }
        GridKind::Perspective | GridKind::Flat | GridKind::Isometric => plane_candidates(g, s, start, vertical, grid),
    }
}

/// A plane's families through the start, toward their vanishing points or
/// along their directions, and its extra vanishing points.
fn plane_candidates(g: &PerspectiveGrid, s: &Space, start: P, vertical: bool, grid: usize) -> Vec<Candidate> {
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

/// Where a vanishing-point guide's helper lines meet: its quad's left edge
/// (corners 0–3) and right edge (1–2). `None` while they run parallel.
pub fn point_vp(c: &[P; 4]) -> Option<P> {
    intersect(c[0], sub(c[3], c[0]), c[1], sub(c[2], c[1]))
}

/// The ring an ellipse guide draws and snaps to, from its corners in
/// document space.
pub fn ellipse(c: &[P; 4]) -> Option<SnapCurve> {
    Some(SnapCurve::Conic { h: Homography::square_to_quad(c)?, r: 0.5 })
}

/// What a press at `p` takes hold of on a guide that isn't a plane, within
/// `tol` document units. Planes go through [`perspective::grab_at`].
///
/// - A ruler: its ends, else its line to move it.
/// - A vanishing point: the point, a helper's end, else a helper to move it.
/// - An ellipse: a corner, inside to move it, just outside a corner to turn
///   it.
pub fn grab_at(kind: GridKind, c: &[P; 4], p: P, tol: f32) -> Option<GridGrab> {
    let near = |q: P, k: f32| len(sub(p, q)) <= tol * k;
    let corner = |n: usize, k: f32| (0..n).find(|&i| near(c[i], k)).map(|i| GridGrab::Corner(i as u8));
    match kind {
        GridKind::Ruler | GridKind::Parallel => corner(2, 1.0).or_else(|| {
            (SnapCurve::Segment { a: c[0], b: c[1] }.distance(p) <= tol).then_some(GridGrab::Move)
        }),
        GridKind::Point => {
            if point_vp(c).is_some_and(|v| near(v, 1.5)) {
                return Some(GridGrab::Vp(0));
            }
            corner(4, 1.0).or_else(|| {
                let on = |a: P, b: P| SnapCurve::Segment { a, b }.distance(p) <= tol;
                (on(c[0], c[3]) || on(c[1], c[2])).then_some(GridGrab::Move)
            })
        }
        GridKind::Ellipse => corner(4, 1.0)
            .or_else(|| inside(c, p).then_some(GridGrab::Move))
            .or_else(|| corner(4, 4.0).map(|_| GridGrab::Rotate)),
        GridKind::Perspective | GridKind::Flat | GridKind::Isometric => None,
    }
}

/// A non-plane guide's corners after dragging `grab` from `start` to `now`,
/// solved against the corners at the press. `snap` (Shift, once dragging)
/// turns a ruler or a helper line in 15° steps about its other end. `None`
/// for a drag that would leave the guide meaningless — a ruler of no
/// length, helper lines that never meet.
pub fn dragged(kind: GridKind, c0: &[P; 4], grab: GridGrab, start: P, now: P, snap: bool) -> Option<[P; 4]> {
    let d = sub(now, start);
    match (kind, grab) {
        (GridKind::Ellipse, _) => perspective::dragged(c0, grab, start, now, snap, false),
        (_, GridGrab::Move) => Some(c0.map(|q| add(q, d))),
        (GridKind::Ruler | GridKind::Parallel, GridGrab::Corner(i)) => {
            let i = (i & 1) as usize;
            let other = c0[1 - i];
            let mut end = add(c0[i], d);
            if snap {
                end = turn_in_steps(other, end);
            }
            if len(sub(end, other)) < 1e-4 {
                return None;
            }
            Some(if i == 0 { line_quad(end, other) } else { line_quad(other, end) })
        }
        (GridKind::Point, GridGrab::Corner(i)) => {
            let i = (i & 3) as usize;
            // Each end's partner on its helper: 0–3 and 1–2.
            let partner = [3, 2, 1, 0][i];
            let mut c = *c0;
            c[i] = add(c0[i], d);
            if snap {
                c[i] = turn_in_steps(c0[partner], c[i]);
            }
            point_vp(&c).map(|_| c)
        }
        (GridKind::Point, GridGrab::Vp(_)) => {
            let v0 = point_vp(c0)?;
            let v = add(v0, d);
            let mut c = *c0;
            for (a, b) in [(0, 3), (1, 2)] {
                // The end nearer the point swings round the far one, keeping
                // its distance from it, so the helper runs to the new point.
                let (near_i, far_i) = if len(sub(c0[a], v0)) <= len(sub(c0[b], v0)) { (a, b) } else { (b, a) };
                let far = c0[far_i];
                let dir = normalize(sub(v, far))?;
                let l = len(sub(c0[near_i], far));
                c[near_i] = add(far, [dir[0] * l, dir[1] * l]);
            }
            point_vp(&c).map(|_| c)
        }
        _ => None,
    }
}

/// `p` turned about `o` to the nearest 15° step, at the same distance.
fn turn_in_steps(o: P, p: P) -> P {
    let v = sub(p, o);
    let step = std::f32::consts::PI / 12.0;
    let a = (v[1].atan2(v[0]) / step).round() * step;
    let l = len(v);
    add(o, [l * a.cos(), l * a.sin()])
}

/// Whether two vectors run the same way or opposite, near enough.
#[cfg(test)]
fn parallel(a: P, b: P) -> bool {
    cross(a, b).abs() <= 1e-3 * len(a) * len(b)
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

    fn fresh_doc(kind: GridKind) -> (PerspectiveGrid, Space) {
        (PerspectiveGrid::fresh(kind), Space::flat(1000.0, 1000.0))
    }

    #[test]
    fn a_ruler_is_fixed_and_an_infinite_one_runs_on() {
        let (mut g, s) = fresh_doc(GridKind::Ruler);
        let c = g.doc_corners(&s);
        let got = candidates(&g, &s, [0.0, 0.0], false, 4);
        assert_eq!(got, vec![Candidate { grid: 4, curve: SnapCurve::Segment { a: c[0], b: c[1] }, fixed: true }]);
        g.infinite_ruler = true;
        let got = candidates(&g, &s, [0.0, 0.0], false, 4);
        assert!(matches!(got[0].curve, SnapCurve::Line { .. }) && got[0].fixed);
        // Past its end the infinite line still holds a point on the line.
        let far = [c[1][0] + 300.0, c[1][1] + 5.0];
        assert!(near(got[0].curve.project(far), [far[0], c[1][1]]));
    }

    #[test]
    fn parallel_lines_and_a_point_run_through_the_start() {
        let (g, s) = fresh_doc(GridKind::Parallel);
        let got = candidates(&g, &s, [10.0, 20.0], false, 0);
        assert!(matches!(got[0].curve, SnapCurve::Line { origin: [10.0, 20.0], dir, vp: None } if dir[1].abs() < 1e-6));
        assert!(!got[0].fixed);
        let (g, s) = fresh_doc(GridKind::Point);
        let v = point_vp(&g.doc_corners(&s)).unwrap();
        // Up at (0, -0.275) frame heights from the middle.
        assert!(near(v, s.to_doc([0.0, -0.275])), "{v:?}");
        let got = candidates(&g, &s, [10.0, 900.0], false, 0);
        assert!(matches!(got[0].curve, SnapCurve::Line { vp: Some(p), .. } if near(p, v)));
    }

    #[test]
    fn an_ellipse_offers_rings_only_from_inside_it() {
        let (mut g, s) = fresh_doc(GridKind::Ellipse);
        let c = g.doc_corners(&s);
        let mid = [(c[0][0] + c[2][0]) / 2.0, (c[0][1] + c[2][1]) / 2.0];
        let inside_pt = [mid[0] + 20.0, mid[1]];
        assert_eq!(candidates(&g, &s, inside_pt, false, 0).len(), 1, "rings off");
        g.rings = true;
        let got = candidates(&g, &s, inside_pt, false, 0);
        assert_eq!(got.len(), 2);
        assert!(!got[1].fixed);
        assert!(near(got[1].curve.project([mid[0], mid[1] - 50.0]), [mid[0], mid[1] - 20.0]));
        // Outside the quad, only the ellipse itself.
        assert_eq!(candidates(&g, &s, [0.0, 0.0], false, 0).len(), 1);
    }

    #[test]
    fn a_ruler_end_drags_alone_and_shift_turns_it_in_steps() {
        let c0 = line_quad([0.0, 0.0], [100.0, 0.0]);
        let c = dragged(GridKind::Ruler, &c0, GridGrab::Corner(1), [100.0, 0.0], [100.0, 30.0], false).unwrap();
        assert_eq!(c, line_quad([0.0, 0.0], [100.0, 30.0]));
        let c = dragged(GridKind::Ruler, &c0, GridGrab::Corner(1), [100.0, 0.0], [100.0, 30.0], true).unwrap();
        let a = c[1][1].atan2(c[1][0]).to_degrees();
        assert!((a - 15.0).abs() < 1e-3, "{a}");
        assert!(dragged(GridKind::Ruler, &c0, GridGrab::Corner(1), [100.0, 0.0], [0.0, 0.0], false).is_none());
        assert_eq!(grab_at(GridKind::Ruler, &c0, [50.0, 3.0], 5.0), Some(GridGrab::Move));
        assert_eq!(grab_at(GridKind::Ruler, &c0, [98.0, 2.0], 5.0), Some(GridGrab::Corner(1)));
        assert_eq!(grab_at(GridKind::Ruler, &c0, [50.0, 30.0], 5.0), None);
    }

    #[test]
    fn dragging_a_vanishing_point_swings_its_helpers_round_their_far_ends() {
        let (g, s) = fresh_doc(GridKind::Point);
        let c0 = g.doc_corners(&s);
        let v0 = point_vp(&c0).unwrap();
        assert_eq!(grab_at(GridKind::Point, &c0, v0, 5.0), Some(GridGrab::Vp(0)));
        let to = [v0[0] + 80.0, v0[1] - 40.0];
        let c = dragged(GridKind::Point, &c0, GridGrab::Vp(0), v0, to, false).unwrap();
        assert!(near(point_vp(&c).unwrap(), to));
        // The far ends stay; the near ones keep their reach.
        assert_eq!((c[2], c[3]), (c0[2], c0[3]));
        assert!((len(sub(c[0], c[3])) - len(sub(c0[0], c0[3]))).abs() < 1e-2);
        assert!(parallel(sub(c[0], c[3]), sub(to, c[3])));
        // Helpers made parallel are refused: they would never meet.
        let along = sub(c0[2], c0[1]);
        let to = sub(c0[3], along);
        assert!(dragged(GridKind::Point, &c0, GridGrab::Corner(0), c0[0], to, false).is_none());
    }

    #[test]
    fn pull_goes_part_way() {
        assert_eq!(pull([0.0, 0.0], [10.0, 20.0], 1.0), [10.0, 20.0]);
        assert!(near(pull([0.0, 0.0], [10.0, 20.0], 0.5), [5.0, 10.0]));
        assert_eq!(pull([3.0, 4.0], [10.0, 20.0], 0.0), [3.0, 4.0]);
    }
}
