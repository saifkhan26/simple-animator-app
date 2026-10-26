//! Shift held partway through a drag: what it holds the drag to.
//!
//! A drag is held to the horizontal or the vertical through where it started,
//! a shape's line to a 15° step, and a box to a square. Each function works in
//! whatever space it is handed. `basis` is the angle, in that space, of the
//! horizontal the lock keeps to: 0 for the space's own axes, something else
//! when the lock follows the screen over a turned view or layer.

use std::f32::consts::{FRAC_PI_2, PI};

type P = (f32, f32);

/// The step a Shift line turns in.
pub const LINE_STEP: f32 = PI / 12.0;

/// `to` held to the nearest direction from `from` that lies a whole number of
/// `step`s off `basis`. The end is projected onto that line rather than swung
/// onto it, so it tracks the pointer along the line.
pub fn snap_dir(from: P, to: P, basis: f32, step: f32) -> P {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    if dx == 0.0 && dy == 0.0 {
        return to;
    }
    let a = basis + ((dy.atan2(dx) - basis) / step).round() * step;
    let (s, c) = a.sin_cos();
    let along = dx * c + dy * s;
    (from.0 + along * c, from.1 + along * s)
}

/// `to` held to the horizontal or the vertical through `from`, whichever the
/// drag is nearer.
pub fn lock_hv(from: P, to: P, basis: f32) -> P {
    snap_dir(from, to, basis, FRAC_PI_2)
}

/// The direction, as an angle, of the horizontal or vertical through `from`
/// that `to` is nearer: a stroke picks one and keeps to it.
pub fn hv_angle(from: P, to: P, basis: f32) -> f32 {
    let a = (to.1 - from.1).atan2(to.0 - from.0);
    basis + ((a - basis) / FRAC_PI_2).round() * FRAC_PI_2
}

/// `p` projected onto the line through `from` at angle `a`.
pub fn onto(from: P, a: f32, p: P) -> P {
    let (s, c) = a.sin_cos();
    let along = (p.0 - from.0) * c + (p.1 - from.1) * s;
    (from.0 + along * c, from.1 + along * s)
}

/// `to` moved so the box from `from` is square: the longer side kept, each
/// side still running the way it was dragged.
pub fn square(from: P, to: P) -> P {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let side = dx.abs().max(dy.abs());
    let sign = |d: f32| if d < 0.0 { -1.0 } else { 1.0 };
    (from.0 + sign(dx) * side, from.1 + sign(dy) * side)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: P, b: P) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn a_drag_keeps_its_longer_axis() {
        assert!(near(lock_hv((10.0, 10.0), (40.0, 17.0), 0.0), (40.0, 10.0)));
        assert!(near(lock_hv((10.0, 10.0), (4.0, -30.0), 0.0), (10.0, -30.0)));
    }

    #[test]
    fn a_turned_basis_locks_along_its_own_axes() {
        // Screen axes over a view turned 30°: a drag along the turned
        // horizontal stays on it, and a sideways wobble is dropped.
        let a = 30f32.to_radians();
        let along = (100.0 * a.cos(), 100.0 * a.sin());
        let wobble = (along.0 - 3.0 * a.sin(), along.1 + 3.0 * a.cos());
        assert!(near(lock_hv((0.0, 0.0), wobble, a), along));
    }

    #[test]
    fn a_line_turns_in_fifteen_degree_steps() {
        // 50° from the horizontal is nearer 45° than 60°.
        let a = 50f32.to_radians();
        let to = snap_dir((0.0, 0.0), (100.0 * a.cos(), 100.0 * a.sin()), 0.0, LINE_STEP);
        assert!((to.1.atan2(to.0).to_degrees() - 45.0).abs() < 1e-3);
        // Nearly flat goes flat, and a still drag stays put.
        assert!(near(snap_dir((0.0, 0.0), (80.0, 3.0), 0.0, LINE_STEP), (80.0, 0.0)));
        assert!(near(snap_dir((5.0, 5.0), (5.0, 5.0), 0.0, LINE_STEP), (5.0, 5.0)));
    }

    #[test]
    fn a_box_squares_up_the_way_it_was_dragged() {
        assert_eq!(square((0.0, 0.0), (30.0, -10.0)), (30.0, -30.0));
        assert_eq!(square((0.0, 0.0), (-5.0, 20.0)), (-20.0, 20.0));
    }

    #[test]
    fn a_stroke_picks_the_axis_it_heads_along() {
        assert!(hv_angle((0.0, 0.0), (3.0, 40.0), 0.0).sin() > 0.99);
        assert!(near(onto((0.0, 0.0), 0.0, (25.0, 7.0)), (25.0, 0.0)));
    }
}
