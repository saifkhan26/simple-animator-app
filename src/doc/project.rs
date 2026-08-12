//! Project — canvas size, fps, cell pool, layers, current edit cursor.
//!
//! Phase C model:
//!   * `cells`  — flat pool of pixel buffers, indexed by `CellId = usize`.
//!   * `layers` — ordered bottom-to-top. Each layer stores per-frame exposures.
//!   * `frame_count` — total timeline length; every layer.exposures matches.
//!   * `current_frame`, `current_layer` — the editing cursor.

use crate::doc::camera::{Camera, CameraKey};
use crate::doc::canvas::Canvas;
use crate::doc::layer::{CellId, Layer};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Project {
    /// Output resolution, and the size of the camera's frame rect. Layers may
    /// sit outside it — see `crate::doc::camera`.
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

    /// Live / static camera. Used directly when `camera_keys` is empty;
    /// otherwise it is the working buffer for the current frame, mirroring how
    /// `Layer::transform` relates to `Layer::transform_keys`.
    #[serde(default)]
    pub camera: Camera,
    /// Sorted camera keyframes. Empty = static `camera`.
    #[serde(default)]
    pub camera_keys: Vec<CameraKey>,
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
            camera: Camera::default(),
            camera_keys: Vec::new(),
        }
    }

    /// The camera showing on `frame`.
    pub fn resolve_camera(&self, frame: usize) -> Camera {
        Camera::resolve(&self.camera_keys, self.camera, frame)
    }

    pub fn has_camera_key(&self, frame: usize) -> bool {
        self.camera_keys.iter().any(|k| k.frame == frame)
    }

    /// Insert (or replace) a camera key at `frame`, keeping keys sorted. A
    /// replaced key keeps its existing ease.
    pub fn set_camera_key(&mut self, frame: usize, camera: Camera) {
        match self.camera_keys.iter_mut().find(|k| k.frame == frame) {
            Some(k) => k.camera = camera,
            None => {
                self.camera_keys.push(CameraKey {
                    frame,
                    camera,
                    ease: crate::doc::camera::Ease::default(),
                });
                self.camera_keys.sort_by_key(|k| k.frame);
            }
        }
    }

    pub fn delete_camera_key(&mut self, frame: usize) {
        self.camera_keys.retain(|k| k.frame != frame);
    }

    pub fn cell(&self, id: CellId) -> Option<&Canvas> {
        self.cells.get(id)
    }
    pub fn cell_mut(&mut self, id: CellId) -> Option<&mut Canvas> {
        self.cells.get_mut(id)
    }

    /// Allocates a new blank cell sized for the active layer.
    pub fn alloc_cell(&mut self) -> CellId {
        self.alloc_cell_for(self.current_layer)
    }

    /// Allocates a new blank cell sized for `layer` — layers with an expanded
    /// canvas get the bigger buffer.
    pub fn alloc_cell_for(&mut self, layer: usize) -> CellId {
        let (w, h) = match self.layers.get(layer) {
            Some(l) => l.cell_size(self.width, self.height),
            None => (self.width, self.height),
        };
        self.cells.push(Canvas::new(w, h));
        self.cells.len() - 1
    }

    /// Pixel size of the buffer a stroke on `(layer, frame)` will land in: the
    /// resolved cell's own size, or — when the slot has no cell yet — the size
    /// [`Project::alloc_cell_for`] is about to create.
    ///
    /// Input mapping must agree with allocation. When it didn't, the first
    /// stroke on a fresh oversized layer was mapped as if the cell were frame
    /// sized and then painted into a bigger one, landing offset by half the pad.
    pub fn draw_cell_size(&self, layer: usize, frame: usize) -> (u32, u32) {
        let Some(l) = self.layers.get(layer) else {
            return (self.width, self.height);
        };
        l.resolve(frame)
            .and_then(|id| self.cell(id))
            .map(|c| (c.width, c.height))
            .unwrap_or_else(|| l.cell_size(self.width, self.height))
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
            None => {
                let (w, h) = self.layers[cur_layer].cell_size(self.width, self.height);
                Canvas::new(w, h)
            }
        };
        self.cells.push(new_cell);
        let id = self.cells.len() - 1;
        self.layers[cur_layer].set_key(cur_frame, id);
        id
    }

    /// Every distinct cell id this layer's exposures reference.
    ///
    /// Safe to resize in place: no cell is ever shared between two layers —
    /// every site that keys a cell allocates (or clones) a fresh one first.
    pub fn layer_cell_ids(&self, layer: usize) -> Vec<CellId> {
        let Some(l) = self.layers.get(layer) else {
            return Vec::new();
        };
        let mut ids: Vec<CellId> = l.exposures.iter().flatten().copied().collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Grow (or shrink) this layer's drawable buffer to `new_w`×`new_h`,
    /// re-padding every cell it owns with the old pixels centered.
    ///
    /// Centering is what keeps the artwork visually still: `Transform` places a
    /// cell by its *center*, so a symmetric pad leaves the layer exactly where
    /// it was while giving strokes more room before the rasterizer clips them.
    pub fn expand_layer_canvas(&mut self, layer: usize, new_w: u32, new_h: u32) {
        let (new_w, new_h) = (new_w.max(1), new_h.max(1));
        for id in self.layer_cell_ids(layer) {
            if let Some(c) = self.cells.get_mut(id) {
                if c.width != new_w || c.height != new_h {
                    *c = recenter(c, new_w, new_h);
                }
            }
        }
        if let Some(l) = self.layers.get_mut(layer) {
            l.cell_w = new_w;
            l.cell_h = new_h;
        }
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
            l.track_insert_frame(insert_at);
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
            // Same cell shows, but the user hasn't tracked the new frame.
            l.track_insert_frame(insert_at);
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
                l.track_points.clear();
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
            l.track_remove_frame(f);
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

    /// Keep `lines_from` links valid after a layer is inserted at `at`:
    /// everything from `at` up shifted one slot higher.
    fn relink_after_insert(&mut self, at: usize) {
        for layer in &mut self.layers {
            if let Some(src) = layer.lines_from {
                if src >= at {
                    layer.lines_from = Some(src + 1);
                }
            }
        }
    }

    /// Keep `lines_from` links valid after the layer at `at` is removed:
    /// links to it are dropped, links above it shift one slot lower.
    pub fn relink_after_remove(&mut self, at: usize) {
        for layer in &mut self.layers {
            match layer.lines_from {
                Some(src) if src == at => layer.lines_from = None,
                Some(src) if src > at => layer.lines_from = Some(src - 1),
                _ => {}
            }
        }
    }

    /// Keep `lines_from` links valid after layers `a` and `b` swap places.
    fn relink_after_swap(&mut self, a: usize, b: usize) {
        for layer in &mut self.layers {
            if let Some(src) = layer.lines_from {
                if src == a {
                    layer.lines_from = Some(b);
                } else if src == b {
                    layer.lines_from = Some(a);
                }
            }
        }
    }

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
                    layer.track_insert_frame(layer.exposures.len() - 1);
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
        self.relink_after_insert(idx);
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
        self.relink_after_insert(0);
        self.current_layer += 1;
        0
    }

    pub fn delete_layer(&mut self) {
        if self.layers.len() <= 1 {
            return;
        }
        let i = self.current_layer;
        self.layers.remove(i);
        self.relink_after_remove(i);
        self.current_layer = self.current_layer.min(self.layers.len() - 1);
    }

    pub fn move_layer_up(&mut self) {
        let i = self.current_layer;
        if i + 1 < self.layers.len() {
            self.layers.swap(i, i + 1);
            self.relink_after_swap(i, i + 1);
            self.current_layer = i + 1;
        }
    }

    pub fn move_layer_down(&mut self) {
        let i = self.current_layer;
        if i > 0 {
            self.layers.swap(i, i - 1);
            self.relink_after_swap(i, i - 1);
            self.current_layer = i - 1;
        }
    }
}

/// Copy `src` into a fresh `new_w`×`new_h` buffer with the old content
/// centered. Content that no longer fits (a shrink) is cropped.
pub fn recenter(src: &Canvas, new_w: u32, new_h: u32) -> Canvas {
    let mut out = Canvas::new(new_w, new_h);
    // Offset of the old origin inside the new buffer; negative when shrinking.
    let ox = (new_w as i64 - src.width as i64) / 2;
    let oy = (new_h as i64 - src.height as i64) / 2;
    let y0 = (-oy).max(0);
    let y1 = (src.height as i64).min(new_h as i64 - oy);
    let x0 = (-ox).max(0);
    let x1 = (src.width as i64).min(new_w as i64 - ox);
    if x1 <= x0 || y1 <= y0 {
        return out;
    }
    let row_bytes = ((x1 - x0) * 4) as usize;
    for y in y0..y1 {
        let s = ((y * src.width as i64 + x0) * 4) as usize;
        let d = (((y + oy) * new_w as i64 + x0 + ox) * 4) as usize;
        out.pixels[d..d + row_bytes].copy_from_slice(&src.pixels[s..s + row_bytes]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A symmetric grow leaves the old pixels centered, which is what keeps a
    /// layer visually still (its transform is defined about the cell center).
    #[test]
    fn recenter_grows_symmetrically() {
        let mut src = Canvas::new(2, 2);
        src.pixels.copy_from_slice(&[
            1, 1, 1, 255, 2, 2, 2, 255, // row 0
            3, 3, 3, 255, 4, 4, 4, 255, // row 1
        ]);
        let out = recenter(&src, 4, 4);
        let at = |x: usize, y: usize| out.pixels[(y * 4 + x) * 4];
        assert_eq!((at(1, 1), at(2, 1), at(1, 2), at(2, 2)), (1, 2, 3, 4));
        assert_eq!(at(0, 0), 0, "pad is transparent");
        assert_eq!(out.pixels.len(), 4 * 4 * 4);
    }

    /// Input mapping sizes an unkeyed slot by the layer's own cell size, not
    /// the frame size — otherwise the first stroke on a fresh oversized layer
    /// is mapped for one buffer and painted into a bigger one.
    #[test]
    fn draw_cell_size_matches_what_alloc_will_create() {
        let mut p = Project::new(100, 50, 12.0);
        p.add_layer();
        let li = p.current_layer;
        p.expand_layer_canvas(li, 400, 50);
        // Nothing drawn yet: no cell exists on this slot.
        assert!(p.layers[li].resolve(p.current_frame).is_none());
        assert_eq!(p.draw_cell_size(li, p.current_frame), (400, 50));

        // Once the cell exists, its real size wins.
        let id = p.ensure_active_cell();
        assert_eq!((p.cells[id].width, p.cells[id].height), (400, 50));
        assert_eq!(p.draw_cell_size(li, p.current_frame), (400, 50));

        // A layer with no override still reports the frame size.
        assert_eq!(p.draw_cell_size(0, 0), (100, 50));
    }

    /// Shrinking crops around the center rather than panicking on the copy.
    #[test]
    fn recenter_shrinks_by_cropping() {
        let mut src = Canvas::new(4, 4);
        for (i, px) in src.pixels.chunks_mut(4).enumerate() {
            px[0] = i as u8;
            px[3] = 255;
        }
        let out = recenter(&src, 2, 2);
        // The 2x2 block starting at (1,1) of the source survives.
        assert_eq!(out.pixels[0], 5);
        assert_eq!(out.pixels[4], 6);
    }

    /// New cells on an expanded layer come out at the layer's size, not the
    /// project's — otherwise the next key would silently shrink back.
    #[test]
    fn expanded_layer_allocates_bigger_cells() {
        let mut p = Project::new(8, 6, 12.0);
        p.add_frame();
        p.expand_layer_canvas(0, 16, 12);
        assert_eq!(p.cells[0].width, 16);
        let id = p.insert_blank_key_here();
        assert_eq!((p.cells[id].width, p.cells[id].height), (16, 12));
    }
}
