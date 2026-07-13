//! Outline shape rasteriser — line, rectangle, ellipse.
//!
//! Shapes build a spine polyline (constant radius, full flow) and reuse the
//! ribbon capsule rasterizer ([`crate::tools::ribbon`]), but without the
//! Catmull-Rom smoothing the freehand pipeline applies — so corners stay
//! crisp and lines stay straight. Corners get round joins from the capsule
//! caps, and `max()` coverage combining keeps them at uniform opacity.
//! Outline only; use the Fill bucket to fill.

use crate::doc::canvas::Canvas;
use crate::tools::ribbon::{union_rect, SpineNode, StrokeWorkspace};
use crate::tools::{BrushSettings, ShapeKind};

/// Rasterise `kind`'s outline between drag anchor `a` and current point `b`
/// into the canvas, compositing over the pre-drag snapshot `pre`.
/// `ws.begin()` must have been called for this canvas.
pub fn rasterize(
    canvas: &mut Canvas,
    ws: &mut StrokeWorkspace,
    pre: &[u8],
    kind: ShapeKind,
    a: (f32, f32),
    b: (f32, f32),
    brush: &BrushSettings,
) {
    let node = |p: (f32, f32)| SpineNode {
        x: p.0,
        y: p.1,
        radius: brush.radius.max(0.1),
        flow: 1.0,
    };

    let spine: Vec<SpineNode> = match kind {
        ShapeKind::Line => vec![node(a), node(b)],
        ShapeKind::Rect => {
            let (x0, y0) = (a.0.min(b.0), a.1.min(b.1));
            let (x1, y1) = (a.0.max(b.0), a.1.max(b.1));
            vec![
                node((x0, y0)),
                node((x1, y0)),
                node((x1, y1)),
                node((x0, y1)),
                node((x0, y0)),
            ]
        }
        ShapeKind::Ellipse => {
            let cx = (a.0 + b.0) * 0.5;
            let cy = (a.1 + b.1) * 0.5;
            let rx = (b.0 - a.0).abs() * 0.5;
            let ry = (b.1 - a.1).abs() * 0.5;
            if rx < 0.5 && ry < 0.5 {
                vec![node((cx, cy))]
            } else {
                // Chord length bounded by sagitta error: a chord c on a curve
                // of radius R deviates by ~c^2/(8R). Use the ellipse's
                // tightest curvature radius (min^2/max) so the flat ends of
                // thin ellipses stay smooth, with a 0.2 px error budget.
                let (mx, mn) = (rx.max(ry), rx.min(ry).max(0.5));
                let r_curv = mn * mn / mx;
                let chord = (8.0 * r_curv * 0.2).sqrt().clamp(1.0, brush.radius.max(1.0));
                // Ramanujan circumference approximation.
                let circ = std::f32::consts::PI
                    * (3.0 * (rx + ry) - ((3.0 * rx + ry) * (rx + 3.0 * ry)).sqrt());
                let n = ((circ / chord).ceil() as usize).clamp(32, 4096);
                (0..=n)
                    .map(|i| {
                        let t = i as f32 / n as f32 * std::f32::consts::TAU;
                        node((cx + rx * t.cos(), cy + ry * t.sin()))
                    })
                    .collect()
            }
        }
    };

    let mut rect = None;
    if spine.len() == 1 {
        if let Some(r) = ws.raster_dot(spine[0]) {
            rect = Some(union_rect(rect, r));
        }
    } else {
        for seg in spine.windows(2) {
            if let Some(r) = ws.raster_capsule(seg[0], seg[1]) {
                rect = Some(union_rect(rect, r));
            }
        }
    }

    if let Some(rect) = rect {
        ws.composite_paint(canvas, pre, rect, brush.color, brush.opacity);
        canvas.mark_dirty(
            rect.min_x,
            rect.min_y,
            rect.max_x - rect.min_x,
            rect.max_y - rect.min_y,
        );
    }
}
