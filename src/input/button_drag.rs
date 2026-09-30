//! What a drag with the middle or the right mouse button does on the canvas.
//! Pen barrel buttons arrive as these same buttons, so this is also what the
//! pen's side switches do.
//!
//! The left button always draws with the active tool. The other two are
//! workspace preferences, picked from [`ButtonDrag::ALL`] in Settings.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonDrag {
    /// Nothing: the button is ignored on the canvas.
    None,
    Pan,
    Zoom,
    Rotate,
    /// Sideways moves the playhead a frame at a time; with Shift held, a key
    /// drawing at a time. The playhead stays where it is let go.
    Scrub,
    /// Sideways flips through the active layer's drawings, and lets go back
    /// on the frame it started from — the animator's flip, to check motion
    /// without losing the drawing being worked on.
    Flip,
    /// Sideways and up and down change the active tool's two main values:
    /// size and opacity for a brush, gap and expand for the bucket.
    ToolAdjust,
    /// A ring of the pinned swatches around the pen; let go over one to pick
    /// it, or in the middle to keep the colour.
    SwatchPie,
}

impl ButtonDrag {
    /// In the order the Settings picker lists them.
    pub const ALL: [ButtonDrag; 8] = [
        ButtonDrag::Pan,
        ButtonDrag::Zoom,
        ButtonDrag::Rotate,
        ButtonDrag::Scrub,
        ButtonDrag::Flip,
        ButtonDrag::ToolAdjust,
        ButtonDrag::SwatchPie,
        ButtonDrag::None,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ButtonDrag::None => "Nothing",
            ButtonDrag::Pan => "Pan canvas",
            ButtonDrag::Zoom => "Zoom canvas",
            ButtonDrag::Rotate => "Rotate canvas",
            ButtonDrag::Scrub => "Scrub timeline",
            ButtonDrag::Flip => "Flip drawings",
            ButtonDrag::ToolAdjust => "Adjust tool",
            ButtonDrag::SwatchPie => "Swatch pie",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            ButtonDrag::None => "The button does nothing on the canvas.",
            ButtonDrag::Pan => "Drag to move the view.",
            ButtonDrag::Zoom => "Drag up and down to zoom the view.",
            ButtonDrag::Rotate => "Drag sideways to rotate the view.",
            ButtonDrag::Scrub => {
                "Drag sideways to move through the frames. Hold Shift to step \
                 from one key drawing to the next."
            }
            ButtonDrag::Flip => {
                "Drag sideways to flip through the active layer's drawings. \
                 Letting go returns to the frame you started on."
            }
            ButtonDrag::ToolAdjust => {
                "Brushes: sideways sets the size, up and down the opacity.\n\
                 Fill: sideways sets the gap, up and down the expand."
            }
            ButtonDrag::SwatchPie => {
                "Hold to open the pinned swatches around the pen, and let go \
                 over one to pick it. Let go in the middle to keep the colour."
            }
        }
    }
}

/// Screen points a value-adjusting drag moves before it starts counting, so
/// the small wander of a plain press never changes a value.
const DEAD_ZONE: f32 = 8.0;
/// Screen points of drag per step of a stepped value.
const STEP_PX: f32 = 12.0;

/// A drag of `d` screen points along one axis with the dead zone taken off,
/// signed like `d`.
pub fn past_dead_zone(d: f32) -> f32 {
    let past = d.abs() - DEAD_ZONE;
    if past.is_nan() || past <= 0.0 {
        0.0
    } else {
        past * d.signum()
    }
}

/// Whole steps in a drag of `d` screen points along one axis: none inside the
/// dead zone, then one per [`STEP_PX`], signed like `d`.
pub fn drag_steps(d: f32) -> i32 {
    (past_dead_zone(d) / STEP_PX).trunc() as i32
}

/// Screen points of sideways drag per frame when scrubbing.
pub const SCRUB_PX_PER_FRAME: f32 = 10.0;
/// Screen points of sideways drag per key drawing when flipping, or scrubbing
/// with Shift.
pub const FLIP_PX_PER_KEY: f32 = 24.0;

/// Swatch pie geometry, in screen points from the press. Inside `PIE_HOLE` is
/// the cancel spot; the first [`PIE_INNER`] swatches ring it out to
/// `PIE_RING`, the rest ring those out to `PIE_OUTER`.
pub const PIE_HOLE: f32 = 26.0;
pub const PIE_RING: f32 = 78.0;
pub const PIE_OUTER: f32 = 128.0;
/// Swatches on the inner ring.
pub const PIE_INNER: usize = 8;

/// How the `n` swatches split between the inner and the outer ring.
pub fn pie_rings(n: usize) -> (usize, usize) {
    let inner = n.min(PIE_INNER);
    (inner, n - inner)
}

/// Heading of `off` in radians, 0 straight up and growing clockwise, in
/// `0..TAU`.
pub fn pie_heading(off: [f32; 2]) -> f32 {
    off[0].atan2(-off[1]).rem_euclid(std::f32::consts::TAU)
}

/// Which of `n` wedges a heading falls in, wedge 0 centred straight up.
fn wedge(heading: f32, n: usize) -> usize {
    let step = std::f32::consts::TAU / n as f32;
    ((heading + step * 0.5) / step).floor() as usize % n
}

/// The swatch under a pen `off` from the pie's centre, out of `n`, or `None`
/// over the cancel spot. Past the rings still counts, by heading alone, so a
/// quick flick picks as well as a careful aim.
pub fn pie_slot(n: usize, off: [f32; 2]) -> Option<usize> {
    let d = off[0].hypot(off[1]);
    if n == 0 || d < PIE_HOLE {
        return None;
    }
    let (inner, outer) = pie_rings(n);
    let heading = pie_heading(off);
    if outer == 0 || d < PIE_RING {
        Some(wedge(heading, inner))
    } else {
        Some(inner + wedge(heading, outer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hole_cancels_and_an_empty_pie_picks_nothing() {
        assert_eq!(pie_slot(8, [0.0, 0.0]), None);
        assert_eq!(pie_slot(8, [10.0, -10.0]), None);
        assert_eq!(pie_slot(0, [0.0, -60.0]), None);
    }

    #[test]
    fn inner_wedges_run_clockwise_from_the_top() {
        let up = [0.0, -50.0];
        let right = [50.0, 0.0];
        let down = [0.0, 50.0];
        let left = [-50.0, 0.0];
        assert_eq!(pie_slot(8, up), Some(0));
        assert_eq!(pie_slot(8, right), Some(2));
        assert_eq!(pie_slot(8, down), Some(4));
        assert_eq!(pie_slot(8, left), Some(6));
        // Wedge 0 straddles the top: a little left of up is still 0.
        assert_eq!(pie_slot(8, [-5.0, -50.0]), Some(0));
        // Fewer swatches, wider wedges.
        assert_eq!(pie_slot(3, right), Some(1));
    }

    #[test]
    fn the_outer_ring_holds_the_rest_and_a_flick_past_it_still_counts() {
        // 20 swatches: 8 inside, 12 outside.
        assert_eq!(pie_rings(20), (8, 12));
        assert_eq!(pie_slot(20, [0.0, -100.0]), Some(8));
        assert_eq!(pie_slot(20, [100.0, 0.0]), Some(8 + 3));
        assert_eq!(pie_slot(20, [400.0, 0.0]), Some(8 + 3));
        // With no outer ring, far out still means the inner ring.
        assert_eq!(pie_slot(5, [0.0, -300.0]), Some(0));
    }

    #[test]
    fn drag_steps_wait_out_the_dead_zone() {
        assert_eq!(drag_steps(0.0), 0);
        assert_eq!(drag_steps(7.9), 0);
        assert_eq!(drag_steps(19.9), 0);
        assert_eq!(drag_steps(20.0), 1);
        assert_eq!(drag_steps(-20.0), -1);
        assert_eq!(drag_steps(40.0), 2);
        assert_eq!(drag_steps(f32::NAN), 0);
        assert_eq!(past_dead_zone(-20.0), -12.0);
        assert_eq!(past_dead_zone(5.0), 0.0);
    }
}
