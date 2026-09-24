//! Onion-skin configuration.
//!
//! Composition: previous drawings tinted (default blue) and next drawings
//! tinted (default red), all below the current cell so they never cover the
//! lines being drawn. Alpha falls off with distance.
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
    /// Offsets switched off in the panel: bit `k - 1` hides the ghost `k`
    /// steps back. A hidden offset keeps its slot, so hiding −1 leaves −2
    /// where it was and at the alpha it had.
    pub prev_hidden: u8,
    /// Same as `prev_hidden`, for the ghosts ahead.
    pub next_hidden: u8,
}

/// Starting colours for new pins, handed out in turn. Kept clear of the
/// default blue/red so a pin never reads as an ordinary prev/next ghost.
pub const PIN_TINTS: [[u8; 3]; 4] = [
    [70, 200, 110],
    [255, 165, 40],
    [185, 110, 255],
    [235, 215, 60],
];

/// A frame of one layer kept on screen as a ghost wherever the playhead is,
/// in its own colour. Session-only: pins live on the `Layer` but aren't
/// written to `.anim`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnionPin {
    pub frame: usize,
    pub tint: [u8; 3],
    pub visible: bool,
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
            prev_hidden: 0,
            next_hidden: 0,
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

    /// Whether the ghost `k` steps away in `direction` is switched off.
    pub fn is_hidden(&self, k: u8, direction: OnionDirection) -> bool {
        let mask = match direction {
            OnionDirection::Prev => self.prev_hidden,
            OnionDirection::Next => self.next_hidden,
        };
        matches!(k, 1..=8) && mask & (1 << (k - 1)) != 0
    }

    /// Switch the ghost `k` steps away in `direction` on or off.
    pub fn toggle_hidden(&mut self, k: u8, direction: OnionDirection) {
        if !matches!(k, 1..=8) {
            return;
        }
        let mask = match direction {
            OnionDirection::Prev => &mut self.prev_hidden,
            OnionDirection::Next => &mut self.next_hidden,
        };
        *mask ^= 1 << (k - 1);
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
    ///
    /// Each ghost's `k` is its slot in the range — the how-many-th drawing
    /// back when stepping by drawings, the frame distance when stepping by
    /// frames — so a hidden offset leaves the others where they were.
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
        loop {
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
            let dist = frame.abs_diff(f);
            if let Some(cell) = layer.resolve(f) {
                // Skip the current frame's own drawing, and any drawing already
                // ghosted from a nearer frame of the same hold — either would
                // paint a second copy in the same place. A hidden ghost still
                // counts as seen, so a later frame of its hold can't step into
                // the slot it left.
                if Some(cell) != current && !seen.contains(&cell) {
                    seen.push(cell);
                    let k = if self.by_key { seen.len() } else { dist } as u8;
                    if !self.is_hidden(k, direction) {
                        out.push(OnionStep { k, cell, frame: f });
                    }
                }
            }
            // Frame stepping stays literal: the walk covers `want` frames each
            // way and simply draws fewer ghosts across a hold. Drawing stepping
            // keeps walking until it has `want` distinct drawings.
            let reached = if self.by_key { seen.len() } else { dist };
            if reached >= want as usize {
                break;
            }
        }
        out
    }
}

/// The pins of `layer` to ghost at `frame`, each with the drawing it shows.
///
/// Same rules as [`OnionConfig::steps`]: a pin on the current frame's own
/// drawing is skipped, since it would sit exactly on top of it, and a drawing
/// two pins share is ghosted once, in the first pin's colour.
pub fn pin_ghosts(layer: &Layer, frame: usize, frame_count: usize) -> Vec<(OnionPin, CellId)> {
    let current = layer.resolve(frame);
    let mut out: Vec<(OnionPin, CellId)> = Vec::new();
    for pin in &layer.onion_pins {
        if !pin.visible || pin.frame >= frame_count {
            continue;
        }
        let Some(cell) = layer.resolve(pin.frame) else {
            continue;
        };
        if Some(cell) != current && !out.iter().any(|&(_, c)| c == cell) {
            out.push((*pin, cell));
        }
    }
    out
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

    #[test]
    fn hiding_the_nearest_offset_keeps_the_next_one_in_its_slot() {
        let layer = on_twos(12, 3);
        let mut cfg = OnionConfig {
            enabled: true,
            prev: 2,
            by_key: true,
            ..Default::default()
        };
        cfg.toggle_hidden(1, OnionDirection::Prev);
        // Frame 7: −1 is cell 1 (hidden), −2 is cell 0, first reached on
        // frame 2 of its hold. Cell 1's hold must not slide into the gap, and
        // −2 keeps k = 2 and so its own alpha.
        let steps = cfg.steps(&layer, 7, 12, OnionDirection::Prev);
        assert_eq!(steps, vec![OnionStep { k: 2, cell: 0, frame: 2 }]);
        // Next is untouched by a Prev toggle.
        cfg.next = 1;
        assert_eq!(cfg.steps(&layer, 7, 12, OnionDirection::Next).len(), 1);
    }

    #[test]
    fn toggling_twice_shows_the_offset_again() {
        let mut cfg = OnionConfig::default();
        cfg.toggle_hidden(3, OnionDirection::Next);
        assert!(cfg.is_hidden(3, OnionDirection::Next));
        assert!(!cfg.is_hidden(3, OnionDirection::Prev));
        cfg.toggle_hidden(3, OnionDirection::Next);
        assert!(!cfg.is_hidden(3, OnionDirection::Next));
        // Out-of-range slots are ignored rather than overflowing the shift.
        cfg.toggle_hidden(0, OnionDirection::Next);
        cfg.toggle_hidden(9, OnionDirection::Next);
        assert_eq!(cfg.next_hidden, 0);
    }

    #[test]
    fn frame_stepping_slots_are_frame_distances() {
        // Keys on 0, 3, 6, 9: from frame 7, one frame back is the current
        // hold, two back is frame 5 (cell 1's hold), three back frame 4.
        let layer = on_twos(12, 3);
        let mut cfg = OnionConfig {
            enabled: true,
            prev: 3,
            by_key: false,
            ..Default::default()
        };
        let steps = cfg.steps(&layer, 7, 12, OnionDirection::Prev);
        assert_eq!(steps, vec![OnionStep { k: 2, cell: 1, frame: 5 }]);
        cfg.toggle_hidden(2, OnionDirection::Prev);
        assert!(cfg.steps(&layer, 7, 12, OnionDirection::Prev).is_empty());
    }

    fn pin(frame: usize) -> OnionPin {
        OnionPin {
            frame,
            tint: PIN_TINTS[0],
            visible: true,
        }
    }

    #[test]
    fn pins_skip_the_current_drawing_and_what_they_cannot_show() {
        let mut layer = on_twos(12, 3);
        layer.onion_pins = vec![
            pin(0),
            // Same drawing as frame 0: ghosted once.
            pin(2),
            // Frame 7's own drawing (key at 6).
            pin(6),
            // Past the end of the timeline.
            pin(40),
            OnionPin {
                visible: false,
                ..pin(9)
            },
            pin(4),
        ];
        let cells: Vec<CellId> = pin_ghosts(&layer, 7, 12).iter().map(|&(_, c)| c).collect();
        assert_eq!(cells, vec![0, 1]);
    }
}
