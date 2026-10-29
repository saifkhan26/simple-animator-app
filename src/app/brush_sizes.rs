//! The sizes each brush was drawn at: the recent ones and the ones pinned to
//! keep, offered under the Size slider; the detents tool adjust's size drag
//! catches on at them; and swapping back to the size drawn at last.
//!
//! A child of `app` so it can work on `AppState`'s private state directly;
//! split out only because `app.rs` is long enough already.

use super::{ActiveTool, AppState};

/// Recent sizes kept per brush.
pub const RECENT_SIZES: usize = 5;
/// Pinned sizes kept per brush: as many as the strip under the Size slider
/// always has room for.
pub const MAX_PINNED_SIZES: usize = 5;
/// How far, in screen points, tool adjust's size drag holds on a remembered
/// size before carrying on past it. A detent, not a snap: every size either
/// side of it is still in reach.
pub const SIZE_DETENT_PX: f32 = 10.0;

/// One brush's remembered sizes: drawn at, newest first, and pinned,
/// smallest first. A workspace preference, like the palette.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SizeMemory {
    pub recent: Vec<f32>,
    pub pinned: Vec<f32>,
}

/// Two sizes that are the same to a tenth, as the strip shows them.
pub fn same_size(a: f32, b: f32) -> bool {
    (a * 10.0).round() == (b * 10.0).round()
}

/// A size as the strip shows it: a tenth at most, no trailing ".0".
pub fn size_label(s: f32) -> String {
    let t = format!("{:.1}", s);
    t.strip_suffix(".0").map(str::to_string).unwrap_or(t)
}

/// Tool adjust's size `d` points past its dead zone from `from`, doubling
/// every `per_doubling` points, and the stop it is held on, if any. Each of
/// `stops` it reaches holds it for `detent` points; past that it carries on
/// from the stop, so a size just past one is as near as one just short.
pub fn size_with_detents(from: f32, d: f32, per_doubling: f32, stops: &[f32], detent: f32) -> (f32, Option<f32>) {
    let w = detent / per_doubling;
    let mut u = d / per_doubling;
    // Each stop's distance from `from` in doublings, nearest first, on the
    // side the drag went.
    let mut ahead: Vec<(f32, f32)> = stops
        .iter()
        .filter(|&&s| s > 0.0)
        .map(|&s| ((s / from).log2(), s))
        .filter(|&(us, _)| us != 0.0 && us.signum() == u.signum())
        .collect();
    ahead.sort_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
    for (us, s) in ahead {
        if u.abs() <= us.abs() {
            break;
        }
        if u.abs() <= us.abs() + w {
            return (s, Some(s));
        }
        u -= w * u.signum();
    }
    (from * u.exp2(), None)
}

impl AppState {
    /// Which brush's sizes the active tool shows. Shape draws at the size
    /// of the pen it follows, so it shares that pen's.
    pub fn size_slot(&self) -> usize {
        if self.tool == ActiveTool::Shape {
            self.last_pen.idx()
        } else {
            self.tool.idx()
        }
    }

    /// The active brush's remembered sizes.
    pub fn size_memory(&self) -> SizeMemory {
        self.brush_sizes.get(self.size_slot()).cloned().unwrap_or_default()
    }

    fn size_memory_mut(&mut self) -> &mut SizeMemory {
        let i = self.size_slot();
        if self.brush_sizes.len() <= i {
            self.brush_sizes.resize(i + 1, SizeMemory::default());
        }
        &mut self.brush_sizes[i]
    }

    /// A stroke landed at the brush's size: to the front of the recent ones.
    pub(super) fn remember_size(&mut self) {
        let r = self.brush.radius;
        let m = self.size_memory_mut();
        m.recent.retain(|&s| !same_size(s, r));
        m.recent.insert(0, r);
        m.recent.truncate(RECENT_SIZES);
    }

    /// Pin `size` to keep, or unpin it if it is pinned. No more than
    /// [`MAX_PINNED_SIZES`]: one past that is not pinned.
    pub fn toggle_pinned_size(&mut self, size: f32) {
        let m = self.size_memory_mut();
        if let Some(i) = m.pinned.iter().position(|&s| same_size(s, size)) {
            m.pinned.remove(i);
        } else if m.pinned.len() < MAX_PINNED_SIZES {
            m.pinned.push(size);
            m.pinned.sort_by(f32::total_cmp);
        }
    }

    /// Swap to the size drawn at last, or back to the one swapped from. At
    /// the size drawn at last already, the one drawn at before it.
    pub fn swap_size(&mut self) {
        let (slot, r) = (self.size_slot(), self.brush.radius);
        let to = match self.size_swap {
            Some((s, to, from)) if s == slot && same_size(to, r) => Some(from),
            _ => self.size_memory().recent.into_iter().find(|&s| !same_size(s, r)),
        };
        if let Some(to) = to {
            self.size_swap = Some((slot, to, r));
            self.brush.radius = to;
        }
    }

    /// The remembered sizes tool adjust's size drag from `from` catches on:
    /// pinned and recent, other than `from` itself.
    pub(super) fn size_detents(&self, from: f32) -> Vec<f32> {
        let m = self.size_memory();
        let mut v: Vec<f32> = m.pinned.iter().chain(&m.recent).copied().filter(|&s| !same_size(s, from)).collect();
        v.sort_by(f32::total_cmp);
        v.dedup_by(|a, b| same_size(*a, *b));
        v
    }

    /// What the brush's size is to the remembered ones, for tool adjust's
    /// readout: one of them, or how far from the size drawn at last.
    pub(super) fn size_note(&self) -> String {
        let (r, m) = (self.brush.radius, self.size_memory());
        if m.pinned.iter().any(|&s| same_size(s, r)) {
            " (pinned)".to_string()
        } else if m.recent.first().is_some_and(|&s| same_size(s, r)) {
            " (drawn last)".to_string()
        } else if m.recent.iter().any(|&s| same_size(s, r)) {
            " (recent)".to_string()
        } else if let Some(&last) = m.recent.first() {
            format!(" (last {})", size_label(last))
        } else {
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::shortcuts::Action;

    const PER: f32 = 150.0;

    /// A stop holds the drag for the detent, then the drag carries on from
    /// it: a size just past the stop is reachable, every size is, and the
    /// size never goes back as the drag goes on.
    #[test]
    fn a_stop_holds_the_drag_then_lets_it_go_on_from_there() {
        let stops = [20.0];
        assert_eq!(size_with_detents(10.0, 0.0, PER, &stops, 10.0), (10.0, None));
        assert_eq!(size_with_detents(10.0, 151.0, PER, &stops, 10.0), (20.0, Some(20.0)));
        assert_eq!(size_with_detents(10.0, 159.0, PER, &stops, 10.0), (20.0, Some(20.0)));
        let (just_past, held) = size_with_detents(10.0, 161.0, PER, &stops, 10.0);
        assert!(held.is_none() && just_past > 20.0 && just_past < 20.1, "{just_past}");
        let (short, _) = size_with_detents(10.0, 149.0, PER, &stops, 10.0);
        assert!(short < 20.0 && short > 19.9, "{short}");

        let mut last = 0.0;
        for d in -600..=600 {
            let (s, _) = size_with_detents(10.0, d as f32, PER, &[2.0, 4.0, 20.0, 40.0], 10.0);
            assert!(s >= last, "went back at {d}: {last} → {s}");
            last = s;
        }
    }

    /// Smaller stops hold a drag the other way, and stops behind the drag
    /// are passed by.
    #[test]
    fn stops_below_hold_a_drag_down() {
        let stops = [5.0, 40.0];
        assert_eq!(size_with_detents(10.0, -151.0, PER, &stops, 10.0), (5.0, Some(5.0)));
        let (s, held) = size_with_detents(10.0, -161.0, PER, &stops, 10.0);
        assert!(held.is_none() && s < 5.0 && s > 4.97, "{s}");
        // Two stops up: the second is ten points further than its size says.
        let (s, held) = size_with_detents(10.0, 2.0 * PER + 10.0 + 1.0, PER, &[20.0, 40.0], 10.0);
        assert_eq!((s, held), (40.0, Some(40.0)));
    }

    fn drawn(st: &mut AppState, size: f32) {
        st.brush.radius = size;
        st.pointer_down(st.make_sample(20.0, 20.0, 0.0));
        st.pointer_move(st.make_sample(60.0, 20.0, 0.1));
        st.pointer_up();
    }

    /// Each brush keeps the sizes it was drawn at, newest first, five of
    /// them, no size twice; Shape keeps the pen's.
    #[test]
    fn each_brush_remembers_the_sizes_it_drew_at() {
        let mut st = AppState::for_test();
        st.dispatch(Action::ToolInk);
        for s in [3.0, 6.0, 3.0, 9.0, 12.0, 15.0, 18.0] {
            drawn(&mut st, s);
        }
        assert_eq!(st.size_memory().recent, vec![18.0, 15.0, 12.0, 9.0, 3.0]);
        st.brush.radius = 7.0;
        assert!(st.size_memory().recent[0] == 18.0, "only drawing remembers");

        st.dispatch(Action::ToolEraser);
        assert!(st.size_memory().recent.is_empty(), "the eraser has its own");
        drawn(&mut st, 40.0);
        assert_eq!(st.size_memory().recent, vec![40.0]);
        st.dispatch(Action::ToolShape);
        assert_eq!(st.size_memory().recent[0], 18.0, "Shape follows the ink");
    }

    /// The swap goes to the size drawn at last, and back again.
    #[test]
    fn swapping_goes_to_the_size_drawn_at_last_and_back() {
        let mut st = AppState::for_test();
        st.dispatch(Action::ToolInk);
        drawn(&mut st, 4.0);
        drawn(&mut st, 12.0);
        st.brush.radius = 30.0;
        st.dispatch(Action::SizeSwap);
        assert_eq!(st.brush.radius, 12.0, "the size drawn at last");
        st.dispatch(Action::SizeSwap);
        assert_eq!(st.brush.radius, 30.0, "and back");
        st.dispatch(Action::SizeSwap);
        assert_eq!(st.brush.radius, 12.0);
        // At the size drawn at last, the one before it.
        st.size_swap = None;
        st.dispatch(Action::SizeSwap);
        assert_eq!(st.brush.radius, 4.0);
    }

    #[test]
    fn pins_toggle_and_stay_sorted() {
        let mut st = AppState::for_test();
        st.toggle_pinned_size(8.0);
        st.toggle_pinned_size(2.5);
        assert_eq!(st.size_memory().pinned, vec![2.5, 8.0]);
        st.toggle_pinned_size(8.04);
        assert_eq!(st.size_memory().pinned, vec![2.5], "the same to a tenth");
        for s in 0..20 {
            st.toggle_pinned_size(10.0 + s as f32);
        }
        assert_eq!(st.size_memory().pinned.len(), MAX_PINNED_SIZES);
    }

    #[test]
    fn labels_drop_a_trailing_zero() {
        assert_eq!(size_label(12.0), "12");
        assert_eq!(size_label(3.45), "3.5");
        assert_eq!(size_label(0.5), "0.5");
    }
}
