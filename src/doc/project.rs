//! Project — canvas size, fps, cell pool, layers, current edit cursor.
//!
//! Phase C model:
//!   * `cells`  — flat pool of pixel buffers, indexed by `CellId = usize`.
//!   * `layers` — ordered bottom-to-top. Each layer stores per-frame exposures.
//!   * `frame_count` — total timeline length; every layer.exposures matches.
//!   * `current_frame`, `current_layer` — the editing cursor.

use std::ops::Range;

use crate::doc::camera::{Camera, CameraKey};
use crate::doc::canvas::Canvas;
use crate::doc::layer::{CellId, Layer, TrackSample};

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

    /// The drawing showing at the active slot, deep-copied.
    ///
    /// A copy, not a `CellId`: cells are never shared between slots (see
    /// [`Project::layer_cell_ids`]), so handing out an id would let two frames
    /// alias one buffer the moment it was pasted.
    pub fn copy_active_cell(&self) -> Option<Canvas> {
        self.resolved_current().and_then(|id| self.cell(id)).cloned()
    }

    /// Take the drawing at the active slot, leaving the frame blank.
    ///
    /// The slot is keyed to a *fresh empty* cell rather than un-keyed: dropping
    /// the key would make the previous drawing hold through this frame, which
    /// looks like the cut went to the wrong frame. Neighbouring frames, other
    /// layers and `frame_count` are untouched, so the layers x frames grid stays
    /// rectangular.
    pub fn cut_active_cell(&mut self) -> Option<Canvas> {
        let taken = self.copy_active_cell()?;
        self.insert_blank_key_here();
        Some(taken)
    }

    /// Key a copy of `src` at the active slot, overwriting whatever was there.
    ///
    /// A drawing pasted into a layer whose cells are a different size is
    /// re-centred rather than stretched — [`recenter`] crops or pads, so line
    /// weight never changes on a paste.
    pub fn paste_cell_here(&mut self, src: &Canvas) -> CellId {
        let (w, h) = self.draw_cell_size(self.current_layer, self.current_frame);
        let cell = if (src.width, src.height) == (w, h) {
            src.clone()
        } else {
            recenter(src, w, h)
        };
        self.cells.push(cell);
        let id = self.cells.len() - 1;
        let (layer, frame) = (self.current_layer, self.current_frame);
        self.layers[layer].set_key(frame, id);
        id
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
            l.pins_insert_frames(insert_at, 1);
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
            l.pins_insert_frames(insert_at, 1);
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
            l.pins_remove_frames(f, 1);
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

    /// Move `delta` frames. With `wrap` the timeline is a ring, as it has
    /// always been; without it the ends are walls, so holding a step key or
    /// spinning the wheel past the last drawing stays there instead of
    /// reappearing at the top of the scene.
    pub fn step(&mut self, delta: isize, wrap: bool) {
        if self.frame_count == 0 {
            return;
        }
        let n = self.frame_count as isize;
        let raw = self.current_frame as isize + delta;
        let next = if wrap {
            raw.rem_euclid(n)
        } else {
            raw.clamp(0, n - 1)
        };
        self.current_frame = next as usize;
    }

    /// Frame of the nearest drawing key on the active layer strictly *before*
    /// the current frame, or `None` when there is none.
    ///
    /// Holds are skipped: on a scene animated on 2s or 3s most frames repeat an
    /// earlier drawing, so stepping by one lands on the same picture. This is
    /// what the timeline's jump-to-key buttons navigate by. `None` is the
    /// clamp — the caller greys its button out rather than wrapping.
    pub fn prev_key_frame(&self) -> Option<usize> {
        let layer = self.layers.get(self.current_layer)?;
        (0..self.current_frame.min(self.frame_count))
            .rev()
            .find(|&f| layer.is_key(f))
    }

    /// Frame of the nearest drawing key on the active layer strictly *after*
    /// the current frame, or `None` when there is none. See
    /// [`Project::prev_key_frame`].
    pub fn next_key_frame(&self) -> Option<usize> {
        let layer = self.layers.get(self.current_layer)?;
        (self.current_frame.saturating_add(1)..self.frame_count).find(|&f| layer.is_key(f))
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

    // --- Drawing blocks (the timeline tracks) ---
    //
    // Everything here only ever *allocates* cells — clones, blanks — and never
    // writes an existing cell's pixels, so the `TimelineState` snapshot that
    // `structural_edit` takes is a complete undo. Same contract as
    // `paste_cell_here`.
    //
    // The timeline's length is fixed unless a key would be pushed off its end;
    // only then does it grow, padding every other layer with holds. A layer's
    // last drawing holds to the end, so a ripple in front of it shortens or
    // lengthens that hold rather than moving the end.

    /// The drawing showing on `frame` and the frames it holds for. `None` before
    /// the layer's first key, where nothing shows.
    pub fn block_at(&self, layer: usize, frame: usize) -> Option<Block> {
        let l = self.layers.get(layer)?;
        let f = frame.min(self.frame_count.checked_sub(1)?);
        let start = (0..=f).rev().find(|&i| l.is_key(i))?;
        let next = (start + 1..self.frame_count).find(|&i| l.is_key(i));
        Some(Block {
            layer,
            start,
            len: next.map(|n| n - start),
        })
    }

    /// Every drawing on `layer`, in timeline order.
    pub fn blocks(&self, layer: usize) -> Vec<Block> {
        let Some(l) = self.layers.get(layer) else {
            return Vec::new();
        };
        let keys: Vec<usize> = (0..self.frame_count).filter(|&f| l.is_key(f)).collect();
        keys.iter()
            .enumerate()
            .map(|(i, &start)| Block {
                layer,
                start,
                len: keys.get(i + 1).map(|&n| n - start),
            })
            .collect()
    }

    /// Grow the timeline to `need` frames, padding every layer shorter than
    /// that with holds. Unlike [`Project::ensure_frame_count`] it leaves a
    /// layer that a ripple already lengthened alone.
    fn settle_length(&mut self, need: usize) {
        if need <= self.frame_count {
            return;
        }
        for l in &mut self.layers {
            while l.exposures.len() < need {
                l.exposures.push(None);
                l.track_insert_frame(l.exposures.len() - 1);
            }
        }
        self.frame_count = need;
        self.loop_end = need;
    }

    /// Insert `n` held frames on one layer right after `after`: the drawing
    /// showing there holds `n` frames longer, and later drawings slide.
    pub fn insert_layer_frames(&mut self, layer: usize, after: usize, n: usize) {
        let fc = self.frame_count;
        let Some(l) = self.layers.get_mut(layer) else {
            return;
        };
        layer_insert(l, after, n);
        let need = layer_fit(l, fc);
        self.settle_length(need);
    }

    /// Remove `n` frames from one layer starting at `at`; later drawings slide
    /// up and the layer's last drawing holds longer to fill the end. Whatever
    /// was showing right after the removed frames still shows there.
    pub fn remove_layer_frames(&mut self, layer: usize, at: usize, n: usize) {
        if let Some(l) = self.layers.get_mut(layer) {
            layer_remove(l, at, n);
        }
    }

    /// Hold `b` for `new_len` frames (at least one), sliding later drawings.
    pub fn retime_block(&mut self, b: Block, new_len: usize) {
        let Some(l) = self.layers.get(b.layer) else {
            return;
        };
        let (w, h) = l.cell_size(self.width, self.height);
        let fc = self.frame_count;
        let cells = &mut self.cells;
        let need = layer_retime(&mut self.layers[b.layer], fc, b, new_len, || {
            cells.push(Canvas::new(w, h));
            cells.len() - 1
        });
        self.settle_length(need);
    }

    /// Give every drawing keyed at `starts` (layer, frame) a hold of `n`
    /// frames. Returns where those drawings start afterwards.
    ///
    /// Left to right with a running shift: retiming a drawing moves every
    /// later one on its layer, so each start is re-found at its shifted spot.
    /// Going right to left instead would let a later insert truncate the
    /// already-retimed last drawing against the end of the timeline.
    pub fn set_timing(&mut self, starts: &[(usize, usize)], n: usize) -> Vec<(usize, usize)> {
        let n = n.max(1);
        let mut out = Vec::new();
        for layer in distinct_layers(starts) {
            let mut frames: Vec<usize> = starts
                .iter()
                .filter(|s| s.0 == layer)
                .map(|s| s.1)
                .collect();
            frames.sort_unstable();
            let mut shift: isize = 0;
            for f in frames {
                let at = (f as isize + shift).max(0) as usize;
                let Some(b) = self.block_at(layer, at).filter(|b| b.start == at) else {
                    continue;
                };
                let old = b.len.unwrap_or(self.frame_count - b.start);
                self.retime_block(b, n);
                shift += n as isize - old as isize;
                out.push((layer, at));
            }
        }
        out
    }

    /// Delete the drawings keyed at `starts`, leaving blank frames: each run
    /// of adjacent ones becomes one fresh blank key, and nothing else moves.
    /// Returns the blank runs' starts.
    ///
    /// One blank per run, never one shared by several runs: painting into a
    /// shared blank would paint every place it shows.
    pub fn clear_blocks(&mut self, starts: &[(usize, usize)]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for layer in distinct_layers(starts) {
            let mut blocks: Vec<Block> = starts
                .iter()
                .filter(|s| s.0 == layer)
                .filter_map(|&(_, f)| self.block_at(layer, f).filter(|b| b.start == f))
                .collect();
            blocks.sort_by_key(|b| b.start);
            blocks.dedup();
            let fc = self.frame_count;
            let mut i = 0;
            while i < blocks.len() {
                let run_start = blocks[i].start;
                let mut end = blocks[i].span(fc).end;
                let mut j = i + 1;
                while j < blocks.len() && blocks[j].start == end {
                    end = blocks[j].span(fc).end;
                    j += 1;
                }
                let blank = self.alloc_cell_for(layer);
                let l = &mut self.layers[layer];
                l.exposures[run_start] = Some(blank);
                for f in run_start + 1..end {
                    l.exposures[f] = None;
                }
                out.push((layer, run_start));
                i = j;
            }
        }
        out
    }

    /// Delete the drawings keyed at `starts` and close the gap: later
    /// drawings on each layer slide up, and its last drawing holds longer.
    pub fn close_blocks(&mut self, starts: &[(usize, usize)]) {
        for layer in distinct_layers(starts) {
            let mut frames: Vec<usize> = starts
                .iter()
                .filter(|s| s.0 == layer)
                .map(|s| s.1)
                .collect();
            // Rightmost first, so the starts still to come stay where they were.
            frames.sort_unstable_by(|a, b| b.cmp(a));
            frames.dedup();
            for f in frames {
                // Re-found each time rather than taken up front: closing the
                // drawing after this one can leave this one the layer's last,
                // holding to the end, and its old length would then cut it
                // short and re-key what was left.
                let Some(b) = self.block_at(layer, f).filter(|b| b.start == f) else {
                    continue;
                };
                let span = b.span(self.frame_count);
                self.remove_layer_frames(layer, span.start, span.len());
            }
        }
    }

    /// Frames block `b` dropped at `start` on `layer` would cover: as many as
    /// it covers now, clipped at the end of the timeline.
    ///
    /// A layer's last drawing also stops at the next drawing already on the
    /// target. Its length is only what was left of the timeline, not a timing
    /// anyone chose, so it must not wipe out every key after it — but it keeps
    /// that length when there is room, rather than flooding the rest of an
    /// emptier layer. Its own key doesn't count: a move is vacating it.
    ///
    /// Moving the source first never changes this: it swaps the source's
    /// cell for a blank but leaves every key where it was.
    pub fn landing_span(&self, layer: usize, start: usize, b: Block) -> Range<usize> {
        let fc = self.frame_count;
        let start = start.min(fc.saturating_sub(1));
        let mut end = (start + b.span(fc).len().max(1)).min(fc);
        if b.len.is_none() {
            let own_key = |f: usize| layer == b.layer && f == b.start;
            if let Some(next) = self
                .layers
                .get(layer)
                .and_then(|l| (start + 1..end).find(|&f| l.is_key(f) && !own_key(f)))
            {
                end = next;
            }
        }
        start..end
    }

    /// Whether a drag from `src` to `dst` may land. Nothing ever edits a
    /// locked or reference layer, and only a move edits its source.
    pub fn drop_allowed(&self, src: usize, dst: usize, copy: bool) -> bool {
        let editable = |i: usize| self.layers.get(i).is_some_and(|l| !l.locked && !l.reference);
        editable(dst) && (copy || editable(src))
    }

    /// A deep copy of `id` sized for `layer`, recentred rather than stretched
    /// when that layer's cells are a different size.
    fn clone_cell_for(&mut self, id: CellId, layer: usize) -> CellId {
        let (w, h) = self.layers[layer].cell_size(self.width, self.height);
        let src = &self.cells[id];
        let cell = if (src.width, src.height) == (w, h) {
            src.clone()
        } else {
            recenter(src, w, h)
        };
        self.cells.push(cell);
        self.cells.len() - 1
    }

    /// Drop block `b` at `dst_start` on `dst_layer`, overwriting what is
    /// there. The drawing that was showing just after the landing span keeps
    /// showing, so nothing further along moves. A move leaves a blank key
    /// where the drawing was; a copy leaves the source alone. Returns where the
    /// drawing landed, or `None` when there was nothing to do.
    pub fn drop_block(
        &mut self,
        b: Block,
        dst_layer: usize,
        dst_start: usize,
        copy: bool,
    ) -> Option<usize> {
        if dst_layer >= self.layers.len() || dst_start >= self.frame_count {
            return None;
        }
        if !copy && b.layer == dst_layer && b.start == dst_start {
            return None;
        }
        let id = *self.layers.get(b.layer)?.exposures.get(b.start)?;
        let id = id?;
        if !copy {
            let blank = self.alloc_cell_for(b.layer);
            self.layers[b.layer].exposures[b.start] = Some(blank);
        }
        let cell = if copy {
            self.clone_cell_for(id, dst_layer)
        } else if b.layer == dst_layer {
            id
        } else {
            // Hand the buffer over when nothing on the source layer still
            // shows it and it already fits; cells are never shared between
            // layers, so an alias left behind has to be a copy instead.
            let size = self.layers[dst_layer].cell_size(self.width, self.height);
            let unshared = !self.layers[b.layer].exposures.contains(&Some(id));
            let fits = (self.cells[id].width, self.cells[id].height) == size;
            if unshared && fits {
                id
            } else {
                self.clone_cell_for(id, dst_layer)
            }
        };

        let span = self.landing_span(dst_layer, dst_start, b);
        // What shows right after the span, read after the source was blanked
        // (a same-layer move can put the source there) but before the write.
        let follow = self.layers[dst_layer]
            .exposures
            .get(span.end)
            .is_some_and(Option::is_none)
            .then(|| self.layers[dst_layer].resolve(span.end));
        let l = &mut self.layers[dst_layer];
        l.exposures[span.start] = Some(cell);
        for f in span.start + 1..span.end {
            l.exposures[f] = None;
        }
        if let Some(follow) = follow {
            // Nothing was showing: stop the dropped drawing with a blank, or
            // it would hold on to the end.
            let key = match follow {
                Some(k) => k,
                None => self.alloc_cell_for(dst_layer),
            };
            self.layers[dst_layer].exposures[span.end] = Some(key);
        }
        Some(span.start)
    }

    /// The blocks keyed exactly at `starts`, skipping any that are not keys.
    pub fn blocks_keyed_at(&self, starts: &[(usize, usize)]) -> Vec<Block> {
        starts
            .iter()
            .filter_map(|&(l, f)| self.block_at(l, f).filter(|b| b.start == f))
            .collect()
    }

    /// Whether [`Project::move_blocks`] would do anything: every block stays
    /// on the sheet, lands only where [`Project::drop_allowed`] says it may,
    /// and the drag actually goes somewhere.
    pub fn can_move_blocks(&self, starts: &[(usize, usize)], dl: isize, df: isize, copy: bool) -> bool {
        if !copy && dl == 0 && df == 0 {
            return false;
        }
        let blocks = self.blocks_keyed_at(starts);
        let n_layers = self.layers.len() as isize;
        let fc = self.frame_count as isize;
        !blocks.is_empty()
            && blocks.iter().all(|b| {
                let tl = b.layer as isize + dl;
                let tf = b.start as isize + df;
                (0..n_layers).contains(&tl)
                    && (0..fc).contains(&tf)
                    && self.drop_allowed(b.layer, tl as usize, copy)
            })
    }

    /// Move (or copy) every block keyed at `starts` by `dl` layers and `df`
    /// frames. `None`, with nothing changed, when any block would leave the
    /// sheet or land on a layer it may not.
    ///
    /// Applied in the order that never lands on a source still waiting to
    /// move: layers in the direction of travel first, and within a layer the
    /// block furthest along the direction of travel first.
    pub fn move_blocks(
        &mut self,
        starts: &[(usize, usize)],
        dl: isize,
        df: isize,
        copy: bool,
    ) -> Option<Vec<(usize, usize)>> {
        if !self.can_move_blocks(starts, dl, df, copy) {
            return None;
        }
        let mut blocks = self.blocks_keyed_at(starts);
        blocks.sort_by_key(|b| {
            let l = b.layer as isize * -dl.signum();
            let f = b.start as isize * -df.signum();
            (l, f)
        });
        let mut out = Vec::new();
        for b in blocks {
            let tl = (b.layer as isize + dl) as usize;
            let tf = (b.start as isize + df) as usize;
            if let Some(at) = self.drop_block(b, tl, tf, copy) {
                out.push((tl, at));
            }
        }
        Some(out)
    }
}

/// A drawing and the frames it holds for, on one layer: the key at `start`
/// through to the next key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub layer: usize,
    pub start: usize,
    /// Frames held. `None` for the layer's last drawing, which holds to the
    /// end of the timeline however long that is.
    pub len: Option<usize>,
}

impl Block {
    /// Frames the block covers on a timeline `frame_count` long.
    pub fn span(&self, frame_count: usize) -> Range<usize> {
        self.start..self.len.map_or(frame_count, |n| self.start + n)
    }
}

/// Each layer named in `starts`, once, in order.
fn distinct_layers(starts: &[(usize, usize)]) -> Vec<usize> {
    let mut layers: Vec<usize> = starts.iter().map(|s| s.0).collect();
    layers.sort_unstable();
    layers.dedup();
    layers
}

/// Insert `n` holds right after `after` on one layer, `track_points` kept in
/// step. The layer can come out longer than the timeline; [`layer_fit`]
/// settles it.
pub fn layer_insert(l: &mut Layer, after: usize, n: usize) {
    let at = (after + 1).min(l.exposures.len());
    l.exposures.splice(at..at, std::iter::repeat(None).take(n));
    for _ in 0..n {
        l.track_insert_frame(at);
    }
    l.pins_insert_frames(at, n);
}

/// Remove `[at, at + n)` from one layer and pad its end with holds. When the
/// cut took the key of a drawing that carries on past it, that drawing is
/// re-keyed at `at`, so the frame after the cut shows what it showed before.
pub fn layer_remove(l: &mut Layer, at: usize, n: usize) {
    let len = l.exposures.len();
    let end = (at + n).min(len);
    if at >= end {
        return;
    }
    let last_key = l.exposures[at..end].iter().rev().find_map(|e| *e);
    l.exposures.drain(at..end);
    l.exposures.resize(len, None);
    for _ in at..end {
        l.track_remove_frame(at);
        l.track_insert_frame(l.track_points.len());
    }
    l.pins_remove_frames(at, end - at);
    if end < len {
        if let Some(k) = last_key {
            if l.exposures[at].is_none() {
                l.exposures[at] = Some(k);
            }
        }
    }
}

/// Trim or pad a layer to the timeline — or past it, when a key was pushed
/// beyond the end. Returns the length the timeline has to become.
pub fn layer_fit(l: &mut Layer, min_len: usize) -> usize {
    let need = l
        .exposures
        .iter()
        .rposition(Option::is_some)
        .map_or(0, |i| i + 1)
        .max(min_len);
    l.exposures.resize(need, None);
    if !l.track_points.is_empty() {
        l.track_points.resize(need, TrackSample::default());
    }
    need
}

/// [`Project::retime_block`] on one layer, with `blank` supplying a fresh blank
/// cell when a last drawing is cut short. Public so the timeline can preview a
/// retime on a copy of the layer, handing it a placeholder id, and show
/// exactly what the edit is about to do. Returns the timeline length needed.
pub fn layer_retime(
    l: &mut Layer,
    frame_count: usize,
    b: Block,
    new_len: usize,
    blank: impl FnOnce() -> CellId,
) -> usize {
    let new_len = new_len.max(1);
    let mut min_len = frame_count;
    match b.len {
        Some(old) if new_len > old => layer_insert(l, b.start + old - 1, new_len - old),
        Some(old) if new_len < old => layer_remove(l, b.start + new_len, old - new_len),
        Some(_) => {}
        None => {
            let old = frame_count.saturating_sub(b.start);
            if new_len < old {
                // A last drawing stops only if something follows it.
                l.exposures[b.start + new_len] = Some(blank());
            } else {
                min_len = min_len.max(b.start + new_len);
            }
        }
    }
    layer_fit(l, min_len)
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

    #[test]
    fn step_wraps_at_both_ends_when_looping() {
        let mut p = Project::new(64, 64, 12.0);
        while p.frame_count < 4 {
            p.add_frame();
        }
        p.goto(3);
        p.step(1, true);
        assert_eq!(p.current_frame, 0, "past the end comes back to the start");
        p.step(-1, true);
        assert_eq!(p.current_frame, 3, "and back the other way");
    }

    #[test]
    fn step_clamps_at_both_ends_when_not_looping() {
        let mut p = Project::new(64, 64, 12.0);
        while p.frame_count < 4 {
            p.add_frame();
        }
        p.goto(3);
        p.step(1, false);
        assert_eq!(p.current_frame, 3, "the last frame is a wall");
        p.goto(0);
        p.step(-5, false);
        assert_eq!(p.current_frame, 0, "so is the first, however big the step");
    }

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

    /// Cut must not shorten this layer or touch any other: every layer's
    /// `exposures` stays `frame_count` long, which the whole timeline assumes.
    #[test]
    fn cut_blanks_the_slot_without_disturbing_the_grid() {
        let mut p = Project::new(4, 4, 12.0);
        p.add_layer();
        p.ensure_frame_count(3);
        p.current_layer = 0;
        p.current_frame = 1;
        p.insert_blank_key_here();
        let id = p.resolved_current().unwrap();
        p.cells[id].pixels[0] = 200;

        let taken = p.cut_active_cell().expect("something to cut");
        assert_eq!(taken.pixels[0], 200);
        assert_eq!(p.frame_count, 3);
        for l in &p.layers {
            assert_eq!(l.exposures.len(), 3);
        }
        // The slot is blank, not holding the previous drawing.
        let now = p.resolved_current().unwrap();
        assert!(p.cells[now].pixels.iter().all(|&b| b == 0));
    }

    #[test]
    fn pasted_drawings_are_independent_copies() {
        let mut p = Project::new(4, 4, 12.0);
        p.ensure_frame_count(3);
        p.current_frame = 0;
        let src = p.insert_blank_key_here();
        p.cells[src].pixels[0] = 111;

        let copied = p.copy_active_cell().unwrap();
        p.current_frame = 2;
        let pasted = p.paste_cell_here(&copied);
        assert_eq!(p.cells[pasted].pixels[0], 111);

        // Editing the paste must not reach back to the original.
        p.cells[pasted].pixels[0] = 222;
        assert_eq!(p.cells[src].pixels[0], 111);
    }

    #[test]
    fn pasting_into_a_differently_sized_layer_recentres() {
        let mut p = Project::new(4, 4, 12.0);
        p.ensure_frame_count(2);
        p.expand_layer_canvas(0, 8, 8);
        let mut src = Canvas::new(4, 4);
        src.pixels[0] = 90; // top-left of the small drawing
        let id = p.paste_cell_here(&src);
        assert_eq!((p.cells[id].width, p.cells[id].height), (8, 8));
        // Centred: the old origin lands at (2,2) in the bigger buffer.
        let at = ((2 * 8 + 2) * 4) as usize;
        assert_eq!(p.cells[id].pixels[at], 90);
    }

    /// Keys on 0 / 4 / 8 of an 11-frame layer — the shape the jump buttons
    /// navigate.
    fn keyed_on_fours() -> Project {
        let mut p = Project::new(4, 4, 12.0);
        p.ensure_frame_count(11);
        for f in [0, 4, 8] {
            p.current_frame = f;
            p.insert_blank_key_here();
        }
        p
    }

    /// The point of the feature: from a held frame the jump reaches the
    /// bracketing drawings, not the neighbouring repeats.
    #[test]
    fn key_jump_skips_holds() {
        let mut p = keyed_on_fours();
        p.current_frame = 6;
        assert_eq!(p.prev_key_frame(), Some(4));
        assert_eq!(p.next_key_frame(), Some(8));
    }

    /// Strictly before / after: sitting on a key must still move off it,
    /// otherwise the button would be a no-op wherever it matters most.
    #[test]
    fn key_jump_is_strict_about_the_current_frame() {
        let mut p = keyed_on_fours();
        p.current_frame = 4;
        assert_eq!(p.prev_key_frame(), Some(0));
        assert_eq!(p.next_key_frame(), Some(8));
    }

    /// Clamping, not wrapping — `None` is what greys the button out.
    #[test]
    fn key_jump_clamps_at_both_ends() {
        let mut p = keyed_on_fours();
        p.current_frame = 0;
        assert_eq!(p.prev_key_frame(), None, "nothing before the first key");
        p.current_frame = 8;
        assert_eq!(p.next_key_frame(), None, "nothing after the last key");
        p.current_frame = 10;
        assert_eq!(p.next_key_frame(), None, "past the last key either");
        assert_eq!(p.prev_key_frame(), Some(8));
    }

    /// The jump reads the *active* layer only. A fresh `Project` keys its
    /// first layer on frame 0, so this needs a second, untouched one.
    #[test]
    fn key_jump_on_an_untouched_layer_finds_nothing() {
        let mut p = Project::new(4, 4, 12.0);
        p.ensure_frame_count(5);
        p.add_layer();
        assert_eq!(p.current_layer, 1);
        p.current_frame = 2;
        assert_eq!(p.prev_key_frame(), None);
        assert_eq!(p.next_key_frame(), None);
        // …while layer 0 still has its seed key.
        p.current_layer = 0;
        assert_eq!(p.prev_key_frame(), Some(0));
    }

    // --- Drawing blocks ---

    /// Key a fresh drawing on (`layer`, `f`) whose first byte is `mark`, so a
    /// test can tell which drawing shows where after it has moved.
    fn key(p: &mut Project, layer: usize, f: usize, mark: u8) -> CellId {
        let id = p.alloc_cell_for(layer);
        p.cells[id].pixels[0] = mark;
        p.layers[layer].set_key(f, id);
        id
    }

    /// One layer, `frames` long, with a drawing keyed on each of `keys`,
    /// marked with its key frame + 1.
    fn sheet(frames: usize, keys: &[usize]) -> Project {
        let mut p = Project::new(4, 4, 12.0);
        p.ensure_frame_count(frames);
        p.layers[0].exposures = vec![None; frames];
        for &f in keys {
            key(&mut p, 0, f, f as u8 + 1);
        }
        p
    }

    fn key_frames(p: &Project, layer: usize) -> Vec<usize> {
        (0..p.frame_count).filter(|&f| p.layers[layer].is_key(f)).collect()
    }

    /// The mark of the drawing showing on every frame; 0 for nothing or blank.
    fn row(p: &Project, layer: usize) -> Vec<u8> {
        (0..p.frame_count)
            .map(|f| p.layers[layer].resolve(f).map_or(0, |id| p.cells[id].pixels[0]))
            .collect()
    }

    /// What the whole timeline assumes: every layer exactly as long as the
    /// timeline, and no cell shown on two layers.
    fn assert_sound(p: &Project) {
        for l in &p.layers {
            assert_eq!(l.exposures.len(), p.frame_count, "layer {} length", l.name);
        }
        let mut owner = std::collections::HashMap::new();
        for li in 0..p.layers.len() {
            for id in p.layer_cell_ids(li) {
                if let Some(prev) = owner.insert(id, li) {
                    assert_eq!(prev, li, "cell {id} is on two layers");
                }
            }
        }
    }

    #[test]
    fn a_block_is_a_key_and_its_hold() {
        let p = sheet(8, &[2, 5]);
        assert_eq!(p.block_at(0, 1), None, "nothing shows before the first key");
        let held = Block { layer: 0, start: 2, len: Some(3) };
        assert_eq!(p.block_at(0, 2), Some(held));
        assert_eq!(p.block_at(0, 4), Some(held), "a held frame belongs to its key");
        let last = Block { layer: 0, start: 5, len: None };
        assert_eq!(p.block_at(0, 7), Some(last), "the last drawing is open-ended");
        assert_eq!(p.blocks(0), vec![held, last]);
    }

    /// The end of the timeline stays put: the last drawing's hold absorbs it.
    #[test]
    fn inserting_frames_slides_only_that_layer() {
        let mut p = sheet(10, &[0, 2, 5]);
        p.add_layer();
        key(&mut p, 1, 3, 50);
        p.insert_layer_frames(0, 3, 2);
        assert_eq!(key_frames(&p, 0), vec![0, 2, 7]);
        assert_eq!(key_frames(&p, 1), vec![3], "other layers never move");
        assert_eq!(p.frame_count, 10);
        assert_sound(&p);
    }

    #[test]
    fn a_key_pushed_off_the_end_grows_every_layer() {
        let mut p = sheet(6, &[0, 5]);
        p.add_layer();
        p.insert_layer_frames(0, 0, 3);
        assert_eq!(key_frames(&p, 0), vec![0, 8]);
        assert_eq!(p.frame_count, 9);
        assert_sound(&p);
    }

    #[test]
    fn removing_frames_keeps_what_showed_after_the_cut() {
        let mut p = sheet(10, &[0, 4]);
        p.layers[0].track_points = (0..10)
            .map(|f| TrackSample {
                a: Some([f as f32, 0.0]),
                b: None,
            })
            .collect();
        // Takes the head of the drawing keyed on 4, which carries on past it.
        p.remove_layer_frames(0, 3, 3);
        assert_eq!(row(&p, 0), vec![1, 1, 1, 5, 5, 5, 5, 5, 5, 5]);
        assert_eq!(p.layers[0].track_points.len(), 10);
        assert_eq!(p.layers[0].track_points[3].a, Some([6.0, 0.0]));
        assert_sound(&p);
    }

    #[test]
    fn retiming_a_drawing_slides_the_ones_after_it() {
        let mut p = sheet(10, &[0, 2, 4]);
        p.retime_block(p.block_at(0, 2).unwrap(), 4);
        assert_eq!(key_frames(&p, 0), vec![0, 2, 6]);
        p.retime_block(p.block_at(0, 2).unwrap(), 1);
        assert_eq!(key_frames(&p, 0), vec![0, 2, 3]);
        assert_eq!(p.frame_count, 10);
        assert_sound(&p);
    }

    #[test]
    fn retiming_the_last_drawing_stops_it_or_grows_the_timeline() {
        let mut p = sheet(10, &[0, 6]);
        p.retime_block(p.block_at(0, 6).unwrap(), 2);
        assert_eq!(row(&p, 0)[6..], [7, 7, 0, 0], "a blank stops it after two");
        assert_eq!(p.frame_count, 10);

        let mut p = sheet(10, &[0, 6]);
        p.add_layer();
        p.retime_block(p.block_at(0, 6).unwrap(), 6);
        assert_eq!(p.frame_count, 12);
        assert_eq!(row(&p, 0)[6..], [7; 6]);
        assert_sound(&p);
    }

    #[test]
    fn set_timing_puts_a_run_on_twos() {
        let mut p = sheet(6, &[0, 1, 2, 3]);
        let out = p.set_timing(&[(0, 0), (0, 1), (0, 2)], 2);
        assert_eq!(out, vec![(0, 0), (0, 2), (0, 4)]);
        // The unselected drawing after the run slides along behind it.
        assert_eq!(key_frames(&p, 0), vec![0, 2, 4, 6]);
        assert_eq!(row(&p, 0)[..6], [1, 1, 2, 2, 3, 3]);
        assert_sound(&p);
    }

    #[test]
    fn clearing_leaves_blank_frames_and_moves_nothing() {
        let mut p = sheet(10, &[0, 2, 4, 6, 8]);
        let out = p.clear_blocks(&[(0, 2), (0, 4), (0, 8)]);
        // 2 and 4 touch, so they become one blank; 8 gets its own.
        assert_eq!(out, vec![(0, 2), (0, 8)]);
        assert_eq!(key_frames(&p, 0), vec![0, 2, 6, 8]);
        assert_eq!(row(&p, 0), vec![1, 1, 0, 0, 0, 0, 7, 7, 0, 0]);
        assert_ne!(
            p.layers[0].exposures[2], p.layers[0].exposures[8],
            "painting one blank must not paint the other"
        );
        assert_sound(&p);
    }

    #[test]
    fn closing_the_gap_slides_later_drawings_up() {
        let mut p = sheet(10, &[0, 2, 4, 7]);
        p.close_blocks(&[(0, 2)]);
        assert_eq!(key_frames(&p, 0), vec![0, 2, 5]);
        p.close_blocks(&[(0, 5)]);
        assert_eq!(row(&p, 0)[5..], [5; 5], "the one before holds to the end");
        assert_sound(&p);

        // Closing the last drawing makes the one before it the last; it must
        // still go, not come back holding over the gap.
        let mut p = sheet(10, &[0, 2, 4, 7]);
        p.close_blocks(&[(0, 4), (0, 7)]);
        assert_eq!(row(&p, 0), vec![1, 1, 3, 3, 3, 3, 3, 3, 3, 3]);
        assert_sound(&p);
    }

    #[test]
    fn moving_within_a_layer_keeps_its_length_and_leaves_a_blank() {
        let mut p = sheet(10, &[0, 2, 4]);
        let b = p.block_at(0, 2).unwrap();
        assert_eq!(p.drop_block(b, 0, 3, false), Some(3));
        assert_eq!(row(&p, 0), vec![1, 1, 0, 3, 3, 5, 5, 5, 5, 5]);
        assert_sound(&p);
        let b = p.block_at(0, 3).unwrap();
        assert_eq!(p.drop_block(b, 0, 3, false), None, "onto itself is no edit");
    }

    #[test]
    fn landing_in_a_hold_keeps_the_rest_of_that_hold() {
        let mut p = sheet(10, &[0, 2, 4]);
        p.add_layer();
        let bg = key(&mut p, 1, 0, 100);
        let b = p.block_at(0, 2).unwrap();
        p.drop_block(b, 1, 5, true);
        assert_eq!(row(&p, 1), vec![100, 100, 100, 100, 100, 3, 3, 100, 100, 100]);
        assert_eq!(p.layers[1].exposures[7], Some(bg), "the same drawing, not a copy");
        // A copy is its own buffer.
        let landed = p.layers[1].exposures[5].unwrap();
        p.cells[landed].pixels[0] = 9;
        assert_eq!(row(&p, 0)[2], 3);
        assert_sound(&p);
    }

    #[test]
    fn moving_to_an_empty_layer_hands_the_buffer_over_and_stops_it() {
        let mut p = sheet(10, &[0, 2, 4]);
        p.add_layer();
        let moved = p.layers[0].exposures[2];
        let b = p.block_at(0, 2).unwrap();
        p.drop_block(b, 1, 5, false);
        assert_eq!(row(&p, 1), vec![0, 0, 0, 0, 0, 3, 3, 0, 0, 0]);
        assert_eq!(p.layers[1].exposures[5], moved, "no copy when nothing else shows it");
        assert!(p.layers[1].is_key(7), "a blank stops it holding to the end");
        assert_eq!(row(&p, 0)[2..4], [0, 0]);
        assert_sound(&p);
    }

    #[test]
    fn a_cross_layer_move_copies_when_it_has_to() {
        // Still shown elsewhere on its own layer (as `duplicate_frame` does).
        let mut p = sheet(6, &[0, 2]);
        let first = p.layers[0].exposures[0].unwrap();
        p.layers[0].set_key(4, first);
        p.add_layer();
        p.drop_block(p.block_at(0, 0).unwrap(), 1, 0, false);
        assert_ne!(p.layers[1].exposures[0], Some(first));
        assert_sound(&p);

        // A different cell size: recentred, not stretched.
        let mut p = sheet(6, &[0]);
        p.add_layer();
        p.expand_layer_canvas(1, 8, 8);
        p.drop_block(p.block_at(0, 0).unwrap(), 1, 0, false);
        let id = p.layers[1].exposures[0].unwrap();
        assert_eq!((p.cells[id].width, p.cells[id].height), (8, 8));
        assert_sound(&p);
    }

    #[test]
    fn a_last_drawing_keeps_its_length_but_stops_at_the_next_key() {
        let mut p = sheet(10, &[0, 7]);
        p.add_layer();
        key(&mut p, 1, 0, 100);
        key(&mut p, 1, 5, 101);
        let last = p.block_at(0, 7).unwrap();
        assert_eq!(last.len, None);
        assert_eq!(p.landing_span(1, 1, last), 1..4, "its three frames, with room");
        assert_eq!(p.landing_span(1, 3, last), 3..5, "stopped by the key on 5");
        let fixed = Block { layer: 0, start: 0, len: Some(4) };
        assert_eq!(p.landing_span(1, 8, fixed), 8..10, "clipped at the end");
    }

    /// The bug the tracks shipped with: the last drawing, dropped on a layer
    /// with nothing after the drop point, flooded the rest of that layer.
    #[test]
    fn a_last_drawing_moved_to_an_emptier_layer_keeps_its_length() {
        let mut p = sheet(8, &[0, 2, 5]);
        p.add_layer();
        key(&mut p, 1, 0, 50);
        p.drop_block(p.block_at(0, 5).unwrap(), 1, 0, false);
        assert_eq!(row(&p, 1), vec![6, 6, 6, 50, 50, 50, 50, 50]);
        assert_eq!(row(&p, 0)[5..], [0, 0, 0]);
        assert_sound(&p);
    }

    /// Moving the last drawing left on its own layer: its vacated key is not
    /// an obstacle, so it keeps all of its frames.
    #[test]
    fn a_last_drawing_moved_left_keeps_its_length() {
        let mut p = sheet(8, &[0, 2, 5]);
        p.drop_block(p.block_at(0, 5).unwrap(), 0, 3, false);
        assert_eq!(row(&p, 0), vec![1, 1, 3, 6, 6, 6, 0, 0]);
        assert_sound(&p);
    }

    #[test]
    fn moving_a_run_lands_every_block_in_either_direction() {
        let mut p = sheet(10, &[0, 4, 6, 8]);
        let out = p.move_blocks(&[(0, 4), (0, 6)], 0, 2, false).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(row(&p, 0), vec![1, 1, 1, 1, 0, 0, 5, 5, 7, 7]);
        assert_sound(&p);

        let mut p = sheet(10, &[0, 4, 6, 8]);
        p.move_blocks(&[(0, 4), (0, 6)], 0, -2, false).unwrap();
        assert_eq!(row(&p, 0), vec![1, 1, 5, 5, 7, 7, 0, 0, 9, 9]);
        assert_sound(&p);
    }

    #[test]
    fn moving_a_rectangle_down_a_layer_moves_the_lower_row_first() {
        let mut p = sheet(6, &[0]);
        p.add_layer();
        p.add_layer();
        key(&mut p, 1, 0, 20);
        let out = p.move_blocks(&[(0, 0), (1, 0)], 1, 0, false).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(row(&p, 2)[0], 20, "layer 1's drawing went up first");
        assert_eq!(row(&p, 1)[0], 1, "then layer 0's took its place");
        assert_sound(&p);
        assert!(
            p.move_blocks(&[(2, 0)], 1, 0, false).is_none(),
            "off the top of the sheet"
        );
    }

    #[test]
    fn locked_layers_are_never_edited_by_a_drag() {
        let mut p = sheet(4, &[0]);
        p.add_layer();
        p.layers[1].locked = true;
        assert!(!p.drop_allowed(0, 1, true));
        assert!(p.drop_allowed(1, 0, true), "copying out of a locked layer is fine");
        assert!(!p.drop_allowed(1, 0, false), "moving out of it is not");
    }

    // --- Onion pins ---

    fn pin_frames(p: &Project, layer: usize) -> Vec<usize> {
        p.layers[layer].onion_pins.iter().map(|p| p.frame).collect()
    }

    fn pinned(frames: usize, keys: &[usize], pins: &[usize]) -> Project {
        let mut p = sheet(frames, keys);
        p.layers[0].onion_pins = pins
            .iter()
            .map(|&frame| crate::timeline::onion::OnionPin {
                frame,
                tint: [0, 255, 0],
                visible: true,
            })
            .collect();
        p
    }

    #[test]
    fn pins_follow_their_drawings_through_frame_inserts() {
        let mut p = pinned(10, &[0, 4, 8], &[0, 4, 8]);
        p.current_frame = 3;
        p.add_frame(); // inserts at 4
        assert_eq!(pin_frames(&p, 0), vec![0, 5, 9]);
        p.current_frame = 5;
        p.duplicate_frame(); // inserts at 6
        assert_eq!(pin_frames(&p, 0), vec![0, 5, 10]);
        p.insert_layer_frames(0, 0, 2); // inserts at 1
        assert_eq!(pin_frames(&p, 0), vec![0, 7, 12]);
        // Every pin still shows the drawing it was put on.
        let marks: Vec<u8> = pin_frames(&p, 0).iter().map(|&f| row(&p, 0)[f]).collect();
        assert_eq!(marks, vec![1, 5, 9]);
    }

    #[test]
    fn a_pin_on_a_removed_frame_goes_with_it() {
        let mut p = pinned(10, &[0, 4, 8], &[0, 4, 8]);
        p.current_frame = 4;
        p.delete_frame();
        assert_eq!(pin_frames(&p, 0), vec![0, 7]);
        p.remove_layer_frames(0, 1, 2);
        assert_eq!(pin_frames(&p, 0), vec![0, 5]);
    }

    #[test]
    fn padding_the_timeline_leaves_pins_alone() {
        let mut p = pinned(4, &[0, 2], &[0, 2]);
        p.ensure_frame_count(9);
        assert_eq!(pin_frames(&p, 0), vec![0, 2]);
    }
}
