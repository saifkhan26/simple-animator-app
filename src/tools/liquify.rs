//! Liquify: pushing a drawing's pixels around with a brush, as Photoshop's
//! Forward Warp does.
//!
//! The pixels are never pushed directly. What the brush edits is a
//! displacement field — for each point, where in the *original* drawing to
//! look — and the drawing on screen is resampled from the original through
//! it. So however long the pen goes back and forth, every pixel is resampled
//! once from the drawing as the session found it, and lines don't soften
//! with each pass.
//!
//! The field lives on a grid every [`STEP`] pixels and is read between grid
//! points bilinearly: an 8K drawing's field is 16 MB instead of 265. A warp
//! brush is never as small as a few grid steps, so nothing is lost.

use crate::doc::canvas::{par_rows, DirtyRect};
use crate::tools::lasso::Mask;

/// Grid spacing of the field, in pixels.
pub const STEP: u32 = 4;
/// The smallest warp brush: two grid steps either side of its centre.
pub const MIN_RADIUS: f32 = 8.0;

/// What a liquify stroke does to the field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LiquifyMode {
    /// Forward warp: drag the pixels along with the pen.
    #[default]
    Push,
    /// Ease kinks out of the warp so it flows evenly.
    Smooth,
}

/// Where each point of a drawing reads from in the original: pixel `p`
/// shows the original at `p + D(p)`.
#[derive(Clone, Debug, PartialEq)]
pub struct LiquifyField {
    /// Cell pixel of grid point (0, 0).
    pub ox: i32,
    pub oy: i32,
    /// Grid points across and down.
    pub nw: usize,
    pub nh: usize,
    pub d: Vec<[f32; 2]>,
}

impl LiquifyField {
    /// A zero field covering the cell pixels of `r`.
    pub fn new(r: DirtyRect) -> Self {
        let nw = ((r.max_x - r.min_x) / STEP + 2) as usize;
        let nh = ((r.max_y - r.min_y) / STEP + 2) as usize;
        Self {
            ox: r.min_x as i32,
            oy: r.min_y as i32,
            nw,
            nh,
            d: vec![[0.0; 2]; nw * nh],
        }
    }

    /// Cell position of grid point `(i, j)`.
    pub fn point(&self, i: usize, j: usize) -> (f32, f32) {
        (
            (self.ox + (i as u32 * STEP) as i32) as f32,
            (self.oy + (j as u32 * STEP) as i32) as f32,
        )
    }

    /// The displacement at cell point `(x, y)`, between grid points. Zero off
    /// the grid, and exactly zero wherever the brush never reached.
    pub fn at(&self, x: f32, y: f32) -> [f32; 2] {
        let gx = (x - self.ox as f32) / STEP as f32;
        let gy = (y - self.oy as f32) / STEP as f32;
        let (mx, my) = ((self.nw - 1) as f32, (self.nh - 1) as f32);
        if !(0.0..=mx).contains(&gx) || !(0.0..=my).contains(&gy) {
            return [0.0; 2];
        }
        let i = (gx.floor() as usize).min(self.nw.saturating_sub(2));
        let j = (gy.floor() as usize).min(self.nh.saturating_sub(2));
        let (tx, ty) = (gx - i as f32, gy - j as f32);
        let at = |i: usize, j: usize| self.d[j * self.nw + i];
        let (a, b, c, e) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
        let mut out = [0.0; 2];
        for k in 0..2 {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bot = c[k] + (e[k] - c[k]) * tx;
            out[k] = top + (bot - top) * ty;
        }
        out
    }

    /// Grid points within `reach` pixels of `(cx, cy)`, as index ranges:
    /// everything a dab there can write.
    pub fn window(&self, cx: f32, cy: f32, reach: f32) -> Option<(usize, usize, usize, usize)> {
        let s = STEP as f32;
        let lo = |c: f32, o: i32| ((c - reach - o as f32) / s).ceil().max(0.0);
        let hi = |c: f32, o: i32, n: usize| ((c + reach - o as f32) / s).floor().min(n as f32 - 1.0);
        let (i0, i1) = (lo(cx, self.ox), hi(cx, self.ox, self.nw));
        let (j0, j1) = (lo(cy, self.oy), hi(cy, self.oy, self.nh));
        (i0 <= i1 && j0 <= j1).then_some((i0 as usize, i1 as usize, j0 as usize, j1 as usize))
    }

    /// A copy of the grid points in `(i0..=i1, j0..=j1)`, grown by `grow`
    /// points each way, as a field of its own — what a dab reads the old
    /// values from while it writes the new ones.
    fn snapshot(&self, (i0, i1, j0, j1): (usize, usize, usize, usize), grow: usize) -> LiquifyField {
        let (i0, j0) = (i0.saturating_sub(grow), j0.saturating_sub(grow));
        let (i1, j1) = ((i1 + grow).min(self.nw - 1), (j1 + grow).min(self.nh - 1));
        let (nw, nh) = (i1 - i0 + 1, j1 - j0 + 1);
        let mut d = Vec::with_capacity(nw * nh);
        for j in j0..=j1 {
            d.extend_from_slice(&self.d[j * self.nw + i0..=j * self.nw + i1]);
        }
        LiquifyField {
            ox: self.ox + (i0 as u32 * STEP) as i32,
            oy: self.oy + (j0 as u32 * STEP) as i32,
            nw,
            nh,
            d,
        }
    }

    /// The cell pixels a change to grid points `(i0..=i1, j0..=j1)` can
    /// reach: one grid step past them each way, where the interpolation
    /// still feels them.
    fn pixels_of(&self, (i0, i1, j0, j1): (usize, usize, usize, usize)) -> DirtyRect {
        let s = STEP as i32;
        let x0 = self.ox + (i0 as i32 - 1) * s;
        let y0 = self.oy + (j0 as i32 - 1) * s;
        DirtyRect {
            min_x: x0.max(0) as u32,
            min_y: y0.max(0) as u32,
            max_x: (self.ox + (i1 as i32 + 1) * s + 1).max(0) as u32,
            max_y: (self.oy + (j1 as i32 + 1) * s + 1).max(0) as u32,
        }
    }

    /// One push dab: the pixels under a brush of `radius` at `(cx, cy)` move
    /// by `v`, fully at the centre and fading to nothing at the rim, scaled
    /// by `strength` and each point's `edge` weight.
    ///
    /// Composed with the warp already there rather than added to it: the new
    /// image at `p` is the current image at `p - v·w`, which reads the old
    /// field there. Plain addition would shear wherever the field already
    /// varies, and two pushes would not add up to one twice as far.
    ///
    /// Returns the cell pixels that may have changed.
    pub fn push(&mut self, (cx, cy): (f32, f32), v: (f32, f32), radius: f32, strength: f32, edge: &[f32]) -> Option<DirtyRect> {
        let win = self.window(cx, cy, radius)?;
        let travel = (v.0.hypot(v.1) * strength.max(0.0) / STEP as f32).ceil() as usize + 1;
        let old = self.snapshot(win, travel);
        let (i0, i1, _, _) = win;
        let (ox, oy) = (self.ox, self.oy);
        self.each_row(win, |j, row| {
            let gy = (oy + (j as u32 * STEP) as i32) as f32;
            for i in i0..=i1 {
                let gx = (ox + (i as u32 * STEP) as i32) as f32;
                let w = strength * falloff(gx - cx, gy - cy, radius) * edge[j * row.len() + i];
                if w <= 0.0 {
                    continue;
                }
                let (bx, by) = (v.0 * w, v.1 * w);
                let back = old.at(gx - bx, gy - by);
                row[i] = [back[0] - bx, back[1] - by];
            }
        });
        Some(self.pixels_of(win))
    }

    /// Run `f` on each grid row `j` of the window `win`, with all of that
    /// row's points, on several threads when the window is big enough to
    /// be worth starting them: a 1000-pixel brush's dab is a quarter of a
    /// million points.
    fn each_row(&mut self, (i0, i1, j0, j1): (usize, usize, usize, usize), f: impl Fn(usize, &mut [[f32; 2]]) + Sync) {
        const PAR_POINTS: usize = 1 << 16;
        let nw = self.nw;
        let rows = &mut self.d[j0 * nw..(j1 + 1) * nw];
        let points = (i1 - i0 + 1) * (j1 - j0 + 1);
        let threads = if points < PAR_POINTS {
            1
        } else {
            std::thread::available_parallelism().map_or(1, |n| n.get().min(8))
        };
        if threads == 1 {
            for (r, row) in rows.chunks_exact_mut(nw).enumerate() {
                f(j0 + r, row);
            }
            return;
        }
        let per = (j1 - j0 + 1).div_ceil(threads);
        std::thread::scope(|s| {
            for (k, band) in rows.chunks_mut(per * nw).enumerate() {
                let f = &f;
                s.spawn(move || {
                    for (r, row) in band.chunks_exact_mut(nw).enumerate() {
                        f(j0 + k * per + r, row);
                    }
                });
            }
        });
    }

    /// One smoothing dab: each point under the brush eases toward the
    /// average of its four neighbours, so a kinked warp flows evenly.
    pub fn smooth(&mut self, (cx, cy): (f32, f32), radius: f32, strength: f32, edge: &[f32]) -> Option<DirtyRect> {
        let win = self.window(cx, cy, radius)?;
        let old = self.snapshot(win, 1);
        let (i0, i1, _, _) = win;
        let (ox, oy) = (self.ox, self.oy);
        self.each_row(win, |j, row| {
            let gy = (oy + (j as u32 * STEP) as i32) as f32;
            for i in i0..=i1 {
                let gx = (ox + (i as u32 * STEP) as i32) as f32;
                let w = strength * falloff(gx - cx, gy - cy, radius) * edge[j * row.len() + i];
                if w <= 0.0 {
                    continue;
                }
                let s = STEP as f32;
                let mut avg = [0.0; 2];
                for (dx, dy) in [(-s, 0.0), (s, 0.0), (0.0, -s), (0.0, s)] {
                    let n = old.at(gx + dx, gy + dy);
                    avg[0] += n[0] * 0.25;
                    avg[1] += n[1] * 0.25;
                }
                let cur = row[i];
                let t = 0.5 * w;
                row[i] = [cur[0] + (avg[0] - cur[0]) * t, cur[1] + (avg[1] - cur[1]) * t];
            }
        });
        Some(self.pixels_of(win))
    }
}

/// The brush's weight at offset `(dx, dy)` from its centre: 1 there, falling
/// smoothly to 0 at the rim with no step in its slope.
fn falloff(dx: f32, dy: f32, radius: f32) -> f32 {
    let t = (dx * dx + dy * dy) / (radius * radius);
    if t >= 1.0 {
        0.0
    } else {
        (1.0 - t) * (1.0 - t)
    }
}

/// How much each grid point of `field` may move: 1 with no selection;
/// inside one, its coverage, eased to nothing over `width` pixels in from
/// the selection's edge so the warp never tears at it.
pub fn edge_weights(field: &LiquifyField, clip: Option<&Mask>, width: f32) -> Vec<f32> {
    let n = field.nw * field.nh;
    let Some(mask) = clip else {
        return vec![1.0; n];
    };
    let cov: Vec<f32> = (0..n)
        .map(|k| {
            let (x, y) = field.point(k % field.nw, k / field.nw);
            if x < 0.0 || y < 0.0 {
                0.0
            } else {
                mask.at(x as u32, y as u32) as f32 / 255.0
            }
        })
        .collect();
    if width <= 0.0 {
        return cov;
    }
    // Distance to the nearest point outside, on the grid: two chamfer passes.
    let s = STEP as f32;
    let diag = s * std::f32::consts::SQRT_2;
    let mut dist: Vec<f32> = cov.iter().map(|&c| if c < 0.5 { 0.0 } else { f32::INFINITY }).collect();
    let (w, h) = (field.nw, field.nh);
    for j in 0..h {
        for i in 0..w {
            let k = j * w + i;
            let mut v = dist[k];
            if i > 0 {
                v = v.min(dist[k - 1] + s);
            }
            if j > 0 {
                v = v.min(dist[k - w] + s);
                if i > 0 {
                    v = v.min(dist[k - w - 1] + diag);
                }
                if i + 1 < w {
                    v = v.min(dist[k - w + 1] + diag);
                }
            }
            dist[k] = v;
        }
    }
    for j in (0..h).rev() {
        for i in (0..w).rev() {
            let k = j * w + i;
            let mut v = dist[k];
            if i + 1 < w {
                v = v.min(dist[k + 1] + s);
            }
            if j + 1 < h {
                v = v.min(dist[k + w] + s);
                if i + 1 < w {
                    v = v.min(dist[k + w + 1] + diag);
                }
                if i > 0 {
                    v = v.min(dist[k + w - 1] + diag);
                }
            }
            dist[k] = v;
        }
    }
    cov.iter()
        .zip(&dist)
        .map(|(&c, &d)| {
            let t = (d / width).clamp(0.0, 1.0);
            c * t * t * (3.0 - 2.0 * t)
        })
        .collect()
}

/// Write `rect` of a `cw`×`ch` cell's pixels, `out`, from `orig` — the same
/// cell as the warp found it — through `src`, which says where in `orig`
/// each pixel centre reads from, or `None` to keep the original bytes.
///
/// Sampled bilinearly on premultiplied colour, so a stroke's soft edge
/// doesn't drag a dark fringe along. Off the cell reads as clear. Under a
/// partial selection the result is blended back toward the original by the
/// coverage, and with `alpha_lock` only the colour moves.
///
/// Generic in `src`, not a `dyn`: it is called for every pixel, and inlined
/// it costs a fraction of an indirect call.
#[allow(clippy::too_many_arguments)]
pub fn resample<F>(
    orig: &[u8],
    cw: u32,
    ch: u32,
    rect: DirtyRect,
    out: &mut [u8],
    clip: Option<&Mask>,
    alpha_lock: bool,
    src: &F,
) where
    F: Fn(f32, f32) -> Option<(f32, f32)> + Sync,
{
    let rect = DirtyRect {
        min_x: rect.min_x.min(cw),
        min_y: rect.min_y.min(ch),
        max_x: rect.max_x.min(cw),
        max_y: rect.max_y.min(ch),
    };
    if rect.max_x <= rect.min_x || rect.max_y <= rect.min_y {
        return;
    }
    let w4 = cw as usize * 4;
    let rows = &mut out[rect.min_y as usize * w4..rect.max_y as usize * w4];
    par_rows(rows, w4, |first, band| {
        for (y, row) in (rect.min_y + first as u32..).zip(band.chunks_exact_mut(w4)) {
            for x in rect.min_x..rect.max_x {
                let to = src(x as f32 + 0.5, y as f32 + 0.5);
                put(orig, cw, ch, row, x, y, clip, alpha_lock, to);
            }
        }
    });
}

/// [`resample`] through `field`, the warp in hand on its own drawing: the
/// same pixels for a fraction of the work. Each row interpolates the grid
/// down to itself once, a pair of values per grid column, and each pixel
/// then only blends the two either side of it — not four grid points for
/// every pixel.
///
/// With `dabs`, the centres of the dabs since the last resample and how far
/// from them the field can have changed, only pixels within that reach of
/// one are redone: a round brush's dabs leave the corners of their square
/// as they were.
#[allow(clippy::too_many_arguments)]
pub fn resample_field(
    orig: &[u8],
    cw: u32,
    ch: u32,
    rect: DirtyRect,
    out: &mut [u8],
    clip: Option<&Mask>,
    alpha_lock: bool,
    field: &LiquifyField,
    dabs: Option<(&[(f32, f32)], f32)>,
) {
    let rect = DirtyRect {
        min_x: rect.min_x.min(cw),
        min_y: rect.min_y.min(ch),
        max_x: rect.max_x.min(cw),
        max_y: rect.max_y.min(ch),
    };
    if rect.max_x <= rect.min_x || rect.max_y <= rect.min_y || field.nw < 2 || field.nh < 2 {
        return;
    }
    let s = STEP as f32;
    let (mx, my) = ((field.nw - 1) as f32, (field.nh - 1) as f32);
    let w4 = cw as usize * 4;
    let rows = &mut out[rect.min_y as usize * w4..rect.max_y as usize * w4];
    par_rows(rows, w4, |first, band| {
        let mut line = vec![[0.0f32; 2]; field.nw];
        for (y, row) in (rect.min_y + first as u32..).zip(band.chunks_exact_mut(w4)) {
            let py = y as f32 + 0.5;
            let gy = (py - field.oy as f32) / s;
            let on_grid = (0.0..=my).contains(&gy);
            if on_grid {
                let j = (gy.floor() as usize).min(field.nh - 2);
                let ty = gy - j as f32;
                let (top, bot) = (&field.d[j * field.nw..(j + 1) * field.nw], &field.d[(j + 1) * field.nw..(j + 2) * field.nw]);
                for ((l, a), b) in line.iter_mut().zip(top).zip(bot) {
                    *l = [a[0] + (b[0] - a[0]) * ty, a[1] + (b[1] - a[1]) * ty];
                }
            }
            // The span of this row the dabs reached.
            let (x0, x1) = match dabs {
                None => (rect.min_x, rect.max_x),
                Some((centres, reach)) => {
                    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
                    for &(cx, cy) in centres {
                        let dy = py - cy;
                        if dy.abs() <= reach {
                            let half = (reach * reach - dy * dy).sqrt();
                            lo = lo.min(cx - half);
                            hi = hi.max(cx + half);
                        }
                    }
                    if hi < lo {
                        continue;
                    }
                    let lo = (lo.floor().max(0.0) as u32).clamp(rect.min_x, rect.max_x);
                    let hi = ((hi.ceil() + 1.0).max(0.0) as u32).clamp(rect.min_x, rect.max_x);
                    (lo, hi)
                }
            };
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                let gx = (px - field.ox as f32) / s;
                let to = if on_grid && (0.0..=mx).contains(&gx) {
                    let i = (gx.floor() as usize).min(field.nw - 2);
                    let tx = gx - i as f32;
                    let (a, b) = (line[i], line[i + 1]);
                    let d = [a[0] + (b[0] - a[0]) * tx, a[1] + (b[1] - a[1]) * tx];
                    (d != [0.0, 0.0]).then_some((px + d[0], py + d[1]))
                } else {
                    None
                };
                put(orig, cw, ch, row, x, y, clip, alpha_lock, to);
            }
        }
    });
}

/// Pixel `x` of `row` (cell row `y`) as a resample writes it: the original
/// where nothing moves it, else the original read at `to`, held to the
/// selection's coverage and an alpha lock.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn put(orig: &[u8], cw: u32, ch: u32, row: &mut [u8], x: u32, y: u32, clip: Option<&Mask>, alpha_lock: bool, to: Option<(f32, f32)>) {
    let i = x as usize * 4;
    let w4 = cw as usize * 4;
    let o = &orig[y as usize * w4 + i..y as usize * w4 + i + 4];
    let cov = clip.map_or(255, |m| m.at(x, y));
    let warped = if cov == 0 {
        None
    } else {
        to.map(|(sx, sy)| sample(orig, cw, ch, sx - 0.5, sy - 0.5))
    };
    let Some(mut px) = warped else {
        row[i..i + 4].copy_from_slice(o);
        return;
    };
    if alpha_lock {
        px = if px[3] == 0 { [o[0], o[1], o[2], o[3]] } else { [px[0], px[1], px[2], o[3]] };
    }
    if cov < 255 {
        let t = cov as f32 / 255.0;
        for c in 0..4 {
            px[c] = (o[c] as f32 + (px[c] as f32 - o[c] as f32) * t).round() as u8;
        }
    }
    row[i..i + 4].copy_from_slice(&px);
}

/// Bilinear RGBA at `(x, y)` in pixel space, premultiplied for the blend and
/// straight again after. Off the cell is clear.
///
/// Most of a line drawing is clear, and most of a fill is one colour: where
/// the four pixels read are all clear, or all the same, that is the answer
/// without the arithmetic.
fn sample(px: &[u8], w: u32, h: u32, x: f32, y: f32) -> [u8; 4] {
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (x - x0, y - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    if x0 >= 0 && y0 >= 0 && x0 + 1 < w as i64 && y0 + 1 < h as i64 {
        // The two pixels on each row as one fixed-size load: no per-byte
        // bounds checks in the hot path below.
        let i = (y0 as usize * w as usize + x0 as usize) * 4;
        let j = i + w as usize * 4;
        let (Ok(top), Ok(bot)) = (<&[u8; 8]>::try_from(&px[i..i + 8]), <&[u8; 8]>::try_from(&px[j..j + 8])) else {
            return [0; 4];
        };
        let quad = |r: &[u8; 8], k: usize| u32::from_ne_bytes([r[k], r[k + 1], r[k + 2], r[k + 3]]);
        let (a, b, c, d) = (quad(top, 0), quad(top, 4), quad(bot, 0), quad(bot, 4));
        if a == b && a == c && a == d {
            return if top[3] == 0 { [0; 4] } else { a.to_ne_bytes() };
        }
        if (top[3] | top[7] | bot[3] | bot[7]) == 0 {
            return [0; 4];
        }
        // Inside a line or a fill, all four opaque: premultiplying by 255
        // and dividing it out again changes nothing, so blend straight.
        if (top[3] & top[7] & bot[3] & bot[7]) == 255 {
            let mut m = [255u8; 4];
            for k in 0..3 {
                let t = top[k] as f32 + (top[4 + k] as f32 - top[k] as f32) * tx;
                let b = bot[k] as f32 + (bot[4 + k] as f32 - bot[k] as f32) * tx;
                m[k] = (t + (b - t) * ty).round().clamp(0.0, 255.0) as u8;
            }
            return m;
        }
    }
    let tap = |xi: i64, yi: i64| -> [f32; 4] {
        if xi < 0 || yi < 0 || xi >= w as i64 || yi >= h as i64 {
            return [0.0; 4];
        }
        let i = (yi as usize * w as usize + xi as usize) * 4;
        let a = px[i + 3] as f32;
        [px[i] as f32 * a, px[i + 1] as f32 * a, px[i + 2] as f32 * a, a]
    };
    let (a, b, c, d) = (tap(x0, y0), tap(x0 + 1, y0), tap(x0, y0 + 1), tap(x0 + 1, y0 + 1));
    let mut m = [0.0f32; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * tx;
        let bot = c[k] + (d[k] - c[k]) * tx;
        m[k] = top + (bot - top) * ty;
    }
    if m[3] <= 0.0 {
        return [0; 4];
    }
    let un = |v: f32| (v / m[3]).round().clamp(0.0, 255.0) as u8;
    [un(m[0]), un(m[1]), un(m[2]), m[3].round().clamp(0.0, 255.0) as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 64;
    const H: u32 = 48;

    fn whole() -> DirtyRect {
        DirtyRect {
            min_x: 0,
            min_y: 0,
            max_x: W,
            max_y: H,
        }
    }

    /// A clear `W`×`H` cell with an opaque 4×4 dot at `(x, y)`.
    fn dot(x: u32, y: u32) -> Vec<u8> {
        let mut px = vec![0u8; (W * H * 4) as usize];
        for yy in y..y + 4 {
            for xx in x..x + 4 {
                let i = ((yy * W + xx) * 4) as usize;
                px[i..i + 4].copy_from_slice(&[200, 30, 10, 255]);
            }
        }
        px
    }

    fn through(field: &LiquifyField) -> impl Fn(f32, f32) -> Option<(f32, f32)> + Sync + '_ {
        move |x, y| {
            let d = field.at(x, y);
            (d != [0.0, 0.0]).then_some((x + d[0], y + d[1]))
        }
    }

    fn warp(orig: &[u8], field: &LiquifyField, clip: Option<&Mask>) -> Vec<u8> {
        let mut out = orig.to_vec();
        resample(orig, W, H, whole(), &mut out, clip, false, &through(field));
        out
    }

    /// Centroid of alpha, x.
    fn centre_x(px: &[u8]) -> f32 {
        let (mut sum, mut n) = (0.0, 0.0);
        for (i, p) in px.chunks_exact(4).enumerate() {
            let a = p[3] as f32;
            sum += (i as u32 % W) as f32 * a;
            n += a;
        }
        sum / n
    }

    #[test]
    fn a_zero_field_is_identity() {
        let orig = dot(20, 20);
        let field = LiquifyField::new(whole());
        assert_eq!(warp(&orig, &field, None), orig);
    }

    #[test]
    fn a_push_moves_a_dot_along_the_push() {
        let orig = dot(20, 20);
        let mut field = LiquifyField::new(whole());
        let edge = edge_weights(&field, None, 0.0);
        field.push((22.0, 22.0), (6.0, 0.0), 20.0, 1.0, &edge);
        let moved = centre_x(&warp(&orig, &field, None)) - centre_x(&orig);
        assert!((moved - 6.0).abs() < 1.0, "moved {moved}");
    }

    /// Two pushes of 3 go as far as one of 6 — the warp composes.
    #[test]
    fn two_pushes_compose_like_one_twice_as_far() {
        let orig = dot(20, 20);
        let mut once = LiquifyField::new(whole());
        let edge = edge_weights(&once, None, 0.0);
        once.push((22.0, 22.0), (6.0, 0.0), 24.0, 1.0, &edge);
        let mut twice = LiquifyField::new(whole());
        twice.push((22.0, 22.0), (3.0, 0.0), 24.0, 1.0, &edge);
        twice.push((25.0, 22.0), (3.0, 0.0), 24.0, 1.0, &edge);
        let a = centre_x(&warp(&orig, &once, None));
        let b = centre_x(&warp(&orig, &twice, None));
        assert!((a - b).abs() < 0.75, "{a} vs {b}");
    }

    #[test]
    fn pixels_outside_the_selection_are_untouched() {
        let orig = dot(20, 20);
        // Selected: the left half only.
        let mask = Mask {
            x: 0,
            y: 0,
            w: W / 2,
            h: H,
            cov: vec![255; (W / 2 * H) as usize],
        };
        let mut field = LiquifyField::new(whole());
        let edge = edge_weights(&field, Some(&mask), 6.0);
        field.push((30.0, 22.0), (10.0, 0.0), 24.0, 1.0, &edge);
        let out = warp(&orig, &field, Some(&mask));
        for y in 0..H {
            for x in W / 2..W {
                let i = ((y * W + x) * 4) as usize;
                assert_eq!(out[i..i + 4], orig[i..i + 4], "({x}, {y})");
            }
        }
        assert_ne!(out, orig, "something inside moved");
    }

    /// The weight eases in from the edge: no jump between neighbours that a
    /// push could tear along.
    #[test]
    fn the_edge_falloff_is_continuous() {
        let mask = Mask {
            x: 8,
            y: 8,
            w: 48,
            h: 32,
            cov: vec![255; 48 * 32],
        };
        let field = LiquifyField::new(whole());
        let edge = edge_weights(&field, Some(&mask), 12.0);
        for j in 0..field.nh {
            for i in 1..field.nw {
                let (a, b) = (edge[j * field.nw + i - 1], edge[j * field.nw + i]);
                assert!((a - b).abs() <= 0.6, "jump {a} -> {b} at ({i}, {j})");
            }
        }
        let (cx, cy) = (8 + 24, 8 + 16);
        let k = (cy as usize / STEP as usize) * field.nw + cx as usize / STEP as usize;
        assert_eq!(edge[k], 1.0, "full strength in the middle");
        assert_eq!(edge[0], 0.0, "none outside");
    }

    #[test]
    fn smooth_reduces_field_variance() {
        let mut field = LiquifyField::new(whole());
        let edge = edge_weights(&field, None, 0.0);
        // A jagged field: alternate points pushed opposite ways.
        for (k, d) in field.d.iter_mut().enumerate() {
            *d = if k % 2 == 0 { [2.0, 0.0] } else { [-2.0, 0.0] };
        }
        let var = |f: &LiquifyField| f.d.iter().map(|d| d[0] * d[0]).sum::<f32>();
        let before = var(&field);
        for _ in 0..4 {
            field.smooth((32.0, 24.0), 40.0, 1.0, &edge);
        }
        assert!(var(&field) < before * 0.7, "{} -> {}", before, var(&field));
    }

    #[test]
    fn pressure_scales_strength() {
        let orig = dot(20, 20);
        let edge = edge_weights(&LiquifyField::new(whole()), None, 0.0);
        let moved = |strength| {
            let mut f = LiquifyField::new(whole());
            f.push((22.0, 22.0), (8.0, 0.0), 24.0, strength, &edge);
            centre_x(&warp(&orig, &f, None)) - centre_x(&orig)
        };
        let (full, half) = (moved(1.0), moved(0.5));
        assert!((half / full - 0.5).abs() < 0.15, "{full} vs {half}");
    }

    #[test]
    fn alpha_lock_keeps_alpha() {
        let orig = dot(20, 20);
        let mut field = LiquifyField::new(whole());
        let edge = edge_weights(&field, None, 0.0);
        field.push((22.0, 22.0), (6.0, 0.0), 20.0, 1.0, &edge);
        let mut out = orig.clone();
        resample(&orig, W, H, whole(), &mut out, None, true, &through(&field));
        for (a, b) in out.chunks_exact(4).zip(orig.chunks_exact(4)) {
            assert_eq!(a[3], b[3]);
        }
    }

    /// Back and forth ten times reads the original once each time, so it is
    /// no softer than going once.
    #[test]
    fn back_and_forth_never_compounds_blur() {
        let orig = dot(20, 20);
        let edge = edge_weights(&LiquifyField::new(whole()), None, 0.0);
        let soft = |px: &[u8]| px.chunks_exact(4).filter(|p| p[3] > 0 && p[3] < 255).count();
        let mut f = LiquifyField::new(whole());
        f.push((22.0, 22.0), (2.5, 0.0), 24.0, 1.0, &edge);
        let once = soft(&warp(&orig, &f, None));
        for _ in 0..10 {
            f.push((24.5, 22.0), (-2.5, 0.0), 24.0, 1.0, &edge);
            f.push((22.0, 22.0), (2.5, 0.0), 24.0, 1.0, &edge);
        }
        let many = soft(&warp(&orig, &f, None));
        assert!(many <= once + 8, "{once} soft pixels after one pass, {many} after ten");
    }

    /// Timings for one 400 px dab on a 4K drawing: the field update and the
    /// resample it causes. `cargo test --release perf_400px -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn perf_400px_dab_on_4k() {
        let (w, h) = (3840u32, 2160u32);
        let orig = vec![128u8; (w * h * 4) as usize];
        let mut out = orig.clone();
        let r = DirtyRect {
            min_x: 0,
            min_y: 0,
            max_x: w,
            max_y: h,
        };
        let mut f = LiquifyField::new(r);
        let edge = edge_weights(&f, None, 0.0);
        let t = std::time::Instant::now();
        let rect = f.push((1900.0, 1000.0), (20.0, 5.0), 400.0, 1.0, &edge).unwrap();
        let field_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = std::time::Instant::now();
        resample_field(&orig, w, h, rect, &mut out, None, false, &f, None);
        let resample_ms = t.elapsed().as_secs_f64() * 1000.0;
        println!("field {field_ms:.2} ms, resample {resample_ms:.2} ms");
    }
}

