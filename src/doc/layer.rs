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

use crate::doc::transform::{Ease, Transform, TransformKey};

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
    /// Index of the layer whose strokes bound this layer's flood fills — the
    /// line-art layer for a paint layer. `None` = fill reads this layer's own
    /// pixels (plain bucket behaviour). Session-only: not written to `.anim`,
    /// so it resets on load.
    #[serde(skip)]
    pub lines_from: Option<usize>,
    /// Per-frame stabilization tracking samples, parallel to `exposures`.
    /// Empty vec = tracker unused on this layer. Project frame edits keep the
    /// indices aligned with `exposures`.
    #[serde(default)]
    pub track_points: Vec<TrackSample>,
    /// Cell buffer size for this layer's drawings. `0` = use the project frame
    /// size. Larger than the frame lets a single layer's artwork extend past
    /// what the camera sees. Must stay the LAST fields: the `.anim` format
    /// (postcard) is positional.
    #[serde(default)]
    pub cell_w: u32,
    #[serde(default)]
    pub cell_h: u32,
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
            lines_from: None,
            track_points: Vec::new(),
            cell_w: 0,
            cell_h: 0,
        }
    }

    /// Size of the cells this layer draws into: its own override, or the
    /// project frame size when unset.
    pub fn cell_size(&self, pw: u32, ph: u32) -> (u32, u32) {
        (
            if self.cell_w == 0 { pw } else { self.cell_w },
            if self.cell_h == 0 { ph } else { self.cell_h },
        )
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
    /// keys, otherwise the eased interpolation of the keys (held flat before the
    /// first and after the last key). Mirrors `Camera::resolve`.
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
                let t = a.ease.apply((frame - a.frame) as f32 / span);
                return Transform::lerp(a.transform, b.transform, t);
            }
        }
        keys[last].transform
    }

    pub fn has_transform_key(&self, frame: usize) -> bool {
        self.transform_keys.iter().any(|k| k.frame == frame)
    }

    /// Insert (or replace) a transform key at `frame`, keeping keys sorted. A
    /// replaced key keeps its existing ease.
    pub fn set_transform_key(&mut self, frame: usize, transform: Transform) {
        match self.transform_keys.iter_mut().find(|k| k.frame == frame) {
            Some(k) => k.transform = transform,
            None => {
                self.transform_keys.push(TransformKey {
                    frame,
                    transform,
                    ease: Ease::default(),
                });
                self.transform_keys.sort_by_key(|k| k.frame);
            }
        }
    }

    /// Set the ease on the key at `frame`, if there is one. No-op otherwise —
    /// ease belongs to a key, not to a bare frame.
    pub fn set_transform_key_ease(&mut self, frame: usize, ease: Ease) {
        if let Some(k) = self.transform_keys.iter_mut().find(|k| k.frame == frame) {
            k.ease = ease;
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

    /// True when `frame` carries its own key rather than holding an earlier
    /// one. `resolve` can't answer this — it walks back through holds — so the
    /// auto-key-on-draw path asks here before breaking a hold.
    pub fn is_key(&self, frame: usize) -> bool {
        self.exposures.get(frame).is_some_and(Option::is_some)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed(a: Transform, b: Transform, ease: Ease) -> Layer {
        let mut l = Layer::new("L", 11);
        l.set_transform_key(0, a);
        l.set_transform_key(10, b);
        l.set_transform_key_ease(0, ease);
        l
    }

    /// The scroll case: two keys, linear ease, even movement in between.
    #[test]
    fn resolve_transform_interpolates_linearly_by_default() {
        let b = Transform {
            tx: 100.0,
            ..Default::default()
        };
        let l = keyed(Transform::default(), b, Ease::Linear);
        assert!((l.resolve_transform(5).tx - 50.0).abs() < 1e-4);
        // Held flat outside the key range.
        assert!((l.resolve_transform(0).tx).abs() < 1e-6);
        assert!((l.resolve_transform(99).tx - 100.0).abs() < 1e-6);
    }

    /// Ease shapes the segment running *out of* the key it sits on.
    #[test]
    fn resolve_transform_applies_ease_of_the_left_key() {
        let b = Transform {
            tx: 100.0,
            ..Default::default()
        };
        let slow_start = keyed(Transform::default(), b, Ease::In).resolve_transform(5);
        let linear = keyed(Transform::default(), b, Ease::Linear).resolve_transform(5);
        let fast_start = keyed(Transform::default(), b, Ease::Out).resolve_transform(5);
        assert!(slow_start.tx < linear.tx, "ease-in should lag at midpoint");
        assert!(fast_start.tx > linear.tx, "ease-out should lead at midpoint");
        // Endpoints are unaffected by easing.
        assert!((keyed(Transform::default(), b, Ease::Both).resolve_transform(10).tx - 100.0).abs() < 1e-4);
    }

    /// Replacing a key keeps the ease already set on it — re-posing a keyframe
    /// shouldn't silently reset its timing.
    #[test]
    fn set_transform_key_preserves_ease() {
        let mut l = Layer::new("L", 4);
        l.set_transform_key(1, Transform::default());
        l.set_transform_key_ease(1, Ease::Both);
        l.set_transform_key(
            1,
            Transform {
                tx: 9.0,
                ..Default::default()
            },
        );
        assert_eq!(l.transform_keys[0].ease, Ease::Both);
        assert_eq!(l.transform_keys[0].transform.tx, 9.0);
    }

    #[test]
    fn is_key_distinguishes_keys_from_holds() {
        let mut l = Layer::new("L", 4);
        l.set_key(0, 7);
        l.set_key(2, 8);
        assert!(l.is_key(0));
        assert!(!l.is_key(1));
        assert!(l.is_key(2));
        // Frame 1 holds cell 7 even though it is not a key.
        assert_eq!(l.resolve(1), Some(7));
        // Out of range is not a key.
        assert!(!l.is_key(99));
    }
}
