# `src/tools/shape.rs` — the Shape tool's lines, rectangles and ellipses

> **Source:** [src/tools/shape.rs](../../../src/tools/shape.rs) · **Module:** `crate::tools::shape` · **Size:** 348 lines · Line links as of commit `1fa5845`

## What this file is for

The **Shape** tool draws straight lines, rectangles and ellipses. While you drag, the outline is computed
as a *spine* polyline (a list of points) and previewed; on release it is painted with the same capsule
rasteriser freehand ink uses ([ribbon.rs](ribbon.md)) — but without freehand smoothing, so corners stay
sharp and lines straight. Curves are cut into chords short enough to stay within 0.2 px of the true curve.

With a **perspective grid** active, rectangles and ellipses can lie *on the grid's plane* (a square on the
floor, an ellipse as a circle in perspective) using the grid's homography.

## How it connects

- **Used by:** [app.rs](../app.md) (Shape drag: preview and pointer-up), [ui/shell.rs](../ui/shell.md)
  (preview drawing).
- **Uses:** [tools/ribbon.rs](ribbon.md) (rasteriser and `StrokeWorkspace`),
  [tools/perspective.rs](perspective.md) (`Homography`), [tools/mod.rs](mod.md) (`ShapeKind`,
  `BrushSettings`).

## Rust concepts in this file

**Returning `Option<Vec<…>>`.** `outline_on_plane` returns `None` when the shape can't lie on the plane
(e.g. it would cross the horizon), and the caller falls back to a flat shape.

## Types

### `struct GridPlane`
[src/tools/shape.rs:23](../../../src/tools/shape.rs#L23)

A perspective grid's plane in the target drawing's pixel space, captured when a shape drag starts.

| Field | Type | Meaning |
|---|---|---|
| `h` | `Homography` | The homography mapping grid coordinates `(u, v)` to cell pixels. |
| `rows` | `u32` | The grid's row count. |
| `cols` | `u32` | The grid's column count. |

## Free functions

### `chord`
[src/tools/shape.rs:32](../../../src/tools/shape.rs#L32)

```rust
fn chord(r_curv: f32, radius: f32) -> f32
```

The chord length that keeps a curve of curvature radius `r_curv` within 0.2 px (a chord `c` on radius `R`
deviates by about `c²/8R`), never longer than the brush's thickness.

### `dist`
[src/tools/shape.rs:36](../../../src/tools/shape.rs#L36)

```rust
fn dist(a: (f32, f32), b: (f32, f32)) -> f32
```

Distance between two points.

### `outline`
[src/tools/shape.rs:43](../../../src/tools/shape.rs#L43)

```rust
pub fn outline(kind: ShapeKind, a: (f32, f32), b: (f32, f32), radius: f32) -> Vec<(f32, f32)>
```

The outline polyline for `kind` between the drag anchor `a` and current point `b` — a line, a rectangle, or an
ellipse cut into chords. Closed shapes repeat their first point at the end.

### `outline_on_plane`
[src/tools/shape.rs:85](../../../src/tools/shape.rs#L85)

```rust
pub fn outline_on_plane(
    kind: ShapeKind,
    a: (f32, f32),
    b: (f32, f32),
    plane: &GridPlane,
    square: bool,
    centred: bool,
    radius: f32,
) -> Option<Vec<(f32, f32)>>
```

The outline lying on a perspective grid's plane: the box corners mapped through the homography, an ellipse
as a circle in grid space mapped into the picture. `square` makes the box as many cells deep as wide;
`centred` grows it from the anchor. `None` for lines (they snap to grid directions instead) or shapes reaching
past the horizon.

### `slide`
[src/tools/shape.rs:155](../../../src/tools/shape.rs#L155)

```rust
pub fn slide(
    anchor: (f32, f32),
    from: (f32, f32),
    to: (f32, f32),
    plane: Option<&GridPlane>,
) -> (f32, f32)
```

Where a shape's anchor moves when the move key slides it from `from` to `to`: on a grid, it keeps its place
relative to the pointer in grid space (so the shape slides along the plane and grows as it comes nearer);
otherwise it moves with the pointer.

### `rasterize`
[src/tools/shape.rs:174](../../../src/tools/shape.rs#L174)

```rust
pub fn rasterize(
    canvas: &mut Canvas,
    ws: &mut StrokeWorkspace,
    pre: &[u8],
    spine: &[(f32, f32)],
    brush: &BrushSettings,
)
```

Paints the spine polyline into the canvas with the brush, compositing over the pre-drag snapshot `pre` (so
re-drawing a preview never stacks up). The workspace must have been started for this canvas.

## Tests

### `mod tests`

Run with `cargo test tools::shape::tests` · 7 tests

| Test | What it checks |
|---|---|
| [`a_square_on_grid_draws_the_flat_rect`](../../../src/tools/shape.rs#L232) | A square on grid draws the flat rect. |
| [`a_rect_on_a_floor_runs_its_sides_to_the_vanishing_point`](../../../src/tools/shape.rs#L244) | A rect on a floor runs its sides to the vanishing point. |
| [`square_means_as_many_cells_deep_as_wide`](../../../src/tools/shape.rs#L267) | Square means as many cells deep as wide. |
| [`a_circle_on_the_floor_has_its_far_half_foreshortened`](../../../src/tools/shape.rs#L280) | A circle on the floor has its far half foreshortened. |
| [`a_centred_box_on_the_grid_has_the_press_for_its_middle`](../../../src/tools/shape.rs#L302) | A centred box on the grid has the press for its middle. |
| [`a_slide_keeps_the_anchor_where_it_was_on_the_floor`](../../../src/tools/shape.rs#L322) | A slide keeps the anchor where it was on the floor. |
| [`a_box_past_the_horizon_is_not_on_the_plane`](../../../src/tools/shape.rs#L338) | A box past the horizon is not on the plane. |

**Test helpers** (used only by the tests above):

- [`close`](../../../src/tools/shape.rs#L216) — Close.
- [`plane`](../../../src/tools/shape.rs#L220) — Plane.

**Test-only types and constants:** [`TRAP`](../../../src/tools/shape.rs#L229)
