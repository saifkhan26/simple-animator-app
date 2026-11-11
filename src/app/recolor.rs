//! Replace color: one colour swapped for another across drawings, the
//! command behind the Replace color window. The pixel work is
//! [`crate::tools::recolor`]; this decides which drawings and holds it to
//! the selection.
//!
//! A child of `app` for the same reason as `select`: it works on
//! `AppState`'s private state directly.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use super::AppState;
use crate::doc::canvas::Canvas;
use crate::doc::layer::CellId;
use crate::tools::recolor::{self, Recolor};

/// Which drawings a colour replace goes over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecolorSpan {
    /// The drawing each selected layer shows on this frame.
    #[default]
    Here,
    /// The drawings selected in the timeline, whichever layers they are on.
    Picked,
    /// Every drawing the selected layers show from `start` to `end`.
    Range,
    /// Every drawing on the selected layers.
    All,
}

/// Which colour well a screen pick fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Well {
    From,
    To,
}

/// The Replace color window and what it was last set to. Kept for the
/// session, not in the prefs: the colours belong to the picture at hand.
#[derive(Clone, Debug)]
pub struct RecolorCfg {
    pub show: bool,
    pub from: [u8; 3],
    pub to: [u8; 3],
    /// 0..=[`recolor::MAX_TOLERANCE`] per channel.
    pub tolerance: u8,
    pub span: RecolorSpan,
    /// The Range span's first and last frames, 0-based, both included.
    pub start: usize,
    pub end: usize,
    /// The well a screen pick is up for, while one is.
    pub picking: Option<Well>,
    /// Whether the window has been opened yet: the first opening takes the
    /// brush colour as the one to replace.
    opened: bool,
}

impl Default for RecolorCfg {
    fn default() -> Self {
        Self {
            show: false,
            from: [0; 3],
            to: [0; 3],
            tolerance: 16,
            span: RecolorSpan::default(),
            start: 0,
            end: 0,
            picking: None,
            opened: false,
        }
    }
}

impl AppState {
    /// Open the Replace color window. The first time, the colour to replace
    /// is the brush's — pick the old colour, then open it — and the range is
    /// the playback loop. Drawings picked in the timeline are what it goes
    /// over, when there are some.
    pub fn open_recolor(&mut self) {
        let c = &mut self.recolor;
        if !c.opened {
            let b = self.brush.color;
            c.from = [b[0], b[1], b[2]];
            c.to = c.from;
            c.start = self.project.loop_start;
            c.end = self.project.loop_end.saturating_sub(1);
            c.opened = true;
        }
        let last = self.project.frame_count.saturating_sub(1);
        c.end = c.end.min(last);
        c.start = c.start.min(c.end);
        if !self.track_sel.is_empty() {
            c.span = RecolorSpan::Picked;
        } else if c.span == RecolorSpan::Picked {
            c.span = RecolorSpan::Here;
        }
        c.show = true;
    }

    /// Start a screen pick that fills one of the window's colour wells
    /// instead of the brush.
    pub fn pick_recolor_well(&mut self, well: Well) {
        self.begin_screen_pick();
        self.recolor.picking = Some(well);
    }

    /// The layers a colour replace may change: the selected ones (a picked
    /// group stands for its layers) that show and may be edited. Bottom
    /// first.
    pub fn recolor_layers(&self) -> Vec<usize> {
        self.selected_drawing_layers()
            .into_iter()
            .filter(|&li| self.project.paintable(li) && self.project.shows(li))
            .collect()
    }

    /// The drawings a colour replace goes over, each once, with its layer
    /// and the first frame it shows on.
    pub fn recolor_targets(&self) -> Vec<(usize, usize, CellId)> {
        let c = &self.recolor;
        let p = &self.project;
        let last = p.frame_count.saturating_sub(1);
        let mut out: Vec<(usize, usize, CellId)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut push = |li: usize, f: usize, id: Option<CellId>| {
            if let Some(id) = id.filter(|&id| seen.insert(id)) {
                out.push((li, f, id));
            }
        };
        if c.span == RecolorSpan::Picked {
            for &(li, f) in &self.track_sel {
                if p.paintable(li) && p.shows(li) {
                    push(li, f, p.layers[li].exposures.get(f).copied().flatten());
                }
            }
            return out;
        }
        let frames = match c.span {
            RecolorSpan::Here => p.current_frame..=p.current_frame,
            RecolorSpan::Range => c.start.min(last)..=c.end.min(last),
            _ => 0..=last,
        };
        for li in self.recolor_layers() {
            for f in frames.clone() {
                push(li, f, p.layers[li].resolve(f));
            }
        }
        out
    }

    /// Replace the old colour with the new on every drawing the window
    /// says, held to the selection when there is one. One undo step; the
    /// drawings picked in the timeline stay picked. Says how it went.
    pub fn replace_color(&mut self) {
        // Floating pixels go down first: they are part of a drawing.
        self.land_float();
        let c = &self.recolor;
        let r = Recolor { from: c.from, to: c.to, tolerance: c.tolerance };
        let targets = self.recolor_targets();
        let bounded = self.sel_mask.as_ref().filter(|m| !m.is_all());
        let (pw, ph) = (self.project.width as f32, self.project.height as f32);
        let mut changed: Vec<(CellId, Arc<Canvas>)> = Vec::new();
        // Copies share their pixels until drawn on; recoloured once, they
        // go on sharing.
        let mut done: HashMap<*const Canvas, Option<Arc<Canvas>>> = HashMap::new();
        for &(li, f, id) in &targets {
            let Some(cell) = self.project.cells.get(id) else {
                continue;
            };
            let new = match bounded {
                Some(m) => {
                    let (cw, ch) = self.project.draw_cell_size(li, f);
                    let clip = m.to_cell(&self.display_transform(li, f), cw, ch, pw, ph);
                    recolor::recolored(cell, &r, Some(&clip)).map(Arc::new)
                }
                None => done
                    .entry(Arc::as_ptr(cell))
                    .or_insert_with(|| recolor::recolored(cell, &r, None).map(Arc::new))
                    .clone(),
            };
            if let Some(new) = new {
                changed.push((id, new));
            }
        }
        let n = changed.len();
        let msg = if targets.is_empty() {
            "Replace color: no drawing to change here".to_string()
        } else if n == 0 {
            "Replace color: that colour isn't in these drawings".to_string()
        } else {
            let picked = std::mem::take(&mut self.track_sel);
            self.structural_edit(true, |p| {
                for (id, canvas) in changed {
                    p.cells[id] = canvas;
                }
            });
            self.track_sel = picked;
            match n {
                1 => "Replaced the colour in 1 drawing".to_string(),
                n => format!("Replaced the colour in {n} drawings"),
            }
        };
        self.save_toast = Some((msg, Instant::now() + Self::TOAST_TTL));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::project::Project;

    const RED: [u8; 4] = [200, 30, 30, 255];
    const BLUE: [u8; 3] = [30, 60, 220];

    /// Two layers over 6 frames, drawings on 0 and 3 of each, every one a red
    /// square, the window set to turn red blue.
    fn scene() -> AppState {
        let mut p = Project::new(16, 16, 24.0);
        p.ensure_frame_count(6);
        p.add_layer();
        for li in 0..2 {
            p.layers[li].exposures.iter_mut().for_each(|e| *e = None);
            for f in [0, 3] {
                let id = p.alloc_cell_for(li);
                let mut full = vec![0u8; 16 * 16 * 4];
                for y in 2..10 {
                    for x in 2..10 {
                        let i = (y * 16 + x) * 4;
                        full[i..i + 4].copy_from_slice(&RED);
                    }
                }
                p.cells[id] = Arc::new(Canvas::packed_from(16, 16, &full));
                p.layers[li].set_key(f, id);
            }
        }
        let mut st = AppState::for_test();
        st.load_project(p, None);
        st.project.current_layer = 0;
        st.recolor.from = [RED[0], RED[1], RED[2]];
        st.recolor.to = BLUE;
        st
    }

    fn color(st: &AppState, li: usize, f: usize) -> [u8; 4] {
        let id = st.project.layers[li].resolve(f).unwrap();
        st.project.cells[id].px(4, 4)
    }

    fn is_blue(st: &AppState, li: usize, f: usize) -> bool {
        color(st, li, f) == [BLUE[0], BLUE[1], BLUE[2], 255]
    }

    #[test]
    fn here_is_the_drawing_on_this_frame_of_the_selected_layer() {
        let mut st = scene();
        st.project.current_frame = 4;
        st.replace_color();
        assert!(is_blue(&st, 0, 3), "the drawing held over frame 4");
        assert!(!is_blue(&st, 0, 0));
        assert!(!is_blue(&st, 1, 3), "another layer");
    }

    #[test]
    fn all_frames_on_every_selected_layer_in_one_undo_step() {
        let mut st = scene();
        let uid = st.project.layers[1].uid;
        st.selected_layers.insert(uid);
        st.recolor.span = RecolorSpan::All;
        let steps = st.history.undo_len();
        st.replace_color();
        assert!([(0, 0), (0, 3), (1, 0), (1, 3)].iter().all(|&(l, f)| is_blue(&st, l, f)));
        assert_eq!(st.history.undo_len(), steps + 1);
        st.undo();
        assert_eq!(color(&st, 1, 3), RED);
        assert_eq!(color(&st, 0, 0), RED);
    }

    #[test]
    fn a_range_takes_the_drawings_it_shows() {
        let mut st = scene();
        st.recolor.span = RecolorSpan::Range;
        (st.recolor.start, st.recolor.end) = (1, 2);
        st.replace_color();
        assert!(is_blue(&st, 0, 0), "held from frame 0 into the range");
        assert!(!is_blue(&st, 0, 3));
    }

    #[test]
    fn picked_drawings_go_whatever_layer_they_are_on_and_stay_picked() {
        let mut st = scene();
        st.track_sel = [(1, 3)].into_iter().collect();
        st.open_recolor();
        assert_eq!(st.recolor.span, RecolorSpan::Picked);
        st.recolor.from = [RED[0], RED[1], RED[2]];
        st.recolor.to = BLUE;
        st.replace_color();
        assert!(is_blue(&st, 1, 3));
        assert!(!is_blue(&st, 0, 3));
        assert_eq!(st.track_sel, [(1, 3)].into_iter().collect());
    }

    #[test]
    fn locked_and_hidden_layers_are_left_alone() {
        let mut st = scene();
        let uid = st.project.layers[1].uid;
        st.selected_layers.insert(uid);
        st.project.layers[0].locked = true;
        st.project.layers[1].visible = false;
        st.recolor.span = RecolorSpan::All;
        let steps = st.history.undo_len();
        st.replace_color();
        assert_eq!(st.history.undo_len(), steps);
        assert!(st.save_toast.as_ref().is_some_and(|(t, _)| t.contains("no drawing")));
    }

    #[test]
    fn a_copy_recoloured_goes_on_sharing_its_pixels() {
        let mut st = scene();
        let a = st.project.layers[0].exposures[0].unwrap();
        let b = st.project.layers[0].exposures[3].unwrap();
        st.project.cells[b] = Arc::clone(&st.project.cells[a]);
        st.recolor.span = RecolorSpan::All;
        st.replace_color();
        assert!(is_blue(&st, 0, 0) && is_blue(&st, 0, 3));
        assert!(Arc::ptr_eq(&st.project.cells[a], &st.project.cells[b]));
    }

    #[test]
    fn the_selection_holds_it_on_every_frame() {
        let mut st = scene();
        st.recolor.span = RecolorSpan::All;
        // The left half of the canvas.
        let mut cov = vec![0u8; 16 * 16];
        for y in 0..16 {
            for x in 0..8 {
                cov[y * 16 + x] = 255;
            }
        }
        st.sel_mask = Some(crate::tools::select_mask::SelectionMask { x: 0, y: 0, w: 16, h: 16, cov, outside: 0 });
        st.replace_color();
        for f in [0, 3] {
            let id = st.project.layers[0].resolve(f).unwrap();
            assert_eq!(st.project.cells[id].px(4, 4), [BLUE[0], BLUE[1], BLUE[2], 255]);
            assert_eq!(st.project.cells[id].px(9, 4), RED, "outside the selection");
        }
    }

    #[test]
    fn a_colour_that_isnt_there_says_so_and_records_nothing() {
        let mut st = scene();
        st.recolor.from = [0, 255, 0];
        let steps = st.history.undo_len();
        st.replace_color();
        assert_eq!(st.history.undo_len(), steps);
        assert!(st.save_toast.as_ref().is_some_and(|(t, _)| t.contains("isn't in")));
    }

    #[test]
    fn a_screen_pick_for_a_well_fills_it_and_leaves_the_brush() {
        let mut st = scene();
        let brush = st.brush.color;
        st.pick_recolor_well(Well::To);
        assert!(st.screen_pick);
        st.commit_screen_pick([1, 2, 3, 255]);
        assert_eq!(st.recolor.to, [1, 2, 3]);
        assert_eq!(st.brush.color, brush);
        assert!(!st.screen_pick && st.recolor.picking.is_none());
        // Cancelled, the next pick is the brush's again.
        st.pick_recolor_well(Well::From);
        st.end_screen_pick();
        assert!(st.recolor.picking.is_none());
    }
}
