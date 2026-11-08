# `src/ui/shell.rs` — the whole app window: panels, menus, canvas input and overlays

> **Source:** [src/ui/shell.rs](../../../src/ui/shell.rs) · **Module:** `crate::ui::shell` · **Size:** 15598 lines · Line links as of commit `67a7cb9`

## What this file is for

This is the file that **draws everything you see** in Animator, every single frame. The
top bar with its menus, the floating panels (Tools, Brush, Layers, Onion, Camera,
X-sheet, Timeline), the Settings and export dialogs, the little "Saved" toasts, the
canvas in the middle of the window and everything painted over it (the brush ring,
selection outlines, perspective grids, the camera frame) all come from here.

It is also where **mouse and pen input on the canvas is turned into actions**. The
function [`draw`](#draw) decides, on every press, what a drag means — a pencil stroke,
a pan of the view, a lasso selection, a camera move, a colour pick from the swatch pie —
and feeds each frame's pointer positions to the matching method on `AppState` (the big
struct in [app.rs](../app.md) that holds the whole program's state). The file itself owns
almost no data: it reads `AppState`, draws widgets for it, and writes the user's changes
straight back into it.

Think of it as one giant React `App` component that re-renders 60 times a second, except
there is no virtual DOM and no component state: each function is called, draws its part
of the screen right away, and returns. That style is called *immediate-mode UI*, and the
library doing it is **egui** (see "Rust concepts" below).

The file is long (about 9,600 lines of code plus 5,400 lines of tests), so its
documentation is split into parts. This first part covers the entry point `draw`, the
panel window machinery, the Tools / Brush / Timeline / Onion panels, the tablet
diagnostics section, the mini timeline and the first half of the Camera panel.

## How it connects

- **Used by:**
  - [app.rs](../app.md) — `AppState::update` (the per-frame callback eframe calls)
    ends with `ui::shell::draw(self, ctx)` at
    [src/app.rs:7542](../../../src/app.rs#L7542). That is the **one entry point** into
    this file. Tests in [app.rs](../app.md) and [app/select.rs](../app/select.md) also call
    `draw` to run whole UI frames headlessly.
  - [ui/tracks.rs](tracks.md) — borrows `combo_text`, `grid_key_marks` and
    `layer_onion_tip` for the Timeline's layer tracks.
  - [ui/krita_import.rs](krita_import.md) — borrows `drag_by_title_only`,
    `floating_frame` and `title_shortfall` so its window behaves like the panels.
  - [ui/color_wheel.rs](color_wheel.md) — calls `clipboard_color` for its paste button.
- **Uses:**
  - [app.rs](../app.md) — `AppState` and nearly all of its methods (`dispatch`,
    `pointer_down`, `select_down`, `begin_camera_drag`, …), plus `PanelId`, `FillMode`,
    `NavKind`, `LassoHeld`.
  - UI helpers: [ui/theme.rs](theme.md) (colours, icon buttons, section headers),
    [ui/tracks.rs](tracks.md) (the Timeline's track area), [ui/color_wheel.rs](color_wheel.md),
    [ui/expr.rs](expr.md) (maths in number fields), [ui/krita_editor.rs](krita_editor.md),
    [ui/brush_picker.rs](brush_picker.md), [ui/widgets.rs](widgets.md),
    [ui/canvas_gpu.rs](canvas_gpu.md), [ui/krita_import.rs](krita_import.md).
  - Input: [input/shortcuts.rs](../input/shortcuts.md) (bound keys, held drags, the wheel
    map), [input/tablet.rs](../input/tablet.md) (pen packets and diagnostics),
    [input/screen_pixel.rs](../input/screen_pixel.md) (reading pixels off the screen),
    [input/button_drag.rs](../input/button_drag.md), [input/pen_record.rs](../input/pen_record.md),
    [input/pointer.rs](../input/pointer.md).
  - Document and tools: [doc/camera.rs](../doc/camera.md), [doc/canvas.rs](../doc/canvas.md),
    [doc/layer.rs](../doc/layer.md), [doc/transform.rs](../doc/transform.md),
    [doc/orient.rs](../doc/orient.md), [timeline/onion.rs](../timeline/onion.md),
    [tools/fill.rs](../tools/fill.md), [tools/select_mask.rs](../tools/select_mask.md),
    [tools/selection.rs](../tools/selection.md), [tools/perspective.rs](../tools/perspective.md),
    [tools/liquify.rs](../tools/liquify.md), [tools/wand.rs](../tools/wand.md),
    [tools/tap_lasso.rs](../tools/tap_lasso.md), [tools/noise.rs](../tools/noise.md),
    [tools/krita/mod.rs](../tools/krita/mod.md), [app/brush_sizes.rs](../app/brush_sizes.md).
  - Files: [io/composite.rs](../io/composite.md), [io/png_import.rs](../io/png_import.md),
    [io/png_save.rs](../io/png_save.md), [io/project_file.rs](../io/project_file.md),
    [io/frame_order.rs](../io/frame_order.md), [io/image_import.rs](../io/image_import.md),
    [io/sprite_sheet.rs](../io/sprite_sheet.md); plus [color.rs](../color.md),
    [krita_link.rs](../krita_link.md) and [krita_helper.rs](../krita_helper.md).

How one frame of UI is put together (all names are functions in this file unless
marked otherwise):

```text
eframe → AppState::update (src/app.rs) → shell::draw(state, ctx)
 ├─ if panels are shown:
 │   ├─ menu_window → title_menu          the floating top bar and its menus
 │   ├─ panel_window(Tools / Brush / Layers / Onion / Camera / X-sheet / Timeline)
 │   │    └─ panel_content → tools_content, brush_content, layers_content,
 │   │                       onion_content, camera_content, xsheet_content,
 │   │                       timeline_content (→ ui::tracks::show)
 │   └─ settings_window
 ├─ else, if wanted: mini_timeline_window   the slim bar shown while panels are hidden
 ├─ brush_settings_window, krita_import_window (ui/krita_import.rs), grid_settings_window
 ├─ new_project_dialog, export_dialog, import_range_dialog, busy_overlay
 ├─ save_error_dialog, save_toast, krita_toast
 ├─ wheel_map                               the mouse wheel over canvas / timeline
 ├─ CentralPanel (the canvas area):
 │   ├─ paint_canvas                        layers, onion ghosts, overlays
 │   ├─ press / drag / release → AppState    strokes, selections, view pan/zoom/rotate…
 │   └─ draw_tool_cursor, draw_readout, draw_swatch_pie
 └─ screen_pick_live                        the eyedropper loupe, drawn last
```

## Rust concepts in this file

**egui and immediate-mode UI.** egui has no retained widget tree. Every frame, your
code *calls* a function for each widget; the call draws it and returns a `Response`
saying whether it was clicked, dragged, hovered… You react right there, in an `if`. The
Tools panel's colour-picker button, from this file:
```rust
let sp = tip(state, Action::PickScreenColor, "Color picker");
if theme::icon_toggle(ui, ic::EYEDROPPER, &sp, state.screen_pick).clicked() {
    state.dispatch(Action::PickScreenColor);
}
```
The closest React equivalent re-renders from state and wires a callback:
```jsx
<IconToggle icon="eyedropper" title={tip} on={state.screenPick}
            onClick={() => dispatch("PickScreenColor")} />
```
The big differences: in egui there is no component state, no `useState`, no diffing —
the whole UI function runs again next frame, reading the current `AppState`. A slider
is given a `&mut` reference to the number it edits
(`egui::Slider::new(&mut state.brush.opacity, 0.0..=1.0)`) and simply overwrites it when
dragged. Things that must survive between frames (a window's position, a text field's
cursor) are kept by egui in its own memory, keyed by an `egui::Id` — which is why
[`panel_key`](#panel_key) and [`panel_area`](#panel_area) exist.
More: [egui and immediate-mode UI](../../rust-primer.md#egui-and-immediate-mode-ui)

**Closures as layout blocks.** Most egui containers take a closure — an inline function,
written `|args| body` — and run it to fill themselves:
```rust
ui.horizontal(|ui| {
    pen_row(state, ui);
    tool_toggle(ui, state, ActiveTool::Eraser, ic::ERASER, &e);
});
let mods = ctx.input(|i| i.modifiers); // read input state through a closure
```
```js
ui.horizontal(ui => { penRow(state, ui); toolToggle(ui, state, "Eraser"); });
```
The inner `ui` is a new layout cursor that places widgets left to right.
More: [Closures](../../rust-primer.md#closures)

**`&AppState` vs `&mut AppState`.** Functions that only *read* the state to draw or
build text take `state: &AppState` (a shared, read-only borrow), e.g. `tip`,
`tablet_status`. Functions whose widgets change it take `state: &mut AppState` (an
exclusive, writable borrow), e.g. `tools_content`. Rust checks at compile time that
nothing else touches the state while a `&mut` borrow is alive — in JS any function can
mutate any object you pass it, here the signature tells you.
More: [Ownership and borrowing](../../rust-primer.md#ownership-and-borrowing)

**`match` on enums.** Rust enums are like TypeScript tagged unions, and `match` must
handle every case — the compiler refuses to build if one is missing:
```rust
match id {
    PanelId::Tools => (ic::TOOLBOX, "Tools"),
    PanelId::Brush => (ic::PAINT_BRUSH, "Brush"),
    // … one arm per panel
}
```
```js
switch (id) { case "Tools": return [icons.toolbox, "Tools"]; /* … */ }
```
More: [Enums and match](../../rust-primer.md#enums-and-match)

**`Option` and `let … else`.** A value that may be missing is an `Option<T>`
(`Some(x)` or `None`), never `null`. `let Some(x) = … else { return; };` unpacks it or
leaves the function early:
```rust
let Some(pointer) = ctx.input(|i| i.pointer.hover_pos()) else {
    return;
};
```
```js
const pointer = ctx.hoverPos(); if (pointer == null) return;
```
More: [Option and Result](../../rust-primer.md#option-and-result)

**`format!` and `&'static str`.** `format!("{base}  ({})", c.display())` builds a new
`String` like a JS template literal. Functions returning fixed text return
`&'static str` — a reference to text baked into the program, which lives forever and
costs nothing to hand out.
More: [Macros](../../rust-primer.md#macros), [Strings and slices](../../rust-primer.md#strings-and-slices)

### Rust concepts in this stretch
**Closures handed to egui for nested layout.** Almost every container in egui takes a
closure that receives a child `ui` to draw into.
```rust
ui.horizontal(|ui| {
    ui.label("X");
    ui.add(egui::DragValue::new(&mut state.project.camera.tx).speed(1.0));
});
```
```js
// Roughly: render a row and pass a callback that fills it.
horizontal(ui => { ui.label("X"); ui.dragValue(camera, "tx"); });
```
The inner `ui` *shadows* the outer one (same name, new value). More:
[Closures](../../rust-primer.md#closures).

**"Remember what was clicked, act after the loop."** While the layer list is drawn, the
code holds a `&mut` borrow of the layer `Vec`. Rust will not let it also call
`state.delete_…()` (which wants `&mut state`) at the same time, so clicks are stored in
`Option` variables and handled once the list is finished.
```rust
let mut select: Option<usize> = None;
// … inside the row loop …
if resp.clicked() { select = Some(i); }
// … after the loop, the borrow has ended:
if let Some(i) = select { state.project.current_layer = i; }
```
```js
let select = null;            // set inside the render callback
if (select !== null) app.currentLayer = select;
```
More: [Ownership and borrowing](../../rust-primer.md#ownership-and-borrowing).

**Split borrows.** Two different fields of the same struct may be borrowed mutably at
once, because the compiler can see they do not overlap:
```rust
let (layers, rename) = (&mut state.project.layers, &mut state.layer_rename);
```
**A struct that holds a reference needs a lifetime.** `struct RowMenu<'a> { grp_key: &'a str, … }`
says "a `RowMenu` borrows a string and must not outlive it". JS has no equivalent because
its garbage collector keeps the string alive for you. More:
[Lifetimes](../../rust-primer.md#lifetimes).

**`let … else`.** `let Some(mut v) = state.project.layers.get(cur).map(|l| l.opacity) else { return };`
unwraps an `Option` or leaves the function early — like
`const v = layers[cur]?.opacity; if (v === undefined) return;`. More:
[Enums and match](../../rust-primer.md#enums-and-match).

**egui's per-widget scratch memory.** `ui.data(|d| d.get_temp::<f32>(id))` and
`ui.data_mut(|d| d.insert_temp(id, value))` store a small value between frames under an
`egui::Id`, like a `useRef` keyed by a string. The Layers panel uses it to remember how
tall its lower sections were last frame.

## Types

### `const WHEEL_ZOOM_STEP`
[src/ui/shell.rs:890](../../../src/ui/shell.rs#L890)

```rust
const WHEEL_ZOOM_STEP: f32 = 1.15;
```

`f32 = 1.15` — how much one mouse-wheel notch zooms the view (15% per notch).

### `const POINTS_PER_SCRUB_NOTCH`
[src/ui/shell.rs:902](../../../src/ui/shell.rs#L902)

```rust
const POINTS_PER_SCRUB_NOTCH: f32 = 24.0;
```

`f32 = 24.0` — how many points of trackpad scrolling add up to one wheel "notch" (a
trackpad sends many tiny scroll events instead of whole notches).

### `const LAYERS_DEFAULT_H`
[src/ui/shell.rs:935](../../../src/ui/shell.rs#L935)

```rust
const LAYERS_DEFAULT_H: f32 = 496.0;
```

`f32 = 496.0` — the Layers panel's starting height: its two toolbars, opacity slider,
three rows of layer list and the folded sections under it.

### `const XSHEET_DEFAULT_H`
[src/ui/shell.rs:939](../../../src/ui/shell.rs#L939)

```rust
const XSHEET_DEFAULT_H: f32 = 330.0;
```

`f32 = 330.0` — the X-sheet panel's starting height: its toolbar and about ten frames of
sheet. (An *x-sheet* or exposure sheet is the frame-by-frame table of which drawing shows
on which frame — see the [glossary](../../glossary.md).)

### `const ONION_DEFAULT_H`
[src/ui/shell.rs:943](../../../src/ui/shell.rs#L943)

```rust
const ONION_DEFAULT_H: f32 = 210.0;
```

`f32 = 210.0` — the Onion panel's starting height: the strip with its bars at their
shortest, the settings and the pin controls. (*Onion skin* = faint ghosts of nearby
frames drawn under the current one.)

### `const TOOLS_START_H`
[src/ui/shell.rs:947](../../../src/ui/shell.rs#L947)

```rust
const TOOLS_START_H: f32 = 248.0;
```

`f32 = 274.0` — the Tools panel's height when it first opens (on the pencil), with its two rows
of tool buttons. Used to
guess where the Brush panel below it starts before Tools has been measured.

### `const TOOLS_MAX_W`
[src/ui/shell.rs:950](../../../src/ui/shell.rs#L950)

```rust
const TOOLS_MAX_W: f32 = 560.0;
```

`f32 = 560.0` — the widest the Tools panel may grow. Its sliders fill its width, so
without a cap a width it once grew to would stick forever.

### `const TIMELINE_START_W`
[src/ui/shell.rs:953](../../../src/ui/shell.rs#L953)

```rust
const TIMELINE_START_W: f32 = 718.0;
```

`f32 = 718.0` — the Timeline's width when it first opens (its toolbar's width). Used to
centre it between the two panel columns.

### `const PANEL_SNAP_PX`
[src/ui/shell.rs:957](../../../src/ui/shell.rs#L957)

```rust
const PANEL_SNAP_PX: f32 = 20.0;
```

`f32 = 20.0` — how close (in points) a released panel must be to an edge or another
panel to snap to the padding distance from it.

### `const PADDING_STEP`
[src/ui/shell.rs:962](../../../src/ui/shell.rs#L962)

```rust
const PADDING_STEP: u8 = 4;
```

`u8 = 4` — when the user changes the panel padding in Settings, the padding the panels
are laid out with moves towards the new value by at most this many points per frame. It
is kept small so a panel that sat one padding away from another stays within snapping
reach of it on the next frame.

### `struct PanelEdges`
[src/ui/shell.rs:967](../../../src/ui/shell.rs#L967)

What the floating panels keep their distance from: `pad` points in from the window's
edges, and `pad` below the top bar. Built once per frame in [`draw`](#draw) and handed to
every [`panel_window`](#panel_window). It derives `Clone, Copy, Debug`, so it is copied
around freely like a number.

| Field | Type | Meaning |
|---|---|---|
| `screen` | `Rect` | The whole window's rectangle. |
| `pad` | `f32` | The padding, in points, panels keep from edges and from each other. |
| `was` | `Option<f32>` | The padding before it changed this frame (`None` when it didn't). Panels it held against an edge move to the new padding. |
| `bar` | `Option<Rect>` | The top menu bar's rectangle, if it has been laid out. A panel under it keeps `pad` below it. |

### `const BRUSH_SIZES`
[src/ui/shell.rs:1439](../../../src/ui/shell.rs#L1439)

```rust
const BRUSH_SIZES: std::ops::RangeInclusive<f32> = 0.5..=128.0;
```

`RangeInclusive<f32> = 0.5..=128.0` — the size range of the pens' Size slider and the
Shape tool's Thickness slider. (`a..=b` is a Rust range that includes both ends.)

### `const LOCK_W`
[src/ui/shell.rs:1648](../../../src/ui/shell.rs#L1648)

```rust
const LOCK_W: f32 = 22.0;
```

How wide the lock toggle at the end of a size row is.

### `const TABLET_ICON_W`
[src/ui/shell.rs:1775](../../../src/ui/shell.rs#L1775)

```rust
const TABLET_ICON_W: f32 = 18.0;
```

`f32 = 18.0` — room left for the tablet status icon beside the Brush settings button.

### `const MINI_BAR_NAME_W`
[src/ui/shell.rs:2673](../../../src/ui/shell.rs#L2673)

```rust
const MINI_BAR_NAME_W: f32 = 140.0;
```

`f32 = 140.0` — width of the mini timeline's slot for the active layer's name: room for
the 18 characters a name is cut to.

### `const STRIP_NUM_H`
[src/ui/shell.rs:3050](../../../src/ui/shell.rs#L3050)

```rust
const STRIP_NUM_H: f32 = 16.0;
```

`f32` values `16.0`, `3.0`, `46.0` — heights in the onion strip: the row of offset
numbers, the gap under it, and the bars at their shortest. A taller Onion panel makes
the bars taller than `STRIP_BAR_H`.

### `const STRIP_GAP`
[src/ui/shell.rs:3051](../../../src/ui/shell.rs#L3051)

```rust
const STRIP_GAP: f32 = 3.0;
```

`f32` values `16.0`, `3.0`, `46.0` — heights in the onion strip: the row of offset
numbers, the gap under it, and the bars at their shortest. A taller Onion panel makes
the bars taller than `STRIP_BAR_H`.

### `const STRIP_BAR_H`
[src/ui/shell.rs:3052](../../../src/ui/shell.rs#L3052)

```rust
const STRIP_BAR_H: f32 = 46.0;
```

`f32` values `16.0`, `3.0`, `46.0` — heights in the onion strip: the row of offset
numbers, the gap under it, and the bars at their shortest. A taller Onion panel makes
the bars taller than `STRIP_BAR_H`.

### `const LAYER_LIST_MIN_H`
[src/ui/shell.rs:3697](../../../src/ui/shell.rs#L3697)

```rust
const LAYER_LIST_MIN_H: f32 = 96.0;
```

`f32 = 96.0`. The shortest (in egui points) the scrolling layer list may get when the
sections under it (Transform, Layer canvas) want room — about one layer row. Below this
the panel grows instead. Used by [`layers_content`](#layers_content).

### `struct LayerDrag`
[src/ui/shell.rs:3700](../../../src/ui/shell.rs#L3700)

A *tuple struct* (a struct whose single field has no name, read as `.0`) used as the
drag-and-drop **payload** when a layer row is dragged. The `usize` is the index of the
dragged layer. egui finds the payload by its type, so wrapping the number in its own type
keeps it from being confused with other drags. More: [Structs](../../rust-primer.md#structs).

| Field | Type | Meaning |
|---|---|---|
| `0` | `usize` |  |

### `const ROW_DOTS_W`
[src/ui/shell.rs:3703](../../../src/ui/shell.rs#L3703)

```rust
const ROW_DOTS_W: f32 = 22.0;
```

`f32 = 22.0`. Width of the ⋮ ("more options") button at the right of each layer row. The
name field is shrunk by this much so the button always fits.

### `enum RowCmd`
[src/ui/shell.rs:3707](../../../src/ui/shell.rs#L3707)

What was picked in a layer row's ⋮ menu. It derives `Clone, Copy, Debug, PartialEq, Eq`
so it can be copied and compared freely.

| Variant | Meaning |
|---|---|
| `Rename` | Start renaming the row's layer inline |
| `GroupSelected` | Put the selected layers into a new group |
| `Krita` | Open just this layer in Krita |
| `Ungroup` | (group rows) dissolve the group, keeping its layers |
| `MergeGroup` | (group rows) flatten the group into one layer |
| `DeleteGroup` | (group rows) delete the group and its layers |

### `struct RowMenu`
[src/ui/shell.rs:3717](../../../src/ui/shell.rs#L3717)

What the row menu needs to know about the row it belongs to. The `'a` lifetime is there
because `grp_key` borrows a string owned by the caller.

| Field | Type | Meaning |
|---|---|---|
| `header` | `bool` | The row is a group header, not a layer |
| `grp_key` | `&'a str` | The shortcut text for "Group selected layers", shown in the label |
| `krita_label` | `&'static str` | Label for the Krita item (from [`krita_menu_label`](../../../src/ui/shell.rs#L9563)) |
| `krita_ok` | `bool` | False for reference layers and layers whose name ends in `-x`, which never go to Krita |

### `const OPACITY_BADGE_PAD`
[src/ui/shell.rs:3892](../../../src/ui/shell.rs#L3892)

```rust
const OPACITY_BADGE_PAD: f32 = 2.0;
```

`f32 = 2.0`. Left/right padding of the small "75%" field on a layer row — just enough to
still look like a field.

### `const ORIENT_ART`
[src/ui/shell.rs:6272](../../../src/ui/shell.rs#L6272)

```rust
const ORIENT_ART: [(crate::doc::orient::Orient, Action, &str, &str); 5] = {
    use crate::doc::orient::Orient;
    [
        (Orient::FLIP_H, Action::ArtFlipH, ic::FLIP_HORIZONTAL, "Flip horizontally"),
        (Orient::FLIP_V, Action::ArtFlipV, ic::FLIP_VERTICAL, "Flip vertically"),
        (Orient::CW, Action::ArtRotateCw, ic::ARROW_CLOCKWISE, "Rotate 90° clockwise"),
// …
```

The five "flip and turn the art" commands as the Lasso options, the Edit menu and the
Layers toolbar list them. Each entry is a **tuple** `(what it does, shortcut action, icon,
name)`:

The value is written as a block `{ use …; [ … ] }`: a block is an expression whose last
line is its value, so the `use` stays local to this constant. `Orient` comes from
[doc/orient.rs](../doc/orient.md). Code loops over it with
`for (o, action, icon, name) in ORIENT_ART { … }` — destructuring each tuple, like
`for (const [o, action, icon, name] of ORIENT_ART)` in JS. More:
[Constants and statics](../../rust-primer.md#constants-and-statics).

**Used by:** [`layer_selection_actions`](#layer_selection_actions),
[`selection_options`](#selection_options) and
[`title_menu`](../../../src/ui/shell.rs#L8479).

### `enum PenOp`
[src/ui/shell.rs:6325](../../../src/ui/shell.rs#L6325)

What a pen slot's right-click menu asks of the row, done once the row is drawn.

| Variant | Meaning |
|---|---|
| `Duplicate(usize)` | Copy this slot into a new one right after it (`AppState::duplicate_pen`). |
| `Remove(usize)` | Remove this slot (`AppState::remove_pen`). |

### `const PEN_MENU_W`
[src/ui/shell.rs:6383](../../../src/ui/shell.rs#L6383)

```rust
const PEN_MENU_W: f32 = 290.0;
```

Wide enough for the icons two rows of eight, and four brushes a row.

### `const STABILIZED_DOT_R`
[src/ui/shell.rs:6446](../../../src/ui/shell.rs#L6446)

```rust
const STABILIZED_DOT_R: f32 = 2.5;
```

`f32 = 2.5`. Radius in points of the dot on a stabilized pen's tool button. Used by
[`tool_toggle`](#tool_toggle) and a test.

### `struct Xform`
[src/ui/shell.rs:7635](../../../src/ui/shell.rs#L7635)

The canvas **view transform**: how the document (the drawing, measured in its own pixels) is
placed on screen. It combines the "fit to window" base scale with the user's zoom, pan, rotation
and mirror flips. The same `Xform` is used both to *draw* the canvas and to turn mouse/pen
positions back into document positions, so drawing and input always agree. It derives `Clone`
and `Copy`, so it is passed around by value like a number
([Traits and derive](../../rust-primer.md#traits-and-derive)).

| Field | Type | Meaning |
|---|---|---|
| `center` | `egui::Pos2` | Centre of the canvas area on screen. |
| `pan` | `Vec2` | Extra screen offset from panning. |
| `scale` | `f32` | Screen points per document pixel (always positive). Also published as `state.view_scale`, which sizes the brush. |
| `fx` | `f32` | Mirror signs (±1), applied *before* the rotation. Kept out of `scale` deliberately: `scale` is clamped positive and is republished as `view_scale`, which sizes the brush — a negative there would be clamped away in one place and taken literally in another. |
| `fy` | `f32` | Vertical mirror sign (±1), like `fx`. |
| `rot_sin` | `f32` | Sine of the view rotation, precomputed once per frame so every point conversion is just multiplies and adds. |
| `rot_cos` | `f32` | Cosine of the view rotation. |
| `dcx` | `f32` | Half the document width — the document's centre, which the view rotates and scales about. |
| `dcy` | `f32` | Half the document height. |
| `cw` | `f32` | Document (canvas) width in pixels. |
| `ch` | `f32` | Document (canvas) height in pixels. |

### `const PEN_MOUSE_AGREEMENT`
[src/ui/shell.rs:8400](../../../src/ui/shell.rs#L8400)

```rust
const PEN_MOUSE_AGREEMENT: f32 = 20.0;
```

`f32 = 20.0`. How far (in egui points) the OS cursor may be from the pen's *recent path* before
the tablet's batch of packets is disbelieved. The doc comment explains why it is measured against
the path and not just the newest packet: the driver moves the cursor from the same packets but a
few milliseconds later, so during a fast flick the cursor trails the pen by much more than 20
points. Checking against the newest packet made every fast stroke fail and fall back to the cursor
(producing long straight chords). A truly wrong mapping puts the whole path elsewhere, so the path
check still catches it.

### `const MAX_PACKET_JUMP`
[src/ui/shell.rs:8407](../../../src/ui/shell.rs#L8407)

```rust
const MAX_PACKET_JUMP: f32 = 400.0;
```

`f32 = 400.0`. How far (in points) a packet may be from the newest packet in the same frame's
batch. One frame at 60 Hz with 400 points of travel would be 24,000 points per second, which no
hand can do — so a packet that far away is not a real position and is dropped.

### `enum PenFrame`
[src/ui/shell.rs:8411](../../../src/ui/shell.rs#L8411)

This frame's tablet input, as far as a stroke is concerned. `#[derive(Debug)]` lets it be printed
for debugging.

| Variant | Meaning |
|---|---|
| `Points(Vec<(egui::Pos2, PenPacket)>)` | The packets as screen positions (oldest first), each paired with the packet it came from. Sub-pixel and usually several per frame — the whole reason this path exists. |
| `Empty` | The pen is live and trusted but sent nothing new this frame. Common, since a live stroke repaints much faster than a pen reports. |
| `Untrusted` | No pen to draw from: none at all, or its mapping disagrees with the OS cursor. |

### `const PRESET_LIST_MAX_H`
[src/ui/shell.rs:8953](../../../src/ui/shell.rs#L8953)

```rust
const PRESET_LIST_MAX_H: f32 = 170.0;
```

`f32 = 170.0`. How tall (in points) the "My presets" list in New project may grow before it
scrolls — about six rows, so a long list never pushes the Create button off the screen.

### `const SIZE_PRESETS`
[src/ui/shell.rs:8957](../../../src/ui/shell.rs#L8957)

```rust
const SIZE_PRESETS: [(&str, u32, u32); 6] = [
    ("HD 720p", 1280, 720),
    ("Full HD", 1920, 1080),
    ("2K", 2048, 1080),
    ("4K UHD", 3840, 2160),
    ("6K", 6144, 3456),
// …
```

One-click sizes as `(label, width, height)` tuples, written landscape. Used for a new canvas in
New project and for a camera frame in the Camera panel.

Used by [`camera_size_ui`](../../../src/ui/shell.rs#L3291) and
[`new_project_dialog`](#new_project_dialog).

### `const MAX_SIDE`
[src/ui/shell.rs:8968](../../../src/ui/shell.rs#L8968)

```rust
const MAX_SIDE: u32 = 16384;
```

`u32 = 16384`. The longest side a camera frame (or a clipboard-sized canvas) may have: the canvas
limit, which is the largest texture a GPU accepts. Used by
[`camera_size_ui`](../../../src/ui/shell.rs#L3291),
[`new_project_dialog`](#new_project_dialog) and tests.

## Free functions

### `tip`
[src/ui/shell.rs:30](../../../src/ui/shell.rs#L30)

```rust
fn tip(state: &AppState, action: Action, base: &str) -> String
```

**What it does.** Builds tooltip text that ends with the keyboard shortcut currently
bound to `action`, e.g. `"Pencil  (Q)"`. If nothing is bound it returns `base` unchanged.
Because shortcuts are user-rebindable, tooltips are rebuilt from `state.shortcuts` every
frame rather than hard-coded.

**Used by:** nearly every button in this file (tool buttons, the Timeline toolbar,
`play_button`, …) and [`key_jump_tip`](#key_jump_tip).

### `key_jump_tip`
[src/ui/shell.rs:39](../../../src/ui/shell.rs#L39)

```rust
fn key_jump_tip(state: &AppState, action: Action, base: &str) -> String
```

**What it does.** Like [`tip`](#tip), for the "previous / next drawing key" buttons. When
the active layer is *key-linked* to another layer (its key jumps also stop at that
layer's drawings — see `Project::key_link`), the tooltip names the other layer:
`"Next drawing key — this layer + Rough"`.

**Used by:** `timeline_content` and `mini_timeline_window` (the key-jump buttons).

### `combo_text`
[src/ui/shell.rs:48](../../../src/ui/shell.rs#L48)

```rust
pub(crate) fn combo_text(state: &AppState, action: Action) -> String
```

**What it does.** Returns just the shortcut text bound to `action` (e.g. `"Ctrl+S"`), or
an empty string if it is unbound. `pub(crate)` means other files in this crate may call
it, but nothing outside it. `.unwrap_or_default()` turns a missing value (`None`) into
the type's default, here `""`.

**Used by:** many tooltips and menu labels in this file (menus, Camera panel, Layers
panel, selection options), and [ui/tracks.rs](tracks.md) for its right-click menus.

### `mid_drag_hint`
[src/ui/shell.rs:58](../../../src/ui/shell.rs#L58)

```rust
fn mid_drag_hint(state: &AppState) -> String
```

**What it does.** Text describing the keys that work *during* a shape or box drag —
"Space moves it, Alt from the middle" — naming whichever of `Action::ShapeMove` and
`Action::ShapeCentre` are bound. Returns an empty string if neither is bound.
It matches on a *tuple* of two `Option`s, which is a neat way to handle all four
combinations at once:
```rust
match (key(Action::ShapeMove), key(Action::ShapeCentre)) {
    (Some(m), Some(c)) => format!("{m} moves it, {c} from the middle"),
    (Some(m), None)    => format!("{m} moves it"),
    (None, Some(c))    => format!("{c} from the middle"),
    (None, None)       => String::new(),
}
```

**Used by:** [`with_mid_drag_hint`](#with_mid_drag_hint) and the Camera panel.

### `with_mid_drag_hint`
[src/ui/shell.rs:69](../../../src/ui/shell.rs#L69)

```rust
fn with_mid_drag_hint(state: &AppState, base: &str) -> String
```

**What it does.** Returns `base` followed by `", "` and the [`mid_drag_hint`](#mid_drag_hint),
or just `base` if no mid-drag keys are bound.

**Used by:** the Shape tool's hint in [`tools_content`](#tools_content) and the
rectangle/ellipse hint in `selection_options`.

### `draw`
[src/ui/shell.rs:76](../../../src/ui/shell.rs#L76)

```rust
pub fn draw(state: &mut AppState, ctx: &egui::Context)
```

**What it does.** The entry point of the whole UI, called once per frame. It draws the
top bar, every panel, window, dialog and toast, then the canvas — and handles all mouse
and pen input on the canvas: deciding what a press means and passing each frame's pointer
positions to the right `AppState` method.

**Parameters.**
- `state` — the whole application state, borrowed writable for the frame.
- `ctx` — the egui context for the window.

**How it works.**

1. **Panels (when `state.show_panels` is on).**
   1. Detects a window resize by swapping `ctx.screen_rect()` into `state.viewport_rect`
      and comparing; the previous rect, if different, is passed to every panel as
      `resized` so it can keep its distance from the nearest edge
      ([`resticked_pos`](#resticked_pos)).
   2. Animates the panel padding: `state.panel_padding_laid` moves towards the
      user's `state.panel_padding` by at most [`PADDING_STEP`](#const-padding_step)
      points a frame, asking for another repaint until it arrives.
   3. Draws the top bar (`menu_window`) and builds the frame's [`PanelEdges`](#struct-paneledges).
   4. Places the panels. Default spots: Tools at the top-left under the bar, Brush
      under Tools, Layers at the top-right, then Onion, Camera and X-sheet stacked under
      Layers, each one padding below the last. "Below the last" is read from egui's
      memory of where that panel was laid out (`egui::AreaState::load`). The Timeline is
      centred between the two columns if [`TIMELINE_START_W`](#const-timeline_start_w)
      fits there, otherwise centred on the window. These are only *default* positions —
      once the user moves a panel, egui remembers it.
   5. Draws the Settings window (`settings_window`).

   Otherwise, if `state.show_mini_timeline` is on, it draws only the mini timeline bar
   (`mini_timeline_window`).
2. **Always-available windows**, drawn whether panels are shown or not (hiding panels is
   what you do to draw, and brush feel and grid snapping get adjusted while drawing):
   Brush settings, the Krita import window, grid settings, the New project / Export /
   Import range dialogs, a busy overlay while a background job runs (`state.bg_label`),
   the save-error dialog, the save and Krita toasts, and [`wheel_map`](#wheel_map).
3. **The canvas.** An `egui::CentralPanel` with a transparent frame fills the space left
   over. It claims its whole rect with `Sense::drag()` and gets a `Response` (`resp`)
   telling it about presses and drags. It stores `state.canvas_rect`, publishes the
   doc→screen scale (`state.view_scale`, from `Xform::new`), and paints the canvas with
   `paint_canvas`. While a stroke is live it requests a repaint every frame, because
   tablet packets are only drained when a frame runs.
4. **Canvas input** (skipped entirely while screen colour-pick mode is on):
   1. Reads modifier "holds" for this frame: a held selection mode (Lasso tool only,
      `held_sel_op`), held lasso-fill modes (Fill tool in lasso mode, `held_lasso`), a
      held drag combo from the shortcut map (`shortcuts.held_drag`, e.g. Space = pan) and
      whether the middle or right mouse button is down (each bound to a `ButtonDrag`
      action in Settings).
   2. **On press** (`resp.drag_started()`): clears the Timeline's track selection, calls
      `state.drag_began()`, then decides **once** what this drag is:
      - a press on the selection's transform box takes it, ignoring modifier holds;
      - a middle/right button bound to Pan/Zoom/Rotate becomes a navigation drag
        (`state.nav_drag`); other button bindings start a *button drag*
        (`begin_button_drag`, e.g. scrub frames, swatch pie, tool adjust);
      - a held combo on the left button starts a navigation or a held drag
        (`begin_held_drag`);
      - with layer-transform mode on, a navigation gesture moves the active layer
        instead of the view (`begin_layer_xform`); with camera edit mode on, it moves the
        camera (`begin_camera_drag`);
      - otherwise the tool gets the press: Draw-camera mode (`camera_box_down`), the
        camera frame (`camera_frame_down`), Tracker (`tracker_click`), Perspective
        (`perspective_down`), Lasso (`select_down` + `wand_anchor`), lasso Fill
        (`fill_lasso_down`), or a stroke. For a stroke, `pen_stroke_start` decides whether
        the pen's own packets or the mouse cursor will drive it, the point is snapped
        (`snap_begin`), mapped into the active cell (`doc_to_active_cell`), turned into a
        sample (`stroke_sample`) and passed to `state.pointer_down`.
   3. **While dragging**: if the held combo was released mid-drag the gesture ends
      there (`finish_canvas_drag`) and the rest of the drag is ignored (`drag_inert`).
      Otherwise the movement goes to whatever the press chose: layer pan/rotate/scale,
      camera pan/rotate/zoom, view pan, view rotate (about the window centre, so the
      middle of the canvas holds still), view zoom (about the pointer, via
      [`zoom_view_at`](#zoom_view_at)), a button drag, the camera frame or box, the
      perspective grid, the selection, the lasso fill, a fill retune drag, or — for a
      stroke — every pen packet or cursor position of this frame, each turned into a
      sample and passed to `state.pointer_move`. While the view is locked to the camera
      (`camera_look_through`) view gestures are swallowed so they don't pile up unseen.
   4. **On release** (`resp.drag_stopped()`): `finish_canvas_drag` (unless the drag was
      made inert), and the per-drag flags are reset.
   5. Every frame it flushes Liquify's queued dabs (`state.flush_liquify()`), and a
      double- or triple-click on the canvas closes an open polygon selection or polygon
      fill.
   6. **Cursor**: in Draw-camera mode a crosshair; otherwise, when hovering the canvas
      and not navigating, it hides the OS cursor and draws the tool's own cursor
      (`draw_tool_cursor`) — held at the press point during a tool-adjust drag so you can
      judge the new size against the drawing — plus any readout text. During a swatch-pie
      drag it draws the pie (`draw_swatch_pie`).
5. Finally [`screen_pick_live`](#screen_pick_live) runs last, so its loupe sits on top.

**Example.** egui has no `pointerdown` / `pointermove` / `pointerup` events. Instead,
each frame, `draw` asks the canvas's `Response` what happened since last frame. The
same flow written as JS event handlers would look like:
```js
canvas.onpointerdown = e => { mode = decideOnce(e); begin(mode, e); };    // resp.drag_started()
canvas.onpointermove = e => { if (dragging && !inert) move(mode, e); };   // resp.dragged()
canvas.onpointerup   = e => { if (!inert) finish(mode); inert = false; }; // resp.drag_stopped()
```
In Rust it is three `if`s in a row in the same function, run every frame:
```rust
if resp.drag_started() { /* decide once, begin */ }
if resp.dragged() && !state.drag_inert { /* route this frame's movement */ }
if resp.drag_stopped() { /* finish */ }
```

**Used by:** `AppState::update` in [app.rs](../app.md) (once per frame), and many UI
tests in [app.rs](../app.md), [app/select.rs](../app/select.md) and this file.

### `screen_pick_live`
[src/ui/shell.rs:597](../../../src/ui/shell.rs#L597)

```rust
fn screen_pick_live(state: &mut AppState, ctx: &egui::Context)
```

**What it does.** Runs the eyedropper ("screen pick") mode while `state.screen_pick` is
on: each frame it reads the real pixels under the operating-system cursor straight from
the screen, shows a magnifying loupe with the colour next to the cursor, and on the next
click commits that colour. Escape cancels. Because it reads the screen itself, it picks
whatever is literally visible — the canvas, or anything behind the window where the
canvas backdrop is transparent.

**How it works.**
1. Returns at once if pick mode is off. Otherwise sets a crosshair cursor and requests a
   repaint so sampling stays live.
2. Escape calls `state.end_screen_pick()` and stops.
3. `state.screen_pick_arm` swallows the click that *opened* the mode (e.g. on the toolbar
   button): a click only counts once the button has been released at least once.
4. Works out if the pointer is over a floating panel by checking which egui *layer* is
   under it: the canvas lives on `Order::Background`, panels on `Order::Middle`. (It
   cannot use `is_pointer_over_area()` because the canvas itself counts as an area.)
5. Captures a 25×25 pixel region around the cursor (local const `HALF = 12`, in screen
   pixels) with [input/screen_pixel.rs](../input/screen_pixel.md); the centre pixel is the
   colour. The region is uploaded into a texture kept on `state.screen_pick_tex`
   (created once, then updated with `set`), using nearest-neighbour filtering so the
   pixels stay crisp when magnified.
6. Off the panels, it draws the loupe ([`draw_screen_pick_loupe`](#draw_screen_pick_loupe))
   and always the hint banner ([`screen_pick_banner`](#screen_pick_banner)).
7. A fresh press off the panels calls `state.commit_screen_pick(colour)`.

**Used by:** [`draw`](#draw), as its last step.

### `screen_pick_banner`
[src/ui/shell.rs:665](../../../src/ui/shell.rs#L665)

```rust
fn screen_pick_banner(ctx: &egui::Context)
```

**What it does.** Paints a small dark rounded label at the top centre of the window,
"Pick colour — click to sample · Esc to cancel", on egui's `Foreground` layer so it sits
above everything. It uses a `Painter` (egui's low-level drawing API: rectangles, text,
lines) rather than widgets, so the banner takes no input.

**Used by:** [`screen_pick_live`](#screen_pick_live).

### `draw_screen_pick_loupe`
[src/ui/shell.rs:686](../../../src/ui/shell.rs#L686)

```rust
fn draw_screen_pick_loupe(
    ctx: &egui::Context,
    cursor: egui::Pos2,
    tex: Option<&egui::TextureHandle>,
    color: Option<[u8; 4]>,
    half: i32,
)
```

**What it does.** Draws a PowerToys-style zoom loupe: the captured screen region
magnified into a 144-point square, the centre pixel outlined in black and white, and
under it a bar with a colour swatch and the colour as text (formatted by
`color::fmt_rgb`; "—" if no colour was read).

**Parameters.**
- `cursor` — where the pointer is; the loupe is offset 34 points from it, flipping to
  the left or above when it would run off the window, so it never covers (or gets
  captured into) the pixel being sampled.
- `tex` — the texture holding the captured region, if any.
- `color` — the sampled RGBA colour, if any.
- `half` — the capture radius; the region is `2*half + 1` pixels wide, which gives the
  size of one magnified pixel.

The text bar grows wider than the loupe when the text needs it, and grows *away* from
the cursor.

**Used by:** [`screen_pick_live`](#screen_pick_live).

### `panel_meta`
[src/ui/shell.rs:745](../../../src/ui/shell.rs#L745)

```rust
fn panel_meta(id: PanelId) -> (&'static str, &'static str)
```

**What it does.** Returns a panel's icon glyph (from the Phosphor icon font, imported as
`ic`) and its title, e.g. `(ic::STACK, "Layers")`. Returns a *tuple* — two values in one,
like returning `[icon, title]` in JS.

**Used by:** [`panel_window`](#panel_window), [`title_bar_shortfall`](#title_bar_shortfall)
and the Panels menu in `title_menu` (one checkbox per panel).

### `wheel_map`
[src/ui/shell.rs:768](../../../src/ui/shell.rs#L768)

```rust
fn wheel_map(state: &mut AppState, ctx: &egui::Context)
```

**What it does.** Handles the mouse wheel over the canvas, the Timeline panel and the
mini timeline, doing whatever the *wheel map* in Settings assigns to the modifiers held
(see `ShortcutMap::wheel_action` in [input/shortcuts.rs](../input/shortcuts.md)): step
frames, jump between drawings, zoom the view, change the brush size, or change layer.

It reads egui's raw `MouseWheel` events rather than egui's smoothed scroll value: a mouse
reports whole "lines" (one per notch), which maps cleanly to one step, and the raw events
are still there even after a scroll area under the pointer has used the smoothed value.

**How it works.**
1. Does nothing mid-stroke or mid-navigation (the stroke is writing into the drawing of
   the frame it started on, so the frame must not change).
2. Finds which surface is under the pointer by its egui layer: the canvas
   (`Order::Background`), the Timeline panel, or the `"mini_timeline"` window. Anything
   else keeps normal scrolling.
3. Looks up the action for the held modifiers. Over the Timeline, the layer-name column
   and the Zoom / PanTimeline actions are left to [ui/tracks.rs](tracks.md).
4. Adds up this frame's wheel events: whole lines (and pages) count as notches; points
   (trackpads) are banked in `state.wheel_scrub_accum` until they reach
   [`POINTS_PER_SCRUB_NOTCH`](#const-points_per_scrub_notch). A change of direction empties
   the bank so reversing is felt at once. With Shift held, sideways scroll counts too
   (a Mac reports Shift+wheel as sideways).
5. Applies the action `n` times (`n` = whole notches, positive = wheel up):
   - **Frames** — stops playback and steps `n × frame_step_delta()` frames (direction
     flipped by the "invert timeline scroll" setting; down advances by default);
   - **Drawings** — jumps key to key (`next_key_frame` / `prev_key_frame`);
   - **Zoom** — over the canvas, zooms by [`WHEEL_ZOOM_STEP`](#const-wheel_zoom_step) to the
     power of the notches, about the pointer;
   - **BrushSize** / **Layers** — dispatches `SizeUp`/`SizeDown` or
     `LayerSelectAbove`/`LayerSelectBelow` once per notch.

**Used by:** [`draw`](#draw).

### `zoom_view_at`
[src/ui/shell.rs:894](../../../src/ui/shell.rs#L894)

```rust
fn zoom_view_at(state: &mut AppState, canvas_rect: Rect, at: egui::Pos2, factor: f32)
```

**What it does.** Multiplies the view zoom by `factor` (clamped to 0.05–64×) while
keeping the document point under the screen point `at` exactly where it is — the usual
"zoom towards the mouse" behaviour.

**How it works.** It converts `at` to document coordinates with the old view, changes
the zoom, converts that document point back to the screen with the new view, and adds
the difference to the pan:
```rust
let before = Xform::new(state, canvas_rect).screen_to_doc(at);
state.view.zoom = (state.view.zoom * factor).clamp(0.05, 64.0);
let after = Xform::new(state, canvas_rect).doc_to_screen(before.0, before.1);
state.view.pan += at - after;
```
(`Xform`, documented later on this page, is the doc↔screen transform.)

**Used by:** [`draw`](#draw) (drag zoom) and [`wheel_map`](#wheel_map) (wheel zoom).

### `panel_key`
[src/ui/shell.rs:907](../../../src/ui/shell.rs#L907)

```rust
fn panel_key(id: PanelId) -> &'static str
```

**What it does.** A fixed string id for each panel window (`"panel_tools"`,
`"panel_layers"`, …). Without it, egui would derive the window's id from its title,
which contains an icon glyph — so changing an icon or a title would silently lose every
saved panel position.

**Used by:** [`panel_area`](#panel_area).

### `panel_content`
[src/ui/shell.rs:921](../../../src/ui/shell.rs#L921)

```rust
fn panel_content(state: &mut AppState, ctx: &egui::Context, ui: &mut egui::Ui, id: PanelId)
```

**What it does.** Draws a panel's body by calling the matching content function:
Tools → [`tools_content`](#tools_content), Brush and Color → [`brush_content`](#brush_content),
Layers → `layers_content`, Onion → [`onion_content`](#onion_content), X-sheet →
`xsheet_content`, Timeline → [`timeline_content`](#timeline_content), Camera →
`camera_content`.

**Used by:** [`panel_window`](#panel_window).

### `padded_pos`
[src/ui/shell.rs:990](../../../src/ui/shell.rs#L990)

```rust
fn padded_pos(rect: Rect, e: PanelEdges, others: &[Rect]) -> Option<egui::Pos2>
```

**What it does.** The "snap" for floating panels. Given where a panel is (`rect`),
returns where it should settle — or `None` if it should stay put. On each axis, a spot
within [`PANEL_SNAP_PX`](#const-panel_snap_px) pulls the panel onto it. Candidate spots
are: one padding in from the window edge; one padding from the side of another panel
level with it (`others`); and, if the padding just changed, the new padding for a panel
that was held at the old one. A panel under the top bar treats the bar's bottom as the
top edge, and a panel overlapping the bar is pushed below it. Nothing pulls a panel out
of the padded window, and a panel too big to fit is left alone.

**Parameters.**
- `rect` — the panel's current rectangle.
- `e` — the frame's [`PanelEdges`](#struct-paneledges) (window, padding, old padding, top bar).
- `others` — the rectangles of the other visible panels.

**Returns.** `Some(new top-left corner)` if it moves by more than half a point, else `None`.

**How it works.**
1. Each candidate spot is a tuple `(place, where the old padding had it, rank)`. Ranks
   are local constants: `EDGE = 0` (window edge), `HANG = 1` (just under / right of a
   panel), `REST = 2` (just above / left of a panel). Lower rank wins, so the window
   edges beat other panels, and "hanging under a panel" beats "resting on one" — which
   makes a stack close up under a panel that shrinks.
2. The `pick` closure first looks for a spot the panel was exactly at under the old
   padding (so it moves with the padding); failing that, the in-reach spot with the
   lowest rank, nearest first.
3. **X axis:** if the panel fits horizontally, candidates are the left and right padded
   edges, plus `other.right() + pad` and `other.left() - pad - width` for every panel
   that overlaps it vertically.
4. **Y axis:** the same with the top (or the bar's bottom + pad), the bottom, and above/
   below panels that overlap it horizontally at its new x. If nothing snaps but the panel
   overlaps the bar, it is moved just under the bar.

**Example.** Window 1600 wide, padding 8, a panel 250 wide at x = 1335 (right edge
1585). The right padded spot is 1600 − 8 − 250 = 1342, which is 7 points away (≤ 20), so
the panel snaps to x = 1342. A panel at x = 700 is 642 points from either edge and stays.

**Used by:** [`panel_window`](#panel_window), and the `panel_padding_tests` tests.

### `panel_window`
[src/ui/shell.rs:1062](../../../src/ui/shell.rs#L1062)

```rust
fn panel_window(
    state: &mut AppState,
    ctx: &egui::Context,
    id: PanelId,
    default_pos: [f32; 2],
    width: f32,
    open: bool,
    resized: Option<Rect>,
    edges: Option<PanelEdges>,
)
```

**What it does.** Draws one panel as a floating, draggable, resizable, collapsible
`egui::Window`, then fills it with [`panel_content`](#panel_content). It also keeps the
panel tidy: after a window resize it re-sticks the panel to its nearest edge, and it
keeps the padding from the window edges and other panels. (`#[allow(clippy::…)]`
silences the linter's warning about having more than seven parameters.)

**Parameters.**
- `id` — which panel.
- `default_pos`, `width` — where and how wide it first opens (egui remembers it after).
- `open` — whether it starts expanded (`true`) or collapsed to its title bar.
- `resized` — the previous window rect, on frames where the window changed size.
- `edges` — the padding rules; `None` turns the padding behaviour off.

**How it works.**
1. Returns if the panel is hidden (`state.panel_shown(id)`).
2. Builds the window with the panel's icon + title, its stable id
   ([`panel_area`](#panel_area)), a close button (`open(&mut shown)`) and the shared
   `floating_frame()` look. This is egui's *builder* style: each `.something()` returns
   the window with one more option set, like a chain of jQuery calls.
3. If the window was resized, forces the position from [`resticked_pos`](#resticked_pos).
4. Otherwise, once the mouse is released (or immediately, if the padding is changing),
   asks [`padded_pos`](#padded_pos) whether the panel should snap, passing the rects of all
   other shown panels (`PanelId::ALL`, filtered).
5. Keeps the panel inside the padded window (`constrain_to`) — except on its very first
   frame, when egui lays it out at a stand-in size.
6. Gives Layers, X-sheet and Onion a starting height (their lists fill the panel's
   height), caps the Tools panel's width at [`TOOLS_MAX_W`](#const-tools_max_w), and
   stops the Timeline getting narrower than its toolbar (whose width
   `timeline_content` stores in egui's temporary memory under
   [`timeline_bar_id`](#timeline_bar_id)).
7. Shows the window; inside, calls [`drag_by_title_only`](#drag_by_title_only) first,
   then [`panel_content`](#panel_content).
8. If the user clicked the close button, records the panel as hidden.

**Used by:** [`draw`](#draw), once per panel.

### `panel_area`
[src/ui/shell.rs:1154](../../../src/ui/shell.rs#L1154)

```rust
fn panel_area(state: &AppState, id: PanelId) -> egui::Id
```

**What it does.** The egui id a panel's window is stored under: its [`panel_key`](#panel_key),
combined with the layout *generation* (`state.panel_layout_gen`) once "Reset panel
layout" has been used. Bumping the generation gives every panel a brand-new id, so egui
forgets the old positions and they reopen at their defaults.

**Used by:** [`draw`](#draw), [`panel_window`](#panel_window), [`wheel_map`](#wheel_map) and
many tests.

### `drag_by_title_only`
[src/ui/shell.rs:1172](../../../src/ui/shell.rs#L1172)

```rust
pub(crate) fn drag_by_title_only(ui: &mut egui::Ui)
```

**What it does.** Makes a window move only when dragged by its title bar. Normally egui
moves a window when you drag any empty spot inside it (the margins, the gap between two
sliders), so a slightly-missed grab on a slider dragged the whole panel. This lays an
invisible drag-catching rectangle over the window body. Because it is added *first*, it
sits above egui's move handle but below every control added after it. The outer edges
are left clear so they still resize the window. Call it first thing in a window's
contents; a collapsed window has no contents, so it still drags from its header.

**How it works.** Reads the window's outer rect from egui memory (the contents' layer id
is the window's id), shrinks it by the resize-grab radius on the sides and bottom,
temporarily widens the clip rect to that body rect (the normal clip would leave the
margins draggable), and registers a `Sense::drag()` interaction there.

**Used by:** [`panel_window`](#panel_window), `brush_settings_window`, `settings_window`,
`grid_settings_window`, and `krita_import_window` in [ui/krita_import.rs](krita_import.md).

### `resticked_pos`
[src/ui/shell.rs:1199](../../../src/ui/shell.rs#L1199)

```rust
fn resticked_pos(ctx: &egui::Context, area: egui::Id, old: Rect, new: Rect) -> Option<[f32; 2]>
```

**What it does.** After the app window changes from `old` to `new`, works out where a
panel should move so it keeps its gap from the edge it was nearest — a right-hand panel
follows the right edge, a bottom one follows the bottom. Each axis is handled separately
by [`restick_axis`](#restick_axis).

**Returns.** The new top-left `[x, y]`, or `None` if egui has no laid-out size for the
panel yet (a panel's `size` is unknown until it has been drawn once, and without it every
gap would be nonsense). The `?` after each lookup returns `None` early when it is missing.

**Used by:** [`panel_window`](#panel_window).

### `restick_axis`
[src/ui/shell.rs:1229](../../../src/ui/shell.rs#L1229)

```rust
fn restick_axis(lo: f32, hi: f32, len: f32, o_lo: f32, o_hi: f32, n_lo: f32, n_hi: f32) -> f32
```

**What it does.** One axis of [`resticked_pos`](#resticked_pos). Returns the panel's new
starting coordinate.

**Parameters.** `lo`/`hi` — the panel's start and end on this axis; `len` — its length;
`o_lo`/`o_hi` — the old window's start and end; `n_lo`/`n_hi` — the new window's.

**How it works.**
1. Measures the gap to each old edge.
2. If the two gaps are within 8 points of each other (local const `CENTRED_TOL`), the
   panel was centred, so it is re-centred in the new window (this keeps the bottom-centre
   Timeline centred).
3. Otherwise it keeps the smaller gap: from the start edge, or from the end edge.
4. Clamps so the panel never ends up off the new window (the `.max(n_lo)` guards a panel
   wider than the window, which would otherwise make the clamp range backwards).

**Example.** A panel at x 1000–1260 in a 1280-wide window has gaps 1000 (left) and 20
(right), so it belongs to the right edge. Widen the window to 1600 and it moves to
1600 − 20 − 260 = 1320.

**Used by:** [`resticked_pos`](#resticked_pos), and the restick tests.

### `tools_content`
[src/ui/shell.rs:1247](../../../src/ui/shell.rs#L1247)

```rust
fn tools_content(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** Draws the Tools panel: two rows of tool buttons, then a header with the
current tool's name, then that tool's options.

**How it works.**
1. **First row** (in `ui.horizontal`): the pen slots and, while there is room, their "+"
   ([`pen_row`](#pen_row)), then the Eraser. **Second row:** Fill (its icon shows bucket or lasso mode), Shape, Lasso (shows a
   magic wand when the wand shape is chosen), Tracker, Perspective, Liquify — each a
   `tool_toggle` with a shortcut tooltip — and the colour-picker toggle, which dispatches
   `Action::PickScreenColor` (a momentary mode that returns to the previous tool).
2. A section header with the tool's name ([`tool_name`](#tool_name)).
3. **Options per tool:**
   - **Fill** — bucket/lasso toggles ([`fill_mode_toggles`](#fill_mode_toggles)), Opacity,
     grain options, lasso options in lasso mode, then (for the bucket, or a lasso with
     "Enclose") Tolerance (bucket only), Expand (px), Gap (px, a logarithmic slider so
     small gaps get most of the track), "Read all visible layers", and a muted line
     saying which layer the fill takes its lines from.
   - **Shape** — Line / Rectangle / Ellipse toggles, a hint (mentions the perspective
     grid when shapes snap to one, plus [`with_mid_drag_hint`](#with_mid_drag_hint)), a
     Thickness row shared with the last pen used (the size lock at its end), and the size strip.
   - **Tracker** — instructions, a "Second point (fix rotation/zoom)" checkbox
     (toggling it clears a half-entered point B), a "Now click point B…" prompt, the
     tracked-frame count, and Stabilize / Clear points buttons (enabled only with ≥ 2 /
     ≥ 1 tracked frames).
   - **Perspective** → `perspective_options`; **Lasso** → `selection_options`;
     **Liquify** → `liquify_options`.
   - **Pens** (every pen slot, and the Eraser) — the Size row (the size lock at its end), the
     size strip, and an Opacity slider limited to two decimals.

`ui.add_enabled(cond, Button::new("…"))` draws a button greyed out when `cond` is false.

**Used by:** [`panel_content`](#panel_content).

### `size_slider`
[src/ui/shell.rs:1445](../../../src/ui/shell.rs#L1445)

```rust
fn size_slider(state: &mut AppState, ui: &mut egui::Ui, label: &str, hover: Option<&str>)
```

**What it does.** Draws the brush size row: the slider (editing `state.brush.radius` over
[`BRUSH_SIZES`](#const-brush_sizes), one decimal) as wide as the row allows next to the
screen-size lock at its end ([`fill_slider_leaving`](#fill_slider_leaving), then
[`brush_size_lock`](#brush_size_lock)), with small tick lines over it for remembered sizes
([`size_marks`](#size_marks)): a strong accent line per *pinned* size, a fainter one per
size drawn at lately. `hover`, if given, becomes the slider's tooltip.

`ui.painter().extend(shapes)` adds a list of ready-made shapes to the panel's drawing.

**Used by:** [`tools_content`](#tools_content) (pens' Size and Shape's Thickness).

### `size_marks`
[src/ui/shell.rs:1463](../../../src/ui/shell.rs#L1463)

```rust
fn size_marks(ui: &egui::Ui, row: Rect, m: &crate::app::brush_sizes::SizeMemory) -> Vec<egui::Shape>
```

**What it does.** Builds the tick lines for [`size_slider`](#size_slider): for a slider row
at `row`, it works out the x position egui gives each value on the slider track and
returns one vertical line per remembered size. Recent sizes (that are not also pinned)
get a short, faint line; pinned sizes a taller accent-coloured one.

**How it works.** The slider occupies the row's first `ui.spacing().slider_width`
points, and egui keeps the handle centre one handle radius (`height / 2.5`) in from both
ends, so the usable track runs from `x0` to `x1`. A value `s` maps linearly:
```rust
let x_at = |s: f32| x0 + (x1 - x0) * ((s - lo) / (hi - lo)).clamp(0.0, 1.0);
```
It must be called right after the slider, so `slider_width` is still the width the row
was laid out with. `SizeMemory` (from [app/brush_sizes.rs](../app/brush_sizes.md)) holds
the `recent` and `pinned` lists.

**Used by:** [`size_slider`](#size_slider).

### `size_strip`
[src/ui/shell.rs:1495](../../../src/ui/shell.rs#L1495)

```rust
fn size_strip(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** Draws the one-row strip of size buttons under the size slider: a pin
button, then the pinned sizes (bold, accent colour), then as many recently used sizes as
fit. Click a size to draw at it; right-click to take it off the strip (unpinned and out of
the recent ones, through `AppState::forget_size`); the pin button pins the current size
(greyed out if it already is pinned, and its tooltip names the `SizeSwap` key if bound). It stays one row on purpose: wrapping would grow the panel for good.

**How it works.**
1. Collects `(size, pinned)` pairs: pinned first, then recent sizes not already listed
   (`same_size` compares sizes with a small tolerance).
2. In a horizontal row, measures each label's width before adding it and stops
   (`break`) when it would not fit in `ui.available_width()`.
3. The clicks are only *recorded* inside the closure (`pick`, `pin_size`, `forget`), then
   applied after it — setting `state.brush.radius`, or calling `state.toggle_pinned_size` or
   `state.forget_size`. This
   "collect, then apply" shape is common in egui code: it keeps the borrow of `state`
   used by the drawing code separate from the code that changes it.

**Used by:** [`tools_content`](#tools_content).

### `fill_mode_toggles`
[src/ui/shell.rs:1567](../../../src/ui/shell.rs#L1567)

```rust
fn fill_mode_toggles(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** The Fill tool's two modes side by side as selectable labels: **Bucket**
("fill the area a press lands in") and **Lasso** ("drag round what to fill, or tap it out
corner by corner"). Clicking one calls `state.set_fill_mode`. The tooltip mentions that
pressing the Fill tool's key again swaps the two. (The `///` comment just above this
function in the source, about "Lock brush to screen size", belongs to
[`brush_size_lock`](#brush_size_lock).)

**Used by:** [`tools_content`](#tools_content).

### `fill_grain_options`
[src/ui/shell.rs:1593](../../../src/ui/shell.rs#L1593)

```rust
fn fill_grain_options(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** The fill's paper-grain controls. A **Grain** slider (0 = flat colour)
lets the paper texture show through the fill like a pencil's tooth — it is Krita's
paper, the one the "Pencil-5 Tilted" brush draws on. Only when grain is above 0 does it
also show **Grain size** and **Grain contrast** (logarithmic sliders over
`fill::GRAIN_SIZES` / `fill::GRAIN_CONTRASTS`) and a **Boil per drawing** checkbox: each
drawing gets its own patch of paper, so the grain changes from drawing to drawing
("boil" is the animator's word for that shimmer).

**Used by:** [`tools_content`](#tools_content).

### `lasso_fill_options`
[src/ui/shell.rs:1623](../../../src/ui/shell.rs#L1623)

```rust
fn lasso_fill_options(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** The lasso fill's switches: **Enclose & fill** (fill only the areas the
lines inside the lasso shut in, not the gap between them and the lasso; holding its key
as the lasso lands does the opposite), **Behind existing paint** (the fill goes under
what is already drawn, as if on a layer below), and a muted how-to line naming the
erase and 15°-snap keys.

**Used by:** [`tools_content`](#tools_content).

### `brush_size_lock`
[src/ui/shell.rs:1658](../../../src/ui/shell.rs#L1658)

```rust
fn brush_size_lock(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** "Lock brush to screen size" (`state.lock_brush_to_view`) as a small monitor
icon at the end of a size row, [`LOCK_W`](#const-lock_w) wide, lit in the accent colour while on;
what it does is only in its tooltip. On, the brush keeps the same size on screen at every zoom.
Off (the default), size is in document pixels, so a line has the same weight in the exported
frame whatever zoom it was drawn at.

**How it works.** It reserves a rect of exactly `LOCK_W` (`allocate_exact_size`) and paints the
background and glyph itself, instead of adding a `SelectableLabel`. A widget sizes itself to its text
plus padding, which came out a few points wider than the room the slider before it leaves; the row
then overflowed, the panel grew, the filling slider took the new width, and the panel grew again
every frame (`size_row_tests` guards this).

**Used by:** [`size_slider`](#size_slider) (pens' Size, Shape's Thickness) and `liquify_options`.

### `swatch_strip`
[src/ui/shell.rs:1684](../../../src/ui/shell.rs#L1684)

```rust
fn swatch_strip(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** The pinned colour swatches under the colour wheel: 16-point squares
(local const `SIZE`) that wrap onto more rows. Click one to use it; right-click for a
"Remove swatch" menu; the `+` button pins the current colour (disabled if already
pinned; when the palette is full, `AppState::MAX_SWATCHES`, the oldest swatch drops).
The current colour's swatch gets a thicker accent outline, so even a palette of
near-identical greys shows where you are. Hovering a swatch shows its colour value.

**How it works.** Each swatch is drawn by hand: `ui.allocate_exact_size(…, Sense::click())`
reserves a square and returns its rect and a `Response`, then the painter fills and
outlines it. As in [`size_strip`](#size_strip), the click and the removal are recorded in
local `Option`s and applied after the row (`set_brush_color`, `palette.remove`).

**Used by:** [`brush_content`](#brush_content).

### `brush_content`
[src/ui/shell.rs:1742](../../../src/ui/shell.rs#L1742)

```rust
fn brush_content(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** Draws the Brush (and Color) panel: the colour wheel with its harmony
set (`color_wheel::color_panel` from [ui/color_wheel.rs](color_wheel.md)), the
[`swatch_strip`](#swatch_strip), a "Brush settings…" button that opens the Brush settings
window (`state.show_brush_settings = true`) with the tablet status icon beside it
([`tablet_status`](#tablet_status)), and the canvas backdrop row ([`backdrop_row`](#backdrop_row)).
The button is sized to leave exactly [`TABLET_ICON_W`](#const-tablet_icon_w) for the icon.

**Used by:** [`panel_content`](#panel_content).

### `tablet_status`
[src/ui/shell.rs:1778](../../../src/ui/shell.rs#L1778)

```rust
fn tablet_status(state: &AppState) -> (&'static str, Color32, &'static str)
```

**What it does.** Describes the pen tablet's state as an icon, its colour and a tooltip.
It has three states, not two, because a driver that opens but never sends data draws
exactly like a mouse, and "Tablet active" would then be a lie:
- pen packets are arriving → pen icon, accent colour, "Tablet active";
- a tablet context is open but no packets lately → warning icon, "Tablet idle" (normal
  while the pen is away from the tablet; if it lasts while drawing, the driver is not
  sending *Wintab* data — Wintab is the Windows tablet API — and strokes fall back to the
  mouse);
- no tablet driver → cursor icon, "Mouse mode" (fixed pressure, no tilt).

**Used by:** [`brush_content`](#brush_content).

### `backdrop_row`
[src/ui/shell.rs:1806](../../../src/ui/shell.rs#L1806)

```rust
fn backdrop_row(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** One row for the canvas *backdrop* (the colour behind the canvas): its
label, a colour button (`color_wheel::hsl_edit_button_f32` editing `state.bg_color`), and
its opacity as a percentage you can drag or type (`DragValue`, 0–100%, stored as
`state.bg_opacity` 0–1). Below 100% the desktop shows through the window. The row is laid
out right to left, so when the panel is narrow the "Backdrop" label is what gets cut
short (`truncate()`).

**Used by:** [`brush_content`](#brush_content).

### `fit_sliders`
[src/ui/shell.rs:1843](../../../src/ui/shell.rs#L1843)

```rust
fn fit_sliders(ui: &mut egui::Ui, label: &str, extra: f32)
```

**What it does.** Narrows the sliders added next in `ui` so one slider plus its value box
and `label` fits the width left (minus `extra` for anything else on the row). It never
makes them wider than the theme's slider width. Without it, a narrow panel widens to fit
a theme-width slider the first time it shows, and egui never shrinks it back.

**How it works.** Measures the text `"0.000"` (a typical value box) and the label with
the current fonts, adds item spacing, `extra` and a 2-point margin — all rounded *up*,
since a row even a fraction too wide makes the panel grow a point every frame — then sets
`ui.spacing_mut().slider_width` to what is left, clamped to between 40 points and the
theme width.

**Used by:** [`fill_slider`](#fill_slider), `layer_selection_actions` and `layers_content`.

### `fill_slider`
[src/ui/shell.rs:1865](../../../src/ui/shell.rs#L1865)

```rust
fn fill_slider(ui: &mut egui::Ui, label: &str, slider: egui::Slider<'_>) -> egui::Response
```

**What it does.** Adds `slider` so it is as long as its row has room for — growing as well
as shrinking, unlike [`fit_sliders`](#fit_sliders). Returns the slider's `Response`.

**How it works.** A slider that fills its row can feed back on itself: a row one point too
wide grows the panel, which gives the slider one more point, and so on across the screen.
So the function remembers, in egui's temporary memory under an id made from the label,
how much the row came out wider than the room it had (`over`), and gives that much back
on every later frame. The panel therefore grows at most once, by that amount.
```rust
let over = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(0.0);
// … lay out the slider with `over` held back …
let past = resp.rect.width() - room;
if past > 0.0 { ui.data_mut(|d| d.insert_temp(id, over + past.ceil())); }
```
`Slider<'_>` has a *lifetime* parameter: the slider holds a borrowed `&mut` to the number
it edits, and `'_` lets the compiler work out how long that borrow lasts
([Lifetimes](../../rust-primer.md#lifetimes)).

**Used by:** [`size_slider`](#size_slider) and the pens' Opacity in [`tools_content`](#tools_content).

### `fill_slider_leaving`
[src/ui/shell.rs:1870](../../../src/ui/shell.rs#L1870)

```rust
fn fill_slider_leaving(ui: &mut egui::Ui, label: &str, slider: egui::Slider<'_>, after: f32) -> egui::Response
```

`fill_slider`, leaving `after` points of its row for what follows it.

### `timeline_bar_id`
[src/ui/shell.rs:1885](../../../src/ui/shell.rs#L1885)

```rust
fn timeline_bar_id() -> egui::Id
```

**What it does.** The egui memory key (`"timeline_bar_width"`) under which
[`timeline_content`](#timeline_content) stores the toolbar's full width, so
[`panel_window`](#panel_window) can use it as the Timeline panel's minimum width.

### `timeline_content`
[src/ui/shell.rs:1889](../../../src/ui/shell.rs#L1889)

```rust
fn timeline_content(state: &mut AppState, ctx: &egui::Context, ui: &mut egui::Ui)
```

**What it does.** Draws the Timeline panel: one toolbar row, then the layer tracks
(drawn by `tracks::show` in [ui/tracks.rs](tracks.md)) filling the rest. There is no frame
slider — the ruler in the tracks scrubs.

**How it works.** The toolbar, left to right:
1. **Transport** — Play/Pause ([`play_button`](#play_button); Shift-click pauses where it
   stops, `play_pause_here`, instead of going back to where playback began), Loop
   ([`loop_toggle`](#loop_toggle)).
2. **Navigation** — go to loop start, step back / forward by the step size
   (`frame_step_delta`), previous / next drawing key. The key targets are worked out
   *before* the `add_enabled_ui` closures, because those closures borrow `state` while
   they run.
3. **Step size** — a `DragValue` "×N" (1–999) shared by the arrows and the insert buttons.
4. **Whole-timeline edits** (every layer) — insert N frames before this one, add N
   frames, duplicate, delete.
5. **Active-layer edits** — insert N frames on this layer (the drawing at the playhead
   holds longer, later ones slide) and remove N frames from the playhead.
6. **Right-aligned group** (laid out right to left) — fps (1–60), the frame count in a
   slot four digits wide (so the bar keeps its width as the scene grows), the current
   frame field and a clock icon. The frame field accepts arithmetic through
   `expr::eval` ([ui/expr.rs](expr.md)): `22/2`, `8+12`, and relative forms like `+12` or
   `/2` measured from the frame at the start of the edit (`base`, frozen before the widget
   is built). `update_while_editing(false)` means the value is applied on Enter, not on
   every keystroke.
7. A right-to-left group that runs out of room keeps going left *over* the buttons and
   adds nothing to the row's width, so the function measures both groups, expands the row
   to include them, and stores that width under [`timeline_bar_id`](#timeline_bar_id).

Under the toolbar it gives the tracks the remaining height minus
[`title_bar_shortfall`](#title_bar_shortfall), so dragging the panel's side edge does not
make it grow taller each frame.

**Used by:** [`panel_content`](#panel_content).

### `title_bar_shortfall`
[src/ui/shell.rs:2098](../../../src/ui/shell.rs#L2098)

```rust
fn title_bar_shortfall(ctx: &egui::Context, id: PanelId) -> f32
```

**What it does.** How much taller a panel's title bar is than egui budgets for — the
work-around for an egui 0.29 resize quirk. While a window edge is dragged, egui sizes the
body as the outer rect minus the frame and a title bar one Heading-text row tall; but the
title bar it actually lays out is at least `interact_size.y` tall, which this app's theme
makes taller. A body that fills the height it is given ends up that much too tall, and
because each frame's drag starts from the last frame's rect, a sideways drag would grow
the panel by that amount every frame. Content that leaves this much height unused comes
out exactly the size dragged. It looks up the panel's title with
[`panel_meta`](#panel_meta) and calls [`title_shortfall`](#title_shortfall).

**Used by:** [`timeline_content`](#timeline_content), [`onion_content`](#onion_content),
`layers_content`, `xsheet_content`.

### `title_shortfall`
[src/ui/shell.rs:2106](../../../src/ui/shell.rs#L2106)

```rust
pub(crate) fn title_shortfall(ctx: &egui::Context, title: &str) -> f32
```

**What it does.** [`title_bar_shortfall`](#title_bar_shortfall) for any window, given its
title text. It measures the title exactly as `egui::Window` does (a plain `RichText` in
the Heading style) and returns `interact_size.y − that height`, or 0 if negative.

**Used by:** [`title_bar_shortfall`](#title_bar_shortfall), `brush_settings_window`,
`settings_window`, `grid_settings_window`, and `krita_import_window` in
[ui/krita_import.rs](krita_import.md).

### `brush_noise`
[src/ui/shell.rs:2117](../../../src/ui/shell.rs#L2117)

```rust
fn brush_noise(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** The brush's *midtone noise* controls. A **Noise** slider (0 = smooth)
breaks the stroke's soft edge into specks while solid parts stay solid and the paper
around stays clean. Only when noise is above 0 it also shows **Noise size** (speck size in
canvas pixels, over `noise::NOISE_SIZES`), **Noise from** Fine (random specks) or Paper
(specks that follow Krita's paper texture), and **Stipple** (every pixel fully inked or
bare: pure dots, no greys). It starts with a function-local `use` that imports names just
for this function.

`ui.selectable_value(&mut field, Value, "label")` is egui's radio-button style widget: it
shows as selected when `field == Value` and sets `field = Value` when clicked.

**Used by:** [`brush_dynamics`](#brush_dynamics).

### `brush_dynamics`
[src/ui/shell.rs:2150](../../../src/ui/shell.rs#L2150)

```rust
fn brush_dynamics(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** The brush shape and response section of the Brush settings window.
Which controls appear depends on the brush's rasterisation *model*: flow, spacing and tilt
only mean anything to a brush that stamps *dabs* (individual brush-tip stamps; see the
[glossary](../../glossary.md)). (Two older `///` comments, about a playback HUD and brush
presets, sit stacked above this one in the source; they describe other code.)

**How it works.**
1. **Model**: Ribbon (one continuous band, even density, no darkening where a stroke
   crosses itself — like ink) or Dabs (stamps that build up where they overlap — like
   graphite).
2. If the brush is a Krita preset (`BrushMode::Krita`), it shows the Krita brush editor
   (`krita_editor::krita_editor`, [ui/krita_editor.rs](krita_editor.md)) and
   [`krita_notes`](#krita_notes), and stops.
3. Otherwise: Hardness, Softness, Grain, Grain scale, then [`brush_noise`](#brush_noise).
4. **Ends** (Round / Flat caps) — enabled only for Ribbon.
5. **Flow** and **Spacing** — enabled only for Dabs.
6. Pressure → size and its curve (the hover text explains: below 1 the brush is full
   width at once, above 1 it stays thin until you press harder, giving a long taper),
   Pressure → flow and its curve.
7. Tilt → shape and Tilt → size — Dabs only — plus a note that tilt needs a tablet when
   no pen is active.

`ui.add_enabled_ui(cond, |ui| …)` greys out and disables everything inside when `cond` is
false.

**Used by:** `brush_settings_window`.

### `krita_notes`
[src/ui/shell.rs:2230](../../../src/ui/shell.rs#L2230)

```rust
fn krita_notes(state: &AppState, ui: &mut egui::Ui)
```

**What it does.** Small muted notes under a Krita preset's editor: Size and Opacity work as
Krita's own sliders, and (when no pen is active) tilt needs a tablet.

**Used by:** [`brush_dynamics`](#brush_dynamics).

### `diag_row`
[src/ui/shell.rs:2246](../../../src/ui/shell.rs#L2246)

```rust
fn diag_row(ui: &mut egui::Ui, key: &str, value: String)
```

**What it does.** One `key   value` line of the tablet readout: the key in small muted
text, the value in small monospace text.

**Used by:** [`display_diagnostics`](#display_diagnostics), [`tablet_diagnostics`](#tablet_diagnostics).

### `display_diagnostics`
[src/ui/shell.rs:2257](../../../src/ui/shell.rs#L2257)

```rust
fn display_diagnostics(ui: &mut egui::Ui, d: &crate::input::tablet::PenDiagnostics)
```

**What it does.** The monitor part of the tablet readout: the screens, and where the
tablet driver puts the pen on them. The usual reason a pen only reaches part of the
window is a driver "work area" spread over every monitor, and this says so in plain
words — judging by where the cursor actually goes, since drivers misreport their own area.

**How it works.** Prints rows for: every monitor (`W×H @x,y`, marking the primary), the
whole desktop, the tablet *context area* if known, the window handle (and whether finding
it by title gave the same handle), what the driver says its area is, and — if it has been
measured — how far the pen reaches. Then a note: a warning if the pen covers more than one
monitor (`reach_fits_one_monitor` is false), a calm note if the driver names the whole
desktop but the cursor keeps to one monitor, or a hint to hover the pen over the window
to measure. Last: the UI scale (pixels per point), the cursor position in desktop pixels,
and how many times the tablet connection was reopened and why. `{h:#x}` in `format!`
prints a number in hex with a `0x` prefix.

**Used by:** [`tablet_diagnostics`](#tablet_diagnostics).

### `tablet_diagnostics`
[src/ui/shell.rs:2337](../../../src/ui/shell.rs#L2337)

```rust
fn tablet_diagnostics(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** The "what is the tablet driver actually reporting" section of Brush
settings. A release build has no console, so this on-screen readout is the only way to see
the tablet's state when something is wrong. And a tablet with a wrong axis mapping does not
fail — it draws confidently in the wrong place; the last line (pen position minus cursor
position) is the number that settles it.

**How it works.**
1. A **Reconnect tablet** button (`state.pen.reconnect()`).
2. A **Record pen (6 s)** button that starts a `PenRecord`
   ([input/pen_record.rs](../input/pen_record.md)), which logs pen, cursor and pointer
   positions frame by frame; while recording it shows an instruction instead, and after
   it the saved path or the error.
3. [`display_diagnostics`](#display_diagnostics).
4. If there is no tablet backend, a line saying why (and when it retries) and stop.
5. Otherwise rows for: packets this frame / peak / total; the packet layout (byte size,
   field mask, which fields are present — 76 bytes on a 64-bit build means the driver
   granted every field); twist and wheel from the last packet; rejected batches and stray
   packets; how many pen vs cursor samples the last stroke used (a pen stroke should show
   0 cursor samples, since a whole-pixel cursor sample among sub-pixel pen samples shows
   as a kink); pressure; tilt, and the same tilt as Krita's sensors read it (elevation and
   direction, via `tools::krita::sensors`); the driver's claimed x/y mapping; the mapping
   the app actually learned (`fit x` / `fit y`, or "learning"); raw and mapped positions;
   the window's client origin; and a warning if the packet queue overflowed.
6. Finally **pen − cursor** in points: the pen's mapped position (converted from desktop
   pixels to points with `pixels_per_point`) minus the egui cursor position, in the warning
   colour if either axis is more than 4 points off. Its tooltip explains that a number near
   zero in the middle of the screen that grows towards the edges means an inverted axis.

**Used by:** `brush_settings_window` (in a collapsing section).

### `smoothing_controls`
[src/ui/shell.rs:2539](../../../src/ui/shell.rs#L2539)

```rust
fn smoothing_controls(o: &mut SmoothingOptions, ui: &mut egui::Ui, hint: &str)
```

**What it does.** The line smoothing controls, modelled on Krita's freehand tool options.
A drop-down (`egui::ComboBox`) picks the kind:
- **None** — straight lines between raw samples;
- **Basic** (default, as in Krita) — no averaging at all; samples are joined by a cubic
  Bézier curve fitted to the local directions. A clean line comes from sub-pixel tablet
  input and that curve fit, not from averaging the hand away;
- **Weighted** — a gaussian average over recent samples, then the same fit; steadier, but
  the stroke trails the pen;
- **Stabilizer** — Krita's stabilizer (shows [`stabilizer_controls`](#stabilizer_controls)
  instead of the rest).

Below it, enabled only for Weighted: Distance, Distance (fast), Tail aggressiveness, Smooth
pressure and Scale distance with zoom. Dragging Distance also drags Distance (fast) as
long as the two were equal ("linked"), like Krita's linked-ratio button.

**Parameters.** `o` — the smoothing settings of the current pen (every pen slot and the
Eraser has its own); `hint` — the drop-down's tooltip, from [`smoothing_hint`](#smoothing_hint).

**Used by:** `brush_settings_window`.

### `smoothing_hint`
[src/ui/shell.rs:2610](../../../src/ui/shell.rs#L2610)

```rust
fn smoothing_hint(state: &AppState) -> String
```

**What it does.** The smoothing drop-down's tooltip: says the setting belongs to this pen
alone, then lists the keys bound to `StabilizerToggle` and `SmoothingCycle`, if any.
(`hint += &format!(…)` appends to a `String`, like `+=` on a JS string.)

**Used by:** `brush_settings_window`.

### `stabilizer_controls`
[src/ui/shell.rs:2622](../../../src/ui/shell.rs#L2622)

```rust
fn stabilizer_controls(o: &mut SmoothingOptions, ui: &mut egui::Ui)
```

**What it does.** Krita's Stabilizer options: **Distance** (how much movement each point
of the line averages, in milliseconds — logarithmic, 3–1000; keeps the fast distance
linked as in [`smoothing_controls`](#smoothing_controls)), **Delay** and its **Delay
distance** (nothing is drawn until the pen is this far from the line's end — the line
follows "on a string"), **Finish line** (draw on to where the pen lifted), **Stabilize
sensors** (average pressure and tilt too) and **Scale distance with zoom**.

**Used by:** [`smoothing_controls`](#smoothing_controls).

### `play_button`
[src/ui/shell.rs:2649](../../../src/ui/shell.rs#L2649)

```rust
fn play_button(state: &AppState, ui: &mut egui::Ui, icon: &str) -> egui::Response
```

**What it does.** The Play / Pause icon button shared by the Timeline and the mini
timeline. While playing, its tooltip explains that pausing returns to the frame playback
started on, and Shift-click (or the `PlayPauseHere` key) stays where it stops. Returns the
button's `Response`; the caller checks `.clicked()`. (The `///` comment above it in the
source starts with a paragraph about the Loop toggle, which belongs to
[`loop_toggle`](#loop_toggle).)

**Used by:** [`timeline_content`](#timeline_content), [`mini_timeline_window`](#mini_timeline_window).

### `loop_toggle`
[src/ui/shell.rs:2662](../../../src/ui/shell.rs#L2662)

```rust
fn loop_toggle(ui: &mut egui::Ui, state: &AppState) -> egui::Response
```

**What it does.** The Loop toggle shared by both timeline windows, lit when
`state.loop_timeline` is on. One flag covers playback, wheel scrub and the frame-step
actions, so "loop off" means the same however the playhead moves: on, playback repeats and
stepping wraps at both ends; off, playback stops at the end and stepping clamps. The
caller flips the flag when it is clicked.

**Used by:** [`timeline_content`](#timeline_content), [`mini_timeline_window`](#mini_timeline_window).

### `mini_timeline_window`
[src/ui/shell.rs:2675](../../../src/ui/shell.rs#L2675)

```rust
fn mini_timeline_window(state: &mut AppState, ctx: &egui::Context)
```

Draws the **mini timeline bar** shown at the bottom centre when all panels are hidden (Tab): a
borderless egui `Window` anchored to the bottom of the screen, holding a play/pause button
(Shift-click stops where playback is), frame-step buttons, the active tool's icon and name, the
active layer's name, and the row of frame dots from `mini_frame_dots`. `egui::Window::new(...)` is
configured with a builder chain (`.title_bar(false).resizable(false)…`) and then `.show(ctx, |ui| …)`
runs the closure to fill it — the closure *is* the window's contents.

### `tool_icon`
[src/ui/shell.rs:2798](../../../src/ui/shell.rs#L2798)

```rust
fn tool_icon(state: &AppState, tool: ActiveTool) -> &'static str
```

The Phosphor icon (a glyph from the icon font) for a tool, used by the mini bar. A pen slot's is the icon it
was given (`PenSlot::glyph`), which is why it needs `state`.

### `tool_name`
[src/ui/shell.rs:2813](../../../src/ui/shell.rs#L2813)

```rust
fn tool_name(state: &AppState, tool: ActiveTool) -> String
```

The display name of a tool ("Lasso select", or a pen slot's own name such as "Pencil"). Returns an owned
`String`, because a slot's name is the artist's text rather than a literal baked into the program; the other
tools' names are literals turned into a `String` with `.to_owned()`.

### `mini_frame_dots`
[src/ui/shell.rs:2831](../../../src/ui/shell.rs#L2831)

```rust
fn mini_frame_dots(state: &mut AppState, ui: &mut egui::Ui)
```

The mini bar's frame indicator: one dot per frame, the current one filled, in a horizontally
scrollable strip that keeps the current frame in view. Click or drag on the dots to scrub.

### `grid_key_marks`
[src/ui/shell.rs:2915](../../../src/ui/shell.rs#L2915)

```rust
pub(crate) fn grid_key_marks(state: &AppState) -> Option<(Vec<usize>, Color32)>
```

The frames where the active perspective grid has keys, plus the grid's colour, so the timeline can
mark them — only while grids are being shown and the grid has keys. The `?` returns `None` early.

### `key_flags`
[src/ui/shell.rs:2928](../../../src/ui/shell.rs#L2928)

```rust
fn key_flags(state: &AppState, n: usize) -> (Vec<bool>, Vec<bool>)
```

Builds two `n`-long true/false tables: "does the active layer's transform have a key on frame i?"
and the same for the camera. Built once per timeline draw, because asking per frame inside the
draw loop would scan the key list every time (quadratic on long scenes).

### `onion_content`
[src/ui/shell.rs:2946](../../../src/ui/shell.rs#L2946)

```rust
fn onion_content(state: &mut AppState, ui: &mut egui::Ui)
```

Draws the **Onion skin panel**: the −8…0…+8 strip of ghost toggles and opacity bars
(`onion_strip`) at the top, and everything else (`onion_settings`, pins) below. The bars take
whatever height the panel has beyond the content below them (remembered from last frame in egui's
temporary data store), so dragging the panel taller makes the bars taller. The lower part is laid
out in a child `Ui` that claims no space of its own, so adding a pin can't grow the panel forever —
a typical immediate-mode layout trick, since sizes are only known after drawing.

### `onion_settings`
[src/ui/shell.rs:2984](../../../src/ui/shell.rs#L2984)

```rust
fn onion_settings(state: &mut AppState, ui: &mut egui::Ui)
```

The Onion panel's lower section: stepping options, a Reset button, the past/future tint colours and
the pinned frames (`onion_pins_content`).

### `button_width`
[src/ui/shell.rs:3017](../../../src/ui/shell.rs#L3017)

```rust
fn button_width(ui: &egui::Ui, text: impl Into<egui::WidgetText>, min: f32) -> f32
```

How wide a button showing `text` would be (text width plus padding), but at least `min`. Used to
line up rows of buttons.

### `pad_to_right`
[src/ui/shell.rs:3030](../../../src/ui/shell.rs#L3030)

```rust
fn pad_to_right(ui: &mut egui::Ui, w: f32)
```

Adds empty space so the next widget, `w` wide, ends at the row's right edge (minus a pixel). Used
instead of a right-to-left layout, which overlaps on narrow panels, and floored so the row can never
be a hair too wide and grow the panel a little every frame.

### `layer_onion_tip`
[src/ui/shell.rs:3038](../../../src/ui/shell.rs#L3038)

```rust
pub(crate) fn layer_onion_tip(on: bool) -> &'static str
```

The tooltip text for a layer's onion toggle (Layers panel and timeline tracks), depending on whether
it is on. The `\` at line ends continues a string literal onto the next line.

### `strip_offset`
[src/ui/shell.rs:3056](../../../src/ui/shell.rs#L3056)

```rust
fn strip_offset(i: usize) -> Option<(u8, OnionDirection)>
```

Maps a column of the onion strip to the ghost it represents: columns left of the middle are past
frames (−8…−1), the middle is 0 (`None`, the on/off switch), the right are future frames (1…8).
`i.cmp(&r)` compares two numbers and returns `Less`, `Equal` or `Greater`, matched on.

### `onion_strip`
[src/ui/shell.rs:3069](../../../src/ui/shell.rs#L3069)

```rust
fn onion_strip(onion: &mut OnionConfig, key: Option<&str>, bar_h: f32, ui: &mut egui::Ui)
```

Draws the Krita-style onion strip `−8 … 0 … 8`. Each number toggles that ghost and the bar above it
sets its opacity; Alt-click shows only that ghost (again to restore); 0 turns onion skinning on or
off; one drag across several bars sets each one it crosses.

### `onion_pins_content`
[src/ui/shell.rs:3226](../../../src/ui/shell.rs#L3226)

```rust
fn onion_pins_content(state: &mut AppState, ui: &mut egui::Ui)
```

The pinned-frames list for the active layer: add a pin by frame number or from the playhead, then
recolour, hide or remove each pin. Pinned frames always show as ghosts, whatever the range.

### `camera_size_ui`
[src/ui/shell.rs:3319](../../../src/ui/shell.rs#L3319)

```rust
fn camera_size_ui(state: &mut AppState, ui: &mut egui::Ui)
```

The camera frame size controls (the size every export comes out at). Changing it leaves zoom and
keys alone, so the frame simply shows more or less of the canvas.

### `camera_loops_ui`
[src/ui/shell.rs:3386](../../../src/ui/shell.rs#L3386)

```rust
fn camera_loops_ui(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** Draws the inside of the Camera panel's folded "Over loops" section. It
lets the user say how many passes of the animation one camera move lasts (for example a
short walk cycle repeating under one long pan), and what the drawings do while the move
is still going.

**Parameters.**
- `state` — the whole app state; the settings live in `state.project.camera_opts`.
- `ui` — where to draw.

**How it works.**
1. A row with "Move spans [N] loop(s)" — a drag box editing `camera_opts.loops`
   (1 to 999) — and a dropdown of [`Fill`](../doc/camera.md) choices (Loop, PingPong, Hold),
   only enabled when loops is more than 1.
2. A muted note underneath: if loops is 1 it explains what a higher number does; if the
   project has fewer than two camera keys (`camera_loops()` returns 1) it says a move is
   needed; otherwise it works out how many frames the stretched move lasts with
   [`shot_len`](../io/frame_order.md) over the loop range, and shows that in frames and
   seconds (`frames / fps`).

**Used by:** [`camera_content`](../../../src/ui/shell.rs#L3396) (inside its "Over loops"
collapsing header).

### `camera_content`
[src/ui/shell.rs:3424](../../../src/ui/shell.rs#L3424)

```rust
fn camera_content(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** Draws the whole **Camera panel**: the camera decides what part of the
canvas ends up in an exported video. The panel sets the export resolution, the camera's
position/zoom/roll, camera **keys** (saved poses on certain frames that the camera
moves between — see [glossary](../../glossary.md)), easing, and view-only overlays.

**How it works.** Top to bottom:
1. A muted intro sentence.
2. A collapsing header "Resolution · W×H" (open by default) holding
   [`camera_size_ui`](../../../src/ui/shell.rs#L3291).
3. "Camera edit mode" checkbox (`state.camera_edit`); turning it on turns
   `state.camera_box` off. A help label shows its shortcut via
   [`combo_text`](../../../src/ui/shell.rs#L47).
4. "Draw camera box" checkbox (calls `state.set_camera_box`) and an indented
   "Free shape" checkbox (`state.camera_box_free`). The help text changes with the free
   shape setting and appends the mid-drag key hint from
   [`mid_drag_hint`](../../../src/ui/shell.rs#L57) when there is one.
5. Number boxes for X, Y, Zoom (0.05–64) and Roll. Roll is stored in radians but edited
   in degrees (`to_degrees()` / `to_radians()`).
6. Four buttons — Fit, Fill, 1:1, Centre — each calling `state.frame_camera(...)` with a
   `CameraFraming` value.
7. "Keep inside canvas" checkbox; a change calls `state.camera_opts_changed()`.
8. A key row: previous/next-key arrow buttons (enabled only when such a key exists; they
   call `state.project.goto(frame)`) and a status like "3 key(s) — keyed on this frame".
9. "Ease out of key" dropdown, enabled only when the current frame has a key. The ease
   belongs to the key and shapes the move *from* it to the next key; picking one calls
   `state.set_camera_key_ease`.
10. Add key / Del key / Reset buttons, and a "Smooth path through keys" checkbox.
11. A folded "Over loops" header holding [`camera_loops_ui`](#camera_loops_ui).
12. A "View" section of checkboxes: look through camera, show camera frame, dim outside
    camera, thirds and safe areas, show active layer bounds.
13. If zoom is above 1, a warning that zooming in upscales (the export size is fixed).

Note the comment at the key row: the previous/next key frames are looked up *before*
`ui.add_enabled_ui(...)`, because that closure borrows `state` while it runs.

**Used by:** [`panel_content`](../../../src/ui/shell.rs#L920) for `PanelId::Camera`; also
drawn directly by tests in this file.

### `layer_row_menu`
[src/ui/shell.rs:3727](../../../src/ui/shell.rs#L3727)

```rust
fn layer_row_menu(ui: &mut egui::Ui, m: &RowMenu, out: &mut Option<RowCmd>)
```

**What it does.** Draws the contents of a layer row's menu (opened from the ⋮ button or a
right-click on the name): only the things that the toolbars and the row itself don't
already show.

**Parameters.**
- `ui` — the open menu's `ui`.
- `m` — facts about the row ([`RowMenu`](#struct-rowmenu)).
- `out` — written with `Some(cmd)` when an item is clicked. The function never acts
  itself; the caller does, after the list is drawn.

**How it works.** A small closure `item` checks whether a button was clicked; if so it
stores the command in `*out` and closes the menu. A group header gets Rename, Ungroup,
"Merge group into one layer" and "Delete group", then returns. A normal layer gets
Rename, "Group selected layers (key)", a separator, and the Krita item — disabled (with
an explanation on hover) when `krita_ok` is false.

**Used by:** [`layers_content`](#layers_content) (both the ⋮ button and the
name's context menu).

### `row_caption`
[src/ui/shell.rs:3769](../../../src/ui/shell.rs#L3769)

```rust
fn row_caption(ui: &mut egui::Ui, text: &str)
```

Draws a small muted caption ("lines from" or "keys with") in a layer row, in a box as
wide as the widest of those two captions so the dropdowns after them line up. It measures
both strings by laying them out as galleys (egui's measured text blocks) and takes the
larger width with `fold(0.0, f32::max)`.

**Used by:** [`layers_content`](#layers_content).

### `icon_button_on`
[src/ui/shell.rs:3788](../../../src/ui/shell.rs#L3788)

```rust
fn icon_button_on(ui: &mut egui::Ui, icon: &str, tooltip: &str, on: bool) -> egui::Response
```

An icon button that stays highlighted (accent colour, "selected" look) while `on` is
true, the same size (28×22) as [`theme::icon_button`](theme.md) so it sits in the same
row. Returns the button's `Response` (egui's "what happened to this widget this frame":
clicked, hovered, dragged…).

**Used by:** [`layer_selection_actions`](#layer_selection_actions) for the
clip and alpha-lock toggles.

### `layer_selection_actions`
[src/ui/shell.rs:3797](../../../src/ui/shell.rs#L3797)

```rust
fn layer_selection_actions(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** Draws the Layers panel's second toolbar and its opacity slider — the
actions that apply to the active layer *and* every other layer picked with it
(Ctrl/Shift-click picks several).

**How it works.**
1. Gets the selected layer indices; texts say "the selected layers" or "this layer".
2. Toolbar row (tight spacing so seven buttons fit a 252-point panel):
   - Five orientation buttons from [`ORIENT_ART`](#const-orient_art) (flip/rotate every
     drawing about the canvas centre). They are disabled unless at least one picked layer
     (or a child of a picked group) is paintable. A click calls
     `state.orient_selected_layers(o)`.
   - **Clip to layer below** and **Lock alpha** toggles drawn with
     [`icon_button_on`](#icon_button_on). They light up according to a
     "lead" layer: the active one if it is a real layer, else the first non-group pick.
     Clicks call `state.toggle_selected_clip()` / `state.toggle_selected_alpha_lock()`.
     *Clipping* shows a layer only where the layer under it has paint; *alpha lock* lets
     painting change only already-painted pixels (see [glossary](../../glossary.md)).
3. Opacity slider (label "opacity ×N" when several are picked), sized by
   [`fit_sliders`](../../../src/ui/shell.rs#L1822). Double-click resets to 100%. A change
   calls `state.set_selected_opacity(v)`.
4. When the slider is not being dragged or typed in, and no row's % badge is in hand
   (`state.opacity_row_uid` is `None`), it calls `state.commit_layer_opacity()` so a whole
   drag becomes one undo step.

**Used by:** [`layers_content`](#layers_content).

### `opacity_badge`
[src/ui/shell.rs:3896](../../../src/ui/shell.rs#L3896)

```rust
fn opacity_badge(ui: &mut egui::Ui, pct: &mut f32) -> egui::Response
```

Draws the small opacity percentage on a layer row as a `DragValue` (a number you can drag
sideways or click to type), 0–100 with a "%" suffix, in small text and only as wide as
its text (from [`opacity_badge_w`](#opacity_badge_w)). `ui.scope` keeps the
style changes local to this widget. Returns the widget's `Response`; `pct` is edited in
place.

**Used by:** [`layers_content`](#layers_content).

### `opacity_badge_w`
[src/ui/shell.rs:3912](../../../src/ui/shell.rs#L3912)

```rust
fn opacity_badge_w(ui: &egui::Ui, opacity: f32) -> f32
```

Returns how wide [`opacity_badge`](#opacity_badge) will be for an opacity
of `opacity` (0–1): measures the text such as `"75%"` in the small text style, adds the
padding on both sides, and rounds up.

**Used by:** [`opacity_badge`](#opacity_badge) and
[`layers_content`](#layers_content) (to reserve room for it).

### `layers_content`
[src/ui/shell.rs:3921](../../../src/ui/shell.rs#L3921)

```rust
fn layers_content(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** Draws the whole **Layers panel**: two toolbars, a folded "Fade other
layers" section, the scrolling list of layer rows (with drag-to-reorder, inline rename,
multi-pick, group folding and per-row menus), and the folded **Transform** and **Layer
canvas** sections underneath. It has no doc comment in the source; it is the biggest
function in this stretch (about 640 lines).

**How it works.**
1. **First toolbar:** add layer above the active one (`state.structural_edit(false, |p|
   p.add_layer_above_active())` — one undoable structural change), delete the selected
   layers, group the selected layers, move layer up / down, merge down (enabled only when
   `state.can_merge_down()`), and a "Fade other layers" toggle (`state.fade_others`).
   Tooltips include the bound shortcut via [`tip`](../../../src/ui/shell.rs#L29).
2. **Second toolbar** and opacity slider:
   [`layer_selection_actions`](#layer_selection_actions).
3. **"Fade other layers"** collapsing header with two sliders (`state.fade.above`,
   `state.fade.below`). Moving either one also switches the fade on, since adjusting an
   amount you can't see is guesswork. This is view-only; it does not change export.
4. **Prepare for the list.** Declares many `Option` "intent" variables (`select`,
   `start_rename`, `drop`, `toggle_fold`, `menu_pick`, `pick`, `row_opacity`, …) that the
   rows will fill in. Copies out everything a row needs to *read* about *other* layers
   into plain `Vec`s first — names, group-header flags, uids, what each clipped layer
   clips to (`clip_base`), key links (`key_link`), whether each layer stays out of Krita
   ([`krita_link::stays_here_at`](../krita_link.md)) — because during the loop one layer
   is borrowed mutably. Then splits the borrow:
   `let (layers, rename) = (&mut state.project.layers, &mut state.layer_rename);`.
5. **Size the list.** The list fills the panel's height minus what the sections below it
   used *last frame* (kept in egui temp memory under `"layers_below_h"`) and minus
   [`title_bar_shortfall`](../../../src/ui/shell.rs#L2072), floored and never below
   [`LAYER_LIST_MIN_H`](#const-layer_list_min_h). The comment explains why: if the list
   were sized to its rows, every new layer would grow the window, and egui never shrinks a
   window below what its content once asked for. Only the list scrolls; the toolbars and
   Transform stay put.
6. **Draw each row** (in `state.project.panel_rows()` order — rows hidden in folded
   groups are skipped). Each row is a rounded `Frame` filled with the accent colour when
   active, a faint tint when picked, otherwise transparent. Text is truncated rather than
   widening the panel. First line:
   - indent if the layer is inside a group;
   - for a group header: a fold chevron (sets `toggle_fold`) and a folder icon;
   - for a clipped layer: an arrow icon whose tooltip names the layer it clips to (or says
     nothing is under it);
   - eye (visibility) and lock buttons that flip `layer.visible` / `layer.locked`
     directly;
   - for non-groups: a light-bulb **reference** toggle (a light-table layer shown dim for
     tracing) and an **onion** toggle (keep onion skin on for this layer while another is
     active; tooltip from [`layer_onion_tip`](../../../src/ui/shell.rs#L3010));
   - the **name**: when this row is being renamed, a single-line text box that grabs focus
     once; Escape cancels, losing focus (Enter or clicking away) commits. Otherwise a
     selectable label that senses both click and drag. It is the drag handle
     (`dnd_set_drag_payload(LayerDrag(i))`); double-click starts a rename; click selects;
     Ctrl/Cmd-click toggles the pick; Shift-click picks a range. Its width is what is
     left after reserving room for the alpha-lock mark, the % badge and the ⋮ button;
   - an alpha-lock checkerboard mark if locked;
   - the % [`opacity_badge`](#opacity_badge) when opacity is under 99.5%
     or while that badge is being dragged (so dragging up to 100% doesn't make it vanish
     mid-drag);
   - the ⋮ button (right-aligned), and the same menu on right-click of the name, both
     drawing [`layer_row_menu`](#layer_row_menu).
7. **Second and third lines** (layers only, not groups):
   - "lines from" dropdown: picks `layer.lines_from`, the layer whose strokes flood fill
     on this layer stops at (so colour can go on a layer under the line art);
   - "keys with" dropdown: links this layer's key jumps (Shift+A / Shift+S, jump buttons,
     wheel) to another layer's drawings, stored by uid; plus a "New cleanup layer" button
     that adds a linked layer over this one for redrawing a rough. *Cleanup* is the
     animation step of redrawing rough sketches as clean lines (see
     [glossary](../../glossary.md)).
8. **Per-row extras after drawing it:** when the active row changes, scroll it into view
   once (remembered under `"layers_active_row"`), so the list still scrolls freely
   otherwise. Every row is also a **drop target**: while a `LayerDrag` hovers it, a 2-px
   accent line is drawn at its top or bottom half (indented when the drop would land
   inside a group); on release `drop = Some((from, to, above))`.
9. **Apply the intents** after the list (the borrow has ended):
   - a drop is first tried on a *clone* of the project (`drop_item` returns whether it is
     allowed); if allowed it is done inside one `structural_edit` (one undo step) and the
     dropped layer becomes active; picks, rename and `expand_cfg` are cleared;
   - fold toggle flips `collapsed`; if the active layer is now hidden, the group row
     becomes active (`panel_row_of`);
   - Ctrl-pick toggles a uid in `state.selected_layers`; Shift-pick adds every row between
     the active one and the clicked one;
   - menu picks run rename / group / `edit_in_krita` / `ungroup_active` / `merge_group` /
     `delete_block`;
   - rename cancel / commit (trimmed, non-empty, changed → one structural edit) / start;
   - plain select makes the layer active and clears picks; key links, cleanup layers and
     row opacity are applied; when a row's % is let go, `commit_layer_opacity()` closes
     the undo step.
10. **Sections under the list** are laid out in a child `ui` that claims no room by
    itself. Its height is saved for next frame (step 5). While the list can still
    shrink, overflow is clipped for one frame instead of claimed, so opening a section
    shrinks the list next frame rather than growing the panel for good. The two sections
    are collapsing headers "Transform"
    ([`layer_transform_section`](#layer_transform_section)) and "Layer
    canvas" ([`layer_canvas_section`](#layer_canvas_section)).

**Example.** The drop-target test in step 8, in JS terms:
```js
const above = pointer.y < row.rect.centerY;          // top half → drop above
const y = above ? row.rect.top : row.rect.bottom;
const inside = (isHeader && !above && !draggedIsHeader) || rowInGroup;
drawLine(inside ? row.rect.left + 14 : row.rect.left, row.rect.right, y);
if (released) drop = { from: dragged, to: i, above };
```

**Used by:** [`panel_content`](../../../src/ui/shell.rs#L920) for `PanelId::Layers`.

### `layer_transform_section`
[src/ui/shell.rs:4568](../../../src/ui/shell.rs#L4568)

```rust
fn layer_transform_section(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** The Layers panel's "Transform" section: the active layer's pose
(position, scale, rotation), its **transform keys** (poses saved on frames that the layer
animates between) and the ease out of the key on this frame.

**How it works.**
1. "Transform mode" (`state.layer_xform`) and "Auto-key transform"
   (`state.auto_key_transform`) checkboxes, plus a help line with the toggle's shortcut.
2. If the active row is a **group**, it only explains that the gestures move the group's
   N layers together about their middle, and returns — a group has no pose of its own.
3. Otherwise, X / Y / Scale (0.01–100) / Rot° drag boxes edit
   `layer.transform` directly (rotation shown in degrees, stored in radians). A small
   closure `settle` sets `xform_settled` only when a drag *stops* or a field loses focus —
   keying on every `changed()` would push one undo entry per mouse move.
4. A status line ("no keys (static)" or "N key(s) — keyed on this frame").
5. If an edit settled and auto-key is on, `state.add_transform_key()`.
6. "Ease out of key" dropdown (enabled only when this frame has a key) calling
   `state.set_transform_key_ease`, same rule as the camera: the ease shapes the move from
   this key to the next.
7. A wrapping row of Add key / Del key / Reset buttons (`add_transform_key`,
   `delete_transform_key`, `reset_active_layer_transform`). It wraps instead of widening
   the panel.

**Used by:** [`layers_content`](#layers_content) (the "Transform" collapsing
header); also drawn by a test in this file.

### `layer_canvas_section`
[src/ui/shell.rs:4710](../../../src/ui/shell.rs#L4710)

```rust
fn layer_canvas_section(state: &mut AppState, ui: &mut egui::Ui)
```

A function defined *inside* another function (allowed in Rust; it is only visible
there). It builds — but does not add — a pixel-size `DragValue` that accepts arithmetic,
like the frame field. The current value is copied into `base` *before* the widget is
built, so an expression starting with an operator (`*2`) measures from where the edit
started. `custom_parser` runs the typed text through [`expr::eval`](expr.md) and rounds
it; `update_while_editing(false)` means the value only changes when you press Enter.
The `'_` in `DragValue<'_>` says the returned widget borrows `value`.

```rust
let base = *value as f64;                    // freeze the starting value
egui::DragValue::new(value)
    .custom_parser(move |s| expr::eval(s, base).map(f64::round))
```
`move |s| …` is a closure that takes ownership of `base` (a copy of the number) so it can
be called later. More: [Closures](../../rust-primer.md#closures).

#### `layer_canvas_section::size_field`
[src/ui/shell.rs:4727](../../../src/ui/shell.rs#L4727)

```rust
fn size_field(value: &mut u32, max: u32) -> egui::DragValue<'_>
```

A function defined *inside* another function (allowed in Rust; it is only visible
there). It builds — but does not add — a pixel-size `DragValue` that accepts arithmetic,
like the frame field. The current value is copied into `base` *before* the widget is
built, so an expression starting with an operator (`*2`) measures from where the edit
started. `custom_parser` runs the typed text through [`expr::eval`](expr.md) and rounds
it; `update_while_editing(false)` means the value only changes when you press Enter.
The `'_` in `DragValue<'_>` says the returned widget borrows `value`.

```rust
let base = *value as f64;                    // freeze the starting value
egui::DragValue::new(value)
    .custom_parser(move |s| expr::eval(s, base).map(f64::round))
```
`move |s| …` is a closure that takes ownership of `base` (a copy of the number) so it can
be called later. More: [Closures](../../rust-primer.md#closures).

### `xsheet_content`
[src/ui/shell.rs:4783](../../../src/ui/shell.rs#L4783)

```rust
fn xsheet_content(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** Draws the **X-sheet panel** — the exposure sheet: a grid with one row
per frame and one column per layer, showing which cell (drawing) each layer exposes on
each frame (see [glossary](../../glossary.md)). Above it is a toolbar of key and drawing
clipboard actions.

**How it works.**
1. Toolbar: insert blank key, insert duplicate key, hold (delete key) — each one
   undoable `structural_edit` on the project — then cut / copy / paste of the active
   drawing. Copy stores `state.project.copy_active_cell()` in `state.cell_clip`; paste is
   enabled only when the clipboard holds something. Cut-then-paste is how a drawing is
   retimed.
2. "Auto-key drawing" checkbox (`state.auto_key_draw`): drawing on a held frame first
   starts a new blank key there.
3. Columns are the Layers panel's rows reversed (bottom layer first). A group's own column
   shows its name with a folder icon and no cells; its layers are marked "›"; folded
   groups' layers are left out.
4. A `ScrollArea` in both directions that takes the panel's full height (minus
   [`title_bar_shortfall`](../../../src/ui/shell.rs#L2072)) whatever the frame count, so
   adding frames never grows the window.
5. Inside an `egui::Grid`: a header row of layer names, then for each frame:
   - even rows get a stripe painted by hand. It reserves a placeholder shape
     (`Shape::Noop`) before the row, then replaces it with a full-width filled rectangle
     once the row's height is known (egui's own striping stops at the last column);
   - a frame-number button (accent with ▸ for the current frame) that jumps there; the
     current frame is scrolled into view only when it changes (remembered under
     `"xsheet_active_frame"`);
   - per layer column: the cell id or "·" for empty, plus "◆" if that layer has a
     transform key on this frame. Clicking a cell makes that frame and layer current.

**Used by:** [`panel_content`](../../../src/ui/shell.rs#L920) for `PanelId::Xsheet`.

### `brush_settings_window`
[src/ui/shell.rs:4945](../../../src/ui/shell.rs#L4945)

```rust
fn brush_settings_window(state: &mut AppState, ctx: &egui::Context)
```

**What it does.** Shows the floating **Brush settings** window (when
`state.show_brush_settings` is true): brush preset, dynamics, smoothing and tablet
diagnostics. Its doc comment explains why it is separate from the docked Brush panel: the
panel is a narrow column for things reached mid-drawing (colour, size, opacity), and a
dozen extra sliders pushed those off the top. These are the settings you set once and
leave.

**Parameters.**
- `state` — app state.
- `ctx` — the egui `Context` (the whole UI), because this creates its own top-level
  `egui::Window` rather than drawing inside a panel.

**How it works.**
1. Returns early if the window is closed. Copies the flag into a local `open`, which
   `egui::Window::open(&mut open)` clears when the user clicks the window's ×, and writes
   it back at the end.
2. The window gets a fixed `Id` (`"window_brush_settings"`) — egui otherwise derives a
   window's saved position from its title, so a later rename would strand it
   off-screen. Default position (260, 120), width 300, resizable, collapsible, framed
   with [`floating_frame`](#floating_frame).
3. [`drag_by_title_only`](../../../src/ui/shell.rs#L1171) so dragging inside doesn't move
   the window, then a vertical `ScrollArea` limited by
   [`title_shortfall`](../../../src/ui/shell.rs#L2080).
4. Sections: "Preset" ([`brush_picker::brush_grid`](brush_picker.md) and an "Import from
   Krita…" button calling `state.open_krita_import()`), "Dynamics"
   ([`brush_dynamics`](../../../src/ui/shell.rs#L2124)), "Smoothing"
   ([`smoothing_controls`](../../../src/ui/shell.rs#L2513) for the tool's own options, or a
   note that every pen slot and the Eraser keep their own when the tool has none), and a
   folded "Tablet" section ([`tablet_diagnostics`](../../../src/ui/shell.rs#L2311)).

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) — deliberately outside the "panels
shown" branch, since hiding the panels is what you do to draw, which is when brush feel
needs adjusting. Also used by [ui/sweep_tests.rs](sweep_tests.md).

### `settings_window`
[src/ui/shell.rs:5002](../../../src/ui/shell.rs#L5002)

```rust
fn settings_window(state: &mut AppState, ctx: &egui::Context)
```

**What it does.** Shows the floating **Settings** window (when `state.show_settings`):
appearance, input options, mouse-button drags, and the full keyboard **shortcut list**
with rebinding, clash warnings and the mouse-wheel map.

**How it works.**
1. Same window set-up as [`brush_settings_window`](#brush_settings_window):
   pinned id `"window_settings"`, default position (360, 80), width 420.
2. [`appearance_section`](#appearance_section).
3. "Input" section: "Invert timeline scroll" and "Shift lock follows the screen"
   checkboxes (whether Shift straightens a drag on the canvas or on the screen when the
   view is rotated).
4. A two-column grid choosing what a **middle** and **right** button drag does
   (`state.middle_drag`, `state.right_drag`), from every
   [`ButtonDrag`](../input/button_drag.md) option. Pen barrel buttons arrive as these same
   buttons.
5. Rebinding help text; while `state.rebinding` is `Some(action)`, a "Rebinding: … —
   press any key…" line.
6. Buttons: **Reset to defaults** (replace `state.shortcuts` with
   `ShortcutMap::default()` and save), **Open shortcuts file** (writes
   `shortcuts.toml` if missing with `ensure_file`, opens it with `open_file`, and stores
   an `Ok("opened …")` or `Err(…)` note in `state.shortcuts_file_note`), **Reload**
   (`state.reload_shortcuts_file()`). The note is shown muted, or in the warning colour
   if it is an error.
7. Lists any lines in `shortcuts.toml` that could not be read (`state.shortcuts.unread`).
8. Computes `state.shortcuts.clashes()` every frame (a map from action to its
   [`Clash`](../input/shortcuts.md) list — cheap, and only while the window is open),
   counts real conflicts, and if there are any shows "N shortcuts clash" with a "Show only
   these" checkbox.
9. A scroll area with two striped 4-column grids: normal (pressed) shortcuts, then "Held
   while dragging" (hold bindings, read while a canvas drag starts — can be bare
   Shift/Ctrl/Alt). Each row is a [`shortcut_row`](#shortcut_row); rows
   without a conflict are skipped when filtering. Last comes
   [`wheel_rows`](#wheel_rows), hidden while filtering (wheel slots can't
   clash).

The `ensure_file(...).and_then(...).map(...).map_err(...)` chain is a `Result` pipeline:
each step runs only if the previous succeeded, and `map_err` turns the error into a
message. More: [Option and Result](../../rust-primer.md#option-and-result).

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) (when the panels are shown and
settings are open); also by tests here and in [ui/sweep_tests.rs](sweep_tests.md).

### `appearance_section`
[src/ui/shell.rs:5225](../../../src/ui/shell.rs#L5225)

```rust
fn appearance_section(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** Settings > Appearance: pick the colour scheme (Graphite, Midnight, Paper
or Custom), edit the five colours of a custom one, and set the panel padding. Changes
apply immediately.

**How it works.**
1. A wrapping row of [`scheme_card`](#scheme_card) previews, one per
   `Scheme::ALL`; clicking another scheme sets `state.color_scheme`.
2. For **Custom**: a grid of five colour buttons (Background, Panels, Buttons and fields,
   Text, Accent) using [`color_wheel::hsl_edit_button`](color_wheel.md); a "Start from…"
   menu that copies a preset's five base colours into `state.custom_colors`; and a
   readability check — it derives the full palette, measures the text-on-panel
   **contrast ratio** with [`theme::contrast`](theme.md) and shows a warning when it is
   under 4.5:1 (the common accessibility threshold).
3. If anything changed, `state.apply_scheme(ctx)` restyles the whole UI.
4. A "Panel padding" drag value (0 to `MAX_PANEL_PADDING` px): how far panels and the top
   bar keep from the window's edges.

**Used by:** [`settings_window`](#settings_window).

### `scheme_card`
[src/ui/shell.rs:5307](../../../src/ui/shell.rs#L5307)

```rust
fn scheme_card(ui: &mut egui::Ui, s: Scheme, p: &crate::ui::theme::Palette, selected: bool) -> egui::Response
```

**What it does.** Paints a small clickable preview (80×54 points) of a colour scheme: its
panel colour on its background, "Aa" in its text colour, a dab of its accent, and the
scheme's name underneath.

**Parameters.** `s` the scheme (for its name), `p` its palette (the colours to paint),
`selected` whether it is the current scheme.

**Returns.** The click `Response` for the card.

**How it works.** Reserves the space with `allocate_exact_size(..., Sense::click())`
(a custom widget: you claim a rectangle and paint it yourself with `ui.painter()`, like
drawing on a `<canvas>` and listening for clicks on its box). The outline is a 2-px accent
when selected, a hover colour when hovered, otherwise thin.

**Used by:** [`appearance_section`](#appearance_section).

### `wheel_rows`
[src/ui/shell.rs:5343](../../../src/ui/shell.rs#L5343)

```rust
fn wheel_rows(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** The "Mouse wheel" part of Settings: one dropdown per modifier slot
(`WheelSlot::ALL`, e.g. plain wheel, Ctrl+wheel…) choosing a `WheelAction` (what turning
the wheel does over the canvas and timelines). Two modifiers at once do nothing.

**How it works.** For each slot it reads the current action (default `Nothing`), shows a
dropdown with every action (each with a hover hint), and if the pick changed calls
`state.shortcuts.set_wheel(slot, pick)` and saves `shortcuts.toml` straight away — like
a rebind.

**Used by:** [`settings_window`](#settings_window).

### `shortcut_row`
[src/ui/shell.rs:5385](../../../src/ui/shell.rs#L5385)

```rust
fn shortcut_row(
    state: &mut AppState,
    ui: &mut egui::Ui,
    action: Action,
    flags: Option<&Vec<Clash>>,
    bad: bool,
)
```

**What it does.** Draws one row of the shortcut grid: the action's name, its key combo
as a button that starts rebinding, an × button that unbinds it, and the clash marker.

**Parameters.**
- `action` — the [`Action`](../input/shortcuts.md) this row is for.
- `flags` — what else shares this combo, if anything.
- `bad` — at least one of `flags` is a real conflict; the combo text is then drawn in the
  warning colour.

**How it works.** The combo button shows the bound combo (or "—"); while this action is
being rebound it shows a prompt instead ("Press key…", or "Press a key, or tap Shift /
Ctrl / Alt…" for hold actions). Clicking toggles `state.rebinding` between this action and
`None` and resets `state.hold_capture`. The actual key capture happens elsewhere, in the
app's key handling. Unbind calls `state.shortcuts.clear(action)` and saves. The last
cell is [`clash_cell`](#clash_cell).

**Used by:** [`settings_window`](#settings_window).

### `clash_cell`
[src/ui/shell.rs:5436](../../../src/ui/shell.rs#L5436)

```rust
fn clash_cell(ui: &mut egui::Ui, action: Action, flags: Option<&Vec<Clash>>)
```

**What it does.** The last cell of a shortcut row: a ⚠ warning icon when the binding
clashes, a muted ⓘ when it only shares its combo on purpose, or an empty label when there
is nothing. Hovering shows one line per clash.

**How it works.** Uses `let Some(flags) = flags else { … return; }`. Each `Clash` becomes
a sentence with a `match`:
- `Clash::Action(b)` on a hold action — "Also held for: … only one of them can start a
  drag"; on a normal action — "Also bound to: … both fire";
- `Clash::Builtin(why)` — a key the app reads outside the shortcut map; shows `why`;
- `Clash::Shared(b, why)` — "Shared with … on purpose: …".
The `if action.is_hold()` after a pattern is a **match guard**: that arm only matches when
the condition is also true.

**Used by:** [`shortcut_row`](#shortcut_row).

### `color_picker_u8`
[src/ui/shell.rs:5461](../../../src/ui/shell.rs#L5461)

```rust
fn color_picker_u8(ui: &mut egui::Ui, label: &str, c: &mut [u8; 3])
```

A colour button (from [`color_wheel::hsl_edit_button`](color_wheel.md)) followed by a
label, on one row. `c` is an RGB colour as three bytes (`[u8; 3]`, a fixed-size array),
edited in place.

**Used by:** [`onion_settings`](../../../src/ui/shell.rs#L2956) and
[`onion_pins_content`](../../../src/ui/shell.rs#L3198).

### `floating_frame`
[src/ui/shell.rs:5468](../../../src/ui/shell.rs#L5468)

```rust
pub(crate) fn floating_frame() -> Frame
```

Returns the look shared by every floating window and panel: the theme's panel fill, a
thin 1-px outline, 10-pt inner margin, 10-pt rounded corners and the theme's window
shadow. `pub(crate)` means other files in this crate may call it, but not outside code.
More: [Modules and visibility](../../rust-primer.md#modules-and-visibility).

**Used by:** [`panel_window`](../../../src/ui/shell.rs#L1061),
[`mini_timeline_window`](../../../src/ui/shell.rs#L2649),
[`brush_settings_window`](#brush_settings_window),
[`settings_window`](#settings_window),
[`grid_settings_window`](#grid_settings_window),
[`menu_window`](../../../src/ui/shell.rs#L8438),
[`new_project_dialog`](../../../src/ui/shell.rs#L8855),
[`export_dialog`](../../../src/ui/shell.rs#L9136),
[`import_range_dialog`](../../../src/ui/shell.rs#L9377),
[`save_error_dialog`](../../../src/ui/shell.rs#L9611), and
[ui/krita_import.rs](krita_import.md).

### `perspective_options`
[src/ui/shell.rs:5480](../../../src/ui/shell.rs#L5480)

```rust
fn perspective_options(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** The tool-options area shown while the **Perspective (Grid) tool** is
active: the switches every grid shares, and the list of grids. Each grid's own settings
open in a separate window ([`grid_settings_window`](#grid_settings_window))
so this list stays short.

**How it works.**
1. A help line on how to drag corners, vanishing points, the horizon and its knobs, and
   that Shift (once dragging) gives 15° steps.
2. Shared switches on `state.perspective`: "Show grids with every tool", "Snap strokes"
   (enabled only when shown), "Vertical lines too" and "Cursor guides" (enabled only when
   snapping), "Auto-key" (moving a grid that already has keys keys this frame) and "Ghost
   neighbour keys" (faint outlines of where the keys on either side put the grid).
3. One row per grid, filled with the accent colour when it is the active one: eye and
   lock toggles, a label like "Grid 2 · wall on 1 ◆" (the kind, or which grid it stands on
   as a wall, and ◆ when it has keys) — click to make active, double-click to open its
   settings — then, right-aligned, a delete button, a settings (gear) button and a
   colour swatch matching how the grid is drawn on the canvas.
4. After the loop, a requested removal runs `cfg.remove(i)` then `cfg.relink()` (fixes the
   walls' links to their parent grids after indices shift).
5. "Add" menu: Perspective, Flat or Isometric (`PerspectiveGrid::fresh(kind)`).
   "Duplicate" copies the active grid, nudges it (and every key's corners) by 0.03 so the
   copy doesn't sit invisibly on top, unlocks it and detaches it from any wall.
6. If a settings button was used, sets `state.show_grid_settings = true`.

**Used by:** [`tools_content`](../../../src/ui/shell.rs#L1246) when the active tool is
`ActiveTool::Perspective`.

### `grid_settings_window`
[src/ui/shell.rs:5639](../../../src/ui/shell.rs#L5639)

```rust
fn grid_settings_window(state: &mut AppState, ctx: &egui::Context)
```

**What it does.** Shows the floating "Grid N settings" window for the **active** grid
when `state.show_grid_settings` is set. It follows the active grid, so picking another
row (or Alt+G) changes what it shows, and like Brush settings it stays up while drawing.

**How it works.** Returns early if closed. If there is no active grid any more (the last
one was deleted) it clears the flag and returns. The title includes the grid number, so
the window has a fixed id (`"window_grid_settings"`) — egui keys a window's remembered
position on its title. Inside: [`drag_by_title_only`](../../../src/ui/shell.rs#L1171) and
a scroll area holding [`grid_settings`](#grid_settings). At the end
`state.show_grid_settings &= open;` (stays true only if it was true *and* the window was
not closed).

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) (outside the panels branch, since
snapping is set while drawing) and [ui/sweep_tests.rs](sweep_tests.md).

### `grid_settings`
[src/ui/shell.rs:5673](../../../src/ui/shell.rs#L5673)

```rust
fn grid_settings(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** The body of the grid settings window for the active grid: its size and
pose, look, lines, what strokes snap to, extra vanishing points, walls, and how it moves
over the shot (follow target and keys).

**How it works.**
1. **Set-up.** Grids are stored in units of *frame heights*, so `frame_h` converts to
   document pixels for the X/Y fields. Gets the grid's mapping to the document on this
   frame (`state.grid_space(g, frame)`), copies the layer names, and declares "intent"
   variables (`pose_edited`, `follow_to`, `wall_on`, `key_add`, `key_del`, `ease_to`) for
   edits that need the whole `state` after the grid's `&mut` borrow ends. Then borrows
   the active grid mutably (`let Some(g) = cfg.active_grid_mut() else { return };`).
2. **Header and size.** "A wall" or "<Kind> grid"; Columns and Rows (1 to
   `MAX_DIVISIONS`).
3. **Pose** (disabled when locked or when it is a wall — a wall's pose is set by its
   floor): X/Y of the grid's centre in document px, Scale against a fresh grid, Rot° —
   all read from the corners every frame; an edit applies the *difference* from what was
   read (`translate`, `scale_by(s / s0)`, `rotate(d - d0)`). Plus rotate ±15° buttons and
   "Reset shape" (a fresh grid's corners, keeping rows, columns and look).
4. **Look.** Colour button, Opacity slider (0.05–1), Line weight slider (0.5–4).
5. **Lines.** "Major line every N". For perspective grids: "Fill canvas" (rays fanned
   round each vanishing point plus floor and ceiling level lines; when on, an indented
   Rays count and a logarithmic Ceiling height slider), "Infinite floor", "Extend lines to
   vanishing points" (disabled when filled or infinite), "Cell diagonals", "Centre
   lines", "Horizon and vanishing points".
6. **Snap strokes to.** Rows, Columns, Verticals (isometric only), and one checkbox per
   extra vanishing point ("VP 3", "VP 4").
7. **Vanishing points** (perspective, not a wall, not locked): add one (up to
   `MAX_EXTRA_VPS`); per point an "On horizon" checkbox (unticking it keeps the point where
   it currently shows by converting its document position back with `s.to_rel(at)`),
   "Rays", and remove.
8. **Walls.** "Stand one on" buttons, one per edge name — a wall is a grid standing on an
   edge of this one, rising toward the first vanishing point off the horizon. If *this*
   grid is a wall, a line "Stands on grid P's … edge" and an unlink button.
9. **Motion** (not for walls, which move with their floor): a "Follows" dropdown —
   Document (stays on the drawing as the camera moves), Camera (stays put in the shot), or
   a Layer (moves with that layer's transform) — a key-count status, "Ease out of key"
   (enabled when this frame has a key), and Add key / Del key.
10. **Apply intents:** `grid_pose_edited`, `set_grid_follow` (a layer pick is resolved
    through `state.follow_layer(index)`, falling back to Document), `add_wall`,
    `set_grid_key_ease`, `add_grid_key`, `delete_grid_key`.

**Used by:** [`grid_settings_window`](#grid_settings_window); also called
directly by [ui/sweep_tests.rs](sweep_tests.md).

### `liquify_options`
[src/ui/shell.rs:6014](../../../src/ui/shell.rs#L6014)

```rust
fn liquify_options(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** Tool options for **Liquify** (a warp brush that pushes pixels around as
if the drawing were soft): mode, brush size, strength, edge softness, and Apply / Cancel
for the warp in progress.

**How it works.**
1. Two mode toggles: **Push** (drag pixels along with the pen) and **Smooth** (ease kinks
   out of the warp), setting `state.brush.liquify_mode`.
2. A logarithmic Size slider (`MIN_RADIUS` to 1000) on `state.brush.radius`, with
   [`brush_size_lock`](#brush_size_lock) at the end of its row.
3. Strength slider shown as 1–100 % but stored as 0.01–1.0
   (`liquify_strength = pct / 100.0`).
4. "Edge falloff" (0–200 px), enabled only when there is a selection that isn't
   "everything": how far in from the selection edge the warp fades out so it never tears
   at the edge.
5. "Show mesh" and "All frames" (apply the same warp to every drawing on the layer)
   checkboxes.
6. Apply (Enter) → `state.land_float()`, Cancel (Esc) → `state.cancel_liquify()`, both
   enabled only while a warp is live (`state.liquify.is_some()`), plus a help paragraph.

**Used by:** [`tools_content`](../../../src/ui/shell.rs#L1246) when the tool is
`ActiveTool::Liquify`.

### `selection_options`
[src/ui/shell.rs:6080](../../../src/ui/shell.rs#L6080)

```rust
fn selection_options(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** Tool options for the **Lasso** (selection) tool: which shape a drag
draws, how the new selection combines with the existing one, and every command that acts
on a selection.

**How it works.**
1. **Shape toggles:** Freehand, Rect, Ellipse, Wand (magic select). Rect and Ellipse
   tooltips add "once dragging: Shift squares it" plus the mid-drag hint from
   [`with_mid_drag_hint`](../../../src/ui/shell.rs#L68). Switching shape first calls
   `state.cancel_gesture()` (a half-placed polygon means nothing to another shape).
2. With the Wand shape picked: [`wand_options`](#wand_options).
3. **Combine-mode toggles** ([`SelOp`](../tools/select_mask.md)): Replace, Add, Subtract,
   Intersect. Each tooltip mentions the key you can *hold* while dragging instead, if one
   is bound.
4. "All frames" checkbox: moving/scaling/rotating the selection lands on every drawing on
   the layer (on each picked layer). When on, a line says how many drawings on how many
   layers it will affect (`state.sel_layers()`, `state.layer_drawings(li)`).
5. Buttons: Select all, Invert, Deselect (enabled with a selection), Fill selection with
   the brush colour (enabled with a selection); a "Fill opacity" slider shared with the
   Fill tool (`state.fill_opacity_mut()` returns a `&mut f32` to edit).
6. A pixel amount (1–200 px) with Grow / Shrink / Feather buttons (feather = soften the
   selection's edge).
7. Flip/rotate buttons from [`ORIENT_ART`](#const-orient_art), whose tooltips end with
   [`orient_scope`](#orient_scope); a click calls `state.orient_art(o)`.
   These change the pixels, unlike the view flip.
8. "Tint outside selection" checkbox, and a help paragraph listing the delete / cut /
   copy / paste shortcuts.

**Used by:** [`tools_content`](../../../src/ui/shell.rs#L1246) when the tool is
`ActiveTool::Lasso`.

### `wand_options`
[src/ui/shell.rs:6228](../../../src/ui/shell.rs#L6228)

```rust
fn wand_options(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** Settings for the **magic select** (wand) shown under the Lasso's shapes
while it is picked. All edit `state.wand`.

- **Tolerance** (0 to `MAX_TOLERANCE`): how far a colour may differ from the clicked one
  and still be picked. (You can also keep the pen down after the click and drag left /
  right.)
- **Gap (px)** (logarithmic, 0 to `fill::MAX_GAP`, only when Contiguous): breaks in lines
  up to this wide count as closed, as with the fill tool. (Or drag up / down after the
  click.)
- **Expand (px)** (0 to `fill::MAX_EXPAND`): grow each pick so a flat colour's selection
  reaches under its anti-aliased (soft-edged) lines.
- **Any paint**: a click on paint takes every touching painted pixel whatever its colour.
- **Contiguous**: only pixels joined to the clicked one; off, every similar pixel anywhere.
- **Read all visible layers**: pick by what all visible layers show together — e.g. on a
  colour layer, select a region walled in by the line layer.

See [tools/wand.rs](../tools/wand.md) for the algorithm.

**Used by:** [`selection_options`](#selection_options).

### `orient_scope`
[src/ui/shell.rs:6289](../../../src/ui/shell.rs#L6289)

```rust
fn orient_scope(state: &AppState) -> &'static str
```

Returns the words that finish the flip/turn tooltips, saying what they would act on right
now: "the selection, about its centre" when a selection is lifted or a bounded selection
mask exists; else "every drawing on this layer, about the canvas centre" when "All
frames" is on; else "this drawing, about the canvas centre". `&'static str` means a string
literal baked into the program, valid forever.

**Used by:** [`selection_options`](#selection_options) and
[`title_menu`](../../../src/ui/shell.rs#L8479).

### `tool_toggle`
[src/ui/shell.rs:6300](../../../src/ui/shell.rs#L6300)

```rust
fn tool_toggle(
    ui: &mut egui::Ui,
    state: &mut AppState,
    target: ActiveTool,
    icon: &str,
    label: &str,
) -> egui::Response
```

**What it does.** One tool button in the Tools panel: lit when `target` is the current
tool; a click switches to it with `state.set_tool(target)`. Returns the button's `Response`, so
[`pen_row`](#pen_row) can hang a right-click menu on it.

**How it works.** If the tool's smoothing setting is the **Stabilizer** (a smoothing mode
where the line trails behind the pen for steadier strokes), the tooltip gets a second line
"Stabilizer on" and a small accent dot of radius
[`STABILIZED_DOT_R`](#const-stabilized_dot_r) is painted at the button's top-right — the
key that turns it on is easy to hit by accident. Clicking the already-selected tool does
nothing.

**Used by:** [`tools_content`](../../../src/ui/shell.rs#L1246).

### `pen_menu_id`
[src/ui/shell.rs:6332](../../../src/ui/shell.rs#L6332)

```rust
fn pen_menu_id(i: usize) -> egui::Id
```

The popup id of pen slot `i`'s menu. Not tied to a `Ui`, so the "+" can open the menu of the slot it adds before that slot's button is drawn.

### `open_pen_menu`
[src/ui/shell.rs:6337](../../../src/ui/shell.rs#L6337)

```rust
fn open_pen_menu(state: &mut AppState, ctx: &egui::Context, i: usize)
```

Open slot `i`'s menu, under its button, and close any other.

### `pen_row`
[src/ui/shell.rs:6345](../../../src/ui/shell.rs#L6345)

```rust
fn pen_row(state: &mut AppState, ui: &mut egui::Ui)
```

**What it does.** Draws one [`tool_toggle`](#tool_toggle) per pen slot, each with the slot's icon and a
tooltip naming it (and its key), then — while there are fewer than `MAX_PENS` — a "+" that adds a slot at
the end (`AppState::add_pen`) and opens its menu.

**How it works.**
1. Each slot's menu is an egui *popup* under its button (`egui::popup_below_widget`), shown while egui's
   memory says that popup is open. A right-click opens it ([`open_pen_menu`](#open_pen_menu)); a click
   anywhere outside the button and the popup, or Esc, closes it.
2. The popup's id ([`pen_menu_id`](#pen_menu_id)) depends only on the slot, so the "+" can open the
   menu of the slot it just added: the button is drawn next frame, and its popup shows then.
3. Copying or removing a slot changes the list the loop walks, so the menu doesn't do it there: it
   records a [`PenOp`](#enum-penop), and the row carries it out after the loop.

**Used by:** [`tools_content`](#tools_content).

### `pen_slot_menu`
[src/ui/shell.rs:6388](../../../src/ui/shell.rs#L6388)

```rust
fn pen_slot_menu(state: &mut AppState, ui: &mut egui::Ui, i: usize, op: &mut Option<PenOp>)
```

**What it does.** The menu of pen slot `i`: a Name field (blank shows as "Pen n"), a grid of the sixteen
[`PEN_ICONS`](../app/pen_slots.md#const-pen_icons), the brush library
([`brush_picker::pen_brush_grid`](brush_picker.md#pen_brush_grid), which puts a picked brush into this slot
whether it is in hand or not), and Duplicate / Remove slot buttons (greyed out at `MAX_PENS` slots / at the
last one). Removing has no undo, so the first click only arms it — the button turns into "Click again to
remove" in the warning colour (`state.pen_remove_armed`) — and the second records the removal.

**Parameters.** `i` — the slot; `op` — where a copy or a removal is recorded for [`pen_row`](#pen_row).

**Used by:** [`pen_row`](#pen_row).

### `shape_kind_toggle`
[src/ui/shell.rs:6448](../../../src/ui/shell.rs#L6448)

```rust
fn shape_kind_toggle(
    ui: &mut egui::Ui,
    state: &mut AppState,
    kind: ShapeKind,
    icon: &str,
    label: &str,
)
```

An icon toggle for one Shape-tool kind (e.g. line, rectangle, ellipse): lit when
`state.brush.shape_kind == kind`, and a click sets it.

**Used by:** [`tools_content`](../../../src/ui/shell.rs#L1246).

### `paint_canvas`
[src/ui/shell.rs:6462](../../../src/ui/shell.rs#L6462)

```rust
fn paint_canvas(state: &AppState, ui: &mut egui::Ui, rect: Rect) -> egui::layers::ShapeIdx
```

**Draws the document on screen** — the biggest drawing function in the app (≈650 lines). It reads
`state` (no `&mut`: painting never changes the app) and paints into `rect` with egui's `Painter`.

**How it works, back to front.**
1. For every visible layer, bottom to top: its onion-skin ghosts (pins first, then far to near, so
   the nearest ghost sits just under the drawing), then the drawing itself as a textured quad placed
   by the layer's transform and the view (`Xform`). Groups below full opacity draw as one merged
   image; clipped layers draw their clip texture; floating selected pixels are drawn just above their
   own layer.
2. The "dim outside selection" tint.
3. Tracker markers, stroke and shape previews (mapped from the drawing's pixel space through the
   layer transform), the Liquify mesh, a reserved slot for the live stroke tail, the Krita
   stabilizer circle.
4. Selection overlays: marching ants, the floating selection's transform box with fixed-screen-size
   handles, the selection being drawn, a lasso fill in progress.
5. Camera guide, document outline and the active layer's bounds.

It returns the index of the slot reserved for the live stroke tail, which `live_tail` fills once the
frame's pen samples are in.

### `live_tail`
[src/ui/shell.rs:7122](../../../src/ui/shell.rs#L7122)

```rust
fn live_tail(state: &AppState, rect: Rect) -> Option<egui::Shape>
```

The part of the live stroke not yet painted into the drawing's texture — from the last painted point
to the pen — drawn as an overlay shape in the slot `paint_canvas` reserved, so the line reaches the
pen tip without lag. Not used for Krita brushes (their dabs don't look like a solid band, and Krita
shows no tail).

### `camera_overlay_lines`
[src/ui/shell.rs:7163](../../../src/ui/shell.rs#L7163)

```rust
fn camera_overlay_lines(q: [egui::Pos2; 4]) -> Vec<[egui::Pos2; 2]>
```

**What it does.** Works out the composition guides drawn inside the camera frame: the
rule-of-thirds lines, the "action-safe" box (90% of the frame) and "title-safe" box (80%), and a
small cross on the centre. These are film/TV framing aids: important action should stay inside
the 90% box and text inside the 80% box. Every line is placed by *fractions* of the frame, so the
guides rotate and scale along with the camera.

**Parameters.**
- `q` — the camera frame's four corners on screen, in TL, TR, BR, BL order.

**Returns.** A list of line segments, each `[start, end]` in screen points (up to 14 of them).

**How it works.**
1. A closure `at(u, v)` maps a fraction across (`u`) and down (`v`) the frame to a screen point
   by blending the top and bottom edges — this works even when the frame is rotated.
2. Thirds: two vertical and two horizontal lines at 1/3 and 2/3.
3. Safe boxes: for insets 5% and 10% from each side, the four edges of the inner box.
4. Centre cross: two short arms along the frame's own across/down directions. The arm length is
   3% of the shorter side, clamped to 3–14 screen points, so both arms are the same length on
   screen however wide the frame is.

**Used by:** [`paint_canvas`](../../../src/ui/shell.rs#L6309) (when the camera overlay toggle,
`state.camera_overlay`, is on) and a test.

### `draw_perspective_grids`
[src/ui/shell.rs:7203](../../../src/ui/shell.rs#L7203)

```rust
fn draw_perspective_grids(state: &AppState, painter: &egui::Painter, xf: &Xform, clip: Rect)
```

**What it does.** Draws every visible perspective grid over the canvas. A perspective grid is a
four-cornered guide (a "quad") whose lines run towards vanishing points, so the artist can draw
buildings and floors in correct perspective (see [glossary](../../glossary.md)). Grids are drawn
last, on top of everything, because they are guides, not artwork. They show while the
Perspective tool is active, or all the time once "Show grids" (`state.perspective.show`) is on.
While the tool is active, the active grid also gets its drag handles and faint dashed outlines
showing where its keyframes on either side put it.

**Parameters.**
- `state` — the whole app state (read only); grids live in `state.perspective.grids`.
- `painter` — the egui painter to draw with.
- `xf` — the canvas view transform ([`Xform`](#struct-xform)), used to turn document points into screen points.
- `clip` — the canvas rectangle on screen; lines are clipped to it and handles outside it are skipped.

**How it works.**
1. Return at once unless the Perspective tool is active or grids are set to always show.
2. For each visible grid: get its corners for the current frame (`state.grid_space` handles
   grids that move over time) and build a `Plane` from them (skip the grid if the corners don't
   make a valid plane).
3. Choose colours: inactive grids fade to 45% while one is being edited or while strokes snap,
   so the grid you are snapping to stands out. `major` lines (every *n*th line) are drawn heavier.
4. If editing the active grid with "ghosts" on, draw dashed outlines of where the previous and
   next keys put it, labelled with their frame numbers.
5. Draw the lines, depending on the grid kind:
   - **rigid** (flat or isometric) grids tile the whole canvas, and the quad is outlined as the
     "home cell";
   - **fill canvas** grids fan rays round each vanishing point (via `fan` and `fade_pieces`,
     which fade rays out where they crowd together) and tile parallel families over the floor and
     ceiling;
   - **infinite** grids tile the whole floor out to the horizon;
   - **extend** carries each line out to its vanishing point (and no further) or the canvas edge.
   Then, unless the grid is "bare" (a fill-canvas grid outside the tool), the diagonals,
   the grid's own lines, dashed centre lines and the quad outline.
6. Draw the horizon line (clipped to the canvas) and a dot on each vanishing point inside the view.
7. Draw any extra vanishing points: their rays, then a diamond marker (numbered from 3 while
   editing, matching the panel's list).
8. While editing the active grid: square corner handles (hollow when locked, or for a wall's two
   base corners, which belong to its floor), round vanishing-point handles, and a tick plus two
   knobs on the horizon for tilting it.

**Used by:** [`paint_canvas`](../../../src/ui/shell.rs#L6309) and a test. Most of the geometry
comes from [tools/perspective.rs](../tools/perspective.md).

### `draw_cursor_guides`
[src/ui/shell.rs:7515](../../../src/ui/shell.rs#L7515)

```rust
fn draw_cursor_guides(
    state: &AppState,
    painter: &egui::Painter,
    xf: &Xform,
    clip: Rect,
    cursor: egui::Pos2,
)
```

**What it does.** While a stroke would snap to the active perspective grid, draws faint dashed
lines from the cursor along every direction the stroke could snap to: through the cursor to each
vanishing point, or straight across the canvas for a family of lines that stays parallel.

**Parameters.**
- `state` — app state; the active grid comes from `state.perspective.active_grid()`.
- `painter`, `xf`, `clip` — as for [`draw_perspective_grids`](#draw_perspective_grids).
- `cursor` — the pointer position in screen points.

**How it works.**
1. Return if there is no active grid.
2. For every snap target (`g.snap_targets(...)`): a vanishing point gives a line from the cursor to
   that point; a direction gives a line from the cursor 100 document pixels along that direction.
3. Clip the line to the canvas (`clip_line`). For a vanishing point, stop at the point itself —
   past it the line means nothing.
4. Draw it as a dashed line in the grid's colour.

**Used by:** [`draw_tool_cursor`](#draw_tool_cursor).

### `fill_outside_quad`
[src/ui/shell.rs:7568](../../../src/ui/shell.rs#L7568)

```rust
fn fill_outside_quad(painter: &egui::Painter, clip: Rect, inner: [egui::Pos2; 4], color: Color32)
```

**What it does.** Darkens everything *outside* a four-cornered shape — used to dim the area
outside the camera frame. It builds a "ring" between the inner quad and a much bigger outer quad
and fills that ring with triangles.

**Parameters.**
- `painter` — where to draw.
- `clip` — the canvas rectangle; the outer quad is made bigger than this so it always covers it.
- `inner` — the quad to leave clear (TL, TR, BR, BL), must be convex.
- `color` — the fill colour (usually semi-transparent black).

**How it works.**
1. Find the centre of the inner quad and its two axis directions (`u` along the top edge, `v`
   down the left edge). If the quad has zero size, use the screen axes instead.
2. Measure how far the clip rect corners and the inner corners reach along `u` and `v`, then pad
   that by `clip.width() + clip.height() + 64` so the outer quad is safely larger than the view.
3. Build the outer quad *in the inner quad's own rotated frame*. The doc comment explains why:
   an axis-aligned outer quad would pair the wrong corners once the camera rolls past about 45°,
   twisting the ring into "bowties" that paint as dark diagonal bands.
4. Put the 4 outer + 4 inner corners into an `egui::Mesh` (a list of coloured vertices plus
   triangle indices), and add two triangles per side — eight in all. Because it is one ring, not
   four overlapping rectangles, the corners don't get darkened twice.

**Example.** The triangle indices, as plain JS:
```js
// vertices 0..3 = outer quad, 4..7 = inner quad
for (let i = 0; i < 4; i++) {
  const a = i, b = (i + 1) % 4;
  indices.push(a, b, b + 4,   a, b + 4, a + 4); // two triangles bridging one side
}
```

**Used by:** [`paint_canvas`](../../../src/ui/shell.rs#L6309).

## `impl Xform`

### `Xform::new`
[src/ui/shell.rs:7654](../../../src/ui/shell.rs#L7654)

```rust
fn new(state: &AppState, rect: Rect) -> Self
```

**What it does.** Builds the view transform for the canvas area `rect` from the current app state.

**Parameters.**
- `state` — the app state: project size, `state.view` (zoom, pan, rotation, flips), and camera settings.
- `rect` — the canvas area on screen.

**Returns.** A ready-to-use `Xform`.

**How it works.**
1. The base scale is the largest that fits the whole document in `rect`.
2. Normally `scale = base × view.zoom`, with the view's own rotation and pan.
3. When **"look through camera"** is on, the scale, rotation and pan come from the camera instead:
   the camera's output frame is fitted to the window, multiplied by the camera's zoom, rotated by
   minus the camera's rotation, and panned to centre on the camera. The comment notes this is not
   a separate drawing path — the camera is the same kind of transform (move, turn, scale), so
   it collapses into the same three numbers and drawing still works while looking through it.
4. Flip flags become signs `-1.0` / `1.0`, and everything is handed to
   [`Xform::from_parts`](#xformfrom_parts).

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) (to publish `view_scale`, and for pan/rotate
gestures), [`zoom_view_at`](../../../src/ui/shell.rs#L893),
[`paint_canvas`](../../../src/ui/shell.rs#L6309),
[`draw_krita_outline`](#draw_krita_outline),
[`draw_tool_cursor`](#draw_tool_cursor),
[`canvas_to_doc_mapping`](#canvas_to_doc_mapping), and many tests.

### `Xform::from_parts`
[src/ui/shell.rs:7701](../../../src/ui/shell.rs#L7701)

```rust
fn from_parts(
    center: egui::Pos2,
    pan: Vec2,
    scale: f32,
    fx: f32,
    fy: f32,
    rotation: f32,
    cw: f32,
    ch: f32,
) -> Self
```

**What it does.** The pure-maths half of [`Xform::new`](#xformnew): builds an `Xform` from raw numbers
without needing an `AppState`, so tests can check the screen↔document round trip across flips and
rotations. It computes `sin`/`cos` of `rotation` once and the document centre (`cw/2`, `ch/2`).
The `#[allow(clippy::too_many_arguments)]` line silences a lint warning about the long argument
list ([Attributes and cfg](../../rust-primer.md#attributes-and-cfg)).

**Used by:** [`Xform::new`](#xformnew) and tests.

### `Xform::doc_to_screen`
[src/ui/shell.rs:7727](../../../src/ui/shell.rs#L7727)

```rust
fn doc_to_screen(&self, x: f32, y: f32) -> egui::Pos2
```

**What it does.** Converts a document pixel position `(x, y)` into a screen point. Steps: move
the origin to the document centre, multiply by `scale` and the flip signs, rotate, then add the
screen centre and pan.

**Example.**
```js
function docToScreen(xf, x, y) {
  const ox = (x - xf.dcx) * xf.scale * xf.fx;
  const oy = (y - xf.dcy) * xf.scale * xf.fy;
  const rx = ox * xf.cos - oy * xf.sin;   // standard 2D rotation
  const ry = ox * xf.sin + oy * xf.cos;
  return { x: xf.center.x + xf.pan.x + rx, y: xf.center.y + xf.pan.y + ry };
}
```

**Used by:** nearly every overlay in this file (grids, lasso, layer corners, cursor).

### `Xform::screen_delta_to_doc`
[src/ui/shell.rs:7737](../../../src/ui/shell.rs#L7737)

```rust
fn screen_delta_to_doc(&self, d: Vec2) -> (f32, f32)
```

**What it does.** Converts a *movement* on screen (a drag delta) into the same movement in
document pixels: undoes the rotation, mirror and scale, but not the position offset (a movement
has no position).

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) when panning a layer or the camera, and a test.

### `Xform::screen_to_doc`
[src/ui/shell.rs:7744](../../../src/ui/shell.rs#L7744)

```rust
fn screen_to_doc(&self, p: egui::Pos2) -> (f32, f32)
```

**What it does.** The exact inverse of [`doc_to_screen`](#xformdoc_to_screen): turns a screen
point (for example the mouse position) into document pixel coordinates.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) (keeping the view centre still while
rotating), [`draw_cursor_guides`](#draw_cursor_guides),
[`canvas_to_doc_mapping`](#canvas_to_doc_mapping), and others.

### `Xform::corners`
[src/ui/shell.rs:7754](../../../src/ui/shell.rs#L7754)

```rust
fn corners(&self) -> [egui::Pos2; 4]
```

**What it does.** The four corners of the whole document on screen, in TL, TR, BR, BL order
(possibly rotated or mirrored).

**Used by:** [`paint_canvas`](../../../src/ui/shell.rs#L6309).

## Free functions

### `canvas_quad`
[src/ui/shell.rs:7770](../../../src/ui/shell.rs#L7770)

```rust
fn canvas_quad(painter: &egui::Painter, state: &AppState, tex: egui::TextureId, corners: [egui::Pos2; 4], tint: Color32)
```

**What it does.** Draws a piece of artwork (a cell, an onion-skin ghost, a group, or a lifted
selection) as a texture stretched onto four screen corners. If the app's own GPU canvas renderer
(`state.canvas_gpu`, see [ui/canvas_gpu.rs](canvas_gpu.md)) is available it draws through that,
because it blends neighbouring texels in gamma space; egui's built-in sampler blends in linear
light, which puts a bright rim on light ink wherever a screen pixel falls between two texels.
If the texture is brand new or was completely re-filled this frame (`state.fresh_full`), or the
GPU path declines, it falls back to [`image_quad`](#image_quad).

**Parameters.**
- `tex` — the egui texture to draw.
- `corners` — where its four corners go on screen (TL, TR, BR, BL).
- `tint` — a colour multiplied into every texel (used for opacity and onion tints).

**Used by:** [`paint_canvas`](../../../src/ui/shell.rs#L6309).

### `image_quad`
[src/ui/shell.rs:7782](../../../src/ui/shell.rs#L7782)

```rust
fn image_quad(painter: &egui::Painter, tex: egui::TextureId, corners: [egui::Pos2; 4], tint: Color32)
```

**What it does.** Draws a texture onto any four-cornered shape (which may be rotated) using
egui's own renderer. It makes a mesh of four vertices — each corner paired with the matching
texture corner `(0,0)`, `(1,0)`, `(1,1)`, `(0,1)` — and two triangles `0,1,2` and `0,2,3`.

**Used by:** [`canvas_quad`](#canvas_quad) (fallback) and
[`paint_canvas`](../../../src/ui/shell.rs#L6309).

### `marching_ants`
[src/ui/shell.rs:7805](../../../src/ui/shell.rs#L7805)

```rust
fn marching_ants(painter: &egui::Painter, pts: &[egui::Pos2], phase: f32)
```

**What it does.** Draws the classic "marching ants" selection border: animated dashes around a
closed loop of screen points. It draws black dashes and white dashes offset by one dash length
(6 points), so the border is visible over both dark and light artwork. Changing `phase` each
frame makes the dashes appear to crawl. Does nothing with fewer than 2 points.

**Parameters.**
- `pts` — the loop's points on screen (the first point is repeated at the end to close it).
- `phase` — how far along the dash pattern starts.

**Used by:** [`paint_canvas`](../../../src/ui/shell.rs#L6309).

### `gesture_path`
[src/ui/shell.rs:7830](../../../src/ui/shell.rs#L7830)

```rust
fn gesture_path(painter: &egui::Painter, pts: &[egui::Pos2], closing: bool)
```

**What it does.** Draws a selection shape while it is still being drawn: a white line over a
wider dark line (readable over ink and empty canvas). If `closing` is true it also draws a faint
line from the last point back to the first, showing that the loop will close itself.

**Used by:** [`paint_canvas`](../../../src/ui/shell.rs#L6309) and
[`draw_tap_lasso`](#draw_tap_lasso).

### `held_lasso`
[src/ui/shell.rs:7847](../../../src/ui/shell.rs#L7847)

```rust
pub(crate) fn held_lasso(state: &AppState, i: &egui::InputState) -> LassoHeld
```

**What it does.** Checks which of the Fill tool's lasso-mode key combos are held right now and
returns them as a `LassoHeld` (three booleans: `erase`, `enclose`, `snap`). Each combo is
whatever the user bound to `FillLassoErase`, `FillLassoEnclose` and `FillPolygonSnap` in the
shortcuts settings. `pub(crate)` means other files in this crate may call it
([Modules and visibility](../../rust-primer.md#modules-and-visibility)).

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) (when a lasso fill drag starts) and
[`paint_canvas`](../../../src/ui/shell.rs#L6309).

### `draw_tap_lasso`
[src/ui/shell.rs:7861](../../../src/ui/shell.rs#L7861)

```rust
fn draw_tap_lasso(
    painter: &egui::Painter,
    xf: &Xform,
    l: &crate::tools::tap_lasso::TapLasso,
    hover: Option<egui::Pos2>,
    next: Option<egui::Pos2>,
)
```

**What it does.** Draws a lasso outline that is being made, shared by the selection Lasso and the
Fill lasso (`TapLasso`, see [tools/tap_lasso.rs](../tools/tap_lasso.md)). A lasso can be drawn by
dragging (a freehand loop) or by tapping corners one at a time (a polygon). It draws the path so
far, closing back to its start. For a polygon between taps it also draws:
the next edge out to `next`, a white dot on each tapped corner, and an accent-coloured ring round
the first corner when the pointer is close enough that a tap there would close the shape.

**Parameters.**
- `xf` — the view transform (lasso points are stored in document pixels).
- `l` — the lasso being drawn.
- `hover` — the pointer position on screen, if over the canvas.
- `next` — where the next polygon edge should go (the pointer, or a snapped corner).

**Used by:** [`paint_canvas`](../../../src/ui/shell.rs#L6309).

### `held_sel_op`
[src/ui/shell.rs:7890](../../../src/ui/shell.rs#L7890)

```rust
pub(crate) fn held_sel_op(state: &AppState, i: &egui::InputState) -> Option<SelOp>
```

**What it does.** Returns the selection mode that a currently held key combo asks for — `Add`,
`Subtract` or `Intersect` (`SelOp`, from [tools/select_mask.rs](../tools/select_mask.md)) — or
`None` if none of those combos is held. It walks a small list of (action, mode) pairs and returns
the first whose shortcut is held.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) (when a Lasso drag starts) and
[`draw_tool_cursor`](#draw_tool_cursor).

### `finish_canvas_drag`
[src/ui/shell.rs:7903](../../../src/ui/shell.rs#L7903)

```rust
fn finish_canvas_drag(state: &mut AppState)
```

**What it does.** Ends whatever the current canvas drag was doing, when the pen lifts or the key
combo holding a gesture is released. It checks, in order: layer-transform drag → commit it;
camera drag → commit it; a button drag (middle/right button picker) → end it; dragging the camera
frame → let go; drawing a camera box → let go. Otherwise, if no view navigation was running, it
calls the "up" handlers for selection, lasso fill and the stroke tool (all of them do nothing
when idle, so a tool change mid-drag cannot leave one stuck). Finally it clears the navigation
flags. `&mut AppState` means it is allowed to change the state
([Ownership and borrowing](../../rust-primer.md#ownership-and-borrowing)).

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) (on drag stop, and when a held combo is let go).

### `layer_screen_corners`
[src/ui/shell.rs:7928](../../../src/ui/shell.rs#L7928)

```rust
fn layer_screen_corners(
    xf: &Xform,
    t: crate::doc::transform::Transform,
    cw: f32,
    ch: f32,
    pw: f32,
    ph: f32,
) -> [egui::Pos2; 4]
```

**What it does.** The screen corners (TL, TR, BR, BL) of a whole cell of size `cw`×`ch`, placed
on a `pw`×`ph` canvas by the layer transform `t` (move/rotate/scale, see
[doc/transform.rs](../doc/transform.md)), then through the view `xf`. A thin wrapper over
[`cell_rect_screen_corners`](#cell_rect_screen_corners) with the full cell rectangle.

**Used by:** [`paint_canvas`](../../../src/ui/shell.rs#L6309) (layers, the camera frame and its
ghost, the camera box).

### `cell_rect_screen_corners`
[src/ui/shell.rs:7943](../../../src/ui/shell.rs#L7943)

```rust
fn cell_rect_screen_corners(
    xf: &Xform,
    t: crate::doc::transform::Transform,
    cw: f32,
    ch: f32,
    pw: f32,
    ph: f32,
    r: [f32; 4],
) -> [egui::Pos2; 4]
```

**What it does.** Like [`layer_screen_corners`](#layer_screen_corners) but for only
part of a cell: the rectangle `r = [x0, y0, x1, y1]` in cell pixels. This is where a texture
cropped to just the drawn area of a cell goes. Each corner is mapped cell → document with
`t.cell_to_doc(...)`, then document → screen with `xf.doc_to_screen(...)`.

**Used by:** [`paint_canvas`](../../../src/ui/shell.rs#L6309) and
[`layer_screen_corners`](#layer_screen_corners).

### `draw_readout`
[src/ui/shell.rs:7964](../../../src/ui/shell.rs#L7964)

```rust
fn draw_readout(painter: &egui::Painter, pos: egui::Pos2, text: String)
```

**What it does.** Draws a small text label beside the pen (14 points right, 12 down from `pos`):
white 12-point text on a rounded dark box, so it reads over any drawing. Used for live values
such as a brush size while adjusting it.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) (button-drag readouts),
[`draw_swatch_pie`](#draw_swatch_pie) and
[`draw_tool_cursor`](#draw_tool_cursor).

### `draw_swatch_pie`
[src/ui/shell.rs:7976](../../../src/ui/shell.rs#L7976)

```rust
fn draw_swatch_pie(
    ctx: &egui::Context,
    palette: &[[u8; 3]],
    cap: usize,
    d: &crate::app::ButtonDragState,
    current: [u8; 4],
)
```

**What it does.** Draws the **swatch pie**: a round colour picker that pops up around the press
point when a button drag is set to "Swatch pie" (see [input/button_drag.rs](../input/button_drag.md)).
The pinned swatches sit as wedges in up to two rings; the wedge under the pen is pulled outward
and outlined in white; the current colour sits in the centre hole, the "cancel" spot, so letting
go there reads as "keep this colour". It draws on egui's *foreground* layer, so it stays above
any panel the pen wanders over.

**Parameters.**
- `ctx` — the egui context (used to get a foreground painter).
- `palette` — the pinned swatch colours as RGB triples.
- `cap` — how many swatches the inner ring holds (passed to `pie_rings`).
- `d` — the button-drag state: `d.anchor` is the pie's centre, `d.pie_hover` the hovered wedge.
- `current` — the current brush colour (RGBA).

**How it works.**
1. `pie_rings` splits the swatches between the inner and outer ring.
2. A dark disc goes behind everything.
3. For each ring, the full circle (`TAU` radians) is divided into equal wedges with a hair of
   space between them; each wedge is drawn by [`pie_wedge`](#pie_wedge), 8 points
   longer and with a white edge if hovered.
4. The current colour fills the centre; its edge is white when no wedge is hovered.
5. With no pinned swatches, a "No pinned swatches" readout appears.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75).

### `pie_wedge`
[src/ui/shell.rs:8031](../../../src/ui/shell.rs#L8031)

```rust
fn pie_wedge(
    painter: &egui::Painter,
    c: egui::Pos2,
    r: (f32, f32),
    a: (f32, f32),
    fill: Color32,
    stroke: Stroke,
)
```

**What it does.** Draws one ring segment (a wedge with its point cut off) around centre `c`,
between radii `r.0` and `r.1` and angles `a.0` and `a.1` (0 = straight up, increasing clockwise).
egui can only fill *convex* shapes, so the wedge is built as a mesh: the arc is split into steps of
about 0.1 radian, each step adds an inner and outer vertex, and two triangles join each step to the
previous. Then the outline is drawn round the outer arc and back along the inner arc.

**Used by:** [`draw_swatch_pie`](#draw_swatch_pie).

### `draw_krita_outline`
[src/ui/shell.rs:8064](../../../src/ui/shell.rs#L8064)

```rust
fn draw_krita_outline(state: &AppState, painter: &egui::Painter, canvas_rect: Rect, pos: egui::Pos2)
```

**What it does.** The cursor for a brush in **Krita mode** (the app's port of Krita's pixel brush
engine, see [tools/krita](../tools/krita/mod.md)): the outline of the brush tip exactly as Krita
draws it. For the Pencil-5 preset that is a thin bar, rotated by the way the pen leans and grown as
it tilts — the same shape the next dab would have (a *dab* is one stamp of the brush tip; see the
[glossary](../../glossary.md)). While hovering, it assumes full pressure, as Krita does; during a
stroke it uses the pen's real pressure.

**Parameters.**
- `state` — app state (pen packet, brush, layer transform).
- `painter` — where to draw.
- `canvas_rect` — the canvas area (to build the [`Xform`](#struct-xform)).
- `pos` — the pointer position on screen.

**How it works.**
1. Read tilt from the latest pen packet (zero if no pen), and pressure (the packet's pressure
   mid-stroke, otherwise `1.0`).
2. Ask the Krita brush for its outline (`state.krita_brush().outline(...)`) at the current
   diameter, pressure, tilt, and the cell's on-screen rotation and mirroring. The outline comes
   back as offsets in cell pixels around the tip.
3. Find which cell pixel is under the pointer (screen → document → cell), add each offset, and
   map each corner back out through the layer transform and the view. This way zoom, rotation and
   flips all apply to the outline.
4. Draw the closed outline black-under-white, and a tiny dot on the hot-spot.

**Used by:** [`draw_tool_cursor`](#draw_tool_cursor).

### `delay_circle`
[src/ui/shell.rs:8111](../../../src/ui/shell.rs#L8111)

```rust
fn delay_circle(painter: &egui::Painter, centre: egui::Pos2, radius: f32)
```

**What it does.** Draws the stabilizer's "delay" radius as a faint double circle (so it reads as a
guide, not as the brush). The *stabilizer* is a smoothing mode where the line trails behind the pen
like it is pulled on a string; with "delay" on, the line only starts moving once the pen has gone
further than this radius. Krita draws the same circle.

**Used by:** [`draw_tool_cursor`](#draw_tool_cursor) and
[`paint_canvas`](../../../src/ui/shell.rs#L6309).

### `draw_tool_cursor`
[src/ui/shell.rs:8120](../../../src/ui/shell.rs#L8120)

```rust
fn draw_tool_cursor(state: &AppState, ui: &egui::Ui, canvas_rect: Rect, pos: egui::Pos2)
```

**What it does.** Paints the active tool's cursor on top of the canvas at `pos`. Which cursor
depends on the tool (a `match` on `state.tool`):

Before that, it also draws the perspective cursor guides (when idle, guides are on and strokes would
snap) and the stabilizer's delay circle (when idle and the stabilizer's delay is on).

**Parameters.**
- `state` — the app state (read only).
- `ui` — the egui `Ui` for the canvas (gives a painter clipped to `canvas_rect`, and input).
- `canvas_rect` — the canvas area on screen.
- `pos` — where to centre the cursor: the pointer, or the press point during a tool-adjust drag
  so the size can be judged against the drawing under it.

**How it works (brush ring size).** The ring radius on screen is
`effective_radius × size curve(pressure) × view scale × layer scale`, at least 2 points.
`effective_radius` is in *cell* pixels, so the layer's own scale must be multiplied in, otherwise
the ring would show the wrong width on a scaled layer. Pressure comes from the pen, or `1.0` for a
mouse.

**Example.** The idea in JS:
```js
const pressure = pen.currentPressure ?? 1.0;
const rCell = Math.max(0.5, effectiveRadius * sizeCurve(pressure));
const r = Math.max(2.0, rCell * view.scale * Math.abs(layer.scale));
ring(pos, r + 0.6, black, 1.5);  // outer dark ring
ring(pos, r, white, 1.0);        // inner light ring
```

**Used by:** [`draw`](../../../src/ui/shell.rs#L75).

### `clipboard_color`
[src/ui/shell.rs:8358](../../../src/ui/shell.rs#L8358)

```rust
pub(crate) fn clipboard_color() -> Option<[u8; 3]>
```

**What it does.** Reads text from the system clipboard (via the `arboard` crate) and tries to parse
it as a colour in any format `parse_color` understands (for example `#hex`, `rgb(…)`, `hsl(…)`, see
[color.rs](../color.md)). Returns `None` if the clipboard can't be opened, has no text, or the text
is not a colour. The `?` after each step means "if this failed, return `None` now"
([Option and Result](../../rust-primer.md#option-and-result)).

**Used by:** [ui/color_wheel.rs](color_wheel.md) (the "paste a colour" button).

### `canvas_to_doc_mapping`
[src/ui/shell.rs:8363](../../../src/ui/shell.rs#L8363)

```rust
fn canvas_to_doc_mapping(state: &AppState, rect: Rect) -> impl Fn(egui::Pos2) -> (f32, f32) + Copy
```

**What it does.** Builds the view transform once and returns a small function that turns any
screen point into document pixels. The return type `impl Fn(egui::Pos2) -> (f32, f32) + Copy`
means "some function value taking a point and returning a pair; it can be copied freely". The
`move` keyword makes the closure keep its own copy of `xf`.

**Example.**
```rust
let canvas_to_doc = canvas_to_doc_mapping(state, canvas_rect);
let (x, y) = canvas_to_doc(pos);
```
```js
const canvasToDoc = ((xf) => (pos) => xf.screenToDoc(pos))(new Xform(state, rect));
const [x, y] = canvasToDoc(pos);
```
More: [Closures](../../rust-primer.md#closures)

**Used by:** [`draw`](../../../src/ui/shell.rs#L75).

### `snapped`
[src/ui/shell.rs:8370](../../../src/ui/shell.rs#L8370)

```rust
fn snapped(state: &mut AppState, doc: (f32, f32)) -> (f32, f32)
```

**What it does.** Passes a stroke point (in document pixels) through `state.stroke_point_doc`,
which applies the Shift axis lock (straight lines) or perspective-grid snapping, and returns the
adjusted point as a tuple.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) (every point of a stroke after the first).

### `doc_to_active_cell`
[src/ui/shell.rs:8379](../../../src/ui/shell.rs#L8379)

```rust
fn doc_to_active_cell(state: &AppState, doc: (f32, f32)) -> (f32, f32)
```

**What it does.** Converts a document point into the active layer's *cell* pixel coordinates (a
cell is one drawing on a layer; see [glossary](../../glossary.md)). It undoes the layer's
transform, so drawing lands in the right place on a layer that has been moved, scaled or rotated.
It sizes the cell with `Project::draw_cell_size`, so on a frame with no drawing yet it maps into
the cell that is about to be created rather than assuming a canvas-sized one. (`AppState` has its
own method of the same name in [app/select.rs](../app/select.md); this is the shell's copy.)

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) (on press and on every drag point) and a test.

### `pen_to_points`
[src/ui/shell.rs:8428](../../../src/ui/shell.rs#L8428)

```rust
fn pen_to_points(p: &PenPacket, (ox, oy): (f32, f32), ppp: f32) -> egui::Pos2
```

**What it does.** Converts a tablet packet's position into an egui screen position. Packets are in
*virtual-desktop physical pixels* (the whole multi-monitor desktop); subtracting the window's
client-area origin `(ox, oy)` gives window pixels, and dividing by `ppp` (pixels per point, the
display scaling factor) gives egui points. The `(ox, oy)` in the parameter list unpacks a tuple
argument directly.

**Used by:** [`pen_agrees_with_cursor`](#pen_agrees_with_cursor),
[`pen_stroke_start`](#pen_stroke_start),
[`pen_stroke_points`](#pen_stroke_points).

### `pen_agrees_with_cursor`
[src/ui/shell.rs:8434](../../../src/ui/shell.rs#L8434)

```rust
fn pen_agrees_with_cursor(
    state: &mut AppState,
    origin: (f32, f32),
    ppp: f32,
    cursor: egui::Pos2,
) -> bool
```

**What it does.** Decides whether to believe the tablet: converts the pen's recent trail of packets
(`state.pen.trail()`) to screen points and checks that the path passes within
[`PEN_MOUSE_AGREEMENT`](#const-pen_mouse_agreement) of the OS cursor. If not, it adds one to
`state.pen_batches_rejected` (shown in the tablet diagnostics) and logs a warning — only once per
stroke, using the `state.pen_outlier_logged` flag.

**Parameters.**
- `origin` — the window client area's top-left in desktop pixels.
- `ppp` — pixels per point.
- `cursor` — where egui says the pointer is.

**Returns.** `true` if the pen can be trusted.

**Used by:** [`pen_stroke_start`](#pen_stroke_start) and
[`pen_stroke_points`](#pen_stroke_points).

### `pen_stroke_start`
[src/ui/shell.rs:8463](../../../src/ui/shell.rs#L8463)

```rust
fn pen_stroke_start(
    state: &mut AppState,
    ppp: f32,
    cursor: egui::Pos2,
) -> Option<(egui::Pos2, PenPacket)>
```

**What it does.** On pen-down, decides where a stroke should start if the pen is to draw it: the
newest tablet packet (from this frame or an earlier one), in screen points. The newest packet is
used rather than the cursor because the cursor is the same position rounded to a whole pixel, and
this frame's older packets lie at or behind it.

**Returns.** `Some((position, packet))` to make it a pen stroke, or `None` to make it a cursor
stroke — when no pen is active, there is no window origin or packet yet, or the pen disagrees with
the cursor. `bool::then_some(x)` turns `true` into `Some(x)` and `false` into `None`.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) (sets `state.stroke_from_pen` from the result).

### `path_passes_near`
[src/ui/shell.rs:8478](../../../src/ui/shell.rs#L8478)

```rust
fn path_passes_near(path: &[egui::Pos2], at: egui::Pos2, reach: f32) -> bool
```

**What it does.** Returns whether point `at` is within `reach` of the polyline (connected line
segments) through `path`. An empty path is never near; a single point is compared directly;
otherwise each pair of neighbouring points (`path.windows(2)`) is a segment, and it checks the
distance from `at` to the closest point on each segment.

**Example.**
```js
function pathPassesNear(path, at, reach) {
  if (path.length === 0) return false;
  if (path.length === 1) return dist(path[0], at) <= reach;
  for (let i = 0; i + 1 < path.length; i++) {
    const a = path[i], b = path[i + 1];
    const ab = sub(b, a);
    const t = clamp(dot(sub(at, a), ab) / Math.max(lenSq(ab), 1e-6), 0, 1);
    if (dist(add(a, mul(ab, t)), at) <= reach) return true; // closest point on segment
  }
  return false;
}
```
The `match path { [] => …, [p] => …, _ => … }` matches on the *shape* of the slice: empty, exactly
one element, or anything else ([Enums and match](../../rust-primer.md#enums-and-match)).

**Used by:** [`pen_agrees_with_cursor`](#pen_agrees_with_cursor) and tests.

### `pen_stroke_points`
[src/ui/shell.rs:8492](../../../src/ui/shell.rs#L8492)

```rust
fn pen_stroke_points(state: &mut AppState, ppp: f32, pointer: Option<egui::Pos2>) -> PenFrame
```

**What it does.** For a stroke the pen is already drawing, gathers this frame's tablet packets as
screen points.

**Parameters.**
- `ppp` — pixels per point.
- `pointer` — egui's pointer position, if known.

**Returns.** A [`PenFrame`](#enum-penframe): `Untrusted` if no pen or no window origin, or if the
path disagrees with the cursor; `Empty` if no new packets; otherwise `Points`.

**How it works.**
1. Bail out with `Untrusted` / `Empty` as above.
2. Check agreement with the cursor *every frame* (an earlier version decided once at pen-down, which
   let a mapping that only looked right at first contact stay trusted for the whole stroke).
3. Convert every packet to screen points and drop any farther than
   [`MAX_PACKET_JUMP`](#const-max_packet_jump) from the newest one. Dropped packets are counted in
   `state.pen_packets_dropped` and logged once per stroke.
4. Return `Points` (never empty, since the newest packet is always within reach of itself).

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) while dragging a pen stroke.

### `stroke_frame_samples`
[src/ui/shell.rs:8549](../../../src/ui/shell.rs#L8549)

```rust
fn stroke_frame_samples(
    from_pen: &mut bool,
    pen: PenFrame,
    cursor: Option<egui::Pos2>,
) -> Vec<(egui::Pos2, Option<PenPacket>)>
```

**What it does.** Chooses this frame's stroke points from *one* source. A pen stroke uses only
packets: `Points` gives them all (each with `Some(packet)`), and `Empty` gives nothing — it does
**not** fall back to the cursor, because the cursor is the pen's position rounded to a whole pixel,
and mixing it in made the line zigzag half a pixel either side (several canvas pixels when zoomed
out). Only `Untrusted` switches the stroke over to the cursor, and it sets `*from_pen = false` so
the switch happens once, for good. A cursor stroke gives the cursor position with `None` for the
packet (or nothing when there is no cursor).

**Parameters.**
- `from_pen` — whether this stroke is drawn by the pen; may be turned off here (`&mut bool` lets
  the function change the caller's variable).
- `pen` — this frame's tablet result.
- `cursor` — egui's pointer position.

**Returns.** A list of `(screen position, optional packet)` pairs.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) and tests.

### `stroke_sample`
[src/ui/shell.rs:8568](../../../src/ui/shell.rs#L8568)

```rust
fn stroke_sample(
    state: &mut AppState,
    x: f32,
    y: f32,
    t: f32,
    packet: Option<PenPacket>,
) -> crate::input::pointer::PointerSample
```

**What it does.** Makes one stroke sample (a `PointerSample`, see [input/pointer.rs](../input/pointer.md))
at cell position `(x, y)` and time `t`. With a packet it uses `state.make_pen_sample` (real
pressure, tilt and so on) and counts it in `state.stroke_pen_samples`; without one it uses
`state.make_sample` and counts it in `state.stroke_mouse_samples`. The counts feed the tablet
readout.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) (on press and on every drag point).

### `menu_window`
[src/ui/shell.rs:8590](../../../src/ui/shell.rs#L8590)

```rust
fn menu_window(state: &mut AppState, ctx: &egui::Context)
```

**What it does.** Draws the small menu strip pinned to the top-left corner (inset by the panel
padding): the menus from [`title_menu`](#title_menu), a button that shows or hides all
panels (`state.show_panels`), and a drag handle (a six-dot grip icon). Because the OS window has no
title bar, pressing on the handle sends `ViewportCommand::StartDrag` so the OS moves the window, and
double-clicking it toggles maximised. The strip itself is anchored, so dragging it never moves it.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75), a test, and
[ui/sweep_tests.rs](sweep_tests.md).

### `title_menu`
[src/ui/shell.rs:8631](../../../src/ui/shell.rs#L8631)

```rust
fn title_menu(state: &mut AppState, ctx: &egui::Context, ui: &mut egui::Ui)
```

**What it does.** Draws the three drop-down menus in the menu strip. Each item calls an `AppState`
method or an I/O helper, then closes the menu.

- **Edit:** Undo / Redo (greyed out when there is nothing to undo/redo, with the bound shortcut in
  the label); a **Flip / rotate** submenu listing the artwork orientation actions from
  [`ORIENT_ART`](../../../src/ui/shell.rs#L6242) (the hover text says what they act on, from
  [`orient_scope`](../../../src/ui/shell.rs#L6259)); and **Settings…**, shown in the warning colour
  when keyboard shortcuts clash. If shortcuts clash, a small warning dot is also painted on the
  Edit menu's corner, because a clash is easy to make and hard to notice.
- **File:** New project…; Open project… (via `project_file::load_dialog`, errors are logged);
  Save — the label names the file that will be overwritten (cut in the middle by
  [`ellipsize_middle`](#ellipsize_middle) if long, full path on hover), since Save
  no longer asks; Save project as…; the Krita items (Edit/Send to Krita, labelled by
  [`krita_menu_label`](#krita_menu_label); Stop Krita link when linked; Install /
  Update / Reinstall Krita helper; Import Krita brushes…); Save PNG of the current frame; Copy
  frame to clipboard; the five exports (PNG sequence, GIF, WebP, MP4, sprite sheet — all through
  [`open_export`](#open_export)); Export .kra…; the imports (PNG sequence, image,
  paste image as background, video, GIF, WebP, .kra); Clear current cell; and Quit (asks the OS
  window to close).
- **Panels:** a checkbox per panel (from `PanelId::ALL` and
  [`panel_meta`](../../../src/ui/shell.rs#L744)), a **Color scheme** submenu (radio buttons over
  `Scheme::ALL`, see [ui/theme.rs](theme.md)), and Reset panel layout.

`ui.menu_button(label, |ui| { … })` draws a button that opens a menu; the closure draws the menu's
contents only while it is open.

**Used by:** [`menu_window`](#menu_window).

### `orientation_toggle`
[src/ui/shell.rs:8976](../../../src/ui/shell.rs#L8976)

```rust
fn orientation_toggle(ui: &mut egui::Ui, w: &mut u32, h: &mut u32) -> bool
```

**What it does.** Draws a Landscape / Portrait pair of toggle labels over a size. Picking the
other orientation swaps `w` and `h` (`std::mem::swap`). The orientation is never stored — it is read
from the numbers (`w >= h` is landscape, so a square counts as landscape), so the toggle can never
disagree with a size typed by hand.

**Parameters.**
- `w`, `h` — the width and height to show and possibly swap (`&mut` so the function can change the
  caller's values).

**Returns.** `true` if it swapped them this frame.

**Used by:** [`camera_size_ui`](../../../src/ui/shell.rs#L3291) and
[`new_project_dialog`](#new_project_dialog).

### `fit_side`
[src/ui/shell.rs:8998](../../../src/ui/shell.rs#L8998)

```rust
fn fit_side(w: u32, h: u32, max: u32) -> (u32, u32)
```

**What it does.** Scales `w`×`h` down, keeping the aspect ratio, until neither side is bigger than
`max`. Sizes that already fit come back unchanged; scaled sides are rounded and kept between 1 and
`max`.

**Example.** `fit_side(20000, 10000, 16384)` → `(16384, 8192)`.

**Used by:** [`new_project_dialog`](#new_project_dialog) (the Clipboard size button)
and tests.

### `new_project_dialog`
[src/ui/shell.rs:9007](../../../src/ui/shell.rs#L9007)

```rust
fn new_project_dialog(state: &mut AppState, ctx: &egui::Context)
```

**What it does.** Draws the **New project** window while `state.show_new_project` is true. The
settings it edits live in `state.new_project_cfg`.

Contents, top to bottom:
1. Width and height number fields (1–16384), the Landscape/Portrait toggle, and FPS (1–120).
2. **Presets**: a button per [`SIZE_PRESETS`](#const-size_presets) entry (flipped to portrait if
   portrait is selected, so picking one doesn't undo that choice), plus a **Clipboard W×H** button
   that copies the size of the image on the system clipboard (fitted to
   [`MAX_SIDE`](#const-max_side); not flipped, since the point is to match it). It is disabled when
   the clipboard has no image.
3. **My presets**: the user's saved presets in a scrolling list (capped at
   [`PRESET_LIST_MAX_H`](#const-preset_list_max_h)). Each row has a star (the default preset New
   project opens with — click to set or clear), the name with size and FPS (highlighted when it
   matches the fields; click applies it, double-click renames it inline, Enter or clicking away
   commits, Escape cancels), and a delete button.
4. A preset-name field with **Save preset** (or **Replace preset** if that name exists), enabled
   only when a name is typed; pressing Enter in the field also saves.
5. **Create** and **Cancel**.

**How it works.**
1. Return if the dialog is closed. Poll the clipboard watcher and ask egui to redraw again after
   `CLIPBOARD_POLL` (250 ms) — a copy in another app is not an input event, so nothing else would
   wake the UI to update the Clipboard button.
2. Draw the window, recording clicks in local flags (`apply`, `star`, `delete`, `start_rename`,
   `rename_commit`, `rename_cancel`, `save`, `create`, `cancel`).
3. After the window: apply the rename (a blank or clashing name keeps the old one), then at most
   one of delete / star / apply, then save, then on Create call `state.reset_with(width, height,
   fps)` and close; Cancel or the window's close button just closes.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75), tests, and [ui/sweep_tests.rs](sweep_tests.md).

### `open_export`
[src/ui/shell.rs:9257](../../../src/ui/shell.rs#L9257)

```rust
fn open_export(state: &mut AppState, kind: ExportKind)
```

**What it does.** Opens the shared export dialog for one format (`kind`: PNG sequence, GIF, WebP,
MP4 or sprite sheet). It keeps the remembered frame range but pulls it back inside the timeline
(start no later than the last frame, end between start and the last frame). If the end is still
`0` — the first time on this project — the range becomes the whole timeline. Then it sets
`state.show_export = true`.

**Used by:** [`title_menu`](#title_menu) (the five Export items) and a test.

### `frame_range_ui`
[src/ui/shell.rs:9271](../../../src/ui/shell.rs#L9271)

```rust
fn frame_range_ui(ui: &mut egui::Ui, start: &mut usize, end: &mut usize, last: usize)
```

**What it does.** A start/end frame picker (both ends included): a two-knob range slider
(`range_slider` from [ui/widgets.rs](widgets.md)) plus exact Start and End number fields, each
limited to `0..=last`. If start ends up after end, end is moved up to start. It is shared by the
export dialog and the import range dialog so the two can't drift apart.

**Used by:** [`export_dialog`](#export_dialog) and
[`import_range_dialog`](#import_range_dialog).

### `export_dialog`
[src/ui/shell.rs:9288](../../../src/ui/shell.rs#L9288)

```rust
fn export_dialog(state: &mut AppState, ctx: &egui::Context)
```

**What it does.** Draws the one export window used by every format while `state.show_export` is
true. The window title comes from `kind.title()`. Settings are kept in `state.export_cfg`.

Contents:
1. A summary line: output size, FPS and frame count.
2. **Frames**: [`frame_range_ui`](#frame_range_ui) and a **Use loop range** button
   that copies the loop bars from the frame strip.
3. **Playback** (all formats except sprite sheet): a Ping-pong checkbox (forward then back) —
   disabled, with a note, when the camera's move is stretched over several loops (set in the Camera
   panel); for MP4 and PNG sequence a **Loop to … s** option (rounded up to whole loops so it
   repeats seamlessly; GIF and WebP repeat on their own); for WebP a **Stop after N plays** option
   (off means it repeats forever).
4. Format-specific settings: for **MP4** a quality slider (CRF 0–51; lower is better and bigger,
   18 is "visually lossless") and an encoder speed preset from `MP4_PRESETS`; for a **sprite
   sheet** the number of columns (0 = automatic, near-square) and the padding between cells.
5. A size/length summary *before* exporting: for a sprite sheet the grid and final pixel size
   (from [io/sprite_sheet.rs](../io/sprite_sheet.md)); for a PNG sequence the file names it will
   write (numbered by position if any frame repeats, matching `png_seq::export_to`); otherwise the
   frames, loops and seconds (from `state.export_info(kind)`).
6. **Export** and **Cancel**.

After the window, Export closes the dialog and calls `state.start_export()`; Cancel or closing the
window just hides it.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75), a test, and [ui/sweep_tests.rs](sweep_tests.md).

### `import_range_dialog`
[src/ui/shell.rs:9529](../../../src/ui/shell.rs#L9529)

```rust
fn import_range_dialog(state: &mut AppState, ctx: &egui::Context)
```

**What it does.** The **Import frame range** window for video, GIF and WebP imports, shown while
`state.show_import_range` is true. It shows thumbnails of the start and end frames, the range
picker, and how many frames will be imported. The active layer is untouched; the chosen range is
laid onto a new layer starting at frame 0.

**How it works.**
1. Copy the total, start and end out of `state.import_range` into local variables first, so the
   window closure never borrows `state` (returns if there is no import pending).
2. Ask for preview images of the start and end frames (`state.request_preview`) and take cheap
   copies of their texture handles (they are reference-counted, so `clone` doesn't copy pixels).
3. Draw the window: a note, two [`preview_box`](#preview_box)es,
   [`frame_range_ui`](#frame_range_ui), the count, and Import / Cancel.
4. Write the edited range back (clamped to the last frame), then call
   `state.confirm_import_range()` or `state.cancel_import_range()`.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) and [ui/sweep_tests.rs](sweep_tests.md).

### `preview_box`
[src/ui/shell.rs:9611](../../../src/ui/shell.rs#L9611)

```rust
fn preview_box(ui: &mut egui::Ui, label: &str, idx: usize, tex: &Option<egui::TextureHandle>)
```

**What it does.** One labelled thumbnail ("Start: frame 12") for the import range dialog. With a
texture it draws the image scaled down to at most 170 points wide (never scaled up); without one
(still loading) it shows a spinner in a 170 × 102 box.

**Used by:** [`import_range_dialog`](#import_range_dialog).

### `save_toast`
[src/ui/shell.rs:9635](../../../src/ui/shell.rs#L9635)

```rust
fn save_toast(state: &mut AppState, ctx: &egui::Context)
```

**What it does.** The short "saved" confirmation at the bottom centre. Ctrl+S no longer opens a
dialog, so this is the only sign the save happened. While a save is still running on its worker
thread (`state.save_job` is set) it shows a spinner with "Saving…" instead. (The first line of the
doc comment above this function, "Modal busy overlay…", actually describes
[`busy_overlay`](#busy_overlay).)

**How it works.**
1. If a save job is running, show "Saving…" and return.
2. Otherwise take the message and its deadline from `state.save_toast`; if there is none, return;
   if the deadline has passed, clear it and return.
3. Ask egui to redraw when the time is up (otherwise the toast would linger until the mouse moves).
4. Fade the colours over the last half second (`gamma_multiply` by the fraction of time left).
5. Draw a check icon and the message.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75).

### `ellipsize_middle`
[src/ui/shell.rs:9694](../../../src/ui/shell.rs#L9694)

```rust
fn ellipsize_middle(name: &str, max: usize) -> String
```

**What it does.** Shortens a file name to `max` characters by cutting out the middle of the name
and putting `…` there, so the start and the end still show — including the extension and any
version or shot number just before it. A name that already fits is returned unchanged. An
extension is only kept separate if it is at most 8 bytes long (counting the dot) and the dot isn't
the first character. About two thirds of the kept characters come from the start and one third
from the end. It counts `chars()` (Unicode characters) rather than bytes
([Strings and slices](../../rust-primer.md#strings-and-slices)).

**Example.**
```rust
ellipsize_middle("my_really_long_scene_name_v12.anim", 20)
// keep = 20 - (5 + 1) = 14 → head 10, tail 4 → "my_really_…_v12.anim"
```

**Used by:** [`title_menu`](#title_menu) (the Save item) and tests.

### `krita_menu_label`
[src/ui/shell.rs:9715](../../../src/ui/shell.rs#L9715)

```rust
fn krita_menu_label(state: &AppState, layer: bool) -> &'static str
```

**What it does.** The text for the "Edit in Krita" menu item. Before a live link to Krita exists it
says "Edit in Krita" (or "Edit layer in Krita" when `layer` is true, for the layer menu that sends
one layer); once linked, the same click sends again, so it says "Send to Krita again" /
"Send layer to Krita again". `&'static str` means a fixed piece of text built into the program
([Lifetimes](../../rust-primer.md#lifetimes)).

**Used by:** [`title_menu`](#title_menu) and
[`layers_content`](../../../src/ui/shell.rs#L3893).

### `krita_toast`
[src/ui/shell.rs:9726](../../../src/ui/shell.rs#L9726)

```rust
fn krita_toast(state: &mut AppState, ctx: &egui::Context)
```

**What it does.** The toast for Krita-link messages: changes arriving from a Krita save, a send to
Krita, or a warning about something in a Krita save that couldn't come across. It reads
`(message, warn, deadline)` from `state.krita_toast`, clears it once the deadline passes, fades over
the last half second like [`save_toast`](#save_toast), and uses a warning icon in
amber for warnings or a brush icon in the accent colour otherwise. It sits higher (64 points from
the bottom instead of 28) so it and the save toast can show together, and wraps long text at 520
points.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75).

### `save_error_dialog`
[src/ui/shell.rs:9763](../../../src/ui/shell.rs#L9763)

```rust
fn save_error_dialog(state: &mut AppState, ctx: &egui::Context)
```

**What it does.** A centred "Save failed" window showing the error in `state.save_error`. It stays
up until dismissed, on purpose: a save that failed silently would leave you thinking your work is
on disk when it isn't. **OK** clears the error; **Save As…** clears it and opens Save As so the
work can be saved somewhere else.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75) and [ui/sweep_tests.rs](sweep_tests.md).

### `busy_overlay`
[src/ui/shell.rs:9798](../../../src/ui/shell.rs#L9798)

```rust
fn busy_overlay(ctx: &egui::Context, label: &str)
```

**What it does.** A small popup in the middle of the window with a spinner and `label`, shown while
a background import job runs. It is an `egui::Area` on the foreground layer.

**Used by:** [`draw`](../../../src/ui/shell.rs#L75), when `state.bg_label` is set.

## Tests

### `mod tests`

Run with `cargo test ui::shell::tests` · 9 tests

| Test | What it checks |
|---|---|
| [`xform_round_trips_under_every_flip`](../../../src/ui/shell.rs#L9824) | For all four mirror combinations of a rotated, zoomed, panned view, turning a document point into a screen point and back gives the same point, so the hand-written inverse `screen_to_doc` never disagrees with `doc_to_screen`. |
| [`screen_delta_matches_position_mapping_under_flip`](../../../src/ui/shell.rs#L9849) | A mouse drag distance converted with `screen_delta_to_doc` equals the difference of two converted positions under every flip, so dragging a layer in a mirrored view doesn't move it backwards. |
| [`right_hand_column_tracks_the_right_edge`](../../../src/ui/shell.rs#L9881) | A panel 24 px from the right edge of a 1280 window is still 24 px from the right edge when the window grows to 2560 (the gap is kept, not the x). |
| [`left_hand_column_stays_put`](../../../src/ui/shell.rs#L9888) | A panel near the left edge keeps its exact x when the window grows. |
| [`centred_panel_recentres_instead_of_drifting`](../../../src/ui/shell.rs#L9894) | A panel with equal gaps on both sides (the Timeline) is re-centred in the bigger window instead of being pinned to one side. |
| [`bottom_anchored_panel_tracks_the_bottom`](../../../src/ui/shell.rs#L9901) | Vertically, a panel 20 px above the bottom stays 20 px above the bottom of a taller window. |
| [`shrinking_keeps_the_panel_on_screen`](../../../src/ui/shell.rs#L9909) | When the window shrinks, a right-anchored panel moves in to keep its gap rather than hanging off the edge. |
| [`panel_wider_than_the_window_clamps_to_the_left`](../../../src/ui/shell.rs#L9916) | If the window becomes narrower than the panel, the panel is placed at x = 0 instead of getting a negative position. |
| [`no_resize_is_a_no_op`](../../../src/ui/shell.rs#L9922) | When the window size didn't change, no panel moves. |

**Test helpers** (used only by the tests above):

- [`restick`](../../../src/ui/shell.rs#L9876) — shorthand that calls `restick_axis` for a panel at `lo` with length `len`, going from the `old` window span to the `new` one.

**Test-only types and constants:** [`SMALL`](../../../src/ui/shell.rs#L9873), [`WIDE`](../../../src/ui/shell.rs#L9874)

### `mod perspective_tests`

Run with `cargo test ui::shell::perspective_tests` · 8 tests

| Test | What it checks |
|---|---|
| [`a_screen_lock_is_level_on_screen`](../../../src/ui/shell.rs#L9963) | With "lock to screen axes" on, the Shift-lock's horizontal direction is level on screen under every view rotation and mirror, and also when looking through a rolled camera; with it off the lock uses the canvas's own axes (angle 0). |
| [`dragging_a_corner_on_the_canvas_reshapes_the_grid`](../../../src/ui/shell.rs#L9990) | Pressing on a grid corner and dragging it 40 px moves just that corner to where the pointer let go, leaves the other corners alone, and paints nothing (no new cell, no stroke). |
| [`grids_paint_with_every_option_and_tool`](../../../src/ui/shell.rs#L10025) | A smoke test: two grids with extra vanishing points (one off the horizon, one in the middle of a grid) draw without panicking, whichever grid is active and whether the Grid tool or the Pencil is selected. |
| [`a_grids_gear_opens_its_settings_which_follow_the_active_grid`](../../../src/ui/shell.rs#L10089) | The Tools panel lists the grids with one gear icon per row but not their settings; clicking the second gear opens the grid-settings window for that grid and makes it active; switching the active grid changes what the window shows; deleting all grids closes it. |
| [`every_grid_kind_line_option_and_guide_paints`](../../../src/ui/shell.rs#L10163) | A smoke test over every grid kind (perspective, flat, isometric following the camera, a full-canvas room), walls, infinite/diagonal/centre lines, keyed grids with ghosts either side of the frame and snapping: all paint under both tools, and the two walls are still there. |
| [`a_grid_that_fills_the_canvas_is_only_its_lines_out_of_the_grid_tool`](../../../src/ui/shell.rs#L10232) | A "fill canvas" one-point grid shows its grabbable four-corner outline only in the Grid tool; under the Pencil it paints just lines: level floor and ceiling lines, plus an even fan of rays every 7.5° that stop short of the vanishing point. |
| [`an_extra_vanishing_point_drags_along_the_horizon_on_the_canvas`](../../../src/ui/shell.rs#L10283) | Dragging an extra vanishing point sideways (and a bit off) slides it along the horizon to the pointer's x while it stays exactly on the horizon, and the grid itself doesn't move. |
| [`a_rect_dragged_on_the_canvas_lies_on_the_grid`](../../../src/ui/shell.rs#L10322) | With snapping on, a Rectangle shape dragged over a one-point floor grid previews on the grid's plane mid-drag and paints a trapezoid that follows the leaning column (ink near the far edge, bare at the flat rectangle's corner). |

**Test helpers** (used only by the tests above):

- [`frame`](../../../src/ui/shell.rs#L9938) — one frame with held modifiers that first lets `state.read_input` read the input (as the app's update does), then draws.
- [`state`](../../../src/ui/shell.rs#L9947) — a test state with panels hidden, 12 frames, playhead on frame 0.
- [`texts`](../../../src/ui/shell.rs#L10064) — every painted text with its rectangle.
- [`find`](../../../src/ui/shell.rs#L10068) — Find.
- [`grid_shapes`](../../../src/ui/shell.rs#L10218) — runs a frame that paints only the perspective grids (via `draw_perspective_grids` on a background painter) and returns the shapes.

**Test-only types and constants:** [`SCREEN`](../../../src/ui/shell.rs#L9936)

### `mod onion_tests`

Run with `cargo test ui::shell::onion_tests` · 7 tests

| Test | What it checks |
|---|---|
| [`a_pin_gets_a_ghost_in_its_own_colour_and_everything_paints`](../../../src/ui/shell.rs#L10371) | A visible onion pin gets a ghost in the pin's own colour (not the "previous frames" tint), an ordinary lit offset gets the normal tint, and an offset that is hidden with its pin switched off gets no ghost; the onion panel also draws. |
| [`the_current_drawing_paints_over_every_ghost`](../../../src/ui/shell.rs#L10416) | Past, future and pinned ghosts are all painted before the current drawing, so no ghost covers the lines being drawn; a pin uses its own opacity (0.4 → alpha 102), not the bars'. |
| [`the_strip_fits_the_default_panel`](../../../src/ui/shell.rs#L10563) | The onion strip fits in the default 232 px panel width and is exactly numbers + gap + bars tall. |
| [`numbers_light_offsets_and_zero_switches_the_skin`](../../../src/ui/shell.rs#L10570) | Clicking the 0 number turns the onion skin on and off; clicking another number toggles only that offset (−5 lights, +1 goes dark) and leaves the others alone. |
| [`alt_click_shows_one_offset_and_again_brings_the_rest_back`](../../../src/ui/shell.rs#L10586) | Alt-clicking a number shows that offset alone ("solo"), and Alt-clicking it again restores exactly the offsets that were shown before. |
| [`a_press_sets_a_bar_and_a_sweep_sets_every_bar_it_crosses`](../../../src/ui/shell.rs#L10596) | Clicking a bar sets that offset's opacity to the click height; one fast drag from +1 (high) to +8 (low) sets every bar in between along a straight falling line with none skipped, and doesn't touch the past side. |
| [`a_layer_with_its_onion_on_ghosts_under_its_own_drawing`](../../../src/ui/shell.rs#L10616) | A non-active layer whose own onion switch is on gets ghosts painted just under its own drawing (and below the active layer's ghosts), while a layer with its onion off gets none. |

**Test helpers** (used only by the tests above):

- [`strip_frame`](../../../src/ui/shell.rs#L10473) — runs one frame with only the onion strip inside a 232 px wide area, with Alt held or not, and returns the strip's rectangle.
- [`new`](../../../src/ui/shell.rs#L10505) — makes the context and a default onion config and lays the strip out once.
- [`col_x`](../../../src/ui/shell.rs#L10512) — the x at the centre of column `i` (17 columns: −8 … +8).
- [`num`](../../../src/ui/shell.rs#L10518) — the screen point of the number in column `i`.
- [`bar`](../../../src/ui/shell.rs#L10523) — the screen point at height `v` (0–1) of column `i`'s bar.
- [`frame`](../../../src/ui/shell.rs#L10527) — one frame with held modifiers that first lets `state.read_input` read the input (as the app's update does), then draws.
- [`button`](../../../src/ui/shell.rs#L10531) — builds a primary-button press or release event with given modifiers.
- [`stroke`](../../../src/ui/shell.rs#L10542) — presses at the first point, moves through the rest (one frame each), releases at the last, then runs a settle frame.
- [`click`](../../../src/ui/shell.rs#L10552) — a press and its release, one frame each.
- [`col`](../../../src/ui/shell.rs#L10558) — turns an offset (−8 … +8) into its column index (0 … 16).

**Test-only types and constants:** [`Strip`](../../../src/ui/shell.rs#L10498)

### `mod fade_tests`

Run with `cargo test ui::shell::fade_tests` · 1 tests

| Test | What it checks |
|---|---|
| [`other_layers_paint_at_their_fade_and_the_active_one_at_full`](../../../src/ui/shell.rs#L10693) | With fading on (above 0.25, below 0.5), the active layer paints at full alpha 255, the layer above at 25 % and the layer below at 50 %. |

### `mod panel_drag_tests`

Run with `cargo test ui::shell::panel_drag_tests` · 6 tests

| Test | What it checks |
|---|---|
| [`the_onion_bars_grow_with_the_panel_and_pins_squeeze_them`](../../../src/ui/shell.rs#L10790) | Dragging the Onion panel's bottom edge down 80 px makes it 80 px taller and the bars take all of that height (the Reset button below them moves by the same amount); the panel doesn't creep at rest, and adding two pins keeps the panel's height while the bars shrink to make room. |
| [`dragging_the_body_leaves_the_panel_where_it_is`](../../../src/ui/shell.rs#L10870) | A drag that starts on the panel's bottom margin or on empty space just above the onion numbers does not move the panel. |
| [`dragging_the_title_bar_moves_the_panel`](../../../src/ui/shell.rs#L10885) | A 40 px diagonal drag on the title bar moves the panel by exactly (40, 40). |
| [`controls_under_the_catcher_still_work`](../../../src/ui/shell.rs#L10891) | A click on the onion strip inside the panel body still changes the onion settings, so the drag catcher doesn't block the panel's own controls. |
| [`the_edges_still_resize`](../../../src/ui/shell.rs#L10911) | Dragging the left edge outwards widens the panel to the left while its right edge stays put. |
| [`dragging_the_menu_strip_leaves_it_where_it_is`](../../../src/ui/shell.rs#L10920) | The top menu strip starts one padding in from the top-left corner and stays there when dragged from its margin or from the gap between its menus. |

**Test helpers** (used only by the tests above):

- [`frame`](../../../src/ui/shell.rs#L10755) — one frame with held modifiers that first lets `state.read_input` read the input (as the app's update does), then draws.
- [`frame_shapes`](../../../src/ui/shell.rs#L10759) — runs one frame on a 1600 × 900 screen that draws only the Onion panel (via `panel_window` at (300, 200), 252 px wide) and returns the painted shapes.
- [`reset_y`](../../../src/ui/shell.rs#L10777) — the y of the Onion panel's "Reset" text, which sits right under the bars and so shows how tall the bars are.
- [`panel_rect`](../../../src/ui/shell.rs#L10837) — reads the Onion panel's current rectangle from egui's memory.
- [`drag`](../../../src/ui/shell.rs#L10844) — press at `from`, move by `by` over 10 frames, release, settle a frame.

### `mod panel_padding_tests`

Run with `cargo test ui::shell::panel_padding_tests` · 9 tests

| Test | What it checks |
|---|---|
| [`a_panel_near_another_settles_a_padding_from_it`](../../../src/ui/shell.rs#L10985) | A panel let go near another panel (below it or beside it, even overlapping a little) snaps to one padding away from it, but one dropped right on top of it or well clear is left alone; the window edge beats a neighbour, and a panel caught between two neighbours hangs from the one above. |
| [`a_panel_near_an_edge_settles_at_the_padding`](../../../src/ui/shell.rs#L11011) | A panel within reach of the right, bottom or top edge snaps to exactly the padding from it (clear of the top bar), while one out in the open, already in place, or too far away gets no new position. |
| [`under_the_top_bar_the_bar_is_the_edge`](../../../src/ui/shell.rs#L11021) | Near the top-left, the bottom of the top bar acts as the edge: a panel just under it, or overlapping it, snaps to one padding below the bar. |
| [`a_new_padding_moves_the_panels_it_held`](../../../src/ui/shell.rs#L11029) | When the padding changes from 8 to 40, panels that were held at the old padding (left, right, and under the moved bar) move to the new one however far, and a free panel stays where it is. |
| [`the_first_layout_keeps_the_padding`](../../../src/ui/shell.rs#L11068) | On first open, the top bar sits 8 px from the corner, the Tools panel 8 px under the bar, and the Layers column 8 px from the top-right corner. |
| [`a_panel_let_go_near_an_edge_settles_at_the_padding_and_no_nearer`](../../../src/ui/shell.rs#L11102) | While held near the right edge the Layers panel follows the pointer, but once let go it snaps to 8 px from the edge; dragged past the edge it still ends 8 px in; dropped in the open it stays exactly where it was let go. |
| [`changing_the_padding_moves_the_bar_and_the_panels_it_held`](../../../src/ui/shell.rs#L11120) | Setting the padding to 30 moves the top bar, the Tools panel, the right column and the Brush panel stacked under Tools to their 30 px positions within a few frames. |
| [`the_first_layout_stacks_the_panels_a_padding_apart`](../../../src/ui/shell.rs#L11141) | On first open, panels stacked in a column (Tools/Brush, Layers/Onion/Camera/X-sheet) are exactly one padding apart and no two panels overlap. |
| [`a_panel_let_go_under_another_settles_a_padding_below_it`](../../../src/ui/shell.rs#L11164) | Dragging the Brush panel to 20 px under Tools and letting go snaps it to 8 px under Tools. |

**Test helpers** (used only by the tests above):

- [`edges`](../../../src/ui/shell.rs#L10970) — builds a `PanelEdges` (the test screen, padding 8, the test top bar, and an optional old padding `was`).
- [`panel`](../../../src/ui/shell.rs#L10974) — reads the Timeline panel's rectangle from egui's memory.
- [`padded_pos_alone`](../../../src/ui/shell.rs#L10978) — calls `padded_pos` with no other panels around.
- [`run`](../../../src/ui/shell.rs#L11037) — runs one frame on the 1400 × 900 screen that calls the given `show` function, and returns the shapes.
- [`rect`](../../../src/ui/shell.rs#L11046) — reads an area's rectangle (by egui `Id`) from egui's memory.
- [`setup`](../../../src/ui/shell.rs#L11050) — a test state with a 1920 × 1080 project loaded and a themed context.
- [`layers`](../../../src/ui/shell.rs#L11061) — the egui `Id` of the Layers panel's area.
- [`drag_to`](../../../src/ui/shell.rs#L11080) — drags a panel by its title bar so its top-left heads for `to`, and returns its rect while still held and after letting go.

**Test-only types and constants:** [`SCREEN`](../../../src/ui/shell.rs#L10967), [`BAR`](../../../src/ui/shell.rs#L10968)

### `mod size_strip_tests`

Run with `cargo test ui::shell::size_strip_tests` · 6 tests

| Test | What it checks |
|---|---|
| [`the_strip_offers_sizes_drawn_at_pins_and_removes_them`](../../../src/ui/shell.rs#L11222) | Sizes drawn at show under the slider: a click draws at one, a right-click pins it, and a pinned size shows once, with its pin. |
| [`the_screen_size_lock_sits_at_the_end_of_the_size_row`](../../../src/ui/shell.rs#L11267) | The lock to screen size is an icon at the end of the Size row, not a line of its own, and clicking it turns the lock on and off. |
| [`the_size_slider_marks_pinned_and_recent_sizes_where_the_handle_sits`](../../../src/ui/shell.rs#L11329) | The slider draws one accent tick for each pinned size and one faint tick for each recent (unpinned) size, each exactly where the handle sits at that size; the ticks don't move when the size changes, and they're painted after (on top of) the slider handle. |
| [`long_values_leave_the_tools_panel_its_width`](../../../src/ui/shell.rs#L11371) | Long decimal values such as radius 16.89237 or opacity 0.92254 don't make the Tools panel grow, even over 20 frames (guards against a slider feeding its own overflow back and widening the panel every frame). |
| [`the_tools_panel_goes_no_wider_than_its_cap`](../../../src/ui/shell.rs#L11394) | Dragging the Tools panel's right edge 1200 px out widens it, but never past `TOOLS_MAX_W` plus its margins. |
| [`the_strip_keeps_to_one_row`](../../../src/ui/shell.rs#L11426) | With five pinned and five recent sizes, all pinned sizes show and every size that shows is on the same single row (recent ones only as many as fit). |

**Test helpers** (used only by the tests above):

- [`frame`](../../../src/ui/shell.rs#L11179) — one frame with held modifiers that first lets `state.read_input` read the input (as the app's update does), then draws.
- [`text_pos`](../../../src/ui/shell.rs#L11192) — the centre of the painted text exactly equal to `text`, if any.
- [`find`](../../../src/ui/shell.rs#L11193) — Find.
- [`click`](../../../src/ui/shell.rs#L11207) — a press and its release, one frame each.
- [`all`](../../../src/ui/shell.rs#L11284) — flattens every painted shape (unpacking nested `Shape::Vec` groups) into one list.
- [`walk`](../../../src/ui/shell.rs#L11285) — Walk.
- [`marks`](../../../src/ui/shell.rs#L11299) — finds the short vertical line segments drawn in a given colour (the slider's size ticks) and returns each one's x and middle y.
- [`handle_x`](../../../src/ui/shell.rs#L11316) — the x of the Size slider's round handle on the row at height `y`.

### `mod stroke_source_tests`

Run with `cargo test ui::shell::stroke_source_tests` · 6 tests

| Test | What it checks |
|---|---|
| [`a_pen_frame_without_packets_adds_nothing`](../../../src/ui/shell.rs#L11473) | The "wobble" regression: on a frame where the pen sent no packets, a pen stroke adds no point at all (it doesn't fall back to the pixel-rounded cursor) and stays a pen stroke. |
| [`a_pen_frame_takes_every_packet_and_only_packets`](../../../src/ui/shell.rs#L11481) | A pen frame with two packets adds exactly those two points, at their sub-pixel positions with pressure data, and ignores the cursor. |
| [`an_untrusted_pen_hands_over_to_the_cursor_for_good`](../../../src/ui/shell.rs#L11496) | When the pen's screen mapping stops agreeing with the cursor, the stroke switches to the cursor once and stays on it, even if good packets arrive later in the same stroke. |
| [`a_cursor_stroke_ignores_packets`](../../../src/ui/shell.rs#L11511) | A stroke that is already drawing from the cursor uses the cursor position and ignores any pen packets. |
| [`a_cursor_trailing_a_fast_pen_still_agrees`](../../../src/ui/shell.rs#L11535) | The fast-flick regression: a cursor lagging 12 ms behind the newest packet (far beyond the tolerance in distance) still lies on the pen's path, even rounded to whole pixels, so the pen keeps the stroke. |
| [`a_path_away_from_the_cursor_disagrees`](../../../src/ui/shell.rs#L11551) | A pen path that is Y-flipped (a buggy driver), or offset sideways by 1.5× the tolerance, or empty, is judged not to agree with the cursor; a single packet 3 px away does agree. |

**Test helpers** (used only by the tests above):

- [`packet`](../../../src/ui/shell.rs#L11457) — a `PenPacket` at (x, y) with half pressure and no tilt or rotation.
- [`flick`](../../../src/ui/shell.rs#L11522) — 32 points 50 px apart along a gentle curve, standing in for a fast pen flick.

### `mod stroke_latency_tests`

Run with `cargo test ui::shell::stroke_latency_tests` · 2 tests

| Test | What it checks |
|---|---|
| [`a_stroke_s_ink_goes_up_on_the_frame_it_is_drawn`](../../../src/ui/shell.rs#L11626) | Every brush engine: each move frame sends its own ink, and leaves nothing for the next frame to send. The regression this pins: the texture sync ran before the frame's samples were painted, so every stroke showed a frame behind the pen. |
| [`the_live_tail_draws_in_its_slot`](../../../src/ui/shell.rs#L11658) | The live tail goes where `paint_canvas` kept its place — under the overlays painted after it, not on top of everything. |

**Test helpers** (used only by the tests above):

- [`frame`](../../../src/ui/shell.rs#L11573) — one frame with held modifiers that first lets `state.read_input` read the input (as the app's update does), then draws.
- [`run`](../../../src/ui/shell.rs#L11579) — runs one frame on the 1400 × 900 screen that calls the given `show` function, and returns the shapes.
- [`button`](../../../src/ui/shell.rs#L11593) — builds a primary-button press or release event with given modifiers.
- [`stroke`](../../../src/ui/shell.rs#L11604) — presses at the first point, moves through the rest (one frame each), releases at the last, then runs a settle frame.

**Test-only types and constants:** [`SCREEN`](../../../src/ui/shell.rs#L11571)

### `mod new_project_tests`

Run with `cargo test ui::shell::new_project_tests` · 4 tests

| Test | What it checks |
|---|---|
| [`dialog_draws_saved_presets_and_a_rename_in_progress`](../../../src/ui/shell.rs#L11712) | With three saved presets and one being renamed, the dialog stays open, keeps all three presets, gives the rename box keyboard focus, and starts on the starred (default) preset's size, 1080 × 1920. |
| [`clipboard_button_takes_the_image_size_as_it_is`](../../../src/ui/shell.rs#L11768) | The button shows the clipboard image's size ("Clipboard 1000×2000"), and clicking it sets that exact width and height, without flipping a portrait image to match the current landscape setting. |
| [`clipboard_button_does_nothing_without_an_image`](../../../src/ui/shell.rs#L11784) | With no image on the clipboard, clicking the button leaves the size unchanged. |
| [`an_oversized_clipboard_image_fits_the_side_limit`](../../../src/ui/shell.rs#L11797) | `fit_side` leaves a normal size alone and scales an oversized one down, keeping its shape, so its longest side is `MAX_SIDE` (16384) and no side drops below 1. |

**Test helpers** (used only by the tests above):

- [`preset`](../../../src/ui/shell.rs#L11701) — builds a `ProjectPreset` with a name, size, 24 fps and whether it is the starred default.
- [`draw`](../../../src/ui/shell.rs#L11740) — runs one frame of just the dialog on a 1600 × 1000 screen and returns the centre of the first text containing `find`, if drawn.
- [`click`](../../../src/ui/shell.rs#L11755) — a press and its release, one frame each.

### `mod timeline_panel_tests`

Run with `cargo test ui::shell::timeline_panel_tests` · 3 tests

| Test | What it checks |
|---|---|
| [`dragging_a_side_edge_keeps_the_height`](../../../src/ui/shell.rs#L11860) | With the theme causing a title-bar shortfall, dragging the right edge 60 px or the left edge 40 px moves that edge to follow the pointer and leaves the panel's height unchanged. |
| [`dragging_the_top_or_bottom_edge_moves_just_that_edge`](../../../src/ui/shell.rs#L11878) | Dragging the top edge up 30 px moves only the top (the bottom stays), and dragging the bottom edge up 50 px moves only the bottom (the top stays). |
| [`the_insert_before_button_adds_frames_at_the_start`](../../../src/ui/shell.rs#L11896) | With frame step 2 and two layers, clicking the toolbar's "insert before" button adds two blank frames in front of the first drawing on every layer and moves the playhead with the old first frame (to frame 2). |

**Test helpers** (used only by the tests above):

- [`frame`](../../../src/ui/shell.rs#L11814) — one frame with held modifiers that first lets `state.read_input` read the input (as the app's update does), then draws.
- [`panel`](../../../src/ui/shell.rs#L11823) — reads the Timeline panel's rectangle from egui's memory.
- [`drag`](../../../src/ui/shell.rs#L11830) — press at `from`, move by `by` over 10 frames, release, settle a frame.
- [`setup`](../../../src/ui/shell.rs#L11847) — a test state with a 1920 × 1080 project loaded and a themed context.

**Test-only types and constants:** [`SCREEN`](../../../src/ui/shell.rs#L11812)

### `mod layers_panel_tests`

Run with `cargo test ui::shell::layers_panel_tests` · 12 tests

| Test | What it checks |
|---|---|
| [`clipped_and_alpha_locked_layers_are_marked_in_the_panel`](../../../src/ui/shell.rs#L12009) | The clip and alpha-lock icons appear once each (the toolbar buttons); turning clip on for one layer and alpha lock on another adds one mark for each on those rows. |
| [`ctrl_click_picks_and_a_group_folds`](../../../src/ui/shell.rs#L12066) | Ctrl-clicking Alpha makes it active while Charlie stays picked; grouping the two makes "Group 1" with an open-folder icon; folding the group hides its layers' rows but not Bravo. |
| [`the_list_scrolls_to_a_layer_selected_by_key`](../../../src/ui/shell.rs#L12112) | With 23 layers the list starts scrolled to the top row; selecting the bottom layer by keyboard action scrolls the list (animated, so it runs 60 frames) until Alpha is visible. |
| [`the_mini_bar_names_the_active_layer`](../../../src/ui/shell.rs#L12139) | With panels hidden, the mini timeline bar shows the active layer's name next to a stack icon, and cuts a long name short with "…". |
| [`dragging_a_layer_name_onto_another_row_reorders_in_one_undo`](../../../src/ui/shell.rs#L12168) | Dragging Alpha's name onto Charlie's row moves Alpha to the top, selects it, and a single undo puts the original order back. |
| [`a_click_on_a_name_still_selects_and_a_double_click_renames`](../../../src/ui/shell.rs#L12190) | A plain click on a name selects that layer without reordering anything, and a double-click opens the rename box on that row. |
| [`the_toolbar_clips_every_picked_layer`](../../../src/ui/shell.rs#L12251) | With all layers picked and only some clipped, the toolbar's clip button turns clip on for all of them, and a second click turns it off for all. |
| [`a_faded_layer_shows_its_opacity_on_its_row`](../../../src/ui/shell.rs#L12264) | No percentage shows at full opacity; setting a layer to 40 % opacity shows "40%" on its row. |
| [`a_row_s_opacity_drags_that_layer_alone`](../../../src/ui/shell.rs#L12277) | Dragging the "40%" on Alpha's row to the right raises only Alpha (other picked layers stay at 40 %) up to 100 %, adds no undo step mid-drag but exactly one when let go, the % disappears once at full and released, and undo restores 40 %. |
| [`the_dots_open_the_layer_menu`](../../../src/ui/shell.rs#L12315) | Clicking the ⋮ button on a row opens that layer's menu, and its "Rename" opens the rename box for that layer. |
| [`the_row_s_cleanup_button_adds_a_linked_layer`](../../../src/ui/shell.rs#L12327) | Every row has a "keys with" dropdown; clicking Alpha's cleanup button adds an "Alpha clean" layer whose keys follow Alpha (`keys_from` is Alpha's uid). |
| [`the_folded_sections_still_draw`](../../../src/ui/shell.rs#L12344) | A smoke test: the layer Transform and Canvas sections draw on their own without panicking. |

**Test helpers** (used only by the tests above):

- [`frame`](../../../src/ui/shell.rs#L11949) — one frame with held modifiers that first lets `state.read_input` read the input (as the app's update does), then draws.
- [`setup`](../../../src/ui/shell.rs#L11964) — a test state with a 1920 × 1080 project loaded and a themed context.
- [`panel_count`](../../../src/ui/shell.rs#L11983) — counts the texts painted inside the Layers panel that contain a given icon glyph or string.
- [`find`](../../../src/ui/shell.rs#L11988) — Find.
- [`panel_shows`](../../../src/ui/shell.rs#L12004) — whether `panel_count` is above zero.
- [`names`](../../../src/ui/shell.rs#L12024) — the layer names in stack order (bottom first).
- [`name_rect`](../../../src/ui/shell.rs#L12031) — where a layer's name label (also its drag handle) was painted inside the Layers panel; panics if it wasn't.
- [`find`](../../../src/ui/shell.rs#L12032) — Find.
- [`button`](../../../src/ui/shell.rs#L12054) — builds a primary-button press or release event with given modifiers.
- [`find`](../../../src/ui/shell.rs#L12144) — Find.
- [`text_rects`](../../../src/ui/shell.rs#L12210) — runs a frame and returns every painted text with its rectangle.
- [`find`](../../../src/ui/shell.rs#L12211) — Find.
- [`click`](../../../src/ui/shell.rs#L12232) — a press and its release, one frame each.
- [`on_row`](../../../src/ui/shell.rs#L12239) — the position of a glyph painted on the same line as a given layer's name; panics if there's none.

**Test-only types and constants:** [`SCREEN`](../../../src/ui/shell.rs#L11945), [`NAMES`](../../../src/ui/shell.rs#L11946)

### `mod lasso_options_tests`

Run with `cargo test ui::shell::lasso_options_tests` · 1 tests

| Test | What it checks |
|---|---|
| [`all_frames_is_offered_and_counts_the_drawings`](../../../src/ui/shell.rs#L12388) | With the Lasso active, the panel offers an "All frames" checkbox and shows no count while it's off; once on, it says "Applies to 3 drawings on this layer". |

**Test helpers** (used only by the tests above):

- [`texts`](../../../src/ui/shell.rs#L12368) — every painted text with its rectangle.
- [`walk`](../../../src/ui/shell.rs#L12369) — Walk.

### `mod fill_tests`

Run with `cargo test ui::shell::fill_tests` · 6 tests

| Test | What it checks |
|---|---|
| [`dragging_a_fill_press_sideways_shuts_the_gap`](../../../src/ui/shell.rs#L12450) | Pressing the bucket inside a box with a 4 px hole and dragging sideways raises the gap-closing setting to 4 live, so the fill turns red inside but doesn't leak out; letting go ends the drag with exactly one undo step. |
| [`the_grain_slider_brings_its_options_and_specks_the_bucket`](../../../src/ui/shell.rs#L12511) | The Fill options show "Grain" but hide grain size/contrast/boil while grain is 0; pressing the Grain slider raises grain and shows those options; a bucket fill on the bare canvas is then red everywhere but with uneven alpha (paper specks). |
| [`quick_taps_on_different_spots_are_not_a_double_click`](../../../src/ui/shell.rs#L12591) | In lasso-fill mode, three fast taps on different spots (quicker than egui's double-click time) add three polygon corners and don't close the polygon early. |
| [`a_lasso_drag_fills_what_it_goes_round`](../../../src/ui/shell.rs#L12602) | Dragging a lasso around a square fills inside it red, nothing outside, in one undo step, and clears the lasso afterwards. |
| [`taps_make_a_polygon_that_fills_on_the_first_corner`](../../../src/ui/shell.rs#L12621) | Unhurried taps lay out triangle corners without filling anything yet; tapping the first corner again closes the polygon and fills inside the triangle only. |
| [`a_double_click_closes_a_polygon_fill`](../../../src/ui/shell.rs#L12639) | After two corners, a double-click on a third spot closes the polygon and fills it. |

**Test helpers** (used only by the tests above):

- [`frame`](../../../src/ui/shell.rs#L12425) — one frame with held modifiers that first lets `state.read_input` read the input (as the app's update does), then draws.
- [`button`](../../../src/ui/shell.rs#L12434) — builds a primary-button press or release event with given modifiers.
- [`px`](../../../src/ui/shell.rs#L12443) — reads the RGBA of one pixel of the current frame's cell.
- [`text_rects`](../../../src/ui/shell.rs#L12488) — runs a frame and returns every painted text with its rectangle.
- [`walk`](../../../src/ui/shell.rs#L12489) — Walk.
- [`lasso_state`](../../../src/ui/shell.rs#L12556) — a state with panels hidden and the Fill tool chosen twice (which swaps it to lasso mode, asserted), red, plus a context after one frame.
- [`at`](../../../src/ui/shell.rs#L12569) — converts a document point to its screen position.
- [`click`](../../../src/ui/shell.rs#L12574) — a press and its release, one frame each.
- [`tap`](../../../src/ui/shell.rs#L12581) — a click followed by 60 empty frames, so the next press is too late to count as a double- or triple-click.

**Test-only types and constants:** [`SCREEN`](../../../src/ui/shell.rs#L12422), [`RED`](../../../src/ui/shell.rs#L12423)

### `mod wheel_tests`

Run with `cargo test ui::shell::wheel_tests` · 5 tests

| Test | What it checks |
|---|---|
| [`the_plain_wheel_still_steps_frames`](../../../src/ui/shell.rs#L12731) | Turning the plain wheel down one notch moves one frame forward, two notches moves two. |
| [`shift_wheel_jumps_through_the_linked_drawings`](../../../src/ui/shell.rs#L12744) | On an empty cleanup layer linked to the rough, Shift + wheel jumps between the rough's drawings (0 → 3 → 9), stops at the last one, goes back with wheel up, never changes layer, and does nothing once the link is removed. |
| [`ctrl_wheel_zooms_the_view_about_the_pointer`](../../../src/ui/shell.rs#L12767) | Ctrl + wheel zooms in (by more than 1.3× for two notches) without changing frame, keeping the document point under the pointer fixed on screen. |
| [`a_remapped_slot_does_its_new_job`](../../../src/ui/shell.rs#L12783) | After remapping the plain wheel to "layers" and Alt to "brush size", the plain wheel walks down and up the layers without changing frame, and Alt + wheel up makes the brush bigger. |
| [`over_the_timeline_alt_pans_and_shift_jumps`](../../../src/ui/shell.rs#L12803) | Over the Timeline tracks, Alt + wheel scrolls the tracks sideways without changing frame, and Shift + wheel no longer pans but jumps to the rough's next drawing (only once the layer is linked). |

**Test helpers** (used only by the tests above):

- [`mods`](../../../src/ui/shell.rs#L12664) — builds an `egui::Modifiers` with Ctrl (and Command), Shift and Alt as asked.
- [`frame`](../../../src/ui/shell.rs#L12675) — one frame with held modifiers that first lets `state.read_input` read the input (as the app's update does), then draws.
- [`turn`](../../../src/ui/shell.rs#L12696) — moves the pointer to a point, then sends a `MouseWheel` event of `notches` lines (positive = up) with the modifiers held.
- [`state`](../../../src/ui/shell.rs#L12712) — a test state with panels hidden, 12 frames, playhead on frame 0.

**Test-only types and constants:** [`SCREEN`](../../../src/ui/shell.rs#L12661), [`AT`](../../../src/ui/shell.rs#L12662)

### `mod button_drag_tests`

Run with `cargo test ui::shell::button_drag_tests` · 15 tests

| Test | What it checks |
|---|---|
| [`a_middle_drag_scrubs_by_default_and_draws_nothing`](../../../src/ui/shell.rs#L12883) | By default a 35 px middle-button drag scrubs the playhead (to frame 3) instead of panning, and paints nothing (no new cell, no stroke left over). |
| [`a_right_drag_picks_from_the_pie_by_default`](../../../src/ui/shell.rs#L12897) | By default a right-button drag picks a colour from the palette pie (here the third swatch), draws nothing and adds no undo step. |
| [`a_button_bound_to_pan_still_pans`](../../../src/ui/shell.rs#L12910) | With the middle button set to Pan in settings, a middle drag pans the view by the drag distance and doesn't change frame. |
| [`settings_shows_the_button_pickers`](../../../src/ui/shell.rs#L12922) | A smoke test: the Settings window, with its button-drag pickers, draws for two frames without panicking. |
| [`holding_space_pans_a_pen_drag_and_draws_nothing`](../../../src/ui/shell.rs#L12998) | With Space bound to pan, a pen drag with Space held pans the view by the drag distance, draws no stroke, and leaves no hold state behind. |
| [`releasing_space_mid_drag_stops_the_pan_and_draws_nothing_after`](../../../src/ui/shell.rs#L13014) | Like Krita: letting go of Space halfway through the drag stops the pan right there (20 of 40 px) and the rest of that pen-down draws nothing; the next drag is an ordinary stroke again. |
| [`holding_shift_adjusts_the_brush`](../../../src/ui/shell.rs#L13032) | With Shift bound to "tool adjust", a pen drag with Shift held makes the brush much bigger (over 1.5×) without panning or drawing. |
| [`a_hold_pan_in_transform_mode_moves_the_layer`](../../../src/ui/shell.rs#L13052) | In layer-transform mode, a Space-held drag moves the layer's transform instead of panning the view. |
| [`alt_on_a_handle_scales_instead_of_rotating_the_canvas`](../../../src/ui/shell.rs#L13102) | Alt-dragging a selection's corner handle scales the selection rather than doing the Alt "rotate canvas" hold. |
| [`shift_alt_on_a_handle_beats_intersect`](../../../src/ui/shell.rs#L13114) | Shift+Alt-dragging a handle scales the selection, and the Shift+Alt "intersect" selection mode doesn't kick in and cut the selection away. |
| [`space_over_a_handle_still_pans`](../../../src/ui/shell.rs#L13125) | With Space bound to pan, a Space-held drag starting on a selection handle pans the view and leaves the selection box alone. |
| [`a_held_mode_inside_an_unlifted_selection_still_starts_a_shape`](../../../src/ui/shell.rs#L13140) | Inside a selection that hasn't been lifted yet, a drag with the Ctrl+Shift "add" mode held draws a new selection shape (keeping the old area and adding the new) instead of lifting the pixels. |
| [`a_liquify_drag_moves_the_pixels`](../../../src/ui/shell.rs#L13155) | A Liquify drag pushes a small red square to the right (its alpha-weighted centre moves more than 5 px), the warp stays open without an undo step yet, and the Liquify panel and mesh then draw. |
| [`the_shape_panel_keeps_a_big_pen_size`](../../../src/ui/shell.rs#L13201) | Showing the Shape tool's Thickness slider doesn't clamp a big brush size (100) down to the slider's range, and switching back to the Pencil keeps it. |
| [`the_lasso_mode_hold_still_wins_on_the_lasso`](../../../src/ui/shell.rs#L13215) | When Space is bound both to pan and to the Lasso's "add" mode, a Space-held drag on the Lasso uses the selection mode and does not pan. |

**Test helpers** (used only by the tests above):

- [`frame`](../../../src/ui/shell.rs#L12840) — one frame with held modifiers that first lets `state.read_input` read the input (as the app's update does), then draws.
- [`state`](../../../src/ui/shell.rs#L12849) — a test state with panels hidden, 12 frames, playhead on frame 0.
- [`drag`](../../../src/ui/shell.rs#L12861) — press at `from`, move by `by` over 10 frames, release, settle a frame.
- [`frame_mods`](../../../src/ui/shell.rs#L12930) — like `frame` but with keyboard modifiers held for the frame.
- [`hold_drag`](../../../src/ui/shell.rs#L12948) — a left (pen) drag in four moves with a key and/or modifiers held from before the press; with `let_go_after: Some(n)` the key and modifiers are released after move `n`, mid-drag.
- [`space_pans`](../../../src/ui/shell.rs#L12991) — resets the shortcuts to defaults, binds Space to canvas pan and Ctrl+Space to play/pause.
- [`boxed`](../../../src/ui/shell.rs#L13069) — paints a red square at 200–300, selects it with the rectangle Lasso (not lifted), with default shortcuts, and returns the screen points of the selection's bottom-right handle and its middle.
- [`mods`](../../../src/ui/shell.rs#L13091) — builds an `egui::Modifiers` with Ctrl (and Command), Shift and Alt as asked.

**Test-only types and constants:** [`SCREEN`](../../../src/ui/shell.rs#L12838)

### `mod tool_button_tests`

Run with `cargo test ui::shell::tool_button_tests` · 1 tests

| Test | What it checks |
|---|---|
| [`a_stabilized_pen_shows_a_dot_on_its_button`](../../../src/ui/shell.rs#L13247) | No dots at first; turning the stabilizer on for Ink puts one accent dot on Ink's button even while the Fill tool is in hand; turning it on for the Eraser too makes two dots. |

**Test helpers** (used only by the tests above):

- [`dots`](../../../src/ui/shell.rs#L13236) — counts the accent-coloured circles of radius `STABILIZED_DOT_R` (the stabilizer marks) in a list of shapes.

### `mod shortcut_clash_tests`

Run with `cargo test ui::shell::shortcut_clash_tests` · 2 tests

| Test | What it checks |
|---|---|
| [`a_clean_map_flags_nothing`](../../../src/ui/shell.rs#L13334) | With no shared shortcuts, nothing mentions "clash", the Edit menu has no warning dot, and the full list (e.g. "Tool: Eraser") shows. |
| [`a_shared_combo_is_counted_flagged_and_filterable`](../../../src/ui/shell.rs#L13344) | Giving Ink the Pencil's key shows "2 shortcuts clash", a warning icon on both rows and the dot on the Edit menu; the "clashes only" filter shows just the two clashing rows; fixing the clash removes the summary, the dot and the filter's effect. |

**Test helpers** (used only by the tests above):

- [`shapes`](../../../src/ui/shell.rs#L13275) — One frame's painted shapes, flattened.
- [`walk`](../../../src/ui/shell.rs#L13276) — Walk.
- [`settle`](../../../src/ui/shell.rs#L13295) — four empty frames of `show`; returns the last frame's shapes.
- [`texts`](../../../src/ui/shell.rs#L13302) — every painted text with its rectangle.
- [`edit_dot`](../../../src/ui/shell.rs#L13313) — whether the orange warning dot (radius 3, `theme::warn()` colour) was painted on the Edit menu.
- [`setup`](../../../src/ui/shell.rs#L13321) — a test state with a 1920 × 1080 project loaded and a themed context.

### `mod krita_brush_tests`

Run with `cargo test ui::shell::krita_brush_tests` · 12 tests

| Test | What it checks |
|---|---|
| [`the_krita_pencil_is_offered_and_paints_through_the_canvas`](../../../src/ui/shell.rs#L13408) | Brush settings offers a "Pencil-5" preset button; once that Krita brush is chosen, a "Krita · Pencil-5 Tilted" summary replaces sliders it ignores (no "Hardness"), and a 180 px drag paints over 100 pixels as exactly one undo step. |
| [`the_stabilizer_offers_krita_s_options`](../../../src/ui/shell.rs#L13500) | Choosing the Stabilizer smoothing shows Krita's options (Delay, Delay distance, Finish line, Stabilize sensors) instead of the Weighted one's "Tail aggressiveness"; switching to Ink shows Ink's own "Basic" smoothing. |
| [`noise_options_come_with_noise_and_not_for_krita_brushes`](../../../src/ui/shell.rs#L13521) | For an app brush, "Noise" shows under Grain with its other controls hidden until noise is above 0 (then Noise size, Noise from, Fine, Paper, Stipple appear); a Krita brush shows none of the noise controls. |
| [`the_brush_picker_offers_krita_presets_and_picks_one`](../../../src/ui/shell.rs#L13559) | The picker shows App and Krita groups and their presets; searching "basic-5" and clicking a preset switches the brush to Krita mode with that preset id while keeping the colour, and shows its summary and "Wash" option; clicking a preset the engine can't paint ("Hairy") changes nothing. |
| [`the_krita_import_window_imports_and_uses_a_brush`](../../../src/ui/shell.rs#L13590) | Against a fake Krita install: the import button is under the picker and in the File menu (just looking doesn't open the window); the window lists "2 of 3 paint" and hides the unpaintable one, doesn't grow wider over frames, Import copies "My Grain" into the brushes folder and the picker, and Use switches to it. |
| [`closing_the_krita_import_window_frees_its_thumbnails_a_frame_later`](../../../src/ui/shell.rs#L13668) | Clicking the import window's X closes it, and its thumbnail textures are not freed in the frame that last drew them but all freed the next frame (freeing them early made wgpu reject the frame and close the app). |
| [`rescanning_brushes_frees_the_pickers_thumbnails_a_frame_later`](../../../src/ui/shell.rs#L13701) | A brush rescan drops an imported brush's thumbnail from the picker (keeping shipped Krita ones), but egui frees that texture only in the following frame, not the one that may have drawn it. |
| [`the_krita_editor_edits_resets_and_saves`](../../../src/ui/shell.rs#L13727) | In the editor under a picked Krita preset, ticking Wash stores an override the stroke uses (marked "edited", Flow appears); Reset drops it; turning the Speed sensor on for Size adds an override and off again removes it; "Save as new…" saves the edit as its own user preset file that joins the picker under Imported. |
| [`the_brush_picker_searches_by_name`](../../../src/ui/shell.rs#L13785) | Searching "tilted" shows matching Krita and app presets ("Pencil-5 Tilted", "Tilted") and hides non-matching ones like "Eraser Circle". |
| [`a_picked_krita_preset_paints_through_the_canvas`](../../../src/ui/shell.rs#L13797) | Picking the shipped wash preset "Basic-2 Opacity" and dragging on the canvas paints over 500 pixels as one undo step. |
| [`a_missing_library_preset_falls_back`](../../../src/ui/shell.rs#L13821) | A brush pointing at a library preset that no longer exists falls back to the built-in Pencil-5 Tilted brush instead of failing. |
| [`the_krita_pencil_cursor_is_its_tip_outline`](../../../src/ui/shell.rs#L13852) | Over the canvas, a Krita pencil's cursor is a thin rectangle (its tip's outline, at least 3× longer than wide) centred on the pointer, not the round brush ring. |

**Test helpers** (used only by the tests above):

- [`run`](../../../src/ui/shell.rs#L13378) — runs one frame on the 1400 × 900 screen that calls the given `show` function, and returns the shapes.
- [`walk`](../../../src/ui/shell.rs#L13379) — Walk.
- [`button`](../../../src/ui/shell.rs#L13398) — builds a primary-button press or release event with given modifiers.
- [`text_at`](../../../src/ui/shell.rs#L13444) — runs a frame and returns the centre of the text exactly equal to `label`, if drawn.
- [`walk`](../../../src/ui/shell.rs#L13445) — Walk.
- [`click_text`](../../../src/ui/shell.rs#L13465) — finds a label with `text_at` (panics if missing), then moves there, presses, releases and settles a frame.
- [`settle_library`](../../../src/ui/shell.rs#L13473) — polls the brush library's background loader (sleeping 5 ms between polls, up to 30 s) until it has finished, as the app's frames would.
- [`grow_brush_settings`](../../../src/ui/shell.rs#L13484) — drags the Brush settings window's corner down to the bottom of the screen so the parts below its default height are drawn.
- [`picker_state`](../../../src/ui/shell.rs#L13544) — a state with panels hidden, the Pencil chosen, Brush settings open and the library loaded, plus a themed context after two frames.
- [`frame_frees`](../../../src/ui/shell.rs#L13649) — one full app frame (textures synced, then drawn) that returns the texture ids egui freed in it.
- [`closed_paths`](../../../src/ui/shell.rs#L13829) — runs a frame and returns the points of every closed outline (`Shape::Path` with `closed`) it drew.
- [`walk`](../../../src/ui/shell.rs#L13830) — Walk.

**Test-only types and constants:** [`SCREEN`](../../../src/ui/shell.rs#L13376)

### `mod color_panel_layout_tests`

Run with `cargo test ui::shell::color_panel_layout_tests` · 1 tests

| Test | What it checks |
|---|---|
| [`the_brush_panel_holds_the_wheel_and_fits_a_1080p_screen`](../../../src/ui/shell.rs#L13886) | On a 1080p screen the Brush panel opens under Tools, ends above the bottom edge and clear of the Timeline, is exactly 220 px wide (no row pushes it wider), and there is no separate Color panel, not even in the Panels menu. |

### `mod camera_panel_tests`

Run with `cargo test ui::shell::camera_panel_tests` · 13 tests

| Test | What it checks |
|---|---|
| [`the_panel_jumps_from_camera_key_to_camera_key`](../../../src/ui/shell.rs#L13981) | With camera keys on frames 3 and 7, the panel's next/previous-key buttons jump the playhead 3 → 7 → 3, and the (unbound by default) next/prev camera-key actions do the same. |
| [`a_preset_gives_the_camera_a_frame_of_its_own`](../../../src/ui/shell.rs#L14002) | Clicking "Full HD" sets the output size to 1920 × 1080 and the header says so, while the canvas stays 3840 × 2160 and the camera doesn't move. |
| [`portrait_stands_the_frame_on_end_and_presets_keep_it_so`](../../../src/ui/shell.rs#L14016) | "Portrait" swaps the frame to 1080 × 1920, a later preset (HD 720p) stays portrait (720 × 1280), and "Landscape" turns it back. |
| [`looking_through_fits_the_camera_s_frame`](../../../src/ui/shell.rs#L14030) | Looking through the camera fits the whole canvas to the view when the camera has no frame of its own, and fits just the 1920 × 1080 frame (centred on the canvas) when it has one. |
| [`the_framing_buttons_frame_the_canvas`](../../../src/ui/shell.rs#L14049) | For a 1080p frame on the 4K canvas, "Fit" sets zoom 0.5 and centres, "1:1" sets zoom 1 and leaves the position, "Centre" recentres; a square frame fills by height and fits by width; each button is one undo step. |
| [`the_last_size_set_is_one_click_away_in_a_new_project`](../../../src/ui/shell.rs#L14084) | The last frame size set is remembered for the session; a "Last: 1080×1920" button is not offered while that size is in use, but appears in a newly loaded project (which opens on its canvas size) and applies it. |
| [`keep_inside_canvas_holds_the_panel_s_edits`](../../../src/ui/shell.rs#L14102) | With "Keep inside canvas" on (the default), panning or zooming the camera is clamped so the frame stays on the canvas; after clicking it off, a pan can go past the edge. |
| [`the_panel_turns_the_free_camera_box_on_and_off`](../../../src/ui/shell.rs#L14119) | The "Free shape" toggle turns the free-shape camera box mode on and off. |
| [`the_overlay_is_drawn_inside_the_frame`](../../../src/ui/shell.rs#L14129) | `camera_overlay_lines` returns 14 guide lines (4 thirds lines, 8 safe-area edges, 2 centre lines), all inside the frame, with the thirds a third in and title-safe a tenth in. |
| [`the_loops_section_says_how_long_the_move_will_run`](../../../src/ui/shell.rs#L14153) | The folded "Over loops" section opens on click; it says "Needs two camera keys" when there aren't two, then how long the stretched move runs ("plays once over 300 frames (12.5s)", 594 with ping-pong), and the header shows the setting ("Over loops · 3×"). |
| [`the_smooth_path_tick_sets_the_session_option`](../../../src/ui/shell.rs#L14177) | The "Smooth path through keys" checkbox turns the camera's smooth-path option on and off. |
| [`the_export_dialog_counts_a_stretched_move`](../../../src/ui/shell.rs#L14188) | The MP4 export dialog reports 100 frames for a plain 100-frame move, and 300 frames plus a note that "The camera's move spans 3 loops (Loop)" once the move is stretched over 3 loops. |
| [`match_canvas_hands_the_frame_back`](../../../src/ui/shell.rs#L14224) | "Match canvas" does nothing when the camera has no frame of its own, and after "Full HD" it removes the frame so output is the canvas size again. |

**Test helpers** (used only by the tests above):

- [`frame`](../../../src/ui/shell.rs#L13924) — one frame with held modifiers that first lets `state.read_input` read the input (as the app's update does), then draws.
- [`walk`](../../../src/ui/shell.rs#L13929) — Walk.
- [`click`](../../../src/ui/shell.rs#L13954) — a press and its release, one frame each.
- [`setup`](../../../src/ui/shell.rs#L13971) — a test state with a 1920 × 1080 project loaded and a themed context.
- [`two_keys`](../../../src/ui/shell.rs#L14140) — makes a 100-frame loop with camera keys on frame 0 and frame 99 (moved 500 px), playhead back on 0.
- [`walk`](../../../src/ui/shell.rs#L14193) — Walk.

**Test-only types and constants:** [`SCREEN`](../../../src/ui/shell.rs#L13919)

### `mod camera_frame_drag_tests`

Run with `cargo test ui::shell::camera_frame_drag_tests` · 10 tests

| Test | What it checks |
|---|---|
| [`with_camera_edit_off_a_drag_over_the_frame_draws`](../../../src/ui/shell.rs#L14291) | With no camera mode on, a drag across the camera frame is an ordinary pencil stroke (pixels painted, camera untouched, one undo step). |
| [`in_camera_edit_mode_a_drag_inside_the_frame_moves_the_camera_instead`](../../../src/ui/shell.rs#L14301) | In camera edit mode, a drag inside the frame moves the camera by the drag (120, 40 in document units), paints nothing, adds one undo step and clears the drag state. |
| [`in_camera_edit_mode_a_drag_clear_of_the_frame_still_draws`](../../../src/ui/shell.rs#L14316) | In camera edit mode, a drag outside the frame still paints and leaves the camera alone. |
| [`a_box_drawn_inside_the_frame_keys_the_camera_into_it`](../../../src/ui/shell.rs#L14341) | In draw-camera mode on frame 12, dragging a box keys the old camera on frame 0 and the box (kept to the frame's 16:9 shape) on frame 12, both easing in and out, paints nothing, in one undo step; frame 6 is in between, and undo removes both keys. |
| [`a_box_over_a_key_replaces_it`](../../../src/ui/shell.rs#L14377) | A box on a frame that already has a camera key replaces that key and keeps its ease; on a frame without one it adds just one new key (no extra frame-0 key once keys exist), and the previous key shows as a ghost. |
| [`a_free_box_drawn_on_the_canvas_sizes_the_camera`](../../../src/ui/shell.rs#L14406) | With "Free shape" on, the dragged box (400 × 300) becomes the camera's output size whatever its shape, with one key. |
| [`a_tap_centres_and_a_box_brought_back_does_nothing`](../../../src/ui/shell.rs#L14421) | A tap in draw-camera mode centres the camera on that point at its current size (one key, one undo); a drag that comes back to where it began shows a box mid-way but keys nothing and adds no undo step. |
| [`the_ghost_and_the_box_are_drawn`](../../../src/ui/shell.rs#L14479) | Draw-camera mode adds a dashed ghost of the frame the camera comes from (many more line segments), and while a box is being dragged two more four-cornered outlines appear (the two-tone box) on the next frame. |
| [`draw_camera_and_camera_edit_put_each_other_out`](../../../src/ui/shell.rs#L14510) | Turning on draw-camera mode turns camera edit off, and the reverse, since both claim the plain drag. |
| [`while_playing_a_press_in_draw_camera_mode_does_nothing`](../../../src/ui/shell.rs#L14521) | During playback, a drag in draw-camera mode neither keys the camera nor paints. |

**Test helpers** (used only by the tests above):

- [`run`](../../../src/ui/shell.rs#L14242) — runs one frame on the 1400 × 900 screen that calls the given `show` function, and returns the shapes.
- [`drag`](../../../src/ui/shell.rs#L14252) — press at `from`, move by `by` over 10 frames, release, settle a frame.
- [`setup`](../../../src/ui/shell.rs#L14269) — a test state with a 1920 × 1080 project loaded and a themed context.
- [`painted`](../../../src/ui/shell.rs#L14284) — counts the pixels with any alpha in a cell.
- [`keyed_box`](../../../src/ui/shell.rs#L14326) — the document-space top-left and bottom-right corners of the camera frame keyed on a given frame.
- [`near`](../../../src/ui/shell.rs#L14333) — whether two points are within 1 unit on both axes.
- [`tally`](../../../src/ui/shell.rs#L14455) — runs a frame and counts closed four-point outlines and line segments painted.
- [`walk`](../../../src/ui/shell.rs#L14456) — Walk.

**Test-only types and constants:** [`SCREEN`](../../../src/ui/shell.rs#L14240)

### `mod window_layout_tests`

Run with `cargo test ui::shell::window_layout_tests` · 8 tests

| Test | What it checks |
|---|---|
| [`a_narrowed_timeline_keeps_its_frame_counter_off_the_buttons`](../../../src/ui/shell.rs#L14627) | Dragging the Timeline panel narrow (to 300 px) keeps the frame counter (clock icon) to the right of the last toolbar button instead of on top of it, and the panel stops before getting narrower than its toolbar. |
| [`a_layer_row_spans_a_widened_layers_panel`](../../../src/ui/shell.rs#L14645) | After widening the Layers panel to 420 px, the selected row's highlight reaches (nearly) the panel's right edge. |
| [`x_sheet_stripes_reach_the_panel_edge`](../../../src/ui/shell.rs#L14662) | After widening the X-sheet panel to 400 px, its alternating row stripes all reach the panel's right edge. |
| [`dragging_settings_sideways_keeps_its_height`](../../../src/ui/shell.rs#L14683) | Widening the Settings window by 80 px, and then dragging it back, changes only its width, never its height (and it can't go narrower than it opened). |
| [`a_panel_closes_from_its_title_bar_and_the_reset_brings_it_back_fresh`](../../../src/ui/shell.rs#L14722) | Clicking the × in the Layers panel's title bar hides it and nothing of it is drawn; "reset panel layout" brings it back as a fresh window (a new area id) at its first spot. |
| [`picking_a_scheme_in_settings_restyles_the_app`](../../../src/ui/shell.rs#L14751) | Clicking the "Paper" colour-scheme card switches the scheme and restyles egui and the theme colours at once; "Custom" shows the five colour pickers ("Buttons and fields"…) and uses colours derived from them; setting text colour equal to the panel colour shows a "hard to read" warning. |
| [`a_long_file_name_is_cut_in_the_middle`](../../../src/ui/shell.rs#L14804) | `ellipsize_middle` leaves short names alone and cuts long ones in the middle with "…" to exactly the asked length, keeping the extension (or the stem's end if there is none), counting characters rather than bytes (Japanese names work). |
| [`cleanup_layers_leave_the_layers_panel_its_size`](../../../src/ui/shell.rs#L14827) | Adding six cleanup layers in a chain (longer and longer names like "Body clean clean") never changes the Layers panel's size; the list scrolls instead, with the newest top row in view and the original "Body" scrolled away. |

**Test helpers** (used only by the tests above):

- [`setup`](../../../src/ui/shell.rs#L14545) — a test state with a 1920 × 1080 project loaded and a themed context.
- [`run`](../../../src/ui/shell.rs#L14553) — runs one frame on the 1400 × 900 screen that calls the given `show` function, and returns the shapes.
- [`settle`](../../../src/ui/shell.rs#L14562) — four empty frames of `show`; returns the last frame's shapes.
- [`window`](../../../src/ui/shell.rs#L14570) — reads a window's rectangle by its egui `Id`.
- [`drag_right_edge`](../../../src/ui/shell.rs#L14575) — drags a window's right edge to x over 12 frames, releases, then settles.
- [`texts`](../../../src/ui/shell.rs#L14595) — every painted text with its rectangle.
- [`walk`](../../../src/ui/shell.rs#L14596) — Walk.
- [`fills`](../../../src/ui/shell.rs#L14611) — every filled rectangle painted in a given colour.
- [`walk`](../../../src/ui/shell.rs#L14612) — Walk.
- [`close_x`](../../../src/ui/shell.rs#L14703) — the middle of the close × egui paints at the right of a window's title bar (the right-most line-segment midpoint in the top 36 px).
- [`walk`](../../../src/ui/shell.rs#L14704) — Walk.
- [`settle_layers`](../../../src/ui/shell.rs#L14819) — `settle` with the Layers panel alone drawn through `panel_window`.

**Test-only types and constants:** [`SCREEN`](../../../src/ui/shell.rs#L14541), [`Shapes`](../../../src/ui/shell.rs#L14543)

### `mod canvas_gpu_tests`

Run with `cargo test ui::shell::canvas_gpu_tests` · 3 tests

| Test | What it checks |
|---|---|
| [`floats_draw_at_their_own_layers_depth`](../../../src/ui/shell.rs#L15015) | With two layers picked and part of each lifted into a floating selection, each float is painted just above its own layer (the bottom layer's float below the top layer's drawing), and the bottom float uses its layer's 50 % opacity (alpha 127). |
| [`gpu_path_draws_what_the_mesh_path_draws`](../../../src/ui/shell.rs#L15063) | On a scene with every kind of artwork, the GPU path paints exactly the same quads as the mesh path (same order, corners, tint and clip); without a GPU no callbacks are used; and with it, every piece (reference layer, merged group, plain and clipped layers, past/future/pinned ghosts, floating selection) goes through the pipeline. |
| [`a_texture_refilled_whole_draws_as_a_mesh_that_frame`](../../../src/ui/shell.rs#L15108) | A texture re-uploaded in full this frame (marked in `fresh_full`) still has its old image on the GPU while the frame is built, so that frame it is drawn as a mesh and not through the pipeline. |

**Test helpers** (used only by the tests above):

- [`tokens`](../../../src/ui/shell.rs#L14878) — turns each painted shape into a `Token`: a 4-vertex mesh or a canvas `Recorded` callback becomes `Quad`, anything else `Other` (it also asserts each callback covers exactly its clip rect).
- [`block`](../../../src/ui/shell.rs#L14904) — a new project-sized `Canvas` with a filled rectangle of one RGBA colour.
- [`scene`](../../../src/ui/shell.rs#L14922) — builds a state with every kind of artwork at once: a reference layer, a group drawn merged at 60 % opacity, a plain layer with past, future and pinned onion ghosts, a layer clipped to it, and a lifted, moved floating selection; returns the state and the reference, group, plain and clipped layer indices.
- [`paint`](../../../src/ui/shell.rs#L14991) — two full frames (textures synced, then drawn) at times 0 and 1 so ghosts get built; returns the second frame's shapes.

**Test-only types and constants:** [`Token`](../../../src/ui/shell.rs#L14868)

### `mod axis_lock_tests`

Run with `cargo test ui::shell::axis_lock_tests` · 3 tests

| Test | What it checks |
|---|---|
| [`shift_after_the_press_holds_a_line_and_at_the_press_still_pans`](../../../src/ui/shell.rs#L15177) | Shift already held when the line tool's press lands is the pan hold (the view pans, nothing is drawn); Shift pressed once the line is under way snaps a slightly slanted line level through the press point. |
| [`space_partway_slides_a_rect_and_alt_grows_one_from_the_press`](../../../src/ui/shell.rs#L15224) | With default keys (Space is also Play/Pause), pressing Space partway through a rectangle slides the whole rectangle with the pointer and doesn't start playback; Alt pressed partway makes the press point the centre (as far left as right) without rotating or panning the view. |
| [`with_space_on_brush_size_space_partway_still_slides_a_rect`](../../../src/ui/shell.rs#L15305) | With plain Space bound to the brush-size hold and Ctrl+Space to play: Space down before the press resizes the brush and draws nothing, while Space pressed partway through a rectangle slides it, leaves the size alone and makes one undo step. |

**Test helpers** (used only by the tests above):

- [`frame`](../../../src/ui/shell.rs#L15142) — one frame with held modifiers that first lets `state.read_input` read the input (as the app's update does), then draws.
- [`button`](../../../src/ui/shell.rs#L15155) — builds a primary-button press or release event with given modifiers.
- [`alpha_at_doc`](../../../src/ui/shell.rs#L15164) — the alpha of the current cell's pixel under a document point (0 if there is no cell).

**Test-only types and constants:** [`SCREEN`](../../../src/ui/shell.rs#L15139)

### `mod pen_slot_tests`

Run with `cargo test ui::shell::pen_slot_tests` · 6 tests

| Test | What it checks |
|---|---|
| [`the_pens_and_the_eraser_lead_and_the_other_tools_follow`](../../../src/ui/shell.rs#L15447) | The pens and the eraser after them on the first row; every other tool on the second, the colour picker last. |
| [`the_menu_sets_a_slots_icon_and_brush`](../../../src/ui/shell.rs#L15465) | The right-click menu gives the slot another icon and another brush, leaving the pen in hand alone. |
| [`the_plus_adds_a_slot_and_opens_its_menu`](../../../src/ui/shell.rs#L15482) | The "+" adds a slot at the end, picks it and opens its menu; Remove there asks for a second click, then takes the slot out. |
| [`duplicate_puts_a_copy_after_its_slot`](../../../src/ui/shell.rs#L15509) | Duplicate in a slot's menu puts a copy right after it, picked, with the copy's own menu open. |
| [`six_slots_and_the_eraser_make_a_row_of_seven`](../../../src/ui/shell.rs#L15523) | Six slots and the eraser are a row of seven, like the tools under them, with no "+" left. |
| [`the_tools_panel_opens_as_tall_as_its_start_height`](../../../src/ui/shell.rs#L15541) | How tall the Tools panel opens on the pencil: what the Brush panel under it is placed by before the panel has been laid out. |

**Test helpers** (used only by the tests above):

- [`frame`](../../../src/ui/shell.rs#L15385) — one frame with held modifiers that first lets `state.read_input` read the input (as the app's update does), then draws.
- [`settle`](../../../src/ui/shell.rs#L15397) — four empty frames of `show`; returns the last frame's shapes.
- [`text_rects`](../../../src/ui/shell.rs#L15405) — runs a frame and returns every painted text with its rectangle.
- [`find`](../../../src/ui/shell.rs#L15406) — Find.
- [`at`](../../../src/ui/shell.rs#L15420) — converts a document point to its screen position.
- [`click`](../../../src/ui/shell.rs#L15425) — a press and its release, one frame each.
- [`setup`](../../../src/ui/shell.rs#L15437) — a test state with a 1920 × 1080 project loaded and a themed context.

### `mod size_row_tests`

Run with `cargo test ui::shell::size_row_tests` · 1 tests

| Test | What it checks |
|---|---|
| [`a_growing_size_never_widens_the_panel`](../../../src/ui/shell.rs#L15571) | Dragging the size up to the largest, a frame per step, on a pen and on Shape, lock on and off, leaves the panel the width it opened at. The lock icon after the slider was drawn wider than the room the slider left it, and from three-digit sizes on, the panel grew every frame. |

**Test helpers** (used only by the tests above):

- [`width`](../../../src/ui/shell.rs#L15555) — Width.
