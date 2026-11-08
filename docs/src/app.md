# `src/app.rs` — the app's central state (`AppState`)

> **Source:** [src/app.rs](../../src/app.rs) · **Module:** `crate::app` · **Size:** 12876 lines · Line links as of commit `67a7cb9`

## What this file is for

This is the heart of the program. It defines **`AppState`**, one big struct that holds
*everything* the running app knows: the open project (layers, cells, timeline), the active
tool and brush, the stroke being drawn, the selection, the camera, the canvas view (zoom /
pan / rotate), undo history, GPU textures for every drawing, open dialogs, background jobs,
the live link to Krita and every workspace preference. If you think of the app as a web page,
`AppState` is the single store (like one Redux store or one big React context) and the UI in
[ui/shell.rs](ui/shell.md) is the component tree that reads it and calls its methods.

The app uses **egui**, an *immediate-mode* GUI: there is no retained DOM. Every frame (up to
~60 times a second) eframe calls `AppState::update` (in the `impl eframe::App for AppState`
block near [line 7341](../../src/app.rs#L7341)). `update` does the per-frame housekeeping —
polls the pen tablet, reads keys and runs shortcuts, advances playback, keeps the
transform/camera/grid edit buffers in step with the current frame, polls background jobs
(imports, exports, saves, the Krita link), frees unreachable drawings, packs idle cells,
uploads changed pixels to the GPU — and then calls `ui::shell::draw(self, ctx)`, which lays
out the whole window from the state. Pointer events from the canvas come back into
`AppState` through methods such as `pointer_down` / `pointer_move` / `pointer_up`, and menu
items and keyboard shortcuts go through `dispatch(action)`.

Besides `AppState`, the file defines many small helper types: dialog configs (new project,
export, frame-range import), drag-in-progress records (shape, fill, selection box, camera
box, grid), the saved-preferences struct `UiPrefs`, and the bookkeeping for GPU textures
(`Dirty`, `CellTex`, `TexPlan`, the texture budget). Because the file is so long, several
feature areas live in child modules under `src/app/`, each adding its own
`impl AppState { … }` block — Rust lets one type's methods be spread over several files of
the same module tree:

| Child module | Page | What it adds to `AppState` |
|---|---|---|
| `brush_sizes` (public) | [app/brush_sizes.md](app/brush_sizes.md) | Recent and pinned brush sizes per tool, size detents, size swap |
| `button_drag` | [app/button_drag.md](app/button_drag.md) | Middle/right-button canvas drags: scrub, flip, tool adjust, swatch pie |
| `grids` | [app/grids.md](app/grids.md) | Perspective grids per project file, grid keys, grid drags and their undo |
| `groups` | [app/groups.md](app/groups.md) | Layer groups drawn as one image, group masks for clipping |
| `lasso_fill` | [app/lasso_fill.md](app/lasso_fill.md) | The Fill tool's lasso mode |
| `liquify` | [app/liquify.md](app/liquify.md) | The Liquify warp session |
| `orient` | [app/orient.md](app/orient.md) | Mirroring / turning the art itself |
| `select` | [app/select.md](app/select.md) | The selection mask, selection gestures, floating pixels, copy/paste |

This page is long. Part 1 (this part) covers the types, the free texture-planning helpers and
`AppState`'s first methods: construction, reset, New-project presets, pointer samples, brush
helpers and dirty-marking. Later parts cover the rest of the methods in source order, the
`eframe::App` implementation, the Krita link, and the tests.

## How it connects

- **Used by:**
  - [main.rs](main.md) — declares `mod app;` and builds the state with `AppState::new(cc)`
    inside `eframe::run_native`.
  - [ui/shell.rs](ui/shell.md) — the whole UI; draws from `AppState` every frame and calls its
    methods (dispatch, pointer handlers, presets, camera, layers…). Also imports `CameraFraming`,
    `ExportKind`, `FillMode`, `LassoHeld`, `NavKind`, `PanelId`, `SelGesture`, `MP4_PRESETS`,
    `LayerRename`, `ProjectPreset`, `FadeOthers`, `ButtonDragState`, `premultiplied_image`.
  - [ui/tracks.rs](ui/tracks.md) (timeline tracks; `CamKeyDrag`),
    [ui/brush_picker.rs](ui/brush_picker.md), [ui/color_wheel.rs](ui/color_wheel.md),
    [ui/krita_import.rs](ui/krita_import.md), [ui/krita_editor.rs](ui/krita_editor.md),
    [ui/canvas_gpu.rs](ui/canvas_gpu.md) (`premultiplied_image`) — panels and windows that take
    `&mut AppState`.
  - Test harnesses: [ui/sweep_tests.rs](ui/sweep_tests.md),
    [tools/golden_tests.rs](tools/golden_tests.md).
- **Uses (almost everything):**
  - Document model: [doc/project.rs](doc/project.md) (`Project`, `Block`),
    [doc/canvas.rs](doc/canvas.md) (`Canvas`, `DirtyRect`, `par_rows`),
    [doc/layer.rs](doc/layer.md) (`CellId`), [doc/camera.rs](doc/camera.md),
    [doc/transform.rs](doc/transform.md), [doc/clip.rs](doc/clip.md),
    [doc/orient.rs](doc/orient.md).
  - Tools: [tools/mod.rs](tools/mod.md) (`ActiveTool`, `BrushSettings`, smoothing),
    [tools/stroke.rs](tools/stroke.md), [tools/ribbon.rs](tools/ribbon.md),
    [tools/fill.rs](tools/fill.md), [tools/shape.rs](tools/shape.md),
    [tools/perspective.rs](tools/perspective.md), [tools/selection.rs](tools/selection.md),
    [tools/select_mask.rs](tools/select_mask.md), [tools/lasso.rs](tools/lasso.md),
    [tools/wand.rs](tools/wand.md), [tools/axis_lock.rs](tools/axis_lock.md),
    [tools/krita/mod.rs](tools/krita/mod.md).
  - Input: [input/tablet.rs](input/tablet.md), [input/pointer.rs](input/pointer.md),
    [input/shortcuts.rs](input/shortcuts.md), [input/button_drag.rs](input/button_drag.md),
    [input/pen_record.rs](input/pen_record.md), [input/clipboard_keys.rs](input/clipboard_keys.md).
  - Timeline: [timeline/playback.rs](timeline/playback.md), [timeline/onion.rs](timeline/onion.md).
  - Files: [io/kra/mod.rs](io/kra/mod.md), [io/frame_order.rs](io/frame_order.md),
    [io/image_import.rs](io/image_import.md), [io/project_file.rs](io/project_file.md) and the
    other exporters/importers under `io/`.
  - [krita_link.rs](krita_link.md), [brush_library/mod.rs](brush_library/mod.md),
    [brush_library/import.rs](brush_library/import.md), [color.rs](color.md),
    [undo.rs](undo.md), [ui/canvas_gpu.rs](ui/canvas_gpu.md), [ui/theme.rs](ui/theme.md),
    [platform.rs](platform.md).

```text
main.rs ── eframe::run_native ──► AppState::new(cc)
                                        │
        every frame: eframe calls  AppState::update(ctx)
                                        │
   track layer change · poll pen · handle_keys → dispatch(action)
   advance playback · sync edit buffers · poll jobs / save / Krita
   collect_cells · pack_idle_cells · warm_pre · sync_textures (GPU uploads)
                                        │
                                        ▼
                         ui::shell::draw(&mut AppState, ctx)
                 panels, timeline, canvas ── pointer_down/move/up ──┐
                                        ▲                          │
                                        └──── AppState methods ◄───┘
```

## Rust concepts in this file

**One struct, methods in many files.** `AppState` is declared once, but `impl AppState { … }`
blocks appear here and in each child module (`mod select;` etc.). A child module can read the
parent's private fields, which is why the feature code could be split out.
```rust
mod select;               // loads src/app/select.rs as crate::app::select
impl AppState {
    pub fn mark_dirty(&mut self, id: CellId) { /* … */ }
}
```
```js
// Closest JS idea: one class whose methods are added from several files
Object.assign(AppState.prototype, selectMethods, gridMethods);
```
More: [Modules and visibility](../rust-primer.md#modules-and-visibility),
[impl blocks and methods](../rust-primer.md#impl-blocks-and-methods)

**`Option<T>` instead of `null`.** Almost every "maybe there is one" field is an `Option`:
`save_job: Option<SaveJob>` is either `Some(job)` or `None`. The compiler forces you to handle
`None` before using the value.
```rust
let Some(job) = self.save_job.take() else { return; };   // take() leaves None behind
```
```js
const job = this.saveJob; this.saveJob = null;
if (job == null) return;
```
More: [Option and Result](../rust-primer.md#option-and-result)

**Enums that carry data, and `match` on pairs.** `Dirty` is either `Clean`, `Full`, or
`Rect(DirtyRect)` with a rectangle inside. `Dirty::with` matches on a *tuple* of two values,
and `|` lets one arm cover several shapes.
```rust
match (self, more) {
    (Dirty::Full, _) | (_, Dirty::Full) => Dirty::Full,
    (Dirty::Clean, d) | (d, Dirty::Clean) => d,
    (Dirty::Rect(a), Dirty::Rect(b)) => Dirty::Rect(union_rect(Some(a), b)),
}
```
More: [Enums and match](../rust-primer.md#enums-and-match)

**`Default`, `derive` and `#[serde(default)]` for saved preferences.** `UiPrefs` derives
`Serialize`/`Deserialize` so eframe can store it, and `#[serde(default)]` fills any field
missing from an older prefs blob with the value from `impl Default for UiPrefs` instead of
failing the whole load.
```rust
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct UiPrefs { show_panels: bool, /* … */ }
```
```js
const prefs = { ...DEFAULT_PREFS, ...JSON.parse(saved) }; // missing keys get defaults
```
More: [Traits and derive](../rust-primer.md#traits-and-derive),
[Serde and file formats](../rust-primer.md#serde-and-file-formats)

**Channels for background work.** Long jobs (export, video probe, save) run on another thread
and send their result back through an `mpsc` channel. The UI keeps a `Receiver` and checks it
each frame with `try_recv()`, so the window never blocks.
```rust
pub enum BgJob {
    Export(Receiver<Result<String>>),
    // …
}
```
```js
// Like keeping a Promise and checking each animation frame whether it settled
```
More: [Threads and channels](../rust-primer.md#threads-and-channels)

**Immediate-mode UI.** Nothing in the state "listens" for changes; egui redraws the whole UI
from `AppState` every frame. GPU textures are the exception — they are expensive, so the file
tracks exactly which pixels changed (`Dirty`) and uploads only those.
More: [egui and immediate-mode UI](../rust-primer.md#egui-and-immediate-mode-ui)

### Rust concepts in this stretch
**`Arc` handles, and telling whether one is shared.** Cells are stored as `Arc<Canvas>`: a
reference-counted pointer that several owners (the project, an undo step, a save running on
another thread) can hold to the *same* buffer without copying it. Two helpers matter here:
```rust
// Same buffer? Compares the pointers, not the pixels.
let changed = !Arc::ptr_eq(&before, after);
// Only I hold it? Then I may change it in place.
let Some(c) = Arc::get_mut(cell) else { continue };
```
```js
// Roughly: identity comparison, and "nobody else has a reference"
const changed = before !== after;
if (refCount(cell) === 1) mutateInPlace(cell);
```
A write to a shared cell copies it first ("copy on write"), so a changed pointer means a
changed cell. More: [Smart pointers](../rust-primer.md#smart-pointers)

**Background work with a thread and a channel.** Slow jobs (decoding a GIF, writing an MP4)
run on a worker thread; the result comes back through a channel the UI checks once a frame.
```rust
let (tx, rx) = mpsc::channel();
thread::spawn(move || {
    let _ = tx.send(decode(&path));
});
self.bg_job = Some(BgJob::FramesDecode { rx, format });
```
```js
// JS equivalent: start a Worker and keep a handle to poll for its message
const worker = new Worker("decode.js"); worker.postMessage(path);
state.bgJob = { kind: "FramesDecode", worker, format };
```
`move` makes the closure take ownership of `path` and `tx`, so they travel to the new thread.
More: [Threads and channels](../rust-primer.md#threads-and-channels)

**Returning a closure.** `about_group_pivot` builds and returns a function. Its return type,
`impl Fn(Transform) -> Transform`, means "some function from a `Transform` to a `Transform`".
```rust
fn about_group_pivot(&self, k: f32, rot: f32) -> impl Fn(Transform) -> Transform {
    let (px, py) = /* pivot */;
    move |t: Transform| Transform { /* ... uses px, py, k, rot ... */ }
}
```
```js
const aboutGroupPivot = (k, rot) => { const [px, py] = pivot; return (t) => ({ /* ... */ }); };
```
More: [Closures](../rust-primer.md#closures)

**Plain function pointers.** `open_frames_import` takes `decode: fn(&Path) -> Result<Vec<Canvas>>`,
a pointer to an ordinary named function (no captured variables), so callers pass
`crate::io::gif_import::decode_all` directly, like passing a function reference in JS.

### Rust concepts in this stretch
**Passing an edit as a closure.** Many methods here hand a small function to
`structural_edit`, which takes an undo snapshot, runs the function on the project, takes a
second snapshot and pushes both as one undo step.
```rust
self.structural_edit(false, |p| p.camera = Camera::default());
```
```js
// JS equivalent: a callback that receives the project to mutate
structuralEdit(false, (p) => { p.camera = new Camera(); });
```
The parameter type is `impl FnOnce(&mut Project)`: "any function that can be called once with a
mutable reference to a `Project`". More: [Closures](../rust-primer.md#closures)

**`let ... else` for early returns.** Unwraps an `Option` or bails out of the function.
```rust
let Some(l) = self.project.layers.get_mut(li) else {
    return;
};
```
```js
const l = project.layers[li];
if (l === undefined) return;
```
More: [Enums and match](../rust-primer.md#enums-and-match)

**`bool::then`.** `cond.then(|| value)` gives `Some(value)` when `cond` is true, else `None`
(JS: `cond ? value : null`). Used in `stabilize_active_layer`.

## Types

### `struct NewProjectConfig`
[src/app.rs:60](../../src/app.rs#L60)

What the **New project** dialog is editing: the size and frame rate the new project will get,
plus the dialog's own preset-editing state. Lives in `AppState::new_project_cfg`.

| Field | Type | Meaning |
|---|---|---|
| `width` | `u32` | New canvas width in pixels. |
| `height` | `u32` | New canvas height in pixels. |
| `fps` | `f32` | Frames per second. |
| `preset_name` | `String` | The "save as preset" name box. |
| `preset_rename` | `Option<LayerRename>` | A saved preset being renamed inline, if any (reuses the layer-rename record: index, text buffer, focus flag). |
| `clipboard` | `crate::io::image_import::ClipboardWatch` | Watches the size of the image on the system clipboard, for the "Clipboard" size preset. See [io/image_import.rs](io/image_import.md). |

### `struct ProjectPreset`
[src/app.rs:77](../../src/app.rs#L77)

One saved New-project size. It is a **workspace preference**, not project data: the list is
saved with the app's preferences (`UiPrefs::project_presets`), so it follows the artist from
file to file. Derives `Clone, Debug, PartialEq, Serialize, Deserialize` and has
`#[serde(default)]` so older saved lists still load.

**Used by:** [ui/shell.rs](ui/shell.md) (the New project dialog and its tests).

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | The preset's name as shown in the dialog. |
| `width` | `u32` | Canvas width. |
| `height` | `u32` | Canvas height. |
| `fps` | `f32` | Frame rate. |
| `is_default` | `bool` | The "starred" preset New project opens on. At most one preset has it set. |

### `const MP4_PRESETS`
[src/app.rs:99](../../src/app.rs#L99)

```rust
pub const MP4_PRESETS: &[&str] = &["ultrafast", "fast", "medium", "slow", "veryslow"];
```

The libx264 encoder speed presets the MP4 export dialog offers, fastest (biggest file) to
slowest (smallest file). `&[&str]` is a borrowed list of string slices — a constant array of
strings. Used by `start_export` and the export dialog in [ui/shell.rs](ui/shell.md).

### `struct Mp4ExportConfig`
[src/app.rs:103](../../src/app.rs#L103)

Encoder settings for the MP4 export.

| Field | Type | Meaning |
|---|---|---|
| `crf` | `u32` | libx264 "constant rate factor", 0..=51. Lower = better quality and a bigger file. |
| `preset_idx` | `usize` | Index into [`MP4_PRESETS`](#const-mp4_presets). |

### `enum ExportKind`
[src/app.rs:122](../../src/app.rs#L122)

Which file the shared export dialog is about to write. `Copy` + `PartialEq`, so it can be
passed by value and compared with `==`.

| Variant | Meaning |
|---|---|
| `PngSequence` | One PNG file per frame. |
| `Gif` | Animated GIF. |
| `Webp` | Animated WebP. |
| `Mp4` | MP4 video (through ffmpeg/libx264). |
| `SpriteSheet` | All frames tiled into one image. |

### `struct ExportConfig`
[src/app.rs:156](../../src/app.rs#L156)

Everything the shared export dialog edits. One struct serves every format: the frame range
applies to all of them, and the format-specific parts are small. Lives in
`AppState::export_cfg`.

| Field | Type | Meaning |
|---|---|---|
| `kind` | `ExportKind` | Which format is being exported. |
| `start` | `usize` | First frame of the range (inclusive). |
| `end` | `usize` | Last frame of the range (inclusive). |
| `mp4` | `Mp4ExportConfig` | MP4 quality and speed. |
| `sheet_columns` | `usize` | Sprite-sheet columns; `0` means "near-square". |
| `sheet_padding` | `u32` | Pixels between sprite-sheet tiles. |
| `ping_pong` | `bool` | Play the range forward then back (MP4, PNG sequence, GIF, WebP). |
| `loop_on` | `bool` | Repeat the range to fill `loop_secs`, rounded up to whole loops (MP4 and PNG sequence only — GIF and WebP repeat by themselves). |
| `loop_secs` | `f32` | How long the repeated output should last. |
| `webp_stop` | `bool` | A WebP stops after `webp_plays` plays instead of looping forever. |
| `webp_plays` | `u16` | Number of WebP plays when `webp_stop` is on. |

### `struct ShapeDrag`
[src/app.rs:208](../../src/app.rs#L208)

A Shape-tool drag in progress. It is only a preview until the pointer lifts; then the shape is
rasterised (turned into pixels) into the target cell. Lives in `AppState::shape_drag`.

| Field | Type | Meaning |
|---|---|---|
| `kind` | `ShapeKind` | Line, rectangle, ellipse… (see [tools/mod.rs](tools/mod.md)). |
| `start` | `(f32, f32)` | Where the drag was anchored. |
| `end` | `(f32, f32)` | Where the drag has got to. |
| `plane` | `Option<GridPlane>` | The perspective-grid plane the shape lies on, in the target cell's pixels, taken at press — so toggling snap mid-drag can't change the shape. `None` = flat. See [tools/shape.rs](tools/shape.md). |

### `struct FillDrag`
[src/app.rs:220](../../src/app.rs#L220)

A Fill press still under the pen. The bucket fills at pen-down; dragging before the pen lifts
re-runs the fill with a new **gap** (drag sideways: how big a hole in the line art the fill
may jump) or **expand** (drag up/down: how far the fill grows under the lines). The whole
press becomes one undo step on release. Lives in `AppState::fill_drag`.

| Field | Type | Meaning |
|---|---|---|
| `session` | `crate::tools::fill::FillSession` | The fill being re-run (see [tools/fill.rs](tools/fill.md)). |
| `anchor` | `Option<egui::Pos2>` | Where the press landed, in screen points; drags are measured from here. (private) |
| `base_gap` | `u8` | The gap slider value at the press. (private) |
| `base_expand` | `u8` | The expand slider value at the press. (private) |

### `struct GridDrag`
[src/app.rs:238](../../src/app.rs#L238)

A drag on a perspective grid. Like `SelDrag`, every frame of the drag is solved against the
grid's corners *as they were at the press* (in document space) rather than adding up small
moves, so nothing drifts. (The doc comment just above it in the source, about the
"floating selection's transform box", belongs to `SelDrag` further down.) Used in
[app/grids.rs](app/grids.md).

| Field | Type | Meaning |
|---|---|---|
| `grid` | `usize` | Index of the grid being dragged. |
| `grab` | `GridGrab` | Which part was grabbed (corner, edge, vanishing point…; see [tools/perspective.rs](tools/perspective.md)). |
| `start` | `[f32; 2]` | Pointer position at the press. |
| `corners` | `[[f32; 2]; 4]` | The grid's four corners at the press. |
| `extra` | `[Option<[f32; 2]>; perspective::MAX_EXTRA_VPS]` | Extra vanishing points, where they were at the press (`None` = not shown). |
| `space` | `perspective::Space` | Where the grid's coordinates land in the document at the press; the camera or layer the grid follows doesn't move mid-drag. |

### `const HORIZON_DETACH_PX`
[src/app.rs:252](../../src/app.rs#L252)

```rust
const HORIZON_DETACH_PX: f32 = 40.0;
```

`40.0` and `10.0` screen points: how far an extra vanishing point sitting on the horizon can be
pulled off it before it lets go, and how close a free one must come to stick back on. Used by
[app/grids.rs](app/grids.md).

### `const HORIZON_ATTACH_PX`
[src/app.rs:253](../../src/app.rs#L253)

```rust
const HORIZON_ATTACH_PX: f32 = 10.0;
```

`40.0` and `10.0` screen points: how far an extra vanishing point sitting on the horizon can be
pulled off it before it lets go, and how close a free one must come to stick back on. Used by
[app/grids.rs](app/grids.md).

### `struct SnapLock`
[src/app.rs:258](../../src/app.rs#L258)

Perspective-snap state for one stroke. Lives in `AppState::snap_lock`.

| Field | Type | Meaning |
|---|---|---|
| `start` | `[f32; 2]` | Where the stroke started. |
| `dir` | `Option<[f32; 2]>` | The grid direction the stroke is locked to, once the pointer has moved far enough to tell; `None` until then. |

### `struct MidKey`
[src/app.rs:268](../../src/app.rs#L268)

A key held **partway through a drag** — the "move" key (slides a shape or box along) and the
"centre" key (grows it from its middle). Sampled once a frame, like Shift. If the key was
already down when the drag was pressed, it was what *chose* that drag (a pan, a rotate, a
brush adjust), so it only counts once let go and pressed again — the same rule
`AppState::axis_lock_on` uses for Shift. Derives `Default` (both flags false).

| Field | Type | Meaning |
|---|---|---|
| `held` | `bool` | The key is down this frame. |
| `stale` | `bool` | The key has been down since before the current drag began, so it does not count yet. |

### `struct StrokeAxis`
[src/app.rs:292](../../src/app.rs#L292)

A stroke held to an axis by Shift. Lives in `AppState::stroke_axis`.

| Field | Type | Meaning |
|---|---|---|
| `from` | `[f32; 2]` | The point it is held from, in document space. |
| `angle` | `Option<f32>` | The angle of the axis it keeps to, once the pointer has moved far enough to tell. |

### `struct SelDrag`
[src/app.rs:298](../../src/app.rs#L298)

A drag in progress on the floating selection's transform box (move / scale / rotate handles).
`pose` is the selection's pose at the moment the handle was pressed; a scale or rotation is
re-solved against it every frame rather than accumulated, so a slow drag and a fast one that
end in the same place give the same result, and floating-point error cannot creep in over a
long drag. Used in [app/select.rs](app/select.md).

| Field | Type | Meaning |
|---|---|---|
| `grab` | `Grab` | Which handle was grabbed (see [tools/selection.rs](tools/selection.md)). (private) |
| `start` | `(f32, f32)` | Where the pointer pressed, in cell space. (private) |
| `pose` | `Pose` | The selection's position/scale/rotation at the press. (private) |

### `struct View`
[src/app.rs:313](../../src/app.rs#L313)

The canvas **view** transform, applied on top of the "fit to window" base scale. It only
changes how the canvas is shown: export never reads `View`, so a flipped view can never reach
a file. Each part can be reset on its own. Lives in `AppState::view`.

| Field | Type | Meaning |
|---|---|---|
| `zoom` | `f32` | Multiplies the fit-to-window scale. |
| `pan` | `egui::Vec2` | Shift in screen pixels. |
| `rotation` | `f32` | Rotation in radians about the canvas centre. |
| `flip_x` | `bool` | Mirror the view left–right (the classic animator's drawing check). |
| `flip_y` | `bool` | Mirror the view top–bottom. |

### `struct SelTint`
[src/app.rs:334](../../src/app.rs#L334)

The dimming wash drawn over everything **outside** the selection (when `tint_outside` is on).
Lives in `AppState::sel_tint`; built in [app/select.rs](app/select.md).

| Field | Type | Meaning |
|---|---|---|
| `ver` | `u64` | The `sel_ver` (selection version number) it was built for; a different version means rebuild. |
| `rect` | `(f32, f32, f32, f32)` | Where it sits in the document: x0, y0, x1, y1. |
| `tex` | `TextureHandle` | The GPU texture of the wash. |

### `enum NavKind`
[src/app.rs:344](../../src/app.rs#L344)

Which non-drawing canvas gesture the current drag is doing. Stored in `AppState::nav_drag`;
read by [ui/shell.rs](ui/shell.md).

| Variant | Meaning |
|---|---|
| `Pan` | Slide the view (or layer/camera when retargeted). |
| `Zoom` | Zoom. |
| `Rotate` | Rotate. |

### `const DEFAULT_BG_COLOR`
[src/app.rs:351](../../src/app.rs#L351)

```rust
const DEFAULT_BG_COLOR: [f32; 3] = [0.12, 0.12, 0.13];
```

`[0.12, 0.12, 0.13]` — the dark grey canvas backdrop (RGB, 0..1) before the artist picks one.

### `const MAX_CAMERA_SIDE`
[src/app.rs:354](../../src/app.rs#L354)

```rust
const MAX_CAMERA_SIDE: u32 = 16384;
```

`16384` — the longest side a camera frame may have (also a canvas's limit).

### `const DEFAULT_PANEL_PADDING`
[src/app.rs:356](../../src/app.rs#L356)

```rust
pub const DEFAULT_PANEL_PADDING: u8 = 8;
```

`8` and `48` points: Settings > Appearance's panel padding (gap between panels and the window
edges) as it starts, and the most it offers. `MAX_PANEL_PADDING` is used by the slider in
[ui/shell.rs](ui/shell.md).

### `const MAX_PANEL_PADDING`
[src/app.rs:358](../../src/app.rs#L358)

```rust
pub const MAX_PANEL_PADDING: u8 = 48;
```

`8` and `48` points: Settings > Appearance's panel padding (gap between panels and the window
edges) as it starts, and the most it offers. `MAX_PANEL_PADDING` is used by the slider in
[ui/shell.rs](ui/shell.md).

### `const MIN_FREE_CAMERA_SIDE`
[src/app.rs:362](../../src/app.rs#L362)

```rust
pub const MIN_FREE_CAMERA_SIDE: u32 = 16;
```

`16` — the smallest side a free-shape camera box gives the camera, so a drag along one axis
doesn't leave it a sliver.

### `struct CamDrag`
[src/app.rs:368](../../src/app.rs#L368)

The camera's frame held by one of its handles: what was grabbed, where, and the camera as it
was. Every step of the drag is solved from that, so nothing drifts. Lives in
`AppState::cam_drag`.

| Field | Type | Meaning |
|---|---|---|
| `grab` | `CamGrab` | Which part of the frame was grabbed (see [doc/camera.rs](doc/camera.md)). |
| `from` | `(f32, f32)` | Where the press landed. |
| `start` | `Camera` | The camera at the press. |

### `const CAMERA_TAP_PX`
[src/app.rs:376](../../src/app.rs#L376)

```rust
const CAMERA_TAP_PX: f32 = 3.0;
```

`3.0` screen pixels: how far a press in Draw-camera mode may wander and still count as a tap
(the same allowance as the selection lasso).

### `struct CamBox`
[src/app.rs:382](../../src/app.rs#L382)

A box being dragged out in **Draw camera** mode, in document space. Lives in
`AppState::cam_box`.

| Field | Type | Meaning |
|---|---|---|
| `from` | `(f32, f32)` | Where the press landed. |
| `to` | `(f32, f32)` | Where the drag has got to. |
| `moved` | `bool` | Whether the drag has ever gone past a tap. |

### `struct CamKeyDrag`
[src/app.rs:391](../../src/app.rs#L391)

A camera key picked up off the timeline ruler. Lives in `AppState::cam_key_drag`; created by
[ui/tracks.rs](ui/tracks.md).

| Field | Type | Meaning |
|---|---|---|
| `from` | `usize` | The frame the key sits on. |
| `to` | `usize` | The frame the drag has reached. |

### `enum CameraFraming`
[src/app.rs:398](../../src/app.rs#L398)

One-click ways to frame the canvas with the camera (Camera panel buttons in
[ui/shell.rs](ui/shell.md); applied by `frame_camera`).

| Variant | Meaning |
|---|---|
| `Fit` | Centred, pulled out until the whole canvas is in the frame. |
| `Fill` | Centred, as wide as the frame goes while showing nothing but canvas. |
| `OneToOne` | One canvas pixel per exported pixel, where the camera stands. |
| `Centre` | Back over the middle of the canvas, zoom unchanged. |

### `const UI_PREFS_KEY`
[src/app.rs:410](../../src/app.rs#L410)

```rust
const UI_PREFS_KEY: &str = "ui_prefs";
```

`"ui_prefs"` — the key under which `UiPrefs` is stored in eframe's persistent storage.

### `struct UiPrefs`
[src/app.rs:422](../../src/app.rs#L422)

The **workspace preferences** that outlive a run but that egui's own memory doesn't cover.
(Panel positions, sizes and collapse state are saved by egui/eframe automatically; these are
plain `AppState` fields, so they are saved by hand.) `AppState::new` reads it from eframe
storage under `UI_PREFS_KEY`; `eframe::App::save` (near
[line 7358](../../src/app.rs#L7358)) writes it back on exit and every 30 s.
`#[serde(default)]` is what keeps a prefs blob from an older build loadable after a field is
added: without it, one missing key would fail the whole struct and silently reset every other
preference too. Most fields copy straight into the `AppState` field of the same name.

| Field | Type | Meaning |
|---|---|---|
| `show_panels` | `bool` | All floating panels shown (Tab toggles). |
| `show_mini_timeline` | `bool` | The small timeline bar shown while panels are hidden. |
| `frame_step` | `usize` | Frames moved per Prev/Next frame press. |
| `show_camera_guide` | `bool` | Draw the camera's frame over the canvas. |
| `dim_outside_camera` | `bool` | Darken what the camera can't see. |
| `camera_box_free` | `bool` | Draw-camera mode's box can take any shape and then sets the camera's size. |
| `show_layer_bounds` | `bool` | Outline the active layer's cell bounds. |
| `lock_brush_to_view` | `bool` | Brush size measured on screen instead of in document pixels. |
| `onion` | `OnionConfig` | Onion-skin tints and counts (see [timeline/onion.rs](timeline/onion.md)). |
| `auto_key_transform` | `bool` | Editing a layer transform writes a key on this frame. |
| `auto_key_draw` | `bool` | Drawing on a held frame breaks the hold into its own drawing first. |
| `invert_timeline_scroll` | `bool` | Which way the mouse wheel scrubs. Off (default): wheel down advances. |
| `lock_screen_axes` | `bool` | Shift partway through a drag locks to the screen's horizontal/vertical instead of the canvas's (default off). |
| `middle_drag` | `ButtonDrag` | What a middle-button drag on the canvas does (pen barrel buttons arrive as mouse buttons). See [input/button_drag.rs](input/button_drag.md). |
| `right_drag` | `ButtonDrag` | What a right-button drag does. |
| `loop_timeline` | `bool` | On (default): playback repeats the loop range and frame stepping wraps. Off: playback runs once and stepping clamps. |
| `smoothing` | `SmoothingOptions` | Old single line-smoothing setting. Read only to seed `pen_smoothing` from an older prefs file, and written as the pencil's for older builds. |
| `pen_smoothing` | `Vec<SmoothingOptions>` | Line smoothing per freehand tool, in `PEN_TOOLS` order (Pencil, Ink, Eraser). Empty in an older file, which gives every tool `smoothing`. |
| `tool_brushes` | `Option<Vec<BrushSettings>>` | Per-tool brush settings. `None` (older blob, or a changed tool list) falls back to the built-in defaults; see `restore_tool_brushes`. |
| `pens` | `Vec<PenSlot>` | The pen slots, each with its brush, smoothing, sizes, name and icon. Empty in a prefs file from before the slots, which makes the first two from that file's pencil and ink. |
| `palette` | `Vec<[u8; 3]>` | Pinned colour swatches. |
| `brush_sizes` | `Vec<brush_sizes::SizeMemory>` | Recent and pinned sizes per tool, by `ActiveTool::idx`. See [app/brush_sizes.rs](app/brush_sizes.md). |
| `track_frame_w` | `f32` | Frame column width in the timeline tracks — how zoomed in the artist likes them. |
| `perspective` | `PerspectiveConfig` | Grid display/snap switches. Its `grids` are now only a *seed*: grids from before they were kept per file, handed to any file the app has none kept for. |
| `grid_files` | `Vec<grids::GridFile>` | Each project file's grids, by path. Kept by the app, never in the `.anim`, never shared between files. See [app/grids.rs](app/grids.md). |
| `fade` | `FadeOthers` | How far "fade other layers" pushes the rest back. Only the amounts persist; the toggle starts off every launch. |
| `krita_path` | `Option<PathBuf>` | Where Krita is installed, once found or picked for Edit in Krita. |
| `project_presets` | `Vec<ProjectPreset>` | Saved New-project presets, in creation order. |
| `sel_shape` | `SelShape` | What a selection drag draws (freehand, box, ellipse…). |
| `sel_amount` | `u32` | Pixels a Grow / Shrink / Feather takes. |
| `wand` | `WandOptions` | Magic-select tolerance, gap, expand and switches (see [tools/wand.rs](tools/wand.md)). |
| `fill_mode` | `FillMode` | Bucket or lasso fill (see [app/lasso_fill.rs](app/lasso_fill.md)). |
| `fill_enclose` | `bool` | Lasso fill fills only what its lines shut in. |
| `fill_behind` | `bool` | Lasso fill goes under the paint already there. |
| `tint_outside` | `bool` | Dim everything outside the selection. |
| `bg_color` | `[f32; 3]` | Canvas backdrop colour (RGB 0..1). |
| `bg_opacity` | `f32` | Backdrop opacity. |
| `harmony` | `Harmony` | The colour-harmony scheme only; the set is rebuilt around the brush colour each launch. See [color.rs](color.md). |
| `last_camera_size` | `Option<(u32, u32)>` | The camera frame size last set. The size itself lasts only for a session; this brings it back with one click next time. |
| `hidden_panels` | `Vec<PanelId>` | Panels closed one at a time from the Panels menu or their title bar. |
| `panel_layout_gen` | `u32` | Bumped by Reset panel layout; see `AppState::panel_layout_gen`. |
| `panel_padding` | `u8` | Gap between panels and window edges; see `AppState::panel_padding`. |
| `color_scheme` | `Scheme` | The UI colour scheme (see [ui/theme.rs](ui/theme.md)). |
| `custom_colors` | `BaseColors` | The five base colours of a Custom scheme, kept while another scheme is picked. |

### `struct FadeOthers`
[src/app.rs:544](../../src/app.rs#L544)

Opacity multipliers for the layers around the active one while **"fade other layers"** is on.
View-only: layer opacity, export and the `.anim` file never see it. Above and below differ
because a layer over the drawing covers it, while one under it is usually what the drawing is
being matched against. Serialisable (part of `UiPrefs`); used by [ui/shell.rs](ui/shell.md)
and [input/shortcuts.rs](input/shortcuts.md).

| Field | Type | Meaning |
|---|---|---|
| `above` | `f32` | Opacity factor for layers above the active one. |
| `below` | `f32` | Opacity factor for layers below it. |

### `const TOOL_COUNT`
[src/app.rs:560](../../src/app.rs#L560)

```rust
const TOOL_COUNT: usize = 9;
```

`9` — the length of the per-tool brush list the prefs file keeps: the pencil and the ink from
before the [pen slots](app/pen_slots.md), then the seven other tools in `ActiveTool::idx` order.
The app itself keeps the pens in `pens` and the others in `tool_brushes`; this layout is only for
reading and writing the prefs, so an older build still understands them.

### `const LEGACY_PENS`
[src/app.rs:562](../../src/app.rs#L562)

```rust
const LEGACY_PENS: usize = 2;
```

The pens at the head of that array, now the first two pen slots.

### `const OTHER_TOOLS`
[src/app.rs:565](../../src/app.rs#L565)

```rust
const OTHER_TOOLS: usize = ActiveTool::OTHERS.len();
```

The tools that are not pens, each with its own brush in `AppState::tool_brushes`.

### `const _`
[src/app.rs:566](../../src/app.rs#L566)

```rust
const _: () = assert!(LEGACY_PENS + OTHER_TOOLS == TOOL_COUNT);
```

A check run by the compiler, not at run time: the build fails if the two pens plus the other
tools ever stop adding up to the prefs' list length. `const _` is an unnamed constant, a common
way to write such a compile-time assertion.

### `const LEGACY_SMOOTHING`
[src/app.rs:612](../../src/app.rs#L612)

```rust
const LEGACY_SMOOTHING: usize = 3;
```

The freehand tools from before the pen slots, in `pen_smoothing` order: the pencil, the ink and the eraser.

### `const ERASER_SMOOTHING`
[src/app.rs:614](../../src/app.rs#L614)

```rust
const ERASER_SMOOTHING: usize = 2;
```

The eraser's place in `pen_smoothing`.

### `enum Dirty`
[src/app.rs:684](../../src/app.rs#L684)

What a cell's GPU texture is missing. A rectangle rather than a yes/no flag: on a layer widened
to 3× a 4K frame each cell is ~100 MB, and re-uploading all of it after a stroke that touched a
few hundred pixels was the hitch at pen-up. Stored per cell in `AppState::cell_dirty`.

| Variant | Meaning |
|---|---|
| `Clean` | Up to date. |
| `Rect(DirtyRect)` | Only this region changed since the last upload. |
| `Full` | Upload the whole cell. |

### `const TEXTURE_BUDGET`
[src/app.rs:706](../../src/app.rs#L706)

```rust
const TEXTURE_BUDGET: usize = 1536 * 1024 * 1024;
```

`1536 * 1024 * 1024` bytes (1.5 GiB): GPU memory cell and ghost textures may use together
before the least recently shown are dropped. Without a cap, long playback of a big layer kept
every cell on the GPU until the driver started paging. Used by `enforce_texture_budget`.

### `const GHOST_MAX_SIDE`
[src/app.rs:711](../../src/app.rs#L711)

```rust
const GHOST_MAX_SIDE: u32 = 2048;
```

`2048` — longest side of an onion ghost's texture. A ghost is a faint silhouette, so a smaller
texture doesn't show, and on a 3×-wide 4K layer it is ~30× less to build and upload.

### `const PACK_BUDGET`
[src/app.rs:714](../../src/app.rs#L714)

```rust
const PACK_BUDGET: Duration = Duration::from_millis(4);
```

`Duration::from_millis(4)` — time per frame `pack_idle_cells` may spend shrinking idle cells
back to just their drawing.

### `const GHOST_REST`
[src/app.rs:717](../../src/app.rs#L717)

```rust
const GHOST_REST: f64 = 0.12;
```

`0.12` seconds the playhead must rest on a frame before a ghost that isn't built yet gets
built, so scrubbing past many frames doesn't build one for each.

### `const WARM_REST`
[src/app.rs:721](../../src/app.rs#L721)

```rust
const WARM_REST: f64 = GHOST_REST;
```

Same as `GHOST_REST`: seconds a drawing must stay the one the pen would land on before its
stroke snapshot is taken ahead of time (see `warm_pre`).

### `struct TexEntry`
[src/app.rs:725](../../src/app.rs#L725)

One texture currently on the GPU, as the texture budget sees it.

| Field | Type | Meaning |
|---|---|---|
| `kind` | `TexKind` | Which texture map it lives in. |
| `id` | `CellId` | The cell it belongs to. |
| `used` | `u64` | The last sync (frame counter) it was on screen. |
| `bytes` | `usize` | Its size in GPU memory. |

### `enum TexKind`
[src/app.rs:737](../../src/app.rs#L737)

Which texture map a `TexEntry` is in. (The doc comment above it in the source starts with a
stray line that belongs to the `evictions` function.)

| Variant | Meaning |
|---|---|
| `Cell` | A plain cell texture (`cell_textures`). |
| `Ghost` | An onion ghost (`ghost_textures`). |
| `Clip(groups::BaseKey)` | A clipped drawing; `id` is the clipped cell, the key is what it is cut by. |
| `Group(groups::GroupKey)` | A group drawn as one image, or a group's mask; `id` is unused. |

### `struct ClipPair`
[src/app.rs:748](../../src/app.rs#L748)

A **clipped layer** (a layer that only shows where the layer under it — its *base* — has paint)
as it shows on the current frame. Built by `clip_pair`; also used in
[app/groups.rs](app/groups.md).

| Field | Type | Meaning |
|---|---|---|
| `cell` | `CellId` | The clipped layer's drawing. |
| `base` | `groups::BaseKey` | What it is cut by: a base cell or a group. |
| `base_layer` | `usize` | The base's layer: a drawing layer, or a group's header. |
| `at` | `crate::doc::clip::Placement` | Where the two drawings sit relative to each other (see [doc/clip.rs](doc/clip.md)). |

### `struct CellTex`
[src/app.rs:760](../../src/app.rs#L760)

A cell's GPU texture. It holds only `rect` of the cell — the drawing plus a thin transparent
rim — not the whole canvas: drawings rarely fill their cell, and a loop of full 2160×2880
cells outgrew `TEXTURE_BUDGET`, so playback re-uploaded every frame and the canvas stuttered.
Stored in `AppState::cell_textures`; read by [ui/shell.rs](ui/shell.md).

| Field | Type | Meaning |
|---|---|---|
| `tex` | `TextureHandle` | The egui GPU texture. Dropping it frees the GPU memory. |
| `rect` | `DirtyRect` | Where the texture sits in the cell; every inked pixel is inside it. Empty for a blank cell, which then draws nothing. |
| `cell` | `[u32; 2]` | The cell's size when the texture was made; a resized cell starts over. (private) |

### `const TEX_RIM`
[src/app.rs:772](../../src/app.rs#L772)

```rust
const TEX_RIM: u32 = 2;
```

`2` — transparent pixels kept around a drawing in its texture, so the GPU's smooth (linear)
filtering at the drawing's edge blends into clear exactly as it would over the whole cell.

### `enum PenTex`
[src/app.rs:833](../../src/app.rs#L833)

What the drawing the pen would paint needs on the GPU so a stroke never re-makes its texture partway: a texture holding the whole cell. A stroke that reached past a cropped one re-sent the whole drawing in one frame — 32 MB at 2160×3840, ~20 ms, the pen running on ahead of the ink. Readied while the pen rests instead. See `AppState::sync_pen_tex`.

| Variant | Meaning |
|---|---|
| `Cell(CellId)` | This drawing's own texture, grown to its whole cell. |
| `Blank([u32; 2])` | A clear texture of this size, for the new drawing a press would make. |

### `enum TexPlan`
[src/app.rs:842](../../src/app.rs#L842)

How a cropped texture catches up with its cell; the answer `plan_upload` gives.

| Variant | Meaning |
|---|---|
| `Keep` | Up to date; do nothing. |
| `Patch(DirtyRect)` | Re-send this rect of the cell; it lies inside the existing texture. |
| `Rebuild(DirtyRect)` | Make the texture anew, holding this rect of the cell. |

### `const STROKE_ROOM`
[src/app.rs:852](../../src/app.rs#L852)

```rust
const STROKE_ROOM: u32 = 512;
```

`512` — least room (pixels) a live stroke's texture gets on each side of what it must hold.

### `struct ClipTex`
[src/app.rs:974](../../src/app.rs#L974)

A clipped drawing's texture as the canvas shows it: already cut to its base. Stored in
`AppState::clip_textures`, keyed by (clipped cell, base).

| Field | Type | Meaning |
|---|---|---|
| `tex` | `TextureHandle` | The GPU texture. |
| `rect` | `DirtyRect` | Where it sits in the clipped cell, like `CellTex::rect`. Kept around the clipped *drawing*, not the cut: a base edit can uncover more of the drawing but never anything outside it. |
| `cell` | `[u32; 2]` | The clipped cell's size when made. |
| `at` | `crate::doc::clip::Placement` | Where the two drawings sat when made; a change means a fresh bake. |
| `aligned` | `bool` | A base pixel lies under the clipped pixel with the same index, so a base change maps onto the same rect here. |
| `pending` | `Dirty` | What changed in either drawing since. |

### `enum PanelId`
[src/app.rs:1020](../../src/app.rs#L1020)

The floating tool panels. Serialised in snake_case (`"tools"`, `"xsheet"`…) as part of
`UiPrefs::hidden_panels`. Derives `Hash` so it can be a map key. Used by
[ui/shell.rs](ui/shell.md) and [ui/sweep_tests.rs](ui/sweep_tests.md).

| Variant | Meaning |
|---|---|
| `Tools` | Tool buttons. |
| `Brush` | Brush settings, colour wheel and harmonies. |
| `Layers` | Layer stack. |
| `Onion` | Onion-skin settings. |
| `Xsheet` | The x-sheet (exposure sheet: which drawing shows on which frame). |
| `Timeline` | Timeline tracks along the bottom. |
| `Camera` | Camera settings. |
| `Color` | Retired: its wheel moved into the Brush panel. Kept only so old prefs that hid it still load; never drawn or listed. |

### `const PanelId::ALL`
[src/app.rs:1037](../../src/app.rs#L1037)

```rust
pub const ALL: [PanelId; 7] = [
    PanelId::Tools,
    PanelId::Brush,
    PanelId::Layers,
    PanelId::Onion,
    PanelId::Camera,
// …
```

Every live panel in Panels-menu order: left column (Tools, Brush, Layers, Onion), right column
(Camera, Xsheet), then Timeline. Leaves out `Color`. Used by [ui/shell.rs](ui/shell.md) (Panels
menu, panel docking).

### `enum ImportSource`
[src/app.rs:1062](../../src/app.rs#L1062)

Where a start/end frame-range import is reading from.

| Variant | Meaning |
|---|---|
| `Video { path: PathBuf, fps: f64 }` | A video file; frames are extracted with ffmpeg on confirm. `fps` drives the timestamp seek used for the dialog's previews. |
| `Frames { frames: Vec<Canvas>, format: &'static str }` | An animated image (GIF, WebP) already decoded to full-size canvases; `format` names it. |

### `struct ImportRangeState`
[src/app.rs:1072](../../src/app.rs#L1072)

Backing state for the frame-range import dialog (video / GIF / WebP). Lives in
`AppState::import_range`.

| Field | Type | Meaning |
|---|---|---|
| `source` | `ImportSource` | What is being imported. |
| `total` | `usize` | Number of source frames (the slider's upper bound is `total - 1`). |
| `start` | `usize` | First frame to import. |
| `end` | `usize` | Last frame to import (inclusive). |

### `type PreviewFrame`
[src/app.rs:1081](../../src/app.rs#L1081)

```rust
pub type PreviewFrame = (u32, u32, Vec<u8>);
```

A decoded preview frame for the range dialog: width, height and RGBA8 bytes. A `type` alias is
just a short name for a longer type (like a TypeScript `type` alias).

### `enum BgJob`
[src/app.rs:1085](../../src/app.rs#L1085)

A long-running import/export step on a worker thread. Each variant holds the `Receiver` end of a
channel the worker will send its result on; `poll_bg_jobs` checks it every frame so the UI never
blocks on ffmpeg or decoding. Only one runs at a time, and while one runs a modal "busy" overlay
shows (`AppState::bg_label`).

| Variant | Meaning |
|---|---|
| `FramesDecode { rx: Receiver<Result<Vec<Canvas>>>, format: &'static str, }` | Decoding an animated image (GIF, WebP) before the range dialog opens. |
| `VideoProbe { rx: Receiver<Result<(usize, f64)>>, path: PathBuf, }` | Probing a video's frame count and fps before the range dialog opens. |
| `Export(Receiver<Result<String>>)` | Writing an export; `Ok` carries the file name for the success toast. |
| `VideoExtract { rx: Receiver<Result<Vec<Canvas>>>, name: String, }` | Extracting the chosen video range, to drop onto a new layer named `name`. |
| `KraImport(Receiver<Result<KraDoc>>)` | Reading a `.kra` for File ▸ Import .kra. |

### `struct SaveJob`
[src/app.rs:1113](../../src/app.rs#L1113)

A project save running on a worker thread. Deliberately *not* a `BgJob`: those raise the modal
busy overlay, and a save that blocks the window is exactly what this avoids — you can keep
drawing while it writes.

| Field | Type | Meaning |
|---|---|---|
| `rx` | `Receiver<Result<()>>` | Where the worker reports success or failure. (private) |
| `path` | `PathBuf` | Committed to `project_path` only once the write actually succeeds. (private) |
| `name` | `String` | File name, for the success toast. (private) |

### `struct KritaSend`
[src/app.rs:1122](../../src/app.rs#L1122)

The worker writing the `.kra` for **Edit in Krita**.

| Field | Type | Meaning |
|---|---|---|
| `rx` | `Receiver<Result<krita_link::Sent>>` | The worker's result (see [krita_link.rs](krita_link.md)). |
| `launch` | `Option<PathBuf>` | The Krita executable to start once the file is on disk; `None` when the Krita launched earlier is still open. |
| `kept_out` | `Vec<String>` | Names of layers left out of the send (reference layers, `-x` layers), listed in the toast. |

### `struct LayerRename`
[src/app.rs:1133](../../src/app.rs#L1133)

An inline rename in progress (used for layers, and reused for New-project presets).

| Field | Type | Meaning |
|---|---|---|
| `index` | `usize` | Which layer (or preset). |
| `buf` | `String` | The text being typed. |
| `focused` | `bool` | Whether the text box has been given keyboard focus yet (done on the first frame only). |

### `struct AppState`
[src/app.rs:1139](../../src/app.rs#L1139)

**The whole running app in one value.** `AppState` holds the open project, every tool
setting, the stroke being drawn, the selection, the camera, the view, the undo history, every
GPU texture, open dialogs, background jobs and all saved preferences. There is exactly one
`AppState` for the life of the program: eframe creates it with [`AppState::new`](#appstatenew)
and then calls its `update` method every frame.

If you know web apps: think of it as one big store object (a Redux store, or the state of a
top-level React component) that the UI reads and mutates directly. Because Rust checks
borrowing, UI functions receive it as `state: &mut AppState` — "a writable borrow of the one
app state" — rather than reaching for globals.

Most fields are documented in the source with `///` comments, shown in the table. Fields
marked *session-only* are deliberately forgotten when the app quits; the ones that persist are
copied into [`UiPrefs`](#struct-uiprefs) by `save`.

| Field | Type | Meaning |
|---|---|---|
| `project` | `Project` | The open animation document — layers, drawings (cells), frames, camera. See [doc/project.rs](doc/project.md). |
| `project_path` | `Option<PathBuf>` | File this project was last saved to or loaded from. `None` = never saved, so Save has to ask for a destination. |
| `save_toast` | `Option<(String, Instant)>` | Transient confirmation — a save, an export, a frame copied — as the line it shows, and the instant it expires. A silent overwrite is otherwise indistinguishable from a save that never ran. |
| `save_error` | `Option<String>` | Failed write, shown as a modal until dismissed. Never merely logged — a silent save that silently failed is how work gets lost. |
| `save_job` | `Option<SaveJob>` | Write in flight on a worker thread. Serialising and compressing a few hundred MB would otherwise stall the window. |
| `krita` | `Option<KritaLink>` | Two-way link with Krita (File ▸ Edit in Krita). Session-only: it ends with the app, a New or an Open. |
| `krita_send` | `Option<KritaSend>` | A send in flight, and the Krita to launch once the file is written. |
| `krita_toast` | `Option<(String, bool, Instant)>` | Latest Krita-link message: text, whether it is a warning, expiry. |
| `krita_path` | `Option<PathBuf>` | Where Krita is installed. Found or picked once, then remembered. |
| `cell_textures` | `HashMap<CellId, CellTex>` | GPU texture per CellId, lazily created. See `CellTex`. |
| `cell_dirty` | `HashMap<CellId, Dirty>` | What each cell's texture is missing: nothing, one rect, or the lot. |
| `sync_frame` | `u64` | Sync counter, and the last sync each texture was on screen — what the texture budget evicts by. |
| `cell_tex_used` | `HashMap<CellId, u64>` | For each drawing's texture, the last sync (`sync_frame` value) it was shown on; the texture budget evicts the least recently shown. |
| `clip_textures` | `HashMap<(CellId, groups::BaseKey), ClipTex>` | Clipped layers' drawings as the canvas shows them, keyed by (clipped cell, base cell), drawn in place of the plain cell texture. See `sync_clip_textures`. |
| `clip_tex_used` | `HashMap<(CellId, groups::BaseKey), u64>` | Same "last shown" counter for clipped-layer textures. |
| `group_images` | `HashMap<groups::GroupKey, groups::GroupImage>` | Groups drawn as one image, and group masks. See `app::groups`. |
| `group_used` | `HashMap<groups::GroupKey, u64>` | Same "last shown" counter for group images. |
| `ghost_tex_used` | `HashMap<CellId, u64>` | Same "last shown" counter for onion-skin ghost textures. |
| `playhead_rest` | `(usize, f64)` | Frame the playhead sat on at the last sync, and since when (egui time). A ghost not built yet waits for the playhead to rest, so a scrub doesn't build one for every frame it passes. |
| `ghost_textures` | `HashMap<CellId, ([u8; 3], TextureHandle)>` | Colorized onion ghosts: one silhouette texture per ghosted cell, with the tint it was built from so a tint change in the panel rebuilds it. Separate from `cell_textures` because the ghost replaces the artwork's RGB wholesale — a vertex-color tint only multiplies, which leaves black line art black. |
| `ghost_stale` | `HashSet<CellId>` | Ghosted cells whose pixels changed: the silhouette is re-uploaded into the existing handle rather than dropped, so an edit never frees a texture the canvas may already have queued a mesh against. |
| `retired_textures` | `Vec<TextureHandle>` | Texture handles that are no longer wanted, parked until the next sync. Dropping a `TextureHandle` frees the GPU texture, and a free that lands after the canvas has queued a mesh referencing it makes wgpu submit against a destroyed texture — which File → New and File → Open would otherwise do, since they run from menu handling *after* the canvas has painted. Nothing painted last frame is released before this frame's own paint, so a one-frame delay is enough. |
| `canvas_gpu` | `Option<CanvasGpu>` | Draws the canvas textures, mixing texels in gamma space — see `ui::canvas_gpu`. `None` without a GPU, as in tests: the canvas then draws them as plain egui meshes. |
| `fresh_full` | `HashSet<egui::TextureId>` | Textures re-filled whole this sync. egui only swaps the new image in after the frame is built, so `canvas_gpu` would bind the old one — stale, and placed by the new rect. These draw as meshes this frame. |
| `tool` | `ActiveTool` | The active tool (Pencil, Ink, Eraser, Fill, Shape, Lasso, …). |
| `brush` | `BrushSettings` | The active tool's brush settings (size, colour, opacity, pressure curves, Krita preset…). Written back to `tool_brushes` on a tool switch. |
| `brush_library` | `BrushLibrary` | Every brush the picker offers, Krita's included. |
| `brush_search` | `String` | The picker's search text. Session-only. |
| `brush_thumbs` | `HashMap<String, TextureHandle>` | The picker's thumbnails, uploaded as they arrive from the library. |
| `krita_import` | `Option<crate::brush_library::import::KritaImport>` | The Import Krita brushes window, while open. |
| `palette` | `Vec<[u8; 3]>` | Pinned swatches, most recently added last. Capped at `AppState::MAX_SWATCHES`. |
| `brush_sizes` | `Vec<brush_sizes::SizeMemory>` | The sizes each tool's brush was drawn at, and the ones pinned, by `ActiveTool::idx`. See `brush_sizes`. |
| `size_swap` | `Option<(ActiveTool, f32, f32)>` | The last size swap: the brush slot, the size swapped to, and the one swapped from, which the next swap goes back to. |
| `harmony` | `HarmonySet` | The Color panel's harmony set. Its scheme is a workspace preference; the base follows the brush colour. |
| `project_presets` | `Vec<ProjectPreset>` | Saved New project presets. Like `palette`, a new project keeps them. |
| `cell_clip` | `Option<Canvas>` | Drawing clipboard: one cell's pixels, cut or copied from a slot. Held as a `Canvas` rather than a `CellId` so it survives the undo of the cut that produced it. |
| `track_sel` | `BTreeSet<(usize, usize)>` | Drawings selected in the timeline tracks, as (layer, key frame). Session-only, and dropped by any structural edit the tracks did not make themselves — keys move, and a stale entry would select whatever slid into its place. |
| `track_anchor` | `Option<(usize, usize)>` | Where the last plain click in the tracks landed, as (layer, frame): the corner a Shift-click selects a rectangle from. |
| `track_drag` | `Option<crate::ui::tracks::TrackDrag>` | In-flight drag in the tracks, or `None`. |
| `track_scroll` | `egui::Vec2` | Track scroll offset in points: x across frames, y down the layers. |
| `track_frame_w` | `f32` | Width of one frame column in the tracks, in points. The wheel zooms it (Ctrl by default; see `WheelAction::Zoom`). |
| `track_timing_n` | `usize` | The N in the tracks' "On N" timing menu. |
| `gc_key` | `Option<(u64, usize)>` | What the cell collector last ran against: the history's revision and the pool's length. `None` makes it run. See `collect_cells`. |
| `blank_cache` | `HashMap<CellId, bool>` | Whether a cell has no painted pixel, per cell id. A scan is a full pass over the buffer, so it is cached, and dropped only when pixels change (`mark_dirty`) — a structural edit moves ids around but never pixels. |
| `blank_scans_left` | `u32` | Blank scans still allowed this frame; see `cell_is_blank`. |
| `sel_mask` | `Option<SelectionMask>` | The selection, in document space: what every paint edit is clipped to. Belongs to no drawing, so it survives frame and layer changes. `None` when nothing is selected. Session-only; see `app::select`. |
| `sel_ver` | `u64` | Bumped whenever `sel_mask` changes; keys everything derived from it. |
| `sel_outline` | `Vec<Vec<(f32, f32)>>` | Marching-ants loops of `sel_mask`, in document space. |
| `clip_cache` | `Option<(select::ClipKey, Arc<Mask>)>` | The active cell's view of `sel_mask`, and what it was built for. |
| `sel_gesture` | `Option<SelGesture>` | Selection gesture in progress (lasso, box, ellipse, polygon). |
| `sel_op` | `SelOp` | Sticky selection mode; a held modifier overrides it for one drag. |
| `sel_all_frames` | `bool` | Land a moved selection on every drawing of the active layer, not just the one it was lifted from. Session-only: it starts off every launch, so it can't quietly spread an edit next week. |
| `sel_shape` | `SelShape` | What a selection drag draws. |
| `wand` | `WandOptions` | How the magic select picks. |
| `wand_drag` | `Option<select::WandDrag>` | A magic select press still under the pen. |
| `wand_repick` | `Option<(f32, f32)>` | Where a plain press on the selection with the magic select landed: if it lets go without moving the pixels, it picks there instead. |
| `fill_mode` | `FillMode` | Whether a Fill press floods or draws a lasso. |
| `fill_enclose` | `bool` | A lasso fill fills only what its lines shut in. |
| `fill_behind` | `bool` | A lasso fill goes under the paint already there. |
| `fill_lasso` | `Option<lasso_fill::FillLasso>` | The lasso fill being drawn. |
| `sel_amount` | `u32` | Pixels a Grow / Shrink / Feather takes. |
| `tint_outside` | `bool` | Dim everything outside the selection. |
| `sel_tint` | `Option<SelTint>` | The tint texture over what is not selected. |
| `selection` | `Option<Selection>` | Floating pixels lifted through the selection, if any. Bound to one cell: changing frame or layer lands them first. |
| `sel_followers` | `Vec<select::Follower>` | The same selection lifted off the other picked layers, moving with `selection`. Empty with one layer picked. |
| `float_layer` | `usize` | The float's own layer, which leads: the active one unless that has nothing to lift. |
| `float_cur` | `usize` | The active layer when the float was made. Picking another lands it. |
| `liquify` | `Option<liquify::LiquifySession>` | A Liquify warp under way, until it lands or is cancelled. |
| `float_pre` | `Vec<u8>` | The float's cell as it was before the lift, so landing records lift and stamp as one step and an undo while floating can put it back. |
| `float_pre_live` | `bool` | Whether `float_pre` currently holds a valid "before" snapshot (a flag beside a reused buffer instead of an `Option<Vec>`). |
| `float_lift_rect` | `Option<DirtyRect>` | Where the lift erased. |
| `sel_drag` | `Option<SelDrag>` | In-flight drag on the selection's transform box, or `None`. |
| `pixel_clip` | `Vec<select::ClipPart>` | Selection clipboard: mask plus lifted pixels, per layer copied from, the lead's first. Empty before any copy. |
| `sel_tex_stale` | `bool` | Set when the floating pixels changed and their texture must be rebuilt. Posing does *not* set it — the pose moves the quad, and the GPU samples the same texture through it. |
| `selection_tex` | `Option<TextureHandle>` | GPU texture for the floating pixels. |
| `tool_brushes` | `[BrushSettings; OTHER_TOOLS]` | Per-tool brush settings preserved across tool switches. |
| `pens` | `Vec<PenSlot>` | The pen slots, in the order of the row: one to `pen_slots::MAX_PENS`. See `pen_slots`. |
| `last_pen` | `ActiveTool` | Pencil or Ink, whichever was active last: the pen whose size Shape draws at. Session-only; the app comes back on the pencil anyway. |
| `prev_pen` | `Option<ActiveTool>` | The pen slot in hand before `last_pen`, for the swap key. Session-only. |
| `pen_menu_search` | `String` | The search box of a pen slot's right-click brush picker, apart from the Brush panel's. |
| `pen_remove_armed` | `Option<usize>` | The pen slot whose menu has had Remove clicked once: the next click removes it. Cleared whenever a slot's menu opens. |
| `wheel_pinned` | `bool` | The colour wheel is pinned: every tool paints in one colour, and the wheel's set leads the swatch pie. Session-only. |
| `stroke` | `Option<StrokeBuilder>` | The freehand stroke being drawn right now, if any: collects pen samples and paints them. See [tools/stroke.rs](tools/stroke.md). |
| `stroke_target` | `Option<CellId>` | CellId being painted into during the current stroke. |
| `stroke_clock` | `Option<Instant>` | When the stroke began: the clock a stabilized stroke paints by. |
| `shape_drag` | `Option<ShapeDrag>` | In-progress Shape-tool drag (preview only until pointer-up). |
| `fill_drag` | `Option<FillDrag>` | In-progress Fill press, adjustable by dragging until pointer-up. |
| `shift_held` | `bool` | Shift state, sampled once a frame. Pointer handlers run off tablet packets rather than egui events, so they have no `InputState` of their own to ask; a selection scale reads this to keep its proportions. |
| `shift_stale` | `bool` | Shift was already down when the drag in hand began, and hasn't been let go since: it chose the drag (a pan, a selection mode), so it doesn't lock it too. See `Self::axis_lock_on`. |
| `stroke_axis` | `Option<StrokeAxis>` | Where a stroke held to an axis by Shift is held, or `None` while it isn't. See `Self::stroke_point_doc`. |
| `stroke_last_doc` | `Option<[f32; 2]>` | The stroke's last point, in document space: where a Shift lock taken partway through holds it from. |
| `alt_held` | `bool` | Alt, likewise: a selection scale about the box centre. |
| `move_key` | `MidKey` | The keys that slide a shape or box along mid-drag, and grow it from its middle. See `Self::move_on`, `Self::centre_on`. |
| `centre_key` | `MidKey` | The "grow from the centre" hold key (Alt by default), tracked the same way as `move_key`. |
| `shape_pointer_doc` | `Option<[f32; 2]>` | A Shape drag's pointer, in document space before any snap: what the move key slides the shape by. |
| `perspective` | `PerspectiveConfig` | Perspective grids and their display/snap switches. |
| `grid_drag` | `Option<GridDrag>` | In-flight drag on a perspective grid, or `None`. |
| `grid_drag_before` | `Option<perspective::PerspectiveGrid>` | The dragged grid as it was at the press, for the drag's undo step. |
| `grid_sync_last` | `Option<usize>` | Last frame keyed grids were brought to. |
| `grid_files` | `Vec<grids::GridFile>` | Every project file's grids but the open one's live copy. |
| `grid_seed` | `Vec<perspective::PerspectiveGrid>` | The grids from before grids were kept per file, for files the app has none kept for. |
| `snap_lock` | `Option<SnapLock>` | Perspective snap for the stroke in progress, or `None` when the stroke isn't snapping. |
| `view` | `View` | Canvas view transform (zoom / pan / rotate). |
| `view_scale` | `f32` | Screen pixels per *document* pixel, republished by the canvas each frame (`View::zoom` alone can't say this — the fit-to-window base scale depends on the canvas rect, which only the UI knows).  The stroke pipeline runs entirely in cell pixels while the pen and the OS pointer work in screen pixels; this is the conversion factor that lets input conditioning be specified in what the user actually sees. |
| `lock_brush_to_view` | `bool` | Keep the brush a fixed size *on screen* instead of in document pixels, so the same hand gesture lays down the same-looking stroke at any zoom. Off by default: line weight is normally absolute in document pixels, which is what makes exports predictable. |
| `nav_drag` | `Option<NavKind>` | Active non-drawing canvas gesture for the current drag, if any. |
| `hold_latch` | `Option<shortcuts::KeyCombo>` | The held combo that started this drag's gesture, while it lasts. Letting go of it ends the gesture there, as Space-pan does in Krita. |
| `drag_inert` | `bool` | The rest of this pen-down does nothing: its held gesture was let go, and drawing from wherever the pen happens to be would be a surprise. |
| `playback` | `Playback` | Play/pause state and the playback clock. See [timeline/playback.rs](timeline/playback.md). |
| `onion` | `OnionConfig` | Onion-skin settings (how many ghosts before/after, colours, opacity). A workspace preference. |
| `fade_others` | `bool` | "Fade other layers": every layer but the active one drops to the `fade` amounts on the canvas. Session-only, so a launch never opens looking washed out; see `AppState::layer_view_alpha`. |
| `fade` | `FadeOthers` | How far other layers fade when "Fade other layers" is on (separately above and below the active one). |
| `auto_key_transform` | `bool` | Editing the active layer's transform writes a key on the current frame instead of moving the whole layer. |
| `auto_key_draw` | `bool` | Drawing on a held frame breaks the hold into its own drawing first, instead of editing the cell shared with the rest of the hold. |
| `frame_step` | `usize` | Frames moved per FramePrev / FrameNext press, and per ◀ / ▶ click. Read through `AppState::frame_step_delta`, which clamps it to >= 1 — a zero would turn frame navigation into a no-op. |
| `invert_timeline_scroll` | `bool` | Invert the mouse-wheel scrub direction. Off: wheel down advances. |
| `lock_screen_axes` | `bool` | Shift partway through a drag holds it to the screen's axes, not the canvas's. See `Self::lock_basis_doc`. |
| `middle_drag` | `ButtonDrag` | What a middle-button drag on the canvas does. |
| `right_drag` | `ButtonDrag` | What a right-button drag on the canvas does. |
| `button_drag` | `Option<ButtonDragState>` | A middle- or right-button drag under way on the canvas. |
| `eraser_smoothing` | `SmoothingOptions` | The eraser's line smoothing, latched into every new stroke; each pen slot keeps its own. See `tools::Smoothing` and `Self::smoothing`. |
| `eraser_before_stabilizer` | `Smoothing` | The smoothing the eraser had before the Stabilizer key turned it on, for the key to turn it back to. Session-only. |
| `loop_timeline` | `bool` | Whether the timeline wraps. Gates playback, wheel scrub and the frame step actions alike, so one toggle means one behaviour everywhere. |
| `pen_outlier_logged` | `bool` | Whether this stroke has already complained about a stray packet. One line per stroke is a report; one per packet is a flood. |
| `pen_batches_rejected` | `u32` | Frames whose packet batch was thrown away because the newest packet disagreed with the cursor, and packets dropped as outliers within an accepted batch. Counted for the whole session and shown in the tablet readout: neither is visible while drawing, which is when they happen. |
| `pen_packets_dropped` | `u32` | Count of single stray tablet packets dropped as outliers this session, shown in the tablet readout. |
| `stroke_from_pen` | `bool` | Where the current stroke's positions come from, decided at pen-down. A pen stroke takes tablet packets only: the cursor is the same position rounded to a whole pixel, and slipping it in on a frame with no packet puts a half-pixel kink into the line — several canvas pixels once zoomed out. Cleared for good if the pen mapping stops agreeing with the cursor partway through. |
| `stroke_pen_samples` | `u32` | Samples the last stroke took from each source, for the tablet readout. A pen stroke should show none from the cursor. |
| `stroke_mouse_samples` | `u32` | How many samples the last stroke took from the mouse cursor (see `stroke_pen_samples`). |
| `wheel_scrub_accum` | `f32` | Leftover trackpad scroll (in points) not yet worth a whole wheel notch. Session-only: a wheel gesture never spans a run. Mice report whole lines and bypass this entirely — see `ui::shell::wheel_map`. |
| `canvas_rect` | `egui::Rect` | The canvas's screen rect as of the last frame, for what needs it before the canvas is laid out again: zooming the view on the wheel. |
| `bg_opacity` | `f32` | Opacity of the window's background (0 = see-through to the desktop, for tracing). |
| `bg_color` | `[f32; 3]` | Background clear color (RGB, 0..1). |
| `pen` | `PenInput` | The pen-tablet connection: polls the driver and keeps the packet queue. See [input/tablet.rs](input/tablet.md). |
| `pen_record` | `Option<crate::input::pen_record::PenRecord>` | A pen recording under way (Tablet readout), and where the last one went — or why it couldn't be written. |
| `pen_record_saved` | `Option<Result<std::path::PathBuf, String>>` | Where the last pen recording was written, or the error text if it could not be. |
| `shortcuts_file_note` | `Option<Result<String, String>>` | What the last open of, or reload from, shortcuts.toml came to, for the Settings window: the file opened, edits loaded, or why not. |
| `history` | `History` | Undo/redo history. See [undo.rs](undo.md). |
| `stroke_pre_pixels` | `Vec<u8>` | Full-pixel snapshot of the cell before the in-flight stroke started. Used to extract the `before` slice for an undo command when the stroke finishes. |
| `stroke_pre_live` | `bool` | Whether `stroke_pre_pixels` holds the current edit's snapshot. A flag beside a kept buffer rather than an `Option<Vec>`: the buffer is reused, because allocating a fresh 100 MB one at every pen-down on a big layer was a hitch of its own. |
| `pre_mirror` | `Option<(CellId, u64)>` | The cell, and its `Canvas::rev`, that `stroke_pre_pixels` still holds exactly: set when an edit lands, so the next edit of the same cell needs nothing copied at pen-down. On an 8K cell the copy was 10 ms, and with the unpack it forced, 40–60 ms. |
| `pre_cell` | `Option<CellId>` | The cell `stroke_pre_pixels` was taken from for the edit in hand. |
| `pre_inked` | `Option<DirtyRect>` | Where `stroke_pre_pixels` may hold anything but zeros; `None` when it is all zeros. Snapshotting another cell clears just this. |
| `pre_w` | `u32` | Pixels per row of what `stroke_pre_pixels` holds. |
| `warm_wait` | `Option<(Option<CellId>, f64)>` | The drawing the pen would land on (`None`: a new blank one) and since when, while waiting to take its snapshot ahead of the pen. See `warm_pre`. |
| `pen_tex_want` | `Option<PenTex>` | What `warm_pre` last asked the GPU to have ready for the pen. See `sync_pen_tex`. |
| `pen_tex_whole` | `Option<CellId>` | The drawing whose texture holds its whole cell for the pen, to be cropped again once the pen has moved on. |
| `spare_tex` | `Option<(TextureHandle, [u32; 2])>` | A clear texture the size of a whole cell, ready to become the new drawing a press makes (auto-key, an empty slot). See `adopt_spare_tex`. |
| `stroke_base` | `Option<DirtyRect>` | Under a stroke, where the target cell held anything before the stroke began (`None`: blank). With the stroke's own `Canvas::dirty` it bounds the drawing, so the texture can grow without a scan. |
| `stroke_ws` | `StrokeWorkspace` | Reusable per-stroke coverage workspace for the ribbon rasterizer. |
| `preview_upload_rect` | `Option<DirtyRect>` | Canvas region updated by stroke flushes since the last texture sync; uploaded to the GPU via a partial texture update each frame. |
| `shortcuts` | `ShortcutMap` | The key bindings in use, loaded from `shortcuts.toml`. See [input/shortcuts.rs](input/shortcuts.md). |
| `rebinding` | `Option<Action>` | While `Some(action)`, the next key press from the user becomes the new binding for that action. |
| `hold_capture` | `shortcuts::HoldCapture` | Capture state while `rebinding` is a hold binding: bare modifiers are only known once they come back up. |
| `shortcut_clashes_only` | `bool` | Settings lists only the bindings that clash. Session-only. |
| `layer_rename` | `Option<LayerRename>` | In-progress inline layer rename in the layers panel. |
| `selected_layers` | `std::collections::BTreeSet<u64>` | Layers picked in the panel along with the active one (Ctrl- and Shift-click), by uid. Cleared when the active layer moves elsewhere. |
| `show_settings` | `bool` | Whether the Settings window is open. Session-only. |
| `show_brush_settings` | `bool` | The brush settings window. Session-only, like `show_settings`: where a window sits persists through egui's own memory, but whether it is open should not outlive the session that opened it. |
| `show_grid_settings` | `bool` | The active grid's settings window. Session-only, like the brush settings window. |
| `show_panels` | `bool` | Master visibility of all floating panel windows. Tab toggles it. |
| `hidden_panels` | `Vec<PanelId>` | Panels closed one at a time, from the Panels menu or the × in their title bar. `show_panels` still hides the rest all at once. |
| `panel_layout_gen` | `u32` | Which generation of panel windows egui is keeping positions and sizes for. Reset panel layout moves to a new one, so every panel opens as if for the first time; 0 is the ids panels have always had. |
| `panel_padding` | `u8` | How far, in points, the panels and the top bar keep from the window's edges. Settings > Appearance. |
| `panel_padding_laid` | `u8` | The padding the panels were last laid out to, so a change moves the ones it held. Kept with `viewport_rect`, on frames that draw panels. |
| `color_scheme` | `Scheme` | Settings > Appearance. Applied with `AppState::apply_scheme`. |
| `custom_colors` | `BaseColors` | The user's own base colours for the Custom colour scheme. |
| `show_mini_timeline` | `bool` | Minimal timeline bar shown when `show_panels` is false. Has its own toggle shortcut so the user can hide it too. |
| `show_new_project` | `bool` | Whether the New project dialog is open. |
| `new_project_cfg` | `NewProjectConfig` | The New project dialog's current values (size, fps, preset name…). |
| `show_export` | `bool` | MP4 export settings dialog visibility + backing config. |
| `export_cfg` | `ExportConfig` | The Export dialog's current values (kind, frame range, MP4 options…). |
| `show_import_range` | `bool` | Frame-range import dialog (video / GIF / WebP) visibility + backing state. |
| `import_range` | `Option<ImportRangeState>` | State of the frame-range import dialog (video/GIF/WebP): source, chosen range, previews. |
| `layer_xform` | `bool` | Layer transform mode: when on, the canvas pan/zoom/rotate gestures move, scale and rotate the active layer instead of the canvas view. |
| `nav_to_layer` | `bool` | True while the current drag is targeting the active layer's transform (decided at drag start) rather than the canvas view. |
| `layer_xform_before` | `Option<undo::TimelineState>` | Timeline snapshot captured at the start of a layer-transform drag, used to push a single undo entry when the drag ends. |
| `layer_opacity_before` | `Option<undo::TimelineState>` | The same for an opacity drag in the Layers panel, which may change several layers at once. |
| `opacity_row_uid` | `Option<u64>` | The layer whose row % is being dragged or typed into: its % stays on the row until it is let go, even at 100%, and the slider over the list leaves the undo step the row opened alone. |
| `group_pivot` | `Option<(f32, f32)>` | What a group being transformed turns and scales about, in document space: the middle of its layers when the drag started. |
| `xform_sync_last` | `Option<(usize, usize)>` | Last (layer, frame) the transform edit buffer was synced for, so the buffer is only reloaded from keys when the cursor actually moves. |
| `camera_edit` | `bool` | Camera edit mode: when on, the canvas pan/zoom/rotate gestures move the camera instead of the canvas view. Same retarget trick as `layer_xform`. |
| `nav_to_camera` | `bool` | True while the current drag is targeting the camera (decided at drag start) rather than the canvas view. |
| `camera_drag_before` | `Option<undo::TimelineState>` | Timeline snapshot captured at the start of a camera drag, so the whole drag lands as one undo entry. |
| `cam_drag` | `Option<CamDrag>` | The camera's frame, while a handle of it is being dragged. |
| `camera_box` | `bool` | Draw camera mode: every canvas drag draws a new frame for the camera and keys it on this frame. Never on with `camera_edit` — both claim the plain drag. |
| `camera_box_free` | `bool` | Draw camera mode's box takes any shape: its size, at zoom 1, becomes the camera's own — the size exports come out at. Off, the box keeps the camera's shape. A workspace preference. |
| `cam_box` | `Option<CamBox>` | The box, while one is being dragged out. |
| `cam_key_drag` | `Option<CamKeyDrag>` | A camera key, while it is being dragged along the timeline ruler. |
| `shot_cam` | `Option<f32>` | Where the camera is, as a time in frames, while a move stretched over loops is playing — it has its own place then, apart from the frame showing. `None` the rest of the time. See `follow_stretched_camera`. |
| `play_from` | `Option<usize>` | The frame playback started on, for Play/Pause to stop back on. Cleared by a pause that stays put. |
| `cam_sync_last` | `Option<usize>` | Last frame the camera edit buffer was synced for. |
| `camera_look_through` | `bool` | Lock the editor viewport to the camera, so what you see is the shot. |
| `show_camera_guide` | `bool` | Draw the camera's frame rect over the canvas. |
| `dim_outside_camera` | `bool` | Dim everything the camera can't see. |
| `camera_overlay` | `bool` | Draw thirds and the safe areas inside the camera's frame. A composition aid for the moment: off each launch. |
| `last_camera_size` | `Option<(u32, u32)>` | The camera frame size last set, kept between runs — the size itself is not. See `AppState::set_camera_size`. |
| `show_layer_bounds` | `bool` | Outline the active layer's cell bounds, so it's visible where a layer's drawable buffer ends (strokes past it are silently clipped). |
| `expand_cfg` | `Option<(usize, u32, u32)>` | Edit buffer for the "expand layer canvas" fields: `(layer, w, h)`. `None` (or a stale layer) means prefill from the active layer's size. |
| `prev_layer` | `Option<u64>` | Layer selected before the current one, for the "jump back" shortcut. Maintained by `track_layer_change` rather than by every site that assigns `current_layer` — there are eight of those. A uid, so adding or deleting layers around it doesn't lose it. |
| `layer_watch` | `(usize, usize)` | Previous frame's (current_layer, layer count), the baseline that `track_layer_change` diffs against. |
| `layer_watch_uid` | `Option<u64>` | Uid of the layer that was active last frame. |
| `tracker_two_points` | `bool` | Tracker tool: when on, each frame takes two clicks (point A then B) so stabilization can correct rotation/zoom shake as well as position. |
| `tracker_pending_b` | `Option<usize>` | Frame awaiting its second (B) tracker click in two-point mode. |
| `bg_job` | `Option<BgJob>` | In-flight background import step (ffmpeg / decode), polled each frame. |
| `bg_label` | `Option<&'static str>` | Label for the modal "busy" overlay while a `bg_job` runs. |
| `preview_tex` | `HashMap<usize, TextureHandle>` | Cached range-dialog preview textures, keyed by source frame index. |
| `preview_rx` | `Option<(usize, Receiver<Result<PreviewFrame>>)>` | In-flight preview extraction (video only), with the frame it's for. |
| `preview_clear_pending` | `bool` | Defer dropping the preview texture handles until the next frame's top. Confirm/cancel run *inside* the egui pass, where the dialog has already emitted draw commands referencing those textures; freeing them in the same frame makes wgpu submit a render pass that references a destroyed texture (validation error → crash). Clearing one frame later, with the dialog gone, is safe. |
| `max_tex` | `u32` | GPU max 2D texture dimension. Imported cells are capped to this so a huge source image can't exceed the limit and crash the texture upload. |
| `viewport_rect` | `Option<egui::Rect>` | Viewport rect as of the last frame that drew panels. Compared each frame to detect a window resize, which re-sticks panels to their nearest edge. |
| `window_styled` | `bool` | One-shot guard: native window chrome (rounded corners/border) applied. |
| `startup_maximize` | `bool` | One-shot: maximize on the first frame. `ViewportBuilder::with_maximized` alone does not take on this frameless window — winit creates it at the requested inner size and the creation-time flag is lost. Sending the viewport command once the window exists goes through `set_maximized`, which does. |
| `screen_pick` | `bool` | Live screen colour-pick mode: while true, the pixel under the cursor is sampled from the OS framebuffer and the next tap commits it as the brush colour. The backdrop is left untouched, so what you see is what you pick: the canvas where it's opaque, whatever is behind the window where it isn't. |
| `screen_pick_arm` | `bool` | Consume the press that opened pick mode: wait for the pointer to be released once before a tap counts as a commit. |
| `screen_pick_return_tool` | `ActiveTool` | Tool to restore after a screen-pick commits (so the tap returns the user to the drawing tool they were using). |
| `screen_pick_tex` | `Option<TextureHandle>` | Live zoom-loupe texture, refreshed each frame while picking. Held here so the handle outlives the frame's paint list. |

### `enum Act`
[src/app.rs:4375](../../src/app.rs#L4375)

A tiny enum declared *inside* `AppState::request_preview`. Rust lets you define a type inside a
function body when only that function needs it. It records the decision ("already have an
image", "start a video frame fetch", "nothing to do") so the function can decide first while
only *reading* `self.import_range`, and then act (which needs to *write* `self`) afterwards —
a common way of keeping the borrow checker happy.

| Variant | Meaning |
|---|---|
| `Have(ColorImage)` | A preview image is ready to upload as a texture. |
| `SpawnVideo(PathBuf, f64)` | Start a worker thread that pulls the frame at this time (seconds) out of this video file. |
| `Nothing` | A texture exists already, or a fetch is already running. |

### `const AppState::BLANK_SCANS_PER_FRAME`
[src/app.rs:5839](../../src/app.rs#L5839)

```rust
const BLANK_SCANS_PER_FRAME: u32 = 1;
```

How many cells `cell_is_blank` may scan in one frame. A blank cell must be read to the end, and on
a 3×-wide 4K layer one cell is about 100 MB. A `const` inside an `impl` block belongs to the type
(like a static class field in JS). More: [Constants and statics](../rust-primer.md#constants-and-statics)

### `const AppState::MAX_SWATCHES`
[src/app.rs:6303](../../src/app.rs#L6303)

```rust
pub const MAX_SWATCHES: usize = 32;
```

The most pinned colour swatches kept; past this the oldest is dropped so the strip stays one or
two rows. A `const` inside an `impl` is reached as `AppState::MAX_SWATCHES` (like a static class
field in JS). Used by `pin_rgb` and `swatch_strip` in [ui/shell.rs](ui/shell.md).

### `const AppState::TOAST_TTL`
[src/app.rs:7071](../../src/app.rs#L7071)

```rust
const TOAST_TTL: Duration = Duration::from_millis(2200);
```

How long a "saved" toast stays up.

### `const AppState::KRITA_NOTE`
[src/app.rs:7767](../../src/app.rs#L7767)

```rust
const KRITA_NOTE: Duration = Duration::from_secs(4);
```

How long a Krita message stays up. Warnings name what was dropped, so they get long enough to read.

### `const AppState::KRITA_WARN`
[src/app.rs:7768](../../src/app.rs#L7768)

```rust
const KRITA_WARN: Duration = Duration::from_secs(9);
```

How long a *warning* from the Krita link stays on screen at minimum (9 seconds). Notes use the
shorter `KRITA_NOTE`. `Duration` is Rust's time-span type (like a number of milliseconds with
units attached).

### `struct Imported`
[src/app.rs:8206](../../src/app.rs#L8206)

A struct declared inside `on_kra_imported`: one Krita layer converted to this app's terms,
waiting to be inserted as a layer. Collecting everything first and inserting afterwards means a
failure part-way leaves the project untouched.

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | The Krita layer's name. |
| `opacity` | `u8` | Krita's opacity, 0–255. |
| `visible` | `bool` | Whether the layer was visible in Krita. |
| `locked` | `bool` | Whether it was locked in Krita. |
| `cells` | `Vec<Canvas>` | Its drawings, as canvases the size of the Krita document. |
| `keys` | `Vec<(usize, usize)>` | `(frame, index into cells)` pairs — which drawing starts on which frame. |
| `group` | `bool` | This entry is a group header rather than a paint layer. |
| `in_group` | `bool` | This paint layer sits inside the group header before it. |
| `collapsed` | `bool` | For a group: whether it was folded in Krita's layer panel. |

## `impl Default for ProjectPreset`

### `ProjectPreset::default`
[src/app.rs:87](../../src/app.rs#L87)

```rust
fn default() -> Self
```

An unnamed 1920 × 1080, 24 fps preset that is not starred.

## `impl Default for Mp4ExportConfig`

### `Mp4ExportConfig::default`
[src/app.rs:111](../../src/app.rs#L111)

```rust
fn default() -> Self
```

`crf: 18` (about visually lossless) and `preset_idx: 2` ("medium", libx264's own default).

## `impl ExportKind`

### `ExportKind::title`
[src/app.rs:131](../../src/app.rs#L131)

```rust
pub fn title(self) -> &'static str
```

The dialog/file-picker title for this kind, e.g. `"Export animated GIF"`. `&'static str` is a
string literal baked into the program, which lives forever (the `'static` lifetime).

**Used by:** `start_export` (file dialog title), `export_dialog` in
[ui/shell.rs](ui/shell.md), and [ui/sweep_tests.rs](ui/sweep_tests.md).

### `ExportKind::busy`
[src/app.rs:142](../../src/app.rs#L142)

```rust
fn busy(self) -> &'static str
```

The present-tense label for the "busy" overlay while the export runs, e.g.
`"Exporting GIF…"`. **Used by:** `start_export`.

## `impl ExportConfig`

### `ExportConfig::playback`
[src/app.rs:178](../../src/app.rs#L178)

```rust
pub fn playback(&self, kind: ExportKind) -> crate::io::frame_order::Playback
```

**What it does.** Picks out the playback options that apply to `kind`: ping-pong for every
kind except a sprite sheet, and the loop length only for MP4 and PNG sequences (and only when
`loop_on`). `bool::then_some(x)` turns `true` into `Some(x)` and `false` into `None`.

**Returns.** A [`frame_order::Playback`](io/frame_order.md) (`ping_pong`, `loop_secs`).

**Used by:** `export_info` and `export_order` (later on this page).

## `impl Default for ExportConfig`

### `ExportConfig::default`
[src/app.rs:188](../../src/app.rs#L188)

```rust
fn default() -> Self
```

PNG sequence, range 0..0, default MP4 settings, near-square sheet with no padding, no
ping-pong, looping off with a 10-second target, WebP looping forever (plays = 1 when stopped).

## `impl MidKey`

### `MidKey::on`
[src/app.rs:274](../../src/app.rs#L274)

```rust
fn on(self) -> bool
```

True when the key is held *and* not stale. **Used by:** `move_on`, `centre_on`.

### `MidKey::read`
[src/app.rs:278](../../src/app.rs#L278)

```rust
fn read(&mut self, held: bool)
```

Records this frame's key state. `self.stale &= held` keeps `stale` true only while the key
stays down — releasing it clears staleness for good. **Used by:** `read_input`,
`hold_mid_keys`.

### `MidKey::began`
[src/app.rs:283](../../src/app.rs#L283)

```rust
fn began(&mut self)
```

Called when a drag starts: a key already held right now becomes stale. **Used by:**
`drag_began`.

**Example.** The life of a key across one drag:
```js
// held at press → doesn't count; released and re-pressed → counts
let k = { held: true, stale: false };
k.stale = k.held;          // began(): stale = true
k.held = true;  k.stale &&= true;   // read(true):  on() = false
k.held = false; k.stale &&= false;  // read(false): stale cleared
k.held = true;  k.stale &&= true;   // read(true):  on() = true
```

## `impl Default for View`

### `View::default`
[src/app.rs:322](../../src/app.rs#L322)

```rust
fn default() -> Self
```

Zoom 1, no pan, no rotation, no flips. Used by `with_prefs` and `reset_with`.

## `impl Default for FadeOthers`

### `FadeOthers::default`
[src/app.rs:550](../../src/app.rs#L550)

```rust
fn default() -> Self
```

`above: 0.2`, `below: 0.35`.

## Free functions

### `default_tool_brushes`
[src/app.rs:569](../../src/app.rs#L569)

```rust
fn default_tool_brushes() -> [BrushSettings; TOOL_COUNT]
```

Builds the factory brush for every tool, in the prefs' layout: a fixed-size array
`[BrushSettings; TOOL_COUNT]`, the pencil and the ink first, then the other tools. A Rust array
type includes its length, so the compiler guarantees there is exactly one brush per place.

### `restore_tool_brushes`
[src/app.rs:591](../../src/app.rs#L591)

```rust
fn restore_tool_brushes(saved: Option<Vec<BrushSettings>>) -> [BrushSettings; TOOL_COUNT]
```

Takes the per-tool brushes saved in the preferences file and turns them back into the
fixed-size array. If the saved list has the wrong length (it came from a build with a different
set of tools) it is thrown away and the defaults are used, because slot *n* must always mean
tool *n* — a shorter list would hand brushes to the wrong tools. One exception: a list from just
before the last-added tools (Perspective, then Liquify) still lines up for the older slots, so
those are kept and only the new tools get defaults.

### `other_brushes`
[src/app.rs:606](../../src/app.rs#L606)

```rust
fn other_brushes(legacy: &[BrushSettings; TOOL_COUNT]) -> [BrushSettings; OTHER_TOOLS]
```

The brushes of the tools that aren't pens, out of the prefs' layout.

### `restore_pen_smoothing`
[src/app.rs:619](../../src/app.rs#L619)

```rust
fn restore_pen_smoothing(prefs: &UiPrefs) -> [SmoothingOptions; LEGACY_SMOOTHING]
```

Returns the line-smoothing setting for each freehand tool from before the pen slots (pencil,
ink, eraser); the first two seed the first two slots of an old prefs file, the third is the
eraser's. It starts every
slot from the old single `smoothing` setting (`[prefs.smoothing; N]` makes an array of N copies)
and then overwrites slots with any per-tool values saved. `iter_mut().zip(...)` walks two
sequences side by side, like looping over two arrays with one index.

## `impl Default for UiPrefs`

### `UiPrefs::default`
[src/app.rs:628](../../src/app.rs#L628)

```rust
fn default() -> Self
```

The out-of-the-box preferences: panels and mini timeline shown, frame step 1, camera guide and
dimming on, layer bounds on, middle drag = `Scrub`, right drag = `SwatchPie`, loop on,
selection freehand with amount 4, bucket fill, backdrop `DEFAULT_BG_COLOR` at opacity 1,
panel padding `DEFAULT_PANEL_PADDING`, track frame width `ui::tracks::DEFAULT_FRAME_W`, and
everything else off / empty / that type's own default. Used when there is no stored blob
(`unwrap_or_default()` in `new`) and to fill missing keys.

## `impl Dirty`

### `Dirty::with`
[src/app.rs:694](../../src/app.rs#L694)

```rust
pub fn with(self, more: Dirty) -> Dirty
```

**What it does.** Combines two pending regions: `Full` wins over anything, `Clean` adds
nothing, and two rects become their bounding union (via `union_rect` from
[tools/ribbon.rs](tools/ribbon.md)).

**Used by:** `mark_region`, `sync_textures`, and [app/groups.rs](app/groups.md).

## Free functions

### `tex_rect`
[src/app.rs:776](../../src/app.rs#L776)

```rust
fn tex_rect(content: Option<DirtyRect>, cw: u32, ch: u32) -> DirtyRect
```

Decides which part of a drawing its GPU texture should cover: the drawing's painted area
(`content`) plus a small rim (`TEX_RIM`) around it, clamped to the cell. A blank cell gets an
empty rectangle. Uploading only the painted area keeps GPU memory small for mostly-empty cells.

### `whole_rect`
[src/app.rs:794](../../src/app.rs#L794)

```rust
fn whole_rect(cw: u32, ch: u32) -> DirtyRect
```

The rectangle covering an entire `cw`×`ch` cell, from (0,0) to (cw,ch).

### `rect_within`
[src/app.rs:804](../../src/app.rs#L804)

```rust
fn rect_within(r: DirtyRect, outer: DirtyRect) -> bool
```

True when rectangle `r` lies completely inside `outer`.

### `rect_meet`
[src/app.rs:809](../../src/app.rs#L809)

```rust
fn rect_meet(a: DirtyRect, b: DirtyRect) -> Option<DirtyRect>
```

The overlap of two rectangles, or `None` if they don't overlap. `bool.then_some(r)` is a handy
Rust idiom: it gives `Some(r)` when the condition is true and `None` otherwise.

### `tex_dims`
[src/app.rs:820](../../src/app.rs#L820)

```rust
fn tex_dims(rect: DirtyRect) -> [usize; 2]
```

The width and height in texels a texture needs for `rect`; never less than 1×1, because a GPU
texture can't be empty.

### `stroke_room`
[src/app.rs:859](../../src/app.rs#L859)

```rust
fn stroke_room(need: Option<DirtyRect>, cw: u32, ch: u32) -> DirtyRect
```

Chooses the texture area for a drawing while a live stroke is painting into it. It takes the area
the stroke needs and adds generous room around it (half its longer side, at least `STROKE_ROOM`
pixels), so a stroke that keeps spreading only forces a bigger texture a few times instead of
every frame. Once the padded area is most of the cell it just uses the whole cell, which never
needs re-making.

### `rect_join`
[src/app.rs:885](../../src/app.rs#L885)

```rust
fn rect_join(a: Option<DirtyRect>, b: Option<DirtyRect>) -> Option<DirtyRect>
```

The smallest rectangle containing both `a` and `b`, where either may be missing or empty. The
local closure `some` filters out empty rectangles before the `match` combines them.

### `plan_upload`
[src/app.rs:903](../../src/app.rs#L903)

```rust
fn plan_upload(c: &Canvas, have: Option<DirtyRect>, dirty: Dirty, stroke: Option<Option<DirtyRect>>) -> TexPlan
```

Decides how to bring a drawing's GPU texture up to date after its pixels changed, returning a
`TexPlan` (patch in place, or re-crop into a new texture).

**How it works.**
1. If the change (`dirty`) lies inside what the texture already covers (`have`), it is a
   **patch**: upload just that rectangle.
2. If the change reaches outside it, the drawing may have grown, so the texture is **re-cropped**
   to the drawing's new extent.
3. During a live stroke (`stroke` is `Some`), it instead grows the texture with spare room
   (`stroke_room`) so the rest of the stroke can stream in as cheap patches.

Nothing scans pixels: the drawing's extent is computed from what was there before plus what the
edit touched (`Canvas::dirty`). This matters on huge (8K) canvases where a scan is slow.

### `crop_image`
[src/app.rs:941](../../src/app.rs#L941)

```rust
fn crop_image(rect: DirtyRect, straight: impl FnOnce() -> Vec<u8>) -> ColorImage
```

Makes an egui `ColorImage` for `rect`, premultiplying the straight-alpha pixels returned by the
`straight` closure (see [premultiplied alpha](../glossary.md)). Taking a closure
(`impl FnOnce() -> Vec<u8>`) means the pixels are only copied out if they're actually needed — an
empty rectangle gets a single transparent texel without reading anything.

### `cell_image`
[src/app.rs:953](../../src/app.rs#L953)

```rust
fn cell_image(c: &Canvas, rect: DirtyRect) -> ColorImage
```

Like `crop_image`, but reads straight from the cell's pixels into the texture image with no
intermediate copy, and splits the work across several threads when the area is big (an 8K cell
is 33 million pixels). An empty rectangle gets one clear texel.

### `evictions`
[src/app.rs:996](../../src/app.rs#L996)

```rust
fn evictions(entries: &[TexEntry], budget: usize, now: u64, playing: bool) -> Vec<TexEntry>
```

Chooses which textures to free so the total fits the GPU memory budget, never touching anything
shown on the current sync. Normally it drops the least recently shown first. During playback it
does the opposite: playback walks the frames in order, so the texture shown longest ago is the
one needed *next* — dropping it would miss on every frame of a loop that doesn't fit. Dropping
the newest instead keeps most of the loop resident.

## `impl Default for NewProjectConfig`

### `NewProjectConfig::default`
[src/app.rs:1049](../../src/app.rs#L1049)

```rust
fn default() -> Self
```

1920 × 1080 at 24 fps, empty name box, no rename, a default clipboard watcher. Used by
`with_prefs`, `reset_with` and `open_new_project` (via `..Default::default()`, which means
"every field I did not list comes from `default()`", like `{ ...defaults, width, height }` in JS).

## `impl AppState`

### `AppState::new`
[src/app.rs:1754](../../src/app.rs#L1754)

```rust
pub fn new(cc: &CreationContext<'_>) -> Self
```

The constructor eframe calls once, when the window has been created (see
[main.rs](main.md)). `cc` (the "creation context") gives access to the GPU, the saved
preferences storage and egui.

**How it works.**
1. Installs the app's fonts/theme into egui (`ui::theme::install`).
2. Reads the GPU's maximum texture size (at least 2048, assuming 8192 if there is no GPU info);
   imported images are later shrunk to fit it.
3. Loads the saved `UiPrefs` from eframe's storage (`ui.ron`), or defaults if none.
4. Builds the rest with `with_prefs`, pointing the brush library at `<config dir>/brushes`.
5. Creates the custom canvas renderer `CanvasGpu` and applies the colour scheme.

The chain `cc.storage.and_then(|s| eframe::get_value(s, KEY)).unwrap_or_default()` reads as:
"if there is storage, try to read the key; if anything is missing, use the default prefs".

### `AppState::for_test`
[src/app.rs:1778](../../src/app.rs#L1778)

```rust
pub fn for_test() -> Self
```

A test-only constructor (`#[cfg(test)]`): default preferences, an 8192 texture limit, and no
brush folder — no window, GPU or user files involved. Tests across the app use it to drive
`AppState` headlessly.

### `AppState::with_prefs`
[src/app.rs:1784](../../src/app.rs#L1784)

```rust
fn with_prefs(prefs: UiPrefs, max_tex: u32, brush_root: Option<PathBuf>) -> Self
```

The real body of construction, shared by `new` and `for_test`. It creates a fresh 1280×720,
24 fps `Project`, loads the keyboard shortcuts, marks every cell's texture as needing a full
upload, restores per-tool brushes and smoothing from `prefs`, and then fills in every one of
`AppState`'s fields — most with an empty or default value, the workspace preferences from
`prefs`. It is long only because `AppState` has so many fields: Rust requires every field to be
given a value when a struct is built.

### `AppState::reset`
[src/app.rs:2029](../../src/app.rs#L2029)

```rust
pub fn reset(&mut self)
```

File → New with the default size: calls `reset_with(1280, 720, 24.0)`.

### `AppState::reset_with`
[src/app.rs:2033](../../src/app.rs#L2033)

```rust
pub fn reset_with(&mut self, width: u32, height: u32, fps: f32)
```

Starts a fresh project of the given size and frame rate while keeping the user's own things
(brushes, shortcuts, panel layout, onion and backdrop settings, the pen).

**How it works.**
1. Waits for any save still running (`finish_pending_save`) so it can't report back and point
   the *new* project at the old file; ends the Krita link; stashes the current file's
   perspective grids.
2. Replaces `project` with `Project::new(width, height, fps)` and forgets `project_path`, so the
   next Save asks where to save instead of overwriting the old file.
3. Retires every GPU texture (parked, not freed — see `retired_textures`), marks all cells dirty,
   resets the view, playback, undo history and every in-progress drag, dialog and selection.
4. Switches back to the Pencil, keeping every tool's brush.

Comments in the code note which fields are *deliberately* not reset (workspace preferences).

### `AppState::open_new_project`
[src/app.rs:2127](../../src/app.rs#L2127)

```rust
pub fn open_new_project(&mut self)
```

Opens the New project dialog. It pre-fills the starred preset if there is one, otherwise the
current project's size and frame rate.

### `AppState::preset_named`
[src/app.rs:2142](../../src/app.rs#L2142)

```rust
pub fn preset_named(&self, name: &str) -> Option<usize>
```

Finds a saved project preset by name, ignoring upper/lower case and surrounding spaces. Returns
its index, or `None`. `.iter().position(...)` is Rust's `findIndex`, returning an `Option`
instead of `-1`.

### `AppState::save_preset`
[src/app.rs:2152](../../src/app.rs#L2152)

```rust
pub fn save_preset(&mut self) -> bool
```

Saves the New project dialog's current size/fps under the name typed in its name box. If a preset
with that name exists it is replaced in place (keeping its position and its star). Returns
`false` for a blank name.

### `AppState::apply_preset`
[src/app.rs:2178](../../src/app.rs#L2178)

```rust
pub fn apply_preset(&mut self, i: usize)
```

Fills the New project dialog from saved preset `i` exactly as saved (unlike the built-in sizes, a
saved preset keeps the orientation it was saved with). The tuple assignment
`(cfg.width, cfg.height, cfg.fps) = (p.width, p.height, p.fps)` sets three fields at once, like JS
destructuring assignment.

### `AppState::rename_preset`
[src/app.rs:2187](../../src/app.rs#L2187)

```rust
pub fn rename_preset(&mut self, i: usize, name: &str) -> bool
```

Renames preset `i`. Refuses (returns `false`) a blank name or one already used by another preset;
changing only the capitalisation of its own name is allowed.

### `AppState::delete_preset`
[src/app.rs:2199](../../src/app.rs#L2199)

```rust
pub fn delete_preset(&mut self, i: usize)
```

Removes preset `i` (if the index is valid) and cancels any rename in progress, since the rename's
index might now point at a different preset.

### `AppState::toggle_default_preset`
[src/app.rs:2209](../../src/app.rs#L2209)

```rust
pub fn toggle_default_preset(&mut self, i: usize)
```

Stars preset `i` so New project opens on it, un-starring all others; starring the already-starred
one clears the star. `let Some(on) = … else { return; }` is *let-else*: bind the value or leave
the function early.

### `AppState::make_sample`
[src/app.rs:2222](../../src/app.rs#L2222)

```rust
pub fn make_sample(&self, x: f32, y: f32, t: f32) -> PointerSample
```

Builds a `PointerSample` (one input point for a stroke) at a position in cell pixels, using the
pressure and tilt from the most recent tablet packet. Used for mouse input and for the first
point of a stroke; real tablet packets go through `make_pen_sample`, since each carries its own
pressure and tilt.

### `AppState::make_pen_sample`
[src/app.rs:2244](../../src/app.rs#L2244)

```rust
pub fn make_pen_sample(&self, x: f32, y: f32, t: f32, p: &PenPacket) -> PointerSample
```

Builds a `PointerSample` from one tablet packet `p`, at a position already converted to cell
pixels. Walking every packet in the queue (instead of sampling the pen once per frame) is what
gives strokes smooth pressure changes.

### `AppState::cell_screen_orientation`
[src/app.rs:2265](../../src/app.rs#L2265)

```rust
pub fn cell_screen_orientation(&self) -> (f64, (bool, bool))
```

How the active drawing is turned on screen: its clockwise rotation in degrees (with any mirroring
removed) and whether the view is mirrored horizontally/vertically. Krita brushes need this
("canvas rotation") because their tilt sensors measure the pen relative to the screen.

### `AppState::cell_view_scale`
[src/app.rs:2283](../../src/app.rs#L2283)

```rust
pub fn cell_view_scale(&self) -> f32
```

Screen pixels per pixel of the *active drawing*: the view zoom multiplied by the active layer's own
scale (strokes are painted in the drawing's own pixel grid). Never returns 0 (`max(1e-6)`), so it
is safe to divide by.

### `AppState::krita_brush`
[src/app.rs:2290](../../src/app.rs#L2290)

```rust
pub fn krita_brush(&self) -> Arc<crate::tools::krita::PixelBrush>
```

Returns the Krita brush preset the current brush paints with. If the brush names a preset that the
library has (with the user's edits applied, or unedited if the edit no longer builds), that one;
otherwise the built-in preset. It returns an `Arc` — a shared, reference-counted pointer — so
callers share one brush instead of copying it.

### `AppState::tick_stroke`
[src/app.rs:2303](../../src/app.rs#L2303)

```rust
pub fn tick_stroke(&mut self) -> bool
```

Krita's stabilizer keeps painting on a timer even while the pen is held still (the line "catches
up" to the pen). Called every frame from `update`: if a stabilized stroke is running, it advances
the stroke's clock, paints what it adds, and returns `true` so `update` keeps requesting frames.

### `AppState::open_krita_import`
[src/app.rs:2313](../../src/app.rs#L2313)

```rust
pub fn open_krita_import(&mut self)
```

Opens the "Import Krita brushes" window and starts reading Krita's installed presets in the
background (finding Krita first). Does nothing if the window is already open.

### `AppState::close_krita_import`
[src/app.rs:2326](../../src/app.rs#L2326)

```rust
pub fn close_krita_import(&mut self)
```

Closes the "Import Krita brushes" window. Its thumbnail textures were drawn this frame, so they
are moved to `retired_textures` (freed next frame) instead of being dropped now — freeing a texture
the GPU is about to draw crashes wgpu.

### `AppState::rescan_brushes`
[src/app.rs:2334](../../src/app.rs#L2334)

```rust
pub fn rescan_brushes(&mut self)
```

Re-reads the imported brushes after an import and forgets their old thumbnails.

### `AppState::forget_user_brush_thumbs`
[src/app.rs:2343](../../src/app.rs#L2343)

```rust
pub fn forget_user_brush_thumbs(&mut self)
```

Makes the brush picker reload thumbnails for imported (`user/…`) brushes. The old textures are
parked in `retired_textures` rather than dropped, for the same "might be on screen this frame"
reason as `close_krita_import`.

### `AppState::effective_radius`
[src/app.rs:2355](../../src/app.rs#L2355)

```rust
pub fn effective_radius(&self) -> f32
```

The brush radius in active-drawing pixels. With "lock brush to view" on, the stored radius means
*screen* pixels, so it is divided by the current scale — the brush then looks the same size at any
zoom. Clamped to 0.1–4096.

### `AppState::mark_dirty`
[src/app.rs:2367](../../src/app.rs#L2367)

```rust
pub fn mark_dirty(&mut self, id: CellId)
```

Records that cell `id`'s pixels changed so its texture gets re-uploaded. The changed area comes
from the canvas itself (`Canvas::dirty`, grown by every pixel write); a canvas without one is
treated as changed everywhere (`Dirty::Full`).

### `AppState::mark_region`
[src/app.rs:2376](../../src/app.rs#L2376)

```rust
fn mark_region(&mut self, id: CellId, region: Dirty)
```

Records that `region` of cell `id` changed: merges it into `cell_dirty`, drops the cached "is this
cell blank?" answer, and flags any ghost, clip or group texture built from the cell as stale.

### `AppState::collect_cells`
[src/app.rs:2414](../../src/app.rs#L2414)

```rust
fn collect_cells(&mut self)
```

**What it does.** A garbage collector for drawings. It frees the pixels of every cell that
nothing can show again: not on any layer, out of reach of undo and redo, and not held by an
edit in progress or by the Krita link. Before this existed the cell pool only ever grew
(deleted keys and layers, drawings drawn over), reaching 2.3 GB of dead cells on a 2160×2880
project. A freed cell becomes a **tombstone** (an empty 0×0 `Canvas`) rather than being
removed, so no other cell's index (`CellId`) shifts.

**How it works.**
1. Skips entirely while a Krita send or pull is in flight (those match drawings to cells by
   id), or when nothing changed since the last run: the key is the pair (undo history
   revision, number of cells), cached in `gc_key`.
2. Builds a `used` flag per cell and a small closure `keep` that sets a flag. It marks:
   every cell any layer exposes on any frame; every cell the undo/redo history still names
   (`History::for_each_cell`); the cell under the stroke being drawn; the floating
   selection's cell and its followers' cells; and every cell the Krita link's baseline maps a
   Krita drawing onto.
3. For each unmarked cell that is not already a tombstone: replaces it with
   `Arc::new(Canvas::tombstone())`, and drops everything cached for it: its GPU texture
   (parked in `retired_textures`, see below), texture-use and dirty records, its onion ghost
   (`retire_ghost`), group images that used it (`retire_group_images`), any clip textures
   where it is the clipped cell or the base, and its blank-scan cache entry.

**Used by:** `AppState::update` (once a frame, in this file).

### `AppState::pack_idle_cells`
[src/app.rs:2474](../../src/app.rs#L2474)

```rust
fn pack_idle_cells(&mut self)
```

**What it does.** Shrinks idle drawings in memory. A `Canvas` can be *packed*: it keeps only
the rectangle that has anything drawn in it, instead of every pixel (see
[doc/canvas.rs](doc/canvas.md)). Writing to a cell unpacks it; this method packs the cells
nobody is drawing on again. It stops after `PACK_BUDGET` (4 ms) per frame, so a bulk edit that
unpacked dozens of cells is packed back over a few frames instead of in one hitch.

**How it works.**
1. Works out the "hot" cells to leave alone: the stroke's target cell, the cell shown on the
   active layer at the current frame, and the floating selection's cell (plus followers').
2. For every other cell that is not already packed, asks `Arc::get_mut` for sole access. If
   the cell is shared (with a save in flight or an undo step) it is skipped; packing would
   force a copy. It will be tried again on a later frame once they let go.
3. Calls `pack()` on it, and breaks out of the loop once the time budget is spent.

**Used by:** `AppState::update` (once a frame, in this file).

### `AppState::retire_ghost`
[src/app.rs:2496](../../src/app.rs#L2496)

```rust
fn retire_ghost(&mut self, id: CellId)
```

**What it does.** Removes one cell's onion-skin **ghost** texture (the tinted silhouette drawn
for neighbouring frames) and parks its handle in `retired_textures` instead of freeing it now.
Dropping a `TextureHandle` frees the GPU texture, and freeing one the canvas has already queued
for drawing this frame crashes wgpu, so handles wait one frame. Also forgets that the ghost was
stale.

**Used by:** `collect_cells`, `sync_ghosts`, `enforce_texture_budget` (all in this file).

### `AppState::retire_cell_textures`
[src/app.rs:2506](../../src/app.rs#L2506)

```rust
fn retire_cell_textures(&mut self)
```

**What it does.** Parks *every* cell texture, clip texture and group image texture in
`retired_textures` (to be freed next frame) and clears their "last used" records. File → New
and File → Open throw away the whole cell pool partway through a frame, after the canvas has
already been painted, so they need this delayed release.

**Used by:** `reset_with` and `load_project` (in this file).

### `AppState::retire_all_ghosts`
[src/app.rs:2519](../../src/app.rs#L2519)

```rust
fn retire_all_ghosts(&mut self)
```

**What it does.** The same as `retire_cell_textures`, for every onion ghost texture: clears
`ghost_stale`, parks every ghost handle in `retired_textures`, and clears the ghost-use records.

**Used by:** `reset_with` and `load_project` (in this file).

### `AppState::apply_touched`
[src/app.rs:2528](../../src/app.rs#L2528)

```rust
fn apply_touched(&mut self, touched: Option<undo::Touched>)
```

**What it does.** After an undo or redo, re-uploads to the GPU only what that step changed. The
history reports what it touched as an `undo::Touched` value (see [undo.rs](undo.md)); an edit
that only moved the timeline changed no pixels and so re-uploads nothing.

**Parameters.**
- `touched`: what the undo/redo step changed, or `None` if there was nothing to undo/redo.

**How it works.** A `match` on the variant:
- `Cell(id)`: one cell's pixels changed: `mark_dirty(id)` (re-upload the region its
  `Canvas::dirty` rect says).
- `Cells(ids)`: these cells changed wholesale: each is marked `Dirty::Full`, because the dirty
  rect inside a buffer the history handed back may be stale.
- `Selection(m)`: puts the selection mask back with `set_mask(m, false)` (`false` = don't
  record another undo step).
- `Grid(g)`: puts a perspective grid back with `restore_grid`.
- `Many(all)`: calls itself for each part (from a compound undo step).
- `Structure` or `None`: nothing to re-upload.

**Used by:** `AppState::undo` and `AppState::redo` (in this file).

### `AppState::frame_step_count`
[src/app.rs:2552](../../src/app.rs#L2552)

```rust
pub fn frame_step_count(&self) -> usize
```

**What it does.** How many frames one "step" action moves or inserts: the user's
`frame_step` setting, but never less than 1 (`max(1)`), so a step of zero from a cleared input
or an old prefs file still does something.

**Used by:** `add_frames`, `duplicate_frames`, `insert_frames_before`, `frame_step_delta` (this
file); the timeline panel in [ui/shell.rs](ui/shell.md); the track context menu in
[ui/tracks.rs](ui/tracks.md).

### `AppState::frame_step_delta`
[src/app.rs:2557](../../src/app.rs#L2557)

```rust
pub fn frame_step_delta(&self) -> isize
```

**What it does.** The same number as `frame_step_count`, converted to a signed integer
(`isize`, which can be negative) so callers can negate it to step backwards with
`Project::step`. `as isize` is a numeric cast; see
[Numbers and casts](../rust-primer.md#numbers-and-casts).

**Used by:** `dispatch` (FramePrev / FrameNext actions, this file); the wheel scrub, the
timeline panel's ◀ / ▶ buttons and the mini timeline in [ui/shell.rs](ui/shell.md).

### `AppState::structural_edit`
[src/app.rs:2567](../../src/app.rs#L2567)

```rust
pub fn structural_edit(&mut self, capture_cells: bool, edit: impl FnOnce(&mut Project))
```

**What it does.** The standard way to make an undoable timeline or layer edit: it runs `edit`
on the project and records the change as one undo step. It is a two-line wrapper: build the
step with `structural_command`, then push it onto `history`.

**Parameters.**
- `capture_cells`: also remember the before/after of any cell whose pixels the edit changes.
  Only needed by the destructive "delete the last frame" path, which clears pixels. Every
  other structural edit leaves the pixels alone, so the cheap `TimelineState` snapshot is
  enough.
- `edit`: a closure that changes the project. `impl FnOnce(&mut Project)` means "a function
  called once with mutable access to the `Project`".

**Example.**
```rust
// Reset the active layer's transform, undoably (from reset_active_layer_transform).
self.structural_edit(false, |p| {
    let li = p.current_layer;
    if let Some(l) = p.layers.get_mut(li) {
        l.transform = Transform::default();
    }
});
```

**Used by:** most commands in the app. In this file: `import_cells_as_layer`,
`merge_layer_down`, `merge_group`, `paste_image_as_background`, the transform-key methods,
`key_drawn_camera`, the camera key and framing methods, `cut_cell`, `paste_cell`,
`track_edit`, `move_active_layer`, `add_cleanup_layer`, the layer grouping/deleting methods,
`dispatch`, the Krita pull and `.kra` import. Elsewhere: [app/orient.rs](app/orient.md)
(`orient_cells`) and the Layers panel and x-sheet in [ui/shell.rs](ui/shell.md).

### `AppState::structural_command`
[src/app.rs:2574](../../src/app.rs#L2574)

```rust
fn structural_command(&mut self, capture_cells: bool, edit: impl FnOnce(&mut Project)) -> undo::Command
```

**What it does.** Does the work of `structural_edit` but hands the undo step back instead of
recording it, for callers that want to add more to the same step.

**Returns.** An `undo::Command::Structural` holding the timeline before and after, plus
before/after buffers for any cell the edit changed (only collected when `capture_cells`).

**How it works.**
1. Lands any floating selection first (`land_float`): its pixels belong to a cell that may be
   about to move.
2. Clears the timeline-track selection (`track_sel`), since keys may move under it; track
   edits that want a selection re-select afterwards.
3. Takes the `before` snapshot (`undo::TimelineState::capture`). If `capture_cells`, also
   copies the list of cell *handles* (`Arc`s, not pixels, so this is cheap).
4. Runs `edit`, then takes the `after` snapshot.
5. Compares each old handle with the cell now at that index using `Arc::ptr_eq`. Because a
   write copies a shared cell first, a different pointer means changed pixels. Each changed
   cell becomes an `undo::CellPixelDelta` and is marked dirty for re-upload; unchanged cells
   keep their GPU textures.

**Used by:** `structural_edit`, `frame_edit`, `edit_cell_on` (in this file).

### `AppState::frame_edit`
[src/app.rs:2623](../../src/app.rs#L2623)

```rust
fn frame_edit(
    &mut self,
    capture_cells: bool,
    grids: impl Fn(&mut perspective::PerspectiveGrid),
    edit: impl FnOnce(&mut Project),
)
```

**What it does.** A structural edit that inserts or removes whole frames on every layer. The
project's own keys move inside `edit`, but perspective grids live on the app (not in the
project), so their keys are moved by the separate `grids` closure, in the same undo step.

**Parameters.**
- `capture_cells`: passed on to `structural_command`.
- `grids`: what to do to each perspective grid's keys (e.g. shift them by the frames
  inserted).
- `edit`: the change to the project.

**How it works.**
1. Builds the structural command for `edit`.
2. For each perspective grid: clones it, runs `grids` on it, and if it changed adds an
   `undo::Command::Grid { before, after }`.
3. Sets `grid_sync_last = None` so keyed grids take the pose their keys now give this frame.
4. Pushes one command, or a `Command::Compound` of all of them so they undo as one step.

**Used by:** `add_frames`, `duplicate_frames`, `delete_frame`, `insert_frames_before`.

### `AppState::add_frames`
[src/app.rs:2647](../../src/app.rs#L2647)

```rust
pub fn add_frames(&mut self)
```

**What it does.** Adds the step size's worth of held frames right after the playhead, on every
layer, as one undo step (calls `Project::add_frame` that many times). Perspective grid keys from
that point on move along with them (`keys_insert_frames`).

**Used by:** `dispatch` (`Action::FrameAdd`, this file); the timeline panel in
[ui/shell.rs](ui/shell.md).

### `AppState::duplicate_frames`
[src/app.rs:2659](../../src/app.rs#L2659)

```rust
pub fn duplicate_frames(&mut self)
```

**What it does.** Like `add_frames`, but each new frame is a copy of the current frame
(`Project::duplicate_frame`), on every layer. One undo step.

**Used by:** `dispatch` (`Action::FrameDuplicate`, this file); the timeline panel in
[ui/shell.rs](ui/shell.md).

### `AppState::delete_frame`
[src/app.rs:2671](../../src/app.rs#L2671)

```rust
pub fn delete_frame(&mut self)
```

**What it does.** Deletes the frame under the playhead from every layer
(`Project::delete_frame`). If it is the only frame left, that frame is cleared instead, which
wipes pixels, so in that case the edit captures cell pixels for undo (`capture_cells = true`)
and leaves the grid keys alone. Otherwise grid keys at that frame are removed and later ones
shift back by one.

**Used by:** `dispatch` (`Action::FrameDelete`, this file); the timeline panel in
[ui/shell.rs](ui/shell.md).

### `AppState::insert_frames_before`
[src/app.rs:2688](../../src/app.rs#L2688)

```rust
pub fn insert_frames_before(&mut self)
```

**What it does.** Inserts the step size's worth of frames *before* the playhead on every layer:
holds of whatever shows before the playhead, or blank frames when it is at the very start.
Every key from the playhead on (the perspective grids' keys too) moves that many frames later,
and the playhead stays on the drawing it was on (`Project::insert_frames_before`). One undo
step.

**Used by:** `dispatch` (`Action::FrameInsertBefore`, this file); the timeline panel in
[ui/shell.rs](ui/shell.md); the track context menu in [ui/tracks.rs](ui/tracks.md).

### `AppState::import_cells_as_layer`
[src/app.rs:2701](../../src/app.rs#L2701)

```rust
pub fn import_cells_as_layer(
    &mut self,
    name: impl Into<String>,
    cells: Vec<Canvas>,
    on_all_frames: bool,
)
```

**What it does.** Puts a list of pictures onto a brand-new layer *below* the active layer, as
one undoable structural edit. The active layer itself is never changed. Does nothing for an
empty list.

**Parameters.**
- `name`: the new layer's name. `impl Into<String>` accepts a `&str` or a `String`.
- `cells`: the pictures, one per frame (or just one).
- `on_all_frames`: `true` keys only the first picture at frame 0, so it shows on every frame
  through holding. `false` gives each picture its own frame starting at frame 0, growing the
  timeline (`ensure_frame_count`) if there are more pictures than frames.

**How it works.**
1. Scales any picture larger than the GPU's texture limit (`max_tex`) down to fit, keeping its
   aspect ratio (`cap_canvas`), so a huge source can't crash the texture upload.
2. Works out a starting layer transform with `fit_transform`: the picture is centred and
   shrunk (never enlarged) to fit inside the canvas.
3. Inside `structural_edit`: adds the layer (`add_layer_below_active`), sets its transform,
   pushes each picture onto the cell pool and keys it on its frame (`set_key`).

**Used by:** `import_image`, `confirm_import_range`, `on_video_extracted` (in this file).

### `AppState::can_merge_down`
[src/app.rs:2740](../../src/app.rs#L2740)

```rust
pub fn can_merge_down(&self) -> bool
```

**What it does.** Whether Merge Down is allowed: there must be a layer below the active one,
both must be editable drawing layers (`Project::paintable`: not locked, not a reference layer,
not a group header), and both must be in the same group (or both in none).

**Used by:** `merge_layer_down` (this file); the Layers panel in [ui/shell.rs](ui/shell.md),
to enable or grey out the Merge Down button.

### `AppState::merge_layer_down`
[src/app.rs:2758](../../src/app.rs#L2758)

```rust
pub fn merge_layer_down(&mut self)
```

**What it does.** Merges the active layer onto the layer below and removes it. Both layers'
per-frame transforms and opacities are "baked" into fresh drawings, so the merged layer ends
with an identity transform (no move, scale or rotation) at full opacity. It only *creates*
cells and never edits existing ones, so the cheap timeline snapshot is a correct undo (undo just
leaves the new cells unused, for `collect_cells` to free later).

**How it works.**
1. Returns if `can_merge_down` says no; stops playback.
2. If the top layer is **clipped** (drawn only where the layer under it has paint), it merges
   as it shows, cut to its base layer, so the base's frames count too. The layer below keeps
   its own pixels whole.
3. Picks the frames to bake. If any of the layers has an animated transform (2 or more
   transform keys), every frame. Otherwise only frames where one of them has an exposure,
   frames of transform keys, and frame 0; sorted with duplicates removed.
4. Measures how big the baked drawings must be. Artwork may sit outside the document rect
   (off camera), so it maps each cell's four corners into document space on every bake frame
   and keeps the largest distance from the document centre. The size is at least the document
   size and at most the GPU texture limit.
5. For each bake frame, makes a blank canvas of that size and composites the lower layer, then
   the top one (clipped if needed), each with its transform and opacity
   (`io::composite::composite_layer`).
6. In one `structural_edit`: clears the lower layer's exposures, transform, keys and opacity,
   sets its cell size to the bake size (so later blank keys match), keys the baked drawings,
   removes the top layer (fixing up layer links with `relink_after_remove`) and makes the
   merged layer active.

**Used by:** the Layers panel's Merge Down button in [ui/shell.rs](ui/shell.md).

### `AppState::merge_group`
[src/app.rs:2875](../../src/app.rs#L2875)

```rust
pub fn merge_group(&mut self, g: usize)
```

**What it does.** Flattens layer group `g` into one drawing layer that looks the same: on every
frame where any of its layers changes, the drawing is the group's composite, and the group's
opacity and visibility carry over to the new layer. Hidden and reference layers are not part of
the picture, so they are not merged away: they stay, as ordinary layers (out of the group) just
below the merged one. One undo step.

**Parameters.**
- `g`: the index of the group's header row in `project.layers`.

**How it works.**
1. Returns if `g` is not a group or is locked; stops playback.
2. Splits the group's children (`children(g)`, a range of layer indices) into `drawn`
   (visible, not reference) and `kept`.
3. Chooses frames the same way as `merge_layer_down` (all frames if any drawn layer's transform
   is animated; otherwise key frames, transform-key frames and frame 0).
4. Finds one canvas size big enough for every frame (`doc::clip::group_space`), capped at the
   texture limit.
5. Renders each frame with `doc::clip::render_group` on a copy of the project in which the
   group is forced visible, so a hidden group merges into a hidden layer rather than an empty
   one.
6. In one `structural_edit`: builds the merged `Layer` (name, opacity, visibility from the
   header), removes the group's rows (top-down), inserts the kept layers with `in_group = false`
   and then the merged layer, and makes it active.
7. Clears the panel's multi-layer pick, any rename in progress and the expand-canvas fields,
   since layer indices changed.

**Used by:** the Layers panel's row menu (`RowCmd::MergeGroup`) in [ui/shell.rs](ui/shell.md).

### `AppState::fit_transform`
[src/app.rs:2970](../../src/app.rs#L2970)

```rust
fn fit_transform(&self, cw: f32, ch: f32) -> Transform
```

**What it does.** The layer transform that shows a `cw`×`ch` picture centred on the canvas and
shrunk to fit inside it, but never enlarged: `scale = min(canvas_w / cw, canvas_h / ch, 1.0)`,
no offset, no rotation. The user can scale up from there to crop.

**Used by:** `import_cells_as_layer`, `paste_image_as_background`.

### `AppState::import_image`
[src/app.rs:2983](../../src/app.rs#L2983)

```rust
pub fn import_image(&mut self)
```

**What it does.** File → Import image…: opens a file dialog (`io::image_import::pick`), and puts
the chosen still image on a new layer below the active one, shown on every frame. Cancelling
does nothing; a decoding error is logged.

**Used by:** the title-bar File menu in [ui/shell.rs](ui/shell.md).

### `AppState::paste_image_as_background`
[src/app.rs:2996](../../src/app.rs#L2996)

```rust
pub fn paste_image_as_background(&mut self)
```

**What it does.** Edit → Paste image (Ctrl+V): reads an image from the system clipboard
(`io::image_import::from_clipboard`) and puts it on a new layer at the very bottom of the stack
(a background, `add_background_layer`), shown on every frame. If the clipboard holds no image it
only writes a log line. Like `import_cells_as_layer`, it caps the picture to the texture limit
and fits it with `fit_transform`, in one undoable edit.

**Used by:** `dispatch` (`Action::PasteImage`, this file); the title-bar Edit menu in
[ui/shell.rs](ui/shell.md).

### `AppState::copy_frame`
[src/app.rs:3023](../../src/app.rs#L3023)

```rust
pub fn copy_frame(&mut self)
```

**What it does.** File → Copy frame to clipboard: flattens the current frame as the screen
shows it (`clipboard_frame`) and puts it on the system clipboard
(`io::png_save::copy_to_clipboard`) for another app to paste. On success it shows a short
"Copied frame N" toast (`save_toast`, for `TOAST_TTL`); on failure it logs the error and shows
a warning message through `krita_notice`.

**Used by:** `dispatch` (`Action::CopyFrame`, this file); the title-bar File menu in
[ui/shell.rs](ui/shell.md).

### `AppState::clipboard_frame`
[src/app.rs:3041](../../src/app.rs#L3041)

```rust
fn clipboard_frame(&self) -> Canvas
```

**What it does.** The picture Copy frame copies: the current frame flattened through the camera
the guide is drawn with (`display_camera`), which can be a nudge not keyed yet, or where a
stretched camera move has got to during playback. Save PNG and the exports use the keyed camera
instead. Uses `io::composite::flatten_through`.

**Returns.** A new `Canvas` the size of the camera's frame.

**Used by:** `copy_frame` (and tests).

### `AppState::open_video_import`
[src/app.rs:3049](../../src/app.rs#L3049)

```rust
pub fn open_video_import(&mut self)
```

**What it does.** File → Import video…: asks for a video file (mp4, mov, mkv, avi, webm, m4v)
with a native file dialog (`rfd::FileDialog`), then starts a worker thread that probes its
frame count and fps (`io::video_import::probe`). It stores the receiving end as
`BgJob::VideoProbe` and sets the busy overlay label "Reading video…". The frame-range dialog
opens once `poll_bg_jobs` sees the probe result.

**Used by:** the title-bar File menu in [ui/shell.rs](ui/shell.md).

### `AppState::open_gif_import`
[src/app.rs:3068](../../src/app.rs#L3068)

```rust
pub fn open_gif_import(&mut self)
```

**What it does.** File → Import GIF…: `open_frames_import` with the GIF decoder
(`io::gif_import::decode_all`), which decodes every frame on a worker thread before the range
dialog opens.

**Used by:** the title-bar File menu in [ui/shell.rs](ui/shell.md).

### `AppState::open_webp_import`
[src/app.rs:3074](../../src/app.rs#L3074)

```rust
pub fn open_webp_import(&mut self)
```

**What it does.** File → Import WebP…: the same as `open_gif_import` with the WebP decoder
(`io::webp_import::decode_all`). A still WebP comes in as one frame.

**Used by:** the title-bar File menu in [ui/shell.rs](ui/shell.md).

### `AppState::open_frames_import`
[src/app.rs:3078](../../src/app.rs#L3078)

```rust
fn open_frames_import(
    &mut self,
    format: &'static str,
    ext: &'static str,
    busy: &'static str,
    decode: fn(&Path) -> Result<Vec<Canvas>>,
)
```

**What it does.** Shared code for the animated-image imports: asks for a file with extension
`ext`, decodes it with `decode` on a worker thread, and stores the job as
`BgJob::FramesDecode` with the busy label `busy`. Cancelling the dialog does nothing.

**Parameters.**
- `format`: a display name ("GIF", "WebP") for the dialog title and filter, and later the layer
  name.
- `ext`: the file extension to filter on.
- `busy`: the text of the busy overlay while decoding.
- `decode`: the decoder function; `fn(...)` is a plain function pointer.

`&'static str` is a string literal that lives for the whole program, so it can be stored in the
job without copying. See [Lifetimes](../rust-primer.md#lifetimes).

**Used by:** `open_gif_import`, `open_webp_import`.

### `AppState::start_export`
[src/app.rs:3107](../../src/app.rs#L3107)

```rust
pub fn start_export(&mut self)
```

**What it does.** Runs the export the export dialog is set up for, whatever the format (PNG
sequence, GIF, WebP, MP4, sprite sheet). Asks for a destination, then writes the file on a
worker thread: flattening a long range is far too slow for the UI thread, and one path means
one place where success and failure reach the user. Called when the export dialog is confirmed.

**How it works.**
1. Opens the right native dialog for `export_cfg.kind`: a folder picker for a PNG sequence, or
   a save dialog with a filter and default name (`animation.gif`, `animation.webp`,
   `animation.mp4`, `sheet.png`). Returns if cancelled.
2. Clamps the start/end frames to the timeline, and gathers the sprite-sheet options, MP4
   settings (CRF quality and libx264 preset from `MP4_PRESETS`) and the WebP play count
   (0 = repeat forever).
3. Gets the frame order, and the per-frame cameras when the camera move is stretched over
   loops, from `export_order`.
4. Clones the whole `Project` (so the worker has its own copy) and spawns a thread that calls
   the matching exporter: `io::png_seq`, `io::gif_export`, `io::webp_export`,
   `io::mp4_export` or `io::sprite_sheet`. The result (the file name on success) goes back over
   a channel.
5. Stores `BgJob::Export(rx)` and the busy label from `ExportKind::busy`; `poll_bg_jobs` picks up
   the result.

**Used by:** the export dialog in [ui/shell.rs](ui/shell.md).

### `AppState::confirm_import_range`
[src/app.rs:3189](../../src/app.rs#L3189)

```rust
pub fn confirm_import_range(&mut self)
```

**What it does.** The frame-range import dialog's OK button: imports the chosen `[start, end]`
frames onto a new layer. For a GIF or WebP the frames are already decoded, so it slices them and
calls `import_cells_as_layer` at once. For a video it starts a worker thread that extracts those
frames with ffmpeg (`io::video_import::extract_frames`), stored as `BgJob::VideoExtract`.

**How it works.**
1. Takes the dialog state out of `import_range` (returns if there is none) and closes the
   dialog.
2. Sets `preview_clear_pending` instead of freeing the dialog's preview thumbnails now: this
   runs inside the egui pass, after the dialog already drew them this frame, and freeing a
   texture still in use crashes wgpu. They are cleared at the top of the next frame.
3. Orders start/end so start ≤ end, then imports as above.

**Used by:** the import range dialog in [ui/shell.rs](ui/shell.md).

### `AppState::cancel_import_range`
[src/app.rs:3224](../../src/app.rs#L3224)

```rust
pub fn cancel_import_range(&mut self)
```

**What it does.** Closes the frame-range dialog without importing: drops its state and any
preview in progress, and (as in `confirm_import_range`) defers freeing the preview textures
to the next frame.

**Used by:** the import range dialog in [ui/shell.rs](ui/shell.md).

### `AppState::display_transform`
[src/app.rs:3238](../../src/app.rs#L3238)

```rust
pub fn display_transform(&self, layer_idx: usize, frame: usize) -> Transform
```

The position/scale/rotation shown for a layer at a frame. For the active layer on the current
frame it returns the live edit buffer (so a drag shows immediately); everywhere else it computes
the value from the layer's transform keys.

### `AppState::sync_active_transform_buffer`
[src/app.rs:3255](../../src/app.rs#L3255)

```rust
fn sync_active_transform_buffer(&mut self)
```

Keeps the active layer's editable transform in step with its keyframes when you change layer or
frame, so the Transform panel shows the right pose. It only reloads when the (layer, frame) pair
actually changed — otherwise it would overwrite edits being made — and skips layers without keys
(their buffer *is* the value) and drags in progress.

### `AppState::reset_active_layer_transform`
[src/app.rs:3273](../../src/app.rs#L3273)

```rust
pub fn reset_active_layer_transform(&mut self)
```

Resets the active layer's transform to "no move, no scale, no rotation", as one undoable step.

### `AppState::begin_layer_xform`
[src/app.rs:3286](../../src/app.rs#L3286)

```rust
pub fn begin_layer_xform(&mut self)
```

Takes a snapshot of the timeline when a layer-transform drag starts, so the whole drag can be
undone in one step (`commit_layer_xform` pushes it).

### `AppState::transform_group`
[src/app.rs:3306](../../src/app.rs#L3306)

```rust
fn transform_group(&mut self, f: impl Fn(Transform) -> Transform)
```

Applies a change of pose `f` to every layer in the active group. A layer whose transform has keys
gets every key changed the same way (or, with auto-key on, a new key on this frame). Layers locked
on their own stay put. `f: impl Fn(Transform) -> Transform` means "any function from a transform to
a transform".

### `AppState::about_group_pivot`
[src/app.rs:3335](../../src/app.rs#L3335)

```rust
fn about_group_pivot(&self, k: f32, rot: f32) -> impl Fn(Transform) -> Transform
```

Builds the function `transform_group` needs for scaling by `k` and rotating by `rot` around the
group's pivot. It *returns a closure* (`-> impl Fn(Transform) -> Transform`); the closure captures
copies of its inputs (`move`), so it can be passed on freely — like returning an arrow function
from a JS function.

### `AppState::commit_layer_xform`
[src/app.rs:3352](../../src/app.rs#L3352)

```rust
pub fn commit_layer_xform(&mut self)
```

Ends a layer-transform drag: compares the snapshot from `begin_layer_xform` with now and pushes a
single undo entry.

### `AppState::active_layer_locked`
[src/app.rs:3375](../../src/app.rs#L3375)

```rust
fn active_layer_locked(&self) -> bool
```

True if the active layer can't be edited: locked itself, locked through its group, or it is a group
header (which has no drawings).

### `AppState::apply_layer_pan`
[src/app.rs:3381](../../src/app.rs#L3381)

```rust
pub fn apply_layer_pan(&mut self, dx: f32, dy: f32)
```

Moves the active layer (or its whole group) by `(dx, dy)` document pixels, unless locked; with
auto-key on, writes a key.

### `AppState::apply_layer_scale`
[src/app.rs:3400](../../src/app.rs#L3400)

```rust
pub fn apply_layer_scale(&mut self, factor: f32)
```

Scales the active layer (or group) by `factor`, multiplicatively (1.1 = 10% bigger).

### `AppState::apply_layer_rotate`
[src/app.rs:3415](../../src/app.rs#L3415)

```rust
pub fn apply_layer_rotate(&mut self, d: f32)
```

Rotates the active layer (or group) by `d` radians.

### `AppState::add_transform_key`
[src/app.rs:3431](../../src/app.rs#L3431)

```rust
pub fn add_transform_key(&mut self)
```

Adds (or replaces) a transform key on the current frame from the active layer's current pose.
Undoable.

### `AppState::set_transform_key_ease`
[src/app.rs:3447](../../src/app.rs#L3447)

```rust
pub fn set_transform_key_ease(&mut self, ease: Ease)
```

Sets the easing (how motion speeds up/slows down into the key) of the active layer's key on the
current frame. Undoable.

### `AppState::delete_transform_key`
[src/app.rs:3460](../../src/app.rs#L3460)

```rust
pub fn delete_transform_key(&mut self)
```

Deletes the active layer's transform key on the current frame, if there is one. Undoable.

### `AppState::display_camera`
[src/app.rs:3487](../../src/app.rs#L3487)

```rust
pub fn display_camera(&self, frame: usize) -> Camera
```

The camera to draw for `frame`: the live edit buffer on the current frame, otherwise computed from
the camera keys. Mirrors `display_transform`.

### `AppState::advance_playback`
[src/app.rs:3500](../../src/app.rs#L3500)

```rust
fn advance_playback(&mut self, now: f64) -> bool
```

Called every frame from `update` with the current time. Moves the playhead forward according to
the frame rate and returns whether the displayed frame changed (so the caller requests a repaint).
A camera move "stretched over loops" keeps its own position meanwhile.

### `AppState::play_pause`
[src/app.rs:3521](../../src/app.rs#L3521)

```rust
pub fn play_pause(&mut self, now: Option<f64>)
```

The Play button / shortcut: starts playback, or stops it and returns to the frame playback started
on. `now` is the egui clock when a button supplies it; a shortcut passes `None`.

### `AppState::play_pause_here`
[src/app.rs:3536](../../src/app.rs#L3536)

```rust
pub fn play_pause_here(&mut self, now: Option<f64>)
```

Alternative play/pause: stopping leaves the playhead on the frame it reached, for catching a
drawing to fix mid-playback.

### `AppState::return_to_play_start`
[src/app.rs:3546](../../src/app.rs#L3546)

```rust
fn return_to_play_start(&mut self) -> bool
```

Puts the playhead back on the frame playback started from. Returns whether the frame changed.
`self.play_from.take()` moves the value out of the `Option` and leaves `None` behind.

### `AppState::follow_stretched_camera`
[src/app.rs:3561](../../src/app.rs#L3561)

```rust
fn follow_stretched_camera(&mut self, stretched: bool)
```

While a camera move that lasts several loops of the animation plays, records where the camera has
got to. When playback stops, moves the playhead to the frame the camera reached.

### `AppState::camera_stretched`
[src/app.rs:3572](../../src/app.rs#L3572)

```rust
fn camera_stretched(&self, kind: ExportKind) -> bool
```

Whether an export of this kind has a camera move that outlasts the drawings (the camera's move
spans more than one loop). Sprite sheets never do.

### `AppState::export_info`
[src/app.rs:3577](../../src/app.rs#L3577)

```rust
pub fn export_info(&self, kind: ExportKind) -> crate::io::frame_order::OrderInfo
```

Summarises what the export dialog's current settings will produce for `kind` (frame count, length,
whether the camera stretches) for the dialog to display. See [io/frame_order.rs](io/frame_order.md).

### `AppState::export_order`
[src/app.rs:3593](../../src/app.rs#L3593)

```rust
fn export_order(&self, kind: ExportKind, range: (usize, usize)) -> (Vec<usize>, Option<Vec<Camera>>)
```

Computes, for an export, which timeline frame each output frame shows and — when the camera move is
stretched over loops — which camera each output frame is seen through.

### `AppState::sync_camera_buffer`
[src/app.rs:3610](../../src/app.rs#L3610)

```rust
fn sync_camera_buffer(&mut self)
```

Keeps the camera's live edit buffer in step with its keys when the frame changes (same rules as
`sync_active_transform_buffer`: only on real changes, never mid-drag, not when there are no keys).

### `AppState::begin_camera_drag`
[src/app.rs:3632](../../src/app.rs#L3632)

```rust
pub fn begin_camera_drag(&mut self)
```

Snapshots the timeline when a camera drag starts, for a one-step undo.

### `AppState::commit_camera_drag`
[src/app.rs:3637](../../src/app.rs#L3637)

```rust
pub fn commit_camera_drag(&mut self)
```

Ends a camera drag: pushes one `Structural` undo command holding the before/after timeline states.

### `AppState::look_through_camera`
[src/app.rs:3652](../../src/app.rs#L3652)

```rust
pub fn look_through_camera(&self) -> Camera
```

The camera the view locks to in "look through camera" mode: the live camera, or — while its frame
is being dragged — the camera as it was when the drag began, so the view holds still instead of
running away from the cursor.

### `AppState::camera_frame_down`
[src/app.rs:3663](../../src/app.rs#L3663)

```rust
pub fn camera_frame_down(&mut self, p: (f32, f32)) -> bool
```

A press at document point `p` in camera-edit mode: grabs the camera's frame (inside = move, a
corner = zoom, just outside a corner = roll). Returns `false` and changes nothing if the mode is off
or the press missed, letting the active tool have the press.

### `AppState::camera_frame_move`
[src/app.rs:3683](../../src/app.rs#L3683)

```rust
pub fn camera_frame_move(&mut self, p: (f32, f32))
```

Continues a camera-frame drag to point `p`. Shift pressed part-way locks a move to horizontal or
vertical.

### `AppState::camera_frame_up`
[src/app.rs:3698](../../src/app.rs#L3698)

```rust
pub fn camera_frame_up(&mut self)
```

Releases the camera frame: one undo step for the whole drag, none if nothing moved.

### `AppState::set_camera_box`
[src/app.rs:3711](../../src/app.rs#L3711)

```rust
pub fn set_camera_box(&mut self, on: bool)
```

Turns Draw-camera mode on/off. Turning it on switches camera-edit mode off, because both want the
plain drag gesture.

### `AppState::camera_box_down`
[src/app.rs:3721](../../src/app.rs#L3721)

```rust
pub fn camera_box_down(&mut self, p: (f32, f32)) -> bool
```

A press in Draw-camera mode: starts dragging out a new camera box at `p`. Returns `false` when the
mode is off or playback is running (no still frame to key).

### `AppState::camera_box_move`
[src/app.rs:3735](../../src/app.rs#L3735)

```rust
pub fn camera_box_move(&mut self, p: (f32, f32))
```

Continues dragging the camera box to `p`; the move key slides the whole box instead.

### `AppState::camera_box_camera`
[src/app.rs:3751](../../src/app.rs#L3751)

```rust
pub fn camera_box_camera(&self) -> Option<Camera>
```

The camera the box being dragged would create (used for the preview and on release), or `None`
while it is still just a tap. `?` and `.map` on `Option` are used to pass the "nothing yet" case
through.

### `AppState::camera_box_frame`
[src/app.rs:3759](../../src/app.rs#L3759)

```rust
pub fn camera_box_frame(&self) -> Option<(Camera, (u32, u32))>
```

Computes the camera from the dragged box, plus the frame size it gives: the camera's own size, or
for a free-shaped box the box's size at zoom 1 (rounded to even pixels, at least
`MIN_FREE_CAMERA_SIDE`). With the centre key held the press point is the box's middle.

### `AppState::camera_box_up`
[src/app.rs:3811](../../src/app.rs#L3811)

```rust
pub fn camera_box_up(&mut self)
```

Releases the camera box and keys its camera on this frame. A tap keys the camera as it was, centred
where you tapped; a box dragged back to nothing keys nothing.

### `AppState::key_drawn_camera`
[src/app.rs:3836](../../src/app.rs#L3836)

```rust
fn key_drawn_camera(&mut self, cam: Camera)
```

Keys camera `cam` on this frame (replacing a key here, or adding one that eases in and out). If the
camera had no keys at all, it first keys the old camera on frame 0, so there is a move into the new
key. One undo step.

### `AppState::camera_ghost`
[src/app.rs:3858](../../src/app.rs#L3858)

```rust
pub fn camera_ghost(&self) -> Option<Camera>
```

For Draw-camera mode's faint "where it came from" frame: the camera at the last key before this
frame, if any. The `?` after `prev_camera_key()` returns `None` early when there is no earlier key.

### `AppState::canvas_size`
[src/app.rs:3863](../../src/app.rs#L3863)

```rust
fn canvas_size(&self) -> (f32, f32)
```

The project's canvas width and height as floats.

### `AppState::camera_frame_size`
[src/app.rs:3867](../../src/app.rs#L3867)

```rust
fn camera_frame_size(&self) -> (f32, f32)
```

The camera frame's (output) size as floats — the size exports come out at.

### `AppState::apply_camera_pan`
[src/app.rs:3874](../../src/app.rs#L3874)

```rust
pub fn apply_camera_pan(&mut self, dx: f32, dy: f32)
```

Moves the camera by `(dx, dy)` document pixels and keeps it inside the canvas if that option is on.

### `AppState::apply_camera_zoom`
[src/app.rs:3881](../../src/app.rs#L3881)

```rust
pub fn apply_camera_zoom(&mut self, factor: f32)
```

Zooms the camera by `factor` (multiplicative), clamped to the camera's zoom limits.

### `AppState::apply_camera_rotate`
[src/app.rs:3888](../../src/app.rs#L3888)

```rust
pub fn apply_camera_rotate(&mut self, d: f32)
```

Rolls the camera by `d` radians.

### `AppState::apply_scheme`
[src/app.rs:3894](../../src/app.rs#L3894)

```rust
pub fn apply_scheme(&self, ctx: &egui::Context)
```

Applies the chosen colour scheme (Graphite, Midnight, Paper or Custom) to egui; it takes effect
from the next frame. See [ui/theme.rs](ui/theme.md).

### `AppState::panel_shown`
[src/app.rs:3898](../../src/app.rs#L3898)

```rust
pub fn panel_shown(&self, id: PanelId) -> bool
```

**What it does.** Says whether the floating panel `id` is open: true unless it is in the
`hidden_panels` list. (The global Tab toggle, `show_panels`, is separate.)

**Used by:** [ui/shell.rs](ui/shell.md) (drawing each panel window, the Panels menu
checkboxes, docking neighbours).

### `AppState::set_panel_shown`
[src/app.rs:3902](../../src/app.rs#L3902)

```rust
pub fn set_panel_shown(&mut self, id: PanelId, shown: bool)
```

**What it does.** Opens or closes one panel. It first removes `id` from `hidden_panels`
(`retain` keeps only items that pass the test, like JS `filter` but in place), then pushes it
back if `shown` is false.

**Used by:** [ui/shell.rs](ui/shell.md) (the × in a panel's title bar and the Panels menu).

### `AppState::reset_panel_layout`
[src/app.rs:3912](../../src/app.rs#L3912)

```rust
pub fn reset_panel_layout(&mut self)
```

**What it does.** Shows every panel again at its original position and size. egui remembers a
window's place by its id, so instead of clearing egui's memory, this bumps
`panel_layout_gen`; the panels then get new ids and open as if for the first time.
`wrapping_add(1).max(1)` adds one (wrapping round at the type's maximum instead of crashing) and
never lands on 0, which is reserved for the original ids.

**Used by:** [ui/shell.rs](ui/shell.md) (Panels menu "Reset panel layout").

### `AppState::reset_camera`
[src/app.rs:3919](../../src/app.rs#L3919)

```rust
pub fn reset_camera(&mut self)
```

**What it does.** Puts the live camera back to the default (no pan, zoom 1, no roll), so the
camera frame sits on the document rectangle. Done through `structural_edit`, so it can be
undone. The camera is the rectangle that decides what an export shows (see
[glossary](../glossary.md)).

**Used by:** [ui/shell.rs](ui/shell.md) (camera panel Reset button).

### `AppState::set_camera_size`
[src/app.rs:3930](../../src/app.rs#L3930)

```rust
pub fn set_camera_size(&mut self, size: Option<(u32, u32)>)
```

**What it does.** Gives the camera its own frame size (the size exports come out at), or with
`None` hands it back the canvas size. Zoom and camera keys are not touched, so the frame simply
takes in more or less of the canvas. This is a session setting, **not** an undo step, so undoing
a stroke never changes the export size.

**Parameters.**
- `size` — the new width and height, or `None` for "same as the canvas".

**How it works.**
1. Clamps each side to `1..=MAX_CAMERA_SIDE` (16384).
2. If the clamped size equals the canvas size, stores `None` (the `.filter(...)` turns a
   matching `Some` into `None`).
3. Remembers a real own size in `last_camera_size`, which is kept between runs.
4. Calls `hold_camera_inside` so a camera set to stay on the canvas still does.

**Used by:** `AppState` itself (finishing a free-shape camera box), [ui/shell.rs](ui/shell.md)
(camera size fields).

### `AppState::camera_opts_changed`
[src/app.rs:3944](../../src/app.rs#L3944)

```rust
pub fn camera_opts_changed(&mut self)
```

**What it does.** Called when one of the camera's session options (the smooth path, staying on
the canvas) changes what the camera keys resolve to. It clears `cam_sync_last`, so the next
frame recomputes the live camera from the keys, exactly as a frame change would.

**Used by:** [ui/shell.rs](ui/shell.md) (camera panel option toggles).

### `AppState::hold_camera_inside`
[src/app.rs:3950](../../src/app.rs#L3950)

```rust
pub fn hold_camera_inside(&mut self)
```

**What it does.** If the project is set to keep the camera on the canvas, pulls the live camera
back inside; otherwise (or when it is already inside) nothing changes. The real work is
`Project::held_inside` in [doc/project.rs](doc/project.md).

**Used by:** the camera pan/zoom/rotate methods just before this stretch, `set_camera_size`, and
another camera path later in `AppState`.

### `AppState::frame_camera`
[src/app.rs:3955](../../src/app.rs#L3955)

```rust
pub fn frame_camera(&mut self, how: CameraFraming)
```

**What it does.** Points the live camera at the canvas in one of four ways, as one undo step.

**Parameters.**
- `how` — `Fit` (centred, zoomed out until the whole canvas fits), `Fill` (centred, zoomed in
  until only canvas shows), `OneToOne` (one canvas pixel per exported pixel, camera stays where
  it is) or `Centre` (keep zoom, re-centre).

**How it works.**
1. Inside `structural_edit`, copies the current camera and computes the canvas size and the
   output (export) size.
2. Sets the zoom with `fit_zoom` / `fill_zoom`, to 1.0, or leaves it.
3. Every option except `OneToOne` resets the pan (`tx`, `ty`) to zero, i.e. centres it.
4. Clamps zoom to `Camera::ZOOM_MIN..ZOOM_MAX` and stores the result through `held_inside`.

**Used by:** [ui/shell.rs](ui/shell.md) (camera panel framing buttons).

### `AppState::add_camera_key`
[src/app.rs:3977](../../src/app.rs#L3977)

```rust
pub fn add_camera_key(&mut self)
```

**What it does.** Writes (or overwrites) a camera key on the current frame from the live camera,
as an undo step. A *key* is a saved value on one frame; frames between keys are interpolated.

**Used by:** the `CameraKeyAdd` shortcut action in `AppState`, [ui/shell.rs](ui/shell.md)
(camera panel), tests.

### `AppState::delete_camera_key`
[src/app.rs:3986](../../src/app.rs#L3986)

```rust
pub fn delete_camera_key(&mut self)
```

**What it does.** Removes the camera key on the current frame, if there is one (otherwise it
returns without making an undo step). After deleting, it re-resolves the live camera from the
remaining keys so the canvas shows the right value.

**Used by:** the `CameraKeyDelete` shortcut action in `AppState`, [ui/shell.rs](ui/shell.md).

### `AppState::move_camera_key`
[src/app.rs:4000](../../src/app.rs#L4000)

```rust
pub fn move_camera_key(&mut self, from: usize, to: usize, copy: bool)
```

**What it does.** Moves the camera key on frame `from` to frame `to`, or copies it there when
`copy` is true. Does nothing if `from == to` or there is no key on `from`. One undo step; the
live camera is re-resolved afterwards. The key logic itself is `Project::move_camera_key` in
[doc/project.rs](doc/project.md).

**Used by:** [ui/tracks.rs](ui/tracks.md) (dragging a camera key along the timeline ruler).

### `AppState::set_camera_key_ease`
[src/app.rs:4012](../../src/app.rs#L4012)

```rust
pub fn set_camera_key_ease(&mut self, ease: Ease)
```

**What it does.** Sets the easing (how the motion speeds up or slows down towards the next key)
of the camera key on the current frame, as an undo step. If the frame has no key, the undo step
records no change.

**Used by:** [ui/shell.rs](ui/shell.md) (camera panel ease picker), tests.

### `AppState::active_layer_cell_size`
[src/app.rs:4023](../../src/app.rs#L4023)

```rust
pub fn active_layer_cell_size(&self) -> (u32, u32)
```

**What it does.** Returns the pixel size of the buffers the active layer draws into. A layer's
cells can be bigger than the canvas (see `expand_active_layer_canvas`). Falls back to the
project size if the layer index is out of range.

**Used by:** `expand_active_layer_canvas`, [ui/shell.rs](ui/shell.md) (Layers panel "expand
canvas" fields).

### `AppState::active_layer_cell_count`
[src/app.rs:4031](../../src/app.rs#L4031)

```rust
pub fn active_layer_cell_count(&self) -> usize
```

**What it does.** How many cells (drawings) the active layer uses. The Layers panel shows it
next to the size so the memory cost of a resize is visible.

**Used by:** [ui/shell.rs](ui/shell.md).

### `AppState::expand_active_layer_canvas`
[src/app.rs:4037](../../src/app.rs#L4037)

```rust
pub fn expand_active_layer_canvas(&mut self, w: u32, h: u32)
```

**What it does.** Grows (or resizes) the drawable buffer of every cell on the active layer to
`w × h`, so the artwork can go past what the camera sees — handy for pans. Undoable.

**Parameters.**
- `w`, `h` — the new cell size in pixels.

**How it works.**
1. Returns if the active layer is locked.
2. Lands any floating selection first (`land_float`), because its cell is about to be re-padded.
3. Clamps each side to `1..=max_tex` (the GPU's largest texture side; one cell is one texture).
4. Returns if the size would not change.
5. Keeps `Arc` handles to the old cell buffers (cheap shared pointers, not copies — see
   [Smart pointers](../rust-primer.md#smart-pointers)).
6. Calls `Project::expand_layer_canvas`, then pushes an `undo::Command::LayerCanvasResize`
   holding the old size, new size and old buffers.
7. Marks every cell on that layer dirty so its GPU texture is re-uploaded.

**Used by:** [ui/shell.rs](ui/shell.md) (Layers panel "expand canvas").

### `AppState::tracker_click`
[src/app.rs:4081](../../src/app.rs#L4081)

```rust
pub fn tracker_click(&mut self, doc: (f32, f32))
```

**What it does.** Records a stabilization tracking point on the active layer at the current
frame. The tracker is used on imported footage: you click the same feature on every frame, and
`stabilize_active_layer` later cancels out the shake. Clicks are not undo steps — you re-click a
frame to fix a miss.

**Parameters.**
- `doc` — the click position in document space (canvas pixels).

**How it works.**
1. Stops playback. Returns if the layer is locked or is a group header.
2. Grows the layer's `track_points` list to one entry per frame if needed.
3. **Single-point mode:** sets point A for this frame and moves to the next frame (no wrap).
4. **Two-point mode** (`tracker_two_points`): the first click on a frame sets A and clears B,
   and remembers the frame in `tracker_pending_b` without advancing; the second click on that
   frame sets B and then advances.

**Used by:** [ui/shell.rs](ui/shell.md) (canvas click while the tracker tool is on).

### `AppState::tracked_point_count`
[src/app.rs:4115](../../src/app.rs#L4115)

```rust
pub fn tracked_point_count(&self) -> usize
```

**What it does.** Counts the frames of the active layer that have a point A. Returns 0 for a bad
layer index.

**Used by:** `clear_track_points`, [ui/shell.rs](ui/shell.md) (tracker panel readout).

### `AppState::clear_track_points`
[src/app.rs:4124](../../src/app.rs#L4124)

```rust
pub fn clear_track_points(&mut self)
```

**What it does.** Removes every tracking point from the active layer, as an undo step, and
forgets any pending second click. Returns early (no undo step) when there are none.

**Used by:** [ui/shell.rs](ui/shell.md) (tracker panel Clear button).

### `AppState::stabilize_active_layer`
[src/app.rs:4141](../../src/app.rs#L4141)

```rust
pub fn stabilize_active_layer(&mut self)
```

**What it does.** Writes transform keys on the active layer so the tracked feature stays where
it was on the first tracked frame (the *reference*). Frames where both this frame and the
reference have point B also get rotation and zoom correction; frames with only A get a shift
only. The whole result is one undo step.

**How it works.**
1. Returns if the layer is locked; stops playback; clears any pending B click.
2. Collects `(frame, sample)` for every frame that has A. Needs at least two.
3. The first one is the reference `r` with point `ra`.
4. Resolves each tracked frame's existing transform **before** adding any key (adding keys as it
   went would change later results).
5. For each frame:
   - If both frames have B and the A→B distance on this frame is not near zero, computes the
     scale `sd` (reference length ÷ this length) and angle `theta` (difference of the two A→B
     angles).
   - With that two-point data, builds a rotation+scale about the canvas centre that maps this
     frame's A onto the reference A, and composes it with the frame's existing transform.
   - Otherwise adds the offset `ra - pa` to the existing translation and keeps scale/rotation.
6. Inside `structural_edit`, writes every key with `set_transform_key` and refreshes the live
   transform for the current frame.
7. Clears `xform_sync_last` so the transform edit fields reload.

**Example.** The translation-only case, as JS:
```js
// ra = reference point A, pa = this frame's point A, base = existing transform
const key = { ...base, tx: base.tx + (ra[0] - pa[0]), ty: base.ty + (ra[1] - pa[1]) };
```

**Used by:** [ui/shell.rs](ui/shell.md) (tracker panel Stabilize button).

### `AppState::poll_bg_jobs`
[src/app.rs:4238](../../src/app.rs#L4238)

```rust
fn poll_bg_jobs(&mut self)
```

**What it does.** Checks the one background job in `bg_job` (if any) and, when it has finished,
hands the result to the right handler and clears the busy overlay label (`bg_label`).

**How it works.**
1. `take()`s the job out of `bg_job` (leaving `None`); returns if there was none.
2. Matches on the kind of job and calls `try_recv` on its channel:
   - `FramesDecode` (GIF/WebP) → `on_frames_decoded`.
   - `VideoProbe` (count frames via ffmpeg) → `on_video_probed`.
   - `VideoExtract` → `on_video_extracted`.
   - `KraImport` → `on_kra_imported` (elsewhere in `AppState`).
   - `Export` → on success shows the "Exported …" toast in `save_toast`; on failure puts the
     message in `save_error` so a modal shows it (exports used to fail silently into the log).
3. A job still running (`Empty`) is put back into `bg_job`; a dead worker (`Disconnected`) is
   logged (and for exports also shown as an error) and dropped.

**Used by:** `AppState::update` (the per-frame `eframe::App` method later in this file).

### `AppState::on_frames_decoded`
[src/app.rs:4325](../../src/app.rs#L4325)

```rust
fn on_frames_decoded(&mut self, res: Result<Vec<Canvas>>, format: &'static str)
```

**What it does.** Handles decoded GIF/WebP frames: if there is at least one, clears old preview
textures and opens the frame-range import dialog (`import_range` + `show_import_range`) covering
all frames. An empty result or an error is only logged. `format` is the file kind's name for
messages (`&'static str` is a string literal baked into the program).

**Used by:** `poll_bg_jobs`.

### `AppState::on_video_probed`
[src/app.rs:4344](../../src/app.rs#L4344)

```rust
fn on_video_probed(&mut self, res: Result<(usize, f64)>, path: PathBuf)
```

**What it does.** Handles the result of probing a video (frame count and frames per second).
With frames, opens the frame-range dialog for that video; otherwise logs an error (the message
hints that ffmpeg may be missing).

**Used by:** `poll_bg_jobs`.

### `AppState::on_video_extracted`
[src/app.rs:4362](../../src/app.rs#L4362)

```rust
fn on_video_extracted(&mut self, res: Result<Vec<Canvas>>, name: String)
```

**What it does.** Turns the extracted video frames into a new layer with
`import_cells_as_layer`, or logs the error.

**Used by:** `poll_bg_jobs`.

### `AppState::request_preview`
[src/app.rs:4372](../../src/app.rs#L4372)

```rust
pub fn request_preview(&mut self, ctx: &egui::Context, idx: usize)
```

**What it does.** Makes sure the frame-range dialog has a thumbnail texture for source frame
`idx`. GIF/WebP thumbnails are made immediately from the decoded frames; video thumbnails are
extracted on a worker thread, one at a time.

**Parameters.**
- `ctx` — the egui context, needed to create textures.
- `idx` — which source frame to preview.

**How it works.**
1. A small local `enum Act` records the decision first, so the code does not keep a borrow of
   `self.import_range` while it then changes `self` (Rust forbids reading through one reference
   while writing through another — see [Ownership and borrowing](../rust-primer.md#ownership-and-borrowing)).
2. Nothing to do if the texture already exists or a video fetch is already running.
3. `Have(image)` → uploads it as texture `preview_{idx}`.
4. `SpawnVideo` → creates a channel, starts a thread that calls
   `io::video_import::extract_preview`, and stores the receiving end in `preview_rx`.

**Used by:** [ui/shell.rs](ui/shell.md) (import range dialog, for the start and end frames).

### `AppState::poll_preview`
[src/app.rs:4411](../../src/app.rs#L4411)

```rust
fn poll_preview(&mut self, ctx: &egui::Context)
```

**What it does.** Checks the in-flight video thumbnail fetch. When it arrives, premultiplies the
pixels and uploads the texture; an extraction error is logged as a warning; if it is still
running the receiver is put back.

**Used by:** `AppState::update`.

### `AppState::ensure_cell_tracking`
[src/app.rs:4429](../../src/app.rs#L4429)

```rust
fn ensure_cell_tracking(&mut self)
```

**What it does.** For every cell id in the project's pool, adds a `Dirty::Full` entry to
`cell_dirty` if it has none yet, so newly made cells are uploaded on the next sync.
`entry(id).or_insert(...)` is the map idiom "insert only if missing".

**Used by:** `sync_textures`, a test.

### `AppState::sync_textures`
[src/app.rs:4443](../../src/app.rs#L4443)

```rust
pub fn sync_textures(&mut self, ctx: &egui::Context)
```

**What it does.** Uploads whatever the canvas is about to draw and the GPU doesn't have yet:
each visible layer's cell on this frame, clipped-layer and group images, the floating
selection, the selection tint and the onion ghosts. Uploads are as small as the change. Runs
once per frame just before the UI is drawn.

**How it works.**
1. Frees textures parked in `retired_textures` last frame (safe now: the meshes that used them
   were submitted a frame ago), clears `fresh_full`, registers new cells, and bumps the
   `sync_frame` counter (`now`).
2. **During a stroke:** if the layer being painted is not clipped, not a clip base and not part
   of a merged group image, and its texture already exists, it sends only the stroke's newly
   flushed rect (`preview_upload_rect`) plus any older pending change, and returns. Otherwise it
   falls through to the full path.
3. **Full path:** walks every shown layer. Groups drawn whole go in `merged`; layers inside a
   group image are skipped; clipped layers become `ClipPair`s; every other layer adds the cell it
   shows on this frame to `needed`.
4. Calls `sync_group_images` and `sync_clip_textures`.
5. For each needed cell: marks it used now and calls `upload_cell` with its dirty state, then
   marks it clean.
6. Rebuilds the floating selection's texture (and each follower's) only if its pixels changed;
   retires it when there is no selection.
7. Finishes with `sync_tint`, `sync_ghosts` and `enforce_texture_budget`.

**Used by:** `AppState::update`; [ui/shell.rs](ui/shell.md) (headless UI tests); tests in
`app.rs` and [app/groups.rs](app/groups.md).

### `AppState::stream_stroke`
[src/app.rs:4562](../../src/app.rs#L4562)

```rust
pub fn stream_stroke(&mut self, ctx: &egui::Context) -> bool
```

Uploads the part of the live stroke painted since the last upload as a small patch of its drawing's
texture, *this frame*. The canvas calls it right after painting the frame's pen samples; egui
applies texture changes before drawing, so the ink appears under the pen without a one-frame lag.
Returns `false` (leaving it to the normal `sync_textures`) when it can't stream: when the layer is
shown through a clip or group texture instead of its own, or the drawing has no texture yet.

### `AppState::paints_through_clip`
[src/app.rs:4587](../../src/app.rs#L4587)

```rust
fn paints_through_clip(&self) -> bool
```

Whether the active layer is displayed through some texture other than its own drawing's: it is
clipped, something is clipped to it, or it is inside a group drawn as one image.

### `AppState::whole_tex`
[src/app.rs:4595](../../src/app.rs#L4595)

```rust
fn whole_tex(&self, id: CellId) -> bool
```

Whether drawing `id` already has a texture covering its whole cell (or can never be drawn on, in
which case it returns `true` so nobody waits for one). `is_some_and(|t| …)` is "the Option has a
value and the value passes this test".

### `AppState::pen_tex_ready`
[src/app.rs:4605](../../src/app.rs#L4605)

```rust
fn pen_tex_ready(&self, want: PenTex) -> bool
```

Whether the texture `warm_pre` asked for (see `PenTex`) has been requested and actually exists on
the GPU.

### `AppState::sync_pen_tex`
[src/app.rs:4624](../../src/app.rs#L4624)

```rust
fn sync_pen_tex(&mut self, ctx: &egui::Context)
```

While the pen is resting, prepares the texture the next stroke will need: the drawing under the pen
grown to a whole-cell texture, or a clear whole-cell texture for the new drawing a press would
create. Only one drawing is kept whole at a time — the previous one is cropped again — because a
whole-cell texture for every drawing blew the texture budget during playback.

### `AppState::adopt_spare_tex`
[src/app.rs:4683](../../src/app.rs#L4683)

```rust
fn adopt_spare_tex(&mut self, target: CellId)
```

A press just created a new blank drawing `target`: gives it the clear whole-cell texture prepared in
advance (`spare_tex`), so its very first stroke streams as patches instead of outgrowing a texture
sized to its first dab.

### `AppState::clip_pair`
[src/app.rs:4708](../../src/app.rs#L4708)

```rust
fn clip_pair(&self, li: usize, b: usize) -> Option<ClipPair>
```

**What it does.** Describes layer `li` clipped to base layer `b` on the current frame: which cell
is drawn, what it is clipped by, and where each sits. A *clipping mask* means the upper layer only
shows where the base layer has paint (see [glossary](../glossary.md)). Returns `None` when
nothing would show: base hidden, base is a reference layer, or either is empty on this frame.
The `?` after an `Option` returns `None` early (see [Option and Result](../rust-primer.md#option-and-result)).

**How it works.**
1. If the base is a **group**, the clip is by the group's merged mask: returns `None` unless some
   child of the group shows a drawing here, else a `ClipPair` keyed by the group image, with a
   default (identity) base transform.
2. Otherwise a `ClipPair` keyed by the base's cell on this frame, with both layers' display
   transforms.

**Used by:** `sync_textures`, `layer_texture`.

### `AppState::layer_texture`
[src/app.rs:4753](../../src/app.rs#L4753)

```rust
pub fn layer_texture(&self, li: usize) -> Option<(CellId, &TextureHandle, DirtyRect)>
```

**What it does.** The texture the canvas should draw for layer `li` this frame: for a clipped
layer the clipped texture, otherwise its cell's own. Also returns the cell id and the rect of the
cell the texture covers. `None` means draw nothing.

**Returns.** `(cell id, texture, covered rect)` or `None`.

**Used by:** [ui/shell.rs](ui/shell.md) (canvas drawing), many tests in `app.rs` and
[app/groups.rs](app/groups.md).

### `AppState::sync_clip_textures`
[src/app.rs:4773](../../src/app.rs#L4773)

```rust
fn sync_clip_textures(&mut self, ctx: &egui::Context, now: u64, pairs: &[ClipPair])
```

**What it does.** Brings each clipped drawing's texture up to date: one texture per
(clipped cell, base) pair showing the drawing cut to its base. Only changed areas are redone; if
the two moved relative to each other the whole thing is redone. Projects without clipping never
get here with any pairs.

**Parameters.**
- `now` — this sync's counter, recorded as "last used" for the eviction budget.
- `pairs` — the `ClipPair`s found by `sync_textures`.

**How it works.**
1. For each pair, looks up both canvases (a group base uses the group image's canvas).
2. Reuses the existing texture only if it was built for the same cell size and placement.
3. Asks `plan_upload` what to do. `Patch` re-sends one rect of `mask_rect(...)` (from
   [doc/clip.rs](doc/clip.md)) with `set_partial`; `Rebuild` makes a new texture and retires
   the old one.
4. Marks the texture clean and refreshes its `aligned` flag (whether clip and base still match
   pixel-for-pixel).

**Used by:** `sync_textures`.

### `AppState::upload_cell`
[src/app.rs:4830](../../src/app.rs#L4830)

```rust
fn upload_cell(&mut self, ctx: &egui::Context, id: CellId, dirty: Dirty)
```

**What it does.** Brings cell `id`'s texture up to date with the `dirty` part of its pixels.

**How it works.**
1. Returns if the cell no longer exists.
2. If this cell is under a live stroke, passes the stroke's starting bounds (`stroke_base`) so
   the texture can grow around the stroke with spare room.
3. Only a texture made for the current cell size counts as existing (expanding a layer canvas
   resizes cells under their textures).
4. `plan_upload` decides:
   - `Keep` — nothing.
   - `Patch(r)` — re-sends rect `r` into the texture with `set_partial`.
   - `Rebuild(rect)` — if the existing texture is the same size, refills it in place with `set`
     and notes it in `fresh_full` (so this frame draws it as a plain mesh, because egui swaps the
     new image in only after the frame); otherwise creates a new texture and parks the old one in
     `retired_textures` so it is freed next frame, not mid-frame.

**Used by:** `sync_textures`.

### `AppState::sync_ghosts`
[src/app.rs:4883](../../src/app.rs#L4883)

```rust
fn sync_ghosts(&mut self, ctx: &egui::Context, now: u64)
```

**What it does.** Builds the coloured silhouette textures for onion-skin ghosts. *Onion skin*
shows neighbouring drawings faintly behind the current one (see [glossary](../glossary.md)).
Ghosts are built small (longest side `GHOST_MAX_SIDE` = 2048) and kept after they leave the
range, within the texture budget, so stepping back and forth reuses them. A ghost not built yet
waits until the playhead has rested `GHOST_REST` (0.12 s), so scrubbing past a hundred frames does
not build a hundred ghosts.

**How it works.**
1. Collects `(cell, tint)` for every onion layer's previous and next steps, its pinned ghosts,
   and the drawing ahead (`ghost_ahead`).
2. Keeps only the first tint per cell (`retain` with a `HashSet`) so a drawing asked for in two
   colours doesn't rebuild every frame.
3. Tracks how long the playhead has been on this frame (`playhead_rest`).
4. For each ghost: skips it if its texture is current (same tint, same size, not stale). If it
   has no texture and the playhead hasn't rested long enough, asks egui to repaint later and
   skips it.
5. Otherwise builds `ghost_image` and either refills the existing texture (same size) or
   retires it and makes a new one; clears its stale mark.

**Used by:** `sync_textures`.

### `AppState::enforce_texture_budget`
[src/app.rs:4951](../../src/app.rs#L4951)

```rust
fn enforce_texture_budget(&mut self)
```

**What it does.** Drops the least recently shown cell, ghost, clip and group textures until
their total size fits `TEXTURE_BUDGET` (1.5 GiB). Nothing shown in this sync is dropped. A dropped
cell re-uploads in full next time it shows.

**How it works.**
1. Builds a list of `TexEntry` (kind, id, last-used sync, bytes = width × height × 4).
2. Asks the free function `evictions` which to drop (it handles the playback special case of
   dropping the newest instead of the oldest).
3. For each victim, removes it from its map, parks its texture in `retired_textures`, and for a
   cell sets its dirty state back to `Full`.

**Used by:** `sync_textures`.

### `AppState::onion_layers`
[src/app.rs:5013](../../src/app.rs#L5013)

```rust
pub fn onion_layers(&self) -> Vec<usize>
```

**What it does.** Lists the layers (bottom first) whose onion ghosts should show: the active layer
plus every other non-group, shown layer with its own onion switch on. Returns an empty list while
playing (playback already shows the motion, and it keeps ghost rebuilding out of the playback
loop) or when onion skin is off.

**Used by:** `sync_ghosts`, [ui/shell.rs](ui/shell.md) (canvas ghost drawing).

### `AppState::onion_steps`
[src/app.rs:5031](../../src/app.rs#L5031)

```rust
pub fn onion_steps(&self, li: usize, dir: OnionDirection) -> Vec<OnionStep>
```

**What it does.** The onion ghosts for layer `li` at the current frame in one direction
(`Prev` or `Next`). Both the texture sync and the canvas use this, so they agree. Empty while
playing. Drawings that are already pinned ghosts are removed, because each cell has only one
cached tint.

**Used by:** `sync_ghosts`, [ui/shell.rs](ui/shell.md).

### `AppState::onion_pin_ghosts`
[src/app.rs:5054](../../src/app.rs#L5054)

```rust
pub fn onion_pin_ghosts(&self, li: usize) -> Vec<(OnionPin, CellId)>
```

**What it does.** Pinned onion ghosts for layer `li` (a drawing pinned to always show as a ghost,
with its own tint), from `timeline::onion::pin_ghosts` in
[timeline/onion.rs](timeline/onion.md). Empty while playing or with onion skin off.

**Used by:** `sync_ghosts`, `onion_steps`, [ui/shell.rs](ui/shell.md).

### `AppState::layer_view_alpha`
[src/app.rs:5077](../../src/app.rs#L5077)

```rust
pub fn layer_view_alpha(&self, li: usize) -> f32
```

**What it does.** The opacity multiplier for layer `li` on the canvas under "Fade other layers":
`fade.above` for layers above the active one, `fade.below` for those under it (clamped to 0..1).

**Returns.** 1.0 when the fade is off, during playback, for the active layer, for reference
layers, and for the active layer's `lines_from` layer (the line-art layer a fill layer reads its
boundaries from, which must stay readable).

**Used by:** [ui/shell.rs](ui/shell.md) (canvas drawing).

### `AppState::begin_edit_cell`
[src/app.rs:5095](../../src/app.rs#L5095)

```rust
fn begin_edit_cell(&mut self) -> Option<CellId>
```

**What it does.** Returns the cell a paint edit on the active layer and frame should write into,
creating a blank one if the slot is empty. `None` on a layer that cannot be painted (locked,
reference). If auto-key had to add a key first, that key's undo step is pushed now.

**Used by:** `pointer_down`, [app/lasso_fill.rs](app/lasso_fill.md).

### `AppState::edit_cell_on`
[src/app.rs:5105](../../src/app.rs#L5105)

```rust
fn edit_cell_on(&mut self, li: usize) -> Option<(CellId, Option<undo::Command>)>
```

**What it does.** The same as `begin_edit_cell` for any layer `li`, but the undo step of an
auto-added key is handed back instead of recorded, so the caller can fold it into a bigger step.

**How it works.**
1. Returns `None` if `Project::paintable(li)` says no.
2. With **auto-key draw** on (`auto_key_draw`), if the current frame is *held* (it shows a drawing
   but has no key of its own — an x-sheet *hold* repeats the previous drawing), it first gives the
   frame a new **blank** cell as its own key, via `structural_command`. Blank rather than a copy,
   so the next drawing starts clean; onion skin shows the previous one.
3. Returns `ensure_cell(li, f)` (the cell on that frame, created if missing) and the optional
   undo command.

**Returns.** `(cell id, Some(undo step) if a key was added)`, or `None`.

**Used by:** `begin_edit_cell`, [app/select.rs](app/select.md) (landing selections on picked
layers).

### `AppState::pointer_down`
[src/app.rs:5133](../../src/app.rs#L5133)

```rust
pub fn pointer_down(&mut self, sample: PointerSample)
```

**What it does.** Starts a paint edit where the pen (or mouse) went down: a Liquify warp, a fill,
a shape, or a freehand stroke, depending on the active tool.

**Parameters.**
- `sample` — the first pointer sample, in the cell's pixel space.

**How it works.**
1. **Liquify** tool: hands off to `liquify_down` (in [app/liquify.rs](app/liquify.md)) and
   returns.
2. Stops playback and lands any floating selection (a stroke must not paint under pixels about
   to land).
3. Gets the target cell from `begin_edit_cell`; returns if there is none. Stores it in
   `stroke_target`.
4. Takes the before-edit snapshot (`snapshot_pre`), copies its inked bounds to `stroke_base`, and
   clears the cell's own `dirty` rect so this edit's changes are tracked fresh.
5. **Fill** tool: builds `FillOptions` from the brush (tolerance, colour, expand, gap, alpha lock,
   opacity, paper grain), finds the boundary source layer and selection clip, and creates a
   `FillSession` seeded at the rounded pointer position. Walls are read from the **snapshot**, not
   the live cell, so re-running the fill mid-drag sees the lines as they were at the press. Applies
   it once, marks the cell dirty, and stores a `FillDrag`; the undo step is recorded on pen-up.
   See [tools/fill.rs](tools/fill.md).
6. **Shape** tool: stores a `ShapeDrag` with start = end = this point (and the perspective plane
   from `shape_plane`). Nothing is painted until pen-up; the UI draws a preview.
7. **Freehand:** prepares the stroke workspace (`stroke_ws.begin`, selection clip, alpha lock),
   fixes the brush radius and view scale for the whole stroke, builds a `StrokeBuilder` with the
   tool, smoothing and any Krita brush, sets its screen orientation, starts the stroke clock,
   pushes the first sample and calls `flush_stroke`. See [tools/stroke.rs](tools/stroke.md).

**Used by:** [ui/shell.rs](ui/shell.md) (canvas input), and many tests (`app.rs`,
[tools/golden_tests.rs](tools/golden_tests.md), `app/*` test modules).

### `AppState::flush_stroke`
[src/app.rs:5254](../../src/app.rs#L5254)

```rust
fn flush_stroke(&mut self, target: CellId)
```

**What it does.** Paints whatever part of the live stroke the builder has settled so far into
cell `target`, and marks only that rect for upload (marking the whole stroke rect would re-send
everything already streamed).

**How it works.**
1. Needs three things at once — the builder, the live snapshot and a writable cell — via a tuple
   `let (Some(a), Some(b), Some(c)) = (...) else { return; }`. `Arc::make_mut` gets a writable
   cell, copying it first if another holder shares it (copy-on-write).
2. `builder.flush(...)` returns the rect it changed, if any. That rect is added to
   `preview_upload_rect` and marked with `mark_region`.
3. Unless the tool is the Eraser, `painted` records that the cell is not blank (saves a full
   blank-scan every frame).

**Used by:** `pointer_down`, `pointer_move`, `tick_stroke` (earlier in `AppState`).

### `AppState::pointer_move`
[src/app.rs:5273](../../src/app.rs#L5273)

```rust
pub fn pointer_move(&mut self, sample: PointerSample)
```

**What it does.** Feeds one more pointer sample into the edit in progress.

**How it works.**
1. Liquify → `liquify_move`.
2. No `stroke_target` → nothing to do.
3. A Shape drag just moves its end point.
4. A freehand stroke pushes the sample, advances the builder's clock (milliseconds since
   pen-down, used by stabilized strokes) and calls `flush_stroke`.

**Used by:** [ui/shell.rs](ui/shell.md), tests.

### `AppState::pointer_up`
[src/app.rs:5295](../../src/app.rs#L5295)

```rust
pub fn pointer_up(&mut self)
```

**What it does.** Ends the edit in progress and records it as one undo step.

**How it works.**
1. Ends any perspective-grid drag and clears the per-stroke snap lock and Shift axis lock state.
2. Liquify session → `liquify_up` and return (the warp continues until it is landed).
3. A fill press → `finish_fill` and return.
4. No target → clears stroke, shape and snapshot state and returns.
5. **Shape:** builds the outline (`shape_outline`), prepares the workspace like a stroke and
   rasterizes it with `tools::shape::rasterize` (see [tools/shape.rs](tools/shape.md)), then marks
   the cell dirty.
6. **Freehand:** advances the clock a last time, calls `builder.finish(...)` and marks only the
   last rect (the rest was already uploaded while drawing).
7. If something was drawn, `remember_size` records the brush size in the size strip.
8. `commit_undo` makes the undo step; if it changed pixels and the tool paints (not the Eraser),
   `painted` marks the cell non-blank.

**Used by:** [ui/shell.rs](ui/shell.md), tests.

### `AppState::fill_anchor`
[src/app.rs:5371](../../src/app.rs#L5371)

```rust
pub fn fill_anchor(&mut self, at: egui::Pos2)
```

**What it does.** Remembers the screen point where a Fill press landed; dragging away from it
adjusts the fill (see `fill_drag_to`).

**Used by:** [ui/shell.rs](ui/shell.md), tests.

### `AppState::fill_drag_to`
[src/app.rs:5386](../../src/app.rs#L5386)

```rust
pub fn fill_drag_to(&mut self, at: egui::Pos2)
```

**What it does.** While the pen is still down on a Fill press, dragging sideways changes the
**gap** (how big a hole in the line art the fill closes) and dragging up/down changes the
**expand** (how many pixels the fill grows under the lines). Each change re-runs the fill so the
canvas follows, and the new values stay in the sliders.

**Parameters.**
- `at` — the pointer's current screen position.

**How it works.**
1. Returns without an anchor. `lock_screen_drag` applies a Shift axis lock if Shift was pressed
   partway (holding the drag to one of the two values).
2. Gap = `step_gap(base_gap, drag_steps(dx))` — a ladder whose steps spread out as it grows, so the
   whole range fits in one drag. Expand = base + steps of `-dy` (screen y grows downward, so up
   means more), clamped to `0..=MAX_EXPAND`.
3. Returns if neither changed. Otherwise stores them in the brush, clears the cell's `dirty`,
   re-applies the session from the snapshot, and marks the cell dirty (only this step's pixels are
   uploaded; pen-up widens the dirty rect again for the undo record).

**Used by:** [ui/shell.rs](ui/shell.md), tests.

### `AppState::active_alpha_lock`
[src/app.rs:5419](../../src/app.rs#L5419)

```rust
pub fn active_alpha_lock(&self) -> bool
```

**What it does.** Whether the active layer has *alpha lock* on (paint only changes pixels that
already have paint; see [glossary](../glossary.md)). `is_some_and` is "the `Option` has a value
and the test passes".

**Used by:** `pointer_down`, `pointer_up`, [app/liquify.rs](app/liquify.md),
[app/lasso_fill.rs](app/lasso_fill.md).

### `AppState::fill_dragging`
[src/app.rs:5427](../../src/app.rs#L5427)

```rust
pub fn fill_dragging(&self) -> bool
```

**What it does.** True while a Fill press is still under the pen.

**Used by:** [ui/shell.rs](ui/shell.md) (routing drags to `fill_drag_to`), tests.

### `AppState::finish_fill`
[src/app.rs:5433](../../src/app.rs#L5433)

```rust
fn finish_fill(&mut self) -> bool
```

**What it does.** Lands a Fill press as one undo step covering every pixel any step of its drag
touched (`session.touched()` becomes the cell's dirty rect). Marks the cell painted if the undo
step changed anything.

**Returns.** `true` if there was a fill press to finish (even one with no target).

**Used by:** `pointer_up`.

### `AppState::cancel_fill`
[src/app.rs:5453](../../src/app.rs#L5453)

```rust
pub fn cancel_fill(&mut self) -> bool
```

**What it does.** Abandons a Fill press still under the pen: the session's `revert` puts the cell
back from the snapshot, and no undo step is left.

**Returns.** `true` if there was a fill press.

**Used by:** `begin_screen_pick`, `handle_keys` (later in `AppState`: Esc backs out of a Fill
press first), tests.

### `AppState::snapshot_pre`
[src/app.rs:5479](../../src/app.rs#L5479)

```rust
fn snapshot_pre(&mut self, cell: CellId)
```

**What it does.** Takes the before-edit snapshot of `cell` into `stroke_pre_pixels`, for the
stroke compositor and for undo, and marks it live (`stroke_pre_live`).

**How it works.**
1. Sets `stroke_pre_live` and `pre_cell`, and takes `pre_mirror` out.
2. If the cell is gone, empties the buffer and returns.
3. If the buffer's length or row width doesn't match the cell, allocates a fresh zeroed buffer
   (fresh zeroed memory is cheap until written).
4. Else, if the buffer already mirrors this exact cell revision, returns — nothing to copy.
5. Else clears the old ink (`clear_pre`).
6. Copies only the rows the cell actually holds (`held_rect`) from the cell (read row by row,
   since the cell may be stored packed) into the buffer, in parallel, and records that rect in
   `pre_inked`.

**Used by:** `pointer_down`, `warm_pre`, [app/select.rs](app/select.md),
[app/liquify.rs](app/liquify.md), [app/lasso_fill.rs](app/lasso_fill.md).

### `AppState::clear_pre`
[src/app.rs:5513](../../src/app.rs#L5513)

```rust
fn clear_pre(&mut self)
```

**What it does.** Zeroes the part of `stroke_pre_pixels` that may hold ink (`pre_inked`), in
parallel, and forgets the mirror. After this the buffer is all zeros, which is the correct
snapshot of a blank cell.

**Used by:** `snapshot_pre`, `warm_pre`.

### `AppState::pen_cell`
[src/app.rs:5530](../../src/app.rs#L5530)

```rust
fn pen_cell(&self) -> Option<Option<CellId>>
```

**What it does.** Where the pen would paint if it went down now.

**Returns.** A nested `Option` with three meanings:
- `Some(Some(id))` — on existing drawing `id`;
- `Some(None)` — on a new blank drawing (auto-key would start one, or the slot is empty);
- `None` — nowhere (layer can't be painted).

**Example.**
```js
// JS-style equivalent of the three outcomes
// { kind: "cell", id } | { kind: "newBlank" } | null
```

**Used by:** `warm_pre`.

### `AppState::held_for_auto_key`
[src/app.rs:5545](../../src/app.rs#L5545)

```rust
fn held_for_auto_key(&self) -> Option<CellId>
```

**What it does.** With auto-key draw on and the current frame only *holding* a drawing (no key of
its own), returns that held drawing — the one that becomes "the drawing before" once the pen
starts a new one here. `None` otherwise.

**Used by:** `pen_cell`, `ghost_ahead`.

### `AppState::ghost_ahead`
[src/app.rs:5560](../../src/app.rs#L5560)

```rust
fn ghost_ahead(&self) -> Option<(CellId, [u8; 3])>
```

**What it does.** The onion ghost the next stroke would bring on: if auto-key is about to start a
new drawing here, the held drawing will become the previous one and be ghosted in the "previous"
tint. Returning it lets `sync_ghosts` build that ghost while the playhead rests, so the first
frame of the stroke doesn't pay tens of milliseconds for it. Only when onion skin is on, the
first previous step is shown, and nothing is playing.

**Used by:** `sync_ghosts`.

### `AppState::warm_pre`
[src/app.rs:5576](../../src/app.rs#L5576)

```rust
fn warm_pre(&mut self, ctx: &egui::Context)
```

**What it does.** Gets the drawing the pen would land on ready to paint **before** the pen goes
down, once it has been the target for `WARM_REST` (0.12 s), so pen-down doesn't stall (30–60 ms
on an 8K drawing). It takes the snapshot, unpacks the cell (cells at rest are stored packed to
save memory — see [doc/canvas.rs](doc/canvas.md)), and clears the stroke workspace's leftover
coverage. For a new blank drawing it just clears the snapshot.

**How it works.**
1. Only for painting tools (every pen slot, Eraser, Fill, Shape, Liquify). Skipped ("busy") while a
   snapshot is in use, while playing, or while a selection floats.
2. The target is `pen_cell()`, except Liquify, which always warps the drawing that is there.
3. If already ready (workspace clear, and the cell unpacked and mirrored — or for a blank target,
   the buffer already zero), returns.
4. Times how long this target has been waited on (`warm_wait`); if not yet `WARM_REST`, asks egui
   to repaint when it will be and returns.
5. Otherwise snapshots (then sets `stroke_pre_live` back to false: no edit is in hand yet),
   unpacks the cell and records the mirror; or clears the buffer for a blank target.

**Used by:** `AppState::update` (each frame, before `sync_textures`), tests.

### `AppState::pre_replaced`
[src/app.rs:5657](../../src/app.rs#L5657)

```rust
pub(crate) fn pre_replaced(&mut self, cell: CellId)
```

**What it does.** Called when `stroke_pre_pixels` has been swapped for a different snapshot of
`cell` (the cell as it was before a selection lifted pixels). Since nothing is known about its
contents, it forgets the mirror and marks the whole cell as possibly inked. `pub(crate)` means
visible anywhere in this crate (program) but not outside it.

**Used by:** [app/select.rs](app/select.md).

### `AppState::commit_undo`
[src/app.rs:5671](../../src/app.rs#L5671)

```rust
fn commit_undo(&mut self, cell: CellId) -> bool
```

**What it does.** Pushes the pixel patch for the edit just finished onto the undo history.

**Returns.** `true` if the edit changed any pixels (and so an undo step was added).

**Used by:** `pointer_up`, `finish_fill`, [app/lasso_fill.rs](app/lasso_fill.md).

### `AppState::take_patch`
[src/app.rs:5683](../../src/app.rs#L5683)

```rust
fn take_patch(&mut self, cell: CellId) -> Option<undo::Command>
```

**What it does.** Builds the `PixelPatch` undo command for the edit on `cell` without pushing it,
for callers that bundle it into a bigger step. Also refreshes the snapshot so it mirrors the cell
again.

**Returns.** `Some(PixelPatch { cell, x, y, w, h, before, after })`, or `None` when no snapshot was
live, the cell has no dirty rect, the rect is empty, or before equals after.

**How it works.**
1. `std::mem::take(&mut self.stroke_pre_live)` reads the flag and resets it to `false` in one go;
   returns `None` if it was already false.
2. Reads the cell's `dirty` rect (what this edit touched).
3. In parallel, copies that rect out of the cell into `after` and out of the snapshot into
   `before`.
4. If the snapshot belongs to this cell, writes `after` into it, grows `pre_inked`, and records
   `pre_mirror` — so the next edit of the same cell needs no copy.
5. Returns `None` for a no-op edit, else the command.

**Used by:** `commit_undo`, [app/select.rs](app/select.md), [app/liquify.rs](app/liquify.md).

### `AppState::begin_screen_pick`
[src/app.rs:5752](../../src/app.rs#L5752)

```rust
pub fn begin_screen_pick(&mut self)
```

**What it does.** Enters live screen-pick mode: the pixel under the cursor is sampled from the
screen (the OS framebuffer, so whatever is really on screen — canvas or desktop behind a
see-through window) and the next tap makes it the brush colour. It is a momentary mode, not a tool.

**How it works.**
1. Does nothing if already picking.
2. Remembers the current tool to return to, sets `screen_pick` and `screen_pick_arm` (the press
   that opened the mode must be released before a tap counts).
3. Drops any edit in progress so the gesture can't leave ink: cancels a fill, clears stroke, target,
   shape and fill state, cancels a selection gesture, and lands a floating selection.

**Used by:** `dispatch` (the shortcut action handler later in `AppState`), tests.

### `AppState::end_screen_pick`
[src/app.rs:5775](../../src/app.rs#L5775)

```rust
pub fn end_screen_pick(&mut self)
```

**What it does.** Leaves screen-pick mode without changing colour, tool or backdrop (cancel path:
Esc or toggling off). It deliberately keeps `screen_pick_tex` (the small loupe texture): freeing it
mid-frame, after the loupe already drew with it, would make wgpu use a destroyed texture and crash.

**Used by:** `commit_screen_pick` and `dispatch` (later in `AppState`),
[ui/shell.rs](ui/shell.md) (Esc while picking).

### `AppState::cut_cell`
[src/app.rs:5790](../../src/app.rs#L5790)

```rust
pub fn cut_cell(&mut self)
```

**What it does.** Moves the active slot's drawing into the drawing clipboard (`cell_clip`) and
leaves the frame blank, as an undo step. Refuses on a locked or reference layer, or when there is
nothing to cut. The copy is read before `structural_edit`, because that borrows the project
mutably for the edit.

**Used by:** `dispatch` (`CellCut` action), [ui/shell.rs](ui/shell.md),
[ui/tracks.rs](ui/tracks.md) (context menus).

### `AppState::paste_cell`
[src/app.rs:5806](../../src/app.rs#L5806)

```rust
pub fn paste_cell(&mut self)
```

**What it does.** Keys a copy of the clipboard drawing at the active slot, as an undo step. Refuses
on a locked or reference layer or with an empty clipboard. Pasting only ever allocates a new cell,
so the cheap timeline-only undo snapshot is enough.

**Used by:** `dispatch` (`CellPaste` action), [ui/shell.rs](ui/shell.md),
[ui/tracks.rs](ui/tracks.md).

### `AppState::clear_track_state`
[src/app.rs:5825](../../src/app.rs#L5825)

```rust
fn clear_track_state(&mut self)
```

**What it does.** Forgets everything the tracks hold about the project — selection, Shift-click
anchor, drag in progress, and the blank-cell cache. Used when the project itself is replaced.

**Used by:** `reset_with` (New project) and `load_project` (Open), elsewhere in `AppState`.

### `AppState::track_selection`
[src/app.rs:5833](../../src/app.rs#L5833)

```rust
pub fn track_selection(&self) -> Vec<(usize, usize)>
```

**What it does.** The selected track blocks as `(layer, key frame)` pairs, in sheet order (the
`BTreeSet` keeps them sorted). `.copied()` turns references into plain values.

**Used by:** the track edit methods below, [ui/tracks.rs](ui/tracks.md).

### `AppState::cell_is_blank`
[src/app.rs:5846](../../src/app.rs#L5846)

```rust
pub fn cell_is_blank(&mut self, id: CellId) -> Option<bool>
```

**What it does.** Whether cell `id` has no painted pixel (alpha only — an erased pixel can keep its
colour at zero coverage). Answers come from `blank_cache` when known. Otherwise it scans the cell,
but only while this frame's scan allowance (`blank_scans_left`) lasts.

**Returns.** `Some(true/false)`, or `None` when it isn't known yet and this frame's scans are
spent — the caller then shows the cell as a drawing and asks again next frame. A missing cell
counts as blank.

**Used by:** [ui/tracks.rs](ui/tracks.md) (drawing blank blocks differently).

### `AppState::painted`
[src/app.rs:5863](../../src/app.rs#L5863)

```rust
fn painted(&mut self, id: CellId)
```

**What it does.** Records in `blank_cache` that cell `id` is not blank, after a paint edit laid
coverage on it (only if the cell has a dirty rect). This avoids re-reading the cell. Not used for
erasing or deleting, which can empty a cell; those leave the answer to a scan.

**Used by:** `flush_stroke`, `pointer_up`, `finish_fill`.

### `AppState::layers_editable`
[src/app.rs:5870](../../src/app.rs#L5870)

```rust
fn layers_editable(&self, layers: impl IntoIterator<Item = usize>) -> bool
```

**What it does.** True if every layer index in `layers` can be painted (`Project::paintable`).
`impl IntoIterator<Item = usize>` accepts anything you can loop over that yields `usize` — an
array like `[b.layer]` or a `.map(...)` iterator. More: [Generics](../rust-primer.md#generics)

**Used by:** the track edit methods below.

### `AppState::track_edit`
[src/app.rs:5875](../../src/app.rs#L5875)

```rust
fn track_edit(&mut self, edit: impl FnOnce(&mut Project) -> Vec<(usize, usize)>)
```

**What it does.** Runs a track edit as one undo step (via `structural_edit`, which also clears the
track selection), then selects the `(layer, frame)` blocks the edit returned.

**Parameters.**
- `edit` — a closure that changes the project and returns which blocks to select afterwards.

**Used by:** `move_track_selection`, `retime_track_block`, `set_track_timing`,
`clear_track_selection`, `close_track_selection`, `insert_layer_frames_here`,
`remove_layer_frames_here`.

### `AppState::move_track_selection`
[src/app.rs:5883](../../src/app.rs#L5883)

```rust
pub fn move_track_selection(&mut self, dl: isize, df: isize, copy: bool, cursor: (usize, usize))
```

**What it does.** Drops the selected drawings `dl` layers and `df` frames away (negative = up /
earlier), moving them or copying them, and puts the cursor on `cursor` (layer, frame). Returns
without change if `Project::can_move_blocks` says the move is not allowed. The cursor move happens
inside the edit, so undo puts the cursor back too. The landed blocks become the new selection.

**Parameters.**
- `dl`, `df` — signed offsets in layers and frames (`isize` is a signed integer the size of a
  pointer).
- `copy` — copy instead of move.
- `cursor` — where the current layer and frame end up.

**Used by:** [ui/tracks.rs](ui/tracks.md) (dropping a block drag).

### `AppState::retime_track_block`
[src/app.rs:5898](../../src/app.rs#L5898)

```rust
pub fn retime_track_block(&mut self, b: Block, new_len: usize)
```

**What it does.** Makes drawing block `b` hold for `new_len` frames, sliding the drawings after it.
A block with `len: None` (the layer's last drawing, held to the end) counts as reaching the end of
the timeline. Does nothing if the length would not change or the layer is not editable. It
re-finds the block at `b.start` inside the edit and selects it afterwards.

**Used by:** [ui/tracks.rs](ui/tracks.md) (dragging a block's edge).

### `AppState::set_track_timing`
[src/app.rs:5913](../../src/app.rs#L5913)

```rust
pub fn set_track_timing(&mut self, n: usize)
```

**What it does.** Puts every selected drawing "on `n`s" — each held for `n` frames (animation
jargon: "on twos" = each drawing shows for two frames). Needs a non-empty selection on editable
layers.

**Used by:** [ui/tracks.rs](ui/tracks.md) ("On N" timing menu).

### `AppState::clear_track_selection`
[src/app.rs:5922](../../src/app.rs#L5922)

```rust
pub fn clear_track_selection(&mut self)
```

**What it does.** Deletes the selected drawings, leaving their frames blank (the frames stay; the
timing of other drawings is unchanged).

**Used by:** `dispatch` (`TrackClear` action), [ui/tracks.rs](ui/tracks.md).

### `AppState::close_track_selection`
[src/app.rs:5931](../../src/app.rs#L5931)

```rust
pub fn close_track_selection(&mut self)
```

**What it does.** Deletes the selected drawings **and** closes the gap they leave, so the later
drawings slide earlier. Nothing is selected afterwards.

**Used by:** `dispatch` (`TrackCloseGap` action), [ui/tracks.rs](ui/tracks.md).

### `AppState::insert_layer_frames_here`
[src/app.rs:5943](../../src/app.rs#L5943)

```rust
pub fn insert_layer_frames_here(&mut self, n: usize)
```

**What it does.** Inserts `n` held frames on the active layer after the playhead (only that layer
shifts), as one undo step. Refuses on a layer that can't be edited.

**Used by:** [ui/shell.rs](ui/shell.md), [ui/tracks.rs](ui/tracks.md).

### `AppState::remove_layer_frames_here`
[src/app.rs:5955](../../src/app.rs#L5955)

```rust
pub fn remove_layer_frames_here(&mut self, n: usize)
```

**What it does.** Removes `n` frames from the active layer starting at the playhead, as one undo
step. Refuses on a layer that can't be edited.

**Used by:** [ui/shell.rs](ui/shell.md), [ui/tracks.rs](ui/tracks.md).

### `AppState::active_layer_is_reference`
[src/app.rs:5966](../../src/app.rs#L5966)

```rust
fn active_layer_is_reference(&self) -> bool
```

**What it does.** True when the active layer is a *reference* layer (a light-table layer that
is shown but not drawn on). `.get(i)` returns an `Option` (`None` if the index is out of range),
`.map(|l| l.reference)` pulls the flag out, and `.unwrap_or(false)` turns "no such layer" into
`false` — like JS `layers[i]?.reference ?? false`.
More: [Option and Result](../rust-primer.md#option-and-result)

**Used by:** `cut_cell`, `paste_cell` (this file), `start_liquify` in
[app/liquify.rs](app/liquify.md), `paste_selection` in [app/select.rs](app/select.md).

### `AppState::frame_size`
[src/app.rs:5974](../../src/app.rs#L5974)

```rust
fn frame_size(&self) -> (f32, f32)
```

**What it does.** The project's frame (document) width and height as floats, returned as a
tuple `(w, h)`. `as f32` is an explicit number conversion; Rust never converts numbers
silently. More: [Numbers and casts](../rust-primer.md#numbers-and-casts)

**Used by:** `shape_plane` (this file), `grid_space` in [app/grids.rs](app/grids.md).

### `AppState::grid_snap_on`
[src/app.rs:5980](../../src/app.rs#L5980)

```rust
fn grid_snap_on(&self) -> bool
```

**What it does.** Whether the active perspective grid can be snapped to: snapping is switched
on, grids are shown, and the active grid exists and is not hidden itself.
`is_some_and(|g| g.visible)` is "there is a value and it passes this test".

**Used by:** `snapping`, `shapes_on_grid`.

### `AppState::snapping`
[src/app.rs:5989](../../src/app.rs#L5989)

```rust
pub fn snapping(&self) -> bool
```

**What it does.** Whether a stroke started now should snap to a perspective-grid direction. Only
"stroke" tools count: every pen slot, the Eraser, and the Shape tool when it draws a straight line.
(Shape rectangles and ellipses lie *on* the grid plane instead — see `shape_plane`.)

**Used by:** `snap_begin` (this file); [ui/shell.rs](ui/shell.md) (`perspective_options`
panel, `draw_tool_cursor`).

### `AppState::shapes_on_grid`
[src/app.rs:6000](../../src/app.rs#L6000)

```rust
pub fn shapes_on_grid(&self) -> bool
```

**What it does.** Whether a rectangle or ellipse dragged out now would lie flat on the active
perspective grid (Shape tool, not a line, and grid snapping available). The Shape tool's options
show this.

**Used by:** `shape_plane` (this file); `tools_content` in [ui/shell.rs](ui/shell.md).

### `AppState::shape_plane`
[src/app.rs:6010](../../src/app.rs#L6010)

```rust
fn shape_plane(&self, target: CellId) -> Option<GridPlane>
```

**What it does.** For a shape drag starting now, returns the active grid's plane expressed in the
pixels of the drawing (cell) being drawn into, or `None` when the shape should be drawn flat. A
*cell* is one drawing's pixel buffer ([glossary](../glossary.md)).

**Parameters.**
- `target` — id of the cell the shape will be drawn into.

**Returns.** `Some(GridPlane { h, rows, cols })`, where `h` is a *homography* (a 3×3 perspective
mapping that turns a unit square into any four-cornered shape) from the grid's square to its
on-cell quad; `None` when shapes aren't on a grid or the homography can't be built.

**How it works.**
1. Bail out with `None` unless `shapes_on_grid()`; the `?` after `active_grid()` also returns
   `None` early if there is no grid.
2. Read the cell's size and the active layer's display transform on this frame.
3. Take the grid's four corners in document space (`doc_corners`) and carry each into cell pixels
   with `Transform::doc_to_cell`. The doc comment notes the layer transform is a *similarity*
   (move/turn/uniform scale), so the carried corners still describe a plane.
4. Build `Homography::square_to_quad` from those corners.

**Used by:** `pointer_down` (this file) when a Shape drag starts.

### `AppState::shape_outline`
[src/app.rs:6037](../../src/app.rs#L6037)

```rust
pub fn shape_outline(&self, drag: &ShapeDrag) -> Vec<(f32, f32)>
```

**What it does.** The polyline (list of points, in cell pixels) that a shape drag draws. Both the
live preview and the final pixels come from here, so they always match.

**Parameters.**
- `drag` — the shape drag in progress: its kind, start, end, and optional grid plane.

**Returns.** The outline as `(x, y)` points.

**How it works.**
1. Read the brush radius, whether a Shift axis lock is on (`axis_lock_on`) and whether the
   centre key is on (`centre_on`).
2. If the drag has a plane, ask `tools::shape::outline_on_plane` for an outline on the grid. That
   returns `None` when the box would cross behind the horizon.
3. Otherwise (`unwrap_or_else`) draw it flat: with Shift, a line snaps to 15° steps
   (`axis_lock::snap_dir` with `LINE_STEP`, measured from the lock basis in cell pixels) and a
   box becomes a square (`axis_lock::square`). With the centre key, the start point is
   mirrored through the pointer (`axis_lock::mirror`) so the press point becomes the middle.
   Then `tools::shape::outline` makes the points.

The `match (lock, drag.kind) { (false, _) => …, (true, ShapeKind::Line) => …, (true, _) => … }`
matches on a pair at once; `_` means "anything".
More: [Enums and match](../rust-primer.md#enums-and-match)

**Used by:** `pointer_up` (this file, to rasterize the shape); `paint_canvas` in
[ui/shell.rs](ui/shell.md) (the preview).

### `AppState::slide_shape`
[src/app.rs:6077](../../src/app.rs#L6077)

```rust
fn slide_shape(&mut self, p: [f32; 2])
```

**What it does.** Called as a Shape drag's pointer reaches document point `p`. While the *move
key* (Space by default) is held, the whole shape slides with the pointer instead of growing —
along its grid if it lies on one. A snapped line's perspective lock moves too and is re-aimed
from where it lands.

**How it works.**
1. Remember `p` as the new pointer position, getting the previous one back (`Option::replace`).
2. Stop unless there was a previous point, the move key is on and a shape drag exists
   (`let Some(last) = … else { return; }` — an early-return unwrap).
3. Convert old and new points to active-cell pixels and move the drag's `start` with
   `tools::shape::slide` (which follows the grid plane when there is one).
4. If a perspective `snap_lock` exists, shift its start by the same document-space delta; if it
   had already picked a direction, re-pick the closest grid direction from the new start
   (`perspective::pick_dir`), keeping the old one if nothing better is found.

**Used by:** `stroke_point_doc` when the tool is Shape.

### `AppState::snap_begin`
[src/app.rs:6110](../../src/app.rs#L6110)

```rust
pub fn snap_begin(&mut self, p: [f32; 2]) -> [f32; 2]
```

**What it does.** Starts a stroke at document point `p`. Arms perspective snapping when it is on
(a `SnapLock` with no direction yet), clears any Shift axis lock, and records `p` as the
stroke's last point and the shape pointer. The first point itself is never moved, so it returns
`p` unchanged. `cond.then_some(x)` is `cond ? Some(x) : None`.

**Used by:** `draw` in [ui/shell.rs](ui/shell.md), at pen-down.

### `AppState::axis_lock_on`
[src/app.rs:6122](../../src/app.rs#L6122)

```rust
pub fn axis_lock_on(&self) -> bool
```

**What it does.** True while Shift is held *and* was pressed after the current drag began. Shift
already down at the press chose the drag (it is the pan hold, and a selection mode modifier), so
it only counts as an axis lock once released and pressed again. The lock holds a stroke to an
axis, a shape line to 15° steps and a box to a square.

**Used by:** `shape_outline`, `lock_screen_drag`, `stroke_point_doc`, `camera_frame_move` (this
file); `perspective_move` in [app/grids.rs](app/grids.md); `sel_drag_path` and
`drag_selection` in [app/select.rs](app/select.md); `drag_target` in
[ui/tracks.rs](ui/tracks.md).

### `AppState::drag_began`
[src/app.rs:6128](../../src/app.rs#L6128)

```rust
pub fn drag_began(&mut self)
```

**What it does.** Marks the start of a drag. Whatever Shift, the move key and the centre key are
doing right now chose this drag, so each is marked "stale" and locks nothing until let go and
pressed again (`MidKey::began`).

**Used by:** `draw` in [ui/shell.rs](ui/shell.md); `show` in [ui/tracks.rs](ui/tracks.md).

### `AppState::read_modifiers`
[src/app.rs:6135](../../src/app.rs#L6135)

```rust
pub(crate) fn read_modifiers(&mut self, m: egui::Modifiers)
```

**What it does.** Stores this frame's Shift and Alt state. `(a, b) = (x, y)` assigns two fields
at once. `self.shift_stale &= self.shift_held` clears the "stale" mark as soon as Shift is let
go, so the next Shift press counts. `pub(crate)` means "public inside this crate only".
More: [Modules and visibility](../rust-primer.md#modules-and-visibility)

**Used by:** `read_input`.

### `AppState::read_input`
[src/app.rs:6142](../../src/app.rs#L6142)

```rust
pub(crate) fn read_input(&mut self, i: &egui::InputState)
```

**What it does.** Samples this frame's modifiers and whether the move key (`Action::ShapeMove`)
and centre key (`Action::ShapeCentre`) are still held, as `update` does before anything else.
The pointer handlers run off tablet packets and have no egui `InputState` of their own, which is
why these are cached on `AppState`.

**Used by:** `update` (inside `ctx.input(|i| self.read_input(i))`).

### `AppState::hold_mid_keys`
[src/app.rs:6151](../../src/app.rs#L6151)

```rust
pub(crate) fn hold_mid_keys(&mut self, slide: bool, centre: bool)
```

**What it does.** Test-only helper (compiled only for `cargo test`, see
[attributes and cfg](../rust-primer.md#attributes-and-cfg)): sets the move and centre keys as a
frame would read them, for tests that drive `AppState` without a real frame.

**Used by:** tests.

### `AppState::move_on`
[src/app.rs:6158](../../src/app.rs#L6158)

```rust
pub fn move_on(&self) -> bool
```

**What it does.** True when the move key went down partway through the current drag: the shape,
selection box or camera box slides under the pointer instead of growing.

**Used by:** `slide_shape`, `camera_box_move` (this file); `select_move` in
[app/select.rs](app/select.md).

### `AppState::centre_on`
[src/app.rs:6164](../../src/app.rs#L6164)

```rust
pub fn centre_on(&self) -> bool
```

**What it does.** True when the centre key (Alt by default) went down partway through the drag:
the shape or box grows outward from the press point as its middle.

**Used by:** `shape_outline`, `camera_box_frame` (this file); `sel_drag_path` in
[app/select.rs](app/select.md).

### `AppState::mid_drag_under_way`
[src/app.rs:6170](../../src/app.rs#L6170)

```rust
fn mid_drag_under_way(&self) -> bool
```

**What it does.** True while a shape, a camera box, or a rectangle/ellipse selection is being
dragged out — the drags the move and centre keys work on. `matches!(value, Pattern)` is a
one-line "does this match the pattern?" test.
More: [Macros](../rust-primer.md#macros)

**Used by:** `handle_keys`, so that Space moving a shape doesn't also trigger Play.

### `AppState::lock_basis_doc`
[src/app.rs:6179](../../src/app.rs#L6179)

```rust
pub fn lock_basis_doc(&self) -> f32
```

**What it does.** The angle (radians, in document space) of the "horizontal" a Shift lock keeps
to. Normally `0.0` — the canvas's own axes. With `lock_screen_axes` on it is the *screen's*
horizontal, brought back into document space through the view's rotation and mirroring.

**How it works.** Return `0.0` unless `lock_screen_axes`. Take the rotation from the camera when
looking through it (negated), else from the view. Then `atan2(-sy·sin, sx·cos)`, where `sx`/`sy`
are `-1` when the view is flipped horizontally/vertically — the same order the drawing transform
applies them (mirror first, then turn).

**Used by:** `lock_basis_cell`, `stroke_point_doc`, `camera_frame_move` (this file);
`perspective_move` in [app/grids.rs](app/grids.md); `lock_basis_lead` in
[app/select.rs](app/select.md).

### `AppState::lock_basis_cell`
[src/app.rs:6196](../../src/app.rs#L6196)

```rust
pub fn lock_basis_cell(&self) -> f32
```

**What it does.** `lock_basis_doc` converted into the active drawing's own pixel space (its layer
may be rotated). It maps the origin and a point 100 px along the document-space axis into cell
pixels and takes the angle between them.

**Used by:** `shape_outline`.

### `AppState::lock_screen_drag`
[src/app.rs:6207](../../src/app.rs#L6207)

```rust
pub fn lock_screen_drag(&self, anchor: egui::Pos2, at: egui::Pos2) -> egui::Pos2
```

**What it does.** For drags whose two directions mean two different things (a Fill press drag:
sideways = gap, up/down = expand), holds screen point `at` to the screen's horizontal or vertical
through `anchor` while Shift locks; otherwise returns `at` unchanged.

**Used by:** `fill_drag_to` (this file); `button_drag_to` in
[app/button_drag.rs](app/button_drag.md); `wand_drag_to` in [app/select.rs](app/select.md).

### `AppState::stroke_point_doc`
[src/app.rs:6226](../../src/app.rs#L6226)

```rust
pub fn stroke_point_doc(&mut self, p: [f32; 2]) -> [f32; 2]
```

**What it does.** Turns a raw pen position `p` (document space) into the point the stroke should
actually use: held to an axis while Shift locks, else snapped to the perspective grid (if
snapping), else unchanged.

**Parameters.**
- `p` — the pointer position in document pixels.

**Returns.** The adjusted point, which is also stored as the stroke's last point.

**How it works.**
1. For the Shape tool, first let `slide_shape` move the shape if the move key is held.
2. **No Shift lock:** if a lock was just released, restart perspective snapping from the
   stroke's last point (so the line picks a new grid direction from there). Return `snap_doc(p)`.
3. **Shift lock, Shape tool:** pass `p` through — the shape's outline does its own locking.
4. **Shift lock, freehand:** keep a `StrokeAxis` starting at the stroke's last point. Until the
   pointer has moved `DECIDE_PX` = 6 *screen* pixels (document distance × `view_scale`), the point
   stays on the start. Once far enough, pick horizontal or vertical once
   (`axis_lock::hv_angle`, relative to `lock_basis_doc`) and from then on project every point onto
   that axis (`axis_lock::onto`).

Picking the axis only once avoids zig-zagging when the hand moves near a diagonal. `const` inside
a function is a local constant. More: [Constants and statics](../rust-primer.md#constants-and-statics)

**Example.** The same idea in JS:
```js
function lockPoint(state, p) {
  const from = state.axis?.from ?? state.lastPoint;
  if (state.axis?.angle == null) {
    if (dist(from, p) * state.viewScale >= 6) state.axis = { from, angle: hvAngle(from, p) };
    else return from;                 // not decided yet: stay put
  }
  return projectOntoLine(from, state.axis.angle, p);
}
```

**Used by:** `snapped` in [ui/shell.rs](ui/shell.md), for every stroke point.

### `AppState::snap_doc`
[src/app.rs:6267](../../src/app.rs#L6267)

```rust
pub fn snap_doc(&mut self, p: [f32; 2]) -> [f32; 2]
```

**What it does.** Snaps a stroke point to the active perspective grid. The direction (one of the
grid's line families, e.g. toward a vanishing point) is picked once, when the pointer is far
enough from the start to show where it is heading, then held for the whole stroke.

**Returns.** `p` unchanged when no snap is armed; the stroke start until the direction is decided;
otherwise `p` projected onto the line from the start along the chosen direction
(`perspective::project`).

**How it works.**
1. No `snap_lock` → return `p`.
2. If no direction yet: measure motion from the start; under `DECIDE_PX` = 10 screen pixels,
   return the start. Otherwise get the grid's candidate directions at the start
   (`snap_dirs`, including verticals if `snap_vertical`) and pick the one closest to the motion
   (`perspective::pick_dir`). A degenerate grid gives none: snapping is dropped and `p` returned.
3. Project `p` onto that direction.

**Used by:** `stroke_point_doc`.

### `AppState::set_brush_color`
[src/app.rs:6309](../../src/app.rs#L6309)

```rust
pub fn set_brush_color(&mut self, rgb: [u8; 3])
```

**What it does.** Sets the brush colour (fully opaque) everywhere it is mirrored. The active
`brush` is not enough: each tool keeps its own copy (a pen slot's in `pens`, any other tool's in
`tool_brushes`), which a tool switch restores from. With the colour wheel pinned, every tool's copy gets the colour; otherwise only
the active tool's slot.

**Parameters.**
- `rgb` — the colour as `[r, g, b]` bytes. `[u8; 3]` is a fixed-size array of three bytes.

**Used by:** `set_wheel_pinned` (this file); `end_button_drag` in
[app/button_drag.rs](app/button_drag.md); `color_panel` in [ui/color_wheel.rs](ui/color_wheel.md);
`swatch_strip` in [ui/shell.rs](ui/shell.md).

### `AppState::brush_of`
[src/app.rs:6322](../../src/app.rs#L6322)

```rust
pub fn brush_of(&self, t: ActiveTool) -> &BrushSettings
```

The brush `t` keeps between tool switches: its pen slot's, or its own.

### `AppState::brush_of_mut`
[src/app.rs:6330](../../src/app.rs#L6330)

```rust
pub fn brush_of_mut(&mut self, t: ActiveTool) -> &mut BrushSettings
```

`brush_of`, but a mutable reference (`&mut`), so the caller can change that kept brush in place —
`self.brush_of_mut(ActiveTool::Fill).opacity = 0.5`.

### `AppState::every_brush_mut`
[src/app.rs:6339](../../src/app.rs#L6339)

```rust
fn every_brush_mut(&mut self) -> impl Iterator<Item = &mut BrushSettings>
```

Every tool's kept brush, every pen slot's among them.

### `AppState::wheel_pinned`
[src/app.rs:6344](../../src/app.rs#L6344)

```rust
pub fn wheel_pinned(&self) -> bool
```

**What it does.** Getter: whether the colour wheel is pinned (all tools share one colour). The
field is private, so the UI reads it through this method.

**Used by:** `color_panel` in [ui/color_wheel.rs](ui/color_wheel.md).

### `AppState::set_wheel_pinned`
[src/app.rs:6351](../../src/app.rs#L6351)

```rust
pub fn set_wheel_pinned(&mut self, on: bool)
```

**What it does.** Pins or unpins the colour wheel. Pinning copies the colour in use to every
tool (through `set_brush_color`); unpinning leaves each tool with that colour, free to drift apart
again.

**Used by:** `dispatch` (`Action::WheelPinToggle`); `color_panel` in
[ui/color_wheel.rs](ui/color_wheel.md).

### `AppState::pie_colors`
[src/app.rs:6362](../../src/app.rs#L6362)

```rust
pub fn pie_colors(&self) -> (Vec<[u8; 3]>, usize)
```

**What it does.** What the swatch pie (a radial colour menu opened by a held button) offers, and
how many of those go on its inner ring.

**Returns.** Unpinned: the pinned swatches, with `PIE_INNER` on the inner ring. Pinned: the colour
harmony set first (converted to RGB) followed by the swatches, with exactly the harmony set on the
inner ring.

**Used by:** `button_drag_to`, `end_button_drag` in [app/button_drag.rs](app/button_drag.md);
`draw` in [ui/shell.rs](ui/shell.md).

### `AppState::stash_brush`
[src/app.rs:6375](../../src/app.rs#L6375)

```rust
pub(crate) fn stash_brush(&mut self)
```

**What it does.** Writes the active brush back to where its tool keeps it (`brush_of_mut`: the pen
slot, or `tool_brushes`), so a tool switch or a save can restore it. For the Shape tool (which has
no size of its own) the thickness is also written into the last pen's brush.
`.clone()` makes an independent copy — needed because Rust won't let two places own the same
value. More: [Ownership and borrowing](../rust-primer.md#ownership-and-borrowing)

**Used by:** `commit_screen_pick`, `reset_with`, `save` (this file); `set_tool` in
[app/select.rs](app/select.md).

### `AppState::smoothing_for`
[src/app.rs:6384](../../src/app.rs#L6384)

```rust
pub fn smoothing_for(&self, tool: ActiveTool) -> Option<&SmoothingOptions>
```

**What it does.** Returns `tool`'s own line-smoothing settings, or `None` for tools that don't
draw freehand: a pen slot's own (`pens[i].smoothing`), or the eraser's (`eraser_smoothing`). The
result is a *borrowed reference* (`&`), not a copy.

**Used by:** `smoothing` (this file); `tool_toggle` in [ui/shell.rs](ui/shell.md).

### `AppState::smoothing`
[src/app.rs:6393](../../src/app.rs#L6393)

```rust
pub fn smoothing(&self) -> Option<&SmoothingOptions>
```

**What it does.** `smoothing_for` the active tool.

**Used by:** `pointer_down` (this file, when a stroke starts); `draw_tool_cursor` in
[ui/shell.rs](ui/shell.md).

### `AppState::smoothing_mut`
[src/app.rs:6397](../../src/app.rs#L6397)

```rust
pub fn smoothing_mut(&mut self) -> Option<&mut SmoothingOptions>
```

**What it does.** Same as `smoothing` but returns a *mutable* reference (`&mut`), so the caller
can change the active pen's smoothing in place.

**Used by:** `brush_settings_window` in [ui/shell.rs](ui/shell.md).

### `AppState::smoothing_and_before_mut`
[src/app.rs:6403](../../src/app.rs#L6403)

```rust
fn smoothing_and_before_mut(&mut self) -> Option<(&mut SmoothingOptions, &mut Smoothing)>
```

The active tool's smoothing, and the kind it had before the Stabilizer key turned it on.

### `AppState::toggle_stabilizer`
[src/app.rs:6413](../../src/app.rs#L6413)

```rust
fn toggle_stabilizer(&mut self)
```

**What it does.** Turns the active pen's Stabilizer smoothing on, remembering the kind it had in
the slot's `before_stabilizer` (the eraser's `eraser_before_stabilizer`); pressed again, it goes
back to that kind. Tools without smoothing do nothing.
The next stroke picks up the change; a stroke under way keeps its own.

**Used by:** `dispatch` (`Action::StabilizerToggle`).

### `AppState::cycle_smoothing`
[src/app.rs:6425](../../src/app.rs#L6425)

```rust
fn cycle_smoothing(&mut self)
```

**What it does.** Steps the active pen's smoothing kind None → Basic → Weighted → Stabilizer →
None. Stepping onto Stabilizer records the previous kind, so `toggle_stabilizer` can turn it back.

**Used by:** `dispatch` (`Action::SmoothingCycle`).

### `AppState::load_brush`
[src/app.rs:6442](../../src/app.rs#L6442)

```rust
pub(crate) fn load_brush(&mut self, t: ActiveTool)
```

**What it does.** Makes `t` the active tool and loads its stored brush (`brush_of`). With the wheel
pinned, the current colour carries over to the new tool. A pen slot becomes `last_pen` (the one it
replaces becomes `prev_pen`, for the swap key). The Shape tool takes the size of the pen used last
rather than a size of its own.

**Used by:** `reset_with`, `commit_screen_pick` (this file); `set_tool` in
[app/select.rs](app/select.md).

### `AppState::pin_swatch`
[src/app.rs:6463](../../src/app.rs#L6463)

```rust
pub fn pin_swatch(&mut self)
```

**What it does.** Pins the current brush colour as a swatch (via `pin_rgb`); pinning a colour
already held does nothing.

**Used by:** `swatch_strip` in [ui/shell.rs](ui/shell.md).

### `AppState::pin_rgb`
[src/app.rs:6469](../../src/app.rs#L6469)

```rust
pub fn pin_rgb(&mut self, rgb: [u8; 3])
```

**What it does.** Appends `rgb` to the swatch `palette` unless it is already there; if that takes
the list past `MAX_SWATCHES`, drops the oldest (index 0).

**Used by:** `pin_swatch`, `pin_set`.

### `AppState::pin_set`
[src/app.rs:6480](../../src/app.rs#L6480)

```rust
pub fn pin_set(&mut self, colors: &[[u8; 3]])
```

**What it does.** Pins every colour of a harmony set in order, skipping ones already held.
`&[[u8; 3]]` is a *slice* — a borrowed view of a list of RGB triples (like a read-only array
argument in JS). More: [Strings and slices](../rust-primer.md#strings-and-slices)

**Used by:** `color_panel` in [ui/color_wheel.rs](ui/color_wheel.md).

### `AppState::commit_screen_pick`
[src/app.rs:6488](../../src/app.rs#L6488)

```rust
pub fn commit_screen_pick(&mut self, color: [u8; 4])
```

**What it does.** Commits a colour sampled from the screen: applies it to every tool, goes back
to the drawing tool that was active before picking, and leaves pick mode.

**How it works.**
1. `stash_brush` first, so any unsaved change to the active brush (a preset, a slider) is kept.
2. Write `color` into every slot of `tool_brushes`.
3. `load_brush(screen_pick_return_tool)`, then set the colour on the brush and its slot again.
4. `end_screen_pick`.

**Used by:** `screen_pick_live` in [ui/shell.rs](ui/shell.md).

### `AppState::apply_picked_color`
[src/app.rs:6505](../../src/app.rs#L6505)

```rust
pub fn apply_picked_color(&mut self, color: [u8; 4])
```

**What it does.** Sets `color` on the active brush and every tool slot. Nothing calls it at
present; `#[allow(dead_code)]` silences the compiler's "unused function" warning.

### `AppState::fill_source`
[src/app.rs:6516](../../src/app.rs#L6516)

```rust
fn fill_source(&self, layer_idx: usize, cell_w: u32, cell_h: u32) -> Option<Canvas>
```

Which pixels a Fill press on `layer_idx` treats as walls: every visible layer (when the Fill tool's
"all visible" option is on), else the layer linked by `lines_from`. `None` means "use the
drawing's own pixels".

### `AppState::visible_lines`
[src/app.rs:6528](../../src/app.rs#L6528)

```rust
fn visible_lines(&self, layer_idx: usize, cell_w: u32, cell_h: u32) -> Option<Canvas>
```

Flattens every visible non-reference layer at the current frame into one image (what you see, minus
view fades), then converts it into the pixel grid of `layer_idx`'s drawing. Built once per Fill
press.

### `AppState::fill_boundary`
[src/app.rs:6543](../../src/app.rs#L6543)

```rust
fn fill_boundary(&self, layer_idx: usize, cell_w: u32, cell_h: u32) -> Option<Canvas>
```

Renders the layer linked by `lines_from` (line art on another layer) into the pixel grid of
`layer_idx`'s drawing, so a flood fill on a colour layer stops at the line layer's strokes.
Returns `None` when there is no link or it can't be resolved on this frame. The linked layer is used
even when hidden: the link is an explicit choice, visibility is just display.

### `AppState::track_layer_change`
[src/app.rs:6583](../../src/app.rs#L6583)

```rust
fn track_layer_change(&mut self)
```

Runs once per frame (first thing in `update`) to remember which layer you came from, for the "jump
to last layer" action. `current_layer` is set in many places; comparing with last frame's values
here catches all of them without touching any. Layers are remembered by unique id (`uid`), so
adding or deleting other layers doesn't confuse it.

### `AppState::goto_last_layer`
[src/app.rs:6602](../../src/app.rs#L6602)

```rust
pub fn goto_last_layer(&mut self)
```

Jumps back to the previously active layer, if it still exists.

### `AppState::select_layer`
[src/app.rs:6611](../../src/app.rs#L6611)

```rust
fn select_layer(&mut self, i: usize)
```

Makes layer `i` active; an out-of-range `i` does nothing (which stops the step keys at the ends).

### `AppState::move_active_layer`
[src/app.rs:6622](../../src/app.rs#L6622)

```rust
pub fn move_active_layer(&mut self, up: bool)
```

Moves the active layer one step up or down the Layers panel — past its neighbour, or into/out of a
group (a whole group moves together). One undo step; nothing at all at the end of the stack, so
holding the key doesn't fill the history with empty steps.

### `AppState::add_cleanup_layer`
[src/app.rs:6637](../../src/app.rs#L6637)

```rust
pub fn add_cleanup_layer(&mut self, of: usize)
```

Adds a new layer above layer `of` for redrawing it cleanly, with its key jumps linked to it. One
undo step.

### `AppState::step_layer_row`
[src/app.rs:6653](../../src/app.rs#L6653)

```rust
fn step_layer_row(&mut self, delta: isize)
```

Selects the row `delta` rows down the Layers panel (negative = up), stopping at the ends. Layers in
folded groups aren't rows and are skipped.

### `AppState::selected_layer_indices`
[src/app.rs:6667](../../src/app.rs#L6667)

```rust
pub fn selected_layer_indices(&self) -> Vec<usize>
```

Every selected layer index, bottom first: the active layer plus any Ctrl/Shift-picked ones.
`(0..n).filter(…).collect()` builds a `Vec` of the indexes passing the test.

### `AppState::selected_drawing_layers`
[src/app.rs:6676](../../src/app.rs#L6676)

```rust
fn selected_drawing_layers(&self) -> Vec<usize>
```

The selected layers that hold drawings; a selected group stands for all its layers. Bottom first,
no duplicates.

### `AppState::orient_selected_layers`
[src/app.rs:6693](../../src/app.rs#L6693)

```rust
pub fn orient_selected_layers(&mut self, o: Orient)
```

Flips or rotates every drawing of every selected layer (groups include their layers), as one undo
step. Locked and reference layers are skipped.

### `AppState::set_selected_opacity`
[src/app.rs:6701](../../src/app.rs#L6701)

```rust
pub fn set_selected_opacity(&mut self, v: f32)
```

Sets the opacity of every selected layer. The first change since the last `commit_layer_opacity`
takes a timeline snapshot, so a whole slider drag undoes in one step.

### `AppState::set_layer_opacity`
[src/app.rs:6712](../../src/app.rs#L6712)

```rust
pub fn set_layer_opacity(&mut self, i: usize, v: f32)
```

Sets one layer's opacity from the % on its row; same one-undo-step-per-drag behaviour.

### `AppState::commit_layer_opacity`
[src/app.rs:6723](../../src/app.rs#L6723)

```rust
pub fn commit_layer_opacity(&mut self)
```

Finishes an opacity drag: pushes the undo step opened by the first change, or nothing if nothing
changed.

### `AppState::toggle_selected_clip`
[src/app.rs:6740](../../src/app.rs#L6740)

```rust
pub fn toggle_selected_clip(&mut self)
```

Toggles "clip to layer below" on the selected layers. All of them go the opposite way to the active
layer, so a mixed selection ends up consistent. Groups can't clip and are skipped.

### `AppState::toggle_selected_alpha_lock`
[src/app.rs:6746](../../src/app.rs#L6746)

```rust
pub fn toggle_selected_alpha_lock(&mut self)
```

Toggles alpha lock (paint only where pixels already exist) on the selected layers, the same way.

### `AppState::toggle_selected_flag`
[src/app.rs:6750](../../src/app.rs#L6750)

```rust
fn toggle_selected_flag(&mut self, flag: impl Fn(&mut crate::doc::layer::Layer) -> &mut bool)
```

Shared logic for the two toggles above. `flag` is a function that, given a layer, returns a
*mutable reference* to one of its `bool` fields (`|l| &mut l.clip`). The function reads the lead
layer's flag, flips it, then writes that value into every selected layer through the same accessor —
a neat way to reuse one toggle for several fields. `*flag(layer) = on` writes through the reference.

### `AppState::group_selected_layers`
[src/app.rs:6765](../../src/app.rs#L6765)

```rust
pub fn group_selected_layers(&mut self)
```

Puts the selected layers into a new group. One undo step.

### `AppState::ungroup_active`
[src/app.rs:6780](../../src/app.rs#L6780)

```rust
pub fn ungroup_active(&mut self)
```

Dissolves the active layer's group (or the active group itself), leaving its layers where they are.
One undo step.

### `AppState::delete_selected_layers`
[src/app.rs:6792](../../src/app.rs#L6792)

```rust
pub fn delete_selected_layers(&mut self)
```

Deletes the selected layers (a group together with its layers). One undo step; the last remaining
layer is never deleted.

### `AppState::fill_boundary_name`
[src/app.rs:6811](../../src/app.rs#L6811)

```rust
pub fn fill_boundary_name(&self) -> Option<&str>
```

The name of the layer that bounds the active layer's fills (its `lines_from` link), for UI hints.
The chain of `?` operators returns `None` as soon as any step is missing.

### `AppState::dispatch`
[src/app.rs:6820](../../src/app.rs#L6820)

```rust
pub fn dispatch(&mut self, action: Action)
```

**The central command handler.** Every keyboard shortcut and many menu items arrive here as an
`Action` value (an enum listing every command — see [input/shortcuts.rs](input/shortcuts.md)), and
one big `match` turns each into a method call: `Action::ToolEraser => self.set_tool(ActiveTool::Eraser)`,
`Action::Undo => self.undo()`, `Action::SaveProject => self.save_project()`, and so on.

**How it works.**
1. First it ignores actions that edit the active slot's pixels or keys when the active row is a
   group header (groups have no drawings).
2. Then the `match` covers every `Action` variant: tools, playback, frame and key editing, layers,
   brush size, smoothing, undo/redo, view reset/flip, art flips/rotations, cell and selection
   clipboard, save/open, camera and transform modes, panels.
3. "Hold" actions (pan, zoom, rotate, tool-adjust, selection modes…) are listed with an empty arm
   `=> {}`: they are not commands but keys the canvas checks while a drag starts.

Because Rust's `match` must be exhaustive, adding a new `Action` variant is a compile error until it
is handled here — which is exactly what you want from a command table.

```js
// The same idea in JS
function dispatch(state, action) {
  switch (action) {
    case "ToolPencil": return state.setTool("Pencil");
    case "Undo":       return state.undo();
    // ...every action...
  }
}
```

**Used by:** `handle_keys` (shortcuts), [ui/shell.rs](ui/shell.md) (menus, buttons), tests.

### `AppState::save_project`
[src/app.rs:7075](../../src/app.rs#L7075)

```rust
pub fn save_project(&mut self)
```

Save (Ctrl+S): writes to the file the project came from, or asks for a file (Save As) if it has
never been saved.

### `AppState::save_project_as`
[src/app.rs:7083](../../src/app.rs#L7083)

```rust
pub fn save_project_as(&mut self)
```

Save As: always shows the save dialog (starting at the current path) and writes there if a path was
chosen.

### `AppState::write_project`
[src/app.rs:7092](../../src/app.rs#L7092)

```rust
fn write_project(&mut self, path: PathBuf)
```

Starts writing the project to `path` **on a background thread**, so the window doesn't freeze while
a large project is compressed and written.

**How it works.**
1. If a save is already running, it returns (one writer at a time — two threads writing the same
   file could corrupt it).
2. `self.project.clone()` takes a snapshot. Cloning is cheap: drawings are `Arc`-shared, so only
   pointers are copied, and a drawing edited during the save is copied on its first write.
3. A channel `(tx, rx)` is made and `thread::spawn(move || …)` runs
   `io::project_file::save_to(&project, &dest)`, sending the result through `tx`.
4. The receiving end is stored in `self.save_job`; `poll_save_job` checks it each frame.

See [walkthroughs.md](../walkthroughs.md) for the whole save path.

### `AppState::poll_save_job`
[src/app.rs:7117](../../src/app.rs#L7117)

```rust
fn poll_save_job(&mut self)
```

Called every frame by `update`. Checks the save worker's channel without blocking (`try_recv`):
- still running (`Empty`) → put the job back and check next frame;
- `Ok(Ok(()))` → success: remember the path, show a "Saved" toast;
- `Ok(Err(e))` → the save failed: show the error in a dialog;
- `Disconnected` → the worker vanished without reporting: also treated as a failure.

`project_path` only changes on success, so retrying a failed save targets the same file.

### `AppState::finish_pending_save`
[src/app.rs:7147](../../src/app.rs#L7147)

```rust
fn finish_pending_save(&mut self)
```

Waits (blocking, `recv()`) for an in-flight save to finish. Called before the project is replaced
(New, Open) and when the app exits, so a background save is never cut short.

### `AppState::load_project`
[src/app.rs:7166](../../src/app.rs#L7166)

```rust
pub fn load_project(&mut self, project: Project, path: Option<PathBuf>)
```

Replaces the current project with a loaded one. Like `reset_with`, it first waits for any running
save, ends the Krita link and stashes grids; then installs the new project, drops drawings nothing
uses (`compact_cells`), remembers `path`, retires all textures and marks every cell for upload,
and resets history, view, playback, strokes, selection and camera drags. User preferences (tools,
brushes, shortcuts, pen) stay. If the loaded camera already goes past the canvas it turns "keep
inside canvas" off and says so in a toast.

### `AppState::undo`
[src/app.rs:7222](../../src/app.rs#L7222)

```rust
pub fn undo(&mut self)
```

Undo (Ctrl+Z). Before touching the history it first backs out of anything still in progress that
isn't recorded yet: a polygon loses its newest corner, a lasso fill or warp its last step, floating
selected pixels go back where they came from. Only if none of those applied does it call
`history.undo(&mut self.project)` and then `apply_touched` to refresh whatever the undo changed
(textures, selection…). See [undo.rs](undo.md).

### `AppState::redo`
[src/app.rs:7239](../../src/app.rs#L7239)

```rust
pub fn redo(&mut self)
```

Redo. Cancels any gesture in progress; if pixels are floating or a warp is open it *lands* them
instead (a new edit, which ends the redo chain anyway); otherwise redoes the next history step.

### `AppState::reload_shortcuts_file`
[src/app.rs:7256](../../src/app.rs#L7256)

```rust
pub(crate) fn reload_shortcuts_file(&mut self)
```

Settings → Reload: re-reads `shortcuts.toml` after the user edited it by hand. If the file doesn't
parse, the bindings in use are kept and the file is left alone (loading defaults would let the next
rebind overwrite the user's file). The result is shown in Settings.

### `AppState::handle_keys`
[src/app.rs:7275](../../src/app.rs#L7275)

```rust
pub(crate) fn handle_keys(&mut self, ctx: &egui::Context) -> bool
```

Called every frame from `update`. Handles this frame's key presses: if a shortcut is being rebound
it captures the new key; otherwise it asks the shortcut map which actions fired and calls
`dispatch` for each. It returns whether a Tab press found nothing focused — egui also treats Tab
as "focus next widget", and `drop_stray_focus` undoes that after the frame is drawn.

### `AppState::drop_stray_focus`
[src/app.rs:7424](../../src/app.rs#L7424)

```rust
pub(crate) fn drop_stray_focus(&self, ctx: &egui::Context, tab_focus: bool)
```

Clears keyboard focus that no widget should hold: any focus while the panels are hidden, and the
focus a Tab press handed out. A focused widget swallows the shortcuts, so without this the keys
would stop working after toggling the panels with Tab.

## `impl eframe::App for AppState`

### `AppState::on_exit`
[src/app.rs:7442](../../src/app.rs#L7442)

```rust
fn on_exit(&mut self)
```

Part of eframe's `App` trait: called just before the program ends. Waits for any background save
to finish so closing the window can't lose work.

### `AppState::raw_input_hook`
[src/app.rs:7448](../../src/app.rs#L7448)

```rust
fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput)
```

Part of eframe's `App` trait: called with the raw input before egui processes it. egui-winit
converts Ctrl+C/X/V into clipboard events; this puts the key presses back so the selection
shortcuts see them. See [input/clipboard_keys.rs](input/clipboard_keys.md).

### `AppState::save`
[src/app.rs:7452](../../src/app.rs#L7452)

```rust
fn save(&mut self, storage: &mut dyn eframe::Storage)
```

Part of eframe's `App` trait: eframe calls it periodically (about every 30 s) and on exit. It first
stores the active tool's brush into its slot (`stash_brush`), then builds a `UiPrefs` value from
the current workspace settings (panels, onion, brushes, palette, grids, Krita path, presets,
selection and fill options, colours…) and writes it into eframe's storage under `UI_PREFS_KEY`;
eframe serialises it to `ui.ron`. Panel positions are saved separately by egui itself.

### `AppState::clear_color`
[src/app.rs:7514](../../src/app.rs#L7514)

```rust
fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4]
```

Part of eframe's `App` trait: the colour the window is cleared to before drawing. It returns the
background colour multiplied by `bg_opacity` (premultiplied RGBA), so lowering the backdrop slider
makes the window see-through.

### `AppState::update`
[src/app.rs:7524](../../src/app.rs#L7524)

```rust
fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame)
```

**The per-frame main loop.** eframe calls this every time it draws a frame (continuously while
something animates, otherwise when input arrives or a repaint is requested). Everything the app does
over time flows from here.

**How it works, in order.**
1. Housekeeping: notice layer changes, receive brush presets/thumbnails, advance a stabilized stroke.
2. First frame only: maximise the window, give the frameless window rounded corners (Windows),
   bind the pen tablet to this window.
3. Input: poll the pen tablet (`pen.poll`), record pen packets if a recording is on, sample modifier
   keys (`read_input`), handle keyboard shortcuts (`handle_keys` → `dispatch`).
4. Time: advance playback; while playing, ask for the next frame at `1/fps` seconds.
5. Keep edit buffers in step with the current frame: layer transform, selection, liquify, camera,
   perspective grids.
6. Background work: collect finished imports, saves, Krita link steps and preview frames; keep
   repainting while any is running.
7. Memory: free unused drawings (`collect_cells`), pack idle ones, pre-warm the next stroke's
   snapshot (`warm_pre`), and upload changed pixels to the GPU (`sync_textures`).
8. Draw the whole UI: `ui::shell::draw(self, ctx)`.
9. Undo stray keyboard focus left by a Tab press.

`ctx.request_repaint_after(d)` is how an immediate-mode app asks to be called again later even when
no input arrives — like `setTimeout(() => requestAnimationFrame(render), d)`.

See [architecture.md](../architecture.md#3-the-frame-loop) for the bigger picture.

## Free functions

### `sync_float_tex`
[src/app.rs:7651](../../src/app.rs#L7651)

```rust
fn sync_float_tex(
    ctx: &egui::Context,
    slot: &mut Option<TextureHandle>,
    sel: &Selection,
    retired: &mut Vec<TextureHandle>,
    fresh_full: &mut HashSet<egui::TextureId>,
)
```

Brings the texture of the floating (lifted, selected) pixels in `slot` up to date with `sel`: refilled
in place while the size is unchanged, otherwise a new texture is made and the old handle retired
(not dropped this frame). Whole refills are recorded in `fresh_full` so the canvas draws them the
safe way this frame.

### `premultiplied_image`
[src/app.rs:7672](../../src/app.rs#L7672)

```rust
pub(crate) fn premultiplied_image(size: [usize; 2], rgba: &[u8]) -> ColorImage
```

Builds an egui `ColorImage` from straight-alpha RGBA bytes, premultiplying each pixel *in gamma
(sRGB) space*. egui's own helper premultiplies in linear space, which makes soft antialiased edges
too bright (a white rim on light strokes) — so the project forbids it in `clippy.toml` and uses this.
`rgba.chunks_exact(4)` walks the bytes four at a time (one pixel each).

### `premultiplied`
[src/app.rs:7679](../../src/app.rs#L7679)

```rust
fn premultiplied(p: &[u8]) -> Color32
```

Premultiplies one RGBA pixel: fully transparent → transparent, fully opaque → unchanged, otherwise
each colour channel becomes `round(c × a / 255)`. Example: red 200 at alpha 128 becomes
`(200×128 + 127)/255 = 100`. See [premultiplied alpha](../glossary.md).

### `ghost_step`
[src/app.rs:7691](../../src/app.rs#L7691)

```rust
fn ghost_step(w: u32, h: u32) -> usize
```

The downscale factor that makes an onion-skin ghost fit within `GHOST_MAX_SIDE` pixels on its longer
side (a 4000-pixel cell with a 1024 max → step 4). `div_ceil` is integer division rounding up.

### `ghost_dims`
[src/app.rs:7696](../../src/app.rs#L7696)

```rust
pub(crate) fn ghost_dims(w: u32, h: u32) -> [usize; 2]
```

The ghost texture's size for a `w`×`h` cell: each side divided by `ghost_step`, rounded up.

### `ghost_image`
[src/app.rs:7709](../../src/app.rs#L7709)

```rust
pub(crate) fn ghost_image(c: &Canvas, tint: [u8; 3]) -> ColorImage
```

Builds a drawing's onion-skin ghost: a flat silhouette in colour `tint` that keeps the drawing's
alpha as its shape but replaces all its colours, box-filtered down to `ghost_dims`. This is why
onion skins read as blue (past) and red (future) — tinting the normal texture only *multiplies*
colours, and black line art times any tint stays black.

### `preview_color_image`
[src/app.rs:7747](../../src/app.rs#L7747)

```rust
fn preview_color_image(w: u32, h: u32, pixels: &[u8]) -> ColorImage
```

Makes a preview image for the import range dialog, shrinking it to at most 360 pixels wide (with a
triangle filter) so previews stay cheap.

## `impl AppState`

### `AppState::krita_notice`
[src/app.rs:7770](../../src/app.rs#L7770)

```rust
fn krita_notice(&mut self, msg: impl Into<String>, warn: bool)
```

Shows a message from the Krita link as a toast. Its display time is ~60 ms per character, at least 9 s
for warnings (shorter for notes) and at most 15 s. `msg: impl Into<String>` accepts either a `&str` or
a `String`.

### `AppState::resolve_krita`
[src/app.rs:7780](../../src/app.rs#L7780)

```rust
fn resolve_krita(&mut self) -> Option<PathBuf>
```

Finds Krita's executable: the remembered path or a usual install location, otherwise asks the user
with a file dialog. Whatever is found is remembered in `krita_path`. The `?` at the end returns
`None` if the user cancels.

### `AppState::edit_in_krita`
[src/app.rs:7799](../../src/app.rs#L7799)

```rust
pub fn edit_in_krita(&mut self, selected: usize, only: Option<usize>)
```

File → Edit in Krita (whole project, with layer `selected` selected in Krita) and the layer menu's
version (`only` = one layer sent alone). The first time it writes an animated `.kra` file and opens
it in Krita. With a link already live it starts a *re-send round*: Krita's latest save is read in
first, then the file is rewritten — and if the helper plugin is installed in Krita, Krita reloads it
in place. See [krita_link.rs](krita_link.md) and the [Krita plugin](../krita-plugin/animator_link/extension.md).

### `AppState::begin_send`
[src/app.rs:7827](../../src/app.rs#L7827)

```rust
fn begin_send(&mut self, selected: usize, only: Option<u64>, launch: Option<PathBuf>)
```

The part of `edit_in_krita` after Krita's location is known (`only` given as a layer uid). Tests call
this directly, so no real Krita is ever launched by `cargo test`.

### `AppState::start_send`
[src/app.rs:7859](../../src/app.rs#L7859)

```rust
fn start_send(&mut self, selected: usize, only: Option<u64>, launch: Option<PathBuf>)
```

Writes the project (or only layer `only`) as the linked `.kra` file on a worker thread; `poll_krita`
picks up the result and launches Krita if asked.

### `AppState::install_krita_helper`
[src/app.rs:7921](../../src/app.rs#L7921)

```rust
pub fn install_krita_helper(&mut self)
```

File → Install Krita helper: copies the small Python plugin into Krita's plugin folder, and reports
success or the error.

### `AppState::stop_krita_link`
[src/app.rs:7940](../../src/app.rs#L7940)

```rust
pub fn stop_krita_link(&mut self)
```

Ends the Krita link. The temporary `.kra` stays on disk, so a later Krita save can still be brought
in with Import .kra.

### `AppState::krita_linked`
[src/app.rs:7946](../../src/app.rs#L7946)

```rust
pub fn krita_linked(&self) -> bool
```

Whether a Krita link is live (so Edit in Krita would re-send rather than start a new link).

### `AppState::poll_krita`
[src/app.rs:7952](../../src/app.rs#L7952)

```rust
fn poll_krita(&mut self, ctx: &egui::Context)
```

Called every frame from `update`. Moves the Krita link along without blocking: lands a finished send
(launching Krita the first time), advances a re-send round, notices that Krita saved the file and
starts reading it, and folds a finished read into the project.

### `AppState::drive_round`
[src/app.rs:8062](../../src/app.rs#L8062)

```rust
fn drive_round(&mut self)
```

Performs one step of a re-send round (see `krita_link::Round`): wait for Krita's save to be read,
apply it, write the file again, ask the helper plugin to reload.

### `AppState::apply_krita_pull`
[src/app.rs:8132](../../src/app.rs#L8132)

```rust
fn apply_krita_pull(&mut self, res: Result<Pulled>)
```

Merges a Krita save that has been read (`res`) into the project as **one undo step**, changing only
what Krita changed; shows a notice on errors.

### `AppState::import_kra`
[src/app.rs:8173](../../src/app.rs#L8173)

```rust
pub fn import_kra(&mut self)
```

File → Import .kra…: lets the user pick a Krita file and decodes it on a worker thread; the paint
layers arrive via `on_kra_imported` as new layers under the active one, at the document's size.

### `AppState::on_kra_imported`
[src/app.rs:8192](../../src/app.rs#L8192)

```rust
fn on_kra_imported(&mut self, res: Result<KraDoc>)
```

Handles the result of File → Import .kra (decoded on a worker thread). On success it converts every
Krita layer into an `Imported` entry — each Krita drawing becomes a cell the size of the Krita
document, keyed on its frames, a blank lead-in key skipped, images too big for the GPU shrunk — drops
groups that ended up empty, then inserts all layers in one undoable `structural_edit` below the active
layer (inside its group unless the import brings groups of its own). Finally it reports what came in:
the count, skipped Krita-only layers, a frame-rate mismatch, and shrunk images.

### `AppState::export_kra`
[src/app.rs:8339](../../src/app.rs#L8339)

```rust
pub fn export_kra(&mut self)
```

File → Export .kra…: writes the whole project as an animated Krita document (on a worker thread).
One-way: it starts no link.

## Free functions

### `cap_canvas`
[src/app.rs:8376](../../src/app.rs#L8376)

```rust
fn cap_canvas(c: Canvas, max: u32) -> Canvas
```

Shrinks a canvas (keeping its aspect ratio) so neither side exceeds `max`; returns it unchanged if it
already fits. Keeps imported images within the GPU's texture size limit.

### `doc_to_cell`
[src/app.rs:8393](../../src/app.rs#L8393)

```rust
fn doc_to_cell(
    doc: Canvas,
    xform: &Transform,
    cell_w: u32,
    cell_h: u32,
    pw: u32,
    ph: u32,
) -> Canvas
```

Converts a document-sized image into the pixel grid of a cell placed by transform `xform` (cells may
be moved, scaled or bigger than the canvas). When the cell sits unmoved and is frame-sized — the usual
case — the image is returned as is.

## Tests

### `mod tests`

Run with `cargo test app::tests` · 135 tests

| Test | What it checks |
|---|---|
| [`a_clipped_layer_shows_only_over_its_base`](../../src/app.rs#L8477) | A layer clipped to the one below is uploaded opaque only where the base has ink and see-through elsewhere, and its plain (unclipped) texture is never made. |
| [`painting_the_base_redoes_just_that_rect_of_the_clip`](../../src/app.rs#L8492) | Inking a small block on the base layer re-cuts only that 10×10 patch of the clipped layer's texture (a partial upload), not the whole image. |
| [`a_flipped_or_turned_base_recuts_its_clipped_layer`](../../src/app.rs#L8520) | Flipping the base horizontally moves the visible part of the clipped layer to the other side; a quarter turn (which changes the base's size) still cuts cleanly, and undo brings the original back. |
| [`hiding_the_base_hides_the_clipped_layer_and_unclipping_restores_it`](../../src/app.rs#L8543) | A hidden base makes its clipped layer disappear too, and turning clipping off draws the layer plain again from its own cell texture. |
| [`a_stroke_on_a_clipped_layer_shows_while_it_is_drawn`](../../src/app.rs#L8559) | While a stroke is still in progress on a clipped layer, the clip texture is refreshed for just the stroke's rectangle, so the stroke is visible as you draw. |
| [`merging_a_clipped_layer_down_keeps_only_what_showed`](../../src/app.rs#L8576) | Merging a clipped layer into its base keeps only the part that was visible (red over the base) and drops the clipped-away part. |
| [`layer_keys_walk_the_stack_and_stop_at_the_ends`](../../src/app.rs#L8596) | The select above / below / top / bottom shortcuts move the active layer and stop at the ends without wrapping; "pick 1..4" counts rows from the top and ignores a row that does not exist. |
| [`moving_a_layer_by_key_is_one_undo_step_and_free_at_the_ends`](../../src/app.rs#L8621) | Moving a layer up by shortcut takes the selection with it and is one undo step; trying to move the top layer further up adds no undo entry. |
| [`the_layer_shortcuts_toggle_clip_and_alpha_lock`](../../src/app.rs#L8637) | The clip and alpha-lock shortcuts switch those flags on the current layer on and back off. |
| [`a_stroke_on_an_alpha_locked_layer_only_recolours`](../../src/app.rs#L8649) | On an alpha-locked layer an Ink stroke recolours pixels that already have ink but leaves empty pixels empty. |
| [`a_project_without_clipping_makes_no_clip_textures`](../../src/app.rs#L8673) | When no layer is clipped, a texture sync creates no clip textures at all (no cost for projects that do not use clipping). |
| [`dragging_sideways_shuts_the_gap_live_and_lands_as_one_undo`](../../src/app.rs#L8728) | Dragging sideways after pressing the bucket raises the gap-closing setting step by step (8 px dead zone, then one step per 12 px), re-runs the fill live so the leak through the hole stops, and the whole press ends as one undo step; the new gap value stays for the next press. |
| [`dragging_up_grows_the_expand_and_back_down_undoes_it`](../../src/app.rs#L8751) | Dragging up raises the fill's "expand" so the colour tucks under the line, and dragging back down lowers it again and gives the line back, still as one undo step. |
| [`a_plain_tap_fills_once`](../../src/app.rs#L8768) | A tap with a few pixels of wobble stays inside the dead zone: the settings do not change and exactly one fill lands. |
| [`cancelling_a_fill_press_leaves_nothing_behind`](../../src/app.rs#L8782) | Cancelling a fill press mid-drag removes the live fill entirely, and lifting the pen afterwards records no undo step. |
| [`reading_all_visible_layers_walls_the_fill_with_another_layers_lines`](../../src/app.rs#L8797) | With "fill reads all visible layers" on, a fill on an empty layer is stopped by line art on another layer; with it off (or with the line layer hidden) the fill floods everywhere. |
| [`tabbing_the_panels_back_leaves_the_shortcuts_live`](../../src/app.rs#L8824) | Pressing Tab to bring the panels back from mini-timeline mode must not leave a panel button focused (egui also reads Tab as "focus next"), otherwise the next shortcut (Q for Pencil) would be swallowed. |
| [`capturing_a_bare_modifier_in_settings`](../../src/app.rs#L8868) | When rebinding a hold action in Settings, a bare Shift is captured once it is released; a press action ignores a bare Shift and waits for a real key (Shift+I). |
| [`space_held_for_pan_does_not_click_a_focused_button`](../../src/app.rs#L8919) | When Space is bound to canvas pan, pressing it does not click a focused button; when it is not bound, Space clicks the button as normal. |
| [`a_pinned_wheel_gives_every_tool_one_colour`](../../src/app.rs#L8969) | Unpinned, each tool keeps its own colour; pinning the colour wheel shares one colour across all tools, unpinning lets them drift apart again, and the shortcut toggles the pin. |
| [`an_opacity_drag_sets_the_pick_in_one_undo_step`](../../src/app.rs#L8998) | Dragging the opacity slider with several layers picked sets every picked layer, and the whole drag (many frames, repeated commits) is one undo step. |
| [`last_layer_survives_adding_and_deleting_layers`](../../src/app.rs#L9021) | "Go to last layer" leads back to the layer you were on before adding a new one, and still leads there after the layer you jumped to is deleted. |
| [`tab_reaches_its_own_binding_from_a_focused_field`](../../src/app.rs#L9051) | With Tab bound to "last layer", pressing Tab while a text field has focus still runs that binding (and Shift+Tab does not), rather than a hard-wired panel toggle. |
| [`krita_save_lands_as_one_undo_step`](../../src/app.rs#L9114) | End to end on a real temp file: send to Krita, a simulated Krita save, the watcher noticing it, and the pull landing as one undo step; a reference background layer is never sent and never touched, and seeing the same save again changes nothing. |
| [`helper_round_reloads_in_place`](../../src/app.rs#L9244) | With the Krita helper plugin in use, a send asks the helper first, waits for "ready", writes the file, posts "sent", and on "reloaded" shows the "Updated in Krita" toast. |
| [`helper_saves_krita_edits_first`](../../src/app.rs#L9272) | If Krita has unsaved edits, the helper saves them and they arrive here as an undo step before the file is rewritten, so the reloaded file has both sides' changes. |
| [`silent_helper_falls_back_to_a_plain_send`](../../src/app.rs#L9295) | If the helper never answers, the send falls back to a plain file write and the toast says the helper didn't answer. |
| [`unreadable_krita_save_changes_nothing_then_recovers`](../../src/app.rs#L9315) | A half-written `.kra` is reported ("Couldn't read") and changes nothing; the next good save still comes in. |
| [`a_second_send_during_a_round_is_ignored`](../../src/app.rs#L9334) | Clicking Send again while a helper round is running sends no second request. |
| [`resend_brings_in_an_unread_krita_save_first`](../../src/app.rs#L9348) | Without the helper, a re-send first pulls in a Krita save the watcher hasn't read yet, so it is not overwritten. |
| [`layer_send_takes_only_that_layer`](../../src/app.rs#L9410) | Sending one layer from the layer menu writes only that layer to the `.kra`; Krita's later save changes that layer and leaves the unsent layer as it was. |
| [`layer_resend_narrows_a_whole_link`](../../src/app.rs#L9432) | When the whole project is linked, re-sending a single layer narrows Krita's file to that layer; the other layer stays here, unlinked, and is not deleted by Krita's next save. |
| [`layer_send_of_a_reference_layer_sends_nothing`](../../src/app.rs#L9457) | Sending a reference layer is refused before anything starts (no link is made) and the toast says it "stays here". |
| [`layer_gone_before_the_send_ends_the_round`](../../src/app.rs#L9468) | If the layer a re-send was for no longer exists, the round ends with a "gone" note, the file is left untouched, and the link keeps going. |
| [`dirty_regions_union_and_full_wins`](../../src/app.rs#L9494) | Combining dirty regions: clean plus a rect is that rect, two rects merge into their bounding box, and a "whole cell changed" mark swallows any rect. |
| [`uploads_patch_inside_the_texture_and_recrop_past_it`](../../src/app.rs#L9520) | `plan_upload` keeps a texture when nothing changed, sends a patch for an edit inside the cropped texture (or only the overlapping part of an eraser pass), and rebuilds (re-crops) when the drawing spreads past it; under a live stroke a small cell grows to its whole size. |
| [`a_stroke_texture_grows_with_room_to_spare`](../../src/app.rs#L9555) | On a huge (4320×7680) cell, a stroke's texture is the stroke's area grown by half again and at least `STROKE_ROOM`, never the whole cell at once; a brand-new blank drawing is not scanned, and going past the room rebuilds with room around the new extent. |
| [`a_cell_texture_holds_just_its_drawing`](../../src/app.rs#L9586) | A blank cell gets a 1×1 texture; an inked cell gets a texture cropped to the drawing plus a 2-pixel clear rim; small edits are patched at texture (not cell) coordinates; ink elsewhere re-crops around all of it. |
| [`a_stroke_past_the_drawing_takes_the_whole_cell_once`](../../src/app.rs#L9615) | On a normal-sized cell, a stroke that goes outside the existing drawing makes the texture cover the whole cell once, and the rest of the stroke is sent as small patches. |
| [`resting_on_a_drawing_readies_its_stroke_snapshot`](../../src/app.rs#L9640) | Pausing on a drawing (not just passing through) pre-takes its "before" snapshot so pen-down has nothing to copy, and undo of the stroke restores it exactly; under auto-key it clears the snapshot instead, and during playback it does nothing. |
| [`auto_key_ghosts_the_held_drawing_ahead_of_the_stroke`](../../src/app.rs#L9692) | With auto-key and onion skin on, resting on a frame builds the held drawing's onion "ghost" texture early, so when a stroke starts a new drawing the ghost is already there and not built again. |
| [`a_cell_image_is_its_premultiplied_pixels`](../../src/app.rs#L9724) | `cell_image` (the fast, possibly threaded reader) gives exactly the same image as premultiplying a copy of the pixels, for several rectangles, on packed and unpacked cells. |
| [`a_spreading_stroke_streams_and_pen_up_sends_only_the_rest`](../../src/app.rs#L9798) | On a 3000×4000 canvas, for both the ribbon and Krita brush engines, a long stroke's texture is re-made at most 6 times, always matches the cell pixel for pixel, and pen-up uploads only a small leftover (under 1/20 of the stroke's box). |
| [`the_budget_drops_the_least_recently_shown_first`](../../src/app.rs#L9842) | Over the texture memory budget, the least recently shown textures are dropped first and only until it fits; under budget nothing is dropped. |
| [`playback_drops_the_most_recently_shown_first`](../../src/app.rs#L9850) | During playback the order flips: the most recently shown texture goes first, because the loop comes back to the oldest one soonest. |
| [`the_budget_never_drops_what_is_on_screen`](../../src/app.rs#L9857) | Textures used on the current frame are never evicted, even when they alone exceed the budget. |
| [`ghosts_of_big_cells_are_small_and_keep_their_coverage`](../../src/app.rs#L9864) | Onion-skin ghosts of huge cells are shrunk (to at most `GHOST_MAX_SIDE`), small cells stay 1:1, a solid block stays solid and half coverage averages to half alpha. |
| [`a_packed_cell_makes_the_same_ghost`](../../src/app.rs#L9889) | A packed (memory-compacted) cell produces exactly the same ghost as the unpacked one, including drawings that start partway through a ghost texel and edge texels. |
| [`a_ghost_that_needs_no_shrinking_is_the_plain_silhouette`](../../src/app.rs#L9909) | When no shrinking is needed, the ghost's alpha is exactly each pixel's alpha (a tinted silhouette). |
| [`blank_means_no_coverage_anywhere`](../../src/app.rs#L9917) | `no_alpha` treats colour with zero alpha as blank, notices a non-zero alpha in the very last pixel, and works on unaligned slices too. |
| [`a_keyed_grid_tweens_as_the_frame_moves_and_undo_takes_a_key_back`](../../src/app.rs#L9958) | A grid keyed at frames 0 and 20 sits halfway (linear tween) at frame 10; undo removes the second key and the grid returns to the single key's pose, redo brings it back. |
| [`inserting_frames_before_moves_grid_keys_in_one_undo`](../../src/app.rs#L9983) | Inserting frames before the playhead shifts grid keys along with camera keys and drawings in one undo step; deleting and adding frames shift them too, and a key on a deleted frame goes with it. |
| [`dragging_a_keyed_grid_keys_the_frame_as_one_undo_step`](../../src/app.rs#L10022) | With auto-key on, dragging a keyed grid adds a key on the current frame (one undo step) so the move sticks; with auto-key off the drag only lasts until the frame changes, like the camera. |
| [`a_grid_without_keys_just_moves`](../../src/app.rs#L10058) | Dragging a grid that has no keys moves it without creating any key, and the drag is still one undo step. |
| [`a_grid_on_the_camera_rides_it_and_switching_keeps_it_in_place`](../../src/app.rs#L10070) | Switching a grid to follow the camera does not make it jump; it then moves with the camera and, when the camera zooms in, shrinks in document space so it keeps its size in the shot. |
| [`a_grid_on_a_layer_finds_it_again_by_name_next_session`](../../src/app.rs#L10090) | A grid following a layer moves with that layer's transform, and when its stored layer id is lost (as after a restart) it finds the layer again by name. |
| [`each_file_keeps_its_own_grids`](../../src/app.rs#L10110) | Grids are remembered per `.anim` path: switching files swaps grid sets, Save As copies them to the new path while the old file keeps its own, a new project starts with one default grid, and the store goes into the preferences. |
| [`a_walls_top_corner_drags_its_height_and_nothing_else_moves_it`](../../src/app.rs#L10143) | Dragging a wall grid's top corner changes only its height while its base stays on the floor; pressing inside the wall selects it without starting a move; undo restores the height. |
| [`dragging_a_grid_corner_moves_only_that_corner`](../../src/app.rs#L10171) | Dragging a corner handle moves just that corner (the other three stay put), and moves after release do nothing. |
| [`a_locked_grid_is_selected_but_not_moved`](../../src/app.rs#L10189) | Pressing a locked grid makes it the active grid, but dragging does not move it. |
| [`pressing_another_grid_makes_it_active`](../../src/app.rs#L10204) | Pressing inside a second grid makes it active and drags only it; pressing empty canvas grabs nothing and keeps the current selection. |
| [`a_folding_corner_drag_is_refused`](../../src/app.rs#L10223) | A corner drag that would fold the grid's quad (cross the far diagonal) is ignored and the last valid shape is kept. |
| [`an_extra_vp_slides_along_the_horizon_until_pulled_off_and_sticks_back`](../../src/app.rs#L10234) | An extra vanishing point stays on the horizon line while dragged near it, comes free when pulled well away, and snaps back onto the horizon when brought close again. |
| [`moving_a_grid_takes_its_extra_vps_along`](../../src/app.rs#L10266) | Moving a whole grid moves its extra vanishing points by the same amount. |
| [`a_stroke_heading_for_an_extra_vp_locks_onto_it`](../../src/app.rs#L10280) | With snap on, a stroke heading toward an extra vanishing point locks onto the line through it rather than the grid's rows or columns. |
| [`snap_locks_a_stroke_to_one_grid_direction`](../../src/app.rs#L10301) | With snap on, a stroke is held at its start until its direction is clear, then locks to one grid direction and stays locked even if the hand drifts; with snap off points pass straight through. |
| [`a_near_vertical_stroke_follows_the_vanishing_point_not_the_vertical`](../../src/app.rs#L10322) | On a one-point floor grid, a nearly vertical stroke leans toward the vanishing point along the grid's column; with "snap verticals" on, it stays truly vertical. |
| [`a_rect_lies_on_the_grid_when_snap_is_on`](../../src/app.rs#L10375) | With snap on, a Shape-tool rectangle is drawn lying on the perspective floor (edges follow the grid's rows and leaning columns), not as a flat screen rectangle. |
| [`with_snap_off_a_rect_stays_flat`](../../src/app.rs#L10387) | With snap off, the same drag draws a plain flat rectangle. |
| [`a_rect_lies_on_the_grid_on_a_transformed_layer_too`](../../src/app.rs#L10396) | On a scaled, rotated and moved layer, the snapped rectangle still lands in the same place on screen as on an untransformed layer. |
| [`shift_squares_a_rect_up_in_grid_cells`](../../src/app.rs#L10414) | Holding Shift while drawing a snapped rectangle makes it square in grid cells (equal number of columns and rows); letting go of Shift frees it again. |
| [`a_shape_line_snaps_like_a_stroke`](../../src/app.rs#L10433) | A Shape-tool line snaps to one grid direction like a pen stroke (it does not lie on the plane), while a rectangle is not direction-locked but lies on the plane instead. |
| [`shift_down_at_the_press_locks_only_once_pressed_again`](../../src/app.rs#L10474) | If Shift was already down when the drag began (it chose a pan or selection mode), it does not lock an axis until it is released and pressed again; if it goes down partway, it locks at once. |
| [`shift_partway_turns_a_shape_line_to_a_fifteen_degree_step`](../../src/app.rs#L10492) | Pressing Shift mid-drag snaps a Shape line drawn at 50° to the nearest 15° step (45°), and the final line is drawn along that angle. |
| [`a_held_line_is_level_on_the_canvas_or_the_screen`](../../src/app.rs#L10512) | The lock's "level" direction is the canvas horizontal, or the screen horizontal when the view is rotated and screen axes are chosen, and it is turned back by the layer's own rotation in cell pixels. |
| [`shift_partway_squares_a_flat_rect`](../../src/app.rs#L10528) | Pressing Shift mid-drag turns a flat rectangle into a square as tall as it is wide. |
| [`a_mid_key_down_at_the_press_counts_only_once_pressed_again`](../../src/app.rs#L10555) | Like Shift, the move and centre keys held when the drag began do not count until released and pressed again mid-drag. |
| [`the_move_key_partway_slides_a_flat_rect`](../../src/app.rs#L10566) | Holding the move key (Space) mid-drag slides the rectangle without resizing it; releasing it resumes growing from where it was slid to, and the result is drawn there, not at the press. |
| [`the_centre_key_partway_grows_a_rect_from_the_press`](../../src/app.rs#L10584) | Holding the centre key (Alt) makes the rectangle grow out from the press point in both directions, and with Shift as well it becomes a square centred on the press. |
| [`a_centred_line_runs_both_ways_from_the_press`](../../src/app.rs#L10602) | A centred Shape line runs equally both ways from the press point, and with the move key too its middle follows the pointer. |
| [`the_move_key_slides_a_rect_along_its_grid`](../../src/app.rs#L10617) | Sliding a perspective-snapped rectangle keeps it the same number of grid cells across while it gets smaller on screen as it moves up the floor toward the horizon. |
| [`a_snapped_line_slid_stays_snapped_from_where_it_went`](../../src/app.rs#L10640) | Sliding a snapped line moves it to another grid row, and after the key is released it stays snapped to that new row. |
| [`shift_partway_holds_a_stroke_to_an_axis_from_where_it_was`](../../src/app.rs#L10663) | Pressing Shift mid-stroke holds the stroke still until a direction shows, then keeps it on that axis from the point where Shift went down; releasing frees it, and a new press starts a new hold from the current point. |
| [`shift_partway_holds_a_grid_move_to_an_axis`](../../src/app.rs#L10685) | Pressing Shift while moving a perspective grid restricts the move to horizontal or vertical, whichever the pointer is going along. |
| [`next_grid_cycles_the_visible_grids_and_wraps`](../../src/app.rs#L10699) | The "next grid" shortcut cycles through visible grids, skipping hidden ones and wrapping around; with only one visible it stays put, and with no grids it does not crash. |
| [`selecting_the_tool_brings_back_a_grid`](../../src/app.rs#L10722) | Choosing the Perspective tool when there are no grids creates one and makes it active. |
| [`write_prefs_fixture`](../../src/app.rs#L10788) | Ignored by default (`#[ignore]`); run by hand with `cargo test write_prefs_fixture -- --ignored` to regenerate the old-format `src/testdata/ui_prefs_v1.ron` fixture. It checks nothing itself. |
| [`prefs_written_before_the_brush_library_still_load`](../../src/app.rs#L10798) | A `ui.ron` from before the Krita brush library still loads with every saved value intact (one unreadable field would silently reset all preferences); newer fields get defaults, old onion sliders become the onion strip, and old fills stay grain-free. |
| [`the_active_brush_is_saved_with_its_latest_changes`](../../src/app.rs#L10836) | Saving stores the active tool's brush as it is right now, not as it was at the last tool switch. |
| [`each_pen_s_smoothing_outlasts_a_restart`](../../src/app.rs#L10851) | Turning the stabilizer on for Ink only is saved per pen: Pencil and Eraser stay on Basic smoothing after a reload. |
| [`the_pen_slots_outlast_a_restart`](../../src/app.rs#L10866) | The pen slots outlast a restart: how many, and each one's name, icon, brush, smoothing and sizes. The app comes back on the first. |
| [`the_first_two_slots_are_written_where_an_older_build_reads_them`](../../src/app.rs#L10893) | A build from before the slots finds the first two where the pencil and the ink always were, and the eraser where it was; and prefs from such a build come back the same. |
| [`prefs_from_before_the_pen_slots_make_the_pencil_and_ink_slots`](../../src/app.rs#L10928) | A prefs file from before the pen slots gives its pencil and ink as the first two slots, and every other tool its own brush, as they were. |
| [`hidden_panels_and_the_layout_reset_outlast_a_restart`](../../src/app.rs#L10947) | A panel layout reset shows every panel again; hidden panels and the reset counter are saved and come back after a reload. |
| [`the_color_scheme_and_custom_colors_outlast_a_restart`](../../src/app.rs#L10965) | The default theme is Graphite; a Custom theme and its colours are saved and restored. |
| [`a_screen_pick_keeps_the_brush`](../../src/app.rs#L10981) | Picking a colour from the screen changes only the brush colour, not its engine or size. |
| [`a_new_project_keeps_the_brushes`](../../src/app.rs#L10994) | Starting a new project keeps every tool's brush settings and switches back to the Pencil. |
| [`shape_shares_its_size_with_the_last_pen`](../../src/app.rs#L11008) | The Shape tool takes the size of the pen used last, and a size set on Shape goes back to that pen; the other pen keeps its own size, and Shape keeps its own hard edge (only size is shared). |
| [`size_keys_keep_shape_and_the_pen_in_step`](../../src/app.rs#L11032) | With the full UI drawn, the `[` / `]` size keys change Ink's size and Shape picks it up, and a size change on Shape goes back to Ink. |
| [`a_shape_thickness_is_saved_with_its_pen`](../../src/app.rs#L11083) | A thickness set on Shape is saved with the pen it follows, so after a restart both that pen and Shape have it. |
| [`a_screen_pick_on_shape_keeps_the_pen_size`](../../src/app.rs#L11099) | Picking a colour from the screen while on Shape stays on Shape and does not change its size (or the pen's). |
| [`a_stabilized_stroke_finishes_at_pen_up`](../../src/app.rs#L11115) | With the Stabilizer on, lifting the pen finishes the line to where the pen lifted, even if no UI frame ran between pen events to advance it. |
| [`each_pen_keeps_its_own_smoothing`](../../src/app.rs#L11141) | Turning the stabilizer on for Ink does not affect Pencil or Eraser strokes, and Fill has no smoothing at all. |
| [`the_smoothing_keys_change_the_pen_in_hand`](../../src/app.rs#L11166) | The Stabilizer key toggles the current pen to Stabilizer and back to what it had; the cycle key steps through Weighted, Stabilizer, None, Basic; neither changes other pens or does anything on a tool without smoothing. |
| [`brushes_saved_before_the_perspective_tool_survive`](../../src/app.rs#L11194) | A saved brush list from before the Perspective tool existed (7 entries) is still restored, while a list of any other wrong length is thrown away. |
| [`perspective_prefs_round_trip_and_default_when_missing`](../../src/app.rs#L11207) | Perspective settings survive a save/load round trip, and preferences from before grids existed load with the default grid. |
| [`the_backdrop_is_kept_between_runs_and_across_new_projects`](../../src/app.rs#L11222) | The canvas backdrop colour and opacity are saved, kept across File > New, and default correctly in old preferences; an old "Polygon" selection shape opens as the freehand lasso, and an old checkerboard setting still loads. |
| [`the_onion_strip_round_trips_and_old_ranges_carry_over`](../../src/app.rs#L11247) | Per-step onion show/hide and opacity survive a round trip, and an old "prev = 3" setting turns into three lit steps back (and one forward). |
| [`the_magic_select_settings_round_trip_and_old_prefs_get_defaults`](../../src/app.rs#L11261) | Magic-wand selection settings survive a round trip and the app's own save; preferences without them, or with only some fields, fill in defaults. |
| [`onion_layers_are_the_active_one_and_those_switched_on`](../../src/app.rs#L11294) | Onion skin shows on the active layer plus layers with their own onion switch on, but not on hidden layers, not during playback, and not when onion skin is off. |
| [`saving_a_preset_needs_a_name_and_replaces_one_it_matches`](../../src/app.rs#L11321) | Saving a New Project preset needs a non-blank name (trimmed); saving under an existing name (any case) overwrites that preset in place and keeps its star. |
| [`renaming_a_preset_rejects_blank_and_taken_names`](../../src/app.rs#L11341) | Renaming a preset refuses a blank name or one another preset already has, but allows changing the case of its own name. |
| [`only_one_preset_is_starred`](../../src/app.rs#L11354) | Only one preset can be the starred default: starring another moves the star, starring the starred one clears it, and deleting it leaves none starred. |
| [`new_project_opens_on_the_starred_preset_as_saved`](../../src/app.rs#L11372) | With no starred preset the New Project dialog starts at the current project's size; with one it starts at that preset, and applying a preset keeps its own orientation. |
| [`presets_round_trip_and_default_to_none_when_missing`](../../src/app.rs#L11394) | Presets survive a save/load round trip, and preferences from before presets existed load with none. |
| [`a_pinned_drawing_is_ghosted_once_in_the_pin_colour`](../../src/app.rs#L11413) | A drawing pinned in the onion skin is ghosted once in the pin's colour and not again as a normal onion step; switching onion skin off hides pins too. |
| [`only_changed_pixels_are_marked_for_upload`](../../src/app.rs#L11445) | Timeline-structure edits (adding frames and keys) mark no texture for re-upload, and a one-pixel edit marks only that pixel's rectangle. |
| [`undo_of_a_structural_edit_uploads_nothing`](../../src/app.rs#L11468) | Undoing a structural edit (adding a frame) marks no cell for re-upload. |
| [`undo_of_a_stroke_uploads_its_rect`](../../src/app.rs#L11480) | Undoing a pixel patch marks only that patch's rectangle for re-upload, not the whole cell. |
| [`fade_others_splits_above_and_below_the_active_layer`](../../src/app.rs#L11512) | With "fade others" on, layers below the active one are drawn at the "below" opacity, layers above at the "above" opacity, and the active layer at full strength. |
| [`fade_others_spares_reference_and_lines_from_layers`](../../src/app.rs#L11519) | Reference layers, and the layer the active layer takes its lines from, are not faded; another layer's lines link does not count. |
| [`the_key_link_toggle_links_the_layer_below`](../../src/app.rs#L11534) | The key-link shortcut links the current layer's keys to the nearest drawing layer below (skipping a group header), pressing it again unlinks, the bottom layer has nothing to link to, and linking is not an undo step. |
| [`a_cleanup_layer_is_linked_and_one_undo`](../../src/app.rs#L11553) | "New cleanup layer" adds a layer directly above the rough, named "<rough> clean", key-linked to it and made active, and one undo takes it all back. |
| [`fade_others_lifts_while_playing_or_switched_off`](../../src/app.rs#L11572) | During playback, or once the toggle is switched off, every layer shows at full opacity. |
| [`a_dropped_drawing_lives_as_long_as_undo_can_reach_it`](../../src/app.rs#L11585) | A deleted layer's drawing is kept while undo can still restore it, and is freed (made a tombstone, texture dropped) once that undo step has fallen out of history, without renumbering other cells. |
| [`packing_changes_nothing_on_screen_or_in_export`](../../src/app.rs#L11615) | Packing an idle cell to save memory leaves its uploaded texture and its flattened export pixels exactly the same. |
| [`the_packer_leaves_the_drawing_being_drawn_alone`](../../src/app.rs#L11645) | The idle-cell packer never packs the cell a stroke is drawing into, even off the active slot, but does pack it once the stroke is done. |
| [`play_shortcut_starts_from_the_current_frame`](../../src/app.rs#L11664) | Starting playback by shortcut steps from the frame the playhead is on: the first tick only starts the clock rather than jumping by however long it was paused. |
| [`stopping_goes_back_to_the_start_frame_unless_paused_here`](../../src/app.rs#L11684) | Play/Pause returns to the frame playback started on when stopped, while the "pause here" action stops on the current frame. |
| [`running_out_with_the_loop_off_goes_back_to_the_start_frame`](../../src/app.rs#L11709) | With looping off, playback that reaches the end of the range stops and returns to the start frame. |
| [`fade_others_starts_off`](../../src/app.rs#L11725) | "Fade others" is session-only: a fresh state always starts with it off. |
| [`perf_big_layer`](../../src/app.rs#L11734) | `#[ignore]` timing probe, not pass/fail: compares old and new ways of uploading, ghosting, blank-scanning and snapshotting one cell of a 3×-wide 4K frame. Run with `cargo test --release perf_big_layer -- --ignored --nocapture`. |
| [`perf_liquify`](../../src/app.rs#L11912) | `#[ignore]` timing probe: Liquify pen-down, move, per-frame and apply times for several canvas and brush sizes. Run with `cargo test --release perf_liquify -- --ignored --nocapture`. |
| [`perf_big_canvas_stroke`](../../src/app.rs#L11984) | `#[ignore]` timing probe: stage-by-stage stroke timings (pen-down, stroke frames, pen-up, upload and undo sizes) on a big 4320×7680 canvas for both brush engines, or on the `.anim` named by the `ANIM_PROBE` environment variable. |
| [`perf_user_strokes`](../../src/app.rs#L12142) | The user's own pens (from a `ui_prefs` blob) on a real project, as wall-clock strokes. Not a pass/fail test — run it for the numbers: `PREFS_PROBE=<ui_prefs.ron> ANIM_PROBE=<x.anim> cargo test --release perf_user_strokes -- --ignored --nocapture` |

**Test helpers** (used only by the tests above):

- [`clip_state`](../../src/app.rs#L8433) — builds a two-layer state: layer 0 inked black on its left half, layer 1 solid red and clipped to it.
- [`sync`](../../src/app.rs#L8458) — runs one headless egui frame of `sync_textures` (with the app's 8192 texture limit) and returns what egui was asked to upload.
- [`uploaded_alpha`](../../src/app.rs#L8468) — reads the alpha of one pixel of an uploaded texture from that upload list.
- [`ink_box`](../../src/app.rs#L8687) — inks a 1-px box outline from 100 to 200 on a layer's current drawing, with a 4-px hole in its top edge.
- [`fill_state`](../../src/app.rs#L8704) — a state on the Fill tool, red, gap and expand at 0, with the leaky box inked on layer 0.
- [`px`](../../src/app.rs#L8714) — reads one RGBA pixel of the current drawing.
- [`fill_press`](../../src/app.rs#L8722) — presses the bucket at a cell point and anchors the press at a screen point (so later drags measure from it).
- [`linked`](../../src/app.rs#L9189) — a state with a live Krita link on a temp `.kra` file after a first send, with zero poll delays and no Krita started.
- [`pump`](../../src/app.rs#L9204) — runs the per-frame Krita step until a condition holds, failing after 10 seconds.
- [`to_krita`](../../src/app.rs#L9214) — reads the last message the app left for the Krita helper (its sequence number and text).
- [`stage`](../../src/app.rs#L9219) — the current stage of the Krita send round, if one is running.
- [`krita_saves`](../../src/app.rs#L9224) — writes a project to the `.kra` path the way Krita would save it, using the link's layer uuids.
- [`first_pixel_on_disk`](../../src/app.rs#L9232) — reads pixel 0 of layer 0's first drawing from the `.kra` on disk.
- [`two_layer_link`](../../src/app.rs#L9369) — a state with two drawn layers and an unsent, helper-less Krita link; returns both layer uids.
- [`names_on_disk`](../../src/app.rs#L9389) — the layer names in the `.kra` on disk.
- [`krita_paints_only`](../../src/app.rs#L9395) — simulates Krita painting pixel 0 of one layer and saving a file holding only that layer.
- [`layer_by_uid`](../../src/app.rs#L9403) — finds a layer by its uid (panics if it is gone).
- [`rect`](../../src/app.rs#L9482) — shorthand to build a `DirtyRect` from min/max corners.
- [`ink`](../../src/app.rs#L9505) — inks a rectangle of a cell opaque black and reports it dirty the way a tool does.
- [`apply`](../../src/app.rs#L9754) — applies one upload list (full images, patches, frees) to the copy and returns how many pixels were sent.
- [`assert_shows`](../../src/app.rs#L9782) — asserts the texture a layer shows matches its cell pixel for pixel and that every inked pixel lies inside it.
- [`entry`](../../src/app.rs#L9832) — builds a texture-cache entry (cell id, last-used frame, size in MB) for the budget tests.
- [`perspective_state`](../../src/app.rs#L9930) — a state on the Perspective tool with one 100-px square grid at (100, 100) and a 1:1 view.
- [`doc_corners`](../../src/app.rs#L9940) — a grid's four corners in document pixels.
- [`near`](../../src/app.rs#L9945) — true when two points are within 0.01 px of each other.
- [`grid_centre_doc`](../../src/app.rs#L9951) — a grid's centre in document pixels on the current frame (after keys and follow links).
- [`shape_state`](../../src/app.rs#L10348) — a state on the Shape tool drawing the given shape kind with snap on over the `FLOOR` grid.
- [`drag_shape`](../../src/app.rs#L10361) — presses at one cell point, drags to another, and releases.
- [`alpha`](../../src/app.rs#L10367) — the alpha of the current drawing at a point.
- [`flat_shapes`](../../src/app.rs#L10457) — a state on the Shape tool with no grid to snap to, 1:1 view.
- [`shift`](../../src/app.rs#L10466) — egui modifier state with Shift up or down.
- [`shape_to`](../../src/app.rs#L10543) — feeds a Shape drag's next point through `stroke_point_doc`, as the canvas does (where the move key slides the shape).
- [`shape_from`](../../src/app.rs#L10549) — starts a Shape drag through `snap_begin`, as the canvas does.
- [`get_string`](../../src/app.rs#L10736) — Get string.
- [`set_string`](../../src/app.rs#L10739) — Set string.
- [`flush`](../../src/app.rs#L10742) — Flush.
- [`fixture_brushes`](../../src/app.rs#L10749) — the brushes stored in that fixture, every slot changed from its default (including the Krita Pencil-5).
- [`fixture_prefs`](../../src/app.rs#L10763) — the full `UiPrefs` value the fixture was written from.
- [`save_as`](../../src/app.rs#L11313) — fills the New Project dialog with a size and fps and saves it as a named preset.
- [`faded_stack`](../../src/app.rs#L11500) — four layers, the second active, "fade others" on (0.2 above, 0.5 below).
- [`probe_project`](../../src/app.rs#L11800) — builds an `n`-frame project of a given size, each frame keyed with its own packed drawing of thick lines (for the perf probes).
- [`probe_frame`](../../src/app.rs#L11829) — runs one full app frame headless (housekeeping, texture sync, whole UI, tessellation) and returns its time and bytes sent to the GPU.
- [`probe_stroke`](../../src/app.rs#L11855) — draws a straight stroke four samples per frame and prints the cost of each stage.
- [`wall_stroke`](../../src/app.rs#L12042) — One wall-clock stroke: samples come due at `hz` against real time, so a slow frame takes more of them, as in the app. Prints the frame-time spread and the worst frames, phase by phase.

**Test-only types and constants:** [`FILL_RED`](../../src/app.rs#L8683), [`GpuCopy`](../../src/app.rs#L9751), [`FLOOR`](../../src/app.rs#L10345), [`MemStorage`](../../src/app.rs#L10733), [`PREFS_FIXTURE`](../../src/app.rs#L10745)

### `mod camera_rig_tests`

Run with `cargo test app::camera_rig_tests` · 25 tests

| Test | What it checks |
|---|---|
| [`a_camera_the_size_of_the_canvas_has_no_size_of_its_own`](../../src/app.rs#L12201) | The output size follows the canvas until a camera size is set; setting it back to the canvas size stores "no size of its own", silly sizes are clamped (at least 1, at most `MAX_CAMERA_SIDE`), and `None` returns to the canvas. |
| [`a_new_size_leaves_zoom_and_keys_alone`](../../src/app.rs#L12218) | Changing the camera size is not a camera move: zoom and camera keys stay the same. |
| [`undo_never_changes_the_camera_s_size`](../../src/app.rs#L12229) | Undo and redo take back camera keys but never change the camera's size (the size is session-only, not part of history). |
| [`the_last_camera_size_is_kept_between_runs`](../../src/app.rs#L12244) | The last camera size set is remembered (even after handing the size back to the canvas) and saved in preferences, but the next run does not apply it to the project; old preferences without it still load. |
| [`a_free_camera_box_sets_the_camera_size`](../../src/app.rs#L12264) | A free-shape camera box sets the camera to the box's size in even pixels, only on release, framed at zoom 1 and centred on the box, and keys it; a thin drag gets at least `MIN_FREE_CAMERA_SIDE`, a box hanging off the canvas is held on it, and with free shape off the box keeps the camera's shape. |
| [`a_camera_box_grows_from_its_middle_and_slides`](../../src/app.rs#L12307) | Holding the centre key makes the press point the camera box's middle (free or shape-locked), and the move key slides the box. |
| [`free_camera_box_and_panel_padding_are_kept_between_runs`](../../src/app.rs#L12332) | The "free camera box" switch and the panel padding are saved and restored with preferences. |
| [`brush_sizes_are_kept_between_runs`](../../src/app.rs#L12349) | Each brush's pinned sizes are saved per tool: an Eraser pin comes back for the Eraser and not for Ink. |
| [`keep_inside_is_on_for_every_project_unless_its_camera_leaves_the_canvas`](../../src/app.rs#L12368) | "Keep inside canvas" is on at launch, for new projects, and for opened projects whose camera stays on the canvas; a project whose camera is framed past the edge opens with it off, unchanged, and a toast says so. |
| [`dragging_inside_the_frame_moves_the_camera_in_one_undo_step`](../../src/app.rs#L12400) | In camera edit mode, dragging inside the frame pans the camera by the drag distance as one undo step. |
| [`shift_partway_holds_a_camera_move_to_an_axis`](../../src/app.rs#L12414) | Pressing Shift partway through a camera drag keeps the move to one axis. |
| [`dragging_a_corner_zooms_and_past_a_corner_rolls`](../../src/app.rs#L12426) | Dragging a frame corner outward zooms the camera (twice as far out gives zoom 0.5); grabbing just past a corner rotates (rolls) it instead, each drag one undo step. |
| [`a_press_off_the_frame_or_outside_edit_mode_is_left_to_the_tool`](../../src/app.rs#L12448) | A press outside the frame, or any press with camera edit off, is not taken by the camera: nothing moves and no undo step is recorded. |
| [`a_click_on_the_frame_that_moves_nothing_is_no_undo_step`](../../src/app.rs#L12461) | Clicking the frame without moving records no undo step. |
| [`a_dragged_frame_is_held_on_the_canvas_when_asked`](../../src/app.rs#L12470) | With "keep inside canvas" on, dragging the frame far away stops it at the canvas edge. |
| [`the_view_holds_still_under_a_dragged_frame`](../../src/app.rs#L12483) | When looking through the camera, the view does not follow the frame during the drag (that would move the canvas under the cursor) and catches up on release. |
| [`at_one_loop_the_camera_plays_in_step_with_the_frames`](../../src/app.rs#L12535) | With the camera move set to one loop, playback shows each frame with its own keyed camera, round and round. |
| [`over_three_loops_the_drawings_repeat_under_one_move`](../../src/app.rs#L12544) | With three loops, the drawings repeat three times while the camera makes one continuous move from start to end, then everything starts over. |
| [`pausing_a_stretched_move_parks_the_playhead_where_the_camera_is`](../../src/app.rs#L12560) | Stopping during a stretched move leaves the camera where it was and moves the playhead to the matching (nearest) frame of the move. |
| [`a_camera_with_no_move_is_never_stretched`](../../src/app.rs#L12585) | A camera with only one key has no move to stretch, so the loops setting is ignored and playback is normal. |
| [`copy_frame_takes_an_unkeyed_nudge_of_the_camera`](../../src/app.rs#L12612) | Copy frame copies what the camera shows, including an unkeyed nudge, rather than the keyed camera that Save PNG and exports use. |
| [`copy_frame_mid_stretched_move_takes_where_the_move_has_got`](../../src/app.rs#L12629) | During playback of a stretched move, Copy frame uses the camera where the move has got to, not the camera keyed on the drawing showing. |
| [`an_export_of_a_stretched_move_gives_each_frame_its_camera`](../../src/app.rs#L12644) | Exporting a move stretched over three loops repeats the frames three times with a camera per output frame (and the right frame count and length); the camera's ping-pong fill replaces the dialog's own; a sprite sheet stays one cell per frame. |
| [`a_stretched_export_keeps_the_camera_s_other_options`](../../src/app.rs#L12681) | A stretched export still applies the camera's size and "keep inside canvas", so every exported camera stays on the canvas. |
| [`opening_a_project_starts_the_camera_on_its_canvas`](../../src/app.rs#L12694) | Opening a project resets the session camera size, so the output is the new project's canvas size. |

**Test helpers** (used only by the tests above):

- [`framed`](../../src/app.rs#L12390) — a 1280×720 project with a 640×360 camera frame in the middle (x 320..960, y 180..540), 1:1 view, camera edit on.
- [`panning`](../../src/app.rs#L12496) — a 64×64, 10 fps, five-frame project with the camera keyed from x 0 on the first frame to x 40 on the last, looping on.
- [`play`](../../src/app.rs#L12517) — advances the app's playback clock a number of frames, as `update` does, and returns the frame and camera x shown after each.
- [`mark`](../../src/app.rs#L12596) — draws a black vertical bar at x 40..44 on the first drawing so a camera pan visibly moves something.

### `mod pen_tex_tests`

Run with `cargo test app::pen_tex_tests` · 4 tests

| Test | What it checks |
|---|---|
| [`a_resting_pen_grows_its_drawing_whole_and_a_stroke_only_patches_it`](../../src/app.rs#L12797) | A resting pen grows its drawing whole and a stroke only patches it. |
| [`moving_on_crops_the_drawing_left_behind`](../../src/app.rs#L12814) | Only the pen's drawing is kept whole: one per drawing outgrew the texture budget in playback. |
| [`auto_key_s_new_drawing_takes_the_spare`](../../src/app.rs#L12840) | With auto-key, the press makes a new drawing: it takes the clear whole texture readied for it, and its first stroke only patches it. |
| [`playback_never_remakes_the_pen_s_drawing_whole`](../../src/app.rs#L12862) | Playing only shows the drawing: one the budget dropped comes back cropped, not whole. |

**Test helpers** (used only by the tests above):

- [`state`](../../src/app.rs#L12713) — Three drawings, each a small blob in the middle, so each texture is cropped well inside its cell.
- [`frame`](../../src/app.rs#L12740) — One frame as `update` runs it, uploads and all.
- [`rest`](../../src/app.rs#L12757) — Longer than `WARM_REST` at egui's 60 Hz default step.
- [`at`](../../src/app.rs#L12763) — At.
- [`stroke_without_rebuild`](../../src/app.rs#L12775) — From the blob to the far corner, a frame per sample. Panics if the drawing's texture is re-made or re-sent whole on the way; returns how many frames sent it a patch.

**Test-only types and constants:** [`W`](../../src/app.rs#L12708), [`H`](../../src/app.rs#L12709)
