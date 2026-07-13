//! Custom egui widgets not provided by the base library.

use egui::{Color32, Response, Sense, Stroke, Ui, Vec2};

use crate::ui::theme;

/// Horizontal range slider with two draggable knobs selecting an inclusive
/// integer range `[start, end]` within `[min, max]`. Returns the response,
/// marked changed when either knob moves. `start <= end` is enforced.
pub fn range_slider(
    ui: &mut Ui,
    start: &mut usize,
    end: &mut usize,
    min: usize,
    max: usize,
) -> Response {
    let height = 24.0;
    let knob_r = 7.0;
    let desired = Vec2::new(ui.available_width().max(160.0), height);
    let (rect, mut resp) = ui.allocate_exact_size(desired, Sense::drag());

    let span = max.saturating_sub(min).max(1) as f32;
    let pad = knob_r + 2.0;
    let x0 = rect.left() + pad;
    let x1 = rect.right() - pad;
    let track_w = (x1 - x0).max(1.0);
    let cy = rect.center().y;

    let val_to_x = |v: usize| -> f32 { x0 + (v.saturating_sub(min) as f32 / span) * track_w };
    let x_to_val = |x: f32| -> usize {
        let t = ((x - x0) / track_w).clamp(0.0, 1.0);
        min + (t * span).round() as usize
    };

    let sx = val_to_x((*start).clamp(min, max));
    let ex = val_to_x((*end).clamp(min, max));

    // Move whichever knob is nearer the pointer while dragging.
    if let Some(pos) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.drag_started() {
            let v = x_to_val(pos.x);
            let d_start = (pos.x - sx).abs();
            let d_end = (pos.x - ex).abs();
            if d_start <= d_end {
                *start = v.min(*end);
            } else {
                *end = v.max(*start);
            }
            resp.mark_changed();
        }
    }

    let painter = ui.painter();
    // Full track.
    painter.line_segment(
        [egui::pos2(x0, cy), egui::pos2(x1, cy)],
        Stroke::new(3.0, theme::STROKE_THIN),
    );
    // Selected segment.
    painter.line_segment(
        [egui::pos2(sx, cy), egui::pos2(ex, cy)],
        Stroke::new(3.0, theme::ACCENT),
    );
    // Knobs on top.
    for x in [sx, ex] {
        painter.circle_filled(egui::pos2(x, cy), knob_r, theme::ACCENT);
        painter.circle_stroke(egui::pos2(x, cy), knob_r, Stroke::new(1.5, Color32::WHITE));
    }

    resp
}
