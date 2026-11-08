# `src/ui/curve_editor.rs` — the sensor curve editor widget

> **Source:** [src/ui/curve_editor.rs](../../../src/ui/curve_editor.rs) · **Module:** `crate::ui::curve_editor` · **Size:** 201 lines · Line links as of commit `36405ad`

## What this file is for

A small widget like Krita's curve widget: a square showing a sensor's response curve (input along the bottom,
the option's value up the side). Drag a point to move it, click or drag on empty space to add one, right-click a
point to remove it. The curve is edited **in place as Krita's text form** (`"0,0;0.5,0.2;1,1;"`), and drawn with
the same spline evaluation Krita uses ([tools/krita/curve.rs](../tools/krita/curve.md)).

## How it connects

- **Used by:** [ui/krita_editor.rs](krita_editor.md) (editing preset sensor curves).
- **Uses:** [tools/krita/curve.rs](../tools/krita/curve.md) (`CubicCurve` parse/format/evaluate).

## Rust concepts in this file

**`&mut String` parameters.** `curve_editor(ui, id, &mut curve)` rewrites the caller's string directly when a point
moves and returns `true` — immediate-mode two-way binding.

## Types

### `const GRAB`
[src/ui/curve_editor.rs:12](../../../src/ui/curve_editor.rs#L12)

```rust
const GRAB: f32 = 8.0;
```

How near (screen pixels) a press must be to grab a point.

### `const GAP`
[src/ui/curve_editor.rs:14](../../../src/ui/curve_editor.rs#L14)

```rust
const GAP: f64 = 1e-3;
```

Minimum x gap between neighbouring points, so they stay in order.

## Free functions

### `curve_editor`
[src/ui/curve_editor.rs:18](../../../src/ui/curve_editor.rs#L18)

```rust
pub fn curve_editor(ui: &mut egui::Ui, id: egui::Id, curve: &mut String) -> bool
```

Draws the curve box and handles editing: parses the curve text, draws the grid and the spline (from Krita's
transfer table), draws the points; a press near a point grabs it (dragging keeps it between its neighbours,
ends pinned to x = 0 and 1), a press elsewhere adds a point, right-click removes one. On any change it writes the
curve back as text and returns `true`.

## Tests

### `mod tests`

Run with `cargo test ui::curve_editor::tests` · 1 tests

| Test | What it checks |
|---|---|
| [`click_adds_drag_moves_right_click_removes`](../../../src/ui/curve_editor.rs#L165) | Click adds drag moves right click removes. |

**Test helpers** (used only by the tests above):

- [`frame`](../../../src/ui/curve_editor.rs#L132) — One frame of the widget alone at the screen's top left: 260 wide, so its curve area runs from (4, 4) to (256, 152).
- [`at`](../../../src/ui/curve_editor.rs#L147) — At.
- [`press`](../../../src/ui/curve_editor.rs#L151) — Press.
- [`points`](../../../src/ui/curve_editor.rs#L160) — Points.
