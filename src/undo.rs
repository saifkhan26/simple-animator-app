//! Undo / redo command stack.
//!
//! Each `Command` captures a localised before/after pixel snapshot, so undoing
//! a full-canvas stroke only costs the dirty-rect area in memory.

use crate::doc::canvas::Canvas;
use crate::doc::layer::{CellId, Layer};
use crate::doc::project::Project;

/// Bounded undo capacity — prevents memory blow-up on long sessions.
const MAX_HISTORY: usize = 80;

/// What an undo/redo touched, so the caller knows what to re-upload.
pub enum Touched {
    /// A single cell changed — mark just that texture dirty.
    Cell(CellId),
    /// Timeline/layer structure changed — refresh every cell texture.
    All,
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
        }
    }

    fn restore(&self, project: &mut Project) {
        project.layers = self.layers.clone();
        project.frame_count = self.frame_count;
        project.current_frame = self.current_frame;
        project.current_layer = self.current_layer;
        project.loop_start = self.loop_start;
        project.loop_end = self.loop_end;
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
}

pub struct History {
    undo_stack: Vec<Command>,
    redo_stack: Vec<Command>,
}

impl Default for History {
    fn default() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }
}

impl History {
    pub fn push(&mut self, cmd: Command) {
        self.redo_stack.clear();
        if self.undo_stack.len() >= MAX_HISTORY {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(cmd);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Undo the last command. Returns what was touched (so the caller can
    /// mark it dirty for re-upload).
    pub fn undo(&mut self, project: &mut Project) -> Option<Touched> {
        let cmd = self.undo_stack.pop()?;
        let touched = apply(project, &cmd, false);
        self.redo_stack.push(cmd);
        Some(touched)
    }

    pub fn redo(&mut self, project: &mut Project) -> Option<Touched> {
        let cmd = self.redo_stack.pop()?;
        let touched = apply(project, &cmd, true);
        self.undo_stack.push(cmd);
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
            Touched::All
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
