//! Top-level egui layout: modern dark UI with Phosphor icons.
//! Floating windows, transparent canvas, layered + onion composite.

use egui::{Align, Color32, Frame, Margin, Rect, Sense, Stroke, Vec2};
use egui_phosphor::regular as ic;

use crate::app::{AppState, ExportKind, NavKind, PanelId, MP4_PRESETS};
use crate::doc::camera::Ease;
use crate::input::shortcuts::{Action, KeyCombo};
use crate::input::tablet::PenPacket;
use crate::io::{composite, png_import, png_save, project_file};
use crate::timeline::onion::OnionDirection;
use crate::tools::{ActiveTool, ShapeKind};
use crate::ui::{expr, theme};

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
    new_project_dialog(state, ctx);
    export_dialog(state, ctx);
    import_range_dialog(state, ctx);
    if let Some(label) = state.bg_label {
        busy_overlay(ctx, label);
    }
    save_error_dialog(state, ctx);
    save_toast(state, ctx);
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
                state.nav_to_camera =
                    !state.nav_to_layer && state.camera_edit && nav_gesture.is_some();
                if state.nav_to_layer {
                    state.begin_layer_xform();
                } else if state.nav_to_camera {
                    state.begin_camera_drag();
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
                        None => {
                            let t = ui.input(|i| i.time as f32);
                            let pointer = resp.interact_pointer_pos();
                            // Tablet packets first: several per frame, each
                            // with its own sub-pixel position and pressure.
                            // `pen_stroke_points` returns None when the pen is
                            // not the live device or when its mapping does not
                            // agree with the OS pointer, and the single egui
                            // position is used instead.
                            match pen_stroke_points(state, ui.ctx(), pointer) {
                                Some(points) => {
                                    for (pos, packet) in points {
                                        let (cx, cy) =
                                            doc_to_active_cell(state, canvas_to_doc(pos));
                                        let s = state.make_pen_sample(cx, cy, t, &packet);
                                        state.pointer_move(s);
                                    }
                                }
                                None => {
                                    if let Some(pos) = pointer {
                                        let (cx, cy) =
                                            doc_to_active_cell(state, canvas_to_doc(pos));
                                        let s = state.make_sample(cx, cy, t);
                                        state.pointer_move(s);
                                    }
                                }
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
                } else if state.nav_drag.is_none() {
                    state.pointer_up();
                }
                state.nav_drag = None;
                state.nav_to_layer = false;
                state.nav_to_camera = false;
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
        PanelId::Camera => (ic::VIDEO_CAMERA, "Camera"),
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
    let scrubs = layer.order == egui::Order::Background
        || layer.id == egui::Id::new(panel_key(PanelId::Timeline))
        || layer.id == egui::Id::new("mini_timeline");
    if !scrubs {
        return;
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
    window.show(ctx, |ui| panel_content(state, ctx, ui, id));
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
                ui.add(egui::Slider::new(&mut state.brush.fill_expand, 0..=8).text("Expand (px)"))
                    .on_hover_text(
                        "Grow the fill by this many pixels so the colour tucks under \
                         anti-aliased lines instead of leaving a halo. Meant for the \
                         'lines from' workflow — on a same-layer fill it eats into \
                         your own strokes.",
                    );
                // The boundary source is a per-layer link, set in the Layers
                // panel — surface it here so the coupling is visible.
                let hint = match state.fill_boundary_name() {
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
            } else if state.tool == ActiveTool::Lasso {
                ui.label(
                    egui::RichText::new(
                        "Draw a loop to select — the path closes itself on release. \
                         Drag inside it to move the pixels, or nudge with the arrow \
                         keys.",
                    )
                    .color(theme::TEXT_MUTED)
                    .size(11.0),
                );
                let del = combo_text(state, Action::SelectionDelete);
                let cut = combo_text(state, Action::SelectionCut);
                let copy = combo_text(state, Action::SelectionCopy);
                let paste = combo_text(state, Action::SelectionPaste);
                let off = combo_text(state, Action::SelectionDeselect);
                ui.label(
                    egui::RichText::new(format!(
                        "{del} erases it - {cut} / {copy} / {paste} move it between \
                         frames and layers - {off} drops it in place. Changing frame \
                         or tool commits it.",
                    ))
                    .color(theme::TEXT_MUTED)
                    .size(11.0),
                );
            } else {
                brush_size_lock(state, ui);
                let label = if state.lock_brush_to_view {
                    "Size (screen px)"
                } else {
                    "Size"
                };
                ui.add(egui::Slider::new(&mut state.brush.radius, 0.5..=128.0).text(label));
                ui.add(egui::Slider::new(&mut state.brush.opacity, 0.0..=1.0).text("Flow"));
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
            resp.on_hover_text(format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]));
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
                                state.set_brush_color([r, g, b]);
                            }
                        }
                    }
                }
            });
            swatch_strip(state, ui);

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
                if loop_toggle(ui, state).clicked() {
                    state.loop_timeline = !state.loop_timeline;
                }
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
                // One value for both halves of the toolbar: how far the arrows
                // move, and how many frames the + / copy buttons insert.
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
                ui.separator();
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
                ui.separator();
                ui.add(egui::Slider::new(&mut state.project.fps, 1.0..=60.0).text("fps"));
            });

            ui.add_space(4.0);

            let n = state.project.frame_count.max(1);
            let mut cur = state.project.current_frame;
            // Frozen before the widgets are built: a relative expression must
            // measure from where the edit started, not from a value the edit
            // has already moved.
            let base = cur as f64;
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(ic::CLOCK)
                        .color(theme::TEXT_MUTED)
                        .size(13.0),
                );
                // Its own DragValue rather than the slider's built-in value:
                // `Slider` hard-codes update-while-editing, which would apply
                // the half-typed "+1" of "+12" and shift the base underfoot.
                let field = ui
                    .add(
                        egui::DragValue::new(&mut cur)
                            .range(0..=n.saturating_sub(1))
                            .speed(1)
                            .update_while_editing(false)
                            .custom_parser(move |s| expr::eval(s, base).map(f64::round)),
                    )
                    .on_hover_text(
                        "Frame number. Takes arithmetic: 22/2, 8+12, (4+8)*2.\n\n\
                         Start with an operator to go relative to this frame: \
                         +12 jumps 12 ahead, -3 back, /2 to the halfway frame. \
                         Enter applies it.",
                    );
                let track = ui.add(
                    egui::Slider::new(&mut cur, 0..=n.saturating_sub(1))
                        .integer()
                        .show_value(false)
                        .text("frame"),
                );
                if field.changed() || track.changed() {
                    state.project.goto(cur);
                }
                ui.label(egui::RichText::new(format!("/ {n}")).color(theme::TEXT_MUTED));
            });

            frame_strip(state, ui);
    }
}

/// Compact playback HUD shown when the floating panels are hidden (Tab).
/// Pinned bottom-centre: play/pause, step, frame counter, scrub strip.
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
        ActiveTool::Shape => ic::SHAPES,
        ActiveTool::Tracker => ic::CROSSHAIR,
        ActiveTool::Lasso => ic::LASSO,
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
                // Keyed frames get a tick under the dot — same two channels and
                // colours as the full frame strip.
                let ty = rect.max.y - 1.5;
                if layer_keys[i] {
                    let x = center.x - if camera_keys[i] { 2.5 } else { 0.0 };
                    painter.circle_filled(egui::pos2(x, ty), 1.5, KEY_LAYER);
                }
                if camera_keys[i] {
                    let x = center.x + if layer_keys[i] { 2.5 } else { 0.0 };
                    painter.circle_filled(egui::pos2(x, ty), 1.5, KEY_CAMERA);
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
const KEY_LAYER: Color32 = Color32::from_rgb(120, 160, 220);
/// Marker colour for camera keys — same amber as the camera-edit guide.
const KEY_CAMERA: Color32 = Color32::from_rgb(255, 190, 90);

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

fn frame_strip(state: &mut AppState, ui: &mut egui::Ui) {
    let n = state.project.frame_count.max(1);
    let cur = state.project.current_frame;
    let (layer_keys, camera_keys) = key_flags(state, n);
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
                // Keyframe markers: layer transform bottom-left, camera
                // bottom-right. Without these the only sign a frame is keyed is
                // a text count in the panels.
                let y = r.max.y - 4.0;
                if layer_keys[i] {
                    painter.circle_filled(egui::pos2(r.min.x + 5.0, y), 2.0, KEY_LAYER);
                }
                if camera_keys[i] {
                    painter.circle_filled(egui::pos2(r.max.x - 5.0, y), 2.0, KEY_CAMERA);
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
            ui.add(egui::Slider::new(&mut state.onion.max_alpha, 0.0..=1.0).text("Max α"));
            ui.add(egui::Slider::new(&mut state.onion.falloff, 0.5..=4.0).text("Falloff"));
            ui.add_space(4.0);
            theme::section_header(ui, ic::PALETTE, "Tints");
            color_picker_u8(ui, "Prev", &mut state.onion.prev_tint);
            color_picker_u8(ui, "Next", &mut state.onion.next_tint);
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

            // --- Layer canvas ---
            //
            // Strokes are clipped to the cell buffer, so a layer parked off to
            // the side of the camera still only gets a frame-sized sheet to
            // draw on until it's expanded here.
            ui.add_space(6.0);
            ui.separator();
            theme::section_header(ui, ic::FRAME_CORNERS, "Layer canvas");
            let (cur_w, cur_h) = state.active_layer_cell_size();
            let li = state.project.current_layer;
            let (mut ew, mut eh) = match state.expand_cfg {
                Some((l, w, h)) if l == li => (w, h),
                _ => (cur_w, cur_h),
            };
            ui.horizontal(|ui| {
                ui.label("W");
                ui.add(egui::DragValue::new(&mut ew).speed(8.0).range(1..=32768));
                ui.label("H");
                ui.add(egui::DragValue::new(&mut eh).speed(8.0).range(1..=32768));
            });
            state.expand_cfg = Some((li, ew, eh));
            ui.horizontal(|ui| {
                for (label, mul) in [("2×", 2u32), ("3×", 3)] {
                    if ui.small_button(label).clicked() {
                        state.expand_cfg =
                            Some((li, state.project.width * mul, state.project.height * mul));
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
            ui.checkbox(&mut state.invert_timeline_scroll, "Invert timeline scroll")
                .on_hover_text(
                    "Mouse wheel over the canvas or the timeline scrubs frames.\n\n\
                     Off (default): wheel down advances.  On: wheel up advances.\n\n\
                     Each notch moves by the timeline's step size (×N).",
                );
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

fn color_picker_u8(ui: &mut egui::Ui, label: &str, c: &mut [u8; 3]) {
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
        // Leaving the lasso puts a floating selection down first.
        state.commit_selection();
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

    // Onion ghosts of the active layer at nearby frames. Drawn inside the layer
    // loop so they sit at the active layer's depth (prev just behind its cell,
    // next just in front) instead of behind/above the whole stack.
    // Ghosts come from `ghost_textures` — silhouettes already baked in the
    // tint colour. The vertex colour only fades them: multiplying a tint over
    // the plain cell texture leaves black line art black.
    let draw_onion = |dir: OnionDirection| {
        // Farthest first so the nearest ghost — the most opaque one, and the
        // one the user is comparing against — ends up on top.
        for step in state.onion_steps(dir).into_iter().rev() {
            let Some((_, tex)) = state.ghost_textures.get(&step.cell) else {
                continue;
            };
            let Some(cell) = state.project.cell(step.cell) else {
                continue;
            };
            let a = state.onion.alpha_for(step.k, dir);
            let t = state.display_transform(cur_layer, step.frame);
            let lc = layer_screen_corners(&xf, t, cell.width as f32, cell.height as f32, pw, ph);
            image_quad(
                &painter,
                tex.id(),
                lc,
                Color32::from_rgba_unmultiplied(255, 255, 255, a),
            );
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
            draw_onion(OnionDirection::Prev);
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
            draw_onion(OnionDirection::Next);
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
        let thick = (state.effective_radius() * 2.0 * scale * layer_scale).max(1.0);
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

    // Lasso preview: the path so far plus a dashed-looking closing chord back
    // to the start, so it's obvious the loop seals itself on release. Drawn in
    // cell space like the other previews, since that is what gets rasterised.
    // Floating selection: the lifted pixels as a quad at their offset, plus
    // marching ants around the path so it reads as "selected", not "drawn".
    if let Some(sel) = &state.selection {
        if let Some(tex) = &state.selection_tex {
            let (ox, oy) = sel.origin();
            let (mw, mh) = (sel.mask.w as f32, sel.mask.h as f32);
            let (ox, oy) = (ox as f32, oy as f32);
            let quad = [
                cell_to_screen(ox, oy),
                cell_to_screen(ox + mw, oy),
                cell_to_screen(ox + mw, oy + mh),
                cell_to_screen(ox, oy + mh),
            ];
            image_quad(&painter, tex.id(), quad, Color32::WHITE);
        }
        if sel.path.len() >= 2 {
            let (dx, dy) = (sel.offset.0 as f32, sel.offset.1 as f32);
            let pts: Vec<egui::Pos2> = sel
                .path
                .iter()
                .map(|&(x, y)| cell_to_screen(x + dx, y + dy))
                .collect();
            let mut closed = pts.clone();
            closed.push(pts[0]);
            // Animated dash offset — the classic marching ants, which is what
            // tells a selection outline apart from an inked line.
            let phase = (ui.input(|i| i.time) * 24.0) as f32 % 12.0;
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
            ui.ctx().request_repaint();
        }
    }

    if let Some(path) = &state.lasso {
        if path.len() >= 2 {
            let pts: Vec<egui::Pos2> = path.iter().map(|&(x, y)| cell_to_screen(x, y)).collect();
            // Two-tone stroke: readable over both ink and empty canvas.
            painter.add(egui::Shape::line(
                pts.clone(),
                Stroke::new(2.2, Color32::from_black_alpha(180)),
            ));
            painter.add(egui::Shape::line(
                pts.clone(),
                Stroke::new(1.0, Color32::WHITE),
            ));
            let (first, last) = (pts[0], pts[pts.len() - 1]);
            painter.line_segment([last, first], Stroke::new(2.2, Color32::from_black_alpha(120)));
            painter.line_segment(
                [last, first],
                Stroke::new(1.0, Color32::from_white_alpha(140)),
            );
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
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(80, 80, 80, outline_a)),
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
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 160, 220, 110)),
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
            let r_cell = (state.effective_radius() * pressure_mul).max(0.5);
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

/// How far, in points, a packet-derived position may sit from the OS pointer
/// before the tablet mapping is disbelieved. Qt uses the same 20-pixel
/// manhattan check to spot a driver running in relative mode.
const PEN_MOUSE_AGREEMENT: f32 = 20.0;

/// This frame's tablet packets as egui screen positions, paired with the
/// packet they came from.
///
/// `None` means "use the egui pointer instead": no tablet, no packets this
/// frame, or a mapping that disagrees with the OS cursor. Positions are
/// sub-pixel and there are typically two to four per frame at 60 fps, against
/// the one whole-pixel position egui reports — which is the entire reason this
/// path exists.
fn pen_stroke_points(
    state: &AppState,
    ctx: &egui::Context,
    pointer: Option<egui::Pos2>,
) -> Option<Vec<(egui::Pos2, PenPacket)>> {
    if !state.pen.pen_active() {
        return None;
    }
    let packets = state.pen.packets();
    let last = packets.last()?;
    let (ox, oy) = state.pen.client_origin()?;
    let ppp = ctx.pixels_per_point();
    // Virtual-desktop physical pixels -> client physical pixels -> points.
    let to_points = |p: &PenPacket| egui::pos2((p.x - ox) / ppp, (p.y - oy) / ppp);

    if let Some(pointer) = pointer {
        let end = to_points(last);
        if (end.x - pointer.x).abs() + (end.y - pointer.y).abs() > PEN_MOUSE_AGREEMENT {
            return None;
        }
    }
    Some(packets.iter().map(|p| (to_points(p), *p)).collect())
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
                ExportKind::PngSequence => format!(
                    "{count} files, frame_{:04}.png to frame_{:04}.png",
                    state.export_cfg.start, state.export_cfg.end
                ),
                _ => format!(
                    "{count} frames - {:.1}s at {:.0} fps",
                    count as f32 / state.project.fps.max(1.0),
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
    let fade = |c: Color32| {
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * a) as u8)
    };

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
