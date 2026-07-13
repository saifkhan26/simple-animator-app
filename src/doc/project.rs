//! Project — canvas size, fps, cell pool, layers, current edit cursor.
//!
//! Phase C model:
//!   * `cells`  — flat pool of pixel buffers, indexed by `CellId = usize`.
//!   * `layers` — ordered bottom-to-top. Each layer stores per-frame exposures.
//!   * `frame_count` — total timeline length; every layer.exposures matches.
//!   * `current_frame`, `current_layer` — the editing cursor.

use crate::doc::canvas::Canvas;
use crate::doc::layer::{CellId, Layer};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Project {
    pub width: u32,
    pub height: u32,
    pub fps: f32,

    pub cells: Vec<Canvas>,
    pub layers: Vec<Layer>,
    pub frame_count: usize,

    pub current_frame: usize,
    pub current_layer: usize,

    pub loop_start: usize,
    pub loop_end: usize,
}

impl Project {
    pub fn new(width: u32, height: u32, fps: f32) -> Self {
        // Seed with one cell and one layer keyed on frame 0.
        let cells = vec![Canvas::new(width, height)];
        let mut layer = Layer::new("Layer 1", 1);
        layer.set_key(0, 0);
        Self {
            width,
            height,
            fps,
            cells,
            layers: vec![layer],
            frame_count: 1,
            current_frame: 0,
            current_layer: 0,
            loop_start: 0,
            loop_end: 1,
        }
    }

    pub fn cell(&self, id: CellId) -> Option<&Canvas> {
        self.cells.get(id)
    }
    pub fn cell_mut(&mut self, id: CellId) -> Option<&mut Canvas> {
        self.cells.get_mut(id)
    }

    /// Allocates a new blank cell and returns its CellId.
    pub fn alloc_cell(&mut self) -> CellId {
        self.cells.push(Canvas::new(self.width, self.height));
        self.cells.len() - 1
    }

    /// Resolved cell id for the currently active (layer, frame).
    pub fn resolved_current(&self) -> Option<CellId> {
        self.layers
            .get(self.current_layer)
            .and_then(|l| l.resolve(self.current_frame))
    }

    /// Ensures the active (layer, frame) slot has its own cell so painting
    /// won't accidentally overwrite a held cell shared with earlier frames.
    /// Returns the CellId of the active cell.
    ///
    /// Behaviour:
    ///   * Slot already keyed → return that CellId.
    ///   * Slot holds an earlier key → return the held CellId (paint into
    ///     shared). To break the hold, callers should use `insert_key` first.
    ///   * No prior key exists at all → allocate a new cell and key it here.
    pub fn ensure_active_cell(&mut self) -> CellId {
        let cur_layer = self.current_layer;
        let cur_frame = self.current_frame;
        let already = self.layers[cur_layer].resolve(cur_frame);
        match already {
            Some(id) => id,
            None => {
                let id = self.alloc_cell();
                self.layers[cur_layer].set_key(cur_frame, id);
                id
            }
        }
    }

    /// Force a new *empty* key at (current_layer, current_frame).
    /// Breaks any hold and starts with a blank cell.
    pub fn insert_blank_key_here(&mut self) -> CellId {
        let cur_layer = self.current_layer;
        let cur_frame = self.current_frame;
        let id = self.alloc_cell();
        self.layers[cur_layer].set_key(cur_frame, id);
        id
    }

    /// Force a new key at (current_layer, current_frame) that is a *copy* of
    /// the previously-resolved cell. Useful when you want to break a hold but
    /// keep the existing drawing as a starting point for tweaks.
    pub fn insert_duplicate_key_here(&mut self) -> CellId {
        let cur_layer = self.current_layer;
        let cur_frame = self.current_frame;
        let resolved = self.layers[cur_layer].resolve(cur_frame);
        let new_cell = match resolved {
            Some(src) => self.cells[src].clone(),
            None => Canvas::new(self.width, self.height),
        };
        self.cells.push(new_cell);
        let id = self.cells.len() - 1;
        self.layers[cur_layer].set_key(cur_frame, id);
        id
    }

    /// Hold the previous cell at the active slot (delete its key).
    pub fn hold_here(&mut self) {
        let cur_layer = self.current_layer;
        let cur_frame = self.current_frame;
        self.layers[cur_layer].hold(cur_frame);
    }

    // --- Timeline edits ---

    pub fn add_frame(&mut self) {
        let insert_at = (self.current_frame + 1).min(self.frame_count);
        for l in &mut self.layers {
            if insert_at >= l.exposures.len() {
                l.exposures.push(None);
            } else {
                l.exposures.insert(insert_at, None);
            }
        }
        self.frame_count += 1;
        self.current_frame = insert_at;
        self.loop_end = self.frame_count;
    }

    pub fn duplicate_frame(&mut self) {
        let insert_at = (self.current_frame + 1).min(self.frame_count);
        for l in &mut self.layers {
            let resolved = l.resolve(self.current_frame);
            l.exposures.insert(insert_at, resolved);
        }
        self.frame_count += 1;
        self.current_frame = insert_at;
        self.loop_end = self.frame_count;
    }

    pub fn delete_frame(&mut self) {
        if self.frame_count <= 1 {
            // Clear the only frame instead of removing it.
            for l in &mut self.layers {
                if !l.exposures.is_empty() {
                    l.exposures[0] = None;
                }
            }
            for c in &mut self.cells {
                c.clear();
            }
            return;
        }
        let f = self.current_frame;
        for l in &mut self.layers {
            if f < l.exposures.len() {
                l.exposures.remove(f);
            }
        }
        self.frame_count -= 1;
        self.current_frame = self.current_frame.min(self.frame_count - 1);
        self.loop_end = self.frame_count;
    }

    pub fn goto(&mut self, frame: usize) {
        if self.frame_count > 0 {
            self.current_frame = frame.min(self.frame_count - 1);
        }
    }

    pub fn step(&mut self, delta: isize) {
        if self.frame_count == 0 {
            return;
        }
        let n = self.frame_count as isize;
        let next = (self.current_frame as isize + delta).rem_euclid(n);
        self.current_frame = next as usize;
    }

    // --- Layer edits ---

    pub fn add_layer(&mut self) {
        let name = format!("Layer {}", self.layers.len() + 1);
        let layer = Layer::new(name, self.frame_count);
        self.layers.push(layer);
        self.current_layer = self.layers.len() - 1;
    }

    /// Grow the timeline to at least `n` frames, padding every layer with holds
    /// so all layers stay the same length. No-op if already long enough.
    pub fn ensure_frame_count(&mut self, n: usize) {
        if n > self.frame_count {
            let extra = n - self.frame_count;
            for layer in &mut self.layers {
                for _ in 0..extra {
                    layer.exposures.push(None);
                }
            }
            self.frame_count = n;
            self.loop_end = n;
        }
    }

    /// Insert a fresh, full-length layer directly *below* the active layer
    /// (lower index). The previously-active layer stays selected (its index
    /// shifts up by one). Returns the new layer's index.
    pub fn add_layer_below_active(&mut self, name: impl Into<String>) -> usize {
        let idx = self.current_layer.min(self.layers.len());
        let layer = Layer::new(name, self.frame_count);
        self.layers.insert(idx, layer);
        self.current_layer = idx + 1;
        idx
    }

    /// Insert a fresh, full-length layer at the very bottom of the stack
    /// (index 0, drawn behind everything) — a background. The previously-active
    /// layer stays selected (its index shifts up by one). Returns the new
    /// layer's index, always 0.
    pub fn add_background_layer(&mut self, name: impl Into<String>) -> usize {
        let layer = Layer::new(name, self.frame_count);
        self.layers.insert(0, layer);
        self.current_layer += 1;
        0
    }

    pub fn delete_layer(&mut self) {
        if self.layers.len() <= 1 {
            return;
        }
        self.layers.remove(self.current_layer);
        self.current_layer = self.current_layer.min(self.layers.len() - 1);
    }

    pub fn move_layer_up(&mut self) {
        let i = self.current_layer;
        if i + 1 < self.layers.len() {
            self.layers.swap(i, i + 1);
            self.current_layer = i + 1;
        }
    }

    pub fn move_layer_down(&mut self) {
        let i = self.current_layer;
        if i > 0 {
            self.layers.swap(i, i - 1);
            self.current_layer = i - 1;
        }
    }
}
