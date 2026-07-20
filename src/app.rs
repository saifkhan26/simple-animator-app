//! Top-level application state. Wires project (timeline + layers), tools, UI.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use anyhow::Result;
use eframe::CreationContext;
use egui::{Color32, ColorImage, TextureHandle, TextureOptions};

use crate::doc::canvas::{Canvas, DirtyRect};
use crate::doc::layer::{CellId, TrackSample};
use crate::doc::project::Project;
use crate::doc::transform::Transform;
use crate::input::pointer::PointerSample;
use crate::input::shortcuts::{self, Action, ShortcutMap};
use crate::input::tablet::PenInput;
use crate::timeline::onion::OnionConfig;
use crate::timeline::playback::Playback;
use crate::tools::ribbon::{union_rect, StrokeWorkspace};
use crate::tools::stroke::StrokeBuilder;
use crate::tools::{ActiveTool, BrushSettings, ShapeKind};
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
#[derive(Clone, Copy)]
pub struct View {
    pub zoom: f32,
    pub pan: egui::Vec2,
    pub rotation: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            rotation: 0.0,
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

/// The tool panels (each rendered as a floating window).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelId {
    Tools,
    Brush,
    Layers,
    Onion,
    Xsheet,
    Timeline,
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
    /// Extracting the chosen video range, to be dropped on a new layer.
    VideoExtract {
        rx: Receiver<Result<Vec<Canvas>>>,
        name: String,
    },
    /// Encoding the project to an MP4 via ffmpeg.
    Mp4Export(Receiver<Result<()>>),
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

    /// GPU texture handle per CellId, lazily created.
    pub cell_textures: HashMap<CellId, TextureHandle>,
    /// Per-CellId dirty flag — re-upload on next sync.
    pub cell_dirty: HashMap<CellId, bool>,

    pub tool: ActiveTool,
    pub brush: BrushSettings,
    /// Per-tool brush settings preserved across tool switches.
    pub tool_brushes: [BrushSettings; 7],
    /// Previous drawing tool — used by ColorPicker to restore the tool after
    /// a colour sample.
    pub prev_tool: Option<ActiveTool>,
    pub stroke: Option<StrokeBuilder>,
    /// CellId being painted into during the current stroke.
    pub stroke_target: Option<CellId>,
    /// In-progress Shape-tool drag (preview only until pointer-up).
    pub shape_drag: Option<ShapeDrag>,

    /// Canvas view transform (zoom / pan / rotate).
    pub view: View,
    /// Active non-drawing canvas gesture for the current drag, if any.
    pub nav_drag: Option<NavKind>,

    pub playback: Playback,
    pub onion: OnionConfig,

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
    /// Master visibility of all floating panel windows. Tab toggles it.
    pub show_panels: bool,
    /// Minimal timeline bar shown when `show_panels` is false. Has its own
    /// toggle shortcut so the user can hide it too.
    pub show_mini_timeline: bool,

    pub show_new_project: bool,
    pub new_project_cfg: NewProjectConfig,

    /// MP4 export settings dialog visibility + backing config.
    pub show_mp4_export: bool,
    pub mp4_cfg: Mp4ExportConfig,

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

    /// One-shot guard: native window chrome (rounded corners/border) applied.
    window_styled: bool,

    /// Live screen colour-pick mode: while true, the pixel under the cursor
    /// (including apps behind our transparent backdrop) is sampled and the next
    /// tap commits it as the brush colour.
    pub screen_pick: bool,
    /// Backdrop opacity to restore when screen-pick mode ends (we force the
    /// backdrop transparent while picking so the desktop behind is visible).
    screen_pick_prev_opacity: f32,
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
        Self {
            project,
            cell_textures: HashMap::new(),
            cell_dirty,
            tool: ActiveTool::Pencil,
            brush: BrushSettings::default_pencil(),
            tool_brushes: [
                BrushSettings::default_pencil(),
                BrushSettings::default_ink(),
                BrushSettings::default_eraser(),
                BrushSettings::default_fill(),
                BrushSettings::default_pencil(),
                BrushSettings::default_shape(),
                // Tracker draws nothing; slot only keeps tool indexing safe.
                BrushSettings::default_shape(),
            ],
            prev_tool: None,
            stroke: None,
            stroke_target: None,
            shape_drag: None,
            view: View::default(),
            nav_drag: None,
            playback: Playback::default(),
            onion: OnionConfig::default(),
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
            show_panels: true,
            show_mini_timeline: true,
            show_new_project: false,
            new_project_cfg: NewProjectConfig::default(),
            show_mp4_export: false,
            mp4_cfg: Mp4ExportConfig::default(),
            show_import_range: false,
            import_range: None,
            layer_xform: false,
            nav_to_layer: false,
            layer_xform_before: None,
            xform_sync_last: None,
            tracker_two_points: false,
            tracker_pending_b: None,
            bg_job: None,
            bg_label: None,
            preview_tex: HashMap::new(),
            preview_rx: None,
            preview_clear_pending: false,
            max_tex,
            window_styled: false,
            screen_pick: false,
            screen_pick_prev_opacity: 1.0,
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
        self.cell_textures.clear();
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
            BrushSettings::default_pencil(),
            BrushSettings::default_shape(),
            BrushSettings::default_shape(),
        ];
        self.stroke = None;
        self.stroke_target = None;
        self.shape_drag = None;
        self.view = View::default();
        self.nav_drag = None;
        self.playback = Playback::default();
        self.onion = OnionConfig::default();
        self.bg_opacity = 1.0;
        self.bg_color = [0.12, 0.12, 0.13];
        self.show_checker = false;
        self.history = History::default();
        self.stroke_pre_pixels = None;
        self.preview_upload_rect = None;
        self.rebinding = None;
        self.layer_rename = None;
        self.show_settings = false;
        self.show_panels = true;
        self.show_mini_timeline = true;
        self.prev_tool = None;
        self.new_project_cfg = NewProjectConfig { width, height, fps };
        self.show_mp4_export = false;
        self.show_import_range = false;
        self.import_range = None;
        self.layer_xform = false;
        self.nav_to_layer = false;
        self.layer_xform_before = None;
        self.xform_sync_last = None;
        self.tracker_pending_b = None;
        self.bg_job = None;
        self.bg_label = None;
        self.preview_tex.clear();
        self.preview_rx = None;
    }

    pub fn make_sample(&self, x: f32, y: f32, t: f32) -> PointerSample {
        let pressure = self.pen.current_pressure().unwrap_or(1.0);
        PointerSample {
            x,
            y,
            pressure,
            tilt_x: 0.0,
            tilt_y: 0.0,
            t,
        }
    }

    pub fn mark_dirty(&mut self, id: CellId) {
        self.cell_dirty.insert(id, true);
    }

    /// Mark every cell for re-upload (used after structural undo/redo, where
    /// the whole timeline may have shifted).
    fn mark_all_dirty(&mut self) {
        for id in 0..self.project.cells.len() {
            self.cell_dirty.insert(id, true);
        }
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

        let baked: Vec<(usize, Canvas)> = bake_frames
            .into_iter()
            .map(|f| {
                let mut out = Canvas::new(pw, ph);
                for l in [below, top] {
                    let Some(id) = l.resolve(f) else { continue };
                    let Some(src) = p.cell(id) else { continue };
                    let xform = l.resolve_transform(f);
                    crate::io::composite::composite_layer(&mut out, src, &xform, l.opacity, pw, ph);
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
            for (f, canvas) in baked {
                let id = p.cells.len();
                p.cells.push(canvas);
                p.layers[bi].set_key(f, id);
            }
            p.layers.remove(li);
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
    pub fn start_mp4_export(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("MP4 video", &["mp4"])
            .set_file_name("animation.mp4")
            .set_title("Export MP4")
            .save_file()
        else {
            return;
        };
        let project = self.project.clone();
        let settings = crate::io::mp4_export::Mp4Settings {
            crf: self.mp4_cfg.crf,
            preset: MP4_PRESETS[self.mp4_cfg.preset_idx.min(MP4_PRESETS.len() - 1)],
        };
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(crate::io::mp4_export::export_to(&project, &path, &settings));
        });
        self.bg_job = Some(BgJob::Mp4Export(rx));
        self.bg_label = Some("Exporting MP4…");
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
            BgJob::Mp4Export(rx) => match rx.try_recv() {
                Ok(res) => {
                    self.bg_label = None;
                    if let Err(e) = res {
                        log::error!("MP4 export failed: {e:#}");
                    }
                    None
                }
                Err(TryRecvError::Empty) => Some(BgJob::Mp4Export(rx)),
                Err(TryRecvError::Disconnected) => {
                    self.bg_label = None;
                    log::error!("MP4 export worker died");
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
        let prev_n = if self.onion.enabled {
            self.onion.prev as usize
        } else {
            0
        };
        let next_n = if self.onion.enabled {
            self.onion.next as usize
        } else {
            0
        };
        let lo = cur.saturating_sub(prev_n);
        let hi = (cur + next_n + 1).min(self.project.frame_count);

        for layer in &self.project.layers {
            if !layer.visible {
                continue;
            }
            for f in lo..hi {
                if let Some(id) = layer.resolve(f) {
                    if !needed.contains(&id) {
                        needed.push(id);
                    }
                }
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
            let image = premultiplied_image([c.width as usize, c.height as usize], &c.pixels);
            if let Some(tex) = self.cell_textures.get_mut(&id) {
                tex.set(image, TextureOptions::LINEAR);
            } else {
                let tex = ctx.load_texture(format!("cell_{id}"), image, TextureOptions::LINEAR);
                self.cell_textures.insert(id, tex);
            }
            self.cell_dirty.insert(id, false);
        }
    }

    pub fn pointer_down(&mut self, sample: PointerSample) {
        self.playback.stop();
        if let Some(layer) = self.project.layers.get(self.project.current_layer) {
            if layer.locked || layer.reference {
                return;
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
            };
            if let Some(c) = self.project.cell_mut(target) {
                crate::tools::fill::flood(
                    c,
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

        if self.tool == ActiveTool::ColorPicker {
            let x = sample.x.round() as i32;
            let y = sample.y.round() as i32;
            let picked = self.pick_color(x, y);
            if let Some(color) = picked {
                self.brush.color = color;
                for b in &mut self.tool_brushes {
                    b.color = color;
                }
            }
            // Restore the previous drawing tool so the user can keep working
            // with the sampled colour immediately.
            if let Some(prev) = self.prev_tool.take() {
                self.tool_brushes[self.tool.idx()] = self.brush.clone();
                self.tool = prev;
                self.brush = self.tool_brushes[prev.idx()].clone();
            }
            self.stroke = None;
            self.stroke_target = None;
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
            .begin(cw, ch, self.brush.hardness, self.brush.grain);

        let mut builder = StrokeBuilder::new(self.brush.clone(), self.tool);
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
            self.stroke_pre_pixels = None;
            self.preview_upload_rect = None;
            return;
        };
        if let Some(drag) = self.shape_drag.take() {
            // Rasterise the final shape now; undo records the dirty rect below.
            let brush = self.brush.clone();
            let (cw, ch) = {
                let c = &self.project.cells[target];
                (c.width, c.height)
            };
            self.stroke_ws.begin(cw, ch, brush.hardness, brush.grain);
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

    /// Enter live screen-pick mode: force the backdrop transparent so the
    /// desktop behind shows through, and arm so the press that opened the mode
    /// is consumed before a tap commits.
    pub fn begin_screen_pick(&mut self) {
        if self.screen_pick {
            return;
        }
        // Tool to return to after a commit. If the eyedropper tool is what
        // launched us, fall back to whatever drawing tool preceded it.
        self.screen_pick_return_tool = if self.tool == ActiveTool::ColorPicker {
            self.prev_tool.unwrap_or(ActiveTool::Pencil)
        } else {
            self.tool
        };
        self.screen_pick = true;
        self.screen_pick_prev_opacity = self.bg_opacity;
        self.bg_opacity = 0.0;
        self.screen_pick_arm = true;
        // Drop any in-flight stroke so the gesture that toggled the mode can't
        // leave ink.
        self.stroke = None;
        self.stroke_target = None;
        self.preview_upload_rect = None;
    }

    /// Leave screen-pick mode and restore the previous backdrop opacity. Does
    /// not change the colour or tool — used for cancel (Esc / toggle off).
    pub fn end_screen_pick(&mut self) {
        if !self.screen_pick {
            return;
        }
        self.screen_pick = false;
        self.bg_opacity = self.screen_pick_prev_opacity;
        self.screen_pick_arm = false;
        // NB: do NOT free `screen_pick_tex` here. End can be called mid-frame
        // (on a commit tap) *after* the loupe already painted with that texture
        // this frame; dropping the handle now makes wgpu submit a render pass
        // referencing a destroyed texture → validation panic. The handle is
        // tiny (≈25×25) and reused via `.set()` on the next pick, so keep it.
    }

    /// Commit a sampled colour: apply it to every tool, restore the drawing tool
    /// that was active before picking, and leave pick mode.
    pub fn commit_screen_pick(&mut self, color: [u8; 4]) {
        // Apply to the active brush and every tool's stored colour so the sample
        // sticks across tool switches (mirrors the in-canvas picker).
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

    pub fn pick_color(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        if x < 0 || y < 0 || x >= self.project.width as i32 || y >= self.project.height as i32 {
            return None;
        }
        let ux = x as u32;
        let uy = y as u32;
        let mut out = [0u8; 4];
        for layer in &self.project.layers {
            if !layer.visible || layer.reference {
                continue;
            }
            let Some(id) = layer.resolve(self.project.current_frame) else {
                continue;
            };
            let Some(cell) = self.project.cell(id) else {
                continue;
            };
            let idx = ((uy * cell.width + ux) * 4) as usize;
            if idx + 3 >= cell.pixels.len() {
                continue;
            }
            let mut px = [
                cell.pixels[idx],
                cell.pixels[idx + 1],
                cell.pixels[idx + 2],
                cell.pixels[idx + 3],
            ];
            if px[3] == 0 {
                continue;
            }
            let op = layer.opacity.clamp(0.0, 1.0);
            if op < 1.0 {
                px[3] = (px[3] as f32 * op).round().min(255.0) as u8;
            }
            let sa = px[3] as f32 / 255.0;
            let da = out[3] as f32 / 255.0;
            let out_a = sa + da * (1.0 - sa);
            if out_a <= 0.0 {
                continue;
            }
            for c in 0..3 {
                let s = px[c] as f32 / 255.0;
                let d = out[c] as f32 / 255.0;
                out[c] = ((s * sa + d * da * (1.0 - sa)) / out_a * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
            out[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        if out[3] > 0 {
            return Some(out);
        }
        let c = [
            (self.bg_color[0] * 255.0) as u8,
            (self.bg_color[1] * 255.0) as u8,
            (self.bg_color[2] * 255.0) as u8,
            255,
        ];
        Some(c)
    }

    pub fn dispatch(&mut self, action: Action) {
        match action {
            Action::ToolPencil => {
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
            Action::ToolColorPicker => {
                self.prev_tool = Some(self.tool);
                self.tool_brushes[self.tool.idx()] = self.brush.clone();
                self.tool = ActiveTool::ColorPicker;
                self.brush = self.tool_brushes[ActiveTool::ColorPicker.idx()].clone();
            }
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
            Action::PlayPause => {
                let now = 0.0; // refreshed by playback.tick on next frame
                let _ = now;
                self.playback.playing = !self.playback.playing;
            }
            Action::FramePrev => self.project.step(-1),
            Action::FrameNext => self.project.step(1),
            Action::FrameAdd => self.structural_edit(false, Project::add_frame),
            Action::FrameDuplicate => self.structural_edit(false, Project::duplicate_frame),
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
            Action::RotateReset => self.view.rotation = 0.0,
            Action::SaveProject => {
                if let Err(e) = crate::io::project_file::save_dialog(&self.project) {
                    log::error!("Save project failed: {e:#}");
                }
            }
            Action::OpenProject => match crate::io::project_file::load_dialog() {
                Ok(Some(p)) => self.load_project(p),
                Ok(None) => {}
                Err(e) => log::error!("Open project failed: {e:#}"),
            },
            // Canvas nav gestures are modifier-only drag binds, handled directly
            // in the canvas input code — never dispatched as press actions.
            Action::CanvasZoom | Action::CanvasPan | Action::CanvasRotate => {}
            Action::LayerTransformToggle => self.layer_xform = !self.layer_xform,
            Action::TransformKeyAdd => self.add_transform_key(),
            Action::TransformKeyDelete => self.delete_transform_key(),
        }
    }

    /// Replace the current project with a loaded one, resetting derived editing
    /// state (textures, history, in-flight strokes, view) but keeping user
    /// preferences (tools, brushes, shortcuts, pen).
    pub fn load_project(&mut self, project: Project) {
        self.project = project;
        self.cell_textures.clear();
        self.cell_dirty.clear();
        for id in 0..self.project.cells.len() {
            self.cell_dirty.insert(id, true);
        }
        self.stroke = None;
        self.stroke_target = None;
        self.shape_drag = None;
        self.stroke_pre_pixels = None;
        self.preview_upload_rect = None;
        self.history = History::default();
        self.view = View::default();
        self.nav_drag = None;
        self.playback = Playback::default();
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
        // Resolve any deferred preview-texture free now, before drawing, so the
        // freed textures are never referenced by this frame's paint list.
        if self.preview_clear_pending {
            self.preview_tex.clear();
            self.preview_clear_pending = false;
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

        self.pen.poll();

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
        if self.playback.tick(&mut self.project, now) {
            ctx.request_repaint();
        }
        if self.playback.playing {
            ctx.request_repaint_after(std::time::Duration::from_secs_f32(
                1.0 / self.project.fps.max(1.0),
            ));
        }

        // Keep the active layer's transform edit buffer synced while scrubbing.
        self.sync_active_transform_buffer();

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
