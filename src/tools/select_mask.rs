//! Persistent selection mask — the Krita half of the lasso.
//!
//! A [`SelectionMask`] is coverage in **document** space: it belongs to the
//! canvas, not to a drawing, so it survives changing frame or layer and every
//! layer sees it through its own transform ([`SelectionMask::to_cell`]). While
//! one exists, every paint operation is clipped to it.
//!
//! Document space has no edges — a layer's cell may reach well past the frame
//! — so a mask is a bounding box plus one byte per pixel inside it, and an
//! `outside` byte that says what everything beyond the box is. `outside` is 0
//! for an ordinary selection and 255 after Select All or Invert, which is what
//! keeps Invert exact and makes Select All cost nothing.
//!
//! Everything here is pure: no egui, no project. The app owns the one live
//! mask and decides when it changes.

use crate::doc::transform::Transform;
use crate::tools::lasso::{self, Mask};

/// Coverage at or above this counts as "inside" for the outline and for grow.
pub const THRESHOLD: u8 = 128;

/// How a newly drawn shape combines with the selection already there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SelOp {
    /// The new shape becomes the selection.
    #[default]
    Replace,
    Add,
    Subtract,
    Intersect,
}

impl SelOp {
    pub fn label(self) -> &'static str {
        match self {
            SelOp::Replace => "Replace",
            SelOp::Add => "Add",
            SelOp::Subtract => "Remove",
            SelOp::Intersect => "Intersect",
        }
    }

    /// Combine existing coverage `a` with new coverage `b`.
    fn apply(self, a: u8, b: u8) -> u8 {
        match self {
            SelOp::Replace => b,
            SelOp::Add => a.max(b),
            SelOp::Subtract => ((a as u32 * (255 - b as u32) + 127) / 255) as u8,
            SelOp::Intersect => a.min(b),
        }
    }
}

/// What a selection drag draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SelShape {
    #[default]
    Freehand,
    Rect,
    Ellipse,
    /// Click corner points; Enter, a double-click or a click on the first
    /// point closes it.
    Polygon,
}

impl SelShape {
    pub fn label(self) -> &'static str {
        match self {
            SelShape::Freehand => "Freehand",
            SelShape::Rect => "Rectangle",
            SelShape::Ellipse => "Ellipse",
            SelShape::Polygon => "Polygon",
        }
    }
}

/// Selection coverage in document space. See the module doc.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionMask {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Row-major, `w * h` bytes.
    pub cov: Vec<u8>,
    /// Coverage of every pixel outside the box: 0 or 255.
    pub outside: u8,
}

/// A [`SelectionMask`] run-length encoded, for the undo history. Selections
/// are mostly long runs of 0 and 255, so this is a small fraction of the raw
/// bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct PackedMask {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    outside: u8,
    runs: Vec<(u8, u32)>,
}

impl PackedMask {
    pub fn unpack(&self) -> SelectionMask {
        let mut cov = Vec::with_capacity((self.w * self.h) as usize);
        for &(v, n) in &self.runs {
            cov.extend(std::iter::repeat(v).take(n as usize));
        }
        SelectionMask {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
            cov,
            outside: self.outside,
        }
    }
}

/// The integer offset from a cell to document space when `xf` is a plain
/// whole-pixel translation — cell pixel `(u, v)` is then document pixel
/// `(u + ox, v + oy)`, and moving a mask between the two is a crop rather than
/// a resample. That is the common case: an untransformed layer whose cell is
/// the project size (offset 0), or larger by an even amount.
pub fn integer_offset(xf: &Transform, cw: u32, ch: u32, pw: f32, ph: f32) -> Option<(i32, i32)> {
    if (xf.scale - 1.0).abs() > 1e-6 || xf.rot != 0.0 {
        return None;
    }
    let ox = pw * 0.5 + xf.tx - cw as f32 * 0.5;
    let oy = ph * 0.5 + xf.ty - ch as f32 * 0.5;
    if (ox - ox.round()).abs() > 1e-3 || (oy - oy.round()).abs() > 1e-3 {
        return None;
    }
    Some((ox.round() as i32, oy.round() as i32))
}

/// Four corners of an axis-aligned rect with whole-pixel edges, so a
/// rectangle selection comes out hard-edged.
pub fn rect_path(a: (f32, f32), b: (f32, f32)) -> Vec<(f32, f32)> {
    let (x0, x1) = (a.0.min(b.0).round(), a.0.max(b.0).round());
    let (y0, y1) = (a.1.min(b.1).round(), a.1.max(b.1).round());
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

/// The ellipse inscribed in the box `a`–`b`, as a polygon fine enough that no
/// chord strays more than 0.2 px from the curve — the same sagitta rule the
/// Shape tool's ellipse uses.
pub fn ellipse_path(a: (f32, f32), b: (f32, f32)) -> Vec<(f32, f32)> {
    let (cx, cy) = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
    let (rx, ry) = ((b.0 - a.0).abs() * 0.5, (b.1 - a.1).abs() * 0.5);
    if rx < 0.5 || ry < 0.5 {
        return Vec::new();
    }
    let (mx, mn) = (rx.max(ry), rx.min(ry));
    let r_curv = mn * mn / mx;
    let chord = (8.0 * r_curv * 0.2).sqrt().max(1.0);
    let circ =
        std::f32::consts::PI * (3.0 * (rx + ry) - ((3.0 * rx + ry) * (rx + 3.0 * ry)).sqrt());
    let n = ((circ / chord).ceil() as usize).clamp(32, 4096);
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            (cx + rx * t.cos(), cy + ry * t.sin())
        })
        .collect()
}

impl SelectionMask {
    /// Everything selected. Costs nothing: the box is empty and `outside`
    /// covers the rest.
    pub fn all() -> Self {
        Self {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            cov: Vec::new(),
            outside: 255,
        }
    }

    /// True for a normalised Select All — no restriction at all.
    pub fn is_all(&self) -> bool {
        self.outside == 255 && (self.w == 0 || self.h == 0)
    }

    /// Coverage at document pixel `(x, y)`.
    pub fn at(&self, x: i32, y: i32) -> u8 {
        if x < self.x || y < self.y {
            return self.outside;
        }
        let (dx, dy) = ((x - self.x) as u32, (y - self.y) as u32);
        if dx >= self.w || dy >= self.h {
            return self.outside;
        }
        self.cov[(dy * self.w + dx) as usize]
    }

    /// The box as `(x0, y0, x1, y1)`, max exclusive, or `None` when empty.
    pub fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        (self.w > 0 && self.h > 0).then(|| {
            (
                self.x,
                self.y,
                self.x + self.w as i32,
                self.y + self.h as i32,
            )
        })
    }

    /// Trim the box to the pixels that differ from `outside`. `None` when
    /// nothing is selected at all.
    pub fn normalize(self) -> Option<Self> {
        let empty = |outside: u8| (outside == 255).then(Self::all);
        if self.w == 0 || self.h == 0 {
            return empty(self.outside);
        }
        let (w, h) = (self.w as usize, self.h as usize);
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
        for y in 0..h {
            let row = &self.cov[y * w..(y + 1) * w];
            let Some(first) = row.iter().position(|&c| c != self.outside) else {
                continue;
            };
            let last = row.iter().rposition(|&c| c != self.outside).unwrap_or(first);
            x0 = x0.min(first);
            x1 = x1.max(last + 1);
            y0 = y0.min(y);
            y1 = y + 1;
        }
        if x1 <= x0 || y1 <= y0 {
            return empty(self.outside);
        }
        if (x0, y0, x1, y1) == (0, 0, w, h) {
            return Some(self);
        }
        let nw = x1 - x0;
        let mut cov = Vec::with_capacity(nw * (y1 - y0));
        for y in y0..y1 {
            cov.extend_from_slice(&self.cov[y * w + x0..y * w + x1]);
        }
        Some(Self {
            x: self.x + x0 as i32,
            y: self.y + y0 as i32,
            w: nw as u32,
            h: (y1 - y0) as u32,
            cov,
            outside: self.outside,
        })
    }

    /// Everything that was not selected. `None` if that is nothing.
    pub fn inverted(&self) -> Option<Self> {
        Self {
            cov: self.cov.iter().map(|&c| 255 - c).collect(),
            outside: 255 - self.outside,
            ..self.clone()
        }
        .normalize()
    }

    /// The same mask moved by whole document pixels.
    pub fn translated(&self, dx: i32, dy: i32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..self.clone()
        }
    }

    /// `a` combined with `b` under `op`, over the union of their boxes. `a` is
    /// `None` when nothing was selected before.
    pub fn combine(a: Option<&Self>, b: &Self, op: SelOp) -> Option<Self> {
        if op == SelOp::Replace {
            return b.clone().normalize();
        }
        let none = Self {
            outside: 0,
            ..Self::all()
        };
        let a = a.unwrap_or(&none);
        let outside = op.apply(a.outside, b.outside);
        let boxes: Vec<_> = [a.bounds(), b.bounds()].into_iter().flatten().collect();
        if boxes.is_empty() {
            return Self { outside, ..none }.normalize();
        }
        let x0 = boxes.iter().map(|b| b.0).min().unwrap_or(0);
        let y0 = boxes.iter().map(|b| b.1).min().unwrap_or(0);
        let x1 = boxes.iter().map(|b| b.2).max().unwrap_or(0);
        let y1 = boxes.iter().map(|b| b.3).max().unwrap_or(0);
        let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
        let mut cov = vec![0u8; (w * h) as usize];
        for yy in 0..h as i32 {
            for xx in 0..w as i32 {
                let (x, y) = (x0 + xx, y0 + yy);
                cov[(yy as u32 * w + xx as u32) as usize] = op.apply(a.at(x, y), b.at(x, y));
            }
        }
        Self {
            x: x0,
            y: y0,
            w,
            h,
            cov,
            outside,
        }
        .normalize()
    }

    /// Rasterise a closed document-space path, clipped to `clip`
    /// (`x0, y0, x1, y1`, max exclusive). `clip` is what stops a lasso drawn
    /// while zoomed far out from asking for a gigapixel buffer.
    pub fn from_path(pts: &[(f32, f32)], clip: (i32, i32, i32, i32)) -> Option<Self> {
        let (x, y, w, h, cov) = lasso::coverage_rect(pts, clip)?;
        Self {
            x,
            y,
            w,
            h,
            cov,
            outside: 0,
        }
        .normalize()
    }

    /// Bilinear coverage at a document point, `0..=255`. Pixel centres sit at
    /// `+0.5`, matching every other resampler in the app.
    fn sample(&self, x: f32, y: f32) -> f32 {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (ix, iy) = (x0 as i32, y0 as i32);
        let p00 = self.at(ix, iy) as f32;
        let p10 = self.at(ix + 1, iy) as f32;
        let p01 = self.at(ix, iy + 1) as f32;
        let p11 = self.at(ix + 1, iy + 1) as f32;
        let top = p00 + (p10 - p00) * tx;
        let bot = p01 + (p11 - p01) * tx;
        top + (bot - top) * ty
    }

    /// This mask as seen by a cell of size `cw x ch` placed by `xf` on a
    /// `pw x ph` canvas — the clip a stroke on that cell is held to.
    ///
    /// A whole-pixel translation (see [`integer_offset`]) is a byte-exact
    /// crop; anything else maps each cell pixel into document space and
    /// samples, supersampled when the layer is scaled up so a thin selection
    /// does not alias away. An empty result (`w == 0`) means nothing on this
    /// cell may be painted.
    pub fn to_cell(&self, xf: &Transform, cw: u32, ch: u32, pw: f32, ph: f32) -> Mask {
        let empty = Mask {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            cov: Vec::new(),
        };
        if cw == 0 || ch == 0 {
            return empty;
        }
        let full = (0, 0, cw as i32, ch as i32);

        if let Some((ox, oy)) = integer_offset(xf, cw, ch, pw, ph) {
            let (u0, v0, u1, v1) = if self.outside == 255 {
                full
            } else {
                let Some((x0, y0, x1, y1)) = self.bounds() else {
                    return empty;
                };
                (
                    (x0 - ox).max(0),
                    (y0 - oy).max(0),
                    (x1 - ox).min(cw as i32),
                    (y1 - oy).min(ch as i32),
                )
            };
            if u1 <= u0 || v1 <= v0 {
                return empty;
            }
            let (w, h) = ((u1 - u0) as u32, (v1 - v0) as u32);
            let mut cov = vec![0u8; (w * h) as usize];
            for v in 0..h as i32 {
                let row = &mut cov[(v as u32 * w) as usize..((v as u32 + 1) * w) as usize];
                for (u, c) in row.iter_mut().enumerate() {
                    *c = self.at(u0 + u as i32 + ox, v0 + v + oy);
                }
            }
            return Mask {
                x: u0 as u32,
                y: v0 as u32,
                w,
                h,
                cov,
            };
        }

        let (cwf, chf) = (cw as f32, ch as f32);
        let (u0, v0, u1, v1) = if self.outside == 255 {
            full
        } else {
            let Some((x0, y0, x1, y1)) = self.bounds() else {
                return empty;
            };
            let corners = [
                (x0 as f32, y0 as f32),
                (x1 as f32, y0 as f32),
                (x1 as f32, y1 as f32),
                (x0 as f32, y1 as f32),
            ];
            let (mut nu, mut nv, mut xu, mut xv) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for (x, y) in corners {
                let (u, v) = xf.doc_to_cell(x, y, cwf, chf, pw, ph);
                nu = nu.min(u);
                nv = nv.min(v);
                xu = xu.max(u);
                xv = xv.max(v);
            }
            (
                (nu.floor() as i32 - 1).max(0),
                (nv.floor() as i32 - 1).max(0),
                (xu.ceil() as i32 + 1).min(cw as i32),
                (xv.ceil() as i32 + 1).min(ch as i32),
            )
        };
        if u1 <= u0 || v1 <= v0 {
            return empty;
        }
        let (w, h) = ((u1 - u0) as u32, (v1 - v0) as u32);
        let n = xf.scale.abs().ceil().clamp(1.0, 4.0) as i32;
        let step = 1.0 / n as f32;
        let inv = 1.0 / (n * n) as f32;
        let mut cov = vec![0u8; (w * h) as usize];
        for v in 0..h {
            for u in 0..w {
                let mut acc = 0.0;
                for sy in 0..n {
                    for sx in 0..n {
                        let cu = (u0 + u as i32) as f32 + (sx as f32 + 0.5) * step;
                        let cv = (v0 + v as i32) as f32 + (sy as f32 + 0.5) * step;
                        let (x, y) = xf.cell_to_doc(cu, cv, cwf, chf, pw, ph);
                        acc += self.sample(x, y);
                    }
                }
                cov[(v * w + u) as usize] = (acc * inv).round().clamp(0.0, 255.0) as u8;
            }
        }
        Mask {
            x: u0 as u32,
            y: v0 as u32,
            w,
            h,
            cov,
        }
    }

    /// The inverse of [`SelectionMask::to_cell`]: cell-space coverage (box at
    /// `(x, y)`, possibly off the cell) mapped back into document space. How a
    /// scaled or rotated float hands its shape back to the selection when it
    /// lands.
    #[allow(clippy::too_many_arguments)]
    pub fn from_cell(
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        cov: &[u8],
        xf: &Transform,
        cw: u32,
        ch: u32,
        pw: f32,
        ph: f32,
    ) -> Option<Self> {
        if w == 0 || h == 0 {
            return None;
        }
        let src = Self {
            x,
            y,
            w,
            h,
            cov: cov.to_vec(),
            outside: 0,
        };
        if let Some((ox, oy)) = integer_offset(xf, cw, ch, pw, ph) {
            return src.translated(ox, oy).normalize();
        }
        let (cwf, chf) = (cw as f32, ch as f32);
        let corners = [
            (x as f32, y as f32),
            ((x + w as i32) as f32, y as f32),
            ((x + w as i32) as f32, (y + h as i32) as f32),
            (x as f32, (y + h as i32) as f32),
        ];
        let (mut nx, mut ny, mut xx, mut xy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (u, v) in corners {
            let (dx, dy) = xf.cell_to_doc(u, v, cwf, chf, pw, ph);
            nx = nx.min(dx);
            ny = ny.min(dy);
            xx = xx.max(dx);
            xy = xy.max(dy);
        }
        let (x0, y0) = (nx.floor() as i32 - 1, ny.floor() as i32 - 1);
        let (x1, y1) = (xx.ceil() as i32 + 1, xy.ceil() as i32 + 1);
        let (dw, dh) = ((x1 - x0).max(0) as u32, (y1 - y0).max(0) as u32);
        // A runaway scale must not try to allocate gigabytes.
        if dw as u64 * dh as u64 > 64 << 20 {
            return None;
        }
        let n = (1.0 / xf.scale.abs().max(1e-6)).ceil().clamp(1.0, 4.0) as i32;
        let step = 1.0 / n as f32;
        let inv = 1.0 / (n * n) as f32;
        let mut out = vec![0u8; (dw * dh) as usize];
        for py in 0..dh {
            for px in 0..dw {
                let mut acc = 0.0;
                for sy in 0..n {
                    for sx in 0..n {
                        let dx = (x0 + px as i32) as f32 + (sx as f32 + 0.5) * step;
                        let dy = (y0 + py as i32) as f32 + (sy as f32 + 0.5) * step;
                        let (u, v) = xf.doc_to_cell(dx, dy, cwf, chf, pw, ph);
                        acc += src.sample(u, v);
                    }
                }
                out[(py * dw + px) as usize] = (acc * inv).round().clamp(0.0, 255.0) as u8;
            }
        }
        Self {
            x: x0,
            y: y0,
            w: dw,
            h: dh,
            cov: out,
            outside: 0,
        }
        .normalize()
    }

    pub fn pack(&self) -> PackedMask {
        let mut runs: Vec<(u8, u32)> = Vec::new();
        for &c in &self.cov {
            match runs.last_mut() {
                Some((v, n)) if *v == c => *n += 1,
                _ => runs.push((c, 1)),
            }
        }
        PackedMask {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
            outside: self.outside,
            runs,
        }
    }

    /// The marching-ants outline: closed loops in document space along the
    /// 50% coverage level, one per boundary — so a selection with a hole cut
    /// in it gets two. Marching squares over pixel centres, crossings
    /// interpolated, then simplified.
    pub fn contours(&self) -> Vec<Vec<(f32, f32)>> {
        let Some((bx0, by0, bx1, by1)) = self.bounds() else {
            return Vec::new();
        };
        // One sample per pixel centre, padded by one pixel of `outside` on
        // every side so each loop closes inside the grid.
        let gw = (bx1 - bx0 + 2) as usize;
        let gh = (by1 - by0 + 2) as usize;
        let mut g = vec![0u8; gw * gh];
        for j in 0..gh {
            for i in 0..gw {
                g[j * gw + i] = self.at(bx0 - 1 + i as i32, by0 - 1 + j as i32);
            }
        }
        let inside = |v: u8| v >= THRESHOLD;
        // Edge ids: `2 * (j * gw + i)` is the edge from sample (i, j) to
        // (i + 1, j); `+ 1` is the edge from (i, j) down to (i, j + 1).
        let hid = |i: usize, j: usize| 2 * (j * gw + i);
        let vid = |i: usize, j: usize| 2 * (j * gw + i) + 1;
        let crossing = |a: u8, b: u8| {
            let (a, b) = (a as f32, b as f32);
            ((THRESHOLD as f32 - 0.5 - a) / (b - a)).clamp(0.0, 1.0)
        };
        let pos = |id: usize| -> (f32, f32) {
            let cell = id / 2;
            let (i, j) = (cell % gw, cell / gw);
            let (sx, sy) = (
                bx0 as f32 - 0.5 + i as f32,
                by0 as f32 - 0.5 + j as f32,
            );
            if id % 2 == 0 {
                (sx + crossing(g[j * gw + i], g[j * gw + i + 1]), sy)
            } else {
                (sx, sy + crossing(g[j * gw + i], g[(j + 1) * gw + i]))
            }
        };

        let mut segs: Vec<(usize, usize)> = Vec::new();
        for j in 0..gh - 1 {
            for i in 0..gw - 1 {
                let tl = g[j * gw + i];
                let tr = g[j * gw + i + 1];
                let br = g[(j + 1) * gw + i + 1];
                let bl = g[(j + 1) * gw + i];
                let case = (inside(tl) as u8) << 3
                    | (inside(tr) as u8) << 2
                    | (inside(br) as u8) << 1
                    | inside(bl) as u8;
                let (top, bottom) = (hid(i, j), hid(i, j + 1));
                let (left, right) = (vid(i, j), vid(i + 1, j));
                let centre_in =
                    (tl as u32 + tr as u32 + br as u32 + bl as u32) as f32 / 4.0 >= THRESHOLD as f32 - 0.5;
                match case {
                    0 | 15 => {}
                    0b1000 | 0b0111 => segs.push((left, top)),
                    0b0100 | 0b1011 => segs.push((top, right)),
                    0b0010 | 0b1101 => segs.push((right, bottom)),
                    0b0001 | 0b1110 => segs.push((bottom, left)),
                    0b1100 | 0b0011 => segs.push((left, right)),
                    0b1001 | 0b0110 => segs.push((top, bottom)),
                    // Saddles: the centre decides whether the two inside
                    // corners join across the cell.
                    0b1010 => {
                        if centre_in {
                            segs.push((top, right));
                            segs.push((bottom, left));
                        } else {
                            segs.push((left, top));
                            segs.push((right, bottom));
                        }
                    }
                    _ => {
                        // 0b0101: TR and BL inside.
                        if centre_in {
                            segs.push((left, top));
                            segs.push((right, bottom));
                        } else {
                            segs.push((top, right));
                            segs.push((bottom, left));
                        }
                    }
                }
            }
        }

        // Every crossing is shared by exactly two segments; walk them into
        // loops.
        let mut at_edge: std::collections::HashMap<usize, [usize; 2]> =
            std::collections::HashMap::with_capacity(segs.len());
        for (s, &(a, b)) in segs.iter().enumerate() {
            for e in [a, b] {
                at_edge
                    .entry(e)
                    .and_modify(|v| v[1] = s)
                    .or_insert([s, usize::MAX]);
            }
        }
        let mut used = vec![false; segs.len()];
        let mut loops = Vec::new();
        for s0 in 0..segs.len() {
            if used[s0] {
                continue;
            }
            used[s0] = true;
            let (start, mut cur) = segs[s0];
            let mut seg = s0;
            let mut pts = vec![pos(start)];
            while cur != start {
                pts.push(pos(cur));
                let pair = at_edge[&cur];
                let next = if pair[0] == seg { pair[1] } else { pair[0] };
                if next == usize::MAX || used[next] {
                    break;
                }
                used[next] = true;
                let (a, b) = segs[next];
                cur = if a == cur { b } else { a };
                seg = next;
            }
            let simplified = simplify_loop(&pts, 0.35);
            if simplified.len() >= 3 {
                loops.push(simplified);
            }
        }
        loops
    }

    /// Grow by `n` px: every pixel within `n` of the selection joins it, with
    /// rounded corners. Exact Euclidean distance, so the cost does not depend
    /// on `n`.
    pub fn grow(&self, n: u32) -> Option<Self> {
        if n == 0 || self.is_all() {
            return Some(self.clone());
        }
        let (x0, y0, x1, y1) = self.bounds()?;
        let pad = n as i32 + 1;
        let (px, py) = (x0 - pad, y0 - pad);
        let (w, h) = ((x1 - x0 + 2 * pad) as usize, (y1 - y0 + 2 * pad) as usize);
        let mut f = vec![0f64; w * h];
        for j in 0..h {
            for i in 0..w {
                let inside = self.at(px + i as i32, py + j as i32) >= THRESHOLD;
                f[j * w + i] = if inside { 0.0 } else { EDT_FAR };
            }
        }
        edt_2d(&mut f, w, h);
        let reach = n as f32 + 1.0;
        let mut cov = vec![0u8; w * h];
        for j in 0..h {
            for i in 0..w {
                let orig = self.at(px + i as i32, py + j as i32);
                let d = (f[j * w + i] as f32).sqrt();
                let grown = ((reach - d).clamp(0.0, 1.0) * 255.0).round() as u8;
                cov[j * w + i] = orig.max(grown);
            }
        }
        Self {
            x: px,
            y: py,
            w: w as u32,
            h: h as u32,
            cov,
            outside: self.outside,
        }
        .normalize()
    }

    /// Shrink by `n` px — grow what is *not* selected. `None` once nothing is
    /// left.
    pub fn shrink(&self, n: u32) -> Option<Self> {
        if n == 0 {
            return Some(self.clone());
        }
        match self.inverted() {
            // Nothing unselected: Select All has no edge to pull in.
            None => Some(self.clone()),
            Some(inv) => inv.grow(n)?.inverted(),
        }
    }

    /// Soften the edge over roughly `r` px: three box blurs, which approximate
    /// a gaussian closely enough that the steps never show.
    pub fn feather(&self, r: u32) -> Option<Self> {
        if r == 0 || self.is_all() {
            return Some(self.clone());
        }
        let (x0, y0, x1, y1) = self.bounds()?;
        let b = (r as usize).div_ceil(3).max(1);
        let pad = 3 * b as i32 + 1;
        let (px, py) = (x0 - pad, y0 - pad);
        let (w, h) = ((x1 - x0 + 2 * pad) as usize, (y1 - y0 + 2 * pad) as usize);
        let mut buf = vec![0f32; w * h];
        for j in 0..h {
            for i in 0..w {
                buf[j * w + i] = self.at(px + i as i32, py + j as i32) as f32;
            }
        }
        let mut line = Vec::new();
        for _ in 0..3 {
            for j in 0..h {
                box_blur(&mut buf[j * w..(j + 1) * w], b, &mut line);
            }
            let mut col = vec![0f32; h];
            for i in 0..w {
                for j in 0..h {
                    col[j] = buf[j * w + i];
                }
                box_blur(&mut col, b, &mut line);
                for j in 0..h {
                    buf[j * w + i] = col[j];
                }
            }
        }
        Self {
            x: px,
            y: py,
            w: w as u32,
            h: h as u32,
            cov: buf
                .iter()
                .map(|&v| v.round().clamp(0.0, 255.0) as u8)
                .collect(),
            outside: self.outside,
        }
        .normalize()
    }
}

/// Stand-in for "infinitely far" in the distance transform. Finite so the
/// parabola arithmetic stays exact in f64; far beyond any image.
const EDT_FAR: f64 = 1e12;

/// Squared Euclidean distance transform, in place: on entry 0 marks a source
/// and [`EDT_FAR`] everything else; on exit each cell holds its squared
/// distance to the nearest source. Felzenszwalb & Huttenlocher's separable
/// lower-envelope algorithm, rows then columns.
fn edt_2d(f: &mut [f64], w: usize, h: usize) {
    let n = w.max(h);
    let mut line = vec![0f64; n];
    let mut out = vec![0f64; n];
    let mut v = vec![0usize; n];
    let mut z = vec![0f64; n + 1];
    for j in 0..h {
        line[..w].copy_from_slice(&f[j * w..(j + 1) * w]);
        edt_1d(&line[..w], &mut out[..w], &mut v, &mut z);
        f[j * w..(j + 1) * w].copy_from_slice(&out[..w]);
    }
    for i in 0..w {
        for j in 0..h {
            line[j] = f[j * w + i];
        }
        edt_1d(&line[..h], &mut out[..h], &mut v, &mut z);
        for j in 0..h {
            f[j * w + i] = out[j];
        }
    }
}

fn edt_1d(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let qf = q as f64;
        let s = loop {
            let pf = v[k] as f64;
            let s = ((f[q] + qf * qf) - (f[v[k]] + pf * pf)) / (2.0 * qf - 2.0 * pf);
            // `z[0]` is -inf and `s` is finite, so this never walks past 0.
            if s <= z[k] {
                k -= 1;
            } else {
                break s;
            }
        };
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, dq) in d.iter_mut().enumerate() {
        let qf = q as f64;
        while z[k + 1] < qf {
            k += 1;
        }
        let p = v[k] as f64;
        *dq = (qf - p) * (qf - p) + f[v[k]];
    }
}

/// One box-blur pass of radius `r` over `data`, clamping at the ends. The
/// callers pad with `outside`, so the clamp only ever repeats that value.
fn box_blur(data: &mut [f32], r: usize, tmp: &mut Vec<f32>) {
    let n = data.len();
    if n == 0 {
        return;
    }
    tmp.clear();
    tmp.extend_from_slice(data);
    let at = |i: isize| tmp[i.clamp(0, n as isize - 1) as usize];
    let ri = r as isize;
    let norm = 1.0 / (2 * r + 1) as f32;
    let mut sum: f32 = (-ri..=ri).map(at).sum();
    for (i, d) in data.iter_mut().enumerate() {
        *d = sum * norm;
        let ii = i as isize;
        sum += at(ii + ri + 1) - at(ii - ri);
    }
}

/// Ramer–Douglas–Peucker on a closed loop: split at the first point and the
/// point farthest from it, and simplify each half.
fn simplify_loop(pts: &[(f32, f32)], eps: f32) -> Vec<(f32, f32)> {
    if pts.len() < 4 {
        return pts.to_vec();
    }
    let p0 = pts[0];
    let far = pts
        .iter()
        .enumerate()
        .max_by(|a, b| {
            let da = (a.1 .0 - p0.0).hypot(a.1 .1 - p0.1);
            let db = (b.1 .0 - p0.0).hypot(b.1 .1 - p0.1);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    if far == 0 {
        return vec![p0];
    }
    let mut first: Vec<(f32, f32)> = pts[..=far].to_vec();
    let mut second: Vec<(f32, f32)> = pts[far..].to_vec();
    second.push(p0);
    first = rdp(&first, eps);
    second = rdp(&second, eps);
    // Both halves share their end points; drop the duplicates.
    first.pop();
    second.pop();
    first.extend(second);
    first
}

fn rdp(pts: &[(f32, f32)], eps: f32) -> Vec<(f32, f32)> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (ax, ay) = pts[a];
        let (bx, by) = pts[b];
        let (dx, dy) = (bx - ax, by - ay);
        let len = dx.hypot(dy);
        let mut best = (0.0f32, a);
        for (i, &(px, py)) in pts.iter().enumerate().take(b).skip(a + 1) {
            let d = if len < 1e-6 {
                (px - ax).hypot(py - ay)
            } else {
                ((px - ax) * dy - (py - ay) * dx).abs() / len
            };
            if d > best.0 {
                best = (d, i);
            }
        }
        if best.0 > eps {
            keep[best.1] = true;
            stack.push((a, best.1));
            stack.push((best.1, b));
        }
    }
    pts.iter()
        .zip(keep)
        .filter_map(|(&p, k)| k.then_some(p))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HUGE: (i32, i32, i32, i32) = (-10_000, -10_000, 10_000, 10_000);

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> SelectionMask {
        SelectionMask::from_path(&rect_path((x0, y0), (x1, y1)), HUGE).expect("rect")
    }

    fn count(m: &SelectionMask, x0: i32, y0: i32, x1: i32, y1: i32) -> usize {
        let mut n = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                if m.at(x, y) >= THRESHOLD {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn a_path_can_sit_left_of_and_above_the_origin() {
        let m = rect(-20.0, -10.0, -4.0, 6.0);
        assert_eq!((m.x, m.y, m.w, m.h), (-20, -10, 16, 16));
        assert_eq!(m.at(-12, 0), 255);
        assert_eq!(m.at(-3, 0), 0);
        assert_eq!(m.outside, 0);
    }

    #[test]
    fn rectangles_have_hard_edges() {
        let m = rect(2.3, 2.6, 9.7, 8.2);
        assert!(m.cov.iter().all(|&c| c == 0 || c == 255), "{:?}", m.cov);
    }

    #[test]
    fn combine_follows_the_truth_table() {
        let a = rect(0.0, 0.0, 10.0, 10.0);
        let b = rect(5.0, 0.0, 15.0, 10.0);
        let add = SelectionMask::combine(Some(&a), &b, SelOp::Add).unwrap();
        assert_eq!(count(&add, -5, -5, 20, 20), 150);
        let sub = SelectionMask::combine(Some(&a), &b, SelOp::Subtract).unwrap();
        assert_eq!(count(&sub, -5, -5, 20, 20), 50);
        assert_eq!(sub.at(7, 5), 0);
        assert_eq!(sub.at(2, 5), 255);
        let int = SelectionMask::combine(Some(&a), &b, SelOp::Intersect).unwrap();
        assert_eq!(count(&int, -5, -5, 20, 20), 50);
        assert_eq!((int.x, int.w), (5, 5), "intersect trims to the overlap");
        let rep = SelectionMask::combine(Some(&a), &b, SelOp::Replace).unwrap();
        assert_eq!(rep, b);
    }

    #[test]
    fn disjoint_intersect_is_nothing_and_add_to_nothing_is_the_shape() {
        let a = rect(0.0, 0.0, 4.0, 4.0);
        let b = rect(10.0, 10.0, 14.0, 14.0);
        assert!(SelectionMask::combine(Some(&a), &b, SelOp::Intersect).is_none());
        assert_eq!(SelectionMask::combine(None, &b, SelOp::Add).unwrap(), b);
        assert!(SelectionMask::combine(None, &b, SelOp::Subtract).is_none());
    }

    #[test]
    fn invert_twice_is_the_original_and_all_minus_a_rect_is_a_hole() {
        let a = rect(3.0, 3.0, 9.0, 7.0);
        let inv = a.inverted().unwrap();
        assert_eq!(inv.outside, 255);
        assert_eq!(inv.at(5, 5), 0);
        assert_eq!(inv.at(-500, 900), 255);
        assert_eq!(inv.inverted().unwrap(), a);

        assert!(SelectionMask::all().inverted().is_none());
        let holed = SelectionMask::combine(Some(&SelectionMask::all()), &a, SelOp::Subtract).unwrap();
        assert_eq!(holed, inv);
    }

    #[test]
    fn normalize_trims_the_box() {
        let mut cov = vec![0u8; 100];
        cov[3 * 10 + 4] = 200;
        cov[6 * 10 + 7] = 255;
        let m = SelectionMask {
            x: 100,
            y: 50,
            w: 10,
            h: 10,
            cov,
            outside: 0,
        }
        .normalize()
        .unwrap();
        assert_eq!((m.x, m.y, m.w, m.h), (104, 53, 4, 4));
        assert_eq!(m.at(104, 53), 200);
        assert_eq!(m.at(107, 56), 255);
        let empty = SelectionMask {
            x: 0,
            y: 0,
            w: 3,
            h: 3,
            cov: vec![0; 9],
            outside: 0,
        };
        assert!(empty.normalize().is_none());
    }

    #[test]
    fn pack_round_trips() {
        for m in [
            rect(0.0, 0.0, 30.0, 20.0),
            SelectionMask::from_path(&ellipse_path((0.0, 0.0), (40.0, 25.0)), HUGE).unwrap(),
            SelectionMask::all(),
            rect(0.0, 0.0, 5.0, 5.0).inverted().unwrap(),
        ] {
            assert_eq!(m.pack().unpack(), m);
        }
    }

    #[test]
    fn identity_to_cell_is_a_byte_exact_crop() {
        let m = SelectionMask::from_path(&ellipse_path((4.0, 6.0), (40.0, 30.0)), HUGE).unwrap();
        let c = m.to_cell(&Transform::default(), 64, 48, 64.0, 48.0);
        for y in 0..48 {
            for x in 0..64 {
                assert_eq!(c.at(x, y), m.at(x as i32, y as i32), "({x},{y})");
            }
        }
    }

    #[test]
    fn an_evenly_larger_cell_is_an_exact_translation() {
        // A 104x88 cell centred on a 64x48 frame: cell (20, 20) is doc (0, 0).
        let m = rect(0.0, 0.0, 10.0, 10.0);
        let xf = Transform::default();
        assert_eq!(integer_offset(&xf, 104, 88, 64.0, 48.0), Some((-20, -20)));
        let c = m.to_cell(&xf, 104, 88, 64.0, 48.0);
        assert_eq!((c.x, c.y, c.w, c.h), (20, 20, 10, 10));
        assert_eq!(c.at(25, 25), 255);
        assert_eq!(c.at(19, 25), 0);
    }

    #[test]
    fn a_rotated_layer_samples_the_selection_where_it_lands() {
        let m = rect(20.0, 20.0, 44.0, 44.0);
        let xf = Transform {
            rot: 0.3,
            scale: 1.5,
            tx: 3.0,
            ty: -2.0,
        };
        let (cw, ch, pw, ph) = (64u32, 64u32, 64.0, 64.0);
        let c = m.to_cell(&xf, cw, ch, pw, ph);
        // The doc-space centre of the selection maps somewhere inside the cell,
        // and is fully selected there; a far corner is not.
        let (u, v) = xf.doc_to_cell(32.0, 32.0, 64.0, 64.0, pw, ph);
        assert!(c.at(u as u32, v as u32) >= 250);
        let (u, v) = xf.doc_to_cell(60.0, 60.0, 64.0, 64.0, pw, ph);
        if u >= 0.0 && v >= 0.0 && u < 64.0 && v < 64.0 {
            assert_eq!(c.at(u as u32, v as u32), 0);
        }
    }

    #[test]
    fn select_all_covers_every_cell_pixel() {
        let xf = Transform {
            rot: 1.0,
            ..Transform::default()
        };
        for xf in [Transform::default(), xf] {
            let c = SelectionMask::all().to_cell(&xf, 16, 12, 16.0, 12.0);
            assert_eq!((c.w, c.h), (16, 12));
            assert!(c.cov.iter().all(|&v| v == 255));
        }
    }

    #[test]
    fn to_cell_then_from_cell_comes_back() {
        let m = SelectionMask::from_path(&ellipse_path((10.0, 10.0), (50.0, 40.0)), HUGE).unwrap();
        let xf = Transform {
            rot: 0.4,
            scale: 1.0,
            tx: 5.0,
            ty: 0.0,
        };
        let c = m.to_cell(&xf, 64, 64, 64.0, 64.0);
        let back = SelectionMask::from_cell(
            c.x as i32, c.y as i32, c.w, c.h, &c.cov, &xf, 64, 64, 64.0, 64.0,
        )
        .unwrap();
        assert!(back.at(30, 25) >= 250);
        assert_eq!(back.at(5, 5), 0);
        let (a, b) = (count(&m, 0, 0, 64, 64) as i64, count(&back, 0, 0, 64, 64) as i64);
        assert!((a - b).abs() < a / 20, "{a} vs {b}");
    }

    #[test]
    fn a_rect_outlines_as_one_loop_on_its_edges() {
        let m = rect(2.0, 3.0, 12.0, 9.0);
        let loops = m.contours();
        assert_eq!(loops.len(), 1);
        let l = &loops[0];
        let (nx, xx) = l.iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
        let (ny, xy) = l.iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
        assert!((nx - 2.0).abs() < 0.01 && (xx - 12.0).abs() < 0.01, "{nx}..{xx}");
        assert!((ny - 3.0).abs() < 0.01 && (xy - 9.0).abs() < 0.01, "{ny}..{xy}");
        assert!(l.len() <= 8, "a rectangle simplifies to a handful of points");
    }

    #[test]
    fn a_donut_outlines_as_two_loops_and_all_as_none() {
        let outer = rect(0.0, 0.0, 20.0, 20.0);
        let hole = rect(6.0, 6.0, 14.0, 14.0);
        let donut = SelectionMask::combine(Some(&outer), &hole, SelOp::Subtract).unwrap();
        assert_eq!(donut.contours().len(), 2);
        assert!(SelectionMask::all().contours().is_empty());
        let holed = hole.inverted().unwrap();
        assert_eq!(holed.contours().len(), 1);
    }

    #[test]
    fn an_ellipse_path_stays_on_the_ellipse() {
        let pts = ellipse_path((0.0, 0.0), (100.0, 40.0));
        assert!(pts.len() >= 32);
        for &(x, y) in &pts {
            let e = ((x - 50.0) / 50.0).powi(2) + ((y - 20.0) / 20.0).powi(2);
            assert!((e - 1.0).abs() < 1e-3, "({x},{y}) off the ellipse: {e}");
        }
        assert!(ellipse_path((0.0, 0.0), (0.2, 30.0)).is_empty());
    }

    #[test]
    fn grow_adds_n_on_each_side_and_rounds_the_corners() {
        let m = rect(10.0, 10.0, 20.0, 20.0);
        let g = m.grow(3).unwrap();
        // Straight edges moved out by exactly 3.
        assert_eq!(g.at(7, 15), 255);
        assert!(g.at(6, 15) < THRESHOLD);
        assert_eq!(g.at(22, 15), 255);
        assert!(g.at(23, 15) < THRESHOLD);
        // The corner is rounded, not square.
        assert!(g.at(7, 7) < THRESHOLD, "corner should be cut: {}", g.at(7, 7));
        assert!(g.at(8, 8) >= THRESHOLD);
    }

    #[test]
    fn shrink_undoes_grow_on_a_convex_shape_and_too_far_is_nothing() {
        let m = rect(10.0, 10.0, 30.0, 26.0);
        let back = m.grow(4).unwrap().shrink(4).unwrap();
        let (a, b) = (count(&back, 0, 0, 40, 40), count(&m, 0, 0, 40, 40));
        assert!(a.abs_diff(b) <= 4, "{a} vs {b}");
        assert_eq!(back.at(10, 10), 255, "the corner comes back square");
        assert!(m.shrink(9).is_none());
        let s = m.shrink(2).unwrap();
        assert_eq!(count(&s, 0, 0, 40, 40), 16 * 12);
    }

    #[test]
    fn feather_softens_the_edge_and_keeps_the_mass() {
        let m = rect(0.0, 0.0, 60.0, 60.0);
        let f = m.feather(9).unwrap();
        assert_eq!(f.at(30, 30), 255);
        let edge = f.at(0, 30) as i32;
        assert!((edge - 128).abs() < 40, "edge {edge}");
        let mass = |m: &SelectionMask| m.cov.iter().map(|&c| c as u64).sum::<u64>();
        let (a, b) = (mass(&m) as f64, mass(&f) as f64);
        assert!((a - b).abs() / a < 0.02, "{a} vs {b}");
    }
}
