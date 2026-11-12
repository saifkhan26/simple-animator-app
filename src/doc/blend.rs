//! Layer blend modes: how a layer's colours mix with what is under it.
//!
//! Every mode here is *separable*: each of red, green and blue mixes on its
//! own through [`BlendMode::mix`], and the result is composited the way the
//! W3C's Compositing and Blending Level 1 spec — and Krita, and every paint
//! program after Photoshop — does it:
//!
//! ```text
//! αo = αs + αb·(1 − αs)
//! co = (αs·(1 − αb)·cs + αb·(1 − αs)·cb + αs·αb·B(cb, cs)) / αo
//! ```
//!
//! `cs`, `cb` are the layer's and the backdrop's colours, unmultiplied, and
//! `B` is the mode's mix. Where nothing is under a layer it shows as itself;
//! where it is transparent the backdrop is left alone. Normal is `B = cs`,
//! which is plain "over".
//!
//! The same formulas run on the CPU (export, merged groups, Krita) through
//! [`blend_px`] and on the GPU (the canvas) in `ui/canvas_blend.wgsl`, which
//! keeps a copy of `mix` in step with this one.
//!
//! To add a mode: a variant at the END of [`BlendMode`] (the file stores its
//! index), a row in each `match` below, a case in the shader's `mix`, and the
//! tests' expected values.

/// How a layer mixes with what is under it. Groups are always Normal.
///
/// The `.anim` file stores the variant's index, so new modes go at the end,
/// never in between, and none is ever removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Add,
    Overlay,
}

impl BlendMode {
    /// Every mode, in the order the menu lists them.
    pub const ALL: [BlendMode; 5] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Add,
        BlendMode::Overlay,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiply",
            BlendMode::Screen => "Screen",
            BlendMode::Add => "Add",
            BlendMode::Overlay => "Overlay",
        }
    }

    /// What the mode is for, in a sentence: the menu's hover text.
    pub fn hint(self) -> &'static str {
        match self {
            BlendMode::Normal => "Covers what is under it.",
            BlendMode::Multiply => "Darkens: white vanishes, colours deepen what is under them. For shadows.",
            BlendMode::Screen => "Lightens: black vanishes, colours brighten what is under them. For light.",
            BlendMode::Add => "Lightens more strongly, burning towards white. For glows and light effects.",
            BlendMode::Overlay => "Darkens the darks and lightens the lights under it; mid grey vanishes. For tone.",
        }
    }

    /// Krita's name for the mode, its `compositeop` in a `.kra`.
    pub fn krita_id(self) -> &'static str {
        match self {
            BlendMode::Normal => "normal",
            BlendMode::Multiply => "multiply",
            BlendMode::Screen => "screen",
            BlendMode::Add => "add",
            BlendMode::Overlay => "overlay",
        }
    }

    /// The mode Krita's `compositeop` names, or `None` for one this app
    /// doesn't have. Krita's Linear Dodge is the same sum as its Addition.
    pub fn from_krita(id: &str) -> Option<BlendMode> {
        match id {
            "linear_dodge" => Some(BlendMode::Add),
            _ => Self::ALL.into_iter().find(|m| m.krita_id() == id),
        }
    }

    /// The number the canvas shader knows the mode by.
    pub fn shader_index(self) -> u32 {
        self as u32
    }

    /// `B(cb, cs)`: one channel of the layer, `cs`, mixed with the same
    /// channel under it, `cb`, both unmultiplied in 0..=1.
    pub fn mix(self, cb: f32, cs: f32) -> f32 {
        match self {
            BlendMode::Normal => cs,
            BlendMode::Multiply => cb * cs,
            BlendMode::Screen => cb + cs - cb * cs,
            BlendMode::Add => (cb + cs).min(1.0),
            // Hard light with the layers swapped: the backdrop decides.
            BlendMode::Overlay => {
                if cb <= 0.5 {
                    2.0 * cb * cs
                } else {
                    let b = 2.0 * cb - 1.0;
                    b + cs - b * cs
                }
            }
        }
    }
}

/// Lay one pixel `s` over one pixel `d` in `mode`, with `s`'s coverage taken
/// as `sa` (its alpha times the layer's opacity, 0..=1). Both unmultiplied
/// RGBA8, as canvases hold them; `d` is written. Rounds as the plain "over"
/// in `io::composite` does.
pub fn blend_px(mode: BlendMode, d: &mut [u8], s: &[u8], sa: f32) {
    if sa <= 0.0 {
        return;
    }
    let da = d[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        d[..4].copy_from_slice(&[0, 0, 0, 0]);
        return;
    }
    for c in 0..3 {
        let cs = s[c] as f32 / 255.0;
        let cb = d[c] as f32 / 255.0;
        let mixed = mode.mix(cb, cs);
        let co = (sa * (1.0 - da) * cs + da * (1.0 - sa) * cb + sa * da * mixed) / out_a;
        d[c] = (co * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    d[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn over(mode: BlendMode, d: [u8; 4], s: [u8; 4], op: f32) -> [u8; 4] {
        let mut d = d;
        blend_px(mode, &mut d, &s, s[3] as f32 / 255.0 * op);
        d
    }

    #[test]
    fn the_file_s_numbers_never_move() {
        // The .anim stores these. Append new modes; never reorder.
        let at = |m: BlendMode| postcard::to_stdvec(&m).unwrap();
        assert_eq!(at(BlendMode::Normal), [0]);
        assert_eq!(at(BlendMode::Multiply), [1]);
        assert_eq!(at(BlendMode::Screen), [2]);
        assert_eq!(at(BlendMode::Add), [3]);
        assert_eq!(at(BlendMode::Overlay), [4]);
        assert_eq!(BlendMode::ALL.len(), 5, "a new mode needs a line here");
        for (i, m) in BlendMode::ALL.into_iter().enumerate() {
            assert_eq!(m.shader_index(), i as u32);
        }
    }

    #[test]
    fn each_mode_mixes_as_its_formula_says() {
        let grey = 0.25;
        assert_eq!(BlendMode::Normal.mix(grey, 0.8), 0.8);
        assert_eq!(BlendMode::Multiply.mix(1.0, 0.8), 0.8, "white vanishes under multiply");
        assert_eq!(BlendMode::Multiply.mix(0.5, 0.5), 0.25);
        assert_eq!(BlendMode::Screen.mix(0.0, 0.3), 0.3, "black vanishes under screen");
        assert_eq!(BlendMode::Screen.mix(0.5, 0.5), 0.75);
        assert_eq!(BlendMode::Add.mix(0.7, 0.6), 1.0, "add burns to white");
        assert_eq!(BlendMode::Add.mix(0.2, 0.3), 0.5);
        assert_eq!(BlendMode::Overlay.mix(0.25, 0.5), 0.25, "mid grey vanishes under overlay");
        assert_eq!(BlendMode::Overlay.mix(0.75, 0.5), 0.75);
        assert_eq!(BlendMode::Overlay.mix(0.25, 1.0), 0.5);
        assert!((BlendMode::Overlay.mix(0.75, 0.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn over_nothing_a_layer_shows_as_itself_and_transparent_leaves_the_backdrop() {
        for m in BlendMode::ALL {
            assert_eq!(over(m, [0, 0, 0, 0], [200, 40, 90, 255], 1.0), [200, 40, 90, 255], "{m:?} over bare");
            assert_eq!(over(m, [10, 20, 30, 255], [200, 40, 90, 0], 1.0), [10, 20, 30, 255], "{m:?} transparent");
        }
    }

    #[test]
    fn opaque_over_opaque_is_the_mix() {
        let d = [255, 128, 64, 255];
        let s = [128, 128, 128, 255];
        assert_eq!(over(BlendMode::Multiply, d, s, 1.0), [128, 64, 32, 255]);
        assert_eq!(over(BlendMode::Screen, d, s, 1.0), [255, 192, 160, 255]);
        assert_eq!(over(BlendMode::Add, d, s, 1.0), [255, 255, 192, 255]);
        // Normal through the formula is plain over.
        assert_eq!(over(BlendMode::Normal, d, s, 1.0), s);
    }

    #[test]
    fn opacity_fades_toward_the_backdrop() {
        let d = [200, 200, 200, 255];
        let s = [0, 0, 0, 255];
        // Half a black multiply over light grey: half way to black.
        assert_eq!(over(BlendMode::Multiply, d, s, 0.5), [100, 100, 100, 255]);
        assert_eq!(over(BlendMode::Multiply, d, s, 0.0), d);
    }

    #[test]
    fn krita_names_round_trip() {
        for m in BlendMode::ALL {
            assert_eq!(BlendMode::from_krita(m.krita_id()), Some(m));
        }
        assert_eq!(BlendMode::from_krita("linear_dodge"), Some(BlendMode::Add));
        assert_eq!(BlendMode::from_krita("color_burn"), None);
    }
}
