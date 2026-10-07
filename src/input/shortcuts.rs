//! Configurable keyboard shortcuts.
//!
//! Defaults are concentrated on the left side of QWERTY so a tablet user can
//! operate them with their non-dominant hand without lifting the pen.
//!
//! `ShortcutMap` is serialised to TOML in the user's config directory so
//! rebinds persist across runs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::input::button_drag::ButtonDrag;

#[derive(Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    ToolPencil,
    ToolInk,
    ToolEraser,
    ToolFill,
    /// Retired — the in-canvas eyedropper was folded into `PickScreenColor`.
    /// Kept so a `shortcuts.toml` written before the merge still deserializes;
    /// `load` strips it. Not in [`Action::ALL`], so it never reaches the UI.
    ToolColorPicker,
    PickScreenColor,
    ToolShape,
    ToolTracker,
    ToolLasso,
    ToolPerspective,
    ToolLiquify,
    TogglePerspectiveGrid,
    TogglePerspectiveSnap,
    /// Make the next visible perspective grid the active one — the one
    /// strokes snap to.
    PerspectiveNextGrid,
    PlayPause,
    FramePrev,
    FrameNext,
    KeyJumpPrev,
    KeyJumpNext,
    FrameAdd,
    FrameDuplicate,
    FrameDelete,
    OnionToggle,
    LayerAdd,
    LayerDelete,
    LayerToggleVisible,
    LayerLast,
    /// Step the active layer one row up / down the Layers panel. Stops at the
    /// ends, like the key jumps, so holding the key never wraps around.
    LayerSelectAbove,
    LayerSelectBelow,
    LayerSelectTop,
    LayerSelectBottom,
    /// Reorder: swap the active layer with the one above / below it.
    LayerMoveUp,
    LayerMoveDown,
    /// Select the Nth layer counting down from the top of the Layers panel.
    LayerPick1,
    LayerPick2,
    LayerPick3,
    LayerPick4,
    LayerPick5,
    LayerPick6,
    LayerPick7,
    LayerPick8,
    LayerPick9,
    FadeOthersToggle,
    LayerClipToggle,
    LayerAlphaLockToggle,
    KeyBlank,
    KeyCopy,
    Hold,
    SizeDown,
    SizeUp,
    Undo,
    Redo,
    ClearCell,
    PasteImage,
    ToggleCheckerBg,
    TogglePanels,
    ToggleMiniTimeline,
    ZoomReset,
    PanReset,
    RotateReset,
    FlipHorizontal,
    FlipVertical,
    // The art itself: the selection, or the drawing. Unbound by default.
    ArtFlipH,
    ArtFlipV,
    ArtRotateCw,
    ArtRotateCcw,
    ArtRotate180,
    // Drawing (cell) clipboard — one layer's drawing at one frame.
    CellCut,
    CellCopy,
    CellPaste,
    // Pixel selection (lasso).
    SelectionCut,
    SelectionCopy,
    SelectionPaste,
    SelectionDelete,
    SelectionDeselect,
    SelectAll,
    SelectInvert,
    SelectionFill,
    // Selection modes (modifier-only binds: held as a selection drag starts).
    SelModeAdd,
    SelModeSubtract,
    SelModeIntersect,
    // Timeline tracks: the drawings selected there.
    TrackClear,
    TrackCloseGap,
    SaveProject,
    SaveProjectAs,
    OpenProject,
    // Canvas drag gestures: held as a canvas drag starts — a key, bare
    // modifiers, or both. The names predate the key option and stay, so old
    // shortcut files keep loading.
    CanvasZoom,
    CanvasPan,
    CanvasRotate,
    // The button-drag modes, held the same way. Unbound until the user picks
    // a key.
    HoldToolAdjust,
    HoldScrub,
    HoldFlip,
    HoldSwatchPie,
    // Layer transform editing.
    LayerTransformToggle,
    TransformKeyAdd,
    TransformKeyDelete,
    // Camera.
    CameraEditToggle,
    CameraKeyAdd,
    CameraKeyDelete,
    CameraLookThrough,
}

impl Action {
    pub const ALL: &'static [Action] = &[
        Action::ToolPencil,
        Action::ToolInk,
        Action::ToolEraser,
        Action::ToolFill,
        Action::PickScreenColor,
        Action::ToolShape,
        Action::ToolTracker,
        Action::ToolLasso,
        Action::ToolPerspective,
        Action::ToolLiquify,
        Action::TogglePerspectiveGrid,
        Action::TogglePerspectiveSnap,
        Action::PerspectiveNextGrid,
        Action::PlayPause,
        Action::FramePrev,
        Action::FrameNext,
        Action::KeyJumpPrev,
        Action::KeyJumpNext,
        Action::FrameAdd,
        Action::FrameDuplicate,
        Action::FrameDelete,
        Action::OnionToggle,
        Action::LayerAdd,
        Action::LayerDelete,
        Action::LayerToggleVisible,
        Action::LayerLast,
        Action::LayerSelectAbove,
        Action::LayerSelectBelow,
        Action::LayerSelectTop,
        Action::LayerSelectBottom,
        Action::LayerMoveUp,
        Action::LayerMoveDown,
        Action::LayerPick1,
        Action::LayerPick2,
        Action::LayerPick3,
        Action::LayerPick4,
        Action::LayerPick5,
        Action::LayerPick6,
        Action::LayerPick7,
        Action::LayerPick8,
        Action::LayerPick9,
        Action::FadeOthersToggle,
        Action::LayerClipToggle,
        Action::LayerAlphaLockToggle,
        Action::KeyBlank,
        Action::KeyCopy,
        Action::Hold,
        Action::SizeDown,
        Action::SizeUp,
        Action::Undo,
        Action::Redo,
        Action::ClearCell,
        Action::PasteImage,
        Action::ToggleCheckerBg,
        Action::TogglePanels,
        Action::ToggleMiniTimeline,
        Action::ZoomReset,
        Action::PanReset,
        Action::RotateReset,
        Action::FlipHorizontal,
        Action::FlipVertical,
        Action::ArtFlipH,
        Action::ArtFlipV,
        Action::ArtRotateCw,
        Action::ArtRotateCcw,
        Action::ArtRotate180,
        Action::CellCut,
        Action::CellCopy,
        Action::CellPaste,
        Action::SelectionCut,
        Action::SelectionCopy,
        Action::SelectionPaste,
        Action::SelectionDelete,
        Action::SelectionDeselect,
        Action::SelectAll,
        Action::SelectInvert,
        Action::SelectionFill,
        Action::SelModeAdd,
        Action::SelModeSubtract,
        Action::SelModeIntersect,
        Action::TrackClear,
        Action::TrackCloseGap,
        Action::SaveProject,
        Action::SaveProjectAs,
        Action::OpenProject,
        Action::CanvasZoom,
        Action::CanvasPan,
        Action::CanvasRotate,
        Action::HoldToolAdjust,
        Action::HoldScrub,
        Action::HoldFlip,
        Action::HoldSwatchPie,
        Action::LayerTransformToggle,
        Action::TransformKeyAdd,
        Action::TransformKeyDelete,
        Action::CameraEditToggle,
        Action::CameraKeyAdd,
        Action::CameraKeyDelete,
        Action::CameraLookThrough,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::ToolPencil => "Tool: Pencil",
            Action::ToolInk => "Tool: Ink",
            Action::ToolEraser => "Tool: Eraser",
            Action::ToolFill => "Tool: Fill",
            Action::ToolColorPicker => "Tool: Color picker (retired)",
            Action::PickScreenColor => "Tool: Color picker",
            Action::ToolShape => "Tool: Shape",
            Action::ToolTracker => "Tool: Tracker",
            Action::ToolLasso => "Tool: Lasso select",
            Action::ToolPerspective => "Tool: Perspective grid",
            Action::ToolLiquify => "Tool: Liquify",
            Action::TogglePerspectiveGrid => "Show perspective grids",
            Action::TogglePerspectiveSnap => "Snap strokes to perspective grid",
            Action::PerspectiveNextGrid => "Next perspective grid",
            Action::PlayPause => "Play / Pause",
            Action::FramePrev => "Previous frame",
            Action::FrameNext => "Next frame",
            Action::KeyJumpPrev => "Previous drawing key",
            Action::KeyJumpNext => "Next drawing key",
            Action::FrameAdd => "Add frame (hold)",
            Action::FrameDuplicate => "Duplicate frame",
            Action::FrameDelete => "Delete frame",
            Action::OnionToggle => "Toggle onion skin",
            Action::LayerAdd => "Add layer",
            Action::LayerDelete => "Delete layer",
            Action::LayerToggleVisible => "Toggle layer visibility",
            Action::LayerLast => "Go to last selected layer",
            Action::LayerSelectAbove => "Select layer above",
            Action::LayerSelectBelow => "Select layer below",
            Action::LayerSelectTop => "Select top layer",
            Action::LayerSelectBottom => "Select bottom layer",
            Action::LayerMoveUp => "Move layer up",
            Action::LayerMoveDown => "Move layer down",
            Action::LayerPick1 => "Select layer 1 (from top)",
            Action::LayerPick2 => "Select layer 2 (from top)",
            Action::LayerPick3 => "Select layer 3 (from top)",
            Action::LayerPick4 => "Select layer 4 (from top)",
            Action::LayerPick5 => "Select layer 5 (from top)",
            Action::LayerPick6 => "Select layer 6 (from top)",
            Action::LayerPick7 => "Select layer 7 (from top)",
            Action::LayerPick8 => "Select layer 8 (from top)",
            Action::LayerPick9 => "Select layer 9 (from top)",
            Action::FadeOthersToggle => "Fade other layers",
            Action::LayerClipToggle => "Clip layer to the one below",
            Action::LayerAlphaLockToggle => "Lock layer alpha",
            Action::KeyBlank => "Insert blank key",
            Action::KeyCopy => "Insert duplicate key",
            Action::Hold => "Hold (delete key)",
            Action::SizeDown => "Brush size down",
            Action::SizeUp => "Brush size up",
            Action::Undo => "Undo",
            Action::Redo => "Redo",
            Action::ClearCell => "Clear current cell",
            Action::PasteImage => "Paste image as background",
            Action::ToggleCheckerBg => "Toggle checker backdrop",
            Action::TogglePanels => "Toggle floating panels",
            Action::ToggleMiniTimeline => "Toggle mini timeline",
            Action::ZoomReset => "Reset zoom",
            Action::PanReset => "Reset pan",
            Action::RotateReset => "Reset rotation",
            Action::FlipHorizontal => "Flip view horizontally",
            Action::FlipVertical => "Flip view vertically",
            Action::ArtFlipH => "Flip art horizontally (selection / drawing)",
            Action::ArtFlipV => "Flip art vertically (selection / drawing)",
            Action::ArtRotateCw => "Rotate art 90° clockwise",
            Action::ArtRotateCcw => "Rotate art 90° anticlockwise",
            Action::ArtRotate180 => "Rotate art 180°",
            Action::CellCut => "Drawing: cut",
            Action::CellCopy => "Drawing: copy",
            Action::CellPaste => "Drawing: paste",
            Action::SelectionCut => "Selection: cut",
            Action::SelectionCopy => "Selection: copy",
            Action::SelectionPaste => "Selection: paste",
            Action::SelectionDelete => "Selection: delete",
            Action::SelectionDeselect => "Selection: deselect",
            Action::SelectAll => "Selection: select all",
            Action::SelectInvert => "Selection: invert",
            Action::SelectionFill => "Selection: fill with brush colour",
            Action::SelModeAdd => "Selection: add to (drag)",
            Action::SelModeSubtract => "Selection: remove from (drag)",
            Action::SelModeIntersect => "Selection: intersect with (drag)",
            Action::TrackClear => "Timeline: delete drawings (leave blank)",
            Action::TrackCloseGap => "Timeline: delete drawings (close gap)",
            Action::SaveProject => "Save project",
            Action::SaveProjectAs => "Save project as…",
            Action::OpenProject => "Open project",
            Action::CanvasZoom => "Canvas: zoom (drag)",
            Action::CanvasPan => "Canvas: pan (drag)",
            Action::CanvasRotate => "Canvas: rotate (drag)",
            Action::HoldToolAdjust => "Canvas: brush size / opacity (drag)",
            Action::HoldScrub => "Canvas: scrub frames (drag)",
            Action::HoldFlip => "Canvas: flip drawings (drag)",
            Action::HoldSwatchPie => "Canvas: swatch pie (drag)",
            Action::LayerTransformToggle => "Layer transform mode",
            Action::TransformKeyAdd => "Add transform key",
            Action::TransformKeyDelete => "Delete transform key",
            Action::CameraEditToggle => "Camera edit mode",
            Action::CameraKeyAdd => "Add camera key",
            Action::CameraKeyDelete => "Delete camera key",
            Action::CameraLookThrough => "Look through camera",
        }
    }

    /// `Some(n)` for the "select layer n" actions: n counts rows down from
    /// the top of the Layers panel, starting at 1.
    pub fn layer_pick(self) -> Option<usize> {
        Some(match self {
            Action::LayerPick1 => 1,
            Action::LayerPick2 => 2,
            Action::LayerPick3 => 3,
            Action::LayerPick4 => 4,
            Action::LayerPick5 => 5,
            Action::LayerPick6 => 6,
            Action::LayerPick7 => 7,
            Action::LayerPick8 => 8,
            Action::LayerPick9 => 9,
            _ => return None,
        })
    }

    /// Held while a canvas drag starts, rather than pressed: the drag
    /// gestures and the selection modes. Its combo can be a key, bare
    /// modifiers or both, and it never fires as a press.
    pub fn is_hold(self) -> bool {
        self.hold_drag().is_some()
            || matches!(
                self,
                Action::SelModeAdd | Action::SelModeSubtract | Action::SelModeIntersect
            )
    }

    /// The drag a hold action starts. `None` for everything else, the
    /// selection modes included — those change what a Lasso drag does
    /// instead of replacing it.
    pub fn hold_drag(self) -> Option<ButtonDrag> {
        Some(match self {
            Action::CanvasZoom => ButtonDrag::Zoom,
            Action::CanvasPan => ButtonDrag::Pan,
            Action::CanvasRotate => ButtonDrag::Rotate,
            Action::HoldToolAdjust => ButtonDrag::ToolAdjust,
            Action::HoldScrub => ButtonDrag::Scrub,
            Action::HoldFlip => ButtonDrag::Flip,
            Action::HoldSwatchPie => ButtonDrag::SwatchPie,
            _ => return None,
        })
    }
}

/// The drag gestures in the order they win when several are held at once
/// with equally specific combos.
pub const HOLD_DRAGS: [Action; 7] = [
    Action::CanvasZoom,
    Action::CanvasRotate,
    Action::CanvasPan,
    Action::HoldToolAdjust,
    Action::HoldScrub,
    Action::HoldFlip,
    Action::HoldSwatchPie,
];

#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub struct KeyCombo {
    /// `None` = modifier-only combo (canvas nav gestures held during a drag).
    pub key: Option<egui::Key>,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyCombo {
    pub fn plain(key: egui::Key) -> Self {
        Self {
            key: Some(key),
            ctrl: false,
            shift: false,
            alt: false,
        }
    }
    pub fn ctrl(key: egui::Key) -> Self {
        Self {
            key: Some(key),
            ctrl: true,
            shift: false,
            alt: false,
        }
    }
    pub fn shift(key: egui::Key) -> Self {
        Self {
            key: Some(key),
            ctrl: false,
            shift: true,
            alt: false,
        }
    }
    pub fn alt(key: egui::Key) -> Self {
        Self {
            key: Some(key),
            ctrl: false,
            shift: false,
            alt: true,
        }
    }

    pub fn ctrl_shift(key: egui::Key) -> Self {
        Self {
            key: Some(key),
            ctrl: true,
            shift: true,
            alt: false,
        }
    }

    pub fn ctrl_alt(key: egui::Key) -> Self {
        Self {
            key: Some(key),
            ctrl: true,
            shift: false,
            alt: true,
        }
    }

    pub fn shift_alt(key: egui::Key) -> Self {
        Self {
            key: Some(key),
            ctrl: false,
            shift: true,
            alt: true,
        }
    }

    /// A combo with no key — only modifiers. Used for canvas drag gestures.
    pub fn modifier_only(ctrl: bool, shift: bool, alt: bool) -> Self {
        Self {
            key: None,
            ctrl,
            shift,
            alt,
        }
    }

    pub fn display(self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        match self.key {
            Some(k) => s.push_str(k.name()),
            None => {
                // Modifier-only: drop the trailing '+'.
                if s.ends_with('+') {
                    s.pop();
                }
            }
        }
        s
    }

    /// Matches if the given egui input state had this combo pressed this
    /// frame (with matching modifier state, ignoring caps/super). Modifier-only
    /// combos never match here — they are drag gestures, see `held`.
    pub fn matches(self, i: &egui::InputState) -> bool {
        let Some(key) = self.key else {
            return false;
        };
        let m = i.modifiers;
        if m.ctrl != self.ctrl || m.shift != self.shift || m.alt != self.alt {
            return false;
        }
        i.key_pressed(key)
    }

    /// True while this combo is held: its key down (if it has one) with
    /// exactly its modifiers. A combo with no key needs at least one modifier.
    /// Used to pick a drag gesture as the drag starts.
    pub fn held(self, i: &egui::InputState) -> bool {
        let m = i.modifiers;
        let mods = m.ctrl == self.ctrl && m.shift == self.shift && m.alt == self.alt;
        match self.key {
            Some(k) => mods && i.key_down(k),
            None => mods && (self.ctrl || self.shift || self.alt),
        }
    }

    /// True while every part of this combo is still down. Looser than
    /// [`KeyCombo::held`]: a modifier pressed on top doesn't count as letting
    /// go, so a gesture started on Space survives Shift going down mid-drag.
    pub fn still_held(self, i: &egui::InputState) -> bool {
        let m = i.modifiers;
        self.key.map_or(true, |k| i.key_down(k))
            && (!self.ctrl || m.ctrl)
            && (!self.shift || m.shift)
            && (!self.alt || m.alt)
    }
}

/// Capture for a hold binding in Settings, which — unlike a press binding —
/// may be bare modifiers. egui reports no key for Shift, Ctrl or Alt on their
/// own, so a bare-modifier combo is the set of modifiers held at their peak,
/// taken when the last of them comes back up.
#[derive(Clone, Copy, Debug, Default)]
pub struct HoldCapture {
    /// Everything held when capture began has been let go. Until then,
    /// modifiers still down from whatever started the capture are ignored.
    armed: bool,
    peak: (bool, bool, bool),
}

impl HoldCapture {
    /// One frame of capture: the modifiers held now, and the key pressed this
    /// frame if any. Returns the captured combo once there is one.
    pub fn step(&mut self, mods: egui::Modifiers, pressed: Option<egui::Key>) -> Option<KeyCombo> {
        if let Some(key) = pressed {
            *self = Self::default();
            return Some(KeyCombo {
                key: Some(key),
                ctrl: mods.ctrl,
                shift: mods.shift,
                alt: mods.alt,
            });
        }
        let now = (mods.ctrl, mods.shift, mods.alt);
        let any = now.0 || now.1 || now.2;
        if !self.armed {
            self.armed = !any;
            return None;
        }
        if any {
            self.peak = (self.peak.0 || now.0, self.peak.1 || now.1, self.peak.2 || now.2);
            return None;
        }
        let (c, s, a) = std::mem::take(&mut self.peak);
        (c || s || a).then(|| KeyCombo::modifier_only(c, s, a))
    }
}

// --- (de)serialise as a single string like "Ctrl+Shift+Z" or "B" ---

impl Serialize for KeyCombo {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.display())
    }
}

impl<'de> Deserialize<'de> for KeyCombo {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s: String = Deserialize::deserialize(de)?;
        parse_combo(&s).ok_or_else(|| serde::de::Error::custom(format!("bad key combo: {s}")))
    }
}

fn parse_combo(s: &str) -> Option<KeyCombo> {
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut key_name: Option<&str> = None;
    for part in s.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmd" | "command" | "super" | "meta" => ctrl = true,
            "shift" => shift = true,
            "alt" | "option" => alt = true,
            _ => key_name = Some(part),
        }
    }
    let key = match key_name {
        Some(name) => Some(egui::Key::from_name(name)?),
        None => {
            // Modifier-only combo is valid only if at least one modifier is set.
            if !(ctrl || shift || alt) {
                return None;
            }
            None
        }
    };
    Some(KeyCombo {
        key,
        ctrl,
        shift,
        alt,
    })
}

// --- clashes, as the Settings list flags them ---

/// Something else on a binding's combo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clash {
    /// Another action on the same combo. On a press both fire; held for a
    /// drag, only one of them can ever start it.
    Action(Action),
    /// A key the app reads outside this map; the text says what it does.
    Builtin(&'static str),
    /// Another action on the same combo by design, kept apart by the rule in
    /// the text. Shown as a note, not a conflict.
    Shared(Action, &'static str),
}

impl Clash {
    /// A real problem, as opposed to a note.
    pub fn is_conflict(self) -> bool {
        !matches!(self, Clash::Shared(..))
    }
}

/// Why `a` and `b` may share a combo, when they may. The dispatch loop in
/// `AppState::handle_keys` lets exactly one of these through.
fn shared_on_purpose(a: Action, b: Action) -> Option<&'static str> {
    match (a.min(b), a.max(b)) {
        (Action::SelectionDelete, Action::TrackClear) => Some(
            "drawings selected in the timeline take the press; otherwise the selection does",
        ),
        _ => None,
    }
}

/// What a combo collides with among the keys `AppState::handle_keys` and
/// [`ShortcutMap::poll_actions`] read directly. Those read the key whatever
/// the modifiers, except the Redo alias, which wants Ctrl and Shift.
///
/// Tab and ` are not here: what gets through a focused widget on them is
/// whatever the map binds there ([`FOCUS_PASS_KEYS`]), so binding them is no
/// different from binding any other key.
fn builtin_clash(action: Action, c: KeyCombo) -> Option<&'static str> {
    use egui::Key as K;
    match c.key? {
        K::Z if c.ctrl && c.shift && action != Action::Redo => {
            Some("Ctrl+Shift+Z always redoes as well")
        }
        K::ArrowUp | K::ArrowDown | K::ArrowLeft | K::ArrowRight => {
            Some("The arrow keys also nudge the selection while there is one")
        }
        _ => None,
    }
}

/// Whether this frame has a key-down for `combo`'s key that isn't an
/// auto-repeat.
fn fresh_press(i: &egui::InputState, combo: &KeyCombo) -> bool {
    i.events.iter().any(|e| {
        matches!(e, egui::Event::Key { key, pressed: true, repeat: false, .. } if Some(*key) == combo.key)
    })
}

/// Keys that still reach the map while a widget has keyboard focus. Tab
/// would otherwise only walk focus from field to field, which is how panels
/// hidden and shown again could leave every shortcut dead; ` is a character
/// no name or number field needs. Whatever the map binds to them fires,
/// matched exactly like any other press.
pub const FOCUS_PASS_KEYS: [egui::Key; 2] = [egui::Key::Tab, egui::Key::Backtick];

/// Every key Settings will take as a binding.
pub const CAPTURE_KEYS: [egui::Key; 65] = {
    use egui::Key::*;
    [
        A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, Num0, Num1,
        Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, OpenBracket, CloseBracket, Semicolon, Comma,
        Period, Slash, Backslash, Minus, Equals, Space, Tab, Backspace, Backtick, F1, F2, F3, F4,
        F5, F6, F7, F8, F9, F10, F11, F12, ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
    ]
};

// --- the map itself ---

/// Every action's combo, saved as `action = "Combo"` lines.
///
/// An action the user unbound is saved as `action = ""` and kept in
/// `unbound`, so loading doesn't hand it its default back. An action absent
/// from both maps is unbound too, but takes its default on the next load.
#[derive(Clone, Debug)]
pub struct ShortcutMap {
    pub bindings: BTreeMap<Action, KeyCombo>,
    pub unbound: BTreeSet<Action>,
}

impl Serialize for ShortcutMap {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let mut out: BTreeMap<Action, String> =
            self.bindings.iter().map(|(&a, c)| (a, c.display())).collect();
        for &a in &self.unbound {
            out.insert(a, String::new());
        }
        out.serialize(ser)
    }
}

impl<'de> Deserialize<'de> for ShortcutMap {
    /// Entry by entry, so one line naming an action that no longer exists, or
    /// a combo that doesn't parse, costs that line rather than every binding.
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        use serde::de::IntoDeserializer;
        let raw: BTreeMap<String, String> = BTreeMap::deserialize(de)?;
        let mut map = ShortcutMap {
            bindings: BTreeMap::new(),
            unbound: BTreeSet::new(),
        };
        for (name, combo) in raw {
            let action: Result<Action, serde::de::value::Error> =
                Action::deserialize(name.as_str().into_deserializer());
            let Ok(action) = action else {
                log::warn!("shortcuts: unknown action {name:?}, skipped");
                continue;
            };
            let combo = combo.trim();
            if combo.is_empty() || combo.eq_ignore_ascii_case("none") {
                map.unbound.insert(action);
            } else if let Some(c) = parse_combo(combo) {
                map.bindings.insert(action, c);
            } else {
                log::warn!("shortcuts: bad combo {combo:?} for {name}, using the default");
            }
        }
        Ok(map)
    }
}

impl Default for ShortcutMap {
    fn default() -> Self {
        use egui::Key as K;
        let mut b = BTreeMap::new();
        // Left-cluster tools.
        b.insert(Action::ToolPencil, KeyCombo::plain(K::Q));
        b.insert(Action::ToolInk, KeyCombo::plain(K::W));
        b.insert(Action::ToolEraser, KeyCombo::plain(K::E));
        b.insert(Action::ToolFill, KeyCombo::plain(K::R));
        // One picker now: it samples the canvas and anything behind the window.
        b.insert(Action::PickScreenColor, KeyCombo::plain(K::C));
        b.insert(Action::ToolShape, KeyCombo::plain(K::G));
        b.insert(Action::ToolTracker, KeyCombo::plain(K::X));
        // Y is the only free key left in the left cluster (see module docs).
        b.insert(Action::ToolLasso, KeyCombo::plain(K::Y));
        // Perspective grid: the G family, beside the Shape tool it guides.
        b.insert(Action::ToolPerspective, KeyCombo::shift(K::G));
        b.insert(Action::TogglePerspectiveGrid, KeyCombo::ctrl(K::G));
        b.insert(Action::TogglePerspectiveSnap, KeyCombo::ctrl_shift(K::G));
        b.insert(Action::PerspectiveNextGrid, KeyCombo::alt(K::G));
        // Frame navigation: A / S.
        b.insert(Action::FramePrev, KeyCombo::plain(K::A));
        b.insert(Action::FrameNext, KeyCombo::plain(K::S));
        // Jump drawing-to-drawing, skipping holds: the shifted siblings of the
        // plain frame step above.
        b.insert(Action::KeyJumpPrev, KeyCombo::shift(K::A));
        b.insert(Action::KeyJumpNext, KeyCombo::shift(K::S));
        // Frame mutate: D / F / Shift+F.
        b.insert(Action::FrameAdd, KeyCombo::plain(K::D));
        b.insert(Action::FrameDuplicate, KeyCombo::plain(K::F));
        b.insert(Action::FrameDelete, KeyCombo::shift(K::F));
        // Playback / onion.
        b.insert(Action::PlayPause, KeyCombo::plain(K::Space));
        b.insert(Action::OnionToggle, KeyCombo::plain(K::O));
        // Layers.
        b.insert(Action::LayerAdd, KeyCombo::plain(K::T));
        b.insert(Action::LayerDelete, KeyCombo::shift(K::T));
        b.insert(Action::LayerToggleVisible, KeyCombo::plain(K::V));
        b.insert(Action::LayerLast, KeyCombo::plain(K::Z));
        // Shifted sibling of the layer jump: push every other layer back.
        b.insert(Action::FadeOthersToggle, KeyCombo::shift(K::Z));
        // Walk the stack: W above S, the vertical pair beside the A / S frame
        // step. Alt keeps them off the tools; Shift goes all the way, Ctrl
        // carries the layer along.
        b.insert(Action::LayerSelectAbove, KeyCombo::alt(K::W));
        b.insert(Action::LayerSelectBelow, KeyCombo::alt(K::S));
        b.insert(Action::LayerSelectTop, KeyCombo::shift_alt(K::W));
        b.insert(Action::LayerSelectBottom, KeyCombo::shift_alt(K::S));
        b.insert(Action::LayerMoveUp, KeyCombo::ctrl_alt(K::W));
        b.insert(Action::LayerMoveDown, KeyCombo::ctrl_alt(K::S));
        // Alt+digit picks a row; the plain digits stay the x-sheet keys.
        let digits = [
            K::Num1,
            K::Num2,
            K::Num3,
            K::Num4,
            K::Num5,
            K::Num6,
            K::Num7,
            K::Num8,
            K::Num9,
        ];
        let picks = [
            Action::LayerPick1,
            Action::LayerPick2,
            Action::LayerPick3,
            Action::LayerPick4,
            Action::LayerPick5,
            Action::LayerPick6,
            Action::LayerPick7,
            Action::LayerPick8,
            Action::LayerPick9,
        ];
        for (a, k) in picks.into_iter().zip(digits) {
            b.insert(a, KeyCombo::alt(k));
        }
        // Photoshop's keys for both.
        b.insert(
            Action::LayerClipToggle,
            KeyCombo {
                key: Some(K::G),
                ctrl: true,
                shift: false,
                alt: true,
            },
        );
        b.insert(Action::LayerAlphaLockToggle, KeyCombo::plain(K::Slash));
        // X-sheet keys: 1 / 2 / 3 (numeric row, left side).
        b.insert(Action::KeyBlank, KeyCombo::plain(K::Num1));
        b.insert(Action::KeyCopy, KeyCombo::plain(K::Num2));
        b.insert(Action::Hold, KeyCombo::plain(K::Num3));
        // Brush size — bracket keys (Photoshop / Krita convention).
        b.insert(Action::SizeDown, KeyCombo::plain(K::OpenBracket));
        b.insert(Action::SizeUp, KeyCombo::plain(K::CloseBracket));
        // History.
        b.insert(Action::Undo, KeyCombo::ctrl(K::Z));
        b.insert(Action::Redo, KeyCombo::ctrl(K::Y));
        // Cell clear.
        b.insert(Action::ClearCell, KeyCombo::plain(K::Backspace));
        // Paste clipboard image as a background layer. Moved off Ctrl+V when
        // the pixel selection took the standard clipboard keys; `load` migrates
        // an existing binding (see below).
        b.insert(Action::PasteImage, KeyCombo::ctrl_shift(K::V));
        // Backdrop toggle.
        b.insert(Action::ToggleCheckerBg, KeyCombo::plain(K::Backtick));
        // Hide / show all floating panels.
        b.insert(Action::TogglePanels, KeyCombo::plain(K::Tab));
        // Mini timeline bar (shown when panels are hidden).
        b.insert(Action::ToggleMiniTimeline, KeyCombo::plain(K::M));
        // Canvas view resets.
        b.insert(Action::ZoomReset, KeyCombo::plain(K::Num0));
        b.insert(Action::PanReset, KeyCombo::plain(K::H));
        b.insert(Action::RotateReset, KeyCombo::plain(K::J));
        // Mirror the view — a drawing check, never applied to the document.
        b.insert(Action::FlipHorizontal, KeyCombo::plain(K::F));
        b.insert(Action::FlipVertical, KeyCombo::shift(K::F));
        // Pixel selection takes the standard clipboard keys.
        b.insert(Action::SelectionCut, KeyCombo::ctrl(K::X));
        b.insert(Action::SelectionCopy, KeyCombo::ctrl(K::C));
        b.insert(Action::SelectionPaste, KeyCombo::ctrl(K::V));
        b.insert(Action::SelectionDelete, KeyCombo::plain(K::Delete));
        b.insert(Action::SelectionDeselect, KeyCombo::ctrl(K::D));
        b.insert(Action::SelectAll, KeyCombo::ctrl(K::A));
        b.insert(Action::SelectInvert, KeyCombo::ctrl_shift(K::I));
        // Shift+ the Clear cell key: `matches` compares modifiers exactly, so
        // this never also clears.
        b.insert(Action::SelectionFill, KeyCombo::shift(K::Backspace));
        // Selection modes, held as a selection drag starts. Shift alone pans
        // and Alt alone rotates, so these take the combos nothing else uses;
        // Shift+Alt intersecting is Krita's own.
        b.insert(Action::SelModeAdd, KeyCombo::modifier_only(true, true, false));
        b.insert(Action::SelModeSubtract, KeyCombo::modifier_only(true, false, true));
        b.insert(Action::SelModeIntersect, KeyCombo::modifier_only(false, true, true));
        // Delete shares its key with the pixel selection's erase; a live pixel
        // selection claims the press (see the dispatch loop in `app`).
        b.insert(Action::TrackClear, KeyCombo::plain(K::Delete));
        b.insert(Action::TrackCloseGap, KeyCombo::ctrl(K::Delete));
        // The drawing clipboard moves a whole cell between frames or layers;
        // Alt keeps it clear of the pixel selection above.
        b.insert(Action::CellCut, KeyCombo::alt(K::X));
        b.insert(Action::CellCopy, KeyCombo::alt(K::C));
        b.insert(Action::CellPaste, KeyCombo::alt(K::V));
        // Project file. Save overwrites the current file once there is one;
        // Save As always prompts. `matches` compares modifiers exactly, so
        // Ctrl+Shift+S never also fires Save.
        b.insert(Action::SaveProject, KeyCombo::ctrl(K::S));
        b.insert(Action::SaveProjectAs, KeyCombo::ctrl_shift(K::S));
        b.insert(Action::OpenProject, KeyCombo::ctrl(K::O));
        // Canvas navigation gestures (held during a canvas drag).
        b.insert(Action::CanvasZoom, KeyCombo::modifier_only(true, false, false));
        b.insert(Action::CanvasPan, KeyCombo::modifier_only(false, true, false));
        b.insert(
            Action::CanvasRotate,
            KeyCombo::modifier_only(false, false, true),
        );
        // Layer transform editing.
        b.insert(Action::LayerTransformToggle, KeyCombo::plain(K::B));
        b.insert(Action::TransformKeyAdd, KeyCombo::plain(K::K));
        b.insert(Action::TransformKeyDelete, KeyCombo::shift(K::K));
        // Camera — shifted/ctrl'd siblings of the layer-transform binds above,
        // since the left cluster has no plain keys left.
        b.insert(Action::CameraEditToggle, KeyCombo::shift(K::B));
        b.insert(Action::CameraKeyAdd, KeyCombo::ctrl(K::K));
        b.insert(Action::CameraKeyDelete, KeyCombo::ctrl_shift(K::K));
        b.insert(Action::CameraLookThrough, KeyCombo::shift(K::V));
        Self {
            bindings: b,
            unbound: BTreeSet::new(),
        }
    }
}

impl ShortcutMap {
    pub fn get(&self, action: Action) -> Option<KeyCombo> {
        self.bindings.get(&action).copied()
    }
    pub fn set(&mut self, action: Action, combo: KeyCombo) {
        self.bindings.insert(action, combo);
        self.unbound.remove(&action);
    }

    /// Unbind `action`, for good: it stays unbound across a save and load
    /// rather than taking its default back.
    pub fn clear(&mut self, action: Action) {
        self.bindings.remove(&action);
        self.unbound.insert(action);
    }

    /// The drag gesture held right now, with the combo that holds it. A combo
    /// with a key beats one of bare modifiers (Ctrl+Space over Ctrl); between
    /// equals, [`HOLD_DRAGS`] order decides.
    pub fn held_drag(&self, i: &egui::InputState) -> Option<(Action, KeyCombo)> {
        let held = HOLD_DRAGS
            .iter()
            .filter_map(|&a| Some((a, self.get(a)?)))
            .filter(|(_, c)| c.held(i));
        let mut best: Option<(Action, KeyCombo)> = None;
        for (a, c) in held {
            if best.map_or(true, |(_, b)| b.key.is_none() && c.key.is_some()) {
                best = Some((a, c));
            }
        }
        best
    }

    /// Keys some hold binding uses. A press on one fires only on its first
    /// key-down, never on the auto-repeat that holding it sends.
    pub(crate) fn hold_keys(&self) -> Vec<egui::Key> {
        self.bindings
            .iter()
            .filter(|(a, _)| a.is_hold())
            .filter_map(|(_, c)| c.key)
            .collect()
    }

    /// Every binding that shares its combo with something, keyed by the
    /// action whose row shows it. Clean bindings are absent.
    pub fn clashes(&self) -> BTreeMap<Action, Vec<Clash>> {
        let mut out: BTreeMap<Action, Vec<Clash>> = BTreeMap::new();
        for (&a, &ca) in &self.bindings {
            for (&b, &cb) in &self.bindings {
                if a != b && ca == cb {
                    let c = match shared_on_purpose(a, b) {
                        Some(why) => Clash::Shared(b, why),
                        None => Clash::Action(b),
                    };
                    out.entry(a).or_default().push(c);
                }
            }
            if let Some(why) = builtin_clash(a, ca) {
                out.entry(a).or_default().push(Clash::Builtin(why));
            }
        }
        out
    }

    /// How many bindings have a real conflict (notes don't count).
    pub fn conflict_count(&self) -> usize {
        self.clashes()
            .values()
            .filter(|v| v.iter().any(|c| c.is_conflict()))
            .count()
    }

    /// Returns every action that has fired this frame. Also Ctrl+Shift+Z is
    /// treated as an alias for Redo because that's the muscle-memory shortcut
    /// on most apps.
    pub fn poll_actions(&self, ctx: &egui::Context) -> Vec<Action> {
        let hold_keys = self.hold_keys();
        ctx.input(|i| {
            let mut out: Vec<Action> = Vec::new();
            for (&action, combo) in &self.bindings {
                // Held, not pressed: the drag gestures and selection modes.
                if action.is_hold() {
                    continue;
                }
                if !combo.matches(i) {
                    continue;
                }
                // With Space held for a pan, pressing Ctrl turns Space's
                // auto-repeat into Ctrl+Space, which would fire whatever is
                // there on every repeat. Only a real key-down counts.
                if combo.key.is_some_and(|k| hold_keys.contains(&k)) && !fresh_press(i, combo) {
                    continue;
                }
                out.push(action);
            }
            // Ctrl+Shift+Z alias for Redo, regardless of user map.
            if i.modifiers.ctrl
                && i.modifiers.shift
                && i.key_pressed(egui::Key::Z)
                && !out.contains(&Action::Redo)
            {
                out.push(Action::Redo);
            }
            out
        })
    }

    /// [`ShortcutMap::poll_actions`] for a frame where a widget has keyboard
    /// focus: only what is bound to one of [`FOCUS_PASS_KEYS`].
    pub fn poll_focused(&self, ctx: &egui::Context) -> Vec<Action> {
        ctx.input(|i| {
            self.bindings
                .iter()
                .filter(|(_, c)| c.key.is_some_and(|k| FOCUS_PASS_KEYS.contains(&k)))
                .filter(|(_, c)| c.matches(i))
                .map(|(&a, _)| a)
                .collect()
        })
    }
}

/// This app's config folder. Everything the app persists across runs lives
/// here — shortcuts, and eframe's UI-layout state (see `main.rs`).
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("animator-app"))
}

/// Path to the persisted shortcuts file.
fn config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("shortcuts.toml"))
}

pub fn load() -> ShortcutMap {
    let Some(path) = config_path() else {
        return ShortcutMap::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ShortcutMap::default();
    };
    let map = load_from_str(&text);
    log::info!("Loaded shortcuts from {}", path.display());
    map
}

/// A saved shortcuts file, migrated, with defaults for whatever it doesn't
/// mention.
pub fn load_from_str(text: &str) -> ShortcutMap {
    match toml::from_str::<ShortcutMap>(text) {
        Ok(mut map) => {
            // Migration: the in-canvas eyedropper was folded into the screen
            // picker. Drop the retired binding, and hand its key to the picker
            // that replaced it unless the user had already rebound that one.
            if let Some(old) = map.bindings.remove(&Action::ToolColorPicker) {
                let picker = map.bindings.get(&Action::PickScreenColor).copied();
                if picker.is_none() || picker == Some(KeyCombo::shift(egui::Key::C)) {
                    map.bindings.insert(Action::PickScreenColor, old);
                }
            }
            // Migration: Ctrl+V now pastes a pixel selection. Move an existing
            // PasteImage binding off it so the two don't both fire, unless the
            // user had already rebound PasteImage themselves.
            if map.bindings.get(&Action::PasteImage) == Some(&KeyCombo::ctrl(egui::Key::V)) {
                map.bindings
                    .insert(Action::PasteImage, KeyCombo::ctrl_shift(egui::Key::V));
            }
            // Merge in any missing defaults so newly added actions are bound,
            // except where the user unbound one on purpose.
            let mut full = ShortcutMap::default();
            for (a, c) in map.bindings {
                full.set(a, c);
            }
            for a in map.unbound {
                full.clear(a);
            }
            full
        }
        Err(e) => {
            log::warn!("shortcuts.toml parse failed ({e}); using defaults");
            ShortcutMap::default()
        }
    }
}

pub fn save(map: &ShortcutMap) {
    let Some(path) = config_path() else {
        log::warn!("no config dir; shortcuts not saved");
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match toml::to_string_pretty(map) {
        Ok(s) => {
            if let Err(e) = std::fs::write(&path, s) {
                log::warn!("shortcut save failed: {e}");
            } else {
                log::info!("Saved shortcuts → {}", path.display());
            }
        }
        Err(e) => log::warn!("shortcut serialize failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The selection's bindings must not collide with anything else's — a
    /// shared press fires both actions, and a shared held combo makes a
    /// selection drag also pan or zoom.
    #[test]
    fn selection_defaults_clash_with_nothing() {
        let map = ShortcutMap::default();
        let ours = [
            Action::SelectAll,
            Action::SelectInvert,
            Action::SelectionFill,
            Action::SelModeAdd,
            Action::SelModeSubtract,
            Action::SelModeIntersect,
        ];
        for a in ours {
            let c = map.get(a).expect("bound");
            for (&b, &other) in &map.bindings {
                if b != a {
                    assert_ne!(c, other, "{a:?} and {b:?} share {}", c.display());
                }
            }
        }
    }

    #[test]
    fn next_grid_is_alt_g_and_clashes_with_nothing() {
        let map = ShortcutMap::default();
        let c = map.get(Action::PerspectiveNextGrid).expect("bound");
        assert_eq!(c, KeyCombo::alt(egui::Key::G));
        for (&b, &other) in &map.bindings {
            if b != Action::PerspectiveNextGrid {
                assert_ne!(c, other, "shares {} with {b:?}", c.display());
            }
        }
    }

    #[test]
    fn layer_nav_defaults_clash_with_nothing() {
        let map = ShortcutMap::default();
        let ours = Action::ALL.iter().copied().filter(|&a| {
            a.layer_pick().is_some()
                || matches!(
                    a,
                    Action::LayerSelectAbove
                        | Action::LayerSelectBelow
                        | Action::LayerSelectTop
                        | Action::LayerSelectBottom
                        | Action::LayerMoveUp
                        | Action::LayerMoveDown
                )
        });
        let mut n = 0;
        for a in ours {
            n += 1;
            let c = map.get(a).expect("bound");
            for (&b, &other) in &map.bindings {
                if b != a {
                    assert_ne!(c, other, "{a:?} and {b:?} share {}", c.display());
                }
            }
        }
        assert_eq!(n, 15);
        assert_eq!(
            map.get(Action::LayerSelectAbove),
            Some(KeyCombo::alt(egui::Key::W))
        );
        assert_eq!(Action::LayerPick9.layer_pick(), Some(9));
    }

    /// The plain digits are x-sheet keys; Alt+digit must pick a layer
    /// without also inserting a key.
    #[test]
    fn alt_digit_picks_a_layer_and_nothing_else() {
        let map = ShortcutMap::default();
        let ctx = egui::Context::default();
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        let mut fired = Vec::new();
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Num1,
                    physical_key: Some(egui::Key::Num1),
                    pressed: true,
                    repeat: false,
                    modifiers: alt,
                }],
                modifiers: alt,
                ..Default::default()
            },
            |ctx| fired = map.poll_actions(ctx),
        );
        assert_eq!(fired, vec![Action::LayerPick1]);
    }

    /// No two defaults may share a combo, so a fresh install opens Settings
    /// with nothing flagged. Delete is shared on purpose and only noted.
    #[test]
    fn defaults_have_no_conflicts() {
        // Known clashes awaiting new default flip keys. Remove a pair here
        // when its fix lands; never add one.
        let known = [
            (Action::FrameDuplicate, Action::FlipHorizontal),
            (Action::FrameDelete, Action::FlipVertical),
        ];
        let is_known = |a: Action, b: Action| known.contains(&(a, b)) || known.contains(&(b, a));
        for (a, clashes) in ShortcutMap::default().clashes() {
            for c in clashes {
                match c {
                    Clash::Action(b) if is_known(a, b) => {}
                    Clash::Shared(..) => {}
                    other => panic!("{a:?} clashes: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn clashes_name_the_other_action_and_note_the_delete_pair() {
        let mut map = ShortcutMap::default();
        map.set(Action::FlipHorizontal, KeyCombo::plain(egui::Key::Num9));
        map.set(Action::FlipVertical, KeyCombo::plain(egui::Key::Num8));
        let clean = map.conflict_count();
        assert_eq!(clean, 0, "{:?}", map.clashes());

        // A press combo taken twice: both rows name each other.
        map.set(Action::ToolInk, KeyCombo::plain(egui::Key::Q));
        let c = map.clashes();
        assert_eq!(c[&Action::ToolInk], vec![Clash::Action(Action::ToolPencil)]);
        assert_eq!(c[&Action::ToolPencil], vec![Clash::Action(Action::ToolInk)]);
        // The Delete pair is a note on both rows, never a conflict.
        assert!(matches!(
            c[&Action::TrackClear][..],
            [Clash::Shared(Action::SelectionDelete, _)]
        ));
        assert!(matches!(
            c[&Action::SelectionDelete][..],
            [Clash::Shared(Action::TrackClear, _)]
        ));
        assert_eq!(map.conflict_count(), 2);

        // Two drag gestures on the same held modifiers.
        map.set(Action::SelModeAdd, KeyCombo::modifier_only(false, true, false));
        assert_eq!(
            map.clashes()[&Action::CanvasPan],
            vec![Clash::Action(Action::SelModeAdd)]
        );
    }

    #[test]
    fn keys_read_outside_the_map_are_flagged() {
        let mut map = ShortcutMap::default();
        map.bindings.clear();
        map.set(Action::FramePrev, KeyCombo::alt(egui::Key::ArrowLeft));
        map.set(Action::OnionToggle, KeyCombo::ctrl_shift(egui::Key::Z));
        map.set(Action::PlayPause, KeyCombo::plain(egui::Key::Tab));
        map.set(Action::Redo, KeyCombo::ctrl_shift(egui::Key::Z));
        map.set(Action::TogglePanels, KeyCombo::ctrl(egui::Key::Tab));
        map.set(Action::ToggleCheckerBg, KeyCombo::plain(egui::Key::Backtick));
        let c = map.clashes();
        assert!(matches!(c[&Action::FramePrev][..], [Clash::Builtin(_)]));
        // Tab and ` reach whatever is bound on them, focus or not.
        assert!(!c.contains_key(&Action::PlayPause));
        // Redo's alias: a clash for the other action, and the two share the
        // combo besides.
        assert!(c[&Action::OnionToggle].contains(&Clash::Action(Action::Redo)));
        assert!(c[&Action::OnionToggle].iter().any(|x| matches!(x, Clash::Builtin(_))));
        assert_eq!(c[&Action::Redo], vec![Clash::Action(Action::OnionToggle)]);
        assert!(!c.contains_key(&Action::TogglePanels));
        assert!(!c.contains_key(&Action::ToggleCheckerBg));
    }

    /// The reported map: the panels on M, the mini timeline on Shift+M and
    /// the last layer on Tab. Settings flagged Tab, because it was read as
    /// the panel toggle whenever a field had focus, whatever the map said.
    #[test]
    fn tab_and_backtick_are_free_to_bind() {
        let mut map = ShortcutMap::default();
        map.set(Action::TogglePanels, KeyCombo::plain(egui::Key::M));
        map.set(Action::ToggleMiniTimeline, KeyCombo::shift(egui::Key::M));
        map.set(Action::LayerLast, KeyCombo::plain(egui::Key::Tab));
        map.set(Action::OnionToggle, KeyCombo::plain(egui::Key::Backtick));
        map.set(Action::ToggleCheckerBg, KeyCombo::ctrl(egui::Key::Backtick));
        let c = map.clashes();
        for a in [
            Action::TogglePanels,
            Action::ToggleMiniTimeline,
            Action::LayerLast,
            Action::OnionToggle,
            Action::ToggleCheckerBg,
        ] {
            assert!(!c.contains_key(&a), "{a:?}: {:?}", c.get(&a));
        }
    }

    /// What `poll` returns for one frame with `key` pressed under `mods`.
    fn press(
        key: egui::Key,
        mods: egui::Modifiers,
        poll: impl FnOnce(&egui::Context) -> Vec<Action>,
    ) -> Vec<Action> {
        let ctx = egui::Context::default();
        let mut fired = Vec::new();
        let mut poll = Some(poll);
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers: mods,
                }],
                modifiers: mods,
                ..Default::default()
            },
            |ctx| {
                if let Some(p) = poll.take() {
                    fired = p(ctx);
                }
            },
        );
        fired
    }

    #[test]
    fn poll_focused_sees_only_the_pass_keys_exactly() {
        let mut map = ShortcutMap::default();
        map.set(Action::TogglePanels, KeyCombo::plain(egui::Key::M));
        map.set(Action::ToggleMiniTimeline, KeyCombo::shift(egui::Key::M));
        map.set(Action::LayerLast, KeyCombo::plain(egui::Key::Tab));
        let none = egui::Modifiers::default();
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            press(egui::Key::Tab, none, |c| map.poll_focused(c)),
            vec![Action::LayerLast]
        );
        assert!(press(egui::Key::Tab, shift, |c| map.poll_focused(c)).is_empty());
        assert!(press(egui::Key::M, none, |c| map.poll_focused(c)).is_empty());
        assert_eq!(
            press(egui::Key::Backtick, none, |c| map.poll_focused(c)),
            vec![Action::ToggleCheckerBg]
        );
        // With nothing focused, M is the panels as usual.
        assert_eq!(
            press(egui::Key::M, none, |c| map.poll_actions(c)),
            vec![Action::TogglePanels]
        );
    }

    /// Run one frame of `events` with `mods` held on `ctx`, and read the input.
    fn with_input<R>(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        mods: egui::Modifiers,
        f: impl FnOnce(&egui::InputState) -> R,
    ) -> R {
        let mut f = Some(f);
        let mut out = None;
        let _ = ctx.run(
            egui::RawInput {
                events,
                modifiers: mods,
                ..Default::default()
            },
            |ctx| {
                if let Some(f) = f.take() {
                    out = Some(ctx.input(f));
                }
            },
        );
        out.unwrap()
    }

    fn key(key: egui::Key, pressed: bool, repeat: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat,
            modifiers,
        }
    }

    const NONE: egui::Modifiers = egui::Modifiers::NONE;
    const CTRL: egui::Modifiers = egui::Modifiers {
        alt: false,
        ctrl: true,
        shift: false,
        mac_cmd: false,
        command: true,
    };
    const SHIFT: egui::Modifiers = egui::Modifiers {
        alt: false,
        ctrl: false,
        shift: true,
        mac_cmd: false,
        command: false,
    };

    #[test]
    fn an_unbound_binding_survives_a_save_over_its_default() {
        let mut map = ShortcutMap::default();
        map.clear(Action::ToolInk);
        let text = toml::to_string_pretty(&map).unwrap();
        assert!(text.contains("tool_ink = \"\""), "{text}");
        let back = load_from_str(&text);
        assert_eq!(back.get(Action::ToolInk), None);
        assert_eq!(back.get(Action::ToolPencil), Some(KeyCombo::plain(egui::Key::Q)));
        // Binding it again makes it ordinary.
        let mut back = back;
        back.set(Action::ToolInk, KeyCombo::plain(egui::Key::W));
        let again = load_from_str(&toml::to_string_pretty(&back).unwrap());
        assert_eq!(again.get(Action::ToolInk), Some(KeyCombo::plain(egui::Key::W)));
    }

    /// Files written before holds could be keys: the gestures keep their
    /// bare modifiers, and the new holds come up unbound.
    #[test]
    fn old_shortcut_files_still_load() {
        let map = load_from_str(
            r#"
            play_pause = "Ctrl+Space"
            layer_last = "Tab"
            toggle_panels = "M"
            canvas_zoom = "Ctrl"
            canvas_pan = "Shift"
            canvas_rotate = "Alt"
            sel_mode_intersect = "Shift+Alt"
            "#,
        );
        assert_eq!(map.get(Action::CanvasZoom), Some(KeyCombo::modifier_only(true, false, false)));
        assert_eq!(map.get(Action::CanvasPan), Some(KeyCombo::modifier_only(false, true, false)));
        assert_eq!(map.get(Action::PlayPause), Some(KeyCombo::ctrl(egui::Key::Space)));
        for a in [Action::HoldToolAdjust, Action::HoldScrub, Action::HoldFlip, Action::HoldSwatchPie] {
            assert_eq!(map.get(a), None, "{a:?} starts unbound");
        }
    }

    #[test]
    fn a_bad_entry_drops_only_itself() {
        let map = load_from_str(
            r#"
            tool_pencil = "P"
            tool_ink = "Ctrl+NotAKey"
            an_action_from_the_future = "Q"
            "#,
        );
        assert_eq!(map.get(Action::ToolPencil), Some(KeyCombo::plain(egui::Key::P)));
        assert_eq!(map.get(Action::ToolInk), Some(KeyCombo::plain(egui::Key::W)), "default kept");
    }

    #[test]
    fn a_key_hold_matches_only_while_down_with_exact_modifiers() {
        let ctx = egui::Context::default();
        let space = KeyCombo::plain(egui::Key::Space);
        let ctrl_space = KeyCombo::ctrl(egui::Key::Space);
        let read = |i: &egui::InputState| {
            (space.held(i), ctrl_space.held(i), space.still_held(i))
        };
        assert_eq!(with_input(&ctx, vec![], NONE, read), (false, false, false));
        let down = vec![key(egui::Key::Space, true, false, NONE)];
        assert_eq!(with_input(&ctx, down, NONE, read), (true, false, true));
        // Still down a frame later, with no new event.
        assert_eq!(with_input(&ctx, vec![], NONE, read), (true, false, true));
        // Ctrl on top: exactly Ctrl+Space now, and Space is still held.
        assert_eq!(with_input(&ctx, vec![], CTRL, read), (false, true, true));
        let up = vec![key(egui::Key::Space, false, false, NONE)];
        assert_eq!(with_input(&ctx, up, NONE, read), (false, false, false));
        // A bare-modifier hold needs exactly its modifiers.
        let shift = KeyCombo::modifier_only(false, true, false);
        assert!(with_input(&ctx, vec![], SHIFT, |i| shift.held(i)));
        assert!(!with_input(&ctx, vec![], NONE, |i| shift.held(i)));
    }

    #[test]
    fn hold_actions_never_fire_as_presses() {
        let mut map = ShortcutMap::default();
        map.bindings.clear();
        map.set(Action::HoldScrub, KeyCombo::plain(egui::Key::J));
        map.set(Action::CanvasPan, KeyCombo::plain(egui::Key::H));
        assert!(press(egui::Key::J, NONE, |c| map.poll_actions(c)).is_empty());
        assert!(press(egui::Key::H, NONE, |c| map.poll_actions(c)).is_empty());
    }

    /// Space held to pan, then Ctrl pressed: Space's auto-repeat now arrives
    /// as Ctrl+Space, which must not play and pause on every repeat.
    #[test]
    fn holding_a_hold_key_does_not_repeat_presses_on_it() {
        let mut map = ShortcutMap::default();
        map.set(Action::CanvasPan, KeyCombo::plain(egui::Key::Space));
        map.set(Action::PlayPause, KeyCombo::ctrl(egui::Key::Space));
        let ctx = egui::Context::default();
        // egui marks a repeat itself, from whether the key was already down.
        let poll = |ctx: &egui::Context, events, mods| {
            let mut fired = Vec::new();
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    modifiers: mods,
                    ..Default::default()
                },
                |ctx| fired = map.poll_actions(ctx),
            );
            fired
        };
        let space = |pressed, mods| key(egui::Key::Space, pressed, false, mods);
        assert!(poll(&ctx, vec![space(true, NONE)], NONE).is_empty(), "Space down: a hold");
        assert!(poll(&ctx, vec![space(true, CTRL)], CTRL).is_empty(), "its repeat under Ctrl");
        assert!(poll(&ctx, vec![space(false, CTRL)], CTRL).is_empty());
        assert_eq!(poll(&ctx, vec![space(true, CTRL)], CTRL), vec![Action::PlayPause]);
        // A key no hold uses still repeats as it always has.
        map.set(Action::FrameNext, KeyCombo::plain(egui::Key::S));
        let mut fired = Vec::new();
        let _ = ctx.run(
            egui::RawInput {
                events: vec![key(egui::Key::S, true, true, NONE)],
                ..Default::default()
            },
            |ctx| fired = map.poll_actions(ctx),
        );
        assert_eq!(fired, vec![Action::FrameNext]);
    }

    #[test]
    fn the_more_specific_hold_wins() {
        let mut map = ShortcutMap::default();
        map.set(Action::CanvasZoom, KeyCombo::modifier_only(true, false, false));
        map.set(Action::HoldScrub, KeyCombo::ctrl(egui::Key::Space));
        let ctx = egui::Context::default();
        let held = |ctx: &egui::Context, events, mods| {
            with_input(ctx, events, mods, |i| map.held_drag(i).map(|(a, _)| a))
        };
        assert_eq!(held(&ctx, vec![], CTRL), Some(Action::CanvasZoom));
        let down = vec![key(egui::Key::Space, true, false, CTRL)];
        assert_eq!(held(&ctx, down, CTRL), Some(Action::HoldScrub));
        assert_eq!(held(&ctx, vec![], NONE), None, "Space alone is bound to nothing");
    }

    #[test]
    fn hold_and_press_on_one_combo_clash() {
        let mut map = ShortcutMap::default();
        map.set(Action::CanvasPan, KeyCombo::plain(egui::Key::Space));
        map.set(Action::PlayPause, KeyCombo::plain(egui::Key::Space));
        let c = map.clashes();
        assert!(c[&Action::CanvasPan].contains(&Clash::Action(Action::PlayPause)));
        assert!(c[&Action::PlayPause].contains(&Clash::Action(Action::CanvasPan)));
        // Different modifiers: the repeat rule keeps them apart.
        map.set(Action::PlayPause, KeyCombo::ctrl(egui::Key::Space));
        let c = map.clashes();
        assert!(!c.contains_key(&Action::CanvasPan) && !c.contains_key(&Action::PlayPause));
    }

    #[test]
    fn capturing_shift_alone_binds_it_on_release() {
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        let shift_alt = egui::Modifiers {
            alt: true,
            shift: true,
            ..Default::default()
        };
        let mut c = HoldCapture::default();
        assert_eq!(c.step(NONE, None), None, "arms");
        assert_eq!(c.step(SHIFT, None), None);
        assert_eq!(c.step(NONE, None), Some(KeyCombo::modifier_only(false, true, false)));
        // Two at once, let go one at a time: the peak is what was held.
        let mut c = HoldCapture::default();
        c.step(NONE, None);
        assert_eq!(c.step(SHIFT, None), None);
        assert_eq!(c.step(shift_alt, None), None);
        assert_eq!(c.step(alt, None), None);
        assert_eq!(c.step(NONE, None), Some(KeyCombo::modifier_only(false, true, true)));
        // Ctrl already down when capture began isn't taken for the binding.
        let mut c = HoldCapture::default();
        assert_eq!(c.step(CTRL, None), None);
        assert_eq!(c.step(NONE, None), None);
        assert_eq!(c.step(NONE, None), None);
        // A key goes in with whatever is held.
        assert_eq!(c.step(CTRL, Some(egui::Key::Space)), Some(KeyCombo::ctrl(egui::Key::Space)));
    }

    #[test]
    fn selection_modes_are_holds_and_never_fire_as_presses() {
        let map = ShortcutMap::default();
        for a in [Action::SelModeAdd, Action::SelModeSubtract, Action::SelModeIntersect] {
            assert!(a.is_hold());
            assert!(map.get(a).is_some_and(|c| c.key.is_none()));
        }
        assert!(!Action::SelectAll.is_hold());
    }

    /// Shift+Backspace fills; it must not also clear the drawing it just
    /// filled, which a loose modifier match would do.
    #[test]
    fn fill_selection_does_not_also_clear() {
        let map = ShortcutMap::default();
        let ctx = egui::Context::default();
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        let mut fired = Vec::new();
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Backspace,
                    physical_key: Some(egui::Key::Backspace),
                    pressed: true,
                    repeat: false,
                    modifiers: shift,
                }],
                modifiers: shift,
                ..Default::default()
            },
            |ctx| fired = map.poll_actions(ctx),
        );
        assert_eq!(fired, vec![Action::SelectionFill]);
    }
}
