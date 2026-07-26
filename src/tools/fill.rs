//! Flood fill tool — scanline-based, alpha + colour tolerance bounded.
//!
//! Algorithm: standard scanline span flood (Smith). Compares against the
//! sampled pixel at click point; expands while neighbouring pixels are within
//! tolerance and the slot is reachable.
//!
//! The region is computed as a mask first, then written in a second pass. That
//! split is what lets the *boundary* pixels come from a different canvas than
//! the one being painted — the animation workflow where line art lives on one
//! layer and flat colour on another.

use std::collections::VecDeque;

use crate::doc::canvas::Canvas;

/// Upper bound on the expand radius, matching the UI slider.
const MAX_EXPAND: i32 = 8;

#[derive(Clone, Copy)]
pub struct FillOptions {
    /// 0..=255 per-channel tolerance.
    pub tolerance: u8,
    /// Fill colour (RGBA8 unmultiplied).
    pub color: [u8; 4],
    /// Grow the filled region by this many pixels after the flood so the colour
    /// tucks under anti-aliased lines. 0 = off.
    pub expand: u8,
}

/// Flood fill on `canvas` starting at integer pixel `(x, y)` using `opts`.
///
/// When `boundary` is `Some`, the region is grown by looking at *those* pixels
/// instead of `canvas`'s own — the line art that walls the fill in lives on
/// another layer. It must match `canvas`'s dimensions. `None` is the plain
/// bucket: read and write the same pixels.
///
/// Note that with `expand > 0` the grown ring overwrites whatever is already on
/// `canvas` within that many pixels of the region edge. That is the intended
/// trade — the overwritten band normally sits underneath the line art.
pub fn flood(canvas: &mut Canvas, boundary: Option<&Canvas>, x: i32, y: i32, opts: FillOptions) {
    if x < 0 || y < 0 || x >= canvas.width as i32 || y >= canvas.height as i32 {
        return;
    }
    let w = canvas.width as i32;
    let h = canvas.height as i32;
    if let Some(b) = boundary {
        if b.width != canvas.width || b.height != canvas.height {
            return;
        }
    }

    // Region pass — reads only, so the source may be `canvas` itself. The
    // borrow ends with this statement; `mask` is owned.
    let Some((mut mask, mut bbox)) = ({
        let src = boundary.unwrap_or(&*canvas);
        // Without a boundary, filling a region that already *is* the fill
        // colour is a no-op. With one, the sampled colour comes from a
        // different layer, so it says nothing about what is already painted.
        if boundary.is_none() && px_eq(read_px(src, x, y), opts.color) {
            None
        } else {
            Some(region(src, x, y, opts.tolerance))
        }
    }) else {
        return;
    };

    let r = (opts.expand as i32).min(MAX_EXPAND);
    if r > 0 {
        dilate(&mut mask, w, h, r);
        bbox = (
            (bbox.0 - r).max(0),
            (bbox.1 - r).max(0),
            (bbox.2 + r).min(w - 1),
            (bbox.3 + r).min(h - 1),
        );
    }

    // Write pass.
    for yi in bbox.1..=bbox.3 {
        let row = yi * w;
        for xi in bbox.0..=bbox.2 {
            if mask[(row + xi) as usize] {
                write_px(canvas, xi, yi, opts.color);
            }
        }
    }

    canvas.mark_dirty(
        bbox.0 as u32,
        bbox.1 as u32,
        (bbox.2 - bbox.0 + 1) as u32,
        (bbox.3 - bbox.1 + 1) as u32,
    );
}

/// Scanline span flood over `src` from `(x, y)`. Returns the reached pixels as
/// a `w * h` bitmap plus their bounding box as `(min_x, min_y, max_x, max_y)`.
fn region(src: &Canvas, x: i32, y: i32, tol: u8) -> (Vec<bool>, (i32, i32, i32, i32)) {
    let w = src.width as i32;
    let h = src.height as i32;
    let target = read_px(src, x, y);

    let mut visited = vec![false; (w * h) as usize];
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
    queue.push_back((x, y));

    let mut min_x = x;
    let mut min_y = y;
    let mut max_x = x;
    let mut max_y = y;

    while let Some((sx, sy)) = queue.pop_front() {
        // Walk left to span start.
        let mut x0 = sx;
        while x0 > 0 && matches_target(src, x0 - 1, sy, target, tol) {
            x0 -= 1;
        }
        // Walk right to span end.
        let mut x1 = sx;
        while x1 + 1 < w && matches_target(src, x1 + 1, sy, target, tol) {
            x1 += 1;
        }

        // Mark the span and seed neighbouring rows.
        let row = sy * w;
        let mut span_above_open = false;
        let mut span_below_open = false;
        for xi in x0..=x1 {
            let idx = (row + xi) as usize;
            if visited[idx] {
                continue;
            }
            visited[idx] = true;

            if sy > 0 {
                let above_match = matches_target(src, xi, sy - 1, target, tol);
                if above_match && !span_above_open {
                    queue.push_back((xi, sy - 1));
                    span_above_open = true;
                } else if !above_match {
                    span_above_open = false;
                }
            }
            if sy + 1 < h {
                let below_match = matches_target(src, xi, sy + 1, target, tol);
                if below_match && !span_below_open {
                    queue.push_back((xi, sy + 1));
                    span_below_open = true;
                } else if !below_match {
                    span_below_open = false;
                }
            }
        }

        min_x = min_x.min(x0);
        max_x = max_x.max(x1);
        min_y = min_y.min(sy);
        max_y = max_y.max(sy);
    }

    (visited, (min_x, min_y, max_x, max_y))
}

/// Grow `mask` by `r` pixels with a square kernel, applied separably
/// (horizontal pass, then vertical) so the cost is O(w * h * r) rather than
/// O(w * h * r²).
fn dilate(mask: &mut [bool], w: i32, h: i32, r: i32) {
    let mut tmp = vec![false; mask.len()];
    // Horizontal.
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let lo = (x - r).max(0);
            let hi = (x + r).min(w - 1);
            tmp[(row + x) as usize] =
                (lo..=hi).any(|xi| mask[(row + xi) as usize]);
        }
    }
    // Vertical.
    for y in 0..h {
        let lo = (y - r).max(0);
        let hi = (y + r).min(h - 1);
        for x in 0..w {
            mask[(y * w + x) as usize] = (lo..=hi).any(|yi| tmp[(yi * w + x) as usize]);
        }
    }
}

#[inline]
fn read_px(canvas: &Canvas, x: i32, y: i32) -> [u8; 4] {
    let idx = ((y as u32 * canvas.width + x as u32) * 4) as usize;
    [
        canvas.pixels[idx],
        canvas.pixels[idx + 1],
        canvas.pixels[idx + 2],
        canvas.pixels[idx + 3],
    ]
}

#[inline]
fn write_px(canvas: &mut Canvas, x: i32, y: i32, p: [u8; 4]) {
    let idx = ((y as u32 * canvas.width + x as u32) * 4) as usize;
    canvas.pixels[idx] = p[0];
    canvas.pixels[idx + 1] = p[1];
    canvas.pixels[idx + 2] = p[2];
    canvas.pixels[idx + 3] = p[3];
}

#[inline]
fn matches_target(canvas: &Canvas, x: i32, y: i32, target: [u8; 4], tol: u8) -> bool {
    let p = read_px(canvas, x, y);
    let t = tol as i32;
    // When filling transparent space, ignore RGB — only check alpha.
    // This consumes anti-aliased stroke edge pixels so the fill
    // reaches the solid stroke without a visible gap.
    if target[3] < 8 {
        return p[3] as i32 <= 128;
    }
    (p[0] as i32 - target[0] as i32).abs() <= t
        && (p[1] as i32 - target[1] as i32).abs() <= t
        && (p[2] as i32 - target[2] as i32).abs() <= t
        && (p[3] as i32 - target[3] as i32).abs() <= t
}

#[inline]
fn px_eq(a: [u8; 4], b: [u8; 4]) -> bool {
    a[0] == b[0] && a[1] == b[1] && a[2] == b[2] && a[3] == b[3]
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];

    fn opts(expand: u8) -> FillOptions {
        FillOptions {
            tolerance: 24,
            color: RED,
            expand,
        }
    }

    /// 16x16 transparent canvas with an opaque black wall down column 5.
    fn walled() -> Canvas {
        let mut c = Canvas::new(16, 16);
        for y in 0..16 {
            write_px(&mut c, 5, y, [0, 0, 0, 255]);
        }
        c
    }

    fn filled_at(c: &Canvas, x: i32, y: i32) -> bool {
        read_px(c, x, y) == RED
    }

    #[test]
    fn boundary_layer_walls_the_fill() {
        let boundary = walled();
        let mut target = Canvas::new(16, 16);
        flood(&mut target, Some(&boundary), 2, 8, opts(0));

        for y in 0..16 {
            for x in 0..16 {
                let want = x < 5;
                assert_eq!(
                    filled_at(&target, x, y),
                    want,
                    "pixel ({x},{y}) should {}be filled",
                    if want { "" } else { "not " }
                );
            }
        }
    }

    #[test]
    fn no_boundary_floods_everything() {
        let mut target = Canvas::new(16, 16);
        flood(&mut target, None, 2, 8, opts(0));
        for y in 0..16 {
            for x in 0..16 {
                assert!(filled_at(&target, x, y), "pixel ({x},{y}) should be filled");
            }
        }
    }

    #[test]
    fn boundary_dimensions_must_match() {
        let boundary = Canvas::new(8, 8);
        let mut target = Canvas::new(16, 16);
        flood(&mut target, Some(&boundary), 2, 8, opts(0));
        assert!(!filled_at(&target, 2, 8), "mismatched boundary must no-op");
    }

    #[test]
    fn expand_grows_region_under_the_wall() {
        let boundary = walled();
        let mut target = Canvas::new(16, 16);
        flood(&mut target, Some(&boundary), 2, 8, opts(2));

        // The fill now reaches into and past the wall columns.
        assert!(filled_at(&target, 5, 8), "expand should reach the wall");
        assert!(filled_at(&target, 6, 8), "expand should tuck under the wall");
        // But only by the expand radius.
        assert!(!filled_at(&target, 7, 8), "expand must stop at radius 2");
    }

    #[test]
    fn expand_clamps_to_canvas_edges() {
        let boundary = walled();
        let mut target = Canvas::new(16, 16);
        // Must not panic or wrap rows when the region touches the border.
        flood(&mut target, Some(&boundary), 0, 0, opts(8));
        assert!(filled_at(&target, 0, 15), "left column stays filled");
        assert!(!filled_at(&target, 15, 8), "no wrap past the wall to the far edge");
    }

    #[test]
    fn expand_zero_matches_unexpanded() {
        let boundary = walled();
        let mut a = Canvas::new(16, 16);
        let mut b = Canvas::new(16, 16);
        flood(&mut a, Some(&boundary), 2, 8, opts(0));
        flood(&mut b, Some(&boundary), 2, 8, opts(0));
        assert_eq!(a.pixels, b.pixels);
    }

    #[test]
    fn same_colour_refill_is_a_noop_without_boundary() {
        let mut target = Canvas::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                write_px(&mut target, x, y, RED);
            }
        }
        target.dirty = None;
        flood(&mut target, None, 1, 1, opts(0));
        assert!(target.dirty.is_none(), "no-op fill must not dirty the cell");
    }
}
