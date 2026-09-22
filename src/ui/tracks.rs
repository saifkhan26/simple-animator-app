//! Timeline layer tracks: a frame ruler over one row per layer, with every
//! drawing shown as a bar from its key to the next. The bars are where timing
//! gets edited — click to select (Shift for a rectangle, Ctrl to toggle), drag
//! to move (Ctrl to copy), drag a bar's right edge to retime, Delete or
//! Ctrl+Delete, and a right-click menu for the rest.
//!
//! One custom-painted widget rather than nested `ScrollArea`s, for two
//! reasons. egui has no frozen panes, and the header column has to stay put
//! while the frames scroll sideways, the ruler while the rows scroll down. And
//! the plain wheel over the Timeline scrubs frames
//! (`shell::timeline_wheel_scrub`), so a `ScrollArea` in here would scroll and
//! scrub on the same notch. The tracks take only the modified wheel — Ctrl
//! zooms, Shift pans — and the wheel over the layer names, and leave the rest
//! to the scrubber.

use egui::{pos2, vec2, Color32, CursorIcon, Id, Pos2, Rect, Sense, Stroke};
use egui_phosphor::regular as ic;

use crate::app::AppState;
use crate::doc::layer::CellId;
use crate::doc::project::{layer_retime, Block};
use crate::input::shortcuts::Action;
use crate::ui::shell::{combo_text, DIAG_BAD, KEY_CAMERA, KEY_LAYER};
use crate::ui::theme;

/// Frame column width a fresh install starts at, in points.
pub const DEFAULT_FRAME_W: f32 = 16.0;
const MIN_FRAME_W: f32 = 6.0;
const MAX_FRAME_W: f32 = 48.0;
const ROW_H: f32 = 24.0;
const RULER_H: f32 = 20.0;
const HEADER_W: f32 = 132.0;
/// Space between a bar and the top and bottom of its row.
const BAR_INSET: f32 = 5.0;
/// Width of the grab zone at a bar's right end, in points.
const EDGE_GRAB: f32 = 5.0;
/// How close to an edge a drag gets before the tracks scroll that way.
const AUTOSCROLL_ZONE: f32 = 20.0;
/// Frames kept between the playhead and the edge when following it.
const FOLLOW_MARGIN: f32 = 3.0;
/// Frames of room past the end, so the last drawing's edge can be dragged
/// out beyond it.
const END_SLACK: f32 = 4.0;
/// Stand-in id for the blank cell a previewed retime would allocate.
const PREVIEW_BLANK: CellId = CellId::MAX;
/// How long the pointer rests on a bar before its details show, in seconds.
const HOVER_DELAY: f32 = 0.5;
/// Where `show` leaves this frame's header rect for the wheel scrubber.
const HEADER_RECT_KEY: &str = "tracks_header_rect";

// The tracks sit in an inset well, a step below the panel the way the app's
// inputs sit on theirs: the theme's own surfaces and its one accent, nothing
// new.
const WELL: Color32 = Color32::from_rgb(19, 21, 25);
const WELL_ROUNDING: f32 = 6.0;
/// Hairline between rows — quieter than the well's own edges.
const ROW_LINE: Color32 = Color32::from_rgb(30, 33, 39);
const BAR: Color32 = Color32::from_rgb(50, 55, 67);
const BAR_HOVER: Color32 = Color32::from_rgb(63, 69, 85);
const BAR_ROUNDING: f32 = 4.0;
/// Thickness of the scroll thumbs along the bottom and right edges.
const THUMB: f32 = 4.0;

/// The active layer's row, lifted just enough to find.
fn active_tint() -> Color32 {
    theme::ACCENT.gamma_multiply(0.07)
}

/// A drag in progress in the tracks.
#[derive(Clone, Copy, Debug)]
pub enum TrackDrag {
    /// Moving the selection — or copying it, if Ctrl is down at release —
    /// picked up at this (layer, frame).
    Body { grab: (usize, usize) },
    /// Stretching one drawing's hold by its right edge.
    Edge { block: Block },
}

/// This frame's header column, if the tracks drew one. The wheel scrolls the
/// rows there instead of scrubbing.
pub fn header_rect(ctx: &egui::Context) -> Option<Rect> {
    let (pass, rect): (u64, Rect) = ctx.data(|d| d.get_temp(Id::new(HEADER_RECT_KEY)))?;
    (pass == ctx.cumulative_pass_nr()).then_some(rect)
}

/// Screen mapping for this frame.
#[derive(Clone, Copy)]
struct Geo {
    tracks: Rect,
    fw: f32,
    scroll: egui::Vec2,
    n_layers: usize,
    frame_count: usize,
}

impl Geo {
    fn x(&self, frame: f32) -> f32 {
        self.tracks.min.x + frame * self.fw - self.scroll.x
    }

    /// Frame under screen `x`, unclamped.
    fn frame_at(&self, x: f32) -> isize {
        ((x - self.tracks.min.x + self.scroll.x) / self.fw).floor() as isize
    }

    /// Frame under screen `x`, clamped onto the timeline.
    fn frame_on_sheet(&self, x: f32) -> usize {
        self.frame_at(x).clamp(0, self.frame_count as isize - 1) as usize
    }

    /// Frame boundary nearest screen `x`.
    fn boundary_at(&self, x: f32) -> isize {
        ((x - self.tracks.min.x + self.scroll.x) / self.fw).round() as isize
    }

    /// Top of layer `layer`'s row. The top layer is drawn first, matching the
    /// Layers panel.
    fn row_y(&self, layer: usize) -> f32 {
        let r = self.n_layers - 1 - layer;
        self.tracks.min.y + r as f32 * ROW_H - self.scroll.y
    }

    fn row(&self, layer: usize) -> Rect {
        let y = self.row_y(layer);
        Rect::from_min_max(pos2(self.tracks.min.x, y), pos2(self.tracks.max.x, y + ROW_H))
    }

    /// Layer under screen `y`, or `None` off the rows.
    fn layer_at(&self, y: f32) -> Option<usize> {
        let r = ((y - self.tracks.min.y + self.scroll.y) / ROW_H).floor();
        (r >= 0.0 && (r as usize) < self.n_layers).then(|| self.n_layers - 1 - r as usize)
    }

    /// Layer under screen `y`, clamped onto the sheet.
    fn layer_on_sheet(&self, y: f32) -> usize {
        let r = ((y - self.tracks.min.y + self.scroll.y) / ROW_H).floor() as isize;
        self.n_layers - 1 - r.clamp(0, self.n_layers as isize - 1) as usize
    }

    /// Bar for frames `start..end` on `layer`.
    fn bar(&self, layer: usize, start: usize, end: usize) -> Rect {
        let row = self.row(layer);
        Rect::from_min_max(
            pos2(self.x(start as f32) + 1.0, row.min.y + BAR_INSET),
            pos2(self.x(end as f32) - 1.0, row.max.y - BAR_INSET),
        )
    }
}

/// What the pointer is over in the tracks.
#[derive(Clone, Copy)]
struct Hit {
    layer: usize,
    frame: usize,
    block: Option<Block>,
    /// On the grab zone at the block's right end.
    edge: bool,
}

fn hit_test(state: &AppState, g: &Geo, p: Pos2) -> Option<Hit> {
    if !g.tracks.contains(p) {
        return None;
    }
    let layer = g.layer_at(p.y)?;
    let f = g.frame_at(p.x);
    if f < 0 || f as usize >= g.frame_count {
        return None;
    }
    let frame = f as usize;
    let block = state.project.block_at(layer, frame);
    let edge = block.is_some_and(|b| {
        let span = b.span(g.frame_count);
        let zone = EDGE_GRAB.min(span.len() as f32 * g.fw / 3.0).max(2.0);
        p.x >= g.x(span.end as f32) - zone
    });
    Some(Hit {
        layer,
        frame,
        block,
        edge,
    })
}

/// What letting go right now would do.
#[derive(Clone, Copy)]
enum Target {
    Body {
        dl: isize,
        df: isize,
        /// (layer, frame) the grabbed frame lands on.
        cursor: (usize, usize),
        moved: bool,
        ok: bool,
    },
    Edge {
        block: Block,
        old_len: usize,
        new_len: usize,
    },
}

fn drag_target(state: &AppState, g: &Geo, drag: TrackDrag, p: Pos2, copy: bool) -> Option<Target> {
    match drag {
        TrackDrag::Body { grab } => {
            let sel = state.track_selection();
            let min_l = sel.iter().map(|s| s.0).min()? as isize;
            let max_l = sel.iter().map(|s| s.0).max()? as isize;
            let min_f = sel.iter().map(|s| s.1).min()? as isize;
            let max_f = sel.iter().map(|s| s.1).max()? as isize;
            // Clamped so the selection stops at the sheet's edges instead of
            // turning red there.
            let dl = (g.layer_on_sheet(p.y) as isize - grab.0 as isize)
                .clamp(-min_l, g.n_layers as isize - 1 - max_l);
            let df = (g.frame_on_sheet(p.x) as isize - grab.1 as isize)
                .clamp(-min_f, g.frame_count as isize - 1 - max_f);
            let moved = copy || dl != 0 || df != 0;
            Some(Target::Body {
                dl,
                df,
                cursor: (
                    (grab.0 as isize + dl) as usize,
                    (grab.1 as isize + df) as usize,
                ),
                moved,
                ok: moved && state.project.can_move_blocks(&sel, dl, df, copy),
            })
        }
        TrackDrag::Edge { block } => {
            let old_len = block.len.unwrap_or(g.frame_count - block.start);
            let new_len = (g.boundary_at(p.x) - block.start as isize).max(1) as usize;
            Some(Target::Edge {
                block,
                old_len,
                new_len,
            })
        }
    }
}

/// (start, end, cell) for every drawing in one layer's exposures.
fn bars(exposures: &[Option<CellId>]) -> Vec<(usize, usize, CellId)> {
    let keys: Vec<(usize, CellId)> = exposures
        .iter()
        .enumerate()
        .filter_map(|(f, e)| e.map(|id| (f, id)))
        .collect();
    keys.iter()
        .enumerate()
        .map(|(i, &(s, id))| (s, keys.get(i + 1).map_or(exposures.len(), |k| k.0), id))
        .collect()
}

/// The tracks: ruler, layer headers and drawing bars. Fills the width it is
/// given.
pub fn show(state: &mut AppState, ui: &mut egui::Ui) {
    let n_layers = state.project.layers.len();
    if n_layers == 0 {
        return;
    }
    let ctx = ui.ctx().clone();
    let id = ui.id().with("tracks");
    let fc = state.project.frame_count.max(1);
    state.track_frame_w = state.track_frame_w.clamp(MIN_FRAME_W, MAX_FRAME_W);

    // Fill whatever the panel gives: dragging its edge is how more layers or
    // more frames come into view.
    let avail = ui.available_size();
    let size = vec2(avail.x.max(HEADER_W + 80.0), avail.y.max(RULER_H + ROW_H));
    let (outer, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(outer, WELL_ROUNDING, WELL);
    let corner = Rect::from_min_size(outer.min, vec2(HEADER_W, RULER_H));
    let ruler = Rect::from_min_max(pos2(corner.max.x, outer.min.y), pos2(outer.max.x, corner.max.y));
    let header = Rect::from_min_max(pos2(outer.min.x, corner.max.y), pos2(corner.max.x, outer.max.y));
    let tracks = Rect::from_min_max(corner.max, outer.max);
    // Read outside `data_mut`: the context's lock is not re-entrant, and
    // asking it anything from inside that closure deadlocks.
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(Id::new(HEADER_RECT_KEY), (pass, header)));

    wheel(state, ui, ruler.union(tracks), header);
    follow_cursor(state, ui, tracks, id);
    clamp_scroll(state, tracks, fc, n_layers);

    let g = Geo {
        tracks,
        fw: state.track_frame_w,
        scroll: state.track_scroll,
        n_layers,
        frame_count: fc,
    };

    corner_ui(state, ui, corner, id);
    ruler_ui(state, ui, &g, ruler, id);
    header_ui(state, ui, &g, header, id);

    let resp = ui.interact(tracks, id.with("area"), Sense::click_and_drag());
    let (pointer, mods) = ui.input(|i| (i.pointer.interact_pos(), i.modifiers));
    let copy = mods.command;

    if resp.clicked() {
        if let Some(h) = resp.interact_pointer_pos().and_then(|p| hit_test(state, &g, p)) {
            click(state, h, mods);
        }
    }
    if resp.secondary_clicked() {
        // The menu acts on the selection and on the slot it was opened over,
        // so both follow the right-click first.
        if let Some(h) = resp.interact_pointer_pos().and_then(|p| hit_test(state, &g, p)) {
            if let Some(b) = h.block {
                if !state.track_sel.contains(&(b.layer, b.start)) {
                    state.track_sel = [(b.layer, b.start)].into_iter().collect();
                    state.track_anchor = Some((h.layer, h.frame));
                }
            }
            state.project.current_layer = h.layer;
            state.project.goto(h.frame);
        }
    }
    if resp.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin());
        if let Some(h) = origin.and_then(|p| hit_test(state, &g, p)) {
            if let Some(b) = h.block {
                state.track_drag = Some(if h.edge {
                    TrackDrag::Edge { block: b }
                } else {
                    // Grabbing an unselected drawing drags it alone, as a
                    // file manager does.
                    if !state.track_sel.contains(&(b.layer, b.start)) {
                        state.track_sel = [(b.layer, b.start)].into_iter().collect();
                        state.track_anchor = Some((h.layer, h.frame));
                    }
                    TrackDrag::Body {
                        grab: (h.layer, h.frame),
                    }
                });
                state.playback.stop();
            }
        }
    }

    let mut target = match (state.track_drag, pointer) {
        (Some(drag), Some(p)) if resp.dragged() || resp.drag_stopped() => {
            drag_target(state, &g, drag, p, copy)
        }
        _ => None,
    };

    if resp.drag_stopped() {
        // Applied, then forgotten: the selection now sits where it landed,
        // so this frame's preview would offset it a second time.
        let applied = target.take();
        if state.track_drag.take().is_some() {
            match applied {
                Some(Target::Body {
                    dl,
                    df,
                    cursor,
                    ok: true,
                    ..
                }) => state.move_track_selection(dl, df, copy, cursor),
                Some(Target::Edge { block, new_len, .. }) => state.retime_track_block(block, new_len),
                _ => {}
            }
        }
    } else if resp.dragged() && state.track_drag.is_some() {
        if let Some(p) = pointer {
            autoscroll(state, &ctx, tracks, p, matches!(target, Some(Target::Body { .. })));
        }
    }

    // What the pointer is doing decides the cursor.
    let hover = if resp.hovered() {
        pointer.and_then(|p| hit_test(state, &g, p))
    } else {
        None
    };
    match target {
        Some(Target::Body { moved, ok, .. }) => ctx.set_cursor_icon(match (moved, ok) {
            (true, false) => CursorIcon::NoDrop,
            (true, true) if copy => CursorIcon::Copy,
            _ => CursorIcon::Grabbing,
        }),
        Some(Target::Edge { .. }) => ctx.set_cursor_icon(CursorIcon::ResizeHorizontal),
        None if hover.is_some_and(|h| h.edge) => ctx.set_cursor_icon(CursorIcon::ResizeHorizontal),
        None => {}
    }

    let dragging = state.track_drag.is_some();
    paint(state, ui, &g, target, copy, hover.filter(|_| !dragging));
    well_lines(ui, outer, header, ruler);
    scrollbars(state, ui, &g, id);

    if let (Some(t), Some(p)) = (target.filter(|_| state.track_drag.is_some()), pointer) {
        if let Some(text) = drag_label(state, t, copy) {
            pointer_label(&ctx, id, p, text);
        }
    }

    resp.context_menu(|ui| context_menu(state, ui));
    // A label of our own, offset from the pointer, rather than egui's
    // at-pointer tooltip: on the frame that tooltip first appears it can sit
    // over the pointer, and a press then lands on it instead of the bar.
    let idle = state.track_drag.is_none()
        && !resp.context_menu_opened()
        && !ui.input(|i| i.pointer.any_down());
    if let (true, Some(b), Some(p)) = (idle, hover.and_then(|h| h.block), pointer) {
        let still = ui.input(|i| i.pointer.time_since_last_movement());
        if still >= HOVER_DELAY {
            pointer_label(&ctx, id.with("hover"), p, hover_text(state, b));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f32(HOVER_DELAY - still));
        }
    }
}

/// A plain click selects the drawing under it, Shift-click selects every
/// drawing in the rectangle back to the last plain click, Ctrl-click toggles
/// one. All three move the cursor there.
fn click(state: &mut AppState, h: Hit, mods: egui::Modifiers) {
    let key = h.block.map(|b| (b.layer, b.start));
    if mods.command {
        if let Some(k) = key {
            if !state.track_sel.remove(&k) {
                state.track_sel.insert(k);
            }
        }
    } else if let (true, Some((al, af))) = (mods.shift, state.track_anchor) {
        let (l0, l1) = (al.min(h.layer), al.max(h.layer));
        let (f0, f1) = (af.min(h.frame), af.max(h.frame));
        let fc = state.project.frame_count;
        let picked: Vec<(usize, usize)> = (l0..=l1)
            .flat_map(|l| state.project.blocks(l))
            .filter(|b| {
                let s = b.span(fc);
                s.start <= f1 && s.end > f0
            })
            .map(|b| (b.layer, b.start))
            .collect();
        state.track_sel = picked.into_iter().collect();
    } else {
        state.track_sel = key.into_iter().collect();
        state.track_anchor = Some((h.layer, h.frame));
    }
    state.project.current_layer = h.layer;
    state.project.goto(h.frame);
}

/// The modified wheel over the tracks: Ctrl zooms the frame width about the
/// pointer, Shift (or a sideways trackpad swipe) pans. The plain wheel over
/// the layer names scrolls the rows. The plain wheel over the frames is left
/// alone — that is the scrubber's.
fn wheel(state: &mut AppState, ui: &egui::Ui, frames: Rect, header: Rect) {
    let Some(p) = ui.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    // A popup or another window over the tracks keeps its own wheel.
    if ui.ctx().layer_id_at(p) != Some(ui.layer_id()) {
        return;
    }
    let (over_frames, over_header) = (frames.contains(p), header.contains(p));
    if !over_frames && !over_header {
        return;
    }
    let (delta, mods) = ui.input(|i| {
        let mut d = egui::Vec2::ZERO;
        for e in &i.events {
            if let egui::Event::MouseWheel { unit, delta, .. } = e {
                d += *delta
                    * match unit {
                        egui::MouseWheelUnit::Point => 1.0,
                        egui::MouseWheelUnit::Line => 40.0,
                        egui::MouseWheelUnit::Page => 200.0,
                    };
            }
        }
        (d, i.modifiers)
    });
    if delta == egui::Vec2::ZERO {
        return;
    }
    if over_header {
        state.track_scroll.y -= delta.y;
    } else if mods.command {
        let old = state.track_frame_w;
        let new = (old * 1.15f32.powf(delta.y / 40.0)).clamp(MIN_FRAME_W, MAX_FRAME_W);
        // Keep the frame under the pointer where it is.
        let at = (p.x - frames.min.x + state.track_scroll.x) / old;
        state.track_frame_w = new;
        state.track_scroll.x = at * new - (p.x - frames.min.x);
    } else if mods.shift {
        state.track_scroll.x -= delta.x + delta.y;
    } else {
        state.track_scroll.x -= delta.x;
    }
}

/// Bring the playhead and the active layer into view when either changes —
/// only then, so the tracks can still be scrolled away from them freely.
fn follow_cursor(state: &mut AppState, ui: &egui::Ui, tracks: Rect, id: Id) {
    let key = id.with("follow");
    let cur = (state.project.current_frame, state.project.current_layer);
    let last: Option<(usize, usize)> = ui.data(|d| d.get_temp(key));
    if last == Some(cur) {
        return;
    }
    ui.data_mut(|d| d.insert_temp(key, cur));
    if state.track_drag.is_some() {
        return;
    }
    let fw = state.track_frame_w;
    let margin = FOLLOW_MARGIN * fw;
    let s = &mut state.track_scroll;
    let x0 = cur.0 as f32 * fw;
    if x0 - margin < s.x {
        s.x = x0 - margin;
    } else if x0 + fw + margin > s.x + tracks.width() {
        s.x = x0 + fw + margin - tracks.width();
    }
    let n = state.project.layers.len();
    let y0 = (n.saturating_sub(1 + cur.1)) as f32 * ROW_H;
    if y0 < s.y {
        s.y = y0;
    } else if y0 + ROW_H > s.y + tracks.height() {
        s.y = y0 + ROW_H - tracks.height();
    }
}

fn clamp_scroll(state: &mut AppState, tracks: Rect, fc: usize, n_layers: usize) {
    let w = (fc as f32 + END_SLACK) * state.track_frame_w;
    let h = n_layers as f32 * ROW_H;
    let s = &mut state.track_scroll;
    s.x = s.x.clamp(0.0, (w - tracks.width()).max(0.0));
    s.y = s.y.clamp(0.0, (h - tracks.height()).max(0.0));
}

/// Scroll toward whichever edge a drag is pressing against. Rows only for a
/// move: a retime stays on its own layer.
fn autoscroll(state: &mut AppState, ctx: &egui::Context, tracks: Rect, p: Pos2, rows: bool) {
    let push = |lo: f32, hi: f32, v: f32| -> f32 {
        if v > hi - AUTOSCROLL_ZONE {
            ((v - (hi - AUTOSCROLL_ZONE)) / AUTOSCROLL_ZONE).min(2.0)
        } else if v < lo + AUTOSCROLL_ZONE {
            -((lo + AUTOSCROLL_ZONE - v) / AUTOSCROLL_ZONE).min(2.0)
        } else {
            0.0
        }
    };
    let dx = push(tracks.min.x, tracks.max.x, p.x) * state.track_frame_w * 0.5;
    let dy = if rows {
        push(tracks.min.y, tracks.max.y, p.y) * ROW_H * 0.25
    } else {
        0.0
    };
    if dx != 0.0 || dy != 0.0 {
        state.track_scroll += vec2(dx, dy);
        ctx.request_repaint();
    }
}

/// Above the layer names: a label, and zoom buttons — Ctrl+wheel does the
/// same, but a gesture nobody can see is one nobody finds.
fn corner_ui(state: &mut AppState, ui: &mut egui::Ui, corner: Rect, id: Id) {
    let painter = ui.painter_at(corner);
    painter.text(
        pos2(corner.min.x + 10.0, corner.center().y),
        egui::Align2::LEFT_CENTER,
        "Layers",
        egui::FontId::proportional(10.5),
        theme::TEXT_MUTED,
    );
    let size = vec2(18.0, 16.0);
    let plus = Rect::from_min_size(
        pos2(corner.max.x - size.x - 4.0, corner.center().y - size.y / 2.0),
        size,
    );
    let minus = plus.translate(vec2(-(size.x + 1.0), 0.0));
    for (rect, glyph, factor, tip) in [
        (minus, ic::MAGNIFYING_GLASS_MINUS, 1.0 / 1.25, "Narrower frames (Ctrl+wheel)"),
        (plus, ic::MAGNIFYING_GLASS_PLUS, 1.25, "Wider frames (Ctrl+wheel)"),
    ] {
        let r = ui.interact(rect, id.with(glyph), Sense::click()).on_hover_text(tip);
        if r.hovered() {
            painter.rect_filled(rect, 4.0, theme::BG_HOVER);
        }
        let color = if r.hovered() { theme::TEXT } else { theme::TEXT_MUTED };
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(11.0),
            color,
        );
        if r.clicked() {
            state.track_frame_w = (state.track_frame_w * factor).clamp(MIN_FRAME_W, MAX_FRAME_W);
        }
    }
}

/// Frame numbers, camera keys, a loop range that isn't the whole timeline,
/// and the playhead as an accent pill carrying its frame number. Click or drag
/// to scrub.
fn ruler_ui(state: &mut AppState, ui: &mut egui::Ui, g: &Geo, ruler: Rect, id: Id) {
    let resp = ui.interact(ruler, id.with("ruler"), Sense::click_and_drag());
    if resp.clicked() || resp.dragged() {
        if let Some(p) = resp.interact_pointer_pos() {
            state.project.goto(g.frame_on_sheet(p.x));
            if resp.dragged() {
                autoscroll(state, ui.ctx(), ruler, p, false);
            }
        }
    }

    let painter = ui.painter_at(ruler);
    let fc = g.frame_count;
    let cur = state.project.current_frame;

    // The playhead's pill, laid out first so the numbers can step around it.
    let pill_text = painter.layout_no_wrap(
        format!("{cur}"),
        egui::FontId::proportional(10.5),
        theme::ACCENT_TEXT,
    );
    let pill_h = RULER_H - 6.0;
    let pill = Rect::from_center_size(
        pos2(g.x(cur as f32 + 0.5), ruler.center().y),
        vec2((pill_text.size().x + 10.0).max(g.fw - 2.0), pill_h),
    );

    // A number every `step` frames, however far out the zoom.
    let step = [1usize, 2, 5, 10, 20, 50, 100, 200, 500]
        .into_iter()
        .find(|&s| s as f32 * g.fw >= 30.0)
        .unwrap_or(1000);
    let first = (g.frame_at(ruler.min.x).max(0) as usize).min(fc);
    let last = (g.frame_at(ruler.max.x).max(0) as usize + 1).min(fc);
    for f in first..last {
        let x = g.x(f as f32);
        if g.fw >= 9.0 || f % step == 0 {
            let tick = if f % step == 0 { 4.0 } else { 2.0 };
            painter.vline(
                x,
                (ruler.max.y - tick)..=ruler.max.y,
                Stroke::new(1.0, theme::STROKE_THIN),
            );
        }
        if f % step != 0 || f == cur {
            continue;
        }
        let label = painter.layout_no_wrap(
            format!("{f}"),
            egui::FontId::proportional(10.0),
            theme::TEXT_MUTED,
        );
        let at = pos2(
            g.x(f as f32 + 0.5) - label.size().x / 2.0,
            ruler.center().y - label.size().y / 2.0 - 1.0,
        );
        let rect = Rect::from_min_size(at, label.size());
        if !rect.expand(2.0).intersects(pill) {
            painter.galley(at, label, theme::TEXT_MUTED);
        }
    }

    let (lo, hi) = (state.project.loop_start, state.project.loop_end.min(fc));
    if lo < hi && (lo, hi) != (0, fc) {
        let r = Rect::from_min_max(
            pos2(g.x(lo as f32), ruler.max.y - 3.0),
            pos2(g.x(hi as f32), ruler.max.y - 1.0),
        );
        painter.rect_filled(r, 1.0, theme::ACCENT.gamma_multiply(0.45));
    }
    for k in &state.project.camera_keys {
        if k.frame < fc {
            let c = pos2(g.x(k.frame as f32 + 0.5), ruler.max.y - 3.0);
            painter.circle_filled(c, 1.8, KEY_CAMERA);
        }
    }

    // Past the end, as in the tracks below.
    let end_x = g.x(fc as f32);
    if end_x < ruler.max.x {
        let shade = Rect::from_min_max(pos2(end_x, ruler.min.y), ruler.max);
        let rounding = egui::Rounding {
            ne: WELL_ROUNDING - 1.0,
            ..Default::default()
        };
        painter.rect_filled(shade, rounding, Color32::from_black_alpha(40));
    }

    painter.rect_filled(pill, pill_h / 2.0, theme::ACCENT);
    painter.galley(pill.center() - pill_text.size() / 2.0, pill_text, theme::ACCENT_TEXT);
}

/// One row per layer: visibility, lock and name, the active layer marked by
/// an accent edge the way the active tool is underlined. Clicking the name
/// selects the layer; the Layers panel keeps everything else.
fn header_ui(state: &mut AppState, ui: &mut egui::Ui, g: &Geo, header: Rect, id: Id) {
    let painter = ui.painter_at(header);
    let cur = state.project.current_layer;
    for layer in (0..g.n_layers).rev() {
        let y = g.row_y(layer);
        let row = Rect::from_min_max(pos2(header.min.x, y), pos2(header.max.x, y + ROW_H));
        if !row.intersects(header) {
            continue;
        }
        let active = layer == cur;
        if active {
            painter.rect_filled(row, 0.0, active_tint());
            let edge = Rect::from_min_size(
                pos2(row.min.x + 3.0, row.min.y + 5.0),
                vec2(2.5, ROW_H - 10.0),
            );
            painter.rect_filled(edge, 1.25, theme::ACCENT);
        }
        painter.hline(row.x_range(), row.max.y, Stroke::new(1.0, ROW_LINE));

        let eye = Rect::from_center_size(pos2(row.min.x + 20.0, row.center().y), vec2(20.0, 20.0));
        let lock = eye.translate(vec2(21.0, 0.0));
        let name = Rect::from_min_max(pos2(lock.max.x + 5.0, row.min.y), pos2(row.max.x - 6.0, row.max.y));

        // Interaction rects are clipped by hand: `ui.interact` would let a
        // row scrolled out of the header take clicks meant for the ruler.
        let hit = |rect: Rect, salt: &str| -> Option<egui::Response> {
            let r = rect.intersect(header);
            r.is_positive()
                .then(|| ui.interact(r, id.with((salt, layer)), Sense::click()))
        };
        let l = &state.project.layers[layer];
        let (visible, locked, reference) = (l.visible, l.locked, l.reference);
        let name_text = l.name.clone();

        // Resting states are quiet; the unusual one (hidden, locked) stands
        // out, since that's the one that explains why a layer won't draw.
        let icon = |rect: Rect, glyph: &str, loud: bool, hovered: bool| {
            if hovered {
                painter.rect_filled(rect, 4.0, theme::BG_HOVER);
            }
            let color = if hovered || loud {
                theme::TEXT
            } else {
                theme::TEXT_MUTED.gamma_multiply(0.6)
            };
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                glyph,
                egui::FontId::proportional(13.0),
                color,
            );
        };
        if let Some(r) = hit(eye, "eye") {
            let r = r.on_hover_text(if visible { "Hide layer" } else { "Show layer" });
            icon(eye, if visible { ic::EYE } else { ic::EYE_SLASH }, !visible, r.hovered());
            if r.clicked() {
                state.project.layers[layer].visible = !visible;
            }
        }
        if let Some(r) = hit(lock, "lock") {
            let r = r.on_hover_text(if locked { "Unlock layer" } else { "Lock layer" });
            let glyph = if locked { ic::LOCK_SIMPLE } else { ic::LOCK_SIMPLE_OPEN };
            icon(lock, glyph, locked, r.hovered());
            if r.clicked() {
                state.project.layers[layer].locked = !locked;
            }
        }
        if let Some(r) = hit(name, "name") {
            let color = if active || r.hovered() {
                theme::TEXT
            } else {
                theme::TEXT_MUTED
            };
            let color = if !visible || reference {
                color.gamma_multiply(0.6)
            } else {
                color
            };
            // One line, cut with an ellipsis rather than clipped mid-letter.
            let mut job = egui::text::LayoutJob::simple_singleline(
                name_text,
                egui::FontId::proportional(12.5),
                color,
            );
            job.wrap = egui::text::TextWrapping {
                max_width: name.width(),
                max_rows: 1,
                break_anywhere: true,
                overflow_character: Some('…'),
            };
            let galley = painter.layout_job(job);
            let at = pos2(name.min.x, row.center().y - galley.size().y / 2.0);
            painter.galley(at, galley, color);
            if r.clicked() {
                state.project.current_layer = layer;
            }
        }
    }
}

/// The well's edges, drawn over everything inside it: its outline, and the
/// hairlines under the ruler and beside the names.
fn well_lines(ui: &egui::Ui, outer: Rect, header: Rect, ruler: Rect) {
    let painter = ui.painter();
    let line = Stroke::new(1.0, theme::STROKE_THIN);
    painter.hline((outer.min.x + 1.0)..=(outer.max.x - 1.0), ruler.max.y, line);
    painter.vline(header.max.x, (outer.min.y + 1.0)..=(outer.max.y - 1.0), line);
    painter.rect_stroke(outer, WELL_ROUNDING, line);
}

/// Thin scroll thumbs along the bottom and right edges, only while there is
/// more to see. The plain wheel scrubs here, so these are the one way to
/// scroll the tracks without a modifier key.
fn scrollbars(state: &mut AppState, ui: &mut egui::Ui, g: &Geo, id: Id) {
    let content = vec2(
        (g.frame_count as f32 + END_SLACK) * g.fw,
        g.n_layers as f32 * ROW_H,
    );
    let view = g.tracks.size();
    let both = content.x > view.x && content.y > view.y;
    for axis in 0..2 {
        let (c, v) = (content[axis], view[axis]);
        if c <= v + 0.5 {
            continue;
        }
        // Leave the corner free when both show, so they never cross.
        let corner = if both { THUMB + 4.0 } else { 0.0 };
        let t = g.tracks;
        let track = if axis == 0 {
            Rect::from_min_max(
                pos2(t.min.x + 4.0, t.max.y - THUMB - 2.0),
                pos2(t.max.x - 4.0 - corner, t.max.y - 2.0),
            )
        } else {
            Rect::from_min_max(
                pos2(t.max.x - THUMB - 2.0, t.min.y + 4.0),
                pos2(t.max.x - 2.0, t.max.y - 4.0 - corner),
            )
        };
        let span = track.size()[axis];
        let len = (v / c * span).clamp(24.0_f32.min(span), span);
        let max_scroll = c - v;
        let frac = (state.track_scroll[axis] / max_scroll).clamp(0.0, 1.0);
        let from = track.min[axis] + frac * (span - len);
        let thumb = if axis == 0 {
            Rect::from_min_size(pos2(from, track.min.y), vec2(len, THUMB))
        } else {
            Rect::from_min_size(pos2(track.min.x, from), vec2(THUMB, len))
        };
        let resp = ui.interact(thumb.expand(3.0), id.with(("thumb", axis)), Sense::drag());
        if resp.dragged() {
            let d = resp.drag_delta()[axis];
            state.track_scroll[axis] += d * max_scroll / (span - len).max(1.0);
        }
        let alpha = if resp.dragged() {
            0.75
        } else if resp.hovered() {
            0.55
        } else {
            0.3
        };
        ui.painter()
            .rect_filled(thumb, THUMB / 2.0, theme::TEXT_MUTED.gamma_multiply(alpha));
    }
}

/// One bar ready to paint.
struct BarPaint {
    layer: usize,
    start: usize,
    end: usize,
    cell: CellId,
    /// Drawn from a previewed retime rather than the real sheet.
    preview: bool,
}

fn paint(
    state: &mut AppState,
    ui: &egui::Ui,
    g: &Geo,
    target: Option<Target>,
    copy: bool,
    hover: Option<Hit>,
) {
    let painter = ui.painter_at(g.tracks);
    let fc = g.frame_count;
    let cur_layer = state.project.current_layer;
    for layer in 0..g.n_layers {
        let row = g.row(layer);
        if !row.intersects(g.tracks) {
            continue;
        }
        if layer == cur_layer {
            painter.rect_filled(row, 0.0, active_tint());
        }
        painter.hline(row.x_range(), row.max.y, Stroke::new(1.0, ROW_LINE));
    }

    // Past the end of the timeline: shaded, with a hairline where it stops.
    let end_x = g.x(fc as f32);
    if end_x < g.tracks.max.x {
        let shade = Rect::from_min_max(pos2(end_x, g.tracks.min.y), g.tracks.max);
        let rounding = egui::Rounding {
            se: WELL_ROUNDING - 1.0,
            ..Default::default()
        };
        painter.rect_filled(shade, rounding, Color32::from_black_alpha(40));
        painter.vline(end_x, g.tracks.y_range(), Stroke::new(1.0, theme::STROKE_THIN));
    }

    // The playhead's column, under the bars.
    let cur = state.project.current_frame;
    let column = Rect::from_min_max(
        pos2(g.x(cur as f32), g.tracks.min.y),
        pos2(g.x(cur as f32 + 1.0), g.tracks.max.y),
    );
    painter.rect_filled(column, 0.0, theme::ACCENT.gamma_multiply(0.08));

    // Gather first: `cell_is_blank` needs the state mutably.
    let mut todo: Vec<BarPaint> = Vec::new();
    for layer in 0..g.n_layers {
        if !g.row(layer).intersects(g.tracks) {
            continue;
        }
        let preview = match target {
            Some(Target::Edge { block, new_len, .. }) if block.layer == layer => {
                let mut l = state.project.layers[layer].clone();
                layer_retime(&mut l, fc, block, new_len, || PREVIEW_BLANK);
                Some(l.exposures)
            }
            _ => None,
        };
        let is_preview = preview.is_some();
        let exposures = preview.unwrap_or_else(|| state.project.layers[layer].exposures.clone());
        for (start, end, cell) in bars(&exposures) {
            if g.x(end as f32) < g.tracks.min.x || g.x(start as f32) > g.tracks.max.x {
                continue;
            }
            todo.push(BarPaint {
                layer,
                start,
                end,
                cell,
                preview: is_preview,
            });
        }
    }

    let moving = match target {
        Some(Target::Body { moved, .. }) => moved && !copy,
        _ => false,
    };
    let mut pending_scan = false;
    for b in &todo {
        let blank = if b.cell == PREVIEW_BLANK {
            true
        } else {
            match state.cell_is_blank(b.cell) {
                Some(blank) => blank,
                None => {
                    pending_scan = true;
                    false
                }
            }
        };
        let selected = !b.preview && state.track_sel.contains(&(b.layer, b.start));
        let hovered = hover
            .and_then(|h| h.block)
            .is_some_and(|h| h.layer == b.layer && h.start == b.start);
        let rect = g.bar(b.layer, b.start, b.end);
        let dim = if moving && selected { 0.4 } else { 1.0 };
        let (fill, stroke) = match (selected, blank) {
            (true, _) => (
                if hovered { theme::ACCENT_HOVER } else { theme::ACCENT }.gamma_multiply(dim),
                Stroke::NONE,
            ),
            (false, false) => (
                if hovered { BAR_HOVER } else { BAR }.gamma_multiply(dim),
                Stroke::NONE,
            ),
            (false, true) => (
                Color32::from_white_alpha(if hovered { 14 } else { 6 }),
                Stroke::new(1.0, theme::TEXT_MUTED.gamma_multiply(0.35 * dim)),
            ),
        };
        painter.rect(rect, BAR_ROUNDING, fill, stroke);

        // The key: a filled dot for a drawing, a ring for a blank.
        let r = 2.5f32.min(rect.width() / 2.0 - 1.0).max(1.0);
        let dot = if rect.width() >= 12.0 {
            pos2(rect.min.x + 6.5, rect.center().y)
        } else {
            rect.center()
        };
        let ink = if selected {
            theme::ACCENT_TEXT
        } else {
            theme::TEXT.gamma_multiply(0.9 * dim)
        };
        if blank {
            painter.circle_stroke(dot, r, Stroke::new(1.0, ink.gamma_multiply(0.8)));
        } else {
            painter.circle_filled(dot, r, ink);
        }

        // Hold length, where there's room to read it. The last drawing holds
        // to the end, so its length is the timeline's, not a timing choice.
        let open = b.end >= fc && !b.preview;
        if rect.width() >= 36.0 && !open {
            let color = if selected {
                theme::ACCENT_TEXT.gamma_multiply(0.85)
            } else {
                theme::TEXT_MUTED
            };
            painter.text(
                pos2(rect.max.x - 5.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                format!("{}", b.end - b.start),
                egui::FontId::proportional(10.0),
                color,
            );
        }

        // On the retime handle: a grip, so the edge reads as something to pull.
        if hovered && hover.is_some_and(|h| h.edge) {
            painter.vline(
                rect.max.x - 2.5,
                (rect.min.y + 3.0)..=(rect.max.y - 3.0),
                Stroke::new(1.5, if selected { theme::ACCENT_TEXT } else { theme::TEXT }),
            );
        }
    }
    if pending_scan {
        ui.ctx().request_repaint();
    }

    // Transform keys, per layer, in the gap under each row's bars.
    for layer in 0..g.n_layers {
        let row = g.row(layer);
        if !row.intersects(g.tracks) {
            continue;
        }
        for k in &state.project.layers[layer].transform_keys {
            if k.frame >= fc {
                continue;
            }
            let c = pos2(g.x(k.frame as f32 + 0.5), row.max.y - 2.5);
            let r = 2.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(c.x, c.y - r),
                    pos2(c.x + r, c.y),
                    pos2(c.x, c.y + r),
                    pos2(c.x - r, c.y),
                ],
                KEY_LAYER,
                Stroke::NONE,
            ));
        }
    }

    // Where a move would land.
    if let Some(Target::Body {
        dl,
        df,
        moved: true,
        ok,
        ..
    }) = target
    {
        let color = if ok { theme::ACCENT } else { DIAG_BAD };
        for b in state.project.blocks_keyed_at(&state.track_selection()) {
            let tl = (b.layer as isize + dl) as usize;
            let ts = (b.start as isize + df) as usize;
            let span = state.project.landing_span(tl, ts, b);
            let rect = g.bar(tl, span.start, span.end).expand(1.5);
            painter.rect(
                rect,
                BAR_ROUNDING + 1.0,
                color.gamma_multiply(0.15),
                Stroke::new(1.5, color),
            );
        }
    }

    // The playhead itself, over everything.
    let x = g.x(cur as f32 + 0.5);
    painter.vline(x, g.tracks.y_range(), Stroke::new(1.0, theme::ACCENT));
}

fn drag_label(state: &AppState, t: Target, copy: bool) -> Option<String> {
    match t {
        Target::Body {
            dl,
            df,
            moved: true,
            ok,
            ..
        } => {
            let n = state.track_sel.len();
            if !ok {
                return Some("Can't drop on a locked or reference layer".into());
            }
            let what = if n == 1 { "drawing".to_string() } else { format!("{n} drawings") };
            let verb = if copy { "Copy" } else { "Move" };
            let mut s = format!("{verb} {what}");
            if df != 0 {
                s.push_str(&format!("  {df:+} fr"));
            }
            if dl != 0 {
                s.push_str(&format!("  {dl:+} layer"));
            }
            Some(s)
        }
        Target::Edge {
            old_len, new_len, ..
        } => Some(format!("{old_len} → {new_len} fr")),
        _ => None,
    }
}

/// A small label beside the pointer. Painted straight onto a tooltip layer
/// rather than laid out as widgets: widgets are hit-tested, and one sitting
/// under the pointer for even a frame would take the press meant for a bar.
fn pointer_label(ctx: &egui::Context, id: Id, p: Pos2, text: String) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, id.with("pointer_label")));
    let galley = painter.layout(text, egui::FontId::proportional(11.0), theme::TEXT, 360.0);
    let pad = vec2(6.0, 4.0);
    let size = galley.size() + pad * 2.0;
    let screen = ctx.screen_rect();
    // Below-right of the pointer, flipped above it at the bottom of the
    // screen — where the Timeline usually sits.
    let mut min = p + vec2(14.0, 12.0);
    if min.y + size.y > screen.max.y {
        min.y = p.y - 12.0 - size.y;
    }
    min.x = min.x.min(screen.max.x - size.x);
    let rect = Rect::from_min_size(min, size);
    painter.rect(rect, 4.0, theme::BG_PANEL, Stroke::new(1.0, theme::STROKE_THIN));
    painter.galley(rect.min + pad, galley, theme::TEXT);
}

fn hover_text(state: &AppState, b: Block) -> String {
    let fc = state.project.frame_count;
    let span = b.span(fc);
    let cell = state.project.layers[b.layer].exposures[b.start].unwrap_or_default();
    let length = match b.len {
        Some(n) => format!("{n} fr"),
        None => "holds to the end".into(),
    };
    format!(
        "Cell {cell} · frames {}–{} · {length}\n\
         Drag to move, Ctrl-drag to copy. Drag the right edge to retime.",
        span.start,
        span.end - 1,
    )
}

fn context_menu(state: &mut AppState, ui: &mut egui::Ui) {
    let has_sel = !state.track_sel.is_empty();
    let del = combo_text(state, Action::TrackClear);
    let close = combo_text(state, Action::TrackCloseGap);
    ui.add_enabled_ui(has_sel, |ui| {
        ui.menu_button("Timing", |ui| {
            for n in 1..=4 {
                if ui.button(format!("On {n}s")).clicked() {
                    state.set_track_timing(n);
                    ui.close_menu();
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut state.track_timing_n)
                        .range(1..=99)
                        .prefix("On ")
                        .suffix("s"),
                );
                if ui.button("Apply").clicked() {
                    let n = state.track_timing_n;
                    state.set_track_timing(n);
                    ui.close_menu();
                }
            });
        })
        .response
        .on_hover_text("Give every selected drawing the same hold; the ones after slide");
        if ui
            .add(egui::Button::new("Delete").shortcut_text(del))
            .on_hover_text("Leave the frames blank — nothing else moves")
            .clicked()
        {
            state.clear_track_selection();
            ui.close_menu();
        }
        if ui
            .add(egui::Button::new("Delete and close gap").shortcut_text(close))
            .on_hover_text("Remove the frames — later drawings on the layer slide up")
            .clicked()
        {
            state.close_track_selection();
            ui.close_menu();
        }
    });
    ui.separator();
    let n = state.frame_step_count();
    let frames = if n == 1 {
        "a frame".to_string()
    } else {
        format!("{n} frames")
    };
    if ui
        .button(format!("Insert {frames} on this layer"))
        .on_hover_text("The drawing here holds longer; later ones on this layer slide")
        .clicked()
    {
        state.insert_layer_frames_here(n);
        ui.close_menu();
    }
    if ui
        .button(format!("Remove {frames} on this layer"))
        .on_hover_text("Later drawings on this layer slide up")
        .clicked()
    {
        state.remove_layer_frames_here(n);
        ui.close_menu();
    }
    ui.separator();
    let cut = combo_text(state, Action::CellCut);
    let copy = combo_text(state, Action::CellCopy);
    let paste = combo_text(state, Action::CellPaste);
    if ui.add(egui::Button::new("Cut drawing").shortcut_text(cut)).clicked() {
        state.cut_cell();
        ui.close_menu();
    }
    if ui.add(egui::Button::new("Copy drawing").shortcut_text(copy)).clicked() {
        state.cell_clip = state.project.copy_active_cell();
        ui.close_menu();
    }
    let can_paste = state.cell_clip.is_some();
    if ui
        .add_enabled(can_paste, egui::Button::new("Paste drawing").shortcut_text(paste))
        .clicked()
    {
        state.paste_cell();
        ui.close_menu();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::project::Project;

    /// One layer, a distinct drawing keyed on each of `keys`, marked with
    /// its key frame + 1 in its first byte (alpha set, so it is not blank).
    fn sheet(frames: usize, keys: &[usize]) -> Project {
        let mut p = Project::new(4, 4, 12.0);
        p.ensure_frame_count(frames);
        p.layers[0].exposures = vec![None; frames];
        for &f in keys {
            let id = p.alloc_cell_for(0);
            p.cells[id].pixels[0] = f as u8 + 1;
            p.cells[id].pixels[3] = 255;
            p.layers[0].set_key(f, id);
        }
        p
    }

    /// The mark of the drawing showing on every frame of `layer`.
    fn row(p: &Project, layer: usize) -> Vec<u8> {
        (0..p.frame_count)
            .map(|f| p.layers[layer].resolve(f).map_or(0, |id| p.cells[id].pixels[0]))
            .collect()
    }

    /// Run one frame of the tracks, laid out from the top-left corner.
    fn frame(ctx: &egui::Context, state: &mut AppState, events: Vec<egui::Event>, modifiers: egui::Modifiers) {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 300.0))),
            events,
            modifiers,
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none())
                .show(ctx, |ui| show(state, ui));
        });
    }

    /// Centre of `frame` on the top row, at the default zoom with no scroll.
    fn at(frame: usize, row: usize) -> Pos2 {
        pos2(
            HEADER_W + (frame as f32 + 0.5) * DEFAULT_FRAME_W,
            RULER_H + (row as f32 + 0.5) * ROW_H,
        )
    }

    /// Press at `from`, drag through to `to`, release — each step its own
    /// frame, as a real drag arrives.
    fn drag(state: &mut AppState, from: Pos2, to: Pos2, modifiers: egui::Modifiers) {
        let ctx = egui::Context::default();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        frame(&ctx, state, vec![], modifiers);
        frame(&ctx, state, vec![egui::Event::PointerMoved(from), button(from, true)], modifiers);
        for i in 1..=4 {
            let p = from + (to - from) * (i as f32 / 4.0);
            frame(&ctx, state, vec![egui::Event::PointerMoved(p)], modifiers);
        }
        frame(&ctx, state, vec![button(to, false)], modifiers);
        frame(&ctx, state, vec![], modifiers);
    }

    #[test]
    fn a_plain_drag_moves_the_drawing() {
        let mut state = AppState::for_test();
        state.project = sheet(3, &[0, 1, 2]);
        drag(&mut state, at(0, 0), at(2, 0), egui::Modifiers::NONE);
        assert_eq!(row(&state.project, 0), vec![0, 2, 1], "A moved onto C, blank behind");
    }

    #[test]
    fn a_ctrl_drag_copies_the_drawing() {
        let mut state = AppState::for_test();
        state.project = sheet(3, &[0, 1, 2]);
        drag(&mut state, at(0, 0), at(2, 0), egui::Modifiers::COMMAND);
        assert_eq!(row(&state.project, 0), vec![1, 2, 1]);
    }

    /// How many frames of `layer` show the drawing marked `mark`.
    fn frames_showing(p: &Project, layer: usize, mark: u8) -> usize {
        row(p, layer).iter().filter(|&&m| m == mark).count()
    }

    /// Every plain drag of every drawing, grabbed at every one of its frames,
    /// to every frame of both layers: a move never leaves the drawing showing
    /// on more frames than it had.
    #[test]
    fn no_plain_drag_ever_duplicates() {
        // Bottom layer: drawings marked 1, 3, 6, the last holding to the end.
        // Top layer: two drawings marked 50 and 51.
        let base = || {
            let mut p = sheet(8, &[0, 2, 5]);
            p.add_layer();
            for (f, mark) in [(0, 50u8), (4, 51)] {
                let id = p.alloc_cell_for(1);
                p.cells[id].pixels[0] = mark;
                p.cells[id].pixels[3] = 255;
                p.layers[1].set_key(f, id);
            }
            p
        };
        // Row on screen for a layer: the top layer is drawn first.
        let row_of = |layer: usize| 1 - layer;
        let probe = base();
        let fc = probe.frame_count;
        let mut bad = Vec::new();
        for from_layer in 0..2 {
            for b in probe.blocks(from_layer) {
                let id = probe.layers[from_layer].exposures[b.start].unwrap();
                let mark = probe.cells[id].pixels[0];
                let before = frames_showing(&probe, 0, mark) + frames_showing(&probe, 1, mark);
                for grab in b.span(fc) {
                    for to_layer in 0..2 {
                        for to in 0..fc {
                            let mut state = AppState::for_test();
                            state.project = base();
                            let (from, dest) = (at(grab, row_of(from_layer)), at(to, row_of(to_layer)));
                            drag(&mut state, from, dest, egui::Modifiers::NONE);
                            let p = &state.project;
                            let after = frames_showing(p, 0, mark) + frames_showing(p, 1, mark);
                            if after > before {
                                bad.push(format!(
                                    "drawing {mark} {:?} on L{from_layer}, grab {grab} -> L{to_layer} f{to}:                                      {before} -> {after} frames  L0 {:?} L1 {:?}",
                                    b.span(fc),
                                    row(p, 0),
                                    row(p, 1)
                                ));
                            }
                        }
                    }
                }
            }
        }
        assert!(bad.is_empty(), "{} drags spread a drawing:
{}", bad.len(), bad.join("
"));
    }

    /// Resting on a bar until its details show, then dragging it, still
    /// drags. egui's at-pointer tooltip used to appear over the pointer and
    /// take the press.
    #[test]
    fn a_drag_after_resting_on_a_bar_still_moves_it() {
        let mut state = AppState::for_test();
        state.project = sheet(3, &[0, 1, 2]);
        let ctx = egui::Context::default();
        let mut t = 0.0;
        let mut step = |state: &mut AppState, events: Vec<egui::Event>| {
            t += 0.1;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 300.0))),
                time: Some(t),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::none())
                    .show(ctx, |ui| show(state, ui));
            });
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let (from, to) = (at(0, 0), at(2, 0));
        step(&mut state, vec![egui::Event::PointerMoved(from)]);
        for _ in 0..10 {
            step(&mut state, vec![]); // a second of resting
        }
        step(&mut state, vec![button(from, true)]);
        for i in 1..=4 {
            step(&mut state, vec![egui::Event::PointerMoved(from + (to - from) * (i as f32 / 4.0))]);
        }
        step(&mut state, vec![button(to, false)]);
        assert_eq!(row(&state.project, 0), vec![0, 2, 1]);
    }
}
