//! Undo / redo command stack.
//!
//! Each `Command` captures a localised before/after pixel snapshot, so undoing
//! a full-canvas stroke only costs the dirty-rect area in memory.

use crate::doc::canvas::Canvas;
use crate::doc::layer::CellId;
use crate::doc::project::Project;

/// Bounded undo capacity — prevents memory blow-up on long sessions.
const MAX_HISTORY: usize = 80;

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

    /// Undo the last command. Returns the CellId touched (so the caller can
    /// mark it dirty for re-upload).
    pub fn undo(&mut self, project: &mut Project) -> Option<CellId> {
        let cmd = self.undo_stack.pop()?;
        let touched = apply(project, &cmd, false);
        self.redo_stack.push(cmd);
        Some(touched)
    }

    pub fn redo(&mut self, project: &mut Project) -> Option<CellId> {
        let cmd = self.redo_stack.pop()?;
        let touched = apply(project, &cmd, true);
        self.undo_stack.push(cmd);
        Some(touched)
    }
}

/// Apply a command in forward (`forward = true`) or reverse direction.
fn apply(project: &mut Project, cmd: &Command, forward: bool) -> CellId {
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
            *cell
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
