# Architecture: how everything connects

This page is the map. It follows the program from the moment it starts, explains the few big ideas the whole
codebase is built around, and links to the detailed page for every piece. For step-by-step traces of single
actions (drawing a line, undo, save…) see [walkthroughs.md](walkthroughs.md). For Rust syntax, see the
[Rust primer](rust-primer.md).

## 1. The big picture

Animator is one Rust program (one *crate*) with a native window drawn by **egui** (an immediate-mode UI
library) on top of **wgpu** (the GPU), hosted by **eframe** (which opens the window and runs the frame loop).

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ eframe  (window, event loop, calls AppState::update every frame)           │
│ ┌──────────────────────────────────────────────────────────────────────┐  │
│ │ AppState  (src/app.rs + src/app/*.rs) — the one big state object     │  │
│ │                                                                      │  │
│ │  input ──────────► commands / pointer handlers ──────► edits         │  │
│ │  (input/*)          (dispatch, pointer_down/move/up)     │           │  │
│ │                                                          ▼           │  │
│ │  tools (tools/*)  ◄────────────── uses ───────────  Project          │  │
│ │  strokes, fill, selection,                          (doc/*)          │  │
│ │  perspective, Krita engine                          layers, cells,   │  │
│ │                                                     camera           │  │
│ │  undo (undo.rs) ◄── every edit records a Command ───────┘            │  │
│ │                                                                      │  │
│ │  textures: CPU pixels ──upload──► GPU textures                       │  │
│ │                                                                      │  │
│ │  ui::shell::draw ──► panels, menus, canvas (ui/*)                    │  │
│ │  io (io/*) on worker threads: save/load/export/import                │  │
│ │  krita_link ◄──► Krita (via .kra + mailbox files + Python plugin)    │  │
│ └──────────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────┘
```

| Folder | Role | Start reading at |
|---|---|---|
| `src/main.rs` | entry point, window options, module tree | [main.md](src/main.md) |
| `src/app.rs`, `src/app/` | `AppState`: all state and behaviour | [app.md](src/app.md) |
| `src/doc/` | the document model (what gets saved) | [doc/mod.md](src/doc/mod.md), [project.md](src/doc/project.md) |
| `src/tools/` | drawing tools and their algorithms | [tools/mod.md](src/tools/mod.md) |
| `src/tools/krita/` | port of Krita's pixel brush engine | [tools/krita/mod.md](src/tools/krita/mod.md) |
| `src/ui/` | everything drawn with egui | [ui/mod.md](src/ui/mod.md), [shell.md](src/ui/shell.md) |
| `src/input/` | shortcuts, pen tablet, OS hooks | [input/mod.md](src/input/mod.md) |
| `src/io/` | file formats, import/export | [io/mod.md](src/io/mod.md) |
| `src/timeline/` | playback clock, onion skin settings | [timeline/mod.md](src/timeline/mod.md) |
| `src/brush_library/` | brush presets on disk, Krita preset import | [brush_library/mod.md](src/brush_library/mod.md) |
| `src/undo.rs` | undo/redo history | [undo.md](src/undo.md) |
| `src/krita_link.rs`, `krita_helper.rs`, `krita-plugin/` | "Edit in Krita" | [krita_link.md](src/krita_link.md) |

## 2. Startup

```text
OS starts animator-app.exe
  └─ main()                                         src/main.rs
       ├─ env_logger init (RUST_LOG)
       └─ eframe::run_native("Animator", native_options(), |cc| AppState::new(cc))
            ├─ opens a frameless, transparent, maximised window; sets up wgpu + egui
            ├─ installs input::msg_hook (raw Windows messages)
            └─ calls the closure once the window exists:
                 AppState::new(cc)                  src/app.rs
                   ├─ ui::theme::install            fonts, icon font, text sizes
                   ├─ reads the GPU's max texture size
                   ├─ loads UiPrefs from ui.ron (eframe storage)
                   ├─ with_prefs(...)
                   │    ├─ Project::new(1280, 720, 24 fps)   one layer, one blank cell
                   │    ├─ input::shortcuts::load()          shortcuts.toml
                   │    └─ every field of AppState initialised
                   ├─ CanvasGpu::new(render state)  custom canvas pipeline
                   └─ apply_scheme                   colour scheme
```

After that, `main` doesn't run again until the window closes; everything happens in the frame loop.

Details: [main.md](src/main.md) · [`AppState::new`](src/app.md#appstatenew) · [`with_prefs`](src/app.md#appstatewith_prefs).

## 3. The frame loop

eframe calls [`AppState::update`](src/app.md#appstateupdate) every time it draws a frame — continuously while
something moves (playback, a stroke, an animation), otherwise when input arrives or someone called
`ctx.request_repaint_after(...)`. There is no event-listener tree: each frame the whole UI is described again
from `AppState` (see [egui and immediate mode UI](rust-primer.md#egui-and-immediate-mode-ui)).

One frame, in order:

1. **Housekeeping** — note layer changes (`track_layer_change`), receive brush presets/thumbnails from the
   library loader, advance a stabilized stroke on its timer (`tick_stroke`).
2. **First frame only** — maximise, give the frameless window rounded corners ([platform.md](src/platform.md)),
   bind the pen tablet to this window.
3. **Input** — poll the tablet driver for pen packets ([tablet.md](src/input/tablet.md)), sample modifier keys
   (`read_input`), turn key presses into commands (`handle_keys` → `ShortcutMap::poll_actions` → `dispatch`).
4. **Time** — advance playback ([playback.md](src/timeline/playback.md)); while playing, ask for the next frame
   after `1/fps` seconds.
5. **Keep edit buffers in step with the current frame** — layer transform, selection, liquify session,
   camera, perspective grids.
6. **Background work** — collect finished imports, saves, Krita link steps and preview frames (non-blocking
   channel polls); keep repainting while any job runs.
7. **Memory and GPU** — free drawings nothing refers to (`collect_cells`), pack idle ones, pre-warm the next
   stroke (`warm_pre`), upload changed pixels to GPU textures (`sync_textures`).
8. **Draw the UI** — [`ui::shell::draw(self, ctx)`](src/ui/shell.md): top bar, panels, dialogs, and the canvas
   (which also reads pen/mouse input and calls `pointer_down/move/up`).
9. **Tidy focus** — undo stray keyboard focus a Tab press gave a widget.

Other `eframe::App` hooks on `AppState`: `save` (writes `UiPrefs` to `ui.ron` every ~30 s and on exit),
`on_exit` (waits for a background save), `raw_input_hook` (restores Ctrl+C/X/V key presses —
[clipboard_keys.md](src/input/clipboard_keys.md)), `clear_color` (window background, transparent for tracing).

## 4. The document model

What the user makes — and what a `.anim` file stores — is a [`Project`](src/doc/project.md):

```text
Project
├── width, height, fps
├── cells: Vec<Arc<Canvas>>      the drawings; CellId = index here      (doc/canvas.rs)
├── layers: Vec<Layer>           bottom → top                           (doc/layer.rs)
│     └── exposures: Vec<Option<CellId>>   per frame: Some = key, None = hold
│         transform (+ keys), opacity, flags, clip, alpha lock, group membership
├── frame_count, current_frame, current_layer, loop range
└── camera (+ camera_keys)                                              (doc/camera.rs)
```

- A **cell** is one drawing: an RGBA pixel buffer ([canvas.md](src/doc/canvas.md)) with a dirty rectangle that
  records what the last edit touched.
- A **layer** shows one cell per frame. Holding a drawing for several frames is just `None` entries after a key
  ([layer.md](src/doc/layer.md)). Groups are header rows in the same flat list ([groups.md](src/doc/groups.md)).
- Cells are `Arc`-shared and **copy-on-write**: cloning a whole `Project` (for a save, an export, a Krita send) only
  copies pointers; a drawing edited while a clone still holds it is copied on its first write (`Arc::make_mut`,
  [smart pointers](rust-primer.md#smart-pointers)). That's what lets saves run on another thread while you keep
  drawing.
- Clipping masks and merged group images are computed on the CPU ([clip.md](src/doc/clip.md),
  [app/groups.md](src/app/groups.md)).

Not in the project: tool settings, brushes, shortcuts, panel layout, perspective grids (kept per file by the app,
[app/grids.md](src/app/grids.md)) — those are preferences, saved separately.

## 5. Input and commands

```text
keyboard ─► egui InputState ─► ShortcutMap::poll_actions ─► Vec<Action> ─► AppState::dispatch(action)
                                (input/shortcuts.rs)                          (one big match → methods)

pen tablet ─► Wintab driver ─► PenInput::poll (packets with pressure, tilt, time)   (input/tablet.rs)
mouse/pen ─► ui::shell canvas input ─► pen_stroke_points ─► AppState::pointer_down / move / up
middle/right drags ─► ButtonDrag pickers (pan, zoom, rotate, scrub…)               (input/button_drag.rs)
```

- Every command has an `Action` enum variant; `dispatch` maps each to a method. Menus and buttons call the same
  methods or `dispatch`. Shortcuts live in `shortcuts.toml`, editable by hand and in Settings
  ([shortcuts.md](src/input/shortcuts.md)).
- Pen positions come from tablet packets when the pen is active (sub-pixel, with pressure and tilt) and from the
  cursor otherwise; the canvas converts screen → document → the active drawing's pixel space.

## 6. Tools and drawing

The active tool is `AppState::tool` (`ActiveTool`), with one `BrushSettings` per tool
([tools/mod.md](src/tools/mod.md)). A freehand stroke:

```text
pointer_down ─► snapshot the drawing (stroke_pre_pixels), StrokeBuilder::new
pointer_move ─► StrokeBuilder::push(sample)  smoothing → Bezier → spine nodes / dabs
each frame  ─► flush_stroke: rasterise new geometry into the coverage buffer (ribbon.rs)
               and composite over the snapshot; stream_stroke uploads that patch this frame
pointer_up  ─► StrokeBuilder::finish; commit_undo records a PixelPatch (dirty rect before/after)
```

- App brushes rasterise as **ribbons** (ink: capsules combined with `max`) or **dabs** (pencil: stamps that build
  up) — [stroke.md](src/tools/stroke.md), [ribbon.md](src/tools/ribbon.md), [dab.md](src/tools/dab.md).
- **Krita brushes** run on the ported engine: sensors → dab shape → tip mask → composite
  ([tools/krita/mod.md](src/tools/krita/mod.md)).
- Other tools: bucket fill with gap closing ([fill.md](src/tools/fill.md)), selections and floating pixels
  ([selection.md](src/tools/selection.md), [select_mask.md](src/tools/select_mask.md), [app/select.md](src/app/select.md)),
  magic wand ([wand.md](src/tools/wand.md)), shapes ([shape.md](src/tools/shape.md)), perspective grids
  ([perspective.md](src/tools/perspective.md)), liquify ([liquify.md](src/tools/liquify.md)).
- Every pixel edit is clipped to the selection mask and respects alpha lock.

## 7. Rendering the canvas

Pixels live on the CPU (`Canvas`). To show them, each visible drawing has a **GPU texture** kept in step:

1. Edits record what changed (`Canvas::dirty` → `AppState::mark_dirty` → `cell_dirty`).
2. `sync_textures` (each frame) uploads changed rectangles as patches, or re-crops a texture to the drawing's
   painted area (`plan_upload`), within a **texture memory budget** that evicts least-recently-shown textures
   (or, during playback, the newest).
3. Onion-skin **ghost textures** are separate tinted silhouettes, downscaled ([onion.md](src/timeline/onion.md),
   `ghost_image`).
4. `ui::shell::paint_canvas` draws every layer back to front as textured quads through
   [canvas_gpu.md](src/ui/canvas_gpu.md) — a copy of egui's pipeline that mixes texels in gamma space so soft edges
   don't get a white rim — then overlays: selection ants, transform boxes, camera guide, grids, cursors.

## 8. Undo

Every edit pushes a [`Command`](src/undo.md) onto `AppState::history`:

| Command | Stores |
|---|---|
| `PixelPatch` | one drawing's dirty rectangle, before and after |
| `Structural` | timeline snapshot (layers/exposures/cursors/camera — no pixels) before and after |
| `LayerCanvasResize` | the original canvases of a resized layer |
| `Selection`, `Grid` | the selection mask / a perspective grid before and after |
| `Compound` | several of the above as one step |

Drags open one step at their start and push it at their end (layer transforms, camera, opacity). The history is
capped at 80 steps and 2 GB of patches. `AppState::undo` first backs out of anything in progress (a polygon corner,
floating pixels, a warp stroke) before touching the history.

## 9. Files and background jobs

Slow work runs on **worker threads** that report back through a channel, polled each frame
([threads and channels](rust-primer.md#threads-and-channels)):

```text
UI thread: project.clone() ─► thread::spawn(move || tx.send(save_to(&project, &path)))
           keep rx in AppState ─► each frame rx.try_recv(): Empty → wait; Ok → toast; Err → error dialog
```

| Job | Code |
|---|---|
| Save / Open `.anim` (header + zlib + postcard) | [io/project_file.md](src/io/project_file.md), [io/legacy.md](src/io/legacy.md) |
| Export PNG sequence / GIF / WebP / MP4 (ffmpeg) / sprite sheet | [io/composite.md](src/io/composite.md) (flatten), [frame_order.md](src/io/frame_order.md), the exporters in [io/](src/io/mod.md) |
| Import images, PNG sequences, GIF, WebP, video (ffmpeg) | [io/](src/io/mod.md) |
| Krita `.kra` read/write | [io/kra/mod.md](src/io/kra/mod.md) |
| Brush library scanning, Krita preset import | [brush_library/](src/brush_library/mod.md) |

## 10. The Krita link

**Edit in Krita** writes the project as an animated `.kra` into a folder named `animator-krita` and opens it in
Krita. A small Python plugin in Krita ([krita-plugin](krita-plugin/animator_link/extension.md)) and the app talk
through two mailbox files next to it ([krita_helper.md](src/krita_helper.md)). When you send again, the plugin saves
Krita's unsaved edits, the app reads that save and merges only what Krita changed (as one undo step), rewrites the
file, and the plugin reloads it in place. Layers named with `-x` stay Krita-only and are carried through untouched.
Details: [krita_link.md](src/krita_link.md).

## 11. Preferences and config files

| What | Where | Code |
|---|---|---|
| Panel layout + `UiPrefs` (brushes, onion, colours, grids per file…) | `%APPDATA%\animator-app\ui.ron` | `AppState::save`, `UiPrefs` in [app.md](src/app.md) |
| Key bindings | `%APPDATA%\animator-app\shortcuts.toml` | [shortcuts.md](src/input/shortcuts.md) |
| Imported brushes | `%APPDATA%\animator-app\brushes\` | [brush_library/mod.md](src/brush_library/mod.md) |

Preferences use `#[serde(default)]` so files from older builds still load ([prefs migration via serde](rust-primer.md#serde-and-file-formats)).

## 12. How it is tested

- **Headless app state**: `AppState::for_test()` builds the whole app without a window or GPU; tests call
  `dispatch`, `pointer_down/move/up` and `undo` directly ([app.md tests](src/app.md#tests)).
- **Headless UI**: tests run real egui frames with synthetic input and inspect what would be painted
  ([shell.md tests](src/ui/shell.md#tests), [sweep_tests.md](src/ui/sweep_tests.md) for every window's layout).
- **Golden images**: every brush is drawn over a matrix of inputs and hashed against stored values
  ([golden_tests.md](src/tools/golden_tests.md)).
- **Stress**: random two-sided editing sessions for the Krita link ([krita_stress.md](src/krita_stress.md)).
- Tests never touch the user's real config (`config_path` is `None` under test).
