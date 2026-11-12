//! Drawing substitution: a key shows another of its layer's drawings, shared
//! rather than copied, so a blink or a mouth shape drawn once is reused and
//! a fix to it shows everywhere it does. The timeline's Drawing picker and
//! "Make independent" are the commands; this is them, the numbers the
//! drawings go by, and the picker's thumbnails.
//!
//! A child of `app` for the same reason as `select`: it works on
//! `AppState`'s private state directly.

use std::collections::HashMap;
use std::time::Instant;

use egui::{ColorImage, TextureHandle, TextureOptions};

use super::AppState;
use crate::doc::canvas::Canvas;
use crate::doc::layer::CellId;

/// A thumbnail's longer side, in pixels: sharp at twice the size it is
/// shown at.
const THUMB_PX: usize = 112;

/// Thumbnails made in one frame. A big drawing takes a few milliseconds;
/// the rest come on the frames after.
const THUMBS_PER_FRAME: u32 = 3;

/// The picker's thumbnails, by drawing, with the pixels they were made from.
#[derive(Default)]
pub struct Thumbs {
    made: HashMap<CellId, (u64, TextureHandle)>,
    left: u32,
}

impl AppState {
    /// Layer `layer`'s drawings in the order they are numbered: by when they
    /// were made, blanks left out. A drawing's number is its place here,
    /// from 1.
    pub fn numbered_drawings(&self, layer: usize) -> Vec<CellId> {
        self.project
            .layer_cell_ids(layer)
            .into_iter()
            .filter(|id| self.blank_cache.get(id) != Some(&true))
            .collect()
    }

    /// Drawing `id`'s number on layer `layer`, or `None` for a blank.
    pub fn drawing_number(&self, layer: usize, id: CellId) -> Option<usize> {
        self.numbered_drawings(layer).iter().position(|&d| d == id).map(|i| i + 1)
    }

    /// Whether some drawing on layer `layer` shows on more than one key —
    /// when its bars are worth numbering.
    pub fn layer_reuses(&self, layer: usize) -> bool {
        let Some(l) = self.project.layers.get(layer) else {
            return false;
        };
        let mut seen = std::collections::HashSet::new();
        l.exposures.iter().flatten().any(|id| !seen.insert(*id))
    }

    /// The keys a substitution on the active layer goes to, and whether it is
    /// the playhead's frame rather than the picked keys. The playhead when
    /// nothing is picked on the layer, or when one key is and the playhead
    /// is partway through its hold — where a right-click on a held frame
    /// leaves them — so a mouth or a blink starts on the frame clicked. Else
    /// the picked keys.
    fn substitution_keys(&self) -> (Vec<usize>, bool) {
        let li = self.project.current_layer;
        let at = self.project.current_frame;
        let picked: Vec<usize> = self.track_sel.iter().filter(|s| s.0 == li).map(|s| s.1).collect();
        let held_within = match picked.as_slice() {
            [k] => self.project.block_at(li, at).is_some_and(|b| b.start == *k && at > *k),
            _ => false,
        };
        if picked.is_empty() || held_within {
            (vec![at], true)
        } else {
            (picked, false)
        }
    }

    /// The drawings a substitution would replace: what those keys show now.
    pub fn substitution_shows(&self) -> Vec<CellId> {
        let l = &self.project.layers[self.project.current_layer];
        self.substitution_keys().0.into_iter().filter_map(|f| l.resolve(f)).collect()
    }

    /// Show drawing `id` on the active layer's picked keys, or from the
    /// playhead (see `substitution_keys`), sharing it. One undo step. A
    /// drawing that no longer shows anywhere is said so: undo brings it back.
    pub fn substitute_drawing(&mut self, id: CellId) {
        let li = self.project.current_layer;
        if !self.project.paintable(li) {
            return;
        }
        let (keys, at_playhead) = self.substitution_keys();
        let l = &self.project.layers[li];
        if keys.iter().all(|&f| l.exposures.get(f).copied().flatten() == Some(id)) {
            return;
        }
        let before = self.numbered_drawings(li);
        let picked = self.track_sel.clone();
        self.structural_edit(false, |p| {
            for &f in &keys {
                p.set_drawing(li, f, id);
            }
        });
        // The picked keys stay picked; a key made at the playhead is picked
        // instead, ready for the next pick.
        self.track_sel = if at_playhead { keys.iter().map(|&f| (li, f)).collect() } else { picked };
        let now = self.project.layer_cell_ids(li);
        let gone: Vec<usize> = before
            .iter()
            .enumerate()
            .filter(|(_, d)| !now.contains(d))
            .map(|(i, _)| i + 1)
            .collect();
        if !gone.is_empty() {
            let which = match gone.as_slice() {
                [n] => format!("Drawing {n} no longer shows anywhere"),
                _ => format!(
                    "Drawings {} no longer show anywhere",
                    gone.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")
                ),
            };
            self.save_toast = Some((format!("{which}. Undo brings it back."), Instant::now() + Self::TOAST_TTL * 2));
        }
    }

    /// Whether any drawing picked in the timeline shares its drawing with
    /// another key — what "Make independent" would change.
    pub fn picked_drawings_shared(&self) -> bool {
        self.track_sel.iter().any(|&(l, f)| {
            self.project.layers[l]
                .exposures
                .get(f)
                .copied()
                .flatten()
                .is_some_and(|id| self.project.keys_showing(l, id) > 1)
        })
    }

    /// Give each drawing picked in the timeline a drawing of its own where it
    /// shares one, so drawing on it changes it alone. One undo step.
    pub fn unlink_track_selection(&mut self) {
        let starts: Vec<(usize, usize)> =
            self.track_selection().into_iter().filter(|&(l, _)| self.project.paintable(l)).collect();
        if !self.picked_drawings_shared() || starts.is_empty() {
            return;
        }
        self.track_edit(|p| p.unlink_keys(&starts));
    }

    /// Drawing `id`'s thumbnail, made now when the frame's allowance has
    /// some left, else on a later frame (`None` until then).
    pub fn drawing_thumb(&mut self, ctx: &egui::Context, id: CellId) -> Option<egui::load::SizedTexture> {
        let cell = self.project.cell(id)?;
        let rev = cell.rev();
        if let Some((made, tex)) = self.drawing_thumbs.made.get(&id) {
            if *made == rev {
                return Some(egui::load::SizedTexture::from_handle(tex));
            }
        }
        if self.drawing_thumbs.left == 0 {
            ctx.request_repaint();
            return None;
        }
        self.drawing_thumbs.left -= 1;
        let img = thumb_image(cell, self.project.width, self.project.height);
        let tex = match self.drawing_thumbs.made.remove(&id) {
            // In place: the old one may be on screen this frame.
            Some((_, mut tex)) => {
                tex.set(img, TextureOptions::LINEAR);
                tex
            }
            None => ctx.load_texture(format!("drawing_thumb_{id}"), img, TextureOptions::LINEAR),
        };
        let sized = egui::load::SizedTexture::from_handle(&tex);
        self.drawing_thumbs.made.insert(id, (rev, tex));
        Some(sized)
    }

    /// A new frame's allowance of thumbnails.
    pub(super) fn refill_thumbs(&mut self) {
        self.drawing_thumbs.left = THUMBS_PER_FRAME;
    }

    /// Drop every thumbnail, parked for a frame — a new project's drawings
    /// reuse the old ones' ids. See `retired_textures`.
    pub(super) fn retire_drawing_thumbs(&mut self) {
        self.retired_textures.extend(self.drawing_thumbs.made.drain().map(|(_, (_, t))| t));
    }
}

/// A drawing's thumbnail: the part of it the canvas frame shows when the
/// layer hasn't moved (a layer's drawings can be bigger than the frame, and
/// sit centred on it), boxed down to fit [`THUMB_PX`]. Averaged by coverage,
/// so a soft edge keeps its colour rather than darkening.
fn thumb_image(c: &Canvas, frame_w: u32, frame_h: u32) -> ColorImage {
    let (w, h) = (frame_w.min(c.width).max(1), frame_h.min(c.height).max(1));
    let (x0, y0) = ((c.width - w) / 2, (c.height - h) / 2);
    // Down to fit, never up: a box filter spreads pixels, it can't fill gaps.
    let scale = (THUMB_PX as f32 / w.max(h) as f32).min(1.0);
    let tw = ((w as f32 * scale).round() as usize).clamp(1, THUMB_PX);
    let th = ((h as f32 * scale).round() as usize).clamp(1, THUMB_PX);
    let bin_x: Vec<usize> = (0..w as usize).map(|x| x * tw / w as usize).collect();
    let bin_y: Vec<usize> = (0..h as usize).map(|y| y * th / h as usize).collect();
    let mut acc = vec![[0f32; 4]; tw * th];
    if let Some((r, px)) = c.held() {
        let rw = (r.max_x - r.min_x) as usize;
        for y in r.min_y.max(y0)..r.max_y.min(y0 + h) {
            let row = &px[(y - r.min_y) as usize * rw * 4..][..rw * 4];
            let by = bin_y[(y - y0) as usize] * tw;
            for x in r.min_x.max(x0)..r.max_x.min(x0 + w) {
                let p = &row[(x - r.min_x) as usize * 4..][..4];
                if p[3] == 0 {
                    continue;
                }
                let a = p[3] as f32;
                let s = &mut acc[by + bin_x[(x - x0) as usize]];
                s[0] += p[0] as f32 * a;
                s[1] += p[1] as f32 * a;
                s[2] += p[2] as f32 * a;
                s[3] += a;
            }
        }
    }
    // How many of the frame's pixels each box holds.
    let mut per_x = vec![0f32; tw];
    bin_x.iter().for_each(|&b| per_x[b] += 1.0);
    let mut per_y = vec![0f32; th];
    bin_y.iter().for_each(|&b| per_y[b] += 1.0);
    let pixels: Vec<egui::Color32> = acc
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if s[3] <= 0.0 {
                return egui::Color32::TRANSPARENT;
            }
            let n = per_x[i % tw] * per_y[i / tw];
            let ch = |k: usize| (s[k] / s[3]).round().clamp(0.0, 255.0) as u8;
            let a = (s[3] / n).round().clamp(0.0, 255.0) as u8;
            egui::Color32::from_rgba_unmultiplied(ch(0), ch(1), ch(2), a)
        })
        .collect();
    ColorImage { size: [tw, th], pixels }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::project::Project;
    use std::sync::Arc;

    /// One layer over 8 frames, drawings keyed on 0, 2 and 4, each with a
    /// dot of its own colour.
    fn keyed() -> AppState {
        let mut p = Project::new(16, 16, 24.0);
        p.ensure_frame_count(8);
        p.layers[0].exposures.iter_mut().for_each(|e| *e = None);
        for (k, f) in [0usize, 2, 4].into_iter().enumerate() {
            let id = p.alloc_cell_for(0);
            let mut full = vec![0u8; 16 * 16 * 4];
            full[(5 * 16 + 5) * 4..][..4].copy_from_slice(&[40 * k as u8, 0, 0, 255]);
            p.cells[id] = Arc::new(Canvas::packed_from(16, 16, &full));
            p.layers[0].set_key(f, id);
        }
        let mut st = AppState::for_test();
        st.load_project(p, None);
        st
    }

    fn shows(st: &AppState, f: usize) -> CellId {
        st.project.layers[0].resolve(f).unwrap()
    }

    #[test]
    fn drawings_are_numbered_by_when_they_were_made() {
        let st = keyed();
        let ids = st.numbered_drawings(0);
        assert_eq!(ids.len(), 3);
        assert_eq!(st.drawing_number(0, shows(&st, 4)), Some(3));
        assert!(!st.layer_reuses(0));
    }

    #[test]
    fn picking_a_drawing_shares_it_on_the_picked_keys_in_one_step() {
        let mut st = keyed();
        let a = shows(&st, 0);
        st.track_sel = [(0, 2), (0, 4)].into_iter().collect();
        let steps = st.history.undo_len();
        st.substitute_drawing(a);
        assert_eq!((shows(&st, 2), shows(&st, 4)), (a, a));
        assert_eq!(st.history.undo_len(), steps + 1);
        assert_eq!(st.track_sel, [(0, 2), (0, 4)].into_iter().collect(), "still picked");
        assert!(st.layer_reuses(0));
        assert!(st.save_toast.as_ref().is_some_and(|(t, _)| t.starts_with("Drawings 2, 3 no longer")));
        st.undo();
        assert_ne!(shows(&st, 2), a);
        // Picking what they already show records nothing.
        let steps = st.history.undo_len();
        st.track_sel = [(0, 0)].into_iter().collect();
        st.substitute_drawing(a);
        assert_eq!(st.history.undo_len(), steps);
    }

    #[test]
    fn with_nothing_picked_it_keys_the_playhead() {
        let mut st = keyed();
        let a = shows(&st, 0);
        st.project.current_frame = 5;
        st.substitute_drawing(a);
        assert!(st.project.layers[0].is_key(5));
        assert_eq!(shows(&st, 7), a, "from the playhead to the next key");
        assert_eq!(shows(&st, 4), st.project.layers[0].exposures[4].unwrap(), "before it, as it was");
        assert!(st.track_sel.contains(&(0, 5)));
    }

    #[test]
    fn partway_through_the_picked_key_s_hold_it_keys_from_the_playhead() {
        // A right-click on frame 3 picks the drawing keyed on 2 and puts the
        // playhead on 3.
        let mut st = keyed();
        let (a, b) = (shows(&st, 0), shows(&st, 2));
        st.track_sel = [(0, 2)].into_iter().collect();
        st.project.current_frame = 3;
        assert_eq!(st.substitution_shows(), vec![b], "what shows on the frame clicked");
        st.substitute_drawing(a);
        assert_eq!(shows(&st, 2), b, "the key it holds from is left alone");
        assert!(st.project.layers[0].is_key(3));
        assert_eq!(shows(&st, 3), a);
        assert_eq!(st.track_sel, [(0, 3)].into_iter().collect(), "the new key is picked, ready for the next");
        assert!(st.save_toast.is_none(), "nothing left the timeline");

        // On the key's own frame, the key itself takes it.
        st.track_sel = [(0, 3)].into_iter().collect();
        st.substitute_drawing(b);
        assert_eq!(shows(&st, 3), b);
        assert_eq!(key_count(&st), 4);
    }

    #[test]
    fn several_picked_keys_take_it_wherever_the_playhead_is() {
        let mut st = keyed();
        let a = shows(&st, 0);
        st.track_sel = [(0, 2), (0, 4)].into_iter().collect();
        st.project.current_frame = 3;
        st.substitute_drawing(a);
        assert_eq!((shows(&st, 2), shows(&st, 4)), (a, a));
        assert!(!st.project.layers[0].is_key(3));
    }

    fn key_count(st: &AppState) -> usize {
        (0..st.project.frame_count).filter(|&f| st.project.layers[0].is_key(f)).count()
    }

    #[test]
    fn a_locked_layer_takes_no_substitute() {
        let mut st = keyed();
        st.project.layers[0].locked = true;
        let a = shows(&st, 0);
        st.track_sel = [(0, 2)].into_iter().collect();
        st.substitute_drawing(a);
        assert_ne!(shows(&st, 2), a);
    }

    #[test]
    fn making_a_shared_drawing_independent_copies_it_for_the_picked_key() {
        let mut st = keyed();
        let a = shows(&st, 0);
        st.track_sel = [(0, 4)].into_iter().collect();
        st.substitute_drawing(a);
        assert!(st.picked_drawings_shared());
        st.unlink_track_selection();
        let own = shows(&st, 4);
        assert_ne!(own, a);
        assert!(!st.picked_drawings_shared());
        assert_eq!(st.track_sel, [(0, 4)].into_iter().collect());
        // Nothing shared: nothing to do, no step.
        let steps = st.history.undo_len();
        st.unlink_track_selection();
        assert_eq!(st.history.undo_len(), steps);
    }

    #[test]
    fn a_thumbnail_keeps_its_colour_and_fits_the_frame() {
        let (w, h) = (THUMB_PX * 2, THUMB_PX);
        let mut full = vec![0u8; w * h * 4];
        // A soft red dot at the middle of the drawing, half the size again.
        full[(h / 2 * w + w / 2) * 4..][..4].copy_from_slice(&[200, 10, 10, 60]);
        let img = thumb_image(&Canvas::from_pixels(w as u32, h as u32, full), w as u32, h as u32);
        assert_eq!(img.size, [THUMB_PX, THUMB_PX / 2]);
        let lit: Vec<_> = img.pixels.iter().filter(|p| p.a() > 0).collect();
        assert!(!lit.is_empty());
        let c = lit[0].to_srgba_unmultiplied();
        // Within what egui's premultiplied storage keeps at so low an alpha;
        // averaged with the bare pixels around it, it would be near black.
        let near = |a: u8, b: u8| a.abs_diff(b) <= 8;
        assert!(near(c[0], 200) && near(c[1], 10) && near(c[2], 10), "averaged by coverage, not darkened: {c:?}");

        // A drawing bigger than the frame: only its middle, the frame's size.
        let big = thumb_image(&Canvas::new(w as u32 * 2, w as u32 * 2), w as u32, h as u32);
        assert_eq!(big.size, [THUMB_PX, THUMB_PX / 2]);
        // A frame smaller than a thumbnail stays its own size.
        assert_eq!(thumb_image(&Canvas::new(40, 20), 40, 20).size, [40, 20]);
    }

    #[test]
    fn thumbnails_are_made_a_few_a_frame_and_kept_until_drawn_on() {
        let mut st = keyed();
        let ctx = egui::Context::default();
        let ids = st.numbered_drawings(0);
        // Room for two this frame: the third waits for the next.
        st.drawing_thumbs.left = 2;
        let made: Vec<bool> = ids.iter().map(|&id| st.drawing_thumb(&ctx, id).is_some()).collect();
        assert_eq!(made, [true, true, false]);
        st.refill_thumbs();
        assert!(ids.iter().all(|&id| st.drawing_thumb(&ctx, id).is_some()));
        // Kept: asked again, no allowance needed.
        st.drawing_thumbs.left = 0;
        assert!(st.drawing_thumb(&ctx, ids[0]).is_some());
        // Drawn on: made again when there is allowance.
        st.project.cell_mut(ids[0]).unwrap().pixels_mut()[0] = 9;
        assert!(st.drawing_thumb(&ctx, ids[0]).is_none());
        st.refill_thumbs();
        assert!(st.drawing_thumb(&ctx, ids[0]).is_some());
    }
}
