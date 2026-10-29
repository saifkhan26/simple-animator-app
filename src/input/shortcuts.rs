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
    /// Play / Pause, the pause staying on the frame it reached instead of
    /// going back to where playback started.
    PlayPauseHere,
    FramePrev,
    FrameNext,
    KeyJumpPrev,
    KeyJumpNext,
    FrameAdd,
    FrameDuplicate,
    FrameDelete,
    /// Insert the step size's worth of frames before the playhead, on every
    /// layer. Unbound by default.
    FrameInsertBefore,
    OnionToggle,
    LayerAdd,
    LayerDelete,
    LayerGroup,
    LayerUngroup,
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
    /// Pin the colour wheel (see `AppState::set_wheel_pinned`). Unbound.
    WheelPinToggle,
    LayerClipToggle,
    LayerAlphaLockToggle,
    /// Link the active layer's key jumps to the nearest drawing layer below
    /// it, or unlink when it already is. Unbound by default.
    LayerKeyLinkBelow,
    KeyBlank,
    KeyCopy,
    Hold,
    SizeDown,
    SizeUp,
    /// Swap to the brush size drawn at last, and back. Unbound by default.
    SizeSwap,
    Undo,
    Redo,
    ClearCell,
    PasteImage,
    /// Retired with the checker backdrop. Kept so a `shortcuts.toml` that
    /// binds it still reads; `parse` strips it. Not in [`Action::ALL`].
    ToggleCheckerBg,
    TogglePanels,
    ToggleMiniTimeline,
    /// Swap what Shift holds a drag to between the canvas's axes and the
    /// screen's. Unbound by default.
    ToggleLockAxes,
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
    // The Fill tool's lasso, the same way: held as its outline is drawn or
    // closed, on the selection modes' combos, which only the Lasso reads.
    FillLassoErase,
    FillLassoEnclose,
    FillPolygonSnap,
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
    CameraBoxToggle,
    CameraKeyAdd,
    CameraKeyDelete,
    CameraLookThrough,
    CameraKeyPrev,
    CameraKeyNext,
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
        Action::PlayPauseHere,
        Action::FramePrev,
        Action::FrameNext,
        Action::KeyJumpPrev,
        Action::KeyJumpNext,
        Action::FrameAdd,
        Action::FrameDuplicate,
        Action::FrameDelete,
        Action::FrameInsertBefore,
        Action::OnionToggle,
        Action::LayerAdd,
        Action::LayerDelete,
        Action::LayerGroup,
        Action::LayerUngroup,
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
        Action::WheelPinToggle,
        Action::LayerClipToggle,
        Action::LayerAlphaLockToggle,
        Action::LayerKeyLinkBelow,
        Action::KeyBlank,
        Action::KeyCopy,
        Action::Hold,
        Action::SizeDown,
        Action::SizeUp,
        Action::SizeSwap,
        Action::Undo,
        Action::Redo,
        Action::ClearCell,
        Action::PasteImage,
        Action::TogglePanels,
        Action::ToggleMiniTimeline,
        Action::ToggleLockAxes,
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
        Action::FillLassoErase,
        Action::FillLassoEnclose,
        Action::FillPolygonSnap,
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
        Action::CameraBoxToggle,
        Action::CameraKeyAdd,
        Action::CameraKeyDelete,
        Action::CameraLookThrough,
        Action::CameraKeyPrev,
        Action::CameraKeyNext,
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
            Action::PlayPause => "Play / Pause (back to the start frame)",
            Action::PlayPauseHere => "Play / Pause where it stops",
            Action::FramePrev => "Previous frame",
            Action::FrameNext => "Next frame",
            Action::KeyJumpPrev => "Previous drawing key",
            Action::KeyJumpNext => "Next drawing key",
            Action::FrameAdd => "Add frame (hold)",
            Action::FrameDuplicate => "Duplicate frame",
            Action::FrameDelete => "Delete frame",
            Action::FrameInsertBefore => "Insert frames before",
            Action::OnionToggle => "Toggle onion skin",
            Action::LayerAdd => "Add layer",
            Action::LayerDelete => "Delete layer",
            Action::LayerGroup => "Group selected layers",
            Action::LayerUngroup => "Ungroup",
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
            Action::WheelPinToggle => "Pin the colour wheel (one colour for every tool)",
            Action::LayerClipToggle => "Clip layer to the one below",
            Action::LayerAlphaLockToggle => "Lock layer alpha",
            Action::LayerKeyLinkBelow => "Toggle key link to layer below",
            Action::KeyBlank => "Insert blank key",
            Action::KeyCopy => "Insert duplicate key",
            Action::Hold => "Hold (delete key)",
            Action::SizeDown => "Brush size down",
            Action::SizeUp => "Brush size up",
            Action::SizeSwap => "Brush size: swap to the one drawn at last",
            Action::Undo => "Undo",
            Action::Redo => "Redo",
            Action::ClearCell => "Clear current cell",
            Action::PasteImage => "Paste image as background",
            Action::ToggleCheckerBg => "Toggle checker backdrop (retired)",
            Action::TogglePanels => "Toggle floating panels",
            Action::ToggleMiniTimeline => "Toggle mini timeline",
            Action::ToggleLockAxes => "Shift lock: canvas / screen axes",
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
            Action::FillLassoErase => "Lasso fill: erase the shape instead (hold)",
            Action::FillLassoEnclose => "Lasso fill: switch Enclose & fill (hold)",
            Action::FillPolygonSnap => "Lasso fill: polygon edges in 15° steps (hold)",
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
            Action::CameraBoxToggle => "Draw camera box",
            Action::CameraKeyAdd => "Add camera key",
            Action::CameraKeyDelete => "Delete camera key",
            Action::CameraLookThrough => "Look through camera",
            Action::CameraKeyPrev => "Previous camera key",
            Action::CameraKeyNext => "Next camera key",
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
                Action::SelModeAdd
                    | Action::SelModeSubtract
                    | Action::SelModeIntersect
                    | Action::FillLassoErase
                    | Action::FillLassoEnclose
                    | Action::FillPolygonSnap
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
    // `Ctrl++` splits into an empty last part: the key is `+` itself.
    if key_name == Some("") && s.trim_end().ends_with("++") {
        key_name = Some("+");
    }
    let key = match key_name {
        // Any case: `space`, `delete` and `esc` are how people write them.
        Some(name) => Some(
            egui::Key::from_name(name)
                // egui's short forms (`Esc`, `Del`) are capitalised.
                .or_else(|| {
                    let mut c = name.chars();
                    let first = c.next()?.to_ascii_uppercase();
                    egui::Key::from_name(&format!("{first}{}", c.as_str().to_ascii_lowercase()))
                })
                .or_else(|| egui::Key::ALL.iter().copied().find(|k| k.name().eq_ignore_ascii_case(name)))?,
        ),
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
        (Action::SelModeAdd, Action::FillLassoEnclose)
        | (Action::SelModeSubtract, Action::FillLassoErase)
        | (Action::SelModeIntersect, Action::FillPolygonSnap) => {
            Some("the Lasso tool reads one, the Fill tool's lasso the other")
        }
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

// --- the mouse wheel ---

/// The modifier the wheel is turned with. Each holds one [`WheelAction`], so
/// the wheel can't clash with itself; two modifiers or more do nothing.
#[derive(Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Debug)]
pub enum WheelSlot {
    Plain,
    Shift,
    Ctrl,
    Alt,
}

impl WheelSlot {
    pub const ALL: [WheelSlot; 4] = [WheelSlot::Plain, WheelSlot::Shift, WheelSlot::Ctrl, WheelSlot::Alt];

    /// The line it goes by in `shortcuts.toml`.
    pub fn key(self) -> &'static str {
        match self {
            WheelSlot::Plain => "wheel",
            WheelSlot::Shift => "shift_wheel",
            WheelSlot::Ctrl => "ctrl_wheel",
            WheelSlot::Alt => "alt_wheel",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WheelSlot::Plain => "Wheel",
            WheelSlot::Shift => "Shift+Wheel",
            WheelSlot::Ctrl => "Ctrl+Wheel",
            WheelSlot::Alt => "Alt+Wheel",
        }
    }

    /// The slot a file line names, if it names one.
    fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.key() == name)
    }

    /// The slot the wheel turns in with `mods` held; `None` for two or more.
    pub fn of(mods: egui::Modifiers) -> Option<Self> {
        match (mods.command || mods.ctrl, mods.shift, mods.alt) {
            (false, false, false) => Some(WheelSlot::Plain),
            (false, true, false) => Some(WheelSlot::Shift),
            (true, false, false) => Some(WheelSlot::Ctrl),
            (false, false, true) => Some(WheelSlot::Alt),
            _ => None,
        }
    }
}

/// What a notch of the wheel does, in whichever [`WheelSlot`] it sits.
/// Over the canvas, the Timeline and the mini timeline alike — except
/// [`WheelAction::PanTimeline`], which only the Timeline's tracks have.
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub enum WheelAction {
    /// Step frames by the timeline's step size.
    Frames,
    /// Jump drawing to drawing, as Shift+A / Shift+S do.
    Drawings,
    /// Zoom the view about the pointer; over the tracks, widen the frames.
    Zoom,
    BrushSize,
    /// Walk the active layer up and down the stack.
    Layers,
    /// Scroll the Timeline's tracks sideways.
    PanTimeline,
    Nothing,
}

impl WheelAction {
    pub const ALL: [WheelAction; 7] = [
        WheelAction::Frames,
        WheelAction::Drawings,
        WheelAction::Zoom,
        WheelAction::BrushSize,
        WheelAction::Layers,
        WheelAction::PanTimeline,
        WheelAction::Nothing,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WheelAction::Frames => "Step frames",
            WheelAction::Drawings => "Jump drawings",
            WheelAction::Zoom => "Zoom",
            WheelAction::BrushSize => "Brush size",
            WheelAction::Layers => "Walk layers",
            WheelAction::PanTimeline => "Pan timeline",
            WheelAction::Nothing => "Nothing",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            WheelAction::Frames => {
                "Each notch steps by the timeline's step size (×N). Wheel down advances \
                 unless Invert timeline scroll is on."
            }
            WheelAction::Drawings => {
                "Each notch jumps to the next or previous drawing, skipping holds — the \
                 drawings Shift+A / Shift+S stop at, the linked layer's included."
            }
            WheelAction::Zoom => {
                "Over the canvas, zooms the view about the pointer. Over the Timeline, \
                 widens or narrows the frames."
            }
            WheelAction::BrushSize => "Each notch is one press of Brush size up / down.",
            WheelAction::Layers => "Wheel up selects the layer above, down the one below.",
            WheelAction::PanTimeline => {
                "Over the Timeline, scrolls the tracks sideways. Does nothing elsewhere."
            }
            WheelAction::Nothing => "The wheel does nothing with this held.",
        }
    }

    /// The value it is written as in `shortcuts.toml`.
    pub fn name(self) -> &'static str {
        match self {
            WheelAction::Frames => "frames",
            WheelAction::Drawings => "drawings",
            WheelAction::Zoom => "zoom",
            WheelAction::BrushSize => "brush_size",
            WheelAction::Layers => "layers",
            WheelAction::PanTimeline => "pan_timeline",
            WheelAction::Nothing => "nothing",
        }
    }

    /// A file value, read in any case. `""` and `none` mean nothing too, as
    /// they unbind a key.
    fn named(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("none") {
            return Some(WheelAction::Nothing);
        }
        Self::ALL.into_iter().find(|a| a.name().eq_ignore_ascii_case(s))
    }
}

/// What the wheel does with nothing changed: frames, Shift for drawings, Ctrl
/// zooms as it always has over the tracks, and Alt pans them — the job Shift
/// had there before it went to the drawings.
fn default_wheel() -> BTreeMap<WheelSlot, WheelAction> {
    BTreeMap::from([
        (WheelSlot::Plain, WheelAction::Frames),
        (WheelSlot::Shift, WheelAction::Drawings),
        (WheelSlot::Ctrl, WheelAction::Zoom),
        (WheelSlot::Alt, WheelAction::PanTimeline),
    ])
}

// --- the map itself ---

/// Every action's combo, saved as `action = "Combo"` lines.
///
/// An action the user unbound is saved as `action = ""` and kept in
/// `unbound`, so loading doesn't hand it its default back. An action absent
/// from both maps is unbound too, but takes its default on the next load.
#[derive(Clone, Debug, PartialEq)]
pub struct ShortcutMap {
    pub bindings: BTreeMap<Action, KeyCombo>,
    pub unbound: BTreeSet<Action>,
    /// Lines of the file that didn't read, as `(name, value)`: an action this
    /// build doesn't know, or a combo that doesn't parse. Its action keeps
    /// its default meanwhile, and saving leaves the line as written until the
    /// action is bound here — a typo in a hand edit is the user's to fix, not
    /// the app's to overwrite.
    pub unread: Vec<(String, String)>,
    /// What the wheel does in each slot, saved as `wheel = "frames"`,
    /// `shift_wheel = "drawings"` and so on. Always holds all four.
    pub wheel: BTreeMap<WheelSlot, WheelAction>,
}

impl Serialize for ShortcutMap {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut out: BTreeMap<Action, String> =
            self.bindings.iter().map(|(&a, c)| (a, c.display())).collect();
        for &a in &self.unbound {
            out.insert(a, String::new());
        }
        let mut m = ser.serialize_map(Some(out.len() + self.wheel.len()))?;
        for (a, c) in &out {
            m.serialize_entry(a, c)?;
        }
        for (slot, a) in &self.wheel {
            m.serialize_entry(slot.key(), a.name())?;
        }
        m.end()
    }
}

impl<'de> Deserialize<'de> for ShortcutMap {
    /// Entry by entry, so one line naming an action that no longer exists, or
    /// a combo that doesn't parse, costs that line rather than every binding.
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        /// A value as written: text, or anything else TOML allows, which
        /// costs only its own line.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Text(String),
            Other(serde::de::IgnoredAny),
        }
        let raw: BTreeMap<String, Raw> = BTreeMap::deserialize(de)?;
        let mut map = ShortcutMap {
            bindings: BTreeMap::new(),
            unbound: BTreeSet::new(),
            unread: Vec::new(),
            wheel: BTreeMap::new(),
        };
        for (name, value) in raw {
            let Raw::Text(combo) = value else {
                log::warn!("shortcuts: {name} isn't text, skipped");
                map.unread.push((name, "(not text)".into()));
                continue;
            };
            if let Some(slot) = WheelSlot::named(&name) {
                match WheelAction::named(&combo) {
                    Some(a) => {
                        map.wheel.insert(slot, a);
                    }
                    None => {
                        log::warn!("shortcuts: bad wheel action {combo:?} for {name}, using the default");
                        map.unread.push((name, combo));
                    }
                }
                continue;
            }
            let Some(action) = action_named(&name) else {
                log::warn!("shortcuts: unknown action {name:?}, skipped");
                map.unread.push((name, combo));
                continue;
            };
            let c = combo.trim();
            if c.is_empty() || c.eq_ignore_ascii_case("none") {
                map.unbound.insert(action);
            } else if let Some(c) = parse_combo(c) {
                map.bindings.insert(action, c);
            } else {
                log::warn!("shortcuts: bad combo {combo:?} for {name}, using the default");
                map.unread.push((name, combo));
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
        b.insert(Action::TogglePerspectiveGrid, KeyCombo::shift_alt(K::G));
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
        b.insert(Action::PlayPauseHere, KeyCombo::shift(K::Space));
        b.insert(Action::OnionToggle, KeyCombo::plain(K::O));
        // Layers.
        b.insert(Action::LayerAdd, KeyCombo::plain(K::T));
        b.insert(Action::LayerDelete, KeyCombo::shift(K::T));
        // Ctrl+G groups, as in Krita and Photoshop.
        b.insert(Action::LayerGroup, KeyCombo::ctrl(K::G));
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
        // The Fill lasso's holds share them: removing is erasing, and the
        // Lasso tool never sees these, nor the Fill tool those.
        b.insert(Action::FillLassoErase, KeyCombo::modifier_only(true, false, true));
        b.insert(Action::FillLassoEnclose, KeyCombo::modifier_only(true, true, false));
        b.insert(Action::FillPolygonSnap, KeyCombo::modifier_only(false, true, true));
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
        // CameraBoxToggle, CameraKeyPrev and CameraKeyNext start unbound:
        // every key near the camera binds is taken, and the panel has a
        // checkbox or a button for each.
        Self {
            bindings: b,
            unbound: BTreeSet::new(),
            unread: Vec::new(),
            wheel: default_wheel(),
        }
    }
}

/// The action a file line names, if any.
fn action_named(name: &str) -> Option<Action> {
    use serde::de::IntoDeserializer;
    let a: Result<Action, serde::de::value::Error> = Action::deserialize(name.into_deserializer());
    a.ok()
}

/// The name an action's line goes by in the file.
fn key_of(action: Action) -> String {
    match toml::Value::try_from(action) {
        Ok(toml::Value::String(s)) => s,
        _ => format!("{action:?}"),
    }
}

impl ShortcutMap {
    pub fn get(&self, action: Action) -> Option<KeyCombo> {
        self.bindings.get(&action).copied()
    }
    pub fn set(&mut self, action: Action, combo: KeyCombo) {
        self.bindings.insert(action, combo);
        self.unbound.remove(&action);
        self.forget_unread(action);
    }

    /// Bound here now: the file's unreadable line for it can go.
    fn forget_unread(&mut self, action: Action) {
        let key = key_of(action);
        self.unread.retain(|(name, _)| *name != key);
    }

    /// What the wheel does with `mods` held. Two modifiers or more: nothing.
    pub fn wheel_action(&self, mods: egui::Modifiers) -> WheelAction {
        WheelSlot::of(mods)
            .and_then(|s| self.wheel.get(&s).copied())
            .unwrap_or(WheelAction::Nothing)
    }

    /// The slot holding `action`, if one does — the first, if several.
    pub fn wheel_slot_of(&self, action: WheelAction) -> Option<WheelSlot> {
        self.wheel.iter().find(|(_, &a)| a == action).map(|(&s, _)| s)
    }

    /// Put `action` in `slot`. The file's unreadable line for the slot can
    /// go, as a rebind's does.
    pub fn set_wheel(&mut self, slot: WheelSlot, action: WheelAction) {
        self.wheel.insert(slot, action);
        self.unread.retain(|(name, _)| name != slot.key());
    }

    /// Unbind `action`, for good: it stays unbound across a save and load
    /// rather than taking its default back.
    pub fn clear(&mut self, action: Action) {
        self.bindings.remove(&action);
        self.unbound.insert(action);
        self.forget_unread(action);
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
///
/// None under test. A test's `AppState` loads and saves like the app's, so a
/// test that rebinds a key saved its map — the defaults it started from —
/// over the real keymap of whoever ran `cargo test`, and every test read
/// that file for its bindings. Tests that want a file bring their own path
/// (`ensure_at`, `save_at`).
fn config_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    config_dir().map(|d| d.join("shortcuts.toml"))
}

/// The saved shortcuts, and why the file didn't load if it didn't — the
/// defaults stand in then, and nothing is saved over the file until it reads.
pub fn load() -> (ShortcutMap, Option<String>) {
    match read_file() {
        Some(Ok(map)) => (map, None),
        Some(Err(e)) => {
            log::warn!("shortcuts.toml parse failed ({e}); using defaults");
            (ShortcutMap::default(), Some(e))
        }
        None => (ShortcutMap::default(), None),
    }
}

/// A saved shortcuts file, migrated, with defaults for whatever it doesn't
/// mention. Defaults throughout when it can't be read.
#[cfg(test)]
pub fn load_from_str(text: &str) -> ShortcutMap {
    parse(text).unwrap_or_else(|e| {
        log::warn!("shortcuts.toml parse failed ({e}); using defaults");
        ShortcutMap::default()
    })
}

/// [`load_from_str`], or why the text can't be read.
pub fn parse(text: &str) -> Result<ShortcutMap, String> {
    match toml::from_str::<ShortcutMap>(text) {
        Ok(mut map) => {
            // Migration: the in-canvas eyedropper was folded into the screen
            // picker. Drop the retired binding, and hand its key to the picker
            // that replaced it unless the user had already rebound that one.
            // Retired with the checker backdrop: nothing to hand on.
            map.bindings.remove(&Action::ToggleCheckerBg);
            map.unbound.remove(&Action::ToggleCheckerBg);
            if let Some(old) = map.bindings.remove(&Action::ToolColorPicker) {
                let picker = map.bindings.get(&Action::PickScreenColor).copied();
                if picker.is_none() || picker == Some(KeyCombo::shift(egui::Key::C)) {
                    map.bindings.insert(Action::PickScreenColor, old);
                }
            }
            // Migration: Ctrl+V now pastes a pixel selection. Move an existing
            // PasteImage binding off it so the two don't both fire — in a file
            // from before selections, that is: one that names selection_paste
            // was saved since, and Ctrl+V there is the user's choice.
            let selections_known = map.bindings.contains_key(&Action::SelectionPaste)
                || map.unbound.contains(&Action::SelectionPaste)
                || map.unread.iter().any(|(n, _)| *n == key_of(Action::SelectionPaste));
            if !selections_known
                && map.bindings.get(&Action::PasteImage) == Some(&KeyCombo::ctrl(egui::Key::V))
            {
                map.bindings
                    .insert(Action::PasteImage, KeyCombo::ctrl_shift(egui::Key::V));
            }
            // Migration: Ctrl+G became "group layers", and the perspective
            // grid toggle moved off it. A file from before keeps its grid on
            // Ctrl+G, and grouping starts unbound there instead of clashing.
            let grid_on_ctrl_g = map.bindings.get(&Action::TogglePerspectiveGrid)
                == Some(&KeyCombo::ctrl(egui::Key::G));
            if grid_on_ctrl_g && !map.bindings.contains_key(&Action::LayerGroup) && !map.unbound.contains(&Action::LayerGroup)
            {
                map.unbound.insert(Action::LayerGroup);
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
            full.unread = map.unread;
            full.wheel.extend(map.wheel);
            Ok(full)
        }
        Err(e) => Err(e.message().to_string()),
    }
}

/// The shortcuts file as it is on disk now; `None` when there isn't one.
/// For picking up edits made to it outside the app.
pub fn read_file() -> Option<Result<ShortcutMap, String>> {
    let text = std::fs::read_to_string(config_path()?).ok()?;
    Some(parse(&text))
}

/// The shortcuts file's path, written out from `map` first if it isn't
/// there yet — a fresh install has none until a binding changes — so an
/// editor opens on every binding.
pub fn ensure_file(map: &ShortcutMap) -> std::io::Result<PathBuf> {
    use std::io::{Error, ErrorKind};
    let path = config_path().ok_or_else(|| Error::new(ErrorKind::NotFound, "no config folder"))?;
    ensure_at(&path, map)?;
    Ok(path)
}

/// [`ensure_file`] at `path`.
fn ensure_at(path: &std::path::Path, map: &ShortcutMap) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(map).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
        std::fs::write(path, text)?;
    }
    Ok(())
}

/// Open `path` with whatever the system opens that kind of file with.
pub fn open_file(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let program = "xdg-open";
    // Not waited on: explorer exits non-zero even when it opened the file.
    std::process::Command::new(program).arg(path).spawn().map(drop)
}

pub fn save(map: &ShortcutMap) {
    let Some(path) = config_path() else {
        log::warn!("no config dir; shortcuts not saved");
        return;
    };
    match save_at(&path, map) {
        Ok(()) => log::info!("Saved shortcuts → {}", path.display()),
        Err(e) => log::warn!("shortcut save failed: {e}"),
    }
}

/// Write `map` to the file at `path`, editing it in place when there is one
/// (see [`save_text`]). A file that isn't TOML at all is left alone: it is a
/// hand edit half done, and the user's to finish.
fn save_at(path: &std::path::Path, map: &ShortcutMap) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    let text = match std::fs::read_to_string(path) {
        Ok(old) => save_text(&old, map).ok_or_else(|| {
            Error::new(ErrorKind::InvalidData, "shortcuts.toml doesn't read; not written over")
        })?,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            toml::to_string_pretty(map).map_err(|e| Error::new(ErrorKind::InvalidData, e))?
        }
        Err(e) => return Err(e),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)
}

/// `old`, the file as it stands, with `map` written into it: only the lines
/// whose binding differs change, so comments, order, spelling (`ctrl+z`) and
/// lines this build can't read all stay as the user wrote them. `None` when
/// `old` isn't TOML.
fn save_text(old: &str, map: &ShortcutMap) -> Option<String> {
    let mut doc: toml_edit::DocumentMut = old.parse().ok()?;
    let mut want: BTreeMap<String, String> =
        map.bindings.iter().map(|(&a, c)| (key_of(a), c.display())).collect();
    for &a in &map.unbound {
        want.insert(key_of(a), String::new());
    }
    for (slot, a) in &map.wheel {
        want.insert(slot.key().to_string(), a.name().to_string());
    }
    let keep: BTreeSet<&str> = map.unread.iter().map(|(n, _)| n.as_str()).collect();
    // Actions the map no longer mentions (a reset dropped them). A name that
    // isn't an action is not ours to remove.
    let stale: Vec<String> = doc
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| !want.contains_key(k) && !keep.contains(k.as_str()) && action_named(k).is_some())
        .collect();
    for k in stale {
        doc.remove(&k);
    }
    let same = |key: &str, written: &str, want: &str| {
        if WheelSlot::named(key).is_some() {
            return WheelAction::named(written).is_some_and(|w| Some(w) == WheelAction::named(want));
        }
        let unbound = |s: &str| s.trim().is_empty() || s.trim().eq_ignore_ascii_case("none");
        (unbound(written) && unbound(want))
            || (!unbound(want) && parse_combo(written.trim()).is_some_and(|c| Some(c) == parse_combo(want)))
    };
    for (k, v) in &want {
        if keep.contains(k.as_str()) {
            continue;
        }
        if !doc.get(k).and_then(|i| i.as_str()).is_some_and(|w| same(k, w, v)) {
            doc[k.as_str()] = toml_edit::value(v.as_str());
        }
    }
    Some(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test suite has no shortcuts file: nothing of the user's to read,
    /// and nowhere for a rebind in a test to be saved over it.
    #[test]
    fn tests_never_reach_the_users_shortcuts_file() {
        assert_eq!(config_path(), None);
        assert!(read_file().is_none());
        assert_eq!(load(), (ShortcutMap::default(), None));
        assert!(ensure_file(&ShortcutMap::default()).is_err());
        // Saving is a no-op rather than a write.
        save(&ShortcutMap::default());
    }

    /// Opening the file on a fresh install writes it out first, with every
    /// binding in it; an existing file — the user's edits — is left as it is.
    #[test]
    fn the_shortcuts_file_is_written_only_when_missing() {
        let dir = std::env::temp_dir().join(format!("animator-keymap-{}", std::process::id()));
        let path = dir.join("nested").join("shortcuts.toml");
        let _ = std::fs::remove_dir_all(&dir);
        let mut map = ShortcutMap::default();
        map.clear(Action::Undo);
        ensure_at(&path, &map).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(parse(&written).unwrap(), map);
        std::fs::write(&path, "undo = \"Ctrl+U\"
").unwrap();
        ensure_at(&path, &ShortcutMap::default()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "undo = \"Ctrl+U\"
");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file that doesn't read says why, rather than quietly turning into
    /// the defaults — which a reload would then save over the user's file.
    #[test]
    fn a_broken_shortcuts_file_is_an_error_not_the_defaults() {
        assert!(parse("undo = [").is_err());
        assert_eq!(parse("").unwrap(), ShortcutMap::default());
        assert_eq!(load_from_str("undo = ["), ShortcutMap::default());
        // And it isn't saved over: the half-done edit is the user's to finish.
        let dir = std::env::temp_dir().join(format!("animator-keymap-broken-{}", std::process::id()));
        let path = dir.join("shortcuts.toml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "undo = [").unwrap();
        assert!(save_at(&path, &ShortcutMap::default()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "undo = [");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A hand-edited file keeps everything the user wrote through a save made
    /// here: comments, their spelling of a combo, a line naming an action
    /// this build doesn't have, and a line that doesn't parse. Only the
    /// binding changed here is rewritten.
    #[test]
    fn a_save_edits_the_file_in_place() {
        let old = "# my keys\n\
                   undo = \"ctrl+z\"  # lower case is fine\n\
                   future_tool = \"Q\"\n\
                   redo = \"Ctrl+Nope\"\n\
                   tool_pencil = \"Q\"\n";
        let mut map = parse(old).unwrap();
        assert_eq!(map.get(Action::Undo), Some(KeyCombo::ctrl(egui::Key::Z)));
        assert_eq!(
            map.unread,
            [("future_tool".to_string(), "Q".to_string()), ("redo".to_string(), "Ctrl+Nope".to_string())]
        );
        // The bad line's action keeps its default meanwhile.
        assert_eq!(map.get(Action::Redo), ShortcutMap::default().get(Action::Redo));
        map.set(Action::ToolPencil, KeyCombo::plain(egui::Key::P));
        let new = save_text(old, &map).unwrap();
        assert!(new.starts_with("# my keys\nundo = \"ctrl+z\"  # lower case is fine\n"), "{new}");
        assert!(new.contains("future_tool = \"Q\"") && new.contains("redo = \"Ctrl+Nope\""), "{new}");
        assert!(new.contains("tool_pencil = \"P\""), "{new}");
        assert_eq!(parse(&new).unwrap(), map);
        // Binding the bad line's action here replaces the line.
        map.set(Action::Redo, KeyCombo::ctrl(egui::Key::Y));
        let fixed = save_text(&new, &map).unwrap();
        assert!(fixed.contains("redo = \"Ctrl+Y\"") && !fixed.contains("Nope"), "{fixed}");
        // A reset drops what has no default, and keeps names that aren't ours.
        let mut bound = map.clone();
        bound.set(Action::HoldScrub, KeyCombo::plain(egui::Key::N));
        let with = save_text(&fixed, &bound).unwrap();
        assert!(with.contains("hold_scrub"));
        let reset = save_text(&with, &ShortcutMap::default()).unwrap();
        assert!(!reset.contains("hold_scrub") && reset.contains("future_tool"), "{reset}");
    }

    #[test]
    fn key_names_read_in_any_case() {
        assert_eq!(parse_combo("ctrl+space"), Some(KeyCombo::ctrl(egui::Key::Space)));
        assert_eq!(parse_combo("delete"), Some(KeyCombo::plain(egui::Key::Delete)));
        assert_eq!(parse_combo("Shift+esc"), Some(KeyCombo::shift(egui::Key::Escape)));
        assert_eq!(parse_combo("Ctrl++"), Some(KeyCombo::ctrl(egui::Key::Plus)));
        assert_eq!(parse_combo("ctrl+nope"), None);
    }

    /// Ctrl+V for paste_image in a file saved since selections came in is the
    /// user's choice: the old migration only moves it in older files.
    #[test]
    fn paste_on_ctrl_v_is_kept_in_a_current_file() {
        let old = load_from_str("paste_image = \"Ctrl+V\"\n");
        assert_eq!(old.get(Action::PasteImage), Some(KeyCombo::ctrl_shift(egui::Key::V)));
        let current = load_from_str("paste_image = \"Ctrl+V\"\nselection_paste = \"Ctrl+Shift+P\"\n");
        assert_eq!(current.get(Action::PasteImage), Some(KeyCombo::ctrl(egui::Key::V)));
    }

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
                if b != a && shared_on_purpose(a, b).is_none() {
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
        map.set(Action::FadeOthersToggle, KeyCombo::plain(egui::Key::Backtick));
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
        assert!(!c.contains_key(&Action::FadeOthersToggle));
    }

    /// The reported map: the panels on M, the mini timeline on Shift+M and
    /// the last layer on Tab. Settings flagged Tab, because it was read as
    /// the panel toggle whenever a field had focus, whatever the map said.
    /// The checker backdrop is gone: a file that binds its toggle still
    /// reads, the binding dropped, with no line left over to warn about.
    #[test]
    fn a_saved_checker_toggle_is_dropped_quietly() {
        let map = load_from_str("toggle_checker_bg = \"Ctrl+B\"\nplay_pause = \"Ctrl+Space\"");
        assert_eq!(map.get(Action::ToggleCheckerBg), None);
        assert!(map.unread.is_empty(), "{:?}", map.unread);
        assert_eq!(map.get(Action::PlayPause), Some(KeyCombo::ctrl(egui::Key::Space)));
        assert!(!Action::ALL.contains(&Action::ToggleCheckerBg), "not offered in Settings");
    }

    #[test]
    fn tab_and_backtick_are_free_to_bind() {
        let mut map = ShortcutMap::default();
        map.set(Action::TogglePanels, KeyCombo::plain(egui::Key::M));
        map.set(Action::ToggleMiniTimeline, KeyCombo::shift(egui::Key::M));
        map.set(Action::LayerLast, KeyCombo::plain(egui::Key::Tab));
        map.set(Action::OnionToggle, KeyCombo::plain(egui::Key::Backtick));
        map.set(Action::FadeOthersToggle, KeyCombo::ctrl(egui::Key::Backtick));
        let c = map.clashes();
        for a in [
            Action::TogglePanels,
            Action::ToggleMiniTimeline,
            Action::LayerLast,
            Action::OnionToggle,
            Action::FadeOthersToggle,
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
        map.set(Action::OnionToggle, KeyCombo::plain(egui::Key::Backtick));
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
            vec![Action::OnionToggle]
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

    /// A fresh install groups on Ctrl+G and finds the perspective grid on
    /// Shift+Alt+G; a file from before keeps its grid on Ctrl+G and gets
    /// grouping unbound, rather than a clash.
    #[test]
    fn ctrl_g_groups_unless_an_old_file_has_the_grid_there() {
        let map = ShortcutMap::default();
        assert_eq!(map.get(Action::LayerGroup), Some(KeyCombo::ctrl(egui::Key::G)));
        assert_eq!(map.get(Action::TogglePerspectiveGrid), Some(KeyCombo::shift_alt(egui::Key::G)));
        assert_eq!(map.get(Action::LayerUngroup), None);
        let old = load_from_str("toggle_perspective_grid = \"Ctrl+G\"\ntool_pencil = \"Q\"\n");
        assert_eq!(old.get(Action::TogglePerspectiveGrid), Some(KeyCombo::ctrl(egui::Key::G)));
        assert_eq!(old.get(Action::LayerGroup), None);
        assert_eq!(old.conflict_count(), map.conflict_count(), "no new clash");
        // Saved and loaded again, it stays that way.
        let again = load_from_str(&toml::to_string_pretty(&old).unwrap());
        assert_eq!(again.get(Action::LayerGroup), None);
        // A file that already says what grouping is keeps it.
        let set = load_from_str("toggle_perspective_grid = \"Ctrl+G\"\nlayer_group = \"Ctrl+Shift+G\"\n");
        assert_eq!(set.get(Action::LayerGroup), Some(KeyCombo::ctrl_shift(egui::Key::G)));
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

    fn mods(ctrl: bool, shift: bool, alt: bool) -> egui::Modifiers {
        egui::Modifiers {
            ctrl,
            command: ctrl,
            shift,
            alt,
            ..Default::default()
        }
    }

    /// Out of the box: frames, Shift for drawings, Ctrl zooms, Alt pans the
    /// tracks. Two modifiers do nothing.
    #[test]
    fn the_wheel_defaults_and_its_modifiers() {
        let map = ShortcutMap::default();
        assert_eq!(map.wheel_action(mods(false, false, false)), WheelAction::Frames);
        assert_eq!(map.wheel_action(mods(false, true, false)), WheelAction::Drawings);
        assert_eq!(map.wheel_action(mods(true, false, false)), WheelAction::Zoom);
        assert_eq!(map.wheel_action(mods(false, false, true)), WheelAction::PanTimeline);
        assert_eq!(map.wheel_action(mods(true, true, false)), WheelAction::Nothing);
        assert_eq!(map.wheel_slot_of(WheelAction::Zoom), Some(WheelSlot::Ctrl));
        assert_eq!(map.wheel_slot_of(WheelAction::Layers), None);
    }

    /// The wheel lines read in any case, `none` empties a slot, and a slot
    /// the file doesn't name keeps its default.
    #[test]
    fn wheel_lines_read_from_the_file() {
        let map = parse("shift_wheel = \"Zoom\"\nctrl_wheel = \"none\"\nalt_wheel = \"brush_size\"\n").unwrap();
        assert_eq!(map.wheel[&WheelSlot::Shift], WheelAction::Zoom);
        assert_eq!(map.wheel[&WheelSlot::Ctrl], WheelAction::Nothing);
        assert_eq!(map.wheel[&WheelSlot::Alt], WheelAction::BrushSize);
        assert_eq!(map.wheel[&WheelSlot::Plain], WheelAction::Frames, "the default");
        assert!(map.unread.is_empty());
    }

    /// A wheel line that doesn't read costs that slot only: its default
    /// stands in, and the line stays as written through a save until the
    /// slot is set here.
    #[test]
    fn a_bad_wheel_line_is_kept_as_written() {
        let old = "# wheel\nshift_wheel = \"sideways\"\nundo = \"Ctrl+Z\"\n";
        let mut map = parse(old).unwrap();
        assert_eq!(map.unread, [("shift_wheel".to_string(), "sideways".to_string())]);
        assert_eq!(map.wheel[&WheelSlot::Shift], WheelAction::Drawings, "default meanwhile");
        let new = save_text(old, &map).unwrap();
        assert!(new.contains("shift_wheel = \"sideways\""), "{new}");
        map.set_wheel(WheelSlot::Shift, WheelAction::Layers);
        assert!(map.unread.is_empty());
        let fixed = save_text(&new, &map).unwrap();
        assert!(fixed.contains("shift_wheel = \"layers\"") && !fixed.contains("sideways"), "{fixed}");
    }

    /// A save writes the wheel lines into the file in place: comments and
    /// the user's own spelling stay, and a changed slot is rewritten.
    #[test]
    fn a_save_writes_the_wheel_in_place() {
        let old = "# my wheel\nwheel = \"FRAMES\"  # notches\n";
        let mut map = parse(old).unwrap();
        let new = save_text(old, &map).unwrap();
        assert!(new.starts_with("# my wheel\nwheel = \"FRAMES\"  # notches\n"), "{new}");
        assert!(new.contains("shift_wheel = \"drawings\""), "{new}");
        assert_eq!(parse(&new).unwrap(), map);
        map.set_wheel(WheelSlot::Plain, WheelAction::Drawings);
        let changed = save_text(&new, &map).unwrap();
        assert!(changed.starts_with("# my wheel\nwheel = \"drawings\""), "{changed}");
        assert!(!changed.contains("FRAMES"), "{changed}");
        assert_eq!(parse(&changed).unwrap(), map);
        // A fresh file carries them as well.
        let fresh = toml::to_string_pretty(&map).unwrap();
        assert_eq!(parse(&fresh).unwrap(), map);
        assert!(fresh.contains("alt_wheel = \"pan_timeline\""), "{fresh}");
    }
}
