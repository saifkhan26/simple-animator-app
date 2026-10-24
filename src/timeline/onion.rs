//! Onion-skin configuration.
//!
//! Composition: previous drawings tinted (default blue) and next drawings
//! tinted (default red), all below the current cell so they never cover the
//! lines being drawn.
//!
//! Which ghosts show and how strongly is set offset by offset, the way
//! Krita's Onion Skins docker does it: a strip of `−8 … +8`, each offset lit
//! or not and with its own opacity.
//!
//! The ghosts are drawn from *colorized* textures (see `AppState::ghost_image`),
//! not by multiplying a tint over the cell texture — a multiply leaves black
//! line art black, which is why ghosts used to read as a grey smudge instead of
//! blue-past / red-future.

use crate::doc::layer::{CellId, Layer};

/// How many offsets the strip covers each way.
pub const ONION_RANGE: u8 = 8;

/// Alpha floor for the outermost ghost under the old sliders, as a fraction
/// of their `max_alpha`. Only used to carry those settings over.
const ALPHA_FLOOR: f32 = 0.18;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(from = "OnionRepr", into = "OnionRepr")]
pub struct OnionConfig {
    pub enabled: bool,
    /// Silhouette color for previous drawings (unmultiplied RGB).
    pub prev_tint: [u8; 3],
    /// Silhouette color for next drawings (unmultiplied RGB).
    pub next_tint: [u8; 3],
    /// Count distinct *drawings* rather than frames when stepping outward, so
    /// animation on twos/threes still shows the real drawings back instead
    /// of the same held cell repeated.
    pub by_key: bool,
    /// Lit offsets: bit `k - 1` shows the ghost `k` steps back. The walk
    /// reaches as far as the farthest lit one. An unlit offset keeps its
    /// slot, so turning off −1 leaves −2 where it was and at its opacity.
    pub prev_shown: u8,
    /// Same as `prev_shown`, for the ghosts ahead.
    pub next_shown: u8,
    /// Opacity (0..=1) of the ghost `k` steps back, at index `k - 1`. Kept
    /// for unlit offsets too, so lighting one again brings its bar back.
    pub prev_opacity: [f32; ONION_RANGE as usize],
    /// Same as `prev_opacity`, for the ghosts ahead.
    pub next_opacity: [f32; ONION_RANGE as usize],
    /// What an Alt-click solo put aside. Not saved.
    solo: Option<Solo>,
}

/// One offset shown alone, and the lit offsets to put back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Solo {
    k: u8,
    dir: OnionDirection,
    saved: (u8, u8),
}

impl Solo {
    /// The lit masks while this solo holds.
    fn masks(&self) -> (u8, u8) {
        let bit = 1 << (self.k - 1);
        match self.dir {
            OnionDirection::Prev => (bit, 0),
            OnionDirection::Next => (0, bit),
        }
    }
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OnionPin {
    pub frame: usize,
    pub tint: [u8; 3],
    pub visible: bool,
    /// How strongly its ghost shows, 0..=1.
    pub opacity: f32,
}

impl OnionPin {
    /// Alpha (0..=255) its ghost is drawn at.
    pub fn alpha(&self) -> u8 {
        (self.opacity.clamp(0.0, 1.0) * 255.0).round() as u8
    }
}

/// Offsets lit by default and by Reset: three each way.
const DEFAULT_SHOWN: u8 = 0b111;

/// Default opacity of the ghost `k` steps away: 75% next to the current
/// drawing, a little less each step out.
fn default_opacity(k: u8) -> f32 {
    0.75 - f32::from(k.saturating_sub(1)) * 0.08
}

fn default_opacities() -> [f32; ONION_RANGE as usize] {
    std::array::from_fn(|i| default_opacity(i as u8 + 1))
}

impl Default for OnionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            prev_tint: [90, 150, 255],
            next_tint: [255, 90, 90],
            by_key: true,
            prev_shown: DEFAULT_SHOWN,
            next_shown: DEFAULT_SHOWN,
            prev_opacity: default_opacities(),
            next_opacity: default_opacities(),
            solo: None,
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

    fn shown_mask(&self, direction: OnionDirection) -> u8 {
        match direction {
            OnionDirection::Prev => self.prev_shown,
            OnionDirection::Next => self.next_shown,
        }
    }

    /// Whether the ghost `k` steps away in `direction` is lit.
    pub fn is_shown(&self, k: u8, direction: OnionDirection) -> bool {
        matches!(k, 1..=ONION_RANGE) && self.shown_mask(direction) & (1 << (k - 1)) != 0
    }

    /// Light or unlight the ghost `k` steps away in `direction`.
    pub fn toggle_shown(&mut self, k: u8, direction: OnionDirection) {
        if !matches!(k, 1..=ONION_RANGE) {
            return;
        }
        let mask = match direction {
            OnionDirection::Prev => &mut self.prev_shown,
            OnionDirection::Next => &mut self.next_shown,
        };
        *mask ^= 1 << (k - 1);
    }

    /// Show only the ghost `k` steps away in `direction`; the same again
    /// puts back what was lit before. Soloing another offset while one is
    /// soloed moves the solo and still remembers the original set. Any other
    /// change to what's lit ends the solo.
    pub fn solo(&mut self, k: u8, dir: OnionDirection) {
        if !matches!(k, 1..=ONION_RANGE) {
            return;
        }
        let now = (self.prev_shown, self.next_shown);
        let held = self.solo.filter(|s| s.masks() == now);
        match held {
            Some(s) if s.k == k && s.dir == dir => {
                (self.prev_shown, self.next_shown) = s.saved;
                self.solo = None;
            }
            _ => {
                let saved = held.map_or(now, |s| s.saved);
                let s = Solo { k, dir, saved };
                (self.prev_shown, self.next_shown) = s.masks();
                self.solo = Some(s);
            }
        }
    }

    /// Whether the ghost `k` steps away in `direction` is the one soloed.
    pub fn is_soloed(&self, k: u8, dir: OnionDirection) -> bool {
        self.solo.is_some_and(|s| {
            s.k == k && s.dir == dir && s.masks() == (self.prev_shown, self.next_shown)
        })
    }

    /// Opacity (0..=1) of the ghost `k` steps away in `direction`.
    pub fn opacity(&self, k: u8, direction: OnionDirection) -> f32 {
        if !matches!(k, 1..=ONION_RANGE) {
            return 0.0;
        }
        let i = usize::from(k - 1);
        match direction {
            OnionDirection::Prev => self.prev_opacity[i],
            OnionDirection::Next => self.next_opacity[i],
        }
    }

    pub fn set_opacity(&mut self, k: u8, direction: OnionDirection, v: f32) {
        if !matches!(k, 1..=ONION_RANGE) {
            return;
        }
        let i = usize::from(k - 1);
        let slot = match direction {
            OnionDirection::Prev => &mut self.prev_opacity[i],
            OnionDirection::Next => &mut self.next_opacity[i],
        };
        *slot = v.clamp(0.0, 1.0);
    }

    /// The farthest lit offset in `direction`, 0 when none is.
    pub fn reach(&self, direction: OnionDirection) -> u8 {
        (u8::BITS - self.shown_mask(direction).leading_zeros()) as u8
    }

    /// Light exactly the nearest `prev` offsets back and `next` ahead.
    #[cfg(test)]
    pub fn light(&mut self, prev: u8, next: u8) {
        let mask = |n: u8| ((1u16 << n.min(ONION_RANGE)) - 1) as u8;
        (self.prev_shown, self.next_shown) = (mask(prev), mask(next));
    }

    /// Bars back to the default fade, three lit each way. Colours, the
    /// on/off switch and the stepping are left alone.
    pub fn reset_bars(&mut self) {
        let d = Self::default();
        (self.prev_shown, self.next_shown) = (d.prev_shown, d.next_shown);
        (self.prev_opacity, self.next_opacity) = (d.prev_opacity, d.next_opacity);
        self.solo = None;
    }

    /// Alpha (0..=255) for a ghost `k` steps away from the current frame:
    /// its bar.
    pub fn alpha_for(&self, k: u8, direction: OnionDirection) -> u8 {
        (self.opacity(k, direction).clamp(0.0, 1.0) * 255.0).round() as u8
    }

    /// Opacity a new pin starts at: as strong as the nearest bar either
    /// side, which is what every pin showed at before they had their own.
    pub fn pin_opacity(&self) -> f32 {
        self.opacity(1, OnionDirection::Prev).max(self.opacity(1, OnionDirection::Next))
    }

    /// The ghosts for `layer` around `frame`, ordered nearest-first.
    ///
    /// A cell that resolves to the same drawing as the current frame is never
    /// emitted: on a hold that ghost would land exactly on top of the current
    /// cell, adding tint but no motion information.
    ///
    /// Each ghost's `k` is its slot in the range — the how-many-th drawing
    /// back when stepping by drawings, the frame distance when stepping by
    /// frames — so an unlit offset leaves the others where they were.
    pub fn steps(
        &self,
        layer: &Layer,
        frame: usize,
        frame_count: usize,
        direction: OnionDirection,
    ) -> Vec<OnionStep> {
        let want = self.reach(direction);
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
                // paint a second copy in the same place. An unlit ghost still
                // counts as seen, so a later frame of its hold can't step into
                // the slot it left.
                if Some(cell) != current && !seen.contains(&cell) {
                    seen.push(cell);
                    let k = if self.by_key { seen.len() } else { dist } as u8;
                    if self.is_shown(k, direction) {
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

/// How the onion settings are written to the prefs, and read back — also
/// from the build before the strip, whose Prev / Next / Max α / Falloff
/// sliders seed it.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct OnionRepr {
    enabled: bool,
    prev_tint: [u8; 3],
    next_tint: [u8; 3],
    by_key: bool,
    /// 1 once written by the strip. Missing — 0 — means the slider build
    /// wrote it, and the strip comes from the fields below.
    strip: u8,
    prev_shown: u8,
    next_shown: u8,
    prev_opacity: [f32; ONION_RANGE as usize],
    next_opacity: [f32; ONION_RANGE as usize],
    // The slider build's settings, read once and never written. Plain
    // types, not `Option`: eframe's RON has no implicit `Some`, and one field
    // it can't read resets every preference.
    #[serde(skip_serializing)]
    prev: u8,
    #[serde(skip_serializing)]
    next: u8,
    #[serde(skip_serializing)]
    falloff: f32,
    #[serde(skip_serializing)]
    max_alpha: f32,
    #[serde(skip_serializing)]
    prev_hidden: u8,
    #[serde(skip_serializing)]
    next_hidden: u8,
}

impl Default for OnionRepr {
    /// What a field missing from the prefs reads as. `strip` stays 0, so a
    /// table without it is taken for the slider build's.
    fn default() -> Self {
        Self {
            strip: 0,
            ..OnionConfig::default().into()
        }
    }
}

impl From<OnionConfig> for OnionRepr {
    fn from(c: OnionConfig) -> Self {
        Self {
            enabled: c.enabled,
            prev_tint: c.prev_tint,
            next_tint: c.next_tint,
            by_key: c.by_key,
            strip: 1,
            prev_shown: c.prev_shown,
            next_shown: c.next_shown,
            prev_opacity: c.prev_opacity,
            next_opacity: c.next_opacity,
            // The slider build's defaults, for a partial old table.
            prev: 1,
            next: 1,
            falloff: 1.2,
            max_alpha: 0.75,
            prev_hidden: 0,
            next_hidden: 0,
        }
    }
}

impl From<OnionRepr> for OnionConfig {
    fn from(r: OnionRepr) -> Self {
        let mut c = OnionConfig {
            enabled: r.enabled,
            prev_tint: r.prev_tint,
            next_tint: r.next_tint,
            by_key: r.by_key,
            prev_shown: r.prev_shown,
            next_shown: r.next_shown,
            prev_opacity: r.prev_opacity.map(|v| v.clamp(0.0, 1.0)),
            next_opacity: r.next_opacity.map(|v| v.clamp(0.0, 1.0)),
            solo: None,
        };
        if r.strip == 0 {
            (c.prev_shown, c.prev_opacity) = from_sliders(r.prev, r.prev_hidden, r.max_alpha, r.falloff);
            (c.next_shown, c.next_opacity) = from_sliders(r.next, r.next_hidden, r.max_alpha, r.falloff);
        }
        c
    }
}

/// The strip one side of the old sliders amounts to: `count` offsets lit
/// less the hidden ones, each bar at the alpha the old curve gave it. Bars
/// past the old range get the default fade, never above the last old one.
fn from_sliders(
    count: u8,
    hidden: u8,
    max_alpha: f32,
    falloff: f32,
) -> (u8, [f32; ONION_RANGE as usize]) {
    let count = count.min(ONION_RANGE);
    let shown = ((1u16 << count) - 1) as u8 & !hidden;
    let old = |k: u8| {
        let t = f32::from(k - 1) / f32::from(count.max(1));
        let a = max_alpha * (1.0 - t).max(0.0).powf(falloff).max(ALPHA_FLOOR);
        a.clamp(0.0, 1.0)
    };
    let last = old(count.max(1));
    let opacity = std::array::from_fn(|i| {
        let k = i as u8 + 1;
        if k <= count { old(k) } else { default_opacity(k).min(last) }
    });
    (shown, opacity)
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

    /// `n` offsets lit each way.
    fn lit(n: u8) -> OnionConfig {
        let mask = ((1u16 << n) - 1) as u8;
        OnionConfig {
            enabled: true,
            prev_shown: mask,
            next_shown: mask,
            ..Default::default()
        }
    }

    #[test]
    fn default_bars_fade_out_and_stay_visible() {
        let cfg = OnionConfig::default();
        let a: Vec<u8> = (1..=ONION_RANGE)
            .map(|k| cfg.alpha_for(k, OnionDirection::Prev))
            .collect();
        assert!(a.windows(2).all(|w| w[0] > w[1]), "{a:?}");
        assert!(a[ONION_RANGE as usize - 1] > 0, "{a:?}");
        assert_eq!((cfg.reach(OnionDirection::Prev), cfg.reach(OnionDirection::Next)), (3, 3));
    }

    #[test]
    fn a_bar_sets_its_ghosts_alpha_alone() {
        let mut cfg = OnionConfig::default();
        cfg.set_opacity(2, OnionDirection::Next, 0.5);
        assert_eq!(cfg.alpha_for(2, OnionDirection::Next), 128);
        assert_eq!(cfg.alpha_for(2, OnionDirection::Prev), OnionConfig::default().alpha_for(2, OnionDirection::Prev));
        cfg.set_opacity(2, OnionDirection::Next, 7.0);
        assert_eq!(cfg.opacity(2, OnionDirection::Next), 1.0, "clamped");
        // Out-of-range offsets are ignored rather than indexing past the end.
        cfg.set_opacity(0, OnionDirection::Next, 0.1);
        cfg.set_opacity(9, OnionDirection::Next, 0.1);
        assert_eq!(cfg.opacity(9, OnionDirection::Next), 0.0);
    }

    #[test]
    fn reach_is_the_farthest_lit_offset() {
        let mut cfg = OnionConfig { prev_shown: 0, ..Default::default() };
        assert_eq!(cfg.reach(OnionDirection::Prev), 0);
        cfg.toggle_shown(5, OnionDirection::Prev);
        cfg.toggle_shown(2, OnionDirection::Prev);
        assert_eq!(cfg.reach(OnionDirection::Prev), 5);
        cfg.toggle_shown(8, OnionDirection::Prev);
        assert_eq!(cfg.reach(OnionDirection::Prev), 8);
    }

    #[test]
    fn by_key_steps_over_holds_to_distinct_drawings() {
        let layer = on_twos(12, 3);
        let cfg = lit(2);
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
        let cfg = lit(3);
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
        let cfg = lit(4);
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
    fn unlighting_the_nearest_offset_keeps_the_next_one_in_its_slot() {
        let layer = on_twos(12, 3);
        let mut cfg = lit(2);
        cfg.toggle_shown(1, OnionDirection::Prev);
        // Frame 7: −1 is cell 1 (unlit), −2 is cell 0, first reached on
        // frame 2 of its hold. Cell 1's hold must not slide into the gap, and
        // −2 keeps k = 2 and so its own bar.
        let steps = cfg.steps(&layer, 7, 12, OnionDirection::Prev);
        assert_eq!(steps, vec![OnionStep { k: 2, cell: 0, frame: 2 }]);
        // Next is untouched by a Prev toggle: frame 9's drawing, the only
        // one ahead.
        assert_eq!(cfg.steps(&layer, 7, 12, OnionDirection::Next).len(), 1);
    }

    #[test]
    fn a_lone_far_offset_walks_past_unlit_ones() {
        // Only −3 lit: the walk still has to count −1 and −2 to find it.
        let layer = on_twos(12, 1);
        let cfg = OnionConfig {
            enabled: true,
            prev_shown: 0b100,
            ..Default::default()
        };
        let steps = cfg.steps(&layer, 6, 12, OnionDirection::Prev);
        assert_eq!(steps, vec![OnionStep { k: 3, cell: 3, frame: 3 }]);
    }

    #[test]
    fn toggling_twice_lights_the_offset_again() {
        let mut cfg = OnionConfig::default();
        cfg.toggle_shown(3, OnionDirection::Next);
        assert!(!cfg.is_shown(3, OnionDirection::Next));
        assert!(cfg.is_shown(3, OnionDirection::Prev));
        cfg.toggle_shown(3, OnionDirection::Next);
        assert!(cfg.is_shown(3, OnionDirection::Next));
        // Out-of-range slots are ignored rather than overflowing the shift.
        let before = cfg.next_shown;
        cfg.toggle_shown(0, OnionDirection::Next);
        cfg.toggle_shown(9, OnionDirection::Next);
        assert_eq!(cfg.next_shown, before);
    }

    #[test]
    fn frame_stepping_slots_are_frame_distances() {
        // Keys on 0, 3, 6, 9: from frame 7, one frame back is the current
        // hold, two back is frame 5 (cell 1's hold), three back frame 4.
        let layer = on_twos(12, 3);
        let mut cfg = OnionConfig {
            by_key: false,
            ..lit(3)
        };
        let steps = cfg.steps(&layer, 7, 12, OnionDirection::Prev);
        assert_eq!(steps, vec![OnionStep { k: 2, cell: 1, frame: 5 }]);
        cfg.toggle_shown(2, OnionDirection::Prev);
        assert!(cfg.steps(&layer, 7, 12, OnionDirection::Prev).is_empty());
    }

    #[test]
    fn solo_shows_one_offset_and_the_same_again_restores() {
        let mut cfg = OnionConfig::default();
        let before = (cfg.prev_shown, cfg.next_shown);
        cfg.solo(2, OnionDirection::Next);
        assert_eq!((cfg.prev_shown, cfg.next_shown), (0, 0b10));
        assert!(cfg.is_soloed(2, OnionDirection::Next));
        // Moving the solo keeps the set from before the first one.
        cfg.solo(1, OnionDirection::Prev);
        assert_eq!((cfg.prev_shown, cfg.next_shown), (0b1, 0));
        assert!(!cfg.is_soloed(2, OnionDirection::Next));
        cfg.solo(1, OnionDirection::Prev);
        assert_eq!((cfg.prev_shown, cfg.next_shown), before);
        assert!(!cfg.is_soloed(1, OnionDirection::Prev));
    }

    #[test]
    fn changing_what_is_lit_ends_a_solo() {
        let mut cfg = OnionConfig::default();
        cfg.solo(2, OnionDirection::Next);
        cfg.toggle_shown(4, OnionDirection::Prev);
        assert!(!cfg.is_soloed(2, OnionDirection::Next));
        // Alt-clicking it again now solos afresh instead of restoring a set
        // the user has since moved on from.
        cfg.solo(2, OnionDirection::Next);
        assert_eq!((cfg.prev_shown, cfg.next_shown), (0, 0b10));
        cfg.solo(2, OnionDirection::Next);
        assert_eq!((cfg.prev_shown, cfg.next_shown), (0b1000, 0b10));
    }

    #[test]
    fn reset_brings_back_the_default_bars_only() {
        let mut cfg = OnionConfig {
            enabled: true,
            by_key: false,
            prev_tint: [1, 2, 3],
            prev_shown: 0b1_0000,
            ..Default::default()
        };
        cfg.set_opacity(1, OnionDirection::Prev, 0.1);
        cfg.reset_bars();
        let d = OnionConfig::default();
        assert_eq!((cfg.prev_shown, cfg.next_shown), (d.prev_shown, d.next_shown));
        assert_eq!(cfg.prev_opacity, d.prev_opacity);
        assert!(cfg.enabled && !cfg.by_key);
        assert_eq!(cfg.prev_tint, [1, 2, 3]);
    }

    #[test]
    fn slider_settings_carry_over_to_the_strip() {
        // The panel as it was: Prev 3 with −2 hidden, Next 1, Max α 0.3.
        let old = "enabled = true\nprev = 3\nnext = 1\nprev_tint = [1, 2, 3]\n\
                   next_tint = [4, 5, 6]\nfalloff = 2.3\nmax_alpha = 0.3\nby_key = false\n\
                   prev_hidden = 2\nnext_hidden = 0\n";
        let cfg: OnionConfig = toml::from_str(old).unwrap();
        assert!(cfg.enabled && !cfg.by_key);
        assert_eq!((cfg.prev_tint, cfg.next_tint), ([1, 2, 3], [4, 5, 6]));
        assert_eq!((cfg.prev_shown, cfg.next_shown), (0b101, 0b1));
        // The nearest ghost was at Max α, and so is its bar.
        assert!((cfg.opacity(1, OnionDirection::Prev) - 0.3).abs() < 1e-6);
        assert!((cfg.opacity(1, OnionDirection::Next) - 0.3).abs() < 1e-6);
        let p: Vec<f32> = cfg.prev_opacity.to_vec();
        assert!(p.windows(2).all(|w| w[0] >= w[1]), "{p:?}");
        assert!(p.iter().all(|&v| v > 0.0), "{p:?}");
    }

    #[test]
    fn the_strip_round_trips_and_a_bare_table_takes_the_slider_defaults() {
        let mut cfg = OnionConfig { enabled: true, ..Default::default() };
        cfg.toggle_shown(6, OnionDirection::Next);
        cfg.set_opacity(6, OnionDirection::Next, 0.42);
        let text = toml::to_string(&cfg).unwrap();
        assert!(!text.contains("max_alpha"), "old fields aren't written: {text}");
        let back: OnionConfig = toml::from_str(&text).unwrap();
        assert_eq!((back.prev_shown, back.next_shown), (cfg.prev_shown, cfg.next_shown));
        assert_eq!(back.next_opacity, cfg.next_opacity);
        assert!(back.enabled);
        // A table from the slider build that only set `enabled` meant
        // Prev 1 / Next 1 then.
        let bare: OnionConfig = toml::from_str("enabled = true").unwrap();
        assert_eq!((bare.prev_shown, bare.next_shown), (0b1, 0b1));
    }

    fn pin(frame: usize) -> OnionPin {
        OnionPin {
            frame,
            tint: PIN_TINTS[0],
            visible: true,
            opacity: 0.75,
        }
    }

    #[test]
    fn a_new_pin_starts_as_strong_as_the_nearest_bar() {
        let mut cfg = OnionConfig::default();
        assert_eq!(cfg.pin_opacity(), 0.75);
        cfg.set_opacity(1, OnionDirection::Prev, 0.2);
        cfg.set_opacity(1, OnionDirection::Next, 0.4);
        assert_eq!(cfg.pin_opacity(), 0.4);
        assert_eq!(OnionPin { opacity: 0.4, ..pin(0) }.alpha(), 102);
        assert_eq!(OnionPin { opacity: 3.0, ..pin(0) }.alpha(), 255, "clamped");
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
