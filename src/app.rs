//! Top-level application state. Wires project (timeline + layers), tools, UI.

mod button_drag;
mod grids;
mod groups;
mod lasso_fill;
mod liquify;
mod orient;
mod select;

pub use button_drag::ButtonDragState;
pub use lasso_fill::{FillMode, LassoHeld};
pub use select::SelGesture;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use eframe::CreationContext;
use egui::{Color32, ColorImage, TextureHandle, TextureOptions};

use crate::brush_library::BrushLibrary;
use crate::color::{Harmony, HarmonySet};
use crate::doc::camera::{CamGrab, Camera, Ease};
use crate::doc::canvas::{par_rows, Canvas, DirtyRect};
use crate::doc::layer::{CellId, TrackSample};
use crate::doc::orient::Orient;
use crate::doc::project::{Block, Project};
use crate::doc::transform::Transform;
use crate::input::button_drag::ButtonDrag;
use crate::input::pointer::PointerSample;
use crate::input::shortcuts::{self, Action, ShortcutMap};
use crate::input::tablet::{PenInput, PenPacket};
use crate::io::kra::{self, KraDoc};
use crate::krita_link::{self, KritaLink, Pulled};
use crate::timeline::onion::{OnionConfig, OnionDirection, OnionPin, OnionStep};
use crate::timeline::playback::Playback;
use crate::tools::lasso::Mask;
use crate::tools::ribbon::{union_rect, StrokeWorkspace};
use crate::tools::perspective::{self, GridGrab, PerspectiveConfig};
use crate::tools::select_mask::{SelOp, SelShape, SelectionMask};
use crate::tools::wand::WandOptions;
use crate::tools::selection::{Grab, Pose, Selection};
use crate::tools::stroke::StrokeBuilder;
use crate::tools::shape::GridPlane;
use crate::tools::{ActiveTool, BrushSettings, ShapeKind, SmoothingOptions};
use crate::ui;
use crate::ui::canvas_gpu::CanvasGpu;
use crate::ui::theme::{BaseColors, Scheme};
use crate::undo::{self, History};

pub struct NewProjectConfig {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    /// The "save as preset" name box.
    pub preset_name: String,
    /// In-progress inline rename of a saved preset.
    pub preset_rename: Option<LayerRename>,
}

/// A saved New project size. A workspace preference, not project data: the
/// sizes an artist keeps coming back to follow them between files.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProjectPreset {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    /// New project opens on this one. At most one preset has it set.
    pub is_default: bool,
}

impl Default for ProjectPreset {
    fn default() -> Self {
        Self {
            name: String::new(),
            width: 1920,
            height: 1080,
            fps: 24.0,
            is_default: false,
        }
    }
}

/// libx264 presets offered in the MP4 export dialog, fastest → smallest.
pub const MP4_PRESETS: &[&str] = &["ultrafast", "fast", "medium", "slow", "veryslow"];

/// Encoder settings backing the MP4 export dialog.
#[derive(Clone)]
pub struct Mp4ExportConfig {
    /// libx264 CRF (0..=51, lower = better quality / bigger file).
    pub crf: u32,
    /// Index into [`MP4_PRESETS`].
    pub preset_idx: usize,
}

impl Default for Mp4ExportConfig {
    fn default() -> Self {
        // crf 18 ≈ visually lossless; "medium" is the libx264 default preset.
        Self {
            crf: 18,
            preset_idx: 2,
        }
    }
}

/// What the export dialog is about to write.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    PngSequence,
    Gif,
    Webp,
    Mp4,
    SpriteSheet,
}

impl ExportKind {
    pub fn title(self) -> &'static str {
        match self {
            ExportKind::PngSequence => "Export PNG sequence",
            ExportKind::Gif => "Export animated GIF",
            ExportKind::Webp => "Export animated WebP",
            ExportKind::Mp4 => "Export MP4",
            ExportKind::SpriteSheet => "Export sprite sheet",
        }
    }

    /// Present-tense label for the busy overlay.
    fn busy(self) -> &'static str {
        match self {
            ExportKind::PngSequence => "Exporting PNG sequence…",
            ExportKind::Gif => "Exporting GIF…",
            ExportKind::Webp => "Exporting WebP…",
            ExportKind::Mp4 => "Exporting MP4…",
            ExportKind::SpriteSheet => "Exporting sprite sheet…",
        }
    }
}

/// Everything the shared export dialog edits. One struct for every format:
/// the range applies to every one of them, and the format-specific parts are
/// small enough that separate configs would cost more than they save.
pub struct ExportConfig {
    pub kind: ExportKind,
    /// Inclusive frame range, like `ImportRangeState`.
    pub start: usize,
    pub end: usize,
    pub mp4: Mp4ExportConfig,
    /// Sprite-sheet columns; `0` = near-square.
    pub sheet_columns: usize,
    pub sheet_padding: u32,
    /// Play the range forward then back (MP4, PNG sequence, GIF, WebP).
    pub ping_pong: bool,
    /// Repeat the range to fill `loop_secs`, rounded up to whole loops (MP4,
    /// PNG sequence — a GIF or WebP repeats on its own).
    pub loop_on: bool,
    pub loop_secs: f32,
    /// A WebP stops after `webp_plays` plays instead of repeating forever.
    pub webp_stop: bool,
    pub webp_plays: u16,
}

impl ExportConfig {
    /// How `kind` plays the range, from the options that apply to it.
    pub fn playback(&self, kind: ExportKind) -> crate::io::frame_order::Playback {
        let loops = matches!(kind, ExportKind::Mp4 | ExportKind::PngSequence);
        crate::io::frame_order::Playback {
            ping_pong: self.ping_pong && kind != ExportKind::SpriteSheet,
            loop_secs: (loops && self.loop_on).then_some(self.loop_secs),
        }
    }
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            kind: ExportKind::PngSequence,
            start: 0,
            end: 0,
            mp4: Mp4ExportConfig::default(),
            sheet_columns: 0,
            sheet_padding: 0,
            ping_pong: false,
            loop_on: false,
            loop_secs: 10.0,
            webp_stop: false,
            webp_plays: 1,
        }
    }
}

/// In-progress shape drag (Shape tool). Anchored at `start`, dragged to `end`;
/// rasterised into the target cell on pointer-up.
#[derive(Clone, Copy)]
pub struct ShapeDrag {
    pub kind: ShapeKind,
    pub start: (f32, f32),
    pub end: (f32, f32),
    /// The perspective grid the shape lies on, in the target cell's pixels —
    /// taken at press, so toggling snap mid-drag can't change the shape.
    pub plane: Option<GridPlane>,
}

/// A Fill press still under the pen. The bucket fills at pen-down; dragging
/// before the pen lifts re-runs it with a new gap (sideways) or expand (up and
/// down), and the whole press lands as one undo step on release.
pub struct FillDrag {
    pub session: crate::tools::fill::FillSession,
    /// Where the press landed, in screen points. Drags count from here.
    anchor: Option<egui::Pos2>,
    /// The slider values at the press, which the drag steps away from.
    base_gap: u8,
    base_expand: u8,
}

/// An in-progress drag on the floating selection's transform box.
///
/// `pose` is the selection's pose at the moment the handle was pressed, and a
/// scale or rotation is re-solved against it every frame rather than
/// accumulated — so a slow drag and a fast one ending in the same place give
/// the same result, and float error cannot creep in over a long drag.
/// A drag on a perspective grid. Like [`SelDrag`], solved against the corners
/// at press time (document space) rather than accumulated.
#[derive(Clone, Copy)]
struct GridDrag {
    grid: usize,
    grab: GridGrab,
    start: [f32; 2],
    corners: [[f32; 2]; 4],
    /// The extra vanishing points where they showed at the press.
    extra: [Option<[f32; 2]>; perspective::MAX_EXTRA_VPS],
    /// Where the grid's coordinates land in the document, at the press. The
    /// camera or layer it follows doesn't move mid-drag.
    space: perspective::Space,
}

/// Screen points an extra vanishing point on the horizon can be pulled off it
/// before it lets go, and how near a free one has to come to stick back on.
const HORIZON_DETACH_PX: f32 = 40.0;
const HORIZON_ATTACH_PX: f32 = 10.0;

/// Perspective snap state for one stroke: where it started, and — once the
/// pointer has moved far enough to tell — the grid direction it is locked to.
#[derive(Clone, Copy)]
struct SnapLock {
    start: [f32; 2],
    dir: Option<[f32; 2]>,
}

#[derive(Clone, Copy)]
pub struct SelDrag {
    grab: Grab,
    /// Where the pointer was, in cell space. A move updates this as it goes,
    /// because a move accumulates in whole pixels; a scale or rotation leaves
    /// it at the press.
    start: (f32, f32),
    pose: Pose,
}

/// Canvas view transform applied on top of the fit-to-window base scale.
/// `zoom` multiplies the base scale, `pan` shifts in screen pixels, `rotation`
/// is in radians about the canvas centre. Each can be reset independently.
///
/// `flip_x` / `flip_y` mirror the *view* only — the drawing check animators
/// reach for constantly. Export never reads `View`, so a flipped view can never
/// reach a file.
#[derive(Clone, Copy)]
pub struct View {
    pub zoom: f32,
    pub pan: egui::Vec2,
    pub rotation: f32,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
        }
    }
}

/// The dimming wash over everything outside the selection.
pub struct SelTint {
    /// The `sel_ver` it was built for.
    pub ver: u64,
    /// Where it sits in the document: x0, y0, x1, y1.
    pub rect: (f32, f32, f32, f32),
    pub tex: TextureHandle,
}

/// Which non-drawing canvas gesture an in-flight drag is performing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavKind {
    Pan,
    Zoom,
    Rotate,
}

/// The canvas backdrop colour before the artist picks one.
const DEFAULT_BG_COLOR: [f32; 3] = [0.12, 0.12, 0.13];

/// The longest side a camera frame may have — a canvas's limit too.
const MAX_CAMERA_SIDE: u32 = 16384;

/// The camera's frame held by one of its handles: what was grabbed, where,
/// and the camera as it was — every step of the drag is solved from that,
/// so nothing drifts.
#[derive(Clone, Copy, Debug)]
pub struct CamDrag {
    pub grab: CamGrab,
    pub from: (f32, f32),
    pub start: Camera,
}

/// How far, in screen pixels, a press in Draw camera mode may wander and
/// still be a tap — the selection lasso's allowance.
const CAMERA_TAP_PX: f32 = 3.0;

/// A box being dragged out in Draw camera mode: where the press landed and
/// where the drag has got to, in document space, and whether it has ever
/// gone past a tap.
#[derive(Clone, Copy, Debug)]
pub struct CamBox {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub moved: bool,
}

/// A camera key picked up off the timeline ruler: the frame it is keyed on,
/// and the frame the drag has reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CamKeyDrag {
    pub from: usize,
    pub to: usize,
}

/// A one-click way to frame the canvas with the camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraFraming {
    /// Centred, pulled out until the whole canvas is in the frame.
    Fit,
    /// Centred, as wide as the frame goes while showing nothing but canvas.
    Fill,
    /// One canvas pixel per exported pixel, where the camera stands.
    OneToOne,
    /// Back over the middle of the canvas, zoom as it is.
    Centre,
}

/// Storage key for [`UiPrefs`].
const UI_PREFS_KEY: &str = "ui_prefs";

/// UI state that outlives a run but that egui's own memory doesn't cover.
/// Panel positions, sizes and collapse state ride along in `egui::Memory`
/// (persisted by eframe automatically); these are plain `AppState` fields, so
/// they need saving by hand.
///
/// `#[serde(default)]` is what keeps a prefs blob written by an older build
/// loadable after a field is added here — without it a missing key fails the
/// whole struct and silently resets every other preference too.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct UiPrefs {
    show_panels: bool,
    show_mini_timeline: bool,
    frame_step: usize,
    show_camera_guide: bool,
    dim_outside_camera: bool,
    show_layer_bounds: bool,
    lock_brush_to_view: bool,
    /// Onion skin is a workspace preference, not project data — tints and
    /// counts a user dialled in should still be there next launch.
    onion: OnionConfig,
    auto_key_transform: bool,
    auto_key_draw: bool,
    /// Which way the mouse wheel scrubs the timeline. Off (default): wheel
    /// down advances. A workspace preference — wheel direction is muscle
    /// memory carried from whatever editor the artist came from.
    invert_timeline_scroll: bool,
    /// What a middle- and a right-button drag do on the canvas. Pen barrel
    /// buttons arrive as those buttons, so this is muscle memory too.
    middle_drag: ButtonDrag,
    right_drag: ButtonDrag,
    /// Whether the timeline wraps. On (default): playback repeats the loop
    /// range and frame stepping wraps at both ends. Off: playback runs once
    /// and stops, and stepping clamps.
    loop_timeline: bool,
    /// Line smoothing. A workspace preference: how much the app fights the
    /// hand is a matter of taste, and of what the artist is used to.
    smoothing: SmoothingOptions,
    /// Per-tool brush settings. A tuned brush is worth as much as a tuned
    /// palette and was previously thrown away on exit. `None` from an
    /// older prefs blob, or from one whose tool list has since changed,
    /// falls back to the built-in defaults.
    tool_brushes: Option<Vec<BrushSettings>>,
    /// Pinned colour swatches. A workspace preference, not project data: a
    /// palette follows the artist between files.
    palette: Vec<[u8; 3]>,
    /// Frame column width in the timeline tracks — how zoomed in the artist
    /// likes to work.
    track_frame_w: f32,
    /// Perspective grid switches. Its `grids` are only a seed now: the grids
    /// from before they were kept per file, handed to any file the app has
    /// none kept for, so nothing set up then went missing.
    perspective: PerspectiveConfig,
    /// Each project file's grids, by path. Kept by the app rather than in
    /// the `.anim`, and never shared between files.
    grid_files: Vec<grids::GridFile>,
    /// How far "fade other layers" pushes the rest of the stack back. Only
    /// the amounts persist — the toggle itself starts off every launch.
    fade: FadeOthers,
    /// Where Krita is installed, once found or picked for Edit in Krita.
    krita_path: Option<PathBuf>,
    /// Saved New project presets, in the order the artist made them.
    project_presets: Vec<ProjectPreset>,
    /// What a selection drag draws, and the px a Grow / Shrink / Feather
    /// takes. Workspace preferences, like the tool brushes.
    sel_shape: SelShape,
    sel_amount: u32,
    /// The magic select's tolerance, gap, expand and switches.
    wand: WandOptions,
    /// The Fill tool's mode and its lasso's two switches. Workspace
    /// preferences, like the selection shape.
    fill_mode: FillMode,
    fill_enclose: bool,
    fill_behind: bool,
    /// Dim everything outside the selection. The switch persists, like the
    /// other view guides.
    tint_outside: bool,
    /// The canvas backdrop: colour (RGB, 0..1) and its opacity.
    /// A workspace preference — the colour an artist works against follows
    /// them from project to project.
    bg_color: [f32; 3],
    bg_opacity: f32,
    /// The Color panel's harmony scheme. Only the scheme persists: the set
    /// is rebuilt around the brush colour each launch.
    harmony: Harmony,
    /// The camera frame size last set. The size itself lasts only for the
    /// session, so this is what brings it back with one click the next time.
    last_camera_size: Option<(u32, u32)>,
    /// Panels closed from the Panels menu or their title bar. Part of the
    /// workspace layout, like where each panel sits.
    hidden_panels: Vec<PanelId>,
    /// Bumped by Reset panel layout; see [`AppState::panel_layout_gen`].
    panel_layout_gen: u32,
    /// The colours the UI is drawn in, and the five a Custom scheme is
    /// made from — kept while another scheme is picked, so switching back
    /// finds them as they were.
    color_scheme: Scheme,
    custom_colors: BaseColors,
}

/// Opacity multipliers for the layers around the active one while "fade other
/// layers" is on. View-only: layer opacity, export and the `.anim` file never
/// see it. Above and below differ because a layer over the drawing covers it,
/// while one under it is usually what the drawing is being matched against.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FadeOthers {
    pub above: f32,
    pub below: f32,
}

impl Default for FadeOthers {
    fn default() -> Self {
        Self {
            above: 0.2,
            below: 0.35,
        }
    }
}

/// Number of tools, and so of slots in the per-tool brush array.
const TOOL_COUNT: usize = 9;

/// The built-in per-tool brushes.
fn default_tool_brushes() -> [BrushSettings; TOOL_COUNT] {
    [
        BrushSettings::default_pencil(),
        BrushSettings::default_ink(),
        BrushSettings::default_eraser(),
        BrushSettings::default_fill(),
        BrushSettings::default_shape(),
        // Tracker, Lasso and Perspective draw nothing; the slots only keep
        // tool indexing into this array safe.
        BrushSettings::default_shape(),
        BrushSettings::default_shape(),
        BrushSettings::default_shape(),
        BrushSettings::default_liquify(),
    ]
}

/// Saved brushes, or the defaults. A blob from a build with a different
/// number of tools is discarded rather than padded: the array is indexed by
/// `ActiveTool::idx`, so a short one would silently reassign brushes to the
/// wrong tools. The exception is a blob from before the tools that were
/// appended last (Perspective, then Liquify): the older slots still line up,
/// and only the new tools' own are missing.
fn restore_tool_brushes(saved: Option<Vec<BrushSettings>>) -> [BrushSettings; TOOL_COUNT] {
    match saved {
        Some(mut v) => {
            if v.len() >= ActiveTool::Perspective.idx() {
                while v.len() < TOOL_COUNT {
                    v.push(default_tool_brushes()[v.len()].clone());
                }
            }
            <[BrushSettings; TOOL_COUNT]>::try_from(v).unwrap_or_else(|_| default_tool_brushes())
        }
        None => default_tool_brushes(),
    }
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            show_panels: true,
            show_mini_timeline: true,
            frame_step: 1,
            show_camera_guide: true,
            dim_outside_camera: true,
            show_layer_bounds: true,
            lock_brush_to_view: false,
            onion: OnionConfig::default(),
            auto_key_transform: false,
            auto_key_draw: false,
            invert_timeline_scroll: false,
            middle_drag: ButtonDrag::Scrub,
            right_drag: ButtonDrag::SwatchPie,
            loop_timeline: true,
            smoothing: SmoothingOptions::default(),
            tool_brushes: None,
            palette: Vec::new(),
            track_frame_w: crate::ui::tracks::DEFAULT_FRAME_W,
            perspective: PerspectiveConfig::default(),
            grid_files: Vec::new(),
            fade: FadeOthers::default(),
            krita_path: None,
            project_presets: Vec::new(),
            sel_shape: SelShape::Freehand,
            sel_amount: 4,
            wand: WandOptions::default(),
            fill_mode: FillMode::Bucket,
            fill_enclose: false,
            fill_behind: false,
            tint_outside: false,
            bg_color: DEFAULT_BG_COLOR,
            bg_opacity: 1.0,
            harmony: Harmony::default(),
            last_camera_size: None,
            hidden_panels: Vec::new(),
            panel_layout_gen: 0,
            color_scheme: Scheme::default(),
            custom_colors: BaseColors::default(),
        }
    }
}

/// What a cell's GPU texture is missing.
///
/// A rect, not a flag: on a layer widened to 3× a 4K frame each cell is
/// ~100 MB, and re-uploading all of it after every stroke — when the stroke
/// touched a few hundred pixels — was the hitch at pen-up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dirty {
    Clean,
    /// Only this region changed since the last upload.
    Rect(DirtyRect),
    /// Upload the whole cell.
    Full,
}

impl Dirty {
    /// This region with `more` added to it.
    pub fn with(self, more: Dirty) -> Dirty {
        match (self, more) {
            (Dirty::Full, _) | (_, Dirty::Full) => Dirty::Full,
            (Dirty::Clean, d) | (d, Dirty::Clean) => d,
            (Dirty::Rect(a), Dirty::Rect(b)) => Dirty::Rect(union_rect(Some(a), b)),
        }
    }
}

/// GPU memory cell and ghost textures may hold together before the least
/// recently shown are dropped. Unbounded, a long playback of a big layer kept
/// every cell resident until the driver started paging.
const TEXTURE_BUDGET: usize = 1536 * 1024 * 1024;

/// Longest side of an onion ghost's texture. A ghost is a faint silhouette, so
/// a smaller texture doesn't show — and on a 3×-wide 4K layer it is ~30× less
/// to build and upload than the cell it comes from.
const GHOST_MAX_SIDE: u32 = 2048;

/// Time a frame may spend packing idle cells. See `AppState::pack_idle_cells`.
const PACK_BUDGET: Duration = Duration::from_millis(4);

/// Seconds the playhead rests before a ghost that isn't built yet gets built.
const GHOST_REST: f64 = 0.12;

/// Seconds a drawing stays the one the pen would land on before its stroke
/// snapshot is taken ahead of the pen. See `AppState::warm_pre`.
const WARM_REST: f64 = GHOST_REST;

/// One resident texture, for the budget.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TexEntry {
    kind: TexKind,
    id: CellId,
    /// Last sync it was on screen.
    used: u64,
    bytes: usize,
}

/// Which textures to drop so the rest fit in `budget` bytes: least recently
/// shown first, and never one shown at `now` — it's on screen this frame.
/// Which texture map a [`TexEntry`] is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TexKind {
    Cell,
    Ghost,
    /// A clipped drawing; `id` is the clipped cell, this what it is cut by.
    Clip(groups::BaseKey),
    /// A group drawn as one image, or a group's mask; `id` is unused.
    Group(groups::GroupKey),
}

/// A clipped layer as it shows on the current frame: see
/// [`AppState::clip_pair`].
struct ClipPair {
    cell: CellId,
    base: groups::BaseKey,
    /// The base's layer: a drawing layer, or a group's header.
    base_layer: usize,
    at: crate::doc::clip::Placement,
}

/// A cell's texture. It holds only `rect` of the cell — the drawing plus a
/// transparent rim — not the whole canvas: drawings rarely fill their cell,
/// and a loop of full 2160×2880 cells outgrew `TEXTURE_BUDGET`, so playback
/// re-uploaded every frame and the canvas stuttered under pan and zoom.
pub struct CellTex {
    pub tex: TextureHandle,
    /// Where the texture sits in the cell. Every inked pixel of the cell is
    /// inside it. Empty for a blank cell, which then draws nothing.
    pub rect: DirtyRect,
    /// The cell's size when it was made. A resized cell starts over.
    cell: [u32; 2],
}

/// Transparent pixels kept around a drawing in its texture, so linear
/// filtering at the drawing's edge blends into clear exactly as it would
/// over the whole cell.
const TEX_RIM: u32 = 2;

/// Where in a `cw`×`ch` cell drawn as `content` its texture should sit:
/// the drawing plus `TEX_RIM`, or an empty rect for a blank cell.
fn tex_rect(content: Option<DirtyRect>, cw: u32, ch: u32) -> DirtyRect {
    match content {
        Some(r) => DirtyRect {
            min_x: r.min_x.saturating_sub(TEX_RIM),
            min_y: r.min_y.saturating_sub(TEX_RIM),
            max_x: (r.max_x + TEX_RIM).min(cw),
            max_y: (r.max_y + TEX_RIM).min(ch),
        },
        None => DirtyRect {
            min_x: 0,
            min_y: 0,
            max_x: 0,
            max_y: 0,
        },
    }
}

/// `r` lies wholly inside `outer`.
fn rect_within(r: DirtyRect, outer: DirtyRect) -> bool {
    r.min_x >= outer.min_x && r.min_y >= outer.min_y && r.max_x <= outer.max_x && r.max_y <= outer.max_y
}

/// The part of `a` inside `b`, if any.
fn rect_meet(a: DirtyRect, b: DirtyRect) -> Option<DirtyRect> {
    let r = DirtyRect {
        min_x: a.min_x.max(b.min_x),
        min_y: a.min_y.max(b.min_y),
        max_x: a.max_x.min(b.max_x),
        max_y: a.max_y.min(b.max_y),
    };
    (r.max_x > r.min_x && r.max_y > r.min_y).then_some(r)
}

/// A texture's size for `rect`. A blank cell still needs one texel.
fn tex_dims(rect: DirtyRect) -> [usize; 2] {
    [
        (rect.max_x - rect.min_x).max(1) as usize,
        (rect.max_y - rect.min_y).max(1) as usize,
    ]
}

/// How a cropped texture catches up with its cell. See `plan_upload`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TexPlan {
    /// Up to date.
    Keep,
    /// Re-send this rect of the cell; it lies inside the texture.
    Patch(DirtyRect),
    /// Make the texture anew, holding this rect of the cell.
    Rebuild(DirtyRect),
}

/// Least room a stroke's texture gets on each side of what it must hold.
const STROKE_ROOM: u32 = 512;

/// A texture for a live stroke that has to hold `need` of a `cw`×`ch` cell:
/// `need` with room around it to grow into — half its longer side, at least
/// `STROKE_ROOM` — so a stroke spreading out re-makes its texture a few
/// times, each bigger, rather than every frame. Once that is most of the
/// cell, the whole cell, which never needs re-making.
fn stroke_room(need: Option<DirtyRect>, cw: u32, ch: u32) -> DirtyRect {
    let whole = DirtyRect {
        min_x: 0,
        min_y: 0,
        max_x: cw,
        max_y: ch,
    };
    let Some(r) = need.filter(|r| r.max_x > r.min_x && r.max_y > r.min_y) else {
        return tex_rect(None, cw, ch);
    };
    let m = ((r.max_x - r.min_x).max(r.max_y - r.min_y) / 2).max(STROKE_ROOM);
    let g = DirtyRect {
        min_x: r.min_x.saturating_sub(m),
        min_y: r.min_y.saturating_sub(m),
        max_x: (r.max_x + m).min(cw),
        max_y: (r.max_y + m).min(ch),
    };
    let area = |r: DirtyRect| (r.max_x - r.min_x) as u64 * (r.max_y - r.min_y) as u64;
    if area(g) * 2 > area(whole) {
        whole
    } else {
        g
    }
}

/// `a` and `b` together; an empty rect adds nothing.
fn rect_join(a: Option<DirtyRect>, b: Option<DirtyRect>) -> Option<DirtyRect> {
    let some = |r: Option<DirtyRect>| r.filter(|r| r.max_x > r.min_x && r.max_y > r.min_y);
    match (some(a), some(b)) {
        (Some(a), b) => Some(b.map_or(a, |b| union_rect(Some(a), b))),
        (None, b) => b,
    }
}

/// What a texture holding `have` of cell `c` (`None`: no usable texture)
/// needs after `dirty` changed. A change inside the texture is a patch. One
/// that reaches past it, where the drawing may have grown, re-crops.
///
/// Unless a live stroke is painting the cell. `stroke` then says where the
/// cell held anything before the stroke began (`Some(None)`: blank), and
/// the texture grows around the drawing with room to spare (`stroke_room`),
/// so the rest of the stroke streams in as patches. Nothing is scanned: the
/// drawing is what was there plus what the stroke touched (`Canvas::dirty`),
/// or what the texture holds plus what changed.
fn plan_upload(c: &Canvas, have: Option<DirtyRect>, dirty: Dirty, stroke: Option<Option<DirtyRect>>) -> TexPlan {
    let whole = DirtyRect {
        min_x: 0,
        min_y: 0,
        max_x: c.width,
        max_y: c.height,
    };
    let fresh = || match stroke {
        Some(base) => stroke_room(rect_join(base, c.dirty), c.width, c.height),
        None => tex_rect(c.content_bounds(), c.width, c.height),
    };
    let Some(have) = have else {
        return TexPlan::Rebuild(fresh());
    };
    let r = match dirty {
        Dirty::Clean => return TexPlan::Keep,
        Dirty::Full => return TexPlan::Rebuild(fresh()),
        Dirty::Rect(r) => r,
    };
    let Some(r) = rect_meet(r, whole) else {
        return TexPlan::Keep;
    };
    if rect_within(r, have) {
        return TexPlan::Patch(r);
    }
    if stroke.is_some() {
        return TexPlan::Rebuild(stroke_room(rect_join(Some(have), Some(r)), c.width, c.height));
    }
    let want = fresh();
    if rect_within(want, have) {
        // The drawing didn't grow: what changed out there is still clear.
        return rect_meet(r, have).map_or(TexPlan::Keep, TexPlan::Patch);
    }
    TexPlan::Rebuild(want)
}

/// A texture image for `rect`, premultiplied from `straight` — its pixels,
/// or nothing for an empty rect, which gets one clear texel.
fn crop_image(rect: DirtyRect, straight: impl FnOnce() -> Vec<u8>) -> ColorImage {
    if rect.max_x > rect.min_x && rect.max_y > rect.min_y {
        premultiplied_image(tex_dims(rect), &straight())
    } else {
        ColorImage::new([1, 1], Color32::TRANSPARENT)
    }
}

/// `rect` of `c` as a texture image — premultiplied exactly as
/// `premultiplied_image` does, but read straight from the cell, with no
/// straight-alpha copy in between, and on several threads once it's big: a
/// whole 8K cell is 33 Mpx. An empty rect gets one clear texel.
fn cell_image(c: &Canvas, rect: DirtyRect) -> ColorImage {
    if rect.max_x <= rect.min_x || rect.max_y <= rect.min_y {
        return ColorImage::new([1, 1], Color32::TRANSPARENT);
    }
    let [w, h] = tex_dims(rect);
    // Zeroed, so its pages come in as each thread fills its own rows rather
    // than all up front on this one.
    let mut pixels: Vec<Color32> = bytemuck::zeroed_vec(w * h);
    par_rows(&mut pixels, w, |first, band| {
        let mut row = vec![0u8; w * 4];
        for (y, dst) in (rect.min_y + first as u32..).zip(band.chunks_exact_mut(w)) {
            c.read_row_into(y, rect.min_x, &mut row);
            for (d, p) in dst.iter_mut().zip(row.chunks_exact(4)) {
                *d = premultiplied(p);
            }
        }
    });
    ColorImage { size: [w, h], pixels }
}

/// A clipped drawing's texture as the canvas shows it: cut to its base.
struct ClipTex {
    tex: TextureHandle,
    /// Where the texture sits in the clipped cell, as `CellTex::rect`. Kept
    /// around the clipped drawing, not the cut: a base edit can uncover more
    /// of the drawing, but never anything outside it.
    rect: DirtyRect,
    /// The clipped cell's size when it was made.
    cell: [u32; 2],
    /// Where the two drawings sat when it was made. A change is a fresh bake.
    at: crate::doc::clip::Placement,
    /// A base pixel lies under the clipped pixel with the same index, so a
    /// change to the base maps onto the same rect here.
    aligned: bool,
    /// What changed in either drawing since.
    pending: Dirty,
}

/// What to drop so `entries` fit in `budget`, never anything shown on sync
/// `now`. Least recently shown first — except during playback, which walks
/// the loop in order: there the texture shown longest ago is the next one
/// needed, so dropping it first would miss on every frame of a loop that
/// doesn't fit. Dropping the newest keeps most of the loop resident instead.
fn evictions(entries: &[TexEntry], budget: usize, now: u64, playing: bool) -> Vec<TexEntry> {
    let mut total: usize = entries.iter().map(|e| e.bytes).sum();
    if total <= budget {
        return Vec::new();
    }
    let mut order: Vec<TexEntry> = entries.iter().copied().filter(|e| e.used < now).collect();
    order.sort_by_key(|e| e.used);
    if playing {
        order.reverse();
    }
    let mut out = Vec::new();
    for e in order {
        if total <= budget {
            break;
        }
        total -= e.bytes;
        out.push(e);
    }
    out
}

/// The tool panels (each rendered as a floating window).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelId {
    Tools,
    Brush,
    Layers,
    Onion,
    Xsheet,
    Timeline,
    Camera,
    /// Retired: its wheel and harmonies moved into the Brush panel. Kept so
    /// prefs that hid it still read; [`PanelId::ALL`] leaves it out, so it
    /// is never drawn or listed.
    Color,
}

impl PanelId {
    /// Every panel, in the order the Panels menu lists them: the left
    /// column, the right column, then the timeline along the bottom.
    pub const ALL: [PanelId; 7] = [
        PanelId::Tools,
        PanelId::Brush,
        PanelId::Layers,
        PanelId::Onion,
        PanelId::Camera,
        PanelId::Xsheet,
        PanelId::Timeline,
    ];
}

impl Default for NewProjectConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 24.0,
            preset_name: String::new(),
            preset_rename: None,
        }
    }
}

/// Source for a start/end frame-range import.
pub enum ImportSource {
    /// Video file; frames extracted on confirm via ffmpeg. `fps` drives the
    /// timestamp seek used for dialog previews.
    Video { path: PathBuf, fps: f64 },
    /// Frames of an animated image already decoded to full-size canvases.
    /// `format` names it: "GIF", "WebP".
    Frames { frames: Vec<Canvas>, format: &'static str },
}

/// Backing state for the frame-range import dialog (video / GIF / WebP).
pub struct ImportRangeState {
    pub source: ImportSource,
    /// Total number of source frames (slider/input upper bound is `total - 1`).
    pub total: usize,
    pub start: usize,
    pub end: usize,
}

/// A decoded preview frame for the range dialog: `(width, height, rgba8)`.
pub type PreviewFrame = (u32, u32, Vec<u8>);

/// A long-running import step running on a worker thread. Polled each frame so
/// the UI never blocks on ffmpeg / decoding. Only one runs at a time.
pub enum BgJob {
    /// Decoding an animated image (`format`: GIF, WebP) before the range
    /// dialog opens.
    FramesDecode {
        rx: Receiver<Result<Vec<Canvas>>>,
        format: &'static str,
    },
    /// Probing a video's frame count / fps before the range dialog opens.
    VideoProbe {
        rx: Receiver<Result<(usize, f64)>>,
        path: PathBuf,
    },
    /// Writing an export. `Ok` carries the file name for the success toast.
    Export(Receiver<Result<String>>),
    /// Extracting the chosen video range, to be dropped on a new layer.
    VideoExtract {
        rx: Receiver<Result<Vec<Canvas>>>,
        name: String,
    },
    /// Reading a `.kra` for File ▸ Import .kra.
    KraImport(Receiver<Result<KraDoc>>),
}

/// A project write running on a worker thread.
///
/// Deliberately *not* a [`BgJob`]: that raises the modal busy overlay, and a
/// save that blocks the window is the thing this exists to avoid. Saving has to
/// leave you free to keep drawing.
pub struct SaveJob {
    rx: Receiver<Result<()>>,
    /// Committed to `project_path` only once the write actually succeeds.
    path: PathBuf,
    /// File name, for the success toast.
    name: String,
}

/// The worker writing the `.kra` for Edit in Krita.
struct KritaSend {
    rx: Receiver<Result<krita_link::Sent>>,
    /// Krita to start once the file is on disk; `None` when the Krita we
    /// launched earlier is still open.
    launch: Option<PathBuf>,
    /// Layers of ours left out (reference, `-x`), named in the toast.
    kept_out: Vec<String>,
}

/// In-progress inline layer rename: which layer, the edit buffer, and
/// whether the TextEdit has been given focus yet (first frame only).
pub struct LayerRename {
    pub index: usize,
    pub buf: String,
    pub focused: bool,
}

pub struct AppState {
    pub project: Project,
    /// File this project was last saved to or loaded from. `None` = never
    /// saved, so Save has to ask for a destination.
    pub project_path: Option<PathBuf>,
    /// Transient confirmation — a save, an export, a frame copied — as the
    /// line it shows, and the instant it expires. A silent overwrite is
    /// otherwise indistinguishable from a save that never ran.
    pub save_toast: Option<(String, Instant)>,
    /// Failed write, shown as a modal until dismissed. Never merely logged — a
    /// silent save that silently failed is how work gets lost.
    pub save_error: Option<String>,
    /// Write in flight on a worker thread. Serialising and compressing a few
    /// hundred MB would otherwise stall the window.
    pub save_job: Option<SaveJob>,
    /// Two-way link with Krita (File ▸ Edit in Krita). Session-only: it ends
    /// with the app, a New or an Open.
    pub krita: Option<KritaLink>,
    /// A send in flight, and the Krita to launch once the file is written.
    krita_send: Option<KritaSend>,
    /// Latest Krita-link message: text, whether it is a warning, expiry.
    pub krita_toast: Option<(String, bool, Instant)>,
    /// Where Krita is installed. Found or picked once, then remembered.
    pub krita_path: Option<PathBuf>,

    /// GPU texture per CellId, lazily created. See `CellTex`.
    pub cell_textures: HashMap<CellId, CellTex>,
    /// What each cell's texture is missing: nothing, one rect, or the lot.
    pub cell_dirty: HashMap<CellId, Dirty>,
    /// Sync counter, and the last sync each texture was on screen — what the
    /// texture budget evicts by.
    sync_frame: u64,
    cell_tex_used: HashMap<CellId, u64>,
    /// Clipped layers' drawings as the canvas shows them, keyed by (clipped
    /// cell, base cell), drawn in place of the plain cell texture. See
    /// `sync_clip_textures`.
    clip_textures: HashMap<(CellId, groups::BaseKey), ClipTex>,
    clip_tex_used: HashMap<(CellId, groups::BaseKey), u64>,
    /// Groups drawn as one image, and group masks. See `app::groups`.
    group_images: HashMap<groups::GroupKey, groups::GroupImage>,
    group_used: HashMap<groups::GroupKey, u64>,
    ghost_tex_used: HashMap<CellId, u64>,
    /// Frame the playhead sat on at the last sync, and since when (egui time).
    /// A ghost not built yet waits for the playhead to rest, so a scrub
    /// doesn't build one for every frame it passes.
    playhead_rest: (usize, f64),
    /// Colorized onion ghosts: one silhouette texture per ghosted cell, with
    /// the tint it was built from so a tint change in the panel rebuilds it.
    /// Separate from `cell_textures` because the ghost replaces the artwork's
    /// RGB wholesale — a vertex-color tint only multiplies, which leaves black
    /// line art black.
    pub ghost_textures: HashMap<CellId, ([u8; 3], TextureHandle)>,
    /// Ghosted cells whose pixels changed: the silhouette is re-uploaded into
    /// the existing handle rather than dropped, so an edit never frees a
    /// texture the canvas may already have queued a mesh against.
    ghost_stale: HashSet<CellId>,
    /// Texture handles that are no longer wanted, parked until the next sync.
    /// Dropping a `TextureHandle` frees the GPU texture, and a free that lands
    /// after the canvas has queued a mesh referencing it makes wgpu submit
    /// against a destroyed texture — which File → New and File → Open would
    /// otherwise do, since they run from menu handling *after* the canvas has
    /// painted. Nothing painted last frame is released before this frame's own
    /// paint, so a one-frame delay is enough.
    retired_textures: Vec<TextureHandle>,
    /// Draws the canvas textures, mixing texels in gamma space — see
    /// `ui::canvas_gpu`. `None` without a GPU, as in tests: the canvas then
    /// draws them as plain egui meshes.
    pub canvas_gpu: Option<CanvasGpu>,
    /// Textures re-filled whole this sync. egui only swaps the new image in
    /// after the frame is built, so `canvas_gpu` would bind the old one —
    /// stale, and placed by the new rect. These draw as meshes this frame.
    pub fresh_full: HashSet<egui::TextureId>,

    pub tool: ActiveTool,
    pub brush: BrushSettings,
    /// Every brush the picker offers, Krita's included.
    pub brush_library: BrushLibrary,
    /// The picker's search text. Session-only.
    pub brush_search: String,
    /// The picker's thumbnails, uploaded as they arrive from the library.
    pub brush_thumbs: HashMap<String, TextureHandle>,
    /// The Import Krita brushes window, while open.
    pub krita_import: Option<crate::brush_library::import::KritaImport>,
    /// Pinned swatches, most recently added last. Capped at
    /// [`AppState::MAX_SWATCHES`].
    pub palette: Vec<[u8; 3]>,
    /// The Color panel's harmony set. Its scheme is a workspace preference;
    /// the base follows the brush colour.
    pub harmony: HarmonySet,
    /// Saved New project presets. Like `palette`, a new project keeps them.
    pub project_presets: Vec<ProjectPreset>,
    /// Drawing clipboard: one cell's pixels, cut or copied from a slot. Held
    /// as a `Canvas` rather than a `CellId` so it survives the undo of the cut
    /// that produced it.
    pub cell_clip: Option<Canvas>,
    /// Drawings selected in the timeline tracks, as (layer, key frame).
    /// Session-only, and dropped by any structural edit the tracks did not
    /// make themselves — keys move, and a stale entry would select whatever
    /// slid into its place.
    pub track_sel: BTreeSet<(usize, usize)>,
    /// Where the last plain click in the tracks landed, as (layer, frame):
    /// the corner a Shift-click selects a rectangle from.
    pub track_anchor: Option<(usize, usize)>,
    /// In-flight drag in the tracks, or `None`.
    pub track_drag: Option<crate::ui::tracks::TrackDrag>,
    /// Track scroll offset in points: x across frames, y down the layers.
    pub track_scroll: egui::Vec2,
    /// Width of one frame column in the tracks, in points. The wheel zooms it
    /// (Ctrl by default; see `WheelAction::Zoom`).
    pub track_frame_w: f32,
    /// The N in the tracks' "On N" timing menu.
    pub track_timing_n: usize,
    /// What the cell collector last ran against: the history's revision and
    /// the pool's length. `None` makes it run. See `collect_cells`.
    gc_key: Option<(u64, usize)>,
    /// Whether a cell has no painted pixel, per cell id. A scan is a full pass
    /// over the buffer, so it is cached, and dropped only when pixels change
    /// (`mark_dirty`) — a structural edit moves ids around but never pixels.
    blank_cache: HashMap<CellId, bool>,
    /// Blank scans still allowed this frame; see `cell_is_blank`.
    blank_scans_left: u32,
    /// The selection, in document space: what every paint edit is clipped to.
    /// Belongs to no drawing, so it survives frame and layer changes. `None`
    /// when nothing is selected. Session-only; see `app::select`.
    pub sel_mask: Option<SelectionMask>,
    /// Bumped whenever `sel_mask` changes; keys everything derived from it.
    pub sel_ver: u64,
    /// Marching-ants loops of `sel_mask`, in document space.
    pub sel_outline: Vec<Vec<(f32, f32)>>,
    /// The active cell's view of `sel_mask`, and what it was built for.
    clip_cache: Option<(select::ClipKey, Arc<Mask>)>,
    /// Selection gesture in progress (lasso, box, ellipse, polygon).
    pub sel_gesture: Option<SelGesture>,
    /// Sticky selection mode; a held modifier overrides it for one drag.
    pub sel_op: SelOp,
    /// Land a moved selection on every drawing of the active layer, not just
    /// the one it was lifted from. Session-only: it starts off every launch,
    /// so it can't quietly spread an edit next week.
    pub sel_all_frames: bool,
    /// What a selection drag draws.
    pub sel_shape: SelShape,
    /// How the magic select picks.
    pub wand: WandOptions,
    /// A magic select press still under the pen.
    wand_drag: Option<select::WandDrag>,
    /// Where a plain press on the selection with the magic select landed: if
    /// it lets go without moving the pixels, it picks there instead.
    wand_repick: Option<(f32, f32)>,
    /// Whether a Fill press floods or draws a lasso.
    pub fill_mode: FillMode,
    /// A lasso fill fills only what its lines shut in.
    pub fill_enclose: bool,
    /// A lasso fill goes under the paint already there.
    pub fill_behind: bool,
    /// The lasso fill being drawn.
    fill_lasso: Option<lasso_fill::FillLasso>,
    /// Pixels a Grow / Shrink / Feather takes.
    pub sel_amount: u32,
    /// Dim everything outside the selection.
    pub tint_outside: bool,
    /// The tint texture over what is not selected.
    pub sel_tint: Option<SelTint>,
    /// Floating pixels lifted through the selection, if any. Bound to one
    /// cell: changing frame or layer lands them first.
    pub selection: Option<Selection>,
    /// A Liquify warp under way, until it lands or is cancelled.
    pub liquify: Option<liquify::LiquifySession>,
    /// The float's cell as it was before the lift, so landing records lift
    /// and stamp as one step and an undo while floating can put it back.
    float_pre: Vec<u8>,
    float_pre_live: bool,
    /// Where the lift erased.
    float_lift_rect: Option<DirtyRect>,
    /// In-flight drag on the selection's transform box, or `None`.
    sel_drag: Option<SelDrag>,
    /// Selection clipboard: mask plus lifted pixels.
    pixel_clip: Option<(Mask, Vec<u8>)>,
    /// Set when the floating pixels changed and their texture must be rebuilt.
    /// Posing does *not* set it — the pose moves the quad, and the GPU samples
    /// the same texture through it.
    sel_tex_stale: bool,
    /// GPU texture for the floating pixels.
    pub selection_tex: Option<TextureHandle>,
    /// Per-tool brush settings preserved across tool switches.
    pub tool_brushes: [BrushSettings; TOOL_COUNT],
    /// Pencil or Ink, whichever was active last: the pen whose size Shape
    /// draws at. Session-only; the app comes back on the pencil anyway.
    pub last_pen: ActiveTool,
    /// The colour wheel is pinned: every tool paints in one colour, and the
    /// wheel's set leads the swatch pie. Session-only.
    wheel_pinned: bool,
    pub stroke: Option<StrokeBuilder>,
    /// CellId being painted into during the current stroke.
    pub stroke_target: Option<CellId>,
    /// When the stroke began: the clock a stabilized stroke paints by.
    stroke_clock: Option<Instant>,
    /// In-progress Shape-tool drag (preview only until pointer-up).
    pub shape_drag: Option<ShapeDrag>,
    /// In-progress Fill press, adjustable by dragging until pointer-up.
    pub fill_drag: Option<FillDrag>,
    /// Shift state, sampled once a frame. Pointer handlers run off tablet
    /// packets rather than egui events, so they have no `InputState` of their
    /// own to ask; a selection scale reads this to keep its proportions.
    shift_held: bool,
    /// Alt, likewise: a selection scale about the box centre.
    alt_held: bool,
    /// Perspective grids and their display/snap switches.
    pub perspective: PerspectiveConfig,
    /// In-flight drag on a perspective grid, or `None`.
    grid_drag: Option<GridDrag>,
    /// The dragged grid as it was at the press, for the drag's undo step.
    grid_drag_before: Option<perspective::PerspectiveGrid>,
    /// Last frame keyed grids were brought to.
    grid_sync_last: Option<usize>,
    /// Every project file's grids but the open one's live copy.
    grid_files: Vec<grids::GridFile>,
    /// The grids from before grids were kept per file, for files the app
    /// has none kept for.
    grid_seed: Vec<perspective::PerspectiveGrid>,
    /// Perspective snap for the stroke in progress, or `None` when the stroke
    /// isn't snapping.
    snap_lock: Option<SnapLock>,

    /// Canvas view transform (zoom / pan / rotate).
    pub view: View,
    /// Screen pixels per *document* pixel, republished by the canvas each
    /// frame (`View::zoom` alone can't say this — the fit-to-window base scale
    /// depends on the canvas rect, which only the UI knows).
    ///
    /// The stroke pipeline runs entirely in cell pixels while the pen and the
    /// OS pointer work in screen pixels; this is the conversion factor that
    /// lets input conditioning be specified in what the user actually sees.
    pub view_scale: f32,
    /// Keep the brush a fixed size *on screen* instead of in document pixels,
    /// so the same hand gesture lays down the same-looking stroke at any zoom.
    /// Off by default: line weight is normally absolute in document pixels,
    /// which is what makes exports predictable.
    pub lock_brush_to_view: bool,
    /// Active non-drawing canvas gesture for the current drag, if any.
    pub nav_drag: Option<NavKind>,
    /// The held combo that started this drag's gesture, while it lasts.
    /// Letting go of it ends the gesture there, as Space-pan does in Krita.
    pub hold_latch: Option<shortcuts::KeyCombo>,
    /// The rest of this pen-down does nothing: its held gesture was let go,
    /// and drawing from wherever the pen happens to be would be a surprise.
    pub drag_inert: bool,

    pub playback: Playback,
    pub onion: OnionConfig,
    /// "Fade other layers": every layer but the active one drops to the
    /// `fade` amounts on the canvas. Session-only, so a launch never opens
    /// looking washed out; see [`AppState::layer_view_alpha`].
    pub fade_others: bool,
    pub fade: FadeOthers,
    /// Editing the active layer's transform writes a key on the current frame
    /// instead of moving the whole layer.
    pub auto_key_transform: bool,
    /// Drawing on a held frame breaks the hold into its own drawing first,
    /// instead of editing the cell shared with the rest of the hold.
    pub auto_key_draw: bool,

    /// Frames moved per FramePrev / FrameNext press, and per ◀ / ▶ click.
    /// Read through [`AppState::frame_step_delta`], which clamps it to >= 1 —
    /// a zero would turn frame navigation into a no-op.
    pub frame_step: usize,
    /// Invert the mouse-wheel scrub direction. Off: wheel down advances.
    pub invert_timeline_scroll: bool,
    /// What a middle-button drag on the canvas does.
    pub middle_drag: ButtonDrag,
    /// What a right-button drag on the canvas does.
    pub right_drag: ButtonDrag,
    /// A middle- or right-button drag under way on the canvas.
    pub button_drag: Option<ButtonDragState>,
    /// Line smoothing, latched into every new stroke. See `tools::Smoothing`.
    pub smoothing: SmoothingOptions,
    /// Whether the timeline wraps. Gates playback, wheel scrub and the frame
    /// step actions alike, so one toggle means one behaviour everywhere.
    pub loop_timeline: bool,
    /// Whether this stroke has already complained about a stray packet.
    /// One line per stroke is a report; one per packet is a flood.
    pub pen_outlier_logged: bool,
    /// Frames whose packet batch was thrown away because the newest packet
    /// disagreed with the cursor, and packets dropped as outliers within an
    /// accepted batch. Counted for the whole session and shown in the tablet
    /// readout: neither is visible while drawing, which is when they happen.
    pub pen_batches_rejected: u32,
    pub pen_packets_dropped: u32,
    /// Where the current stroke's positions come from, decided at pen-down.
    /// A pen stroke takes tablet packets only: the cursor is the same
    /// position rounded to a whole pixel, and slipping it in on a frame with
    /// no packet puts a half-pixel kink into the line — several canvas pixels
    /// once zoomed out. Cleared for good if the pen mapping stops agreeing
    /// with the cursor partway through.
    pub stroke_from_pen: bool,
    /// Samples the last stroke took from each source, for the tablet readout.
    /// A pen stroke should show none from the cursor.
    pub stroke_pen_samples: u32,
    pub stroke_mouse_samples: u32,
    /// Leftover trackpad scroll (in points) not yet worth a whole wheel
    /// notch. Session-only: a wheel gesture never spans a run. Mice report
    /// whole lines and bypass this entirely — see `ui::shell::wheel_map`.
    pub wheel_scrub_accum: f32,
    /// The canvas's screen rect as of the last frame, for what needs it
    /// before the canvas is laid out again: zooming the view on the wheel.
    pub canvas_rect: egui::Rect,

    pub bg_opacity: f32,
    /// Background clear color (RGB, 0..1).
    pub bg_color: [f32; 3],

    pub pen: PenInput,
    /// A pen recording under way (Tablet readout), and where the last one
    /// went — or why it couldn't be written.
    pub pen_record: Option<crate::input::pen_record::PenRecord>,
    pub pen_record_saved: Option<Result<std::path::PathBuf, String>>,
    /// What the last open of, or reload from, shortcuts.toml came to, for
    /// the Settings window: the file opened, edits loaded, or why not.
    pub shortcuts_file_note: Option<Result<String, String>>,

    pub history: History,
    /// Full-pixel snapshot of the cell before the in-flight stroke started.
    /// Used to extract the `before` slice for an undo command when the stroke
    /// finishes.
    stroke_pre_pixels: Vec<u8>,
    /// Whether `stroke_pre_pixels` holds the current edit's snapshot. A flag
    /// beside a kept buffer rather than an `Option<Vec>`: the buffer is reused,
    /// because allocating a fresh 100 MB one at every pen-down on a big layer
    /// was a hitch of its own.
    stroke_pre_live: bool,
    /// The cell, and its `Canvas::rev`, that `stroke_pre_pixels` still holds
    /// exactly: set when an edit lands, so the next edit of the same cell
    /// needs nothing copied at pen-down. On an 8K cell the copy was 10 ms,
    /// and with the unpack it forced, 40–60 ms.
    pre_mirror: Option<(CellId, u64)>,
    /// The cell `stroke_pre_pixels` was taken from for the edit in hand.
    pre_cell: Option<CellId>,
    /// Where `stroke_pre_pixels` may hold anything but zeros; `None` when it
    /// is all zeros. Snapshotting another cell clears just this.
    pre_inked: Option<DirtyRect>,
    /// Pixels per row of what `stroke_pre_pixels` holds.
    pre_w: u32,
    /// The drawing the pen would land on (`None`: a new blank one) and since
    /// when, while waiting to take its snapshot ahead of the pen. See
    /// `warm_pre`.
    warm_wait: Option<(Option<CellId>, f64)>,
    /// Under a stroke, where the target cell held anything before the
    /// stroke began (`None`: blank). With the stroke's own `Canvas::dirty`
    /// it bounds the drawing, so the texture can grow without a scan.
    stroke_base: Option<DirtyRect>,
    /// Reusable per-stroke coverage workspace for the ribbon rasterizer.
    stroke_ws: StrokeWorkspace,
    /// Canvas region updated by stroke flushes since the last texture sync;
    /// uploaded to the GPU via a partial texture update each frame.
    pub preview_upload_rect: Option<DirtyRect>,

    pub shortcuts: ShortcutMap,
    /// While `Some(action)`, the next key press from the user becomes the
    /// new binding for that action.
    pub rebinding: Option<Action>,
    /// Capture state while `rebinding` is a hold binding: bare modifiers are
    /// only known once they come back up.
    pub hold_capture: shortcuts::HoldCapture,
    /// Settings lists only the bindings that clash. Session-only.
    pub shortcut_clashes_only: bool,
    /// In-progress inline layer rename in the layers panel.
    pub layer_rename: Option<LayerRename>,
    /// Layers picked in the panel along with the active one (Ctrl- and
    /// Shift-click), by uid. Cleared when the active layer moves elsewhere.
    pub selected_layers: std::collections::BTreeSet<u64>,
    pub show_settings: bool,
    /// The brush settings window. Session-only, like `show_settings`:
    /// where a window sits persists through egui's own memory, but
    /// whether it is open should not outlive the session that opened it.
    pub show_brush_settings: bool,
    /// The active grid's settings window. Session-only, like the brush
    /// settings window.
    pub show_grid_settings: bool,
    /// Master visibility of all floating panel windows. Tab toggles it.
    pub show_panels: bool,
    /// Panels closed one at a time, from the Panels menu or the × in their
    /// title bar. `show_panels` still hides the rest all at once.
    pub hidden_panels: Vec<PanelId>,
    /// Which generation of panel windows egui is keeping positions and sizes
    /// for. Reset panel layout moves to a new one, so every panel opens as if
    /// for the first time; 0 is the ids panels have always had.
    pub panel_layout_gen: u32,
    /// Settings > Appearance. Applied with [`AppState::apply_scheme`].
    pub color_scheme: Scheme,
    pub custom_colors: BaseColors,
    /// Minimal timeline bar shown when `show_panels` is false. Has its own
    /// toggle shortcut so the user can hide it too.
    pub show_mini_timeline: bool,

    pub show_new_project: bool,
    pub new_project_cfg: NewProjectConfig,

    /// MP4 export settings dialog visibility + backing config.
    pub show_export: bool,
    pub export_cfg: ExportConfig,

    /// Frame-range import dialog (video / GIF / WebP) visibility + backing state.
    pub show_import_range: bool,
    pub import_range: Option<ImportRangeState>,

    /// Layer transform mode: when on, the canvas pan/zoom/rotate gestures move,
    /// scale and rotate the active layer instead of the canvas view.
    pub layer_xform: bool,
    /// True while the current drag is targeting the active layer's transform
    /// (decided at drag start) rather than the canvas view.
    pub nav_to_layer: bool,
    /// Timeline snapshot captured at the start of a layer-transform drag, used
    /// to push a single undo entry when the drag ends.
    layer_xform_before: Option<undo::TimelineState>,
    /// The same for an opacity drag in the Layers panel, which may change
    /// several layers at once.
    layer_opacity_before: Option<undo::TimelineState>,
    /// What a group being transformed turns and scales about, in document
    /// space: the middle of its layers when the drag started.
    group_pivot: Option<(f32, f32)>,
    /// Last (layer, frame) the transform edit buffer was synced for, so the
    /// buffer is only reloaded from keys when the cursor actually moves.
    xform_sync_last: Option<(usize, usize)>,

    /// Camera edit mode: when on, the canvas pan/zoom/rotate gestures move the
    /// camera instead of the canvas view. Same retarget trick as `layer_xform`.
    pub camera_edit: bool,
    /// True while the current drag is targeting the camera (decided at drag
    /// start) rather than the canvas view.
    pub nav_to_camera: bool,
    /// Timeline snapshot captured at the start of a camera drag, so the whole
    /// drag lands as one undo entry.
    camera_drag_before: Option<undo::TimelineState>,
    /// The camera's frame, while a handle of it is being dragged.
    pub cam_drag: Option<CamDrag>,
    /// Draw camera mode: every canvas drag draws a new frame for the camera
    /// and keys it on this frame. Never on with `camera_edit` — both claim
    /// the plain drag.
    pub camera_box: bool,
    /// The box, while one is being dragged out.
    pub cam_box: Option<CamBox>,
    /// A camera key, while it is being dragged along the timeline ruler.
    pub cam_key_drag: Option<CamKeyDrag>,
    /// Where the camera is, as a time in frames, while a move stretched over
    /// loops is playing — it has its own place then, apart from the frame
    /// showing. `None` the rest of the time. See `follow_stretched_camera`.
    shot_cam: Option<f32>,
    /// The frame playback started on, for Play/Pause to stop back on.
    /// Cleared by a pause that stays put.
    play_from: Option<usize>,
    /// Last frame the camera edit buffer was synced for.
    cam_sync_last: Option<usize>,
    /// Lock the editor viewport to the camera, so what you see is the shot.
    pub camera_look_through: bool,
    /// Draw the camera's frame rect over the canvas.
    pub show_camera_guide: bool,
    /// Dim everything the camera can't see.
    pub dim_outside_camera: bool,
    /// Draw thirds and the safe areas inside the camera's frame. A
    /// composition aid for the moment: off each launch.
    pub camera_overlay: bool,
    /// The camera frame size last set, kept between runs — the size itself
    /// is not. See [`AppState::set_camera_size`].
    pub last_camera_size: Option<(u32, u32)>,
    /// Outline the active layer's cell bounds, so it's visible where a layer's
    /// drawable buffer ends (strokes past it are silently clipped).
    pub show_layer_bounds: bool,
    /// Edit buffer for the "expand layer canvas" fields: `(layer, w, h)`.
    /// `None` (or a stale layer) means prefill from the active layer's size.
    pub expand_cfg: Option<(usize, u32, u32)>,

    /// Layer selected before the current one, for the "jump back" shortcut.
    /// Maintained by `track_layer_change` rather than by every site that
    /// assigns `current_layer` — there are eight of those. A uid, so adding
    /// or deleting layers around it doesn't lose it.
    prev_layer: Option<u64>,
    /// Previous frame's (current_layer, layer count), the baseline that
    /// `track_layer_change` diffs against.
    layer_watch: (usize, usize),
    /// Uid of the layer that was active last frame.
    layer_watch_uid: Option<u64>,

    /// Tracker tool: when on, each frame takes two clicks (point A then B) so
    /// stabilization can correct rotation/zoom shake as well as position.
    pub tracker_two_points: bool,
    /// Frame awaiting its second (B) tracker click in two-point mode.
    pub tracker_pending_b: Option<usize>,

    /// In-flight background import step (ffmpeg / decode), polled each frame.
    pub bg_job: Option<BgJob>,
    /// Label for the modal "busy" overlay while a `bg_job` runs.
    pub bg_label: Option<&'static str>,
    /// Cached range-dialog preview textures, keyed by source frame index.
    pub preview_tex: HashMap<usize, TextureHandle>,
    /// In-flight preview extraction (video only), with the frame it's for.
    pub preview_rx: Option<(usize, Receiver<Result<PreviewFrame>>)>,
    /// Defer dropping the preview texture handles until the next frame's top.
    /// Confirm/cancel run *inside* the egui pass, where the dialog has already
    /// emitted draw commands referencing those textures; freeing them in the
    /// same frame makes wgpu submit a render pass that references a destroyed
    /// texture (validation error → crash). Clearing one frame later, with the
    /// dialog gone, is safe.
    pub preview_clear_pending: bool,

    /// GPU max 2D texture dimension. Imported cells are capped to this so a
    /// huge source image can't exceed the limit and crash the texture upload.
    pub max_tex: u32,

    /// Viewport rect as of the last frame that drew panels. Compared each frame
    /// to detect a window resize, which re-sticks panels to their nearest edge.
    pub viewport_rect: Option<egui::Rect>,

    /// One-shot guard: native window chrome (rounded corners/border) applied.
    window_styled: bool,
    /// One-shot: maximize on the first frame. `ViewportBuilder::with_maximized`
    /// alone does not take on this frameless window — winit creates it at the
    /// requested inner size and the creation-time flag is lost. Sending the
    /// viewport command once the window exists goes through `set_maximized`,
    /// which does.
    startup_maximize: bool,

    /// Live screen colour-pick mode: while true, the pixel under the cursor is
    /// sampled from the OS framebuffer and the next tap commits it as the brush
    /// colour. The backdrop is left untouched, so what you see is what you pick:
    /// the canvas where it's opaque, whatever is behind the window where it
    /// isn't.
    pub screen_pick: bool,
    /// Consume the press that opened pick mode: wait for the pointer to be
    /// released once before a tap counts as a commit.
    pub screen_pick_arm: bool,
    /// Tool to restore after a screen-pick commits (so the tap returns the user
    /// to the drawing tool they were using).
    screen_pick_return_tool: ActiveTool,
    /// Live zoom-loupe texture, refreshed each frame while picking. Held here so
    /// the handle outlives the frame's paint list.
    pub screen_pick_tex: Option<TextureHandle>,
}

impl AppState {
    pub fn new(cc: &CreationContext<'_>) -> Self {
        crate::ui::theme::install(&cc.egui_ctx);
        // GPU max texture size — cap imported cells to this. Fall back to the
        // 8192 wgpu downlevel guarantee if the render state isn't available.
        let max_tex = cc
            .wgpu_render_state
            .as_ref()
            .map(|rs| rs.device.limits().max_texture_dimension_2d)
            .unwrap_or(8192)
            .max(2048);
        let prefs: UiPrefs = cc
            .storage
            .and_then(|s| eframe::get_value(s, UI_PREFS_KEY))
            .unwrap_or_default();
        let root = shortcuts::config_dir().map(|d| d.join("brushes"));
        let mut state = Self::with_prefs(prefs, max_tex, root);
        state.canvas_gpu = cc.wgpu_render_state.as_ref().and_then(CanvasGpu::new);
        state.apply_scheme(&cc.egui_ctx);
        state
    }

    /// A state with default preferences and no window, for driving UI code
    /// from tests. `CreationContext` can't be built outside eframe.
    #[cfg(test)]
    pub fn for_test() -> Self {
        Self::with_prefs(UiPrefs::default(), 8192, None)
    }

    /// Everything `new` does that doesn't need the window.
    /// `brush_root`: where imported Krita presets live (none in tests).
    fn with_prefs(prefs: UiPrefs, max_tex: u32, brush_root: Option<PathBuf>) -> Self {
        let project = Project::new(1280, 720, 24.0);
        let (shortcuts, shortcuts_err) = shortcuts::load();
        let mut cell_dirty = HashMap::new();
        for id in 0..project.cells.len() {
            cell_dirty.insert(id, Dirty::Full);
        }
        Self {
            project,
            project_path: None,
            save_toast: None,
            save_error: None,
            save_job: None,
            krita: None,
            krita_send: None,
            krita_toast: None,
            krita_path: prefs.krita_path.clone(),
            cell_textures: HashMap::new(),
            ghost_textures: HashMap::new(),
            ghost_stale: HashSet::new(),
            retired_textures: Vec::new(),
            canvas_gpu: None,
            fresh_full: HashSet::new(),
            cell_dirty,
            sync_frame: 0,
            cell_tex_used: HashMap::new(),
            clip_textures: HashMap::new(),
            clip_tex_used: HashMap::new(),
            group_images: HashMap::new(),
            group_used: HashMap::new(),
            ghost_tex_used: HashMap::new(),
            playhead_rest: (usize::MAX, 0.0),
            tool: ActiveTool::Pencil,
            brush: restore_tool_brushes(prefs.tool_brushes.clone())[ActiveTool::Pencil.idx()]
                .clone(),
            brush_library: BrushLibrary::new(brush_root),
            brush_search: String::new(),
            brush_thumbs: HashMap::new(),
            krita_import: None,
            palette: prefs.palette,
            harmony: {
                let c = restore_tool_brushes(prefs.tool_brushes.clone())[ActiveTool::Pencil.idx()].color;
                HarmonySet::new(prefs.harmony, [c[0], c[1], c[2]])
            },
            project_presets: prefs.project_presets,
            cell_clip: None,
            track_sel: BTreeSet::new(),
            track_anchor: None,
            track_drag: None,
            track_scroll: egui::Vec2::ZERO,
            track_frame_w: prefs.track_frame_w,
            track_timing_n: 2,
            gc_key: None,
            blank_cache: HashMap::new(),
            blank_scans_left: 0,
            sel_mask: None,
            sel_ver: 0,
            sel_outline: Vec::new(),
            clip_cache: None,
            sel_gesture: None,
            sel_op: SelOp::Replace,
            sel_all_frames: false,
            sel_shape: prefs.sel_shape,
            wand: prefs.wand,
            wand_drag: None,
            wand_repick: None,
            fill_mode: prefs.fill_mode,
            fill_enclose: prefs.fill_enclose,
            fill_behind: prefs.fill_behind,
            fill_lasso: None,
            sel_amount: prefs.sel_amount.max(1),
            tint_outside: prefs.tint_outside,
            sel_tint: None,
            selection: None,
            liquify: None,
            float_pre: Vec::new(),
            float_pre_live: false,
            float_lift_rect: None,
            sel_drag: None,
            pixel_clip: None,
            sel_tex_stale: false,
            selection_tex: None,
            tool_brushes: restore_tool_brushes(prefs.tool_brushes.clone()),
            last_pen: ActiveTool::Pencil,
            wheel_pinned: false,
            stroke: None,
            stroke_target: None,
            shape_drag: None,
            fill_drag: None,
            shift_held: false,
            alt_held: false,
            perspective: {
                let mut cfg = prefs.perspective.clone();
                cfg.ensure_ids();
                cfg
            },
            grid_drag: None,
            grid_drag_before: None,
            grid_sync_last: None,
            grid_files: prefs.grid_files.clone(),
            grid_seed: prefs.perspective.grids.clone(),
            snap_lock: None,
            view: View::default(),
            view_scale: 1.0,
            lock_brush_to_view: prefs.lock_brush_to_view,
            nav_drag: None,
            hold_latch: None,
            drag_inert: false,
            playback: Playback::default(),
            onion: prefs.onion,
            fade_others: false,
            fade: prefs.fade,
            auto_key_transform: prefs.auto_key_transform,
            auto_key_draw: prefs.auto_key_draw,
            frame_step: prefs.frame_step,
            invert_timeline_scroll: prefs.invert_timeline_scroll,
            middle_drag: prefs.middle_drag,
            right_drag: prefs.right_drag,
            button_drag: None,
            loop_timeline: prefs.loop_timeline,
            smoothing: prefs.smoothing,
            pen_outlier_logged: false,
            pen_batches_rejected: 0,
            pen_packets_dropped: 0,
            stroke_from_pen: false,
            stroke_pen_samples: 0,
            stroke_mouse_samples: 0,
            wheel_scrub_accum: 0.0,
            canvas_rect: egui::Rect::NOTHING,
            bg_opacity: prefs.bg_opacity,
            bg_color: prefs.bg_color,
            pen: PenInput::new(),
            pen_record: None,
            pen_record_saved: None,
            shortcuts_file_note: shortcuts_err
                .map(|e| Err(format!("shortcuts.toml didn't load, defaults in use: {e}"))),
            history: History::default(),
            stroke_pre_pixels: Vec::new(),
            stroke_pre_live: false,
            pre_mirror: None,
            pre_cell: None,
            pre_inked: None,
            pre_w: 0,
            warm_wait: None,
            stroke_base: None,
            stroke_clock: None,
            stroke_ws: StrokeWorkspace::new(),
            preview_upload_rect: None,
            shortcuts,
            rebinding: None,
            hold_capture: Default::default(),
            shortcut_clashes_only: false,
            layer_rename: None,
            selected_layers: Default::default(),
            show_settings: false,
            show_brush_settings: false,
            show_grid_settings: false,
            show_panels: prefs.show_panels,
            hidden_panels: prefs.hidden_panels.into_iter().filter(|&p| p != PanelId::Color).collect(),
            panel_layout_gen: prefs.panel_layout_gen,
            color_scheme: prefs.color_scheme,
            custom_colors: prefs.custom_colors,
            show_mini_timeline: prefs.show_mini_timeline,
            show_new_project: false,
            new_project_cfg: NewProjectConfig::default(),
            show_export: false,
            export_cfg: ExportConfig::default(),
            show_import_range: false,
            import_range: None,
            layer_xform: false,
            nav_to_layer: false,
            layer_xform_before: None,
            layer_opacity_before: None,
            group_pivot: None,
            xform_sync_last: None,
            camera_edit: false,
            nav_to_camera: false,
            camera_drag_before: None,
            cam_drag: None,
            camera_box: false,
            cam_box: None,
            cam_key_drag: None,
            shot_cam: None,
            play_from: None,
            cam_sync_last: None,
            camera_look_through: false,
            show_camera_guide: prefs.show_camera_guide,
            dim_outside_camera: prefs.dim_outside_camera,
            camera_overlay: false,
            last_camera_size: prefs.last_camera_size,
            show_layer_bounds: prefs.show_layer_bounds,
            expand_cfg: None,
            prev_layer: None,
            layer_watch: (0, 1),
            layer_watch_uid: None,
            tracker_two_points: false,
            tracker_pending_b: None,
            bg_job: None,
            bg_label: None,
            preview_tex: HashMap::new(),
            preview_rx: None,
            preview_clear_pending: false,
            max_tex,
            viewport_rect: None,
            window_styled: false,
            startup_maximize: true,
            screen_pick: false,
            screen_pick_arm: false,
            screen_pick_return_tool: ActiveTool::Pencil,
            screen_pick_tex: None,
        }
    }

    /// Reset the project and all editing state back to a fresh start.
    /// Keeps user preferences (shortcuts) and hardware state (pen).
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.reset_with(1280, 720, 24.0);
    }

    pub fn reset_with(&mut self, width: u32, height: u32, fps: f32) {
        // Let any write of the outgoing project land first. Otherwise it would
        // report success *after* the swap and set `project_path` back to the
        // old file — the exact overwrite the reset below is guarding against.
        self.finish_pending_save();
        self.stop_krita_link();
        self.stash_grids();
        self.project = Project::new(width, height, fps);
        // Forget the old file, or the next Save silently overwrites the project
        // the user just navigated away from.
        self.project_path = None;
        self.unstash_grids(None);
        self.save_toast = None;
        self.save_error = None;
        self.retire_cell_textures();
        self.retire_all_ghosts();
        self.cell_dirty.clear();
        self.clear_track_state();
        for id in 0..self.project.cells.len() {
            self.cell_dirty.insert(id, Dirty::Full);
        }
        // Brushes are the artist's, not the project's: keep every tool's,
        // the active one included, and come back on the pencil.
        self.stash_brush();
        self.load_brush(ActiveTool::Pencil);
        self.grid_drag = None;
        self.snap_lock = None;
        self.stroke = None;
        self.stroke_target = None;
        self.shape_drag = None;
        self.fill_drag = None;
        self.clear_selection_state();
        self.view = View::default();
        // Republished by the canvas next frame; kept in step with `view` so a
        // reset never leaves a stale scale behind for one frame of input.
        self.view_scale = 1.0;
        self.nav_drag = None;
        self.playback = Playback::default();
        // `onion` and the backdrop are deliberately not reset: like
        // `show_panels` they are workspace preferences, and resetting them
        // here is what made tuned settings feel like they never stuck.
        self.history = History::default();
        self.gc_key = None;
        self.stroke_pre_live = false;
        self.preview_upload_rect = None;
        self.rebinding = None;
        self.layer_rename = None;
        self.show_settings = false;
        self.show_brush_settings = false;
        self.show_grid_settings = false;
        // `show_panels` / `show_mini_timeline` are deliberately not reset here.
        // They're preferences that persist across runs, like `shortcuts` — a new
        // project shouldn't shove hidden panels back on screen.
        self.new_project_cfg = NewProjectConfig {
            width,
            height,
            fps,
            ..Default::default()
        };
        self.show_export = false;
        self.show_import_range = false;
        self.import_range = None;
        self.layer_xform = false;
        self.nav_to_layer = false;
        self.layer_xform_before = None;
        self.layer_opacity_before = None;
        self.xform_sync_last = None;
        // Camera guide / dim / bounds are view preferences, kept like the panel
        // toggles above; only the transient edit + lock modes reset.
        self.camera_edit = false;
        self.nav_to_camera = false;
        self.camera_drag_before = None;
        self.cam_drag = None;
        self.camera_box = false;
        self.cam_box = None;
        self.cam_key_drag = None;
        self.shot_cam = None;
        self.cam_sync_last = None;
        self.camera_look_through = false;
        self.expand_cfg = None;
        self.prev_layer = None;
        self.layer_watch = (self.project.current_layer, self.project.layers.len());
        self.layer_watch_uid = self.project.layers.get(self.project.current_layer).map(|l| l.uid);
        self.tracker_pending_b = None;
        self.bg_job = None;
        self.bg_label = None;
        self.preview_tex.clear();
        self.preview_rx = None;
    }

    /// Open the New project dialog on the starred preset, or on the current
    /// project's size when none is starred.
    pub fn open_new_project(&mut self) {
        let (width, height, fps) = match self.project_presets.iter().find(|p| p.is_default) {
            Some(p) => (p.width, p.height, p.fps),
            None => (self.project.width, self.project.height, self.project.fps),
        };
        self.new_project_cfg = NewProjectConfig {
            width,
            height,
            fps,
            ..Default::default()
        };
        self.show_new_project = true;
    }

    /// The saved preset called `name`, ignoring case and surrounding space.
    pub fn preset_named(&self, name: &str) -> Option<usize> {
        let name = name.trim().to_lowercase();
        self.project_presets
            .iter()
            .position(|p| p.name.to_lowercase() == name)
    }

    /// Save the dialog's size under the name in its name box. A name already
    /// in use is replaced in place, keeping its place in the list and its
    /// star. False for a blank name.
    pub fn save_preset(&mut self) -> bool {
        let name = self.new_project_cfg.preset_name.trim().to_string();
        if name.is_empty() {
            return false;
        }
        let cfg = &self.new_project_cfg;
        let (width, height, fps) = (cfg.width.max(1), cfg.height.max(1), cfg.fps.max(1.0));
        match self.preset_named(&name) {
            Some(i) => {
                let p = &mut self.project_presets[i];
                (p.name, p.width, p.height, p.fps) = (name, width, height, fps);
            }
            None => self.project_presets.push(ProjectPreset {
                name,
                width,
                height,
                fps,
                is_default: false,
            }),
        }
        self.new_project_cfg.preset_name.clear();
        true
    }

    /// Fill the dialog from a saved preset exactly as saved — unlike the
    /// built-in sizes, it keeps the orientation it was saved in.
    pub fn apply_preset(&mut self, i: usize) {
        if let Some(p) = self.project_presets.get(i) {
            let cfg = &mut self.new_project_cfg;
            (cfg.width, cfg.height, cfg.fps) = (p.width, p.height, p.fps);
        }
    }

    /// Rename a saved preset. False for a blank name or one another preset
    /// already has; changing only the case of its own name is fine.
    pub fn rename_preset(&mut self, i: usize, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || i >= self.project_presets.len() {
            return false;
        }
        if self.preset_named(name).is_some_and(|j| j != i) {
            return false;
        }
        self.project_presets[i].name = name.to_string();
        true
    }

    pub fn delete_preset(&mut self, i: usize) {
        if i < self.project_presets.len() {
            self.project_presets.remove(i);
            // The rename's index may now point at a different preset.
            self.new_project_cfg.preset_rename = None;
        }
    }

    /// Star a preset so New project opens on it, unstarring any other.
    /// Starring the starred one clears it.
    pub fn toggle_default_preset(&mut self, i: usize) {
        let Some(on) = self.project_presets.get(i).map(|p| !p.is_default) else {
            return;
        };
        for (j, p) in self.project_presets.iter_mut().enumerate() {
            p.is_default = on && j == i;
        }
    }

    /// A sample at a cell-space position, taking pressure and tilt from the
    /// most recent tablet packet. Used for the mouse path and for stroke
    /// start; the tablet's own packets carry their own dynamics and go through
    /// [`AppState::make_pen_sample`] instead.
    pub fn make_sample(&self, x: f32, y: f32, t: f32) -> PointerSample {
        let (pressure, tilt_x, tilt_y) = match self.pen.packets().last() {
            Some(p) => (p.pressure, p.tilt_x, p.tilt_y),
            // No packet this frame: the last known pressure, or 1.0 once the
            // pen has gone idle and the mouse has taken over.
            None => (self.pen.current_pressure().unwrap_or(1.0), 0.0, 0.0),
        };
        PointerSample {
            x,
            y,
            pressure,
            tilt_x,
            tilt_y,
            t,
            time_ms: t as f64 * 1000.0,
            ..Default::default()
        }
    }

    /// A sample built from one tablet packet, at an already-mapped cell-space
    /// position. Every packet carries its own pressure and tilt, which is the
    /// point of walking the queue instead of sampling it once per frame.
    pub fn make_pen_sample(&self, x: f32, y: f32, t: f32, p: &PenPacket) -> PointerSample {
        PointerSample {
            x,
            y,
            pressure: p.pressure,
            tilt_x: p.tilt_x,
            tilt_y: p.tilt_y,
            t,
            // The driver's own clock when it has one: packets arrive in
            // per-frame batches, so frame time would make them simultaneous.
            time_ms: if p.time_ms != 0.0 { p.time_ms } else { t as f64 * 1000.0 },
            rotation: p.rotation,
            tangential: p.tangential,
            ..Default::default()
        }
    }

    /// How the active cell is turned on screen: its rotation in degrees,
    /// clockwise, with any mirroring factored out, and whether the view is
    /// mirrored on each axis. This is Krita's "canvas rotation": its tilt
    /// sensors measure the pen on screen, and a Krita brush corrects by it.
    pub fn cell_screen_orientation(&self) -> (f64, (bool, bool)) {
        let view_rot = if self.camera_look_through {
            -self.display_camera(self.project.current_frame).rot
        } else {
            self.view.rotation
        };
        let layer_rot = self
            .display_transform(self.project.current_layer, self.project.current_frame)
            .rot;
        // A mirror turns the layer's rotation the other way on screen.
        let flipped_once = self.view.flip_x != self.view.flip_y;
        let rot = view_rot + if flipped_once { -layer_rot } else { layer_rot };
        ((rot as f64).to_degrees(), (self.view.flip_x, self.view.flip_y))
    }

    /// Screen pixels per *active-cell* pixel: the view scale folded with the
    /// active layer's own scale, since strokes are rasterized in cell space.
    /// Mirrors the `scale * layer_scale` product the canvas uses for previews.
    pub fn cell_view_scale(&self) -> f32 {
        let t = self.display_transform(self.project.current_layer, self.project.current_frame);
        (self.view_scale * t.scale.abs()).max(1e-6)
    }

    /// The Krita preset the current brush paints with: the library's, if
    /// it names one the library has, else the built-in one.
    pub fn krita_brush(&self) -> Arc<crate::tools::krita::PixelBrush> {
        if let Some(id) = self.brush.krita_id.as_deref() {
            // An edit that no longer builds paints the preset unedited.
            if let Some(b) = self.brush_library.brush_with(id, &self.brush.krita_overrides).ok().or_else(|| self.brush_library.brush(id)) {
                return b;
            }
        }
        self.brush.krita.brush()
    }

    /// Krita's stabilizer paints on a timer, not only when the pen moves:
    /// give a stabilized stroke the time and paint what it adds. Returns
    /// whether one is running, so the caller keeps frames coming.
    pub fn tick_stroke(&mut self) -> bool {
        let (Some(target), Some(t0)) = (self.stroke_target, self.stroke_clock) else { return false };
        let Some(builder) = self.stroke.as_mut().filter(|b| b.stabilized()) else { return false };
        builder.tick(t0.elapsed().as_secs_f64() * 1000.0);
        self.flush_stroke(target);
        true
    }

    /// Open the Import Krita brushes window, which starts reading Krita's
    /// presets. Already open: left as it is.
    pub fn open_krita_import(&mut self) {
        if self.krita_import.is_some() {
            return;
        }
        let exe = krita_link::find_krita(self.krita_path.as_deref());
        let places = crate::brush_library::import::KritaPlaces::find(exe.as_deref());
        let root = self.brush_library.root().map(|p| p.to_path_buf());
        self.krita_import = Some(crate::brush_library::import::KritaImport::start(places, root));
    }

    /// Close the Import Krita brushes window. The window drew its thumbnails
    /// this frame, so they are parked for the next one rather than dropped
    /// with it — see `retired_textures`.
    pub fn close_krita_import(&mut self) {
        if let Some(mut imp) = self.krita_import.take() {
            self.retired_textures.extend(imp.thumbs.drain().map(|(_, t)| t));
        }
    }

    /// Re-read the imported brushes after an import, dropping thumbnails
    /// a reused folder name would otherwise keep.
    pub fn rescan_brushes(&mut self) {
        self.brush_library.rescan();
        self.forget_user_brush_thumbs();
    }

    /// Have the picker load the imported brushes' thumbnails afresh. The old
    /// ones are parked, not dropped: Brush settings, open beside whatever
    /// asked for this, may have drawn them this frame — see
    /// `retired_textures`.
    pub fn forget_user_brush_thumbs(&mut self) {
        let user: Vec<String> =
            self.brush_thumbs.keys().filter(|id| id.starts_with("user/")).cloned().collect();
        for id in user {
            self.retired_textures.extend(self.brush_thumbs.remove(&id));
        }
    }

    /// Brush radius in active-cell pixels. With `lock_brush_to_view` the stored
    /// radius means *screen* pixels, so it is divided back out by the current
    /// scale — the brush then keeps a constant on-screen footprint and the same
    /// gesture draws the same stroke at any zoom.
    pub fn effective_radius(&self) -> f32 {
        if self.lock_brush_to_view {
            (self.brush.radius / self.cell_view_scale()).clamp(0.1, 4096.0)
        } else {
            self.brush.radius
        }
    }

    /// Note that cell `id`'s pixels changed. The region comes from the canvas
    /// itself: every pixel edit resets `Canvas::dirty` when it starts and
    /// grows it as it writes, so it is exactly what this edit touched. A canvas
    /// without one is taken as changed everywhere.
    pub fn mark_dirty(&mut self, id: CellId) {
        let region = match self.project.cell(id).and_then(|c| c.dirty) {
            Some(r) => Dirty::Rect(r),
            None => Dirty::Full,
        };
        self.mark_region(id, region);
    }

    /// Note that `region` of cell `id`'s pixels changed. See `mark_dirty`.
    fn mark_region(&mut self, id: CellId, region: Dirty) {
        let pending = self.cell_dirty.get(&id).copied().unwrap_or(Dirty::Full);
        self.cell_dirty.insert(id, pending.with(region));
        // Clipped drawings made from this cell — as the clipped drawing or as
        // its base — need the same region redone.
        // A base edit can't grow the clipped drawing, so what it redoes is
        // kept inside the texture: anything in `pending` past it is the
        // drawing itself spreading, which re-crops.
        for (&(c, base), t) in self.clip_textures.iter_mut() {
            if c == id {
                t.pending = t.pending.with(region);
            } else if base == groups::BaseKey::Cell(id) {
                let inside = match region {
                    Dirty::Clean => None,
                    Dirty::Rect(r) if t.aligned => rect_meet(r, t.rect),
                    // Not pixel for pixel: the whole texture (none if blank).
                    _ => rect_meet(t.rect, t.rect),
                };
                t.pending = t.pending.with(inside.map_or(Dirty::Clean, Dirty::Rect));
            }
        }
        self.mark_group_region(id, region);
        self.blank_cache.remove(&id);
        // The ghost is baked from these pixels, so it needs rebuilding — but
        // re-uploaded in place, never freed. See `ghost_stale`.
        if self.ghost_textures.contains_key(&id) {
            self.ghost_stale.insert(id);
        }
    }

    /// Free the pixels of every cell nothing can show again: on no layer, out
    /// of undo's and redo's reach, and not something an edit or the Krita
    /// link is holding. The pool only ever grew before — deleted keys and
    /// layers, drawings keyed over — and on a 2160×2880 project the dead
    /// cells came to 2.3 GB. A freed cell becomes a tombstone, so no id moves.
    ///
    /// Runs when the history or the pool changed. Skipped while a Krita send
    /// or pull is in flight: those match drawings to cells by id.
    fn collect_cells(&mut self) {
        let busy = self.krita_send.is_some()
            || self.krita.as_ref().is_some_and(|k| k.pending.is_some() || k.round.is_some());
        let key = (self.history.revision(), self.project.cells.len());
        if busy || self.gc_key == Some(key) {
            return;
        }
        self.gc_key = Some(key);
        let mut used = vec![false; self.project.cells.len()];
        let mut keep = |id: CellId| {
            if let Some(u) = used.get_mut(id) {
                *u = true;
            }
        };
        for l in &self.project.layers {
            l.exposures.iter().flatten().for_each(|&id| keep(id));
        }
        self.history.for_each_cell(&mut keep);
        self.stroke_target.into_iter().for_each(&mut keep);
        self.selection.as_ref().map(|s| s.cell).into_iter().for_each(&mut keep);
        // The link maps Krita's drawings, old ones included, onto cells.
        let link = self.krita.as_ref().and_then(|k| k.baseline.as_ref());
        for &id in link.into_iter().flat_map(|b| b.layers.iter().flat_map(|l| l.by_hash.values())) {
            keep(id);
        }
        for (id, used) in used.into_iter().enumerate() {
            if used || self.project.cells[id].is_tombstone() {
                continue;
            }
            self.project.cells[id] = Arc::new(Canvas::tombstone());
            if let Some(t) = self.cell_textures.remove(&id) {
                self.retired_textures.push(t.tex);
            }
            self.cell_tex_used.remove(&id);
            self.cell_dirty.remove(&id);
            self.retire_ghost(id);
            self.ghost_tex_used.remove(&id);
            self.retire_group_images(Some(id));
            let gone: Vec<_> = self
                .clip_textures
                .keys()
                .copied()
                .filter(|&(c, b)| c == id || b == groups::BaseKey::Cell(id))
                .collect();
            for key in gone {
                if let Some(t) = self.clip_textures.remove(&key) {
                    self.retired_textures.push(t.tex);
                }
                self.clip_tex_used.remove(&key);
            }
            self.blank_cache.remove(&id);
        }
    }

    /// Pack the cells nobody is drawing on (see `Canvas`). A write unpacks a
    /// cell and it stays so while in use — the stroke's, the active slot's,
    /// the floating selection's; once it isn't, this puts it back to just
    /// its drawing. Within `PACK_BUDGET` a frame, so a bulk edit that
    /// unpacked dozens packs over a few frames instead of in one hitch.
    fn pack_idle_cells(&mut self) {
        let start = Instant::now();
        let p = &self.project;
        let active = p.layers.get(p.current_layer).and_then(|l| l.resolve(p.current_frame));
        let hot = [self.stroke_target, active, self.selection.as_ref().map(|s| s.cell)];
        for (id, cell) in self.project.cells.iter_mut().enumerate() {
            if cell.is_packed() || hot.contains(&Some(id)) {
                continue;
            }
            // Shared with a save in flight or an undo step: packing would
            // copy it first. It comes round again once they let go.
            let Some(c) = Arc::get_mut(cell) else {
                continue;
            };
            c.pack();
            if start.elapsed() >= PACK_BUDGET {
                break;
            }
        }
    }

    /// Park one ghost for release on the next sync. See `retired_textures`.
    fn retire_ghost(&mut self, id: CellId) {
        self.ghost_stale.remove(&id);
        if let Some((_, tex)) = self.ghost_textures.remove(&id) {
            self.retired_textures.push(tex);
        }
    }

    /// Park every cell texture for release on the next sync. Used by File →
    /// New and File → Open, which discard the whole cell pool mid-frame.
    /// See `retired_textures`.
    fn retire_cell_textures(&mut self) {
        self.retired_textures
            .extend(self.cell_textures.drain().map(|(_, t)| t.tex));
        self.cell_tex_used.clear();
        self.retired_textures
            .extend(self.clip_textures.drain().map(|(_, t)| t.tex));
        self.clip_tex_used.clear();
        self.retired_textures
            .extend(self.group_images.drain().filter_map(|(_, g)| g.tex));
        self.group_used.clear();
    }

    /// Park every ghost for release on the next sync. See `retired_textures`.
    fn retire_all_ghosts(&mut self) {
        self.ghost_stale.clear();
        self.retired_textures
            .extend(self.ghost_textures.drain().map(|(_, (_, tex))| tex));
        self.ghost_tex_used.clear();
    }

    /// Re-upload what an undo or redo changed, and nothing else. An edit that
    /// only moved the timeline changed no pixels, so it re-uploads nothing.
    fn apply_touched(&mut self, touched: Option<undo::Touched>) {
        match touched {
            Some(undo::Touched::Cell(id)) => self.mark_dirty(id),
            // Wholesale: what the canvas says it touched may be stale — a
            // buffer the history handed back can't be written to say so.
            Some(undo::Touched::Cells(ids)) => {
                for id in ids {
                    self.mark_region(id, Dirty::Full);
                }
            }
            Some(undo::Touched::Selection(m)) => self.set_mask(m, false),
            Some(undo::Touched::Grid(g)) => self.restore_grid(&g),
            Some(undo::Touched::Many(all)) => {
                for t in all {
                    self.apply_touched(Some(t));
                }
            }
            Some(undo::Touched::Structure) | None => {}
        }
    }

    /// Frames moved or inserted by one step action, from the user's step size.
    /// `max(1)` so a step of zero — from a cleared input or a stale prefs blob
    /// — still does something instead of silently doing nothing.
    pub fn frame_step_count(&self) -> usize {
        self.frame_step.max(1)
    }

    /// Signed form of [`AppState::frame_step_count`], for `Project::step`.
    pub fn frame_step_delta(&self) -> isize {
        self.frame_step_count() as isize
    }

    /// Run a timeline/layer edit while recording it on the undo stack.
    ///
    /// `capture_cells` snapshots every cell's pixels before/after — needed only
    /// for the destructive "delete the last frame" path, which wipes pixels.
    /// All other structural edits leave the cell pool intact, so a cheap
    /// `TimelineState` snapshot is enough.
    pub fn structural_edit(&mut self, capture_cells: bool, edit: impl FnOnce(&mut Project)) {
        // Floating pixels belong to a cell that may be about to move.
        self.land_float();
        // Keys may move under the track selection; the track edits that want
        // one re-select after this returns.
        self.track_sel.clear();
        let before = undo::TimelineState::capture(&self.project);
        // Shared handles, not copies: a cell the edit writes to is copied on
        // that write, so a changed pointer is a changed cell.
        let cells_before: Vec<Arc<Canvas>> = if capture_cells {
            self.project.cells.clone()
        } else {
            Vec::new()
        };

        edit(&mut self.project);

        let after = undo::TimelineState::capture(&self.project);
        let cell_pixels: Vec<undo::CellPixelDelta> = cells_before
            .into_iter()
            .enumerate()
            .filter_map(|(cell, before)| {
                let after = self.project.cells.get(cell)?;
                (!Arc::ptr_eq(&before, after)).then(|| undo::CellPixelDelta {
                    cell,
                    before,
                    after: Arc::clone(after),
                })
            })
            .collect();

        // Only cells whose pixels this edit actually changed need a new
        // texture. The rest are exactly as uploaded — the timeline around them
        // moved, not them.
        let changed: Vec<CellId> = cell_pixels.iter().map(|d| d.cell).collect();
        self.history.push(undo::Command::Structural {
            before,
            after,
            cell_pixels,
        });
        for id in changed {
            self.mark_dirty(id);
        }
    }

    /// Insert `cells` as a brand-new layer *below* the active layer, recorded as
    /// a single undoable structural edit. The active layer is never modified.
    ///
    /// `on_all_frames` keys one cell at frame 0 (so it shows on every frame via
    /// hold resolution). Otherwise each cell becomes its own consecutive
    /// timeline frame from frame 0, growing the timeline if the run is longer
    /// than the current frame count.
    pub fn import_cells_as_layer(
        &mut self,
        name: impl Into<String>,
        cells: Vec<Canvas>,
        on_all_frames: bool,
    ) {
        if cells.is_empty() {
            return;
        }
        let name = name.into();
        // Cap each cell to the GPU texture limit so a huge source can't crash
        // the upload (keeps full res up to the device max, aspect preserved).
        let max_tex = self.max_tex;
        let cells: Vec<Canvas> = cells.into_iter().map(|c| cap_canvas(c, max_tex)).collect();
        // Initial transform fits the (native-resolution) cell within the canvas
        // without upscaling, centered. The user can scale up from here to crop.
        let init_xform = self.fit_transform(cells[0].width as f32, cells[0].height as f32);
        self.structural_edit(false, move |p| {
            if on_all_frames {
                let idx = p.add_layer_below_active(name);
                p.layers[idx].transform = init_xform;
                let id = p.cells.len();
                p.cells.push(cells.into_iter().next().unwrap().into());
                p.layers[idx].set_key(0, id);
            } else {
                p.ensure_frame_count(cells.len());
                let idx = p.add_layer_below_active(name);
                p.layers[idx].transform = init_xform;
                for (f, canvas) in cells.into_iter().enumerate() {
                    let id = p.cells.len();
                    p.cells.push(canvas.into());
                    p.layers[idx].set_key(f, id);
                }
            }
        });
    }

    /// True when the active layer can be merged onto the layer below: there is
    /// a layer below, and neither layer is locked or a reference layer.
    pub fn can_merge_down(&self) -> bool {
        let li = self.project.current_layer;
        if li == 0 {
            return false;
        }
        // Both editable drawing layers, in the same group or both in none.
        self.project.paintable(li)
            && self.project.paintable(li - 1)
            && self.project.group_of(li) == self.project.group_of(li - 1)
    }

    /// Merge the active layer onto the layer below and remove it.
    ///
    /// Both layers' per-frame transforms and opacities are baked into fresh
    /// doc-sized cells, so the merged layer ends up with an identity transform
    /// at full opacity. Only *creates* cells — existing cells are untouched, so
    /// a cheap `TimelineState` snapshot is a correct undo (undo leaves the new
    /// cells orphaned in the pool, same as layer import).
    pub fn merge_layer_down(&mut self) {
        if !self.can_merge_down() {
            return;
        }
        self.playback.stop();
        let li = self.project.current_layer;
        let bi = li - 1;
        let p = &self.project;
        let (pw, ph) = (p.width, p.height);
        let top = &p.layers[li];
        let below = &p.layers[bi];
        // A clipped top layer merges as it shows, cut to its base, so the
        // base's own changes are frames to bake too. The layer below keeps
        // its pixels whole: it keeps its own clipping.
        let base = p.clip_base(li).map(|b| &p.layers[b]);

        // Interpolated transforms change the picture on every frame; otherwise
        // only exposure keys (and a lone transform key's frame) matter.
        let animated = top.transform_keys.len() >= 2
            || below.transform_keys.len() >= 2
            || base.is_some_and(|b| b.transform_keys.len() >= 2);
        let mut bake_frames: Vec<usize> = if animated {
            (0..p.frame_count).collect()
        } else {
            let mut fs: Vec<usize> = (0..p.frame_count)
                .filter(|&f| {
                    [Some(top), Some(below), base]
                        .into_iter()
                        .flatten()
                        .any(|l| l.exposures.get(f).copied().flatten().is_some())
                })
                .collect();
            for l in [Some(top), Some(below), base].into_iter().flatten() {
                for k in &l.transform_keys {
                    fs.push(k.frame.min(p.frame_count.saturating_sub(1)));
                }
            }
            fs.push(0);
            fs.sort_unstable();
            fs.dedup();
            fs
        };
        bake_frames.retain(|&f| f < p.frame_count);

        // Off-frame artwork is a first-class thing now (see `doc::camera`), so
        // the baked cell has to be big enough to hold both layers wherever they
        // sit — baking into the doc rect would silently crop a layer parked
        // outside the camera. The bake is placed with an identity transform, so
        // it stays centred on the doc centre and only a half-extent is needed.
        let (pwf, phf) = (pw as f32, ph as f32);
        let (mut hx, mut hy) = (pwf * 0.5, phf * 0.5);
        for l in [top, below] {
            for &f in &bake_frames {
                let Some(id) = l.resolve(f) else { continue };
                let Some(src) = p.cell(id) else { continue };
                let t = l.resolve_transform(f);
                let (cw, ch) = (src.width as f32, src.height as f32);
                for (u, v) in [(0.0, 0.0), (cw, 0.0), (cw, ch), (0.0, ch)] {
                    let (x, y) = t.cell_to_doc(u, v, cw, ch, pwf, phf);
                    hx = hx.max((x - pwf * 0.5).abs());
                    hy = hy.max((y - phf * 0.5).abs());
                }
            }
        }
        let cap = self.max_tex as f32 * 0.5;
        let bw = ((hx.min(cap).ceil() as u32) * 2).max(pw);
        let bh = ((hy.min(cap).ceil() as u32) * 2).max(ph);

        let baked: Vec<(usize, Canvas)> = bake_frames
            .into_iter()
            .map(|f| {
                let mut out = Canvas::new(bw, bh);
                for (idx, l) in [(bi, below), (li, top)] {
                    let src = if idx == li {
                        crate::doc::clip::shown(p, li, f)
                    } else {
                        l.resolve(f).and_then(|id| p.cell(id)).map(std::borrow::Cow::Borrowed)
                    };
                    let Some(src) = src else { continue };
                    let xform = l.resolve_transform(f);
                    // Passing the *baked* size as the doc size keeps the
                    // transform maths centred on the same point, since the
                    // baked cell is itself centred on the doc.
                    crate::io::composite::composite_layer(&mut out, &src, &xform, l.opacity, bw, bh);
                }
                (f, out)
            })
            .collect();

        self.structural_edit(false, move |p| {
            let n = p.frame_count;
            let merged = &mut p.layers[bi];
            merged.exposures = vec![None; n];
            merged.transform = Transform::default();
            merged.transform_keys.clear();
            merged.opacity = 1.0;
            // Keep future keys on the merged layer the same size as the bake,
            // or the next blank key would shrink back to the doc rect.
            merged.cell_w = bw;
            merged.cell_h = bh;
            for (f, canvas) in baked {
                let id = p.cells.len();
                p.cells.push(canvas.into());
                p.layers[bi].set_key(f, id);
            }
            p.layers.remove(li);
            p.relink_after_remove(li);
            p.current_layer = bi;
        });
    }

    /// Merge group `g` into one drawing layer that looks the same: its
    /// layers' composite on every frame where any of them changes, as the
    /// group draws it, with the group's opacity carried over as the layer's.
    /// Hidden and reference layers aren't in the picture, so they aren't
    /// merged away either: they stay, as plain layers just below. One undo
    /// step.
    pub fn merge_group(&mut self, g: usize) {
        let p = &self.project;
        if !p.is_group(g) || p.locked_eff(g) {
            return;
        }
        self.playback.stop();
        let kids = p.children(g);
        let drawn: Vec<usize> = kids.clone().filter(|&c| p.layers[c].visible && !p.layers[c].reference).collect();
        let kept: Vec<usize> = kids.clone().filter(|c| !drawn.contains(c)).collect();
        let animated = drawn.iter().any(|&c| p.layers[c].transform_keys.len() >= 2);
        let mut frames: Vec<usize> = if animated {
            (0..p.frame_count).collect()
        } else {
            let mut fs: Vec<usize> = (0..p.frame_count)
                .filter(|&f| drawn.iter().any(|&c| p.layers[c].is_key(f)))
                .collect();
            for &c in &drawn {
                fs.extend(p.layers[c].transform_keys.iter().map(|k| k.frame.min(p.frame_count.saturating_sub(1))));
            }
            fs.push(0);
            fs.sort_unstable();
            fs.dedup();
            fs
        };
        frames.retain(|&f| f < p.frame_count);
        // One canvas size for every frame: big enough for wherever the layers
        // go, capped at the texture limit.
        let (mut bw, mut bh) = (p.width, p.height);
        for &f in &frames {
            let xf = |li: usize| p.layers[li].resolve_transform(f);
            let (w, h) = crate::doc::clip::group_space(p, g, f, &xf);
            (bw, bh) = (bw.max(w), bh.max(h));
        }
        let cap = self.max_tex;
        (bw, bh) = (bw.min(cap), bh.min(cap));
        // Baked as if the group were shown — a hidden group merges into a
        // hidden layer, not an empty one. Its hidden layers and references
        // stay out: render_group skips them.
        let mut shown = p.clone();
        shown.layers[g].visible = true;
        let q = &shown;
        let baked: Vec<(usize, Canvas)> = frames
            .into_iter()
            .map(|f| {
                let mut out = Canvas::new(bw, bh);
                let xf = |li: usize| q.layers[li].resolve_transform(f);
                let all = DirtyRect {
                    min_x: 0,
                    min_y: 0,
                    max_x: bw,
                    max_y: bh,
                };
                crate::doc::clip::render_group(q, g, f, true, &xf, &mut out, all);
                (f, out)
            })
            .collect();
        let header = p.layers[g].clone();
        let kept_layers: Vec<crate::doc::layer::Layer> = kept.iter().map(|&c| p.layers[c].clone()).collect();
        let block = kids.start..g + 1;
        self.structural_edit(false, move |p| {
            let n = p.frame_count;
            let mut merged = crate::doc::layer::Layer::new(header.name.clone(), n);
            merged.opacity = header.opacity;
            merged.visible = header.visible;
            merged.cell_w = bw;
            merged.cell_h = bh;
            for (f, canvas) in baked {
                let id = p.cells.len();
                p.cells.push(canvas.into());
                merged.set_key(f, id);
            }
            // The group's rows go; what was kept and the merged layer take
            // their place, bottom first.
            for i in block.clone().rev() {
                p.layers.remove(i);
                p.relink_after_remove(i);
            }
            let mut at = block.start;
            for mut l in kept_layers {
                l.in_group = false;
                p.layers.insert(at, l);
                p.relink_after_insert(at);
                at += 1;
            }
            p.layers.insert(at, merged);
            p.relink_after_insert(at);
            p.current_layer = at;
        });
        self.selected_layers.clear();
        self.layer_rename = None;
        self.expand_cfg = None;
    }

    /// Transform that fits a `cw`×`ch` source within the canvas without
    /// upscaling, centered. The user can scale up from here to crop.
    fn fit_transform(&self, cw: f32, ch: f32) -> Transform {
        let (pw, ph) = (self.project.width as f32, self.project.height as f32);
        let fit = (pw / cw).min(ph / ch).min(1.0);
        Transform {
            tx: 0.0,
            ty: 0.0,
            scale: fit,
            rot: 0.0,
        }
    }

    /// File → Import image…: pick a still image and drop it on a new layer
    /// below the active layer, shown on every frame.
    pub fn import_image(&mut self) {
        match crate::io::image_import::pick() {
            Ok(Some(canvas)) => {
                self.import_cells_as_layer("Imported image", vec![canvas], true)
            }
            Ok(None) => {}
            Err(e) => log::error!("Image import failed: {e:#}"),
        }
    }

    /// Edit → Paste image (Ctrl+V): read an image from the clipboard and drop
    /// it on a new layer at the very bottom of the stack (a background), shown
    /// on every frame. No-op with a log line when the clipboard holds no image.
    pub fn paste_image_as_background(&mut self) {
        let canvas = match crate::io::image_import::from_clipboard() {
            Ok(Some(c)) => c,
            Ok(None) => {
                log::info!("Clipboard has no image to paste");
                return;
            }
            Err(e) => {
                log::error!("Paste image failed: {e:#}");
                return;
            }
        };
        // Cap to the GPU texture limit so a huge clipboard image can't crash
        // the upload (matches import_cells_as_layer).
        let canvas = cap_canvas(canvas, self.max_tex);
        let init_xform = self.fit_transform(canvas.width as f32, canvas.height as f32);
        self.structural_edit(false, move |p| {
            let idx = p.add_background_layer("Pasted background");
            p.layers[idx].transform = init_xform;
            let id = p.cells.len();
            p.cells.push(canvas.into());
            p.layers[idx].set_key(0, id);
        });
    }

    /// File → Copy frame to clipboard: the current frame as Save PNG writes it,
    /// on the system clipboard for another app to paste.
    pub fn copy_frame(&mut self) {
        let frame = self.project.current_frame;
        let flat = crate::io::composite::flatten_frame(&self.project, frame);
        match crate::io::png_save::copy_to_clipboard(&flat) {
            Ok(()) => {
                self.save_toast = Some((format!("Copied frame {}", frame + 1), Instant::now() + Self::TOAST_TTL));
            }
            Err(e) => {
                log::error!("Copy frame failed: {e:#}");
                self.krita_notice(format!("Couldn't copy the frame: {e:#}"), true);
            }
        }
    }

    /// File → Import video…: pick a file, then probe its frame count / fps on a
    /// worker thread. The range dialog opens once the probe returns.
    pub fn open_video_import(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Video", &["mp4", "mov", "mkv", "avi", "webm", "m4v"])
            .set_title("Choose a video to import")
            .pick_file()
        else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let probe_path = path.clone();
        thread::spawn(move || {
            let _ = tx.send(crate::io::video_import::probe(&probe_path));
        });
        self.bg_job = Some(BgJob::VideoProbe { rx, path });
        self.bg_label = Some("Reading video…");
    }

    /// File → Import GIF…: decode all frames on a worker thread, then open the
    /// range dialog.
    pub fn open_gif_import(&mut self) {
        self.open_frames_import("GIF", "gif", "Decoding GIF…", crate::io::gif_import::decode_all);
    }

    /// File → Import WebP…: as [`Self::open_gif_import`]. A still WebP comes
    /// in as one frame.
    pub fn open_webp_import(&mut self) {
        self.open_frames_import("WebP", "webp", "Decoding WebP…", crate::io::webp_import::decode_all);
    }

    fn open_frames_import(
        &mut self,
        format: &'static str,
        ext: &'static str,
        busy: &'static str,
        decode: fn(&Path) -> Result<Vec<Canvas>>,
    ) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter(format, &[ext])
            .set_title(format!("Choose a {format} to import"))
            .pick_file()
        else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(decode(&path));
        });
        self.bg_job = Some(BgJob::FramesDecode { rx, format });
        self.bg_label = Some(busy);
    }

    /// File → Export MP4…: pick an output path, then encode the whole project on
    /// a worker thread via ffmpeg. Called when the settings dialog is confirmed.
    /// Ask for a destination and run the configured export on a worker thread.
    ///
    /// Every format goes through here, not just MP4: flattening a long range is
    /// far too slow to hold the UI thread, and one path means one place where
    /// success and failure reach the user.
    pub fn start_export(&mut self) {
        let kind = self.export_cfg.kind;
        let dialog = rfd::FileDialog::new().set_title(kind.title());
        let target = match kind {
            ExportKind::PngSequence => dialog.pick_folder(),
            ExportKind::Gif => dialog
                .add_filter("GIF", &["gif"])
                .set_file_name("animation.gif")
                .save_file(),
            ExportKind::Webp => dialog
                .add_filter("WebP", &["webp"])
                .set_file_name("animation.webp")
                .save_file(),
            ExportKind::Mp4 => dialog
                .add_filter("MP4 video", &["mp4"])
                .set_file_name("animation.mp4")
                .save_file(),
            ExportKind::SpriteSheet => dialog
                .add_filter("PNG image", &["png"])
                .set_file_name("sheet.png")
                .save_file(),
        };
        let Some(path) = target else {
            return;
        };

        let last = self.project.frame_count.saturating_sub(1);
        let range = (
            self.export_cfg.start.min(last),
            self.export_cfg.end.min(last),
        );
        let sheet = crate::io::sprite_sheet::SheetOptions {
            columns: self.export_cfg.sheet_columns,
            padding: self.export_cfg.sheet_padding,
        };
        let settings = crate::io::mp4_export::Mp4Settings {
            crf: self.export_cfg.mp4.crf,
            preset: MP4_PRESETS[self
                .export_cfg
                .mp4
                .preset_idx
                .min(MP4_PRESETS.len() - 1)],
        };
        // 0 = forever, as the file has it.
        let plays = if self.export_cfg.webp_stop { self.export_cfg.webp_plays.max(1) } else { 0 };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let (order, cams) = self.export_order(kind, range);
        let project = self.project.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let res = match kind {
                ExportKind::PngSequence => {
                    crate::io::png_seq::export_to(&project, &path, &order, cams.as_deref())
                }
                ExportKind::Gif => {
                    crate::io::gif_export::export_to(&project, &path, &order, cams.as_deref())
                }
                ExportKind::Webp => {
                    crate::io::webp_export::export_to(&project, &path, &order, cams.as_deref(), plays)
                }
                ExportKind::Mp4 => crate::io::mp4_export::export_to(
                    &project,
                    &path,
                    &settings,
                    &order,
                    cams.as_deref(),
                ),
                ExportKind::SpriteSheet => {
                    crate::io::sprite_sheet::export_to(&project, &path, range, &sheet)
                }
            };
            let _ = tx.send(res.map(|()| name));
        });
        self.bg_job = Some(BgJob::Export(rx));
        self.bg_label = Some(kind.busy());
    }

    /// Confirm the frame-range dialog: slice (GIF / WebP, instant) or extract (video,
    /// on a worker thread) the chosen `[start, end]` range onto a new layer.
    pub fn confirm_import_range(&mut self) {
        let Some(st) = self.import_range.take() else {
            return;
        };
        self.show_import_range = false;
        // Defer freeing preview textures: this runs inside the egui pass and the
        // dialog already drew those thumbnails this frame (see field doc).
        self.preview_clear_pending = true;
        self.preview_rx = None;
        let start = st.start.min(st.end);
        let end = st.end.max(st.start);
        match st.source {
            ImportSource::Video { path, .. } => {
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || {
                    let _ = tx.send(crate::io::video_import::extract_frames(&path, start, end));
                });
                self.bg_job = Some(BgJob::VideoExtract {
                    rx,
                    name: "Imported video".to_string(),
                });
                self.bg_label = Some("Importing video…");
            }
            ImportSource::Frames { frames, format } => {
                let slice: Vec<Canvas> = frames
                    .into_iter()
                    .skip(start)
                    .take(end - start + 1)
                    .collect();
                self.import_cells_as_layer(format!("Imported {format}"), slice, false);
            }
        }
    }

    /// Cancel the frame-range dialog without importing.
    pub fn cancel_import_range(&mut self) {
        self.import_range = None;
        self.show_import_range = false;
        // Defer freeing preview textures until next frame (see field doc); the
        // dialog already referenced them in this frame's paint list.
        self.preview_clear_pending = true;
        self.preview_rx = None;
    }

    // --- Layer transform editing ---

    /// The transform currently shown for `layer_idx` at `frame`. The active
    /// layer at the current frame uses the live edit buffer so drags give
    /// immediate feedback; everything else resolves from the keyframes.
    pub fn display_transform(&self, layer_idx: usize, frame: usize) -> Transform {
        let Some(layer) = self.project.layers.get(layer_idx) else {
            return Transform::default();
        };
        if layer_idx == self.project.current_layer && frame == self.project.current_frame {
            layer.transform
        } else {
            layer.resolve_transform(frame)
        }
    }

    /// Keep the active layer's edit buffer in sync with its keyed value when the
    /// cursor (layer or frame) moves, so the Transform panel and "add key"
    /// reflect the current pose. Only reloads on an actual cursor change so live
    /// panel/drag edits between frames are not clobbered. Skipped during a
    /// layer-transform drag and for layers with no keys (the buffer *is* the
    /// value).
    fn sync_active_transform_buffer(&mut self) {
        if self.nav_to_layer {
            return;
        }
        let li = self.project.current_layer;
        let f = self.project.current_frame;
        if self.xform_sync_last == Some((li, f)) {
            return;
        }
        self.xform_sync_last = Some((li, f));
        if let Some(layer) = self.project.layers.get_mut(li) {
            if !layer.transform_keys.is_empty() {
                layer.transform = layer.resolve_transform(f);
            }
        }
    }

    /// Reset the active layer's transform to identity (undoable).
    pub fn reset_active_layer_transform(&mut self) {
        if self.active_layer_locked() {
            return;
        }
        self.structural_edit(false, |p| {
            let li = p.current_layer;
            if let Some(l) = p.layers.get_mut(li) {
                l.transform = Transform::default();
            }
        });
    }

    /// Snapshot timeline state at the start of a layer-transform drag.
    pub fn begin_layer_xform(&mut self) {
        self.layer_xform_before = Some(undo::TimelineState::capture(&self.project));
        // A group turns and scales about the middle of its layers, as they
        // stand when the drag starts.
        let p = &self.project;
        let g = p.current_layer;
        self.group_pivot = p.is_group(g).then(|| {
            let f = p.current_frame;
            let kids: Vec<Transform> = p.children(g).map(|c| p.layers[c].resolve_transform(f)).collect();
            let n = kids.len().max(1) as f32;
            let (sx, sy) = kids.iter().fold((0.0, 0.0), |(x, y), t| (x + t.tx, y + t.ty));
            (p.width as f32 / 2.0 + sx / n, p.height as f32 / 2.0 + sy / n)
        });
    }

    /// Move, turn or scale every layer of the active group by `f`, a change
    /// to a layer's pose. A layer whose transform is keyed has every key
    /// changed the same way, so it moves along on every frame — or, with
    /// auto-key on, gets a key here instead, as a single layer would. Layers
    /// locked on their own stay put.
    fn transform_group(&mut self, f: impl Fn(Transform) -> Transform) {
        let p = &mut self.project;
        let g = p.current_layer;
        if !p.is_group(g) || p.locked_eff(g) {
            return;
        }
        let frame = p.current_frame;
        let auto = self.auto_key_transform;
        for c in p.children(g) {
            if p.layers[c].locked {
                continue;
            }
            let l = &mut p.layers[c];
            if auto {
                let pose = f(l.resolve_transform(frame));
                l.set_transform_key(frame, pose);
                l.transform = pose;
            } else {
                for k in &mut l.transform_keys {
                    k.transform = f(k.transform);
                }
                l.transform = f(l.transform);
            }
        }
    }

    /// What scaling the whole group by `k` and turning it by `rot` about its
    /// pivot does to one layer's pose. Owns its inputs, so it can be handed
    /// to [`AppState::transform_group`].
    fn about_group_pivot(&self, k: f32, rot: f32) -> impl Fn(Transform) -> Transform {
        let (hw, hh) = (self.project.width as f32 / 2.0, self.project.height as f32 / 2.0);
        let (px, py) = self.group_pivot.unwrap_or((hw, hh));
        let (s, c) = rot.sin_cos();
        move |t: Transform| {
            let (cx, cy) = (hw + t.tx - px, hh + t.ty - py);
            let (x, y) = (k * (cx * c - cy * s), k * (cx * s + cy * c));
            Transform {
                tx: px + x - hw,
                ty: py + y - hh,
                scale: (t.scale * k).clamp(0.01, 100.0),
                rot: t.rot + rot,
            }
        }
    }

    /// Push a single undo entry for a completed layer-transform drag.
    pub fn commit_layer_xform(&mut self) {
        if let Some(before) = self.layer_xform_before.take() {
            // Auto-key writes the pose before the `after` snapshot, so the drag
            // and its key land in one undo entry rather than two.
            if self.auto_key_transform && !self.active_layer_locked() {
                let f = self.project.current_frame;
                let li = self.project.current_layer;
                if let Some(l) = self.project.layers.get_mut(li) {
                    let t = l.transform;
                    l.set_transform_key(f, t);
                }
            }
            let after = undo::TimelineState::capture(&self.project);
            self.history.push(undo::Command::Structural {
                before,
                after,
                cell_pixels: Vec::new(),
            });
        }
    }

    /// Locked itself or by its group — or a group header, which has nothing
    /// to edit.
    fn active_layer_locked(&self) -> bool {
        let li = self.project.current_layer;
        self.project.locked_eff(li) || self.project.is_group(li)
    }

    /// Translate the active layer by a document-space delta.
    pub fn apply_layer_pan(&mut self, dx: f32, dy: f32) {
        if self.project.is_group(self.project.current_layer) {
            self.transform_group(|t| Transform {
                tx: t.tx + dx,
                ty: t.ty + dy,
                ..t
            });
            return;
        }
        if self.active_layer_locked() {
            return;
        }
        if let Some(l) = self.project.layers.get_mut(self.project.current_layer) {
            l.transform.tx += dx;
            l.transform.ty += dy;
        }
    }

    /// Scale the active layer by `factor` (multiplicative).
    pub fn apply_layer_scale(&mut self, factor: f32) {
        if self.project.is_group(self.project.current_layer) {
            let f = self.about_group_pivot(factor, 0.0);
            self.transform_group(f);
            return;
        }
        if self.active_layer_locked() {
            return;
        }
        if let Some(l) = self.project.layers.get_mut(self.project.current_layer) {
            l.transform.scale = (l.transform.scale * factor).clamp(0.01, 100.0);
        }
    }

    /// Rotate the active layer by `d` radians.
    pub fn apply_layer_rotate(&mut self, d: f32) {
        if self.project.is_group(self.project.current_layer) {
            let f = self.about_group_pivot(1.0, d);
            self.transform_group(f);
            return;
        }
        if self.active_layer_locked() {
            return;
        }
        if let Some(l) = self.project.layers.get_mut(self.project.current_layer) {
            l.transform.rot += d;
        }
    }

    /// Add (or replace) a transform key at the current frame, using the active
    /// layer's current pose. Undoable.
    pub fn add_transform_key(&mut self) {
        if self.active_layer_locked() {
            return;
        }
        self.structural_edit(false, |p| {
            let f = p.current_frame;
            let li = p.current_layer;
            if let Some(l) = p.layers.get_mut(li) {
                let t = l.transform;
                l.set_transform_key(f, t);
            }
        });
    }

    /// Set the easing of the active layer's transform key on the current frame.
    /// Undoable. Mirrors [`AppState::set_camera_key_ease`].
    pub fn set_transform_key_ease(&mut self, ease: Ease) {
        if self.active_layer_locked() {
            return;
        }
        self.structural_edit(false, |p| {
            let (li, f) = (p.current_layer, p.current_frame);
            if let Some(l) = p.layers.get_mut(li) {
                l.set_transform_key_ease(f, ease);
            }
        });
    }

    /// Delete the transform key at the current frame (if any). Undoable.
    pub fn delete_transform_key(&mut self) {
        let (li, f) = (self.project.current_layer, self.project.current_frame);
        let has = self
            .project
            .layers
            .get(li)
            .map(|l| l.has_transform_key(f))
            .unwrap_or(false);
        if !has {
            return;
        }
        self.structural_edit(false, |p| {
            let f = p.current_frame;
            let li = p.current_layer;
            if let Some(l) = p.layers.get_mut(li) {
                l.delete_transform_key(f);
                // Keep the live buffer consistent with the new resolved value.
                l.transform = l.resolve_transform(f);
            }
        });
    }

    // --- Camera ---

    /// The camera to draw the guide with: the live edit buffer on the current
    /// frame (so panel edits and drags show immediately), resolved from the
    /// keys everywhere else. Mirrors `display_transform`.
    pub fn display_camera(&self, frame: usize) -> Camera {
        if frame == self.project.current_frame {
            self.project.camera
        } else {
            self.project.resolve_camera(frame)
        }
    }

    /// Step playback to the clock's `now`. Returns whether what is on screen
    /// moved on.
    ///
    /// A camera move stretched over loops keeps its own count of where it
    /// is; otherwise the frames and the camera run together, as ever.
    fn advance_playback(&mut self, now: f64) -> bool {
        let stretched = self.project.camera_loops() > 1;
        let was = self.playback.playing;
        let mut moved = if stretched {
            self.playback
                .tick_shot(&mut self.project, now, self.loop_timeline)
        } else {
            self.playback
                .tick(&mut self.project, now, self.loop_timeline)
        };
        // Run out at the end of the range with the loop off: back to the
        // start, as a stop would.
        if was && !self.playback.playing {
            moved |= self.return_to_play_start();
        }
        self.follow_stretched_camera(stretched);
        moved
    }

    /// Play, or stop and go back to the frame playback started on. `now` is
    /// the clock when there is one to hand (a button); a shortcut has none.
    pub fn play_pause(&mut self, now: Option<f64>) {
        if self.playback.playing {
            self.playback.stop();
            self.return_to_play_start();
            return;
        }
        self.play_from = Some(self.project.current_frame);
        match now {
            Some(t) => self.playback.toggle(t),
            None => self.playback.toggle_unclocked(),
        }
    }

    /// Play, or stop on the frame playback has got to: for catching a
    /// drawing to fix mid-playback.
    pub fn play_pause_here(&mut self, now: Option<f64>) {
        if self.playback.playing {
            self.play_from = None;
            self.playback.stop();
        } else {
            self.play_pause(now);
        }
    }

    /// Put the playhead back where playback started. Whether it moved.
    fn return_to_play_start(&mut self) -> bool {
        let Some(f) = self.play_from.take() else { return false };
        // A camera move stretched over loops would otherwise put the
        // playhead on the frame the camera reached.
        self.shot_cam = None;
        self.cam_sync_last = None;
        let was = self.project.current_frame;
        self.project.goto(f);
        self.project.current_frame != was
    }

    /// While a camera move stretched over loops plays, note where it has got
    /// to, for `sync_camera_buffer` to show. When it stops, put the playhead
    /// on the frame the camera had reached: the camera stays where it was,
    /// and the timeline is back to saying where in its move that is.
    fn follow_stretched_camera(&mut self, stretched: bool) {
        if stretched && self.playback.playing {
            self.shot_cam = Some(self.playback.shot_cam_time(&self.project));
        } else if let Some(t) = self.shot_cam.take() {
            self.project.goto(t.round().max(0.0) as usize);
            self.cam_sync_last = None;
        }
    }

    /// Whether an export of `kind` has the camera's move outlast the
    /// drawings. A sprite sheet is its frames side by side: never.
    fn camera_stretched(&self, kind: ExportKind) -> bool {
        kind != ExportKind::SpriteSheet && self.project.camera_loops() > 1
    }

    /// What the export dialog's settings add up to for `kind`.
    pub fn export_info(&self, kind: ExportKind) -> crate::io::frame_order::OrderInfo {
        use crate::io::frame_order;
        let (start, end) = (self.export_cfg.start, self.export_cfg.end);
        let pb = self.export_cfg.playback(kind);
        let p = &self.project;
        if self.camera_stretched(kind) {
            let fill = p.camera_opts.fill;
            frame_order::shot_info(start, end, p.camera_loops(), fill, pb.loop_secs, p.fps)
        } else {
            frame_order::info(start, end, pb, p.fps)
        }
    }

    /// The timeline frame each frame of an export of `kind` over `range`
    /// shows, and — when the camera's move is stretched over loops — the
    /// camera each is seen through.
    fn export_order(&self, kind: ExportKind, range: (usize, usize)) -> (Vec<usize>, Option<Vec<Camera>>) {
        use crate::io::frame_order;
        let pb = self.export_cfg.playback(kind);
        let p = &self.project;
        if !self.camera_stretched(kind) {
            return (frame_order::frame_order(range.0, range.1, pb, p.fps), None);
        }
        let fill = p.camera_opts.fill;
        let shot = frame_order::shot(range.0, range.1, p.camera_loops(), fill, pb.loop_secs, p.fps);
        let cams = shot.cam_times.iter().map(|&t| p.resolve_camera_at(t)).collect();
        (shot.frames, Some(cams))
    }

    /// Keep the camera edit buffer in sync with its keyed value as the frame
    /// cursor moves. Same contract as `sync_active_transform_buffer`: only on
    /// an actual frame change, never mid-drag, and never when there are no keys
    /// (then the buffer *is* the value).
    fn sync_camera_buffer(&mut self) {
        if self.nav_to_camera {
            return;
        }
        // A move stretched over loops is playing: the camera is where the
        // move has got to, between the keys of some other frame.
        if let Some(t) = self.shot_cam {
            self.project.camera = self.project.resolve_camera_at(t);
            self.cam_sync_last = None;
            return;
        }
        let f = self.project.current_frame;
        if self.cam_sync_last == Some(f) {
            return;
        }
        self.cam_sync_last = Some(f);
        if !self.project.camera_keys.is_empty() {
            self.project.camera = self.project.resolve_camera(f);
        }
    }

    /// Snapshot timeline state at the start of a camera drag.
    pub fn begin_camera_drag(&mut self) {
        self.camera_drag_before = Some(undo::TimelineState::capture(&self.project));
    }

    /// Push a single undo entry for a completed camera drag.
    pub fn commit_camera_drag(&mut self) {
        if let Some(before) = self.camera_drag_before.take() {
            let after = undo::TimelineState::capture(&self.project);
            self.history.push(undo::Command::Structural {
                before,
                after,
                cell_pixels: Vec::new(),
            });
        }
    }

    /// The camera the view is locked to while looking through it: the live
    /// one — or, while its frame is being dragged, the one the drag began
    /// from. A view that followed a frame that follows the cursor would run
    /// away from it; this way the view holds still and catches up on release.
    pub fn look_through_camera(&self) -> Camera {
        match self.cam_drag {
            Some(d) => d.start,
            None => self.display_camera(self.project.current_frame),
        }
    }

    /// A press at document point `p` while editing the camera: take hold of
    /// its frame — inside to move it, a corner to zoom, just outside a corner
    /// to roll. `false`, with nothing changed, when camera edit mode is off
    /// or the press missed the frame; the active tool gets it then.
    pub fn camera_frame_down(&mut self, p: (f32, f32)) -> bool {
        if !self.camera_edit || self.playback.playing {
            return false;
        }
        let cam = self.project.camera;
        let tol = crate::tools::selection::HANDLE_PX / self.view_scale.max(1e-6);
        let Some(grab) = cam.grab_at(p, self.canvas_size(), self.camera_frame_size(), tol) else {
            return false;
        };
        self.begin_camera_drag();
        self.cam_drag = Some(CamDrag {
            grab,
            from: p,
            start: cam,
        });
        true
    }

    /// The held frame's drag has reached document point `p`.
    pub fn camera_frame_move(&mut self, p: (f32, f32)) {
        let Some(d) = self.cam_drag else {
            return;
        };
        self.project.camera = d.start.dragged(d.grab, d.from, p, self.canvas_size());
        self.hold_camera_inside();
    }

    /// Let go of the camera's frame: one undo step for the whole drag, and
    /// none for a press that moved nothing.
    pub fn camera_frame_up(&mut self) {
        let Some(d) = self.cam_drag.take() else {
            return;
        };
        if self.project.camera == d.start {
            self.camera_drag_before = None;
        } else {
            self.commit_camera_drag();
        }
    }

    /// Turn Draw camera mode on or off. On, it puts camera edit mode out:
    /// both want the plain drag.
    pub fn set_camera_box(&mut self, on: bool) {
        self.camera_box = on;
        if on {
            self.camera_edit = false;
        }
    }

    /// A press at document point `p` in Draw camera mode: start a box there.
    /// `false`, with nothing started, when the mode is off — or while
    /// playing, when no frame holds still to key.
    pub fn camera_box_down(&mut self, p: (f32, f32)) -> bool {
        if !self.camera_box || self.playback.playing {
            return false;
        }
        self.cam_box = Some(CamBox {
            from: p,
            to: p,
            moved: false,
        });
        true
    }

    /// The box's drag has reached document point `p`.
    pub fn camera_box_move(&mut self, p: (f32, f32)) {
        let tap = CAMERA_TAP_PX / self.view_scale.max(1e-6);
        if let Some(b) = &mut self.cam_box {
            b.to = p;
            b.moved |= (p.0 - b.from.0).hypot(p.1 - b.from.1) > tap;
        }
    }

    /// The camera the box being dragged out gives: what its preview shows
    /// and what letting go keys. `None` while it is still a tap, or brought
    /// back to one.
    pub fn camera_box_camera(&self) -> Option<Camera> {
        let b = self.cam_box.filter(|b| b.moved)?;
        let tap = CAMERA_TAP_PX / self.view_scale.max(1e-6);
        if (b.to.0 - b.from.0).hypot(b.to.1 - b.from.1) <= tap {
            return None;
        }
        let cam = self
            .project
            .camera
            .boxed(b.from, b.to, self.canvas_size(), self.camera_frame_size());
        Some(self.project.held_inside(cam))
    }

    /// Let go of the box: key its camera on this frame. A tap keys the
    /// camera as it was, centred where it landed. A box dragged back to
    /// nothing is let go of, and keys nothing.
    pub fn camera_box_up(&mut self) {
        let boxed = self.camera_box_camera();
        let Some(b) = self.cam_box.take() else {
            return;
        };
        let cam = match boxed {
            Some(cam) => cam,
            None if !b.moved => {
                let cam = self.project.camera.centred_on(b.from, self.canvas_size());
                self.project.held_inside(cam)
            }
            None => return,
        };
        self.key_drawn_camera(cam);
    }

    /// Key `cam` on this frame, as Draw camera mode does: over the key
    /// here, or as a new one that eases in and out. A camera with no keys
    /// yet is first keyed as it was on frame 0, so there is a move from it
    /// into the new key. One undo step.
    fn key_drawn_camera(&mut self, cam: Camera) {
        self.structural_edit(false, |p| {
            let f = p.current_frame;
            let mut fresh = Vec::new();
            if p.camera_keys.is_empty() && f > 0 {
                let was = p.camera;
                p.set_camera_key(0, was);
                fresh.push(0);
            }
            if !p.has_camera_key(f) {
                fresh.push(f);
            }
            p.set_camera_key(f, cam);
            for k in p.camera_keys.iter_mut().filter(|k| fresh.contains(&k.frame)) {
                k.ease = Ease::Both;
            }
            p.camera = cam;
        });
    }

    /// Where the camera comes from, for Draw camera mode's ghost: the
    /// camera on the last key before this frame, if there is one.
    pub fn camera_ghost(&self) -> Option<Camera> {
        let f = self.project.prev_camera_key()?;
        Some(self.project.resolve_camera(f))
    }

    fn canvas_size(&self) -> (f32, f32) {
        (self.project.width as f32, self.project.height as f32)
    }

    fn camera_frame_size(&self) -> (f32, f32) {
        let (w, h) = self.project.output_size();
        (w as f32, h as f32)
    }

    /// Move the camera by a document-space delta. The guide rect follows the
    /// cursor, so dragging right shows what is to the right.
    pub fn apply_camera_pan(&mut self, dx: f32, dy: f32) {
        self.project.camera.tx += dx;
        self.project.camera.ty += dy;
        self.hold_camera_inside();
    }

    /// Push the camera in/out by `factor` (multiplicative).
    pub fn apply_camera_zoom(&mut self, factor: f32) {
        self.project.camera.zoom =
            (self.project.camera.zoom * factor).clamp(Camera::ZOOM_MIN, Camera::ZOOM_MAX);
        self.hold_camera_inside();
    }

    /// Roll the camera by `d` radians.
    pub fn apply_camera_rotate(&mut self, d: f32) {
        self.project.camera.rot += d;
        self.hold_camera_inside();
    }

    /// Draw the UI in the picked colour scheme, from the next frame on.
    pub fn apply_scheme(&self, ctx: &egui::Context) {
        crate::ui::theme::apply(ctx, self.color_scheme.palette(self.custom_colors));
    }

    pub fn panel_shown(&self, id: PanelId) -> bool {
        !self.hidden_panels.contains(&id)
    }

    pub fn set_panel_shown(&mut self, id: PanelId, shown: bool) {
        self.hidden_panels.retain(|&p| p != id);
        if !shown {
            self.hidden_panels.push(id);
        }
    }

    /// Every panel shown again, back where it first opened and at its first
    /// size. egui keeps those per window id, so the panels move to new ids
    /// rather than having each piece of egui state cleared.
    pub fn reset_panel_layout(&mut self) {
        self.hidden_panels.clear();
        self.panel_layout_gen = self.panel_layout_gen.wrapping_add(1).max(1);
    }

    /// Reset the camera to identity — frame rect back on the document rect.
    /// Undoable.
    pub fn reset_camera(&mut self) {
        self.structural_edit(false, |p| p.camera = Camera::default());
    }

    /// Give the camera a frame of its own size — or, with `None` or the
    /// canvas's size, hand it back the canvas's. Zoom and keys are left
    /// alone, so what it frames keeps its scale and the frame takes in more
    /// or less of the canvas.
    ///
    /// A setting for the session rather than an edit: not on the undo stack,
    /// so undoing a stroke never changes what the export comes out at.
    pub fn set_camera_size(&mut self, size: Option<(u32, u32)>) {
        let canvas = (self.project.width, self.project.height);
        self.project.camera_opts.size = size
            .map(|(w, h)| (w.clamp(1, MAX_CAMERA_SIDE), h.clamp(1, MAX_CAMERA_SIDE)))
            .filter(|&s| s != canvas);
        if let Some(own) = self.project.camera_opts.size {
            self.last_camera_size = Some(own);
        }
        self.hold_camera_inside();
    }

    /// One of the camera's session options changed what the keys resolve to
    /// — the smooth path, the hold on the canvas: bring the live camera back
    /// in step with them, as a frame change would.
    pub fn camera_opts_changed(&mut self) {
        self.cam_sync_last = None;
    }

    /// Keep the live camera on the canvas while it is set to stay there. A
    /// no-op otherwise, and for a camera already inside.
    pub fn hold_camera_inside(&mut self) {
        self.project.camera = self.project.held_inside(self.project.camera);
    }

    /// Frame the canvas with the live camera. Undoable, like the reset.
    pub fn frame_camera(&mut self, how: CameraFraming) {
        self.structural_edit(false, |p| {
            let canvas = (p.width as f32, p.height as f32);
            let (ow, oh) = p.output_size();
            let out = (ow as f32, oh as f32);
            let mut cam = p.camera;
            match how {
                CameraFraming::Fit => cam.zoom = cam.fit_zoom(canvas, out),
                CameraFraming::Fill => cam.zoom = cam.fill_zoom(canvas, out),
                CameraFraming::OneToOne => cam.zoom = 1.0,
                CameraFraming::Centre => {}
            }
            if how != CameraFraming::OneToOne {
                (cam.tx, cam.ty) = (0.0, 0.0);
            }
            cam.zoom = cam.zoom.clamp(Camera::ZOOM_MIN, Camera::ZOOM_MAX);
            p.camera = p.held_inside(cam);
        });
    }

    /// Add (or replace) a camera key at the current frame from the live camera.
    /// Undoable.
    pub fn add_camera_key(&mut self) {
        self.structural_edit(false, |p| {
            let f = p.current_frame;
            let cam = p.camera;
            p.set_camera_key(f, cam);
        });
    }

    /// Delete the camera key at the current frame (if any). Undoable.
    pub fn delete_camera_key(&mut self) {
        if !self.project.has_camera_key(self.project.current_frame) {
            return;
        }
        self.structural_edit(false, |p| {
            let f = p.current_frame;
            p.delete_camera_key(f);
            // Keep the live buffer consistent with the new resolved value.
            p.camera = p.resolve_camera(f);
        });
    }

    /// Move the camera key on `from` to `to`, or copy it there. Undoable.
    /// See [`Project::move_camera_key`].
    pub fn move_camera_key(&mut self, from: usize, to: usize, copy: bool) {
        if from == to || !self.project.has_camera_key(from) {
            return;
        }
        self.structural_edit(false, |p| {
            p.move_camera_key(from, to, copy);
            // Keep the live buffer on what the keys now say for this frame.
            p.camera = p.resolve_camera(p.current_frame);
        });
    }

    /// Set the easing of the camera key on the current frame. Undoable.
    pub fn set_camera_key_ease(&mut self, ease: Ease) {
        let f = self.project.current_frame;
        self.structural_edit(false, |p| {
            if let Some(k) = p.camera_keys.iter_mut().find(|k| k.frame == f) {
                k.ease = ease;
            }
        });
    }

    /// Cell size the active layer draws into, and how many cells a resize would
    /// touch — the Layers panel shows both so the memory cost is visible.
    pub fn active_layer_cell_size(&self) -> (u32, u32) {
        self.project
            .layers
            .get(self.project.current_layer)
            .map(|l| l.cell_size(self.project.width, self.project.height))
            .unwrap_or((self.project.width, self.project.height))
    }

    pub fn active_layer_cell_count(&self) -> usize {
        self.project.layer_cell_ids(self.project.current_layer).len()
    }

    /// Grow the active layer's drawable buffer, so its artwork can extend past
    /// what the camera sees. Undoable.
    pub fn expand_active_layer_canvas(&mut self, w: u32, h: u32) {
        if self.active_layer_locked() {
            return;
        }
        // The float's cell is about to be re-padded under it.
        self.land_float();
        // Each cell is one GPU texture; a side past the device limit can't be
        // uploaded at all.
        let (w, h) = (w.clamp(1, self.max_tex), h.clamp(1, self.max_tex));
        let layer = self.project.current_layer;
        let before_size = self.active_layer_cell_size();
        if (w, h) == before_size {
            return;
        }
        // The buffers themselves: the re-pad swaps in new ones, so holding
        // these costs nothing beyond what the layer already had.
        let before: Vec<(CellId, Arc<Canvas>)> = self
            .project
            .layer_cell_ids(layer)
            .into_iter()
            .filter_map(|id| self.project.cells.get(id).map(|c| (id, Arc::clone(c))))
            .collect();
        self.project.expand_layer_canvas(layer, w, h);
        self.history.push(undo::Command::LayerCanvasResize {
            layer,
            before_size,
            after_size: (w, h),
            before,
        });
        // Every cell on this layer was re-padded; no other layer's was.
        for id in self.project.layer_cell_ids(layer) {
            self.mark_dirty(id);
        }
    }

    // --- Tracker / stabilization ---

    /// Record a stabilization tracking point (doc space) on the active layer at
    /// the current frame.
    ///
    /// Single-point mode: sets point A and advances one frame (no wrap) so the
    /// user can click straight through the clip. Two-point mode: the first
    /// click on a frame sets A, the second sets B, then the frame advances.
    /// Clicks are not pushed to undo history — re-click a frame to fix a miss.
    pub fn tracker_click(&mut self, doc: (f32, f32)) {
        self.playback.stop();
        let li = self.project.current_layer;
        let f = self.project.current_frame;
        let n = self.project.frame_count;
        if self.project.locked_eff(li) || self.project.is_group(li) {
            return;
        }
        let Some(l) = self.project.layers.get_mut(li) else {
            return;
        };
        if l.track_points.len() < n {
            l.track_points.resize(n, TrackSample::default());
        }
        let pt = Some([doc.0, doc.1]);
        let mut advance = true;
        if self.tracker_two_points {
            if self.tracker_pending_b == Some(f) {
                l.track_points[f].b = pt;
                self.tracker_pending_b = None;
            } else {
                l.track_points[f] = TrackSample { a: pt, b: None };
                self.tracker_pending_b = Some(f);
                advance = false;
            }
        } else {
            l.track_points[f].a = pt;
        }
        if advance && f + 1 < n {
            self.project.goto(f + 1);
        }
    }

    /// Number of frames with a tracking point on the active layer.
    pub fn tracked_point_count(&self) -> usize {
        self.project
            .layers
            .get(self.project.current_layer)
            .map(|l| l.track_points.iter().filter(|s| s.a.is_some()).count())
            .unwrap_or(0)
    }

    /// Remove every tracking point on the active layer. Undoable.
    pub fn clear_track_points(&mut self) {
        self.tracker_pending_b = None;
        if self.tracked_point_count() == 0 {
            return;
        }
        self.structural_edit(false, |p| {
            let li = p.current_layer;
            if let Some(l) = p.layers.get_mut(li) {
                l.track_points.clear();
            }
        });
    }

    /// Write transform keys on the active layer so the tracked feature stays
    /// where it is on the first tracked frame (the reference). Frames whose
    /// sample has both points get rotation/zoom correction as well; A-only
    /// frames get translation only. One undo entry.
    pub fn stabilize_active_layer(&mut self) {
        if self.active_layer_locked() {
            return;
        }
        self.playback.stop();
        self.tracker_pending_b = None;
        let li = self.project.current_layer;
        let Some(layer) = self.project.layers.get(li) else {
            return;
        };
        let tracked: Vec<(usize, TrackSample)> = layer
            .track_points
            .iter()
            .enumerate()
            .filter(|(_, s)| s.a.is_some())
            .map(|(f, s)| (f, *s))
            .collect();
        if tracked.len() < 2 {
            return;
        }
        let (_, r) = tracked[0];
        let ra = r.a.unwrap();
        // Resolve every base transform against the pre-stabilize key set —
        // inserting keys as we go would skew later resolves.
        let bases: Vec<Transform> = tracked
            .iter()
            .map(|&(f, _)| layer.resolve_transform(f))
            .collect();
        let (pw, ph) = (self.project.width as f32, self.project.height as f32);
        let (ccx, ccy) = (pw * 0.5, ph * 0.5);

        let keys: Vec<(usize, Transform)> = tracked
            .iter()
            .zip(&bases)
            .map(|(&(f, s), &base)| {
                let pa = s.a.unwrap();
                // Two-point similarity needs B on both this frame and the
                // reference, and a non-degenerate A→B span.
                let two = match (s.b, r.b) {
                    (Some(pb), Some(rb)) => {
                        let v = [pb[0] - pa[0], pb[1] - pa[1]];
                        let w = [rb[0] - ra[0], rb[1] - ra[1]];
                        let vlen = (v[0] * v[0] + v[1] * v[1]).sqrt();
                        let wlen = (w[0] * w[0] + w[1] * w[1]).sqrt();
                        (vlen > 1e-3).then(|| {
                            let sd = wlen / vlen;
                            let theta = w[1].atan2(w[0]) - v[1].atan2(v[0]);
                            (sd, theta)
                        })
                    }
                    _ => None,
                };
                let t = match two {
                    Some((sd, theta)) => {
                        // Doc-space similarity about the canvas center that maps
                        // this frame's points onto the reference points, composed
                        // with the frame's base transform.
                        let (sin, cos) = theta.sin_cos();
                        let rot = |x: f32, y: f32| (x * cos - y * sin, x * sin + y * cos);
                        let (px, py) = (pa[0] - ccx, pa[1] - ccy);
                        let (rpx, rpy) = rot(px, py);
                        let dx = (ra[0] - ccx) - sd * rpx;
                        let dy = (ra[1] - ccy) - sd * rpy;
                        let (rtx, rty) = rot(base.tx, base.ty);
                        Transform {
                            tx: sd * rtx + dx,
                            ty: sd * rty + dy,
                            scale: base.scale * sd,
                            rot: base.rot + theta,
                        }
                    }
                    None => Transform {
                        tx: base.tx + (ra[0] - pa[0]),
                        ty: base.ty + (ra[1] - pa[1]),
                        ..base
                    },
                };
                (f, t)
            })
            .collect();

        self.structural_edit(false, move |p| {
            let li = p.current_layer;
            let f = p.current_frame;
            if let Some(l) = p.layers.get_mut(li) {
                for (frame, t) in keys {
                    l.set_transform_key(frame, t);
                }
                // Keep the live edit buffer showing the current frame's pose.
                l.transform = l.resolve_transform(f);
            }
        });
        // The buffer changed under the cursor-sync cache — force a re-sync.
        self.xform_sync_last = None;
    }

    /// Poll the active background import job; advance state when it completes.
    fn poll_bg_jobs(&mut self) {
        let Some(job) = self.bg_job.take() else {
            return;
        };
        self.bg_job = match job {
            BgJob::FramesDecode { rx, format } => match rx.try_recv() {
                Ok(res) => {
                    self.bg_label = None;
                    self.on_frames_decoded(res, format);
                    None
                }
                Err(TryRecvError::Empty) => Some(BgJob::FramesDecode { rx, format }),
                Err(TryRecvError::Disconnected) => {
                    self.bg_label = None;
                    log::error!("{format} decode worker died");
                    None
                }
            },
            BgJob::VideoProbe { rx, path } => match rx.try_recv() {
                Ok(res) => {
                    self.bg_label = None;
                    self.on_video_probed(res, path);
                    None
                }
                Err(TryRecvError::Empty) => Some(BgJob::VideoProbe { rx, path }),
                Err(TryRecvError::Disconnected) => {
                    self.bg_label = None;
                    log::error!("Video probe worker died");
                    None
                }
            },
            BgJob::VideoExtract { rx, name } => match rx.try_recv() {
                Ok(res) => {
                    self.bg_label = None;
                    self.on_video_extracted(res, name);
                    None
                }
                Err(TryRecvError::Empty) => Some(BgJob::VideoExtract { rx, name }),
                Err(TryRecvError::Disconnected) => {
                    self.bg_label = None;
                    log::error!("Video extract worker died");
                    None
                }
            },
            BgJob::KraImport(rx) => match rx.try_recv() {
                Ok(res) => {
                    self.bg_label = None;
                    self.on_kra_imported(res);
                    None
                }
                Err(TryRecvError::Empty) => Some(BgJob::KraImport(rx)),
                Err(TryRecvError::Disconnected) => {
                    self.bg_label = None;
                    log::error!("Krita import worker died");
                    None
                }
            },
            BgJob::Export(rx) => match rx.try_recv() {
                Ok(res) => {
                    self.bg_label = None;
                    // Exports used to fail into the log only, where nobody saw
                    // them. Route both outcomes to the same toast / modal the
                    // project save uses.
                    match res {
                        Ok(name) => {
                            self.save_toast = Some((format!("Exported {name}"), Instant::now() + Self::TOAST_TTL));
                            self.save_error = None;
                        }
                        Err(e) => {
                            log::error!("Export failed: {e:#}");
                            self.save_error = Some(format!("{e:#}"));
                        }
                    }
                    None
                }
                Err(TryRecvError::Empty) => Some(BgJob::Export(rx)),
                Err(TryRecvError::Disconnected) => {
                    self.bg_label = None;
                    let msg = "Export worker died".to_string();
                    log::error!("{msg}");
                    self.save_error = Some(msg);
                    None
                }
            },
        };
    }

    fn on_frames_decoded(&mut self, res: Result<Vec<Canvas>>, format: &'static str) {
        match res {
            Ok(frames) if !frames.is_empty() => {
                let total = frames.len();
                self.preview_tex.clear();
                self.preview_rx = None;
                self.import_range = Some(ImportRangeState {
                    source: ImportSource::Frames { frames, format },
                    total,
                    start: 0,
                    end: total - 1,
                });
                self.show_import_range = true;
            }
            Ok(_) => log::error!("{format} has no frames"),
            Err(e) => log::error!("{format} decode failed: {e:#}"),
        }
    }

    fn on_video_probed(&mut self, res: Result<(usize, f64)>, path: PathBuf) {
        match res {
            Ok((total, fps)) if total > 0 => {
                self.preview_tex.clear();
                self.preview_rx = None;
                self.import_range = Some(ImportRangeState {
                    source: ImportSource::Video { path, fps },
                    total,
                    start: 0,
                    end: total - 1,
                });
                self.show_import_range = true;
            }
            Ok(_) => log::error!("Video has no frames"),
            Err(e) => log::error!("Video probe failed (is ffmpeg installed?): {e:#}"),
        }
    }

    fn on_video_extracted(&mut self, res: Result<Vec<Canvas>>, name: String) {
        match res {
            Ok(cells) => self.import_cells_as_layer(name, cells, false),
            Err(e) => log::error!("Video import failed: {e:#}"),
        }
    }

    /// Ensure a range-dialog preview texture exists (or is being fetched) for
    /// source frame `idx`. GIF / WebP previews are built instantly from decoded frames;
    /// video previews are fetched on a worker thread (one at a time).
    pub fn request_preview(&mut self, ctx: &egui::Context, idx: usize) {
        // Decide what to do without holding an import_range borrow across the
        // texture/channel mutations.
        enum Act {
            Have(ColorImage),
            SpawnVideo(PathBuf, f64),
            Nothing,
        }
        let act = if self.preview_tex.contains_key(&idx) || self.preview_rx.is_some() {
            Act::Nothing
        } else {
            match &self.import_range {
                Some(st) => match &st.source {
                    ImportSource::Frames { frames, .. } => match frames.get(idx) {
                        Some(c) => Act::Have(preview_color_image(c.width, c.height, &c.pixels())),
                        None => Act::Nothing,
                    },
                    ImportSource::Video { path, fps } => Act::SpawnVideo(path.clone(), *fps),
                },
                None => Act::Nothing,
            }
        };
        match act {
            Act::Have(img) => {
                let tex = ctx.load_texture(format!("preview_{idx}"), img, TextureOptions::LINEAR);
                self.preview_tex.insert(idx, tex);
            }
            Act::SpawnVideo(path, fps) => {
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || {
                    let _ = tx.send(crate::io::video_import::extract_preview(&path, idx, fps));
                });
                self.preview_rx = Some((idx, rx));
            }
            Act::Nothing => {}
        }
    }

    /// Poll an in-flight video preview extraction; upload the texture when ready.
    fn poll_preview(&mut self, ctx: &egui::Context) {
        let Some((idx, rx)) = self.preview_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok((w, h, buf))) => {
                let img = premultiplied_image([w as usize, h as usize], &buf);
                let tex = ctx.load_texture(format!("preview_{idx}"), img, TextureOptions::LINEAR);
                self.preview_tex.insert(idx, tex);
            }
            Ok(Err(e)) => log::warn!("preview frame failed: {e:#}"),
            Err(TryRecvError::Empty) => self.preview_rx = Some((idx, rx)),
            Err(TryRecvError::Disconnected) => {}
        }
    }

    /// Mirror new CellIds into the dirty map so freshly-allocated cells get
    /// uploaded on the next sync.
    fn ensure_cell_tracking(&mut self) {
        for id in 0..self.project.cells.len() {
            self.cell_dirty.entry(id).or_insert(Dirty::Full);
        }
    }

    /// Upload what the canvas is about to draw and doesn't have yet: each
    /// visible layer's cell on this frame, and the active layer's onion ghosts.
    ///
    /// Uploads are as small as the change. A cell that only had a stroke added
    /// sends that stroke's rect; a new texture holds just the cell's drawing
    /// (see `CellTex`). During a stroke, only the region flushed since the last
    /// frame goes up — the full refresh the pen-up used to trigger on a big
    /// layer was the hitch.
    pub fn sync_textures(&mut self, ctx: &egui::Context) {
        // Release last frame's discarded ghosts here, before anything paints
        // this frame: the meshes that referenced them were submitted a frame
        // ago, so freeing now can't invalidate a texture mid-submit.
        self.retired_textures.clear();
        self.fresh_full.clear();
        self.ensure_cell_tracking();
        self.sync_frame += 1;
        let now = self.sync_frame;

        // During an active stroke, stream only the flushed sub-rect to the
        // GPU as a partial texture update — exact WYSIWYG feedback without
        // full-buffer uploads on large canvases.
        if self.stroke.is_some() {
            let Some(target) = self.stroke_target else {
                return;
            };
            // Painting a clipped layer, or a base something is clipped to:
            // what shows is the clipped texture, so take the full path below,
            // which redoes just the changed rect of it.
            let cur = self.project.current_layer;
            let clipping = self.project.clip_base(cur).is_some()
                || (0..self.project.layers.len()).any(|i| self.project.clip_base(i) == Some(cur))
                || self.feeds_group_image(cur);
            if !clipping && self.cell_textures.contains_key(&target) {
                self.cell_tex_used.insert(target, now);
                // What the stroke flushed, and anything left over from
                // before it. Sent now, so pen-up has only the last flush left
                // to send rather than the whole stroke again.
                let pending = self.cell_dirty.insert(target, Dirty::Clean).unwrap_or(Dirty::Full);
                let dirty = match self.preview_upload_rect.take() {
                    Some(r) => pending.with(Dirty::Rect(r)),
                    None => pending,
                };
                if dirty != Dirty::Clean {
                    self.upload_cell(ctx, target, dirty);
                }
                return;
            }
            // No texture for the target cell yet — fall through so the full
            // upload below creates it.
            self.preview_upload_rect = None;
        }

        // Only what's drawn. Onion ghosts are drawn from their own silhouette
        // textures, so a ghosted cell's plain texture isn't needed at all.
        let mut needed: Vec<CellId> = Vec::new();
        let mut clipped: Vec<ClipPair> = Vec::new();
        let mut merged: Vec<usize> = Vec::new();
        let cur = self.project.current_frame;
        if !self.group_images.is_empty() {
            self.retire_group_images(None);
        }
        for (li, layer) in self.project.layers.iter().enumerate() {
            if !self.project.shows(li) {
                continue;
            }
            // A group has no drawing of its own; one drawn merged is one
            // image, its layers inside it.
            if layer.is_group() {
                if self.group_drawn_whole(li) {
                    merged.push(li);
                }
                continue;
            }
            if self.in_group_image(li) {
                continue;
            }
            // A clipped layer shows as its clipped texture instead — or not
            // at all, with its base hidden or empty here.
            if !layer.reference {
                if let Some(b) = self.project.clip_base(li) {
                    if let Some(pair) = self.clip_pair(li, b) {
                        clipped.push(pair);
                    }
                    continue;
                }
            }
            if let Some(id) = layer.resolve(cur) {
                if !needed.contains(&id) {
                    needed.push(id);
                }
            }
        }
        self.sync_group_images(ctx, now, &merged, &clipped);
        self.sync_clip_textures(ctx, now, &clipped);

        for id in needed {
            self.cell_tex_used.insert(id, now);
            if self.project.cell(id).is_none() {
                continue;
            }
            let dirty = self.cell_dirty.get(&id).copied().unwrap_or(Dirty::Full);
            self.upload_cell(ctx, id, dirty);
            self.cell_dirty.insert(id, Dirty::Clean);
        }

        // The floating selection's own texture. Rebuilt only when the pixels
        // change: a move shifts the quad, not the texture.
        match &self.selection {
            Some(sel) if self.sel_tex_stale || self.selection_tex.is_none() => {
                let dims = [sel.mask.w as usize, sel.mask.h as usize];
                let image = premultiplied_image(dims, &sel.pixels);
                match self.selection_tex.as_mut() {
                    Some(tex) if tex.size() == dims => {
                        tex.set(image, TextureOptions::LINEAR);
                        self.fresh_full.insert(tex.id());
                    }
                    _ => {
                        if let Some(old) = self.selection_tex.take() {
                            self.retired_textures.push(old);
                        }
                        self.selection_tex = Some(ctx.load_texture(
                            "selection",
                            image,
                            TextureOptions::LINEAR,
                        ));
                    }
                }
                self.sel_tex_stale = false;
            }
            None if self.selection_tex.is_some() => {
                if let Some(old) = self.selection_tex.take() {
                    self.retired_textures.push(old);
                }
            }
            _ => {}
        }

        self.sync_tint(ctx);
        self.sync_ghosts(ctx, now);
        self.enforce_texture_budget();
    }

    /// Layer `li`, clipped to `b`, as it shows on the current frame: its
    /// drawing and its base's, and where each sits. `None` when it shows
    /// nothing — base hidden, a reference, or empty here.
    fn clip_pair(&self, li: usize, b: usize) -> Option<ClipPair> {
        let p = &self.project;
        let f = p.current_frame;
        let base = p.layers.get(b)?;
        if !p.shows(b) || base.reference {
            return None;
        }
        let cell = p.layers[li].resolve(f)?;
        let xf = self.display_transform(li, f);
        if base.is_group() {
            // Cut by the group's mask, which sits in group space.
            let shows_any = p
                .children(b)
                .any(|c| p.shows(c) && !p.layers[c].reference && p.layers[c].resolve(f).is_some());
            if !shows_any {
                return None;
            }
            return Some(ClipPair {
                cell,
                base: groups::BaseKey::Group(self.group_key(b, false)),
                base_layer: b,
                at: crate::doc::clip::Placement {
                    xf,
                    base_xf: crate::doc::transform::Transform::default(),
                    pw: p.width,
                    ph: p.height,
                },
            });
        }
        Some(ClipPair {
            cell,
            base: groups::BaseKey::Cell(base.resolve(f)?),
            base_layer: b,
            at: crate::doc::clip::Placement {
                xf,
                base_xf: self.display_transform(b, f),
                pw: p.width,
                ph: p.height,
            },
        })
    }

    /// The texture the canvas draws for layer `li` on the current frame, the
    /// cell it stands for, and the rect of that cell it holds: its cell's
    /// own, or for a clipped layer, the clipped one. `None` draws nothing.
    pub fn layer_texture(&self, li: usize) -> Option<(CellId, &TextureHandle, DirtyRect)> {
        let layer = self.project.layers.get(li)?;
        match self.project.clip_base(li).filter(|_| !layer.reference) {
            Some(b) => {
                let pair = self.clip_pair(li, b)?;
                let t = self.clip_textures.get(&(pair.cell, pair.base))?;
                Some((pair.cell, &t.tex, t.rect))
            }
            None => {
                let id = layer.resolve(self.project.current_frame)?;
                let t = self.cell_textures.get(&id)?;
                Some((id, &t.tex, t.rect))
            }
        }
    }

    /// Bring the clipped drawings on screen up to date: one texture per
    /// (clipped, base) pairing, the drawing cut to its base. Only what changed
    /// in either is redone; the two moving relative to each other redoes it
    /// all. Nothing here runs for a project without clipping.
    fn sync_clip_textures(&mut self, ctx: &egui::Context, now: u64, pairs: &[ClipPair]) {
        use crate::doc::clip::mask_rect;
        for pair in pairs {
            let key = (pair.cell, pair.base);
            self.clip_tex_used.insert(key, now);
            let base = match pair.base {
                groups::BaseKey::Cell(id) => self.project.cell(id),
                groups::BaseKey::Group(k) => self.group_images.get(&k).map(|g| &g.canvas),
            };
            let (Some(clip), Some(base)) = (self.project.cell(pair.cell), base) else {
                continue;
            };
            let cell = [clip.width, clip.height];
            let aligned = pair.at.aligned(clip, base);
            let stroke = (self.stroke.is_some() && self.stroke_target == Some(pair.cell)).then_some(self.stroke_base);
            let (have, pending) = match self.clip_textures.get(&key) {
                Some(t) if t.cell == cell && t.at == pair.at => (Some(t.rect), t.pending),
                _ => (None, Dirty::Full),
            };
            match plan_upload(clip, have, pending, stroke) {
                TexPlan::Keep => {}
                TexPlan::Patch(r) => {
                    if let Some(t) = self.clip_textures.get_mut(&key) {
                        let image = premultiplied_image(tex_dims(r), &mask_rect(clip, Some(base), &pair.at, r));
                        let at = [(r.min_x - t.rect.min_x) as usize, (r.min_y - t.rect.min_y) as usize];
                        t.tex.set_partial(at, image, TextureOptions::LINEAR);
                    }
                }
                TexPlan::Rebuild(rect) => {
                    let image = crop_image(rect, || mask_rect(clip, Some(base), &pair.at, rect));
                    let name = format!("clip_{}_{}", pair.cell, pair.base_layer);
                    let tex = ctx.load_texture(name, image, TextureOptions::LINEAR);
                    let fresh = ClipTex {
                        tex,
                        rect,
                        cell,
                        at: pair.at,
                        aligned,
                        pending: Dirty::Clean,
                    };
                    if let Some(old) = self.clip_textures.insert(key, fresh) {
                        self.retired_textures.push(old.tex);
                    }
                    continue;
                }
            }
            if let Some(t) = self.clip_textures.get_mut(&key) {
                t.pending = Dirty::Clean;
                // A resized base leaves the placement alone but can end the
                // 1:1 match.
                t.aligned = aligned;
            }
        }
    }

    /// Bring cell `id`'s texture up to date with `dirty` of its pixels — see
    /// `plan_upload`.
    fn upload_cell(&mut self, ctx: &egui::Context, id: CellId, dirty: Dirty) {
        let Some(c) = self.project.cell(id) else {
            return;
        };
        let cell = [c.width, c.height];
        let stroke = (self.stroke.is_some() && self.stroke_target == Some(id)).then_some(self.stroke_base);
        // Expanding a layer's canvas resizes cells under their textures, so
        // only a texture made for this size can be patched.
        let have = self.cell_textures.get(&id).filter(|t| t.cell == cell).map(|t| t.rect);
        match plan_upload(c, have, dirty, stroke) {
            TexPlan::Keep => {}
            TexPlan::Patch(r) => {
                if let Some(t) = self.cell_textures.get_mut(&id) {
                    let image = cell_image(c, r);
                    let at = [(r.min_x - t.rect.min_x) as usize, (r.min_y - t.rect.min_y) as usize];
                    t.tex.set_partial(at, image, TextureOptions::LINEAR);
                }
            }
            TexPlan::Rebuild(rect) => {
                let image = cell_image(c, rect);
                match self.cell_textures.get_mut(&id) {
                    // Same size: re-fill the handle rather than swap it.
                    Some(t) if t.tex.size() == image.size => {
                        t.tex.set(image, TextureOptions::LINEAR);
                        self.fresh_full.insert(t.tex.id());
                        t.rect = rect;
                        t.cell = cell;
                    }
                    _ => {
                        let tex = ctx.load_texture(format!("cell_{id}"), image, TextureOptions::LINEAR);
                        if let Some(old) = self.cell_textures.insert(id, CellTex { tex, rect, cell }) {
                            self.retired_textures.push(old.tex);
                        }
                    }
                }
            }
        }
    }

    /// Onion ghosts: built small (see `GHOST_MAX_SIDE`) and kept after they
    /// leave the onion range, within the texture budget, so stepping back and
    /// forth reuses them. One that isn't built yet waits until the playhead
    /// has rested for `GHOST_REST`, so a scrub past a hundred frames doesn't
    /// build a hundred ghosts on the way.
    fn sync_ghosts(&mut self, ctx: &egui::Context, now: u64) {
        let mut ghosts: Vec<(CellId, [u8; 3])> = Vec::new();
        for li in self.onion_layers() {
            for dir in [OnionDirection::Prev, OnionDirection::Next] {
                let tint = self.onion.tint_rgb(dir);
                ghosts.extend(self.onion_steps(li, dir).into_iter().map(|s| (s.cell, tint)));
            }
            ghosts.extend(self.onion_pin_ghosts(li).into_iter().map(|(pin, cell)| (cell, pin.tint)));
        }
        ghosts.extend(self.ghost_ahead());
        // One silhouette per cell, in the first colour asked for. A drawing
        // ghosted twice in two colours — exposed both sides of the playhead,
        // or by two layers sharing it — would otherwise rebuild in one, then
        // the other, every frame.
        let mut seen = HashSet::new();
        ghosts.retain(|&(id, _)| seen.insert(id));

        let time = ctx.input(|i| i.time);
        let cur = self.project.current_frame;
        if self.playhead_rest.0 != cur {
            self.playhead_rest = (cur, time);
        }
        let rested = time - self.playhead_rest.1;

        for (id, tint) in ghosts {
            self.ghost_tex_used.insert(id, now);
            let Some(c) = self.project.cell(id) else {
                continue;
            };
            let dims = ghost_dims(c.width, c.height);
            let fresh = !self.ghost_stale.contains(&id)
                && self
                    .ghost_textures
                    .get(&id)
                    .is_some_and(|(t, tex)| *t == tint && tex.size() == dims);
            if fresh {
                continue;
            }
            if !self.ghost_textures.contains_key(&id) && rested < GHOST_REST {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(GHOST_REST - rested));
                continue;
            }
            let image = ghost_image(c, tint);
            // Same reuse rule as the cell textures: keep the handle when the
            // dimensions still match — re-uploading into it avoids freeing a
            // texture the last frame may still have queued a mesh against.
            let reusable = self
                .ghost_textures
                .get(&id)
                .is_some_and(|(_, tex)| tex.size() == dims);
            if reusable {
                if let Some((t, tex)) = self.ghost_textures.get_mut(&id) {
                    tex.set(image, TextureOptions::LINEAR);
                    self.fresh_full.insert(tex.id());
                    *t = tint;
                }
            } else {
                self.retire_ghost(id);
                let tex = ctx.load_texture(format!("ghost{id}"), image, TextureOptions::LINEAR);
                self.ghost_textures.insert(id, (tint, tex));
            }
            self.ghost_stale.remove(&id);
        }
    }

    /// Drop the least recently shown cell and ghost textures until what's
    /// resident fits `TEXTURE_BUDGET`. Nothing on screen this sync is
    /// touched. A dropped cell re-uploads in full the next time it's shown.
    fn enforce_texture_budget(&mut self) {
        let bytes = |t: &TextureHandle| t.size()[0] * t.size()[1] * 4;
        let mut entries: Vec<TexEntry> = Vec::new();
        for (&id, t) in &self.cell_textures {
            entries.push(TexEntry {
                kind: TexKind::Cell,
                id,
                used: self.cell_tex_used.get(&id).copied().unwrap_or(0),
                bytes: bytes(&t.tex),
            });
        }
        for (&id, (_, tex)) in &self.ghost_textures {
            entries.push(TexEntry {
                kind: TexKind::Ghost,
                id,
                used: self.ghost_tex_used.get(&id).copied().unwrap_or(0),
                bytes: bytes(tex),
            });
        }
        for (&key, t) in &self.clip_textures {
            entries.push(TexEntry {
                kind: TexKind::Clip(key.1),
                id: key.0,
                used: self.clip_tex_used.get(&key).copied().unwrap_or(0),
                bytes: bytes(&t.tex),
            });
        }
        for (&key, img) in &self.group_images {
            entries.push(TexEntry {
                kind: TexKind::Group(key),
                id: 0,
                used: self.group_used.get(&key).copied().unwrap_or(0),
                bytes: img.bytes(),
            });
        }
        for e in evictions(&entries, TEXTURE_BUDGET, self.sync_frame, self.playback.playing) {
            match e.kind {
                TexKind::Group(key) => self.drop_group_image(key),
                TexKind::Ghost => {
                    self.retire_ghost(e.id);
                    self.ghost_tex_used.remove(&e.id);
                }
                TexKind::Clip(base) => {
                    if let Some(t) = self.clip_textures.remove(&(e.id, base)) {
                        self.retired_textures.push(t.tex);
                    }
                    self.clip_tex_used.remove(&(e.id, base));
                }
                TexKind::Cell => {
                    if let Some(t) = self.cell_textures.remove(&e.id) {
                        self.retired_textures.push(t.tex);
                        self.cell_tex_used.remove(&e.id);
                        self.cell_dirty.insert(e.id, Dirty::Full);
                    }
                }
            }
        }
    }

    /// The layers whose onion ghosts show, bottom first: the active layer and
    /// every other one with its onion switched on, of those drawn. None while
    /// playing or with the onion skin off.
    pub fn onion_layers(&self) -> Vec<usize> {
        // Ghosts are for judging a pose against its neighbours, which playback
        // already shows; skipping them here also keeps the per-frame silhouette
        // rebuild out of the playback loop.
        if self.playback.playing || !self.onion.enabled {
            return Vec::new();
        }
        let cur = self.project.current_layer;
        (0..self.project.layers.len())
            .filter(|&li| {
                let l = &self.project.layers[li];
                (li == cur || l.onion) && !l.is_group() && self.project.shows(li)
            })
            .collect()
    }

    /// Onion ghosts for layer `li` at the current frame. Shared by the
    /// texture sync and the canvas so both agree on which cells are ghosted.
    pub fn onion_steps(&self, li: usize, dir: OnionDirection) -> Vec<OnionStep> {
        if self.playback.playing {
            return Vec::new();
        }
        let Some(layer) = self.project.layers.get(li) else {
            return Vec::new();
        };
        let mut steps = self.onion.steps(
            layer,
            self.project.current_frame,
            self.project.frame_count,
            dir,
        );
        // A pinned drawing is ghosted once, in the pin's colour: the ghost
        // cache holds one tint per cell.
        let pins = self.onion_pin_ghosts(li);
        steps.retain(|s| !pins.iter().any(|&(_, cell)| cell == s.cell));
        steps
    }

    /// Pinned onion ghosts for layer `li` at the current frame, under the
    /// same conditions as [`Self::onion_steps`] — and hidden with the rest of
    /// the onion skin when it's switched off.
    pub fn onion_pin_ghosts(&self, li: usize) -> Vec<(OnionPin, CellId)> {
        if self.playback.playing || !self.onion.enabled {
            return Vec::new();
        }
        let Some(layer) = self.project.layers.get(li) else {
            return Vec::new();
        };
        crate::timeline::onion::pin_ghosts(
            layer,
            self.project.current_frame,
            self.project.frame_count,
        )
    }

    /// Canvas opacity multiplier for layer `li` under "fade other layers":
    /// `fade.above` for layers drawn over the active one, `fade.below` for
    /// those under it, 1.0 when the fade is off. Always 1.0 for
    ///   * the active layer — it's the one being brought forward;
    ///   * any layer during playback — like onion skin, playing shows the real
    ///     mix, which is what's being checked;
    ///   * the active layer's `lines_from` layer — it bounds the fill being
    ///     painted, so it has to stay readable;
    ///   * reference layers — they already sit at their own fixed dim.
    pub fn layer_view_alpha(&self, li: usize) -> f32 {
        let cur = self.project.current_layer;
        if !self.fade_others || self.playback.playing || li == cur {
            return 1.0;
        }
        let Some(layer) = self.project.layers.get(li) else {
            return 1.0;
        };
        let lines_from = self.project.layers.get(cur).and_then(|l| l.lines_from);
        if layer.reference || lines_from == Some(li) {
            return 1.0;
        }
        let a = if li > cur { self.fade.above } else { self.fade.below };
        a.clamp(0.0, 1.0)
    }

    /// The cell a paint edit on the active slot should write to, allocating
    /// it if the slot is empty. `None` on a locked or reference layer.
    fn begin_edit_cell(&mut self) -> Option<CellId> {
        if !self.project.paintable(self.project.current_layer) {
            return None;
        }
        // Auto-key: break the hold first, so the stroke starts a drawing of
        // this frame's own instead of editing the cell every frame in the hold
        // shares. Blank, not a duplicate: the point is to draw the next
        // drawing without reaching for the X-sheet, and a duplicate would put
        // the previous drawing's strokes on the new frame. Seeing the previous
        // drawing is what onion skin is for.
        if self.auto_key_draw {
            let f = self.project.current_frame;
            let holding = self
                .project
                .layers
                .get(self.project.current_layer)
                .is_some_and(|l| l.resolve(f).is_some() && !l.is_key(f));
            if holding {
                self.structural_edit(false, |p| {
                    p.insert_blank_key_here();
                });
            }
        }
        Some(self.project.ensure_active_cell())
    }

    pub fn pointer_down(&mut self, sample: PointerSample) {
        // Before anything lands: landing would end the warp in hand.
        if self.tool == ActiveTool::Liquify {
            self.liquify_down(sample);
            return;
        }
        self.playback.stop();
        // Put floating pixels down first: a stroke is held to the selection
        // where it now is, and must not paint under pixels about to land.
        self.land_float();
        let Some(target) = self.begin_edit_cell() else {
            return;
        };
        self.stroke_target = Some(target);

        // Snapshot pre-stroke state so undo can roll back the dirty sub-rect.
        self.snapshot_pre(target);
        // The snapshot mirrors the cell, so where it holds anything bounds
        // the drawing as the stroke found it.
        self.stroke_base = self.pre_inked;
        if let Some(c) = self.project.cell_mut(target) {
            c.dirty = None;
        }

        if self.tool == ActiveTool::Fill {
            let opts = crate::tools::fill::FillOptions {
                tolerance: self.brush.fill_tolerance,
                color: self.brush.color,
                expand: self.brush.fill_expand,
                gap: self.brush.fill_gap,
                alpha_lock: self.active_alpha_lock(),
                opacity: self.brush.opacity,
                grain: self.fill_grain(target),
            };
            // Owned, so the immutable project borrow ends before `cell_mut`.
            let (cw, ch) = {
                let c = &self.project.cells[target];
                (c.width, c.height)
            };
            let boundary = self.fill_source(self.project.current_layer, cw, ch);
            let clip = self.active_cell_clip();
            let seed = (sample.x.round() as i32, sample.y.round() as i32);
            self.stroke = None;
            // Walls are read from the snapshot, never from the cell being
            // painted, so re-running the fill mid-drag sees the lines as they
            // were at the press.
            let session = crate::tools::fill::FillSession::new(
                &self.stroke_pre_pixels,
                (cw, ch),
                seed,
                &opts,
                boundary,
                clip,
            );
            let Some(mut session) = session else {
                self.stroke_target = None;
                self.stroke_pre_live = false;
                return;
            };
            let applied = self
                .project
                .cell_mut(target)
                .and_then(|c| session.apply(c, &self.stroke_pre_pixels, opts.gap, opts.expand))
                .is_some();
            if applied {
                self.mark_dirty(target);
            }
            // Undo is recorded on pointer-up, once the drag has settled.
            self.fill_drag = Some(FillDrag {
                session,
                anchor: None,
                base_gap: opts.gap,
                base_expand: opts.expand,
            });
            return;
        }

        if self.tool == ActiveTool::Shape {
            // Anchor the shape; it is rasterised into the cell on pointer-up.
            // Preview is drawn by egui shapes in `paint_canvas` until then.
            self.shape_drag = Some(ShapeDrag {
                kind: self.brush.shape_kind,
                start: (sample.x, sample.y),
                end: (sample.x, sample.y),
                plane: self.shape_plane(target),
            });
            self.stroke = None;
            return;
        }

        let (cw, ch) = {
            let c = &self.project.cells[target];
            (c.width, c.height)
        };
        self.stroke_ws
            .begin(cw, ch, &self.brush);
        let clip = self.active_cell_clip();
        self.stroke_ws.set_clip(clip);
        self.stroke_ws.set_alpha_lock(self.active_alpha_lock());

        // Resolve the radius and the view scale once, here: a stroke must not
        // change width or smoothing behaviour partway through if the view moves.
        let mut brush = self.brush.clone();
        brush.radius = self.effective_radius();
        let krita = self.krita_brush();
        let mut builder =
            StrokeBuilder::new_with_krita(brush, self.tool, self.cell_view_scale(), self.smoothing, krita);
        let (rotation, mirrored) = self.cell_screen_orientation();
        builder.orient(rotation, mirrored);
        self.stroke_clock = Some(Instant::now());
        builder.tick(0.0);
        builder.push(sample);
        self.stroke = Some(builder);
        self.flush_stroke(target);
    }

    /// Paint what the stroke on `target` has decided so far, and mark just
    /// that for upload. Marking the stroke's whole rect instead, as
    /// `mark_dirty` would, re-sent everything already streamed at pen-up.
    fn flush_stroke(&mut self, target: CellId) {
        let (Some(builder), Some(pre), Some(c)) = (
            self.stroke.as_mut(),
            self.stroke_pre_live.then_some(&self.stroke_pre_pixels[..]),
            self.project.cells.get_mut(target).map(Arc::make_mut),
        ) else {
            return;
        };
        if let Some(r) = builder.flush(c, &mut self.stroke_ws, pre) {
            self.preview_upload_rect = Some(union_rect(self.preview_upload_rect, r));
            self.mark_region(target, Dirty::Rect(r));
            // Marking forgets whether the cell is blank. Paint says it isn't;
            // asking again had the timeline scan the whole cell every frame.
            if self.tool != ActiveTool::Eraser {
                self.painted(target);
            }
        }
    }

    pub fn pointer_move(&mut self, sample: PointerSample) {
        if self.tool == ActiveTool::Liquify {
            self.liquify_move(sample);
            return;
        }
        let Some(target) = self.stroke_target else {
            return;
        };
        if let Some(drag) = &mut self.shape_drag {
            drag.end = (sample.x, sample.y);
            return;
        }
        let Some(builder) = &mut self.stroke else {
            return;
        };
        builder.push(sample);
        if let Some(t0) = self.stroke_clock {
            builder.tick(t0.elapsed().as_secs_f64() * 1000.0);
        }
        self.flush_stroke(target);
    }

    pub fn pointer_up(&mut self) {
        self.finish_grid_drag();
        self.snap_lock = None;
        // A warp has no stroke to finish, and its snapshot must outlive the
        // pen-up: the session goes on until it lands.
        if self.liquify.is_some() {
            self.liquify_up();
            return;
        }
        if self.finish_fill() {
            return;
        }
        let Some(target) = self.stroke_target.take() else {
            self.stroke = None;
            self.shape_drag = None;
            self.stroke_pre_live = false;
            self.preview_upload_rect = None;
            return;
        };
        let mut paints = false;
        if let Some(drag) = self.shape_drag.take() {
            // Rasterise the final shape now; undo records the dirty rect below.
            let outline = self.shape_outline(&drag);
            let mut brush = self.brush.clone();
            brush.radius = self.effective_radius();
            let (cw, ch) = {
                let c = &self.project.cells[target];
                (c.width, c.height)
            };
            self.stroke_ws.begin(cw, ch, &brush);
            let clip = self.active_cell_clip();
            self.stroke_ws.set_clip(clip);
            self.stroke_ws.set_alpha_lock(self.active_alpha_lock());
            if let (Some(pre), Some(c)) = (
                self.stroke_pre_live.then_some(&self.stroke_pre_pixels[..]),
                self.project.cell_mut(target),
            ) {
                crate::tools::shape::rasterize(c, &mut self.stroke_ws, pre, &outline, &brush);
            }
            self.mark_dirty(target);
            paints = true;
        } else if let Some(mut builder) = self.stroke.take() {
            if let Some(t0) = self.stroke_clock.take() {
                builder.tick(t0.elapsed().as_secs_f64() * 1000.0);
            }
            let mut last = None;
            if let (Some(pre), Some(c)) = (
                self.stroke_pre_live.then_some(&self.stroke_pre_pixels[..]),
                self.project.cell_mut(target),
            ) {
                last = builder.finish(c, &mut self.stroke_ws, pre);
            }
            // Only the last of it: the rest went up while it was drawn, and
            // what hadn't yet is still marked.
            if let Some(r) = last {
                self.mark_region(target, Dirty::Rect(r));
            }
            paints = self.tool != ActiveTool::Eraser;
        }
        self.preview_upload_rect = None;
        // Only a stroke that changed something marks the drawing as painted:
        // one the selection clipped away entirely left the cell as it was.
        if self.commit_undo(target) && paints {
            self.painted(target);
        }
    }

    /// Remember where on screen a Fill press landed: its drag counts from here.
    pub fn fill_anchor(&mut self, at: egui::Pos2) {
        if let Some(d) = &mut self.fill_drag {
            d.anchor = Some(at);
        }
    }

    /// The pen, still down on a Fill press, is at screen point `at`. Sideways
    /// sets the gap and up and down the expand, each in steps away from its
    /// value at the press; a change re-runs the fill so the canvas follows.
    /// The new values stay in the sliders.
    ///
    /// Expand steps a pixel at a time. The gap climbs a ladder whose rungs
    /// spread out as it grows, so its whole range fits in a drag.
    pub fn fill_drag_to(&mut self, at: egui::Pos2) {
        use crate::input::button_drag::drag_steps;
        use crate::tools::fill::{step_gap, MAX_EXPAND};
        let (Some(drag), Some(target)) = (&mut self.fill_drag, self.stroke_target) else {
            return;
        };
        let Some(anchor) = drag.anchor else {
            return;
        };
        let gap = step_gap(drag.base_gap, drag_steps(at.x - anchor.x));
        // Screen y grows downward, and up means more.
        let expand = (drag.base_expand as i32 + drag_steps(anchor.y - at.y))
            .clamp(0, MAX_EXPAND as i32) as u8;
        if gap == self.brush.fill_gap && expand == self.brush.fill_expand {
            return;
        }
        self.brush.fill_gap = gap;
        self.brush.fill_expand = expand;
        // Only this step's pixels go up to the GPU; pointer-up widens the
        // dirty rect back to the whole press for the undo record.
        let applied = self.project.cell_mut(target).is_some_and(|c| {
            c.dirty = None;
            drag.session
                .apply(c, &self.stroke_pre_pixels, gap, expand)
                .is_some()
        });
        if applied {
            self.mark_dirty(target);
        }
    }

    /// Whether the active layer's alpha is locked.
    pub fn active_alpha_lock(&self) -> bool {
        self.project
            .layers
            .get(self.project.current_layer)
            .is_some_and(|l| l.alpha_lock)
    }

    /// Whether a Fill press is still under the pen.
    pub fn fill_dragging(&self) -> bool {
        self.fill_drag.is_some()
    }

    /// Land a Fill press as one undo step covering everything its drag
    /// touched. `true` if there was one.
    fn finish_fill(&mut self) -> bool {
        let Some(drag) = self.fill_drag.take() else {
            return false;
        };
        self.preview_upload_rect = None;
        let Some(target) = self.stroke_target.take() else {
            self.stroke_pre_live = false;
            return true;
        };
        if let Some(c) = self.project.cell_mut(target) {
            c.dirty = drag.session.touched();
        }
        if self.commit_undo(target) {
            self.painted(target);
        }
        true
    }

    /// Abandon a Fill press still under the pen: the cell goes back to how it
    /// was, and no undo step is left. `true` if there was one.
    pub fn cancel_fill(&mut self) -> bool {
        let Some(mut drag) = self.fill_drag.take() else {
            return false;
        };
        if let Some(target) = self.stroke_target.take() {
            let reverted = self.stroke_pre_live
                && self.project.cell_mut(target).is_some_and(|c| {
                    c.dirty = None;
                    drag.session.revert(c, &self.stroke_pre_pixels).is_some()
                });
            if reverted {
                self.mark_dirty(target);
            }
        }
        self.stroke_pre_live = false;
        true
    }

    /// Snapshot `cell`'s pixels as they are before an edit, for the stroke
    /// compositor and for undo, into the kept buffer.
    ///
    /// Usually there's nothing to copy: the buffer still mirrors the cell
    /// the last edit landed on (see `pre_mirror`), and drawing goes on in
    /// one drawing for many strokes. Otherwise only what either drawing
    /// holds is touched — the old one's ink cleared, the new one's copied
    /// from where it's packed — not the whole canvas.
    fn snapshot_pre(&mut self, cell: CellId) {
        self.stroke_pre_live = true;
        self.pre_cell = Some(cell);
        let mirror = self.pre_mirror.take();
        let Some((len, width, rev)) = self.project.cell(cell).map(|c| (c.byte_len(), c.width, c.rev())) else {
            self.stroke_pre_pixels.clear();
            self.pre_inked = None;
            return;
        };
        if self.stroke_pre_pixels.len() != len || self.pre_w != width {
            // Fresh zeroed pages cost nothing until they're written.
            self.stroke_pre_pixels = vec![0; len];
            self.pre_inked = None;
            self.pre_w = width;
        } else if mirror == Some((cell, rev)) {
            return;
        } else {
            self.clear_pre();
        }
        let c = &self.project.cells[cell];
        let w = width as usize * 4;
        self.pre_inked = c.held_rect();
        if let Some(r) = self.pre_inked {
            let rows = &mut self.stroke_pre_pixels[r.min_y as usize * w..r.max_y as usize * w];
            par_rows(rows, w, |first, band| {
                for (y, out) in (r.min_y + first as u32..).zip(band.chunks_exact_mut(w)) {
                    let (x0, span) = c.row_span(y);
                    out[x0 as usize * 4..x0 as usize * 4 + span.len()].copy_from_slice(span);
                }
            });
        }
    }

    /// Clear what `stroke_pre_pixels` holds, back to all zeros.
    fn clear_pre(&mut self) {
        self.pre_mirror = None;
        let Some(r) = self.pre_inked.take() else {
            return;
        };
        let (x0, x1, w) = (r.min_x as usize * 4, r.max_x as usize * 4, self.pre_w as usize * 4);
        let rows = &mut self.stroke_pre_pixels[r.min_y as usize * w..r.max_y as usize * w];
        par_rows(rows, w, |_, band| {
            for out in band.chunks_exact_mut(w) {
                out[x0..x1].fill(0);
            }
        });
    }

    /// Where the pen would paint if it went down now: `Some(Some(id))` on
    /// drawing `id`, `Some(None)` on a new blank one (what auto-key or an
    /// empty slot makes), and `None` nowhere. See `begin_edit_cell`.
    fn pen_cell(&self) -> Option<Option<CellId>> {
        let p = &self.project;
        let l = p.layers.get(p.current_layer)?;
        if !p.paintable(p.current_layer) {
            return None;
        }
        match self.held_for_auto_key() {
            Some(_) => Some(None),
            None => Some(l.resolve(p.current_frame)),
        }
    }

    /// The drawing this frame holds when the pen, with auto-key on, would
    /// start a new one here instead: the drawing about to become the one
    /// before. See `begin_edit_cell`.
    fn held_for_auto_key(&self) -> Option<CellId> {
        let p = &self.project;
        let l = p.layers.get(p.current_layer)?;
        let f = p.current_frame;
        if !self.auto_key_draw || !p.paintable(p.current_layer) || l.is_key(f) {
            return None;
        }
        l.resolve(f)
    }

    /// The onion ghost the next stroke would bring on: with auto-key about
    /// to start a new drawing here, the held one becomes the drawing before,
    /// ghosted. Built ahead with the others while the playhead rests, so the
    /// first frame of the stroke doesn't build it — on a big 8K drawing,
    /// tens of ms.
    fn ghost_ahead(&self) -> Option<(CellId, [u8; 3])> {
        let on = self.onion.enabled
            && self.onion.is_shown(1, OnionDirection::Prev)
            && !self.playback.playing;
        let id = self.held_for_auto_key().filter(|_| on)?;
        Some((id, self.onion.tint_rgb(OnionDirection::Prev)))
    }

    /// Get the drawing the pen would land on ready to paint, once it has
    /// been that one for `WARM_REST`, so pen-down doesn't pay for it: on a
    /// big 8K drawing that was 30–60 ms. Its stroke snapshot taken, it
    /// unpacked — kept so it packs again for free if nothing gets drawn —
    /// and the last stroke's coverage cleared. For a new blank drawing, the
    /// snapshot cleared. Only for the tools that paint, and not while an
    /// edit holds the snapshot, nor while playing or flipping through
    /// frames, which would pay for every drawing passed.
    fn warm_pre(&mut self, ctx: &egui::Context) {
        let paints = matches!(
            self.tool,
            ActiveTool::Pencil
                | ActiveTool::Ink
                | ActiveTool::Eraser
                | ActiveTool::Fill
                | ActiveTool::Shape
                | ActiveTool::Liquify
        );
        let busy = self.stroke_pre_live || self.playback.playing || self.selection.is_some() || !paints;
        // A warp works on the drawing that is there, auto-key or not.
        let target = if self.tool == ActiveTool::Liquify {
            let p = &self.project;
            p.paintable(p.current_layer).then(|| p.resolved_current())
        } else {
            self.pen_cell()
        };
        let Some(target) = target.filter(|_| !busy) else {
            self.warm_wait = None;
            return;
        };
        let ready = self.stroke_ws.is_clear()
            && match target {
                Some(id) => self
                    .project
                    .cell(id)
                    .is_some_and(|c| !c.is_packed() && self.pre_mirror == Some((id, c.rev()))),
                None => self.pre_inked.is_none(),
            };
        if ready {
            self.warm_wait = None;
            return;
        }
        let now = ctx.input(|i| i.time);
        let since = match self.warm_wait {
            Some((t, since)) if t == target => since,
            _ => {
                self.warm_wait = Some((target, now));
                now
            }
        };
        let left = WARM_REST - (now - since);
        if left > 0.0 {
            ctx.request_repaint_after(Duration::from_secs_f64(left));
            return;
        }
        self.warm_wait = None;
        self.stroke_ws.clear_last();
        match target {
            Some(id) => {
                // Snapshot first, from the packed drawing: just its rect.
                self.snapshot_pre(id);
                self.stroke_pre_live = false;
                if let Some(c) = self.project.cell_mut(id) {
                    c.unpack();
                }
                self.pre_mirror = self.project.cell(id).map(|c| (id, c.rev()));
            }
            None => self.clear_pre(),
        }
    }

    /// `stroke_pre_pixels` was swapped for another snapshot of `cell` — the
    /// cell as it was before a float lifted — so all that's known of it is
    /// that it may hold anything, anywhere in that cell.
    pub(crate) fn pre_replaced(&mut self, cell: CellId) {
        self.pre_mirror = None;
        self.pre_cell = Some(cell);
        self.pre_w = self.project.cell(cell).map_or(0, |c| c.width);
        self.pre_inked = self.project.cell(cell).map(|c| DirtyRect {
            min_x: 0,
            min_y: 0,
            max_x: c.width,
            max_y: c.height,
        });
    }

    /// Push a PixelPatch covering the dirty rect accumulated since the last
    /// `stroke_pre_pixels` snapshot. `true` if anything changed.
    fn commit_undo(&mut self, cell: CellId) -> bool {
        match self.take_patch(cell) {
            Some(cmd) => {
                self.history.push(cmd);
                true
            }
            None => false,
        }
    }

    /// The PixelPatch `commit_undo` would push, for a caller that records it
    /// as part of something larger. `None` when the edit changed nothing.
    fn take_patch(&mut self, cell: CellId) -> Option<undo::Command> {
        if !std::mem::take(&mut self.stroke_pre_live) {
            return None;
        }
        let canvas = self.project.cells.get(cell)?;
        let rect = canvas.dirty?;
        let w = rect.max_x.saturating_sub(rect.min_x);
        let h = rect.max_y.saturating_sub(rect.min_y);
        if w == 0 || h == 0 {
            return None;
        }

        // The before from the snapshot and the after from the cell, each
        // split across threads: on a stroke across an 8K cell these are tens
        // of MB. Then, the edit having landed, the after goes into the
        // snapshot. Past `rect` it already matches the cell, so it then
        // mirrors the cell and the next edit of this cell copies nothing.
        let pre = &mut self.stroke_pre_pixels;
        let (cw, row, x0) = (canvas.width as usize * 4, w as usize * 4, rect.min_x as usize * 4);
        // The snapshot's rows the rect spans, whole.
        let span = rect.min_y as usize * cw..rect.max_y as usize * cw;
        let mut after = vec![0u8; row * h as usize];
        par_rows(&mut after, row, |first, band| {
            for (y, out) in (rect.min_y + first as u32..).zip(band.chunks_exact_mut(row)) {
                canvas.read_row_into(y, rect.min_x, out);
            }
        });
        let mut before = vec![0u8; row * h as usize];
        let was = &pre[span.clone()];
        par_rows(&mut before, row, |first, band| {
            for (i, out) in (first..).zip(band.chunks_exact_mut(row)) {
                out.copy_from_slice(&was[i * cw + x0..i * cw + x0 + row]);
            }
        });
        let changed = before != after;
        if self.pre_cell == Some(cell) && pre.len() == canvas.byte_len() {
            let after = &after;
            par_rows(&mut pre[span], cw, |first, band| {
                for (i, out) in (first..).zip(band.chunks_exact_mut(cw)) {
                    out[x0..x0 + row].copy_from_slice(&after[i * row..(i + 1) * row]);
                }
            });
            self.pre_inked = Some(union_rect(self.pre_inked, rect));
            self.pre_mirror = Some((cell, canvas.rev()));
        }

        // Skip recording no-op strokes (before == after).
        if !changed {
            return None;
        }

        Some(undo::Command::PixelPatch {
            cell,
            x: rect.min_x,
            y: rect.min_y,
            w,
            h,
            before,
            after,
        })
    }

    /// Enter live screen-pick mode, arming so the press that opened the mode is
    /// consumed before a tap commits.
    ///
    /// The backdrop is left exactly as the user set it. Sampling reads the OS
    /// framebuffer, so whatever is actually on screen is what gets picked —
    /// an opaque backdrop simply means the canvas is sampled instead of the
    /// desktop behind it.
    pub fn begin_screen_pick(&mut self) {
        if self.screen_pick {
            return;
        }
        // Picking is a momentary mode, not a tool: return to whatever was
        // active once a colour is committed.
        self.screen_pick_return_tool = self.tool;
        self.screen_pick = true;
        self.screen_pick_arm = true;
        // Drop any in-flight stroke so the gesture that toggled the mode can't
        // leave ink.
        self.cancel_fill();
        self.stroke = None;
        self.stroke_target = None;
        self.shape_drag = None;
        self.fill_drag = None;
        self.cancel_gesture();
        self.preview_upload_rect = None;
        self.land_float();
    }

    /// Leave screen-pick mode. Does not change the colour, tool or backdrop —
    /// used for cancel (Esc / toggle off).
    pub fn end_screen_pick(&mut self) {
        if !self.screen_pick {
            return;
        }
        self.screen_pick = false;
        self.screen_pick_arm = false;
        // NB: do NOT free `screen_pick_tex` here. End can be called mid-frame
        // (on a commit tap) *after* the loupe already painted with that texture
        // this frame; dropping the handle now makes wgpu submit a render pass
        // referencing a destroyed texture → validation panic. The handle is
        // tiny (≈25×25) and reused via `.set()` on the next pick, so keep it.
    }

    /// Take the active slot's drawing into the clipboard, blanking the frame.
    /// Refuses on a locked or reference layer, like every other edit.
    pub fn cut_cell(&mut self) {
        if self.active_layer_locked() || self.active_layer_is_reference() {
            return;
        }
        // Read before the edit: `structural_edit` borrows the project mutably.
        let taken = self.project.copy_active_cell();
        if taken.is_none() {
            return;
        }
        self.cell_clip = taken;
        self.structural_edit(false, |p| {
            p.cut_active_cell();
        });
    }

    /// Key the clipboard drawing at the active slot.
    pub fn paste_cell(&mut self) {
        if self.active_layer_locked() || self.active_layer_is_reference() {
            return;
        }
        let Some(src) = self.cell_clip.clone() else {
            return;
        };
        // Only ever allocates a cell, never mutates an existing one, so the
        // cheap `TimelineState` snapshot is a correct undo (same contract as
        // `merge_layer_down`).
        self.structural_edit(false, |p| {
            p.paste_cell_here(&src);
        });
    }

    // --- Timeline tracks ---

    /// Forget everything the tracks hold about the project: selection, drag,
    /// and which cells are blank. For when the project itself is replaced.
    fn clear_track_state(&mut self) {
        self.track_sel.clear();
        self.track_anchor = None;
        self.track_drag = None;
        self.blank_cache.clear();
    }

    /// The track selection as (layer, key frame) pairs, in sheet order.
    pub fn track_selection(&self) -> Vec<(usize, usize)> {
        self.track_sel.iter().copied().collect()
    }

    /// Scans `cell_is_blank` may start in one frame. A blank cell has to be
    /// read to the end, and one on a 3×-wide 4K layer is ~100 MB.
    const BLANK_SCANS_PER_FRAME: u32 = 1;

    /// Whether cell `id` has no painted pixel. `None` when that isn't known
    /// yet and this frame's scans are spent: the caller shows it as a drawing
    /// and asks for another frame.
    ///
    /// Alpha only — an erased pixel can keep its colour at zero coverage.
    pub fn cell_is_blank(&mut self, id: CellId) -> Option<bool> {
        if let Some(&blank) = self.blank_cache.get(&id) {
            return Some(blank);
        }
        if self.blank_scans_left == 0 {
            return None;
        }
        self.blank_scans_left -= 1;
        let blank = self.project.cell(id).map_or(true, Canvas::is_blank);
        self.blank_cache.insert(id, blank);
        Some(blank)
    }

    /// Record that a paint edit just laid coverage on `id`, so the tracks know
    /// it isn't blank without reading the cell back. Only for edits that add
    /// paint: an eraser or a delete can empty a cell, and those leave the
    /// answer to a scan.
    fn painted(&mut self, id: CellId) {
        if self.project.cell(id).is_some_and(|c| c.dirty.is_some()) {
            self.blank_cache.insert(id, false);
        }
    }

    /// Whether every one of `layers` may be edited.
    fn layers_editable(&self, layers: impl IntoIterator<Item = usize>) -> bool {
        layers.into_iter().all(|i| self.project.paintable(i))
    }

    /// Run a track edit as one undo step, then select what it hands back.
    fn track_edit(&mut self, edit: impl FnOnce(&mut Project) -> Vec<(usize, usize)>) {
        let mut selected = Vec::new();
        self.structural_edit(false, |p| selected = edit(p));
        self.track_sel = selected.into_iter().collect();
    }

    /// Drop the selected drawings `dl` layers and `df` frames away, moving or
    /// copying them, and put the cursor on `cursor` (layer, frame).
    pub fn move_track_selection(&mut self, dl: isize, df: isize, copy: bool, cursor: (usize, usize)) {
        let starts = self.track_selection();
        if !self.project.can_move_blocks(&starts, dl, df, copy) {
            return;
        }
        self.track_edit(|p| {
            let landed = p.move_blocks(&starts, dl, df, copy).unwrap_or_default();
            // Inside the edit, so undo puts the cursor back too.
            p.current_layer = cursor.0.min(p.layers.len().saturating_sub(1));
            p.goto(cursor.1);
            landed
        });
    }

    /// Hold the drawing `b` for `new_len` frames, sliding the ones after it.
    pub fn retime_track_block(&mut self, b: Block, new_len: usize) {
        let old = b.len.unwrap_or(self.project.frame_count.saturating_sub(b.start));
        if new_len.max(1) == old || !self.layers_editable([b.layer]) {
            return;
        }
        self.track_edit(|p| match p.block_at(b.layer, b.start).filter(|x| x.start == b.start) {
            Some(b) => {
                p.retime_block(b, new_len);
                vec![(b.layer, b.start)]
            }
            None => Vec::new(),
        });
    }

    /// Put every selected drawing on `n`s.
    pub fn set_track_timing(&mut self, n: usize) {
        let starts = self.track_selection();
        if starts.is_empty() || !self.layers_editable(starts.iter().map(|s| s.0)) {
            return;
        }
        self.track_edit(|p| p.set_timing(&starts, n));
    }

    /// Delete the selected drawings, leaving their frames blank.
    pub fn clear_track_selection(&mut self) {
        let starts = self.track_selection();
        if starts.is_empty() || !self.layers_editable(starts.iter().map(|s| s.0)) {
            return;
        }
        self.track_edit(|p| p.clear_blocks(&starts));
    }

    /// Delete the selected drawings and close the gap they leave.
    pub fn close_track_selection(&mut self) {
        let starts = self.track_selection();
        if starts.is_empty() || !self.layers_editable(starts.iter().map(|s| s.0)) {
            return;
        }
        self.track_edit(|p| {
            p.close_blocks(&starts);
            Vec::new()
        });
    }

    /// Insert `n` held frames on the active layer after the playhead.
    pub fn insert_layer_frames_here(&mut self, n: usize) {
        let (layer, frame) = (self.project.current_layer, self.project.current_frame);
        if !self.layers_editable([layer]) {
            return;
        }
        self.track_edit(|p| {
            p.insert_layer_frames(layer, frame, n);
            Vec::new()
        });
    }

    /// Remove `n` frames from the active layer, starting at the playhead.
    pub fn remove_layer_frames_here(&mut self, n: usize) {
        let (layer, frame) = (self.project.current_layer, self.project.current_frame);
        if !self.layers_editable([layer]) {
            return;
        }
        self.track_edit(|p| {
            p.remove_layer_frames(layer, frame, n);
            Vec::new()
        });
    }

    fn active_layer_is_reference(&self) -> bool {
        self.project
            .layers
            .get(self.project.current_layer)
            .map(|l| l.reference)
            .unwrap_or(false)
    }

    fn frame_size(&self) -> (f32, f32) {
        (self.project.width as f32, self.project.height as f32)
    }

    /// Whether the active grid is up for snapping to: switched on, shown, and
    /// not hidden itself.
    fn grid_snap_on(&self) -> bool {
        self.perspective.snap
            && self.perspective.show
            && self.perspective.active_grid().is_some_and(|g| g.visible)
    }

    /// Whether strokes should snap to a grid direction right now. A shape
    /// line counts as a stroke; rectangles and ellipses lie on the plane
    /// instead (see [`Self::shape_plane`]).
    pub fn snapping(&self) -> bool {
        let stroke_tool = match self.tool {
            ActiveTool::Pencil | ActiveTool::Ink | ActiveTool::Eraser => true,
            ActiveTool::Shape => self.brush.shape_kind == ShapeKind::Line,
            _ => false,
        };
        stroke_tool && self.grid_snap_on()
    }

    /// Whether a shape drag started now would lie on the active grid — the
    /// Shape tool options show it.
    pub fn shapes_on_grid(&self) -> bool {
        self.tool == ActiveTool::Shape
            && self.brush.shape_kind != ShapeKind::Line
            && self.grid_snap_on()
    }

    /// The active grid's plane in cell `target`'s pixels, for a shape drag
    /// starting now, or `None` when the shape is drawn flat. The layer
    /// transform is a similarity, so the grid's corners carried through it
    /// still span a plane — and the drag never has to leave cell space.
    fn shape_plane(&self, target: CellId) -> Option<GridPlane> {
        if !self.shapes_on_grid() {
            return None;
        }
        let g = self.perspective.active_grid()?;
        let (pw, ph) = self.frame_size();
        let c = &self.project.cells[target];
        let (cw, ch) = (c.width as f32, c.height as f32);
        let f = self.project.current_frame;
        let t = self.display_transform(self.project.current_layer, f);
        let corners = g.doc_corners(&self.grid_space(g, f)).map(|p| {
            let (x, y) = t.doc_to_cell(p[0], p[1], cw, ch, pw, ph);
            [x, y]
        });
        Some(GridPlane {
            h: perspective::Homography::square_to_quad(&corners)?,
            rows: g.rows,
            cols: g.cols,
        })
    }

    /// The polyline a shape drag draws, in cell pixels: on its grid when it
    /// has one and the box stays in front of the horizon, flat otherwise.
    /// Shift, held once the drag is under way, squares it up on the grid.
    /// Preview and pixels both come from here, so they always agree.
    pub fn shape_outline(&self, drag: &ShapeDrag) -> Vec<(f32, f32)> {
        let radius = self.effective_radius();
        drag.plane
            .and_then(|plane| {
                crate::tools::shape::outline_on_plane(
                    drag.kind,
                    drag.start,
                    drag.end,
                    &plane,
                    self.shift_held,
                    radius,
                )
            })
            .unwrap_or_else(|| crate::tools::shape::outline(drag.kind, drag.start, drag.end, radius))
    }

    /// Start of a stroke at document point `p`. Arms perspective snap when it
    /// is on; the first point itself is never moved.
    pub fn snap_begin(&mut self, p: [f32; 2]) -> [f32; 2] {
        self.snap_lock = self.snapping().then_some(SnapLock { start: p, dir: None });
        p
    }

    /// A stroke point at document point `p`, snapped to the active grid.
    ///
    /// The direction is picked once, when the pointer is far enough from the
    /// start to show where it is heading, and then held for the whole stroke —
    /// re-picking per point would let a wobbly hand flip between families.
    /// Until then every point stays on the start.
    pub fn snap_doc(&mut self, p: [f32; 2]) -> [f32; 2] {
        const DECIDE_PX: f32 = 10.0;
        let Some(mut lock) = self.snap_lock else {
            return p;
        };
        let dir = match lock.dir {
            Some(d) => d,
            None => {
                let motion = [p[0] - lock.start[0], p[1] - lock.start[1]];
                if motion[0].hypot(motion[1]) * self.view_scale < DECIDE_PX {
                    return lock.start;
                }
                let f = self.project.current_frame;
                let dirs = self
                    .perspective
                    .active_grid()
                    .map(|g| {
                        let s = self.grid_space(g, f);
                        g.snap_dirs(&s, lock.start, self.perspective.snap_vertical)
                    })
                    .unwrap_or_default();
                let Some(d) = perspective::pick_dir(&dirs, motion) else {
                    // A degenerate grid has nothing to snap to.
                    self.snap_lock = None;
                    return p;
                };
                lock.dir = Some(d);
                self.snap_lock = Some(lock);
                d
            }
        };
        perspective::project(lock.start, dir, p)
    }

    /// Most swatches kept. Past this the oldest is dropped, so the strip stays
    /// one or two rows and never eats the Brush panel.
    pub const MAX_SWATCHES: usize = 32;

    /// Set the brush colour everywhere it is mirrored. `brush` alone is not
    /// enough: the per-tool copy in `tool_brushes` is what a tool switch
    /// restores from, so writing only one of them loses the colour on the next
    /// tool change. Same reasoning as `commit_screen_pick`.
    pub fn set_brush_color(&mut self, rgb: [u8; 3]) {
        let c = [rgb[0], rgb[1], rgb[2], 255];
        self.brush.color = c;
        if self.wheel_pinned {
            for b in &mut self.tool_brushes {
                b.color = c;
            }
        } else {
            let idx = self.tool.idx();
            self.tool_brushes[idx].color = c;
        }
    }

    /// Whether the colour wheel is pinned.
    pub fn wheel_pinned(&self) -> bool {
        self.wheel_pinned
    }

    /// Pin the colour wheel, or let it go. Pinning hands the colour in use
    /// to every tool; letting go leaves each tool with it, to drift apart
    /// again from there.
    pub fn set_wheel_pinned(&mut self, on: bool) {
        self.wheel_pinned = on;
        if on {
            let c = self.brush.color;
            self.set_brush_color([c[0], c[1], c[2]]);
        }
    }

    /// What the swatch pie offers, and how many of those its inner ring
    /// takes: the pinned swatches, or with the wheel pinned its set first,
    /// on the inner ring by itself.
    pub fn pie_colors(&self) -> (Vec<[u8; 3]>, usize) {
        if !self.wheel_pinned {
            return (self.palette.clone(), crate::input::button_drag::PIE_INNER);
        }
        let mut colors: Vec<[u8; 3]> = self.harmony.members().iter().map(|m| m.to_rgb()).collect();
        let set = colors.len();
        colors.extend_from_slice(&self.palette);
        (colors, set)
    }

    /// Write the active brush back to its slot, for a tool switch or a save
    /// to restore from. Shape has no size of its own, so its Thickness goes
    /// to the pen it follows too.
    pub(crate) fn stash_brush(&mut self) {
        self.tool_brushes[self.tool.idx()] = self.brush.clone();
        match self.tool {
            ActiveTool::Pencil | ActiveTool::Ink => self.last_pen = self.tool,
            ActiveTool::Shape => self.tool_brushes[self.last_pen.idx()].radius = self.brush.radius,
            _ => {}
        }
    }

    /// Make `t` the active tool with its stored brush. Shape draws at the
    /// size of the pen used last, not at a size of its own.
    pub(crate) fn load_brush(&mut self, t: ActiveTool) {
        // A pinned wheel's colour goes with the pen to the next tool, however
        // it was set.
        let shared = self.wheel_pinned.then_some(self.brush.color);
        self.tool = t;
        self.brush = self.tool_brushes[t.idx()].clone();
        if let Some(c) = shared {
            self.brush.color = c;
            self.tool_brushes[t.idx()].color = c;
        }
        if t == ActiveTool::Shape {
            self.brush.radius = self.tool_brushes[self.last_pen.idx()].radius;
        }
    }

    /// Pin the current brush colour. Re-pinning a colour already held is a
    /// no-op rather than a duplicate.
    pub fn pin_swatch(&mut self) {
        let c = self.brush.color;
        self.pin_rgb([c[0], c[1], c[2]]);
    }

    /// Pin `rgb` unless it is already held; past the cap the oldest drops.
    pub fn pin_rgb(&mut self, rgb: [u8; 3]) {
        if self.palette.contains(&rgb) {
            return;
        }
        self.palette.push(rgb);
        if self.palette.len() > Self::MAX_SWATCHES {
            self.palette.remove(0);
        }
    }

    /// Pin every colour of a harmony set, in order, skipping ones held.
    pub fn pin_set(&mut self, colors: &[[u8; 3]]) {
        for &rgb in colors {
            self.pin_rgb(rgb);
        }
    }

    /// Commit a sampled colour: apply it to every tool, restore the drawing tool
    /// that was active before picking, and leave pick mode.
    pub fn commit_screen_pick(&mut self, color: [u8; 4]) {
        // The active brush may have changed since its slot was last written
        // (a preset, a slider); keep that before reloading from the slots.
        self.stash_brush();
        // Apply to the active brush and every tool's stored colour so the sample
        // sticks across tool switches.
        for b in &mut self.tool_brushes {
            b.color = color;
        }
        let tool = self.screen_pick_return_tool;
        self.load_brush(tool);
        self.brush.color = color;
        self.tool_brushes[tool.idx()].color = color;
        self.end_screen_pick();
    }

    #[allow(dead_code)]
    pub fn apply_picked_color(&mut self, color: [u8; 4]) {
        self.brush.color = color;
        for b in &mut self.tool_brushes {
            b.color = color;
        }
    }

    /// The pixels a Fill press on `layer_idx` reads its walls from, in the
    /// *cell space* of that layer's own cell: every visible layer at once when
    /// the Fill tool asks for that, else the `lines_from` link. `None` reads
    /// the cell's own pixels.
    fn fill_source(&self, layer_idx: usize, cell_w: u32, cell_h: u32) -> Option<Canvas> {
        if self.brush.fill_all_visible {
            self.visible_lines(layer_idx, cell_w, cell_h)
        } else {
            self.fill_boundary(layer_idx, cell_w, cell_h)
        }
    }

    /// Every visible layer at the current frame, reference layers aside,
    /// composited in document space at its own opacity — what the artist sees,
    /// less the view's fades — then brought into the cell space of
    /// `layer_idx`. Built once per press, and only when asked for.
    fn visible_lines(&self, layer_idx: usize, cell_w: u32, cell_h: u32) -> Option<Canvas> {
        let p = &self.project;
        let frame = p.current_frame;
        let dst_xform = p.layers.get(layer_idx)?.resolve_transform(frame);
        let doc = crate::io::composite::flatten_doc(p, frame);
        Some(doc_to_cell(doc, &dst_xform, cell_w, cell_h, p.width, p.height))
    }

    /// Render the layer linked from `layer_idx` via `lines_from` into the *cell
    /// space* of that layer's own cell, so flood fill can treat its strokes as
    /// walls. `None` when no link is set or the link can't be resolved on this
    /// frame — the caller then falls back to plain same-layer filling.
    ///
    /// The linked layer bounds the fill even when hidden or set as a reference
    /// layer: the link is an explicit choice, visibility is a display concern.
    fn fill_boundary(&self, layer_idx: usize, cell_w: u32, cell_h: u32) -> Option<Canvas> {
        let p = &self.project;
        let dst_layer = p.layers.get(layer_idx)?;
        let src_idx = dst_layer.lines_from?;
        if src_idx == layer_idx {
            return None;
        }
        let src_layer = p.layers.get(src_idx)?;
        let src = p.cell(src_layer.resolve(p.current_frame)?)?;

        let frame = p.current_frame;
        let src_xform = src_layer.resolve_transform(frame);
        let dst_xform = dst_layer.resolve_transform(frame);

        // Both layers sit unmoved on a doc-sized cell — the common case.
        if src_xform.is_identity()
            && dst_xform.is_identity()
            && src.width == cell_w
            && src.height == cell_h
        {
            return Some(src.clone());
        }

        // Otherwise bake the source layer into document space. Opacity is
        // forced to 1.0: a line-art layer dialled down to 30% must still be a
        // solid wall, or the fill leaks straight through it.
        let (pw, ph) = (p.width, p.height);
        let mut doc = Canvas::new(pw, ph);
        crate::io::composite::composite_layer(&mut doc, src, &src_xform, 1.0, pw, ph);
        Some(doc_to_cell(doc, &dst_xform, cell_w, cell_h, pw, ph))
    }

    /// Remember the layer we just came from, so `Action::LayerLast` can jump
    /// back. Run once per frame: `current_layer` is assigned from eight
    /// different places (panel clicks, x-sheet clicks, add/delete/move, undo,
    /// import), and diffing here catches all of them without touching any.
    ///
    /// Tracked by uid, so a new layer still jumps back to the one it was
    /// added over. A layer that was deleted out from under the selection
    /// isn't remembered: the one before it stays the way back.
    fn track_layer_change(&mut self) {
        let now = (self.project.current_layer, self.project.layers.len());
        let uid = self.project.layers.get(now.0).map(|l| l.uid);
        if uid != self.layer_watch_uid {
            if let Some(old) = self.layer_watch_uid.filter(|&u| self.project.layers.iter().any(|l| l.uid == u)) {
                self.prev_layer = Some(old);
            }
            self.layer_watch_uid = uid;
        }
        // Moving the active layer anywhere the panel's picks don't include
        // ends the multi-selection.
        if now != self.layer_watch && uid.is_some_and(|u| !self.selected_layers.contains(&u)) {
            self.selected_layers.clear();
        }
        self.layer_watch = now;
    }

    /// Jump to the layer selected before the current one. No-op when there
    /// isn't one yet or it has since been deleted.
    pub fn goto_last_layer(&mut self) {
        let Some(prev) = self.prev_layer else { return };
        if let Some(i) = self.project.layers.iter().position(|l| l.uid == prev) {
            self.project.current_layer = i;
        }
    }

    /// Make layer `i` the active one. Out of range is a no-op, which is what
    /// stops the step keys at either end of the stack.
    fn select_layer(&mut self, i: usize) {
        if i < self.project.layers.len() {
            self.project.current_layer = i;
        }
    }

    /// Move the active layer one step, `up` meaning towards the top of the
    /// Layers panel: past its neighbour, or into or out of a group it meets
    /// (see [`Project::move_item`]); a group moves whole. One undo step;
    /// nothing at all at the end of the stack, so a held key leaves no empty
    /// undo entries behind.
    pub fn move_active_layer(&mut self, up: bool) {
        let i = self.project.current_layer;
        if !self.project.clone().move_item(i, up) {
            return;
        }
        self.structural_edit(false, |p| {
            p.move_item(i, up);
        });
        // Both edit buffers are keyed by layer index.
        self.layer_rename = None;
        self.expand_cfg = None;
    }

    /// A new layer over `of` to redraw it clean, its key jumps linked to it
    /// (see [`Project::add_cleanup_layer`]). One undo step.
    pub fn add_cleanup_layer(&mut self, of: usize) {
        if self.project.layers.get(of).map_or(true, |l| l.is_group()) {
            return;
        }
        self.structural_edit(false, |p| {
            p.add_cleanup_layer(of);
        });
        // Both edit buffers are keyed by layer index, and the insert shifted
        // every index above it.
        self.layer_rename = None;
        self.expand_cfg = None;
    }

    /// Select the row `delta` rows down the Layers panel from the active
    /// layer's (negative: up), stopping at either end. Folded groups' layers
    /// aren't rows.
    fn step_layer_row(&mut self, delta: isize) {
        let rows = self.project.panel_rows();
        let at = self.project.panel_row_of(self.project.current_layer);
        let Some(pos) = rows.iter().position(|&r| r == at) else {
            return;
        };
        let to = pos as isize + delta;
        if (0..rows.len() as isize).contains(&to) {
            self.select_layer(rows[to as usize]);
        }
    }

    /// Every selected layer, bottom first: the active one, and any picked
    /// alongside it in the panel.
    pub fn selected_layer_indices(&self) -> Vec<usize> {
        let cur = self.project.current_layer;
        (0..self.project.layers.len())
            .filter(|&i| i == cur || self.selected_layers.contains(&self.project.layers[i].uid))
            .collect()
    }

    /// The selected layers that hold drawings: a picked group stands for
    /// its layers. Bottom first, each once.
    fn selected_drawing_layers(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for i in self.selected_layer_indices() {
            if self.project.is_group(i) {
                out.extend(self.project.children(i));
            } else {
                out.push(i);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Flip or turn every drawing of every selected layer, a picked group's
    /// layers with them. One undo step; locked and reference layers are
    /// left alone.
    pub fn orient_selected_layers(&mut self, o: Orient) {
        let layers = self.selected_drawing_layers();
        self.orient_layers(&layers, o);
    }

    /// Set every selected layer's opacity, a group's own included. The first
    /// change since the last [`AppState::commit_layer_opacity`] opens the
    /// undo step, so a whole drag lands as one.
    pub fn set_selected_opacity(&mut self, v: f32) {
        if self.layer_opacity_before.is_none() {
            self.layer_opacity_before = Some(undo::TimelineState::capture(&self.project));
        }
        for i in self.selected_layer_indices() {
            self.project.layers[i].opacity = v.clamp(0.0, 1.0);
        }
    }

    /// Close the undo step an opacity drag opened. Nothing when it changed
    /// nothing.
    pub fn commit_layer_opacity(&mut self) {
        let Some(before) = self.layer_opacity_before.take() else { return };
        let changed = before.layers.len() != self.project.layers.len()
            || before.layers.iter().zip(&self.project.layers).any(|(a, b)| a.opacity != b.opacity);
        if changed {
            let after = undo::TimelineState::capture(&self.project);
            self.history.push(undo::Command::Structural {
                before,
                after,
                cell_pixels: Vec::new(),
            });
        }
    }

    /// Clip to the layer below on the selected layers, or off again: all go
    /// the opposite way to the active one, so a mixed pick ends up alike.
    /// Groups don't clip and are skipped.
    pub fn toggle_selected_clip(&mut self) {
        self.toggle_selected_flag(|l| &mut l.clip);
    }

    /// Alpha lock on the selected layers, the same way as
    /// [`AppState::toggle_selected_clip`].
    pub fn toggle_selected_alpha_lock(&mut self) {
        self.toggle_selected_flag(|l| &mut l.alpha_lock);
    }

    fn toggle_selected_flag(&mut self, flag: impl Fn(&mut crate::doc::layer::Layer) -> &mut bool) {
        let cur = self.project.current_layer;
        let sel: Vec<usize> =
            self.selected_layer_indices().into_iter().filter(|&i| !self.project.is_group(i)).collect();
        // A group active says nothing; the first picked layer leads then.
        let Some(lead) = sel.iter().copied().find(|&i| i == cur).or(sel.first().copied()) else {
            return;
        };
        let on = !*flag(&mut self.project.layers[lead]);
        for i in sel {
            *flag(&mut self.project.layers[i]) = on;
        }
    }

    /// Put the selected layers in a new group. One undo step.
    pub fn group_selected_layers(&mut self) {
        let sel = self.selected_layer_indices();
        if sel.is_empty() {
            return;
        }
        self.structural_edit(false, |p| {
            p.group_layers(&sel);
        });
        self.selected_layers.clear();
        self.layer_rename = None;
        self.expand_cfg = None;
    }

    /// Dissolve the active layer's group (or the active group), its layers
    /// staying where they are. One undo step.
    pub fn ungroup_active(&mut self) {
        let cur = self.project.current_layer;
        let Some(g) = (if self.project.is_group(cur) { Some(cur) } else { self.project.group_of(cur) }) else {
            return;
        };
        self.structural_edit(false, |p| p.ungroup(g));
        self.layer_rename = None;
        self.expand_cfg = None;
    }

    /// Delete the selected layers — a group with its layers. One undo step;
    /// never the last layer.
    pub fn delete_selected_layers(&mut self) {
        let uids: Vec<u64> = self
            .selected_layer_indices()
            .into_iter()
            .map(|i| self.project.layers[i].uid)
            .collect();
        self.structural_edit(false, |p| {
            for uid in uids {
                if let Some(i) = p.layers.iter().position(|l| l.uid == uid) {
                    p.delete_block(i);
                }
            }
        });
        self.selected_layers.clear();
        self.layer_rename = None;
        self.expand_cfg = None;
    }

    /// Name of the layer bounding the active layer's fills, for UI hints.
    pub fn fill_boundary_name(&self) -> Option<&str> {
        let src = self
            .project
            .layers
            .get(self.project.current_layer)?
            .lines_from?;
        Some(self.project.layers.get(src)?.name.as_str())
    }

    pub fn dispatch(&mut self, action: Action) {
        // A group's header has no drawings and no pixels: what edits the
        // active slot leaves it alone.
        let slot_edit = matches!(
            action,
            Action::KeyBlank
                | Action::KeyCopy
                | Action::Hold
                | Action::ClearCell
                | Action::CellCut
                | Action::CellPaste
                | Action::SelectionPaste
                | Action::SelectionCut
                | Action::SelectionDelete
                | Action::SelectionFill
                | Action::LayerClipToggle
                | Action::LayerAlphaLockToggle
        );
        if slot_edit && self.project.is_group(self.project.current_layer) {
            return;
        }
        match action {
            Action::ToolPencil => self.set_tool(ActiveTool::Pencil),
            Action::ToolInk => self.set_tool(ActiveTool::Ink),
            Action::ToolEraser => self.set_tool(ActiveTool::Eraser),
            // Again while Fill is the tool, it swaps the bucket and the lasso.
            Action::ToolFill if self.tool == ActiveTool::Fill => self.toggle_fill_mode(),
            Action::ToolFill => self.set_tool(ActiveTool::Fill),
            // Retired: the in-canvas eyedropper was folded into PickScreenColor,
            // which samples the canvas as well as everything behind the window.
            // The variant survives only so an old shortcuts.toml still parses.
            Action::ToolColorPicker | Action::ToggleCheckerBg => {}
            Action::PickScreenColor => {
                if self.screen_pick {
                    self.end_screen_pick();
                } else {
                    self.begin_screen_pick();
                }
            }
            Action::ToolShape => self.set_tool(ActiveTool::Shape),
            Action::ToolTracker => self.set_tool(ActiveTool::Tracker),
            Action::ToolLasso => self.set_tool(ActiveTool::Lasso),
            Action::ToolPerspective => self.set_tool(ActiveTool::Perspective),
            Action::ToolLiquify => self.set_tool(ActiveTool::Liquify),
            Action::TogglePerspectiveGrid => {
                self.perspective.show = !self.perspective.show;
            }
            Action::TogglePerspectiveSnap => {
                self.perspective.snap = !self.perspective.snap;
            }
            Action::PerspectiveNextGrid => self.cycle_perspective_grid(),
            Action::PlayPause => self.play_pause(None),
            Action::PlayPauseHere => self.play_pause_here(None),
            Action::FramePrev => self
                .project
                .step(-self.frame_step_delta(), self.loop_timeline),
            Action::FrameNext => self
                .project
                .step(self.frame_step_delta(), self.loop_timeline),
            // Unlike the plain step above these clamp: no key ahead means stay
            // put, so holding the key never wraps back to the top of the scene.
            Action::KeyJumpPrev => {
                if let Some(f) = self.project.prev_key_frame() {
                    self.project.goto(f);
                }
            }
            Action::KeyJumpNext => {
                if let Some(f) = self.project.next_key_frame() {
                    self.project.goto(f);
                }
            }
            // Insert the user's step size worth of frames. One `structural_edit`
            // wraps the whole loop, so N frames cost one undo entry, not N.
            Action::FrameAdd => {
                let n = self.frame_step_count();
                self.structural_edit(false, |p| {
                    for _ in 0..n {
                        p.add_frame();
                    }
                });
            }
            Action::FrameDuplicate => {
                let n = self.frame_step_count();
                self.structural_edit(false, |p| {
                    for _ in 0..n {
                        p.duplicate_frame();
                    }
                });
            }
            Action::FrameDelete => {
                let wipes_pixels = self.project.frame_count <= 1;
                self.structural_edit(wipes_pixels, Project::delete_frame);
            }
            Action::OnionToggle => self.onion.enabled = !self.onion.enabled,
            Action::LayerAdd => self.structural_edit(false, |p| {
                p.add_layer_above_active();
            }),
            Action::LayerDelete => self.delete_selected_layers(),
            Action::LayerGroup => self.group_selected_layers(),
            Action::LayerUngroup => self.ungroup_active(),
            Action::LayerToggleVisible => {
                if let Some(l) = self.project.layers.get_mut(self.project.current_layer) {
                    l.visible = !l.visible;
                }
            }
            Action::LayerLast => self.goto_last_layer(),
            // Panel order: the top row is the last layer.
            Action::LayerSelectAbove => self.step_layer_row(-1),
            Action::LayerSelectBelow => self.step_layer_row(1),
            Action::LayerSelectTop => {
                if let Some(&top) = self.project.panel_rows().first() {
                    self.select_layer(top);
                }
            }
            Action::LayerSelectBottom => {
                if let Some(&bottom) = self.project.panel_rows().last() {
                    self.select_layer(bottom);
                }
            }
            Action::LayerMoveUp => self.move_active_layer(true),
            Action::LayerMoveDown => self.move_active_layer(false),
            Action::LayerPick1
            | Action::LayerPick2
            | Action::LayerPick3
            | Action::LayerPick4
            | Action::LayerPick5
            | Action::LayerPick6
            | Action::LayerPick7
            | Action::LayerPick8
            | Action::LayerPick9 => {
                let n = action.layer_pick().unwrap_or(1);
                if let Some(&i) = self.project.panel_rows().get(n - 1) {
                    self.select_layer(i);
                }
            }
            Action::FadeOthersToggle => self.fade_others = !self.fade_others,
            Action::WheelPinToggle => self.set_wheel_pinned(!self.wheel_pinned),
            Action::LayerClipToggle => self.toggle_selected_clip(),
            Action::LayerAlphaLockToggle => self.toggle_selected_alpha_lock(),
            Action::LayerKeyLinkBelow => {
                self.project.toggle_key_link_below(self.project.current_layer);
            }
            Action::KeyBlank => {
                self.structural_edit(false, |p| {
                    p.insert_blank_key_here();
                });
            }
            Action::KeyCopy => {
                self.structural_edit(false, |p| {
                    p.insert_duplicate_key_here();
                });
            }
            Action::Hold => self.structural_edit(false, Project::hold_here),
            Action::SizeDown => {
                self.brush.radius = (self.brush.radius * 0.85).max(0.5);
            }
            Action::SizeUp => {
                self.brush.radius = (self.brush.radius * 1.18).min(256.0);
            }
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::ClearCell => self.clear_active(),
            Action::PasteImage => self.paste_image_as_background(),
            Action::TogglePanels => self.show_panels = !self.show_panels,
            Action::ToggleMiniTimeline => self.show_mini_timeline = !self.show_mini_timeline,
            Action::ZoomReset => self.view.zoom = 1.0,
            Action::PanReset => self.view.pan = egui::Vec2::ZERO,
            Action::RotateReset => {
                self.view.rotation = 0.0;
                // Un-mirror too: rotation and flip are the two ways the view
                // stops matching the document, and one reset for both means a
                // forgotten flip can't survive a "straighten up".
                self.view.flip_x = false;
                self.view.flip_y = false;
            }
            Action::FlipHorizontal => self.view.flip_x = !self.view.flip_x,
            Action::FlipVertical => self.view.flip_y = !self.view.flip_y,
            Action::ArtFlipH => self.orient_art(Orient::FLIP_H),
            Action::ArtFlipV => self.orient_art(Orient::FLIP_V),
            Action::ArtRotateCw => self.orient_art(Orient::CW),
            Action::ArtRotateCcw => self.orient_art(Orient::CCW),
            Action::ArtRotate180 => self.orient_art(Orient::HALF),
            Action::CellCopy => self.cell_clip = self.project.copy_active_cell(),
            Action::CellCut => self.cut_cell(),
            Action::CellPaste => self.paste_cell(),
            Action::SelectionCopy => self.copy_selection(),
            Action::SelectionCut => {
                self.copy_selection();
                self.delete_selection();
            }
            Action::SelectionPaste => self.paste_selection(),
            Action::SelectionDelete => self.delete_selection(),
            Action::TrackClear => self.clear_track_selection(),
            Action::TrackCloseGap => self.close_track_selection(),
            Action::SelectionDeselect => self.deselect(),
            Action::SelectAll => self.select_all(),
            Action::SelectInvert => self.select_invert(),
            Action::SelectionFill => self.fill_selection(),
            Action::SaveProject => self.save_project(),
            Action::SaveProjectAs => self.save_project_as(),
            Action::OpenProject => match crate::io::project_file::load_dialog() {
                Ok(Some((p, path))) => self.load_project(p, Some(path)),
                Ok(None) => {}
                Err(e) => log::error!("Open project failed: {e:#}"),
            },
            // Held, not pressed: read by the canvas input code as a drag
            // starts, never dispatched.
            Action::CanvasZoom
            | Action::CanvasPan
            | Action::CanvasRotate
            | Action::HoldToolAdjust
            | Action::HoldScrub
            | Action::HoldFlip
            | Action::HoldSwatchPie
            | Action::SelModeAdd
            | Action::SelModeSubtract
            | Action::SelModeIntersect
            | Action::FillLassoErase
            | Action::FillLassoEnclose
            | Action::FillPolygonSnap => {}
            Action::LayerTransformToggle => self.layer_xform = !self.layer_xform,
            Action::TransformKeyAdd => self.add_transform_key(),
            Action::TransformKeyDelete => self.delete_transform_key(),
            Action::CameraEditToggle => {
                self.camera_edit = !self.camera_edit;
                // The two retarget modes claim the same drag gestures, so only
                // one can be live at a time.
                if self.camera_edit {
                    self.layer_xform = false;
                    self.camera_box = false;
                }
            }
            Action::CameraBoxToggle => self.set_camera_box(!self.camera_box),
            Action::CameraKeyAdd => self.add_camera_key(),
            Action::CameraKeyDelete => self.delete_camera_key(),
            Action::CameraLookThrough => self.camera_look_through = !self.camera_look_through,
            Action::CameraKeyPrev => {
                if let Some(f) = self.project.prev_camera_key() {
                    self.project.goto(f);
                }
            }
            Action::CameraKeyNext => {
                if let Some(f) = self.project.next_camera_key() {
                    self.project.goto(f);
                }
            }
        }
    }

    /// How long a "saved" toast stays up.
    const TOAST_TTL: Duration = Duration::from_millis(2200);

    /// Save shortcut: overwrite the remembered file, or ask when there isn't
    /// one yet.
    pub fn save_project(&mut self) {
        match self.project_path.clone() {
            Some(path) => self.write_project(path),
            None => self.save_project_as(),
        }
    }

    /// Save-As shortcut: always prompt, seeded from the current path.
    pub fn save_project_as(&mut self) {
        if let Some(path) = crate::io::project_file::ask_save_path(self.project_path.as_deref()) {
            self.write_project(path);
        }
    }

    /// Write to `path`; remember it and raise a toast on success, or surface the
    /// failure as a modal. `project_path` is left alone on failure so a retry
    /// still targets the same file.
    fn write_project(&mut self, path: PathBuf) {
        // One writer at a time. Two threads racing the same path could
        // interleave into a corrupt file, and the second save would be of a
        // project the user has barely changed since the first.
        if self.save_job.is_some() {
            return;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string();
        // Snapshot so drawing can continue while the write runs. The same
        // clone-and-move `start_export` already does, and the same cost.
        let project = self.project.clone();
        let dest = path.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(crate::io::project_file::save_to(&project, &dest));
        });
        self.save_job = Some(SaveJob { rx, path, name });
    }

    /// Collect a finished background write. `project_path` moves only on
    /// success, so a failed save leaves a retry pointing at the same file.
    fn poll_save_job(&mut self) {
        let Some(job) = self.save_job.take() else {
            return;
        };
        match job.rx.try_recv() {
            Err(TryRecvError::Empty) => self.save_job = Some(job),
            Ok(Ok(())) => {
                self.grids_saved_as(&job.path);
                self.project_path = Some(job.path);
                self.save_toast = Some((format!("Saved  {}", job.name), Instant::now() + Self::TOAST_TTL));
                self.save_error = None;
            }
            Ok(Err(e)) => {
                log::error!("Save project failed: {e:#}");
                self.save_error = Some(format!("{e:#}"));
            }
            // The worker died without reporting. Treat it as a failure rather
            // than as a save that quietly never happened.
            Err(TryRecvError::Disconnected) => {
                log::error!("Save project worker vanished");
                self.save_error = Some("the save worker stopped unexpectedly".into());
            }
        }
    }

    /// Block until any in-flight write finishes.
    ///
    /// Called wherever the project is about to be replaced or the process is
    /// about to end — a backgrounded save cut short by either would lose work
    /// with no warning at all.
    fn finish_pending_save(&mut self) {
        let Some(job) = self.save_job.take() else {
            return;
        };
        log::info!("Waiting for the in-flight save to finish");
        match job.rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => log::error!("Save project failed: {e:#}"),
            Err(e) => log::error!("Save project worker vanished: {e}"),
        }
    }

    /// Replace the current project with a loaded one, resetting derived editing
    /// state (textures, history, in-flight strokes, view) but keeping user
    /// preferences (tools, brushes, shortcuts, pen).
    ///
    /// `path` is where it came from — taking it as a parameter rather than an
    /// assignment at the call site means a caller can't forget to update it and
    /// leave Save pointing at the previous file.
    pub fn load_project(&mut self, project: Project, path: Option<PathBuf>) {
        // Same reason as `reset_with`: a write still in flight would report
        // back after the swap and point the freshly opened project at the file
        // the previous one was being saved to.
        self.finish_pending_save();
        self.stop_krita_link();
        self.stash_grids();
        self.project = project;
        // Nothing holds an id into it yet, so this is the one moment the
        // cells nothing shows can go for good, renumbering the rest.
        self.project.compact_cells();
        self.unstash_grids(path.as_deref());
        self.project_path = path;
        self.save_toast = None;
        self.save_error = None;
        self.retire_cell_textures();
        self.retire_all_ghosts();
        self.cell_dirty.clear();
        self.clear_track_state();
        for id in 0..self.project.cells.len() {
            self.cell_dirty.insert(id, Dirty::Full);
        }
        self.stroke = None;
        self.stroke_target = None;
        self.shape_drag = None;
        self.fill_drag = None;
        self.clear_selection_state();
        self.stroke_pre_live = false;
        self.preview_upload_rect = None;
        self.history = History::default();
        self.gc_key = None;
        self.view = View::default();
        // Republished by the canvas next frame; kept in step with `view` so a
        // reset never leaves a stale scale behind for one frame of input.
        self.view_scale = 1.0;
        self.nav_drag = None;
        self.playback = Playback::default();
        // The loaded project brings its own camera and layer poses; drop the
        // caches that decide when to reload the live edit buffers.
        self.xform_sync_last = None;
        self.cam_sync_last = None;
        self.nav_to_camera = false;
        self.camera_drag_before = None;
        self.cam_drag = None;
        self.cam_box = None;
        self.cam_key_drag = None;
        self.shot_cam = None;
    }

    pub fn undo(&mut self) {
        self.track_sel.clear();
        // Back out of whatever is in hand before touching the history: a
        // polygon loses its newest corner, a warp its last stroke, and
        // floating pixels go back where they were picked up — nothing about
        // any of them is recorded yet.
        if self.polygon_back() || self.fill_lasso_back() || self.liquify_step_back() {
            return;
        }
        self.cancel_gesture();
        if self.cancel_float() {
            return;
        }
        let touched = self.history.undo(&mut self.project);
        self.apply_touched(touched);
    }

    pub fn redo(&mut self) {
        self.track_sel.clear();
        self.cancel_gesture();
        // Landing a float or a warp is a new edit, and like any other it ends
        // the redo chain; there is nothing left to redo after it.
        if self.selection.is_some() || self.liquify.is_some() {
            self.land_float();
            return;
        }
        let touched = self.history.redo(&mut self.project);
        self.apply_touched(touched);
    }

    /// Load edits made to shortcuts.toml outside the app (Settings' Reload
    /// button). A file that doesn't read is left alone, and so are the
    /// bindings in use — loading it would mean defaults, which the next
    /// rebind here would then write over the user's file.
    pub(crate) fn reload_shortcuts_file(&mut self) {
        self.shortcuts_file_note = Some(match shortcuts::read_file() {
            Some(Ok(map)) if map != self.shortcuts => {
                self.shortcuts = map;
                self.rebinding = None;
                Ok("loaded your edits to shortcuts.toml".into())
            }
            Some(Ok(_)) => Ok("shortcuts.toml matches what's in use: nothing to load".into()),
            Some(Err(e)) => Err(format!("shortcuts.toml didn't load, nothing changed: {e}")),
            None => Err("there's no shortcuts.toml yet: Open shortcuts file writes one".into()),
        });
    }

    /// Rebind capture and shortcut dispatch for this frame's key presses.
    ///
    /// Returns whether a Tab press found nothing focused. Tab is a shortcut
    /// like any other (the panel toggle by default), but egui also reads it
    /// as "focus the next widget" and hands focus to the first one drawn after
    /// it; `drop_stray_focus` takes that back once the frame is drawn.
    pub(crate) fn handle_keys(&mut self, ctx: &egui::Context) -> bool {
        let tab_focus = ctx.input(|i| i.key_pressed(egui::Key::Tab))
            && ctx.memory(|m| m.focused()).is_none();

        // A focused button takes Space as a click, auto-repeat included. With
        // Space held for a gesture that would press whatever button last had
        // focus, so a hold key lets go of it first. Text fields keep it.
        if let Some(id) = ctx.memory(|m| m.focused()) {
            let hold_keys = self.shortcuts.hold_keys();
            if ctx.input(|i| hold_keys.iter().any(|&k| i.key_pressed(k)))
                && egui::text_edit::TextEditState::load(ctx, id).is_none()
            {
                ctx.memory_mut(|m| m.surrender_focus(id));
            }
        }

        // Shortcut rebind capture: when an Action is "rebinding", the next
        // key press becomes its new combo and rebind mode ends. A hold
        // binding may also be bare modifiers, taken when they come back up.
        if let Some(action) = self.rebinding {
            let (mods, pressed) = ctx.input(|i| {
                let key = shortcuts::CAPTURE_KEYS
                    .iter()
                    .copied()
                    .find(|&k| i.key_pressed(k));
                (i.modifiers, key)
            });
            let captured = if action.is_hold() {
                self.hold_capture.step(mods, pressed)
            } else {
                // A modifier on its own is no press binding.
                pressed.map(|key| shortcuts::KeyCombo {
                    key: Some(key),
                    ctrl: mods.ctrl,
                    shift: mods.shift,
                    alt: mods.alt,
                })
            };
            if let Some(combo) = captured {
                self.shortcuts.set(action, combo);
                self.rebinding = None;
                shortcuts::save(&self.shortcuts);
            } else if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.rebinding = None;
            }
        } else {
            // Normal shortcut dispatch — skip when a text widget has keyboard
            // focus so the user can type into fields (e.g. DragValue in the
            // new-project dialog) without triggering shortcuts.
            // Whatever is bound to Tab or ` is always let through
            // (`FOCUS_PASS_KEYS`), so the panels — on Tab by default — can be
            // got back if focus is stuck.
            if ctx.memory(|m| m.focused()).is_none() {
                let actions = self.shortcuts.poll_actions(ctx);
                // Delete is both the selection's erase and the tracks'
                // delete. Decided once, before either runs: drawings selected
                // in the tracks claim it, and otherwise the selection does —
                // a selection can now stay up for as long as the user likes,
                // so it cannot be the one to win whenever it exists.
                let tracks_own_delete = !self.track_sel.is_empty();
                // Backspace takes back a polygon corner while one is being
                // placed, rather than clearing the drawing under it.
                let polygon = self.polygon_active() || self.fill_polygon_active();
                for a in actions {
                    match a {
                        Action::TrackClear if !tracks_own_delete => continue,
                        Action::SelectionDelete if tracks_own_delete => continue,
                        Action::ClearCell if polygon => {
                            let _ = self.polygon_back() || self.fill_lasso_back();
                            continue;
                        }
                        _ => {}
                    }
                    self.dispatch(a);
                }
                let (enter, escape) = ctx.input(|i| {
                    (
                        i.key_pressed(egui::Key::Enter),
                        i.key_pressed(egui::Key::Escape),
                    )
                });
                // Enter closes a polygon, or puts floating pixels down.
                if enter {
                    if self.polygon_active() {
                        self.polygon_finish();
                    } else if self.fill_polygon_active() {
                        self.fill_lasso_finish();
                    } else {
                        self.land_float();
                    }
                }
                // Esc backs out one step at a time: a Fill press still under
                // the pen, a middle- or right-button drag, a polygon in
                // progress, a drag in the tracks, the drawings selected
                // there, and last the canvas selection.
                if escape && !self.screen_pick {
                    if self.cancel_liquify()
                        || self.cancel_fill()
                        || self.cancel_button_drag()
                        || self.cancel_gesture()
                        || self.track_drag.take().is_some()
                    {
                    } else if !self.track_sel.is_empty() {
                        self.track_sel.clear();
                    } else {
                        self.deselect();
                    }
                }
                // Arrow keys nudge floating pixels by one. Not bound actions:
                // they only mean anything while something is selected, and
                // stealing the arrows outright would be worse. With only a
                // selection, the Lasso lifts it first — as a drag inside would.
                let (mut dx, mut dy) = (0, 0);
                ctx.input(|i| {
                    dx += i.key_pressed(egui::Key::ArrowRight) as i32;
                    dx -= i.key_pressed(egui::Key::ArrowLeft) as i32;
                    dy += i.key_pressed(egui::Key::ArrowDown) as i32;
                    dy -= i.key_pressed(egui::Key::ArrowUp) as i32;
                });
                if (dx != 0 || dy != 0)
                    && (self.selection.is_some()
                        || (self.tool == ActiveTool::Lasso && self.make_float()))
                {
                    self.nudge_selection(dx, dy);
                }
            } else {
                for a in self.shortcuts.poll_focused(ctx) {
                    self.dispatch(a);
                }
            }
        }
        tab_focus
    }

    /// Clear keyboard focus nothing should be holding: any at all while the
    /// panels are hidden, and whatever a Tab press handed out (see
    /// `handle_keys`). A focused widget gates off every shortcut but those on
    /// Tab and `, so Tabbing the panels back would otherwise leave the first
    /// panel button focused and the shortcuts dead until the next click.
    pub(crate) fn drop_stray_focus(&self, ctx: &egui::Context, tab_focus: bool) {
        if !self.show_panels || tab_focus {
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
    }
}

impl eframe::App for AppState {
    /// Called by eframe on exit and every `auto_save_interval` (30s). Panel
    /// geometry and collapse state are saved separately via
    /// `persist_egui_memory`, which defaults to true.
    /// Last call before the process goes away. A save runs on a worker thread
    /// now, so closing the window mid-write would drop it silently — wait for
    /// it instead.
    fn on_exit(&mut self) {
        self.finish_pending_save();
    }

    /// Put back the Ctrl+C / X / V presses egui-winit turned into clipboard
    /// events, so the selection shortcuts can see them.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        crate::input::clipboard_keys::inject(raw_input);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Only a tool switch writes the active brush back to its slot, so a
        // change made since the last switch lives in `brush` alone until this
        // stashes it.
        self.stash_brush();
        let tool_brushes = self.tool_brushes.to_vec();
        eframe::set_value(
            storage,
            UI_PREFS_KEY,
            &UiPrefs {
                show_panels: self.show_panels,
                show_mini_timeline: self.show_mini_timeline,
                frame_step: self.frame_step,
                show_camera_guide: self.show_camera_guide,
                dim_outside_camera: self.dim_outside_camera,
                show_layer_bounds: self.show_layer_bounds,
                lock_brush_to_view: self.lock_brush_to_view,
                onion: self.onion,
                auto_key_transform: self.auto_key_transform,
                auto_key_draw: self.auto_key_draw,
                invert_timeline_scroll: self.invert_timeline_scroll,
                middle_drag: self.middle_drag,
                right_drag: self.right_drag,
                loop_timeline: self.loop_timeline,
                smoothing: self.smoothing,
                tool_brushes: Some(tool_brushes),
                palette: self.palette.clone(),
                track_frame_w: self.track_frame_w,
                perspective: PerspectiveConfig {
                    grids: self.grid_seed.clone(),
                    active: 0,
                    ..self.perspective.clone()
                },
                grid_files: self.grid_files_for_prefs(),
                fade: self.fade,
                krita_path: self.krita_path.clone(),
                project_presets: self.project_presets.clone(),
                sel_shape: self.sel_shape,
                wand: self.wand,
                fill_mode: self.fill_mode,
                fill_enclose: self.fill_enclose,
                fill_behind: self.fill_behind,
                sel_amount: self.sel_amount,
                tint_outside: self.tint_outside,
                bg_color: self.bg_color,
                bg_opacity: self.bg_opacity,
                harmony: self.harmony.scheme,
                last_camera_size: self.last_camera_size,
                hidden_panels: self.hidden_panels.clone(),
                panel_layout_gen: self.panel_layout_gen,
                color_scheme: self.color_scheme,
                custom_colors: self.custom_colors,
            },
        );
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let a = self.bg_opacity.clamp(0.0, 1.0);
        [
            self.bg_color[0] * a,
            self.bg_color[1] * a,
            self.bg_color[2] * a,
            a,
        ]
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Pick up any layer selection made last frame, whatever moved it.
        self.track_layer_change();
        // Krita presets and thumbnails arrive from the library's loader.
        if self.brush_library.poll() || self.brush_library.loading() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        // A stabilized line keeps catching up while the pen is held still.
        if self.tick_stroke() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        self.blank_scans_left = Self::BLANK_SCANS_PER_FRAME;

        // Resolve any deferred preview-texture free now, before drawing, so the
        // freed textures are never referenced by this frame's paint list.
        if self.preview_clear_pending {
            self.preview_tex.clear();
            self.preview_clear_pending = false;
        }

        // First frame: maximize. Done here rather than at build time because the
        // creation-time flag doesn't survive on a frameless window (see the
        // `startup_maximize` field). Also keeps us on the monitor work area, so
        // an undecorated window can't end up covering the taskbar.
        //
        // Deliberately *after* the first frame lays panels out, not before: the
        // panel `default_pos` values are absolute and tuned for the un-maximized
        // size, so they're only correct on that first small frame. Maximizing
        // then produces a resize, and the edge re-stick in `ui::shell::draw`
        // carries the panels out to the new edges. Maximize before frame one and
        // there is no resize to react to, leaving them stranded mid-screen.
        if self.startup_maximize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            self.startup_maximize = false;
        }

        // First frame: re-apply the OS rounded corners + border to our frameless
        // window (Windows only). Done once, after the window handle exists.
        #[cfg(target_os = "windows")]
        if !self.window_styled {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = frame.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    crate::platform::round_window(w.hwnd.get());
                }
            }
            self.window_styled = true;
        }
        #[cfg(not(target_os = "windows"))]
        let _ = (&frame, self.window_styled);
        // The pen binds to this window's handle rather than to whichever
        // window is titled "Animator".
        #[cfg(target_os = "windows")]
        if !self.pen.has_hwnd() {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = frame.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    self.pen.set_hwnd(w.hwnd.get());
                }
            }
        }

        self.pen.poll(ctx.input(|i| i.pointer.any_down()));
        if let Some(rec) = &mut self.pen_record {
            if rec.frame(ctx, &self.pen) {
                let rec = self.pen_record.take().expect("recording");
                self.pen_record_saved = Some(rec.finish().map_err(|e| e.to_string()));
            }
            // Every frame, pen or no pen: a quiet spell is part of the record.
            ctx.request_repaint();
        }
        // Sampled here rather than in the pointer handlers: those run off
        // tablet packets and never see an `InputState`.
        (self.shift_held, self.alt_held) = ctx.input(|i| (i.modifiers.shift, i.modifiers.alt));

        let tab_focus = self.handle_keys(ctx);

        let now = ctx.input(|i| i.time);
        if self.advance_playback(now) {
            ctx.request_repaint();
        }
        if self.playback.playing {
            ctx.request_repaint_after(std::time::Duration::from_secs_f32(
                1.0 / self.project.fps.max(1.0),
            ));
        }

        // Keep the active layer's transform edit buffer synced while scrubbing.
        self.sync_active_transform_buffer();
        self.sync_selection();
        self.sync_liquify();
        self.sync_camera_buffer();
        // However the live camera was just moved — a drag, the panel's
        // fields, a new frame size.
        self.hold_camera_inside();
        // After the camera and layer buffers, which grids may follow.
        self.sync_grid_buffers();

        // Advance background import jobs / preview fetches without blocking.
        self.poll_bg_jobs();
        self.poll_save_job();
        self.poll_krita(ctx);
        self.poll_preview(ctx);
        // A save reports back on a channel, not an input event, so without this
        // the toast would wait for the next mouse move to appear.
        if self.bg_job.is_some() || self.preview_rx.is_some() || self.save_job.is_some() {
            ctx.request_repaint();
        }

        self.collect_cells();
        self.pack_idle_cells();
        self.warm_pre(ctx);
        self.sync_textures(ctx);
        ui::shell::draw(self, ctx);

        self.drop_stray_focus(ctx, tab_focus);
    }
}

/// Build a `ColorImage` by premultiplying straight-alpha RGBA8 in gamma
/// (sRGB) space. egui's `from_rgba_unmultiplied` premultiplies in linear
/// space, which over-brightens fractional-alpha AA edges once egui blends
/// them in gamma space — light strokes get a white rim. The same rim comes
/// back between texels if they are *mixed* in linear light, as egui's
/// sampler does, so the canvas draws them through `ui::canvas_gpu`.
pub(crate) fn premultiplied_image(size: [usize; 2], rgba: &[u8]) -> ColorImage {
    let pixels = rgba.chunks_exact(4).map(premultiplied).collect();
    ColorImage { size, pixels }
}

/// One straight-alpha RGBA8 pixel, premultiplied in gamma space. See
/// `premultiplied_image`.
fn premultiplied(p: &[u8]) -> Color32 {
    match p[3] {
        0 => Color32::TRANSPARENT,
        255 => Color32::from_rgb(p[0], p[1], p[2]),
        a => {
            let m = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
            Color32::from_rgba_premultiplied(m(p[0]), m(p[1]), m(p[2]), a)
        }
    }
}

/// Box-filter factor that brings a `w`×`h` cell within `GHOST_MAX_SIDE`.
fn ghost_step(w: u32, h: u32) -> usize {
    w.max(h).div_ceil(GHOST_MAX_SIDE).max(1) as usize
}

/// Texture size of the onion ghost for a `w`×`h` cell.
pub(crate) fn ghost_dims(w: u32, h: u32) -> [usize; 2] {
    let k = ghost_step(w, h);
    [(w as usize).div_ceil(k), (h as usize).div_ceil(k)]
}

/// Build a cell's onion ghost: a flat silhouette in `tint`, the source alpha
/// kept as the shape and the RGB replaced wholesale, box-filtered down to
/// `ghost_dims`.
///
/// This is what makes onion skins actually read as blue-past / red-future.
/// Painting the cell texture with a tinted vertex color only *multiplies*, and
/// black line art times any tint is still black. The canvas stretches it back
/// over the cell's corners, so its smaller size never shows as misplacement.
pub(crate) fn ghost_image(c: &Canvas, tint: [u8; 3]) -> ColorImage {
    let k = ghost_step(c.width, c.height);
    let [ow, oh] = ghost_dims(c.width, c.height);
    let (w, h) = (c.width as usize, c.height as usize);
    // Read where the cell holds anything: on a packed 8K cell, just the
    // drawing, never a whole buffer built to read.
    let mut pixels: Vec<Color32> = bytemuck::zeroed_vec(ow * oh);
    par_rows(&mut pixels, ow, |first, band| {
        // Alpha summed per ghost texel of the row.
        let mut sum = vec![0u32; ow];
        for (oy, dst) in (first..).zip(band.chunks_exact_mut(ow)) {
            sum.fill(0);
            for y in oy * k..((oy + 1) * k).min(h) {
                let (x0, mut rest) = c.row_span(y as u32);
                let mut x = x0 as usize;
                // One texel per k-pixel run; a run can start partway.
                while !rest.is_empty() {
                    let n = (k - x % k).min(rest.len() / 4);
                    let (run, tail) = rest.split_at(n * 4);
                    sum[x / k] += run.chunks_exact(4).map(|p| p[3] as u32).sum::<u32>();
                    (x, rest) = (x + n, tail);
                }
            }
            for (ox, (d, &s)) in dst.iter_mut().zip(&sum).enumerate() {
                // Texels on the right and bottom edges can cover fewer pixels.
                let n = ((w - ox * k).min(k) * (h - oy * k).min(k)) as u32;
                // Premultiplied in gamma space, matching `premultiplied_image`.
                *d = premultiplied(&[tint[0], tint[1], tint[2], ((s + n / 2) / n) as u8]);
            }
        }
    });
    ColorImage {
        size: [ow, oh],
        pixels,
    }
}

// Build a small (≤360px wide) preview `ColorImage` from RGBA pixels.
fn preview_color_image(w: u32, h: u32, pixels: &[u8]) -> ColorImage {
    let maxw = 360u32;
    if w <= maxw {
        return premultiplied_image([w as usize, h as usize], pixels);
    }
    match image::RgbaImage::from_raw(w, h, pixels.to_vec()) {
        Some(im) => {
            let nh = (h * maxw / w).max(1);
            let r = image::imageops::resize(&im, maxw, nh, image::imageops::FilterType::Triangle);
            premultiplied_image([maxw as usize, nh as usize], r.as_raw())
        }
        None => premultiplied_image([w as usize, h as usize], pixels),
    }
}

// --- Krita: the Edit-in-Krita link, and one-way .kra import / export --------

impl AppState {
    /// How long a Krita message stays up. Warnings name what was dropped, so
    /// they get long enough to read.
    const KRITA_NOTE: Duration = Duration::from_secs(4);
    const KRITA_WARN: Duration = Duration::from_secs(9);

    fn krita_notice(&mut self, msg: impl Into<String>, warn: bool) {
        let msg = msg.into();
        // Long enough to read: ~60 ms a character, within bounds.
        let floor = if warn { Self::KRITA_WARN } else { Self::KRITA_NOTE };
        let ttl = floor.max(Duration::from_millis(msg.len() as u64 * 60)).min(Duration::from_secs(15));
        self.krita_toast = Some((msg, warn, Instant::now() + ttl));
    }

    /// Krita's executable: the remembered or usual location, else ask. What
    /// was found is remembered.
    fn resolve_krita(&mut self) -> Option<PathBuf> {
        let found = krita_link::find_krita(self.krita_path.as_deref()).or_else(|| {
            let d = rfd::FileDialog::new().set_title("Where is Krita?");
            let d = if cfg!(windows) { d.add_filter("Krita (krita.exe)", &["exe"]) } else { d };
            d.pick_file()
        })?;
        self.krita_path = Some(found.clone());
        Some(found)
    }

    /// File ▸ Edit in Krita (`only` = `None`: the whole project, layer
    /// `selected` selected in Krita), and the layer menu's (`only` = that
    /// layer, sent alone; the rest stay here, unlinked, and a Krita save
    /// leaves them be). The first time, writes an animated `.kra` and opens
    /// it in Krita. With a link live it starts a re-send round (see
    /// [`krita_link::Round`]): Krita's latest save comes in first, then the
    /// file is rewritten — and with the helper plugin enabled in Krita, Krita
    /// reloads it in place. Without it, Krita (which neither watches files
    /// nor has a Reload command) needs the file closed and reopened there.
    pub fn edit_in_krita(&mut self, selected: usize, only: Option<usize>) {
        if self.krita_send.is_some() || self.krita.as_ref().is_some_and(|k| k.round.is_some()) {
            return;
        }
        let only = match only {
            Some(li) if li >= self.project.layers.len() => return,
            Some(li) if krita_link::stays_here_at(&self.project, li) => {
                let name = self.project.layers[li].name.clone();
                self.krita_notice(format!("\u{201c}{name}\u{201d} stays here (reference or \u{2026}-x): not sent."), true);
                return;
            }
            Some(li) => Some(self.project.layers[li].uid),
            None => None,
        };
        let running = self.krita.as_mut().is_some_and(KritaLink::krita_running);
        let launch = if running {
            None
        } else {
            match self.resolve_krita() {
                Some(exe) => Some(exe),
                None => return,
            }
        };
        self.begin_send(selected, only, launch);
    }

    /// [`Self::edit_in_krita`] once Krita's whereabouts are settled, `only`
    /// as a layer uid. Tests come in here, so no real Krita is ever started.
    fn begin_send(&mut self, selected: usize, only: Option<u64>, launch: Option<PathBuf>) {
        let Some(link) = self.krita.as_mut().filter(|k| k.baseline.is_some()) else {
            // First send: there's nothing in Krita to bring in or reload.
            self.start_send(selected, only, launch);
            return;
        };
        let helper = link.use_helper.unwrap_or_else(crate::krita_helper::installed);
        let mut round = krita_link::Round {
            seq: 0,
            stage: krita_link::Stage::Sync,
            selected,
            only,
            launch,
            helper_ready: false,
            helper_silent: false,
            note: String::new(),
        };
        if helper {
            link.helper_seq += 1;
            round.seq = link.helper_seq;
            match crate::krita_helper::tell(&link.path, round.seq, "request") {
                Ok(()) => {
                    round.stage = krita_link::Stage::AwaitReady { until: Instant::now() + link.ready_timeout };
                }
                Err(e) => log::warn!("Krita helper mailbox: {e:#}"),
            }
        }
        link.round = Some(round);
    }

    /// Write the project — or just layer `only` — as the linked `.kra` on a
    /// worker; `poll_krita` picks up the result.
    fn start_send(&mut self, selected: usize, only: Option<u64>, launch: Option<PathBuf>) {
        let scope = match only.map(|uid| self.project.layers.iter().position(|l| l.uid == uid)) {
            None => None,
            Some(Some(li)) if !krita_link::stays_here_at(&self.project, li) => Some(li),
            Some(found) => {
                // Krita's save, just brought in, deleted the layer — or it was
                // made to stay here since. Nothing to send.
                if let Some(link) = self.krita.as_mut() {
                    link.round = None;
                }
                let msg = match found {
                    Some(li) => format!(
                        "\u{201c}{}\u{201d} stays here (reference or \u{2026}-x): not sent.",
                        self.project.layers[li].name
                    ),
                    None => "That layer is gone from here: nothing sent to Krita.".to_string(),
                };
                self.krita_notice(msg, true);
                return;
            }
        };
        let selected = scope.unwrap_or(selected);
        let rows = scope.map(|li| self.project.block(li));
        let temp = KritaLink::temp_path(self.project_path.as_deref());
        let link = self.krita.get_or_insert_with(|| KritaLink::new(temp));
        // A read in flight was matched against the baseline this send
        // replaces; polling waits for the new one.
        link.pending = None;
        link.baseline = None;
        // Reference and `-x` layers stay here: not sent, and unlinked if they
        // were linked before. So do the layers a one-layer send leaves out —
        // a Krita save then leaves them be.
        let mut keep: Vec<bool> = krita_link::sendable(&self.project);
        let mut uuids = Vec::new();
        let mut kept_out = Vec::new();
        for (li, (l, k)) in self.project.layers.iter().zip(&mut keep).enumerate() {
            let in_scope = rows.as_ref().map_or(true, |r| r.contains(&li));
            if *k && in_scope {
                uuids.push(link.uuids.entry(l.uid).or_insert_with(krita_link::new_uuid).clone());
            } else {
                *k = false;
                link.uuids.remove(&l.uid);
                if in_scope {
                    kept_out.push(l.name.clone());
                }
            }
        }
        let selected = keep
            .get(selected)
            .copied()
            .unwrap_or(false)
            .then(|| keep[..selected].iter().filter(|&&k| k).count());
        let path = link.path.clone();
        let project = self.project.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(krita_link::send(project, &keep, &uuids, selected, &path));
        });
        self.krita_send = Some(KritaSend { rx, launch, kept_out });
    }

    /// File ▸ Install Krita helper: put the plugin in Krita's plugin folder.
    pub fn install_krita_helper(&mut self) {
        match crate::krita_helper::install() {
            Ok(dir) => self.krita_notice(
                format!(
                    "Krita helper installed in {}. Now in Krita: Settings ▸ Configure Krita ▸ \
                     Python Plugin Manager, tick \u{201c}Animator Link\u{201d}, then restart Krita.",
                    dir.display()
                ),
                false,
            ),
            Err(e) => {
                log::error!("Installing the Krita helper failed: {e:#}");
                self.krita_notice(format!("Couldn't install the Krita helper: {e:#}"), true);
            }
        }
    }

    /// End the link. The temp `.kra` stays on disk, so a save made in Krita
    /// afterwards can still come in through Import .kra.
    pub fn stop_krita_link(&mut self) {
        self.krita = None;
        self.krita_send = None;
    }

    /// Whether Edit in Krita would re-send rather than start a link.
    pub fn krita_linked(&self) -> bool {
        self.krita.is_some()
    }

    /// Per frame: land a finished send, move a re-send round along, start a
    /// read once Krita has saved, and fold a finished read into the project.
    fn poll_krita(&mut self, ctx: &egui::Context) {
        if let Some(send) = self.krita_send.take() {
            match send.rx.try_recv() {
                Err(TryRecvError::Empty) => self.krita_send = Some(send),
                Ok(Ok(sent)) => {
                    let Some(link) = self.krita.as_mut() else { return };
                    link.baseline = Some(sent.baseline);
                    link.mark_seen(sent.stamp);
                    let mut note = String::new();
                    if !send.kept_out.is_empty() {
                        note.push_str(&format!(" Kept out: {}.", send.kept_out.join(", ")));
                    }
                    if !sent.kept.is_empty() {
                        note.push_str(&format!(" Krita-only layers kept: {}.", sent.kept.join(", ")));
                    }
                    // A re-send the helper is waiting on: tell it the file is
                    // there, and let its answer make the toast.
                    let mut silent = false;
                    let mut resend = false;
                    if let Some(round) = link.round.as_mut().filter(|r| r.stage == krita_link::Stage::Sending) {
                        resend = true;
                        if round.helper_ready {
                            match crate::krita_helper::tell(&link.path, round.seq, "sent") {
                                Ok(()) => {
                                    round.stage =
                                        krita_link::Stage::AwaitReloaded { until: Instant::now() + link.reload_timeout };
                                    round.note = note;
                                    return;
                                }
                                Err(e) => log::warn!("Krita helper mailbox: {e:#}"),
                            }
                        }
                        silent = round.helper_silent;
                        link.round = None;
                    }
                    let (mut msg, warn) = match send.launch {
                        Some(exe) => match link.launch(&exe) {
                            Ok(()) => ("Opened in Krita. Save there to bring changes back.".to_string(), false),
                            Err(e) => {
                                log::error!("Starting Krita failed: {e:#}");
                                (format!("Couldn't start Krita: {e:#}"), true)
                            }
                        },
                        None => (
                            "Sent to Krita again. Krita doesn't reload open files: close it there \
                             without saving, then reopen it from File ▸ Open Recent."
                                .to_string(),
                            false,
                        ),
                    };
                    msg.push_str(&note);
                    if silent {
                        msg.push_str(
                            " The Krita helper didn't answer: tick \u{201c}Animator Link\u{201d} in Krita's \
                             Settings ▸ Configure Krita ▸ Python Plugin Manager and restart Krita.",
                        );
                    } else if resend && !crate::krita_helper::installed() {
                        msg.push_str(" File ▸ Install Krita helper makes this automatic.");
                    }
                    self.krita_notice(msg, warn);
                }
                Ok(Err(e)) => {
                    log::error!("Sending to Krita failed: {e:#}");
                    self.krita = None;
                    self.krita_notice(format!("Couldn't write the Krita file: {e:#}"), true);
                }
                Err(TryRecvError::Disconnected) => {
                    self.krita = None;
                    self.krita_notice("Sending to Krita stopped unexpectedly.", true);
                }
            }
        }

        let stroking = self.stroke.is_some();
        let Some(link) = self.krita.as_mut() else {
            return;
        };
        // A Krita save is a file change, not an input event: keep checking
        // while the window sits idle.
        ctx.request_repaint_after(Duration::from_millis(500));
        if let Some((stamp, rx)) = link.pending.take() {
            // Never restructure the timeline under a stroke in progress; the
            // result waits in the channel until the pen lifts.
            if stroking {
                link.pending = Some((stamp, rx));
                return;
            }
            match rx.try_recv() {
                Err(TryRecvError::Empty) => link.pending = Some((stamp, rx)),
                Ok(res) => {
                    link.mark_seen(Some(stamp));
                    self.apply_krita_pull(res);
                }
                Err(TryRecvError::Disconnected) => {
                    link.mark_seen(Some(stamp));
                    self.krita_notice("Reading Krita's save stopped unexpectedly.", true);
                }
            }
            return;
        }
        if link.round.is_some() {
            self.drive_round();
            return;
        }
        if let Some(stamp) = link.poll(Instant::now()) {
            link.start_pull(stamp, krita_link::present(&self.project, &link.uuids));
        }
    }

    /// One step of a re-send round (see [`krita_link::Round`]).
    fn drive_round(&mut self) {
        use crate::krita_helper::Reply;
        use krita_link::Stage;
        let Some(link) = self.krita.as_mut() else { return };
        let Some(mut round) = link.round.take() else { return };
        let now = Instant::now();
        let mut send = None;
        let mut done: Option<(String, bool)> = None;
        match round.stage {
            Stage::AwaitReady { until } => match crate::krita_helper::reply(&link.path) {
                Some(Reply::Ready { seq, .. }) if seq == round.seq => {
                    // Krita is open on the file and will reload it itself.
                    round.helper_ready = true;
                    round.launch = None;
                    round.stage = Stage::Sync;
                }
                Some(Reply::Error { seq, text }) if seq == round.seq => {
                    log::warn!("Krita helper: {text}");
                    round.helper_silent = true;
                    round.stage = Stage::Sync;
                }
                _ if now >= until => {
                    round.helper_silent = true;
                    round.stage = Stage::Sync;
                }
                _ => {}
            },
            Stage::Sync => {
                // Krita's latest save comes in before the file is rewritten,
                // or the rewrite would bury it. (With the helper, that's the
                // save it just made.)
                if let Some(at) = link.unseen_change() {
                    link.start_pull(at, krita_link::present(&self.project, &link.uuids));
                } else {
                    round.stage = Stage::Sending;
                    send = Some((round.selected, round.only, round.launch.take()));
                }
            }
            Stage::Sending => {}
            Stage::AwaitReloaded { until } => {
                done = match crate::krita_helper::reply(&link.path) {
                    Some(Reply::Reloaded { seq }) if seq == round.seq => {
                        Some((format!("Updated in Krita.{}", round.note), false))
                    }
                    Some(Reply::Error { seq, text }) if seq == round.seq => Some((
                        format!("Sent, but Krita couldn't reload it ({text}): close and reopen it there."),
                        true,
                    )),
                    _ if now >= until => Some((
                        "Sent, but Krita didn't confirm the reload. If it still shows the old \
                         version, close and reopen it there."
                            .to_string(),
                        true,
                    )),
                    _ => None,
                };
            }
        }
        if done.is_none() {
            link.round = Some(round);
        }
        if let Some((selected, only, launch)) = send {
            self.start_send(selected, only, launch);
        }
        if let Some((msg, warn)) = done {
            self.krita_notice(msg, warn);
        }
    }

    /// Fold a read Krita save into the project as one undo step.
    fn apply_krita_pull(&mut self, res: Result<Pulled>) {
        let Some(link) = self.krita.as_ref() else {
            return;
        };
        let Some(base) = link.baseline.as_ref() else {
            return;
        };
        let plan = match res.and_then(|pulled| krita_link::plan(&self.project, base, &link.uuids, pulled)) {
            Ok(p) => p,
            Err(e) => {
                log::error!("Krita pull failed: {e:#}");
                self.krita_notice(format!("Couldn't read Krita's save: {e:#}"), true);
                return;
            }
        };
        let warnings = plan.warnings.join("; ");
        let next = plan.next.clone();
        let unlink = plan.unlink.clone();
        let summary = (!plan.is_noop()).then(|| plan.report.summary());
        if summary.is_some() {
            self.playback.stop();
            let mut linked = Vec::new();
            self.structural_edit(false, |p| linked = plan.apply(p));
            if let Some(link) = self.krita.as_mut() {
                link.uuids.extend(linked);
            }
        }
        if let Some(link) = self.krita.as_mut() {
            link.baseline = Some(next);
            link.uuids.retain(|uid, _| !unlink.contains(uid));
        }
        match (summary, warnings.is_empty()) {
            (Some(s), true) => self.krita_notice(format!("Updated from Krita: {s}"), false),
            (Some(s), false) => self.krita_notice(format!("Updated from Krita: {s}. {warnings}"), true),
            (None, false) => self.krita_notice(format!("Krita save: {warnings}"), true),
            (None, true) => {}
        }
    }

    /// File ▸ Import .kra…: a Krita document's paint layers as new layers
    /// under the active one, 1:1 at the document's size.
    pub fn import_kra(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import Krita document")
            .add_filter("Krita document", &["kra"])
            .pick_file()
        else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let res = std::fs::read(&path)
                .with_context(|| format!("reading {}", path.display()))
                .and_then(|b| kra::read_filtered(b, &krita_link::comes_back));
            let _ = tx.send(res);
        });
        self.bg_job = Some(BgJob::KraImport(rx));
        self.bg_label = Some("Importing Krita document…");
    }

    fn on_kra_imported(&mut self, res: Result<KraDoc>) {
        let doc = match res {
            Ok(d) => d,
            Err(e) => {
                log::error!("Krita import failed: {e:#}");
                self.krita_notice(format!("Couldn't import: {e:#}"), true);
                return;
            }
        };
        let (kw, kh) = (doc.width, doc.height);
        let (pw, ph) = (self.project.width, self.project.height);
        let max_tex = self.max_tex;
        let mut need = doc.range_end.map_or(1, |e| e + 1);
        let mut clipped = 0usize;
        struct Imported {
            name: String,
            opacity: u8,
            visible: bool,
            locked: bool,
            cells: Vec<Canvas>,
            keys: Vec<(usize, usize)>,
            group: bool,
            in_group: bool,
            collapsed: bool,
        }
        let mut layers: Vec<Imported> = Vec::new();
        for kl in &doc.layers {
            // Krita's canvas *is* its frame, so each drawing becomes a cell of
            // the canvas size; the identity transform centers it on ours.
            let mut cells: Vec<Canvas> = Vec::new();
            let mut index: HashMap<usize, usize> = HashMap::new();
            let mut keys: Vec<(usize, usize)> = Vec::new();
            for &(t, di) in &kl.keys {
                let ci = match index.get(&di) {
                    Some(&ci) => ci,
                    None => {
                        let (c, clip) = kl.drawings[di].place(0, 0, kw, kh);
                        // Nothing to show before the first drawing: a blank
                        // lead-in key is just Krita's way of saying so.
                        if keys.is_empty() && c.is_all_zero() {
                            continue;
                        }
                        clipped += clip as usize;
                        cells.push(cap_canvas(c, max_tex));
                        index.insert(di, cells.len() - 1);
                        cells.len() - 1
                    }
                };
                keys.push((t, ci));
                need = need.max(t + 1);
            }
            layers.push(Imported {
                name: kl.name.clone(),
                opacity: kl.opacity,
                visible: kl.visible,
                locked: kl.locked,
                cells,
                keys,
                group: kl.group,
                in_group: kl.in_group,
                collapsed: kl.collapsed,
            });
        }
        // A group of nothing — every layer in it held back — isn't worth
        // bringing in.
        let mut i = 0;
        while i < layers.len() {
            if layers[i].group && (i == 0 || !layers[i - 1].in_group) {
                layers.remove(i);
            } else {
                i += 1;
            }
        }
        if layers.iter().all(|l| l.group) {
            layers.clear();
        }
        if layers.is_empty() {
            let why = if doc.held.is_empty() {
                "That Krita file has no paint layers to import."
            } else {
                "Every paint layer in that Krita file is Krita-only (named …-x); nothing imported."
            };
            self.krita_notice(why, true);
            return;
        }
        let n = layers.iter().filter(|l| !l.group).count();
        let has_groups = layers.iter().any(|l| l.group);
        self.structural_edit(false, move |p| {
            p.ensure_frame_count(need);
            // Bottom first: each lands on top of the one before, all under
            // the active layer — and in its group, unless what comes in has
            // groups of its own, which can't go inside one: then under the
            // whole group.
            let cur = p.current_layer;
            let at0 = if has_groups { p.block(p.group_of(cur).unwrap_or(cur)).start } else { cur };
            let inherit = !has_groups && p.layers.get(cur).is_some_and(|l| l.in_group || l.is_group());
            let mut at = at0.min(p.layers.len());
            for im in layers {
                let mut l = if im.group {
                    crate::doc::layer::Layer::group(im.name, p.frame_count)
                } else {
                    crate::doc::layer::Layer::new(im.name, p.frame_count)
                };
                l.opacity = im.opacity as f32 / 255.0;
                l.visible = im.visible;
                l.locked = im.locked;
                l.in_group = im.in_group || (inherit && !im.group);
                l.collapsed = im.collapsed;
                let base = p.cells.len();
                if let Some((cw, ch)) = im.cells.first().map(|c| (c.width, c.height)).filter(|&s| s != (pw, ph)) {
                    l.cell_w = cw;
                    l.cell_h = ch;
                }
                p.cells.extend(im.cells.into_iter().map(std::sync::Arc::new));
                for (t, ci) in im.keys {
                    l.set_key(t, base + ci);
                }
                p.layers.insert(at, l);
                p.relink_after_insert(at);
                at += 1;
            }
            if cur >= at0 {
                p.current_layer = cur + (at - at0);
            }
            p.normalize_groups();
        });
        let mut warnings = doc.warnings;
        if let Some(fps) = doc.fps.filter(|f| (f - self.project.fps).abs() > 0.01) {
            warnings.push(format!("the Krita file is {fps} fps; the project stays at {} fps", self.project.fps));
        }
        if clipped > 0 {
            warnings.push(format!("{clipped} drawing(s) had paint outside the Krita canvas (clipped)"));
        }
        let mut msg = format!("Imported {n} layer{} from Krita", if n == 1 { "" } else { "s" });
        if !doc.held.is_empty() {
            let h = doc.held.len();
            msg.push_str(&format!(" ({h} Krita-only -x layer{} skipped)", if h == 1 { "" } else { "s" }));
        }
        if warnings.is_empty() {
            self.krita_notice(msg, false);
        } else {
            self.krita_notice(format!("{msg}. {}", warnings.join("; ")), true);
        }
    }

    /// File ▸ Export .kra…: the whole project as an animated Krita document.
    /// One-way — this starts no link.
    pub fn export_kra(&mut self) {
        let stem = self
            .project_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or_else(|| "animation".to_string(), |s| s.to_string_lossy().into_owned());
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export Krita document")
            .add_filter("Krita document", &["kra"])
            .set_file_name(format!("{stem}.kra"))
            .save_file()
        else {
            return;
        };
        let name = path
            .file_name()
            .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let uuids: Vec<String> = self.project.layers.iter().map(|_| krita_link::new_uuid()).collect();
        let selected = self.project.current_layer;
        let project = self.project.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            // Krita has no clipping to match ours: clipped layers go cut.
            let mut project = project;
            crate::doc::clip::bake_clipping(&mut project);
            let res = kra::write(&project, &kra::WriteOpts { layer_uuids: &uuids, selected_layer: selected, carried: &[] })
                .and_then(|bytes| std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display())))
                .map(|()| name);
            let _ = tx.send(res);
        });
        self.bg_job = Some(BgJob::Export(rx));
        self.bg_label = Some("Exporting Krita document…");
    }
}

/// Downscale a canvas (aspect-preserving) so neither side exceeds `max`. No-op
/// if it already fits. Used to keep imported cells within the GPU texture limit.
fn cap_canvas(c: Canvas, max: u32) -> Canvas {
    if c.width <= max && c.height <= max {
        return c;
    }
    let s = (max as f32 / c.width as f32).min(max as f32 / c.height as f32);
    let nw = ((c.width as f32 * s).floor() as u32).max(1);
    let nh = ((c.height as f32 * s).floor() as u32).max(1);
    let Some(img) = image::RgbaImage::from_raw(c.width, c.height, c.into_pixels()) else {
        return Canvas::new(nw, nh);
    };
    let resized = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Lanczos3);
    Canvas::from_pixels(nw, nh, resized.into_raw())
}

/// A document-space canvas `pw`×`ph` brought into the pixels of a
/// `cell_w`×`cell_h` cell placed by `xform`. Handed straight back when the
/// cell sits unmoved on a frame-sized canvas — the common case.
fn doc_to_cell(
    doc: Canvas,
    xform: &Transform,
    cell_w: u32,
    cell_h: u32,
    pw: u32,
    ph: u32,
) -> Canvas {
    if xform.is_identity() && cell_w == pw && cell_h == ph {
        return doc;
    }
    // The cell is transformed, so walk its pixels and pull the matching
    // document-space sample for each.
    let mut out = Canvas::new(cell_w, cell_h);
    let (cw, ch) = (cell_w as f32, cell_h as f32);
    let (pwf, phf) = (pw as f32, ph as f32);
    for v in 0..cell_h {
        for u in 0..cell_w {
            let (dx, dy) = xform.cell_to_doc(u as f32 + 0.5, v as f32 + 0.5, cw, ch, pwf, phf);
            let (sx, sy) = (dx - 0.5, dy - 0.5);
            if sx < -0.5 || sy < -0.5 || sx > pwf - 0.5 || sy > phf - 0.5 {
                continue;
            }
            let px = crate::io::composite::sample_bilinear(&doc, sx, sy);
            let i = ((v * cell_w + u) * 4) as usize;
            out.pixels_mut()[i..i + 4].copy_from_slice(&px);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::canvas::no_alpha;

    // --- Clipped layers on the canvas ---

    /// Layer 0 inked opaque black on its left half; layer 1, clipped to it,
    /// painted solid red all over.
    fn clip_state() -> AppState {
        let mut st = AppState::for_test();
        let (w, h) = (st.project.width, st.project.height);
        let mut base = Canvas::new(w, h);
        let mut top = Canvas::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                if x < w / 2 {
                    base.pixels_mut()[i..i + 4].copy_from_slice(&[0, 0, 0, 255]);
                }
                top.pixels_mut()[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        st.project.add_layer();
        let n = st.project.cells.len();
        st.project.cells.push(base.into());
        st.project.cells.push(top.into());
        st.project.layers[0].set_key(0, n);
        st.project.layers[1].set_key(0, n + 1);
        st.project.layers[1].clip = true;
        st
    }

    /// Run one texture sync and hand back what egui was asked to upload.
    fn sync(st: &mut AppState, ctx: &egui::Context) -> egui::TexturesDelta {
        // The app's limit (see `AppState::new`), not egui's headless 2048.
        let raw = egui::RawInput {
            max_texture_side: Some(8192),
            ..Default::default()
        };
        ctx.run(raw, |ctx| st.sync_textures(ctx)).textures_delta
    }

    /// Alpha of the uploaded image for texture `id` at `(x, y)` of the image.
    fn uploaded_alpha(delta: &egui::TexturesDelta, id: egui::TextureId, x: usize, y: usize) -> u8 {
        let (_, d) = delta.set.iter().find(|(t, _)| *t == id).expect("texture uploaded");
        match &d.image {
            egui::ImageData::Color(img) => img.pixels[y * img.size[0] + x].a(),
            _ => panic!("colour image expected"),
        }
    }

    #[test]
    fn a_clipped_layer_shows_only_over_its_base() {
        let mut st = clip_state();
        let ctx = egui::Context::default();
        let delta = sync(&mut st, &ctx);
        let (id, tex, _) = st.layer_texture(1).expect("clipped layer drawn");
        assert_eq!(id, st.project.layers[1].resolve(0).unwrap());
        let tex = tex.id();
        let w = st.project.width as usize;
        assert_eq!(uploaded_alpha(&delta, tex, 10, 10), 255, "over the base");
        assert_eq!(uploaded_alpha(&delta, tex, w - 10, 10), 0, "past it");
        // Its plain cell texture is never needed.
        assert!(!st.cell_textures.contains_key(&id));
    }

    #[test]
    fn painting_the_base_redoes_just_that_rect_of_the_clip() {
        let mut st = clip_state();
        let ctx = egui::Context::default();
        let _ = sync(&mut st, &ctx);
        let tex = st.layer_texture(1).unwrap().1.id();
        // Ink a block of the base's empty right half.
        let base = st.project.layers[0].resolve(0).unwrap();
        let w = st.project.width;
        let c = st.project.cell_mut(base).unwrap();
        c.dirty = None;
        for y in 100..110u32 {
            for x in w - 50..w - 40 {
                let i = ((y * w + x) * 4) as usize;
                c.pixels_mut()[i..i + 4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
        c.mark_dirty(w - 50, 100, 10, 10);
        st.mark_dirty(base);
        let delta = sync(&mut st, &ctx);
        let (_, d) = delta.set.iter().find(|(t, _)| *t == tex).expect("clip redone");
        assert_eq!(d.pos, Some([(w - 50) as usize, 100]), "a partial upload");
        assert_eq!(d.image.size(), [10, 10]);
        assert_eq!(uploaded_alpha(&delta, tex, 0, 0), 255);
    }

    /// Flipping or turning the base swaps its cell for a new one, maybe of
    /// another size; the clipped layer must be cut against the new one.
    #[test]
    fn a_flipped_or_turned_base_recuts_its_clipped_layer() {
        let mut st = clip_state();
        let ctx = egui::Context::default();
        let _ = sync(&mut st, &ctx);
        let w = st.project.width as usize;
        st.orient_drawings(0, Orient::FLIP_H, false);
        let delta = sync(&mut st, &ctx);
        let tex = st.layer_texture(1).expect("still drawn").1.id();
        assert_eq!(uploaded_alpha(&delta, tex, 10, 10), 0, "the base left this side");
        assert_eq!(uploaded_alpha(&delta, tex, w - 10, 10), 255, "and moved here");
        // A quarter turn makes the base a different size: still a clean cut.
        st.orient_drawings(0, Orient::CW, false);
        let _ = sync(&mut st, &ctx);
        assert!(st.layer_texture(1).is_some());
        assert!(st.layer_texture(0).is_some());
        st.undo();
        st.undo();
        let delta = sync(&mut st, &ctx);
        let tex = st.layer_texture(1).unwrap().1.id();
        assert_eq!(uploaded_alpha(&delta, tex, 10, 10), 255, "undone");
    }

    #[test]
    fn hiding_the_base_hides_the_clipped_layer_and_unclipping_restores_it() {
        let mut st = clip_state();
        let ctx = egui::Context::default();
        let _ = sync(&mut st, &ctx);
        st.project.layers[0].visible = false;
        let _ = sync(&mut st, &ctx);
        assert!(st.layer_texture(1).is_none());
        st.project.layers[0].visible = true;
        st.project.layers[1].clip = false;
        let _ = sync(&mut st, &ctx);
        let id = st.project.layers[1].resolve(0).unwrap();
        assert_eq!(st.layer_texture(1).map(|(c, _, _)| c), Some(id));
        assert!(st.cell_textures.contains_key(&id), "drawn plain again");
    }

    #[test]
    fn a_stroke_on_a_clipped_layer_shows_while_it_is_drawn() {
        let mut st = clip_state();
        let ctx = egui::Context::default();
        let _ = sync(&mut st, &ctx);
        let tex = st.layer_texture(1).unwrap().1.id();
        st.project.current_layer = 1;
        st.dispatch(Action::ToolInk);
        st.pointer_down(st.make_sample(20.0, 20.0, 0.0));
        st.pointer_move(st.make_sample(60.0, 20.0, 0.1));
        assert!(st.stroke.is_some(), "mid-stroke");
        let delta = sync(&mut st, &ctx);
        let (_, d) = delta.set.iter().find(|(t, _)| *t == tex).expect("clip redone mid-stroke");
        assert!(d.pos.is_some(), "just the stroke's rect");
        st.pointer_up();
    }

    #[test]
    fn merging_a_clipped_layer_down_keeps_only_what_showed() {
        let mut st = clip_state();
        st.project.current_layer = 1;
        assert!(st.can_merge_down());
        st.merge_layer_down();
        assert_eq!(st.project.layers.len(), 1);
        let id = st.project.layers[0].resolve(0).unwrap();
        let c = st.project.cell(id).unwrap();
        let w = st.project.width;
        // Merged cells may be bigger than the frame, centred on it.
        let (ox, oy) = ((c.width - w) / 2, (c.height - st.project.height) / 2);
        let px = |x: u32, y: u32| {
            let i = (((y + oy) * c.width + x + ox) * 4) as usize;
            [c.pixels()[i], c.pixels()[i + 1], c.pixels()[i + 2], c.pixels()[i + 3]]
        };
        assert_eq!(px(10, 10), [255, 0, 0, 255], "red over the base");
        assert_eq!(px(w - 10, 10)[3], 0, "the clipped-away part stays away");
    }

    #[test]
    fn layer_keys_walk_the_stack_and_stop_at_the_ends() {
        let mut st = AppState::for_test();
        st.project.add_layer();
        st.project.add_layer();
        st.project.current_layer = 1;
        st.dispatch(Action::LayerSelectAbove);
        assert_eq!(st.project.current_layer, 2);
        st.dispatch(Action::LayerSelectAbove);
        assert_eq!(st.project.current_layer, 2, "stops at the top, no wrap");
        st.dispatch(Action::LayerSelectBottom);
        assert_eq!(st.project.current_layer, 0);
        st.dispatch(Action::LayerSelectBelow);
        assert_eq!(st.project.current_layer, 0, "stops at the bottom, no wrap");
        st.dispatch(Action::LayerSelectTop);
        assert_eq!(st.project.current_layer, 2);
        // Picks count rows down from the top of the panel.
        st.dispatch(Action::LayerPick1);
        assert_eq!(st.project.current_layer, 2);
        st.dispatch(Action::LayerPick3);
        assert_eq!(st.project.current_layer, 0);
        st.dispatch(Action::LayerPick4);
        assert_eq!(st.project.current_layer, 0, "no fourth row: stay put");
    }

    #[test]
    fn moving_a_layer_by_key_is_one_undo_step_and_free_at_the_ends() {
        let mut st = AppState::for_test();
        st.project.add_layer();
        st.project.current_layer = 0;
        let bottom = st.project.layers[0].uid;
        st.dispatch(Action::LayerMoveUp);
        assert_eq!(st.project.layers[1].uid, bottom, "moved up");
        assert_eq!(st.project.current_layer, 1, "selection rides along");
        // Already on top: no edit, so no undo entry either.
        st.dispatch(Action::LayerMoveUp);
        st.undo();
        assert_eq!(st.project.layers[0].uid, bottom, "one undo puts it back");
        assert_eq!(st.project.current_layer, 0);
    }

    #[test]
    fn the_layer_shortcuts_toggle_clip_and_alpha_lock() {
        let mut st = AppState::for_test();
        st.project.add_layer();
        st.dispatch(Action::LayerClipToggle);
        st.dispatch(Action::LayerAlphaLockToggle);
        let l = &st.project.layers[st.project.current_layer];
        assert!(l.clip && l.alpha_lock);
        st.dispatch(Action::LayerClipToggle);
        assert!(!st.project.layers[st.project.current_layer].clip);
    }

    #[test]
    fn a_stroke_on_an_alpha_locked_layer_only_recolours() {
        let mut st = clip_state();
        // Paint on the base, locked: its left half is ink, its right bare.
        st.project.layers[1].clip = false;
        st.project.current_layer = 0;
        st.project.layers[0].alpha_lock = true;
        st.dispatch(Action::ToolInk);
        st.brush.color = [0, 200, 0, 255];
        st.brush.radius = 6.0;
        let mid = (st.project.width / 2) as f32;
        st.pointer_down(st.make_sample(mid - 40.0, 50.0, 0.0));
        st.pointer_move(st.make_sample(mid + 40.0, 50.0, 0.1));
        st.pointer_up();
        let id = st.project.layers[0].resolve(0).unwrap();
        let c = st.project.cell(id).unwrap();
        let px = |x: f32| {
            let i = ((50 * c.width + x as u32) * 4) as usize;
            [c.pixels()[i], c.pixels()[i + 1], c.pixels()[i + 2], c.pixels()[i + 3]]
        };
        assert_eq!(px(mid - 30.0), [0, 200, 0, 255], "ink recoloured");
        assert_eq!(px(mid + 30.0)[3], 0, "bare canvas still bare");
    }

    #[test]
    fn a_project_without_clipping_makes_no_clip_textures() {
        let mut st = clip_state();
        st.project.layers[1].clip = false;
        let ctx = egui::Context::default();
        let _ = sync(&mut st, &ctx);
        assert!(st.clip_textures.is_empty());
    }

    // --- Fill presses ---

    const FILL_RED: [u8; 4] = [255, 0, 0, 255];

    /// Ink a 1-px box outline over 100..=200 on `layer`'s drawing at the current
    /// frame, its top edge broken by a 4-px hole at x = 140..144.
    fn ink_box(state: &mut AppState, layer: usize) {
        let keep = state.project.current_layer;
        state.project.current_layer = layer;
        let id = state.project.ensure_active_cell();
        state.project.current_layer = keep;
        let c = state.project.cell_mut(id).unwrap();
        for i in 100..=200u32 {
            for (x, y) in [(i, 100), (i, 200), (100, i), (200, i)] {
                if y == 100 && (140..144).contains(&x) {
                    continue;
                }
                let k = ((y * c.width + x) * 4) as usize;
                c.pixels_mut()[k..k + 4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
    }

    fn fill_state() -> AppState {
        let mut state = AppState::for_test();
        state.dispatch(Action::ToolFill);
        state.brush.color = FILL_RED;
        state.brush.fill_gap = 0;
        state.brush.fill_expand = 0;
        ink_box(&mut state, 0);
        state
    }

    fn px(state: &AppState, x: u32, y: u32) -> [u8; 4] {
        let id = state.project.resolved_current().unwrap();
        let c = state.project.cell(id).unwrap();
        let k = ((y * c.width + x) * 4) as usize;
        [c.pixels()[k], c.pixels()[k + 1], c.pixels()[k + 2], c.pixels()[k + 3]]
    }

    /// Press the bucket at cell point `(x, y)`, landing on screen at `at`.
    fn fill_press(state: &mut AppState, x: f32, y: f32, at: egui::Pos2) {
        state.pointer_down(state.make_sample(x, y, 0.0));
        state.fill_anchor(at);
    }

    #[test]
    fn dragging_sideways_shuts_the_gap_live_and_lands_as_one_undo() {
        let mut state = fill_state();
        let at = egui::pos2(500.0, 500.0);
        fill_press(&mut state, 150.0, 150.0, at);
        assert_eq!(px(&state, 50, 50), FILL_RED, "gap 0: pours out of the hole");
        // 8 px dead zone, then 12 px a step: 60 px is four steps.
        state.fill_drag_to(at + egui::vec2(60.0, 0.0));
        assert_eq!(state.brush.fill_gap, 4);
        assert_eq!(state.brush.fill_expand, 0);
        assert_eq!(px(&state, 50, 50)[3], 0, "the drag put the leak back");
        assert_eq!(px(&state, 150, 150), FILL_RED);
        assert!(state.fill_dragging());
        state.pointer_up();
        assert!(!state.fill_dragging());
        assert_eq!(state.history.undo_len(), 1);
        state.undo();
        assert_eq!(px(&state, 150, 150)[3], 0);
        assert_eq!(px(&state, 50, 50)[3], 0);
        // The drag's value stays in the slider for the next press.
        assert_eq!(state.brush.fill_gap, 4);
    }

    #[test]
    fn dragging_up_grows_the_expand_and_back_down_undoes_it() {
        let mut state = fill_state();
        state.brush.fill_gap = 4;
        let at = egui::pos2(500.0, 500.0);
        fill_press(&mut state, 150.0, 150.0, at);
        assert_eq!(px(&state, 100, 150), [0, 0, 0, 255], "the line, before any expand");
        state.fill_drag_to(at + egui::vec2(0.0, -30.0));
        assert_eq!((state.brush.fill_gap, state.brush.fill_expand), (4, 1));
        assert_eq!(px(&state, 100, 150), FILL_RED, "expand 1 tucks under the line");
        state.fill_drag_to(at + egui::vec2(0.0, 5.0));
        assert_eq!(state.brush.fill_expand, 0);
        assert_eq!(px(&state, 100, 150), [0, 0, 0, 255], "and gives it back");
        state.pointer_up();
        assert_eq!(state.history.undo_len(), 1);
    }

    #[test]
    fn a_plain_tap_fills_once() {
        let mut state = fill_state();
        state.brush.fill_gap = 4;
        fill_press(&mut state, 150.0, 150.0, egui::pos2(10.0, 10.0));
        // A few points of wobble stay inside the dead zone.
        state.fill_drag_to(egui::pos2(15.0, 6.0));
        state.pointer_up();
        assert_eq!((state.brush.fill_gap, state.brush.fill_expand), (4, 0));
        assert_eq!(px(&state, 150, 150), FILL_RED);
        assert_eq!(px(&state, 50, 50)[3], 0);
        assert_eq!(state.history.undo_len(), 1);
    }

    #[test]
    fn cancelling_a_fill_press_leaves_nothing_behind() {
        let mut state = fill_state();
        let at = egui::pos2(500.0, 500.0);
        fill_press(&mut state, 150.0, 150.0, at);
        state.fill_drag_to(at + egui::vec2(60.0, -30.0));
        assert!(state.cancel_fill());
        assert_eq!(px(&state, 150, 150)[3], 0);
        assert_eq!(px(&state, 50, 50)[3], 0);
        assert_eq!(px(&state, 100, 150), [0, 0, 0, 255]);
        // The pen lifting afterwards records nothing.
        state.pointer_up();
        assert_eq!(state.history.undo_len(), 0);
    }

    #[test]
    fn reading_all_visible_layers_walls_the_fill_with_another_layers_lines() {
        let mut state = fill_state();
        // Lines on layer 0, colour on a fresh layer above with no link.
        state.project.add_layer_above_active();
        state.brush.fill_gap = 4;
        let at = egui::pos2(0.0, 0.0);
        fill_press(&mut state, 150.0, 150.0, at);
        state.pointer_up();
        assert_eq!(px(&state, 50, 50), FILL_RED, "its own empty pixels: floods");
        state.undo();
        state.brush.fill_all_visible = true;
        fill_press(&mut state, 150.0, 150.0, at);
        state.pointer_up();
        assert_eq!(px(&state, 150, 150), FILL_RED);
        assert_eq!(px(&state, 50, 50)[3], 0, "held by layer 0's box");
        // Hidden layers don't count.
        state.undo();
        state.project.layers[0].visible = false;
        fill_press(&mut state, 150.0, 150.0, at);
        state.pointer_up();
        assert_eq!(px(&state, 50, 50), FILL_RED);
    }

    /// Tab back out of the mini-timeline mode leaves the shortcuts live. egui
    /// also reads that Tab as "focus the next widget", which on that frame is
    /// the first panel button, and a focused widget gates off the shortcuts.
    #[test]
    fn tabbing_the_panels_back_leaves_the_shortcuts_live() {
        let mut st = AppState::for_test();
        st.shortcuts = ShortcutMap::default();
        st.show_panels = false;
        st.show_mini_timeline = true;
        st.dispatch(Action::ToolInk);
        let ctx = egui::Context::default();
        // One frame the way `update` runs it, with `key` pressed.
        let frame = |st: &mut AppState, key: Option<egui::Key>| {
            let events = key
                .map(|key| egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                })
                .into_iter()
                .collect();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                let tab_focus = st.handle_keys(ctx);
                ui::shell::draw(st, ctx);
                st.drop_stray_focus(ctx, tab_focus);
            });
        };
        frame(&mut st, None);
        frame(&mut st, Some(egui::Key::Tab));
        assert!(st.show_panels);
        assert_eq!(ctx.memory(|m| m.focused()), None, "Tab left a panel widget focused");
        frame(&mut st, Some(egui::Key::Q));
        assert_eq!(st.tool, ActiveTool::Pencil);
    }

    /// Settings takes a bare Shift for a hold binding once it comes back up,
    /// and a key with whatever is held for a press binding as before.
    #[test]
    fn capturing_a_bare_modifier_in_settings() {
        use crate::input::shortcuts::KeyCombo;
        let mut st = AppState::for_test();
        st.shortcuts = ShortcutMap::default();
        let ctx = egui::Context::default();
        let frame = |st: &mut AppState, mods: egui::Modifiers, key: Option<egui::Key>| {
            let events = key
                .map(|key| egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers: mods,
                })
                .into_iter()
                .collect();
            let raw = egui::RawInput {
                events,
                modifiers: mods,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                st.handle_keys(ctx);
            });
        };
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        st.rebinding = Some(Action::HoldToolAdjust);
        frame(&mut st, egui::Modifiers::NONE, None);
        frame(&mut st, shift, None);
        assert_eq!(st.rebinding, Some(Action::HoldToolAdjust), "still listening while held");
        frame(&mut st, egui::Modifiers::NONE, None);
        assert_eq!(st.rebinding, None);
        assert_eq!(
            st.shortcuts.get(Action::HoldToolAdjust),
            Some(KeyCombo::modifier_only(false, true, false))
        );
        // A press binding ignores the bare modifier and waits for a key.
        st.rebinding = Some(Action::ToolInk);
        frame(&mut st, shift, None);
        frame(&mut st, egui::Modifiers::NONE, None);
        assert_eq!(st.rebinding, Some(Action::ToolInk));
        frame(&mut st, shift, Some(egui::Key::I));
        assert_eq!(st.shortcuts.get(Action::ToolInk), Some(KeyCombo::shift(egui::Key::I)));
    }

    /// A focused button takes Space as a click. Held for a pan, Space must
    /// let go of the button rather than press it.
    #[test]
    fn space_held_for_pan_does_not_click_a_focused_button() {
        use crate::input::shortcuts::KeyCombo;
        for (pan_on_space, clicks) in [(true, false), (false, true)] {
            let mut st = AppState::for_test();
            st.shortcuts = ShortcutMap::default();
            st.shortcuts.bindings.clear();
            if pan_on_space {
                st.shortcuts.set(Action::CanvasPan, KeyCombo::plain(egui::Key::Space));
            }
            let ctx = egui::Context::default();
            let button = egui::Id::new("test button");
            let mut clicked = false;
            let mut frame = |st: &mut AppState, space: bool, focus: bool| {
                let events = if space {
                    vec![egui::Event::Key {
                        key: egui::Key::Space,
                        physical_key: Some(egui::Key::Space),
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }]
                } else {
                    vec![]
                };
                let raw = egui::RawInput {
                    events,
                    ..Default::default()
                };
                let _ = ctx.run(raw, |ctx| {
                    st.handle_keys(ctx);
                    egui::Area::new(egui::Id::new("test area")).show(ctx, |ui| {
                        let r = ui.add(egui::Button::new("b").sense(egui::Sense::click()));
                        let r = ui.interact(r.rect, button, egui::Sense::click());
                        if focus {
                            r.request_focus();
                        }
                        clicked |= r.clicked();
                    });
                });
            };
            frame(&mut st, false, true);
            assert_eq!(ctx.memory(|m| m.focused()), Some(button));
            frame(&mut st, true, false);
            assert_eq!(clicked, clicks, "Space with pan on it: {pan_on_space}");
        }
    }

    /// Pinned, the wheel's colour is every tool's: set on one, it is there on
    /// the next. Unpinned, each tool keeps its own again.
    #[test]
    fn a_pinned_wheel_gives_every_tool_one_colour() {
        let mut st = AppState::for_test();
        st.set_tool(ActiveTool::Pencil);
        st.set_brush_color([200, 0, 0]);
        st.set_tool(ActiveTool::Ink);
        st.set_brush_color([0, 0, 200]);
        st.set_tool(ActiveTool::Pencil);
        assert_eq!(st.brush.color, [200, 0, 0, 255], "unpinned: each its own");

        st.set_wheel_pinned(true);
        st.set_tool(ActiveTool::Fill);
        assert_eq!(st.brush.color, [200, 0, 0, 255], "pinning hands the colour on");
        st.set_brush_color([0, 150, 0]);
        for t in [ActiveTool::Ink, ActiveTool::Shape, ActiveTool::Pencil] {
            st.set_tool(t);
            assert_eq!(st.brush.color, [0, 150, 0, 255], "{t:?}");
        }

        st.set_wheel_pinned(false);
        st.set_brush_color([9, 9, 9]);
        st.set_tool(ActiveTool::Ink);
        assert_eq!(st.brush.color, [0, 150, 0, 255], "let go: tools drift apart again");
        st.dispatch(Action::WheelPinToggle);
        assert!(st.wheel_pinned());
    }

    /// An opacity drag over a pick sets every picked layer, and lands as one
    /// undo step however many frames it took.
    #[test]
    fn an_opacity_drag_sets_the_pick_in_one_undo_step() {
        let mut st = AppState::for_test();
        st.project.add_layer();
        st.project.add_layer();
        st.selected_layers.insert(st.project.layers[0].uid);
        st.project.current_layer = 2;
        let depth = st.history.undo_len();
        for v in [0.9, 0.7, 0.5] {
            st.set_selected_opacity(v);
        }
        st.commit_layer_opacity();
        st.commit_layer_opacity();
        let opac: Vec<f32> = st.project.layers.iter().map(|l| l.opacity).collect();
        assert_eq!(opac, [0.5, 1.0, 0.5]);
        assert_eq!(st.history.undo_len(), depth + 1);
        st.undo();
        assert!(st.project.layers.iter().all(|l| l.opacity == 1.0));
    }

    /// "Last layer" survives the stack changing: after adding a layer it goes
    /// back to the one the layer was added over, and a deleted layer leaves
    /// the one before it as the way back.
    #[test]
    fn last_layer_survives_adding_and_deleting_layers() {
        let mut st = AppState::for_test();
        st.project.add_layer();
        st.project.add_layer();
        st.project.current_layer = 1;
        st.track_layer_change();
        let rough = st.project.layers[1].uid;

        st.structural_edit(false, |p| {
            p.add_layer_above_active();
        });
        st.track_layer_change();
        assert_eq!(st.project.current_layer, 2, "the new layer is active");
        st.goto_last_layer();
        st.track_layer_change();
        assert_eq!(st.project.layers[st.project.current_layer].uid, rough);

        // Over to the top, then delete it: back still leads to the rough.
        st.project.current_layer = st.project.layers.len() - 1;
        st.track_layer_change();
        st.delete_selected_layers();
        st.track_layer_change();
        st.goto_last_layer();
        assert_eq!(st.project.layers[st.project.current_layer].uid, rough);
    }

    /// With the panels moved to M and "last layer" put on Tab, Tab inside a
    /// text field must go to the last layer: what gets through focus is the
    /// map's binding on Tab, not a key wired to the panels.
    #[test]
    fn tab_reaches_its_own_binding_from_a_focused_field() {
        use crate::input::shortcuts::KeyCombo;
        let mut st = AppState::for_test();
        st.shortcuts = ShortcutMap::default();
        st.shortcuts.set(Action::TogglePanels, KeyCombo::plain(egui::Key::M));
        st.shortcuts.set(Action::ToggleMiniTimeline, KeyCombo::shift(egui::Key::M));
        st.shortcuts.set(Action::LayerLast, KeyCombo::plain(egui::Key::Tab));
        st.project.add_layer();
        let panels = st.show_panels;
        let ctx = egui::Context::default();
        let field = egui::Id::new("test field");
        let mut text = String::new();
        // One frame: keys, then a text field (focused on request), then the
        // focus clean-up, the order `update` runs them in.
        let mut frame = |st: &mut AppState, key: Option<(egui::Key, egui::Modifiers)>, focus: bool| {
            let events = key
                .map(|(key, modifiers)| egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers,
                })
                .into_iter()
                .collect();
            let raw = egui::RawInput {
                events,
                modifiers: key.map_or(egui::Modifiers::NONE, |k| k.1),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                let tab_focus = st.handle_keys(ctx);
                egui::Area::new(egui::Id::new("test area")).show(ctx, |ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut text).id(field));
                    if focus {
                        r.request_focus();
                    }
                });
                st.drop_stray_focus(ctx, tab_focus);
            });
        };
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        for (mods, jumps) in [(egui::Modifiers::NONE, true), (shift, false)] {
            st.project.current_layer = 1;
            st.prev_layer = Some(st.project.layers[0].uid);
            frame(&mut st, None, true);
            assert_eq!(ctx.memory(|m| m.focused()), Some(field));
            frame(&mut st, Some((egui::Key::Tab, mods)), false);
            let want = if jumps { 0 } else { 1 };
            assert_eq!(st.project.current_layer, want, "Tab with {mods:?}");
            assert_eq!(st.show_panels, panels, "Tab is no longer the panels");
        }
    }

    /// The Krita link end to end on a real file, minus Krita: send, a "Krita
    /// save" (our own writer standing in — the reader is proven against
    /// Krita's output in `io::kra`), the watcher noticing it, the worker
    /// reading it, and the pull landing as one undo step. A reference
    /// background rides along to check it never leaves this app.
    #[test]
    fn krita_save_lands_as_one_undo_step() {
        let mut st = AppState::for_test();
        let ctx = egui::Context::default();
        let dir = std::env::temp_dir().join(format!("animator-krita-test-{}", std::process::id()));
        let path = dir.join("link.kra");

        // A reference background under the drawing layer.
        let bg = st.project.add_background_layer("BG");
        st.project.layers[bg].reference = true;
        let bg_cell = st.project.cells.len();
        st.project.cells.push(Canvas::new(st.project.width, st.project.height).into());
        st.project.layers[bg].set_key(0, bg_cell);
        let bg_before = st.project.layers[bg].exposures.clone();
        let ink = 1;

        let mut link = KritaLink::new(path.clone());
        link.poll_every = Duration::ZERO;
        link.settle = Duration::ZERO;
        let keep: Vec<bool> = krita_link::sendable(&st.project);
        assert_eq!(keep, [false, true]);
        let ink_uuid = krita_link::new_uuid();
        link.uuids.insert(st.project.layers[ink].uid, ink_uuid.clone());
        let uuids = vec![ink_uuid];
        let krita_link::Sent { baseline: base, stamp, .. } =
            krita_link::send(st.project.clone(), &keep, &uuids, Some(0), &path).unwrap();
        link.baseline = Some(base);
        link.mark_seen(stamp);
        st.krita = Some(link);

        // The background never reached the file.
        let sent = kra::read(std::fs::read(&path).unwrap()).unwrap();
        let names: Vec<&str> = sent.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, [st.project.layers[ink].name.as_str()]);

        // Nothing changed on disk: polling does nothing.
        for _ in 0..3 {
            st.poll_krita(&ctx);
        }
        assert!(!st.history.can_undo());

        // "Krita" paints on the drawing and saves; what Krita holds is the
        // ink layer alone.
        let mut k = st.project.clone();
        k.layers.remove(bg);
        let id = k.layers[0].exposures[0].unwrap();
        k.cell_mut(id).unwrap().pixels_mut()[0..4].copy_from_slice(&[255, 0, 0, 255]);
        let bytes = kra::write(&k, &kra::WriteOpts { layer_uuids: &uuids, selected_layer: 0, carried: &[] }).unwrap();
        std::fs::write(&path, bytes).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        while !st.history.can_undo() && Instant::now() < deadline {
            st.poll_krita(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        let now = st.project.layers[ink].exposures[0].unwrap();
        assert_ne!(now, id, "the pull should have landed");
        assert_eq!(&st.project.cells[now].pixels()[0..4], &[255, 0, 0, 255]);
        assert!(st.krita_toast.as_ref().is_some_and(|t| t.0.starts_with("Updated from Krita")));
        // The background is where it was, untouched, still not linked.
        assert_eq!(st.project.layers[bg].name, "BG");
        assert_eq!(st.project.layers[bg].exposures, bg_before);

        // The same save seen again changes nothing.
        for _ in 0..3 {
            st.poll_krita(&ctx);
        }
        st.undo();
        assert_eq!(st.project.layers[ink].exposures[0], Some(id));
        assert!(!st.history.can_undo());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A live link on a temp file, as a first send leaves it — without
    /// starting Krita — and with zero poll delays.
    fn linked(tag: &str) -> (AppState, PathBuf, PathBuf) {
        let mut st = AppState::for_test();
        let dir = std::env::temp_dir().join(format!("animator-round-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("link.kra");
        let mut link = KritaLink::new(path.clone());
        link.poll_every = Duration::ZERO;
        link.settle = Duration::ZERO;
        st.krita = Some(link);
        st.begin_send(0, None, None);
        pump(&mut st, |st| st.krita.as_ref().is_some_and(|k| k.baseline.is_some()));
        (st, dir, path)
    }

    /// Run the app's per-frame Krita step until `done`, or fail after 10 s.
    fn pump(st: &mut AppState, done: impl Fn(&AppState) -> bool) {
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(st) {
            assert!(Instant::now() < deadline, "timed out");
            st.poll_krita(&ctx);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn to_krita(path: &std::path::Path) -> (String, String) {
        let m = crate::krita_helper::read_msg(&crate::krita_helper::to_krita(path)).unwrap_or_default();
        (m.get("seq").cloned().unwrap_or_default(), m.get("msg").cloned().unwrap_or_default())
    }

    fn stage(st: &AppState) -> Option<krita_link::Stage> {
        st.krita.as_ref().and_then(|k| k.round.as_ref()).map(|r| r.stage)
    }

    /// "Krita" (the file on disk) as it would save `k`, under the link's uuids.
    fn krita_saves(st: &AppState, k: &Project, path: &std::path::Path) {
        let link = st.krita.as_ref().unwrap();
        let uuids: Vec<String> = k.layers.iter().map(|l| link.uuids[&l.uid].clone()).collect();
        std::fs::write(path, kra::write(k, &kra::WriteOpts { layer_uuids: &uuids, selected_layer: 0, carried: &[] }).unwrap())
            .unwrap();
    }

    /// Pixel 0 of layer 0's first drawing in the `.kra` on disk.
    fn first_pixel_on_disk(st: &AppState, path: &std::path::Path) -> [u8; 4] {
        let doc = kra::read(std::fs::read(path).unwrap()).unwrap();
        let l = &doc.layers[0];
        let (pw, ph) = (st.project.width, st.project.height);
        let (x, y) = kra::cell_origin(pw, ph, pw, ph);
        let (c, _) = l.drawings[l.keys[0].1].place(x, y, pw, ph);
        [c.pixels()[0], c.pixels()[1], c.pixels()[2], c.pixels()[3]]
    }

    /// The helper answers "ready", the new file goes out, "sent" is posted,
    /// and the helper's "reloaded" makes the toast — no closing in Krita.
    #[test]
    fn helper_round_reloads_in_place() {
        let (mut st, dir, path) = linked("clean");
        st.krita.as_mut().unwrap().use_helper = Some(true);
        st.project.layers[0].name = "Ink here".into();

        st.begin_send(0, None, None);
        assert_eq!(to_krita(&path), ("1".into(), "request".into()));
        for _ in 0..5 {
            st.poll_krita(&egui::Context::default());
        }
        assert!(matches!(stage(&st), Some(krita_link::Stage::AwaitReady { .. })), "waits for the helper");

        crate::krita_helper::tests::plugin_says(&path, &[("seq", "1"), ("msg", "ready"), ("saved", "0")]);
        pump(&mut st, |_| to_krita(&path).1 == "sent");
        let doc = kra::read(std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(doc.layers[0].name, "Ink here");
        assert!(matches!(stage(&st), Some(krita_link::Stage::AwaitReloaded { .. })));

        crate::krita_helper::tests::plugin_says(&path, &[("seq", "1"), ("msg", "reloaded")]);
        pump(&mut st, |st| stage(st).is_none());
        assert!(st.krita_toast.as_ref().is_some_and(|t| t.0.starts_with("Updated in Krita") && !t.1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Unsaved Krita edits: the helper saves them, they land here as an undo
    /// step *before* the file is rewritten, and the reloaded file has both
    /// sides' changes.
    #[test]
    fn helper_saves_krita_edits_first() {
        let (mut st, dir, path) = linked("saved");
        st.krita.as_mut().unwrap().use_helper = Some(true);
        let mut k = st.project.clone(); // what Krita has: the sent version…
        st.project.layers[0].name = "Ink here".into(); // …while we rename here
        let id = k.layers[0].exposures[0].unwrap();
        k.cell_mut(id).unwrap().pixels_mut()[0..4].copy_from_slice(&[255, 0, 0, 255]); // …and Krita paints

        st.begin_send(0, None, None);
        krita_saves(&st, &k, &path);
        crate::krita_helper::tests::plugin_says(&path, &[("seq", "1"), ("msg", "ready"), ("saved", "1")]);
        pump(&mut st, |_| to_krita(&path).1 == "sent");

        assert!(st.history.can_undo(), "Krita's save came in as an undo step");
        let doc = kra::read(std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(doc.layers[0].name, "Ink here");
        assert_eq!(first_pixel_on_disk(&st, &path), [255, 0, 0, 255]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Installed but not enabled (or Krita closed): no answer, so it falls
    /// back to a plain send and says how to turn the helper on.
    #[test]
    fn silent_helper_falls_back_to_a_plain_send() {
        let (mut st, dir, path) = linked("silent");
        {
            let l = st.krita.as_mut().unwrap();
            l.use_helper = Some(true);
            l.ready_timeout = Duration::ZERO;
        }
        st.project.layers[0].name = "Ink here".into();
        st.begin_send(0, None, None);
        pump(&mut st, |st| stage(st).is_none() && st.krita_send.is_none());
        assert_eq!(to_krita(&path).1, "request", "no \"sent\" without a ready helper");
        let doc = kra::read(std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(doc.layers[0].name, "Ink here");
        assert!(st.krita_toast.as_ref().is_some_and(|t| t.0.contains("helper didn't answer")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A save caught half-written (or any unreadable file) is reported and
    /// changes nothing; the next good save still comes in.
    #[test]
    fn unreadable_krita_save_changes_nothing_then_recovers() {
        let (mut st, dir, path) = linked("corrupt");
        let good = std::fs::read(&path).unwrap();
        std::fs::write(&path, &good[..good.len() / 2]).unwrap();
        pump(&mut st, |st| st.krita_toast.as_ref().is_some_and(|t| t.0.contains("Couldn't read")));
        assert!(!st.history.can_undo());

        let mut k = st.project.clone();
        let id = k.layers[0].exposures[0].unwrap();
        k.cell_mut(id).unwrap().pixels_mut()[0..4].copy_from_slice(&[9, 9, 9, 255]);
        krita_saves(&st, &k, &path);
        pump(&mut st, |st| st.history.can_undo());
        assert_eq!(first_pixel_on_disk(&st, &path), [9, 9, 9, 255]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Clicking Send again while a round is under way does nothing — no
    /// second request, no Krita lookup.
    #[test]
    fn a_second_send_during_a_round_is_ignored() {
        let (mut st, dir, path) = linked("twice");
        st.krita.as_mut().unwrap().use_helper = Some(true);
        st.begin_send(0, None, None);
        st.edit_in_krita(0, None);
        st.edit_in_krita(0, None);
        assert_eq!(to_krita(&path), ("1".into(), "request".into()));
        assert_eq!(st.krita.as_ref().unwrap().helper_seq, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Without the helper, a re-send still brings in a Krita save the
    /// watcher hasn't read yet before rewriting the file over it.
    #[test]
    fn resend_brings_in_an_unread_krita_save_first() {
        let (mut st, dir, path) = linked("unread");
        {
            let l = st.krita.as_mut().unwrap();
            l.use_helper = Some(false);
            l.poll_every = Duration::from_secs(3600); // the watcher won't see it
        }
        let mut k = st.project.clone();
        let id = k.layers[0].exposures[0].unwrap();
        k.cell_mut(id).unwrap().pixels_mut()[0..4].copy_from_slice(&[0, 255, 0, 255]);
        krita_saves(&st, &k, &path);

        st.begin_send(0, None, None);
        pump(&mut st, |st| stage(st).is_none() && st.krita_send.is_none());
        assert!(st.history.can_undo());
        assert_eq!(first_pixel_on_disk(&st, &path), [0, 255, 0, 255]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two drawn layers and a link not yet sent, zero poll delays. Returns
    /// the bottom and top layers' uids too.
    fn two_layer_link(tag: &str) -> (AppState, PathBuf, PathBuf, u64, u64) {
        let mut st = AppState::for_test();
        st.project.add_layer();
        let id = st.project.cells.len();
        let mut c = Canvas::new(st.project.width, st.project.height);
        c.pixels_mut()[0..4].copy_from_slice(&[0, 0, 255, 255]);
        st.project.cells.push(c.into());
        st.project.layers[1].set_key(0, id);
        let dir = std::env::temp_dir().join(format!("animator-layer-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("link.kra");
        let mut link = KritaLink::new(path.clone());
        link.poll_every = Duration::ZERO;
        link.settle = Duration::ZERO;
        link.use_helper = Some(false);
        st.krita = Some(link);
        let (bottom, top) = (st.project.layers[0].uid, st.project.layers[1].uid);
        (st, dir, path, bottom, top)
    }

    fn names_on_disk(path: &std::path::Path) -> Vec<String> {
        kra::read(std::fs::read(path).unwrap()).unwrap().layers.into_iter().map(|l| l.name).collect()
    }

    /// "Krita" paints pixel 0 of layer `uid`'s first drawing and saves, as
    /// Krita holds it: that layer alone.
    fn krita_paints_only(st: &AppState, uid: u64, px: [u8; 4], path: &std::path::Path) {
        let mut k = st.project.clone();
        k.layers.retain(|l| l.uid == uid);
        let id = k.layers[0].exposures[0].unwrap();
        k.cell_mut(id).unwrap().pixels_mut()[0..4].copy_from_slice(&px);
        krita_saves(st, &k, path);
    }

    fn layer_by_uid(st: &AppState, uid: u64) -> &crate::doc::layer::Layer {
        st.project.layers.iter().find(|l| l.uid == uid).expect("layer still here")
    }

    /// The layer menu's send takes that layer alone; a Krita save then
    /// changes it and leaves the layer that wasn't sent as it was.
    #[test]
    fn layer_send_takes_only_that_layer() {
        let (mut st, dir, path, bottom, top) = two_layer_link("only");
        let bottom_before = st.project.layers[0].exposures.clone();
        st.begin_send(0, Some(top), None);
        pump(&mut st, |st| st.krita.as_ref().is_some_and(|k| k.baseline.is_some()));
        assert_eq!(names_on_disk(&path), ["Layer 2"]);
        assert_eq!(st.krita.as_ref().unwrap().uuids.keys().copied().collect::<Vec<_>>(), [top]);
        assert!(st.krita_toast.as_ref().is_some_and(|t| !t.0.contains("Kept out")));

        krita_paints_only(&st, top, [1, 2, 3, 255], &path);
        pump(&mut st, |st| st.history.can_undo());
        assert_eq!(st.project.layers.len(), 2);
        let now = layer_by_uid(&st, top).exposures[0].unwrap();
        assert_eq!(&st.project.cells[now].pixels()[0..4], &[1, 2, 3, 255]);
        assert_eq!(layer_by_uid(&st, bottom).exposures, bottom_before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With the whole project in Krita, the layer menu's re-send narrows
    /// Krita's file to that layer; the other layer stays here, unlinked, and
    /// Krita's next save doesn't delete it.
    #[test]
    fn layer_resend_narrows_a_whole_link() {
        let (mut st, dir, path, bottom, top) = two_layer_link("narrow");
        let bottom_before = st.project.layers[0].exposures.clone();
        st.begin_send(1, None, None);
        pump(&mut st, |st| st.krita.as_ref().is_some_and(|k| k.baseline.is_some()));
        assert_eq!(names_on_disk(&path), ["Layer 1", "Layer 2"]);

        st.begin_send(0, Some(top), None);
        pump(&mut st, |st| stage(st).is_none() && st.krita_send.is_none());
        assert_eq!(names_on_disk(&path), ["Layer 2"]);
        assert!(!st.krita.as_ref().unwrap().uuids.contains_key(&bottom));
        assert!(st.krita_toast.as_ref().is_some_and(|t| !t.0.contains("Kept out")));

        krita_paints_only(&st, top, [4, 5, 6, 255], &path);
        pump(&mut st, |st| st.history.can_undo());
        assert_eq!(st.project.layers.len(), 2);
        assert_eq!(layer_by_uid(&st, bottom).exposures, bottom_before);
        let now = layer_by_uid(&st, top).exposures[0].unwrap();
        assert_eq!(&st.project.cells[now].pixels()[0..4], &[4, 5, 6, 255]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A reference layer's menu send is refused before anything starts —
    /// no link, no Krita lookup.
    #[test]
    fn layer_send_of_a_reference_layer_sends_nothing() {
        let mut st = AppState::for_test();
        st.project.layers[0].reference = true;
        st.edit_in_krita(0, Some(0));
        assert!(st.krita.is_none() && st.krita_send.is_none());
        assert!(st.krita_toast.as_ref().is_some_and(|t| t.0.contains("stays here") && t.1));
    }

    /// The layer a re-send was for is gone once Krita's save is in: the
    /// round ends with a note and the file is left as it was.
    #[test]
    fn layer_gone_before_the_send_ends_the_round() {
        let (mut st, dir, path, _, _) = two_layer_link("gone");
        st.begin_send(0, None, None);
        pump(&mut st, |st| st.krita.as_ref().is_some_and(|k| k.baseline.is_some()));
        let before = std::fs::read(&path).unwrap();
        st.begin_send(0, Some(u64::MAX), None);
        pump(&mut st, |st| stage(st).is_none());
        assert!(st.krita_send.is_none());
        assert!(st.krita_toast.as_ref().is_some_and(|t| t.0.contains("gone") && t.1));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(st.krita.as_ref().unwrap().baseline.is_some(), "the link carries on");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn rect(min_x: u32, min_y: u32, max_x: u32, max_y: u32) -> DirtyRect {
        DirtyRect {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    /// The upload a stroke sends is that stroke's rect, grown by later ones,
    /// and a whole-cell change swallows any rect.
    #[test]
    fn dirty_regions_union_and_full_wins() {
        let a = Dirty::Rect(rect(10, 10, 20, 20));
        let b = Dirty::Rect(rect(15, 5, 40, 12));
        assert_eq!(Dirty::Clean.with(a), a);
        assert_eq!(a.with(Dirty::Clean), a);
        assert_eq!(a.with(b), Dirty::Rect(rect(10, 5, 40, 20)));
        assert_eq!(a.with(Dirty::Full), Dirty::Full);
        assert_eq!(Dirty::Full.with(a), Dirty::Full);
    }

    /// Ink `r` of cell `id` opaque black and report it the way a tool does.
    fn ink(st: &mut AppState, id: CellId, r: DirtyRect) {
        let c = st.project.cell_mut(id).unwrap();
        c.dirty = None;
        let w = c.width;
        for y in r.min_y..r.max_y {
            for x in r.min_x..r.max_x {
                let i = ((y * w + x) * 4) as usize;
                c.pixels_mut()[i..i + 4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
        c.mark_dirty(r.min_x, r.min_y, r.max_x - r.min_x, r.max_y - r.min_y);
        st.mark_dirty(id);
    }

    #[test]
    fn uploads_patch_inside_the_texture_and_recrop_past_it() {
        let mut c = Canvas::new(100, 50);
        c.dirty = None;
        assert_eq!(plan_upload(&c, None, Dirty::Full, None), TexPlan::Rebuild(rect(0, 0, 0, 0)));
        for x in 10..20 {
            c.pixels_mut()[(5 * 100 + x) * 4 + 3] = 255;
        }
        let have = rect(8, 3, 22, 8);
        assert_eq!(plan_upload(&c, None, Dirty::Clean, None), TexPlan::Rebuild(have), "no texture yet");
        assert_eq!(plan_upload(&c, Some(have), Dirty::Clean, None), TexPlan::Keep);
        let inside = rect(12, 4, 15, 6);
        assert_eq!(plan_upload(&c, Some(have), Dirty::Rect(inside), None), TexPlan::Patch(inside));
        // An edit out in the clear that left it clear (an eraser pass):
        // just the part over the texture.
        let erased = rect(0, 0, 12, 4);
        assert_eq!(
            plan_upload(&c, Some(have), Dirty::Rect(erased), None),
            TexPlan::Patch(rect(8, 3, 12, 4))
        );
        assert_eq!(plan_upload(&c, Some(have), Dirty::Rect(rect(50, 30, 60, 40)), None), TexPlan::Keep);
        // The drawing spread: crop around all of it.
        c.pixels_mut()[(40 * 100 + 70) * 4 + 3] = 255;
        let grown = rect(8, 3, 73, 43);
        assert_eq!(plan_upload(&c, Some(have), Dirty::Rect(rect(70, 40, 71, 41)), None), TexPlan::Rebuild(grown));
        assert_eq!(plan_upload(&c, Some(have), Dirty::Full, None), TexPlan::Rebuild(grown));
        // Under a live stroke a cell this small grows to all of itself.
        let whole = rect(0, 0, 100, 50);
        let stroke = Some(Some(have));
        assert_eq!(plan_upload(&c, Some(have), Dirty::Rect(rect(70, 40, 71, 41)), stroke), TexPlan::Rebuild(whole));
        assert_eq!(plan_upload(&c, Some(have), Dirty::Rect(inside), stroke), TexPlan::Patch(inside));
    }

    /// On a big cell a stroke's texture grows around the stroke, by half
    /// again and at least `STROKE_ROOM`, never to the whole cell at once.
    #[test]
    fn a_stroke_texture_grows_with_room_to_spare() {
        let (w, h) = (4320, 7680);
        let dab = rect(2000, 3000, 2100, 3050);
        let room = rect(1488, 2488, 2612, 3562);
        assert_eq!(stroke_room(Some(dab), w, h), room);
        assert_eq!(stroke_room(None, w, h), rect(0, 0, 0, 0), "blank: nothing to hold");
        assert_eq!(stroke_room(Some(rect(0, 0, 3000, 5000)), w, h), rect(0, 0, w, h), "most of it: all of it");
        // A new drawing: blank before the stroke, so its first dab is all
        // there is. Nothing is scanned — the cell is never even unpacked.
        let mut c = Canvas::new(w, h);
        c.dirty = Some(dab);
        assert_eq!(plan_upload(&c, None, Dirty::Full, Some(None)), TexPlan::Rebuild(room));
        // On a drawing, around it and the dab.
        let drawn = rect(100, 100, 300, 300);
        assert_eq!(
            plan_upload(&c, None, Dirty::Full, Some(Some(drawn))),
            TexPlan::Rebuild(stroke_room(Some(rect(100, 100, 2100, 3050)), w, h))
        );
        // Inside the room: a patch. Past it: room around the texture and
        // what reached past it.
        let inside = rect(1500, 2500, 1600, 2600);
        assert_eq!(plan_upload(&c, Some(room), Dirty::Rect(inside), Some(None)), TexPlan::Patch(inside));
        let past = rect(2600, 3500, 2700, 3700);
        assert_eq!(
            plan_upload(&c, Some(room), Dirty::Rect(past), Some(None)),
            TexPlan::Rebuild(stroke_room(Some(rect(1488, 2488, 2700, 3700)), w, h))
        );
    }


    #[test]
    fn a_cell_texture_holds_just_its_drawing() {
        let mut st = AppState::for_test();
        let ctx = egui::Context::default();
        let id = st.project.layers[0].resolve(0).unwrap();
        let _ = sync(&mut st, &ctx);
        assert!(st.layer_texture(0).is_some_and(|(_, t, r)| t.size() == [1, 1] && r == rect(0, 0, 0, 0)), "blank");

        ink(&mut st, id, rect(100, 50, 120, 60));
        let delta = sync(&mut st, &ctx);
        let (_, t, r) = st.layer_texture(0).unwrap();
        assert_eq!(r, rect(98, 48, 122, 62), "the drawing and its rim");
        assert_eq!(t.size(), [24, 14]);
        let tex = t.id();
        assert_eq!(uploaded_alpha(&delta, tex, 0, 0), 0, "the rim");
        assert_eq!(uploaded_alpha(&delta, tex, 2, 2), 255, "the drawing");

        ink(&mut st, id, rect(105, 52, 107, 54));
        let delta = sync(&mut st, &ctx);
        let (_, d) = delta.set.iter().find(|(t, _)| *t == tex).expect("patched");
        assert_eq!(d.pos, Some([7, 4]), "placed in texture, not cell, pixels");

        ink(&mut st, id, rect(300, 200, 310, 205));
        let _ = sync(&mut st, &ctx);
        assert_eq!(st.layer_texture(0).unwrap().2, rect(98, 48, 312, 207), "re-cropped around both");
    }

    /// On a cell this small the room a stroke's texture gets is most of it,
    /// so the texture takes all of it, once.
    #[test]
    fn a_stroke_past_the_drawing_takes_the_whole_cell_once() {
        let mut st = AppState::for_test();
        let ctx = egui::Context::default();
        let id = st.project.layers[0].resolve(0).unwrap();
        ink(&mut st, id, rect(100, 50, 120, 60));
        let _ = sync(&mut st, &ctx);
        st.dispatch(Action::ToolInk);
        st.pointer_down(st.make_sample(400.0, 300.0, 0.0));
        st.pointer_move(st.make_sample(450.0, 300.0, 0.1));
        let _ = sync(&mut st, &ctx);
        let (w, h) = (st.project.width, st.project.height);
        assert_eq!(st.layer_texture(0).unwrap().2, rect(0, 0, w, h));
        let tex = st.layer_texture(0).unwrap().1.id();
        st.pointer_move(st.make_sample(600.0, 500.0, 0.2));
        let delta = sync(&mut st, &ctx);
        let (_, d) = delta.set.iter().find(|(t, _)| *t == tex).expect("streamed");
        assert!(d.pos.is_some(), "the rest of the stroke is patches");
        st.pointer_up();
    }

    /// Resting on a drawing takes its stroke snapshot ahead of the pen, so
    /// pen-down has nothing to copy. On a frame where auto-key would start
    /// a new drawing it clears the snapshot instead. Passing through, or
    /// playing, it does nothing.
    #[test]
    fn resting_on_a_drawing_readies_its_stroke_snapshot() {
        let mut st = AppState::for_test();
        st.auto_key_draw = false;
        let ctx = egui::Context::default();
        let at = |st: &mut AppState, t: f64| {
            let raw = egui::RawInput {
                time: Some(t),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| st.warm_pre(ctx));
        };
        let id = st.project.layers[0].resolve(0).unwrap();
        ink(&mut st, id, rect(100, 50, 120, 60));
        Arc::make_mut(&mut st.project.cells[id]).pack();
        at(&mut st, 0.0);
        at(&mut st, 0.05);
        assert_eq!(st.pre_mirror, None, "not yet: it may only be passing");
        at(&mut st, 0.2);
        let c = st.project.cell(id).unwrap();
        assert_eq!(st.pre_mirror, Some((id, c.rev())));
        assert!(st.stroke_pre_pixels == *c.pixels(), "the snapshot is the drawing");
        assert!(!st.stroke_pre_live, "and no edit is open");

        // A stroke from it, undone, leaves the drawing as it was.
        let before = c.pixels().into_owned();
        st.dispatch(Action::ToolInk);
        st.pointer_down(st.make_sample(300.0, 300.0, 0.0));
        st.pointer_move(st.make_sample(400.0, 320.0, 0.1));
        st.pointer_up();
        st.undo();
        assert!(*st.project.cell(id).unwrap().pixels() == before[..]);

        // Held, under auto-key: the pen would start a blank drawing.
        st.auto_key_draw = true;
        st.project.add_frame();
        at(&mut st, 1.0);
        at(&mut st, 1.2);
        assert!(st.pre_inked.is_none() && st.stroke_pre_pixels.iter().all(|&b| b == 0), "cleared");

        // Playing: nothing, whatever the rest.
        st.auto_key_draw = false;
        st.project.goto(0);
        st.playback.playing = true;
        at(&mut st, 2.0);
        at(&mut st, 3.0);
        assert_eq!(st.pre_mirror, None);
    }

    /// With auto-key on, resting on a frame that holds a drawing builds that
    /// drawing's ghost: the next stroke makes it the drawing before, and its
    /// first frame then finds the ghost made.
    #[test]
    fn auto_key_ghosts_the_held_drawing_ahead_of_the_stroke() {
        let mut st = AppState::for_test();
        st.auto_key_draw = true;
        st.onion.enabled = true;
        let id = st.project.layers[0].resolve(0).unwrap();
        ink(&mut st, id, rect(10, 10, 20, 20));
        st.project.add_frame();
        let ctx = egui::Context::default();
        let at = |st: &mut AppState, t: f64| {
            let raw = egui::RawInput {
                time: Some(t),
                ..Default::default()
            };
            ctx.run(raw, |ctx| st.sync_textures(ctx)).textures_delta
        };
        let _ = at(&mut st, 0.0);
        assert!(!st.ghost_textures.contains_key(&id), "not while passing through");
        let _ = at(&mut st, 0.5);
        let tex = st.ghost_textures.get(&id).expect("built while resting").1.id();

        st.dispatch(Action::ToolInk);
        st.pointer_down(st.make_sample(300.0, 300.0, 0.0));
        assert_ne!(st.project.layers[0].resolve(1), Some(id), "a drawing of its own");
        let delta = at(&mut st, 0.6);
        assert!(st.ghost_textures.contains_key(&id));
        assert!(!delta.set.iter().any(|(t, _)| *t == tex), "the ghost wasn't built again");
        st.pointer_up();
    }

    /// Read straight from the cell, on threads or not, packed or not: the
    /// same image as premultiplying a copy of its pixels.
    #[test]
    fn a_cell_image_is_its_premultiplied_pixels() {
        let (w, h) = (1100u32, 1000u32);
        let mut c = Canvas::new(w, h);
        let px = c.pixels_mut();
        for y in 200..900usize {
            for x in 100..1000usize {
                let i = (y * w as usize + x) * 4;
                let v = (x * 7 + y * 13) as u8;
                px[i..i + 4].copy_from_slice(&[v, v.wrapping_mul(3), 255 - v, v.wrapping_add(x as u8)]);
            }
        }
        let rects = [rect(0, 0, w, h), rect(50, 150, 1050, 950), rect(300, 400, 310, 401), rect(5, 5, 5, 9)];
        for packed in [false, true] {
            if packed {
                c.pack();
            }
            for r in rects {
                let want = crop_image(r, || c.read_rect(r));
                let got = cell_image(&c, r);
                assert!(got.size == want.size && got.pixels == want.pixels, "{r:?}, packed: {packed}");
            }
        }
    }

    /// The GPU's copy of every texture, kept by replaying what egui was told
    /// to upload.
    #[derive(Default)]
    struct GpuCopy(HashMap<egui::TextureId, ColorImage>);

    impl GpuCopy {
        fn apply(&mut self, delta: &egui::TexturesDelta) -> usize {
            let mut sent = 0;
            for (id, d) in &delta.set {
                let egui::ImageData::Color(img) = &d.image else {
                    continue;
                };
                sent += img.pixels.len();
                match d.pos {
                    None => {
                        self.0.insert(*id, (**img).clone());
                    }
                    Some([x, y]) => {
                        let t = self.0.get_mut(id).expect("patch of a texture that exists");
                        for row in 0..img.size[1] {
                            let at = (y + row) * t.size[0] + x;
                            t.pixels[at..at + img.size[0]]
                                .copy_from_slice(&img.pixels[row * img.size[0]..(row + 1) * img.size[0]]);
                        }
                    }
                }
            }
            for id in &delta.free {
                self.0.remove(id);
            }
            sent
        }

        /// The texture layer `li` shows matches its cell, pixel for pixel.
        fn assert_shows(&self, st: &AppState, li: usize, what: &str) {
            let (id, tex, r) = st.layer_texture(li).expect("a texture");
            let img = &self.0[&tex.id()];
            let c = st.project.cell(id).unwrap();
            let want = crop_image(r, || c.read_rect(r));
            assert!(img.size == want.size && img.pixels == want.pixels, "{what}: the texture is the cell");
            let outside = c.content_bounds().map_or(true, |b| rect_within(b, r));
            assert!(outside, "{what}: every inked pixel is inside the texture");
        }
    }

    /// A stroke that spreads across a big cell: the texture grows around it
    /// a few times rather than taking the whole cell, stays exactly the
    /// cell the whole way, and pen-up sends only what hadn't gone up yet —
    /// for a Krita brush too, which lays no dab at pen-down.
    #[test]
    fn a_spreading_stroke_streams_and_pen_up_sends_only_the_rest() {
        use crate::tools::BrushMode;
        for mode in [BrushMode::Ribbon, BrushMode::Krita] {
            let mut st = AppState::for_test();
            st.load_project(Project::new(3000, 4000, 24.0), None);
            st.tool = ActiveTool::Pencil;
            st.brush.mode = mode;
            st.brush.radius = 6.0;
            let ctx = egui::Context::default();
            let mut gpu = GpuCopy::default();
            gpu.apply(&sync(&mut st, &ctx));
            st.pointer_down(st.make_sample(200.0, 300.0, 0.0));
            gpu.apply(&sync(&mut st, &ctx));
            let mut sizes = Vec::new();
            for i in 1..=120 {
                let t = i as f32 / 120.0;
                st.pointer_move(st.make_sample(200.0 + 2400.0 * t, 300.0 + 1500.0 * t, i as f32 * 0.01));
                gpu.apply(&sync(&mut st, &ctx));
                gpu.assert_shows(&st, 0, &format!("{mode:?} mid-stroke"));
                let size = st.layer_texture(0).unwrap().1.size();
                if sizes.last() != Some(&size) {
                    sizes.push(size);
                }
            }
            assert!(sizes.len() <= 6, "{mode:?}: re-made {} times: {sizes:?}", sizes.len());
            st.pointer_up();
            let sent = gpu.apply(&sync(&mut st, &ctx));
            gpu.assert_shows(&st, 0, &format!("{mode:?} after pen-up"));
            let r = st.project.cell(st.project.resolved_current().unwrap()).unwrap().dirty.unwrap();
            let bbox = ((r.max_x - r.min_x) * (r.max_y - r.min_y)) as usize;
            assert!(sent * 20 < bbox, "{mode:?}: pen-up sent {sent} px of a {bbox} px stroke");
        }
    }

    fn entry(id: CellId, used: u64, mb: usize) -> TexEntry {
        TexEntry {
            kind: TexKind::Cell,
            id,
            used,
            bytes: mb << 20,
        }
    }

    #[test]
    fn the_budget_drops_the_least_recently_shown_first() {
        let e = [entry(0, 5, 100), entry(1, 2, 100), entry(2, 9, 100), entry(3, 7, 100)];
        let out: Vec<CellId> = evictions(&e, 250 << 20, 9, false).iter().map(|e| e.id).collect();
        assert_eq!(out, vec![1, 0], "oldest first, and only until it fits");
        assert!(evictions(&e, 400 << 20, 9, false).is_empty(), "under budget: nothing");
    }

    #[test]
    fn playback_drops_the_most_recently_shown_first() {
        let e = [entry(0, 5, 100), entry(1, 2, 100), entry(2, 9, 100), entry(3, 7, 100)];
        let out: Vec<CellId> = evictions(&e, 250 << 20, 9, true).iter().map(|e| e.id).collect();
        assert_eq!(out, vec![3, 0], "the loop comes back to the oldest soonest");
    }

    #[test]
    fn the_budget_never_drops_what_is_on_screen() {
        let e = [entry(0, 9, 900), entry(1, 9, 900)];
        assert!(evictions(&e, 100 << 20, 9, false).is_empty());
        assert!(evictions(&e, 100 << 20, 9, true).is_empty());
    }

    #[test]
    fn ghosts_of_big_cells_are_small_and_keep_their_coverage() {
        assert_eq!(ghost_dims(11520, 2160), [1920, 360]);
        assert_eq!(ghost_dims(1920, 1080), [1920, 1080], "small cells stay 1:1");
        assert!(ghost_dims(20000, 16).iter().all(|&d| d <= GHOST_MAX_SIDE as usize));

        // A solid block stays solid; half coverage averages to half.
        let (w, h) = (4096u32, 8u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        for p in px.chunks_exact_mut(4) {
            p[3] = 255;
        }
        let g = ghost_image(&Canvas::from_pixels(w, h, px.clone()), [255, 0, 0]);
        assert_eq!(g.size, [2048, 4]);
        assert!(g.pixels.iter().all(|&c| c == Color32::from_rgb(255, 0, 0)));
        for (i, p) in px.chunks_exact_mut(4).enumerate() {
            p[3] = if i % 2 == 0 { 255 } else { 0 };
        }
        let g = ghost_image(&Canvas::from_pixels(w, h, px), [255, 0, 0]);
        assert!(g.pixels.iter().all(|c| (127..=128).contains(&c.a())));
    }

    /// A ghost is read from where the cell holds anything, so packed or not
    /// it's the same ghost, down to a drawing that starts partway through a
    /// ghost texel and edge texels that cover fewer pixels.
    #[test]
    fn a_packed_cell_makes_the_same_ghost() {
        let (w, h) = (4100u32, 2050u32);
        let mut c = Canvas::new(w, h);
        let px = c.pixels_mut();
        for y in 301..1777usize {
            for x in 1003..4100usize {
                px[(y * w as usize + x) * 4 + 3] = ((x * 3 + y) % 256) as u8;
            }
        }
        let full = ghost_image(&c, [10, 20, 30]);
        assert_eq!(full.size, [1367, 684]);
        c.pack();
        let packed = ghost_image(&c, [10, 20, 30]);
        assert!(packed.pixels == full.pixels);
        let one = |a: u8| ghost_image(&Canvas::from_pixels(1, 1, vec![0, 0, 0, a]), [0, 0, 255]).pixels[0];
        assert_eq!(one(255), Color32::from_rgb(0, 0, 255));
        assert_eq!(one(0), Color32::TRANSPARENT);
    }

    #[test]
    fn a_ghost_that_needs_no_shrinking_is_the_plain_silhouette() {
        let px = [9, 9, 9, 0, 9, 9, 9, 255, 9, 9, 9, 128, 9, 9, 9, 1];
        let g = ghost_image(&Canvas::from_pixels(4, 1, px.to_vec()), [0, 0, 255]);
        let alphas: Vec<u8> = g.pixels.iter().map(|c| c.a()).collect();
        assert_eq!(alphas, vec![0, 255, 128, 1]);
    }

    #[test]
    fn blank_means_no_coverage_anywhere() {
        let mut px = vec![0u8; 4 * 100_003];
        // Colour without coverage is still blank: an eraser can leave that.
        px[0] = 200;
        assert!(no_alpha(&px));
        let last = px.len() - 1;
        px[last] = 1;
        assert!(!no_alpha(&px), "the last pixel counts");
        assert!(!no_alpha(&px[4..]), "and on an unaligned slice too");
    }

    /// A state on the perspective tool with one square grid, 100 doc px a
    /// side, and a 1:1 view so handle tolerance is `HANDLE_PX` doc px.
    fn perspective_state() -> AppState {
        let mut state = AppState::for_test();
        state.dispatch(Action::ToolPerspective);
        state.view_scale = 1.0;
        let (w, h) = state.frame_size();
        let sq = [[100.0, 100.0], [200.0, 100.0], [200.0, 200.0], [100.0, 200.0]];
        state.perspective.grids[0].set_doc_corners(sq, &perspective::Space::flat(w, h));
        state
    }

    fn doc_corners(state: &AppState, i: usize) -> [[f32; 2]; 4] {
        let (w, h) = state.frame_size();
        state.perspective.grids[i].doc_corners(&perspective::Space::flat(w, h))
    }

    fn near(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-2 && (a[1] - b[1]).abs() < 1e-2
    }

    // --- Grid keys, follow links and the per-file store ---

    fn grid_centre_doc(state: &AppState, i: usize) -> [f32; 2] {
        let g = &state.perspective.grids[i];
        let s = state.grid_space(g, state.project.current_frame);
        perspective::centroid(&g.doc_corners(&s))
    }

    #[test]
    fn a_keyed_grid_tweens_as_the_frame_moves_and_undo_takes_a_key_back() {
        let mut state = perspective_state();
        state.project.ensure_frame_count(30);
        state.sync_grid_buffers();
        state.add_grid_key();
        let c0 = grid_centre_doc(&state, 0);
        state.project.goto(20);
        state.sync_grid_buffers();
        state.perspective.grids[0].translate([0.2, 0.0]);
        state.add_grid_key();
        state.project.goto(10);
        state.sync_grid_buffers();
        // Halfway, linear: half of 0.2 frame heights of 720.
        let c10 = grid_centre_doc(&state, 0);
        assert!((c10[0] - c0[0] - 72.0).abs() < 0.5, "{c10:?} vs {c0:?}");
        state.undo();
        assert_eq!(state.perspective.grids[0].keys.len(), 1);
        assert!(near(grid_centre_doc(&state, 0), c0), "back to the one key's pose");
        state.redo();
        assert_eq!(state.perspective.grids[0].keys.len(), 2);
    }

    #[test]
    fn dragging_a_keyed_grid_keys_the_frame_as_one_undo_step() {
        let mut state = perspective_state();
        state.project.ensure_frame_count(30);
        state.sync_grid_buffers();
        state.add_grid_key();
        state.project.goto(12);
        state.sync_grid_buffers();
        let steps = state.history.undo_len();
        state.perspective_down([150.0, 150.0]);
        state.perspective_move([160.0, 150.0]);
        state.perspective_move([170.0, 150.0]);
        state.pointer_up();
        assert!(state.perspective.grids[0].has_key(12), "auto-key");
        assert_eq!(state.history.undo_len(), steps + 1);
        // Away and back: it stuck, because it was keyed.
        state.project.goto(0);
        state.sync_grid_buffers();
        state.project.goto(12);
        state.sync_grid_buffers();
        assert!(near(doc_corners(&state, 0)[0], [120.0, 100.0]));
        state.undo();
        assert!(!state.perspective.grids[0].has_key(12));

        // Without auto-key a drag on a keyed grid lasts until the frame
        // changes, as the camera's does.
        state.perspective.auto_key = false;
        state.perspective_down([150.0, 150.0]);
        state.perspective_move([170.0, 150.0]);
        state.pointer_up();
        assert!(!state.perspective.grids[0].has_key(12));
        state.project.goto(13);
        state.sync_grid_buffers();
        assert!(near(doc_corners(&state, 0)[0], [100.0, 100.0]));
    }

    #[test]
    fn a_grid_without_keys_just_moves() {
        let mut state = perspective_state();
        state.perspective_down([150.0, 150.0]);
        state.perspective_move([170.0, 150.0]);
        state.pointer_up();
        assert!(state.perspective.grids[0].keys.is_empty());
        // Still one undo step for the drag.
        state.undo();
        assert!(near(doc_corners(&state, 0)[0], [100.0, 100.0]));
    }

    #[test]
    fn a_grid_on_the_camera_rides_it_and_switching_keeps_it_in_place() {
        let mut state = perspective_state();
        state.project.camera.tx = 50.0;
        let before = doc_corners(&state, 0);
        state.set_grid_follow(0, perspective::Follow::Camera);
        let at = |state: &AppState| {
            let g = &state.perspective.grids[0];
            g.doc_corners(&state.grid_space(g, state.project.current_frame))
        };
        assert!(near(at(&state)[0], before[0]), "switching doesn't jump it");
        state.project.camera.tx = 80.0;
        assert!(near(at(&state)[0], [before[0][0] + 30.0, before[0][1]]));
        // Zooming the camera in shrinks it in the document, so it keeps its
        // size in the shot.
        state.project.camera.zoom = 2.0;
        let c = at(&state);
        assert!((c[1][0] - c[0][0] - 50.0).abs() < 1e-2);
    }

    #[test]
    fn a_grid_on_a_layer_finds_it_again_by_name_next_session() {
        let mut state = perspective_state();
        let l = state.project.current_layer;
        let link = state.follow_layer(l).unwrap();
        state.set_grid_follow(0, link);
        state.project.layers[l].transform.tx = 40.0;
        let at = |state: &AppState| {
            let g = &state.perspective.grids[0];
            g.doc_corners(&state.grid_space(g, state.project.current_frame))[0]
        };
        assert!(near(at(&state), [140.0, 100.0]));
        // What the store gives back: the id is gone, the name is not.
        if let perspective::Follow::Layer { uid, .. } = &mut state.perspective.grids[0].follow {
            *uid = 0;
        }
        state.sync_grid_buffers();
        assert!(near(at(&state), [140.0, 100.0]));
    }

    #[test]
    fn each_file_keeps_its_own_grids() {
        let mut state = perspective_state();
        let a = PathBuf::from("C:/shots/a.anim");
        let b = PathBuf::from("C:/shots/b.anim");
        state.load_project(Project::new(1280, 720, 24.0), Some(a.clone()));
        state.perspective.grids[0].translate([0.1, 0.0]);
        let a_grids = state.perspective.grids.clone();
        state.load_project(Project::new(1280, 720, 24.0), Some(b.clone()));
        assert_ne!(state.perspective.grids, a_grids, "b has its own");
        state.perspective.grids[0].rows = 9;
        state.load_project(Project::new(1280, 720, 24.0), Some(a.clone()));
        assert_eq!(state.perspective.grids, a_grids);
        // Save As takes them along; the old file keeps its own.
        let c = PathBuf::from("C:/shots/c.anim");
        state.grids_saved_as(&c);
        state.project_path = Some(c.clone());
        state.perspective.grids[0].cols = 2;
        state.load_project(Project::new(1280, 720, 24.0), Some(a.clone()));
        assert_eq!(state.perspective.grids, a_grids);
        state.load_project(Project::new(1280, 720, 24.0), Some(c));
        assert_eq!(state.perspective.grids[0].cols, 2);
        state.load_project(Project::new(1280, 720, 24.0), Some(b));
        assert_eq!(state.perspective.grids[0].rows, 9);
        // A new project starts with one plain grid.
        state.reset_with(640, 480, 24.0);
        assert_eq!(state.perspective.grids.len(), 1);
        let fresh = perspective::PerspectiveGrid::default();
        assert_eq!(state.perspective.grids[0].corners, fresh.corners);
        // And the store survives a trip through the preferences.
        assert!(state.grid_files_for_prefs().len() >= 3);
    }

    #[test]
    fn a_walls_top_corner_drags_its_height_and_nothing_else_moves_it() {
        let mut state = perspective_state();
        let id = state.perspective.grids[0].id;
        state.perspective.grids[0] = perspective::PerspectiveGrid {
            id,
            ..Default::default()
        };
        let w = state.perspective.add_wall(0, 0).unwrap();
        let (fw, fh) = state.frame_size();
        let s = perspective::Space::flat(fw, fh);
        let wall = state.perspective.grids[w].doc_corners(&s);
        let h0 = state.perspective.grids[w].wall.unwrap().height;
        state.perspective_down(wall[0]);
        state.perspective_move([wall[0][0], wall[0][1] - 30.0]);
        state.pointer_up();
        let h1 = state.perspective.grids[w].wall.unwrap().height;
        assert!(h1 > h0, "{h1} > {h0}");
        let base = state.perspective.grids[w].doc_corners(&s);
        assert!(near(base[3], wall[3]) && near(base[2], wall[2]), "the base stays on the floor");
        // Inside it is selected, never moved.
        state.perspective_down(perspective::centroid(&base));
        assert!(state.grid_drag.is_none());
        assert_eq!(state.perspective.active, w);
        state.undo();
        assert_eq!(state.perspective.grids[w].wall.unwrap().height, h0);
    }

    #[test]
    fn dragging_a_grid_corner_moves_only_that_corner() {
        let mut state = perspective_state();
        let before = doc_corners(&state, 0);
        state.perspective_down([201.0, 99.0]);
        state.perspective_move([231.0, 89.0]);
        state.perspective_move([240.0, 80.0]);
        state.pointer_up();
        let after = doc_corners(&state, 0);
        assert!(near(after[1], [239.0, 81.0]), "{:?}", after[1]);
        for i in [0, 2, 3] {
            assert!(near(after[i], before[i]));
        }
        // Released: further moves do nothing.
        state.perspective_move([0.0, 0.0]);
        assert_eq!(doc_corners(&state, 0), after);
    }

    #[test]
    fn a_locked_grid_is_selected_but_not_moved() {
        let mut state = perspective_state();
        state.perspective.grids.push(state.perspective.grids[0].clone());
        state.perspective.grids[0].locked = true;
        state.perspective.active = 1;
        // Hide grid 1 so the press can only land on the locked one.
        state.perspective.grids[1].visible = false;
        let before = doc_corners(&state, 0);
        state.perspective_down([150.0, 150.0]);
        assert_eq!(state.perspective.active, 0);
        state.perspective_move([170.0, 170.0]);
        assert_eq!(doc_corners(&state, 0), before);
    }

    #[test]
    fn pressing_another_grid_makes_it_active() {
        let mut state = perspective_state();
        let mut other = state.perspective.grids[0].clone();
        let (w, h) = state.frame_size();
        let sq = [[400.0, 100.0], [500.0, 100.0], [500.0, 200.0], [400.0, 200.0]];
        other.set_doc_corners(sq, &perspective::Space::flat(w, h));
        state.perspective.grids.push(other);
        state.perspective_down([450.0, 150.0]);
        assert_eq!(state.perspective.active, 1);
        state.perspective_move([460.0, 150.0]);
        assert!(near(doc_corners(&state, 1)[0], [410.0, 100.0]));
        assert!(near(doc_corners(&state, 0)[0], [100.0, 100.0]), "grid 0 untouched");
        // Empty canvas grabs nothing and keeps the selection.
        state.pointer_up();
        state.perspective_down([900.0, 600.0]);
        assert_eq!(state.perspective.active, 1);
    }

    #[test]
    fn a_folding_corner_drag_is_refused() {
        let mut state = perspective_state();
        state.perspective_down([200.0, 100.0]);
        state.perspective_move([210.0, 110.0]);
        let ok = doc_corners(&state, 0);
        // Across the far diagonal: the quad would fold.
        state.perspective_move([50.0, 250.0]);
        assert_eq!(doc_corners(&state, 0), ok);
    }

    #[test]
    fn an_extra_vp_slides_along_the_horizon_until_pulled_off_and_sticks_back() {
        let mut state = perspective_state();
        let (w, h) = state.frame_size();
        // One-point floor: a level horizon through (150, 66.7).
        let trap = [[140.0, 100.0], [160.0, 100.0], [250.0, 400.0], [50.0, 400.0]];
        state.perspective.grids[0].set_doc_corners(trap, &perspective::Space::flat(w, h));
        assert!(state.perspective.grids[0].add_extra_vp(&perspective::Space::flat(w, h)));
        let s = perspective::Space::flat(w, h);
        let at = |state: &AppState| state.perspective.grids[0].extra_doc(&s)[0];
        let v0 = at(&state);
        let level = 200.0 / 3.0;
        assert!((v0[1] - level).abs() < 0.1, "{v0:?}");

        // Grab it and wander: it keeps to the horizon.
        state.perspective_down(v0);
        state.perspective_move([v0[0] + 30.0, v0[1] + 20.0]);
        assert!(near(at(&state), [v0[0] + 30.0, level]), "{:?}", at(&state));
        // Pulled well off, it comes free and follows the pointer.
        state.perspective_move([v0[0] + 30.0, v0[1] + 60.0]);
        assert!(near(at(&state), [v0[0] + 30.0, v0[1] + 60.0]));
        assert!(!state.perspective.grids[0].extra_vps[0].on_horizon);
        state.pointer_up();

        // Brought back near the horizon, it sticks again.
        let v1 = at(&state);
        state.perspective_down(v1);
        state.perspective_move([v1[0], level + 6.0]);
        assert!(near(at(&state), [v1[0], level]));
        assert!(state.perspective.grids[0].extra_vps[0].on_horizon);
    }

    #[test]
    fn moving_a_grid_takes_its_extra_vps_along() {
        let mut state = perspective_state();
        let (w, h) = state.frame_size();
        // Seen square-on the horizon is at infinity: the point goes above.
        assert!(state.perspective.grids[0].add_extra_vp(&perspective::Space::flat(w, h)));
        let before = state.perspective.grids[0].extra_doc(&perspective::Space::flat(w, h))[0];
        state.perspective_down([150.0, 150.0]);
        state.perspective_move([160.0, 170.0]);
        state.pointer_up();
        let after = state.perspective.grids[0].extra_doc(&perspective::Space::flat(w, h))[0];
        assert!(near(after, [before[0] + 10.0, before[1] + 20.0]), "{after:?}");
    }

    #[test]
    fn a_stroke_heading_for_an_extra_vp_locks_onto_it() {
        let mut state = perspective_state();
        let (w, h) = state.frame_size();
        state.perspective.grids[0]
            .extra_vps
            .push(crate::tools::perspective::ExtraVp {
                pos: perspective::from_doc([450.0, 450.0], w, h),
                on_horizon: false,
                rays: false,
                snap: true,
            });
        state.dispatch(Action::ToolPencil);
        state.perspective.show = true;
        state.perspective.snap = true;
        state.snap_begin([150.0, 150.0]);
        // Heading down-right, on the diagonal to (450, 450), not along the
        // square's rows or columns.
        assert!(near(state.snap_doc([170.0, 172.0]), [171.0, 171.0]));
    }

    #[test]
    fn snap_locks_a_stroke_to_one_grid_direction() {
        let mut state = perspective_state();
        state.dispatch(Action::ToolPencil);
        state.perspective.show = true;
        state.perspective.snap = true;
        // A square seen square-on: rows horizontal, columns vertical.
        assert_eq!(state.snap_begin([150.0, 150.0]), [150.0, 150.0]);
        // Too close to tell yet: held on the start.
        assert_eq!(state.snap_doc([152.0, 151.0]), [150.0, 150.0]);
        // Heading right: locked horizontal...
        assert!(near(state.snap_doc([170.0, 153.0]), [170.0, 150.0]));
        // ...and it stays locked even when the hand drifts steeply.
        assert!(near(state.snap_doc([175.0, 190.0]), [175.0, 150.0]));
        state.pointer_up();
        // Snap off: points pass straight through.
        state.perspective.snap = false;
        state.snap_begin([150.0, 150.0]);
        assert_eq!(state.snap_doc([170.0, 153.0]), [170.0, 153.0]);
    }

    #[test]
    fn a_near_vertical_stroke_follows_the_vanishing_point_not_the_vertical() {
        let mut state = perspective_state();
        let (w, h) = state.frame_size();
        // One-point floor: the columns meet at about (150, 67).
        let trap = [[140.0, 100.0], [160.0, 100.0], [250.0, 400.0], [50.0, 400.0]];
        state.perspective.grids[0].set_doc_corners(trap, &perspective::Space::flat(w, h));
        state.dispatch(Action::ToolPencil);
        state.perspective.show = true;
        state.perspective.snap = true;
        // Straight up from just right of centre: nearly vertical, but the
        // column through here leans toward the vanishing point.
        state.snap_begin([170.0, 400.0]);
        let p = state.snap_doc([170.0, 300.0]);
        assert!(p[0] < 170.0 - 1.0, "leans toward the VP: {p:?}");
        state.pointer_up();
        // With verticals on, the same stroke stays vertical.
        state.perspective.snap_vertical = true;
        state.snap_begin([170.0, 400.0]);
        let p = state.snap_doc([170.0, 300.0]);
        assert!((p[0] - 170.0).abs() < 1e-3, "{p:?}");
    }

    /// One-point floor whose columns meet at (150, 66.7).
    const FLOOR: [[f32; 2]; 4] = [[140.0, 100.0], [160.0, 100.0], [250.0, 400.0], [50.0, 400.0]];

    /// The Shape tool drawing `kind` with snap on, over the `FLOOR` grid.
    fn shape_state(kind: ShapeKind) -> AppState {
        let mut state = perspective_state();
        let (w, h) = state.frame_size();
        state.perspective.grids[0].set_doc_corners(FLOOR, &perspective::Space::flat(w, h));
        state.dispatch(Action::ToolShape);
        state.brush.shape_kind = kind;
        state.brush.radius = 3.0;
        state.perspective.show = true;
        state.perspective.snap = true;
        state
    }

    /// Press at cell point `a`, drag to `b`, release.
    fn drag_shape(state: &mut AppState, a: (f32, f32), b: (f32, f32)) {
        state.pointer_down(state.make_sample(a.0, a.1, 0.0));
        state.pointer_move(state.make_sample(b.0, b.1, 0.1));
        state.pointer_up();
    }

    fn alpha(state: &AppState, p: (f32, f32)) -> u8 {
        let id = state.project.resolved_current().unwrap();
        let c = state.project.cell(id).unwrap();
        let (x, y) = (p.0.round() as u32, p.1.round() as u32);
        c.pixels()[((y * c.width + x) * 4 + 3) as usize]
    }

    #[test]
    fn a_rect_lies_on_the_grid_when_snap_is_on() {
        let mut state = shape_state(ShapeKind::Rect);
        drag_shape(&mut state, (120.0, 380.0), (180.0, 200.0));
        // The near edge runs out to the column through the cursor, which
        // leans out to x ≈ 220 down here...
        assert!(alpha(&state, (210.0, 380.0)) > 200, "near edge, past the flat rect");
        assert!(alpha(&state, (200.0, 290.0)) > 200, "right side, leaning to the VP");
        // ...and the flat rect's far-left corner is left bare.
        assert_eq!(alpha(&state, (120.0, 200.0)), 0);
    }

    #[test]
    fn with_snap_off_a_rect_stays_flat() {
        let mut state = shape_state(ShapeKind::Rect);
        state.perspective.snap = false;
        drag_shape(&mut state, (120.0, 380.0), (180.0, 200.0));
        assert!(alpha(&state, (120.0, 200.0)) > 200);
        assert_eq!(alpha(&state, (210.0, 380.0)), 0);
    }

    #[test]
    fn a_rect_lies_on_the_grid_on_a_transformed_layer_too() {
        let mut state = shape_state(ShapeKind::Rect);
        let li = state.project.current_layer;
        let t = &mut state.project.layers[li].transform;
        (t.scale, t.rot, t.tx) = (2.0, 0.3, 25.0);
        let t = *t;
        let (pw, ph) = state.frame_size();
        let (cw, ch) = state.project.draw_cell_size(li, state.project.current_frame);
        let cell = |p: (f32, f32)| t.doc_to_cell(p.0, p.1, cw as f32, ch as f32, pw, ph);
        // The same drag as above, as the pointer would feed it on this layer.
        drag_shape(&mut state, cell((120.0, 380.0)), cell((180.0, 200.0)));
        // Where it lands on screen matches the untransformed case.
        assert!(alpha(&state, cell((210.0, 380.0))) > 0, "near edge");
        assert!(alpha(&state, cell((200.0, 290.0))) > 0, "right side");
        assert_eq!(alpha(&state, cell((120.0, 200.0))), 0);
    }

    #[test]
    fn shift_squares_a_rect_up_in_grid_cells() {
        let mut state = shape_state(ShapeKind::Rect);
        state.pointer_down(state.make_sample(120.0, 380.0, 0.0));
        state.pointer_move(state.make_sample(180.0, 200.0, 0.1));
        state.shift_held = true;
        let drag = state.shape_drag.unwrap();
        let plane = drag.plane.expect("snap on: the drag lies on the grid");
        let o = state.shape_outline(&drag);
        let [u0, v0] = plane.h.unmap([o[0].0, o[0].1]).unwrap();
        let [u1, v1] = plane.h.unmap([o[2].0, o[2].1]).unwrap();
        let (cols, rows) = (plane.cols as f32, plane.rows as f32);
        assert!(((u1 - u0).abs() * cols - (v1 - v0).abs() * rows).abs() < 1e-3);
        state.shift_held = false;
        let o = state.shape_outline(&drag);
        let [u1b, v1b] = plane.h.unmap([o[2].0, o[2].1]).unwrap();
        assert!(((u1b - u0).abs() * cols - (v1b - v0).abs() * rows).abs() > 0.1, "free again");
    }

    #[test]
    fn a_shape_line_snaps_like_a_stroke() {
        let mut state = shape_state(ShapeKind::Line);
        let (w, h) = state.frame_size();
        let sq = [[100.0, 100.0], [200.0, 100.0], [200.0, 200.0], [100.0, 200.0]];
        state.perspective.grids[0].set_doc_corners(sq, &perspective::Space::flat(w, h));
        // What the canvas does: snap in document space, then feed the cell.
        let a = state.snap_begin([150.0, 150.0]);
        state.pointer_down(state.make_sample(a[0], a[1], 0.0));
        let b = state.snap_doc([190.0, 165.0]);
        assert!(near(b, [190.0, 150.0]), "locked to the row: {b:?}");
        state.pointer_move(state.make_sample(b[0], b[1], 0.1));
        assert!(state.shape_drag.unwrap().plane.is_none(), "lines don't lie on the plane");
        state.pointer_up();
        assert!(alpha(&state, (185.0, 150.0)) > 200);
        assert_eq!(alpha(&state, (185.0, 163.0)), 0);
        // Rectangles lie on the plane rather than locking to a direction.
        state.brush.shape_kind = ShapeKind::Rect;
        state.snap_begin([150.0, 150.0]);
        assert_eq!(state.snap_doc([190.0, 165.0]), [190.0, 165.0]);
    }

    #[test]
    fn next_grid_cycles_the_visible_grids_and_wraps() {
        let mut state = AppState::for_test();
        let g = state.perspective.grids[0].clone();
        state.perspective.grids = vec![g.clone(), g.clone(), g.clone(), g];
        state.perspective.grids[2].visible = false;
        state.perspective.active = 0;
        let mut seen = Vec::new();
        for _ in 0..4 {
            state.dispatch(Action::PerspectiveNextGrid);
            seen.push(state.perspective.active);
        }
        assert_eq!(seen, [1, 3, 0, 1], "hidden grid 2 skipped, wraps to 0");
        // With every other grid hidden, it stays where it is.
        for i in [0, 2, 3] {
            state.perspective.grids[i].visible = false;
        }
        state.dispatch(Action::PerspectiveNextGrid);
        assert_eq!(state.perspective.active, 1);
        state.perspective.grids.clear();
        state.dispatch(Action::PerspectiveNextGrid);
    }

    #[test]
    fn selecting_the_tool_brings_back_a_grid() {
        let mut state = AppState::for_test();
        state.perspective.grids.clear();
        state.dispatch(Action::ToolPerspective);
        assert_eq!(state.perspective.grids.len(), 1);
        assert_eq!(state.perspective.active, 0);
    }

    /// `eframe::Storage` in memory, so prefs go through the same RON
    /// serialisation the app's `ui.ron` does.
    #[derive(Default)]
    pub(super) struct MemStorage(HashMap<String, String>);

    impl eframe::Storage for MemStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn flush(&mut self) {}
    }

    const PREFS_FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/testdata/ui_prefs_v1.ron");

    /// The brushes in the prefs fixture: every slot tuned away from its
    /// default, the Krita Pencil-5 among them.
    fn fixture_brushes() -> Vec<BrushSettings> {
        let mut v = default_tool_brushes().to_vec();
        v[0] = BrushSettings::pencil5_krita();
        v[0].radius = 21.0;
        v[0].color = [10, 20, 30, 255];
        v[1].radius = 7.5;
        v[1].cap = crate::tools::StrokeCap::Flat;
        v[2].mode = crate::tools::BrushMode::Dab;
        v[2].opacity = 0.5;
        v[3].fill_gap = 4;
        v[4].shape_kind = ShapeKind::Ellipse;
        v
    }

    fn fixture_prefs() -> UiPrefs {
        UiPrefs {
            show_panels: false,
            frame_step: 3,
            smoothing: SmoothingOptions {
                kind: crate::tools::Smoothing::Weighted,
                distance_max: 33.0,
                ..SmoothingOptions::default()
            },
            tool_brushes: Some(fixture_brushes()),
            palette: vec![[1, 2, 3], [250, 128, 0]],
            krita_path: Some(PathBuf::from("C:/Krita (x64)/bin/krita.exe")),
            bg_color: [0.1, 0.2, 0.3],
            ..UiPrefs::default()
        }
    }

    /// Writes the prefs fixture. Run once, by the build whose prefs format
    /// the fixture pins (it was, before the Krita brush library existed):
    ///
    /// ```text
    /// cargo test write_prefs_fixture -- --ignored
    /// ```
    #[test]
    #[ignore]
    fn write_prefs_fixture() {
        let mut storage = MemStorage::default();
        eframe::set_value(&mut storage, UI_PREFS_KEY, &fixture_prefs());
        std::fs::write(PREFS_FIXTURE, &storage.0[UI_PREFS_KEY]).unwrap();
    }

    /// A `ui.ron` written before the Krita brush library still loads, every
    /// brush as it was. One field this build can't read would make
    /// `get_value` fail and silently reset *every* preference.
    #[test]
    fn prefs_written_before_the_brush_library_still_load() {
        let mut storage = MemStorage::default();
        let text = std::fs::read_to_string(PREFS_FIXTURE).expect("fixture");
        eframe::Storage::set_string(&mut storage, UI_PREFS_KEY, text);
        let prefs: UiPrefs = eframe::get_value(&storage, UI_PREFS_KEY).expect("prefs reset");
        let want = fixture_prefs();
        assert!(!prefs.show_panels);
        assert_eq!(prefs.frame_step, 3);
        assert_eq!(format!("{:?}", prefs.smoothing), format!("{:?}", want.smoothing));
        assert_eq!(prefs.palette, want.palette);
        assert_eq!(prefs.krita_path, want.krita_path);
        assert_eq!(prefs.bg_color, want.bg_color);
        assert!(prefs.hidden_panels.is_empty() && prefs.panel_layout_gen == 0, "every panel where it was");
        assert_eq!(prefs.color_scheme, Scheme::Graphite);
        // The onion sliders it saved (Prev 1, Next 1, Max α 0.75) become the
        // strip: one lit each way, at 75%.
        assert_eq!((prefs.onion.prev_shown, prefs.onion.next_shown), (0b1, 0b1));
        assert_eq!(prefs.onion.alpha_for(1, OnionDirection::Next), 191);
        let brushes = restore_tool_brushes(prefs.tool_brushes);
        assert_eq!(format!("{:?}", brushes.to_vec()), format!("{:?}", fixture_brushes()));
        assert_eq!(brushes[0].mode, crate::tools::BrushMode::Krita);
        let fill = &brushes[ActiveTool::Fill.idx()];
        assert_eq!(
            (fill.fill_grain, fill.fill_grain_size, fill.fill_grain_contrast, fill.fill_grain_boil),
            (0.0, 1.0, 1.0, false),
            "fills saved before grain stay flat"
        );
    }

    /// The active tool's brush is saved as it is now, not as it was at the
    /// last tool switch.
    #[test]
    fn the_active_brush_is_saved_with_its_latest_changes() {
        use eframe::App as _;
        let mut st = AppState::for_test();
        st.brush = BrushSettings::pencil5_krita();
        st.brush.radius = 33.0;
        let mut storage = MemStorage::default();
        st.save(&mut storage);
        let prefs: UiPrefs = eframe::get_value(&storage, UI_PREFS_KEY).unwrap();
        let pencil = &restore_tool_brushes(prefs.tool_brushes)[ActiveTool::Pencil.idx()];
        assert_eq!(pencil.mode, crate::tools::BrushMode::Krita);
        assert_eq!(pencil.radius, 33.0);
    }

    #[test]
    fn hidden_panels_and_the_layout_reset_outlast_a_restart() {
        use eframe::App as _;
        let mut st = AppState::for_test();
        st.set_panel_shown(PanelId::Brush, false);
        st.reset_panel_layout();
        assert!(st.panel_shown(PanelId::Brush), "a reset shows every panel");
        st.set_panel_shown(PanelId::Onion, false);
        st.set_panel_shown(PanelId::Xsheet, false);
        st.set_panel_shown(PanelId::Xsheet, true);
        let mut storage = MemStorage::default();
        st.save(&mut storage);
        let prefs: UiPrefs = eframe::get_value(&storage, UI_PREFS_KEY).unwrap();
        let back = AppState::with_prefs(prefs, 8192, None);
        assert_eq!(back.hidden_panels, vec![PanelId::Onion]);
        assert_eq!(back.panel_layout_gen, 1);
    }

    #[test]
    fn the_color_scheme_and_custom_colors_outlast_a_restart() {
        use eframe::App as _;
        let mut st = AppState::for_test();
        assert_eq!(st.color_scheme, Scheme::Graphite, "what the app always looked like");
        st.color_scheme = Scheme::Custom;
        st.custom_colors.accent = [250, 120, 60];
        let mut storage = MemStorage::default();
        st.save(&mut storage);
        let prefs: UiPrefs = eframe::get_value(&storage, UI_PREFS_KEY).unwrap();
        let back = AppState::with_prefs(prefs, 8192, None);
        assert_eq!(back.color_scheme, Scheme::Custom);
        assert_eq!(back.custom_colors, st.custom_colors);
    }

    /// Picking a colour off the screen changes the colour and nothing else.
    #[test]
    fn a_screen_pick_keeps_the_brush() {
        let mut st = AppState::for_test();
        st.brush = BrushSettings::pencil5_krita();
        st.brush.radius = 33.0;
        st.begin_screen_pick();
        st.commit_screen_pick([1, 2, 3, 255]);
        assert_eq!(st.brush.mode, crate::tools::BrushMode::Krita);
        assert_eq!(st.brush.radius, 33.0);
        assert_eq!(st.brush.color, [1, 2, 3, 255]);
    }

    /// A new project keeps every tool's brush and comes back on the pencil.
    #[test]
    fn a_new_project_keeps_the_brushes() {
        let mut st = AppState::for_test();
        st.brush.radius = 33.0;
        st.set_tool(ActiveTool::Ink);
        st.brush.radius = 9.0;
        st.reset_with(640, 480, 24.0);
        assert_eq!(st.tool, ActiveTool::Pencil);
        assert_eq!(st.brush.radius, 33.0);
        assert_eq!(st.tool_brushes[ActiveTool::Ink.idx()].radius, 9.0);
    }

    /// Shape draws at the size of the pen used last, and its Thickness goes
    /// back to that pen. The two pens keep their own sizes.
    #[test]
    fn shape_shares_its_size_with_the_last_pen() {
        let mut st = AppState::for_test();
        st.brush.radius = 12.0;
        st.set_tool(ActiveTool::Ink);
        st.brush.radius = 4.0;
        st.set_tool(ActiveTool::Shape);
        assert_eq!(st.brush.radius, 4.0, "from the ink");
        st.brush.radius = 7.0;
        st.set_tool(ActiveTool::Fill);
        st.set_tool(ActiveTool::Shape);
        assert_eq!(st.brush.radius, 7.0, "kept through another tool");
        st.set_tool(ActiveTool::Ink);
        assert_eq!(st.brush.radius, 7.0, "Thickness went back to the ink");
        st.set_tool(ActiveTool::Pencil);
        assert_eq!(st.brush.radius, 12.0, "the pencil kept its own");
        st.set_tool(ActiveTool::Shape);
        assert_eq!(st.brush.radius, 12.0, "now from the pencil");
        // Only the size is shared: Shape keeps its own hard edge.
        assert_eq!(st.brush.hardness, BrushSettings::default_shape().hardness);
    }

    /// Brush size keys keep Shape and the pen in step, with the panels up
    /// and the keys going through `handle_keys` the way `update` runs them.
    #[test]
    fn size_keys_keep_shape_and_the_pen_in_step() {
        let mut st = AppState::for_test();
        st.shortcuts = ShortcutMap::default();
        st.show_panels = true;
        let ctx = egui::Context::default();
        let frame = |st: &mut AppState, key: Option<egui::Key>| {
            let events = key
                .map(|key| egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                })
                .into_iter()
                .collect();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 1000.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                let tab_focus = st.handle_keys(ctx);
                ui::shell::draw(st, ctx);
                st.drop_stray_focus(ctx, tab_focus);
            });
        };
        frame(&mut st, None);
        frame(&mut st, Some(egui::Key::W));
        assert_eq!(st.tool, ActiveTool::Ink);
        let start = st.brush.radius;
        for _ in 0..3 {
            frame(&mut st, Some(egui::Key::CloseBracket));
        }
        let ink = st.brush.radius;
        assert!(ink > start, "{start} -> {ink}");
        frame(&mut st, Some(egui::Key::G));
        assert_eq!(st.tool, ActiveTool::Shape);
        assert_eq!(st.brush.radius, ink, "Shape took the ink's new size");
        frame(&mut st, Some(egui::Key::OpenBracket));
        let shape = st.brush.radius;
        assert!(shape < ink);
        frame(&mut st, Some(egui::Key::W));
        assert_eq!(st.brush.radius, shape, "the ink took Shape's new size");
    }

    /// A Thickness set on Shape outlasts a restart, on the pen it follows.
    #[test]
    fn a_shape_thickness_is_saved_with_its_pen() {
        use eframe::App as _;
        let mut st = AppState::for_test();
        st.set_tool(ActiveTool::Shape);
        st.brush.radius = 21.0;
        let mut storage = MemStorage::default();
        st.save(&mut storage);
        let prefs: UiPrefs = eframe::get_value(&storage, UI_PREFS_KEY).unwrap();
        let mut back = AppState::with_prefs(prefs, 8192, None);
        assert_eq!(back.brush.radius, 21.0, "back on the pencil at Shape's size");
        back.set_tool(ActiveTool::Shape);
        assert_eq!(back.brush.radius, 21.0);
    }

    /// A colour picked off the screen while on Shape leaves its size alone.
    #[test]
    fn a_screen_pick_on_shape_keeps_the_pen_size() {
        let mut st = AppState::for_test();
        st.set_tool(ActiveTool::Ink);
        st.brush.radius = 5.0;
        st.set_tool(ActiveTool::Shape);
        st.brush.radius = 9.0;
        st.begin_screen_pick();
        st.commit_screen_pick([1, 2, 3, 255]);
        assert_eq!(st.tool, ActiveTool::Shape);
        assert_eq!(st.brush.radius, 9.0);
        assert_eq!(st.tool_brushes[ActiveTool::Ink.idx()].radius, 9.0);
    }

    /// A stabilized stroke through the app: pen-up finishes the line even
    /// when no frame came between the pen's events to tick it.
    #[test]
    fn a_stabilized_stroke_finishes_at_pen_up() {
        let mut st = AppState::for_test();
        st.dispatch(Action::ToolPencil);
        st.smoothing = SmoothingOptions {
            kind: crate::tools::Smoothing::Stabilizer,
            use_delay: false,
            ..SmoothingOptions::default()
        };
        st.pointer_down(st.make_sample(100.0, 100.0, 0.0));
        for i in 1..=40 {
            st.pointer_move(st.make_sample(100.0 + i as f32 * 4.0, 100.0, i as f32 * 0.004));
        }
        assert!(st.stroke.as_ref().unwrap().stabilized());
        st.pointer_up();
        let cell = st.project.layers[st.project.current_layer]
            .resolve(st.project.current_frame)
            .and_then(|id| st.project.cell(id))
            .expect("the stroke made a drawing");
        let alpha = |x: u32, y: u32| cell.pixels()[((y * cell.width + x) * 4 + 3) as usize];
        assert!(alpha(250, 100) > 0, "the line runs on to where the pen lifted");
        assert_eq!(alpha(275, 100), 0);
    }

    #[test]
    fn brushes_saved_before_the_perspective_tool_survive() {
        let mut old: Vec<BrushSettings> = default_tool_brushes()[..7].to_vec();
        old[0].radius = 42.0;
        let restored = restore_tool_brushes(Some(old));
        assert_eq!(restored[0].radius, 42.0, "tuned pencil kept");
        // Any other length is still thrown out.
        let odd = default_tool_brushes()[..5].to_vec();
        let mut short = odd;
        short[0].radius = 42.0;
        assert_ne!(restore_tool_brushes(Some(short))[0].radius, 42.0);
    }

    #[test]
    fn perspective_prefs_round_trip_and_default_when_missing() {
        let mut prefs = UiPrefs::default();
        prefs.perspective.show = true;
        prefs.perspective.grids[0].rows = 11;
        prefs.perspective.grids[0].locked = true;
        let text = toml::to_string(&prefs).unwrap();
        let back: UiPrefs = toml::from_str(&text).unwrap();
        assert_eq!(back.perspective, prefs.perspective);
        // A blob from before grids existed still loads, with the default grid.
        let old: UiPrefs = toml::from_str("show_panels = false").unwrap();
        assert!(!old.show_panels);
        assert_eq!(old.perspective, PerspectiveConfig::default());
    }

    #[test]
    fn the_backdrop_is_kept_between_runs_and_across_new_projects() {
        let prefs = UiPrefs {
            bg_color: [0.9, 0.85, 0.7],
            bg_opacity: 0.5,
            ..UiPrefs::default()
        };
        let back: UiPrefs = toml::from_str(&toml::to_string(&prefs).unwrap()).unwrap();
        let mut st = AppState::with_prefs(back, 8192, None);
        assert_eq!((st.bg_color, st.bg_opacity), ([0.9, 0.85, 0.7], 0.5));
        // File > New starts a fresh project, not a fresh workspace.
        st.reset();
        assert_eq!((st.bg_color, st.bg_opacity), ([0.9, 0.85, 0.7], 0.5));
        // Prefs saved before the backdrop was kept load with the old default.
        let old: UiPrefs = toml::from_str("show_panels = false").unwrap();
        assert_eq!((old.bg_color, old.bg_opacity), (DEFAULT_BG_COLOR, 1.0));
        // A polygon selection shape from before the lasso took taps opens
        // as the lasso.
        let poly: UiPrefs = toml::from_str("sel_shape = \"Polygon\"").unwrap();
        assert_eq!(poly.sel_shape, SelShape::Freehand);
        // And ones from while there was a checker still load, without it.
        let checker: UiPrefs = toml::from_str("show_checker = true\nbg_opacity = 0.5").unwrap();
        assert_eq!(checker.bg_opacity, 0.5);
    }

    #[test]
    fn the_onion_strip_round_trips_and_old_ranges_carry_over() {
        let mut prefs = UiPrefs::default();
        prefs.onion.toggle_shown(1, OnionDirection::Prev);
        prefs.onion.set_opacity(4, OnionDirection::Next, 0.33);
        let back: UiPrefs = toml::from_str(&toml::to_string(&prefs).unwrap()).unwrap();
        assert!(!back.onion.is_shown(1, OnionDirection::Prev));
        assert_eq!(back.onion.opacity(4, OnionDirection::Next), 0.33);
        // Onion prefs saved before the hide toggles existed: Prev 3 lights
        // the three back, and Next was 1 then.
        let old: UiPrefs = toml::from_str("[onion]\nprev = 3\n").unwrap();
        assert_eq!((old.onion.prev_shown, old.onion.next_shown), (0b111, 0b1));
    }

    #[test]
    fn the_magic_select_settings_round_trip_and_old_prefs_get_defaults() {
        let mut prefs = UiPrefs::default();
        prefs.sel_shape = SelShape::Wand;
        prefs.wand = WandOptions {
            tolerance: 40,
            gap: 6,
            expand: 2,
            any_paint: true,
            contiguous: false,
            all_visible: true,
        };
        let back: UiPrefs = toml::from_str(&toml::to_string(&prefs).unwrap()).unwrap();
        let st = AppState::with_prefs(back, 8192, None);
        assert_eq!(st.sel_shape, SelShape::Wand);
        assert_eq!(st.wand, prefs.wand);
        // Prefs from before the wand, or with only some of its settings.
        let old: UiPrefs = toml::from_str("show_panels = false").unwrap();
        assert_eq!(old.wand, WandOptions::default());
        let part: UiPrefs = toml::from_str("[wand]\ngap = 3\n").unwrap();
        assert_eq!(part.wand, WandOptions { gap: 3, ..WandOptions::default() });
        // And through the app's own save, as a drag left them.
        use eframe::App as _;
        let mut st = AppState::for_test();
        st.wand.gap = 9;
        let mut storage = MemStorage::default();
        st.save(&mut storage);
        let saved: UiPrefs = eframe::get_value(&storage, UI_PREFS_KEY).unwrap();
        assert_eq!(saved.wand.gap, 9);
    }

    /// The active layer and those with their onion on ghost — of those
    /// drawn, and not while playing or with the skin off.
    #[test]
    fn onion_layers_are_the_active_one_and_those_switched_on() {
        let mut state = AppState::for_test();
        while state.project.layers.len() < 4 {
            state.project.add_layer();
        }
        state.project.current_layer = 1;
        assert!(state.onion_layers().is_empty(), "skin off");
        state.onion.enabled = true;
        assert_eq!(state.onion_layers(), vec![1]);
        state.project.layers[3].onion = true;
        state.project.layers[0].onion = true;
        assert_eq!(state.onion_layers(), vec![0, 1, 3]);
        state.project.layers[3].visible = false;
        assert_eq!(state.onion_layers(), vec![0, 1], "a hidden layer has nothing to compare");
        state.playback.playing = true;
        assert!(state.onion_layers().is_empty(), "playing");
    }

    /// Put `w`×`h` @ `fps` in the New project dialog and save it as `name`.
    fn save_as(st: &mut AppState, name: &str, w: u32, h: u32, fps: f32) -> bool {
        let cfg = &mut st.new_project_cfg;
        (cfg.width, cfg.height, cfg.fps) = (w, h, fps);
        cfg.preset_name = name.to_string();
        st.save_preset()
    }

    #[test]
    fn saving_a_preset_needs_a_name_and_replaces_one_it_matches() {
        let mut st = AppState::for_test();
        assert!(!save_as(&mut st, "   ", 1080, 1920, 12.0));
        assert!(st.project_presets.is_empty());

        assert!(save_as(&mut st, " Phone ", 1080, 1920, 12.0));
        assert!(save_as(&mut st, "Square", 1080, 1080, 24.0));
        assert_eq!(st.project_presets[0].name, "Phone", "trimmed");
        assert!(st.new_project_cfg.preset_name.is_empty(), "name box cleared");

        // Same name, any case: overwritten in place, star kept.
        st.toggle_default_preset(0);
        assert!(save_as(&mut st, "PHONE", 720, 1280, 30.0));
        assert_eq!(st.project_presets.len(), 2);
        let p = &st.project_presets[0];
        assert_eq!((p.name.as_str(), p.width, p.height, p.fps), ("PHONE", 720, 1280, 30.0));
        assert!(p.is_default);
    }

    #[test]
    fn renaming_a_preset_rejects_blank_and_taken_names() {
        let mut st = AppState::for_test();
        save_as(&mut st, "Phone", 1080, 1920, 12.0);
        save_as(&mut st, "Square", 1080, 1080, 24.0);
        assert!(!st.rename_preset(0, "  "));
        assert!(!st.rename_preset(0, "square"));
        assert!(st.rename_preset(0, "phone"), "own name, new case");
        assert!(st.rename_preset(1, " Insta "));
        let names: Vec<_> = st.project_presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["phone", "Insta"]);
    }

    #[test]
    fn only_one_preset_is_starred() {
        let mut st = AppState::for_test();
        save_as(&mut st, "A", 100, 100, 24.0);
        save_as(&mut st, "B", 200, 200, 24.0);
        let starred = |st: &AppState| -> Vec<bool> {
            st.project_presets.iter().map(|p| p.is_default).collect()
        };
        st.toggle_default_preset(0);
        st.toggle_default_preset(1);
        assert_eq!(starred(&st), [false, true]);
        st.toggle_default_preset(1);
        assert_eq!(starred(&st), [false, false], "starring the starred one clears it");
        st.toggle_default_preset(0);
        st.delete_preset(0);
        assert_eq!(starred(&st), [false]);
    }

    #[test]
    fn new_project_opens_on_the_starred_preset_as_saved() {
        let mut st = AppState::for_test();
        save_as(&mut st, "Phone", 1080, 1920, 12.0);
        // Nothing starred: the current project's size, as before presets.
        st.open_new_project();
        let cfg = &st.new_project_cfg;
        assert_eq!((cfg.width, cfg.height, cfg.fps), (st.project.width, st.project.height, st.project.fps));
        assert!(st.show_new_project);

        st.toggle_default_preset(0);
        st.open_new_project();
        let cfg = &st.new_project_cfg;
        assert_eq!((cfg.width, cfg.height, cfg.fps), (1080, 1920, 12.0), "portrait stays portrait");

        // Applying from a landscape dialog keeps the preset's orientation too.
        st.new_project_cfg.width = 3840;
        st.new_project_cfg.height = 2160;
        st.apply_preset(0);
        assert_eq!((st.new_project_cfg.width, st.new_project_cfg.height), (1080, 1920));
    }

    #[test]
    fn presets_round_trip_and_default_to_none_when_missing() {
        let prefs = UiPrefs {
            project_presets: vec![ProjectPreset {
                name: "Phone".into(),
                width: 1080,
                height: 1920,
                fps: 12.0,
                is_default: true,
            }],
            ..Default::default()
        };
        let back: UiPrefs = toml::from_str(&toml::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(back.project_presets, prefs.project_presets);
        // Prefs saved before presets existed load with none.
        let old: UiPrefs = toml::from_str("show_panels = false").unwrap();
        assert!(old.project_presets.is_empty());
    }

    #[test]
    fn a_pinned_drawing_is_ghosted_once_in_the_pin_colour() {
        let mut state = AppState::for_test();
        state.onion.enabled = true;
        state.onion.light(2, 1);
        for _ in 0..3 {
            state.structural_edit(false, |p| {
                p.add_frame();
                p.insert_blank_key_here();
            });
        }
        let li = state.project.current_layer;
        let pin_cell = state.project.layers[li].resolve(1).unwrap();
        state.project.layers[li].onion_pins.push(OnionPin {
            frame: 1,
            tint: [0, 200, 0],
            visible: true,
            opacity: 0.75,
        });
        let pins = state.onion_pin_ghosts(li);
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].1, pin_cell);
        let prev = state.onion_steps(li, OnionDirection::Prev);
        assert!(prev.iter().all(|s| s.cell != pin_cell), "{prev:?}");
        assert_eq!(prev.len(), 1, "the other ghost in range still shows");
        // Switching the onion skin off hides pins too.
        state.onion.enabled = false;
        assert!(state.onion_pin_ghosts(li).is_empty());
    }

    /// The fix for the big-layer stalls in one test: moving the timeline
    /// around re-uploads nothing, and a stroke re-uploads only its own rect.
    #[test]
    fn only_changed_pixels_are_marked_for_upload() {
        let mut state = AppState::for_test();
        state.structural_edit(false, |p| p.add_frame());
        for d in state.cell_dirty.values_mut() {
            *d = Dirty::Clean;
        }
        state.structural_edit(false, |p| {
            p.add_frame();
            p.insert_blank_key_here();
        });
        state.ensure_cell_tracking();
        let old = 0;
        assert_eq!(state.cell_dirty[&old], Dirty::Clean, "structure alone uploads nothing");

        let c = state.project.cell_mut(old).unwrap();
        c.dirty = None;
        c.pixels_mut()[3] = 255;
        c.mark_dirty(0, 0, 1, 1);
        state.mark_dirty(old);
        assert_eq!(state.cell_dirty[&old], Dirty::Rect(rect(0, 0, 1, 1)));
    }

    #[test]
    fn undo_of_a_structural_edit_uploads_nothing() {
        let mut state = AppState::for_test();
        state.structural_edit(false, |p| p.add_frame());
        for d in state.cell_dirty.values_mut() {
            *d = Dirty::Clean;
        }
        state.undo();
        assert!(state.cell_dirty.values().all(|&d| d == Dirty::Clean));
    }

    /// Undo of a stroke re-uploads the stroke's rect, not the cell.
    #[test]
    fn undo_of_a_stroke_uploads_its_rect() {
        let mut state = AppState::for_test();
        let id = 0;
        state.history.push(undo::Command::PixelPatch {
            cell: id,
            x: 4,
            y: 6,
            w: 2,
            h: 3,
            before: vec![0; 2 * 3 * 4],
            after: vec![255; 2 * 3 * 4],
        });
        for d in state.cell_dirty.values_mut() {
            *d = Dirty::Clean;
        }
        state.undo();
        assert_eq!(state.cell_dirty[&id], Dirty::Rect(rect(4, 6, 6, 9)));
    }

    /// Four layers, the second one active, fade on.
    fn faded_stack() -> AppState {
        let mut state = AppState::for_test();
        while state.project.layers.len() < 4 {
            state.project.add_layer();
        }
        state.project.current_layer = 1;
        state.fade = FadeOthers { above: 0.2, below: 0.5 };
        state.fade_others = true;
        state
    }

    #[test]
    fn fade_others_splits_above_and_below_the_active_layer() {
        let state = faded_stack();
        let alphas: Vec<f32> = (0..4).map(|i| state.layer_view_alpha(i)).collect();
        assert_eq!(alphas, [0.5, 1.0, 0.2, 0.2]);
    }

    #[test]
    fn fade_others_spares_reference_and_lines_from_layers() {
        let mut state = faded_stack();
        state.project.layers[3].reference = true;
        state.project.layers[1].lines_from = Some(0);
        assert_eq!(state.layer_view_alpha(3), 1.0);
        assert_eq!(state.layer_view_alpha(0), 1.0);
        // Only the *active* layer's link counts.
        state.project.layers[1].lines_from = None;
        state.project.layers[2].lines_from = Some(0);
        assert_eq!(state.layer_view_alpha(0), 0.5);
    }

    /// The keymap's link toggle: to the nearest drawing layer below, past a
    /// header; again to unlink; nothing under the bottom layer.
    #[test]
    fn the_key_link_toggle_links_the_layer_below() {
        let mut state = AppState::for_test();
        state.project.add_layer();
        state.project.add_layer();
        state.project.layers[1].kind = crate::doc::layer::LayerKind::Group;
        state.project.current_layer = 2;
        state.dispatch(Action::LayerKeyLinkBelow);
        assert_eq!(state.project.key_link(2), Some(0), "past the header");
        state.dispatch(Action::LayerKeyLinkBelow);
        assert_eq!(state.project.layers[2].keys_from, None, "again unlinks");
        state.project.current_layer = 0;
        state.dispatch(Action::LayerKeyLinkBelow);
        assert_eq!(state.project.layers[0].keys_from, None, "nothing under the bottom");
        assert_eq!(state.history.undo_len(), 0, "a link is no undo step");
    }

    /// "New cleanup layer": over the rough, named after it, linked to it,
    /// active — and one undo takes the lot back.
    #[test]
    fn a_cleanup_layer_is_linked_and_one_undo() {
        let mut state = AppState::for_test();
        state.project.add_layer();
        state.project.layers[0].name = "Rough".into();
        let rough = state.project.layers[0].uid;
        let n = state.project.layers.len();
        state.add_cleanup_layer(0);
        assert_eq!(state.project.layers.len(), n + 1);
        assert_eq!(state.project.current_layer, 1, "directly over the rough");
        let clean = &state.project.layers[1];
        assert_eq!(clean.name, "Rough clean");
        assert_eq!(clean.keys_from, Some(rough));
        assert_eq!(state.history.undo_len(), 1);
        state.undo();
        assert_eq!(state.project.layers.len(), n);
        assert!(state.project.layers.iter().all(|l| l.keys_from.is_none()));
    }

    #[test]
    fn fade_others_lifts_while_playing_or_switched_off() {
        let mut state = faded_stack();
        state.playback.playing = true;
        assert!((0..4).all(|i| state.layer_view_alpha(i) == 1.0));
        state.playback.playing = false;
        state.dispatch(Action::FadeOthersToggle);
        assert!(!state.fade_others);
        assert!((0..4).all(|i| state.layer_view_alpha(i) == 1.0));
    }

    /// A deleted layer's drawing is kept while undo can bring it back, and
    /// freed — texture and all — once the step that could has left history.
    #[test]
    fn a_dropped_drawing_lives_as_long_as_undo_can_reach_it() {
        let mut st = AppState::for_test();
        let ctx = egui::Context::default();
        st.project.cell_mut(0).unwrap().pixels_mut()[3] = 255;
        st.project.add_layer();
        st.project.current_layer = 0;
        let pool = st.project.cells.len();
        let _ = sync(&mut st, &ctx);
        assert!(st.cell_textures.contains_key(&0));

        st.structural_edit(false, |p| p.delete_layer());
        st.collect_cells();
        assert!(!st.project.cells[0].is_tombstone(), "undo can still bring it back");
        st.undo();
        assert_eq!(st.project.layers[0].resolve(0), Some(0));
        assert_eq!(st.project.cells[0].pixels()[3], 255);

        st.project.current_layer = 0;
        st.structural_edit(false, |p| p.delete_layer());
        for _ in 0..80 {
            st.history.push(undo::Command::Selection { before: None, after: None });
        }
        st.collect_cells();
        assert!(st.project.cells[0].is_tombstone(), "out of undo's reach: freed");
        assert!(!st.cell_textures.contains_key(&0));
        assert_eq!(st.project.cells.len(), pool, "no id moved");
    }

    /// A packed drawing looks the same: same texture, same export.
    #[test]
    fn packing_changes_nothing_on_screen_or_in_export() {
        let mut st = AppState::for_test();
        let ctx = egui::Context::default();
        ink(&mut st, 0, rect(100, 50, 120, 60));
        ink(&mut st, 0, rect(300, 200, 310, 205));
        let full = sync(&mut st, &ctx);
        let flat = crate::io::composite::flatten_frame(&st.project, 0).into_pixels();
        let tex = st.layer_texture(0).unwrap().1.id();
        let image = |d: &egui::TexturesDelta| {
            let (_, d) = d.set.iter().find(|(t, _)| *t == tex).expect("uploaded");
            match &d.image {
                egui::ImageData::Color(img) => img.pixels.clone(),
                _ => panic!("colour image expected"),
            }
        };
        let before = image(&full);

        assert!(!st.project.cells[0].is_packed(), "drawn on, so whole");
        st.project.current_layer = 0;
        st.project.add_layer(); // the drawing is no longer the active slot
        st.pack_idle_cells();
        assert!(st.project.cells[0].is_packed());
        st.mark_region(0, Dirty::Full);
        let again = sync(&mut st, &ctx);
        assert_eq!(image(&again), before);
        assert_eq!(crate::io::composite::flatten_frame(&st.project, 0).into_pixels(), flat);
    }

    /// The packer never takes the buffer out from under a stroke.
    #[test]
    fn the_packer_leaves_the_drawing_being_drawn_alone() {
        let mut st = AppState::for_test();
        st.dispatch(Action::ToolInk);
        st.pointer_down(st.make_sample(40.0, 40.0, 0.0));
        st.pointer_move(st.make_sample(80.0, 40.0, 0.1));
        let target = st.stroke_target.unwrap();
        // Off the active slot, it's still the stroke's.
        st.project.add_layer();
        st.pack_idle_cells();
        assert!(!st.project.cells[target].is_packed());
        st.pointer_up();
        st.pack_idle_cells();
        assert!(st.project.cells[target].is_packed(), "done with: packed");
        assert!(!st.project.cells[target].is_blank());
    }

    /// The shortcut has no clock to start from, but the first tick must still
    /// step from where the playhead sits, not by however long it was paused.
    #[test]
    fn play_shortcut_starts_from_the_current_frame() {
        let mut st = AppState::for_test();
        while st.project.frame_count < 10 {
            st.project.add_frame();
        }
        st.project.goto(3);
        st.dispatch(Action::PlayPause);
        let fps = st.project.fps as f64;
        // A while after launch: 175 frames' worth past the old anchor, which
        // isn't a whole number of 10-frame loops.
        let t = 7.3;
        st.playback.tick(&mut st.project, t, true);
        assert_eq!(st.project.current_frame, 3, "the first tick only starts the clock");
        st.playback.tick(&mut st.project, t + 1.0 / fps, true);
        assert_eq!(st.project.current_frame, 4);
    }

    /// Play/Pause stops back on the frame playback started on, wherever the
    /// playhead had got to; the pause-here action stays put.
    #[test]
    fn stopping_goes_back_to_the_start_frame_unless_paused_here() {
        let mut st = AppState::for_test();
        st.project.ensure_frame_count(24);
        let fps = st.project.fps as f64;
        st.project.goto(8);
        st.play_pause(Some(0.0));
        st.advance_playback(5.2 / fps);
        assert_eq!(st.project.current_frame, 13, "played on");
        st.play_pause(Some(6.0 / fps));
        assert!(!st.playback.playing);
        assert_eq!(st.project.current_frame, 8);

        st.dispatch(Action::PlayPauseHere);
        st.advance_playback(1.0);
        st.advance_playback(1.0 + 3.2 / fps);
        assert_eq!(st.project.current_frame, 11);
        st.dispatch(Action::PlayPauseHere);
        assert!(!st.playback.playing);
        st.advance_playback(2.0);
        assert_eq!(st.project.current_frame, 11, "paused where it was");
    }

    /// With the loop off, running out at the end of the range goes back to
    /// the start frame too.
    #[test]
    fn running_out_with_the_loop_off_goes_back_to_the_start_frame() {
        let mut st = AppState::for_test();
        st.project.ensure_frame_count(12);
        st.loop_timeline = false;
        let fps = st.project.fps as f64;
        st.project.goto(8);
        st.play_pause(Some(0.0));
        st.advance_playback(2.2 / fps);
        assert_eq!(st.project.current_frame, 10);
        assert!(st.advance_playback(9.0 / fps), "the frame on screen changed");
        assert!(!st.playback.playing);
        assert_eq!(st.project.current_frame, 8);
    }

    /// The toggle is session-only: a fresh state never opens faded.
    #[test]
    fn fade_others_starts_off() {
        assert!(!AppState::for_test().fade_others);
    }

    /// Old against new on one cell of a 4K frame widened to 3×. Not a
    /// pass/fail test — run it for the numbers:
    /// `cargo test --release perf_big_layer -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn perf_big_layer() {
        use std::hint::black_box;
        use std::time::Instant;
        let (w, h) = (11520u32, 2160u32);
        let (wu, hu) = (w as usize, h as usize);
        // A band of semi-transparent paint, like a few strokes.
        let mut px = vec![0u8; wu * hu * 4];
        for y in 1000..1100 {
            for x in 0..wu {
                let i = (y * wu + x) * 4;
                px[i..i + 4].copy_from_slice(&[20, 20, 20, 200]);
            }
        }
        let blank = vec![0u8; px.len()];
        let time = |label: &str, f: &mut dyn FnMut()| {
            f(); // warm the caches and the allocator
            let start = Instant::now();
            for _ in 0..3 {
                f();
            }
            let ms = start.elapsed().as_secs_f64() * 1e3 / 3.0;
            eprintln!("{label:<46} {ms:>8.1} ms");
        };
        time("pen-up upload, whole cell (old)", &mut || {
            black_box(premultiplied_image([wu, hu], &px));
        });
        time("pen-up upload, 300x300 stroke rect (new)", &mut || {
            let mut buf = Vec::with_capacity(300 * 300 * 4);
            for y in 1000..1300 {
                let row = (y * wu + 5000) * 4;
                buf.extend_from_slice(&px[row..row + 300 * 4]);
            }
            black_box(premultiplied_image([300, 300], &buf));
        });
        time("onion ghost, full size (old)", &mut || {
            let pixels: Vec<Color32> = px
                .chunks_exact(4)
                .map(|p| match p[3] {
                    0 => Color32::TRANSPARENT,
                    a => Color32::from_rgba_premultiplied(0, 0, a, a),
                })
                .collect();
            black_box(pixels);
        });
        time("onion ghost, downsampled (new)", &mut || {
            black_box(ghost_image(&Canvas::from_pixels(w, h, px.clone()), [0, 0, 255]));
        });
        time("blank scan of a blank cell, per pixel (old)", &mut || {
            black_box(blank.chunks_exact(4).all(|p| p[3] == 0));
        });
        time("blank scan of a blank cell, word-wise (new)", &mut || {
            black_box(no_alpha(&blank));
        });
        time("pen-down snapshot, fresh clone (old)", &mut || {
            black_box(px.clone());
        });
        let mut kept: Vec<u8> = Vec::new();
        time("pen-down snapshot, kept buffer (new)", &mut || {
            kept.clear();
            kept.extend_from_slice(&px);
            black_box(&kept);
        });
    }

    /// `n` frames of a `w`×`h` project, each keyed with its own drawing: a
    /// few dozen thick lines across the middle of the canvas, packed.
    fn probe_project(w: u32, h: u32, n: usize) -> Project {
        let mut p = Project::new(w, h, 24.0);
        for f in 0..n {
            if f > 0 {
                p.add_frame();
                p.goto(f);
                p.insert_blank_key_here();
            }
            let id = p.ensure_active_cell();
            let c = p.cell_mut(id).expect("keyed cell");
            let (x0, x1) = (w as usize / 5, w as usize * 4 / 5);
            let px = c.pixels_mut();
            for k in 0..40 {
                let y0 = h as usize / 5 + k * (h as usize * 3 / 5) / 40 + f * 7;
                for y in y0..y0 + 6 {
                    let row = y * w as usize * 4;
                    for x in x0..x1 {
                        px[row + x * 4..row + x * 4 + 4].copy_from_slice(&[20, 20, 20, 255]);
                    }
                }
            }
            c.pack();
        }
        p.goto(0);
        p
    }

    /// One frame as the app runs it: housekeeping, texture sync, the whole
    /// UI, tessellation. Its time in ms and the bytes it sent to the GPU.
    fn probe_frame(st: &mut AppState, ctx: &egui::Context) -> (f64, usize) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
            ..Default::default()
        };
        let start = Instant::now();
        let out = ctx.run(raw, |ctx| {
            st.collect_cells();
            st.pack_idle_cells();
            st.warm_pre(ctx);
            st.sync_textures(ctx);
            ui::shell::draw(st, ctx);
        });
        std::hint::black_box(ctx.tessellate(out.shapes, out.pixels_per_point));
        let ms = start.elapsed().as_secs_f64() * 1e3;
        let bytes = out
            .textures_delta
            .set
            .iter()
            .map(|(_, d)| d.image.width() * d.image.height() * 4)
            .sum();
        (ms, bytes)
    }

    /// Draw a straight stroke from `a` to `b`, four pen samples a frame,
    /// and print what each stage cost.
    fn probe_stroke(st: &mut AppState, ctx: &egui::Context, label: &str, a: (f32, f32), b: (f32, f32)) {
        const MB: f64 = 1024.0 * 1024.0;
        let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
        let steps = ((b.0 - a.0).hypot(b.1 - a.1) / 6.0).ceil().max(1.0) as usize;
        let sample = |i: usize| {
            let t = i as f32 / steps as f32;
            PointerSample {
                x: a.0 + (b.0 - a.0) * t,
                y: a.1 + (b.1 - a.1) * t,
                pressure: 0.8,
                t: i as f32 / 240.0,
                time_ms: i as f64 * 1000.0 / 240.0,
                ..Default::default()
            }
        };
        let undo_before = st.history.patch_bytes();
        let start = Instant::now();
        st.pointer_down(sample(0));
        let down = ms(start);
        let (first, mut sent) = probe_frame(st, ctx);
        let mut frames = Vec::new();
        let moves: Vec<usize> = (1..=steps).collect();
        for chunk in moves.chunks(4) {
            let start = Instant::now();
            for &i in chunk {
                st.pointer_move(sample(i));
            }
            let pushed = ms(start);
            let (f, bytes) = probe_frame(st, ctx);
            frames.push(pushed + f);
            sent += bytes;
        }
        let start = Instant::now();
        st.pointer_up();
        let up = ms(start);
        let (after, after_bytes) = probe_frame(st, ctx);
        for _ in 0..5 {
            probe_frame(st, ctx);
        }
        let mean = frames.iter().sum::<f64>() / frames.len().max(1) as f64;
        let max = frames.iter().copied().fold(0.0, f64::max);
        let undo = st.history.patch_bytes().saturating_sub(undo_before);
        eprintln!(
            "{label:<30} down {down:>6.1} +{first:>6.1} | stroke mean {mean:>5.1} max {max:>6.1} \
             sent {:>6.1} MB | up {up:>6.1} +{after:>6.1} sent {:>6.1} MB | undo {:>5.1} MB",
            sent as f64 / MB,
            after_bytes as f64 / MB,
            undo as f64 / MB,
        );
    }

    /// Liquify timings by canvas and brush size: pen-down, then a 600 px push
    /// at four pen samples a frame (`move`, the pointer calls; `total`, them
    /// and the frame after, where the warp catches up), and the apply
    /// (Enter). `cargo test --release perf_liquify -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn perf_liquify() {
        const MB: f64 = 1024.0 * 1024.0;
        let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
        for (w, h) in [(1920u32, 1080u32), (3840, 2160), (7680, 4320)] {
            for radius in [50.0f32, 200.0, 400.0, 700.0, 1000.0] {
                let mut st = AppState::for_test();
                st.load_project(probe_project(w, h, 1), None);
                st.lock_brush_to_view = false;
                st.set_tool(ActiveTool::Liquify);
                st.brush.radius = radius;
                st.brush.liquify_strength = 1.0;
                let ctx = egui::Context::default();
                for _ in 0..30 {
                    probe_frame(&mut st, &ctx);
                }
                let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
                let (a, b) = ((cx - 300.0, cy), (cx + 300.0, cy + 60.0));
                let steps = 100usize;
                let sample = |i: usize| {
                    let t = i as f32 / steps as f32;
                    PointerSample {
                        x: a.0 + (b.0 - a.0) * t,
                        y: a.1 + (b.1 - a.1) * t,
                        pressure: 1.0,
                        t: i as f32 / 240.0,
                        time_ms: i as f64 * 1000.0 / 240.0,
                        ..Default::default()
                    }
                };
                let start = Instant::now();
                st.pointer_down(sample(0));
                let down = ms(start);
                let (first, _) = probe_frame(&mut st, &ctx);
                let (mut moves, mut frames, mut sent) = (Vec::new(), Vec::new(), 0usize);
                let all: Vec<usize> = (1..=steps).collect();
                for chunk in all.chunks(4) {
                    let start = Instant::now();
                    for &i in chunk {
                        st.pointer_move(sample(i));
                    }
                    moves.push(ms(start));
                    let (f, bytes) = probe_frame(&mut st, &ctx);
                    frames.push(f + moves[moves.len() - 1]);
                    sent += bytes;
                }
                st.pointer_up();
                let start = Instant::now();
                st.commit_liquify();
                let apply = ms(start);
                let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
                let max = |v: &[f64]| v.iter().copied().fold(0.0, f64::max);
                eprintln!(
                    "{w}x{h} r{radius:<4} down {down:>6.1} +{first:>6.1} | move mean {:>6.1} max {:>6.1} | \
                     total mean {:>6.1} max {:>6.1} | sent {:>7.1} MB | apply {apply:>6.1}",
                    mean(&moves),
                    max(&moves),
                    mean(&frames),
                    max(&frames),
                    sent as f64 / MB,
                );
            }
        }
    }

    /// Stroke timings on a big portrait canvas, stage by stage: pen-down
    /// (the call, then the frame after it), the frames of the stroke, and
    /// pen-up. Not a pass/fail test — run it for the numbers:
    /// `cargo test --release perf_big_canvas_stroke -- --ignored --nocapture`
    /// With `ANIM_PROBE` set to an `.anim` path it measures that project,
    /// read only, instead of a made-up 4320×7680 one.
    #[test]
    #[ignore]
    fn perf_big_canvas_stroke() {
        use crate::tools::BrushMode;
        // Loaded afresh each time: a project the probe kept a copy of would
        // share its cells, and the first write to each would copy it.
        let project = || match std::env::var_os("ANIM_PROBE") {
            Some(p) => crate::io::project_file::load_from(std::path::Path::new(&p)).expect("probe project"),
            None => probe_project(4320, 7680, 3),
        };
        for mode in [BrushMode::Ribbon, BrushMode::Krita] {
            let mut st = AppState::for_test();
            st.load_project(project(), None);
            let (w, h) = (st.project.width as f32, st.project.height as f32);
            st.auto_key_draw = true;
            st.lock_brush_to_view = false;
            // As drawn with: one ghost each side.
            st.onion.enabled = true;
            st.onion.light(1, 1);
            st.tool = ActiveTool::Pencil;
            st.brush.mode = mode;
            st.brush.radius = 30.0;
            let ctx = egui::Context::default();
            let settle = |st: &mut AppState| {
                for _ in 0..30 {
                    probe_frame(st, &ctx);
                }
            };
            settle(&mut st);
            eprintln!(
                "\n{mode:?} brush, {}x{}, {} cells. Times in ms.",
                st.project.width,
                st.project.height,
                st.project.cells.len()
            );
            // A frame past the end holds the last drawing; auto-key gives
            // the first stroke there a new blank one.
            st.project.goto(st.project.frame_count - 1);
            st.project.add_frame();
            settle(&mut st);
            let (cx, cy) = (w / 2.0, h / 2.0);
            probe_stroke(&mut st, &ctx, "new drawing, 600 px", (cx - 300.0, cy), (cx + 300.0, cy + 100.0));
            probe_stroke(&mut st, &ctx, "same drawing, 600 px", (cx - 300.0, cy + 200.0), (cx + 300.0, cy + 300.0));
            probe_stroke(&mut st, &ctx, "same drawing, long diagonal", (w * 0.15, h * 0.2), (w * 0.85, h * 0.6));
            // Back to an old drawing, inside what's drawn there.
            st.project.goto(0);
            settle(&mut st);
            let mid = st
                .project
                .resolved_current()
                .and_then(|id| st.project.cell(id))
                .and_then(Canvas::content_bounds)
                .map_or((cx, cy), |r| ((r.min_x + r.max_x) as f32 / 2.0, (r.min_y + r.max_y) as f32 / 2.0));
            probe_stroke(&mut st, &ctx, "old drawing, 600 px", (mid.0 - 300.0, mid.1), (mid.0 + 300.0, mid.1 + 100.0));
        }
    }
}

#[cfg(test)]
mod camera_rig_tests {
    use super::*;
    use crate::doc::camera::Fill;

    #[test]
    fn a_camera_the_size_of_the_canvas_has_no_size_of_its_own() {
        let mut st = AppState::for_test();
        let canvas = (st.project.width, st.project.height);
        assert_eq!(st.project.output_size(), canvas);
        st.set_camera_size(Some((640, 360)));
        assert_eq!(st.project.output_size(), (640, 360));
        st.set_camera_size(Some(canvas));
        assert_eq!(st.project.camera_opts.size, None);
        st.set_camera_size(Some((0, 99_999)));
        assert_eq!(st.project.output_size(), (1, MAX_CAMERA_SIDE));
        st.set_camera_size(None);
        assert_eq!(st.project.output_size(), canvas);
    }

    /// Changing the size is not a camera move: zoom and keys stay as they
    /// were, so the frame shows the same pixels at the same scale.
    #[test]
    fn a_new_size_leaves_zoom_and_keys_alone() {
        let mut st = AppState::for_test();
        st.project.camera.zoom = 1.5;
        st.add_camera_key();
        let (cam, keys) = (st.project.camera, st.project.camera_keys.len());
        st.set_camera_size(Some((640, 360)));
        assert_eq!(st.project.camera, cam);
        assert_eq!(st.project.camera_keys.len(), keys);
    }

    #[test]
    fn undo_never_changes_the_camera_s_size() {
        let mut st = AppState::for_test();
        st.set_camera_size(Some((640, 360)));
        st.add_camera_key();
        st.set_camera_size(Some((320, 180)));
        st.undo();
        assert!(st.project.camera_keys.is_empty(), "the key went");
        assert_eq!(st.project.output_size(), (320, 180), "the size stayed");
        st.redo();
        assert_eq!(st.project.output_size(), (320, 180));
    }

    /// The size itself is the session's, but the last one set is kept
    /// between runs — and an older prefs blob without it still loads.
    #[test]
    fn the_last_camera_size_is_kept_between_runs() {
        use eframe::App as _;
        let mut st = AppState::for_test();
        st.set_camera_size(Some((640, 360)));
        st.set_camera_size(None);
        assert_eq!(st.last_camera_size, Some((640, 360)), "handing it back keeps the memory");
        let mut storage = tests::MemStorage::default();
        st.save(&mut storage);
        let prefs: UiPrefs = eframe::get_value(&storage, UI_PREFS_KEY).unwrap();
        assert_eq!(prefs.last_camera_size, Some((640, 360)));
        let next = AppState::with_prefs(prefs, 8192, None);
        assert_eq!(next.last_camera_size, Some((640, 360)));
        assert_eq!(next.project.camera_opts.size, None, "the size is not");
        assert_eq!(UiPrefs::default().last_camera_size, None);
    }

    /// A 1280×720 canvas with a 640×360 frame in the middle of it: x
    /// 320..960, y 180..540, the centre at (640, 360).
    fn framed() -> AppState {
        let mut st = AppState::for_test();
        st.load_project(Project::new(1280, 720, 24.0), None);
        st.set_camera_size(Some((640, 360)));
        st.view_scale = 1.0;
        st.camera_edit = true;
        st
    }

    #[test]
    fn dragging_inside_the_frame_moves_the_camera_in_one_undo_step() {
        let mut st = framed();
        assert!(st.camera_frame_down((600.0, 300.0)));
        st.camera_frame_move((650.0, 320.0));
        st.camera_frame_move((700.0, 280.0));
        st.camera_frame_up();
        let cam = st.project.camera;
        assert_eq!((cam.tx, cam.ty, cam.zoom, cam.rot), (100.0, -20.0, 1.0, 0.0));
        assert_eq!(st.history.undo_len(), 1);
        st.undo();
        assert!(st.project.camera.is_identity());
    }

    #[test]
    fn dragging_a_corner_zooms_and_past_a_corner_rolls() {
        let mut st = framed();
        // The corner at (960, 540) pulled out to twice as far from the centre.
        assert!(st.camera_frame_down((960.0, 540.0)));
        st.camera_frame_move((1280.0, 720.0));
        st.camera_frame_up();
        assert!((st.project.camera.zoom - 0.5).abs() < 1e-5);
        assert_eq!((st.project.camera.tx, st.project.camera.rot), (0.0, 0.0));

        // The frame is the whole canvas now; just past its corner rolls it.
        assert!(st.camera_frame_down((1288.0, 728.0)));
        st.camera_frame_move((1288.0, 728.0));
        assert_eq!(st.cam_drag.map(|d| d.grab), Some(CamGrab::Roll));
        st.camera_frame_move((640.0 - 368.0, 360.0 + 648.0));
        st.camera_frame_up();
        assert!((st.project.camera.rot - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        assert_eq!(st.history.undo_len(), 2);
    }

    /// Off the frame, and always with camera edit off, the press is the
    /// tool's — nothing is grabbed and nothing lands on the undo stack.
    #[test]
    fn a_press_off_the_frame_or_outside_edit_mode_is_left_to_the_tool() {
        let mut st = framed();
        assert!(!st.camera_frame_down((100.0, 100.0)));
        st.camera_edit = false;
        assert!(!st.camera_frame_down((600.0, 300.0)));
        st.camera_frame_move((700.0, 300.0));
        st.camera_frame_up();
        assert!(st.project.camera.is_identity());
        assert!(st.cam_drag.is_none());
        assert_eq!(st.history.undo_len(), 0);
    }

    #[test]
    fn a_click_on_the_frame_that_moves_nothing_is_no_undo_step() {
        let mut st = framed();
        assert!(st.camera_frame_down((600.0, 300.0)));
        st.camera_frame_up();
        assert_eq!(st.history.undo_len(), 0);
        assert!(st.cam_drag.is_none());
    }

    #[test]
    fn a_dragged_frame_is_held_on_the_canvas_when_asked() {
        let mut st = framed();
        st.project.camera_opts.keep_inside = true;
        assert!(st.camera_frame_down((600.0, 300.0)));
        st.camera_frame_move((5000.0, 300.0));
        st.camera_frame_up();
        assert_eq!(st.project.camera.tx, 320.0, "half the canvas less half the frame");
    }

    /// Looking through the camera, the view stays where it was while the
    /// frame is dragged — following it would move the ground under the
    /// cursor — and catches up when it is let go.
    #[test]
    fn the_view_holds_still_under_a_dragged_frame() {
        let mut st = framed();
        st.camera_look_through = true;
        assert!(st.camera_frame_down((600.0, 300.0)));
        st.camera_frame_move((700.0, 300.0));
        assert_eq!(st.project.camera.tx, 100.0);
        assert!(st.look_through_camera().is_identity());
        st.camera_frame_up();
        assert_eq!(st.look_through_camera().tx, 100.0);
    }

    /// Five frames at 10 fps, the camera keyed from 0 on the first to 40 on
    /// the last.
    fn panning() -> AppState {
        use crate::doc::camera::CameraKey;
        let mut p = Project::new(64, 64, 10.0);
        p.ensure_frame_count(5);
        let key = |frame, tx| CameraKey {
            frame,
            camera: Camera {
                tx,
                ..Default::default()
            },
            ease: Ease::Linear,
        };
        p.camera_keys = vec![key(0, 0.0), key(4, 40.0)];
        let mut st = AppState::for_test();
        st.load_project(p, None);
        st.loop_timeline = true;
        st
    }

    /// Play `steps` frames of the app's clock from where it stands, as
    /// `update` does: the drawing and the camera showing after each.
    fn play(st: &mut AppState, steps: usize) -> (Vec<usize>, Vec<f32>) {
        if !st.playback.playing {
            st.playback.toggle(0.0);
        }
        let (mut frames, mut cams) = (Vec::new(), Vec::new());
        let t0 = st.playback.last_advance_t;
        for i in 1..=steps {
            st.advance_playback(t0 + (i as f64 + 0.25) / st.project.fps as f64);
            st.sync_camera_buffer();
            frames.push(st.project.current_frame);
            cams.push(st.display_camera(st.project.current_frame).tx);
        }
        (frames, cams)
    }

    /// The pin: with the move left at one loop, playback is the frames with
    /// the camera keyed on each, round and round.
    #[test]
    fn at_one_loop_the_camera_plays_in_step_with_the_frames() {
        let mut st = panning();
        let (frames, cams) = play(&mut st, 7);
        assert_eq!(frames, [1, 2, 3, 4, 0, 1, 2]);
        assert_eq!(cams, [10.0, 20.0, 30.0, 40.0, 0.0, 10.0, 20.0]);
        assert!(st.shot_cam.is_none());
    }

    #[test]
    fn over_three_loops_the_drawings_repeat_under_one_move() {
        let mut st = panning();
        st.project.camera_opts.loops = 3;
        let (frames, cams) = play(&mut st, 14);
        assert_eq!(frames, [1, 2, 3, 4, 0, 1, 2, 3, 4, 0, 1, 2, 3, 4]);
        assert!(cams.windows(2).all(|w| w[1] > w[0]), "one way, all the way: {cams:?}");
        assert!((cams[13] - 40.0).abs() < 1e-3);
        // ...and round again, camera and all.
        let (frames, cams) = play(&mut st, 2);
        assert_eq!(frames, [0, 1]);
        assert!(cams[0].abs() < 1e-3, "{cams:?}");
    }

    /// Stopping leaves the camera where it had got to and puts the playhead
    /// on that place in the move: paused, the timeline is the camera's again.
    #[test]
    fn pausing_a_stretched_move_parks_the_playhead_where_the_camera_is() {
        let mut st = panning();
        st.project.camera_opts.loops = 3;
        let (frames, cams) = play(&mut st, 7);
        assert_eq!(frames[6], 2, "the third drawing, second time round");
        assert!((cams[6] - 20.0).abs() < 1e-3, "halfway through the move");
        st.playback.stop();
        st.advance_playback(99.0);
        st.sync_camera_buffer();
        assert_eq!(st.project.current_frame, 2, "halfway along the timeline");
        assert!(st.shot_cam.is_none());
        assert_eq!(st.project.camera.tx, 20.0, "and the camera has not moved");

        // A pause a little further on parks on the nearest frame of the move.
        let (_, cams) = play(&mut st, 2);
        assert!((cams[1] - 9.0 * 40.0 / 14.0).abs() < 1e-3, "{cams:?}");
        st.playback.stop();
        st.advance_playback(199.0);
        st.sync_camera_buffer();
        assert_eq!(st.project.current_frame, 3);
        assert_eq!(st.project.camera.tx, 30.0);
    }

    /// A still camera has no move to stretch: it plays as it always did.
    #[test]
    fn a_camera_with_no_move_is_never_stretched() {
        let mut st = panning();
        st.project.camera_opts.loops = 3;
        st.project.camera_keys.truncate(1);
        assert_eq!(st.project.camera_loops(), 1);
        let (frames, _) = play(&mut st, 6);
        assert_eq!(frames, [1, 2, 3, 4, 0, 1]);
        assert!(st.shot_cam.is_none());
    }

    #[test]
    fn an_export_of_a_stretched_move_gives_each_frame_its_camera() {
        let mut st = panning();
        st.export_cfg.start = 0;
        st.export_cfg.end = 4;

        // One loop: the plain order, each frame through its own key time.
        let (order, cams) = st.export_order(ExportKind::Mp4, (0, 4));
        assert_eq!(order, [0, 1, 2, 3, 4]);
        assert!(cams.is_none());
        assert_eq!(st.export_info(ExportKind::Mp4).frames, 5);

        st.project.camera_opts.loops = 3;
        let (order, cams) = st.export_order(ExportKind::Mp4, (0, 4));
        let cams = cams.expect("a camera a frame");
        assert_eq!(order, [0, 1, 2, 3, 4, 0, 1, 2, 3, 4, 0, 1, 2, 3, 4]);
        assert_eq!(cams.len(), 15);
        assert_eq!((cams[0].tx, cams[14].tx), (0.0, 40.0));
        assert!(cams.windows(2).all(|w| w[1].tx > w[0].tx));
        let info = st.export_info(ExportKind::Mp4);
        assert_eq!((info.frames, info.secs), (15, 1.5));

        // The dialog's own ping-pong gives way to the camera's fill.
        st.export_cfg.ping_pong = true;
        assert_eq!(st.export_order(ExportKind::Gif, (0, 4)).0.len(), 15);
        st.project.camera_opts.fill = Fill::PingPong;
        let (order, cams) = st.export_order(ExportKind::Gif, (0, 4));
        assert_eq!(order[..9], [0, 1, 2, 3, 4, 3, 2, 1, 0]);
        assert_eq!(cams.unwrap().len(), 24);

        // A sheet is its frames side by side, each through its own camera.
        assert!(st.export_order(ExportKind::SpriteSheet, (0, 4)).1.is_none());
        assert_eq!(st.export_info(ExportKind::SpriteSheet).frames, 5);
    }

    /// The held camera and the smooth path reach a stretched move's frames
    /// too: every camera of the export is on the canvas.
    #[test]
    fn a_stretched_export_keeps_the_camera_s_other_options() {
        let mut st = panning();
        st.set_camera_size(Some((32, 32)));
        st.project.camera_opts.loops = 2;
        st.project.camera_opts.keep_inside = true;
        let (_, cams) = st.export_order(ExportKind::PngSequence, (0, 4));
        let cams = cams.unwrap();
        // A 32-wide frame on a 64-wide canvas has 16 either side.
        assert!(cams.iter().all(|c| c.tx <= 16.0), "{cams:?}");
        assert_eq!(cams[9].tx, 16.0);
    }

    #[test]
    fn opening_a_project_starts_the_camera_on_its_canvas() {
        let mut st = AppState::for_test();
        st.set_camera_size(Some((640, 360)));
        st.load_project(Project::new(1280, 720, 24.0), None);
        assert_eq!(st.project.output_size(), (1280, 720));
    }
}
