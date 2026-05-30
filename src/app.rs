//! Top-level application state. Wires project (timeline + layers), tools, UI.

use std::collections::HashMap;

use eframe::CreationContext;
use egui::{ColorImage, TextureHandle, TextureOptions};

use crate::doc::layer::CellId;
use crate::doc::project::Project;
use crate::input::pointer::PointerSample;
use crate::input::shortcuts::{self, Action, ShortcutMap};
use crate::input::tablet::PenInput;
use crate::timeline::onion::OnionConfig;
use crate::timeline::playback::Playback;
use crate::tools::stroke::StrokeBuilder;
use crate::tools::{ActiveTool, BrushSettings};
use crate::ui;
use crate::undo::{self, History};

pub struct AppState {
    pub project: Project,

    /// GPU texture handle per CellId, lazily created.
    pub cell_textures: HashMap<CellId, TextureHandle>,
    /// Per-CellId dirty flag — re-upload on next sync.
    pub cell_dirty: HashMap<CellId, bool>,

    pub tool: ActiveTool,
    pub brush: BrushSettings,
    pub stroke: Option<StrokeBuilder>,
    /// CellId being painted into during the current stroke.
    pub stroke_target: Option<CellId>,

    pub playback: Playback,
    pub onion: OnionConfig,

    pub bg_opacity: f32,
    pub show_checker: bool,

    pub pen: PenInput,

    pub history: History,
    /// Full-pixel snapshot of the cell before the in-flight stroke started.
    /// Used to extract the `before` slice for an undo command when the stroke
    /// finishes.
    stroke_pre_pixels: Option<Vec<u8>>,

    pub shortcuts: ShortcutMap,
    /// While `Some(action)`, the next key press from the user becomes the
    /// new binding for that action.
    pub rebinding: Option<Action>,
    pub show_settings: bool,
    /// Master visibility of all floating panel windows. Tab toggles it.
    pub show_panels: bool,
}

impl AppState {
    pub fn new(cc: &CreationContext<'_>) -> Self {
        crate::ui::theme::install(&cc.egui_ctx);

        let project = Project::new(1280, 720, 24.0);
        let mut cell_dirty = HashMap::new();
        for id in 0..project.cells.len() {
            cell_dirty.insert(id, true);
        }
        Self {
            project,
            cell_textures: HashMap::new(),
            cell_dirty,
            tool: ActiveTool::Pencil,
            brush: BrushSettings::default_pencil(),
            stroke: None,
            stroke_target: None,
            playback: Playback::default(),
            onion: OnionConfig::default(),
            bg_opacity: 1.0,
            show_checker: false,
            pen: PenInput::new(),
            history: History::default(),
            stroke_pre_pixels: None,
            shortcuts: shortcuts::load(),
            rebinding: None,
            show_settings: false,
            show_panels: true,
        }
    }

    /// Build a PointerSample at (x, y, t), filling pressure from the tablet
    /// backend when available, otherwise 1.0 (mouse).
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

    /// Mirror new CellIds into the dirty map so freshly-allocated cells get
    /// uploaded on the next sync.
    fn ensure_cell_tracking(&mut self) {
        for id in 0..self.project.cells.len() {
            self.cell_dirty.entry(id).or_insert(true);
        }
    }

    /// Upload any dirty cells used by the composite this frame
    /// (current + onion neighbours, across all visible layers).
    pub fn sync_textures(&mut self, ctx: &egui::Context) {
        self.ensure_cell_tracking();

        let mut needed: Vec<CellId> = Vec::new();
        let cur = self.project.current_frame;
        let prev_n = if self.onion.enabled { self.onion.prev as usize } else { 0 };
        let next_n = if self.onion.enabled { self.onion.next as usize } else { 0 };
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
            let image = ColorImage::from_rgba_unmultiplied(
                [c.width as usize, c.height as usize],
                &c.pixels,
            );
            if let Some(tex) = self.cell_textures.get_mut(&id) {
                tex.set(image, TextureOptions::LINEAR);
            } else {
                let tex =
                    ctx.load_texture(format!("cell_{id}"), image, TextureOptions::LINEAR);
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
                crate::tools::fill::flood(c, sample.x.round() as i32, sample.y.round() as i32, opts);
            }
            self.commit_undo(target);
            self.mark_dirty(target);
            self.stroke = None;
            self.stroke_target = None;
            return;
        }

        let mut builder = StrokeBuilder::new(self.brush.clone(), self.tool);
        builder.push(sample);
        if let Some(c) = self.project.cell_mut(target) {
            builder.flush_to(c);
        }
        self.stroke = Some(builder);
        self.mark_dirty(target);
    }

    pub fn pointer_move(&mut self, sample: PointerSample) {
        let Some(target) = self.stroke_target else {
            return;
        };
        let Some(builder) = &mut self.stroke else {
            return;
        };
        builder.push(sample);
        if let Some(c) = self.project.cell_mut(target) {
            builder.flush_to(c);
        }
        self.mark_dirty(target);
    }

    pub fn pointer_up(&mut self) {
        let Some(target) = self.stroke_target.take() else {
            self.stroke = None;
            self.stroke_pre_pixels = None;
            return;
        };
        if let Some(mut builder) = self.stroke.take() {
            if let Some(c) = self.project.cell_mut(target) {
                builder.finish(c);
            }
            self.mark_dirty(target);
        }
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

    /// Run one keyboard action on the document. Called from the per-frame
    /// shortcut dispatcher in `update()`.
    pub fn dispatch(&mut self, action: Action) {
        use crate::tools::BrushSettings;
        match action {
            Action::ToolPencil => {
                self.tool = ActiveTool::Pencil;
                self.brush = BrushSettings::default_pencil();
            }
            Action::ToolInk => {
                self.tool = ActiveTool::Ink;
                self.brush = BrushSettings::default_ink();
            }
            Action::ToolEraser => {
                self.tool = ActiveTool::Eraser;
                self.brush = BrushSettings::default_eraser();
            }
            Action::ToolFill => {
                self.tool = ActiveTool::Fill;
                self.brush = BrushSettings::default_fill();
            }
            Action::PlayPause => {
                let now = 0.0; // refreshed by playback.tick on next frame
                let _ = now;
                self.playback.playing = !self.playback.playing;
            }
            Action::FramePrev => self.project.step(-1),
            Action::FrameNext => self.project.step(1),
            Action::FrameAdd => self.project.add_frame(),
            Action::FrameDuplicate => self.project.duplicate_frame(),
            Action::FrameDelete => self.project.delete_frame(),
            Action::OnionToggle => self.onion.enabled = !self.onion.enabled,
            Action::LayerAdd => self.project.add_layer(),
            Action::LayerDelete => self.project.delete_layer(),
            Action::LayerToggleVisible => {
                if let Some(l) = self.project.layers.get_mut(self.project.current_layer) {
                    l.visible = !l.visible;
                }
            }
            Action::KeyBlank => {
                let id = self.project.insert_blank_key_here();
                self.mark_dirty(id);
            }
            Action::KeyCopy => {
                let id = self.project.insert_duplicate_key_here();
                self.mark_dirty(id);
            }
            Action::Hold => self.project.hold_here(),
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
            Action::ToggleCheckerBg => self.show_checker = !self.show_checker,
            Action::TogglePanels => self.show_panels = !self.show_panels,
        }
    }

    pub fn undo(&mut self) {
        if let Some(touched) = self.history.undo(&mut self.project) {
            self.mark_dirty(touched);
        }
    }

    pub fn redo(&mut self) {
        if let Some(touched) = self.history.redo(&mut self.project) {
            self.mark_dirty(touched);
        }
    }

}

impl eframe::App for AppState {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let a = self.bg_opacity.clamp(0.0, 1.0);
        [0.12 * a, 0.12 * a, 0.13 * a, a]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pen.poll();

        // Shortcut rebind capture: when an Action is "rebinding", the next
        // key press becomes its new combo and rebind mode ends.
        if let Some(action) = self.rebinding {
            let captured = ctx.input(|i| {
                use egui::Key;
                let mods = i.modifiers;
                // Ignore modifier-only key presses (Shift on its own etc.).
                for &key in &[
                    Key::A, Key::B, Key::C, Key::D, Key::E, Key::F, Key::G, Key::H, Key::I,
                    Key::J, Key::K, Key::L, Key::M, Key::N, Key::O, Key::P, Key::Q, Key::R,
                    Key::S, Key::T, Key::U, Key::V, Key::W, Key::X, Key::Y, Key::Z,
                    Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5,
                    Key::Num6, Key::Num7, Key::Num8, Key::Num9,
                    Key::OpenBracket, Key::CloseBracket, Key::Semicolon, Key::Comma,
                    Key::Period, Key::Slash, Key::Backslash, Key::Minus, Key::Equals,
                    Key::Space, Key::Tab, Key::Backspace, Key::Backtick,
                    Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6,
                    Key::F7, Key::F8, Key::F9, Key::F10, Key::F11, Key::F12,
                    Key::ArrowUp, Key::ArrowDown, Key::ArrowLeft, Key::ArrowRight,
                ] {
                    if i.key_pressed(key) {
                        return Some(crate::input::shortcuts::KeyCombo {
                            key,
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
            // Normal shortcut dispatch.
            let actions = self.shortcuts.poll_actions(ctx);
            for a in actions {
                self.dispatch(a);
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

        self.sync_textures(ctx);
        ui::shell::draw(self, ctx);
    }
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
