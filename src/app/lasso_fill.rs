//! The Fill tool's lasso: drag an outline, or tap one out corner by corner,
//! and what it closes round fills with the brush colour.
//!
//! One gesture serves both. A press that drags draws freehand, and letting
//! go fills. A press that doesn't move starts a polygon instead: each tap
//! adds a corner, and a tap back on the first one, a double-click or Enter
//! closes it and fills. Held combos change what lands (see [`LassoHeld`]).
//!
//! The outline is kept in document space, like the selection's, and only
//! goes into a drawing's pixels when it lands: the drawing it fills is the
//! one active then.
//!
//! A child of `app` for the same reason as `select`.

use super::AppState;
use crate::tools::lasso::{self, Mask};
use crate::tools::selection::HANDLE_PX;

/// What the Fill tool does with a press: flood from it, or draw a lasso.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillMode {
    #[default]
    Bucket,
    Lasso,
}

/// What the combos held at a lasso fill's last press ask for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LassoHeld {
    /// Clear the shape instead of filling it.
    pub erase: bool,
    /// The other way from the Enclose & fill toggle, for this shape.
    pub enclose: bool,
    /// Polygon edges turn in 15° steps.
    pub snap: bool,
}

/// A lasso fill being drawn, in document space.
#[derive(Clone, Debug)]
pub struct FillLasso {
    /// Freehand samples, or the polygon's corners.
    pub pts: Vec<(f32, f32)>,
    /// Tapped out corner by corner.
    pub polygon: bool,
    /// The pen is down: the freehand line grows, or the newest corner
    /// follows it.
    pub pressed: bool,
    /// Furthest the pen got from where it went down, in document pixels:
    /// tells a tap from a drag.
    max_dist: f32,
    held: LassoHeld,
}

/// A press that travels less than this many screen pixels is a tap.
const TAP_PX: f32 = 3.0;

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// `to`, turned about `from` to the nearest 15° — the same distance out.
pub fn snap_15(from: (f32, f32), to: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let r = dx.hypot(dy);
    if r <= 0.0 {
        return to;
    }
    let step = 15f32.to_radians();
    let a = (dy.atan2(dx) / step).round() * step;
    (from.0 + r * a.cos(), from.1 + r * a.sin())
}

impl AppState {
    /// Swap the Fill tool between the bucket and the lasso.
    pub fn toggle_fill_mode(&mut self) {
        self.set_fill_mode(match self.fill_mode {
            FillMode::Bucket => FillMode::Lasso,
            FillMode::Lasso => FillMode::Bucket,
        });
    }

    /// Put the Fill tool in mode `m`, dropping a lasso half drawn.
    pub fn set_fill_mode(&mut self, m: FillMode) {
        if m != self.fill_mode {
            self.fill_lasso = None;
            self.fill_mode = m;
        }
    }

    /// A polygon fill is being tapped out: Enter, Backspace and a
    /// double-click are its until it closes.
    pub fn fill_polygon_active(&self) -> bool {
        self.fill_lasso.as_ref().is_some_and(|l| l.polygon)
    }

    /// The lasso fill in progress, for drawing it.
    pub fn fill_lasso(&self) -> Option<&FillLasso> {
        self.fill_lasso.as_ref()
    }

    /// Where the next polygon corner goes with the pointer at `doc`: there,
    /// or turned to a 15° step from the last corner while `snap` is held.
    pub fn fill_corner_at(&self, doc: (f32, f32), snap: bool) -> (f32, f32) {
        let Some(l) = &self.fill_lasso else { return doc };
        // While a corner is held down it is the last one; it snaps to the one
        // before.
        let n = l.pts.len() - usize::from(l.pressed);
        match (snap, n.checked_sub(1).and_then(|i| l.pts.get(i))) {
            (true, Some(&from)) => snap_15(from, doc),
            _ => doc,
        }
    }

    /// Lasso fill press at document point `doc`.
    pub fn fill_lasso_down(&mut self, doc: (f32, f32), held: LassoHeld) {
        self.playback.stop();
        if self.fill_polygon_active() {
            let at = self.fill_corner_at(doc, held.snap);
            let close = HANDLE_PX * 1.5 / self.view_scale.max(1e-6);
            let Some(l) = &mut self.fill_lasso else { return };
            l.held = held;
            if l.pts.len() >= 3 && dist(l.pts[0], doc) <= close {
                self.fill_lasso_finish();
            } else {
                l.pts.push(at);
                l.pressed = true;
            }
            return;
        }
        self.fill_lasso = Some(FillLasso {
            pts: vec![doc],
            polygon: false,
            pressed: true,
            max_dist: 0.0,
            held,
        });
    }

    /// Lasso fill drag to document point `doc`.
    pub fn fill_lasso_move(&mut self, doc: (f32, f32), held: LassoHeld) {
        let at = self.fill_corner_at(doc, held.snap);
        // Finer than a document pixel zoomed in, as for the selection.
        let step = (1.0 / self.view_scale.max(1e-6)).min(1.0);
        let Some(l) = &mut self.fill_lasso else { return };
        if !l.pressed {
            return;
        }
        if l.polygon {
            if let Some(p) = l.pts.last_mut() {
                *p = at;
            }
            return;
        }
        l.max_dist = l.max_dist.max(dist(l.pts[0], doc));
        if l.pts.last().map_or(true, |&p| dist(p, doc) >= step) {
            l.pts.push(doc);
        }
    }

    /// Lasso fill release: a freehand outline lands; a tap starts a polygon,
    /// or puts down its corner.
    pub fn fill_lasso_up(&mut self) {
        let tap = TAP_PX / self.view_scale.max(1e-6);
        let Some(l) = &mut self.fill_lasso else { return };
        l.pressed = false;
        if l.polygon {
            return;
        }
        if l.max_dist < tap {
            l.polygon = true;
            l.pts.truncate(1);
            return;
        }
        let Some(l) = self.fill_lasso.take() else { return };
        self.land_lasso_fill(l.pts, l.held);
    }

    /// Close the polygon in progress and fill it. Fewer than three distinct
    /// corners is no shape, and just ends it.
    pub fn fill_lasso_finish(&mut self) {
        if !self.fill_polygon_active() {
            return;
        }
        let Some(l) = self.fill_lasso.take() else { return };
        self.land_lasso_fill(l.pts, l.held);
    }

    /// A double-click on the canvas: closes the polygon when both its presses
    /// landed on one spot, its second corner on top of the first. egui counts
    /// any two quick clicks, so corners tapped out fast would otherwise close
    /// it early.
    pub fn fill_lasso_double_click(&mut self) {
        let near = HANDLE_PX / self.view_scale.max(1e-6);
        let doubled = self
            .fill_lasso
            .as_ref()
            .filter(|l| l.polygon && l.pts.len() >= 2)
            .is_some_and(|l| dist(l.pts[l.pts.len() - 1], l.pts[l.pts.len() - 2]) <= near);
        if doubled {
            self.fill_lasso_finish();
        }
    }

    /// Take back the newest polygon corner. `false` when no polygon is being
    /// tapped out, so the key can mean what it usually does.
    pub fn fill_lasso_back(&mut self) -> bool {
        let Some(l) = self.fill_lasso.as_mut().filter(|l| l.polygon) else {
            return false;
        };
        l.pts.pop();
        if l.pts.is_empty() {
            self.fill_lasso = None;
        }
        true
    }

    /// Fill (or with `held.erase` clear) the closed outline `pts` on the
    /// active drawing: inside the selection when there is one, at the Fill
    /// tool's opacity, behind the paint with Behind on, or only the areas
    /// its lines shut in with Enclose & fill. One undo step.
    fn land_lasso_fill(&mut self, mut pts: Vec<(f32, f32)>, held: LassoHeld) {
        use crate::tools::fill;
        // A double-click lands its second press on top of the first.
        pts.dedup_by(|a, b| dist(*a, *b) < 1e-3);
        if pts.len() < 3 {
            return;
        }
        let opacity = *self.fill_opacity_mut();
        if !held.erase && opacity <= 0.0 {
            return;
        }
        self.land_float();
        let Some(target) = self.begin_edit_cell() else {
            return;
        };
        let (cw, ch) = {
            let c = &self.project.cells[target];
            (c.width, c.height)
        };
        let cell: Vec<(f32, f32)> = pts.iter().map(|&p| self.doc_to_active_cell(p)).collect();
        let Some((x, y, w, h, cov)) = lasso::coverage_rect(&cell, (0, 0, cw as i32, ch as i32)) else {
            return;
        };
        let shape = Mask {
            x: x as u32,
            y: y as u32,
            w,
            h,
            cov,
        };
        let clip = self.active_cell_clip();
        // Held to the selection; nothing at all outside it.
        let held_shape = match &clip {
            Some(c) => match shape.intersect(c) {
                Some(m) => m,
                None => return,
            },
            None => shape.clone(),
        };
        let enclose = !held.erase && (self.fill_enclose != held.enclose);
        let lines = if enclose {
            self.fill_source(self.project.current_layer, cw, ch).map(|c| c.into_pixels())
        } else {
            None
        };
        let opts = fill::FillOptions {
            tolerance: self.brush.fill_tolerance,
            color: self.brush.color,
            expand: self.brush.fill_expand,
            gap: self.brush.fill_gap,
            alpha_lock: self.active_alpha_lock(),
            opacity,
        };
        let behind = self.fill_behind;
        self.snapshot_pre(target);
        if let Some(c) = self.project.cell_mut(target) {
            c.dirty = None;
            if held.erase {
                lasso::erase_masked(c, &held_shape);
            } else if enclose {
                fill::enclose_fill(c, &self.stroke_pre_pixels, lines.as_deref(), &shape, &opts, behind, clip.as_deref());
            } else {
                fill::fill_masked(c, &held_shape, opts.color, opacity, opts.alpha_lock, behind);
            }
        }
        if self.project.cell(target).is_some_and(|c| c.dirty.is_some()) {
            self.mark_dirty(target);
        }
        if self.commit_undo(target) && !held.erase {
            self.painted(target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::select_mask::SelectionMask;
    use crate::tools::ActiveTool;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    /// The Fill tool's lasso, red, on a fresh project.
    fn state() -> AppState {
        let mut st = AppState::for_test();
        st.set_tool(ActiveTool::Fill);
        st.set_fill_mode(FillMode::Lasso);
        st.brush.color = RED;
        st.brush.opacity = 1.0;
        st
    }

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
        let p = c.pixels();
        [p[i], p[i + 1], p[i + 2], p[i + 3]]
    }

    /// A freehand square from (x0, y0) to (x1, y1), drawn and let go.
    fn drag_square(st: &mut AppState, (x0, y0, x1, y1): (f32, f32, f32, f32), held: LassoHeld) {
        st.fill_lasso_down((x0, y0), held);
        for p in [(x1, y0), (x1, y1), (x0, y1)] {
            st.fill_lasso_move(p, held);
        }
        st.fill_lasso_up();
    }

    #[test]
    fn a_held_erase_clears_the_shape() {
        let mut st = state();
        paint(&mut st, (0, 0, 200, 200), BLACK);
        let held = LassoHeld { erase: true, ..Default::default() };
        drag_square(&mut st, (50.0, 50.0, 150.0, 150.0), held);
        assert_eq!(px(&st, 100, 100)[3], 0);
        assert_eq!(px(&st, 20, 20), BLACK);
        st.undo();
        assert_eq!(px(&st, 100, 100), BLACK, "one undo step");
    }

    #[test]
    fn the_selection_holds_the_fill() {
        let mut st = state();
        let left = [(0.0, 0.0), (100.0, 0.0), (100.0, 400.0), (0.0, 400.0)];
        let m = SelectionMask::from_path(&left, st.selection_universe());
        st.set_mask(m, false);
        drag_square(&mut st, (50.0, 50.0, 150.0, 150.0), LassoHeld::default());
        assert_eq!(px(&st, 75, 100), RED);
        assert_eq!(px(&st, 125, 100)[3], 0, "outside the selection");
    }

    #[test]
    fn behind_fills_round_the_lines_and_held_enclose_fills_only_inside_them() {
        let mut st = state();
        // A closed black box, 60..=140.
        for i in 60..=140 {
            paint(&mut st, (i, 60, i + 1, 61), BLACK);
            paint(&mut st, (i, 140, i + 1, 141), BLACK);
            paint(&mut st, (60, i, 61, i + 1), BLACK);
            paint(&mut st, (140, i, 141, i + 1), BLACK);
        }
        st.fill_behind = true;
        drag_square(&mut st, (20.0, 20.0, 180.0, 180.0), LassoHeld { enclose: true, ..Default::default() });
        assert_eq!(px(&st, 100, 100), RED, "inside the box");
        assert_eq!(px(&st, 30, 30)[3], 0, "round it stays bare");
        assert_eq!(px(&st, 60, 100), BLACK, "the line, behind");

        // The toggle on, the same held combo does the other: plain fill.
        st.undo();
        st.fill_enclose = true;
        drag_square(&mut st, (20.0, 20.0, 180.0, 180.0), LassoHeld { enclose: true, ..Default::default() });
        assert_eq!(px(&st, 30, 30), RED);
        assert_eq!(px(&st, 60, 100), BLACK, "still behind the line");
    }

    #[test]
    fn backspace_and_undo_take_back_corners_and_esc_drops_the_polygon() {
        let mut st = state();
        for p in [(10.0, 10.0), (100.0, 10.0), (100.0, 100.0)] {
            st.fill_lasso_down(p, LassoHeld::default());
            st.fill_lasso_up();
        }
        assert_eq!(st.fill_lasso().map(|l| l.pts.len()), Some(3));
        st.undo();
        assert_eq!(st.fill_lasso().map(|l| l.pts.len()), Some(2), "undo took a corner, not history");
        assert!(st.fill_lasso_back());
        assert!(st.cancel_gesture(), "Esc's path drops it");
        assert!(st.fill_lasso().is_none());
        assert!(!st.fill_lasso_back());
    }

    #[test]
    fn a_held_snap_turns_polygon_edges_to_15_degrees() {
        let (x, y) = snap_15((0.0, 0.0), (100.0, 3.0));
        assert!((x - 100.045).abs() < 0.01 && y.abs() < 1e-3, "{x} {y}");
        let (x, y) = snap_15((0.0, 0.0), (50.0, 48.0));
        assert!((x - y).abs() < 1e-3, "45°: {x} {y}");

        let mut st = state();
        st.fill_lasso_down((0.0, 0.0), LassoHeld::default());
        st.fill_lasso_up();
        let snap = LassoHeld { snap: true, ..Default::default() };
        st.fill_lasso_down((100.0, 4.0), snap);
        let corner = st.fill_lasso().unwrap().pts[1];
        assert!(corner.1.abs() < 1e-3, "flat: {corner:?}");
    }

    #[test]
    fn switching_mode_drops_a_lasso_half_drawn() {
        let mut st = state();
        st.fill_lasso_down((10.0, 10.0), LassoHeld::default());
        st.fill_lasso_up();
        assert!(st.fill_polygon_active());
        st.toggle_fill_mode();
        assert_eq!(st.fill_mode, FillMode::Bucket);
        assert!(st.fill_lasso().is_none());
    }
}
