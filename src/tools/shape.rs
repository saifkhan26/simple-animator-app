//! Outline shape rasteriser — line, rectangle, ellipse.
//!
//! Shapes build a spine polyline (constant radius, full flow) and reuse the
//! ribbon capsule rasterizer ([`crate::tools::ribbon`]), but without the
//! Catmull-Rom smoothing the freehand pipeline applies — so corners stay
//! crisp and lines stay straight. Corners get round joins from the capsule
//! caps, and `max()` coverage combining keeps them at uniform opacity.
//! Outline only; use the Fill bucket to fill.
//!
//! With perspective snap on, rectangles and ellipses lie on the active grid's
//! plane instead: the drag spans a box in the grid's own `(u, v)` space, and
//! that box — or the ellipse inside it — is mapped onto the picture, so a
//! rectangle's sides run to the vanishing points and a circle foreshortens.

use crate::doc::canvas::Canvas;
use crate::tools::perspective::Homography;
use crate::tools::ribbon::{union_rect, SpineNode, StrokeWorkspace};
use crate::tools::{BrushSettings, ShapeKind};

/// A perspective grid's plane in the target cell's pixel space, taken when a
/// shape drag starts.
#[derive(Clone, Copy, Debug)]
pub struct GridPlane {
    pub h: Homography,
    pub rows: u32,
    pub cols: u32,
}

/// Chord length that keeps a curve of tightest curvature radius `r_curv`
/// within 0.2 px of true: a chord c on a curve of radius R deviates by
/// ~c^2/(8R). Never longer than the brush is thick, so joins stay hidden.
fn chord(r_curv: f32, radius: f32) -> f32 {
    (8.0 * r_curv * 0.2).sqrt().clamp(1.0, radius.max(1.0))
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// `kind`'s outline between drag anchor `a` and current point `b`, as a
/// polyline — closed shapes repeat their first point at the end. `radius` is
/// the stroke's, which bounds how finely curves are cut.
pub fn outline(kind: ShapeKind, a: (f32, f32), b: (f32, f32), radius: f32) -> Vec<(f32, f32)> {
    match kind {
        ShapeKind::Line => vec![a, b],
        ShapeKind::Rect => {
            let (x0, y0) = (a.0.min(b.0), a.1.min(b.1));
            let (x1, y1) = (a.0.max(b.0), a.1.max(b.1));
            vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
        }
        ShapeKind::Ellipse => {
            let cx = (a.0 + b.0) * 0.5;
            let cy = (a.1 + b.1) * 0.5;
            let rx = (b.0 - a.0).abs() * 0.5;
            let ry = (b.1 - a.1).abs() * 0.5;
            if rx < 0.5 && ry < 0.5 {
                vec![(cx, cy)]
            } else {
                // Use the ellipse's tightest curvature radius (min^2/max) so
                // the flat ends of thin ellipses stay smooth.
                let (mx, mn) = (rx.max(ry), rx.min(ry).max(0.5));
                let chord = chord(mn * mn / mx, radius);
                // Ramanujan circumference approximation.
                let circ = std::f32::consts::PI
                    * (3.0 * (rx + ry) - ((3.0 * rx + ry) * (rx + 3.0 * ry)).sqrt());
                let n = ((circ / chord).ceil() as usize).clamp(32, 4096);
                (0..=n)
                    .map(|i| {
                        let t = i as f32 / n as f32 * std::f32::consts::TAU;
                        (cx + rx * t.cos(), cy + ry * t.sin())
                    })
                    .collect()
            }
        }
    }
}

/// `kind`'s outline lying on `plane`, or `None` when it can't: a line (lines
/// snap to a grid direction instead), or a box that reaches past the plane's
/// horizon, where it would wrap round behind the viewer.
///
/// `square` makes the box as many grid cells deep as it is wide — a square
/// on the grid, and the ellipse inside it a circle.
pub fn outline_on_plane(
    kind: ShapeKind,
    a: (f32, f32),
    b: (f32, f32),
    plane: &GridPlane,
    square: bool,
    radius: f32,
) -> Option<Vec<(f32, f32)>> {
    if kind == ShapeKind::Line {
        return None;
    }
    let [u0, v0] = plane.h.unmap([a.0, a.1])?;
    let [mut u1, mut v1] = plane.h.unmap([b.0, b.1])?;
    if square {
        let (cols, rows) = (plane.cols.max(1) as f32, plane.rows.max(1) as f32);
        let (du, dv) = (u1 - u0, v1 - v0);
        let cells = (du.abs() * cols).max(dv.abs() * rows);
        let sign = |d: f32| if d < 0.0 { -1.0 } else { 1.0 };
        u1 = u0 + sign(du) * cells / cols;
        v1 = v0 + sign(dv) * cells / rows;
    }
    // `w` is affine in (u, v), so with every corner in front the whole box —
    // and the ellipse inside it — is too.
    let mut box_pts = [(0.0, 0.0); 4];
    for (k, (u, v)) in [(u0, v0), (u1, v0), (u1, v1), (u0, v1)].into_iter().enumerate() {
        let p = plane.h.map_front(u, v)?;
        box_pts[k] = (p[0], p[1]);
    }
    if kind == ShapeKind::Rect {
        let mut out = box_pts.to_vec();
        out.push(box_pts[0]);
        return Some(out);
    }

    let (cu, cv) = ((u0 + u1) * 0.5, (v0 + v1) * 0.5);
    let (ru, rv) = ((u1 - u0) * 0.5, (v1 - v0) * 0.5);
    let at = |t: f32| {
        let p = plane.h.map(cu + ru * t.cos(), cv + rv * t.sin());
        (p[0], p[1])
    };
    let edges = [0, 1, 2, 3].map(|k| dist(box_pts[k], box_pts[(k + 1) % 4]));
    let (mn, mx) = edges
        .iter()
        .fold((f32::INFINITY, 0.0f32), |(lo, hi), &e| (lo.min(e), hi.max(e)));
    if mx < 1.0 {
        return Some(vec![at(0.0)]);
    }
    // The flat rule, on the box as the picture shows it. Foreshortening
    // squeezes the far side's samples together, so size the count off the
    // longest step of a coarse pass: that keeps the near side smooth too.
    let (mn, mx) = (mn.max(1.0) * 0.5, mx * 0.5);
    let chord = chord(mn * mn / mx, radius);
    const COARSE: usize = 64;
    let step = |i: usize, n: usize| i as f32 / n as f32 * std::f32::consts::TAU;
    let longest = (0..COARSE)
        .map(|i| dist(at(step(i, COARSE)), at(step(i + 1, COARSE))))
        .fold(0.0f32, f32::max);
    let n = ((COARSE as f32 * longest / chord).ceil() as usize).clamp(COARSE, 4096);
    Some((0..=n).map(|i| at(step(i, n))).collect())
}

/// Rasterise the `spine` polyline (from [`outline`] or [`outline_on_plane`])
/// into the canvas, compositing over the pre-drag snapshot `pre`.
/// `ws.begin()` must have been called for this canvas.
pub fn rasterize(
    canvas: &mut Canvas,
    ws: &mut StrokeWorkspace,
    pre: &[u8],
    spine: &[(f32, f32)],
    brush: &BrushSettings,
) {
    let node = |p: &(f32, f32)| SpineNode {
        x: p.0,
        y: p.1,
        radius: brush.radius.max(0.1),
        flow: 1.0,
    };

    let mut rect = None;
    if let [p] = spine {
        if let Some(r) = ws.raster_dot(node(p)) {
            rect = Some(union_rect(rect, r));
        }
    } else {
        for seg in spine.windows(2) {
            if let Some(r) = ws.raster_capsule(node(&seg[0]), node(&seg[1])) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-2 && (a.1 - b.1).abs() < 1e-2
    }

    fn plane(c: [[f32; 2]; 4], rows: u32, cols: u32) -> GridPlane {
        GridPlane {
            h: Homography::square_to_quad(&c).unwrap(),
            rows,
            cols,
        }
    }

    /// One-point floor: the sides meet at (50, -25).
    const TRAP: [[f32; 2]; 4] = [[40.0, 0.0], [60.0, 0.0], [100.0, 100.0], [0.0, 100.0]];

    #[test]
    fn a_square_on_grid_draws_the_flat_rect() {
        let g = plane([[10.0, 20.0], [110.0, 20.0], [110.0, 70.0], [10.0, 70.0]], 4, 4);
        let (a, b) = ((30.0, 30.0), (80.0, 60.0));
        let flat = outline(ShapeKind::Rect, a, b, 2.0);
        let on = outline_on_plane(ShapeKind::Rect, a, b, &g, false, 2.0).unwrap();
        assert_eq!(flat.len(), on.len());
        for (p, q) in flat.iter().zip(&on) {
            assert!(close(*p, *q), "{p:?} vs {q:?}");
        }
    }

    #[test]
    fn a_rect_on_a_floor_runs_its_sides_to_the_vanishing_point() {
        let g = plane(TRAP, 4, 4);
        let r = outline_on_plane(ShapeKind::Rect, (30.0, 90.0), (70.0, 40.0), &g, false, 2.0)
            .unwrap();
        assert_eq!(r.len(), 5);
        assert!(close(r[0], (30.0, 90.0)), "starts at the anchor");
        assert!(close(r[2], (70.0, 40.0)), "opposite corner under the cursor");
        // Its other two corners aren't the flat rect's...
        assert!(!close(r[1], (70.0, 90.0)) && !close(r[3], (30.0, 40.0)));
        // ...because the sides lean in toward (50, -25), and the near and far
        // edges stay horizontal like the grid's rows.
        let vp = (50.0, -25.0);
        let toward_vp = |p: (f32, f32), q: (f32, f32)| {
            let (d, e) = ((q.0 - p.0, q.1 - p.1), (vp.0 - p.0, vp.1 - p.1));
            (d.0 * e.1 - d.1 * e.0).abs() / (dist(p, q) * dist(p, vp)) < 1e-3
        };
        let (a, b, c, d) = (r[0], r[1], r[2], r[3]);
        assert!(toward_vp(a, d), "{a:?} {d:?}");
        assert!(toward_vp(b, c), "{b:?} {c:?}");
        assert!((a.1 - b.1).abs() < 1e-3 && (c.1 - d.1).abs() < 1e-3);
    }

    #[test]
    fn square_means_as_many_cells_deep_as_wide() {
        // 4 columns, 8 rows: a square in cells is twice as deep in v as wide in u.
        let g = plane(TRAP, 8, 4);
        let r = outline_on_plane(ShapeKind::Rect, (30.0, 90.0), (70.0, 80.0), &g, true, 2.0)
            .unwrap();
        let [u0, v0] = g.h.unmap([r[0].0, r[0].1]).unwrap();
        let [u1, v1] = g.h.unmap([r[2].0, r[2].1]).unwrap();
        assert!(((u1 - u0).abs() * 4.0 - (v1 - v0).abs() * 8.0).abs() < 1e-3);
        // The wider extent wins, and each keeps its direction.
        assert!(u1 > u0 && v1 < v0);
    }

    #[test]
    fn a_circle_on_the_floor_has_its_far_half_foreshortened() {
        let g = plane(TRAP, 4, 4);
        let (a, b) = ((20.0, 95.0), (80.0, 40.0));
        let e = outline_on_plane(ShapeKind::Ellipse, a, b, &g, true, 2.0).unwrap();
        assert!(close(e[0], *e.last().unwrap()), "closed");
        // The circle's centre on the floor sits below the middle of its
        // picture: the half nearer the horizon is squeezed.
        let [u0, v0] = g.h.unmap([a.0, a.1]).unwrap();
        let [u1, _] = g.h.unmap([b.0, b.1]).unwrap();
        let v1 = v0 - (u1 - u0); // square: 4 rows, 4 columns, drawn upward
        let centre = g.h.map((u0 + u1) * 0.5, (v0 + v1) * 0.5);
        let (top, bottom) = e.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(t, b), p| {
            (t.min(p.1), b.max(p.1))
        });
        assert!(centre[1] - top < bottom - centre[1], "{top} {} {bottom}", centre[1]);
        // Fine enough that no step is much longer than the brush is thick,
        // near side included.
        let longest = e.windows(2).map(|w| dist(w[0], w[1])).fold(0.0f32, f32::max);
        assert!(longest <= 2.2, "{longest}");
    }

    #[test]
    fn a_box_past_the_horizon_is_not_on_the_plane() {
        let g = plane(TRAP, 4, 4);
        // Above the horizon at y = -25.
        assert!(outline_on_plane(ShapeKind::Rect, (50.0, 50.0), (60.0, -40.0), &g, false, 2.0)
            .is_none());
        // Lines never lie on the plane; they snap to a direction instead.
        assert!(outline_on_plane(ShapeKind::Line, (50.0, 50.0), (60.0, 60.0), &g, false, 2.0)
            .is_none());
    }
}
