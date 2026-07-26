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

/// Erase everything inside the closed polygon `pts`, given in the cell's own
/// pixel space. Returns `false` when the path is degenerate (under 3 points, or
/// entirely off-canvas) or nothing changed.
pub fn erase(canvas: &mut Canvas, pts: &[(f32, f32)]) -> bool {
    if pts.len() < 3 {
        return false;
    }
    let w = canvas.width as i32;
    let h = canvas.height as i32;

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
        return false;
    }

    // One row of coverage at a time — element 0 is canvas column `x0`.
    let mut cov = vec![0f32; (x1 - x0) as usize];
    let mut xs: Vec<(f32, i32)> = Vec::new();
    let weight = 1.0 / SUB as f32;
    let mut touched = false;

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

        for (i, &c) in cov.iter().enumerate() {
            if c <= 0.0 {
                continue;
            }
            let idx = ((py * w + x0 + i as i32) * 4) as usize;
            let a_pre = canvas.pixels[idx + 3];
            if a_pre == 0 {
                continue;
            }
            let a_out = ((a_pre as f32 / 255.0) * (1.0 - c.min(1.0)) * 255.0).round() as u8;
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
        canvas.mark_dirty(x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32);
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
}
