//! Top-level egui layout: modern dark UI with Phosphor icons.
//! Floating windows, transparent canvas, layered + onion composite.

use egui::{Align, Color32, Frame, Layout, Margin, Rect, Sense, Stroke, Vec2};
use egui_phosphor::regular as ic;

use crate::app::AppState;
use crate::input::shortcuts::Action;
use crate::io::{composite, gif_export, png_save, png_seq};
use crate::timeline::onion::OnionDirection;
use crate::tools::{ActiveTool, BrushSettings};
use crate::ui::theme;

/// Tooltip text including the currently-bound shortcut (e.g. "Pencil  (Q)").
fn tip(state: &AppState, action: Action, base: &str) -> String {
    match state.shortcuts.get(action) {
        Some(c) => format!("{base}  ({})", c.display()),
        None => base.to_string(),
    }
}

pub fn draw(state: &mut AppState, ctx: &egui::Context) {
    if state.show_panels {
        menu_window(state, ctx);
        tools_window(state, ctx);
        brush_window(state, ctx);
        timeline_window(state, ctx);
        onion_window(state, ctx);
        layers_window(state, ctx);
        xsheet_window(state, ctx);
        settings_window(state, ctx);
    }

    egui::CentralPanel::default()
        .frame(Frame::none().fill(Color32::TRANSPARENT))
        .show(ctx, |ui| {
            let avail = ui.available_size();
            let (canvas_rect, resp) = ui.allocate_exact_size(avail, Sense::drag());
            paint_canvas(state, ui, canvas_rect);

            let canvas_to_doc = canvas_to_doc_mapping(state, canvas_rect);

            if resp.drag_started() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let (x, y) = canvas_to_doc(pos);
                    let t = ui.input(|i| i.time as f32);
                    let s = state.make_sample(x, y, t);
                    state.pointer_down(s);
                }
            }
            if resp.dragged() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let (x, y) = canvas_to_doc(pos);
                    let t = ui.input(|i| i.time as f32);
                    let s = state.make_sample(x, y, t);
                    state.pointer_move(s);
                }
            }
            if resp.drag_stopped() {
                state.pointer_up();
            }

            // Tool cursor preview — drawn on top, only when pointer is over the
            // canvas area and not over a floating panel.
            if resp.hovered() || resp.dragged() {
                let pos = resp.hover_pos().or_else(|| resp.interact_pointer_pos());
                if let Some(pos) = pos {
                    if canvas_rect.contains(pos) {
                        draw_tool_cursor(state, ui, canvas_rect, pos);
                        ctx.set_cursor_icon(egui::CursorIcon::None);
                    }
                }
            }
        });
}

fn tools_window(state: &mut AppState, ctx: &egui::Context) {
    egui::Window::new(theme::icon_text(ic::TOOLBOX, "Tools"))
        .default_pos([16.0, 60.0])
        .default_width(230.0)
        .resizable(true)
        .collapsible(true)
        .frame(floating_frame())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let p = tip(state, Action::ToolPencil, "Pencil");
                let i = tip(state, Action::ToolInk, "Ink");
                let e = tip(state, Action::ToolEraser, "Eraser");
                let f = tip(state, Action::ToolFill, "Fill");
                tool_toggle(ui, state, ActiveTool::Pencil, ic::PENCIL, &p);
                tool_toggle(ui, state, ActiveTool::Ink, ic::PEN_NIB, &i);
                tool_toggle(ui, state, ActiveTool::Eraser, ic::ERASER, &e);
                tool_toggle(ui, state, ActiveTool::Fill, ic::PAINT_BUCKET, &f);
            });
            ui.add_space(4.0);

            if state.tool == ActiveTool::Fill {
                ui.add(
                    egui::Slider::new(&mut state.brush.fill_tolerance, 0..=128).text("Tolerance"),
                );
            } else {
                ui.add(egui::Slider::new(&mut state.brush.radius, 0.5..=128.0).text("Size"));
                ui.add(egui::Slider::new(&mut state.brush.opacity, 0.0..=1.0).text("Flow"));
            }

            ui.add_space(6.0);
            theme::section_header(ui, ic::MONITOR, "Window");
            ui.add(egui::Slider::new(&mut state.bg_opacity, 0.0..=1.0).text("Background α"));
            ui.checkbox(&mut state.show_checker, "Checker backdrop");

            ui.add_space(4.0);
            let pen_label = if state.pen.is_active() {
                egui::RichText::new(format!("{}  Tablet active", ic::PEN))
                    .color(theme::ACCENT)
                    .size(11.0)
            } else {
                egui::RichText::new(format!("{}  Mouse mode", ic::CURSOR))
                    .color(theme::TEXT_MUTED)
                    .size(11.0)
            };
            ui.label(pen_label);
        });
}

fn brush_window(state: &mut AppState, ctx: &egui::Context) {
    egui::Window::new(theme::icon_text(ic::PAINT_BRUSH, "Brush"))
        .default_pos([16.0, 290.0])
        .default_width(230.0)
        .resizable(true)
        .collapsible(true)
        .frame(floating_frame())
        .show(ctx, |ui| {
            theme::section_header(ui, ic::PALETTE, "Color");
            ui.horizontal(|ui| {
                let mut rgba = [
                    state.brush.color[0] as f32 / 255.0,
                    state.brush.color[1] as f32 / 255.0,
                    state.brush.color[2] as f32 / 255.0,
                ];
                if ui.color_edit_button_rgb(&mut rgba).changed() {
                    state.brush.color[0] = (rgba[0] * 255.0).round() as u8;
                    state.brush.color[1] = (rgba[1] * 255.0).round() as u8;
                    state.brush.color[2] = (rgba[2] * 255.0).round() as u8;
                }
                ui.label(format!(
                    "#{:02X}{:02X}{:02X}",
                    state.brush.color[0], state.brush.color[1], state.brush.color[2]
                ));
            });

            ui.add_space(6.0);
            theme::section_header(ui, ic::SLIDERS, "Dynamics");
            ui.add(egui::Slider::new(&mut state.brush.hardness, 0.5..=8.0).text("Hardness"));
            ui.add(egui::Slider::new(&mut state.brush.spacing, 0.02..=0.5).text("Spacing"));
            ui.add(egui::Slider::new(&mut state.brush.pressure_size, 0.0..=1.0).text("Pres → size"));
            ui.add(
                egui::Slider::new(&mut state.brush.pressure_opacity, 0.0..=1.0)
                    .text("Pres → flow"),
            );
        });
}

fn timeline_window(state: &mut AppState, ctx: &egui::Context) {
    egui::Window::new(theme::icon_text(ic::FILM_STRIP, "Timeline"))
        .default_pos([300.0, 720.0])
        .default_width(720.0)
        .resizable(true)
        .collapsible(true)
        .frame(floating_frame())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let play_icon = if state.playback.playing { ic::PAUSE } else { ic::PLAY };
                let play_base = if state.playback.playing { "Pause" } else { "Play" };
                let play_tip = tip(state, Action::PlayPause, play_base);
                if theme::icon_button(ui, play_icon, &play_tip).clicked() {
                    let now = ctx.input(|i| i.time);
                    state.playback.toggle(now);
                }
                if theme::icon_button(ui, ic::SKIP_BACK, "Go to loop start").clicked() {
                    state.project.goto(state.project.loop_start);
                }
                let prev_tip = tip(state, Action::FramePrev, "Step back");
                if theme::icon_button(ui, ic::CARET_LEFT, &prev_tip).clicked() {
                    state.project.step(-1);
                }
                let next_tip = tip(state, Action::FrameNext, "Step forward");
                if theme::icon_button(ui, ic::CARET_RIGHT, &next_tip).clicked() {
                    state.project.step(1);
                }
                ui.separator();
                let add_tip = tip(state, Action::FrameAdd, "Add frame (hold)");
                if theme::icon_button(ui, ic::PLUS, &add_tip).clicked() {
                    state.project.add_frame();
                }
                let dup_tip = tip(state, Action::FrameDuplicate, "Duplicate frame");
                if theme::icon_button(ui, ic::COPY, &dup_tip).clicked() {
                    state.project.duplicate_frame();
                }
                let del_tip = tip(state, Action::FrameDelete, "Delete frame");
                if theme::icon_button(ui, ic::TRASH, &del_tip).clicked() {
                    state.project.delete_frame();
                }
                ui.separator();
                ui.add(egui::Slider::new(&mut state.project.fps, 1.0..=60.0).text("fps"));
            });

            ui.add_space(4.0);

            let n = state.project.frame_count.max(1);
            let mut cur = state.project.current_frame;
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(ic::CLOCK)
                        .color(theme::TEXT_MUTED)
                        .size(13.0),
                );
                let resp = ui.add(
                    egui::Slider::new(&mut cur, 0..=n.saturating_sub(1))
                        .integer()
                        .show_value(true)
                        .text("frame"),
                );
                if resp.changed() {
                    state.project.goto(cur);
                }
                ui.label(
                    egui::RichText::new(format!("/ {n}"))
                        .color(theme::TEXT_MUTED),
                );
            });

            frame_strip(state, ui);
        });
}

fn frame_strip(state: &mut AppState, ui: &mut egui::Ui) {
    let n = state.project.frame_count.max(1);
    let avail = ui.available_size_before_wrap();
    let height = 26.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(avail.x, height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let cell_w = rect.width() / n as f32;

    painter.rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(10, 11, 14, 220));
    for i in 0..n {
        let x = rect.min.x + i as f32 * cell_w;
        let r = Rect::from_min_size(egui::pos2(x, rect.min.y), egui::vec2(cell_w, height));
        let fill = if i == state.project.current_frame {
            theme::ACCENT
        } else if i >= state.project.loop_start && i < state.project.loop_end {
            theme::BG_HOVER
        } else {
            theme::BG_INACTIVE
        };
        painter.rect_filled(r.shrink(1.5), 3.0, fill);
        if cell_w > 18.0 {
            let txt_color = if i == state.project.current_frame {
                Color32::WHITE
            } else {
                theme::TEXT_MUTED
            };
            painter.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                format!("{i}"),
                egui::FontId::monospace(10.0),
                txt_color,
            );
        }
    }

    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let rel = ((pos.x - rect.min.x) / cell_w).floor() as isize;
            let idx = rel.clamp(0, n as isize - 1) as usize;
            state.project.goto(idx);
        }
    }
}

fn onion_window(state: &mut AppState, ctx: &egui::Context) {
    egui::Window::new(theme::icon_text(ic::CIRCLES_THREE, "Onion skin"))
        .default_pos([260.0, 60.0])
        .default_open(false)
        .frame(floating_frame())
        .show(ctx, |ui| {
            ui.checkbox(&mut state.onion.enabled, "Enabled");
            ui.add_space(4.0);
            ui.add(egui::Slider::new(&mut state.onion.prev, 0..=8).text("Prev"));
            ui.add(egui::Slider::new(&mut state.onion.next, 0..=8).text("Next"));
            ui.add(egui::Slider::new(&mut state.onion.max_alpha, 0.0..=1.0).text("Max α"));
            ui.add(egui::Slider::new(&mut state.onion.falloff, 0.5..=4.0).text("Falloff"));
            ui.add_space(4.0);
            theme::section_header(ui, ic::PALETTE, "Tints");
            color_picker_u8(ui, "Prev", &mut state.onion.prev_tint);
            color_picker_u8(ui, "Next", &mut state.onion.next_tint);
        });
}

fn layers_window(state: &mut AppState, ctx: &egui::Context) {
    egui::Window::new(theme::icon_text(ic::STACK, "Layers"))
        .default_pos([1040.0, 60.0])
        .default_width(280.0)
        .resizable(true)
        .collapsible(true)
        .frame(floating_frame())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if theme::icon_button(ui, ic::PLUS, "Add layer").clicked() {
                    state.project.add_layer();
                }
                if theme::icon_button(ui, ic::MINUS, "Delete layer").clicked() {
                    state.project.delete_layer();
                }
                if theme::icon_button(ui, ic::ARROW_UP, "Move layer up").clicked() {
                    state.project.move_layer_up();
                }
                if theme::icon_button(ui, ic::ARROW_DOWN, "Move layer down").clicked() {
                    state.project.move_layer_down();
                }
            });
            ui.add_space(4.0);
            ui.separator();

            let n = state.project.layers.len();
            let cur = state.project.current_layer;
            let mut select: Option<usize> = None;
            for i in (0..n).rev() {
                let layer = &mut state.project.layers[i];
                let selected = i == cur;

                Frame::none()
                    .fill(if selected {
                        theme::ACCENT_DIM
                    } else {
                        Color32::TRANSPARENT
                    })
                    .rounding(egui::Rounding::same(6.0))
                    .inner_margin(Margin::symmetric(6.0, 4.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // Eye toggle.
                            let eye_icon = if layer.visible { ic::EYE } else { ic::EYE_SLASH };
                            if theme::icon_button(ui, eye_icon, "Toggle visibility").clicked() {
                                layer.visible = !layer.visible;
                            }
                            // Lock toggle.
                            let lock_icon = if layer.locked {
                                ic::LOCK_SIMPLE
                            } else {
                                ic::LOCK_SIMPLE_OPEN
                            };
                            if theme::icon_button(ui, lock_icon, "Toggle lock").clicked() {
                                layer.locked = !layer.locked;
                            }
                            // Reference / light-table toggle.
                            let ref_color = if layer.reference {
                                theme::ACCENT
                            } else {
                                theme::TEXT_MUTED
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(ic::LIGHTBULB).size(16.0).color(ref_color),
                                    )
                                    .min_size(egui::vec2(30.0, 24.0)),
                                )
                                .on_hover_text("Light-table reference layer")
                                .clicked()
                            {
                                layer.reference = !layer.reference;
                            }
                            // Name select.
                            let resp = ui.add(
                                egui::SelectableLabel::new(
                                    selected,
                                    egui::RichText::new(&layer.name).strong(),
                                ),
                            );
                            if resp.clicked() {
                                select = Some(i);
                            }
                        });
                        ui.add(egui::Slider::new(&mut layer.opacity, 0.0..=1.0).text("opacity"));
                    });
                ui.add_space(2.0);
            }
            if let Some(i) = select {
                state.project.current_layer = i;
            }
        });
}

fn xsheet_window(state: &mut AppState, ctx: &egui::Context) {
    egui::Window::new(theme::icon_text(ic::TABLE, "X-sheet"))
        .default_pos([1040.0, 420.0])
        .default_width(280.0)
        .default_open(false)
        .resizable(true)
        .collapsible(true)
        .frame(floating_frame())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if theme::icon_button(ui, ic::PLUS_SQUARE, "Insert blank key").clicked() {
                    let id = state.project.insert_blank_key_here();
                    state.mark_dirty(id);
                }
                if theme::icon_button(ui, ic::COPY, "Insert duplicate key").clicked() {
                    let id = state.project.insert_duplicate_key_here();
                    state.mark_dirty(id);
                }
                if theme::icon_button(ui, ic::PUSH_PIN, "Hold (delete key)").clicked() {
                    state.project.hold_here();
                }
            });
            ui.add_space(4.0);
            ui.separator();

            let layer_count = state.project.layers.len();
            let frame_count = state.project.frame_count;

            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("xsheet_grid")
                        .striped(true)
                        .min_col_width(28.0)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("Fr")
                                    .color(theme::TEXT_MUTED)
                                    .strong(),
                            );
                            for li in 0..layer_count {
                                ui.label(
                                    egui::RichText::new(&state.project.layers[li].name)
                                        .color(theme::TEXT_MUTED)
                                        .strong(),
                                );
                            }
                            ui.end_row();

                            for f in 0..frame_count {
                                let active_f = f == state.project.current_frame;
                                let lbl = if active_f {
                                    egui::RichText::new(format!("{}  {f}", ic::CARET_RIGHT))
                                        .color(theme::ACCENT)
                                        .strong()
                                } else {
                                    egui::RichText::new(format!("{f}")).color(theme::TEXT_MUTED)
                                };
                                if ui.button(lbl).clicked() {
                                    state.project.goto(f);
                                }
                                for li in 0..layer_count {
                                    let cell = state.project.layers[li].exposures[f];
                                    let active_l = li == state.project.current_layer;
                                    let label = match cell {
                                        Some(id) => format!("{id}"),
                                        None => "·".to_string(),
                                    };
                                    let selected = active_f && active_l;
                                    let resp = ui.selectable_label(selected, label);
                                    if resp.clicked() {
                                        state.project.current_frame = f;
                                        state.project.current_layer = li;
                                    }
                                }
                                ui.end_row();
                            }
                        });
                });
        });
}

fn settings_window(state: &mut AppState, ctx: &egui::Context) {
    if !state.show_settings {
        return;
    }
    let mut open = state.show_settings;
    egui::Window::new(theme::icon_text(ic::GEAR, "Settings — Shortcuts"))
        .open(&mut open)
        .default_pos([360.0, 80.0])
        .default_width(420.0)
        .resizable(true)
        .collapsible(true)
        .frame(floating_frame())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Click a binding, then press a new key combo. Esc = cancel.")
                        .color(theme::TEXT_MUTED)
                        .size(11.0),
                );
            });
            if let Some(action) = state.rebinding {
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{}  Rebinding: {} — press any key…",
                        ic::KEYBOARD,
                        action.label()
                    ))
                    .color(theme::TEXT)
                    .strong(),
                );
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui
                    .button(theme::icon_text(ic::ARROW_COUNTER_CLOCKWISE, "Reset to defaults"))
                    .clicked()
                {
                    state.shortcuts = crate::input::shortcuts::ShortcutMap::default();
                    crate::input::shortcuts::save(&state.shortcuts);
                }
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("shortcuts_grid")
                        .num_columns(3)
                        .striped(true)
                        .min_col_width(100.0)
                        .show(ui, |ui| {
                            for &action in Action::ALL {
                                ui.label(action.label());
                                let combo_text = state
                                    .shortcuts
                                    .get(action)
                                    .map(|c| c.display())
                                    .unwrap_or_else(|| "—".to_string());
                                let is_active = state.rebinding == Some(action);
                                let btn = egui::Button::new(if is_active {
                                    "Press key…".to_string()
                                } else {
                                    combo_text
                                })
                                .min_size(egui::vec2(140.0, 22.0));
                                if ui.add(btn).clicked() {
                                    state.rebinding = if is_active { None } else { Some(action) };
                                }
                                if ui
                                    .small_button(theme::icon_text(ic::X, ""))
                                    .on_hover_text("Unbind")
                                    .clicked()
                                {
                                    state.shortcuts.bindings.remove(&action);
                                    crate::input::shortcuts::save(&state.shortcuts);
                                }
                                ui.end_row();
                            }
                        });
                });
        });
    state.show_settings = open;
}

fn color_picker_u8(ui: &mut egui::Ui, label: &str, c: &mut [u8; 4]) {
    ui.horizontal(|ui| {
        let mut rgb = [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0];
        if ui.color_edit_button_rgb(&mut rgb).changed() {
            c[0] = (rgb[0] * 255.0).round() as u8;
            c[1] = (rgb[1] * 255.0).round() as u8;
            c[2] = (rgb[2] * 255.0).round() as u8;
        }
        ui.label(label);
    });
}

fn floating_frame() -> Frame {
    Frame::window(&egui::Style::default())
        .fill(theme::BG_PANEL)
        .stroke(Stroke::new(1.0, theme::STROKE_THIN))
        .inner_margin(Margin::same(7.0))
        .rounding(egui::Rounding::same(7.0))
        .shadow(egui::Shadow {
            offset: egui::vec2(0.0, 3.0),
            blur: 12.0,
            spread: 0.0,
            color: Color32::from_black_alpha(140),
        })
}

fn tool_toggle(
    ui: &mut egui::Ui,
    state: &mut AppState,
    target: ActiveTool,
    icon: &str,
    label: &str,
) {
    let selected = state.tool == target;
    if theme::icon_toggle(ui, icon, label, selected).clicked() && !selected {
        state.tool = target;
        match target {
            ActiveTool::Pencil => state.brush = BrushSettings::default_pencil(),
            ActiveTool::Ink => state.brush = BrushSettings::default_ink(),
            ActiveTool::Eraser => state.brush = BrushSettings::default_eraser(),
            ActiveTool::Fill => state.brush = BrushSettings::default_fill(),
        }
    }
}

fn paint_canvas(state: &AppState, ui: &mut egui::Ui, rect: Rect) {
    let painter = ui.painter_at(rect);

    let cw = state.project.width as f32;
    let ch = state.project.height as f32;
    let scale = (rect.width() / cw).min(rect.height() / ch);
    let dw = cw * scale;
    let dh = ch * scale;
    let origin = rect.center() - Vec2::new(dw, dh) * 0.5;
    let dst = Rect::from_min_size(origin, Vec2::new(dw, dh));

    if state.show_checker {
        let cell = 12.0;
        let cols = (dst.width() / cell).ceil() as i32;
        let rows = (dst.height() / cell).ceil() as i32;
        let a = Color32::from_gray(48);
        let b = Color32::from_gray(64);
        for r in 0..rows {
            for c in 0..cols {
                let color = if (r + c) & 1 == 0 { a } else { b };
                let p = dst.min + Vec2::new(c as f32 * cell, r as f32 * cell);
                let cell_rect = Rect::from_min_size(p, Vec2::new(cell, cell)).intersect(dst);
                painter.rect_filled(cell_rect, 0.0, color);
            }
        }
    }

    let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    let cur_frame = state.project.current_frame;
    let cur_layer = state.project.current_layer;

    for layer in state.project.layers.iter() {
        if !layer.visible || !layer.reference {
            continue;
        }
        if let Some(id) = layer.resolve(cur_frame) {
            if let Some(tex) = state.cell_textures.get(&id) {
                let dim = (layer.opacity * 0.45).clamp(0.0, 1.0);
                let a = (dim * 255.0) as u8;
                painter.image(
                    tex.id(),
                    dst,
                    uv,
                    Color32::from_rgba_unmultiplied(255, 255, 255, a),
                );
            }
        }
    }

    if state.onion.enabled {
        if let Some(layer) = state.project.layers.get(cur_layer) {
            for k in (1..=state.onion.prev).rev() {
                let f_i = cur_frame as isize - k as isize;
                if f_i < 0 {
                    continue;
                }
                let f = f_i as usize;
                if let Some(id) = layer.resolve(f) {
                    if let Some(tex) = state.cell_textures.get(&id) {
                        let tint = state.onion.tint_for(k, OnionDirection::Prev);
                        let c = Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], tint[3]);
                        painter.image(tex.id(), dst, uv, c);
                    }
                }
            }
        }
    }

    for layer in &state.project.layers {
        if !layer.visible || layer.reference {
            continue;
        }
        let Some(id) = layer.resolve(cur_frame) else {
            continue;
        };
        let Some(tex) = state.cell_textures.get(&id) else {
            continue;
        };
        let a = (layer.opacity.clamp(0.0, 1.0) * 255.0) as u8;
        painter.image(
            tex.id(),
            dst,
            uv,
            Color32::from_rgba_unmultiplied(255, 255, 255, a),
        );
    }

    if state.onion.enabled {
        if let Some(layer) = state.project.layers.get(cur_layer) {
            for k in 1..=state.onion.next {
                let f = cur_frame + k as usize;
                if f >= state.project.frame_count {
                    continue;
                }
                if let Some(id) = layer.resolve(f) {
                    if let Some(tex) = state.cell_textures.get(&id) {
                        let tint = state.onion.tint_for(k, OnionDirection::Next);
                        let c = Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], tint[3]);
                        painter.image(tex.id(), dst, uv, c);
                    }
                }
            }
        }
    }

    let outline_a = (state.bg_opacity * 180.0) as u8;
    if outline_a > 0 {
        painter.rect_stroke(
            dst,
            0.0,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(80, 80, 80, outline_a)),
        );
    }
}

/// Paint the active tool's cursor preview on top of the canvas.
/// Pencil/Ink/Eraser → outline circle sized by brush radius (in doc px → screen
/// px via current canvas scale). Eraser shown with a dashed inner ring.
/// Fill → crosshair + small filled dot at the click point.
fn draw_tool_cursor(state: &AppState, ui: &egui::Ui, canvas_rect: Rect, pos: egui::Pos2) {
    let painter = ui.painter_at(canvas_rect);
    let cw = state.project.width as f32;
    let ch = state.project.height as f32;
    let scale = (canvas_rect.width() / cw).min(canvas_rect.height() / ch);

    let white = Color32::from_rgba_unmultiplied(255, 255, 255, 220);
    let black = Color32::from_rgba_unmultiplied(0, 0, 0, 180);

    match state.tool {
        ActiveTool::Pencil | ActiveTool::Ink | ActiveTool::Eraser => {
            // Effective radius scales with pressure (mouse = 1.0 always).
            let pressure = state.pen.current_pressure().unwrap_or(1.0);
            let p_size = state.brush.pressure_size.clamp(0.0, 1.0);
            let pressure_mul = (1.0 - p_size) + p_size * pressure;
            let r_doc = (state.brush.radius * pressure_mul).max(0.5);
            let r = (r_doc * scale).max(2.0);

            // Double-ring (black outside, white inside) so cursor stays visible
            // on any background.
            painter.circle_stroke(pos, r + 0.6, Stroke::new(1.5, black));
            painter.circle_stroke(pos, r, Stroke::new(1.0, white));

            // Eraser: dashed-looking second inner ring at ~70% radius.
            if matches!(state.tool, ActiveTool::Eraser) && r > 6.0 {
                painter.circle_stroke(pos, r * 0.6, Stroke::new(1.0, black));
            }

            // 1-px centre dot to mark exact hot-spot.
            painter.circle_filled(pos, 1.2, white);
            painter.circle_stroke(pos, 1.2, Stroke::new(0.6, black));
        }
        ActiveTool::Fill => {
            // Crosshair + filled centre dot. No radius — fill is a point op.
            let arm = 9.0;
            painter.line_segment(
                [egui::pos2(pos.x - arm, pos.y), egui::pos2(pos.x + arm, pos.y)],
                Stroke::new(1.4, black),
            );
            painter.line_segment(
                [egui::pos2(pos.x, pos.y - arm), egui::pos2(pos.x, pos.y + arm)],
                Stroke::new(1.4, black),
            );
            painter.line_segment(
                [egui::pos2(pos.x - arm, pos.y), egui::pos2(pos.x + arm, pos.y)],
                Stroke::new(0.8, white),
            );
            painter.line_segment(
                [egui::pos2(pos.x, pos.y - arm), egui::pos2(pos.x, pos.y + arm)],
                Stroke::new(0.8, white),
            );
            // Tiny preview of brush colour.
            let c = state.brush.color;
            painter.circle_filled(
                pos,
                2.8,
                Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 255),
            );
            painter.circle_stroke(pos, 2.8, Stroke::new(0.8, black));
        }
    }
}

fn canvas_to_doc_mapping(
    state: &AppState,
    rect: Rect,
) -> impl Fn(egui::Pos2) -> (f32, f32) + Copy {
    let cw = state.project.width as f32;
    let ch = state.project.height as f32;
    let scale = (rect.width() / cw).min(rect.height() / ch);
    let dw = cw * scale;
    let dh = ch * scale;
    let origin = rect.center() - Vec2::new(dw, dh) * 0.5;
    move |pos: egui::Pos2| -> (f32, f32) {
        let lx = (pos.x - origin.x) / scale;
        let ly = (pos.y - origin.y) / scale;
        (lx, ly)
    }
}

/// Compact floating window holding File / Edit menus + window drag + window
/// controls. Replaces the old top title-bar so the canvas can extend edge-to-
/// edge while still giving the user a way to move/close the borderless window.
fn menu_window(state: &mut AppState, ctx: &egui::Context) {
    egui::Window::new(theme::icon_text(ic::SPARKLE, "Animator"))
        .default_pos([16.0, 16.0])
        .resizable(false)
        .collapsible(true)
        .title_bar(false)
        .frame(floating_frame())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                title_menu(state, ctx, ui);
                ui.separator();
                title_drag_area(ctx, ui);
                ui.separator();
                title_window_buttons(ctx, ui);
            });
        });
}

fn title_menu(state: &mut AppState, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.menu_button(theme::icon_text(ic::PENCIL_SIMPLE, "Edit"), |ui| {
        let can_undo = state.history.can_undo();
        let can_redo = state.history.can_redo();
        let undo_label = format!(
            "Undo    {}",
            state.shortcuts.get(Action::Undo).map(|c| c.display()).unwrap_or_default()
        );
        let redo_label = format!(
            "Redo    {}",
            state.shortcuts.get(Action::Redo).map(|c| c.display()).unwrap_or_default()
        );
        if ui
            .add_enabled(
                can_undo,
                egui::Button::new(theme::icon_text(ic::ARROW_COUNTER_CLOCKWISE, &undo_label)),
            )
            .clicked()
        {
            state.undo();
            ui.close_menu();
        }
        if ui
            .add_enabled(
                can_redo,
                egui::Button::new(theme::icon_text(ic::ARROW_CLOCKWISE, &redo_label)),
            )
            .clicked()
        {
            state.redo();
            ui.close_menu();
        }
        ui.separator();
        if ui
            .button(theme::icon_text(ic::GEAR, "Shortcuts…"))
            .clicked()
        {
            state.show_settings = true;
            ui.close_menu();
        }
    });
    ui.menu_button(theme::icon_text(ic::FOLDER, "File"), |ui| {
        if ui
            .button(theme::icon_text(ic::FLOPPY_DISK, "Save PNG (current frame)…"))
            .clicked()
        {
            let flat = composite::flatten_frame(&state.project, state.project.current_frame);
            if let Err(e) = png_save::save_dialog(&flat) {
                log::error!("Save failed: {e:#}");
            }
            ui.close_menu();
        }
        if ui
            .button(theme::icon_text(ic::IMAGES, "Export PNG sequence…"))
            .clicked()
        {
            if let Err(e) = png_seq::export_dialog(&state.project) {
                log::error!("PNG sequence export failed: {e:#}");
            }
            ui.close_menu();
        }
        if ui
            .button(theme::icon_text(ic::FILM_REEL, "Export animated GIF…"))
            .clicked()
        {
            if let Err(e) = gif_export::export_dialog(&state.project) {
                log::error!("GIF export failed: {e:#}");
            }
            ui.close_menu();
        }
        ui.separator();
        if ui
            .button(theme::icon_text(ic::ERASER, "Clear current cell"))
            .clicked()
        {
            if let Some(id) = state.project.resolved_current() {
                if let Some(c) = state.project.cell_mut(id) {
                    c.clear();
                }
                state.mark_dirty(id);
            }
            ui.close_menu();
        }
        ui.separator();
        if ui
            .button(theme::icon_text(ic::SIGN_OUT, "Quit"))
            .clicked()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    });
}

fn title_drag_area(ctx: &egui::Context, ui: &mut egui::Ui) {
    // Fixed-width grip handle: drag = move window, double-click = max toggle.
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(70.0, 22.0), Sense::click_and_drag());
    if resp.is_pointer_button_down_on() {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if resp.double_clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(
            !ctx.input(|i| i.viewport().maximized.unwrap_or(false)),
        ));
    }
    let painter = ui.painter_at(rect);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        ic::DOTS_SIX_VERTICAL,
        egui::FontId::proportional(15.0),
        theme::TEXT_MUTED,
    );
    if resp.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::Grab);
    }
}

fn title_window_buttons(ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if theme::icon_button(ui, ic::X, "Close").clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let max_icon = if is_max { ic::ARROWS_IN_SIMPLE } else { ic::ARROWS_OUT_SIMPLE };
        let max_tip = if is_max { "Restore" } else { "Maximize" };
        if theme::icon_button(ui, max_icon, max_tip).clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_max));
        }
        if theme::icon_button(ui, ic::MINUS, "Minimize").clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
    });
}
