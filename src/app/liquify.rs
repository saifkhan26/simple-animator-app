//! The Liquify tool's session: a warp of the current drawing, painted with
//! the pen and kept live until it is applied or cancelled.
//!
//! The drawing as the session found it is the stroke snapshot
//! (`stroke_pre_pixels`), held for the whole session; every frame the area
//! the brush reached is resampled from it through the warp field into the
//! cell itself, so what's on screen is the result and every texture that
//! follows the cell (clip cuts, onion) follows the warp too. See
//! [`crate::tools::liquify`] for the field.
//!
//! The session ends — the warp lands as one undo step — on Enter, on a tool,
//! frame or layer change, on any edit that lands a float, or on a structural
//! edit. Esc throws it away; Ctrl+Z inside it takes back one stroke.
//!
//! A child of `app` for the same reason as `select`: it works on
//! `AppState`'s private state directly.

use std::sync::Arc;

use super::{AppState, Dirty};
use crate::tools::ribbon::union_rect;
use crate::doc::canvas::DirtyRect;
use crate::doc::layer::CellId;
use crate::input::pointer::PointerSample;
use crate::tools::lasso::Mask;
use crate::tools::liquify::{edge_weights, resample, resample_field, LiquifyField, LiquifyMode, MIN_RADIUS, STEP};
use crate::tools::selection::Placement;
use crate::tools::ActiveTool;
use crate::undo;

/// What the per-stroke copies of the field, kept for Ctrl+Z, may take
/// between them; the oldest go first.
const BACKUP_BYTES: usize = 256 << 20;

/// Grid points a side of a tile of the field, the unit a stroke's backup
/// keeps: 128 pixels.
const TILE: usize = 32;

/// A tile's (column, row) among the field's tiles.
type TileAt = (usize, usize);

/// The field as it was before one stroke, kept only where the stroke
/// wrote: each tile it reached, saved the first time a dab did. A copy of
/// the whole field at every pen-down was 16 MB on an 8K drawing.
#[derive(Default)]
struct StrokeBackup {
    /// Each tile kept, and the values its points had.
    tiles: Vec<(TileAt, Vec<[f32; 2]>)>,
    saved: std::collections::HashSet<TileAt>,
}

impl StrokeBackup {
    /// Keep the tiles under grid points `(i0..=i1, j0..=j1)` not kept yet,
    /// before a dab writes there.
    fn save(&mut self, field: &LiquifyField, (i0, i1, j0, j1): (usize, usize, usize, usize)) {
        for tj in j0 / TILE..=j1 / TILE {
            for ti in i0 / TILE..=i1 / TILE {
                if !self.saved.insert((ti, tj)) {
                    continue;
                }
                let (x0, x1) = (ti * TILE, ((ti + 1) * TILE).min(field.nw));
                let (y0, y1) = (tj * TILE, ((tj + 1) * TILE).min(field.nh));
                let mut v = Vec::with_capacity((x1 - x0) * (y1 - y0));
                for j in y0..y1 {
                    v.extend_from_slice(&field.d[j * field.nw + x0..j * field.nw + x1]);
                }
                self.tiles.push(((ti, tj), v));
            }
        }
    }

    /// Put the kept tiles back.
    fn restore(&self, field: &mut LiquifyField) {
        for &((ti, tj), ref v) in &self.tiles {
            let (x0, x1) = (ti * TILE, ((ti + 1) * TILE).min(field.nw));
            for (r, j) in (tj * TILE..((tj + 1) * TILE).min(field.nh)).enumerate() {
                let w = x1 - x0;
                field.d[j * field.nw + x0..j * field.nw + x1].copy_from_slice(&v[r * w..(r + 1) * w]);
            }
        }
    }

    fn bytes(&self) -> usize {
        self.tiles.iter().map(|(_, v)| v.len() * std::mem::size_of::<[f32; 2]>()).sum()
    }
}

/// A warp under way on one drawing.
pub struct LiquifySession {
    pub cell: CellId,
    pub field: LiquifyField,
    /// How far each grid point may move — the selection's soft edge.
    edge: Vec<f32>,
    /// The edge falloff `edge` was built for, to rebuild it when it changes.
    edge_width: f32,
    /// The selection as this drawing sees it; `None` warps the whole drawing.
    pub clip: Option<Arc<Mask>>,
    alpha_lock: bool,
    /// Where the drawing sat in the document, for warping its frame-mates.
    placement: Placement,
    /// Cell pixels the warp has written since the session began.
    touched: Option<DirtyRect>,
    /// Where the last dab landed, while the pen is down.
    last: Option<(f32, f32)>,
    /// Where the pen has got to since, and its pressure there: dabbed toward
    /// once a frame (see [`AppState::flush_liquify`]).
    aim: Option<((f32, f32), f32)>,
    /// Cell pixels the dabs have changed the field under since the drawing
    /// was last resampled, and where those dabs landed.
    pending: Option<DirtyRect>,
    dabs: Vec<(f32, f32)>,
    /// The field before each stroke, newest last: what Ctrl+Z goes back to.
    backups: Vec<StrokeBackup>,
}

impl AppState {
    /// The pen went down with Liquify: carry on this drawing's warp, or
    /// start one. Needs a drawing there already — a warp of nothing is
    /// nothing — on a layer that can be edited.
    pub(super) fn liquify_down(&mut self, s: PointerSample) {
        self.playback.stop();
        let here = self.project.resolved_current();
        if self.liquify.as_ref().is_some_and(|l| Some(l.cell) != here) {
            self.commit_liquify();
        }
        // Floating pixels land first, and the selection where they landed
        // holds the warp in.
        self.land_float_only();
        if self.liquify.is_none() && !self.start_liquify() {
            return;
        }
        let width = self.brush.liquify_edge.max(0.0);
        let Some(l) = self.liquify.as_mut() else {
            return;
        };
        if l.edge_width != width {
            l.edge = edge_weights(&l.field, l.clip.as_deref(), width);
            l.edge_width = width;
        }
        // The oldest strokes' backups go first past the cap.
        let mut kept: usize = l.backups.iter().map(StrokeBackup::bytes).sum();
        while !l.backups.is_empty() && kept > BACKUP_BYTES {
            kept -= l.backups.remove(0).bytes();
        }
        l.backups.push(StrokeBackup::default());
        l.last = Some((s.x, s.y));
        l.aim = None;
        if self.brush.liquify_mode == LiquifyMode::Smooth {
            self.liquify_dab((s.x, s.y), (s.x, s.y), s.pressure);
        }
    }

    /// The pen moved on. A push is noted, and dabbed toward when the
    /// frame's samples are all in (see [`AppState::flush_liquify`]). A
    /// smooth dabs now, every sample as before: held still, a smooth keeps
    /// working at the pen's rate, and a frame's worth in one would ease less.
    pub(super) fn liquify_move(&mut self, s: PointerSample) {
        let smooth = self.brush.liquify_mode == LiquifyMode::Smooth;
        let Some(l) = self.liquify.as_mut().filter(|l| l.last.is_some()) else {
            return;
        };
        if smooth {
            if let Some(from) = l.last {
                self.liquify_dab(from, (s.x, s.y), s.pressure);
            }
        } else {
            l.aim = Some(((s.x, s.y), s.pressure));
        }
    }

    pub(super) fn liquify_up(&mut self) {
        self.flush_liquify();
        if let Some(l) = &mut self.liquify {
            l.last = None;
        }
    }

    /// Catch the warp up with the pen, once a frame: dab from where the last
    /// dab landed to where the pen is now, then resample what all of that
    /// changed. A pen sends several samples a frame, and every dab moves the
    /// field under the whole brush; dabbing and resampling a frame at a time,
    /// not a sample at a time, is what keeps a big brush smooth. The canvas
    /// calls it after each frame's samples; pen-up and landing the warp call
    /// it too.
    pub fn flush_liquify(&mut self) {
        let step = self.liquify.as_mut().and_then(|l| Some((l.last?, l.aim.take()?)));
        if let Some((from, (to, pressure))) = step {
            self.liquify_dab(from, to, pressure);
        }
        let Some(l) = self.liquify.as_mut() else {
            return;
        };
        let dabs = std::mem::take(&mut l.dabs);
        if let Some(r) = l.pending.take() {
            // Past the brush's rim the field never moved; a grid step either
            // way more, the pixels between still read from points that did.
            let reach = self.effective_radius().max(MIN_RADIUS) + 2.0 * STEP as f32;
            self.liquify_render(r, Some((&dabs, reach)));
        }
    }

    fn start_liquify(&mut self) -> bool {
        let Some(cell) = self.project.resolved_current() else {
            return false;
        };
        if self.active_layer_locked() || self.active_layer_is_reference() {
            return false;
        }
        let Some((cw, ch)) = self.project.cell(cell).map(|c| (c.width, c.height)) else {
            return false;
        };
        let clip = self.active_cell_clip();
        // The field covers the selection's box, a grid step to spare, so an
        // 8K drawing with a small selection doesn't pay for the whole canvas.
        let region = match &clip {
            Some(m) => DirtyRect {
                min_x: m.x.saturating_sub(STEP).min(cw),
                min_y: m.y.saturating_sub(STEP).min(ch),
                max_x: (m.x + m.w + STEP).min(cw),
                max_y: (m.y + m.h + STEP).min(ch),
            },
            None => DirtyRect {
                min_x: 0,
                min_y: 0,
                max_x: cw,
                max_y: ch,
            },
        };
        if region.max_x <= region.min_x || region.max_y <= region.min_y {
            return false;
        }
        let field = LiquifyField::new(region);
        let edge_width = self.brush.liquify_edge.max(0.0);
        let edge = edge_weights(&field, clip.as_deref(), edge_width);
        // Usually free: the drawing was readied while the pen rested (see
        // `warm_pre`).
        self.snapshot_pre(cell);
        if let Some(c) = self.project.cell_mut(cell) {
            c.dirty = None;
        }
        self.liquify = Some(LiquifySession {
            cell,
            field,
            edge,
            edge_width,
            clip,
            alpha_lock: self.active_alpha_lock(),
            placement: self.active_placement(),
            touched: None,
            last: None,
            aim: None,
            pending: None,
            dabs: Vec::new(),
            backups: Vec::new(),
        });
        true
    }

    /// Dabs from `from` to `to`, a quarter of the brush apart: close enough
    /// that a push never carries a point past the next dab, which is what
    /// keeps the warp from folding over itself.
    fn liquify_dab(&mut self, from: (f32, f32), to: (f32, f32), pressure: f32) {
        let radius = self.effective_radius().max(MIN_RADIUS);
        let strength = self.brush.liquify_strength.clamp(0.0, 1.0) * pressure.clamp(0.0, 1.0);
        let mode = self.brush.liquify_mode;
        let Some(l) = self.liquify.as_mut() else {
            return;
        };
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let dist = dx.hypot(dy);
        let spacing = (radius / 4.0).max(1.0);
        let mut dirty: Option<DirtyRect> = None;
        // What each dab is about to write is kept first, for Ctrl+Z.
        let keep = |l: &mut LiquifySession, at: (f32, f32)| {
            if let (Some(b), Some(win)) = (l.backups.last_mut(), l.field.window(at.0, at.1, radius)) {
                b.save(&l.field, win);
            }
        };
        match mode {
            LiquifyMode::Push => {
                if dist > 0.0 {
                    let n = (dist / spacing).ceil().max(1.0);
                    let step = (dx / n, dy / n);
                    for k in 0..n as usize {
                        let at = (from.0 + step.0 * k as f32, from.1 + step.1 * k as f32);
                        keep(l, at);
                        if let Some(r) = l.field.push(at, step, radius, strength, &l.edge) {
                            dirty = Some(union_rect(dirty, r));
                            l.dabs.push(at);
                        }
                    }
                }
            }
            LiquifyMode::Smooth => {
                let n = (dist / spacing).ceil() as usize;
                for k in 0..=n {
                    let t = if n == 0 { 0.0 } else { k as f32 / n as f32 };
                    let at = (from.0 + dx * t, from.1 + dy * t);
                    keep(l, at);
                    if let Some(r) = l.field.smooth(at, radius, strength, &l.edge) {
                        dirty = Some(union_rect(dirty, r));
                        l.dabs.push(at);
                    }
                }
            }
        }
        l.last = Some(to);
        if let Some(r) = dirty {
            l.pending = Some(union_rect(l.pending, r));
        }
    }

    /// Resample `rect` of the warped drawing from the original: all of it,
    /// or with `dabs` only what those dabs reached (see `resample_field`).
    fn liquify_render(&mut self, rect: DirtyRect, dabs: Option<(&[(f32, f32)], f32)>) {
        let Some(l) = self.liquify.as_mut() else {
            return;
        };
        let cell = l.cell;
        let Some(c) = self.project.cells.get_mut(cell).map(Arc::make_mut) else {
            return;
        };
        let (cw, ch) = (c.width, c.height);
        let rect = DirtyRect {
            min_x: rect.min_x.min(cw),
            min_y: rect.min_y.min(ch),
            max_x: rect.max_x.min(cw),
            max_y: rect.max_y.min(ch),
        };
        if rect.max_x <= rect.min_x || rect.max_y <= rect.min_y {
            return;
        }
        resample_field(
            &self.stroke_pre_pixels,
            cw,
            ch,
            rect,
            c.pixels_mut(),
            l.clip.as_deref(),
            l.alpha_lock,
            &l.field,
            dabs,
        );
        c.mark_dirty(rect.min_x, rect.min_y, rect.max_x - rect.min_x, rect.max_y - rect.min_y);
        l.touched = Some(union_rect(l.touched, rect));
        self.mark_region(cell, Dirty::Rect(rect));
    }

    /// Land the warp, if one is under way: one undo step, for every drawing
    /// it reached with the Lasso's "All frames" on.
    pub fn commit_liquify(&mut self) {
        self.flush_liquify();
        let Some(l) = self.liquify.take() else {
            return;
        };
        let Some(touched) = l.touched else {
            self.stroke_pre_live = false;
            return;
        };
        if let Some(c) = self.project.cell_mut(l.cell) {
            c.dirty = Some(touched);
        }
        let mut parts: Vec<undo::Command> = self.take_patch(l.cell).into_iter().collect();
        if self.sel_all_frames {
            parts.extend(self.liquify_other_drawings(&l, touched));
        }
        match parts.len() {
            0 => {}
            1 => self.history.push(parts.remove(0)),
            _ => self.history.push(undo::Command::Compound(parts)),
        }
    }

    /// The same warp on every other drawing of the session's layer. The
    /// field is the active drawing's, read as a warp of the document: a
    /// drawing placed the same way takes it as it is, and one keyed
    /// elsewhere has each pixel sent into the active drawing's space, warped
    /// there, and sent back.
    fn liquify_other_drawings(&mut self, l: &LiquifySession, touched: DirtyRect) -> Vec<undo::Command> {
        let Some(li) = (0..self.project.layers.len())
            .find(|&li| self.project.layers[li].exposures.contains(&Some(l.cell)))
        else {
            return Vec::new();
        };
        let doc_mask = self.sel_mask.clone().filter(|m| !m.is_all());
        let (pw, ph) = (self.project.width as f32, self.project.height as f32);
        let p0 = l.placement;
        let mut parts = Vec::new();
        for (cell, f) in self.layer_drawings(li) {
            if cell == l.cell {
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
            let clip = doc_mask.as_ref().map(|m| m.to_cell(&pk.xf, pk.cw, pk.ch, pw, ph));
            let same = pk == p0;
            let to_k = |x: f32, y: f32| {
                let d = p0.xf.cell_to_doc(x, y, p0.cw as f32, p0.ch as f32, pw, ph);
                pk.xf.doc_to_cell(d.0, d.1, pk.cw as f32, pk.ch as f32, pw, ph)
            };
            // Where the warp can reach in this drawing.
            let rect = if same {
                touched
            } else {
                let corners = [
                    to_k(touched.min_x as f32, touched.min_y as f32),
                    to_k(touched.max_x as f32, touched.min_y as f32),
                    to_k(touched.min_x as f32, touched.max_y as f32),
                    to_k(touched.max_x as f32, touched.max_y as f32),
                ];
                let lo = |v: f32| (v.floor() - 2.0).max(0.0) as u32;
                let hi = |v: f32| (v.ceil() + 2.0).max(0.0) as u32;
                DirtyRect {
                    min_x: lo(corners.iter().map(|c| c.0).fold(f32::INFINITY, f32::min)),
                    min_y: lo(corners.iter().map(|c| c.1).fold(f32::INFINITY, f32::min)),
                    max_x: hi(corners.iter().map(|c| c.0).fold(f32::NEG_INFINITY, f32::max)).min(pk.cw),
                    max_y: hi(corners.iter().map(|c| c.1).fold(f32::NEG_INFINITY, f32::max)).min(pk.ch),
                }
            };
            if rect.max_x <= rect.min_x || rect.max_y <= rect.min_y {
                continue;
            }
            self.snapshot_pre(cell);
            let field = &l.field;
            let src = |x: f32, y: f32| -> Option<(f32, f32)> {
                if same {
                    let d = field.at(x, y);
                    return (d != [0.0, 0.0]).then_some((x + d[0], y + d[1]));
                }
                let doc = pk.xf.cell_to_doc(x, y, pk.cw as f32, pk.ch as f32, pw, ph);
                let c0 = p0.xf.doc_to_cell(doc.0, doc.1, p0.cw as f32, p0.ch as f32, pw, ph);
                let d = field.at(c0.0, c0.1);
                if d == [0.0, 0.0] {
                    return None;
                }
                let s = p0.xf.cell_to_doc(c0.0 + d[0], c0.1 + d[1], p0.cw as f32, p0.ch as f32, pw, ph);
                Some(pk.xf.doc_to_cell(s.0, s.1, pk.cw as f32, pk.ch as f32, pw, ph))
            };
            let Some(c) = self.project.cells.get_mut(cell).map(Arc::make_mut) else {
                continue;
            };
            c.dirty = None;
            resample(
                &self.stroke_pre_pixels,
                pk.cw,
                pk.ch,
                rect,
                c.pixels_mut(),
                clip.as_ref(),
                l.alpha_lock,
                &src,
            );
            c.mark_dirty(rect.min_x, rect.min_y, rect.max_x - rect.min_x, rect.max_y - rect.min_y);
            self.mark_dirty(cell);
            parts.extend(self.take_patch(cell));
        }
        parts
    }

    /// Esc: put the drawing back as the session found it and forget the
    /// warp. `true` if there was one.
    pub fn cancel_liquify(&mut self) -> bool {
        let Some(l) = self.liquify.take() else {
            return false;
        };
        if let Some(r) = l.touched {
            if let Some(c) = self.project.cells.get_mut(l.cell).map(Arc::make_mut) {
                let w4 = c.width as usize * 4;
                let (x0, x1) = (r.min_x as usize * 4, r.max_x as usize * 4);
                let px = c.pixels_mut();
                for y in r.min_y as usize..r.max_y as usize {
                    px[y * w4 + x0..y * w4 + x1].copy_from_slice(&self.stroke_pre_pixels[y * w4 + x0..y * w4 + x1]);
                }
                c.mark_dirty(r.min_x, r.min_y, r.max_x - r.min_x, r.max_y - r.min_y);
            }
            self.mark_region(l.cell, Dirty::Rect(r));
        }
        self.stroke_pre_live = false;
        true
    }

    /// Ctrl+Z inside a session: take back the last stroke, or — with none
    /// left to take back — the whole session. `true` if there was a session.
    pub(super) fn liquify_step_back(&mut self) -> bool {
        let Some(l) = self.liquify.as_mut() else {
            return false;
        };
        let Some(prev) = l.backups.pop() else {
            return self.cancel_liquify();
        };
        prev.restore(&mut l.field);
        l.last = None;
        l.aim = None;
        l.pending = None;
        l.dabs.clear();
        if let Some(r) = l.touched {
            self.liquify_render(r, None);
        }
        true
    }

    /// Per-frame upkeep: a warp lands once its drawing stops being the one
    /// in hand — another frame or layer, or another tool.
    pub(super) fn sync_liquify(&mut self) {
        let here = self.project.resolved_current();
        if self
            .liquify
            .as_ref()
            .is_some_and(|l| self.tool != ActiveTool::Liquify || Some(l.cell) != here)
        {
            self.commit_liquify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::button_drag::ButtonDrag;

    const RED: [u8; 4] = [200, 0, 0, 255];

    /// Paint an 8×8 dot with its corner at `(x, y)` in `cell`, outside the
    /// history.
    fn dot(st: &mut AppState, cell: CellId, x: u32, y: u32) {
        let c = st.project.cell_mut(cell).unwrap();
        for yy in y..y + 8 {
            for xx in x..x + 8 {
                let i = ((yy * c.width + xx) * 4) as usize;
                c.pixels_mut()[i..i + 4].copy_from_slice(&RED);
            }
        }
    }

    /// Alpha-weighted centre of `cell`'s pixels.
    fn centre(st: &AppState, cell: CellId) -> (f32, f32) {
        let c = st.project.cell(cell).unwrap();
        let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
        for (i, p) in c.pixels().chunks_exact(4).enumerate() {
            let a = p[3] as f32;
            sx += (i as u32 % c.width) as f32 * a;
            sy += (i as u32 / c.width) as f32 * a;
            n += a;
        }
        (sx / n, sy / n)
    }

    fn pixels(st: &AppState, cell: CellId) -> Vec<u8> {
        st.project.cell(cell).unwrap().pixels().into_owned()
    }

    /// A drawing with a dot at (100, 100), Liquify up with a big brush.
    fn liquify() -> (AppState, CellId) {
        let mut st = AppState::for_test();
        let cell = st.project.ensure_active_cell();
        dot(&mut st, cell, 100, 100);
        st.set_tool(ActiveTool::Liquify);
        st.brush.radius = 30.0;
        st.brush.liquify_strength = 1.0;
        (st, cell)
    }

    /// A push from `(x, y)` by `dx` to the right, in a few moves.
    fn push(st: &mut AppState, (x, y): (f32, f32), dx: f32) {
        st.pointer_down(st.make_sample(x, y, 0.0));
        for i in 1..=6 {
            let t = i as f32 / 6.0;
            st.pointer_move(st.make_sample(x + dx * t, y, t));
        }
        st.pointer_up();
    }

    #[test]
    fn a_liquify_session_commits_as_one_undo_step() {
        let (mut st, cell) = liquify();
        let before = pixels(&st, cell);
        let start = centre(&st, cell);
        let steps = st.history.undo_len();
        push(&mut st, (104.0, 104.0), 6.0);
        push(&mut st, (110.0, 104.0), 6.0);
        assert!(st.liquify.is_some(), "still live between strokes");
        assert_eq!(st.history.undo_len(), steps, "nothing recorded yet");
        let moved = centre(&st, cell).0 - start.0;
        assert!(moved > 6.0, "moved {moved}");
        st.land_float();
        assert!(st.liquify.is_none());
        assert_eq!(st.history.undo_len(), steps + 1);
        st.undo();
        assert!(pixels(&st, cell) == before, "undo is byte-exact");
        st.redo();
        assert!((centre(&st, cell).0 - start.0 - moved).abs() < 1e-3);
    }

    /// Ctrl+Z puts the field back exactly as it was before each stroke, from
    /// the tiles the stroke kept — at the drawing's edge too, where the
    /// tiles are cut short.
    #[test]
    fn ctrl_z_restores_the_field_exactly_from_its_tiles() {
        let (mut st, _) = liquify();
        st.brush.radius = 60.0;
        let (w, h) = (st.project.width as f32, st.project.height as f32);
        push(&mut st, (w - 20.0, h - 20.0), -40.0);
        let after_one = st.liquify.as_ref().unwrap().field.d.clone();
        push(&mut st, (100.0, 104.0), 50.0);
        assert_ne!(st.liquify.as_ref().unwrap().field.d, after_one);
        st.undo();
        assert_eq!(st.liquify.as_ref().unwrap().field.d, after_one, "back to after the first stroke");
        st.undo();
        assert!(st.liquify.as_ref().unwrap().field.d.iter().all(|d| *d == [0.0, 0.0]), "and before it");
    }

    #[test]
    fn esc_restores_exactly() {
        let (mut st, cell) = liquify();
        let before = pixels(&st, cell);
        let steps = st.history.undo_len();
        push(&mut st, (104.0, 104.0), 10.0);
        assert!(pixels(&st, cell) != before);
        assert!(st.cancel_liquify());
        assert!(pixels(&st, cell) == before);
        assert_eq!(st.history.undo_len(), steps);
        assert!(!st.cancel_liquify(), "nothing left to cancel");
    }

    #[test]
    fn switching_tool_or_frame_commits() {
        let (mut st, _) = liquify();
        let steps = st.history.undo_len();
        push(&mut st, (104.0, 104.0), 8.0);
        st.set_tool(ActiveTool::Pencil);
        st.sync_liquify();
        assert!(st.liquify.is_none());
        assert_eq!(st.history.undo_len(), steps + 1);

        // Another frame with a drawing of its own.
        let (mut st, _) = liquify();
        st.structural_edit(false, |p| {
            p.add_frame();
            p.insert_blank_key_here();
        });
        st.project.goto(0);
        let steps = st.history.undo_len();
        push(&mut st, (104.0, 104.0), 8.0);
        st.project.goto(1);
        st.sync_liquify();
        assert!(st.liquify.is_none());
        assert_eq!(st.history.undo_len(), steps + 1);
    }

    #[test]
    fn ctrl_z_steps_back_one_stroke_in_the_session() {
        let (mut st, cell) = liquify();
        let before = pixels(&st, cell);
        let steps = st.history.undo_len();
        push(&mut st, (104.0, 104.0), 6.0);
        let after_one = pixels(&st, cell);
        push(&mut st, (110.0, 104.0), 6.0);
        st.undo();
        assert!(pixels(&st, cell) == after_one, "back one stroke");
        assert!(st.liquify.is_some());
        st.undo();
        assert!(pixels(&st, cell) == before, "back to the start");
        st.undo();
        assert!(st.liquify.is_none(), "and out of the session");
        assert!(pixels(&st, cell) == before);
        assert_eq!(st.history.undo_len(), steps, "nothing recorded or undone");
    }

    /// With "All frames" on, every drawing of the layer takes the same warp,
    /// in one undo step — including one drawn on a bigger canvas, where the
    /// same place in the document is elsewhere in the cell.
    #[test]
    fn all_frames_lands_the_warp_on_every_drawing() {
        let (mut st, first) = liquify();
        st.structural_edit(false, |p| {
            p.add_frame();
            p.insert_blank_key_here();
        });
        let second = st.project.resolved_current().unwrap();
        let (w, h) = (st.project.width, st.project.height);
        st.project.cells[second] = Arc::new(crate::doc::canvas::Canvas::new(w + 40, h + 40));
        dot(&mut st, second, 120, 120);
        st.project.goto(0);
        let starts = [centre(&st, first), centre(&st, second)];
        let befores = [pixels(&st, first), pixels(&st, second)];
        st.sel_all_frames = true;
        let steps = st.history.undo_len();
        push(&mut st, (104.0, 104.0), 8.0);
        st.land_float();
        assert_eq!(st.history.undo_len(), steps + 1, "one step for both");
        let moved = [centre(&st, first).0 - starts[0].0, centre(&st, second).0 - starts[1].0];
        assert!(moved[0] > 4.0, "{moved:?}");
        assert!((moved[0] - moved[1]).abs() < 0.5, "the same warp: {moved:?}");
        st.undo();
        assert!(pixels(&st, first) == befores[0]);
        assert!(pixels(&st, second) == befores[1]);
    }

    #[test]
    fn a_float_is_landed_before_liquify() {
        let (mut st, _) = liquify();
        st.set_tool(ActiveTool::Lasso);
        st.sel_shape = crate::tools::select_mask::SelShape::Rect;
        st.select_down((90.0, 90.0), None);
        st.select_move((130.0, 130.0));
        st.select_up();
        assert!(st.make_float());
        st.nudge_selection(3, 0);
        st.tool = ActiveTool::Liquify;
        push(&mut st, (107.0, 104.0), 6.0);
        assert!(st.selection.is_none(), "landed first");
        assert!(st.liquify.is_some());
    }

    #[test]
    fn a_locked_or_reference_layer_is_refused() {
        for lock in [true, false] {
            let (mut st, cell) = liquify();
            st.project.layers[0].locked = lock;
            st.project.layers[0].reference = !lock;
            let before = pixels(&st, cell);
            push(&mut st, (104.0, 104.0), 10.0);
            assert!(st.liquify.is_none());
            assert!(pixels(&st, cell) == before);
        }
    }

    #[test]
    fn nothing_to_warp_on_an_empty_frame() {
        let mut st = AppState::for_test();
        st.project.layers[0].exposures[0] = None;
        st.set_tool(ActiveTool::Liquify);
        let cells = st.project.cells.len();
        push(&mut st, (50.0, 50.0), 10.0);
        assert!(st.liquify.is_none());
        assert_eq!(st.project.cells.len(), cells, "no drawing made for it");
    }

    #[test]
    fn tool_adjust_sets_liquify_size_and_strength() {
        let (mut st, _) = liquify();
        st.brush.liquify_strength = 0.5;
        let at = egui::pos2(400.0, 300.0);
        st.begin_button_drag(ButtonDrag::ToolAdjust, at);
        st.button_drag_to(at + egui::vec2(200.0, -150.0));
        assert!(st.brush.radius > 45.0, "{}", st.brush.radius);
        assert!(st.brush.liquify_strength > 0.8, "{}", st.brush.liquify_strength);
        assert!(st.button_drag_readout().unwrap().starts_with("Size"));
        st.cancel_button_drag();
        assert_eq!(st.brush.radius, 30.0);
        assert_eq!(st.brush.liquify_strength, 0.5);
    }
}
