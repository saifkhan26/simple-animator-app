//! Colour replace: one colour swapped for another on a drawing.
//!
//! A pixel is the old colour when its red, green and blue are each within
//! the tolerance of it, whatever its alpha. Pixels are stored unmultiplied,
//! so a line's soft edge and a pencil's grain hold the line's own colour and
//! go over whole. A pixel taken keeps its alpha and its offset from the old
//! colour, so the grain and noise inside the tolerance survive the swap.
//!
//! Where the old colour meets another on the same drawing — a fill against
//! its lines — the pixels between are a mix of the two. A painted pixel
//! beside the old colour is checked against the colours around it: when it
//! lies on the way from one of them to the old colour, it moves toward the
//! new colour by as much of the old as it holds. The edge keeps its blend,
//! and no fringe of the old colour is left.
//!
//! Pure: pixels in, pixels out. The app decides which drawings.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::doc::canvas::{par_rows, par_rows_as, Canvas};
use crate::tools::lasso::Mask;

/// Upper bound on the tolerance, as for the bucket and the wand.
pub const MAX_TOLERANCE: u8 = 128;

/// How far from the old colour a mixed pixel is looked for, and how far
/// from a mixed pixel the colour it is mixed with.
const REACH: usize = 2;

/// Less of the old colour than this in a pixel is no mix: it is the other
/// colour, give or take its own noise.
const MIN_MIX: f32 = 0.08;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recolor {
    pub from: [u8; 3],
    pub to: [u8; 3],
    /// 0..=[`MAX_TOLERANCE`] per channel.
    pub tolerance: u8,
}

impl Recolor {
    /// Whether `p`, one RGBA pixel, is painted in the old colour.
    fn is_old(&self, p: &[u8]) -> bool {
        p[3] > 0 && (0..3).all(|c| p[c].abs_diff(self.from[c]) <= self.tolerance)
    }

    /// How far off the line from a colour to the old one a pixel may be
    /// and still count as a mix of the two: the old colour's own spread,
    /// within reason.
    fn slack(&self) -> f32 {
        (self.tolerance as f32).clamp(4.0, 24.0)
    }
}

/// `canvas` with the colour replaced, or `None` when no pixel changed. Held
/// to `clip` — in the canvas's own pixels — by its coverage, when given.
pub fn recolored(canvas: &Canvas, r: &Recolor, clip: Option<&Mask>) -> Option<Canvas> {
    if r.from == r.to {
        return None;
    }
    let (rect, src) = canvas.held()?;
    let (w, h) = ((rect.max_x - rect.min_x) as usize, (rect.max_y - rect.min_y) as usize);
    let mut old = vec![false; w * h];
    let any = AtomicBool::new(false);
    // A flag a pixel, worked out from its four bytes: threads as for those.
    par_rows_as(&mut old, w, 4, |y0, band| {
        let mut hit = false;
        for (y, row) in (y0..).zip(band.chunks_exact_mut(w)) {
            for (x, o) in row.iter_mut().enumerate() {
                let i = (y * w + x) * 4;
                *o = r.is_old(&src[i..i + 4]);
                hit |= *o;
            }
        }
        if hit {
            any.store(true, Ordering::Relaxed);
        }
    });
    if !any.into_inner() {
        return None;
    }
    let near = near_old(&old, w, h);
    let shift = [0, 1, 2].map(|c| r.to[c] as f32 - r.from[c] as f32);
    // Copied over band by band, on as many threads as work it.
    let mut out = vec![0u8; src.len()];
    let changed = AtomicBool::new(false);
    par_rows(&mut out, w * 4, |y0, band| {
        let mut moved = false;
        for (y, row) in (y0..).zip(band.chunks_exact_mut(w * 4)) {
            row.copy_from_slice(&src[y * w * 4..(y + 1) * w * 4]);
            for x in 0..w {
                let i = y * w + x;
                let p = &src[i * 4..i * 4 + 4];
                if p[3] == 0 {
                    continue;
                }
                let mut t = match (old[i], near[i]) {
                    (true, _) => 1.0,
                    (false, true) => mixed(src, &old, w, h, x, y, r),
                    (false, false) => continue,
                };
                if let Some(m) = clip {
                    t *= m.at(rect.min_x + x as u32, rect.min_y + y as u32) as f32 / 255.0;
                }
                if t <= 0.0 {
                    continue;
                }
                for c in 0..3 {
                    let v = (p[c] as f32 + t * shift[c]).round().clamp(0.0, 255.0) as u8;
                    // A clip's soft edge or a faint mix can round back to
                    // what was there.
                    moved |= v != p[c];
                    row[x * 4 + c] = v;
                }
            }
        }
        if moved {
            changed.store(true, Ordering::Relaxed);
        }
    });
    changed.into_inner().then(|| canvas.with_held(out))
}

/// Each pixel with one of `old` within [`REACH`] of it, across and down:
/// the only ones that can be a mix. A run along each row, then a look up
/// and down the column, rather than the whole square at every pixel.
fn near_old(old: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut across = vec![false; w * h];
    par_rows_as(&mut across, w, 4, |y0, band| {
        for (y, row) in (y0..).zip(band.chunks_exact_mut(w)) {
            let src = &old[y * w..(y + 1) * w];
            // How many old pixels lie in `x - REACH ..= x + REACH`.
            let mut n = src[..(REACH + 1).min(w)].iter().filter(|&&b| b).count();
            for x in 0..w {
                row[x] = n > 0;
                if x + REACH + 1 < w && src[x + REACH + 1] {
                    n += 1;
                }
                if x >= REACH && src[x - REACH] {
                    n -= 1;
                }
            }
        }
    });
    let mut near = vec![false; w * h];
    par_rows_as(&mut near, w, 4, |y0, band| {
        for (y, row) in (y0..).zip(band.chunks_exact_mut(w)) {
            let ys = y.saturating_sub(REACH)..=(y + REACH).min(h - 1);
            for (x, n) in row.iter_mut().enumerate() {
                *n = ys.clone().any(|j| across[j * w + x]);
            }
        }
    });
    near
}

/// How much of the old colour the pixel at `(x, y)` holds when it is a mix
/// of the old colour and one beside it; 0 when it isn't one. The colour it
/// is mixed with is taken as the farthest from the old one that it lies on
/// the way from — the core of a line, not another pixel of its soft edge.
/// Asked only of pixels [`near_old`] marks.
fn mixed(src: &[u8], old: &[bool], w: usize, h: usize, x: usize, y: usize, r: &Recolor) -> f32 {
    let xs = x.saturating_sub(REACH)..=(x + REACH).min(w - 1);
    let ys = y.saturating_sub(REACH)..=(y + REACH).min(h - 1);
    let rgb = |i: usize| [0, 1, 2].map(|c| src[i * 4 + c] as f32);
    let p = rgb(y * w + x);
    let f = r.from.map(f32::from);
    let (mut far, mut t_best) = (0.0, 0.0);
    for j in ys {
        for i in xs.clone() {
            let k = j * w + i;
            if old[k] || src[k * 4 + 3] == 0 {
                continue;
            }
            let q = rgb(k);
            let v = [0, 1, 2].map(|c| f[c] - q[c]);
            let len2: f32 = v.iter().map(|d| d * d).sum();
            if len2 <= far {
                continue;
            }
            let t = (0..3).map(|c| (p[c] - q[c]) * v[c]).sum::<f32>() / len2;
            if !(MIN_MIX..1.0).contains(&t) {
                continue;
            }
            let off = (0..3).map(|c| (p[c] - q[c] - t * v[c]).abs()).fold(0.0, f32::max);
            if off <= r.slack() {
                (far, t_best) = (len2, t);
            }
        }
    }
    t_best
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 3] = [200, 30, 30];
    const BLUE: [u8; 3] = [30, 60, 220];

    fn swap(tolerance: u8) -> Recolor {
        Recolor { from: RED, to: BLUE, tolerance }
    }

    /// A `w`×1 strip of these pixels.
    fn strip(px: &[[u8; 4]]) -> Canvas {
        Canvas::from_pixels(px.len() as u32, 1, px.concat())
    }

    fn rgba(c: [u8; 3], a: u8) -> [u8; 4] {
        [c[0], c[1], c[2], a]
    }

    #[test]
    fn the_old_colour_goes_over_whole_keeping_its_alpha() {
        let c = strip(&[rgba(RED, 255), rgba(RED, 90), rgba(RED, 3), [9, 9, 9, 0]]);
        let out = recolored(&c, &swap(0), None).unwrap();
        assert_eq!(out.px(0, 0), rgba(BLUE, 255));
        assert_eq!(out.px(1, 0), rgba(BLUE, 90), "a soft edge keeps its softness");
        assert_eq!(out.px(2, 0), rgba(BLUE, 3));
        assert_eq!(out.px(3, 0), [9, 9, 9, 0], "bare canvas is left alone");
        assert_ne!(out.rev(), c.rev());
    }

    #[test]
    fn tolerance_takes_near_colours_and_keeps_their_offset() {
        let near = [210, 22, 30, 255];
        let far = [200, 80, 30, 255];
        let c = strip(&[near, far, near]);
        let out = recolored(&c, &swap(10), None).unwrap();
        assert_eq!(out.px(0, 0), [40, 52, 220, 255], "grain inside the tolerance survives");
        assert_eq!(out.px(1, 0), far, "a colour past it is another colour");
        assert!(recolored(&c, &swap(5), None).is_none(), "nothing that close");
    }

    #[test]
    fn a_mix_with_another_colour_moves_by_as_much_of_the_old_as_it_holds() {
        // A black line against a red fill, on one drawing, the edge between
        // them half and half, then a quarter red.
        let black = [0, 0, 0];
        let half = [100, 15, 15];
        let quarter = [50, 8, 8];
        let c = strip(&[
            rgba(black, 255),
            rgba(black, 255),
            rgba(quarter, 255),
            rgba(half, 255),
            rgba(RED, 255),
            rgba(RED, 255),
        ]);
        let out = recolored(&c, &swap(0), None).unwrap();
        assert_eq!(out.px(0, 0), rgba(black, 255), "the line's core stays");
        assert_eq!(out.px(1, 0), rgba(black, 255));
        assert_eq!(out.px(3, 0), [15, 30, 110, 255], "half way from black to blue");
        assert_eq!(out.px(2, 0), [7, 16, 56, 255], "a quarter of the way");
        assert_eq!(out.px(4, 0), rgba(BLUE, 255));
    }

    #[test]
    fn a_colour_beside_the_old_one_but_off_the_way_to_it_is_left() {
        let green = [20, 200, 40];
        let c = strip(&[rgba(green, 255), rgba([60, 190, 40], 255), rgba(RED, 255)]);
        let out = recolored(&c, &swap(0), None).unwrap();
        assert_eq!(out.px(0, 0), rgba(green, 255));
        assert_eq!(out.px(1, 0), [60, 190, 40, 255], "no mix of green and red");
    }

    #[test]
    fn a_clip_holds_it_by_coverage() {
        let c = strip(&[rgba(RED, 255); 3]);
        let clip = Mask { x: 1, y: 0, w: 2, h: 1, cov: vec![255, 128] };
        let out = recolored(&c, &swap(0), Some(&clip)).unwrap();
        assert_eq!(out.px(0, 0), rgba(RED, 255), "outside the clip");
        assert_eq!(out.px(1, 0), rgba(BLUE, 255));
        assert_eq!(out.px(2, 0), [115, 45, 125, 255], "half covered, half way");
    }

    #[test]
    fn near_marks_what_lies_within_reach_of_the_old_colour() {
        // 7×5, one old pixel at (3, 2).
        let (w, h) = (7, 5);
        let mut old = vec![false; w * h];
        old[2 * w + 3] = true;
        let near = near_old(&old, w, h);
        for y in 0..h {
            for x in 0..w {
                let want = x.abs_diff(3) <= REACH && y.abs_diff(2) <= REACH;
                assert_eq!(near[y * w + x], want, "({x}, {y})");
            }
        }
        // At the edges, and a row too short to reach across.
        let near = near_old(&[true, false, false, false], 4, 1);
        assert_eq!(near, [true, true, true, false]);
        assert_eq!(near_old(&[false, true], 2, 1), [true, true]);
    }

    #[test]
    fn nothing_to_change_gives_nothing() {
        let c = strip(&[rgba(RED, 255)]);
        assert!(recolored(&c, &Recolor { from: RED, to: RED, tolerance: 4 }, None).is_none());
        assert!(recolored(&Canvas::new(8, 8), &swap(20), None).is_none(), "a blank drawing");
        let off = Mask { x: 5, y: 5, w: 1, h: 1, cov: vec![255] };
        assert!(recolored(&c, &swap(0), Some(&off)).is_none(), "clipped away");
    }

    /// Time on a 1080p drawing, half of it the old colour and its edge
    /// against black. `cargo test perf_recolor -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn perf_recolor() {
        let (w, h) = (1920usize, 1080usize);
        let mut px = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let c = if x < w / 2 { rgba(RED, 255) } else { [0, 0, 0, 255] };
                px[(y * w + x) * 4..][..4].copy_from_slice(&c);
            }
        }
        let c = Canvas::from_pixels(w as u32, h as u32, px);
        let t = std::time::Instant::now();
        let n = 20;
        for _ in 0..n {
            assert!(recolored(&c, &swap(16), None).is_some());
        }
        eprintln!("1080p recolour: {:.1} ms each", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
    }

    #[test]
    fn a_packed_drawing_stays_packed_over_the_same_pixels() {
        let mut full = vec![0u8; 16 * 16 * 4];
        for y in 4..8 {
            for x in 6..10 {
                let i = (y * 16 + x) * 4;
                full[i..i + 4].copy_from_slice(&rgba(RED, 200));
            }
        }
        let c = Canvas::packed_from(16, 16, &full);
        let out = recolored(&c, &swap(0), None).unwrap();
        assert!(out.is_packed());
        assert_eq!(out.held_rect(), c.held_rect());
        assert_eq!(out.dirty, c.held_rect());
        assert_eq!(out.px(7, 5), rgba(BLUE, 200));
        assert_eq!(out.px(0, 0), [0; 4]);
    }
}
