//! Modern dark theme + Phosphor icon font installation.
//!
//! Cool-indigo accent over a layered neutral-dark surface stack. Four+ surface
//! elevations (base → panel → surface → hover) instead of a single flat gray,
//! a single desaturated accent (`ACCENT`) used only for active / selected /
//! playhead states, and high-contrast text for long sessions.

use egui::{Color32, FontDefinitions, Margin, Rounding, Shadow, Stroke, Visuals};

// --- Surface elevation stack (dark, neutral, slightly cool) ---
/// Deepest level — window background behind floating panels.
pub const BG_BASE: Color32 = Color32::from_rgb(14, 15, 18);
/// Floating panel fill (slightly translucent so the canvas glows through).
pub const BG_PANEL: Color32 = Color32::from_rgba_premultiplied(22, 24, 28, 240);
/// Elevated control surface (inputs, headers).
pub const BG_SURFACE: Color32 = Color32::from_rgb(31, 34, 40);
/// Resting widget fill.
pub const BG_INACTIVE: Color32 = Color32::from_rgb(27, 29, 34);
/// Hovered widget fill.
pub const BG_HOVER: Color32 = Color32::from_rgb(42, 46, 55);

// --- Text ---
pub const TEXT: Color32 = Color32::from_rgb(216, 218, 224);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(138, 142, 151);
/// Text/icon drawn on top of an `ACCENT` fill.
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(244, 245, 255);

// --- Borders ---
pub const STROKE_THIN: Color32 = Color32::from_rgb(44, 47, 55);

// --- Accent (cool indigo) ---
pub const ACCENT: Color32 = Color32::from_rgb(110, 123, 255);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(140, 150, 255);
/// Subtle indigo tint for selection backgrounds.
pub const ACCENT_DIM: Color32 = Color32::from_rgb(35, 38, 62);

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
    ctx.set_style(style);
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.window_fill = BG_PANEL;
    v.window_stroke = Stroke::new(1.0, STROKE_THIN);
    v.window_rounding = Rounding::same(10.0);
    v.window_shadow = Shadow {
        offset: egui::vec2(0.0, 8.0),
        blur: 28.0,
        spread: 0.0,
        color: Color32::from_black_alpha(120),
    };
    v.popup_shadow = Shadow {
        offset: egui::vec2(0.0, 4.0),
        blur: 14.0,
        spread: 0.0,
        color: Color32::from_black_alpha(110),
    };
    v.menu_rounding = Rounding::same(8.0);
    v.panel_fill = BG_BASE;
    v.faint_bg_color = Color32::from_rgb(24, 26, 31);
    v.extreme_bg_color = Color32::from_rgb(12, 13, 16);

    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;

    let rounding = Rounding::same(6.0);
    v.widgets.noninteractive.rounding = rounding;
    v.widgets.inactive.rounding = rounding;
    v.widgets.hovered.rounding = rounding;
    v.widgets.active.rounding = rounding;
    v.widgets.open.rounding = rounding;

    v.widgets.inactive.bg_fill = BG_INACTIVE;
    // Inputs (sliders, drag-values) sit on a slightly elevated surface.
    v.widgets.inactive.weak_bg_fill = BG_SURFACE;
    v.widgets.hovered.bg_fill = BG_HOVER;
    v.widgets.hovered.weak_bg_fill = BG_HOVER;
    // Pressed / active widgets flash the accent.
    v.widgets.active.bg_fill = ACCENT;
    v.widgets.active.weak_bg_fill = ACCENT;
    v.widgets.open.bg_fill = BG_HOVER;
    v.widgets.open.weak_bg_fill = BG_HOVER;

    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.fg_stroke = Stroke::new(1.0, ACCENT_TEXT);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_MUTED);

    v.widgets.noninteractive.bg_stroke = Stroke::new(0.5, STROKE_THIN);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, STROKE_THIN);
    // Subtle indigo outline on hover — a quiet, modern affordance cue.
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_HOVER);
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.open.bg_stroke = Stroke::new(1.0, STROKE_THIN);

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
            Stroke::new(2.0, ACCENT),
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
        .color(TEXT_MUTED)
        .strong()
        .size(11.0);
    ui.add(egui::Label::new(label));
    ui.add_space(2.0);
    ui.separator();
}
