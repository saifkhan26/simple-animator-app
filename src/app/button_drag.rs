//! Middle- and right-button drags on the canvas: scrub, flip, tool adjust and
//! the swatch pie. Pan, zoom and rotate reuse the canvas navigation drag, so
//! they never come through here.
//!
//! A child of `app` so it can work on `AppState`'s private state directly;
//! split out only because `app.rs` is long enough already.

use super::AppState;
use crate::doc::project::Project;
use crate::input::button_drag::{
    drag_steps, past_dead_zone, pie_slot, ButtonDrag, FLIP_PX_PER_KEY, SCRUB_PX_PER_FRAME,
};
use crate::tools::fill::{step_gap, MAX_EXPAND};
use crate::tools::ActiveTool;

/// Screen points of sideways drag that double (or halve) the brush size.
const SIZE_PX_PER_DOUBLING: f32 = 150.0;
/// Screen points of drag up that take the opacity from nothing to full.
const OPACITY_PX: f32 = 300.0;
/// Brush radius bounds for a size drag — the same as the size keys'.
const MIN_RADIUS: f32 = 0.5;
const MAX_RADIUS: f32 = 256.0;
/// A size drag never takes a brush all the way to invisible.
const MIN_OPACITY: f32 = 0.01;

/// A middle- or right-button drag under way on the canvas.
#[derive(Clone, Copy, Debug)]
pub struct ButtonDragState {
    pub kind: ButtonDrag,
    /// Where the press landed, in screen points. Tool adjust keeps the brush
    /// ring here and the swatch pie is centred on it.
    pub anchor: egui::Pos2,
    /// Where a scrub counts from: the press, or wherever Shift last went down
    /// or up — switching between frames and keys starts afresh from there.
    from: egui::Pos2,
    from_frame: usize,
    /// The frame the drag started on, which Flip and Esc go back to.
    start_frame: usize,
    /// Whether the scrub steps key drawings rather than frames.
    keys_only: bool,
    // The tool values at the press, which tool adjust moves away from.
    radius: f32,
    opacity: f32,
    gap: u8,
    expand: u8,
    /// Swatch under the pen, for the pie.
    pub pie_hover: Option<usize>,
}

impl AppState {
    /// Start a drag the pressed button is bound to, at screen point `at`.
    /// [`ButtonDrag::None`] starts one too, which swallows the drag so it
    /// never reaches the active tool.
    pub fn begin_button_drag(&mut self, kind: ButtonDrag, at: egui::Pos2) {
        if matches!(kind, ButtonDrag::Scrub | ButtonDrag::Flip) {
            // Playback rewrites the frame every tick; a scrub over it would be
            // invisible.
            self.playback.stop();
        }
        let frame = self.project.current_frame;
        self.button_drag = Some(ButtonDragState {
            kind,
            anchor: at,
            from: at,
            from_frame: frame,
            start_frame: frame,
            keys_only: kind == ButtonDrag::Flip || (kind == ButtonDrag::Scrub && self.shift_held),
            radius: self.brush.radius,
            opacity: self.brush.opacity,
            gap: self.brush.fill_gap,
            expand: self.brush.fill_expand,
            pie_hover: None,
        });
    }

    /// The pen, still holding the button, is at screen point `at`.
    pub fn button_drag_to(&mut self, at: egui::Pos2) {
        let Some(mut d) = self.button_drag else {
            return;
        };
        match d.kind {
            ButtonDrag::Scrub | ButtonDrag::Flip => {
                let keys_only = d.kind == ButtonDrag::Flip || self.shift_held;
                if keys_only != d.keys_only {
                    d.keys_only = keys_only;
                    d.from = at;
                    d.from_frame = self.project.current_frame;
                }
                let dx = at.x - d.from.x;
                let frame = if d.keys_only {
                    key_steps(&self.project, d.from_frame, (dx / FLIP_PX_PER_KEY).trunc() as i32)
                } else {
                    let n = self.project.frame_count.max(1) as i64;
                    let f = d.from_frame as i64 + (dx / SCRUB_PX_PER_FRAME).trunc() as i64;
                    f.clamp(0, n - 1) as usize
                };
                self.project.goto(frame);
            }
            ButtonDrag::ToolAdjust => {
                let (dx, up) = (at.x - d.anchor.x, d.anchor.y - at.y);
                self.adjust_tool(&d, dx, up);
            }
            ButtonDrag::SwatchPie => {
                let off = at - d.anchor;
                d.pie_hover = pie_slot(self.palette.len(), [off.x, off.y]);
            }
            ButtonDrag::None | ButtonDrag::Pan | ButtonDrag::Zoom | ButtonDrag::Rotate => {}
        }
        self.button_drag = Some(d);
    }

    /// The button came up: finish the drag.
    pub fn end_button_drag(&mut self) {
        let Some(d) = self.button_drag.take() else {
            return;
        };
        match d.kind {
            ButtonDrag::Flip => self.project.goto(d.start_frame),
            ButtonDrag::SwatchPie => {
                if let Some(&rgb) = d.pie_hover.and_then(|i| self.palette.get(i)) {
                    self.set_brush_color(rgb);
                }
            }
            _ => {}
        }
    }

    /// Esc during a button drag: undo what it did so far and ignore the rest
    /// of it until the button comes up. `true` if there was one.
    pub fn cancel_button_drag(&mut self) -> bool {
        let Some(d) = self.button_drag else {
            return false;
        };
        match d.kind {
            ButtonDrag::Scrub | ButtonDrag::Flip => self.project.goto(d.start_frame),
            ButtonDrag::ToolAdjust => {
                self.brush.radius = d.radius;
                self.brush.opacity = d.opacity;
                self.brush.fill_gap = d.gap;
                self.brush.fill_expand = d.expand;
            }
            _ => {}
        }
        self.button_drag = Some(ButtonDragState {
            kind: ButtonDrag::None,
            pie_hover: None,
            ..d
        });
        true
    }

    /// What to show beside the pen while a button drag runs, if anything.
    pub fn button_drag_readout(&self) -> Option<String> {
        let d = self.button_drag.as_ref()?;
        match d.kind {
            ButtonDrag::Scrub | ButtonDrag::Flip => {
                let frame = format!(
                    "{} / {}",
                    self.project.current_frame + 1,
                    self.project.frame_count
                );
                Some(if d.keys_only { format!("{frame} · keys") } else { frame })
            }
            ButtonDrag::ToolAdjust => match self.tool {
                ActiveTool::Fill => Some(format!(
                    "Gap {} · Expand {}",
                    self.brush.fill_gap, self.brush.fill_expand
                )),
                t if adjusts_brush(t) => Some(format!(
                    "Size {:.1} · Opacity {:.0}%",
                    self.brush.radius,
                    self.brush.opacity * 100.0
                )),
                _ => None,
            },
            _ => None,
        }
    }

    /// Tool adjust: `dx` sideways and `up` upward, in screen points from the
    /// press. The bucket steps its gap and expand as its own drag does; a
    /// brush scales its size and slides its opacity.
    fn adjust_tool(&mut self, d: &ButtonDragState, dx: f32, up: f32) {
        match self.tool {
            ActiveTool::Fill => {
                self.brush.fill_gap = step_gap(d.gap, drag_steps(dx));
                self.brush.fill_expand =
                    (d.expand as i32 + drag_steps(up)).clamp(0, MAX_EXPAND as i32) as u8;
            }
            t if adjusts_brush(t) => {
                let grow = (past_dead_zone(dx) / SIZE_PX_PER_DOUBLING).exp2();
                self.brush.radius = (d.radius * grow).clamp(MIN_RADIUS, MAX_RADIUS);
                self.brush.opacity =
                    (d.opacity + past_dead_zone(up) / OPACITY_PX).clamp(MIN_OPACITY, 1.0);
            }
            _ => {}
        }
    }
}

/// Whether tool adjust sets `t`'s size and opacity.
fn adjusts_brush(t: ActiveTool) -> bool {
    matches!(
        t,
        ActiveTool::Pencil | ActiveTool::Ink | ActiveTool::Eraser | ActiveTool::Shape
    )
}

/// The frame of the drawing `steps` key drawings away from the one showing on
/// `from`, on the active layer. Clamps at the first and last drawing; no
/// drawing that way stays on `from`.
fn key_steps(p: &Project, from: usize, steps: i32) -> usize {
    let Some(layer) = p.layers.get(p.current_layer) else {
        return from;
    };
    let n = p.frame_count;
    if steps > 0 {
        let after: Vec<usize> = (from + 1..n).filter(|&f| layer.is_key(f)).collect();
        after
            .get(steps as usize - 1)
            .or(after.last())
            .copied()
            .unwrap_or(from)
    } else if steps < 0 {
        // Back from the drawing that is showing, not from `from` itself: on a
        // held frame the drawing started earlier.
        let showing = (0..=from.min(n.saturating_sub(1))).rev().find(|&f| layer.is_key(f));
        let Some(showing) = showing else {
            return from;
        };
        let before: Vec<usize> = (0..showing).rev().filter(|&f| layer.is_key(f)).collect();
        before
            .get((-steps) as usize - 1)
            .or(before.last())
            .copied()
            .unwrap_or(showing)
    } else {
        from
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    /// Twelve frames, with drawings on frames 0, 3, 6 and 9 of the active
    /// layer and holds between.
    fn timeline() -> AppState {
        let mut st = AppState::for_test();
        while st.project.frame_count < 12 {
            st.project.add_frame();
        }
        let (w, h) = (st.project.width, st.project.height);
        for f in [0, 3, 6, 9] {
            let id = st.project.cells.len();
            st.project.cells.push(crate::doc::canvas::Canvas::new(w, h).into());
            st.project.layers[0].set_key(f, id);
        }
        for f in [1, 2, 4, 5, 7, 8, 10, 11] {
            assert!(!st.project.layers[0].is_key(f), "frame {f} should hold");
        }
        st.project.goto(0);
        st
    }

    const AT: egui::Pos2 = pos2(400.0, 300.0);

    #[test]
    fn scrubbing_moves_a_frame_every_ten_points_and_stays() {
        let mut st = timeline();
        st.begin_button_drag(ButtonDrag::Scrub, AT);
        st.button_drag_to(AT + vec2(9.0, 0.0));
        assert_eq!(st.project.current_frame, 0);
        st.button_drag_to(AT + vec2(35.0, 40.0));
        assert_eq!(st.project.current_frame, 3);
        st.button_drag_to(AT + vec2(900.0, 0.0));
        assert_eq!(st.project.current_frame, 11, "clamps at the end");
        st.button_drag_to(AT + vec2(-50.0, 0.0));
        assert_eq!(st.project.current_frame, 0, "and at the start");
        st.button_drag_to(AT + vec2(52.0, 0.0));
        st.end_button_drag();
        assert_eq!(st.project.current_frame, 5);
        assert!(st.button_drag.is_none());
    }

    #[test]
    fn shift_scrubs_key_to_key_from_where_it_went_down() {
        let mut st = timeline();
        st.project.goto(4);
        st.begin_button_drag(ButtonDrag::Scrub, AT);
        st.button_drag_to(AT + vec2(20.0, 0.0));
        assert_eq!(st.project.current_frame, 6);
        // Shift down: counting starts afresh from here, a drawing per 24.
        st.shift_held = true;
        st.button_drag_to(AT + vec2(20.0, 0.0));
        assert_eq!(st.project.current_frame, 6);
        st.button_drag_to(AT + vec2(44.0, 0.0));
        assert_eq!(st.project.current_frame, 9);
        st.button_drag_to(AT + vec2(-40.0, 0.0));
        assert_eq!(st.project.current_frame, 0, "back two drawings from 6, then clamped");
    }

    #[test]
    fn flipping_goes_drawing_to_drawing_and_lets_go_back_home() {
        let mut st = timeline();
        st.project.goto(7);
        st.begin_button_drag(ButtonDrag::Flip, AT);
        // Back one from the drawing showing on 7 (frame 6's) is frame 3's.
        st.button_drag_to(AT + vec2(-30.0, 0.0));
        assert_eq!(st.project.current_frame, 3);
        st.button_drag_to(AT + vec2(30.0, 0.0));
        assert_eq!(st.project.current_frame, 9);
        assert_eq!(st.button_drag_readout().as_deref(), Some("10 / 12 · keys"));
        st.end_button_drag();
        assert_eq!(st.project.current_frame, 7);
    }

    #[test]
    fn tool_adjust_sizes_and_fades_a_brush() {
        let mut st = AppState::for_test();
        st.dispatch(crate::input::shortcuts::Action::ToolInk);
        st.brush.radius = 4.0;
        st.brush.opacity = 0.5;
        st.begin_button_drag(ButtonDrag::ToolAdjust, AT);
        st.button_drag_to(AT + vec2(8.0 + SIZE_PX_PER_DOUBLING, -(8.0 + 60.0)));
        assert!((st.brush.radius - 8.0).abs() < 1e-3, "{}", st.brush.radius);
        assert!((st.brush.opacity - 0.7).abs() < 1e-3, "{}", st.brush.opacity);
        st.button_drag_to(AT + vec2(-2000.0, 2000.0));
        assert_eq!((st.brush.radius, st.brush.opacity), (MIN_RADIUS, MIN_OPACITY));
        st.end_button_drag();
        assert_eq!(st.brush.radius, MIN_RADIUS, "the new size stays");
    }

    #[test]
    fn tool_adjust_steps_the_buckets_gap_and_expand_without_filling() {
        let mut st = AppState::for_test();
        st.dispatch(crate::input::shortcuts::Action::ToolFill);
        st.brush.fill_gap = 0;
        st.brush.fill_expand = 0;
        let cells = st.project.cells.len();
        st.begin_button_drag(ButtonDrag::ToolAdjust, AT);
        st.button_drag_to(AT + vec2(8.0 + 12.0 * 13.0, -(8.0 + 24.0)));
        assert_eq!((st.brush.fill_gap, st.brush.fill_expand), (14, 2));
        assert_eq!(
            st.button_drag_readout().as_deref(),
            Some("Gap 14 · Expand 2")
        );
        st.end_button_drag();
        assert_eq!(st.project.cells.len(), cells, "nothing was painted");
        assert_eq!(st.history.undo_len(), 0);
    }

    #[test]
    fn the_pie_picks_the_swatch_it_is_let_go_over() {
        let mut st = AppState::for_test();
        st.palette = (0..10u8).map(|i| [i * 20, 0, 0]).collect();
        st.begin_button_drag(ButtonDrag::SwatchPie, AT);
        // Inner ring, pointing right: swatch 2 of 8.
        st.button_drag_to(AT + vec2(50.0, 0.0));
        assert_eq!(st.button_drag.unwrap().pie_hover, Some(2));
        st.end_button_drag();
        assert_eq!(st.brush.color, [40, 0, 0, 255]);

        // Let go in the hole: the colour stays.
        st.begin_button_drag(ButtonDrag::SwatchPie, AT);
        st.button_drag_to(AT + vec2(50.0, 0.0));
        st.button_drag_to(AT + vec2(4.0, 4.0));
        st.end_button_drag();
        assert_eq!(st.brush.color, [40, 0, 0, 255]);
    }

    #[test]
    fn esc_puts_everything_back_and_swallows_the_rest() {
        let mut st = timeline();
        st.begin_button_drag(ButtonDrag::Scrub, AT);
        st.button_drag_to(AT + vec2(60.0, 0.0));
        assert_eq!(st.project.current_frame, 6);
        assert!(st.cancel_button_drag());
        assert_eq!(st.project.current_frame, 0);
        st.button_drag_to(AT + vec2(90.0, 0.0));
        assert_eq!(st.project.current_frame, 0, "the rest of the drag is ignored");
        st.end_button_drag();
        assert!(st.button_drag.is_none());

        st.brush.radius = 4.0;
        st.begin_button_drag(ButtonDrag::ToolAdjust, AT);
        st.button_drag_to(AT + vec2(300.0, 0.0));
        assert!(st.cancel_button_drag());
        assert_eq!(st.brush.radius, 4.0);
    }
}
