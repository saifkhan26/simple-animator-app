//! A headless sweep over every window the UI shows, hunting layout faults:
//! a window that creeps or ratchets larger, text drawn over text, text cut
//! off at a window's edge, a drag on one edge that moves the other, and egui
//! ID clashes.
//!
//! Each window is drawn on its own, so every shape in a frame's output is
//! that window's. Each check collects its findings across all windows and
//! fails once with the whole list.

use super::*;
use crate::app::{ExportKind, ImportRangeState, ImportSource};
use egui::{pos2, Pos2};

type Shapes = Vec<egui::epaint::ClippedShape>;

const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, pos2(1600.0, 1000.0));

const PANELS: [PanelId; 7] = [
    PanelId::Tools,
    PanelId::Brush,
    PanelId::Layers,
    PanelId::Onion,
    PanelId::Camera,
    PanelId::Xsheet,
    PanelId::Timeline,
];

#[derive(Clone, Copy, PartialEq)]
enum Win {
    Panel(PanelId),
    /// The Tools panel with another tool up: its options are that tool's.
    ToolPanel(ActiveTool),
    Menu,
    MiniTimeline,
    Settings,
    BrushSettings,
    GridSettings,
    KritaImport,
    NewProject,
    Export(ExportKind),
    ImportRange,
    SaveError,
}

impl std::fmt::Debug for Win {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Win::Panel(p) => write!(f, "{p:?} panel"),
            Win::ToolPanel(t) => write!(f, "Tools panel ({t:?})"),
            Win::Export(kind) => write!(f, "{:?}", kind.title()),
            Win::Menu => write!(f, "Menu"),
            Win::MiniTimeline => write!(f, "MiniTimeline"),
            Win::Settings => write!(f, "Settings"),
            Win::BrushSettings => write!(f, "BrushSettings"),
            Win::GridSettings => write!(f, "GridSettings"),
            Win::KritaImport => write!(f, "KritaImport"),
            Win::NewProject => write!(f, "NewProject"),
            Win::ImportRange => write!(f, "ImportRange"),
            Win::SaveError => write!(f, "SaveError"),
        }
    }
}

fn all() -> Vec<Win> {
    let mut v: Vec<Win> = PANELS.iter().map(|&p| Win::Panel(p)).collect();
    v.extend([
        Win::ToolPanel(ActiveTool::Fill),
        Win::ToolPanel(ActiveTool::Lasso),
        Win::Menu,
        Win::MiniTimeline,
        Win::Settings,
        Win::BrushSettings,
        Win::GridSettings,
        Win::KritaImport,
        Win::NewProject,
        Win::Export(ExportKind::PngSequence),
        Win::Export(ExportKind::Gif),
        Win::Export(ExportKind::Webp),
        Win::Export(ExportKind::Mp4),
        Win::Export(ExportKind::SpriteSheet),
        Win::ImportRange,
        Win::SaveError,
    ]);
    v
}

/// The width `draw` gives each panel.
fn panel_width(id: PanelId) -> f32 {
    match id {
        PanelId::Brush | PanelId::Color => 200.0,
        PanelId::Tools => 232.0,
        PanelId::Timeline => 640.0,
        _ => 252.0,
    }
}

impl Win {
    fn open(self, state: &mut AppState) {
        match self {
            Win::Panel(_) | Win::Menu => {}
            Win::ToolPanel(t) => {
                state.set_tool(t);
                // Grain on, so the sweeps see every row the Fill tool can show.
                if t == ActiveTool::Fill {
                    state.brush.fill_grain = 0.5;
                }
                // And the magic select, whose settings add rows to the Lasso's.
                if t == ActiveTool::Lasso {
                    state.sel_shape = crate::tools::select_mask::SelShape::Wand;
                }
            }
            Win::MiniTimeline => {
                state.show_panels = false;
                state.show_mini_timeline = true;
            }
            Win::Settings => state.show_settings = true,
            Win::BrushSettings => {
                state.show_brush_settings = true;
                // Noise on, so the sweeps see every row it brings.
                state.brush.noise = 0.5;
            }
            Win::GridSettings => {
                state.show_grid_settings = true;
                state.perspective.show = true;
                if state.perspective.grids.is_empty() {
                    state.perspective.grids.push(Default::default());
                }
                // Fill canvas on, so the sweeps see its Rays and Ceiling rows.
                if let Some(g) = state.perspective.active_grid_mut() {
                    g.fill_canvas = true;
                }
            }
            Win::KritaImport => {
                use crate::brush_library::import::{KritaImport, KritaPlaces};
                state.krita_import = Some(KritaImport::start(KritaPlaces::default(), None));
            }
            Win::NewProject => state.show_new_project = true,
            Win::Export(kind) => {
                state.show_export = true;
                state.export_cfg.kind = kind;
            }
            Win::ImportRange => {
                let frames = vec![crate::doc::canvas::Canvas::new(64, 48); 5];
                state.import_range = Some(ImportRangeState {
                    source: ImportSource::Frames { frames, format: "GIF" },
                    total: 5,
                    start: 0,
                    end: 4,
                });
                state.show_import_range = true;
            }
            Win::SaveError => {
                state.save_error =
                    Some("Could not write D:\\Animation\\shot_04\\shot_04_cleanup.anim: access is denied".into())
            }
        }
    }

    fn show(self, state: &mut AppState, ctx: &egui::Context) {
        match self {
            Win::Panel(id) => panel_window(state, ctx, id, [40.0, 40.0], panel_width(id), true, None, None),
            Win::ToolPanel(_) => {
                let id = PanelId::Tools;
                panel_window(state, ctx, id, [40.0, 40.0], panel_width(id), true, None, None)
            }
            Win::Menu => menu_window(state, ctx),
            Win::MiniTimeline => mini_timeline_window(state, ctx),
            Win::Settings => settings_window(state, ctx),
            Win::BrushSettings => brush_settings_window(state, ctx),
            Win::GridSettings => grid_settings_window(state, ctx),
            Win::KritaImport => crate::ui::krita_import::krita_import_window(state, ctx),
            Win::NewProject => new_project_dialog(state, ctx),
            Win::Export(_) => export_dialog(state, ctx),
            Win::ImportRange => import_range_dialog(state, ctx),
            Win::SaveError => save_error_dialog(state, ctx),
        }
    }

    /// Whether text running past a side edge is a list scrolled sideways
    /// rather than cut off.
    fn scrolls_sideways(self) -> bool {
        matches!(self, Win::Panel(PanelId::Timeline | PanelId::Xsheet))
    }

    fn resizable(self) -> bool {
        matches!(
            self,
            Win::Panel(_)
                | Win::ToolPanel(_)
                | Win::Settings
                | Win::BrushSettings
                | Win::GridSettings
                | Win::KritaImport
        )
    }
}

/// One window, drawn on its own in a fresh UI.
struct Rig {
    win: Win,
    ctx: egui::Context,
    state: AppState,
}

impl Rig {
    fn new(win: Win) -> Self {
        let mut state = AppState::for_test();
        state.load_project(crate::doc::project::Project::new(1920, 1080, 24.0), None);
        state.show_panels = true;
        win.open(&mut state);
        let ctx = egui::Context::default();
        crate::ui::theme::install(&ctx);
        let mut rig = Self { win, ctx, state };
        rig.settle();
        rig
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> Shapes {
        let raw = egui::RawInput {
            screen_rect: Some(SCREEN),
            events,
            ..Default::default()
        };
        let (win, state) = (self.win, &mut self.state);
        self.ctx.run(raw, |ctx| win.show(state, ctx)).shapes
    }

    /// Enough frames for egui's open/close and scroll animations to finish.
    fn settle(&mut self) -> Shapes {
        let mut shapes = Shapes::new();
        for _ in 0..40 {
            shapes = self.frame(vec![]);
        }
        shapes
    }

    /// The window's outer rect. Exactly one is shown.
    fn rect(&self) -> Rect {
        let layers: Vec<egui::LayerId> = self.ctx.memory(|m| {
            m.areas()
                .visible_layer_ids()
                .into_iter()
                .filter(|l| l.order == egui::Order::Middle)
                .collect()
        });
        assert_eq!(layers.len(), 1, "{:?}: windows {layers:?}", self.win);
        egui::AreaState::load(&self.ctx, layers[0].id).expect("laid out").rect()
    }

    fn click(&mut self, at: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(at), button(at, true)]);
        self.frame(vec![button(at, false)]);
        // Off the window, so a hover tooltip doesn't sit over the next look.
        self.frame(vec![egui::Event::PointerMoved(pos2(SCREEN.max.x - 2.0, SCREEN.max.y - 2.0))]);
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(from)]);
        self.frame(vec![button(from, true)]);
        for i in 1..=12 {
            let t = i as f32 / 12.0;
            self.frame(vec![egui::Event::PointerMoved(from + (to - from) * t)]);
        }
        self.frame(vec![button(to, false)]);
        self.frame(vec![egui::Event::PointerMoved(pos2(SCREEN.max.x - 2.0, SCREEN.max.y - 2.0))]);
        self.settle();
    }

    fn drag_right_edge(&mut self, dx: f32) {
        let r = self.rect();
        let from = pos2(r.max.x - 1.0, r.center().y);
        self.drag(from, from + egui::vec2(dx, 0.0));
    }

    fn drag_bottom_edge(&mut self, dy: f32) {
        let r = self.rect();
        let from = pos2(r.center().x, r.max.y - 1.0);
        self.drag(from, from + egui::vec2(0.0, dy));
    }

    /// Open every folded section, one click at a time, until none is left.
    fn open_sections(&mut self) {
        let mut tried: Vec<Pos2> = Vec::new();
        for _ in 0..24 {
            let shapes = self.settle();
            let r = self.rect();
            let next = closed_headers(&shapes)
                .into_iter()
                // Not the title bar's own collapse arrow.
                .filter(|p| p.y > r.min.y + 32.0 && r.contains(*p))
                .find(|p| !tried.iter().any(|t| t.distance(*p) < 3.0));
            let Some(at) = next else { break };
            tried.push(at);
            self.click(at);
        }
    }
}

fn button(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// A piece of text drawn: its string, its bounds and the clip it was drawn in.
struct Text {
    text: String,
    rect: Rect,
    clip: Rect,
}

fn texts(shapes: &Shapes) -> Vec<Text> {
    fn walk(shape: &egui::Shape, clip: Rect, out: &mut Vec<Text>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            egui::Shape::Text(t) if !t.galley.text().trim().is_empty() => out.push(Text {
                text: t.galley.text().to_string(),
                rect: t.visual_bounding_rect(),
                clip,
            }),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in shapes {
        walk(&s.shape, s.clip_rect, &mut out);
    }
    out
}

/// Where a folded `CollapsingHeader` arrow is drawn: egui paints it as a
/// small filled triangle pointing right. (Open, it points down, as does a
/// combo box's.)
fn closed_headers(shapes: &Shapes) -> Vec<Pos2> {
    fn walk(shape: &egui::Shape, clip: Rect, out: &mut Vec<Pos2>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            egui::Shape::Path(p) if p.points.len() == 3 && p.closed && p.fill != Color32::TRANSPARENT => {
                let mut pts = p.points.clone();
                pts.sort_by(|a, b| a.x.total_cmp(&b.x));
                let size = Rect::from_points(&pts).size();
                let right = (pts[0].x - pts[1].x).abs() < 0.5 && pts[2].x > pts[1].x + 2.0;
                let centre = Rect::from_points(&pts).center();
                if right && size.x < 16.0 && size.y < 16.0 && clip.contains(centre) {
                    out.push(centre);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in shapes {
        walk(&s.shape, s.clip_rect, &mut out);
    }
    out
}

/// Where a combo box's arrow is drawn: a small filled triangle pointing
/// down. (An open `CollapsingHeader`'s points down too; a click on it just
/// folds the section.)
fn down_arrows(shapes: &Shapes) -> Vec<Pos2> {
    fn walk(shape: &egui::Shape, clip: Rect, out: &mut Vec<Pos2>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            egui::Shape::Path(p) if p.points.len() == 3 && p.closed && p.fill != Color32::TRANSPARENT => {
                let mut pts = p.points.clone();
                pts.sort_by(|a, b| a.y.total_cmp(&b.y));
                let size = Rect::from_points(&pts).size();
                let down = (pts[0].y - pts[1].y).abs() < 0.5 && pts[2].y > pts[1].y + 2.0;
                let centre = Rect::from_points(&pts).center();
                if down && size.x < 16.0 && size.y < 16.0 && clip.contains(centre) {
                    out.push(centre);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in shapes {
        walk(&s.shape, s.clip_rect, &mut out);
    }
    out
}

/// Everything wrong with one frame of one window.
fn faults(rig: &Rig, shapes: &Shapes, when: &str) -> Vec<String> {
    let win = rig.rect();
    let mut out = Vec::new();
    let say = |what: String| format!("{:?} {when}: {what}", rig.win);
    if !SCREEN.contains_rect(win) {
        out.push(say(format!("window {win:?} runs off the {SCREEN:?} screen")));
    }
    let drawn: Vec<(Text, Rect)> = texts(shapes)
        .into_iter()
        .filter_map(|t| {
            let seen = t.rect.intersect(t.clip);
            seen.is_positive().then_some((t, seen))
        })
        .collect();
    for (t, _) in &drawn {
        if t.text.contains("use of") && t.text.contains(" ID ") {
            out.push(say(format!("egui ID clash: {}", t.text)));
        }
        // Only the side edges: a list scrolled partway cuts its rows at the
        // top and bottom, which is no fault.
        let past = t.rect.min.x < win.min.x - 0.5 || t.rect.max.x > win.max.x + 0.5;
        if past && !rig.win.scrolls_sideways() {
            out.push(say(format!("{:?} at {:?} runs past the window edge {win:?}", t.text, t.rect)));
        }
    }
    for (i, (a, ra)) in drawn.iter().enumerate() {
        for (b, rb) in &drawn[i + 1..] {
            let o = ra.intersect(*rb);
            if o.width() > 2.0 && o.height() > 2.0 {
                out.push(say(format!("{:?} {:?} overlaps {:?} {:?}", a.text, a.rect, b.text, b.rect)));
            }
        }
    }
    out
}

fn report(findings: Vec<String>) {
    assert!(findings.is_empty(), "{} findings:\n{}", findings.len(), findings.join("\n"));
}

/// Shown, left alone: nothing overlaps, runs off or clashes, and the
/// window holds its size frame after frame.
#[test]
fn every_window_is_clean_and_steady_at_rest() {
    let mut findings = Vec::new();
    for win in all() {
        let mut rig = Rig::new(win);
        let shapes = rig.settle();
        findings.extend(faults(&rig, &shapes, "at rest"));
        let before = rig.rect();
        rig.settle();
        let after = rig.rect();
        if (after.size() - before.size()).length() > 0.25 {
            findings.push(format!("{win:?}: creeps at rest, {before:?} → {after:?}"));
        }
    }
    report(findings);
}

/// Dragging one edge leaves the other dimension alone and lands where
/// dragged; dragging back returns the window to its size. (Wider, a window
/// sized by wrapped text may get shorter: it wraps onto fewer lines.)
#[test]
fn every_window_resizes_on_one_axis_at_a_time() {
    let mut findings = Vec::new();
    for win in all().into_iter().filter(|w| w.resizable()) {
        let mut rig = Rig::new(win);
        let start = rig.rect();
        rig.drag_right_edge(80.0);
        let wide = rig.rect();
        if wide.height() > start.height() + 0.5 {
            findings.push(format!("{win:?}: a sideways drag made it taller, {start:?} → {wide:?}"));
        }
        if (wide.width() - start.width() - 80.0).abs() > 3.0 {
            findings.push(format!("{win:?}: dragged 80 wider, went {start:?} → {wide:?}"));
        }
        rig.drag_right_edge(-80.0);
        let back = rig.rect();
        if (back.width() - start.width()).abs() > 3.0 || back.height() > start.height() + 0.5 {
            findings.push(format!("{win:?}: dragged back, {start:?} → {back:?}"));
        }
        rig.drag_bottom_edge(80.0);
        let tall = rig.rect();
        if (tall.width() - back.width()).abs() > 0.5 {
            findings.push(format!("{win:?}: a downward drag changed the width, {back:?} → {tall:?}"));
        }
        let shapes = rig.settle();
        findings.extend(faults(&rig, &shapes, "made taller"));
    }
    report(findings);
}

/// Pulled as narrow as it goes: what is left still doesn't overlap or spill.
#[test]
fn every_window_stays_clean_at_its_narrowest() {
    let mut findings = Vec::new();
    for win in all().into_iter().filter(|w| w.resizable()) {
        let mut rig = Rig::new(win);
        rig.drag_right_edge(-400.0);
        let shapes = rig.settle();
        findings.extend(faults(&rig, &shapes, "at its narrowest"));
    }
    report(findings);
}

/// Every folded section opened: nothing overlaps or spills, the window holds
/// steady, and it gets no wider than it opened at — a section is laid out to
/// the window, not the other way round.
#[test]
fn every_window_is_clean_with_its_sections_open() {
    let mut findings = Vec::new();
    for win in all() {
        let mut rig = Rig::new(win);
        let start = rig.rect();
        rig.open_sections();
        let shapes = rig.settle();
        findings.extend(faults(&rig, &shapes, "sections open"));
        let open = rig.rect();
        if open.width() > start.width() + 0.5 {
            findings.push(format!("{win:?}: opening its sections widened it, {start:?} → {open:?}"));
        }
        rig.settle();
        let later = rig.rect();
        if (later.size() - open.size()).length() > 0.25 {
            findings.push(format!("{win:?}: creeps with sections open, {open:?} → {later:?}"));
        }
    }
    report(findings);
}

/// A busier project — more layers, long names, cleanup chains, a group,
/// faded layers, a long scene, camera keys, a brush with many sizes
/// remembered, brush values a drag left unrounded — leaves every window the
/// size it was, and still clean.
#[test]
fn a_busy_project_leaves_every_window_its_size() {
    type Step = (&'static str, fn(&mut AppState));
    let steps: [Step; 10] = [
        ("12 more layers", |s| {
            for k in 0..12 {
                s.project.add_layer();
                let i = s.project.current_layer;
                s.project.layers[i].name = format!("Layer {k}");
            }
        }),
        ("a long layer name", |s| {
            let i = s.project.current_layer;
            s.project.layers[i].name = "Character A, left arm: cleanup pass 3 (final)".into();
        }),
        ("a cleanup chain", |s| {
            for _ in 0..3 {
                s.add_cleanup_layer(s.project.current_layer);
            }
        }),
        ("fill lines from a long name", |s| {
            let i = s.project.current_layer;
            s.project.layers[0].lines_from = Some(i);
        }),
        ("a group", |s| {
            let other = s.project.layers[0].uid;
            s.selected_layers.insert(other);
            s.group_selected_layers();
        }),
        ("faded layers, the long name's too", |s| {
            for l in &mut s.project.layers {
                l.opacity = 0.35;
            }
        }),
        ("240 frames", |s| s.project.ensure_frame_count(240)),
        ("brush values a drag left unrounded", |s| {
            s.brush.radius = 16.892_37;
            s.brush.opacity = 0.922_54;
        }),
        ("many brush sizes remembered", |s| {
            let m = crate::app::brush_sizes::SizeMemory {
                recent: vec![0.5, 3.3, 12.5, 64.0, 127.9],
                pinned: vec![1.0, 2.5, 7.5, 100.0, 128.0],
            };
            s.brush_sizes = vec![m; 16];
        }),
        ("camera keys", |s| {
            for f in [0, 12, 48, 200] {
                s.project.current_frame = f;
                s.add_camera_key();
            }
            s.project.current_frame = 0;
        }),
    ];
    let mut findings = Vec::new();
    for win in all() {
        let mut rig = Rig::new(win);
        let start = rig.rect();
        for (what, step) in &steps {
            step(&mut rig.state);
            let shapes = rig.settle();
            let now = rig.rect();
            if now.width() > start.width() + 0.5 || now.height() > start.height() + 0.5 {
                findings.push(format!("{win:?}: {what} grew it, {start:?} → {now:?}"));
            }
            findings.extend(faults(&rig, &shapes, &format!("after {what}")));
        }
    }
    report(findings);
}

/// Every combo box opened, one at a time, with every section open: its list
/// lands on the screen, and the window under it keeps its width.
#[test]
fn every_combo_box_opens_on_screen() {
    // Keys on the current frame, so the ease lists are live.
    let open = |win| {
        let mut rig = Rig::new(win);
        rig.state.add_camera_key();
        rig.state.add_transform_key();
        rig.open_sections();
        rig
    };
    let mut findings = Vec::new();
    let mut opened = 0;
    for win in all() {
        let mut rig = open(win);
        let shapes = rig.settle();
        let top = rig.rect().min.y + 32.0;
        let arrows: Vec<Pos2> = down_arrows(&shapes).into_iter().filter(|p| p.y > top).collect();
        for at in arrows {
            let mut rig = open(win);
            let before = rig.rect();
            rig.frame(vec![egui::Event::PointerMoved(at), button(at, true)]);
            rig.frame(vec![button(at, false)]);
            for _ in 0..10 {
                rig.frame(vec![]);
            }
            let layers: Vec<egui::LayerId> = rig.ctx.memory(|m| {
                m.areas()
                    .visible_layer_ids()
                    .into_iter()
                    .filter(|l| l.order == egui::Order::Foreground)
                    .collect()
            });
            let popups: Vec<Rect> = layers
                .iter()
                .filter_map(|l| egui::AreaState::load(&rig.ctx, l.id).map(|s| s.rect()))
                .collect();
            opened += popups.len();
            for p in popups {
                if !SCREEN.expand(0.5).contains_rect(p) {
                    findings.push(format!("{win:?}: the list from {at:?} at {p:?} runs off screen"));
                }
            }
            let after = rig.rect();
            if after.width() > before.width() + 0.5 {
                findings.push(format!("{win:?}: opening the list at {at:?} widened it, {before:?} → {after:?}"));
            }
        }
    }
    assert!(opened >= 7, "only {opened} lists opened: the arrow search went blind");
    report(findings);
}

/// The whole UI at common screen sizes: every window on screen, and no
/// ID clashes between windows.
#[test]
fn the_whole_ui_fits_common_screens() {
    let mut findings = Vec::new();
    for size in [pos2(1280.0, 720.0), pos2(1920.0, 1080.0), pos2(2560.0, 1440.0)] {
        let screen = Rect::from_min_max(Pos2::ZERO, size);
        let mut state = AppState::for_test();
        state.load_project(crate::doc::project::Project::new(1920, 1080, 24.0), None);
        state.show_panels = true;
        let ctx = egui::Context::default();
        crate::ui::theme::install(&ctx);
        let mut shapes = Shapes::new();
        for _ in 0..40 {
            let raw = egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            shapes = ctx.run(raw, |ctx| draw(&mut state, ctx)).shapes;
        }
        for t in texts(&shapes) {
            if t.text.contains("use of") && t.text.contains(" ID ") {
                findings.push(format!("{size:?}: egui ID clash: {}", t.text));
            }
        }
        let windows: Vec<egui::LayerId> = ctx.memory(|m| {
            m.areas()
                .visible_layer_ids()
                .into_iter()
                .filter(|l| l.order == egui::Order::Middle)
                .collect()
        });
        for l in windows {
            let r = egui::AreaState::load(&ctx, l.id).expect("laid out").rect();
            if !screen.expand(0.5).contains_rect(r) {
                findings.push(format!("{size:?}: window {:?} at {r:?} runs off screen", l.id));
            }
        }
    }
    report(findings);
}


/// However narrow the Brush panel goes, the H/S/L fields stay clear of the
/// Paste button after them, and the backdrop row keeps its colour and
/// opacity. (Their frames, not their text, would collide, which the overlap
/// sweep doesn't see.)
#[test]
fn the_brush_panel_keeps_its_rows_apart_at_its_narrowest() {
    let mut rig = Rig::new(Win::Panel(PanelId::Brush));
    rig.drag_right_edge(-400.0);
    let shapes = rig.settle();
    let t = texts(&shapes);
    let paste = t
        .iter()
        .find(|t| t.text == egui_phosphor::regular::CLIPBOARD_TEXT)
        .expect("Paste is drawn")
        .rect;
    let fields: Vec<&Text> = t
        .iter()
        .filter(|f| (f.rect.center().y - paste.center().y).abs() < 8.0 && (f.text.ends_with('%') || f.text.ends_with('\u{b0}')))
        .collect();
    assert_eq!(fields.len(), 3, "H, S and L on Paste's row");
    for f in fields {
        // The field's padding and the button's together, at the least.
        assert!(f.rect.max.x + 12.0 <= paste.min.x, "{:?} at {:?} runs into Paste at {paste:?}", f.text, f.rect);
    }
    assert!(t.iter().any(|f| f.text == "100%"), "the backdrop's opacity is still there");
}

/// What `rig` draws, painted on the CPU into an image cropped to its window:
/// egui's meshes over the theme background, at `ppp` pixels a point. For
/// looking at a layout without starting the app, which would share the
/// real config folder.
fn snapshot(rig: &mut Rig, ppp: f32) -> image::RgbaImage {
    rig.ctx.set_pixels_per_point(ppp);
    rig.settle();
    let raw = egui::RawInput {
        screen_rect: Some(SCREEN),
        ..Default::default()
    };
    let (win, state) = (rig.win, &mut rig.state);
    let out = rig.ctx.run(raw, |ctx| win.show(state, ctx));
    let prims = rig.ctx.tessellate(out.shapes, out.pixels_per_point);
    let font = rig.ctx.fonts(|f| f.image());
    let (fw, fh) = (font.width(), font.height());
    let tex: Vec<Color32> = font.srgba_pixels(None).collect();
    let win_r = rig.rect().expand(4.0);
    let (x0, y0) = ((win_r.min.x * ppp).floor() as i32, (win_r.min.y * ppp).floor() as i32);
    let (w, h) = ((win_r.width() * ppp).ceil() as i32, (win_r.height() * ppp).ceil() as i32);
    let bg = theme::palette().bg_base;
    let mut px: Vec<[f32; 4]> = vec![
        [bg.r() as f32 / 255.0, bg.g() as f32 / 255.0, bg.b() as f32 / 255.0, 1.0];
        (w * h) as usize
    ];
    for p in prims {
        let egui::epaint::Primitive::Mesh(m) = p.primitive else { continue };
        let textured = m.texture_id == egui::TextureId::default();
        let clip = p.clip_rect;
        for t in m.indices.chunks(3) {
            let v = [&m.vertices[t[0] as usize], &m.vertices[t[1] as usize], &m.vertices[t[2] as usize]];
            let q = v.map(|v| (v.pos.x * ppp - x0 as f32, v.pos.y * ppp - y0 as f32));
            let area = (q[1].0 - q[0].0) * (q[2].1 - q[0].1) - (q[2].0 - q[0].0) * (q[1].1 - q[0].1);
            if area.abs() < 1e-6 {
                continue;
            }
            let lo_x = q.iter().map(|p| p.0).fold(f32::MAX, f32::min).max(clip.min.x * ppp - x0 as f32).max(0.0) as i32;
            let hi_x = q.iter().map(|p| p.0).fold(f32::MIN, f32::max).min(clip.max.x * ppp - x0 as f32).min(w as f32 - 1.0) as i32;
            let lo_y = q.iter().map(|p| p.1).fold(f32::MAX, f32::min).max(clip.min.y * ppp - y0 as f32).max(0.0) as i32;
            let hi_y = q.iter().map(|p| p.1).fold(f32::MIN, f32::max).min(clip.max.y * ppp - y0 as f32).min(h as f32 - 1.0) as i32;
            for y in lo_y..=hi_y {
                for x in lo_x..=hi_x {
                    let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let edge = |a: (f32, f32), b: (f32, f32)| ((b.0 - a.0) * (cy - a.1) - (b.1 - a.1) * (cx - a.0)) / area;
                    let b = [edge(q[1], q[2]), edge(q[2], q[0]), edge(q[0], q[1])];
                    if b.iter().any(|&k| k < -1e-4) {
                        continue;
                    }
                    let mut c = [0f32; 4];
                    for (k, vert) in v.iter().enumerate() {
                        let a = vert.color.to_array();
                        for i in 0..4 {
                            c[i] += b[k] * a[i] as f32 / 255.0;
                        }
                    }
                    if textured {
                        let u = (b[0] * v[0].uv.x + b[1] * v[1].uv.x + b[2] * v[2].uv.x) * fw as f32;
                        let vv = (b[0] * v[0].uv.y + b[1] * v[1].uv.y + b[2] * v[2].uv.y) * fh as f32;
                        let s = tex[(vv as usize).min(fh - 1) * fw + (u as usize).min(fw - 1)].to_array();
                        for i in 0..4 {
                            c[i] *= s[i] as f32 / 255.0;
                        }
                    }
                    // Premultiplied, over what is there.
                    let d = &mut px[(y * w + x) as usize];
                    for i in 0..4 {
                        d[i] = c[i] + d[i] * (1.0 - c[3]);
                    }
                }
            }
        }
    }
    let bytes: Vec<u8> = px.iter().flat_map(|p| p.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
    image::RgbaImage::from_raw(w as u32, h as u32, bytes).expect("sized to fit")
}

/// Writes the Brush panel and Settings as PNGs into `SNAPSHOT_DIR`, for a
/// look at the layout. `SNAPSHOT_DIR=... cargo test -- --ignored snapshots`
#[test]
#[ignore]
fn snapshots() {
    let Some(dir) = std::env::var_os("SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
        return;
    };
    let mut rig = Rig::new(Win::Panel(PanelId::Brush));
    rig.state.palette = vec![[250, 200, 40], [10, 10, 10], [24, 32, 30], [30, 52, 40], [230, 215, 190], [90, 90, 90], [245, 240, 230]];
    rig.state.set_brush_color([83, 63, 4]);
    snapshot(&mut rig, 2.0).save(dir.join("brush.png")).unwrap();
    rig.state.set_wheel_pinned(true);
    snapshot(&mut rig, 2.0).save(dir.join("brush_pinned.png")).unwrap();
    // A swatch from outside the set: the set stays, nothing lit.
    rig.state.set_brush_color([230, 215, 190]);
    snapshot(&mut rig, 2.0).save(dir.join("brush_pinned_off_set.png")).unwrap();
    rig.state.set_wheel_pinned(false);
    rig.drag_right_edge(-400.0);
    snapshot(&mut rig, 2.0).save(dir.join("brush_narrowest.png")).unwrap();
    let mut rig = Rig::new(Win::Settings);
    snapshot(&mut rig, 1.5).save(dir.join("settings.png")).unwrap();
    // A grid's settings, with Fill canvas's rows showing.
    let mut rig = Rig::new(Win::GridSettings);
    snapshot(&mut rig, 2.0).save(dir.join("grid_settings.png")).unwrap();

    // Brush settings with noise on, tall enough to see it.
    let mut rig = Rig::new(Win::BrushSettings);
    rig.state.brush = crate::tools::BrushSettings::grainy_airbrush();
    rig.drag_bottom_edge(500.0);
    snapshot(&mut rig, 1.5).save(dir.join("brush_settings_noise.png")).unwrap();

    // The Fill tool's options with grain on, bucket then lasso.
    let mut rig = Rig::new(Win::ToolPanel(ActiveTool::Fill));
    snapshot(&mut rig, 2.0).save(dir.join("fill_grain.png")).unwrap();
    rig.state.set_fill_mode(crate::app::FillMode::Lasso);
    rig.settle();
    snapshot(&mut rig, 2.0).save(dir.join("fill_grain_lasso.png")).unwrap();

    // The Lasso with the magic select picked.
    let mut rig = Rig::new(Win::ToolPanel(ActiveTool::Lasso));
    snapshot(&mut rig, 2.0).save(dir.join("lasso_wand.png")).unwrap();

    // The onion strip: off, then on with a bar dragged and an offset soloed.
    let mut rig = Rig::new(Win::Panel(PanelId::Onion));
    snapshot(&mut rig, 2.0).save(dir.join("onion.png")).unwrap();
    rig.state.onion.enabled = true;
    rig.state.onion.toggle_shown(5, OnionDirection::Prev);
    rig.state.onion.set_opacity(2, OnionDirection::Next, 0.3);
    snapshot(&mut rig, 2.0).save(dir.join("onion_on.png")).unwrap();
    rig.state.onion.solo(2, OnionDirection::Next);
    snapshot(&mut rig, 2.0).save(dir.join("onion_solo.png")).unwrap();
    rig.state.onion.solo(2, OnionDirection::Next);
    // Two pins at their own opacities, and the panel dragged taller.
    let li = rig.state.project.current_layer;
    rig.state.project.layers[li].onion_pins = vec![
        OnionPin { frame: 0, tint: PIN_TINTS[0], visible: true, opacity: 0.6 },
        OnionPin { frame: 0, tint: PIN_TINTS[1], visible: true, opacity: 0.25 },
    ];
    rig.settle();
    snapshot(&mut rig, 2.0).save(dir.join("onion_pins.png")).unwrap();
    rig.drag_bottom_edge(120.0);
    snapshot(&mut rig, 2.0).save(dir.join("onion_tall.png")).unwrap();
    rig.drag_right_edge(-400.0);
    snapshot(&mut rig, 2.0).save(dir.join("onion_narrowest.png")).unwrap();

    // The per-layer onion toggles: layer 2 kept on, layer 3 active.
    for win in [Win::Panel(PanelId::Layers), Win::Panel(PanelId::Timeline)] {
        let mut rig = Rig::new(win);
        while rig.state.project.layers.len() < 3 {
            rig.state.project.add_layer();
        }
        rig.state.project.layers[1].onion = true;
        rig.state.project.current_layer = 2;
        rig.settle();
        let name = if matches!(win, Win::Panel(PanelId::Layers)) { "layers.png" } else { "timeline.png" };
        snapshot(&mut rig, 2.0).save(dir.join(name)).unwrap();
    }
}
