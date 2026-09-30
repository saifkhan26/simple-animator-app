//! Clipping: a layer shown only where its *base* — the first layer under it
//! that isn't clipped itself — has pixels. Several clipped layers in a row
//! share one base, as in Photoshop or Clip Studio.
//!
//! The mask is the base drawing's own alpha on that frame. It is applied to
//! the clipped drawing's pixels in the clipped cell's own space, so the result
//! composites exactly as an ordinary drawing would: the canvas draws it as a
//! plain texture, and export composites it as a plain layer.
//!
//! Nothing is destroyed: the clipped layer keeps every pixel it has, and the
//! mask is worked out afresh wherever it is shown. Only a send to Krita bakes
//! it in ([`bake_clipping`]).

use std::collections::HashMap;

use crate::doc::canvas::{Canvas, DirtyRect};
use crate::doc::layer::CellId;
use crate::doc::project::Project;
use crate::doc::transform::Transform;

/// How a clipped drawing and its base sit in a `pw`×`ph` document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub xf: Transform,
    pub base_xf: Transform,
    pub pw: u32,
    pub ph: u32,
}

impl Placement {
    /// Where layer `li` and its base `base` sit on `frame`.
    pub fn of(p: &Project, li: usize, base: usize, frame: usize) -> Self {
        Self {
            xf: p.layers[li].resolve_transform(frame),
            base_xf: p.layers[base].resolve_transform(frame),
            pw: p.width,
            ph: p.height,
        }
    }
}

impl Placement {
    /// Whether a base pixel lies straight under the clipped pixel with the
    /// same index: both layers placed alike on same-sized cells, the common
    /// case, and the one where a change to the base maps 1:1 onto the clip.
    pub fn aligned(&self, clip: &Canvas, base: &Canvas) -> bool {
        self.xf == self.base_xf && base.width == clip.width && base.height == clip.height
    }
}

/// `a` scaled by `k`, both 0..=255 fractions.
#[inline]
fn mul(a: u8, k: u8) -> u8 {
    ((a as u32 * k as u32 + 127) / 255) as u8
}

/// Where a clip-cell pixel centre lands in a base cell that sits elsewhere, as
/// the bilinear sample position there: `u = ux·x + uy·y + u0`, likewise `v`.
/// Both placements are similarities, so the whole chain is affine and three
/// points pin it down.
struct BaseMap {
    ux: f32,
    uy: f32,
    u0: f32,
    vx: f32,
    vy: f32,
    v0: f32,
}

impl BaseMap {
    fn new(clip: &Canvas, base: &Canvas, at: &Placement) -> Self {
        let (cw, ch) = (clip.width as f32, clip.height as f32);
        let (bw, bh) = (base.width as f32, base.height as f32);
        let (pw, ph) = (at.pw as f32, at.ph as f32);
        let to = |x: f32, y: f32| {
            let (dx, dy) = at.xf.cell_to_doc(x + 0.5, y + 0.5, cw, ch, pw, ph);
            let (u, v) = at.base_xf.doc_to_cell(dx, dy, bw, bh, pw, ph);
            (u - 0.5, v - 0.5)
        };
        let (o, ex, ey) = (to(0.0, 0.0), to(1.0, 0.0), to(0.0, 1.0));
        Self {
            ux: ex.0 - o.0,
            uy: ey.0 - o.0,
            u0: o.0,
            vx: ex.1 - o.1,
            vy: ey.1 - o.1,
            v0: o.1,
        }
    }
}

/// Bilinear alpha of `b` at `(u, v)`, 0 off its edge.
#[inline]
fn alpha_at(b: &Canvas, u: f32, v: f32) -> u8 {
    let (w, h) = (b.width as i32, b.height as i32);
    if u < -0.5 || v < -0.5 || u > w as f32 - 0.5 || v > h as f32 - 0.5 {
        return 0;
    }
    let (x0, y0) = (u.floor() as i32, v.floor() as i32);
    let (fx, fy) = (u - x0 as f32, v - y0 as f32);
    let a = |x: i32, y: i32| {
        b.pixels[((y.clamp(0, h - 1) * w + x.clamp(0, w - 1)) * 4 + 3) as usize] as f32
    };
    let top = a(x0, y0) + (a(x0 + 1, y0) - a(x0, y0)) * fx;
    let bot = a(x0, y0 + 1) + (a(x0 + 1, y0 + 1) - a(x0, y0 + 1)) * fx;
    (top + (bot - top) * fy).round().clamp(0.0, 255.0) as u8
}

/// `rect` clamped to `clip`, as `(x0, y0, x1, y1)`.
fn inside(clip: &Canvas, rect: DirtyRect) -> (u32, u32, u32, u32) {
    let (w, h) = (clip.width, clip.height);
    (rect.min_x.min(w), rect.min_y.min(h), rect.max_x.min(w), rect.max_y.min(h))
}

/// Row `y`, columns `x0..x1`, of `clip` with its alpha cut to `base`'s, into
/// `out` — exactly that many pixels.
fn mask_row(
    out: &mut [u8],
    clip: &Canvas,
    base: Option<&Canvas>,
    map: Option<&BaseMap>,
    y: u32,
    x0: u32,
    x1: u32,
) {
    let w = clip.width as usize;
    let span = (y as usize * w + x0 as usize) * 4..(y as usize * w + x1 as usize) * 4;
    let src = &clip.pixels[span.clone()];
    out.copy_from_slice(src);
    match (base, map) {
        (None, _) => out.chunks_exact_mut(4).for_each(|o| o[3] = 0),
        // Aligned: the base pixel under each clip pixel has the same index.
        (Some(b), None) => {
            for (o, k) in out.chunks_exact_mut(4).zip(b.pixels[span].chunks_exact(4)) {
                o[3] = mul(o[3], k[3]);
            }
        }
        (Some(b), Some(m)) => {
            let (yf, xf) = (y as f32, x0 as f32);
            let (mut u, mut v) = (m.u0 + m.uy * yf + m.ux * xf, m.v0 + m.vy * yf + m.vx * xf);
            for o in out.chunks_exact_mut(4) {
                if o[3] != 0 {
                    o[3] = mul(o[3], alpha_at(b, u, v));
                }
                u += m.ux;
                v += m.vx;
            }
        }
    }
}

/// Write `clip`'s pixels, their alpha cut down to `base`'s, into `out` — a
/// buffer the size of `clip` — over `rect` of the clip cell. Pixels outside
/// `rect` are left alone. With no base drawing on the frame, everything is
/// cut away.
pub fn mask_into(
    out: &mut [u8],
    clip: &Canvas,
    base: Option<&Canvas>,
    at: &Placement,
    rect: DirtyRect,
) {
    let map = base.filter(|b| !at.aligned(clip, b)).map(|b| BaseMap::new(clip, b, at));
    let (x0, y0, x1, y1) = inside(clip, rect);
    let w = clip.width as usize;
    for y in y0..y1 {
        let row = &mut out[(y as usize * w + x0 as usize) * 4..(y as usize * w + x1 as usize) * 4];
        mask_row(row, clip, base, map.as_ref(), y, x0, x1);
    }
}

/// [`mask_into`] for just `rect`, packed into a buffer of its own — what a
/// partial texture upload takes.
pub fn mask_rect(clip: &Canvas, base: Option<&Canvas>, at: &Placement, rect: DirtyRect) -> Vec<u8> {
    let map = base.filter(|b| !at.aligned(clip, b)).map(|b| BaseMap::new(clip, b, at));
    let (x0, y0, x1, y1) = inside(clip, rect);
    let stride = ((x1 - x0) * 4) as usize;
    let mut out = vec![0u8; stride * (y1 - y0) as usize];
    if stride > 0 {
        for (y, row) in (y0..y1).zip(out.chunks_exact_mut(stride)) {
            mask_row(row, clip, base, map.as_ref(), y, x0, x1);
        }
    }
    out
}

/// `clip` masked by `base` across the whole cell, as a new canvas.
pub fn masked(clip: &Canvas, base: Option<&Canvas>, at: &Placement) -> Canvas {
    let mut out = clip.clone();
    let all = DirtyRect {
        min_x: 0,
        min_y: 0,
        max_x: clip.width,
        max_y: clip.height,
    };
    mask_into(&mut out.pixels, clip, base, at, all);
    out.dirty = None;
    out
}

/// The drawing layer `li` shows on `frame`, clipped if it is clipped: its
/// base's alpha applied, or `None` when a clipped layer's base is hidden, a
/// reference, or has nothing on the frame — the layer shows nothing then.
/// An unclipped layer's own cell comes back borrowed.
pub fn shown(p: &Project, li: usize, frame: usize) -> Option<std::borrow::Cow<'_, Canvas>> {
    let layer = p.layers.get(li)?;
    let cell = p.cell(layer.resolve(frame)?)?;
    let Some(b) = p.clip_base(li) else {
        return Some(std::borrow::Cow::Borrowed(cell));
    };
    let base = &p.layers[b];
    if !base.visible || base.reference {
        return None;
    }
    let base_cell = p.cell(base.resolve(frame)?)?;
    Some(std::borrow::Cow::Owned(masked(cell, Some(base_cell), &Placement::of(p, li, b, frame))))
}

/// Bake every clipped layer's mask into its pixels and unclip it — for a copy
/// of the project headed somewhere that has no clipping of its own (Krita).
///
/// A clipped drawing held while its base changes underneath shows differently
/// on each base drawing, so it becomes one baked drawing per pairing, keyed
/// where the pairing changes. Returns, for each baked cell, the clipped cell
/// it was made from.
pub fn bake_clipping(p: &mut Project) -> HashMap<CellId, CellId> {
    let mut origin = HashMap::new();
    // Every base first: unclipping a layer as it bakes would make it a base
    // for the clipped layers above it.
    let bases: Vec<Option<usize>> = (0..p.layers.len()).map(|li| p.clip_base(li)).collect();
    let fc = p.frame_count;
    for (li, base) in bases.into_iter().enumerate() {
        let Some(b) = base else {
            continue;
        };
        let mut made: HashMap<(CellId, Option<CellId>), CellId> = HashMap::new();
        let mut exposures = vec![None; p.layers[li].exposures.len().max(fc)];
        let mut last = None;
        for (f, slot) in exposures.iter_mut().enumerate().take(fc) {
            let Some(c) = p.layers[li].resolve(f) else {
                continue;
            };
            let pair = (c, p.layers[b].resolve(f));
            if last == Some(pair) {
                continue;
            }
            last = Some(pair);
            let id = match made.get(&pair) {
                Some(&id) => id,
                None => {
                    let Some(cell) = p.cell(c) else {
                        continue;
                    };
                    let base_cell = pair.1.and_then(|id| p.cell(id));
                    let baked = masked(cell, base_cell, &Placement::of(p, li, b, f));
                    let id = p.cells.len();
                    p.cells.push(baked);
                    made.insert(pair, id);
                    origin.insert(id, c);
                    id
                }
            };
            *slot = Some(id);
        }
        let layer = &mut p.layers[li];
        layer.exposures = exposures;
        layer.clip = false;
    }
    origin
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An 8×8 cell painted `rgba` inside `x0..x1`, transparent elsewhere.
    fn band(x0: u32, x1: u32, rgba: [u8; 4]) -> Canvas {
        let mut c = Canvas::new(8, 8);
        for y in 0..8 {
            for x in x0..x1 {
                let i = ((y * 8 + x) * 4) as usize;
                c.pixels[i..i + 4].copy_from_slice(&rgba);
            }
        }
        c
    }

    fn alpha(c: &Canvas, x: u32, y: u32) -> u8 {
        c.pixels[((y * c.width + x) * 4 + 3) as usize]
    }

    fn still(pw: u32, ph: u32) -> Placement {
        Placement {
            xf: Transform::default(),
            base_xf: Transform::default(),
            pw,
            ph,
        }
    }

    #[test]
    fn clipped_layers_share_the_first_unclipped_layer_below() {
        let mut p = Project::new(8, 8, 12.0);
        for _ in 0..4 {
            p.add_layer();
        }
        for li in [0, 1, 2, 4] {
            p.layers[li].clip = true;
        }
        let bases: Vec<Option<usize>> = (0..5).map(|li| p.clip_base(li)).collect();
        // A clipped bottom layer has nothing to clip to.
        assert_eq!(bases, [None, None, None, None, Some(3)]);
        p.layers[0].clip = false;
        let bases: Vec<Option<usize>> = (0..5).map(|li| p.clip_base(li)).collect();
        assert_eq!(bases, [None, Some(0), Some(0), None, Some(3)]);
    }

    #[test]
    fn the_mask_is_the_bases_alpha() {
        let clip = band(0, 8, [200, 10, 10, 255]);
        let base = band(2, 5, [0, 0, 0, 128]);
        let out = masked(&clip, Some(&base), &still(8, 8));
        assert_eq!(alpha(&out, 0, 3), 0);
        assert_eq!(alpha(&out, 3, 3), 128);
        assert_eq!(&out.pixels[..3], &clip.pixels[..3], "colour kept");
        // Nothing under it: nothing shows.
        assert!(masked(&clip, None, &still(8, 8)).pixels.chunks(4).all(|p| p[3] == 0));
    }

    #[test]
    fn a_moved_base_moves_the_mask() {
        let clip = band(0, 8, [200, 10, 10, 255]);
        let base = band(2, 3, [0, 0, 0, 255]);
        let mut at = still(8, 8);
        at.base_xf.tx = 3.0;
        let out = masked(&clip, Some(&base), &at);
        assert_eq!(alpha(&out, 2, 4), 0);
        assert_eq!(alpha(&out, 5, 4), 255, "base column 2 now sits over clip column 5");
    }

    #[test]
    fn only_the_rect_is_touched() {
        let clip = band(0, 8, [200, 10, 10, 255]);
        let base = band(0, 8, [0, 0, 0, 0]);
        let mut out = vec![7u8; 8 * 8 * 4];
        let r = DirtyRect {
            min_x: 1,
            min_y: 1,
            max_x: 3,
            max_y: 2,
        };
        mask_into(&mut out, &clip, Some(&base), &still(8, 8), r);
        assert_eq!(out[((8 + 1) * 4 + 3) as usize], 0);
        assert_eq!(out[3], 7, "outside the rect");
    }

    #[test]
    fn a_packed_rect_matches_the_same_rect_in_place() {
        let clip = band(0, 8, [200, 10, 10, 255]);
        let base = band(2, 6, [0, 0, 0, 200]);
        let at = still(8, 8);
        let r = DirtyRect {
            min_x: 1,
            min_y: 2,
            max_x: 5,
            max_y: 4,
        };
        let packed = mask_rect(&clip, Some(&base), &at, r);
        let full = masked(&clip, Some(&base), &at);
        for (row, y) in (2..4).enumerate() {
            let a = ((y * 8 + 1) * 4) as usize;
            assert_eq!(&packed[row * 16..row * 16 + 16], &full.pixels[a..a + 16]);
        }
    }

    #[test]
    fn a_hidden_base_hides_what_is_clipped_to_it() {
        let mut p = Project::new(8, 8, 12.0);
        p.add_layer();
        p.cells.push(band(0, 8, [0, 0, 0, 255]));
        p.cells.push(band(0, 8, [200, 10, 10, 255]));
        let n = p.cells.len();
        p.layers[0].set_key(0, n - 2);
        p.layers[1].set_key(0, n - 1);
        p.layers[1].clip = true;
        assert!(shown(&p, 1, 0).is_some());
        p.layers[0].visible = false;
        assert!(shown(&p, 1, 0).is_none());
    }

    #[test]
    fn baking_splits_a_held_drawing_where_its_base_changes() {
        let mut p = Project::new(8, 8, 12.0);
        while p.frame_count < 4 {
            p.add_frame();
        }
        p.add_layer();
        let c0 = p.cells.len();
        p.cells.push(band(0, 4, [0, 0, 0, 255]));
        p.cells.push(band(4, 8, [0, 0, 0, 255]));
        p.cells.push(band(0, 8, [200, 10, 10, 255]));
        let clip_cell = c0 + 2;
        p.layers[0].set_key(0, c0);
        p.layers[0].set_key(2, c0 + 1);
        p.layers[1].set_key(0, clip_cell);
        p.layers[1].clip = true;

        let origin = bake_clipping(&mut p);
        let l = &p.layers[1];
        assert!(!l.clip);
        let keys: Vec<usize> = (0..4).filter(|&f| l.is_key(f)).collect();
        assert_eq!(keys, [0, 2]);
        let (a, b) = (l.resolve(0).unwrap(), l.resolve(2).unwrap());
        assert_ne!(a, b);
        assert_eq!((origin[&a], origin[&b]), (clip_cell, clip_cell));
        assert_eq!(alpha(&p.cells[a], 1, 1), 255);
        assert_eq!(alpha(&p.cells[a], 6, 1), 0);
        assert_eq!(alpha(&p.cells[b], 6, 1), 255);
        // The base itself is untouched.
        assert_eq!(p.layers[0].resolve(3), Some(c0 + 1));
    }

    /// Cost at a 1080p frame, for the record: `cargo test --release
    /// clip_timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn clip_timing() {
        use std::time::Instant;
        let (w, h) = (1920u32, 1080u32);
        let mut clip = Canvas::new(w, h);
        let mut base = Canvas::new(w, h);
        for (i, px) in clip.pixels.chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(&[200, 10, 10, 255]);
            if (i as u32 % w) < w / 2 {
                base.pixels[i * 4 + 3] = 255;
            }
        }
        let whole = DirtyRect {
            min_x: 0,
            min_y: 0,
            max_x: w,
            max_y: h,
        };
        let at = still(w, h);
        let t = Instant::now();
        let buf = mask_rect(&clip, Some(&base), &at, whole);
        println!("whole cell, aligned: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        let mut moved = at;
        moved.base_xf.tx = 12.0;
        let t = Instant::now();
        let buf2 = mask_rect(&clip, Some(&base), &moved, whole);
        println!("whole cell, base moved: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        let stroke = DirtyRect {
            min_x: 500,
            min_y: 500,
            max_x: 564,
            max_y: 564,
        };
        let t = Instant::now();
        let buf3 = mask_rect(&clip, Some(&base), &at, stroke);
        println!("64 px stroke rect: {:.3} ms", t.elapsed().as_secs_f64() * 1e3);
        assert!(!buf.is_empty() && !buf2.is_empty() && !buf3.is_empty());
    }
}
