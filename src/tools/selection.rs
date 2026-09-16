//! Floating pixel selection — the movable half of the lasso.
//!
//! A selection is lifted out of one cell: its pixels are copied through the
//! lasso's coverage mask, and the source is erased through the same mask, so
//! the two halves still sum to the original drawing. Until it is committed it
//! floats under a [`Pose`] — displacement, per-axis scale and rotation — drawn
//! as a textured quad rather than written back into the cell.
//!
//! `pixels` is written once, at the lift, and is **never rewritten by a
//! transform**. The pose is metadata over that one pristine buffer: on screen
//! the GPU samples it through `image_quad`, and the single CPU resample happens
//! at commit. So dragging a handle for ten seconds costs one resample rather
//! than one per frame, and adjusting a selection twice is no softer than
//! adjusting it once.
//!
//! A pose that is still *pixel-aligned* — no rotation, unit scale, whole-pixel
//! offset — takes the original integer blit, which is byte-exact. That is what
//! keeps a plain move lossless.

use crate::doc::canvas::Canvas;
use crate::doc::layer::CellId;
use crate::tools::lasso::Mask;

/// Half-size of a transform handle, in *screen* pixels. Shared with the overlay
/// so the square that gets drawn is the square that can be grabbed.
pub const HANDLE_PX: f32 = 5.0;

/// Scale limits for a handle drag. Zero has no inverse and would leave a box
/// with no grabbable handles; the ceiling keeps a runaway drag from asking for
/// a gigapixel resample.
pub const MIN_SCALE: f32 = 0.02;
pub const MAX_SCALE: f32 = 64.0;

/// Which part of the transform box a press landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grab {
    /// Inside the covered pixels — the drag moves the selection.
    Move,
    /// One of the eight box handles. 0..=3 are the corners TL, TR, BR, BL;
    /// 4..=7 are the edge midpoints T, R, B, L.
    Scale(u8),
    /// The ring just outside a corner.
    Rotate,
}

impl Grab {
    /// Where handle `id` sits in the lifted buffer's own pixel space.
    pub fn handle_pos(id: u8, w: f32, h: f32) -> (f32, f32) {
        match id {
            0 => (0.0, 0.0),
            1 => (w, 0.0),
            2 => (w, h),
            3 => (0.0, h),
            4 => (w * 0.5, 0.0),
            5 => (w, h * 0.5),
            6 => (w * 0.5, h),
            _ => (0.0, h * 0.5),
        }
    }

    /// All eight handle positions in buffer space, in id order — what the
    /// overlay walks to draw them.
    pub fn handle_positions(w: f32, h: f32) -> [(f32, f32); 8] {
        let mut out = [(0.0, 0.0); 8];
        for (i, p) in out.iter_mut().enumerate() {
            *p = Self::handle_pos(i as u8, w, h);
        }
        out
    }

    /// The point that must stay put while this handle is dragged, in buffer
    /// space, plus which axes the drag drives. `None` for non-scale grabs.
    pub fn anchor(self, w: f32, h: f32) -> Option<((f32, f32), (bool, bool))> {
        let id = match self {
            Grab::Scale(i) => i,
            _ => return None,
        };
        Some(match id {
            0 => ((w, h), (true, true)),
            1 => ((0.0, h), (true, true)),
            2 => ((0.0, 0.0), (true, true)),
            3 => ((w, 0.0), (true, true)),
            4 => ((w * 0.5, h), (false, true)),
            5 => ((0.0, h * 0.5), (true, false)),
            6 => ((w * 0.5, 0.0), (false, true)),
            _ => ((w, h * 0.5), (true, false)),
        })
    }
}

/// Where the lifted buffer sits relative to where it was cut from: scale and
/// rotate about the buffer's centre, then displace.
///
/// Kept separate from [`Selection`] so a drag can be solved against the pose
/// recorded when the handle was grabbed, without cloning the pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// Displacement from where the pixels were lifted, in cell pixels.
    /// Fractional only once a scale or rotation has anchored the box somewhere
    /// other than its centre — a plain drag or an arrow nudge still moves by
    /// whole pixels.
    pub offset: (f32, f32),
    /// Per-axis scale about the box centre. `(1.0, 1.0)` is native size.
    pub scale: (f32, f32),
    /// Rotation about the box centre, in radians.
    pub rot: f32,
}

impl Default for Pose {
    fn default() -> Self {
        Self {
            offset: (0.0, 0.0),
            scale: (1.0, 1.0),
            rot: 0.0,
        }
    }
}

impl Pose {
    /// True while the pose is an exact whole-pixel translation — the case the
    /// original integer blit handles losslessly.
    pub fn is_pixel_aligned(&self) -> bool {
        self.rot == 0.0
            && self.scale.0 == 1.0
            && self.scale.1 == 1.0
            && self.offset.0.fract() == 0.0
            && self.offset.1.fract() == 0.0
    }

    /// Map a point in the lifted buffer's pixel space (`0..m.w`, `0..m.h`) into
    /// cell space. Same shape as [`crate::doc::transform::Transform::cell_to_doc`],
    /// with a per-axis scale instead of a uniform one.
    pub fn buf_to_cell(&self, m: &Mask, u: f32, v: f32) -> (f32, f32) {
        let (cu, cv) = (m.w as f32 * 0.5, m.h as f32 * 0.5);
        let lx = (u - cu) * self.scale.0;
        let ly = (v - cv) * self.scale.1;
        let (s, c) = self.rot.sin_cos();
        (
            m.x as f32 + cu + self.offset.0 + lx * c - ly * s,
            m.y as f32 + cv + self.offset.1 + lx * s + ly * c,
        )
    }

    /// Exact inverse of [`Pose::buf_to_cell`].
    pub fn cell_to_buf(&self, m: &Mask, x: f32, y: f32) -> (f32, f32) {
        let (cu, cv) = (m.w as f32 * 0.5, m.h as f32 * 0.5);
        let dx = x - m.x as f32 - cu - self.offset.0;
        let dy = y - m.y as f32 - cv - self.offset.1;
        let (s, c) = self.rot.sin_cos();
        // Rotate by -rot.
        let rx = dx * c + dy * s;
        let ry = -dx * s + dy * c;
        let nz = |v: f32| if v.abs() < 1e-9 { 1e-9 } else { v };
        (rx / nz(self.scale.0) + cu, ry / nz(self.scale.1) + cv)
    }

    /// The transformed mask rect in cell space, TL / TR / BR / BL.
    pub fn corners(&self, m: &Mask) -> [(f32, f32); 4] {
        let (w, h) = (m.w as f32, m.h as f32);
        [
            self.buf_to_cell(m, 0.0, 0.0),
            self.buf_to_cell(m, w, 0.0),
            self.buf_to_cell(m, w, h),
            self.buf_to_cell(m, 0.0, h),
        ]
    }

    /// The pose this one becomes when `grab` is dragged from `start` to `now`,
    /// both in cell space. `uniform` is the Shift constraint: it locks a corner
    /// scale to one ratio, and snaps a rotation to 15°.
    ///
    /// Always solved against `self`, which callers hold fixed at the pose
    /// recorded when the handle was pressed. A drag therefore depends only on
    /// where the pointer is now, not on the path it took to get there.
    ///
    /// [`Grab::Move`] is not handled here — the pointer path owns it, so that a
    /// move keeps accumulating in whole pixels and stays lossless.
    pub fn dragged(
        &self,
        m: &Mask,
        grab: Grab,
        start: (f32, f32),
        now: (f32, f32),
        uniform: bool,
    ) -> Pose {
        let (w, h) = (m.w as f32, m.h as f32);
        match grab {
            Grab::Move => *self,
            Grab::Rotate => {
                // The centre is a fixed point of a rotation about itself, so
                // the offset needs no correction.
                let c = self.buf_to_cell(m, w * 0.5, h * 0.5);
                let a0 = (start.1 - c.1).atan2(start.0 - c.0);
                let a1 = (now.1 - c.1).atan2(now.0 - c.0);
                let mut rot = self.rot + (a1 - a0);
                if uniform {
                    let step = std::f32::consts::FRAC_PI_2 / 6.0;
                    rot = (rot / step).round() * step;
                }
                Pose { rot, ..*self }
            }
            Grab::Scale(id) => {
                let (anchor, (drives_x, drives_y)) =
                    grab.anchor(w, h).expect("Scale always has an anchor");
                let handle = Grab::handle_pos(id, w, h);
                // Solve in the frame where the anchor is the origin and the
                // rotation is undone: the pointer delta then reads straight off
                // as a per-axis ratio against the handle's buffer-space offset.
                let pa = self.buf_to_cell(m, anchor.0, anchor.1);
                let (s, c) = self.rot.sin_cos();
                let (dx, dy) = (now.0 - pa.0, now.1 - pa.1);
                let qx = dx * c + dy * s;
                let qy = -dx * s + dy * c;
                let (hx, hy) = (handle.0 - anchor.0, handle.1 - anchor.1);

                let mut sx = if drives_x && hx.abs() > 1e-6 {
                    qx / hx
                } else {
                    self.scale.0
                };
                let mut sy = if drives_y && hy.abs() > 1e-6 {
                    qy / hy
                } else {
                    self.scale.1
                };
                if uniform && drives_x && drives_y {
                    let u = sx.abs().max(sy.abs());
                    sx = u;
                    sy = u;
                }
                // Clamped rather than let through zero: a negative scale would
                // mirror the art, which is not what dragging a handle past the
                // far edge is asking for, and zero has no inverse.
                let mut p = Pose {
                    scale: (
                        sx.clamp(MIN_SCALE, MAX_SCALE),
                        sy.clamp(MIN_SCALE, MAX_SCALE),
                    ),
                    ..*self
                };
                // Scaling happens about the centre, so the anchor drifted. Put
                // it back — this is the one thing that makes `offset`
                // fractional.
                let moved = p.buf_to_cell(m, anchor.0, anchor.1);
                p.offset.0 += pa.0 - moved.0;
                p.offset.1 += pa.1 - moved.1;
                p
            }
        }
    }
}

#[derive(Clone)]
pub struct Selection {
    /// Cell the pixels were lifted from. A selection never outlives its cell:
    /// changing frame or layer commits it first.
    pub cell: CellId,
    pub mask: Mask,
    /// RGBA of the masked region, `mask.w * mask.h * 4`, straight alpha with
    /// the mask coverage already folded in.
    ///
    /// Written once at the lift and never touched again — see the module doc.
    pub pixels: Vec<u8>,
    /// Where the floating pixels currently sit relative to the lift.
    pub pose: Pose,
    /// The lasso path, kept in cell space as it was drawn. Run it through
    /// [`Selection::path_point`] to draw the outline under the current pose.
    pub path: Vec<(f32, f32)>,
    /// Whether the source pixels have been erased yet. Deferred until the first
    /// move so that selecting and then deselecting changes nothing.
    pub lifted: bool,
}

impl Selection {
    /// Copy the masked pixels out of `canvas`. The source is left intact —
    /// see [`Selection::lift_source`].
    pub fn new(cell: CellId, canvas: &Canvas, mask: Mask, path: Vec<(f32, f32)>) -> Self {
        let (mw, mh) = (mask.w, mask.h);
        let mut pixels = vec![0u8; (mw * mh * 4) as usize];
        for my in 0..mh {
            let py = mask.y + my;
            if py >= canvas.height {
                break;
            }
            for mx in 0..mw {
                let px = mask.x + mx;
                if px >= canvas.width {
                    break;
                }
                let c = mask.cov[(my * mw + mx) as usize];
                if c == 0 {
                    continue;
                }
                let s = ((py * canvas.width + px) * 4) as usize;
                let d = ((my * mw + mx) * 4) as usize;
                pixels[d] = canvas.pixels[s];
                pixels[d + 1] = canvas.pixels[s + 1];
                pixels[d + 2] = canvas.pixels[s + 2];
                // Coverage rides in the alpha, so a soft lasso edge stays soft
                // and lift + erase still sum to the original.
                pixels[d + 3] = ((canvas.pixels[s + 3] as u32 * c as u32 + 127) / 255) as u8;
            }
        }
        Self {
            cell,
            mask,
            pixels,
            pose: Pose::default(),
            path,
            lifted: false,
        }
    }

    /// Erase the source region, once, the first time the selection moves.
    pub fn lift_source(&mut self, canvas: &mut Canvas) {
        if self.lifted {
            return;
        }
        crate::tools::lasso::erase_masked(canvas, &self.mask);
        self.lifted = true;
    }

    /// Top-left of the floating pixels in cell space, for the pixel-aligned
    /// path. Meaningless once the selection is scaled or rotated.
    pub fn origin(&self) -> (i32, i32) {
        (
            self.mask.x as i32 + self.pose.offset.0.round() as i32,
            self.mask.y as i32 + self.pose.offset.1.round() as i32,
        )
    }

    pub fn buf_to_cell(&self, u: f32, v: f32) -> (f32, f32) {
        self.pose.buf_to_cell(&self.mask, u, v)
    }

    pub fn cell_to_buf(&self, x: f32, y: f32) -> (f32, f32) {
        self.pose.cell_to_buf(&self.mask, x, y)
    }

    /// The transformed mask rect in cell space, TL / TR / BR / BL.
    pub fn corners(&self) -> [(f32, f32); 4] {
        self.pose.corners(&self.mask)
    }

    /// Map a point of `path` (cell space, as drawn) through the current pose,
    /// so the outline tracks a scaled or rotated box.
    pub fn path_point(&self, x: f32, y: f32) -> (f32, f32) {
        self.buf_to_cell(x - self.mask.x as f32, y - self.mask.y as f32)
    }

    /// True when cell-space point `(x, y)` lands on covered pixels — the test
    /// for "is this drag a move, or a new lasso". Goes through the pose, so it
    /// follows a rotated or scaled box.
    pub fn hit(&self, x: f32, y: f32) -> bool {
        let (u, v) = self.cell_to_buf(x, y);
        // `Mask::contains` works in the cell space the mask was cut from, so
        // put the buffer coordinates back on that origin.
        self.mask.contains(
            u.floor() as i32 + self.mask.x as i32,
            v.floor() as i32 + self.mask.y as i32,
        )
    }

    /// Which part of the transform box cell-space point `(x, y)` lands on.
    /// `tol` is the handle's half-size in *cell* pixels, so the hit area stays
    /// a constant size on screen at any zoom.
    ///
    /// Tested in buffer space, where the box is always the plain mask rect —
    /// that keeps one set of comparisons correct under any rotation.
    pub fn grab_at(&self, x: f32, y: f32, tol: f32) -> Option<Grab> {
        let (u, v) = self.cell_to_buf(x, y);
        let (w, h) = (self.mask.w as f32, self.mask.h as f32);
        // Buffer space is pre-scale, so a fixed on-screen tolerance is worth
        // more buffer pixels on a shrunken axis than on a stretched one. Capped
        // so the handles of a small selection cannot swallow its whole inside.
        let tu = (tol / self.pose.scale.0.abs().max(1e-6)).min(w * 0.4);
        let tv = (tol / self.pose.scale.1.abs().max(1e-6)).min(h * 0.4);

        let (near_l, near_r) = (u.abs() <= tu, (u - w).abs() <= tu);
        let (near_t, near_b) = (v.abs() <= tv, (v - h).abs() <= tv);
        let on_box = u >= -tu && u <= w + tu && v >= -tv && v <= h + tv;

        if on_box {
            let corner = match (near_l, near_t, near_r, near_b) {
                (true, true, _, _) => Some(0),
                (_, true, true, _) => Some(1),
                (_, _, true, true) => Some(2),
                (true, _, _, true) => Some(3),
                _ => None,
            };
            if let Some(c) = corner {
                return Some(Grab::Scale(c));
            }
            if near_t {
                return Some(Grab::Scale(4));
            }
            if near_r {
                return Some(Grab::Scale(5));
            }
            if near_b {
                return Some(Grab::Scale(6));
            }
            if near_l {
                return Some(Grab::Scale(7));
            }
        }

        // Rotate ring: diagonally outside a corner, so an edge drag still means
        // a one-axis scale.
        if (u < 0.0 || u > w) && (v < 0.0 || v > h) {
            let cu = if u < 0.0 { 0.0 } else { w };
            let cv = if v < 0.0 { 0.0 } else { h };
            if (u - cu).abs() <= tu * 3.0 && (v - cv).abs() <= tv * 3.0 {
                return Some(Grab::Rotate);
            }
        }

        if self.hit(x, y) {
            return Some(Grab::Move);
        }
        None
    }

    /// Composite the floating pixels back into `canvas` under the current pose.
    /// Straight-alpha src-over, matching every other compositor in the app.
    pub fn stamp(&self, canvas: &mut Canvas) {
        if self.pose.is_pixel_aligned() {
            self.stamp_aligned(canvas);
        } else {
            self.stamp_transformed(canvas);
        }
    }

    /// The lossless path: a whole-pixel blit, byte for byte what was lifted.
    fn stamp_aligned(&self, canvas: &mut Canvas) {
        let (ox, oy) = self.origin();
        let (mw, mh) = (self.mask.w as i32, self.mask.h as i32);
        let (cw, ch) = (canvas.width as i32, canvas.height as i32);
        for my in 0..mh {
            let py = oy + my;
            if py < 0 || py >= ch {
                continue;
            }
            for mx in 0..mw {
                let px = ox + mx;
                if px < 0 || px >= cw {
                    continue;
                }
                let s = ((my * mw + mx) * 4) as usize;
                let sa = self.pixels[s + 3] as f32 / 255.0;
                if sa <= 0.0 {
                    continue;
                }
                let src = [
                    self.pixels[s] as f32 / 255.0,
                    self.pixels[s + 1] as f32 / 255.0,
                    self.pixels[s + 2] as f32 / 255.0,
                ];
                blend_over(canvas, px, py, src, sa);
            }
        }
        // Dirty the destination; the source rect is dirtied by the lift, and
        // `Canvas::mark_dirty` unions the two into the one rect undo records.
        let x0 = ox.max(0) as u32;
        let y0 = oy.max(0) as u32;
        let x1 = (ox + mw).clamp(0, cw) as u32;
        let y1 = (oy + mh).clamp(0, ch) as u32;
        if x1 > x0 && y1 > y0 {
            canvas.mark_dirty(x0, y0, x1 - x0, y1 - y0);
        }
    }

    /// Resampled stamp for a scaled or rotated selection: walk the destination
    /// rect and inverse-map each pixel back into the lifted buffer, the same
    /// shape as [`crate::io::composite::composite_layer`].
    fn stamp_transformed(&self, canvas: &mut Canvas) {
        let (cw, ch) = (canvas.width as i32, canvas.height as i32);
        let Some((x0, y0, x1, y1)) = clip_rect(self.dest_bounds(), cw, ch) else {
            return;
        };
        let n = self.supersample();
        for py in y0..y1 {
            for px in x0..x1 {
                if let Some((src, sa)) = self.resample(px, py, n) {
                    blend_over(canvas, px, py, src, sa);
                }
            }
        }
        canvas.mark_dirty(x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32);
    }

    /// Resample the pose into a fresh mask plus straight-alpha buffer, as if
    /// the selection had been stamped onto empty space and lifted again.
    ///
    /// `None` when the pose is already pixel-aligned — the caller should keep
    /// the pristine buffer in that case, which costs nothing and loses nothing.
    pub fn bake(&self) -> Option<(Mask, Vec<u8>)> {
        if self.pose.is_pixel_aligned() {
            return None;
        }
        let (x0, y0, x1, y1) = self.dest_bounds();
        let (w, h) = ((x1 - x0).max(1), (y1 - y0).max(1));
        // A runaway scale must not try to allocate gigabytes.
        if w as i64 * h as i64 > 64 << 20 {
            return None;
        }
        let (w, h) = (w as u32, h as u32);
        let n = self.supersample();
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let mut cov = vec![0u8; (w * h) as usize];
        let to8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
        for dy in 0..h {
            for dx in 0..w {
                let Some((src, a)) = self.resample(x0 + dx as i32, y0 + dy as i32, n) else {
                    continue;
                };
                let i = ((dy * w + dx) * 4) as usize;
                pixels[i] = to8(src[0]);
                pixels[i + 1] = to8(src[1]);
                pixels[i + 2] = to8(src[2]);
                pixels[i + 3] = to8(a);
                cov[(dy * w + dx) as usize] = to8(a);
            }
        }
        Some((
            Mask {
                x: x0.max(0) as u32,
                y: y0.max(0) as u32,
                w,
                h,
                cov,
            },
            pixels,
        ))
    }

    /// Integer bounds of the transformed box in cell space, unclipped.
    fn dest_bounds(&self) -> (i32, i32, i32, i32) {
        let (mut nx, mut ny) = (f32::MAX, f32::MAX);
        let (mut xx, mut xy) = (f32::MIN, f32::MIN);
        for &(x, y) in self.corners().iter() {
            nx = nx.min(x);
            xx = xx.max(x);
            ny = ny.min(y);
            xy = xy.max(y);
        }
        (
            nx.floor() as i32,
            ny.floor() as i32,
            xx.ceil() as i32 + 1,
            xy.ceil() as i32 + 1,
        )
    }

    /// Sample grid per destination pixel. Shrinking means each destination
    /// pixel covers several source texels, and a single bilinear tap would just
    /// pick one of them — anti-aliased line art then sparkles.
    fn supersample(&self) -> i32 {
        let s = self
            .pose
            .scale
            .0
            .abs()
            .min(self.pose.scale.1.abs())
            .max(1e-6);
        if s >= 1.0 {
            1
        } else {
            (1.0 / s).ceil().clamp(1.0, 4.0) as i32
        }
    }

    /// Resolve destination pixel `(px, py)` by inverse-mapping `n * n` sample
    /// points back into the lifted buffer. Returns straight-alpha RGB plus
    /// alpha, or `None` where nothing landed.
    fn resample(&self, px: i32, py: i32, n: i32) -> Option<([f32; 3], f32)> {
        let step = 1.0 / n as f32;
        let inv = 1.0 / (n * n) as f32;
        let (mut ar, mut ag, mut ab, mut aa) = (0.0f32, 0.0, 0.0, 0.0);
        for sy in 0..n {
            for sx in 0..n {
                let fx = px as f32 + (sx as f32 + 0.5) * step;
                let fy = py as f32 + (sy as f32 + 0.5) * step;
                let (u, v) = self.cell_to_buf(fx, fy);
                // Same half-pixel grid convention as `composite_layer`: the
                // centre of buffer texel 0 is u = 0.5.
                let s = self.sample_premul(u - 0.5, v - 0.5);
                ar += s[0];
                ag += s[1];
                ab += s[2];
                aa += s[3];
            }
        }
        let a = aa * inv;
        if a <= 0.0 {
            return None;
        }
        // Averaged premultiplied, then back to straight for the blend.
        Some(([ar * inv / a, ag * inv / a, ab * inv / a], a))
    }

    /// Bilinear tap on the lifted buffer, in **premultiplied** space, returning
    /// premultiplied RGBA in `0..=1`.
    ///
    /// `pixels` is straight alpha, and lerping straight alpha drags the RGB of
    /// fully transparent texels (all zeros) into the edge, haloing the art
    /// dark. Premultiplying before the lerp is what keeps the edge the colour
    /// the artist drew — which is why `io::composite::sample_bilinear` is not
    /// reused here.
    fn sample_premul(&self, x: f32, y: f32) -> [f32; 4] {
        let (w, h) = (self.mask.w as i32, self.mask.h as i32);
        let (fx0, fy0) = (x.floor(), y.floor());
        let (fx, fy) = (x - fx0, y - fy0);
        let (x0, y0) = (fx0 as i32, fy0 as i32);
        let tap = |ix: i32, iy: i32| -> [f32; 4] {
            // Outside the buffer is empty, not clamped: clamping would smear
            // the border texels outwards as the box grows.
            if ix < 0 || iy < 0 || ix >= w || iy >= h {
                return [0.0; 4];
            }
            let i = ((iy * w + ix) * 4) as usize;
            let a = self.pixels[i + 3] as f32 / 255.0;
            [
                self.pixels[i] as f32 / 255.0 * a,
                self.pixels[i + 1] as f32 / 255.0 * a,
                self.pixels[i + 2] as f32 / 255.0 * a,
                a,
            ]
        };
        let p00 = tap(x0, y0);
        let p10 = tap(x0 + 1, y0);
        let p01 = tap(x0, y0 + 1);
        let p11 = tap(x0 + 1, y0 + 1);
        let mut out = [0.0f32; 4];
        for c in 0..4 {
            let top = p00[c] + (p10[c] - p00[c]) * fx;
            let bot = p01[c] + (p11[c] - p01[c]) * fx;
            out[c] = top + (bot - top) * fy;
        }
        out
    }
}

/// Straight-alpha src-over of one source sample onto `canvas` at `(px, py)`.
/// `src` is unpremultiplied RGB in `0..=1`, `sa` its alpha.
fn blend_over(canvas: &mut Canvas, px: i32, py: i32, src: [f32; 3], sa: f32) {
    let d = ((py * canvas.width as i32 + px) * 4) as usize;
    let da = canvas.pixels[d + 3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        canvas.pixels[d..d + 4].copy_from_slice(&[0, 0, 0, 0]);
        return;
    }
    for (c, &sv) in src.iter().enumerate() {
        let dv = canvas.pixels[d + c] as f32 / 255.0;
        let ov = (sv * sa + dv * da * (1.0 - sa)) / out_a;
        canvas.pixels[d + c] = (ov * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    canvas.pixels[d + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Clip integer bounds to a canvas, or `None` when nothing is left.
fn clip_rect(b: (i32, i32, i32, i32), cw: i32, ch: i32) -> Option<(i32, i32, i32, i32)> {
    let x0 = b.0.clamp(0, cw);
    let y0 = b.1.clamp(0, ch);
    let x1 = b.2.clamp(0, cw);
    let y1 = b.3.clamp(0, ch);
    if x1 <= x0 || y1 <= y0 {
        None
    } else {
        Some((x0, y0, x1, y1))
    }
}

/// The mask's bounding box as a closed path.
///
/// Used for a pasted selection, where the original lasso path is long gone —
/// the outline then marks the region rather than tracing the artwork.
pub fn outline_rect(mask: &Mask) -> Vec<(f32, f32)> {
    let (x0, y0) = (mask.x as f32, mask.y as f32);
    let (x1, y1) = (x0 + mask.w as f32, y0 + mask.h as f32);
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::lasso;

    fn filled(w: u32, h: u32, rgba: [u8; 4]) -> Canvas {
        let mut c = Canvas::new(w, h);
        for px in c.pixels.chunks_exact_mut(4) {
            px.copy_from_slice(&rgba);
        }
        c
    }

    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<(f32, f32)> {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    fn sel_on(canvas: &Canvas, path: Vec<(f32, f32)>) -> Selection {
        let mask = lasso::coverage(&path, canvas.width, canvas.height).expect("mask");
        Selection::new(0, canvas, mask, path)
    }

    fn rgba(c: &Canvas, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * c.width + x) * 4) as usize;
        [c.pixels[i], c.pixels[i + 1], c.pixels[i + 2], c.pixels[i + 3]]
    }

    #[test]
    fn lift_then_stamp_in_place_restores_the_drawing() {
        let mut canvas = filled(8, 8, [10, 20, 30, 255]);
        let before = canvas.pixels.clone();
        let mut sel = sel_on(&canvas, square(2.0, 2.0, 6.0, 6.0));

        sel.lift_source(&mut canvas);
        // The hole is real.
        let mid = ((3 * 8 + 3) * 4) as usize;
        assert_eq!(canvas.pixels[mid + 3], 0);

        sel.stamp(&mut canvas);
        for (i, (&a, &b)) in before.iter().zip(canvas.pixels.iter()).enumerate() {
            assert!(
                a.abs_diff(b) <= 1,
                "byte {i}: {a} != {b} after lift+stamp round trip"
            );
        }
    }

    #[test]
    fn moving_puts_the_pixels_at_the_offset() {
        let mut canvas = filled(8, 8, [200, 100, 50, 255]);
        let mut sel = sel_on(&canvas, square(1.0, 1.0, 3.0, 3.0));
        sel.lift_source(&mut canvas);
        sel.pose.offset = (4.0, 4.0);
        sel.stamp(&mut canvas);

        // Source pixel cleared, destination pixel painted.
        let src = ((2 * 8 + 2) * 4) as usize;
        let dst = ((6 * 8 + 6) * 4) as usize;
        assert_eq!(canvas.pixels[src + 3], 0);
        assert_eq!(canvas.pixels[dst + 3], 255);
        assert_eq!(canvas.pixels[dst], 200);
    }

    #[test]
    fn pixels_moved_off_canvas_are_dropped_not_wrapped() {
        let mut canvas = filled(8, 8, [1, 2, 3, 255]);
        let mut sel = sel_on(&canvas, square(1.0, 1.0, 3.0, 3.0));
        sel.lift_source(&mut canvas);
        sel.pose.offset = (-40.0, 0.0);
        sel.stamp(&mut canvas);
        // Nothing reappeared on the far side.
        let right = ((2 * 8 + 7) * 4) as usize;
        assert_eq!(canvas.pixels[right + 3], 255);
    }

    #[test]
    fn hit_testing_follows_the_offset() {
        let canvas = filled(8, 8, [0, 0, 0, 255]);
        let mut sel = sel_on(&canvas, square(1.0, 1.0, 4.0, 4.0));
        assert!(sel.hit(2.0, 2.0));
        assert!(!sel.hit(6.0, 6.0));
        sel.pose.offset = (4.0, 4.0);
        assert!(!sel.hit(2.0, 2.0));
        assert!(sel.hit(6.0, 6.0));
    }

    /// A whole-pixel move must keep taking the exact blit, or the README's
    /// "moves never resample" promise quietly stops being true.
    #[test]
    fn a_whole_pixel_move_is_still_pixel_aligned() {
        let mut p = Pose::default();
        assert!(p.is_pixel_aligned());
        p.offset = (-7.0, 3.0);
        assert!(p.is_pixel_aligned());
        p.offset = (0.5, 0.0);
        assert!(!p.is_pixel_aligned());
        p.offset = (0.0, 0.0);
        p.scale = (1.5, 1.0);
        assert!(!p.is_pixel_aligned());
        p.scale = (1.0, 1.0);
        p.rot = 0.01;
        assert!(!p.is_pixel_aligned());
    }

    /// `cell_to_buf` is a hand-written inverse of `buf_to_cell`, so the two can
    /// silently disagree — exactly the failure the `Xform` round-trip test in
    /// the shell exists to catch. Check a pose using every term at once.
    #[test]
    fn buf_and_cell_mappings_round_trip() {
        let m = Mask {
            x: 13,
            y: 7,
            w: 40,
            h: 25,
            cov: vec![255; 40 * 25],
        };
        let p = Pose {
            offset: (3.25, -9.75),
            scale: (1.7, 0.6),
            rot: 0.9,
        };
        for &(u, v) in &[(0.0, 0.0), (40.0, 0.0), (17.5, 9.25), (40.0, 25.0)] {
            let (x, y) = p.buf_to_cell(&m, u, v);
            let (bu, bv) = p.cell_to_buf(&m, x, y);
            assert!(
                (bu - u).abs() < 1e-3 && (bv - v).abs() < 1e-3,
                "({u},{v}) -> ({x},{y}) -> ({bu},{bv})"
            );
        }
    }

    /// A quarter turn about the box centre must land the corners on each
    /// other's places, not somewhere near them.
    #[test]
    fn a_quarter_turn_puts_the_corners_where_they_belong() {
        let m = Mask {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            cov: vec![255; 100],
        };
        let p = Pose {
            rot: std::f32::consts::FRAC_PI_2,
            ..Pose::default()
        };
        let c = p.corners(&m);
        // TL swings to where TR was, and so on round the box.
        let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
        assert!(near(c[0], (10.0, 0.0)), "TL went to {:?}", c[0]);
        assert!(near(c[1], (10.0, 10.0)), "TR went to {:?}", c[1]);
        assert!(near(c[2], (0.0, 10.0)), "BR went to {:?}", c[2]);
        assert!(near(c[3], (0.0, 0.0)), "BL went to {:?}", c[3]);
    }

    /// Rotating a tall selection by a quarter turn should make it wide, and the
    /// pixels should actually be there.
    #[test]
    fn rotating_a_selection_stamps_it_turned() {
        let mut canvas = Canvas::new(21, 21);
        // A 3x11 vertical bar down the middle.
        for y in 5..16 {
            for x in 9..12 {
                let i = ((y * 21 + x) * 4) as usize;
                canvas.pixels[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let mut sel = sel_on(&canvas, square(9.0, 5.0, 12.0, 16.0));
        sel.lift_source(&mut canvas);
        sel.pose.rot = std::f32::consts::FRAC_PI_2;
        sel.stamp(&mut canvas);

        // The bar now runs across, not down: it is painted well out along the
        // centre row, and the rows it used to reach are empty.
        assert!(rgba(&canvas, 8, 11)[3] > 200, "left of the turned bar");
        assert!(rgba(&canvas, 15, 11)[3] > 200, "right of the turned bar");
        assert_eq!(rgba(&canvas, 10, 5)[3], 0, "old top of the bar is empty");
        assert_eq!(rgba(&canvas, 10, 15)[3], 0, "old bottom of the bar is empty");
    }

    /// Doubling the scale about the centre must widen the covered span to match.
    #[test]
    fn scaling_up_covers_twice_the_span() {
        let mut canvas = Canvas::new(40, 40);
        for y in 16..24 {
            for x in 16..24 {
                let i = ((y * 40 + x) * 4) as usize;
                canvas.pixels[i..i + 4].copy_from_slice(&[0, 200, 0, 255]);
            }
        }
        let mut sel = sel_on(&canvas, square(16.0, 16.0, 24.0, 24.0));
        sel.lift_source(&mut canvas);
        sel.pose.scale = (2.0, 2.0);
        sel.stamp(&mut canvas);

        // An 8x8 block centred on (20, 20) becomes 16x16: (13, 20) is now
        // inside where it used to be well outside.
        assert!(rgba(&canvas, 13, 20)[3] > 200, "grew leftwards");
        assert!(rgba(&canvas, 26, 20)[3] > 200, "grew rightwards");
        assert_eq!(rgba(&canvas, 10, 20)[3], 0, "but not without limit");
    }

    /// Straight-alpha bilinear would pull the zeroed RGB of the transparent
    /// surround into the edge and mud the colour toward black. Premultiplied
    /// sampling must not.
    #[test]
    fn scaling_does_not_darken_the_edge() {
        let mut canvas = Canvas::new(32, 32);
        for y in 12..20 {
            for x in 12..20 {
                let i = ((y * 32 + x) * 4) as usize;
                canvas.pixels[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let mut sel = sel_on(&canvas, square(12.0, 12.0, 20.0, 20.0));
        sel.lift_source(&mut canvas);
        sel.pose.scale = (2.5, 2.5);
        sel.stamp(&mut canvas);

        // Walk the row through the centre and check every painted pixel is
        // still red, however partial its coverage.
        let mut seen_partial = false;
        for x in 0..32 {
            let p = rgba(&canvas, x, 16);
            if p[3] == 0 {
                continue;
            }
            if p[3] < 250 {
                seen_partial = true;
            }
            assert!(
                p[0] > 240 && p[1] < 12 && p[2] < 12,
                "pixel {x} came out {p:?}, not red — the edge got muddied"
            );
        }
        assert!(seen_partial, "no partially covered edge pixel to check");
    }

    /// The pose is metadata over a pristine buffer, so scaling out and back
    /// again must not soften anything: the second pose is resolved from the
    /// original pixels, not from the result of the first.
    #[test]
    fn adjusting_the_pose_does_not_accumulate_blur() {
        let mut canvas = Canvas::new(24, 24);
        for y in 8..16 {
            for x in 8..16 {
                let i = ((y * 24 + x) * 4) as usize;
                canvas.pixels[i..i + 4].copy_from_slice(&[30, 40, 50, 255]);
            }
        }
        let before = canvas.pixels.clone();
        let mut sel = sel_on(&canvas, square(8.0, 8.0, 16.0, 16.0));
        sel.lift_source(&mut canvas);
        // Out to 3x, then all the way back. The buffer never changed, so the
        // stamp is the pixel-aligned one again.
        sel.pose.scale = (3.0, 3.0);
        sel.pose.scale = (1.0, 1.0);
        assert!(sel.pose.is_pixel_aligned());
        sel.stamp(&mut canvas);
        for (i, (&a, &b)) in before.iter().zip(canvas.pixels.iter()).enumerate() {
            assert!(a.abs_diff(b) <= 1, "byte {i}: {a} != {b}");
        }
    }

    /// Scaling from a corner handle must pin the opposite corner, or the
    /// drawing slides out from under the pointer.
    #[test]
    fn a_corner_drag_pins_the_opposite_corner() {
        let m = Mask {
            x: 5,
            y: 5,
            w: 20,
            h: 10,
            cov: vec![255; 200],
        };
        let p = Pose::default();
        // Grab BR (handle 2) at (25, 15) and pull it to (45, 35).
        let after = p.dragged(&m, Grab::Scale(2), (25.0, 15.0), (45.0, 35.0), false);
        let tl = after.buf_to_cell(&m, 0.0, 0.0);
        let br = after.buf_to_cell(&m, 20.0, 10.0);
        assert!(
            (tl.0 - 5.0).abs() < 1e-3 && (tl.1 - 5.0).abs() < 1e-3,
            "TL moved to {tl:?}"
        );
        assert!(
            (br.0 - 45.0).abs() < 1e-3 && (br.1 - 35.0).abs() < 1e-3,
            "BR landed at {br:?}, not under the pointer"
        );
        assert!((after.scale.0 - 2.0).abs() < 1e-3);
        assert!((after.scale.1 - 3.0).abs() < 1e-3);
    }

    /// Shift on a corner locks the two axes to one ratio.
    #[test]
    fn shift_makes_a_corner_drag_uniform() {
        let m = Mask {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            cov: vec![255; 100],
        };
        let after = Pose::default().dragged(&m, Grab::Scale(2), (10.0, 10.0), (40.0, 20.0), true);
        assert!((after.scale.0 - after.scale.1).abs() < 1e-4);
        // The bigger of the two ratios wins (4x across, 2x down).
        assert!((after.scale.0 - 4.0).abs() < 1e-3, "got {}", after.scale.0);
    }

    /// An edge handle drives one axis and leaves the other alone.
    #[test]
    fn an_edge_drag_scales_one_axis() {
        let m = Mask {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            cov: vec![255; 100],
        };
        // Handle 5 is the right edge.
        let after = Pose::default().dragged(&m, Grab::Scale(5), (10.0, 5.0), (30.0, 5.0), false);
        assert!((after.scale.0 - 3.0).abs() < 1e-3, "got {}", after.scale.0);
        assert!((after.scale.1 - 1.0).abs() < 1e-6, "y must not move");
    }

    /// Dragging a handle past its anchor clamps instead of mirroring the art.
    #[test]
    fn dragging_past_the_anchor_clamps_rather_than_flips() {
        let m = Mask {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            cov: vec![255; 100],
        };
        let after = Pose::default().dragged(&m, Grab::Scale(2), (10.0, 10.0), (-30.0, -30.0), false);
        assert_eq!(after.scale.0, MIN_SCALE);
        assert_eq!(after.scale.1, MIN_SCALE);
    }

    /// Rotation turns about the centre and leaves it where it was.
    #[test]
    fn a_rotate_drag_keeps_the_centre_put() {
        let m = Mask {
            x: 10,
            y: 10,
            w: 20,
            h: 20,
            cov: vec![255; 400],
        };
        let p = Pose::default();
        let before = p.buf_to_cell(&m, 10.0, 10.0);
        // From due east of the centre to due south: a quarter turn.
        let after = p.dragged(&m, Grab::Rotate, (40.0, 20.0), (20.0, 40.0), false);
        let moved = after.buf_to_cell(&m, 10.0, 10.0);
        assert!((moved.0 - before.0).abs() < 1e-3 && (moved.1 - before.1).abs() < 1e-3);
        assert!(
            (after.rot - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "got {}",
            after.rot
        );
    }

    /// Shift snaps a rotation to 15°, for squaring a drawing back up.
    #[test]
    fn shift_snaps_rotation_to_fifteen_degrees() {
        let m = Mask {
            x: 0,
            y: 0,
            w: 20,
            h: 20,
            cov: vec![255; 400],
        };
        let step = std::f32::consts::FRAC_PI_2 / 6.0;
        let after = Pose::default().dragged(&m, Grab::Rotate, (30.0, 10.0), (29.0, 13.0), true);
        let k = (after.rot / step).round();
        assert!((after.rot - k * step).abs() < 1e-4, "got {}", after.rot);
    }

    /// The handles must be grabbable, and the interior must still read as a
    /// move so a plain drag is unchanged.
    #[test]
    fn grab_picks_handles_then_the_interior() {
        let canvas = filled(40, 40, [0, 0, 0, 255]);
        let sel = sel_on(&canvas, square(10.0, 10.0, 30.0, 30.0));
        let tol = 2.0;
        assert_eq!(sel.grab_at(10.0, 10.0, tol), Some(Grab::Scale(0)), "TL");
        assert_eq!(sel.grab_at(30.0, 10.0, tol), Some(Grab::Scale(1)), "TR");
        assert_eq!(sel.grab_at(30.0, 30.0, tol), Some(Grab::Scale(2)), "BR");
        assert_eq!(sel.grab_at(10.0, 30.0, tol), Some(Grab::Scale(3)), "BL");
        assert_eq!(sel.grab_at(20.0, 10.0, tol), Some(Grab::Scale(4)), "top");
        assert_eq!(sel.grab_at(30.0, 20.0, tol), Some(Grab::Scale(5)), "right");
        assert_eq!(sel.grab_at(20.0, 20.0, tol), Some(Grab::Move), "interior");
        assert_eq!(sel.grab_at(7.0, 7.0, tol), Some(Grab::Rotate), "outside TL");
        assert_eq!(sel.grab_at(39.0, 39.0, tol), None, "far away");
    }

    /// Hit testing goes through the pose, so a rotated box is grabbable where
    /// it looks rather than where it was lifted.
    #[test]
    fn hit_testing_follows_a_rotation() {
        let canvas = filled(40, 40, [0, 0, 0, 255]);
        // A wide, short region: rotating it must swap which points are inside.
        let mut sel = sel_on(&canvas, square(10.0, 18.0, 30.0, 22.0));
        assert!(sel.hit(28.0, 20.0), "near the right end before the turn");
        assert!(!sel.hit(20.0, 28.0), "below the middle before the turn");
        sel.pose.rot = std::f32::consts::FRAC_PI_2;
        assert!(!sel.hit(28.0, 20.0), "right end after the turn");
        assert!(sel.hit(20.0, 28.0), "below the middle after the turn");
    }

    /// The clipboard bakes a transformed selection so it pastes as it looked,
    /// and leaves a pixel-aligned one alone so a plain copy stays lossless.
    #[test]
    fn bake_resamples_only_when_the_pose_needs_it() {
        let canvas = filled(30, 30, [90, 90, 90, 255]);
        let mut sel = sel_on(&canvas, square(10.0, 10.0, 20.0, 20.0));
        assert!(sel.bake().is_none(), "identity needs no bake");
        sel.pose.offset = (3.0, -2.0);
        assert!(sel.bake().is_none(), "a whole-pixel move needs no bake");

        let (mw, mh) = (sel.mask.w, sel.mask.h);
        sel.pose.scale = (2.0, 2.0);
        let (mask, pixels) = sel.bake().expect("a scaled pose bakes");
        assert_eq!(pixels.len(), (mask.w * mask.h * 4) as usize);
        assert_eq!(mask.cov.len(), (mask.w * mask.h) as usize);
        // Twice the lifted buffer, give or take the bounds' rounding margin.
        assert!(
            (mw * 2..=mw * 2 + 3).contains(&mask.w),
            "baked width {} for a {mw}-wide buffer",
            mask.w
        );
        assert!(
            (mh * 2..=mh * 2 + 3).contains(&mask.h),
            "baked height {} for a {mh}-tall buffer",
            mask.h
        );
        // The middle of the baked patch carries the original colour.
        let i = (((mask.h / 2) * mask.w + mask.w / 2) * 4) as usize;
        assert_eq!(pixels[i + 3], 255);
        assert!(pixels[i].abs_diff(90) <= 1);
    }
}
