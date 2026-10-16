//! The app's colours + Phosphor icon font installation.
//!
//! A layered surface stack (base → panel → surface → hover) instead of one
//! flat fill, a single accent used only for active / selected / playhead
//! states, and high-contrast text for long sessions. The colours live in a
//! [`Palette`] that can be swapped at runtime ([`apply`]); the accessors below
//! read whichever is current.

use std::cell::Cell;

use egui::{Color32, FontDefinitions, Margin, Rounding, Shadow, Stroke, Visuals};

/// Every colour the app's own chrome is drawn in. Canvas overlays — guides,
/// handles, the lasso — are drawn over the artwork rather than the chrome and
/// keep their own colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Deepest level — window background behind floating panels.
    pub bg_base: Color32,
    /// Floating panel fill (slightly translucent so the canvas glows through).
    pub bg_panel: Color32,
    /// Elevated control surface (inputs, headers).
    pub bg_surface: Color32,
    /// Resting widget fill.
    pub bg_inactive: Color32,
    /// Hovered widget fill.
    pub bg_hover: Color32,
    /// Alternate rows in striped lists.
    pub faint_bg: Color32,
    /// Text edits and other sunken fields.
    pub extreme_bg: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    /// Icons and text on a hovered widget.
    pub text_hover: Color32,
    /// Text/icon drawn on top of an `accent` fill.
    pub accent_text: Color32,
    pub stroke_thin: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    /// Subtle accent tint for selection backgrounds.
    pub accent_dim: Color32,
    /// The timeline's inset well, a step below the panel.
    pub well: Color32,
    /// Hairline between timeline rows — quieter than the well's own edges.
    pub row_line: Color32,
    /// A drawing's bar on the timeline.
    pub bar: Color32,
    pub bar_hover: Color32,
    /// Something out of range or clashing.
    pub warn: Color32,
    /// Timeline markers for layer transform keys and camera keys.
    pub key_layer: Color32,
    pub key_camera: Color32,
    /// What a faint highlight lightens or darkens toward: white over dark
    /// surfaces, black over light ones. See [`tint`].
    pub sheen: Color32,
    /// Surfaces are light, so egui starts from its light visuals.
    pub light: bool,
}

impl Palette {
    /// The cool-indigo dark the app shipped with.
    pub const GRAPHITE: Palette = Palette {
        bg_base: Color32::from_rgb(14, 15, 18),
        bg_panel: Color32::from_rgba_premultiplied(22, 24, 28, 240),
        bg_surface: Color32::from_rgb(31, 34, 40),
        bg_inactive: Color32::from_rgb(27, 29, 34),
        bg_hover: Color32::from_rgb(42, 46, 55),
        faint_bg: Color32::from_rgb(24, 26, 31),
        extreme_bg: Color32::from_rgb(12, 13, 16),
        text: Color32::from_rgb(216, 218, 224),
        text_muted: Color32::from_rgb(138, 142, 151),
        text_hover: Color32::WHITE,
        accent_text: Color32::from_rgb(244, 245, 255),
        stroke_thin: Color32::from_rgb(44, 47, 55),
        accent: Color32::from_rgb(110, 123, 255),
        accent_hover: Color32::from_rgb(140, 150, 255),
        accent_dim: Color32::from_rgb(35, 38, 62),
        well: Color32::from_rgb(19, 21, 25),
        row_line: Color32::from_rgb(30, 33, 39),
        bar: Color32::from_rgb(50, 55, 67),
        bar_hover: Color32::from_rgb(63, 69, 85),
        warn: Color32::from_rgb(232, 138, 96),
        // Same blue as the layer bounds outline on the canvas, and the same
        // amber as the camera-edit guide.
        key_layer: Color32::from_rgb(120, 160, 220),
        key_camera: Color32::from_rgb(255, 190, 90),
        sheen: Color32::WHITE,
        light: false,
    };

    /// Blue-black, with a cyan accent.
    pub const MIDNIGHT: Palette = Palette {
        bg_base: Color32::from_rgb(8, 12, 20),
        bg_panel: Color32::from_rgba_premultiplied(13, 18, 28, 240),
        bg_surface: Color32::from_rgb(22, 30, 44),
        bg_inactive: Color32::from_rgb(18, 25, 37),
        bg_hover: Color32::from_rgb(32, 43, 62),
        faint_bg: Color32::from_rgb(16, 22, 33),
        extreme_bg: Color32::from_rgb(6, 9, 15),
        text: Color32::from_rgb(214, 224, 238),
        text_muted: Color32::from_rgb(128, 146, 172),
        text_hover: Color32::WHITE,
        // Dark on the cyan: white on it barely reads.
        accent_text: Color32::from_rgb(6, 20, 28),
        stroke_thin: Color32::from_rgb(34, 46, 66),
        accent: Color32::from_rgb(70, 200, 235),
        accent_hover: Color32::from_rgb(120, 220, 245),
        accent_dim: Color32::from_rgb(18, 50, 66),
        well: Color32::from_rgb(11, 16, 26),
        row_line: Color32::from_rgb(24, 32, 46),
        bar: Color32::from_rgb(40, 54, 78),
        bar_hover: Color32::from_rgb(52, 70, 100),
        warn: Color32::from_rgb(240, 140, 100),
        key_layer: Color32::from_rgb(130, 170, 240),
        key_camera: Color32::from_rgb(255, 196, 100),
        sheen: Color32::WHITE,
        light: false,
    };

    /// Light grey surfaces and dark text.
    pub const PAPER: Palette = Palette {
        bg_base: Color32::from_rgb(200, 202, 207),
        // premul(236, 237, 240, 245): still a touch see-through.
        bg_panel: Color32::from_rgba_premultiplied(227, 228, 231, 245),
        bg_surface: Color32::from_rgb(252, 252, 253),
        bg_inactive: Color32::from_rgb(226, 228, 233),
        bg_hover: Color32::from_rgb(212, 216, 225),
        faint_bg: Color32::from_rgb(226, 228, 232),
        extreme_bg: Color32::from_rgb(255, 255, 255),
        text: Color32::from_rgb(28, 30, 36),
        text_muted: Color32::from_rgb(88, 93, 104),
        text_hover: Color32::BLACK,
        accent_text: Color32::WHITE,
        stroke_thin: Color32::from_rgb(196, 199, 207),
        accent: Color32::from_rgb(70, 84, 220),
        accent_hover: Color32::from_rgb(96, 110, 236),
        accent_dim: Color32::from_rgb(206, 211, 247),
        well: Color32::from_rgb(216, 218, 224),
        row_line: Color32::from_rgb(202, 205, 213),
        bar: Color32::from_rgb(176, 182, 197),
        bar_hover: Color32::from_rgb(158, 166, 185),
        warn: Color32::from_rgb(190, 82, 34),
        key_layer: Color32::from_rgb(46, 96, 186),
        key_camera: Color32::from_rgb(196, 116, 6),
        sheen: Color32::BLACK,
        light: true,
    };

    /// A whole palette worked out from five colours. Everything else is a
    /// mix of those, the way the presets' in-between shades relate to them,
    /// so a custom scheme hangs together however the five are picked.
    pub fn derive(b: BaseColors) -> Palette {
        let rgb = |c: [u8; 3]| Color32::from_rgb(c[0], c[1], c[2]);
        let (bg, panel, widget, text, accent) =
            (rgb(b.bg), rgb(b.panel), rgb(b.widget), rgb(b.text), rgb(b.accent));
        let light = luminance(panel) > 0.4;
        let (sheen, far) = if light {
            (Color32::BLACK, Color32::WHITE)
        } else {
            (Color32::WHITE, Color32::BLACK)
        };
        let well = mix(bg, panel, 0.6);
        Palette {
            bg_base: bg,
            bg_panel: premul(panel.r(), panel.g(), panel.b(), 240),
            bg_surface: widget,
            bg_inactive: mix(widget, panel, 0.4),
            bg_hover: mix(widget, text, 0.1),
            faint_bg: mix(panel, text, 0.04),
            extreme_bg: mix(widget, far, 0.4),
            text,
            text_muted: mix(text, panel, 0.42),
            text_hover: mix(text, sheen, 0.5),
            accent_text: if luminance(accent) > 0.4 {
                Color32::from_rgb(12, 14, 18)
            } else {
                Color32::from_rgb(244, 245, 255)
            },
            stroke_thin: mix(panel, text, 0.14),
            accent,
            accent_hover: mix(accent, sheen, 0.2),
            accent_dim: mix(panel, accent, 0.22),
            well,
            row_line: mix(well, text, 0.07),
            bar: mix(widget, text, 0.14),
            bar_hover: mix(widget, text, 0.24),
            warn: if light { Palette::PAPER.warn } else { Palette::GRAPHITE.warn },
            key_layer: if light { Palette::PAPER.key_layer } else { Palette::GRAPHITE.key_layer },
            key_camera: if light { Palette::PAPER.key_camera } else { Palette::GRAPHITE.key_camera },
            sheen,
            light,
        }
    }

    /// The five colours a custom scheme is made from, as this palette has
    /// them — where "Start from" begins.
    pub fn base(&self) -> BaseColors {
        let rgb = |c: Color32| [c.r(), c.g(), c.b()];
        // The panel fill is stored translucent; its colour is what shows.
        let a = self.bg_panel.a().max(1) as u32;
        let un = |c: u8| ((c as u32 * 255 + a / 2) / a).min(255) as u8;
        let p = self.bg_panel;
        BaseColors {
            bg: rgb(self.bg_base),
            panel: [un(p.r()), un(p.g()), un(p.b())],
            widget: rgb(self.bg_surface),
            text: rgb(self.text),
            accent: rgb(self.accent),
        }
    }
}

/// Which colours the UI is drawn in. Stored with the app's preferences,
/// never in a project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scheme {
    #[default]
    Graphite,
    Midnight,
    Paper,
    /// Worked out from the [`BaseColors`] kept beside the scheme.
    Custom,
}

impl Scheme {
    pub const ALL: [Scheme; 4] = [Scheme::Graphite, Scheme::Midnight, Scheme::Paper, Scheme::Custom];
    pub const PRESETS: [Scheme; 3] = [Scheme::Graphite, Scheme::Midnight, Scheme::Paper];

    pub fn label(self) -> &'static str {
        match self {
            Scheme::Graphite => "Graphite",
            Scheme::Midnight => "Midnight",
            Scheme::Paper => "Paper",
            Scheme::Custom => "Custom",
        }
    }

    /// The palette this scheme draws in; `custom` is only read by Custom.
    pub fn palette(self, custom: BaseColors) -> Palette {
        match self {
            Scheme::Graphite => Palette::GRAPHITE,
            Scheme::Midnight => Palette::MIDNIGHT,
            Scheme::Paper => Palette::PAPER,
            Scheme::Custom => Palette::derive(custom),
        }
    }
}

/// The five colours a custom scheme is picked as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BaseColors {
    /// Behind the floating panels.
    pub bg: [u8; 3],
    pub panel: [u8; 3],
    /// Buttons, fields and sliders.
    pub widget: [u8; 3],
    pub text: [u8; 3],
    pub accent: [u8; 3],
}

impl Default for BaseColors {
    fn default() -> Self {
        Palette::GRAPHITE.base()
    }
}

/// `a` moved fraction `t` of the way to `b`, in sRGB.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// WCAG relative luminance, 0 (black) to 1 (white).
pub fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// WCAG contrast ratio between two colours, 1 to 21.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

thread_local! {
    // Per thread: egui draws on one, and each test gets its own.
    static PALETTE: Cell<Palette> = const { Cell::new(Palette::GRAPHITE) };
}

/// The palette the UI is drawn in now.
pub fn palette() -> Palette {
    PALETTE.with(Cell::get)
}

macro_rules! tokens {
    ($($name:ident),* $(,)?) => {
        $(
            #[doc = concat!("The current palette's `", stringify!($name), "`.")]
            pub fn $name() -> Color32 {
                palette().$name
            }
        )*
    };
}

tokens!(
    bg_base, bg_panel, bg_surface, bg_hover, text, text_muted, accent_text, stroke_thin, accent,
    accent_hover, accent_dim, well, row_line, bar, bar_hover, warn, key_layer, key_camera,
);

/// A faint highlight at alpha `a`, toward white on a dark palette and black
/// on a light one — what a picked row or a hovered bar is lifted by.
pub fn tint(a: u8) -> Color32 {
    let s = palette().sheen;
    premul(s.r(), s.g(), s.b(), a)
}

/// The shadow under floating windows: soft on light surfaces, where a dark
/// one reads as a smudge.
pub fn window_shadow() -> Shadow {
    Shadow {
        offset: egui::vec2(0.0, 8.0),
        blur: 28.0,
        spread: 0.0,
        color: Color32::from_black_alpha(if palette().light { 48 } else { 120 }),
    }
}

// --- Translucent colours ---
//
// egui 0.29's `Color32::from_rgba_unmultiplied` and `from_white_alpha`
// premultiply in *linear* space and convert back to gamma, but egui-wgpu
// blends in *gamma* space. The result has RGB above alpha — white at half
// alpha comes out as (188, 188, 188, 128) — so anything translucent adds more
// light than its opacity allows and clips toward white, like blown
// highlights. The CPU compositor that exports frames premultiplies in gamma
// space, so these match it; `clippy.toml` bans the egui helpers.

/// Straight-alpha sRGB colour, premultiplied in gamma space.
pub fn premul(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    let m = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
    Color32::from_rgba_premultiplied(m(r), m(g), m(b), a)
}

/// White at alpha `a` — also the tint that draws a texture at opacity `a`.
pub const fn white_alpha(a: u8) -> Color32 {
    Color32::from_rgba_premultiplied(a, a, a, a)
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals();
    // Smooth, subtle state transitions (toggles, collapsibles, hover).
    style.animation_time = 0.11;
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    style.spacing.item_spacing = egui::vec2(7.0, 6.0);
    style.spacing.window_margin = Margin::same(9.0);
    style.spacing.menu_margin = Margin::same(6.0);
    style.spacing.slider_width = 124.0;
    style.spacing.interact_size = egui::vec2(26.0, 22.0);
    style.spacing.icon_width = 15.0;
    style.spacing.icon_spacing = 6.0;
    style.spacing.scroll = egui::style::ScrollStyle::thin();
    // Slightly smaller body text for a denser, more pro feel.
    use egui::{FontFamily, FontId, TextStyle};
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(12.5, FontFamily::Proportional));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::new(12.5, FontFamily::Proportional));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::new(10.5, FontFamily::Proportional));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::new(14.0, FontFamily::Proportional));
    // Both of egui's slots: which one is live follows the OS theme, and a
    // switch there mid-session would otherwise fall back to egui's defaults.
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
}

/// Draw the UI in `p` from the next frame on.
pub fn apply(ctx: &egui::Context, p: Palette) {
    PALETTE.with(|c| c.set(p));
    for t in [egui::Theme::Dark, egui::Theme::Light] {
        let mut style = (*ctx.style_of(t)).clone();
        style.visuals = visuals();
        ctx.set_style_of(t, style);
    }
}

fn visuals() -> Visuals {
    let p = palette();
    let mut v = if p.light { Visuals::light() } else { Visuals::dark() };
    v.override_text_color = Some(p.text);
    v.window_fill = p.bg_panel;
    v.window_stroke = Stroke::new(1.0, p.stroke_thin);
    v.window_rounding = Rounding::same(10.0);
    v.window_shadow = window_shadow();
    v.popup_shadow = Shadow {
        offset: egui::vec2(0.0, 4.0),
        blur: 14.0,
        spread: 0.0,
        color: Color32::from_black_alpha(if p.light { 44 } else { 110 }),
    };
    v.menu_rounding = Rounding::same(8.0);
    v.panel_fill = p.bg_base;
    v.faint_bg_color = p.faint_bg;
    v.extreme_bg_color = p.extreme_bg;

    v.selection.bg_fill = p.accent_dim;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.hyperlink_color = p.accent;

    let rounding = Rounding::same(6.0);
    v.widgets.noninteractive.rounding = rounding;
    v.widgets.inactive.rounding = rounding;
    v.widgets.hovered.rounding = rounding;
    v.widgets.active.rounding = rounding;
    v.widgets.open.rounding = rounding;

    v.widgets.inactive.bg_fill = p.bg_inactive;
    // Inputs (sliders, drag-values) sit on a slightly elevated surface.
    v.widgets.inactive.weak_bg_fill = p.bg_surface;
    v.widgets.hovered.bg_fill = p.bg_hover;
    v.widgets.hovered.weak_bg_fill = p.bg_hover;
    // Pressed / active widgets flash the accent.
    v.widgets.active.bg_fill = p.accent;
    v.widgets.active.weak_bg_fill = p.accent;
    v.widgets.open.bg_fill = p.bg_hover;
    v.widgets.open.weak_bg_fill = p.bg_hover;

    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text_hover);
    v.widgets.active.fg_stroke = Stroke::new(1.0, p.accent_text);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text_muted);

    v.widgets.noninteractive.bg_stroke = Stroke::new(0.5, p.stroke_thin);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.stroke_thin);
    // Subtle accent outline on hover — a quiet, modern affordance cue.
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent_hover);
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.open.bg_stroke = Stroke::new(1.0, p.stroke_thin);

    v
}

/// Tooltip-decorated icon-only button. 26×22 minimum.
pub fn icon_button(ui: &mut egui::Ui, icon: &str, tooltip: &str) -> egui::Response {
    let resp = ui.add(
        egui::Button::new(egui::RichText::new(icon).size(15.0))
            .min_size(egui::vec2(28.0, 22.0)),
    );
    if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    }
}

/// Selectable icon button used in the tool palette. When selected it shows the
/// accent fill via the active widget visuals; an accent underline reinforces it.
pub fn icon_toggle(ui: &mut egui::Ui, icon: &str, tooltip: &str, selected: bool) -> egui::Response {
    let text = egui::RichText::new(icon).size(16.0);
    let resp = ui.add_sized([32.0, 28.0], egui::SelectableLabel::new(selected, text));
    if selected {
        // Thin accent bar under the active tool for an unambiguous, modern cue.
        let r = resp.rect;
        let y = r.bottom() - 2.0;
        ui.painter().line_segment(
            [egui::pos2(r.left() + 5.0, y), egui::pos2(r.right() - 5.0, y)],
            Stroke::new(2.0, accent()),
        );
    }
    if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    }
}

pub fn icon_text(icon: &str, label: &str) -> String {
    format!("{icon}  {label}")
}

pub fn section_header(ui: &mut egui::Ui, icon: &str, title: &str) {
    let label = egui::RichText::new(format!("{icon}  {title}"))
        .color(text_muted())
        .strong()
        .size(11.0);
    ui.add(egui::Label::new(label));
    ui.add_space(2.0);
    ui.separator();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opaque colour a translucent panel fill shows as over `under`.
    fn over(c: Color32, under: Color32) -> Color32 {
        let k = 1.0 - c.a() as f32 / 255.0;
        let m = |x: u8, y: u8| (x as f32 + y as f32 * k).round().min(255.0) as u8;
        Color32::from_rgb(m(c.r(), under.r()), m(c.g(), under.g()), m(c.b(), under.b()))
    }

    /// Body text, muted text and the accent read on the surfaces they sit on:
    /// WCAG AA for text (4.5), and 3 for muted labels and the accent.
    fn readable(name: &str, p: &Palette) {
        let panel = over(p.bg_panel, p.bg_base);
        for (what, fg, bg, min) in [
            ("text on panel", p.text, panel, 4.5),
            ("text on widget", p.text, p.bg_surface, 4.5),
            ("text on hover", p.text, p.bg_hover, 4.5),
            ("muted on panel", p.text_muted, panel, 3.0),
            ("accent text on accent", p.accent_text, p.accent, 3.0),
            ("accent on panel", p.accent, panel, 3.0),
            ("warn on panel", p.warn, panel, 3.0),
            ("camera key on well", p.key_camera, p.well, 1.8),
            ("layer key on well", p.key_layer, p.well, 1.8),
        ] {
            let c = contrast(fg, bg);
            assert!(c >= min, "{name}: {what} is {c:.2}, wants {min}");
        }
    }

    #[test]
    fn every_preset_reads() {
        for s in Scheme::PRESETS {
            readable(s.label(), &s.palette(BaseColors::default()));
        }
    }

    #[test]
    fn a_custom_scheme_made_from_a_preset_reads_too() {
        for s in Scheme::PRESETS {
            let p = Palette::derive(s.palette(BaseColors::default()).base());
            readable(&format!("custom from {}", s.label()), &p);
            assert_eq!(p.light, s == Scheme::Paper, "{}", s.label());
        }
    }

    #[test]
    fn a_custom_scheme_keeps_its_five_colours() {
        let b = BaseColors {
            bg: [30, 10, 10],
            panel: [50, 20, 20],
            widget: [70, 30, 30],
            text: [240, 230, 220],
            accent: [250, 120, 60],
        };
        let p = Palette::derive(b);
        assert_eq!(p.base(), b, "round trip through the palette");
        assert_eq!(Scheme::Custom.palette(b), p);
        assert_eq!(BaseColors::default(), Palette::GRAPHITE.base());
    }

    /// The scheme lands in both of egui's style slots, and the accessors
    /// follow it.
    #[test]
    fn applying_a_scheme_restyles_both_slots() {
        let ctx = egui::Context::default();
        install(&ctx);
        apply(&ctx, Palette::PAPER);
        for t in [egui::Theme::Dark, egui::Theme::Light] {
            let v = &ctx.style_of(t).visuals;
            assert_eq!(v.window_fill, Palette::PAPER.bg_panel);
            assert!(!v.dark_mode, "light visuals under Paper");
        }
        assert_eq!(text(), Palette::PAPER.text);
        assert_eq!(tint(10), premul(0, 0, 0, 10), "highlights darken on light");
        apply(&ctx, Palette::GRAPHITE);
        assert_eq!(ctx.style_of(egui::Theme::Light).visuals.window_fill, Palette::GRAPHITE.bg_panel);
    }

    #[test]
    fn premultiplying_stays_in_gamma_space() {
        assert_eq!(premul(255, 255, 255, 128), Color32::from_rgba_premultiplied(128, 128, 128, 128));
        assert_eq!(white_alpha(128), premul(255, 255, 255, 128));
        assert_eq!(premul(10, 20, 30, 255), Color32::from_rgb(10, 20, 30));
        assert_eq!(premul(10, 20, 30, 0), Color32::TRANSPARENT);
        // RGB never exceeds alpha: nothing adds more light than it covers.
        for a in 0..=255u8 {
            let c = premul(255, 200, 80, a);
            assert!(c.r() <= a && c.g() <= a && c.b() <= a, "alpha {a}: {c:?}");
        }
    }
}
