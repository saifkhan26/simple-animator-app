//! The persistent selection: the document-space mask, the gestures that edit
//! it, and the floating pixels a drag lifts through it.
//!
//! A child of `app` so it can work on `AppState`'s private state directly;
//! split out only because `app.rs` is long enough already.
//!
//! Two things live here and are easy to confuse:
//!
//! - **The mask** (`sel_mask`): which part of the canvas is selected. Every
//!   paint edit is clipped to it. It belongs to no drawing, so it survives
//!   frame and layer changes.
//! - **The float** (`selection`): pixels lifted out of one cell through the
//!   mask, moving under a pose. It exists only while pixels are being moved
//!   and lands back into its cell — taking the mask along to where it landed —
//!   before anything else edits.

use std::sync::Arc;

use super::{AppState, SelDrag, SelTint};
use crate::tools::lasso::{self, Mask};
use crate::tools::select_mask::{self, SelOp, SelShape, SelectionMask};
use crate::tools::selection::{self, Placement, Pose, Selection, HANDLE_PX};
use crate::tools::ActiveTool;
use crate::undo;

/// An in-progress selection gesture, in document space.
#[derive(Clone, Debug)]
pub enum SelGesture {
    /// Freehand, rectangle or ellipse: one press, drag, release.
    Drag {
        op: SelOp,
        shape: SelShape,
        /// Freehand samples. Rectangle and ellipse only use `start`/`end`.
        pts: Vec<(f32, f32)>,
        start: (f32, f32),
        end: (f32, f32),
        /// Furthest the pointer got from `start` — tells a click from a drag.
        max_dist: f32,
    },
    /// Polygon: a corner per click until it is closed. While `pressed`, the
    /// newest corner follows the pointer, so a click can be dragged into place.
    Polygon {
        op: SelOp,
        pts: Vec<(f32, f32)>,
        pressed: bool,
    },
}

/// What the cached cell mask was built for: the selection's version and where
/// the active cell sits.
#[derive(Clone, Copy, PartialEq)]
pub struct ClipKey {
    ver: u64,
    at: Placement,
}

/// A click that travels less than this many screen pixels is a click, not a
/// drag — in Replace mode it deselects.
const CLICK_PX: f32 = 3.0;

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The pose that moves another drawing — cut through mask `mk`, placed at `pk`
/// — the way `sel` moved on screen.
///
/// Usually the drawing sits exactly where the float's did, so its mask is the
/// same and the pose carries over as it is; a plain move then stays a lossless
/// whole-pixel blit. Where the layer is keyed to a different transform on that
/// frame, or the drawing's canvas is another size, the mask box's centre is
/// sent through the float's motion in document space and back, and the pose is
/// rebuilt around it. Rotation and uniform scale carry over exactly; a
/// non-uniform scale is only exact while the two frames share a layer rotation,
/// since the pose can only scale along the drawing's own axes.
fn pose_for(sel: &Selection, pk: &Placement, mk: &Mask) -> Pose {
    let p0 = &sel.placement;
    let m0 = &sel.mask;
    if pk == p0 && (mk.x, mk.y, mk.w, mk.h) == (m0.x, m0.y, m0.w, m0.h) {
        return sel.pose;
    }
    let ck = (mk.x as f32 + mk.w as f32 * 0.5, mk.y as f32 + mk.h as f32 * 0.5);
    let doc = pk.xf.cell_to_doc(ck.0, ck.1, pk.cw as f32, pk.ch as f32, pk.pw, pk.ph);
    let c0 = p0.xf.doc_to_cell(doc.0, doc.1, p0.cw as f32, p0.ch as f32, p0.pw, p0.ph);
    let moved0 = sel.path_point(c0.0, c0.1);
    let moved = p0.xf.cell_to_doc(moved0.0, moved0.1, p0.cw as f32, p0.ch as f32, p0.pw, p0.ph);
    let target = pk.xf.doc_to_cell(moved.0, moved.1, pk.cw as f32, pk.ch as f32, pk.pw, pk.ph);
    let mut offset = (target.0 - ck.0, target.1 - ck.1);
    // A whole-pixel move between drawings that differ only by where they sit
    // is still a whole-pixel move: keep it on the lossless path.
    if sel.pose.is_pixel_aligned() && pk.xf.rot == p0.xf.rot && pk.xf.scale == p0.xf.scale {
        offset = (offset.0.round(), offset.1.round());
    }
    Pose { offset, ..sel.pose }
}

impl AppState {
    // --- Where things are ---

    /// Where the active cell sits in the document right now: the layer's live
    /// transform and the size of the cell a stroke here would paint into.
    pub fn active_placement(&self) -> Placement {
        let li = self.project.current_layer;
        let f = self.project.current_frame;
        let (cw, ch) = self.project.draw_cell_size(li, f);
        Placement {
            xf: self.display_transform(li, f),
            cw,
            ch,
            pw: self.project.width as f32,
            ph: self.project.height as f32,
        }
    }

    /// A document point in the active cell's pixel space.
    pub fn doc_to_active_cell(&self, doc: (f32, f32)) -> (f32, f32) {
        let p = self.active_placement();
        p.xf
            .doc_to_cell(doc.0, doc.1, p.cw as f32, p.ch as f32, p.pw, p.ph)
    }

    // --- The mask ---

    /// Replace the selection. `record` pushes an undo step; undo and redo
    /// themselves pass `false`.
    pub(crate) fn set_mask(&mut self, m: Option<SelectionMask>, record: bool) {
        if m == self.sel_mask {
            return;
        }
        if record {
            self.history.push(undo::Command::Selection {
                before: self.sel_mask.as_ref().map(SelectionMask::pack),
                after: m.as_ref().map(SelectionMask::pack),
            });
        }
        self.sel_mask = m;
        self.mask_changed();
    }

    /// Invalidate everything derived from `sel_mask`.
    fn mask_changed(&mut self) {
        self.sel_ver = self.sel_ver.wrapping_add(1);
        self.sel_outline = self
            .sel_mask
            .as_ref()
            .map(SelectionMask::contours)
            .unwrap_or_default();
        self.clip_cache = None;
    }

    /// The selection as the active cell sees it, or `None` when nothing is
    /// selected. Select All comes back as the whole cell. Cached until the
    /// selection, the layer's placement or the cell size changes — a stroke
    /// asks at every pen-down.
    pub fn active_cell_mask(&mut self) -> Option<Arc<Mask>> {
        let m = self.sel_mask.as_ref()?;
        let key = ClipKey {
            ver: self.sel_ver,
            at: self.active_placement(),
        };
        if let Some((k, c)) = &self.clip_cache {
            if *k == key {
                return Some(c.clone());
            }
        }
        let p = key.at;
        let c = Arc::new(m.to_cell(&p.xf, p.cw, p.ch, p.pw, p.ph));
        self.clip_cache = Some((key, c.clone()));
        Some(c)
    }

    /// The clip a paint edit on the active cell is held to. `None` means paint
    /// freely: nothing is selected, or everything is.
    pub fn active_cell_clip(&mut self) -> Option<Arc<Mask>> {
        if self.sel_mask.as_ref()?.is_all() {
            return None;
        }
        self.active_cell_mask()
    }

    /// The cached cell mask, but only while it is current — for drawing, which
    /// has no `&mut` to build one with. [`AppState::sync_selection`] keeps it
    /// warm while the Lasso tool needs it.
    pub fn cached_cell_mask(&self) -> Option<&Mask> {
        let (k, m) = self.clip_cache.as_ref()?;
        let now = ClipKey {
            ver: self.sel_ver,
            at: self.active_placement(),
        };
        (*k == now).then_some(m.as_ref())
    }

    /// Whether a plain press on the selection lifts its pixels. Only for a
    /// bounded selection on an editable drawing: after Select All or Invert,
    /// every drag would turn into a move and there would be no way left to
    /// draw a new selection.
    pub fn mask_is_grabbable(&self) -> bool {
        self.sel_mask.as_ref().is_some_and(|m| m.outside == 0)
            && !self.active_layer_locked()
            && !self.active_layer_is_reference()
            && self.project.resolved_current().is_some()
    }

    /// Per-frame upkeep: land a float whose cell stopped being the active one,
    /// and keep the cell mask warm for the transform box `paint_canvas` draws.
    pub(super) fn sync_selection(&mut self) {
        self.land_float_if_orphaned();
        if self.tool == ActiveTool::Lasso && self.selection.is_none() && self.mask_is_grabbable() {
            let _ = self.active_cell_mask();
        }
    }

    /// The document box a new selection is clipped to: the frame plus every
    /// layer's cell at this frame. Nothing outside it can be painted, and it
    /// stops a lasso drawn while zoomed far out from asking for a gigapixel
    /// mask.
    fn selection_universe(&self) -> (i32, i32, i32, i32) {
        let (pw, ph) = (self.project.width as f32, self.project.height as f32);
        let (mut x0, mut y0, mut x1, mut y1) = (0.0f32, 0.0f32, pw, ph);
        let f = self.project.current_frame;
        for li in 0..self.project.layers.len() {
            let (cw, ch) = self.project.draw_cell_size(li, f);
            let (cw, ch) = (cw as f32, ch as f32);
            let t = self.display_transform(li, f);
            for (u, v) in [(0.0, 0.0), (cw, 0.0), (cw, ch), (0.0, ch)] {
                let (x, y) = t.cell_to_doc(u, v, cw, ch, pw, ph);
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
        (
            x0.floor() as i32 - 2,
            y0.floor() as i32 - 2,
            x1.ceil() as i32 + 2,
            y1.ceil() as i32 + 2,
        )
    }

    /// Combine a freshly drawn shape into the selection, as one undo step.
    /// `None` is a shape that covered nothing.
    pub fn apply_op(&mut self, op: SelOp, shape: Option<SelectionMask>) {
        self.land_float();
        let next = match (op, shape) {
            (SelOp::Replace, m) => m,
            (SelOp::Intersect, None) => None,
            (_, None) => self.sel_mask.clone(),
            (op, Some(b)) => SelectionMask::combine(self.sel_mask.as_ref(), &b, op),
        };
        self.set_mask(next, true);
    }

    // --- Gestures ---

    /// Selection press at document point `doc`. `held` is the mode a held
    /// modifier asks for, which beats the sticky one for this drag.
    pub fn select_down(&mut self, doc: (f32, f32), held: Option<SelOp>) {
        self.playback.stop();
        if self.polygon_active() {
            // A press back on the first corner closes the shape; anywhere
            // else adds a corner.
            let close = HANDLE_PX * 1.5 / self.view_scale.max(1e-6);
            let closes = matches!(
                &self.sel_gesture,
                Some(SelGesture::Polygon { pts, .. }) if pts.len() >= 3 && dist(pts[0], doc) <= close
            );
            if closes {
                self.polygon_finish();
            } else if let Some(SelGesture::Polygon { pts, pressed, .. }) = &mut self.sel_gesture {
                pts.push(doc);
                *pressed = true;
            }
            return;
        }
        let op = held.unwrap_or(self.sel_op);
        // A plain press on the selection moves it. A held modifier always
        // starts a new shape, even inside — that is how you add to a
        // selection from within it.
        if held.is_none() && op == SelOp::Replace && self.try_grab(doc) {
            return;
        }
        self.land_float();
        self.sel_gesture = Some(match self.sel_shape {
            SelShape::Polygon => SelGesture::Polygon {
                op,
                pts: vec![doc],
                pressed: true,
            },
            shape => SelGesture::Drag {
                op,
                shape,
                pts: vec![doc],
                start: doc,
                end: doc,
                max_dist: 0.0,
            },
        });
    }

    /// Selection drag to document point `doc`.
    pub fn select_move(&mut self, doc: (f32, f32)) {
        if let Some(drag) = self.sel_drag {
            let (cx, cy) = self.doc_to_active_cell(doc);
            self.drag_selection(drag, cx, cy);
            return;
        }
        // Finer than a document pixel when zoomed in, so a close-up lasso
        // follows the hand; one pixel when zoomed out, where the pen emits far
        // more samples than the polygon needs.
        let step = (1.0 / self.view_scale.max(1e-6)).min(1.0);
        match &mut self.sel_gesture {
            Some(SelGesture::Drag {
                shape,
                pts,
                start,
                end,
                max_dist,
                ..
            }) => {
                *end = doc;
                *max_dist = max_dist.max(dist(*start, doc));
                if *shape == SelShape::Freehand
                    && pts.last().map_or(true, |&p| dist(p, doc) >= step)
                {
                    pts.push(doc);
                }
            }
            Some(SelGesture::Polygon {
                pts, pressed: true, ..
            }) => {
                if let Some(p) = pts.last_mut() {
                    *p = doc;
                }
            }
            _ => {}
        }
    }

    /// Selection release.
    pub fn select_up(&mut self) {
        if self.sel_drag.take().is_some() {
            // A press on the selection that never moved lifted nothing: put
            // the candidate float away and leave everything as it was.
            if self.selection.as_ref().is_some_and(|s| !s.lifted) {
                self.selection = None;
                self.retire_float_tex();
            }
            return;
        }
        match self.sel_gesture.take() {
            Some(SelGesture::Drag {
                op,
                shape,
                pts,
                start,
                end,
                max_dist,
            }) => {
                if max_dist * self.view_scale < CLICK_PX {
                    if op == SelOp::Replace {
                        self.deselect();
                    }
                    return;
                }
                let path = match shape {
                    SelShape::Rect => select_mask::rect_path(start, end),
                    SelShape::Ellipse => select_mask::ellipse_path(start, end),
                    SelShape::Freehand | SelShape::Polygon => pts,
                };
                let m = SelectionMask::from_path(&path, self.selection_universe());
                self.apply_op(op, m);
            }
            Some(SelGesture::Polygon { op, pts, .. }) => {
                self.sel_gesture = Some(SelGesture::Polygon {
                    op,
                    pts,
                    pressed: false,
                });
            }
            None => {}
        }
    }

    pub fn polygon_active(&self) -> bool {
        matches!(self.sel_gesture, Some(SelGesture::Polygon { .. }))
    }

    /// Close the polygon in progress and apply it. Fewer than three distinct
    /// corners is no shape at all, and just ends the gesture.
    pub fn polygon_finish(&mut self) {
        if !self.polygon_active() {
            return;
        }
        let Some(SelGesture::Polygon { op, mut pts, .. }) = self.sel_gesture.take() else {
            return;
        };
        // A double-click lands its second press on top of the first.
        pts.dedup_by(|a, b| dist(*a, *b) < 1e-3);
        if pts.len() < 3 {
            return;
        }
        let m = SelectionMask::from_path(&pts, self.selection_universe());
        self.apply_op(op, m);
    }

    /// Take back the newest polygon corner. `false` when no polygon is in
    /// progress, so the key can mean what it usually does.
    pub fn polygon_back(&mut self) -> bool {
        let Some(SelGesture::Polygon { pts, .. }) = &mut self.sel_gesture else {
            return false;
        };
        pts.pop();
        if pts.is_empty() {
            self.sel_gesture = None;
        }
        true
    }

    /// Abandon a selection gesture in progress. `true` if there was one.
    pub fn cancel_gesture(&mut self) -> bool {
        self.sel_gesture.take().is_some()
    }

    /// A press on the float's transform box, or on the selection itself when
    /// there is no float yet, which lifts one. `true` when the press was
    /// taken.
    fn try_grab(&mut self, doc: (f32, f32)) -> bool {
        let (cx, cy) = self.doc_to_active_cell(doc);
        let tol = HANDLE_PX / self.cell_view_scale();
        if let Some(sel) = &self.selection {
            let Some(grab) = sel.grab_at(cx, cy, tol) else {
                return false;
            };
            self.sel_drag = Some(SelDrag {
                grab,
                start: (cx, cy),
                pose: sel.pose,
            });
            return true;
        }
        if !self.mask_is_grabbable() {
            return false;
        }
        let Some(clip) = self.active_cell_mask() else {
            return false;
        };
        let Some(grab) = selection::grab_at(&clip, &Pose::default(), cx, cy, tol) else {
            return false;
        };
        if !self.make_float_from(&clip) {
            return false;
        }
        self.sel_drag = Some(SelDrag {
            grab,
            start: (cx, cy),
            pose: Pose::default(),
        });
        true
    }

    // --- Commands ---

    pub fn select_all(&mut self) {
        self.cancel_gesture();
        self.apply_op(SelOp::Replace, Some(SelectionMask::all()));
    }

    pub fn select_invert(&mut self) {
        self.cancel_gesture();
        self.land_float();
        let next = match &self.sel_mask {
            None => Some(SelectionMask::all()),
            Some(m) => m.inverted(),
        };
        self.set_mask(next, true);
    }

    /// Drop the selection: land any float where it is, then select nothing.
    pub fn deselect(&mut self) {
        self.cancel_gesture();
        self.land_float();
        self.set_mask(None, true);
    }

    pub fn grow_selection(&mut self) {
        self.reshape_selection(|m, n| m.grow(n));
    }

    pub fn shrink_selection(&mut self) {
        self.reshape_selection(|m, n| m.shrink(n));
    }

    pub fn feather_selection(&mut self) {
        self.reshape_selection(|m, n| m.feather(n));
    }

    fn reshape_selection(&mut self, f: impl FnOnce(&SelectionMask, u32) -> Option<SelectionMask>) {
        self.land_float();
        let Some(m) = &self.sel_mask else {
            return;
        };
        let next = f(m, self.sel_amount.max(1));
        self.set_mask(next, true);
    }

    /// Fill the selection with the brush colour, on the active drawing.
    pub fn fill_selection(&mut self) {
        if self.sel_mask.is_none() {
            return;
        }
        self.cancel_gesture();
        self.land_float();
        let Some(target) = self.begin_edit_cell() else {
            return;
        };
        let Some(clip) = self.active_cell_mask() else {
            return;
        };
        let color = self.brush.color;
        self.snapshot_pre(target);
        if let Some(c) = self.project.cell_mut(target) {
            c.dirty = None;
            crate::tools::fill::fill_masked(c, &clip, color);
        }
        self.mark_dirty(target);
        if self.commit_undo(target) {
            self.painted(target);
        }
    }

    /// Clear the active drawing — only inside the selection when there is one.
    /// One undo step either way.
    pub fn clear_active(&mut self) {
        self.cancel_gesture();
        self.land_float();
        if self.active_layer_locked() || self.active_layer_is_reference() {
            return;
        }
        let Some(id) = self.project.resolved_current() else {
            return;
        };
        let clip = match self.sel_mask {
            Some(_) => match self.active_cell_mask() {
                Some(m) => Some(m),
                None => return,
            },
            None => None,
        };
        self.snapshot_pre(id);
        if let Some(c) = self.project.cell_mut(id) {
            c.dirty = None;
            match &clip {
                Some(m) => {
                    lasso::erase_masked(c, m);
                }
                None => c.clear(),
            }
        }
        if self.project.cell(id).is_some_and(|c| c.dirty.is_some()) {
            self.mark_dirty(id);
        }
        self.commit_undo(id);
    }

    /// Erase what is selected. A float is simply dropped — its pixels already
    /// left the cell when it lifted. The selection itself stays, as in Krita.
    pub fn delete_selection(&mut self) {
        if let Some(sel) = self.selection.take() {
            self.sel_drag = None;
            self.retire_float_tex();
            if sel.lifted {
                // Record the lift, which nothing else now will.
                if std::mem::take(&mut self.float_pre_live) && self.project.cell(sel.cell).is_some() {
                    std::mem::swap(&mut self.stroke_pre_pixels, &mut self.float_pre);
                    self.stroke_pre_live = true;
                    let rect = self.float_lift_rect;
                    if let Some(c) = self.project.cell_mut(sel.cell) {
                        c.dirty = rect;
                    }
                    self.commit_undo(sel.cell);
                }
                return;
            }
        }
        if self.sel_mask.is_none() || self.active_layer_locked() || self.active_layer_is_reference() {
            return;
        }
        let Some(id) = self.project.resolved_current() else {
            return;
        };
        let Some(clip) = self.active_cell_mask() else {
            return;
        };
        self.snapshot_pre(id);
        if let Some(c) = self.project.cell_mut(id) {
            c.dirty = None;
            lasso::erase_masked(c, &clip);
        }
        if self.project.cell(id).is_some_and(|c| c.dirty.is_some()) {
            self.mark_dirty(id);
        }
        self.commit_undo(id);
    }

    /// Copy the selected pixels to the selection clipboard.
    ///
    /// A scaled or rotated float is baked first: what the user is looking at
    /// is what they expect to paste. A pixel-aligned one keeps its pristine
    /// buffer, so an ordinary copy still costs nothing and loses nothing.
    pub fn copy_selection(&mut self) {
        if let Some(sel) = &self.selection {
            self.pixel_clip = Some(
                sel.bake()
                    .unwrap_or_else(|| (sel.mask.clone(), sel.pixels.clone())),
            );
            return;
        }
        if self.sel_mask.is_none() {
            return;
        }
        let Some(id) = self.project.resolved_current() else {
            return;
        };
        let Some(clip) = self.active_cell_mask() else {
            return;
        };
        if clip.w == 0 || clip.h == 0 {
            return;
        }
        let Some(canvas) = self.project.cell(id) else {
            return;
        };
        let s = Selection::new(id, canvas, (*clip).clone(), Placement::default());
        self.pixel_clip = Some((s.mask, s.pixels));
    }

    /// Drop the clipboard pixels onto the active drawing as a float, so they
    /// can be placed before they land. Works across frames and layers, since
    /// it targets whatever cell is active now.
    pub fn paste_selection(&mut self) {
        let Some((mask, pixels)) = self.pixel_clip.clone() else {
            return;
        };
        if self.active_layer_locked() || self.active_layer_is_reference() {
            return;
        }
        self.cancel_gesture();
        self.land_float();
        let cell = self.project.ensure_active_cell();
        let placement = self.active_placement();
        self.selection = Some(Selection::from_clip(cell, mask, pixels, placement));
        // Clipboard pixels left no hole: landing records only the stamp.
        self.float_pre_live = false;
        self.sel_tex_stale = true;
    }

    /// Switch tool. Lands a float and abandons a selection gesture first: the
    /// transform box and a half-drawn polygon belong to the Lasso.
    pub fn set_tool(&mut self, t: ActiveTool) {
        if t != self.tool {
            self.land_float();
            self.cancel_gesture();
            self.tool_brushes[self.tool.idx()] = self.brush.clone();
            self.tool = t;
            self.brush = self.tool_brushes[t.idx()].clone();
        }
        if t == ActiveTool::Perspective {
            self.ensure_perspective_grid();
        }
    }

    // --- The float ---

    /// Lift the selection on the active drawing into a float, if there isn't
    /// one already — for the arrow keys, which move pixels without a press.
    pub fn make_float(&mut self) -> bool {
        if self.selection.is_some() {
            return true;
        }
        if self.sel_mask.is_none() || self.active_layer_locked() || self.active_layer_is_reference() {
            return false;
        }
        let Some(clip) = self.active_cell_mask() else {
            return false;
        };
        self.make_float_from(&clip)
    }

    fn make_float_from(&mut self, clip: &Mask) -> bool {
        if clip.w == 0 || clip.h == 0 {
            return false;
        }
        let Some(cell) = self.project.resolved_current() else {
            return false;
        };
        let placement = self.active_placement();
        let Some(canvas) = self.project.cell(cell) else {
            return false;
        };
        self.selection = Some(Selection::new(cell, canvas, clip.clone(), placement));
        self.float_pre_live = false;
        self.sel_tex_stale = true;
        true
    }

    /// Put a float down: stamp its pixels into its cell and move the selection
    /// to where they landed. One undo step for all of it — the lift, the stamp
    /// and the selection — so one Ctrl+Z puts everything back where it was
    /// picked up. Safe to call with nothing floating.
    pub fn land_float(&mut self) {
        let Some(sel) = self.selection.take() else {
            return;
        };
        self.sel_drag = None;
        self.retire_float_tex();
        // Never lifted means never moved: the cell was left untouched, and the
        // selection is where it was.
        if !sel.lifted {
            self.float_pre_live = false;
            return;
        }
        let cell = sel.cell;
        if self.project.cell(cell).is_none() {
            self.float_pre_live = false;
            return;
        }
        let from_lift = std::mem::take(&mut self.float_pre_live);
        if from_lift {
            // The "before" is the cell as it was before the pixels left it.
            std::mem::swap(&mut self.stroke_pre_pixels, &mut self.float_pre);
            self.stroke_pre_live = true;
            let rect = self.float_lift_rect;
            if let Some(c) = self.project.cell_mut(cell) {
                c.dirty = rect;
            }
        } else {
            // Pasted pixels: nothing was lifted, only the stamp to record.
            self.snapshot_pre(cell);
            if let Some(c) = self.project.cell_mut(cell) {
                c.dirty = None;
            }
        }
        if let Some(c) = self.project.cell_mut(cell) {
            sel.stamp(c);
        }
        self.mark_dirty(cell);

        let mut parts: Vec<undo::Command> = self.take_patch(cell).into_iter().collect();
        // All frames: the same move, through the same selection, on every other
        // drawing of the layer — while `sel_mask` still says where the pixels
        // were picked up. Pasted pixels came from no drawing, so they stay put.
        if self.sel_all_frames && from_lift {
            parts.extend(self.land_on_other_drawings(&sel));
        }
        let (x, y, w, h, cov) = sel.posed_cov();
        let p = sel.placement;
        let after = SelectionMask::from_cell(x, y, w, h, &cov, &p.xf, p.cw, p.ch, p.pw, p.ph);
        if after != self.sel_mask {
            parts.push(undo::Command::Selection {
                before: self.sel_mask.as_ref().map(SelectionMask::pack),
                after: after.as_ref().map(SelectionMask::pack),
            });
            self.sel_mask = after;
            self.mask_changed();
        }
        match parts.len() {
            0 => {}
            1 => self.history.push(parts.remove(0)),
            _ => self.history.push(undo::Command::Compound(parts)),
        }
    }

    /// Each distinct drawing on layer `li`, with the first frame it shows on.
    /// A drawing held over several frames, or keyed twice, appears once — so
    /// an edit applied to each lands once.
    pub fn layer_drawings(&self, li: usize) -> Vec<(crate::doc::layer::CellId, usize)> {
        let Some(layer) = self.project.layers.get(li) else {
            return Vec::new();
        };
        let mut seen = std::collections::HashSet::new();
        layer
            .exposures
            .iter()
            .enumerate()
            .filter_map(|(f, e)| e.filter(|id| seen.insert(*id)).map(|id| (id, f)))
            .collect()
    }

    /// Land `sel`'s move on every other drawing of its layer: each one is cut
    /// through the same document-space selection and moved the same way on
    /// screen. Returns their undo patches, for `land_float` to fold into its
    /// one step.
    fn land_on_other_drawings(&mut self, sel: &Selection) -> Vec<undo::Command> {
        let Some(doc_mask) = self.sel_mask.clone() else {
            return Vec::new();
        };
        // The float's own layer, which is not the active one when it is being
        // landed because the user switched layer.
        let Some(li) = (0..self.project.layers.len()).find(|&li| {
            self.project.layers[li].exposures.contains(&Some(sel.cell))
        }) else {
            return Vec::new();
        };
        let (pw, ph) = (self.project.width as f32, self.project.height as f32);
        let mut parts = Vec::new();
        for (cell, f) in self.layer_drawings(li) {
            if cell == sel.cell {
                continue;
            }
            let Some(canvas) = self.project.cell(cell) else {
                continue;
            };
            let pk = Placement {
                xf: self.display_transform(li, f),
                cw: canvas.width,
                ch: canvas.height,
                pw,
                ph,
            };
            let mask = doc_mask.to_cell(&pk.xf, pk.cw, pk.ch, pw, ph);
            if mask.w == 0 || mask.h == 0 {
                continue;
            }
            let mut other = Selection::new(cell, canvas, mask, pk);
            other.pose = pose_for(sel, &pk, &other.mask);
            self.snapshot_pre(cell);
            if let Some(c) = self.project.cell_mut(cell) {
                c.dirty = None;
                other.lift_source(c);
                other.stamp(c);
            }
            self.mark_dirty(cell);
            parts.extend(self.take_patch(cell));
        }
        parts
    }

    /// Put a float's pixels back where they were lifted from and forget it —
    /// what Ctrl+Z does while pixels are floating. Nothing was recorded for
    /// the float yet, so the history is left alone. `true` if there was one.
    pub(super) fn cancel_float(&mut self) -> bool {
        let Some(sel) = self.selection.take() else {
            return false;
        };
        self.sel_drag = None;
        self.retire_float_tex();
        if sel.lifted && std::mem::take(&mut self.float_pre_live) {
            let rect = self.float_lift_rect;
            let mut restored = false;
            if let (Some(r), Some(c)) = (rect, self.project.cell_mut(sel.cell)) {
                if c.pixels.len() == self.float_pre.len() {
                    let stride = c.width as usize * 4;
                    for y in r.min_y as usize..r.max_y as usize {
                        let a = y * stride + r.min_x as usize * 4;
                        let b = y * stride + r.max_x as usize * 4;
                        c.pixels[a..b].copy_from_slice(&self.float_pre[a..b]);
                    }
                    c.dirty = Some(r);
                    restored = true;
                }
            }
            if restored {
                self.mark_dirty(sel.cell);
            }
        }
        self.float_pre_live = false;
        true
    }

    /// Nudge the float by whole pixels (arrow keys, and the plain drag). Stays
    /// integral, so a move alone never reaches the resampler.
    pub fn nudge_selection(&mut self, dx: i32, dy: i32) {
        let Some(sel) = self.selection.as_mut() else {
            return;
        };
        sel.pose.offset.0 += dx as f32;
        sel.pose.offset.1 += dy as f32;
        self.touch_selection();
    }

    /// Apply a transform-box drag to the float.
    ///
    /// A move accumulates in whole pixels, exactly as it always has — the drag
    /// anchor walks with the pointer and sub-pixel remainders are dropped, so
    /// dragging a selection around is still lossless. A scale or rotation is
    /// re-solved from the pose recorded at the press, so it depends only on
    /// where the pointer is now.
    pub(super) fn drag_selection(&mut self, drag: SelDrag, x: f32, y: f32) {
        if let selection::Grab::Move = drag.grab {
            let (dx, dy) = (
                (x - drag.start.0).round() as i32,
                (y - drag.start.1).round() as i32,
            );
            if dx == 0 && dy == 0 {
                return;
            }
            if let Some(d) = self.sel_drag.as_mut() {
                d.start = (x, y);
            }
            self.nudge_selection(dx, dy);
            return;
        }
        let uniform = self.shift_held;
        let Some(sel) = self.selection.as_mut() else {
            return;
        };
        let pose = drag
            .pose
            .dragged(&sel.mask, drag.grab, drag.start, (x, y), uniform);
        if pose == sel.pose {
            return;
        }
        sel.pose = pose;
        self.touch_selection();
    }

    /// Note that the float has been posed, erasing the source behind it the
    /// first time that happens. Shared by every gesture, so a first *rotate*
    /// lifts exactly like a first move does.
    fn touch_selection(&mut self) {
        let Some(sel) = self.selection.as_ref() else {
            return;
        };
        if sel.lifted {
            return;
        }
        let cell = sel.cell;
        self.lift_selection_source(cell);
    }

    /// Erase the source region behind the float, once. Not recorded yet: the
    /// cell as it was is kept in `float_pre`, and the landing records lift and
    /// stamp together — or an undo while floating puts it straight back.
    fn lift_selection_source(&mut self, cell: crate::doc::layer::CellId) {
        if self.project.cell(cell).is_none() {
            return;
        }
        let Some(mut sel) = self.selection.take() else {
            return;
        };
        self.float_pre.clear();
        self.float_pre
            .extend_from_slice(&self.project.cells[cell].pixels);
        self.float_pre_live = true;
        self.float_lift_rect = None;
        if let Some(c) = self.project.cell_mut(cell) {
            c.dirty = None;
            sel.lift_source(c);
            self.float_lift_rect = c.dirty;
        }
        self.selection = Some(sel);
        if self.float_lift_rect.is_some() {
            self.mark_dirty(cell);
        }
    }

    /// Land a float whose cell is no longer the active one — scrubbing to
    /// another frame must not leave pixels hovering over a drawing they do not
    /// belong to.
    fn land_float_if_orphaned(&mut self) {
        let Some(sel) = &self.selection else {
            return;
        };
        if self.project.resolved_current() != Some(sel.cell) {
            self.land_float();
        }
    }

    fn retire_float_tex(&mut self) {
        if let Some(old) = self.selection_tex.take() {
            self.retired_textures.push(old);
        }
    }

    /// Forget the selection, float and all, without writing anything back —
    /// for File → New and Open, where the cells it points at are gone.
    pub(super) fn clear_selection_state(&mut self) {
        self.selection = None;
        self.sel_drag = None;
        self.retire_float_tex();
        self.float_pre_live = false;
        self.sel_gesture = None;
        self.sel_mask = None;
        self.mask_changed();
    }

    /// The tint texture over everything outside the selection, rebuilt only
    /// when the selection changes. Downsampled so its long side stays within
    /// 2048 texels: it is a dimming wash, not artwork.
    pub(super) fn sync_tint(&mut self, ctx: &egui::Context) {
        let want = self.tint_outside && self.sel_mask.is_some();
        if !want {
            if let Some(old) = self.sel_tint.take() {
                self.retired_textures.push(old.tex);
            }
            return;
        }
        if self.sel_tint.as_ref().is_some_and(|t| t.ver == self.sel_ver) {
            return;
        }
        let Some(m) = &self.sel_mask else {
            return;
        };
        let (pw, ph) = (self.project.width as i32, self.project.height as i32);
        let (mut x0, mut y0, mut x1, mut y1) = (0, 0, pw, ph);
        if let Some((a, b, c, d)) = m.bounds() {
            x0 = x0.min(a);
            y0 = y0.min(b);
            x1 = x1.max(c);
            y1 = y1.max(d);
        }
        let (w, h) = ((x1 - x0).max(1), (y1 - y0).max(1));
        let step = ((w.max(h) + 2047) / 2048).max(1);
        let (tw, th) = (((w + step - 1) / step) as usize, ((h + step - 1) / step) as usize);
        let mut pixels = Vec::with_capacity(tw * th);
        for j in 0..th as i32 {
            for i in 0..tw as i32 {
                let c = m.at(x0 + i * step + step / 2, y0 + j * step + step / 2);
                let a = ((255 - c) as f32 * 0.45).round() as u8;
                pixels.push(egui::Color32::from_black_alpha(a));
            }
        }
        let image = egui::ColorImage {
            size: [tw, th],
            pixels,
        };
        let rect = (
            x0 as f32,
            y0 as f32,
            (x0 + tw as i32 * step) as f32,
            (y0 + th as i32 * step) as f32,
        );
        let tex = ctx.load_texture("selection_tint", image, egui::TextureOptions::LINEAR);
        let fresh = SelTint {
            ver: self.sel_ver,
            rect,
            tex,
        };
        if let Some(old) = self.sel_tint.replace(fresh) {
            self.retired_textures.push(old.tex);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::shortcuts::{Action, ShortcutMap};

    /// A fresh state with the Lasso up, drawing rectangles.
    fn lasso() -> AppState {
        let mut st = AppState::for_test();
        st.shortcuts = ShortcutMap::default();
        st.set_tool(ActiveTool::Lasso);
        st.sel_shape = SelShape::Rect;
        st
    }

    /// Fill `x0..x1, y0..y1` of the active cell with `rgba`, outside the
    /// history — the drawing a test starts from.
    fn paint(st: &mut AppState, (x0, y0, x1, y1): (u32, u32, u32, u32), rgba: [u8; 4]) {
        let id = st.project.ensure_active_cell();
        let c = st.project.cell_mut(id).unwrap();
        for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * c.width + x) * 4) as usize;
                c.pixels[i..i + 4].copy_from_slice(&rgba);
            }
        }
    }

    fn px(st: &AppState, x: u32, y: u32) -> [u8; 4] {
        let id = st.project.resolved_current().unwrap();
        let c = st.project.cell(id).unwrap();
        let i = ((y * c.width + x) * 4) as usize;
        [c.pixels[i], c.pixels[i + 1], c.pixels[i + 2], c.pixels[i + 3]]
    }

    /// Drag the current selection shape from `a` to `b` in document space.
    fn drag(st: &mut AppState, a: (f32, f32), b: (f32, f32), held: Option<SelOp>) {
        st.select_down(a, held);
        for i in 1..=8 {
            let t = i as f32 / 8.0;
            st.select_move((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
        st.select_up();
    }

    fn at(st: &AppState, x: i32, y: i32) -> u8 {
        st.sel_mask.as_ref().map_or(0, |m| m.at(x, y))
    }

    /// An Ink stroke along `y`, from `x0` to `x1`, in cell space.
    fn ink_line(st: &mut AppState, y: f32, x0: f32, x1: f32) {
        st.set_tool(ActiveTool::Ink);
        st.brush.radius = 3.0;
        st.pointer_down(st.make_sample(x0, y, 0.0));
        for i in 1..=40 {
            let x = x0 + (x1 - x0) * i as f32 / 40.0;
            st.pointer_move(st.make_sample(x, y, i as f32 * 0.01));
        }
        st.pointer_up();
    }

    #[test]
    fn a_selection_is_one_undo_step() {
        let mut st = lasso();
        drag(&mut st, (100.0, 100.0), (200.0, 160.0), None);
        assert_eq!(at(&st, 150, 130), 255);
        assert_eq!(at(&st, 250, 130), 0);
        st.undo();
        assert!(st.sel_mask.is_none());
        st.redo();
        assert_eq!(at(&st, 150, 130), 255);
    }

    #[test]
    fn strokes_stay_inside_the_selection() {
        let mut st = lasso();
        drag(&mut st, (100.0, 100.0), (200.0, 200.0), None);
        ink_line(&mut st, 150.0, 40.0, 260.0);
        assert!(px(&st, 150, 150)[3] > 200, "inside");
        assert_eq!(px(&st, 60, 150)[3], 0, "left of the selection");
        assert_eq!(px(&st, 240, 150)[3], 0, "right of the selection");
    }

    /// The selection lives in document space: on a layer shifted 10 px right,
    /// the clip lands 10 px left in the layer's own pixels.
    #[test]
    fn a_moved_layer_is_clipped_where_the_selection_is_on_screen() {
        let mut st = lasso();
        st.project.layers[0].transform.tx = 10.0;
        drag(&mut st, (100.0, 100.0), (200.0, 200.0), None);
        ink_line(&mut st, 150.0, 20.0, 260.0);
        // Cell u is document u - 10.
        assert!(px(&st, 95, 150)[3] > 200, "doc 105 is inside");
        assert_eq!(px(&st, 85, 150)[3], 0, "doc 95 is outside");
        assert!(px(&st, 185, 150)[3] > 200, "doc 195 is inside");
        assert_eq!(px(&st, 195, 150)[3], 0, "doc 205 is outside");
    }

    #[test]
    fn dragging_inside_moves_the_pixels_and_the_selection_follows() {
        let mut st = lasso();
        paint(&mut st, (100, 100, 140, 140), [200, 0, 0, 255]);
        drag(&mut st, (90.0, 90.0), (150.0, 150.0), None);
        let steps = st.history.undo_len();
        // A plain drag from inside: a move, 30 px right.
        drag(&mut st, (120.0, 120.0), (150.0, 120.0), None);
        assert!(st.selection.is_some(), "floating");
        st.land_float();
        assert_eq!(px(&st, 165, 120), [200, 0, 0, 255]);
        assert_eq!(px(&st, 105, 120)[3], 0, "the hole it left");
        assert_eq!(at(&st, 170, 120), 255, "selection moved with it");
        assert_eq!(at(&st, 95, 120), 0);
        assert_eq!(st.history.undo_len(), steps + 1, "a move is one undo step");
        st.undo();
        assert_eq!(px(&st, 105, 120), [200, 0, 0, 255]);
        assert_eq!(px(&st, 165, 120)[3], 0);
        assert_eq!(at(&st, 95, 120), 255);
    }

    const RED: [u8; 4] = [200, 0, 0, 255];

    /// Pixel of layer 0's drawing on frame `f`, in that drawing's own pixels.
    fn px_on(st: &AppState, f: usize, x: u32, y: u32) -> [u8; 4] {
        let id = st.project.layers[0].resolve(f).expect("a drawing on this frame");
        let c = st.project.cell(id).unwrap();
        let i = ((y * c.width + x) * 4) as usize;
        [c.pixels[i], c.pixels[i + 1], c.pixels[i + 2], c.pixels[i + 3]]
    }

    /// Next frame, with a drawing of its own.
    fn new_drawing(st: &mut AppState) {
        st.structural_edit(false, |p| {
            p.add_frame();
            p.insert_blank_key_here();
        });
    }

    /// Three drawings on frames 0-2 with a red square at 100..140 in each,
    /// frame 3 holding the third; the playhead back on frame 0.
    fn animated() -> AppState {
        let mut st = lasso();
        paint(&mut st, (100, 100, 140, 140), RED);
        for _ in 0..2 {
            new_drawing(&mut st);
            paint(&mut st, (100, 100, 140, 140), RED);
        }
        st.structural_edit(false, |p| p.add_frame());
        st.project.goto(0);
        assert_eq!(st.layer_drawings(0).len(), 3, "the hold is not a drawing");
        st
    }

    #[test]
    fn all_frames_moves_every_drawing_once_in_one_undo() {
        let mut st = animated();
        drag(&mut st, (90.0, 90.0), (150.0, 150.0), None);
        st.sel_all_frames = true;
        let steps = st.history.undo_len();
        drag(&mut st, (120.0, 120.0), (150.0, 120.0), None);
        st.land_float();
        for f in 0..4 {
            // Once, 30 px: 130..170. Twice would reach 160..200.
            assert_eq!(px_on(&st, f, 135, 120), RED, "frame {f} moved");
            assert_eq!(px_on(&st, f, 185, 120)[3], 0, "frame {f} moved once");
            assert_eq!(px_on(&st, f, 105, 120)[3], 0, "frame {f} left a hole");
        }
        assert_eq!(st.history.undo_len(), steps + 1, "every drawing, one step");
        st.undo();
        for f in 0..4 {
            assert_eq!(px_on(&st, f, 105, 120), RED, "frame {f} back");
            assert_eq!(px_on(&st, f, 165, 120)[3], 0);
        }
        st.redo();
        for f in 0..4 {
            assert_eq!(px_on(&st, f, 165, 120), RED, "frame {f} redone");
        }
    }

    #[test]
    fn without_all_frames_only_this_drawing_moves() {
        let mut st = animated();
        drag(&mut st, (90.0, 90.0), (150.0, 150.0), None);
        drag(&mut st, (120.0, 120.0), (150.0, 120.0), None);
        st.land_float();
        assert_eq!(px_on(&st, 0, 165, 120), RED);
        for f in 1..4 {
            assert_eq!(px_on(&st, f, 105, 120), RED, "frame {f} untouched");
            assert_eq!(px_on(&st, f, 165, 120)[3], 0);
        }
    }

    #[test]
    fn pasted_pixels_land_on_one_drawing_even_with_all_frames() {
        let mut st = animated();
        drag(&mut st, (90.0, 90.0), (150.0, 150.0), None);
        st.copy_selection();
        st.sel_all_frames = true;
        st.paste_selection();
        st.nudge_selection(60, 0);
        st.land_float();
        assert_eq!(px_on(&st, 0, 165, 120), RED, "pasted here");
        assert_eq!(px_on(&st, 0, 105, 120), RED, "and the original stays");
        assert_eq!(px_on(&st, 1, 165, 120)[3], 0, "nowhere else");
    }

    /// Frame 1's layer is keyed 15 px right, so its drawing's square sits at
    /// 85..125 in its own pixels to show up where the others do. The move is
    /// the same on screen: 30 px right in every drawing.
    #[test]
    fn a_keyed_layer_moves_the_same_on_screen() {
        use crate::doc::transform::Transform;
        let mut st = lasso();
        paint(&mut st, (100, 100, 140, 140), RED);
        new_drawing(&mut st);
        paint(&mut st, (85, 100, 125, 140), RED);
        let shifted = Transform { tx: 15.0, ..Transform::default() };
        st.project.layers[0].set_transform_key(0, Transform::default());
        st.project.layers[0].set_transform_key(1, shifted);
        st.project.goto(0);
        st.project.layers[0].transform = Transform::default();

        drag(&mut st, (90.0, 90.0), (150.0, 150.0), None);
        st.sel_all_frames = true;
        drag(&mut st, (120.0, 120.0), (150.0, 120.0), None);
        st.land_float();
        assert_eq!(px_on(&st, 0, 135, 120), RED);
        assert_eq!(px_on(&st, 1, 120, 120), RED, "frame 1: 115..155 in its pixels");
        assert_eq!(px_on(&st, 1, 110, 120)[3], 0, "and not 15 px short of it");
        assert_eq!(px_on(&st, 1, 160, 120)[3], 0, "nor past it");
    }

    #[test]
    fn a_rotation_lands_identically_on_every_drawing() {
        let mut st = animated();
        // An L, so a quarter turn is visible.
        for f in 0..3 {
            st.project.goto(f);
            paint(&mut st, (100, 100, 140, 140), [0, 0, 0, 0]);
            paint(&mut st, (100, 100, 110, 140), RED);
            paint(&mut st, (100, 130, 140, 140), RED);
        }
        st.project.goto(0);
        drag(&mut st, (90.0, 90.0), (150.0, 150.0), None);
        st.sel_all_frames = true;
        assert!(st.make_float());
        if let Some(sel) = st.selection.as_mut() {
            sel.pose.rot = std::f32::consts::FRAC_PI_2;
        }
        st.touch_selection();
        st.land_float();
        let cell = |f: usize| {
            let id = st.project.layers[0].resolve(f).unwrap();
            st.project.cell(id).unwrap().pixels.clone()
        };
        let first = cell(0);
        assert_ne!(first, {
            let mut before = lasso();
            paint(&mut before, (100, 100, 110, 140), RED);
            paint(&mut before, (100, 130, 140, 140), RED);
            before.project.cells[0].pixels.clone()
        }, "it did turn");
        for f in 1..3 {
            assert!(cell(f) == first, "frame {f} matches frame 0");
        }
    }

    #[test]
    fn undo_while_floating_puts_the_pixels_back_untouched() {
        let mut st = lasso();
        paint(&mut st, (100, 100, 140, 140), [10, 200, 30, 255]);
        let before = st.project.cells[0].pixels.clone();
        drag(&mut st, (90.0, 90.0), (150.0, 150.0), None);
        let mask = st.sel_mask.clone();
        let steps = st.history.undo_len();
        drag(&mut st, (120.0, 120.0), (170.0, 140.0), None);
        st.undo();
        assert!(st.selection.is_none());
        assert_eq!(st.project.cells[0].pixels, before);
        assert_eq!(st.history.undo_len(), steps);
        assert_eq!(st.sel_mask, mask);
    }

    #[test]
    fn the_selection_survives_frame_and_layer_changes() {
        let mut st = lasso();
        drag(&mut st, (10.0, 10.0), (50.0, 50.0), None);
        st.dispatch(Action::LayerAdd);
        st.dispatch(Action::FrameAdd);
        st.dispatch(Action::FrameNext);
        st.sync_selection();
        assert_eq!(at(&st, 30, 30), 255);
    }

    #[test]
    fn clear_empties_only_the_selection_and_can_be_undone() {
        let mut st = lasso();
        paint(&mut st, (0, 0, 300, 300), [0, 0, 0, 255]);
        drag(&mut st, (100.0, 100.0), (200.0, 200.0), None);
        st.dispatch(Action::ClearCell);
        assert_eq!(px(&st, 150, 150)[3], 0);
        assert_eq!(px(&st, 50, 50)[3], 255);
        assert!(st.sel_mask.is_some(), "the selection stays");
        st.undo();
        assert_eq!(px(&st, 150, 150)[3], 255);
    }

    #[test]
    fn switching_tool_by_key_lands_the_float() {
        let mut st = lasso();
        paint(&mut st, (100, 100, 140, 140), [0, 0, 255, 255]);
        drag(&mut st, (90.0, 90.0), (150.0, 150.0), None);
        drag(&mut st, (120.0, 120.0), (160.0, 120.0), None);
        st.dispatch(Action::ToolPencil);
        assert!(st.selection.is_none());
        assert_eq!(px(&st, 170, 120), [0, 0, 255, 255]);
    }

    #[test]
    fn a_structural_edit_lands_the_float_first() {
        let mut st = lasso();
        paint(&mut st, (100, 100, 140, 140), [0, 0, 255, 255]);
        drag(&mut st, (90.0, 90.0), (150.0, 150.0), None);
        drag(&mut st, (120.0, 120.0), (160.0, 120.0), None);
        st.dispatch(Action::LayerAdd);
        assert!(st.selection.is_none());
        assert_eq!(st.project.cells[0].pixels[((120 * 1280 + 170) * 4 + 2) as usize], 255);
    }

    #[test]
    fn a_new_project_forgets_the_selection() {
        let mut st = lasso();
        drag(&mut st, (10.0, 10.0), (50.0, 50.0), None);
        st.reset_with(640, 480, 24.0);
        assert!(st.sel_mask.is_none() && st.sel_outline.is_empty());
    }

    #[test]
    fn held_modes_add_remove_and_intersect() {
        let mut st = lasso();
        drag(&mut st, (0.0, 0.0), (100.0, 100.0), None);
        // Held Add starts a new shape even from inside the selection.
        drag(&mut st, (50.0, 50.0), (150.0, 100.0), Some(SelOp::Add));
        assert!(st.selection.is_none(), "not a move");
        assert_eq!(at(&st, 120, 70), 255);
        assert_eq!(at(&st, 20, 20), 255);
        drag(&mut st, (40.0, 0.0), (60.0, 100.0), Some(SelOp::Subtract));
        assert_eq!(at(&st, 50, 50), 0);
        assert_eq!(at(&st, 20, 50), 255);
        drag(&mut st, (0.0, 40.0), (200.0, 60.0), Some(SelOp::Intersect));
        assert_eq!(at(&st, 20, 20), 0);
        assert_eq!(at(&st, 20, 50), 255);
        assert_eq!(at(&st, 120, 50), 255);
    }

    #[test]
    fn a_polygon_takes_a_corner_per_click() {
        let mut st = lasso();
        st.sel_shape = SelShape::Polygon;
        for p in [(100.0, 100.0), (300.0, 100.0), (200.0, 260.0)] {
            st.select_down(p, None);
            st.select_up();
        }
        assert!(st.sel_mask.is_none(), "not closed yet");
        st.polygon_finish();
        assert_eq!(at(&st, 200, 150), 255);
        assert_eq!(at(&st, 110, 240), 0);
        // Esc-style cancel leaves the selection alone.
        st.select_down((10.0, 10.0), None);
        st.select_up();
        assert!(st.cancel_gesture());
        assert_eq!(at(&st, 200, 150), 255);
    }

    #[test]
    fn a_click_deselects_in_replace_mode_only() {
        let mut st = lasso();
        drag(&mut st, (100.0, 100.0), (200.0, 200.0), None);
        drag(&mut st, (400.0, 400.0), (401.0, 400.0), Some(SelOp::Add));
        assert!(st.sel_mask.is_some(), "a click in Add mode does nothing");
        drag(&mut st, (400.0, 400.0), (401.0, 400.0), None);
        assert!(st.sel_mask.is_none());
    }

    #[test]
    fn grow_and_fill_act_on_the_selection() {
        let mut st = lasso();
        drag(&mut st, (100.0, 100.0), (200.0, 200.0), None);
        st.sel_amount = 5;
        st.grow_selection();
        assert_eq!(at(&st, 96, 150), 255);
        st.undo();
        assert_eq!(at(&st, 96, 150), 0);
        st.brush.color = [0, 120, 0, 255];
        st.fill_selection();
        assert_eq!(px(&st, 150, 150), [0, 120, 0, 255]);
        assert_eq!(px(&st, 90, 150)[3], 0);
    }

    #[test]
    fn delete_erases_inside_and_keeps_the_selection() {
        let mut st = lasso();
        paint(&mut st, (0, 0, 300, 300), [9, 9, 9, 255]);
        drag(&mut st, (100.0, 100.0), (200.0, 200.0), None);
        st.dispatch(Action::SelectionDelete);
        assert_eq!(px(&st, 150, 150)[3], 0);
        assert_eq!(px(&st, 250, 150)[3], 255);
        assert!(st.sel_mask.is_some());
    }

    /// The same, driven through the real canvas: a Ctrl+Shift drag adds, and
    /// Ctrl alone still zooms while the Lasso is up.
    #[test]
    fn modifiers_on_the_canvas_pick_the_selection_mode() {
        let mut st = lasso();
        st.show_panels = false;
        st.show_mini_timeline = false;
        drag(&mut st, (0.0, 0.0), (40.0, 40.0), None);
        let area = |st: &AppState| {
            st.sel_mask
                .as_ref()
                .map_or(0, |m| m.cov.iter().filter(|&&c| c >= 128).count())
        };
        let before = area(&st);
        let ctx = egui::Context::default();
        let run = |st: &mut AppState, events: Vec<egui::Event>, modifiers: egui::Modifiers| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                events,
                modifiers,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                let tab_focus = st.handle_keys(ctx);
                crate::ui::shell::draw(st, ctx);
                st.drop_stray_focus(ctx, tab_focus);
            });
        };
        let gesture = |st: &mut AppState, modifiers: egui::Modifiers| {
            let (from, to) = (egui::pos2(500.0, 300.0), egui::pos2(700.0, 450.0));
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            };
            // Hover at the start first: a drag is measured from where the
            // pointer was, and the last gesture left it at `to`.
            run(st, vec![egui::Event::PointerMoved(from)], modifiers);
            run(st, vec![egui::Event::PointerMoved(from), button(from, true)], modifiers);
            for i in 1..=4 {
                let p = from + (to - from) * (i as f32 / 4.0);
                run(st, vec![egui::Event::PointerMoved(p)], modifiers);
            }
            run(st, vec![button(to, false)], modifiers);
            run(st, vec![], egui::Modifiers::NONE);
        };
        let ctrl_shift = egui::Modifiers {
            ctrl: true,
            shift: true,
            command: true,
            ..Default::default()
        };
        gesture(&mut st, ctrl_shift);
        assert!(area(&st) > before, "Ctrl+Shift drag added to the selection");
        assert_eq!(at(&st, 20, 20), 255, "and kept what was there");

        let added = st.sel_mask.clone();
        let zoom = st.view.zoom;
        let ctrl = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        gesture(&mut st, ctrl);
        assert_eq!(st.sel_mask, added, "Ctrl alone is not a selection mode");
        assert!((st.view.zoom - zoom).abs() > 1e-3, "it still zooms");
    }
}
