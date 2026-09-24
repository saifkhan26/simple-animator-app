//! Ribbon stroke rasterizer.
//!
//! A stroke is a polyline spine of `SpineNode`s (position + pressure-modulated
//! radius + flow). Each spine segment is rasterized as a variable-radius
//! capsule into a per-stroke coverage buffer combined with `max()`, then the
//! coverage is composited src-over the pre-stroke snapshot into the canvas.
//!
//! It also stamps dabs, for brushes that want them: same coverage buffer,
//! same falloff, same grain, but combined so overlapping stamps darken.
//!
//! The combine rule is the real difference between the two models. `max()`
//! writes each stroke pixel once or twice, gives uniform per-stroke opacity
//! and never darkens at joints or self-crossings — an ink line. Build-up
//! writes each pixel ~1/spacing times and lets density accumulate, which is
//! what makes graphite look like graphite. Both are monotone non-decreasing,
//! which is what keeps incremental compositing exact (see `composite_paint`).
//! Both composite against the pre-stroke pixels, so painting semi-
//! transparently over already-opaque content works.
//!
//! The rasterize/composite split is deliberate: a wgpu compute port replaces
//! the internals of this module without touching the stroke input model.

use crate::doc::canvas::{Canvas, DirtyRect};
use crate::tools::dab::Dab;
use crate::tools::paper::paper;
use crate::tools::{BrushMode, BrushSettings};

/// One vertex of the stroke spine.
#[derive(Clone, Copy, Debug)]
pub struct SpineNode {
    pub x: f32,
    pub y: f32,
    /// Brush radius at this node in pixels (pressure-modulated).
    pub radius: f32,
    /// Falloff peak at this node, 0..=1 (pressure -> opacity modulation).
    /// Final pixel alpha = coverage * per-stroke opacity.
    pub flow: f32,
}

/// Half-pixel anti-aliasing band added to the local radius.
const AA: f32 = 0.5;

/// A straight cut across one end of a stroke: a point on the cut and the unit
/// normal pointing *into* the stroke. Coverage on the far side is removed,
/// with a one-pixel anti-aliased edge centred on the line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapPlane {
    pub x: f32,
    pub y: f32,
    pub ux: f32,
    pub uy: f32,
}

impl CapPlane {
    /// Fraction of a pixel centred at `(fx, fy)` that lies on the kept side.
    #[inline]
    fn keep(&self, fx: f32, fy: f32) -> f32 {
        ((fx - self.x) * self.ux + (fy - self.y) * self.uy + 0.5).clamp(0.0, 1.0)
    }
}

/// Persistent, reusable per-stroke scratch state. One per app; `begin()` is
/// called at stroke start and clears only the region the previous stroke
/// touched.
pub struct StrokeWorkspace {
    /// Canvas-sized coverage, row-major, u16 fixed point (65535 == 1.0).
    cov: Vec<u16>,
    w: u32,
    h: u32,
    /// Union of everything rasterized this stroke. Cleared lazily by the
    /// next `begin()`.
    stroke_rect: Option<DirtyRect>,
    /// Edge hardness, 0..=1: fraction of the radius that is fully solid.
    /// 1.0 = crisp edge (falloff only in the ~1px AA rim), 0.0 = airbrush
    /// (falloff across the whole radius).
    hardness: f32,
    /// Exponent on the falloff; see `BrushSettings::softness`.
    softness: f32,
    /// Paper-grain strength, 0..=1: how deeply canvas-position noise eats
    /// into the coverage (0 = smooth ink).
    grain: f32,
    /// Canvas pixels per grain texel.
    grain_scale: f32,
    /// How a new stamp combines with the coverage already there.
    build_up: bool,
}

impl StrokeWorkspace {
    pub fn new() -> Self {
        Self {
            cov: Vec::new(),
            w: 0,
            h: 0,
            stroke_rect: None,
            hardness: 1.0,
            softness: 1.0,
            grain: 0.0,
            grain_scale: 1.5,
            build_up: false,
        }
    }

    /// Prepare for a new stroke: size the coverage buffer to the target
    /// canvas, zero the previous stroke's footprint, latch brush profile.
    pub fn begin(&mut self, w: u32, h: u32, brush: &BrushSettings) {
        if self.w != w || self.h != h {
            self.cov = vec![0u16; (w * h) as usize];
            self.w = w;
            self.h = h;
        } else if let Some(r) = self.stroke_rect {
            for y in r.min_y..r.max_y {
                let row = (y * self.w) as usize;
                self.cov[row + r.min_x as usize..row + r.max_x as usize].fill(0);
            }
        }
        self.stroke_rect = None;
        self.hardness = brush.hardness.clamp(0.0, 1.0);
        self.softness = brush.softness.clamp(0.1, 8.0);
        self.grain = brush.grain.clamp(0.0, 1.0);
        self.grain_scale = brush.grain_scale.max(0.05);
        self.build_up = brush.mode == BrushMode::Dab;
    }

    /// Falloff mask for distance `d` from the spine given the local outer
    /// radius: fully solid core, smoothstep rim of width
    /// `(1 - hardness) * outer`, never narrower than 1px (anti-aliasing).
    #[inline]
    fn mask(&self, d: f32, outer: f32) -> f32 {
        self.mask_w(d, outer, 1.0)
    }

    /// As `mask`, but with an explicit floor on the falloff width. The dab
    /// rasterizer works in normalised ellipse coordinates, where one canvas
    /// pixel of anti-aliasing is not one unit.
    #[inline]
    fn mask_w(&self, d: f32, outer: f32, min_width: f32) -> f32 {
        let w = ((1.0 - self.hardness) * outer).max(min_width);
        let s = ((outer - d) / w).clamp(0.0, 1.0);
        let base = s * s * (3.0 - 2.0 * s);
        if self.softness == 1.0 {
            base
        } else {
            base.powf(self.softness)
        }
    }

    /// Combine a new stamp's coverage into the buffer: `max()` for a
    /// ribbon, `a + b(1 - a)` for dabs.
    #[inline]
    fn combine(&mut self, idx: usize, c16: u16) {
        let cell = &mut self.cov[idx];
        if self.build_up {
            let a = *cell as u32;
            let b = c16 as u32;
            *cell = (a + b - (a * b) / 65535).min(65535) as u16;
        } else if c16 > *cell {
            *cell = c16;
        }
    }

    /// Grain-attenuated coverage for a pixel. Noise is a pure hash of the
    /// canvas position (paper texture): deterministic, so overlapping
    /// strokes and `max()` re-rasterization see identical values.
    #[inline]
    fn pixel_cov(&self, mask: f32, flow: f32, x: u32, y: u32) -> u16 {
        let mut cov = mask * flow;
        if self.grain > 0.0 {
            let inv = 1.0 / self.grain_scale;
            let tex = paper().sample(x as f32 * inv, y as f32 * inv);
            cov *= 1.0 - self.grain * (1.0 - tex);
        }
        (cov.clamp(0.0, 1.0) * 65535.0) as u16
    }

    /// Rasterize a variable-radius capsule from `a` to `b` into the coverage
    /// buffer with `max()` combining. Returns the clipped pixel bbox touched
    /// (exclusive max), or `None` if fully off-canvas.
    ///
    /// The projection method is used: each pixel takes the radius/flow lerped
    /// at its clamped projection onto the segment. Clamping yields round caps
    /// at both ends for free; interior joints are covered by adjacent
    /// segments' caps and deduplicated by `max()`. The shape is well-formed
    /// for any taper, so no |r0-r1| special-casing is needed.
    pub fn raster_capsule(&mut self, a: SpineNode, b: SpineNode) -> Option<DirtyRect> {
        self.raster_capsule_clipped(a, b, &[])
    }

    /// `raster_capsule`, with coverage beyond each of `clips` cut away. This
    /// is how a flat end is made: the round caps are drawn as usual and then
    /// sliced off square.
    pub fn raster_capsule_clipped(
        &mut self,
        a: SpineNode,
        b: SpineNode,
        clips: &[CapPlane],
    ) -> Option<DirtyRect> {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len2 = dx * dx + dy * dy;
        if len2 < 1e-6 {
            let n = if a.radius >= b.radius { a } else { b };
            return self.raster_dot_clipped(n, clips);
        }
        let inv_len2 = 1.0 / len2;
        let dr = b.radius - a.radius;
        let dflow = b.flow - a.flow;

        let (x0, y0, x1, y1) = self.capsule_box(a, b)?;

        for py in y0..=y1 {
            let fy = py as f32 + 0.5;
            let row = (py as u32 * self.w) as usize;
            for px in x0..=x1 {
                let fx = px as f32 + 0.5;
                // Clamped projection parameter -> round caps.
                let t = (((fx - a.x) * dx + (fy - a.y) * dy) * inv_len2).clamp(0.0, 1.0);
                let ex = fx - (a.x + t * dx);
                let ey = fy - (a.y + t * dy);
                let d2 = ex * ex + ey * ey;
                let outer = a.radius + t * dr + AA;
                if d2 >= outer * outer {
                    continue;
                }
                let mut mask = self.mask(d2.sqrt(), outer);
                for c in clips {
                    mask *= c.keep(fx, fy);
                }
                if mask <= 0.0 {
                    continue;
                }
                let c16 = self.pixel_cov(mask, a.flow + t * dflow, px as u32, py as u32);
                self.combine(row + px as usize, c16);
            }
        }

        self.touched(x0, y0, x1, y1)
    }

    /// Inclusive, canvas-clipped pixel box a capsule from `a` to `b` can
    /// touch, or `None` if it lies wholly off the canvas. A dot is the
    /// capsule from a node to itself.
    fn capsule_box(&self, a: SpineNode, b: SpineNode) -> Option<(i32, i32, i32, i32)> {
        let pad = a.radius.max(b.radius) + AA + 1.0;
        let x0 = ((a.x.min(b.x) - pad).floor() as i32).max(0);
        let y0 = ((a.y.min(b.y) - pad).floor() as i32).max(0);
        let x1 = ((a.x.max(b.x) + pad).ceil() as i32).min(self.w as i32 - 1);
        let y1 = ((a.y.max(b.y) + pad).ceil() as i32).min(self.h as i32 - 1);
        (x1 >= x0 && y1 >= y0).then_some((x0, y0, x1, y1))
    }

    /// The rect `raster_capsule(a, b)` would return, without drawing it.
    pub fn capsule_rect(&self, a: SpineNode, b: SpineNode) -> Option<DirtyRect> {
        let (x0, y0, x1, y1) = self.capsule_box(a, b)?;
        Some(DirtyRect {
            min_x: x0 as u32,
            min_y: y0 as u32,
            max_x: (x1 + 1) as u32,
            max_y: (y1 + 1) as u32,
        })
    }

    /// Rasterize a single dot (tap / first sample): a plain disc, the
    /// degenerate case of `raster_capsule`.
    pub fn raster_dot(&mut self, n: SpineNode) -> Option<DirtyRect> {
        self.raster_dot_clipped(n, &[])
    }

    /// `raster_dot` with coverage beyond each of `clips` cut away.
    pub fn raster_dot_clipped(&mut self, n: SpineNode, clips: &[CapPlane]) -> Option<DirtyRect> {
        let (x0, y0, x1, y1) = self.capsule_box(n, n)?;
        let outer = n.radius + AA;
        let outer2 = outer * outer;

        for py in y0..=y1 {
            let fy = py as f32 + 0.5;
            let row = (py as u32 * self.w) as usize;
            for px in x0..=x1 {
                let fx = px as f32 + 0.5;
                let ex = fx - n.x;
                let ey = fy - n.y;
                let d2 = ex * ex + ey * ey;
                if d2 >= outer2 {
                    continue;
                }
                let mut mask = self.mask(d2.sqrt(), outer);
                for c in clips {
                    mask *= c.keep(fx, fy);
                }
                if mask <= 0.0 {
                    continue;
                }
                let c16 = self.pixel_cov(mask, n.flow, px as u32, py as u32);
                self.combine(row + px as usize, c16);
            }
        }

        self.touched(x0, y0, x1, y1)
    }

    /// Stamp one elliptical dab.
    ///
    /// Pixels are tested in the dab's own frame, where it is a unit circle,
    /// so a rotated or flattened dab costs no more than a round one and lands
    /// exactly on its sub-pixel position. The anti-alias band is derived from
    /// the *minor* axis, since that is the direction in which a flattened dab
    /// has the least room for a soft edge.
    pub fn raster_dab(&mut self, d: Dab) -> Option<DirtyRect> {
        let a = d.radius.max(0.05);
        let b = (d.radius * d.ratio).max(0.05);
        let (sin, cos) = d.angle.sin_cos();

        // Half-extents of the rotated ellipse's bounding box.
        let ex = ((a * cos) * (a * cos) + (b * sin) * (b * sin)).sqrt() + AA + 1.0;
        let ey = ((a * sin) * (a * sin) + (b * cos) * (b * cos)).sqrt() + AA + 1.0;
        let x0 = ((d.x - ex).floor() as i32).max(0);
        let y0 = ((d.y - ey).floor() as i32).max(0);
        let x1 = ((d.x + ex).ceil() as i32).min(self.w as i32 - 1);
        let y1 = ((d.y + ey).ceil() as i32).min(self.h as i32 - 1);
        if x1 < x0 || y1 < y0 {
            return None;
        }

        // One canvas pixel of rim, expressed in the normalised frame.
        let aa_n = (AA / b).min(0.5);
        let outer = 1.0 + aa_n;
        let outer2 = outer * outer;

        for py in y0..=y1 {
            let fy = py as f32 + 0.5 - d.y;
            let row = (py as u32 * self.w) as usize;
            for px in x0..=x1 {
                let fx = px as f32 + 0.5 - d.x;
                // Into the dab's frame, then squash it to a unit circle.
                let lx = (fx * cos + fy * sin) / a;
                let ly = (-fx * sin + fy * cos) / b;
                let dn2 = lx * lx + ly * ly;
                if dn2 >= outer2 {
                    continue;
                }
                let mask = self.mask_w(dn2.sqrt(), outer, aa_n);
                let c16 = self.pixel_cov(mask, d.flow, px as u32, py as u32);
                self.combine(row + px as usize, c16);
            }
        }

        self.touched(x0, y0, x1, y1)
    }

    /// Composite `coverage * opacity` src-over the pre-stroke snapshot into
    /// the canvas, only inside `rect`.
    ///
    /// Correctness of incremental compositing: the result is a pure function
    /// of (pre-stroke pixels, current coverage), never of the canvas's
    /// current value, and coverage is monotone non-decreasing under `max()`.
    /// Pixels outside `rect` have unchanged coverage and already hold the
    /// correct value; pixels inside are recomputed from `pre`. Hence
    /// compositing only each flush's new-segment bbox union is exact, even
    /// where a new segment overlaps previously composited regions.
    pub fn composite_paint(
        &self,
        canvas: &mut Canvas,
        pre: &[u8],
        rect: DirtyRect,
        color: [u8; 4],
        opacity: f32,
    ) {
        let opacity = opacity.clamp(0.0, 1.0);
        let (br, bg, bb) = (color[0] as f32, color[1] as f32, color[2] as f32);
        for y in rect.min_y..rect.max_y.min(self.h) {
            let row = (y * self.w) as usize;
            for x in rect.min_x..rect.max_x.min(self.w) {
                let cov = self.cov[row + x as usize];
                let idx = (row + x as usize) * 4;
                if cov == 0 {
                    // Coverage 0 now was always 0 (monotonicity): the canvas
                    // still equals `pre` here. A cap cut is the one thing
                    // that lowers coverage, and it restores `pre` itself
                    // first — see `restore_pre`.
                    continue;
                }
                let a_src = cov as f32 / 65535.0 * opacity;
                let a_pre = pre[idx + 3] as f32 / 255.0;
                let a_out = a_src + a_pre * (1.0 - a_src);
                let dst = &mut canvas.pixels[idx..idx + 4];
                if a_out <= 0.0 {
                    dst.copy_from_slice(&[0, 0, 0, 0]);
                    continue;
                }
                let w_src = a_src / a_out;
                let w_pre = a_pre * (1.0 - a_src) / a_out;
                dst[0] = (br * w_src + pre[idx] as f32 * w_pre).round() as u8;
                dst[1] = (bg * w_src + pre[idx + 1] as f32 * w_pre).round() as u8;
                dst[2] = (bb * w_src + pre[idx + 2] as f32 * w_pre).round() as u8;
                dst[3] = (a_out * 255.0).round() as u8;
            }
        }
    }

    /// Eraser composite: `out_alpha = pre_alpha * (1 - coverage * strength)`,
    /// RGB carried from the snapshot, zeroed when fully erased.
    pub fn composite_erase(
        &self,
        canvas: &mut Canvas,
        pre: &[u8],
        rect: DirtyRect,
        strength: f32,
    ) {
        let strength = strength.clamp(0.0, 1.0);
        for y in rect.min_y..rect.max_y.min(self.h) {
            let row = (y * self.w) as usize;
            for x in rect.min_x..rect.max_x.min(self.w) {
                let cov = self.cov[row + x as usize];
                let idx = (row + x as usize) * 4;
                if cov == 0 {
                    continue;
                }
                let a_pre = pre[idx + 3] as f32 / 255.0;
                let a_out = a_pre * (1.0 - cov as f32 / 65535.0 * strength);
                let a8 = (a_out * 255.0).round() as u8;
                let dst = &mut canvas.pixels[idx..idx + 4];
                if a8 == 0 {
                    dst.copy_from_slice(&[0, 0, 0, 0]);
                } else {
                    dst[0] = pre[idx];
                    dst[1] = pre[idx + 1];
                    dst[2] = pre[idx + 2];
                    dst[3] = a8;
                }
            }
        }
    }

    /// Zero the coverage inside `rect`, ready for it to be redrawn. Only for
    /// re-cutting a stroke's ends: it breaks the monotonicity the composite
    /// relies on, so follow it with `restore_pre` over the same rect.
    pub fn clear_coverage(&mut self, rect: DirtyRect) {
        for y in rect.min_y..rect.max_y.min(self.h) {
            let row = (y * self.w) as usize;
            let (x0, x1) = (rect.min_x.min(self.w), rect.max_x.min(self.w));
            self.cov[row + x0 as usize..row + x1 as usize].fill(0);
        }
    }

    /// Put the pre-stroke pixels back inside `rect`. The composite skips
    /// zero-coverage pixels on the grounds that they were never painted; after
    /// `clear_coverage` that is no longer true, and without this the stroke's
    /// old round ends would stay on the canvas beyond the cut.
    pub fn restore_pre(&self, canvas: &mut Canvas, pre: &[u8], rect: DirtyRect) {
        for y in rect.min_y..rect.max_y.min(self.h) {
            let row = (y * self.w) as usize;
            let a = (row + rect.min_x.min(self.w) as usize) * 4;
            let b = (row + rect.max_x.min(self.w) as usize) * 4;
            canvas.pixels[a..b].copy_from_slice(&pre[a..b]);
        }
    }

    /// Record a touched bbox (inclusive pixel coords) into the stroke rect
    /// and return it as an exclusive-max `DirtyRect`.
    fn touched(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) -> Option<DirtyRect> {
        let r = DirtyRect {
            min_x: x0 as u32,
            min_y: y0 as u32,
            max_x: (x1 + 1) as u32,
            max_y: (y1 + 1) as u32,
        };
        self.stroke_rect = Some(union_rect(self.stroke_rect, r));
        Some(r)
    }
}

/// Union two dirty rects (the accumulator may be empty).
pub fn union_rect(acc: Option<DirtyRect>, r: DirtyRect) -> DirtyRect {
    match acc {
        None => r,
        Some(a) => DirtyRect {
            min_x: a.min_x.min(r.min_x),
            min_y: a.min_y.min(r.min_y),
            max_x: a.max_x.max(r.max_x),
            max_y: a.max_y.max(r.max_y),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(x: f32, y: f32, radius: f32, flow: f32) -> SpineNode {
        SpineNode { x, y, radius, flow }
    }

    /// A ribbon brush with the given edge profile and grain.
    fn ribbon(hardness: f32, grain: f32) -> BrushSettings {
        BrushSettings {
            hardness,
            grain,
            mode: BrushMode::Ribbon,
            ..BrushSettings::default()
        }
    }

    /// A dab brush that stamps at full flow, so build-up is visible.
    fn dabs(flow: f32) -> BrushSettings {
        BrushSettings {
            hardness: 1.0,
            grain: 0.0,
            flow,
            mode: BrushMode::Dab,
            ..BrushSettings::default()
        }
    }

    fn dab(x: f32, y: f32, radius: f32, ratio: f32, angle: f32, flow: f32) -> Dab {
        Dab { x, y, radius, ratio, angle, flow }
    }

    #[test]
    fn zero_length_capsule_matches_dot() {
        let mut a = StrokeWorkspace::new();
        a.begin(64, 64, &ribbon(0.8, 0.0));
        a.raster_capsule(node(32.0, 32.0, 8.0, 1.0), node(32.0, 32.0, 8.0, 1.0));

        let mut b = StrokeWorkspace::new();
        b.begin(64, 64, &ribbon(0.8, 0.0));
        b.raster_dot(node(32.0, 32.0, 8.0, 1.0));

        assert_eq!(a.cov, b.cov);
    }

    #[test]
    fn a_capsule_with_no_cuts_is_unchanged() {
        let (a0, b0) = (node(12.0, 20.0, 6.0, 1.0), node(50.0, 41.0, 3.0, 0.8));
        let mut a = StrokeWorkspace::new();
        a.begin(64, 64, &ribbon(0.7, 0.3));
        a.raster_capsule(a0, b0);
        let mut b = StrokeWorkspace::new();
        b.begin(64, 64, &ribbon(0.7, 0.3));
        b.raster_capsule_clipped(a0, b0, &[]);
        assert_eq!(a.cov, b.cov);
    }

    /// A cut through the middle keeps one side, with a one-pixel edge on
    /// the line itself.
    #[test]
    fn a_cut_removes_everything_beyond_it() {
        let mut ws = StrokeWorkspace::new();
        ws.begin(64, 64, &ribbon(1.0, 0.0));
        let cut = CapPlane { x: 32.0, y: 32.0, ux: 1.0, uy: 0.0 };
        ws.raster_capsule_clipped(node(16.0, 32.0, 8.0, 1.0), node(48.0, 32.0, 8.0, 1.0), &[cut]);
        for y in 26..=38 {
            for x in 0..32 {
                assert_eq!(ws.cov[y * 64 + x], 0, "kept ({x}, {y}) beyond the cut");
            }
        }
        assert_eq!(ws.cov[32 * 64 + 32], 65535, "pixel on the kept side of the line");
        assert_eq!(ws.cov[32 * 64 + 40], 65535);
    }

    #[test]
    fn hard_brush_has_solid_core() {
        // Regression for the airbrush-look defect: at hardness 1 every pixel
        // inside the core must be at full coverage, not just the spine.
        let mut ws = StrokeWorkspace::new();
        ws.begin(64, 64, &ribbon(1.0, 0.0));
        ws.raster_capsule(node(16.0, 32.0, 8.0, 1.0), node(48.0, 32.0, 8.0, 1.0));

        // Sample across the stroke width at x=32: |dy| <= 6 is well inside
        // the core (radius 8, >=1px rim), must be fully solid.
        for dy in -6i32..=6 {
            let y = (32 + dy) as usize;
            let c = ws.cov[y * 64 + 32];
            assert_eq!(c, 65535, "pixel at dy={dy} must be solid, got {c}");
        }
        // Well outside the radius: zero.
        assert_eq!(ws.cov[(32 + 12) * 64 + 32], 0);
    }

    #[test]
    fn grain_is_deterministic_per_canvas_position() {
        let mut a = StrokeWorkspace::new();
        a.begin(64, 64, &ribbon(1.0, 0.5));
        a.raster_dot(node(32.0, 32.0, 8.0, 1.0));
        let first = a.cov.clone();

        // New stroke over the same spot: identical grain pattern.
        a.begin(64, 64, &ribbon(1.0, 0.5));
        a.raster_dot(node(32.0, 32.0, 8.0, 1.0));
        assert_eq!(a.cov, first);

        // Grain actually attenuates some core pixels.
        let center = a.cov[32 * 64 + 32];
        assert!(center < 65535 || a.cov[32 * 64 + 33] < 65535);
    }

    #[test]
    fn max_combine_is_monotonic_and_idempotent() {
        let mut ws = StrokeWorkspace::new();
        ws.begin(64, 64, &ribbon(0.8, 0.0));
        let a = node(10.0, 30.0, 6.0, 1.0);
        let b = node(50.0, 34.0, 6.0, 1.0);
        ws.raster_capsule(a, b);
        let first = ws.cov.clone();
        ws.raster_capsule(a, b);
        assert_eq!(ws.cov, first, "re-rasterizing must not change coverage");
    }

    #[test]
    fn composite_matches_reference_src_over() {
        let mut ws = StrokeWorkspace::new();
        ws.begin(16, 16, &ribbon(1.0, 0.0));
        let mut canvas = Canvas::new(16, 16);
        // Pre: mid-gray at alpha 128.
        for px in canvas.pixels.chunks_exact_mut(4) {
            px.copy_from_slice(&[100, 100, 100, 128]);
        }
        let pre = canvas.pixels.clone();
        ws.raster_dot(node(8.0, 8.0, 4.0, 1.0));
        let rect = ws.stroke_rect.unwrap();
        ws.composite_paint(&mut canvas, &pre, rect, [200, 40, 40, 255], 0.5);

        // Check center pixel against the reference formula.
        let idx = ((8 * 16 + 8) * 4) as usize;
        let cov = ws.cov[8 * 16 + 8] as f32 / 65535.0;
        let a_src = cov * 0.5;
        let a_pre = 128.0 / 255.0;
        let a_out = a_src + a_pre * (1.0 - a_src);
        let expect_r =
            ((200.0 * a_src + 100.0 * a_pre * (1.0 - a_src)) / a_out).round() as u8;
        assert_eq!(canvas.pixels[idx], expect_r);
        assert_eq!(canvas.pixels[idx + 3], (a_out * 255.0).round() as u8);
    }

    #[test]
    fn paints_over_opaque_pixels() {
        // Regression for the old blend_flow defect: semi-transparent paint
        // over an already-opaque layer must tint it.
        let mut ws = StrokeWorkspace::new();
        ws.begin(16, 16, &ribbon(1.0, 0.0));
        let mut canvas = Canvas::new(16, 16);
        for px in canvas.pixels.chunks_exact_mut(4) {
            px.copy_from_slice(&[0, 0, 255, 255]); // opaque blue
        }
        let pre = canvas.pixels.clone();
        ws.raster_dot(node(8.0, 8.0, 5.0, 1.0));
        let rect = ws.stroke_rect.unwrap();
        ws.composite_paint(&mut canvas, &pre, rect, [255, 0, 0, 255], 0.5);

        let idx = ((8 * 16 + 8) * 4) as usize;
        assert!(canvas.pixels[idx] > 60, "red must show through");
        assert!(canvas.pixels[idx + 2] < 255, "blue must be reduced");
        assert_eq!(canvas.pixels[idx + 3], 255, "stays opaque");
    }

    #[test]
    fn erase_reduces_alpha_proportionally() {
        let mut ws = StrokeWorkspace::new();
        ws.begin(16, 16, &ribbon(1.0, 0.0));
        let mut canvas = Canvas::new(16, 16);
        for px in canvas.pixels.chunks_exact_mut(4) {
            px.copy_from_slice(&[50, 60, 70, 200]);
        }
        let pre = canvas.pixels.clone();
        ws.raster_dot(node(8.0, 8.0, 5.0, 1.0));
        let rect = ws.stroke_rect.unwrap();
        ws.composite_erase(&mut canvas, &pre, rect, 0.5);

        let idx = ((8 * 16 + 8) * 4) as usize;
        let cov = ws.cov[8 * 16 + 8] as f32 / 65535.0;
        let expect = (200.0 / 255.0 * (1.0 - cov * 0.5) * 255.0).round() as u8;
        assert_eq!(canvas.pixels[idx + 3], expect);
        assert_eq!(canvas.pixels[idx], 50, "RGB carried from snapshot");
    }

    #[test]
    fn begin_clears_previous_stroke_footprint() {
        let mut ws = StrokeWorkspace::new();
        ws.begin(64, 64, &ribbon(1.0, 0.0));
        ws.raster_dot(node(20.0, 20.0, 6.0, 1.0));
        assert!(ws.cov.iter().any(|&c| c > 0));
        ws.begin(64, 64, &ribbon(1.0, 0.0));
        assert!(ws.cov.iter().all(|&c| c == 0));
    }

    /// Build-up is the whole reason dabs exist: a second stamp on the same
    /// ground must darken it, where a ribbon's `max()` would not.
    #[test]
    fn dabs_build_up_where_a_ribbon_would_not() {
        let mut ws = StrokeWorkspace::new();
        ws.begin(64, 64, &dabs(0.4));
        ws.raster_dab(dab(32.0, 32.0, 8.0, 1.0, 0.0, 0.4));
        let once = ws.cov[32 * 64 + 32];
        ws.raster_dab(dab(32.0, 32.0, 8.0, 1.0, 0.0, 0.4));
        let twice = ws.cov[32 * 64 + 32];

        assert!(once > 0 && twice > once, "{once} -> {twice} is not build-up");
        // a + b(1-a) with a = b = 0.4 is 0.64.
        let expected = (0.64 * 65535.0) as u16;
        assert!(
            (twice as i32 - expected as i32).abs() < 400,
            "expected ~{expected}, got {twice}"
        );
    }

    /// Build-up must saturate rather than wrap, or a long stroke over its own
    /// path would suddenly go transparent.
    #[test]
    fn build_up_saturates() {
        let mut ws = StrokeWorkspace::new();
        ws.begin(64, 64, &dabs(0.9));
        let mut last = 0u16;
        for _ in 0..40 {
            ws.raster_dab(dab(32.0, 32.0, 8.0, 1.0, 0.0, 0.9));
            let c = ws.cov[32 * 64 + 32];
            assert!(c >= last, "coverage went backwards: {last} -> {c}");
            last = c;
        }
        assert!(last >= 65000, "never reached full coverage: {last}");
    }

    /// A flattened dab must cover ground along its major axis and none across
    /// the minor one — that is what puts a tilted pencil on its side.
    #[test]
    fn a_flattened_dab_is_an_ellipse_about_its_angle() {
        let mut ws = StrokeWorkspace::new();
        ws.begin(64, 64, &dabs(1.0));
        // Major axis along x, 12 long; minor 3.
        ws.raster_dab(dab(32.0, 32.0, 12.0, 0.25, 0.0, 1.0));

        assert!(ws.cov[32 * 64 + 41] > 0, "should reach 9px along the major axis");
        assert_eq!(ws.cov[41 * 64 + 32], 0, "must not reach 9px across the minor axis");
        assert!(ws.cov[33 * 64 + 32] > 0, "should cover 1px across the minor axis");
    }

    /// The same dab rotated a quarter turn must cover the transpose of what it
    /// covered before, or the rotation is being applied the wrong way round.
    #[test]
    fn rotating_a_dab_rotates_its_footprint() {
        let mut flat = StrokeWorkspace::new();
        flat.begin(64, 64, &dabs(1.0));
        flat.raster_dab(dab(32.0, 32.0, 12.0, 0.25, 0.0, 1.0));

        let mut turned = StrokeWorkspace::new();
        turned.begin(64, 64, &dabs(1.0));
        turned.raster_dab(dab(32.0, 32.0, 12.0, 0.25, std::f32::consts::FRAC_PI_2, 1.0));

        for y in 0..64usize {
            for x in 0..64usize {
                let a = flat.cov[y * 64 + x];
                let b = turned.cov[x * 64 + y];
                assert!(
                    (a as i32 - b as i32).abs() < 600,
                    "footprint is not the transpose at ({x}, {y}): {a} vs {b}"
                );
            }
        }
    }

    /// Softness must actually soften: raising it has to pull coverage down
    /// inside the dab, not only at the rim.
    #[test]
    fn softness_fades_the_dab_from_inside_its_rim() {
        let sample = |softness: f32| {
            let mut ws = StrokeWorkspace::new();
            ws.begin(
                64,
                64,
                &BrushSettings {
                    hardness: 0.1,
                    softness,
                    grain: 0.0,
                    mode: BrushMode::Dab,
                    ..BrushSettings::default()
                },
            );
            ws.raster_dab(dab(32.0, 32.0, 10.0, 1.0, 0.0, 1.0));
            ws.cov[32 * 64 + 36]
        };
        assert!(
            sample(2.5) < sample(1.0),
            "a softer edge must lay down less at 4px from the centre"
        );
    }

    /// Grain has to be a function of canvas position, not of stroke position:
    /// `max()` and build-up both re-visit pixels, and a grain that moved would
    /// make the result depend on visit order.
    #[test]
    fn grain_follows_the_canvas_not_the_stroke() {
        let brush = BrushSettings {
            hardness: 1.0,
            grain: 0.6,
            grain_scale: 2.0,
            mode: BrushMode::Ribbon,
            ..BrushSettings::default()
        };
        let mut ws = StrokeWorkspace::new();
        ws.begin(64, 64, &brush);
        ws.raster_dot(node(32.0, 32.0, 10.0, 1.0));
        let from_here = ws.cov[30 * 64 + 34];

        // A different stroke that happens to cover the same pixel.
        ws.begin(64, 64, &brush);
        ws.raster_dot(node(28.0, 27.0, 10.0, 1.0));
        assert_eq!(ws.cov[30 * 64 + 34], from_here);
    }
}
