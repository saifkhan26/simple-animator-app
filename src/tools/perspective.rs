//! Perspective grids — drawing guides that live in the viewport, not on a
//! layer.
//!
//! A grid is a plane seen in perspective: four free corners (TL, TR, BR, BL)
//! define a projective map from the unit square, so its rows and columns
//! foreshorten the way a tiled floor does. The vanishing points are never
//! placed by hand — they fall out of the corners. Opposite edges that stay
//! parallel give a point at infinity, so a trapezoid is one-point perspective
//! and a general quad is two-point.
//!
//! Grids are a workspace preference, not project data. Corners are stored
//! *frame-relative* — offset from the frame centre, in units of the frame
//! height — so a grid set up in one project keeps its shape, and stays
//! centred, in a project of another resolution.

use serde::{Deserialize, Serialize};

/// A point or vector in document space.
pub type P = [f32; 2];

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}
fn dot(a: P, b: P) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: P, b: P) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}
fn len(a: P) -> f32 {
    a[0].hypot(a[1])
}
fn normalize(a: P) -> Option<P> {
    let l = len(a);
    (l > 1e-9).then(|| [a[0] / l, a[1] / l])
}

/// Projective map from the unit square onto a quad: `(u, v)` → document
/// point. Row-major 3×3, applied to `(u, v, 1)`.
#[derive(Clone, Copy, Debug)]
pub struct Homography {
    m: [f64; 9],
}

impl Homography {
    /// The map taking (0,0), (1,0), (1,1), (0,1) to TL, TR, BR, BL.
    /// Heckbert's closed form. `None` when three corners are collinear.
    pub fn square_to_quad(c: &[P; 4]) -> Option<Self> {
        let [x0, y0] = [c[0][0] as f64, c[0][1] as f64];
        let [x1, y1] = [c[1][0] as f64, c[1][1] as f64];
        let [x2, y2] = [c[2][0] as f64, c[2][1] as f64];
        let [x3, y3] = [c[3][0] as f64, c[3][1] as f64];
        let sx = x0 - x1 + x2 - x3;
        let sy = y0 - y1 + y2 - y3;
        let (dx1, dx2, dy1, dy2) = (x1 - x2, x3 - x2, y1 - y2, y3 - y2);
        let den = dx1 * dy2 - dx2 * dy1;
        if den.abs() < 1e-12 {
            return None;
        }
        // A parallelogram has sx = sy = 0, so g = h = 0 and this reduces to
        // the affine map — one formula covers both.
        let g = (sx * dy2 - dx2 * sy) / den;
        let h = (dx1 * sy - sx * dy1) / den;
        let m = [
            x1 - x0 + g * x1,
            x3 - x0 + h * x3,
            x0,
            y1 - y0 + g * y1,
            y3 - y0 + h * y3,
            y0,
            g,
            h,
            1.0,
        ];
        // `den` only sees the corners around BR; three collinear corners
        // elsewhere slip past it and leave the map singular.
        let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
            + m[2] * (m[3] * m[7] - m[4] * m[6]);
        let scale = m[0].abs().max(m[1].abs()).max(m[3].abs()).max(m[4].abs());
        if !det.is_finite() || det.abs() <= 1e-9 * scale * scale {
            return None;
        }
        Some(Self { m })
    }

    fn apply(&self, x: f64, y: f64, w: f64) -> [f64; 3] {
        let m = &self.m;
        [
            m[0] * x + m[1] * y + m[2] * w,
            m[3] * x + m[4] * y + m[5] * w,
            m[6] * x + m[7] * y + m[8] * w,
        ]
    }

    pub fn map(&self, u: f32, v: f32) -> P {
        let [x, y, w] = self.apply(u as f64, v as f64, 1.0);
        [(x / w) as f32, (y / w) as f32]
    }

    /// Where the image of a direction in the square ends up: a finite
    /// vanishing point, or a direction when those lines stay parallel.
    fn vanish(&self, du: f64, dv: f64, near: P, size: f32) -> Vp {
        let [x, y, w] = self.apply(du, dv, 0.0);
        let dir = normalize([x as f32, y as f32]).unwrap_or([1.0, 0.0]);
        if w.abs() > 1e-12 {
            let p = [(x / w) as f32, (y / w) as f32];
            // Past a thousand grid-widths away the lines are parallel to
            // within a hair; treating the point as finite would only lose
            // precision.
            if p[0].is_finite() && p[1].is_finite() && len(sub(p, near)) < size * 1000.0 {
                return Vp::Point(p);
            }
        }
        Vp::Dir(dir)
    }
}

/// A vanishing point: finite, or a direction (point at infinity).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Vp {
    Point(P),
    Dir(P),
}

impl Vp {
    /// Unit direction of the family's line through `at`.
    pub fn dir_at(self, at: P) -> Option<P> {
        match self {
            Vp::Point(p) => normalize(sub(p, at)),
            Vp::Dir(d) => Some(d),
        }
    }
}

/// An infinite line: a point on it and a unit direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub p: P,
    pub d: P,
}

/// What a press on a grid grabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridGrab {
    Corner(u8),
    /// A finite vanishing point: 0 the rows', 1 the columns'.
    Vp(u8),
    Rotate,
    Move,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PerspectiveGrid {
    /// TL, TR, BR, BL, frame-relative (see the module docs).
    pub corners: [P; 4],
    pub rows: u32,
    pub cols: u32,
    pub color: [u8; 3],
    pub opacity: f32,
    /// Line weight in screen pixels.
    pub weight: f32,
    pub visible: bool,
    /// A locked grid can still be selected, but not dragged.
    pub locked: bool,
    /// Carry every grid line out to its vanishing point.
    pub extend: bool,
    pub horizon: bool,
}

pub const MAX_DIVISIONS: u32 = 64;

impl Default for PerspectiveGrid {
    /// A floor-like trapezoid in the middle of the frame: one-point
    /// perspective, so the grid reads as perspective the moment it appears.
    fn default() -> Self {
        Self {
            corners: [[-0.25, -0.1], [0.25, -0.1], [0.45, 0.3], [-0.45, 0.3]],
            rows: 6,
            cols: 6,
            color: [80, 150, 255],
            opacity: 0.8,
            weight: 1.0,
            visible: true,
            locked: false,
            extend: false,
            horizon: true,
        }
    }
}

impl PerspectiveGrid {
    /// Corners in document space for a `w`×`h` frame.
    pub fn doc_corners(&self, w: f32, h: f32) -> [P; 4] {
        self.corners.map(|n| to_doc(n, w, h))
    }

    pub fn set_doc_corners(&mut self, c: [P; 4], w: f32, h: f32) {
        self.corners = c.map(|p| from_doc(p, w, h));
    }

    /// Rotate about the corner centroid. Frame-relative storage is a uniform
    /// scale plus a translation of document space, so rotating it directly is
    /// the same as rotating in the document.
    pub fn rotate(&mut self, angle: f32) {
        self.corners = rotate(&self.corners, angle);
    }
}

pub fn to_doc(n: P, w: f32, h: f32) -> P {
    [w * 0.5 + n[0] * h, h * 0.5 + n[1] * h]
}

pub fn from_doc(p: P, w: f32, h: f32) -> P {
    let h = h.max(1.0);
    [(p[0] - w * 0.5) / h, (p[1] - h * 0.5) / h]
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PerspectiveConfig {
    pub grids: Vec<PerspectiveGrid>,
    /// Index into `grids` of the grid being edited, and the one strokes snap
    /// to.
    pub active: usize,
    /// Draw the grids while other tools are active. The perspective tool
    /// always shows them.
    pub show: bool,
    /// Lock brush strokes to the active grid's lines.
    pub snap: bool,
    /// Also offer the vertical as a snap direction. Off by default: near the
    /// middle of a one-point grid the columns are nearly vertical too, and a
    /// stroke meant to run to the vanishing point would lock straight up.
    pub snap_vertical: bool,
}

impl Default for PerspectiveConfig {
    fn default() -> Self {
        Self {
            grids: vec![PerspectiveGrid::default()],
            active: 0,
            show: false,
            snap: false,
            snap_vertical: false,
        }
    }
}

impl PerspectiveConfig {
    pub fn active_grid(&self) -> Option<&PerspectiveGrid> {
        self.grids.get(self.active)
    }

    pub fn active_grid_mut(&mut self) -> Option<&mut PerspectiveGrid> {
        self.grids.get_mut(self.active)
    }

    /// Remove grid `i`, keeping `active` on the same grid where it can.
    pub fn remove(&mut self, i: usize) {
        if i >= self.grids.len() {
            return;
        }
        self.grids.remove(i);
        if self.active > i || self.active >= self.grids.len() {
            self.active = self.active.saturating_sub(1);
        }
    }
}

/// Everything derived from a grid's corners that drawing and snapping need.
#[derive(Clone, Debug)]
pub struct Plane {
    pub h: Homography,
    /// Where lines of constant v (the rows, running along u) converge.
    pub vp_rows: Vp,
    /// Where lines of constant u (the columns, running along v) converge.
    pub vp_cols: Vp,
}

impl Plane {
    pub fn new(corners: [P; 4]) -> Option<Self> {
        let h = Homography::square_to_quad(&corners)?;
        let size = corners
            .iter()
            .map(|&c| len(sub(c, corners[0])))
            .fold(1.0f32, f32::max);
        Some(Self {
            h,
            vp_rows: h.vanish(1.0, 0.0, corners[0], size),
            vp_cols: h.vanish(0.0, 1.0, corners[0], size),
        })
    }

    /// Row and column segments across the quad, borders included. Each is
    /// tagged with the vanishing point its line runs to.
    pub fn grid_lines(&self, rows: u32, cols: u32) -> Vec<(P, P, Vp)> {
        let (rows, cols) = (rows.clamp(1, MAX_DIVISIONS), cols.clamp(1, MAX_DIVISIONS));
        let mut out = Vec::with_capacity((rows + cols + 2) as usize);
        for i in 0..=cols {
            let u = i as f32 / cols as f32;
            out.push((self.h.map(u, 0.0), self.h.map(u, 1.0), self.vp_cols));
        }
        for i in 0..=rows {
            let v = i as f32 / rows as f32;
            out.push((self.h.map(0.0, v), self.h.map(1.0, v), self.vp_rows));
        }
        out
    }

    /// The plane's vanishing line. `None` when both families stay parallel —
    /// a plane seen square-on has its horizon at infinity.
    ///
    /// Its point is the one nearest the grid, not a vanishing point: a nearly
    /// parallel pair of edges puts its vanishing point hundreds of grid-widths
    /// away, and once the view zooms in that is far enough out for `f32`
    /// screen coordinates to lose the line's angle.
    pub fn horizon(&self) -> Option<Line> {
        let (p, d) = match (self.vp_rows, self.vp_cols) {
            (Vp::Point(a), Vp::Point(b)) => (a, normalize(sub(b, a))?),
            (Vp::Point(p), Vp::Dir(d)) | (Vp::Dir(d), Vp::Point(p)) => (p, d),
            (Vp::Dir(_), Vp::Dir(_)) => return None,
        };
        Some(Line {
            p: project(p, d, self.h.map(0.5, 0.5)),
            d,
        })
    }

    /// Candidate snap directions at `at`: toward each vanishing point, plus,
    /// with `vertical`, the vertical — perpendicular to the horizon, or to
    /// the rows when the horizon is at infinity.
    pub fn snap_dirs(&self, at: P, vertical: bool) -> Vec<P> {
        let mut out = Vec::with_capacity(3);
        out.extend(self.vp_rows.dir_at(at));
        out.extend(self.vp_cols.dir_at(at));
        if vertical {
            let across = match self.horizon() {
                Some(l) => Some(l.d),
                None => self.vp_rows.dir_at(at),
            };
            if let Some(d) = across {
                out.push([-d[1], d[0]]);
            }
        }
        out
    }

    /// Vanishing point 0 (rows) or 1 (columns), when finite.
    pub fn vp_point(&self, which: u8) -> Option<P> {
        match if which == 0 { self.vp_rows } else { self.vp_cols } {
            Vp::Point(p) => Some(p),
            Vp::Dir(_) => None,
        }
    }
}

/// Whether the quad is strictly convex, in either winding. A corner drag that
/// would fold it into a bow-tie or a dart is refused.
pub fn is_convex(c: &[P; 4]) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let e0 = sub(c[(i + 1) % 4], c[i]);
        let e1 = sub(c[(i + 2) % 4], c[(i + 1) % 4]);
        let z = cross(e0, e1);
        // Relative to the edge lengths, so the test doesn't depend on scale.
        if z.abs() <= 1e-4 * len(e0) * len(e1) {
            return false;
        }
        if sign == 0.0 {
            sign = z.signum();
        } else if z.signum() != sign {
            return false;
        }
    }
    true
}

pub fn centroid(c: &[P; 4]) -> P {
    [
        (c[0][0] + c[1][0] + c[2][0] + c[3][0]) * 0.25,
        (c[0][1] + c[1][1] + c[2][1] + c[3][1]) * 0.25,
    ]
}

/// Rotate the corners about their centroid.
pub fn rotate(c: &[P; 4], angle: f32) -> [P; 4] {
    let o = centroid(c);
    let (s, co) = angle.sin_cos();
    c.map(|p| {
        let d = sub(p, o);
        [o[0] + d[0] * co - d[1] * s, o[1] + d[0] * s + d[1] * co]
    })
}

fn inside(c: &[P; 4], p: P) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let z = cross(sub(c[(i + 1) % 4], c[i]), sub(p, c[i]));
        if z == 0.0 {
            continue;
        }
        if sign == 0.0 {
            sign = z.signum();
        } else if z.signum() != sign {
            return false;
        }
    }
    true
}

/// What a press at `p` grabs on a grid with corners `c`, with `tol` the
/// handle radius in document units. Corners win, then the vanishing points,
/// then a ring just outside the corners rotates, then anywhere inside moves.
pub fn grab_at(c: &[P; 4], p: P, tol: f32) -> Option<GridGrab> {
    let (i, d) = c
        .iter()
        .enumerate()
        .map(|(i, &k)| (i, len(sub(p, k))))
        .fold((0, f32::INFINITY), |a, b| if b.1 < a.1 { b } else { a });
    if d <= tol {
        return Some(GridGrab::Corner(i as u8));
    }
    if let Some(plane) = Plane::new(*c) {
        for which in 0..2 {
            if plane
                .vp_point(which)
                .is_some_and(|v| len(sub(p, v)) <= tol * 1.5)
            {
                return Some(GridGrab::Vp(which));
            }
        }
    }
    let is_inside = inside(c, p);
    if !is_inside && d <= tol * 4.0 {
        return Some(GridGrab::Rotate);
    }
    is_inside.then_some(GridGrab::Move)
}

/// The corners after dragging `grab` from `start` to `now`, solved against
/// the corners at press time rather than accumulated. `snap` rounds a
/// rotation to 15° steps. A corner drag that would break convexity returns
/// `None`.
pub fn dragged(start_c: &[P; 4], grab: GridGrab, start: P, now: P, snap: bool) -> Option<[P; 4]> {
    let delta = sub(now, start);
    match grab {
        GridGrab::Move => Some(start_c.map(|p| add(p, delta))),
        GridGrab::Corner(i) => {
            let mut c = *start_c;
            c[i as usize] = add(c[i as usize], delta);
            is_convex(&c).then_some(c)
        }
        GridGrab::Vp(which) => {
            let v0 = Plane::new(*start_c)?.vp_point(which)?;
            move_vp(start_c, which, add(v0, delta))
        }
        GridGrab::Rotate => {
            let o = centroid(start_c);
            let a0 = (start[1] - o[1]).atan2(start[0] - o[0]);
            let a1 = (now[1] - o[1]).atan2(now[0] - o[0]);
            let mut a = a1 - a0;
            if snap {
                let step = std::f32::consts::PI / 12.0;
                a = (a / step).round() * step;
            }
            Some(rotate(start_c, a))
        }
    }
}

/// Where `p + s·d` meets `q + r·e`, or `None` for parallel lines.
fn intersect(p: P, d: P, q: P, e: P) -> Option<P> {
    let den = cross(d, e);
    if den.abs() <= 1e-9 * len(d) * len(e) {
        return None;
    }
    let s = cross(sub(q, p), e) / den;
    Some([p[0] + d[0] * s, p[1] + d[1] * s])
}

/// The corners re-aimed so vanishing point `which` (0 rows, 1 columns) sits
/// at `v`, from corners whose vanishing point `which` is finite.
///
/// The two edges that run to it swing to meet at `v`. Of the two edges
/// across them, the one farther from the vanishing point — the near edge of
/// the plane — stays put. The far edge keeps its fraction of the way toward
/// the vanishing point and keeps running to the *other* vanishing point, so
/// dragging one never disturbs the other. `None` if that would fold the quad.
pub fn move_vp(c: &[P; 4], which: u8, v: P) -> Option<[P; 4]> {
    let plane = Plane::new(*c)?;
    let v0 = plane.vp_point(which)?;
    let other = if which == 0 {
        plane.vp_cols
    } else {
        plane.vp_rows
    };
    // The converging edges as (corner, partner) pairs: the first corners form
    // one cross edge, the partners the other.
    let (a, b) = if which == 0 {
        ([0, 3], [1, 2])
    } else {
        ([0, 1], [3, 2])
    };
    let mid = |i: [usize; 2]| {
        [
            (c[i[0]][0] + c[i[1]][0]) * 0.5,
            (c[i[0]][1] + c[i[1]][1]) * 0.5,
        ]
    };
    let (kept, moving) = if len(sub(mid(a), v0)) >= len(sub(mid(b), v0)) {
        (a, b)
    } else {
        (b, a)
    };
    let (k0, k1) = (c[kept[0]], c[kept[1]]);
    let to_v0 = sub(v0, k0);
    let t = dot(sub(c[moving[0]], k0), to_v0) / dot(to_v0, to_v0).max(1e-12);
    let n0 = add(k0, sub(v, k0).map(|x| x * t));
    let n1 = intersect(n0, other.dir_at(n0)?, k1, sub(v, k1))?;
    let mut out = *c;
    out[moving[0]] = n0;
    out[moving[1]] = n1;
    is_convex(&out).then_some(out)
}

/// Of `dirs`, the one most nearly parallel to `motion` (either sense).
pub fn pick_dir(dirs: &[P], motion: P) -> Option<P> {
    let m = normalize(motion)?;
    dirs.iter()
        .copied()
        .max_by(|a, b| dot(*a, m).abs().total_cmp(&dot(*b, m).abs()))
}

/// `p` projected onto the line through `origin` along unit `dir`.
pub fn project(origin: P, dir: P, p: P) -> P {
    let t = dot(sub(p, origin), dir);
    [origin[0] + dir[0] * t, origin[1] + dir[1] * t]
}

/// Parameter range `[t0, t1]` of the infinite line `a + t·(b − a)` inside the
/// rectangle `lo..hi`, or `None` when it misses. Liang–Barsky.
pub fn clip_line(a: P, b: P, lo: P, hi: P) -> Option<(f32, f32)> {
    let d = sub(b, a);
    let (mut t0, mut t1) = (f32::NEG_INFINITY, f32::INFINITY);
    for k in 0..2 {
        if d[k].abs() < 1e-9 {
            if a[k] < lo[k] || a[k] > hi[k] {
                return None;
            }
            continue;
        }
        let (mut ta, mut tb) = ((lo[k] - a[k]) / d[k], (hi[k] - a[k]) / d[k]);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        t0 = t0.max(ta);
        t1 = t1.min(tb);
    }
    (t0 <= t1).then_some((t0, t1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: P, b: P) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    const TRAP: [P; 4] = [[40.0, 0.0], [60.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
    const QUAD: [P; 4] = [[10.0, 5.0], [90.0, 20.0], [80.0, 70.0], [0.0, 90.0]];

    #[test]
    fn the_homography_hits_the_corners() {
        let h = Homography::square_to_quad(&QUAD).unwrap();
        for (uv, c) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
            .iter()
            .zip(QUAD)
        {
            assert!(close(h.map(uv.0, uv.1), c), "{uv:?}");
        }
        // The centre of the square lands on the intersection of the
        // diagonals — true of any projective map, not of a bilinear one.
        let c = h.map(0.5, 0.5);
        let (d0, d1) = (sub(QUAD[2], QUAD[0]), sub(QUAD[3], QUAD[1]));
        assert!(cross(d0, sub(c, QUAD[0])).abs() < 1e-2 * len(d0));
        assert!(cross(d1, sub(c, QUAD[1])).abs() < 1e-2 * len(d1));
    }

    #[test]
    fn collinear_corners_have_no_homography() {
        let flat = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [0.0, 1.0]];
        assert!(Homography::square_to_quad(&flat).is_none());
    }

    #[test]
    fn a_rectangle_has_both_vanishing_points_at_infinity() {
        let p = Plane::new([[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]]).unwrap();
        assert!(matches!(p.vp_rows, Vp::Dir(d) if d[1].abs() < 1e-6));
        assert!(matches!(p.vp_cols, Vp::Dir(d) if d[0].abs() < 1e-6));
        assert!(p.horizon().is_none());
    }

    #[test]
    fn a_trapezoid_is_one_point_perspective() {
        let p = Plane::new(TRAP).unwrap();
        // The slanted sides meet at (50, -25).
        match p.vp_cols {
            Vp::Point(v) => assert!(close(v, [50.0, -25.0]), "{v:?}"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(p.vp_rows, Vp::Dir(_)));
        let hz = p.horizon().unwrap();
        // Anchored above the grid's centre, on the line through the VP.
        assert!((hz.p[1] + 25.0).abs() < 1e-3, "{:?}", hz.p);
        assert!(hz.d[1].abs() < 1e-6, "horizon is horizontal");
    }

    #[test]
    fn a_general_quad_is_two_point_and_its_horizon_joins_them() {
        let p = Plane::new(QUAD).unwrap();
        let (Vp::Point(a), Vp::Point(b)) = (p.vp_rows, p.vp_cols) else {
            panic!("expected two finite vanishing points");
        };
        // Each VP lies on the extensions of its pair of edges.
        let on =
            |p: P, q0: P, q1: P| cross(sub(q1, q0), sub(p, q0)).abs() < 1e-1 * len(sub(q1, q0));
        assert!(on(a, QUAD[0], QUAD[1]) && on(a, QUAD[3], QUAD[2]));
        assert!(on(b, QUAD[0], QUAD[3]) && on(b, QUAD[1], QUAD[2]));
        let hz = p.horizon().unwrap();
        // Through both vanishing points...
        let off = |v: P| cross(hz.d, sub(v, hz.p)).abs() / len(sub(v, hz.p)).max(1.0);
        assert!(off(a) < 1e-3 && off(b) < 1e-3);
        // ...but anchored near the grid, not out at either of them.
        let c = centroid(&QUAD);
        assert!(len(sub(hz.p, c)) < len(sub(a, c)).min(len(sub(b, c))));
    }

    #[test]
    fn a_far_vanishing_point_still_anchors_the_horizon_near_the_grid() {
        // Bottom edge a hair off parallel: the rows' VP is finite but far.
        let c = [[40.0, 0.0], [60.0, 0.0], [100.0, 100.3], [0.0, 100.0]];
        let p = Plane::new(c).unwrap();
        assert!(matches!(p.vp_rows, Vp::Point(_)));
        let hz = p.horizon().unwrap();
        assert!(len(sub(hz.p, centroid(&c))) < 500.0, "{:?}", hz.p);
    }

    #[test]
    fn dragging_a_vanishing_point_re_aims_the_grid() {
        // TRAP's columns meet at (50, -25); move that to (60, -25).
        let moved = move_vp(&TRAP, 1, [60.0, -25.0]).unwrap();
        // The near (bottom) edge stays; the far edge keeps its height.
        assert_eq!((moved[2], moved[3]), (TRAP[2], TRAP[3]));
        assert!(close(moved[0], [48.0, 0.0]), "{:?}", moved[0]);
        assert!(close(moved[1], [68.0, 0.0]), "{:?}", moved[1]);
        let p = Plane::new(moved).unwrap();
        assert!(matches!(p.vp_cols, Vp::Point(v) if close(v, [60.0, -25.0])));
        assert!(matches!(p.vp_rows, Vp::Dir(_)), "the rows stay parallel");
    }

    #[test]
    fn dragging_one_vanishing_point_keeps_the_other() {
        let before = Plane::new(QUAD).unwrap();
        let (Vp::Point(a), Vp::Point(b)) = (before.vp_rows, before.vp_cols) else {
            panic!("two-point quad expected");
        };
        let target = [a[0] + 30.0, a[1] - 20.0];
        let moved = move_vp(&QUAD, 0, target).unwrap();
        let after = Plane::new(moved).unwrap();
        let near = |p: Vp, q: P| matches!(p, Vp::Point(v) if len(sub(v, q)) < 0.5);
        assert!(near(after.vp_rows, target), "{:?}", after.vp_rows);
        assert!(near(after.vp_cols, b), "{:?} vs {b:?}", after.vp_cols);
    }

    #[test]
    fn a_vanishing_point_is_grabbable_and_drags() {
        assert_eq!(grab_at(&TRAP, [51.0, -24.0], 5.0), Some(GridGrab::Vp(1)));
        let moved =
            dragged(&TRAP, GridGrab::Vp(1), [51.0, -24.0], [61.0, -24.0], false).unwrap();
        let p = Plane::new(moved).unwrap();
        assert!(matches!(p.vp_cols, Vp::Point(v) if close(v, [60.0, -25.0])));
    }

    #[test]
    fn grid_lines_include_the_borders() {
        let p = Plane::new(TRAP).unwrap();
        let lines = p.grid_lines(2, 3);
        assert_eq!(lines.len(), 3 + 1 + 2 + 1);
        assert!(close(lines[0].0, TRAP[0]) && close(lines[0].1, TRAP[3]));
        assert!(close(lines[3].0, TRAP[1]) && close(lines[3].1, TRAP[2]));
    }

    #[test]
    fn convexity_rejects_a_bow_tie() {
        assert!(is_convex(&QUAD));
        let bow = [QUAD[0], QUAD[2], QUAD[1], QUAD[3]];
        assert!(!is_convex(&bow));
        let mut dart = QUAD;
        dart[1] = [30.0, 40.0];
        assert!(!is_convex(&dart));
    }

    #[test]
    fn rotation_keeps_the_centroid_and_edges() {
        let r = rotate(&QUAD, 0.7);
        assert!(close(centroid(&r), centroid(&QUAD)));
        for i in 0..4 {
            let a = len(sub(QUAD[(i + 1) % 4], QUAD[i]));
            let b = len(sub(r[(i + 1) % 4], r[i]));
            assert!((a - b).abs() < 1e-3);
        }
    }

    #[test]
    fn grab_priority_is_corner_then_ring_then_inside() {
        let sq = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        assert_eq!(grab_at(&sq, [2.0, 3.0], 5.0), Some(GridGrab::Corner(0)));
        assert_eq!(grab_at(&sq, [101.0, 99.0], 5.0), Some(GridGrab::Corner(2)));
        assert_eq!(grab_at(&sq, [-10.0, -10.0], 5.0), Some(GridGrab::Rotate));
        // Inside near a corner, but past the handle: a move, not a rotate.
        assert_eq!(grab_at(&sq, [12.0, 12.0], 5.0), Some(GridGrab::Move));
        assert_eq!(grab_at(&sq, [50.0, 50.0], 5.0), Some(GridGrab::Move));
        assert_eq!(grab_at(&sq, [200.0, 50.0], 5.0), None);
    }

    #[test]
    fn grabbing_follows_a_rotation() {
        let sq = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        let r = rotate(&sq, 0.5);
        assert_eq!(grab_at(&r, r[1], 5.0), Some(GridGrab::Corner(1)));
        assert_eq!(
            grab_at(&r, sq[1], 5.0),
            None,
            "the old corner spot is empty"
        );
    }

    #[test]
    fn corner_drags_move_one_corner_and_refuse_to_fold() {
        let moved = dragged(&QUAD, GridGrab::Corner(1), QUAD[1], [95.0, 10.0], false).unwrap();
        assert_eq!(moved[0], QUAD[0]);
        assert!(close(moved[1], [95.0, 10.0]));
        // Dragging TR across to the far side of BL folds the quad.
        assert!(dragged(&QUAD, GridGrab::Corner(1), QUAD[1], [-50.0, 150.0], false).is_none());
    }

    #[test]
    fn rotate_drags_snap_to_fifteen_degrees() {
        let sq = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let a = 0.3f32; // ~17°, snaps to 15°
        let r = dragged(
            &sq,
            GridGrab::Rotate,
            [2.0, 0.0],
            [2.0 * a.cos(), 2.0 * a.sin()],
            true,
        )
        .unwrap();
        let want = rotate(&sq, std::f32::consts::PI / 12.0);
        assert!(close(r[0], want[0]));
    }

    #[test]
    fn snapping_picks_the_nearest_family_and_projects_onto_it() {
        let p = Plane::new(TRAP).unwrap();
        let start = [50.0, 50.0];
        assert_eq!(p.snap_dirs(start, false).len(), 2);
        let dirs = p.snap_dirs(start, true);
        assert_eq!(dirs.len(), 3);
        // Heading roughly right: the rows (horizontal) win.
        let d = pick_dir(&dirs, [10.0, 1.0]).unwrap();
        assert!(d[1].abs() < 1e-3);
        let q = project(start, d, [60.0, 53.0]);
        assert!(close(q, [60.0, 50.0]));
        // Heading up toward the VP at (50, -25).
        let d = pick_dir(&dirs, [0.5, -10.0]).unwrap();
        let q = project(start, d, [52.0, 10.0]);
        assert!((q[0] - 50.0).abs() < 1e-3);
    }

    #[test]
    fn frame_relative_corners_follow_the_frame() {
        let mut g = PerspectiveGrid::default();
        let c = [
            [600.0, 300.0],
            [700.0, 300.0],
            [720.0, 400.0],
            [580.0, 400.0],
        ];
        g.set_doc_corners(c, 1280.0, 720.0);
        let back = g.doc_corners(1280.0, 720.0);
        for i in 0..4 {
            assert!(close(back[i], c[i]));
        }
        // Half-size frame: same shape at half scale, still centred.
        let small = g.doc_corners(640.0, 360.0);
        assert!(close(
            centroid(&small),
            [centroid(&c)[0] / 2.0, centroid(&c)[1] / 2.0]
        ));
    }

    #[test]
    fn clipping_an_infinite_line_to_a_rect() {
        let (t0, t1) = clip_line([0.0, 5.0], [1.0, 5.0], [0.0, 0.0], [10.0, 10.0]).unwrap();
        assert!((t0 - 0.0).abs() < 1e-6 && (t1 - 10.0).abs() < 1e-6);
        assert!(clip_line([0.0, 20.0], [1.0, 20.0], [0.0, 0.0], [10.0, 10.0]).is_none());
    }

    #[test]
    fn removing_a_grid_keeps_the_selection_sane() {
        let mut cfg = PerspectiveConfig::default();
        cfg.grids.push(PerspectiveGrid::default());
        cfg.grids.push(PerspectiveGrid::default());
        cfg.active = 2;
        cfg.remove(0);
        assert_eq!(cfg.active, 1);
        cfg.remove(1);
        assert_eq!(cfg.active, 0);
        cfg.remove(0);
        assert!(cfg.grids.is_empty() && cfg.active_grid().is_none());
    }
}
