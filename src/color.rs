//! HSL, the colour language of the UI, and the harmony sets the Color panel
//! builds around a base colour.
//!
//! Only the UI speaks HSL. Pixels, the pinned palette, tints and prefs stay
//! 8-bit sRGB: the GPU, PNG, GIF, MP4 and Krita all want those bytes, so
//! storing HSL would only add a lossy hop. HSL here is the CSS kind — computed
//! straight on the gamma-encoded sRGB values.

use egui::Color32;
use serde::{Deserialize, Serialize};

/// A colour as hue (degrees, `0..360`), saturation and lightness (`0..=1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsl {
    pub h: f32,
    pub s: f32,
    pub l: f32,
}

impl Hsl {
    pub fn new(h: f32, s: f32, l: f32) -> Self {
        Self { h: wrap_hue(h), s: s.clamp(0.0, 1.0), l: l.clamp(0.0, 1.0) }
    }

    pub fn from_rgb(rgb: [u8; 3]) -> Self {
        let [r, g, b] = rgb.map(|c| c as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        let d = max - min;
        if d == 0.0 {
            return Self { h: 0.0, s: 0.0, l };
        }
        let s = d / (1.0 - (2.0 * l - 1.0).abs());
        let h = if max == r {
            60.0 * ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        Self { h: wrap_hue(h), s: s.clamp(0.0, 1.0), l }
    }

    /// `rgb` as HSL, keeping what 8-bit RGB cannot say from `prev`: the hue of
    /// a grey, and the hue and saturation of black and white. Without this a
    /// picker jumps to red the moment it reaches the centre or an end of the
    /// lightness bar. If `prev` already rounds to `rgb` it comes back
    /// untouched, so a value read back from the bytes every frame never drifts.
    pub fn from_rgb_keeping(rgb: [u8; 3], prev: Hsl) -> Self {
        if prev.to_rgb() == rgb {
            return prev;
        }
        let mut c = Self::from_rgb(rgb);
        let max = rgb.iter().copied().max().unwrap_or(0);
        let min = rgb.iter().copied().min().unwrap_or(0);
        if max == min {
            c.h = prev.h;
            if max == 0 || min == 255 {
                c.s = prev.s;
            }
        }
        c
    }

    pub fn to_rgb(self) -> [u8; 3] {
        let h = wrap_hue(self.h) / 60.0;
        let s = self.s.clamp(0.0, 1.0);
        let l = self.l.clamp(0.0, 1.0);
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = l - c / 2.0;
        [r, g, b].map(|v| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8)
    }

    pub fn to_color32(self) -> Color32 {
        let [r, g, b] = self.to_rgb();
        Color32::from_rgb(r, g, b)
    }

    /// `hsl(210, 50%, 40%)`, the form [`parse_color`] reads back.
    pub fn fmt(self) -> String {
        let h = self.h.round() as u32 % 360;
        let s = (self.s * 100.0).round() as u32;
        let l = (self.l * 100.0).round() as u32;
        format!("hsl({h}, {s}%, {l}%)")
    }
}

/// `rgb` written as `hsl(…)`.
pub fn fmt_rgb(rgb: [u8; 3]) -> String {
    Hsl::from_rgb(rgb).fmt()
}

/// A hue folded into `0..360`.
pub fn wrap_hue(h: f32) -> f32 {
    let w = h.rem_euclid(360.0);
    // `rem_euclid` of a tiny negative rounds up to exactly 360.
    if w >= 360.0 {
        0.0
    } else {
        w
    }
}

/// `d` folded into `-180..=180`: the short way round from one hue to another.
fn signed_hue(d: f32) -> f32 {
    let w = wrap_hue(d);
    if w > 180.0 {
        w - 360.0
    } else {
        w
    }
}

/// A colour typed or pasted as text: `hsl(h, s%, l%)`, `rgb(r, g, b)` or
/// hex (`#RGB`, `#RRGGBB`, `#` optional). HSL is what the app shows; the
/// other two are what other apps copy, so a paste from them still lands.
/// Commas and spaces both separate, as in CSS; an alpha part is ignored.
pub fn parse_color(text: &str) -> Option<[u8; 3]> {
    let t = text.trim().to_ascii_lowercase();
    if let Some(inner) = func_args(&t, &["hsla(", "hsl("]) {
        let mut it = inner.iter();
        let h = it.next()?.trim_end_matches("deg").parse::<f32>().ok()?;
        let s = percent(it.next()?)?;
        let l = percent(it.next()?)?;
        if !h.is_finite() {
            return None;
        }
        return Some(Hsl::new(h, s, l).to_rgb());
    }
    if let Some(inner) = func_args(&t, &["rgba(", "rgb("]) {
        let mut it = inner.iter();
        let mut ch = || it.next()?.parse::<u8>().ok();
        return Some([ch()?, ch()?, ch()?]);
    }
    let hex = t.strip_prefix('#').unwrap_or(&t);
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let nib = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok();
    match hex.len() {
        3 => Some([nib(0)? * 17, nib(1)? * 17, nib(2)? * 17]),
        6 => {
            let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
            Some([byte(0)?, byte(2)?, byte(4)?])
        }
        _ => None,
    }
}

/// The arguments of `name(…)`, split on commas, spaces and the alpha `/`.
fn func_args<'a>(t: &'a str, names: &[&str]) -> Option<Vec<&'a str>> {
    let inner = names
        .iter()
        .find_map(|n| t.strip_prefix(n))?
        .strip_suffix(')')?;
    Some(
        inner
            .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
            .filter(|p| !p.is_empty())
            .collect(),
    )
}

/// `40%` (or a bare `40`) as `0.4`; out of `0..=100` is no colour.
fn percent(p: &str) -> Option<f32> {
    let v = p.trim_end_matches('%').parse::<f32>().ok()?;
    (0.0..=100.0).contains(&v).then_some(v / 100.0)
}

/// Which colours the Color panel offers alongside the base.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Harmony {
    /// The base and the hue opposite it.
    #[default]
    Complementary,
    /// Five lightnesses of the base hue, darkest first.
    Monochromatic,
    /// The base flanked by its neighbours on the wheel.
    Analogous,
    /// Three hues a third of the wheel apart.
    Triadic,
    /// Two complementary pairs: the base, a hue beside it, and their
    /// opposites — a rectangle on the wheel.
    Tetradic,
}

impl Harmony {
    /// In the order the scheme picker lists them.
    pub const ALL: [Harmony; 5] = [
        Harmony::Complementary,
        Harmony::Monochromatic,
        Harmony::Analogous,
        Harmony::Triadic,
        Harmony::Tetradic,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Harmony::Complementary => "Complementary",
            Harmony::Monochromatic => "Monochromatic",
            Harmony::Analogous => "Analogous",
            Harmony::Triadic => "Triadic",
            Harmony::Tetradic => "Tetradic",
        }
    }

    /// Which member is the base. The flanked and stepped schemes put it in
    /// the middle, so the swatch row reads left to right around it.
    pub fn base_index(self) -> usize {
        match self {
            Harmony::Analogous => 1,
            Harmony::Monochromatic => 2,
            _ => 0,
        }
    }

    pub fn len(self) -> usize {
        match self {
            Harmony::Complementary => 2,
            Harmony::Monochromatic => 5,
            Harmony::Analogous | Harmony::Triadic => 3,
            Harmony::Tetradic => 4,
        }
    }
}

/// Spread limits, in degrees for the hue schemes and lightness for the
/// monochrome step. The lower ends keep members from collapsing onto the base.
pub const ANALOG_SPREAD: std::ops::RangeInclusive<f32> = 5.0..=90.0;
pub const TETRA_SPREAD: std::ops::RangeInclusive<f32> = 15.0..=165.0;
pub const MONO_STEP: std::ops::RangeInclusive<f32> = 0.02..=0.3;
/// How far from black and white the monochrome steps may go, so the ends of
/// a set stay colours rather than all turning into the same black.
const MONO_L: std::ops::RangeInclusive<f32> = 0.03..=0.97;

/// A harmony laid around a base colour, and which member the brush is on.
///
/// The set is its own thing, not a view of the brush: picking a member sets
/// the brush but leaves the set where it is, so you can hop between its
/// colours while painting. It moves when you drag on the wheel, or when the
/// brush colour changes from somewhere else ([`HarmonySet::rebase`]).
#[derive(Clone, Debug, PartialEq)]
pub struct HarmonySet {
    pub scheme: Harmony,
    pub base: Hsl,
    /// The member the brush is painting with.
    pub active: usize,
    pub analog_spread: f32,
    pub tetra_spread: f32,
    pub mono_step: f32,
}

impl HarmonySet {
    pub fn new(scheme: Harmony, rgb: [u8; 3]) -> Self {
        Self {
            scheme,
            base: Hsl::from_rgb(rgb),
            active: scheme.base_index(),
            analog_spread: 30.0,
            tetra_spread: 60.0,
            mono_step: 0.15,
        }
    }

    pub fn len(&self) -> usize {
        self.scheme.len()
    }

    /// Switch scheme around the same base; the brush goes back to the base.
    pub fn set_scheme(&mut self, scheme: Harmony) {
        self.scheme = scheme;
        self.active = scheme.base_index();
    }

    /// Hue of member `i` relative to the base.
    fn offset(&self, i: usize) -> f32 {
        let a = self.analog_spread;
        let t = self.tetra_spread;
        match (self.scheme, i) {
            (Harmony::Complementary, 1) => 180.0,
            (Harmony::Analogous, 0) => -a,
            (Harmony::Analogous, 2) => a,
            (Harmony::Triadic, 1) => 120.0,
            (Harmony::Triadic, 2) => 240.0,
            (Harmony::Tetradic, 1) => t,
            (Harmony::Tetradic, 2) => 180.0,
            (Harmony::Tetradic, 3) => 180.0 + t,
            _ => 0.0,
        }
    }

    pub fn members(&self) -> Vec<Hsl> {
        let b = self.base;
        if self.scheme == Harmony::Monochromatic {
            return (-2i32..=2)
                .map(|k| {
                    // The base itself is never clamped, or a black base would
                    // stop matching the brush it came from.
                    let l = if k == 0 {
                        b.l
                    } else {
                        (b.l + k as f32 * self.mono_step).clamp(*MONO_L.start(), *MONO_L.end())
                    };
                    Hsl { l, ..b }
                })
                .collect();
        }
        (0..self.len())
            .map(|i| Hsl { h: wrap_hue(b.h + self.offset(i)), ..b })
            .collect()
    }

    pub fn member(&self, i: usize) -> Hsl {
        self.members()[i.min(self.len() - 1)]
    }

    pub fn active_rgb(&self) -> [u8; 3] {
        self.member(self.active).to_rgb()
    }

    /// Paint with member `i`. The set stays put.
    pub fn pick(&mut self, i: usize) {
        self.active = i.min(self.len() - 1);
    }

    /// Member `i` dragged to hue `h`, saturation `s` on the wheel (or typed
    /// into the H/S fields). The fixed shapes turn as a whole; a flanking
    /// member of Analogous or Tetradic opens or closes the spread instead,
    /// with the base held. Saturation is shared, so it moves every member.
    pub fn move_member(&mut self, i: usize, h: f32, s: f32) {
        let i = i.min(self.len() - 1);
        self.base.s = s.clamp(0.0, 1.0);
        match (self.scheme, i) {
            (Harmony::Monochromatic, _) => self.base.h = wrap_hue(h),
            (Harmony::Analogous, 0 | 2) => {
                let side = if i == 0 { -1.0 } else { 1.0 };
                let d = signed_hue(h - self.base.h) * side;
                self.analog_spread = d.clamp(*ANALOG_SPREAD.start(), *ANALOG_SPREAD.end());
            }
            (Harmony::Tetradic, 1 | 3) => {
                let from = if i == 3 { 180.0 } else { 0.0 };
                let d = signed_hue(h - self.base.h - from);
                self.tetra_spread = d.clamp(*TETRA_SPREAD.start(), *TETRA_SPREAD.end());
            }
            _ => self.base.h = wrap_hue(h - self.offset(i)),
        }
        self.active = i;
    }

    /// Member `i` set to lightness `l`. The hue schemes share one lightness;
    /// a monochrome step other than the base sets the step size instead.
    pub fn set_member_lightness(&mut self, i: usize, l: f32) {
        let i = i.min(self.len() - 1);
        let base = self.scheme.base_index();
        if self.scheme == Harmony::Monochromatic && i != base {
            let k = i as f32 - base as f32;
            self.mono_step = ((l - self.base.l) / k).clamp(*MONO_STEP.start(), *MONO_STEP.end());
        } else {
            self.base.l = l.clamp(0.0, 1.0);
        }
        self.active = i;
    }

    /// The brush colour changed from outside the panel (eyedropper, a
    /// pinned swatch, Paste, a tool switch): rebuild the set around it.
    pub fn rebase(&mut self, rgb: [u8; 3]) {
        self.base = Hsl::from_rgb_keeping(rgb, self.base);
        self.active = self.scheme.base_index();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn every_8_bit_colour_survives_the_round_trip() {
        let vals: Vec<u8> = (0..=255u8).step_by(5).chain([1, 127, 128, 254]).collect();
        for &r in &vals {
            for &g in &vals {
                for &b in &vals {
                    let rgb = [r, g, b];
                    assert_eq!(Hsl::from_rgb(rgb).to_rgb(), rgb, "{rgb:?}");
                }
            }
        }
    }

    #[test]
    fn primaries_land_where_css_puts_them() {
        let red = Hsl::from_rgb([255, 0, 0]);
        assert!(close(red.h, 0.0) && close(red.s, 1.0) && close(red.l, 0.5));
        assert!(close(Hsl::from_rgb([0, 255, 0]).h, 120.0));
        assert!(close(Hsl::from_rgb([0, 0, 255]).h, 240.0));
        assert_eq!(Hsl::new(210.0, 0.5, 0.4).to_rgb(), [51, 102, 153]);
        assert_eq!(Hsl::from_rgb([51, 102, 153]).fmt(), "hsl(210, 50%, 40%)");
    }

    #[test]
    fn greys_black_and_white_keep_the_previous_hue() {
        let prev = Hsl::new(200.0, 0.8, 0.5);
        let grey = Hsl::from_rgb_keeping([128, 128, 128], prev);
        assert!(close(grey.h, 200.0) && grey.s == 0.0);
        let black = Hsl::from_rgb_keeping([0, 0, 0], prev);
        assert!(close(black.h, 200.0) && close(black.s, 0.8) && black.l == 0.0);
        let white = Hsl::from_rgb_keeping([255, 255, 255], prev);
        assert!(close(white.h, 200.0) && close(white.s, 0.8) && white.l == 1.0);
        assert_eq!(black.to_rgb(), [0, 0, 0]);
    }

    #[test]
    fn a_value_that_already_rounds_to_the_bytes_does_not_drift() {
        let prev = Hsl::new(33.3, 0.414, 0.617);
        assert_eq!(Hsl::from_rgb_keeping(prev.to_rgb(), prev), prev);
    }

    #[test]
    fn pasted_colours_in_every_form() {
        assert_eq!(parse_color("hsl(210, 50%, 40%)"), Some([51, 102, 153]));
        assert_eq!(parse_color("  HSL(210deg 50% 40%) "), Some([51, 102, 153]));
        assert_eq!(parse_color("hsla(210, 50%, 40%, 0.5)"), Some([51, 102, 153]));
        assert_eq!(parse_color("hsl(-150, 50%, 40%)"), Some([51, 102, 153]));
        assert_eq!(parse_color("rgb(157, 89, 76)"), Some([157, 89, 76]));
        assert_eq!(parse_color("RGB(157 89 76)"), Some([157, 89, 76]));
        assert_eq!(parse_color("#9D594C"), Some([157, 89, 76]));
        assert_eq!(parse_color("9d594c"), Some([157, 89, 76]));
        assert_eq!(parse_color("#fff"), Some([255, 255, 255]));
        for bad in ["", "hsl(210, 120%, 40%)", "hsl(210, 50%)", "rgb(300, 0, 0)", "#12345", "red", "#ggg"] {
            assert_eq!(parse_color(bad), None, "{bad}");
        }
    }

    fn hues(set: &HarmonySet) -> Vec<f32> {
        set.members().iter().map(|m| m.h.round()).collect()
    }

    #[test]
    fn each_scheme_lays_its_hues_around_the_base() {
        let base = Hsl::new(30.0, 0.6, 0.5).to_rgb();
        let h = Hsl::from_rgb(base).h.round();
        let mut set = HarmonySet::new(Harmony::Complementary, base);
        assert_eq!(hues(&set), [h, h + 180.0]);
        set.set_scheme(Harmony::Analogous);
        assert_eq!(hues(&set), [h - 30.0, h, h + 30.0]);
        set.set_scheme(Harmony::Triadic);
        assert_eq!(hues(&set), [h, h + 120.0, h + 240.0]);
        set.set_scheme(Harmony::Tetradic);
        assert_eq!(hues(&set), [h, h + 60.0, h + 180.0, wrap_hue(h + 240.0)]);
        set.set_scheme(Harmony::Monochromatic);
        let ls: Vec<f32> = set.members().iter().map(|m| (m.l * 100.0).round()).collect();
        assert_eq!(ls, [20.0, 35.0, 50.0, 65.0, 80.0]);
        assert_eq!(set.active_rgb(), base, "switching scheme puts the brush on the base");
    }

    #[test]
    fn picking_a_member_leaves_the_set_in_place() {
        let mut set = HarmonySet::new(Harmony::Analogous, [200, 80, 40]);
        let before = set.members();
        set.pick(2);
        assert_eq!(set.members(), before);
        assert_eq!(set.active_rgb(), before[2].to_rgb());
    }

    #[test]
    fn dragging_a_fixed_member_turns_the_whole_set() {
        let mut set = HarmonySet::new(Harmony::Triadic, Hsl::new(0.0, 1.0, 0.5).to_rgb());
        set.move_member(1, 150.0, 0.4);
        assert!(close(set.base.h, 30.0), "member 1 sits 120° on, so the base follows to 30°");
        assert!(close(set.base.s, 0.4));
        assert_eq!(set.active, 1);
        assert!(close(set.member(1).h, 150.0));
    }

    #[test]
    fn dragging_a_flank_opens_the_spread_and_holds_the_base() {
        let mut set = HarmonySet::new(Harmony::Analogous, Hsl::new(100.0, 1.0, 0.5).to_rgb());
        let h = set.base.h;
        set.move_member(2, h + 45.0, 1.0);
        assert!(close(set.base.h, h) && close(set.analog_spread, 45.0));
        set.move_member(0, h - 120.0, 1.0);
        assert!(close(set.analog_spread, 90.0), "clamped at the widest");
        set.move_member(0, h + 10.0, 1.0);
        assert!(close(set.analog_spread, 5.0), "past the base clamps at the narrowest");

        let mut tet = HarmonySet::new(Harmony::Tetradic, Hsl::new(0.0, 1.0, 0.5).to_rgb());
        tet.move_member(3, 180.0 + 80.0, 1.0);
        assert!(close(tet.base.h, 0.0) && close(tet.tetra_spread, 80.0));
        tet.move_member(2, 200.0, 1.0);
        assert!(close(tet.base.h, 20.0), "the opposite of the base turns the set");
    }

    #[test]
    fn a_monochrome_tick_sets_the_step() {
        let mut set = HarmonySet::new(Harmony::Monochromatic, Hsl::new(0.0, 1.0, 0.5).to_rgb());
        set.set_member_lightness(4, 0.9);
        assert!(close(set.mono_step, 0.2), "two steps up to 90% from 50%");
        assert!(close(set.base.l, 0.5));
        set.set_member_lightness(2, 0.4);
        assert!(close(set.base.l, 0.4), "the base tick moves the base");
        set.move_member(0, 90.0, 0.3);
        assert!(set.members().iter().all(|m| close(m.h, 90.0) && close(m.s, 0.3)));
    }

    #[test]
    fn a_black_monochrome_base_still_matches_the_brush() {
        let mut set = HarmonySet::new(Harmony::Monochromatic, [0, 0, 0]);
        set.rebase([0, 0, 0]);
        assert_eq!(set.active_rgb(), [0, 0, 0]);
    }

    #[test]
    fn rebase_moves_the_set_onto_the_new_colour() {
        let mut set = HarmonySet::new(Harmony::Analogous, [255, 0, 0]);
        set.pick(0);
        set.rebase([0, 0, 255]);
        assert_eq!(set.active, 1);
        assert_eq!(set.active_rgb(), [0, 0, 255]);
        assert!(close(set.base.h, 240.0));
    }
}
