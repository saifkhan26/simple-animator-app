//! Flood fill tool — scanline-based, alpha + colour tolerance bounded.
//!
//! Algorithm: standard scanline span flood (Smith). Compares against the
//! sampled pixel at click point; expands while neighbouring pixels are within
//! tolerance and the slot is reachable.

use std::collections::VecDeque;

use crate::doc::canvas::Canvas;

#[derive(Clone, Copy)]
pub struct FillOptions {
    /// 0..=255 per-channel tolerance.
    pub tolerance: u8,
    /// Fill colour (RGBA8 unmultiplied).
    pub color: [u8; 4],
}

/// Flood fill on `canvas` starting at integer pixel `(x, y)` using `opts`.
pub fn flood(canvas: &mut Canvas, x: i32, y: i32, opts: FillOptions) {
    if x < 0 || y < 0 || x >= canvas.width as i32 || y >= canvas.height as i32 {
        return;
    }
    let w = canvas.width as i32;
    let h = canvas.height as i32;
    let target = read_px(canvas, x, y);

    // No-op when target already matches the fill colour exactly.
    if px_eq(target, opts.color) {
        return;
    }

    // Track visited pixels with a bitmap to avoid re-pushing.
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
        while x0 > 0 && matches_target(canvas, x0 - 1, sy, target, opts.tolerance) {
            x0 -= 1;
        }
        // Walk right to span end.
        let mut x1 = sx;
        while x1 + 1 < w && matches_target(canvas, x1 + 1, sy, target, opts.tolerance) {
            x1 += 1;
        }

        // Paint the span and seed neighbouring rows.
        let row = sy * w;
        let mut span_above_open = false;
        let mut span_below_open = false;
        for xi in x0..=x1 {
            let idx = (row + xi) as usize;
            if visited[idx] {
                continue;
            }
            visited[idx] = true;
            write_px(canvas, xi, sy, opts.color);

            if sy > 0 {
                let above_match = matches_target(canvas, xi, sy - 1, target, opts.tolerance);
                if above_match && !span_above_open {
                    queue.push_back((xi, sy - 1));
                    span_above_open = true;
                } else if !above_match {
                    span_above_open = false;
                }
            }
            if sy + 1 < h {
                let below_match = matches_target(canvas, xi, sy + 1, target, opts.tolerance);
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

    canvas.mark_dirty(
        min_x as u32,
        min_y as u32,
        (max_x - min_x + 1) as u32,
        (max_y - min_y + 1) as u32,
    );
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
    (p[0] as i32 - target[0] as i32).abs() <= t
        && (p[1] as i32 - target[1] as i32).abs() <= t
        && (p[2] as i32 - target[2] as i32).abs() <= t
        && (p[3] as i32 - target[3] as i32).abs() <= t
}

#[inline]
fn px_eq(a: [u8; 4], b: [u8; 4]) -> bool {
    a[0] == b[0] && a[1] == b[1] && a[2] == b[2] && a[3] == b[3]
}
