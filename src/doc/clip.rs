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

/// Clip pixel `(x, y)` with its alpha cut to the base's.
#[inline]
fn masked_px(clip: &Canvas, base: Option<&Canvas>, at: &Placement, aligned: bool, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * clip.width + x) * 4) as usize;
    let a = clip.pixels[i + 3];
    let k = match base {
        _ if a == 0 => 0,
        None => 0,
        Some(b) if aligned => b.pixels[i + 3],
        Some(b) => {
            let (cw, ch) = (clip.width as f32, clip.height as f32);
            let (pw, ph) = (at.pw as f32, at.ph as f32);
            let (dx, dy) = at.xf.cell_to_doc(x as f32 + 0.5, y as f32 + 0.5, cw, ch, pw, ph);
            let (bw, bh) = (b.width as f32, b.height as f32);
            let (u, v) = at.base_xf.doc_to_cell(dx, dy, bw, bh, pw, ph);
            let (su, sv) = (u - 0.5, v - 0.5);
            if su < -0.5 || sv < -0.5 || su > bw - 0.5 || sv > bh - 0.5 {
                0
            } else {
                crate::io::composite::sample_bilinear(b, su, sv)[3]
            }
        }
    };
    let c = &clip.pixels[i..i + 4];
    [c[0], c[1], c[2], ((a as u32 * k as u32 + 127) / 255) as u8]
}

/// `rect` clamped to `clip`, as `(x0, y0, x1, y1)`.
fn inside(clip: &Canvas, rect: DirtyRect) -> (u32, u32, u32, u32) {
    let (w, h) = (clip.width, clip.height);
    (rect.min_x.min(w), rect.min_y.min(h), rect.max_x.min(w), rect.max_y.min(h))
}

/// Write `clip`'s pixels, their alpha cut down to `base`'s, into `out` — a
/// buffer the size of `clip` — over `rect` of the clip cell. Pixels outside
/// `rect` are left alone. With no base drawing on the frame, everything is
/// cut away.
pub fn mask_into(out: &mut [u8], clip: &Canvas, base: Option<&Canvas>, at: &Placement, rect: DirtyRect) {
    let aligned = base.is_some_and(|b| at.aligned(clip, b));
    let (x0, y0, x1, y1) = inside(clip, rect);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * clip.width + x) * 4) as usize;
            out[i..i + 4].copy_from_slice(&masked_px(clip, base, at, aligned, x, y));
        }
    }
}

/// [`mask_into`] for just `rect`, packed into a buffer of its own — what a
/// partial texture upload takes.
pub fn mask_rect(clip: &Canvas, base: Option<&Canvas>, at: &Placement, rect: DirtyRect) -> Vec<u8> {
    let aligned = base.is_some_and(|b| at.aligned(clip, b));
    let (x0, y0, x1, y1) = inside(clip, rect);
    let mut out = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
    for y in y0..y1 {
        for x in x0..x1 {
            out.extend_from_slice(&masked_px(clip, base, at, aligned, x, y));
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
}
