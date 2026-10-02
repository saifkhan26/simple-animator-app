//! Ports of the Qt 5.15 raster routines Krita's pixel brush runs through.
//!
//! Krita never rasterizes a bitmap brush tip itself; it hands the tip to Qt.
//! `QImage::scaled` builds the tip's mipmap levels and scales the paper
//! texture, and a `QPainter` with `SmoothPixmapTransform` draws the chosen
//! level scaled, rotated and nudged by the dab's sub-pixel offset. So the
//! pixels of a Krita dab are whatever Qt's raster engine makes of that: its
//! 16.16 fixed-point sampling, the 16-bit-per-channel pipeline it switches to
//! for a `Format_ARGB32` target, the rasterizer's choice of which pixels a
//! rotated image covers, and the approximate reciprocal it un-premultiplies
//! with.
//!
//! Each function names the Qt 5.15.7 function it follows (the version Krita
//! 5.2.9 ships). Where Qt dispatches to an x86 SIMD variant at runtime, the
//! variant's arithmetic is the one reproduced, since that is what runs.

use std::cell::Cell;

/// A `QImage` in one of the 32-bit formats: one `0xAARRGGBB` word per pixel,
/// rows packed. Whether the colour is premultiplied is the caller's business,
/// exactly as with `Format_ARGB32` versus `Format_ARGB32_Premultiplied`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image32 {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

impl Image32 {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![0; w * h],
        }
    }

    /// `QImage::copy(x, y, w, h)`: the part outside the source comes back
    /// zero, which for `Format_ARGB32` is transparent.
    pub fn copy(&self, x: i32, y: i32, w: usize, h: usize) -> Self {
        let mut out = Self::new(w, h);
        for oy in 0..h {
            let sy = y + oy as i32;
            if sy < 0 || sy >= self.h as i32 {
                continue;
            }
            for ox in 0..w {
                let sx = x + ox as i32;
                if sx < 0 || sx >= self.w as i32 {
                    continue;
                }
                out.px[oy * w + ox] = self.px[sy as usize * self.w + sx as usize];
            }
        }
        out
    }
}

#[inline]
pub fn q_red(p: u32) -> u32 {
    (p >> 16) & 0xff
}
#[inline]
pub fn q_green(p: u32) -> u32 {
    (p >> 8) & 0xff
}
#[inline]
pub fn q_blue(p: u32) -> u32 {
    p & 0xff
}
#[inline]
pub fn q_alpha(p: u32) -> u32 {
    p >> 24
}
#[inline]
pub fn q_rgba(r: u32, g: u32, b: u32, a: u32) -> u32 {
    ((a & 0xff) << 24) | ((r & 0xff) << 16) | ((g & 0xff) << 8) | (b & 0xff)
}

/// `qRound(double)`, which rounds halves up rather than away from zero.
#[inline]
pub fn qround(d: f64) -> i32 {
    if d >= 0.0 {
        (d + 0.5) as i32
    } else {
        (d - ((d - 1.0) as i32) as f64 + 0.5) as i32 + (d - 1.0) as i32
    }
}

#[inline]
pub fn fuzzy_is_null(d: f64) -> bool {
    d.abs() <= 0.000_000_000_001
}

#[inline]
pub fn fuzzy_compare(p1: f64, p2: f64) -> bool {
    (p1 - p2).abs() * 1_000_000_000_000.0 <= p1.abs().min(p2.abs())
}

/// Approximate reciprocal, `_mm_rcp_ss`. Krita and Qt both divide through
/// this instruction in their pixel loops; its result is the CPU's own table
/// lookup, so the only way to agree with them bit for bit is to ask the same
/// instruction.
#[inline]
pub fn rcp(x: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: SSE is part of the x86_64 baseline.
        unsafe {
            use std::arch::x86_64::{_mm_cvtss_f32, _mm_rcp_ss, _mm_set_ss};
            _mm_cvtss_f32(_mm_rcp_ss(_mm_set_ss(x)))
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        1.0 / x
    }
}

/// `reciprocal_mul_ss` / `reciprocal_mul_ps`: the approximate reciprocal
/// sharpened by one Newton-Raphson step, then scaled.
#[inline]
fn reciprocal_mul(a: f32, mul: f32) -> f32 {
    let ia = rcp(a);
    let ia = (ia + ia) - ia * (ia * a);
    ia * mul
}

/// `_mm_cvtps_epi32` under the default rounding mode: nearest, ties to even.
#[inline]
fn cvt_round(x: f32) -> i32 {
    if !(-2_147_483_648.0..2_147_483_648.0).contains(&x) {
        i32::MIN
    } else {
        x.round_ties_even() as i32
    }
}

// ---------------------------------------------------------------------------
// QTransform
// ---------------------------------------------------------------------------

/// `QTransform::TransformationType`. The discriminants are Qt's: the
/// "dirty" bookkeeping compares them numerically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Tx {
    None = 0,
    Translate = 1,
    Scale = 2,
    Rotate = 4,
    Shear = 8,
}

/// An affine `QTransform`, including its lazily classified type.
///
/// The classification is part of the arithmetic, not an optimisation to be
/// skipped: `scale()` on a matrix Qt has already decided is the identity
/// *overwrites* the diagonal instead of multiplying into it, and Krita's
/// pyramid leans on that when it nudges a pure translation off being one.
#[derive(Clone, Debug)]
pub struct QTransform {
    pub m11: f64,
    pub m12: f64,
    pub m21: f64,
    pub m22: f64,
    pub dx: f64,
    pub dy: f64,
    ty: Cell<Tx>,
    dirty: Cell<Tx>,
}

impl Default for QTransform {
    fn default() -> Self {
        Self::identity()
    }
}

impl QTransform {
    pub fn identity() -> Self {
        Self {
            m11: 1.0,
            m12: 0.0,
            m21: 0.0,
            m22: 1.0,
            dx: 0.0,
            dy: 0.0,
            ty: Cell::new(Tx::None),
            dirty: Cell::new(Tx::None),
        }
    }

    pub fn from_translate(dx: f64, dy: f64) -> Self {
        let mut t = Self::identity();
        t.dx = dx;
        t.dy = dy;
        t.ty.set(if dx == 0.0 && dy == 0.0 {
            Tx::None
        } else {
            Tx::Translate
        });
        t
    }

    pub fn from_scale(sx: f64, sy: f64) -> Self {
        let mut t = Self::identity();
        t.m11 = sx;
        t.m22 = sy;
        t.ty.set(if sx == 1.0 && sy == 1.0 { Tx::None } else { Tx::Scale });
        t
    }

    /// `QTransform::type()`, caching its answer the way Qt does.
    pub fn ty(&self) -> Tx {
        let dirty = self.dirty.get();
        let ty = self.ty.get();
        if dirty == Tx::None || dirty < ty {
            return ty;
        }
        let mut level = dirty;
        let result = loop {
            match level {
                Tx::Shear | Tx::Rotate => {
                    if !fuzzy_is_null(self.m12) || !fuzzy_is_null(self.m21) {
                        let dot = self.m11 * self.m21 + self.m12 * self.m22;
                        break if fuzzy_is_null(dot) { Tx::Rotate } else { Tx::Shear };
                    }
                    level = Tx::Scale;
                }
                Tx::Scale => {
                    if !fuzzy_is_null(self.m11 - 1.0) || !fuzzy_is_null(self.m22 - 1.0) {
                        break Tx::Scale;
                    }
                    level = Tx::Translate;
                }
                Tx::Translate => {
                    if !fuzzy_is_null(self.dx) || !fuzzy_is_null(self.dy) {
                        break Tx::Translate;
                    }
                    level = Tx::None;
                }
                Tx::None => break Tx::None,
            }
        };
        self.ty.set(result);
        self.dirty.set(Tx::None);
        result
    }

    #[inline]
    fn inline_type(&self) -> Tx {
        if self.dirty.get() == Tx::None {
            self.ty.get()
        } else {
            self.ty()
        }
    }

    fn mark_dirty(&self, at_least: Tx) {
        if self.dirty.get() < at_least {
            self.dirty.set(at_least);
        }
    }

    pub fn is_identity(&self) -> bool {
        self.inline_type() == Tx::None
    }

    pub fn translate(&mut self, dx: f64, dy: f64) -> &mut Self {
        if dx == 0.0 && dy == 0.0 {
            return self;
        }
        match self.inline_type() {
            Tx::None => {
                self.dx = dx;
                self.dy = dy;
            }
            Tx::Translate => {
                self.dx += dx;
                self.dy += dy;
            }
            Tx::Scale => {
                self.dx += dx * self.m11;
                self.dy += dy * self.m22;
            }
            Tx::Rotate | Tx::Shear => {
                self.dx += dx * self.m11 + dy * self.m21;
                self.dy += dy * self.m22 + dx * self.m12;
            }
        }
        self.mark_dirty(Tx::Translate);
        self
    }

    pub fn scale(&mut self, sx: f64, sy: f64) -> &mut Self {
        if sx == 1.0 && sy == 1.0 {
            return self;
        }
        match self.inline_type() {
            Tx::None | Tx::Translate => {
                self.m11 = sx;
                self.m22 = sy;
            }
            Tx::Rotate | Tx::Shear => {
                self.m12 *= sx;
                self.m21 *= sy;
                self.m11 *= sx;
                self.m22 *= sy;
            }
            Tx::Scale => {
                self.m11 *= sx;
                self.m22 *= sy;
            }
        }
        self.mark_dirty(Tx::Scale);
        self
    }

    pub fn rotate_radians(&mut self, a: f64) -> &mut Self {
        let sina = a.sin();
        let cosa = a.cos();
        match self.inline_type() {
            Tx::None | Tx::Translate => {
                self.m11 = cosa;
                self.m12 = sina;
                self.m21 = -sina;
                self.m22 = cosa;
            }
            Tx::Scale => {
                let tm11 = cosa * self.m11;
                let tm12 = sina * self.m22;
                let tm21 = -sina * self.m11;
                let tm22 = cosa * self.m22;
                self.m11 = tm11;
                self.m12 = tm12;
                self.m21 = tm21;
                self.m22 = tm22;
            }
            Tx::Rotate | Tx::Shear => {
                let tm11 = cosa * self.m11 + sina * self.m21;
                let tm12 = cosa * self.m12 + sina * self.m22;
                let tm21 = -sina * self.m11 + cosa * self.m21;
                let tm22 = -sina * self.m12 + cosa * self.m22;
                self.m11 = tm11;
                self.m12 = tm12;
                self.m21 = tm21;
                self.m22 = tm22;
            }
        }
        self.mark_dirty(Tx::Rotate);
        self
    }

    /// `self * m`: apply `self`, then `m`.
    pub fn mul(&self, m: &QTransform) -> QTransform {
        let other = m.inline_type();
        if other == Tx::None {
            return self.clone();
        }
        let this = self.inline_type();
        if this == Tx::None {
            return m.clone();
        }
        let t = this.max(other);
        let mut r = QTransform::identity();
        match t {
            Tx::None => {}
            Tx::Translate => {
                r.dx = self.dx + m.dx;
                r.dy += self.dy + m.dy;
            }
            Tx::Scale => {
                r.m11 = self.m11 * m.m11;
                r.m22 = self.m22 * m.m22;
                r.dx = self.dx * m.m11 + m.dx;
                r.dy = self.dy * m.m22 + m.dy;
            }
            Tx::Rotate | Tx::Shear => {
                r.m11 = self.m11 * m.m11 + self.m12 * m.m21;
                r.m12 = self.m11 * m.m12 + self.m12 * m.m22;
                r.m21 = self.m21 * m.m11 + self.m22 * m.m21;
                r.m22 = self.m21 * m.m12 + self.m22 * m.m22;
                r.dx = self.dx * m.m11 + self.dy * m.m21 + m.dx;
                r.dy = self.dx * m.m12 + self.dy * m.m22 + m.dy;
            }
        }
        r.ty.set(t);
        r.dirty.set(t);
        r
    }

    /// `operator*=`. Differs from [`mul`](Self::mul) only in how it treats
    /// an identity left-hand side, which it replaces wholesale.
    pub fn mul_assign(&mut self, o: &QTransform) {
        let other = o.inline_type();
        if other == Tx::None {
            return;
        }
        let this = self.inline_type();
        if this == Tx::None {
            *self = o.clone();
            return;
        }
        let t = this.max(other);
        match t {
            Tx::None => {}
            Tx::Translate => {
                self.dx += o.dx;
                self.dy += o.dy;
            }
            Tx::Scale => {
                let m11 = self.m11 * o.m11;
                let m22 = self.m22 * o.m22;
                let m31 = self.dx * o.m11 + o.dx;
                let m32 = self.dy * o.m22 + o.dy;
                self.m11 = m11;
                self.m22 = m22;
                self.dx = m31;
                self.dy = m32;
            }
            Tx::Rotate | Tx::Shear => {
                let m11 = self.m11 * o.m11 + self.m12 * o.m21;
                let m12 = self.m11 * o.m12 + self.m12 * o.m22;
                let m21 = self.m21 * o.m11 + self.m22 * o.m21;
                let m22 = self.m21 * o.m12 + self.m22 * o.m22;
                let m31 = self.dx * o.m11 + self.dy * o.m21 + o.dx;
                let m32 = self.dx * o.m12 + self.dy * o.m22 + o.dy;
                self.m11 = m11;
                self.m12 = m12;
                self.m21 = m21;
                self.m22 = m22;
                self.dx = m31;
                self.dy = m32;
            }
        }
        self.dirty.set(t);
        self.ty.set(t);
    }

    pub fn inverted(&self) -> QTransform {
        let mut inv = QTransform::identity();
        match self.inline_type() {
            Tx::None => {}
            Tx::Translate => {
                inv.dx = -self.dx;
                inv.dy = -self.dy;
            }
            Tx::Scale => {
                if !fuzzy_is_null(self.m11) && !fuzzy_is_null(self.m22) {
                    inv.m11 = 1.0 / self.m11;
                    inv.m22 = 1.0 / self.m22;
                    inv.dx = -self.dx * inv.m11;
                    inv.dy = -self.dy * inv.m22;
                }
            }
            Tx::Rotate | Tx::Shear => {
                // QMatrix::inverted.
                let dtr = self.m11 * self.m22 - self.m12 * self.m21;
                if dtr != 0.0 {
                    let dinv = 1.0 / dtr;
                    inv.m11 = self.m22 * dinv;
                    inv.m12 = -self.m12 * dinv;
                    inv.m21 = -self.m21 * dinv;
                    inv.m22 = self.m11 * dinv;
                    inv.dx = (self.m21 * self.dy - self.m22 * self.dx) * dinv;
                    inv.dy = (self.m12 * self.dx - self.m11 * self.dy) * dinv;
                }
            }
        }
        inv.ty.set(self.ty.get());
        inv.dirty.set(self.dirty.get());
        inv
    }

    #[inline]
    fn map_with(&self, t: Tx, x: f64, y: f64) -> (f64, f64) {
        match t {
            Tx::None => (x, y),
            Tx::Translate => (x + self.dx, y + self.dy),
            Tx::Scale => (self.m11 * x + self.dx, self.m22 * y + self.dy),
            Tx::Rotate | Tx::Shear => (
                self.m11 * x + self.m21 * y + self.dx,
                self.m12 * x + self.m22 * y + self.dy,
            ),
        }
    }

    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        let t = self.inline_type();
        self.map_with(t, x, y)
    }

    /// `QTransform::mapRect(const QRectF &)`.
    pub fn map_rect(&self, r: RectF) -> RectF {
        let t = self.inline_type();
        if t <= Tx::Translate {
            return RectF {
                x: r.x + self.dx,
                y: r.y + self.dy,
                w: r.w,
                h: r.h,
            };
        }
        if t <= Tx::Scale {
            let mut x = self.m11 * r.x + self.dx;
            let mut y = self.m22 * r.y + self.dy;
            let mut w = self.m11 * r.w;
            let mut h = self.m22 * r.h;
            if w < 0.0 {
                w = -w;
                x -= w;
            }
            if h < 0.0 {
                h = -h;
                y -= h;
            }
            return RectF { x, y, w, h };
        }
        let corners = [
            (r.x, r.y),
            (r.x + r.w, r.y),
            (r.x + r.w, r.y + r.h),
            (r.x, r.y + r.h),
        ];
        let (x, y) = self.map_with(t, corners[0].0, corners[0].1);
        let (mut xmin, mut ymin, mut xmax, mut ymax) = (x, y, x, y);
        for &(cx, cy) in &corners[1..] {
            let (x, y) = self.map_with(t, cx, cy);
            xmin = xmin.min(x);
            ymin = ymin.min(y);
            xmax = xmax.max(x);
            ymax = ymax.max(y);
        }
        RectF {
            x: xmin,
            y: ymin,
            w: xmax - xmin,
            h: ymax - ymin,
        }
    }

    /// `QTransform::mapRect(const QRect &)`, for the scale-only case it is
    /// used with.
    pub fn map_rect_int(&self, r: Rect) -> Rect {
        let t = self.inline_type();
        if t <= Tx::Translate {
            return Rect {
                x: r.x + qround(self.dx),
                y: r.y + qround(self.dy),
                w: r.w,
                h: r.h,
            };
        }
        debug_assert!(t <= Tx::Scale, "only scale-only integer rects are needed");
        let mut x = qround(self.m11 * r.x as f64 + self.dx);
        let mut y = qround(self.m22 * r.y as f64 + self.dy);
        let mut w = qround(self.m11 * r.w as f64);
        let mut h = qround(self.m22 * r.h as f64);
        if w < 0 {
            w = -w;
            x -= w;
        }
        if h < 0 {
            h = -h;
            y -= h;
        }
        Rect { x, y, w, h }
    }
}

/// `QRectF`: origin and size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl RectF {
    pub fn is_valid(self) -> bool {
        self.w > 0.0 && self.h > 0.0
    }

    pub fn to_aligned_rect(self) -> Rect {
        let xmin = self.x.floor() as i32;
        let xmax = (self.x + self.w).ceil() as i32;
        let ymin = self.y.floor() as i32;
        let ymax = (self.y + self.h).ceil() as i32;
        Rect {
            x: xmin,
            y: ymin,
            w: xmax - xmin,
            h: ymax - ymin,
        }
    }
}

/// `QRect`, as origin and size (Qt's inclusive `right()` is `x + w - 1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

// ---------------------------------------------------------------------------
// qSmoothScaleImage (qimagescale.cpp, with the SSE4.1 kernels' arithmetic)
// ---------------------------------------------------------------------------

fn calc_points(s: i32, d: i32) -> Vec<i32> {
    let up = d >= s;
    let mut val: i64 = if up {
        (0x8000 * s / d - 0x8000) as i64
    } else {
        0
    };
    let inc: i64 = ((s as i64) << 16) / d as i64;
    let mut p = Vec::with_capacity(d as usize);
    for _ in 0..d {
        p.push((val >> 16).max(0) as i32);
        val += inc;
    }
    p
}

fn calc_a_points(s: i32, d: i32, up: bool) -> Vec<i32> {
    let mut p = Vec::with_capacity(d as usize);
    if up {
        let mut val: i64 = (0x8000 * s / d - 0x8000) as i64;
        let inc: i64 = ((s as i64) << 16) / d as i64;
        for _ in 0..d {
            let pos = (val >> 16) as i32;
            if pos < 0 || pos >= s - 1 {
                p.push(0);
            } else {
                p.push(((val >> 8) - ((val >> 8) & 0xffff_ff00)) as i32);
            }
            val += inc;
        }
    } else {
        let mut val: i64 = 0;
        let inc: i64 = ((s as i64) << 16) / d as i64;
        let cp = ((d << 14) + s - 1) / s;
        for _ in 0..d {
            let ap = ((0x10000 - (val & 0xffff) as i32) * cp) >> 16;
            p.push(ap | (cp << 16));
            val += inc;
        }
    }
    p
}

#[inline]
fn channels(p: u32) -> [u32; 4] {
    [q_red(p), q_green(p), q_blue(p), q_alpha(p)]
}

/// `qt_qimageScaleAARGBA_helper`, in the unsigned 32-bit lanes the SSE4.1
/// kernel accumulates in.
#[inline]
fn scale_helper(src: &[u32], mut pix: usize, xyap: i32, cxy: i32, step: usize) -> [u32; 4] {
    let mut acc = channels(src[pix]).map(|c| c.wrapping_mul(xyap as u32));
    let mut j = (1 << 14) - xyap;
    while j > cxy {
        pix += step;
        let c = channels(src[pix]);
        for k in 0..4 {
            acc[k] = acc[k].wrapping_add(c[k].wrapping_mul(cxy as u32));
        }
        j -= cxy;
    }
    pix += step;
    let c = channels(src[pix]);
    for k in 0..4 {
        acc[k] = acc[k].wrapping_add(c[k].wrapping_mul(j as u32));
    }
    acc
}

/// `INTERPOLATE_PIXEL_256`.
#[inline]
fn interpolate_pixel_256(x: u32, a: u32, y: u32, b: u32) -> u32 {
    let cx = channels(x);
    let cy = channels(y);
    let mut o = [0u32; 4];
    for k in 0..4 {
        o[k] = ((cx[k] * a + cy[k] * b) >> 8) & 0xff;
    }
    q_rgba(o[0], o[1], o[2], o[3])
}

/// `interpolate_4_pixels_sse2`: vertical first, then horizontal.
#[inline]
fn interpolate_4_pixels(tl: u32, tr: u32, bl: u32, br: u32, distx: u32, disty: u32) -> u32 {
    let (ctl, ctr, cbl, cbr) = (channels(tl), channels(tr), channels(bl), channels(br));
    let mut o = [0u32; 4];
    for k in 0..4 {
        let l = ((ctl[k] * (256 - disty)) & 0xffff).wrapping_add((cbl[k] * disty) & 0xffff) & 0xffff;
        let l = l >> 8;
        let r = ((ctr[k] * (256 - disty)) & 0xffff).wrapping_add((cbr[k] * disty) & 0xffff) & 0xffff;
        let r = r >> 8;
        let v = ((l * (256 - distx) + r * distx) >> 8).min(255);
        o[k] = v;
    }
    q_rgba(o[0], o[1], o[2], o[3])
}

/// `qSmoothScaleImage` for a 32-bit source. `alpha` picks the ARGB kernels
/// (`hasAlphaChannel()`); without it the result is forced opaque, as the
/// RGB32 kernels do.
pub fn smooth_scale(src: &Image32, dw: usize, dh: usize, alpha: bool) -> Image32 {
    let (sw, sh) = (src.w as i32, src.h as i32);
    let (dw_i, dh_i) = (dw as i32, dh as i32);
    let mut out = Image32::new(dw, dh);
    if dw == 0 || dh == 0 || sw == 0 || sh == 0 {
        return out;
    }
    let xup_yup = (dw_i >= sw) as u8 + (((dh_i >= sh) as u8) << 1);
    let xpoints = calc_points(sw, dw_i);
    let ypoints = calc_points(sh, dh_i);
    let xapoints = calc_a_points(sw, dw_i, xup_yup & 1 != 0);
    let yapoints = calc_a_points(sh, dh_i, xup_yup & 2 != 0);
    let sow = src.w;
    let s = &src.px;
    let finish = |v: [u32; 4], shift: u32| -> u32 {
        let c = v.map(|c| (c >> shift).min(255));
        if alpha {
            q_rgba(c[0], c[1], c[2], c[3])
        } else {
            q_rgba(c[0], c[1], c[2], 0xff)
        }
    };

    for y in 0..dh {
        let row = ypoints[y] as usize * sow;
        for x in 0..dw {
            let sptr = row + xpoints[x] as usize;
            let px = match xup_yup {
                3 => {
                    let yap = yapoints[y] as u32;
                    let xap = xapoints[x] as u32;
                    if yap > 0 {
                        if xap > 0 {
                            interpolate_4_pixels(s[sptr], s[sptr + 1], s[sptr + sow], s[sptr + sow + 1], xap, yap)
                        } else {
                            interpolate_pixel_256(s[sptr], 256 - yap, s[sptr + sow], yap)
                        }
                    } else if xap > 0 {
                        interpolate_pixel_256(s[sptr], 256 - xap, s[sptr + 1], xap)
                    } else {
                        s[sptr]
                    }
                }
                1 => {
                    // Up in x, down in y.
                    let cy = yapoints[y] >> 16;
                    let yap = yapoints[y] & 0xffff;
                    let mut v = scale_helper(s, sptr, yap, cy, sow);
                    let xap = xapoints[x] as u32;
                    if xap > 0 {
                        let vr = scale_helper(s, sptr + 1, yap, cy, sow);
                        for k in 0..4 {
                            v[k] = v[k]
                                .wrapping_mul(256 - xap)
                                .wrapping_add(vr[k].wrapping_mul(xap))
                                >> 8;
                        }
                    }
                    finish(v, 14)
                }
                2 => {
                    // Down in x, up in y.
                    let cx = xapoints[x] >> 16;
                    let xap = xapoints[x] & 0xffff;
                    let mut v = scale_helper(s, sptr, xap, cx, 1);
                    let yap = yapoints[y] as u32;
                    if yap > 0 {
                        let vr = scale_helper(s, sptr + sow, xap, cx, 1);
                        for k in 0..4 {
                            v[k] = v[k]
                                .wrapping_mul(256 - yap)
                                .wrapping_add(vr[k].wrapping_mul(yap))
                                >> 8;
                        }
                    }
                    finish(v, 14)
                }
                _ => {
                    // Down both ways.
                    let cy = yapoints[y] >> 16;
                    let yap = yapoints[y] & 0xffff;
                    let cx = xapoints[x] >> 16;
                    let xap = xapoints[x] & 0xffff;
                    let mut p = sptr;
                    let v = scale_helper(s, p, xap, cx, 1);
                    let mut r = v.map(|c| (c >> 4).wrapping_mul(yap as u32));
                    let mut j = (1 << 14) - yap;
                    while j > cy {
                        p += sow;
                        let v = scale_helper(s, p, xap, cx, 1);
                        for k in 0..4 {
                            r[k] = r[k].wrapping_add((v[k] >> 4).wrapping_mul(cy as u32));
                        }
                        j -= cy;
                    }
                    p += sow;
                    let v = scale_helper(s, p, xap, cx, 1);
                    for k in 0..4 {
                        r[k] = r[k].wrapping_add((v[k] >> 4).wrapping_mul(j as u32));
                    }
                    finish(r, 24)
                }
            };
            out.px[y * dw + x] = if xup_yup == 3 && !alpha {
                px | 0xff00_0000
            } else {
                px
            };
        }
    }
    out
}

/// `QImage::scaled(size, aspect, Qt::SmoothTransformation)` for a 32-bit
/// image, down to the size it actually produces. `premultiplied` says
/// whether `src` is already `ARGB32_Premultiplied` (or opaque `RGB32`);
/// `Format_ARGB32` sources are premultiplied first, as `smoothScaled` does.
pub fn scaled_smooth(src: &Image32, w: usize, h: usize, alpha: bool) -> Image32 {
    let (w, h) = (w.max(1), h.max(1));
    if w == src.w && h == src.h {
        return src.clone();
    }
    // QImage::transformed: `int(|m11| * ws + 0.9999)`.
    let m11 = w as f64 / src.w as f64;
    let m22 = h as f64 / src.h as f64;
    let wd = (m11.abs() * src.w as f64 + 0.9999) as usize;
    let hd = (m22.abs() * src.h as f64 + 0.9999) as usize;
    if alpha {
        let pm = Image32 {
            w: src.w,
            h: src.h,
            px: src.px.iter().map(|&p| premultiply_argb32(p)).collect(),
        };
        smooth_scale(&pm, wd, hd, true)
    } else {
        smooth_scale(src, wd, hd, false)
    }
}

/// `qPremultiply`.
pub fn premultiply_argb32(p: u32) -> u32 {
    let a = q_alpha(p);
    if a == 255 {
        return p;
    }
    if a == 0 {
        return 0;
    }
    let mul = |c: u32| {
        let t = c * a;
        (t + ((t >> 8) & 0xff) + 0x80) >> 8
    };
    q_rgba(mul(q_red(p)), mul(q_green(p)), mul(q_blue(p)), a)
}

// ---------------------------------------------------------------------------
// QPainter::drawImage with SmoothPixmapTransform onto a Format_ARGB32 target
// ---------------------------------------------------------------------------

type Rgba64 = [u16; 4];

/// `QRgba64::fromArgb32(p).premultiplied()`.
fn argb32_to_rgba64_pm(p: u32) -> Rgba64 {
    let exp = |c: u32| (c | (c << 8)) as u64;
    let (r, g, b, a) = (exp(q_red(p)), exp(q_green(p)), exp(q_blue(p)), exp(q_alpha(p)));
    if a == 0xffff {
        return [r as u16, g as u16, b as u16, a as u16];
    }
    if a == 0 {
        return [0; 4];
    }
    let mul = |c: u64| {
        let t = c * a;
        ((t + ((t >> 16) & 0xffff) + 0x8000) >> 16) as u16
    };
    [mul(r), mul(g), mul(b), a as u16]
}

#[inline]
fn mulhi(a: u16, b: u16) -> u16 {
    ((a as u32 * b as u32) >> 16) as u16
}

/// `interpolate_4_pixels_rgb64`, SSE2 flavour.
#[inline]
fn interpolate_4_pixels_rgb64(t: [Rgba64; 2], b: [Rgba64; 2], distx: u32, disty: u32) -> Rgba64 {
    let mut v = t;
    if disty != 0 {
        let idy = (0x10000 - disty) as u16;
        let dy = disty as u16;
        for p in 0..2 {
            for k in 0..4 {
                v[p][k] = mulhi(v[p][k], idy).wrapping_add(mulhi(b[p][k], dy));
            }
        }
    }
    if distx != 0 {
        let idx = (0x10000 - distx) as u16;
        let dx = distx as u16;
        let mut o = [0u16; 4];
        for k in 0..4 {
            o[k] = mulhi(v[0][k], idx).wrapping_add(mulhi(v[1][k], dx));
        }
        return o;
    }
    v[0]
}

#[inline]
fn div_257(x: u32) -> u32 {
    let x = x + 128;
    (x - (x >> 8)) >> 8
}

/// `convertARGBFromRGBA64PM_sse4<false>`: a span of premultiplied 16-bit
/// pixels back to `Format_ARGB32`. It works four pixels at a time, and how
/// it un-premultiplies depends on the whole block, so a pixel's result
/// depends on where in its span it falls.
fn store_argb32_from_rgba64_pm(dst: &mut [u32], src: &[Rgba64]) {
    let count = src.len();
    let mut i = 0usize;
    while i + 3 < count {
        let block = &src[i..i + 4];
        let all_transparent = block.iter().all(|p| p[3] == 0);
        let all_opaque = block.iter().all(|p| p[3] == 0xffff);
        if all_transparent {
            dst[i..i + 4].fill(0);
        } else if all_opaque {
            for (k, p) in block.iter().enumerate() {
                let c = p.map(|c| div_257(c as u32).min(255));
                dst[i + k] = q_rgba(c[0], c[1], c[2], c[3]);
            }
        } else {
            for (k, p) in block.iter().enumerate() {
                let a = p[3] as u32;
                let alpha8 = div_257(a).min(255);
                let ia = reciprocal_mul(a as f32, 255.0);
                let ch = |c: u16| -> u32 {
                    let v = cvt_round(c as f32 * ia).clamp(0, 65535);
                    // packus_epi16 reads the 16-bit lane as signed.
                    let v = v as u16 as i16 as i32;
                    v.clamp(0, 255) as u32
                };
                dst[i + k] = if a == 0 {
                    q_rgba(0, 0, 0, alpha8)
                } else {
                    q_rgba(ch(p[0]), ch(p[1]), ch(p[2]), alpha8)
                };
            }
        }
        i += 4;
    }
    while i < count {
        dst[i] = rgba64_to_rgb32_sse4(src[i]);
        i += 1;
    }
}

/// `qConvertRgba64ToRgb32_sse4`.
fn rgba64_to_rgb32_sse4(p: Rgba64) -> u32 {
    let a = p[3];
    if a == 0 {
        return 0;
    }
    let mut c = p;
    if a != 0xffff {
        let via = reciprocal_mul(a as f32, 65535.0);
        for k in 0..3 {
            c[k] = cvt_round(p[k] as f32 * via).clamp(0, 65535) as u16;
        }
    }
    let o = c.map(|v| div_257(v as u32).min(255));
    q_rgba(o[0], o[1], o[2], o[3])
}

/// The inverse matrix and its 16.16 set-up, from `QSpanData::setupMatrix`.
struct TextureMatrix {
    m11: f64,
    m12: f64,
    m21: f64,
    m22: f64,
    dx: f64,
    dy: f64,
}

impl TextureMatrix {
    fn new(matrix: &QTransform) -> Self {
        let mut delta = QTransform::identity();
        // "make sure we round off correctly in qdrawhelper.cpp"
        delta.translate(1.0 / 65536.0, 1.0 / 65536.0);
        let inv = delta.mul(matrix).inverted();
        Self {
            m11: inv.m11,
            m12: inv.m12,
            m21: inv.m21,
            m22: inv.m22,
            dx: inv.dx,
            dy: inv.dy,
        }
    }
}

/// `fetchTransformedBilinear_pixelBounds<BlendTransformedBilinear>`.
#[inline]
fn pixel_bounds(v: i32, l2: i32) -> (i32, i32) {
    if v < 0 {
        (0, 0)
    } else if v >= l2 {
        (l2, l2)
    } else {
        (v, v + 1)
    }
}

/// One span through `blend_src_generic_rgb64`: fetch (bilinear, 16-bit),
/// SourceOver onto the zero-filled target, store back to ARGB32.
fn blend_span(dst: &mut Image32, src: &Image32, tm: &TextureMatrix, x: i32, y: i32, len: usize) {
    const FIXED: f64 = 65536.0;
    let cx = x as f64 + 0.5;
    let cy = y as f64 + 0.5;
    let fdx = (tm.m11 * FIXED) as i32;
    let fdy = (tm.m12 * FIXED) as i32;
    let mut fx = ((tm.m21 * cy + tm.m11 * cx + tm.dx) * FIXED) as i32;
    let mut fy = ((tm.m22 * cy + tm.m12 * cx + tm.dy) * FIXED) as i32;
    fx -= 1 << 15;
    fy -= 1 << 15;

    let (l2x, l2y) = (src.w as i32 - 1, src.h as i32 - 1);
    let fetch = |xx: i32, yy: i32| argb32_to_rgba64_pm(src.px[yy as usize * src.w + xx as usize]);
    let mut buf: Vec<Rgba64> = Vec::with_capacity(len);
    for _ in 0..len {
        let (x1, x2) = pixel_bounds(fx >> 16, l2x);
        let (y1, y2) = pixel_bounds(fy >> 16, l2y);
        let t = [fetch(x1, y1), fetch(x2, y1)];
        let b = [fetch(x1, y2), fetch(x2, y2)];
        let distx = (fx & 0xffff) as u32;
        let disty = (fy & 0xffff) as u32;
        let s = interpolate_4_pixels_rgb64(t, b, distx, disty);
        // SourceOver onto a pixel that is still zero leaves the source.
        buf.push(if s[3] == 0 { [0; 4] } else { s });
        fx = fx.wrapping_add(fdx);
        fy = fy.wrapping_add(fdy);
    }
    let row = y as usize * dst.w + x as usize;
    store_argb32_from_rgba64_pm(&mut dst.px[row..row + len], &buf);
}

#[inline]
fn q26dot6_compare(p1: f64, p2: f64) -> bool {
    ((p2 - p1) * 64.0) as i32 == 0
}

#[inline]
fn snap_26dot6(p: (f64, f64)) -> (f64, f64) {
    (
        (p.0 * 64.0).floor() * (1.0 / 64.0),
        (p.1 * 64.0).floor() * (1.0 / 64.0),
    )
}

#[inline]
fn safe_divide(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        if x > 0.0 {
            1e20
        } else {
            -1e20
        }
    } else {
        x / y
    }
}

#[inline]
fn safe_float_to_q16(x: f64) -> i32 {
    let tmp = x * 65536.0;
    if tmp > i32::MAX as f64 {
        i32::MAX
    } else if tmp < i32::MIN as f64 {
        -i32::MAX
    } else {
        tmp as i32
    }
}

/// `QRasterizer::rasterizeLine`, aliased, without square caps: the spans a
/// rotated image rectangle covers when `QPainter` draws it as a thick line
/// between the midpoints of its left and right edges.
fn rasterize_line(a: (f64, f64), b: (f64, f64), mut width: f64, dw: i32, dh: i32) -> Vec<(i32, i32, i32)> {
    let mut spans = Vec::new();
    if a == b || width.is_nan() || width <= 0.0 || dw <= 0 || dh <= 0 {
        return spans;
    }
    let (clip_l, clip_t, clip_r, clip_b) = (0i32, 0i32, dw - 1, dh - 1);
    let mut pa = a;
    let mut pb = b;

    let offs = ((b.1 - a.1).abs() * width * 0.5, (b.0 - a.0).abs() * width * 0.5);
    let cl = clip_l as f64 - offs.0;
    let ct = clip_t as f64 - offs.1;
    let cr = (clip_r + 1) as f64 + offs.0;
    let cb = (clip_b + 1) as f64 + offs.1;
    let contains = |p: (f64, f64)| {
        let (l, r) = if cr - cl < 0.0 { (cr, cl) } else { (cl, cr) };
        let (t, bb) = if cb - ct < 0.0 { (cb, ct) } else { (ct, cb) };
        !(l == r || p.0 < l || p.0 > r || t == bb || p.1 < t || p.1 > bb)
    };
    if !contains(pa) || !contains(pb) {
        let mut t1 = 0.0f64;
        let mut t2 = 1.0f64;
        let o = [pa.0, pa.1];
        let d = [pb.0 - pa.0, pb.1 - pa.1];
        let low = [cl, ct];
        let high = [cr, cb];
        for i in 0..2 {
            if d[i] == 0.0 {
                if o[i] <= low[i] || o[i] >= high[i] {
                    return spans;
                }
                continue;
            }
            let d_inv = 1.0 / d[i];
            let mut t_low = (low[i] - o[i]) * d_inv;
            let mut t_high = (high[i] - o[i]) * d_inv;
            if t_low > t_high {
                std::mem::swap(&mut t_low, &mut t_high);
            }
            if t1 < t_low {
                t1 = t_low;
            }
            if t2 > t_high {
                t2 = t_high;
            }
            if t1 >= t2 {
                return spans;
            }
        }
        let npa = (pa.0 + (pb.0 - pa.0) * t1, pa.1 + (pb.1 - pa.1) * t1);
        let npb = (pa.0 + (pb.0 - pa.0) * t2, pa.1 + (pb.1 - pa.1) * t2);
        pa = npa;
        pb = npb;
    }

    {
        let d0 = (a.0 - b.0, a.1 - b.1);
        let w0 = d0.0 * d0.0 + d0.1 * d0.1;
        let d = (pa.0 - pb.0, pa.1 - pb.1);
        let w = d.0 * d.0 + d.1 * d.1;
        if w == 0.0 {
            return spans;
        }
        width *= (w0 / w).sqrt();
    }

    let mut add_span = |x: i32, len: i32, y: i32| {
        if len > 0 {
            spans.push((x, len, y));
        }
    };

    if q26dot6_compare(pa.1, pb.1) {
        let x = (pa.0 + pb.0) * 0.5;
        let dx = (pb.0 - pa.0).abs() * 0.5;
        let y = pa.1;
        let dy = width * dx;
        pa = (x, y - dy);
        pb = (x, y + dy);
        width = 1.0 / width;
    }

    if q26dot6_compare(pa.0, pb.0) {
        if pa.1 > pb.1 {
            std::mem::swap(&mut pa, &mut pb);
        }
        let dy = pb.1 - pa.1;
        let half_width = 0.5 * width * dy;
        let clampx = |v: f64| v.clamp(clip_l as f64, (clip_r + 1) as f64);
        let clampy = |v: f64| v.clamp(clip_t as f64, (clip_b + 1) as f64);
        let left = clampx(pa.0 - half_width);
        let right = clampx(pa.0 + half_width);
        pa.1 = clampy(pa.1);
        pb.1 = clampy(pb.1);
        if q26dot6_compare(left, right) || q26dot6_compare(pa.1, pb.1) {
            return spans;
        }
        let i_top = (pa.1 + 0.5) as i32;
        let i_bottom = if pb.1 < 0.5 { -1 } else { (pb.1 - 0.5) as i32 };
        let i_left = (left + 0.5) as i32;
        let i_right = if right < 0.5 { -1 } else { (right - 0.5) as i32 };
        let i_width = i_right - i_left + 1;
        for y in i_top..=i_bottom {
            add_span(i_left, i_width, y);
        }
        return spans;
    }

    if pa.1 > pb.1 {
        std::mem::swap(&mut pa, &mut pb);
    }
    let k = 0.5 * width;
    let delta = ((pb.0 - pa.0) * k, (pb.1 - pa.1) * k);
    let perp = (delta.1, -delta.0);
    let add = |p: (f64, f64), q: (f64, f64)| (p.0 + q.0, p.1 + q.1);
    let sub = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0, p.1 - q.1);
    let (top, left, right, bottom) = if pa.0 < pb.0 {
        (add(pa, perp), sub(pa, perp), add(pb, perp), sub(pb, perp))
    } else {
        (sub(pa, perp), sub(pb, perp), add(pa, perp), add(pb, perp))
    };
    let top = snap_26dot6(top);
    let bottom = snap_26dot6(bottom);
    let left = snap_26dot6(left);
    let right = snap_26dot6(right);

    let tl_edge = sub(left, top);
    let tr_edge = sub(right, top);
    let bl_edge = sub(bottom, left);
    let br_edge = sub(bottom, right);
    let tl_slope = safe_divide(tl_edge.0, tl_edge.1);
    let bl_slope = safe_divide(bl_edge.0, bl_edge.1);
    let tr_slope = safe_divide(tr_edge.0, tr_edge.1);
    let br_slope = safe_divide(br_edge.0, br_edge.1);
    let tl_fp = safe_float_to_q16(tl_slope);
    let tr_fp = safe_float_to_q16(tr_slope);
    let bl_fp = safe_float_to_q16(bl_slope);
    let br_fp = safe_float_to_q16(br_slope);

    let i_top = (top.1 + 0.5) as i32;
    let mut i_left = if left.1 < 0.5 { -1 } else { (left.1 - 0.5) as i32 };
    let mut i_right = if right.1 < 0.5 { -1 } else { (right.1 - 0.5) as i32 };
    let mut i_bottom = if bottom.1 < 0.5 { -1 } else { (bottom.1 - 0.5) as i32 };
    let mut i_middle = i_left.min(i_right);

    // `iTop + 0.5f` is float arithmetic in Qt; exact for these magnitudes.
    let mut left_af = safe_float_to_q16(top.0 + 0.5 + ((i_top as f32 + 0.5) as f64 - top.1) * tl_slope);
    let mut left_bf = safe_float_to_q16(left.0 + 0.5 + ((i_left as f32 + 1.5) as f64 - left.1) * bl_slope);
    let mut right_af = safe_float_to_q16(top.0 - 0.5 + ((i_top as f32 + 0.5) as f64 - top.1) * tr_slope);
    let mut right_bf = safe_float_to_q16(right.0 - 0.5 + ((i_right as f32 + 1.5) as f64 - right.1) * br_slope);

    let mut y = i_top;
    let mut segment = |next: &mut i32, li: &mut i32, ri: &mut i32, ls: i32, rs: i32, y: &mut i32| {
        let ny = (*next + 1).min(clip_t);
        if *y < ny {
            *li = li.wrapping_add(ls.wrapping_mul(ny - *y));
            *ri = ri.wrapping_add(rs.wrapping_mul(ny - *y));
            *y = ny;
        }
        if *next > clip_b {
            *next = clip_b;
        }
        while *y <= *next {
            let x1 = (*li >> 16).max(clip_l);
            let x2 = (*ri >> 16).min(clip_r);
            if x2 >= x1 {
                add_span(x1, x2 - x1 + 1, *y);
            }
            *li = li.wrapping_add(ls);
            *ri = ri.wrapping_add(rs);
            *y += 1;
        }
    };
    segment(&mut i_middle, &mut left_af, &mut right_af, tl_fp, tr_fp, &mut y);
    segment(&mut i_right, &mut left_bf, &mut right_af, bl_fp, tr_fp, &mut y);
    segment(&mut i_left, &mut left_af, &mut right_bf, tl_fp, br_fp, &mut y);
    segment(&mut i_bottom, &mut left_bf, &mut right_bf, bl_fp, br_fp, &mut y);
    spans
}

/// `qt_scaleForTransform`'s answer: is the transform free of shear?
fn no_shear(m: &QTransform) -> bool {
    let ty = m.ty();
    if ty <= Tx::Translate {
        return true;
    }
    if ty == Tx::Scale {
        return fuzzy_compare(m.m11.abs(), m.m22.abs());
    }
    let x1 = m.m11 * m.m11 + m.m21 * m.m21;
    let y1 = m.m12 * m.m12 + m.m22 * m.m22;
    let x2 = m.m11 * m.m11 + m.m12 * m.m12;
    let y2 = m.m21 * m.m21 + m.m22 * m.m22;
    if (x1 - y1).abs() > (x2 - y2).abs() {
        ty == Tx::Rotate && fuzzy_compare(x1, y1)
    } else {
        ty == Tx::Rotate && fuzzy_compare(x2, y2)
    }
}

/// `QPainter p(&dst); p.setTransform(matrix);
/// p.setRenderHints(SmoothPixmapTransform); p.drawImage(QPointF(), src);`
/// onto a zero-filled `Format_ARGB32` target, `src` being `Format_ARGB32`.
///
/// Only the paths Krita's pyramid can reach are here: a matrix that at least
/// scales (Krita nudges pure translations off being one), no clip, no
/// antialiasing, SourceOver, full opacity.
pub fn draw_image(dst: &mut Image32, src: &Image32, matrix: &QTransform) {
    let ty = matrix.ty();
    assert!(ty > Tx::Translate, "Krita never draws a tip untransformed");
    let tm = TextureMatrix::new(matrix);
    let (dw, dh) = (dst.w as i32, dst.h as i32);
    let r = RectF {
        x: 0.0,
        y: 0.0,
        w: src.w as f64,
        h: src.h as f64,
    };

    if ty == Tx::Scale {
        // fillRect_normalized over the mapped rect.
        let rr = matrix.map_rect(r);
        let x1 = qround(rr.x);
        let y1 = qround(rr.y);
        let x2 = qround(rr.x + rr.w);
        let y2 = qround(rr.y + rr.h);
        let (x1, x2) = (x1.max(0), x2.min(dw));
        let (y1, y2) = (y1.max(0), y2.min(dh));
        if x2 <= x1 || y2 <= y1 {
            return;
        }
        for y in y1..y2 {
            blend_span(dst, src, &tm, x1, y, (x2 - x1) as usize);
        }
        return;
    }

    assert!(no_shear(matrix), "Krita's dab shapes here have no shear");
    // The rect's left and right edge midpoints, mapped.
    let a = matrix.map((r.x + r.x) * 0.5, (r.y + (r.y + r.h)) * 0.5);
    let b = matrix.map(((r.x + r.w) + (r.x + r.w)) * 0.5, (r.y + (r.y + r.h)) * 0.5);
    for (x, len, y) in rasterize_line(a, b, r.h / r.w, dw, dh) {
        blend_span(dst, src, &tm, x, y, len as usize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qround_rounds_halves_up() {
        assert_eq!(qround(0.5), 1);
        assert_eq!(qround(-0.5), 0);
        assert_eq!(qround(-1.5), -1);
        assert_eq!(qround(2.49), 2);
        assert_eq!(qround(-2.51), -3);
    }

    #[test]
    fn transform_type_is_classified_lazily_and_fuzzily() {
        let mut t = QTransform::identity();
        t.scale(1.0 + 1e-14, 1.0);
        assert_eq!(t.ty(), Tx::None, "a sub-picometre scale is no scale to Qt");
        t.rotate_radians(0.3);
        assert_eq!(t.ty(), Tx::Rotate);
    }

    #[test]
    fn inverse_round_trips() {
        let mut t = QTransform::identity();
        t.scale(0.7, 0.7);
        t = t.mul(&{
            let mut r = QTransform::identity();
            r.rotate_radians(1.1);
            r
        });
        t = t.mul(&QTransform::from_translate(3.25, -1.5));
        let inv = t.inverted();
        let (x, y) = t.map(10.0, 4.0);
        let (bx, by) = inv.map(x, y);
        assert!((bx - 10.0).abs() < 1e-9 && (by - 4.0).abs() < 1e-9);
    }

    /// Scaling a flat colour must give the same flat colour back, whichever
    /// of the four kernels runs.
    #[test]
    fn smooth_scale_preserves_a_flat_image() {
        let src = Image32 {
            w: 40,
            h: 30,
            px: vec![0xff80_4020; 1200],
        };
        for &(w, h) in &[(20, 15), (80, 60), (20, 60), (80, 15), (7, 3)] {
            let out = smooth_scale(&src, w, h, false);
            assert!(
                out.px.iter().all(|&p| p == 0xff80_4020),
                "{w}x{h}: {:08x?}",
                &out.px[..4]
            );
        }
    }

    #[test]
    fn smooth_downscale_averages() {
        // Alternating black and white columns halve to mid grey.
        let mut src = Image32::new(8, 2);
        for y in 0..2 {
            for x in 0..8 {
                src.px[y * 8 + x] = if x % 2 == 0 { 0xff00_0000 } else { 0xffff_ffff };
            }
        }
        let out = smooth_scale(&src, 4, 1, false);
        for &p in &out.px {
            let r = q_red(p);
            assert!((126..=128).contains(&r), "got {r}");
        }
    }

    #[test]
    fn unpremultiply_block_matches_exact_division_closely() {
        // An opaque-ish block goes through the reciprocal path; the result
        // must agree with plain division to within the last bit.
        let px: Vec<Rgba64> = (0..4).map(|i| {
            let a: u16 = 30000 + i * 5000;
            let c = (a as u32 * 3 / 5) as u16;
            [c, c, c, a]
        }).collect();
        let mut out = [0u32; 4];
        store_argb32_from_rgba64_pm(&mut out, &px);
        for &p in &out {
            let r = q_red(p) as i32;
            assert!((r - 153).abs() <= 1, "got {r}");
        }
    }

    /// An axis-aligned 2x upscale through the transformed path lands every
    /// source pixel where it should, and fills the whole target.
    #[test]
    fn draw_image_scales_onto_the_target() {
        let mut src = Image32::new(4, 4);
        src.px.fill(0xffff_ffff);
        let mut dst = Image32::new(8, 8);
        let m = QTransform::from_scale(2.0, 2.0);
        draw_image(&mut dst, &src, &m);
        // Interior pixels are opaque white; the outer ring blends towards
        // the clamped edge, which is also white.
        assert!(dst.px.iter().all(|&p| p == 0xffff_ffff), "{:08x?}", dst.px);
    }

    /// A rotated image covers a diamond of spans: none may fall outside
    /// the target, and the middle row must be the widest.
    #[test]
    fn rotated_spans_stay_inside_the_target() {
        let spans = rasterize_line((0.0, 10.0), (20.0, 10.5), 0.9, 21, 21);
        assert!(!spans.is_empty());
        for &(x, len, y) in &spans {
            assert!(x >= 0 && x + len <= 21 && (0..21).contains(&y));
        }
    }
}
