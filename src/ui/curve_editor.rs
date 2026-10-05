//! Krita's curve widget (`KisCurveWidget`): a sensor's response, its input
//! along the bottom and the option's value up the side. Drag a point to move
//! it, click or drag on empty space to add one, right-click a point to
//! remove it (two always stay).

use egui::{Pos2, Sense, Stroke, Vec2};

use crate::tools::krita::curve::CubicCurve;
use crate::ui::theme;

/// How near, in screen pixels, a press has to be to grab a point.
const GRAB: f32 = 8.0;
/// The gap kept between neighbouring points' x, so they stay in order.
const GAP: f64 = 1e-3;

/// Edit `curve` (Krita's `"x,y;…"` text) in place. Returns whether it
/// changed.
pub fn curve_editor(ui: &mut egui::Ui, id: egui::Id, curve: &mut String) -> bool {
    let Some(parsed) = CubicCurve::parse(curve) else { return false };
    let mut pts = parsed.points().to_vec();
    let w = ui.available_width().clamp(120.0, 260.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, (w * 0.6).round()), Sense::click_and_drag());
    let area = rect.shrink(4.0);
    let to_screen = |(x, y): (f64, f64)| Pos2::new(area.left() + x as f32 * area.width(), area.bottom() - y as f32 * area.height());
    let to_curve = |p: Pos2| {
        (
            (((p.x - area.left()) / area.width()) as f64).clamp(0.0, 1.0),
            (((area.bottom() - p.y) / area.height()) as f64).clamp(0.0, 1.0),
        )
    };
    let near = |pts: &[(f64, f64)], at: Pos2| {
        pts.iter()
            .enumerate()
            .map(|(i, &p)| (i, to_screen(p).distance(at)))
            .filter(|&(_, d)| d < GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    let drag_id = id.with("dragging");
    let mut dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id)).flatten();
    let mut changed = false;

    // A new point goes where it keeps the x order.
    let add = |pts: &mut Vec<(f64, f64)>, p: (f64, f64)| -> usize {
        let i = pts.iter().position(|q| q.0 > p.0).unwrap_or(pts.len());
        pts.insert(i, p);
        i
    };
    if resp.drag_started() {
        // Where the press was: by the time a drag is decided the pointer
        // has already moved, maybe off the point.
        if let Some(at) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()) {
            dragging = near(&pts, at).or_else(|| {
                changed = true;
                Some(add(&mut pts, to_curve(at)))
            });
        }
    } else if resp.clicked() {
        if let Some(at) = resp.interact_pointer_pos() {
            if near(&pts, at).is_none() {
                add(&mut pts, to_curve(at));
                changed = true;
            }
        }
    }
    if resp.secondary_clicked() && pts.len() > 2 {
        if let Some(i) = resp.interact_pointer_pos().and_then(|at| near(&pts, at)) {
            pts.remove(i);
            changed = true;
        }
    }
    if let (Some(i), true, Some(at)) = (dragging, resp.dragged(), resp.interact_pointer_pos()) {
        if i < pts.len() {
            let (mut x, y) = to_curve(at);
            let lo = if i == 0 { 0.0 } else { pts[i - 1].0 + GAP };
            let hi = if i + 1 == pts.len() { 1.0 } else { pts[i + 1].0 - GAP };
            if lo <= hi {
                x = x.clamp(lo, hi);
            }
            if pts[i] != (x, y) {
                pts[i] = (x, y);
                changed = true;
            }
        }
    }
    if resp.drag_stopped() {
        dragging = None;
    }
    ui.data_mut(|d| d.insert_temp(drag_id, dragging));

    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, theme::BG_BASE);
    p.rect_stroke(rect, 3.0, Stroke::new(1.0, theme::STROKE_THIN));
    for q in [0.25, 0.5, 0.75] {
        let a = to_screen((q, 0.0));
        let b = to_screen((q, 1.0));
        p.line_segment([a, b], Stroke::new(1.0, theme::STROKE_THIN));
        let a = to_screen((0.0, q));
        let b = to_screen((1.0, q));
        p.line_segment([a, b], Stroke::new(1.0, theme::STROKE_THIN));
    }
    p.line_segment([to_screen((0.0, 0.0)), to_screen((1.0, 1.0))], Stroke::new(1.0, theme::BG_HOVER));
    let c = CubicCurve::from_points(pts.clone());
    let n = (area.width() as usize).max(2);
    let line: Vec<Pos2> = c
        .float_transfer(n)
        .iter()
        .enumerate()
        .map(|(i, &y)| to_screen((i as f64 / (n - 1) as f64, y)))
        .collect();
    p.add(egui::Shape::line(line, Stroke::new(1.5, theme::ACCENT)));
    let hover = resp.hover_pos().and_then(|at| near(&pts, at));
    for (i, &q) in pts.iter().enumerate() {
        let hot = Some(i) == dragging || Some(i) == hover;
        let r = if hot { 4.5 } else { 3.5 };
        p.circle(to_screen(q), r, theme::BG_SURFACE, Stroke::new(1.5, if hot { theme::ACCENT_HOVER } else { theme::TEXT }));
    }
    resp.on_hover_text("Drag a point to move it. Click to add one; right-click one to remove it.");
    if changed {
        *curve = c.to_krita_string();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, Event, PointerButton};

    /// One frame of the widget alone at the screen's top left: 260 wide,
    /// so its curve area runs from (4, 4) to (256, 152).
    fn frame(ctx: &egui::Context, curve: &mut String, events: Vec<Event>) -> bool {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0))),
            events,
            ..Default::default()
        };
        let mut changed = false;
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
                changed |= curve_editor(ui, egui::Id::new("t"), curve);
            });
        });
        changed
    }

    fn at(x: f64, y: f64) -> egui::Pos2 {
        pos2(4.0 + x as f32 * 252.0, 152.0 - y as f32 * 148.0)
    }

    fn press(p: egui::Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton {
            pos: p,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn points(curve: &str) -> Vec<(f64, f64)> {
        CubicCurve::parse(curve).unwrap().points().to_vec()
    }

    #[test]
    fn click_adds_drag_moves_right_click_removes() {
        let ctx = egui::Context::default();
        let mut curve = "0,0;1,1;".to_owned();
        frame(&ctx, &mut curve, vec![]);
        // Click on empty space: a point there.
        let p = at(0.5, 0.8);
        frame(&ctx, &mut curve, vec![Event::PointerMoved(p), press(p, PointerButton::Primary, true)]);
        assert!(frame(&ctx, &mut curve, vec![press(p, PointerButton::Primary, false)]), "{curve}");
        let pts = points(&curve);
        assert_eq!(pts.len(), 3, "{curve}");
        assert!((pts[1].0 - 0.5).abs() < 0.01 && (pts[1].1 - 0.8).abs() < 0.01, "{curve}");

        // Drag it down; it stays between its neighbours however far right.
        frame(&ctx, &mut curve, vec![Event::PointerMoved(p), press(p, PointerButton::Primary, true)]);
        for i in 1..=10 {
            let q = egui::Pos2::new(p.x + i as f32 * 40.0, p.y + i as f32 * 9.0);
            frame(&ctx, &mut curve, vec![Event::PointerMoved(q)]);
        }
        let end = egui::Pos2::new(p.x + 400.0, p.y + 90.0);
        frame(&ctx, &mut curve, vec![press(end, PointerButton::Primary, false)]);
        let pts = points(&curve);
        assert_eq!(pts.len(), 3, "{curve}");
        assert!(pts[1].0 < 1.0 && pts[1].0 > 0.99, "held left of the end point: {curve}");
        assert!(pts[1].1 < 0.3, "{curve}");

        // Right-click removes it; the ends stay whatever is clicked.
        let r = at(pts[1].0, pts[1].1);
        frame(&ctx, &mut curve, vec![Event::PointerMoved(r), press(r, PointerButton::Secondary, true)]);
        frame(&ctx, &mut curve, vec![press(r, PointerButton::Secondary, false)]);
        assert_eq!(points(&curve).len(), 2, "{curve}");
        let e = at(0.0, 0.0);
        frame(&ctx, &mut curve, vec![Event::PointerMoved(e), press(e, PointerButton::Secondary, true)]);
        frame(&ctx, &mut curve, vec![press(e, PointerButton::Secondary, false)]);
        assert_eq!(points(&curve).len(), 2, "two always stay");
    }
}
