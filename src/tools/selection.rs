//! Floating pixel selection — the movable half of the lasso.
//!
//! A selection is lifted out of one cell: its pixels are copied through the
//! lasso's coverage mask, and the source is erased through the same mask, so
//! the two halves still sum to the original drawing. Until it is committed it
//! floats at an integer `offset`, drawn as a textured quad rather than written
//! back into the cell.
//!
//! Offsets are integers on purpose. A fractional offset would have to resample
//! on every drag, and repeated resampling softens line art a little more each
//! time — a move should be lossless.

use crate::doc::canvas::Canvas;
use crate::doc::layer::CellId;
use crate::tools::lasso::Mask;

#[derive(Clone)]
pub struct Selection {
    /// Cell the pixels were lifted from. A selection never outlives its cell:
    /// changing frame or layer commits it first.
    pub cell: CellId,
    pub mask: Mask,
    /// RGBA of the masked region, `mask.w * mask.h * 4`, straight alpha with
    /// the mask coverage already folded in.
    pub pixels: Vec<u8>,
    /// Displacement from where it was lifted, in cell pixels.
    pub offset: (i32, i32),
    /// The lasso path, kept in cell space to draw the outline.
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
            offset: (0, 0),
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

    /// Top-left of the floating pixels in cell space, with the offset applied.
    pub fn origin(&self) -> (i32, i32) {
        (
            self.mask.x as i32 + self.offset.0,
            self.mask.y as i32 + self.offset.1,
        )
    }

    /// True when cell-space point `(x, y)` lands on covered pixels — the test
    /// for "is this drag a move, or a new lasso".
    pub fn hit(&self, x: f32, y: f32) -> bool {
        let (ox, oy) = self.origin();
        self.mask.contains(
            x.floor() as i32 - ox + self.mask.x as i32,
            y.floor() as i32 - oy + self.mask.y as i32,
        )
    }

    /// Composite the floating pixels back into `canvas` at the current offset.
    /// Straight-alpha src-over, matching every other compositor in the app.
    pub fn stamp(&self, canvas: &mut Canvas) {
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
                let d = ((py * cw + px) * 4) as usize;
                let da = canvas.pixels[d + 3] as f32 / 255.0;
                let out_a = sa + da * (1.0 - sa);
                if out_a <= 0.0 {
                    canvas.pixels[d..d + 4].copy_from_slice(&[0, 0, 0, 0]);
                    continue;
                }
                for c in 0..3 {
                    let sv = self.pixels[s + c] as f32 / 255.0;
                    let dv = canvas.pixels[d + c] as f32 / 255.0;
                    let ov = (sv * sa + dv * da * (1.0 - sa)) / out_a;
                    canvas.pixels[d + c] = (ov * 255.0).round().clamp(0.0, 255.0) as u8;
                }
                canvas.pixels[d + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
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

    #[test]
    fn lift_then_stamp_in_place_restores_the_drawing() {
        let mut canvas = filled(8, 8, [10, 20, 30, 255]);
        let before = canvas.pixels.clone();
        let path = square(2.0, 2.0, 6.0, 6.0);
        let mask = lasso::coverage(&path, 8, 8).unwrap();
        let mut sel = Selection::new(0, &canvas, mask, path);

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
        let path = square(1.0, 1.0, 3.0, 3.0);
        let mask = lasso::coverage(&path, 8, 8).unwrap();
        let mut sel = Selection::new(0, &canvas, mask, path);
        sel.lift_source(&mut canvas);
        sel.offset = (4, 4);
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
        let path = square(1.0, 1.0, 3.0, 3.0);
        let mask = lasso::coverage(&path, 8, 8).unwrap();
        let mut sel = Selection::new(0, &canvas, mask, path);
        sel.lift_source(&mut canvas);
        sel.offset = (-40, 0);
        sel.stamp(&mut canvas);
        // Nothing reappeared on the far side.
        let right = ((2 * 8 + 7) * 4) as usize;
        assert_eq!(canvas.pixels[right + 3], 255);
    }

    #[test]
    fn hit_testing_follows_the_offset() {
        let canvas = filled(8, 8, [0, 0, 0, 255]);
        let path = square(1.0, 1.0, 4.0, 4.0);
        let mask = lasso::coverage(&path, 8, 8).unwrap();
        let mut sel = Selection::new(0, &canvas, mask, path);
        assert!(sel.hit(2.0, 2.0));
        assert!(!sel.hit(6.0, 6.0));
        sel.offset = (4, 4);
        assert!(!sel.hit(2.0, 2.0));
        assert!(sel.hit(6.0, 6.0));
    }
}
