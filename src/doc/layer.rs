//! Layer = stack of cell exposures across the timeline.
//!
//! `exposures[frame]` decides what shows on that frame for this layer:
//!   * `Some(cell_id)` = a key — the cell with that id appears.
//!   * `None`          = hold — the previous non-None entry continues to show.
//!
//! Drawing on a frame that resolves to a held cell modifies the shared cell
//! (animator convention). To break a hold, the user inserts a new key via the
//! X-sheet panel.

pub type CellId = usize;

use crate::doc::transform::{Transform, TransformKey};

/// One frame's stabilization tracking points, in document space.
/// `a` is the primary point (translation); `b` is optional and enables
/// rotation/scale correction when present on both the reference frame and the
/// tracked frame.
#[derive(Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct TrackSample {
    pub a: Option<[f32; 2]>,
    pub b: Option<[f32; 2]>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Layer {
    pub name: String,
    pub opacity: f32,
    pub visible: bool,
    pub locked: bool,
    /// If true, this layer renders at a fixed dim opacity and is excluded
    /// from final export — used as a light-table / reference layer.
    pub reference: bool,
    pub exposures: Vec<Option<CellId>>,
    /// Live / static layer transform. Used directly when `transform_keys` is
    /// empty; otherwise it is the working buffer for the current frame
    /// (synced from the keys when scrubbing, edited live when dragging).
    #[serde(default)]
    pub transform: Transform,
    /// Sorted keyframes for the layer transform. Empty = static `transform`.
    #[serde(default)]
    pub transform_keys: Vec<TransformKey>,
    /// Per-frame stabilization tracking samples, parallel to `exposures`.
    /// Empty vec = tracker unused on this layer. Project frame edits keep the
    /// indices aligned with `exposures`. Must stay the LAST field: the `.anim`
    /// format (postcard) is positional.
    #[serde(default)]
    pub track_points: Vec<TrackSample>,
}

impl Layer {
    pub fn new(name: impl Into<String>, frames: usize) -> Self {
        Self {
            name: name.into(),
            opacity: 1.0,
            visible: true,
            locked: false,
            reference: false,
            exposures: vec![None; frames.max(1)],
            transform: Transform::default(),
            transform_keys: Vec::new(),
            track_points: Vec::new(),
        }
    }

    /// Keep `track_points` index-aligned with `exposures` after a frame is
    /// inserted at `at`. No-op while the tracker is unused (empty vec).
    pub fn track_insert_frame(&mut self, at: usize) {
        if self.track_points.is_empty() {
            return;
        }
        if at >= self.track_points.len() {
            self.track_points.push(TrackSample::default());
        } else {
            self.track_points.insert(at, TrackSample::default());
        }
    }

    /// Keep `track_points` index-aligned with `exposures` after frame `at` is
    /// removed. No-op while the tracker is unused (empty vec).
    pub fn track_remove_frame(&mut self, at: usize) {
        if at < self.track_points.len() {
            self.track_points.remove(at);
        }
    }

    /// The transform shown on `frame`: the static `transform` when there are no
    /// keys, otherwise the piecewise-linear interpolation of the keys (held flat
    /// before the first and after the last key).
    pub fn resolve_transform(&self, frame: usize) -> Transform {
        let keys = &self.transform_keys;
        if keys.is_empty() {
            return self.transform;
        }
        if frame <= keys[0].frame {
            return keys[0].transform;
        }
        let last = keys.len() - 1;
        if frame >= keys[last].frame {
            return keys[last].transform;
        }
        // Find the bracketing pair.
        for w in keys.windows(2) {
            let (a, b) = (w[0], w[1]);
            if frame >= a.frame && frame <= b.frame {
                let span = (b.frame - a.frame).max(1) as f32;
                let t = (frame - a.frame) as f32 / span;
                return Transform::lerp(a.transform, b.transform, t);
            }
        }
        keys[last].transform
    }

    pub fn has_transform_key(&self, frame: usize) -> bool {
        self.transform_keys.iter().any(|k| k.frame == frame)
    }

    /// Insert (or replace) a transform key at `frame`, keeping keys sorted.
    pub fn set_transform_key(&mut self, frame: usize, transform: Transform) {
        match self.transform_keys.iter_mut().find(|k| k.frame == frame) {
            Some(k) => k.transform = transform,
            None => {
                self.transform_keys.push(TransformKey { frame, transform });
                self.transform_keys.sort_by_key(|k| k.frame);
            }
        }
    }

    pub fn delete_transform_key(&mut self, frame: usize) {
        self.transform_keys.retain(|k| k.frame != frame);
    }

    /// Returns the resolved CellId showing on `frame`, walking back through
    /// holds.
    pub fn resolve(&self, frame: usize) -> Option<CellId> {
        if self.exposures.is_empty() {
            return None;
        }
        let f = frame.min(self.exposures.len() - 1);
        for i in (0..=f).rev() {
            if let Some(id) = self.exposures[i] {
                return Some(id);
            }
        }
        None
    }

    pub fn set_key(&mut self, frame: usize, cell: CellId) {
        if frame < self.exposures.len() {
            self.exposures[frame] = Some(cell);
        }
    }

    pub fn hold(&mut self, frame: usize) {
        if frame < self.exposures.len() {
            self.exposures[frame] = None;
        }
    }
}
