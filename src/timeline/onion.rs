//! Onion-skin configuration.
//!
//! Composition: previous drawings tinted (default blue) below the current cell,
//! next drawings tinted (default red) above. Alpha falls off with distance.
//!
//! The ghosts are drawn from *colorized* textures (see `AppState::ghost_image`),
//! not by multiplying a tint over the cell texture — a multiply leaves black
//! line art black, which is why ghosts used to read as a grey smudge instead of
//! blue-past / red-future.

use crate::doc::layer::{CellId, Layer};

/// Alpha floor for the outermost ghost, as a fraction of `max_alpha`. Without
/// it the farthest frame in the range fades to nothing and the Prev/Next
/// sliders appear to do less than they do.
const ALPHA_FLOOR: f32 = 0.18;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct OnionConfig {
    pub enabled: bool,
    /// Number of previous drawings to show (0..=8).
    pub prev: u8,
    /// Number of next drawings to show (0..=8).
    pub next: u8,
    /// Silhouette color for previous drawings (unmultiplied RGB).
    pub prev_tint: [u8; 3],
    /// Silhouette color for next drawings (unmultiplied RGB).
    pub next_tint: [u8; 3],
    /// Alpha falloff exponent — higher = distant drawings fade faster.
    pub falloff: f32,
    /// Max alpha (0..=1) for the nearest ghost.
    pub max_alpha: f32,
    /// Count distinct *drawings* rather than frames when stepping outward, so
    /// animation on twos/threes still shows `prev` real drawings back instead
    /// of the same held cell repeated.
    pub by_key: bool,
}

impl Default for OnionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            prev: 1,
            next: 1,
            prev_tint: [90, 150, 255],
            next_tint: [255, 90, 90],
            falloff: 1.2,
            max_alpha: 0.75,
            by_key: true,
        }
    }
}

/// One onion ghost to draw: how many steps away it is, which cell to draw, and
/// the frame its layer transform resolves from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnionStep {
    pub k: u8,
    pub cell: CellId,
    pub frame: usize,
}

impl OnionConfig {
    /// Silhouette color for `direction`.
    pub fn tint_rgb(&self, direction: OnionDirection) -> [u8; 3] {
        match direction {
            OnionDirection::Prev => self.prev_tint,
            OnionDirection::Next => self.next_tint,
        }
    }

    /// Alpha (0..=255) for a ghost `k` steps away from the current frame.
    ///
    /// `k == 1` gets the full `max_alpha` and `k == count` keeps
    /// [`ALPHA_FLOOR`] of it. The old form divided by `count` alone, which put
    /// the farthest ghost at exactly zero — and since the default range is one
    /// frame each way, that made enabling onion skin draw nothing at all.
    pub fn alpha_for(&self, k: u8, direction: OnionDirection) -> u8 {
        let count = match direction {
            OnionDirection::Prev => self.prev,
            OnionDirection::Next => self.next,
        }
        .max(1) as f32;
        let t = (k.max(1) - 1) as f32 / count;
        let falloff = (1.0 - t).max(0.0).powf(self.falloff);
        let alpha = self.max_alpha * falloff.max(ALPHA_FLOOR);
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    /// The ghosts for `layer` around `frame`, ordered nearest-first.
    ///
    /// A cell that resolves to the same drawing as the current frame is never
    /// emitted: on a hold that ghost would land exactly on top of the current
    /// cell, adding tint but no motion information.
    pub fn steps(
        &self,
        layer: &Layer,
        frame: usize,
        frame_count: usize,
        direction: OnionDirection,
    ) -> Vec<OnionStep> {
        let want = match direction {
            OnionDirection::Prev => self.prev,
            OnionDirection::Next => self.next,
        };
        let mut out = Vec::new();
        if !self.enabled || want == 0 || frame_count == 0 {
            return out;
        }
        let current = layer.resolve(frame);
        let mut seen: Vec<CellId> = Vec::new();
        let mut f = frame;
        while out.len() < want as usize {
            f = match direction {
                OnionDirection::Prev => match f.checked_sub(1) {
                    Some(p) => p,
                    None => break,
                },
                OnionDirection::Next => {
                    let n = f + 1;
                    if n >= frame_count {
                        break;
                    }
                    n
                }
            };
            if let Some(cell) = layer.resolve(f) {
                // Skip the current frame's own drawing, and any drawing already
                // ghosted from a nearer frame of the same hold — either would
                // paint a second copy in the same place.
                if Some(cell) != current && !seen.contains(&cell) {
                    seen.push(cell);
                    out.push(OnionStep {
                        k: out.len() as u8 + 1,
                        cell,
                        frame: f,
                    });
                }
            }
            // Frame stepping stays literal: the walk covers `want` frames each
            // way and simply draws fewer ghosts across a hold. Drawing stepping
            // keeps walking until it has `want` distinct drawings.
            if !self.by_key && frame.abs_diff(f) >= want as usize {
                break;
            }
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnionDirection {
    Prev,
    Next,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Layer keyed every `every` frames over `frames` frames, cells 0.. in order.
    fn on_twos(frames: usize, every: usize) -> Layer {
        let mut l = Layer::new("t", frames);
        for (cell, f) in (0..frames).step_by(every).enumerate() {
            l.set_key(f, cell);
        }
        l
    }

    #[test]
    fn nearest_ghost_is_fully_opaque_and_farthest_still_visible() {
        for count in 1..=8u8 {
            let cfg = OnionConfig {
                prev: count,
                ..Default::default()
            };
            let near = cfg.alpha_for(1, OnionDirection::Prev);
            let far = cfg.alpha_for(count, OnionDirection::Prev);
            assert_eq!(near, (cfg.max_alpha * 255.0).round() as u8, "count={count}");
            // The regression that made onion skin invisible at defaults.
            assert!(far > 0, "count={count} farthest ghost vanished");
            assert!(far <= near);
        }
    }

    #[test]
    fn alpha_decreases_with_distance() {
        let cfg = OnionConfig {
            prev: 4,
            ..Default::default()
        };
        let a: Vec<u8> = (1..=4)
            .map(|k| cfg.alpha_for(k, OnionDirection::Prev))
            .collect();
        assert!(a.windows(2).all(|w| w[0] >= w[1]), "{a:?}");
    }

    #[test]
    fn by_key_steps_over_holds_to_distinct_drawings() {
        let layer = on_twos(12, 3);
        let cfg = OnionConfig {
            enabled: true,
            prev: 2,
            by_key: true,
            ..Default::default()
        };
        // Frame 7 holds the key from frame 6 (cell 2). The two previous
        // drawings are cells 1 (frame 3) and 0 (frame 0).
        let steps = cfg.steps(&layer, 7, 12, OnionDirection::Prev);
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].cell, 1);
        assert_eq!(steps[1].cell, 0);
    }

    #[test]
    fn current_drawing_is_never_ghosted() {
        let layer = on_twos(12, 3);
        let cfg = OnionConfig {
            enabled: true,
            prev: 3,
            next: 3,
            by_key: true,
            ..Default::default()
        };
        let cur = layer.resolve(7);
        for dir in [OnionDirection::Prev, OnionDirection::Next] {
            for s in cfg.steps(&layer, 7, 12, dir) {
                assert_ne!(Some(s.cell), cur, "{dir:?} ghosted the current drawing");
            }
        }
    }

    #[test]
    fn steps_stop_at_the_timeline_ends() {
        let layer = on_twos(6, 1);
        let cfg = OnionConfig {
            enabled: true,
            prev: 4,
            next: 4,
            ..Default::default()
        };
        assert!(cfg.steps(&layer, 0, 6, OnionDirection::Prev).is_empty());
        assert!(cfg.steps(&layer, 5, 6, OnionDirection::Next).is_empty());
        assert_eq!(cfg.steps(&layer, 2, 6, OnionDirection::Prev).len(), 2);
        assert_eq!(cfg.steps(&layer, 2, 6, OnionDirection::Next).len(), 3);
    }

    #[test]
    fn disabled_yields_no_steps() {
        let layer = on_twos(6, 1);
        let cfg = OnionConfig::default();
        assert!(cfg.steps(&layer, 3, 6, OnionDirection::Prev).is_empty());
    }
}
