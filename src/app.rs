//! Top-level application state. Wires project (timeline + layers), tools, UI.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::CreationContext;
use egui::{Color32, ColorImage, TextureHandle, TextureOptions};

use crate::doc::camera::{Camera, Ease};
use crate::doc::canvas::{Canvas, DirtyRect};
use crate::doc::layer::{CellId, TrackSample};
use crate::doc::project::Project;
use crate::doc::transform::Transform;
use crate::input::pointer::PointerSample;
use crate::input::shortcuts::{self, Action, ShortcutMap};
use crate::input::tablet::{PenInput, PenPacket};
use crate::timeline::onion::{OnionConfig, OnionDirection, OnionStep};
use crate::timeline::playback::Playback;
use crate::tools::lasso::Mask;
use crate::tools::ribbon::{union_rect, StrokeWorkspace};
use crate::tools::selection::Selection;
use crate::tools::stroke::StrokeBuilder;
use crate::tools::{ActiveTool, BrushSettings, ShapeKind, SmoothingOptions};
use crate::ui;
use crate::undo::{self, History};

#[derive(Clone)]
pub struct NewProjectConfig {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
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
    Mp4,
    SpriteSheet,
}

impl ExportKind {
    pub fn title(self) -> &'static str {
        match self {
            ExportKind::PngSequence => "Export PNG sequence",
            ExportKind::Gif => "Export animated GIF",
            ExportKind::Mp4 => "Export MP4",
            ExportKind::SpriteSheet => "Export sprite sheet",
        }
    }

    /// Present-tense label for the busy overlay.
    fn busy(self) -> &'static str {
        match self {
            ExportKind::PngSequence => "Exporting PNG sequence…",
            ExportKind::Gif => "Exporting GIF…",
            ExportKind::Mp4 => "Exporting MP4…",
            ExportKind::SpriteSheet => "Exporting sprite sheet…",
        }
    }
}

/// Everything the shared export dialog edits. One struct for all four formats:
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

/// Which non-drawing canvas gesture an in-flight drag is performing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavKind {
    Pan,
    Zoom,
    Rotate,
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
}

/// The built-in per-tool brushes.
fn default_tool_brushes() -> [BrushSettings; 7] {
    [
        BrushSettings::default_pencil(),
        BrushSettings::default_ink(),
        BrushSettings::default_eraser(),
        BrushSettings::default_fill(),
        BrushSettings::default_shape(),
        // Tracker and Lasso draw nothing; the slots only keep tool indexing
        // into this array safe.
        BrushSettings::default_shape(),
        BrushSettings::default_shape(),
    ]
}

/// Saved brushes, or the defaults. A blob from a build with a different
/// number of tools is discarded rather than padded: the array is indexed by
/// `ActiveTool::idx`, so a short one would silently reassign brushes to the
/// wrong tools.
fn restore_tool_brushes(saved: Option<Vec<BrushSettings>>) -> [BrushSettings; 7] {
    match saved {
        Some(v) => <[BrushSettings; 7]>::try_from(v).unwrap_or_else(|_| default_tool_brushes()),
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
            loop_timeline: true,
            smoothing: SmoothingOptions::default(),
            tool_brushes: None,
            palette: Vec::new(),
        }
    }
}

/// The tool panels (each rendered as a floating window).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelId {
    Tools,
    Brush,
    Layers,
    Onion,
    Xsheet,
    Timeline,
    Camera,
}

impl Default for NewProjectConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 24.0,
        }
    }
}

/// Source for a start/end frame-range import.
pub enum ImportSource {
    /// Video file; frames extracted on confirm via ffmpeg. `fps` drives the
    /// timestamp seek used for dialog previews.
    Video { path: PathBuf, fps: f64 },
    /// GIF frames already decoded to full-size canvases.
    Gif(Vec<Canvas>),
}

/// Backing state for the frame-range import dialog (video / GIF).
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
    /// Decoding a GIF before the range dialog opens.
    GifDecode(Receiver<Result<Vec<Canvas>>>),
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
    /// Transient "saved" confirmation and the instant it expires. A silent
    /// overwrite is otherwise indistinguishable from a save that never ran.
    pub save_toast: Option<(String, Instant)>,
    /// Failed write, shown as a modal until dismissed. Never merely logged — a
    /// silent save that silently failed is how work gets lost.
    pub save_error: Option<String>,

    /// GPU texture handle per CellId, lazily created.
    pub cell_textures: HashMap<CellId, TextureHandle>,
    /// Per-CellId dirty flag — re-upload on next sync.
    pub cell_dirty: HashMap<CellId, bool>,
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

    pub tool: ActiveTool,
    pub brush: BrushSettings,
    /// Pinned swatches, most recently added last. Capped at
    /// [`AppState::MAX_SWATCHES`].
    pub palette: Vec<[u8; 3]>,
    /// Drawing clipboard: one cell's pixels, cut or copied from a slot. Held
    /// as a `Canvas` rather than a `CellId` so it survives the undo of the cut
    /// that produced it.
    pub cell_clip: Option<Canvas>,
    /// Floating lasso selection, if any. Bound to one cell: changing frame or
    /// layer commits it first.
    pub selection: Option<Selection>,
    /// Pointer position (cell space) of the last selection-move sample, or
    /// `None` when no move drag is in flight.
    sel_drag: Option<(f32, f32)>,
    /// Selection clipboard: mask plus lifted pixels.
    pixel_clip: Option<(Mask, Vec<u8>)>,
    /// Set when the floating pixels changed and their texture must be rebuilt.
    /// Moving does *not* set it — the offset moves the quad, not the texture.
    sel_tex_stale: bool,
    /// GPU texture for the floating pixels.
    pub selection_tex: Option<TextureHandle>,
    /// Per-tool brush settings preserved across tool switches.
    pub tool_brushes: [BrushSettings; 7],
    pub stroke: Option<StrokeBuilder>,
    /// CellId being painted into during the current stroke.
    pub stroke_target: Option<CellId>,
    /// In-progress Shape-tool drag (preview only until pointer-up).
    pub shape_drag: Option<ShapeDrag>,
    /// In-progress Lasso path in active-cell pixel space. Preview only; the
    /// enclosed pixels are erased on pointer-up.
    pub lasso: Option<Vec<(f32, f32)>>,

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

    pub playback: Playback,
    pub onion: OnionConfig,
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
    /// Leftover trackpad scroll (in points) not yet worth a whole frame step.
    /// Session-only: a wheel gesture never spans a run. Mice report whole
    /// lines and bypass this entirely — see `ui::shell::timeline_wheel_scrub`.
    pub wheel_scrub_accum: f32,

    pub bg_opacity: f32,
    /// Background clear color (RGB, 0..1).
    pub bg_color: [f32; 3],
    pub show_checker: bool,

    pub pen: PenInput,

    pub history: History,
    /// Full-pixel snapshot of the cell before the in-flight stroke started.
    /// Used to extract the `before` slice for an undo command when the stroke
    /// finishes.
    stroke_pre_pixels: Option<Vec<u8>>,
    /// Reusable per-stroke coverage workspace for the ribbon rasterizer.
    stroke_ws: StrokeWorkspace,
    /// Canvas region updated by stroke flushes since the last texture sync;
    /// uploaded to the GPU via a partial texture update each frame.
    pub preview_upload_rect: Option<DirtyRect>,

    pub shortcuts: ShortcutMap,
    /// While `Some(action)`, the next key press from the user becomes the
    /// new binding for that action.
    pub rebinding: Option<Action>,
    /// In-progress inline layer rename in the layers panel.
    pub layer_rename: Option<LayerRename>,
    pub show_settings: bool,
    /// The brush settings window. Session-only, like `show_settings`:
    /// where a window sits persists through egui's own memory, but
    /// whether it is open should not outlive the session that opened it.
    pub show_brush_settings: bool,
    /// Master visibility of all floating panel windows. Tab toggles it.
    pub show_panels: bool,
    /// Minimal timeline bar shown when `show_panels` is false. Has its own
    /// toggle shortcut so the user can hide it too.
    pub show_mini_timeline: bool,

    pub show_new_project: bool,
    pub new_project_cfg: NewProjectConfig,

    /// MP4 export settings dialog visibility + backing config.
    pub show_export: bool,
    pub export_cfg: ExportConfig,

    /// Frame-range import dialog (video / GIF) visibility + backing state.
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
    /// Last frame the camera edit buffer was synced for.
    cam_sync_last: Option<usize>,
    /// Lock the editor viewport to the camera, so what you see is the shot.
    pub camera_look_through: bool,
    /// Draw the camera's frame rect over the canvas.
    pub show_camera_guide: bool,
    /// Dim everything the camera can't see.
    pub dim_outside_camera: bool,
    /// Outline the active layer's cell bounds, so it's visible where a layer's
    /// drawable buffer ends (strokes past it are silently clipped).
    pub show_layer_bounds: bool,
    /// Edit buffer for the "expand layer canvas" fields: `(layer, w, h)`.
    /// `None` (or a stale layer) means prefill from the active layer's size.
    pub expand_cfg: Option<(usize, u32, u32)>,

    /// Layer selected before the current one, for the "jump back" shortcut.
    /// Maintained by `track_layer_change` rather than by every site that
    /// assigns `current_layer` — there are eight of those.
    prev_layer: Option<usize>,
    /// Previous frame's (current_layer, layer count), the baseline that
    /// `track_layer_change` diffs against.
    layer_watch: (usize, usize),

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

        let project = Project::new(1280, 720, 24.0);
        let mut cell_dirty = HashMap::new();
        for id in 0..project.cells.len() {
            cell_dirty.insert(id, true);
        }
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
        Self {
            project,
            project_path: None,
            save_toast: None,
            save_error: None,
            cell_textures: HashMap::new(),
            ghost_textures: HashMap::new(),
            ghost_stale: HashSet::new(),
            retired_textures: Vec::new(),
            cell_dirty,
            tool: ActiveTool::Pencil,
            brush: restore_tool_brushes(prefs.tool_brushes.clone())[ActiveTool::Pencil.idx()]
                .clone(),
            palette: prefs.palette,
            cell_clip: None,
            selection: None,
            sel_drag: None,
            pixel_clip: None,
            sel_tex_stale: false,
            selection_tex: None,
            tool_brushes: restore_tool_brushes(prefs.tool_brushes.clone()),
            stroke: None,
            stroke_target: None,
            shape_drag: None,
            lasso: None,
            view: View::default(),
            view_scale: 1.0,
            lock_brush_to_view: prefs.lock_brush_to_view,
            nav_drag: None,
            playback: Playback::default(),
            onion: prefs.onion,
            auto_key_transform: prefs.auto_key_transform,
            auto_key_draw: prefs.auto_key_draw,
            frame_step: prefs.frame_step,
            invert_timeline_scroll: prefs.invert_timeline_scroll,
            loop_timeline: prefs.loop_timeline,
            smoothing: prefs.smoothing,
            pen_outlier_logged: false,
            pen_batches_rejected: 0,
            pen_packets_dropped: 0,
            wheel_scrub_accum: 0.0,
            bg_opacity: 1.0,
            bg_color: [0.12, 0.12, 0.13],
            show_checker: false,
            pen: PenInput::new(),
            history: History::default(),
            stroke_pre_pixels: None,
            stroke_ws: StrokeWorkspace::new(),
            preview_upload_rect: None,
            shortcuts: shortcuts::load(),
            rebinding: None,
            layer_rename: None,
            show_settings: false,
            show_brush_settings: false,
            show_panels: prefs.show_panels,
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
            xform_sync_last: None,
            camera_edit: false,
            nav_to_camera: false,
            camera_drag_before: None,
            cam_sync_last: None,
            camera_look_through: false,
            show_camera_guide: prefs.show_camera_guide,
            dim_outside_camera: prefs.dim_outside_camera,
            show_layer_bounds: prefs.show_layer_bounds,
            expand_cfg: None,
            prev_layer: None,
            layer_watch: (0, 1),
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
        self.project = Project::new(width, height, fps);
        // Forget the old file, or the next Save silently overwrites the project
        // the user just navigated away from.
        self.project_path = None;
        self.save_toast = None;
        self.save_error = None;
        self.retire_cell_textures();
        self.retire_all_ghosts();
        self.cell_dirty.clear();
        for id in 0..self.project.cells.len() {
            self.cell_dirty.insert(id, true);
        }
        self.tool = ActiveTool::Pencil;
        self.brush = BrushSettings::default_pencil();
        self.tool_brushes = [
            BrushSettings::default_pencil(),
            BrushSettings::default_ink(),
            BrushSettings::default_eraser(),
            BrushSettings::default_fill(),
            BrushSettings::default_shape(),
            BrushSettings::default_shape(),
            BrushSettings::default_shape(),
        ];
        self.stroke = None;
        self.stroke_target = None;
        self.shape_drag = None;
        self.lasso = None;
        self.view = View::default();
        // Republished by the canvas next frame; kept in step with `view` so a
        // reset never leaves a stale scale behind for one frame of input.
        self.view_scale = 1.0;
        self.nav_drag = None;
        self.playback = Playback::default();
        // `onion` is deliberately not reset: like `show_panels` it is a
        // workspace preference, and resetting it here is what made tuned
        // settings feel like they never stuck.
        self.bg_opacity = 1.0;
        self.bg_color = [0.12, 0.12, 0.13];
        self.show_checker = false;
        self.history = History::default();
        self.stroke_pre_pixels = None;
        self.preview_upload_rect = None;
        self.rebinding = None;
        self.layer_rename = None;
        self.show_settings = false;
        self.show_brush_settings = false;
        // `show_panels` / `show_mini_timeline` are deliberately not reset here.
        // They're preferences that persist across runs, like `shortcuts` — a new
        // project shouldn't shove hidden panels back on screen.
        self.new_project_cfg = NewProjectConfig { width, height, fps };
        self.show_export = false;
        self.show_import_range = false;
        self.import_range = None;
        self.layer_xform = false;
        self.nav_to_layer = false;
        self.layer_xform_before = None;
        self.xform_sync_last = None;
        // Camera guide / dim / bounds are view preferences, kept like the panel
        // toggles above; only the transient edit + lock modes reset.
        self.camera_edit = false;
        self.nav_to_camera = false;
        self.camera_drag_before = None;
        self.cam_sync_last = None;
        self.camera_look_through = false;
        self.expand_cfg = None;
        self.prev_layer = None;
        self.layer_watch = (self.project.current_layer, self.project.layers.len());
        self.tracker_pending_b = None;
        self.bg_job = None;
        self.bg_label = None;
        self.preview_tex.clear();
        self.preview_rx = None;
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
        }
    }

    /// Screen pixels per *active-cell* pixel: the view scale folded with the
    /// active layer's own scale, since strokes are rasterized in cell space.
    /// Mirrors the `scale * layer_scale` product the canvas uses for previews.
    pub fn cell_view_scale(&self) -> f32 {
        let t = self.display_transform(self.project.current_layer, self.project.current_frame);
        (self.view_scale * t.scale.abs()).max(1e-6)
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

    pub fn mark_dirty(&mut self, id: CellId) {
        self.cell_dirty.insert(id, true);
        // The ghost is baked from these pixels, so it needs rebuilding — but
        // re-uploaded in place, never freed. See `ghost_stale`.
        if self.ghost_textures.contains_key(&id) {
            self.ghost_stale.insert(id);
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
            .extend(self.cell_textures.drain().map(|(_, tex)| tex));
    }

    /// Park every ghost for release on the next sync. See `retired_textures`.
    fn retire_all_ghosts(&mut self) {
        self.ghost_stale.clear();
        self.retired_textures
            .extend(self.ghost_textures.drain().map(|(_, (_, tex))| tex));
    }

    /// Mark every cell for re-upload (used after structural undo/redo, where
    /// the whole timeline may have shifted).
    fn mark_all_dirty(&mut self) {
        for id in 0..self.project.cells.len() {
            self.cell_dirty.insert(id, true);
        }
        self.retire_all_ghosts();
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
        let before = undo::TimelineState::capture(&self.project);
        let cells_before: Vec<Vec<u8>> = if capture_cells {
            self.project.cells.iter().map(|c| c.pixels.clone()).collect()
        } else {
            Vec::new()
        };

        edit(&mut self.project);

        let after = undo::TimelineState::capture(&self.project);
        let cell_pixels: Vec<undo::CellPixelDelta> = cells_before
            .into_iter()
            .enumerate()
            .filter_map(|(cell, before)| {
                let after = self.project.cells.get(cell)?.pixels.clone();
                (before != after).then_some(undo::CellPixelDelta { cell, before, after })
            })
            .collect();

        self.history.push(undo::Command::Structural {
            before,
            after,
            cell_pixels,
        });
        self.mark_all_dirty();
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
                p.cells.push(cells.into_iter().next().unwrap());
                p.layers[idx].set_key(0, id);
            } else {
                p.ensure_frame_count(cells.len());
                let idx = p.add_layer_below_active(name);
                p.layers[idx].transform = init_xform;
                for (f, canvas) in cells.into_iter().enumerate() {
                    let id = p.cells.len();
                    p.cells.push(canvas);
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
        let (Some(top), Some(below)) = (self.project.layers.get(li), self.project.layers.get(li - 1))
        else {
            return false;
        };
        !top.locked && !below.locked && !top.reference && !below.reference
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

        // Interpolated transforms change the picture on every frame; otherwise
        // only exposure keys (and a lone transform key's frame) matter.
        let animated = top.transform_keys.len() >= 2 || below.transform_keys.len() >= 2;
        let mut bake_frames: Vec<usize> = if animated {
            (0..p.frame_count).collect()
        } else {
            let mut fs: Vec<usize> = (0..p.frame_count)
                .filter(|&f| {
                    top.exposures.get(f).copied().flatten().is_some()
                        || below.exposures.get(f).copied().flatten().is_some()
                })
                .collect();
            for l in [top, below] {
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
                for l in [below, top] {
                    let Some(id) = l.resolve(f) else { continue };
                    let Some(src) = p.cell(id) else { continue };
                    let xform = l.resolve_transform(f);
                    // Passing the *baked* size as the doc size keeps the
                    // transform maths centred on the same point, since the
                    // baked cell is itself centred on the doc.
                    crate::io::composite::composite_layer(&mut out, src, &xform, l.opacity, bw, bh);
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
                p.cells.push(canvas);
                p.layers[bi].set_key(f, id);
            }
            p.layers.remove(li);
            p.relink_after_remove(li);
            p.current_layer = bi;
        });
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
            p.cells.push(canvas);
            p.layers[idx].set_key(0, id);
        });
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
        let Some(path) = rfd::FileDialog::new()
            .add_filter("GIF", &["gif"])
            .set_title("Choose a GIF to import")
            .pick_file()
        else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(crate::io::gif_import::decode_all(&path));
        });
        self.bg_job = Some(BgJob::GifDecode(rx));
        self.bg_label = Some("Decoding GIF…");
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
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let project = self.project.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let res = match kind {
                ExportKind::PngSequence => {
                    crate::io::png_seq::export_to(&project, &path, range)
                }
                ExportKind::Gif => crate::io::gif_export::export_to(&project, &path, range),
                ExportKind::Mp4 => {
                    crate::io::mp4_export::export_to(&project, &path, &settings, range)
                }
                ExportKind::SpriteSheet => {
                    crate::io::sprite_sheet::export_to(&project, &path, range, &sheet)
                }
            };
            let _ = tx.send(res.map(|()| name));
        });
        self.bg_job = Some(BgJob::Export(rx));
        self.bg_label = Some(kind.busy());
    }

    /// Confirm the frame-range dialog: slice (GIF, instant) or extract (video,
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
            ImportSource::Gif(frames) => {
                let slice: Vec<Canvas> = frames
                    .into_iter()
                    .skip(start)
                    .take(end - start + 1)
                    .collect();
                self.import_cells_as_layer("Imported GIF", slice, false);
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

    fn active_layer_locked(&self) -> bool {
        self.project
            .layers
            .get(self.project.current_layer)
            .map(|l| l.locked)
            .unwrap_or(true)
    }

    /// Translate the active layer by a document-space delta.
    pub fn apply_layer_pan(&mut self, dx: f32, dy: f32) {
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
        if self.active_layer_locked() {
            return;
        }
        if let Some(l) = self.project.layers.get_mut(self.project.current_layer) {
            l.transform.scale = (l.transform.scale * factor).clamp(0.01, 100.0);
        }
    }

    /// Rotate the active layer by `d` radians.
    pub fn apply_layer_rotate(&mut self, d: f32) {
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

    /// Keep the camera edit buffer in sync with its keyed value as the frame
    /// cursor moves. Same contract as `sync_active_transform_buffer`: only on
    /// an actual frame change, never mid-drag, and never when there are no keys
    /// (then the buffer *is* the value).
    fn sync_camera_buffer(&mut self) {
        if self.nav_to_camera {
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

    /// Move the camera by a document-space delta. The guide rect follows the
    /// cursor, so dragging right shows what is to the right.
    pub fn apply_camera_pan(&mut self, dx: f32, dy: f32) {
        self.project.camera.tx += dx;
        self.project.camera.ty += dy;
    }

    /// Push the camera in/out by `factor` (multiplicative).
    pub fn apply_camera_zoom(&mut self, factor: f32) {
        self.project.camera.zoom = (self.project.camera.zoom * factor).clamp(0.05, 64.0);
    }

    /// Roll the camera by `d` radians.
    pub fn apply_camera_rotate(&mut self, d: f32) {
        self.project.camera.rot += d;
    }

    /// Reset the camera to identity — frame rect back on the document rect.
    /// Undoable.
    pub fn reset_camera(&mut self) {
        self.structural_edit(false, |p| p.camera = Camera::default());
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
        let layer = self.project.current_layer;
        let before_size = self.active_layer_cell_size();
        if (w, h) == before_size {
            return;
        }
        let before: Vec<(CellId, Canvas)> = self
            .project
            .layer_cell_ids(layer)
            .into_iter()
            .filter_map(|id| self.project.cell(id).map(|c| (id, c.clone())))
            .collect();
        self.project.expand_layer_canvas(layer, w, h);
        self.history.push(undo::Command::LayerCanvasResize {
            layer,
            before_size,
            after_size: (w, h),
            before,
        });
        self.mark_all_dirty();
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
        let Some(l) = self.project.layers.get_mut(li) else {
            return;
        };
        if l.locked {
            return;
        }
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
            BgJob::GifDecode(rx) => match rx.try_recv() {
                Ok(res) => {
                    self.bg_label = None;
                    self.on_gif_decoded(res);
                    None
                }
                Err(TryRecvError::Empty) => Some(BgJob::GifDecode(rx)),
                Err(TryRecvError::Disconnected) => {
                    self.bg_label = None;
                    log::error!("GIF decode worker died");
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

    fn on_gif_decoded(&mut self, res: Result<Vec<Canvas>>) {
        match res {
            Ok(frames) if !frames.is_empty() => {
                let total = frames.len();
                self.preview_tex.clear();
                self.preview_rx = None;
                self.import_range = Some(ImportRangeState {
                    source: ImportSource::Gif(frames),
                    total,
                    start: 0,
                    end: total - 1,
                });
                self.show_import_range = true;
            }
            Ok(_) => log::error!("GIF has no frames"),
            Err(e) => log::error!("GIF decode failed: {e:#}"),
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
    /// source frame `idx`. GIF previews are built instantly from decoded frames;
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
                    ImportSource::Gif(frames) => match frames.get(idx) {
                        Some(c) => Act::Have(preview_color_image(c.width, c.height, &c.pixels)),
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
            self.cell_dirty.entry(id).or_insert(true);
        }
    }

    /// Upload any dirty cells used by the composite this frame
    /// (current + onion neighbours, across all visible layers).
    /// During active strokes the full-buffer upload is skipped entirely so that
    /// high-resolution canvases (4K+) don't lag; visual feedback is provided
    /// via egui shape overlay in `paint_canvas`. The texture is refreshed on
    /// the first frame after the stroke ends.
    pub fn sync_textures(&mut self, ctx: &egui::Context) {
        // Release last frame's discarded ghosts here, before anything paints
        // this frame: the meshes that referenced them were submitted a frame
        // ago, so freeing now can't invalidate a texture mid-submit.
        self.retired_textures.clear();
        self.ensure_cell_tracking();

        // During an active stroke, stream only the flushed sub-rect to the
        // GPU as a partial texture update — exact WYSIWYG feedback without
        // full-buffer uploads on large canvases. A full re-upload happens on
        // the first frame after the stroke ends (cell marked dirty).
        if self.stroke.is_some() {
            let Some(target) = self.stroke_target else {
                return;
            };
            if self.cell_textures.contains_key(&target) {
                if let (Some(rect), Some(c)) =
                    (self.preview_upload_rect.take(), self.project.cell(target))
                {
                    let w = rect.max_x.saturating_sub(rect.min_x) as usize;
                    let h = rect.max_y.saturating_sub(rect.min_y) as usize;
                    if w > 0 && h > 0 {
                        let mut buf = Vec::with_capacity(w * h * 4);
                        for y in rect.min_y..rect.max_y {
                            let row = ((y * c.width + rect.min_x) * 4) as usize;
                            buf.extend_from_slice(&c.pixels[row..row + w * 4]);
                        }
                        let img = premultiplied_image([w, h], &buf);
                        if let Some(tex) = self.cell_textures.get_mut(&target) {
                            tex.set_partial(
                                [rect.min_x as usize, rect.min_y as usize],
                                img,
                                TextureOptions::LINEAR,
                            );
                        }
                    }
                }
                return;
            }
            // No texture for the target cell yet — fall through so the full
            // upload below creates it.
            self.preview_upload_rect = None;
        }

        let mut needed: Vec<CellId> = Vec::new();
        let cur = self.project.current_frame;

        for layer in &self.project.layers {
            if !layer.visible {
                continue;
            }
            if let Some(id) = layer.resolve(cur) {
                if !needed.contains(&id) {
                    needed.push(id);
                }
            }
        }

        // Onion ghosts come from the same walker the canvas draws with, so the
        // uploaded set can never drift from what gets painted — with drawing
        // stepping the ghosts can sit well outside `cur ± prev/next`.
        let ghosts: Vec<(CellId, [u8; 3])> = [OnionDirection::Prev, OnionDirection::Next]
            .into_iter()
            .flat_map(|dir| {
                let tint = self.onion.tint_rgb(dir);
                self.onion_steps(dir)
                    .into_iter()
                    .map(move |s| (s.cell, tint))
            })
            .collect();
        for (id, _) in &ghosts {
            if !needed.contains(id) {
                needed.push(*id);
            }
        }

        for id in needed {
            let dirty = self.cell_dirty.get(&id).copied().unwrap_or(true);
            let has_tex = self.cell_textures.contains_key(&id);
            if !dirty && has_tex {
                continue;
            }
            let Some(c) = self.project.cell(id) else {
                continue;
            };
            let dims = [c.width as usize, c.height as usize];
            let image = premultiplied_image(dims, &c.pixels);
            // Expanding a layer's canvas resizes cells under their textures, so
            // only reuse a handle whose dimensions still match; otherwise
            // allocate a fresh one.
            let reusable = self
                .cell_textures
                .get(&id)
                .is_some_and(|t| t.size() == dims);
            if reusable {
                if let Some(tex) = self.cell_textures.get_mut(&id) {
                    tex.set(image, TextureOptions::LINEAR);
                }
            } else {
                let tex = ctx.load_texture(format!("cell_{id}"), image, TextureOptions::LINEAR);
                self.cell_textures.insert(id, tex);
            }
            self.cell_dirty.insert(id, false);
        }

        // The floating selection's own texture. Rebuilt only when the pixels
        // change: a move shifts the quad, not the texture.
        match &self.selection {
            Some(sel) if self.sel_tex_stale || self.selection_tex.is_none() => {
                let dims = [sel.mask.w as usize, sel.mask.h as usize];
                let image = premultiplied_image(dims, &sel.pixels);
                match self.selection_tex.as_mut() {
                    Some(tex) if tex.size() == dims => tex.set(image, TextureOptions::LINEAR),
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

        // Ghost silhouettes for the onion cells. Bounded to the ghosted cells:
        // each one is a second full-size texture, which matters on 4K canvases.
        let stale: Vec<CellId> = self
            .ghost_textures
            .keys()
            .copied()
            .filter(|id| !ghosts.iter().any(|(g, _)| g == id))
            .collect();
        for id in stale {
            self.retire_ghost(id);
        }
        for (id, tint) in ghosts {
            let dims = match self.project.cell(id) {
                Some(c) => [c.width as usize, c.height as usize],
                None => continue,
            };
            let fresh = !self.ghost_stale.contains(&id)
                && self
                    .ghost_textures
                    .get(&id)
                    .is_some_and(|(t, tex)| *t == tint && tex.size() == dims);
            if fresh {
                continue;
            }
            let Some(c) = self.project.cell(id) else {
                continue;
            };
            let image = ghost_image(dims, &c.pixels, tint);
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
                    *t = tint;
                }
                self.ghost_stale.remove(&id);
            } else {
                self.retire_ghost(id);
                let tex = ctx.load_texture(format!("ghost{id}"), image, TextureOptions::LINEAR);
                self.ghost_textures.insert(id, (tint, tex));
                self.ghost_stale.remove(&id);
            }
        }
    }

    /// Onion ghosts for the active layer at the current frame. Shared by the
    /// texture sync and the canvas so both agree on which cells are ghosted.
    pub fn onion_steps(&self, dir: OnionDirection) -> Vec<OnionStep> {
        // Ghosts are for judging a pose against its neighbours, which playback
        // already shows; skipping them here also keeps the per-frame silhouette
        // rebuild out of the playback loop.
        if self.playback.playing {
            return Vec::new();
        }
        let Some(layer) = self.project.layers.get(self.project.current_layer) else {
            return Vec::new();
        };
        self.onion.steps(
            layer,
            self.project.current_frame,
            self.project.frame_count,
            dir,
        )
    }

    pub fn pointer_down(&mut self, sample: PointerSample) {
        self.playback.stop();
        if let Some(layer) = self.project.layers.get(self.project.current_layer) {
            if layer.locked || layer.reference {
                return;
            }
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
        let target = self.project.ensure_active_cell();
        self.stroke_target = Some(target);

        // Snapshot pre-stroke state so undo can roll back the dirty sub-rect.
        self.stroke_pre_pixels = Some(self.project.cells[target].pixels.clone());
        self.project.cells[target].dirty = None;

        if self.tool == ActiveTool::Fill {
            let opts = crate::tools::fill::FillOptions {
                tolerance: self.brush.fill_tolerance,
                color: self.brush.color,
                expand: self.brush.fill_expand,
            };
            // Owned, so the immutable project borrow ends before `cell_mut`.
            let (cw, ch) = {
                let c = &self.project.cells[target];
                (c.width, c.height)
            };
            let boundary = self.fill_boundary(self.project.current_layer, cw, ch);
            if let Some(c) = self.project.cell_mut(target) {
                crate::tools::fill::flood(
                    c,
                    boundary.as_ref(),
                    sample.x.round() as i32,
                    sample.y.round() as i32,
                    opts,
                );
            }
            self.commit_undo(target);
            self.mark_dirty(target);
            self.stroke = None;
            self.stroke_target = None;
            return;
        }

        if self.tool == ActiveTool::Lasso {
            // Pressing inside an existing selection moves it; anywhere else
            // commits it and starts a new lasso.
            if self
                .selection
                .as_ref()
                .is_some_and(|s| s.hit(sample.x, sample.y))
            {
                self.sel_drag = Some((sample.x, sample.y));
                self.stroke = None;
                self.stroke_pre_pixels = None;
                return;
            }
            self.commit_selection();
            // Collect the path; the enclosed pixels become a selection on
            // pointer-up. Preview is drawn by egui shapes in `paint_canvas`.
            self.lasso = Some(vec![(sample.x, sample.y)]);
            self.stroke = None;
            self.stroke_pre_pixels = None;
            return;
        }

        if self.tool == ActiveTool::Shape {
            // Anchor the shape; it is rasterised into the cell on pointer-up.
            // Preview is drawn by egui shapes in `paint_canvas` until then.
            self.shape_drag = Some(ShapeDrag {
                kind: self.brush.shape_kind,
                start: (sample.x, sample.y),
                end: (sample.x, sample.y),
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

        // Resolve the radius and the view scale once, here: a stroke must not
        // change width or smoothing behaviour partway through if the view moves.
        let mut brush = self.brush.clone();
        brush.radius = self.effective_radius();
        let mut builder =
            StrokeBuilder::new(brush, self.tool, self.cell_view_scale(), self.smoothing);
        builder.push(sample);
        if let (Some(pre), Some(c)) = (
            self.stroke_pre_pixels.as_deref(),
            self.project.cell_mut(target),
        ) {
            if let Some(r) = builder.flush(c, &mut self.stroke_ws, pre) {
                self.preview_upload_rect = Some(union_rect(self.preview_upload_rect, r));
            }
        }
        self.stroke = Some(builder);
        self.mark_dirty(target);
    }

    pub fn pointer_move(&mut self, sample: PointerSample) {
        let Some(target) = self.stroke_target else {
            return;
        };
        if let Some(drag) = &mut self.shape_drag {
            drag.end = (sample.x, sample.y);
            return;
        }
        if let Some(last) = self.sel_drag {
            let (dx, dy) = (
                (sample.x - last.0).round() as i32,
                (sample.y - last.1).round() as i32,
            );
            if dx != 0 || dy != 0 {
                self.sel_drag = Some((sample.x, sample.y));
                self.nudge_selection(dx, dy);
            }
            return;
        }
        if let Some(path) = &mut self.lasso {
            // Decimate: a pen emits far more samples than the polygon needs,
            // and every extra vertex costs an edge test on every scanline.
            let far = path
                .last()
                .map(|&(x, y)| (sample.x - x).hypot(sample.y - y) >= 1.0)
                .unwrap_or(true);
            if far {
                path.push((sample.x, sample.y));
            }
            return;
        }
        let Some(builder) = &mut self.stroke else {
            return;
        };
        builder.push(sample);
        if let (Some(pre), Some(c)) = (
            self.stroke_pre_pixels.as_deref(),
            self.project.cell_mut(target),
        ) {
            if let Some(r) = builder.flush(c, &mut self.stroke_ws, pre) {
                self.preview_upload_rect = Some(union_rect(self.preview_upload_rect, r));
            }
        }
        self.mark_dirty(target);
    }

    pub fn pointer_up(&mut self) {
        let Some(target) = self.stroke_target.take() else {
            self.stroke = None;
            self.shape_drag = None;
            self.lasso = None;
            self.stroke_pre_pixels = None;
            self.preview_upload_rect = None;
            return;
        };
        if let Some(path) = self.lasso.take() {
            // The lasso now *selects* rather than erasing outright — Delete on
            // the selection is the erase.
            self.stroke_pre_pixels = None;
            self.preview_upload_rect = None;
            self.begin_selection(target, path);
            return;
        }
        if let Some(drag) = self.shape_drag.take() {
            // Rasterise the final shape now; undo records the dirty rect below.
            let mut brush = self.brush.clone();
            brush.radius = self.effective_radius();
            let (cw, ch) = {
                let c = &self.project.cells[target];
                (c.width, c.height)
            };
            self.stroke_ws.begin(cw, ch, &brush);
            if let (Some(pre), Some(c)) = (
                self.stroke_pre_pixels.as_deref(),
                self.project.cell_mut(target),
            ) {
                crate::tools::shape::rasterize(
                    c,
                    &mut self.stroke_ws,
                    pre,
                    drag.kind,
                    drag.start,
                    drag.end,
                    &brush,
                );
            }
            self.mark_dirty(target);
        } else if let Some(mut builder) = self.stroke.take() {
            if let (Some(pre), Some(c)) = (
                self.stroke_pre_pixels.as_deref(),
                self.project.cell_mut(target),
            ) {
                builder.finish(c, &mut self.stroke_ws, pre);
            }
            self.mark_dirty(target);
        }
        // Any pending partial upload is superseded by the full re-upload the
        // dirty flag triggers now that the stroke ended.
        self.preview_upload_rect = None;
        self.commit_undo(target);
    }

    /// Push a PixelPatch covering the dirty rect accumulated since the last
    /// `stroke_pre_pixels` snapshot.
    fn commit_undo(&mut self, cell: CellId) {
        let Some(pre) = self.stroke_pre_pixels.take() else {
            return;
        };
        let canvas = &self.project.cells[cell];
        let Some(rect) = canvas.dirty else {
            return;
        };
        let w = rect.max_x.saturating_sub(rect.min_x);
        let h = rect.max_y.saturating_sub(rect.min_y);
        if w == 0 || h == 0 {
            return;
        }

        let before = subrect_from_buffer(&pre, canvas.width, rect.min_x, rect.min_y, w, h);
        let after = undo::snapshot_subrect(canvas, rect.min_x, rect.min_y, w, h);

        // Skip recording no-op strokes (before == after).
        if before == after {
            return;
        }

        self.history.push(undo::Command::PixelPatch {
            cell,
            x: rect.min_x,
            y: rect.min_y,
            w,
            h,
            before,
            after,
        });
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
        self.stroke = None;
        self.stroke_target = None;
        self.shape_drag = None;
        self.lasso = None;
        self.preview_upload_rect = None;
        self.commit_selection();
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

    fn active_layer_is_reference(&self) -> bool {
        self.project
            .layers
            .get(self.project.current_layer)
            .map(|l| l.reference)
            .unwrap_or(false)
    }

    // --- Lasso selection ---

    /// Write a floating selection back into its cell and drop it. Safe to call
    /// when there is nothing selected.
    ///
    /// One undo entry: the stamp is snapshotted and committed on its own, so an
    /// undo of a move puts the pixels back where the lift left them, and a
    /// second undo restores the lift.
    pub fn commit_selection(&mut self) {
        let Some(sel) = self.selection.take() else {
            return;
        };
        self.sel_drag = None;
        if let Some(old) = self.selection_tex.take() {
            self.retired_textures.push(old);
        }
        // Never lifted means never moved: the cell was left untouched, so there
        // is nothing to write back and nothing to record.
        if !sel.lifted {
            return;
        }
        let cell = sel.cell;
        let Some(canvas) = self.project.cell(cell) else {
            return;
        };
        self.stroke_pre_pixels = Some(canvas.pixels.clone());
        if let Some(c) = self.project.cell_mut(cell) {
            c.dirty = None;
            sel.stamp(c);
        }
        self.mark_dirty(cell);
        self.commit_undo(cell);
    }

    /// Start a selection from a finished lasso path on `cell`.
    fn begin_selection(&mut self, cell: CellId, path: Vec<(f32, f32)>) {
        let Some(canvas) = self.project.cell(cell) else {
            return;
        };
        let Some(mask) = crate::tools::lasso::coverage(&path, canvas.width, canvas.height) else {
            return;
        };
        self.selection = Some(Selection::new(cell, canvas, mask, path));
        self.sel_tex_stale = true;
    }

    /// Erase the selected pixels and drop the selection.
    pub fn delete_selection(&mut self) {
        let Some(sel) = self.selection.take() else {
            return;
        };
        self.sel_drag = None;
        if let Some(old) = self.selection_tex.take() {
            self.retired_textures.push(old);
        }
        // Already lifted: the pixels left the cell when the move started, so
        // dropping the float *is* the delete, and the lift's own undo entry
        // covers it.
        if sel.lifted {
            return;
        }
        let cell = sel.cell;
        let Some(canvas) = self.project.cell(cell) else {
            return;
        };
        self.stroke_pre_pixels = Some(canvas.pixels.clone());
        if let Some(c) = self.project.cell_mut(cell) {
            c.dirty = None;
            crate::tools::lasso::erase_masked(c, &sel.mask);
        }
        self.mark_dirty(cell);
        self.commit_undo(cell);
    }

    /// Copy the floating pixels to the selection clipboard.
    pub fn copy_selection(&mut self) {
        if let Some(sel) = &self.selection {
            self.pixel_clip = Some((sel.mask.clone(), sel.pixels.clone()));
        }
    }

    /// Drop the clipboard pixels onto the current cell as a new floating
    /// selection, so it can be positioned before it commits. Works across
    /// frames and layers, since it targets whatever cell is active now.
    pub fn paste_selection(&mut self) {
        let Some((mask, pixels)) = self.pixel_clip.clone() else {
            return;
        };
        if self.active_layer_locked() || self.active_layer_is_reference() {
            return;
        }
        self.commit_selection();
        let cell = self.project.ensure_active_cell();
        let path = crate::tools::selection::outline_rect(&mask);
        // Already lifted: these pixels came from the clipboard, not from this
        // cell, so there is no source region to erase.
        self.selection = Some(Selection {
            cell,
            mask,
            pixels,
            offset: (0, 0),
            path,
            lifted: true,
        });
        self.sel_tex_stale = true;
    }

    /// Nudge a floating selection by whole pixels (arrow keys).
    pub fn nudge_selection(&mut self, dx: i32, dy: i32) {
        let Some(sel) = self.selection.as_mut() else {
            return;
        };
        let cell = sel.cell;
        let lift = !sel.lifted;
        sel.offset.0 += dx;
        sel.offset.1 += dy;
        if lift {
            self.lift_selection_source(cell);
        }
    }

    /// Erase the source region behind a selection and record it, once.
    fn lift_selection_source(&mut self, cell: CellId) {
        let Some(canvas) = self.project.cell(cell) else {
            return;
        };
        self.stroke_pre_pixels = Some(canvas.pixels.clone());
        let mut sel = match self.selection.take() {
            Some(s) => s,
            None => return,
        };
        if let Some(c) = self.project.cell_mut(cell) {
            c.dirty = None;
            sel.lift_source(c);
        }
        self.selection = Some(sel);
        self.mark_dirty(cell);
        self.commit_undo(cell);
    }

    /// Commit a floating selection whose cell is no longer the active one —
    /// scrubbing to another frame must not leave pixels hovering over a
    /// drawing they do not belong to.
    fn commit_selection_if_orphaned(&mut self) {
        let Some(sel) = &self.selection else {
            return;
        };
        if self.project.resolved_current() != Some(sel.cell) {
            self.commit_selection();
        }
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
        let idx = self.tool.idx();
        self.tool_brushes[idx].color = c;
    }

    /// Pin the current brush colour. Re-pinning a colour already held is a
    /// no-op rather than a duplicate.
    pub fn pin_swatch(&mut self) {
        let c = self.brush.color;
        let rgb = [c[0], c[1], c[2]];
        if self.palette.contains(&rgb) {
            return;
        }
        self.palette.push(rgb);
        if self.palette.len() > Self::MAX_SWATCHES {
            self.palette.remove(0);
        }
    }

    /// Commit a sampled colour: apply it to every tool, restore the drawing tool
    /// that was active before picking, and leave pick mode.
    pub fn commit_screen_pick(&mut self, color: [u8; 4]) {
        // Apply to the active brush and every tool's stored colour so the sample
        // sticks across tool switches.
        for b in &mut self.tool_brushes {
            b.color = color;
        }
        let tool = self.screen_pick_return_tool;
        self.tool = tool;
        self.brush = self.tool_brushes[tool.idx()].clone();
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

        if dst_xform.is_identity() && cell_w == pw && cell_h == ph {
            return Some(doc);
        }

        // The destination cell is transformed, so walk its pixels and pull the
        // matching document-space sample for each.
        let mut out = Canvas::new(cell_w, cell_h);
        let (cw, ch) = (cell_w as f32, cell_h as f32);
        let (pwf, phf) = (pw as f32, ph as f32);
        for v in 0..cell_h {
            for u in 0..cell_w {
                let (dx, dy) =
                    dst_xform.cell_to_doc(u as f32 + 0.5, v as f32 + 0.5, cw, ch, pwf, phf);
                let (sx, sy) = (dx - 0.5, dy - 0.5);
                if sx < -0.5 || sy < -0.5 || sx > pwf - 0.5 || sy > phf - 0.5 {
                    continue;
                }
                let px = crate::io::composite::sample_bilinear(&doc, sx, sy);
                let i = ((v * cell_w + u) * 4) as usize;
                out.pixels[i..i + 4].copy_from_slice(&px);
            }
        }
        Some(out)
    }

    /// Remember the layer we just came from, so `Action::LayerLast` can jump
    /// back. Run once per frame: `current_layer` is assigned from eight
    /// different places (panel clicks, x-sheet clicks, add/delete/move, undo,
    /// import), and diffing here catches all of them without touching any.
    ///
    /// A change in layer *count* drops the link instead of remapping it — after
    /// an add or delete the stored index may mean a different layer, and
    /// silently jumping to the wrong one is worse than not jumping.
    fn track_layer_change(&mut self) {
        let now = (self.project.current_layer, self.project.layers.len());
        if now.1 != self.layer_watch.1 {
            self.prev_layer = None;
        } else if now.0 != self.layer_watch.0 {
            self.prev_layer = Some(self.layer_watch.0);
        }
        self.layer_watch = now;
    }

    /// Jump to the layer selected before the current one. No-op when there
    /// isn't one yet or it has gone stale.
    pub fn goto_last_layer(&mut self) {
        let Some(prev) = self.prev_layer else { return };
        if prev < self.project.layers.len() && prev != self.project.current_layer {
            self.project.current_layer = prev;
        }
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
        match action {
            Action::ToolPencil => {
                self.commit_selection();
                self.tool_brushes[self.tool.idx()] = self.brush.clone();
                self.tool = ActiveTool::Pencil;
                self.brush = self.tool_brushes[ActiveTool::Pencil.idx()].clone();
            }
            Action::ToolInk => {
                self.tool_brushes[self.tool.idx()] = self.brush.clone();
                self.tool = ActiveTool::Ink;
                self.brush = self.tool_brushes[ActiveTool::Ink.idx()].clone();
            }
            Action::ToolEraser => {
                self.tool_brushes[self.tool.idx()] = self.brush.clone();
                self.tool = ActiveTool::Eraser;
                self.brush = self.tool_brushes[ActiveTool::Eraser.idx()].clone();
            }
            Action::ToolFill => {
                self.tool_brushes[self.tool.idx()] = self.brush.clone();
                self.tool = ActiveTool::Fill;
                self.brush = self.tool_brushes[ActiveTool::Fill.idx()].clone();
            }
            // Retired: the in-canvas eyedropper was folded into PickScreenColor,
            // which samples the canvas as well as everything behind the window.
            // The variant survives only so an old shortcuts.toml still parses.
            Action::ToolColorPicker => {}
            Action::PickScreenColor => {
                if self.screen_pick {
                    self.end_screen_pick();
                } else {
                    self.begin_screen_pick();
                }
            }
            Action::ToolShape => {
                self.tool_brushes[self.tool.idx()] = self.brush.clone();
                self.tool = ActiveTool::Shape;
                self.brush = self.tool_brushes[ActiveTool::Shape.idx()].clone();
            }
            Action::ToolTracker => {
                self.tool_brushes[self.tool.idx()] = self.brush.clone();
                self.tool = ActiveTool::Tracker;
                self.brush = self.tool_brushes[ActiveTool::Tracker.idx()].clone();
            }
            Action::ToolLasso => {
                self.tool_brushes[self.tool.idx()] = self.brush.clone();
                self.tool = ActiveTool::Lasso;
                self.brush = self.tool_brushes[ActiveTool::Lasso.idx()].clone();
            }
            Action::PlayPause => {
                let now = 0.0; // refreshed by playback.tick on next frame
                let _ = now;
                self.playback.playing = !self.playback.playing;
            }
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
            Action::LayerAdd => self.structural_edit(false, Project::add_layer),
            Action::LayerDelete => self.structural_edit(false, Project::delete_layer),
            Action::LayerToggleVisible => {
                if let Some(l) = self.project.layers.get_mut(self.project.current_layer) {
                    l.visible = !l.visible;
                }
            }
            Action::LayerLast => self.goto_last_layer(),
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
            Action::ClearCell => {
                if let Some(id) = self.project.resolved_current() {
                    if let Some(c) = self.project.cell_mut(id) {
                        c.clear();
                    }
                    self.mark_dirty(id);
                }
            }
            Action::PasteImage => self.paste_image_as_background(),
            Action::ToggleCheckerBg => self.show_checker = !self.show_checker,
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
            Action::SelectionDeselect => self.commit_selection(),
            Action::SaveProject => self.save_project(),
            Action::SaveProjectAs => self.save_project_as(),
            Action::OpenProject => match crate::io::project_file::load_dialog() {
                Ok(Some((p, path))) => self.load_project(p, Some(path)),
                Ok(None) => {}
                Err(e) => log::error!("Open project failed: {e:#}"),
            },
            // Canvas nav gestures are modifier-only drag binds, handled directly
            // in the canvas input code — never dispatched as press actions.
            Action::CanvasZoom | Action::CanvasPan | Action::CanvasRotate => {}
            Action::LayerTransformToggle => self.layer_xform = !self.layer_xform,
            Action::TransformKeyAdd => self.add_transform_key(),
            Action::TransformKeyDelete => self.delete_transform_key(),
            Action::CameraEditToggle => {
                self.camera_edit = !self.camera_edit;
                // The two retarget modes claim the same drag gestures, so only
                // one can be live at a time.
                if self.camera_edit {
                    self.layer_xform = false;
                }
            }
            Action::CameraKeyAdd => self.add_camera_key(),
            Action::CameraKeyDelete => self.delete_camera_key(),
            Action::CameraLookThrough => self.camera_look_through = !self.camera_look_through,
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
        match crate::io::project_file::save_to(&self.project, &path) {
            Ok(()) => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("project")
                    .to_string();
                self.project_path = Some(path);
                self.save_toast = Some((name, Instant::now() + Self::TOAST_TTL));
                self.save_error = None;
            }
            Err(e) => {
                log::error!("Save project failed: {e:#}");
                self.save_error = Some(format!("{e:#}"));
            }
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
        self.project = project;
        self.project_path = path;
        self.save_toast = None;
        self.save_error = None;
        self.retire_cell_textures();
        self.retire_all_ghosts();
        self.cell_dirty.clear();
        for id in 0..self.project.cells.len() {
            self.cell_dirty.insert(id, true);
        }
        self.stroke = None;
        self.stroke_target = None;
        self.shape_drag = None;
        self.lasso = None;
        self.stroke_pre_pixels = None;
        self.preview_upload_rect = None;
        self.history = History::default();
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
    }

    pub fn undo(&mut self) {
        match self.history.undo(&mut self.project) {
            Some(undo::Touched::Cell(id)) => self.mark_dirty(id),
            Some(undo::Touched::All) => self.mark_all_dirty(),
            None => {}
        }
    }

    pub fn redo(&mut self) {
        match self.history.redo(&mut self.project) {
            Some(undo::Touched::Cell(id)) => self.mark_dirty(id),
            Some(undo::Touched::All) => self.mark_all_dirty(),
            None => {}
        }
    }
}

impl eframe::App for AppState {
    /// Called by eframe on exit and every `auto_save_interval` (30s). Panel
    /// geometry and collapse state are saved separately via
    /// `persist_egui_memory`, which defaults to true.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
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
                loop_timeline: self.loop_timeline,
                smoothing: self.smoothing,
                tool_brushes: Some(self.tool_brushes.to_vec()),
                palette: self.palette.clone(),
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

        self.pen.poll(ctx.input(|i| i.pointer.any_down()));

        // Shortcut rebind capture: when an Action is "rebinding", the next
        // key press becomes its new combo and rebind mode ends.
        if let Some(action) = self.rebinding {
            let captured = ctx.input(|i| {
                use egui::Key;
                let mods = i.modifiers;
                // Ignore modifier-only key presses (Shift on its own etc.).
                for &key in &[
                    Key::A,
                    Key::B,
                    Key::C,
                    Key::D,
                    Key::E,
                    Key::F,
                    Key::G,
                    Key::H,
                    Key::I,
                    Key::J,
                    Key::K,
                    Key::L,
                    Key::M,
                    Key::N,
                    Key::O,
                    Key::P,
                    Key::Q,
                    Key::R,
                    Key::S,
                    Key::T,
                    Key::U,
                    Key::V,
                    Key::W,
                    Key::X,
                    Key::Y,
                    Key::Z,
                    Key::Num0,
                    Key::Num1,
                    Key::Num2,
                    Key::Num3,
                    Key::Num4,
                    Key::Num5,
                    Key::Num6,
                    Key::Num7,
                    Key::Num8,
                    Key::Num9,
                    Key::OpenBracket,
                    Key::CloseBracket,
                    Key::Semicolon,
                    Key::Comma,
                    Key::Period,
                    Key::Slash,
                    Key::Backslash,
                    Key::Minus,
                    Key::Equals,
                    Key::Space,
                    Key::Tab,
                    Key::Backspace,
                    Key::Backtick,
                    Key::F1,
                    Key::F2,
                    Key::F3,
                    Key::F4,
                    Key::F5,
                    Key::F6,
                    Key::F7,
                    Key::F8,
                    Key::F9,
                    Key::F10,
                    Key::F11,
                    Key::F12,
                    Key::ArrowUp,
                    Key::ArrowDown,
                    Key::ArrowLeft,
                    Key::ArrowRight,
                ] {
                    if i.key_pressed(key) {
                        return Some(crate::input::shortcuts::KeyCombo {
                            key: Some(key),
                            ctrl: mods.ctrl,
                            shift: mods.shift,
                            alt: mods.alt,
                        });
                    }
                }
                None
            });
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
            // Tab (TogglePanels) and Backtick (ToggleCheckerBg) are always
            // allowed through so the user can recover panels if focus is stuck.
            if ctx.memory(|m| m.focused()).is_none() {
                let actions = self.shortcuts.poll_actions(ctx);
                for a in actions {
                    self.dispatch(a);
                }
                // Arrow keys nudge a floating selection by a pixel. Not bound
                // actions: they only mean anything while something is selected,
                // and stealing the arrows outright would be worse.
                if self.selection.is_some() {
                    let (mut dx, mut dy) = (0, 0);
                    ctx.input(|i| {
                        dx += i.key_pressed(egui::Key::ArrowRight) as i32;
                        dx -= i.key_pressed(egui::Key::ArrowLeft) as i32;
                        dy += i.key_pressed(egui::Key::ArrowDown) as i32;
                        dy -= i.key_pressed(egui::Key::ArrowUp) as i32;
                    });
                    if dx != 0 || dy != 0 {
                        self.nudge_selection(dx, dy);
                    }
                }
            } else {
                if ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
                    self.dispatch(Action::TogglePanels);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Backtick)) {
                    self.dispatch(Action::ToggleCheckerBg);
                }
            }
        }

        let now = ctx.input(|i| i.time);
        if self
            .playback
            .tick(&mut self.project, now, self.loop_timeline)
        {
            ctx.request_repaint();
        }
        if self.playback.playing {
            ctx.request_repaint_after(std::time::Duration::from_secs_f32(
                1.0 / self.project.fps.max(1.0),
            ));
        }

        // Keep the active layer's transform edit buffer synced while scrubbing.
        self.sync_active_transform_buffer();
        self.commit_selection_if_orphaned();
        self.sync_camera_buffer();

        // Advance background import jobs / preview fetches without blocking.
        self.poll_bg_jobs();
        self.poll_preview(ctx);
        if self.bg_job.is_some() || self.preview_rx.is_some() {
            ctx.request_repaint();
        }

        self.sync_textures(ctx);
        ui::shell::draw(self, ctx);

        // If panels are hidden, clear any stale keyboard focus so shortcuts
        // (especially Tab → TogglePanels) work on the next press.
        if !self.show_panels {
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
    }
}

/// Build a `ColorImage` by premultiplying straight-alpha RGBA8 in gamma
/// (sRGB) space. egui's `from_rgba_unmultiplied` premultiplies in linear
/// space, which over-brightens fractional-alpha AA edges once egui blends
/// them in gamma space — light strokes get a white rim.
pub(crate) fn premultiplied_image(size: [usize; 2], rgba: &[u8]) -> ColorImage {
    let pixels = rgba
        .chunks_exact(4)
        .map(|p| match p[3] {
            0 => Color32::TRANSPARENT,
            255 => Color32::from_rgb(p[0], p[1], p[2]),
            a => {
                let m = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
                Color32::from_rgba_premultiplied(m(p[0]), m(p[1]), m(p[2]), a)
            }
        })
        .collect();
    ColorImage { size, pixels }
}

/// Build a `ColorImage` of a cell as a flat silhouette in `tint`: the source
/// alpha is kept as the shape and the RGB is replaced wholesale.
///
/// This is what makes onion skins actually read as blue-past / red-future.
/// Painting the cell texture with a tinted vertex color only *multiplies*, and
/// black line art times any tint is still black.
pub(crate) fn ghost_image(size: [usize; 2], rgba: &[u8], tint: [u8; 3]) -> ColorImage {
    // Premultiplied in gamma space, matching `premultiplied_image`.
    let pixels = rgba
        .chunks_exact(4)
        .map(|p| match p[3] {
            0 => Color32::TRANSPARENT,
            255 => Color32::from_rgb(tint[0], tint[1], tint[2]),
            a => {
                let m = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
                Color32::from_rgba_premultiplied(m(tint[0]), m(tint[1]), m(tint[2]), a)
            }
        })
        .collect();
    ColorImage { size, pixels }
}

/// Build a small (≤360px wide) preview `ColorImage` from RGBA pixels.
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

/// Downscale a canvas (aspect-preserving) so neither side exceeds `max`. No-op
/// if it already fits. Used to keep imported cells within the GPU texture limit.
fn cap_canvas(c: Canvas, max: u32) -> Canvas {
    if c.width <= max && c.height <= max {
        return c;
    }
    let s = (max as f32 / c.width as f32).min(max as f32 / c.height as f32);
    let nw = ((c.width as f32 * s).floor() as u32).max(1);
    let nh = ((c.height as f32 * s).floor() as u32).max(1);
    let Some(img) = image::RgbaImage::from_raw(c.width, c.height, c.pixels) else {
        return Canvas::new(nw, nh);
    };
    let resized = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Lanczos3);
    let mut out = Canvas::new(nw, nh);
    out.pixels = resized.into_raw();
    out
}

/// Slice a sub-rectangle (RGBA8) out of a buffer of width `full_w`.
fn subrect_from_buffer(buf: &[u8], full_w: u32, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    let row_bytes = w as usize * 4;
    let mut out = vec![0u8; (w * h * 4) as usize];
    for row in 0..h as usize {
        let src_off = ((y as usize + row) * full_w as usize + x as usize) * 4;
        let dst_off = row * row_bytes;
        out[dst_off..dst_off + row_bytes].copy_from_slice(&buf[src_off..src_off + row_bytes]);
    }
    out
}
