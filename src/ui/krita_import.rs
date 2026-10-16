//! The Import Krita brushes window: the presets found in the installed
//! Krita, grouped by folder or bundle. Each row says whether this app paints
//! it; one that does has Import, or Use once it is here.

use std::time::Duration;

use egui::{RichText, Vec2};
use egui_phosphor::regular as ic;

use crate::app::AppState;
use crate::brush_library::import::{Candidate, KritaImport, Known};
use crate::ui::brush_picker::{pick, PAPER};
use crate::ui::shell::{drag_by_title_only, floating_frame, title_shortfall};
use crate::ui::theme;

/// A row's thumbnail size.
const THUMB: f32 = 28.0;

/// What a click asked for, done once the window is drawn.
enum Do {
    Import(usize),
    ImportAll(usize),
    Use(String, String),
    AddFiles,
}

pub fn krita_import_window(state: &mut AppState, ctx: &egui::Context) {
    let Some(imp) = state.krita_import.as_mut() else { return };
    if imp.poll() || imp.scanning() {
        ctx.request_repaint_after(Duration::from_millis(50));
    }
    for (i, c) in imp.candidates.iter().enumerate() {
        if let (false, Some(img)) = (imp.thumbs.contains_key(&i), &c.thumb) {
            let tex = ctx.load_texture(format!("krita-import-{i}"), img.clone(), egui::TextureOptions::LINEAR);
            imp.thumbs.insert(i, tex);
        }
    }
    let mut open = true;
    let mut act = None;
    let title = theme::icon_text(ic::DOWNLOAD_SIMPLE, "Import Krita brushes");
    let shortfall = title_shortfall(ctx, &title);
    egui::Window::new(title)
        // Pinned, like the other windows, so a rename keeps its place.
        .id(egui::Id::new("window_krita_import"))
        .open(&mut open)
        .default_pos([320.0, 100.0])
        .default_size([400.0, 520.0])
        .resizable(true)
        .collapsible(true)
        .frame(floating_frame())
        .show(ctx, |ui| {
            drag_by_title_only(ui);
            body(imp, ui, shortfall, &mut act);
        });
    match act {
        Some(Do::Import(i)) => import(state, &[i]),
        Some(Do::ImportAll(g)) => {
            let Some(imp) = &state.krita_import else { return };
            let new: Vec<usize> = imp.visible(g).into_iter().filter(|&i| importable(&imp.candidates[i])).collect();
            import(state, &new);
        }
        Some(Do::Use(id, name)) => {
            pick(state, &id);
            if let Some(imp) = &mut state.krita_import {
                imp.notice = Some(format!("Painting with {name}."));
            }
        }
        Some(Do::AddFiles) => {
            let picked = rfd::FileDialog::new()
                .set_title("Add Krita brushes")
                .add_filter("Krita presets and bundles", &["kpp", "bundle"])
                .pick_files();
            if let (Some(paths), Some(imp)) = (picked, &mut state.krita_import) {
                imp.add_files(&paths);
            }
        }
        None => {}
    }
    if !open {
        state.close_krita_import();
    }
}

fn importable(c: &Candidate) -> bool {
    c.status.is_ok() && c.known == Known::New
}

/// Import candidates `which`, then show the library what arrived.
fn import(state: &mut AppState, which: &[usize]) {
    let Some(imp) = &mut state.krita_import else { return };
    let mut done = Vec::new();
    let mut failed = None;
    for &i in which {
        let name = imp.candidates[i].name.clone();
        match imp.import(i) {
            Ok(_) => done.push(name),
            Err(e) => {
                failed = Some(format!("Couldn't import {name}: {e:#}"));
                break;
            }
        }
    }
    imp.notice = match (&done[..], failed) {
        (_, Some(why)) => Some(why),
        ([one], None) => Some(format!("Imported {one}. It's in Brush settings, under Imported.")),
        (many, None) => Some(format!("Imported {} brushes. They're in Brush settings, under Imported.", many.len())),
    };
    if !done.is_empty() {
        state.rescan_brushes();
    }
}

fn body(imp: &mut KritaImport, ui: &mut egui::Ui, shortfall: f32, act: &mut Option<Do>) {
    let found = imp.candidates.len();
    let paints = imp.candidates.iter().filter(|c| c.status.is_ok()).count();
    if imp.groups.is_empty() {
        ui.label("Krita's brushes weren't found on this computer.");
        ui.label(
            RichText::new("If Krita keeps them somewhere else, add its .kpp or .bundle files.")
                .small()
                .color(theme::TEXT_MUTED),
        );
    } else if imp.scanning() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(format!("Reading Krita's brushes… {found} found"));
        });
    } else {
        ui.label(RichText::new(format!("{found} presets found, {paints} of them paint here.")).color(theme::TEXT_MUTED));
    }
    ui.horizontal(|ui| {
        ui.label(theme::icon_text(ic::MAGNIFYING_GLASS, ""));
        ui.add(egui::TextEdit::singleline(&mut imp.search).hint_text("Search").desired_width(150.0));
        ui.checkbox(&mut imp.show_unsupported, "Show the rest")
            .on_hover_text("Also list presets this app can't paint, with the reason");
    });
    if ui
        .small_button(theme::icon_text(ic::FOLDER_OPEN, "Add .kpp or .bundle files…"))
        .on_hover_text("Brushes downloaded or kept outside Krita's folders")
        .clicked()
    {
        *act = Some(Do::AddFiles);
    }
    if let Some(n) = &imp.notice {
        ui.label(RichText::new(n).color(theme::ACCENT));
    }
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("krita-import-list")
        .auto_shrink([false, false])
        // The list fills the window; see `title_shortfall`.
        .max_height((ui.available_height() - shortfall).max(0.0))
        .show(ui, |ui| {
            for g in 0..imp.groups.len() {
                let rows = imp.visible(g);
                if rows.is_empty() {
                    continue;
                }
                let group = &imp.groups[g];
                let all = imp.candidates.iter().filter(|c| c.group == g).count();
                let ok = imp.candidates.iter().filter(|c| c.group == g && c.status.is_ok()).count();
                let new = rows.iter().filter(|&&i| importable(&imp.candidates[i])).count();
                let header = egui::CollapsingHeader::new(RichText::new(format!("{}  ·  {ok} of {all} paint", group.label)).strong())
                    .id_salt(("krita-import-group", group.path.display().to_string()))
                    .default_open(true)
                    .show(ui, |ui| {
                        if new > 1 && ui.small_button(format!("Import all {new}")).clicked() {
                            *act = Some(Do::ImportAll(g));
                        }
                        for i in rows {
                            row(ui, &imp.candidates[i], imp.thumbs.get(&i), i, act);
                        }
                    });
                header.header_response.on_hover_text(group.path.display().to_string());
            }
        });
}

fn row(ui: &mut egui::Ui, c: &Candidate, thumb: Option<&egui::TextureHandle>, i: usize, act: &mut Option<Do>) {
    let ok = c.status.is_ok();
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(THUMB), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 3.0, PAPER.gamma_multiply(if ok { 1.0 } else { 0.35 }));
        if let Some(t) = thumb {
            let [w, h] = t.size();
            let s = THUMB / w.max(h).max(1) as f32;
            let img = egui::Rect::from_center_size(rect.center(), Vec2::new(w as f32 * s, h as f32 * s));
            let tint = if ok { egui::Color32::WHITE } else { theme::white_alpha(70) };
            p.image(t.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), tint);
        }
        let name = RichText::new(&c.name).color(if ok { ui.visuals().text_color() } else { theme::TEXT_MUTED });
        ui.label(name);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match (&c.status, c.known, &c.id) {
            (Err(why), _, _) => {
                ui.label(RichText::new("Can't paint").small().color(theme::TEXT_MUTED)).on_hover_text(why);
            }
            (Ok(()), Known::New, _) => {
                if ui.button("Import").on_hover_text("Copy it into this app's brushes").clicked() {
                    *act = Some(Do::Import(i));
                }
            }
            (Ok(()), known, Some(id)) => {
                if ui.button("Use").on_hover_text("Paint with it now").clicked() {
                    *act = Some(Do::Use(id.clone(), c.name.clone()));
                }
                let what = if known == Known::BuiltIn { "Built in" } else { "Imported" };
                ui.label(RichText::new(what).small().color(theme::TEXT_MUTED));
            }
            (Ok(()), _, None) => {}
        });
    });
}
