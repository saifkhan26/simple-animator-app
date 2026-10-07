//! Groups on screen: a group drawn as one image, and the mask a layer
//! clipped to a group is cut by.
//!
//! A group at full opacity needs neither: its layers draw one by one, which
//! comes out identical. Below full opacity its layers are merged first, so
//! they don't show through one another, and the merged image is drawn at the
//! group's opacity. Both kinds of image are CPU canvases in group space
//! ([`crate::doc::clip::group_space`]), kept per group and per what they were
//! made from, so a looping playback builds each once. An edit to one of the
//! group's drawings redoes just its rect where the drawing lies pixel for
//! pixel on the image, and the whole image otherwise.
//!
//! A child of `app` for the same reason as `select`: it works on
//! `AppState`'s private state directly.

use egui::{TextureHandle, TextureOptions};

use super::{cell_image, rect_meet, AppState, ClipPair, Dirty};
use crate::doc::canvas::{Canvas, DirtyRect};
use crate::doc::clip;
use crate::doc::layer::CellId;

/// Which image of which group: its layer's uid, a hash of what the image is
/// made from ([`clip::group_sig`]), and whether it is the merged image
/// (drawn on screen) or the mask (read by clip textures).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GroupKey {
    pub uid: u64,
    pub sig: u64,
    pub merged: bool,
}

/// What a clipped layer is cut by: one drawing, or a whole group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BaseKey {
    Cell(CellId),
    Group(GroupKey),
}

/// A group drawn as one image.
pub struct GroupImage {
    pub canvas: Canvas,
    /// The cells it is drawn from, and whether each lies pixel for pixel on
    /// it — an edit to one of those redoes the same rect here.
    cells: Vec<(CellId, bool)>,
    /// What changed in those cells since it was drawn.
    pending: Dirty,
    /// The merged image on the GPU; masks stay on the CPU.
    pub tex: Option<TextureHandle>,
}

impl GroupImage {
    pub fn bytes(&self) -> usize {
        let tex = self.tex.as_ref().map_or(0, |t| t.size()[0] * t.size()[1] * 4);
        self.canvas.byte_len() + tex
    }
}

impl AppState {
    /// Group `g`'s image key on the current frame.
    pub(crate) fn group_key(&self, g: usize, merged: bool) -> GroupKey {
        let f = self.project.current_frame;
        let xf = |li: usize| self.display_transform(li, f);
        GroupKey {
            uid: self.project.layers[g].uid,
            sig: clip::group_sig(&self.project, g, f, merged, &xf),
            merged,
        }
    }

    /// Whether group `g` draws as one merged image: it shows, below full
    /// opacity. Except while "fade other layers" is fading around one of its
    /// own layers: then its layers draw one by one, so the active one can
    /// stand out — an approximation, fine for a view aid.
    pub(crate) fn group_drawn_whole(&self, g: usize) -> bool {
        let p = &self.project;
        let Some(l) = p.layers.get(g).filter(|l| l.is_group()) else {
            return false;
        };
        if !l.visible || l.opacity >= 1.0 {
            return false;
        }
        let fading_inside = self.fade_others && !self.playback.playing && p.group_of(p.current_layer) == Some(g);
        !fading_inside
    }

    /// Whether layer `li` shows only as part of its group's merged image.
    pub(crate) fn in_group_image(&self, li: usize) -> bool {
        self.project.group_of(li).is_some_and(|g| self.group_drawn_whole(g))
    }

    /// Whether what shows of layer `li`'s drawing comes from a group image:
    /// its group is drawn merged, or something is clipped to its group.
    /// Strokes on it can't take the plain texture's fast path.
    pub(crate) fn feeds_group_image(&self, li: usize) -> bool {
        let Some(g) = self.project.group_of(li) else {
            return false;
        };
        self.group_drawn_whole(g) || (0..self.project.layers.len()).any(|i| self.project.clip_base(i) == Some(g))
    }

    /// Group `g`'s merged texture on the current frame and its group-space
    /// size, if it has been made.
    pub(crate) fn group_texture(&self, g: usize) -> Option<(&TextureHandle, (u32, u32))> {
        let img = self.group_images.get(&self.group_key(g, true))?;
        Some((img.tex.as_ref()?, (img.canvas.width, img.canvas.height)))
    }

    /// Bring the group images this frame needs up to date: the merged ones
    /// for `merged` groups, and the masks the `clipped` pairs are cut by.
    pub(super) fn sync_group_images(&mut self, ctx: &egui::Context, now: u64, merged: &[usize], clipped: &[ClipPair]) {
        let mut want: Vec<(GroupKey, usize)> = merged.iter().map(|&g| (self.group_key(g, true), g)).collect();
        for pair in clipped {
            if let BaseKey::Group(k) = pair.base {
                if !want.iter().any(|(w, _)| *w == k) {
                    want.push((k, pair.base_layer));
                }
            }
        }
        let f = self.project.current_frame;
        let xfs: Vec<_> = (0..self.project.layers.len()).map(|li| self.display_transform(li, f)).collect();
        let xf = |li: usize| xfs[li];
        for (key, g) in want {
            self.group_used.insert(key, now);
            let p = &self.project;
            match self.group_images.get_mut(&key) {
                Some(img) => {
                    let rect = match img.pending {
                        Dirty::Clean => continue,
                        Dirty::Rect(r) => r,
                        Dirty::Full => full(&img.canvas),
                    };
                    clip::render_group(p, g, f, key.merged, &xf, &mut img.canvas, rect);
                    img.pending = Dirty::Clean;
                    if let (Some(tex), Some(r)) = (img.tex.as_mut(), rect_meet(rect, full(&img.canvas))) {
                        tex.set_partial([r.min_x as usize, r.min_y as usize], cell_image(&img.canvas, r), TextureOptions::LINEAR);
                    }
                }
                None => {
                    let (w, h) = clip::group_space(p, g, f, &xf);
                    let mut canvas = Canvas::new(w, h);
                    let all = full(&canvas);
                    clip::render_group(p, g, f, key.merged, &xf, &mut canvas, all);
                    let cells = clip::group_cells(p, g, f, &xf);
                    let tex = key
                        .merged
                        .then(|| ctx.load_texture(format!("group_{}", key.uid), cell_image(&canvas, all), TextureOptions::LINEAR));
                    self.group_images.insert(
                        key,
                        GroupImage {
                            canvas,
                            cells,
                            pending: Dirty::Clean,
                            tex,
                        },
                    );
                }
            }
        }
    }

    /// Note that `region` of cell `id` changed, for the group images drawn
    /// from it and the clip textures cut by those.
    pub(super) fn mark_group_region(&mut self, id: CellId, region: Dirty) {
        let mut hit: Vec<(GroupKey, Dirty)> = Vec::new();
        for (key, img) in self.group_images.iter_mut() {
            if let Some(&(_, aligned)) = img.cells.iter().find(|(c, _)| *c == id) {
                let r = match region {
                    Dirty::Clean => continue,
                    Dirty::Rect(r) if aligned => Dirty::Rect(r),
                    _ => Dirty::Full,
                };
                img.pending = img.pending.with(r);
                hit.push((*key, r));
            }
        }
        if hit.is_empty() {
            return;
        }
        for (&(_, base), t) in self.clip_textures.iter_mut() {
            let BaseKey::Group(k) = base else {
                continue;
            };
            let Some(&(_, r)) = hit.iter().find(|(h, _)| *h == k) else {
                continue;
            };
            let inside = match r {
                Dirty::Rect(r) if t.aligned => rect_meet(r, t.rect),
                _ => rect_meet(t.rect, t.rect),
            };
            t.pending = t.pending.with(inside.map_or(Dirty::Clean, Dirty::Rect));
        }
    }

    /// Drop group images that no longer belong to any group, or that cells
    /// went away from under.
    pub(super) fn retire_group_images(&mut self, gone_cell: Option<CellId>) {
        let uids: std::collections::HashSet<u64> = self.project.layers.iter().filter(|l| l.is_group()).map(|l| l.uid).collect();
        let stale: Vec<GroupKey> = self
            .group_images
            .iter()
            .filter(|(k, img)| !uids.contains(&k.uid) || gone_cell.is_some_and(|c| img.cells.iter().any(|(id, _)| *id == c)))
            .map(|(k, _)| *k)
            .collect();
        for k in stale {
            self.drop_group_image(k);
        }
    }

    pub(super) fn drop_group_image(&mut self, k: GroupKey) {
        if let Some(img) = self.group_images.remove(&k) {
            self.retired_textures.extend(img.tex);
        }
        self.group_used.remove(&k);
        let cut: Vec<_> = self.clip_textures.keys().copied().filter(|&(_, b)| b == BaseKey::Group(k)).collect();
        for key in cut {
            if let Some(t) = self.clip_textures.remove(&key) {
                self.retired_textures.push(t.tex);
            }
            self.clip_tex_used.remove(&key);
        }
    }
}

fn full(c: &Canvas) -> DirtyRect {
    DirtyRect {
        min_x: 0,
        min_y: 0,
        max_x: c.width,
        max_y: c.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Run one texture sync and hand back what egui was asked to upload.
    fn sync(st: &mut AppState, ctx: &egui::Context) -> egui::TexturesDelta {
        let raw = egui::RawInput {
            max_texture_side: Some(8192),
            ..Default::default()
        };
        ctx.run(raw, |ctx| st.sync_textures(ctx)).textures_delta
    }

    fn alpha(delta: &egui::TexturesDelta, id: egui::TextureId, x: usize, y: usize) -> Option<u8> {
        let (_, d) = delta.set.iter().find(|(t, _)| *t == id)?;
        match &d.image {
            egui::ImageData::Color(img) => Some(img.pixels[y * img.size[0] + x].a()),
            _ => None,
        }
    }

    /// Two layers on the left half of the frame, red over blue, in a group;
    /// a third layer, green all over, above it.
    fn grouped() -> (AppState, usize) {
        let mut st = AppState::for_test();
        let (w, h) = (st.project.width, st.project.height);
        st.project.add_layer();
        st.project.add_layer();
        let paint = |x1: u32, rgba: [u8; 4]| {
            let mut c = Canvas::new(w, h);
            let px = c.pixels_mut();
            for y in 0..h {
                for x in 0..x1 {
                    let i = ((y * w + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(&rgba);
                }
            }
            c
        };
        for (li, c) in [paint(w / 2, [0, 0, 255, 255]), paint(w / 2, [255, 0, 0, 255]), paint(w, [0, 255, 0, 255])]
            .into_iter()
            .enumerate()
        {
            let id = st.project.cells.len();
            st.project.cells.push(Arc::new(c));
            st.project.layers[li].set_key(0, id);
        }
        let g = st.project.group_layers(&[0, 1]).unwrap();
        (st, g)
    }

    impl AppState {
        fn ungroup_active_of(&mut self, g: usize) {
            self.project.current_layer = g;
            self.ungroup_active();
        }
    }

    #[test]
    fn a_half_opaque_group_draws_one_merged_texture() {
        let (mut st, g) = grouped();
        st.project.layers[g].opacity = 0.5;
        let ctx = egui::Context::default();
        let delta = sync(&mut st, &ctx);
        let (tex, (gw, _)) = st.group_texture(g).expect("a merged texture");
        assert_eq!(gw, st.project.width);
        let kids: Vec<_> = st.project.children(g).map(|c| st.project.layers[c].resolve(0).unwrap()).collect();
        assert!(kids.iter().all(|id| !st.cell_textures.contains_key(id)), "its layers aren't drawn alone");
        assert_eq!(alpha(&delta, tex.id(), 10, 10), Some(255), "merged at full strength; opacity is the quad's");
    }

    #[test]
    fn an_opaque_group_draws_its_layers_directly() {
        let (mut st, g) = grouped();
        let ctx = egui::Context::default();
        let _ = sync(&mut st, &ctx);
        assert!(st.group_images.is_empty());
        assert!(st.group_texture(g).is_none());
        for c in st.project.children(g) {
            assert!(st.cell_textures.contains_key(&st.project.layers[c].resolve(0).unwrap()));
        }
    }

    #[test]
    fn painting_a_layer_of_the_group_patches_just_that_rect() {
        let (mut st, g) = grouped();
        st.project.layers[g].opacity = 0.5;
        let ctx = egui::Context::default();
        let _ = sync(&mut st, &ctx);
        let tex = st.group_texture(g).unwrap().0.id();
        let id = st.project.layers[1].resolve(0).unwrap();
        let w = st.project.width;
        let c = st.project.cell_mut(id).unwrap();
        c.dirty = None;
        for y in 100..110u32 {
            for x in w - 50..w - 40 {
                let i = ((y * w + x) * 4) as usize;
                c.pixels_mut()[i..i + 4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
        c.mark_dirty(w - 50, 100, 10, 10);
        st.mark_dirty(id);
        let delta = sync(&mut st, &ctx);
        let (_, d) = delta.set.iter().find(|(t, _)| *t == tex).expect("group image redone");
        assert_eq!(d.pos, Some([(w - 50) as usize, 100]), "a partial upload");
        assert_eq!(d.image.size(), [10, 10]);
    }

    #[test]
    fn a_layer_above_a_group_is_cut_to_it_on_screen() {
        let (mut st, g) = grouped();
        let top = st.project.layers.len() - 1;
        st.project.layers[top].clip = true;
        let ctx = egui::Context::default();
        let delta = sync(&mut st, &ctx);
        let (id, tex, _) = st.layer_texture(top).expect("drawn cut");
        assert_eq!(id, st.project.layers[top].resolve(0).unwrap());
        let w = st.project.width as usize;
        assert_eq!(alpha(&delta, tex.id(), 10, 10), Some(255), "over the group");
        assert_eq!(alpha(&delta, tex.id(), w - 10, 10), Some(0), "past it");
        // Hiding the group hides what's clipped to it, and its layers.
        st.project.layers[g].visible = false;
        let _ = sync(&mut st, &ctx);
        assert!(st.layer_texture(top).is_none());
    }

    /// Merging a half-opaque group gives one layer that exports the same.
    #[test]
    fn merge_group_matches_the_group_at_its_opacity() {
        let (mut st, g) = grouped();
        st.project.layers[g].opacity = 0.5;
        let before = crate::io::composite::flatten_frame(&st.project, 0);
        let steps = st.history.undo_len();
        st.merge_group(g);
        assert_eq!(st.history.undo_len(), steps + 1);
        assert!(!st.project.layers.iter().any(|l| l.is_group() || l.in_group));
        assert_eq!(st.project.layers.len(), 2);
        assert_eq!(st.project.layers[0].opacity, 0.5);
        let after = crate::io::composite::flatten_frame(&st.project, 0);
        let worst = before
            .pixels()
            .iter()
            .zip(after.pixels().iter())
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .max()
            .unwrap();
        assert!(worst <= 1, "off by {worst}");
        st.undo();
        assert!(st.project.is_group(g), "one undo brings the group back");
    }

    #[test]
    fn merge_group_keeps_its_hidden_layers() {
        let (mut st, g) = grouped();
        st.project.layers[0].visible = false;
        st.merge_group(g);
        let rows: Vec<(bool, bool)> = st.project.layers.iter().map(|l| (l.visible, l.is_group())).collect();
        assert_eq!(rows, [(false, false), (true, false), (true, false)], "hidden kept, then merged, then top");
    }

    #[test]
    fn grouping_the_picked_layers_is_one_undo() {
        let (mut st, g) = grouped();
        st.ungroup_active_of(g);
        // [blue, red, green]: pick blue and green, active on green.
        st.project.current_layer = 2;
        st.selected_layers.insert(st.project.layers[0].uid);
        let steps = st.history.undo_len();
        st.group_selected_layers();
        assert_eq!(st.history.undo_len(), steps + 1);
        let rows: Vec<(bool, bool)> = st.project.layers.iter().map(|l| (l.is_group(), l.in_group)).collect();
        assert_eq!(rows, [(false, false), (false, true), (false, true), (true, false)]);
        st.undo();
        assert!(!st.project.layers.iter().any(|l| l.is_group()));
    }

    #[test]
    fn a_header_refuses_drawing_and_key_edits() {
        let (mut st, g) = grouped();
        st.project.current_layer = g;
        let steps = st.history.undo_len();
        st.set_tool(crate::tools::ActiveTool::Ink);
        st.pointer_down(st.make_sample(10.0, 10.0, 0.0));
        st.pointer_move(st.make_sample(40.0, 10.0, 0.1));
        st.pointer_up();
        st.dispatch(crate::input::shortcuts::Action::KeyBlank);
        st.dispatch(crate::input::shortcuts::Action::LayerClipToggle);
        assert_eq!(st.history.undo_len(), steps);
        assert!(st.project.layers[g].exposures.iter().all(Option::is_none));
        assert!(!st.project.layers[g].clip);
        // Nor does merging down cross into or out of a group.
        st.project.current_layer = g + 1;
        assert!(!st.can_merge_down());
    }

    #[test]
    fn the_layer_keys_walk_the_rows_the_panel_shows() {
        let (mut st, g) = grouped();
        let top = st.project.layers.len() - 1;
        st.project.layers[g].collapsed = true;
        st.project.current_layer = top;
        st.dispatch(crate::input::shortcuts::Action::LayerSelectBelow);
        assert_eq!(st.project.current_layer, g, "onto the folded group");
        st.dispatch(crate::input::shortcuts::Action::LayerSelectBelow);
        assert_eq!(st.project.current_layer, g, "its layers are out of sight");
        st.project.layers[g].collapsed = false;
        st.dispatch(crate::input::shortcuts::Action::LayerSelectBelow);
        assert_eq!(st.project.current_layer, g - 1, "unfolded, into it");
        st.dispatch(crate::input::shortcuts::Action::LayerPick1);
        assert_eq!(st.project.current_layer, top);
    }

    #[test]
    fn group_images_go_with_their_group() {
        let (mut st, g) = grouped();
        st.project.layers[g].opacity = 0.5;
        let ctx = egui::Context::default();
        let _ = sync(&mut st, &ctx);
        assert!(!st.group_images.is_empty());
        st.project.ungroup(g);
        let _ = sync(&mut st, &ctx);
        assert!(st.group_images.is_empty());
    }
}
