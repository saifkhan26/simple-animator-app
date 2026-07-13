//! Ribbon stroke rasterizer.
//!
//! A stroke is a polyline spine of `SpineNode`s (position + pressure-modulated
//! radius + flow). Each spine segment is rasterized as a variable-radius
//! capsule into a per-stroke coverage buffer combined with `max()`, then the
//! coverage is composited src-over the pre-stroke snapshot into the canvas.
//!
//! Compared to dab stamping this writes each stroke pixel ~1-2 times instead
//! of ~1/spacing times, produces uniform per-stroke opacity (no darkening at
//! joints or self-crossings), and composites against the pre-stroke pixels so
//! painting semi-transparently over already-opaque content works.
//!
//! The rasterize/composite split is deliberate: a wgpu compute port replaces
//! the internals of this module without touching the stroke input model.

use crate::doc::canvas::{Canvas, DirtyRect};

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
    /// Paper-grain strength, 0..=1: how deeply canvas-position noise eats
    /// into the coverage (0 = smooth ink).
    grain: f32,
}

impl StrokeWorkspace {
    pub fn new() -> Self {
        Self {
            cov: Vec::new(),
            w: 0,
            h: 0,
            stroke_rect: None,
            hardness: 1.0,
            grain: 0.0,
        }
    }

    /// Prepare for a new stroke: size the coverage buffer to the target
    /// canvas, zero the previous stroke's footprint, latch brush profile.
    pub fn begin(&mut self, w: u32, h: u32, hardness: f32, grain: f32) {
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
        self.hardness = hardness.clamp(0.0, 1.0);
        self.grain = grain.clamp(0.0, 1.0);
    }

    /// Falloff mask for distance `d` from the spine given the local outer
    /// radius: fully solid core, smoothstep rim of width
    /// `(1 - hardness) * outer`, never narrower than 1px (anti-aliasing).
    #[inline]
    fn mask(&self, d: f32, outer: f32) -> f32 {
        let w = ((1.0 - self.hardness) * outer).max(1.0);
        let s = ((outer - d) / w).clamp(0.0, 1.0);
        s * s * (3.0 - 2.0 * s)
    }

    /// Grain-attenuated coverage for a pixel. Noise is a pure hash of the
    /// canvas position (paper texture): deterministic, so overlapping
    /// strokes and `max()` re-rasterization see identical values.
    #[inline]
    fn pixel_cov(&self, mask: f32, flow: f32, x: u32, y: u32) -> u16 {
        let mut cov = mask * flow;
        if self.grain > 0.0 {
            cov *= 1.0 - self.grain * grain_hash(x, y);
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
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len2 = dx * dx + dy * dy;
        if len2 < 1e-6 {
            let n = if a.radius >= b.radius { a } else { b };
            return self.raster_dot(n);
        }
        let inv_len2 = 1.0 / len2;
        let dr = b.radius - a.radius;
        let dflow = b.flow - a.flow;

        let pad = a.radius.max(b.radius) + AA + 1.0;
        let x0 = ((a.x.min(b.x) - pad).floor() as i32).max(0);
        let y0 = ((a.y.min(b.y) - pad).floor() as i32).max(0);
        let x1 = ((a.x.max(b.x) + pad).ceil() as i32).min(self.w as i32 - 1);
        let y1 = ((a.y.max(b.y) + pad).ceil() as i32).min(self.h as i32 - 1);
        if x1 < x0 || y1 < y0 {
            return None;
        }

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
                let mask = self.mask(d2.sqrt(), outer);
                let c16 = self.pixel_cov(mask, a.flow + t * dflow, px as u32, py as u32);
                let cell = &mut self.cov[row + px as usize];
                if c16 > *cell {
                    *cell = c16;
                }
            }
        }

        self.touched(x0, y0, x1, y1)
    }

    /// Rasterize a single dot (tap / first sample): a plain disc, the
    /// degenerate case of `raster_capsule`.
    pub fn raster_dot(&mut self, n: SpineNode) -> Option<DirtyRect> {
        let pad = n.radius + AA + 1.0;
        let x0 = ((n.x - pad).floor() as i32).max(0);
        let y0 = ((n.y - pad).floor() as i32).max(0);
        let x1 = ((n.x + pad).ceil() as i32).min(self.w as i32 - 1);
        let y1 = ((n.y + pad).ceil() as i32).min(self.h as i32 - 1);
        if x1 < x0 || y1 < y0 {
            return None;
        }
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
                let mask = self.mask(d2.sqrt(), outer);
                let c16 = self.pixel_cov(mask, n.flow, px as u32, py as u32);
                let cell = &mut self.cov[row + px as usize];
                if c16 > *cell {
                    *cell = c16;
                }
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
                    // still equals `pre` here.
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

/// Cheap integer-mix hash of a canvas position, 0..=1. Used as fixed paper
/// grain so pencil texture is stable across strokes and frames.
#[inline]
fn grain_hash(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 65535.0
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

    #[test]
    fn zero_length_capsule_matches_dot() {
        let mut a = StrokeWorkspace::new();
        a.begin(64, 64, 0.8, 0.0);
        a.raster_capsule(node(32.0, 32.0, 8.0, 1.0), node(32.0, 32.0, 8.0, 1.0));

        let mut b = StrokeWorkspace::new();
        b.begin(64, 64, 0.8, 0.0);
        b.raster_dot(node(32.0, 32.0, 8.0, 1.0));

        assert_eq!(a.cov, b.cov);
    }

    #[test]
    fn hard_brush_has_solid_core() {
        // Regression for the airbrush-look defect: at hardness 1 every pixel
        // inside the core must be at full coverage, not just the spine.
        let mut ws = StrokeWorkspace::new();
        ws.begin(64, 64, 1.0, 0.0);
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
        a.begin(64, 64, 1.0, 0.5);
        a.raster_dot(node(32.0, 32.0, 8.0, 1.0));
        let first = a.cov.clone();

        // New stroke over the same spot: identical grain pattern.
        a.begin(64, 64, 1.0, 0.5);
        a.raster_dot(node(32.0, 32.0, 8.0, 1.0));
        assert_eq!(a.cov, first);

        // Grain actually attenuates some core pixels.
        let center = a.cov[32 * 64 + 32];
        assert!(center < 65535 || a.cov[32 * 64 + 33] < 65535);
    }

    #[test]
    fn max_combine_is_monotonic_and_idempotent() {
        let mut ws = StrokeWorkspace::new();
        ws.begin(64, 64, 0.8, 0.0);
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
        ws.begin(16, 16, 1.0, 0.0);
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
        ws.begin(16, 16, 1.0, 0.0);
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
        ws.begin(16, 16, 1.0, 0.0);
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
        ws.begin(64, 64, 1.0, 0.0);
        ws.raster_dot(node(20.0, 20.0, 6.0, 1.0));
        assert!(ws.cov.iter().any(|&c| c > 0));
        ws.begin(64, 64, 1.0, 0.0);
        assert!(ws.cov.iter().all(|&c| c == 0));
    }
}
