//! Top-level egui layout: modern dark UI with Phosphor icons.
//! Floating windows, transparent canvas, layered + onion composite.

use egui::{Align, Color32, Frame, Margin, Rect, Sense, Stroke, Vec2};
use egui_phosphor::regular as ic;

use crate::app::{AppState, NavKind, PanelId, MP4_PRESETS};
use crate::input::shortcuts::{Action, KeyCombo};
use crate::io::{composite, gif_export, png_import, png_save, png_seq, project_file};
use crate::timeline::onion::OnionDirection;
use crate::tools::{ActiveTool, ShapeKind};
use crate::ui::theme;

/// Tooltip text including the currently-bound shortcut (e.g. "Pencil  (Q)").
fn tip(state: &AppState, action: Action, base: &str) -> String {
    match state.shortcuts.get(action) {
        Some(c) => format!("{base}  ({})", c.display()),
        None => base.to_string(),
    }
}

/// Bare shortcut text for an action (e.g. "Ctrl+S"), empty if unbound.
fn combo_text(state: &AppState, action: Action) -> String {
    state
        .shortcuts
        .get(action)
        .map(|c| c.display())
        .unwrap_or_default()
}

pub fn draw(state: &mut AppState, ctx: &egui::Context) {
    if state.show_panels {
        menu_window(state, ctx);
        // Left: tools + brush.  Right: layers / onion / x-sheet.  Bottom: timeline.
        panel_window(state, ctx, PanelId::Tools, [12.0, 48.0], 232.0, true);
        panel_window(state, ctx, PanelId::Brush, [12.0, 300.0], 232.0, true);
        panel_window(state, ctx, PanelId::Layers, [1004.0, 48.0], 252.0, true);
        panel_window(state, ctx, PanelId::Onion, [1004.0, 300.0], 252.0, false);
        panel_window(state, ctx, PanelId::Xsheet, [1004.0, 470.0], 252.0, false);
        panel_window(state, ctx, PanelId::Timeline, [320.0, 600.0], 640.0, true);
        settings_window(state, ctx);
    } else if state.show_mini_timeline {
        mini_timeline_window(state, ctx);
    }
    new_project_dialog(state, ctx);
    mp4_export_dialog(state, ctx);
    import_range_dialog(state, ctx);
    if let Some(label) = state.bg_label {
        busy_overlay(ctx, label);
    }

    egui::CentralPanel::default()
        .frame(Frame::none().fill(Color32::TRANSPARENT))
        .show(ctx, |ui| {
            let avail = ui.available_size();
            let (canvas_rect, resp) = ui.allocate_exact_size(avail, Sense::drag());
            paint_canvas(state, ui, canvas_rect);

            // While live screen-pick is active the canvas swallows all drawing
            // input — a tap commits a sampled colour instead (see
            // `screen_pick_live`). Skip the entire draw/nav interaction.
            if !state.screen_pick {
            let canvas_to_doc = canvas_to_doc_mapping(state, canvas_rect);

            // Canvas navigation gesture binds (modifier-only, configurable in
            // Settings). Held during a drag, they reinterpret it as
            // zoom/pan/rotate instead of drawing.
            let zoom_bind = state.shortcuts.get(Action::CanvasZoom);
            let pan_bind = state.shortcuts.get(Action::CanvasPan);
            let rotate_bind = state.shortcuts.get(Action::CanvasRotate);
            let (nav_gesture, mid_down) = ui.input(|i| {
                let g = if zoom_bind.is_some_and(|c| c.mods_held(i)) {
                    Some(NavKind::Zoom)
                } else if rotate_bind.is_some_and(|c| c.mods_held(i)) {
                    Some(NavKind::Rotate)
                } else if pan_bind.is_some_and(|c| c.mods_held(i)) {
                    Some(NavKind::Pan)
                } else {
                    None
                };
                (g, i.pointer.button_down(egui::PointerButton::Middle))
            });

            if resp.drag_started() {
                // Decide once, on press, what this drag does. Configurable
                // modifiers pick zoom/rotate/pan; middle-mouse always pans the
                // canvas; otherwise draw. When layer-transform mode is on, the
                // modifier gestures retarget the active layer instead of the
                // view.
                state.nav_drag = nav_gesture.or(if mid_down {
                    Some(NavKind::Pan)
                } else {
                    None
                });
                state.nav_to_layer = state.layer_xform && nav_gesture.is_some();
                if state.nav_to_layer {
                    state.begin_layer_xform();
                } else if state.nav_drag.is_none() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        if state.tool == ActiveTool::Tracker {
                            // Tracker takes the raw doc-space point — no cell
                            // mapping, no cell allocation.
                            state.tracker_click(canvas_to_doc(pos));
                        } else {
                            let (cx, cy) = doc_to_active_cell(state, canvas_to_doc(pos));
                            let t = ui.input(|i| i.time as f32);
                            let s = state.make_sample(cx, cy, t);
                            state.pointer_down(s);
                        }
                    }
                }
            }
            if resp.dragged() {
                if state.nav_to_layer {
                    // Apply the gesture to the active layer's transform.
                    match state.nav_drag {
                        Some(NavKind::Pan) => {
                            let xform = Xform::new(state, canvas_rect);
                            let d = resp.drag_delta();
                            let (s, c) = state.view.rotation.sin_cos();
                            let ddx = (d.x * c + d.y * s) / xform.scale;
                            let ddy = (-d.x * s + d.y * c) / xform.scale;
                            state.apply_layer_pan(ddx, ddy);
                        }
                        Some(NavKind::Rotate) => {
                            state.apply_layer_rotate(resp.drag_delta().x * 0.01);
                        }
                        Some(NavKind::Zoom) => {
                            let dy = resp.drag_delta().y;
                            if dy.abs() > 0.0 {
                                state.apply_layer_scale((-dy * 0.01).exp());
                            }
                        }
                        None => {}
                    }
                } else {
                    match state.nav_drag {
                        Some(NavKind::Pan) => {
                            state.view.pan += resp.drag_delta();
                        }
                        Some(NavKind::Rotate) => {
                            state.view.rotation += resp.drag_delta().x * 0.01;
                        }
                        Some(NavKind::Zoom) => {
                            let dy = resp.drag_delta().y;
                            if dy.abs() > 0.0 {
                                // Anchor zoom on the pointer: keep the doc point
                                // under the cursor fixed while the scale changes.
                                let cursor = resp
                                    .interact_pointer_pos()
                                    .unwrap_or_else(|| canvas_rect.center());
                                let before = Xform::new(state, canvas_rect).screen_to_doc(cursor);
                                let factor = (-dy * 0.01).exp();
                                state.view.zoom = (state.view.zoom * factor).clamp(0.05, 64.0);
                                let after =
                                    Xform::new(state, canvas_rect).doc_to_screen(before.0, before.1);
                                state.view.pan += cursor - after;
                            }
                        }
                        None => {
                            if let Some(pos) = resp.interact_pointer_pos() {
                                let (cx, cy) = doc_to_active_cell(state, canvas_to_doc(pos));
                                let t = ui.input(|i| i.time as f32);
                                let s = state.make_sample(cx, cy, t);
                                state.pointer_move(s);
                            }
                        }
                    }
                }
            }
            if resp.drag_stopped() {
                if state.nav_to_layer {
                    state.commit_layer_xform();
                } else if state.nav_drag.is_none() {
                    state.pointer_up();
                }
                state.nav_drag = None;
                state.nav_to_layer = false;
            }

            // Tool cursor preview — only while drawing (not during nav gestures),
            // when the pointer is over the canvas and not over a floating panel.
            if state.nav_drag.is_none() && (resp.hovered() || resp.dragged()) {
                let pos = resp.hover_pos().or_else(|| resp.interact_pointer_pos());
                if let Some(pos) = pos {
                    if canvas_rect.contains(pos) {
                        draw_tool_cursor(state, ui, canvas_rect, pos);
                        ctx.set_cursor_icon(egui::CursorIcon::None);
                    }
                }
            }
            } // end: if !state.screen_pick
        });

    // Live screen colour-pick: sample the pixel under the cursor each frame and
    // commit on tap. Drawn last so the loupe sits above the canvas.
    screen_pick_live(state, ctx);
}

/// Drive live screen-pick mode: read the pixel under the OS cursor (the desktop
/// behind our now-transparent backdrop), show a swatch/hex loupe at the cursor,
/// and commit the colour on the next tap. Escape cancels.
fn screen_pick_live(state: &mut AppState, ctx: &egui::Context) {
    if !state.screen_pick {
        return;
    }
    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
    ctx.request_repaint(); // keep sampling live

    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.end_screen_pick();
        return;
    }

    let (down, pressed) = ctx.input(|i| {
        (
            i.pointer.primary_down(),
            i.pointer.primary_pressed(),
        )
    });

    // Consume the press that opened the mode (e.g. clicking the toolbar button):
    // a tap only commits once the pointer has been released at least once.
    if state.screen_pick_arm && !down {
        state.screen_pick_arm = false;
    }

    // Don't sample/commit when the pointer is over a floating panel (toolbar
    // etc.) — only the see-through canvas reveals what's behind. NOTE: can't use
    // `is_pointer_over_area()` here: the canvas allocates its whole rect as a
    // widget, so that returns true over the canvas too. Check the layer instead
    // — the canvas is `Order::Background`, floating panels are `Order::Middle`.
    let over_ui = ctx
        .input(|i| i.pointer.interact_pos())
        .and_then(|p| ctx.layer_id_at(p))
        .map_or(false, |l| l.order != egui::Order::Background);

    // Live region capture around the OS cursor: the centre pixel is the colour,
    // the whole region is the zoom-loupe preview. HALF is in screen pixels.
    const HALF: i32 = 12; // 25x25 px sampled around the cursor
    let region = crate::input::screen_pixel::cursor_pos()
        .and_then(|(x, y)| crate::input::screen_pixel::capture_region(x, y, HALF));
    let color = region.as_ref().map(|r| r.center());

    // Refresh the loupe texture (kept on AppState so it outlives this frame).
    if let Some(r) = &region {
        let img = crate::app::premultiplied_image([r.w as usize, r.h as usize], &r.buf);
        if let Some(t) = &mut state.screen_pick_tex {
            t.set(img, egui::TextureOptions::NEAREST);
        } else {
            state.screen_pick_tex =
                Some(ctx.load_texture("screen_pick_loupe", img, egui::TextureOptions::NEAREST));
        }
    }

    if !over_ui {
        if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
            draw_screen_pick_loupe(ctx, pos, state.screen_pick_tex.as_ref(), color, HALF);
        }
    }
    screen_pick_banner(ctx);

    if let Some(col) = color {
        if !state.screen_pick_arm && pressed && !over_ui {
            state.commit_screen_pick(col);
        }
    }
}

/// Small top-centre hint shown while screen-pick mode is active.
fn screen_pick_banner(ctx: &egui::Context) {
    let scr = ctx.screen_rect();
    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("screen_pick_banner"));
    let painter = ctx.layer_painter(layer);
    let center = egui::pos2(scr.center().x, scr.min.y + 22.0);
    let text = "Pick colour — click to sample · Esc to cancel";
    let galley = painter.layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(13.0),
        Color32::WHITE,
    );
    let pad = Vec2::new(12.0, 6.0);
    let rect = Rect::from_center_size(center, galley.size() + pad * 2.0);
    painter.rect_filled(rect, 6.0, Color32::from_rgba_unmultiplied(10, 11, 14, 230));
    painter.rect_stroke(rect, 6.0, Stroke::new(1.0, Color32::from_gray(80)));
    painter.galley(rect.min + pad, galley, Color32::WHITE);
}

/// PowerToys-style zoom loupe: the captured region magnified, the centre pixel
/// outlined, plus a hex readout. Offset from the cursor so it never covers (or
/// gets captured into) the pixel being sampled.
fn draw_screen_pick_loupe(
    ctx: &egui::Context,
    cursor: egui::Pos2,
    tex: Option<&egui::TextureHandle>,
    color: Option<[u8; 4]>,
    half: i32,
) {
    let side = (half * 2 + 1) as f32;
    let size = 144.0_f32;
    let scr = ctx.screen_rect();

    // Place clear of the captured region so the loupe isn't grabbed into it.
    let mut min = cursor + Vec2::new(34.0, 34.0);
    if min.x + size > scr.max.x {
        min.x = cursor.x - 34.0 - size;
    }
    if min.y + size + 28.0 > scr.max.y {
        min.y = cursor.y - 34.0 - size - 28.0;
    }
    let rect = Rect::from_min_size(min, Vec2::splat(size));

    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("screen_pick_loupe"));
    let painter = ctx.layer_painter(layer);

    painter.rect_filled(rect, 6.0, Color32::from_rgba_unmultiplied(10, 11, 14, 235));
    if let Some(t) = tex {
        painter.image(
            t.id(),
            rect,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    // Outline the centre pixel (the one being sampled).
    let cell = size / side;
    let cpix = Rect::from_center_size(rect.center(), Vec2::splat(cell));
    painter.rect_stroke(cpix, 0.0, Stroke::new(1.5, Color32::BLACK));
    painter.rect_stroke(cpix, 0.0, Stroke::new(0.5, Color32::WHITE));
    painter.rect_stroke(rect, 6.0, Stroke::new(1.0, Color32::from_gray(90)));

    // Hex readout + swatch below the loupe.
    let (swatch, label) = match color {
        Some(c) => (
            Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 255),
            format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]),
        ),
        None => (Color32::from_gray(40), "—".to_string()),
    };
    let bar = Rect::from_min_size(egui::pos2(rect.min.x, rect.max.y + 4.0), Vec2::new(size, 22.0));
    painter.rect_filled(bar, 4.0, Color32::from_rgba_unmultiplied(10, 11, 14, 235));
    let sw = Rect::from_min_size(bar.min + Vec2::new(5.0, 4.0), Vec2::splat(14.0));
    painter.rect_filled(sw, 2.0, swatch);
    painter.rect_stroke(sw, 2.0, Stroke::new(1.0, Color32::from_gray(90)));
    painter.text(
        bar.min + Vec2::new(26.0, 3.0),
        egui::Align2::LEFT_TOP,
        label,
        egui::FontId::monospace(13.0),
        Color32::WHITE,
    );
}

/// Icon + title for a panel.
fn panel_meta(id: PanelId) -> (&'static str, &'static str) {
    match id {
        PanelId::Tools => (ic::TOOLBOX, "Tools"),
        PanelId::Brush => (ic::PAINT_BRUSH, "Brush"),
        PanelId::Layers => (ic::STACK, "Layers"),
        PanelId::Onion => (ic::CIRCLES_THREE, "Onion"),
        PanelId::Xsheet => (ic::TABLE, "X-sheet"),
        PanelId::Timeline => (ic::FILM_STRIP, "Timeline"),
    }
}

/// Draw a panel's body.
fn panel_content(state: &mut AppState, ctx: &egui::Context, ui: &mut egui::Ui, id: PanelId) {
    match id {
        PanelId::Tools => tools_content(state, ui),
        PanelId::Brush => brush_content(state, ui),
        PanelId::Layers => layers_content(state, ui),
        PanelId::Onion => onion_content(state, ui),
        PanelId::Xsheet => xsheet_content(state, ui),
        PanelId::Timeline => timeline_content(state, ctx, ui),
    }
}

/// Render a panel as a draggable floating window.
fn panel_window(
    state: &mut AppState,
    ctx: &egui::Context,
    id: PanelId,
    default_pos: [f32; 2],
    width: f32,
    open: bool,
) {
    let (icon, title) = panel_meta(id);
    egui::Window::new(theme::icon_text(icon, title))
        .default_pos(default_pos)
        .default_width(width)
        .default_open(open)
        .resizable(true)
        .collapsible(true)
        .frame(floating_frame())
        .show(ctx, |ui| panel_content(state, ctx, ui, id));
}

fn tools_content(state: &mut AppState, ui: &mut egui::Ui) {
    {
            // Tool palette — two rows of three so each icon stays large/tappable.
            ui.horizontal(|ui| {
                let p = tip(state, Action::ToolPencil, "Pencil");
                let i = tip(state, Action::ToolInk, "Ink");
                let e = tip(state, Action::ToolEraser, "Eraser");
                tool_toggle(ui, state, ActiveTool::Pencil, ic::PENCIL, &p);
                tool_toggle(ui, state, ActiveTool::Ink, ic::PEN_NIB, &i);
                tool_toggle(ui, state, ActiveTool::Eraser, ic::ERASER, &e);
                ui.add_space(6.0);
                let f = tip(state, Action::ToolFill, "Fill");
                let c = tip(state, Action::ToolColorPicker, "Color picker");
                let g = tip(state, Action::ToolShape, "Shape");
                tool_toggle(ui, state, ActiveTool::Fill, ic::PAINT_BUCKET, &f);
                tool_toggle(ui, state, ActiveTool::ColorPicker, ic::EYEDROPPER, &c);
                tool_toggle(ui, state, ActiveTool::Shape, ic::SHAPES, &g);
                let tr = tip(state, Action::ToolTracker, "Tracker (stabilize)");
                tool_toggle(ui, state, ActiveTool::Tracker, ic::CROSSHAIR, &tr);
                ui.add_space(6.0);
                // Screen colour pick — an action, not a persistent tool: opens a
                // fullscreen snapshot overlay to sample any pixel on the desktop.
                let sp = tip(state, Action::PickScreenColor, "Pick color from screen");
                if theme::icon_toggle(ui, ic::EYEDROPPER_SAMPLE, &sp, false).clicked() {
                    state.dispatch(Action::PickScreenColor);
                }
            });
            ui.add_space(6.0);
            theme::section_header(ui, ic::SLIDERS, tool_name(state.tool));

            // Tool-specific options only — keeps this panel about tools alone.
            if state.tool == ActiveTool::Fill {
                ui.add(
                    egui::Slider::new(&mut state.brush.fill_tolerance, 0..=128).text("Tolerance"),
                );
            } else if state.tool == ActiveTool::Shape {
                ui.horizontal(|ui| {
                    shape_kind_toggle(ui, state, ShapeKind::Line, ic::LINE_SEGMENT, "Line");
                    shape_kind_toggle(ui, state, ShapeKind::Rect, ic::RECTANGLE, "Rectangle");
                    shape_kind_toggle(ui, state, ShapeKind::Ellipse, ic::CIRCLE, "Ellipse");
                });
                ui.add(egui::Slider::new(&mut state.brush.radius, 0.5..=64.0).text("Thickness"));
            } else if state.tool == ActiveTool::Tracker {
                ui.label(
                    egui::RichText::new(
                        "Click the same feature on each frame — the view advances a frame per point. Re-click a frame to fix a miss.",
                    )
                    .color(theme::TEXT_MUTED)
                    .size(11.0),
                );
                let two_before = state.tracker_two_points;
                ui.checkbox(
                    &mut state.tracker_two_points,
                    "Second point (fix rotation/zoom)",
                )
                .on_hover_text(
                    "Each frame takes two clicks: point A, then point B on another feature. \
                     Stabilize then corrects rotation and zoom shake too.",
                );
                if two_before != state.tracker_two_points {
                    state.tracker_pending_b = None;
                }
                if state.tracker_pending_b == Some(state.project.current_frame) {
                    ui.label(
                        egui::RichText::new("Now click point B…")
                            .color(theme::ACCENT)
                            .size(11.0),
                    );
                }
                let count = state.tracked_point_count();
                ui.label(
                    egui::RichText::new(format!("Tracked frames: {count}"))
                        .color(theme::TEXT_MUTED)
                        .size(11.0),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(count >= 2, egui::Button::new("Stabilize"))
                        .on_hover_text(
                            "Offset this layer per frame so the tracked point stays still",
                        )
                        .clicked()
                    {
                        state.stabilize_active_layer();
                    }
                    if ui
                        .add_enabled(count > 0, egui::Button::new("Clear points"))
                        .clicked()
                    {
                        state.clear_track_points();
                    }
                });
            } else if state.tool == ActiveTool::ColorPicker {
                ui.label(
                    egui::RichText::new("Click the canvas to sample a colour.")
                        .color(theme::TEXT_MUTED)
                        .size(11.0),
                );
                let hint = tip(
                    state,
                    Action::PickScreenColor,
                    "Sample any pixel behind the window (drops backdrop)",
                );
                ui.label(egui::RichText::new(hint).color(theme::TEXT_MUTED).size(11.0));
            } else {
                ui.add(egui::Slider::new(&mut state.brush.radius, 0.5..=128.0).text("Size"));
                ui.add(egui::Slider::new(&mut state.brush.opacity, 0.0..=1.0).text("Flow"));
            }
    }
}

fn brush_content(state: &mut AppState, ui: &mut egui::Ui) {
    {
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
            ui.horizontal(|ui| {
                let rgb = state.brush.color;
                ui.label(format!("rgb({}, {}, {})", rgb[0], rgb[1], rgb[2]));
                if ui.button("Paste").clicked() {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        if let Ok(s) = cb.get_text() {
                            if let Some([r, g, b, _]) = parse_rgb(&s) {
                                state.brush.color = [r, g, b, 255];
                            }
                        }
                    }
                }
            });

            ui.add_space(6.0);
            theme::section_header(ui, ic::SLIDERS, "Dynamics");
            ui.add(egui::Slider::new(&mut state.brush.hardness, 0.0..=1.0).text("Hardness"));
            ui.add(egui::Slider::new(&mut state.brush.grain, 0.0..=1.0).text("Grain"));
            ui.add(
                egui::Slider::new(&mut state.brush.pressure_size, 0.0..=1.0).text("Pres → size"),
            );
            ui.add(
                egui::Slider::new(&mut state.brush.pressure_opacity, 0.0..=1.0).text("Pres → flow"),
            );

            // Canvas backdrop + input status — collapsed by default so the panel
            // stays compact, expandable when needed.
            ui.add_space(6.0);
            egui::CollapsingHeader::new(theme::icon_text(ic::IMAGE, "Backdrop"))
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(&mut state.bg_opacity, 0.0..=1.0).text("Opacity"));
                        ui.color_edit_button_rgb(&mut state.bg_color);
                    });
                    ui.checkbox(&mut state.show_checker, "Checker backdrop");
                    ui.horizontal(|ui| {
                        let r = (state.bg_color[0] * 255.0).round() as u8;
                        let g = (state.bg_color[1] * 255.0).round() as u8;
                        let b = (state.bg_color[2] * 255.0).round() as u8;
                        ui.label(
                            egui::RichText::new(format!("rgb({r}, {g}, {b})"))
                                .color(theme::TEXT_MUTED),
                        );
                        if ui.button("Paste").clicked() {
                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                if let Ok(s) = cb.get_text() {
                                    if let Some([r, g, b, _]) = parse_rgb(&s) {
                                        state.bg_color = [
                                            r as f32 / 255.0,
                                            g as f32 / 255.0,
                                            b as f32 / 255.0,
                                        ];
                                    }
                                }
                            }
                        }
                    });
                });

            ui.add_space(2.0);
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
    }
}

fn timeline_content(state: &mut AppState, ctx: &egui::Context, ui: &mut egui::Ui) {
    {
            ui.horizontal(|ui| {
                let play_icon = if state.playback.playing {
                    ic::PAUSE
                } else {
                    ic::PLAY
                };
                let play_base = if state.playback.playing {
                    "Pause"
                } else {
                    "Play"
                };
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
                    state.structural_edit(false, |p| p.add_frame());
                }
                let dup_tip = tip(state, Action::FrameDuplicate, "Duplicate frame");
                if theme::icon_button(ui, ic::COPY, &dup_tip).clicked() {
                    state.structural_edit(false, |p| p.duplicate_frame());
                }
                let del_tip = tip(state, Action::FrameDelete, "Delete frame");
                if theme::icon_button(ui, ic::TRASH, &del_tip).clicked() {
                    let wipes_pixels = state.project.frame_count <= 1;
                    state.structural_edit(wipes_pixels, |p| p.delete_frame());
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
                ui.label(egui::RichText::new(format!("/ {n}")).color(theme::TEXT_MUTED));
            });

            frame_strip(state, ui);
    }
}

/// Compact playback HUD shown when the floating panels are hidden (Tab).
/// Pinned bottom-centre: play/pause, step, frame counter, scrub strip.
fn mini_timeline_window(state: &mut AppState, ctx: &egui::Context) {
    egui::Window::new("mini_timeline")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -12.0))
        .default_width(360.0)
        .frame(floating_frame())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let play_icon = if state.playback.playing {
                    ic::PAUSE
                } else {
                    ic::PLAY
                };
                let play_base = if state.playback.playing { "Pause" } else { "Play" };
                let play_tip = tip(state, Action::PlayPause, play_base);
                if theme::icon_button(ui, play_icon, &play_tip).clicked() {
                    let now = ctx.input(|i| i.time);
                    state.playback.toggle(now);
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
                let n = state.project.frame_count.max(1);
                ui.label(
                    egui::RichText::new(format!("{} / {}", state.project.current_frame + 1, n))
                        .monospace()
                        .color(theme::TEXT),
                );
                ui.separator();
                // Active tool icon + brush colour swatch.
                ui.label(
                    egui::RichText::new(tool_icon(state.tool))
                        .size(15.0)
                        .color(theme::TEXT),
                )
                .on_hover_text(tool_name(state.tool));
                let c = state.brush.color;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(13.0, 13.0), Sense::hover());
                ui.painter().rect_filled(
                    rect,
                    3.0,
                    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 255),
                );
                ui.painter()
                    .rect_stroke(rect, 3.0, Stroke::new(1.0, theme::STROKE_THIN));
            });
            ui.add_space(3.0);
            mini_frame_dots(state, ui);
        });
}

/// Phosphor icon for a tool — shared by the mini bar.
fn tool_icon(tool: ActiveTool) -> &'static str {
    match tool {
        ActiveTool::Pencil => ic::PENCIL,
        ActiveTool::Ink => ic::PEN_NIB,
        ActiveTool::Eraser => ic::ERASER,
        ActiveTool::Fill => ic::PAINT_BUCKET,
        ActiveTool::ColorPicker => ic::EYEDROPPER,
        ActiveTool::Shape => ic::SHAPES,
        ActiveTool::Tracker => ic::CROSSHAIR,
    }
}

fn tool_name(tool: ActiveTool) -> &'static str {
    match tool {
        ActiveTool::Pencil => "Pencil",
        ActiveTool::Ink => "Ink",
        ActiveTool::Eraser => "Eraser",
        ActiveTool::Fill => "Fill",
        ActiveTool::ColorPicker => "Color picker",
        ActiveTool::Shape => "Shape",
        ActiveTool::Tracker => "Tracker",
    }
}

/// Minimal frame indicator for the mini bar: one fixed-size dot per frame,
/// active filled. Horizontally scrollable so dots stay distinguishable on long
/// timelines; auto-scrolls to keep the active frame in view. Click / drag to
/// scrub. (Unhide the full Timeline panel for detail.)
fn mini_frame_dots(state: &mut AppState, ui: &mut egui::Ui) {
    let n = state.project.frame_count.max(1);
    let cur = state.project.current_frame;
    let dot_step = 14.0;
    let height = 16.0;
    let view_w = 320.0_f32;

    egui::ScrollArea::horizontal()
        .max_width(view_w)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let total_w = (n as f32 * dot_step).max(view_w);
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(total_w, height), Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            let cy = rect.center().y;

            for i in 0..n {
                let center = egui::pos2(rect.min.x + (i as f32 + 0.5) * dot_step, cy);
                if i == cur {
                    painter.circle_filled(center, 4.0, theme::ACCENT);
                    painter.circle_stroke(center, 5.5, Stroke::new(1.0, theme::ACCENT));
                } else {
                    let in_loop = i >= state.project.loop_start && i < state.project.loop_end;
                    let col = if in_loop { theme::TEXT_MUTED } else { theme::BG_HOVER };
                    painter.circle_filled(center, 2.6, col);
                }
            }

            // Auto-scroll to the active dot only when the frame changes, so the
            // user can still scroll freely the rest of the time.
            let mem_id = ui.id().with("mini_dots_frame");
            let last: Option<usize> = ui.data(|d| d.get_temp(mem_id));
            if last != Some(cur) {
                let active = egui::Rect::from_center_size(
                    egui::pos2(rect.min.x + (cur as f32 + 0.5) * dot_step, cy),
                    egui::vec2(dot_step * 3.0, height),
                );
                ui.scroll_to_rect(active, Some(Align::Center));
                ui.data_mut(|d| d.insert_temp(mem_id, cur));
            }

            if resp.dragged() || resp.clicked() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let rel = ((pos.x - rect.min.x) / dot_step).floor() as isize;
                    let idx = rel.clamp(0, n as isize - 1) as usize;
                    state.project.goto(idx);
                }
            }
        });
}

fn frame_strip(state: &mut AppState, ui: &mut egui::Ui) {
    let n = state.project.frame_count.max(1);
    let cur = state.project.current_frame;
    let height = 26.0;
    // Fill the panel while frames fit; clamp so cells never squeeze below a
    // readable width — the strip scrolls instead.
    let cell_w = (ui.available_width() / n as f32).max(22.0);

    egui::ScrollArea::horizontal()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let (rect, resp) = ui.allocate_exact_size(
                egui::vec2(n as f32 * cell_w, height),
                Sense::click_and_drag(),
            );
            let painter = ui.painter_at(rect);

            painter.rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(10, 11, 14, 220));
            for i in 0..n {
                let x = rect.min.x + i as f32 * cell_w;
                let r = Rect::from_min_size(egui::pos2(x, rect.min.y), egui::vec2(cell_w, height));
                let fill = if i == cur {
                    theme::ACCENT
                } else if i >= state.project.loop_start && i < state.project.loop_end {
                    theme::BG_HOVER
                } else {
                    theme::BG_INACTIVE
                };
                painter.rect_filled(r.shrink(1.5), 3.0, fill);
                if cell_w > 18.0 {
                    let txt_color = if i == cur {
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

            // Keep the active frame centered, but only when it changes so the
            // user can still scroll the strip freely, and never while
            // drag-scrubbing (recentering would shift the content under the
            // pointer and make the drag jump).
            let mem_id = ui.id().with("frame_strip_frame");
            let last: Option<usize> = ui.data(|d| d.get_temp(mem_id));
            if last != Some(cur) {
                if !resp.dragged() {
                    let active = Rect::from_center_size(
                        egui::pos2(rect.min.x + (cur as f32 + 0.5) * cell_w, rect.center().y),
                        egui::vec2(cell_w * 3.0, height),
                    );
                    ui.scroll_to_rect(active, Some(egui::Align::Center));
                }
                ui.data_mut(|d| d.insert_temp(mem_id, cur));
            }

            if resp.dragged() || resp.clicked() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let rel = ((pos.x - rect.min.x) / cell_w).floor() as isize;
                    let idx = rel.clamp(0, n as isize - 1) as usize;
                    state.project.goto(idx);
                }
            }
        });
}

fn onion_content(state: &mut AppState, ui: &mut egui::Ui) {
    {
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
    }
}

fn layers_content(state: &mut AppState, ui: &mut egui::Ui) {
    {
            ui.horizontal(|ui| {
                if theme::icon_button(ui, ic::PLUS, "Add layer").clicked() {
                    state.structural_edit(false, |p| p.add_layer());
                }
                if theme::icon_button(ui, ic::MINUS, "Delete layer").clicked() {
                    state.structural_edit(false, |p| p.delete_layer());
                }
                if theme::icon_button(ui, ic::ARROW_UP, "Move layer up").clicked() {
                    state.project.move_layer_up();
                }
                if theme::icon_button(ui, ic::ARROW_DOWN, "Move layer down").clicked() {
                    state.project.move_layer_down();
                }
                ui.add_enabled_ui(state.can_merge_down(), |ui| {
                    if theme::icon_button(ui, ic::ARROWS_MERGE, "Merge layer down").clicked() {
                        state.merge_layer_down();
                    }
                });
            });
            ui.add_space(4.0);
            ui.separator();

            let n = state.project.layers.len();
            let cur = state.project.current_layer;
            let mut select: Option<usize> = None;
            let mut start_rename: Option<usize> = None;
            let mut rename_commit = false;
            let mut rename_cancel = false;
            // Split borrow: rows need &mut layer while the rename edit buffer
            // lives on AppState next to it.
            let (layers, rename) = (&mut state.project.layers, &mut state.layer_rename);
            for i in (0..n).rev() {
                let layer = &mut layers[i];
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
                            let eye_icon = if layer.visible {
                                ic::EYE
                            } else {
                                ic::EYE_SLASH
                            };
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
                                        egui::RichText::new(ic::LIGHTBULB)
                                            .size(16.0)
                                            .color(ref_color),
                                    )
                                    .min_size(egui::vec2(30.0, 24.0)),
                                )
                                .on_hover_text("Light-table reference layer")
                                .clicked()
                            {
                                layer.reference = !layer.reference;
                            }
                            // Name: click selects, double-click renames inline.
                            let editing = matches!(rename.as_ref(), Some(r) if r.index == i);
                            if editing {
                                let r = rename.as_mut().unwrap();
                                let te = ui.add(
                                    egui::TextEdit::singleline(&mut r.buf).desired_width(110.0),
                                );
                                if !r.focused {
                                    te.request_focus();
                                    r.focused = true;
                                }
                                if ui.input(|inp| inp.key_pressed(egui::Key::Escape)) {
                                    rename_cancel = true;
                                } else if te.lost_focus() {
                                    // Covers Enter and clicking away.
                                    rename_commit = true;
                                }
                            } else {
                                let resp = ui
                                    .add(egui::SelectableLabel::new(
                                        selected,
                                        egui::RichText::new(&layer.name).strong(),
                                    ))
                                    .on_hover_text("Double-click to rename");
                                if resp.double_clicked() {
                                    start_rename = Some(i);
                                } else if resp.clicked() {
                                    select = Some(i);
                                }
                            }
                        });
                        ui.add(egui::Slider::new(&mut layer.opacity, 0.0..=1.0).text("opacity"));
                    });
                ui.add_space(2.0);
            }
            if rename_cancel {
                state.layer_rename = None;
            } else if rename_commit {
                if let Some(r) = state.layer_rename.take() {
                    let name = r.buf.trim().to_string();
                    if !name.is_empty()
                        && r.index < state.project.layers.len()
                        && state.project.layers[r.index].name != name
                    {
                        state.structural_edit(false, |p| p.layers[r.index].name = name);
                    }
                }
            } else if let Some(i) = start_rename {
                state.layer_rename = Some(crate::app::LayerRename {
                    index: i,
                    buf: state.project.layers[i].name.clone(),
                    focused: false,
                });
            }
            if let Some(i) = select {
                state.project.current_layer = i;
            }

            // --- Layer transform ---
            ui.add_space(6.0);
            ui.separator();
            theme::section_header(ui, ic::RECTANGLE, "Transform");

            ui.checkbox(&mut state.layer_xform, "Transform mode");
            let toggle = combo_text(state, Action::LayerTransformToggle);
            ui.label(
                egui::RichText::new(format!(
                    "Toggle ({toggle}), then drag with your canvas gesture keys: zoom-key = scale, pan-key = move, rotate-key = rotate the active layer.",
                ))
                .color(theme::TEXT_MUTED)
                .size(10.5),
            );
            ui.add_space(4.0);

            let cf = state.project.current_frame;
            let li = state.project.current_layer;
            if let Some(l) = state.project.layers.get_mut(li) {
                ui.horizontal(|ui| {
                    ui.label("X");
                    ui.add(egui::DragValue::new(&mut l.transform.tx).speed(1.0));
                    ui.label("Y");
                    ui.add(egui::DragValue::new(&mut l.transform.ty).speed(1.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Scale");
                    ui.add(
                        egui::DragValue::new(&mut l.transform.scale)
                            .speed(0.01)
                            .range(0.01..=100.0),
                    );
                    ui.label("Rot°");
                    let mut deg = l.transform.rot.to_degrees();
                    if ui.add(egui::DragValue::new(&mut deg).speed(0.5)).changed() {
                        l.transform.rot = deg.to_radians();
                    }
                });
                let nkeys = l.transform_keys.len();
                let here = l.has_transform_key(cf);
                let status = if nkeys == 0 {
                    "no keys (static)".to_string()
                } else {
                    format!(
                        "{nkeys} key(s){}",
                        if here { " — keyed on this frame" } else { "" }
                    )
                };
                ui.label(
                    egui::RichText::new(status)
                        .color(theme::TEXT_MUTED)
                        .size(10.5),
                );
            }

            ui.horizontal(|ui| {
                let add = combo_text(state, Action::TransformKeyAdd);
                if ui
                    .button(theme::icon_text(ic::PLUS_SQUARE, &format!("Add key ({add})")))
                    .clicked()
                {
                    state.add_transform_key();
                }
                if ui.button(theme::icon_text(ic::X, "Del key")).clicked() {
                    state.delete_transform_key();
                }
                if ui
                    .button(theme::icon_text(ic::ARROW_COUNTER_CLOCKWISE, "Reset"))
                    .clicked()
                {
                    state.reset_active_layer_transform();
                }
            });
    }
}

fn xsheet_content(state: &mut AppState, ui: &mut egui::Ui) {
    {
            ui.horizontal(|ui| {
                if theme::icon_button(ui, ic::PLUS_SQUARE, "Insert blank key").clicked() {
                    state.structural_edit(false, |p| {
                        p.insert_blank_key_here();
                    });
                }
                if theme::icon_button(ui, ic::COPY, "Insert duplicate key").clicked() {
                    state.structural_edit(false, |p| {
                        p.insert_duplicate_key_here();
                    });
                }
                if theme::icon_button(ui, ic::PUSH_PIN, "Hold (delete key)").clicked() {
                    state.structural_edit(false, |p| p.hold_here());
                }
            });
            ui.add_space(4.0);
            ui.separator();

            let layer_count = state.project.layers.len();
            let frame_count = state.project.frame_count;

            egui::ScrollArea::both()
                .auto_shrink([false, true])
                .max_height(260.0)
                .show(ui, |ui| {
                    egui::Grid::new("xsheet_grid")
                        .striped(true)
                        .min_col_width(28.0)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new("Fr").color(theme::TEXT_MUTED).strong());
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
                                let fr_resp = ui.button(lbl);
                                if fr_resp.clicked() {
                                    state.project.goto(f);
                                }
                                // Scroll the active frame into view, but only when
                                // the frame changes — otherwise the user can't
                                // scroll the sheet freely.
                                if active_f {
                                    let mem_id = egui::Id::new("xsheet_active_frame");
                                    let last: Option<usize> = ui.data(|d| d.get_temp(mem_id));
                                    if last != Some(f) {
                                        ui.scroll_to_rect(fr_resp.rect, Some(Align::Center));
                                        ui.data_mut(|d| d.insert_temp(mem_id, f));
                                    }
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
    }
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
                    egui::RichText::new(
                        "Click a binding, then press a new key combo. Esc = cancel.",
                    )
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
                    .button(theme::icon_text(
                        ic::ARROW_COUNTER_CLOCKWISE,
                        "Reset to defaults",
                    ))
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
                                // Canvas nav gestures are modifier-only; show a
                                // modifier picker instead of press-to-bind (egui
                                // has no Key for a bare Ctrl/Shift/Alt).
                                if matches!(
                                    action,
                                    Action::CanvasZoom
                                        | Action::CanvasPan
                                        | Action::CanvasRotate
                                ) {
                                    ui.label(action.label());
                                    let cur = state.shortcuts.get(action);
                                    let cur_label = match cur {
                                        Some(c) if c.ctrl => "Ctrl",
                                        Some(c) if c.shift => "Shift",
                                        Some(c) if c.alt => "Alt",
                                        _ => "(none)",
                                    };
                                    egui::ComboBox::from_id_salt(("navmod", action))
                                        .selected_text(cur_label)
                                        .width(140.0)
                                        .show_ui(ui, |ui| {
                                            for (lbl, combo) in [
                                                (
                                                    "Ctrl",
                                                    Some(KeyCombo::modifier_only(
                                                        true, false, false,
                                                    )),
                                                ),
                                                (
                                                    "Shift",
                                                    Some(KeyCombo::modifier_only(
                                                        false, true, false,
                                                    )),
                                                ),
                                                (
                                                    "Alt",
                                                    Some(KeyCombo::modifier_only(
                                                        false, false, true,
                                                    )),
                                                ),
                                                ("(none)", None),
                                            ] {
                                                if ui
                                                    .selectable_label(cur_label == lbl, lbl)
                                                    .clicked()
                                                {
                                                    match combo {
                                                        Some(c) => {
                                                            state.shortcuts.set(action, c)
                                                        }
                                                        None => {
                                                            state
                                                                .shortcuts
                                                                .bindings
                                                                .remove(&action);
                                                        }
                                                    }
                                                    crate::input::shortcuts::save(
                                                        &state.shortcuts,
                                                    );
                                                }
                                            }
                                        });
                                    ui.label("");
                                    ui.end_row();
                                    continue;
                                }
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
        let mut rgb = [
            c[0] as f32 / 255.0,
            c[1] as f32 / 255.0,
            c[2] as f32 / 255.0,
        ];
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
        .inner_margin(Margin::same(10.0))
        .rounding(egui::Rounding::same(10.0))
        .shadow(egui::Shadow {
            offset: egui::vec2(0.0, 8.0),
            blur: 28.0,
            spread: 0.0,
            color: Color32::from_black_alpha(120),
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
        if target == ActiveTool::ColorPicker {
            state.prev_tool = Some(state.tool);
        }
        state.tool_brushes[state.tool.idx()] = state.brush.clone();
        state.tool = target;
        state.brush = state.tool_brushes[target.idx()].clone();
    }
}

fn shape_kind_toggle(
    ui: &mut egui::Ui,
    state: &mut AppState,
    kind: ShapeKind,
    icon: &str,
    label: &str,
) {
    let selected = state.brush.shape_kind == kind;
    if theme::icon_toggle(ui, icon, label, selected).clicked() {
        state.brush.shape_kind = kind;
    }
}

fn paint_canvas(state: &AppState, ui: &mut egui::Ui, rect: Rect) {
    let painter = ui.painter_at(rect);

    let xf = Xform::new(state, rect);
    let scale = xf.scale;
    let corners = xf.corners();

    // Checker backdrop. Follows zoom/pan; skipped while rotated (the rotated
    // doc quad isn't axis-aligned and a flat fallback would be misleading).
    // Also skipped during screen-pick so the desktop behind stays visible.
    if state.show_checker && state.view.rotation.abs() < 1e-3 && !state.screen_pick {
        let dst = Rect::from_two_pos(corners[0], corners[2]);
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

    let cur_frame = state.project.current_frame;
    let cur_layer = state.project.current_layer;
    let (pw, ph) = (xf.cw, xf.ch);

    // Corners for the resolved cell of `layer_idx` at the current frame, or None
    // if the cell is missing.
    let cell_corners = |layer_idx: usize, id: usize| -> Option<[egui::Pos2; 4]> {
        let cell = state.project.cell(id)?;
        Some(layer_screen_corners(
            &xf,
            state.display_transform(layer_idx, cur_frame),
            cell.width as f32,
            cell.height as f32,
            pw,
            ph,
        ))
    };

    // Onion ghosts of the active layer at nearby frames. Drawn inside the layer
    // loop so they sit at the active layer's depth (prev just behind its cell,
    // next just in front) instead of behind/above the whole stack.
    let draw_onion_prev = || {
        if !state.onion.enabled {
            return;
        }
        let Some(layer) = state.project.layers.get(cur_layer) else {
            return;
        };
        for k in (1..=state.onion.prev).rev() {
            let f_i = cur_frame as isize - k as isize;
            if f_i < 0 {
                continue;
            }
            let f = f_i as usize;
            if let Some(id) = layer.resolve(f) {
                if let (Some(tex), Some(cell)) =
                    (state.cell_textures.get(&id), state.project.cell(id))
                {
                    let tint = state.onion.tint_for(k, OnionDirection::Prev);
                    let c = Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], tint[3]);
                    let t = state.display_transform(cur_layer, f);
                    let lc =
                        layer_screen_corners(&xf, t, cell.width as f32, cell.height as f32, pw, ph);
                    image_quad(&painter, tex.id(), lc, c);
                }
            }
        }
    };
    let draw_onion_next = || {
        if !state.onion.enabled {
            return;
        }
        let Some(layer) = state.project.layers.get(cur_layer) else {
            return;
        };
        for k in 1..=state.onion.next {
            let f = cur_frame + k as usize;
            if f >= state.project.frame_count {
                continue;
            }
            if let Some(id) = layer.resolve(f) {
                if let (Some(tex), Some(cell)) =
                    (state.cell_textures.get(&id), state.project.cell(id))
                {
                    let tint = state.onion.tint_for(k, OnionDirection::Next);
                    let c = Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], tint[3]);
                    let t = state.display_transform(cur_layer, f);
                    let lc =
                        layer_screen_corners(&xf, t, cell.width as f32, cell.height as f32, pw, ph);
                    image_quad(&painter, tex.id(), lc, c);
                }
            }
        }
    };

    for (li, layer) in state.project.layers.iter().enumerate() {
        if !layer.visible || !layer.reference {
            continue;
        }
        if let Some(id) = layer.resolve(cur_frame) {
            if let (Some(tex), Some(lc)) = (state.cell_textures.get(&id), cell_corners(li, id)) {
                let dim = (layer.opacity * 0.45).clamp(0.0, 1.0);
                let a = (dim * 255.0) as u8;
                image_quad(
                    &painter,
                    tex.id(),
                    lc,
                    Color32::from_rgba_unmultiplied(255, 255, 255, a),
                );
            }
        }
    }

    for (li, layer) in state.project.layers.iter().enumerate() {
        if !layer.visible || layer.reference {
            continue;
        }
        // Onion ghosts render at the active layer's depth: previous frames just
        // behind its current cell, next frames just in front.
        let active = li == cur_layer;
        if active {
            draw_onion_prev();
        }
        if let Some(id) = layer.resolve(cur_frame) {
            if let (Some(tex), Some(lc)) = (state.cell_textures.get(&id), cell_corners(li, id)) {
                let a = (layer.opacity.clamp(0.0, 1.0) * 255.0) as u8;
                image_quad(
                    &painter,
                    tex.id(),
                    lc,
                    Color32::from_rgba_unmultiplied(255, 255, 255, a),
                );
            }
        }
        if active {
            draw_onion_next();
        }
    }

    // Tracker markers: the active layer's points for the current frame, plus a
    // dimmed ghost of the previous frame's point so the user can re-click the
    // same feature. Doc-space points map straight through the view transform.
    if state.tool == crate::tools::ActiveTool::Tracker {
        if let Some(layer) = state.project.layers.get(cur_layer) {
            let a_col = theme::ACCENT;
            let b_col = Color32::from_rgb(255, 170, 60);
            let draw_marker = |p: [f32; 2], col: Color32, alpha: u8, label: &str| {
                let col = Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), alpha);
                let pos = xf.doc_to_screen(p[0], p[1]);
                let arm = 7.0;
                painter.line_segment(
                    [egui::pos2(pos.x - arm, pos.y), egui::pos2(pos.x + arm, pos.y)],
                    Stroke::new(1.5, col),
                );
                painter.line_segment(
                    [egui::pos2(pos.x, pos.y - arm), egui::pos2(pos.x, pos.y + arm)],
                    Stroke::new(1.5, col),
                );
                painter.circle_stroke(pos, 4.0, Stroke::new(1.5, col));
                painter.text(
                    pos + egui::vec2(6.0, -6.0),
                    egui::Align2::LEFT_BOTTOM,
                    label,
                    egui::FontId::proportional(10.0),
                    col,
                );
            };
            if cur_frame > 0 {
                if let Some(s) = layer.track_points.get(cur_frame - 1) {
                    if let Some(p) = s.a {
                        draw_marker(p, a_col, 90, "");
                    }
                    if let Some(p) = s.b {
                        draw_marker(p, b_col, 90, "");
                    }
                }
            }
            if let Some(s) = layer.track_points.get(cur_frame) {
                if let Some(p) = s.a {
                    draw_marker(p, a_col, 255, "A");
                }
                if let Some(p) = s.b {
                    draw_marker(p, b_col, 255, "B");
                }
            }
        }
    }

    // Stroke/shape previews are captured in active-cell pixel space (the same
    // space they're rasterised into). Map them through the layer transform into
    // document space before going to screen, so previews line up with the
    // committed pixels on moved / scaled / rotated layers.
    let preview_xf = state.display_transform(cur_layer, cur_frame);
    let (preview_cw, preview_ch) = state
        .stroke_target
        .and_then(|id| state.project.cell(id))
        .map(|c| (c.width as f32, c.height as f32))
        .unwrap_or((pw, ph));
    let cell_to_screen = |u: f32, v: f32| -> egui::Pos2 {
        let (dx, dy) = preview_xf.cell_to_doc(u, v, preview_cw, preview_ch, pw, ph);
        xf.doc_to_screen(dx, dy)
    };
    // On-screen pixel size scales with both the view zoom and the layer scale.
    let layer_scale = preview_xf.scale.abs();

    // Live-tail overlay: the committed stroke is already streamed into the
    // cell texture via partial uploads, so only the short uncommitted span
    // between the last rasterized spine node and the cursor needs an overlay
    // (it hides the one-sample Catmull-Rom commit lag).
    if let Some(ref builder) = state.stroke {
        if let (Some((tail, _)), Some(cur)) = (builder.live_tail(), builder.current_node()) {
            let is_eraser = builder.tool == crate::tools::ActiveTool::Eraser;
            let a = builder.brush.opacity.clamp(0.0, 1.0);
            // Premultiply in gamma space to match the CPU compositor; egui's
            // `from_rgba_unmultiplied` premultiplies in linear space, which
            // over-brightens light colors at fractional alpha.
            let gamma_premul = |c: [u8; 3], a: u8| {
                let m = |v: u8| ((v as u16 * a as u16 + 127) / 255) as u8;
                Color32::from_rgba_premultiplied(m(c[0]), m(c[1]), m(c[2]), a)
            };
            let fill = if is_eraser {
                // Translucent cool-grey — reads as "lifting", not painting.
                gamma_premul([150, 158, 172], (a * 80.0) as u8)
            } else {
                let c = builder.brush.color;
                gamma_premul([c[0], c[1], c[2]], (a * 255.0) as u8)
            };
            let p0 = cell_to_screen(tail.x, tail.y);
            let p1 = cell_to_screen(cur.x, cur.y);
            let r0 = (tail.radius * scale * layer_scale).max(0.5);
            let r1 = (cur.radius * scale * layer_scale).max(0.5);
            painter.line_segment([p0, p1], Stroke::new(r0.min(r1) * 2.0, fill));
            painter.circle_filled(p0, r0, fill);
            painter.circle_filled(p1, r1, fill);
        }
    }

    // Shape-tool preview: drawn as egui shapes while dragging; the real pixels
    // are rasterised into the cell on pointer-up. Mirrors the freehand preview.
    if let Some(drag) = state.shape_drag {
        // Geometry is in active-cell pixel space; map every point through the
        // layer transform (cell_to_screen) so the preview follows zoom / pan /
        // rotation and the layer transform, matching the rasterised result.
        let c = state.brush.color;
        let col = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 255);
        let thick = (state.brush.radius * 2.0 * scale * layer_scale).max(1.0);
        let stroke = Stroke::new(thick, col);
        let (sx, sy) = drag.start;
        let (ex, ey) = drag.end;
        match drag.kind {
            ShapeKind::Line => {
                painter.line_segment([cell_to_screen(sx, sy), cell_to_screen(ex, ey)], stroke);
            }
            ShapeKind::Rect => {
                let (x0, y0) = (sx.min(ex), sy.min(ey));
                let (x1, y1) = (sx.max(ex), sy.max(ey));
                let pts = vec![
                    cell_to_screen(x0, y0),
                    cell_to_screen(x1, y0),
                    cell_to_screen(x1, y1),
                    cell_to_screen(x0, y1),
                ];
                painter.add(egui::Shape::closed_line(pts, stroke));
            }
            ShapeKind::Ellipse => {
                let cx = (sx + ex) * 0.5;
                let cy = (sy + ey) * 0.5;
                let rx = (ex - sx).abs() * 0.5;
                let ry = (ey - sy).abs() * 0.5;
                let n = 48;
                let mut pts = Vec::with_capacity(n);
                for i in 0..n {
                    let t = i as f32 / n as f32 * std::f32::consts::TAU;
                    pts.push(cell_to_screen(cx + rx * t.cos(), cy + ry * t.sin()));
                }
                painter.add(egui::Shape::closed_line(pts, stroke));
            }
        }
    }

    let outline_a = (state.bg_opacity * 180.0) as u8;
    if outline_a > 0 {
        painter.add(egui::Shape::closed_line(
            corners.to_vec(),
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(80, 80, 80, outline_a)),
        ));
    }
}

/// Canvas view transform: fit-to-window base scale combined with the user's
/// zoom / pan / rotation. Shared by rendering and input so they stay in sync.
#[derive(Clone, Copy)]
struct Xform {
    center: egui::Pos2,
    pan: Vec2,
    scale: f32,
    rot_sin: f32,
    rot_cos: f32,
    dcx: f32,
    dcy: f32,
    cw: f32,
    ch: f32,
}

impl Xform {
    fn new(state: &AppState, rect: Rect) -> Self {
        let cw = state.project.width as f32;
        let ch = state.project.height as f32;
        let base = (rect.width() / cw).min(rect.height() / ch);
        let scale = (base * state.view.zoom).max(1e-6);
        let (rot_sin, rot_cos) = state.view.rotation.sin_cos();
        Self {
            center: rect.center(),
            pan: state.view.pan,
            scale,
            rot_sin,
            rot_cos,
            dcx: cw * 0.5,
            dcy: ch * 0.5,
            cw,
            ch,
        }
    }

    fn doc_to_screen(&self, x: f32, y: f32) -> egui::Pos2 {
        let ox = (x - self.dcx) * self.scale;
        let oy = (y - self.dcy) * self.scale;
        let rx = ox * self.rot_cos - oy * self.rot_sin;
        let ry = ox * self.rot_sin + oy * self.rot_cos;
        self.center + self.pan + Vec2::new(rx, ry)
    }

    fn screen_to_doc(&self, p: egui::Pos2) -> (f32, f32) {
        let v = p - self.center - self.pan;
        let rx = v.x * self.rot_cos + v.y * self.rot_sin;
        let ry = -v.x * self.rot_sin + v.y * self.rot_cos;
        (rx / self.scale + self.dcx, ry / self.scale + self.dcy)
    }

    fn corners(&self) -> [egui::Pos2; 4] {
        [
            self.doc_to_screen(0.0, 0.0),
            self.doc_to_screen(self.cw, 0.0),
            self.doc_to_screen(self.cw, self.ch),
            self.doc_to_screen(0.0, self.ch),
        ]
    }
}

/// Draw a texture onto an arbitrary (possibly rotated) quad given its 4 screen
/// corners in TL, TR, BR, BL order. `tint` multiplies the sampled texels.
fn image_quad(painter: &egui::Painter, tex: egui::TextureId, corners: [egui::Pos2; 4], tint: Color32) {
    use egui::epaint::Vertex;
    let uv = [
        egui::pos2(0.0, 0.0),
        egui::pos2(1.0, 0.0),
        egui::pos2(1.0, 1.0),
        egui::pos2(0.0, 1.0),
    ];
    let mut mesh = egui::Mesh::with_texture(tex);
    for i in 0..4 {
        mesh.vertices.push(Vertex {
            pos: corners[i],
            uv: uv[i],
            color: tint,
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// Screen-space corners (TL, TR, BR, BL) of a cell of size `cw`×`ch` placed by
/// `t` on a `pw`×`ph` canvas, then mapped through the canvas view `xf`.
fn layer_screen_corners(
    xf: &Xform,
    t: crate::doc::transform::Transform,
    cw: f32,
    ch: f32,
    pw: f32,
    ph: f32,
) -> [egui::Pos2; 4] {
    let pts = [(0.0, 0.0), (cw, 0.0), (cw, ch), (0.0, ch)];
    let mut out = [egui::Pos2::ZERO; 4];
    for (i, (u, v)) in pts.iter().enumerate() {
        let (dx, dy) = t.cell_to_doc(*u, *v, cw, ch, pw, ph);
        out[i] = xf.doc_to_screen(dx, dy);
    }
    out
}

/// Paint the active tool's cursor preview on top of the canvas.
/// Pencil/Ink/Eraser → outline circle sized by brush radius (in doc px → screen
/// px via current canvas scale). Eraser shown with a dashed inner ring.
/// Fill → crosshair + small filled dot at the click point.
fn draw_tool_cursor(state: &AppState, ui: &egui::Ui, canvas_rect: Rect, pos: egui::Pos2) {
    let painter = ui.painter_at(canvas_rect);
    // Effective document-pixels → screen-pixels scale (includes zoom).
    let scale = Xform::new(state, canvas_rect).scale;

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
                [
                    egui::pos2(pos.x - arm, pos.y),
                    egui::pos2(pos.x + arm, pos.y),
                ],
                Stroke::new(1.4, black),
            );
            painter.line_segment(
                [
                    egui::pos2(pos.x, pos.y - arm),
                    egui::pos2(pos.x, pos.y + arm),
                ],
                Stroke::new(1.4, black),
            );
            painter.line_segment(
                [
                    egui::pos2(pos.x - arm, pos.y),
                    egui::pos2(pos.x + arm, pos.y),
                ],
                Stroke::new(0.8, white),
            );
            painter.line_segment(
                [
                    egui::pos2(pos.x, pos.y - arm),
                    egui::pos2(pos.x, pos.y + arm),
                ],
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
        ActiveTool::Shape => {
            // Crosshair anchor; the live preview shows the actual geometry.
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
            let c = state.brush.color;
            painter.circle_filled(
                pos,
                2.8,
                Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 255),
            );
            painter.circle_stroke(pos, 2.8, Stroke::new(0.8, black));
        }
        ActiveTool::ColorPicker => {
            // Simple crosshair + colour preview dot.
            let arm = 9.0;
            painter.line_segment(
                [
                    egui::pos2(pos.x - arm, pos.y),
                    egui::pos2(pos.x + arm, pos.y),
                ],
                Stroke::new(1.4, black),
            );
            painter.line_segment(
                [
                    egui::pos2(pos.x, pos.y - arm),
                    egui::pos2(pos.x, pos.y + arm),
                ],
                Stroke::new(1.4, black),
            );
            painter.line_segment(
                [
                    egui::pos2(pos.x - arm, pos.y),
                    egui::pos2(pos.x + arm, pos.y),
                ],
                Stroke::new(0.8, white),
            );
            painter.line_segment(
                [
                    egui::pos2(pos.x, pos.y - arm),
                    egui::pos2(pos.x, pos.y + arm),
                ],
                Stroke::new(0.8, white),
            );
            let c = state.brush.color;
            painter.circle_filled(
                pos,
                4.0,
                Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 255),
            );
            painter.circle_stroke(pos, 4.0, Stroke::new(1.2, white));
        }
        ActiveTool::Tracker => {
            // Wide crosshair with an open centre — precise point placement.
            let arm = 12.0;
            let gap = 3.0;
            for (w, col) in [(1.4, black), (0.8, white)] {
                for (a, b) in [
                    (egui::pos2(pos.x - arm, pos.y), egui::pos2(pos.x - gap, pos.y)),
                    (egui::pos2(pos.x + gap, pos.y), egui::pos2(pos.x + arm, pos.y)),
                    (egui::pos2(pos.x, pos.y - arm), egui::pos2(pos.x, pos.y - gap)),
                    (egui::pos2(pos.x, pos.y + gap), egui::pos2(pos.x, pos.y + arm)),
                ] {
                    painter.line_segment([a, b], Stroke::new(w, col));
                }
            }
            painter.circle_stroke(pos, gap, Stroke::new(1.0, theme::ACCENT));
        }
    }
}

/// Parse a color string like `"rgb(157, 89, 76)"` into RGBA bytes.
/// Returns `[r, g, b, 255]` on success, `None` on parse failure.
fn parse_rgb(text: &str) -> Option<[u8; 4]> {
    let text = text.trim();
    let inner = text
        .strip_prefix("rgb(")
        .or_else(|| text.strip_prefix("RGB("))
        .or_else(|| text.strip_prefix("Rgb("))?
        .strip_suffix(")")?;
    let mut parts = inner.split(',');
    let r = parts.next()?.trim().parse::<u8>().ok()?;
    let g = parts.next()?.trim().parse::<u8>().ok()?;
    let b = parts.next()?.trim().parse::<u8>().ok()?;
    Some([r, g, b, 255])
}

fn canvas_to_doc_mapping(state: &AppState, rect: Rect) -> impl Fn(egui::Pos2) -> (f32, f32) + Copy {
    let xf = Xform::new(state, rect);
    move |pos: egui::Pos2| -> (f32, f32) { xf.screen_to_doc(pos) }
}

/// Map a document point to the active layer's cell-local pixel space, inverting
/// the layer transform so drawing lands correctly on moved/scaled/rotated
/// layers. Falls back to project size when the frame has no cell yet.
fn doc_to_active_cell(state: &AppState, doc: (f32, f32)) -> (f32, f32) {
    let li = state.project.current_layer;
    let f = state.project.current_frame;
    let t = state.display_transform(li, f);
    let (cw, ch) = state
        .project
        .layers
        .get(li)
        .and_then(|l| l.resolve(f))
        .and_then(|id| state.project.cell(id))
        .map(|c| (c.width as f32, c.height as f32))
        .unwrap_or((state.project.width as f32, state.project.height as f32));
    let (pw, ph) = (state.project.width as f32, state.project.height as f32);
    t.doc_to_cell(doc.0, doc.1, cw, ch, pw, ph)
}

/// Small floating menu strip: File / Edit menus + a panel-visibility toggle.
/// The OS window frame provides the border and window controls.
fn menu_window(state: &mut AppState, ctx: &egui::Context) {
    egui::Window::new("menu_bar")
        .title_bar(false)
        .resizable(false)
        .default_pos([12.0, 8.0])
        .frame(floating_frame())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                title_menu(state, ctx, ui);
                ui.separator();
                let mt = tip(state, Action::TogglePanels, "Toggle panels");
                if theme::icon_button(ui, ic::SIDEBAR_SIMPLE, &mt).clicked() {
                    state.show_panels = !state.show_panels;
                }
                ui.separator();
                // Drag handle to move the borderless OS window (double-click =
                // maximize toggle), since there's no title bar.
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(40.0, 20.0), Sense::click_and_drag());
                if resp.is_pointer_button_down_on() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if resp.double_clicked() {
                    let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                }
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    ic::DOTS_SIX_VERTICAL,
                    egui::FontId::proportional(15.0),
                    theme::TEXT_MUTED,
                );
                if resp.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::Grab);
                }
            });
        });
}

fn title_menu(state: &mut AppState, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.menu_button(theme::icon_text(ic::PENCIL_SIMPLE, "Edit"), |ui| {
        let can_undo = state.history.can_undo();
        let can_redo = state.history.can_redo();
        let undo_label = format!(
            "Undo    {}",
            state
                .shortcuts
                .get(Action::Undo)
                .map(|c| c.display())
                .unwrap_or_default()
        );
        let redo_label = format!(
            "Redo    {}",
            state
                .shortcuts
                .get(Action::Redo)
                .map(|c| c.display())
                .unwrap_or_default()
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
            .button(theme::icon_text(ic::FILE_PLUS, "New project…"))
            .clicked()
        {
            state.new_project_cfg.width = state.project.width;
            state.new_project_cfg.height = state.project.height;
            state.new_project_cfg.fps = state.project.fps;
            state.show_new_project = true;
            ui.close_menu();
        }
        ui.separator();
        let open_label = format!("Open project…    {}", combo_text(state, Action::OpenProject));
        if ui
            .button(theme::icon_text(ic::FOLDER_OPEN, &open_label))
            .clicked()
        {
            match project_file::load_dialog() {
                Ok(Some(p)) => state.load_project(p),
                Ok(None) => {}
                Err(e) => log::error!("Open project failed: {e:#}"),
            }
            ui.close_menu();
        }
        let save_label = format!("Save project…    {}", combo_text(state, Action::SaveProject));
        if ui
            .button(theme::icon_text(ic::FLOPPY_DISK, &save_label))
            .clicked()
        {
            if let Err(e) = project_file::save_dialog(&state.project) {
                log::error!("Save project failed: {e:#}");
            }
            ui.close_menu();
        }
        ui.separator();
        if ui
            .button(theme::icon_text(
                ic::IMAGE,
                "Save PNG (current frame)…",
            ))
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
        if ui
            .button(theme::icon_text(ic::FILM_STRIP, "Export MP4…"))
            .clicked()
        {
            state.show_mp4_export = true;
            ui.close_menu();
        }
        ui.separator();
        if ui
            .button(theme::icon_text(
                ic::ARROW_LINE_DOWN,
                "Import PNG sequence…",
            ))
            .clicked()
        {
            if let Err(e) = png_import::import_dialog(&mut state.project) {
                log::error!("PNG import failed: {e:#}");
            }
            ui.close_menu();
        }
        if ui
            .button(theme::icon_text(ic::IMAGE, "Import image…"))
            .clicked()
        {
            state.import_image();
            ui.close_menu();
        }
        if ui
            .button(theme::icon_text(
                ic::CLIPBOARD,
                "Paste image as background",
            ))
            .clicked()
        {
            state.paste_image_as_background();
            ui.close_menu();
        }
        if ui
            .button(theme::icon_text(ic::FILM_REEL, "Import video…"))
            .clicked()
        {
            state.open_video_import();
            ui.close_menu();
        }
        if ui
            .button(theme::icon_text(ic::FILM_STRIP, "Import GIF…"))
            .clicked()
        {
            state.open_gif_import();
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
        if ui.button(theme::icon_text(ic::SIGN_OUT, "Quit")).clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    });
}

fn new_project_dialog(state: &mut AppState, ctx: &egui::Context) {
    if !state.show_new_project {
        return;
    }
    let mut open = true;
    let mut create = false;
    let mut cancel = false;
    egui::Window::new(theme::icon_text(ic::FILE_PLUS, "New project"))
        .open(&mut open)
        .default_pos([400.0, 200.0])
        .resizable(false)
        .collapsible(false)
        .frame(floating_frame())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Width:");
                ui.add(
                    egui::DragValue::new(&mut state.new_project_cfg.width)
                        .range(1..=16384)
                        .speed(1),
                );
                ui.label("Height:");
                ui.add(
                    egui::DragValue::new(&mut state.new_project_cfg.height)
                        .range(1..=16384)
                        .speed(1),
                );
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("FPS:");
                ui.add(
                    egui::DragValue::new(&mut state.new_project_cfg.fps)
                        .range(1.0..=120.0)
                        .speed(0.5),
                );
            });

            ui.add_space(8.0);
            theme::section_header(ui, ic::RECTANGLE, "Presets");
            ui.horizontal(|ui| {
                for &(label, w, h) in &[
                    ("HD 720p", 1280, 720),
                    ("Full HD", 1920, 1080),
                    ("2K", 2048, 1080),
                    ("4K UHD", 3840, 2160),
                    ("6K", 6144, 3456),
                    ("8K", 7680, 4320),
                ] {
                    if ui.button(label).clicked() {
                        state.new_project_cfg.width = w;
                        state.new_project_cfg.height = h;
                    }
                }
            });

            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(theme::icon_text(ic::CHECK, "Create")).clicked() {
                    create = true;
                }
                if ui.button(theme::icon_text(ic::X, "Cancel")).clicked() {
                    cancel = true;
                }
            });
        });

    // Process the result outside the egui closure to avoid borrow conflicts.
    if create {
        let (w, h, f) = (
            state.new_project_cfg.width.max(1),
            state.new_project_cfg.height.max(1),
            state.new_project_cfg.fps.max(1.0),
        );
        state.reset_with(w, h, f);
        state.show_new_project = false;
    } else if cancel || !open {
        state.show_new_project = false;
    }
}

fn mp4_export_dialog(state: &mut AppState, ctx: &egui::Context) {
    if !state.show_mp4_export {
        return;
    }
    let mut open = true;
    let mut export = false;
    let mut cancel = false;
    egui::Window::new(theme::icon_text(ic::FILM_STRIP, "Export MP4"))
        .open(&mut open)
        .default_pos([400.0, 200.0])
        .resizable(false)
        .collapsible(false)
        .frame(floating_frame())
        .show(ctx, |ui| {
            ui.label(format!(
                "{} × {} @ {:.0} fps · {} frames",
                state.project.width,
                state.project.height,
                state.project.fps,
                state.project.frame_count,
            ));
            ui.add_space(8.0);

            ui.add(
                egui::Slider::new(&mut state.mp4_cfg.crf, 0..=51).text("Quality (CRF)"),
            );
            ui.label(
                egui::RichText::new("Lower = better quality, larger file. 18 ≈ visually lossless.")
                    .small()
                    .color(theme::TEXT_MUTED),
            );
            ui.add_space(6.0);

            egui::ComboBox::from_label("Preset")
                .selected_text(MP4_PRESETS[state.mp4_cfg.preset_idx.min(MP4_PRESETS.len() - 1)])
                .show_ui(ui, |ui| {
                    for (i, p) in MP4_PRESETS.iter().enumerate() {
                        ui.selectable_value(&mut state.mp4_cfg.preset_idx, i, *p);
                    }
                });
            ui.label(
                egui::RichText::new("Slower preset = smaller file, longer encode.")
                    .small()
                    .color(theme::TEXT_MUTED),
            );

            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(theme::icon_text(ic::CHECK, "Export")).clicked() {
                    export = true;
                }
                if ui.button(theme::icon_text(ic::X, "Cancel")).clicked() {
                    cancel = true;
                }
            });
        });

    // Handle the result outside the closure to avoid borrowing `state` twice.
    if export {
        state.show_mp4_export = false;
        state.start_mp4_export();
    } else if cancel || !open {
        state.show_mp4_export = false;
    }
}

/// Start/end frame-range picker for video / GIF import (dual-knob slider plus
/// exact numeric inputs, with live previews of the start/end frames). The active
/// layer is untouched; the chosen range is laid onto a new layer from frame 0.
fn import_range_dialog(state: &mut AppState, ctx: &egui::Context) {
    if !state.show_import_range {
        return;
    }

    // Snapshot the range outside the window closure so the closure never borrows
    // `state` (lets us pull preview textures without borrow conflicts).
    let (total, mut start, mut end) = match &state.import_range {
        Some(st) => (st.total, st.start, st.end),
        None => return,
    };
    let last = total.saturating_sub(1);

    // Ensure previews for the two endpoints are loaded / loading, then grab the
    // (cheap, Arc-backed) texture handles to draw inside the closure.
    state.request_preview(ctx, start);
    state.request_preview(ctx, end);
    let start_tex = state.preview_tex.get(&start).cloned();
    let end_tex = state.preview_tex.get(&end).cloned();

    let mut open = true;
    let mut confirm = false;
    let mut cancel = false;
    egui::Window::new(theme::icon_text(ic::FILM_REEL, "Import frame range"))
        .open(&mut open)
        .default_pos([360.0, 140.0])
        .default_width(400.0)
        .resizable(false)
        .collapsible(false)
        .frame(floating_frame())
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(format!(
                    "{total} source frames. Pick the range — frames map onto the timeline starting at frame 0.",
                ))
                .color(theme::TEXT_MUTED)
                .size(11.0),
            );
            ui.add_space(8.0);

            ui.horizontal_top(|ui| {
                preview_box(ui, "Start", start, &start_tex);
                ui.add_space(12.0);
                preview_box(ui, "End", end, &end_tex);
            });
            ui.add_space(10.0);

            crate::ui::widgets::range_slider(ui, &mut start, &mut end, 0, last);
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.label("Start:");
                ui.add(egui::DragValue::new(&mut start).range(0..=last).speed(1));
                ui.add_space(12.0);
                ui.label("End:");
                ui.add(egui::DragValue::new(&mut end).range(0..=last).speed(1));
            });
            if start > end {
                end = start;
            }
            let count = end - start + 1;
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("{count} frame(s) will be imported."))
                    .color(theme::TEXT_MUTED)
                    .size(11.0),
            );

            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(theme::icon_text(ic::CHECK, "Import")).clicked() {
                    confirm = true;
                }
                if ui.button(theme::icon_text(ic::X, "Cancel")).clicked() {
                    cancel = true;
                }
            });
        });

    // Write the adjusted range back.
    if let Some(st) = state.import_range.as_mut() {
        st.start = start.min(last);
        st.end = end.min(last);
    }

    if confirm {
        state.confirm_import_range();
    } else if cancel || !open {
        state.cancel_import_range();
    }
}

/// One labelled preview thumbnail (or a spinner while it loads).
fn preview_box(ui: &mut egui::Ui, label: &str, idx: usize, tex: &Option<egui::TextureHandle>) {
    ui.vertical(|ui| {
        ui.label(
            egui::RichText::new(format!("{label}: frame {idx}"))
                .color(theme::TEXT_MUTED)
                .size(11.0),
        );
        let target_w = 170.0;
        match tex {
            Some(t) => {
                let s = t.size_vec2();
                let scale = (target_w / s.x.max(1.0)).min(1.0);
                ui.add(egui::Image::new(egui::load::SizedTexture::new(t.id(), s * scale)));
            }
            None => {
                ui.add_sized([target_w, target_w * 0.6], egui::Spinner::new());
            }
        }
    });
}

/// Modal "busy" overlay shown while a background import job runs.
fn busy_overlay(ctx: &egui::Context, label: &str) {
    egui::Area::new(egui::Id::new("import_busy"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(theme::BG_PANEL)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new());
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(label).color(theme::TEXT));
                    });
                });
        });
}
