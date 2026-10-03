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
//! Grids are kept by the app, never in the `.anim` file: one set per project
//! file, remembered by its path. Corners are stored *frame-relative* — offset
//! from the frame centre, in units of the frame height — so a grid keeps its
//! shape, and stays centred, whatever the resolution.
//!
//! A grid can move over the shot two ways, and both stack. It can *follow*
//! the camera or a layer, so its frame-relative coordinates ride that
//! parent's pose (see [`Space`]). And it can carry keys of its own, like the
//! camera: a [`GridPose`] per keyed frame, tweened with the same eases.
//!
//! Besides perspective planes there are flat (square) and isometric grids.
//! They are the same four-corner quad held to a square or a 60° rhombus, and
//! their lines tile the whole canvas.

use serde::{Deserialize, Serialize};

use crate::doc::transform::{Ease, Transform};

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

/// How far in front of the vanishing line a point must be, as a homogeneous
/// `w`, to count as on the plane. The corners sit at `w` around 1; right at
/// the line a point is infinitely far out along the plane.
const FRONT_EPS: f64 = 1e-6;

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

    /// Whether the map is affine — the quad a parallelogram, every family of
    /// lines parallel, nothing vanishing.
    pub fn is_affine(&self) -> bool {
        // `w` runs 1 + g·u + h·v across the square: when it barely changes
        // over the quad, f32 corners can't tell the map from an affine one.
        self.m[6].abs() < 1e-6 && self.m[7].abs() < 1e-6
    }

    /// [`map`](Self::map), but `None` for a `(u, v)` on or past the plane's
    /// vanishing line — behind the viewer, where the map folds the plane
    /// back over the picture upside down. The corners of a convex quad are
    /// always in front.
    pub fn map_front(&self, u: f32, v: f32) -> Option<P> {
        let [x, y, w] = self.apply(u as f64, v as f64, 1.0);
        (w > FRONT_EPS).then(|| [(x / w) as f32, (y / w) as f32])
    }

    /// The inverse: document point → `(u, v)`. `None` for a point on or past
    /// the vanishing line (above a floor's horizon), which no point of the
    /// plane in front of the viewer lands on.
    pub fn unmap(&self, p: P) -> Option<[f32; 2]> {
        let m = &self.m;
        // Rows of the inverse, from the cofactors.
        let inv = [
            m[4] * m[8] - m[5] * m[7],
            m[2] * m[7] - m[1] * m[8],
            m[1] * m[5] - m[2] * m[4],
            m[5] * m[6] - m[3] * m[8],
            m[0] * m[8] - m[2] * m[6],
            m[2] * m[3] - m[0] * m[5],
            m[3] * m[7] - m[4] * m[6],
            m[1] * m[6] - m[0] * m[7],
            m[0] * m[4] - m[1] * m[3],
        ];
        let det = m[0] * inv[0] + m[1] * inv[3] + m[2] * inv[6];
        let (x, y) = (p[0] as f64, p[1] as f64);
        let u = (inv[0] * x + inv[1] * y + inv[2]) / det;
        let v = (inv[3] * x + inv[4] * y + inv[5]) / det;
        let w = (inv[6] * x + inv[7] * y + inv[8]) / det;
        // Mapping `(u, v)` forward gives a `w` of `1 / w` here: the point is
        // in front exactly when this one is positive. The divide by `det`
        // (not just the adjugate) is what keeps that sign honest.
        (w.is_finite() && w > FRONT_EPS).then(|| [(u / w) as f32, (v / w) as f32])
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

/// One family of lines on a grid's plane: those where `alpha·u + beta·v` is
/// a whole number. Rows are `(0, rows)`, columns `(cols, 0)`, and the two
/// diagonals of every cell `(cols, ±rows)` — on an isometric grid the first
/// of those is its verticals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Family {
    pub alpha: f32,
    pub beta: f32,
}

impl Family {
    /// Lines of constant v, running along u.
    pub fn rows(rows: u32) -> Self {
        Self {
            alpha: 0.0,
            beta: rows.clamp(1, MAX_DIVISIONS) as f32,
        }
    }

    /// Lines of constant u, running along v.
    pub fn cols(cols: u32) -> Self {
        Self {
            alpha: cols.clamp(1, MAX_DIVISIONS) as f32,
            beta: 0.0,
        }
    }

    /// `u·cols + v·rows = k`: through every cell's TR and BL corners.
    pub fn diag(rows: u32, cols: u32) -> Self {
        Self {
            alpha: cols.clamp(1, MAX_DIVISIONS) as f32,
            beta: rows.clamp(1, MAX_DIVISIONS) as f32,
        }
    }

    /// `u·cols − v·rows = k`: through every cell's TL and BR corners.
    pub fn anti_diag(rows: u32, cols: u32) -> Self {
        Self {
            alpha: cols.clamp(1, MAX_DIVISIONS) as f32,
            beta: -(rows.clamp(1, MAX_DIVISIONS) as f32),
        }
    }

    /// Along the lines, in (u, v).
    fn dir(self) -> [f64; 2] {
        [self.beta as f64, -self.alpha as f64]
    }

    /// A point on line `k`, in (u, v).
    fn point(self, k: f32) -> [f64; 2] {
        let (a, b, k) = (self.alpha as f64, self.beta as f64, k as f64);
        if a.abs() >= b.abs() {
            [k / a, 0.0]
        } else {
            [0.0, k / b]
        }
    }
}

/// A line of a grid's plane as it shows: whole, or — where the plane
/// recedes — a ray out of its vanishing point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shown {
    /// The whole line, through `p` along unit `d`.
    Line { p: P, d: P },
    /// From vanishing point `from` along unit `d`, out to infinity.
    Ray { from: P, d: P },
}

/// What a press on a grid grabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridGrab {
    Corner(u8),
    /// A finite vanishing point: 0 the rows', 1 the columns'.
    Vp(u8),
    /// One of the grid's extra vanishing points, by index.
    Extra(u8),
    /// The horizon line itself: raises or lowers the eye level.
    Horizon,
    /// A knob on the horizon: tilts it about the point above the grid.
    Tilt,
    Rotate,
    Move,
}

/// What kind of guide a grid is. All three are a four-corner quad; flat and
/// isometric grids hold theirs to a square or a 60° rhombus, and tile it over
/// the whole canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GridKind {
    #[default]
    Perspective,
    /// A square 2D grid: two families of lines at right angles.
    Flat,
    /// Three families at 60° to each other: the rhombus's two edge
    /// directions, and the vertical through its corners.
    Isometric,
}

impl GridKind {
    pub fn label(self) -> &'static str {
        match self {
            GridKind::Perspective => "Perspective",
            GridKind::Flat => "Flat",
            GridKind::Isometric => "Isometric",
        }
    }

    /// Whether the quad keeps its shape: flat and isometric grids only move,
    /// turn and scale.
    pub fn rigid(self) -> bool {
        self != GridKind::Perspective
    }
}

/// What a grid's coordinates are relative to, and so what carries it along
/// over the shot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Follow {
    /// The document: the grid stays on the drawing while the camera moves
    /// over it.
    #[default]
    Document,
    /// The camera's frame: the grid stays put in the shot as the camera pans,
    /// zooms and rolls.
    Camera,
    /// A layer's transform. Named and numbered rather than by the layer's own
    /// id, which only lasts a session — grids outlive it. `uid` caches the
    /// match within a session, so a rename or a reorder keeps the link.
    Layer {
        name: String,
        index: usize,
        #[serde(skip)]
        uid: u64,
    },
}

/// Where a grid's frame-relative coordinates land in document space on one
/// frame: the frame size, and the pose of what the grid follows — the
/// camera's frame rect, or a layer's transform — as a similarity about the
/// frame centre. Identity for a grid on the document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Space {
    pub w: f32,
    pub h: f32,
    pub xf: Transform,
}

impl Space {
    /// The document itself, for a `w`×`h` frame.
    pub fn flat(w: f32, h: f32) -> Self {
        Self {
            w,
            h,
            xf: Transform::default(),
        }
    }

    /// A unit frame. Geometry worked out here is the frame-relative
    /// coordinates shifted by half a unit, which perspective constructions
    /// don't notice — walls are built in it.
    pub fn unit() -> Self {
        Self::flat(1.0, 1.0)
    }

    pub fn to_doc(self, n: P) -> P {
        let f = to_doc(n, self.w, self.h);
        if self.xf.is_identity() {
            return f;
        }
        let (x, y) = self.xf.cell_to_doc(f[0], f[1], self.w, self.h, self.w, self.h);
        [x, y]
    }

    /// The inverse of [`Self::to_doc`]: a document point, frame-relative.
    pub fn to_rel(self, p: P) -> P {
        let f = if self.xf.is_identity() {
            p
        } else {
            let (x, y) = self.xf.doc_to_cell(p[0], p[1], self.w, self.h, self.w, self.h);
            [x, y]
        };
        from_doc(f, self.w, self.h)
    }
}

/// The part of a grid that animates: where it is, and how strongly it shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GridPose {
    /// TL, TR, BR, BL, frame-relative in the grid's [`Space`].
    pub corners: [P; 4],
    /// Each extra vanishing point's spot, in order.
    pub extras: Vec<P>,
    pub opacity: f32,
}

impl Default for GridPose {
    fn default() -> Self {
        let g = PerspectiveGrid::default();
        Self {
            corners: g.corners,
            extras: Vec::new(),
            opacity: g.opacity,
        }
    }
}

/// A grid keyframe. `ease` shapes the segment running from this key to the
/// next, as on camera keys; it is ignored on the last key.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GridKey {
    pub frame: usize,
    pub pose: GridPose,
    pub ease: Ease,
}

/// A wall: a grid standing on an edge of another, its parent, and rebuilt
/// from it whenever the parent changes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WallLink {
    /// The parent's [`PerspectiveGrid::id`].
    pub parent: u32,
    /// The parent's edge it stands on — see [`EDGE_NAMES`].
    pub edge: u8,
    /// How tall: the fraction of the way to the vertical vanishing point, or
    /// — when the verticals stay parallel — a multiple of the edge's length.
    pub height: f32,
}

/// The quad's edges by where they sit on a floor: 0 TL–TR, 1 TR–BR, 2 BL–BR,
/// 3 TL–BL.
pub const EDGE_NAMES: [&str; 4] = ["Back", "Right", "Front", "Left"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PerspectiveGrid {
    /// Unique among a file's grids, and how a wall names its parent. 0 until
    /// [`PerspectiveConfig::ensure_ids`] hands one out.
    pub id: u32,
    pub kind: GridKind,
    /// TL, TR, BR, BL, frame-relative (see the module docs). With keys, this
    /// is what the current frame resolves to — or an edit not keyed yet.
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
    /// Vanishing points beyond the two the rows and columns run to — at most
    /// [`MAX_EXTRA_VPS`].
    pub extra_vps: Vec<ExtraVp>,
    /// Tile rows and columns on past the quad, out to the horizon and the
    /// canvas edge.
    pub infinite: bool,
    /// Draw every Nth line heavier, counting from the quad's first edge; 0
    /// for none.
    pub major_every: u32,
    /// Both diagonals of every cell.
    pub diagonals: bool,
    /// The lines through the middle of the quad.
    pub centre_lines: bool,
    /// Which families strokes may snap along: the rows, the columns, and an
    /// isometric grid's verticals.
    pub snap_rows: bool,
    pub snap_cols: bool,
    pub snap_third: bool,
    pub follow: Follow,
    /// Sorted by frame. None: the grid holds still in its space.
    pub keys: Vec<GridKey>,
    pub wall: Option<WallLink>,
}

pub const MAX_DIVISIONS: u32 = 64;

/// Most vanishing points a grid adds to the two of its own plane, for four in
/// all.
pub const MAX_EXTRA_VPS: usize = 2;

/// A vanishing point a grid carries besides its plane's own: somewhere else
/// strokes can snap toward. On the horizon it is a second pair of directions
/// on the same floor — a box turned another way. Off it, above or below, it
/// is the third point verticals run to.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtraVp {
    /// Frame-relative, like the corners. A point on the horizon shows at this
    /// spot's foot on it, so it rides along as the grid is re-aimed.
    pub pos: P,
    pub on_horizon: bool,
    /// Fan guide lines from it across the grid.
    pub rays: bool,
    /// Strokes may snap toward it.
    pub snap: bool,
}

impl Default for ExtraVp {
    fn default() -> Self {
        Self {
            pos: [0.0, -0.5],
            on_horizon: true,
            rays: true,
            snap: true,
        }
    }
}

impl Default for PerspectiveGrid {
    /// A floor-like trapezoid in the middle of the frame: one-point
    /// perspective, so the grid reads as perspective the moment it appears.
    fn default() -> Self {
        Self {
            id: 0,
            kind: GridKind::Perspective,
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
            extra_vps: Vec::new(),
            infinite: false,
            major_every: 0,
            diagonals: false,
            centre_lines: false,
            snap_rows: true,
            snap_cols: true,
            snap_third: true,
            follow: Follow::Document,
            keys: Vec::new(),
            wall: None,
        }
    }
}

/// Side of a fresh flat or isometric grid's home quad, in frame heights.
const RIGID_SIDE: f32 = 0.4;

impl PerspectiveGrid {
    /// A fresh grid of `kind`: a floor trapezoid, a square, or a rhombus with
    /// its long diagonal level — each centred, four cells to a side for the
    /// tiled kinds.
    pub fn fresh(kind: GridKind) -> Self {
        let base = Self {
            kind,
            ..Self::default()
        };
        let l = RIGID_SIDE;
        match kind {
            GridKind::Perspective => base,
            GridKind::Flat => Self {
                corners: [[-l, -l], [l, -l], [l, l], [-l, l]].map(|p| p.map(|x| x / 2.0)),
                rows: 4,
                cols: 4,
                horizon: false,
                ..base
            },
            GridKind::Isometric => {
                let (s, c) = (std::f32::consts::FRAC_PI_6).sin_cos();
                let a = [c * l, -s * l];
                let b = [c * l, s * l];
                let o = [-c * l, 0.0];
                Self {
                    corners: [o, add(o, a), add(add(o, a), b), add(o, b)],
                    rows: 4,
                    cols: 4,
                    horizon: false,
                    ..base
                }
            }
        }
    }

    /// Corners in document space.
    pub fn doc_corners(&self, s: &Space) -> [P; 4] {
        self.corners.map(|n| s.to_doc(n))
    }

    pub fn set_doc_corners(&mut self, c: [P; 4], s: &Space) {
        self.corners = c.map(|p| s.to_rel(p));
    }

    pub fn is_wall(&self) -> bool {
        self.wall.is_some()
    }

    /// Rotate about the corner centroid. Frame-relative storage is a uniform
    /// scale plus a translation of document space, so rotating it directly is
    /// the same as rotating in the document. The extra vanishing points turn
    /// with it.
    pub fn rotate(&mut self, angle: f32) {
        let o = centroid(&self.corners);
        self.corners = rotate(&self.corners, angle);
        for v in &mut self.extra_vps {
            v.pos = rotate_about(o, v.pos, angle);
        }
    }

    /// Where each extra vanishing point sits in document space: on the
    /// horizon at the foot of its spot, when it keeps to the horizon and the
    /// grid has one, else at the spot itself.
    pub fn extra_doc(&self, s: &Space) -> Vec<P> {
        let horizon = Plane::new(self.doc_corners(s)).and_then(|p| p.horizon());
        self.extra_vps
            .iter()
            .map(|v| {
                let at = s.to_doc(v.pos);
                match horizon {
                    Some(hz) if v.on_horizon => project(hz.p, hz.d, at),
                    _ => at,
                }
            })
            .collect()
    }

    /// Add an extra vanishing point, unless the grid has all it can take. On
    /// the horizon, well out to one side of the grid — the other side for the
    /// second — or straight above it when the horizon is at infinity. Every
    /// key gets it too, where it shows now.
    pub fn add_extra_vp(&mut self, s: &Space) -> bool {
        if self.extra_vps.len() >= MAX_EXTRA_VPS {
            return false;
        }
        let c = self.doc_corners(s);
        let centre = centroid(&c);
        let reach = spread(&c) * 3.0;
        let side = if self.extra_vps.is_empty() { 1.0 } else { -1.0 };
        let horizon = Plane::new(c).and_then(|p| p.horizon());
        let (at, on_horizon) = match horizon {
            Some(hz) => {
                let foot = project(hz.p, hz.d, centre);
                (add(foot, hz.d.map(|x| x * side * reach)), true)
            }
            None => ([centre[0], centre[1] - side * reach], false),
        };
        let pos = s.to_rel(at);
        self.extra_vps.push(ExtraVp {
            pos,
            on_horizon,
            ..ExtraVp::default()
        });
        for k in &mut self.keys {
            k.pose.extras.push(pos);
        }
        true
    }

    /// Remove extra vanishing point `i`, from every key too.
    pub fn remove_extra_vp(&mut self, i: usize) {
        if i >= self.extra_vps.len() {
            return;
        }
        self.extra_vps.remove(i);
        for k in &mut self.keys {
            if i < k.pose.extras.len() {
                k.pose.extras.remove(i);
            }
        }
    }

    /// The families of lines the grid is made of: rows and columns, and an
    /// isometric grid's verticals.
    pub fn families(&self) -> Vec<Family> {
        let mut out = vec![Family::rows(self.rows), Family::cols(self.cols)];
        if self.kind == GridKind::Isometric {
            out.push(Family::diag(self.rows, self.cols));
        }
        out
    }

    /// Everything strokes may snap toward, in document space: the families
    /// switched on, the vertical when asked (perspective grids only — square
    /// to the horizon, or to the rows when it is at infinity), and the extra
    /// vanishing points switched on.
    pub fn snap_targets(&self, s: &Space, vertical: bool) -> Vec<Vp> {
        let c = self.doc_corners(s);
        let Some(plane) = Plane::new(c) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(5);
        if self.snap_rows {
            out.push(plane.vp_rows);
        }
        if self.snap_cols {
            out.push(plane.vp_cols);
        }
        if self.kind == GridKind::Isometric && self.snap_third {
            out.push(plane.family_vp(Family::diag(self.rows, self.cols)));
        }
        if vertical && self.kind == GridKind::Perspective {
            let across = match plane.horizon() {
                Some(l) => Some(l.d),
                None => plane.vp_rows.dir_at(centroid(&c)),
            };
            if let Some(d) = across {
                out.push(Vp::Dir([-d[1], d[0]]));
            }
        }
        for (v, at) in self.extra_vps.iter().zip(self.extra_doc(s)) {
            if v.snap {
                out.push(Vp::Point(at));
            }
        }
        out
    }

    /// Candidate snap directions at `at`: toward each of
    /// [`Self::snap_targets`].
    pub fn snap_dirs(&self, s: &Space, at: P, vertical: bool) -> Vec<P> {
        self.snap_targets(s, vertical)
            .into_iter()
            .filter_map(|v| v.dir_at(at))
            .collect()
    }

    // The pose below is what the grid's X / Y / Scale / Rotation fields show.
    // It is read off the corners rather than stored, so a corner drag and a
    // field edit can never disagree. All of it is frame-relative, like the
    // corners: offsets in frame heights, from the frame centre.

    /// Where the grid sits: its corner centroid, from the frame centre.
    pub fn centre(&self) -> P {
        centroid(&self.corners)
    }

    pub fn translate(&mut self, d: P) {
        self.corners = self.corners.map(|c| add(c, d));
        for v in &mut self.extra_vps {
            v.pos = add(v.pos, d);
        }
    }

    /// Size relative to a fresh grid of its kind, which reads 1.
    pub fn scale(&self) -> f32 {
        spread(&self.corners) / spread(&Self::fresh(self.kind).corners)
    }

    /// Scale by `k` about the centroid. Ignores a `k` that would collapse or
    /// flip the grid.
    pub fn scale_by(&mut self, k: f32) {
        if !(k.is_finite() && k > 1e-4) {
            return;
        }
        let o = self.centre();
        let grow = |c: P| add(o, sub(c, o).map(|x| x * k));
        self.corners = self.corners.map(grow);
        for v in &mut self.extra_vps {
            v.pos = grow(v.pos);
        }
    }

    /// Heading of the grid, in radians, against a fresh grid of its kind
    /// (which reads 0): from the middle of its left edge to the middle of its
    /// right. [`Self::rotate`] turns it by exactly its angle.
    pub fn angle(&self) -> f32 {
        heading(&self.corners) - heading(&Self::fresh(self.kind).corners)
    }

    // --- Keys ---

    /// The live pose.
    pub fn pose(&self) -> GridPose {
        GridPose {
            corners: self.corners,
            extras: self.extra_vps.iter().map(|v| v.pos).collect(),
            opacity: self.opacity,
        }
    }

    /// Make `p` the live pose. Extra vanishing points it has no spot for
    /// stay where they are.
    pub fn set_pose(&mut self, p: &GridPose) {
        self.corners = p.corners;
        for (v, &e) in self.extra_vps.iter_mut().zip(&p.extras) {
            v.pos = e;
        }
        self.opacity = p.opacity;
    }

    pub fn key_at(&self, frame: usize) -> Option<&GridKey> {
        self.keys.iter().find(|k| k.frame == frame)
    }

    pub fn has_key(&self, frame: usize) -> bool {
        self.key_at(frame).is_some()
    }

    /// Key the live pose on `frame`, replacing a key there but keeping its
    /// ease.
    pub fn set_key(&mut self, frame: usize) {
        let pose = self.pose();
        match self.keys.iter_mut().find(|k| k.frame == frame) {
            Some(k) => k.pose = pose,
            None => {
                self.keys.push(GridKey {
                    frame,
                    pose,
                    ease: Ease::default(),
                });
                self.keys.sort_by_key(|k| k.frame);
            }
        }
    }

    pub fn delete_key(&mut self, frame: usize) {
        self.keys.retain(|k| k.frame != frame);
    }

    pub fn set_key_ease(&mut self, frame: usize, ease: Ease) {
        if let Some(k) = self.keys.iter_mut().find(|k| k.frame == frame) {
            k.ease = ease;
        }
    }

    /// The pose on `frame`: tweened from the keys, or the live pose when
    /// there are none.
    pub fn resolve(&self, frame: usize) -> GridPose {
        resolve_keys(&self.keys, frame).unwrap_or_else(|| self.pose())
    }

    /// Take back everything a drag or a key step can change — the pose, the
    /// keys, a wall's height — from `snap`, leaving the look and the
    /// switches as they are now.
    pub fn restore_motion(&mut self, snap: &PerspectiveGrid) {
        self.corners = snap.corners;
        self.extra_vps.clone_from(&snap.extra_vps);
        self.opacity = snap.opacity;
        self.keys.clone_from(&snap.keys);
        self.wall = snap.wall;
    }

    /// The nearest keys strictly before and strictly after `frame`.
    pub fn neighbour_keys(&self, frame: usize) -> (Option<&GridKey>, Option<&GridKey>) {
        let prev = self.keys.iter().rev().find(|k| k.frame < frame);
        let next = self.keys.iter().find(|k| k.frame > frame);
        (prev, next)
    }
}

/// Mean distance of the corners from their centroid.
fn spread(c: &[P; 4]) -> f32 {
    let o = centroid(c);
    c.iter().map(|&p| len(sub(p, o))).sum::<f32>() / 4.0
}

/// From the middle of the quad's left edge to the middle of its right, in
/// radians.
fn heading(c: &[P; 4]) -> f32 {
    let mid = |a: P, b: P| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    let d = sub(mid(c[1], c[2]), mid(c[0], c[3]));
    d[1].atan2(d[0])
}

pub fn to_doc(n: P, w: f32, h: f32) -> P {
    [w * 0.5 + n[0] * h, h * 0.5 + n[1] * h]
}

pub fn from_doc(p: P, w: f32, h: f32) -> P {
    let h = h.max(1.0);
    [(p[0] - w * 0.5) / h, (p[1] - h * 0.5) / h]
}

/// A quad's similarity frame: centroid, spread and heading. Tweening goes
/// through it, so a grid turning between two keys turns rather than
/// shrinking through the chord.
#[derive(Clone, Copy)]
struct PoseFrame {
    o: P,
    s: f32,
    a: f32,
}

impl PoseFrame {
    fn of(c: &[P; 4]) -> Self {
        Self {
            o: centroid(c),
            s: spread(c).max(1e-9),
            a: heading(c),
        }
    }

    fn local(&self, p: P) -> P {
        let d = rotate_about([0.0, 0.0], sub(p, self.o), -self.a);
        [d[0] / self.s, d[1] / self.s]
    }

    fn world(&self, l: P) -> P {
        let d = rotate_about([0.0, 0.0], l, self.a);
        [self.o[0] + d[0] * self.s, self.o[1] + d[1] * self.s]
    }
}

fn lerp_p(a: P, b: P, t: f32) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// The pose `t` of the way from `a` to `b`. Like the camera: the centre moves
/// in a straight line, the size at a steady rate, the heading the short way
/// round. What is left of the corners' shape once that is taken out blends
/// straight across, so a 90° turn keeps its size. A blend that would fold
/// the quad falls back to sliding each corner straight, then to holding the
/// nearer key.
pub fn tween(a: &GridPose, b: &GridPose, t: f32) -> GridPose {
    if t <= 0.0 {
        return a.clone();
    }
    if t >= 1.0 {
        return b.clone();
    }
    let (fa, fb) = (PoseFrame::of(&a.corners), PoseFrame::of(&b.corners));
    let mut turn = (fb.a - fa.a).rem_euclid(std::f32::consts::TAU);
    if turn > std::f32::consts::PI {
        turn -= std::f32::consts::TAU;
    }
    let f = PoseFrame {
        o: lerp_p(fa.o, fb.o, t),
        s: fa.s * (fb.s / fa.s).powf(t),
        a: fa.a + turn * t,
    };
    let mix = |pa: P, pb: P| f.world(lerp_p(fa.local(pa), fb.local(pb), t));
    let mut corners = [0, 1, 2, 3].map(|i| mix(a.corners[i], b.corners[i]));
    if !is_convex(&corners) {
        corners = [0, 1, 2, 3].map(|i| lerp_p(a.corners[i], b.corners[i], t));
        if !is_convex(&corners) {
            corners = if t < 0.5 { a.corners } else { b.corners };
        }
    }
    let mut extras: Vec<P> = a
        .extras
        .iter()
        .zip(&b.extras)
        .map(|(&pa, &pb)| mix(pa, pb))
        .collect();
    extras.extend(a.extras.iter().skip(extras.len()));
    GridPose {
        corners,
        extras,
        opacity: a.opacity + (b.opacity - a.opacity) * t,
    }
}

/// The pose keys give `frame`, or `None` with no keys. Held flat before the
/// first key and after the last, eased within a segment — the same rules as
/// `Camera::resolve`.
pub fn resolve_keys(keys: &[GridKey], frame: usize) -> Option<GridPose> {
    let first = keys.first()?;
    if frame <= first.frame {
        return Some(first.pose.clone());
    }
    let last = keys.last()?;
    if frame >= last.frame {
        return Some(last.pose.clone());
    }
    for w in keys.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if frame >= a.frame && frame <= b.frame {
            let span = (b.frame - a.frame).max(1) as f32;
            let t = a.ease.apply((frame - a.frame) as f32 / span);
            return Some(tween(&a.pose, &b.pose, t));
        }
    }
    Some(last.pose.clone())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PerspectiveConfig {
    /// The open file's grids. Kept by the app per file, not in the file.
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
    /// Once a grid has keys, an edit to its pose keys the current frame.
    pub auto_key: bool,
    /// Faint outlines of the active grid at its keys either side.
    pub ghosts: bool,
    /// While a stroke would snap, faint lines from the cursor along every way
    /// it could go.
    pub cursor_guides: bool,
}

impl Default for PerspectiveConfig {
    fn default() -> Self {
        let mut cfg = Self {
            grids: vec![PerspectiveGrid::default()],
            active: 0,
            show: false,
            snap: false,
            snap_vertical: false,
            auto_key: true,
            ghosts: true,
            cursor_guides: true,
        };
        cfg.ensure_ids();
        cfg
    }
}

impl PerspectiveConfig {
    pub fn active_grid(&self) -> Option<&PerspectiveGrid> {
        self.grids.get(self.active)
    }

    pub fn active_grid_mut(&mut self) -> Option<&mut PerspectiveGrid> {
        self.grids.get_mut(self.active)
    }

    /// Remove grid `i`, keeping `active` on the same grid where it can. Walls
    /// standing on it come loose on the next [`Self::relink`].
    pub fn remove(&mut self, i: usize) {
        if i >= self.grids.len() {
            return;
        }
        self.grids.remove(i);
        if self.active > i || self.active >= self.grids.len() {
            self.active = self.active.saturating_sub(1);
        }
    }

    pub fn index_of(&self, id: u32) -> Option<usize> {
        self.grids.iter().position(|g| g.id == id)
    }

    fn next_id(&self) -> u32 {
        self.grids.iter().map(|g| g.id).max().unwrap_or(0) + 1
    }

    /// Give every grid an id, and make them unique — grids from before ids
    /// all read 0.
    pub fn ensure_ids(&mut self) {
        let mut seen = std::collections::HashSet::new();
        for i in 0..self.grids.len() {
            if self.grids[i].id == 0 || !seen.insert(self.grids[i].id) {
                let id = self.next_id();
                self.grids[i].id = id;
                seen.insert(id);
            }
        }
    }

    /// Add `g` with a fresh id and make it the active grid. Returns its index.
    pub fn push(&mut self, mut g: PerspectiveGrid) -> usize {
        g.id = self.next_id();
        self.grids.push(g);
        self.active = self.grids.len() - 1;
        self.active
    }

    /// Stand a wall on edge `edge` of grid `i`, which must be a perspective
    /// grid. It takes the edge's divisions across, so its lines meet the
    /// floor's, and becomes the active grid.
    pub fn add_wall(&mut self, i: usize, edge: u8) -> Option<usize> {
        let parent = self.grids.get(i)?;
        if parent.kind != GridKind::Perspective {
            return None;
        }
        let edge = edge & 3;
        let height = default_wall_height(parent, edge)?;
        let corners = wall_corners(parent, edge, height)?;
        let wall = PerspectiveGrid {
            corners,
            rows: 4,
            cols: if edge % 2 == 0 { parent.cols } else { parent.rows },
            color: WALL_COLOR,
            opacity: parent.opacity,
            weight: parent.weight,
            horizon: false,
            follow: parent.follow.clone(),
            wall: Some(WallLink {
                parent: parent.id,
                edge,
                height,
            }),
            ..PerspectiveGrid::default()
        };
        Some(self.push(wall))
    }

    /// Rebuild every wall from its parent as the parent is now. A wall whose
    /// parent is gone, or no longer a perspective grid, comes loose and keeps
    /// its last shape. Walls carry no keys of their own and follow what their
    /// parent follows: the parent's motion is theirs.
    pub fn relink(&mut self) {
        // Twice, so a wall on a wall catches up within one call.
        for _ in 0..2 {
            for i in 0..self.grids.len() {
                let Some(link) = self.grids[i].wall else {
                    continue;
                };
                let parent = self
                    .index_of(link.parent)
                    .filter(|&p| p != i && self.grids[p].kind == GridKind::Perspective);
                let Some(p) = parent else {
                    self.grids[i].wall = None;
                    continue;
                };
                let follow = self.grids[p].follow.clone();
                let corners = wall_corners(&self.grids[p], link.edge, link.height);
                let g = &mut self.grids[i];
                if let Some(c) = corners {
                    g.corners = c;
                }
                g.follow = follow;
                g.keys.clear();
            }
        }
    }
}

/// A new wall's colour: warm, against the default grid's blue.
const WALL_COLOR: [u8; 3] = [255, 150, 80];

/// The two ends of edge `e`, in the order the wall on it runs: its BL, then
/// its BR.
fn edge_ends(c: &[P; 4], e: u8) -> (P, P) {
    match e & 3 {
        0 => (c[0], c[1]),
        1 => (c[1], c[2]),
        2 => (c[3], c[2]),
        _ => (c[0], c[3]),
    }
}

/// Where the lines along edge `e` run.
fn edge_vp(plane: &Plane, e: u8) -> Vp {
    if e % 2 == 0 {
        plane.vp_rows
    } else {
        plane.vp_cols
    }
}

/// "Up" off a floor, in [`Space::unit`]: square to its horizon and toward it,
/// or — horizon at infinity — square to the rows and up the screen.
fn floor_up(plane: &Plane, o: P) -> Option<P> {
    match plane.horizon() {
        Some(hz) => {
            let n = [-hz.d[1], hz.d[0]];
            Some(if dot(n, sub(hz.p, o)) >= 0.0 { n } else { [-n[0], -n[1]] })
        }
        None => {
            let d = plane.vp_rows.dir_at(o)?;
            let n = [-d[1], d[0]];
            Some(if n[1] <= 0.0 { n } else { [-n[0], -n[1]] })
        }
    }
}

/// What a wall on `parent` stands up toward, in [`Space::unit`]: the first
/// extra vanishing point off the horizon, with whether it is above the floor
/// (verticals rise to it) or below (they rise away from it, as from a bird's
/// eye); or, with none, straight up — parallel verticals.
fn wall_vertical(parent: &PerspectiveGrid) -> Option<(Vp, bool)> {
    let s = Space::unit();
    let c = parent.doc_corners(&s);
    let plane = Plane::new(c)?;
    let o = centroid(&c);
    let up = floor_up(&plane, o)?;
    let extras = parent.extra_doc(&s);
    match parent.extra_vps.iter().position(|v| !v.on_horizon) {
        Some(i) => Some((Vp::Point(extras[i]), dot(sub(extras[i], o), up) > 0.0)),
        None => Some((Vp::Dir(up), true)),
    }
}

/// The top of the vertical from `a`, `height` up (see [`WallLink::height`]).
fn rise(a: P, v: Vp, toward: bool, height: f32, edge_len: f32) -> P {
    match v {
        Vp::Point(p) => {
            let k = if toward { height } else { -height };
            add(a, sub(p, a).map(|x| x * k))
        }
        Vp::Dir(d) => add(a, d.map(|x| x * height * edge_len)),
    }
}

/// The height a vertical from `a` reaching up to `top` stands for.
fn height_of(a: P, top: P, v: Vp, toward: bool, edge_len: f32) -> f32 {
    match v {
        Vp::Point(p) => {
            let to = sub(p, a);
            let t = dot(sub(top, a), to) / dot(to, to).max(1e-12);
            let t = if toward { t } else { -t };
            t.clamp(0.02, if toward { 0.98 } else { 50.0 })
        }
        Vp::Dir(d) => (dot(sub(top, a), d) / edge_len.max(1e-9)).clamp(0.02, 50.0),
    }
}

/// A wall's corners (TL, TR, BR, BL, frame-relative) standing on edge `edge`
/// of `parent`: verticals up from both ends toward the vertical vanishing
/// point, and a top edge running to the same point as the base, so it is
/// level in the scene.
pub fn wall_corners(parent: &PerspectiveGrid, edge: u8, height: f32) -> Option<[P; 4]> {
    let s = Space::unit();
    let c = parent.doc_corners(&s);
    let plane = Plane::new(c)?;
    let (a, b) = edge_ends(&c, edge);
    let along = edge_vp(&plane, edge);
    let (v, toward) = wall_vertical(parent)?;
    let ta = rise(a, v, toward, height, len(sub(b, a)));
    let tb = intersect(b, v.dir_at(b)?, ta, along.dir_at(ta)?)?;
    let out = [ta, tb, b, a];
    is_convex(&out).then(|| out.map(|p| s.to_rel(p)))
}

/// A new wall's height: about six tenths of its edge's length.
fn default_wall_height(parent: &PerspectiveGrid, edge: u8) -> Option<f32> {
    let s = Space::unit();
    let c = parent.doc_corners(&s);
    let (a, b) = edge_ends(&c, edge);
    let (v, toward) = wall_vertical(parent)?;
    let edge_len = len(sub(b, a));
    Some(match v {
        Vp::Point(p) => {
            let t = 0.6 * edge_len / len(sub(p, a)).max(1e-9);
            if toward {
                t.clamp(0.02, 0.9)
            } else {
                t.clamp(0.02, 50.0)
            }
        }
        Vp::Dir(_) => 0.6,
    })
}

/// The height a wall on `edge` of `parent` takes when its top corner
/// `corner` (0 TL, 1 TR) is dragged to frame-relative point `p`.
pub fn wall_height_at(parent: &PerspectiveGrid, edge: u8, corner: u8, p: P) -> Option<f32> {
    let s = Space::unit();
    let c = parent.doc_corners(&s);
    let plane = Plane::new(c)?;
    let (a, b) = edge_ends(&c, edge);
    let (v, toward) = wall_vertical(parent)?;
    let p = s.to_doc(p);
    let top_a = if corner == 0 {
        project(a, v.dir_at(a)?, p)
    } else {
        // Up the far vertical to the pointer, then back along the top edge
        // to the near one.
        let q = project(b, v.dir_at(b)?, p);
        intersect(a, v.dir_at(a)?, q, edge_vp(&plane, edge).dir_at(q)?)?
    };
    Some(height_of(a, top_a, v, toward, len(sub(b, a))))
}

/// Everything derived from a grid's corners that drawing and snapping need.
#[derive(Clone, Debug)]
pub struct Plane {
    pub h: Homography,
    /// Where lines of constant v (the rows, running along u) converge.
    pub vp_rows: Vp,
    /// Where lines of constant u (the columns, running along v) converge.
    pub vp_cols: Vp,
    /// TL, and the quad's reach from it: what "far away" is measured
    /// against when deciding a vanishing point is at infinity.
    near: P,
    size: f32,
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
            near: corners[0],
            size,
        })
    }

    /// Row and column segments across the quad, borders included. Each is
    /// tagged with the vanishing point its line runs to, and its index from
    /// the quad's first edge — for picking out the major lines.
    pub fn grid_lines(&self, rows: u32, cols: u32) -> Vec<(P, P, Vp, u32)> {
        let (rows, cols) = (rows.clamp(1, MAX_DIVISIONS), cols.clamp(1, MAX_DIVISIONS));
        let mut out = Vec::with_capacity((rows + cols + 2) as usize);
        for i in 0..=cols {
            let u = i as f32 / cols as f32;
            out.push((self.h.map(u, 0.0), self.h.map(u, 1.0), self.vp_cols, i));
        }
        for i in 0..=rows {
            let v = i as f32 / rows as f32;
            out.push((self.h.map(0.0, v), self.h.map(1.0, v), self.vp_rows, i));
        }
        out
    }

    /// Where a family's lines converge.
    pub fn family_vp(&self, f: Family) -> Vp {
        let d = f.dir();
        self.h.vanish(d[0], d[1], self.near, self.size)
    }

    /// Line `k` of family `f` across the whole plane, as much of it as is in
    /// front of the viewer: all of it when the family stays parallel, else
    /// the ray from its vanishing point out through the plane. `None` when
    /// the line is wholly behind the viewer.
    pub fn family_line(&self, f: Family, k: f32) -> Option<Shown> {
        let p0 = f.point(k);
        let d = f.dir();
        let a = self.h.apply(p0[0], p0[1], 1.0);
        let b = self.h.apply(d[0], d[1], 0.0);
        let along = b[0].hypot(b[1]);
        if along < 1e-12 {
            return None;
        }
        let dir = [(b[0] / along) as f32, (b[1] / along) as f32];
        // `w` along the line is `a.w + s·b.w`: in front where positive.
        let s_star = if b[2] != 0.0 { -a[2] / b[2] } else { 0.0 };
        let at = |s: f64| {
            let w = a[2] + s * b[2];
            [((a[0] + s * b[0]) / w) as f32, ((a[1] + s * b[1]) / w) as f32]
        };
        // Parallel to a hair, or near enough that a vanishing point would
        // only lose precision — the same cut `vanish` makes.
        if b[2].abs() * self.size as f64 * 1000.0 <= along {
            if a[2] > FRONT_EPS {
                return Some(Shown::Line { p: at(0.0), d: dir });
            }
            if b[2] == 0.0 {
                return None;
            }
            let s1 = s_star + b[2].signum() * (1.0 + s_star.abs());
            return Some(Shown::Line { p: at(s1), d: dir });
        }
        let vp = [(b[0] / b[2]) as f32, (b[1] / b[2]) as f32];
        // `w` here is |b.w|·(1 + |s*|): in front, and well clear of the line
        // where the plane passes the viewer.
        let s1 = s_star + b[2].signum() * (1.0 + s_star.abs());
        let d = normalize(sub(at(s1), vp))?;
        Some(Shown::Ray { from: vp, d })
    }

    /// The lines of family `f` inside the quad only, as document segments
    /// with their index `k`.
    pub fn quad_family(&self, f: Family) -> Vec<(i32, P, P)> {
        let vals = [0.0, f.alpha, f.beta, f.alpha + f.beta];
        let lo = vals.iter().copied().fold(f32::INFINITY, f32::min).floor() as i32;
        let hi = vals.iter().copied().fold(f32::NEG_INFINITY, f32::max).ceil() as i32;
        let d = f.dir();
        let d = [d[0] as f32, d[1] as f32];
        (lo..=hi)
            .filter_map(|k| {
                let p = f.point(k as f32);
                let p = [p[0] as f32, p[1] as f32];
                let (t0, t1) = clip_line(p, add(p, d), [0.0, 0.0], [1.0, 1.0])?;
                if t1 - t0 < 1e-5 {
                    return None;
                }
                let a = add(p, d.map(|x| x * t0));
                let b = add(p, d.map(|x| x * t1));
                Some((k, self.h.map(a[0], a[1]), self.h.map(b[0], b[1])))
            })
            .collect()
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
    #[cfg(test)]
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
    c.map(|p| rotate_about(o, p, angle))
}

fn rotate_about(o: P, p: P, angle: f32) -> P {
    let (s, co) = angle.sin_cos();
    let d = sub(p, o);
    [o[0] + d[0] * co - d[1] * s, o[1] + d[0] * s + d[1] * co]
}

/// `p` carried along by the move, turn or scale that took corners `from` to
/// `to` — the similarity fixed by the first two corners. For a grid moved or
/// rotated whole, which is when its extra vanishing points come along.
pub fn carry(from: &[P; 4], to: &[P; 4], p: P) -> P {
    let (a0, b0) = (from[0], from[1]);
    let (a1, b1) = (to[0], to[1]);
    let (e0, e1) = (sub(b0, a0), sub(b1, a1));
    let n = dot(e0, e0);
    if n <= 1e-12 {
        return add(p, sub(a1, a0));
    }
    // (e1 / e0) as complex numbers: the turn and scale between the edges.
    let (re, im) = (dot(e1, e0) / n, cross(e0, e1) / n);
    let d = sub(p, a0);
    [a1[0] + re * d[0] - im * d[1], a1[1] + im * d[0] + re * d[1]]
}

/// Where an extra vanishing point dragged to document point `to` lands, and
/// whether it keeps to the horizon. One on the horizon slides along it until
/// pulled more than `detach` off it, and is then free; a free one sticks back
/// on once within `attach`. Without a horizon it goes where it is put.
pub fn place_extra(
    horizon: Option<Line>,
    on_horizon: bool,
    to: P,
    detach: f32,
    attach: f32,
) -> (P, bool) {
    let Some(hz) = horizon else {
        return (to, on_horizon);
    };
    let foot = project(hz.p, hz.d, to);
    let off = len(sub(to, foot));
    let stick = if on_horizon { off <= detach } else { off <= attach };
    if stick {
        (foot, true)
    } else {
        (to, false)
    }
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

/// How far out along the horizon its tilt knobs sit from the point above
/// the grid, in handle radii.
pub const TILT_KNOB: f32 = 12.0;

/// The two tilt knobs on horizon `hz`, for a handle radius of `tol`.
pub fn tilt_knobs(hz: Line, tol: f32) -> [P; 2] {
    let r = tol * TILT_KNOB;
    [add(hz.p, hz.d.map(|x| x * r)), sub(hz.p, hz.d.map(|x| x * r))]
}

/// What a press at `p` grabs on a grid with corners `c` and extra vanishing
/// points at `extra`, with `tol` the handle radius in document units. Corners
/// win, then the vanishing points, then — with `eye_level`, when the grid
/// shows its horizon — the tilt knobs and the horizon line, then a ring just
/// outside the corners rotates, then anywhere inside moves.
pub fn grab_at(c: &[P; 4], extra: &[P], p: P, tol: f32, eye_level: bool) -> Option<GridGrab> {
    let (i, d) = c
        .iter()
        .enumerate()
        .map(|(i, &k)| (i, len(sub(p, k))))
        .fold((0, f32::INFINITY), |a, b| if b.1 < a.1 { b } else { a });
    if d <= tol {
        return Some(GridGrab::Corner(i as u8));
    }
    let plane = Plane::new(*c);
    if let Some(plane) = &plane {
        for which in 0..2 {
            if plane
                .vp_point(which)
                .is_some_and(|v| len(sub(p, v)) <= tol * 1.5)
            {
                return Some(GridGrab::Vp(which));
            }
        }
    }
    if let Some(i) = extra.iter().position(|&v| len(sub(p, v)) <= tol * 1.5) {
        return Some(GridGrab::Extra(i as u8));
    }
    if let Some(hz) = plane.as_ref().filter(|_| eye_level).and_then(|pl| pl.horizon()) {
        if tilt_knobs(hz, tol).iter().any(|&k| len(sub(p, k)) <= tol * 1.5) {
            return Some(GridGrab::Tilt);
        }
        if cross(hz.d, sub(p, hz.p)).abs() <= tol {
            return Some(GridGrab::Horizon);
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
/// rotation to 15° steps, and a tilt to a horizon a whole 15° off level. A
/// `rigid` grid (flat, isometric) keeps its shape: a corner drag turns and
/// scales it about its centre instead. A drag that would break convexity
/// returns `None`.
pub fn dragged(
    start_c: &[P; 4],
    grab: GridGrab,
    start: P,
    now: P,
    snap: bool,
    rigid: bool,
) -> Option<[P; 4]> {
    let delta = sub(now, start);
    let step = std::f32::consts::PI / 12.0;
    match grab {
        GridGrab::Move => Some(start_c.map(|p| add(p, delta))),
        GridGrab::Corner(i) if rigid => {
            let o = centroid(start_c);
            let v0 = sub(start_c[i as usize], o);
            let v1 = sub(add(start_c[i as usize], delta), o);
            let (l0, l1) = (len(v0), len(v1));
            if l0 < 1e-9 || l1 < 1e-9 {
                return None;
            }
            let mut a = cross(v0, v1).atan2(dot(v0, v1));
            if snap {
                a = (a / step).round() * step;
            }
            let k = l1 / l0;
            Some(start_c.map(|p| add(o, rotate_about([0.0, 0.0], sub(p, o), a).map(|x| x * k))))
        }
        GridGrab::Corner(i) => {
            let mut c = *start_c;
            c[i as usize] = add(c[i as usize], delta);
            is_convex(&c).then_some(c)
        }
        GridGrab::Vp(_) | GridGrab::Horizon | GridGrab::Tilt if rigid => None,
        GridGrab::Vp(which) => {
            let v0 = Plane::new(*start_c)?.vp_point(which)?;
            move_vp(start_c, which, add(v0, delta))
        }
        GridGrab::Horizon => {
            let hz = Plane::new(*start_c)?.horizon()?;
            raise_horizon(start_c, cross(hz.d, delta))
        }
        GridGrab::Tilt => {
            let hz = Plane::new(*start_c)?.horizon()?;
            let (a0, a1) = (sub(start, hz.p), sub(now, hz.p));
            let mut a = cross(a0, a1).atan2(dot(a0, a1));
            if snap {
                // Snap where the horizon ends up, not the turn: level has to
                // be reachable from any start.
                let h0 = hz.d[1].atan2(hz.d[0]);
                a = ((h0 + a) / step).round() * step - h0;
            }
            tilt_horizon(start_c, hz.p, a)
        }
        // The point moves, not the grid: see `place_extra`.
        GridGrab::Extra(_) => Some(*start_c),
        GridGrab::Rotate => {
            let o = centroid(start_c);
            let a0 = (start[1] - o[1]).atan2(start[0] - o[0]);
            let a1 = (now[1] - o[1]).atan2(now[0] - o[0]);
            let mut a = a1 - a0;
            if snap {
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

/// The corner nearest the viewer: farthest from the horizon.
fn near_corner(c: &[P; 4], hz: Line) -> usize {
    let off = |k: usize| cross(hz.d, sub(c[k], hz.p)).abs();
    (0..4).max_by(|&i, &j| off(i).total_cmp(&off(j))).unwrap_or(0)
}

/// The corners re-aimed so the rows run to `rows` and the columns to `cols`.
/// The corner nearest the viewer stays put; its two edges swing to the new
/// vanishing points keeping their share of the way there (or their length,
/// when either end is at infinity), and the far corner closes the quad.
/// `None` without a horizon, or if the quad would fold.
pub fn re_aim(c: &[P; 4], rows: Vp, cols: Vp) -> Option<[P; 4]> {
    let plane = Plane::new(*c)?;
    let hz = plane.horizon()?;
    let k = near_corner(c, hz);
    // Neighbours along the rows (0–1, 3–2) and the columns (0–3, 1–2), and
    // the corner across.
    let (r, col, opp) = (k ^ 1, 3 - k, k ^ 2);
    let kp = c[k];
    let slide = |old: Vp, new: Vp, n: P| -> Option<P> {
        let off = sub(n, kp);
        let l = len(off);
        match (old, new) {
            (Vp::Point(v0), Vp::Point(v1)) => {
                let to = sub(v0, kp);
                let f = dot(off, to) / dot(to, to).max(1e-12);
                Some(add(kp, sub(v1, kp).map(|x| x * f)))
            }
            (_, Vp::Point(v1)) => {
                let d = normalize(sub(v1, kp))?;
                let sgn = if dot(d, off) >= 0.0 { 1.0 } else { -1.0 };
                Some(add(kp, d.map(|x| x * l * sgn)))
            }
            (_, Vp::Dir(d)) => {
                let sgn = if dot(d, off) >= 0.0 { 1.0 } else { -1.0 };
                Some(add(kp, d.map(|x| x * l * sgn)))
            }
        }
    };
    let nr = slide(plane.vp_rows, rows, c[r])?;
    let nc = slide(plane.vp_cols, cols, c[col])?;
    let no = intersect(nr, cols.dir_at(nr)?, nc, rows.dir_at(nc)?)?;
    let mut out = *c;
    out[r] = nr;
    out[col] = nc;
    out[opp] = no;
    is_convex(&out).then_some(out)
}

/// The corners re-aimed for an eye level moved `by` square to the horizon —
/// along its left normal, the way `cross(d, ·)` measures.
pub fn raise_horizon(c: &[P; 4], by: f32) -> Option<[P; 4]> {
    let plane = Plane::new(*c)?;
    let hz = plane.horizon()?;
    let shift = [-hz.d[1] * by, hz.d[0] * by];
    let up = |v: Vp| match v {
        Vp::Point(p) => Vp::Point(add(p, shift)),
        dir => dir,
    };
    re_aim(c, up(plane.vp_rows), up(plane.vp_cols))
}

/// The corners re-aimed for a horizon turned `angle` about `pivot`.
pub fn tilt_horizon(c: &[P; 4], pivot: P, angle: f32) -> Option<[P; 4]> {
    let plane = Plane::new(*c)?;
    let turn = |v: Vp| match v {
        Vp::Point(p) => Vp::Point(rotate_about(pivot, p, angle)),
        Vp::Dir(d) => Vp::Dir(rotate_about([0.0, 0.0], d, angle)),
    };
    re_aim(c, turn(plane.vp_rows), turn(plane.vp_cols))
}

/// One tiled line on screen: its index in its family, and its two ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    pub k: i32,
    pub a: P,
    pub b: P,
}

/// Most lines tiled each way from the quad, per family.
const MAX_TILES: i32 = 400;

/// Screen segments of family `f`'s lines across the whole plane, clipped to
/// the screen rect `lo..hi`. `to_screen` must be a similarity — the canvas
/// view — so that lines stay lines.
///
/// On a parallelogram every line is parallel and evenly spaced on screen, so
/// the lines that cross the rect are found directly; packed closer than
/// `min_gap` screen px, only every 2nd, 4th... is kept. On a receding plane
/// the lines are walked out from the quad's first, both ways, and a way stops
/// once they crowd closer than `min_gap` (they are running into the horizon),
/// pass behind the viewer, or reach [`MAX_TILES`].
pub fn tile_family(
    plane: &Plane,
    f: Family,
    to_screen: &dyn Fn(P) -> P,
    lo: P,
    hi: P,
    min_gap: f32,
) -> Vec<Tile> {
    let mut out = Vec::new();
    if plane.h.is_affine() {
        let (Some(Shown::Line { p: p0, d }), Some(Shown::Line { p: p1, .. })) =
            (plane.family_line(f, 0.0), plane.family_line(f, 1.0))
        else {
            return out;
        };
        let (s0, s1) = (to_screen(p0), to_screen(p1));
        let Some(along) = normalize(sub(to_screen(add(p0, d)), s0)) else {
            return out;
        };
        let n = [-along[1], along[0]];
        let step = dot(sub(s1, s0), n);
        if step.abs() < 1e-6 {
            return out;
        }
        let mut every = 1i64;
        while (step * every as f32).abs() < min_gap && every < (1 << 20) {
            every *= 2;
        }
        let offs = [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]].map(|q| dot(sub(q, s0), n) / step);
        let k_lo = offs.iter().copied().fold(f32::INFINITY, f32::min).ceil() as i64;
        let k_hi = offs.iter().copied().fold(f32::NEG_INFINITY, f32::max).floor() as i64;
        let mut k = k_lo.div_euclid(every) * every;
        while k <= k_hi && out.len() < 4 * MAX_TILES as usize {
            if k >= k_lo {
                let at = add(s0, n.map(|x| x * step * k as f32));
                if let Some((t0, t1)) = clip_line(at, add(at, along), lo, hi) {
                    out.push(Tile {
                        k: k as i32,
                        a: add(at, along.map(|x| x * t0)),
                        b: add(at, along.map(|x| x * t1)),
                    });
                }
            }
            k += every;
        }
        return out;
    }
    let clip = |shown: Shown| -> Option<(P, P)> {
        let (from, d, ray) = match shown {
            Shown::Line { p, d } => (p, d, false),
            Shown::Ray { from, d } => (from, d, true),
        };
        let a = to_screen(from);
        let b = to_screen(add(from, d.map(|x| x * plane.size)));
        let (mut t0, t1) = clip_line(a, b, lo, hi)?;
        if ray {
            t0 = t0.max(0.0);
        }
        (t1 - t0 > 1e-4).then(|| (lerp_p(a, b, t0), lerp_p(a, b, t1)))
    };
    for way in [1i32, -1] {
        let mut k = if way > 0 { 0 } else { -1 };
        let mut behind = 0;
        let mut prev: Option<(P, P)> = None;
        for _ in 0..MAX_TILES {
            match plane.family_line(f, k as f32) {
                None => {
                    behind += 1;
                    if behind >= 3 {
                        break;
                    }
                }
                Some(shown) => {
                    behind = 0;
                    if let Some((a, b)) = clip(shown) {
                        let mid = lerp_p(a, b, 0.5);
                        if let Some((pa, pd)) = prev {
                            if cross(pd, sub(mid, pa)).abs() < min_gap {
                                break;
                            }
                        }
                        if let Some(d) = normalize(sub(b, a)) {
                            prev = Some((a, d));
                        }
                        out.push(Tile { k, a, b });
                    }
                }
            }
            k += way;
        }
    }
    out
}

/// Guide rays from vanishing point `v` fanned across a grid with corners `c`,
/// as segments starting at `v`: `n + 1` of them, spread evenly in angle
/// between the two outermost corners as seen from `v`, each running a little
/// past the farthest corner. All the way round, `2n` of them, when `v` sits
/// inside the grid.
pub fn vp_rays(c: &[P; 4], v: P, n: u32) -> Vec<(P, P)> {
    let n = n.clamp(1, MAX_DIVISIONS);
    let reach = c.iter().map(|&k| len(sub(k, v))).fold(0.0f32, f32::max) * 1.15;
    if reach <= 1e-6 {
        return Vec::new();
    }
    // Sitting right on the centre, any heading will do to start the round.
    let base = normalize(sub(centroid(c), v)).unwrap_or([0.0, -1.0]);
    let (a0, a1, count) = if inside(c, v) {
        let step = std::f32::consts::TAU / (2 * n) as f32;
        (0.0, step * (2 * n - 1) as f32, 2 * n)
    } else {
        let side = |k: P| {
            let d = sub(k, v);
            cross(base, d).atan2(dot(base, d))
        };
        let (lo, hi) = c
            .iter()
            .map(|&k| side(k))
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), a| (lo.min(a), hi.max(a)));
        (lo, hi, n + 1)
    };
    (0..count)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / (count - 1).max(1) as f32;
            let (s, co) = a.sin_cos();
            let d = [base[0] * co - base[1] * s, base[0] * s + base[1] * co];
            (v, add(v, d.map(|x| x * reach)))
        })
        .collect()
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
    fn unmap_inverts_map() {
        for c in [QUAD, TRAP] {
            let h = Homography::square_to_quad(&c).unwrap();
            for (u, v) in [(0.0, 0.0), (1.0, 1.0), (0.3, 0.7), (-0.5, 2.0), (1.5, -0.2)] {
                let Some(p) = h.map_front(u, v) else {
                    continue;
                };
                let [u2, v2] = h.unmap(p).unwrap();
                assert!((u2 - u).abs() < 1e-3 && (v2 - v).abs() < 1e-3, "{c:?} {u},{v}");
            }
        }
    }

    #[test]
    fn nothing_past_the_horizon_is_on_the_plane() {
        // TRAP's floor recedes up to the vanishing point at (50, -25).
        let h = Homography::square_to_quad(&TRAP).unwrap();
        assert!(h.unmap([50.0, -10.0]).is_some(), "below the horizon");
        assert!(h.unmap([50.0, -40.0]).is_none(), "above it");
        assert!(h.unmap([500.0, -40.0]).is_none(), "above it, off to the side");
        // Receding (v → −∞) the floor approaches the horizon but never
        // crosses it; the other way, past the viewer, it wraps round behind.
        assert!(h.map_front(0.5, -1e4).is_some());
        assert!(h.map_front(0.5, 2.0).is_none());
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
        assert_eq!(grab_at(&TRAP, &[], [51.0, -24.0], 5.0, false), Some(GridGrab::Vp(1)));
        let moved =
            dragged(&TRAP, GridGrab::Vp(1), [51.0, -24.0], [61.0, -24.0], false, false).unwrap();
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
        assert_eq!(grab_at(&sq, &[], [2.0, 3.0], 5.0, false), Some(GridGrab::Corner(0)));
        assert_eq!(grab_at(&sq, &[], [101.0, 99.0], 5.0, false), Some(GridGrab::Corner(2)));
        assert_eq!(grab_at(&sq, &[], [-10.0, -10.0], 5.0, false), Some(GridGrab::Rotate));
        // Inside near a corner, but past the handle: a move, not a rotate.
        assert_eq!(grab_at(&sq, &[], [12.0, 12.0], 5.0, false), Some(GridGrab::Move));
        assert_eq!(grab_at(&sq, &[], [50.0, 50.0], 5.0, false), Some(GridGrab::Move));
        assert_eq!(grab_at(&sq, &[], [200.0, 50.0], 5.0, false), None);
    }

    #[test]
    fn grabbing_follows_a_rotation() {
        let sq = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        let r = rotate(&sq, 0.5);
        assert_eq!(grab_at(&r, &[], r[1], 5.0, false), Some(GridGrab::Corner(1)));
        assert_eq!(
            grab_at(&r, &[], sq[1], 5.0, false),
            None,
            "the old corner spot is empty"
        );
    }

    #[test]
    fn corner_drags_move_one_corner_and_refuse_to_fold() {
        let moved =
            dragged(&QUAD, GridGrab::Corner(1), QUAD[1], [95.0, 10.0], false, false).unwrap();
        assert_eq!(moved[0], QUAD[0]);
        assert!(close(moved[1], [95.0, 10.0]));
        // Dragging TR across to the far side of BL folds the quad.
        let folded = dragged(&QUAD, GridGrab::Corner(1), QUAD[1], [-50.0, 150.0], false, false);
        assert!(folded.is_none());
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
            true, false)
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
        g.set_doc_corners(c, &Space::flat(1280.0, 720.0));
        let back = g.doc_corners(&Space::flat(1280.0, 720.0));
        for i in 0..4 {
            assert!(close(back[i], c[i]));
        }
        // Half-size frame: same shape at half scale, still centred.
        let small = g.doc_corners(&Space::flat(640.0, 360.0));
        assert!(close(
            centroid(&small),
            [centroid(&c)[0] / 2.0, centroid(&c)[1] / 2.0]
        ));
    }

    #[test]
    fn a_fresh_grid_is_unscaled_and_level() {
        let g = PerspectiveGrid::default();
        assert!((g.scale() - 1.0).abs() < 1e-6);
        assert!(g.angle().abs() < 1e-6);
        assert!(g.centre()[0].abs() < 1e-6, "centred across the frame");
    }

    #[test]
    fn each_pose_edit_moves_only_its_own_value() {
        let mut g = PerspectiveGrid::default();
        let (c0, s0, a0) = (g.centre(), g.scale(), g.angle());

        g.translate([0.1, -0.2]);
        assert!(close(g.centre(), [c0[0] + 0.1, c0[1] - 0.2]));
        assert!((g.scale() - s0).abs() < 1e-5 && (g.angle() - a0).abs() < 1e-5);

        let c1 = g.centre();
        g.scale_by(2.5);
        assert!((g.scale() - 2.5 * s0).abs() < 1e-4);
        assert!(close(g.centre(), c1) && (g.angle() - a0).abs() < 1e-5);

        g.rotate(0.4);
        assert!((g.angle() - (a0 + 0.4)).abs() < 1e-5);
        assert!(close(g.centre(), c1) && (g.scale() - 2.5 * s0).abs() < 1e-4);

        // A collapsing or flipping scale is refused.
        let before = g.corners;
        g.scale_by(0.0);
        g.scale_by(-1.0);
        g.scale_by(f32::NAN);
        assert_eq!(g.corners, before);
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

    // --- Extra vanishing points ---

    const W: f32 = 1000.0;
    const H: f32 = 1000.0;

    fn grid(c: [P; 4]) -> PerspectiveGrid {
        let mut g = PerspectiveGrid::default();
        g.set_doc_corners(c, &Space::flat(W, H));
        g
    }

    /// How far `p` is off `g`'s horizon, in document units.
    fn off_horizon(g: &PerspectiveGrid, p: P) -> f32 {
        let hz = Plane::new(g.doc_corners(&Space::flat(W, H))).unwrap().horizon().unwrap();
        cross(hz.d, sub(p, hz.p)).abs()
    }

    #[test]
    fn an_extra_vp_lands_on_the_horizon_and_rides_it_when_the_grid_is_reaimed() {
        let mut g = grid(QUAD);
        assert!(g.add_extra_vp(&Space::flat(W, H)));
        let v = g.extra_doc(&Space::flat(W, H))[0];
        assert!(off_horizon(&g, v) < 1e-2);
        assert!(len(sub(v, centroid(&QUAD))) > 100.0, "well out to the side");
        // Tilt the plane: the point follows the new horizon.
        let mut c = QUAD;
        c[2] = [85.0, 60.0];
        g.set_doc_corners(c, &Space::flat(W, H));
        assert!(off_horizon(&g, g.extra_doc(&Space::flat(W, H))[0]) < 1e-2);
    }

    #[test]
    fn a_grid_takes_two_extra_vps_one_to_each_side() {
        let mut g = grid(QUAD);
        assert!(g.add_extra_vp(&Space::flat(W, H)) && g.add_extra_vp(&Space::flat(W, H)));
        assert!(!g.add_extra_vp(&Space::flat(W, H)), "four in all");
        let [a, b] = [g.extra_doc(&Space::flat(W, H))[0], g.extra_doc(&Space::flat(W, H))[1]];
        let hz = Plane::new(QUAD).unwrap().horizon().unwrap();
        let c = project(hz.p, hz.d, centroid(&QUAD));
        assert!(dot(sub(a, c), hz.d) * dot(sub(b, c), hz.d) < 0.0, "opposite sides");
        // With the horizon at infinity there is no side: straight up instead.
        let mut flat = grid([[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]]);
        flat.add_extra_vp(&Space::flat(W, H));
        assert!(!flat.extra_vps[0].on_horizon);
        assert!(flat.extra_doc(&Space::flat(W, H))[0][1] < 0.0);
    }

    #[test]
    fn pulling_an_extra_vp_off_the_horizon_frees_it_and_near_it_sticks_again() {
        let hz = Some(Line { p: [0.0, 0.0], d: [1.0, 0.0] });
        // On the horizon: slides along it, however the pointer wanders...
        assert_eq!(place_extra(hz, true, [50.0, 30.0], 40.0, 10.0), ([50.0, 0.0], true));
        // ...until pulled past the detach distance.
        assert_eq!(place_extra(hz, true, [50.0, 45.0], 40.0, 10.0), ([50.0, 45.0], false));
        // Free: stays free until it comes close.
        assert_eq!(place_extra(hz, false, [50.0, 30.0], 40.0, 10.0), ([50.0, 30.0], false));
        assert_eq!(place_extra(hz, false, [50.0, -8.0], 40.0, 10.0), ([50.0, 0.0], true));
        // No horizon: wherever it is put.
        assert_eq!(place_extra(None, true, [5.0, 6.0], 40.0, 10.0), ([5.0, 6.0], true));
    }

    #[test]
    fn moving_turning_and_scaling_a_grid_carries_its_extra_vps() {
        let mut g = grid(QUAD);
        g.extra_vps.push(ExtraVp {
            pos: from_doc([300.0, -200.0], W, H),
            on_horizon: false,
            rays: true,
            snap: true,
        });
        let before = g.extra_doc(&Space::flat(W, H))[0];
        let o = to_doc(g.centre(), W, H);
        g.translate([0.01, 0.02]);
        assert!(close(g.extra_doc(&Space::flat(W, H))[0], add(before, [10.0, 20.0])));
        let o = add(o, [10.0, 20.0]);
        let at = g.extra_doc(&Space::flat(W, H))[0];
        g.rotate(0.5);
        assert!(close(g.extra_doc(&Space::flat(W, H))[0], rotate_about(o, at, 0.5)));
        let at = g.extra_doc(&Space::flat(W, H))[0];
        g.scale_by(2.0);
        assert!(close(g.extra_doc(&Space::flat(W, H))[0], add(o, sub(at, o).map(|x| x * 2.0))));
    }

    #[test]
    fn carry_follows_a_whole_grid_move_or_turn() {
        let p = [300.0, -40.0];
        let moved = QUAD.map(|c| add(c, [7.0, -3.0]));
        assert!(close(carry(&QUAD, &moved, p), add(p, [7.0, -3.0])));
        let turned = rotate(&QUAD, 0.3);
        assert!(close(carry(&QUAD, &turned, p), rotate_about(centroid(&QUAD), p, 0.3)));
    }

    #[test]
    fn extra_vps_are_grabbed_after_the_planes_own() {
        let c = QUAD;
        let extra = [[400.0, -300.0]];
        assert_eq!(grab_at(&c, &extra, [401.0, -299.0], 4.0, false), Some(GridGrab::Extra(0)));
        // A corner still wins where both are in reach.
        assert_eq!(grab_at(&c, &[[11.0, 5.0]], [10.0, 5.0], 4.0, false), Some(GridGrab::Corner(0)));
        // Dragging one leaves the corners alone.
        let same = dragged(&c, GridGrab::Extra(0), [0.0, 0.0], [50.0, 50.0], false, false);
        assert_eq!(same, Some(c));
    }

    #[test]
    fn strokes_can_snap_toward_an_extra_vp() {
        let mut g = grid(QUAD);
        let plain = g.snap_dirs(&Space::flat(W, H), [50.0, 50.0], false).len();
        g.extra_vps.push(ExtraVp {
            pos: from_doc([50.0, -500.0], W, H),
            on_horizon: false,
            rays: false,
            snap: true,
        });
        let dirs = g.snap_dirs(&Space::flat(W, H), [50.0, 50.0], false);
        assert_eq!(dirs.len(), plain + 1);
        assert!(close(*dirs.last().unwrap(), [0.0, -1.0]));
    }

    #[test]
    fn rays_fan_between_the_outermost_corners() {
        let c = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        let v = [50.0, -100.0];
        let rays = vp_rays(&c, v, 4);
        assert_eq!(rays.len(), 5);
        assert!(rays.iter().all(|&(a, _)| a == v));
        // The outer two run through the corners widest apart as seen from
        // the point — from above a square, its near two.
        let through = |(a, b): (P, P), k: P| {
            let (d, e) = (sub(b, a), sub(k, a));
            cross(d, e).abs() / (len(d) * len(e)) < 1e-3
        };
        assert!(through(rays[0], c[0]) || through(rays[0], c[1]));
        assert!(through(rays[4], c[0]) || through(rays[4], c[1]));
        assert!(!through(rays[0], c[1]) || !through(rays[4], c[1]), "not both the same");
        // Past the farthest corner.
        let far = len(sub(c[2], v));
        assert!(rays.iter().all(|&(a, b)| len(sub(b, a)) > far));
        // Inside the grid: all the way round.
        assert_eq!(vp_rays(&c, [50.0, 50.0], 4).len(), 8);
    }

    // --- Keys and tweening ---

    fn pose(c: [P; 4]) -> GridPose {
        GridPose {
            corners: c,
            extras: Vec::new(),
            opacity: 1.0,
        }
    }

    #[test]
    fn a_quarter_turn_tweens_at_full_size() {
        let sq = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let turned = rotate(&sq, std::f32::consts::FRAC_PI_2).map(|p| add(p, [10.0, 0.0]));
        let mid = tween(&pose(sq), &pose(turned), 0.5);
        // Straight corner blending would cut the chord and shrink it to
        // ~0.71; through the pose it stays the same size.
        assert!((spread(&mid.corners) - spread(&sq)).abs() < 1e-4);
        assert!(close(centroid(&mid.corners), [5.0, 0.0]));
        assert!((heading(&mid.corners) - std::f32::consts::FRAC_PI_4).abs() < 1e-4);
    }

    #[test]
    fn a_tween_turns_the_short_way_round() {
        let sq = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let a = rotate(&sq, 3.0);
        let b = rotate(&sq, -3.0);
        let mid = tween(&pose(a), &pose(b), 0.5);
        // Through π, not through 0.
        assert!((heading(&mid.corners).abs() - std::f32::consts::PI).abs() < 1e-3);
    }

    #[test]
    fn scale_and_opacity_tween_like_the_camera() {
        let sq = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let big = sq.map(|p| p.map(|x| x * 4.0));
        let mut b = pose(big);
        b.opacity = 0.0;
        let mid = tween(&pose(sq), &b, 0.5);
        // Geometric: halfway from 1x to 4x is 2x.
        assert!((spread(&mid.corners) / spread(&sq) - 2.0).abs() < 1e-3);
        assert!((mid.opacity - 0.5).abs() < 1e-6);
    }

    #[test]
    fn keys_hold_outside_their_range_and_ease_within() {
        let mut g = grid(QUAD);
        g.set_key(10);
        g.translate([0.2, 0.0]);
        g.set_key(20);
        g.set_key_ease(10, Ease::Both);
        let x = |f: usize| centroid(&g.resolve(f).corners)[0];
        let (x0, x1) = (x(10), x(20));
        assert!((x1 - x0 - 0.2).abs() < 1e-5);
        assert_eq!(x(0), x0, "held before the first key");
        assert_eq!(x(99), x1, "held after the last");
        assert!((x(15) - (x0 + 0.1)).abs() < 1e-5, "smoothstep's midpoint is the midpoint");
        assert!(x(12) - x0 < 0.2 * 0.2, "but it starts slow");
        // Replacing a key keeps its ease.
        g.set_key(10);
        assert_eq!(g.key_at(10).unwrap().ease, Ease::Both);
        g.delete_key(10);
        assert!(!g.has_key(10) && g.has_key(20));
    }

    #[test]
    fn keys_carry_the_extra_vanishing_points() {
        let s = Space::flat(W, H);
        let mut g = grid(QUAD);
        g.set_key(0);
        assert!(g.add_extra_vp(&s));
        assert_eq!(g.keys[0].pose.extras.len(), 1, "a new point joins every key");
        g.extra_vps[0].pos = [0.4, -0.4];
        g.set_key(5);
        assert!(close(g.resolve(5).extras[0], [0.4, -0.4]));
        g.remove_extra_vp(0);
        assert!(g.keys.iter().all(|k| k.pose.extras.is_empty()));
    }

    #[test]
    fn a_space_maps_there_and_back() {
        let s = Space {
            w: 1280.0,
            h: 720.0,
            xf: Transform {
                tx: 120.0,
                ty: -40.0,
                scale: 0.5,
                rot: 0.7,
            },
        };
        for n in [[0.0, 0.0], [0.3, -0.2], [-1.0, 2.0]] {
            assert!(close(s.to_rel(s.to_doc(n)), n));
        }
        // The frame centre sits at the parent's offset.
        assert!(close(s.to_doc([0.0, 0.0]), [640.0 + 120.0, 360.0 - 40.0]));
        // Identity is the plain frame-relative mapping.
        assert!(close(Space::flat(W, H).to_doc([0.1, 0.2]), to_doc([0.1, 0.2], W, H)));
    }

    // --- The eye level ---

    /// Two-point floor, its horizon well above it.
    const FLOOR2: [P; 4] = [[400.0, 300.0], [600.0, 320.0], [560.0, 420.0], [330.0, 390.0]];

    #[test]
    fn raising_the_horizon_moves_both_vanishing_points_and_keeps_the_near_corner() {
        let before = Plane::new(FLOOR2).unwrap();
        let hz0 = before.horizon().unwrap();
        let k = near_corner(&FLOOR2, hz0);
        let up = -25.0; // toward the horizon's left normal's opposite
        let c = raise_horizon(&FLOOR2, up).unwrap();
        assert_eq!(c[k], FLOOR2[k], "the corner nearest the viewer stays put");
        let after = Plane::new(c).unwrap();
        let n = [-hz0.d[1], hz0.d[0]];
        for (a, b) in [(before.vp_rows, after.vp_rows), (before.vp_cols, after.vp_cols)] {
            let (Vp::Point(a), Vp::Point(b)) = (a, b) else {
                panic!("two-point floor expected");
            };
            let d = sub(b, a);
            assert!((dot(d, n) - up).abs() < 1.0, "{d:?}");
            assert!(dot(d, hz0.d).abs() < 1.0, "straight up, not along");
        }
    }

    #[test]
    fn tilting_the_horizon_turns_it_about_its_pivot() {
        let hz0 = Plane::new(FLOOR2).unwrap().horizon().unwrap();
        let c = tilt_horizon(&FLOOR2, hz0.p, 0.1).unwrap();
        let hz1 = Plane::new(c).unwrap().horizon().unwrap();
        let turn = cross(hz0.d, hz1.d).atan2(dot(hz0.d, hz1.d));
        assert!((turn.abs() - 0.1).abs() < 1e-3, "{turn}");
        // A one-point floor tilts too: its parallel rows turn with it.
        let hz = Plane::new(TRAP).unwrap().horizon().unwrap();
        let c = tilt_horizon(&TRAP, hz.p, 0.2).unwrap();
        let p = Plane::new(c).unwrap();
        let Vp::Dir(d) = p.vp_rows else {
            panic!("rows stay parallel");
        };
        assert!(((d[1] / d[0]).atan().abs() - 0.2).abs() < 1e-3);
    }

    #[test]
    fn the_horizon_and_its_knobs_are_grabbable_only_with_the_eye_level_on() {
        let hz = Plane::new(TRAP).unwrap().horizon().unwrap();
        let tol = 2.0;
        // Well off to the side of the vanishing point, on the line.
        let on = add(hz.p, hz.d.map(|x| x * 200.0));
        assert_eq!(grab_at(&TRAP, &[], on, tol, true), Some(GridGrab::Horizon));
        assert_eq!(grab_at(&TRAP, &[], on, tol, false), None);
        let knob = tilt_knobs(hz, tol)[0];
        assert_eq!(grab_at(&TRAP, &[], knob, tol, true), Some(GridGrab::Tilt));
        // Dragging it straight off the line raises the horizon.
        let n = [-hz.d[1], hz.d[0]];
        let c = dragged(&TRAP, GridGrab::Horizon, on, add(on, n.map(|x| x * 10.0)), false, false)
            .unwrap();
        let moved = Plane::new(c).unwrap().horizon().unwrap();
        assert!((cross(hz.d, sub(moved.p, hz.p)) - 10.0).abs() < 0.1);
    }

    #[test]
    fn a_tilt_snaps_to_level() {
        // A grid whose horizon is a few degrees off level.
        let c = rotate(&TRAP, 0.05);
        let hz = Plane::new(c).unwrap().horizon().unwrap();
        let from = tilt_knobs(hz, 2.0)[0];
        let to = rotate_about(hz.p, from, -0.03);
        let out = dragged(&c, GridGrab::Tilt, from, to, true, false).unwrap();
        let d = Plane::new(out).unwrap().horizon().unwrap().d;
        assert!(d[1].abs() < 1e-3, "level, not 0.02 off: {d:?}");
    }

    // --- Flat and isometric grids ---

    #[test]
    fn a_rigid_corner_drag_turns_and_scales_but_keeps_the_square() {
        let sq = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        let c = dragged(&sq, GridGrab::Corner(2), sq[2], [150.0, 120.0], false, true).unwrap();
        assert!(close(c[2], [150.0, 120.0]), "the grabbed corner follows");
        let side = |i: usize| len(sub(c[(i + 1) % 4], c[i]));
        for i in 1..4 {
            assert!((side(i) - side(0)).abs() < 1e-2);
        }
        assert!(dot(sub(c[1], c[0]), sub(c[3], c[0])).abs() < 1e-1, "still square");
        // And a rigid grid has no vanishing points or horizon to drag.
        assert!(dragged(&sq, GridGrab::Horizon, [0.0, 0.0], [0.0, 5.0], false, true).is_none());
    }

    #[test]
    fn a_fresh_flat_or_isometric_grid_is_level_unscaled_and_centred() {
        for kind in [GridKind::Flat, GridKind::Isometric] {
            let g = PerspectiveGrid::fresh(kind);
            assert!(g.angle().abs() < 1e-6, "{kind:?}");
            assert!((g.scale() - 1.0).abs() < 1e-6, "{kind:?}");
            assert!(close(g.centre(), [0.0, 0.0]), "{kind:?}");
            assert!(is_convex(&g.corners));
        }
    }

    #[test]
    fn an_isometric_grid_snaps_three_ways_and_the_third_is_vertical() {
        let s = Space::flat(W, H);
        let mut g = PerspectiveGrid::fresh(GridKind::Isometric);
        let dirs = g.snap_dirs(&s, [500.0, 500.0], false);
        assert_eq!(dirs.len(), 3);
        assert!(dirs.iter().any(|d| d[0].abs() < 1e-4), "{dirs:?}");
        // The two edge families at ±30°.
        let pi_6 = std::f32::consts::FRAC_PI_6;
        let thirty = |d: &P| ((d[1] / d[0]).atan().abs() - pi_6).abs() < 1e-3;
        assert!(dirs.iter().any(thirty));
        g.snap_third = false;
        g.snap_rows = false;
        assert_eq!(g.snap_dirs(&s, [500.0, 500.0], false).len(), 1);
        // The vertical option only ever applies to perspective grids.
        assert_eq!(g.snap_dirs(&s, [500.0, 500.0], true).len(), 1);
    }

    #[test]
    fn a_flat_grid_tiles_the_whole_rect_and_thins_when_dense() {
        let p = Plane::new([[0.0, 0.0], [40.0, 0.0], [40.0, 40.0], [0.0, 40.0]]).unwrap();
        let id = |q: P| q;
        let (lo, hi) = ([-100.0, -100.0], [300.0, 300.0]);
        // Columns 10 px apart over a 400 px rect: 41 of them.
        let cols = tile_family(&p, Family::cols(4), &id, lo, hi, 4.0);
        assert_eq!(cols.len(), 41);
        assert!(cols.iter().all(|t| (t.a[0] - t.b[0]).abs() < 1e-3), "vertical");
        // Needing 15 px between them keeps every other one.
        let sparse = tile_family(&p, Family::cols(4), &id, lo, hi, 15.0);
        assert!(sparse.iter().all(|t| t.k % 2 == 0));
        assert!(sparse.len() < 25);
    }

    #[test]
    fn an_infinite_floor_runs_to_the_horizon_and_stops_there() {
        let p = Plane::new(TRAP).unwrap();
        let id = |q: P| q;
        let (lo, hi) = ([-500.0, -200.0], [600.0, 400.0]);
        // The rows recede up toward the horizon at y = -25 and crowd there.
        let rows = tile_family(&p, Family::rows(4), &id, lo, hi, 2.0);
        assert!(rows.len() > 5 && rows.len() < 400, "{}", rows.len());
        assert!(rows.iter().all(|t| t.a[1] > -25.0 - 1e-2 && t.b[1] > -25.0 - 1e-2));
        // Columns are rays out of the vanishing point, never above it.
        let cols = tile_family(&p, Family::cols(4), &id, lo, hi, 2.0);
        assert!(cols.len() > 5, "{}", cols.len());
        for t in &cols {
            assert!(t.a[1].min(t.b[1]) >= -25.0 - 1e-2, "{t:?}");
        }
        // The quad's own lines are among them.
        assert!((0..=4).all(|k| cols.iter().any(|t| t.k == k)));
    }

    #[test]
    fn a_line_behind_the_viewer_is_not_shown() {
        let p = Plane::new(TRAP).unwrap();
        // Toward the viewer the floor passes under the eye at some v; the
        // row far past it is behind.
        assert!(p.family_line(Family::rows(1), 50.0).is_none());
        assert!(matches!(p.family_line(Family::cols(1), 0.5), Some(Shown::Ray { .. })));
        assert!(matches!(p.family_line(Family::rows(1), 0.5), Some(Shown::Line { .. })));
    }

    #[test]
    fn cell_diagonals_cross_every_cell() {
        let p = Plane::new([[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]]).unwrap();
        // 3 x 2 cells: k = 1..=4 for one diagonal family, with the corners
        // k = 0 and 5 only touching.
        let d = p.quad_family(Family::diag(2, 3));
        assert_eq!(d.len(), 4);
        let a = p.quad_family(Family::anti_diag(2, 3));
        assert_eq!(a.len(), 4);
    }

    // --- Walls ---

    #[test]
    fn a_wall_stands_on_its_edge_and_rises_to_the_vertical_point() {
        let mut cfg = PerspectiveConfig::default();
        cfg.grids[0].set_doc_corners(FLOOR2, &Space::unit());
        // A third point well above the floor.
        cfg.grids[0].extra_vps.push(ExtraVp {
            pos: Space::unit().to_rel([480.0, -2000.0]),
            on_horizon: false,
            ..ExtraVp::default()
        });
        let w = cfg.add_wall(0, 0).expect("a wall on the back edge");
        let s = Space::unit();
        let floor = cfg.grids[0].doc_corners(&s);
        let wall = cfg.grids[w].doc_corners(&s);
        assert!(close(wall[3], floor[0]) && close(wall[2], floor[1]), "base on the edge");
        let v = [480.0, -2000.0];
        let through = |a: P, b: P| cross(sub(b, a), sub(v, a)).abs() / len(sub(b, a)) < 0.5;
        assert!(through(wall[3], wall[0]) && through(wall[2], wall[1]), "verticals to the point");
        // Its top runs to the same vanishing point as its base.
        let Vp::Point(e) = Plane::new(floor).unwrap().vp_rows else {
            panic!("two-point floor expected");
        };
        let top = sub(wall[1], wall[0]);
        assert!(cross(top, sub(e, wall[0])).abs() / len(top) < 3.0);
        // It has as many columns as the edge has divisions, and no horizon.
        assert_eq!(cfg.grids[w].cols, cfg.grids[0].cols);
        assert!(!cfg.grids[w].horizon);
    }

    #[test]
    fn a_wall_follows_its_floor_and_comes_loose_without_it() {
        let mut cfg = PerspectiveConfig::default();
        let w = cfg.add_wall(0, 1).expect("a wall on the right edge");
        cfg.grids[0].translate([0.1, 0.05]);
        cfg.relink();
        let s = Space::unit();
        let floor = cfg.grids[0].doc_corners(&s);
        let wall = cfg.grids[w].doc_corners(&s);
        assert!(close(wall[3], floor[1]) && close(wall[2], floor[2]));
        // Walls have no keys of their own.
        cfg.grids[w].set_key(3);
        cfg.relink();
        assert!(cfg.grids[w].keys.is_empty());
        let id = cfg.grids[w].id;
        cfg.remove(0);
        cfg.relink();
        let w = cfg.index_of(id).unwrap();
        assert!(!cfg.grids[w].is_wall());
    }

    #[test]
    fn dragging_a_walls_top_corner_sets_its_height() {
        let mut cfg = PerspectiveConfig::default();
        let w = cfg.add_wall(0, 0).unwrap();
        let link = cfg.grids[w].wall.unwrap();
        let top = cfg.grids[w].corners[0];
        let h = wall_height_at(&cfg.grids[0], link.edge, 0, top).unwrap();
        assert!((h - link.height).abs() < 1e-3, "{h} vs {}", link.height);
        // Higher up, taller.
        let up = [top[0], top[1] - 0.1];
        let taller = wall_height_at(&cfg.grids[0], link.edge, 0, up).unwrap();
        assert!(taller > link.height);
        // Either top corner says the same.
        let tr = cfg.grids[w].corners[1];
        let h1 = wall_height_at(&cfg.grids[0], link.edge, 1, tr).unwrap();
        assert!((h1 - link.height).abs() < 1e-3);
    }

    #[test]
    fn ids_are_handed_out_and_kept_unique() {
        let mut cfg = PerspectiveConfig::default();
        cfg.grids.push(PerspectiveGrid::default());
        cfg.grids.push(cfg.grids[0].clone());
        cfg.ensure_ids();
        let ids: std::collections::HashSet<u32> = cfg.grids.iter().map(|g| g.id).collect();
        assert_eq!(ids.len(), 3);
        assert!(!ids.contains(&0));
        let i = cfg.push(PerspectiveGrid::fresh(GridKind::Flat));
        assert_eq!(cfg.active, i);
        assert!(!ids.contains(&cfg.grids[i].id));
    }
}
