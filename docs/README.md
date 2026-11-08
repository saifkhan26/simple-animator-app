# Animator — developer documentation

Documentation for every part of the Animator codebase, written for someone who knows **JavaScript** well and is new
to **Rust**. Every source file has its own page, and every function on it is explained, with links that open the code
at the right line (Ctrl+click in VS Code, or click on GitHub).

## Start here

1. **[getting-started.md](getting-started.md)** — install Rust, build, run, test; what is in the repository; what each
   dependency does.
2. **[rust-primer.md](rust-primer.md)** — the Rust you need, compared with JavaScript (ownership, `Option`/`Result`,
   traits, closures, threads, egui's immediate-mode UI…). Keep it open while reading.
3. **[architecture.md](architecture.md)** — how everything connects: startup, the frame loop, the document model,
   input, tools, rendering, undo, files, the Krita link.
4. **[walkthroughs.md](walkthroughs.md)** — follow one action through the code: drawing a line, undo, save, open,
   play, fill, export, Edit in Krita.
5. **[src/main.md](src/main.md)** then **[src/app.md](src/app.md)** — the entry point and the central `AppState`.
6. **[glossary.md](glossary.md)** — animation, graphics and Krita terms.

How the pages are laid out: [HOW-THESE-DOCS-WORK.md](HOW-THESE-DOCS-WORK.md). After changing code, refresh the
pages' line links and find what needs explaining with `python docs/_tools/refresh.py` then
`python docs/_tools/check.py` ([_tools/README.md](_tools/README.md)).

## Every source file

Each link opens that file's page. The page links to the source itself.


### Entry point and app state

| File | What it is |
|---|---|
| [`src/app.rs`](src/app.md) | the app's central state (`AppState`) |
| [`src/app/brush_sizes.rs`](src/app/brush_sizes.md) | remembered brush sizes (recent, pinned, swap, detents) |
| [`src/app/button_drag.rs`](src/app/button_drag.md) | what middle/right-button drags do on the canvas |
| [`src/app/grids.rs`](src/app/grids.md) | perspective grids on the app side (per-file store, follow, keys, drags) |
| [`src/app/groups.rs`](src/app/groups.md) | drawing layer groups on screen |
| [`src/app/lasso_fill.rs`](src/app/lasso_fill.md) | the Fill tool's lasso mode |
| [`src/app/liquify.rs`](src/app/liquify.md) | a Liquify session on AppState |
| [`src/app/orient.rs`](src/app/orient.md) | flipping and turning the artwork |
| [`src/app/pen_slots.rs`](src/app/pen_slots.md) | the pen slots (the row of pens in the Tools panel) |
| [`src/app/select.rs`](src/app/select.md) | the selection: mask, gestures and floating pixels |
| [`src/main.rs`](src/main.md) | where the program starts |

### Document model (`src/doc/`)

| File | What it is |
|---|---|
| [`src/doc/camera.rs`](src/doc/camera.md) | the camera (what each exported frame shows) |
| [`src/doc/canvas.rs`](src/doc/canvas.md) | Pixel storage for one drawing (a cell) |
| [`src/doc/clip.rs`](src/doc/clip.md) | clipping masks and merged group images |
| [`src/doc/groups.rs`](src/doc/groups.md) | layer groups in the flat layer list |
| [`src/doc/layer.rs`](src/doc/layer.md) | one layer of the animation |
| [`src/doc/mod.rs`](src/doc/mod.md) | The document model's table of contents |
| [`src/doc/orient.rs`](src/doc/orient.md) | lossless flips and quarter turns |
| [`src/doc/project.rs`](src/doc/project.md) | the animation document (Project) |
| [`src/doc/transform.rs`](src/doc/transform.md) | Where a layer's drawings sit, and ease curves |

### Tools (`src/tools/`)

| File | What it is |
|---|---|
| [`src/tools/axis_lock.rs`](src/tools/axis_lock.md) | Shift-lock geometry for drags |
| [`src/tools/dab.rs`](src/tools/dab.md) | brush dabs (one stamp of a brush) |
| [`src/tools/fill.rs`](src/tools/fill.md) | flood fill, gap closing and fill painting |
| [`src/tools/golden_tests.rs`](src/tools/golden_tests.md) | golden-image tests for every brush (test-only) |
| [`src/tools/lasso.rs`](src/tools/lasso.md) | turning a lasso outline into a coverage mask |
| [`src/tools/liquify.rs`](src/tools/liquify.md) | Liquify: warp field and resampling |
| [`src/tools/mod.rs`](src/tools/mod.md) | the tools folder, tool list and brush settings |
| [`src/tools/noise.rs`](src/tools/noise.md) | midtone noise for app brushes |
| [`src/tools/paper.rs`](src/tools/paper.md) | paper grain texture |
| [`src/tools/perspective.rs`](src/tools/perspective.md) | perspective grids: geometry, walls, snapping |
| [`src/tools/ribbon.rs`](src/tools/ribbon.md) | the stroke rasteriser (ribbons and dabs) |
| [`src/tools/scan.rs`](src/tools/scan.md) | fast byte scans, eight bytes at a time |
| [`src/tools/select_mask.rs`](src/tools/select_mask.md) | the selection mask in document space |
| [`src/tools/selection.rs`](src/tools/selection.md) | floating selected pixels and the transform box |
| [`src/tools/shape.rs`](src/tools/shape.md) | the Shape tool's lines, rectangles and ellipses |
| [`src/tools/stabilizer.rs`](src/tools/stabilizer.md) | Krita's Stabilizer smoothing |
| [`src/tools/stroke.rs`](src/tools/stroke.md) | building a stroke from pen samples |
| [`src/tools/tap_lasso.rs`](src/tools/tap_lasso.md) | the lasso outline (drag or tap corners) |
| [`src/tools/wand.rs`](src/tools/wand.md) | magic select (the wand) |

### Krita brush engine (`src/tools/krita/`)

| File | What it is |
|---|---|
| [`src/tools/krita/composite.rs`](src/tools/krita/composite.md) | Krita's 8-bit blending of dabs |
| [`src/tools/krita/curve.rs`](src/tools/krita/curve.md) | Krita's sensor curves (cubic splines) |
| [`src/tools/krita/defaults.rs`](src/tools/krita/defaults.md) | the 24 Krita brush presets built into the app |
| [`src/tools/krita/distance.rs`](src/tools/krita/distance.md) | what a Krita stroke remembers between dabs |
| [`src/tools/krita/edit.rs`](src/tools/krita/edit.md) | editing a Krita preset as a list of overrides |
| [`src/tools/krita/kpp.rs`](src/tools/krita/kpp.md) | reading Krita brush presets (.kpp) |
| [`src/tools/krita/kpp_write.rs`](src/tools/krita/kpp_write.md) | writing Krita brush presets (.kpp) |
| [`src/tools/krita/mod.rs`](src/tools/krita/mod.md) | Krita's pixel brush engine, ported |
| [`src/tools/krita/options.rs`](src/tools/krita/options.md) | the list of curve options a Krita brush reads |
| [`src/tools/krita/qt.rs`](src/tools/krita/qt.md) | Qt's painting maths, reproduced |
| [`src/tools/krita/random.rs`](src/tools/krita/random.md) | Krita's random number source |
| [`src/tools/krita/sensors.rs`](src/tools/krita/sensors.md) | sensors: turning pen input into dab size, rotation, opacity |
| [`src/tools/krita/spacing.rs`](src/tools/krita/spacing.md) | dab spacing and the path walker |
| [`src/tools/krita/speed.rs`](src/tools/krita/speed.md) | the speed sensor's input |
| [`src/tools/krita/texture.rs`](src/tools/krita/texture.md) | Krita brush textures (the Pattern option) |
| [`src/tools/krita/tip/auto.rs`](src/tools/krita/tip/auto.md) | generated ("auto") brush tips |
| [`src/tools/krita/tip/gbr.rs`](src/tools/krita/tip/gbr.md) | GIMP brush tips (.gbr and .gih) |
| [`src/tools/krita/tip/mod.rs`](src/tools/krita/tip/mod.md) | bitmap brush tips |
| [`src/tools/krita/tip/simd_math.rs`](src/tools/krita/tip/simd_math.md) | Krita's exact floating-point maths |

### User interface (`src/ui/`)

| File | What it is |
|---|---|
| [`src/ui/brush_picker.rs`](src/ui/brush_picker.md) | the brush picker grid |
| [`src/ui/canvas_gpu.rs`](src/ui/canvas_gpu.md) | drawing the canvas on the GPU |
| [`src/ui/color_wheel.rs`](src/ui/color_wheel.md) | the HSL colour wheel and Color panel |
| [`src/ui/curve_editor.rs`](src/ui/curve_editor.md) | the sensor curve editor widget |
| [`src/ui/expr.rs`](src/ui/expr.md) | arithmetic in number fields |
| [`src/ui/krita_editor.rs`](src/ui/krita_editor.md) | the Krita brush editor |
| [`src/ui/krita_import.rs`](src/ui/krita_import.md) | the Import Krita brushes window |
| [`src/ui/mod.rs`](src/ui/mod.md) | the ui folder |
| [`src/ui/shell.rs`](src/ui/shell.md) | the whole app window: panels, menus, canvas input and overlays |
| [`src/ui/sweep_tests.rs`](src/ui/sweep_tests.md) | headless layout sweep over every window (test-only) |
| [`src/ui/theme.rs`](src/ui/theme.md) | colour schemes, fonts and icon buttons |
| [`src/ui/tracks.rs`](src/ui/tracks.md) | the timeline's layer tracks |
| [`src/ui/widgets.rs`](src/ui/widgets.md) | custom widgets |

### Input (`src/input/`)

| File | What it is |
|---|---|
| [`src/input/button_drag.rs`](src/input/button_drag.md) | what middle / right button (and pen barrel button) drags do |
| [`src/input/clipboard_keys.rs`](src/input/clipboard_keys.md) | making Ctrl+C / Ctrl+X / Ctrl+V usable as shortcuts |
| [`src/input/mod.rs`](src/input/mod.md) | the input folder's table of contents |
| [`src/input/msg_hook.rs`](src/input/msg_hook.md) | peeking at raw Windows messages before egui does |
| [`src/input/pen_record.rs`](src/input/pen_record.md) | record a few seconds of pen vs. cursor positions to a CSV file |
| [`src/input/pointer.rs`](src/input/pointer.md) | one pointer sample (position, pressure, tilt, time) |
| [`src/input/screen_pixel.rs`](src/input/screen_pixel.md) | read the OS cursor position and grab screen pixels |
| [`src/input/shortcuts.rs`](src/input/shortcuts.md) | key bindings, the mouse-wheel map and `shortcuts.toml` |
| [`src/input/tablet.rs`](src/input/tablet.md) | pen tablet input through the Wintab driver |

### Files: import, export, formats (`src/io/`)

| File | What it is |
|---|---|
| [`src/io/composite.rs`](src/io/composite.md) | flatten all layers of a frame into one picture |
| [`src/io/fit.rs`](src/io/fit.md) | fit an image inside the canvas, centred, without stretching |
| [`src/io/frame_order.rs`](src/io/frame_order.md) | which timeline frame each exported frame shows |
| [`src/io/gif_export.rs`](src/io/gif_export.md) | write the animation as an animated GIF |
| [`src/io/gif_import.rs`](src/io/gif_import.md) | decode an animated GIF into one canvas per frame |
| [`src/io/image_import.rs`](src/io/image_import.md) | importing a still image (file or clipboard) |
| [`src/io/kra/mod.rs`](src/io/kra/mod.md) | reading and writing Krita .kra files |
| [`src/io/kra/tiles.rs`](src/io/kra/tiles.md) | Krita's tiled pixel files |
| [`src/io/kra/xml.rs`](src/io/kra/xml.md) | the XML parts of a .kra |
| [`src/io/legacy.rs`](src/io/legacy.md) | frozen layouts of old `.anim` versions |
| [`src/io/mod.rs`](src/io/mod.md) | the list of file-format modules |
| [`src/io/mp4_export.rs`](src/io/mp4_export.md) | MP4 export through ffmpeg |
| [`src/io/png_import.rs`](src/io/png_import.md) | importing a PNG sequence as a layer |
| [`src/io/png_save.rs`](src/io/png_save.md) | saving one frame as PNG, or copying it |
| [`src/io/png_seq.rs`](src/io/png_seq.md) | PNG sequence export |
| [`src/io/project_file.rs`](src/io/project_file.md) | saving and opening `.anim` projects |
| [`src/io/sprite_sheet.rs`](src/io/sprite_sheet.md) | sprite-sheet export |
| [`src/io/video_import.rs`](src/io/video_import.md) | importing frames from a video (ffmpeg) |
| [`src/io/webp_export.rs`](src/io/webp_export.md) | animated WebP export |
| [`src/io/webp_import.rs`](src/io/webp_import.md) | animated WebP import |

### Timeline (`src/timeline/`)

| File | What it is |
|---|---|
| [`src/timeline/mod.rs`](src/timeline/mod.md) | the timeline folder |
| [`src/timeline/onion.rs`](src/timeline/onion.md) | onion-skin settings and which ghosts to draw |
| [`src/timeline/playback.rs`](src/timeline/playback.md) | the playback clock |

### Brush library (`src/brush_library/`)

| File | What it is |
|---|---|
| [`src/brush_library/import.rs`](src/brush_library/import.md) | importing presets from an installed Krita |
| [`src/brush_library/mod.rs`](src/brush_library/mod.md) | the brush library (every preset the picker offers) |

### Krita link

| File | What it is |
|---|---|
| [`krita-plugin/animator_link.desktop`](krita-plugin/animator_link.desktop.md) | the plugin's manifest for Krita |
| [`krita-plugin/animator_link/__init__.py`](krita-plugin/animator_link/__init__.md) | plugin entry point |
| [`krita-plugin/animator_link/extension.py`](krita-plugin/animator_link/extension.md) | the plugin running inside Krita |
| [`krita-plugin/animator_link/mailbox.py`](krita-plugin/animator_link/mailbox.md) | the file protocol, Krita side |
| [`krita-plugin/tests/test_mailbox.py`](krita-plugin/tests/test_mailbox.md) | tests for the plugin's protocol |
| [`src/krita_helper.rs`](src/krita_helper.md) | installing the Krita plugin and the mailbox files |
| [`src/krita_link.rs`](src/krita_link.md) | the two-way live link with Krita |
| [`src/krita_stress.rs`](src/krita_stress.md) | stress tests for the Krita link (test-only) |

### Everything else

| File | What it is |
|---|---|
| [`src/color.rs`](src/color.md) | HSL colours and colour harmonies |
| [`src/platform.rs`](src/platform.md) | rounded corners and a border for the frameless window (Windows) |
| [`src/undo.rs`](src/undo.md) | undo and redo history |

_116 pages._
