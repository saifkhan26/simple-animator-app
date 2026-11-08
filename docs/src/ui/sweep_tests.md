# `src/ui/sweep_tests.rs` — headless layout sweep over every window (test-only)

> **Source:** [src/ui/sweep_tests.rs](../../../src/ui/sweep_tests.rs) · **Module:** `crate::ui::sweep_tests` · **Size:** 886 lines · Line links as of commit `1fa5845`

## What this file is for

Test code that opens **every window the UI shows** — each panel, the Tools panel with each tool, the menu, the
mini timeline, Settings, Brush settings, grid settings, the Krita import window, New project, every Export kind,
the import range dialog, the save error — one at a time in a fresh headless egui context, and hunts for **layout
faults**:

- a window that creeps or ratchets larger frame after frame;
- text drawn over other text, or cut off at the window's edge;
- dragging one edge changing the other dimension;
- combo-box lists opening off screen;
- egui ID clashes between windows;
- all of that at rest, at the narrowest width, with every folded section open, with a busy project, and at common
  screen sizes.

It's compiled only for tests: [ui/shell.rs](shell.md) declares it as
`#[cfg(test)] #[path = "sweep_tests.rs"] mod sweep_tests;`, so it is the module `crate::ui::shell::sweep_tests` and
can use `shell`'s private functions (`use super::*;`). Run it with:

```bash
cargo test sweep_tests
```

How it works: a `Rig` holds one window, an `AppState::for_test()` and an `egui::Context`; `Rig::frame` runs one egui
frame with synthetic input events and returns the shapes egui would paint; the checks then walk those shapes
(text runs with their clip rectangles, collapsing-header arrows, combo arrows) to find faults. No real window or
GPU is involved.

## How it connects

- **Uses:** [ui/shell.rs](shell.md) (draws each window), [app.rs](../app.md) (`AppState::for_test`, export/import
  types).

## Rust concepts in this file

**`#[path]` attribute.** Lets a module live in a file whose location doesn't follow the usual rule — here
`src/ui/sweep_tests.rs` is a child of `shell`, not of `ui`. More: [Attributes and cfg](../../rust-primer.md#attributes-and-cfg)

**Implementing `Debug` by hand.** `impl std::fmt::Debug for Win` controls how a window's name prints in failure
messages. More: [Traits and derive](../../rust-primer.md#traits-and-derive)

**Nested recursive helpers.** `texts`, `closed_headers` and `down_arrows` each contain an inner `fn walk` that
recurses through egui's nested shapes.

## Types

### `type Shapes`
[src/ui/sweep_tests.rs:14](../../../src/ui/sweep_tests.rs#L14)

```rust
type Shapes = Vec<egui::epaint::ClippedShape>;
```

The shapes egui produced for one frame, with their clip rectangles.

### `const SCREEN`
[src/ui/sweep_tests.rs:16](../../../src/ui/sweep_tests.rs#L16)

```rust
const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, pos2(1600.0, 1000.0));
```

The headless screen size the sweep uses.

### `const PANELS`
[src/ui/sweep_tests.rs:18](../../../src/ui/sweep_tests.rs#L18)

```rust
const PANELS: [PanelId; 7] = [
    PanelId::Tools,
    PanelId::Brush,
    PanelId::Layers,
    PanelId::Onion,
    PanelId::Camera,
// …
```

Every floating panel to test.

### `enum Win`
[src/ui/sweep_tests.rs:29](../../../src/ui/sweep_tests.rs#L29)

One window the sweep can open.

| Variant | Meaning |
|---|---|
| `Panel(PanelId)` | One floating panel. |
| `ToolPanel(ActiveTool)` | The Tools panel with another tool up: its options are that tool's. |
| `Menu` | The top-bar menu. |
| `MiniTimeline` | The mini timeline bar. |
| `Settings` | The Settings window. |
| `BrushSettings` | Brush settings. |
| `GridSettings` | Perspective grid settings. |
| `KritaImport` | Import Krita brushes. |
| `NewProject` | The New project dialog. |
| `Export(ExportKind)` | An export dialog of the given kind. |
| `ImportRange` | The frame-range import dialog. |
| `SaveError` | The save-failed dialog. |

### `struct Rig`
[src/ui/sweep_tests.rs:203](../../../src/ui/sweep_tests.rs#L203)

One window, drawn on its own in a fresh UI.

| Field | Type | Meaning |
|---|---|---|
| `win` | `Win` | The window under test. |
| `ctx` | `egui::Context` | Its own egui context. |
| `state` | `AppState` | Its own app state. |

### `struct Text`
[src/ui/sweep_tests.rs:313](../../../src/ui/sweep_tests.rs#L313)

A piece of text drawn: its string, its bounds and the clip it was drawn in.

| Field | Type | Meaning |
|---|---|---|
| `text` | `String` | The string. |
| `rect` | `Rect` | Its bounds. |
| `clip` | `Rect` | The clip rectangle it was drawn within. |

## `impl std::fmt::Debug for Win`

### `Win::fmt`
[src/ui/sweep_tests.rs:46](../../../src/ui/sweep_tests.rs#L46)

```rust
fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
```

Prints the window's name for failure reports (the `Debug` trait).

## Free functions

### `all`
[src/ui/sweep_tests.rs:64](../../../src/ui/sweep_tests.rs#L64)

```rust
fn all() -> Vec<Win>
```

Every window to sweep.

### `panel_width`
[src/ui/sweep_tests.rs:88](../../../src/ui/sweep_tests.rs#L88)

```rust
fn panel_width(id: PanelId) -> f32
```

The width `draw` gives each panel.

## `impl Win`

### `Win::open`
[src/ui/sweep_tests.rs:98](../../../src/ui/sweep_tests.rs#L98)

```rust
fn open(self, state: &mut AppState)
```

Puts `state` in the condition that shows this window (opens the dialog, picks the tool, sets up an import…).

### `Win::show`
[src/ui/sweep_tests.rs:163](../../../src/ui/sweep_tests.rs#L163)

```rust
fn show(self, state: &mut AppState, ctx: &egui::Context)
```

Draws just this window for one frame.

### `Win::scrolls_sideways`
[src/ui/sweep_tests.rs:185](../../../src/ui/sweep_tests.rs#L185)

```rust
fn scrolls_sideways(self) -> bool
```

Whether text running past a side edge is a list scrolled sideways rather than cut off.

### `Win::resizable`
[src/ui/sweep_tests.rs:189](../../../src/ui/sweep_tests.rs#L189)

```rust
fn resizable(self) -> bool
```

Whether the window can be resized by its edges.

## `impl Rig`

### `Rig::new`
[src/ui/sweep_tests.rs:210](../../../src/ui/sweep_tests.rs#L210)

```rust
fn new(win: Win) -> Self
```

A fresh rig: new `AppState::for_test()`, new egui context, the window opened.

### `Rig::frame`
[src/ui/sweep_tests.rs:222](../../../src/ui/sweep_tests.rs#L222)

```rust
fn frame(&mut self, events: Vec<egui::Event>) -> Shapes
```

Runs one egui frame with `events` as input and returns the painted shapes.

### `Rig::settle`
[src/ui/sweep_tests.rs:233](../../../src/ui/sweep_tests.rs#L233)

```rust
fn settle(&mut self) -> Shapes
```

Enough frames for egui's open/close and scroll animations to finish.

### `Rig::rect`
[src/ui/sweep_tests.rs:242](../../../src/ui/sweep_tests.rs#L242)

```rust
fn rect(&self) -> Rect
```

The window's outer rect. Exactly one is shown.

### `Rig::click`
[src/ui/sweep_tests.rs:254](../../../src/ui/sweep_tests.rs#L254)

```rust
fn click(&mut self, at: Pos2)
```

A synthetic press and release at `at`.

### `Rig::drag`
[src/ui/sweep_tests.rs:261](../../../src/ui/sweep_tests.rs#L261)

```rust
fn drag(&mut self, from: Pos2, to: Pos2)
```

A synthetic drag from `from` to `to` over several frames.

### `Rig::drag_right_edge`
[src/ui/sweep_tests.rs:273](../../../src/ui/sweep_tests.rs#L273)

```rust
fn drag_right_edge(&mut self, dx: f32)
```

Drags the window's right edge by `dx`.

### `Rig::drag_bottom_edge`
[src/ui/sweep_tests.rs:279](../../../src/ui/sweep_tests.rs#L279)

```rust
fn drag_bottom_edge(&mut self, dy: f32)
```

Drags the window's bottom edge by `dy`.

### `Rig::open_sections`
[src/ui/sweep_tests.rs:286](../../../src/ui/sweep_tests.rs#L286)

```rust
fn open_sections(&mut self)
```

Open every folded section, one click at a time, until none is left.

## Free functions

### `button`
[src/ui/sweep_tests.rs:303](../../../src/ui/sweep_tests.rs#L303)

```rust
fn button(pos: Pos2, pressed: bool) -> egui::Event
```

A primary-button pointer event at `pos`.

### `texts`
[src/ui/sweep_tests.rs:319](../../../src/ui/sweep_tests.rs#L319)

```rust
fn texts(shapes: &Shapes) -> Vec<Text>
```

Every text run in a frame's shapes.

#### `texts::walk`
[src/ui/sweep_tests.rs:320](../../../src/ui/sweep_tests.rs#L320)

```rust
fn walk(shape: &egui::Shape, clip: Rect, out: &mut Vec<Text>)
```

Recurses through nested shapes collecting text runs.

### `closed_headers`
[src/ui/sweep_tests.rs:341](../../../src/ui/sweep_tests.rs#L341)

```rust
fn closed_headers(shapes: &Shapes) -> Vec<Pos2>
```

Where a folded `CollapsingHeader` arrow is drawn: egui paints it as a small filled triangle pointing right. (Open, it points down, as does a combo box's.)

#### `closed_headers::walk`
[src/ui/sweep_tests.rs:342](../../../src/ui/sweep_tests.rs#L342)

```rust
fn walk(shape: &egui::Shape, clip: Rect, out: &mut Vec<Pos2>)
```

Recurses through shapes looking for right-pointing triangles.

### `down_arrows`
[src/ui/sweep_tests.rs:368](../../../src/ui/sweep_tests.rs#L368)

```rust
fn down_arrows(shapes: &Shapes) -> Vec<Pos2>
```

Where a combo box's arrow is drawn: a small filled triangle pointing down. (An open `CollapsingHeader`'s points down too; a click on it just folds the section.)

#### `down_arrows::walk`
[src/ui/sweep_tests.rs:369](../../../src/ui/sweep_tests.rs#L369)

```rust
fn walk(shape: &egui::Shape, clip: Rect, out: &mut Vec<Pos2>)
```

Recurses through shapes looking for down-pointing triangles.

### `faults`
[src/ui/sweep_tests.rs:393](../../../src/ui/sweep_tests.rs#L393)

```rust
fn faults(rig: &Rig, shapes: &Shapes, when: &str) -> Vec<String>
```

Everything wrong with one frame of one window.

### `report`
[src/ui/sweep_tests.rs:429](../../../src/ui/sweep_tests.rs#L429)

```rust
fn report(findings: Vec<String>)
```

Fails the test with every finding listed, if there are any.

### `snapshot`
[src/ui/sweep_tests.rs:726](../../../src/ui/sweep_tests.rs#L726)

```rust
fn snapshot(rig: &mut Rig, ppp: f32) -> image::RgbaImage
```

What `rig` draws, painted on the CPU into an image cropped to its window: egui's meshes over the theme background, at `ppp` pixels a point. For looking at a layout without starting the app, which would share the real config folder.

## Tests

### Tests in this file

Run with `cargo test ui::sweep_tests` · 9 tests

| Test | What it checks |
|---|---|
| [`every_window_is_clean_and_steady_at_rest`](../../../src/ui/sweep_tests.rs#L436) | Shown, left alone: nothing overlaps, runs off or clashes, and the window holds its size frame after frame. |
| [`every_window_resizes_on_one_axis_at_a_time`](../../../src/ui/sweep_tests.rs#L456) | Dragging one edge leaves the other dimension alone and lands where dragged; dragging back returns the window to its size. (Wider, a window sized by wrapped text may get shorter: it wraps onto fewer lines.) |
| [`every_window_stays_clean_at_its_narrowest`](../../../src/ui/sweep_tests.rs#L487) | Pulled as narrow as it goes: what is left still doesn't overlap or spill. |
| [`every_window_is_clean_with_its_sections_open`](../../../src/ui/sweep_tests.rs#L502) | Every folded section opened: nothing overlaps or spills, the window holds steady, and it gets no wider than it opened at — a section is laid out to the window, not the other way round. |
| [`a_busy_project_leaves_every_window_its_size`](../../../src/ui/sweep_tests.rs#L528) | A busier project — more layers, long names, cleanup chains, a group, faded layers, a long scene, camera keys, a brush with many sizes remembered, brush values a drag left unrounded — leaves every window the size it was, and still clean. |
| [`every_combo_box_opens_on_screen`](../../../src/ui/sweep_tests.rs#L601) | Every combo box opened, one at a time, with every section open: its list lands on the screen, and the window under it keeps its width. |
| [`the_whole_ui_fits_common_screens`](../../../src/ui/sweep_tests.rs#L655) | The whole UI at common screen sizes: every window on screen, and no ID clashes between windows. |
| [`the_brush_panel_keeps_its_rows_apart_at_its_narrowest`](../../../src/ui/sweep_tests.rs#L700) | However narrow the Brush panel goes, the H/S/L fields stay clear of the Paste button after them, and the backdrop row keeps its colour and opacity. (Their frames, not their text, would collide, which the overlap sweep doesn't see.) |
| [`snapshots`](../../../src/ui/sweep_tests.rs#L802) | Writes the Brush panel and Settings as PNGs into `SNAPSHOT_DIR`, for a look at the layout. `SNAPSHOT_DIR=... cargo test -- --ignored snapshots` |
