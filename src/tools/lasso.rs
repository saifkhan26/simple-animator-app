//! Lasso erase — freehand polygon selection; everything inside is deleted.
//!
//! The drawn path is closed implicitly (last point back to the first) and
//! filled with the **nonzero winding** rule, so a loop that crosses itself
//! still erases solid instead of punching an even-odd hole in the middle.
//!
//! Coverage is computed by scanline: `SUB` sample rows per pixel row, each
//! intersected against every edge, the resulting spans accumulated with
//! fractional coverage at the ends. That anti-aliases the boundary for the same
//! reason the ribbon rasterizer supersamples — a hard 1-bit edge reads as
//! stair-stepping next to anti-aliased line art.
//!
//! Erase semantics match [`crate::tools::ribbon::StrokeWorkspace::composite_erase`]:
//! alpha scales by `1 - coverage`, RGB is left alone, and a fully erased pixel
//! is zeroed so it carries no stale colour.

use crate::doc::canvas::Canvas;

/// Sample rows per pixel row. 4 hides stepping on near-horizontal edges and is
/// cheap enough to run in one shot on pointer-up.
const SUB: i32 = 4;

/// Per-pixel coverage of a closed polygon, as a bounding box plus one byte per
/// pixel inside it (0 = outside, 255 = fully inside).
///
/// This is the shape of a selection: `erase` weights alpha by it, a lift copies
/// pixels through it, and a stamp composites with it.
#[derive(Clone, Debug)]
pub struct Mask {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Row-major, `w * h` bytes.
    pub cov: Vec<u8>,
}

impl Mask {
    /// Coverage at a canvas pixel, or 0 outside the bounding box.
    pub fn at(&self, x: u32, y: u32) -> u8 {
        if x < self.x || y < self.y || x >= self.x + self.w || y >= self.y + self.h {
            return 0;
        }
        self.cov[((y - self.y) * self.w + (x - self.x)) as usize]
    }

    /// True where the mask actually covers something — the hit test for
    /// "did the user press inside the selection".
    pub fn contains(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 {
            return false;
        }
        self.at(x as u32, y as u32) > 0
    }
}

/// Rasterise the closed polygon `pts` (in the cell's own pixel space) into a
/// coverage mask. `None` when the path is degenerate — under 3 points, or
/// entirely off-canvas.
///
/// Scanline with `SUB` sample rows per pixel row and nonzero winding, so a path
/// that crosses itself stays solid instead of punching an even-odd hole.
pub fn coverage(pts: &[(f32, f32)], cw: u32, ch: u32) -> Option<Mask> {
    if pts.len() < 3 {
        return None;
    }
    let w = cw as i32;
    let h = ch as i32;

    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
    for &(x, y) in pts {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let x0 = (min_x.floor() as i32).clamp(0, w);
    let y0 = (min_y.floor() as i32).clamp(0, h);
    let x1 = (max_x.ceil() as i32 + 1).clamp(0, w);
    let y1 = (max_y.ceil() as i32 + 1).clamp(0, h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }

    let (mw, mh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut out = vec![0u8; mw * mh];
    // One row of coverage at a time — element 0 is canvas column `x0`.
    let mut cov = vec![0f32; mw];
    let mut xs: Vec<(f32, i32)> = Vec::new();
    let weight = 1.0 / SUB as f32;

    for py in y0..y1 {
        cov.iter_mut().for_each(|c| *c = 0.0);

        for s in 0..SUB {
            let sy = py as f32 + (s as f32 + 0.5) / SUB as f32;
            xs.clear();
            for i in 0..pts.len() {
                let (ax, ay) = pts[i];
                let (bx, by) = pts[(i + 1) % pts.len()];
                if ay == by {
                    continue;
                }
                // Half-open in y: a sample row landing exactly on a shared
                // vertex is counted by one edge only, never zero or twice.
                let (lo, hi, dir) = if ay < by { (ay, by, 1) } else { (by, ay, -1) };
                if sy < lo || sy >= hi {
                    continue;
                }
                let t = (sy - ay) / (by - ay);
                xs.push((ax + t * (bx - ax), dir));
            }
            if xs.len() < 2 {
                continue;
            }
            xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

            // Nonzero winding: inside wherever the running direction sum != 0.
            let mut wind = 0;
            let mut span_start = 0.0f32;
            for &(x, dir) in xs.iter() {
                if wind == 0 {
                    span_start = x;
                }
                wind += dir;
                if wind == 0 {
                    add_span(&mut cov, x0, span_start, x, weight);
                }
            }
        }

        let row = (py - y0) as usize * mw;
        for (i, &c) in cov.iter().enumerate() {
            if c <= 0.0 {
                continue;
            }
            out[row + i] = (c.min(1.0) * 255.0).round() as u8;
        }
    }

    Some(Mask {
        x: x0 as u32,
        y: y0 as u32,
        w: mw as u32,
        h: mh as u32,
        cov: out,
    })
}

/// Erase through an existing mask: alpha scales by `1 - coverage`, RGB is left
/// alone, and a fully erased pixel is zeroed so it carries no stale colour.
pub fn erase_masked(canvas: &mut Canvas, mask: &Mask) -> bool {
    let w = canvas.width;
    let mut touched = false;
    for my in 0..mask.h {
        let py = mask.y + my;
        if py >= canvas.height {
            break;
        }
        for mx in 0..mask.w {
            let px = mask.x + mx;
            if px >= w {
                break;
            }
            let c = mask.cov[(my * mask.w + mx) as usize];
            if c == 0 {
                continue;
            }
            let idx = ((py * w + px) * 4) as usize;
            let a_pre = canvas.pixels[idx + 3];
            if a_pre == 0 {
                continue;
            }
            let a_out =
                ((a_pre as f32 / 255.0) * (1.0 - c as f32 / 255.0) * 255.0).round() as u8;
            if a_out == a_pre {
                continue;
            }
            if a_out == 0 {
                canvas.pixels[idx..idx + 4].copy_from_slice(&[0, 0, 0, 0]);
            } else {
                canvas.pixels[idx + 3] = a_out;
            }
            touched = true;
        }
    }
    if touched {
        canvas.mark_dirty(mask.x, mask.y, mask.w, mask.h);
    }
    touched
}

/// Accumulate horizontal coverage for the span `[xa, xb)` into `cov`, whose
/// element 0 is canvas column `x0`. Partially covered end pixels get their
/// fractional overlap, which is what anti-aliases the polygon edge.
fn add_span(cov: &mut [f32], x0: i32, xa: f32, xb: f32, weight: f32) {
    let lo = xa.max(x0 as f32);
    let hi = xb.min(x0 as f32 + cov.len() as f32);
    if hi <= lo {
        return;
    }
    let i0 = lo.floor() as i32 - x0;
    let i1 = (hi.ceil() as i32 - 1) - x0;
    for i in i0..=i1 {
        let px_lo = (x0 + i) as f32;
        let overlap = (hi.min(px_lo + 1.0) - lo.max(px_lo)).max(0.0);
        cov[i as usize] += overlap * weight;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pre-selection entry point: rasterise a path and erase through it.
    /// The tool now splits these two steps so the mask can outlive the erase,
    /// but the composed behaviour is still what these tests pin down.
    fn erase(canvas: &mut Canvas, pts: &[(f32, f32)]) -> bool {
        match coverage(pts, canvas.width, canvas.height) {
            Some(mask) => erase_masked(canvas, &mask),
            None => false,
        }
    }

    /// 16x16 fully opaque white.
    fn solid() -> Canvas {
        let mut c = Canvas::new(16, 16);
        for px in c.pixels.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 255, 255, 255]);
        }
        c.dirty = None;
        c
    }

    fn alpha(c: &Canvas, x: u32, y: u32) -> u8 {
        c.pixels[((y * c.width + x) * 4 + 3) as usize]
    }

    /// Axis-aligned rectangle path, corners on pixel boundaries.
    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<(f32, f32)> {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    #[test]
    fn erases_inside_and_spares_outside() {
        let mut c = solid();
        assert!(erase(&mut c, &rect(4.0, 4.0, 12.0, 12.0)));
        for y in 0..16 {
            for x in 0..16 {
                let inside = (4..12).contains(&x) && (4..12).contains(&y);
                let a = alpha(&c, x, y);
                if inside {
                    assert_eq!(a, 0, "({x},{y}) inside the lasso must be erased");
                } else {
                    assert_eq!(a, 255, "({x},{y}) outside the lasso must be untouched");
                }
            }
        }
    }

    #[test]
    fn erase_zeroes_rgb_so_no_stale_colour_remains() {
        let mut c = solid();
        erase(&mut c, &rect(4.0, 4.0, 12.0, 12.0));
        let idx = ((8 * 16 + 8) * 4) as usize;
        assert_eq!(&c.pixels[idx..idx + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn partial_edge_coverage_is_antialiased() {
        let mut c = solid();
        // Right edge lands mid-pixel: column 8 is half covered.
        erase(&mut c, &rect(4.0, 4.0, 8.5, 12.0));
        let a = alpha(&c, 8, 8);
        assert!(a > 100 && a < 160, "half-covered edge pixel got alpha {a}");
        assert_eq!(alpha(&c, 7, 8), 0, "fully covered column still erased");
    }

    #[test]
    fn overlapping_loop_stays_solid() {
        let mut c = solid();
        // The same rectangle walked twice, so every interior point has winding
        // 2. Even-odd would read that as *outside* and erase nothing; nonzero
        // must erase it solid. This is the rule difference, in one path.
        let mut pts = rect(4.0, 4.0, 12.0, 12.0);
        pts.extend(rect(4.0, 4.0, 12.0, 12.0));
        assert!(erase(&mut c, &pts));
        assert_eq!(alpha(&c, 8, 8), 0, "double-wound interior must be erased");
        assert_eq!(alpha(&c, 2, 2), 255, "outside still untouched");
    }

    #[test]
    fn degenerate_paths_are_a_noop() {
        let mut c = solid();
        assert!(!erase(&mut c, &[(1.0, 1.0), (2.0, 2.0)]), "under 3 points");
        assert!(c.dirty.is_none(), "no-op must not dirty the cell");
    }

    #[test]
    fn offscreen_path_is_a_noop() {
        let mut c = solid();
        assert!(!erase(&mut c, &rect(-40.0, -40.0, -20.0, -20.0)));
        assert!(c.dirty.is_none());
        assert_eq!(alpha(&c, 0, 0), 255);
    }

    #[test]
    fn path_clipped_at_the_canvas_edge_does_not_wrap() {
        let mut c = solid();
        // Extends past the left/top edges; must clamp, not wrap onto row above.
        erase(&mut c, &rect(-8.0, -8.0, 4.0, 4.0));
        assert_eq!(alpha(&c, 0, 0), 0, "clipped region still erased");
        assert_eq!(alpha(&c, 15, 0), 255, "no wrap to the far end of the row");
    }

    #[test]
    fn coverage_bounds_the_path_and_anti_aliases_its_edge() {
        // A 4x4 square from (2,2) to (6,6) inside an 8x8 cell.
        let sq = [(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)];
        let m = coverage(&sq, 8, 8).expect("mask");
        assert!(m.x <= 2 && m.y <= 2);
        assert!(m.x + m.w >= 6 && m.y + m.h >= 6);
        // Solid in the middle, empty outside.
        assert_eq!(m.at(3, 3), 255);
        assert_eq!(m.at(0, 0), 0);
        assert_eq!(m.at(7, 7), 0);
        assert!(m.contains(3, 3));
        assert!(!m.contains(0, 0));
        assert!(!m.contains(-1, 3));
    }

    #[test]
    fn coverage_rejects_degenerate_and_offscreen_paths() {
        assert!(coverage(&[(0.0, 0.0), (1.0, 1.0)], 8, 8).is_none());
        assert!(coverage(&[(20.0, 20.0), (30.0, 20.0), (30.0, 30.0)], 8, 8).is_none());
    }
}
