//! Onion-skin configuration.
//!
//! Composition: previous frames tinted (default blue) below the current cell,
//! next frames tinted (default red) above. Alpha falls off with distance.

#[derive(Clone, Copy, Debug)]
pub struct OnionConfig {
    pub enabled: bool,
    /// Number of previous frames to show (0..=8).
    pub prev: u8,
    /// Number of next frames to show (0..=8).
    pub next: u8,
    /// Multiplicative tint applied to previous frames (unmultiplied RGBA).
    pub prev_tint: [u8; 4],
    /// Multiplicative tint applied to next frames (unmultiplied RGBA).
    pub next_tint: [u8; 4],
    /// Alpha falloff exponent — higher = older frames fade faster.
    pub falloff: f32,
    /// Max alpha (0..=1) for the nearest onion-skin frame.
    pub max_alpha: f32,
}

impl Default for OnionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            prev: 1,
            next: 1,
            prev_tint: [120, 160, 255, 255],
            next_tint: [255, 130, 130, 255],
            falloff: 1.6,
            max_alpha: 0.55,
        }
    }
}

impl OnionConfig {
    /// Returns the tint (rgba unmultiplied) for an onion frame `k` steps away
    /// from current, in the given direction.
    pub fn tint_for(&self, k: u8, direction: OnionDirection) -> [u8; 4] {
        let base = match direction {
            OnionDirection::Prev => self.prev_tint,
            OnionDirection::Next => self.next_tint,
        };
        let count = match direction {
            OnionDirection::Prev => self.prev,
            OnionDirection::Next => self.next,
        };
        let count = count.max(1) as f32;
        let t = (k.max(1) as f32) / count;
        let alpha = self.max_alpha * (1.0 - t).max(0.0).powf(self.falloff);
        [
            base[0],
            base[1],
            base[2],
            (alpha.clamp(0.0, 1.0) * base[3] as f32).round() as u8,
        ]
    }
}

#[derive(Clone, Copy, Debug)]
pub enum OnionDirection {
    Prev,
    Next,
}
