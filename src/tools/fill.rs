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
//!
//! A press is a [`FillSession`]. It keeps what the press worked out, so a drag
//! that changes the gap or the expand afterwards only redoes the cheap part,
//! and the canvas can follow the pen live.
//!
//! Gap closing treats a break in the lines up to `gap` pixels wide as shut. It
//! works on the distance from each pixel to the nearest wall:
//!   * *open space* is everything farther from every wall than half the gap —
//!     too far out for a gap that narrow to carry it through. It comes in
//!     pieces, and the click's piece is the fill's core;
//!   * the band between open space and the walls is then shared out among all
//!     the pieces by a watershed, so the fill still reaches every corner right
//!     up to its lines, while the throat of a gap splits between its two sides.
//!
//! The result is always part of what the plain flood fills, so closing gaps can
//! hold a fill back but never push one through a line.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::doc::canvas::{Canvas, DirtyRect};
use crate::tools::lasso::Mask;
use crate::tools::ribbon::union_rect;

/// Upper bound on the expand radius, matching the UI slider.
pub const MAX_EXPAND: u8 = 32;
/// Upper bound on the gap setting, matching the UI slider.
pub const MAX_GAP: u8 = 255;

/// The gaps a drag steps through: every pixel while gaps are small, wider
/// strides as they grow, so the whole range is a comfortable pen's travel
/// either way.
const GAP_LADDER: [u8; 37] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 16, 18, 20, 22, 24, 28, 32, 36, 40, 48, 56,
    64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 255,
];

/// Screen points a fill drag moves before it starts counting, so the small
/// wander of a plain tap never changes a value.
const DRAG_DEAD_ZONE: f32 = 8.0;
/// Screen points of drag per step of a value.
const DRAG_STEP: f32 = 12.0;

#[derive(Clone, Copy)]
pub struct FillOptions {
    /// 0..=255 per-channel tolerance.
    pub tolerance: u8,
    /// Fill colour (RGBA8 unmultiplied).
    pub color: [u8; 4],
    /// Grow the filled region by this many pixels after the flood so the colour
    /// tucks under anti-aliased lines. 0 = off.
    pub expand: u8,
    /// Treat breaks in the lines up to this many pixels wide as shut. 0 = off.
    pub gap: u8,
}

/// Inclusive pixel bounds, `(min_x, min_y, max_x, max_y)`.
type Bbox = (i32, i32, i32, i32);

/// Whole steps in a drag of `d` screen points along one axis: none inside the
/// dead zone, then one per [`DRAG_STEP`], signed like `d`.
pub fn drag_steps(d: f32) -> i32 {
    let past = d.abs() - DRAG_DEAD_ZONE;
    if past.is_nan() || past <= 0.0 {
        return 0;
    }
    (past / DRAG_STEP).floor() as i32 * d.signum() as i32
}

/// The gap `steps` rungs of [`GAP_LADDER`] away from `base`. No steps keeps
/// `base` as it is, even off the ladder; the first step lands on the next rung
/// in that direction.
pub fn step_gap(base: u8, steps: i32) -> u8 {
    let rungs = GAP_LADDER.len() as i32;
    let at = if steps > 0 {
        let above = GAP_LADDER.iter().position(|&g| g > base).unwrap_or(GAP_LADDER.len()) as i32;
        above + steps - 1
    } else if steps < 0 {
        let below = GAP_LADDER.iter().rposition(|&g| g < base).map_or(-1, |i| i as i32);
        below + steps + 1
    } else {
        return base;
    };
    GAP_LADDER[at.clamp(0, rungs - 1) as usize]
}

/// The flood's reach at one gap setting: a `w * h` bitmap and its bounds.
struct Region {
    gap: u8,
    mask: Vec<bool>,
    bbox: Bbox,
}

/// Squared distance to the nearest wall over a window of the cell, 0 on the
/// walls. The window is the plain flood's bounds grown by `reach`, which puts
/// every wall close enough to matter for a gap up to `2 * (reach - 1)` inside
/// it; a gap fill never reaches past the plain flood, so nothing outside is
/// needed.
struct Field {
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    reach: i32,
    /// The window is the whole cell, so it serves any gap.
    whole: bool,
    dist: Vec<f32>,
}

/// One press of the bucket. Built at pen-down; [`apply`](Self::apply) paints
/// it, and may be called again as a drag changes the gap or the expand — each
/// call puts back what the last one painted first.
pub struct FillSession {
    w: i32,
    h: i32,
    seed: (i32, i32),
    target: [u8; 4],
    tolerance: u8,
    color: [u8; 4],
    /// Lines from another layer, already in the cell's pixels. `None` reads
    /// the cell's own pixels as they were before the press.
    boundary: Option<Canvas>,
    clip: Option<Arc<Mask>>,
    /// A same-layer refill with the colour already there: nothing to do.
    inert: bool,
    /// The plain flood, gap 0. Every gap region lies inside it.
    plain: Option<Region>,
    /// Built the first time a gap is asked for, then kept for the press.
    field: Option<Field>,
    /// The last gap region worked out, for a gap above 0.
    region: Option<Region>,
    /// Bounds of what the last `apply` painted.
    written: Option<Bbox>,
    /// Everything any `apply` of this press changed, for the undo record.
    touched: Option<DirtyRect>,
}

impl FillSession {
    /// A press at `seed` on a `size` cell whose pixels before the press are
    /// `pre`.
    ///
    /// When `boundary` is `Some`, the region is grown by looking at *those*
    /// pixels instead of the cell's own — the line art that walls the fill in
    /// lives on another layer. `clip` holds the fill to a selection.
    ///
    /// `None` when the press can't fill anything: outside the cell or the
    /// selection, or a boundary whose size doesn't match.
    pub fn new(
        pre: &[u8],
        size: (u32, u32),
        seed: (i32, i32),
        opts: &FillOptions,
        boundary: Option<Canvas>,
        clip: Option<Arc<Mask>>,
    ) -> Option<Self> {
        let (w, h) = (size.0 as i32, size.1 as i32);
        let (x, y) = seed;
        if x < 0 || y < 0 || x >= w || y >= h || pre.len() != size.0 as usize * size.1 as usize * 4
        {
            return None;
        }
        if clip.as_ref().is_some_and(|m| m.at(x as u32, y as u32) == 0) {
            return None;
        }
        if boundary
            .as_ref()
            .is_some_and(|b| b.width != size.0 || b.height != size.1)
        {
            return None;
        }
        let target = src_px(boundary.as_ref().map_or(pre, |b| &b.pixels[..]), w, x, y);
        Some(Self {
            w,
            h,
            seed,
            target,
            tolerance: opts.tolerance,
            color: opts.color,
            // Without a boundary, filling a region that already *is* the fill
            // colour is a no-op. With one, the sampled colour comes from a
            // different layer, so it says nothing about what is already painted.
            inert: boundary.is_none() && px_eq(target, opts.color),
            boundary,
            clip,
            plain: None,
            field: None,
            region: None,
            written: None,
            touched: None,
        })
    }

    /// Paint the fill for `gap` and `expand` into `canvas`, first putting back
    /// what the previous call painted. `pre` is the cell as it was before the
    /// press. Returns the pixels this call changed — the old fill's bounds and
    /// the new one's — and marks them dirty on `canvas`. `None` when the press
    /// fills nothing.
    ///
    /// With `expand > 0` the grown ring overwrites whatever is already on the
    /// canvas within that many pixels of the region edge. That is the intended
    /// trade — the overwritten band normally sits underneath the line art.
    pub fn apply(
        &mut self,
        canvas: &mut Canvas,
        pre: &[u8],
        gap: u8,
        expand: u8,
    ) -> Option<DirtyRect> {
        if self.inert || !self.fits(canvas, pre) {
            return None;
        }
        let old = self.written.take();
        if let Some(b) = old {
            restore(canvas, pre, b);
        }
        self.ensure_regions(pre, gap);
        let region = if gap == 0 {
            self.plain.as_ref()
        } else {
            self.region.as_ref()
        }?;
        let clip = self.clip.as_deref();
        let r = expand.min(MAX_EXPAND) as i32;
        let bbox = if r > 0 {
            let grown = dilate_window(&region.mask, self.w, self.h, region.bbox, r);
            let b = grown.bbox();
            paint(canvas, b, |x, y| grown.at(x, y), self.color, clip);
            b
        } else {
            let (mask, w) = (&region.mask, self.w);
            paint(
                canvas,
                region.bbox,
                |x, y| mask[(y * w + x) as usize],
                self.color,
                clip,
            );
            region.bbox
        };
        self.written = Some(bbox);
        let rect = to_rect(old.map_or(bbox, |o| join(o, bbox)));
        mark(canvas, rect);
        self.touched = Some(union_rect(self.touched, rect));
        Some(rect)
    }

    /// Put back what the last [`apply`](Self::apply) painted, as if the press
    /// never happened. Returns the pixels restored, also marked dirty.
    pub fn revert(&mut self, canvas: &mut Canvas, pre: &[u8]) -> Option<DirtyRect> {
        if !self.fits(canvas, pre) {
            return None;
        }
        let b = self.written.take()?;
        restore(canvas, pre, b);
        let rect = to_rect(b);
        mark(canvas, rect);
        Some(rect)
    }

    /// Everything this press has changed so far, over every `apply`.
    pub fn touched(&self) -> Option<DirtyRect> {
        self.touched
    }

    fn fits(&self, canvas: &Canvas, pre: &[u8]) -> bool {
        canvas.width as i32 == self.w
            && canvas.height as i32 == self.h
            && pre.len() == canvas.pixels.len()
    }

    /// Work out the plain flood, and the region for `gap` when that is above
    /// 0, unless they are already to hand.
    fn ensure_regions(&mut self, pre: &[u8], gap: u8) {
        let (w, h, seed) = (self.w, self.h, self.seed);
        let (target, tol) = (self.target, self.tolerance);
        let src = self.boundary.as_ref().map_or(pre, |b| &b.pixels[..]);
        let plain = self.plain.get_or_insert_with(|| {
            let (mask, bbox) =
                span_flood(w, h, seed, |x, y| matches_target(src, w, x, y, target, tol));
            Region { gap: 0, mask, bbox }
        });
        if gap == 0 || self.region.as_ref().is_some_and(|r| r.gap == gap) {
            return;
        }
        // A wider gap needs walls from farther out. Rebuild in doubling
        // steps, so a long drag outward rebuilds only a few times.
        let needed = gap.div_ceil(2) as i32 + 1;
        if self.field.as_ref().is_some_and(|f| !f.whole && f.reach < needed) {
            self.field = None;
        }
        let field = self.field.get_or_insert_with(|| {
            let reach = (needed as u32).next_power_of_two().max(16) as i32;
            wall_distance(src, w, h, plain.bbox, reach, target, tol)
        });
        let (mask, bbox) = gap_flood(field, &plain.mask, w, h, seed, gap);
        self.region = Some(Region { gap, mask, bbox });
    }
}

/// Flood fill on `canvas` starting at integer pixel `(x, y)` using `opts` —
/// one press with no drag. `boundary` as in [`FillSession::new`]; `None` is
/// the plain bucket: read and write the same pixels.
#[cfg(test)]
pub fn flood(canvas: &mut Canvas, boundary: Option<&Canvas>, x: i32, y: i32, opts: FillOptions) {
    flood_clipped(canvas, boundary, x, y, opts, None);
}

/// [`flood`] held to a selection: only pixels `clip` covers are written, a
/// partly covered one is blended toward the fill colour by its coverage, and
/// a click outside the selection does nothing.
#[cfg(test)]
pub fn flood_clipped(
    canvas: &mut Canvas,
    boundary: Option<&Canvas>,
    x: i32,
    y: i32,
    opts: FillOptions,
    clip: Option<&Mask>,
) {
    let pre = canvas.pixels.clone();
    let size = (canvas.width, canvas.height);
    let clip = clip.map(|m| Arc::new(m.clone()));
    if let Some(mut s) = FillSession::new(&pre, size, (x, y), &opts, boundary.cloned(), clip) {
        s.apply(canvas, &pre, opts.gap, opts.expand);
    }
}

/// Fill everything `mask` covers with `color` — Fill selection. Fully covered
/// pixels are replaced, as the bucket does; a feathered edge blends. Returns
/// whether anything was inside the canvas to fill.
pub fn fill_masked(canvas: &mut Canvas, mask: &Mask, color: [u8; 4]) -> bool {
    let (x1, y1) = (
        (mask.x + mask.w).min(canvas.width),
        (mask.y + mask.h).min(canvas.height),
    );
    if x1 <= mask.x || y1 <= mask.y {
        return false;
    }
    for y in mask.y..y1 {
        for x in mask.x..x1 {
            match mask.at(x, y) {
                0 => {}
                255 => write_px(canvas, x as i32, y as i32, color),
                k => blend_toward(canvas, x as i32, y as i32, color, k as f32 / 255.0),
            }
        }
    }
    canvas.mark_dirty(mask.x, mask.y, x1 - mask.x, y1 - mask.y);
    true
}

/// Write `color` over every pixel of `bbox` that `inside` says belongs to the
/// fill, held to `clip` when there is one.
fn paint(
    canvas: &mut Canvas,
    bbox: Bbox,
    inside: impl Fn(i32, i32) -> bool,
    color: [u8; 4],
    clip: Option<&Mask>,
) {
    for yi in bbox.1..=bbox.3 {
        for xi in bbox.0..=bbox.2 {
            if !inside(xi, yi) {
                continue;
            }
            match clip.map(|m| m.at(xi as u32, yi as u32)) {
                None | Some(255) => write_px(canvas, xi, yi, color),
                Some(0) => {}
                Some(k) => blend_toward(canvas, xi, yi, color, k as f32 / 255.0),
            }
        }
    }
}

/// Copy `bbox` of `pre` back over `canvas`, row by row.
fn restore(canvas: &mut Canvas, pre: &[u8], b: Bbox) {
    let w = canvas.width as usize;
    for y in b.1 as usize..=b.3 as usize {
        let s = (y * w + b.0 as usize) * 4;
        let e = (y * w + b.2 as usize + 1) * 4;
        canvas.pixels[s..e].copy_from_slice(&pre[s..e]);
    }
}

fn mark(canvas: &mut Canvas, r: DirtyRect) {
    canvas.mark_dirty(r.min_x, r.min_y, r.max_x - r.min_x, r.max_y - r.min_y);
}

fn to_rect(b: Bbox) -> DirtyRect {
    DirtyRect {
        min_x: b.0 as u32,
        min_y: b.1 as u32,
        max_x: b.2 as u32 + 1,
        max_y: b.3 as u32 + 1,
    }
}

fn join(a: Bbox, b: Bbox) -> Bbox {
    (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))
}

/// Move a pixel `t` of the way toward `color`, lerping premultiplied so a
/// half-covered pixel over transparency keeps the fill's own colour rather
/// than going muddy.
fn blend_toward(canvas: &mut Canvas, x: i32, y: i32, color: [u8; 4], t: f32) {
    let p = read_px(canvas, x, y);
    let (a0, a1) = (p[3] as f32 / 255.0, color[3] as f32 / 255.0);
    let a = a0 + (a1 - a0) * t;
    if a <= 0.0 {
        write_px(canvas, x, y, [0, 0, 0, 0]);
        return;
    }
    let ch = |i: usize| {
        let v = (p[i] as f32 * a0 * (1.0 - t) + color[i] as f32 * a1 * t) / a;
        v.round().clamp(0.0, 255.0) as u8
    };
    write_px(
        canvas,
        x,
        y,
        [
            ch(0),
            ch(1),
            ch(2),
            (a * 255.0).round().clamp(0.0, 255.0) as u8,
        ],
    );
}

/// Scanline span flood over a `w`×`h` grid from `(x, y)`, entering every
/// pixel `pass` accepts. The start itself is taken as given. Returns the
/// reached pixels as a `w * h` bitmap plus their bounding box.
fn span_flood(
    w: i32,
    h: i32,
    (x, y): (i32, i32),
    pass: impl Fn(i32, i32) -> bool,
) -> (Vec<bool>, Bbox) {
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
        while x0 > 0 && pass(x0 - 1, sy) {
            x0 -= 1;
        }
        // Walk right to span end.
        let mut x1 = sx;
        while x1 + 1 < w && pass(x1 + 1, sy) {
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
                let above_match = pass(xi, sy - 1);
                if above_match && !span_above_open {
                    queue.push_back((xi, sy - 1));
                    span_above_open = true;
                } else if !above_match {
                    span_above_open = false;
                }
            }
            if sy + 1 < h {
                let below_match = pass(xi, sy + 1);
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

/// The in-bounds 4-neighbours of `(x, y)`.
fn neighbours(x: i32, y: i32, w: i32, h: i32) -> impl Iterator<Item = (i32, i32)> {
    [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
        .into_iter()
        .filter(move |&(a, b)| a >= 0 && b >= 0 && a < w && b < h)
}

/// Who a pixel belongs to while [`gap_flood`] runs.
const FREE: u8 = 0;
const OURS: u8 = 1;
const THEIRS: u8 = 2;
/// A wall, or a pixel the plain flood never reached: nobody's.
const OUT: u8 = 3;

/// The region of a fill from `seed` that shuts gaps up to `gap` pixels wide,
/// on a `cw`×`ch` cell whose plain flood is `plain`. See the module docs.
///
/// A watershed with markers. Open space — farther from every wall than half
/// the gap — comes in pieces that only gaps join. The piece the click belongs
/// to is ours; every other piece is someone else's. The narrow band between
/// open space and the walls is then handed out by flooding down from all of
/// them at once, highest distance first, each pixel going to whoever reaches
/// it first. The band inside a gap's throat is the last to go, and splits
/// between the two sides, so neither pours through.
///
/// Works in the field's window, which holds the whole plain flood.
fn gap_flood(
    field: &Field,
    plain: &[bool],
    cw: i32,
    ch: i32,
    seed: (i32, i32),
    gap: u8,
) -> (Vec<bool>, Bbox) {
    let (fx, fy, w, h) = (field.x0, field.y0, field.w, field.h);
    let dist = &field.dist[..];
    let at = |x: i32, y: i32| dist[(y * w + x) as usize];
    // On a pixel grid a gap an odd number of pixels wide has a centre pixel as
    // far out as the next even width's, so odd settings shut one pixel more.
    let half = gap.div_ceil(2) as f32;
    let open = half * half;
    let mut label: Vec<u8> = (0..h)
        .flat_map(|y| (0..w).map(move |x| ((y + fy) * cw + x + fx) as usize))
        .map(|i| if plain[i] { FREE } else { OUT })
        .collect();

    // Climb from the click to the top of its hill, so a click right beside a
    // line is judged by the open space it belongs to. Steepest ascent, and
    // across a level stretch when one blocks the way up.
    let seed = (seed.0 - fx, seed.1 - fy);
    let mut path = vec![seed];
    let mut seen: Vec<bool> = Vec::new();
    let mut flat: Vec<(i32, i32)> = Vec::new();
    let mut p = seed;
    loop {
        let best =
            neighbours(p.0, p.1, w, h)
                .fold(p, |b, q| if at(q.0, q.1) > at(b.0, b.1) { q } else { b });
        if best != p {
            p = best;
            path.push(p);
            continue;
        }
        let level = at(p.0, p.1);
        if level > open {
            break;
        }
        if seen.is_empty() {
            seen = vec![false; (w * h) as usize];
        }
        flat.clear();
        let mut queue = VecDeque::from([p]);
        seen[(p.1 * w + p.0) as usize] = true;
        let mut up = None;
        while let Some(q) = queue.pop_front() {
            flat.push(q);
            for r in neighbours(q.0, q.1, w, h) {
                let d = at(r.0, r.1);
                if d > level {
                    up = up.or(Some(r));
                } else if d == level && !seen[(r.1 * w + r.0) as usize] {
                    seen[(r.1 * w + r.0) as usize] = true;
                    queue.push_back(r);
                }
            }
        }
        match up {
            Some(r) => {
                p = r;
                path.push(p);
            }
            None => break,
        }
    }

    if at(p.0, p.1) > open {
        let (core, _) = span_flood(w, h, p, |x, y| at(x, y) > open);
        for (l, _) in label.iter_mut().zip(&core).filter(|(_, &c)| c) {
            *l = OURS;
        }
    } else {
        // The hill never reaches open space: a shape smaller than the gap
        // itself. Its own top stands in, so it fills instead of vanishing.
        for &(x, y) in &flat {
            label[(y * w + x) as usize] = OURS;
        }
    }
    for &(x, y) in &path {
        label[(y * w + x) as usize] = OURS;
    }
    for (l, &d) in label.iter_mut().zip(dist) {
        if *l == FREE && d > open {
            *l = THEIRS;
        }
    }

    // Everything still free is band, no farther out than `open`, so its
    // squared distance — a whole number — indexes a bucket directly.
    let top = open as usize;
    let mut buckets: Vec<VecDeque<u32>> = vec![VecDeque::new(); top + 1];
    let claim = |label: &mut [u8], buckets: &mut [VecDeque<u32>], from: u8, x: i32, y: i32| {
        let mut highest = 0;
        for (a, b) in neighbours(x, y, w, h) {
            let i = (b * w + a) as usize;
            if label[i] == FREE {
                label[i] = from;
                let k = dist[i] as usize;
                buckets[k].push_back(i as u32);
                highest = highest.max(k);
            }
        }
        highest
    };
    // Gather the markers before any claim: a pixel claimed during this pass
    // must wait its turn in the buckets, or labels would creep along the band
    // in scan order instead of by distance.
    let markers: Vec<(i32, i32)> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            matches!(label[(y * w + x) as usize], OURS | THEIRS)
                && neighbours(x, y, w, h).any(|(a, b)| label[(b * w + a) as usize] == FREE)
        })
        .collect();
    for (x, y) in markers {
        let l = label[(y * w + x) as usize];
        claim(&mut label, &mut buckets, l, x, y);
    }
    let mut cur = top;
    loop {
        while cur > 0 && buckets[cur].is_empty() {
            cur -= 1;
        }
        let Some(i) = buckets[cur].pop_front() else {
            break;
        };
        let (x, y) = (i as i32 % w, i as i32 / w);
        let l = label[i as usize];
        // A spot that rises again past a dip is reached from below, so the
        // flood may have to step back up a level.
        cur = cur.max(claim(&mut label, &mut buckets, l, x, y));
    }

    let (mut x0, mut y0, mut x1, mut y1) = (cw, ch, -1, -1);
    let mut fill = vec![false; (cw * ch) as usize];
    for y in 0..h {
        for x in 0..w {
            if label[(y * w + x) as usize] == OURS {
                let (x, y) = (x + fx, y + fy);
                fill[(y * cw + x) as usize] = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (fill, (x0, y0, x1, y1))
}

/// Squared distance of a parabola that never touches down — a line with no
/// wall on it. Finite, so the arithmetic below never meets `inf - inf`.
const FAR: f64 = 1e30;

/// The [`Field`] for a plain flood with bounds `bbox` on a `w`×`h` cell:
/// squared Euclidean distance from each pixel to the nearest wall — a pixel
/// the flood would not enter — with the cell edge counting as one, so a line
/// that stops just short of the edge still shuts. 0 on the walls.
/// Felzenszwalb & Huttenlocher's exact transform: along each row, then down
/// each column.
///
/// The window is `bbox` grown by `reach`. Walls outside it are farther from
/// the flood than the gaps it serves look, so leaving them out changes nothing
/// that matters.
fn wall_distance(
    src: &[u8],
    w: i32,
    h: i32,
    bbox: Bbox,
    reach: i32,
    target: [u8; 4],
    tol: u8,
) -> Field {
    let (x0, y0) = ((bbox.0 - reach).max(0), (bbox.1 - reach).max(0));
    let (x1, y1) = ((bbox.2 + reach).min(w - 1), (bbox.3 + reach).min(h - 1));
    let (fw, fh) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
    let n = fw.max(fh);
    let mut f = vec![0f64; n];
    let mut d = vec![0f64; n];
    let mut v = vec![0usize; n];
    let mut z = vec![0f64; n + 1];
    let mut dist = vec![0f32; fw * fh];
    for y in 0..fh {
        let cy = y0 + y as i32;
        for (x, fx) in f[..fw].iter_mut().enumerate() {
            let wall = !matches_target(src, w, x0 + x as i32, cy, target, tol);
            *fx = if wall { 0.0 } else { FAR };
        }
        edt_line(&f[..fw], &mut d[..fw], &mut v, &mut z);
        for (o, &dx) in dist[y * fw..(y + 1) * fw].iter_mut().zip(&d[..fw]) {
            *o = dx as f32;
        }
    }
    for x in 0..fw {
        for (y, fy) in f[..fh].iter_mut().enumerate() {
            *fy = dist[y * fw + x] as f64;
        }
        edt_line(&f[..fh], &mut d[..fh], &mut v, &mut z);
        let cx = x0 + x as i32;
        let edge_x = (cx + 1).min(w - cx);
        for (y, &dy) in d[..fh].iter().enumerate() {
            let cy = y0 + y as i32;
            let edge = edge_x.min(cy + 1).min(h - cy) as f64;
            dist[y * fw + x] = dy.min(edge * edge) as f32;
        }
    }
    Field {
        x0,
        y0,
        w: fw as i32,
        h: fh as i32,
        reach,
        whole: x0 == 0 && y0 == 0 && x1 == w - 1 && y1 == h - 1,
        dist,
    }
}

/// One line of the squared distance transform: `d[q] = min over p of
/// (q - p)² + f[p]`, via the lower envelope of those parabolas. `v` and `z`
/// are scratch, at least `f.len()` and one more.
fn edt_line(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let meet = |q: usize, p: usize| {
        ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * (q - p) as f64)
    };
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let mut s = meet(q, v[k]);
        // `z[0]` is -inf and `s` is finite, so this stops before `k` runs out.
        while s <= z[k] {
            k -= 1;
            s = meet(q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, dq) in d.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        let t = q as f64 - p as f64;
        *dq = t * t + f[p];
    }
}

/// A grown region: a window of the canvas and its bits.
struct Grown {
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    bits: Vec<bool>,
}

impl Grown {
    fn at(&self, x: i32, y: i32) -> bool {
        self.bits[((y - self.y0) * self.w + (x - self.x0)) as usize]
    }

    fn bbox(&self) -> Bbox {
        (self.x0, self.y0, self.x0 + self.w - 1, self.y0 + self.h - 1)
    }
}

/// `mask` grown by `r` pixels with a square kernel, clamped to the canvas —
/// the same pixels as a plain separable dilate of the whole canvas, but worked
/// over the region's bounds grown by `r` only, with running counts, so the
/// cost depends on neither `r` nor the canvas size.
fn dilate_window(mask: &[bool], w: i32, h: i32, bbox: Bbox, r: i32) -> Grown {
    let (x0, y0) = ((bbox.0 - r).max(0), (bbox.1 - r).max(0));
    let (x1, y1) = ((bbox.2 + r).min(w - 1), (bbox.3 + r).min(h - 1));
    let (gw, gh) = (x1 - x0 + 1, y1 - y0 + 1);

    // Horizontal: count the set pixels in [x - r, x + r] along each row of the
    // region. Nothing outside `bbox` is set, so no other row needs a look.
    let mut rows = vec![false; (gw * gh) as usize];
    for y in bbox.1..=bbox.3 {
        let src = &mask[(y * w) as usize..((y + 1) * w) as usize];
        let set = |x: i32| x >= bbox.0 && x <= bbox.2 && src[x as usize];
        let out = ((y - y0) * gw) as usize;
        let mut count = (x0 - r..=x0 + r).filter(|&x| set(x)).count() as i32;
        for x in x0..=x1 {
            if x > x0 {
                count += set(x + r) as i32 - set(x - r - 1) as i32;
            }
            rows[out + (x - x0) as usize] = count > 0;
        }
    }

    // Vertical, the same down the rows just built.
    let mut bits = vec![false; (gw * gh) as usize];
    for x in 0..gw {
        let set = |y: i32| y >= 0 && y < gh && rows[(y * gw + x) as usize];
        let mut count = (-r..=r).filter(|&y| set(y)).count() as i32;
        for y in 0..gh {
            if y > 0 {
                count += set(y + r) as i32 - set(y - r - 1) as i32;
            }
            bits[(y * gw + x) as usize] = count > 0;
        }
    }
    Grown {
        x0,
        y0,
        w: gw,
        h: gh,
        bits,
    }
}

#[inline]
fn read_px(canvas: &Canvas, x: i32, y: i32) -> [u8; 4] {
    src_px(&canvas.pixels, canvas.width as i32, x, y)
}

/// Pixel `(x, y)` of an RGBA8 buffer `w` pixels wide.
#[inline]
fn src_px(src: &[u8], w: i32, x: i32, y: i32) -> [u8; 4] {
    let idx = ((y * w + x) * 4) as usize;
    [src[idx], src[idx + 1], src[idx + 2], src[idx + 3]]
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
fn matches_target(src: &[u8], w: i32, x: i32, y: i32, target: [u8; 4], tol: u8) -> bool {
    let p = src_px(src, w, x, y);
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
            gap: 0,
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
        assert!(
            filled_at(&target, 6, 8),
            "expand should tuck under the wall"
        );
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
        assert!(
            !filled_at(&target, 15, 8),
            "no wrap past the wall to the far edge"
        );
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

    /// Left half of a 16x16 canvas selected at coverage `k`.
    fn left_half(k: u8) -> Mask {
        Mask {
            x: 0,
            y: 0,
            w: 8,
            h: 16,
            cov: vec![k; 8 * 16],
        }
    }

    #[test]
    fn a_selection_holds_the_fill_inside_it() {
        let mut target = Canvas::new(16, 16);
        flood_clipped(&mut target, None, 2, 8, opts(0), Some(&left_half(255)));
        assert!(filled_at(&target, 7, 3));
        assert!(!filled_at(&target, 8, 3), "past the selection edge");
        assert_eq!(read_px(&target, 12, 3)[3], 0);
    }

    #[test]
    fn a_click_outside_the_selection_fills_nothing() {
        let mut target = Canvas::new(16, 16);
        target.dirty = None;
        flood_clipped(&mut target, None, 12, 8, opts(0), Some(&left_half(255)));
        assert!(target.pixels.iter().all(|&b| b == 0));
        assert!(target.dirty.is_none());
    }

    #[test]
    fn a_feathered_selection_blends_the_fill() {
        let mut target = Canvas::new(16, 16);
        flood_clipped(&mut target, None, 2, 8, opts(0), Some(&left_half(128)));
        let p = read_px(&target, 3, 3);
        assert!((p[3] as i32 - 128).abs() <= 1, "alpha {}", p[3]);
        assert_eq!(&p[..3], &RED[..3], "colour stays the fill's own");
    }

    #[test]
    fn fill_selection_paints_only_the_mask() {
        let mut target = Canvas::new(16, 16);
        assert!(fill_masked(&mut target, &left_half(255), RED));
        assert!(filled_at(&target, 0, 0) && filled_at(&target, 7, 15));
        assert_eq!(read_px(&target, 8, 0)[3], 0);
    }

    // --- Gap closing ---

    const INK: [u8; 4] = [0, 0, 0, 255];

    /// A `w`×`h` line layer with opaque ink wherever `wall` says.
    fn lines(w: u32, h: u32, wall: impl Fn(i32, i32) -> bool) -> Canvas {
        let mut c = Canvas::new(w, h);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                if wall(x, y) {
                    write_px(&mut c, x, y, INK);
                }
            }
        }
        c
    }

    /// A fresh paint layer filled from `(x, y)` against `boundary`.
    fn fill_on(boundary: &Canvas, x: i32, y: i32, gap: u8, expand: u8) -> Canvas {
        let mut target = Canvas::new(boundary.width, boundary.height);
        let o = FillOptions {
            gap,
            ..opts(expand)
        };
        flood(&mut target, Some(boundary), x, y, o);
        target
    }

    fn count_filled(c: &Canvas) -> usize {
        c.pixels.chunks_exact(4).filter(|p| *p == RED).count()
    }

    /// 1-px outline of the box `x0..=x1` × `y0..=y1`, its top edge broken by a
    /// hole `hole` pixels wide starting at `hole_x`.
    fn box_outline(
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        hole_x: i32,
        hole: i32,
    ) -> impl Fn(i32, i32) -> bool {
        move |x, y| {
            let edge = ((x == x0 || x == x1) && (y0..=y1).contains(&y))
                || ((y == y0 || y == y1) && (x0..=x1).contains(&x));
            let in_hole = y == y0 && x >= hole_x && x < hole_x + hole;
            edge && !in_hole
        }
    }

    fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
    }

    /// Anti-aliased ring of radius 20 round (32, 32): the core is solid and the
    /// rim fades, so the walls the flood sees are ragged.
    fn soft_ring() -> Canvas {
        let mut c = Canvas::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let d = ((x - 32) as f32).hypot((y - 32) as f32);
                let cov = (1.8 - (d - 20.0).abs()).clamp(0.0, 1.0);
                if cov > 0.0 {
                    write_px(&mut c, x, y, [0, 0, 0, (cov * 255.0) as u8]);
                }
            }
        }
        c
    }

    #[test]
    fn closed_shapes_fill_the_same_at_any_gap() {
        let ring = |x: i32, y: i32| (((x - 32) as f32).hypot((y - 32) as f32) - 20.0).abs() <= 1.0;
        let tri = |x: i32, y: i32| {
            let p = (x as f32, y as f32);
            let (a, b, c) = ((8.0, 56.0), (56.0, 56.0), (18.0, 6.0));
            seg_dist(p, a, b)
                .min(seg_dist(p, b, c))
                .min(seg_dist(p, c, a))
                <= 1.0
        };
        // An L-shaped room: everything in the box but the L is ink.
        let l_room = |x: i32, y: i32| {
            let in_box = (8..=56).contains(&x) && (8..=56).contains(&y);
            let in_l = ((9..=31).contains(&x) && (9..=55).contains(&y))
                || ((9..=55).contains(&x) && (33..=55).contains(&y));
            in_box && !in_l
        };
        let shapes = [
            ("ring", lines(64, 64, ring), (32, 32)),
            (
                "box",
                lines(64, 64, box_outline(8, 8, 56, 56, 0, 0)),
                (30, 30),
            ),
            (
                "outside the box",
                lines(64, 64, box_outline(8, 8, 56, 56, 0, 0)),
                (2, 2),
            ),
            ("triangle", lines(64, 64, tri), (28, 45)),
            ("acute corner", lines(64, 64, tri), (19, 12)),
            ("L", lines(64, 64, l_room), (15, 50)),
            ("soft ring", soft_ring(), (32, 32)),
        ];
        for (name, b, (x, y)) in &shapes {
            let plain = fill_on(b, *x, *y, 0, 0);
            assert!(count_filled(&plain) > 0, "{name}");
            for gap in [2, 5, 6, 12, MAX_GAP] {
                let closed = fill_on(b, *x, *y, gap, 0);
                assert_eq!(
                    count_filled(&closed),
                    count_filled(&plain),
                    "{name}: gap {gap} changed a closed fill"
                );
                assert!(closed.pixels == plain.pixels, "{name}: gap {gap}");
            }
        }
    }

    #[test]
    fn a_gap_up_to_the_setting_holds_the_fill() {
        // (hole width, gap setting, holds)
        for (hole, gap, holds) in [
            (4, 4, true),
            (4, 6, true),
            (4, 2, false),
            (4, 0, false),
            (6, 6, true),
            (6, 4, false),
            (5, 5, true),
            (3, 3, true),
            (1, 1, true),
        ] {
            let b = lines(64, 64, box_outline(16, 16, 48, 48, 30, hole));
            let out = fill_on(&b, 32, 32, gap, 0);
            assert_eq!(!filled_at(&out, 2, 2), holds, "hole {hole}, gap {gap}");
            assert!(
                filled_at(&out, 17, 17) && filled_at(&out, 47, 47),
                "corners: hole {hole}, gap {gap}"
            );
            if holds {
                // Right up to the hole on the inside, nothing beyond it.
                assert!(filled_at(&out, 30, 17), "hole {hole}, gap {gap}");
                assert!(!filled_at(&out, 31, 14), "hole {hole}, gap {gap}");
            }
        }
    }

    #[test]
    fn a_line_ending_short_of_another_does_not_leak_along_it() {
        // A T whose stem stops 2 px short of the bar: the classic gap. The band
        // along the bar runs straight past the stem's end at one distance.
        let b = lines(64, 64, |x, y| y == 10 || (x == 32 && y >= 13));
        assert!(
            filled_at(&fill_on(&b, 16, 40, 0, 0), 48, 40),
            "no gap: leaks"
        );
        let out = fill_on(&b, 16, 40, 4, 0);
        assert!(!filled_at(&out, 48, 40));
        assert!(!filled_at(&out, 48, 11), "not even along the bar");
        assert!(filled_at(&out, 16, 11) && filled_at(&out, 31, 63));
    }

    #[test]
    fn a_click_beside_a_line_still_shuts_the_gap() {
        let b = lines(64, 64, box_outline(16, 16, 48, 48, 30, 4));
        let out = fill_on(&b, 31, 17, 4, 0);
        assert!(!filled_at(&out, 2, 2));
        assert!(filled_at(&out, 40, 40));
    }

    #[test]
    fn a_shape_smaller_than_the_gap_still_fills() {
        // A 4×4 room.
        let b = lines(32, 32, box_outline(10, 10, 15, 15, 0, 0));
        let plain = fill_on(&b, 12, 12, 0, 0);
        assert_eq!(count_filled(&plain), 16);
        assert!(fill_on(&b, 12, 12, 12, 0).pixels == plain.pixels);
    }

    #[test]
    fn a_line_stopping_short_of_the_edge_counts_as_shut() {
        // Down x = 24, stopping 2 px short of the top.
        let b = lines(48, 48, |x, y| x == 24 && y >= 2);
        assert!(
            filled_at(&fill_on(&b, 8, 24, 0, 0), 40, 24),
            "no gap: leaks round the end"
        );
        let out = fill_on(&b, 8, 24, 4, 0);
        assert!(!filled_at(&out, 40, 24) && !filled_at(&out, 40, 0));
        assert!(filled_at(&out, 0, 0) && filled_at(&out, 23, 47));
    }

    // --- Drag session ---

    #[test]
    fn a_session_redoes_the_fill_as_the_values_change() {
        let b = lines(64, 64, box_outline(16, 16, 48, 48, 30, 4));
        let mut canvas = Canvas::new(64, 64);
        let pre = canvas.pixels.clone();
        let o = opts(0);
        let mut s = FillSession::new(&pre, (64, 64), (32, 32), &o, Some(b.clone()), None).unwrap();
        for (gap, expand) in [(0, 0), (4, 0), (4, 3), (0, 2), (6, 1)] {
            s.apply(&mut canvas, &pre, gap, expand).unwrap();
            assert!(
                canvas.pixels == fill_on(&b, 32, 32, gap, expand).pixels,
                "gap {gap}, expand {expand}"
            );
        }
        // The first apply flooded everything; undo must cover all of it.
        let t = s.touched().unwrap();
        assert_eq!((t.min_x, t.min_y, t.max_x, t.max_y), (0, 0, 64, 64));
        s.revert(&mut canvas, &pre).unwrap();
        assert!(canvas.pixels == pre);
    }

    #[test]
    fn a_same_layer_session_reads_its_lines_from_before_the_press() {
        // Expand paints over the layer's own lines; a later apply must still
        // see them as they were.
        let lines_here = lines(64, 64, box_outline(16, 16, 48, 48, 30, 4));
        let mut canvas = lines_here.clone();
        let pre = canvas.pixels.clone();
        let o = opts(0);
        let mut s = FillSession::new(&pre, (64, 64), (32, 32), &o, None, None).unwrap();
        s.apply(&mut canvas, &pre, 4, 3).unwrap();
        s.apply(&mut canvas, &pre, 4, 0).unwrap();
        let mut fresh = lines_here.clone();
        flood(&mut fresh, None, 32, 32, FillOptions { gap: 4, ..o });
        assert!(canvas.pixels == fresh.pixels);
        assert!(filled_at(&canvas, 20, 20) && !filled_at(&canvas, 2, 2));
    }

    #[test]
    fn an_inert_press_never_paints() {
        let mut canvas = Canvas::new(8, 8);
        let pre = [255u8, 0, 0, 255].repeat(64);
        canvas.pixels.copy_from_slice(&pre);
        let mut s = FillSession::new(&pre, (8, 8), (1, 1), &opts(0), None, None).unwrap();
        assert!(s.apply(&mut canvas, &pre, 4, 4).is_none());
        assert!(s.touched().is_none());
    }

    #[test]
    fn the_window_dilate_matches_the_plain_one() {
        /// The whole-canvas dilate this module used to run.
        fn reference(mask: &mut [bool], w: i32, h: i32, r: i32) {
            let mut tmp = vec![false; mask.len()];
            for y in 0..h {
                for x in 0..w {
                    let (lo, hi) = ((x - r).max(0), (x + r).min(w - 1));
                    tmp[(y * w + x) as usize] = (lo..=hi).any(|xi| mask[(y * w + xi) as usize]);
                }
            }
            for y in 0..h {
                let (lo, hi) = ((y - r).max(0), (y + r).min(h - 1));
                for x in 0..w {
                    mask[(y * w + x) as usize] = (lo..=hi).any(|yi| tmp[(yi * w + x) as usize]);
                }
            }
        }
        let (w, h) = (37, 29);
        let mut seed = 12345u32;
        for round in 0..6 {
            // A sparse random blob, kept off one side so the window clamps on
            // some edges and not others.
            let mut mask = vec![false; (w * h) as usize];
            let (mut x0, mut y0, mut x1, mut y1) = (w, h, -1, -1);
            for y in 3..h - round {
                for x in round..w - 5 {
                    seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                    if (seed >> 16) % 23 == 0 {
                        mask[(y * w + x) as usize] = true;
                        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                    }
                }
            }
            for r in 1..=MAX_EXPAND as i32 {
                let grown = dilate_window(&mask, w, h, (x0, y0, x1, y1), r);
                let mut want = mask.clone();
                reference(&mut want, w, h, r);
                for y in 0..h {
                    for x in 0..w {
                        let (gx0, gy0, gx1, gy1) = grown.bbox();
                        let got = x >= gx0 && x <= gx1 && y >= gy0 && y <= gy1 && grown.at(x, y);
                        assert_eq!(
                            got,
                            want[(y * w + x) as usize],
                            "round {round}, r {r}, ({x}, {y})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_wide_gap_holds_and_the_field_grows_to_serve_it() {
        // A closed room split down the middle by a wall with a 40-px doorway.
        // The room is small against the cell, so the field starts out as a
        // window, too narrow for the second gap asked for.
        let room = |x: i32, y: i32| {
            let outer = ((x == 40 || x == 216) && (40..=216).contains(&y))
                || ((y == 40 || y == 216) && (40..=216).contains(&x));
            let split = x == 128 && (40..=216).contains(&y) && !(108..148).contains(&y);
            outer || split
        };
        let b = lines(512, 512, room);
        let mut canvas = Canvas::new(512, 512);
        let pre = canvas.pixels.clone();
        let mut s = FillSession::new(&pre, (512, 512), (80, 128), &opts(0), Some(b.clone()), None)
            .unwrap();
        s.apply(&mut canvas, &pre, 4, 0).unwrap();
        assert!(filled_at(&canvas, 170, 128), "gap 4: pours through the doorway");
        s.apply(&mut canvas, &pre, 48, 0).unwrap();
        assert!(!filled_at(&canvas, 170, 128), "gap 48: held");
        assert!(filled_at(&canvas, 41, 41) && filled_at(&canvas, 127, 215));
        assert!(canvas.pixels == fill_on(&b, 80, 128, 48, 0).pixels);
    }

    #[test]
    fn gap_drags_climb_a_ladder_that_spreads_out() {
        assert_eq!(step_gap(0, 0), 0);
        assert_eq!(step_gap(0, 4), 4);
        assert_eq!(step_gap(12, 1), 14);
        assert_eq!(step_gap(0, -3), 0);
        assert_eq!(step_gap(0, 1000), MAX_GAP);
        // Off the ladder: stays put until moved, then takes the next rung.
        assert_eq!(step_gap(50, 0), 50);
        assert_eq!(step_gap(50, 1), 56);
        assert_eq!(step_gap(50, -1), 48);
        assert_eq!(step_gap(13, -1), 12);
        // Every rung is reachable, and in order.
        let mut last = 0;
        for n in 1..GAP_LADDER.len() as i32 {
            let g = step_gap(0, n);
            assert!(g > last, "rung {n}");
            last = g;
        }
        assert_eq!(last, MAX_GAP);
    }

    #[test]
    fn drag_steps_wait_out_the_dead_zone() {
        assert_eq!(drag_steps(0.0), 0);
        assert_eq!(drag_steps(7.9), 0);
        assert_eq!(drag_steps(19.9), 0);
        assert_eq!(drag_steps(20.0), 1);
        assert_eq!(drag_steps(-20.0), -1);
        assert_eq!(drag_steps(40.0), 2);
        assert_eq!(drag_steps(f32::NAN), 0);
    }

    /// Cost at a 1080p frame, for the record: `cargo test --release fill_timing
    /// -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn fill_timing() {
        use std::time::Instant;
        let (w, h) = (1920u32, 1080u32);
        // A grid of rings over an otherwise empty frame; every other one is
        // broken top and bottom.
        let b = lines(w, h, |x, y| {
            let (cx, cy) = ((x / 160) * 160 + 80, (y / 160) * 160 + 80);
            let d = ((x - cx) as f32).hypot((y - cy) as f32);
            let broken = (x / 160 + y / 160) % 2 == 0 && (x - cx).abs() <= 3;
            (d - 60.0).abs() <= 1.5 && !broken
        });
        let pre = vec![0u8; (w * h * 4) as usize];
        let o = opts(0);
        let mut canvas = Canvas::new(w, h);
        let time = |label: &str, f: &mut dyn FnMut()| {
            let t = Instant::now();
            f();
            println!("{label}: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        };
        // (240, 80) is the middle of a closed ring.
        let mut s = FillSession::new(&pre, (w, h), (240, 80), &o, Some(b.clone()), None).unwrap();
        time("ring: plain press", &mut || {
            s.apply(&mut canvas, &pre, 0, 0);
        });
        time("ring: expand 4 step", &mut || {
            s.apply(&mut canvas, &pre, 0, 4);
        });
        time("ring: gap 6 step (builds the field)", &mut || {
            s.apply(&mut canvas, &pre, 6, 0);
        });
        time("ring: gap 8 step", &mut || {
            s.apply(&mut canvas, &pre, 8, 0);
        });
        time("ring: gap 64 step (grows the field)", &mut || {
            s.apply(&mut canvas, &pre, 64, 0);
        });
        s.revert(&mut canvas, &pre);
        let mut s = FillSession::new(&pre, (w, h), (5, 5), &o, Some(b.clone()), None).unwrap();
        time("background: press with expand 4", &mut || {
            s.apply(&mut canvas, &pre, 0, 4);
        });
        s.revert(&mut canvas, &pre);
        let mut s = FillSession::new(&pre, (w, h), (5, 5), &o, Some(b), None).unwrap();
        time("background: plain press", &mut || {
            s.apply(&mut canvas, &pre, 0, 0);
        });
        time("background: expand 4 step", &mut || {
            s.apply(&mut canvas, &pre, 0, 4);
        });
        time("background: gap 6 step (builds the field)", &mut || {
            s.apply(&mut canvas, &pre, 6, 0);
        });
        time("background: gap 8 step", &mut || {
            s.apply(&mut canvas, &pre, 8, 0);
        });
        time("background: expand 3 step at gap 8", &mut || {
            s.apply(&mut canvas, &pre, 8, 3);
        });
        time("background: gap 64 step", &mut || {
            s.apply(&mut canvas, &pre, 64, 0);
        });
        time("background: gap 255 step", &mut || {
            s.apply(&mut canvas, &pre, 255, 0);
        });
    }
}
