//! The pen slots: the row of pens at the top of the Tools panel. Each slot
//! holds any brush, at its own size, with its own smoothing, colour, name
//! and icon. There are one to [`MAX_PENS`] of them; the first two start as
//! the pencil and the ink. The row's "+" adds one, and every slot can be
//! renamed, given another icon or brush, copied, or removed, the last one
//! excepted.
//!
//! A workspace preference, like the tool brushes: the slots are kept in the
//! app's prefs, never in the `.anim`. A prefs file from before there were
//! slots gives its pencil and ink as the first two, and the pencil's and the
//! ink's old fields are still written, so an older build finds them.
//!
//! A child of `app`, like `brush_sizes`, so it can work on `AppState`'s
//! private state directly.

use egui_phosphor::regular as ic;

use super::brush_sizes::SizeMemory;
use super::{default_tool_brushes, restore_pen_smoothing, AppState, UiPrefs, LEGACY_PENS, TOOL_COUNT};
use crate::input::shortcuts::Action;
use crate::tools::{ActiveTool, BrushSettings, Smoothing, SmoothingOptions};

/// The most pen slots the row takes: six and the eraser make seven buttons,
/// as many as the row of the other tools under it.
pub const MAX_PENS: usize = Action::PENS.len();

/// The icons a slot's button can wear: the name the prefs keep it by, the
/// glyph, and what the picker calls it.
pub const PEN_ICONS: [(&str, &str, &str); 16] = [
    ("pencil", ic::PENCIL, "Pencil"),
    ("pencil-simple", ic::PENCIL_SIMPLE, "Plain pencil"),
    ("pencil-line", ic::PENCIL_LINE, "Pencil and line"),
    ("pen", ic::PEN, "Pen"),
    ("pen-nib", ic::PEN_NIB, "Nib"),
    ("pen-nib-straight", ic::PEN_NIB_STRAIGHT, "Straight nib"),
    ("paint-brush", ic::PAINT_BRUSH, "Brush"),
    ("paint-brush-broad", ic::PAINT_BRUSH_BROAD, "Broad brush"),
    ("highlighter", ic::HIGHLIGHTER, "Highlighter"),
    ("marker", ic::MARKER_CIRCLE, "Marker"),
    ("feather", ic::FEATHER, "Feather"),
    ("spray", ic::SPRAY_BOTTLE, "Spray"),
    ("scribble", ic::SCRIBBLE, "Scribble"),
    ("signature", ic::SIGNATURE, "Signature"),
    ("sparkle", ic::SPARKLE, "Sparkle"),
    ("cloud", ic::CLOUD, "Cloud"),
];

/// The glyph for an icon the prefs name. One this build doesn't know (from
/// a newer one) gets the plain pencil rather than failing the prefs.
pub fn icon_glyph(key: &str) -> &'static str {
    PEN_ICONS.iter().find(|i| i.0 == key).map_or(ic::PENCIL_SIMPLE, |i| i.1)
}

/// The key action that picks slot `i`. The first two keep the names the
/// pencil's and the ink's keys always had, so a `shortcuts.toml` binding
/// them still binds the same slots.
pub fn pen_action(i: usize) -> Option<Action> {
    Action::PENS.get(i).copied()
}

fn basic() -> Smoothing {
    Smoothing::Basic
}

/// One pen slot. Its name and icon are only what the button shows: the
/// brush is what draws.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PenSlot {
    /// As the artist named it. Left blank, it shows as "Pen n".
    pub name: String,
    /// A [`PEN_ICONS`] name. Kept as text, so an icon this build doesn't
    /// know can't fail the prefs and reset every other preference.
    pub icon: String,
    pub brush: BrushSettings,
    pub smoothing: SmoothingOptions,
    /// The sizes this slot drew at lately, and the ones pinned.
    pub sizes: SizeMemory,
    /// The smoothing it had before the Stabilizer key turned it on, for the
    /// key to turn it back to. Session-only.
    #[serde(skip, default = "basic")]
    pub before_stabilizer: Smoothing,
}

impl Default for PenSlot {
    /// A new slot: a pencil brush under a neutral name, ready for whatever
    /// brush goes in it.
    fn default() -> Self {
        Self::new("Brush", "paint-brush", BrushSettings::default_pencil())
    }
}

impl PenSlot {
    pub fn new(name: &str, icon: &str, brush: BrushSettings) -> Self {
        Self {
            name: name.to_owned(),
            icon: icon.to_owned(),
            brush,
            smoothing: SmoothingOptions::default(),
            sizes: SizeMemory::default(),
            before_stabilizer: Smoothing::Basic,
        }
    }

    pub fn pencil() -> Self {
        Self::new("Pencil", "pencil", BrushSettings::default_pencil())
    }

    pub fn ink() -> Self {
        Self::new("Ink", "pen-nib", BrushSettings::default_ink())
    }

    pub fn glyph(&self) -> &'static str {
        icon_glyph(&self.icon)
    }
}

/// The slots a fresh workspace starts with.
pub fn default_pens() -> Vec<PenSlot> {
    vec![PenSlot::pencil(), PenSlot::ink()]
}

/// The slots the prefs keep, or from a prefs file written before there were
/// slots, its pencil and ink: their brushes, smoothing and sizes as they
/// were. `legacy` is that file's per-tool brushes, pencil and ink first.
pub(super) fn restore_pens(prefs: &UiPrefs, legacy: &[BrushSettings; TOOL_COUNT]) -> Vec<PenSlot> {
    if !prefs.pens.is_empty() {
        let mut pens = prefs.pens.clone();
        pens.truncate(MAX_PENS);
        return pens;
    }
    let smoothing = restore_pen_smoothing(prefs);
    let mut pens = default_pens();
    for (i, p) in pens.iter_mut().enumerate().take(LEGACY_PENS) {
        p.brush = legacy[i].clone();
        p.smoothing = smoothing[i];
        p.sizes = prefs.brush_sizes.get(i).cloned().unwrap_or_default();
    }
    pens
}

impl AppState {
    /// Slot `i`'s name as its button and the Tools panel show it.
    pub fn pen_name(&self, i: usize) -> String {
        match self.pens.get(i) {
            Some(p) if !p.name.trim().is_empty() => p.name.trim().to_owned(),
            _ => format!("Pen {}", i + 1),
        }
    }

    /// Make slot `i` the tool, if there is one.
    pub fn select_pen(&mut self, i: usize) {
        if i < self.pens.len() {
            self.set_tool(ActiveTool::Pen(i as u8));
        }
    }

    /// The next slot along the row, or the one before, round at the ends.
    /// From a tool that isn't a pen, back to the pen used last.
    pub fn step_pen(&mut self, forward: bool) {
        let n = self.pens.len();
        let to = match self.tool.pen() {
            Some(i) if forward => (i + 1) % n,
            Some(i) => (i + n - 1) % n,
            None => self.last_pen.pen().unwrap_or(0),
        };
        self.select_pen(to);
    }

    /// Back to the pen used before this one, and pressed again, back here.
    /// From a tool that isn't a pen, back to the pen used last.
    pub fn swap_pen(&mut self) {
        let to = if self.tool.is_pen() { self.prev_pen } else { Some(self.last_pen) };
        if let Some(i) = to.and_then(ActiveTool::pen) {
            self.select_pen(i);
        }
    }

    /// A new slot at the end of the row, holding a pencil brush in the
    /// colour of the pen used last, made the tool. Its place, or `None` with
    /// the row full.
    pub fn add_pen(&mut self) -> Option<usize> {
        if self.pens.len() >= MAX_PENS {
            return None;
        }
        self.stash_brush();
        let mut slot = PenSlot {
            name: self.unused_pen_name("Brush"),
            ..PenSlot::default()
        };
        slot.brush.color = self.brush_of(self.last_pen).color;
        self.pens.push(slot);
        let at = self.pens.len() - 1;
        self.select_pen(at);
        Some(at)
    }

    /// A copy of slot `i` right after it: its brush, sizes, smoothing and
    /// icon, under its name with a number after it. Made the tool. Its
    /// place, or `None` with the row full.
    pub fn duplicate_pen(&mut self, i: usize) -> Option<usize> {
        if self.pens.len() >= MAX_PENS || i >= self.pens.len() {
            return None;
        }
        self.stash_brush();
        let mut slot = self.pens[i].clone();
        slot.name = self.unused_pen_name(&self.pen_name(i));
        let at = i + 1;
        self.pens.insert(at, slot);
        self.remap_pens(|j| if j >= at { j + 1 } else { j });
        self.select_pen(at);
        Some(at)
    }

    /// Take slot `i` out of the row, its brush with it. The last slot stays.
    /// A tool on it goes to the slot before it, or the one after for the
    /// first.
    pub fn remove_pen(&mut self, i: usize) -> bool {
        if self.pens.len() <= 1 || i >= self.pens.len() {
            return false;
        }
        let gone = ActiveTool::Pen(i as u8);
        let on_it = self.tool == gone;
        let follows_it = self.tool == ActiveTool::Shape && self.last_pen == gone;
        if on_it {
            // As a tool switch would: the float and the gesture are the
            // tool's.
            self.land_float();
            self.cancel_gesture();
        }
        self.stash_brush();
        self.pens.remove(i);
        let near = i.saturating_sub(1);
        self.remap_pens(|j| match j {
            _ if j == i => near,
            _ if j > i => j - 1,
            _ => j,
        });
        if on_it || follows_it {
            // Not `set_tool`: its stash would write the removed slot's brush
            // over the one taking its place.
            self.load_brush(self.tool);
        }
        true
    }

    /// Put `brush` in slot `i`. The slot in hand takes it at once; so does
    /// Shape while it follows that slot's size.
    pub fn set_pen_brush(&mut self, i: usize, brush: BrushSettings) {
        let t = ActiveTool::Pen(i as u8);
        if self.tool == t {
            self.brush = brush.clone();
        } else if self.tool == ActiveTool::Shape && self.last_pen == t {
            self.brush.radius = brush.radius;
        }
        if let Some(p) = self.pens.get_mut(i) {
            p.brush = brush;
        }
    }

    /// Every slot reference after slots moved: `f` takes an old slot to the
    /// one now in its place.
    fn remap_pens(&mut self, f: impl Fn(usize) -> usize) {
        let m = |t: ActiveTool| match t {
            ActiveTool::Pen(j) => ActiveTool::Pen(f(j as usize) as u8),
            t => t,
        };
        self.tool = m(self.tool);
        self.last_pen = m(self.last_pen);
        let last = self.last_pen;
        self.prev_pen = self.prev_pen.map(m).filter(|&p| p != last);
        self.screen_pick_return_tool = m(self.screen_pick_return_tool);
        self.size_swap = None;
    }

    /// The per-tool brushes, smoothing and sizes in the prefs' layout from
    /// before the slots, pencil and ink first: the first two slots in their
    /// places, so a build from before the slots still finds them.
    pub(super) fn legacy_tool_prefs(&self) -> (Vec<BrushSettings>, Vec<SmoothingOptions>, Vec<SizeMemory>) {
        let defaults = default_tool_brushes();
        let firsts = || (0..LEGACY_PENS).map(|i| self.pens.get(i));
        let mut brushes: Vec<BrushSettings> =
            firsts().enumerate().map(|(i, p)| p.map_or_else(|| defaults[i].clone(), |p| p.brush.clone())).collect();
        brushes.extend(self.tool_brushes.iter().cloned());
        let mut smoothing: Vec<SmoothingOptions> = firsts().map(|p| p.map_or_else(SmoothingOptions::default, |p| p.smoothing)).collect();
        smoothing.push(self.eraser_smoothing);
        let mut sizes: Vec<SizeMemory> = firsts().map(|p| p.map(|p| p.sizes.clone()).unwrap_or_default()).collect();
        sizes.extend(self.brush_sizes.iter().cloned());
        (brushes, smoothing, sizes)
    }

    /// `base`, or with a number after it when a slot has that name already.
    /// A number `base` ends in already is the one counted on: a copy of
    /// "Brush 2" is "Brush 3", not "Brush 2 2".
    fn unused_pen_name(&self, base: &str) -> String {
        let taken = |n: &str| self.pens.iter().any(|p| p.name.trim().eq_ignore_ascii_case(n));
        if !taken(base) {
            return base.to_owned();
        }
        let base = match base.rsplit_once(' ') {
            Some((stem, n)) if !stem.is_empty() && n.parse::<u32>().is_ok() => stem,
            _ => base,
        };
        (2..).map(|k| format!("{base} {k}")).find(|n| !taken(n)).expect("a free name")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatch_all(st: &mut AppState, actions: &[Action]) {
        for &a in actions {
            st.dispatch(a);
        }
    }

    /// A fresh workspace has the pencil and the ink, and nothing else.
    #[test]
    fn a_fresh_workspace_has_the_pencil_and_the_ink() {
        let st = AppState::for_test();
        let names: Vec<_> = (0..st.pens.len()).map(|i| st.pen_name(i)).collect();
        assert_eq!(names, ["Pencil", "Ink"]);
        assert_eq!(st.tool, ActiveTool::PENCIL);
        assert_eq!(st.pens[1].brush.mode, BrushSettings::default_ink().mode);
    }

    /// A slot added goes at the end and becomes the tool, in the colour of
    /// the pen used last, under a name no other slot has.
    #[test]
    fn an_added_slot_goes_at_the_end_in_the_last_pens_colour() {
        let mut st = AppState::for_test();
        st.set_tool(ActiveTool::INK);
        st.brush.color = [200, 10, 10, 255];
        st.set_tool(ActiveTool::Fill);
        assert_eq!(st.add_pen(), Some(2));
        assert_eq!(st.tool, ActiveTool::Pen(2));
        assert_eq!(st.pen_name(2), "Brush");
        assert_eq!(st.brush.color, [200, 10, 10, 255], "the ink's colour");
        assert_eq!(st.pens[1].brush.color, [200, 10, 10, 255], "the ink kept its own");
        assert_eq!(st.add_pen(), Some(3));
        assert_eq!(st.pen_name(3), "Brush 2");
    }

    /// A copy lands right after its slot with that slot's brush, sizes and
    /// smoothing, numbered on from its name.
    #[test]
    fn a_copy_lands_after_its_slot_with_everything_it_had() {
        let mut st = AppState::for_test();
        st.brush.radius = 23.0;
        st.toggle_pinned_size(23.0);
        st.dispatch(Action::StabilizerToggle);
        assert_eq!(st.duplicate_pen(0), Some(1));
        assert_eq!(st.tool, ActiveTool::Pen(1));
        assert_eq!((st.pen_name(1), st.pen_name(2)), ("Pencil 2".into(), "Ink".into()));
        assert_eq!(st.brush.radius, 23.0);
        assert_eq!(st.size_memory().pinned, vec![23.0]);
        assert_eq!(st.smoothing().unwrap().kind, Smoothing::Stabilizer);
        assert_eq!(st.pens[1].icon, st.pens[0].icon);
        assert_eq!(st.duplicate_pen(1), Some(2));
        assert_eq!(st.pen_name(2), "Pencil 3", "not Pencil 2 2");
    }

    /// No more than six slots, and the last one can't go.
    #[test]
    fn the_row_holds_one_to_six_slots() {
        let mut st = AppState::for_test();
        while st.add_pen().is_some() {}
        assert_eq!(st.pens.len(), MAX_PENS);
        while st.remove_pen(0) {}
        assert_eq!(st.pens.len(), 1);
        assert_eq!(st.tool, ActiveTool::Pen(0));
        assert!(!st.remove_pen(0));
    }

    /// Removing the slot in hand goes to the one before it, with that slot's
    /// own brush, not the removed one's.
    #[test]
    fn removing_the_slot_in_hand_goes_to_the_one_before() {
        let mut st = AppState::for_test();
        st.brush.radius = 3.0;
        st.set_tool(ActiveTool::INK);
        st.brush.radius = 30.0;
        assert!(st.remove_pen(1));
        assert_eq!(st.tool, ActiveTool::PENCIL);
        assert_eq!(st.brush.radius, 3.0);
        assert_eq!(st.pens[0].brush.radius, 3.0);
        assert_eq!(st.pens.len(), 1);
    }

    /// Removing a slot before the one in hand keeps the same pen in hand.
    #[test]
    fn removing_an_earlier_slot_keeps_the_pen_in_hand() {
        let mut st = AppState::for_test();
        st.set_tool(ActiveTool::INK);
        st.brush.radius = 17.0;
        assert!(st.remove_pen(0));
        assert_eq!(st.tool, ActiveTool::Pen(0));
        assert_eq!(st.pen_name(0), "Ink");
        assert_eq!(st.brush.radius, 17.0);
        st.set_tool(ActiveTool::Fill);
        st.set_tool(ActiveTool::Pen(0));
        assert_eq!(st.brush.radius, 17.0, "and its slot has it");
    }

    /// Shape following a removed slot follows the slot taking its place.
    #[test]
    fn shape_follows_the_slot_that_takes_a_removed_ones_place() {
        let mut st = AppState::for_test();
        st.brush.radius = 5.0;
        st.set_tool(ActiveTool::INK);
        st.brush.radius = 25.0;
        st.set_tool(ActiveTool::Shape);
        assert_eq!(st.brush.radius, 25.0);
        assert!(st.remove_pen(1));
        assert_eq!(st.tool, ActiveTool::Shape);
        assert_eq!(st.brush.radius, 5.0, "the pencil's size now");
    }

    /// Next and previous go round the row, and from another tool, back to
    /// the pen used last.
    #[test]
    fn next_and_previous_go_round_the_row() {
        let mut st = AppState::for_test();
        st.add_pen();
        st.dispatch(Action::ToolPencil);
        let mut seen = Vec::new();
        for _ in 0..4 {
            st.dispatch(Action::PenNext);
            seen.push(st.tool);
        }
        assert_eq!(seen, [ActiveTool::Pen(1), ActiveTool::Pen(2), ActiveTool::Pen(0), ActiveTool::Pen(1)]);
        st.dispatch(Action::PenPrev);
        st.dispatch(Action::PenPrev);
        assert_eq!(st.tool, ActiveTool::Pen(2), "round the start");
        st.dispatch(Action::ToolFill);
        st.dispatch(Action::PenNext);
        assert_eq!(st.tool, ActiveTool::Pen(2), "back to the pen used last first");
    }

    /// The swap goes to the pen used before, and back, through other tools.
    #[test]
    fn the_swap_goes_between_the_last_two_pens() {
        let mut st = AppState::for_test();
        st.add_pen();
        dispatch_all(&mut st, &[Action::ToolPen3, Action::ToolPencil, Action::PenSwap]);
        assert_eq!(st.tool, ActiveTool::Pen(2));
        st.dispatch(Action::PenSwap);
        assert_eq!(st.tool, ActiveTool::Pen(0));
        dispatch_all(&mut st, &[Action::ToolFill, Action::PenSwap]);
        assert_eq!(st.tool, ActiveTool::Pen(0), "from Fill, the pen used last");
        st.dispatch(Action::PenSwap);
        assert_eq!(st.tool, ActiveTool::Pen(2));
    }

    /// A slot key with no slot behind it does nothing.
    #[test]
    fn a_key_for_a_missing_slot_does_nothing() {
        let mut st = AppState::for_test();
        st.dispatch(Action::ToolPen6);
        assert_eq!(st.tool, ActiveTool::PENCIL);
        assert_eq!(pen_action(5), Some(Action::ToolPen6));
        assert_eq!(Action::ToolPen6.pen_slot(), Some(5));
        assert_eq!(pen_action(MAX_PENS), None);
    }

    /// A brush put in a slot not in hand waits there; in the slot in hand,
    /// it draws at once.
    #[test]
    fn a_brush_goes_into_its_slot() {
        let mut st = AppState::for_test();
        st.set_pen_brush(1, BrushSettings::airbrush());
        assert_eq!(st.brush.radius, BrushSettings::default_pencil().radius, "the pencil in hand is as it was");
        st.set_tool(ActiveTool::INK);
        assert_eq!(st.brush.radius, BrushSettings::airbrush().radius);
        st.set_pen_brush(1, BrushSettings::default_pencil());
        assert_eq!(st.brush.radius, BrushSettings::default_pencil().radius);
    }

    /// Each slot keeps its own smoothing, the new ones too.
    #[test]
    fn every_slot_keeps_its_own_smoothing() {
        let mut st = AppState::for_test();
        st.add_pen();
        st.dispatch(Action::StabilizerToggle);
        assert_eq!(st.smoothing().unwrap().kind, Smoothing::Stabilizer);
        for i in 0..2 {
            assert_eq!(st.smoothing_for(ActiveTool::Pen(i)).unwrap().kind, Smoothing::Basic);
        }
        st.dispatch(Action::StabilizerToggle);
        assert_eq!(st.smoothing().unwrap().kind, Smoothing::Basic, "back to what it had");
    }

    /// An icon from a newer build shows as the plain pencil.
    #[test]
    fn an_unknown_icon_shows_as_the_plain_pencil() {
        assert_eq!(icon_glyph("pen-nib"), ic::PEN_NIB);
        assert_eq!(icon_glyph("rocket"), ic::PENCIL_SIMPLE);
        let names: std::collections::HashSet<_> = PEN_ICONS.iter().map(|i| i.0).collect();
        assert_eq!(names.len(), PEN_ICONS.len(), "no icon twice");
    }
}
