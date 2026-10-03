//! Top-level egui layout: modern dark UI with Phosphor icons.
//! Floating windows, transparent canvas, layered + onion composite.

use egui::{Align, Color32, Frame, Margin, Rect, Sense, Stroke, Vec2};
use egui_phosphor::regular as ic;

use crate::app::{AppState, ExportKind, NavKind, PanelId, SelGesture, MP4_PRESETS};
use crate::color::{fmt_rgb, parse_color};
use crate::doc::camera::Ease;
use crate::doc::canvas::DirtyRect;
use crate::doc::layer::CellId;
use crate::input::button_drag::{self, ButtonDrag};
use crate::input::shortcuts::{Action, KeyCombo};
use crate::input::tablet::PenPacket;
use crate::io::{composite, png_import, png_save, project_file};
use crate::timeline::onion::{OnionConfig, OnionDirection, OnionPin, OnionStep, PIN_TINTS};
use crate::tools::fill;
use crate::tools::select_mask::{self, SelOp, SelShape};
use crate::tools::selection::Grab as SelGrab;
use crate::tools::{ActiveTool, BrushMode, BrushSettings, ShapeKind, Smoothing, StrokeCap};
use crate::ui::{color_wheel, expr, theme};

/// Tooltip text including the currently-bound shortcut (e.g. "Pencil  (Q)").
fn tip(state: &AppState, action: Action, base: &str) -> String {
    match state.shortcuts.get(action) {
        Some(c) => format!("{base}  ({})", c.display()),
        None => base.to_string(),
    }
}

/// Bare shortcut text for an action (e.g. "Ctrl+S"), empty if unbound.
pub(crate) fn combo_text(state: &AppState, action: Action) -> String {
    state
        .shortcuts
        .get(action)
        .map(|c| c.display())
        .unwrap_or_default()
}

pub fn draw(state: &mut AppState, ctx: &egui::Context) {
    if state.show_panels {
        // Did the window change size since panels were last drawn? If so each
        // panel is nudged to keep its distance from the screen edge it sits
        // nearest, so the right-hand column tracks the right edge instead of
        // being stranded mid-canvas. The baseline is only updated on frames that
        // actually draw panels — a resize while they're hidden is still applied
        // once they come back.
        let screen = ctx.screen_rect();
        let resized = state
            .viewport_rect
            .replace(screen)
            .filter(|prev| *prev != screen);

        menu_window(state, ctx);
        // Left: tools + brush.  Right: layers / onion / x-sheet.  Bottom: timeline.
        panel_window(state, ctx, PanelId::Tools, [12.0, 48.0], 232.0, true, resized);
        panel_window(state, ctx, PanelId::Brush, [12.0, 300.0], 232.0, true, resized);
        panel_window(state, ctx, PanelId::Color, [12.0, COLOR_PANEL_Y], 232.0, true, resized);
        panel_window(state, ctx, PanelId::Layers, [1004.0, 48.0], 252.0, true, resized);
        panel_window(state, ctx, PanelId::Onion, [1004.0, 300.0], 252.0, false, resized);
        panel_window(state, ctx, PanelId::Camera, [1004.0, 400.0], 252.0, false, resized);
        panel_window(state, ctx, PanelId::Xsheet, [1004.0, 470.0], 252.0, false, resized);
        panel_window(
            state,
            ctx,
            PanelId::Timeline,
            [320.0, 600.0],
            640.0,
            true,
            resized,
        );
        settings_window(state, ctx);
    } else if state.show_mini_timeline {
        mini_timeline_window(state, ctx);
    }
    // Deliberately outside the `show_panels` branch that holds
    // `settings_window`: hiding the panels is what you do to draw, and
    // drawing is when brush feel wants adjusting.
    brush_settings_window(state, ctx);
    // Outside it too: snapping is set per grid, and set while drawing.
    grid_settings_window(state, ctx);
    new_project_dialog(state, ctx);
    export_dialog(state, ctx);
    import_range_dialog(state, ctx);
    if let Some(label) = state.bg_label {
        busy_overlay(ctx, label);
    }
    save_error_dialog(state, ctx);
    save_toast(state, ctx);
    krita_toast(state, ctx);
    timeline_wheel_scrub(state, ctx);

    egui::CentralPanel::default()
        .frame(Frame::none().fill(Color32::TRANSPARENT))
        .show(ctx, |ui| {
            let avail = ui.available_size();
            let (canvas_rect, resp) = ui.allocate_exact_size(avail, Sense::drag());
            // Republish the doc → screen scale before anything reads it: the
            // brush-size lock and the stroke input filter both need it, and
            // only the canvas knows the fit-to-window base scale.
            state.view_scale = Xform::new(state, canvas_rect).scale;
            paint_canvas(state, ui, canvas_rect);

            // A live stroke is fed from the tablet packet queue, which is only
            // drained when a frame runs. eframe is reactive by default, so
            // without this the redraw cadence follows the OS mouse and the
            // queue backs up between events — dropping exactly the samples the
            // Wintab path exists to collect.
            if state.stroke.is_some() {
                ctx.request_repaint();
            }

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
            // A selection mode held as the drag starts. Only the Lasso reads
            // them, and while it is active they win over a nav gesture bound
            // to the same keys.
            let held_op = if state.tool == ActiveTool::Lasso {
                ui.input(|i| held_sel_op(state, i))
            } else {
                None
            };
            // What the middle or right button does, when one of those — not
            // the left, which always uses the tool — is what went down.
            let (middle, right) = (state.middle_drag, state.right_drag);
            let (nav_gesture, button) = ui.input(|i| {
                let g = if held_op.is_some() {
                    None
                } else if zoom_bind.is_some_and(|c| c.mods_held(i)) {
                    Some(NavKind::Zoom)
                } else if rotate_bind.is_some_and(|c| c.mods_held(i)) {
                    Some(NavKind::Rotate)
                } else if pan_bind.is_some_and(|c| c.mods_held(i)) {
                    Some(NavKind::Pan)
                } else {
                    None
                };
                let b = if i.pointer.primary_down() {
                    None
                } else if i.pointer.button_down(egui::PointerButton::Middle) {
                    Some(middle)
                } else if i.pointer.button_down(egui::PointerButton::Secondary) {
                    Some(right)
                } else {
                    None
                };
                (g, b)
            });

            if resp.drag_started() {
                // Pressing on the canvas hands the keyboard back to it: Delete
                // stops meaning "delete the drawings selected in the tracks".
                state.track_sel.clear();
                // Decide once, on press, what this drag does. A middle or
                // right button does what Settings binds it to, the view
                // gestures among those through the navigation drag. Otherwise
                // configurable modifiers pick zoom/rotate/pan, or the tool
                // draws. When layer-transform mode is on, the modifier
                // gestures retarget the active layer instead of the view.
                let (button_nav, button_drag) = match button {
                    Some(ButtonDrag::Pan) => (Some(NavKind::Pan), None),
                    Some(ButtonDrag::Zoom) => (Some(NavKind::Zoom), None),
                    Some(ButtonDrag::Rotate) => (Some(NavKind::Rotate), None),
                    other => (None, other),
                };
                // Modifiers only reinterpret a left-button drag: Shift with a
                // scrub means keys, not pan.
                let nav_gesture = if button.is_some() { None } else { nav_gesture };
                state.nav_drag = nav_gesture.or(button_nav);
                state.nav_to_layer = state.layer_xform && nav_gesture.is_some();
                state.nav_to_camera =
                    !state.nav_to_layer && state.camera_edit && nav_gesture.is_some();
                if state.nav_to_layer {
                    state.begin_layer_xform();
                } else if state.nav_to_camera {
                    state.begin_camera_drag();
                } else if let Some(b) = button_drag {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        state.begin_button_drag(b, pos);
                    }
                } else if state.nav_drag.is_none() {
                    // One complaint per stroke, not one per frame.
                    state.pen_outlier_logged = false;
                    if let Some(pos) = resp.interact_pointer_pos() {
                        if state.tool == ActiveTool::Tracker {
                            // Tracker takes the raw doc-space point — no cell
                            // mapping, no cell allocation.
                            state.tracker_click(canvas_to_doc(pos));
                        } else if state.tool == ActiveTool::Perspective {
                            // So do the grids: they live in document space,
                            // on no layer.
                            let (x, y) = canvas_to_doc(pos);
                            state.perspective_down([x, y]);
                        } else if state.tool == ActiveTool::Lasso {
                            // As does the selection: it belongs to the canvas,
                            // not to the layer under it. The cursor, not pen
                            // packets — a selection edge has no pressure.
                            state.select_down(canvas_to_doc(pos), held_op);
                        } else {
                            // Decided once, here: see `AppState::stroke_from_pen`.
                            let ppp = ui.ctx().pixels_per_point();
                            let start = pen_stroke_start(state, ppp, pos);
                            state.stroke_from_pen = start.is_some();
                            state.stroke_pen_samples = 0;
                            state.stroke_mouse_samples = 0;
                            let (at, packet) = match start {
                                Some((at, p)) => (at, Some(p)),
                                None => (pos, None),
                            };
                            let (x, y) = canvas_to_doc(at);
                            let [x, y] = state.snap_begin([x, y]);
                            let (cx, cy) = doc_to_active_cell(state, (x, y));
                            let t = ui.input(|i| i.time as f32);
                            let s = stroke_sample(state, cx, cy, t, packet);
                            state.pointer_down(s);
                            // A Fill press keeps listening: dragging before
                            // the pen lifts retunes it, counted from here.
                            state.fill_anchor(pos);
                        }
                    }
                }
            }
            if resp.dragged() {
                if state.nav_to_layer {
                    // Apply the gesture to the active layer's transform.
                    match state.nav_drag {
                        Some(NavKind::Pan) => {
                            // Un-rotate through the transform actually used to
                            // render, not `view.rotation` — they differ while
                            // the view is locked to the camera.
                            let xform = Xform::new(state, canvas_rect);
                            let (ddx, ddy) = xform.screen_delta_to_doc(resp.drag_delta());
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
                } else if state.nav_to_camera {
                    // Same retarget as the layer path, aimed at the camera. The
                    // guide rect follows the cursor: drag right to look right.
                    match state.nav_drag {
                        Some(NavKind::Pan) => {
                            let xform = Xform::new(state, canvas_rect);
                            let (ddx, ddy) = xform.screen_delta_to_doc(resp.drag_delta());
                            state.apply_camera_pan(ddx, ddy);
                        }
                        Some(NavKind::Rotate) => {
                            state.apply_camera_rotate(resp.drag_delta().x * 0.01);
                        }
                        Some(NavKind::Zoom) => {
                            let dy = resp.drag_delta().y;
                            if dy.abs() > 0.0 {
                                state.apply_camera_zoom((-dy * 0.01).exp());
                            }
                        }
                        None => {}
                    }
                } else if state.camera_look_through && state.nav_drag.is_some() {
                    // While the view is locked to the camera, `Xform` ignores
                    // `state.view` entirely. Swallow the view gestures rather
                    // than let them accumulate invisibly and jump the canvas
                    // the moment the lock comes off.
                } else {
                    match state.nav_drag {
                        Some(NavKind::Pan) => {
                            state.view.pan += resp.drag_delta();
                        }
                        Some(NavKind::Rotate) => {
                            let dx = resp.drag_delta().x;
                            if dx.abs() > 0.0 {
                                // Anchor rotation on the viewport center, so the
                                // doc point in the middle of the window holds
                                // still. Without the correction the pivot is the
                                // *document* center — invisible at fit zoom, but
                                // it swings the canvas off-screen once you are
                                // zoomed in and panned away from it.
                                let anchor = canvas_rect.center();
                                let before =
                                    Xform::new(state, canvas_rect).screen_to_doc(anchor);
                                state.view.rotation += dx * 0.01;
                                let after =
                                    Xform::new(state, canvas_rect).doc_to_screen(before.0, before.1);
                                state.view.pan += anchor - after;
                            }
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
                        None if state.button_drag.is_some() => {
                            if let Some(pos) = resp.interact_pointer_pos() {
                                state.button_drag_to(pos);
                            }
                        }
                        None if state.tool == ActiveTool::Perspective => {
                            if let Some(pos) = resp.interact_pointer_pos() {
                                let (x, y) = canvas_to_doc(pos);
                                state.perspective_move([x, y]);
                            }
                        }
                        None if state.tool == ActiveTool::Lasso => {
                            if let Some(pos) = resp.interact_pointer_pos() {
                                state.select_move(canvas_to_doc(pos));
                            }
                        }
                        // Screen points, not canvas: a drag step is the same
                        // length of pen travel at any zoom.
                        None if state.fill_dragging() => {
                            if let Some(pos) = resp.interact_pointer_pos() {
                                state.fill_drag_to(pos);
                            }
                        }
                        // The press frame: `pointer_down` has just started
                        // the stroke at the newest packet, and this frame's
                        // other packets lie at or behind it. Feeding them
                        // too would double the line back on itself.
                        None if resp.drag_started() => {}
                        None => {
                            let t = ui.input(|i| i.time as f32);
                            let pointer = resp.interact_pointer_pos();
                            let frame = if state.stroke_from_pen {
                                let ppp = ui.ctx().pixels_per_point();
                                pen_stroke_points(state, ppp, pointer)
                            } else {
                                PenFrame::Untrusted
                            };
                            let samples =
                                stroke_frame_samples(&mut state.stroke_from_pen, frame, pointer);
                            for (pos, packet) in samples {
                                let doc = snapped(state, canvas_to_doc(pos));
                                let (cx, cy) = doc_to_active_cell(state, doc);
                                let s = stroke_sample(state, cx, cy, t, packet);
                                state.pointer_move(s);
                            }
                        }
                    }
                }
            }
            if resp.drag_stopped() {
                if state.nav_to_layer {
                    state.commit_layer_xform();
                } else if state.nav_to_camera {
                    state.commit_camera_drag();
                } else if state.button_drag.is_some() {
                    state.end_button_drag();
                } else if state.nav_drag.is_none() {
                    // Both are no-ops when idle, so a tool change mid-drag
                    // cannot strand either.
                    state.select_up();
                    state.pointer_up();
                }
                state.nav_drag = None;
                state.nav_to_layer = false;
                state.nav_to_camera = false;
            }
            // A double-click closes a polygon selection. Its second press has
            // already placed a corner on top of the first; `polygon_finish`
            // drops the duplicate.
            if state.polygon_active()
                && resp.hovered()
                && ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary))
            {
                state.polygon_finish();
            }

            // Tool cursor preview — only while drawing (not during nav gestures),
            // when the pointer is over the canvas and not over a floating panel.
            // A tool adjust holds the brush ring where the press landed, so the
            // size can be judged against the drawing under it; the swatch pie
            // shows the pointer instead.
            let pie = state.button_drag.is_some_and(|d| d.kind == ButtonDrag::SwatchPie);
            if state.nav_drag.is_none() && !pie && (resp.hovered() || resp.dragged()) {
                let pos = resp.hover_pos().or_else(|| resp.interact_pointer_pos());
                if let Some(pos) = pos {
                    if canvas_rect.contains(pos) {
                        let ring = match state.button_drag {
                            Some(d) if d.kind == ButtonDrag::ToolAdjust => d.anchor,
                            _ => pos,
                        };
                        draw_tool_cursor(state, ui, canvas_rect, ring);
                        if let Some(text) = state.button_drag_readout() {
                            draw_readout(&ui.painter_at(canvas_rect), pos, text);
                        }
                        ctx.set_cursor_icon(egui::CursorIcon::None);
                    }
                }
            }
            if let Some(d) = state.button_drag.filter(|d| d.kind == ButtonDrag::SwatchPie) {
                draw_swatch_pie(ctx, &state.palette, &d, state.brush.color);
            }
            } // end: if !state.screen_pick
        });

    // Live screen colour-pick: sample the pixel under the cursor each frame and
    // commit on tap. Drawn last so the loupe sits above the canvas.
    screen_pick_live(state, ctx);
}

/// Drive live screen-pick mode: read the pixel under the OS cursor straight
/// from the screen, show a swatch/hex loupe at the cursor, and commit the
/// colour on the next tap. Escape cancels.
///
/// Because the sample comes from the OS framebuffer, this picks whatever is
/// literally on screen — the canvas and its backdrop as currently configured,
/// or whatever sits behind the window wherever the backdrop is transparent.
/// Pick mode never changes the backdrop to make that happen.
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
    // etc.) — a click there is aimed at the widget, not at a colour. NOTE: can't use
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
    painter.rect_filled(rect, 6.0, theme::premul(10, 11, 14, 230));
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

    painter.rect_filled(rect, 6.0, theme::premul(10, 11, 14, 235));
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

    // HSL readout + swatch below the loupe.
    let (swatch, label) = match color {
        Some(c) => (Color32::from_rgb(c[0], c[1], c[2]), fmt_rgb([c[0], c[1], c[2]])),
        None => (Color32::from_gray(40), "—".to_string()),
    };
    let galley = painter.layout_no_wrap(label, egui::FontId::monospace(13.0), Color32::WHITE);
    // `hsl(360, 100%, 100%)` is wider than the loupe, so the bar grows to fit.
    // It grows away from the cursor, so it stays out of the sampled region.
    let bar_w = size.max(galley.size().x + 32.0);
    let bar_x = if rect.min.x < cursor.x { rect.max.x - bar_w } else { rect.min.x };
    let bar = Rect::from_min_size(egui::pos2(bar_x, rect.max.y + 4.0), Vec2::new(bar_w, 22.0));
    painter.rect_filled(bar, 4.0, theme::premul(10, 11, 14, 235));
    let sw = Rect::from_min_size(bar.min + Vec2::new(5.0, 4.0), Vec2::splat(14.0));
    painter.rect_filled(sw, 2.0, swatch);
    painter.rect_stroke(sw, 2.0, Stroke::new(1.0, Color32::from_gray(90)));
    painter.galley(bar.min + Vec2::new(26.0, 3.0), galley, Color32::WHITE);
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
        PanelId::Camera => (ic::VIDEO_CAMERA, "Camera"),
        PanelId::Color => (ic::PALETTE, "Color"),
    }
}

/// Mouse wheel over the canvas or the timeline scrubs the frame.
///
/// Reads the raw `MouseWheel` events rather than `smooth_scroll_delta`: a mouse
/// reports whole `Line`s (one per notch), which maps to one step with no
/// threshold guessing, while `smooth_scroll_delta` has already been eaten by
/// whichever `ScrollArea` the pointer sits over. The raw events survive that
/// consumption, so the layer test below is what stops us double-handling a
/// scroll meant for a panel's own list.
fn timeline_wheel_scrub(state: &mut AppState, ctx: &egui::Context) {
    // Mid-stroke or mid-nav-drag the frame must not move: the stroke is writing
    // into the cell resolved for the frame it started on.
    if state.stroke.is_some() || state.nav_drag.is_some() {
        return;
    }

    // Which surface is under the pointer? Same trick as the colour-pick guard
    // above: the canvas is `Order::Background`, floating panels are `Middle`,
    // so a layer id identifies the window. Only the canvas and the two timeline
    // windows scrub — everything else keeps scrolling normally.
    let Some(layer) = ctx
        .input(|i| i.pointer.hover_pos())
        .and_then(|p| ctx.layer_id_at(p))
    else {
        return;
    };
    let over_timeline = layer.id == egui::Id::new(panel_key(PanelId::Timeline));
    let scrubs = layer.order == egui::Order::Background
        || over_timeline
        || layer.id == egui::Id::new("mini_timeline");
    if !scrubs {
        return;
    }
    // The tracks own the modified wheel (Ctrl zooms, Shift pans) and the
    // wheel over the layer names (scrolls the rows).
    if over_timeline {
        let mods = ctx.input(|i| i.modifiers);
        let pointer = ctx.input(|i| i.pointer.hover_pos());
        let on_names = crate::ui::tracks::header_rect(ctx)
            .zip(pointer)
            .is_some_and(|(r, p)| r.contains(p));
        if mods.command || mods.shift || on_names {
            return;
        }
    }

    // Only the vertical axis. Horizontal (shift+wheel, trackpad-X) is left for
    // the frame strip's own `ScrollArea` to pan with.
    let (lines, points) = ctx.input(|i| {
        let mut lines = 0.0_f32;
        let mut points = 0.0_f32;
        for e in &i.events {
            if let egui::Event::MouseWheel { unit, delta, .. } = e {
                match unit {
                    egui::MouseWheelUnit::Line => lines += delta.y,
                    egui::MouseWheelUnit::Point => points += delta.y,
                    egui::MouseWheelUnit::Page => lines += delta.y,
                }
            }
        }
        (lines, points)
    });

    // Trackpads report points, and one flick is many tiny events — bank them
    // until they add up to a notch. A direction change drops the bank so the
    // reversal is felt immediately instead of paying off the old debt first.
    let mut notches = lines;
    if points != 0.0 {
        if state.wheel_scrub_accum != 0.0 && points.signum() != state.wheel_scrub_accum.signum() {
            state.wheel_scrub_accum = 0.0;
        }
        state.wheel_scrub_accum += points;
        let whole = (state.wheel_scrub_accum / POINTS_PER_SCRUB_NOTCH).trunc();
        state.wheel_scrub_accum -= whole * POINTS_PER_SCRUB_NOTCH;
        notches += whole;
    }
    if notches == 0.0 {
        return;
    }

    // Wheel *down* gives a negative y, and down advances the timeline by
    // default — hence the negation.
    let mut dir = -notches.signum() as isize;
    if state.invert_timeline_scroll {
        dir = -dir;
    }
    // Playback rewrites `current_frame` every tick, so a scrub during playback
    // would be invisible. Stop it, as a pointer-down on the canvas already does.
    state.playback.stop();
    let delta = dir * notches.abs() as isize * state.frame_step_delta();
    let wrap = state.loop_timeline;
    state.project.step(delta, wrap);
}

/// Trackpad scroll (in points) that counts as one wheel notch.
const POINTS_PER_SCRUB_NOTCH: f32 = 24.0;

/// Default top of the Color panel: just under Brush in the left column,
/// clear of the timeline, which starts further right.
const COLOR_PANEL_Y: f32 = 546.0;

/// Stable egui Id for a panel window. Without this the Id is hashed from the
/// window's title — which embeds a phosphor glyph — so bumping the icon font or
/// editing a title in `panel_meta` would silently orphan every saved position.
fn panel_key(id: PanelId) -> &'static str {
    match id {
        PanelId::Tools => "panel_tools",
        PanelId::Brush => "panel_brush",
        PanelId::Layers => "panel_layers",
        PanelId::Onion => "panel_onion",
        PanelId::Xsheet => "panel_xsheet",
        PanelId::Timeline => "panel_timeline",
        PanelId::Camera => "panel_camera",
        PanelId::Color => "panel_color",
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
        PanelId::Camera => camera_content(state, ui),
        PanelId::Color => color_wheel::color_panel(state, ui),
    }
}

/// Render a panel as a draggable floating window.
///
/// `resized` carries the previous viewport rect on frames where the window
/// changed size; the panel is then re-stuck to its nearest edge.
fn panel_window(
    state: &mut AppState,
    ctx: &egui::Context,
    id: PanelId,
    default_pos: [f32; 2],
    width: f32,
    open: bool,
    resized: Option<Rect>,
) {
    let (icon, title) = panel_meta(id);
    let mut window = egui::Window::new(theme::icon_text(icon, title))
        .id(egui::Id::new(panel_key(id)))
        .default_pos(default_pos)
        .default_width(width)
        .default_open(open)
        .resizable(true)
        .collapsible(true)
        .frame(floating_frame());
    if let Some(old) = resized {
        if let Some(pos) = resticked_pos(ctx, panel_key(id), old, ctx.screen_rect()) {
            window = window.current_pos(pos);
        }
    }
    window.show(ctx, |ui| {
        drag_by_title_only(ui);
        panel_content(state, ctx, ui, id)
    });
}

/// Make a window move by its title bar only.
///
/// egui moves a window on a drag anywhere no widget claims — the margins, the
/// gap between two sliders — so a slightly-off grab on a control dragged the
/// whole panel. This lays a drag-only catcher over the body, above egui's
/// move handle and under every control, since those are added after it.
/// The outer edges stay clear so they still resize.
///
/// Call it first thing in the window's contents. A collapsed window has no
/// contents, so it still drags from its header.
fn drag_by_title_only(ui: &mut egui::Ui) {
    // Contents live on the window's own layer, whose id is the window's.
    let id = ui.layer_id().id;
    let Some(outer) = egui::AreaState::load(ui.ctx(), id).map(|s| s.rect()) else {
        return;
    };
    let edge = ui.style().interaction.resize_grab_radius_side;
    let body = Rect::from_min_max(
        egui::pos2(outer.left() + edge, ui.max_rect().top()),
        outer.max - Vec2::splat(edge),
    );
    if !body.is_positive() {
        return;
    }
    // The body clip would trim the catcher to the content and leave the
    // margins draggable, so interact under the catcher's own rect.
    let clip = ui.clip_rect();
    ui.set_clip_rect(body);
    ui.interact(body, id.with("body_drag_catcher"), Sense::drag());
    ui.set_clip_rect(clip);
}

/// Where a panel should sit after the viewport went from `old` to `new`.
///
/// Each axis is handled independently and keeps the panel's gap from whichever
/// edge it was nearest — a right-hand panel tracks the right edge, a bottom one
/// tracks the bottom. `None` means leave it alone.
fn resticked_pos(ctx: &egui::Context, key: &str, old: Rect, new: Rect) -> Option<[f32; 2]> {
    let st = egui::AreaState::load(ctx, egui::Id::new(key))?;
    // `size` is `None` until the area has been laid out once (and it is
    // deliberately not persisted by egui). Without it `rect()` reports a
    // zero-sized box and every gap below would be nonsense.
    let size = st.size?;
    let top_left = st.left_top_pos();
    Some([
        restick_axis(
            top_left.x,
            top_left.x + size.x,
            size.x,
            old.min.x,
            old.max.x,
            new.min.x,
            new.max.x,
        ),
        restick_axis(
            top_left.y,
            top_left.y + size.y,
            size.y,
            old.min.y,
            old.max.y,
            new.min.y,
            new.max.y,
        ),
    ])
}

/// One axis of [`resticked_pos`] — returns the new minimum coordinate.
fn restick_axis(lo: f32, hi: f32, len: f32, o_lo: f32, o_hi: f32, n_lo: f32, n_hi: f32) -> f32 {
    let gap_lo = lo - o_lo;
    let gap_hi = o_hi - hi;
    // A panel that was centred to within a few pixels stays centred, so the
    // bottom-centre Timeline doesn't slide off to one side on a wider window.
    const CENTRED_TOL: f32 = 8.0;
    let pos = if (gap_lo - gap_hi).abs() <= CENTRED_TOL {
        n_lo + ((n_hi - n_lo) - len) * 0.5
    } else if gap_lo <= gap_hi {
        n_lo + gap_lo
    } else {
        n_hi - gap_hi - len
    };
    // Never push a panel off the new viewport. `max` guards the case where the
    // panel is larger than the window, which would invert the clamp range.
    pos.clamp(n_lo, (n_hi - len).max(n_lo))
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
                let g = tip(state, Action::ToolShape, "Shape");
                let l = tip(state, Action::ToolLasso, "Lasso select");
                tool_toggle(ui, state, ActiveTool::Fill, ic::PAINT_BUCKET, &f);
                tool_toggle(ui, state, ActiveTool::Shape, ic::SHAPES, &g);
                tool_toggle(ui, state, ActiveTool::Lasso, ic::LASSO, &l);
                let tr = tip(state, Action::ToolTracker, "Tracker (stabilize)");
                tool_toggle(ui, state, ActiveTool::Tracker, ic::CROSSHAIR, &tr);
                let pg = tip(state, Action::ToolPerspective, "Perspective grid");
                tool_toggle(ui, state, ActiveTool::Perspective, ic::PERSPECTIVE, &pg);
                ui.add_space(6.0);
                // The one colour picker — a momentary mode, not a persistent
                // tool. Samples any pixel on screen, canvas included, leaving
                // the backdrop as-is, then returns to the tool that was active.
                let sp = tip(state, Action::PickScreenColor, "Color picker");
                if theme::icon_toggle(ui, ic::EYEDROPPER, &sp, state.screen_pick).clicked() {
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
                ui.add(
                    egui::Slider::new(&mut state.brush.fill_expand, 0..=fill::MAX_EXPAND)
                        .text("Expand (px)"),
                )
                .on_hover_text(
                    "Grow the fill by this many pixels so the colour tucks under \
                     anti-aliased lines instead of leaving a halo. Meant for the \
                     'lines from' workflow — on a same-layer fill it eats into \
                     your own strokes.\n\nOr press on the canvas and drag up / down.",
                );
                ui.add(
                    // Logarithmic, so the small gaps line art mostly needs
                    // keep most of the track.
                    egui::Slider::new(&mut state.brush.fill_gap, 0..=fill::MAX_GAP)
                        .logarithmic(true)
                        .smallest_positive(1.0)
                        .text("Gap (px)"),
                )
                .on_hover_text(
                    "Treat breaks in the lines up to this wide as closed, so the \
                     colour doesn't pour out through them.\n\nOr press on the \
                     canvas and drag left / right.",
                );
                ui.checkbox(&mut state.brush.fill_all_visible, "Read all visible layers")
                    .on_hover_text(
                        "Stop at lines on any visible layer, not only this layer or \
                         the one it takes its lines from.",
                    );
                // The boundary source is a per-layer link, set in the Layers
                // panel — surface it here so the coupling is visible.
                let hint = match state.fill_boundary_name() {
                    _ if state.brush.fill_all_visible => {
                        "Lines from: all visible layers".to_string()
                    }
                    Some(name) => format!("Lines from: {name}"),
                    None => "Lines from: this layer — set it in the Layers panel".to_string(),
                };
                ui.label(egui::RichText::new(hint).color(theme::TEXT_MUTED).size(11.0));
            } else if state.tool == ActiveTool::Shape {
                ui.horizontal(|ui| {
                    shape_kind_toggle(ui, state, ShapeKind::Line, ic::LINE_SEGMENT, "Line");
                    shape_kind_toggle(ui, state, ShapeKind::Rect, ic::RECTANGLE, "Rectangle");
                    shape_kind_toggle(ui, state, ShapeKind::Ellipse, ic::CIRCLE, "Ellipse");
                });
                if state.shapes_on_grid() {
                    let n = state.perspective.active + 1;
                    ui.label(
                        egui::RichText::new(format!(
                            "On perspective grid {n} — Shift once dragging: square / circle"
                        ))
                        .color(theme::TEXT_MUTED)
                        .size(11.0),
                    );
                }
                brush_size_lock(state, ui);
                let label = if state.lock_brush_to_view {
                    "Thickness (screen px)"
                } else {
                    "Thickness"
                };
                ui.add(egui::Slider::new(&mut state.brush.radius, 0.5..=64.0).text(label));
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
            } else if state.tool == ActiveTool::Perspective {
                perspective_options(state, ui);
            } else if state.tool == ActiveTool::Lasso {
                selection_options(state, ui);
            } else {
                brush_size_lock(state, ui);
                let label = if state.lock_brush_to_view {
                    "Size (screen px)"
                } else {
                    "Size"
                };
                ui.add(egui::Slider::new(&mut state.brush.radius, 0.5..=128.0).text(label));
                ui.add(egui::Slider::new(&mut state.brush.opacity, 0.0..=1.0).text("Opacity"));
            }
    }
}

/// "Lock brush to screen size" toggle, shared by the freehand and Shape size
/// sliders (both of which relabel themselves when it is on).
fn brush_size_lock(state: &mut AppState, ui: &mut egui::Ui) {
    ui.checkbox(&mut state.lock_brush_to_view, "Lock brush to screen size")
        .on_hover_text(
            "Keep the brush the same size on screen at every zoom, so the same \
             hand movement always draws the same stroke.\n\nOff (default): size \
             is in document pixels, so a line keeps its weight in the exported \
             frame no matter what zoom you drew it at.",
        );
}

/// Pinned colour swatches: click to use, `+` to pin the current colour,
/// right-click a swatch to remove it. Wraps, so a full palette costs two rows.
fn swatch_strip(state: &mut AppState, ui: &mut egui::Ui) {
    const SIZE: f32 = 16.0;
    let cur = {
        let c = state.brush.color;
        [c[0], c[1], c[2]]
    };
    let mut pick: Option<[u8; 3]> = None;
    let mut remove: Option<usize> = None;

    ui.horizontal_wrapped(|ui| {
        for (i, rgb) in state.palette.iter().copied().enumerate() {
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::splat(SIZE), Sense::click());
            let fill = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            ui.painter().rect_filled(rect, 3.0, fill);
            // The active colour is called out with an accent ring, so a palette
            // of near-identical greys still tells you where you are.
            let stroke = if rgb == cur {
                Stroke::new(2.0, theme::ACCENT)
            } else {
                Stroke::new(1.0, theme::STROKE_THIN)
            };
            ui.painter().rect_stroke(rect, 3.0, stroke);
            if resp.clicked() {
                pick = Some(rgb);
            }
            resp.context_menu(|ui| {
                if ui.button("Remove swatch").clicked() {
                    remove = Some(i);
                    ui.close_menu();
                }
            });
            resp.on_hover_text(fmt_rgb(rgb));
        }
        let full = state.palette.len() >= AppState::MAX_SWATCHES;
        let held = state.palette.contains(&cur);
        if ui
            .add_enabled(!held, egui::Button::new("+").min_size(Vec2::splat(SIZE)))
            .on_hover_text(if full {
                "Pin this colour (the oldest swatch drops)"
            } else {
                "Pin this colour"
            })
            .on_disabled_hover_text("Already pinned")
            .clicked()
        {
            state.pin_swatch();
        }
    });

    if let Some(rgb) = pick {
        state.set_brush_color(rgb);
    }
    if let Some(i) = remove {
        state.palette.remove(i);
    }
}

fn brush_content(state: &mut AppState, ui: &mut egui::Ui) {
    {
            theme::section_header(ui, ic::PALETTE, "Color");
            ui.horizontal(|ui| {
                let c = state.brush.color;
                let mut rgb = [c[0], c[1], c[2]];
                if color_wheel::hsl_edit_button(ui, &mut rgb).changed() {
                    state.set_brush_color(rgb);
                }
                ui.label(fmt_rgb(rgb));
                if paste_button(ui) {
                    if let Some(rgb) = clipboard_color() {
                        state.set_brush_color(rgb);
                    }
                }
            });
            swatch_strip(state, ui);

            ui.add_space(6.0);
            if ui
                .add_sized(
                    [ui.available_width(), 24.0],
                    egui::Button::new(theme::icon_text(ic::SLIDERS, "Brush settings…")),
                )
                .on_hover_text("Presets, dab shape, paper grain, pressure and tilt, smoothing.")
                .clicked()
            {
                state.show_brush_settings = true;
            }

            // Canvas backdrop + input status — collapsed by default so the panel
            // stays compact, expandable when needed.
            ui.add_space(6.0);
            egui::CollapsingHeader::new(theme::icon_text(ic::IMAGE, "Backdrop"))
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(&mut state.bg_opacity, 0.0..=1.0).text("Opacity"));
                        color_wheel::hsl_edit_button_f32(ui, &mut state.bg_color);
                    });
                    ui.checkbox(&mut state.show_checker, "Checker backdrop");
                    ui.horizontal(|ui| {
                        let rgb = state.bg_color.map(|c| (c * 255.0).round() as u8);
                        ui.label(egui::RichText::new(fmt_rgb(rgb)).color(theme::TEXT_MUTED));
                        if paste_button(ui) {
                            if let Some(rgb) = clipboard_color() {
                                state.bg_color = rgb.map(|c| c as f32 / 255.0);
                            }
                        }
                    });
                });

            ui.add_space(2.0);
            // Three states, not two. A driver whose context opens but never
            // reports is the failure worth being able to see at a glance —
            // it draws exactly like a mouse, and "Tablet active" would be a
            // lie about the interesting case.
            let (icon, text, color, tip): (&str, &str, _, &str) = match (
                state.pen.is_active(),
                state.pen.pen_active(),
            ) {
                (_, true) => (
                    ic::PEN,
                    "Tablet active",
                    theme::ACCENT,
                    "Pen packets are arriving: pressure, tilt and sub-pixel positions are live.",
                ),
                (true, false) => (
                    ic::WARNING,
                    "Tablet idle",
                    theme::TEXT_MUTED,
                    "A tablet context is open but no packets have arrived recently. \
                     Normal while the pen is away from the tablet; if it persists \
                     while drawing, the driver is not sending Wintab data and \
                     strokes fall back to the mouse.",
                ),
                (false, _) => (
                    ic::CURSOR,
                    "Mouse mode",
                    theme::TEXT_MUTED,
                    "No tablet driver was found, so pressure is fixed and tilt reports none.",
                ),
            };
            ui.label(
                egui::RichText::new(format!("{icon}  {text}"))
                    .color(color)
                    .size(11.0),
            )
            .on_hover_text(tip);
    }
}

fn timeline_content(state: &mut AppState, ctx: &egui::Context, ui: &mut egui::Ui) {
    // One strip: transport, step size, whole-timeline frame edits, then the
    // active layer's own frame edits, with the frame counter and fps pinned to
    // the right. The ruler under it scrubs, so there is no frame slider.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let gap = |ui: &mut egui::Ui| {
            ui.add(egui::Separator::default().spacing(12.0));
        };

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
        if loop_toggle(ui, state).clicked() {
            state.loop_timeline = !state.loop_timeline;
        }
        gap(ui);

        if theme::icon_button(ui, ic::SKIP_BACK, "Go to loop start").clicked() {
            state.project.goto(state.project.loop_start);
        }
        let prev_tip = tip(state, Action::FramePrev, "Step back");
        if theme::icon_button(ui, ic::CARET_LEFT, &prev_tip).clicked() {
            state.project.step(-state.frame_step_delta(), state.loop_timeline);
        }
        let next_tip = tip(state, Action::FrameNext, "Step forward");
        if theme::icon_button(ui, ic::CARET_RIGHT, &next_tip).clicked() {
            state.project.step(state.frame_step_delta(), state.loop_timeline);
        }
        // Jump drawing-to-drawing, skipping holds. Resolved before the
        // closures: `add_enabled_ui` borrows `state` for the duration, so the
        // target frame has to be in hand first.
        let key_prev = state.project.prev_key_frame();
        let key_next = state.project.next_key_frame();
        let key_prev_tip = tip(state, Action::KeyJumpPrev, "Previous drawing key");
        ui.add_enabled_ui(key_prev.is_some(), |ui| {
            if theme::icon_button(ui, ic::CARET_LINE_LEFT, &key_prev_tip).clicked() {
                state.project.goto(key_prev.unwrap_or_default());
            }
        });
        let key_next_tip = tip(state, Action::KeyJumpNext, "Next drawing key");
        ui.add_enabled_ui(key_next.is_some(), |ui| {
            if theme::icon_button(ui, ic::CARET_LINE_RIGHT, &key_next_tip).clicked() {
                state.project.goto(key_next.unwrap_or_default());
            }
        });
        // One value for both halves of the toolbar: how far the arrows move,
        // and how many frames the + / copy buttons insert.
        let step_tip = format!(
            "Step size — frames moved by {} / {}, frames inserted by {} / {}",
            combo_text(state, Action::FramePrev),
            combo_text(state, Action::FrameNext),
            combo_text(state, Action::FrameAdd),
            combo_text(state, Action::FrameDuplicate),
        );
        ui.add(
            egui::DragValue::new(&mut state.frame_step)
                .range(1..=999)
                .speed(1)
                .prefix("×"),
        )
        .on_hover_text(step_tip);
        gap(ui);

        let n = state.frame_step_count();
        let add_base = if n == 1 {
            "Add frame (hold)".to_string()
        } else {
            format!("Add {n} frames (hold)")
        };
        let add_tip = tip(state, Action::FrameAdd, &add_base);
        if theme::icon_button(ui, ic::PLUS, &add_tip).clicked() {
            state.structural_edit(false, |p| {
                for _ in 0..n {
                    p.add_frame();
                }
            });
        }
        let dup_base = if n == 1 {
            "Duplicate frame".to_string()
        } else {
            format!("Duplicate frame ×{n}")
        };
        let dup_tip = tip(state, Action::FrameDuplicate, &dup_base);
        if theme::icon_button(ui, ic::COPY, &dup_tip).clicked() {
            state.structural_edit(false, |p| {
                for _ in 0..n {
                    p.duplicate_frame();
                }
            });
        }
        let del_tip = tip(state, Action::FrameDelete, "Delete frame");
        if theme::icon_button(ui, ic::TRASH, &del_tip).clicked() {
            let wipes_pixels = state.project.frame_count <= 1;
            state.structural_edit(wipes_pixels, |p| p.delete_frame());
        }
        gap(ui);

        // The same step size, on the active layer alone: its drawings slide,
        // every other layer stays where it is.
        let frames = if n == 1 {
            "a frame".to_string()
        } else {
            format!("{n} frames")
        };
        let ins_tip = format!(
            "Insert {frames} on this layer — the drawing at the playhead holds longer, \
             later ones slide"
        );
        if theme::icon_button(ui, ic::ARROWS_OUT_LINE_HORIZONTAL, &ins_tip).clicked() {
            state.insert_layer_frames_here(n);
        }
        let rem_tip = format!(
            "Remove {frames} on this layer, from the playhead — later drawings slide up"
        );
        if theme::icon_button(ui, ic::ARROWS_IN_LINE_HORIZONTAL, &rem_tip).clicked() {
            state.remove_layer_frames_here(n);
        }

        // Right to left: fps at the edge, the frame counter beside it.
        let count = state.project.frame_count.max(1);
        let mut cur = state.project.current_frame;
        // Frozen before the widget is built: a relative expression must
        // measure from where the edit started, not from a value the edit has
        // already moved.
        let base = cur as f64;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::DragValue::new(&mut state.project.fps)
                    .range(1.0..=60.0)
                    .speed(0.1)
                    .max_decimals(1)
                    .suffix(" fps"),
            )
            .on_hover_text("Playback speed");
            gap(ui);
            ui.label(egui::RichText::new(format!("/ {count}")).color(theme::TEXT_MUTED));
            let field = ui
                .add(
                    egui::DragValue::new(&mut cur)
                        .range(0..=count.saturating_sub(1))
                        .speed(1)
                        .update_while_editing(false)
                        .custom_parser(move |s| expr::eval(s, base).map(f64::round)),
                )
                .on_hover_text(
                    "Frame number. Takes arithmetic: 22/2, 8+12, (4+8)*2.\n\n\
                     Start with an operator to go relative to this frame: +12 jumps 12 \
                     ahead, -3 back, /2 to the halfway frame. Enter applies it.",
                );
            if field.changed() {
                state.project.goto(cur);
            }
            ui.label(egui::RichText::new(ic::CLOCK).color(theme::TEXT_MUTED).size(13.0));
        });
    });

    ui.add_space(6.0);
    // The tracks fill the panel, so they are the one body egui's resize
    // arithmetic can push around — leave its shortfall unused.
    let avail = ui.available_size();
    let h = (avail.y - title_bar_shortfall(ctx, PanelId::Timeline)).max(0.0);
    ui.allocate_ui(egui::vec2(avail.x, h), |ui| crate::ui::tracks::show(state, ui));
}

/// How much taller a panel's title bar is than egui budgets for.
///
/// While a window edge is dragged, egui 0.29 sizes the body as the dragged
/// outer rect minus the frame and a title bar one Heading row tall — but the
/// title bar it lays out is at least `interact_size.y` tall, which our theme
/// makes taller than that row. A body that fills the height it is given then
/// ends up that much taller than the rect being dragged, and since each
/// frame's drag starts from the last frame's rect, a sideways drag grows the
/// panel by this much every frame. Content that leaves it unused comes out
/// exactly the size dragged.
fn title_bar_shortfall(ctx: &egui::Context, id: PanelId) -> f32 {
    let (icon, title) = panel_meta(id);
    let style = ctx.style();
    // Measured exactly as `Window` measures its title: a plain `RichText`,
    // which `Window::new` falls back to the Heading style.
    let text = egui::RichText::new(theme::icon_text(icon, title)).text_style(egui::TextStyle::Heading);
    let budget = ctx.fonts(|f| text.font_height(f, &style));
    (style.spacing.interact_size.y - budget).max(0.0)
}

/// Compact playback HUD shown when the floating panels are hidden (Tab).
/// Pinned bottom-centre: play/pause, step, frame counter, scrub strip.
/// One-click brush presets, matched to the Krita brushes they are named
/// after. Colour is deliberately kept: swapping preset should not change
/// what you are drawing with.
fn brush_presets(state: &mut AppState, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        for (label, tip, build) in BrushSettings::PRESETS {
            if ui.button(*label).on_hover_text(*tip).clicked() {
                let color = state.brush.color;
                state.brush = build();
                state.brush.color = color;
            }
        }
    });
}

/// Brush shape and response. Which controls appear depends on the brush's
/// rasterization model: flow, spacing and tilt only mean anything to a
/// brush that stamps dabs.
fn brush_dynamics(state: &mut AppState, ui: &mut egui::Ui) {
    let dab = state.brush.mode == BrushMode::Dab;
    ui.horizontal(|ui| {
        ui.label("Model");
        ui.selectable_value(&mut state.brush.mode, BrushMode::Ribbon, "Ribbon")
            .on_hover_text(
                "One continuous band. Even density however slowly you draw, \
                 and no darkening where the stroke crosses itself — ink.",
            );
        ui.selectable_value(&mut state.brush.mode, BrushMode::Dab, "Dabs")
            .on_hover_text(
                "Stamps that build up where they overlap. Density comes from \
                 how much you go over the same ground — graphite.",
            );
    });

    if state.brush.mode == BrushMode::Krita {
        krita_brush_summary(state, ui);
        return;
    }

    ui.add(egui::Slider::new(&mut state.brush.hardness, 0.0..=1.0).text("Hardness"))
        .on_hover_text("Fraction of the radius that stays fully solid.");
    ui.add(egui::Slider::new(&mut state.brush.softness, 0.2..=4.0).text("Softness"))
        .on_hover_text("Bends the edge falloff inwards. 1.0 is a plain taper.");
    ui.add(egui::Slider::new(&mut state.brush.grain, 0.0..=1.0).text("Grain"))
        .on_hover_text("How deeply the paper tooth eats into the stroke.");
    ui.add(egui::Slider::new(&mut state.brush.grain_scale, 0.5..=6.0).text("Grain scale"))
        .on_hover_text("Canvas pixels per grain texel — coarser paper as it rises.");

    ui.add_enabled_ui(!dab, |ui| {
        ui.horizontal(|ui| {
            ui.label("Ends");
            ui.selectable_value(&mut state.brush.cap, StrokeCap::Round, "Round")
                .on_hover_text("The stroke ends in a half-circle.");
            ui.selectable_value(&mut state.brush.cap, StrokeCap::Flat, "Flat")
                .on_hover_text(
                    "Cut straight across where the pen touched down and lifted. \
                     A quick tap still leaves a dot.",
                );
        });
    });

    ui.add_enabled_ui(dab, |ui| {
        ui.add(egui::Slider::new(&mut state.brush.flow, 0.02..=1.0).text("Flow"))
            .on_hover_text("Alpha of a single stamp. Low values build density slowly.");
        ui.add(egui::Slider::new(&mut state.brush.spacing, 0.02..=1.0).text("Spacing"))
            .on_hover_text("Gap between stamps, as a fraction of the dab.");
    });

    ui.add(egui::Slider::new(&mut state.brush.size.amount, 0.0..=1.0).text("Pres → size"));
    ui.add(egui::Slider::new(&mut state.brush.size.gamma, 0.3..=3.0).text("Size curve"))
        .on_hover_text(
            "Below 1 the brush reaches full width the moment it touches; \
             above 1 it holds thin until you lean on it, which is what \
             gives a long taper.",
        );
    ui.add(egui::Slider::new(&mut state.brush.flow_dyn.amount, 0.0..=1.0).text("Pres → flow"));
    ui.add(egui::Slider::new(&mut state.brush.flow_dyn.gamma, 0.3..=3.0).text("Flow curve"));

    ui.add_enabled_ui(dab, |ui| {
        ui.add(
            egui::Slider::new(&mut state.brush.tilt_elongation, 0.0..=1.0).text("Tilt → shape"),
        )
        .on_hover_text("Leaning the pen flattens the dab across the lean.");
        ui.add(egui::Slider::new(&mut state.brush.tilt_size, 0.0..=1.0).text("Tilt → size"));
    });
    if dab && !state.pen.pen_active() {
        ui.label(
            egui::RichText::new("Tilt needs a tablet — mouse input reports none.")
                .small()
                .color(theme::TEXT_MUTED),
        );
    }
}

/// What a Krita preset does, in place of the sliders it does not use: its
/// response is the preset's own, so the only controls that still apply are
/// Size and Opacity, which act as Krita's.
fn krita_brush_summary(state: &AppState, ui: &mut egui::Ui) {
    use crate::tools::krita::KritaPreset;
    let (name, lines): (&str, &[&str]) = match state.brush.krita {
        KritaPreset::Pencil5Tilted => (
            "Krita · Pencil-5 Tilted",
            &[
                "Size ← tilt: a quarter size upright, full size laid flat",
                "Rotation ← the way the pen leans",
                "Opacity ← pressure, on the preset's curve",
                "Paper texture, stronger the harder you press",
                "Builds up dab by dab in 8 bits, as Krita does",
            ],
        ),
    };
    ui.label(egui::RichText::new(name).strong());
    for l in lines {
        ui.label(egui::RichText::new(*l).small());
    }
    ui.label(
        egui::RichText::new("Size and Opacity work as Krita's own sliders.")
            .small()
            .color(theme::TEXT_MUTED),
    );
    if !state.pen.pen_active() {
        ui.label(
            egui::RichText::new("Tilt needs a tablet — mouse input reports none.")
                .small()
                .color(theme::TEXT_MUTED),
        );
    }
}

/// One `key   value` line of the tablet readout.
fn diag_row(ui: &mut egui::Ui, key: &str, value: String) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(key).color(theme::TEXT_MUTED).size(11.0));
        ui.label(egui::RichText::new(value).monospace().size(11.0));
    });
}

/// Colour for a diagnostic reading that is out of range. Local because the
/// theme has no warning colour and one readout does not justify adding to it.
pub(crate) const DIAG_BAD: Color32 = Color32::from_rgb(232, 138, 96);

/// What the tablet driver is actually reporting.
///
/// A release build has no console, so nothing this module logs is reachable
/// when it matters. And a tablet whose axis mapping is wrong does not fail —
/// it draws, confidently, somewhere else. The number that settles it is the
/// last line: the gap between where the pen says it is and where the cursor
/// is. If that grows the further you get from the middle of the screen, an
/// axis is inverted.
fn tablet_diagnostics(state: &AppState, ui: &mut egui::Ui) {
    let d = state.pen.diagnostics();
    let cursor = ui.input(|i| i.pointer.latest_pos());


    if !d.backend {
        ui.label(
            egui::RichText::new("No Wintab driver loaded.")
                .color(theme::TEXT_MUTED)
                .size(11.0),
        );
        return;
    }

    diag_row(
        ui,
        "packets",
        format!(
            "{} this frame, {} peak, {} total",
            d.packets_this_frame, d.max_packets_per_frame, d.total_packets
        ),
    );
    // The stride question, answered. 76 bytes on a 64-bit build means the
    // driver granted every field; anything less means the packet is shorter
    // than the reference struct, which is the case that used to misread every
    // packet after the first in a batch.
    diag_row(
        ui,
        "packet",
        format!(
            "{} bytes, mask {:#06x}, x {:?} y {:?} pressure {:?} tilt {:?}",
            d.layout.size, d.layout.mask, d.layout.x, d.layout.y, d.layout.pressure,
            d.layout.orientation
        ),
    );
    diag_row(
        ui,
        "rejected",
        format!(
            "{} batches, {} stray packets, {} full drains",
            state.pen_batches_rejected, state.pen_packets_dropped, d.drained_full
        ),
    );
    // A pen stroke should read "0 cursor": any cursor sample in it is a
    // whole-pixel position among sub-pixel ones, and shows up as a kink.
    diag_row(
        ui,
        "last stroke",
        format!(
            "{} pen / {} cursor samples",
            state.stroke_pen_samples, state.stroke_mouse_samples
        ),
    );
    diag_row(ui, "pressure", format!("{:.3}", d.pressure));
    diag_row(ui, "tilt", format!("{:.1}, {:.1} deg", d.tilt.0, d.tilt.1));
    {
        // The same tilt as a Krita brush reads it: whole degrees, then its
        // elevation and direction sensors.
        use crate::tools::krita::{qt_tilt, sensors};
        let info = sensors::PaintInfo {
            x_tilt: qt_tilt(d.tilt.0) as f64,
            y_tilt: qt_tilt(d.tilt.1) as f64,
            ..Default::default()
        };
        diag_row(
            ui,
            "krita tilt",
            format!(
                "elevation {:.2}, direction {:.0} deg",
                sensors::tilt_elevation(&info, 60.0, 60.0, true),
                sensors::tilt_direction(&info, false).to_degrees()
            ),
        );
    }
    diag_row(
        ui,
        "map x",
        format!("origin {:.0}, packet origin {:.0}, {:.5} px/unit", d.map_x.0, d.map_x.1, d.map_x.2),
    );
    diag_row(
        ui,
        "map y",
        format!("origin {:.0}, packet origin {:.0}, {:.5} px/unit", d.map_y.0, d.map_y.1, d.map_y.2),
    );
    // What the positions actually go through once learned — the rows above
    // are only what the driver claims.
    for (name, fit) in [("fit x", d.fit_x), ("fit y", d.fit_y)] {
        let text = match fit {
            Some(f) => format!(
                "origin {:.1}, {:.5} px/unit, rms {:.2} px",
                f.origin, f.scale, f.rms
            ),
            None => format!("learning, {} pairs — move the pen around", d.fit_pairs),
        };
        diag_row(ui, name, text);
    }
    if let Some((rx, ry)) = d.last_raw {
        diag_row(ui, "raw", format!("{rx}, {ry}"));
    }
    if let Some((mx, my)) = d.last_mapped {
        diag_row(ui, "mapped", format!("{mx:.1}, {my:.1} desktop px"));
    }
    if let Some((ox, oy)) = d.client_origin {
        diag_row(ui, "client origin", format!("{ox:.0}, {oy:.0}"));
    }
    if d.queue_overflowed {
        ui.label(
            egui::RichText::new("packet queue has overflowed")
                .color(DIAG_BAD)
                .size(11.0),
        );
    }

    // The one that matters.
    let ppp = ui.ctx().pixels_per_point();
    match (d.last_mapped, d.client_origin, cursor) {
        (Some((mx, my)), Some((ox, oy)), Some(c)) => {
            let pen = egui::pos2((mx - ox) / ppp, (my - oy) / ppp);
            let (dx, dy) = (pen.x - c.x, pen.y - c.y);
            let bad = dx.abs() > 4.0 || dy.abs() > 4.0;
            ui.label(
                egui::RichText::new(format!("pen - cursor: {dx:+.1}, {dy:+.1} pt"))
                    .monospace()
                    .size(11.0)
                    .color(if bad { DIAG_BAD } else { theme::ACCENT }),
            )
            .on_hover_text(
                "Move the pen around the screen with this open. Both numbers should \
                 stay near zero. One that is near zero across the middle of the \
                 screen and grows towards the edges means that axis is inverted.",
            );
        }
        _ => {
            diag_row(ui, "pen - cursor", "hover the pen over the window".into());
        }
    }
}

/// Line smoothing controls, mirroring Krita's freehand tool options.
///
/// Basic is the default in both apps, and it does no positional filtering at
/// all — a clean line comes from sub-pixel tablet input and the Bezier fit
/// through it, not from averaging the hand away. Weighted is there for people
/// who want the extra help and can live with the lag it costs.
fn smoothing_controls(state: &mut AppState, ui: &mut egui::Ui) {
    let label = match state.smoothing.kind {
        Smoothing::None => "None",
        Smoothing::Basic => "Basic",
        Smoothing::Weighted => "Weighted",
    };
    egui::ComboBox::from_id_salt("smoothing_kind")
        .selected_text(label)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut state.smoothing.kind, Smoothing::None, "None")
                .on_hover_text("Straight lines between raw samples.");
            ui.selectable_value(&mut state.smoothing.kind, Smoothing::Basic, "Basic")
                .on_hover_text(
                    "Krita's default. No averaging — samples are joined by a cubic                      Bezier fitted to the local tangents.",
                );
            ui.selectable_value(&mut state.smoothing.kind, Smoothing::Weighted, "Weighted")
                .on_hover_text(
                    "A gaussian average over the recent samples, then the same                      Bezier fit. Steadier, at the cost of the stroke trailing                      the pen.",
                );
        });

    ui.add_enabled_ui(state.smoothing.kind == Smoothing::Weighted, |ui| {
        // Krita keeps a separate width for fast strokes. Dragging the main
        // slider carries the other with it unless the artist has deliberately
        // split them, which is the same thing Krita's linked-ratio button does.
        let linked = (state.smoothing.distance_min - state.smoothing.distance_max).abs() < 0.01;
        let resp = ui.add(
            egui::Slider::new(&mut state.smoothing.distance_max, 3.0..=200.0).text("Distance"),
        );
        if linked && resp.changed() {
            state.smoothing.distance_min = state.smoothing.distance_max;
        }
        ui.add(
            egui::Slider::new(&mut state.smoothing.distance_min, 3.0..=200.0)
                .text("Distance (fast)"),
        )
        .on_hover_text("Filter width once the pen is moving quickly.");
        ui.add(
            egui::Slider::new(&mut state.smoothing.tail_aggressiveness, 0.0..=1.0)
                .text("Tail aggressiveness"),
        )
        .on_hover_text("How hard the filter resists the thinning at a stroke's start.");
        ui.checkbox(&mut state.smoothing.smooth_pressure, "Smooth pressure")
            .on_hover_text("Run pressure through the same filter as position.");
        ui.checkbox(&mut state.smoothing.scalable_distance, "Scale distance with zoom")
            .on_hover_text(
                "Measure the distance above in screen pixels rather than canvas                  pixels, so smoothing feels the same at every zoom. Off, a                  zoomed-out stroke is barely smoothed at all.",
            );
    });
}

/// The Loop toggle shared by both timeline windows. One flag covers playback,
/// wheel scrub and the frame step actions, so "loop off" means the same thing
/// however the playhead is being moved.
fn loop_toggle(ui: &mut egui::Ui, state: &AppState) -> egui::Response {
    let tip = if state.loop_timeline {
        "Loop on — playback repeats, and stepping wraps at both ends"
    } else {
        "Loop off — playback stops at the end, and stepping clamps"
    };
    theme::icon_toggle(ui, ic::REPEAT, tip, state.loop_timeline)
}

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
                if loop_toggle(ui, state).clicked() {
                    state.loop_timeline = !state.loop_timeline;
                }
                let prev_tip = tip(state, Action::FramePrev, "Step back");
                if theme::icon_button(ui, ic::CARET_LEFT, &prev_tip).clicked() {
                    state.project.step(-state.frame_step_delta(), state.loop_timeline);
                }
                let next_tip = tip(state, Action::FrameNext, "Step forward");
                if theme::icon_button(ui, ic::CARET_RIGHT, &next_tip).clicked() {
                    state.project.step(state.frame_step_delta(), state.loop_timeline);
                }
                // Jump drawing-to-drawing, skipping holds. Resolved before
                // the closures: `add_enabled_ui` borrows `state` for the
                // duration, so the target frame has to be in hand first.
                let key_prev = state.project.prev_key_frame();
                let key_next = state.project.next_key_frame();
                let key_prev_tip = tip(state, Action::KeyJumpPrev, "Previous drawing key");
                ui.add_enabled_ui(key_prev.is_some(), |ui| {
                    if theme::icon_button(ui, ic::CARET_LINE_LEFT, &key_prev_tip).clicked() {
                        state.project.goto(key_prev.unwrap_or_default());
                    }
                });
                let key_next_tip = tip(state, Action::KeyJumpNext, "Next drawing key");
                ui.add_enabled_ui(key_next.is_some(), |ui| {
                    if theme::icon_button(ui, ic::CARET_LINE_RIGHT, &key_next_tip).clicked() {
                        state.project.goto(key_next.unwrap_or_default());
                    }
                });
                // Same step size as the timeline panel. Drag-only here: the
                // panels-hidden path surrenders keyboard focus every frame.
                ui.add(
                    egui::DragValue::new(&mut state.frame_step)
                        .range(1..=999)
                        .speed(1)
                        .prefix("×"),
                )
                .on_hover_text("Step size (move / insert)");
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
                    Color32::from_rgb(c[0], c[1], c[2]),
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
        ActiveTool::Shape => ic::SHAPES,
        ActiveTool::Tracker => ic::CROSSHAIR,
        ActiveTool::Lasso => ic::LASSO,
        ActiveTool::Perspective => ic::PERSPECTIVE,
    }
}

fn tool_name(tool: ActiveTool) -> &'static str {
    match tool {
        ActiveTool::Pencil => "Pencil",
        ActiveTool::Ink => "Ink",
        ActiveTool::Eraser => "Eraser",
        ActiveTool::Fill => "Fill",
        ActiveTool::Shape => "Shape",
        ActiveTool::Tracker => "Tracker",
        ActiveTool::Lasso => "Lasso select",
        ActiveTool::Perspective => "Perspective grid",
    }
}

/// Minimal frame indicator for the mini bar: one fixed-size dot per frame,
/// active filled. Horizontally scrollable so dots stay distinguishable on long
/// timelines; auto-scrolls to keep the active frame in view. Click / drag to
/// scrub. (Unhide the full Timeline panel for detail.)
fn mini_frame_dots(state: &mut AppState, ui: &mut egui::Ui) {
    let n = state.project.frame_count.max(1);
    let cur = state.project.current_frame;
    let (layer_keys, camera_keys) = key_flags(state, n);
    let mut grid_keys = vec![false; n];
    let grid_color = grid_key_marks(state).map(|(frames, color)| {
        for f in frames.into_iter().filter(|&f| f < n) {
            grid_keys[f] = true;
        }
        color
    });
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
                // Keyed frames get a tick under the dot — the layer, the
                // active grid and the camera, side by side when they share a
                // frame, in the colours the full timeline uses.
                let ty = rect.max.y - 1.5;
                let ticks: Vec<Color32> = [
                    (layer_keys[i], KEY_LAYER),
                    (grid_keys[i], grid_color.unwrap_or(KEY_LAYER)),
                    (camera_keys[i], KEY_CAMERA),
                ]
                .into_iter()
                .filter_map(|(on, c)| on.then_some(c))
                .collect();
                let x0 = center.x - (ticks.len() as f32 - 1.0) * 2.5;
                for (k, c) in ticks.into_iter().enumerate() {
                    painter.circle_filled(egui::pos2(x0 + k as f32 * 5.0, ty), 1.5, c);
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

/// Marker colour for active-layer transform keys — same blue as the layer
/// bounds outline on the canvas.
pub(crate) const KEY_LAYER: Color32 = Color32::from_rgb(120, 160, 220);
/// Marker colour for camera keys — same amber as the camera-edit guide.
pub(crate) const KEY_CAMERA: Color32 = Color32::from_rgb(255, 190, 90);

/// The active grid's keyed frames and its colour, while grids are on show —
/// the perspective tool is up, or "Show grids" is on — and it has keys.
pub(crate) fn grid_key_marks(state: &AppState) -> Option<(Vec<usize>, Color32)> {
    let shown = state.tool == ActiveTool::Perspective || state.perspective.show;
    let g = state.perspective.active_grid().filter(|g| shown && g.visible && !g.keys.is_empty())?;
    let [r, gr, b] = g.color;
    Some((g.keys.iter().map(|k| k.frame).collect(), Color32::from_rgb(r, gr, b)))
}

/// Per-frame "is there a key here" flags for the active layer's transform and
/// for the camera, as two `n`-long tables.
///
/// Built once per timeline widget rather than probed inside the draw loop:
/// `has_transform_key` is a linear scan, so asking it per frame is quadratic on
/// a long scene.
fn key_flags(state: &AppState, n: usize) -> (Vec<bool>, Vec<bool>) {
    let mut layer = vec![false; n];
    let mut camera = vec![false; n];
    if let Some(l) = state.project.layers.get(state.project.current_layer) {
        for k in &l.transform_keys {
            if k.frame < n {
                layer[k.frame] = true;
            }
        }
    }
    for k in &state.project.camera_keys {
        if k.frame < n {
            camera[k.frame] = true;
        }
    }
    (layer, camera)
}

fn onion_content(state: &mut AppState, ui: &mut egui::Ui) {
    {
            ui.checkbox(&mut state.onion.enabled, "Enabled");
            ui.checkbox(&mut state.onion.by_key, "Step by drawings")
                .on_hover_text(
                    "Count distinct drawings instead of frames, so on twos and \
                     threes Prev = 2 reaches the two previous drawings rather \
                     than two frames of the same held one.\n\nOff: steps frame \
                     by frame, and a hold simply shows fewer ghosts.",
                );
            ui.add_space(4.0);
            ui.add(egui::Slider::new(&mut state.onion.prev, 0..=8).text("Prev"));
            ui.add(egui::Slider::new(&mut state.onion.next, 0..=8).text("Next"));
            onion_offset_chips(state, ui);
            ui.add(egui::Slider::new(&mut state.onion.max_alpha, 0.0..=1.0).text("Max α"));
            ui.add(egui::Slider::new(&mut state.onion.falloff, 0.5..=4.0).text("Falloff"));
            ui.add_space(4.0);
            theme::section_header(ui, ic::PALETTE, "Tints");
            color_picker_u8(ui, "Prev", &mut state.onion.prev_tint);
            color_picker_u8(ui, "Next", &mut state.onion.next_tint);
    }
    ui.add_space(6.0);
    onion_pins_content(state, ui);
}

/// Offset chip size, and the gap between chips.
const CHIP: Vec2 = Vec2::new(18.0, 16.0);
const CHIP_GAP: f32 = 2.0;

/// One chip per ghost in range, `−prev … −1 • +1 … +next`; clicking one
/// hides or shows that offset without touching the others. One row when it
/// fits the panel; otherwise the past on one row and the future under it,
/// rather than wrapping mid-range or stretching the panel.
fn onion_offset_chips(state: &mut AppState, ui: &mut egui::Ui) {
    let onion = &mut state.onion;
    let (prev, next) = (onion.prev, onion.next);
    if prev == 0 && next == 0 {
        return;
    }
    let unit = if onion.by_key { "drawing" } else { "frame" };
    let step = CHIP.x + CHIP_GAP;
    let one_row = f32::from(prev + next) * step + CHIP.y * 0.5 <= ui.available_width();
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(CHIP_GAP, CHIP_GAP);
        let past = |ui: &mut egui::Ui, onion: &mut OnionConfig| {
            for k in (1..=prev).rev() {
                offset_chip(ui, onion, k, OnionDirection::Prev, unit);
            }
        };
        let future = |ui: &mut egui::Ui, onion: &mut OnionConfig| {
            for k in 1..=next {
                offset_chip(ui, onion, k, OnionDirection::Next, unit);
            }
        };
        if one_row {
            ui.horizontal(|ui| {
                past(ui, onion);
                // The current frame. Painted: the UI font has no bullet glyph.
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(CHIP.y * 0.5, CHIP.y), Sense::hover());
                ui.painter().circle_filled(rect.center(), 2.5, theme::TEXT_MUTED);
                resp.on_hover_text("The current frame");
                future(ui, onion);
            });
        } else {
            if prev > 0 {
                ui.horizontal(|ui| past(ui, onion));
            }
            if next > 0 {
                ui.horizontal(|ui| future(ui, onion));
            }
        }
    });
    ui.add_space(2.0);
}

fn offset_chip(
    ui: &mut egui::Ui,
    onion: &mut OnionConfig,
    k: u8,
    dir: OnionDirection,
    unit: &str,
) {
    let shown = !onion.is_hidden(k, dir);
    let [r, g, b] = onion.tint_rgb(dir);
    let tint = Color32::from_rgb(r, g, b);
    let (sign, way) = match dir {
        OnionDirection::Prev => ('−', "back"),
        OnionDirection::Next => ('+', "ahead"),
    };
    let (rect, resp) = ui.allocate_exact_size(CHIP, Sense::click());
    let painter = ui.painter();
    let text = if shown {
        painter.rect_filled(rect, 3.0, tint);
        // Dark or light label, whichever reads on the tint.
        let luma = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
        if luma > 150.0 { Color32::BLACK } else { Color32::WHITE }
    } else {
        painter.rect_stroke(rect, 3.0, Stroke::new(1.0, tint.gamma_multiply(0.6)));
        theme::TEXT_MUTED
    };
    if resp.hovered() {
        painter.rect_stroke(rect, 3.0, Stroke::new(1.0, theme::TEXT));
    }
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("{sign}{k}"),
        egui::FontId::proportional(10.0),
        text,
    );
    let plural = if k == 1 { "" } else { "s" };
    let state = if shown { "shown — click to hide" } else { "hidden — click to show" };
    if resp
        .on_hover_text(format!("{k} {unit}{plural} {way}: {state}"))
        .clicked()
    {
        onion.toggle_hidden(k, dir);
    }
}

/// Pinned frames of the active layer: pin one by number or from the
/// playhead, then recolour, hide or remove each.
fn onion_pins_content(state: &mut AppState, ui: &mut egui::Ui) {
    theme::section_header(ui, ic::PUSH_PIN, "Pinned frames");
    let count = state.project.frame_count.max(1);
    let cur = state.project.current_frame;
    let li = state.project.current_layer;
    let Some(layer) = state.project.layers.get_mut(li) else {
        return;
    };
    ui.label(
        egui::RichText::new(format!(
            "Pins on \u{201c}{}\u{201d} show while it's the active layer.",
            layer.name
        ))
        .color(theme::TEXT_MUTED)
        .size(10.5),
    );

    // The frame to pin: a scratch value, kept in egui memory rather than in
    // the state. Numbered like the timeline, from 0.
    let mem_id = ui.id().with("onion_pin_frame");
    let mut typed: usize = ui.data(|d| d.get_temp(mem_id)).unwrap_or(0);
    typed = typed.min(count - 1);
    let mut add = None;
    ui.horizontal(|ui| {
        ui.label("Frame");
        ui.add(egui::DragValue::new(&mut typed).range(0..=count - 1).speed(0.25));
        if theme::icon_button(ui, ic::PLUS, "Pin this frame").clicked() {
            add = Some(typed);
        }
        if theme::icon_button(
            ui,
            ic::PUSH_PIN,
            "Pin the current frame. Its ghost shows once the playhead moves off it.",
        )
        .clicked()
        {
            add = Some(cur);
        }
    });
    ui.data_mut(|d| d.insert_temp(mem_id, typed));
    if let Some(frame) = add {
        if !layer.onion_pins.iter().any(|p| p.frame == frame) {
            let tint = PIN_TINTS[layer.onion_pins.len() % PIN_TINTS.len()];
            layer.onion_pins.push(OnionPin {
                frame,
                tint,
                visible: true,
            });
        }
    }

    let mut remove = None;
    for (i, pin) in layer.onion_pins.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let eye = if pin.visible { ic::EYE } else { ic::EYE_SLASH };
            if theme::icon_button(ui, eye, "Show / hide this pin").clicked() {
                pin.visible = !pin.visible;
            }
            color_picker_u8(ui, "", &mut pin.tint);
            ui.label("Frame");
            ui.add(egui::DragValue::new(&mut pin.frame).range(0..=count - 1).speed(0.25));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(ui, ic::TRASH, "Remove pin").clicked() {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove {
        layer.onion_pins.remove(i);
    }
    if !state.onion.enabled && !layer.onion_pins.is_empty() {
        ui.label(
            egui::RichText::new("Onion skin is off — pins show when it's enabled.")
                .color(theme::TEXT_MUTED)
                .size(10.5),
        );
    }
}

/// Camera panel — what the export sees, and how that moves over time.
fn camera_content(state: &mut AppState, ui: &mut egui::Ui) {
    ui.label(
        egui::RichText::new(
            "The camera frames the exported video. Layers can sit outside it — park a \
             character off to the side, then key the camera to pan over.",
        )
        .color(theme::TEXT_MUTED)
        .size(10.5),
    );
    ui.add_space(6.0);

    ui.checkbox(&mut state.camera_edit, "Camera edit mode");
    let toggle = combo_text(state, Action::CameraEditToggle);
    ui.label(
        egui::RichText::new(format!(
            "Toggle ({toggle}), then drag with your canvas gesture keys: pan-key = move the \
             camera, zoom-key = push in, rotate-key = roll.",
        ))
        .color(theme::TEXT_MUTED)
        .size(10.5),
    );
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("X");
        ui.add(egui::DragValue::new(&mut state.project.camera.tx).speed(1.0));
        ui.label("Y");
        ui.add(egui::DragValue::new(&mut state.project.camera.ty).speed(1.0));
    });
    ui.horizontal(|ui| {
        ui.label("Zoom");
        ui.add(
            egui::DragValue::new(&mut state.project.camera.zoom)
                .speed(0.01)
                .range(0.05..=64.0),
        );
        ui.label("Roll°");
        let mut deg = state.project.camera.rot.to_degrees();
        if ui.add(egui::DragValue::new(&mut deg).speed(0.5)).changed() {
            state.project.camera.rot = deg.to_radians();
        }
    });

    let cf = state.project.current_frame;
    let nkeys = state.project.camera_keys.len();
    let here = state.project.has_camera_key(cf);
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

    // Ease of the key on this frame. It shapes the segment running *from* this
    // key to the next, which is why it lives on the key rather than the pair.
    let mut ease = state
        .project
        .camera_keys
        .iter()
        .find(|k| k.frame == cf)
        .map(|k| k.ease)
        .unwrap_or_default();
    ui.add_enabled_ui(here, |ui| {
        ui.horizontal(|ui| {
            ui.label("Ease out of key");
            let mut changed = false;
            egui::ComboBox::from_id_salt("cam_ease")
                .selected_text(ease.label())
                .show_ui(ui, |ui| {
                    for e in Ease::ALL {
                        changed |= ui.selectable_value(&mut ease, e, e.label()).changed();
                    }
                });
            if changed {
                state.set_camera_key_ease(ease);
            }
        });
    });

    ui.horizontal(|ui| {
        let add = combo_text(state, Action::CameraKeyAdd);
        if ui
            .button(theme::icon_text(ic::PLUS_SQUARE, &format!("Add key ({add})")))
            .clicked()
        {
            state.add_camera_key();
        }
        if ui.button(theme::icon_text(ic::X, "Del key")).clicked() {
            state.delete_camera_key();
        }
        if ui
            .button(theme::icon_text(ic::ARROW_COUNTER_CLOCKWISE, "Reset"))
            .clicked()
        {
            state.reset_camera();
        }
    });

    ui.add_space(6.0);
    ui.separator();
    theme::section_header(ui, ic::EYE, "View");
    let look = combo_text(state, Action::CameraLookThrough);
    ui.checkbox(
        &mut state.camera_look_through,
        format!("Look through camera ({look})"),
    );
    ui.checkbox(&mut state.show_camera_guide, "Show camera frame");
    ui.checkbox(&mut state.dim_outside_camera, "Dim outside camera");
    ui.checkbox(&mut state.show_layer_bounds, "Show active layer bounds");

    if state.project.camera.zoom > 1.001 {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!(
                "Zoom {:.2}× upscales: the export is fixed at {}×{}. For a clean push-in, \
                 expand the layer canvas and start zoomed out.",
                state.project.camera.zoom, state.project.width, state.project.height
            ))
            .color(theme::TEXT_MUTED)
            .size(10.5),
        );
    }
}

/// How tall the layer list may grow before it starts scrolling, in points.
/// Each row is three lines — name, opacity, fill-boundary link — so this is
/// roughly four layers, which is enough to reorder against without the panel
/// running off the bottom of a laptop screen.
const LAYER_LIST_MAX_H: f32 = 320.0;

/// Drag-and-drop payload for a layer row: the dragged layer's index.
struct LayerDrag(usize);

/// Where layer `from` ends up when dropped just above (`above`) or just below
/// layer `target` in the stack — its final index, as `Project::move_layer`
/// takes it. Dropped onto itself, or into the slot it already sits in, it
/// stays put.
fn drop_index(from: usize, target: usize, above: bool) -> usize {
    if from == target {
        return from;
    }
    // Taking `from` out first shifts everything above it down one.
    match (above, from < target) {
        (true, true) => target,
        (true, false) => target + 1,
        (false, true) => target - 1,
        (false, false) => target,
    }
}

fn layers_content(state: &mut AppState, ui: &mut egui::Ui) {
    {
            ui.horizontal(|ui| {
                let add = tip(state, Action::LayerAdd, "Add layer above the selected one");
                if theme::icon_button(ui, ic::PLUS, &add).clicked() {
                    state.structural_edit(false, |p| {
                        p.add_layer_above_active();
                    });
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
                let fade_key = combo_text(state, Action::FadeOthersToggle);
                if theme::icon_toggle(
                    ui,
                    ic::CIRCLE_HALF,
                    &format!("Fade other layers ({fade_key})"),
                    state.fade_others,
                )
                .clicked()
                {
                    state.fade_others = !state.fade_others;
                }
            });
            // How faint the other layers go. Dragging either slider switches
            // the fade on — adjusting an amount you can't see is guesswork.
            // Folded away with the rest: the toggle above is what gets used.
            egui::CollapsingHeader::new(theme::icon_text(ic::CIRCLE_HALF, "Fade other layers"))
                .id_salt("layers_fade")
                .default_open(false)
                .show(ui, |ui| {
                    let fade_help = "How visible the other layers stay while faded. \
                                     View only: layer opacity and export are unaffected.";
                    let above = ui
                        .add(egui::Slider::new(&mut state.fade.above, 0.0..=1.0).text("fade above"))
                        .on_hover_text(fade_help);
                    let below = ui
                        .add(egui::Slider::new(&mut state.fade.below, 0.0..=1.0).text("fade below"))
                        .on_hover_text(fade_help);
                    if above.changed() || below.changed() {
                        state.fade_others = true;
                    }
                });
            ui.add_space(4.0);
            ui.separator();

            let n = state.project.layers.len();
            let cur = state.project.current_layer;
            let mut select: Option<usize> = None;
            let mut start_rename: Option<usize> = None;
            let mut rename_commit = false;
            let mut rename_cancel = false;
            let mut krita_edit: Option<usize> = None;
            let mut toggle_clip: Option<usize> = None;
            let mut toggle_alpha_lock: Option<usize> = None;
            let mut drop: Option<(usize, usize)> = None;
            let krita_label = krita_menu_label(state);
            let clip_key = combo_text(state, Action::LayerClipToggle);
            let lock_key = combo_text(state, Action::LayerAlphaLockToggle);
            // What each clipped layer clips to — `None` for one clipped with
            // nothing under it to clip to.
            let bases: Vec<Option<usize>> =
                (0..n).map(|i| state.project.clip_base(i)).collect();
            // Owned copy: the "lines from" combo lists every layer's name while
            // a single layer is mutably borrowed below.
            let names: Vec<String> = state
                .project
                .layers
                .iter()
                .map(|l| l.name.clone())
                .collect();
            // Split borrow: rows need &mut layer while the rename edit buffer
            // lives on AppState next to it.
            let (layers, rename) = (&mut state.project.layers, &mut state.layer_rename);
            // The rows scroll rather than growing the window: each one is three
            // lines tall, so a stack of them would otherwise push the Transform
            // section below the bottom of the screen. Only the list scrolls —
            // the toolbar above and Transform below stay put, which is what
            // makes reordering a layer while looking at its transform possible.
            //
            // `auto_shrink` vertically so one or two layers do not reserve the
            // full height as blank space, but never horizontally: the rows
            // should keep the panel's width.
            egui::ScrollArea::vertical()
                .id_salt("layer_rows")
                .max_height(LAYER_LIST_MAX_H)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for i in (0..n).rev() {
                        let layer = &mut layers[i];
                        let selected = i == cur;

                        let row = Frame::none()
                            .fill(if selected {
                                theme::ACCENT_DIM
                            } else {
                                Color32::TRANSPARENT
                            })
                            .rounding(egui::Rounding::same(6.0))
                            .inner_margin(Margin::symmetric(6.0, 4.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // A clipped layer sits indented, an arrow
                                    // pointing down at its base.
                                    if layer.clip {
                                        ui.add_space(8.0);
                                        let (tint, tip) = match bases[i] {
                                            Some(b) => (
                                                theme::ACCENT,
                                                format!("Clipped to {}", names[b]),
                                            ),
                                            None => (
                                                theme::TEXT_MUTED,
                                                "Clipped, but there is nothing under it to clip to"
                                                    .to_string(),
                                            ),
                                        };
                                        ui.label(
                                            egui::RichText::new(ic::ARROW_BEND_LEFT_DOWN)
                                                .size(16.0)
                                                .color(tint),
                                        )
                                        .on_hover_text(tip);
                                    }
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
                                        // The name is also the drag handle. A drag
                                        // only starts once the pointer moves, so a
                                        // click still selects and a double-click
                                        // still renames. One widget senses both:
                                        // a drag-only one laid over the label
                                        // would win the hit test and eat clicks.
                                        let resp = ui
                                            .add(egui::SelectableLabel::new(
                                                selected,
                                                egui::RichText::new(&layer.name).strong(),
                                            ))
                                            .interact(Sense::click_and_drag())
                                            .on_hover_text("Double-click to rename, drag to reorder");
                                        resp.dnd_set_drag_payload(LayerDrag(i));
                                        if resp.dragged() {
                                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                        }
                                        if resp.double_clicked() {
                                            start_rename = Some(i);
                                        } else if resp.clicked() {
                                            select = Some(i);
                                        }
                                        if layer.alpha_lock {
                                            ui.label(
                                                egui::RichText::new(ic::CHECKERBOARD)
                                                    .size(14.0)
                                                    .color(theme::ACCENT),
                                            )
                                            .on_hover_text(format!(
                                                "Alpha locked ({lock_key}): painting only \
                                                 changes pixels that already have paint"
                                            ));
                                        }
                                        resp.context_menu(|ui| {
                                            if ui
                                                .selectable_label(
                                                    layer.clip,
                                                    format!("Clip to layer below ({clip_key})"),
                                                )
                                                .on_hover_text(
                                                    "Show this layer only where the first \
                                                     unclipped layer under it has paint",
                                                )
                                                .clicked()
                                            {
                                                toggle_clip = Some(i);
                                                ui.close_menu();
                                            }
                                            if ui
                                                .selectable_label(
                                                    layer.alpha_lock,
                                                    format!("Lock alpha ({lock_key})"),
                                                )
                                                .on_hover_text(
                                                    "Painting only changes pixels that already \
                                                     have paint",
                                                )
                                                .clicked()
                                            {
                                                toggle_alpha_lock = Some(i);
                                                ui.close_menu();
                                            }
                                            ui.separator();
                                            if ui
                                                .button(theme::icon_text(ic::PAINT_BRUSH, krita_label))
                                                .on_hover_text("Opens the project in Krita with this layer selected")
                                                .clicked()
                                            {
                                                krita_edit = Some(i);
                                                ui.close_menu();
                                            }
                                        });
                                    }
                                });
                                ui.add(egui::Slider::new(&mut layer.opacity, 0.0..=1.0).text("opacity"));
                                // Flood fill on this layer reads its boundaries from
                                // the linked layer — line art above, colour below.
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("lines from")
                                            .color(theme::TEXT_MUTED)
                                            .size(11.0),
                                    );
                                    let current = match layer.lines_from {
                                        Some(s) => names.get(s).map(String::as_str).unwrap_or("—"),
                                        None => "— none —",
                                    };
                                    egui::ComboBox::from_id_salt(("lines_from", i))
                                        .selected_text(egui::RichText::new(current).size(11.0))
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(&mut layer.lines_from, None, "— none —");
                                            for (j, name) in names.iter().enumerate() {
                                                if j == i {
                                                    continue;
                                                }
                                                ui.selectable_value(&mut layer.lines_from, Some(j), name);
                                            }
                                        });
                                })
                                .response
                                .on_hover_text(
                                    "Flood fill on this layer stops at the linked layer's strokes, \
                                     so colour can be painted under line art.",
                                );
                            })
                            .response;
                        // Every row is a drop target: the top half drops the
                        // dragged layer above this one, the bottom half below.
                        if let Some(from) = row.dnd_hover_payload::<LayerDrag>() {
                            let above = ui
                                .input(|inp| inp.pointer.interact_pos())
                                .is_some_and(|p| p.y < row.rect.center().y);
                            let to = drop_index(from.0, i, above);
                            if to != from.0 {
                                let y = if above { row.rect.top() } else { row.rect.bottom() };
                                ui.painter()
                                    .hline(row.rect.x_range(), y, Stroke::new(2.0, theme::ACCENT));
                            }
                            if row.dnd_release_payload::<LayerDrag>().is_some() {
                                drop = Some((from.0, to));
                            }
                        }
                        ui.add_space(2.0);
                    }
            });
            if let Some((from, to)) = drop.filter(|(from, to)| from != to) {
                // One undo step. Both edit buffers below are keyed by index.
                state.structural_edit(false, |p| p.move_layer(from, to));
                state.layer_rename = None;
                state.expand_cfg = None;
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
            if let Some(i) = krita_edit {
                state.edit_in_krita(i);
            }
            if let Some(l) = toggle_clip.and_then(|i| state.project.layers.get_mut(i)) {
                l.clip = !l.clip;
            }
            if let Some(l) = toggle_alpha_lock.and_then(|i| state.project.layers.get_mut(i)) {
                l.alpha_lock = !l.alpha_lock;
            }

            // --- Layer transform ---
            ui.add_space(6.0);
            egui::CollapsingHeader::new(theme::icon_text(ic::RECTANGLE, "Transform"))
                .id_salt("layers_transform")
                .default_open(false)
                .show(ui, |ui| layer_transform_section(state, ui));

            // --- Layer canvas ---
            //
            // Strokes are clipped to the cell buffer, so a layer parked off to
            // the side of the camera still only gets a frame-sized sheet to
            // draw on until it's expanded here.
            egui::CollapsingHeader::new(theme::icon_text(ic::FRAME_CORNERS, "Layer canvas"))
                .id_salt("layers_canvas")
                .default_open(false)
                .show(ui, |ui| layer_canvas_section(state, ui));
    }
}

/// The Layers panel's Transform section: the active layer's pose, its
/// transform keys and their ease.
fn layer_transform_section(state: &mut AppState, ui: &mut egui::Ui) {
    ui.checkbox(&mut state.layer_xform, "Transform mode");
    ui.checkbox(&mut state.auto_key_transform, "Auto-key transform")
        .on_hover_text(
            "Moving, scaling or rotating the active layer sets a \
             transform key on the current frame, so the change animates \
             from here instead of shifting the layer on every \
             frame.\n\nOff (default): the layer moves as a whole until \
             you add a key yourself.",
        );
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
    // Set once an edit *settles*, never on every `changed()`: keying
    // mid-drag would push one undo entry per mouse-move.
    let mut xform_settled = false;
    if let Some(l) = state.project.layers.get_mut(li) {
        let mut settle = |r: &egui::Response| {
            if r.drag_stopped() || r.lost_focus() {
                xform_settled = true;
            }
        };
        ui.horizontal(|ui| {
            ui.label("X");
            settle(&ui.add(egui::DragValue::new(&mut l.transform.tx).speed(1.0)));
            ui.label("Y");
            settle(&ui.add(egui::DragValue::new(&mut l.transform.ty).speed(1.0)));
        });
        ui.horizontal(|ui| {
            ui.label("Scale");
            settle(&ui.add(
                egui::DragValue::new(&mut l.transform.scale)
                    .speed(0.01)
                    .range(0.01..=100.0),
            ));
            ui.label("Rot°");
            let mut deg = l.transform.rot.to_degrees();
            let r = ui.add(egui::DragValue::new(&mut deg).speed(0.5));
            if r.changed() {
                l.transform.rot = deg.to_radians();
            }
            settle(&r);
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
    if xform_settled && state.auto_key_transform {
        state.add_transform_key();
    }

    // Ease of the key on this frame, same contract as the camera's: it
    // shapes the segment running *from* this key to the next.
    let here = state
        .project
        .layers
        .get(li)
        .map(|l| l.has_transform_key(cf))
        .unwrap_or(false);
    let mut ease = state
        .project
        .layers
        .get(li)
        .and_then(|l| l.transform_keys.iter().find(|k| k.frame == cf))
        .map(|k| k.ease)
        .unwrap_or_default();
    ui.add_enabled_ui(here, |ui| {
        ui.horizontal(|ui| {
            ui.label("Ease out of key");
            let mut changed = false;
            egui::ComboBox::from_id_salt("layer_ease")
                .selected_text(ease.label())
                .show_ui(ui, |ui| {
                    for e in Ease::ALL {
                        changed |= ui.selectable_value(&mut ease, e, e.label()).changed();
                    }
                });
            if changed {
                state.set_transform_key_ease(ease);
            }
        });
    });

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

/// The Layers panel's Layer canvas section: grow the active layer's cells
/// past the frame.
fn layer_canvas_section(state: &mut AppState, ui: &mut egui::Ui) {
    let (cur_w, cur_h) = state.active_layer_cell_size();
    let li = state.project.current_layer;
    // One texture per cell, so no side may exceed what the GPU holds
    // in one texture — past it the upload fails outright.
    let max = state.max_tex;
    let (mut ew, mut eh) = match state.expand_cfg {
        Some((l, w, h)) if l == li => (w, h),
        _ => (cur_w, cur_h),
    };
    let size_tip = "Size in pixels. Takes arithmetic: 3840*3, (1920+64)*2.\n\n\
                    Start with an operator to change what's there: *3 triples it, \
                    +512 adds 512, /2 halves it. Enter applies it.";
    ui.horizontal(|ui| {
        /// A pixel-size field that takes arithmetic, as the frame
        /// field does. The base is frozen before the widget is built:
        /// a relative expression measures from where the edit started.
        fn size_field(value: &mut u32, max: u32) -> egui::DragValue<'_> {
            let base = *value as f64;
            egui::DragValue::new(value)
                .speed(8.0)
                .range(1..=max)
                .update_while_editing(false)
                .custom_parser(move |s| expr::eval(s, base).map(f64::round))
        }
        ui.label("W");
        ui.add(size_field(&mut ew, max)).on_hover_text(size_tip);
        ui.label("H");
        ui.add(size_field(&mut eh, max)).on_hover_text(size_tip);
    });
    state.expand_cfg = Some((li, ew, eh));
    ui.horizontal(|ui| {
        for (label, mul) in [("2×", 2u32), ("3×", 3)] {
            if ui.small_button(label).clicked() {
                let (w, h) = (state.project.width * mul, state.project.height * mul);
                state.expand_cfg = Some((li, w.min(max), h.min(max)));
            }
        }
        if ui.small_button("Frame").clicked() {
            state.expand_cfg = Some((li, state.project.width, state.project.height));
        }
    });
    let cells = state.active_layer_cell_count().max(1);
    let mb = (ew as u64 * eh as u64 * 4 * cells as u64) as f64 / (1024.0 * 1024.0);
    ui.label(
        egui::RichText::new(format!(
            "now {cur_w}×{cur_h} · {cells} cell(s) · resize costs {mb:.0} MB"
        ))
        .color(theme::TEXT_MUTED)
        .size(10.5),
    );
    if ew == max || eh == max {
        ui.label(
            egui::RichText::new(format!("GPU limit: {max} px per side"))
                .color(theme::TEXT_MUTED)
                .size(10.5),
        );
    }
    let changed = (ew, eh) != (cur_w, cur_h);
    if ui
        .add_enabled(
            changed,
            egui::Button::new(theme::icon_text(ic::ARROWS_OUT, "Resize layer canvas")),
        )
        .on_hover_text(
            "Re-pads every cell on this layer, keeping the artwork centred. Undoable.",
        )
        .clicked()
    {
        state.expand_active_layer_canvas(ew, eh);
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
                ui.separator();
                // Drawing clipboard: moves one cell between frames or layers.
                // Cut-then-paste is how a drawing gets retimed.
                if theme::icon_button(ui, ic::SCISSORS, &tip(state, Action::CellCut, "Cut drawing"))
                    .clicked()
                {
                    state.cut_cell();
                }
                if theme::icon_button(
                    ui,
                    ic::CLIPBOARD_TEXT,
                    &tip(state, Action::CellCopy, "Copy drawing"),
                )
                .clicked()
                {
                    state.cell_clip = state.project.copy_active_cell();
                }
                let has_clip = state.cell_clip.is_some();
                let paste_tip = tip(state, Action::CellPaste, "Paste drawing");
                ui.add_enabled_ui(has_clip, |ui| {
                    if theme::icon_button(ui, ic::CLIPBOARD, &paste_tip).clicked() {
                        state.paste_cell();
                    }
                });
            });
            ui.checkbox(&mut state.auto_key_draw, "Auto-key drawing")
                .on_hover_text(
                    "Drawing on a held frame starts a new blank key on that \
                     frame first, so you can keep drawing frame after frame \
                     without inserting keys by hand.\n\nOff (default): the \
                     stroke edits the cell shared by every frame in the \
                     hold.\n\nThe previous drawing stays visible through onion \
                     skin, and an auto-keyed stroke takes two undos — one for \
                     the stroke, one for the key.",
                );
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
                                    // The sheet is frames × layers, so unlike the
                                    // frame strip it can show every layer's
                                    // transform keys, not just the active one.
                                    let label = if state.project.layers[li].has_transform_key(f) {
                                        format!("{label} ◆")
                                    } else {
                                        label
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

/// Brush shape, response and smoothing.
///
/// Split out of the brush panel because it outgrew it: the panel is a docked
/// 232-pixel column holding the handful of controls reached mid-drawing —
/// colour, size, opacity — and a dozen sliders below them pushed those off the
/// top. These are the ones set once and left alone.
fn brush_settings_window(state: &mut AppState, ctx: &egui::Context) {
    if !state.show_brush_settings {
        return;
    }
    let mut open = state.show_brush_settings;
    egui::Window::new(theme::icon_text(ic::PAINT_BRUSH, "Brush settings"))
        // Pinned, for the same reason as the Settings window: egui derives a
        // window's persisted position from its title, so a rename would strand
        // it off-screen.
        .id(egui::Id::new("window_brush_settings"))
        .open(&mut open)
        .default_pos([260.0, 120.0])
        .default_width(300.0)
        .resizable(true)
        .collapsible(true)
        .frame(floating_frame())
        .show(ctx, |ui| {
            drag_by_title_only(ui);
            egui::ScrollArea::vertical().show(ui, |ui| {
                theme::section_header(ui, ic::PEN_NIB, "Preset");
                brush_presets(state, ui);

                ui.add_space(6.0);
                theme::section_header(ui, ic::SLIDERS, "Dynamics");
                brush_dynamics(state, ui);

                ui.add_space(6.0);
                theme::section_header(ui, ic::SCRIBBLE, "Smoothing");
                smoothing_controls(state, ui);

                ui.add_space(6.0);
                egui::CollapsingHeader::new(theme::icon_text(ic::PEN, "Tablet"))
                    .default_open(false)
                    .show(ui, |ui| tablet_diagnostics(state, ui));
            });
        });
    state.show_brush_settings = open;
}

fn settings_window(state: &mut AppState, ctx: &egui::Context) {
    if !state.show_settings {
        return;
    }
    let mut open = state.show_settings;
    egui::Window::new(theme::icon_text(ic::GEAR, "Settings"))
        // Pinned: without it the window's persisted position is derived from
        // its title, so renaming it would strand the window off-screen.
        .id(egui::Id::new("window_settings"))
        .open(&mut open)
        .default_pos([360.0, 80.0])
        .default_width(420.0)
        .resizable(true)
        .collapsible(true)
        .frame(floating_frame())
        .show(ctx, |ui| {
            drag_by_title_only(ui);
            ui.checkbox(&mut state.invert_timeline_scroll, "Invert timeline scroll")
                .on_hover_text(
                    "Mouse wheel over the canvas or the timeline scrubs frames.\n\n\
                     Off (default): wheel down advances.  On: wheel up advances.\n\n\
                     Each notch moves by the timeline's step size (×N).",
                );
            ui.add_space(4.0);
            // Pen barrel buttons arrive as these same two buttons.
            egui::Grid::new("button_drags")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    for (label, slot) in [
                        ("Middle button drag", &mut state.middle_drag),
                        ("Right button drag", &mut state.right_drag),
                    ] {
                        ui.label(label);
                        egui::ComboBox::from_id_salt(label)
                            .selected_text(slot.label())
                            .width(160.0)
                            .show_ui(ui, |ui| {
                                for b in ButtonDrag::ALL {
                                    ui.selectable_value(slot, b, b.label())
                                        .on_hover_text(b.hint());
                                }
                            })
                            .response
                            .on_hover_text(slot.hint());
                        ui.end_row();
                    }
                });
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);
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
                                if action.is_drag_modifier() {
                                    ui.label(action.label());
                                    let cur = state.shortcuts.get(action);
                                    let cur_label = cur
                                        .map(|c| c.display())
                                        .unwrap_or_else(|| "(none)".to_string());
                                    egui::ComboBox::from_id_salt(("navmod", action))
                                        .selected_text(cur_label.as_str())
                                        .width(140.0)
                                        .show_ui(ui, |ui| {
                                            let choices = crate::input::shortcuts::DRAG_MODIFIERS
                                                .iter()
                                                .map(|&(c, s, a)| Some(KeyCombo::modifier_only(c, s, a)))
                                                .chain(std::iter::once(None));
                                            for combo in choices {
                                                let lbl = combo
                                                    .map(|c| c.display())
                                                    .unwrap_or_else(|| "(none)".to_string());
                                                if ui
                                                    .selectable_label(cur_label == lbl, lbl.as_str())
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

fn color_picker_u8(ui: &mut egui::Ui, label: &str, c: &mut [u8; 3]) {
    ui.horizontal(|ui| {
        color_wheel::hsl_edit_button(ui, c);
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

/// Perspective tool options: the switches every grid shares, and the grid
/// list. A grid's own settings open in a window from its row — see
/// [`grid_settings_window`] — so the list stays short enough to work from.
fn perspective_options(state: &mut AppState, ui: &mut egui::Ui) {
    use crate::tools::perspective::{GridKind, PerspectiveGrid};

    ui.label(
        egui::RichText::new(
            "Drag a corner to reshape, a vanishing point to re-aim, the horizon to raise \
             the eye level or its round knobs to tilt it, just outside a corner to rotate \
             (hold Shift once dragging for 15° steps), inside to move.",
        )
        .color(theme::TEXT_MUTED)
        .size(11.0),
    );
    let show = combo_text(state, Action::TogglePerspectiveGrid);
    let snap = combo_text(state, Action::TogglePerspectiveSnap);
    ui.checkbox(&mut state.perspective.show, format!("Show grids with every tool ({show})"));
    ui.add_enabled(
        state.perspective.show,
        egui::Checkbox::new(&mut state.perspective.snap, format!("Snap strokes ({snap})")),
    )
    .on_hover_text(
        "Pencil, ink, eraser and shape lines lock to the active grid, toward whichever \
         vanishing point the stroke starts out heading for. Rectangles and ellipses lie \
         on it; hold Shift once dragging for a square or circle.",
    );
    let snapping = state.perspective.show && state.perspective.snap;
    ui.add_enabled(
        snapping,
        egui::Checkbox::new(&mut state.perspective.snap_vertical, "Vertical lines too"),
    )
    .on_hover_text(
        "Also snap to the vertical (square to the horizon) — for building edges. \
         Near the middle of a one-point grid it competes with the columns.",
    );
    ui.add_enabled(
        snapping,
        egui::Checkbox::new(&mut state.perspective.cursor_guides, "Cursor guides"),
    )
    .on_hover_text("Faint lines from the cursor along every way a stroke could snap");

    ui.checkbox(&mut state.perspective.auto_key, "Auto-key").on_hover_text(
        "Once a grid has keys, moving it — on the canvas or in its settings — keys \
         this frame",
    );
    ui.checkbox(&mut state.perspective.ghosts, "Ghost neighbour keys")
        .on_hover_text("Faint outlines where the keys either side put the active grid");

    ui.add_space(4.0);
    let cfg = &mut state.perspective;
    let mut open_settings = false;
    let mut remove = None;
    for i in 0..cfg.grids.len() {
        let selected = i == cfg.active;
        let parent_no = cfg.grids[i]
            .wall
            .and_then(|w| cfg.index_of(w.parent))
            .map(|p| p + 1);
        Frame::none()
            .fill(if selected { theme::ACCENT_DIM } else { Color32::TRANSPARENT })
            .rounding(egui::Rounding::same(6.0))
            .inner_margin(Margin::symmetric(6.0, 2.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let g = &mut cfg.grids[i];
                    let eye = if g.visible { ic::EYE } else { ic::EYE_SLASH };
                    if theme::icon_button(ui, eye, "Toggle visibility").clicked() {
                        g.visible = !g.visible;
                    }
                    let lock = if g.locked { ic::LOCK_SIMPLE } else { ic::LOCK_SIMPLE_OPEN };
                    if theme::icon_button(ui, lock, "Toggle lock").clicked() {
                        g.locked = !g.locked;
                    }
                    let [r, gr, b] = g.color;
                    let kind = match (g.kind, parent_no) {
                        (_, Some(p)) => format!(" · wall on {p}"),
                        (GridKind::Perspective, None) => String::new(),
                        (k, None) => format!(" · {}", k.label().to_lowercase()),
                    };
                    let keyed = if g.keys.is_empty() { "" } else { " ◆" };
                    let label = format!("Grid {}{kind}{keyed}", i + 1);
                    let name = ui
                        .selectable_label(selected, label)
                        .on_hover_text("Double-click for its settings");
                    if name.clicked() {
                        cfg.active = i;
                    }
                    if name.double_clicked() {
                        cfg.active = i;
                        open_settings = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::icon_button(ui, ic::TRASH, "Delete grid").clicked() {
                            remove = Some(i);
                        }
                        if theme::icon_button(ui, ic::GEAR, "Grid settings").clicked() {
                            cfg.active = i;
                            open_settings = true;
                        }
                        // Colour swatch, so grids on the canvas can be matched
                        // to their rows here.
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
                        ui.painter().rect_filled(rect, 3.0, Color32::from_rgb(r, gr, b));
                        ui.painter().rect_stroke(
                            rect,
                            3.0,
                            Stroke::new(1.0, Color32::from_black_alpha(120)),
                        );
                    });
                });
            });
    }
    if let Some(i) = remove {
        cfg.remove(i);
        cfg.relink();
    }
    ui.horizontal(|ui| {
        ui.menu_button(theme::icon_text(ic::PLUS, "Add"), |ui| {
            for (kind, icon, tip) in [
                (GridKind::Perspective, ic::PERSPECTIVE, "A floor in perspective"),
                (GridKind::Flat, ic::GRID_FOUR, "A square grid over the whole canvas"),
                (GridKind::Isometric, ic::CUBE, "An isometric grid over the whole canvas"),
            ] {
                if ui.button(theme::icon_text(icon, kind.label())).on_hover_text(tip).clicked() {
                    cfg.push(PerspectiveGrid::fresh(kind));
                    ui.close_menu();
                }
            }
        });
        let dup = cfg.active_grid().cloned();
        if ui
            .add_enabled(dup.is_some(), egui::Button::new(theme::icon_text(ic::COPY, "Duplicate")))
            .clicked()
        {
            if let Some(mut g) = dup {
                // Nudged so the copy doesn't sit invisibly on the original,
                // and free: a second wall on the same edge would be the same
                // wall.
                g.translate([0.03, 0.03]);
                for k in &mut g.keys {
                    k.pose.corners = k.pose.corners.map(|c| [c[0] + 0.03, c[1] + 0.03]);
                }
                g.locked = false;
                g.wall = None;
                cfg.push(g);
            }
        }
    });

    if open_settings {
        state.show_grid_settings = true;
    }
}

/// The active grid's own settings: its shape and pose, its lines, what
/// strokes snap to on it, its vanishing points and walls, and how it moves
/// over the shot. A floating window opened from the grid's row, and like the
/// brush settings it stays up while you draw. It follows the active grid, so
/// picking another row — or Alt+G — switches what it shows.
fn grid_settings_window(state: &mut AppState, ctx: &egui::Context) {
    if !state.show_grid_settings {
        return;
    }
    let Some(_) = state.perspective.active_grid() else {
        // Nothing left to show: the last grid went.
        state.show_grid_settings = false;
        return;
    };
    let title = format!("Grid {} settings", state.perspective.active + 1);
    let mut open = true;
    egui::Window::new(theme::icon_text(ic::GEAR, &title))
        // Pinned: egui keys a window's remembered position on its title, and
        // this one's changes with the grid.
        .id(egui::Id::new("window_grid_settings"))
        .open(&mut open)
        .default_pos([260.0, 140.0])
        .default_width(300.0)
        .resizable(true)
        .collapsible(true)
        .frame(floating_frame())
        .show(ctx, |ui| {
            drag_by_title_only(ui);
            egui::ScrollArea::vertical().show(ui, |ui| grid_settings(state, ui));
        });
    state.show_grid_settings &= open;
}

/// The body of [`grid_settings_window`], for the active grid.
fn grid_settings(state: &mut AppState, ui: &mut egui::Ui) {
    use crate::tools::perspective::{
        Follow, GridKind, PerspectiveGrid, EDGE_NAMES, MAX_DIVISIONS, MAX_EXTRA_VPS,
    };

    // Grids are stored in frame heights; the X / Y fields show document px.
    let frame_h = (state.project.height as f32).max(1.0);
    let frame = state.project.current_frame;
    let space = state
        .perspective
        .active_grid()
        .map(|g| state.grid_space(g, frame));
    let layer_names: Vec<String> = state.project.layers.iter().map(|l| l.name.clone()).collect();
    // Edits that need the whole state, made once the grid lets go of it.
    let mut pose_edited = false;
    let mut follow_to: Option<Follow> = None;
    let mut wall_on: Option<u8> = None;
    let mut key_add = false;
    let mut key_del = false;
    let mut ease_to: Option<Ease> = None;

    let cfg = &mut state.perspective;
    let active = cfg.active;
    let parent_no = cfg
        .active_grid()
        .and_then(|g| g.wall)
        .and_then(|w| cfg.index_of(w.parent))
        .map(|p| p + 1);
    let Some(g) = cfg.active_grid_mut() else {
        return;
    };
    let wall = g.is_wall();
    let perspective = g.kind == GridKind::Perspective;
    let what = match (g.kind, parent_no) {
        (_, Some(_)) => "A wall".to_string(),
        (k, None) => format!("{} grid", k.label()),
    };
    theme::section_header(ui, ic::PERSPECTIVE, &what);
    ui.horizontal(|ui| {
        ui.label("Columns");
        ui.add(egui::DragValue::new(&mut g.cols).range(1..=MAX_DIVISIONS));
        ui.label("Rows");
        ui.add(egui::DragValue::new(&mut g.rows).range(1..=MAX_DIVISIONS));
    });
    // The grid's pose, laid out like a layer's: X / Y from the frame centre,
    // scale against a fresh grid, heading in degrees. Read off the corners
    // each frame, and an edit applies as the change from what was read. A
    // wall's pose is its floor's to set.
    ui.add_enabled_ui(!g.locked && !wall, |ui| {
        ui.horizontal(|ui| {
            let c = g.centre();
            let (mut x, mut y) = (c[0] * frame_h, c[1] * frame_h);
            ui.label("X");
            let rx = ui.add(egui::DragValue::new(&mut x).speed(1.0).max_decimals(1));
            ui.label("Y");
            let ry = ui.add(egui::DragValue::new(&mut y).speed(1.0).max_decimals(1));
            if rx.changed() || ry.changed() {
                g.translate([x / frame_h - c[0], y / frame_h - c[1]]);
                pose_edited = true;
            }
        });
        ui.horizontal(|ui| {
            let s0 = g.scale();
            let mut s = s0;
            ui.label("Scale");
            if ui
                .add(egui::DragValue::new(&mut s).speed(0.01).max_decimals(2))
                .changed()
            {
                // A scale at or below zero is refused by `scale_by`.
                g.scale_by(s / s0);
                pose_edited = true;
            }
            let d0 = g.angle().to_degrees();
            let mut d = d0;
            ui.label("Rot°");
            if ui
                .add(egui::DragValue::new(&mut d).speed(0.5).max_decimals(1))
                .changed()
            {
                g.rotate((d - d0).to_radians());
                pose_edited = true;
            }
        });
        ui.horizontal(|ui| {
            if theme::icon_button(ui, ic::ARROW_COUNTER_CLOCKWISE, "Rotate 15° left").clicked() {
                g.rotate(-std::f32::consts::PI / 12.0);
                pose_edited = true;
            }
            if theme::icon_button(ui, ic::ARROW_CLOCKWISE, "Rotate 15° right").clicked() {
                g.rotate(std::f32::consts::PI / 12.0);
                pose_edited = true;
            }
            if ui
                .button("Reset shape")
                .on_hover_text("Back to a fresh grid's shape, keeping rows, columns and look")
                .clicked()
            {
                g.corners = PerspectiveGrid::fresh(g.kind).corners;
                pose_edited = true;
            }
        });
    });
    ui.horizontal(|ui| {
        color_wheel::hsl_edit_button(ui, &mut g.color);
        if ui
            .add(egui::Slider::new(&mut g.opacity, 0.05..=1.0).text("Opacity"))
            .changed()
        {
            pose_edited = true;
        }
    });
    ui.add(egui::Slider::new(&mut g.weight, 0.5..=4.0).text("Line weight"));

    ui.add_space(6.0);
    theme::section_header(ui, ic::LINE_SEGMENTS, "Lines");
    ui.horizontal(|ui| {
        ui.label("Major line every");
        ui.add(egui::DragValue::new(&mut g.major_every).range(0..=MAX_DIVISIONS))
            .on_hover_text("Draw every Nth line heavier. 0 or 1: none.");
    });
    if perspective {
        ui.checkbox(&mut g.infinite, "Infinite floor")
            .on_hover_text("Keep tiling rows and columns past the grid, out to the horizon");
        let extend = egui::Checkbox::new(&mut g.extend, "Extend lines to vanishing points");
        ui.add_enabled(!g.infinite, extend);
        ui.checkbox(&mut g.diagonals, "Cell diagonals")
            .on_hover_text("Both diagonals of every cell, for finding centres in perspective");
        ui.checkbox(&mut g.centre_lines, "Centre lines");
        ui.add_enabled(!wall, egui::Checkbox::new(&mut g.horizon, "Horizon and vanishing points"));
    }

    ui.add_space(6.0);
    theme::section_header(ui, ic::MAGNET, "Snap strokes to");
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut g.snap_rows, "Rows");
        ui.checkbox(&mut g.snap_cols, "Columns");
        if g.kind == GridKind::Isometric {
            ui.checkbox(&mut g.snap_third, "Verticals");
        }
        for (k, v) in g.extra_vps.iter_mut().enumerate() {
            ui.checkbox(&mut v.snap, format!("VP {}", k + 3));
        }
    });

    // Up to two more vanishing points, numbered after the plane's own two.
    if perspective && !wall {
        ui.add_space(6.0);
        theme::section_header(ui, ic::CROSSHAIR, "Vanishing points");
        ui.add_enabled_ui(!g.locked, |ui| {
            let room = g.extra_vps.len() < MAX_EXTRA_VPS;
            if ui
                .add_enabled(room, egui::Button::new(theme::icon_text(ic::PLUS, "Vanishing point")))
                .on_hover_text(
                    "Another point strokes can snap toward. On the horizon it slides along \
                     it — for a box turned another way on the same floor. Pull it well off \
                     the horizon for a third, vertical vanishing point; bring it back and it \
                     sticks again. Walls rise toward the first one off the horizon.",
                )
                .clicked()
            {
                if let Some(s) = &space {
                    g.add_extra_vp(s);
                }
            }
            let shown = space.map(|s| g.extra_doc(&s)).unwrap_or_default();
            let mut drop = None;
            for (k, v) in g.extra_vps.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("VP {}", k + 3));
                    let was = v.on_horizon;
                    ui.checkbox(&mut v.on_horizon, "On horizon");
                    // Letting go of the horizon keeps the point where it shows.
                    if was && !v.on_horizon {
                        if let (Some(s), Some(&at)) = (&space, shown.get(k)) {
                            v.pos = s.to_rel(at);
                        }
                    }
                    ui.checkbox(&mut v.rays, "Rays");
                    if theme::icon_button(ui, ic::TRASH, "Remove this vanishing point").clicked() {
                        drop = Some(k);
                    }
                });
            }
            if let Some(k) = drop {
                g.remove_extra_vp(k);
            }
        });

        ui.add_space(6.0);
        theme::section_header(ui, ic::CUBE, "Walls");
        ui.horizontal_wrapped(|ui| {
            ui.label("Stand one on").on_hover_text(
                "Stand a grid on an edge of this one, rising to the first vanishing \
                 point off the horizon — or straight up without one. It stays on the \
                 edge as this grid moves; drag its top corners to set its height.",
            );
            for (e, name) in EDGE_NAMES.iter().enumerate() {
                if ui.small_button(*name).clicked() {
                    wall_on = Some(e as u8);
                }
            }
        });
    }
    if let (Some(link), Some(p)) = (g.wall, parent_no) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!(
                    "Stands on grid {p}'s {} edge",
                    EDGE_NAMES[link.edge as usize & 3].to_lowercase()
                ))
                .color(theme::TEXT_MUTED)
                .size(11.0),
            );
            let unlink = theme::icon_button(ui, ic::LINK_BREAK, "Unlink: a free grid from here on");
            if unlink.clicked() {
                g.wall = None;
            }
        });
    }

    ui.add_space(6.0);
    theme::section_header(ui, ic::FILM_STRIP, "Motion");
    if wall {
        ui.label(
            egui::RichText::new("A wall moves with its floor.")
                .color(theme::TEXT_MUTED)
                .size(11.0),
        );
    } else {
        let follow_name = match &g.follow {
            Follow::Document => "Document".to_string(),
            Follow::Camera => "Camera".to_string(),
            Follow::Layer { name, .. } => format!("Layer: {name}"),
        };
        ui.horizontal(|ui| {
            ui.label("Follows").on_hover_text(
                "What carries the grid along. Document: it stays on the drawing as the \
                 camera moves. Camera: it stays put in the shot. A layer: it moves with \
                 that layer's transform. Its own keys add on top.",
            );
            egui::ComboBox::from_id_salt("grid_follow")
                .selected_text(follow_name)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(g.follow == Follow::Document, "Document").clicked() {
                        follow_to = Some(Follow::Document);
                    }
                    if ui.selectable_label(g.follow == Follow::Camera, "Camera").clicked() {
                        follow_to = Some(Follow::Camera);
                    }
                    for (li, name) in layer_names.iter().enumerate().rev() {
                        let on = matches!(&g.follow, Follow::Layer { index, .. } if *index == li);
                        if ui.selectable_label(on, format!("Layer: {name}")).clicked() {
                            follow_to = Some(Follow::Layer {
                                name: name.clone(),
                                index: li,
                                uid: 0,
                            });
                        }
                    }
                });
        });

        let here = g.has_key(frame);
        let status = match g.keys.len() {
            0 => "No keys: the grid holds still".to_string(),
            n => format!("{n} key(s){}", if here { " — keyed on this frame" } else { "" }),
        };
        ui.label(egui::RichText::new(status).color(theme::TEXT_MUTED).size(10.5));
        // Ease of the key on this frame: it shapes the segment running from
        // it to the next, as on camera keys.
        let mut ease = g.key_at(frame).map(|k| k.ease).unwrap_or_default();
        ui.add_enabled_ui(here, |ui| {
            ui.horizontal(|ui| {
                ui.label("Ease out of key");
                egui::ComboBox::from_id_salt("grid_ease")
                    .selected_text(ease.label())
                    .show_ui(ui, |ui| {
                        for e in Ease::ALL {
                            if ui.selectable_value(&mut ease, e, e.label()).changed() {
                                ease_to = Some(e);
                            }
                        }
                    });
            });
        });
        ui.horizontal(|ui| {
            key_add = ui
                .button(theme::icon_text(ic::PLUS_SQUARE, "Add key"))
                .on_hover_text("Key the grid where it is now, on this frame")
                .clicked();
            key_del = ui
                .add_enabled(here, egui::Button::new(theme::icon_text(ic::X, "Del key")))
                .clicked();
        });
    }
    if pose_edited {
        state.grid_pose_edited(active);
    }
    if let Some(mut f) = follow_to {
        if let Follow::Layer { index, .. } = f {
            f = state.follow_layer(index).unwrap_or(Follow::Document);
        }
        state.set_grid_follow(active, f);
    }
    if let Some(e) = wall_on {
        state.perspective.add_wall(active, e);
    }
    if let Some(e) = ease_to {
        state.set_grid_key_ease(e);
    }
    if key_add {
        state.add_grid_key();
    }
    if key_del {
        state.delete_grid_key();
    }
}

/// The Lasso's options: what a drag draws, how it combines with the
/// selection already there, and the commands that act on the selection.
fn selection_options(state: &mut AppState, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for (shape, icon) in [
            (SelShape::Freehand, ic::LASSO),
            (SelShape::Rect, ic::RECTANGLE_DASHED),
            (SelShape::Ellipse, ic::CIRCLE_DASHED),
            (SelShape::Polygon, ic::POLYGON),
        ] {
            if theme::icon_toggle(ui, icon, shape.label(), state.sel_shape == shape).clicked() {
                // A half-placed polygon means nothing to the other shapes.
                if shape != state.sel_shape {
                    state.cancel_gesture();
                }
                state.sel_shape = shape;
            }
        }
    });
    ui.horizontal(|ui| {
        for (op, icon, bind) in [
            (SelOp::Replace, ic::SELECTION, None),
            (SelOp::Add, ic::UNITE, Some(Action::SelModeAdd)),
            (SelOp::Subtract, ic::SUBTRACT, Some(Action::SelModeSubtract)),
            (SelOp::Intersect, ic::INTERSECT, Some(Action::SelModeIntersect)),
        ] {
            let held = bind
                .and_then(|a| state.shortcuts.get(a))
                .map(|c| format!(" — or hold {} as you drag", c.display()))
                .unwrap_or_default();
            let tip = format!("{}{held}", op.label());
            if theme::icon_toggle(ui, icon, &tip, state.sel_op == op).clicked() {
                state.sel_op = op;
            }
        }
    });
    ui.checkbox(&mut state.sel_all_frames, "All frames").on_hover_text(
        "Moving, scaling or rotating the selection lands on every drawing on this \
         layer, through the same selection. Draw it around the object's whole path — \
         onion skin shows where it goes.",
    );
    if state.sel_all_frames {
        let n = state.layer_drawings(state.project.current_layer).len();
        let what = if n == 1 { "drawing" } else { "drawings" };
        ui.label(
            egui::RichText::new(format!("Applies to {n} {what} on this layer"))
                .color(theme::TEXT_MUTED)
                .size(11.0),
        );
    }
    ui.horizontal(|ui| {
        let all = tip(state, Action::SelectAll, "Select all");
        if theme::icon_button(ui, ic::SELECTION_ALL, &all).clicked() {
            state.select_all();
        }
        let inv = tip(state, Action::SelectInvert, "Invert selection");
        if theme::icon_button(ui, ic::SELECTION_INVERSE, &inv).clicked() {
            state.select_invert();
        }
        let off = tip(state, Action::SelectionDeselect, "Deselect (Esc)");
        if ui
            .add_enabled(
                state.sel_mask.is_some(),
                egui::Button::new(egui::RichText::new(ic::SELECTION_SLASH).size(15.0))
                    .min_size(egui::vec2(28.0, 22.0)),
            )
            .on_hover_text(off)
            .clicked()
        {
            state.deselect();
        }
        let fill = tip(state, Action::SelectionFill, "Fill with brush colour");
        if ui
            .add_enabled(
                state.sel_mask.is_some(),
                egui::Button::new(egui::RichText::new(ic::PAINT_BUCKET).size(15.0))
                    .min_size(egui::vec2(28.0, 22.0)),
            )
            .on_hover_text(fill)
            .clicked()
        {
            state.fill_selection();
        }
    });
    ui.horizontal(|ui| {
        let on = state.sel_mask.is_some();
        ui.add(
            egui::DragValue::new(&mut state.sel_amount)
                .range(1..=200)
                .suffix(" px"),
        );
        if ui.add_enabled(on, egui::Button::new("Grow")).clicked() {
            state.grow_selection();
        }
        if ui.add_enabled(on, egui::Button::new("Shrink")).clicked() {
            state.shrink_selection();
        }
        if ui.add_enabled(on, egui::Button::new("Feather")).clicked() {
            state.feather_selection();
        }
    });
    ui.checkbox(&mut state.tint_outside, "Tint outside selection");
    let del = combo_text(state, Action::SelectionDelete);
    let cut = combo_text(state, Action::SelectionCut);
    let copy = combo_text(state, Action::SelectionCopy);
    let paste = combo_text(state, Action::SelectionPaste);
    ui.label(
        egui::RichText::new(format!(
            "Every stroke and fill stays inside the selection. Drag inside it to              move the pixels, or nudge with the arrow keys; Enter puts them down.              {del} erases inside it - {cut} / {copy} / {paste} move it between              frames and layers.",
        ))
        .color(theme::TEXT_MUTED)
        .size(11.0),
    );
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
        state.set_tool(target);
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
    // Left alone during screen-pick: the picker no longer alters the backdrop,
    // so hiding the checker would be the same unwanted change by another route.
    if state.show_checker && state.view.rotation.abs() < 1e-3 {
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
    // Corners for a texture holding `r` of layer `layer_idx`'s cell `id` (see
    // `CellTex`), or None for a blank one, which draws nothing.
    let tex_corners = |layer_idx: usize, id: usize, r: DirtyRect| -> Option<[egui::Pos2; 4]> {
        if r.max_x <= r.min_x || r.max_y <= r.min_y {
            return None;
        }
        let cell = state.project.cell(id)?;
        Some(cell_rect_screen_corners(
            &xf,
            state.display_transform(layer_idx, cur_frame),
            cell.width as f32,
            cell.height as f32,
            pw,
            ph,
            [r.min_x as f32, r.min_y as f32, r.max_x as f32, r.max_y as f32],
        ))
    };

    // Onion ghosts of the active layer at nearby frames. Drawn inside the layer
    // loop so they sit at the active layer's depth, all of them just behind
    // its cell — never over the lines being drawn.
    // Ghosts come from `ghost_textures` — silhouettes already baked in the
    // tint colour. The vertex colour only fades them: multiplying a tint over
    // the plain cell texture leaves black line art black.
    let draw_ghost = |id: CellId, frame: usize, a: u8| {
        let Some((_, tex)) = state.ghost_textures.get(&id) else {
            return;
        };
        let Some(cell) = state.project.cell(id) else {
            return;
        };
        let t = state.display_transform(cur_layer, frame);
        let lc = layer_screen_corners(&xf, t, cell.width as f32, cell.height as f32, pw, ph);
        image_quad(&painter, tex.id(), lc, theme::white_alpha(a));
    };
    let draw_onion = || {
        // Pins at the bottom: they're references, usually far off. Then the
        // range ghosts of both sides, farthest first, so the nearest ones — the
        // most opaque, and what the user is comparing against — sit right
        // under the drawing.
        let pin_alpha = (state.onion.max_alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        for (pin, id) in state.onion_pin_ghosts() {
            draw_ghost(id, pin.frame, pin_alpha);
        }
        let dirs = [OnionDirection::Prev, OnionDirection::Next];
        let mut steps: Vec<(OnionStep, OnionDirection)> = dirs
            .into_iter()
            .flat_map(|dir| state.onion_steps(dir).into_iter().map(move |s| (s, dir)))
            .collect();
        steps.sort_by_key(|(s, _)| std::cmp::Reverse(s.k));
        for (step, dir) in steps {
            draw_ghost(step.cell, step.frame, state.onion.alpha_for(step.k, dir));
        }
    };

    for (li, layer) in state.project.layers.iter().enumerate() {
        if !layer.visible || !layer.reference {
            continue;
        }
        if let Some(id) = layer.resolve(cur_frame) {
            let tex = state.cell_textures.get(&id);
            if let Some((t, lc)) = tex.and_then(|t| Some((t, tex_corners(li, id, t.rect)?))) {
                let dim = (layer.opacity * 0.45).clamp(0.0, 1.0);
                let a = (dim * 255.0) as u8;
                image_quad(
                    &painter,
                    t.tex.id(),
                    lc,
                    theme::white_alpha(a),
                );
            }
        }
    }

    for (li, layer) in state.project.layers.iter().enumerate() {
        if !layer.visible || layer.reference {
            continue;
        }
        // Onion ghosts render at the active layer's depth, just behind its
        // current cell.
        if li == cur_layer {
            draw_onion();
        }
        // A clipped layer draws its drawing cut to its base.
        if let Some((id, tex, rect)) = state.layer_texture(li) {
            if let Some(lc) = tex_corners(li, id, rect) {
                let op = layer.opacity * state.layer_view_alpha(li);
                let a = (op.clamp(0.0, 1.0) * 255.0) as u8;
                image_quad(
                    &painter,
                    tex.id(),
                    lc,
                    theme::white_alpha(a),
                );
            }
        }
    }

    // Dim what is not selected. Over the artwork, under every overlay.
    if state.tint_outside && state.sel_mask.is_some() {
        if let Some(t) = &state.sel_tint {
            let (x0, y0, x1, y1) = t.rect;
            let q = [
                xf.doc_to_screen(x0, y0),
                xf.doc_to_screen(x1, y0),
                xf.doc_to_screen(x1, y1),
                xf.doc_to_screen(x0, y1),
            ];
            image_quad(&painter, t.tex.id(), q, Color32::WHITE);
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
                let col = theme::premul(col.r(), col.g(), col.b(), alpha);
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
    // Falls back to the active layer's cell size, not the project size: a
    // floating selection has no `stroke_target` between drags, and a layer with
    // an expanded canvas would then have its float and outline mapped through
    // the wrong dimensions.
    let (preview_cw, preview_ch) = state
        .stroke_target
        .and_then(|id| state.project.cell(id))
        .map(|c| (c.width as f32, c.height as f32))
        .unwrap_or_else(|| {
            let (w, h) = state.project.draw_cell_size(cur_layer, cur_frame);
            (w as f32, h as f32)
        });
    let cell_to_screen = |u: f32, v: f32| -> egui::Pos2 {
        let (dx, dy) = preview_xf.cell_to_doc(u, v, preview_cw, preview_ch, pw, ph);
        xf.doc_to_screen(dx, dy)
    };
    // On-screen pixel size scales with both the view zoom and the layer scale.
    let layer_scale = preview_xf.scale.abs();

    // Live-tail overlay: the committed stroke is already streamed into the
    // cell texture via partial uploads, so only the short uncommitted span
    // between the last rasterized spine node and the cursor needs an overlay
    // (it hides the one-sample Catmull-Rom commit lag). Not for a Krita brush:
    // a solid band would be nothing like its dabs, and Krita shows no tail.
    if let Some(builder) = state.stroke.as_ref().filter(|b| b.brush.mode != BrushMode::Krita) {
        if let (Some((tail, _)), Some(cur)) = (builder.live_tail(), builder.current_node()) {
            let is_eraser = builder.tool == crate::tools::ActiveTool::Eraser;
            let a = builder.brush.opacity.clamp(0.0, 1.0);
            // Premultiplied in gamma space to match the CPU compositor (see
            // `theme::premul`).
            let gamma_premul = |c: [u8; 3], a: u8| theme::premul(c[0], c[1], c[2], a);
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
        let col = Color32::from_rgb(c[0], c[1], c[2]);
        let thick = (state.effective_radius() * 2.0 * scale * layer_scale).max(1.0);
        let stroke = Stroke::new(thick, col);
        // The same outline pointer-up rasterises, grid and Shift included.
        let mut pts: Vec<egui::Pos2> = state
            .shape_outline(&drag)
            .into_iter()
            .map(|(x, y)| cell_to_screen(x, y))
            .collect();
        match (drag.kind, pts.len()) {
            (_, 0) => {}
            (_, 1) => {
                painter.circle_filled(pts[0], thick * 0.5, col);
            }
            (ShapeKind::Line, _) => {
                painter.add(egui::Shape::line(pts, stroke));
            }
            (ShapeKind::Rect | ShapeKind::Ellipse, _) => {
                // Closed outlines repeat their first point; `closed_line`
                // joins the ends itself.
                pts.pop();
                painter.add(egui::Shape::closed_line(pts, stroke));
            }
        }
    }

    // Marching ants: animated dashes, which is what tells a selection outline
    // apart from an inked line. Only repainted a few times a second — a
    // selection can stay up for as long as the user likes.
    let phase = (ui.input(|i| i.time) * 24.0) as f32 % 12.0;
    let mut ants_shown = false;

    // Floating selection: the lifted pixels as a quad under their pose, plus
    // marching ants around the mask so it reads as "selected", not "drawn",
    // and the transform box that scales and rotates it.
    if let Some(sel) = &state.selection {
        // The pose is applied to the quad's corners rather than baked into the
        // texture, so the GPU does the scaling and rotation for free and the
        // lifted pixels are never resampled until the selection commits.
        let corners = sel.corners();
        let box_pts: Vec<egui::Pos2> = corners
            .iter()
            .map(|&(x, y)| cell_to_screen(x, y))
            .collect();
        if let Some(tex) = &state.selection_tex {
            image_quad(
                &painter,
                tex.id(),
                [box_pts[0], box_pts[1], box_pts[2], box_pts[3]],
                Color32::WHITE,
            );
        }
        for outline in &sel.outline {
            let pts: Vec<egui::Pos2> = outline
                .iter()
                .map(|&(x, y)| {
                    let (cx, cy) = sel.path_point(x, y);
                    cell_to_screen(cx, cy)
                })
                .collect();
            marching_ants(&painter, &pts, phase);
        }
        ants_shown = true;

        // Transform box: a thin outline plus the eight handles, drawn at a
        // fixed *screen* size so they stay grabbable however far out the view
        // is zoomed — which is the same size `grab_at` tests against.
        painter.add(egui::Shape::closed_line(
            box_pts,
            Stroke::new(1.0, Color32::from_black_alpha(120)),
        ));
        let r = crate::tools::selection::HANDLE_PX;
        for (hu, hv) in SelGrab::handle_positions(sel.mask.w as f32, sel.mask.h as f32) {
            let (cx, cy) = sel.buf_to_cell(hu, hv);
            let rect = egui::Rect::from_center_size(
                cell_to_screen(cx, cy),
                egui::vec2(r * 2.0, r * 2.0),
            );
            painter.rect_filled(rect, 1.0, Color32::WHITE);
            painter.rect_stroke(rect, 1.0, Stroke::new(1.0, Color32::from_black_alpha(200)));
        }
    } else if let Some(mask) = &state.sel_mask {
        // The selection itself, in document space. Past a couple of hundred
        // thousand vertices the outline costs more than it tells; its box
        // says where it is instead.
        let total: usize = state.sel_outline.iter().map(Vec::len).sum();
        if total <= 200_000 {
            for outline in &state.sel_outline {
                let pts: Vec<egui::Pos2> = outline
                    .iter()
                    .map(|&(x, y)| xf.doc_to_screen(x, y))
                    .collect();
                marching_ants(&painter, &pts, phase);
            }
        } else if let Some((x0, y0, x1, y1)) = mask.bounds() {
            let (x0, y0, x1, y1) = (x0 as f32, y0 as f32, x1 as f32, y1 as f32);
            let pts = [
                xf.doc_to_screen(x0, y0),
                xf.doc_to_screen(x1, y0),
                xf.doc_to_screen(x1, y1),
                xf.doc_to_screen(x0, y1),
            ];
            marching_ants(&painter, &pts, phase);
        }
        // Everything beyond the outline is selected too (Select All, or an
        // inverted selection): the frame edge says so.
        if mask.outside == 255 {
            marching_ants(&painter, &corners, phase);
        }
        ants_shown = true;

        // The transform box a plain drag would pick up — the handles that
        // scale and rotate the selected pixels before any have moved.
        let boxed = state.tool == crate::tools::ActiveTool::Lasso
            && state.sel_op == SelOp::Replace
            && state.sel_gesture.is_none()
            && state.mask_is_grabbable();
        if let Some(m) = state.cached_cell_mask().filter(|m| boxed && m.w > 0 && m.h > 0) {
            let (mx, my) = (m.x as f32, m.y as f32);
            let (w, h) = (m.w as f32, m.h as f32);
            let box_pts = vec![
                cell_to_screen(mx, my),
                cell_to_screen(mx + w, my),
                cell_to_screen(mx + w, my + h),
                cell_to_screen(mx, my + h),
            ];
            painter.add(egui::Shape::closed_line(
                box_pts,
                Stroke::new(1.0, Color32::from_black_alpha(120)),
            ));
            let r = crate::tools::selection::HANDLE_PX;
            for (hu, hv) in SelGrab::handle_positions(w, h) {
                let rect = egui::Rect::from_center_size(
                    cell_to_screen(mx + hu, my + hv),
                    egui::vec2(r * 2.0, r * 2.0),
                );
                painter.rect_filled(rect, 1.0, theme::white_alpha(200));
                painter.rect_stroke(rect, 1.0, Stroke::new(1.0, Color32::from_black_alpha(160)));
            }
        }
    }
    if ants_shown {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(60));
    }

    // The selection being drawn, in document space like the selection itself.
    if let Some(g) = &state.sel_gesture {
        let to_screen = |&(x, y): &(f32, f32)| xf.doc_to_screen(x, y);
        match g {
            SelGesture::Drag {
                shape,
                pts,
                start,
                end,
                ..
            } => {
                let path: Vec<egui::Pos2> = match shape {
                    SelShape::Rect => select_mask::rect_path(*start, *end)
                        .iter()
                        .map(to_screen)
                        .collect(),
                    SelShape::Ellipse => select_mask::ellipse_path(*start, *end)
                        .iter()
                        .map(to_screen)
                        .collect(),
                    SelShape::Freehand | SelShape::Polygon => pts.iter().map(to_screen).collect(),
                };
                gesture_path(&painter, &path, true);
            }
            SelGesture::Polygon { pts, pressed, .. } => {
                let mut path: Vec<egui::Pos2> = pts.iter().map(to_screen).collect();
                let hover = ui
                    .input(|i| i.pointer.hover_pos())
                    .filter(|p| rect.contains(*p));
                // Between clicks the next edge rubber-bands to the pointer.
                if let (false, Some(h)) = (*pressed, hover) {
                    path.push(h);
                }
                gesture_path(&painter, &path, true);
                for p in pts.iter().map(to_screen) {
                    painter.circle_filled(p, 2.5, Color32::WHITE);
                    painter.circle_stroke(p, 2.5, Stroke::new(1.0, Color32::from_black_alpha(200)));
                }
                // Ring the first corner when a click there would close it.
                let close = crate::tools::selection::HANDLE_PX * 1.5;
                if let (Some(first), Some(h)) = (pts.first().map(to_screen), hover) {
                    if pts.len() >= 3 && first.distance(h) <= close {
                        painter.circle_stroke(first, close, Stroke::new(1.5, theme::ACCENT));
                    }
                }
            }
        }
    }

    // --- Camera guide + layer bounds ---
    //
    // The document rect is no longer "the drawable area" — it is just where a
    // resting camera looks. Layers can sit anywhere in doc space, so the guide
    // is what tells the user which slab of their world the export will contain.
    let cam = state.display_camera(cur_frame);
    let cam_corners = layer_screen_corners(&xf, cam.as_frame_transform(), pw, ph, pw, ph);

    // Faint doc-rect outline. Skipped when the bright guide would land on the
    // exact same pixels.
    let outline_a = (state.bg_opacity * 180.0) as u8;
    if outline_a > 0 && !(state.show_camera_guide && cam.is_identity()) {
        painter.add(egui::Shape::closed_line(
            corners.to_vec(),
            Stroke::new(1.0, theme::premul(80, 80, 80, outline_a)),
        ));
    }

    // Active layer's cell bounds. Strokes past this edge are silently dropped
    // by the rasterizer, so without an outline they just vanish. Only worth
    // drawing once the layer has moved or grown away from the doc rect.
    if state.show_layer_bounds {
        let interesting = state
            .project
            .layers
            .get(cur_layer)
            .map(|l| {
                (l.cell_w, l.cell_h) != (0, 0) || !state.display_transform(cur_layer, cur_frame).is_identity()
            })
            .unwrap_or(false);
        let resolved = state
            .project
            .layers
            .get(cur_layer)
            .and_then(|l| l.resolve(cur_frame));
        if interesting {
            if let Some(c) = resolved.and_then(|id| cell_corners(cur_layer, id)) {
                painter.add(egui::Shape::closed_line(
                    c.to_vec(),
                    Stroke::new(1.0, theme::premul(120, 160, 220, 110)),
                ));
            }
        }
    }

    // A mirrored view is easy to forget and expensive to forget: you can draw a
    // whole scene backwards. Say so, always, in the corner of the canvas.
    if state.view.flip_x || state.view.flip_y {
        let axes = match (state.view.flip_x, state.view.flip_y) {
            (true, true) => "FLIPPED H+V",
            (true, false) => "FLIPPED H",
            _ => "FLIPPED V",
        };
        let at = rect.left_top() + Vec2::new(10.0, 8.0);
        let galley = painter.layout_no_wrap(
            axes.to_owned(),
            egui::FontId::proportional(11.0),
            Color32::from_rgb(20, 20, 24),
        );
        let pad = Vec2::new(6.0, 3.0);
        let chip = Rect::from_min_size(at, galley.size() + pad * 2.0);
        painter.rect_filled(chip, 3.0, Color32::from_rgb(255, 190, 90));
        painter.galley(at + pad, galley, Color32::PLACEHOLDER);
    }

    if state.show_camera_guide {
        if state.dim_outside_camera {
            fill_outside_quad(&painter, rect, cam_corners, Color32::from_black_alpha(110));
        }
        let accent = if state.camera_edit {
            Color32::from_rgb(255, 190, 90)
        } else {
            Color32::from_rgb(235, 235, 245)
        };
        // Two-tone, like the lasso preview: readable over ink and over empty
        // canvas alike.
        painter.add(egui::Shape::closed_line(
            cam_corners.to_vec(),
            Stroke::new(2.6, Color32::from_black_alpha(150)),
        ));
        painter.add(egui::Shape::closed_line(
            cam_corners.to_vec(),
            Stroke::new(1.2, accent),
        ));
        // Corner ticks, so the quad reads as a camera frame against busy art.
        let edge = |a: egui::Pos2, b: egui::Pos2| (b - a).length();
        let tick = (edge(cam_corners[0], cam_corners[1]).min(edge(cam_corners[1], cam_corners[2]))
            * 0.08)
            .clamp(4.0, 28.0);
        for i in 0..4 {
            let c = cam_corners[i];
            for n in [cam_corners[(i + 1) % 4], cam_corners[(i + 3) % 4]] {
                let d = n - c;
                let len = d.length();
                if len > 1e-3 {
                    painter.line_segment([c, c + d / len * tick], Stroke::new(2.6, accent));
                }
            }
        }
    }

    draw_perspective_grids(state, &painter, &xf, rect);
}

/// The perspective grids, over everything else: guides, not artwork.
///
/// Shown while the perspective tool is active, or everywhere once "Show
/// grids" is on. The active grid also gets its handles while the tool is
/// active — sized in screen pixels like the selection's, which is the size
/// `grab_at` tests against — and faint outlines where its keys either side
/// put it.
fn draw_perspective_grids(state: &AppState, painter: &egui::Painter, xf: &Xform, clip: Rect) {
    use crate::tools::perspective::{clip_line, tile_family, tilt_knobs, vp_rays, Family, Plane, Vp};

    let editing = state.tool == ActiveTool::Perspective;
    if !editing && !state.perspective.show {
        return;
    }
    let frame = state.project.current_frame;
    let to_screen = |p: [f32; 2]| xf.doc_to_screen(p[0], p[1]);
    let to_screen_p = |p: [f32; 2]| {
        let s = xf.doc_to_screen(p[0], p[1]);
        [s.x, s.y]
    };
    let pos = |p: [f32; 2]| egui::pos2(p[0], p[1]);
    let lo = [clip.min.x, clip.min.y];
    let hi = [clip.max.x, clip.max.y];
    let at = |a: egui::Pos2, b: egui::Pos2, t: f32| a + (b - a) * t;
    let handle = crate::tools::selection::HANDLE_PX;

    for (i, g) in state.perspective.grids.iter().enumerate() {
        if !g.visible {
            continue;
        }
        let active = i == state.perspective.active;
        let space = state.grid_space(g, frame);
        let corners = g.doc_corners(&space);
        let Some(plane) = Plane::new(corners) else {
            continue;
        };
        // Inactive grids recede while one is being edited, and while strokes
        // snap — so the grid they snap to is the one that stands out.
        let fade = if (editing || state.perspective.snap) && !active {
            0.45
        } else {
            1.0
        };
        let alpha = (g.opacity.clamp(0.0, 1.0) * fade * 255.0) as u8;
        let tint = |a: u8| theme::premul(g.color[0], g.color[1], g.color[2], a);
        let color = tint(alpha);
        let weight = g.weight.max(0.25);
        let thin = Stroke::new(weight, color);
        let faint = Stroke::new(weight * 0.8, tint(alpha / 3));
        let major = |k: i32| g.major_every > 1 && k.rem_euclid(g.major_every as i32) == 0;
        let heavy = Stroke::new(weight + 0.8, color);
        let heavy_faint = Stroke::new(weight * 0.8 + 0.6, tint(alpha / 2));

        // Where the keys either side put it: a dashed outline and the key's
        // frame, so a tween can be seen coming.
        if editing && active && state.perspective.ghosts {
            let (prev, next) = g.neighbour_keys(frame);
            for k in [prev, next].into_iter().flatten() {
                let s = state.grid_space(g, k.frame);
                let mut ring: Vec<egui::Pos2> =
                    k.pose.corners.iter().map(|&n| to_screen(s.to_doc(n))).collect();
                ring.push(ring[0]);
                painter.extend(egui::Shape::dashed_line(
                    &ring,
                    Stroke::new(1.0, tint(alpha / 2)),
                    6.0,
                    4.0,
                ));
                painter.text(
                    ring[0] + egui::vec2(-4.0, -4.0),
                    egui::Align2::RIGHT_BOTTOM,
                    format!("{}", k.frame),
                    egui::FontId::proportional(10.5),
                    tint(alpha.max(120)),
                );
            }
        }

        let quad: Vec<egui::Pos2> = corners.iter().map(|&c| to_screen(c)).collect();
        if g.kind.rigid() {
            // A flat or isometric grid covers the canvas; its quad is just
            // the home cell, outlined so there is something to take hold of.
            let gap = 6.0;
            for fam in g.families() {
                for t in tile_family(&plane, fam, &to_screen_p, lo, hi, gap) {
                    let s = if major(t.k) { heavy } else { thin };
                    painter.line_segment([pos(t.a), pos(t.b)], s);
                }
            }
            painter.add(egui::Shape::closed_line(quad.clone(), Stroke::new(weight + 1.0, color)));
        } else {
            let lines = plane.grid_lines(g.rows, g.cols);
            if g.infinite {
                // The whole floor, out to the horizon and the canvas edge,
                // under the quad's own lines.
                for fam in [Family::rows(g.rows), Family::cols(g.cols)] {
                    for t in tile_family(&plane, fam, &to_screen_p, lo, hi, 4.0) {
                        let s = if major(t.k) { heavy_faint } else { faint };
                        painter.line_segment([pos(t.a), pos(t.b)], s);
                    }
                }
            } else if g.extend {
                // Carry each line out to its vanishing point — and no
                // further, so the rays converge rather than crossing — or to
                // the canvas edge when it has none.
                let vp_screen = |vp: Vp| match vp {
                    Vp::Point(p) => Some(to_screen(p)),
                    Vp::Dir(_) => None,
                };
                for &(a, b, vp, _) in &lines {
                    let (sa, sb) = (to_screen(a), to_screen(b));
                    let Some((mut t0, mut t1)) = clip_line([sa.x, sa.y], [sb.x, sb.y], lo, hi)
                    else {
                        continue;
                    };
                    if let Some(v) = vp_screen(vp) {
                        let d = sb - sa;
                        let dd = d.length_sq();
                        if dd > 1e-6 {
                            let tv = (v - sa).dot(d) / dd;
                            if tv > 1.0 {
                                t1 = t1.min(tv);
                            } else if tv < 0.0 {
                                t0 = t0.max(tv);
                            }
                        }
                    }
                    if t0 < 0.0 {
                        painter.line_segment([at(sa, sb, t0), sa], faint);
                    }
                    if t1 > 1.0 {
                        painter.line_segment([sb, at(sa, sb, t1)], faint);
                    }
                }
            }
            if g.diagonals {
                for fam in [Family::diag(g.rows, g.cols), Family::anti_diag(g.rows, g.cols)] {
                    for (_, a, b) in plane.quad_family(fam) {
                        painter.line_segment([to_screen(a), to_screen(b)], faint);
                    }
                }
            }
            for &(a, b, _, k) in &lines {
                let s = if major(k as i32) { heavy } else { thin };
                painter.line_segment([to_screen(a), to_screen(b)], s);
            }
            if g.centre_lines {
                for (a, b) in [((0.5, 0.0), (0.5, 1.0)), ((0.0, 0.5), (1.0, 0.5))] {
                    let ends = [to_screen(plane.h.map(a.0, a.1)), to_screen(plane.h.map(b.0, b.1))];
                    painter.extend(egui::Shape::dashed_line(&ends, heavy, 8.0, 5.0));
                }
            }
            painter.add(egui::Shape::closed_line(quad.clone(), Stroke::new(weight + 1.0, color)));
        }

        let vp_screen = |vp: Vp| match vp {
            Vp::Point(p) => Some(to_screen(p)),
            Vp::Dir(_) => None,
        };
        let horizon = plane.horizon().filter(|_| g.horizon && !g.kind.rigid());
        if let Some(hz) = horizon {
            let a = to_screen(hz.p);
            // A second point a good way along, in document space: one pixel
            // apart, the direction would be at the mercy of rounding once the
            // view is zoomed in.
            let b = to_screen([hz.p[0] + hz.d[0] * 100.0, hz.p[1] + hz.d[1] * 100.0]);
            if let Some((t0, t1)) = clip_line([a.x, a.y], [b.x, b.y], lo, hi) {
                let line = Stroke::new(weight + 0.5, color);
                painter.line_segment([at(a, b, t0), at(a, b, t1)], line);
            }
            for v in [plane.vp_rows, plane.vp_cols].into_iter().filter_map(vp_screen) {
                if clip.contains(v) {
                    painter.circle_filled(v, 3.5, color);
                    painter.circle_stroke(v, 3.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
                }
            }
        }

        // The extra vanishing points: their rays under everything they
        // mark, then a diamond each — round is taken by the plane's own.
        let extras = g.extra_doc(&space);
        let ray = Stroke::new(weight * 0.8, tint(alpha / 2));
        for (v, &at) in g.extra_vps.iter().zip(&extras) {
            if v.rays {
                for (a, b) in vp_rays(&corners, at, g.rows.max(g.cols)) {
                    painter.line_segment([to_screen(a), to_screen(b)], ray);
                }
            }
        }
        let handles = editing && active && !g.locked && !g.is_wall();
        for (k, &at) in extras.iter().enumerate() {
            let v = to_screen(at);
            if !clip.contains(v) || !(g.horizon || handles) {
                continue;
            }
            let r = if handles { handle * 1.4 } else { 4.5 };
            let diamond = vec![
                v + egui::vec2(0.0, -r),
                v + egui::vec2(r, 0.0),
                v + egui::vec2(0.0, r),
                v + egui::vec2(-r, 0.0),
            ];
            let (fill, edge) = if handles {
                (Color32::WHITE, Color32::from_black_alpha(200))
            } else {
                (color, Color32::from_black_alpha(160))
            };
            painter.add(egui::Shape::convex_polygon(diamond, fill, Stroke::new(1.0, edge)));
            if handles {
                painter.circle_filled(v, 2.0, color);
                // Numbered after the plane's own two, as the panel lists them.
                painter.text(
                    v + egui::vec2(r + 3.0, -r),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{}", k + 3),
                    egui::FontId::proportional(11.0),
                    Color32::WHITE,
                );
            }
        }

        if editing && active {
            let r = handle;
            for (k, &c) in quad.iter().enumerate() {
                let rect = egui::Rect::from_center_size(c, egui::vec2(r * 2.0, r * 2.0));
                // A wall's base belongs to its floor: only its top corners
                // take hold.
                let held = g.locked || (g.is_wall() && k >= 2);
                if held {
                    painter.rect_stroke(rect, 1.0, Stroke::new(1.0, color));
                } else {
                    painter.rect_filled(rect, 1.0, Color32::WHITE);
                    painter.rect_stroke(rect, 1.0, Stroke::new(1.0, Color32::from_black_alpha(200)));
                }
            }
            if handles && !g.kind.rigid() {
                // Vanishing-point handles: round, to tell them from the
                // corners. Sized to the 1.5x tolerance `grab_at` gives them.
                for v in [plane.vp_rows, plane.vp_cols].into_iter().filter_map(vp_screen) {
                    if clip.contains(v) {
                        painter.circle_filled(v, r * 1.3, Color32::WHITE);
                        painter.circle_stroke(v, r * 1.3, Stroke::new(1.0, Color32::from_black_alpha(200)));
                        painter.circle_filled(v, 2.0, color);
                    }
                }
                // The eye level: a tick where it turns, and a knob either
                // side to turn it by.
                if let Some(hz) = horizon {
                    let tol = r / state.view_scale.max(1e-6);
                    let p = to_screen(hz.p);
                    let n = egui::vec2(-hz.d[1], hz.d[0]) * 5.0;
                    painter.line_segment([p - n, p + n], Stroke::new(2.0, Color32::WHITE));
                    for k in tilt_knobs(hz, tol) {
                        let k = to_screen(k);
                        if clip.contains(k) {
                            painter.circle_filled(k, r, color);
                            painter.circle_stroke(k, r, Stroke::new(1.5, Color32::WHITE));
                        }
                    }
                }
            }
        }
    }
}

/// While a stroke would snap, faint dashed lines from the cursor along every
/// way it could go: through it to each vanishing point, or right across the
/// canvas for a family that stays parallel.
fn draw_cursor_guides(
    state: &AppState,
    painter: &egui::Painter,
    xf: &Xform,
    clip: Rect,
    cursor: egui::Pos2,
) {
    use crate::tools::perspective::{clip_line, Vp};

    let Some(g) = state.perspective.active_grid() else {
        return;
    };
    let s = state.grid_space(g, state.project.current_frame);
    let (x, y) = xf.screen_to_doc(cursor);
    let stroke = Stroke::new(1.0, theme::premul(g.color[0], g.color[1], g.color[2], 150));
    let (lo, hi) = ([clip.min.x, clip.min.y], [clip.max.x, clip.max.y]);
    for target in g.snap_targets(&s, state.perspective.snap_vertical) {
        let (b, to_vp) = match target {
            Vp::Point(v) => (xf.doc_to_screen(v[0], v[1]), true),
            Vp::Dir(d) => (xf.doc_to_screen(x + d[0] * 100.0, y + d[1] * 100.0), false),
        };
        let a = cursor;
        if (b - a).length() < 1.0 {
            continue;
        }
        let Some((t0, mut t1)) = clip_line([a.x, a.y], [b.x, b.y], lo, hi) else {
            continue;
        };
        // Past its vanishing point a line means nothing.
        if to_vp {
            t1 = t1.min(1.0);
        }
        if t1 > t0 {
            let ends = [a + (b - a) * t0, a + (b - a) * t1];
            painter.extend(egui::Shape::dashed_line(&ends, stroke, 6.0, 4.0));
        }
    }
}

/// Fill everything *outside* the convex quad `inner` (TL, TR, BR, BL) with
/// `color`, by triangulating the ring between it and a much larger outer quad.
///
/// The outer quad is built in the *inner quad's own frame*, not axis-aligned:
/// the ring is only well formed while corner `i` of the outer quad stays
/// angularly beside corner `i` of the inner one, and an axis-aligned outer
/// loses that pairing once the camera rolls past ~45°, folding the ring
/// segments into bowties that paint as dark diagonal bands. Sizing it along
/// the inner quad's axes keeps the pairing exact at every roll angle.
///
/// It is inflated past the clip rect, so the ring always properly contains
/// `inner` — however the camera is rotated, zoomed, or pushed off screen. The
/// painter's own clip trims the excess, and because it is one ring (not four
/// overlapping slabs) the corners don't double-darken.
fn fill_outside_quad(painter: &egui::Painter, clip: Rect, inner: [egui::Pos2; 4], color: Color32) {
    use egui::epaint::{Vertex, WHITE_UV};

    let center =
        ((inner[0].to_vec2() + inner[1].to_vec2() + inner[2].to_vec2() + inner[3].to_vec2()) / 4.0)
            .to_pos2();
    let (ax, ay) = (inner[1] - inner[0], inner[3] - inner[0]);
    let (lx, ly) = (ax.length(), ay.length());
    // A degenerate quad (zero-sized cell) has no axes to borrow; fall back to
    // screen axes, where any pairing is as good as any other.
    let (u, v) = if lx > 1e-3 && ly > 1e-3 {
        (ax / lx, ay / ly)
    } else {
        (Vec2::X, Vec2::Y)
    };

    // Extent along those axes of everything the ring must cover.
    let (mut min_u, mut max_u) = (f32::MAX, f32::MIN);
    let (mut min_v, mut max_v) = (f32::MAX, f32::MIN);
    for p in [
        clip.left_top(),
        clip.right_top(),
        clip.right_bottom(),
        clip.left_bottom(),
    ]
    .iter()
    .chain(inner.iter())
    {
        let d = *p - center;
        let (a, b) = (d.dot(u), d.dot(v));
        min_u = min_u.min(a);
        max_u = max_u.max(a);
        min_v = min_v.min(b);
        max_v = max_v.max(b);
    }
    let pad = clip.width() + clip.height() + 64.0;
    let (min_u, max_u) = (min_u - pad, max_u + pad);
    let (min_v, max_v) = (min_v - pad, max_v + pad);
    // Same corner order as `inner`: (−u,−v), (+u,−v), (+u,+v), (−u,+v).
    let c = |a: f32, b: f32| center + u * a + v * b;
    let outer = [
        c(min_u, min_v),
        c(max_u, min_v),
        c(max_u, max_v),
        c(min_u, max_v),
    ];

    let mut mesh = egui::Mesh::default();
    for p in outer.iter().chain(inner.iter()) {
        mesh.vertices.push(Vertex {
            pos: *p,
            uv: WHITE_UV,
            color,
        });
    }
    for i in 0..4u32 {
        let (a, b) = (i, (i + 1) % 4);
        // outer[a] outer[b] inner[b] / outer[a] inner[b] inner[a]
        mesh.indices
            .extend_from_slice(&[a, b, b + 4, a, b + 4, a + 4]);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Canvas view transform: fit-to-window base scale combined with the user's
/// zoom / pan / rotation. Shared by rendering and input so they stay in sync.
#[derive(Clone, Copy)]
struct Xform {
    center: egui::Pos2,
    pan: Vec2,
    scale: f32,
    /// Mirror signs (±1), applied *before* the rotation. Kept out of `scale`
    /// deliberately: `scale` is clamped positive and is republished as
    /// `view_scale`, which sizes the brush — a negative there would be clamped
    /// away in one place and taken literally in another.
    fx: f32,
    fy: f32,
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
        // "Look through camera" is not a second render path: locking the view
        // to the camera collapses into this same (scale, rot, pan) triple,
        // because the camera is a similarity transform like the view is. That
        // keeps `screen_to_doc` an exact inverse, so drawing still works while
        // locked.
        let (scale, rotation, pan) = if state.camera_look_through {
            let cam = state.display_camera(state.project.current_frame);
            let s = (base * cam.zoom).max(1e-6);
            let (sn, cs) = (-cam.rot).sin_cos();
            let pan = Vec2::new(
                -(cam.tx * cs - cam.ty * sn) * s,
                -(cam.tx * sn + cam.ty * cs) * s,
            );
            (s, -cam.rot, pan)
        } else {
            (
                (base * state.view.zoom).max(1e-6),
                state.view.rotation,
                state.view.pan,
            )
        };
        let sign = |on: bool| if on { -1.0 } else { 1.0 };
        Self::from_parts(
            rect.center(),
            pan,
            scale,
            sign(state.view.flip_x),
            sign(state.view.flip_y),
            rotation,
            cw,
            ch,
        )
    }

    /// The maths half of [`Xform::new`], without an `AppState` — so the
    /// screen/doc round trip can be unit-tested across flips and rotations.
    // The argument list *is* the transform: bundling it into a struct would
    // just be `Xform` with extra steps.
    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        center: egui::Pos2,
        pan: Vec2,
        scale: f32,
        fx: f32,
        fy: f32,
        rotation: f32,
        cw: f32,
        ch: f32,
    ) -> Self {
        let (rot_sin, rot_cos) = rotation.sin_cos();
        Self {
            center,
            pan,
            scale,
            fx,
            fy,
            rot_sin,
            rot_cos,
            dcx: cw * 0.5,
            dcy: ch * 0.5,
            cw,
            ch,
        }
    }

    fn doc_to_screen(&self, x: f32, y: f32) -> egui::Pos2 {
        let ox = (x - self.dcx) * self.scale * self.fx;
        let oy = (y - self.dcy) * self.scale * self.fy;
        let rx = ox * self.rot_cos - oy * self.rot_sin;
        let ry = ox * self.rot_sin + oy * self.rot_cos;
        self.center + self.pan + Vec2::new(rx, ry)
    }

    /// A screen-space drag delta expressed in document pixels — the rotation,
    /// mirror and scale part of `screen_to_doc`, without the translation.
    fn screen_delta_to_doc(&self, d: Vec2) -> (f32, f32) {
        (
            (d.x * self.rot_cos + d.y * self.rot_sin) / (self.scale * self.fx),
            (-d.x * self.rot_sin + d.y * self.rot_cos) / (self.scale * self.fy),
        )
    }

    fn screen_to_doc(&self, p: egui::Pos2) -> (f32, f32) {
        let v = p - self.center - self.pan;
        let rx = v.x * self.rot_cos + v.y * self.rot_sin;
        let ry = -v.x * self.rot_sin + v.y * self.rot_cos;
        (
            rx / (self.scale * self.fx) + self.dcx,
            ry / (self.scale * self.fy) + self.dcy,
        )
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

/// Two-tone animated dashes around a closed loop of screen points — the
/// marching ants. Black and white offset by one dash, so they read over any
/// artwork.
fn marching_ants(painter: &egui::Painter, pts: &[egui::Pos2], phase: f32) {
    if pts.len() < 2 {
        return;
    }
    let mut closed = pts.to_vec();
    closed.push(pts[0]);
    painter.add(egui::Shape::dashed_line_with_offset(
        &closed,
        Stroke::new(1.6, Color32::from_black_alpha(190)),
        &[6.0],
        &[6.0],
        phase,
    ));
    painter.add(egui::Shape::dashed_line_with_offset(
        &closed,
        Stroke::new(1.6, Color32::WHITE),
        &[6.0],
        &[6.0],
        phase + 6.0,
    ));
}

/// A selection shape being drawn: a two-tone line, readable over both ink and
/// empty canvas, plus a faint chord back to the start when `closing`, so it's
/// obvious the loop seals itself.
fn gesture_path(painter: &egui::Painter, pts: &[egui::Pos2], closing: bool) {
    if pts.len() < 2 {
        return;
    }
    painter.add(egui::Shape::line(
        pts.to_vec(),
        Stroke::new(2.2, Color32::from_black_alpha(180)),
    ));
    painter.add(egui::Shape::line(pts.to_vec(), Stroke::new(1.0, Color32::WHITE)));
    if closing {
        let (first, last) = (pts[0], pts[pts.len() - 1]);
        painter.line_segment([last, first], Stroke::new(2.2, Color32::from_black_alpha(120)));
        painter.line_segment([last, first], Stroke::new(1.0, theme::white_alpha(140)));
    }
}

/// The selection mode a held modifier asks for, if any.
pub(crate) fn held_sel_op(state: &AppState, i: &egui::InputState) -> Option<SelOp> {
    [
        (Action::SelModeAdd, SelOp::Add),
        (Action::SelModeSubtract, SelOp::Subtract),
        (Action::SelModeIntersect, SelOp::Intersect),
    ]
    .into_iter()
    .find(|(a, _)| state.shortcuts.get(*a).is_some_and(|c| c.mods_held(i)))
    .map(|(_, op)| op)
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
    cell_rect_screen_corners(xf, t, cw, ch, pw, ph, [0.0, 0.0, cw, ch])
}

/// The same for just the `[x0, y0, x1, y1]` part of the cell, in cell
/// pixels — where a texture cropped to its drawing goes.
// As with `Xform::from_parts`, the arguments are the mapping itself.
#[allow(clippy::too_many_arguments)]
fn cell_rect_screen_corners(
    xf: &Xform,
    t: crate::doc::transform::Transform,
    cw: f32,
    ch: f32,
    pw: f32,
    ph: f32,
    r: [f32; 4],
) -> [egui::Pos2; 4] {
    let [x0, y0, x1, y1] = r;
    let pts = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    let mut out = [egui::Pos2::ZERO; 4];
    for (i, (u, v)) in pts.iter().enumerate() {
        let (dx, dy) = t.cell_to_doc(*u, *v, cw, ch, pw, ph);
        out[i] = xf.doc_to_screen(dx, dy);
    }
    out
}

/// A small value readout beside the pen at `pos`: white text on a dark
/// backing, so it reads over any drawing.
fn draw_readout(painter: &egui::Painter, pos: egui::Pos2, text: String) {
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(12.0), Color32::WHITE);
    let at = pos + Vec2::new(14.0, 12.0);
    let back = Rect::from_min_size(at, galley.size()).expand2(Vec2::new(5.0, 2.0));
    painter.rect_filled(back, 3.0, theme::premul(0, 0, 0, 170));
    painter.galley(at, galley, Color32::WHITE);
}

/// The swatch pie around the press: the pinned swatches in two rings, the one
/// under the pen pulled out, and the current colour in the cancel spot, so
/// letting go there reads as "keep this one". On the foreground layer, above
/// any panel the pen wanders over.
fn draw_swatch_pie(
    ctx: &egui::Context,
    palette: &[[u8; 3]],
    d: &crate::app::ButtonDragState,
    current: [u8; 4],
) {
    use button_drag::{pie_rings, PIE_HOLE, PIE_OUTER, PIE_RING};
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("swatch_pie"),
    ));
    let c = d.anchor;
    let (inner, outer) = pie_rings(palette.len());
    let edge = if outer > 0 { PIE_OUTER } else { PIE_RING };
    painter.circle_filled(c, edge + 4.0, theme::premul(20, 20, 22, 210));
    let quiet = Stroke::new(1.0, theme::premul(0, 0, 0, 160));
    let hot = Stroke::new(2.0, Color32::WHITE);
    // A hair of space between wedges, as an angle at each ring's middle.
    let spacing = |r: f32| 1.5 / r;
    for (first, count, r0, r1) in [
        (0, inner, PIE_HOLE + 3.0, PIE_RING - 1.5),
        (inner, outer, PIE_RING + 1.5, PIE_OUTER),
    ] {
        if count == 0 {
            continue;
        }
        let step = std::f32::consts::TAU / count as f32;
        let pad = spacing((r0 + r1) * 0.5);
        for k in 0..count {
            let i = first + k;
            let on = d.pie_hover == Some(i);
            let mid = k as f32 * step;
            let [r, g, b] = palette[i];
            pie_wedge(
                &painter,
                c,
                (r0, r1 + if on { 8.0 } else { 0.0 }),
                (mid - step * 0.5 + pad, mid + step * 0.5 - pad),
                Color32::from_rgb(r, g, b),
                if on { hot } else { quiet },
            );
        }
    }
    let [r, g, b, _] = current;
    let keep = if d.pie_hover.is_none() { hot } else { quiet };
    painter.circle_filled(c, PIE_HOLE - 4.0, Color32::from_rgb(r, g, b));
    painter.circle_stroke(c, PIE_HOLE - 4.0, keep);
    if palette.is_empty() {
        draw_readout(&painter, c + Vec2::new(PIE_HOLE, -8.0), "No pinned swatches".into());
    }
}

/// One ring wedge between radii `r.0..r.1` and headings `a.0..a.1` (0 up,
/// clockwise) around `c`. A mesh, since egui only fills convex paths.
fn pie_wedge(
    painter: &egui::Painter,
    c: egui::Pos2,
    r: (f32, f32),
    a: (f32, f32),
    fill: Color32,
    stroke: Stroke,
) {
    let n = (((a.1 - a.0) / 0.1).ceil() as usize).max(2);
    let at = |rad: f32, j: usize| {
        let h = a.0 + (a.1 - a.0) * j as f32 / n as f32;
        egui::pos2(c.x + rad * h.sin(), c.y - rad * h.cos())
    };
    let mut mesh = egui::Mesh::default();
    for j in 0..=n {
        mesh.colored_vertex(at(r.0, j), fill);
        mesh.colored_vertex(at(r.1, j), fill);
        if j > 0 {
            let v = 2 * j as u32;
            mesh.add_triangle(v - 2, v - 1, v);
            mesh.add_triangle(v - 1, v + 1, v);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    let mut edge: Vec<egui::Pos2> = (0..=n).map(|j| at(r.1, j)).collect();
    edge.extend((0..=n).rev().map(|j| at(r.0, j)));
    painter.add(egui::Shape::closed_line(edge, stroke));
}

/// A Krita preset's cursor: its tip's outline, as Krita draws it. For
/// Pencil-5 that is the bar, turned by the way the pen leans and grown as it
/// tilts, exactly as the next dab would be. Hovering reads as full pressure,
/// as in Krita; mid-stroke it is the pen's own.
fn draw_krita_outline(state: &AppState, painter: &egui::Painter, canvas_rect: Rect, pos: egui::Pos2) {
    let xf = Xform::new(state, canvas_rect);
    let packet = state.pen.last_packet().filter(|_| state.pen.pen_active());
    let tilt = packet.map_or((0.0, 0.0), |p| (p.tilt_x, p.tilt_y));
    let pressure = match (state.stroke.is_some(), packet) {
        (true, Some(p)) => p.pressure,
        _ => 1.0,
    };
    let (rotation, mirrored) = state.cell_screen_orientation();
    let corners = state.brush.krita.brush().outline(
        state.effective_radius() as f64 * 2.0,
        pressure,
        tilt,
        rotation,
        mirrored,
    );

    // The corners sit around the pointer in the cell's pixels: find the cell
    // point under it, then map each corner back out through the layer and
    // the view, so zoom, rotation and flips all come along.
    let li = state.project.current_layer;
    let f = state.project.current_frame;
    let t = state.display_transform(li, f);
    let (cw, ch) = state.project.draw_cell_size(li, f);
    let (cw, ch) = (cw as f32, ch as f32);
    let (pw, ph) = (state.project.width as f32, state.project.height as f32);
    let doc = xf.screen_to_doc(pos);
    let (u, v) = t.doc_to_cell(doc.0, doc.1, cw, ch, pw, ph);
    let outline: Vec<egui::Pos2> = corners
        .iter()
        .map(|&(dx, dy)| {
            let (x, y) = t.cell_to_doc(u + dx as f32, v + dy as f32, cw, ch, pw, ph);
            xf.doc_to_screen(x, y)
        })
        .collect();

    // Black under white, so it shows on any drawing.
    let white = theme::white_alpha(220);
    let black = theme::premul(0, 0, 0, 180);
    painter.add(egui::Shape::closed_line(outline.clone(), Stroke::new(2.5, black)));
    painter.add(egui::Shape::closed_line(outline, Stroke::new(1.0, white)));
    painter.circle_filled(pos, 1.2, white);
    painter.circle_stroke(pos, 1.2, Stroke::new(0.6, black));
}

/// Paint the active tool's cursor preview on top of the canvas.
/// Pencil/Ink/Eraser → outline circle sized by brush radius (in doc px → screen
/// px via current canvas scale). Eraser shown with a dashed inner ring.
/// Fill → crosshair + small filled dot at the click point.
fn draw_tool_cursor(state: &AppState, ui: &egui::Ui, canvas_rect: Rect, pos: egui::Pos2) {
    let painter = ui.painter_at(canvas_rect);
    // Effective document-pixels → screen-pixels scale (includes zoom).
    let xf = Xform::new(state, canvas_rect);
    let scale = xf.scale;
    // Before a stroke picks its way, show the ways it could pick.
    let idle = state.stroke.is_none() && state.shape_drag.is_none();
    if idle && state.perspective.cursor_guides && state.snapping() {
        draw_cursor_guides(state, &painter, &xf, canvas_rect, pos);
    }

    let white = theme::white_alpha(220);
    let black = theme::premul(0, 0, 0, 180);

    match state.tool {
        ActiveTool::Pencil | ActiveTool::Ink | ActiveTool::Eraser
            if state.brush.mode == BrushMode::Krita =>
        {
            draw_krita_outline(state, &painter, canvas_rect, pos);
        }
        ActiveTool::Pencil | ActiveTool::Ink | ActiveTool::Eraser => {
            // Effective radius scales with pressure (mouse = 1.0 always).
            let pressure = state.pen.current_pressure().unwrap_or(1.0);
            let r_cell = (state.effective_radius() * state.brush.size.apply(pressure)).max(0.5);
            // `effective_radius` is in *cell* pixels, so the layer's own scale
            // belongs here too — otherwise the ring misreports the stroke width
            // on a scaled layer, and misses the point entirely under the
            // screen-size lock (which divides that same factor back out).
            let layer_scale = state
                .display_transform(state.project.current_layer, state.project.current_frame)
                .scale
                .abs();
            let r = (r_cell * scale * layer_scale).max(2.0);

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
                Color32::from_rgb(c[0], c[1], c[2]),
            );
            painter.circle_stroke(pos, 2.8, Stroke::new(0.8, black));
            // What a press being dragged is set to — beside the pen, and only
            // while the drag lasts.
            if state.fill_dragging() {
                let text = format!(
                    "Gap {} · Expand {}",
                    state.brush.fill_gap, state.brush.fill_expand
                );
                draw_readout(&painter, pos, text);
            }
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
                Color32::from_rgb(c[0], c[1], c[2]),
            );
            painter.circle_stroke(pos, 2.8, Stroke::new(0.8, black));
        }
        ActiveTool::Lasso => {
            // Small crosshair (the path is the real feedback) with an open
            // loop above-right, mirroring the tool icon.
            let arm = 7.0;
            for (w, col) in [(1.4, black), (0.8, white)] {
                painter.line_segment(
                    [egui::pos2(pos.x - arm, pos.y), egui::pos2(pos.x + arm, pos.y)],
                    Stroke::new(w, col),
                );
                painter.line_segment(
                    [egui::pos2(pos.x, pos.y - arm), egui::pos2(pos.x, pos.y + arm)],
                    Stroke::new(w, col),
                );
            }
            let c = pos + egui::vec2(9.0, -9.0);
            painter.circle_stroke(c, 5.0, Stroke::new(1.6, black));
            painter.circle_stroke(c, 5.0, Stroke::new(0.9, white));
            // What the next drag does to the selection: add, remove or
            // intersect. Replace needs no badge.
            let op = ui.input(|i| held_sel_op(state, i)).unwrap_or(state.sel_op);
            let badge = match op {
                SelOp::Replace => None,
                SelOp::Add => Some("+"),
                SelOp::Subtract => Some("−"),
                SelOp::Intersect => Some("∩"),
            };
            if let Some(b) = badge {
                let at = pos + egui::vec2(9.0, 10.0);
                let font = egui::FontId::proportional(13.0);
                for (d, col) in [(egui::vec2(1.0, 1.0), black), (egui::Vec2::ZERO, white)] {
                    painter.text(at + d, egui::Align2::CENTER_CENTER, b, font.clone(), col);
                }
            }
        }
        ActiveTool::Perspective => {
            // Plain crosshair: the handles are the real feedback.
            let arm = 7.0;
            for (w, col) in [(1.4, black), (0.8, white)] {
                painter.line_segment(
                    [egui::pos2(pos.x - arm, pos.y), egui::pos2(pos.x + arm, pos.y)],
                    Stroke::new(w, col),
                );
                painter.line_segment(
                    [egui::pos2(pos.x, pos.y - arm), egui::pos2(pos.x, pos.y + arm)],
                    Stroke::new(w, col),
                );
            }
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

/// The Paste button beside a colour readout.
fn paste_button(ui: &mut egui::Ui) -> bool {
    ui.button("Paste")
        .on_hover_text("Paste a colour copied as hsl(…), rgb(…) or #hex")
        .clicked()
}

/// The colour on the clipboard, in any form [`parse_color`] reads.
fn clipboard_color() -> Option<[u8; 3]> {
    let text = arboard::Clipboard::new().ok()?.get_text().ok()?;
    parse_color(&text)
}

fn canvas_to_doc_mapping(state: &AppState, rect: Rect) -> impl Fn(egui::Pos2) -> (f32, f32) + Copy {
    let xf = Xform::new(state, rect);
    move |pos: egui::Pos2| -> (f32, f32) { xf.screen_to_doc(pos) }
}

/// A stroke point in document space, passed through perspective snap.
fn snapped(state: &mut AppState, doc: (f32, f32)) -> (f32, f32) {
    let [x, y] = state.snap_doc([doc.0, doc.1]);
    (x, y)
}

/// Map a document point to the active layer's cell-local pixel space, inverting
/// the layer transform so drawing lands correctly on moved/scaled/rotated
/// layers. Sizes through `Project::draw_cell_size`, so an unkeyed frame maps
/// into the cell that is about to be allocated rather than a frame-sized one.
fn doc_to_active_cell(state: &AppState, doc: (f32, f32)) -> (f32, f32) {
    let li = state.project.current_layer;
    let f = state.project.current_frame;
    let t = state.display_transform(li, f);
    let (cw, ch) = state.project.draw_cell_size(li, f);
    let (pw, ph) = (state.project.width as f32, state.project.height as f32);
    t.doc_to_cell(doc.0, doc.1, cw as f32, ch as f32, pw, ph)
}

/// How far, in points, the newest packet may sit from the OS cursor before
/// the batch is disbelieved.
///
/// Both are read at the same moment — packets are drained at the top of the
/// frame, the cursor position comes from that frame's input — and the driver
/// moves the cursor from the very same packets. So they agree closely however
/// fast the stroke is: it is the *oldest* packet in a batch that trails the
/// cursor, never the newest. Disagreement here means the mapping is wrong, not
/// that the hand moved, which makes this a cheap check on a class of bug whose
/// symptom is otherwise a stroke drawn confidently in the wrong place.
const PEN_MOUSE_AGREEMENT: f32 = 20.0;

/// How far, in points, a packet may sit from the newest one in its own batch.
///
/// A batch is one frame's worth, so this bounds a frame of travel: 400 points
/// inside one 60 Hz frame is 24,000 points per second, which no hand produces.
/// What it catches is a packet that is not a position at all.
const MAX_PACKET_JUMP: f32 = 400.0;

/// This frame's tablet input, as far as a stroke is concerned.
#[derive(Debug)]
enum PenFrame {
    /// Packets as egui screen positions, oldest first, each paired with the
    /// packet it came from. Sub-pixel, and typically several per frame,
    /// against the one whole-pixel position egui reports — which is the
    /// entire reason this path exists.
    Points(Vec<(egui::Pos2, PenPacket)>),
    /// The pen is live and its mapping trusted, but it reported nothing new
    /// this frame. Common: a live stroke repaints as fast as it can, far
    /// faster than a pen reports.
    Empty,
    /// No pen to draw from: none at all, or a mapping that disagrees with the
    /// OS cursor.
    Untrusted,
}

/// A tablet packet as an egui screen position: virtual-desktop physical
/// pixels -> client physical pixels -> points.
fn pen_to_points(p: &PenPacket, (ox, oy): (f32, f32), ppp: f32) -> egui::Pos2 {
    egui::pos2((p.x - ox) / ppp, (p.y - oy) / ppp)
}

/// Whether a packet position is close enough to the OS cursor to believe.
/// Counts and logs (once per stroke) when it is not.
fn pen_agrees_with_cursor(state: &mut AppState, pen: egui::Pos2, cursor: egui::Pos2) -> bool {
    if (pen.x - cursor.x).abs() + (pen.y - cursor.y).abs() <= PEN_MOUSE_AGREEMENT {
        return true;
    }
    state.pen_batches_rejected = state.pen_batches_rejected.saturating_add(1);
    if !state.pen_outlier_logged {
        state.pen_outlier_logged = true;
        log::warn!(
            "tablet reports {pen:?} but the cursor is at {cursor:?}; \
             drawing this stroke from the cursor instead"
        );
    }
    false
}

/// Where a stroke pressed at `cursor` should start, if the pen is to draw
/// it: the newest packet, from this frame or an earlier one, in screen
/// points. `None` makes it a cursor stroke.
///
/// The newest packet rather than the cursor, because the cursor is that same
/// position rounded to a whole pixel — and because this frame's older
/// packets, which are skipped, lie at or behind it.
fn pen_stroke_start(
    state: &mut AppState,
    ppp: f32,
    cursor: egui::Pos2,
) -> Option<(egui::Pos2, PenPacket)> {
    if !state.pen.pen_active() {
        return None;
    }
    let origin = state.pen.client_origin()?;
    let packet = state.pen.last_packet()?;
    let at = pen_to_points(&packet, origin, ppp);
    pen_agrees_with_cursor(state, at, cursor).then_some((at, packet))
}

/// This frame's tablet packets, for a stroke already being drawn by the pen.
fn pen_stroke_points(state: &mut AppState, ppp: f32, pointer: Option<egui::Pos2>) -> PenFrame {
    if !state.pen.pen_active() {
        return PenFrame::Untrusted;
    }
    let Some(origin) = state.pen.client_origin() else {
        return PenFrame::Untrusted;
    };
    let raw = state.pen.packets();
    let Some(last) = raw.last() else {
        return PenFrame::Empty;
    };
    let newest = pen_to_points(last, origin, ppp);

    // Checked every frame rather than latched once per stroke. An earlier
    // version decided at pen-down and held, on the theory that the newest
    // packet outruns the cursor during fast motion — it does not, and holding
    // the decision meant a mapping that only looked right at the moment of
    // contact stayed trusted for the whole stroke.
    if let Some(pointer) = pointer {
        if !pen_agrees_with_cursor(state, newest, pointer) {
            return PenFrame::Untrusted;
        }
    }
    let raw = state.pen.packets();

    // Everything else in the batch is measured against the newest, not against
    // the cursor: the oldest is legitimately a frame of travel behind.
    let points: Vec<(egui::Pos2, PenPacket)> = raw
        .iter()
        .map(|p| (pen_to_points(p, origin, ppp), *p))
        .filter(|(at, _)| at.distance(newest) <= MAX_PACKET_JUMP)
        .collect();
    let dropped = raw.len() - points.len();
    state.pen_packets_dropped = state.pen_packets_dropped.saturating_add(dropped as u32);
    if dropped > 0 && !state.pen_outlier_logged {
        state.pen_outlier_logged = true;
        log::warn!(
            "dropped {} of {} tablet packets more than {MAX_PACKET_JUMP} points from the \
             newest in their own batch",
            raw.len() - points.len(),
            raw.len(),
        );
    }
    // Never empty — the newest packet is always within reach of itself.
    PenFrame::Points(points)
}

/// One frame's stroke samples, as screen positions paired with the packet
/// each came from (`None` for the cursor).
///
/// A stroke draws from one source. The cursor is the pen's own position
/// rounded to a whole pixel, so a pen stroke that took it on every frame
/// without a packet — most frames, since a live stroke repaints far faster
/// than a pen reports — zigzagged half a pixel either side of the line, which
/// is several canvas pixels once zoomed out. So a pen frame with nothing new
/// adds nothing. Only a mapping that stops agreeing with the cursor hands the
/// stroke over, and then for good: one switch rather than one per frame.
fn stroke_frame_samples(
    from_pen: &mut bool,
    pen: PenFrame,
    cursor: Option<egui::Pos2>,
) -> Vec<(egui::Pos2, Option<PenPacket>)> {
    if *from_pen {
        match pen {
            PenFrame::Points(points) => {
                return points.into_iter().map(|(at, p)| (at, Some(p))).collect();
            }
            PenFrame::Empty => return Vec::new(),
            PenFrame::Untrusted => *from_pen = false,
        }
    }
    cursor.map(|at| (at, None)).into_iter().collect()
}

/// A stroke sample at a cell-space position — from its own packet when the
/// pen drew it — counted against its source for the tablet readout.
fn stroke_sample(
    state: &mut AppState,
    x: f32,
    y: f32,
    t: f32,
    packet: Option<PenPacket>,
) -> crate::input::pointer::PointerSample {
    match packet {
        Some(p) => {
            state.stroke_pen_samples = state.stroke_pen_samples.saturating_add(1);
            state.make_pen_sample(x, y, t, &p)
        }
        None => {
            state.stroke_mouse_samples = state.stroke_mouse_samples.saturating_add(1);
            state.make_sample(x, y, t)
        }
    }
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
            state.open_new_project();
            ui.close_menu();
        }
        ui.separator();
        let open_label = format!("Open project…    {}", combo_text(state, Action::OpenProject));
        if ui
            .button(theme::icon_text(ic::FOLDER_OPEN, &open_label))
            .clicked()
        {
            match project_file::load_dialog() {
                Ok(Some((p, path))) => state.load_project(p, Some(path)),
                Ok(None) => {}
                Err(e) => log::error!("Open project failed: {e:#}"),
            }
            ui.close_menu();
        }
        // Name the file Save is about to overwrite — it no longer prompts, so
        // this is where you check what you are clobbering.
        let save_label = match state.project_path.as_ref().and_then(|p| p.file_name()) {
            Some(name) => format!(
                "Save  {}    {}",
                name.to_string_lossy(),
                combo_text(state, Action::SaveProject)
            ),
            None => format!("Save project…    {}", combo_text(state, Action::SaveProject)),
        };
        if ui
            .button(theme::icon_text(ic::FLOPPY_DISK, &save_label))
            .clicked()
        {
            state.save_project();
            ui.close_menu();
        }
        let save_as_label = format!(
            "Save project as…    {}",
            combo_text(state, Action::SaveProjectAs)
        );
        if ui
            .button(theme::icon_text(ic::FLOPPY_DISK_BACK, &save_as_label))
            .clicked()
        {
            state.save_project_as();
            ui.close_menu();
        }
        ui.separator();
        if ui
            .button(theme::icon_text(ic::PAINT_BRUSH, krita_menu_label(state)))
            .on_hover_text(
                "Opens the project in Krita as an animation. Each time you save \
                 there, the drawings, timing and layers you changed come back \
                 here as one undo step.\n\nReference layers and layers named …-x \
                 stay here; Krita layers named …-x stay in Krita.",
            )
            .clicked()
        {
            state.edit_in_krita(state.project.current_layer);
            ui.close_menu();
        }
        if state.krita_linked()
            && ui
                .button(theme::icon_text(ic::LINK_BREAK, "Stop Krita link"))
                .on_hover_text("Stop bringing Krita saves back here")
                .clicked()
        {
            state.stop_krita_link();
            ui.close_menu();
        }
        let helper_label = if !crate::krita_helper::installed() {
            "Install Krita helper…"
        } else if crate::krita_helper::outdated() {
            "Update Krita helper…"
        } else {
            "Reinstall Krita helper…"
        };
        if ui
            .button(theme::icon_text(ic::PUZZLE_PIECE, helper_label))
            .on_hover_text(
                "Puts a small plugin in Krita's plugin folder so Send to Krita again \
                 reloads the file in Krita by itself — no closing and reopening. \
                 Enable \u{201c}Animator Link\u{201d} in Krita afterwards.",
            )
            .clicked()
        {
            state.install_krita_helper();
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
        for (icon, label, kind) in [
            (ic::IMAGES, "Export PNG sequence…", ExportKind::PngSequence),
            (ic::FILM_REEL, "Export animated GIF…", ExportKind::Gif),
            (ic::FILM_STRIP, "Export MP4…", ExportKind::Mp4),
            (ic::GRID_FOUR, "Export sprite sheet…", ExportKind::SpriteSheet),
        ] {
            if ui.button(theme::icon_text(icon, label)).clicked() {
                open_export(state, kind);
                ui.close_menu();
            }
        }
        if ui.button(theme::icon_text(ic::FILE_ARROW_UP, "Export .kra…")).clicked() {
            state.export_kra();
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
        if ui
            .button(theme::icon_text(ic::FILE_ARROW_DOWN, "Import .kra…"))
            .on_hover_text("Adds the Krita document's paint layers under the active layer")
            .clicked()
        {
            state.import_kra();
            ui.close_menu();
        }
        ui.separator();
        if ui
            .button(theme::icon_text(ic::ERASER, "Clear current cell"))
            .clicked()
        {
            state.clear_active();
            ui.close_menu();
        }
        ui.separator();
        if ui.button(theme::icon_text(ic::SIGN_OUT, "Quit")).clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    });
}

/// How tall the saved-preset list may grow before it scrolls, in points —
/// about six rows, so a long list never pushes Create off the screen.
const PRESET_LIST_MAX_H: f32 = 170.0;

fn new_project_dialog(state: &mut AppState, ctx: &egui::Context) {
    if !state.show_new_project {
        return;
    }
    let mut open = true;
    let mut create = false;
    let mut cancel = false;
    // Saved-preset actions, applied after the window closure like the
    // create / cancel flags above.
    let mut apply = None;
    let mut star = None;
    let mut delete = None;
    let mut start_rename = None;
    let mut rename_commit = false;
    let mut rename_cancel = false;
    let mut save = false;
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
                // Orientation is read back out of the numbers rather than
                // stored, so the toggle can never disagree with a size the
                // user typed by hand. A square canvas reads as landscape.
                let cfg = &mut state.new_project_cfg;
                let landscape = cfg.width >= cfg.height;
                let mut want = landscape;
                if ui
                    .selectable_label(landscape, theme::icon_text(ic::MONITOR, "Landscape"))
                    .clicked()
                {
                    want = true;
                }
                if ui
                    .selectable_label(!landscape, theme::icon_text(ic::DEVICE_MOBILE, "Portrait"))
                    .clicked()
                {
                    want = false;
                }
                if want != landscape {
                    std::mem::swap(&mut cfg.width, &mut cfg.height);
                }
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
                // The table is written landscape; a preset is flipped to
                // whichever orientation is currently selected, so picking one
                // doesn't quietly undo the choice.
                let landscape = state.new_project_cfg.width >= state.new_project_cfg.height;
                for &(label, w, h) in &[
                    ("HD 720p", 1280, 720),
                    ("Full HD", 1920, 1080),
                    ("2K", 2048, 1080),
                    ("4K UHD", 3840, 2160),
                    ("6K", 6144, 3456),
                    ("8K", 7680, 4320),
                ] {
                    if ui.button(label).clicked() {
                        let (w, h) = if landscape { (w, h) } else { (h, w) };
                        state.new_project_cfg.width = w;
                        state.new_project_cfg.height = h;
                    }
                }
            });

            ui.add_space(8.0);
            theme::section_header(ui, ic::BOOKMARK_SIMPLE, "My presets");
            let replacing = state.preset_named(&state.new_project_cfg.preset_name).is_some();
            let (presets, cfg) = (&state.project_presets, &mut state.new_project_cfg);
            if presets.is_empty() {
                ui.label(
                    egui::RichText::new("Name the size above and save it to reuse it.")
                        .color(theme::TEXT_MUTED)
                        .size(11.0),
                );
            }
            egui::ScrollArea::vertical()
                .id_salt("project_presets")
                .max_height(PRESET_LIST_MAX_H)
                .show(ui, |ui| {
                    for (i, p) in presets.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let star_color = if p.is_default {
                                theme::ACCENT
                            } else {
                                theme::TEXT_MUTED
                            };
                            let tip = if p.is_default {
                                "New project opens on this preset. Click to stop."
                            } else {
                                "Open New project on this preset"
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(ic::STAR).size(15.0).color(star_color),
                                    )
                                    .min_size(egui::vec2(28.0, 22.0)),
                                )
                                .on_hover_text(tip)
                                .clicked()
                            {
                                star = Some(i);
                            }
                            // Name: click applies, double-click renames inline.
                            let editing = matches!(cfg.preset_rename.as_ref(), Some(r) if r.index == i);
                            if editing {
                                let r = cfg.preset_rename.as_mut().unwrap();
                                let te = ui.add(
                                    egui::TextEdit::singleline(&mut r.buf).desired_width(160.0),
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
                                let current =
                                    (cfg.width, cfg.height, cfg.fps) == (p.width, p.height, p.fps);
                                let resp = ui
                                    .selectable_label(
                                        current,
                                        format!("{}   {}×{} · {} fps", p.name, p.width, p.height, p.fps),
                                    )
                                    .on_hover_text("Click to use, double-click to rename");
                                if resp.double_clicked() {
                                    start_rename = Some(i);
                                } else if resp.clicked() {
                                    apply = Some(i);
                                }
                            }
                            if theme::icon_button(ui, ic::TRASH, "Delete preset").clicked() {
                                delete = Some(i);
                            }
                        });
                    }
                });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let te = ui.add(
                    egui::TextEdit::singleline(&mut cfg.preset_name)
                        .hint_text("Preset name")
                        .desired_width(160.0),
                );
                let entered = te.lost_focus() && ui.input(|inp| inp.key_pressed(egui::Key::Enter));
                let named = !cfg.preset_name.trim().is_empty();
                let (label, tip) = if replacing {
                    ("Replace preset", "Overwrite the saved preset with this name")
                } else {
                    ("Save preset", "Save the size and FPS above under this name")
                };
                if ui
                    .add_enabled(named, egui::Button::new(theme::icon_text(ic::FLOPPY_DISK, label)))
                    .on_hover_text(tip)
                    .on_disabled_hover_text("Type a name first")
                    .clicked()
                    || (named && entered)
                {
                    save = true;
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
    if rename_cancel {
        state.new_project_cfg.preset_rename = None;
    } else if rename_commit {
        if let Some(r) = state.new_project_cfg.preset_rename.take() {
            // A blank or clashing name keeps the old one, like a layer rename.
            state.rename_preset(r.index, &r.buf);
        }
    } else if let Some(i) = start_rename {
        state.new_project_cfg.preset_rename = Some(crate::app::LayerRename {
            index: i,
            buf: state.project_presets[i].name.clone(),
            focused: false,
        });
    }
    if let Some(i) = delete {
        state.delete_preset(i);
    } else if let Some(i) = star {
        state.toggle_default_preset(i);
    } else if let Some(i) = apply {
        state.apply_preset(i);
    }
    if save {
        state.save_preset();
    }
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

/// Open the shared export dialog for `kind`, defaulting the range to the whole
/// timeline the first time it is used on this project.
fn open_export(state: &mut AppState, kind: ExportKind) {
    let last = state.project.frame_count.saturating_sub(1);
    state.export_cfg.kind = kind;
    state.export_cfg.start = state.export_cfg.start.min(last);
    state.export_cfg.end = state.export_cfg.end.clamp(state.export_cfg.start, last);
    if state.export_cfg.end == 0 {
        state.export_cfg.end = last;
    }
    state.show_export = true;
}

/// Inclusive start/end frame picker: dual-knob slider plus exact inputs.
/// Shared by the export dialog and the import range dialog so the two cannot
/// drift apart.
fn frame_range_ui(ui: &mut egui::Ui, start: &mut usize, end: &mut usize, last: usize) {
    crate::ui::widgets::range_slider(ui, start, end, 0, last);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Start:");
        ui.add(egui::DragValue::new(start).range(0..=last).speed(1));
        ui.add_space(12.0);
        ui.label("End:");
        ui.add(egui::DragValue::new(end).range(0..=last).speed(1));
    });
    if start > end {
        *end = *start;
    }
}

/// One modal for every export format: the frame range applies to all of them,
/// and the format-specific controls are only a few widgets each.
fn export_dialog(state: &mut AppState, ctx: &egui::Context) {
    if !state.show_export {
        return;
    }
    let kind = state.export_cfg.kind;
    let last = state.project.frame_count.saturating_sub(1);
    let loop_start = state.project.loop_start.min(last);
    let loop_end = state.project.loop_end.saturating_sub(1).min(last);

    let mut open = true;
    let mut export = false;
    let mut cancel = false;
    egui::Window::new(theme::icon_text(ic::FILM_STRIP, kind.title()))
        .open(&mut open)
        .default_pos([380.0, 180.0])
        .default_width(400.0)
        .resizable(false)
        .collapsible(false)
        .frame(floating_frame())
        .show(ctx, |ui| {
            ui.label(format!(
                "{} x {} @ {:.0} fps - {} frames",
                state.project.width,
                state.project.height,
                state.project.fps,
                state.project.frame_count,
            ));
            ui.add_space(8.0);

            theme::section_header(ui, ic::FILM_REEL, "Frames");
            frame_range_ui(
                ui,
                &mut state.export_cfg.start,
                &mut state.export_cfg.end,
                last,
            );
            if ui
                .button("Use loop range")
                .on_hover_text("Match the loop bars shown on the frame strip")
                .clicked()
            {
                state.export_cfg.start = loop_start;
                state.export_cfg.end = loop_end.max(loop_start);
            }

            let count = state.export_cfg.end.saturating_sub(state.export_cfg.start) + 1;

            if kind != ExportKind::SpriteSheet {
                ui.add_space(6.0);
                theme::section_header(ui, ic::REPEAT, "Playback");
                ui.checkbox(&mut state.export_cfg.ping_pong, "Ping-pong (forward, then back)");
                // A GIF repeats forever by itself; only files with an end
                // need a length.
                if matches!(kind, ExportKind::Mp4 | ExportKind::PngSequence) {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut state.export_cfg.loop_on, "Loop to");
                        ui.add_enabled(
                            state.export_cfg.loop_on,
                            egui::DragValue::new(&mut state.export_cfg.loop_secs)
                                .range(0.1..=3600.0)
                                .speed(0.1)
                                .max_decimals(2)
                                .suffix(" s"),
                        );
                    });
                    ui.label(
                        egui::RichText::new("Rounds up to whole loops, so it repeats seamlessly.")
                            .small()
                            .color(theme::TEXT_MUTED),
                    );
                }
            }
            let order = crate::io::frame_order::info(
                state.export_cfg.start,
                state.export_cfg.end,
                state.export_cfg.playback(kind),
                state.project.fps,
            );

            match kind {
                ExportKind::Mp4 => {
                    ui.add_space(6.0);
                    theme::section_header(ui, ic::SLIDERS, "Encoding");
                    ui.add(
                        egui::Slider::new(&mut state.export_cfg.mp4.crf, 0..=51)
                            .text("Quality (CRF)"),
                    );
                    ui.label(
                        egui::RichText::new(
                            "Lower = better quality, larger file. 18 is visually lossless.",
                        )
                        .small()
                        .color(theme::TEXT_MUTED),
                    );
                    ui.add_space(6.0);
                    egui::ComboBox::from_label("Preset")
                        .selected_text(
                            MP4_PRESETS[state.export_cfg.mp4.preset_idx.min(MP4_PRESETS.len() - 1)],
                        )
                        .show_ui(ui, |ui| {
                            for (i, preset) in MP4_PRESETS.iter().enumerate() {
                                ui.selectable_value(
                                    &mut state.export_cfg.mp4.preset_idx,
                                    i,
                                    *preset,
                                );
                            }
                        });
                    ui.label(
                        egui::RichText::new("Slower preset = smaller file, longer encode.")
                            .small()
                            .color(theme::TEXT_MUTED),
                    );
                }
                ExportKind::SpriteSheet => {
                    ui.add_space(6.0);
                    theme::section_header(ui, ic::GRID_FOUR, "Grid");
                    ui.horizontal(|ui| {
                        ui.label("Columns:");
                        ui.add(
                            egui::DragValue::new(&mut state.export_cfg.sheet_columns)
                                .range(0..=64)
                                .speed(1),
                        )
                        .on_hover_text("0 = auto (near-square)");
                        ui.add_space(12.0);
                        ui.label("Padding:");
                        ui.add(
                            egui::DragValue::new(&mut state.export_cfg.sheet_padding)
                                .range(0..=64)
                                .speed(1),
                        )
                        .on_hover_text("Transparent gutter between cells");
                    });
                }
                _ => {}
            }

            ui.add_space(6.0);
            // Say how big this gets *before* the user waits for it: a long range
            // at 4K makes a sheet tens of thousands of pixels wide.
            let summary = match kind {
                ExportKind::SpriteSheet => {
                    let (cols, rows) =
                        crate::io::sprite_sheet::grid(count, state.export_cfg.sheet_columns);
                    let (sw, sh) = crate::io::sprite_sheet::sheet_size(
                        state.project.width,
                        state.project.height,
                        cols,
                        rows,
                        state.export_cfg.sheet_padding,
                    );
                    format!("{count} frames - {cols}x{rows} grid - {sw} x {sh} px")
                }
                // Same rule as `png_seq::export_to`: numbered by position once
                // any frame shows twice.
                ExportKind::PngSequence if order.frames > count => format!(
                    "{} files, frame_0000.png to frame_{:04}.png",
                    order.frames,
                    order.frames - 1
                ),
                ExportKind::PngSequence => format!(
                    "{count} files, frame_{:04}.png to frame_{:04}.png",
                    state.export_cfg.start, state.export_cfg.end
                ),
                _ if order.loops > 1 => format!(
                    "{} loops of {} frames - {} frames - {:.1}s at {:.0} fps",
                    order.loops,
                    order.cycle,
                    order.frames,
                    order.secs,
                    state.project.fps
                ),
                _ => format!(
                    "{} frames - {:.1}s at {:.0} fps",
                    order.frames,
                    order.secs,
                    state.project.fps
                ),
            };
            ui.label(
                egui::RichText::new(summary)
                    .color(theme::TEXT_MUTED)
                    .size(11.0),
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
        state.show_export = false;
        state.start_export();
    } else if cancel || !open {
        state.show_export = false;
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

            frame_range_ui(ui, &mut start, &mut end, last);
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
/// Transient "saved" confirmation. Ctrl+S no longer opens a dialog, so this is
/// the only sign the write happened at all.
fn save_toast(state: &mut AppState, ctx: &egui::Context) {
    // A write runs on a worker thread, so without this there would be no sign
    // at all between pressing Save and the toast arriving.
    if state.save_job.is_some() {
        egui::Area::new(egui::Id::new("save_toast"))
            .order(egui::Order::Foreground)
            .interactable(false)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -28.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .fill(theme::BG_PANEL)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(14.0));
                            ui.label(egui::RichText::new("Saving…").color(theme::TEXT));
                        });
                    });
            });
        return;
    }
    let Some((name, deadline)) = state.save_toast.clone() else {
        return;
    };
    let now = std::time::Instant::now();
    let Some(left) = deadline.checked_duration_since(now) else {
        state.save_toast = None;
        return;
    };
    // Keep the frames coming, or the toast lingers until the next mouse move.
    ctx.request_repaint_after(left);
    // Fade over the last half second.
    let a = (left.as_secs_f32() / 0.5).clamp(0.0, 1.0);
    let fade = |c: Color32| c.gamma_multiply(a);

    egui::Area::new(egui::Id::new("save_toast"))
        .order(egui::Order::Foreground)
        .interactable(false)
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -28.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(fade(theme::BG_PANEL))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(ic::CHECK_CIRCLE)
                                .color(fade(theme::ACCENT))
                                .size(15.0),
                        );
                        ui.label(
                            egui::RichText::new(format!("Saved  {name}")).color(fade(theme::TEXT)),
                        );
                    });
                });
        });
}

/// "Edit in Krita" until a link is live, then what the click does instead.
fn krita_menu_label(state: &AppState) -> &'static str {
    if state.krita_linked() {
        "Send to Krita again"
    } else {
        "Edit in Krita"
    }
}

/// Krita-link messages: a pull landing, a send, or what a Krita save held
/// that couldn't come across. Sits above the save toast so both can show.
fn krita_toast(state: &mut AppState, ctx: &egui::Context) {
    let Some((msg, warn, deadline)) = state.krita_toast.clone() else {
        return;
    };
    let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) else {
        state.krita_toast = None;
        return;
    };
    ctx.request_repaint_after(left);
    let a = (left.as_secs_f32() / 0.5).clamp(0.0, 1.0);
    let fade = |c: Color32| c.gamma_multiply(a);
    let (icon, tint) = if warn {
        (ic::WARNING, Color32::from_rgb(232, 176, 72))
    } else {
        (ic::PAINT_BRUSH, theme::ACCENT)
    };
    egui::Area::new(egui::Id::new("krita_toast"))
        .order(egui::Order::Foreground)
        .interactable(false)
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -64.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(fade(theme::BG_PANEL))
                .show(ui, |ui| {
                    ui.set_max_width(520.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(icon).color(fade(tint)).size(15.0));
                        ui.add(
                            egui::Label::new(egui::RichText::new(msg).color(fade(theme::TEXT))).wrap(),
                        );
                    });
                });
        });
}

/// Modal report for a failed write. Blocking on purpose: a save that silently
/// failed leaves you believing your work is on disk when it isn't.
fn save_error_dialog(state: &mut AppState, ctx: &egui::Context) {
    let Some(msg) = state.save_error.clone() else {
        return;
    };
    let mut dismiss = false;
    let mut save_as = false;

    egui::Window::new(theme::icon_text(ic::WARNING, "Save failed"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .frame(floating_frame())
        .show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.label(egui::RichText::new(&msg).color(theme::TEXT));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Save As…").clicked() {
                    save_as = true;
                }
                if ui.button("OK").clicked() {
                    dismiss = true;
                }
            });
        });

    // Resolve outside the closure so `state` isn't borrowed twice.
    if save_as {
        state.save_error = None;
        state.save_project_as();
    } else if dismiss {
        state.save_error = None;
    }
}

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

#[cfg(test)]
mod tests {
    use super::Xform;

    /// `screen_to_doc` is a hand-written inverse of `doc_to_screen`, so the two
    /// can silently disagree — and a mirror is exactly the kind of term that
    /// gets applied on the wrong side of the rotation. Round-trip every flip
    /// combination through a rotated, zoomed, panned view.
    #[test]
    fn xform_round_trips_under_every_flip() {
        for (fx, fy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
            let xf = Xform::from_parts(
                egui::pos2(640.0, 360.0),
                egui::vec2(37.0, -19.0),
                2.5,
                fx,
                fy,
                0.7,
                1280.0,
                720.0,
            );
            for (x, y) in [(0.0, 0.0), (1280.0, 720.0), (100.0, 640.0), (933.0, 12.0)] {
                let (rx, ry) = xf.screen_to_doc(xf.doc_to_screen(x, y));
                assert!(
                    (rx - x).abs() < 0.01 && (ry - y).abs() < 0.01,
                    "flip ({fx},{fy}): ({x},{y}) -> ({rx},{ry})"
                );
            }
        }
    }

    /// A drag delta must map to document space with the same handedness as a
    /// position, or panning a layer under a mirrored view runs backwards.
    #[test]
    fn screen_delta_matches_position_mapping_under_flip() {
        for (fx, fy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
            let xf = Xform::from_parts(
                egui::pos2(300.0, 200.0),
                egui::Vec2::ZERO,
                3.0,
                fx,
                fy,
                -0.4,
                800.0,
                600.0,
            );
            let d = egui::vec2(21.0, -13.0);
            let a = xf.screen_to_doc(egui::pos2(100.0, 100.0));
            let b = xf.screen_to_doc(egui::pos2(100.0, 100.0) + d);
            let (dx, dy) = xf.screen_delta_to_doc(d);
            assert!((dx - (b.0 - a.0)).abs() < 0.01, "flip ({fx},{fy}) dx");
            assert!((dy - (b.1 - a.1)).abs() < 0.01, "flip ({fx},{fy}) dy");
        }
    }

    use super::restick_axis;

    /// The un-maximized default layout, and the maximized window it grows into.
    const SMALL: (f32, f32) = (0.0, 1280.0);
    const WIDE: (f32, f32) = (0.0, 2560.0);

    fn restick(lo: f32, len: f32, old: (f32, f32), new: (f32, f32)) -> f32 {
        restick_axis(lo, lo + len, len, old.0, old.1, new.0, new.1)
    }

    #[test]
    fn right_hand_column_tracks_the_right_edge() {
        // Layers/Onion/X-sheet default: x=1004, width 252 → 24px from the right.
        // That gap is what must survive, not the absolute x.
        assert_eq!(restick(1004.0, 252.0, SMALL, WIDE), 2560.0 - 24.0 - 252.0);
    }

    #[test]
    fn left_hand_column_stays_put() {
        // Tools/Brush default: x=12, width 232.
        assert_eq!(restick(12.0, 232.0, SMALL, WIDE), 12.0);
    }

    #[test]
    fn centred_panel_recentres_instead_of_drifting() {
        // Timeline default: x=320, width 640 → equal 320px gaps either side.
        // Nearest-edge alone would tie and pin it left; it should re-centre.
        assert_eq!(restick(320.0, 640.0, SMALL, WIDE), (2560.0 - 640.0) / 2.0);
    }

    #[test]
    fn bottom_anchored_panel_tracks_the_bottom() {
        // Timeline vertically: y=600, height 180 on an 800-tall window → 20px up
        // from the bottom.
        let tall = (0.0, 1400.0);
        assert_eq!(restick(600.0, 180.0, (0.0, 800.0), tall), 1400.0 - 20.0 - 180.0);
    }

    #[test]
    fn shrinking_keeps_the_panel_on_screen() {
        // Right-anchored panel, window shrinks: still 24px from the new right
        // edge, not hanging off it.
        assert_eq!(restick(1004.0, 252.0, SMALL, (0.0, 700.0)), 700.0 - 24.0 - 252.0);
    }

    #[test]
    fn panel_wider_than_the_window_clamps_to_the_left() {
        // Clamp range would invert here; must not produce a negative position.
        assert_eq!(restick(1004.0, 252.0, SMALL, (0.0, 100.0)), 0.0);
    }

    #[test]
    fn no_resize_is_a_no_op() {
        for lo in [12.0, 320.0, 1004.0] {
            assert_eq!(restick(lo, 252.0, SMALL, SMALL), lo, "lo={lo}");
        }
    }
}

/// The perspective tool through the real canvas: egui events in, grid out.
#[cfg(test)]
mod perspective_tests {
    use super::*;
    use crate::tools::perspective::Space;
    use egui::{pos2, vec2, Pos2};

    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, pos2(1200.0, 800.0));

    fn frame(ctx: &egui::Context, state: &mut AppState, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(SCREEN),
            events,
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| draw(state, ctx));
    }

    fn state() -> AppState {
        let mut state = AppState::for_test();
        // Floating panels would sit over the canvas and take the press.
        state.show_panels = false;
        state.show_mini_timeline = false;
        state.dispatch(Action::ToolPerspective);
        let g = &mut state.perspective.grids[0];
        g.extend = true;
        g.horizon = true;
        state
    }

    #[test]
    fn dragging_a_corner_on_the_canvas_reshapes_the_grid() {
        let mut state = state();
        let ctx = egui::Context::default();
        frame(&ctx, &mut state, vec![]);
        let cells = state.project.cells.len();
        let (w, h) = (state.project.width as f32, state.project.height as f32);
        let before = state.perspective.grids[0].doc_corners(&Space::flat(w, h));
        let xf = Xform::new(&state, SCREEN);
        let from = xf.doc_to_screen(before[1][0], before[1][1]);
        let to = from + vec2(40.0, 0.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from), button(from, true)]);
        for i in 1..=4 {
            let p = from + (to - from) * (i as f32 / 4.0);
            frame(&ctx, &mut state, vec![egui::Event::PointerMoved(p)]);
        }
        frame(&ctx, &mut state, vec![button(to, false)]);
        frame(&ctx, &mut state, vec![]);

        let after = state.perspective.grids[0].doc_corners(&Space::flat(w, h));
        let moved = xf.screen_to_doc(to);
        assert!((after[1][0] - moved.0).abs() < 0.5, "{:?} vs {moved:?}", after[1]);
        assert!((after[1][1] - moved.1).abs() < 0.5);
        assert_eq!(after[0], before[0], "the other corners stay put");
        // Grids are guides: nothing was drawn, so no cell was allocated.
        assert_eq!(state.project.cells.len(), cells);
        assert!(state.stroke.is_none());
    }

    #[test]
    fn grids_paint_with_every_option_and_tool() {
        let mut state = state();
        state.show_panels = true;
        state.show_grid_settings = true;
        state.perspective.show = true;
        state.perspective.grids.push(Default::default());
        // A two-point grid, so both rays and the horizon have finite ends.
        let (w, h) = (state.project.width as f32, state.project.height as f32);
        state.perspective.grids[1].set_doc_corners(
            [[300.0, 200.0], [700.0, 260.0], [650.0, 500.0], [250.0, 560.0]],
            &Space::flat(w, h),
        );
        // Both extra points on the two-point grid, one pulled off the
        // horizon; and one sitting right in the middle of the first grid,
        // where its rays go all the way round.
        let g = &mut state.perspective.grids[1];
        g.add_extra_vp(&Space::flat(w, h));
        g.add_extra_vp(&Space::flat(w, h));
        g.extra_vps[1].on_horizon = false;
        g.extra_vps[1].pos[1] -= 0.5;
        let c = state.perspective.grids[0].centre();
        state.perspective.grids[0].extra_vps.push(crate::tools::perspective::ExtraVp {
            pos: c,
            on_horizon: false,
            rays: true,
            snap: true,
        });
        let ctx = egui::Context::default();
        for active in [0, 1] {
            state.perspective.active = active;
            for tool in [Action::ToolPerspective, Action::ToolPencil] {
                state.dispatch(tool);
                frame(&ctx, &mut state, vec![]);
                frame(&ctx, &mut state, vec![]);
            }
        }
    }

    /// Every painted text in `shapes` that `keep` accepts, with where it is.
    fn texts(
        shapes: &[egui::epaint::ClippedShape],
        keep: &dyn Fn(&str) -> bool,
    ) -> Vec<(String, Rect)> {
        fn find(
            shape: &egui::Shape,
            keep: &dyn Fn(&str) -> bool,
            out: &mut Vec<(String, Rect)>,
        ) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| find(s, keep, out)),
                egui::Shape::Text(t) if keep(t.galley.text()) => {
                    out.push((t.galley.text().to_string(), t.visual_bounding_rect()));
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for s in shapes {
            find(&s.shape, keep, &mut out);
        }
        out
    }

    #[test]
    fn a_grids_gear_opens_its_settings_which_follow_the_active_grid() {
        use crate::tools::perspective::{GridKind, PerspectiveGrid};
        let mut state = state();
        state.show_panels = true;
        state.perspective.push(PerspectiveGrid::fresh(GridKind::Flat));
        state.perspective.active = 0;
        let ctx = egui::Context::default();
        crate::ui::theme::install(&ctx);
        let run = |state: &mut AppState, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(SCREEN),
                events,
                ..Default::default()
            };
            ctx.run(raw, |ctx| draw(state, ctx)).shapes
        };
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = run(&mut state, vec![]);
        }
        let shown = |shapes: &[egui::epaint::ClippedShape], text: &str| {
            !texts(shapes, &|t| t.contains(text)).is_empty()
        };
        // The panel lists the grids; their own settings are not in it.
        assert!(shown(&shapes, "Grid 2"));
        assert!(!shown(&shapes, "Columns") && !shown(&shapes, "Snap strokes to"));
        let tools = egui::AreaState::load(&ctx, egui::Id::new(panel_key(PanelId::Tools)))
            .expect("Tools panel laid out")
            .rect();
        let gears: Vec<Rect> = texts(&shapes, &|t| t == ic::GEAR)
            .into_iter()
            .map(|(_, r)| r)
            .filter(|r| tools.contains_rect(*r))
            .collect();
        assert_eq!(gears.len(), 2, "one gear a row");

        // The test screen is small enough for the Timeline panel's default
        // spot to cover the right half of the gear; press its left end.
        let at = gears[1].left_center() - vec2(3.0, 0.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(&mut state, vec![egui::Event::PointerMoved(at), button(true)]);
        run(&mut state, vec![button(false)]);
        assert!(state.show_grid_settings);
        assert_eq!(state.perspective.active, 1, "the gear picks its grid");
        run(&mut state, vec![]);
        let shapes = run(&mut state, vec![]);
        assert!(shown(&shapes, "Grid 2 settings") && shown(&shapes, "Flat grid"));
        assert!(shown(&shapes, "Columns"));

        // Picking another grid switches what the window shows.
        state.perspective.active = 0;
        run(&mut state, vec![]);
        let shapes = run(&mut state, vec![]);
        assert!(shown(&shapes, "Grid 1 settings") && shown(&shapes, "Perspective grid"));

        // With no grid left it closes itself.
        state.perspective.grids.clear();
        run(&mut state, vec![]);
        assert!(!state.show_grid_settings);
    }

    #[test]
    fn every_grid_kind_line_option_and_guide_paints() {
        use crate::tools::perspective::{Follow, GridKind, PerspectiveGrid};
        let mut state = state();
        state.show_panels = true;
        state.show_grid_settings = true;
        state.perspective.show = true;
        state.perspective.snap = true;
        state.project.ensure_frame_count(20);
        let (w, h) = (state.project.width as f32, state.project.height as f32);
        let s = Space::flat(w, h);
        {
            let g = &mut state.perspective.grids[0];
            g.infinite = true;
            g.diagonals = true;
            g.centre_lines = true;
            g.major_every = 2;
            g.add_extra_vp(&s);
            g.extra_vps[0].on_horizon = false;
            g.extra_vps[0].pos[1] -= 1.0;
        }
        assert!(state.perspective.add_wall(0, 0).is_some());
        assert!(state.perspective.add_wall(0, 1).is_some());
        state.perspective.push(PerspectiveGrid::fresh(GridKind::Flat));
        let iso = state.perspective.push(PerspectiveGrid::fresh(GridKind::Isometric));
        state.set_grid_follow(iso, Follow::Camera);
        // Keys either side of the frame, so the ghosts show.
        state.perspective.active = 0;
        state.add_grid_key();
        state.project.goto(10);
        state.perspective.grids[0].translate([0.05, 0.0]);
        state.add_grid_key();
        state.project.goto(5);
        let ctx = egui::Context::default();
        let hover = vec![egui::Event::PointerMoved(SCREEN.center())];
        for active in 0..state.perspective.grids.len() {
            state.perspective.active = active;
            for tool in [Action::ToolPerspective, Action::ToolPencil] {
                state.dispatch(tool);
                frame(&ctx, &mut state, hover.clone());
                frame(&ctx, &mut state, hover.clone());
            }
        }
        assert_eq!(state.perspective.grids.iter().filter(|g| g.is_wall()).count(), 2);
    }

    #[test]
    fn an_extra_vanishing_point_drags_along_the_horizon_on_the_canvas() {
        let mut state = state();
        let (w, h) = (state.project.width as f32, state.project.height as f32);
        // One-point floor with a level horizon at y = 66.7.
        state.perspective.grids[0].set_doc_corners(
            [[140.0, 100.0], [160.0, 100.0], [250.0, 400.0], [50.0, 400.0]],
            &Space::flat(w, h),
        );
        state.perspective.grids[0].add_extra_vp(&Space::flat(w, h));
        let v0 = state.perspective.grids[0].extra_doc(&Space::flat(w, h))[0];
        let ctx = egui::Context::default();
        frame(&ctx, &mut state, vec![]);
        let xf = Xform::new(&state, SCREEN);
        let from = xf.doc_to_screen(v0[0], v0[1]);
        // Along, and a little off: it stays on the horizon.
        let to = from + vec2(-40.0, 12.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from), button(from, true)]);
        for i in 1..=4 {
            let p = from + (to - from) * (i as f32 / 4.0);
            frame(&ctx, &mut state, vec![egui::Event::PointerMoved(p)]);
        }
        frame(&ctx, &mut state, vec![button(to, false)]);
        let v1 = state.perspective.grids[0].extra_doc(&Space::flat(w, h))[0];
        let want = xf.screen_to_doc(to);
        assert!((v1[0] - want.0).abs() < 0.5, "{v1:?} vs {want:?}");
        assert!((v1[1] - 200.0 / 3.0).abs() < 0.1, "still on the horizon: {v1:?}");
        assert!(state.perspective.grids[0].extra_vps[0].on_horizon);
        // The grid itself never moved.
        let tl = state.perspective.grids[0].doc_corners(&Space::flat(w, h))[0];
        assert!((tl[0] - 140.0).abs() < 1e-3);
    }

    #[test]
    fn a_rect_dragged_on_the_canvas_lies_on_the_grid() {
        let mut state = state();
        let (w, h) = (state.project.width as f32, state.project.height as f32);
        // One-point floor whose columns meet at (150, 66.7).
        state.perspective.grids[0].set_doc_corners(
            [[140.0, 100.0], [160.0, 100.0], [250.0, 400.0], [50.0, 400.0]],
            &Space::flat(w, h),
        );
        state.perspective.show = true;
        state.perspective.snap = true;
        state.dispatch(Action::ToolShape);
        state.brush.shape_kind = ShapeKind::Rect;
        state.brush.radius = 3.0;
        let ctx = egui::Context::default();
        frame(&ctx, &mut state, vec![]);
        let xf = Xform::new(&state, SCREEN);
        let from = xf.doc_to_screen(120.0, 380.0);
        let to = xf.doc_to_screen(180.0, 200.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from), button(from, true)]);
        for i in 1..=4 {
            let p = from + (to - from) * (i as f32 / 4.0);
            frame(&ctx, &mut state, vec![egui::Event::PointerMoved(p)]);
        }
        // Mid-drag, the preview paints from the grid outline.
        assert!(state.shape_drag.is_some_and(|d| d.plane.is_some()));
        frame(&ctx, &mut state, vec![button(to, false)]);
        frame(&ctx, &mut state, vec![]);

        let id = state.project.resolved_current().expect("the shape drew a cell");
        let c = state.project.cell(id).unwrap();
        let alpha = |x: u32, y: u32| c.pixels[((y * c.width + x) * 4 + 3) as usize];
        // Near edge out to the leaning column (x ≈ 220), flat corner bare.
        assert!(alpha(210, 380) > 200);
        assert_eq!(alpha(120, 200), 0);
    }
}

#[cfg(test)]
mod onion_tests {
    use super::*;
    use egui::{pos2, Pos2};

    #[test]
    fn a_pin_gets_a_ghost_in_its_own_colour_and_everything_paints() {
        let mut state = AppState::for_test();
        state.onion.enabled = true;
        state.onion.prev = 3;
        state.onion.next = 2;
        state.onion.toggle_hidden(1, OnionDirection::Prev);
        for _ in 0..4 {
            state.structural_edit(false, |p| {
                p.add_frame();
                p.insert_blank_key_here();
            });
        }
        // Playhead on 4: −1 is frame 3, −2 frame 2, −3 frame 1.
        let green = [0, 200, 0];
        let li = state.project.current_layer;
        state.project.layers[li].onion_pins = vec![
            OnionPin { frame: 1, tint: green, visible: true },
            OnionPin { frame: 3, tint: [9, 9, 9], visible: false },
        ];
        let cell_on = |f| state.project.layers[li].resolve(f).unwrap();
        let (pinned, plain, hidden) = (cell_on(1), cell_on(2), cell_on(3));

        let ctx = egui::Context::default();
        // Two passes, the second past `GHOST_REST`, so ghosts get built.
        for time in [0.0, 1.0] {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_max(Pos2::ZERO, pos2(1200.0, 800.0))),
                time: Some(time),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                state.sync_textures(ctx);
                draw(&mut state, ctx);
                egui::Window::new("onion").show(ctx, |ui| onion_content(&mut state, ui));
            });
        }
        let tint = |id| state.ghost_textures.get(&id).map(|(t, _)| *t);
        assert_eq!(tint(pinned), Some(green), "pin colour, not the prev tint");
        assert_eq!(tint(plain), Some(state.onion.prev_tint));
        // −1 is a hidden offset, and its pin is switched off.
        assert_eq!(tint(hidden), None);
    }

    /// Every ghost — past, future and pinned — paints before the current
    /// drawing, so none of them lands on top of the lines being drawn.
    #[test]
    fn the_current_drawing_paints_over_every_ghost() {
        let mut state = AppState::for_test();
        state.show_panels = false;
        state.onion.enabled = true;
        state.onion.prev = 1;
        state.onion.next = 1;
        for _ in 0..4 {
            state.structural_edit(false, |p| {
                p.add_frame();
                p.insert_blank_key_here();
            });
        }
        let li = state.project.current_layer;
        state.project.layers[li].onion_pins =
            vec![OnionPin { frame: 4, tint: [0, 200, 0], visible: true }];
        state.project.goto(2);
        let cell_on = |f| state.project.layers[li].resolve(f).unwrap();
        let (past, current, future, pinned) = (cell_on(1), cell_on(2), cell_on(3), cell_on(4));
        // A blank drawing paints nothing; give the current one a dot of ink.
        state.project.cell_mut(current).unwrap().pixels[3] = 255;

        let ctx = egui::Context::default();
        let mut meshes = Vec::new();
        for time in [0.0, 1.0] {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_max(Pos2::ZERO, pos2(1200.0, 800.0))),
                time: Some(time),
                ..Default::default()
            };
            let out = ctx.run(raw, |ctx| {
                state.sync_textures(ctx);
                draw(&mut state, ctx);
            });
            meshes = out
                .shapes
                .into_iter()
                .filter_map(|c| match c.shape {
                    egui::Shape::Mesh(m) => Some(m.texture_id),
                    _ => None,
                })
                .collect();
        }
        let at = |tex: Option<egui::TextureId>| {
            let tex = tex.expect("texture built");
            meshes.iter().position(|&t| t == tex).expect("painted")
        };
        let cell = at(state.cell_textures.get(&current).map(|t| t.tex.id()));
        for (name, id) in [("past", past), ("future", future), ("pinned", pinned)] {
            let ghost = at(state.ghost_textures.get(&id).map(|(_, t)| t.id()));
            assert!(ghost < cell, "{name} ghost painted over the drawing");
        }
    }

    /// Size of the chip row laid out `width` wide.
    fn chip_row(width: f32, prev: u8, next: u8) -> Vec2 {
        let mut state = AppState::for_test();
        state.onion.prev = prev;
        state.onion.next = next;
        let ctx = egui::Context::default();
        let mut size = Vec2::ZERO;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_max(Pos2::ZERO, pos2(1200.0, 800.0))),
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                size = ui
                    .allocate_ui(egui::vec2(width, 400.0), |ui| {
                        onion_offset_chips(&mut state, ui);
                        ui.min_rect().size()
                    })
                    .inner;
            });
        });
        size
    }

    /// Eight each way in the default-width panel (232px inside): past and
    /// future on a row each, never wrapped mid-range or wider than the panel.
    #[test]
    fn a_full_range_splits_into_two_rows_in_a_narrow_panel() {
        let s = chip_row(232.0, 8, 8);
        assert!(s.x <= 232.0, "{s:?}");
        assert!(s.y > 2.0 * CHIP.y && s.y < 3.0 * CHIP.y, "two rows: {s:?}");
    }

    #[test]
    fn a_range_that_fits_stays_on_one_compact_row() {
        let narrow = chip_row(232.0, 3, 3);
        assert!(narrow.y < 2.0 * CHIP.y, "{narrow:?}");
        // A wide panel doesn't stretch the chips to fill it.
        let wide = chip_row(460.0, 8, 8);
        assert!(wide.y < 2.0 * CHIP.y, "{wide:?}");
        assert!(wide.x <= 16.0 * (CHIP.x + CHIP_GAP) + CHIP.y, "{wide:?}");
    }
}

/// "Fade other layers" through the real canvas: the quad each layer paints.
#[cfg(test)]
mod fade_tests {
    use super::*;
    use egui::{pos2, Pos2};

    #[test]
    fn other_layers_paint_at_their_fade_and_the_active_one_at_full() {
        let mut state = AppState::for_test();
        state.show_panels = false;
        state.show_mini_timeline = false;
        // Three layers, each with a drawing on frame 0.
        while state.project.layers.len() < 3 {
            state.project.add_layer();
        }
        for li in 0..3 {
            state.project.current_layer = li;
            if state.project.layers[li].resolve(0).is_none() {
                state.structural_edit(false, |p| {
                    p.insert_blank_key_here();
                });
            }
        }
        state.project.current_layer = 1;
        state.fade = crate::app::FadeOthers { above: 0.25, below: 0.5 };
        state.fade_others = true;
        let cell_of = |li: usize| state.project.layers[li].resolve(0).unwrap();
        let (below, active, above) = (cell_of(0), cell_of(1), cell_of(2));
        // A blank drawing paints nothing: a dot of ink in each.
        for id in [below, active, above] {
            state.project.cell_mut(id).unwrap().pixels[3] = 255;
        }

        let ctx = egui::Context::default();
        let mut meshes = Vec::new();
        for time in [0.0, 1.0] {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_max(Pos2::ZERO, pos2(1200.0, 800.0))),
                time: Some(time),
                ..Default::default()
            };
            let out = ctx.run(raw, |ctx| {
                state.sync_textures(ctx);
                draw(&mut state, ctx);
            });
            meshes = out
                .shapes
                .into_iter()
                .filter_map(|c| match c.shape {
                    egui::Shape::Mesh(m) => Some((m.texture_id, m.vertices[0].color.a())),
                    _ => None,
                })
                .collect();
        }
        let alpha = |id: CellId| {
            let tex = state.cell_textures.get(&id).expect("texture built").tex.id();
            meshes.iter().find(|&&(t, _)| t == tex).expect("painted").1
        };
        assert_eq!(alpha(active), 255);
        assert_eq!(alpha(above), (0.25f32 * 255.0) as u8);
        assert_eq!(alpha(below), (0.5f32 * 255.0) as u8);
    }
}

#[cfg(test)]
mod panel_drag_tests {
    use super::*;
    use egui::{pos2, vec2, Pos2};

    fn frame(ctx: &egui::Context, state: &mut AppState, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_max(Pos2::ZERO, pos2(1600.0, 900.0))),
            events,
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            panel_window(state, ctx, PanelId::Onion, [300.0, 200.0], 252.0, true, None);
        });
    }

    fn panel_rect(ctx: &egui::Context) -> Rect {
        egui::AreaState::load(ctx, egui::Id::new(panel_key(PanelId::Onion)))
            .expect("panel laid out")
            .rect()
    }

    /// Press at `from`, drag 40px by `dir`, release; the panel's rect after.
    fn drag(from: Pos2, dir: Vec2) -> (Rect, Rect) {
        let mut state = AppState::for_test();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&ctx, &mut state, vec![]);
        }
        let before = panel_rect(&ctx);
        let from = before.min + from.to_vec2();
        let to = from + dir * 40.0;
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from), button(from, true)]);
        for i in 1..=4 {
            let p = from + (to - from) * (i as f32 / 4.0);
            frame(&ctx, &mut state, vec![egui::Event::PointerMoved(p)]);
        }
        frame(&ctx, &mut state, vec![button(to, false)]);
        frame(&ctx, &mut state, vec![]);
        (before, panel_rect(&ctx))
    }

    #[test]
    fn dragging_the_body_leaves_the_panel_where_it_is() {
        // Offsets from the panel's top-left: the bottom margin, clear of any
        // control, and beside the "Enabled" checkbox.
        for at in [pos2(40.0, -8.0), pos2(200.0, 50.0)] {
            let mut state = AppState::for_test();
            let ctx = egui::Context::default();
            frame(&ctx, &mut state, vec![]);
            let h = panel_rect(&ctx).height();
            let at = if at.y < 0.0 { pos2(at.x, h + at.y) } else { at };
            let (before, after) = drag(at, vec2(1.0, 1.0));
            assert_eq!(after.min, before.min, "grabbed at {at:?}");
        }
    }

    #[test]
    fn dragging_the_title_bar_moves_the_panel() {
        let (before, after) = drag(pos2(120.0, 14.0), vec2(1.0, 1.0));
        assert_eq!(after.min - before.min, vec2(40.0, 40.0));
    }

    #[test]
    fn controls_under_the_catcher_still_work() {
        let mut state = AppState::for_test();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&ctx, &mut state, vec![]);
        }
        let at = panel_rect(&ctx).min + vec2(24.0, 50.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let was = state.onion.enabled;
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(at), button(true)]);
        frame(&ctx, &mut state, vec![button(false)]);
        assert_ne!(state.onion.enabled, was, "the Enabled checkbox took the click");
    }

    #[test]
    fn the_edges_still_resize() {
        let (before, after) = drag(pos2(1.0, 150.0), vec2(-1.0, 0.0));
        assert!(after.left() < before.left() - 30.0, "{before:?} → {after:?}");
        assert_eq!(after.right(), before.right());
    }
}

/// Which source a stroke draws from, frame by frame.
#[cfg(test)]
mod stroke_source_tests {
    use super::{stroke_frame_samples, PenFrame};
    use crate::input::tablet::PenPacket;
    use egui::pos2;

    fn packet(x: f32, y: f32) -> PenPacket {
        PenPacket {
            x,
            y,
            pressure: 0.5,
            tilt_x: 0.0,
            tilt_y: 0.0,
        }
    }

    /// The wobble regression. A pen stroke on a frame with no packet must add
    /// nothing — not the cursor, which is the same point rounded to a pixel.
    #[test]
    fn a_pen_frame_without_packets_adds_nothing() {
        let mut from_pen = true;
        let out = stroke_frame_samples(&mut from_pen, PenFrame::Empty, Some(pos2(10.0, 10.0)));
        assert!(out.is_empty(), "took {out:?} from the cursor");
        assert!(from_pen);
    }

    #[test]
    fn a_pen_frame_takes_every_packet_and_only_packets() {
        let mut from_pen = true;
        let points = vec![
            (pos2(10.25, 10.5), packet(10.25, 10.5)),
            (pos2(11.75, 10.5), packet(11.75, 10.5)),
        ];
        let out = stroke_frame_samples(&mut from_pen, PenFrame::Points(points), Some(pos2(12.0, 11.0)));
        let at: Vec<_> = out.iter().map(|(p, _)| *p).collect();
        assert_eq!(at, vec![pos2(10.25, 10.5), pos2(11.75, 10.5)]);
        assert!(out.iter().all(|(_, p)| p.is_some()));
    }

    /// A mapping that stops agreeing hands the stroke to the cursor once, and
    /// the stroke stays there: switching back and forth is the zigzag again.
    #[test]
    fn an_untrusted_pen_hands_over_to_the_cursor_for_good() {
        let mut from_pen = true;
        let out = stroke_frame_samples(&mut from_pen, PenFrame::Untrusted, Some(pos2(5.0, 6.0)));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, pos2(5.0, 6.0));
        assert!(out[0].1.is_none());
        assert!(!from_pen);

        let points = vec![(pos2(7.5, 6.0), packet(7.5, 6.0))];
        let out = stroke_frame_samples(&mut from_pen, PenFrame::Points(points), Some(pos2(8.0, 6.0)));
        assert_eq!(out.len(), 1);
        assert!(out[0].1.is_none(), "went back to the pen mid-stroke");
    }

    #[test]
    fn a_cursor_stroke_ignores_packets() {
        let mut from_pen = false;
        let points = vec![(pos2(7.5, 6.0), packet(7.5, 6.0))];
        let out = stroke_frame_samples(&mut from_pen, PenFrame::Points(points), Some(pos2(8.0, 6.0)));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, pos2(8.0, 6.0));
        assert!(out[0].1.is_none());
    }
}

/// The New project dialog's saved presets, drawn headless.
#[cfg(test)]
mod new_project_tests {
    use super::*;
    use crate::app::{LayerRename, ProjectPreset};

    fn preset(name: &str, width: u32, height: u32, is_default: bool) -> ProjectPreset {
        ProjectPreset {
            name: name.into(),
            width,
            height,
            fps: 24.0,
            is_default,
        }
    }

    #[test]
    fn dialog_draws_saved_presets_and_a_rename_in_progress() {
        let mut state = AppState::for_test();
        state.project_presets = vec![
            preset("Phone", 1080, 1920, true),
            preset("Square", 1080, 1080, false),
            preset("Cinema", 2048, 858, false),
        ];
        state.open_new_project();
        state.new_project_cfg.preset_rename = Some(LayerRename {
            index: 1,
            buf: "Square".into(),
            focused: false,
        });
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| new_project_dialog(&mut state, ctx));
        }
        assert!(state.show_new_project);
        assert_eq!(state.project_presets.len(), 3);
        let r = state.new_project_cfg.preset_rename.as_ref().expect("rename still open");
        assert!(r.focused, "rename box took focus");
        // Opened on the starred preset, which the list shows as current.
        let cfg = &state.new_project_cfg;
        assert_eq!((cfg.width, cfg.height), (1080, 1920));
    }
}

/// The Timeline panel's edges through the real window, with the app's theme:
/// its tracks fill the panel, so any slack in egui's resize arithmetic shows.
#[cfg(test)]
mod timeline_panel_tests {
    use super::*;
    use egui::{pos2, vec2, Pos2};

    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, pos2(1200.0, 800.0));

    fn frame(ctx: &egui::Context, state: &mut AppState, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(SCREEN),
            events,
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| draw(state, ctx));
    }

    fn panel(ctx: &egui::Context) -> Rect {
        egui::AreaState::load(ctx, egui::Id::new(panel_key(PanelId::Timeline)))
            .unwrap()
            .rect()
    }

    /// Press at `from`, drag by `by` over several frames, release.
    fn drag(ctx: &egui::Context, state: &mut AppState, from: Pos2, by: egui::Vec2) {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(ctx, state, vec![egui::Event::PointerMoved(from)]);
        frame(ctx, state, vec![button(from, true)]);
        for i in 1..=12 {
            let p = from + by * (i as f32 / 12.0);
            frame(ctx, state, vec![egui::Event::PointerMoved(p)]);
        }
        frame(ctx, state, vec![button(from + by, false)]);
        frame(ctx, state, vec![]);
    }

    fn setup() -> (AppState, egui::Context) {
        let mut state = AppState::for_test();
        state.show_panels = true;
        let ctx = egui::Context::default();
        // The theme is what makes the title bar taller than egui budgets for.
        crate::ui::theme::install(&ctx);
        for _ in 0..4 {
            frame(&ctx, &mut state, vec![]);
        }
        (state, ctx)
    }

    #[test]
    fn dragging_a_side_edge_keeps_the_height() {
        let (mut state, ctx) = setup();
        let before = panel(&ctx);
        assert!(title_bar_shortfall(&ctx, PanelId::Timeline) > 0.0, "the case that grew");
        // Edges follow the pointer, wherever on the grab zone it pressed.
        let from = pos2(before.max.x - 1.0, before.center().y);
        drag(&ctx, &mut state, from, vec2(60.0, 0.0));
        let after = panel(&ctx);
        assert!((after.max.x - (from.x + 60.0)).abs() < 1.0, "{before:?} → {after:?}");
        assert!((after.height() - before.height()).abs() < 1.0, "{before:?} → {after:?}");
        let from = pos2(after.min.x + 1.0, after.center().y);
        drag(&ctx, &mut state, from, vec2(40.0, 0.0));
        let after = panel(&ctx);
        assert!((after.min.x - (from.x + 40.0)).abs() < 1.0, "{after:?}");
        assert!((after.height() - before.height()).abs() < 1.0, "left edge too: {after:?}");
    }

    #[test]
    fn dragging_the_top_or_bottom_edge_moves_just_that_edge() {
        let (mut state, ctx) = setup();
        let before = panel(&ctx);
        let from = pos2(before.center().x, before.min.y + 1.0);
        drag(&ctx, &mut state, from, vec2(0.0, -30.0));
        let taller = panel(&ctx);
        assert!((taller.min.y - (from.y - 30.0)).abs() < 1.0, "{before:?} → {taller:?}");
        assert!((taller.max.y - before.max.y).abs() < 1.0, "the bottom stays put");
        let from = pos2(taller.center().x, taller.max.y - 1.0);
        drag(&ctx, &mut state, from, vec2(0.0, -50.0));
        let shorter = panel(&ctx);
        assert!((shorter.max.y - (from.y - 50.0)).abs() < 1.0, "{taller:?} → {shorter:?}");
        assert!((shorter.min.y - taller.min.y).abs() < 1.0, "the top stays put");
    }
}

/// The Layers panel through the real window: drag-and-drop reordering, and
/// the sections folded into collapsing headers.
#[cfg(test)]
mod layers_panel_tests {
    use super::*;
    use egui::{pos2, Pos2};

    // Tall enough that the Timeline, which a fresh layout opens tall, sits
    // below the Layers panel instead of over its rows.
    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, pos2(1280.0, 1400.0));
    const NAMES: [&str; 3] = ["Alpha", "Bravo", "Charlie"];

    /// Run a frame; returns what it painted.
    fn frame(
        ctx: &egui::Context,
        state: &mut AppState,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        let raw = egui::RawInput {
            screen_rect: Some(SCREEN),
            events,
            ..Default::default()
        };
        ctx.run(raw, |ctx| draw(state, ctx)).shapes
    }

    /// Alpha (bottom), Bravo, Charlie (top), with the panels up and themed.
    /// Returns the last frame's shapes too.
    fn setup() -> (AppState, egui::Context, Vec<egui::epaint::ClippedShape>) {
        let mut state = AppState::for_test();
        state.show_panels = true;
        state.project.layers[0].name = NAMES[0].into();
        for name in &NAMES[1..] {
            state.project.add_layer();
            let i = state.project.current_layer;
            state.project.layers[i].name = (*name).into();
        }
        let ctx = egui::Context::default();
        crate::ui::theme::install(&ctx);
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame(&ctx, &mut state, vec![]);
        }
        (state, ctx, shapes)
    }

    /// Whether any text painted inside the Layers panel contains `glyph`.
    fn panel_shows(
        ctx: &egui::Context,
        shapes: &[egui::epaint::ClippedShape],
        glyph: &str,
    ) -> bool {
        fn find(shape: &egui::Shape, glyph: &str, panel: Rect) -> bool {
            match shape {
                egui::Shape::Vec(v) => v.iter().any(|s| find(s, glyph, panel)),
                egui::Shape::Text(t) => {
                    t.galley.text().contains(glyph) && panel.contains_rect(t.visual_bounding_rect())
                }
                _ => false,
            }
        }
        let panel = egui::AreaState::load(ctx, egui::Id::new(panel_key(PanelId::Layers)))
            .expect("Layers panel laid out")
            .rect();
        shapes.iter().any(|s| find(&s.shape, glyph, panel))
    }

    #[test]
    fn clipped_and_alpha_locked_layers_are_marked_in_the_panel() {
        let (mut state, ctx, shapes) = setup();
        assert!(!panel_shows(&ctx, &shapes, ic::ARROW_BEND_LEFT_DOWN));
        assert!(!panel_shows(&ctx, &shapes, ic::CHECKERBOARD));
        state.project.layers[1].clip = true;
        state.project.layers[2].alpha_lock = true;
        let mut shapes = Vec::new();
        for _ in 0..2 {
            shapes = frame(&ctx, &mut state, vec![]);
        }
        assert!(panel_shows(&ctx, &shapes, ic::ARROW_BEND_LEFT_DOWN));
        assert!(panel_shows(&ctx, &shapes, ic::CHECKERBOARD));
    }

    fn names(state: &AppState) -> Vec<&str> {
        state.project.layers.iter().map(|l| l.name.as_str()).collect()
    }

    /// Where `name` was painted inside the Layers panel — the layer's name
    /// label, which is also its drag handle. (The Timeline lists the names
    /// too, so the search is held to the panel.)
    fn name_rect(ctx: &egui::Context, shapes: &[egui::epaint::ClippedShape], name: &str) -> Rect {
        fn find(shape: &egui::Shape, name: &str, panel: Rect, out: &mut Option<Rect>) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| find(s, name, panel, out)),
                egui::Shape::Text(t) if t.galley.text() == name => {
                    let r = t.visual_bounding_rect();
                    if panel.contains_rect(r) {
                        *out = Some(r);
                    }
                }
                _ => {}
            }
        }
        let panel = egui::AreaState::load(ctx, egui::Id::new(panel_key(PanelId::Layers)))
            .expect("Layers panel laid out")
            .rect();
        let mut out = None;
        for s in shapes {
            find(&s.shape, name, panel, &mut out);
        }
        out.unwrap_or_else(|| panic!("{name} not painted in the Layers panel"))
    }

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn drop_index_lands_just_above_or_below_the_target() {
        // Up the stack: taking the layer out first shifts the target down.
        assert_eq!(drop_index(0, 2, true), 2);
        assert_eq!(drop_index(0, 2, false), 1);
        // Down the stack.
        assert_eq!(drop_index(3, 1, true), 2);
        assert_eq!(drop_index(3, 1, false), 1);
        // Onto itself, or into the slot it already fills: no move.
        assert_eq!(drop_index(2, 2, true), 2);
        assert_eq!(drop_index(2, 2, false), 2);
        assert_eq!(drop_index(1, 2, false), 1);
        assert_eq!(drop_index(2, 1, true), 2);
    }

    #[test]
    fn dragging_a_layer_name_onto_another_row_reorders_in_one_undo() {
        let (mut state, ctx, shapes) = setup();
        state.project.current_layer = 1;
        let from = name_rect(&ctx, &shapes, "Alpha").center();
        // The name sits in the top half of its row: drop Alpha above Charlie.
        let to = name_rect(&ctx, &shapes, "Charlie").center();
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from)]);
        frame(&ctx, &mut state, vec![button(from, true)]);
        for k in 1..=8 {
            let p = from + (to - from) * (k as f32 / 8.0);
            frame(&ctx, &mut state, vec![egui::Event::PointerMoved(p)]);
        }
        frame(&ctx, &mut state, vec![button(to, false)]);
        frame(&ctx, &mut state, vec![]);

        assert_eq!(names(&state), ["Bravo", "Charlie", "Alpha"]);
        assert_eq!(state.project.current_layer, 2, "the dropped layer is selected");
        state.undo();
        assert_eq!(names(&state), NAMES);
    }

    #[test]
    fn a_click_on_a_name_still_selects_and_a_double_click_renames() {
        let (mut state, ctx, shapes) = setup();
        state.project.current_layer = 2;
        let at = name_rect(&ctx, &shapes, "Alpha").center();
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(at), button(at, true)]);
        frame(&ctx, &mut state, vec![button(at, false)]);
        frame(&ctx, &mut state, vec![]);
        assert_eq!(state.project.current_layer, 0);
        assert_eq!(names(&state), NAMES, "a click is not a drag");

        frame(&ctx, &mut state, vec![button(at, true)]);
        frame(&ctx, &mut state, vec![button(at, false), button(at, true)]);
        frame(&ctx, &mut state, vec![button(at, false)]);
        assert!(
            state.layer_rename.as_ref().is_some_and(|r| r.index == 0),
            "double-click opens the rename box"
        );
    }

    #[test]
    fn the_folded_sections_still_draw() {
        let (mut state, ctx, _) = setup();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(SCREEN),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    layer_transform_section(&mut state, ui);
                    layer_canvas_section(&mut state, ui);
                });
            },
        );
    }
}

/// The Lasso's options through the real window.
#[cfg(test)]
mod lasso_options_tests {
    use super::*;
    use egui::{pos2, Pos2};

    /// Every piece of text one frame painted.
    fn texts(ctx: &egui::Context, state: &mut AppState) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                _ => {}
            }
        }
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_max(Pos2::ZERO, pos2(1280.0, 1400.0))),
            ..Default::default()
        };
        let mut out = Vec::new();
        for s in ctx.run(raw, |ctx| draw(state, ctx)).shapes {
            walk(&s.shape, &mut out);
        }
        out
    }

    #[test]
    fn all_frames_is_offered_and_counts_the_drawings() {
        let mut state = AppState::for_test();
        state.show_panels = true;
        state.set_tool(ActiveTool::Lasso);
        state.project.ensure_active_cell();
        for _ in 0..2 {
            state.structural_edit(false, |p| {
                p.add_frame();
                p.insert_blank_key_here();
            });
            state.project.ensure_active_cell();
        }
        let ctx = egui::Context::default();
        crate::ui::theme::install(&ctx);
        texts(&ctx, &mut state);
        let t = texts(&ctx, &mut state);
        assert!(t.iter().any(|s| s == "All frames"), "the checkbox");
        assert!(!t.iter().any(|s| s.starts_with("Applies to")), "no count while off");

        state.sel_all_frames = true;
        let t = texts(&ctx, &mut state);
        assert!(
            t.iter().any(|s| s == "Applies to 3 drawings on this layer"),
            "{t:?}"
        );
    }
}

/// A Fill press through the real canvas: egui events in, pixels out.
#[cfg(test)]
mod fill_tests {
    use super::*;
    use egui::{pos2, vec2, Pos2};

    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, pos2(1200.0, 800.0));
    const RED: [u8; 4] = [255, 0, 0, 255];

    fn frame(ctx: &egui::Context, state: &mut AppState, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(SCREEN),
            events,
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| draw(state, ctx));
    }

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn px(state: &AppState, x: u32, y: u32) -> [u8; 4] {
        let c = state.project.cell(state.project.resolved_current().unwrap()).unwrap();
        let k = ((y * c.width + x) * 4) as usize;
        [c.pixels[k], c.pixels[k + 1], c.pixels[k + 2], c.pixels[k + 3]]
    }

    #[test]
    fn dragging_a_fill_press_sideways_shuts_the_gap() {
        let mut state = AppState::for_test();
        state.show_panels = false;
        state.show_mini_timeline = false;
        state.dispatch(Action::ToolFill);
        state.brush.color = RED;
        state.brush.fill_gap = 0;
        // A box with a 4-px hole in its top edge.
        let id = state.project.ensure_active_cell();
        let c = state.project.cell_mut(id).unwrap();
        for i in 100..=300u32 {
            for (x, y) in [(i, 100), (i, 300), (100, i), (300, i)] {
                if !(y == 100 && (190..194).contains(&x)) {
                    let k = ((y * c.width + x) * 4) as usize;
                    c.pixels[k..k + 4].copy_from_slice(&[0, 0, 0, 255]);
                }
            }
        }
        let ctx = egui::Context::default();
        frame(&ctx, &mut state, vec![]);
        let from = Xform::new(&state, SCREEN).doc_to_screen(200.0, 200.0);

        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from), button(from, true)]);
        // Past the dead zone and into the drag proper, in a few moves.
        for dx in [4.0, 30.0, 60.0] {
            frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from + vec2(dx, 0.0))]);
        }
        assert!(state.fill_dragging());
        assert_eq!(state.brush.fill_gap, 4);
        assert_eq!(px(&state, 200, 200), RED);
        assert_eq!(px(&state, 20, 20)[3], 0, "held inside the box");
        frame(&ctx, &mut state, vec![button(from + vec2(60.0, 0.0), false)]);
        frame(&ctx, &mut state, vec![]);
        assert!(!state.fill_dragging());
        assert_eq!(state.history.undo_len(), 1);
    }
}

/// Middle- and right-button drags through the real canvas.
#[cfg(test)]
mod button_drag_tests {
    use super::*;
    use egui::{pos2, vec2, Pos2};

    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, pos2(1200.0, 800.0));

    fn frame(ctx: &egui::Context, state: &mut AppState, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(SCREEN),
            events,
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| draw(state, ctx));
    }

    fn state() -> AppState {
        let mut state = AppState::for_test();
        state.show_panels = false;
        state.show_mini_timeline = false;
        while state.project.frame_count < 12 {
            state.project.add_frame();
        }
        state.project.goto(0);
        state
    }

    /// Press `button` at `from`, drag by `by` in a few moves, and let go.
    fn drag(
        ctx: &egui::Context,
        state: &mut AppState,
        button: egui::PointerButton,
        from: Pos2,
        by: Vec2,
    ) {
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(ctx, state, vec![egui::Event::PointerMoved(from), press(from, true)]);
        for i in 1..=4 {
            frame(ctx, state, vec![egui::Event::PointerMoved(from + by * (i as f32 / 4.0))]);
        }
        frame(ctx, state, vec![press(from + by, false)]);
        frame(ctx, state, vec![]);
    }

    #[test]
    fn a_middle_drag_scrubs_by_default_and_draws_nothing() {
        let mut state = state();
        let ctx = egui::Context::default();
        frame(&ctx, &mut state, vec![]);
        let cells = state.project.cells.len();
        let pan = state.view.pan;
        drag(&ctx, &mut state, egui::PointerButton::Middle, pos2(600.0, 400.0), vec2(35.0, 0.0));
        assert_eq!(state.project.current_frame, 3);
        assert_eq!(state.view.pan, pan, "no longer pans");
        assert_eq!(state.project.cells.len(), cells);
        assert!(state.button_drag.is_none() && state.stroke.is_none());
    }

    #[test]
    fn a_right_drag_picks_from_the_pie_by_default() {
        let mut state = state();
        state.palette = (0..10u8).map(|i| [i * 20, 0, 0]).collect();
        let ctx = egui::Context::default();
        frame(&ctx, &mut state, vec![]);
        let cells = state.project.cells.len();
        drag(&ctx, &mut state, egui::PointerButton::Secondary, pos2(600.0, 400.0), vec2(50.0, 0.0));
        assert_eq!(state.brush.color, [40, 0, 0, 255]);
        assert_eq!(state.project.cells.len(), cells, "a right drag no longer draws");
        assert_eq!(state.history.undo_len(), 0);
    }

    #[test]
    fn a_button_bound_to_pan_still_pans() {
        let mut state = state();
        state.middle_drag = ButtonDrag::Pan;
        let ctx = egui::Context::default();
        frame(&ctx, &mut state, vec![]);
        let pan = state.view.pan;
        drag(&ctx, &mut state, egui::PointerButton::Middle, pos2(600.0, 400.0), vec2(40.0, 20.0));
        assert!((state.view.pan - pan - vec2(40.0, 20.0)).length() < 1.0, "{:?}", state.view.pan);
        assert_eq!(state.project.current_frame, 0);
    }

    #[test]
    fn settings_shows_the_button_pickers() {
        let mut state = state();
        state.show_settings = true;
        let ctx = egui::Context::default();
        frame(&ctx, &mut state, vec![]);
        frame(&ctx, &mut state, vec![]);
    }
}

/// The Krita brush through the real UI: the preset is offered, its summary
/// replaces the sliders it does not use, and a drag on the canvas paints
/// with it as one undoable stroke.
#[cfg(test)]
mod krita_brush_tests {
    use super::*;
    use crate::tools::BrushSettings;
    use egui::{pos2, vec2, Pos2};

    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, pos2(1200.0, 900.0));

    fn run(ctx: &egui::Context, state: &mut AppState, events: Vec<egui::Event>) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                _ => {}
            }
        }
        let raw = egui::RawInput {
            screen_rect: Some(SCREEN),
            events,
            ..Default::default()
        };
        let mut out = Vec::new();
        for s in ctx.run(raw, |ctx| draw(state, ctx)).shapes {
            walk(&s.shape, &mut out);
        }
        out
    }

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn the_krita_pencil_is_offered_and_paints_through_the_canvas() {
        let mut state = AppState::for_test();
        state.show_panels = false;
        state.show_mini_timeline = false;
        state.dispatch(Action::ToolPencil);
        state.show_brush_settings = true;
        let ctx = egui::Context::default();
        crate::ui::theme::install(&ctx);
        run(&ctx, &mut state, vec![]);
        let t = run(&ctx, &mut state, vec![]);
        assert!(t.iter().any(|s| s == "Pencil-5"), "preset button");
        assert!(!t.iter().any(|s| s.starts_with("Krita · ")), "no summary yet");

        state.brush = BrushSettings::pencil5_krita();
        let t = run(&ctx, &mut state, vec![]);
        assert!(t.iter().any(|s| s == "Krita · Pencil-5 Tilted"), "{t:?}");
        assert!(!t.iter().any(|s| s == "Hardness"), "sliders it ignores are gone");

        state.show_brush_settings = false;
        run(&ctx, &mut state, vec![]);
        let id = state.project.ensure_active_cell();
        let from = Xform::new(&state, SCREEN).doc_to_screen(300.0, 300.0);
        run(&ctx, &mut state, vec![egui::Event::PointerMoved(from), button(from, true)]);
        for i in 1..=30 {
            run(&ctx, &mut state, vec![egui::Event::PointerMoved(from + vec2(i as f32 * 6.0, 0.0))]);
        }
        run(&ctx, &mut state, vec![button(from + vec2(180.0, 0.0), false)]);
        run(&ctx, &mut state, vec![]);

        let c = state.project.cell(id).unwrap();
        let painted = c.pixels.chunks(4).filter(|p| p[3] > 0).count();
        assert!(painted > 100, "painted {painted} pixels");
        assert_eq!(state.history.undo_len(), 1, "one stroke, one undo step");
    }

    /// Every closed outline one frame drew.
    fn closed_paths(ctx: &egui::Context, state: &mut AppState, events: Vec<egui::Event>) -> Vec<Vec<Pos2>> {
        fn walk(shape: &egui::Shape, out: &mut Vec<Vec<Pos2>>) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                egui::Shape::Path(p) if p.closed => out.push(p.points.clone()),
                _ => {}
            }
        }
        let raw = egui::RawInput {
            screen_rect: Some(SCREEN),
            events,
            ..Default::default()
        };
        let mut out = Vec::new();
        for s in ctx.run(raw, |ctx| draw(state, ctx)).shapes {
            walk(&s.shape, &mut out);
        }
        out
    }

    /// Over the canvas a Krita brush shows its tip's outline — a thin
    /// rectangle round the pointer — instead of the round brush ring.
    #[test]
    fn the_krita_pencil_cursor_is_its_tip_outline() {
        let mut state = AppState::for_test();
        state.show_panels = false;
        state.show_mini_timeline = false;
        state.dispatch(Action::ToolPencil);
        state.brush = BrushSettings::pencil5_krita();
        let ctx = egui::Context::default();
        crate::ui::theme::install(&ctx);
        closed_paths(&ctx, &mut state, vec![]);
        let at = Xform::new(&state, SCREEN).doc_to_screen(300.0, 300.0);
        let paths = closed_paths(&ctx, &mut state, vec![egui::Event::PointerMoved(at)]);
        // The canvas frame is a closed rectangle too; the outline is the one
        // round the pointer.
        let around = |r: &&Vec<Pos2>| {
            let mid = (r[0].to_vec2() + r[2].to_vec2()) / 2.0;
            (mid - at.to_vec2()).length() < 3.0
        };
        let rects: Vec<_> = paths.iter().filter(|p| p.len() == 4).filter(around).collect();
        assert!(!rects.is_empty(), "no outline round the pointer among {paths:?}");
        for r in rects {
            let (a, b) = ((r[1] - r[0]).length(), (r[2] - r[1]).length());
            assert!(a.max(b) > 3.0 * a.min(b), "a thin bar: {a} x {b}");
        }
    }
}

#[cfg(test)]
mod color_panel_layout_tests {
    use super::*;
    use egui::{pos2, Pos2};

    #[test]
    fn the_color_panel_opens_under_brush_on_a_1080p_screen() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut state = AppState::for_test();
        // A few frames: windows settle their size after the first.
        for _ in 0..4 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_max(Pos2::ZERO, pos2(1920.0, 1080.0))),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| draw(&mut state, ctx));
        }
        let rect = |id| {
            egui::AreaState::load(&ctx, egui::Id::new(panel_key(id)))
                .expect("panel laid out")
                .rect()
        };
        let (brush, color) = (rect(PanelId::Brush), rect(PanelId::Color));
        assert!(color.top() >= brush.bottom(), "{brush:?} vs {color:?}");
        assert!(color.bottom() <= 1080.0, "{color:?}");
        assert!(!color.intersects(rect(PanelId::Timeline)), "{color:?}");
    }
}
