//! Tight monochrome dark theme + Phosphor icon font installation.

use egui::{Color32, FontDefinitions, Margin, Rounding, Shadow, Stroke, Visuals};

// Monochrome palette — no accent colour. "Selected" / hover are just lighter
// grays. Cleaner, less attention-grabbing, more pro-tool feel.
pub const SELECT: Color32 = Color32::from_rgb(170, 172, 178);
pub const SELECT_DIM: Color32 = Color32::from_rgb(78, 80, 86);
pub const BG_BASE: Color32 = Color32::from_rgb(16, 17, 19);
pub const BG_PANEL: Color32 = Color32::from_rgba_premultiplied(20, 21, 24, 235);
pub const BG_HOVER: Color32 = Color32::from_rgb(42, 44, 50);
pub const BG_INACTIVE: Color32 = Color32::from_rgb(28, 30, 34);
pub const TEXT: Color32 = Color32::from_rgb(215, 217, 222);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(135, 138, 145);
pub const STROKE_THIN: Color32 = Color32::from_rgb(46, 48, 54);

// Backwards-compat aliases for callers that still reference ACCENT/ACCENT_DIM.
pub const ACCENT: Color32 = SELECT;
pub const ACCENT_DIM: Color32 = SELECT_DIM;

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals();
    style.spacing.button_padding = egui::vec2(7.0, 3.0);
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.window_margin = Margin::same(7.0);
    style.spacing.menu_margin = Margin::same(5.0);
    style.spacing.slider_width = 120.0;
    style.spacing.interact_size = egui::vec2(24.0, 20.0);
    style.spacing.icon_width = 14.0;
    style.spacing.icon_spacing = 5.0;
    // Slightly smaller body text for a denser, more pro feel.
    use egui::{FontFamily, FontId, TextStyle};
    style.text_styles.insert(TextStyle::Body, FontId::new(12.5, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Button, FontId::new(12.5, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Small, FontId::new(10.5, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Heading, FontId::new(14.0, FontFamily::Proportional));
    ctx.set_style(style);
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.window_fill = BG_PANEL;
    v.window_stroke = Stroke::new(1.0, STROKE_THIN);
    v.window_rounding = Rounding::same(7.0);
    v.window_shadow = Shadow {
        offset: egui::vec2(0.0, 3.0),
        blur: 12.0,
        spread: 0.0,
        color: Color32::from_black_alpha(140),
    };
    v.popup_shadow = Shadow {
        offset: egui::vec2(0.0, 2.0),
        blur: 8.0,
        spread: 0.0,
        color: Color32::from_black_alpha(120),
    };
    v.menu_rounding = Rounding::same(6.0);
    v.panel_fill = BG_BASE;
    v.faint_bg_color = Color32::from_rgb(24, 26, 30);
    v.extreme_bg_color = Color32::from_rgb(12, 13, 16);

    v.selection.bg_fill = SELECT_DIM;
    v.selection.stroke = Stroke::new(1.0, SELECT);
    v.hyperlink_color = SELECT;

    v.widgets.noninteractive.rounding = Rounding::same(4.0);
    v.widgets.inactive.rounding = Rounding::same(4.0);
    v.widgets.hovered.rounding = Rounding::same(4.0);
    v.widgets.active.rounding = Rounding::same(4.0);
    v.widgets.open.rounding = Rounding::same(4.0);

    v.widgets.inactive.bg_fill = BG_INACTIVE;
    v.widgets.inactive.weak_bg_fill = BG_INACTIVE;
    v.widgets.hovered.bg_fill = BG_HOVER;
    v.widgets.hovered.weak_bg_fill = BG_HOVER;
    v.widgets.active.bg_fill = SELECT;
    v.widgets.active.weak_bg_fill = SELECT;
    v.widgets.open.bg_fill = BG_HOVER;
    v.widgets.open.weak_bg_fill = BG_HOVER;

    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::BLACK);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_MUTED);
    v.widgets.noninteractive.bg_stroke = Stroke::new(0.5, STROKE_THIN);
    v.widgets.inactive.bg_stroke = Stroke::new(0.5, STROKE_THIN);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, STROKE_THIN);
    v.widgets.active.bg_stroke = Stroke::new(1.0, SELECT);
    v
}

/// Tooltip-decorated icon-only button. 26×20 minimum.
pub fn icon_button(ui: &mut egui::Ui, icon: &str, tooltip: &str) -> egui::Response {
    let resp = ui.add(
        egui::Button::new(egui::RichText::new(icon).size(14.0))
            .min_size(egui::vec2(26.0, 20.0)),
    );
    if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    }
}

/// Selectable icon button used in the tool palette.
pub fn icon_toggle(ui: &mut egui::Ui, icon: &str, tooltip: &str, selected: bool) -> egui::Response {
    let text = egui::RichText::new(icon).size(15.0);
    let resp = ui.add_sized([30.0, 26.0], egui::SelectableLabel::new(selected, text));
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
    ui.add_space(1.0);
    ui.separator();
}
