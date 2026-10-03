//! Undo / redo command stack.
//!
//! Each `Command` captures a localised before/after pixel snapshot, so undoing
//! a full-canvas stroke only costs the dirty-rect area in memory.

use std::sync::Arc;

use crate::doc::camera::{Camera, CameraKey};
use crate::doc::canvas::Canvas;
use crate::doc::layer::{CellId, Layer};
use crate::doc::project::Project;
use crate::tools::perspective::PerspectiveGrid;
use crate::tools::select_mask::{PackedMask, SelectionMask};

/// Bounded undo capacity — prevents memory blow-up on long sessions.
const MAX_HISTORY: usize = 80;

/// What an undo/redo touched, so the caller knows what to re-upload.
pub enum Touched {
    /// One cell's pixels changed; its `Canvas::dirty` says where.
    Cell(CellId),
    /// These cells' pixels changed wholesale.
    Cells(Vec<CellId>),
    /// Only the timeline structure changed. Every pixel is where it was, so
    /// every texture is still good — on a big layer, re-uploading them all is
    /// what made undo stall.
    Structure,
    /// The selection became this.
    Selection(Option<SelectionMask>),
    /// A perspective grid's pose, keys or wall height went back to this.
    Grid(Box<PerspectiveGrid>),
    /// Several of the above, from a [`Command::Compound`].
    Many(Vec<Touched>),
}

/// Snapshot of the timeline structure (exposures + cursors), excluding the
/// heavy pixel buffers. Cheap to clone, so structural edits stay undoable
/// without blowing up history memory.
#[derive(Clone)]
pub struct TimelineState {
    pub layers: Vec<Layer>,
    pub frame_count: usize,
    pub current_frame: usize,
    pub current_layer: usize,
    pub loop_start: usize,
    pub loop_end: usize,
    pub camera: Camera,
    pub camera_keys: Vec<CameraKey>,
}

impl TimelineState {
    pub fn capture(project: &Project) -> Self {
        Self {
            layers: project.layers.clone(),
            frame_count: project.frame_count,
            current_frame: project.current_frame,
            current_layer: project.current_layer,
            loop_start: project.loop_start,
            loop_end: project.loop_end,
            camera: project.camera,
            camera_keys: project.camera_keys.clone(),
        }
    }

    fn restore(&self, project: &mut Project) {
        project.layers = self.layers.clone();
        project.frame_count = self.frame_count;
        project.current_frame = self.current_frame;
        project.current_layer = self.current_layer;
        project.loop_start = self.loop_start;
        project.loop_end = self.loop_end;
        project.camera = self.camera;
        project.camera_keys = self.camera_keys.clone();
    }
}

/// Full-buffer before/after for a cell whose pixels a structural edit wiped
/// (only the "delete the last remaining frame" path clears pixels).
pub struct CellPixelDelta {
    pub cell: CellId,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

pub enum Command {
    /// A paint operation on a single cell within a sub-rect.
    PixelPatch {
        cell: CellId,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        before: Vec<u8>,
        after: Vec<u8>,
    },
    /// A timeline/layer edit (add/duplicate/delete frame, insert/hold key,
    /// add/delete layer). Restores the structure wholesale; `cell_pixels`
    /// carries pixel buffers only when the edit also wiped cell contents.
    Structural {
        before: TimelineState,
        after: TimelineState,
        cell_pixels: Vec<CellPixelDelta>,
    },
    /// A layer's drawable buffer was resized, re-padding every cell it owns.
    /// No existing command covers this — `Structural` restores layers but not
    /// `project.cells`.
    ///
    /// Only the originals are stored — the very buffers the resize replaced,
    /// shared rather than copied — and redo re-runs the deterministic re-pad
    /// rather than keeping a second full copy of every cell.
    LayerCanvasResize {
        layer: usize,
        before_size: (u32, u32),
        after_size: (u32, u32),
        before: Vec<(CellId, Arc<Canvas>)>,
    },
    /// The selection changed. Only the selection: the pixels it covers are
    /// untouched, so this restores no cell.
    Selection {
        before: Option<PackedMask>,
        after: Option<PackedMask>,
    },
    /// A perspective grid moved, or its keys changed. Grids live on the app,
    /// not in the project, so this restores nothing here: the caller puts
    /// the snapshot back by the grid's id.
    Grid {
        before: Box<PerspectiveGrid>,
        after: Box<PerspectiveGrid>,
    },
    /// Several commands that undo and redo as one step — a float landing is
    /// its pixels *and* the selection that moved with them. Applied in order
    /// on redo, in reverse on undo.
    Compound(Vec<Command>),
}

#[derive(Default)]
pub struct History {
    undo_stack: Vec<Command>,
    redo_stack: Vec<Command>,
    /// Bumped by every push, undo and redo — whatever can change which cells
    /// the history still reaches. See [`History::for_each_cell`].
    revision: u64,
}

impl History {
    pub fn push(&mut self, cmd: Command) {
        self.redo_stack.clear();
        if self.undo_stack.len() >= MAX_HISTORY {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(cmd);
        self.revision += 1;
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Every cell an undo or redo could put back on a layer or write to:
    /// while any step still names a cell, its pixels must be kept.
    pub fn for_each_cell(&self, f: &mut impl FnMut(CellId)) {
        fn walk(cmd: &Command, f: &mut impl FnMut(CellId)) {
            match cmd {
                Command::PixelPatch { cell, .. } => f(*cell),
                Command::Structural {
                    before,
                    after,
                    cell_pixels,
                } => {
                    for state in [before, after] {
                        for l in &state.layers {
                            l.exposures.iter().flatten().for_each(|&id| f(id));
                        }
                    }
                    cell_pixels.iter().for_each(|d| f(d.cell));
                }
                Command::LayerCanvasResize { before, .. } => before.iter().for_each(|(id, _)| f(*id)),
                Command::Selection { .. } | Command::Grid { .. } => {}
                Command::Compound(cmds) => cmds.iter().for_each(|c| walk(c, f)),
            }
        }
        self.undo_stack.iter().chain(&self.redo_stack).for_each(|c| walk(c, f));
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    #[cfg(test)]
    pub fn undo_len(&self) -> usize {
        self.undo_stack.len()
    }

    /// Undo the last command. Returns what was touched (so the caller can
    /// mark it dirty for re-upload).
    pub fn undo(&mut self, project: &mut Project) -> Option<Touched> {
        let cmd = self.undo_stack.pop()?;
        let touched = apply(project, &cmd, false);
        self.redo_stack.push(cmd);
        self.revision += 1;
        Some(touched)
    }

    pub fn redo(&mut self, project: &mut Project) -> Option<Touched> {
        let cmd = self.redo_stack.pop()?;
        let touched = apply(project, &cmd, true);
        self.undo_stack.push(cmd);
        self.revision += 1;
        Some(touched)
    }
}

/// Apply a command in forward (`forward = true`) or reverse direction.
fn apply(project: &mut Project, cmd: &Command, forward: bool) -> Touched {
    match cmd {
        Command::PixelPatch {
            cell,
            x,
            y,
            w,
            h,
            before,
            after,
        } => {
            let bytes = if forward { after } else { before };
            if let Some(c) = project.cell_mut(*cell) {
                blit_subrect(c, *x, *y, *w, *h, bytes);
                // Exactly the patch, not unioned with whatever the last edit
                // left there: this is what gets re-uploaded.
                c.dirty = Some(crate::doc::canvas::DirtyRect {
                    min_x: *x,
                    min_y: *y,
                    max_x: x + w,
                    max_y: y + h,
                });
            }
            Touched::Cell(*cell)
        }
        Command::Structural {
            before,
            after,
            cell_pixels,
        } => {
            let state = if forward { after } else { before };
            state.restore(project);
            for d in cell_pixels {
                let bytes = if forward { &d.after } else { &d.before };
                if let Some(c) = project.cell_mut(d.cell) {
                    c.pixels.copy_from_slice(bytes);
                    c.dirty = Some(crate::doc::canvas::DirtyRect {
                        min_x: 0,
                        min_y: 0,
                        max_x: c.width,
                        max_y: c.height,
                    });
                }
            }
            if cell_pixels.is_empty() {
                Touched::Structure
            } else {
                Touched::Cells(cell_pixels.iter().map(|d| d.cell).collect())
            }
        }
        Command::LayerCanvasResize {
            layer,
            before_size,
            after_size,
            before,
        } => {
            if forward {
                project.expand_layer_canvas(*layer, after_size.0, after_size.1);
            } else {
                // Shared back, not copied: the history keeps its handle for a
                // redo. `Touched::Cells` tells the caller they changed whole.
                for (id, canvas) in before {
                    if let Some(c) = project.cells.get_mut(*id) {
                        *c = Arc::clone(canvas);
                    }
                }
                if let Some(l) = project.layers.get_mut(*layer) {
                    l.cell_w = before_size.0;
                    l.cell_h = before_size.1;
                }
            }
            Touched::Cells(project.layer_cell_ids(*layer))
        }
        Command::Selection { before, after } => {
            let m = if forward { after } else { before };
            Touched::Selection(m.as_ref().map(PackedMask::unpack))
        }
        Command::Grid { before, after } => {
            Touched::Grid(if forward { after.clone() } else { before.clone() })
        }
        Command::Compound(cmds) => {
            let mut out = Vec::with_capacity(cmds.len());
            if forward {
                for c in cmds {
                    out.push(apply(project, c, true));
                }
            } else {
                for c in cmds.iter().rev() {
                    out.push(apply(project, c, false));
                }
            }
            Touched::Many(out)
        }
    }
}

/// Copy `bytes` (length = w * h * 4) into `canvas` at (x, y).
fn blit_subrect(canvas: &mut Canvas, x: u32, y: u32, w: u32, h: u32, bytes: &[u8]) {
    let row_bytes = w as usize * 4;
    for row in 0..h as usize {
        let dst_off = ((y as usize + row) * canvas.width as usize + x as usize) * 4;
        let src_off = row * row_bytes;
        canvas.pixels[dst_off..dst_off + row_bytes]
            .copy_from_slice(&bytes[src_off..src_off + row_bytes]);
    }
}

/// Capture an RGBA8 sub-rect from `canvas` into a fresh owned buffer.
pub fn snapshot_subrect(canvas: &Canvas, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    let row_bytes = w as usize * 4;
    for row in 0..h as usize {
        let src_off = ((y as usize + row) * canvas.width as usize + x as usize) * 4;
        let dst_off = row * row_bytes;
        out[dst_off..dst_off + row_bytes]
            .copy_from_slice(&canvas.pixels[src_off..src_off + row_bytes]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::select_mask::SelectionMask;

    fn rect(x: i32, w: u32) -> SelectionMask {
        SelectionMask {
            x,
            y: 0,
            w,
            h: 1,
            cov: vec![255; w as usize],
            outside: 0,
        }
    }

    fn selection_of(t: Option<Touched>) -> Option<SelectionMask> {
        match t {
            Some(Touched::Selection(m)) => m,
            _ => panic!("not a selection change"),
        }
    }

    #[test]
    fn a_selection_step_undoes_and_redoes() {
        let mut p = Project::new(8, 8, 12.0);
        let mut h = History::default();
        h.push(Command::Selection {
            before: None,
            after: Some(rect(2, 3).pack()),
        });
        assert_eq!(selection_of(h.undo(&mut p)), None);
        assert_eq!(selection_of(h.redo(&mut p)), Some(rect(2, 3)));
    }

    /// A compound undoes its parts last-first, so a later part that depends on
    /// an earlier one is always taken back before it.
    #[test]
    fn a_compound_undoes_in_reverse() {
        let mut p = Project::new(8, 8, 12.0);
        let id = p.ensure_active_cell();
        let mut h = History::default();
        h.push(Command::Compound(vec![
            Command::PixelPatch {
                cell: id,
                x: 0,
                y: 0,
                w: 1,
                h: 1,
                before: vec![0, 0, 0, 0],
                after: vec![9, 9, 9, 255],
            },
            Command::Selection {
                before: Some(rect(0, 1).pack()),
                after: Some(rect(5, 1).pack()),
            },
        ]));
        match h.undo(&mut p) {
            Some(Touched::Many(v)) => {
                assert!(matches!(&v[0], Touched::Selection(Some(m)) if m.x == 0));
                assert!(matches!(v[1], Touched::Cell(c) if c == id));
            }
            _ => panic!("expected a compound"),
        }
        assert_eq!(&p.cells[id].pixels[..4], &[0, 0, 0, 0]);
        match h.redo(&mut p) {
            Some(Touched::Many(v)) => {
                assert!(matches!(v[0], Touched::Cell(_)));
                assert!(matches!(&v[1], Touched::Selection(Some(m)) if m.x == 5));
            }
            _ => panic!("expected a compound"),
        }
        assert_eq!(&p.cells[id].pixels[..4], &[9, 9, 9, 255]);
    }
}
