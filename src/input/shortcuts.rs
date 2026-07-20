//! Configurable keyboard shortcuts.
//!
//! Defaults are concentrated on the left side of QWERTY so a tablet user can
//! operate them with their non-dominant hand without lifting the pen.
//!
//! `ShortcutMap` is serialised to TOML in the user's config directory so
//! rebinds persist across runs.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    ToolPencil,
    ToolInk,
    ToolEraser,
    ToolFill,
    ToolColorPicker,
    PickScreenColor,
    ToolShape,
    ToolTracker,
    PlayPause,
    FramePrev,
    FrameNext,
    FrameAdd,
    FrameDuplicate,
    FrameDelete,
    OnionToggle,
    LayerAdd,
    LayerDelete,
    LayerToggleVisible,
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
    SaveProject,
    OpenProject,
    // Canvas navigation gestures (modifier-only binds: no key, held during drag).
    CanvasZoom,
    CanvasPan,
    CanvasRotate,
    // Layer transform editing.
    LayerTransformToggle,
    TransformKeyAdd,
    TransformKeyDelete,
}

impl Action {
    pub const ALL: &'static [Action] = &[
        Action::ToolPencil,
        Action::ToolInk,
        Action::ToolEraser,
        Action::ToolFill,
        Action::ToolColorPicker,
        Action::PickScreenColor,
        Action::ToolShape,
        Action::ToolTracker,
        Action::PlayPause,
        Action::FramePrev,
        Action::FrameNext,
        Action::FrameAdd,
        Action::FrameDuplicate,
        Action::FrameDelete,
        Action::OnionToggle,
        Action::LayerAdd,
        Action::LayerDelete,
        Action::LayerToggleVisible,
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
        Action::SaveProject,
        Action::OpenProject,
        Action::CanvasZoom,
        Action::CanvasPan,
        Action::CanvasRotate,
        Action::LayerTransformToggle,
        Action::TransformKeyAdd,
        Action::TransformKeyDelete,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::ToolPencil => "Tool: Pencil",
            Action::ToolInk => "Tool: Ink",
            Action::ToolEraser => "Tool: Eraser",
            Action::ToolFill => "Tool: Fill",
            Action::ToolColorPicker => "Tool: Color picker",
            Action::PickScreenColor => "Pick color from screen",
            Action::ToolShape => "Tool: Shape",
            Action::ToolTracker => "Tool: Tracker",
            Action::PlayPause => "Play / Pause",
            Action::FramePrev => "Previous frame",
            Action::FrameNext => "Next frame",
            Action::FrameAdd => "Add frame (hold)",
            Action::FrameDuplicate => "Duplicate frame",
            Action::FrameDelete => "Delete frame",
            Action::OnionToggle => "Toggle onion skin",
            Action::LayerAdd => "Add layer",
            Action::LayerDelete => "Delete layer",
            Action::LayerToggleVisible => "Toggle layer visibility",
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
            Action::SaveProject => "Save project",
            Action::OpenProject => "Open project",
            Action::CanvasZoom => "Canvas: zoom (drag)",
            Action::CanvasPan => "Canvas: pan (drag)",
            Action::CanvasRotate => "Canvas: rotate (drag)",
            Action::LayerTransformToggle => "Layer transform mode",
            Action::TransformKeyAdd => "Add transform key",
            Action::TransformKeyDelete => "Delete transform key",
        }
    }
}

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
    /// combos never match here — they are drag gestures, see `mods_held`.
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

    /// True when exactly this combo's modifiers are currently held (and at least
    /// one is). Used to pick a canvas navigation gesture at drag start.
    pub fn mods_held(self, i: &egui::InputState) -> bool {
        let m = i.modifiers;
        (self.ctrl || self.shift || self.alt)
            && m.ctrl == self.ctrl
            && m.shift == self.shift
            && m.alt == self.alt
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

// --- the map itself ---

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShortcutMap {
    #[serde(flatten)]
    pub bindings: BTreeMap<Action, KeyCombo>,
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
        b.insert(Action::ToolColorPicker, KeyCombo::plain(K::C));
        b.insert(Action::PickScreenColor, KeyCombo::shift(K::C));
        b.insert(Action::ToolShape, KeyCombo::plain(K::G));
        b.insert(Action::ToolTracker, KeyCombo::plain(K::X));
        // Frame navigation: A / S.
        b.insert(Action::FramePrev, KeyCombo::plain(K::A));
        b.insert(Action::FrameNext, KeyCombo::plain(K::S));
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
        // Paste clipboard image as a background layer.
        b.insert(Action::PasteImage, KeyCombo::ctrl(K::V));
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
        // Project file.
        b.insert(Action::SaveProject, KeyCombo::ctrl(K::S));
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
        Self { bindings: b }
    }
}

impl ShortcutMap {
    pub fn get(&self, action: Action) -> Option<KeyCombo> {
        self.bindings.get(&action).copied()
    }
    pub fn set(&mut self, action: Action, combo: KeyCombo) {
        self.bindings.insert(action, combo);
    }

    /// Returns every action that has fired this frame. Also Ctrl+Shift+Z is
    /// treated as an alias for Redo because that's the muscle-memory shortcut
    /// on most apps.
    pub fn poll_actions(&self, ctx: &egui::Context) -> Vec<Action> {
        ctx.input(|i| {
            let mut out: Vec<Action> = Vec::new();
            for (&action, combo) in &self.bindings {
                // Modifier-only combos (canvas gestures) are not press actions.
                if combo.key.is_none() {
                    continue;
                }
                if combo.matches(i) {
                    out.push(action);
                }
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
}

/// Path to the persisted shortcuts file.
fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("animator-app").join("shortcuts.toml"))
}

pub fn load() -> ShortcutMap {
    let Some(path) = config_path() else {
        return ShortcutMap::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ShortcutMap::default();
    };
    match toml::from_str::<ShortcutMap>(&text) {
        Ok(map) => {
            log::info!("Loaded shortcuts from {}", path.display());
            // Merge in any missing defaults so newly added actions are bound.
            let mut full = ShortcutMap::default();
            for (a, c) in map.bindings {
                full.bindings.insert(a, c);
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
