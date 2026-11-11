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
//!
//! The curved guides bend their families through side points:
//!
//! - A pan's curves are [`SnapCurve::Wave`]s: in its own frame, the graph of
//!   an arch over the span between two side points, the next span's arch
//!   below by turns. A horizontal line seen by a camera turning on the spot
//!   comes out like that — a cosine on a cylindrical panorama, which the
//!   arch with [`PerspectiveGrid::bend`] at 0 is close to. Its arches run
//!   through the side points; the same wave shifted a span runs through
//!   the centre point, for the lines going away from the viewer.
//! - A fisheye's are [`SnapCurve::Arc`]s, as in the classic five-point
//!   construction: circles through two opposite side points, scaled to the
//!   box, so the outermost one is the 180° ellipse through all four.

use super::perspective::{
    self, add, centroid, dot, inside, intersect, len, line_quad, normalize, project, sub,
    GridGrab, GridKind, Homography, PerspectiveGrid, Space, Vp, P,
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
    /// A pan's curve, in the frame at `o` along unit `u` and unit `n` square
    /// to it: the graph `y = amp · wave(x / a − shift)`, where the wave is an
    /// arch of height 1 over every span 2 wide between its zeros at the odd
    /// numbers, above and below by turns, shaped by `bend` (see [`arch`]).
    /// So its points are `a` apart, at `x = (shift ± 1) · a`, `(shift ± 3) ·
    /// a`… It is drawn `span` points' spacing either side of `o`.
    Wave { o: P, u: P, n: P, a: f32, shift: f32, amp: f32, bend: f32, span: f32 },
    /// A fisheye's curve: in the frame at `o` along `u` and `n`, scaled by
    /// `a` along `u` and `b` along `n`, the circle through the side points
    /// (±1, 0) whose middle point — on the axis through `o` along `n` — is
    /// `apex`. An ellipse arc, square to the box.
    Arc { o: P, u: P, n: P, a: f32, b: f32, apex: f32 },
}

/// Flattest and fullest a pan's arches may be — see [`PerspectiveGrid::bend`].
pub const BEND: std::ops::RangeInclusive<f32> = -0.75..=1.0;

/// A pan's arch at `t` in −1..=1, 1 high at 0 and 0 at the ends: `1 − (1 −
/// c)t² − ct⁴` for `bend` `c`. At 0 a parabola; toward 1 the shoulders fill
/// out and it meets its ends steeply; below 0 it flattens and meets them at
/// a shallow angle. Whatever the bend it falls all the way, and its arches
/// cross at their points at an angle that grows with their height — as
/// lines running to a vanishing point do.
pub fn arch(bend: f32, t: f32) -> f32 {
    let c = bend.clamp(*BEND.start(), *BEND.end());
    let t2 = t * t;
    1.0 - (1.0 - c) * t2 - c * t2 * t2
}

/// The slope of [`arch`] at `t`.
fn arch_slope(bend: f32, t: f32) -> f32 {
    let c = bend.clamp(*BEND.start(), *BEND.end());
    -2.0 * (1.0 - c) * t - 4.0 * c * t * t * t
}

/// The wave a pan's curves follow, and its slope, at `t`: [`arch`] over
/// every span from one odd number to the next, upside down every other
/// span, so it runs on for ever, 4 to a cycle.
pub fn wave(bend: f32, t: f32) -> (f32, f32) {
    let m = (t * 0.5).round();
    let tau = t - 2.0 * m;
    let sign = if (m as i64).rem_euclid(2) == 0 { 1.0 } else { -1.0 };
    (sign * arch(bend, tau), sign * arch_slope(bend, tau))
}

/// A curved guide's own frame, read off its box: the centre point `o`, unit
/// `u` along its horizon and unit `n` square to it toward the box's bottom
/// edge, and how far the box reaches along each — `w` to its sides, `h` to
/// its top and bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub o: P,
    pub u: P,
    pub n: P,
    pub w: f32,
    pub h: f32,
}

impl Frame {
    /// The frame of a curved guide's corners. `n` is taken from the box, not
    /// from `u`, so a mirrored space keeps its handedness. `None` for a box
    /// with no width or height.
    pub fn of(c: &[P; 4]) -> Option<Self> {
        let mid = |a: P, b: P| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
        let across = sub(mid(c[1], c[2]), mid(c[0], c[3]));
        let down = sub(mid(c[2], c[3]), mid(c[0], c[1]));
        let u = normalize(across)?;
        // Square to `u`: a box tweened between two turns is no rectangle.
        let n = normalize(sub(down, u.map(|x| x * dot(down, u))))?;
        let (w, h) = (len(across) * 0.5, dot(down, n) * 0.5);
        (w > 1e-6 && h > 1e-6).then_some(Self { o: centroid(c), u, n, w, h })
    }

    /// The document point `x` along `u` and `y` along `n` from the centre.
    pub fn at(&self, x: f32, y: f32) -> P {
        [
            self.o[0] + self.u[0] * x + self.n[0] * y,
            self.o[1] + self.u[1] * x + self.n[1] * y,
        ]
    }

    /// The box this frame reads off, TL, TR, BR, BL.
    pub fn corners(&self) -> [P; 4] {
        let (w, h) = (self.w, self.h);
        [self.at(-w, -h), self.at(w, -h), self.at(w, h), self.at(-w, h)]
    }
}

/// The wave of a pan's family through `p`: `None` when `p` sits square
/// over one of its points, where no curve of the family passes but the
/// straight line square to the axis.
#[allow(clippy::too_many_arguments)]
pub fn wave_through(o: P, u: P, n: P, a: f32, shift: f32, bend: f32, span: f32, p: P) -> Option<SnapCurve> {
    let d = sub(p, o);
    let (x, y) = (dot(d, u), dot(d, n));
    let (w, _) = wave(bend, x / a - shift);
    if w.abs() < 1e-3 {
        return (y.abs() < 1e-6).then_some(SnapCurve::Wave { o, u, n, a, shift, amp: 0.0, bend, span });
    }
    Some(SnapCurve::Wave { o, u, n, a, shift, amp: y / w, bend, span })
}

/// A fisheye's arc through `p`, round the side points `o ± a·u`, in a box
/// `b` deep along `n`. On the axis itself, the straight line along it.
pub fn arc_through(o: P, u: P, n: P, a: f32, b: f32, p: P) -> SnapCurve {
    let d = sub(p, o);
    let (t, s) = (dot(d, u) / a, dot(d, n) / b);
    if s.abs() < 1e-6 {
        return SnapCurve::Line { origin: o, dir: u, vp: None };
    }
    // The circle through (±1, 0) and (t, s) has its middle at (0, k).
    let k = (t * t + s * s - 1.0) / (2.0 * s);
    let r = (1.0 + k * k).sqrt();
    let apex = if s > 0.0 { k + r } else { k - r };
    SnapCurve::Arc { o, u, n, a, b, apex }
}

/// The middle and radius, in its scaled frame, of the circle an arc is part
/// of.
fn arc_circle(apex: f32) -> (f32, f32) {
    let k = (apex * apex - 1.0) / (2.0 * apex);
    (k, (1.0 + k * k).sqrt())
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
            SnapCurve::Wave { o, u, n, .. } => {
                let (x, y) = self.wave_nearest(p);
                [o[0] + u[0] * x + n[0] * y, o[1] + u[1] * x + n[1] * y]
            }
            SnapCurve::Arc { o, u, n, a, b, apex } => {
                // Straight out from the circle's middle in the scaled frame,
                // as a conic does from its square's.
                let d = sub(p, o);
                let (t, s) = (dot(d, u) / a, dot(d, n) / b);
                let (k, r) = arc_circle(apex);
                let Some(q) = normalize([t, s - k]) else {
                    return p;
                };
                let (t, s) = (q[0] * r, k + q[1] * r);
                [o[0] + u[0] * a * t + n[0] * b * s, o[1] + u[1] * a * t + n[1] * b * s]
            }
        }
    }

    /// The nearest point of a wave to `p`, as `(x, y)` in its frame. It lies
    /// no farther along the axis than the curve lies from `p` straight
    /// across, so that stretch is searched — coarsely, then closer in.
    fn wave_nearest(&self, p: P) -> (f32, f32) {
        let SnapCurve::Wave { o, u, n, a, shift, amp, bend, .. } = *self else {
            return (0.0, 0.0);
        };
        let d = sub(p, o);
        let (x0, y0) = (dot(d, u), dot(d, n));
        let y_at = |x: f32| amp * wave(bend, x / a - shift).0;
        let dist = |x: f32| (x - x0).powi(2) + (y_at(x) - y0).powi(2);
        let reach = (y_at(x0) - y0).abs();
        if reach < 1e-6 {
            return (x0, y_at(x0));
        }
        const STEPS: usize = 48;
        let step = reach * 2.0 / STEPS as f32;
        let best = (0..=STEPS)
            .map(|i| x0 - reach + step * i as f32)
            .min_by(|&p, &q| dist(p).total_cmp(&dist(q)))
            .unwrap_or(x0);
        // Golden-section search either side of the best sample.
        let (mut lo, mut hi) = (best - step, best + step);
        const G: f32 = 0.618_034;
        for _ in 0..24 {
            let (m1, m2) = (hi - G * (hi - lo), lo + G * (hi - lo));
            if dist(m1) < dist(m2) {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        let x = (lo + hi) * 0.5;
        (x, y_at(x))
    }

    /// The curve's unit direction at the point nearest `p`.
    pub fn tangent_at(&self, p: P) -> Option<P> {
        match *self {
            SnapCurve::Line { dir, .. } => Some(dir),
            SnapCurve::Segment { a, b } => normalize(sub(b, a)),
            SnapCurve::Wave { u, n, a, shift, amp, bend, .. } => {
                let (x, _) = self.wave_nearest(p);
                let k = amp * wave(bend, x / a - shift).1 / a;
                normalize(add(u, n.map(|c| c * k)))
            }
            SnapCurve::Arc { o, u, n, a, b, apex } => {
                let d = sub(p, o);
                let (t, s) = (dot(d, u) / a, dot(d, n) / b);
                let (k, _) = arc_circle(apex);
                // Square to the radius in the scaled frame, scaled back.
                let (dt, ds) = (-(s - k), t);
                normalize(add(u.map(|c| c * a * dt), n.map(|c| c * b * ds)))
            }
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
    /// a stroke's start — what a shape slid mid-drag keeps to: the line
    /// along the same way, or the same family's curve. Fixed curves stay
    /// where they are.
    pub fn through(self, start: P) -> Self {
        match self {
            SnapCurve::Line { dir, vp, .. } => SnapCurve::Line { origin: start, dir, vp },
            SnapCurve::Wave { o, u, n, a, shift, bend, span, .. } => {
                wave_through(o, u, n, a, shift, bend, span, start).unwrap_or(self)
            }
            SnapCurve::Arc { o, u, n, a, b, .. } => arc_through(o, u, n, a, b, start),
            other => other,
        }
    }

    /// A pan's or a fisheye's curve as polylines in document space, for
    /// drawing over the box `lo`–`hi`: a wave where it crosses the box, out
    /// to its `span`, broken where it leaves the box far behind; an arc from
    /// one side point to the other.
    pub fn bend_points(&self, lo: P, hi: P) -> Vec<Vec<P>> {
        match *self {
            SnapCurve::Wave { o, u, n, a, shift, amp, bend, span } => {
                let box_c = [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]];
                let along = |v: P| box_c.map(|q| dot(sub(q, o), v));
                let (xs, ys) = (along(u), along(n));
                let min = |v: [f32; 4]| v.iter().copied().fold(f32::INFINITY, f32::min);
                let max = |v: [f32; 4]| v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let (mut x0, mut x1) = (min(xs), max(xs));
                if span.is_finite() {
                    x0 = x0.max(-span * a);
                    x1 = x1.min(span * a);
                }
                let (y0, y1) = (min(ys), max(ys));
                let slack = y1 - y0;
                if x1 <= x0 || a <= 0.0 {
                    return Vec::new();
                }
                // Fine enough to bend smoothly, few enough to draw.
                let n_steps = (((x1 - x0) / (a / 32.0)).ceil() as usize).clamp(2, 1500);
                let mut runs: Vec<Vec<P>> = Vec::new();
                let mut run: Vec<P> = Vec::new();
                for i in 0..=n_steps {
                    let x = x0 + (x1 - x0) * i as f32 / n_steps as f32;
                    let y = amp * wave(bend, x / a - shift).0;
                    if y < y0 - slack || y > y1 + slack {
                        if run.len() > 1 {
                            runs.push(std::mem::take(&mut run));
                        }
                        run.clear();
                        continue;
                    }
                    run.push([o[0] + u[0] * x + n[0] * y, o[1] + u[1] * x + n[1] * y]);
                }
                if run.len() > 1 {
                    runs.push(run);
                }
                runs
            }
            SnapCurve::Arc { o, u, n, a, b, apex } => {
                let (k, r) = arc_circle(apex);
                let tau = std::f32::consts::TAU;
                let start = (-k).atan2(1.0);
                let end = (-k).atan2(-1.0);
                let mid = (apex - k).atan2(0.0);
                // Round from one side point to the other by way of the apex.
                let ccw = (end - start).rem_euclid(tau);
                let sweep = if (mid - start).rem_euclid(tau) <= ccw { ccw } else { ccw - tau };
                const STEPS: usize = 64;
                let pts = (0..=STEPS)
                    .map(|i| {
                        let th = start + sweep * i as f32 / STEPS as f32;
                        let (t, s) = (r * th.cos(), k + r * th.sin());
                        [o[0] + u[0] * a * t + n[0] * b * s, o[1] + u[1] * a * t + n[1] * b * s]
                    })
                    .collect();
                vec![pts]
            }
            _ => Vec::new(),
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
        GridKind::PanHorizontal | GridKind::PanVertical | GridKind::PanLoop | GridKind::Fisheye => {
            curved_candidates(g, &c, start, grid)
        }
    }
}

/// Which way a pan bends, in its box's [`Frame`]: its curves are graphs
/// across `u`, out along `n`, with side points `a` apart along `u`; drawn
/// `span` side points' spacing either side; its third family straight
/// along `straight`. Its box reaches `reach` out along `n` either side: the
/// outermost arch drawn, which carries the curve-shape handle on its `side`
/// (−1 or 1 along `n`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanAxis {
    pub u: P,
    pub n: P,
    pub a: f32,
    pub span: f32,
    pub straight: P,
    pub reach: f32,
    pub side: f32,
}

/// A pan's axis. A horizontal pan's side points are at its box's sides and
/// its verticals stay straight; a vertical pan's are at the top and bottom
/// and its horizontals stay straight. A 360° pan's box is a whole turn, its
/// side points a quarter turn apart, and its curves run on past it. The
/// shape handle sits top right, or on a vertical pan bottom right, as in
/// Harmony.
pub fn pan_axis(kind: GridKind, f: &Frame) -> Option<PanAxis> {
    let across = |a: f32, span: f32| PanAxis {
        u: f.u,
        n: f.n,
        a,
        span,
        straight: f.n,
        reach: f.h,
        side: -1.0,
    };
    match kind {
        GridKind::PanHorizontal => Some(across(f.w, 1.0)),
        GridKind::PanLoop => Some(across(f.w * 0.5, f32::INFINITY)),
        GridKind::PanVertical => Some(PanAxis {
            u: f.n,
            n: f.u,
            a: f.h,
            span: 1.0,
            straight: f.u,
            reach: f.w,
            side: 1.0,
        }),
        _ => None,
    }
}

/// Where a pan's curve-shape handle sits: on its outermost arch, half way
/// from the middle to a side point.
pub fn bend_handle(kind: GridKind, bend: f32, c: &[P; 4]) -> Option<P> {
    let f = Frame::of(c)?;
    let ax = pan_axis(kind, &f)?;
    let y = ax.side * ax.reach * arch(bend, 0.5);
    Some(add(f.o, add(ax.u.map(|k| k * ax.a * 0.5), ax.n.map(|k| k * y))))
}

/// The bend that puts a pan's shape handle as near `p` as it can go:
/// out toward the box's edge for fuller arches, in for flatter ones.
pub fn bend_at(kind: GridKind, c: &[P; 4], p: P) -> Option<f32> {
    let f = Frame::of(c)?;
    let ax = pan_axis(kind, &f)?;
    let out = ax.side * dot(sub(p, f.o), ax.n) / ax.reach;
    // `arch(c, ½)` is ¾ + 3c⁄16.
    let b = (out - 0.75) / 0.1875;
    Some(b.clamp(*BEND.start(), *BEND.end()))
}

/// A curved guide's side points, each with the grab that stretches it: a
/// pan's two, a 360° pan's four (the inner pair a quarter turn out, the
/// outer the box's ends), a fisheye's four. See [`dragged`].
pub fn side_points(kind: GridKind, c: &[P; 4]) -> Vec<(P, GridGrab)> {
    let Some(f) = Frame::of(c) else {
        return Vec::new();
    };
    let along = |v: P, k: f32, g| [(add(f.o, v.map(|x| x * k)), g), (sub(f.o, v.map(|x| x * k)), g)];
    let mut out = Vec::new();
    if matches!(kind, GridKind::PanHorizontal | GridKind::PanLoop | GridKind::Fisheye) {
        out.extend(along(f.u, f.w, GridGrab::Vp(0)));
    }
    if matches!(kind, GridKind::PanVertical | GridKind::Fisheye) {
        out.extend(along(f.n, f.h, GridGrab::Vp(1)));
    }
    if kind == GridKind::PanLoop {
        out.extend(along(f.u, f.w * 0.5, GridGrab::Vp(2)));
    }
    out
}

/// A curved guide's families through the start. A pan's: its arch through
/// the side points (the lines across), the same wave a span along through
/// the centre point (the lines going away), and its straight lines. A
/// fisheye's: its arcs through the side points either way, and the line
/// to the centre.
fn curved_candidates(g: &PerspectiveGrid, c: &[P; 4], start: P, grid: usize) -> Vec<Candidate> {
    let Some(f) = Frame::of(c) else {
        return Vec::new();
    };
    let cand = |curve| Candidate { grid, curve, fixed: false };
    if g.kind == GridKind::Fisheye {
        let mut out = vec![
            cand(arc_through(f.o, f.u, f.n, f.w, f.h, start)),
            cand(arc_through(f.o, f.n, f.u, f.h, f.w, start)),
        ];
        if let Some(dir) = normalize(sub(f.o, start)) {
            out.push(cand(SnapCurve::Line { origin: start, dir, vp: Some(f.o) }));
        }
        return out;
    }
    let Some(ax) = pan_axis(g.kind, &f) else {
        return Vec::new();
    };
    let mut out: Vec<Candidate> = [0.0, 1.0]
        .into_iter()
        .filter_map(|shift| wave_through(f.o, ax.u, ax.n, ax.a, shift, g.bend, ax.span, start))
        .map(cand)
        .collect();
    out.push(cand(SnapCurve::Line { origin: start, dir: ax.straight, vp: None }));
    out
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
/// matches `motion`, either way along it. Ties go to the later one, so with
/// one grid the pick is the one it always was.
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
/// - A pan or a fisheye: its centre point to move it, a side point to
///   stretch it, a pan's curve-shape handle (for `bend`), else its horizon
///   to turn it.
pub fn grab_at(kind: GridKind, bend: f32, c: &[P; 4], p: P, tol: f32) -> Option<GridGrab> {
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
        GridKind::PanHorizontal | GridKind::PanVertical | GridKind::PanLoop | GridKind::Fisheye => {
            let f = Frame::of(c)?;
            if near(f.o, 1.5) {
                return Some(GridGrab::Move);
            }
            if let Some(&(_, g)) = side_points(kind, c).iter().find(|(q, _)| near(*q, 1.5)) {
                return Some(g);
            }
            if bend_handle(kind, bend, c).is_some_and(|h| near(h, 1.5)) {
                return Some(GridGrab::Bend);
            }
            (perspective::cross(f.u, sub(p, f.o)).abs() <= tol).then_some(GridGrab::Rotate)
        }
    }
}

/// A non-plane guide's corners after dragging `grab` from `start` to `now`,
/// solved against the corners at the press. `snap` (Shift, once dragging)
/// turns a ruler or a helper line in 15° steps about its other end, and a
/// curved guide by its horizon; on a curved guide's side point it scales
/// the whole guide instead of stretching it one way. `None` for a drag that
/// would leave the guide meaningless — a ruler of no length, helper lines
/// that never meet. A pan's shape handle leaves the corners be (see
/// [`bend_at`]).
pub fn dragged(kind: GridKind, c0: &[P; 4], grab: GridGrab, start: P, now: P, snap: bool) -> Option<[P; 4]> {
    let d = sub(now, start);
    match (kind, grab) {
        (GridKind::Ellipse, _) => perspective::dragged(c0, grab, start, now, snap, false),
        (k, GridGrab::Vp(which)) if k.is_curved() => {
            // Both points of the pair stay mirrored about the centre point,
            // so the side point's distance from it scales the box that way.
            let f = Frame::of(c0)?;
            let axis = if which == 1 { f.n } else { f.u };
            let (d0, d1) = (dot(sub(start, f.o), axis).abs(), dot(sub(now, f.o), axis).abs());
            if d0 < 1e-6 {
                return None;
            }
            let k = (d1 / d0).max(1e-3);
            let (w, h) = match (snap, which) {
                (true, _) => (f.w * k, f.h * k),
                (false, 1) => (f.w, f.h * k),
                (false, _) => (f.w * k, f.h),
            };
            Some(Frame { w, h, ..f }.corners())
        }
        (k, GridGrab::Rotate) if k.is_curved() => perspective::dragged(c0, grab, start, now, snap, false),
        (_, GridGrab::Bend) => Some(*c0),
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
    perspective::cross(a, b).abs() <= 1e-3 * len(a) * len(b)
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
        assert_eq!(grab_at(GridKind::Ruler, 0.0, &c0, [50.0, 3.0], 5.0), Some(GridGrab::Move));
        assert_eq!(grab_at(GridKind::Ruler, 0.0, &c0, [98.0, 2.0], 5.0), Some(GridGrab::Corner(1)));
        assert_eq!(grab_at(GridKind::Ruler, 0.0, &c0, [50.0, 30.0], 5.0), None);
    }

    #[test]
    fn dragging_a_vanishing_point_swings_its_helpers_round_their_far_ends() {
        let (g, s) = fresh_doc(GridKind::Point);
        let c0 = g.doc_corners(&s);
        let v0 = point_vp(&c0).unwrap();
        assert_eq!(grab_at(GridKind::Point, 0.0, &c0, v0, 5.0), Some(GridGrab::Vp(0)));
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
    fn a_pan_wave_crosses_at_its_points_and_runs_on_for_ever() {
        for bend in [-0.75, -0.3, 0.0, 0.5, 1.0] {
            assert_eq!(arch(bend, 0.0), 1.0, "{bend}: a whole unit high in the middle");
            for t in [-3.0, -1.0, 1.0, 3.0] {
                assert!(wave(bend, t).0.abs() < 1e-5, "{bend}: on its point at {t}");
            }
            // Falling all the way from the middle to the point.
            let mut last = 1.0;
            for i in 1..=20 {
                let y = arch(bend, i as f32 / 20.0);
                assert!(y < last, "{bend}: falls at {i}");
                last = y;
            }
            for t in [-1.7, -0.4, 0.3, 0.9, 2.2] {
                let (w0, w1) = (wave(bend, t), wave(bend, t + 4.0));
                assert!((w0.0 - w1.0).abs() < 1e-4 && (w0.1 - w1.1).abs() < 1e-3, "{bend}: one cycle is 4");
                // The slope is the wave's own.
                let e = 1e-3;
                let fd = (wave(bend, t + e).0 - wave(bend, t - e).0) / (2.0 * e);
                assert!((fd - w0.1).abs() < 1e-2, "{bend} at {t}: {fd} vs {}", w0.1);
            }
            // Through a point it goes on smoothly into the next arch, below.
            let (before, after) = (wave(bend, 1.0 - 1e-4), wave(bend, 1.0 + 1e-4));
            assert!((before.1 - after.1).abs() < 1e-2 && before.1 < 0.0, "{bend}: {before:?} {after:?}");
            assert!(wave(bend, 2.0).0 < -0.99, "{bend}: the next arch hangs below");
        }
        // Fuller arches meet their points more steeply.
        assert!(arch_slope(1.0, 1.0) < arch_slope(0.0, 1.0) && arch_slope(0.0, 1.0) < arch_slope(-0.75, 1.0));
        assert!(arch(1.0, 0.5) > arch(0.0, 0.5) && arch(0.0, 0.5) > arch(-0.75, 0.5));
    }

    /// A pan's arch family on a frame turned 30°, `a` = 100, through `p`.
    fn tilted_wave(p: P, bend: f32) -> SnapCurve {
        let (s, c) = (0.5f32, 0.75f32.sqrt());
        let (o, u, n) = ([300.0, 200.0], [c, s], [-s, c]);
        wave_through(o, u, n, 100.0, 0.0, bend, 1.0, p).unwrap()
    }

    #[test]
    fn a_wave_holds_points_on_its_graph_at_the_nearest_spot() {
        for bend in [-0.5, 0.0, 1.0] {
            let start = [340.0, 230.0];
            let c = tilted_wave(start, bend);
            assert!(near(c.project(start), start), "{bend}: the start is on it");
            let SnapCurve::Wave { o, u, n, a, amp, .. } = c else { unreachable!() };
            let on = |x: f32| add(add(o, u.map(|k| k * x)), n.map(|k| k * amp * wave(bend, x / a).0));
            for q in [[400.0, 150.0], [250.0, 260.0], [330.0, 330.0], [500.0, 260.0]] {
                let got = c.project(q);
                // On the graph…
                let d = sub(got, o);
                let x = dot(d, u);
                assert!(len(sub(on(x), got)) < 1e-2, "{bend}: {got:?} off the curve");
                // …and no other point of it is nearer.
                let best = (-4000..=4000)
                    .map(|i| len(sub(on(i as f32 * 0.1), q)))
                    .fold(f32::INFINITY, f32::min);
                assert!(len(sub(got, q)) <= best + 0.05, "{bend}: {q:?} → {got:?}, {} vs {best}", len(sub(got, q)));
                // Its direction there runs along the graph.
                let t = c.tangent_at(q).unwrap();
                let along = normalize(sub(on(x + 0.25), on(x - 0.25))).unwrap();
                assert!(perspective::cross(t, along).abs() < 1e-3, "{bend}: {t:?} vs {along:?}");
            }
        }
    }

    #[test]
    fn a_wave_on_the_horizon_is_the_horizon_and_none_stands_over_a_point() {
        let (o, u, n) = ([0.0, 0.0], [1.0, 0.0], [0.0, 1.0]);
        let flat = wave_through(o, u, n, 100.0, 0.0, 0.0, 1.0, [30.0, 0.0]).unwrap();
        assert!(near(flat.project([60.0, 25.0]), [60.0, 0.0]));
        assert!(near(flat.tangent_at([60.0, 25.0]).unwrap(), [1.0, 0.0]));
        assert!(wave_through(o, u, n, 100.0, 0.0, 0.0, 1.0, [100.0, 40.0]).is_none(), "square over a side point");
        // Shifted a span, the family runs through the centre instead.
        let depth = wave_through(o, u, n, 100.0, 1.0, 0.0, 1.0, [50.0, 30.0]).unwrap();
        assert!(near(depth.project([0.0, 0.0]), [0.0, 0.0]));
        assert!(wave_through(o, u, n, 100.0, 1.0, 0.0, 1.0, [0.0, 30.0]).is_none());
    }

    #[test]
    fn an_arc_in_a_square_box_is_a_circle_through_both_side_points() {
        let (o, u, n) = ([500.0, 500.0], [1.0, 0.0], [0.0, 1.0]);
        for start in [[560.0, 440.0], [450.0, 580.0], [700.0, 300.0]] {
            let c = arc_through(o, u, n, 100.0, 100.0, start);
            assert!(near(c.project(start), start), "{start:?} is on it");
            for side in [[400.0, 500.0], [600.0, 500.0]] {
                assert!(c.distance(side) < 1e-2, "{start:?}: through {side:?}");
            }
            let SnapCurve::Arc { apex, .. } = c else { unreachable!() };
            assert_eq!(apex > 0.0, start[1] > 500.0, "{start:?}: its middle on the start's side");
            // Projected straight out from the circle's middle, at one radius,
            // its direction square to the radius.
            let (k, r) = arc_circle(apex);
            let mid = [500.0, 500.0 + 100.0 * k];
            for q in [[520.0, 420.0], [380.0, 610.0]] {
                let got = c.project(q);
                assert!((len(sub(got, mid)) - 100.0 * r).abs() < 1e-2);
                assert!(perspective::cross(sub(got, mid), sub(q, mid)).abs() < 1e-1, "radially out");
                assert!(dot(c.tangent_at(q).unwrap(), sub(got, mid)).abs() < 1e-2);
            }
        }
        // On the axis: the straight line along it.
        assert!(matches!(arc_through(o, u, n, 100.0, 100.0, [530.0, 500.0]), SnapCurve::Line { .. }));
    }

    #[test]
    fn a_fisheye_arc_in_a_long_box_passes_its_points_and_the_widest_is_the_rim() {
        let (o, u, n) = ([0.0, 0.0], [1.0, 0.0], [0.0, 1.0]);
        let c = arc_through(o, u, n, 200.0, 100.0, [80.0, -40.0]);
        assert!(c.distance([200.0, 0.0]) < 1e-2 && c.distance([-200.0, 0.0]) < 1e-2);
        let t = c.tangent_at([80.0, -40.0]).unwrap();
        let e = 0.5;
        let along = normalize(sub(c.project([80.0 + e, -40.0]), c.project([80.0 - e, -40.0]))).unwrap();
        assert!(perspective::cross(t, along).abs() < 1e-2, "{t:?} vs {along:?}");
        // Through the top side point: the rim, an ellipse 200 by 100.
        let rim = arc_through(o, u, n, 200.0, 100.0, [0.0, -100.0]);
        for th in [0.3f32, 1.0, 2.0] {
            let p = [200.0 * th.cos(), -100.0 * th.sin()];
            assert!(rim.distance(p) < 1e-2, "{p:?}");
        }
        let pts = rim.bend_points([-1e4, -1e4], [1e4, 1e4]).concat();
        assert!(near(pts[0], [200.0, 0.0]) && near(*pts.last().unwrap(), [-200.0, 0.0]));
        assert!(pts.iter().all(|p| p[1] <= 1e-3), "round the top, by way of its apex");
    }

    fn doc_frame(g: &PerspectiveGrid, s: &Space) -> Frame {
        Frame::of(&g.doc_corners(s)).unwrap()
    }

    #[test]
    fn a_horizontal_pan_bends_its_level_lines_through_the_side_points() {
        let (g, s) = fresh_doc(GridKind::PanHorizontal);
        let f = doc_frame(&g, &s);
        assert_eq!((f.o, f.u, f.n), ([500.0, 500.0], [1.0, 0.0], [0.0, 1.0]));
        let (l, r) = ([500.0 - f.w, 500.0], [500.0 + f.w, 500.0]);
        let start = [700.0, 380.0];
        let got = candidates(&g, &s, start, false, 2);
        assert_eq!(got.len(), 3);
        assert!(got.iter().all(|c| c.grid == 2 && !c.fixed));
        let (across, away, up) = (got[0].curve, got[1].curve, got[2].curve);
        for c in [across, away] {
            assert!(c.distance(start) < 1e-2, "through the start");
        }
        assert!(across.distance(l) < 1e-2 && across.distance(r) < 1e-2, "an arch between the side points");
        assert!(away.distance(f.o) < 1e-2, "a depth line through the centre point");
        assert!(matches!(up, SnapCurve::Line { dir: [0.0, 1.0], vp: None, .. }), "verticals stay straight");
        // A stroke heading along the arch picks it.
        let heading = across.tangent_at(start).unwrap();
        assert_eq!(pick(&got, start, heading).unwrap().curve, across);
        // Fuller arches stand taller between the same points.
        let mut full = g.clone();
        full.bend = 1.0;
        let tall = candidates(&full, &s, start, false, 0)[0].curve;
        let SnapCurve::Wave { amp: a0, .. } = across else { unreachable!() };
        let SnapCurve::Wave { amp: a1, .. } = tall else { unreachable!() };
        assert!(a1.abs() < a0.abs(), "{a1} vs {a0}: the start is nearer a fuller arch's top");
    }

    #[test]
    fn a_vertical_pan_bends_its_verticals_through_points_above_and_below() {
        let (g, s) = fresh_doc(GridKind::PanVertical);
        let f = doc_frame(&g, &s);
        let (top, bottom) = ([500.0, 500.0 - f.h], [500.0, 500.0 + f.h]);
        let got = candidates(&g, &s, [560.0, 300.0], false, 0);
        assert!(got[0].curve.distance(top) < 1e-2 && got[0].curve.distance(bottom) < 1e-2);
        assert!(got[1].curve.distance(f.o) < 1e-2);
        assert!(matches!(got[2].curve, SnapCurve::Line { dir: [1.0, 0.0], .. }), "horizontals stay straight");
    }

    #[test]
    fn a_360_pan_puts_four_points_to_its_box_and_runs_on_past_it() {
        let (g, s) = fresh_doc(GridKind::PanLoop);
        let f = doc_frame(&g, &s);
        let quarter = f.w * 0.5;
        let got = candidates(&g, &s, [530.0, 400.0], false, 0);
        let (across, away) = (got[0].curve, got[1].curve);
        assert!(matches!(across, SnapCurve::Wave { span, .. } if span.is_infinite()));
        for k in [-3.0, -1.0, 1.0, 3.0, 5.0] {
            assert!(across.distance([500.0 + quarter * k, 500.0]) < 1e-2, "an arch ends at {k}");
        }
        for k in [-4.0, -2.0, 0.0, 2.0, 4.0] {
            assert!(away.distance([500.0 + quarter * k, 500.0]) < 1e-2, "a depth curve crosses at {k}");
        }
        // The box's ends are one turn apart: the curves there match.
        let SnapCurve::Wave { amp, bend, a, .. } = across else { unreachable!() };
        let (w0, w1) = (amp * wave(bend, -f.w / a).0, amp * wave(bend, f.w / a).0);
        assert!((w0 - w1).abs() < 1e-3, "it loops");
    }

    #[test]
    fn a_fisheye_arcs_both_ways_and_runs_depth_lines_to_its_centre() {
        let (g, s) = fresh_doc(GridKind::Fisheye);
        let f = doc_frame(&g, &s);
        let start = [600.0, 420.0];
        let got = candidates(&g, &s, start, false, 0);
        assert_eq!(got.len(), 3);
        let sides = [[500.0 - f.w, 500.0], [500.0 + f.w, 500.0]];
        let ends = [[500.0, 500.0 - f.h], [500.0, 500.0 + f.h]];
        assert!(sides.iter().all(|&p| got[0].curve.distance(p) < 1e-2), "horizontals through the sides");
        assert!(ends.iter().all(|&p| got[1].curve.distance(p) < 1e-2), "verticals through top and bottom");
        assert!(matches!(got[2].curve, SnapCurve::Line { vp: Some(v), .. } if near(v, f.o)));
        for c in &got {
            assert!(c.curve.distance(start) < 1e-2);
        }
        // Slid elsewhere, a fisheye's arc keeps to its family.
        let moved = got[0].curve.through([450.0, 600.0]);
        assert!(moved.distance([450.0, 600.0]) < 1e-2 && moved.distance(sides[1]) < 1e-2);
    }

    #[test]
    fn a_wave_draws_inside_its_span_and_stays_on_the_canvas() {
        let (g, s) = fresh_doc(GridKind::PanHorizontal);
        let f = doc_frame(&g, &s);
        let c = candidates(&g, &s, [600.0, 420.0], false, 0)[0].curve;
        let runs = c.bend_points([-5000.0, 0.0], [5000.0, 1000.0]);
        let pts = runs.concat();
        assert!(!pts.is_empty());
        assert!(pts.iter().all(|p| (p[0] - 500.0).abs() <= f.w + 1e-2), "between its side points only");
        assert!(pts.iter().all(|&p| c.distance(p) < 1e-2));
        // Clear of the box, nothing.
        assert!(c.bend_points([3000.0, 0.0], [4000.0, 1000.0]).is_empty());
    }

    #[test]
    fn a_curved_guide_moves_by_its_centre_stretches_by_its_points_and_turns_by_its_horizon() {
        let (g, s) = fresh_doc(GridKind::PanHorizontal);
        let c0 = g.doc_corners(&s);
        let f = Frame::of(&c0).unwrap();
        let grab = |p: P| grab_at(GridKind::PanHorizontal, 0.0, &c0, p, 5.0);
        assert_eq!(grab([502.0, 499.0]), Some(GridGrab::Move));
        assert_eq!(grab([500.0 + f.w, 503.0]), Some(GridGrab::Vp(0)));
        assert_eq!(grab([500.0 - f.w, 500.0]), Some(GridGrab::Vp(0)));
        let handle = bend_handle(GridKind::PanHorizontal, 0.0, &c0).unwrap();
        assert!(near(handle, [500.0 + f.w / 2.0, 500.0 - f.h * 0.75]), "top right, on the outer arch");
        assert_eq!(grab(handle), Some(GridGrab::Bend));
        assert_eq!(grab([200.0, 503.0]), Some(GridGrab::Rotate), "anywhere on the horizon");
        assert_eq!(grab([200.0, 300.0]), None);

        // A side point stretches the box its way only, both sides alike.
        let r = [500.0 + f.w, 500.0];
        let c = dragged(GridKind::PanHorizontal, &c0, GridGrab::Vp(0), r, [500.0 + f.w * 1.5, 520.0], false).unwrap();
        let f1 = Frame::of(&c).unwrap();
        assert!((f1.w - f.w * 1.5).abs() < 1e-2 && (f1.h - f.h).abs() < 1e-2 && near(f1.o, f.o));
        assert!(near(f1.u, f.u), "it doesn't turn");
        // With Shift the whole guide scales.
        let c = dragged(GridKind::PanHorizontal, &c0, GridGrab::Vp(0), r, [500.0 + f.w * 1.5, 500.0], true).unwrap();
        let f1 = Frame::of(&c).unwrap();
        assert!((f1.w - f.w * 1.5).abs() < 1e-2 && (f1.h - f.h * 1.5).abs() < 1e-2);
        // The horizon turns it about the centre point; Shift in 15° steps.
        let from = [800.0, 500.0];
        let to = [500.0 + 300.0 * 0.4f32.cos(), 500.0 + 300.0 * 0.4f32.sin()];
        let c = dragged(GridKind::PanHorizontal, &c0, GridGrab::Rotate, from, to, false).unwrap();
        let f1 = Frame::of(&c).unwrap();
        assert!((f1.u[1].atan2(f1.u[0]) - 0.4).abs() < 1e-3 && near(f1.o, f.o));
        let c = dragged(GridKind::PanHorizontal, &c0, GridGrab::Rotate, from, to, true).unwrap();
        let f1 = Frame::of(&c).unwrap();
        assert!((f1.u[1].atan2(f1.u[0]).to_degrees() - 30.0).abs() < 1e-2);
        // The centre point moves it.
        let c = dragged(GridKind::PanHorizontal, &c0, GridGrab::Move, f.o, [530.0, 480.0], false).unwrap();
        assert!(near(Frame::of(&c).unwrap().o, [530.0, 480.0]));
    }

    #[test]
    fn each_curved_kind_has_its_own_side_points() {
        let points = |kind| {
            let (g, s) = fresh_doc(kind);
            side_points(kind, &g.doc_corners(&s)).iter().map(|&(_, g)| g).collect::<Vec<_>>()
        };
        use GridGrab::Vp;
        assert_eq!(points(GridKind::PanHorizontal), [Vp(0), Vp(0)]);
        assert_eq!(points(GridKind::PanVertical), [Vp(1), Vp(1)]);
        assert_eq!(points(GridKind::PanLoop), [Vp(0), Vp(0), Vp(2), Vp(2)]);
        assert_eq!(points(GridKind::Fisheye), [Vp(0), Vp(0), Vp(1), Vp(1)]);
        // A 360° pan's inner points stretch the turn as much again.
        let (g, s) = fresh_doc(GridKind::PanLoop);
        let c0 = g.doc_corners(&s);
        let f = Frame::of(&c0).unwrap();
        let inner = [500.0 + f.w / 2.0, 500.0];
        let c = dragged(GridKind::PanLoop, &c0, Vp(2), inner, [500.0 + f.w, 500.0], false).unwrap();
        assert!((Frame::of(&c).unwrap().w - f.w * 2.0).abs() < 1e-2);
        // A fisheye's top point stretches it up and down only.
        let (g, s) = fresh_doc(GridKind::Fisheye);
        let c0 = g.doc_corners(&s);
        let f = Frame::of(&c0).unwrap();
        let top = [500.0, 500.0 - f.h];
        assert_eq!(grab_at(GridKind::Fisheye, 0.0, &c0, top, 5.0), Some(Vp(1)));
        let c = dragged(GridKind::Fisheye, &c0, Vp(1), top, [510.0, 500.0 - f.h * 0.5], false).unwrap();
        let f1 = Frame::of(&c).unwrap();
        assert!((f1.h - f.h * 0.5).abs() < 1e-2 && (f1.w - f.w).abs() < 1e-2);
        assert!(bend_handle(GridKind::Fisheye, 0.0, &c0).is_none(), "no curve-shape handle: its points shape it");
    }

    #[test]
    fn a_pans_shape_handle_sets_its_bend_from_how_far_out_it_is() {
        for kind in [GridKind::PanHorizontal, GridKind::PanVertical, GridKind::PanLoop] {
            let (g, s) = fresh_doc(kind);
            let c = g.doc_corners(&s);
            for bend in [-0.5, 0.0, 0.6] {
                let h = bend_handle(kind, bend, &c).unwrap();
                let back = bend_at(kind, &c, h).unwrap();
                assert!((back - bend).abs() < 1e-4, "{kind:?}: {bend} → {back}");
            }
            // Out to the box's edge and past: as full as it goes; in to the
            // horizon: as flat.
            let f = Frame::of(&c).unwrap();
            let ax = pan_axis(kind, &f).unwrap();
            let out = add(f.o, ax.n.map(|k| k * ax.side * ax.reach * 2.0));
            assert_eq!(bend_at(kind, &c, out), Some(*BEND.end()), "{kind:?}");
            assert_eq!(bend_at(kind, &c, f.o), Some(*BEND.start()), "{kind:?}");
        }
        // A vertical pan's handle is at the bottom right, as in Harmony.
        let (g, s) = fresh_doc(GridKind::PanVertical);
        let h = bend_handle(GridKind::PanVertical, 0.0, &g.doc_corners(&s)).unwrap();
        assert!(h[0] > 500.0 && h[1] > 500.0, "{h:?}");
    }

    #[test]
    fn pull_goes_part_way() {
        assert_eq!(pull([0.0, 0.0], [10.0, 20.0], 1.0), [10.0, 20.0]);
        assert!(near(pull([0.0, 0.0], [10.0, 20.0], 0.5), [5.0, 10.0]));
        assert_eq!(pull([3.0, 4.0], [10.0, 20.0], 0.0), [3.0, 4.0]);
    }
}
