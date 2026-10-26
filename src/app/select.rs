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
use crate::doc::layer::CellId;
use crate::tools::axis_lock;
use crate::tools::fill::{self, Grain};
use crate::tools::lasso::{self, Mask};
use crate::tools::select_mask::{self, SelOp, SelShape, SelectionMask};
use crate::tools::selection::{self, Grab, Placement, Pose, Selection, HANDLE_PX};
use crate::tools::tap_lasso::TapLasso;
use crate::tools::wand::{self, WandPick};
use crate::tools::ActiveTool;
use crate::undo;

/// An in-progress selection gesture, in document space.
#[derive(Clone, Debug)]
pub enum SelGesture {
    /// Rectangle or ellipse: one press, drag, release.
    Drag {
        op: SelOp,
        shape: SelShape,
        start: (f32, f32),
        end: (f32, f32),
        /// Furthest the pointer got from `start` — tells a click from a drag.
        max_dist: f32,
    },
    /// The lasso: dragged freehand, or tapped out as a polygon with
    /// freehand stretches between corners. See [`TapLasso`].
    Lasso { op: SelOp, lasso: TapLasso },
}

/// A magic select press still under the pen: what it reads and the selection
/// it started from, so a drag can retune it and Esc can put things back.
pub struct WandDrag {
    pick: WandPick,
    /// Where the pixels it reads sit: the active cell, or `None` for the
    /// document itself when it reads every visible layer.
    at: Option<Placement>,
    op: SelOp,
    /// The selection before the press.
    before: Option<SelectionMask>,
    /// Where on screen the press landed; a drag counts from here.
    anchor: Option<egui::Pos2>,
    base_tolerance: u8,
    base_gap: u8,
}

/// `shape` combined into `before` by `op`. `None` is a shape that covered
/// nothing: Replace with it selects nothing, Intersect leaves nothing, and Add
/// or Remove leave `before` as it was.
fn combined(
    op: SelOp,
    before: Option<&SelectionMask>,
    shape: Option<SelectionMask>,
) -> Option<SelectionMask> {
    match (op, shape) {
        (SelOp::Replace, m) => m,
        (SelOp::Intersect, None) => None,
        (_, None) => before.cloned(),
        (op, Some(b)) => SelectionMask::combine(before, &b, op),
    }
}

/// What the cached cell mask was built for: the selection's version and where
/// the active cell sits.
#[derive(Clone, Copy, PartialEq)]
pub struct ClipKey {
    ver: u64,
    at: Placement,
}

/// A click that travels less than this many screen pixels is a click, not a
/// drag: on a rectangle or ellipse, in Replace mode, it deselects; on the
/// lasso it is a polygon corner.
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
    pub(super) fn selection_universe(&self) -> (i32, i32, i32, i32) {
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
        let next = combined(op, self.sel_mask.as_ref(), shape);
        self.set_mask(next, true);
    }

    // --- Gestures ---

    /// Selection press at document point `doc`. `held` is the mode a held
    /// modifier asks for, which beats the sticky one for this drag.
    pub fn select_down(&mut self, doc: (f32, f32), held: Option<SelOp>) {
        self.playback.stop();
        if self.polygon_active() {
            // A press back on the first corner closes the shape; anywhere
            // else adds a corner, and a drag from it a freehand stretch.
            let close = HANDLE_PX * 1.5 / self.view_scale.max(1e-6);
            let closes = match &mut self.sel_gesture {
                Some(SelGesture::Lasso { lasso, .. }) => lasso.press(doc, doc, close),
                _ => false,
            };
            if closes {
                self.polygon_finish();
            }
            return;
        }
        let op = held.unwrap_or(self.sel_op);
        // A plain press on the selection moves it. A held modifier always
        // starts a new shape, even inside — that is how you add to a
        // selection from within it.
        if held.is_none() && op == SelOp::Replace && self.try_grab(doc) {
            if self.sel_shape == SelShape::Wand {
                self.wand_repick = Some(doc);
            }
            return;
        }
        if self.sel_shape == SelShape::Wand {
            self.wand_down(doc, op);
            return;
        }
        self.land_float();
        self.sel_gesture = Some(match self.sel_shape {
            SelShape::Freehand => SelGesture::Lasso {
                op,
                lasso: TapLasso::start(doc),
            },
            shape => SelGesture::Drag {
                op,
                shape,
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
        let tap = CLICK_PX / self.view_scale.max(1e-6);
        match &mut self.sel_gesture {
            Some(SelGesture::Drag {
                start,
                end,
                max_dist,
                ..
            }) => {
                *end = doc;
                *max_dist = max_dist.max(dist(*start, doc));
            }
            Some(SelGesture::Lasso { lasso, .. }) => lasso.drag(doc, tap, step),
            None => {}
        }
    }

    /// Selection release.
    pub fn select_up(&mut self) {
        let repick = self.wand_repick.take();
        if self.sel_drag.take().is_some() {
            // A press on the selection that never moved lifted nothing: put
            // the candidate float away and leave everything as it was — or,
            // with the magic select, pick afresh where it pressed.
            if self.selection.as_ref().is_some_and(|s| !s.lifted) {
                self.selection = None;
                self.retire_float_tex();
                if let Some(doc) = repick {
                    self.wand_down(doc, SelOp::Replace);
                    self.wand_up();
                }
            }
            return;
        }
        if self.wand_up() {
            return;
        }
        match self.sel_gesture.take() {
            Some(SelGesture::Drag {
                op,
                shape,
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
                let path = self.sel_drag_path(shape, start, end);
                let m = SelectionMask::from_path(&path, self.selection_universe());
                self.apply_op(op, m);
            }
            // A drag lands; a tap leaves a polygon to tap out.
            Some(SelGesture::Lasso { op, mut lasso }) => {
                if lasso.release() {
                    self.apply_lasso(op, lasso);
                } else {
                    self.sel_gesture = Some(SelGesture::Lasso { op, lasso });
                }
            }
            None => {}
        }
    }

    /// The outline a rectangle or ellipse selection dragged from `start` to
    /// `end` makes, in document space — a square or circle while Shift
    /// locks. What the drag shows and what it lands are both this.
    pub fn sel_drag_path(&self, shape: SelShape, start: (f32, f32), end: (f32, f32)) -> Vec<(f32, f32)> {
        let end = if self.axis_lock_on() {
            axis_lock::square(start, end)
        } else {
            end
        };
        match shape {
            SelShape::Ellipse => select_mask::ellipse_path(start, end),
            _ => select_mask::rect_path(start, end),
        }
    }

    /// Combine a finished lasso outline into the selection. Fewer than three
    /// distinct points is no shape at all, and changes nothing.
    fn apply_lasso(&mut self, op: SelOp, lasso: TapLasso) {
        if let Some(pts) = lasso.finish() {
            let m = SelectionMask::from_path(&pts, self.selection_universe());
            self.apply_op(op, m);
        }
    }

    /// A lasso is being tapped out as a polygon.
    pub fn polygon_active(&self) -> bool {
        matches!(&self.sel_gesture, Some(SelGesture::Lasso { lasso, .. }) if lasso.polygon)
    }

    /// Close the polygon in progress and apply it.
    pub fn polygon_finish(&mut self) {
        if !self.polygon_active() {
            return;
        }
        if let Some(SelGesture::Lasso { op, lasso }) = self.sel_gesture.take() {
            self.apply_lasso(op, lasso);
        }
    }

    /// A double-click on the canvas: closes the polygon when both its presses
    /// landed on one spot (see [`TapLasso::doubled`]).
    pub fn polygon_double_click(&mut self) {
        let near = HANDLE_PX / self.view_scale.max(1e-6);
        if matches!(&self.sel_gesture, Some(SelGesture::Lasso { lasso, .. }) if lasso.polygon && lasso.doubled(near)) {
            self.polygon_finish();
        }
    }

    /// Take back the newest polygon corner, with any stretch drawn from it.
    /// `false` when no polygon is in progress, so the key can mean what it
    /// usually does.
    pub fn polygon_back(&mut self) -> bool {
        let Some(SelGesture::Lasso { lasso, .. }) = &mut self.sel_gesture else {
            return false;
        };
        if !lasso.polygon {
            return false;
        }
        if !lasso.back() {
            self.sel_gesture = None;
        }
        true
    }

    /// Abandon a selection gesture, or a lasso fill, in progress. `true` if
    /// there was one.
    pub fn cancel_gesture(&mut self) -> bool {
        // The Fill tool's lasso too: it ends the same ways.
        let fill = self.fill_lasso.take().is_some();
        self.wand_repick = None;
        // A magic select press puts back the selection it found.
        let wand = match self.wand_drag.take() {
            Some(d) => {
                self.sel_mask = d.before;
                self.mask_changed();
                true
            }
            None => false,
        };
        self.sel_gesture.take().is_some() || fill || wand
    }

    // --- Magic select ---

    /// A magic select press at `doc`: pick what joins the pixel there and show
    /// it at once, combined by `op`. The pen may stay down to retune it with
    /// [`AppState::wand_drag_to`]; [`AppState::wand_up`] lands it as one undo
    /// step.
    fn wand_down(&mut self, doc: (f32, f32), op: SelOp) {
        self.land_float();
        let any_paint = self.wand.any_paint;
        let (pick, at) = if self.wand.all_visible {
            // What the artist sees, in document space.
            let c = crate::io::composite::flatten_doc(&self.project, self.project.current_frame);
            let size = (c.width, c.height);
            let seed = (doc.0.floor() as i32, doc.1.floor() as i32);
            (WandPick::new(&c.pixels(), size, seed, any_paint), None)
        } else {
            let at = self.active_placement();
            let (cx, cy) = self.doc_to_active_cell(doc);
            let seed = (cx.floor() as i32, cy.floor() as i32);
            let size = (at.cw, at.ch);
            let cell = self.project.resolved_current().and_then(|id| self.project.cell(id));
            // Read in place: the press keeps only what it works out.
            let pick = match cell {
                Some(c) if (c.width, c.height) == size => {
                    WandPick::new(&c.pixels(), size, seed, any_paint)
                }
                // No drawing here yet: all bare canvas.
                _ => WandPick::new(&vec![0; at.cw as usize * at.ch as usize * 4], size, seed, any_paint),
            };
            (pick, Some(at))
        };
        let Some(pick) = pick else {
            // Off the pixels: a shape that covered nothing.
            self.apply_op(op, None);
            return;
        };
        let mut drag = WandDrag {
            pick,
            at,
            op,
            before: self.sel_mask.clone(),
            anchor: None,
            base_tolerance: self.wand.tolerance,
            base_gap: self.wand.gap,
        };
        self.wand_show(&mut drag);
        self.wand_drag = Some(drag);
    }

    /// Show what `drag` picks at the wand's settings now, combined into the
    /// selection it started from. Not recorded: [`AppState::wand_up`] does that
    /// once.
    fn wand_show(&mut self, drag: &mut WandDrag) {
        let p = drag.pick.pick(&self.wand);
        let shape = match &drag.at {
            // A whole-pixel placement moves the pick as it is, uncopied —
            // what `from_cell` does there, less the copies.
            Some(at) => match select_mask::integer_offset(&at.xf, at.cw, at.ch, at.pw, at.ph) {
                Some((ox, oy)) => SelectionMask {
                    x: p.x + ox,
                    y: p.y + oy,
                    w: p.w,
                    h: p.h,
                    cov: p.cov,
                    outside: 0,
                }
                .normalize(),
                None => SelectionMask::from_cell(
                    p.x, p.y, p.w, p.h, &p.cov, &at.xf, at.cw, at.ch, at.pw, at.ph,
                ),
            },
            None => SelectionMask {
                x: p.x,
                y: p.y,
                w: p.w,
                h: p.h,
                cov: p.cov,
                outside: 0,
            }
            .normalize(),
        };
        let next = combined(drag.op, drag.before.as_ref(), shape);
        if next != self.sel_mask {
            self.sel_mask = next;
            self.mask_changed();
        }
    }

    /// Remember where on screen a magic select press landed: its drag counts
    /// from here.
    pub fn wand_anchor(&mut self, at: egui::Pos2) {
        if let Some(d) = &mut self.wand_drag {
            d.anchor.get_or_insert(at);
        }
    }

    /// The pen, still down on a magic select press, is at screen point `at`.
    /// Sideways steps the tolerance and up and down the gap, each away from
    /// its value at the press, the way a Fill press steps its gap and expand;
    /// a change picks again so the outline follows. The new values stay in the
    /// sliders.
    pub fn wand_drag_to(&mut self, at: egui::Pos2) {
        use crate::input::button_drag::drag_steps;
        let Some(mut drag) = self.wand_drag.take() else {
            return;
        };
        if let Some(anchor) = drag.anchor {
            // Shift, pressed partway, keeps to one of the two.
            let at = self.lock_screen_drag(anchor, at);
            let tolerance = wand::step_tolerance(drag.base_tolerance, drag_steps(at.x - anchor.x));
            // Screen y grows downward, and up means more.
            let gap = fill::step_gap(drag.base_gap, drag_steps(anchor.y - at.y));
            if (tolerance, gap) != (self.wand.tolerance, self.wand.gap) {
                self.wand.tolerance = tolerance;
                self.wand.gap = gap;
                self.wand_show(&mut drag);
            }
        }
        self.wand_drag = Some(drag);
    }

    /// Whether a magic select press is still under the pen.
    pub fn wand_dragging(&self) -> bool {
        self.wand_drag.is_some()
    }

    /// Land a magic select press as one undo step, from the selection it found
    /// to the one it leaves. `true` if there was one.
    fn wand_up(&mut self) -> bool {
        let Some(drag) = self.wand_drag.take() else {
            return false;
        };
        if self.sel_mask != drag.before {
            self.history.push(undo::Command::Selection {
                before: drag.before.as_ref().map(SelectionMask::pack),
                after: self.sel_mask.as_ref().map(SelectionMask::pack),
            });
        }
        true
    }

    /// A press on the float's transform box, or on the selection itself when
    /// there is no float yet, which lifts one. `true` when the press was
    /// taken.
    /// What a plain press at `doc` would take hold of on the transform box,
    /// without taking it: on a float, any part of it; on a selection not yet
    /// lifted, only its handles and ring — inside it, a held selection mode
    /// still starts a new shape, which is how you add to it from within.
    pub fn transform_grab_at(&mut self, doc: (f32, f32)) -> Option<Grab> {
        if self.polygon_active() || self.sel_op != SelOp::Replace {
            return None;
        }
        let (cx, cy) = self.doc_to_active_cell(doc);
        let tol = HANDLE_PX / self.cell_view_scale();
        if let Some(sel) = &self.selection {
            return sel.grab_at(cx, cy, tol);
        }
        if !self.mask_is_grabbable() {
            return None;
        }
        let clip = self.active_cell_mask()?;
        selection::grab_at(&clip, &Pose::default(), cx, cy, tol).filter(|g| *g != Grab::Move)
    }

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

    /// The Fill tool's opacity, which Fill selection shares: the live brush
    /// while the bucket is the tool, its stored copy otherwise.
    pub fn fill_opacity_mut(&mut self) -> &mut f32 {
        if self.tool == ActiveTool::Fill {
            &mut self.brush.opacity
        } else {
            &mut self.tool_brushes[ActiveTool::Fill.idx()].opacity
        }
    }

    /// The Fill tool's paper grain for a fill on drawing `cell`, which Fill
    /// selection shares like the opacity. Boil per drawing gives each
    /// drawing its own patch of paper.
    pub fn fill_grain(&self, cell: CellId) -> Grain {
        let b = if self.tool == ActiveTool::Fill {
            &self.brush
        } else {
            &self.tool_brushes[ActiveTool::Fill.idx()]
        };
        let (s, c) = (fill::GRAIN_SIZES, fill::GRAIN_CONTRASTS);
        let grain = Grain {
            amount: b.fill_grain.clamp(0.0, 1.0),
            size: b.fill_grain_size.clamp(*s.start(), *s.end()),
            contrast: b.fill_grain_contrast.clamp(*c.start(), *c.end()),
            ..Grain::NONE
        };
        if b.fill_grain_boil {
            grain.boiled(cell)
        } else {
            grain
        }
    }

    /// Fill the selection with the brush colour at the Fill tool's opacity, on
    /// the active drawing.
    pub fn fill_selection(&mut self) {
        let opacity = *self.fill_opacity_mut();
        if self.sel_mask.is_none() || opacity <= 0.0 {
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
        let alpha_lock = self.active_alpha_lock();
        let grain = self.fill_grain(target);
        self.snapshot_pre(target);
        if let Some(c) = self.project.cell_mut(target) {
            c.dirty = None;
            fill::fill_masked(c, &clip, color, opacity, alpha_lock, false, grain);
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
                    self.pre_replaced(sel.cell);
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
            self.stash_brush();
            self.load_brush(t);
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
        // Lifting from a drawing mid-warp would lift the preview and leave
        // the warp's snapshot behind; the warp lands first.
        self.commit_liquify();
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

    /// Mirror or turn the selected pixels where they are, lifting them into a
    /// float first if they aren't one yet. The float stays up, so it can still
    /// be moved, and the whole thing lands — or Ctrl+Z puts it all back — as
    /// one step. `false` when there was nothing to turn.
    pub(super) fn orient_selection(&mut self, o: crate::doc::orient::Orient) -> bool {
        if !self.make_float() {
            return false;
        }
        self.touch_selection();
        let Some(sel) = self.selection.as_mut().filter(|s| s.lifted) else {
            return false;
        };
        sel.orient(o);
        self.sel_drag = None;
        self.sel_tex_stale = true;
        true
    }

    /// Settle what's in hand before an edit: a Liquify warp lands, then any
    /// floating pixels. Everything that must not edit around either calls
    /// this. Safe with nothing in hand.
    pub fn land_float(&mut self) {
        self.commit_liquify();
        self.land_float_only();
    }

    /// Put a float down: stamp its pixels into its cell and move the selection
    /// to where they landed. One undo step for all of it — the lift, the stamp
    /// and the selection — so one Ctrl+Z puts everything back where it was
    /// picked up. Safe to call with nothing floating.
    pub(super) fn land_float_only(&mut self) {
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
            self.pre_replaced(cell);
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
            self.snapshot_pre(cell);
            if let Some(c) = self.project.cell_mut(cell) {
                c.dirty = None;
                // Erased through the mask as cut, then turned the way the
                // float was — before its pose, which `pose_for` reads off the
                // turned box.
                other.lift_source(c);
                other.orient(sel.orient);
                other.pose = pose_for(sel, &pk, &other.mask);
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
                if c.byte_len() == self.float_pre.len() {
                    let stride = c.width as usize * 4;
                    for y in r.min_y as usize..r.max_y as usize {
                        let a = y * stride + r.min_x as usize * 4;
                        let b = y * stride + r.max_x as usize * 4;
                        c.pixels_mut()[a..b].copy_from_slice(&self.float_pre[a..b]);
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

    /// Nudge the float by whole pixels (arrow keys). Stays integral, so a
    /// move alone never reaches the resampler.
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
    /// Every drag is re-solved from the pose recorded at the press, so it
    /// depends only on where the pointer is now. A move goes in whole pixels,
    /// so dragging a selection around is still lossless, and Shift pressed
    /// partway holds it to the horizontal or vertical through the press.
    pub(super) fn drag_selection(&mut self, drag: SelDrag, x: f32, y: f32) {
        if let Grab::Move = drag.grab {
            let to = if self.axis_lock_on() {
                axis_lock::lock_hv(drag.start, (x, y), self.lock_basis_cell())
            } else {
                (x, y)
            };
            let offset = (
                drag.pose.offset.0 + (to.0 - drag.start.0).round(),
                drag.pose.offset.1 + (to.1 - drag.start.1).round(),
            );
            let Some(sel) = self.selection.as_mut() else {
                return;
            };
            if offset == sel.pose.offset {
                return;
            }
            sel.pose.offset = offset;
            self.touch_selection();
            return;
        }
        let mods = selection::DragMods {
            proportional: self.shift_held,
            centred: self.alt_held,
        };
        let Some(sel) = self.selection.as_mut() else {
            return;
        };
        let pose = drag
            .pose
            .dragged(&sel.mask, drag.grab, drag.start, (x, y), mods);
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
        // Unpacked first: the lift writes it straight after.
        if let Some(c) = self.project.cell_mut(cell) {
            self.float_pre.extend_from_slice(c.pixels_mut());
        }
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
        self.fill_lasso = None;
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
    use crate::doc::orient::Orient;
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
                c.pixels_mut()[i..i + 4].copy_from_slice(&rgba);
            }
        }
    }

    fn px(st: &AppState, x: u32, y: u32) -> [u8; 4] {
        let id = st.project.resolved_current().unwrap();
        let c = st.project.cell(id).unwrap();
        let i = ((y * c.width + x) * 4) as usize;
        [c.pixels()[i], c.pixels()[i + 1], c.pixels()[i + 2], c.pixels()[i + 3]]
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

    /// Shift pressed partway through a move holds the float to the line
    /// through the press, whichever way it has gone further; let go, it
    /// follows the pointer again. Every step stays in whole pixels.
    #[test]
    fn shift_partway_holds_a_move_to_an_axis() {
        let mut st = lasso();
        paint(&mut st, (100, 100, 140, 140), [200, 0, 0, 255]);
        drag(&mut st, (90.0, 90.0), (150.0, 150.0), None);
        st.select_down((120.0, 120.0), None);
        let offset = |st: &AppState| st.selection.as_ref().unwrap().pose.offset;
        st.select_move((130.4, 124.2));
        assert_eq!(offset(&st), (10.0, 4.0), "free");
        st.read_modifiers(egui::Modifiers::SHIFT);
        st.select_move((150.0, 127.0));
        assert_eq!(offset(&st), (30.0, 0.0), "held across");
        st.select_move((126.0, 170.0));
        assert_eq!(offset(&st), (0.0, 50.0), "held down");
        st.read_modifiers(egui::Modifiers::NONE);
        st.select_move((131.0, 170.0));
        assert_eq!(offset(&st), (11.0, 50.0), "free again");
        st.select_up();
        st.land_float();
        assert_eq!(px(&st, 125, 175), [200, 0, 0, 255]);
    }

    #[test]
    fn shift_partway_squares_a_rectangle_selection() {
        let mut st = lasso();
        st.select_down((100.0, 100.0), None);
        st.select_move((200.0, 140.0));
        st.read_modifiers(egui::Modifiers::SHIFT);
        st.select_move((200.0, 140.0));
        st.select_up();
        assert_eq!(at(&st, 150, 190), 255, "as tall as it is wide");
        assert_eq!(at(&st, 150, 210), 0);
    }

    /// A selection mode held as the drag starts has Shift in it: that Shift
    /// chose the mode, so the box isn't squared.
    #[test]
    fn a_held_mode_with_shift_in_it_doesnt_square() {
        let mut st = lasso();
        st.read_modifiers(egui::Modifiers::SHIFT | egui::Modifiers::CTRL);
        st.drag_began();
        drag(&mut st, (100.0, 100.0), (200.0, 140.0), Some(SelOp::Add));
        assert_eq!(at(&st, 150, 120), 255);
        assert_eq!(at(&st, 150, 170), 0, "not squared");
    }

    /// Once an edit lands, the stroke snapshot either is the cell it names,
    /// byte for byte, or names none: the next edit trusts it uncopied.
    fn assert_pre_mirrors(st: &AppState, what: &str) {
        let Some((id, rev)) = st.pre_mirror else {
            return;
        };
        let c = st.project.cell(id).unwrap();
        if c.rev() == rev {
            assert!(st.stroke_pre_pixels == *c.pixels(), "after {what}: the snapshot is cell {id}");
        }
    }

    fn stroke_with(st: &mut AppState, tool: ActiveTool, y: f32) {
        st.set_tool(tool);
        st.brush.radius = 4.0;
        st.pointer_down(st.make_sample(20.0, y, 0.0));
        for i in 1..=30 {
            st.pointer_move(st.make_sample(20.0 + i as f32 * 8.0, y, i as f32 * 0.01));
        }
        st.pointer_up();
    }

    #[test]
    fn the_stroke_snapshot_stays_the_cell_through_every_kind_of_edit() {
        let mut st = lasso();
        paint(&mut st, (100, 100, 140, 140), RED);
        let id = st.project.resolved_current().unwrap();
        let orig = st.project.cell(id).unwrap().pixels().into_owned();
        let steps = st.history.undo_len();

        stroke_with(&mut st, ActiveTool::Ink, 50.0);
        let rev = st.project.cell(id).unwrap().rev();
        assert_eq!(st.pre_mirror, Some((id, rev)), "a landed stroke leaves it mirrored");
        assert_pre_mirrors(&st, "a stroke");
        // The next one starts from the mirror, copying nothing.
        stroke_with(&mut st, ActiveTool::Ink, 70.0);
        assert_pre_mirrors(&st, "a stroke from the mirror");
        stroke_with(&mut st, ActiveTool::Eraser, 50.0);
        assert_pre_mirrors(&st, "an eraser stroke");
        st.set_tool(ActiveTool::Fill);
        st.pointer_down(st.make_sample(600.0, 400.0, 0.0));
        st.pointer_up();
        assert_pre_mirrors(&st, "a bucket fill");
        // An undo writes the cell, so the mirror no longer counts.
        st.undo();
        assert_ne!(st.pre_mirror.map(|m| m.1), Some(st.project.cell(id).unwrap().rev()));
        stroke_with(&mut st, ActiveTool::Ink, 90.0);
        assert_pre_mirrors(&st, "a stroke after an undo");

        st.set_tool(ActiveTool::Lasso);
        drag(&mut st, (90.0, 90.0), (150.0, 150.0), None);
        st.fill_selection();
        assert_pre_mirrors(&st, "a selection fill");
        drag(&mut st, (120.0, 120.0), (150.0, 120.0), None);
        st.land_float();
        assert_pre_mirrors(&st, "a lasso move");
        st.clear_selection_state();

        // Another drawing: the old one's ink has to leave the snapshot.
        let other = st.project.cells.len();
        st.project.cells.push(crate::doc::canvas::Canvas::new(st.project.width, st.project.height).into());
        st.project.layers[0].set_key(0, other);
        stroke_with(&mut st, ActiveTool::Ink, 300.0);
        assert_pre_mirrors(&st, "a stroke on another drawing");
        st.project.layers[0].set_key(0, id);
        stroke_with(&mut st, ActiveTool::Ink, 310.0);
        assert_pre_mirrors(&st, "a stroke back on the first");

        while st.history.undo_len() > steps {
            st.undo();
        }
        assert!(*st.project.cell(id).unwrap().pixels() == orig[..], "every edit undone exactly");
    }

    const RED: [u8; 4] = [200, 0, 0, 255];

    /// Pixel of layer 0's drawing on frame `f`, in that drawing's own pixels.
    fn px_on(st: &AppState, f: usize, x: u32, y: u32) -> [u8; 4] {
        let id = st.project.layers[0].resolve(f).expect("a drawing on this frame");
        let c = st.project.cell(id).unwrap();
        let i = ((y * c.width + x) * 4) as usize;
        [c.pixels()[i], c.pixels()[i + 1], c.pixels()[i + 2], c.pixels()[i + 3]]
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

    const BLUE: [u8; 4] = [0, 0, 255, 255];

    /// Red on the left, blue on the right, inside a selection around both:
    /// flipped, they trade sides about the selection's centre.
    fn two_stripes(st: &mut AppState) {
        paint(st, (100, 100, 110, 140), RED);
        paint(st, (130, 100, 140, 140), BLUE);
    }

    #[test]
    fn flipping_a_selection_lands_as_one_undo() {
        let mut st = lasso();
        two_stripes(&mut st);
        drag(&mut st, (95.0, 95.0), (145.0, 145.0), None);
        let steps = st.history.undo_len();
        st.orient_art(Orient::FLIP_H);
        assert!(st.selection.is_some(), "still floating, so it can be moved on");
        st.land_float();
        assert_eq!(px(&st, 105, 120), BLUE);
        assert_eq!(px(&st, 135, 120), RED);
        assert_eq!(px(&st, 60, 120)[3], 0, "nothing leaked outside");
        assert_eq!(st.history.undo_len(), steps + 1);
        st.undo();
        assert_eq!(px(&st, 105, 120), RED);
        assert_eq!(px(&st, 135, 120), BLUE);
    }

    /// Alt pressed and let go mid-drag switches the pivot back and forth, and
    /// every frame is solved from the press, so nothing creeps.
    #[test]
    fn alt_toggled_mid_drag_switches_pivot_without_drift() {
        let mut st = lasso();
        two_stripes(&mut st);
        drag(&mut st, (95.0, 95.0), (145.0, 145.0), None);
        let m = st.active_cell_mask().unwrap();
        let br = ((m.x + m.w) as f32, (m.y + m.h) as f32);
        let centre = ((m.x as f32 + br.0) * 0.5, (m.y as f32 + br.1) * 0.5);
        let to = (br.0 + 10.0, br.1 + 10.0);
        st.select_down(br, None);
        let mid = |st: &AppState| {
            let s = st.selection.as_ref().unwrap();
            s.buf_to_cell(s.mask.w as f32 * 0.5, s.mask.h as f32 * 0.5)
        };
        st.alt_held = true;
        st.select_move(to);
        let centred = st.selection.as_ref().unwrap().pose;
        let c = mid(&st);
        assert!((c.0 - centre.0).abs() < 1e-3 && (c.1 - centre.1).abs() < 1e-3, "{c:?}");
        st.alt_held = false;
        st.select_move(to);
        let c = mid(&st);
        assert!(c.0 > centre.0 + 1.0, "pinned at the far corner, the middle moves: {c:?}");
        st.alt_held = true;
        st.select_move(to);
        assert_eq!(st.selection.as_ref().unwrap().pose, centred, "no drift");
        st.select_up();
    }

    #[test]
    fn undo_while_floating_puts_the_flip_back() {
        let mut st = lasso();
        two_stripes(&mut st);
        drag(&mut st, (95.0, 95.0), (145.0, 145.0), None);
        let steps = st.history.undo_len();
        st.orient_art(Orient::CW);
        st.orient_art(Orient::FLIP_V);
        st.undo();
        assert!(st.selection.is_none());
        assert_eq!(px(&st, 105, 120), RED);
        assert_eq!(px(&st, 135, 120), BLUE);
        assert_eq!(st.history.undo_len(), steps, "nothing recorded");
    }

    #[test]
    fn all_frames_lands_the_flip_on_every_drawing() {
        let mut st = animated();
        for (cell, _) in st.layer_drawings(0) {
            let c = st.project.cell_mut(cell).unwrap();
            for y in 100..140u32 {
                for x in 130..140u32 {
                    let i = ((y * c.width + x) * 4) as usize;
                    c.pixels_mut()[i..i + 4].copy_from_slice(&BLUE);
                }
            }
        }
        drag(&mut st, (95.0, 95.0), (145.0, 145.0), None);
        st.sel_all_frames = true;
        let steps = st.history.undo_len();
        st.orient_art(Orient::FLIP_H);
        st.land_float();
        for f in 0..4 {
            assert_eq!(px_on(&st, f, 104, 120), BLUE, "frame {f}");
            assert_eq!(px_on(&st, f, 120, 120), RED, "frame {f}");
            assert_eq!(px_on(&st, f, 136, 120), RED, "frame {f}");
        }
        assert_eq!(st.history.undo_len(), steps + 1);
        st.undo();
        for f in 0..4 {
            assert_eq!(px_on(&st, f, 136, 120), BLUE, "frame {f} back");
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
            st.project.cell(id).unwrap().pixels().into_owned()
        };
        let first = cell(0);
        assert_ne!(first, {
            let mut before = lasso();
            paint(&mut before, (100, 100, 110, 140), RED);
            paint(&mut before, (100, 130, 140, 140), RED);
            before.project.cells[0].pixels().into_owned()
        }, "it did turn");
        for f in 1..3 {
            assert!(cell(f) == first, "frame {f} matches frame 0");
        }
    }

    #[test]
    fn undo_while_floating_puts_the_pixels_back_untouched() {
        let mut st = lasso();
        paint(&mut st, (100, 100, 140, 140), [10, 200, 30, 255]);
        let before = st.project.cells[0].pixels().into_owned();
        drag(&mut st, (90.0, 90.0), (150.0, 150.0), None);
        let mask = st.sel_mask.clone();
        let steps = st.history.undo_len();
        drag(&mut st, (120.0, 120.0), (170.0, 140.0), None);
        st.undo();
        assert!(st.selection.is_none());
        assert_eq!(st.project.cells[0].pixels(), before);
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
        assert_eq!(st.project.cells[0].pixels()[((120 * 1280 + 170) * 4 + 2) as usize], 255);
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

    /// On the lasso a tap starts a polygon, a corner per tap after.
    #[test]
    fn a_polygon_takes_a_corner_per_click() {
        let mut st = lasso();
        st.sel_shape = SelShape::Freehand;
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

    /// A drag on the lasso is freehand; between tapped corners a drag adds a
    /// freehand stretch, and the shape keeps going until it is closed.
    #[test]
    fn the_lasso_draws_freehand_and_mixes_stretches_into_a_polygon() {
        let mut st = lasso();
        st.sel_shape = SelShape::Freehand;
        drag(&mut st, (100.0, 100.0), (300.0, 300.0), None);
        assert!(st.sel_mask.is_none(), "a straight drag is no shape");
        st.select_down((100.0, 100.0), None);
        for p in [(300.0, 100.0), (300.0, 300.0), (100.0, 300.0)] {
            st.select_move(p);
        }
        st.select_up();
        assert!(!st.polygon_active(), "a drag lands as it lets go");
        assert_eq!(at(&st, 200, 200), 255);

        // Taps, with a stretch drawn from the second corner round a bulge.
        st.deselect();
        st.select_down((400.0, 100.0), None);
        st.select_up();
        st.select_down((500.0, 100.0), None);
        for p in [(560.0, 150.0), (500.0, 200.0)] {
            st.select_move(p);
        }
        st.select_up();
        assert!(st.polygon_active(), "still open after the stretch");
        st.select_down((400.0, 200.0), None);
        st.select_up();
        st.select_down((400.0, 100.0), None);
        st.select_up();
        assert!(!st.polygon_active(), "closed on its first corner");
        assert_eq!(at(&st, 540, 150), 255, "inside the bulge");
        assert_eq!(at(&st, 450, 150), 255);
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
    fn fill_selection_paints_at_the_fill_tools_opacity() {
        let mut st = lasso();
        paint(&mut st, (0, 0, 300, 300), [0, 0, 255, 255]);
        drag(&mut st, (100.0, 100.0), (200.0, 200.0), None);
        // Set from the Lasso panel: it lands on the Fill tool's own copy.
        *st.fill_opacity_mut() = 0.5;
        assert_eq!(st.brush.opacity, 1.0, "the Lasso's own brush is untouched");
        st.brush.color = [255, 0, 0, 255];
        st.fill_selection();
        assert_eq!(px(&st, 150, 150), [128, 0, 128, 255], "half over the blue");
        assert_eq!(px(&st, 90, 150), [0, 0, 255, 255]);
        // The bucket shows the same value, and a change there reaches back.
        st.set_tool(ActiveTool::Fill);
        assert_eq!(st.brush.opacity, 0.5);
        st.brush.opacity = 0.0;
        st.set_tool(ActiveTool::Lasso);
        assert_eq!(*st.fill_opacity_mut(), 0.0);
        let steps = st.history.undo_len();
        st.fill_selection();
        assert_eq!(st.history.undo_len(), steps, "nothing to fill at zero");
    }

    #[test]
    fn fill_selection_takes_the_fill_tools_grain() {
        let mut st = lasso();
        drag(&mut st, (100.0, 100.0), (200.0, 200.0), None);
        st.tool_brushes[ActiveTool::Fill.idx()].fill_grain = 1.0;
        st.brush.color = [255, 0, 0, 255];
        st.fill_selection();
        let row: Vec<[u8; 4]> = (110..190).map(|x| px(&st, x, 150)).collect();
        assert!(row.iter().any(|p| p[3] < 60) && row.iter().any(|p| p[3] > 180), "specks: {row:?}");
        assert!(row.iter().all(|p| p[..3] == [255, 0, 0]));
        assert_eq!(px(&st, 90, 150)[3], 0, "still held to the selection");

        // Boil: a patch of paper per drawing, the same one every time.
        assert_eq!(st.fill_grain(0).offset, (0, 0));
        st.tool_brushes[ActiveTool::Fill.idx()].fill_grain_boil = true;
        assert_ne!(st.fill_grain(0).offset, st.fill_grain(1).offset);
        assert_eq!(st.fill_grain(1), st.fill_grain(1));
        // The bucket shows the same settings.
        st.set_tool(ActiveTool::Fill);
        assert_eq!((st.brush.fill_grain, st.brush.fill_grain_boil), (1.0, true));
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

    // --- Magic select ---

    /// A fresh state with the Lasso up as the magic select.
    fn wand() -> AppState {
        let mut st = lasso();
        st.sel_shape = SelShape::Wand;
        st
    }

    /// A one-pixel square outline from `(a, a)` to `(b, b)` on the active cell.
    fn ring(st: &mut AppState, a: u32, b: u32, rgba: [u8; 4]) {
        paint(st, (a, a, b + 1, a + 1), rgba);
        paint(st, (a, b, b + 1, b + 1), rgba);
        paint(st, (a, a, a + 1, b + 1), rgba);
        paint(st, (b, a, b + 1, b + 1), rgba);
    }

    fn click(st: &mut AppState, doc: (f32, f32), held: Option<SelOp>) {
        st.select_down(doc, held);
        st.select_up();
    }

    fn area(st: &AppState) -> usize {
        st.sel_mask.as_ref().map_or(0, |m| m.cov.iter().filter(|&&c| c >= 128).count())
    }

    #[test]
    fn a_wand_click_picks_what_joins_it_as_one_undo_step() {
        let mut st = wand();
        ring(&mut st, 100, 140, RED);
        let steps = st.history.undo_len();
        click(&mut st, (120.5, 120.5), None);
        assert_eq!(area(&st), 39 * 39, "the inside of the ring");
        assert_eq!(at(&st, 101, 101), 255);
        assert_eq!(at(&st, 100, 120), 0, "not the line");
        assert_eq!(st.history.undo_len(), steps + 1);
        assert!(!st.wand_dragging());
        st.undo();
        assert!(st.sel_mask.is_none());
    }

    #[test]
    fn held_modes_combine_wand_picks() {
        let mut st = wand();
        ring(&mut st, 100, 140, RED);
        paint(&mut st, (200, 200, 220, 220), RED);
        click(&mut st, (120.5, 120.5), None);
        click(&mut st, (210.5, 210.5), Some(SelOp::Add));
        assert_eq!(at(&st, 120, 120), 255);
        assert_eq!(at(&st, 210, 210), 255);
        click(&mut st, (120.5, 120.5), Some(SelOp::Subtract));
        assert_eq!(at(&st, 120, 120), 0);
        assert_eq!(at(&st, 210, 210), 255);
    }

    #[test]
    fn any_paint_takes_the_lines_and_the_colour_inside_together() {
        let mut st = wand();
        ring(&mut st, 100, 140, [10, 10, 10, 255]);
        paint(&mut st, (101, 101, 140, 140), RED);
        st.wand.any_paint = true;
        click(&mut st, (120.5, 120.5), None);
        assert_eq!(area(&st), 41 * 41);
        assert_eq!(at(&st, 100, 100), 255);
    }

    #[test]
    fn a_click_inside_the_selection_picks_again_and_a_drag_moves_it() {
        let mut st = wand();
        paint(&mut st, (100, 100, 140, 140), RED);
        click(&mut st, (120.5, 120.5), None);
        assert_eq!(area(&st), 40 * 40);
        // A click back on it picks afresh there, at the settings now.
        st.wand.expand = 2;
        let steps = st.history.undo_len();
        click(&mut st, (120.5, 120.5), None);
        assert!(st.selection.is_none(), "nothing lifted");
        assert_eq!(area(&st), 44 * 44);
        assert_eq!(st.history.undo_len(), steps + 1);
        // A drag from it moves the pixels.
        drag(&mut st, (120.5, 120.5), (150.5, 120.5), None);
        assert!(st.selection.is_some(), "floating");
        st.land_float();
        assert_eq!(px(&st, 165, 120), RED);
        assert_eq!(px(&st, 105, 120)[3], 0);
    }

    #[test]
    fn dragging_a_wand_press_retunes_it_live_and_lands_once() {
        let mut st = wand();
        paint(&mut st, (100, 100, 120, 140), [100, 100, 100, 255]);
        paint(&mut st, (120, 100, 140, 140), [130, 100, 100, 255]);
        st.wand.tolerance = 8;
        let steps = st.history.undo_len();
        st.select_down((110.5, 120.5), None);
        let at0 = egui::pos2(400.0, 300.0);
        st.wand_anchor(at0);
        assert_eq!(area(&st), 20 * 40, "shown at the press");
        // Sideways: tolerance climbs its ladder, and the pick follows.
        st.wand_drag_to(at0 + egui::vec2(200.0, 0.0));
        assert!(st.wand.tolerance >= 30, "tolerance {}", st.wand.tolerance);
        assert_eq!(area(&st), 40 * 40);
        // Up: the gap climbs.
        st.wand_drag_to(at0 + egui::vec2(200.0, -40.0));
        assert!(st.wand.gap > 0);
        st.select_up();
        assert_eq!(st.history.undo_len(), steps + 1, "one step for the whole press");
        st.undo();
        assert!(st.sel_mask.is_none());
    }

    #[test]
    fn esc_mid_press_puts_the_old_selection_back() {
        let mut st = wand();
        ring(&mut st, 100, 140, RED);
        click(&mut st, (120.5, 120.5), None);
        let kept = st.sel_mask.clone();
        let steps = st.history.undo_len();
        st.select_down((10.5, 10.5), None);
        assert_ne!(st.sel_mask, kept, "the new pick shows");
        assert!(st.cancel_gesture());
        assert_eq!(st.sel_mask, kept);
        st.select_up();
        assert_eq!(st.history.undo_len(), steps, "nothing recorded");
    }

    #[test]
    fn a_wand_click_off_the_drawing_picks_nothing() {
        let mut st = wand();
        click(&mut st, (10.5, 10.5), None);
        assert!(st.sel_mask.is_some(), "a blank drawing is all bare canvas");
        click(&mut st, (-500.0, 300.0), Some(SelOp::Add));
        assert!(st.sel_mask.is_some(), "Add with nothing keeps what was there");
        click(&mut st, (-500.0, 300.0), None);
        assert!(st.sel_mask.is_none(), "Replace with nothing selects nothing");
    }

    #[test]
    fn a_wand_pick_lands_where_a_moved_layer_shows_it() {
        let mut st = wand();
        ring(&mut st, 100, 140, RED);
        st.project.layers[0].transform.tx = 10.0;
        // Cell u is document u - 10.
        click(&mut st, (130.5, 120.5), None);
        assert_eq!(at(&st, 111, 120), 255, "cell 101");
        assert_eq!(at(&st, 110, 120), 0, "cell 100, the line");
        assert_eq!(at(&st, 149, 120), 255, "cell 139");
        assert_eq!(at(&st, 150, 120), 0, "cell 140, the line");
    }

    #[test]
    fn all_visible_picks_by_the_lines_on_another_layer() {
        let mut st = wand();
        ring(&mut st, 100, 140, RED);
        st.dispatch(Action::LayerAdd);
        assert_eq!(st.project.current_layer, 1, "drawing on a new, empty layer");
        click(&mut st, (120.5, 120.5), None);
        assert!(at(&st, 30, 30) == 255, "on its own the empty layer is all bare");
        st.wand.all_visible = true;
        click(&mut st, (120.5, 120.5), None);
        assert_eq!(area(&st), 39 * 39);
        assert_eq!(at(&st, 30, 30), 0);
    }

    /// The canvas hands a press and its drag to the wand in screen points.
    #[test]
    fn the_canvas_drags_a_wand_press_in_screen_points() {
        let mut st = wand();
        st.show_panels = false;
        st.show_mini_timeline = false;
        st.wand.tolerance = 8;
        st.wand.gap = 0;
        let steps = st.history.undo_len();
        let ctx = egui::Context::default();
        let run = |st: &mut AppState, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                let tab_focus = st.handle_keys(ctx);
                crate::ui::shell::draw(st, ctx);
                st.drop_stray_focus(ctx, tab_focus);
            });
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let (from, to) = (egui::pos2(500.0, 300.0), egui::pos2(700.0, 260.0));
        run(&mut st, vec![egui::Event::PointerMoved(from)]);
        run(&mut st, vec![egui::Event::PointerMoved(from), button(from, true)]);
        for i in 1..=8 {
            let p = from + (to - from) * (i as f32 / 8.0);
            run(&mut st, vec![egui::Event::PointerMoved(p)]);
        }
        assert!(st.wand_dragging(), "still under the pen");
        run(&mut st, vec![button(to, false)]);
        run(&mut st, vec![]);
        assert!(!st.wand_dragging());
        assert!(st.wand.tolerance > 8, "sideways retuned it: {}", st.wand.tolerance);
        assert!(st.wand.gap > 0, "up retuned the gap");
        assert!(st.sel_mask.is_some());
        assert_eq!(st.history.undo_len(), steps + 1);
    }

    /// What a magic select press costs, end to end, on a 1080p and an 8K
    /// frame: a click on the bare background, one inside a ring, and drag
    /// steps, each with the frame after it. `cargo test --release wand_timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn wand_timing() {
        use std::time::Instant;
        for (w, h) in [(1920u32, 1080u32), (7680, 4320)] {
            let mut st = wand();
            st.load_project(crate::doc::project::Project::new(w, h, 24.0), None);
            st.sel_shape = SelShape::Wand;
            let (r0, r1) = (h / 4, h / 2);
            ring(&mut st, r0, r1, RED);
            let mid = ((r0 + r1) as f32 * 0.5, (r0 + r1) as f32 * 0.5);
            // Each step with the next frame's upkeep, which rebuilds the
            // active cell's view of a changed selection.
            let time = |label: &str, st: &mut AppState, f: &mut dyn FnMut(&mut AppState)| {
                let t = Instant::now();
                f(st);
                st.sync_selection();
                println!("{w}x{h} {label}: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
            };
            time("background click", &mut st, &mut |st| click(st, (5.5, 5.5), None));
            time("ring click", &mut st, &mut |st| click(st, mid, Some(SelOp::Add)));
            st.deselect();
            let at0 = egui::pos2(400.0, 300.0);
            time("background press", &mut st, &mut |st| {
                st.select_down((5.5, 5.5), None);
                st.wand_anchor(at0);
            });
            for i in 1..=3 {
                time("  tolerance step", &mut st, &mut |st| {
                    st.wand_drag_to(at0 + egui::vec2(8.0 + 12.0 * i as f32, 0.0))
                });
            }
            time("  gap step (builds the field)", &mut st, &mut |st| {
                st.wand_drag_to(at0 + egui::vec2(44.0, -20.0))
            });
            time("  gap step", &mut st, &mut |st| st.wand_drag_to(at0 + egui::vec2(44.0, -32.0)));
            time("release", &mut st, &mut |st| st.select_up());
            st.wand.all_visible = true;
            time("all visible click", &mut st, &mut |st| click(st, mid, None));
        }
    }
}
