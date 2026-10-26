//! Magic select: the pixels joined to a click.
//!
//! The Lasso tool's wand. A press picks the run of pixels that look like the
//! one under it and touch it, the way the bucket floods — or, with
//! [`WandOptions::any_paint`], every painted pixel touching it whatever its
//! colour — and the selection takes that shape. It shares the bucket's flood,
//! its gap closing and its expand, so a pick stops where a fill would.
//!
//! A press is a [`WandPick`]. It keeps the pixels and what it worked out, so a
//! drag that retunes the tolerance or the gap afterwards only redoes what
//! changed.
//!
//! Pure: pixels in, coverage out. The app decides which pixels, and the space
//! they are in.

use crate::doc::canvas::par_rows;
use crate::tools::fill::{self, Bbox, Field};
use crate::tools::scan::{self, OPEN, SHUT, TAKEN};

/// Alpha at or above this is paint to [`WandOptions::any_paint`] — the same
/// line the bucket draws between paint and bare canvas.
const PAINT: u8 = 8;

/// Upper bound on the tolerance, matching the bucket's slider.
pub const MAX_TOLERANCE: u8 = 128;

/// The tolerances a drag steps through: every one while they are small, wider
/// strides as they grow, so the whole range is a comfortable pen's travel.
const TOLERANCE_LADDER: [u8; 23] = [
    0, 1, 2, 3, 4, 6, 8, 10, 12, 14, 16, 20, 24, 28, 32, 40, 48, 56, 64, 80, 96, 112, 128,
];

/// The tolerance `steps` rungs away from `base`, as [`fill::step_gap`]
/// counts them.
pub fn step_tolerance(base: u8, steps: i32) -> u8 {
    fill::step_ladder(&TOLERANCE_LADDER, base, steps)
}

/// The wand's settings. Workspace preferences, like the selection shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WandOptions {
    /// 0..=[`MAX_TOLERANCE`] per channel, matched as the bucket matches.
    pub tolerance: u8,
    /// Treat breaks in the lines up to this many pixels wide as shut. 0 = off.
    pub gap: u8,
    /// Grow each pick by this many pixels, so a flat colour's selection
    /// reaches under its anti-aliased lines. 0 = off.
    pub expand: u8,
    /// A press on paint takes every painted pixel touching it, whatever its
    /// colour. A press on bare canvas picks as it always does.
    pub any_paint: bool,
    /// Off: every matching pixel on the drawing, touching or not.
    pub contiguous: bool,
    /// Read every visible layer at once instead of the active drawing.
    pub all_visible: bool,
}

impl Default for WandOptions {
    fn default() -> Self {
        Self {
            tolerance: 24,
            gap: 0,
            expand: 0,
            any_paint: false,
            contiguous: true,
            all_visible: false,
        }
    }
}

/// What a pick covers: a box in the pixels' own space, and a byte per pixel
/// in it, 0 or 255.
#[derive(Clone, Debug, PartialEq)]
pub struct Picked {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub cov: Vec<u8>,
}

/// The pick at one tolerance: a byte per pixel, [`SHUT`] or [`OPEN`], and
/// once flooded, [`TAKEN`] for the run joined to the press — a [`scan::flood`]
/// map.
struct Level {
    tolerance: u8,
    map: Vec<u8>,
    /// The flood's bounds, once it has run.
    flood: Option<Bbox>,
    /// The flood as `bool`s, for the gap and the expand, once asked for.
    taken: Option<Vec<bool>>,
}

/// One press of the wand. Built at pen-down; [`pick`](Self::pick) works out
/// what it takes, and may be called again as a drag retunes it.
///
/// The press reads its pixels once, into how far each is from the one
/// pressed on, so a tolerance is a compare per pixel and the pixels needn't
/// be kept.
pub struct WandPick {
    w: i32,
    h: i32,
    seed: (i32, i32),
    /// Per pixel: the smallest tolerance that lets it in. 255 is never.
    reach: Vec<u8>,
    level: Option<Level>,
    /// Distance to the walls, for the tolerance it was built at.
    field: Option<(u8, Field)>,
}

impl WandPick {
    /// A press at `seed` on `size` RGBA8 pixels `src`. `None` off them.
    /// `any_paint` as [`WandOptions::any_paint`]: it is settled at the press.
    pub fn new(src: &[u8], size: (u32, u32), seed: (i32, i32), any_paint: bool) -> Option<Self> {
        let (w, h) = (size.0 as i32, size.1 as i32);
        let (x, y) = seed;
        if x < 0 || y < 0 || x >= w || y >= h || src.len() != size.0 as usize * size.1 as usize * 4
        {
            return None;
        }
        let target = fill::src_px(src, w, x, y);
        let n = size.0 as usize;
        let mut reach = vec![0u8; n * size.1 as usize];
        // The bucket's rules (`fill::matches_target`), as the tolerance each
        // pixel needs: on paint with Any paint, any paint; on bare canvas,
        // anything at most half covered; otherwise the widest channel apart.
        let paint = any_paint && target[3] >= PAINT;
        let bare = target[3] < 8;
        par_rows(&mut reach, n, |y0, band| {
            let src = &src[y0 * n * 4..(y0 * n + band.len()) * 4];
            let px = band.iter_mut().zip(src.chunks_exact(4));
            if paint {
                px.for_each(|(r, p)| *r = if p[3] >= PAINT { 0 } else { 255 });
            } else if bare {
                px.for_each(|(r, p)| *r = if p[3] <= 128 { 0 } else { 255 });
            } else {
                px.for_each(|(r, p)| {
                    let d = |c: usize| p[c].abs_diff(target[c]);
                    *r = d(0).max(d(1)).max(d(2)).max(d(3));
                });
            }
        });
        Some(Self {
            w,
            h,
            seed,
            reach,
            level: None,
            field: None,
        })
    }

    /// What `o` picks. Always holds the pixel pressed on.
    pub fn pick(&mut self, o: &WandOptions) -> Picked {
        let (w, h, seed) = (self.w, self.h, self.seed);
        let tolerance = o.tolerance.min(MAX_TOLERANCE);
        if self.level.as_ref().map_or(true, |l| l.tolerance != tolerance) {
            let n = w as usize;
            let mut map = vec![SHUT; self.reach.len()];
            let reach = &self.reach;
            par_rows(&mut map, n, |y0, band| {
                let reach = &reach[y0 * n..y0 * n + band.len()];
                for (m, &r) in band.iter_mut().zip(reach) {
                    *m = if r <= tolerance { OPEN } else { SHUT };
                }
            });
            self.level = Some(Level {
                tolerance,
                map,
                flood: None,
                taken: None,
            });
        }
        let level = self.level.as_mut().expect("just made");
        let r = o.expand.min(fill::MAX_EXPAND) as i32;
        if !o.contiguous {
            // Every pixel let in, flooded or not.
            let Some(bbox) = bounds(&level.map, w as usize, |v| v != SHUT) else {
                return cover_map(&level.map, w, (0, 0, -1, -1), |v| v != SHUT);
            };
            if r == 0 {
                return cover_map(&level.map, w, bbox, |v| v != SHUT);
            }
            let bits: Vec<bool> = level.map.iter().map(|&v| v != SHUT).collect();
            let grown = fill::dilate_window(&bits, w, h, bbox, r);
            return cover(grown.bbox(), |x, y| grown.at(x, y));
        }
        let pbox = *level
            .flood
            .get_or_insert_with(|| scan::flood(&mut level.map, w as usize, seed));
        if o.gap == 0 && r == 0 {
            return cover_map(&level.map, w, pbox, |v| v == TAKEN);
        }
        let map = &level.map;
        let taken = level
            .taken
            .get_or_insert_with(|| map.iter().map(|&v| v == TAKEN).collect());
        let owned: (Vec<bool>, Bbox);
        let (mask, bbox) = if o.gap == 0 {
            (&taken[..], pbox)
        } else {
            // As the bucket does: walls from farther out for a wider gap,
            // rebuilt in doubling steps.
            let needed = o.gap.div_ceil(2) as i32 + 1;
            let stale = self
                .field
                .as_ref()
                .map_or(true, |(t, f)| *t != tolerance || (!f.whole && f.reach < needed));
            if stale {
                let reach = (needed as u32).next_power_of_two().max(16) as i32;
                let pass = |x: i32, y: i32| map[(y * w + x) as usize] != SHUT;
                self.field = Some((tolerance, fill::wall_distance(w, h, pbox, reach, pass)));
            }
            let (_, field) = self.field.as_ref().expect("just built");
            owned = fill::gap_flood(field, taken, w, h, seed, o.gap);
            (&owned.0[..], owned.1)
        };
        if r > 0 {
            let grown = fill::dilate_window(mask, w, h, bbox, r);
            cover(grown.bbox(), |x, y| grown.at(x, y))
        } else {
            cover_bits(mask, w, bbox)
        }
    }
}

/// The bounds of the pixels of `map`, `w` wide, whose value `keep` takes.
fn bounds(map: &[u8], w: usize, keep: impl Fn(u8) -> bool + Copy) -> Option<Bbox> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, usize::MAX, 0, 0);
    for (y, row) in map.chunks_exact(w).enumerate() {
        let Some(first) = row.iter().position(|&v| keep(v)) else {
            continue;
        };
        let last = row.iter().rposition(|&v| keep(v)).unwrap_or(first);
        x0 = x0.min(first);
        x1 = x1.max(last);
        y0 = y0.min(y);
        y1 = y;
    }
    (y0 != usize::MAX).then_some((x0 as i32, y0 as i32, x1 as i32, y1 as i32))
}

/// The coverage of `bbox` where `keep` takes the value of `map`, `w` wide.
fn cover_map(map: &[u8], w: i32, bbox: Bbox, keep: impl Fn(u8) -> bool) -> Picked {
    cover_rows(bbox, |y, x0, x1, out| {
        let row = &map[(y * w + x0) as usize..=(y * w + x1) as usize];
        out.extend(row.iter().map(|&v| (keep(v) as u8).wrapping_neg()));
    })
}

/// The coverage of `bbox` where `mask`, `w` wide, is set.
fn cover_bits(mask: &[bool], w: i32, bbox: Bbox) -> Picked {
    cover_rows(bbox, |y, x0, x1, out| {
        let row = &mask[(y * w + x0) as usize..=(y * w + x1) as usize];
        out.extend(row.iter().map(|&b| (b as u8).wrapping_neg()));
    })
}

/// The coverage of `bbox` where `set` holds.
fn cover(bbox: Bbox, set: impl Fn(i32, i32) -> bool) -> Picked {
    cover_rows(bbox, |y, x0, x1, out| {
        out.extend((x0..=x1).map(|x| (set(x, y) as u8).wrapping_neg()));
    })
}

/// A [`Picked`] over `bbox`, each row's bytes appended by `row(y, x0, x1, out)`.
fn cover_rows(bbox: Bbox, row: impl Fn(i32, i32, i32, &mut Vec<u8>)) -> Picked {
    let (x0, y0, x1, y1) = bbox;
    let (w, h) = ((x1 - x0 + 1).max(0), (y1 - y0 + 1).max(0));
    let mut cov = Vec::with_capacity((w * h) as usize);
    if w > 0 {
        for y in y0..=y1 {
            row(y, x0, x1, &mut cov);
        }
    }
    Picked {
        x: x0,
        y: y0,
        w: w as u32,
        h: h as u32,
        cov,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 40;
    const H: u32 = 30;
    const INK: [u8; 4] = [10, 10, 10, 255];
    const RED: [u8; 4] = [200, 30, 30, 255];

    fn blank() -> Vec<u8> {
        vec![0; (W * H * 4) as usize]
    }

    fn rect(px: &mut [u8], (x0, y0, x1, y1): (u32, u32, u32, u32), c: [u8; 4]) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = ((y * W + x) * 4) as usize;
                px[i..i + 4].copy_from_slice(&c);
            }
        }
    }

    /// A square outline from (5,5) to (20,20), one pixel thick.
    fn ring(px: &mut [u8]) {
        rect(px, (5, 5, 20, 5), INK);
        rect(px, (5, 20, 20, 20), INK);
        rect(px, (5, 5, 5, 20), INK);
        rect(px, (20, 5, 20, 20), INK);
    }

    fn pick(px: Vec<u8>, seed: (i32, i32), o: WandOptions) -> Picked {
        WandPick::new(&px, (W, H), seed, o.any_paint).unwrap().pick(&o)
    }

    fn count(p: &Picked) -> usize {
        p.cov.iter().filter(|&&c| c == 255).count()
    }

    fn at(p: &Picked, x: i32, y: i32) -> bool {
        let (u, v) = (x - p.x, y - p.y);
        u >= 0 && v >= 0 && u < p.w as i32 && v < p.h as i32 && p.cov[(v * p.w as i32 + u) as usize] == 255
    }

    #[test]
    fn a_click_inside_a_ring_takes_the_inside_only() {
        let mut px = blank();
        ring(&mut px);
        let p = pick(px, (12, 12), WandOptions::default());
        assert_eq!((p.x, p.y, p.w, p.h), (6, 6, 14, 14));
        assert_eq!(count(&p), 14 * 14);
    }

    #[test]
    fn a_click_on_the_line_takes_the_line() {
        let mut px = blank();
        ring(&mut px);
        let p = pick(px, (5, 12), WandOptions::default());
        assert_eq!(count(&p), 16 * 4 - 4);
        assert!(at(&p, 20, 20) && !at(&p, 12, 12));
    }

    #[test]
    fn a_click_outside_takes_the_bare_canvas_round_it() {
        let mut px = blank();
        ring(&mut px);
        let p = pick(px, (30, 2), WandOptions::default());
        assert_eq!(count(&p), (W * H) as usize - 16 * 16);
        assert!(!at(&p, 12, 12));
    }

    #[test]
    fn tolerance_decides_how_far_a_colour_runs() {
        let mut px = blank();
        rect(&mut px, (0, 0, 9, 9), [100, 100, 100, 255]);
        rect(&mut px, (10, 0, 19, 9), [120, 100, 100, 255]);
        let tight = WandOptions { tolerance: 10, ..Default::default() };
        let loose = WandOptions { tolerance: 20, ..Default::default() };
        assert_eq!(count(&pick(px.clone(), (2, 2), tight)), 100);
        assert_eq!(count(&pick(px, (2, 2), loose)), 200);
    }

    #[test]
    fn any_paint_takes_the_whole_blob_whatever_its_colours() {
        let mut px = blank();
        ring(&mut px);
        rect(&mut px, (6, 6, 19, 19), RED);
        rect(&mut px, (30, 0, 35, 5), INK);
        let like = pick(px.clone(), (12, 12), WandOptions::default());
        assert_eq!(count(&like), 14 * 14);
        let any = WandOptions { any_paint: true, ..Default::default() };
        let blob = pick(px.clone(), (12, 12), any);
        assert_eq!(count(&blob), 16 * 16, "lines and fill, not the blob apart");
        // On bare canvas it still picks the bare canvas.
        let bare = pick(px, (25, 25), any);
        assert_eq!(count(&bare), (W * H) as usize - 16 * 16 - 36);
    }

    #[test]
    fn contiguous_off_takes_every_match_anywhere() {
        let mut px = blank();
        rect(&mut px, (0, 0, 3, 3), RED);
        rect(&mut px, (10, 10, 13, 13), RED);
        let on = pick(px.clone(), (1, 1), WandOptions::default());
        assert_eq!(count(&on), 16);
        let off = WandOptions { contiguous: false, ..Default::default() };
        let p = pick(px, (1, 1), off);
        assert_eq!(count(&p), 32);
        assert_eq!((p.x, p.y, p.w, p.h), (0, 0, 14, 14));
    }

    #[test]
    fn gap_closing_stops_at_a_break_in_the_line() {
        let mut px = blank();
        ring(&mut px);
        // A two-pixel break in the top edge.
        rect(&mut px, (12, 5, 13, 5), [0, 0, 0, 0]);
        let leaks = pick(px.clone(), (12, 12), WandOptions::default());
        assert!(at(&leaks, 30, 25), "with no gap closing it runs out");
        let shut = WandOptions { gap: 4, ..Default::default() };
        let p = pick(px, (12, 12), shut);
        assert!(!at(&p, 30, 25));
        assert!(at(&p, 6, 6) && at(&p, 19, 19), "still reaches the corners");
    }

    #[test]
    fn expand_grows_the_pick() {
        let mut px = blank();
        ring(&mut px);
        let o = WandOptions { expand: 1, ..Default::default() };
        let p = pick(px, (12, 12), o);
        assert_eq!((p.x, p.y, p.w, p.h), (5, 5, 16, 16));
        assert_eq!(count(&p), 16 * 16);
    }

    #[test]
    fn a_retuned_pick_matches_a_fresh_one() {
        let mut px = blank();
        ring(&mut px);
        rect(&mut px, (12, 5, 13, 5), [0, 0, 0, 0]);
        rect(&mut px, (8, 8, 10, 10), [30, 30, 30, 255]);
        let mut live = WandPick::new(&px, (W, H), (12, 12), false).unwrap();
        for (tolerance, gap) in [(0, 0), (0, 4), (40, 4), (40, 0), (0, 8)] {
            let o = WandOptions { tolerance, gap, ..Default::default() };
            assert_eq!(live.pick(&o), pick(px.clone(), (12, 12), o), "tol {tolerance} gap {gap}");
        }
    }


    /// The pick as first written: the bucket's own flood and rules, straight
    /// off the pixels. What the fast one must match, pixel for pixel.
    fn reference_pick(src: &[u8], size: (u32, u32), seed: (i32, i32), o: &WandOptions) -> Picked {
        let (w, h) = (size.0 as i32, size.1 as i32);
        let target = fill::src_px(src, w, seed.0, seed.1);
        let paint = o.any_paint && target[3] >= PAINT;
        let tol = o.tolerance.min(MAX_TOLERANCE);
        let pass = |x: i32, y: i32| {
            if paint {
                fill::src_px(src, w, x, y)[3] >= PAINT
            } else {
                fill::matches_target(src, w, x, y, target, tol)
            }
        };
        let owned: (Vec<bool>, Bbox);
        let (mask, bbox) = if !o.contiguous {
            let mut mask = vec![false; (w * h) as usize];
            let (mut x0, mut y0, mut x1, mut y1) = (w, h, -1, -1);
            for y in 0..h {
                for x in 0..w {
                    if pass(x, y) {
                        mask[(y * w + x) as usize] = true;
                        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                    }
                }
            }
            owned = (mask, (x0, y0, x1, y1));
            (&owned.0[..], owned.1)
        } else {
            let (plain, pbox) = fill::span_flood(w, h, seed, pass);
            if o.gap == 0 {
                owned = (plain, pbox);
                (&owned.0[..], owned.1)
            } else {
                let needed = o.gap.div_ceil(2) as i32 + 1;
                let reach = (needed as u32).next_power_of_two().max(16) as i32;
                let field = fill::wall_distance(w, h, pbox, reach, pass);
                owned = fill::gap_flood(&field, &plain, w, h, seed, o.gap);
                (&owned.0[..], owned.1)
            }
        };
        let r = o.expand.min(fill::MAX_EXPAND) as i32;
        if r > 0 {
            let grown = fill::dilate_window(mask, w, h, bbox, r);
            cover(grown.bbox(), |x, y| grown.at(x, y))
        } else {
            cover(bbox, |x, y| mask[(y * w + x) as usize])
        }
    }

    /// A small xorshift, so the random scenes are the same every run.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u32) -> u32 {
            (self.next() % n as u64) as u32
        }
    }

    /// Random line art: boxes of a few close colours, broken outlines,
    /// faint anti-aliased specks and bare canvas, at sizes that are and
    /// aren't a multiple of the scan's word.
    fn scene(rng: &mut Rng, w: u32, h: u32) -> Vec<u8> {
        let mut px = vec![0u8; (w * h * 4) as usize];
        let colours = [[10, 10, 10, 255], [200, 30, 30, 255], [205, 36, 30, 255], [60, 60, 200, 90], [0, 0, 0, 20]];
        for _ in 0..12 {
            let (x0, y0) = (rng.below(w), rng.below(h));
            let (x1, y1) = ((x0 + 1 + rng.below(w / 2)).min(w), (y0 + 1 + rng.below(h / 2)).min(h));
            let c = colours[rng.below(colours.len() as u32) as usize];
            let outline = rng.below(2) == 0;
            for y in y0..y1 {
                for x in x0..x1 {
                    let edge = x == x0 || y == y0 || x == x1 - 1 || y == y1 - 1;
                    if (!outline || edge) && rng.below(29) != 0 {
                        let i = ((y * w + x) * 4) as usize;
                        px[i..i + 4].copy_from_slice(&c);
                    }
                }
            }
        }
        px
    }

    /// The fast pick against the first one, over random scenes, presses and
    /// settings, a press retuned the way a drag retunes it.
    #[test]
    fn the_fast_pick_matches_the_first_one_exactly() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for round in 0..60 {
            let (w, h) = (17 + rng.below(70), 9 + rng.below(50));
            let px = scene(&mut rng, w, h);
            let seed = (rng.below(w) as i32, rng.below(h) as i32);
            let any_paint = rng.below(3) == 0;
            let contiguous = rng.below(4) != 0;
            let expand = [0, 0, 1, 3][rng.below(4) as usize];
            let mut live = WandPick::new(&px, (w, h), seed, any_paint).unwrap();
            for _ in 0..4 {
                let o = WandOptions {
                    tolerance: [0, 5, 6, 24, 128][rng.below(5) as usize],
                    gap: [0, 0, 1, 2, 4, 9][rng.below(6) as usize],
                    expand,
                    any_paint,
                    contiguous,
                    all_visible: false,
                };
                let want = reference_pick(&px, (w, h), seed, &o);
                assert_eq!(live.pick(&o), want, "round {round}, {w}x{h}, seed {seed:?}, {o:?}");
            }
        }
    }

    #[test]
    fn a_press_off_the_pixels_picks_nothing() {
        assert!(WandPick::new(&blank(), (W, H), (-1, 3), false).is_none());
        assert!(WandPick::new(&blank(), (W, H), (3, H as i32), false).is_none());
    }

    #[test]
    fn tolerance_steps_climb_a_ladder() {
        assert_eq!(step_tolerance(24, 0), 24);
        assert_eq!(step_tolerance(24, 1), 28);
        assert_eq!(step_tolerance(24, -2), 16);
        assert_eq!(step_tolerance(25, 1), 28);
        assert_eq!(step_tolerance(0, -3), 0);
        assert_eq!(step_tolerance(100, 40), MAX_TOLERANCE);
    }

    /// Where an 8K pick's time goes, phase by phase.
    /// `cargo test --release wand_phases -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn wand_phases() {
        use crate::tools::select_mask::{SelOp, SelectionMask};
        use std::time::Instant;
        let (w, h) = (7680u32, 4320u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        let (a, b) = (h / 4, h / 2);
        for y in a..=b {
            for x in a..=b {
                if x == a || x == b || y == a || y == b {
                    let i = ((y * w + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(&INK);
                }
            }
        }
        let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        let mut pick = WandPick::new(&px, (w, h), (5, 5), false).unwrap();
        println!("press (reach map): {:.1}", ms(t));
        let o = WandOptions::default();
        let t = Instant::now();
        let p = pick.pick(&o);
        println!("bg pick (flood + cover): {:.1}", ms(t));
        let t = Instant::now();
        let p2 = pick.pick(&o);
        println!("bg pick again (cover only): {:.1}", ms(t));
        assert_eq!(p, p2);
        let t = Instant::now();
        let m = SelectionMask { x: p.x, y: p.y, w: p.w, h: p.h, cov: p.cov.clone(), outside: 0 };
        let m = m.normalize().unwrap();
        println!("normalize: {:.1}", ms(t));
        let t = Instant::now();
        let r = SelectionMask::combine(None, &m, SelOp::Replace).unwrap();
        println!("combine replace: {:.1}", ms(t));
        let t = Instant::now();
        let _ = SelectionMask::combine(Some(&r), &m, SelOp::Add).unwrap();
        println!("combine add: {:.1}", ms(t));
        let t = Instant::now();
        let c = m.contours();
        println!("contours: {:.1} ({} loops)", ms(t), c.len());
        let t = Instant::now();
        let _ = m.pack();
        println!("pack: {:.1}", ms(t));
        let t = Instant::now();
        let _ = m.to_cell(&crate::doc::transform::Transform::default(), w, h, w as f32, h as f32);
        println!("to_cell: {:.1}", ms(t));
        let gap = WandOptions { gap: 6, ..o };
        let t = Instant::now();
        let _ = pick.pick(&gap);
        println!("gap 6 (field + watershed + cover): {:.1}", ms(t));
        let t = Instant::now();
        let _ = pick.pick(&WandOptions { gap: 8, ..o });
        println!("gap 8 (watershed + cover): {:.1}", ms(t));
        let t = Instant::now();
        let _ = pick.pick(&WandOptions { tolerance: 30, ..o });
        println!("tolerance step: {:.1}", ms(t));
    }
}
