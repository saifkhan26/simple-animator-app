# `src/app/lasso_fill.rs` — the Fill tool's lasso mode

> **Source:** [src/app/lasso_fill.rs](../../../src/app/lasso_fill.rs) · **Module:** `crate::app::lasso_fill` · **Size:** 441 lines · Line links as of commit `36405ad`

## What this file is for

The Fill tool has two modes: **Bucket** (click to flood an area) and **Lasso** (draw an outline and
whatever it encloses fills with the brush colour). This file is the lasso mode. The outline works exactly
like the Lasso selection's (shared `TapLasso`, see [tools/tap_lasso.rs](../tools/tap_lasso.md)):

- **drag** to draw it freehand — letting go fills;
- **tap** to start a polygon, tap corners, and close it by tapping the first corner, double-clicking or
  pressing Enter (Backspace removes the last corner).

Held keys change what a fill does: erase the shape instead, flip the "Enclose & fill" option (fill only
the areas the shape's lines shut in), and snap polygon edges to 15° steps. Fills respect the selection,
alpha lock, the Fill tool's opacity and "Behind" (paint under existing paint). One undo step each.

## How it connects

- **Used by:** [app.rs](../app.md) (`dispatch`: `ToolFill` toggles mode; pointer handlers route Fill
  presses here when in Lasso mode; `undo` calls `fill_lasso_back`), [ui/shell.rs](../ui/shell.md)
  (drawing the outline, Tools panel mode toggle).
- **Uses:** [tools/tap_lasso.rs](../tools/tap_lasso.md), [tools/lasso.rs](../tools/lasso.md) (turning the
  outline into a mask), [tools/fill.rs](../tools/fill.md) (enclose mode, painting the fill).

## Rust concepts in this file

**`atan2`, `hypot`, `to_radians` are methods.** `dy.atan2(dx)` is `Math.atan2(dy, dx)`, `dx.hypot(dy)` is
`Math.hypot(dx, dy)`.

**Matching on a tuple with an `Option` inside.**
```rust
match (snap, self.fill_lasso.as_ref().and_then(|l| l.lasso.pts.last())) {
    (true, Some(&from)) => snap_15(from, doc),
    _ => doc,
}
```
Both conditions in one pattern: "snapping, and there is a last point". More:
[Enums and match](../../rust-primer.md#enums-and-match)

## Types

### `enum FillMode`
[src/app/lasso_fill.rs:24](../../../src/app/lasso_fill.rs#L24)

What the Fill tool does with a press: flood from it, or draw a lasso.

| Variant | Meaning |
|---|---|
| `Bucket` | A press floods outward from where it lands (classic paint bucket). |
| `Lasso` | A press starts drawing a lasso outline to fill. |

### `struct LassoHeld`
[src/app/lasso_fill.rs:32](../../../src/app/lasso_fill.rs#L32)

What the combos held at a lasso fill's last press ask for.

| Field | Type | Meaning |
|---|---|---|
| `erase` | `bool` | Clear the shape instead of filling it. |
| `enclose` | `bool` | The other way from the Enclose & fill toggle, for this shape. |
| `snap` | `bool` | Polygon corners turn in 15° steps. |

### `struct FillLasso`
[src/app/lasso_fill.rs:43](../../../src/app/lasso_fill.rs#L43)

A lasso fill being drawn, in document space.

| Field | Type | Meaning |
|---|---|---|
| `lasso` | `TapLasso` | The outline being drawn (freehand or polygon), in document space. |
| `held` | `LassoHeld` | The modifier holds read at the last press. |

### `const TAP_PX`
[src/app/lasso_fill.rs:49](../../../src/app/lasso_fill.rs#L49)

```rust
const TAP_PX: f32 = 3.0;
```

A press that travels less than this many screen pixels is a tap.

## Free functions

### `snap_15`
[src/app/lasso_fill.rs:52](../../../src/app/lasso_fill.rs#L52)

```rust
pub fn snap_15(from: (f32, f32), to: (f32, f32)) -> (f32, f32)
```

Rotates point `to` around `from` to the nearest multiple of 15°, keeping its distance: compute the
angle with `atan2`, round it to a 15° step, and rebuild the point with `cos`/`sin`.

## `impl AppState`

### `AppState::toggle_fill_mode`
[src/app/lasso_fill.rs:65](../../../src/app/lasso_fill.rs#L65)

```rust
pub fn toggle_fill_mode(&mut self)
```

Switches the Fill tool between Bucket and Lasso (pressing the Fill key while Fill is already active).

### `AppState::set_fill_mode`
[src/app/lasso_fill.rs:73](../../../src/app/lasso_fill.rs#L73)

```rust
pub fn set_fill_mode(&mut self, m: FillMode)
```

Sets the Fill mode, discarding a half-drawn lasso when it changes.

### `AppState::fill_polygon_active`
[src/app/lasso_fill.rs:82](../../../src/app/lasso_fill.rs#L82)

```rust
pub fn fill_polygon_active(&self) -> bool
```

Whether a polygon fill is being tapped out (then Enter, Backspace and double-click belong to it).

### `AppState::fill_lasso`
[src/app/lasso_fill.rs:87](../../../src/app/lasso_fill.rs#L87)

```rust
pub fn fill_lasso(&self) -> Option<&FillLasso>
```

The lasso fill in progress, for the canvas to draw.

### `AppState::fill_corner_at`
[src/app/lasso_fill.rs:94](../../../src/app/lasso_fill.rs#L94)

```rust
pub fn fill_corner_at(&self, doc: (f32, f32), snap: bool) -> (f32, f32)
```

Where the next polygon corner goes: the pointer position, or snapped to a 15° step from the last point
while the snap hold is down.

### `AppState::fill_lasso_down`
[src/app/lasso_fill.rs:102](../../../src/app/lasso_fill.rs#L102)

```rust
pub fn fill_lasso_down(&mut self, doc: (f32, f32), held: LassoHeld)
```

A press with the lasso fill at document point `doc`: starts a new outline, or adds a corner to the polygon
in progress, remembering which holds were down.

### `AppState::fill_lasso_move`
[src/app/lasso_fill.rs:122](../../../src/app/lasso_fill.rs#L122)

```rust
pub fn fill_lasso_move(&mut self, doc: (f32, f32))
```

A drag: extends the freehand outline (points at least a fraction of a document pixel apart, finer when
zoomed in). A move shorter than `TAP_PX` screen pixels still counts as a tap.

### `AppState::fill_lasso_up`
[src/app/lasso_fill.rs:133](../../../src/app/lasso_fill.rs#L133)

```rust
pub fn fill_lasso_up(&mut self)
```

Release: a freehand outline lands as a fill; a tap starts a polygon or places its corner.

### `AppState::fill_lasso_finish`
[src/app/lasso_fill.rs:144](../../../src/app/lasso_fill.rs#L144)

```rust
pub fn fill_lasso_finish(&mut self)
```

Closes the polygon in progress and fills it (Enter, or a tap on the first corner).

### `AppState::fill_lasso_double_click`
[src/app/lasso_fill.rs:155](../../../src/app/lasso_fill.rs#L155)

```rust
pub fn fill_lasso_double_click(&mut self)
```

A double-click closes the polygon when both clicks landed on the same spot.

### `AppState::fill_lasso_back`
[src/app/lasso_fill.rs:165](../../../src/app/lasso_fill.rs#L165)

```rust
pub fn fill_lasso_back(&mut self) -> bool
```

Backspace / Ctrl+Z during a polygon: removes the newest corner (and the whole polygon when none is left).
Returns `false` when no polygon is active, so the key keeps its normal meaning.

### `AppState::land_lasso_fill`
[src/app/lasso_fill.rs:180](../../../src/app/lasso_fill.rs#L180)

```rust
fn land_lasso_fill(&mut self, lasso: TapLasso, held: LassoHeld)
```

Performs the fill: turns the closed outline into a mask, then fills (or erases) it on the active drawing —
within the selection, at the Fill tool's opacity, behind existing paint with "Behind", or only the regions
its lines enclose with "Enclose & fill" — as one undo step. An outline that encloses nothing does nothing.

## Tests

### `mod tests`

Run with `cargo test app::lasso_fill::tests` · 9 tests

| Test | What it checks |
|---|---|
| [`a_held_erase_clears_the_shape`](../../../src/app/lasso_fill.rs#L301) | A held erase clears the shape. |
| [`the_selection_holds_the_fill`](../../../src/app/lasso_fill.rs#L313) | The selection holds the fill. |
| [`behind_fills_round_the_lines_and_held_enclose_fills_only_inside_them`](../../../src/app/lasso_fill.rs#L324) | Behind fills round the lines and held enclose fills only inside them. |
| [`a_grainy_lasso_fill_matches_the_bucket`](../../../src/app/lasso_fill.rs#L349) | The lasso and the bucket lay down the same specks of paper. |
| [`enclose_fills_a_shape_closed_by_the_canvas_edge`](../../../src/app/lasso_fill.rs#L366) | A lasso dragged out past the canvas's corner fills the shape two strokes make with its edges. |
| [`backspace_and_undo_take_back_corners_and_esc_drops_the_polygon`](../../../src/app/lasso_fill.rs#L378) | Backspace and undo take back corners and esc drops the polygon. |
| [`a_held_snap_turns_polygon_edges_to_15_degrees`](../../../src/app/lasso_fill.rs#L394) | A held snap turns polygon edges to 15 degrees. |
| [`a_polygon_fill_takes_a_freehand_stretch`](../../../src/app/lasso_fill.rs#L412) | Between tapped corners a drag adds a freehand stretch to the fill's outline, as on the selection. |
| [`switching_mode_drops_a_lasso_half_drawn`](../../../src/app/lasso_fill.rs#L431) | Switching mode drops a lasso half drawn. |

**Test helpers** (used only by the tests above):

- [`state`](../../../src/app/lasso_fill.rs#L263) — The Fill tool's lasso, red, on a fresh project.
- [`paint`](../../../src/app/lasso_fill.rs#L272) — Paint.
- [`px`](../../../src/app/lasso_fill.rs#L283) — Px.
- [`drag_square`](../../../src/app/lasso_fill.rs#L292) — A freehand square from (x0, y0) to (x1, y1), drawn and let go.

**Test-only types and constants:** [`RED`](../../../src/app/lasso_fill.rs#L259), [`BLACK`](../../../src/app/lasso_fill.rs#L260)
