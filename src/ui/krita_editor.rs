//! The Krita brush editor in Brush settings: a preset's painting mode, tip
//! and curve options, laid out after Krita's own editor and trimmed to what
//! this engine paints. Changes are kept as overrides on the preset (see
//! `tools::krita::edit`): Reset drops them, Save as new writes them into a
//! preset of their own.

use egui::RichText;

use crate::app::AppState;
use crate::tools::krita::kpp::{OptionData, Preset};
use crate::tools::krita::options::{self, OptionSpec};
use crate::tools::krita::edit;
use crate::ui::curve_editor::curve_editor;
use crate::ui::theme;

/// What a Krita brush without a library id paints: the built-in Pencil-5,
/// which the library holds too.
const BUILT_IN: &str = "krita/c)_Pencil-5_Tilted";

/// The sensors this engine reads, as the editor names them.
const SENSORS: [(&str, &str); 11] = [
    ("pressure", "Pressure"),
    ("speed", "Speed"),
    ("drawingangle", "Direction"),
    ("fuzzy", "Random"),
    ("fade", "Fade"),
    ("xtilt", "Tilt X"),
    ("ytilt", "Tilt Y"),
    ("ascension", "Tilt direction"),
    ("declination", "Tilt amount"),
    ("rotation", "Barrel rotation"),
    ("tangentialpressure", "Wheel"),
];

/// `KisCurveOptionData::curveMode`: how two sensors' values combine.
const MODES: [&str; 5] = ["Multiply", "Add", "Maximum", "Minimum", "Difference"];

fn sensor_name(id: &str) -> &str {
    SENSORS.iter().find(|(s, _)| *s == id).map_or(id, |(_, n)| n)
}

/// Round a slider's value so an override reads like Krita's own.
fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

pub fn krita_editor(state: &mut AppState, ui: &mut egui::Ui) {
    let id = state.brush.krita_id.clone().unwrap_or_else(|| BUILT_IN.to_owned());
    let lib = &state.brush_library;
    let (Some(base), Some(cur)) = (lib.preset(&id), lib.edited_preset(&id, &state.brush.krita_overrides)) else {
        ui.label(RichText::new("This brush's preset can't be read, so it can't be edited.").small().color(theme::TEXT_MUTED));
        return;
    };
    let name = lib.entry(&id).map(|e| e.name.clone()).unwrap_or_default();
    let tips = lib.tips_for(&id);
    let built = lib.brush_with(&id, &state.brush.krita_overrides);
    let mut o = state.brush.krita_overrides.clone();
    let mut save_as = None;

    let head = ui.horizontal(|ui| {
        let r = ui.label(RichText::new(format!("Krita · {name}")).strong());
        if !o.is_empty() {
            ui.label(RichText::new("edited").small().color(theme::ACCENT));
        }
        r
    });
    match &built {
        Ok(b) => {
            head.inner.on_hover_text(crate::ui::brush_picker::describe(b).join("\n"));
        }
        Err(why) => {
            ui.label(RichText::new(format!("This change can't paint, so the brush paints unchanged: {why}")).small().color(theme::ACCENT_HOVER));
        }
    }
    save_row(ui, &name, &mut o, &mut save_as);
    let error_id = egui::Id::new("krita-save-error");
    if let Some(e) = ui.data(|d| d.get_temp::<String>(error_id)) {
        ui.label(RichText::new(e).small().color(theme::ACCENT_HOVER));
    }

    let wash = painting(ui, &base, &cur, &mut o);
    let auto = tip(ui, &base, &cur, &mut o, &tips, state.brush.radius as f64 * 2.0);
    let textured = texture(ui, &base, &cur, &mut o);
    egui::CollapsingHeader::new(RichText::new("Response").strong())
        .id_salt("krita-response")
        .default_open(true)
        .show(ui, |ui| {
            let list: [(OptionSpec, &str, bool, bool); 8] = [
                (options::SIZE, "Size", true, true),
                // Strength is the app's Opacity slider.
                (options::OPACITY, "Opacity", false, true),
                // Strength is Flow, above.
                (options::FLOW, "Flow", false, wash),
                (options::ROTATION, "Rotation", true, true),
                (options::RATIO, "Ratio", true, true),
                (options::SOFTNESS, "Softness", true, auto),
                (options::SCATTER, "Scatter", true, true),
                (options::TEXTURE_STRENGTH, "Texture strength", true, textured),
            ];
            for (spec, label, strength, shown) in list {
                if shown {
                    option(ui, &base, &cur, &mut o, &spec, label, strength);
                }
            }
        });

    if o != state.brush.krita_overrides {
        state.brush.krita_id.get_or_insert(id.clone());
        state.brush.krita_overrides = o;
    }
    if let Some(new_name) = save_as {
        match state.brush_library.save_as_new(&id, &state.brush.krita_overrides, &new_name) {
            Ok(new_id) => {
                state.brush.krita_id = Some(new_id);
                state.brush.krita_overrides.clear();
                state.forget_user_brush_thumbs();
                ui.data_mut(|d| d.remove::<String>(error_id));
            }
            Err(e) => ui.data_mut(|d| d.insert_temp(error_id, format!("Couldn't save: {e:#}"))),
        }
    }
}

/// Reset, and Save as new with its name field.
fn save_row(ui: &mut egui::Ui, name: &str, o: &mut Vec<(String, String)>, save_as: &mut Option<String>) {
    let name_id = egui::Id::new("krita-save-as-name");
    let mut pending: Option<String> = ui.data(|d| d.get_temp(name_id));
    ui.horizontal(|ui| {
        let edited = !o.is_empty();
        if ui
            .add_enabled(edited, egui::Button::new("Reset").small())
            .on_hover_text("Back to the preset as it came")
            .clicked()
        {
            o.clear();
        }
        if pending.is_none()
            && ui
                .small_button("Save as new…")
                .on_hover_text("Keep these settings as a brush of their own, under Imported")
                .clicked()
        {
            pending = Some(format!("{name} edited"));
        }
    });
    if let Some(text) = &mut pending {
        let mut close = false;
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(text).desired_width(150.0));
            if ui.add_enabled(!text.trim().is_empty(), egui::Button::new("Save").small()).clicked() {
                *save_as = Some(text.trim().to_owned());
                close = true;
            }
            if ui.small_button("Cancel").clicked() {
                close = true;
            }
        });
        if close {
            pending = None;
        }
    }
    ui.data_mut(|d| match pending {
        Some(p) => d.insert_temp(name_id, p),
        None => d.remove::<String>(name_id),
    });
}

/// Build-up or wash, and wash's flow. Returns whether it washes.
fn painting(ui: &mut egui::Ui, base: &Preset, cur: &Preset, o: &mut Vec<(String, String)>) -> bool {
    let wash = cur.int("PaintOpAction", 1) == 2;
    ui.horizontal(|ui| {
        ui.label("Painting");
        if ui
            .selectable_label(!wash, "Build-up")
            .on_hover_text("Each dab adds to the last, as a pencil does")
            .clicked()
            && wash
        {
            edit::set(base, o, "PaintOpAction", "1".into());
        }
        if ui
            .selectable_label(wash, "Wash")
            .on_hover_text("Dabs pool into one coat that never gets past the opacity")
            .clicked()
            && !wash
        {
            edit::set(base, o, "PaintOpAction", "2".into());
        }
    });
    if wash {
        let mut flow = cur.double("FlowValue", 1.0);
        if ui
            .add(egui::Slider::new(&mut flow, 0.0..=1.0).text("Flow"))
            .on_hover_text("How much each dab adds to the coat")
            .changed()
        {
            edit::set(base, o, "FlowValue", edit::num(r3(flow)));
        }
    }
    wash
}

/// The tip: which, its shape if generated, angle and spacing. Returns
/// whether it is generated.
fn tip(ui: &mut egui::Ui, base: &Preset, cur: &Preset, o: &mut Vec<(String, String)>, tips: &[String], diameter: f64) -> bool {
    let def = cur.string("brush_definition").unwrap_or_default();
    let get = |el: &str, k: &str, d: &str| edit::attr(&def, el, k).unwrap_or_else(|| d.to_owned());
    let auto = get("Brush", "type", "") == "auto_brush";
    let file = get("Brush", "filename", "");
    let spacing = get("Brush", "spacing", "0.1");
    let use_auto = get("Brush", "useAutoSpacing", "0");
    let coeff = get("Brush", "autoSpacingCoeff", "1");
    let num = |el: &str, k: &str, d: f64| edit::attr(&def, el, k).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(d);
    let mut new_def: Option<String> = None;
    let mut new_file: Option<String> = None;
    egui::CollapsingHeader::new(RichText::new("Tip").strong())
        .id_salt("krita-tip")
        .default_open(true)
        .show(ui, |ui| {
            let shown = if auto { "Generated".to_owned() } else { file.clone() };
            egui::ComboBox::from_id_salt("krita-tip-file").selected_text(shown).show_ui(ui, |ui| {
                if ui.selectable_label(auto, "Generated").clicked() && !auto {
                    new_def = Some(edit::auto_tip_definition(diameter, &spacing, &use_auto, &coeff));
                    new_file = Some(String::new());
                }
                for t in tips {
                    if ui.selectable_label(!auto && *t == file, t).clicked() && (auto || *t != file) {
                        new_def = Some(edit::bitmap_tip_definition(t, &spacing, &use_auto, &coeff));
                        new_file = Some(t.clone());
                    }
                }
            });
            let mut set = |el: &str, k: &str, v: String| {
                let d = new_def.clone().unwrap_or_else(|| def.clone());
                new_def = Some(edit::set_attr(&d, el, k, &v));
            };
            if auto {
                let round = get("MaskGenerator", "type", "circle") == "circle";
                let profile = get("MaskGenerator", "id", "default");
                ui.horizontal(|ui| {
                    ui.label("Shape");
                    if ui.selectable_label(round, "Round").clicked() && !round {
                        set("MaskGenerator", "type", "circle".into());
                    }
                    if ui.selectable_label(!round, "Square").clicked() && round {
                        set("MaskGenerator", "type", "rect".into());
                        // Only the hard square paints here.
                        set("MaskGenerator", "id", "default".into());
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Edge");
                    if ui.selectable_label(profile == "default", "Hard").clicked() && profile != "default" {
                        set("MaskGenerator", "id", "default".into());
                    }
                    let gauss = ui
                        .add_enabled(round, egui::SelectableLabel::new(profile == "gauss", "Gaussian"))
                        .on_disabled_hover_text("A gaussian square isn't supported");
                    if gauss.clicked() && profile != "gauss" {
                        set("MaskGenerator", "id", "gauss".into());
                    }
                    if profile == "soft" {
                        let _ = ui.selectable_label(true, "Soft curve");
                    }
                });
                let mut ratio = num("MaskGenerator", "ratio", 1.0);
                if ui.add(egui::Slider::new(&mut ratio, 0.01..=1.0).text("Ratio")).changed() {
                    set("MaskGenerator", "ratio", edit::num(r3(ratio)));
                }
                let mut fade = num("MaskGenerator", "hfade", 0.0);
                if ui
                    .add(egui::Slider::new(&mut fade, 0.0..=1.0).text("Fade"))
                    .on_hover_text("How far in from the edge the tip starts to fade")
                    .changed()
                {
                    set("MaskGenerator", "hfade", edit::num(r3(fade)));
                    set("MaskGenerator", "vfade", edit::num(r3(fade)));
                }
                let mut spikes = num("MaskGenerator", "spikes", 2.0) as i32;
                if ui.add(egui::Slider::new(&mut spikes, 2..=20).text("Spikes")).changed() {
                    set("MaskGenerator", "spikes", spikes.to_string());
                }
                let mut density = num("Brush", "density", 1.0);
                if ui
                    .add(egui::Slider::new(&mut density, 0.0..=1.0).text("Density"))
                    .on_hover_text("Below 1, pixels drop out of each dab at random")
                    .changed()
                {
                    set("Brush", "density", edit::num(r3(density)));
                }
                let mut random = num("Brush", "randomness", 0.0);
                if ui.add(egui::Slider::new(&mut random, 0.0..=1.0).text("Randomness")).changed() {
                    set("Brush", "randomness", edit::num(r3(random)));
                }
            }
            let mut angle = num("Brush", "angle", 0.0).to_degrees();
            if ui
                .add(egui::Slider::new(&mut angle, -180.0..=180.0).text("Angle").suffix("°"))
                .changed()
            {
                set("Brush", "angle", edit::num(angle.round().to_radians()));
            }
            let mut auto_spacing = use_auto == "1";
            if ui.checkbox(&mut auto_spacing, "Automatic spacing").changed() {
                set("Brush", "useAutoSpacing", if auto_spacing { "1" } else { "0" }.into());
            }
            if auto_spacing {
                let mut c = coeff.parse::<f64>().unwrap_or(1.0);
                if ui.add(egui::Slider::new(&mut c, 0.1..=3.0).text("Spacing ×")).changed() {
                    set("Brush", "autoSpacingCoeff", edit::num(r3(c)));
                }
            } else {
                let mut s = spacing.parse::<f64>().unwrap_or(0.1);
                if ui
                    .add(egui::Slider::new(&mut s, 0.02..=3.0).logarithmic(true).text("Spacing"))
                    .on_hover_text("The gap between dabs, as a fraction of the tip")
                    .changed()
                {
                    set("Brush", "spacing", edit::num(r3(s)));
                }
            }
        });
    if let Some(d) = new_def {
        edit::set(base, o, "brush_definition", d);
    }
    if let Some(f) = new_file {
        edit::set(base, o, "requiredBrushFile", f.clone());
        edit::set(base, o, "requiredBrushFilesList", f);
    }
    auto
}

/// The paper texture, for a preset that has one. Returns whether it is on.
fn texture(ui: &mut egui::Ui, base: &Preset, cur: &Preset, o: &mut Vec<(String, String)>) -> bool {
    if !cur.has("Texture/Pattern/Pattern") {
        return false;
    }
    let mut on = cur.bool("Texture/Pattern/Enabled", false);
    egui::CollapsingHeader::new(RichText::new("Paper texture").strong())
        .id_salt("krita-texture")
        .default_open(false)
        .show(ui, |ui| {
            if ui.checkbox(&mut on, "On").changed() {
                edit::set(base, o, "Texture/Pattern/Enabled", on.to_string());
            }
            ui.add_enabled_ui(on, |ui| {
                let mut scale = cur.double("Texture/Pattern/Scale", 1.0);
                if ui.add(egui::Slider::new(&mut scale, 0.1..=2.0).text("Scale")).changed() {
                    edit::set(base, o, "Texture/Pattern/Scale", edit::num(r3(scale)));
                }
                let mode = cur.int("Texture/Pattern/TexturingMode", 0);
                ui.horizontal(|ui| {
                    ui.label("Grain");
                    if ui.selectable_label(mode == 0, "Multiply").on_hover_text("The paper lightens the stroke").clicked() && mode != 0 {
                        edit::set(base, o, "Texture/Pattern/TexturingMode", "0".into());
                    }
                    if ui
                        .selectable_label(mode == 1, "Subtract")
                        .on_hover_text("The paper eats into the stroke, more as you press lightly")
                        .clicked()
                        && mode != 1
                    {
                        edit::set(base, o, "Texture/Pattern/TexturingMode", "1".into());
                    }
                });
                let mut invert = cur.bool("Texture/Pattern/Invert", false);
                if ui.checkbox(&mut invert, "Invert").changed() {
                    edit::set(base, o, "Texture/Pattern/Invert", invert.to_string());
                }
            });
        });
    on
}

/// One curve option: on or off, strength, the sensors it reads, and the
/// curve of the one picked.
fn option(ui: &mut egui::Ui, base: &Preset, cur: &Preset, o: &mut Vec<(String, String)>, spec: &OptionSpec, label: &str, strength: bool) {
    let Ok(mut d) = cur.option_data(spec) else {
        ui.label(RichText::new(format!("{label}: can't be edited")).small().color(theme::TEXT_MUTED));
        return;
    };
    let before = d.clone();
    let on = !spec.checkable || d.checked;
    let reads: Vec<&str> = d.slots.iter().filter(|s| s.active).map(|s| sensor_name(s.id)).collect();
    let header = if !on {
        format!("{label}: off")
    } else if !d.use_curve {
        format!("{label}: fixed")
    } else {
        format!("{label} ← {}", reads.join(" + "))
    };
    egui::CollapsingHeader::new(header)
        .id_salt(("krita-option", spec.id))
        .default_open(false)
        .show(ui, |ui| {
            if spec.checkable {
                ui.checkbox(&mut d.checked, "On");
            }
            ui.add_enabled_ui(!spec.checkable || d.checked, |ui| option_body(ui, &mut d, spec, strength));
        });
    if d != before {
        let written = d.write(spec);
        let keys: Vec<String> = written.iter().map(|(k, _)| k.clone()).collect();
        edit::set_all(base, o, written);
        // Back where the preset had it: no overrides needed.
        if base.option_data(spec).ok().as_ref() == Some(&d) {
            o.retain(|(k, _)| !keys.contains(k));
        }
    }
}

fn option_body(ui: &mut egui::Ui, d: &mut OptionData, spec: &OptionSpec, strength: bool) {
    if strength {
        ui.add(egui::Slider::new(&mut d.strength, spec.min..=spec.max).text("Strength"));
        d.strength = r3(d.strength);
    }
    ui.checkbox(&mut d.use_curve, "Follow the pen")
        .on_hover_text("Off: the option holds at its strength, whatever the pen does");
    if !d.use_curve {
        return;
    }
    let active = d.slots.iter().filter(|s| s.active).count();
    ui.horizontal_wrapped(|ui| {
        for (sid, name) in SENSORS {
            let Some(slot) = d.slots.iter_mut().find(|s| s.id == sid) else { continue };
            let mut on = slot.active;
            // One sensor always stays on, as in Krita.
            let lone = on && active == 1;
            if ui.add_enabled(!lone, egui::Checkbox::new(&mut on, name)).changed() {
                slot.active = on;
            }
        }
    });
    let active: Vec<&'static str> = d.slots.iter().filter(|s| s.active).map(|s| s.id).collect();
    if active.len() > 1 {
        ui.horizontal(|ui| {
            ui.label("Combine");
            egui::ComboBox::from_id_salt(("krita-mode", spec.id))
                .selected_text(*MODES.get(d.curve_mode as usize).unwrap_or(&"Multiply"))
                .show_ui(ui, |ui| {
                    for (i, m) in MODES.iter().enumerate() {
                        ui.selectable_value(&mut d.curve_mode, i as i32, *m);
                    }
                });
        });
        ui.checkbox(&mut d.use_same_curve, "One curve for all");
    }
    // Which sensor's curve the widget shows.
    let pick_id = egui::Id::new(("krita-curve-of", spec.id));
    let mut picked: String = ui.data(|m| m.get_temp(pick_id)).unwrap_or_default();
    if !active.contains(&picked.as_str()) {
        picked = active.first().map(|s| s.to_string()).unwrap_or_default();
    }
    if !d.use_same_curve && active.len() > 1 {
        ui.horizontal(|ui| {
            ui.label("Curve of");
            egui::ComboBox::from_id_salt(("krita-curve-pick", spec.id))
                .selected_text(sensor_name(&picked))
                .show_ui(ui, |ui| {
                    for s in &active {
                        ui.selectable_value(&mut picked, s.to_string(), sensor_name(s));
                    }
                });
        });
    }
    ui.data_mut(|m| m.insert_temp(pick_id, picked.clone()));
    let curve_id = egui::Id::new(("krita-curve", spec.id, picked.clone(), d.use_same_curve));
    if d.use_same_curve {
        curve_editor(ui, curve_id, &mut d.common_curve);
    } else if let Some(slot) = d.slots.iter_mut().find(|s| s.id == picked) {
        curve_editor(ui, curve_id, &mut slot.curve);
    }
    // The two sensors with settings of their own.
    if let Some(s) = d.slots.iter_mut().find(|s| s.id == "drawingangle" && s.active) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.fan_corners_enabled, "Fan corners")
                .on_hover_text("Fill sharp turns with dabs at the angles between");
            ui.add_enabled(s.fan_corners_enabled, egui::DragValue::new(&mut s.fan_corners_step).range(5..=90).suffix("°"));
        });
        ui.add(egui::Slider::new(&mut s.angle_offset, 0..=359).text("Direction offset").suffix("°"));
    }
    if let Some(s) = d.slots.iter_mut().find(|s| s.id == "fade" && s.active) {
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut s.length).range(1..=10000).prefix("Fade over ").suffix(" px"));
            ui.checkbox(&mut s.periodic, "Repeat");
        });
    }
}
