//! The brush picker in the Brush settings window: a searchable grid of
//! thumbnails in the library's three groups, and a summary of the Krita
//! preset in use.

use egui::{RichText, Vec2};

use crate::app::AppState;
use crate::brush_library::{Group, Source, Status};
use crate::tools::krita::sensors::SensorKind;
use crate::tools::krita::tip::auto::{Profile, Shape};
use crate::tools::krita::{PixelBrush, Tip};
use crate::tools::{BrushMode, BrushSettings};
use crate::ui::theme;

/// Thumbnail size on screen.
const TILE: f32 = 48.0;
/// A tile's footprint: the thumbnail with a margin, and two lines of name.
const TILE_W: f32 = TILE + 16.0;
const TILE_H: f32 = TILE + 32.0;
/// What thumbnails sit on: a paper colour, so dark strokes read on the
/// dark theme.
pub(crate) const PAPER: egui::Color32 = egui::Color32::from_rgb(226, 226, 222);

/// One brush in the grid: a fixed-size tile, so the grid lines up however
/// long the names are. A brush that cannot paint is drawn faded.
fn tile(ui: &mut egui::Ui, thumb: Option<&egui::TextureHandle>, name: &str, selected: bool, enabled: bool) -> egui::Response {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(TILE_W, TILE_H), sense);
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let p = ui.painter_at(rect);
    let bg = if selected {
        theme::ACCENT_DIM
    } else if enabled && resp.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    p.rect_filled(rect, 4.0, bg);
    if selected {
        p.rect_stroke(rect, 4.0, egui::Stroke::new(1.0, theme::ACCENT));
    }
    let fade = if enabled { egui::Color32::WHITE } else { theme::white_alpha(70) };
    let well = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 4.0 + TILE / 2.0), Vec2::splat(TILE));
    p.rect_filled(well, 3.0, PAPER.gamma_multiply(if enabled { 1.0 } else { 0.35 }));
    if let Some(t) = thumb {
        let [w, h] = t.size();
        let s = TILE / w.max(h).max(1) as f32;
        let img = egui::Rect::from_center_size(well.center(), Vec2::new(w as f32 * s, h as f32 * s));
        p.image(t.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), fade);
    }
    let color = if enabled { ui.visuals().text_color() } else { theme::TEXT_MUTED };
    let galley = ui.painter().layout(name.to_owned(), egui::FontId::proportional(10.0), color, TILE_W - 4.0);
    let at = egui::pos2(rect.center().x - galley.size().x / 2.0, well.bottom() + 3.0);
    p.galley(at, galley, color);
    resp
}

/// Upload the thumbnails the library has decoded and the picker has not.
fn sync_thumbs(state: &mut AppState, ctx: &egui::Context) {
    let missing: Vec<String> = state
        .brush_library
        .entries
        .iter()
        .filter(|e| !state.brush_thumbs.contains_key(&e.id))
        .map(|e| e.id.clone())
        .collect();
    for id in missing {
        if let Some(img) = state.brush_library.thumbnail(&id) {
            let tex = ctx.load_texture(format!("brush-thumb-{id}"), img.clone(), egui::TextureOptions::LINEAR);
            state.brush_thumbs.insert(id, tex);
        }
    }
}

/// Pick `id`: an app preset replaces the brush, a Krita preset sets the
/// Krita engine on it at the preset's own size. The colour stays.
pub fn pick(state: &mut AppState, id: &str) {
    let Some(entry) = state.brush_library.entry(id).cloned() else { return };
    let color = state.brush.color;
    match entry.source {
        Source::App(i) => state.brush = (BrushSettings::PRESETS[i].2)(),
        _ => {
            let Some(b) = state.brush_library.brush(id) else { return };
            state.brush = BrushSettings {
                radius: (b.diameter() / 2.0) as f32,
                mode: BrushMode::Krita,
                krita_id: Some(id.to_owned()),
                ..BrushSettings::default()
            };
        }
    }
    state.brush.color = color;
}

pub fn brush_grid(state: &mut AppState, ui: &mut egui::Ui) {
    sync_thumbs(state, ui.ctx());
    ui.horizontal(|ui| {
        ui.label(theme::icon_text(egui_phosphor::regular::MAGNIFYING_GLASS, ""));
        ui.add(egui::TextEdit::singleline(&mut state.brush_search).hint_text("Search brushes").desired_width(f32::INFINITY));
    });
    let needle = state.brush_search.trim().to_lowercase();
    let mut picked = None;
    // Its own scroll area, so the controls below stay in view.
    egui::ScrollArea::vertical()
        .id_salt("brush-grid")
        .max_height(260.0)
        .auto_shrink([false, true])
        .show(ui, |ui| groups(state, ui, &needle, &mut picked));
    if let Some(id) = picked {
        pick(state, &id);
    }
}

fn groups(state: &AppState, ui: &mut egui::Ui, needle: &str, picked: &mut Option<String>) {
    for group in [Group::App, Group::Krita, Group::Imported] {
        let entries: Vec<_> = state
            .brush_library
            .entries
            .iter()
            .filter(|e| e.group == group && (needle.is_empty() || e.name.to_lowercase().contains(needle)))
            .cloned()
            .collect();
        if entries.is_empty() {
            continue;
        }
        egui::CollapsingHeader::new(RichText::new(group.label()).strong())
            .id_salt(("brush-group", group.label()))
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                    for e in &entries {
                        let selected = state.brush.mode == BrushMode::Krita && state.brush.krita_id.as_deref() == Some(&e.id);
                        let ready = e.status == Status::Ready;
                        let r = tile(ui, state.brush_thumbs.get(&e.id), &e.name, selected, ready);
                        let r = match &e.status {
                            Status::Refused(why) => r.on_hover_text(format!("{}

Not available: {why}", e.tooltip)),
                            Status::Loading => r.on_hover_text("Loading…"),
                            Status::Ready => r.on_hover_text(&e.tooltip),
                        };
                        if ready && r.clicked() {
                            *picked = Some(e.id.clone());
                        }
                    }
                });
            });
    }
}

fn sensor_name(k: SensorKind) -> &'static str {
    match k {
        SensorKind::Pressure => "pressure",
        SensorKind::TangentialPressure => "tangential pressure",
        SensorKind::DrawingAngle { .. } => "drawing direction",
        SensorKind::XTilt => "tilt across",
        SensorKind::YTilt => "tilt along",
        SensorKind::TiltDirection => "the way the pen leans",
        SensorKind::TiltElevation => "how far the pen leans",
        SensorKind::Rotation => "barrel rotation",
        SensorKind::FuzzyPerDab => "randomness",
        SensorKind::Speed => "speed",
        SensorKind::Fade { .. } => "fade along the stroke",
    }
}

/// What a library preset does, in a few lines.
pub fn describe(b: &PixelBrush) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(match &b.tip {
        Tip::Png(_) => "Tip: a picture".to_owned(),
        Tip::Pipe { cells, .. } => format!("Tip: {} pictures, one per dab", cells.len()),
        Tip::Auto(d) => {
            let shape = if d.shape == Shape::Circle { "round" } else { "square" };
            let edge = match d.profile {
                Profile::Default => "hard-edged",
                Profile::Gauss => "gaussian",
                Profile::Soft(_) => "soft",
            };
            format!("Tip: {edge} {shape}")
        }
    });
    for (name, o) in [("Size", &b.size), ("Opacity", &b.opacity), ("Rotation", &b.rotation), ("Flow", &b.flow)] {
        if !o.checked || o.sensors.is_empty() {
            continue;
        }
        let what: Vec<&str> = o.sensors.iter().map(|s| sensor_name(s.kind)).collect();
        lines.push(format!("{name} ← {}", what.join(" + ")));
    }
    if b.texture.is_some() {
        lines.push("Paper texture".to_owned());
    }
    lines.push(if b.wash {
        "Wash: the stroke builds up once, not dab by dab".to_owned()
    } else {
        "Build-up: dabs pile up dab by dab".to_owned()
    });
    if b.erase {
        lines.push("Erases".to_owned());
    }
    lines
}

/// The summary for a brush picked from the library, or `None` for the
/// built-in Pencil-5.
pub fn library_summary(state: &AppState, ui: &mut egui::Ui) -> bool {
    let Some(id) = state.brush.krita_id.as_deref() else { return false };
    let Some(entry) = state.brush_library.entry(id) else { return false };
    let Some(b) = state.brush_library.brush(id) else { return false };
    ui.label(RichText::new(format!("Krita · {}", entry.name)).strong());
    for l in describe(&b) {
        ui.label(RichText::new(l).small());
    }
    true
}
